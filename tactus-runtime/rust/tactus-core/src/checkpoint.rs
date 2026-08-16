use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fmt,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use agentro_contracts::{CanonicalHasher, DigestError, Sha256Digest};
use ignore::WalkBuilder;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::{
    CancellationToken, OutputBudget, ProcessError, ProcessSpec, ProcessSupervisor, ProcessTimeouts,
    TerminationReason,
};

const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MAX_RESTORE_PATHS: usize = 1_000;
const MAX_SCAN_ENTRIES: u64 = 1_000_000;
const MAX_SCAN_BYTES: u64 = 1 << 40;
const MAX_SCAN_DEPTH: usize = 256;
const MAX_SCAN_DURATION: Duration = Duration::from_secs(60 * 60);
const MAX_MANIFEST_ENTRIES: usize = 100_000;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;
const MAX_OBJECT_BYTES: u64 = 1 << 40;

/// Native path component and total byte limits used before manifest encoding.
#[derive(Clone, Copy, Debug)]
pub struct PathPolicy {
    max_components: usize,
    max_component_bytes: u64,
    max_path_bytes: u64,
}

impl PathPolicy {
    /// Constructs non-zero path limits under the runtime hard maxima.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::InvalidConfiguration`] for excessive values.
    pub fn new(
        max_components: usize,
        max_component_bytes: u64,
        max_path_bytes: u64,
    ) -> Result<Self, CheckpointError> {
        if max_components == 0 || max_components > 256 {
            return Err(CheckpointError::InvalidConfiguration {
                field: "path components",
            });
        }
        if max_component_bytes == 0 || max_component_bytes > 1_024 {
            return Err(CheckpointError::InvalidConfiguration {
                field: "path component bytes",
            });
        }
        if max_path_bytes == 0 || max_path_bytes > 32_768 {
            return Err(CheckpointError::InvalidConfiguration {
                field: "path bytes",
            });
        }
        Ok(Self {
            max_components,
            max_component_bytes,
            max_path_bytes,
        })
    }
}

/// File-count, byte, depth, and duration limits for one workspace scan.
#[derive(Clone, Copy, Debug)]
pub struct ScanBudget {
    max_entries: u64,
    max_total_bytes: u64,
    max_file_bytes: u64,
    max_depth: usize,
    max_duration: Duration,
}

impl ScanBudget {
    /// Constructs complete non-zero scan limits.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::InvalidConfiguration`] outside hard maxima.
    pub fn new(
        max_entries: u64,
        max_total_bytes: u64,
        max_file_bytes: u64,
        max_depth: usize,
        max_duration: Duration,
    ) -> Result<Self, CheckpointError> {
        if max_entries == 0 || max_entries > MAX_SCAN_ENTRIES {
            return Err(CheckpointError::InvalidConfiguration {
                field: "scan entries",
            });
        }
        if max_total_bytes == 0 || max_total_bytes > MAX_SCAN_BYTES {
            return Err(CheckpointError::InvalidConfiguration {
                field: "scan bytes",
            });
        }
        if max_file_bytes == 0 || max_file_bytes > max_total_bytes {
            return Err(CheckpointError::InvalidConfiguration {
                field: "scan file bytes",
            });
        }
        if max_depth == 0 || max_depth > MAX_SCAN_DEPTH {
            return Err(CheckpointError::InvalidConfiguration {
                field: "scan depth",
            });
        }
        if max_duration.is_zero() || max_duration > MAX_SCAN_DURATION {
            return Err(CheckpointError::InvalidConfiguration {
                field: "scan duration",
            });
        }
        Ok(Self {
            max_entries,
            max_total_bytes,
            max_file_bytes,
            max_depth,
            max_duration,
        })
    }
}

/// Read-only repository context used around the common CAS scan.
pub trait GitMetadataPort: Send + Sync {
    /// Reads a bounded repository context digest without writing `.git`.
    ///
    /// # Errors
    ///
    /// Returns typed process, cancellation, command, or path failures.
    fn read_context(
        &self,
        workspace_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Sha256Digest, CheckpointError>;
}

/// Read-only official-Git adapter held by a ProcessSupervisor.
pub struct GitCliMetadata<S> {
    supervisor: S,
    executable: PathBuf,
    environment: BTreeMap<OsString, OsString>,
    timeout: Duration,
    max_output_bytes: u64,
}

impl<S> GitCliMetadata<S> {
    /// Constructs a bounded read-only Git metadata adapter.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError::InvalidConfiguration`] for relative paths or
    /// zero/excessive process limits.
    pub fn new(
        supervisor: S,
        executable: PathBuf,
        environment: BTreeMap<OsString, OsString>,
        timeout: Duration,
        max_output_bytes: u64,
    ) -> Result<Self, CheckpointError> {
        if !executable.is_absolute() {
            return Err(CheckpointError::InvalidConfiguration {
                field: "Git executable",
            });
        }
        if timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err(CheckpointError::InvalidConfiguration {
                field: "Git timeout",
            });
        }
        if max_output_bytes == 0 || max_output_bytes > 2 * 1024 * 1024 {
            return Err(CheckpointError::InvalidConfiguration {
                field: "Git output bytes",
            });
        }
        Ok(Self {
            supervisor,
            executable,
            environment,
            timeout,
            max_output_bytes,
        })
    }
}

impl<S: ProcessSupervisor> GitMetadataPort for GitCliMetadata<S> {
    fn read_context(
        &self,
        workspace_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Sha256Digest, CheckpointError> {
        if !workspace_root.is_absolute() {
            return Err(CheckpointError::InvalidWorkspaceRoot);
        }
        let mut environment: BTreeMap<OsString, OsString> = self
            .environment
            .iter()
            .filter(|(name, _)| {
                !name
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("GIT_")
            })
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        environment.insert("GIT_OPTIONAL_LOCKS".into(), "0".into());
        environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        environment.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        environment.insert(
            "GIT_CONFIG_GLOBAL".into(),
            if cfg!(windows) { "NUL" } else { "/dev/null" }.into(),
        );
        let arguments = vec![
            OsString::from("--no-optional-locks"),
            OsString::from("-c"),
            OsString::from("core.fsmonitor=false"),
            OsString::from("-c"),
            OsString::from("core.untrackedCache=false"),
            OsString::from("-c"),
            OsString::from("maintenance.auto=false"),
            OsString::from("-C"),
            workspace_root.as_os_str().to_owned(),
            OsString::from("status"),
            OsString::from("--porcelain=v2"),
            OsString::from("--branch"),
            OsString::from("-z"),
            OsString::from("--untracked-files=all"),
            OsString::from("--ignored=matching"),
        ];
        let spec = ProcessSpec::new(
            self.executable.clone(),
            arguments,
            workspace_root.to_path_buf(),
            environment,
            ProcessTimeouts::new(self.timeout, Duration::from_millis(250))
                .map_err(CheckpointError::GitProcess)?,
            OutputBudget::new(
                self.max_output_bytes,
                self.max_output_bytes,
                self.max_output_bytes.saturating_mul(2),
            )
            .map_err(CheckpointError::GitProcess)?,
        )
        .map_err(CheckpointError::GitProcess)?;
        let output = self
            .supervisor
            .run(spec, cancellation)
            .map_err(CheckpointError::GitProcess)?;
        if output.termination() != TerminationReason::Exited || !output.success() {
            return Err(CheckpointError::GitCommandFailed);
        }
        let mut hasher = Sha256::new();
        hasher.update(b"tactus.git-context.v1\0");
        hasher.update(output.stdout());
        Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
    }
}

/// One immutable SHA-256 object reference and its verified byte length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlobRef {
    digest: Sha256Digest,
    length: u64,
}

impl BlobRef {
    /// Creates a typed immutable object reference.
    #[must_use]
    pub const fn new(digest: Sha256Digest, length: u64) -> Self {
        Self { digest, length }
    }

    /// Returns the SHA-256 object identity.
    #[must_use]
    pub const fn digest(self) -> Sha256Digest {
        self.digest
    }

    /// Returns the expected object length.
    #[must_use]
    pub const fn length(self) -> u64 {
        self.length
    }
}

/// Content-derived identity of a checkpoint contract.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CheckpointId(Sha256Digest);

impl CheckpointId {
    /// Creates a checkpoint identity from a canonical digest.
    #[must_use]
    pub const fn from_digest(digest: Sha256Digest) -> Self {
        Self(digest)
    }

    /// Returns the canonical SHA-256 checkpoint identity.
    #[must_use]
    pub const fn digest(self) -> Sha256Digest {
        self.0
    }

    pub(crate) fn parse(value: &str) -> Result<Self, CheckpointError> {
        Sha256Digest::parse(value)
            .map(Self)
            .map_err(CheckpointError::Digest)
    }
}

impl fmt::Display for CheckpointId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Metadata adapter used while building a common external-CAS checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointBackendKind {
    /// Pure filesystem metadata with the common CAS manifest.
    NonGit,
    /// Read-only Git metadata around the same CAS scan.
    GitAware,
}

impl CheckpointBackendKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::NonGit => "non_git",
            Self::GitAware => "git_aware",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "non_git" => Some(Self::NonGit),
            "git_aware" => Some(Self::GitAware),
            _ => None,
        }
    }
}

/// Restore fidelity honestly exposed by a checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RollbackFidelity {
    /// The bounded scan represents every included manifest path.
    FullManifest,
    /// Only caller-declared paths can be considered for restore.
    DeclaredPaths,
}

impl RollbackFidelity {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::FullManifest => "full_manifest",
            Self::DeclaredPaths => "declared_paths",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "full_manifest" => Some(Self::FullManifest),
            "declared_paths" => Some(Self::DeclaredPaths),
            _ => None,
        }
    }
}

/// Kind of one checkpoint path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointEntryKind {
    /// Immutable regular-file bytes.
    File,
    /// Immutable symbolic-link target text, never followed.
    Symlink,
}

impl CheckpointEntryKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Symlink => "symlink",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "symlink" => Some(Self::Symlink),
            _ => None,
        }
    }
}

/// One canonical path-to-object edge retained for durable restore planning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointEntry {
    path: String,
    kind: CheckpointEntryKind,
    object: BlobRef,
    is_executable: bool,
}

impl CheckpointEntry {
    /// Returns the canonical slash-separated workspace path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the represented filesystem kind.
    #[must_use]
    pub const fn kind(&self) -> CheckpointEntryKind {
        self.kind
    }

    /// Returns the immutable content or link-target reference.
    #[must_use]
    pub const fn object(&self) -> BlobRef {
        self.object
    }

    /// Reports the captured executable bit for regular files.
    #[must_use]
    pub const fn is_executable(&self) -> bool {
        self.is_executable
    }

    pub(crate) fn from_stored(
        path: String,
        kind: CheckpointEntryKind,
        object: BlobRef,
        is_executable: bool,
    ) -> Self {
        Self {
            path,
            kind,
            object,
            is_executable,
        }
    }
}

/// A canonical manifest and optional read-only Git context in external CAS.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Checkpoint {
    id: CheckpointId,
    manifest: BlobRef,
    backend: CheckpointBackendKind,
    fidelity: RollbackFidelity,
    git_context: Option<Sha256Digest>,
    entries: Vec<CheckpointEntry>,
    total_file_bytes: u64,
}

impl Checkpoint {
    /// Returns the content-derived checkpoint identity.
    #[must_use]
    pub const fn id(&self) -> CheckpointId {
        self.id
    }

    /// Returns the canonical manifest CAS reference.
    #[must_use]
    pub const fn manifest(&self) -> BlobRef {
        self.manifest
    }

    /// Returns the metadata adapter used for capture.
    #[must_use]
    pub const fn backend(&self) -> CheckpointBackendKind {
        self.backend
    }

    /// Returns the declared rollback fidelity.
    #[must_use]
    pub const fn fidelity(&self) -> RollbackFidelity {
        self.fidelity
    }

    /// Returns the read-only repository context digest when Git-aware.
    #[must_use]
    pub const fn git_context(&self) -> Option<Sha256Digest> {
        self.git_context
    }

    /// Returns canonical entries in path order.
    #[must_use]
    pub fn entries(&self) -> &[CheckpointEntry] {
        &self.entries
    }

    /// Returns aggregate regular-file bytes represented by the scan.
    #[must_use]
    pub const fn total_file_bytes(&self) -> u64 {
        self.total_file_bytes
    }

    pub(crate) fn from_stored(
        id: CheckpointId,
        manifest: BlobRef,
        backend: CheckpointBackendKind,
        fidelity: RollbackFidelity,
        git_context: Option<Sha256Digest>,
        entries: Vec<CheckpointEntry>,
        total_file_bytes: u64,
    ) -> Self {
        Self {
            id,
            manifest,
            backend,
            fidelity,
            git_context,
            entries,
            total_file_bytes,
        }
    }
}

/// Complete bounded settings for capture and conservative restore.
#[derive(Clone, Copy, Debug)]
pub struct CheckpointConfig {
    path_policy: PathPolicy,
    scan_budget: ScanBudget,
    put_budget: PutBudget,
    manifest_budget: ManifestBudget,
    max_restore_paths: usize,
}

impl CheckpointConfig {
    /// Constructs checkpoint budgets under shared primitive hard limits.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] for an invalid object, manifest, or restore
    /// bound. Path and scan budgets are validated by their own constructors.
    pub fn new(
        path_policy: PathPolicy,
        scan_budget: ScanBudget,
        max_object_bytes: u64,
        max_manifest_entries: usize,
        max_manifest_bytes: u64,
        max_restore_paths: usize,
    ) -> Result<Self, CheckpointError> {
        if max_restore_paths == 0 || max_restore_paths > MAX_RESTORE_PATHS {
            return Err(CheckpointError::InvalidConfiguration {
                field: "restore paths",
            });
        }
        Ok(Self {
            path_policy,
            scan_budget,
            put_budget: PutBudget::new(max_object_bytes).map_err(|_| CheckpointError::Cas)?,
            manifest_budget: ManifestBudget::new(max_manifest_entries, max_manifest_bytes)
                .map_err(|_| CheckpointError::Manifest)?,
            max_restore_paths,
        })
    }
}

/// Why a declared restore path was deliberately left untouched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreConflictReason {
    /// The baseline lacks the path, so restoring would require deletion.
    WouldDelete,
    /// The current path no longer matches the explicitly observed checkpoint.
    ExternalChange,
    /// The initial slice restores regular files only.
    UnsupportedKind,
    /// Runtime and Git control paths are never restore targets.
    ProtectedPath,
}

/// One bounded, non-destructive restore conflict.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreConflict {
    path: String,
    reason: RestoreConflictReason,
}

impl RestoreConflict {
    /// Returns the canonical declared path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns why no write occurred.
    #[must_use]
    pub const fn reason(&self) -> RestoreConflictReason {
        self.reason
    }
}

/// Result of an explicit conservative restore operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreReport {
    restored: Vec<String>,
    conflicts: Vec<RestoreConflict>,
}

impl RestoreReport {
    /// Returns paths atomically replaced from baseline CAS objects.
    #[must_use]
    pub fn restored(&self) -> &[String] {
        &self.restored
    }

    /// Returns paths preserved for explicit conflict resolution.
    #[must_use]
    pub fn conflicts(&self) -> &[RestoreConflict] {
        &self.conflicts
    }
}

/// Checkpoint configuration, scan, integrity, or filesystem failure.
#[derive(Debug, Error)]
pub enum CheckpointError {
    /// A checkpoint setting was zero or above its hard limit.
    #[error("invalid checkpoint configuration: {field}")]
    InvalidConfiguration {
        /// Invalid setting name.
        field: &'static str,
    },
    /// Workspace roots must be explicit absolute directories.
    #[error("workspace root must be an absolute directory")]
    InvalidWorkspaceRoot,
    /// Service CAS state must not be nested in the mutable workspace.
    #[error("service CAS must be outside the workspace")]
    StateInsideWorkspace,
    /// A native path cannot be represented by the portable manifest contract.
    #[error("workspace contains a path that is not portable manifest text")]
    NonPortablePath,
    /// The caller requested cancellation.
    #[error("checkpoint operation cancelled")]
    Cancelled,
    /// Git or filesystem metadata changed during capture.
    #[error("workspace changed while checkpoint was being captured")]
    WorkspaceChanged,
    /// A blob read would exceed the caller's explicit byte limit.
    #[error("blob exceeds read byte limit {maximum}")]
    BlobReadLimit {
        /// Caller-selected maximum bytes.
        maximum: u64,
    },
    /// A CAS object or copied restore object failed digest verification.
    #[error("checkpoint object failed SHA-256 integrity verification")]
    Integrity,
    /// Bounded scanner rejected a path, filesystem event, or budget.
    #[error("workspace scan failed")]
    Scan,
    /// Streaming CAS publication or lookup failed.
    #[error("CAS operation failed")]
    Cas,
    /// Canonical manifest validation or budget failed.
    #[error("canonical manifest operation failed")]
    Manifest,
    /// A supervised read-only Git operation failed at process level.
    #[error("read-only Git process failed")]
    GitProcess(#[source] ProcessError),
    /// Git returned non-success or another terminal condition.
    #[error("read-only Git metadata command failed")]
    GitCommandFailed,
    /// Canonical checkpoint digest failure.
    #[error("canonical checkpoint identity failed")]
    Digest(#[source] DigestError),
    /// Workspace or restore filesystem failure.
    #[error("checkpoint filesystem operation failed during {operation}")]
    Io {
        /// Stable operation label.
        operation: &'static str,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },
}

/// Checkpoint and blob operations needed by the Tactus application layer.
pub trait WorkspacePort: Send + Sync {
    /// Publishes one bounded immutable blob.
    ///
    /// # Errors
    ///
    /// Returns a typed CAS or budget failure.
    fn put_blob(&self, bytes: &[u8]) -> Result<BlobRef, CheckpointError>;

    /// Reads one immutable blob under an explicit allocation limit.
    ///
    /// # Errors
    ///
    /// Returns a typed limit, integrity, or I/O failure.
    fn read_blob(&self, object: BlobRef, max_bytes: u64) -> Result<Vec<u8>, CheckpointError>;

    /// Captures one bounded common-CAS workspace checkpoint.
    ///
    /// # Errors
    ///
    /// Returns typed cancellation, conflict, scan, Git, CAS, or manifest errors.
    fn capture(
        &self,
        workspace_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Checkpoint, CheckpointError>;

    /// Restores only explicitly declared regular files when their current
    /// bytes still match an observed checkpoint. New files are never deleted.
    ///
    /// # Errors
    ///
    /// Returns typed path, integrity, budget, or filesystem errors.
    fn restore_declared(
        &self,
        workspace_root: &Path,
        baseline: &Checkpoint,
        observed: &Checkpoint,
        declared_paths: &[String],
        cancellation: &CancellationToken,
    ) -> Result<RestoreReport, CheckpointError>;
}

/// External-CAS checkpoint implementation with optional read-only Git context.
pub struct CasCheckpointBackend {
    cas: Cas,
    config: CheckpointConfig,
    git: Option<Arc<dyn GitMetadataPort + Send + Sync>>,
}

impl CasCheckpointBackend {
    /// Opens a non-Git checkpoint backend using the common CAS manifest.
    ///
    /// # Errors
    ///
    /// Returns a typed CAS root failure.
    pub fn non_git(cas_root: PathBuf, config: CheckpointConfig) -> Result<Self, CheckpointError> {
        Ok(Self {
            cas: Cas::open(cas_root).map_err(|_| CheckpointError::Cas)?,
            config,
            git: None,
        })
    }

    /// Opens a Git-aware backend whose adapter is read-only and supervised.
    ///
    /// # Errors
    ///
    /// Returns a typed CAS root failure.
    pub fn git_aware<G>(
        cas_root: PathBuf,
        config: CheckpointConfig,
        git: G,
    ) -> Result<Self, CheckpointError>
    where
        G: GitMetadataPort + Send + Sync + 'static,
    {
        Ok(Self {
            cas: Cas::open(cas_root).map_err(|_| CheckpointError::Cas)?,
            config,
            git: Some(Arc::new(git)),
        })
    }

    /// Returns the service-private CAS root for diagnostics.
    #[must_use]
    pub fn cas_root(&self) -> &Path {
        self.cas.root()
    }
}

impl WorkspacePort for CasCheckpointBackend {
    fn put_blob(&self, bytes: &[u8]) -> Result<BlobRef, CheckpointError> {
        let length = u64::try_from(bytes.len()).map_err(|_| CheckpointError::Integrity)?;
        let put = self
            .cas
            .put(bytes, self.config.put_budget)
            .map_err(|_| CheckpointError::Cas)?;
        Ok(BlobRef::new(to_contract_digest(put.digest()), length))
    }

    fn read_blob(&self, object: BlobRef, max_bytes: u64) -> Result<Vec<u8>, CheckpointError> {
        if object.length() > max_bytes {
            return Err(CheckpointError::BlobReadLimit { maximum: max_bytes });
        }
        let mut file = self
            .cas
            .open_object(to_object_digest(object.digest()), object.length())
            .map_err(|_| CheckpointError::Cas)?;
        let capacity = usize::try_from(object.length())
            .map_err(|_| CheckpointError::BlobReadLimit { maximum: max_bytes })?;
        let mut bytes = Vec::with_capacity(capacity);
        file.read_to_end(&mut bytes)
            .map_err(|source| io_error("read CAS object", source))?;
        let actual = Sha256Digest::from_bytes(Sha256::digest(&bytes).into());
        if actual != object.digest() {
            return Err(CheckpointError::Integrity);
        }
        Ok(bytes)
    }

    fn capture(
        &self,
        workspace_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Checkpoint, CheckpointError> {
        let root = validate_workspace_root(workspace_root)?;
        let cas_root = self
            .cas
            .root()
            .canonicalize()
            .map_err(|source| io_error("resolve CAS root", source))?;
        if cas_root.starts_with(&root) {
            return Err(CheckpointError::StateInsideWorkspace);
        }
        if cancellation.is_cancelled() {
            return Err(CheckpointError::Cancelled);
        }

        let git_before = self
            .git
            .as_ref()
            .map(|git| git.read_context(&root, cancellation))
            .transpose()?;
        let scan = scan_project(
            &root,
            self.config.path_policy,
            self.config.scan_budget,
            cancellation,
        )
        .map_err(|_| CheckpointError::Scan)?;
        let mut checkpoint_entries = Vec::new();
        let mut manifest_entries = Vec::new();
        for scan_entry in scan.entries() {
            if cancellation.is_cancelled() {
                return Err(CheckpointError::Cancelled);
            }
            if scan_entry.kind() == ScanEntryKind::Directory {
                continue;
            }
            let relative = scan_entry.path().as_path();
            let portable = portable_path(relative)?;
            let absolute = root.join(relative);
            let (kind, put, is_executable) = match scan_entry.kind() {
                ScanEntryKind::File => {
                    let before = fs::symlink_metadata(&absolute)
                        .map_err(|source| io_error("inspect workspace file", source))?;
                    if !before.file_type().is_file() {
                        return Err(CheckpointError::WorkspaceChanged);
                    }
                    let file = File::open(&absolute)
                        .map_err(|source| io_error("open workspace file", source))?;
                    let put = self
                        .cas
                        .put(file, self.config.put_budget)
                        .map_err(|_| CheckpointError::Cas)?;
                    let after = fs::symlink_metadata(&absolute)
                        .map_err(|source| io_error("reinspect workspace file", source))?;
                    if !same_file_observation(&before, &after) || put.length() != after.len() {
                        return Err(CheckpointError::WorkspaceChanged);
                    }
                    (CheckpointEntryKind::File, put, executable_bit(&after))
                }
                ScanEntryKind::Symlink => {
                    let target = fs::read_link(&absolute)
                        .map_err(|source| io_error("read symbolic link", source))?;
                    let target = target.to_str().ok_or(CheckpointError::NonPortablePath)?;
                    let put = self
                        .cas
                        .put(target.as_bytes(), self.config.put_budget)
                        .map_err(|_| CheckpointError::Cas)?;
                    (CheckpointEntryKind::Symlink, put, false)
                }
                ScanEntryKind::Directory => continue,
            };
            let object = BlobRef::new(to_contract_digest(put.digest()), put.length());
            let path = ManifestPath::parse(portable).map_err(|_| CheckpointError::Manifest)?;
            let manifest_kind = match kind {
                CheckpointEntryKind::File => ManifestEntryKind::File,
                CheckpointEntryKind::Symlink => ManifestEntryKind::Symlink,
            };
            manifest_entries.push(
                ManifestEntry::new(
                    path,
                    manifest_kind,
                    put.digest(),
                    put.length(),
                    is_executable,
                )
                .map_err(|_| CheckpointError::Manifest)?,
            );
            checkpoint_entries.push(CheckpointEntry {
                path: portable_path(relative)?,
                kind,
                object,
                is_executable,
            });
        }

        let git_after = self
            .git
            .as_ref()
            .map(|git| git.read_context(&root, cancellation))
            .transpose()?;
        if git_before != git_after {
            return Err(CheckpointError::WorkspaceChanged);
        }
        let git_context = git_after;
        let manifest = Manifest::build(manifest_entries, self.config.manifest_budget)
            .map_err(|_| CheckpointError::Manifest)?;
        let manifest_put = self
            .cas
            .put(manifest.canonical_bytes(), self.config.put_budget)
            .map_err(|_| CheckpointError::Cas)?;
        if manifest_put.digest() != manifest.digest() {
            return Err(CheckpointError::Integrity);
        }
        let backend = if self.git.is_some() {
            CheckpointBackendKind::GitAware
        } else {
            CheckpointBackendKind::NonGit
        };
        let manifest_ref = BlobRef::new(
            to_contract_digest(manifest_put.digest()),
            manifest_put.length(),
        );
        let id = checkpoint_id(backend, manifest_ref, git_context)?;
        checkpoint_entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Checkpoint {
            id,
            manifest: manifest_ref,
            backend,
            fidelity: RollbackFidelity::FullManifest,
            git_context,
            entries: checkpoint_entries,
            total_file_bytes: scan.total_file_bytes(),
        })
    }

    fn restore_declared(
        &self,
        workspace_root: &Path,
        baseline: &Checkpoint,
        observed: &Checkpoint,
        declared_paths: &[String],
        cancellation: &CancellationToken,
    ) -> Result<RestoreReport, CheckpointError> {
        if declared_paths.len() > self.config.max_restore_paths {
            return Err(CheckpointError::InvalidConfiguration {
                field: "restore paths",
            });
        }
        let root = validate_workspace_root(workspace_root)?;
        let baseline_entries: BTreeMap<_, _> = baseline
            .entries()
            .iter()
            .map(|entry| (entry.path(), entry))
            .collect();
        let observed_entries: BTreeMap<_, _> = observed
            .entries()
            .iter()
            .map(|entry| (entry.path(), entry))
            .collect();
        let mut unique = BTreeSet::new();
        for path in declared_paths {
            ManifestPath::parse(path.clone()).map_err(|_| CheckpointError::Manifest)?;
            unique.insert(path.clone());
        }

        let mut restored = Vec::new();
        let mut conflicts = Vec::new();
        for path in unique {
            if cancellation.is_cancelled() {
                return Err(CheckpointError::Cancelled);
            }
            if is_protected_path(&path) {
                conflicts.push(conflict(path, RestoreConflictReason::ProtectedPath));
                continue;
            }
            let Some(baseline_entry) = baseline_entries.get(path.as_str()) else {
                conflicts.push(conflict(path, RestoreConflictReason::WouldDelete));
                continue;
            };
            let Some(observed_entry) = observed_entries.get(path.as_str()) else {
                conflicts.push(conflict(path, RestoreConflictReason::ExternalChange));
                continue;
            };
            if baseline_entry.kind() != CheckpointEntryKind::File
                || observed_entry.kind() != CheckpointEntryKind::File
            {
                conflicts.push(conflict(path, RestoreConflictReason::UnsupportedKind));
                continue;
            }
            if baseline_entry.object() == observed_entry.object() {
                continue;
            }
            let destination = root.join(native_relative_path(&path));
            if !file_matches(&destination, observed_entry.object())? {
                conflicts.push(conflict(path, RestoreConflictReason::ExternalChange));
                continue;
            }
            restore_regular_file(
                &self.cas,
                &destination,
                baseline_entry.object(),
                observed_entry.object(),
            )?;
            restored.push(path);
        }
        Ok(RestoreReport {
            restored,
            conflicts,
        })
    }
}

fn validate_workspace_root(root: &Path) -> Result<PathBuf, CheckpointError> {
    if !root.is_absolute() {
        return Err(CheckpointError::InvalidWorkspaceRoot);
    }
    let resolved = root
        .canonicalize()
        .map_err(|_| CheckpointError::InvalidWorkspaceRoot)?;
    let metadata =
        fs::symlink_metadata(&resolved).map_err(|_| CheckpointError::InvalidWorkspaceRoot)?;
    if !metadata.file_type().is_dir() {
        return Err(CheckpointError::InvalidWorkspaceRoot);
    }
    Ok(resolved)
}

pub(crate) fn workspace_binding_digest(root: &Path) -> Result<Sha256Digest, CheckpointError> {
    let resolved = validate_workspace_root(root)?;
    let mut hasher = Sha256::new();
    hasher.update(b"tactus.workspace-binding.v1\0");
    update_native_path(&mut hasher, resolved.as_os_str());
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}

#[cfg(windows)]
fn update_native_path(hasher: &mut Sha256, value: &std::ffi::OsStr) {
    use std::os::windows::ffi::OsStrExt;
    for unit in value.encode_wide() {
        hasher.update(unit.to_le_bytes());
    }
}

#[cfg(unix)]
fn update_native_path(hasher: &mut Sha256, value: &std::ffi::OsStr) {
    use std::os::unix::ffi::OsStrExt;
    hasher.update(value.as_bytes());
}

#[cfg(not(any(unix, windows)))]
fn update_native_path(hasher: &mut Sha256, value: &std::ffi::OsStr) {
    hasher.update(value.to_string_lossy().as_bytes());
}

fn portable_path(path: &Path) -> Result<String, CheckpointError> {
    let mut portable = String::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(CheckpointError::NonPortablePath);
        };
        let component = component.to_str().ok_or(CheckpointError::NonPortablePath)?;
        if !portable.is_empty() {
            portable.push('/');
        }
        portable.push_str(component);
    }
    if portable.is_empty() {
        Err(CheckpointError::NonPortablePath)
    } else {
        Ok(portable)
    }
}

fn native_relative_path(path: &str) -> PathBuf {
    path.split('/').collect()
}

fn same_file_observation(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    before.file_type().is_file()
        && after.file_type().is_file()
        && before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
}

#[cfg(unix)]
fn executable_bit(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
const fn executable_bit(_metadata: &fs::Metadata) -> bool {
    false
}

fn checkpoint_id(
    backend: CheckpointBackendKind,
    manifest: BlobRef,
    git_context: Option<Sha256Digest>,
) -> Result<CheckpointId, CheckpointError> {
    let mut hasher = CanonicalHasher::new("tactus.checkpoint").map_err(CheckpointError::Digest)?;
    hasher
        .write_field("backend", backend.as_str().as_bytes())
        .map_err(CheckpointError::Digest)?;
    hasher
        .write_field(
            "git_context",
            git_context
                .as_ref()
                .map_or(&[][..], |digest| digest.as_bytes()),
        )
        .map_err(CheckpointError::Digest)?;
    hasher
        .write_field("manifest", manifest.digest().as_bytes())
        .map_err(CheckpointError::Digest)?;
    Ok(CheckpointId::from_digest(hasher.finish()))
}

fn to_contract_digest(digest: ObjectDigest) -> Sha256Digest {
    Sha256Digest::from_bytes(*digest.as_bytes())
}

fn to_object_digest(digest: Sha256Digest) -> ObjectDigest {
    ObjectDigest::from_bytes(*digest.as_bytes())
}

fn is_protected_path(path: &str) -> bool {
    let Some(first) = path.split('/').next() else {
        return false;
    };
    #[cfg(windows)]
    {
        [".git", ".tactus", ".agentro", ".agentro-state"]
            .iter()
            .any(|protected| first.eq_ignore_ascii_case(protected))
    }
    #[cfg(not(windows))]
    {
        matches!(first, ".git" | ".tactus" | ".agentro" | ".agentro-state")
    }
}

fn conflict(path: String, reason: RestoreConflictReason) -> RestoreConflict {
    RestoreConflict { path, reason }
}

fn file_matches(path: &Path, expected: BlobRef) -> Result<bool, CheckpointError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(io_error("inspect restore target", source)),
    };
    if !metadata.file_type().is_file() || metadata.len() != expected.length() {
        return Ok(false);
    }
    let mut file = File::open(path).map_err(|source| io_error("open restore target", source))?;
    let (digest, length) = hash_reader(&mut file)?;
    Ok(digest == expected.digest() && length == expected.length())
}

fn restore_regular_file(
    cas: &Cas,
    destination: &Path,
    baseline: BlobRef,
    observed: BlobRef,
) -> Result<(), CheckpointError> {
    let parent = destination
        .parent()
        .ok_or(CheckpointError::NonPortablePath)?;
    if !parent.is_dir() || !file_matches(destination, observed)? {
        return Err(CheckpointError::WorkspaceChanged);
    }
    let mut source = cas
        .open_object(to_object_digest(baseline.digest()), baseline.length())
        .map_err(|_| CheckpointError::Cas)?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|source| io_error("create restore temporary file", source))?;
    let mut hasher = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|source| io_error("read baseline object", source))?;
        if read == 0 {
            break;
        }
        let read_u64 = u64::try_from(read).map_err(|_| CheckpointError::Integrity)?;
        length = length
            .checked_add(read_u64)
            .ok_or(CheckpointError::Integrity)?;
        hasher.update(&buffer[..read]);
        temporary
            .write_all(&buffer[..read])
            .map_err(|source| io_error("write restore temporary file", source))?;
    }
    let digest = Sha256Digest::from_bytes(hasher.finalize().into());
    if digest != baseline.digest() || length != baseline.length() {
        return Err(CheckpointError::Integrity);
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|source| io_error("synchronize restore temporary file", source))?;
    if !file_matches(destination, observed)? {
        return Err(CheckpointError::WorkspaceChanged);
    }
    temporary
        .persist(destination)
        .map_err(|error| io_error("publish restored file", error.error))?;
    sync_directory(parent)?;
    Ok(())
}

fn hash_reader(reader: &mut impl Read) -> Result<(Sha256Digest, u64), CheckpointError> {
    let mut hasher = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| io_error("hash workspace file", source))?;
        if read == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(read).map_err(|_| CheckpointError::Integrity)?)
            .ok_or(CheckpointError::Integrity)?;
        hasher.update(&buffer[..read]);
    }
    Ok((Sha256Digest::from_bytes(hasher.finalize().into()), length))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), CheckpointError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("synchronize restored file directory", source))
}

#[cfg(not(unix))]
const fn sync_directory(_path: &Path) -> Result<(), CheckpointError> {
    Ok(())
}

fn io_error(operation: &'static str, source: io::Error) -> CheckpointError {
    CheckpointError::Io { operation, source }
}

type ObjectDigest = Sha256Digest;

#[derive(Clone, Copy, Debug)]
struct PutBudget {
    max_bytes: u64,
}

impl PutBudget {
    fn new(max_bytes: u64) -> Result<Self, PrimitiveError> {
        if max_bytes == 0 || max_bytes > MAX_OBJECT_BYTES {
            return Err(PrimitiveError);
        }
        Ok(Self { max_bytes })
    }
}

#[derive(Clone, Copy, Debug)]
struct ManifestBudget {
    max_entries: usize,
    max_bytes: u64,
}

impl ManifestBudget {
    fn new(max_entries: usize, max_bytes: u64) -> Result<Self, PrimitiveError> {
        if max_entries == 0
            || max_entries > MAX_MANIFEST_ENTRIES
            || max_bytes == 0
            || max_bytes > MAX_MANIFEST_BYTES
        {
            return Err(PrimitiveError);
        }
        Ok(Self {
            max_entries,
            max_bytes,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct PrimitiveError;

#[derive(Debug)]
struct ObjectPut {
    digest: ObjectDigest,
    length: u64,
}

impl ObjectPut {
    const fn digest(&self) -> ObjectDigest {
        self.digest
    }

    const fn length(&self) -> u64 {
        self.length
    }
}

#[derive(Debug)]
struct Cas {
    root: PathBuf,
    objects: PathBuf,
    temporary: PathBuf,
}

impl Cas {
    fn open(root: PathBuf) -> Result<Self, PrimitiveError> {
        if !root.is_absolute() {
            return Err(PrimitiveError);
        }
        let objects = root.join("objects").join("sha256");
        let temporary = root.join("objects").join(".tmp");
        fs::create_dir_all(&objects).map_err(|_| PrimitiveError)?;
        fs::create_dir_all(&temporary).map_err(|_| PrimitiveError)?;
        Ok(Self {
            root,
            objects,
            temporary,
        })
    }

    fn root(&self) -> &Path {
        self.root.as_path()
    }

    fn put(&self, mut reader: impl Read, budget: PutBudget) -> Result<ObjectPut, PrimitiveError> {
        let mut temporary = NamedTempFile::new_in(&self.temporary).map_err(|_| PrimitiveError)?;
        let mut hasher = Sha256::new();
        let mut length = 0_u64;
        let mut buffer = [0_u8; COPY_BUFFER_BYTES];
        loop {
            let read = reader.read(&mut buffer).map_err(|_| PrimitiveError)?;
            if read == 0 {
                break;
            }
            length = length
                .checked_add(u64::try_from(read).map_err(|_| PrimitiveError)?)
                .ok_or(PrimitiveError)?;
            if length > budget.max_bytes {
                return Err(PrimitiveError);
            }
            hasher.update(&buffer[..read]);
            temporary
                .write_all(&buffer[..read])
                .map_err(|_| PrimitiveError)?;
        }
        temporary.as_file().sync_all().map_err(|_| PrimitiveError)?;
        let digest = Sha256Digest::from_bytes(hasher.finalize().into());
        let destination = self.object_path(digest);
        let parent = destination.parent().ok_or(PrimitiveError)?;
        fs::create_dir_all(parent).map_err(|_| PrimitiveError)?;
        if destination.exists() {
            let metadata = fs::symlink_metadata(&destination).map_err(|_| PrimitiveError)?;
            if !metadata.file_type().is_file() || metadata.len() != length {
                return Err(PrimitiveError);
            }
            return Ok(ObjectPut { digest, length });
        }
        match temporary.persist_noclobber(&destination) {
            Ok(file) => file.sync_all().map_err(|_| PrimitiveError)?,
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&destination).map_err(|_| PrimitiveError)?;
                if !metadata.file_type().is_file() || metadata.len() != length {
                    return Err(PrimitiveError);
                }
            }
            Err(_) => return Err(PrimitiveError),
        }
        Ok(ObjectPut { digest, length })
    }

    fn open_object(
        &self,
        digest: ObjectDigest,
        expected_length: u64,
    ) -> Result<File, PrimitiveError> {
        let path = self.object_path(digest);
        let metadata = fs::symlink_metadata(&path).map_err(|_| PrimitiveError)?;
        if !metadata.file_type().is_file() || metadata.len() != expected_length {
            return Err(PrimitiveError);
        }
        File::open(path).map_err(|_| PrimitiveError)
    }

    fn object_path(&self, digest: ObjectDigest) -> PathBuf {
        let encoded = digest.to_string();
        let hexadecimal = encoded.strip_prefix("sha256:").unwrap_or(encoded.as_str());
        self.objects.join(&hexadecimal[..2]).join(hexadecimal)
    }
}

#[derive(Clone, Debug)]
struct ManifestPath(String);

impl ManifestPath {
    fn parse(value: String) -> Result<Self, PrimitiveError> {
        if value.is_empty()
            || value.len() > 4_096
            || value.starts_with('/')
            || value.ends_with('/')
            || value.contains(['\\', ':', '\0'])
        {
            return Err(PrimitiveError);
        }
        for component in value.split('/') {
            if component.is_empty()
                || component == "."
                || component == ".."
                || component.len() > 255
                || component.chars().any(char::is_control)
                || component.ends_with(['.', ' '])
                || windows_reserved(component)
            {
                return Err(PrimitiveError);
            }
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManifestEntryKind {
    File,
    Symlink,
}

#[derive(Clone, Debug)]
struct ManifestEntry {
    path: ManifestPath,
    kind: ManifestEntryKind,
    digest: ObjectDigest,
    length: u64,
    is_executable: bool,
}

impl ManifestEntry {
    fn new(
        path: ManifestPath,
        kind: ManifestEntryKind,
        digest: ObjectDigest,
        length: u64,
        is_executable: bool,
    ) -> Result<Self, PrimitiveError> {
        if kind == ManifestEntryKind::Symlink && is_executable {
            return Err(PrimitiveError);
        }
        Ok(Self {
            path,
            kind,
            digest,
            length,
            is_executable,
        })
    }
}

#[derive(Debug)]
struct Manifest {
    canonical: Vec<u8>,
    digest: ObjectDigest,
}

impl Manifest {
    fn build(
        entries: impl IntoIterator<Item = ManifestEntry>,
        budget: ManifestBudget,
    ) -> Result<Self, PrimitiveError> {
        let mut sorted = BTreeMap::new();
        let mut casefolded = BTreeSet::new();
        for entry in entries {
            if sorted.len() >= budget.max_entries
                || sorted.contains_key(&entry.path.0)
                || !casefolded.insert(entry.path.0.to_ascii_lowercase())
            {
                return Err(PrimitiveError);
            }
            sorted.insert(entry.path.0.clone(), entry);
        }
        let mut canonical = Vec::new();
        append_manifest(&mut canonical, b"tactus.cas.manifest\0", budget.max_bytes)?;
        append_manifest(&mut canonical, &1_u32.to_be_bytes(), budget.max_bytes)?;
        append_manifest(
            &mut canonical,
            &u64::try_from(sorted.len())
                .map_err(|_| PrimitiveError)?
                .to_be_bytes(),
            budget.max_bytes,
        )?;
        for entry in sorted.into_values() {
            let path = entry.path.0.as_bytes();
            append_manifest(
                &mut canonical,
                &u32::try_from(path.len())
                    .map_err(|_| PrimitiveError)?
                    .to_be_bytes(),
                budget.max_bytes,
            )?;
            append_manifest(&mut canonical, path, budget.max_bytes)?;
            append_manifest(
                &mut canonical,
                &[
                    if entry.kind == ManifestEntryKind::File {
                        1
                    } else {
                        2
                    },
                    u8::from(entry.is_executable),
                ],
                budget.max_bytes,
            )?;
            append_manifest(
                &mut canonical,
                &entry.length.to_be_bytes(),
                budget.max_bytes,
            )?;
            append_manifest(&mut canonical, entry.digest.as_bytes(), budget.max_bytes)?;
        }
        let digest = Sha256Digest::from_bytes(Sha256::digest(&canonical).into());
        Ok(Self { canonical, digest })
    }

    fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    const fn digest(&self) -> ObjectDigest {
        self.digest
    }
}

fn append_manifest(target: &mut Vec<u8>, value: &[u8], maximum: u64) -> Result<(), PrimitiveError> {
    let next = u64::try_from(target.len())
        .map_err(|_| PrimitiveError)?
        .checked_add(u64::try_from(value.len()).map_err(|_| PrimitiveError)?)
        .ok_or(PrimitiveError)?;
    if next > maximum {
        return Err(PrimitiveError);
    }
    target.extend_from_slice(value);
    Ok(())
}

fn windows_reserved(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScanEntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone, Debug)]
struct RelativePath(PathBuf);

impl RelativePath {
    fn as_path(&self) -> &Path {
        &self.0
    }
}

#[derive(Clone, Debug)]
struct ScanEntry {
    path: RelativePath,
    kind: ScanEntryKind,
}

impl ScanEntry {
    fn path(&self) -> &RelativePath {
        &self.path
    }

    const fn kind(&self) -> ScanEntryKind {
        self.kind
    }
}

#[derive(Debug)]
struct ScanResult {
    entries: Vec<ScanEntry>,
    total_file_bytes: u64,
}

impl ScanResult {
    fn entries(&self) -> &[ScanEntry] {
        &self.entries
    }

    const fn total_file_bytes(&self) -> u64 {
        self.total_file_bytes
    }
}

fn scan_project(
    root: &Path,
    path_policy: PathPolicy,
    budget: ScanBudget,
    cancellation: &CancellationToken,
) -> Result<ScanResult, PrimitiveError> {
    let started = std::time::Instant::now();
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(false)
        .parents(false)
        .ignore(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .follow_links(false)
        .max_depth(Some(budget.max_depth))
        .sort_by_file_name(OsStr::cmp)
        .add_custom_ignore_filename(".agentroignore")
        .filter_entry(|entry| !always_ignored(entry));
    let mut entries = Vec::new();
    let mut visited = 0_u64;
    let mut total_file_bytes = 0_u64;
    for item in builder.build() {
        if cancellation.is_cancelled() || started.elapsed() > budget.max_duration {
            return Err(PrimitiveError);
        }
        let item = item.map_err(|_| PrimitiveError)?;
        if item.depth() == 0 {
            continue;
        }
        visited = visited.checked_add(1).ok_or(PrimitiveError)?;
        if visited > budget.max_entries {
            return Err(PrimitiveError);
        }
        let relative = item.path().strip_prefix(root).map_err(|_| PrimitiveError)?;
        validate_relative_path(relative, path_policy)?;
        let metadata = fs::symlink_metadata(item.path()).map_err(|_| PrimitiveError)?;
        let kind = if metadata.file_type().is_file() {
            if metadata.len() > budget.max_file_bytes {
                return Err(PrimitiveError);
            }
            total_file_bytes = total_file_bytes
                .checked_add(metadata.len())
                .ok_or(PrimitiveError)?;
            if total_file_bytes > budget.max_total_bytes {
                return Err(PrimitiveError);
            }
            ScanEntryKind::File
        } else if metadata.file_type().is_dir() {
            ScanEntryKind::Directory
        } else if metadata.file_type().is_symlink() {
            ScanEntryKind::Symlink
        } else {
            continue;
        };
        entries.push(ScanEntry {
            path: RelativePath(relative.to_path_buf()),
            kind,
        });
    }
    Ok(ScanResult {
        entries,
        total_file_bytes,
    })
}

fn always_ignored(entry: &ignore::DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return false;
    }
    matches!(
        entry.file_name().to_str(),
        Some(
            ".git"
                | ".tactus"
                | ".agentro"
                | ".agentro-state"
                | "node_modules"
                | ".venv"
                | "build"
                | "target"
        )
    )
}

fn validate_relative_path(path: &Path, policy: PathPolicy) -> Result<(), PrimitiveError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(PrimitiveError);
    }
    let mut count = 0_usize;
    let mut total = 0_u64;
    for component in path.components() {
        let Component::Normal(value) = component else {
            return Err(PrimitiveError);
        };
        count = count.checked_add(1).ok_or(PrimitiveError)?;
        let bytes = native_component_bytes(value);
        total = total
            .checked_add(bytes.saturating_add(1))
            .ok_or(PrimitiveError)?;
        if count > policy.max_components
            || bytes > policy.max_component_bytes
            || total > policy.max_path_bytes
        {
            return Err(PrimitiveError);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn native_component_bytes(value: &OsStr) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    value
        .encode_wide()
        .fold(0_u64, |total, _| total.saturating_add(2))
}

#[cfg(unix)]
fn native_component_bytes(value: &OsStr) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().len() as u64
}

#[cfg(not(any(unix, windows)))]
fn native_component_bytes(value: &OsStr) -> u64 {
    value.to_string_lossy().len() as u64
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        error::Error,
        ffi::OsString,
        process::{Command, Stdio},
        time::Duration,
    };

    use tempfile::tempdir;

    use super::*;

    fn config() -> Result<CheckpointConfig, Box<dyn Error>> {
        Ok(CheckpointConfig::new(
            PathPolicy::new(32, 255, 4_096)?,
            ScanBudget::new(1_000, 1_048_576, 1_048_576, 32, Duration::from_secs(5))?,
            1_048_576,
            1_000,
            1_048_576,
            100,
        )?)
    }

    #[test]
    fn non_git_checkpoint_and_restore_share_external_cas_contract() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let workspace = temporary.path().join("workspace");
        let cas = temporary.path().join("state").join("cas");
        fs::create_dir(&workspace)?;
        fs::write(workspace.join("tracked.txt"), b"before\n")?;
        let backend = CasCheckpointBackend::non_git(cas, config()?)?;
        let cancellation = CancellationToken::new();
        let baseline = backend.capture(&workspace, &cancellation)?;

        fs::write(workspace.join("tracked.txt"), b"worker edit\n")?;
        fs::write(workspace.join("new.txt"), b"preserve\n")?;
        let observed = backend.capture(&workspace, &cancellation)?;
        let report = backend.restore_declared(
            &workspace,
            &baseline,
            &observed,
            &["tracked.txt".to_owned(), "new.txt".to_owned()],
            &cancellation,
        )?;
        assert_eq!(report.restored(), &["tracked.txt".to_owned()]);
        assert_eq!(fs::read(workspace.join("tracked.txt"))?, b"before\n");
        assert_eq!(fs::read(workspace.join("new.txt"))?, b"preserve\n");
        assert_eq!(
            report.conflicts()[0].reason(),
            RestoreConflictReason::WouldDelete
        );
        assert_eq!(baseline.backend(), CheckpointBackendKind::NonGit);
        assert_eq!(baseline.fidelity(), RollbackFidelity::FullManifest);
        Ok(())
    }

    #[test]
    fn restore_refuses_to_overwrite_change_after_observation() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let workspace = temporary.path().join("workspace");
        fs::create_dir(&workspace)?;
        fs::write(workspace.join("data.txt"), b"base")?;
        let backend =
            CasCheckpointBackend::non_git(temporary.path().join("state").join("cas"), config()?)?;
        let cancellation = CancellationToken::new();
        let baseline = backend.capture(&workspace, &cancellation)?;
        fs::write(workspace.join("data.txt"), b"worker")?;
        let observed = backend.capture(&workspace, &cancellation)?;
        fs::write(workspace.join("data.txt"), b"external")?;

        let report = backend.restore_declared(
            &workspace,
            &baseline,
            &observed,
            &["data.txt".to_owned()],
            &cancellation,
        )?;
        assert!(report.restored().is_empty());
        assert_eq!(
            report.conflicts()[0].reason(),
            RestoreConflictReason::ExternalChange
        );
        assert_eq!(fs::read(workspace.join("data.txt"))?, b"external");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn protected_paths_are_case_insensitive_on_windows() {
        for path in [
            ".GIT/index",
            ".Tactus/state.sqlite3",
            ".AGENTRO/run.db",
            ".Agentro-State/lease",
        ] {
            assert!(is_protected_path(path), "expected {path} to be protected");
        }
        assert!(!is_protected_path(".github/workflows/check.yml"));
        assert!(!is_protected_path(".tactus-cache/data"));
        assert!(!is_protected_path("src/main.rs"));
    }

    #[cfg(not(windows))]
    #[test]
    fn protected_paths_remain_case_sensitive_off_windows() {
        assert!(is_protected_path(".git/index"));
        assert!(is_protected_path(".tactus/state.sqlite3"));
        assert!(!is_protected_path(".GIT/index"));
        assert!(!is_protected_path(".Tactus/state.sqlite3"));
        assert!(!is_protected_path(".github/workflows/check.yml"));
        assert!(!is_protected_path(".tactus-cache/data"));
        assert!(!is_protected_path("src/main.rs"));
    }

    #[test]
    fn git_aware_capture_does_not_modify_temporary_git_directory() -> Result<(), Box<dyn Error>> {
        let Some(git) = locate_git() else {
            return Ok(());
        };
        let temporary = tempdir()?;
        let workspace = temporary.path().join("workspace");
        fs::create_dir(&workspace)?;
        let environment = minimal_git_environment();
        let initialized = Command::new(&git)
            .arg("init")
            .arg("--quiet")
            .current_dir(&workspace)
            .env_clear()
            .envs(&environment)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !initialized.success() {
            return Err("temporary git init failed".into());
        }
        fs::write(workspace.join("input.txt"), b"input\n")?;
        let before = snapshot_directory(&workspace.join(".git"))?;
        let git_metadata = GitCliMetadata::new(
            crate::NativeProcessSupervisor::new(),
            git,
            environment,
            Duration::from_secs(5),
            1024 * 1024,
        )?;
        let backend = CasCheckpointBackend::git_aware(
            temporary.path().join("state").join("cas"),
            config()?,
            git_metadata,
        )?;

        let checkpoint = backend.capture(&workspace, &CancellationToken::new())?;
        let after = snapshot_directory(&workspace.join(".git"))?;

        assert_eq!(checkpoint.backend(), CheckpointBackendKind::GitAware);
        assert!(checkpoint.git_context().is_some());
        assert_eq!(before, after);
        Ok(())
    }

    fn locate_git() -> Option<PathBuf> {
        let path = env::var_os("PATH")?;
        for directory in env::split_paths(&path) {
            for name in if cfg!(windows) {
                &["git.exe", "git.cmd"][..]
            } else {
                &["git"][..]
            } {
                let candidate = directory.join(name);
                if candidate.is_file()
                    && let Ok(resolved) = candidate.canonicalize()
                {
                    return Some(resolved);
                }
            }
        }
        None
    }

    fn minimal_git_environment() -> BTreeMap<OsString, OsString> {
        ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"]
            .into_iter()
            .filter_map(|name| env::var_os(name).map(|value| (OsString::from(name), value)))
            .collect()
    }

    fn snapshot_directory(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, io::Error> {
        let mut pending = vec![root.to_path_buf()];
        let mut snapshot = BTreeMap::new();
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if file_type.is_dir() {
                    pending.push(entry.path());
                } else if file_type.is_file() {
                    let relative = entry
                        .path()
                        .strip_prefix(root)
                        .map_err(io::Error::other)?
                        .to_path_buf();
                    snapshot.insert(relative, fs::read(entry.path())?);
                }
            }
        }
        Ok(snapshot)
    }
}
