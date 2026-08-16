//! Bounded process-group supervision and incremental plugin transport.

use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    ops::{Deref, DerefMut},
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

use command_group::{CommandGroup, GroupChild};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::{
    FrameSequence, PluginFrame, PluginRequest, ProtocolFault, TerminalResult, decode_frame,
};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const CALLBACK_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const PIPE_WORKER_DRAIN_TIMEOUT: Duration = Duration::from_millis(250);

struct SupervisedChild {
    child: GroupChild,
    armed: bool,
}

impl SupervisedChild {
    fn new(child: GroupChild) -> Self {
        Self { child, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Deref for SupervisedChild {
    type Target = GroupChild;

    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl DerefMut for SupervisedChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

impl Drop for SupervisedChild {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Cooperative cancellation shared between a caller and the supervisor.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    state: Arc<AtomicU8>,
}

impl CancellationToken {
    /// Create an unset token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Repeated calls are harmless.
    pub fn cancel(&self) {
        self.state.fetch_max(1, Ordering::AcqRel);
    }

    /// Mark cancellation as originating from an OS termination signal.
    ///
    /// A signalled dispatcher must hard-stop its own child group immediately,
    /// leaving time for its parent supervisor's graceful termination window.
    pub(crate) fn cancel_from_signal(&self) {
        self.state.store(2, Ordering::Release);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) != 0
    }

    fn requires_immediate_kill(&self) -> bool {
        self.state.load(Ordering::Acquire) >= 2
    }
}

/// Hard transport limits for one plugin process.
#[derive(Clone, Debug)]
pub struct ProcessLimits {
    /// Overall wall-clock deadline. `None` deliberately disables it.
    pub deadline: Option<Duration>,
    /// Maximum encoded request size.
    pub max_request_bytes: usize,
    /// Maximum bytes in one stdout JSONL frame.
    pub max_frame_bytes: usize,
    /// Maximum total stdout bytes drained for this invocation.
    pub max_stdout_bytes: usize,
    /// Maximum accepted stdout frames, including the terminal result.
    pub max_frames: u64,
    /// Maximum stderr bytes retained; excess bytes are drained and discarded.
    pub max_stderr_bytes: usize,
    /// Bounded number of decoded frames waiting for the callback.
    pub event_queue_bound: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            deadline: Some(Duration::from_secs(30 * 60)),
            max_request_bytes: 1024 * 1024,
            max_frame_bytes: 1024 * 1024,
            max_stdout_bytes: 64 * 1024 * 1024,
            max_frames: 10_000,
            max_stderr_bytes: 1024 * 1024,
            event_queue_bound: 128,
        }
    }
}

/// Portable command specification for one plugin invocation.
#[derive(Clone, Debug)]
pub struct ProcessSpec {
    /// Executable followed by arguments.
    pub command: Vec<String>,
    /// Child working directory.
    pub cwd: PathBuf,
    /// Environment entries added to the inherited environment.
    pub environment: BTreeMap<String, String>,
    /// Transport bounds.
    pub limits: ProcessLimits,
}

impl ProcessSpec {
    /// Create a specification with inherited environment and safe defaults.
    #[must_use]
    pub fn new(command: Vec<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            command,
            cwd: cwd.into(),
            environment: BTreeMap::new(),
            limits: ProcessLimits::default(),
        }
    }
}

/// Normalized terminal state of an invocation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationKind {
    /// Exit code zero and a successful terminal result.
    Succeeded,
    /// A valid terminal plugin failure.
    PluginFailed,
    /// The process exit status contradicted a successful terminal result.
    ProcessFailed,
    /// Stdout violated the JSONL contract.
    ProtocolFailed,
    /// A pipe or worker failed after the child was spawned.
    RuntimeFailed,
    /// The configured wall-clock deadline elapsed.
    DeadlineExceeded,
    /// The caller requested cancellation.
    Cancelled,
}

/// Complete factual result of supervising one process group.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProcessOutcome {
    /// Normalized terminal state.
    pub kind: InvocationKind,
    /// Leader exit code when the operating system supplies one.
    pub exit_code: Option<i32>,
    /// Valid terminal plugin value, if one arrived before termination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalResult>,
    /// Number of valid frames accepted in order.
    pub frames_seen: u64,
    /// Bounded, lossy-decoded diagnostic output.
    pub stderr: String,
    /// Whether additional stderr bytes were discarded.
    pub stderr_truncated: bool,
    /// Protocol or runtime reason for a non-domain failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Elapsed wall-clock milliseconds.
    pub elapsed_ms: u64,
}

/// Terminal state for a supervised non-plugin command such as GHC or Cabal.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// The command exited on its own.
    Exited,
    /// The wall-clock deadline elapsed.
    DeadlineExceeded,
    /// The caller requested cancellation.
    Cancelled,
    /// Waiting for or reaping the child failed.
    RuntimeFailed,
}

/// Factual result of a process-group command with inherited console streams.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CommandOutcome {
    /// Normalized terminal state.
    pub kind: CommandKind,
    /// Leader exit code when available.
    pub exit_code: Option<i32>,
    /// Runtime diagnostic when supervision failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Elapsed wall-clock milliseconds.
    pub elapsed_ms: u64,
}

impl CommandOutcome {
    /// True only for a normal zero exit code.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.kind == CommandKind::Exited && self.exit_code == Some(0)
    }
}

impl ProcessOutcome {
    /// True only for an ordinary successful invocation.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.kind == InvocationKind::Succeeded
    }
}

/// Failure to construct or start supervision. Post-spawn failures are outcomes.
#[derive(Debug, Error)]
pub enum ProcessError {
    /// A command or limit was invalid.
    #[error("invalid process specification: {0}")]
    InvalidSpec(String),
    /// The request could not be encoded.
    #[error("cannot encode plugin request: {0}")]
    Encode(#[from] serde_json::Error),
    /// The contained child could not be started.
    #[error("cannot spawn plugin process: {0}")]
    Spawn(#[source] io::Error),
    /// A standard pipe was unexpectedly unavailable.
    #[error("plugin {0} pipe was unavailable")]
    MissingPipe(&'static str),
}

/// Synchronous supervisor which streams events while monitoring a process group.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessSupervisor;

impl ProcessSupervisor {
    /// Run a non-plugin command in a contained process group with inherited I/O.
    pub fn run_command(
        &self,
        spec: &ProcessSpec,
        cancellation: &CancellationToken,
    ) -> Result<CommandOutcome, ProcessError> {
        validate_spec(spec)?;
        let mut command = Command::new(&spec.command[0]);
        command
            .args(&spec.command[1..])
            .current_dir(&spec.cwd)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .envs(&spec.environment);
        let mut child = SupervisedChild::new(command.group_spawn().map_err(ProcessError::Spawn)?);
        let started = Instant::now();
        loop {
            if cancellation.is_cancelled() {
                let outcome = command_termination(&mut child, CommandKind::Cancelled, started);
                if outcome.kind != CommandKind::RuntimeFailed {
                    child.disarm();
                }
                return Ok(outcome);
            }
            if spec
                .limits
                .deadline
                .is_some_and(|deadline| started.elapsed() >= deadline)
            {
                let outcome =
                    command_termination(&mut child, CommandKind::DeadlineExceeded, started);
                if outcome.kind != CommandKind::RuntimeFailed {
                    child.disarm();
                }
                return Ok(outcome);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let cleanup = cleanup_owned_group(&mut child);
                    if cleanup.is_ok() {
                        child.disarm();
                    }
                    return Ok(match cleanup {
                        Ok(()) => CommandOutcome {
                            kind: CommandKind::Exited,
                            exit_code: status.code(),
                            error: None,
                            elapsed_ms: elapsed_millis(started.elapsed()),
                        },
                        Err(error) => CommandOutcome {
                            kind: CommandKind::RuntimeFailed,
                            exit_code: status.code(),
                            error: Some(format!(
                                "cannot clean the completed command process group: {error}"
                            )),
                            elapsed_ms: elapsed_millis(started.elapsed()),
                        },
                    });
                }
                Ok(None) => thread::sleep(POLL_INTERVAL),
                Err(error) => {
                    let mut message = format!("cannot wait for command: {error}");
                    let (exit_code, reaped) = match kill_and_wait(&mut child) {
                        Ok(status) => (status.code(), true),
                        Err(kill_error) => {
                            message.push_str(&format!("; cannot reap it: {kill_error}"));
                            (None, false)
                        }
                    };
                    if reaped {
                        child.disarm();
                    }
                    return Ok(CommandOutcome {
                        kind: CommandKind::RuntimeFailed,
                        exit_code,
                        error: Some(message),
                        elapsed_ms: elapsed_millis(started.elapsed()),
                    });
                }
            }
        }
    }

    /// Invoke one plugin, delivering validated frames through a bounded worker.
    ///
    /// The plugin process is placed in a Unix process group or Windows Job Object
    /// by `command-group`, so deadline and cancellation terminate descendants too.
    pub fn invoke<F>(
        &self,
        spec: &ProcessSpec,
        request: &PluginRequest,
        cancellation: &CancellationToken,
        mut on_frame: F,
    ) -> Result<ProcessOutcome, ProcessError>
    where
        F: FnMut(&PluginFrame) + Send + 'static,
    {
        validate_spec(spec)?;
        request
            .validate()
            .map_err(|error| ProcessError::InvalidSpec(error.to_string()))?;
        let mut input = serde_json::to_vec(request)?;
        input.push(b'\n');
        if input.len() > spec.limits.max_request_bytes {
            return Err(ProcessError::InvalidSpec(format!(
                "encoded request is {} bytes; limit is {}",
                input.len(),
                spec.limits.max_request_bytes
            )));
        }

        let mut command = Command::new(&spec.command[0]);
        command
            .args(&spec.command[1..])
            .current_dir(&spec.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .envs(&spec.environment);
        let mut child = SupervisedChild::new(command.group_spawn().map_err(ProcessError::Spawn)?);
        let stdin = take_pipe(&mut child, Pipe::Stdin)?;
        let stdout = take_pipe(&mut child, Pipe::Stdout)?;
        let stderr = take_pipe(&mut child, Pipe::Stderr)?;

        let (sender, receiver) = mpsc::sync_channel(spec.limits.event_queue_bound);
        let max_frame_bytes = spec.limits.max_frame_bytes;
        let max_stdout_bytes = spec.limits.max_stdout_bytes;
        let stdout_worker = thread::spawn(move || {
            read_stdout(stdout, sender, max_frame_bytes, max_stdout_bytes);
        });
        let max_stderr_bytes = spec.limits.max_stderr_bytes;
        let stderr_worker = thread::spawn(move || capture_stderr(stderr, max_stderr_bytes));
        let stdin_worker = thread::spawn(move || write_stdin(stdin, &input));
        let (callback_sender, callback_receiver) =
            mpsc::sync_channel::<PluginFrame>(spec.limits.event_queue_bound);
        let (callback_done_sender, callback_done_receiver) =
            mpsc::sync_channel::<Result<(), String>>(1);
        let callback_worker = thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                while let Ok(frame) = callback_receiver.recv() {
                    on_frame(&frame);
                }
            }))
            .map_err(|_| "plugin frame callback panicked".to_owned());
            let _ = callback_done_sender.send(result);
        });

        let started = Instant::now();
        let mut sequence = FrameSequence::new(request.id.clone());
        let mut status: Option<ExitStatus> = None;
        let mut child_done = false;
        let mut stdout_done = false;
        let mut termination_attempted = false;
        let mut forced: Option<InvocationKind> = None;
        let mut failure: Option<String> = None;

        while !child_done || !stdout_done {
            // A process-group leader may exit while one of its descendants
            // still owns the inherited stdout/stderr pipes. Cancellation and
            // deadlines therefore govern the whole supervised I/O lifetime,
            // not only the leader's lifetime.
            if forced.is_none() {
                if cancellation.is_cancelled() {
                    forced = Some(InvocationKind::Cancelled);
                    failure = Some("plugin invocation was cancelled".to_owned());
                } else if spec
                    .limits
                    .deadline
                    .is_some_and(|deadline| started.elapsed() >= deadline)
                {
                    forced = Some(InvocationKind::DeadlineExceeded);
                    failure = Some("plugin invocation exceeded its deadline".to_owned());
                }
            }

            match receive_frame(&receiver) {
                Receive::Frame(frame) => {
                    if sequence.frames_seen() >= spec.limits.max_frames {
                        forced = Some(InvocationKind::ProtocolFailed);
                        failure = Some(format!(
                            "plugin emitted more than {} frames",
                            spec.limits.max_frames
                        ));
                        stdout_done = true;
                    } else if let Err(error) = sequence.accept(&frame) {
                        forced = Some(InvocationKind::ProtocolFailed);
                        failure = Some(error.to_string());
                        stdout_done = true;
                    } else {
                        match callback_sender.try_send(frame) {
                            Ok(()) => {}
                            Err(mpsc::TrySendError::Full(_)) => {
                                forced = Some(InvocationKind::RuntimeFailed);
                                failure = Some(
                                    "plugin frame callback queue exceeded its bounded capacity"
                                        .to_owned(),
                                );
                            }
                            Err(mpsc::TrySendError::Disconnected(_)) => {
                                forced = Some(InvocationKind::RuntimeFailed);
                                failure =
                                    Some("plugin frame callback stopped unexpectedly".to_owned());
                            }
                        }
                    }
                }
                Receive::Fault(error) => {
                    forced = Some(InvocationKind::ProtocolFailed);
                    failure = Some(error.to_string());
                    stdout_done = true;
                }
                Receive::Runtime(error) => {
                    forced = Some(InvocationKind::RuntimeFailed);
                    failure = Some(error);
                    stdout_done = true;
                }
                Receive::End | Receive::Disconnected => stdout_done = true,
                Receive::Pending => {}
            }

            if !child_done && forced.is_none() {
                match child.try_wait() {
                    Ok(Some(value)) => {
                        status = Some(value);
                        child_done = true;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        forced = Some(InvocationKind::RuntimeFailed);
                        failure = Some(format!("cannot wait for plugin process: {error}"));
                    }
                }
            }
            // A protocol failure can also be observed after the leader has
            // exited. Terminate the group exactly once so a pipe-holding
            // descendant cannot strand either reader worker.
            if forced.is_some() && !termination_attempted {
                termination_attempted = true;
                let termination = if forced == Some(InvocationKind::Cancelled)
                    && cancellation.requires_immediate_kill()
                {
                    hard_kill_and_wait(&mut child)
                } else {
                    kill_and_wait(&mut child)
                };
                match termination {
                    Ok(value) => status = Some(value),
                    Err(error) => {
                        forced = Some(InvocationKind::RuntimeFailed);
                        failure = Some(format!("cannot reap failed plugin: {error}"));
                    }
                }
                child_done = true;
            }
            if forced.is_some() && termination_attempted {
                break;
            }
        }

        // Successful process leaders are not allowed to leave background work
        // behind. This also closes the same-process-group containment gap used
        // by built-in provider hosts on Unix.
        match cleanup_owned_group(&mut child) {
            Ok(()) => child.disarm(),
            Err(error) if forced.is_none() => {
                forced = Some(InvocationKind::RuntimeFailed);
                failure = Some(format!(
                    "cannot clean the completed plugin process group: {error}"
                ));
            }
            Err(_) => {}
        }

        // Fault paths intentionally stop consuming frames. Dropping the bounded
        // receiver releases a producer that may be waiting on backpressure.
        drop(receiver);
        drop(callback_sender);
        let stdin_result = finish_worker(stdin_worker, PIPE_WORKER_DRAIN_TIMEOUT);
        let stdout_result = finish_worker(stdout_worker, PIPE_WORKER_DRAIN_TIMEOUT);
        let stderr_result = finish_worker(stderr_worker, PIPE_WORKER_DRAIN_TIMEOUT);
        match stdout_result {
            WorkerFinish::Completed(()) => {}
            WorkerFinish::Panicked if forced.is_none() => {
                forced = Some(InvocationKind::RuntimeFailed);
                failure = Some("stdout worker panicked".to_owned());
            }
            WorkerFinish::Stalled if forced.is_none() => {
                forced = Some(InvocationKind::RuntimeFailed);
                failure = Some("stdout pipe remained open after process completion".to_owned());
            }
            WorkerFinish::Panicked | WorkerFinish::Stalled => {}
        }
        match stdin_result {
            WorkerFinish::Completed(Ok(())) => {}
            WorkerFinish::Completed(Err(error)) if forced.is_none() => {
                forced = Some(InvocationKind::RuntimeFailed);
                failure = Some(error);
            }
            WorkerFinish::Panicked if forced.is_none() => {
                forced = Some(InvocationKind::RuntimeFailed);
                failure = Some("stdin worker panicked".to_owned());
            }
            WorkerFinish::Stalled if forced.is_none() => {
                forced = Some(InvocationKind::RuntimeFailed);
                failure = Some("stdin pipe remained open after process completion".to_owned());
            }
            WorkerFinish::Completed(Err(_)) | WorkerFinish::Panicked | WorkerFinish::Stalled => {}
        }
        let captured = match stderr_result {
            WorkerFinish::Completed(Ok(value)) => value,
            WorkerFinish::Completed(Err(error)) => {
                if forced.is_none() {
                    forced = Some(InvocationKind::RuntimeFailed);
                    failure = Some(error);
                }
                CapturedStderr::default()
            }
            WorkerFinish::Panicked => {
                if forced.is_none() {
                    forced = Some(InvocationKind::RuntimeFailed);
                    failure = Some("stderr worker panicked".to_owned());
                }
                CapturedStderr::default()
            }
            WorkerFinish::Stalled => {
                if forced.is_none() {
                    forced = Some(InvocationKind::RuntimeFailed);
                    failure = Some("stderr pipe remained open after process completion".to_owned());
                }
                CapturedStderr::default()
            }
        };
        let mut callback_finished = false;
        if forced.is_none() {
            let callback_deadline = Instant::now() + CALLBACK_DRAIN_TIMEOUT;
            loop {
                match callback_done_receiver.recv_timeout(POLL_INTERVAL) {
                    Ok(Ok(())) => {
                        callback_finished = true;
                        break;
                    }
                    Ok(Err(error)) => {
                        callback_finished = true;
                        forced = Some(InvocationKind::RuntimeFailed);
                        failure = Some(error);
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        callback_finished = true;
                        forced = Some(InvocationKind::RuntimeFailed);
                        failure = Some("plugin frame callback stopped unexpectedly".to_owned());
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if cancellation.is_cancelled() {
                            forced = Some(InvocationKind::Cancelled);
                            failure = Some(
                                "plugin invocation was cancelled while delivering frames"
                                    .to_owned(),
                            );
                            break;
                        }
                        if spec
                            .limits
                            .deadline
                            .is_some_and(|deadline| started.elapsed() >= deadline)
                        {
                            forced = Some(InvocationKind::DeadlineExceeded);
                            failure = Some(
                                "plugin frame callback exceeded the invocation deadline".to_owned(),
                            );
                            break;
                        }
                        if Instant::now() >= callback_deadline {
                            forced = Some(InvocationKind::RuntimeFailed);
                            failure = Some(
                                "plugin frame callback exceeded its delivery deadline".to_owned(),
                            );
                            break;
                        }
                    }
                }
            }
        }
        if callback_finished || callback_worker.is_finished() {
            let _ = callback_worker.join();
        }

        let frames_seen = sequence.frames_seen();
        let terminal = match sequence.finish() {
            Ok(value) => Some(value),
            Err(error) => {
                if forced.is_none() {
                    forced = Some(InvocationKind::ProtocolFailed);
                    failure = Some(error.to_string());
                }
                None
            }
        };
        let status_success = status.as_ref().is_some_and(ExitStatus::success);
        let kind = forced.unwrap_or(match (&terminal, status_success) {
            (Some(TerminalResult::Success { .. }), true) => InvocationKind::Succeeded,
            (Some(TerminalResult::Failure { .. }), _) => InvocationKind::PluginFailed,
            (Some(TerminalResult::Success { .. }), false) => InvocationKind::ProcessFailed,
            (None, _) => InvocationKind::ProtocolFailed,
        });
        if kind == InvocationKind::ProcessFailed && failure.is_none() {
            failure = Some(format!(
                "plugin returned success but process exited with {:?}",
                status.as_ref().and_then(ExitStatus::code)
            ));
        }
        Ok(ProcessOutcome {
            kind,
            exit_code: status.as_ref().and_then(ExitStatus::code),
            terminal,
            frames_seen,
            stderr: String::from_utf8_lossy(&captured.bytes).into_owned(),
            stderr_truncated: captured.truncated,
            error: failure,
            elapsed_ms: elapsed_millis(started.elapsed()),
        })
    }
}

fn command_termination(
    child: &mut GroupChild,
    requested: CommandKind,
    started: Instant,
) -> CommandOutcome {
    match kill_and_wait(child) {
        Ok(status) => CommandOutcome {
            kind: requested,
            exit_code: status.code(),
            error: None,
            elapsed_ms: elapsed_millis(started.elapsed()),
        },
        Err(error) => CommandOutcome {
            kind: CommandKind::RuntimeFailed,
            exit_code: None,
            error: Some(format!("cannot reap terminated command: {error}")),
            elapsed_ms: elapsed_millis(started.elapsed()),
        },
    }
}

fn validate_spec(spec: &ProcessSpec) -> Result<(), ProcessError> {
    if spec.command.first().is_none_or(String::is_empty) {
        return Err(ProcessError::InvalidSpec(
            "command must contain a non-empty executable".to_owned(),
        ));
    }
    if !spec.cwd.is_dir() {
        return Err(ProcessError::InvalidSpec(format!(
            "working directory does not exist: {}",
            spec.cwd.display()
        )));
    }
    if spec.limits.max_request_bytes == 0
        || spec.limits.max_frame_bytes == 0
        || spec.limits.max_stdout_bytes == 0
        || spec.limits.max_frames == 0
        || spec.limits.event_queue_bound == 0
    {
        return Err(ProcessError::InvalidSpec(
            "request, frame, event, and queue limits must be positive".to_owned(),
        ));
    }
    Ok(())
}

enum PipeValue {
    Stdin(std::process::ChildStdin),
    Stdout(std::process::ChildStdout),
    Stderr(std::process::ChildStderr),
}

enum Pipe {
    Stdin,
    Stdout,
    Stderr,
}

fn take_pipe(child: &mut GroupChild, pipe: Pipe) -> Result<PipeValue, ProcessError> {
    match pipe {
        Pipe::Stdin => child
            .inner()
            .stdin
            .take()
            .map(PipeValue::Stdin)
            .ok_or(ProcessError::MissingPipe("stdin")),
        Pipe::Stdout => child
            .inner()
            .stdout
            .take()
            .map(PipeValue::Stdout)
            .ok_or(ProcessError::MissingPipe("stdout")),
        Pipe::Stderr => child
            .inner()
            .stderr
            .take()
            .map(PipeValue::Stderr)
            .ok_or(ProcessError::MissingPipe("stderr")),
    }
}

impl PipeValue {
    fn into_stdin(self) -> std::process::ChildStdin {
        match self {
            Self::Stdin(value) => value,
            Self::Stdout(_) | Self::Stderr(_) => unreachable!("pipe type checked by caller"),
        }
    }

    fn into_stdout(self) -> std::process::ChildStdout {
        match self {
            Self::Stdout(value) => value,
            Self::Stdin(_) | Self::Stderr(_) => unreachable!("pipe type checked by caller"),
        }
    }

    fn into_stderr(self) -> std::process::ChildStderr {
        match self {
            Self::Stderr(value) => value,
            Self::Stdin(_) | Self::Stdout(_) => unreachable!("pipe type checked by caller"),
        }
    }
}

enum ReaderMessage {
    Frame(PluginFrame),
    Fault(ProtocolFault),
    Runtime(String),
    End,
}

enum Receive {
    Frame(PluginFrame),
    Fault(ProtocolFault),
    Runtime(String),
    End,
    Pending,
    Disconnected,
}

fn receive_frame(receiver: &Receiver<ReaderMessage>) -> Receive {
    match receiver.recv_timeout(POLL_INTERVAL) {
        Ok(ReaderMessage::Frame(frame)) => Receive::Frame(frame),
        Ok(ReaderMessage::Fault(error)) => Receive::Fault(error),
        Ok(ReaderMessage::Runtime(error)) => Receive::Runtime(error),
        Ok(ReaderMessage::End) => Receive::End,
        Err(mpsc::RecvTimeoutError::Timeout) => Receive::Pending,
        Err(mpsc::RecvTimeoutError::Disconnected) => Receive::Disconnected,
    }
}

fn read_stdout(
    stdout: PipeValue,
    sender: SyncSender<ReaderMessage>,
    max_frame_bytes: usize,
    max_stdout_bytes: usize,
) {
    let mut stdout = stdout.into_stdout();
    let mut chunk = [0_u8; 8192];
    let mut line = Vec::new();
    let mut total_bytes = 0_usize;
    loop {
        let count = match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) => {
                let _ = sender.send(ReaderMessage::Runtime(format!(
                    "cannot read plugin stdout: {error}"
                )));
                return;
            }
        };
        total_bytes = match total_bytes.checked_add(count) {
            Some(total) if total <= max_stdout_bytes => total,
            _ => {
                let _ = sender.send(ReaderMessage::Fault(ProtocolFault::InvalidJson(format!(
                    "plugin stdout exceeds {max_stdout_bytes} bytes"
                ))));
                return;
            }
        };
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if !send_line(&sender, &line) {
                    return;
                }
                line.clear();
            } else if line.len() >= max_frame_bytes {
                let _ = sender.send(ReaderMessage::Fault(ProtocolFault::InvalidJson(format!(
                    "JSONL frame exceeds {max_frame_bytes} bytes"
                ))));
                return;
            } else {
                line.push(*byte);
            }
        }
    }
    if !line.is_empty() && !send_line(&sender, &line) {
        return;
    }
    let _ = sender.send(ReaderMessage::End);
}

fn send_line(sender: &SyncSender<ReaderMessage>, line: &[u8]) -> bool {
    match decode_frame(line) {
        Ok(frame) => sender.send(ReaderMessage::Frame(frame)).is_ok(),
        Err(error) => {
            let _ = sender.send(ReaderMessage::Fault(error));
            false
        }
    }
}

#[derive(Default)]
struct CapturedStderr {
    bytes: Vec<u8>,
    truncated: bool,
}

fn capture_stderr(stderr: PipeValue, limit: usize) -> Result<CapturedStderr, String> {
    let mut stderr = stderr.into_stderr();
    let mut captured = CapturedStderr::default();
    let mut chunk = [0_u8; 8192];
    loop {
        let count = stderr
            .read(&mut chunk)
            .map_err(|error| format!("cannot read plugin stderr: {error}"))?;
        if count == 0 {
            break;
        }
        let available = limit.saturating_sub(captured.bytes.len());
        let retained = available.min(count);
        captured.bytes.extend_from_slice(&chunk[..retained]);
        captured.truncated |= retained < count;
    }
    Ok(captured)
}

fn write_stdin(stdin: PipeValue, input: &[u8]) -> Result<(), String> {
    let mut stdin = stdin.into_stdin();
    stdin
        .write_all(input)
        .and_then(|()| stdin.flush())
        .map_err(|error| format!("cannot write plugin stdin: {error}"))
}

fn kill_and_wait(child: &mut GroupChild) -> io::Result<ExitStatus> {
    #[cfg(unix)]
    {
        use command_group::{Signal, UnixChildExt};

        let _ = child.signal(Signal::SIGTERM);
        let grace_deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < grace_deadline {
            if child.try_wait()?.is_some() {
                break;
            }
            thread::sleep(
                POLL_INTERVAL.min(grace_deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    hard_kill_and_wait(child)
}

fn hard_kill_and_wait(child: &mut GroupChild) -> io::Result<ExitStatus> {
    if let Err(error) = child.kill()
        && !process_is_absent(&error)
        && child.try_wait()?.is_none()
    {
        return Err(error);
    }
    child.wait()
}

enum WorkerFinish<T> {
    Completed(T),
    Panicked,
    Stalled,
}

fn finish_worker<T>(worker: thread::JoinHandle<T>, timeout: Duration) -> WorkerFinish<T> {
    let deadline = Instant::now() + timeout;
    while !worker.is_finished() && Instant::now() < deadline {
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
    if !worker.is_finished() {
        return WorkerFinish::Stalled;
    }
    match worker.join() {
        Ok(value) => WorkerFinish::Completed(value),
        Err(_) => WorkerFinish::Panicked,
    }
}

fn cleanup_owned_group(child: &mut GroupChild) -> io::Result<()> {
    match child.kill() {
        Ok(()) => Ok(()),
        Err(error) if process_is_absent(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

fn process_is_absent(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::NotFound
    ) {
        return true;
    }
    #[cfg(unix)]
    if error.raw_os_error() == Some(3) {
        return true;
    }
    false
}

fn elapsed_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
