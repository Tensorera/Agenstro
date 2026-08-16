//! Bounded project paths, ignore-aware scanning, and read-only Git metadata.
//!
//! Native project-relative paths are validated before entering application
//! code. Scans use ripgrep's maintained `ignore` walker without following
//! links or consulting global Git configuration, and every returned list has
//! file, byte, depth, duration, and entry limits. Git metadata is obtained only
//! through an argv-based bounded [`agentro_process::ProcessSupervisor`].

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod git;
mod path;
mod scan;

pub use git::{
    GitChangeCounts, GitCli, GitHead, GitMetadata, GitMetadataError, GitMetadataPort,
    MAX_GIT_OUTPUT_BYTES,
};
pub use path::{
    MAX_PATH_BYTES, MAX_PATH_COMPONENT_BYTES, MAX_PATH_COMPONENTS, PathPolicy, ProjectPathError,
    ProjectRelativePath,
};
pub use scan::{
    MAX_SCAN_BYTES, MAX_SCAN_DEPTH, MAX_SCAN_DURATION, MAX_SCAN_ENTRIES, ScanBudget, ScanEntry,
    ScanEntryKind, ScanError, ScanResult, scan_project,
};
