use std::{fmt, num::NonZeroU64, str::FromStr};

use agentro_contracts::{DigestError, RequestId, Sha256Digest};
use thiserror::Error;
use uuid::{Uuid, Variant};

/// A malformed Tactus storage key or fencing token.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum KeyError {
    /// The key is not syntactically valid UUID text.
    #[error("storage key is not a valid UUID")]
    Malformed,
    /// The key is not canonical lower-case hyphenated text.
    #[error("storage key is not in canonical lower-case UUID form")]
    NonCanonical,
    /// The key does not use the RFC UUID variant.
    #[error("storage key does not use the RFC UUID variant")]
    WrongVariant,
    /// The key is not UUIDv7.
    #[error("storage key is not UUIDv7")]
    WrongVersion,
    /// Fencing tokens start at one.
    #[error("fencing token must be greater than zero")]
    ZeroFence,
}

fn parse_uuid_key(value: &str) -> Result<Uuid, KeyError> {
    let uuid = Uuid::try_parse(value).map_err(|_| KeyError::Malformed)?;
    if uuid.get_variant() != Variant::RFC4122 {
        return Err(KeyError::WrongVariant);
    }
    if uuid.get_version_num() != 7 {
        return Err(KeyError::WrongVersion);
    }
    if uuid.hyphenated().to_string() != value {
        return Err(KeyError::NonCanonical);
    }
    Ok(uuid)
}

macro_rules! define_uuid_key {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Uuid);

        impl $name {
            /// Parses one canonical lower-case UUIDv7 storage key.
            ///
            /// # Errors
            ///
            /// Returns [`KeyError`] for malformed, non-canonical, non-RFC, or
            /// non-v7 input.
            pub fn parse(value: &str) -> Result<Self, KeyError> {
                parse_uuid_key(value).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.hyphenated().fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = KeyError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

define_uuid_key!(
    /// Storage identity of one Tactus project.
    ProjectKey
);
define_uuid_key!(
    /// Storage identity of one stable Tactus cell.
    CellKey
);
define_uuid_key!(
    /// Storage identity of one Tactus run attempt.
    RunKey
);
define_uuid_key!(
    /// Storage identity of one workspace transaction.
    WorkspaceTransactionKey
);
define_uuid_key!(
    /// Storage identity of one process holding a project lease.
    LeaseOwnerKey
);

/// Content-derived storage identity of a checkpoint.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CheckpointKey(Sha256Digest);

impl CheckpointKey {
    /// Creates a checkpoint key from its canonical digest.
    #[must_use]
    pub const fn from_digest(digest: Sha256Digest) -> Self {
        Self(digest)
    }

    /// Parses canonical SHA-256 text.
    ///
    /// # Errors
    ///
    /// Returns [`DigestError`] when the text is not canonical SHA-256.
    pub fn parse(value: &str) -> Result<Self, DigestError> {
        Sha256Digest::parse(value).map(Self)
    }

    /// Returns the underlying canonical digest.
    #[must_use]
    pub const fn digest(self) -> Sha256Digest {
        self.0
    }
}

impl fmt::Display for CheckpointKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for CheckpointKey {
    type Err = DigestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// A non-zero monotonically increasing project writer token.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FencingToken(NonZeroU64);

impl FencingToken {
    /// Creates a non-zero token.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError::ZeroFence`] for zero.
    pub fn new(value: u64) -> Result<Self, KeyError> {
        NonZeroU64::new(value).map(Self).ok_or(KeyError::ZeroFence)
    }

    /// Returns the durable integer value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0.get()
    }
}

/// Durable state text stored for one execution attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunState {
    /// Intent exists but execution has not started.
    Pending,
    /// A supervised worker owns execution.
    Running,
    /// Cancellation is durably propagating.
    Cancelling,
    /// Restart reconciliation is in progress.
    Recovering,
    /// Result checkpoint and terminal state committed.
    Succeeded,
    /// Execution or checkpoint publication failed.
    Failed,
    /// Explicit cancellation completed.
    Cancelled,
    /// A prior worker cannot be resumed.
    Interrupted,
}

/// Durable state text stored for one cell attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellState {
    /// The cell waits behind a durable intent.
    Queued,
    /// The cell is executing.
    Running,
    /// Restart reconciliation is in progress.
    Recovering,
    /// Cell result and checkpoint committed.
    Succeeded,
    /// Cell execution failed.
    Failed,
    /// Cell execution was cancelled.
    Cancelled,
    /// The worker disappeared.
    Interrupted,
}

/// Durable workspace transaction state text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionState {
    /// Intent and fence exist; baseline publication is incomplete.
    Prepared,
    /// Baseline is durable and execution may start.
    Active,
    /// Result checkpoint and success committed.
    Committed,
    /// No successful result was published.
    Abandoned,
    /// Workspace consistency could not be proven.
    Conflict,
}

/// Durable worker output stream text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
    /// Reduced Jupyter display output.
    Display,
}

/// Durable checkpoint backend text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointBackend {
    /// Pure filesystem metadata with the common CAS manifest.
    NonGit,
    /// Read-only Git metadata around the same CAS scan.
    GitAware,
}

/// Durable checkpoint restore-fidelity text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RollbackFidelity {
    /// The bounded scan represents every included manifest path.
    FullManifest,
    /// Only caller-declared paths can be restored.
    DeclaredPaths,
}

/// Durable checkpoint-entry kind text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointEntryKind {
    /// Immutable regular-file bytes.
    File,
    /// Immutable symbolic-link target text.
    Symlink,
}

/// One immutable object digest and its verified byte length.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlobRef {
    /// Canonical object identity.
    pub digest: Sha256Digest,
    /// Verified object byte length.
    pub length: u64,
}

/// Invalid durable output byte or record limits.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("durable output limits must both be greater than zero")]
pub struct OutputBudgetError;

/// Per-run limits checked atomically when output metadata is appended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputBudget {
    max_bytes: u64,
    max_records: u64,
}

impl OutputBudget {
    /// Creates positive durable output byte and record limits.
    ///
    /// # Errors
    ///
    /// Returns [`OutputBudgetError`] when either limit is zero.
    pub fn new(max_bytes: u64, max_records: u64) -> Result<Self, OutputBudgetError> {
        if max_bytes == 0 || max_records == 0 {
            return Err(OutputBudgetError);
        }
        Ok(Self {
            max_bytes,
            max_records,
        })
    }

    /// Returns the maximum durable output bytes for one run.
    #[must_use]
    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }

    /// Returns the maximum durable output records for one run.
    #[must_use]
    pub const fn max_records(self) -> u64 {
        self.max_records
    }
}

/// One durable project lease returned by storage operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseGrant {
    /// Leased project.
    pub project_id: ProjectKey,
    /// Process instance holding the lease.
    pub owner_id: LeaseOwnerKey,
    /// Monotonic writer token.
    pub fence: FencingToken,
    /// Absolute lease expiry in Unix milliseconds.
    pub expires_at_ms: u64,
}

/// Persistence-facing input for the later begin-intent method group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeginIntent {
    /// Idempotent command identity.
    pub request_id: RequestId,
    /// Canonical digest of the request payload.
    pub request_digest: Sha256Digest,
    /// New run identity.
    pub run_id: RunKey,
    /// New workspace transaction identity.
    pub transaction_id: WorkspaceTransactionKey,
    /// Project identity.
    pub project_id: ProjectKey,
    /// Stable cell identity.
    pub cell_id: CellKey,
    /// Source object metadata before publication.
    pub source: BlobRef,
    /// Digest binding this attempt to a workspace.
    pub workspace_binding: Sha256Digest,
    /// Process instance requesting the lease.
    pub owner_id: LeaseOwnerKey,
    /// Current Unix time in milliseconds.
    pub now_ms: u64,
    /// Requested absolute lease expiry in Unix milliseconds.
    pub expires_at_ms: u64,
}

/// Persistence-facing input for one durable worker output record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppendOutput {
    /// Run receiving the output.
    pub run_id: RunKey,
    /// Current project lease and fencing token.
    pub lease: LeaseGrant,
    /// Positive worker-local idempotency sequence.
    pub worker_sequence: u64,
    /// Output stream classification.
    pub stream: OutputStream,
    /// Immutable output object metadata.
    pub blob: BlobRef,
    /// Current Unix time in milliseconds.
    pub now_ms: u64,
    /// Durable aggregate byte and record limits.
    pub budget: OutputBudget,
}

/// Persistence-facing input for one successful terminal transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinishSuccess {
    /// Run receiving the successful result.
    pub run_id: RunKey,
    /// Current project lease and fencing token.
    pub lease: LeaseGrant,
    /// Result checkpoint already constructed and published outside SQLite.
    pub result: CheckpointRecord,
    /// Execution-environment digest observed by the worker.
    pub environment: Sha256Digest,
    /// Positive worker kernel generation.
    pub kernel_generation: u64,
    /// Current Unix time in milliseconds.
    pub now_ms: u64,
}

/// Persistence-facing input for a closed non-success terminal transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinishTerminal {
    /// Run receiving the terminal result.
    pub run_id: RunKey,
    /// Current project lease and fencing token.
    pub lease: LeaseGrant,
    /// Closed non-success outcome classification.
    pub disposition: FinishDisposition,
    /// Stable terminal reason code supplied by the runtime.
    pub code: String,
    /// Optional execution-environment digest observed by the worker.
    pub environment: Option<Sha256Digest>,
    /// Optional positive worker kernel generation.
    pub kernel_generation: Option<u64>,
    /// Current Unix time in milliseconds.
    pub now_ms: u64,
}

/// Persistence-facing result of the later begin-intent method group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeginIntentResult {
    /// Current durable run record.
    pub run: RunRecord,
    /// Current project lease.
    pub lease: LeaseGrant,
    /// Whether an existing idempotent request was replayed.
    pub replayed: bool,
}

/// Persistence-facing durable run record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    /// Run identity.
    pub run_id: RunKey,
    /// Project identity.
    pub project_id: ProjectKey,
    /// Workspace transaction identity.
    pub transaction_id: WorkspaceTransactionKey,
    /// Stable cell identity.
    pub cell_id: CellKey,
    /// Positive source revision for the stable cell.
    pub cell_revision: u64,
    /// Lease under which this run was written.
    pub lease: LeaseGrant,
    /// Workspace binding digest.
    pub workspace_binding: Sha256Digest,
    /// Durable run state.
    pub state: RunState,
    /// Durable cell state.
    pub cell_state: CellState,
    /// Durable workspace transaction state.
    pub transaction_state: TransactionState,
    /// Source object metadata.
    pub source: BlobRef,
    /// Whether source bytes have a durable object reference.
    pub source_is_published: bool,
    /// Optional baseline checkpoint.
    pub baseline: Option<CheckpointKey>,
    /// Optional result checkpoint.
    pub result: Option<CheckpointKey>,
    /// Optional execution-environment digest.
    pub environment: Option<Sha256Digest>,
    /// Optional positive worker kernel generation.
    pub kernel_generation: Option<u64>,
    /// Optional bounded terminal code.
    pub terminal_code: Option<String>,
    /// Last durable event sequence.
    pub last_sequence: u64,
    /// Positive optimistic record revision.
    pub revision: u64,
    /// Creation Unix time in milliseconds.
    pub created_at_ms: u64,
    /// Last update Unix time in milliseconds.
    pub updated_at_ms: u64,
}

/// Persistence-facing durable event record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredEvent {
    /// Positive run-local event sequence.
    pub sequence: u64,
    /// Bounded event kind.
    pub kind: String,
    /// Optional positive worker-local sequence.
    pub worker_sequence: Option<u64>,
    /// Optional output stream.
    pub stream: Option<OutputStream>,
    /// Optional immutable payload reference.
    pub blob: Option<BlobRef>,
    /// Event Unix time in milliseconds.
    pub occurred_at_ms: u64,
}

/// Persistence-facing checkpoint entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointEntry {
    /// Canonical project-relative path text.
    pub path: String,
    /// Stored entry kind.
    pub kind: CheckpointEntryKind,
    /// Immutable entry object.
    pub object: BlobRef,
    /// Executable metadata bit.
    pub is_executable: bool,
}

/// Persistence-facing checkpoint aggregate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointRecord {
    /// Content-derived checkpoint identity.
    pub id: CheckpointKey,
    /// Immutable manifest object.
    pub manifest: BlobRef,
    /// Metadata backend.
    pub backend: CheckpointBackend,
    /// Honest restore fidelity.
    pub fidelity: RollbackFidelity,
    /// Optional digest of read-only Git context.
    pub git_context: Option<Sha256Digest>,
    /// Canonically ordered checkpoint entries.
    pub entries: Vec<CheckpointEntry>,
    /// Total bytes represented by regular-file entries.
    pub total_file_bytes: u64,
    /// Creation Unix time in milliseconds.
    pub created_at_ms: u64,
}

/// Closed terminal failure disposition accepted by later repository methods.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishDisposition {
    /// Execution failed.
    Failed,
    /// Explicit cancellation completed.
    Cancelled,
    /// Worker or protocol ownership was lost.
    Interrupted,
    /// Workspace restore or publication conflicted.
    Conflict,
}
