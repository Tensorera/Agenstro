use std::{
    fs,
    io::{self, Read},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use command_group::{CommandGroup, GroupChild};
use thiserror::Error;

use crate::{CancellationToken, ProcessSpec};

const POLL_INTERVAL: Duration = Duration::from_millis(5);
const CAPTURE_NONE: u8 = 0;
const CAPTURE_STDOUT: u8 = 1;
const CAPTURE_STDERR: u8 = 2;
const CAPTURE_COMBINED: u8 = 3;

/// Native process-tree containment provided by the current platform.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ContainmentKind {
    /// A Windows Job Object with kill-on-close and suspended assignment.
    WindowsJobObject,
    /// A Linux process group. Hard cgroup resource controls are not enabled.
    LinuxProcessGroup,
    /// The target does not have an implemented containment adapter.
    Unsupported,
}

/// Portable graceful-termination capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GracefulTermination {
    /// `SIGTERM` is sent to the entire process group before force termination.
    ProcessGroupSignal,
    /// No safe generic soft signal exists; cancellation proceeds to Job kill.
    Unavailable,
}

/// Capabilities actually enforced by a process supervisor implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessCapabilities {
    /// Native tree-containment mechanism.
    pub containment: ContainmentKind,
    /// Graceful-termination mechanism.
    pub graceful_termination: GracefulTermination,
    /// Whether a requested memory limit is enforced.
    pub hard_memory_limit: bool,
    /// Whether a requested process-count limit is enforced.
    pub hard_process_limit: bool,
    /// Whether a requested CPU-time limit is enforced.
    pub hard_cpu_limit: bool,
}

/// Output stream which crossed a hard byte limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
    /// Aggregate standard output and standard error.
    Combined,
}

/// Why a supervised operation reached terminal state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TerminationReason {
    /// The process leader exited before another terminal condition was observed.
    Exited,
    /// The caller requested cancellation.
    Cancelled,
    /// The overall process deadline elapsed.
    DeadlineExceeded,
    /// A stream or aggregate output budget was exceeded.
    OutputLimitExceeded(OutputStream),
}

/// Bounded terminal output and normalized process status.
pub struct ProcessOutput {
    termination: TerminationReason,
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ProcessOutput {
    /// Returns the terminal reason observed by the supervisor.
    #[must_use]
    pub fn termination(&self) -> TerminationReason {
        self.termination
    }

    /// Returns the process leader's portable exit code, when available.
    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        self.status.as_ref().and_then(ExitStatus::code)
    }

    /// Reports a normal, successful leader exit.
    #[must_use]
    pub fn success(&self) -> bool {
        self.termination == TerminationReason::Exited
            && self.status.as_ref().is_some_and(ExitStatus::success)
    }

    /// Returns the bounded standard-output bytes.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns the bounded standard-error bytes.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

/// Infrastructure failure while supervising a process tree.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProcessError {
    /// The current platform has no implemented tree containment.
    #[error("process containment is unsupported on this platform")]
    UnsupportedPlatform,
    /// A requested hard limit cannot be enforced by this implementation.
    #[error("the native supervisor cannot enforce the requested {resource} limit")]
    UnsupportedResourceLimit {
        /// Resource whose hard limit was requested.
        resource: &'static str,
    },
    /// The executable was missing or ceased to be a regular file.
    #[error("the resolved executable is unavailable")]
    ExecutableUnavailable {
        /// Underlying filesystem error.
        #[source]
        source: io::Error,
    },
    /// The working directory was missing or ceased to be a directory.
    #[error("the working directory is unavailable")]
    WorkingDirectoryUnavailable {
        /// Underlying filesystem error.
        #[source]
        source: io::Error,
    },
    /// The child could not be spawned and contained.
    #[error("failed to spawn the contained process")]
    Spawn {
        /// Underlying operating-system error.
        #[source]
        source: io::Error,
    },
    /// A requested child pipe was unexpectedly absent.
    #[error("the child {stream} pipe was unavailable")]
    PipeUnavailable {
        /// Pipe which was absent.
        stream: &'static str,
    },
    /// An output-draining thread could not be started.
    #[error("failed to start the {stream} drain")]
    ReaderSpawn {
        /// Output stream whose reader failed.
        stream: &'static str,
        /// Underlying thread creation error.
        #[source]
        source: io::Error,
    },
    /// Reading an output pipe failed.
    #[error("failed while draining {stream}")]
    Reader {
        /// Output stream whose read failed.
        stream: &'static str,
        /// Underlying pipe error.
        #[source]
        source: io::Error,
    },
    /// An output-draining thread panicked.
    #[error("the {stream} drain panicked")]
    ReaderPanicked {
        /// Output stream whose reader panicked.
        stream: &'static str,
    },
    /// Polling or reaping the process group failed.
    #[error("failed while waiting for the contained process")]
    Wait {
        /// Underlying operating-system error.
        #[source]
        source: io::Error,
    },
    /// Graceful or forceful tree termination failed.
    #[error("failed to terminate the contained process tree")]
    Terminate {
        /// Underlying operating-system error.
        #[source]
        source: io::Error,
    },
}

/// Synchronous port for one fully owned process operation.
///
/// Implementations must return only after output drains and process-tree
/// cleanup are complete. Callers should invoke this blocking port from a
/// dedicated thread rather than an async runtime worker.
pub trait ProcessSupervisor: Send + Sync {
    /// Reports the containment and resource controls actually enforced.
    fn capabilities(&self) -> ProcessCapabilities;

    /// Runs one argv-based process under a hard deadline and output budgets.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError`] for setup, containment, pipe, wait, or cleanup
    /// failures. Process exit failure, timeout, cancellation, and output-limit
    /// termination are represented in [`ProcessOutput`].
    fn run(
        &self,
        spec: ProcessSpec,
        cancellation: &CancellationToken,
    ) -> Result<ProcessOutput, ProcessError>;
}

/// Native Windows Job Object or Linux process-group supervisor.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeProcessSupervisor;

impl NativeProcessSupervisor {
    /// Constructs the stateless native supervisor.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ProcessSupervisor for NativeProcessSupervisor {
    fn capabilities(&self) -> ProcessCapabilities {
        native_capabilities()
    }

    fn run(
        &self,
        spec: ProcessSpec,
        cancellation: &CancellationToken,
    ) -> Result<ProcessOutput, ProcessError> {
        if self.capabilities().containment == ContainmentKind::Unsupported {
            return Err(ProcessError::UnsupportedPlatform);
        }
        if let Some(resource) = spec.resource_limits().first_requested() {
            return Err(ProcessError::UnsupportedResourceLimit { resource });
        }
        validate_spawn_paths(&spec)?;

        let mut command = Command::new(spec.executable());
        command
            .args(spec.arguments())
            .current_dir(spec.working_directory())
            .env_clear()
            .envs(spec.environment())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut group_builder = command.group();
        #[cfg(windows)]
        group_builder.kill_on_drop(true);
        let child = group_builder
            .spawn()
            .map_err(|source| ProcessError::Spawn { source })?;
        supervise_child(child, &spec, cancellation, self.capabilities())
    }
}

fn native_capabilities() -> ProcessCapabilities {
    #[cfg(windows)]
    {
        ProcessCapabilities {
            containment: ContainmentKind::WindowsJobObject,
            graceful_termination: GracefulTermination::Unavailable,
            hard_memory_limit: false,
            hard_process_limit: false,
            hard_cpu_limit: false,
        }
    }
    #[cfg(target_os = "linux")]
    {
        ProcessCapabilities {
            containment: ContainmentKind::LinuxProcessGroup,
            graceful_termination: GracefulTermination::ProcessGroupSignal,
            hard_memory_limit: false,
            hard_process_limit: false,
            hard_cpu_limit: false,
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        ProcessCapabilities {
            containment: ContainmentKind::Unsupported,
            graceful_termination: GracefulTermination::Unavailable,
            hard_memory_limit: false,
            hard_process_limit: false,
            hard_cpu_limit: false,
        }
    }
}

fn validate_spawn_paths(spec: &ProcessSpec) -> Result<(), ProcessError> {
    let executable = fs::metadata(spec.executable())
        .map_err(|source| ProcessError::ExecutableUnavailable { source })?;
    if !executable.is_file() {
        return Err(ProcessError::ExecutableUnavailable {
            source: io::Error::new(io::ErrorKind::InvalidInput, "not a regular file"),
        });
    }
    let working_directory = fs::metadata(spec.working_directory())
        .map_err(|source| ProcessError::WorkingDirectoryUnavailable { source })?;
    if !working_directory.is_dir() {
        return Err(ProcessError::WorkingDirectoryUnavailable {
            source: io::Error::new(io::ErrorKind::InvalidInput, "not a directory"),
        });
    }
    Ok(())
}

struct ChildGuard {
    child: Option<GroupChild>,
}

impl ChildGuard {
    fn new(child: GroupChild) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> Result<&mut GroupChild, ProcessError> {
        self.child.as_mut().ok_or_else(|| ProcessError::Wait {
            source: io::Error::other("process ownership was already released"),
        })
    }

    fn release(mut self) {
        self.child = None;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
    }
}

fn supervise_child(
    child: GroupChild,
    spec: &ProcessSpec,
    cancellation: &CancellationToken,
    capabilities: ProcessCapabilities,
) -> Result<ProcessOutput, ProcessError> {
    let mut guard = ChildGuard::new(child);
    let stdout = match guard.child_mut()?.inner().stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_and_reap(guard.child_mut()?)?;
            return Err(ProcessError::PipeUnavailable { stream: "stdout" });
        }
    };
    let stderr = match guard.child_mut()?.inner().stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_and_reap(guard.child_mut()?)?;
            return Err(ProcessError::PipeUnavailable { stream: "stderr" });
        }
    };

    let signal = Arc::new(CaptureSignal::new());
    let stdout_reader = match spawn_reader(
        "agentro-stdout-drain",
        "stdout",
        stdout,
        spec.output_budget().stdout_bytes(),
        spec.output_budget().total_bytes(),
        CAPTURE_STDOUT,
        Arc::clone(&signal),
    ) {
        Ok(reader) => reader,
        Err(error) => {
            terminate_and_reap(guard.child_mut()?)?;
            return Err(error);
        }
    };
    let stderr_reader = match spawn_reader(
        "agentro-stderr-drain",
        "stderr",
        stderr,
        spec.output_budget().stderr_bytes(),
        spec.output_budget().total_bytes(),
        CAPTURE_STDERR,
        Arc::clone(&signal),
    ) {
        Ok(reader) => reader,
        Err(error) => {
            terminate_and_reap(guard.child_mut()?)?;
            let _ = join_reader(stdout_reader, "stdout");
            return Err(error);
        }
    };

    let deadline = Instant::now() + spec.timeouts().overall();
    let (termination, mut status) =
        wait_for_terminal(guard.child_mut()?, cancellation, &signal, deadline)?;

    match termination {
        TerminationReason::Exited => {
            // A process leader may intentionally or accidentally leave a
            // descendant holding the pipes. A supervised run never adopts it.
            ignore_already_exited(force_kill(guard.child_mut()?))?;
            reap_after_force(guard.child_mut()?)?;
        }
        TerminationReason::Cancelled | TerminationReason::DeadlineExceeded => {
            status = terminate_for_reason(
                guard.child_mut()?,
                status,
                spec.timeouts().termination_grace(),
                capabilities,
            )?;
        }
        TerminationReason::OutputLimitExceeded(_) => {
            if status.is_none() {
                ignore_already_exited(force_kill(guard.child_mut()?))?;
                status = Some(
                    guard
                        .child_mut()?
                        .wait()
                        .map_err(|source| ProcessError::Wait { source })?,
                );
            }
        }
    }

    let stdout = join_reader(stdout_reader, "stdout")?;
    let stderr = join_reader(stderr_reader, "stderr")?;
    guard.release();
    Ok(ProcessOutput {
        termination,
        status,
        stdout,
        stderr,
    })
}

fn wait_for_terminal(
    child: &mut GroupChild,
    cancellation: &CancellationToken,
    signal: &CaptureSignal,
    deadline: Instant,
) -> Result<(TerminationReason, Option<ExitStatus>), ProcessError> {
    loop {
        if let Some(stream) = signal.overflow_stream() {
            return Ok((TerminationReason::OutputLimitExceeded(stream), None));
        }
        if cancellation.is_cancelled() {
            return Ok((TerminationReason::Cancelled, None));
        }
        if Instant::now() >= deadline {
            return Ok((TerminationReason::DeadlineExceeded, None));
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|source| ProcessError::Wait { source })?
        {
            return Ok((TerminationReason::Exited, Some(status)));
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

fn terminate_for_reason(
    child: &mut GroupChild,
    mut status: Option<ExitStatus>,
    grace: Duration,
    capabilities: ProcessCapabilities,
) -> Result<Option<ExitStatus>, ProcessError> {
    if status.is_none()
        && capabilities.graceful_termination == GracefulTermination::ProcessGroupSignal
    {
        send_graceful(child)?;
        let grace_deadline = Instant::now() + grace;
        while Instant::now() < grace_deadline {
            if let Some(exit) = child
                .try_wait()
                .map_err(|source| ProcessError::Wait { source })?
            {
                status = Some(exit);
                break;
            }
            thread::sleep(
                POLL_INTERVAL.min(grace_deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    // The leader may exit during grace while a descendant ignores SIGTERM and
    // keeps an output pipe open. Always terminate the owned group before
    // joining drain threads; waiting only for the leader can otherwise hang.
    ignore_already_exited(force_kill(child))?;
    if status.is_none() {
        status = Some(
            child
                .wait()
                .map_err(|source| ProcessError::Wait { source })?,
        );
    }
    Ok(status)
}

#[cfg(unix)]
fn send_graceful(child: &GroupChild) -> Result<(), ProcessError> {
    use command_group::{Signal, UnixChildExt};
    ignore_already_exited(
        child
            .signal(Signal::SIGTERM)
            .map_err(|source| ProcessError::Terminate { source }),
    )
}

#[cfg(not(unix))]
fn send_graceful(_child: &GroupChild) -> Result<(), ProcessError> {
    Ok(())
}

fn force_kill(child: &mut GroupChild) -> Result<(), ProcessError> {
    child
        .kill()
        .map_err(|source| ProcessError::Terminate { source })
}

fn terminate_and_reap(child: &mut GroupChild) -> Result<(), ProcessError> {
    ignore_already_exited(force_kill(child))?;
    reap_after_force(child)
}

fn reap_after_force(child: &mut GroupChild) -> Result<(), ProcessError> {
    match child.wait() {
        Ok(_) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::NotFound
            ) =>
        {
            Ok(())
        }
        Err(source) => Err(ProcessError::Wait { source }),
    }
}

fn ignore_already_exited(result: Result<(), ProcessError>) -> Result<(), ProcessError> {
    match result {
        Ok(()) => Ok(()),
        Err(ProcessError::Terminate { source })
            if matches!(
                source.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::NotFound
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

struct CaptureSignal {
    overflow: AtomicU8,
    total_seen: AtomicU64,
    total_captured: AtomicU64,
}

impl CaptureSignal {
    fn new() -> Self {
        Self {
            overflow: AtomicU8::new(CAPTURE_NONE),
            total_seen: AtomicU64::new(0),
            total_captured: AtomicU64::new(0),
        }
    }

    fn mark_overflow(&self, stream: u8) {
        let _ = self.overflow.compare_exchange(
            CAPTURE_NONE,
            stream,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn note_seen(&self, bytes: u64, total_limit: u64) {
        let previous =
            self.total_seen
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    Some(current.saturating_add(bytes))
                });
        let current = match previous {
            Ok(value) | Err(value) => value.saturating_add(bytes),
        };
        if current > total_limit {
            self.mark_overflow(CAPTURE_COMBINED);
        }
    }

    fn reserve_capture(&self, requested: u64, total_limit: u64) -> u64 {
        loop {
            let current = self.total_captured.load(Ordering::Acquire);
            let remaining = total_limit.saturating_sub(current);
            let granted = requested.min(remaining);
            if granted == 0 {
                return 0;
            }
            if self
                .total_captured
                .compare_exchange(
                    current,
                    current.saturating_add(granted),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return granted;
            }
        }
    }

    fn overflow_stream(&self) -> Option<OutputStream> {
        match self.overflow.load(Ordering::Acquire) {
            CAPTURE_STDOUT => Some(OutputStream::Stdout),
            CAPTURE_STDERR => Some(OutputStream::Stderr),
            CAPTURE_COMBINED => Some(OutputStream::Combined),
            _ => None,
        }
    }
}

fn spawn_reader<R>(
    thread_name: &'static str,
    stream_name: &'static str,
    reader: R,
    stream_limit: u64,
    total_limit: u64,
    stream_code: u8,
    signal: Arc<CaptureSignal>,
) -> Result<JoinHandle<Result<Vec<u8>, io::Error>>, ProcessError>
where
    R: Read + Send + 'static,
{
    thread::Builder::new()
        .name(thread_name.to_owned())
        .spawn(move || read_bounded(reader, stream_limit, total_limit, stream_code, &signal))
        .map_err(|source| ProcessError::ReaderSpawn {
            stream: stream_name,
            source,
        })
}

fn read_bounded<R: Read>(
    mut reader: R,
    stream_limit: u64,
    total_limit: u64,
    stream_code: u8,
    signal: &CaptureSignal,
) -> Result<Vec<u8>, io::Error> {
    let initial_capacity = usize::try_from(stream_limit.min(64 * 1_024)).unwrap_or(0);
    let mut captured = Vec::with_capacity(initial_capacity);
    let mut stream_seen = 0_u64;
    let mut chunk = [0_u8; 8 * 1_024];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok(captured);
        }
        let read_u64 = u64::try_from(read).unwrap_or(u64::MAX);
        stream_seen = stream_seen.saturating_add(read_u64);
        signal.note_seen(read_u64, total_limit);
        if stream_seen > stream_limit {
            signal.mark_overflow(stream_code);
        }

        let stream_remaining =
            stream_limit.saturating_sub(u64::try_from(captured.len()).unwrap_or(u64::MAX));
        let requested = read_u64.min(stream_remaining);
        let granted = signal.reserve_capture(requested, total_limit);
        let granted = usize::try_from(granted).unwrap_or(0).min(read);
        captured.extend_from_slice(&chunk[..granted]);
    }
}

fn join_reader(
    reader: JoinHandle<Result<Vec<u8>, io::Error>>,
    stream: &'static str,
) -> Result<Vec<u8>, ProcessError> {
    match reader.join() {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(source)) => Err(ProcessError::Reader { stream, source }),
        Err(_) => Err(ProcessError::ReaderPanicked { stream }),
    }
}
