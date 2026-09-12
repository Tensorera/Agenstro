//! End-to-end daemon policy tests using a model-free provider and Cabal fixture.

#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tactus_runtime::workspace::{ProviderDefinition, Workspace, initialize_workspace};

struct Project {
    _directory: tempfile::TempDir,
    workspace: Workspace,
    bin: PathBuf,
}

impl Project {
    fn new(provider_mode: &str, retry_authorized: bool) -> Self {
        let directory = tempfile::tempdir().expect("tempdir");
        let sdk = directory.path().join("sdk");
        fs::create_dir_all(&sdk).unwrap();
        fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").unwrap();
        let workspace = initialize_workspace(directory.path().join("project"), Some(&sdk))
            .unwrap()
            .workspace;
        fs::write(
            workspace.scripts_path.join("010_first.hs"),
            "main = pure ()\n",
        )
        .unwrap();
        fs::write(
            workspace.scripts_path.join("020_work.hs"),
            "main = broken\n",
        )
        .unwrap();
        let mut config = workspace.load_config().unwrap();
        config.providers.insert(
            "watcher".to_owned(),
            ProviderDefinition {
                command: vec![
                    env!("CARGO_BIN_EXE_tactus-plugin-fixture").to_owned(),
                    provider_mode.to_owned(),
                ],
                model: None,
                effort: None,
                options: Default::default(),
            },
        );
        // Selecting this independent key must not invoke the default real provider.
        config.daemon.provider = Some("watcher".to_owned());
        if retry_authorized {
            config
                .daemon
                .retry_scripts
                .push(PathBuf::from(".tactus/scripts/020_work.hs"));
        }
        fs::write(&workspace.config_path, toml::to_string(&config).unwrap()).unwrap();
        let bin = directory.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let cabal = bin.join("cabal");
        fs::write(
            &cabal,
            r#"#!/bin/sh
if [ "$1" = build ]; then exit 0; fi
for arg do
    case "$arg" in *.hs) entry="$arg";; esac
done
printf 'attempt\n' >> "$TACTUS_TEST_COUNTS.${TACTUS_WORKFLOW_NAME##*/}"
if [ -n "$TACTUS_TEST_AMBIGUOUS" ]; then
    printf 'malformed diagnostic\n' > "$TACTUS_DIAGNOSTIC_PATH"
    exit 1
fi
case "$(cat "$entry")" in
    *broken*) printf 'synthetic compiler error: unresolved identifier\n' >&2; exit 1;;
esac
exit 0
"#,
        )
        .unwrap();
        fs::set_permissions(&cabal, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            _directory: directory,
            workspace,
            bin,
        }
    }

    fn run(&self, enabled: bool, ambiguous: bool) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tactus"));
        command
            .args(["run", "--root"])
            .arg(&self.workspace.root)
            .args(["--all"]);
        if enabled {
            command.arg("--daemon");
        }
        command.env(
            "PATH",
            std::env::join_paths(
                std::iter::once(self.bin.clone()).chain(std::env::split_paths(
                    &std::env::var_os("PATH").unwrap_or_default(),
                )),
            )
            .unwrap(),
        );
        command.env("TACTUS_TEST_COUNTS", self.workspace.root.join("counts"));
        if ambiguous {
            command.env("TACTUS_TEST_AMBIGUOUS", "1");
        }
        command.output().expect("run fixture")
    }

    fn attempts(&self, entry: &str) -> usize {
        fs::read_to_string(self.workspace.root.join(format!("counts.{entry}")))
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn log(&self) -> String {
        let path = fs::read_dir(&self.workspace.runs_path)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("daemon.jsonl"))
            .find(|path| path.is_file())
            .expect("daemon log");
        fs::read_to_string(path).unwrap()
    }
}

#[test]
fn daemon_repairs_and_reruns_only_failed_selected_entry() {
    let project = Project::new("daemon-repair", true);
    let output = project.run(true, false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(project.attempts("010_first.hs"), 1);
    assert_eq!(project.attempts("020_work.hs"), 2);
    assert_eq!(
        fs::read_to_string(project.workspace.scripts_path.join("020_work.hs")).unwrap(),
        "main = pure ()\n"
    );
    let log = project.log();
    assert!(log.contains("daemon.repair_applied"));
    assert!(!log.contains("\"old\":"));
    assert!(!log.contains("synthetic compiler error"));
    assert!(
        fs::read_to_string(project.workspace.root.join("daemon-prompt.txt"))
            .unwrap()
            .contains("synthetic compiler error")
    );
}

#[test]
fn configured_daemon_policy_does_not_activate_without_task_flag() {
    let project = Project::new("daemon-repair", true);
    let output = project.run(false, false);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(project.attempts("020_work.hs"), 1);
    assert!(!project.workspace.root.join("daemon-prompt.txt").exists());
}

#[test]
fn monitoring_does_not_implicitly_authorize_reruns() {
    let project = Project::new("daemon-repair", false);
    let output = project.run(true, false);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(project.attempts("020_work.hs"), 1);
    assert!(project.log().contains("not explicitly authorized"));
    assert!(!project.workspace.root.join("daemon-prompt.txt").exists());
}

#[test]
fn provider_escalation_and_ambiguous_diagnostics_never_rerun() {
    let project = Project::new("daemon-escalate", true);
    let output = project.run(true, false);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(project.attempts("020_work.hs"), 1);
    assert!(project.log().contains("This failure needs review"));

    let project = Project::new("daemon-repair", true);
    let output = project.run(true, true);
    assert!(!output.status.success());
    assert_eq!(project.attempts("010_first.hs"), 1);
    assert_eq!(project.attempts("020_work.hs"), 0);
    assert!(project.log().contains("outcome unknown"));
    assert!(!project.workspace.root.join("daemon-prompt.txt").exists());
}

#[test]
fn daemon_overrides_require_explicit_activation() {
    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["run", "--all", "--daemon-provider", "watcher"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--daemon"));
}

#[test]
fn retry_authorization_rejects_non_source_and_linked_targets() {
    use std::os::unix::fs::symlink;
    let project = Project::new("daemon-repair", true);
    let entry = project.workspace.scripts_path.join("020_work.hs");
    fs::remove_file(&entry).unwrap();
    let external = project._directory.path().join("external.hs");
    fs::write(&external, "main = broken\n").unwrap();
    symlink(&external, &entry).unwrap();
    let output = project.run(true, false);
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(&external).unwrap(), "main = broken\n");
    assert!(!project.workspace.root.join("daemon-prompt.txt").exists());
    assert_eq!(project.attempts("010_first.hs"), 0);
    assert!(Path::new(&entry).is_symlink());
}
