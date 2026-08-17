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
}

impl FakeNativeCli {
    fn create() -> Self {
        let directory = Arc::new(tempdir().expect("fake CLI directory"));
        let arguments_path = directory.path().join("arguments.txt");
        let environment_path = directory.path().join("environment.txt");
        let prefix = create_fake_script(directory.path());
        Self {
            _directory: directory,
            prefix,
            arguments_path,
            environment_path,
        }
    }

    fn options(&self) -> Value {
        json!({
            "command_prefix":self.prefix,
            "timeout_seconds":5,
            "extra_env":{
                "FAKE_ARGS_PATH":self.arguments_path,
                "FAKE_ENV_PATH":self.environment_path
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
    use std::os::unix::fs::PermissionsExt;

    let script = directory.join("fake-provider.sh");
    fs::write(
        &script,
        r#"#!/bin/sh
printf '%s\n' "$@" > "$FAKE_ARGS_PATH"
printf '%s' "$OPENCODE_CONFIG_CONTENT" > "$FAKE_ENV_PATH"
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
    let mut permissions = fs::metadata(&script)
        .expect("script metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("script executable");
    vec![script.to_string_lossy().into_owned()]
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
        assert_eq!(frames.len(), 2, "{provider}: {frames:?}");
        assert_eq!(frames[0]["type"], "event");
        let terminal = &frames[1];
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
    assert!(!workspace.path().join(".tactus/path-effect").exists());
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
    input.extend(std::iter::repeat_n(b'x', 1024 * 1024));
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
    assert_eq!(frames[0]["error"]["code"], "outcome_unknown");
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
    assert_eq!(frames.len(), 2, "raw native events leaked: {frames:?}");
    assert_eq!(frames[0]["type"], "event");
    assert_eq!(frames[0]["event"]["type"], "provider.diagnostic");
    assert_eq!(frames[0]["event"]["native_events"], 20_001);
    assert_eq!(frames[0]["event"]["thinking_events_suppressed"], 20_000);
    assert!(
        frames
            .iter()
            .all(|frame| frame["event"]["type"] != "provider.raw")
    );
    assert_eq!(frames[1]["ok"], true);
    assert_eq!(frames[1]["value"]["text"], "TACTUS_OK");
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
