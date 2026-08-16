use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use segno_core::{
    CronDialect, CronExpression, DstFoldPolicy, DstGapPolicy, IanaTimeZone, MisfirePolicy,
    OverlapPolicy, RetryPolicy, SchedulePolicy, Sha256Digest, TaskId,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;
use zip::ZipArchive;

const MANIFEST_NAME: &str = "segno-flow.json";
const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const HARD_MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
const HARD_MAX_ENTRIES: usize = 1_000;
const HARD_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const HARD_MAX_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;
const HARD_MAX_COMPRESSION_RATIO: u64 = 200;
const HARD_MAX_PATH_DEPTH: usize = 32;
const HARD_MAX_COMPONENT_BYTES: usize = 255;
const HARD_MAX_PATH_BYTES: usize = 4_096;

/// Hard resource limits for one ZIP import.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArchiveBudget {
    /// Maximum compressed archive bytes.
    pub max_archive_bytes: u64,
    /// Maximum central-directory entries, including directories.
    pub max_entries: usize,
    /// Maximum expanded bytes for one regular file.
    pub max_file_bytes: u64,
    /// Maximum total expanded regular-file bytes.
    pub max_expanded_bytes: u64,
    /// Maximum expanded-to-compressed ratio for one file.
    pub max_compression_ratio: u64,
    /// Maximum portable path components.
    pub max_path_depth: usize,
    /// Maximum UTF-8 bytes in one path component.
    pub max_component_bytes: usize,
    /// Maximum UTF-8 bytes in one complete member path.
    pub max_path_bytes: usize,
}

impl Default for ArchiveBudget {
    fn default() -> Self {
        Self {
            max_archive_bytes: HARD_MAX_ARCHIVE_BYTES,
            max_entries: HARD_MAX_ENTRIES,
            max_file_bytes: HARD_MAX_FILE_BYTES,
            max_expanded_bytes: HARD_MAX_EXPANDED_BYTES,
            max_compression_ratio: HARD_MAX_COMPRESSION_RATIO,
            max_path_depth: HARD_MAX_PATH_DEPTH,
            max_component_bytes: HARD_MAX_COMPONENT_BYTES,
            max_path_bytes: HARD_MAX_PATH_BYTES,
        }
    }
}

impl ArchiveBudget {
    fn validate(self) -> Result<Self, ArchiveError> {
        if self.max_archive_bytes == 0
            || self.max_entries == 0
            || self.max_file_bytes == 0
            || self.max_expanded_bytes == 0
            || self.max_compression_ratio == 0
            || self.max_path_depth == 0
            || self.max_component_bytes == 0
            || self.max_path_bytes == 0
            || self.max_archive_bytes > HARD_MAX_ARCHIVE_BYTES
            || self.max_entries > HARD_MAX_ENTRIES
            || self.max_file_bytes > HARD_MAX_FILE_BYTES
            || self.max_expanded_bytes > HARD_MAX_EXPANDED_BYTES
            || self.max_compression_ratio > HARD_MAX_COMPRESSION_RATIO
            || self.max_path_depth > HARD_MAX_PATH_DEPTH
            || self.max_component_bytes > HARD_MAX_COMPONENT_BYTES
            || self.max_path_bytes > HARD_MAX_PATH_BYTES
            || self.max_file_bytes > self.max_expanded_bytes
        {
            return Err(ArchiveError::InvalidBudget);
        }
        Ok(self)
    }
}

/// JSON task package manifest accepted by the Rust importer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    /// Manifest schema version. Only version 1 is accepted.
    pub schema_version: u32,
    /// Stable task identity.
    pub id: String,
    /// Human-facing bounded task name.
    pub name: String,
    /// Explicit schedule and recovery policy.
    pub schedule: ScheduleManifest,
    /// Immutable stage paths delegated to Clef/Tactus.
    pub scripts: ScriptsManifest,
}

impl PackageManifest {
    /// Converts the manifest into validated domain values.
    ///
    /// # Errors
    ///
    /// Rejects unsupported schema, malformed IDs/names, stage paths, policy
    /// bounds, or non-IANA timezone envelopes.
    pub fn validate(&self) -> Result<(TaskId, SchedulePolicy), ArchiveError> {
        if self.schema_version != 1 || self.name.is_empty() || self.name.len() > 120 {
            return Err(ArchiveError::ManifestInvalid("schema or task name"));
        }
        let task_id = TaskId::parse(&self.id).map_err(|_| ArchiveError::ManifestInvalid("id"))?;
        let stages = [&self.scripts.pre, &self.scripts.main, &self.scripts.post];
        let mut unique = BTreeSet::new();
        for stage in stages {
            let path = PortablePath::parse(stage, ArchiveBudget::default())?;
            if !stage.ends_with(".py") || !unique.insert(path.collision_key) {
                return Err(ArchiveError::ManifestInvalid("scripts"));
            }
        }
        self.schedule.to_policy().map(|policy| (task_id, policy))
    }
}

/// Explicit persisted schedule policy from a package manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScheduleManifest {
    /// Must be `unix5`.
    pub cron_dialect: String,
    /// Strict five-field cron expression.
    pub cron: String,
    /// Explicit IANA timezone, never `local`.
    pub timezone: String,
    /// `skip` or `next_valid`.
    pub dst_gap: String,
    /// `first`, `second`, or `both`.
    pub dst_fold: String,
    /// Tagged misfire policy.
    pub misfire: MisfireManifest,
    /// Tagged overlap policy.
    pub overlap: OverlapManifest,
    /// Tagged retry policy.
    pub retry: RetryManifest,
    /// Maximum deterministic jitter in seconds.
    pub jitter_seconds: u32,
}

impl ScheduleManifest {
    fn to_policy(&self) -> Result<SchedulePolicy, ArchiveError> {
        if self.cron_dialect != "unix5" || self.jitter_seconds > 86_400 {
            return Err(ArchiveError::ManifestInvalid("schedule"));
        }
        let dst_gap = match self.dst_gap.as_str() {
            "skip" => DstGapPolicy::Skip,
            "next_valid" => DstGapPolicy::NextValid,
            _ => return Err(ArchiveError::ManifestInvalid("dst_gap")),
        };
        let dst_fold = match self.dst_fold.as_str() {
            "first" => DstFoldPolicy::First,
            "second" => DstFoldPolicy::Second,
            "both" => DstFoldPolicy::Both,
            _ => return Err(ArchiveError::ManifestInvalid("dst_fold")),
        };
        Ok(SchedulePolicy {
            dialect: CronDialect::UnixFiveField,
            cron: CronExpression::parse(&self.cron)
                .map_err(|_| ArchiveError::ManifestInvalid("cron"))?,
            timezone: IanaTimeZone::parse(&self.timezone)
                .map_err(|_| ArchiveError::ManifestInvalid("timezone"))?,
            dst_gap,
            dst_fold,
            misfire: self.misfire.to_policy()?,
            overlap: self.overlap.to_policy()?,
            retry: self.retry.to_policy()?,
            jitter_ms: u64::from(self.jitter_seconds) * 1_000,
        })
    }
}

/// Tagged manifest form of misfire policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MisfireManifest {
    /// Skip stale instants beyond a grace window.
    Skip {
        /// Grace window in seconds.
        grace_seconds: u32,
    },
    /// Collapse downtime to the newest due instant.
    Coalesce,
    /// Admit a bounded number of newest missed instants.
    BoundedCatchUp {
        /// Maximum catch-up occurrences.
        limit: u16,
    },
}

impl MisfireManifest {
    fn to_policy(&self) -> Result<MisfirePolicy, ArchiveError> {
        match *self {
            Self::Skip { grace_seconds } if grace_seconds <= 86_400 => Ok(MisfirePolicy::Skip {
                grace_ms: u64::from(grace_seconds) * 1_000,
            }),
            Self::Coalesce => Ok(MisfirePolicy::Coalesce),
            Self::BoundedCatchUp { limit } => std::num::NonZeroU16::new(limit)
                .filter(|value| value.get() <= 1_000)
                .map(MisfirePolicy::BoundedCatchUp)
                .ok_or(ArchiveError::ManifestInvalid("misfire limit")),
            _ => Err(ArchiveError::ManifestInvalid("misfire grace")),
        }
    }
}

/// Tagged manifest form of overlap policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OverlapManifest {
    /// Do not overlap task runs.
    Forbid,
    /// Keep one queued successor.
    QueueOne,
    /// Allow a bounded active count.
    AllowWithLimit {
        /// Maximum active runs.
        limit: u16,
    },
}

impl OverlapManifest {
    fn to_policy(&self) -> Result<OverlapPolicy, ArchiveError> {
        match *self {
            Self::Forbid => Ok(OverlapPolicy::Forbid),
            Self::QueueOne => Ok(OverlapPolicy::QueueOne),
            Self::AllowWithLimit { limit } => std::num::NonZeroU16::new(limit)
                .filter(|value| value.get() <= 64)
                .map(OverlapPolicy::AllowWithLimit)
                .ok_or(ArchiveError::ManifestInvalid("overlap limit")),
        }
    }
}

/// Tagged manifest form of dispatch retry policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RetryManifest {
    /// No automatic retry.
    None,
    /// Bounded retry for an explicitly idempotent task.
    BoundedIdempotent {
        /// Total attempts including the first.
        max_attempts: u16,
        /// Delay between attempts in seconds.
        delay_seconds: u32,
    },
}

impl RetryManifest {
    fn to_policy(&self) -> Result<RetryPolicy, ArchiveError> {
        match *self {
            Self::None => Ok(RetryPolicy::None),
            Self::BoundedIdempotent {
                max_attempts,
                delay_seconds,
            } if delay_seconds <= 86_400 => std::num::NonZeroU16::new(max_attempts)
                .filter(|value| value.get() <= 32)
                .map(|max_attempts| RetryPolicy::BoundedIdempotent {
                    max_attempts,
                    delay_ms: u64::from(delay_seconds) * 1_000,
                })
                .ok_or(ArchiveError::ManifestInvalid("retry attempts")),
            _ => Err(ArchiveError::ManifestInvalid("retry delay")),
        }
    }
}

/// Required immutable package stage paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScriptsManifest {
    /// Preparation stage path.
    pub pre: String,
    /// Main stage path.
    pub main: String,
    /// Finalization stage path.
    pub post: String,
}

/// Atomically published immutable package.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedPackage {
    /// Canonical package content digest.
    pub digest: Sha256Digest,
    /// Immutable package directory.
    pub directory: PathBuf,
    /// Validated package manifest.
    pub manifest: PackageManifest,
}

/// ZIP preflight, staging, and immutable publication owner.
pub struct PackageImporter {
    packages_root: PathBuf,
    staging_root: PathBuf,
    budget: ArchiveBudget,
}

impl PackageImporter {
    /// Creates package directories under an absolute local state root.
    ///
    /// # Errors
    ///
    /// Rejects relative roots, invalid budgets, or filesystem failures.
    pub fn new(state_root: &Path, budget: ArchiveBudget) -> Result<Self, ArchiveError> {
        if !state_root.is_absolute() {
            return Err(ArchiveError::StateRootNotAbsolute);
        }
        let budget = budget.validate()?;
        let packages_root = state_root.join("packages").join("sha256");
        let staging_root = state_root.join("packages").join(".staging");
        fs::create_dir_all(&packages_root)?;
        fs::create_dir_all(&staging_root)?;
        Ok(Self {
            packages_root,
            staging_root,
            budget,
        })
    }

    /// Preflights every central-directory entry, extracts with streaming
    /// verification, validates the manifest, and atomically publishes by digest.
    ///
    /// # Errors
    ///
    /// Returns a typed package, path, quota, ZIP, JSON, or filesystem failure.
    pub fn import(&self, archive_path: &Path) -> Result<PublishedPackage, ArchiveError> {
        if archive_path.extension().and_then(|value| value.to_str()) != Some("zip") {
            return Err(ArchiveError::PackageInvalid("package extension"));
        }
        let metadata = fs::metadata(archive_path)?;
        if !metadata.is_file() || metadata.len() > self.budget.max_archive_bytes {
            return Err(ArchiveError::QuotaExceeded("compressed bytes"));
        }
        let file = File::open(archive_path)?;
        let mut archive = ZipArchive::new(file)?;
        let paths = preflight(&mut archive, self.budget)?;
        let staging = tempfile::Builder::new()
            .prefix("import-")
            .tempdir_in(&self.staging_root)?;
        let records = extract(&mut archive, &paths, staging.path(), self.budget)?;
        let manifest_path = staging.path().join(MANIFEST_NAME);
        let manifest_size = fs::metadata(&manifest_path)?.len();
        if manifest_size > MAX_MANIFEST_BYTES {
            return Err(ArchiveError::QuotaExceeded("manifest bytes"));
        }
        let manifest_bytes = fs::read(&manifest_path)?;
        let manifest: PackageManifest = serde_json::from_slice(&manifest_bytes)?;
        let (_task_id, policy) = manifest.validate()?;
        crate::CronEngine::new(&policy)
            .map_err(|_| ArchiveError::ManifestInvalid("cron or timezone"))?;
        for stage in [
            &manifest.scripts.pre,
            &manifest.scripts.main,
            &manifest.scripts.post,
        ] {
            if !staging.path().join(stage).is_file() {
                return Err(ArchiveError::ManifestInvalid("stage file missing"));
            }
        }
        let digest = package_digest(&records);
        let target = self.packages_root.join(digest_hex(digest));
        publish(staging, &target, digest, self.budget)?;
        Ok(PublishedPackage {
            digest,
            directory: target,
            manifest,
        })
    }
}

#[derive(Clone, Debug)]
struct PortablePath {
    components: Vec<String>,
    collision_key: String,
}

impl PortablePath {
    fn parse(value: &str, budget: ArchiveBudget) -> Result<Self, ArchiveError> {
        if value.is_empty()
            || value.len() > budget.max_path_bytes
            || value.starts_with('/')
            || value.starts_with("//")
            || value.contains(['\\', '\0'])
            || value.chars().any(char::is_control)
        {
            return Err(ArchiveError::PathPolicyViolation(value.into()));
        }
        let trimmed = value.strip_suffix('/').unwrap_or(value);
        let components: Vec<String> = trimmed.split('/').map(str::to_owned).collect();
        if components.is_empty() || components.len() > budget.max_path_depth {
            return Err(ArchiveError::PathPolicyViolation(value.into()));
        }
        for component in &components {
            if component.is_empty()
                || matches!(component.as_str(), "." | "..")
                || component.len() > budget.max_component_bytes
                || component.contains(':')
                || component.ends_with(['.', ' '])
                || is_windows_reserved(component)
            {
                return Err(ArchiveError::PathPolicyViolation(value.into()));
            }
        }
        let normalized: String = components.join("/").nfc().collect();
        let collision_key = normalized
            .chars()
            .flat_map(char::to_lowercase)
            .collect::<String>()
            .nfc()
            .collect();
        Ok(Self {
            components,
            collision_key,
        })
    }

    fn relative_path(&self) -> PathBuf {
        self.components.iter().collect()
    }
}

fn is_windows_reserved(component: &str) -> bool {
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

fn preflight(
    archive: &mut ZipArchive<File>,
    budget: ArchiveBudget,
) -> Result<Vec<PortablePath>, ArchiveError> {
    if archive.is_empty() || archive.len() > budget.max_entries {
        return Err(ArchiveError::QuotaExceeded("entry count"));
    }
    let mut total = 0_u64;
    let mut seen = BTreeSet::new();
    let mut paths = Vec::with_capacity(archive.len());
    let mut has_manifest = false;
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index)?;
        let name = entry.name().to_owned();
        let path = PortablePath::parse(&name, budget)?;
        if !seen.insert(path.collision_key.clone()) {
            return Err(ArchiveError::PathCollision(name));
        }
        if entry.encrypted() {
            return Err(ArchiveError::PackageInvalid("encrypted member"));
        }
        let is_directory = entry.is_dir();
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170_000;
            if kind == 0o120_000 {
                return Err(ArchiveError::PackageInvalid("symbolic link"));
            }
            if kind != 0 && kind != 0o100_000 && !(is_directory && kind == 0o040_000) {
                return Err(ArchiveError::PackageInvalid("special file"));
            }
        }
        if !is_directory {
            if entry.size() > budget.max_file_bytes {
                return Err(ArchiveError::QuotaExceeded("single file bytes"));
            }
            total = total
                .checked_add(entry.size())
                .ok_or(ArchiveError::QuotaExceeded("expanded bytes"))?;
            if total > budget.max_expanded_bytes {
                return Err(ArchiveError::QuotaExceeded("expanded bytes"));
            }
            if entry.size() > 0
                && (entry.compressed_size() == 0
                    || entry.size()
                        > entry
                            .compressed_size()
                            .saturating_mul(budget.max_compression_ratio))
            {
                return Err(ArchiveError::QuotaExceeded("compression ratio"));
            }
        }
        has_manifest |= !is_directory && path.components.as_slice() == [MANIFEST_NAME];
        paths.push(path);
    }
    if !has_manifest {
        return Err(ArchiveError::ManifestMissing);
    }
    Ok(paths)
}

#[derive(Clone, Debug)]
struct FileRecord {
    path: String,
    size: u64,
    digest: [u8; 32],
}

fn extract(
    archive: &mut ZipArchive<File>,
    paths: &[PortablePath],
    staging: &Path,
    budget: ArchiveBudget,
) -> Result<Vec<FileRecord>, ArchiveError> {
    let mut records = Vec::with_capacity(paths.len());
    let mut total_written = 0_u64;
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    for (index, portable) in paths.iter().enumerate() {
        let mut entry = archive.by_index(index)?;
        let target = staging.join(portable.relative_path());
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::options().write(true).create_new(true).open(&target)?;
        let mut hasher = Sha256::new();
        let mut written = 0_u64;
        loop {
            let read = entry.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let read_u64 =
                u64::try_from(read).map_err(|_| ArchiveError::QuotaExceeded("file bytes"))?;
            written = written
                .checked_add(read_u64)
                .ok_or(ArchiveError::QuotaExceeded("file bytes"))?;
            total_written = total_written
                .checked_add(read_u64)
                .ok_or(ArchiveError::QuotaExceeded("expanded bytes"))?;
            if written > entry.size()
                || written > budget.max_file_bytes
                || total_written > budget.max_expanded_bytes
            {
                return Err(ArchiveError::QuotaExceeded("streamed expanded bytes"));
            }
            output.write_all(&buffer[..read])?;
            hasher.update(&buffer[..read]);
        }
        if written != entry.size() {
            return Err(ArchiveError::PackageInvalid("member size mismatch"));
        }
        output.sync_all()?;
        records.push(FileRecord {
            path: portable.components.join("/"),
            size: written,
            digest: hasher.finalize().into(),
        });
    }
    records.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(records)
}

fn package_digest(records: &[FileRecord]) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"segno-package-v1\0");
    for record in records {
        hasher.update(
            u64::try_from(record.path.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        hasher.update(record.path.as_bytes());
        hasher.update(record.size.to_be_bytes());
        hasher.update(record.digest);
    }
    Sha256Digest::from_bytes(hasher.finalize().into())
}

fn digest_hex(digest: Sha256Digest) -> String {
    digest.to_string().trim_start_matches("sha256:").to_owned()
}

fn publish(
    staging: TempDir,
    target: &Path,
    digest: Sha256Digest,
    budget: ArchiveBudget,
) -> Result<(), ArchiveError> {
    if target.exists() {
        return verify_existing(target, digest, budget);
    }
    let staging_path = staging.keep();
    match fs::rename(&staging_path, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_dir_all(staging_path)?;
            verify_existing(target, digest, budget)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(staging_path);
            Err(ArchiveError::Io(error))
        }
    }
}

fn verify_existing(
    target: &Path,
    digest: Sha256Digest,
    budget: ArchiveBudget,
) -> Result<(), ArchiveError> {
    if !target.is_dir()
        || target.file_name().and_then(|value| value.to_str()) != Some(digest_hex(digest).as_str())
    {
        Err(ArchiveError::PublishConflict)
    } else if package_digest(&existing_records(target, budget)?) == digest {
        Ok(())
    } else {
        Err(ArchiveError::PublishConflict)
    }
}

fn existing_records(root: &Path, budget: ArchiveBudget) -> Result<Vec<FileRecord>, ArchiveError> {
    let mut pending = vec![(root.to_path_buf(), String::new())];
    let mut records = Vec::new();
    let mut entries = 0_usize;
    let mut total = 0_u64;
    let mut buffer = [0_u8; COPY_BUFFER_BYTES];
    while let Some((directory, prefix)) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            entries = entries
                .checked_add(1)
                .ok_or(ArchiveError::QuotaExceeded("entry count"))?;
            if entries > budget.max_entries {
                return Err(ArchiveError::QuotaExceeded("entry count"));
            }
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| ArchiveError::PublishConflict)?;
            let relative = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let portable = PortablePath::parse(&relative, budget)?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                return Err(ArchiveError::PublishConflict);
            }
            if metadata.is_dir() {
                pending.push((entry.path(), portable.components.join("/")));
                continue;
            }
            if !metadata.is_file() || metadata.len() > budget.max_file_bytes {
                return Err(ArchiveError::PublishConflict);
            }
            total = total
                .checked_add(metadata.len())
                .ok_or(ArchiveError::QuotaExceeded("expanded bytes"))?;
            if total > budget.max_expanded_bytes {
                return Err(ArchiveError::QuotaExceeded("expanded bytes"));
            }
            let mut file = File::open(entry.path())?;
            let mut hasher = Sha256::new();
            let mut read_total = 0_u64;
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                read_total = read_total
                    .checked_add(
                        u64::try_from(read)
                            .map_err(|_| ArchiveError::QuotaExceeded("file bytes"))?,
                    )
                    .ok_or(ArchiveError::QuotaExceeded("file bytes"))?;
                if read_total > metadata.len() {
                    return Err(ArchiveError::PublishConflict);
                }
                hasher.update(&buffer[..read]);
            }
            if read_total != metadata.len() {
                return Err(ArchiveError::PublishConflict);
            }
            records.push(FileRecord {
                path: portable.components.join("/"),
                size: read_total,
                digest: hasher.finalize().into(),
            });
        }
    }
    records.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(records)
}

/// Package import failure with stable category.
#[derive(Debug, Error)]
pub enum ArchiveError {
    /// State root must be absolute.
    #[error("package state root must be absolute")]
    StateRootNotAbsolute,
    /// A hard budget is zero.
    #[error("archive budget is invalid")]
    InvalidBudget,
    /// Generic invalid ZIP/package structure.
    #[error("package is invalid: {0}")]
    PackageInvalid(&'static str),
    /// Portable member path policy failed.
    #[error("portable path policy rejected member: {0}")]
    PathPolicyViolation(String),
    /// Portable case/Unicode-equivalent paths collided.
    #[error("portable member path collides: {0}")]
    PathCollision(String),
    /// A configured archive quota was exceeded.
    #[error("archive quota exceeded: {0}")]
    QuotaExceeded(&'static str),
    /// Root manifest is absent.
    #[error("ZIP root does not contain segno-flow.json")]
    ManifestMissing,
    /// Manifest semantic validation failed.
    #[error("manifest is invalid: {0}")]
    ManifestInvalid(&'static str),
    /// Existing immutable path does not match the intended object.
    #[error("immutable package publish conflict")]
    PublishConflict,
    /// ZIP framing/decompression failed.
    #[error("ZIP decoding failed")]
    Zip(#[from] zip::result::ZipError),
    /// Manifest JSON failed strict Serde decoding.
    #[error("manifest JSON decoding failed")]
    Json(#[from] serde_json::Error),
    /// Filesystem operation failed.
    #[error("package filesystem operation failed")]
    Io(#[from] io::Error),
}
