use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};

use serde_json::{Value, json};
use tactus_runtime::adapters::{run_provider_host, run_workspace_paths_host};
use tempfile::{TempDir, tempdir};

#[derive(Clone)]
struct FakeNativeCli {
    _directory: Arc<TempDir>,
    prefix: Vec<String>,
    arguments_path: PathBuf,
    environment_path: PathBuf,
    workspace_path: PathBuf,
}

impl FakeNativeCli {
    fn create() -> Self {
        let directory = Arc::new(tempdir().expect("fake CLI directory"));
        let arguments_path = directory.path().join("arguments.txt");
        let environment_path = directory.path().join("environment.txt");
        let workspace_path = directory.path().join("workspace.txt");
        let prefix = create_fake_script(directory.path());
        Self {
            _directory: directory,
            prefix,
            arguments_path,
            environment_path,
            workspace_path,
        }
    }

    fn options(&self) -> Value {
        json!({
            "command_prefix":self.prefix,
            "timeout_seconds":5,
            "extra_env":{
                "FAKE_ARGS_PATH":self.arguments_path,
                "FAKE_ENV_PATH":self.environment_path,
                "FAKE_CWD_PATH":self.workspace_path
            }
        })
    }

    fn arguments(&self) -> Vec<String> {
        fs::read_to_string(&self.arguments_path)
            .expect("captured arguments")
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(windows)]
fn create_fake_script(directory: &Path) -> Vec<String> {
    let script = directory.join("fake-provider.cmd");
    fs::write(
        &script,
        r#"@echo off
setlocal EnableExtensions DisableDelayedExpansion
> "%FAKE_ARGS_PATH%" echo(%~1
>> "%FAKE_ARGS_PATH%" echo(%*
> "%FAKE_ENV_PATH%" echo(%OPENCODE_CONFIG_CONTENT%
> "%FAKE_CWD_PATH%" cd
echo %* | %SystemRoot%\System32\findstr.exe /C:"--version" >nul
if not errorlevel 1 (
  echo fake-cli 1.0
  exit /b 0
)
if "%FAKE_OUTPUT_MODE%"=="oversize" (
  powershell.exe -NoProfile -NonInteractive -Command "[Console]::Out.Write('x' * 8388609)"
  exit /b 0
)
if "%FAKE_STDERR_MODE%"=="oversize" (
  powershell.exe -NoProfile -NonInteractive -Command "[Console]::Error.Write('e' * 1052672)"
)
%SystemRoot%\System32\more.com >nul
if "%~1"=="codex" (
  echo {"type":"item.completed","item":{"type":"agent_message","text":"TACTUS_OK"}}
) else if "%~1"=="claude" (
  echo {"type":"result","result":"TACTUS_OK"}
) else (
  echo {"type":"text","part":{"text":"TACTUS_OK"}}
)
"#,
    )
    .expect("fake cmd provider");
    vec![
        "cmd.exe".to_owned(),
        "/d".to_owned(),
        "/c".to_owned(),
        script.to_string_lossy().into_owned(),
    ]
}

#[cfg(unix)]
fn create_fake_script(directory: &Path) -> Vec<String> {
    let script = directory.join("fake-provider.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
printf '%s\n' "$@" > "$FAKE_ARGS_PATH"
printf '%s' "$OPENCODE_CONFIG_CONTENT" > "$FAKE_ENV_PATH"
pwd > "$FAKE_CWD_PATH"
for item in "$@"; do
  if [ "$item" = "--version" ]; then printf '%s\n' 'fake-cli 1.0'; exit 0; fi
done
if [ "$FAKE_OUTPUT_MODE" = "oversize" ]; then
  dd if=/dev/zero bs=8388609 count=1 2>/dev/null | tr '\000' x
  exit 0
fi
if [ "$FAKE_STDERR_MODE" = "oversize" ]; then
  dd if=/dev/zero bs=1052672 count=1 2>/dev/null | tr '\000' e >&2
fi
cat >/dev/null
case "$1" in
  codex) printf '%s\n' '{"type":"item.completed","item":{"type":"agent_message","text":"TACTUS_OK"}}' ;;
  claude) printf '%s\n' '{"type":"result","result":"TACTUS_OK"}' ;;
  *) printf '%s\n' '{"type":"text","part":{"text":"TACTUS_OK"}}' ;;
esac
"#,
    )
    .expect("fake shell provider");
    // Execute the stable interpreter, not the freshly written fixture inode.
    // Concurrent forked tests may temporarily retain the script's writable
    // descriptor and make direct exec fail with ETXTBSY (Text file busy).
    vec!["/bin/sh".to_owned(), script.to_string_lossy().into_owned()]
}

#[test]
fn permission_descriptions_advertise_only_validated_adapter_mappings() {
    for provider in ["codex", "claude-code", "opencode", "antigravity"] {
        let (code, frames, _) = call_provider(provider, "describe", json!({}));
        assert_eq!(code, 0);
        let capabilities = &frames.last().unwrap()["value"]["permissions_capabilities"];
        assert_eq!(capabilities["allowed_skills"]["support"], "unsupported");
        assert_eq!(
            capabilities["allowed_tools"]["support"],
            if provider == "claude-code" {
                "partial"
            } else {
                "unsupported"
            }
        );
        assert_eq!(
            capabilities["sandbox"]["support"],
            if provider == "codex" {
                "supported"
            } else {
                "unsupported"
            }
        );
    }
}

#[test]
fn codex_requested_sandbox_replaces_bypass_and_selects_actual_working_directory() {
    let workspace = tempdir().unwrap();
    let selected = tempdir().unwrap();
    for (mode, native_mode, bypass) in [
        ("read-only", "read-only", false),
        ("workspace-write", "workspace-write", false),
        ("unrestricted", "danger-full-access", true),
    ] {
        let fake = FakeNativeCli::create();
        let requested = json!({"sandbox":{"mode":mode,"workspace":selected.path()}});
        let (code, frames, diagnostics) = call_provider(
            "codex",
            "invoke",
            json!({"workspace":workspace.path(),"prompt":"fixture","options":fake.options(),"permissions":requested}),
        );
        assert_eq!(code, 0, "{diagnostics}");
        let args = fake.arguments().join("\n");
        assert!(args.contains("--sandbox"));
        assert!(args.contains(native_mode));
        assert!(args.contains("approval_policy=\"never\""));
        if mode == "workspace-write" {
            assert!(args.contains("sandbox_workspace_write.writable_roots=[]"));
            assert!(args.contains("sandbox_workspace_write.exclude_tmpdir_env_var=true"));
            assert!(args.contains("sandbox_workspace_write.exclude_slash_tmp=true"));
        }
        assert!(!args.contains("--dangerously-bypass-approvals-and-sandbox"));
        assert!(args.contains(&selected.path().to_string_lossy().to_string()));
        let actual_cwd = fs::read_to_string(&fake.workspace_path).unwrap();
        assert_eq!(
            dunce::canonicalize(actual_cwd.trim()).unwrap(),
            dunce::canonicalize(selected.path()).unwrap()
        );
        let value = &frames.last().unwrap()["value"];
        assert_eq!(value["full_bypass"], bypass);
        assert_eq!(value["permission_report"]["requested"], requested);
        assert_eq!(
            value["permission_report"]["applications"][0]["status"],
            "applied"
        );
        assert_eq!(frames[0]["event"]["type"], "provider.permissions");
    }
}

#[test]
fn unsupported_permission_fields_fall_back_with_result_event_and_stderr_evidence() {
    let workspace = tempdir().unwrap();
    for provider in ["codex", "claude-code", "opencode"] {
        let fake = FakeNativeCli::create();
        let requested = if provider == "codex" {
            json!({"allowed_tools":[],"allowed_skills":["native-skill"]})
        } else {
            json!({"allowed_skills":[],"sandbox":{"mode":"read-only"}})
        };
        let (code, frames, diagnostics) = call_provider(
            provider,
            "invoke",
            json!({"workspace":workspace.path(),"prompt":"fixture","options":fake.options(),"permissions":requested}),
        );
        assert_eq!(code, 0, "{provider}: {diagnostics}");
        let report = &frames.last().unwrap()["value"]["permission_report"];
        assert_eq!(report["requested"], requested);
        assert_eq!(report["effective"], json!({}));
        assert_eq!(report["warnings"].as_array().unwrap().len(), 2);
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["event"]["type"] == "provider.permission.warning")
                .count(),
            2
        );
        assert!(diagnostics.contains("permission_unsupported"));
        assert!(!fake.arguments().join("\n").contains("--sandbox"));
        assert_eq!(
            frames.last().unwrap()["value"]["full_bypass"],
            provider != "opencode"
        );
    }
}

#[test]
fn claude_tool_catalog_selection_is_partial_and_unknown_names_fall_back_whole_field() {
    let workspace = tempdir().unwrap();
    for tools in [
        json!([]),
        json!(["Read", "Bash"]),
        json!(["Read", "mcp__server__read"]),
    ] {
        let fake = FakeNativeCli::create();
        let (code, frames, diagnostics) = call_provider(
            "claude-code",
            "invoke",
            json!({"workspace":workspace.path(),"prompt":"fixture","options":fake.options(),"permissions":{"allowed_tools":tools}}),
        );
        assert_eq!(code, 0, "{diagnostics}");
        let report = &frames.last().unwrap()["value"]["permission_report"];
        let unknown = tools
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool.as_str().unwrap().starts_with("mcp__"));
        assert_eq!(
            report["applications"][0]["status"],
            if unknown { "unsupported" } else { "partial" }
        );
        assert_eq!(
            report["effective"],
            if unknown {
                json!({})
            } else {
                json!({"allowed_tools":tools})
            }
        );
        assert_eq!(fake.arguments().join("\n").contains("--tools"), !unknown);
        assert!(diagnostics.contains(if unknown {
            "permission_unsupported"
        } else {
            "MCP"
        }));
    }
}

#[test]
fn supported_permissions_reject_conflicting_extra_args_before_native_spawn() {
    let workspace = tempdir().unwrap();
    for (provider, permission, argument) in [
        (
            "codex",
            json!({"sandbox":{"mode":"read-only"}}),
            "--full-auto",
        ),
        (
            "codex",
            json!({"sandbox":{"mode":"read-only"}}),
            "--sandbox=danger-full-access",
        ),
        (
            "codex",
            json!({"sandbox":{"mode":"read-only"}}),
            "-csandbox_mode=\"danger-full-access\"",
        ),
        (
            "claude-code",
            json!({"allowed_tools":[]}),
            "--tools=default",
        ),
    ] {
        let fake = FakeNativeCli::create();
        let mut options = fake.options();
        options["extra_args"] = json!([argument]);
        let (code, frames, _) = call_provider(
            provider,
            "invoke",
            json!({"workspace":workspace.path(),"prompt":"fixture","options":options,"permissions":permission}),
        );
        assert_ne!(code, 0);
        assert_eq!(frames.last().unwrap()["error"]["code"], "invalid_params");
        assert!(
            !fake.arguments_path.exists(),
            "invalid request spawned native CLI"
        );
    }
}

#[test]
fn empty_permission_object_retains_ordinary_mode_without_warnings() {
    let workspace = tempdir().unwrap();
    let fake = FakeNativeCli::create();
    let (code, frames, diagnostics) = call_provider(
        "codex",
        "invoke",
        json!({"workspace":workspace.path(),"prompt":"fixture","options":fake.options(),"permissions":{}}),
    );
    assert_eq!(code, 0, "{diagnostics}");
    assert!(
        fake.arguments()
            .join("\n")
            .contains("--dangerously-bypass-approvals-and-sandbox")
    );
    assert_eq!(
        frames.last().unwrap()["value"]["permission_report"]["applications"],
        json!([])
    );
    assert!(!diagnostics.contains("permission_unsupported"));
}

#[test]
fn providers_use_exact_permissive_native_arguments_and_normalize_results() {
    let workspace = tempdir().expect("workspace");
    for (provider, executable) in [
        ("codex", "codex"),
        ("claude-code", "claude"),
        ("opencode", "opencode"),
    ] {
        let fake = FakeNativeCli::create();
        let mut options = fake.options();
        options["extra_args"] = json!(["--caller-open-option"]);
        let params = if provider == "opencode" {
            json!({
                "prompt":"count holes",
                "workspace":workspace.path(),
                "model":"fake-model",
                "variant":"xhigh",
                "options":options
            })
        } else {
            json!({
                "prompt":"count holes",
                "workspace":workspace.path(),
                "model":"fake-model",
                "effort":"high",
                "options":options
            })
        };
        let (code, frames, diagnostics) = call_provider(provider, "invoke", params);
        assert_eq!(code, 0, "{provider}: {diagnostics}");
        assert_eq!(frames.len(), 3, "{provider}: {frames:?}");
        assert_eq!(frames[0]["event"]["type"], "provider.progress");
        assert_eq!(frames[1]["event"]["type"], "provider.diagnostic");
        let terminal = &frames[2];
        assert_eq!(terminal["ok"], true);
        assert_eq!(terminal["value"]["text"], "TACTUS_OK");
        assert_eq!(terminal["value"]["full_bypass"], provider != "opencode");

        let arguments = fake.arguments();
        assert_eq!(arguments[0], executable);
        let command_line = arguments.join("\n");
        assert!(command_line.contains("fake-model"));
        assert!(command_line.contains("--caller-open-option"));
        match provider {
            "codex" => {
                assert!(command_line.contains("exec"));
                assert!(command_line.contains("--dangerously-bypass-approvals-and-sandbox"));
                assert!(command_line.contains("--skip-git-repo-check"));
                assert!(command_line.contains("--ephemeral"));
                assert!(command_line.contains("model_reasoning_effort="));
                assert!(command_line.contains("high"));
                assert!(command_line.trim_end().ends_with('-'));
            }
            "claude-code" => {
                assert!(command_line.contains("-p"));
                assert!(command_line.contains("--dangerously-skip-permissions"));
                assert!(command_line.contains("stream-json"));
                assert!(command_line.contains("--no-session-persistence"));
                assert!(command_line.contains("high"));
            }
            "opencode" => {
                assert!(command_line.contains("run"));
                assert!(command_line.contains("--auto"));
                assert!(command_line.contains("--format"));
                assert!(command_line.contains("--variant"));
                assert!(terminal["value"]["warning"].is_string());
                let inline: Value = serde_json::from_str(
                    &fs::read_to_string(&fake.environment_path).expect("OpenCode environment"),
                )
                .expect("OpenCode inline config");
                assert_eq!(inline["permission"], "allow");
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(unix)]
#[test]
fn fake_provider_runs_while_its_script_is_open_for_writing() {
    let workspace = tempdir().expect("workspace");
    let fake = FakeNativeCli::create();
    // A concurrently forked child can retain another test's write descriptor
    // until it execs, even after fs::write has returned in the creating thread.
    // Holding a writer reproduces the same inode condition deterministically.
    let _inherited_writer = fs::OpenOptions::new()
        .write(true)
        .open(fake._directory.path().join("fake-provider.sh"))
        .expect("retained script writer");
    let (code, frames, diagnostics) = call_provider(
        "codex",
        "invoke",
        json!({"prompt":"fixture only", "workspace":workspace.path(), "options":fake.options()}),
    );
    assert_eq!(code, 0, "frames: {frames:?}; diagnostics: {diagnostics}");
    assert_eq!(
        frames.last().expect("terminal")["value"]["text"],
        "TACTUS_OK"
    );
}

#[test]
fn smoke_is_offline_unless_live_is_explicit() {
    let workspace = tempdir().expect("workspace");
    let fake = FakeNativeCli::create();
    let (code, frames, diagnostics) = call_provider(
        "codex",
        "smoke",
        json!({"workspace":workspace.path(), "options":fake.options()}),
    );
    assert_eq!(code, 0, "{diagnostics}");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["value"]["live"], false);
    assert_eq!(frames[0]["value"]["version"], "fake-cli 1.0");
    assert!(fake.arguments().join("\n").contains("--version"));

    for provider in ["claude-code", "opencode"] {
        let fake = FakeNativeCli::create();
        let (code, frames, diagnostics) = call_provider(
            provider,
            "smoke",
            json!({
                "workspace":workspace.path(),
                "live":true,
                "options":fake.options()
            }),
        );
        assert_eq!(code, 0, "{provider}: {diagnostics}");
        assert_eq!(frames.last().expect("terminal")["value"]["live"], true);
        assert_eq!(
            frames.last().expect("terminal")["value"]["full_bypass"],
            provider == "claude-code"
        );
    }
}

fn antigravity_options(capture: &Path, mode: &str) -> Value {
    json!({
        "command_prefix":[env!("CARGO_BIN_EXE_tactus-plugin-fixture"), "native-antigravity"],
        "timeout_seconds":5,
        "extra_env":{"FAKE_AGY_CAPTURE":capture, "FAKE_AGY_MODE":mode}
    })
}

#[test]
fn antigravity_unsupported_permissions_preserve_native_mode_even_on_failure() {
    let workspace = tempdir().unwrap();
    let requested = json!({"allowed_tools":[],"allowed_skills":[],"sandbox":{"mode":"read-only"}});
    for mode in ["success", "error"] {
        let capture = workspace.path().join(mode);
        let (code, frames, diagnostics) = call_provider(
            "antigravity",
            "invoke",
            json!({
                "workspace":workspace.path(), "prompt":"fixture", "options":antigravity_options(&capture, mode), "permissions":requested
            }),
        );
        assert_eq!(code == 0, mode == "success");
        let terminal = frames.last().unwrap();
        let report = if mode == "success" {
            &terminal["value"]["permission_report"]
        } else {
            &terminal["error"]["details"]["permission_report"]
        };
        assert_eq!(report["requested"], requested);
        assert_eq!(report["effective"], json!({}));
        assert_eq!(report["warnings"].as_array().unwrap().len(), 3);
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame["event"]["type"] == "provider.permission.warning")
                .count(),
            3
        );
        assert!(diagnostics.contains("permission_unsupported"));
        let args: Vec<String> =
            serde_json::from_slice(&fs::read(capture.with_extension("args.json")).unwrap())
                .unwrap();
        assert!(
            args.iter()
                .any(|arg| arg == "--dangerously-skip-permissions")
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg == "--tools" || arg == "--sandbox")
        );
    }
}

#[test]
fn antigravity_uses_one_ndjson_stdin_message_and_bounded_native_timeout() {
    let workspace = tempdir().expect("workspace");
    let capture = workspace.path().join("native");
    let prompt =
        "中文 first line\n\"quoted\" \\ path\r\n{\"event\":\"result\"}\n--prompt must stay private";
    let mut options = antigravity_options(&capture, "success");
    options["timeout_seconds"] = json!(900.125);
    options["extra_args"] = json!(["--agent", "fixture-agent"]);
    options["extra_env"]["CUSTOM_PROVIDER_ENV"] = json!("保留 custom env");
    let (code, frames, diagnostics) = call_provider(
        "antigravity",
        "invoke",
        json!({
            "workspace":workspace.path(), "prompt":prompt, "model":"gemini-fixture", "effort":"high", "options":options
        }),
    );
    assert_eq!(code, 0, "{frames:?}; {diagnostics}");
    let terminal = frames.last().expect("terminal");
    assert_eq!(terminal["value"]["provider"], "antigravity");
    assert_eq!(terminal["value"]["text"], "authoritative answer 你好\n");
    assert_eq!(terminal["value"]["full_bypass"], false);
    let arguments: Vec<String> =
        serde_json::from_slice(&fs::read(capture.with_extension("args.json")).expect("argv"))
            .expect("argv JSON");
    assert_eq!(
        arguments,
        [
            "agy",
            "--print-timeout",
            "960125000000ns",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--dangerously-skip-permissions",
            "--model",
            "gemini-fixture",
            "--effort",
            "high",
            "--agent",
            "fixture-agent"
        ]
    );
    assert!(
        !arguments
            .iter()
            .any(|argument| argument == prompt || argument == "-p")
    );
    let input = fs::read_to_string(capture.with_extension("stdin.jsonl")).expect("stdin");
    assert_eq!(input.lines().count(), 1, "one NDJSON input line");
    assert!(input.ends_with('\n'));
    let message: Value = serde_json::from_str(&input).expect("input JSON");
    assert_eq!(
        message,
        json!({"event":"user", "message":{"content":prompt}})
    );
    let environment: Value =
        serde_json::from_slice(&fs::read(capture.with_extension("env.json")).expect("env"))
            .expect("env JSON");
    assert_eq!(environment["AGY_CLI_DISABLE_AUTO_UPDATE"], "true");
    assert_eq!(environment["CUSTOM_PROVIDER_ENV"], "保留 custom env");
    assert_eq!(
        Path::new(environment["cwd"].as_str().expect("cwd")),
        dunce::canonicalize(workspace.path()).expect("canonical cwd")
    );
    let event_output = serde_json::to_string(
        &frames
            .iter()
            .filter(|frame| frame["type"] == "event")
            .collect::<Vec<_>>(),
    )
    .expect("event JSON");
    for hidden in [
        prompt,
        "private-auth",
        "secret_tool",
        "authoritative answer",
    ] {
        assert!(!event_output.contains(hidden), "event leakage: {hidden}");
        assert!(
            !diagnostics.contains(hidden),
            "diagnostic leakage: {hidden}"
        );
    }
}

#[test]
fn antigravity_aliases_empty_response_and_default_timeout_are_explicit() {
    let workspace = tempdir().expect("workspace");
    let capture = workspace.path().join("native");
    for alias in ["antigravity", "agy", "anti-gravity"] {
        let (code, frames, _) = call_provider(alias, "describe", json!({}));
        assert_eq!(code, 0);
        let description = &frames[0]["value"];
        assert_eq!(description["name"], "antigravity");
        assert_eq!(description["executable"], "agy");
        assert_eq!(description["aliases"], json!(["agy", "anti-gravity"]));
        assert_eq!(description["auth_status_supported"], false);
        assert_eq!(description["default_timeout_seconds"], 300);
        assert_eq!(description["native_timeout_headroom_seconds"], 60);
        let mut options = antigravity_options(&capture, "empty");
        options
            .as_object_mut()
            .expect("options")
            .remove("timeout_seconds");
        options["extra_env"]["AGY_CLI_DISABLE_AUTO_UPDATE"] = json!("false");
        let (code, frames, diagnostics) = call_provider(
            alias,
            "invoke",
            json!({"workspace":workspace.path(), "prompt":"fixture only", "options":options}),
        );
        assert_eq!(code, 0, "{frames:?}; {diagnostics}");
        assert_eq!(frames.last().expect("terminal")["value"]["text"], "");
        let arguments: Value =
            serde_json::from_slice(&fs::read(capture.with_extension("args.json")).expect("argv"))
                .expect("argv JSON");
        assert_eq!(arguments[1], "--print-timeout");
        assert_eq!(arguments[2], "360000000000ns");
        let environment: Value =
            serde_json::from_slice(&fs::read(capture.with_extension("env.json")).expect("env"))
                .expect("env JSON");
        assert_eq!(environment["AGY_CLI_DISABLE_AUTO_UPDATE"], "false");
    }
}

#[test]
fn antigravity_smoke_checks_version_and_rejects_unsupported_auth_without_spawning() {
    let workspace = tempdir().expect("workspace");
    let capture = workspace.path().join("native");
    let (code, frames, diagnostics) = call_provider(
        "antigravity",
        "smoke",
        json!({"workspace":workspace.path(), "options":antigravity_options(&capture, "success")}),
    );
    assert_eq!(code, 0, "{frames:?}; {diagnostics}");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["value"]["live"], false);
    assert_eq!(
        frames[0]["value"]["version"],
        "Antigravity CLI 1.2.1 (offline fixture)"
    );
    let arguments: Value =
        serde_json::from_slice(&fs::read(capture.with_extension("args.json")).expect("argv"))
            .expect("argv JSON");
    assert_eq!(arguments, json!(["agy", "--version"]));
    assert!(!capture.with_extension("stdin.jsonl").exists());
    fs::remove_file(capture.with_extension("args.json")).expect("remove capture");
    let (code, frames, _) = call_provider(
        "antigravity",
        "smoke",
        json!({"workspace":workspace.path(), "auth_status":true, "live":true, "options":antigravity_options(&capture, "success")}),
    );
    assert_ne!(code, 0);
    assert_eq!(frames[0]["error"]["code"], "auth_status_unsupported");
    assert!(
        !capture.with_extension("args.json").exists(),
        "unsupported health check must not spawn auth or a live model"
    );
}

#[test]
fn antigravity_native_failures_and_ambiguous_results_remain_distinct() {
    let workspace = tempdir().expect("workspace");
    let capture = workspace.path().join("native");
    for mode in [
        "error",
        "invalid",
        "early-error",
        "canceled",
        "interrupted",
        "waiting",
        "running",
        "unknown-status",
        "missing",
        "malformed",
        "invalid-utf8",
        "mismatch",
        "bad-response",
        "duplicate-keys",
        "duplicate-result",
        "post-terminal",
        "no-init",
        "nonzero",
    ] {
        let (code, frames, diagnostics) = call_provider(
            "antigravity",
            "invoke",
            json!({"workspace":workspace.path(), "prompt":"fixture only", "options":antigravity_options(&capture, mode)}),
        );
        assert_ne!(code, 0, "{mode}: {frames:?}; {diagnostics}");
        let expected = if matches!(mode, "error" | "invalid" | "early-error") {
            "provider_reported_failure"
        } else {
            "outcome_unknown"
        };
        assert_eq!(
            frames.last().expect("terminal")["error"]["code"],
            expected,
            "{mode}: {frames:?}"
        );
    }
    for (mode, key, limit, cause) in [
        (
            "success",
            "native_max_result_bytes",
            json!(4),
            "provider_result_limit",
        ),
        (
            "oversize-line",
            "native_max_line_bytes",
            json!(256),
            "native_output_limit",
        ),
        ("timeout", "timeout_seconds", json!(0.05), "timeout"),
    ] {
        let mut options = antigravity_options(&capture, mode);
        options[key] = limit;
        let started = std::time::Instant::now();
        let (code, frames, _) = call_provider(
            "antigravity",
            "invoke",
            json!({"workspace":workspace.path(), "prompt":"fixture only", "options":options}),
        );
        assert_ne!(code, 0);
        let terminal = frames.last().expect("terminal");
        assert_eq!(terminal["error"]["code"], "outcome_unknown");
        assert_eq!(terminal["error"]["details"]["cause"], cause);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "deadline/limit must bound native process lifetime"
        );
    }
}

#[test]
fn antigravity_rejects_native_timeout_overrides_before_dispatch() {
    let workspace = tempdir().expect("workspace");
    let capture = workspace.path().join("native");
    for arguments in [
        json!(["--print-timeout", "1ms"]),
        json!(["--print-timeout=1ms"]),
        json!(["-print-timeout=1ms"]),
        json!(["--"]),
    ] {
        let mut options = antigravity_options(&capture, "success");
        options["extra_args"] = arguments;
        let (code, frames, _) = call_provider(
            "antigravity",
            "invoke",
            json!({"workspace":workspace.path(), "prompt":"fixture only", "options":options}),
        );
        assert_ne!(code, 0);
        assert_eq!(
            frames.last().expect("terminal")["error"]["code"],
            "invalid_params"
        );
        assert!(!capture.with_extension("args.json").exists());
    }
}

#[test]
fn antigravity_streams_only_agent_deltas_before_terminal_without_final_duplication() {
    let workspace = tempdir().expect("workspace");
    tactus_runtime::workspace::initialize_workspace(workspace.path(), None).expect("initialize");
    let capture = workspace.path().join("native");
    let socket =
        std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("UDP receiver");
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .expect("receive timeout");
    fs::write(workspace.path().join(".tactus/watch.json"), serde_json::to_vec(&json!({"api":"tactus.watch/v1", "port":socket.local_addr().expect("address").port(), "token":"watch-offline-test-012345678901234567890123456789"})).expect("descriptor JSON")).expect("descriptor");
    let root = workspace.path().to_path_buf();
    let options = antigravity_options(&capture, "stream");
    let worker = thread::spawn(move || {
        call_provider(
            "antigravity",
            "invoke",
            json!({"workspace":root, "prompt":"private invocation prompt", "options":options}),
        )
    });
    let mut bytes = [0_u8; 65536];
    let receive = |buffer: &mut [u8]| {
        let count = socket.recv(buffer).expect("native packet");
        serde_json::from_slice::<Value>(&buffer[..count]).expect("packet JSON")
    };
    assert_eq!(receive(&mut bytes)["kind"], "started");
    let first = receive(&mut bytes);
    let running_before_terminal = !worker.is_finished();
    fs::write(capture.with_extension("release"), "finish").expect("release native provider");
    let (code, frames, diagnostics) = worker.join().expect("provider worker");
    assert!(running_before_terminal);
    assert_eq!(first["kind"], "text.delta");
    assert_eq!(first["text"], "live first 你好\n");
    let second = receive(&mut bytes);
    assert_eq!(second["kind"], "text.delta");
    assert_eq!(second["text"], "live last\n");
    assert_eq!(
        receive(&mut bytes)["kind"],
        "finished",
        "no repeated final response or private native records"
    );
    assert_eq!(code, 0, "{frames:?}; {diagnostics}");
    assert_eq!(
        frames.last().expect("terminal")["value"]["text"],
        "authoritative answer 你好\n"
    );
    let events = serde_json::to_string(
        &frames
            .iter()
            .filter(|frame| frame["type"] == "event")
            .collect::<Vec<_>>(),
    )
    .expect("events JSON");
    for hidden in [
        "private invocation prompt",
        "private-native-content",
        "private-tool-arguments",
        "private-auth",
        "live first",
        "live last",
    ] {
        assert!(!events.contains(hidden));
        assert!(!diagnostics.contains(hidden));
    }
}

#[test]
fn path_effect_tracks_hash_deltas_without_obeying_gitignore() {
    let workspace = tempdir().expect("workspace");
    fs::create_dir(workspace.path().join(".git")).expect("git directory");
    fs::create_dir(workspace.path().join(".tactus")).expect("tactus directory");
    fs::create_dir_all(workspace.path().join(".tactus/dist-newstyle/cache"))
        .expect("compiler cache");
    fs::create_dir(workspace.path().join(".tactus/runs")).expect("runs directory");
    fs::create_dir(workspace.path().join(".tactus/scripts")).expect("scripts directory");
    fs::write(workspace.path().join(".git/hidden"), "git data").expect("git data");
    fs::write(
        workspace.path().join(".tactus/dist-newstyle/cache/hidden"),
        "cache",
    )
    .expect("cache data");
    fs::write(workspace.path().join(".tactus/scripts/010.hs"), "stage one").expect("script");
    fs::write(workspace.path().join(".gitignore"), "ignored.txt\n").expect("gitignore");
    fs::write(workspace.path().join("kept.txt"), "before").expect("kept");
    let first = effect_success("snapshot", json!({"workspace":workspace.path()}));

    for directory in ["target", "node_modules", "build", "dist-newstyle"] {
        fs::create_dir_all(workspace.path().join(directory)).expect("excluded directory");
        fs::write(workspace.path().join(directory).join("hidden"), "generated")
            .expect("excluded data");
    }
    fs::write(workspace.path().join("kept.txt"), "after").expect("modify kept");
    fs::write(workspace.path().join("ignored.txt"), "still observed").expect("ignored file");
    fs::write(workspace.path().join(".tactus/scripts/020.hs"), "stage two").expect("second script");
    fs::write(workspace.path().join(".git/new-hidden"), "hidden").expect("new git file");
    fs::write(
        workspace
            .path()
            .join(".tactus/dist-newstyle/cache/new-hidden"),
        "hidden",
    )
    .expect("new cache file");
    fs::write(workspace.path().join(".tactus/runs/self.jsonl"), "journal").expect("self journal");
    let second = effect_success("snapshot", json!({"workspace":workspace.path()}));
    let delta = effect_success(
        "diff",
        json!({"workspace":workspace.path(), "before":first, "after":second}),
    );
    let added = string_values(&delta["added"]);
    let modified = string_values(&delta["modified"]);
    assert!(added.contains(&"ignored.txt".to_owned()));
    assert!(added.contains(&".tactus/scripts/020.hs".to_owned()));
    assert!(modified.contains(&"kept.txt".to_owned()));
    assert!(!added.iter().any(|path| path.starts_with(".git/")));
    assert!(
        !added
            .iter()
            .any(|path| path.starts_with(".tactus/dist-newstyle/"))
    );
    assert!(!added.iter().any(|path| path.starts_with(".tactus/runs/")));
    for directory in ["target", "node_modules", "build", "dist-newstyle"] {
        assert!(
            !added.iter().any(|path| path.starts_with(directory)),
            "default generated directory {directory:?} was observed"
        );
    }

    assert_eq!(
        effect_success(
            "forget",
            json!({"workspace":workspace.path(), "snapshot_id":first["snapshot_id"]}),
        )["forgotten"],
        true
    );
    assert_eq!(
        effect_success(
            "forget",
            json!({"workspace":workspace.path(), "snapshot_id":second["snapshot_id"]}),
        )["forgotten"],
        true
    );
}

#[test]
fn observation_tokens_are_opaque_content_free_and_concurrently_idempotent() {
    let workspace = tempdir().expect("workspace");
    fs::create_dir(workspace.path().join(".tactus")).expect("tactus directory");
    fs::write(workspace.path().join("shape.txt"), "TOP_SECRET_CONTENT").expect("initial shape");
    let invocation = json!({"provider":"fake", "step":10});
    let begin = effect_success(
        "observe.begin",
        json!({"workspace":workspace.path(), "invocation":invocation}),
    );
    let state_files = fs::read_dir(workspace.path().join(".tactus/path-effect"))
        .expect("state directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("state files");
    assert_eq!(state_files.len(), 1);
    let state = fs::read_to_string(state_files[0].path()).expect("state JSON");
    assert!(!state.contains("TOP_SECRET_CONTENT"));
    assert!(state.contains("sha256"));

    fs::write(workspace.path().join("shape.txt"), "changed").expect("changed shape");
    let parameters = json!({
        "workspace":workspace.path(),
        "invocation":invocation,
        "begin":begin,
        "outcome":{"status":"succeeded"}
    });
    let left = parameters.clone();
    let right = parameters;
    let first = thread::spawn(move || call_effect("observe.end", left));
    let second = thread::spawn(move || call_effect("observe.end", right));
    let outcomes = [
        first.join().expect("first end"),
        second.join().expect("second end"),
    ];
    assert!(
        outcomes.iter().all(|(code, _, _)| *code == 0),
        "{outcomes:?}"
    );
    let first = outcomes[0].1.last().expect("first terminal");
    let second = outcomes[1].1.last().expect("second terminal");
    assert_eq!(first["value"], second["value"]);
    assert!(string_values(&first["value"]["delta"]["modified"]).contains(&"shape.txt".to_owned()));
}

#[test]
fn interrupted_final_observation_claim_is_recoverable() {
    let workspace = tempdir().expect("workspace");
    fs::create_dir(workspace.path().join(".tactus")).expect("tactus directory");
    let invocation = json!({"step":"recover"});
    let begin = effect_success(
        "observe.begin",
        json!({"workspace":workspace.path(), "invocation":invocation}),
    );
    let state = fs::read_dir(workspace.path().join(".tactus/path-effect"))
        .expect("state directory")
        .next()
        .expect("state entry")
        .expect("state");
    let claimed = state
        .path()
        .parent()
        .expect("parent")
        .join(format!(".{}.claimed", state.file_name().to_string_lossy()));
    fs::rename(state.path(), &claimed).expect("simulate interrupted final claim");
    let ended = effect_success(
        "observe.end",
        json!({
            "workspace":workspace.path(),
            "invocation":invocation,
            "begin":begin,
            "outcome":"ok"
        }),
    );
    assert_eq!(ended["outcome"], "ok");
    assert!(!claimed.exists());
}

#[test]
fn completed_observation_recovers_both_crash_windows_and_is_forgettable() {
    let workspace = tempdir().expect("workspace");
    fs::create_dir(workspace.path().join(".tactus")).expect("tactus directory");
    fs::write(workspace.path().join("shape.txt"), "before").expect("initial shape");
    let invocation = json!({"step":"durable-completion"});
    let begin = effect_success(
        "observe.begin",
        json!({"workspace":workspace.path(), "invocation":invocation}),
    );
    let original = fs::read_dir(workspace.path().join(".tactus/path-effect"))
        .expect("state directory")
        .next()
        .expect("state entry")
        .expect("state")
        .path();
    let original_bytes = fs::read(&original).expect("original state");
    fs::write(workspace.path().join("shape.txt"), "after").expect("changed shape");
    let parameters = json!({
        "workspace":workspace.path(),
        "invocation":invocation,
        "begin":begin,
        "outcome":{"status":"succeeded"}
    });

    let committed = effect_success("observe.end", parameters.clone());
    assert!(
        !original.exists(),
        "normal commit removes the original state"
    );
    let after_remove_retry = effect_success("observe.end", parameters.clone());
    assert_eq!(after_remove_retry, committed);

    fs::write(&original, original_bytes).expect("simulate crash before original cleanup");
    let before_remove_retry = effect_success("observe.end", parameters);
    assert_eq!(before_remove_retry, committed);
    assert!(!original.exists(), "idempotent retry cleans residual state");

    let forgotten = effect_success(
        "forget",
        json!({
            "workspace":workspace.path(),
            "invocation":invocation,
            "begin":begin
        }),
    );
    assert_eq!(forgotten["forgotten"], true);
    assert!(
        fs::read_dir(workspace.path().join(".tactus/path-effect"))
            .expect("shared state directory remains available to writers")
            .next()
            .is_none(),
        "forget removes all observation records"
    );
}

#[test]
fn concurrent_observer_begins_share_real_state_directories() {
    let workspace = tempdir().expect("workspace");
    let barrier = Arc::new(Barrier::new(8));
    let mut workers = Vec::new();
    for step in 0..8 {
        let root = workspace.path().to_path_buf();
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            call_effect(
                "observe.begin",
                json!({"workspace":root, "invocation":{"step":step}}),
            )
        }));
    }
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().expect("observer begin"))
        .collect();
    assert!(
        outcomes.iter().all(|(code, _, _)| *code == 0),
        "{outcomes:?}"
    );
    for (step, (_, frames, _)) in outcomes.into_iter().enumerate() {
        let begin = frames.last().expect("terminal")["value"].clone();
        let forgotten = effect_success(
            "forget",
            json!({
                "workspace":workspace.path(),
                "begin":begin,
                "invocation":{"step":step}
            }),
        );
        assert_eq!(forgotten["forgotten"], true);
    }
}

#[test]
fn hosts_reject_duplicate_request_keys_and_emit_one_terminal() {
    let input = br#"{"api":"agenstro.plugin/v1","id":"a","method":"describe","method":"smoke","params":{}}"#;
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    let code = run_workspace_paths_host(Cursor::new(input), &mut output, &mut diagnostics);
    assert_eq!(code, 2);
    let frames = frames(&output);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["type"], "result");
    assert_eq!(frames[0]["ok"], false);
    assert_eq!(frames[0]["error"]["code"], "invalid_json");
}

#[test]
fn hosts_bound_their_own_request_input() {
    let mut input =
        br#"{"api":"agenstro.plugin/v1","id":"a","method":"describe","params":{"padding":""#
            .to_vec();
    input.extend(std::iter::repeat_n(b'x', 16 * 1024 * 1024));
    input.extend_from_slice(br#""}}"#);
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    let code = run_workspace_paths_host(Cursor::new(input), &mut output, &mut diagnostics);
    assert_eq!(code, 2);
    let frames = frames(&output);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["error"]["code"], "request_too_large");
}

#[test]
fn provider_timeout_rejects_huge_finite_values_without_panicking() {
    let workspace = tempdir().expect("workspace");
    let (code, frames, diagnostics) = call_provider(
        "codex",
        "invoke",
        json!({
            "prompt":"count holes",
            "workspace":workspace.path(),
            "options":{"timeout_seconds":1e308}
        }),
    );
    assert_eq!(code, 1, "{diagnostics}");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["ok"], false);
    assert_eq!(frames[0]["error"]["code"], "invalid_params");
    assert!(
        frames[0]["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("supported duration range")
    );
}

#[test]
fn provider_host_kills_native_output_that_exceeds_its_line_bound() {
    let workspace = tempdir().expect("workspace");
    let fake = FakeNativeCli::create();
    let mut options = fake.options();
    // PowerShell startup and an 8 MiB pipe write can exceed the shared
    // five-second fixture budget on a busy Windows runner. This test verifies
    // the output bound, not deadline behavior; deadline semantics have their
    // own focused tests.
    options["timeout_seconds"] = json!(30);
    options["extra_env"]["FAKE_OUTPUT_MODE"] = Value::String("oversize".to_owned());
    let (code, frames, diagnostics) = call_provider(
        "codex",
        "invoke",
        json!({
            "prompt":"count holes",
            "workspace":workspace.path(),
            "options":options
        }),
    );
    assert_eq!(code, 1, "{diagnostics}");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["ok"], false);
    assert_eq!(
        frames[0]["error"]["code"], "outcome_unknown",
        "frames: {frames:?}; diagnostics: {diagnostics}"
    );
    assert_eq!(
        frames[0]["error"]["details"]["cause"],
        "native_output_limit"
    );
}

#[test]
fn provider_host_drains_but_bounds_large_native_stderr() {
    let workspace = tempdir().expect("workspace");
    let fake = FakeNativeCli::create();
    let mut options = fake.options();
    // Keep this diagnostic-volume test independent of Windows runner startup
    // jitter. Dedicated timeout tests continue to use short finite budgets.
    options["timeout_seconds"] = json!(30);
    options["extra_env"]["FAKE_STDERR_MODE"] = Value::String("oversize".to_owned());
    let (code, frames, diagnostics) = call_provider(
        "codex",
        "invoke",
        json!({
            "prompt":"count holes",
            "workspace":workspace.path(),
            "options":options
        }),
    );
    assert_eq!(code, 0, "{diagnostics}");
    assert_eq!(frames.last().expect("terminal")["ok"], true);
    assert!(diagnostics.contains("stderr truncated after 1048576 bytes"));
    assert!(diagnostics.len() <= 1024 * 1024 + 256);
}

#[test]
fn claude_twenty_thousand_thinking_events_are_aggregated() {
    let workspace = tempdir().expect("workspace");
    let (code, frames, diagnostics) = call_provider(
        "claude-code",
        "invoke",
        json!({
            "prompt":"count holes",
            "workspace":workspace.path(),
            "options":{
                "command_prefix":[
                    env!("CARGO_BIN_EXE_tactus-plugin-fixture"),
                    "native-claude-flood"
                ],
                "timeout_seconds":10
            }
        }),
    );
    assert_eq!(code, 0, "{diagnostics}");
    assert!(
        frames.len() <= 32,
        "native events were not bounded: {frames:?}"
    );
    let (terminal, events) = frames.split_last().expect("terminal");
    let summaries = events
        .iter()
        .filter(|frame| frame["event"]["type"] == "provider.diagnostic")
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0]["event"]["native_events"], 20_001);
    assert_eq!(summaries[0]["event"]["thinking_events_suppressed"], 20_000);
    for event in events {
        assert_eq!(event["type"], "event");
        if event["event"]["type"] != "provider.diagnostic" {
            assert_eq!(event["event"]["type"], "provider.progress");
            assert!(
                event["event"]
                    .as_object()
                    .expect("progress")
                    .keys()
                    .all(|key| {
                        matches!(
                            key.as_str(),
                            "type" | "provider" | "phase" | "native_lines_seen"
                        )
                    })
            );
        }
    }
    assert_eq!(terminal["type"], "result");
    assert_eq!(terminal["ok"], true);
    assert_eq!(terminal["value"]["text"], "TACTUS_OK");
}

#[test]
fn provider_deadline_kills_pipe_holding_descendant_after_leader_exit() {
    let workspace = tempdir().expect("workspace");
    let marker_directory = tempdir().expect("marker directory");
    let marker = marker_directory.path().join("provider-orphan");
    let options = orphan_options("native-orphan-provider", &marker);
    let (code, frames, diagnostics) = call_provider(
        "codex",
        "invoke",
        json!({
            "prompt":"count holes",
            "workspace":workspace.path(),
            "options":options
        }),
    );
    assert_eq!(code, 1, "{diagnostics}");
    assert_eq!(
        frames.last().expect("terminal")["error"]["code"],
        "outcome_unknown"
    );
    assert_eq!(
        frames.last().expect("terminal")["error"]["details"]["cause"],
        "timeout"
    );
    assert!(marker.with_extension("started").is_file());
    thread::sleep(std::time::Duration::from_millis(550));
    assert!(!marker.with_extension("survived").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn provider_waits_without_spinning_after_stdout_closes() {
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    let workspace = tempdir().expect("workspace");
    let script = workspace.path().join("close-stdout.sh");
    let ready = workspace.path().join("ready");
    let release = workspace.path().join("release");
    fs::write(
        &script,
        "cat >/dev/null\nprintf '%s\\n' '{\"type\":\"result\",\"result\":\"TACTUS_OK\"}'\nexec 1>&-\nprintf ready > \"$FAKE_READY\"\nwhile [ ! -f \"$FAKE_RELEASE\" ]; do sleep 0.01; done\n",
    )
    .expect("fake provider");
    let request = json!({
        "api":"agenstro.plugin/v1","id":"early-eof","method":"invoke",
        "params":{
            "prompt":"offline test","workspace":workspace.path(),
            "options":{
                "command_prefix":["/bin/sh",script],"timeout_seconds":10,
                "extra_env":{"FAKE_READY":ready,"FAKE_RELEASE":release}
            }
        }
    });
    let ticks = Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .expect("Linux process clock frequency");
    assert!(ticks.status.success());
    let ticks_per_second: u64 = String::from_utf8(ticks.stdout)
        .expect("clock frequency")
        .trim()
        .parse()
        .expect("integer clock frequency");
    let mut host = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["provider-host", "claude-code"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("provider host");
    host.stdin
        .take()
        .expect("host stdin")
        .write_all(&serde_json::to_vec(&request).expect("request JSON"))
        .expect("write request");
    let started = Instant::now();
    while !ready.exists() && started.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    let read_cpu_ticks = || {
        let stat = fs::read_to_string(format!("/proc/{}/stat", host.id())).expect("host stat");
        // comm may contain spaces or parentheses; fields after its final ')' start at state.
        let fields: Vec<_> = stat[stat.rfind(')').expect("comm field") + 1..]
            .split_whitespace()
            .collect();
        fields[11].parse::<u64>().expect("utime") + fields[12].parse::<u64>().expect("stime")
    };
    let was_ready = ready.exists();
    let before = read_cpu_ticks();
    thread::sleep(Duration::from_millis(350));
    let consumed = read_cpu_ticks().saturating_sub(before);
    fs::write(release, b"release").expect("release fake provider");
    let output = host.wait_with_output().expect("host completion");
    assert!(was_ready, "fake provider did not close stdout in time");
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(frames(&output.stdout).last().expect("terminal")["ok"], true);
    assert!(
        consumed <= ticks_per_second / 8,
        "provider host used {consumed} CPU ticks while waiting on closed stdout"
    );
}

#[test]
fn health_deadline_kills_pipe_holding_descendant_after_leader_exit() {
    let workspace = tempdir().expect("workspace");
    let marker_directory = tempdir().expect("marker directory");
    let marker = marker_directory.path().join("health-orphan");
    let options = orphan_options("native-orphan-health", &marker);
    let (code, frames, diagnostics) = call_provider(
        "codex",
        "smoke",
        json!({"workspace":workspace.path(), "options":options}),
    );
    assert_eq!(code, 1, "{diagnostics}");
    assert_eq!(
        frames.last().expect("terminal")["error"]["code"],
        "provider_timeout"
    );
    assert!(marker.with_extension("started").is_file());
    thread::sleep(std::time::Duration::from_millis(550));
    assert!(!marker.with_extension("survived").exists());
}

fn orphan_options(mode: &str, marker: &Path) -> Value {
    json!({
        "command_prefix":[env!("CARGO_BIN_EXE_tactus-plugin-fixture"), mode],
        "timeout_seconds":0.15,
        "extra_env":{"FAKE_ORPHAN_MARKER":marker}
    })
}

fn call_provider(provider: &str, method: &str, params: Value) -> (i32, Vec<Value>, String) {
    let request = json!({
        "api":"agenstro.plugin/v1",
        "id":"provider-test",
        "method":method,
        "params":params
    });
    let input = serde_json::to_vec(&request).expect("request JSON");
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    let code = run_provider_host(provider, Cursor::new(input), &mut output, &mut diagnostics);
    (
        code,
        frames(&output),
        String::from_utf8(diagnostics).expect("UTF-8 diagnostics"),
    )
}

fn call_effect(method: &str, params: Value) -> (i32, Vec<Value>, String) {
    let request = json!({
        "api":"agenstro.plugin/v1",
        "id":"effect-test",
        "method":method,
        "params":params
    });
    let input = serde_json::to_vec(&request).expect("request JSON");
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    let code = run_workspace_paths_host(Cursor::new(input), &mut output, &mut diagnostics);
    (
        code,
        frames(&output),
        String::from_utf8(diagnostics).expect("UTF-8 diagnostics"),
    )
}

fn effect_success(method: &str, params: Value) -> Value {
    let (code, frames, diagnostics) = call_effect(method, params);
    assert_eq!(code, 0, "{method}: {diagnostics}");
    let terminal = frames.last().expect("terminal frame");
    assert_eq!(terminal["ok"], true, "{method}: {terminal}");
    terminal["value"].clone()
}

fn frames(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes)
        .expect("UTF-8 frames")
        .split_terminator('\n')
        .map(|line| serde_json::from_str(line.strip_suffix('\r').unwrap_or(line)).expect("frame"))
        .collect()
}

fn string_values(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("string array")
        .iter()
        .map(|value| value.as_str().expect("string").to_owned())
        .collect()
}
