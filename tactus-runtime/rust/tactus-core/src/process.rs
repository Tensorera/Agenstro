use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use command_group::{CommandGroup, GroupChild};
use thiserror::Error;

const MAX_ARGUMENTS: usize = 4_096;
const MAX_ARGUMENT_BYTES: u64 = 1024 * 1024;
const MAX_ENVIRONMENT_VARIABLES: usize = 256;
const MAX_ENVIRONMENT_BYTES: u64 = 1024 * 1024;
const MAX_STREAM_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PROCESS_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_TERMINATION_GRACE: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Owner of an idempotent cooperative cancellation signal.
#[derive(Debug, Default)]
pub struct CancellationSource {
    cancelled: Arc<AtomicBool>,
}

impl CancellationSource {
    /// Creates an active cancellation source.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a cloneable read-only token.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        CancellationToken {
            cancelled: Arc::clone(&self.cancelled),
        }
    }

    /// Requests cancellation. Repeated calls are idempotent.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Read-only cooperative cancellation signal.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates an active standalone token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reports whether the owner requested cancellation.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Validated process-wide and force-termination durations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessTimeouts {
    overall: Duration,
    termination_grace: Duration,
}

impl ProcessTimeouts {
    /// Constructs non-zero bounded process durations.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidConfiguration`] outside hard limits.
    pub fn new(overall: Duration, termination_grace: Duration) -> Result<Self, ProcessError> {
        if overall.is_zero() || overall > MAX_PROCESS_TIMEOUT {
            return Err(ProcessError::InvalidConfiguration {
                field: "overall timeout",
            });
        }
        if termination_grace.is_zero() || termination_grace > MAX_TERMINATION_GRACE {
            return Err(ProcessError::InvalidConfiguration {
                field: "termination grace",
            });
        }
        Ok(Self {
            overall,
            termination_grace,
        })
    }
}

/// Per-stream and aggregate retained output limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputBudget {
    stdout_bytes: u64,
    stderr_bytes: u64,
    total_bytes: u64,
}

impl OutputBudget {
    /// Constructs non-zero bounded output limits.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidConfiguration`] above hard limits.
    pub fn new(
        stdout_bytes: u64,
        stderr_bytes: u64,
        total_bytes: u64,
    ) -> Result<Self, ProcessError> {
        if stdout_bytes == 0 || stdout_bytes > MAX_STREAM_BYTES {
            return Err(ProcessError::InvalidConfiguration {
                field: "stdout bytes",
            });
        }
        if stderr_bytes == 0 || stderr_bytes > MAX_STREAM_BYTES {
            return Err(ProcessError::InvalidConfiguration {
                field: "stderr bytes",
            });
        }
        if total_bytes == 0 || total_bytes > MAX_TOTAL_BYTES {
            return Err(ProcessError::InvalidConfiguration {
                field: "total output bytes",
            });
        }
        Ok(Self {
            stdout_bytes,
            stderr_bytes,
            total_bytes,
        })
    }
}

/// Fully validated shell-free process launch request.
#[derive(Debug)]
pub struct ProcessSpec {
    executable: PathBuf,
    arguments: Vec<OsString>,
    working_directory: PathBuf,
    environment: BTreeMap<OsString, OsString>,
    timeouts: ProcessTimeouts,
    output_budget: OutputBudget,
}

impl ProcessSpec {
    /// Constructs an argv-based request with explicit environment and bounds.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessError::InvalidConfiguration`] for relative paths,
    /// NULs, or excessive argv/environment resources.
    pub fn new(
        executable: PathBuf,
        arguments: Vec<OsString>,
        working_directory: PathBuf,
        environment: BTreeMap<OsString, OsString>,
        timeouts: ProcessTimeouts,
        output_budget: OutputBudget,
    ) -> Result<Self, ProcessError> {
        if !executable.is_absolute() {
            return Err(ProcessError::InvalidConfiguration {
                field: "executable",
            });
        }
        if !working_directory.is_absolute() {
            return Err(ProcessError::InvalidConfiguration {
                field: "working directory",
            });
        }
        if arguments.len() > MAX_ARGUMENTS {
            return Err(ProcessError::InvalidConfiguration {
                field: "argument count",
            });
        }
        if environment.len() > MAX_ENVIRONMENT_VARIABLES {
            return Err(ProcessError::InvalidConfiguration {
                field: "environment count",
            });
        }
        let argument_bytes = arguments.iter().try_fold(0_u64, |total, value| {
            if contains_nul(value) {
                return None;
            }
            total.checked_add(os_bytes(value))
        });
        if argument_bytes.is_none_or(|bytes| bytes > MAX_ARGUMENT_BYTES) {
            return Err(ProcessError::InvalidConfiguration {
                field: "argument bytes",
            });
        }
        let environment_bytes = environment.iter().try_fold(0_u64, |total, (name, value)| {
            if name.is_empty()
                || name.to_string_lossy().contains('=')
                || contains_nul(name)
                || contains_nul(value)
            {
                return None;
            }
            total
                .checked_add(os_bytes(name))
                .and_then(|next| next.checked_add(os_bytes(value)))
        });
        if environment_bytes.is_none_or(|bytes| bytes > MAX_ENVIRONMENT_BYTES) {
            return Err(ProcessError::InvalidConfiguration {
                field: "environment bytes",
            });
        }
        Ok(Self {
            executable,
            arguments,
            working_directory,
            environment,
            timeouts,
            output_budget,
        })
    }
}

/// Native process-tree containment kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContainmentKind {
    /// Windows Job Object owned by `command-group`.
    WindowsJobObject,
    /// Linux process group owned by `command-group`.
    LinuxProcessGroup,
    /// No supported containment implementation exists.
    Unsupported,
}

/// Process capabilities actually enforced by the native implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessCapabilities {
    /// Process-tree ownership mechanism.
    pub containment: ContainmentKind,
    /// Whether a group-level graceful signal is available.
    pub graceful_group_signal: bool,
}

/// Why a supervised process operation became terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminationReason {
    /// Leader exited; descendants were still cleaned before return.
    Exited,
    /// Caller cancellation was observed.
    Cancelled,
    /// Overall deadline elapsed.
    DeadlineExceeded,
    /// A stream or aggregate output limit was crossed.
    OutputLimitExceeded,
}

/// Bounded output and normalized process status after complete cleanup.
#[derive(Debug)]
pub struct ProcessOutput {
    termination: TerminationReason,
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl ProcessOutput {
    /// Returns why supervision terminated.
    #[must_use]
    pub const fn termination(&self) -> TerminationReason {
        self.termination
    }

    /// Returns a portable leader exit code when available.
    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        self.status.as_ref().and_then(ExitStatus::code)
    }

    /// Reports a normal zero leader exit.
    #[must_use]
    pub fn success(&self) -> bool {
        self.termination == TerminationReason::Exited
            && self.status.as_ref().is_some_and(ExitStatus::success)
    }

    /// Returns bounded stdout bytes.
    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns bounded stderr bytes.
    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

/// Process validation, spawn, output, wait, or containment failure.
#[derive(Debug, Error)]
pub enum ProcessError {
    /// A bounded process setting was invalid.
    #[error("invalid process configuration: {field}")]
    InvalidConfiguration {
        /// Invalid setting name.
        field: &'static str,
    },
    /// Current target has no supported process-tree owner.
    #[error("process containment is unsupported on this platform")]
    UnsupportedPlatform,
    /// Resolved executable or working directory is unavailable.
    #[error("process launch path is unavailable")]
    Path(#[source] io::Error),
    /// Child could not be spawned inside containment.
    #[error("contained process spawn failed")]
    Spawn(#[source] io::Error),
    /// A required output pipe was absent.
    #[error("contained process output pipe is unavailable")]
    PipeUnavailable,
    /// Output drain thread creation failed.
    #[error("contained process output drain could not start")]
    ReaderSpawn(#[source] io::Error),
    /// Output drain failed.
    #[error("contained process output drain failed")]
    Reader(#[source] io::Error),
    /// Output drain panicked.
    #[error("contained process output drain panicked")]
    ReaderPanicked,
    /// Wait or reap failed.
    #[error("contained process wait failed")]
    Wait(#[source] io::Error),
    /// Tree termination failed.
    #[error("contained process termination failed")]
    Terminate(#[source] io::Error),
}

/// Sole process owner used by workers and read-only Git adapters.
pub trait ProcessSupervisor: Send + Sync {
    /// Reports actual containment.
    fn capabilities(&self) -> ProcessCapabilities;

    /// Runs one process and returns only after pipes and the process tree close.
    ///
    /// # Errors
    ///
    /// Returns setup, I/O, wait, or containment failures. Ordinary non-zero
    /// exit, cancellation, timeout, and output limits are normalized output.
    fn run(
        &self,
        spec: ProcessSpec,
        cancellation: &CancellationToken,
    ) -> Result<ProcessOutput, ProcessError>;
}

/// Native Job Object or process-group supervisor.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeProcessSupervisor;

impl NativeProcessSupervisor {
    /// Creates the stateless native supervisor.
    #[must_use]
    pub const fn new() -> Self {
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
        let executable = fs::metadata(&spec.executable).map_err(ProcessError::Path)?;
        let directory = fs::metadata(&spec.working_directory).map_err(ProcessError::Path)?;
        if !executable.is_file() || !directory.is_dir() {
            return Err(ProcessError::Path(io::Error::new(
                io::ErrorKind::InvalidInput,
                "launch path has the wrong type",
            )));
        }
        let mut command = Command::new(&spec.executable);
        command
            .args(&spec.arguments)
            .current_dir(&spec.working_directory)
            .env_clear()
            .envs(&spec.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut grouped = command.group();
        #[cfg(windows)]
        grouped.kill_on_drop(true);
        let child = grouped.spawn().map_err(ProcessError::Spawn)?;
        supervise(child, &spec, cancellation, self.capabilities())
    }
}

fn native_capabilities() -> ProcessCapabilities {
    #[cfg(windows)]
    {
        ProcessCapabilities {
            containment: ContainmentKind::WindowsJobObject,
            graceful_group_signal: false,
        }
    }
    #[cfg(target_os = "linux")]
    {
        ProcessCapabilities {
            containment: ContainmentKind::LinuxProcessGroup,
            graceful_group_signal: true,
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        ProcessCapabilities {
            containment: ContainmentKind::Unsupported,
            graceful_group_signal: false,
        }
    }
}

fn supervise(
    mut child: GroupChild,
    spec: &ProcessSpec,
    cancellation: &CancellationToken,
    capabilities: ProcessCapabilities,
) -> Result<ProcessOutput, ProcessError> {
    let stdout = child
        .inner()
        .stdout
        .take()
        .ok_or(ProcessError::PipeUnavailable)?;
    let stderr = child
        .inner()
        .stderr
        .take()
        .ok_or(ProcessError::PipeUnavailable)?;
    let total_seen = Arc::new(AtomicU64::new(0));
    let total_captured = Arc::new(AtomicU64::new(0));
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_reader(
        stdout,
        spec.output_budget.stdout_bytes,
        spec.output_budget.total_bytes,
        Arc::clone(&total_seen),
        Arc::clone(&total_captured),
        Arc::clone(&overflow),
    )?;
    let stderr_reader = spawn_reader(
        stderr,
        spec.output_budget.stderr_bytes,
        spec.output_budget.total_bytes,
        Arc::clone(&total_seen),
        Arc::clone(&total_captured),
        Arc::clone(&overflow),
    )?;
    let deadline = Instant::now() + spec.timeouts.overall;
    let (reason, mut status) = loop {
        if overflow.load(Ordering::Acquire) {
            break (TerminationReason::OutputLimitExceeded, None);
        }
        if cancellation.is_cancelled() {
            break (TerminationReason::Cancelled, None);
        }
        if Instant::now() >= deadline {
            break (TerminationReason::DeadlineExceeded, None);
        }
        if let Some(status) = child.try_wait().map_err(ProcessError::Wait)? {
            break (TerminationReason::Exited, Some(status));
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    };

    if reason != TerminationReason::Exited && capabilities.graceful_group_signal {
        send_graceful(&child)?;
        let grace_deadline = Instant::now() + spec.timeouts.termination_grace;
        while Instant::now() < grace_deadline {
            if let Some(exit) = child.try_wait().map_err(ProcessError::Wait)? {
                status = Some(exit);
                break;
            }
            thread::sleep(
                POLL_INTERVAL.min(grace_deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
    // A leader can exit during graceful cancellation while a descendant keeps
    // running and holds stdout/stderr open. Kill the group before joining the
    // drain owners regardless of the leader's status.
    ignore_gone(child.kill().map_err(ProcessError::Terminate))?;
    if status.is_none() || reason == TerminationReason::Exited {
        status = child.wait().map(Some).or_else(|error| {
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::NotFound
            ) {
                Ok(status)
            } else {
                Err(ProcessError::Wait(error))
            }
        })?;
    }
    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    Ok(ProcessOutput {
        termination: reason,
        status,
        stdout,
        stderr,
    })
}

#[cfg(unix)]
fn send_graceful(child: &GroupChild) -> Result<(), ProcessError> {
    use command_group::{Signal, UnixChildExt};
    ignore_gone(
        child
            .signal(Signal::SIGTERM)
            .map_err(ProcessError::Terminate),
    )
}

#[cfg(not(unix))]
const fn send_graceful(_child: &GroupChild) -> Result<(), ProcessError> {
    Ok(())
}

fn ignore_gone(result: Result<(), ProcessError>) -> Result<(), ProcessError> {
    match result {
        Err(ProcessError::Terminate(error)) if process_is_already_absent(&error) => Ok(()),
        other => other,
    }
}

fn process_is_already_absent(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::NotFound
    ) {
        return true;
    }

    // Rust 1.88 classifies Linux ESRCH as Uncategorized. A process-group
    // termination racing with normal leader exit is still successful cleanup.
    #[cfg(target_os = "linux")]
    if error.raw_os_error() == Some(3) {
        return true;
    }

    false
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::process_is_already_absent;
    use std::io;

    #[test]
    fn esrch_is_an_idempotent_termination_result() {
        assert!(process_is_already_absent(&io::Error::from_raw_os_error(3)));
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    reader: R,
    stream_limit: u64,
    total_limit: u64,
    total_seen: Arc<AtomicU64>,
    total_captured: Arc<AtomicU64>,
    overflow: Arc<AtomicBool>,
) -> Result<JoinHandle<Result<Vec<u8>, io::Error>>, ProcessError> {
    thread::Builder::new()
        .name("tactus-output-drain".to_owned())
        .spawn(move || {
            read_bounded(
                reader,
                stream_limit,
                total_limit,
                &total_seen,
                &total_captured,
                &overflow,
            )
        })
        .map_err(ProcessError::ReaderSpawn)
}

fn read_bounded(
    mut reader: impl Read,
    stream_limit: u64,
    total_limit: u64,
    total_seen: &AtomicU64,
    total_captured: &AtomicU64,
    overflow: &AtomicBool,
) -> Result<Vec<u8>, io::Error> {
    let mut output = Vec::with_capacity(usize::try_from(stream_limit.min(64 * 1024)).unwrap_or(0));
    let mut seen = 0_u64;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(output);
        }
        let bytes = u64::try_from(read).unwrap_or(u64::MAX);
        seen = seen.saturating_add(bytes);
        let previous = total_seen.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            Some(current.saturating_add(bytes))
        });
        let aggregate = match previous {
            Ok(value) | Err(value) => value.saturating_add(bytes),
        };
        if seen > stream_limit || aggregate > total_limit {
            overflow.store(true, Ordering::Release);
        }
        let stream_remaining =
            stream_limit.saturating_sub(u64::try_from(output.len()).unwrap_or(u64::MAX));
        let requested = bytes.min(stream_remaining);
        let retain = usize::try_from(reserve_capture(total_captured, requested, total_limit))
            .unwrap_or(0)
            .min(read);
        output.extend_from_slice(&buffer[..retain]);
    }
}

fn reserve_capture(total_captured: &AtomicU64, requested: u64, total_limit: u64) -> u64 {
    loop {
        let current = total_captured.load(Ordering::Acquire);
        let granted = requested.min(total_limit.saturating_sub(current));
        if granted == 0 {
            return 0;
        }
        if total_captured
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

fn join_reader(reader: JoinHandle<Result<Vec<u8>, io::Error>>) -> Result<Vec<u8>, ProcessError> {
    match reader.join() {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(ProcessError::Reader(error)),
        Err(_) => Err(ProcessError::ReaderPanicked),
    }
}

#[cfg(windows)]
fn contains_nul(value: &OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().any(|unit| unit == 0)
}

#[cfg(unix)]
fn contains_nul(value: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().contains(&0)
}

#[cfg(not(any(unix, windows)))]
fn contains_nul(value: &OsStr) -> bool {
    value.to_string_lossy().contains('\0')
}

#[cfg(windows)]
fn os_bytes(value: &OsStr) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    value
        .encode_wide()
        .fold(0_u64, |total, _| total.saturating_add(2))
}

#[cfg(unix)]
fn os_bytes(value: &OsStr) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().len() as u64
}

#[cfg(not(any(unix, windows)))]
fn os_bytes(value: &OsStr) -> u64 {
    value.to_string_lossy().len() as u64
}

#[cfg(test)]
mod output_tests {
    use std::{io::Cursor, sync::atomic::AtomicBool};

    use super::*;

    #[test]
    fn aggregate_capture_never_exceeds_the_total_budget() -> Result<(), io::Error> {
        let total_seen = AtomicU64::new(0);
        let total_captured = AtomicU64::new(0);
        let overflow = AtomicBool::new(false);

        let stdout = read_bounded(
            Cursor::new(vec![b'a'; 8]),
            8,
            10,
            &total_seen,
            &total_captured,
            &overflow,
        )?;
        let stderr = read_bounded(
            Cursor::new(vec![b'b'; 8]),
            8,
            10,
            &total_seen,
            &total_captured,
            &overflow,
        )?;

        assert_eq!(stdout.len() + stderr.len(), 10);
        assert!(overflow.load(Ordering::Acquire));
        Ok(())
    }
}
