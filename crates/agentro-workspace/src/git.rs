use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use agentro_process::{
    CancellationToken, OutputBudget, ProcessError, ProcessSpec, ProcessSupervisor, ProcessTimeouts,
    ResourceLimits, TerminationReason,
};
use thiserror::Error;

/// Hard maximum bytes retained from either Git output stream.
pub const MAX_GIT_OUTPUT_BYTES: u64 = 2 * 1_048_576;
const MAX_GIT_RECORDS: u32 = 100_000;
const MAX_GIT_TEXT_BYTES: usize = 1_024;
const MAX_GIT_PATH_RECORD_BYTES: usize = 64 * 1_024;

/// Normalized Git HEAD state from porcelain-v2 branch headers.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GitHead {
    /// Repository has no commit yet.
    Unborn {
        /// Initial branch name when Git reported one.
        branch: Option<String>,
    },
    /// HEAD points directly at an object ID.
    Detached {
        /// Current hexadecimal object ID.
        oid: String,
    },
    /// HEAD points at a named branch.
    Branch {
        /// Branch short name.
        name: String,
        /// Current hexadecimal object ID.
        oid: String,
    },
}

/// Bounded counts of porcelain-v2 worktree records.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GitChangeCounts {
    tracked: u32,
    renamed_or_copied: u32,
    unmerged: u32,
    untracked: u32,
    ignored: u32,
}

impl GitChangeCounts {
    /// Returns ordinary tracked modifications, including rename/copy records.
    #[must_use]
    pub fn tracked(self) -> u32 {
        self.tracked
    }

    /// Returns rename/copy records within the tracked count.
    #[must_use]
    pub fn renamed_or_copied(self) -> u32 {
        self.renamed_or_copied
    }

    /// Returns unmerged records.
    #[must_use]
    pub fn unmerged(self) -> u32 {
        self.unmerged
    }

    /// Returns untracked records.
    #[must_use]
    pub fn untracked(self) -> u32 {
        self.untracked
    }

    /// Returns ignored records requested for metadata fidelity.
    #[must_use]
    pub fn ignored(self) -> u32 {
        self.ignored
    }
}

/// Read-only, normalized repository metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitMetadata {
    head: GitHead,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
    changes: GitChangeCounts,
}

impl GitMetadata {
    /// Returns the normalized HEAD state.
    #[must_use]
    pub fn head(&self) -> &GitHead {
        &self.head
    }

    /// Returns the configured upstream branch reported by status.
    #[must_use]
    pub fn upstream(&self) -> Option<&str> {
        self.upstream.as_deref()
    }

    /// Returns commits ahead of upstream.
    #[must_use]
    pub fn ahead(&self) -> u32 {
        self.ahead
    }

    /// Returns commits behind upstream.
    #[must_use]
    pub fn behind(&self) -> u32 {
        self.behind
    }

    /// Returns bounded worktree record counts.
    #[must_use]
    pub fn changes(&self) -> GitChangeCounts {
        self.changes
    }

    /// Reports tracked, unmerged, or untracked changes; ignored records alone
    /// do not make the repository dirty.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.changes.tracked > 0 || self.changes.unmerged > 0 || self.changes.untracked > 0
    }
}

/// Read-only Git metadata port used by checkpoint application code.
pub trait GitMetadataPort {
    /// Reads one bounded metadata snapshot without modifying `.git`.
    ///
    /// # Errors
    ///
    /// Returns typed root, process, Git exit, output, or parse failures.
    fn read_metadata(
        &self,
        project_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<GitMetadata, GitMetadataError>;
}

/// Git metadata setup, process, bounded-output, or protocol failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GitMetadataError {
    /// Git executable and repository roots must be explicit absolute paths.
    #[error("Git executable and project root must be absolute")]
    PathNotAbsolute,
    /// The selected project root was not a directory.
    #[error("Git project root is not a directory")]
    RootNotDirectory,
    /// Git output/timeout configuration was zero or above its hard limit.
    #[error("invalid Git metadata configuration: {field}")]
    InvalidConfiguration {
        /// Name of the invalid field.
        field: &'static str,
    },
    /// Process specification validation failed.
    #[error("Git process specification is invalid")]
    ProcessSpec {
        /// Underlying process specification error.
        #[source]
        source: agentro_process::SpecError,
    },
    /// Native process supervision failed.
    #[error("Git process supervision failed")]
    Process {
        /// Underlying process supervisor error.
        #[source]
        source: ProcessError,
    },
    /// Git was cancelled, timed out, or crossed an output hard limit.
    #[error("Git metadata process terminated before normal exit: {reason:?}")]
    ProcessTerminated {
        /// Normalized terminal reason.
        reason: TerminationReason,
    },
    /// Git exited non-zero with a bounded diagnostic tail.
    #[error("Git metadata command failed with code {code:?}: {stderr_excerpt}")]
    GitFailed {
        /// Portable exit code, when available.
        code: Option<i32>,
        /// Bounded lossy diagnostic excerpt.
        stderr_excerpt: String,
    },
    /// Porcelain-v2 output was malformed, excessive, or incomplete.
    #[error("Git porcelain-v2 metadata is invalid")]
    InvalidPorcelain,
}

/// Git CLI adapter using a supplied bounded process supervisor.
pub struct GitCli<S> {
    supervisor: S,
    executable: PathBuf,
    environment: BTreeMap<OsString, OsString>,
    timeout: Duration,
    max_output_bytes: u64,
}

impl<S> GitCli<S> {
    /// Constructs a read-only Git metadata adapter.
    ///
    /// The environment is explicit; global and system Git config are disabled
    /// by the adapter, and `GIT_OPTIONAL_LOCKS=0` prevents index refresh writes.
    ///
    /// # Errors
    ///
    /// Returns typed absolute-path, timeout, or output budget errors.
    pub fn new(
        supervisor: S,
        executable: PathBuf,
        environment: BTreeMap<OsString, OsString>,
        timeout: Duration,
        max_output_bytes: u64,
    ) -> Result<Self, GitMetadataError> {
        if !executable.is_absolute() {
            return Err(GitMetadataError::PathNotAbsolute);
        }
        if timeout.is_zero() || timeout > Duration::from_secs(60) {
            return Err(GitMetadataError::InvalidConfiguration { field: "timeout" });
        }
        if max_output_bytes == 0 || max_output_bytes > MAX_GIT_OUTPUT_BYTES {
            return Err(GitMetadataError::InvalidConfiguration {
                field: "output bytes",
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

impl<S: ProcessSupervisor> GitMetadataPort for GitCli<S> {
    fn read_metadata(
        &self,
        project_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<GitMetadata, GitMetadataError> {
        if !project_root.is_absolute() {
            return Err(GitMetadataError::PathNotAbsolute);
        }
        let root_metadata =
            fs::symlink_metadata(project_root).map_err(|_| GitMetadataError::RootNotDirectory)?;
        if !root_metadata.file_type().is_dir() {
            return Err(GitMetadataError::RootNotDirectory);
        }
        let mut environment = sanitized_environment(&self.environment);
        environment.insert("GIT_OPTIONAL_LOCKS".into(), "0".into());
        environment.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
        environment.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        environment.insert("GIT_CONFIG_GLOBAL".into(), null_device().into());

        let arguments = vec![
            OsString::from("--no-optional-locks"),
            OsString::from("-c"),
            OsString::from("core.fsmonitor=false"),
            OsString::from("-c"),
            OsString::from("core.untrackedCache=false"),
            OsString::from("-C"),
            project_root.as_os_str().to_owned(),
            OsString::from("status"),
            OsString::from("--porcelain=v2"),
            OsString::from("--branch"),
            OsString::from("-z"),
            OsString::from("--untracked-files=all"),
            OsString::from("--ignored=matching"),
        ];
        let total_output =
            self.max_output_bytes
                .checked_mul(2)
                .ok_or(GitMetadataError::InvalidConfiguration {
                    field: "output bytes",
                })?;
        let spec = ProcessSpec::new(
            self.executable.clone(),
            arguments,
            project_root.to_path_buf(),
            environment,
            ProcessTimeouts::new(self.timeout, Duration::from_millis(250))
                .map_err(|source| GitMetadataError::ProcessSpec { source })?,
            OutputBudget::new(self.max_output_bytes, self.max_output_bytes, total_output)
                .map_err(|source| GitMetadataError::ProcessSpec { source })?,
            ResourceLimits::default(),
        )
        .map_err(|source| GitMetadataError::ProcessSpec { source })?;
        let output = self
            .supervisor
            .run(spec, cancellation)
            .map_err(|source| GitMetadataError::Process { source })?;
        if output.termination() != TerminationReason::Exited {
            return Err(GitMetadataError::ProcessTerminated {
                reason: output.termination(),
            });
        }
        if !output.success() {
            return Err(GitMetadataError::GitFailed {
                code: output.exit_code(),
                stderr_excerpt: bounded_diagnostic(output.stderr()),
            });
        }
        parse_porcelain_v2(output.stdout())
    }
}

fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

fn sanitized_environment(
    environment: &BTreeMap<OsString, OsString>,
) -> BTreeMap<OsString, OsString> {
    environment
        .iter()
        .filter(|(name, _)| {
            !name
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn bounded_diagnostic(bytes: &[u8]) -> String {
    let start = bytes.len().saturating_sub(MAX_GIT_TEXT_BYTES);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

#[derive(Default)]
struct PorcelainState {
    oid: Option<String>,
    head_name: Option<String>,
    upstream: Option<String>,
    ahead: u32,
    behind: u32,
    changes: GitChangeCounts,
    skip_rename_source: bool,
    records: u32,
    saw_oid: bool,
    saw_head: bool,
}

fn parse_porcelain_v2(output: &[u8]) -> Result<GitMetadata, GitMetadataError> {
    let mut state = PorcelainState::default();

    for record in output.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        if state.skip_rename_source {
            if record.len() > MAX_GIT_PATH_RECORD_BYTES {
                return Err(GitMetadataError::InvalidPorcelain);
            }
            state.skip_rename_source = false;
            continue;
        }
        state.records = checked_record_count(state.records)?;
        match record.first().copied() {
            Some(b'#') => parse_header(record, &mut state)?,
            Some(b'1') if record.starts_with(b"1 ") => {
                state.changes.tracked = checked_record_count(state.changes.tracked)?;
            }
            Some(b'2') if record.starts_with(b"2 ") => {
                state.changes.tracked = checked_record_count(state.changes.tracked)?;
                state.changes.renamed_or_copied =
                    checked_record_count(state.changes.renamed_or_copied)?;
                state.skip_rename_source = true;
            }
            Some(b'u') if record.starts_with(b"u ") => {
                state.changes.unmerged = checked_record_count(state.changes.unmerged)?;
            }
            Some(b'?') if record.starts_with(b"? ") => {
                state.changes.untracked = checked_record_count(state.changes.untracked)?;
            }
            Some(b'!') if record.starts_with(b"! ") => {
                state.changes.ignored = checked_record_count(state.changes.ignored)?;
            }
            _ => return Err(GitMetadataError::InvalidPorcelain),
        }
    }
    if state.skip_rename_source || !state.saw_oid || !state.saw_head {
        return Err(GitMetadataError::InvalidPorcelain);
    }
    let head = match (state.oid, state.head_name) {
        (None, branch) => GitHead::Unborn { branch },
        (Some(oid), Some(name)) if name == "(detached)" => GitHead::Detached { oid },
        (Some(oid), Some(name)) => GitHead::Branch { name, oid },
        (Some(_), None) => return Err(GitMetadataError::InvalidPorcelain),
    };
    Ok(GitMetadata {
        head,
        upstream: state.upstream,
        ahead: state.ahead,
        behind: state.behind,
        changes: state.changes,
    })
}

fn checked_record_count(current: u32) -> Result<u32, GitMetadataError> {
    let next = current
        .checked_add(1)
        .ok_or(GitMetadataError::InvalidPorcelain)?;
    if next > MAX_GIT_RECORDS {
        return Err(GitMetadataError::InvalidPorcelain);
    }
    Ok(next)
}

fn parse_header(record: &[u8], state: &mut PorcelainState) -> Result<(), GitMetadataError> {
    if record.len() > MAX_GIT_TEXT_BYTES {
        return Err(GitMetadataError::InvalidPorcelain);
    }
    let text = std::str::from_utf8(record).map_err(|_| GitMetadataError::InvalidPorcelain)?;
    if let Some(value) = text.strip_prefix("# branch.oid ") {
        if state.saw_oid {
            return Err(GitMetadataError::InvalidPorcelain);
        }
        state.saw_oid = true;
        if value == "(initial)" {
            state.oid = None;
        } else if valid_oid(value) {
            state.oid = Some(value.to_owned());
        } else {
            return Err(GitMetadataError::InvalidPorcelain);
        }
    } else if let Some(value) = text.strip_prefix("# branch.head ") {
        if state.saw_head {
            return Err(GitMetadataError::InvalidPorcelain);
        }
        state.saw_head = true;
        validate_git_text(value)?;
        state.head_name = Some(value.to_owned());
    } else if let Some(value) = text.strip_prefix("# branch.upstream ") {
        validate_git_text(value)?;
        state.upstream = Some(value.to_owned());
    } else if let Some(value) = text.strip_prefix("# branch.ab +") {
        let (ahead_value, behind_value) = value
            .split_once(" -")
            .ok_or(GitMetadataError::InvalidPorcelain)?;
        state.ahead = ahead_value
            .parse()
            .map_err(|_| GitMetadataError::InvalidPorcelain)?;
        state.behind = behind_value
            .parse()
            .map_err(|_| GitMetadataError::InvalidPorcelain)?;
    } else if !text.starts_with("# ") {
        return Err(GitMetadataError::InvalidPorcelain);
    }
    Ok(())
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_git_text(value: &str) -> Result<(), GitMetadataError> {
    if value.is_empty() || value.len() > MAX_GIT_TEXT_BYTES || value.chars().any(char::is_control) {
        return Err(GitMetadataError::InvalidPorcelain);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        env,
        error::Error,
        ffi::OsString,
        fs, io,
        path::{Path, PathBuf},
        process::{Command, Stdio},
        time::Duration,
    };

    use agentro_process::{CancellationToken, NativeProcessSupervisor};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn porcelain_parser_normalizes_branch_and_counts() -> Result<(), GitMetadataError> {
        let output = b"# branch.oid 0123456789012345678901234567890123456789\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0? untracked.txt\0! ignored.tmp\0";
        let metadata = parse_porcelain_v2(output)?;
        assert!(matches!(metadata.head(), GitHead::Branch { name, .. } if name == "main"));
        assert_eq!(metadata.ahead(), 2);
        assert_eq!(metadata.behind(), 1);
        assert_eq!(metadata.changes().untracked(), 1);
        assert_eq!(metadata.changes().ignored(), 1);
        assert!(metadata.is_dirty());
        Ok(())
    }

    #[test]
    fn porcelain_parser_requires_branch_headers_and_valid_record_prefixes() {
        assert!(matches!(
            parse_porcelain_v2(b"? file.txt\0"),
            Err(GitMetadataError::InvalidPorcelain)
        ));
        assert!(matches!(
            parse_porcelain_v2(b"# branch.oid (initial)\0# branch.head main\0?missing-space\0"),
            Err(GitMetadataError::InvalidPorcelain)
        ));
    }

    #[test]
    fn caller_git_redirection_environment_is_removed() {
        let mut input = BTreeMap::new();
        input.insert(OsString::from("GIT_DIR"), OsString::from("elsewhere"));
        input.insert(OsString::from("PATH"), OsString::from("tools"));
        let sanitized = sanitized_environment(&input);
        assert!(!sanitized.contains_key(&OsString::from("GIT_DIR")));
        assert_eq!(
            sanitized.get(&OsString::from("PATH")),
            Some(&OsString::from("tools"))
        );
    }

    #[test]
    fn real_git_metadata_does_not_change_temporary_git_directory() -> Result<(), Box<dyn Error>> {
        let git = locate_git()?;
        let temporary = tempdir()?;
        let root = temporary.path();
        let environment = minimal_test_environment();
        let status = Command::new(&git)
            .arg("init")
            .arg("--quiet")
            .current_dir(root)
            .env_clear()
            .envs(&environment)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", null_device())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            return Err("temporary git init failed".into());
        }
        fs::write(root.join("untracked.txt"), "content")?;
        let before = snapshot_directory(&root.join(".git"))?;

        let adapter = GitCli::new(
            NativeProcessSupervisor::new(),
            git,
            environment,
            Duration::from_secs(5),
            1_048_576,
        )?;
        let metadata = adapter.read_metadata(root, &CancellationToken::new())?;
        let after = snapshot_directory(&root.join(".git"))?;

        assert!(matches!(metadata.head(), GitHead::Unborn { .. }));
        assert_eq!(metadata.changes().untracked(), 1);
        assert_eq!(before, after);
        Ok(())
    }

    fn locate_git() -> Result<PathBuf, io::Error> {
        let path = env::var_os("PATH")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PATH is unavailable"))?;
        for directory in env::split_paths(&path) {
            for name in git_names() {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return candidate.canonicalize();
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "git executable was not found on PATH",
        ))
    }

    fn git_names() -> &'static [&'static str] {
        if cfg!(windows) {
            &["git.exe", "git.cmd"]
        } else {
            &["git"]
        }
    }

    fn minimal_test_environment() -> BTreeMap<OsString, OsString> {
        let mut environment = BTreeMap::new();
        for name in ["PATH", "SystemRoot", "WINDIR", "TMP", "TEMP"] {
            if let Some(value) = env::var_os(name) {
                environment.insert(OsString::from(name), value);
            }
        }
        environment
    }

    fn snapshot_directory(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>, io::Error> {
        let mut pending = vec![root.to_path_buf()];
        let mut snapshot = BTreeMap::new();
        while let Some(directory) = pending.pop() {
            for item in fs::read_dir(&directory)? {
                let item = item?;
                let file_type = item.file_type()?;
                if file_type.is_dir() {
                    pending.push(item.path());
                } else if file_type.is_file() {
                    let relative = item
                        .path()
                        .strip_prefix(root)
                        .map_err(io::Error::other)?
                        .to_path_buf();
                    snapshot.insert(relative, fs::read(item.path())?);
                }
            }
        }
        Ok(snapshot)
    }
}
