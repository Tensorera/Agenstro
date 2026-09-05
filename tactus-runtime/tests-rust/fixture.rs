use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

fn main() {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "success".to_owned());
    if mode == "orphan-pipe-descendant" {
        orphan_pipe_descendant();
        return;
    }
    #[cfg(unix)]
    if mode == "escaped-pipe-descendant" {
        let cwd = std::env::current_dir().expect("fixture cwd");
        std::fs::write(cwd.join("escaped.started"), b"started").expect("started marker");
        thread::sleep(Duration::from_millis(1_200));
        return;
    }
    if matches!(
        mode.as_str(),
        "native-orphan-provider"
            | "native-orphan-health"
            | "native-orphan-descendant"
            | "native-claude-flood"
    ) {
        if mode == "native-claude-flood" {
            native_claude_flood();
            return;
        }
        native_orphan(&mode);
        return;
    }
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .expect("read request");
    let request: Value = serde_json::from_str(&input).expect("request JSON");
    let id = request.get("id").cloned().expect("request id");
    match mode.as_str() {
        "success" => success(&id, Duration::ZERO, false),
        "stream" => success(&id, Duration::from_millis(400), false),
        "stderr" => success(&id, Duration::ZERO, true),
        "generate" => generate(&request, &id),
        "generate-helper" | "generate-blank-helper" => {
            let workspace = request["params"]["workspace"].as_str().expect("workspace");
            let directory = std::path::Path::new(workspace).join(".tactus/scripts/Helpers");
            std::fs::create_dir_all(&directory).expect("helper directory");
            let source = if mode == "generate-blank-helper" {
                " \n\t"
            } else {
                "module Helpers.Value where\nanswer :: Int\nanswer = 42\n"
            };
            std::fs::write(directory.join("Value.hs"), source).expect("helper source");
            write_json(&serde_json::json!({
                "type":"result", "id":id, "ok":true,
                "value":{"prompt":request["params"]["prompt"]}
            }));
        }
        "topology-stage" => topology_stage(&request, &id),
        "echo-params" => write_json(&serde_json::json!({
            "type":"result",
            "id":id,
            "ok":true,
            "value":request["params"]
        })),
        "reported-failure" => {
            write_json(&serde_json::json!({
                "type":"result",
                "id":id,
                "ok":false,
                "error":{
                    "code":"generation_rejected",
                    "message":"the requested workflow could not be generated"
                }
            }));
        }
        "duplicate" => {
            write_json(&serde_json::json!({"type":"result","id":id,"ok":true,"value":1}));
            write_json(&serde_json::json!({"type":"result","id":id,"ok":true,"value":2}));
        }
        "flood" => {
            let payload = "x".repeat(1024);
            for step in 0..10_000 {
                write_json(&serde_json::json!({
                    "type":"event",
                    "id":id,
                    "event":{"type":"progress","step":step,"payload":payload}
                }));
            }
            write_json(&serde_json::json!({"type":"result","id":id,"ok":true,"value":1}));
        }
        "missing" => {
            write_json(&serde_json::json!({
                "type":"event","id":id,"event":{"type":"progress","step":1}
            }));
        }
        "invalid-utf8" => {
            std::io::stdout()
                .write_all(&[0xff, b'\n'])
                .expect("write invalid bytes");
        }
        "orphan-pipe" => orphan_pipe_parent(),
        #[cfg(unix)]
        "escaped-pipe" => escaped_pipe_parent(),
        "timeout" => thread::sleep(Duration::from_secs(30)),
        other => panic!("unknown fixture mode {other}"),
    }
}

fn native_claude_flood() {
    let mut prompt = Vec::new();
    std::io::stdin()
        .read_to_end(&mut prompt)
        .expect("read native prompt");
    let stdout = std::io::stdout();
    let mut stdout = std::io::BufWriter::new(stdout.lock());
    for sequence in 0..20_000_u64 {
        serde_json::to_writer(
            &mut stdout,
            &serde_json::json!({
                "type":"system",
                "subtype":"thinking_tokens",
                "estimated_tokens":sequence + 1,
                "estimated_tokens_delta":1,
            }),
        )
        .expect("thinking event");
        stdout.write_all(b"\n").expect("thinking LF");
    }
    serde_json::to_writer(
        &mut stdout,
        &serde_json::json!({"type":"result","result":"TACTUS_OK"}),
    )
    .expect("Claude result");
    stdout.write_all(b"\n").expect("result LF");
    stdout.flush().expect("flush flood");
}

#[cfg(unix)]
#[allow(clippy::zombie_processes)] // Deliberately escapes the owned group to test bounded pipe workers.
fn escaped_pipe_parent() {
    use std::os::unix::process::CommandExt as _;

    let mut command = Command::new(std::env::current_exe().expect("fixture executable"));
    command
        .arg("escaped-pipe-descendant")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command.process_group(0);
    command.spawn().expect("spawn escaped pipe descendant");
}

#[allow(clippy::zombie_processes)] // This fixture intentionally leaves the descendant to the supervisor.
fn orphan_pipe_parent() {
    Command::new(std::env::current_exe().expect("fixture executable"))
        .arg("orphan-pipe-descendant")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn pipe-holding descendant");
    let marker = std::env::current_dir()
        .expect("fixture cwd")
        .join("descendant.started");
    let started = Instant::now();
    while !marker.is_file() && started.elapsed() < Duration::from_millis(100) {
        thread::sleep(Duration::from_millis(5));
    }
}

fn orphan_pipe_descendant() {
    let cwd = std::env::current_dir().expect("fixture cwd");
    std::fs::write(cwd.join("descendant.started"), b"started").expect("started marker");
    thread::sleep(Duration::from_millis(500));
    std::fs::write(cwd.join("descendant.survived"), b"survived").expect("survived marker");
}

#[allow(clippy::zombie_processes)] // Parent modes intentionally exercise group cleanup after leader exit.
fn native_orphan(mode: &str) {
    let marker = std::path::PathBuf::from(
        std::env::var_os("FAKE_ORPHAN_MARKER").expect("native orphan marker"),
    );
    if mode == "native-orphan-descendant" {
        std::fs::write(marker.with_extension("started"), b"started").expect("started marker");
        let survive_after = std::env::var("FAKE_ORPHAN_SURVIVE_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(500);
        thread::sleep(Duration::from_millis(survive_after));
        std::fs::write(marker.with_extension("survived"), b"survived").expect("survived marker");
        return;
    }

    let mut prompt = Vec::new();
    std::io::stdin()
        .read_to_end(&mut prompt)
        .expect("read native prompt");
    Command::new(std::env::current_exe().expect("fixture executable"))
        .arg("native-orphan-descendant")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn native pipe-holding descendant");
    let started_marker = marker.with_extension("started");
    let started = Instant::now();
    while !started_marker.is_file() && started.elapsed() < Duration::from_millis(100) {
        thread::sleep(Duration::from_millis(5));
    }
    if mode == "native-orphan-provider" {
        println!(
            r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"TOO_LATE"}}}}"#
        );
    } else {
        println!("fake-cli 1.0");
    }
}

fn generate(request: &Value, id: &Value) {
    let workspace = request["params"]["workspace"]
        .as_str()
        .expect("workspace param");
    let scripts = std::path::Path::new(workspace).join(".tactus/scripts");
    let sources = [
        (
            "010_atomic_regions.hs",
            "main :: IO ()\nmain = print (3 :: Int) -- connected planar regions\n",
        ),
        (
            "020_atomic_boundaries.hs",
            "main :: IO ()\nmain = print (4 :: Int) -- boundary loops\n",
        ),
        (
            "030_compose_holes.hs",
            "-- For one connected planar component: holes = boundary loops - 1\nmain :: IO ()\nmain = print ((4 - 1) :: Int)\n",
        ),
    ];
    for (index, (name, source)) in sources.into_iter().enumerate() {
        std::fs::write(scripts.join(name), source).expect("write generated script");
        write_json(&serde_json::json!({
            "type":"event",
            "id":id,
            "event":{"type":"progress","step":index + 1,"path":name}
        }));
    }
    write_json(&serde_json::json!({
        "type":"result","id":id,"ok":true,"value":{"created":3}
    }));
}

fn topology_stage(request: &Value, id: &Value) {
    let task = request["params"]["task"].as_str().expect("task param");
    let workspace = std::path::Path::new(
        request["params"]["workspace"]
            .as_str()
            .expect("workspace param"),
    );
    let solution = workspace.join("solution");
    let evidence = workspace.join(".tactus/test-evidence");
    std::fs::create_dir_all(&solution).expect("solution directory");
    std::fs::create_dir_all(&evidence).expect("evidence directory");

    let text = match task {
        "topology-contract-and-parser" => {
            std::fs::write(solution.join("stage-010.ok"), b"parser").expect("stage 010");
            stage_report(
                "parser contract complete",
                "solution/stage-010.ok",
                "parser tests",
            )
        }
        "topology-foreground-components" => {
            assert!(
                solution.join("stage-010.ok").is_file(),
                "stage 010 prerequisite"
            );
            std::fs::write(solution.join("stage-020.ok"), b"components").expect("stage 020");
            stage_report(
                "foreground components complete",
                "solution/stage-020.ok",
                "component tests",
            )
        }
        "topology-holes-and-euler" => {
            assert!(
                solution.join("stage-020.ok").is_file(),
                "stage 020 prerequisite"
            );
            std::fs::write(solution.join("stage-030.ok"), b"holes").expect("stage 030");
            stage_report(
                "holes and Euler complete",
                "solution/stage-030.ok",
                "topology tests",
            )
        }
        "topology-algorithm-review" | "topology-interface-review" => {
            assert!(
                solution.join("stage-030.ok").is_file(),
                "stage 030 prerequisite"
            );
            let started = unix_millis();
            thread::sleep(Duration::from_millis(350));
            let finished = unix_millis();
            std::fs::write(
                evidence.join(format!("{task}.interval")),
                format!("{started},{finished}"),
            )
            .expect("review evidence");
            serde_json::to_string(&serde_json::json!({
                "approved":true,
                "findings":[format!("{task} approved")]
            }))
            .expect("review JSON")
        }
        "topology-integrate-cli" => {
            assert!(
                solution.join("stage-030.ok").is_file(),
                "stage 030 prerequisite"
            );
            for review in ["topology-algorithm-review", "topology-interface-review"] {
                assert!(
                    evidence.join(format!("{review}.interval")).is_file(),
                    "review prerequisite {review}"
                );
                assert!(
                    request["params"]["prompt"]
                        .as_str()
                        .is_some_and(|prompt| prompt.contains(&format!("{review} approved"))),
                    "typed finding from {review} was not composed into integration"
                );
            }
            let expected = serde_json::json!({
                "width":9,
                "height":5,
                "solidComponents":1,
                "holes":2,
                "eulerCharacteristic":-1,
                "solidCells":27,
                "backgroundCells":18,
                "holeAreas":[9,9]
            });
            std::fs::write(
                solution.join("final.json"),
                serde_json::to_vec(&expected).expect("final JSON"),
            )
            .expect("final marker");
            stage_report(
                "integrated CLI complete",
                "solution/final.json",
                "complete fixture suite",
            )
        }
        other => panic!("unknown topology task {other}"),
    };
    write_json(&serde_json::json!({
        "type":"result",
        "id":id,
        "ok":true,
        "value":{"text":text}
    }));
}

fn stage_report(summary: &str, file: &str, test: &str) -> String {
    serde_json::to_string(&serde_json::json!({
        "summary":summary,
        "files":[file],
        "testsRun":[test]
    }))
    .expect("stage report JSON")
}

fn unix_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn success(id: &Value, delay: Duration, flood_stderr: bool) {
    write_json(&serde_json::json!({
        "type":"event",
        "id":id,
        "event":{"type":"progress","message":"孔洞","step":1}
    }));
    if flood_stderr {
        let block = vec![b'x'; 256 * 1024];
        std::io::stderr().write_all(&block).expect("write stderr");
    }
    thread::sleep(delay);
    write_json(&serde_json::json!({
        "type":"result","id":id,"ok":true,"value":{"holes":2}
    }));
}

fn write_json(value: &Value) {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, value).expect("write JSON");
    stdout.write_all(b"\n").expect("write LF");
    stdout.flush().expect("flush frame");
}
