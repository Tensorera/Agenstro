use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use serde_json::{Value, json};
use tactus_runtime::workspace::initialize_workspace;

fn project(mode: &str, permissions: Option<Value>) -> (tempfile::TempDir, std::path::PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("clef-sdk");
    let workspace = initialize_workspace(temporary.path().join("project"), Some(&sdk))
        .unwrap()
        .workspace;
    let mut config = workspace.load_config().unwrap();
    let provider = config.providers.get_mut("codex").unwrap();
    provider.command = vec![
        env!("CARGO_BIN_EXE_tactus-plugin-fixture").to_owned(),
        mode.to_owned(),
    ];
    provider.permissions = permissions.map(|value| serde_json::from_value(value).unwrap());
    fs::write(&workspace.config_path, toml::to_string(&config).unwrap()).unwrap();
    (temporary, workspace.root)
}

fn call(root: &Path, params: Value) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "plugin-call",
            "codex",
            "invoke",
            "--namespace",
            "provider",
            "--root",
        ])
        .arg(root)
        .args(["--params", &params.to_string(), "--json"])
        .output()
        .unwrap()
}

fn value(output: &std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["summary"]["outcome"]["terminal"]["value"].clone()
}

fn calls(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("permission-calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn each_call_replaces_registry_permissions_without_mutating_user_files() {
    let defaults = json!({"allowed_tools":["Read"],"allowed_skills":["audit"]});
    let (_temporary, root) = project("permission-capable", Some(defaults.clone()));
    let before = fs::read(root.join(".tactus/tactus.toml")).unwrap();
    let first = value(&call(&root, json!({"prompt":"first"})));
    assert_eq!(first["permissions"], defaults);
    assert_eq!(first["permission_report"]["effective"], defaults);
    let changed = json!({"allowed_tools":[]});
    let second = value(&call(
        &root,
        json!({"prompt":"second","permissions":changed}),
    ));
    assert_eq!(second["permissions"], changed);
    assert!(second["permissions"].get("allowed_skills").is_none());
    let third = value(&call(&root, json!({"permissions":{}})));
    assert!(third.get("permissions").is_none());
    assert_eq!(third["permission_report"]["effective"], json!({}));
    assert_eq!(
        calls(&root)
            .iter()
            .map(|call| call["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["describe", "invoke", "describe", "invoke", "invoke"]
    );
    assert_eq!(before, fs::read(root.join(".tactus/tactus.toml")).unwrap());
    let runtime = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["runtime-json", "--root"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(runtime.status.success());
    let runtime: Value = serde_json::from_slice(&runtime.stdout).unwrap();
    assert_eq!(runtime["providers"]["codex"]["permissions"], defaults);
}

#[test]
fn partial_support_applies_each_field_and_records_fallback_warnings() {
    let (_temporary, root) = project("permission-partial", None);
    let requested = json!({"allowed_tools":["Read"],"allowed_skills":[],"sandbox":{"mode":"read-only","workspace":root}});
    let output = call(&root, json!({"permissions":requested}));
    let received = value(&output);
    let report = &received["permission_report"];
    assert_eq!(report["requested"], requested);
    assert_eq!(
        received["permissions"],
        json!({"allowed_tools":["Read"],"sandbox":{"mode":"read-only"}})
    );
    assert_eq!(report["effective"], received["permissions"]);
    assert_eq!(report["warnings"].as_array().unwrap().len(), 3);
    let text = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "plugin-call",
            "codex",
            "invoke",
            "--namespace",
            "provider",
            "--root",
        ])
        .arg(&root)
        .args([
            "--params",
            &json!({"permissions":{"allowed_skills":[]}}).to_string(),
        ])
        .output()
        .unwrap();
    assert!(text.status.success());
    assert!(String::from_utf8_lossy(&text.stderr).contains("allowed skills were not applied"));
    assert!(!String::from_utf8_lossy(&text.stderr).contains("PRIVATE_PERMISSION_REASON"));
    let trace: String = fs::read_dir(root.join(".tactus/runs"))
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path().join("events.jsonl")).unwrap())
        .collect();
    assert!(trace.contains("provider.permission.warning"));
    assert!(trace.contains("permission_partial"));
    assert!(!trace.contains("PRIVATE_PERMISSION_REASON"));
    assert!(
        report["warnings"]
            .to_string()
            .contains("PRIVATE_PERMISSION_REASON")
    );

    let unsupported = value(&call(
        &root,
        json!({"permissions":{"sandbox":{"mode":"workspace-write"}}}),
    ));
    assert!(unsupported.get("permissions").is_none());
    assert_eq!(
        unsupported["permission_report"]["applications"][0]["status"],
        "unsupported"
    );
}

#[test]
fn legacy_and_invalid_descriptions_fall_back_without_leaking_new_request_fields() {
    for mode in ["permission-legacy", "permission-invalid"] {
        let (_temporary, root) = project(mode, None);
        let result = value(&call(
            &root,
            json!({"permissions":{"allowed_tools":[],"allowed_skills":[],"sandbox":{"mode":"read-only"}}}),
        ));
        assert!(result.get("permissions").is_none());
        assert_eq!(result["permission_report"]["effective"], json!({}));
        assert_eq!(
            result["permission_report"]["warnings"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        let records = calls(&root);
        assert_eq!(records.len(), 2);
        assert!(records[1]["params"].get("permissions").is_none());
    }
}

#[test]
fn malformed_permission_requests_fail_before_any_provider_is_started() {
    let (_temporary, root) = project("permission-capable", None);
    for permissions in [
        json!(null),
        json!({"allowed_tools":"Read"}),
        json!({"sandbox":{"mode":"read-only","workspace":"relative"}}),
        json!({"extra":true}),
    ] {
        let output = call(&root, json!({"permissions":permissions}));
        assert!(!output.status.success());
        assert!(!root.join("permission-calls.jsonl").exists());
    }
}

#[test]
fn dispatch_forwards_warning_frames_before_one_terminal_result() {
    let (_temporary, root) = project("permission-legacy", None);
    let mut child = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "dispatch",
            "--namespace",
            "provider",
            "--name",
            "codex",
            "--root",
        ])
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{}", json!({"api":"agenstro.plugin/v1","id":"one","method":"invoke","params":{"permissions":{"allowed_skills":[]}}})).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frames: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame["type"] == "result")
            .count(),
        1
    );
    assert_eq!(frames[0]["event"]["type"], "provider.permission.warning");
    assert_eq!(
        frames.last().unwrap()["value"]["permission_report"]["warnings"][0]["code"],
        "permission_unsupported"
    );
}

#[test]
fn concurrent_calls_keep_independent_permission_objects() {
    let (_temporary, root) = project("permission-capable", None);
    let first_root = root.clone();
    let first = std::thread::spawn(move || {
        value(&call(
            &first_root,
            json!({"permissions":{"allowed_tools":["Read"],"allowed_skills":["audit"]}}),
        ))
    });
    let second = value(&call(
        &root,
        json!({"permissions":{"allowed_tools":[],"sandbox":{"mode":"read-only"}}}),
    ));
    assert_eq!(
        first.join().unwrap()["permissions"],
        json!({"allowed_tools":["Read"],"allowed_skills":["audit"]})
    );
    assert_eq!(
        second["permissions"],
        json!({"allowed_tools":[],"sandbox":{"mode":"read-only"}})
    );
}

#[test]
#[ignore = "requires GHC/Cabal; run explicitly in cross-language CI"]
fn haskell_permissions_cross_dispatch_without_double_probe() {
    let defaults = json!({"allowed_tools":["Read"],"allowed_skills":["audit"]});
    let (_temporary, root) = project("permission-capable", Some(defaults.clone()));
    fs::write(root.join(".tactus/scripts/010_permissions.hs"), r#"
{-# LANGUAGE OverloadedStrings #-}
module Main (main) where
import Clef
import Data.Aeson (Value, encode)
import qualified Data.ByteString.Lazy as Bytes
import System.Environment (getEnv)
import System.FilePath ((</>))
main :: IO ()
main = do
  let task = jsonTask "permissions" (const "record effective request") :: Task () Value
      changed = (providerRef "codex") {providerRefPermissions = Just (emptyPermissions {allowedTools = Just []})}
      ordinary = (providerRef "codex") {providerRefPermissions = Just emptyPermissions}
  values <- runTactus $ do
    first <- invoke task ()
    second <- invokeWith changed task ()
    third <- invokeWith ordinary task ()
    pure [first, second, third]
  root <- getEnv "TACTUS_WORKSPACE"
  Bytes.writeFile (root </> "permission-results.json") (encode values)
"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["run", "--root"])
        .arg(&root)
        .args(["--script", ".tactus/scripts/010_permissions.hs"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let received: Value =
        serde_json::from_slice(&fs::read(root.join("permission-results.json")).unwrap()).unwrap();
    assert_eq!(received[0]["permissions"], defaults);
    assert_eq!(received[1]["permissions"], json!({"allowed_tools":[]}));
    assert!(received[2].get("permissions").is_none());
    assert_eq!(
        calls(&root)
            .iter()
            .map(|call| call["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["describe", "invoke", "describe", "invoke", "invoke"]
    );
}

#[cfg(unix)]
#[test]
fn cancellation_during_capability_probe_does_not_dispatch_invoke() {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    use std::time::{Duration, Instant};
    let (_temporary, root) = project("permission-slow-description", None);
    let mut child = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "plugin-call",
            "codex",
            "invoke",
            "--namespace",
            "provider",
            "--root",
        ])
        .arg(&root)
        .args([
            "--params",
            r#"{"permissions":{"allowed_tools":[]}}"#,
            "--json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !root.join("permission-calls.jsonl").exists()
        && started.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(root.join("permission-calls.jsonl").exists());
    kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).unwrap();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > Duration::from_secs(8) {
            child.kill().unwrap();
            panic!("cancelled capability probe did not finish");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        calls(&root)
            .iter()
            .map(|call| call["method"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["describe"]
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["summary"]["outcome"]["terminal"]["error"]["code"],
        "provider_dispatch_cancelled"
    );
}
