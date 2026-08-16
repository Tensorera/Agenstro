use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Map, Value};
use tactus_runtime::{
    CancellationToken, InvocationKind, JsonField, PluginFrame, PluginRequest, ProcessSpec,
    ProcessSupervisor, TerminalResult,
    protocol::decode_frame,
    workspace::{Workspace, discover_scripts, initialize_workspace},
};
use tempfile::tempdir;

fn fixture_spec(mode: &str) -> (tempfile::TempDir, ProcessSpec) {
    let temporary = tempdir().expect("temporary directory");
    let command = vec![
        env!("CARGO_BIN_EXE_tactus-plugin-fixture").to_owned(),
        mode.to_owned(),
    ];
    let cwd = temporary.path().to_path_buf();
    (temporary, ProcessSpec::new(command, cwd))
}

fn set_command(project: &std::path::Path, needle: &str, mode: &str) {
    let config_path = project.join(".tactus/tactus.toml");
    let config = fs::read_to_string(&config_path).expect("read config");
    let executable = serde_json::to_string(env!("CARGO_BIN_EXE_tactus-plugin-fixture"))
        .expect("quote executable");
    let replacement = format!("command = [{executable}, {mode:?}]");
    fs::write(config_path, config.replacen(needle, &replacement, 1)).expect("write config");
}

fn initialized_project() -> (tempfile::TempDir, std::path::PathBuf) {
    let temporary = tempdir().expect("temporary directory");
    let sdk = temporary.path().join("sdk");
    fs::create_dir(&sdk).expect("sdk dir");
    fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("sdk cabal");
    let project = temporary.path().join("project");
    initialize_workspace(&project, Some(&sdk)).expect("init");
    (temporary, project)
}

#[test]
fn studio_control_api_is_versioned_redacted_and_path_safe() {
    let (_temporary, project) = initialized_project();
    fs::write(
        project.join(".tactus/scripts/010_unicode.hs"),
        "main = putStrLn \"你好\"\n",
    )
    .expect("script");
    let config_path = project.join(".tactus/tactus.toml");
    let executable = serde_json::to_string(env!("CARGO_BIN_EXE_tactus-plugin-fixture"))
        .expect("quote executable");
    let mut config = fs::read_to_string(&config_path).expect("config");
    config.push_str(&format!(
        "\n[plugins.audit]\ncommand = [{executable}, \"success\"]\noptions = {{ token = \"never-project-this\" }}\n"
    ));
    fs::write(&config_path, config).expect("config");

    let invocation = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "plugin-call",
            "audit",
            "inspect",
            "--namespace",
            "plugin",
            "--root",
        ])
        .arg(&project)
        .arg("--json")
        .output()
        .expect("plugin call");
    assert!(invocation.status.success());

    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["studio", "inspect", "--root"])
        .arg(&project)
        .args(["--run-limit", "10"])
        .output()
        .expect("studio inspect");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Value = serde_json::from_slice(&output.stdout).expect("control json");
    assert_eq!(body["api"], "tactus.control/v1");
    assert_eq!(body["command"], "studio.inspect");
    assert_eq!(body["status"], "completed");
    assert_eq!(body["data"]["api"], "agenstro.studio/v1");
    assert_eq!(
        body["data"]["scripts"][0]["relativePath"],
        ".tactus/scripts/010_unicode.hs"
    );
    assert!(body["data"]["runs"][0]["startedUnixMs"].is_string());
    let encoded = String::from_utf8(output.stdout).expect("utf8");
    assert!(!encoded.contains("never-project-this"));
    assert!(!encoded.contains(&project.display().to_string()));

    let invalid = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["studio", "events", "../escape", "--root"])
        .arg(&project)
        .output()
        .expect("invalid run");
    assert!(!invalid.status.success());
    assert!(invalid.stderr.is_empty());
    let failure: Value = serde_json::from_slice(&invalid.stdout).expect("failure envelope");
    assert_eq!(failure["status"], "error");
    assert_eq!(failure["error"]["code"], "invalid_run_id");
}

fn request() -> PluginRequest {
    PluginRequest::new("test-run", "holes.compute", Map::new()).expect("request")
}

#[test]
fn streams_before_the_terminal_result() {
    let (_temporary, spec) = fixture_spec("stream");
    let started = Instant::now();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&observed);
    let outcome = ProcessSupervisor
        .invoke(&spec, &request(), &CancellationToken::new(), move |frame| {
            sink.lock()
                .expect("event lock")
                .push((started.elapsed(), frame.clone()));
        })
        .expect("invoke");
    assert_eq!(outcome.kind, InvocationKind::Succeeded);
    assert!(matches!(
        outcome.terminal,
        Some(TerminalResult::Success { .. })
    ));
    let observed = observed.lock().expect("event lock");
    assert_eq!(observed.len(), 2);
    assert!(matches!(observed[0].1, PluginFrame::Event { .. }));
    assert!(observed[1].0.saturating_sub(observed[0].0) >= Duration::from_millis(300));
}

#[test]
fn concurrently_drains_and_bounds_stderr() {
    let (_temporary, mut spec) = fixture_spec("stderr");
    spec.limits.max_stderr_bytes = 1024;
    let outcome = ProcessSupervisor
        .invoke(&spec, &request(), &CancellationToken::new(), |_| {})
        .expect("invoke");
    assert_eq!(outcome.kind, InvocationKind::Succeeded);
    assert_eq!(outcome.stderr.len(), 1024);
    assert!(outcome.stderr_truncated);
}

#[test]
fn rejects_duplicate_terminal_and_invalid_utf8() {
    for mode in ["duplicate", "invalid-utf8", "missing"] {
        let (_temporary, spec) = fixture_spec(mode);
        let outcome = ProcessSupervisor
            .invoke(&spec, &request(), &CancellationToken::new(), |_| {})
            .expect("invoke");
        assert_eq!(outcome.kind, InvocationKind::ProtocolFailed, "{mode}");
        assert!(outcome.error.is_some(), "{mode}");
    }
}

#[test]
fn enforces_the_aggregate_stdout_budget() {
    let (_temporary, mut spec) = fixture_spec("success");
    spec.limits.max_stdout_bytes = 64;
    let outcome = ProcessSupervisor
        .invoke(&spec, &request(), &CancellationToken::new(), |_| {})
        .expect("invoke");
    assert_eq!(outcome.kind, InvocationKind::ProtocolFailed);
    assert!(
        outcome
            .error
            .as_deref()
            .is_some_and(|value| value.contains("stdout"))
    );
}

#[test]
fn deadline_and_cooperative_cancel_terminate_the_group() {
    let (_temporary, mut deadline_spec) = fixture_spec("timeout");
    deadline_spec.limits.deadline = Some(Duration::from_millis(50));
    let deadline = ProcessSupervisor
        .invoke(
            &deadline_spec,
            &request(),
            &CancellationToken::new(),
            |_| {},
        )
        .expect("deadline invoke");
    assert_eq!(deadline.kind, InvocationKind::DeadlineExceeded);

    let (_temporary, mut cancel_spec) = fixture_spec("timeout");
    cancel_spec.limits.deadline = None;
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        trigger.cancel();
    });
    let cancelled = ProcessSupervisor
        .invoke(&cancel_spec, &request(), &cancellation, |_| {})
        .expect("cancel invoke");
    assert_eq!(cancelled.kind, InvocationKind::Cancelled);
}

#[test]
fn deadline_terminates_descendant_after_the_group_leader_exits() {
    let (temporary, mut spec) = fixture_spec("orphan-pipe");
    spec.limits.deadline = Some(Duration::from_millis(150));
    let started = Instant::now();
    let outcome = ProcessSupervisor
        .invoke(&spec, &request(), &CancellationToken::new(), |_| {})
        .expect("deadline invoke");
    assert_eq!(outcome.kind, InvocationKind::DeadlineExceeded);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(temporary.path().join("descendant.started").is_file());
    thread::sleep(Duration::from_millis(550));
    assert!(
        !temporary.path().join("descendant.survived").exists(),
        "the contained descendant survived the deadline"
    );
}

#[test]
fn blocked_frame_sink_cannot_defeat_supervision() {
    let (_temporary, mut spec) = fixture_spec("stream");
    spec.limits.deadline = Some(Duration::from_millis(75));
    let started = Instant::now();
    let outcome = ProcessSupervisor
        .invoke(&spec, &request(), &CancellationToken::new(), |_| {
            thread::sleep(Duration::from_secs(5));
        })
        .expect("deadline invoke");
    assert_eq!(outcome.kind, InvocationKind::DeadlineExceeded);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a blocked event consumer froze supervision"
    );
}

#[test]
fn outer_deadline_cascades_through_dispatch_and_provider_host() {
    let (_temporary, project) = initialized_project();
    let marker = project.join("nested-native");
    let fixture = env!("CARGO_BIN_EXE_tactus-plugin-fixture");
    let mut params = Map::new();
    params.insert("prompt".to_owned(), serde_json::json!("nested containment"));
    params.insert(
        "workspace".to_owned(),
        serde_json::json!(project.to_string_lossy()),
    );
    params.insert(
        "options".to_owned(),
        serde_json::json!({
            "command_prefix":[fixture, "native-orphan-provider"],
            "timeout_seconds":30,
            "extra_env":{
                "FAKE_ORPHAN_MARKER":marker.to_string_lossy(),
                "FAKE_ORPHAN_SURVIVE_MS":"2500"
            }
        }),
    );
    let nested_request =
        PluginRequest::new("nested-run", "invoke", params).expect("nested request");
    let mut spec = ProcessSpec::new(
        vec![
            env!("CARGO_BIN_EXE_tactus").to_owned(),
            "dispatch".to_owned(),
            "--namespace".to_owned(),
            "provider".to_owned(),
            "--name".to_owned(),
            "codex".to_owned(),
            "--root".to_owned(),
            project.to_string_lossy().into_owned(),
            "--timeout-seconds".to_owned(),
            "30".to_owned(),
        ],
        &project,
    );
    spec.limits.deadline = Some(Duration::from_millis(750));
    let started = Instant::now();
    let outcome = ProcessSupervisor
        .invoke(&spec, &nested_request, &CancellationToken::new(), |_| {})
        .expect("nested dispatch");
    assert_eq!(outcome.kind, InvocationKind::DeadlineExceeded);
    // This path crosses three freshly started Windows processes and performs
    // nested group cleanup. Keep the assertion well below the adapter's
    // 30-second deadline without coupling it to CI scheduler latency.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the outer deadline did not bound nested process cleanup"
    );
    assert!(marker.with_extension("started").is_file());
    thread::sleep(Duration::from_millis(2_600));
    assert!(
        !marker.with_extension("survived").exists(),
        "a nested provider descendant survived the outer deadline"
    );
}

#[cfg(unix)]
#[test]
fn deadline_is_bounded_when_an_escaped_descendant_holds_every_pipe() {
    let (temporary, mut spec) = fixture_spec("escaped-pipe");
    spec.limits.deadline = Some(Duration::from_millis(100));
    let started = Instant::now();
    let outcome = ProcessSupervisor
        .invoke(&spec, &request(), &CancellationToken::new(), |_| {})
        .expect("escaped invocation");
    assert_eq!(outcome.kind, InvocationKind::DeadlineExceeded);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(temporary.path().join("escaped.started").is_file());
    thread::sleep(Duration::from_millis(1_250));
}

#[cfg(unix)]
fn term_ignoring_dispatch(
    marker_name: &str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    ProcessSpec,
    PluginRequest,
) {
    let (temporary, project) = initialized_project();
    let marker = project.join(marker_name);
    let script = r#"trap '' TERM
touch "${FAKE_ORPHAN_MARKER}.started"
(trap '' TERM; sleep 2; touch "${FAKE_ORPHAN_MARKER}.survived") &
wait
"#;
    let mut params = Map::new();
    params.insert("prompt".to_owned(), serde_json::json!("nested containment"));
    params.insert(
        "workspace".to_owned(),
        serde_json::json!(project.to_string_lossy()),
    );
    params.insert(
        "options".to_owned(),
        serde_json::json!({
            "command_prefix":["sh", "-c", script, "tactus-term-fixture"],
            "timeout_seconds":30,
            "extra_env":{"FAKE_ORPHAN_MARKER":marker.to_string_lossy()}
        }),
    );
    let nested_request = PluginRequest::new("term-run", "invoke", params).expect("nested request");
    let spec = ProcessSpec::new(
        vec![
            env!("CARGO_BIN_EXE_tactus").to_owned(),
            "dispatch".to_owned(),
            "--namespace".to_owned(),
            "provider".to_owned(),
            "--name".to_owned(),
            "codex".to_owned(),
            "--root".to_owned(),
            project.to_string_lossy().into_owned(),
            "--timeout-seconds".to_owned(),
            "30".to_owned(),
        ],
        &project,
    );
    (temporary, marker, spec, nested_request)
}

#[cfg(unix)]
#[test]
fn outer_term_cascade_hard_kills_a_term_ignoring_provider_group() {
    let (_temporary, marker, mut spec, nested_request) =
        term_ignoring_dispatch("term-ignoring-deadline");
    spec.limits.deadline = Some(Duration::from_millis(750));
    let outcome = ProcessSupervisor
        .invoke(&spec, &nested_request, &CancellationToken::new(), |_| {})
        .expect("nested dispatch");
    assert_eq!(outcome.kind, InvocationKind::DeadlineExceeded);
    assert!(marker.with_extension("started").is_file());
    thread::sleep(Duration::from_millis(2_100));
    assert!(!marker.with_extension("survived").exists());
}

#[cfg(unix)]
#[test]
fn outer_token_cancel_cascades_before_hard_kill() {
    let (_temporary, marker, mut spec, nested_request) =
        term_ignoring_dispatch("term-ignoring-cancel");
    spec.limits.deadline = Some(Duration::from_secs(30));
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(750));
        trigger.cancel();
    });
    let outcome = ProcessSupervisor
        .invoke(&spec, &nested_request, &cancellation, |_| {})
        .expect("nested dispatch");
    assert_eq!(outcome.kind, InvocationKind::Cancelled);
    assert!(marker.with_extension("started").is_file());
    thread::sleep(Duration::from_millis(2_100));
    assert!(!marker.with_extension("survived").exists());
}

#[test]
fn workspace_init_is_idempotent_and_scripts_are_ordered() {
    let temporary = tempdir().expect("temporary directory");
    let sdk = temporary.path().join("sdk");
    fs::create_dir(&sdk).expect("sdk dir");
    fs::write(sdk.join("clef-sdk.cabal"), "name: clef-sdk\n").expect("sdk cabal");
    let first =
        initialize_workspace(temporary.path().join("project"), Some(&sdk)).expect("first init");
    let second = initialize_workspace(&first.workspace.root, Some(&sdk)).expect("second init");
    assert!(!first.created.is_empty());
    assert!(second.created.is_empty());
    assert_eq!(first.created.len(), second.preserved.len());

    fs::write(
        first.workspace.scripts_path.join("020_compose.hs"),
        "main= pure ()",
    )
    .expect("entry two");
    fs::write(
        first.workspace.scripts_path.join("010_atoms.hs"),
        "main= pure ()",
    )
    .expect("entry one");
    fs::write(
        first.workspace.scripts_path.join("Geometry.hs"),
        "module Geometry where",
    )
    .expect("helper");
    let opened = Workspace::open(&first.workspace.root).expect("open");
    let scripts = discover_scripts(&opened).expect("scripts");
    assert_eq!(scripts[0].order, Some(10));
    assert_eq!(scripts[1].order, Some(20));
    assert!(!scripts[2].runnable);
    let runtime = opened.runtime_json().expect("runtime JSON");
    assert!(runtime["plugins"].is_object());
    assert_eq!(runtime["providers"]["codex"]["command"][1], "dispatch");
}

#[test]
fn dispatch_maps_generic_method_transport_failure_to_one_outcome_unknown() {
    let (_temporary, project) = initialized_project();
    let config_path = project.join(".tactus/tactus.toml");
    let mut config = fs::read_to_string(&config_path).expect("config");
    let executable = serde_json::to_string(env!("CARGO_BIN_EXE_tactus-plugin-fixture"))
        .expect("quote executable");
    config.push_str(&format!(
        "fake = {{ command = [{executable}, \"duplicate\"] }}\n"
    ));
    fs::write(config_path, config).expect("config");

    let mut child = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "dispatch",
            "--namespace",
            "plugin",
            "--name",
            "fake",
            "--root",
        ])
        .arg(&project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn dispatch");
    let request = PluginRequest::new("outer", "topology.compute", Map::new()).expect("request");
    serde_json::to_writer(child.stdin.as_mut().expect("stdin"), &request).expect("request JSON");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"\n")
        .expect("LF");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("dispatch output");
    assert!(!output.status.success());
    let frames = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| decode_frame(line).expect("outer frame"))
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 1);
    match &frames[0] {
        PluginFrame::Result {
            ok: false,
            error: JsonField::Present(error),
            ..
        } => {
            assert_eq!(error.code, "outcome_unknown");
            assert_eq!(
                error.details.as_ref().expect("details")["cause"],
                "tactus.protocol_failed"
            );
        }
        frame => panic!("unexpected frame {frame:?}"),
    }
}

#[test]
fn dispatch_exits_when_its_stdout_consumer_stops_reading() {
    let (_temporary, project) = initialized_project();
    let config_path = project.join(".tactus/tactus.toml");
    let mut config = fs::read_to_string(&config_path).expect("config");
    let executable = serde_json::to_string(env!("CARGO_BIN_EXE_tactus-plugin-fixture"))
        .expect("quote executable");
    config.push_str(&format!(
        "blocked = {{ command = [{executable}, \"flood\"] }}\n"
    ));
    fs::write(config_path, config).expect("config");

    let mut child = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "dispatch",
            "--namespace",
            "plugin",
            "--name",
            "blocked",
            "--root",
        ])
        .arg(&project)
        .args(["--timeout-seconds", "2"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn dispatch");
    let request = PluginRequest::new("blocked", "invoke", Map::new()).expect("request");
    serde_json::to_writer(child.stdin.as_mut().expect("stdin"), &request).expect("request JSON");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"\n")
        .expect("LF");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait dispatch") {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "dispatch was frozen by its blocked stdout"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert!(!status.success());
}

#[test]
fn invocation_journal_does_not_copy_plugin_arguments_or_credentials() {
    let (_temporary, project) = initialized_project();
    let config_path = project.join(".tactus/tactus.toml");
    let mut config = fs::read_to_string(&config_path).expect("config");
    let executable = serde_json::to_string(env!("CARGO_BIN_EXE_tactus-plugin-fixture"))
        .expect("quote executable");
    let secret = "credential=do-not-journal-this";
    config.push_str(&format!(
        "audit = {{ command = [{executable}, \"success\", {secret:?}] }}\n"
    ));
    fs::write(config_path, config).expect("config");

    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "plugin-call",
            "audit",
            "topology.compute",
            "--namespace",
            "plugin",
            "--root",
        ])
        .arg(&project)
        .arg("--json")
        .output()
        .expect("plugin call");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("report");
    let run_path = report["run_path"].as_str().expect("run path");
    let journal =
        fs::read_to_string(std::path::Path::new(run_path).join("events.jsonl")).expect("journal");
    assert!(!journal.contains(secret));
    let started: serde_json::Value =
        serde_json::from_str(journal.lines().next().expect("started event")).expect("event");
    assert_eq!(started["kind"], "invocation.started");
    assert_eq!(started["data"]["namespace"], "plugin");
    assert_eq!(started["data"]["argument_count"], 2);
    assert!(started["data"].get("command").is_none());
}

#[test]
fn spawn_failure_still_publishes_a_terminal_run_summary() {
    let (_temporary, project) = initialized_project();
    let config_path = project.join(".tactus/tactus.toml");
    let mut config = fs::read_to_string(&config_path).expect("config");
    config.push_str("missing = { command = [\"definitely-not-a-real-tactus-plugin-7f98c\"] }\n");
    fs::write(config_path, config).expect("config");
    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "plugin-call",
            "missing",
            "inspect",
            "--namespace",
            "plugin",
            "--root",
        ])
        .arg(&project)
        .output()
        .expect("plugin call");
    assert!(!output.status.success());
    let runs = fs::read_dir(project.join(".tactus/runs"))
        .expect("runs")
        .collect::<Result<Vec<_>, _>>()
        .expect("run entries");
    assert_eq!(runs.len(), 1);
    let summary: Value =
        serde_json::from_slice(&fs::read(runs[0].path().join("summary.json")).expect("summary"))
            .expect("summary json");
    assert_eq!(summary["outcome"]["kind"], "runtime_failed");
    assert!(summary["outcome"]["error"].as_str().is_some());
}

#[test]
fn plugin_call_and_smoke_inject_typed_registry_defaults() {
    let (_temporary, project) = initialized_project();
    let config_path = project.join(".tactus/tactus.toml");
    let config = fs::read_to_string(&config_path).expect("config");
    let executable = serde_json::to_string(env!("CARGO_BIN_EXE_tactus-plugin-fixture"))
        .expect("quote executable");
    let section = "[providers.codex]\ncommand = [\"tactus\", \"provider-host\", \"codex\"]";
    let replacement = format!(
        "[providers.codex]\ncommand = [{executable}, \"echo-params\"]\nmodel = \"typed-model\"\neffort = \"high\"\noptions = {{ configured = \"yes\", overridden = \"config\" }}"
    );
    fs::write(config_path, config.replace(section, &replacement)).expect("config");
    let canonical_project = Workspace::open(&project)
        .expect("canonical workspace")
        .root
        .to_string_lossy()
        .into_owned();

    let call = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args([
            "plugin-call",
            "codex",
            "inspect",
            "--namespace",
            "provider",
            "--root",
        ])
        .arg(&project)
        .args(["--params", r#"{"options":{"overridden":"call"}}"#, "--json"])
        .output()
        .expect("plugin call");
    assert!(
        call.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&call.stderr)
    );
    let call: Value = serde_json::from_slice(&call.stdout).expect("call report");
    let call_params = &call["summary"]["outcome"]["terminal"]["value"];
    assert_eq!(call_params["workspace"], canonical_project);
    assert_eq!(call_params["model"], "typed-model");
    assert_eq!(call_params["effort"], "high");
    assert_eq!(call_params["options"]["configured"], "yes");
    assert_eq!(call_params["options"]["overridden"], "call");

    let smoke = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["smoke", "provider:codex", "--root"])
        .arg(&project)
        .arg("--json")
        .output()
        .expect("smoke");
    assert!(smoke.status.success());
    let smoke: Value = serde_json::from_slice(&smoke.stdout).expect("smoke report");
    let smoke_params = &smoke[0]["summary"]["outcome"]["terminal"]["value"];
    assert_eq!(smoke_params["workspace"], canonical_project);
    assert_eq!(smoke_params["model"], "typed-model");
    assert_eq!(smoke_params["effort"], "high");
    assert_eq!(smoke_params["options"]["configured"], "yes");
    assert_eq!(smoke_params["live"], false);
}

#[test]
fn generate_offline_creates_an_ordered_multi_step_workflow() {
    let (_temporary, project) = initialized_project();
    set_command(
        &project,
        "command = [\"tactus\", \"provider-host\", \"codex\"]",
        "generate",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["generate", "--root"])
        .arg(&project)
        .args(["--provider", "codex", "--json", "compute planar holes"])
        .output()
        .expect("generate");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("report");
    assert_eq!(report["ok"], true);
    assert_eq!(report["scripts"].as_array().expect("scripts").len(), 3);
    assert_eq!(report["effects"].as_array().expect("effects").len(), 2);
    assert!(
        report["observer_errors"]
            .as_array()
            .expect("observer errors")
            .is_empty()
    );
    assert!(
        project
            .join(".tactus/scripts/010_atomic_regions.hs")
            .is_file()
    );
    assert!(
        project
            .join(".tactus/scripts/030_compose_holes.hs")
            .is_file()
    );
}

#[test]
fn generate_distinguishes_provider_success_from_a_script_delta() {
    let (_temporary, project) = initialized_project();
    set_command(
        &project,
        "command = [\"tactus\", \"provider-host\", \"codex\"]",
        "success",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["generate", "--root"])
        .arg(&project)
        .args(["--provider", "codex", "--json", "make no files"])
        .output()
        .expect("generate");
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("report");
    assert_eq!(report["provider_ok"], true);
    assert_eq!(report["ok"], false);
    assert!(
        report["generated_delta"]
            .as_array()
            .expect("delta")
            .is_empty()
    );
    assert!(
        report["generation_error"]
            .as_str()
            .is_some_and(|message| message.contains("numbered Haskell entry"))
    );
}

#[test]
fn generate_text_mode_prints_structured_provider_failure() {
    let (_temporary, project) = initialized_project();
    set_command(
        &project,
        "command = [\"tactus\", \"provider-host\", \"codex\"]",
        "reported-failure",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["generate", "--root"])
        .arg(&project)
        .args(["--provider", "codex", "offline failure"])
        .output()
        .expect("generate");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("generation_rejected"), "{stderr}");
    assert!(
        stderr.contains("the requested workflow could not be generated"),
        "{stderr}"
    );
}

#[test]
fn smoke_without_selectors_checks_every_registry() {
    let (_temporary, project) = initialized_project();
    for needle in [
        "command = [\"tactus\", \"provider-host\", \"codex\"]",
        "command = [\"tactus\", \"provider-host\", \"claude-code\"]",
        "command = [\"tactus\", \"provider-host\", \"opencode\"]",
    ] {
        set_command(&project, needle, "success");
    }
    let config_path = project.join(".tactus/tactus.toml");
    let mut config = fs::read_to_string(&config_path).expect("config");
    let executable = serde_json::to_string(env!("CARGO_BIN_EXE_tactus-plugin-fixture"))
        .expect("quote executable");
    config.push_str(&format!(
        "fake = {{ command = [{executable}, \"success\"] }}\n"
    ));
    fs::write(config_path, config).expect("config");
    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["smoke", "--root"])
        .arg(&project)
        .arg("--json")
        .output()
        .expect("smoke");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reports: serde_json::Value = serde_json::from_slice(&output.stdout).expect("reports");
    assert_eq!(reports.as_array().expect("report array").len(), 5);
}

#[test]
#[ignore = "requires the GHC/Cabal toolchain; run explicitly in cross-language CI"]
fn haskell_generic_plugin_routes_through_absolute_tactus_dispatch() {
    let temporary = tempdir().expect("temporary directory");
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository");
    let sdk = repository.join("clef-sdk");
    let project = temporary.path().join("haskell-e2e");
    initialize_workspace(&project, Some(&sdk)).expect("init");
    let config_path = project.join(".tactus/tactus.toml");
    let mut config = fs::read_to_string(&config_path).expect("config");
    let executable = serde_json::to_string(env!("CARGO_BIN_EXE_tactus-plugin-fixture"))
        .expect("quote executable");
    config.push_str(&format!(
        "holes = {{ command = [{executable}, \"success\"] }}\n"
    ));
    fs::write(config_path, config).expect("config");
    let script = project.join(".tactus/scripts/010_generic_plugin.hs");
    fs::write(
        &script,
        r#"{-# LANGUAGE OverloadedStrings #-}
import Clef
import Data.Aeson (object, (.=))

main :: IO ()
main = runTactus (call (rawPlugin "holes" "holes.compute") (object ["regions" .= (3 :: Int)])) >>= print
"#,
    )
    .expect("script");
    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["run", "--root"])
        .arg(&project)
        .arg("--script")
        .arg(&script)
        .arg("--timeout-seconds=600")
        .output()
        .expect("Haskell workflow");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("holes"));
}

#[test]
#[ignore = "requires GHC/Cabal; run explicitly as the offline multi-step acceptance test"]
fn haskell_topology_workflow_runs_all_stages_with_parallel_reviews() {
    let temporary = tempdir().expect("temporary directory");
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository");
    let sdk = repository.join("clef-sdk");
    let project = temporary.path().join("topology-e2e");
    let initialized = initialize_workspace(&project, Some(&sdk)).expect("init");
    for source in fs::read_dir(repository.join("examples/topology-holes/workflow"))
        .expect("workflow directory")
    {
        let source = source.expect("workflow source");
        fs::copy(
            source.path(),
            initialized.workspace.scripts_path.join(source.file_name()),
        )
        .expect("copy workflow source");
    }
    set_command(
        &project,
        "command = [\"tactus\", \"provider-host\", \"codex\"]",
        "topology-stage",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_tactus"))
        .args(["run", "--root"])
        .arg(&project)
        .args(["--timeout-seconds", "600"])
        .output()
        .expect("run multi-step workflow");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for marker in ["stage-010.ok", "stage-020.ok", "stage-030.ok", "final.json"] {
        assert!(project.join("solution").join(marker).is_file(), "{marker}");
    }
    let actual: Value = serde_json::from_slice(
        &fs::read(project.join("solution/final.json")).expect("final marker"),
    )
    .expect("final JSON");
    let expected: Value = serde_json::from_slice(
        &fs::read(repository.join("examples/topology-holes/fixtures/two-holes.expected.json"))
            .expect("expected fixture"),
    )
    .expect("expected JSON");
    assert_eq!(actual, expected);
    assert_eq!(actual["holes"], 2);
    assert_eq!(actual["eulerCharacteristic"], -1);

    let interval = |task: &str| {
        let value = fs::read_to_string(
            project
                .join(".tactus/test-evidence")
                .join(format!("{task}.interval")),
        )
        .expect("review interval");
        let (start, end) = value.trim().split_once(',').expect("interval fields");
        (
            start.parse::<u128>().expect("start millis"),
            end.parse::<u128>().expect("end millis"),
        )
    };
    let algorithm = interval("topology-algorithm-review");
    let interface = interval("topology-interface-review");
    assert!(
        algorithm.0 < interface.1 && interface.0 < algorithm.1,
        "review tasks did not overlap: {algorithm:?} vs {interface:?}"
    );

    let run_directories = fs::read_dir(project.join(".tactus/runs"))
        .expect("run journals")
        .collect::<Result<Vec<_>, _>>()
        .expect("run entries");
    let summaries = run_directories
        .iter()
        .filter(|entry| entry.path().join("summary.json").is_file())
        .count();
    assert!(
        summaries >= 18,
        "expected provider/effect journals, got {summaries}"
    );
    let events = run_directories
        .iter()
        .filter_map(|entry| fs::read_to_string(entry.path().join("events.jsonl")).ok())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(events.contains(r#""method":"observe.begin""#));
    assert!(events.contains(r#""method":"observe.end""#));
}
