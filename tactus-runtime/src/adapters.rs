//! Built-in one-shot provider adapters and the observational workspace path effect.
//!
//! These hosts implement the language-neutral `agenstro.plugin/v1` boundary. They
//! deliberately do not add authentication, policy, content storage, rollback, or
//! artifact semantics. Provider credentials and environment are inherited by the
//! native command.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use command_group::{CommandGroup, GroupChild};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::{
    executable::{ExecutableResolutionError, ExecutableResolver},
    protocol::{PLUGIN_API, PluginFailure, PluginRequest, RequestId, decode_request},
};

const IMPLEMENTATION_VERSION: &str = env!("CARGO_PKG_VERSION");
const OPENCODE_WARNING: &str = "OpenCode --auto only approves ask decisions and does not override explicit deny or managed configuration; full bypass cannot be guaranteed.";
const PATH_EFFECT_NAME: &str = "workspace.paths";
const DEFAULT_SMOKE_TIMEOUT: Duration = Duration::from_secs(20);
const PROCESS_POLL: Duration = Duration::from_millis(10);
const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
// The outer plugin protocol is bounded independently by the validated
// workspace limit (1 MiB by default, up to this 16 MiB host ceiling).
// Native agent CLIs are consumed inside the provider host and may emit larger
// stream-json tool payloads, so their drain budget is independent. Extracted
// result text remains bounded separately before the host emits a plugin frame.
const MAX_NATIVE_LINE_BYTES: usize = 8 * 1024 * 1024;
const MAX_NATIVE_STDOUT_BYTES: usize = 1024 * 1024 * 1024;
const MAX_PROVIDER_RESULT_BYTES: usize = 4 * 1024 * 1024;
const MAX_NATIVE_STDERR_BYTES: usize = 1024 * 1024;
const MAX_HEALTH_STDOUT_BYTES: usize = 1024 * 1024;
// Eight full-size lines bound queued native stdout to roughly 64 MiB while
// preserving backpressure between the pipe reader and JSON parser.
const NATIVE_OUTPUT_QUEUE: usize = 8;
const MAX_SNAPSHOT_PATHS: usize = 100_000;
const MAX_SNAPSHOT_HASH_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SNAPSHOT_DURATION: Duration = Duration::from_secs(30);
const MAX_SNAPSHOT_WARNING_EXAMPLES: usize = 16;
const OBSERVATION_COMPLETION_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_COMPLETION_CLEANUP_ENTRIES: usize = 1_024;
const MAX_COMPLETION_CLEANUP_DURATION: Duration = Duration::from_millis(100);
pub(crate) const SUPERVISED_PROCESS_GROUP_ENV: &str = "TACTUS_SUPERVISED_PROCESS_GROUP";

/// Execute the built-in provider host for one strict JSON request.
///
/// `kind` accepts `codex`, `claude-code` (or `claude`), and `opencode`. Every
/// emitted frame is UTF-8 JSON followed by LF and is flushed immediately.
pub fn run_provider_host<R, W, E>(kind: &str, stdin: R, stdout: W, stderr: E) -> i32
where
    R: Read,
    W: Write,
    E: Write,
{
    let provider = match Provider::parse(kind) {
        Some(provider) => provider,
        None => {
            return run_unknown_host(
                stdin,
                stdout,
                stderr,
                AdapterError::new(
                    "invalid_provider",
                    format!("unknown built-in provider {kind:?}"),
                )
                .with_details(json!({
                    "providers": ["codex", "claude-code", "opencode"]
                })),
            );
        }
    };
    run_host(stdin, stdout, stderr, |request, writer, diagnostics| {
        handle_provider(provider, request, writer, diagnostics)
    })
}

/// Execute the built-in observational `workspace.paths` effect for one request.
pub fn run_workspace_paths_host<R, W, E>(stdin: R, stdout: W, stderr: E) -> i32
where
    R: Read,
    W: Write,
    E: Write,
{
    run_host(stdin, stdout, stderr, |request, writer, _diagnostics| {
        handle_workspace_paths(request, writer)
    })
}

#[derive(Clone, Debug)]
struct AdapterError {
    code: String,
    message: String,
    details: Option<Value>,
}

impl AdapterError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    fn failure(&self) -> PluginFailure {
        PluginFailure {
            code: self.code.clone(),
            message: self.message.clone(),
            details: self.details.clone(),
        }
    }
}

struct HostWriter<W> {
    output: W,
    id: Value,
    terminal_written: bool,
}

impl<W: Write> HostWriter<W> {
    fn new(output: W, id: Value) -> Self {
        Self {
            output,
            id,
            terminal_written: false,
        }
    }

    fn event(&mut self, kind: &str, mut payload: Map<String, Value>) -> Result<(), AdapterError> {
        if kind.is_empty() {
            return Err(AdapterError::new(
                "internal_error",
                "event type must be non-empty",
            ));
        }
        if self.terminal_written {
            return Err(AdapterError::new(
                "internal_error",
                "cannot emit an event after the terminal result",
            ));
        }
        payload.insert("type".to_owned(), Value::String(kind.to_owned()));
        self.frame(&json!({"type":"event", "id":self.id, "event":payload}))
    }

    fn success(&mut self, value: Value) -> Result<(), AdapterError> {
        self.terminal(json!({"type":"result", "id":self.id, "ok":true, "value":value}))
    }

    fn failure(&mut self, error: &AdapterError) -> Result<(), AdapterError> {
        self.terminal(json!({
            "type":"result",
            "id":self.id,
            "ok":false,
            "error":error.failure()
        }))
    }

    fn terminal(&mut self, value: Value) -> Result<(), AdapterError> {
        if self.terminal_written {
            return Err(AdapterError::new(
                "internal_error",
                "terminal result was already emitted",
            ));
        }
        self.frame(&value)?;
        self.terminal_written = true;
        Ok(())
    }

    fn frame(&mut self, value: &Value) -> Result<(), AdapterError> {
        serde_json::to_writer(&mut self.output, value)
            .and_then(|()| self.output.write_all(b"\n").map_err(serde_json::Error::io))
            .and_then(|()| self.output.flush().map_err(serde_json::Error::io))
            .map_err(|error| {
                AdapterError::new(
                    "protocol_write_failed",
                    format!("cannot write frame: {error}"),
                )
            })
    }
}

fn run_host<R, W, E, H>(mut input: R, output: W, mut diagnostics: E, handler: H) -> i32
where
    R: Read,
    W: Write,
    E: Write,
    H: FnOnce(&PluginRequest, &mut HostWriter<W>, &mut E) -> Result<Value, AdapterError>,
{
    let mut bytes = Vec::new();
    let mut bounded_input = input.by_ref().take((MAX_REQUEST_BYTES + 1) as u64);
    if let Err(error) = bounded_input.read_to_end(&mut bytes) {
        let mut writer = HostWriter::new(output, Value::Null);
        let failure = AdapterError::new(
            "invalid_json",
            format!("cannot read the single plugin request: {error}"),
        );
        let _ = writeln!(
            diagnostics,
            "[error] Plugin protocol error: {}",
            failure.message
        );
        return finish_failure(&mut writer, &failure, 2);
    }
    if bytes.len() > MAX_REQUEST_BYTES {
        let mut writer = HostWriter::new(output, Value::Null);
        let failure = AdapterError::new(
            "request_too_large",
            format!("plugin request exceeds {MAX_REQUEST_BYTES} bytes"),
        )
        .with_details(json!({"max_bytes":MAX_REQUEST_BYTES}));
        let _ = writeln!(
            diagnostics,
            "[error] Plugin protocol error: {}",
            failure.message
        );
        return finish_failure(&mut writer, &failure, 2);
    }

    let request = match decode_request(&bytes) {
        Ok(request) => request,
        Err(error) => {
            let mut writer = HostWriter::new(output, Value::Null);
            let failure = AdapterError::new(
                "invalid_json",
                "stdin must contain exactly one strict JSON plugin request",
            )
            .with_details(json!({"reason": error.to_string()}));
            let _ = writeln!(diagnostics, "[error] Plugin protocol error: {error}");
            return finish_failure(&mut writer, &failure, 2);
        }
    };
    let id = request_id_value(&request.id);
    let mut writer = HostWriter::new(output, id);
    match handler(&request, &mut writer, &mut diagnostics) {
        Ok(value) => match writer.success(value) {
            Ok(()) => 0,
            Err(error) => {
                let _ = writeln!(
                    diagnostics,
                    "[error] Plugin failed with {}: {}",
                    error.code, error.message
                );
                1
            }
        },
        Err(error) => {
            let _ = writeln!(
                diagnostics,
                "[error] Plugin failed with {}: {}",
                error.code, error.message
            );
            finish_failure(&mut writer, &error, 1)
        }
    }
}

fn run_unknown_host<R, W, E>(input: R, output: W, diagnostics: E, error: AdapterError) -> i32
where
    R: Read,
    W: Write,
    E: Write,
{
    run_host(
        input,
        output,
        diagnostics,
        |_request, _writer, _diagnostics| Err(error),
    )
}

fn finish_failure<W: Write>(writer: &mut HostWriter<W>, error: &AdapterError, code: i32) -> i32 {
    if writer.failure(error).is_ok() {
        code
    } else {
        1
    }
}

fn request_id_value(id: &RequestId) -> Value {
    serde_json::to_value(id).unwrap_or(Value::Null)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Provider {
    Codex,
    ClaudeCode,
    OpenCode,
}

impl Provider {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "codex" => Some(Self::Codex),
            "claude-code" | "claude" => Some(Self::ClaudeCode),
            "opencode" => Some(Self::OpenCode),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::OpenCode => "opencode",
        }
    }

    fn executable(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
            Self::OpenCode => "opencode",
        }
    }

    fn full_bypass(self) -> bool {
        !matches!(self, Self::OpenCode)
    }

    fn warning(self) -> Option<&'static str> {
        match self {
            Self::OpenCode => Some(OPENCODE_WARNING),
            Self::Codex | Self::ClaudeCode => None,
        }
    }
}

#[derive(Debug)]
struct InvocationOptions {
    command_prefix: Vec<String>,
    executable: Option<String>,
    timeout: Option<Duration>,
    native_limits: NativeProviderLimits,
    extra_args: Vec<String>,
    extra_env: BTreeMap<String, String>,
    open: Map<String, Value>,
}

#[derive(Clone, Copy, Debug)]
struct NativeProviderLimits {
    max_line_bytes: usize,
    max_stdout_bytes: usize,
    max_result_bytes: usize,
    max_stderr_bytes: usize,
    output_queue_bound: usize,
}

impl Default for NativeProviderLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: MAX_NATIVE_LINE_BYTES,
            max_stdout_bytes: MAX_NATIVE_STDOUT_BYTES,
            max_result_bytes: MAX_PROVIDER_RESULT_BYTES,
            max_stderr_bytes: MAX_NATIVE_STDERR_BYTES,
            output_queue_bound: NATIVE_OUTPUT_QUEUE,
        }
    }
}

fn handle_provider<W: Write, E: Write>(
    provider: Provider,
    request: &PluginRequest,
    writer: &mut HostWriter<W>,
    diagnostics: &mut E,
) -> Result<Value, AdapterError> {
    match request.method.as_str() {
        "describe" => Ok(provider_description(provider)),
        "smoke" => provider_smoke(provider, &request.params, writer, diagnostics),
        "invoke" => provider_invoke(provider, &request.params, writer, diagnostics),
        method => Err(AdapterError::new(
            "method_not_found",
            format!(
                "provider {:?} does not implement {method:?}",
                provider.name()
            ),
        )
        .with_details(json!({"methods":["describe", "smoke", "invoke"]}))),
    }
}

fn provider_description(provider: Provider) -> Value {
    let mut value = json!({
        "api": PLUGIN_API,
        "kind": "provider",
        "name": provider.name(),
        "implementation_version": IMPLEMENTATION_VERSION,
        "aliases": if provider == Provider::ClaudeCode { json!(["claude"]) } else { json!([]) },
        "executable": provider.executable(),
        "methods": ["describe", "smoke", "invoke"],
        "operations": ["describe", "smoke", "invoke"],
        "full_bypass": provider.full_bypass(),
        "reasoning_parameter": if provider == Provider::OpenCode { "variant" } else { "effort" },
        "options_schema": {
            "type":"object",
            "additionalProperties":true,
            "properties": {
                "command_prefix":{"type":"array", "items":{"type":"string"}},
                "executable":{"type":"string", "minLength":1},
                "timeout_seconds":{"type":"number", "exclusiveMinimum":0},
                "native_max_line_bytes":{"type":"integer", "minimum":1, "maximum":33554432},
                "native_max_stdout_bytes":{"type":"integer", "minimum":1, "maximum":4294967296_u64},
                "native_max_result_bytes":{"type":"integer", "minimum":1, "maximum":5242880},
                "native_max_stderr_bytes":{"type":"integer", "minimum":1, "maximum":16777216},
                "native_output_queue_bound":{"type":"integer", "minimum":1, "maximum":128},
                "extra_args":{"type":"array", "items":{"type":"string"}},
                "extra_env":{"type":"object", "additionalProperties":{"type":"string"}},
                "auth_status":{"type":"boolean"}
            }
        }
    });
    if let Some(warning) = provider.warning() {
        value["warning"] = Value::String(warning.to_owned());
    }
    value
}

fn provider_smoke<W: Write, E: Write>(
    provider: Provider,
    params: &Map<String, Value>,
    writer: &mut HostWriter<W>,
    diagnostics: &mut E,
) -> Result<Value, AdapterError> {
    let options = invocation_options(params, Some(DEFAULT_SMOKE_TIMEOUT))?;
    let workspace = optional_workspace(params)?;
    let executable = resolved_executable(provider, &options, workspace.as_deref())?;
    let environment = provider_environment(provider, &options.extra_env)?;
    let version_argv = command_with_prefix(
        &options.command_prefix,
        &executable,
        &["--version".to_owned()],
    );
    let version = run_health_command(
        provider,
        &version_argv,
        workspace.as_deref(),
        &environment,
        options.timeout,
        diagnostics,
    )?;
    let auth_status = if bool_option(params, &options.open, "auth_status", false)? {
        let arguments = auth_arguments(provider);
        let argv = command_with_prefix(&options.command_prefix, &executable, &arguments);
        Some(run_health_command(
            provider,
            &argv,
            workspace.as_deref(),
            &environment,
            options.timeout,
            diagnostics,
        )?)
    } else {
        None
    };

    let live = bool_option(params, &options.open, "live", false)?;
    if !live {
        let mut value = json!({
            "provider": provider.name(),
            "text": version.trim(),
            "version": version.trim(),
            "live": false,
            "full_bypass": provider.full_bypass()
        });
        if let Some(status) = auth_status {
            value["auth_status"] = Value::String(status.trim().to_owned());
        }
        if let Some(warning) = provider.warning() {
            value["warning"] = Value::String(warning.to_owned());
        }
        return Ok(value);
    }

    let mut live_params = params.clone();
    live_params.insert(
        "prompt".to_owned(),
        Value::String("Reply exactly TACTUS_OK. Do not use tools.".to_owned()),
    );
    if !live_params.contains_key("workspace") {
        live_params.insert(
            "workspace".to_owned(),
            Value::String(
                env::current_dir()
                    .map_err(|error| {
                        AdapterError::new(
                            "invalid_params",
                            format!("cannot resolve current workspace: {error}"),
                        )
                    })?
                    .to_string_lossy()
                    .into_owned(),
            ),
        );
    }
    if provider == Provider::ClaudeCode {
        let mut arguments = options.extra_args;
        arguments.extend(["--tools".to_owned(), String::new()]);
        live_params.insert(
            "extra_args".to_owned(),
            Value::Array(arguments.into_iter().map(Value::String).collect()),
        );
    }
    let mut result = provider_invoke(provider, &live_params, writer, diagnostics)?;
    let text = result
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !text.contains("TACTUS_OK") {
        return Err(AdapterError::new(
            "smoke_mismatch",
            format!("{} live smoke did not return TACTUS_OK", provider.name()),
        )
        .with_details(json!({"provider":provider.name(), "text":text})));
    }
    result["version"] = Value::String(version.trim().to_owned());
    result["live"] = Value::Bool(true);
    if let Some(status) = auth_status {
        result["auth_status"] = Value::String(status.trim().to_owned());
    }
    Ok(result)
}

fn provider_invoke<W: Write, E: Write>(
    provider: Provider,
    params: &Map<String, Value>,
    writer: &mut HostWriter<W>,
    diagnostics: &mut E,
) -> Result<Value, AdapterError> {
    let prompt = required_string(params, "prompt")?;
    let workspace = required_workspace(params)?;
    let options = invocation_options(params, None)?;
    let model = optional_string(params, "model")?.or(optional_string(&options.open, "model")?);
    let mut reasoning =
        optional_string(params, "effort")?.or(optional_string(&options.open, "effort")?);
    if provider == Provider::OpenCode {
        reasoning = optional_string(params, "variant")?
            .or(optional_string(&options.open, "variant")?)
            .or(reasoning);
    }
    let environment = provider_environment(provider, &options.extra_env)?;
    let executable = resolved_executable(provider, &options, Some(&workspace))?;
    let temporary = if provider == Provider::Codex {
        Some(TemporaryDirectory::create("tactus-codex")?)
    } else {
        None
    };
    let last_message = temporary
        .as_ref()
        .map(|directory| directory.path.join("last-message.txt"));
    let native_arguments = provider_arguments(
        provider,
        &workspace,
        model.as_deref(),
        reasoning.as_deref(),
        &options.extra_args,
        last_message.as_deref(),
    );
    let argv = command_with_prefix(&options.command_prefix, &executable, &native_arguments);
    let completed = run_provider_command(
        ProviderRun {
            provider,
            argv: &argv,
            workspace: &workspace,
            environment: &environment,
            prompt: &prompt,
            timeout: options.timeout,
            limits: options.native_limits,
        },
        writer,
        diagnostics,
    )?;
    let reported_failure = completed.reported_failure;
    let (text, result_recognized) = match last_message {
        Some(path) => match read_codex_last_message(&path, options.native_limits.max_result_bytes)?
        {
            Some(text) => (text, true),
            None => (completed.text, completed.result_recognized),
        },
        None => (completed.text, completed.result_recognized),
    };
    if reported_failure {
        return Err(AdapterError::new(
            "provider_reported_failure",
            format!("{} reported a terminal native failure", provider.name()),
        )
        .with_details(json!({
            "provider":provider.name(),
            "cause":"native_reported_failure",
        })));
    }
    if !completed.status.success() {
        let mut details = json!({
            "provider":provider.name(),
            "cause":"provider_exit",
            "exit_code":completed.status.code(),
            "text":text,
            "full_bypass":provider.full_bypass()
        });
        if let Some(warning) = provider.warning() {
            details["warning"] = Value::String(warning.to_owned());
        }
        return Err(AdapterError::new(
            "outcome_unknown",
            format!(
                "{} exited without proving whether the external request completed",
                provider.name()
            ),
        )
        .with_details(details));
    }
    if !result_recognized {
        return Err(AdapterError::new(
            "outcome_unknown",
            format!(
                "{} completed without a recognized native result record",
                provider.name()
            ),
        )
        .with_details(json!({
            "provider":provider.name(),
            "cause":"native_result_unrecognized",
        })));
    }

    let mut value = json!({
        "provider":provider.name(),
        "text":text,
        "exit_code":0,
        "full_bypass":provider.full_bypass()
    });
    if let Some(warning) = provider.warning() {
        value["warning"] = Value::String(warning.to_owned());
    }
    Ok(value)
}

fn provider_arguments(
    provider: Provider,
    workspace: &Path,
    model: Option<&str>,
    reasoning: Option<&str>,
    extra_args: &[String],
    last_message: Option<&Path>,
) -> Vec<String> {
    let mut arguments = match provider {
        Provider::Codex => vec![
            "exec".to_owned(),
            "--dangerously-bypass-approvals-and-sandbox".to_owned(),
            "--json".to_owned(),
            "-C".to_owned(),
            workspace.to_string_lossy().into_owned(),
            "--skip-git-repo-check".to_owned(),
            "--ephemeral".to_owned(),
        ],
        Provider::ClaudeCode => vec![
            "-p".to_owned(),
            "--dangerously-skip-permissions".to_owned(),
            "--output-format".to_owned(),
            "stream-json".to_owned(),
            "--verbose".to_owned(),
            "--no-session-persistence".to_owned(),
        ],
        Provider::OpenCode => vec![
            "run".to_owned(),
            "--auto".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
            "--dir".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ],
    };
    if let Some(model) = model {
        arguments.extend(["--model".to_owned(), model.to_owned()]);
    }
    if let Some(reasoning) = reasoning {
        match provider {
            Provider::Codex => arguments.extend([
                "-c".to_owned(),
                format!(
                    "model_reasoning_effort={}",
                    serde_json::to_string(reasoning).expect("strings always encode")
                ),
            ]),
            Provider::ClaudeCode => {
                arguments.extend(["--effort".to_owned(), reasoning.to_owned()]);
            }
            Provider::OpenCode => {
                arguments.extend(["--variant".to_owned(), reasoning.to_owned()]);
            }
        }
    }
    if let Some(path) = last_message {
        arguments.extend(["-o".to_owned(), path.to_string_lossy().into_owned()]);
    }
    arguments.extend_from_slice(extra_args);
    if provider == Provider::Codex {
        arguments.push("-".to_owned());
    }
    arguments
}

fn auth_arguments(provider: Provider) -> Vec<String> {
    match provider {
        Provider::Codex => vec!["login".to_owned(), "status".to_owned()],
        Provider::ClaudeCode => vec!["auth".to_owned(), "status".to_owned(), "--json".to_owned()],
        Provider::OpenCode => vec!["auth".to_owned(), "list".to_owned()],
    }
}

fn command_with_prefix(prefix: &[String], executable: &str, arguments: &[String]) -> Vec<String> {
    let mut argv = prefix.to_vec();
    argv.push(executable.to_owned());
    argv.extend_from_slice(arguments);
    argv
}

fn invocation_options(
    params: &Map<String, Value>,
    default_timeout: Option<Duration>,
) -> Result<InvocationOptions, AdapterError> {
    let open = match params.get("options") {
        None => Map::new(),
        Some(Value::Object(options)) => options.clone(),
        Some(_) => {
            return Err(AdapterError::new(
                "invalid_params",
                "options must be a JSON object",
            ));
        }
    };
    let command_prefix = string_array(open.get("command_prefix"), "options.command_prefix", false)?;
    let executable = optional_string(&open, "executable")?;
    let timeout = match open.get("timeout_seconds") {
        None => default_timeout,
        Some(Value::Number(number)) => {
            let seconds = number.as_f64().ok_or_else(|| {
                AdapterError::new(
                    "invalid_params",
                    "options.timeout_seconds must be a finite number",
                )
            })?;
            if !seconds.is_finite() || seconds <= 0.0 {
                return Err(AdapterError::new(
                    "invalid_params",
                    "options.timeout_seconds must be positive",
                ));
            }
            Some(Duration::try_from_secs_f64(seconds).map_err(|_| {
                AdapterError::new(
                    "invalid_params",
                    "options.timeout_seconds is outside the supported duration range",
                )
            })?)
        }
        Some(_) => {
            return Err(AdapterError::new(
                "invalid_params",
                "options.timeout_seconds must be a number",
            ));
        }
    };
    let extra_args = string_array(
        params.get("extra_args").or_else(|| open.get("extra_args")),
        "extra_args",
        true,
    )?;
    let extra_env_value = params.get("extra_env").or_else(|| open.get("extra_env"));
    let mut extra_env = BTreeMap::new();
    match extra_env_value {
        None => {}
        Some(Value::Object(entries)) => {
            for (name, value) in entries {
                let value = value.as_str().ok_or_else(|| {
                    AdapterError::new("invalid_params", "extra_env must map strings to strings")
                })?;
                extra_env.insert(name.clone(), value.to_owned());
            }
        }
        Some(_) => {
            return Err(AdapterError::new(
                "invalid_params",
                "extra_env must map strings to strings",
            ));
        }
    }
    let native_limits = native_provider_limits(&open)?;
    Ok(InvocationOptions {
        command_prefix,
        executable,
        timeout,
        native_limits,
        extra_args,
        extra_env,
        open,
    })
}

fn native_provider_limits(open: &Map<String, Value>) -> Result<NativeProviderLimits, AdapterError> {
    let defaults = NativeProviderLimits::default();
    let limits = NativeProviderLimits {
        max_line_bytes: bounded_usize_option(
            open,
            "native_max_line_bytes",
            defaults.max_line_bytes,
            32 * 1024 * 1024,
        )?,
        max_stdout_bytes: bounded_usize_option(
            open,
            "native_max_stdout_bytes",
            defaults.max_stdout_bytes,
            4usize * 1024 * 1024 * 1024,
        )?,
        max_result_bytes: bounded_usize_option(
            open,
            "native_max_result_bytes",
            defaults.max_result_bytes,
            5 * 1024 * 1024,
        )?,
        max_stderr_bytes: bounded_usize_option(
            open,
            "native_max_stderr_bytes",
            defaults.max_stderr_bytes,
            16 * 1024 * 1024,
        )?,
        output_queue_bound: bounded_usize_option(
            open,
            "native_output_queue_bound",
            defaults.output_queue_bound,
            128,
        )?,
    };
    if limits.max_stdout_bytes < limits.max_line_bytes {
        return Err(AdapterError::new(
            "invalid_params",
            "options.native_max_stdout_bytes must not be smaller than native_max_line_bytes",
        ));
    }
    let resident = limits
        .max_line_bytes
        .checked_mul(limits.output_queue_bound)
        .ok_or_else(|| AdapterError::new("invalid_params", "native output queue size overflows"))?;
    if resident > 256 * 1024 * 1024 {
        return Err(AdapterError::new(
            "invalid_params",
            "native line size multiplied by queue bound must not exceed 268435456 bytes",
        ));
    }
    Ok(limits)
}

fn bounded_usize_option(
    open: &Map<String, Value>,
    name: &str,
    default: usize,
    maximum: usize,
) -> Result<usize, AdapterError> {
    let Some(value) = open.get(name) else {
        return Ok(default);
    };
    let Some(number) = value.as_u64() else {
        return Err(AdapterError::new(
            "invalid_params",
            format!("options.{name} must be a positive integer"),
        ));
    };
    let converted = usize::try_from(number).map_err(|_| {
        AdapterError::new(
            "invalid_params",
            format!("options.{name} is outside the supported integer range"),
        )
    })?;
    if converted == 0 || converted > maximum {
        return Err(AdapterError::new(
            "invalid_params",
            format!("options.{name} must be between 1 and {maximum}"),
        ));
    }
    Ok(converted)
}

fn string_array(
    value: Option<&Value>,
    name: &str,
    allow_empty: bool,
) -> Result<Vec<String>, AdapterError> {
    let expected = if allow_empty {
        "an array of strings"
    } else {
        "an array of non-empty strings"
    };
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|item| allow_empty || !item.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        AdapterError::new("invalid_params", format!("{name} must be {expected}"))
                    })
            })
            .collect(),
        Some(_) => Err(AdapterError::new(
            "invalid_params",
            format!("{name} must be {expected}"),
        )),
    }
}

fn bool_option(
    params: &Map<String, Value>,
    options: &Map<String, Value>,
    name: &str,
    default: bool,
) -> Result<bool, AdapterError> {
    match params.get(name).or_else(|| options.get(name)) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(AdapterError::new(
            "invalid_params",
            format!("{name} must be a boolean"),
        )),
    }
}

fn provider_environment(
    provider: Provider,
    extra: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, AdapterError> {
    let mut result = extra.clone();
    if provider != Provider::OpenCode {
        return Ok(result);
    }
    let existing = extra
        .get("OPENCODE_CONFIG_CONTENT")
        .cloned()
        .or_else(|| env::var("OPENCODE_CONFIG_CONTENT").ok());
    let mut inline = match existing {
        Some(value) => serde_json::from_str::<Value>(&value)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default(),
        None => Map::new(),
    };
    inline.insert("permission".to_owned(), Value::String("allow".to_owned()));
    result.insert(
        "OPENCODE_CONFIG_CONTENT".to_owned(),
        serde_json::to_string(&inline).map_err(|error| {
            AdapterError::new(
                "internal_error",
                format!("cannot encode OpenCode permission override: {error}"),
            )
        })?,
    );
    Ok(result)
}

fn resolved_executable(
    provider: Provider,
    options: &InvocationOptions,
    workspace: Option<&Path>,
) -> Result<String, AdapterError> {
    let configured = options
        .executable
        .as_deref()
        .unwrap_or_else(|| provider.executable());
    if !options.command_prefix.is_empty() {
        return Ok(configured.to_owned());
    }
    let working_directory = match workspace {
        Some(path) => path.to_path_buf(),
        None => env::current_dir().map_err(|error| {
            AdapterError::new(
                "provider_resolution_failed",
                format!("cannot determine the provider working directory: {error}"),
            )
        })?,
    };
    let resolver = ExecutableResolver::environment(working_directory);
    let resolved = resolver
        .resolve(configured)
        .map_err(|error| executable_resolution_error(provider, configured, &error))?;
    resolved.into_os_string().into_string().map_err(|path| {
        AdapterError::new(
            "provider_executable_not_utf8",
            format!(
                "{} native executable path is not valid UTF-8",
                provider.name()
            ),
        )
        .with_details(json!({
            "provider":provider.name(),
            "executable":path.to_string_lossy()
        }))
    })
}

fn executable_resolution_error(
    provider: Provider,
    configured: &str,
    error: &ExecutableResolutionError,
) -> AdapterError {
    AdapterError::new(
        error.code(),
        format!(
            "cannot resolve {} native executable: {error}",
            provider.name()
        ),
    )
    .with_details(json!({
        "provider":provider.name(),
        "executable":configured,
        "candidates":error
            .candidates()
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
    }))
}

struct ProviderCompletion {
    status: ExitStatus,
    text: String,
    result_recognized: bool,
    reported_failure: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProviderResultKind {
    Plain,
    Fragments,
    Final,
}

#[derive(Debug, Eq, PartialEq)]
struct ProviderResultOverflow {
    result_bytes_at_least: usize,
    max_result_bytes: usize,
}

/// Retains only the text that can become the provider's terminal result.
///
/// A native stream may be much larger than the plugin result budget. Once the
/// selected candidate crosses that budget, its storage is released and the
/// overflow is remembered while the caller continues draining native output.
struct ProviderResultAccumulator {
    kind: ProviderResultKind,
    text: String,
    items: usize,
    observed_bytes: usize,
    max_result_bytes: usize,
    overflow: Option<ProviderResultOverflow>,
    reported_failure: bool,
}

impl ProviderResultAccumulator {
    fn new(max_result_bytes: usize) -> Self {
        Self {
            kind: ProviderResultKind::Plain,
            text: String::new(),
            items: 0,
            observed_bytes: 0,
            max_result_bytes,
            overflow: None,
            reported_failure: false,
        }
    }

    #[cfg(test)]
    fn observe_plain(&mut self, text: &str) {
        if self.overflow.is_some() || self.kind != ProviderResultKind::Plain {
            return;
        }
        let separator = if self.items == 0 { "" } else { "\n" };
        self.append(text, separator);
    }

    fn observe_fragments(&mut self, fragments: &[&str]) {
        if fragments.is_empty() {
            return;
        }
        if self.kind == ProviderResultKind::Plain {
            self.reset(ProviderResultKind::Fragments);
        }
        if self.kind != ProviderResultKind::Fragments || self.overflow.is_some() {
            return;
        }
        for fragment in fragments {
            self.append(fragment, "");
            if self.overflow.is_some() {
                break;
            }
        }
    }

    fn observe_final(&mut self, text: &str) {
        // A final event is authoritative, even when an earlier fallback or
        // fragment candidate exceeded the retention budget. Providers may
        // also emit more than one final event; matching the previous parser,
        // the last one wins.
        self.reset(ProviderResultKind::Final);
        self.append(text, "");
    }

    fn observe_reported_failure(&mut self) {
        self.reported_failure = true;
    }

    fn reset(&mut self, kind: ProviderResultKind) {
        self.kind = kind;
        self.text.clear();
        self.items = 0;
        self.observed_bytes = 0;
        self.overflow = None;
    }

    fn append(&mut self, text: &str, separator: &str) {
        let separator = if self.items == 0 { "" } else { separator };
        self.items = self.items.saturating_add(1);
        let next_size = self
            .observed_bytes
            .checked_add(separator.len())
            .and_then(|size| size.checked_add(text.len()))
            .unwrap_or(usize::MAX);
        self.observed_bytes = next_size;
        if next_size > self.max_result_bytes {
            self.text.clear();
            self.overflow = Some(ProviderResultOverflow {
                result_bytes_at_least: next_size,
                max_result_bytes: self.max_result_bytes,
            });
            return;
        }
        self.text.push_str(separator);
        self.text.push_str(text);
    }

    fn finish(self) -> Result<String, ProviderResultOverflow> {
        match self.overflow {
            Some(overflow) => Err(overflow),
            None => Ok(self.text),
        }
    }

    fn has_authoritative_result(&self, provider: Provider) -> bool {
        self.items > 0
            && match provider {
                Provider::Codex | Provider::ClaudeCode => self.kind == ProviderResultKind::Final,
                // OpenCode's JSON stream currently exposes text records rather
                // than a separate terminal result record; successful process
                // exit completes the accumulated text sequence.
                Provider::OpenCode => {
                    matches!(
                        self.kind,
                        ProviderResultKind::Fragments | ProviderResultKind::Final
                    )
                }
            }
    }

    fn reported_failure(&self) -> bool {
        self.reported_failure
    }
}

fn finish_provider_result(
    provider: Provider,
    result: ProviderResultAccumulator,
) -> Result<String, AdapterError> {
    result.finish().map_err(|overflow| {
        AdapterError::new(
            "outcome_unknown",
            format!(
                "{} completed but its result exceeded the host transport budget",
                provider.name()
            ),
        )
        .with_details(json!({
            "provider":provider.name(),
            "cause":"provider_result_limit",
            "result_bytes_at_least":overflow.result_bytes_at_least,
            "max_result_bytes":overflow.max_result_bytes
        }))
    })
}

fn finalize_provider_result(
    provider: Provider,
    result: ProviderResultAccumulator,
) -> Result<(String, bool, bool), AdapterError> {
    let reported_failure = result.reported_failure();
    if reported_failure {
        // A provider-owned terminal failure is authoritative by itself. Any
        // earlier assistant text is non-terminal and must not turn that known
        // failure into an ambiguous result-size error.
        return Ok((String::new(), true, true));
    }
    let result_recognized = result.has_authoritative_result(provider);
    let text = finish_provider_result(provider, result)?;
    Ok((text, result_recognized, false))
}

#[derive(Default)]
struct ProviderEventDiagnostics {
    native_events: u64,
    json_events: u64,
    plain_lines: u64,
    thinking_events_suppressed: u64,
    raw_bytes: u64,
    stderr_bytes: u64,
    stderr_lines: u64,
    stderr_truncated: bool,
    stderr_sha256: Option<String>,
    event_type_fingerprints: BTreeMap<String, u64>,
}

impl ProviderEventDiagnostics {
    fn observe_json(&mut self, provider: Provider, raw: &Value, bytes: usize) {
        self.native_events = self.native_events.saturating_add(1);
        self.json_events = self.json_events.saturating_add(1);
        self.raw_bytes = self
            .raw_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
        let event_type = raw
            .as_object()
            .and_then(|event| event.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let subtype = raw
            .as_object()
            .and_then(|event| event.get("subtype"))
            .and_then(Value::as_str);
        if provider == Provider::ClaudeCode
            && (subtype == Some("thinking_tokens")
                || event_type == "thinking"
                || event_type == "thinking_delta")
        {
            self.thinking_events_suppressed = self.thinking_events_suppressed.saturating_add(1);
        }
        let event_type_digest = format!("{:x}", Sha256::digest(event_type.as_bytes()));
        let event_type_fingerprint = format!("sha256:{}", &event_type_digest[..16]);
        if self.event_type_fingerprints.len() < 64
            || self
                .event_type_fingerprints
                .contains_key(&event_type_fingerprint)
        {
            let count = self
                .event_type_fingerprints
                .entry(event_type_fingerprint)
                .or_default();
            *count = count.saturating_add(1);
        }
    }

    fn observe_plain(&mut self, bytes: usize) {
        self.native_events = self.native_events.saturating_add(1);
        self.plain_lines = self.plain_lines.saturating_add(1);
        self.raw_bytes = self
            .raw_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    fn observe_stderr(&mut self, bytes: &[u8], truncated: bool) {
        self.stderr_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.stderr_lines = if bytes.is_empty() {
            0
        } else {
            u64::try_from(bytes.split(|byte| *byte == b'\n').count()).unwrap_or(u64::MAX)
        };
        self.stderr_truncated = truncated;
        if !bytes.is_empty() {
            self.stderr_sha256 = Some(format!("{:x}", Sha256::digest(bytes)));
        }
    }

    fn payload(&self, provider: Provider) -> Map<String, Value> {
        Map::from_iter([
            (
                "provider".to_owned(),
                Value::String(provider.name().to_owned()),
            ),
            ("native_events".to_owned(), json!(self.native_events)),
            ("json_events".to_owned(), json!(self.json_events)),
            ("plain_lines".to_owned(), json!(self.plain_lines)),
            (
                "thinking_events_suppressed".to_owned(),
                json!(self.thinking_events_suppressed),
            ),
            ("raw_bytes".to_owned(), json!(self.raw_bytes)),
            ("stderr_bytes".to_owned(), json!(self.stderr_bytes)),
            ("stderr_lines".to_owned(), json!(self.stderr_lines)),
            ("stderr_truncated".to_owned(), json!(self.stderr_truncated)),
            ("stderr_sha256".to_owned(), json!(self.stderr_sha256)),
            (
                "event_type_fingerprints".to_owned(),
                json!(self.event_type_fingerprints),
            ),
        ])
    }
}

fn observe_provider_output(
    provider: Provider,
    line: &[u8],
    result: &mut ProviderResultAccumulator,
    diagnostics: &mut ProviderEventDiagnostics,
) -> Result<(), AdapterError> {
    let text = std::str::from_utf8(line).map_err(|error| {
        AdapterError::new(
            "outcome_unknown",
            format!(
                "{} may have completed externally before emitting invalid UTF-8 output",
                provider.name()
            ),
        )
        .with_details(json!({
            "provider":provider.name(),
            "cause":"native_output_invalid_utf8",
            "valid_up_to":error.valid_up_to(),
        }))
    })?;
    if text.trim().is_empty() {
        return Ok(());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(raw) => {
            diagnostics.observe_json(provider, &raw, line.len());
            if provider == Provider::ClaudeCode
                && raw.get("type").and_then(Value::as_str) == Some("result")
                && (raw.get("is_error").and_then(Value::as_bool) == Some(true)
                    || raw
                        .get("subtype")
                        .and_then(Value::as_str)
                        .is_some_and(|subtype| subtype.starts_with("error_")))
            {
                result.observe_reported_failure();
            }
            let (event_fragments, event_final) = provider_event_text(provider, &raw);
            result.observe_fragments(&event_fragments);
            if let Some(event_final) = event_final {
                result.observe_final(event_final);
            }
        }
        Err(error) => {
            diagnostics.observe_plain(line.len());
            return Err(AdapterError::new(
                "outcome_unknown",
                format!(
                    "{} may have completed externally before emitting malformed JSON output",
                    provider.name()
                ),
            )
            .with_details(json!({
                "provider":provider.name(),
                "cause":"native_output_malformed_json",
                "line_bytes":line.len(),
                "reason":error.to_string(),
            })));
        }
    }
    Ok(())
}

enum NativeChildInner {
    Group(GroupChild),
    #[cfg(unix)]
    Attached(Child),
}

struct NativeChild {
    inner: NativeChildInner,
    armed: bool,
}

impl NativeChild {
    fn grouped(child: GroupChild) -> Self {
        Self {
            inner: NativeChildInner::Group(child),
            armed: true,
        }
    }

    #[cfg(unix)]
    fn attached(child: Child) -> Self {
        Self {
            inner: NativeChildInner::Attached(child),
            armed: true,
        }
    }

    fn inner(&mut self) -> &mut Child {
        match &mut self.inner {
            NativeChildInner::Group(child) => child.inner(),
            #[cfg(unix)]
            NativeChildInner::Attached(child) => child,
        }
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.inner {
            NativeChildInner::Group(child) => child.try_wait(),
            #[cfg(unix)]
            NativeChildInner::Attached(child) => child.try_wait(),
        }
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        match &mut self.inner {
            NativeChildInner::Group(child) => child.wait(),
            #[cfg(unix)]
            NativeChildInner::Attached(child) => child.wait(),
        }
    }

    fn kill(&mut self) -> io::Result<()> {
        match &mut self.inner {
            NativeChildInner::Group(child) => child.kill(),
            #[cfg(unix)]
            NativeChildInner::Attached(child) => child.kill(),
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for NativeChild {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.kill();
            let _ = self.wait();
        }
    }
}

struct ProviderRun<'a> {
    provider: Provider,
    argv: &'a [String],
    workspace: &'a Path,
    environment: &'a BTreeMap<String, String>,
    prompt: &'a str,
    timeout: Option<Duration>,
    limits: NativeProviderLimits,
}

fn dispatched_provider_failure(
    provider: Provider,
    cause: &'static str,
    message: impl Into<String>,
) -> AdapterError {
    AdapterError::new("outcome_unknown", message).with_details(json!({
        "provider":provider.name(),
        "cause":cause,
    }))
}

enum OutputMessage {
    Line(Vec<u8>),
    ReadError(String),
    LimitExceeded(String),
    End,
}

fn run_provider_command<W: Write, E: Write>(
    run: ProviderRun<'_>,
    writer: &mut HostWriter<W>,
    diagnostics: &mut E,
) -> Result<ProviderCompletion, AdapterError> {
    let ProviderRun {
        provider,
        argv,
        workspace,
        environment,
        prompt,
        timeout,
        limits,
    } = run;
    let mut child = spawn_group(argv, Some(workspace), environment, true)?;
    let stdin = child.inner().stdin.take().ok_or_else(|| {
        dispatched_provider_failure(
            provider,
            "native_stdin_unavailable",
            "native provider started but its stdin pipe was unavailable",
        )
    })?;
    let stdout = child.inner().stdout.take().ok_or_else(|| {
        dispatched_provider_failure(
            provider,
            "native_stdout_unavailable",
            "native provider started but its stdout pipe was unavailable",
        )
    })?;
    let stderr = child.inner().stderr.take().ok_or_else(|| {
        dispatched_provider_failure(
            provider,
            "native_stderr_unavailable",
            "native provider started but its stderr pipe was unavailable",
        )
    })?;
    let prompt = prompt.as_bytes().to_vec();
    let input_worker = thread::spawn(move || -> io::Result<()> {
        let mut stdin = stdin;
        stdin.write_all(&prompt)?;
        stdin.flush()
    });
    let (sender, receiver) = mpsc::sync_channel(limits.output_queue_bound);
    let output_worker = thread::spawn(move || {
        read_lf_lines(
            stdout,
            sender,
            limits.max_line_bytes,
            limits.max_stdout_bytes,
        )
    });
    let diagnostics_worker =
        thread::spawn(move || read_bounded_and_drain(stderr, limits.max_stderr_bytes));

    let started = Instant::now();
    let mut status = None;
    let mut output_done = false;
    let mut result = ProviderResultAccumulator::new(limits.max_result_bytes);
    let mut event_diagnostics = ProviderEventDiagnostics::default();
    let mut native_lines_seen = 0_u64;
    let mut next_progress_milestone = 1_u64;
    while status.is_none()
        || !output_done
        || !input_worker.is_finished()
        || !diagnostics_worker.is_finished()
    {
        match receiver.recv_timeout(PROCESS_POLL) {
            Ok(OutputMessage::Line(line)) => {
                native_lines_seen = native_lines_seen.saturating_add(1);
                if native_lines_seen >= next_progress_milestone {
                    writer
                        .event(
                            "provider.progress",
                            Map::from_iter([
                                (
                                    "provider".to_owned(),
                                    Value::String(provider.name().to_owned()),
                                ),
                                (
                                    "phase".to_owned(),
                                    Value::String("partial_output".to_owned()),
                                ),
                                ("native_lines_seen".to_owned(), json!(native_lines_seen)),
                            ]),
                        )
                        .map_err(|error| {
                            dispatched_provider_failure(
                                provider,
                                "host_progress_write_failed",
                                error.message,
                            )
                        })?;
                    next_progress_milestone = next_progress_milestone.saturating_mul(2).max(2);
                }
                if let Err(error) =
                    observe_provider_output(provider, &line, &mut result, &mut event_diagnostics)
                {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            }
            Ok(OutputMessage::ReadError(error)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(dispatched_provider_failure(
                    provider,
                    "native_stdout_read_failed",
                    format!("cannot read provider stdout: {error}"),
                ));
            }
            Ok(OutputMessage::LimitExceeded(message)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AdapterError::new(
                    "outcome_unknown",
                    format!(
                        "{} may have completed externally before its output limit was exceeded",
                        provider.name()
                    ),
                )
                .with_details(json!({
                    "provider":provider.name(),
                    "cause":"native_output_limit",
                    "reason":message,
                    "max_line_bytes":limits.max_line_bytes,
                    "max_stdout_bytes":limits.max_stdout_bytes
                })));
            }
            Ok(OutputMessage::End) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                output_done = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = match child.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(dispatched_provider_failure(
                        provider,
                        "native_wait_failed",
                        format!("cannot wait for {}: {error}", provider.name()),
                    ));
                }
            };
        }
        // The native CLI leader can exit while a descendant still owns one of
        // the inherited pipes. The deadline governs the whole supervised I/O
        // lifetime so joining a reader can never wait forever in that state.
        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
            let _ = child.kill();
            let _ = child.wait();
            drop(receiver);
            // A native descendant may still own the captured pipes. The
            // provider host must be allowed to report its structured failure;
            // the outer Tactus supervisor owns and cleans the shared group.
            drop(output_worker);
            drop(input_worker);
            drop(diagnostics_worker);
            return Err(AdapterError::new(
                "outcome_unknown",
                format!(
                    "{} may have completed externally before its configured timeout",
                    provider.name()
                ),
            )
            .with_details(json!({
                "provider":provider.name(),
                "cause":"timeout",
                "timeout_seconds":timeout.map(|value| value.as_secs_f64())
            })));
        }
    }
    let status = match status {
        Some(status) => status,
        None => child.wait().map_err(|error| {
            dispatched_provider_failure(
                provider,
                "native_wait_failed",
                format!("cannot wait for {}: {error}", provider.name()),
            )
        })?,
    };
    if output_worker.join().is_err() {
        return Err(dispatched_provider_failure(
            provider,
            "native_stdout_reader_failed",
            "native provider stdout reader panicked",
        ));
    }
    match input_worker.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return Err(dispatched_provider_failure(
                provider,
                "native_prompt_write_failed",
                format!("cannot write provider prompt: {error}"),
            ));
        }
        Err(_) => {
            return Err(dispatched_provider_failure(
                provider,
                "native_prompt_writer_failed",
                "native provider prompt writer panicked",
            ));
        }
    }
    let captured = join_capture(diagnostics_worker, "provider stderr").map_err(|error| {
        dispatched_provider_failure(provider, "native_stderr_capture_failed", error.message)
    })?;
    event_diagnostics.observe_stderr(&captured.bytes, captured.truncated);
    forward_diagnostics(provider, &captured.bytes, diagnostics);
    if captured.truncated {
        let _ = writeln!(
            diagnostics,
            "[warning] {} stderr truncated after {} bytes.",
            provider.name(),
            limits.max_stderr_bytes
        );
    }
    // Native provider formats are intentionally not part of plugin-v1. Keep
    // one bounded diagnostic aggregate instead of forwarding token-level raw
    // JSON. This protects both the durable trace and human observers while
    // retaining enough evidence to diagnose event-shape changes.
    writer
        .event("provider.diagnostic", event_diagnostics.payload(provider))
        .map_err(|error| {
            dispatched_provider_failure(provider, "host_diagnostic_write_failed", error.message)
        })?;
    child.disarm();
    let (text, result_recognized, reported_failure) = finalize_provider_result(provider, result)?;
    Ok(ProviderCompletion {
        status,
        text,
        result_recognized,
        reported_failure,
    })
}

fn read_codex_last_message(
    path: &Path,
    max_result_bytes: usize,
) -> Result<Option<String>, AdapterError> {
    let file = match File::open(path) {
        Ok(file) => file,
        // Codex may exit before publishing the optional file. Its normalized
        // stdout remains a valid fallback, matching the pre-bounded behavior.
        Err(_) => return Ok(None),
    };
    let mut bytes = Vec::with_capacity(max_result_bytes + 1);
    if file
        .take((max_result_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return Ok(None);
    }
    if bytes.len() > max_result_bytes {
        return Err(AdapterError::new(
            "outcome_unknown",
            "Codex completed but its last-message file exceeded the host transport budget",
        )
        .with_details(json!({
            "provider":"codex",
            "cause":"provider_result_limit",
            "result_bytes_at_least":bytes.len(),
            "max_result_bytes":max_result_bytes
        })));
    }
    String::from_utf8(bytes).map(Some).map_err(|error| {
        AdapterError::new(
            "outcome_unknown",
            "Codex completed but its last-message file was not UTF-8",
        )
        .with_details(json!({
            "provider":"codex",
            "cause":"provider_result_invalid_utf8",
            "valid_up_to":error.utf8_error().valid_up_to()
        }))
    })
}

fn run_health_command<E: Write>(
    provider: Provider,
    argv: &[String],
    cwd: Option<&Path>,
    environment: &BTreeMap<String, String>,
    timeout: Option<Duration>,
    diagnostics: &mut E,
) -> Result<String, AdapterError> {
    let mut child = spawn_group(argv, cwd, environment, false)?;
    let stdout = child.inner().stdout.take().ok_or_else(|| {
        AdapterError::new("provider_spawn_failed", "health stdout was unavailable")
    })?;
    let stderr = child.inner().stderr.take().ok_or_else(|| {
        AdapterError::new("provider_spawn_failed", "health stderr was unavailable")
    })?;
    let stdout_worker =
        thread::spawn(move || read_bounded_and_drain(stdout, MAX_HEALTH_STDOUT_BYTES));
    let stderr_worker =
        thread::spawn(move || read_bounded_and_drain(stderr, MAX_NATIVE_STDERR_BYTES));
    let started = Instant::now();
    let mut status = None;
    loop {
        if status.is_none() {
            status = child.try_wait().map_err(|error| {
                AdapterError::new(
                    "provider_wait_failed",
                    format!(
                        "cannot wait for {} health command: {error}",
                        provider.name()
                    ),
                )
            })?;
        }
        if status.is_some() && stdout_worker.is_finished() && stderr_worker.is_finished() {
            break;
        }
        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
            let _ = child.kill();
            let _ = child.wait();
            // See `run_provider_command`: inherited pipes are owned by the
            // outer supervised group and are deliberately not joined here.
            drop(stdout_worker);
            drop(stderr_worker);
            return Err(AdapterError::new(
                "provider_timeout",
                format!("{} health command exceeded its timeout", provider.name()),
            )
            .with_details(json!({
                "provider":provider.name(),
                "timeout_seconds":timeout.map(|value| value.as_secs_f64())
            })));
        }
        thread::sleep(PROCESS_POLL);
    }
    let status = status.ok_or_else(|| {
        AdapterError::new(
            "provider_wait_failed",
            format!(
                "{} health command completed without a status",
                provider.name()
            ),
        )
    })?;
    let stdout = join_capture(stdout_worker, "health stdout")?;
    let stderr = join_capture(stderr_worker, "health stderr")?;
    child.disarm();
    forward_diagnostics(provider, &stderr.bytes, diagnostics);
    if stderr.truncated {
        let _ = writeln!(
            diagnostics,
            "[warning] {} health stderr truncated after {} bytes.",
            provider.name(),
            MAX_NATIVE_STDERR_BYTES
        );
    }
    if stdout.truncated {
        return Err(AdapterError::new(
            "provider_health_output_limit",
            format!(
                "{} health stdout exceeded {} bytes",
                provider.name(),
                MAX_HEALTH_STDOUT_BYTES
            ),
        )
        .with_details(json!({
            "provider":provider.name(),
            "max_stdout_bytes":MAX_HEALTH_STDOUT_BYTES
        })));
    }
    if !status.success() {
        return Err(AdapterError::new(
            "provider_health_failed",
            format!(
                "{} health command exited with {:?}",
                provider.name(),
                status.code()
            ),
        )
        .with_details(json!({"provider":provider.name(), "exit_code":status.code()})));
    }
    Ok(String::from_utf8_lossy(&stdout.bytes).into_owned())
}

fn spawn_group(
    argv: &[String],
    cwd: Option<&Path>,
    environment: &BTreeMap<String, String>,
    prompt_input: bool,
) -> Result<NativeChild, AdapterError> {
    let executable = argv
        .first()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AdapterError::new("provider_spawn_failed", "provider command is empty"))?;
    let mut command = Command::new(executable);
    command
        .args(&argv[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(environment);
    if prompt_input {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    #[cfg(unix)]
    if env::var_os(SUPERVISED_PROCESS_GROUP_ENV).is_some() {
        use std::os::unix::process::CommandExt as _;

        let pid = i32::try_from(std::process::id()).map_err(|_| {
            AdapterError::new("provider_spawn_failed", "provider host PID exceeds i32")
        })?;
        let pgid = nix::unistd::getpgrp().as_raw();
        if pgid != pid {
            return Err(AdapterError::new(
                "provider_spawn_failed",
                "supervised provider host is not its process-group leader",
            ));
        }
        command.process_group(pgid);
        return command.spawn().map(NativeChild::attached).map_err(|error| {
            AdapterError::new(
                "provider_not_found",
                format!("could not start {executable:?}: {error}"),
            )
            .with_details(json!({"executable":executable}))
        });
    }
    let mut group = command.group();
    #[cfg(windows)]
    group.kill_on_drop(true);
    group.spawn().map(NativeChild::grouped).map_err(|error| {
        AdapterError::new(
            "provider_not_found",
            format!("could not start {executable:?}: {error}"),
        )
        .with_details(json!({"executable":executable}))
    })
}

fn read_lf_lines<R: Read>(
    mut reader: R,
    sender: mpsc::SyncSender<OutputMessage>,
    max_line_bytes: usize,
    max_total_bytes: usize,
) {
    let mut chunk = [0_u8; 8192];
    let mut line = Vec::new();
    let mut total = 0_usize;
    loop {
        let count = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) => {
                let _ = sender.send(OutputMessage::ReadError(error.to_string()));
                return;
            }
        };
        total = total.saturating_add(count);
        if total > max_total_bytes {
            let _ = sender.send(OutputMessage::LimitExceeded(format!(
                "native stdout exceeded {max_total_bytes} bytes"
            )));
            return;
        }
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if sender
                    .send(OutputMessage::Line(std::mem::take(&mut line)))
                    .is_err()
                {
                    return;
                }
            } else if line.len() >= max_line_bytes {
                let _ = sender.send(OutputMessage::LimitExceeded(format!(
                    "native stdout line exceeded {max_line_bytes} bytes"
                )));
                return;
            } else {
                line.push(*byte);
            }
        }
    }
    if !line.is_empty() && sender.send(OutputMessage::Line(line)).is_err() {
        return;
    }
    let _ = sender.send(OutputMessage::End);
}

struct BoundedCapture {
    bytes: Vec<u8>,
    truncated: bool,
}

fn read_bounded_and_drain<R: Read>(mut reader: R, limit: usize) -> io::Result<BoundedCapture> {
    let mut capture = BoundedCapture {
        bytes: Vec::new(),
        truncated: false,
    };
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let available = limit.saturating_sub(capture.bytes.len());
        let retained = available.min(count);
        capture.bytes.extend_from_slice(&chunk[..retained]);
        capture.truncated |= retained < count;
    }
    Ok(capture)
}

fn join_capture(
    worker: thread::JoinHandle<io::Result<BoundedCapture>>,
    name: &str,
) -> Result<BoundedCapture, AdapterError> {
    worker
        .join()
        .map_err(|_| AdapterError::new("provider_io_failed", format!("{name} reader panicked")))?
        .map_err(|error| {
            AdapterError::new("provider_io_failed", format!("cannot read {name}: {error}"))
        })
}

fn forward_diagnostics<E: Write>(provider: Provider, bytes: &[u8], diagnostics: &mut E) {
    if !bytes.is_empty() {
        let lines = bytes.split(|byte| *byte == b'\n').count();
        let digest = format!("{:x}", Sha256::digest(bytes));
        let _ = writeln!(
            diagnostics,
            "[warning] {} produced {} bytes across {} native diagnostic lines; raw content was withheld (sha256 {}).",
            provider.name(),
            bytes.len(),
            lines,
            digest
        );
    }
    let _ = diagnostics.flush();
}

fn provider_event_text(provider: Provider, raw: &Value) -> (Vec<&str>, Option<&str>) {
    let Some(event) = raw.as_object() else {
        return (Vec::new(), None);
    };
    let event_type = event.get("type").and_then(Value::as_str);
    match provider {
        Provider::Codex if event_type == Some("item.completed") => {
            let text = event
                .get("item")
                .and_then(Value::as_object)
                .filter(|item| item.get("type").and_then(Value::as_str) == Some("agent_message"))
                .and_then(|item| item.get("text"))
                .and_then(Value::as_str);
            (Vec::new(), text)
        }
        Provider::ClaudeCode if event_type == Some("result") => {
            let text = event.get("result").and_then(Value::as_str);
            (Vec::new(), text)
        }
        Provider::ClaudeCode if event_type == Some("assistant") => {
            let fragments = event
                .get("message")
                .and_then(Value::as_object)
                .and_then(|message| message.get("content"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect();
            (fragments, None)
        }
        Provider::OpenCode if event_type == Some("text") => {
            let text = event
                .get("part")
                .and_then(Value::as_object)
                .and_then(|part| part.get("text"))
                .and_then(Value::as_str)
                .or_else(|| event.get("text").and_then(Value::as_str));
            (text.into_iter().collect(), None)
        }
        Provider::Codex | Provider::ClaudeCode | Provider::OpenCode => (Vec::new(), None),
    }
}

fn required_string(params: &Map<String, Value>, name: &str) -> Result<String, AdapterError> {
    params
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            AdapterError::new(
                "invalid_params",
                format!("{name} must be a non-empty string"),
            )
        })
}

fn optional_string(
    params: &Map<String, Value>,
    name: &str,
) -> Result<Option<String>, AdapterError> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(_) => Err(AdapterError::new(
            "invalid_params",
            format!("{name} must be null or a non-empty string"),
        )),
    }
}

fn required_workspace(params: &Map<String, Value>) -> Result<PathBuf, AdapterError> {
    optional_workspace(params)?.ok_or_else(|| {
        AdapterError::new(
            "invalid_params",
            "workspace must be a non-empty string naming an existing directory",
        )
    })
}

fn optional_workspace(params: &Map<String, Value>) -> Result<Option<PathBuf>, AdapterError> {
    let Some(value) = params.get("workspace") else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AdapterError::new(
                "invalid_params",
                "workspace must be a non-empty string naming an existing directory",
            )
        })?;
    let path = dunce::canonicalize(Path::new(value)).map_err(|error| {
        AdapterError::new(
            "invalid_params",
            format!("cannot resolve workspace {value:?}: {error}"),
        )
        .with_details(json!({"workspace":value}))
    })?;
    if !path.is_dir() {
        return Err(AdapterError::new(
            "invalid_params",
            "workspace must name an existing directory",
        )
        .with_details(json!({"workspace":path})));
    }
    Ok(Some(path))
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn create(prefix: &str) -> Result<Self, AdapterError> {
        let path = env::temp_dir().join(format!("{prefix}-{}", unique_token()));
        fs::create_dir(&path).map_err(|error| {
            AdapterError::new(
                "temporary_directory_failed",
                format!("cannot create temporary provider directory: {error}"),
            )
        })?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct WorkspaceSnapshot {
    workspace: String,
    paths: BTreeMap<String, PathMetadata>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    skipped_paths: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct SnapshotWarnings {
    skipped_paths: BTreeSet<String>,
    examples: Vec<SnapshotWarning>,
}

#[derive(Debug, Serialize)]
struct SnapshotWarning {
    path: String,
    reason: String,
}

impl SnapshotWarnings {
    fn record(&mut self, root: &Path, path: &Path, reason: impl Into<String>) {
        let relative = path
            .strip_prefix(root)
            .map(slash_relative)
            .unwrap_or_else(|_| "<unresolved>".to_owned());
        if self.skipped_paths.insert(relative.clone())
            && self.examples.len() < MAX_SNAPSHOT_WARNING_EXAMPLES
        {
            self.examples.push(SnapshotWarning {
                path: relative,
                reason: reason.into(),
            });
        }
    }

    fn is_empty(&self) -> bool {
        self.skipped_paths.is_empty()
    }
}

struct SnapshotCapture {
    snapshot: WorkspaceSnapshot,
    warnings: SnapshotWarnings,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PathMetadata {
    kind: PathKind,
    size: u64,
    sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PathKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct EffectState {
    api: String,
    effect: String,
    state_kind: StateKind,
    workspace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invocation: Option<Value>,
    snapshot: WorkspaceSnapshot,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ObservationCompletion {
    api: String,
    effect: String,
    state_kind: ObservationCompletionKind,
    workspace: String,
    invocation: Value,
    value: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ObservationCompletionKind {
    ObservationCompletion,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StateKind {
    Observation,
    Snapshot,
}

struct SnapshotBudget {
    started: Instant,
    paths: usize,
    hash_bytes: u64,
}

impl SnapshotBudget {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            paths: 0,
            hash_bytes: 0,
        }
    }

    fn check_time(&self) -> io::Result<()> {
        if self.started.elapsed() > MAX_SNAPSHOT_DURATION {
            return Err(io::Error::other(format!(
                "snapshot budget exceeded: more than {} seconds elapsed",
                MAX_SNAPSHOT_DURATION.as_secs()
            )));
        }
        Ok(())
    }

    fn visit_path(&mut self) -> io::Result<()> {
        self.paths = self.paths.saturating_add(1);
        if self.paths > MAX_SNAPSHOT_PATHS {
            return Err(io::Error::other(format!(
                "snapshot budget exceeded: more than {MAX_SNAPSHOT_PATHS} paths"
            )));
        }
        self.check_time()
    }

    fn hash_bytes(&mut self, count: usize) -> io::Result<()> {
        self.hash_bytes = self
            .hash_bytes
            .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        if self.hash_bytes > MAX_SNAPSHOT_HASH_BYTES {
            return Err(io::Error::other(format!(
                "snapshot budget exceeded: more than {MAX_SNAPSHOT_HASH_BYTES} bytes hashed"
            )));
        }
        self.check_time()
    }
}

fn handle_workspace_paths<W: Write>(
    request: &PluginRequest,
    writer: &mut HostWriter<W>,
) -> Result<Value, AdapterError> {
    let (value, warnings) = match request.method.as_str() {
        "describe" => Ok((workspace_paths_description(), SnapshotWarnings::default())),
        "smoke" => Ok((
            json!({
                "name":"workspace.paths",
                "text":"workspace.paths ok",
                "live":false
            }),
            SnapshotWarnings::default(),
        )),
        "snapshot" => persist_snapshot(&request.params),
        "diff" => diff_persisted_snapshots(&request.params)
            .map(|value| (value, SnapshotWarnings::default())),
        "forget" => forget_state(&request.params).map(|value| (value, SnapshotWarnings::default())),
        "observe.begin" => observe_begin(&request.params),
        "observe.end" => observe_end(&request.params),
        method => Err(AdapterError::new(
            "method_not_found",
            format!("effect {PATH_EFFECT_NAME:?} does not implement {method:?}"),
        )
        .with_details(json!({
            "methods":[
                "describe", "smoke", "snapshot", "diff", "forget",
                "observe.begin", "observe.end"
            ]
        }))),
    }?;
    emit_snapshot_warning(writer, &warnings)?;
    Ok(value)
}

fn emit_snapshot_warning<W: Write>(
    writer: &mut HostWriter<W>,
    warnings: &SnapshotWarnings,
) -> Result<(), AdapterError> {
    if warnings.is_empty() {
        return Ok(());
    }
    let skipped = warnings.skipped_paths.len();
    writer.event(
        "effect.warning",
        Map::from_iter([
            (
                "code".to_owned(),
                Value::String("workspace.paths.skipped_paths".to_owned()),
            ),
            (
                "message".to_owned(),
                Value::String(format!(
                    "workspace.paths skipped {skipped} path(s) that could not be inspected; execution continued."
                )),
            ),
            ("skipped_paths".to_owned(), json!(skipped)),
            ("examples".to_owned(), json!(warnings.examples)),
        ]),
    )
}

fn workspace_paths_description() -> Value {
    json!({
        "api":PLUGIN_API,
        "kind":"effect",
        "name":PATH_EFFECT_NAME,
        "implementation_version":IMPLEMENTATION_VERSION,
        "methods":[
            "describe", "smoke", "observe.begin", "observe.end", "snapshot", "diff",
            "forget"
        ],
        "operations":[
            "describe", "smoke", "observe.begin", "observe.end", "snapshot", "diff",
            "forget"
        ],
        "options_schema":{"type":"object", "additionalProperties":true},
        "observation_end_semantics":"one durable commit with idempotent readers",
        "observation_completion_retention_seconds":OBSERVATION_COMPLETION_RETENTION.as_secs(),
        "observes":["added", "modified", "deleted", "type_changed"],
        "excludes":[
            "/.git", "/.tactus/path-effect", "/.tactus/dist-newstyle", "/.tactus/runs",
            "**/target", "**/node_modules", "**/build", "**/dist-newstyle"
        ],
        "budgets":{
            "max_paths":MAX_SNAPSHOT_PATHS,
            "max_hash_bytes":MAX_SNAPSHOT_HASH_BYTES,
            "max_duration_seconds":MAX_SNAPSHOT_DURATION.as_secs()
        },
        "transparent_directories":["/.tactus"],
        "follows_symlinks":false,
        "enforcement":false,
        "stores_content":false,
        "rollback":false
    })
}

fn persist_snapshot(
    params: &Map<String, Value>,
) -> Result<(Value, SnapshotWarnings), AdapterError> {
    let workspace = required_workspace(params)?;
    let capture = snapshot_workspace(&workspace)?;
    let snapshot = capture.snapshot;
    let snapshot_id = unique_token();
    let record = EffectState {
        api: PLUGIN_API.to_owned(),
        effect: PATH_EFFECT_NAME.to_owned(),
        state_kind: StateKind::Snapshot,
        workspace: workspace.to_string_lossy().into_owned(),
        invocation: None,
        snapshot,
    };
    let path = snapshot_state_path(&workspace, &snapshot_id, true)?;
    write_state_atomic(&path, &record)?;
    Ok((json!({"snapshot_id":snapshot_id}), capture.warnings))
}

fn diff_persisted_snapshots(params: &Map<String, Value>) -> Result<Value, AdapterError> {
    let workspace = required_workspace(params)?;
    let before_id = snapshot_handle(params, "before")?;
    let after_id = snapshot_handle(params, "after")?;
    let before = load_snapshot_state(&workspace, &before_id)?;
    let after = load_snapshot_state(&workspace, &after_id)?;
    Ok(snapshot_delta(&before, &after))
}

fn observe_begin(params: &Map<String, Value>) -> Result<(Value, SnapshotWarnings), AdapterError> {
    let workspace = required_workspace(params)?;
    cleanup_expired_observation_files(&workspace)?;
    let invocation = params
        .get("invocation")
        .cloned()
        .ok_or_else(|| AdapterError::new("invalid_params", "invocation is required"))?;
    let capture = snapshot_workspace(&workspace)?;
    let snapshot = capture.snapshot;
    let path_count = snapshot.paths.len();
    let token = unique_token();
    let record = EffectState {
        api: PLUGIN_API.to_owned(),
        effect: PATH_EFFECT_NAME.to_owned(),
        state_kind: StateKind::Observation,
        workspace: workspace.to_string_lossy().into_owned(),
        invocation: Some(invocation),
        snapshot,
    };
    let path = observation_state_path(&workspace, &token, true)?;
    write_state_atomic(&path, &record)?;
    Ok((
        json!({"token":token, "path_count":path_count}),
        capture.warnings,
    ))
}

fn observe_end(params: &Map<String, Value>) -> Result<(Value, SnapshotWarnings), AdapterError> {
    let workspace = required_workspace(params)?;
    let invocation = params
        .get("invocation")
        .cloned()
        .ok_or_else(|| AdapterError::new("invalid_params", "invocation is required"))?;
    let outcome = params
        .get("outcome")
        .cloned()
        .ok_or_else(|| AdapterError::new("invalid_params", "outcome is required"))?;
    let token = observation_token(params)?;
    let original = observation_state_path(&workspace, &token, false)?;
    if let Some(value) = load_observation_completion(&original, &workspace, Some(&invocation))? {
        cleanup_completed_observation(&original)?;
        return Ok((value, SnapshotWarnings::default()));
    }
    recover_interrupted_observation(&original)?;
    // Keep the durable token in its original location during the expensive
    // snapshot. If this one-shot host is interrupted, a later cleanup call can
    // retry it. The final completion record is created atomically: of concurrent
    // end calls, only one can commit the token successfully.
    let record = match load_state(&original, "observation_not_found") {
        Ok(record) => record,
        Err(error) if error.code == "observation_not_found" => {
            if let Some(value) =
                load_observation_completion(&original, &workspace, Some(&invocation))?
            {
                cleanup_completed_observation(&original)?;
                return Ok((value, SnapshotWarnings::default()));
            }
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    validate_state(&record, &workspace, StateKind::Observation)?;
    if record.invocation.as_ref() != Some(&invocation) {
        return Err(AdapterError::new(
            "state_invalid",
            "observation invocation does not match",
        ));
    }
    let capture = snapshot_workspace(&workspace)?;
    let after = capture.snapshot;
    let value = json!({
        "invocation":invocation,
        "outcome":outcome,
        "delta":snapshot_delta(&record.snapshot, &after),
        "before_count":record.snapshot.paths.len(),
        "after_count":after.paths.len()
    });
    let value = commit_observation(&original, &workspace, &invocation, &value)?;
    Ok((value, capture.warnings))
}

fn observation_completion_path(original: &Path) -> Result<PathBuf, AdapterError> {
    let parent = original.parent().ok_or_else(|| {
        AdapterError::new(
            "state_commit_failed",
            "observation state path has no parent",
        )
    })?;
    Ok(parent.join(format!(
        ".{}.completed",
        original.file_name().unwrap_or_default().to_string_lossy()
    )))
}

fn load_observation_completion(
    original: &Path,
    workspace: &Path,
    invocation: Option<&Value>,
) -> Result<Option<Value>, AdapterError> {
    let completed = observation_completion_path(original)?;
    let bytes = match fs::read(&completed) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AdapterError::new(
                "state_read_failed",
                format!("cannot read completed observation state: {error}"),
            )
            .with_details(json!({"path":completed})));
        }
    };
    let completion: ObservationCompletion = serde_json::from_slice(&bytes).map_err(|error| {
        AdapterError::new(
            "state_read_failed",
            format!("completed observation state is invalid JSON: {error}"),
        )
        .with_details(json!({"path":completed}))
    })?;
    if completion.api != PLUGIN_API
        || completion.effect != PATH_EFFECT_NAME
        || completion.state_kind != ObservationCompletionKind::ObservationCompletion
        || Path::new(&completion.workspace) != workspace
    {
        return Err(AdapterError::new(
            "state_invalid",
            "completed observation state metadata does not match this workspace effect",
        )
        .with_details(json!({"path":completed})));
    }
    if invocation.is_some_and(|invocation| &completion.invocation != invocation) {
        return Err(AdapterError::new(
            "state_invalid",
            "completed observation invocation does not match",
        ));
    }
    Ok(Some(completion.value))
}

fn commit_observation(
    original: &Path,
    workspace: &Path,
    invocation: &Value,
    value: &Value,
) -> Result<Value, AdapterError> {
    let completed = observation_completion_path(original)?;
    let parent = original.parent().ok_or_else(|| {
        AdapterError::new(
            "state_commit_failed",
            "observation state path has no parent",
        )
    })?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        original.file_name().unwrap_or_default().to_string_lossy(),
        unique_token()
    ));
    let completion = ObservationCompletion {
        api: PLUGIN_API.to_owned(),
        effect: PATH_EFFECT_NAME.to_owned(),
        state_kind: ObservationCompletionKind::ObservationCompletion,
        workspace: workspace.to_string_lossy().into_owned(),
        invocation: invocation.clone(),
        value: value.clone(),
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            AdapterError::new(
                "state_commit_failed",
                format!("cannot create completed observation state: {error}"),
            )
            .with_details(json!({"path":temporary}))
        })?;
    let operation = (|| {
        serde_json::to_writer(&mut file, &completion).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = operation {
        let _ = fs::remove_file(&temporary);
        return Err(AdapterError::new(
            "state_commit_failed",
            format!("cannot persist completed observation state: {error}"),
        )
        .with_details(json!({"path":temporary})));
    }
    let published = match fs::hard_link(&temporary, &completed) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(AdapterError::new(
                "state_commit_failed",
                format!("cannot atomically publish completed observation state: {error}"),
            )
            .with_details(json!({"path":completed, "temporary":temporary})));
        }
    };
    let _ = fs::remove_file(&temporary);
    let persisted = if published {
        value.clone()
    } else {
        load_observation_completion(original, workspace, Some(invocation))?.ok_or_else(|| {
            AdapterError::new(
                "state_commit_failed",
                "a concurrent completion disappeared before it could be read",
            )
            .with_details(json!({"path":completed}))
        })?
    };
    cleanup_completed_observation(original)?;
    Ok(persisted)
}

fn cleanup_completed_observation(original: &Path) -> Result<(), AdapterError> {
    match fs::remove_file(original) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AdapterError::new(
                "state_delete_failed",
                format!("cannot clean completed observation state: {error}"),
            )
            .with_details(json!({"path":original})));
        }
    }
    let claimed = observation_claim_path(original)?;
    match fs::remove_file(&claimed) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AdapterError::new(
                "state_delete_failed",
                format!("cannot clean legacy claimed observation state: {error}"),
            )
            .with_details(json!({"path":claimed})));
        }
    }
    Ok(())
}

fn observation_claim_path(original: &Path) -> Result<PathBuf, AdapterError> {
    let parent = original.parent().ok_or_else(|| {
        AdapterError::new("state_claim_failed", "observation state path has no parent")
    })?;
    Ok(parent.join(format!(
        ".{}.claimed",
        original.file_name().unwrap_or_default().to_string_lossy()
    )))
}

fn recover_interrupted_observation(original: &Path) -> Result<(), AdapterError> {
    if original.is_file() {
        return Ok(());
    }
    let claimed = observation_claim_path(original)?;
    if !claimed.is_file() {
        return Ok(());
    }
    // A healthy consumer holds the final claim for only one atomic rename and
    // delete. Give it a brief chance to finish before treating the claim as a
    // one-shot host that was interrupted between those operations.
    thread::sleep(Duration::from_millis(50));
    if original.is_file() || !claimed.is_file() {
        return Ok(());
    }
    fs::rename(&claimed, original).map_err(|error| {
        AdapterError::new(
            "state_restore_failed",
            format!("cannot recover interrupted observation state: {error}"),
        )
        .with_details(json!({"path":original}))
    })
}

fn cleanup_expired_observation_files(workspace: &Path) -> Result<(), AdapterError> {
    cleanup_expired_observation_files_with_retention(workspace, OBSERVATION_COMPLETION_RETENTION)
}

fn cleanup_expired_observation_files_with_retention(
    workspace: &Path,
    retention: Duration,
) -> Result<(), AdapterError> {
    let state = state_directory(workspace, false)?;
    let entries = match fs::read_dir(&state) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(state_directory_error(&state, "scan", error)),
    };
    let started = Instant::now();
    let mut removed = 0_usize;
    for entry in entries {
        if started.elapsed() >= MAX_COMPLETION_CLEANUP_DURATION {
            break;
        }
        let entry = entry.map_err(|error| state_directory_error(&state, "scan", error))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let completion_original = name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".completed"))
            .filter(|name| name.ends_with(".json"));
        let temporary = name.starts_with('.') && name.ends_with(".tmp");
        if completion_original.is_none() && !temporary {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(state_directory_error(&entry.path(), "inspect", error)),
        };
        if !metadata.is_file()
            || metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_none_or(|age| age < retention)
        {
            continue;
        }
        if removed >= MAX_COMPLETION_CLEANUP_ENTRIES {
            break;
        }
        if let Some(original_name) = completion_original {
            let original = state.join(original_name);
            match fs::remove_file(&original) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                // Windows can transiently deny deletion while a concurrent
                // cleanup iterator inspects the same entry. Retention cleanup
                // is best effort and must not make observe.begin fail.
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => continue,
                Err(error) => {
                    return Err(AdapterError::new(
                        "state_delete_failed",
                        format!("cannot delete residual expired observation state: {error}"),
                    )
                    .with_details(json!({"path":original})));
                }
            }
        }
        match fs::remove_file(entry.path()) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => continue,
            Err(error) => {
                return Err(AdapterError::new(
                    "state_delete_failed",
                    format!("cannot delete expired observation state: {error}"),
                )
                .with_details(json!({"path":entry.path()})));
            }
        }
    }
    Ok(())
}

fn forget_state(params: &Map<String, Value>) -> Result<Value, AdapterError> {
    let workspace = required_workspace(params)?;
    let (path, validate_invocation, observation) = if let Some(value) = params.get("snapshot_id") {
        let id = valid_token(value, "snapshot_id")?;
        (snapshot_state_path(&workspace, &id, false)?, None, false)
    } else {
        let token = observation_token(params)?;
        (
            observation_state_path(&workspace, &token, false)?,
            params.get("invocation").cloned(),
            true,
        )
    };
    if observation
        && load_observation_completion(&path, &workspace, validate_invocation.as_ref())?.is_some()
    {
        let completed = observation_completion_path(&path)?;
        match fs::remove_file(&completed) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(json!({"forgotten":false}));
            }
            Err(error) => {
                return Err(AdapterError::new(
                    "state_delete_failed",
                    format!("cannot delete completed observation state: {error}"),
                )
                .with_details(json!({"path":completed})));
            }
        }
        cleanup_completed_observation(&path)?;
        return Ok(json!({"forgotten":true}));
    }
    let Some(claimed) = claim_state(&path, true, "state_not_found")? else {
        return Ok(json!({"forgotten":false}));
    };
    let operation = (|| {
        if let Some(invocation) = &validate_invocation {
            let record = load_state(&claimed, "state_not_found")?;
            validate_state(&record, &workspace, StateKind::Observation)?;
            if record.invocation.as_ref() != Some(invocation) {
                return Err(AdapterError::new(
                    "state_invalid",
                    "observation invocation does not match",
                ));
            }
        }
        fs::remove_file(&claimed).map_err(|error| {
            AdapterError::new(
                "state_delete_failed",
                format!("cannot delete effect state: {error}"),
            )
            .with_details(json!({"path":claimed}))
        })
    })();
    if operation.is_err() {
        restore_claim(&claimed, &path)?;
        operation?;
    }
    Ok(json!({"forgotten":true}))
}

fn snapshot_workspace(workspace: &Path) -> Result<SnapshotCapture, AdapterError> {
    let workspace = dunce::canonicalize(workspace).map_err(|error| {
        AdapterError::new(
            "snapshot_failed",
            format!("cannot resolve workspace: {error}"),
        )
    })?;
    if !workspace.is_dir() {
        return Err(AdapterError::new(
            "invalid_params",
            "workspace must name an existing directory",
        ));
    }
    let mut paths = BTreeMap::new();
    let mut budget = SnapshotBudget::new();
    let mut warnings = SnapshotWarnings::default();
    scan_directory(
        &workspace,
        &workspace,
        &mut paths,
        &mut budget,
        &mut warnings,
    )?;
    Ok(SnapshotCapture {
        snapshot: WorkspaceSnapshot {
            workspace: workspace.to_string_lossy().into_owned(),
            paths,
            skipped_paths: warnings.skipped_paths.clone(),
        },
        warnings,
    })
}

fn scan_directory(
    root: &Path,
    directory: &Path,
    paths: &mut BTreeMap<String, PathMetadata>,
    budget: &mut SnapshotBudget,
    warnings: &mut SnapshotWarnings,
) -> Result<(), AdapterError> {
    budget
        .check_time()
        .map_err(|error| snapshot_io_error(root, directory, error))?;
    ensure_real_path_within(root, directory)
        .map_err(|error| snapshot_io_error(root, directory, error))?;
    let reader =
        fs::read_dir(directory).map_err(|error| snapshot_io_error(root, directory, error))?;
    let mut entries = Vec::new();
    for entry in reader {
        budget
            .check_time()
            .map_err(|error| snapshot_io_error(root, directory, error))?;
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                warnings.record(root, directory, "directory_entry_unreadable");
                continue;
            }
        };
        let absolute = entry.path();
        let relative_path = match absolute.strip_prefix(root) {
            Ok(relative) => relative,
            Err(_) => {
                warnings.record(root, &absolute, "path_outside_workspace");
                continue;
            }
        };
        let relative = slash_relative(relative_path);
        if is_excluded(&relative) {
            continue;
        }
        budget
            .visit_path()
            .map_err(|error| snapshot_io_error(root, &absolute, error))?;
        entries.push((entry, relative));
    }
    ensure_real_path_within(root, directory)
        .map_err(|error| snapshot_io_error(root, directory, error))?;
    entries.sort_by_key(|(entry, _)| entry.file_name());
    for (entry, relative) in entries {
        let absolute = entry.path();
        let metadata = match fs::symlink_metadata(&absolute) {
            Ok(metadata) => metadata,
            Err(error) => {
                warnings.record(root, &absolute, snapshot_warning_reason(&error));
                continue;
            }
        };
        let path_metadata = match read_path_metadata(root, &absolute, &metadata, budget) {
            Ok(metadata) => metadata,
            Err(error) if is_snapshot_budget_error(&error) => {
                return Err(snapshot_io_error(root, &absolute, error));
            }
            Err(error) => {
                warnings.record(root, &absolute, snapshot_warning_reason(&error));
                continue;
            }
        };
        let transparent = normalize_relative(&relative) == normalize_relative(".tactus");
        if !transparent {
            paths.insert(relative, path_metadata.clone());
        }
        if path_metadata.kind == PathKind::Directory
            && let Err(error) = scan_directory(root, &absolute, paths, budget, warnings)
        {
            if error.code == "snapshot_budget_exceeded" {
                return Err(error);
            }
            warnings.record(root, &absolute, "directory_unreadable");
        }
    }
    Ok(())
}

fn read_path_metadata(
    root: &Path,
    path: &Path,
    initial: &fs::Metadata,
    budget: &mut SnapshotBudget,
) -> io::Result<PathMetadata> {
    let file_type = initial.file_type();
    if file_type.is_symlink() {
        let target = fs::read_link(path)?;
        let bytes = path_bytes(&target);
        return Ok(PathMetadata {
            kind: PathKind::Symlink,
            size: initial.len(),
            sha256: Some(hex_digest(&bytes)),
        });
    }
    if file_type.is_dir() {
        return Ok(PathMetadata {
            kind: PathKind::Directory,
            size: 0,
            sha256: None,
        });
    }
    if file_type.is_file() {
        let current_before_open = fs::symlink_metadata(path)?;
        if !current_before_open.is_file()
            || current_before_open.file_type().is_symlink()
            || mutation_identity(initial) != mutation_identity(&current_before_open)
        {
            return Err(io::Error::other("file changed before hashing"));
        }
        ensure_real_path_within(root, path)?;
        let mut file = File::open(path)?;
        let opened = file.metadata()?;
        if !opened.is_file() || mutation_identity(initial) != mutation_identity(&opened) {
            return Err(io::Error::other("file changed before hashing"));
        }
        let mut digest = Sha256::new();
        // Keep the hashing buffer off the stack. The effect host runs on the
        // process main thread, whose default Windows stack can be only 1 MiB;
        // a 1 MiB local array therefore overflowed before the first file was
        // hashed.
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            budget.check_time()?;
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            budget.hash_bytes(count)?;
            digest.update(&buffer[..count]);
        }
        let finished = file.metadata()?;
        let current = fs::symlink_metadata(path)?;
        ensure_real_path_within(root, path)?;
        if !current.is_file()
            || mutation_identity(&opened) != mutation_identity(&finished)
            || mutation_identity(&finished) != mutation_identity(&current)
        {
            return Err(io::Error::other("file changed while hashing"));
        }
        return Ok(PathMetadata {
            kind: PathKind::File,
            size: current.len(),
            sha256: Some(format!("{:x}", digest.finalize())),
        });
    }
    Ok(PathMetadata {
        kind: PathKind::Other,
        size: initial.len(),
        sha256: None,
    })
}

fn ensure_real_path_within(root: &Path, path: &Path) -> io::Result<()> {
    let relative = path.strip_prefix(root).map_err(|_| {
        io::Error::other(format!(
            "path is lexically outside workspace: {}",
            path.display()
        ))
    })?;
    // `dunce::canonicalize` intentionally preserves the Windows `\\?\` prefix
    // for paths longer than MAX_PATH. Comparing that value with a short,
    // prefix-free root rejects a valid descendant. Use the standard canonical
    // representation for both sides, then derive the expected lexical path from
    // the same canonical root. This keeps the symlink/junction escape check while
    // treating long and short descendants consistently.
    let resolved_root = fs::canonicalize(root)?;
    let resolved = fs::canonicalize(path)?;
    if !resolved.starts_with(&resolved_root) {
        return Err(io::Error::other(format!(
            "path resolved outside workspace: {}",
            resolved.display()
        )));
    }
    let expected = resolved_root.join(relative);
    if resolved != expected {
        return Err(io::Error::other(format!(
            "path changed into a symlink or alias during snapshot: {}",
            path.display()
        )));
    }
    Ok(())
}

fn is_snapshot_budget_error(error: &io::Error) -> bool {
    error.to_string().starts_with("snapshot budget exceeded:")
}

fn snapshot_warning_reason(error: &io::Error) -> &'static str {
    let message = error.to_string();
    if message.contains("outside workspace") {
        "path_outside_workspace"
    } else if message.contains("symlink or alias") {
        "path_alias_changed"
    } else if message.contains("changed") {
        "path_changed_during_snapshot"
    } else {
        match error.kind() {
            io::ErrorKind::NotFound => "path_disappeared",
            io::ErrorKind::PermissionDenied => "permission_denied",
            _ => "path_unreadable",
        }
    }
}

fn mutation_identity(metadata: &fs::Metadata) -> (u64, Option<SystemTime>, Option<SystemTime>) {
    (
        metadata.len(),
        metadata.modified().ok(),
        metadata.created().ok(),
    )
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn slash_relative(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_relative(value: &str) -> String {
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value.to_owned()
    }
}

fn is_excluded(relative: &str) -> bool {
    let relative = normalize_relative(relative);
    let exact = [
        ".git",
        ".tactus/path-effect",
        ".tactus/dist-newstyle",
        ".tactus/runs",
    ]
    .iter()
    .map(|value| normalize_relative(value))
    .any(|excluded| relative == excluded || relative.starts_with(&format!("{excluded}/")));
    exact
        || relative.split('/').any(|segment| {
            matches!(
                segment,
                "target" | "node_modules" | "build" | "dist-newstyle"
            )
        })
}

fn snapshot_io_error(root: &Path, path: &Path, error: io::Error) -> AdapterError {
    let budget_exceeded = error.to_string().starts_with("snapshot budget exceeded:");
    AdapterError::new(
        if budget_exceeded {
            "snapshot_budget_exceeded"
        } else {
            "snapshot_failed"
        },
        format!("cannot snapshot workspace path {}: {error}", path.display()),
    )
    .with_details(json!({
        "workspace":root,
        "path":path,
        "max_paths":MAX_SNAPSHOT_PATHS,
        "max_hash_bytes":MAX_SNAPSHOT_HASH_BYTES,
        "max_duration_seconds":MAX_SNAPSHOT_DURATION.as_secs()
    }))
}

fn snapshot_delta(before: &WorkspaceSnapshot, after: &WorkspaceSnapshot) -> Value {
    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    let mut type_changed = Vec::new();
    for (path, new) in &after.paths {
        if snapshot_path_was_skipped(before, after, path) {
            continue;
        }
        match before.paths.get(path) {
            None => added.push(path.clone()),
            Some(old) if old.kind != new.kind => type_changed.push(path.clone()),
            Some(old) if old != new => modified.push(path.clone()),
            Some(_) => {}
        }
    }
    for path in before.paths.keys() {
        if !snapshot_path_was_skipped(before, after, path) && !after.paths.contains_key(path) {
            deleted.push(path.clone());
        }
    }
    json!({
        "added":added,
        "modified":modified,
        "deleted":deleted,
        "type_changed":type_changed
    })
}

fn snapshot_path_was_skipped(
    before: &WorkspaceSnapshot,
    after: &WorkspaceSnapshot,
    path: &str,
) -> bool {
    let path = normalize_relative(path);
    before
        .skipped_paths
        .iter()
        .chain(&after.skipped_paths)
        .any(|skipped| {
            let skipped = normalize_relative(skipped);
            path == skipped || path.starts_with(&format!("{skipped}/"))
        })
}

fn state_directory(workspace: &Path, create: bool) -> Result<PathBuf, AdapterError> {
    // Keep this shared directory after deleting its records. Another one-shot
    // host may have prepared a path here but not yet opened its temporary file;
    // removing an apparently empty directory would make that write fail.
    let tactus = workspace.join(".tactus");
    let state = tactus.join("path-effect");
    for directory in [&tactus, &state] {
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(AdapterError::new(
                    "state_path_invalid",
                    "effect state path must contain only real directories",
                )
                .with_details(json!({"path":directory})));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                match fs::create_dir(directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let metadata = fs::symlink_metadata(directory).map_err(|error| {
                            state_directory_error(directory, "reinspect", error)
                        })?;
                        if !metadata.is_dir() || metadata.file_type().is_symlink() {
                            return Err(AdapterError::new(
                                "state_path_invalid",
                                "concurrently created effect state path is not a real directory",
                            )
                            .with_details(json!({"path":directory})));
                        }
                    }
                    Err(error) => {
                        return Err(state_directory_error(directory, "create", error));
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(state.clone()),
            Err(error) => {
                return Err(AdapterError::new(
                    "state_path_invalid",
                    format!("cannot inspect effect state directory: {error}"),
                )
                .with_details(json!({"path":directory})));
            }
        }
    }
    Ok(state)
}

fn state_directory_error(path: &Path, operation: &str, error: io::Error) -> AdapterError {
    AdapterError::new(
        "state_path_invalid",
        format!("cannot {operation} effect state directory: {error}"),
    )
    .with_details(json!({"path":path}))
}

fn observation_state_path(
    workspace: &Path,
    token: &str,
    create: bool,
) -> Result<PathBuf, AdapterError> {
    validate_token(token, "token")?;
    Ok(state_directory(workspace, create)?.join(format!("{token}.json")))
}

fn snapshot_state_path(
    workspace: &Path,
    snapshot_id: &str,
    create: bool,
) -> Result<PathBuf, AdapterError> {
    validate_token(snapshot_id, "snapshot_id")?;
    Ok(state_directory(workspace, create)?.join(format!("snapshot-{snapshot_id}.json")))
}

fn write_state_atomic(path: &Path, record: &EffectState) -> Result<(), AdapterError> {
    let parent = path.parent().ok_or_else(|| {
        AdapterError::new("state_write_failed", "effect state path has no parent")
    })?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        unique_token()
    ));
    let operation = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        serde_json::to_writer(&mut file, record).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if let Err(error) = operation {
        let _ = fs::remove_file(&temporary);
        return Err(AdapterError::new(
            "state_write_failed",
            format!("cannot persist effect state atomically: {error}"),
        )
        .with_details(json!({"path":path})));
    }
    Ok(())
}

fn claim_state(
    path: &Path,
    missing_ok: bool,
    not_found_code: &str,
) -> Result<Option<PathBuf>, AdapterError> {
    let parent = path.parent().ok_or_else(|| {
        AdapterError::new("state_claim_failed", "effect state path has no parent")
    })?;
    let claimed = parent.join(format!(
        ".{}.{}.claimed",
        path.file_name().unwrap_or_default().to_string_lossy(),
        unique_token()
    ));
    match fs::rename(path, &claimed) {
        Ok(()) => Ok(Some(claimed)),
        Err(error) if error.kind() == io::ErrorKind::NotFound && missing_ok => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(AdapterError::new(
            not_found_code,
            "effect state was not found or was already claimed",
        )),
        Err(error) => Err(AdapterError::new(
            "state_claim_failed",
            format!("cannot claim effect state: {error}"),
        )
        .with_details(json!({"path":path}))),
    }
}

fn restore_claim(claimed: &Path, original: &Path) -> Result<(), AdapterError> {
    fs::rename(claimed, original).map_err(|error| {
        AdapterError::new(
            "state_restore_failed",
            format!("cannot restore claimed effect state: {error}"),
        )
        .with_details(json!({"path":original}))
    })
}

fn load_state(path: &Path, not_found_code: &str) -> Result<EffectState, AdapterError> {
    let bytes = fs::read(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            AdapterError::new(not_found_code, "effect state was not found")
        } else {
            AdapterError::new(
                "state_read_failed",
                format!("cannot read effect state: {error}"),
            )
            .with_details(json!({"path":path}))
        }
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        AdapterError::new(
            "state_read_failed",
            format!("effect state is invalid JSON: {error}"),
        )
        .with_details(json!({"path":path}))
    })
}

fn load_snapshot_state(
    workspace: &Path,
    snapshot_id: &str,
) -> Result<WorkspaceSnapshot, AdapterError> {
    let path = snapshot_state_path(workspace, snapshot_id, false)?;
    let state = load_state(&path, "snapshot_not_found")?;
    validate_state(&state, workspace, StateKind::Snapshot)?;
    Ok(state.snapshot)
}

fn validate_state(
    state: &EffectState,
    workspace: &Path,
    kind: StateKind,
) -> Result<(), AdapterError> {
    if state.api != PLUGIN_API
        || state.effect != PATH_EFFECT_NAME
        || state.state_kind != kind
        || state.workspace != workspace.to_string_lossy()
        || state.snapshot.workspace != workspace.to_string_lossy()
    {
        return Err(AdapterError::new(
            "state_invalid",
            "stored effect state does not match its protocol, kind, or workspace",
        ));
    }
    Ok(())
}

fn observation_token(params: &Map<String, Value>) -> Result<String, AdapterError> {
    let nested = match params.get("begin") {
        None => None,
        Some(Value::Object(begin)) => begin.get("token"),
        Some(_) => {
            return Err(AdapterError::new(
                "invalid_params",
                "begin must be a JSON object",
            ));
        }
    };
    let direct = params.get("token");
    if let (Some(nested), Some(direct)) = (nested, direct)
        && nested != direct
    {
        return Err(AdapterError::new(
            "invalid_params",
            "begin.token and token do not match",
        ));
    }
    valid_token(nested.or(direct).unwrap_or(&Value::Null), "token")
}

fn snapshot_handle(params: &Map<String, Value>, name: &str) -> Result<String, AdapterError> {
    let handle = params.get(name).and_then(Value::as_object).ok_or_else(|| {
        AdapterError::new(
            "invalid_params",
            format!("{name} must be a snapshot handle object"),
        )
    })?;
    valid_token(
        handle.get("snapshot_id").unwrap_or(&Value::Null),
        "snapshot_id",
    )
}

fn valid_token(value: &Value, name: &str) -> Result<String, AdapterError> {
    let token = value.as_str().ok_or_else(|| {
        AdapterError::new(
            "invalid_params",
            format!("{name} must be a 48-character lowercase hex string"),
        )
    })?;
    validate_token(token, name)?;
    Ok(token.to_owned())
}

fn validate_token(token: &str, name: &str) -> Result<(), AdapterError> {
    if token.len() == 48
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(AdapterError::new(
            "invalid_params",
            format!("{name} must be a 48-character lowercase hex string"),
        ))
    }
}

fn unique_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    static PROCESS_SALT: OnceLock<[u8; 32]> = OnceLock::new();
    let salt = PROCESS_SALT.get_or_init(|| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let mut digest = Sha256::new();
        digest.update(now.to_le_bytes());
        digest.update(std::process::id().to_le_bytes());
        digest.finalize().into()
    });
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut digest = Sha256::new();
    digest.update(salt);
    digest.update(now.to_le_bytes());
    digest.update(count.to_le_bytes());
    format!("{:x}", digest.finalize())[..48].to_owned()
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::{Arc, Barrier};

    #[test]
    fn builtin_host_accepts_requests_above_the_default_but_below_the_hard_ceiling() {
        let request = serde_json::to_vec(&serde_json::json!({
            "api":PLUGIN_API,
            "id":"large-request",
            "method":"fixture",
            "params":{"payload":"x".repeat(1536 * 1024)},
        }))
        .expect("large request JSON");
        assert!(request.len() > 1024 * 1024);
        assert!(request.len() < MAX_REQUEST_BYTES);
        let mut output = Vec::new();
        let status = run_host(
            Cursor::new(request),
            &mut output,
            Vec::new(),
            |_request, _writer, _diagnostics| Ok(serde_json::json!({"accepted":true})),
        );
        assert_eq!(status, 0);
        assert!(
            String::from_utf8(output)
                .expect("host output")
                .contains("\"accepted\":true")
        );
    }

    #[test]
    fn invocation_options_accept_an_explicit_native_executable() {
        let params = json!({
            "options": {
                "executable": "tools/中文 provider/claude.exe"
            }
        });
        let options = invocation_options(
            params.as_object().expect("params object"),
            Some(DEFAULT_SMOKE_TIMEOUT),
        )
        .expect("provider options");

        assert_eq!(
            options.executable.as_deref(),
            Some("tools/中文 provider/claude.exe")
        );
    }

    #[cfg(windows)]
    #[test]
    fn workspace_snapshot_accepts_valid_paths_longer_than_max_path() {
        use std::os::windows::ffi::OsStrExt;

        let workspace = tempfile::tempdir().expect("temporary workspace");
        let nested = workspace.path().join("a".repeat(90)).join("b".repeat(90));
        fs::create_dir_all(&nested).expect("long nested directory");
        let file = nested.join(format!("{}.txt", "c".repeat(90)));
        fs::write(&file, b"long path content").expect("long path file");
        assert!(file.as_os_str().encode_wide().count() > 260);

        let capture = snapshot_workspace(workspace.path()).expect("long path snapshot");
        let relative = slash_relative(
            file.strip_prefix(workspace.path())
                .expect("relative long path"),
        );
        assert!(capture.snapshot.paths.contains_key(&relative));
        assert!(capture.warnings.is_empty());
    }

    #[test]
    fn skipped_paths_do_not_become_false_workspace_deltas() {
        let metadata = PathMetadata {
            kind: PathKind::File,
            size: 4,
            sha256: Some("old".to_owned()),
        };
        let before = WorkspaceSnapshot {
            workspace: "/workspace".to_owned(),
            paths: BTreeMap::from([
                ("blocked/file.txt".to_owned(), metadata.clone()),
                ("visible.txt".to_owned(), metadata),
            ]),
            skipped_paths: BTreeSet::new(),
        };
        let after = WorkspaceSnapshot {
            workspace: "/workspace".to_owned(),
            paths: BTreeMap::new(),
            skipped_paths: BTreeSet::from(["blocked".to_owned()]),
        };

        let delta = snapshot_delta(&before, &after);
        assert_eq!(delta["deleted"], json!(["visible.txt"]));
    }

    #[test]
    fn skipped_paths_emit_one_non_terminal_warning_before_success() {
        let mut output = Vec::new();
        let mut writer = HostWriter::new(&mut output, json!("warning-test"));
        let mut warnings = SnapshotWarnings::default();
        warnings.record(
            Path::new("/workspace"),
            Path::new("/workspace/blocked.txt"),
            "permission denied",
        );
        emit_snapshot_warning(&mut writer, &warnings).expect("warning event");
        writer
            .success(json!({"ok":true}))
            .expect("terminal success");
        drop(writer);

        let frames = std::str::from_utf8(&output)
            .expect("UTF-8 frames")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("JSON frame"))
            .collect::<Vec<_>>();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["type"], "event");
        assert_eq!(frames[0]["event"]["type"], "effect.warning");
        assert_eq!(frames[0]["event"]["skipped_paths"], 1);
        assert_eq!(frames[1]["type"], "result");
        assert_eq!(frames[1]["ok"], true);
    }

    #[test]
    fn snapshot_budgets_fail_closed_before_unbounded_work() {
        let mut paths = SnapshotBudget {
            started: Instant::now(),
            paths: MAX_SNAPSHOT_PATHS,
            hash_bytes: 0,
        };
        assert!(
            paths
                .visit_path()
                .expect_err("path budget")
                .to_string()
                .contains("snapshot budget exceeded")
        );

        let mut bytes = SnapshotBudget {
            started: Instant::now(),
            paths: 0,
            hash_bytes: MAX_SNAPSHOT_HASH_BYTES,
        };
        assert!(
            bytes
                .hash_bytes(1)
                .expect_err("byte budget")
                .to_string()
                .contains("snapshot budget exceeded")
        );
    }

    #[test]
    fn pending_state_write_survives_concurrent_observation_cleanup() {
        for forget_existing in [false, true] {
            let temporary = tempfile::tempdir().expect("temporary workspace");
            let workspace = dunce::canonicalize(temporary.path()).expect("workspace");
            let mut params = Map::from_iter([
                ("workspace".to_owned(), json!(workspace)),
                ("invocation".to_owned(), json!({"step":"earlier"})),
            ]);
            let earlier =
                forget_existing.then(|| observe_begin(&params).expect("earlier observation").0);
            let token = unique_token();
            // Pause a writer after it has prepared its directory but before
            // creating the temporary file. Run the other caller's cleanup in
            // that exact window rather than depending on thread scheduling.
            let pending = observation_state_path(&workspace, &token, true).expect("pending path");
            let record = EffectState {
                api: PLUGIN_API.to_owned(),
                effect: PATH_EFFECT_NAME.to_owned(),
                state_kind: StateKind::Observation,
                workspace: workspace.to_string_lossy().into_owned(),
                invocation: Some(json!({"step":"pending"})),
                snapshot: snapshot_workspace(&workspace).expect("snapshot").snapshot,
            };
            if let Some(earlier) = earlier {
                params.insert("begin".to_owned(), earlier);
                assert_eq!(
                    forget_state(&params).expect("forget earlier")["forgotten"],
                    true
                );
            } else {
                cleanup_expired_observation_files(&workspace).expect("concurrent begin cleanup");
            }
            write_state_atomic(&pending, &record)
                .expect("cleanup must not remove a writer's directory");
            let persisted = load_state(&pending, "observation_not_found").expect("persisted state");
            validate_state(&persisted, &workspace, StateKind::Observation).expect("valid state");
            assert_eq!(persisted.invocation, record.invocation);
            assert_eq!(persisted.snapshot, record.snapshot);
        }
    }

    #[test]
    fn concurrent_expired_completion_cleanup_tolerates_racing_removals() {
        let temporary = tempfile::tempdir().expect("temporary workspace");
        let state = temporary.path().join(".tactus/path-effect");
        fs::create_dir_all(&state).expect("state directory");
        for index in 0..256 {
            fs::write(state.join(format!(".{index:048x}.json.completed")), b"done")
                .expect("completion fixture");
            fs::write(
                state.join(format!(".{index:048x}.json.{index:048x}.tmp")),
                b"temp",
            )
            .expect("temporary fixture");
        }
        let barrier = Arc::new(Barrier::new(2));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let workspace = temporary.path().to_path_buf();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                cleanup_expired_observation_files_with_retention(&workspace, Duration::ZERO)
            }));
        }
        for worker in workers {
            worker
                .join()
                .expect("cleanup worker")
                .expect("concurrent cleanup");
        }
        // Cleanup is deliberately time-bounded. Under parallel test load one
        // pass may make only partial progress, so exercise the same eventual
        // maintenance contract a caller would use on later invocations.
        for _ in 0..8 {
            cleanup_expired_observation_files_with_retention(temporary.path(), Duration::ZERO)
                .expect("final cleanup after concurrent iterators close");
            if !state.exists()
                || fs::read_dir(&state)
                    .expect("remaining state directory")
                    .next()
                    .is_none()
            {
                break;
            }
        }
        assert!(
            !state.exists()
                || fs::read_dir(&state)
                    .expect("remaining state directory")
                    .next()
                    .is_none()
        );
    }

    #[test]
    fn codex_last_message_is_bounded_while_reading() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let path = temporary.path().join("last-message.txt");
        fs::write(&path, "孔洞").expect("small last message");
        assert_eq!(
            read_codex_last_message(&path, MAX_PROVIDER_RESULT_BYTES).expect("bounded read"),
            Some("孔洞".to_owned())
        );

        fs::write(&path, vec![b'x'; MAX_PROVIDER_RESULT_BYTES + 1])
            .expect("oversized last message");
        let error =
            read_codex_last_message(&path, MAX_PROVIDER_RESULT_BYTES).expect_err("result limit");
        assert_eq!(error.code, "outcome_unknown");
        assert_eq!(
            error.details.as_ref().expect("error details")["cause"],
            "provider_result_limit"
        );
    }

    #[test]
    fn provider_result_accumulator_bounds_fragments_with_an_injected_limit() {
        let mut result = ProviderResultAccumulator::new(8);
        result.observe_fragments(&["1234", "5678"]);
        assert_eq!(result.text, "12345678");

        result.observe_fragments(&["9"]);
        assert!(result.text.is_empty(), "oversized text stayed retained");
        result.observe_fragments(&["more output that must not be retained"]);

        assert_eq!(
            result.finish(),
            Err(ProviderResultOverflow {
                result_bytes_at_least: 9,
                max_result_bytes: 8,
            })
        );
    }

    #[test]
    fn provider_result_overflow_keeps_observing_and_becomes_outcome_unknown() {
        let mut result = ProviderResultAccumulator::new(4);
        let mut diagnostics = ProviderEventDiagnostics::default();
        observe_provider_output(
            Provider::ClaudeCode,
            br#"{"type":"result","result":"abcde"}"#,
            &mut result,
            &mut diagnostics,
        )
        .expect("valid oversized result event");
        observe_provider_output(
            Provider::ClaudeCode,
            br#"{"type":"thinking"}"#,
            &mut result,
            &mut diagnostics,
        )
        .expect("valid diagnostic event");

        assert_eq!(diagnostics.native_events, 2);
        assert_eq!(diagnostics.plain_lines, 0);
        assert_eq!(diagnostics.json_events, 2);
        let error = finish_provider_result(Provider::ClaudeCode, result)
            .expect_err("oversized result must remain outcome-unknown");
        assert_eq!(error.code, "outcome_unknown");
        let details = error.details.expect("overflow details");
        assert_eq!(details["cause"], "provider_result_limit");
        assert_eq!(details["result_bytes_at_least"], 5);
        assert_eq!(details["max_result_bytes"], 4);
    }

    #[test]
    fn provider_result_accumulator_preserves_plain_line_and_final_precedence() {
        let mut plain = ProviderResultAccumulator::new(5);
        plain.observe_plain("ab");
        plain.observe_plain("cd");
        assert_eq!(plain.finish().expect("bounded plain text"), "ab\ncd");

        let mut final_result = ProviderResultAccumulator::new(8);
        final_result.observe_plain("fallback");
        final_result.observe_final("final");
        final_result.observe_plain("ignored");
        assert_eq!(final_result.finish().expect("bounded final text"), "final");
    }

    #[test]
    fn provider_result_accumulator_recovers_when_a_higher_precedence_candidate_fits() {
        let mut fragments = ProviderResultAccumulator::new(4);
        fragments.observe_plain("oversized fallback");
        fragments.observe_fragments(&["okay"]);
        assert_eq!(fragments.finish().expect("bounded fragments"), "okay");

        let mut final_result = ProviderResultAccumulator::new(5);
        final_result.observe_fragments(&["oversized fragments"]);
        final_result.observe_final("first");
        final_result.observe_final("last");
        assert_eq!(final_result.finish().expect("bounded final text"), "last");

        let mut recovered_final = ProviderResultAccumulator::new(5);
        recovered_final.observe_final("oversized final");
        recovered_final.observe_final("last");
        assert_eq!(
            recovered_final.finish().expect("later bounded final text"),
            "last"
        );
    }

    #[test]
    fn native_output_must_be_utf8_json_and_contain_a_recognized_result() {
        let mut invalid_utf8 = ProviderResultAccumulator::new(64);
        let mut diagnostics = ProviderEventDiagnostics::default();
        let error = observe_provider_output(
            Provider::ClaudeCode,
            b"answer:\xff",
            &mut invalid_utf8,
            &mut diagnostics,
        )
        .expect_err("invalid UTF-8 cannot become a successful result");
        assert_eq!(error.code, "outcome_unknown");
        assert_eq!(
            error.details.expect("UTF-8 details")["cause"],
            "native_output_invalid_utf8"
        );

        let mut malformed = ProviderResultAccumulator::new(64);
        let error = observe_provider_output(
            Provider::ClaudeCode,
            b"not-json",
            &mut malformed,
            &mut diagnostics,
        )
        .expect_err("malformed JSON cannot become a plain-text fallback");
        assert_eq!(
            error.details.expect("JSON details")["cause"],
            "native_output_malformed_json"
        );

        let mut unknown = ProviderResultAccumulator::new(64);
        observe_provider_output(
            Provider::ClaudeCode,
            br#"{"type":"future.schema","payload":1}"#,
            &mut unknown,
            &mut diagnostics,
        )
        .expect("unknown valid JSON remains observational");
        assert!(!unknown.has_authoritative_result(Provider::ClaudeCode));

        let mut partial = ProviderResultAccumulator::new(64);
        observe_provider_output(
            Provider::ClaudeCode,
            br#"{"type":"assistant","message":{"content":[{"type":"text","text":"partial"}]}}"#,
            &mut partial,
            &mut diagnostics,
        )
        .expect("valid partial result");
        assert!(!partial.has_authoritative_result(Provider::ClaudeCode));

        let mut reported_failure = ProviderResultAccumulator::new(64);
        observe_provider_output(
            Provider::ClaudeCode,
            br#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"failed"}"#,
            &mut reported_failure,
            &mut diagnostics,
        )
        .expect("valid terminal failure record");
        assert!(reported_failure.has_authoritative_result(Provider::ClaudeCode));
        assert!(reported_failure.reported_failure());

        let mut failure_after_oversized_partial = ProviderResultAccumulator::new(4);
        observe_provider_output(
            Provider::ClaudeCode,
            br#"{"type":"assistant","message":{"content":[{"type":"text","text":"oversized partial"}]}}"#,
            &mut failure_after_oversized_partial,
            &mut diagnostics,
        )
        .expect("oversized partial remains observational");
        observe_provider_output(
            Provider::ClaudeCode,
            br#"{"type":"result","subtype":"error_during_execution","is_error":true}"#,
            &mut failure_after_oversized_partial,
            &mut diagnostics,
        )
        .expect("terminal failure without result text");
        let (text, recognized, reported) =
            finalize_provider_result(Provider::ClaudeCode, failure_after_oversized_partial)
                .expect("known terminal failure outranks partial overflow");
        assert!(text.is_empty());
        assert!(recognized);
        assert!(reported);

        let mut empty = ProviderResultAccumulator::new(64);
        observe_provider_output(
            Provider::ClaudeCode,
            br#"{"type":"result","result":""}"#,
            &mut empty,
            &mut diagnostics,
        )
        .expect("empty but explicit result");
        assert!(empty.has_authoritative_result(Provider::ClaudeCode));
        assert_eq!(empty.finish().expect("empty result"), "");

        let mut opencode = ProviderResultAccumulator::new(64);
        observe_provider_output(
            Provider::OpenCode,
            br#"{"type":"text","part":{"text":"done"}}"#,
            &mut opencode,
            &mut diagnostics,
        )
        .expect("OpenCode text record");
        assert!(opencode.has_authoritative_result(Provider::OpenCode));
    }

    #[test]
    fn every_post_spawn_infrastructure_failure_is_ambiguous() {
        for cause in [
            "native_stdout_read_failed",
            "native_wait_failed",
            "native_prompt_write_failed",
            "native_stderr_capture_failed",
        ] {
            let error = dispatched_provider_failure(Provider::Codex, cause, "bounded fixture");
            assert_eq!(error.code, "outcome_unknown");
            assert_eq!(error.details.expect("details")["cause"], cause);
        }
    }
}
