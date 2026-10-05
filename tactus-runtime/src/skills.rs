//! Conservative distribution and upgrades of the runtime's bundled skills.
//!
//! Only paths in `bundled_files` are destinations. The manifest records known
//! installed bytes; it never supplies filesystem paths to read or write.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workspace::{
    Workspace, WorkspaceError, directory_is_reparse_point, reject_linked_directories,
};

const MANIFEST: &str = ".tactus/skills-manifest.json";
const LOCK: &str = ".tactus/skills.lock";
const API: &str = "tactus.skills-manifest/v1";
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    api: String,
    /// SHA-256 after CRLF-to-LF normalization, keyed by static bundle paths.
    files: BTreeMap<String, String>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            api: API.to_owned(),
            files: BTreeMap::new(),
        }
    }
}

/// One result, or proposed result when `dry_run` is true.
#[derive(Clone, Debug, Serialize)]
pub struct SkillFileReport {
    /// Workspace-relative, runtime-selected destination.
    pub path: String,
    /// `installed`, `updated`, `unchanged`, `preserved`, or `skipped`.
    pub action: String,
    /// Explanation suitable for human review.
    pub reason: String,
    /// Incoming bundled content for a preserved local file, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate: Option<String>,
}

/// Skill upgrade report. A dry run uses the same action vocabulary as a real run.
#[derive(Clone, Debug, Serialize)]
pub struct SkillUpdateReport {
    /// Versioned report format.
    pub api: &'static str,
    /// Absolute workspace root.
    pub workspace: PathBuf,
    /// True means no file or directory was created or changed.
    pub dry_run: bool,
    /// Stable identifier of this runtime's bundled contents.
    pub bundle_id: String,
    /// Workspace-relative baseline manifest path.
    pub manifest: &'static str,
    /// Whether the manifest is, or would be, created or changed.
    pub manifest_changed: bool,
    /// Per-file outcome in deterministic bundle order.
    pub files: Vec<SkillFileReport>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum InstallMode {
    Initialize,
    Update,
}

/// Return the explicit-only skill payload owned by `.tactus`.
pub(crate) fn bundled_files() -> Vec<(String, String)> {
    let canonical = [
        (
            "tactus/SKILL.md",
            include_str!("../../skills/tactus/SKILL.md"),
        ),
        (
            "tactus/agents/openai.yaml",
            include_str!("../../skills/tactus/agents/openai.yaml"),
        ),
        (
            "tactus/references/commands.md",
            include_str!("../../skills/tactus/references/commands.md"),
        ),
        (
            "tactus/references/outcomes.md",
            include_str!("../../skills/tactus/references/outcomes.md"),
        ),
        (
            "tactus/references/supervision.md",
            include_str!("../../skills/tactus/references/supervision.md"),
        ),
        (
            "motivo/SKILL.md",
            include_str!("../../skills/motivo/SKILL.md"),
        ),
        (
            "motivo/references/methods.md",
            include_str!("../../skills/motivo/references/methods.md"),
        ),
        (
            "motivo/references/runs.md",
            include_str!("../../skills/motivo/references/runs.md"),
        ),
        (
            "motivo/references/tdd.md",
            include_str!("../../skills/motivo/references/tdd.md"),
        ),
        (
            "motivo/assets/report-template.html",
            include_str!("../../motivo-studio/report-template.html"),
        ),
        (
            "segno/SKILL.md",
            include_str!("../../skills/segno/SKILL.md"),
        ),
        (
            "segno/agents/openai.yaml",
            include_str!("../../skills/segno/agents/openai.yaml"),
        ),
        (
            "segno/references/organizer.md",
            include_str!("../../skills/segno/references/organizer.md"),
        ),
        (
            "segno/references/service.md",
            include_str!("../../skills/segno/references/service.md"),
        ),
        (
            "segno/assets/segno.hs",
            include_str!("../../skills/segno/assets/segno.hs"),
        ),
        (
            "segno/assets/010_business.hs",
            include_str!("../../skills/segno/assets/010_business.hs"),
        ),
    ];
    canonical
        .into_iter()
        .map(|(path, content)| (format!(".tactus/skills/{path}"), content.to_owned()))
        .collect()
}

/// Upgrade bundled skills without overwriting locally changed or unknown versions.
/// This does not read or rewrite runtime configuration or business scripts.
pub fn update_skills(start: &Path, dry_run: bool) -> Result<SkillUpdateReport, WorkspaceError> {
    let workspace = Workspace::discover(start)?;
    let _lock = if dry_run {
        None
    } else {
        Some(acquire_lock(&workspace.root)?)
    };
    reconcile(&workspace.root, InstallMode::Update, dry_run)
}

/// Held across both ordinary initialization and skill distribution.
pub(crate) struct SkillLock {
    file: File,
}

impl Drop for SkillLock {
    fn drop(&mut self) {
        // Closing this descriptor alone does not release a Unix flock while a
        // concurrently forked child retains it before exec. End the lock's
        // lifetime with the operation, independently of inherited descriptors.
        let _ = fs4::FileExt::unlock(&self.file);
    }
}

pub(crate) fn acquire_lock(root: &Path) -> Result<SkillLock, WorkspaceError> {
    let control = root.join(".tactus");
    reject_linked_directories(root, &control)?;
    fs::create_dir_all(&control).map_err(WorkspaceError::Io)?;
    let path = root.join(LOCK);
    ensure_plain_or_missing(&path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    no_follow(&mut options);
    let file = options.open(&path).map_err(WorkspaceError::Io)?;
    ensure_plain_metadata(&file.metadata().map_err(WorkspaceError::Io)?, &path)?;
    fs4::FileExt::try_lock(&file)
        .map_err(|_| invalid("another Tactus initialization or skill update is running"))?;
    Ok(SkillLock { file })
}

/// Caller holds the initialization/update lock unless this is a read-only preview.
pub(crate) fn reconcile(
    root: &Path,
    mode: InstallMode,
    dry_run: bool,
) -> Result<SkillUpdateReport, WorkspaceError> {
    let bundled = bundled_files();
    let bundle_id = bundle_digest(&bundled);
    let manifest_path = root.join(MANIFEST);
    let previous_manifest = read_plain(root, &manifest_path, MAX_MANIFEST_BYTES)?;
    let mut manifest = match &previous_manifest {
        Some(bytes) => serde_json::from_slice::<Manifest>(bytes)
            .map_err(|error| invalid(format!("cannot read {MANIFEST}: {error}")))?,
        None => Manifest::default(),
    };
    if manifest.api != API
        || manifest
            .files
            .values()
            .any(|hash| hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(invalid(format!(
            "unsupported or invalid {MANIFEST}; preserve it for manual review"
        )));
    }
    // Never retain unknown manifest keys as write destinations, or copy them into a
    // new baseline. Runtime-owned destinations come exclusively from the bundle.
    manifest
        .files
        .retain(|path, _| bundled.iter().any(|(known, _)| path == known));
    let mut files = Vec::new();
    for (relative, content) in &bundled {
        let path = root.join(relative);
        let mut entry = SkillFileReport {
            path: relative.clone(),
            action: String::new(),
            reason: String::new(),
            candidate: None,
        };
        if let Err(error) =
            reject_linked_directories(root, path.parent().expect("static bundled parent"))
        {
            entry.action = "skipped".to_owned();
            entry.reason = error.to_string();
            files.push(entry);
            continue;
        }
        let current = match read_plain(root, &path, MAX_FILE_BYTES) {
            Ok(value) => value,
            Err(error) => {
                entry.action = "skipped".to_owned();
                entry.reason = error.to_string();
                files.push(entry);
                continue;
            }
        };
        let incoming_hash = content_hash(content.as_bytes());
        let current_hash = current.as_deref().map(content_hash);
        let known = current_hash.as_ref().is_some_and(|hash| {
            manifest.files.get(relative) == Some(hash)
                || legacy_hashes(relative).contains(&hash.as_str())
        });
        if current.is_none() {
            entry.action = "installed".to_owned();
            entry.reason = "bundled file is missing".to_owned();
        } else if current_hash.as_ref() == Some(&incoming_hash) {
            entry.action = "unchanged".to_owned();
            entry.reason = "already matches the bundled version".to_owned();
        } else if mode == InstallMode::Update && known {
            entry.action = "updated".to_owned();
            entry.reason = "matches its recorded baseline or a known legacy release".to_owned();
        } else {
            entry.action = "preserved".to_owned();
            entry.reason = if mode == InstallMode::Initialize {
                "initialization preserves existing files"
            } else {
                "local content differs from known managed versions"
            }
            .to_owned();
        }
        if matches!(entry.action.as_str(), "installed" | "updated") && !dry_run {
            atomic_write(root, &path, content.as_bytes(), current.as_deref())?;
        }
        if matches!(entry.action.as_str(), "installed" | "updated" | "unchanged") {
            manifest.files.insert(relative.clone(), incoming_hash);
        } else if mode == InstallMode::Update {
            entry.candidate = Some(write_candidate(
                root,
                &bundle_id,
                relative,
                content.as_bytes(),
                dry_run,
            )?);
        }
        files.push(entry);
    }
    let mut manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(|error| invalid(error.to_string()))?;
    manifest_bytes.push(b'\n');
    let manifest_changed = previous_manifest.as_deref() != Some(manifest_bytes.as_slice());
    if manifest_changed && !dry_run {
        atomic_write(
            root,
            &manifest_path,
            &manifest_bytes,
            previous_manifest.as_deref(),
        )?;
    }
    Ok(SkillUpdateReport {
        api: "tactus.skills-update/v1",
        workspace: root.to_path_buf(),
        dry_run,
        bundle_id,
        manifest: MANIFEST,
        manifest_changed,
        files,
    })
}

fn write_candidate(
    root: &Path,
    bundle_id: &str,
    relative: &str,
    content: &[u8],
    dry_run: bool,
) -> Result<String, WorkspaceError> {
    let base = format!(".tactus/skill-updates/{bundle_id}/{relative}");
    for index in 0..1000 {
        let candidate = if index == 0 {
            format!("{base}.incoming")
        } else {
            format!("{base}.incoming.{index}")
        };
        let path = root.join(&candidate);
        let current = read_plain(root, &path, MAX_FILE_BYTES)?;
        if let Some(bytes) = current {
            if bytes == content {
                return Ok(candidate);
            }
            continue;
        }
        if !dry_run {
            atomic_write(root, &path, content, None)?;
        }
        return Ok(candidate);
    }
    Err(invalid(
        "too many locally modified incoming skill candidates",
    ))
}

/// Read only plain files, with bounded memory and no Unix link/FIFO following.
fn read_plain(root: &Path, path: &Path, limit: usize) -> Result<Option<Vec<u8>>, WorkspaceError> {
    reject_linked_directories(root, path.parent().expect("known path parent"))?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure_plain_metadata(&metadata, path)?;
            if metadata.len() > limit as u64 {
                return Err(invalid(format!(
                    "file exceeds {limit} bytes: {}",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkspaceError::Io(error)),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let file = options.open(path).map_err(WorkspaceError::Io)?;
    ensure_plain_metadata(&file.metadata().map_err(WorkspaceError::Io)?, path)?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(WorkspaceError::Io)?;
    if bytes.len() > limit {
        return Err(invalid(format!(
            "file exceeds {limit} bytes: {}",
            path.display()
        )));
    }
    Ok(Some(bytes))
}

fn ensure_plain_or_missing(path: &Path) -> Result<(), WorkspaceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure_plain_metadata(&metadata, path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WorkspaceError::Io(error)),
    }
}

fn ensure_plain_metadata(metadata: &fs::Metadata, path: &Path) -> Result<(), WorkspaceError> {
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || directory_is_reparse_point(metadata)
    {
        return Err(invalid(format!(
            "skill path must be a plain file, not a link or special file: {}",
            path.display()
        )));
    }
    Ok(())
}

fn no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
}

/// Stage beside the destination and rename atomically after rechecking the bytes.
fn atomic_write(
    root: &Path,
    path: &Path,
    bytes: &[u8],
    expected: Option<&[u8]>,
) -> Result<(), WorkspaceError> {
    let parent = path.parent().expect("known path parent");
    reject_linked_directories(root, parent)?;
    fs::create_dir_all(parent).map_err(WorkspaceError::Io)?;
    reject_linked_directories(root, parent)?;
    if read_plain(root, path, MAX_MANIFEST_BYTES)?.as_deref() != expected {
        return Err(invalid(format!(
            "file changed during skill update; retry after reviewing {}",
            path.display()
        )));
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(WorkspaceError::Io)?;
    if expected.is_some() {
        temporary
            .as_file()
            .set_permissions(
                fs::metadata(path)
                    .map_err(WorkspaceError::Io)?
                    .permissions(),
            )
            .map_err(WorkspaceError::Io)?;
    }
    temporary.write_all(bytes).map_err(WorkspaceError::Io)?;
    temporary.as_file().sync_all().map_err(WorkspaceError::Io)?;
    if read_plain(root, path, MAX_MANIFEST_BYTES)?.as_deref() != expected {
        return Err(invalid(format!(
            "file changed while staging skill update; retry after reviewing {}",
            path.display()
        )));
    }
    if expected.is_some() {
        temporary
            .persist(path)
            .map_err(|error| WorkspaceError::Io(error.error))?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|error| WorkspaceError::Io(error.error))?;
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::InvalidConfig(message.into())
}

fn content_hash(content: &[u8]) -> String {
    // Git's Windows checkout policy may change only line endings in include_str!
    // payloads. Normalize that one difference; all other local edits stay distinct.
    let normalized: Vec<u8> = content
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| {
            (*byte != b'\r' || content.get(index + 1) != Some(&b'\n')).then_some(*byte)
        })
        .collect();
    format!("{:x}", Sha256::digest(normalized))
}

fn bundle_digest(files: &[(String, String)]) -> String {
    let mut digest = Sha256::new();
    for (path, content) in files {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(content_hash(content.as_bytes()));
    }
    format!("{digest:x}", digest = digest.finalize())
}

/// Normalized hashes of the seven canonical files
/// shipped by `tactus init` at the pre-PR main baseline (34abcba) and the later
/// pre-manifest bundle (3e90ad1). No Git access at runtime.
fn legacy_hashes(path: &str) -> &'static [&'static str] {
    match path {
        ".tactus/skills/tactus/SKILL.md" => &[
            "e7ee3e5cf7b89d74409d87695708cf49746d9600414c1b6e576b3c992e546660",
            "df769b4da6b2f0f410e2da2c26ac0c92dbf10fce78e4033d3ca0e4f325fc6ddf",
        ],
        ".tactus/skills/tactus/references/commands.md" => &[
            "1e792bb9f204c85c7f05f5593f050e84fb4dcaed4c355d7335c33f42d3c95a82",
            "0b4ecaa5ee408b8fc1e680888fc28b09bff9ab239b0c12f4ef0376ce851d6145",
        ],
        ".tactus/skills/tactus/references/outcomes.md" => {
            &["b18fff2d4c053c5096661aca7e47524ece386d84f7f1b394abebd88fc0eb5064"]
        }
        ".tactus/skills/motivo/SKILL.md" => {
            &["986a8a2551c1dcfa63626fe6abf185a55c6aac0764344f4ba454163cf2011ac8"]
        }
        ".tactus/skills/motivo/references/methods.md" => {
            &["db714e821b689b6d7ce7101ed7567e4b7ee2bf3c49c85b124d0c403335611676"]
        }
        ".tactus/skills/motivo/references/runs.md" => {
            &["e183e1457cdfffe1d2203c35f5b83ef756a313f60d560fcc047845354f10816b"]
        }
        ".tactus/skills/motivo/assets/report-template.html" => {
            &["d97d37453f3f51f078055d943115386cf1f6eae005be6656548d7dee4dd4e2ca"]
        }
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::initialize_workspace;

    fn project() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let sdk = temporary.path().join("sdk");
        fs::create_dir(&sdk).expect("sdk directory");
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("sdk manifest");
        let root = temporary.path().join("project");
        initialize_workspace(&root, Some(&sdk)).expect("initialize");
        (temporary, root, sdk)
    }

    fn entry<'a>(report: &'a SkillUpdateReport, path: &str) -> &'a SkillFileReport {
        report
            .files
            .iter()
            .find(|entry| entry.path == path)
            .expect("reported path")
    }

    fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        fn walk(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
            for entry in fs::read_dir(directory).expect("read directory") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    files.insert(
                        path.strip_prefix(root).expect("relative").to_path_buf(),
                        None,
                    );
                    walk(root, &path, files);
                } else {
                    files.insert(
                        path.strip_prefix(root).expect("relative").to_path_buf(),
                        Some(fs::read(path).expect("read file")),
                    );
                }
            }
        }
        let mut result = BTreeMap::new();
        walk(root, root, &mut result);
        result
    }

    #[test]
    fn fresh_initialization_distributes_all_three_skills_and_preserves_repeated_customizations() {
        let (_temporary, root, sdk) = project();
        let bundle = bundled_files();
        assert!(
            bundle
                .iter()
                .all(|(path, _)| path.starts_with(".tactus/skills/"))
        );
        for host in [".codex", ".claude", ".opencode", ".agents"] {
            assert!(!root.join(host).exists(), "{host} must remain caller-owned");
        }
        for (path, content) in &bundle {
            assert_eq!(
                fs::read(root.join(path)).expect("installed file"),
                content.as_bytes(),
                "{path}"
            );
        }
        let manifest_before = fs::read(root.join(MANIFEST)).expect("manifest");
        let manifest: Manifest = serde_json::from_slice(&manifest_before).expect("manifest JSON");
        assert_eq!(manifest.files.len(), bundle.len());
        for (path, content) in &bundle {
            assert_eq!(manifest.files[path], content_hash(content.as_bytes()));
        }
        let customized = [
            ".tactus/skills/tactus/SKILL.md",
            ".tactus/skills/segno/SKILL.md",
        ];
        for path in customized {
            fs::write(root.join(path), "user customization\n").expect("customize");
        }
        initialize_workspace(&root, Some(&sdk)).expect("repeat init");
        assert_eq!(
            fs::read(root.join(MANIFEST)).expect("baseline"),
            manifest_before,
            "repeat init must not bless user edits"
        );
        let report = update_skills(&root, false).expect("update customized workspace");
        for path in customized {
            assert_eq!(entry(&report, path).action, "preserved");
            assert_eq!(
                fs::read_to_string(root.join(path)).expect("local file"),
                "user customization\n"
            );
            assert!(
                root.join(entry(&report, path).candidate.as_ref().expect("candidate"))
                    .is_file()
            );
        }
    }

    #[test]
    fn legacy_release_upgrades_without_a_manifest_including_crlf_checkouts() {
        assert_legacy_upgrade(
            include_str!("../tests-rust/fixtures/skills-legacy/tactus.md"),
            include_str!("../tests-rust/fixtures/skills-legacy/commands.md"),
        );
    }

    #[test]
    fn main_branch_release_upgrades_without_a_manifest_including_crlf_checkouts() {
        // Original `tactus init` bundle at the pre-PR merge base, 34abcba.
        assert_legacy_upgrade(
            include_str!("../tests-rust/fixtures/skills-legacy/main/tactus.md"),
            include_str!("../tests-rust/fixtures/skills-legacy/main/commands.md"),
        );
    }

    #[test]
    fn retired_host_mirrors_are_preserved_and_removed_from_the_managed_manifest() {
        let (_temporary, root, sdk) = project();
        let manifest_path = root.join(MANIFEST);
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(&manifest_path).expect("manifest"))
                .expect("manifest JSON");
        let hosts = [".codex", ".claude", ".opencode", ".agents"];
        for host in hosts {
            let relative = format!("{host}/skills/tactus/SKILL.md");
            let path = root.join(&relative);
            fs::create_dir_all(path.parent().expect("mirror parent")).expect("host directory");
            fs::write(&path, "previous host skill\n").expect("old mirror");
            manifest
                .files
                .insert(relative, content_hash(b"previous host skill\n"));
        }
        fs::write(&manifest_path, serde_json::to_vec(&manifest).expect("JSON"))
            .expect("legacy manifest");
        let before = snapshot(&root);
        let preview = update_skills(&root, true).expect("preview");
        assert!(preview.manifest_changed);
        assert_eq!(
            snapshot(&root),
            before,
            "preview must not alter old mirrors"
        );
        let applied = update_skills(&root, false).expect("update");
        assert!(
            applied
                .files
                .iter()
                .all(|entry| entry.path.starts_with(".tactus/skills/"))
        );
        let managed: Manifest =
            serde_json::from_slice(&fs::read(&manifest_path).expect("updated manifest"))
                .expect("manifest JSON");
        assert!(
            managed
                .files
                .keys()
                .all(|path| path.starts_with(".tactus/skills/"))
        );
        initialize_workspace(&root, Some(&sdk)).expect("repeat init");
        for host in hosts {
            assert_eq!(
                fs::read(root.join(format!("{host}/skills/tactus/SKILL.md")))
                    .expect("preserved mirror"),
                b"previous host skill\n",
                "{host} is no longer a managed destination"
            );
        }
    }

    fn assert_legacy_upgrade(tactus: &str, commands: &str) {
        for crlf in [false, true] {
            let (_temporary, root, _sdk) = project();
            fs::remove_dir_all(root.join(".tactus/skills")).expect("remove new bundle");
            fs::remove_file(root.join(MANIFEST)).expect("remove manifest");
            let bundle = bundled_files();
            let mut legacy_count = 0;
            for (path, current) in &bundle {
                let expected = legacy_hashes(path);
                if expected.is_empty() {
                    continue;
                }
                let legacy = match path.as_str() {
                    ".tactus/skills/tactus/SKILL.md" => tactus.to_owned(),
                    ".tactus/skills/tactus/references/commands.md" => commands.to_owned(),
                    ".tactus/skills/motivo/SKILL.md" => {
                        include_str!("../tests-rust/fixtures/skills-legacy/motivo.md").to_owned()
                    }
                    ".tactus/skills/motivo/references/runs.md" => {
                        include_str!("../tests-rust/fixtures/skills-legacy/motivo-runs.md")
                            .to_owned()
                    }
                    _ => current.clone(),
                };
                let legacy = legacy.replace("\r\n", "\n");
                let legacy = if crlf {
                    legacy.replace('\n', "\r\n")
                } else {
                    legacy
                };
                assert!(
                    expected.contains(&content_hash(legacy.as_bytes()).as_str()),
                    "legacy fixture for {path}"
                );
                fs::create_dir_all(root.join(path).parent().expect("parent")).expect("directory");
                fs::write(root.join(path), legacy).expect("old installed bytes");
                legacy_count += 1;
            }
            assert_eq!(legacy_count, 7);
            let report = update_skills(&root, false).expect("legacy update");
            for path in [
                ".tactus/skills/tactus/SKILL.md",
                ".tactus/skills/tactus/references/commands.md",
            ] {
                assert_eq!(entry(&report, path).action, "updated", "{path}");
            }
            assert!(report.files.iter().all(|entry| entry.candidate.is_none()));
            for (path, content) in bundle {
                assert_eq!(
                    content_hash(&fs::read(root.join(path)).expect("upgraded file")),
                    content_hash(content.as_bytes())
                );
            }
        }
    }

    #[test]
    fn a_recorded_previous_bundle_upgrades_and_manifest_keys_never_select_paths() {
        let (_temporary, root, _sdk) = project();
        let path = ".tactus/skills/tactus/references/supervision.md";
        let prior = b"previously bundled guidance\n";
        fs::write(root.join(path), prior).expect("prior version");
        let mut manifest: Manifest =
            serde_json::from_slice(&fs::read(root.join(MANIFEST)).expect("manifest"))
                .expect("JSON");
        manifest.files.insert(path.to_owned(), content_hash(prior));
        manifest.files.insert(
            ".tactus/scripts/010_business.hs".to_owned(),
            content_hash(prior),
        );
        manifest
            .files
            .insert("../../outside".to_owned(), content_hash(prior));
        fs::write(root.join(".tactus/scripts/010_business.hs"), prior).expect("business file");
        fs::write(
            root.join(MANIFEST),
            serde_json::to_vec(&manifest).expect("JSON"),
        )
        .expect("prior manifest");
        let report = update_skills(&root, false).expect("upgrade");
        assert_eq!(entry(&report, path).action, "updated");
        assert_eq!(
            fs::read(root.join(".tactus/scripts/010_business.hs")).expect("business preserved"),
            prior
        );
        let updated: Manifest =
            serde_json::from_slice(&fs::read(root.join(MANIFEST)).expect("manifest"))
                .expect("JSON");
        assert!(!updated.files.contains_key("../../outside"));
        assert!(
            !updated
                .files
                .contains_key(".tactus/scripts/010_business.hs")
        );
    }

    #[test]
    fn unknown_files_get_review_candidates_and_dry_run_creates_nothing() {
        let (_temporary, root, _sdk) = project();
        let path = ".tactus/skills/segno/SKILL.md";
        fs::write(root.join(path), "unknown installed version\n").expect("unknown skill");
        fs::remove_file(root.join(".tactus/skills/segno/agents/openai.yaml"))
            .expect("missing canonical asset");
        fs::remove_file(root.join(MANIFEST)).expect("no baseline");
        fs::remove_file(root.join(LOCK)).expect("no lock file");
        for path in [
            ".tactus/tactus.toml",
            ".tactus/PROMPT.md",
            ".tactus/segno.hs",
            ".tactus/scripts/010_business.hs",
            ".tactus/motivo/index.html",
            ".tactus/skills/segno/local.md",
        ] {
            fs::write(root.join(path), "custom project content\n").expect("custom file");
        }
        let before = snapshot(&root);
        let preview = update_skills(&root, true).expect("preview");
        assert!(preview.dry_run);
        assert_eq!(snapshot(&root), before);
        assert_eq!(entry(&preview, path).action, "preserved");
        let report = update_skills(&root, false).expect("apply");
        let candidate = entry(&report, path)
            .candidate
            .as_ref()
            .expect("incoming path");
        assert_eq!(entry(&preview, path).candidate.as_ref(), Some(candidate));
        let incoming = fs::read(root.join(candidate)).expect("incoming content");
        assert_eq!(
            content_hash(&incoming),
            content_hash(include_bytes!("../../skills/segno/SKILL.md"))
        );
        for (path, content) in before {
            if let Some(content) = content {
                assert_eq!(fs::read(root.join(path)).expect("existing file"), content);
            }
        }
        fs::write(root.join(candidate), "human review notes\n").expect("edit candidate");
        let repeated = update_skills(&root, false).expect("repeat update");
        let next = entry(&repeated, path)
            .candidate
            .as_ref()
            .expect("second candidate");
        assert_ne!(next, candidate);
        assert_eq!(
            fs::read_to_string(root.join(candidate)).expect("review preserved"),
            "human review notes\n"
        );
        assert_eq!(fs::read(root.join(next)).expect("new candidate"), incoming);
    }

    #[test]
    fn initialization_and_updates_share_the_same_exclusive_lock() {
        let (_temporary, root, sdk) = project();
        let _lock = acquire_lock(&root).expect("hold lock");
        assert!(
            update_skills(&root, false)
                .expect_err("update locked")
                .to_string()
                .contains("another Tactus")
        );
        assert!(
            initialize_workspace(&root, Some(&sdk))
                .expect_err("init locked")
                .to_string()
                .contains("another Tactus")
        );
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_lock_releases_it_while_a_duplicate_descriptor_remains_open() {
        let (_temporary, root, _sdk) = project();
        let lock = acquire_lock(&root).expect("hold lock");
        // A forked child retains the same open file description until exec.
        // Duplication reproduces that lifetime without a timing-dependent fork.
        let _inherited_descriptor = lock.file.try_clone().expect("duplicate descriptor");
        drop(lock);
        let _next_lock = acquire_lock(&root).expect("previous operation released its lock");
    }

    #[cfg(unix)]
    #[test]
    fn linked_and_special_skill_paths_are_skipped_without_reading_or_writing_through_them() {
        use std::os::unix::fs::symlink;
        let (temporary, root, _sdk) = project();
        let outside = temporary.path().join("outside");
        fs::create_dir(&outside).expect("outside directory");
        let target = outside.join("user.md");
        fs::write(&target, "outside content").expect("outside file");
        let link = ".tactus/skills/tactus/SKILL.md";
        fs::remove_file(root.join(link)).expect("remove skill");
        symlink(&target, root.join(link)).expect("skill link");
        symlink(&outside, root.join(".claude")).expect("host link");
        let fifo = ".tactus/skills/segno/assets/segno.hs";
        fs::remove_file(root.join(fifo)).expect("remove asset");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join(fifo))
                .status()
                .expect("mkfifo")
                .success()
        );
        let report = update_skills(&root, false).expect("skip unsafe files");
        assert_eq!(entry(&report, link).action, "skipped");
        assert_eq!(entry(&report, fifo).action, "skipped");
        assert!(
            report
                .files
                .iter()
                .all(|entry| entry.path.starts_with(".tactus/skills/"))
        );
        assert_eq!(
            fs::read_to_string(target).expect("outside file"),
            "outside content"
        );
        assert_eq!(fs::read_dir(outside).expect("outside directory").count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_metadata_is_rejected_and_oversized_skills_are_bounded() {
        use std::os::unix::fs::symlink;
        let (temporary, root, _sdk) = project();
        let path = ".tactus/skills/segno/SKILL.md";
        fs::write(root.join(path), vec![b'x'; MAX_FILE_BYTES + 1]).expect("oversized file");
        assert_eq!(
            entry(&update_skills(&root, true).expect("preview"), path).action,
            "skipped"
        );
        fs::remove_file(root.join(MANIFEST)).expect("remove manifest");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join(MANIFEST))
                .status()
                .expect("mkfifo")
                .success()
        );
        assert!(
            update_skills(&root, false)
                .expect_err("FIFO rejected")
                .to_string()
                .contains("plain file")
        );
        fs::remove_file(root.join(MANIFEST)).expect("remove FIFO");
        let outside = temporary.path().join("manifest.json");
        fs::write(&outside, "user-owned metadata").expect("outside manifest");
        symlink(&outside, root.join(MANIFEST)).expect("manifest link");
        assert!(
            update_skills(&root, false)
                .expect_err("link rejected")
                .to_string()
                .contains("plain file")
        );
        assert_eq!(
            fs::read_to_string(outside).expect("outside manifest"),
            "user-owned metadata"
        );
        fs::remove_file(root.join(LOCK)).expect("remove lock");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(root.join(LOCK))
                .status()
                .expect("mkfifo")
                .success()
        );
        assert!(
            update_skills(&root, false)
                .expect_err("lock FIFO rejected")
                .to_string()
                .contains("plain file")
        );
    }
}
