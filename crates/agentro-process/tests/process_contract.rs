use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use agentro_process::{
    CancellationSource, CancellationToken, ContainmentKind, NativeProcessSupervisor, OutputBudget,
    ProcessSpec, ProcessSupervisor, ProcessTimeouts, ResourceLimits, TerminationReason,
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_agentro-process-fixture"))
}

fn spec(
    arguments: &[&str],
    overall: Duration,
    budget: OutputBudget,
) -> Result<ProcessSpec, Box<dyn Error>> {
    let arguments = arguments.iter().map(OsString::from).collect();
    Ok(ProcessSpec::new(
        fixture(),
        arguments,
        absolute_current_dir()?,
        BTreeMap::new(),
        ProcessTimeouts::new(overall, Duration::from_millis(100))?,
        budget,
        ResourceLimits::default(),
    )?)
}

fn absolute_current_dir() -> io::Result<PathBuf> {
    let directory = std::env::current_dir()?;
    if directory.is_absolute() {
        Ok(directory)
    } else {
        Path::new(".").canonicalize()
    }
}

#[test]
fn argv_is_literal_and_both_streams_are_drained() -> Result<(), Box<dyn Error>> {
    let supervisor = NativeProcessSupervisor::new();
    let output = supervisor.run(
        spec(
            &["echo", "literal;&|value"],
            Duration::from_secs(2),
            OutputBudget::new(4_096, 4_096, 8_192)?,
        )?,
        &CancellationToken::new(),
    )?;

    assert_eq!(output.termination(), TerminationReason::Exited);
    assert!(output.success());
    assert_eq!(output.stdout(), b"literal;&|value\n");
    assert_eq!(output.stderr(), b"fixture-stderr\n");
    Ok(())
}

#[test]
fn output_flood_terminates_at_hard_capture_budget() -> Result<(), Box<dyn Error>> {
    let supervisor = NativeProcessSupervisor::new();
    let output = supervisor.run(
        spec(
            &["flood", "1048576"],
            Duration::from_secs(5),
            OutputBudget::new(4_096, 4_096, 6_000)?,
        )?,
        &CancellationToken::new(),
    )?;

    assert!(matches!(
        output.termination(),
        TerminationReason::OutputLimitExceeded(_)
    ));
    assert!(output.stdout().len() <= 4_096);
    assert!(output.stderr().len() <= 4_096);
    assert!(output.stdout().len() + output.stderr().len() <= 6_000);
    Ok(())
}

#[test]
fn leader_exit_does_not_leave_a_pipe_holding_descendant() -> Result<(), Box<dyn Error>> {
    let supervisor = NativeProcessSupervisor::new();
    let started = Instant::now();
    let output = supervisor.run(
        spec(
            &["spawn-child", "5000"],
            Duration::from_secs(2),
            OutputBudget::new(4_096, 4_096, 8_192)?,
        )?,
        &CancellationToken::new(),
    )?;

    assert_eq!(output.termination(), TerminationReason::Exited);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(output.stdout(), b"child-started\n");
    Ok(())
}

#[test]
fn cancellation_owns_termination_and_reap() -> Result<(), Box<dyn Error>> {
    let source = CancellationSource::new();
    let token = source.token();
    let request = spec(
        &["sleep", "5000"],
        Duration::from_secs(10),
        OutputBudget::new(4_096, 4_096, 8_192)?,
    )?;
    let started = Instant::now();
    let worker = thread::spawn(move || NativeProcessSupervisor::new().run(request, &token));
    thread::sleep(Duration::from_millis(100));
    source.cancel();
    let output = worker
        .join()
        .map_err(|_| io::Error::other("supervisor test thread panicked"))??;

    assert_eq!(output.termination(), TerminationReason::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

#[test]
fn overall_deadline_terminates_and_reaps() -> Result<(), Box<dyn Error>> {
    let supervisor = NativeProcessSupervisor::new();
    let started = Instant::now();
    let output = supervisor.run(
        spec(
            &["sleep", "5000"],
            Duration::from_millis(100),
            OutputBudget::new(4_096, 4_096, 8_192)?,
        )?,
        &CancellationToken::new(),
    )?;

    assert_eq!(output.termination(), TerminationReason::DeadlineExceeded);
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

#[test]
fn capabilities_do_not_claim_unimplemented_resource_limits() {
    let capabilities = NativeProcessSupervisor::new().capabilities();
    #[cfg(windows)]
    assert_eq!(capabilities.containment, ContainmentKind::WindowsJobObject);
    #[cfg(target_os = "linux")]
    assert_eq!(capabilities.containment, ContainmentKind::LinuxProcessGroup);
    assert!(!capabilities.hard_memory_limit);
    assert!(!capabilities.hard_process_limit);
    assert!(!capabilities.hard_cpu_limit);
}
