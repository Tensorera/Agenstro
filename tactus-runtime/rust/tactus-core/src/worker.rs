use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

use agentro_contracts::{ErrorCode, Sha256Digest};
use thiserror::Error;

use crate::{
    BlobRef, CancellationToken, CellId, CheckpointId, OutputBudget, ProcessError, ProcessSpec,
    ProcessSupervisor, ProcessTimeouts, ProjectId, RunId, TerminationReason,
    WorkspaceTransactionId,
};

const FRAME_MAGIC: &[u8; 4] = b"TACT";
const FRAME_HEADER_BYTES: usize = 48;
const PROTOCOL_MAJOR: u16 = 1;
const PROTOCOL_MINOR: u16 = 0;
const MAX_FRAME_BYTES: usize = 256 * 1024;
const MAX_OUTPUT_CHUNK_BYTES: usize = 64 * 1024;

/// A worker output channel. No ordering is claimed across different streams.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputStream {
    /// Raw standard-output bytes.
    Stdout,
    /// Raw standard-error bytes.
    Stderr,
    /// Jupyter display or IOPub bytes already reduced to a bounded chunk.
    Display,
}

impl OutputStream {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Display => "display",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "stdout" => Some(Self::Stdout),
            "stderr" => Some(Self::Stderr),
            "display" => Some(Self::Display),
            _ => None,
        }
    }
}

/// Terminal result reported by a versioned worker payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerTerminal {
    /// User code and worker cleanup completed successfully.
    Succeeded,
    /// User code or the worker failed with a bounded stable code.
    Failed(ErrorCode),
    /// The worker acknowledged cancellation.
    Cancelled,
}

/// One decoded worker lifecycle event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerEvent {
    /// Initial protocol declaration; this must be sequence one.
    Hello {
        /// Worker-local monotonic event sequence.
        sequence: u64,
        /// Worker protocol major version.
        protocol_major: u16,
        /// Worker protocol minor version.
        protocol_minor: u16,
    },
    /// Worker environment is ready for the requested cell.
    Ready {
        /// Worker-local monotonic event sequence.
        sequence: u64,
        /// Environment fingerprint; Python memory is intentionally absent.
        environment: Sha256Digest,
        /// Non-zero worker or kernel generation.
        kernel_generation: u64,
    },
    /// One bounded raw output chunk.
    Output {
        /// Worker-local monotonic event sequence.
        sequence: u64,
        /// Independent output stream.
        stream: OutputStream,
        /// Bounded bytes to publish by CAS reference.
        bytes: Vec<u8>,
    },
    /// Liveness event that carries no durable interpreter state.
    Heartbeat {
        /// Worker-local monotonic event sequence.
        sequence: u64,
    },
    /// Exactly one terminal worker result.
    Finished {
        /// Worker-local monotonic event sequence.
        sequence: u64,
        /// Normalized terminal result.
        terminal: WorkerTerminal,
    },
}

impl WorkerEvent {
    /// Returns the worker-local monotonic sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        match self {
            Self::Hello { sequence, .. }
            | Self::Ready { sequence, .. }
            | Self::Output { sequence, .. }
            | Self::Heartbeat { sequence }
            | Self::Finished { sequence, .. } => *sequence,
        }
    }
}

/// Immutable references supplied to one externally supervised worker.
#[derive(Clone, Debug)]
pub struct WorkerCommand {
    run_id: RunId,
    project_id: ProjectId,
    transaction_id: WorkspaceTransactionId,
    cell_id: CellId,
    source: BlobRef,
    baseline: CheckpointId,
    workspace_root: PathBuf,
}

impl WorkerCommand {
    /// Creates a worker command containing references, never Python memory.
    #[must_use]
    pub const fn new(
        run_id: RunId,
        project_id: ProjectId,
        transaction_id: WorkspaceTransactionId,
        cell_id: CellId,
        source: BlobRef,
        baseline: CheckpointId,
        workspace_root: PathBuf,
    ) -> Self {
        Self {
            run_id,
            project_id,
            transaction_id,
            cell_id,
            source,
            baseline,
            workspace_root,
        }
    }

    /// Returns the execution identity used by every frame.
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    /// Returns the stable project identity.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }

    /// Returns the workspace transaction identity.
    #[must_use]
    pub const fn transaction_id(&self) -> WorkspaceTransactionId {
        self.transaction_id
    }

    /// Returns the stable cell identity.
    #[must_use]
    pub const fn cell_id(&self) -> CellId {
        self.cell_id
    }

    /// Returns the immutable source object reference.
    #[must_use]
    pub const fn source(&self) -> BlobRef {
        self.source
    }

    /// Returns the immutable baseline checkpoint reference.
    #[must_use]
    pub const fn baseline(&self) -> CheckpointId {
        self.baseline
    }

    /// Returns the explicitly rebound workspace root.
    #[must_use]
    pub fn workspace_root(&self) -> &PathBuf {
        &self.workspace_root
    }
}

/// Validated worker environment and terminal result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerCompletion {
    environment: Sha256Digest,
    kernel_generation: u64,
    terminal: WorkerTerminal,
}

impl WorkerCompletion {
    /// Creates a validated completion for worker adapters and test fixtures.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError::InvalidLifecycle`] for generation zero.
    pub fn new(
        environment: Sha256Digest,
        kernel_generation: u64,
        terminal: WorkerTerminal,
    ) -> Result<Self, WorkerError> {
        if kernel_generation == 0 {
            return Err(WorkerError::InvalidLifecycle);
        }
        Ok(Self {
            environment,
            kernel_generation,
            terminal,
        })
    }

    /// Returns the environment fingerprint reported before execution.
    #[must_use]
    pub const fn environment(&self) -> Sha256Digest {
        self.environment
    }

    /// Returns the non-zero worker or kernel generation.
    #[must_use]
    pub const fn kernel_generation(&self) -> u64 {
        self.kernel_generation
    }

    /// Returns the terminal result.
    #[must_use]
    pub const fn terminal(&self) -> &WorkerTerminal {
        &self.terminal
    }
}

/// Worker process, frame, lifecycle, or output-sink failure.
#[derive(Debug, Error)]
pub enum WorkerError {
    /// A framing or worker setting was invalid.
    #[error("invalid worker configuration: {field}")]
    InvalidConfiguration {
        /// Invalid setting name.
        field: &'static str,
    },
    /// Process specification validation failed.
    #[error("worker process specification is invalid")]
    ProcessSpec(#[source] ProcessError),
    /// The mandatory process supervisor failed.
    #[error("worker process supervision failed")]
    Process(#[source] ProcessError),
    /// ProcessSupervisor completed caller cancellation and tree cleanup.
    #[error("worker execution was cancelled")]
    Cancelled,
    /// The supervised process exceeded its execution deadline.
    #[error("worker execution deadline exceeded")]
    DeadlineExceeded,
    /// The process-level stdout/stderr budget was exceeded.
    #[error("worker process output budget exceeded")]
    ProcessOutputLimit,
    /// The worker process exited unsuccessfully or on an unknown condition.
    #[error("worker process died with exit code {exit_code:?}")]
    ProcessDied {
        /// Portable process exit code when available.
        exit_code: Option<i32>,
    },
    /// Frame magic or header bytes were malformed.
    #[error("worker frame header is malformed")]
    MalformedFrame,
    /// A frame declared an unsupported major or newer minor protocol.
    #[error("worker frame protocol version is unsupported")]
    ProtocolMismatch,
    /// A payload length exceeded the configured limit before allocation.
    #[error("worker frame payload exceeds {maximum} bytes")]
    FrameTooLarge {
        /// Configured maximum payload bytes.
        maximum: usize,
    },
    /// The final frame ended before its declared payload length.
    #[error("worker output ended with a truncated frame")]
    TruncatedFrame,
    /// A frame belonged to another run request.
    #[error("worker frame request identity does not match the active run")]
    RequestMismatch,
    /// The generated Protobuf payload adapter rejected a bounded payload.
    #[error("worker payload is invalid")]
    InvalidPayload,
    /// Worker event sequence was duplicated, skipped, or reordered.
    #[error("worker event sequence is not contiguous")]
    InvalidSequence,
    /// Worker events violated Hello/Ready/Finished ordering.
    #[error("worker lifecycle event is invalid in its current state")]
    InvalidLifecycle,
    /// The process exited without a terminal worker event.
    #[error("worker exited without a terminal event")]
    MissingTerminal,
    /// A decoded output event exceeded the per-chunk protocol bound.
    #[error("worker output chunk exceeds its protocol limit")]
    OutputChunkTooLarge,
    /// Durable CAS or event publication rejected an output chunk.
    #[error("worker output sink rejected an event")]
    SinkRejected,
    /// A worker supplied an invalid or excessive stable failure code.
    #[error("worker terminal error code is invalid")]
    InvalidErrorCode,
}

/// Decoder boundary for generated Protobuf worker payload types.
///
/// The fixed frame parser validates magic, version, request ID, and length
/// before this port receives bytes. Concrete Python worker Protobuf generation
/// belongs to the worker package rather than the Tactus domain crate.
pub trait WorkerPayloadDecoder: Send + Sync {
    /// Decodes one bounded Protobuf payload into a stable lifecycle event.
    ///
    /// # Errors
    ///
    /// Returns a typed payload or semantic validation error.
    fn decode(&self, payload: &[u8]) -> Result<WorkerEvent, WorkerError>;
}

/// Streaming sink for output chunks after lifecycle validation.
pub trait WorkerEventSink {
    /// Publishes one bounded output chunk by durable reference.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError::SinkRejected`] when publication cannot commit.
    fn output(
        &mut self,
        worker_sequence: u64,
        stream: OutputStream,
        bytes: &[u8],
    ) -> Result<(), WorkerError>;
}

/// Synchronous lifecycle port for one externally supervised worker operation.
pub trait WorkerPort: Send + Sync {
    /// Executes one worker command and returns only after process-tree cleanup.
    ///
    /// # Errors
    ///
    /// Returns typed process, framing, lifecycle, cancellation, or sink errors.
    fn execute(
        &self,
        command: WorkerCommand,
        cancellation: &CancellationToken,
        sink: &mut dyn WorkerEventSink,
    ) -> Result<WorkerCompletion, WorkerError>;
}

/// Validated argv, environment, timeout, output, and frame settings.
#[derive(Clone, Debug)]
pub struct FramedWorkerConfig {
    executable: PathBuf,
    base_arguments: Vec<OsString>,
    environment: BTreeMap<OsString, OsString>,
    timeouts: ProcessTimeouts,
    output_budget: OutputBudget,
    max_frame_bytes: usize,
}

impl FramedWorkerConfig {
    /// Constructs a shell-free worker launch configuration.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerError::InvalidConfiguration`] for a relative executable
    /// or zero/excessive frame bound. Complete argv and environment bounds are
    /// revalidated by [`ProcessSpec`] for each run.
    pub fn new(
        executable: PathBuf,
        base_arguments: Vec<OsString>,
        environment: BTreeMap<OsString, OsString>,
        timeouts: ProcessTimeouts,
        output_budget: OutputBudget,
        max_frame_bytes: usize,
    ) -> Result<Self, WorkerError> {
        if !executable.is_absolute() {
            return Err(WorkerError::InvalidConfiguration {
                field: "executable",
            });
        }
        if max_frame_bytes == 0 || max_frame_bytes > MAX_FRAME_BYTES {
            return Err(WorkerError::InvalidConfiguration {
                field: "frame bytes",
            });
        }
        Ok(Self {
            executable,
            base_arguments,
            environment,
            timeouts,
            output_budget,
            max_frame_bytes,
        })
    }
}

/// Worker adapter that cannot launch outside a supplied ProcessSupervisor.
pub struct FramedWorker<S, D> {
    supervisor: S,
    decoder: D,
    config: FramedWorkerConfig,
}

impl<S, D> FramedWorker<S, D> {
    /// Binds framing and payload decoding to the sole process owner.
    #[must_use]
    pub const fn new(supervisor: S, decoder: D, config: FramedWorkerConfig) -> Self {
        Self {
            supervisor,
            decoder,
            config,
        }
    }
}

impl<S, D> WorkerPort for FramedWorker<S, D>
where
    S: ProcessSupervisor,
    D: WorkerPayloadDecoder,
{
    fn execute(
        &self,
        command: WorkerCommand,
        cancellation: &CancellationToken,
        sink: &mut dyn WorkerEventSink,
    ) -> Result<WorkerCompletion, WorkerError> {
        let mut arguments = self.config.base_arguments.clone();
        arguments.extend([
            OsString::from("--tactus-worker-protocol"),
            OsString::from("1.0"),
            OsString::from("--run-id"),
            OsString::from(command.run_id().to_string()),
            OsString::from("--project-id"),
            OsString::from(command.project_id().to_string()),
            OsString::from("--transaction-id"),
            OsString::from(command.transaction_id().to_string()),
            OsString::from("--cell-id"),
            OsString::from(command.cell_id().to_string()),
            OsString::from("--source-digest"),
            OsString::from(command.source().digest().to_string()),
            OsString::from("--source-length"),
            OsString::from(command.source().length().to_string()),
            OsString::from("--baseline-checkpoint"),
            OsString::from(command.baseline().to_string()),
        ]);
        let spec = ProcessSpec::new(
            self.config.executable.clone(),
            arguments,
            command.workspace_root().clone(),
            self.config.environment.clone(),
            self.config.timeouts,
            self.config.output_budget,
        )
        .map_err(WorkerError::ProcessSpec)?;
        let output = self
            .supervisor
            .run(spec, cancellation)
            .map_err(WorkerError::Process)?;
        match output.termination() {
            TerminationReason::Cancelled => return Err(WorkerError::Cancelled),
            TerminationReason::DeadlineExceeded => return Err(WorkerError::DeadlineExceeded),
            TerminationReason::OutputLimitExceeded => {
                return Err(WorkerError::ProcessOutputLimit);
            }
            TerminationReason::Exited => {}
        }
        if !output.success() {
            return Err(WorkerError::ProcessDied {
                exit_code: output.exit_code(),
            });
        }
        drive_frames(
            output.stdout(),
            command.run_id(),
            self.config.max_frame_bytes,
            &self.decoder,
            sink,
        )
    }
}

/// Encodes one fixed-header frame for a bounded Protobuf payload.
///
/// This helper is primarily used by worker conformance fixtures. Production
/// workers should use generated protocol libraries around the same header.
///
/// # Errors
///
/// Returns [`WorkerError::FrameTooLarge`] above the hard payload maximum.
pub fn encode_worker_frame(run_id: RunId, payload: &[u8]) -> Result<Vec<u8>, WorkerError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(WorkerError::FrameTooLarge {
            maximum: MAX_FRAME_BYTES,
        });
    }
    let payload_length = u32::try_from(payload.len()).map_err(|_| WorkerError::FrameTooLarge {
        maximum: MAX_FRAME_BYTES,
    })?;
    let run_id = run_id.to_string();
    if run_id.len() != 36 {
        return Err(WorkerError::MalformedFrame);
    }
    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES.saturating_add(payload.len()));
    frame.extend_from_slice(FRAME_MAGIC);
    frame.extend_from_slice(&PROTOCOL_MAJOR.to_be_bytes());
    frame.extend_from_slice(&PROTOCOL_MINOR.to_be_bytes());
    frame.extend_from_slice(&payload_length.to_be_bytes());
    frame.extend_from_slice(run_id.as_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifecycleState {
    AwaitHello,
    AwaitReady,
    Running,
    Finished,
}

fn drive_frames<D: WorkerPayloadDecoder>(
    bytes: &[u8],
    expected_run_id: RunId,
    max_frame_bytes: usize,
    decoder: &D,
    sink: &mut dyn WorkerEventSink,
) -> Result<WorkerCompletion, WorkerError> {
    let mut offset = 0_usize;
    let mut state = LifecycleState::AwaitHello;
    let mut next_sequence = 1_u64;
    let mut environment = None;
    let mut kernel_generation = None;
    let mut terminal = None;
    while offset < bytes.len() {
        let remaining = bytes.len().saturating_sub(offset);
        if remaining < FRAME_HEADER_BYTES {
            return Err(WorkerError::TruncatedFrame);
        }
        let header = &bytes[offset..offset + FRAME_HEADER_BYTES];
        if &header[..4] != FRAME_MAGIC {
            return Err(WorkerError::MalformedFrame);
        }
        let major = u16::from_be_bytes([header[4], header[5]]);
        let minor = u16::from_be_bytes([header[6], header[7]]);
        if major != PROTOCOL_MAJOR || minor > PROTOCOL_MINOR {
            return Err(WorkerError::ProtocolMismatch);
        }
        let payload_length =
            u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
        if payload_length > max_frame_bytes {
            return Err(WorkerError::FrameTooLarge {
                maximum: max_frame_bytes,
            });
        }
        let request_text =
            std::str::from_utf8(&header[12..48]).map_err(|_| WorkerError::MalformedFrame)?;
        let request = RunId::parse(request_text).map_err(|_| WorkerError::MalformedFrame)?;
        if request != expected_run_id {
            return Err(WorkerError::RequestMismatch);
        }
        let payload_start = offset.saturating_add(FRAME_HEADER_BYTES);
        let payload_end = payload_start
            .checked_add(payload_length)
            .ok_or(WorkerError::TruncatedFrame)?;
        if payload_end > bytes.len() {
            return Err(WorkerError::TruncatedFrame);
        }
        let event = decoder.decode(&bytes[payload_start..payload_end])?;
        if event.sequence() != next_sequence {
            return Err(WorkerError::InvalidSequence);
        }
        next_sequence = next_sequence
            .checked_add(1)
            .ok_or(WorkerError::InvalidSequence)?;
        match event {
            WorkerEvent::Hello {
                protocol_major,
                protocol_minor,
                ..
            } if state == LifecycleState::AwaitHello
                && protocol_major == PROTOCOL_MAJOR
                && protocol_minor == PROTOCOL_MINOR =>
            {
                state = LifecycleState::AwaitReady;
            }
            WorkerEvent::Ready {
                environment: fingerprint,
                kernel_generation: generation,
                ..
            } if state == LifecycleState::AwaitReady && generation > 0 => {
                environment = Some(fingerprint);
                kernel_generation = Some(generation);
                state = LifecycleState::Running;
            }
            WorkerEvent::Output {
                sequence,
                stream,
                bytes,
            } if state == LifecycleState::Running => {
                if bytes.len() > MAX_OUTPUT_CHUNK_BYTES {
                    return Err(WorkerError::OutputChunkTooLarge);
                }
                sink.output(sequence, stream, &bytes)?;
            }
            WorkerEvent::Heartbeat { .. } if state == LifecycleState::Running => {}
            WorkerEvent::Finished {
                terminal: result, ..
            } if state == LifecycleState::Running => {
                terminal = Some(result);
                state = LifecycleState::Finished;
            }
            _ => return Err(WorkerError::InvalidLifecycle),
        }
        offset = payload_end;
    }
    if state != LifecycleState::Finished {
        return Err(WorkerError::MissingTerminal);
    }
    WorkerCompletion::new(
        environment.ok_or(WorkerError::InvalidLifecycle)?,
        kernel_generation.ok_or(WorkerError::InvalidLifecycle)?,
        terminal.ok_or(WorkerError::MissingTerminal)?,
    )
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use proptest::prelude::*;
    use tempfile::tempdir;

    use super::*;
    use crate::{ContainmentKind, ProcessCapabilities, ProcessOutput};

    struct TestDecoder;

    impl WorkerPayloadDecoder for TestDecoder {
        fn decode(&self, payload: &[u8]) -> Result<WorkerEvent, WorkerError> {
            if payload.len() < 9 {
                return Err(WorkerError::InvalidPayload);
            }
            let sequence = u64::from_be_bytes(
                payload[..8]
                    .try_into()
                    .map_err(|_| WorkerError::InvalidPayload)?,
            );
            match payload[8] {
                1 => Ok(WorkerEvent::Hello {
                    sequence,
                    protocol_major: 1,
                    protocol_minor: 0,
                }),
                2 => Ok(WorkerEvent::Ready {
                    sequence,
                    environment: Sha256Digest::from_bytes([7; 32]),
                    kernel_generation: 1,
                }),
                3 => Ok(WorkerEvent::Output {
                    sequence,
                    stream: OutputStream::Stdout,
                    bytes: payload[9..].to_vec(),
                }),
                4 => Ok(WorkerEvent::Finished {
                    sequence,
                    terminal: WorkerTerminal::Succeeded,
                }),
                _ => Err(WorkerError::InvalidPayload),
            }
        }
    }

    #[derive(Default)]
    struct CollectSink(Vec<Vec<u8>>);

    impl WorkerEventSink for CollectSink {
        fn output(
            &mut self,
            _worker_sequence: u64,
            _stream: OutputStream,
            bytes: &[u8],
        ) -> Result<(), WorkerError> {
            self.0.push(bytes.to_vec());
            Ok(())
        }
    }

    fn payload(sequence: u64, kind: u8, body: &[u8]) -> Vec<u8> {
        let mut payload = sequence.to_be_bytes().to_vec();
        payload.push(kind);
        payload.extend_from_slice(body);
        payload
    }

    #[test]
    fn framing_enforces_order_and_yields_bounded_output() -> Result<(), Box<dyn Error>> {
        let run_id = RunId::generate();
        let mut bytes = Vec::new();
        for event in [
            payload(1, 1, &[]),
            payload(2, 2, &[]),
            payload(3, 3, b"hello"),
            payload(4, 4, &[]),
        ] {
            bytes.extend(encode_worker_frame(run_id, &event)?);
        }
        let mut sink = CollectSink::default();
        let completion = drive_frames(&bytes, run_id, 64 * 1024, &TestDecoder, &mut sink)?;
        assert_eq!(completion.terminal(), &WorkerTerminal::Succeeded);
        assert_eq!(sink.0, vec![b"hello".to_vec()]);
        Ok(())
    }

    #[test]
    fn truncated_and_wrong_request_frames_fail_closed() -> Result<(), Box<dyn Error>> {
        let run_id = RunId::generate();
        let mut frame = encode_worker_frame(run_id, &payload(1, 1, &[]))?;
        frame.pop();
        assert!(matches!(
            drive_frames(
                &frame,
                run_id,
                64 * 1024,
                &TestDecoder,
                &mut CollectSink::default()
            ),
            Err(WorkerError::TruncatedFrame)
        ));

        let frame = encode_worker_frame(RunId::generate(), &payload(1, 1, &[]))?;
        assert!(matches!(
            drive_frames(
                &frame,
                run_id,
                64 * 1024,
                &TestDecoder,
                &mut CollectSink::default()
            ),
            Err(WorkerError::RequestMismatch)
        ));
        Ok(())
    }

    proptest! {
        #[test]
        fn arbitrary_frame_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
            let _ = drive_frames(
                &bytes,
                RunId::generate(),
                1024,
                &TestDecoder,
                &mut CollectSink::default(),
            );
        }
    }

    struct RejectingSupervisor {
        called: Arc<AtomicBool>,
    }

    impl ProcessSupervisor for RejectingSupervisor {
        fn capabilities(&self) -> ProcessCapabilities {
            ProcessCapabilities {
                containment: ContainmentKind::Unsupported,
                graceful_group_signal: false,
            }
        }

        fn run(
            &self,
            _spec: ProcessSpec,
            _cancellation: &CancellationToken,
        ) -> Result<ProcessOutput, ProcessError> {
            self.called.store(true, Ordering::Release);
            Err(ProcessError::UnsupportedPlatform)
        }
    }

    #[test]
    fn framed_worker_cannot_bypass_process_supervisor() -> Result<(), Box<dyn Error>> {
        let temporary = tempdir()?;
        let executable = std::env::current_exe()?;
        let called = Arc::new(AtomicBool::new(false));
        let config = FramedWorkerConfig::new(
            executable,
            Vec::new(),
            BTreeMap::new(),
            ProcessTimeouts::new(Duration::from_secs(1), Duration::from_millis(10))?,
            OutputBudget::new(1024, 1024, 2048)?,
            1024,
        )?;
        let worker = FramedWorker::new(
            RejectingSupervisor {
                called: Arc::clone(&called),
            },
            TestDecoder,
            config,
        );
        let command = WorkerCommand::new(
            RunId::generate(),
            ProjectId::generate(),
            WorkspaceTransactionId::generate(),
            CellId::generate(),
            BlobRef::new(Sha256Digest::from_bytes([1; 32]), 1),
            CheckpointId::from_digest(Sha256Digest::from_bytes([2; 32])),
            temporary.path().to_path_buf(),
        );
        assert!(matches!(
            worker.execute(
                command,
                &CancellationToken::new(),
                &mut CollectSink::default()
            ),
            Err(WorkerError::Process(ProcessError::UnsupportedPlatform))
        ));
        assert!(called.load(Ordering::Acquire));
        Ok(())
    }
}
