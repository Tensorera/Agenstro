#![cfg(target_os = "linux")]

use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::unix::{fs::symlink, net::UnixListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

const BINARY: &str = env!("CARGO_BIN_EXE_motivo-test");

fn workspace() -> TempDir {
    let root = tempdir().unwrap();
    fs::create_dir(root.path().join(".tactus")).unwrap();
    root
}

fn request(params: Value) -> Value {
    json!({"api":"agenstro.plugin/v1","id":"experiment-1","method":"run","params":params})
}

fn params(argv: &[&str], timeout_seconds: u64) -> Value {
    json!({"run_id":"investigation","sample_id":"sample_1","argv":argv,"timeout_seconds":timeout_seconds})
}

fn spawn(root: &Path, request: &Value) -> Child {
    let mut child = Command::new(BINARY)
        .current_dir(root)
        .env("MOTIVO_TEST_SECRET", "must-not-enter-sandbox")
        .env("SSH_AUTH_SOCK", "/run/not-available.sock")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    serde_json::to_writer(&mut stdin, request).unwrap();
    stdin.write_all(b"\n").unwrap();
    drop(stdin);
    child
}

fn call(root: &Path, request: &Value) -> Value {
    let output = spawn(root, request).wait_with_output().unwrap();
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
    assert!(!frames.is_empty());
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame["type"] == "result")
            .count(),
        1
    );
    assert!(frames.iter().all(|frame| frame["id"] == request["id"]));
    let result = frames.last().unwrap();
    assert_eq!(result["type"], "result");
    result.clone()
}

fn value(root: &Path, request: &Value) -> Value {
    let result = call(root, request);
    assert_eq!(result["ok"], true, "{result}");
    result["value"].clone()
}

fn sample(root: &Path) -> PathBuf {
    root.join(".tactus/motivotest/investigation/sample_1")
}

#[test]
fn describe_and_smoke_are_offline_and_factual() {
    let root = workspace();
    let description = value(
        root.path(),
        &json!({"api":"agenstro.plugin/v1","id":1,"method":"describe","params":{}}),
    );
    assert_eq!(description["timeout_seconds"]["maximum"], 600);
    let smoke = value(
        root.path(),
        &json!({"api":"agenstro.plugin/v1","id":2,"method":"smoke","params":{"live":false}}),
    );
    assert_eq!(smoke["offline"], true);
    assert_eq!(smoke["execution_verified"], false);
    assert!(!root.path().join(".tactus/motivotest").exists());
    let invalid = call(
        root.path(),
        &json!({"api":"agenstro.plugin/v1","id":3,"method":"smoke","params":{"live":true}}),
    );
    assert_eq!(invalid["ok"], false);
}

#[test]
fn rejects_invalid_bounds_ids_and_unknown_options_before_writing() {
    let root = workspace();
    for timeout in [
        json!(0),
        json!(601),
        json!(-1),
        json!(1.5),
        json!("10"),
        Value::Null,
    ] {
        let mut input = params(&["/usr/bin/true"], 1);
        input["timeout_seconds"] = timeout;
        assert_eq!(
            call(root.path(), &request(input))["error"]["code"],
            "invalid_params"
        );
    }
    for id in ["..", "../outside", "/outside", "a/b", "a\\b", "", "a b"] {
        let mut input = params(&["/usr/bin/true"], 1);
        input["run_id"] = json!(id);
        assert_eq!(
            call(root.path(), &request(input))["error"]["code"],
            "invalid_params"
        );
    }
    let mut input = params(&["/usr/bin/true"], 1);
    input["options"] = json!({"network":true});
    assert_eq!(
        call(root.path(), &request(input))["error"]["code"],
        "invalid_params"
    );
    assert!(!root.path().join(".tactus/motivotest").exists());
}

#[test]
fn injected_workspace_and_empty_options_are_accepted_without_allowing_redirection() {
    let root = workspace();
    let mut input = params(&["/usr/bin/true"], 10);
    input["workspace"] = json!(root.path());
    input["options"] = json!({});
    assert_eq!(value(root.path(), &request(input))["exit_code"], 0);

    let root = workspace();
    let other = workspace();
    let mut input = params(&["/usr/bin/true"], 10);
    input["workspace"] = json!(other.path());
    let result = call(root.path(), &request(input));
    assert_eq!(result["error"]["code"], "invalid_workspace");
    assert!(!root.path().join(".tactus/motivotest").exists());
    assert!(!other.path().join(".tactus/motivotest").exists());
}

#[test]
fn real_sandbox_allows_sample_writes_and_blocks_project_host_network_and_credentials() {
    let root = workspace();
    fs::write(root.path().join("source.txt"), "original").unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("secret"), "host-only").unwrap();
    let _socket = UnixListener::bind(root.path().join("host.sock")).unwrap();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let code = format!(
        r#"
import json, os, pathlib, socket
assert pathlib.Path('/project/source.txt').read_text() == 'original'
assert os.getcwd() == '/work'
assert not os.environ.get('MOTIVO_TEST_SECRET')
assert not os.environ.get('SSH_AUTH_SOCK')
assert not os.environ.get('DBUS_SESSION_BUS_ADDRESS')
assert os.environ['HOME'] == '/work/home'
for path in ['/project/source.txt', '/project/new-file', '/outside-file', '/project/.tactus/motivotest/investigation/sample_1/alias']:
    try: open(path, 'w').write('bad')
    except OSError: pass
    else: raise AssertionError('unexpected write: '+path)
assert not pathlib.Path({secret:?}).exists()
for family, address in [(socket.AF_UNIX, '/project/host.sock'), (socket.AF_INET, ('127.0.0.1', {port}))]:
    conn=socket.socket(family); conn.settimeout(.2)
    try: conn.connect(address)
    except OSError: pass
    else: raise AssertionError('host socket was reachable')
pathlib.Path('result.txt').write_text('allowed')
pathlib.Path('/tmp/temporary.txt').write_text('temporary')
pathlib.Path(os.environ['HOME']+'/home.txt').write_text('home')
print('isolation passed')
"#,
        secret = outside.path().join("secret").to_string_lossy(),
        port = server.local_addr().unwrap().port()
    );
    let result = value(
        root.path(),
        &request(params(&["/usr/bin/python3", "-c", &code], 10)),
    );
    assert_eq!(
        result["exit_code"],
        0,
        "{}",
        fs::read_to_string(root.path().join(result["stderr_path"].as_str().unwrap())).unwrap()
    );
    assert_eq!(
        fs::read_to_string(root.path().join("source.txt")).unwrap(),
        "original"
    );
    assert_eq!(
        fs::read_to_string(sample(root.path()).join("result.txt")).unwrap(),
        "allowed"
    );
    assert!(sample(root.path()).join("tmp/temporary.txt").is_file());
    assert!(sample(root.path()).join("home/home.txt").is_file());
}

#[test]
fn sample_fixtures_are_supported_but_symlinks_and_hardlinks_are_rejected() {
    let root = workspace();
    fs::create_dir_all(sample(root.path())).unwrap();
    fs::write(sample(root.path()).join("fixture.txt"), "fixture").unwrap();
    let result = value(
        root.path(),
        &request(params(
            &["/bin/sh", "-c", "cat fixture.txt > copied.txt"],
            10,
        )),
    );
    assert_eq!(result["exit_code"], 0);
    assert_eq!(
        fs::read_to_string(sample(root.path()).join("copied.txt")).unwrap(),
        "fixture"
    );
    let duplicate = call(root.path(), &request(params(&["/usr/bin/true"], 10)));
    assert_eq!(duplicate["error"]["code"], "sample_exists");

    for hardlink in [false, true] {
        let root = workspace();
        let outside = tempdir().unwrap();
        fs::create_dir_all(sample(root.path())).unwrap();
        let source = outside.path().join("source");
        fs::write(&source, "do not change").unwrap();
        let fixture = sample(root.path()).join("fixture");
        if hardlink {
            fs::hard_link(&source, &fixture).unwrap();
        } else {
            symlink(&source, &fixture).unwrap();
        }
        let result = call(
            root.path(),
            &request(params(&["/bin/sh", "-c", "echo changed > fixture"], 10)),
        );
        assert_eq!(result["error"]["code"], "unsafe_path");
        assert_eq!(fs::read_to_string(source).unwrap(), "do not change");
    }
}

#[test]
fn symlinked_directory_ancestors_cannot_redirect_writes() {
    for component in [
        ".tactus",
        ".tactus/motivotest",
        ".tactus/motivotest/investigation",
        ".tactus/motivotest/investigation/sample_1",
    ] {
        let root = workspace();
        let outside = tempdir().unwrap();
        let link = root.path().join(component);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        if link.is_dir() {
            fs::remove_dir(&link).unwrap();
        }
        symlink(outside.path(), &link).unwrap();
        let result = call(root.path(), &request(params(&["/usr/bin/true"], 10)));
        assert_eq!(result["ok"], false, "{component}: {result}");
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}

#[test]
fn arguments_are_literal_and_nonzero_exit_is_an_observation() {
    let root = workspace();
    let result = value(
        root.path(),
        &request(params(
            &["/usr/bin/printf", "%s", "$(touch injected); `false` $HOME"],
            10,
        )),
    );
    assert_eq!(
        fs::read_to_string(root.path().join(result["stdout_path"].as_str().unwrap())).unwrap(),
        "$(touch injected); `false` $HOME"
    );
    assert!(!sample(root.path()).join("injected").exists());

    for code in [23, 126, 127] {
        let root = workspace();
        let result = value(
            root.path(),
            &request(params(
                &["/bin/sh", "-c", &format!("echo negative >&2; exit {code}")],
                10,
            )),
        );
        assert_eq!(result["status"], "exited");
        assert_eq!(result["exit_code"], code);
        assert!(!result["logs_may_be_partial"].as_bool().unwrap());
    }
}

#[test]
fn command_start_failure_is_not_a_test_observation() {
    let root = workspace();
    let result = call(
        root.path(),
        &request(params(&["/usr/bin/not-a-real-motivo-tool"], 10)),
    );
    assert_eq!(result["error"]["code"], "command_unavailable");
    let root = workspace();
    let result = value(
        root.path(),
        &request(params(&["/bin/sh", "-c", "exec /missing-tool"], 10)),
    );
    assert_eq!(result["status"], "exited");
    assert_eq!(result["exit_code"], 127);
}

#[test]
fn unavailable_namespaces_fail_without_executing_payload() {
    let root = workspace();
    // This outer namespace disables nested user namespaces. Bubblewrap exists,
    // but the plugin cannot construct its required sandbox and must fail closed.
    let mut child = Command::new("/usr/bin/bwrap")
        .args([
            "--unshare-user",
            "--disable-userns",
            "--unshare-pid",
            "--die-with-parent",
            "--ro-bind",
            "/",
            "/",
        ])
        .arg("--bind")
        .arg(root.path())
        .arg(root.path())
        .args(["--proc", "/proc", "--"])
        .arg(BINARY)
        .current_dir(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = request(params(&["/bin/sh", "-c", "echo bad > fallback-ran"], 10));
    serde_json::to_writer(child.stdin.take().unwrap(), &input).unwrap();
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
    assert!(matches!(
        frames.last().unwrap()["error"]["code"].as_str(),
        Some("sandbox_unavailable" | "sandbox_start_failed")
    ));
    assert!(!sample(root.path()).join("fallback-ran").exists());
    assert!(!root.path().join("fallback-ran").exists());
}

#[test]
fn output_is_drained_with_fixed_retention_and_logs_are_read_only_in_sandbox() {
    let root = workspace();
    let script = "import os; os.write(1,b'x'*3000000); os.write(2,b'y'*3000000)";
    let result = value(
        root.path(),
        &request(params(&["/usr/bin/python3", "-c", script], 10)),
    );
    assert_eq!(result["exit_code"], 0);
    for stream in ["stdout", "stderr"] {
        assert_eq!(result[format!("{stream}_truncated")], true);
        assert_eq!(
            fs::metadata(
                root.path()
                    .join(result[format!("{stream}_path")].as_str().unwrap())
            )
            .unwrap()
            .len(),
            1024 * 1024
        );
    }
    let root = workspace();
    let result = value(
        root.path(),
        &request(params(
            &[
                "/bin/sh",
                "-c",
                "if echo changed > .motivo-logs/stdout.log; then exit 77; fi; echo observed",
            ],
            10,
        )),
    );
    assert_eq!(result["exit_code"], 0);
    assert_eq!(
        fs::read_to_string(sample(root.path()).join(".motivo-logs/stdout.log")).unwrap(),
        "observed\n"
    );
}

#[test]
fn single_deadline_covers_multiple_commands_and_preserves_partial_logs() {
    let root = workspace();
    let started = Instant::now();
    let result = value(
        root.path(),
        &request(params(
            &[
                "/bin/sh",
                "-c",
                "echo preparing; sleep 1; echo prepared; sleep 1; echo too-late > escaped",
            ],
            2,
        )),
    );
    assert_eq!(result["status"], "timed_out");
    assert!(
        started.elapsed() < Duration::from_millis(2300),
        "{:?}",
        started.elapsed()
    );
    let stdout = fs::read_to_string(sample(root.path()).join(".motivo-logs/stdout.log")).unwrap();
    assert!(stdout.contains("preparing\n"));
    assert!(stdout.contains("prepared\n"));
    assert!(!sample(root.path()).join("escaped").exists());
}

const ESCAPING_SCRIPT: &str = "setsid sh -c 'trap \"\" TERM; sleep 1.5; echo escaped > escaped' </dev/null >/dev/null 2>&1 & echo ready > ready; sleep 20";

#[test]
fn timeout_kills_descendants_that_create_a_new_session() {
    let root = workspace();
    let result = value(
        root.path(),
        &request(params(&["/bin/sh", "-c", ESCAPING_SCRIPT], 1)),
    );
    assert_eq!(result["status"], "timed_out");
    assert!(sample(root.path()).join("ready").exists());
    thread::sleep(Duration::from_millis(1600));
    assert!(!sample(root.path()).join("escaped").exists());
}

#[test]
fn ordinary_completion_does_not_leave_background_work_alive() {
    let root = workspace();
    let script = "setsid sh -c 'sleep 1.5; echo escaped > escaped' </dev/null >/dev/null 2>&1 & echo finished";
    let result = value(
        root.path(),
        &request(params(&["/bin/sh", "-c", script], 10)),
    );
    assert_eq!(result["status"], "exited");
    assert_eq!(result["exit_code"], 0);
    thread::sleep(Duration::from_millis(1600));
    assert!(!sample(root.path()).join("escaped").exists());
}

#[test]
fn killing_public_plugin_also_kills_its_worker_and_namespace() {
    let root = workspace();
    let mut child = spawn(
        root.path(),
        &request(params(&["/bin/sh", "-c", ESCAPING_SCRIPT], 20)),
    );
    let until = Instant::now() + Duration::from_secs(5);
    while !sample(root.path()).join("ready").exists() && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(sample(root.path()).join("ready").exists());
    child.kill().unwrap();
    child.wait().unwrap();
    thread::sleep(Duration::from_millis(1600));
    assert!(!sample(root.path()).join("escaped").exists());
    // Closed output pipes also establish that the detached worker is gone.
    let mut remaining = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut remaining)
        .unwrap();
}

#[test]
fn sigterm_cancels_and_returns_one_terminal_result() {
    let root = workspace();
    let child = spawn(
        root.path(),
        &request(params(&["/bin/sh", "-c", ESCAPING_SCRIPT], 20)),
    );
    let until = Instant::now() + Duration::from_secs(5);
    while !sample(root.path()).join("ready").exists() && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(sample(root.path()).join("ready").exists());
    kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
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
    assert_eq!(frames.last().unwrap()["value"]["status"], "cancelled");
    thread::sleep(Duration::from_millis(1600));
    assert!(!sample(root.path()).join("escaped").exists());
}

#[test]
fn duplicate_json_keys_are_protocol_errors_without_execution() {
    let root = workspace();
    let mut child = Command::new(BINARY)
        .current_dir(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(br#"{"api":"agenstro.plugin/v1","id":1,"method":"run","params":{"run_id":"a","run_id":"b"}}"#).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!root.path().join(".tactus/motivotest").exists());
}
