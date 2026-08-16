use std::{
    collections::BTreeMap,
    env,
    error::Error,
    ffi::OsString,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use tactus_core::{
    BlobRef, CancellationSource, CancellationToken, CellId, CheckpointId, ContainmentKind,
    FramedWorker, FramedWorkerConfig, NativeProcessSupervisor, OutputBudget, OutputStream,
    ProcessSupervisor, ProcessTimeouts, ProjectId, RunId, Sha256Digest, WorkerCommand,
    WorkerCompletion, WorkerError, WorkerEvent, WorkerEventSink, WorkerPayloadDecoder, WorkerPort,
    WorkerTerminal, WorkspaceTransactionId,
};
use tempfile::tempdir;

struct FixtureDecoder;

impl WorkerPayloadDecoder for FixtureDecoder {
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
                environment: Sha256Digest::from_bytes([9; 32]),
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
struct Sink(Vec<u8>);

impl WorkerEventSink for Sink {
    fn output(
        &mut self,
        _worker_sequence: u64,
        _stream: OutputStream,
        bytes: &[u8],
    ) -> Result<(), WorkerError> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

#[test]
fn native_supervisor_owns_framed_worker_to_terminal() -> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let executable = fixture_executable()?;
    let supervisor = NativeProcessSupervisor::new();
    assert_ne!(
        supervisor.capabilities().containment,
        ContainmentKind::Unsupported
    );
    let worker = fixture_worker(executable, Vec::new())?;
    let command = command(temporary.path());
    let mut sink = Sink::default();

    let completion: WorkerCompletion =
        worker.execute(command, &CancellationToken::new(), &mut sink)?;

    assert_eq!(completion.terminal(), &WorkerTerminal::Succeeded);
    assert_eq!(sink.0, b"fixture output\n");
    Ok(())
}

#[test]
fn cancellation_reaps_supervised_fixture_before_return() -> Result<(), Box<dyn Error>> {
    let temporary = tempdir()?;
    let worker = fixture_worker(
        fixture_executable()?,
        vec![OsString::from("--fixture-mode"), OsString::from("sleep")],
    )?;
    let command = command(temporary.path());
    let cancellation = CancellationSource::new();
    let token = cancellation.token();
    let started = Instant::now();
    let join = thread::spawn(move || worker.execute(command, &token, &mut Sink::default()));
    thread::sleep(Duration::from_millis(50));
    cancellation.cancel();
    let result = join.join().map_err(|_| "worker test thread panicked")?;

    assert!(matches!(result, Err(WorkerError::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(5));
    Ok(())
}

fn fixture_worker(
    executable: PathBuf,
    base_arguments: Vec<OsString>,
) -> Result<FramedWorker<NativeProcessSupervisor, FixtureDecoder>, WorkerError> {
    let config = FramedWorkerConfig::new(
        executable,
        base_arguments,
        minimal_environment(),
        ProcessTimeouts::new(Duration::from_secs(5), Duration::from_millis(100))
            .map_err(WorkerError::ProcessSpec)?,
        OutputBudget::new(1024 * 1024, 1024 * 1024, 2 * 1024 * 1024)
            .map_err(WorkerError::ProcessSpec)?,
        64 * 1024,
    )?;
    Ok(FramedWorker::new(
        NativeProcessSupervisor::new(),
        FixtureDecoder,
        config,
    ))
}

fn command(workspace: &Path) -> WorkerCommand {
    WorkerCommand::new(
        RunId::generate(),
        ProjectId::generate(),
        WorkspaceTransactionId::generate(),
        CellId::generate(),
        BlobRef::new(Sha256Digest::from_bytes([1; 32]), 1),
        CheckpointId::from_digest(Sha256Digest::from_bytes([2; 32])),
        workspace.to_path_buf(),
    )
}

fn fixture_executable() -> Result<PathBuf, Box<dyn Error>> {
    Ok(PathBuf::from(env!("CARGO_BIN_EXE_tactus-worker-fixture")).canonicalize()?)
}

fn minimal_environment() -> BTreeMap<OsString, OsString> {
    ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"]
        .into_iter()
        .filter_map(|name| env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect()
}
