use std::{
    ffi::OsStr,
    fs, io,
    path::Path,
    time::{Duration, Instant},
};

use agentro_process::CancellationToken;
use ignore::WalkBuilder;
use thiserror::Error;

use crate::{PathPolicy, ProjectPathError, ProjectRelativePath};

/// Hard maximum entries examined and returned by one scan.
pub const MAX_SCAN_ENTRIES: u64 = 1_000_000;
/// Hard maximum regular-file bytes represented by one scan.
pub const MAX_SCAN_BYTES: u64 = 1 << 40;
/// Hard maximum recursive depth.
pub const MAX_SCAN_DEPTH: usize = 256;
/// Hard maximum scan duration.
pub const MAX_SCAN_DURATION: Duration = Duration::from_secs(60 * 60);

/// Invalid scan configuration, cancellation, budget, or filesystem failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ScanError {
    /// Scan roots must be absolute directories.
    #[error("scan root must be an absolute directory")]
    InvalidRoot,
    /// A budget was zero or above its hard maximum.
    #[error("invalid scan budget: {field}")]
    InvalidBudget {
        /// Name of the invalid field.
        field: &'static str,
    },
    /// Caller cancellation was observed between entries.
    #[error("workspace scan cancelled")]
    Cancelled,
    /// The scan duration budget elapsed.
    #[error("workspace scan duration budget exceeded")]
    DurationBudgetExceeded,
    /// Entry count exceeded the selected budget.
    #[error("workspace scan entry budget {maximum} exceeded")]
    EntryBudgetExceeded {
        /// Maximum examined entries.
        maximum: u64,
    },
    /// One regular file exceeded its selected byte budget.
    #[error("workspace scan single-file budget {maximum} exceeded")]
    FileBudgetExceeded {
        /// Maximum single-file bytes.
        maximum: u64,
    },
    /// Aggregate regular-file bytes exceeded the selected budget.
    #[error("workspace scan total byte budget {maximum} exceeded")]
    TotalBytesBudgetExceeded {
        /// Maximum aggregate file bytes.
        maximum: u64,
    },
    /// A yielded path violated the selected project-relative policy.
    #[error("workspace entry path is invalid")]
    Path {
        /// Underlying path validation error.
        #[source]
        source: ProjectPathError,
    },
    /// The ignore-aware walker failed.
    #[error("ignore-aware workspace walk failed")]
    Walk {
        /// Underlying walker error.
        #[source]
        source: ignore::Error,
    },
    /// Metadata changed, disappeared, or became unreadable during scan.
    #[error("workspace entry metadata failed")]
    Metadata {
        /// Underlying filesystem error.
        #[source]
        source: io::Error,
    },
}

/// Caller-selected scan bounds under hard maxima.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanBudget {
    max_entries: u64,
    max_total_bytes: u64,
    max_file_bytes: u64,
    max_depth: usize,
    max_duration: Duration,
}

impl ScanBudget {
    /// Constructs a complete bounded scan budget.
    ///
    /// # Errors
    ///
    /// Returns [`ScanError::InvalidBudget`] for zero or excessive values.
    pub fn new(
        max_entries: u64,
        max_total_bytes: u64,
        max_file_bytes: u64,
        max_depth: usize,
        max_duration: Duration,
    ) -> Result<Self, ScanError> {
        if max_entries == 0 || max_entries > MAX_SCAN_ENTRIES {
            return Err(ScanError::InvalidBudget { field: "entries" });
        }
        if max_total_bytes == 0 || max_total_bytes > MAX_SCAN_BYTES {
            return Err(ScanError::InvalidBudget {
                field: "total bytes",
            });
        }
        if max_file_bytes == 0 || max_file_bytes > max_total_bytes {
            return Err(ScanError::InvalidBudget {
                field: "single-file bytes",
            });
        }
        if max_depth == 0 || max_depth > MAX_SCAN_DEPTH {
            return Err(ScanError::InvalidBudget { field: "depth" });
        }
        if max_duration.is_zero() || max_duration > MAX_SCAN_DURATION {
            return Err(ScanError::InvalidBudget { field: "duration" });
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

/// Filesystem kind retained in a scan result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ScanEntryKind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link or Windows reparse link, not followed.
    Symlink,
}

/// One bounded scan result entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanEntry {
    path: ProjectRelativePath,
    kind: ScanEntryKind,
    file_bytes: u64,
}

impl ScanEntry {
    /// Returns the validated project-relative native path.
    #[must_use]
    pub fn path(&self) -> &ProjectRelativePath {
        &self.path
    }

    /// Returns the discovered entry kind.
    #[must_use]
    pub fn kind(&self) -> ScanEntryKind {
        self.kind
    }

    /// Returns regular-file bytes, or zero for directories and links.
    #[must_use]
    pub fn file_bytes(&self) -> u64 {
        self.file_bytes
    }
}

/// Complete bounded scan output and resource counters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanResult {
    entries: Vec<ScanEntry>,
    total_file_bytes: u64,
    skipped_special_entries: u64,
}

impl ScanResult {
    /// Returns entries in deterministic per-directory filename order.
    #[must_use]
    pub fn entries(&self) -> &[ScanEntry] {
        &self.entries
    }

    /// Returns aggregate represented regular-file bytes.
    #[must_use]
    pub fn total_file_bytes(&self) -> u64 {
        self.total_file_bytes
    }

    /// Returns FIFO/socket/device entries deliberately omitted from output.
    #[must_use]
    pub fn skipped_special_entries(&self) -> u64 {
        self.skipped_special_entries
    }
}

/// Performs one synchronous ignore-aware, no-follow project scan.
///
/// The scanner does not read global Git ignore configuration or parent ignore
/// files. It honors project `.gitignore`, `.ignore`, and `.agentroignore` files
/// and always prunes known Agentro state plus `.git`, `node_modules`, `.venv`,
/// `build`, and `target` directories.
///
/// # Errors
///
/// Returns typed cancellation, budget, path, walker, or metadata failures.
pub fn scan_project(
    root: &Path,
    path_policy: PathPolicy,
    budget: ScanBudget,
    cancellation: &CancellationToken,
) -> Result<ScanResult, ScanError> {
    if !root.is_absolute() {
        return Err(ScanError::InvalidRoot);
    }
    let root_metadata = fs::symlink_metadata(root).map_err(|_| ScanError::InvalidRoot)?;
    if !root_metadata.file_type().is_dir() {
        return Err(ScanError::InvalidRoot);
    }
    let started = Instant::now();
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
        .filter_entry(|entry| !is_always_ignored_directory(entry));

    let mut visited = 0_u64;
    let mut total_file_bytes = 0_u64;
    let mut skipped_special_entries = 0_u64;
    let mut entries = Vec::new();
    for item in builder.build() {
        if cancellation.is_cancelled() {
            return Err(ScanError::Cancelled);
        }
        if started.elapsed() > budget.max_duration {
            return Err(ScanError::DurationBudgetExceeded);
        }
        let entry = item.map_err(|source| ScanError::Walk { source })?;
        if entry.depth() == 0 {
            continue;
        }
        visited = visited
            .checked_add(1)
            .ok_or(ScanError::EntryBudgetExceeded {
                maximum: budget.max_entries,
            })?;
        if visited > budget.max_entries {
            return Err(ScanError::EntryBudgetExceeded {
                maximum: budget.max_entries,
            });
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| ScanError::InvalidRoot)?;
        let path = ProjectRelativePath::parse(relative, path_policy)
            .map_err(|source| ScanError::Path { source })?;
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|source| ScanError::Metadata { source })?;
        let file_type = metadata.file_type();
        let (kind, file_bytes) = if file_type.is_file() {
            if metadata.len() > budget.max_file_bytes {
                return Err(ScanError::FileBudgetExceeded {
                    maximum: budget.max_file_bytes,
                });
            }
            total_file_bytes = total_file_bytes.checked_add(metadata.len()).ok_or(
                ScanError::TotalBytesBudgetExceeded {
                    maximum: budget.max_total_bytes,
                },
            )?;
            if total_file_bytes > budget.max_total_bytes {
                return Err(ScanError::TotalBytesBudgetExceeded {
                    maximum: budget.max_total_bytes,
                });
            }
            (ScanEntryKind::File, metadata.len())
        } else if file_type.is_dir() {
            (ScanEntryKind::Directory, 0)
        } else if file_type.is_symlink() {
            (ScanEntryKind::Symlink, 0)
        } else {
            skipped_special_entries = skipped_special_entries.saturating_add(1);
            continue;
        };
        entries.push(ScanEntry {
            path,
            kind,
            file_bytes,
        });
    }
    Ok(ScanResult {
        entries,
        total_file_bytes,
        skipped_special_entries,
    })
}

fn is_always_ignored_directory(entry: &ignore::DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return false;
    }
    matches!(
        entry.file_name().to_str(),
        Some(
            ".git"
                | ".agentro"
                | ".agentro-state"
                | ".clef-state"
                | ".tactus"
                | "node_modules"
                | ".venv"
                | "build"
                | "target"
        )
    )
}

#[cfg(test)]
mod tests {
    use std::{error::Error, fs, path::PathBuf, time::Duration};

    use agentro_process::CancellationSource;
    use tempfile::tempdir;

    use super::*;

    fn policy() -> Result<PathPolicy, ProjectPathError> {
        PathPolicy::new(32, 255, 4_096)
    }

    fn budget(entries: u64, bytes: u64) -> Result<ScanBudget, ScanError> {
        ScanBudget::new(entries, bytes, bytes, 16, Duration::from_secs(5))
    }

    #[test]
    fn project_ignore_files_and_builtin_prunes_are_honored() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let root = temporary.path();
        fs::write(root.join(".gitignore"), "ignored.txt\n")?;
        fs::write(root.join(".ignore"), "ignored-dir/\n")?;
        fs::write(root.join(".agentroignore"), "*.tmp\n")?;
        fs::write(root.join("keep.txt"), "keep")?;
        fs::write(root.join("ignored.txt"), "ignored")?;
        fs::write(root.join("scratch.tmp"), "ignored")?;
        fs::create_dir(root.join("ignored-dir"))?;
        fs::write(root.join("ignored-dir").join("inside.txt"), "ignored")?;
        fs::create_dir(root.join("node_modules"))?;
        fs::write(root.join("node_modules").join("package.js"), "ignored")?;
        fs::create_dir(root.join(".git"))?;
        fs::write(root.join(".git").join("config"), "ignored")?;

        let result = scan_project(
            root,
            policy()?,
            budget(32, 1_024)?,
            &CancellationToken::new(),
        )?;
        let paths: Vec<PathBuf> = result
            .entries()
            .iter()
            .map(|entry| entry.path().as_path().to_path_buf())
            .collect();
        assert!(paths.contains(&PathBuf::from("keep.txt")));
        assert!(!paths.contains(&PathBuf::from("ignored.txt")));
        assert!(!paths.iter().any(|path| path.starts_with("node_modules")));
        assert!(!paths.iter().any(|path| path.starts_with(".git")));
        Ok(())
    }

    #[test]
    fn file_and_entry_budgets_fail_closed() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        fs::write(temporary.path().join("large.bin"), [0_u8; 8])?;
        assert!(matches!(
            scan_project(
                temporary.path(),
                policy()?,
                budget(8, 4)?,
                &CancellationToken::new()
            ),
            Err(ScanError::FileBudgetExceeded { maximum: 4 })
        ));
        fs::write(temporary.path().join("other.bin"), [0_u8; 1])?;
        assert!(matches!(
            scan_project(
                temporary.path(),
                policy()?,
                budget(1, 32)?,
                &CancellationToken::new()
            ),
            Err(ScanError::EntryBudgetExceeded { maximum: 1 })
        ));
        Ok(())
    }

    #[test]
    fn cancellation_is_observed_before_traversal() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let source = CancellationSource::new();
        let token = source.token();
        source.cancel();
        assert!(matches!(
            scan_project(temporary.path(), policy()?, budget(8, 32)?, &token),
            Err(ScanError::Cancelled)
        ));
        Ok(())
    }
}
