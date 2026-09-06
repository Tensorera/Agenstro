//! A one-shot experiment effect. The public process supervises a private worker;
//! that worker owns a Bubblewrap PID namespace and bounded diagnostic drains.
//! Killing either owner destroys the namespace, including setsid descendants.

use std::{
    collections::BTreeMap,
    env,
    io::{self, Read, Write},
    process::ExitCode,
};

#[cfg(target_os = "linux")]
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{Map, Value, json};
#[cfg(target_os = "linux")]
use tactus_runtime::{
    CancellationToken, InvocationKind, ProcessSpec, ProcessSupervisor, TerminalResult,
};
use tactus_runtime::{PluginFailure, PluginRequest, RequestId, decode_request};

#[cfg(target_os = "linux")]
mod sandbox;

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;
const LOG_LIMIT: u64 = 1024 * 1024;
const MAX_TIMEOUT_SECONDS: u64 = 600;
// Tactus allows up to 500 ms to terminate its owned group. Reserve that time
// *inside* the experiment budget, including its 10 ms polling interval.
#[cfg(target_os = "linux")]
const CLEANUP_RESERVE: Duration = Duration::from_millis(550);

type Result<T> = std::result::Result<T, PluginFailure>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunParams {
    run_id: String,
    sample_id: String,
    argv: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    options: BTreeMap<String, Value>,
}

const fn default_timeout() -> u64 {
    MAX_TIMEOUT_SECONDS
}

impl RunParams {
    fn parse(params: Map<String, Value>) -> Result<Self> {
        let parsed: Self = serde_json::from_value(Value::Object(params))
            .map_err(|error| failure("invalid_params", error.to_string()))?;
        for (label, value) in [("run_id", &parsed.run_id), ("sample_id", &parsed.sample_id)] {
            if value.is_empty()
                || value.len() > 100
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            {
                return Err(failure(
                    "invalid_params",
                    format!(
                        "{label} must contain 1..100 ASCII letters, digits, underscores or hyphens"
                    ),
                ));
            }
        }
        if !(1..=MAX_TIMEOUT_SECONDS).contains(&parsed.timeout_seconds) {
            return Err(failure(
                "invalid_params",
                "timeout_seconds must be an integer in 1..600",
            ));
        }
        if parsed.argv.is_empty()
            || parsed.argv.len() > 256
            || parsed.argv[0].is_empty()
            || parsed.argv.iter().any(|arg| arg.contains('\0'))
        {
            return Err(failure(
                "invalid_params",
                "argv must contain 1..256 strings and a non-empty executable, without NUL bytes",
            ));
        }
        if !parsed.options.is_empty() {
            return Err(failure(
                "invalid_params",
                "motivo.test currently accepts no plugin options",
            ));
        }
        if parsed
            .workspace
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.contains('\0'))
        {
            return Err(failure(
                "invalid_params",
                "workspace must be a non-empty path without NUL bytes",
            ));
        }
        Ok(parsed)
    }

    #[cfg(target_os = "linux")]
    fn relative_dir(&self) -> PathBuf {
        PathBuf::from(".tactus/motivotest")
            .join(&self.run_id)
            .join(&self.sample_id)
    }
}

fn failure(code: impl Into<String>, message: impl Into<String>) -> PluginFailure {
    PluginFailure {
        code: code.into(),
        message: message.into(),
        details: None,
    }
}

fn io_failure(error: io::Error) -> PluginFailure {
    failure("io_error", error.to_string())
}

fn emit(value: &Value) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

fn terminal(id: &RequestId, result: Result<Value>) -> io::Result<()> {
    emit(&match result {
        Ok(value) => json!({"type":"result", "id":id, "ok":true, "value":value}),
        Err(error) => json!({"type":"result", "id":id, "ok":false, "error":error}),
    })
}

fn read_request() -> Result<PluginRequest> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_failure)?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err(failure("invalid_request", "request exceeds 1 MiB"));
    }
    decode_request(&bytes).map_err(|error| failure("invalid_request", error.to_string()))
}

fn describe() -> Value {
    json!({
        "name":"motivo.test", "version":env!("CARGO_PKG_VERSION"), "kind":"effect",
        "methods":["describe","smoke","run"], "platforms":["linux"],
        "backend":"/usr/bin/bwrap", "network":false,
        "timeout_seconds":{"default":600,"minimum":1,"maximum":600,"scope":"one experiment, including preparation and all commands"},
        "params":{"required":["run_id","sample_id","argv"],"optional":["timeout_seconds"],"options":{}},
        "project_mount":"/project (read-only)", "working_directory":"/work",
        "writable_directory":".tactus/motivotest/<run_id>/<sample_id>",
        "retained_bytes_per_log":LOG_LIMIT,
        "sample_ids":"1..100 ASCII letters, digits, underscores or hyphens; each sample executes once",
        "note":"A nonzero command exit is an observation, not a claim that the test passed. Host credentials and host runtime sockets are not mounted. Project files are readable."
    })
}

fn smoke(params: &Map<String, Value>) -> Result<Value> {
    if params
        .get("live")
        .is_some_and(|value| value != &Value::Bool(false))
    {
        return Err(failure(
            "invalid_params",
            "smoke is offline; live must be false",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        if !Path::new("/usr/bin/bwrap").is_file() {
            return Err(failure(
                "backend_unavailable",
                "/usr/bin/bwrap is not installed",
            ));
        }
        Ok(
            json!({"available":true,"offline":true,"backend":"/usr/bin/bwrap",
            "execution_verified":false,"note":"Checks backend presence only; run verifies namespace and mount support without fallback."}),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(failure(
            "unsupported_platform",
            "motivo.test currently requires Linux and Bubblewrap",
        ))
    }
}

#[cfg(target_os = "linux")]
fn outcome_metadata(params: &RunParams, status: &str, started: Instant) -> Value {
    let relative = params.relative_dir();
    json!({
        "status":status,"exit_code":null,"signal":null,
        "duration_ms":u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "timeout_seconds":params.timeout_seconds,
        "workspace_dir":relative,
        "stdout_path":relative.join(".motivo-logs/stdout.log"),
        "stderr_path":relative.join(".motivo-logs/stderr.log"),
        "logs_may_be_partial":true,
        "stdout_truncated":null,"stderr_truncated":null
    })
}

fn run(request: &PluginRequest) -> Result<Value> {
    let params = RunParams::parse(request.params.clone())?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = params;
        Err(failure(
            "unsupported_platform",
            "motivo.test currently requires Linux and Bubblewrap",
        ))
    }

    #[cfg(target_os = "linux")]
    {
        let started = Instant::now();
        let token = CancellationToken::new();
        let signal_token = token.clone();
        ctrlc::set_handler(move || signal_token.cancel())
            .map_err(|error| failure("signal_handler", error.to_string()))?;
        let executable = env::current_exe().map_err(io_failure)?;
        let cwd = env::current_dir().map_err(io_failure)?;
        let mut spec = ProcessSpec::new(
            vec![
                executable.to_string_lossy().into_owned(),
                "--worker".into(),
                std::process::id().to_string(),
            ],
            cwd,
        );
        spec.limits.deadline = Some(
            Duration::from_secs(params.timeout_seconds)
                .saturating_sub(started.elapsed())
                .saturating_sub(CLEANUP_RESERVE),
        );
        spec.limits.max_stdout_bytes = 64 * 1024;
        spec.limits.max_stderr_bytes = 16 * 1024;
        spec.limits.max_frame_bytes = 32 * 1024;
        let outcome = ProcessSupervisor
            .invoke(&spec, request, &token, |frame| {
                if let tactus_runtime::PluginFrame::Event { .. } = frame
                    && let Ok(value) = serde_json::to_value(frame)
                {
                    let _ = emit(&value);
                }
            })
            .map_err(|error| failure("worker_start_failed", error.to_string()))?;
        match outcome.kind {
            InvocationKind::Succeeded => match outcome.terminal {
                Some(TerminalResult::Success { mut value }) => {
                    value["duration_ms"] =
                        json!(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
                    Ok(value)
                }
                _ => Err(failure(
                    "worker_failed",
                    "worker completed without a result",
                )),
            },
            InvocationKind::PluginFailed => match outcome.terminal {
                Some(TerminalResult::Failure { error }) => Err(error),
                _ => Err(failure("worker_failed", "worker failed without an error")),
            },
            InvocationKind::DeadlineExceeded => Ok(outcome_metadata(&params, "timed_out", started)),
            InvocationKind::Cancelled => Ok(outcome_metadata(&params, "cancelled", started)),
            _ => {
                let mut error = failure(
                    "worker_failed",
                    outcome
                        .error
                        .unwrap_or_else(|| format!("worker ended as {:?}", outcome.kind)),
                );
                error.details = Some(outcome_metadata(&params, "execution_error", started));
                Err(error)
            }
        }
    }
}

fn main() -> ExitCode {
    let worker = env::args().nth(1).as_deref() == Some("--worker");
    #[cfg(target_os = "linux")]
    if worker {
        // Arm this before reading stdin. Comparing the expected parent after
        // prctl closes the race where the public supervisor died before arm.
        if nix::sys::prctl::set_pdeathsig(nix::sys::signal::Signal::SIGKILL).is_err()
            || env::args()
                .nth(2)
                .and_then(|value| value.parse::<i32>().ok())
                != Some(nix::unistd::getppid().as_raw())
        {
            eprintln!("worker has no live owning supervisor");
            return ExitCode::from(2);
        }
    }
    let request = match read_request() {
        Ok(request) => request,
        Err(error) => {
            // An invalid request has no trustworthy correlation ID.
            eprintln!("{}: {}", error.code, error.message);
            return ExitCode::from(2);
        }
    };
    let result = if worker {
        #[cfg(target_os = "linux")]
        {
            sandbox::execute(&request)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(failure("unsupported_platform", "Linux is required"))
        }
    } else {
        match request.method.as_str() {
            "describe" => Ok(describe()),
            "smoke" => smoke(&request.params),
            "run" => run(&request),
            _ => Err(failure(
                "unsupported_method",
                format!("unsupported method {:?}", request.method),
            )),
        }
    };
    match terminal(&request.id, result) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cannot write plugin result: {error}");
            ExitCode::from(1)
        }
    }
}
