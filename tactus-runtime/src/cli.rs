//! Command-line surface for the Rust Tactus runtime.

use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, TryLockError, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    adapters::{SUPERVISED_PROCESS_GROUP_ENV, run_provider_host, run_workspace_paths_host},
    journal::{
        JournalError, Presentation, PresentationCategory, RunJournal, RunSummary, TransitionGuard,
        TransitionTrigger, TriggerKind, diagnostic_summary, diagnostic_value_summary,
    },
    process::{
        CancellationToken, InvocationKind, ProcessError, ProcessOutcome, ProcessSpec,
        ProcessSupervisor,
    },
    protocol::{
        JsonField, PluginFailure, PluginFrame, PluginRequest, TerminalResult, decode_json,
        decode_request,
    },
    studio::{ControlFailure, ControlSuccess},
    workspace::{
        PluginNamespace, ResolvedPlugin, ScriptInfo, Workspace, WorkspaceError, discover_scripts,
        doctor, effective_path, initialize_workspace,
    },
};

/// Parsed Tactus command line.
#[derive(Debug, Parser)]
#[command(
    name = "tactus",
    version,
    about = "Typed runtime for project-local Clef workflows"
)]
pub struct Arguments {
    /// Operation to perform.
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize an idempotent `.tactus` workspace.
    Init {
        /// Project directory.
        #[arg(default_value = ".")]
        root: PathBuf,
        /// Clef SDK directory containing `clef-sdk.cabal`.
        #[arg(long)]
        sdk: Option<PathBuf>,
        /// Emit one JSON document.
        #[arg(long)]
        json: bool,
    },
    /// List ordered entries followed by helper modules.
    List {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Emit one JSON document.
        #[arg(long)]
        json: bool,
    },
    /// Print the resolved generation instructions.
    Prompt {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Validate config, tools, SDK linkage, and plugin commands.
    Doctor {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Emit one JSON document.
        #[arg(long)]
        json: bool,
    },
    /// Print the normalized runtime JSON consumed by Clef.
    RuntimeJson {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
    },
    /// Type-check Haskell sources without running them.
    Check {
        /// Explicit sources; defaults to every discovered Haskell source.
        scripts: Vec<PathBuf>,
        /// Continue after a source fails.
        #[arg(long)]
        keep_going: bool,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Deadline for each Cabal/GHC process.
        #[arg(long, default_value_t = 1800)]
        timeout_seconds: u64,
        /// Additional Cabal library packages exposed while checking. Clef is
        /// always included. Repeat for more than one extension package.
        #[arg(long = "package", value_parser = validate_haskell_package)]
        packages: Vec<String>,
    },
    /// Run ordered Haskell workflow entry points.
    Run {
        /// Explicit entry source. Repeat to control selection and order.
        #[arg(long = "script")]
        scripts: Vec<PathBuf>,
        /// Continue after an entry fails.
        #[arg(long)]
        keep_going: bool,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Deadline for each Cabal/runghc process.
        #[arg(long, default_value_t = 1800)]
        timeout_seconds: u64,
        /// Additional Cabal library packages exposed while running. Clef is
        /// always included. Repeat for more than one extension package.
        #[arg(long = "package", value_parser = validate_haskell_package)]
        packages: Vec<String>,
        /// Arguments passed to every selected entry after `--`.
        #[arg(last = true)]
        arguments: Vec<String>,
    },
    /// Ask a provider to generate a sequence of Haskell workflow scripts.
    Generate {
        /// Natural-language workflow goal.
        #[arg(required = true, num_args = 1..)]
        goal: Vec<String>,
        /// Provider registry key; defaults to `default_provider`.
        #[arg(long)]
        provider: Option<String>,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Provider invocation deadline.
        #[arg(long, default_value_t = 1800)]
        timeout_seconds: u64,
        /// Emit one terminal JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Invoke any registered provider, effect, or generic plugin.
    PluginCall {
        /// Registry key.
        name: String,
        /// Plugin-defined method.
        method: String,
        /// JSON object delivered as params.
        #[arg(long, default_value = "{}")]
        params: String,
        /// Registry category. Auto succeeds only for an unambiguous name.
        #[arg(long, value_enum, default_value = "auto")]
        namespace: NamespaceArgument,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Wall-clock deadline in seconds; zero disables it.
        #[arg(long, default_value_t = 1800)]
        timeout_seconds: u64,
        /// Emit one terminal JSON report on stdout.
        #[arg(long)]
        json: bool,
    },
    /// Supervise one configured plugin while preserving the plugin-v1 stream.
    #[command(hide = true)]
    Dispatch {
        /// Exact registry category.
        #[arg(long, value_enum)]
        namespace: NamespaceArgument,
        /// Registry key.
        #[arg(long)]
        name: String,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Wall-clock deadline in seconds; zero disables it.
        #[arg(long, default_value_t = 1800)]
        timeout_seconds: u64,
    },
    /// Call the `smoke` method on selected unambiguous plugins.
    Smoke {
        /// Plugin names. With none, checks all generic `[plugins]` entries.
        names: Vec<String>,
        /// Allow adapters to perform a live external probe.
        #[arg(long)]
        live: bool,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Emit a JSON array.
        #[arg(long)]
        json: bool,
    },
    /// Machine-readable, redacted projections for Motivo Studio.
    Studio {
        /// Read-only Studio query.
        #[command(subcommand)]
        command: StudioCommand,
    },
    /// Built-in adapter from plugin-v1 requests to a native agent CLI.
    #[command(hide = true)]
    ProviderHost {
        /// `codex`, `claude-code`, or `opencode`.
        kind: String,
    },
    /// Built-in observational effect host.
    #[command(hide = true)]
    EffectHost {
        /// Currently only `workspace-paths`.
        kind: String,
    },
}

#[derive(Debug, Subcommand)]
enum StudioCommand {
    /// Inspect scripts, health, registries, and recent invocation traces.
    Inspect {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Maximum number of recent runs to project.
        #[arg(long, default_value_t = 50)]
        run_limit: usize,
    },
    /// Read a bounded page of one invocation trace.
    Events {
        /// Opaque run identifier returned by `studio inspect`.
        run_id: String,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Return events whose sequence is greater than this value.
        #[arg(long, default_value_t = 0)]
        after: u64,
        /// Maximum number of events in the page.
        #[arg(long, default_value_t = 250)]
        limit: usize,
        /// Maximum event bytes scanned for this page.
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
    },
}

/// CLI form of a plugin registry category.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum NamespaceArgument {
    /// Require an unambiguous match across registries.
    Auto,
    /// Generic plugin registry.
    Plugin,
    /// Provider registry.
    Provider,
    /// Effect registry.
    Effect,
}

impl From<NamespaceArgument> for PluginNamespace {
    fn from(value: NamespaceArgument) -> Self {
        match value {
            NamespaceArgument::Auto => Self::Auto,
            NamespaceArgument::Plugin => Self::Plugin,
            NamespaceArgument::Provider => Self::Provider,
            NamespaceArgument::Effect => Self::Effect,
        }
    }
}

/// Parse process arguments, execute the command, and return its exit code.
pub fn run() -> Result<i32, CliError> {
    run_with(Arguments::parse())
}

/// Execute already parsed arguments. Kept separate for embedding and tests.
pub fn run_with(arguments: Arguments) -> Result<i32, CliError> {
    match arguments.command {
        Command::Init { root, sdk, json } => initialize(&root, sdk.as_deref(), json),
        Command::List { root, json } => list(&root, json),
        Command::Prompt { root } => prompt(&root),
        Command::Doctor { root, json } => diagnose(&root, json),
        Command::RuntimeJson { root } => runtime_json(&root),
        Command::Check {
            scripts,
            keep_going,
            root,
            timeout_seconds,
            packages,
        } => check(&root, &scripts, &packages, keep_going, timeout_seconds),
        Command::Run {
            scripts,
            keep_going,
            root,
            timeout_seconds,
            packages,
            arguments,
        } => run_scripts_command(
            &root,
            &scripts,
            &packages,
            &arguments,
            keep_going,
            timeout_seconds,
        ),
        Command::Generate {
            goal,
            provider,
            root,
            timeout_seconds,
            json,
        } => generate(
            &root,
            &goal.join(" "),
            provider.as_deref(),
            timeout_seconds,
            json,
        ),
        Command::PluginCall {
            name,
            method,
            params,
            namespace,
            root,
            timeout_seconds,
            json,
        } => plugin_call(
            &root,
            &name,
            &method,
            &params,
            namespace.into(),
            timeout_seconds,
            json,
        ),
        Command::Dispatch {
            namespace,
            name,
            root,
            timeout_seconds,
        } => dispatch(&root, &name, namespace.into(), timeout_seconds),
        Command::Smoke {
            names,
            live,
            root,
            json,
        } => smoke(&root, &names, live, json),
        Command::Studio { command } => studio(command),
        Command::ProviderHost { kind } => Ok(run_provider_host(
            &kind,
            io::stdin().lock(),
            io::stdout().lock(),
            io::stderr().lock(),
        )),
        Command::EffectHost { kind } => {
            if kind != "workspace-paths" {
                return Err(CliError::InvalidArguments(format!(
                    "unknown built-in effect host {kind:?}"
                )));
            }
            Ok(run_workspace_paths_host(
                io::stdin().lock(),
                io::stdout().lock(),
                io::stderr().lock(),
            ))
        }
    }
}

fn initialize(root: &Path, sdk: Option<&Path>, json: bool) -> Result<i32, CliError> {
    let report = initialize_workspace(root, sdk)?;
    if json {
        print_json(&serde_json::json!({
            "workspace": report.workspace.root,
            "clef_sdk": report.clef_sdk,
            "created": report.created,
            "preserved": report.preserved,
        }))?;
    } else {
        render_presentation(&Presentation::new(
            PresentationCategory::State,
            format!(
                "Tactus workspace is initialized at {}.",
                report.workspace.root.display()
            ),
        ));
        for path in report.created {
            render_presentation(&Presentation::new(
                PresentationCategory::Info,
                format!("Created {path}."),
            ));
        }
        for path in report.preserved {
            render_presentation(&Presentation::new(
                PresentationCategory::Info,
                format!("Preserved {path}."),
            ));
        }
    }
    Ok(0)
}

fn list(start: &Path, json: bool) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let scripts = discover_scripts(&workspace)?;
    if json {
        print_json(&serde_json::json!({
            "workspace": workspace.root,
            "scripts": scripts,
        }))?;
    } else {
        for script in scripts {
            let kind = if script.runnable { "entry " } else { "helper" };
            let order = script
                .order
                .map_or_else(|| "---".to_owned(), |value| format!("{value:03}"));
            render_presentation(&Presentation::new(
                PresentationCategory::Info,
                format!("{order} {kind}: {}", script.relative_path),
            ));
        }
    }
    Ok(0)
}

fn prompt(start: &Path) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let config = workspace.load_config()?;
    let value = workspace.read_prompt(&config)?;
    print!("{value}");
    if !value.ends_with('\n') {
        println!();
    }
    Ok(0)
}

fn diagnose(start: &Path, json: bool) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let checks = doctor(&workspace);
    let healthy = checks.iter().all(|check| check.ok);
    if json {
        print_json(&serde_json::json!({
            "workspace": workspace.root,
            "ok": healthy,
            "checks": checks,
        }))?;
    } else {
        for check in checks {
            render_presentation(&Presentation::new(
                if check.ok {
                    PresentationCategory::Info
                } else {
                    PresentationCategory::Error
                },
                format!("{}: {}", check.name, check.detail),
            ));
        }
    }
    Ok(if healthy { 0 } else { 1 })
}

fn studio(command: StudioCommand) -> Result<i32, CliError> {
    match command {
        StudioCommand::Inspect { root, run_limit } => {
            match crate::studio::inspect(&root, run_limit) {
                Ok(snapshot) => {
                    print_json(&ControlSuccess::new("studio.inspect", snapshot))?;
                    Ok(0)
                }
                Err(error) => {
                    print_json(&ControlFailure::new("studio.inspect", &error))?;
                    Ok(1)
                }
            }
        }
        StudioCommand::Events {
            run_id,
            root,
            after,
            limit,
            max_bytes,
        } => match crate::studio::events(&root, &run_id, after, limit, max_bytes) {
            Ok(events) => {
                print_json(&ControlSuccess::new("studio.events", events))?;
                Ok(0)
            }
            Err(error) => {
                print_json(&ControlFailure::new("studio.events", &error))?;
                Ok(1)
            }
        },
    }
}

fn runtime_json(start: &Path) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    print_json(&runtime_document(&workspace)?)?;
    Ok(0)
}

fn check(
    start: &Path,
    explicit: &[PathBuf],
    additional_packages: &[String],
    keep_going: bool,
    timeout_seconds: u64,
) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let cancellation = install_cancellation()?;
    let scripts = select_scripts(&workspace, explicit, false)?;
    if scripts.is_empty() {
        return Err(CliError::InvalidArguments(
            "no Haskell scripts were selected for checking".to_owned(),
        ));
    }
    let tool_runtime = ToolRuntime::create(&workspace)?;
    let environment = &tool_runtime.environment;
    let project = workspace.control.display().to_string();
    let packages = haskell_packages(additional_packages);
    let mut build_command = vec![
        "cabal".to_owned(),
        "build".to_owned(),
        "--project-dir".to_owned(),
        project.clone(),
    ];
    build_command.extend(packages.iter().map(|package| format!("lib:{package}")));
    let build = execute_tool(
        &workspace,
        build_command,
        environment,
        timeout_seconds,
        &cancellation,
    )?;
    if build != 0 {
        return Ok(build);
    }
    let include = format!("-i{}", workspace.scripts_path.display());
    let mut first_failure = 0;
    for script in scripts {
        let mut command = vec![
            "cabal".to_owned(),
            "exec".to_owned(),
            "--project-dir".to_owned(),
            project.clone(),
            "--".to_owned(),
            "ghc".to_owned(),
            "-fno-code".to_owned(),
        ];
        for package in &packages {
            command.push("-package".to_owned());
            command.push(package.clone());
        }
        command.extend([include.clone(), script.display().to_string()]);
        let status = execute_tool(
            &workspace,
            command,
            environment,
            timeout_seconds,
            &cancellation,
        )?;
        if status != 0 {
            first_failure = first_failure.max(status);
            if !keep_going {
                break;
            }
        }
    }
    Ok(first_failure)
}

fn run_scripts_command(
    start: &Path,
    explicit: &[PathBuf],
    additional_packages: &[String],
    arguments: &[String],
    keep_going: bool,
    timeout_seconds: u64,
) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let cancellation = install_cancellation()?;
    let scripts = select_scripts(&workspace, explicit, true)?;
    if scripts.is_empty() {
        return Err(CliError::InvalidArguments(
            "no numbered Haskell entry scripts were found".to_owned(),
        ));
    }
    let started = std::time::Instant::now();
    let (mut journal, journal_create_error) = create_journal_preserving_execution(&workspace);
    let run_id = journal.run_id().to_owned();
    let started_presentation = Presentation::new(
        PresentationCategory::State,
        format!(
            "Script batch {run_id} is preparing {} entries.",
            scripts.len()
        ),
    );
    let mut journal_degradation = journal_create_error.map(|error| error.to_string());
    if journal_degradation.is_none()
        && let Err(error) = journal.record_transition(
            "ready",
            TransitionTrigger::new(TriggerKind::Request, "tactus.run", "script_batch.requested")
                .with_details(serde_json::json!({
                    "run_id":run_id,
                    "script_count":scripts.len(),
                    "keep_going":keep_going,
                    "argument_count":arguments.len(),
                })),
            TransitionGuard::new(
                "at least one runnable Haskell entry was selected",
                true,
                "Script discovery and explicit path validation produced a non-empty batch.",
            ),
            "preparing",
            started_presentation.clone(),
        )
    {
        note_journal_degradation(&mut journal_degradation, error);
    }
    render_presentation(&started_presentation);

    let running_presentation = Presentation::new(
        PresentationCategory::State,
        format!("Script batch {run_id} finished preparation and started running."),
    );
    let mut reached_running = false;
    let mut clef_observation_error = None;
    let result = match ToolRuntime::create(&workspace) {
        Ok(mut tool_runtime) => {
            tool_runtime
                .environment
                .insert("TACTUS_RUN_ID".to_owned(), run_id.clone());
            execute_script_batch(
                &workspace,
                &scripts,
                additional_packages,
                arguments,
                keep_going,
                timeout_seconds,
                &cancellation,
                &tool_runtime.environment,
                tool_runtime.directory.path(),
                |observation| match observation {
                    ScriptBatchObservation::Prepared => {
                        reached_running = true;
                        if journal_degradation.is_none()
                            && let Err(error) = journal.record_transition(
                                "preparing",
                                TransitionTrigger::new(
                                    TriggerKind::InternalResult,
                                    "tactus.cabal",
                                    "script_batch.build_succeeded",
                                )
                                .with_details(serde_json::json!({
                                    "run_id":run_id,
                                    "script_count":scripts.len(),
                                })),
                                TransitionGuard::new(
                                    "Clef and requested Haskell packages built successfully",
                                    true,
                                    "Cabal completed the batch preparation command with exit code zero.",
                                ),
                                "running",
                                running_presentation.clone(),
                            )
                        {
                            note_journal_degradation(&mut journal_degradation, error);
                        }
                        render_presentation(&running_presentation);
                    }
                    ScriptBatchObservation::Diagnostics(diagnostic_path) => {
                        if journal_degradation.is_some() {
                            return;
                        }
                        match import_clef_diagnostic_sidecar(&mut journal, diagnostic_path) {
                            Ok(_) => {}
                            Err(ClefSidecarError::Journal(error)) => {
                                note_journal_degradation(&mut journal_degradation, error);
                            }
                            Err(error) => {
                                note_observation_degradation(&mut clef_observation_error, error);
                            }
                        }
                    }
                },
            )
        }
        Err(error) => Err(error),
    };
    let mut outcome = script_batch_outcome(&result, scripts.len(), started.elapsed());
    outcome.observation_error = clef_observation_error.clone();
    let state_after = if outcome.kind == InvocationKind::Succeeded {
        "succeeded"
    } else {
        "failed"
    };
    let terminal_presentation = Presentation::new(
        PresentationCategory::State,
        if state_after == "succeeded" {
            format!("Script batch {run_id} succeeded.")
        } else {
            format!("Script batch {run_id} entered the failed state.")
        },
    );
    if journal_degradation.is_none()
        && let Some(error) = clef_observation_error.as_deref()
        && let Err(journal_error) = journal.record_with_presentation(
            "runtime.observer_degraded",
            serde_json::json!({
                "source":"clef.sidecar",
                "diagnostic":error,
            }),
            Some(Presentation::new(
                PresentationCategory::Warning,
                "Some Clef diagnostics could not be imported; the script result was preserved.",
            )),
        )
    {
        note_journal_degradation(&mut journal_degradation, journal_error);
    }
    if journal_degradation.is_none()
        && let Err(error) = journal.record_transition(
            if reached_running {
                "running"
            } else {
                "preparing"
            },
            TransitionTrigger::new(
                TriggerKind::InternalResult,
                "tactus.script_batch",
                if state_after == "succeeded" {
                    "script_batch.completed"
                } else {
                    "script_batch.failed"
                },
            )
            .with_details(serde_json::json!({
                "run_id":run_id,
                "script_count":scripts.len(),
                "exit_code":outcome.exit_code,
                "outcome_kind":outcome_kind_name(outcome.kind),
                "diagnostic":outcome.error.as_deref(),
                "observation_error":outcome.observation_error.as_deref(),
            })),
            TransitionGuard::new(
                "script batch result classified",
                true,
                if state_after == "succeeded" {
                    "The build and every selected script completed with exit code zero."
                } else {
                    "The build, a selected script, or its command supervisor reported failure."
                },
            ),
            state_after,
            terminal_presentation.clone(),
        )
    {
        note_journal_degradation(&mut journal_degradation, error);
    }
    let (_, final_degradation) =
        finish_journal_preserving_outcome(&mut journal, outcome, journal_degradation);
    render_presentation(&terminal_presentation);
    if clef_observation_error.is_some() {
        render_presentation(&Presentation::new(
            PresentationCategory::Warning,
            "Some Clef diagnostics could not be imported; the script result was preserved.",
        ));
    }
    if let Some(diagnostic) = final_degradation.as_deref() {
        render_journal_degradation(diagnostic);
    }
    result
}

enum ScriptBatchObservation<'a> {
    Prepared,
    Diagnostics(&'a Path),
}

#[allow(clippy::too_many_arguments)]
fn execute_script_batch(
    workspace: &Workspace,
    scripts: &[PathBuf],
    additional_packages: &[String],
    arguments: &[String],
    keep_going: bool,
    timeout_seconds: u64,
    cancellation: &CancellationToken,
    environment: &BTreeMap<String, String>,
    diagnostic_directory: &Path,
    mut on_observation: impl FnMut(ScriptBatchObservation<'_>),
) -> Result<i32, CliError> {
    let project = workspace.control.display().to_string();
    let packages = haskell_packages(additional_packages);
    let mut build_command = vec![
        "cabal".to_owned(),
        "build".to_owned(),
        "--project-dir".to_owned(),
        project.clone(),
    ];
    build_command.extend(packages.iter().map(|package| format!("lib:{package}")));
    let build = execute_tool(
        workspace,
        build_command,
        environment,
        timeout_seconds,
        cancellation,
    )?;
    if build != 0 {
        return Ok(build);
    }
    on_observation(ScriptBatchObservation::Prepared);
    let include = format!("-i{}", workspace.scripts_path.display());
    let mut first_failure = 0;
    for (index, script) in scripts.iter().enumerate() {
        let mut command = vec![
            "cabal".to_owned(),
            "exec".to_owned(),
            "--project-dir".to_owned(),
            project.clone(),
            "--".to_owned(),
            "runghc".to_owned(),
        ];
        command.extend(
            packages
                .iter()
                .map(|package| format!("--ghc-arg=-package={package}")),
        );
        command.extend([format!("--ghc-arg={include}"), script.display().to_string()]);
        command.extend_from_slice(arguments);
        let diagnostic_path = diagnostic_directory.join(format!(
            "clef-script-{index:04}-{}.jsonl",
            std::process::id()
        ));
        let mut script_environment = environment.clone();
        script_environment.insert(
            "TACTUS_DIAGNOSTIC_PATH".to_owned(),
            diagnostic_path.display().to_string(),
        );
        let status = execute_tool(
            workspace,
            command,
            &script_environment,
            timeout_seconds,
            cancellation,
        );
        on_observation(ScriptBatchObservation::Diagnostics(&diagnostic_path));
        let status = status?;
        if status != 0 {
            first_failure = first_failure.max(status);
            if !keep_going {
                break;
            }
        }
    }
    Ok(first_failure)
}

fn script_batch_outcome(
    result: &Result<i32, CliError>,
    script_count: usize,
    elapsed: Duration,
) -> ProcessOutcome {
    let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(0) => ProcessOutcome {
            kind: InvocationKind::Succeeded,
            exit_code: Some(0),
            terminal: Some(TerminalResult::Success {
                value: serde_json::json!({"script_count":script_count,"exit_code":0}),
            }),
            frames_seen: 0,
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: None,
            elapsed_ms,
        },
        Ok(exit_code) => ProcessOutcome {
            kind: InvocationKind::PluginFailed,
            exit_code: Some(*exit_code),
            terminal: Some(TerminalResult::Failure {
                error: PluginFailure {
                    code: "script_batch_failed".to_owned(),
                    message: format!("script batch exited with code {exit_code}"),
                    details: Some(serde_json::json!({"script_count":script_count})),
                },
            }),
            frames_seen: 0,
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: None,
            elapsed_ms,
        },
        Err(error) => ProcessOutcome {
            kind: InvocationKind::RuntimeFailed,
            exit_code: None,
            terminal: None,
            frames_seen: 0,
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: Some(error.to_string()),
            elapsed_ms,
        },
    }
}

fn generate(
    start: &Path,
    goal: &str,
    selected_provider: Option<&str>,
    timeout_seconds: u64,
    json: bool,
) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let cancellation = install_cancellation()?;
    let config = workspace.load_config()?;
    let provider_name = selected_provider.unwrap_or(&config.default_provider);
    config
        .providers
        .get(provider_name)
        .ok_or_else(|| WorkspaceError::UnknownPlugin(provider_name.to_owned()))?;
    let instructions = workspace.read_prompt(&config)?;
    let skill = workspace.read_tactus_skill()?;
    let generation_prompt = format!(
        "{instructions}\n\n# Tactus agent skill\n\n{skill}\n\n# Generation goal\n\n{goal}\n\nCreate a multi-step workflow: begin with the smallest atomic scripts, then compose them into a final complete program. Only write DSL files; do not build or run them.\n"
    );
    let mut params = Map::new();
    params.insert("prompt".to_owned(), Value::String(generation_prompt));
    params.insert(
        "workspace".to_owned(),
        Value::String(workspace.root.display().to_string()),
    );
    let script_baseline = script_fingerprints(&workspace)?;

    let (mut generation_journal, journal_create_error) =
        create_journal_preserving_execution(&workspace);
    let generation_path = generation_journal.run_path().to_path_buf();
    let invocation = generation_journal.run_id().to_owned();
    let mut generation_journal_degradation = journal_create_error.map(|error| error.to_string());
    let context = serde_json::json!({
        "source": "tactus.generate",
        "provider": provider_name,
        "goal": goal,
    });
    if generation_journal_degradation.is_none()
        && let Err(error) = generation_journal.record_transition(
            "ready",
            TransitionTrigger::new(
                TriggerKind::Request,
                "tactus.generate",
                "workflow.generation_requested",
            )
            .with_details(serde_json::json!({"provider": provider_name, "goal": goal})),
            TransitionGuard::new(
                "selected provider exists in the workspace registry",
                true,
                "The selected provider was resolved before generation started.",
            ),
            "running",
            Presentation::new(
                PresentationCategory::State,
                format!("Workflow generation with {provider_name} started."),
            ),
        )
    {
        note_journal_degradation(&mut generation_journal_degradation, error);
    }
    let observer_context = ObserverContext {
        workspace: &workspace,
        invocation: &invocation,
        context: &context,
        timeout_seconds,
        console_events: !json,
    };
    let mut active = Vec::new();
    let mut evidence = Vec::new();
    let mut observer_errors = Vec::new();
    let mut provider_error: Option<String> = None;

    for (effect_name, effect) in &config.effects {
        if !effect.observe_invocations {
            continue;
        }
        let begin_params = observer_params(&observer_context, &effect.options, None, None)?;
        match invoke_registered(
            &workspace,
            effect_name,
            "observe.begin",
            begin_params,
            InvocationControl {
                namespace: PluginNamespace::Effect,
                timeout_seconds,
                console_events: !json,
                cancellation: &cancellation,
            },
        ) {
            Ok(report) => {
                record_journal_event(
                    &mut generation_journal,
                    &mut generation_journal_degradation,
                    "observer.begin",
                    report.diagnostic_value(),
                );
                let begin = successful_value(&report);
                let succeeded = begin.is_some();
                evidence.push(ObserverEvidence {
                    effect: effect_name.clone(),
                    phase: "observe.begin".to_owned(),
                    report: Some(report),
                    error: None,
                });
                if let Some(begin) = begin {
                    active.push(ActiveObserver {
                        name: effect_name.clone(),
                        options: effect.options.clone(),
                        begin,
                    });
                } else if !succeeded {
                    provider_error = Some(format!("effect {effect_name:?} observe.begin failed"));
                    break;
                }
            }
            Err(error) => {
                let message = error.to_string();
                record_journal_event(
                    &mut generation_journal,
                    &mut generation_journal_degradation,
                    "observer.begin_error",
                    serde_json::json!({"effect": effect_name, "error": message}),
                );
                evidence.push(ObserverEvidence {
                    effect: effect_name.clone(),
                    phase: "observe.begin".to_owned(),
                    report: None,
                    error: Some(message.clone()),
                });
                provider_error = Some(message);
                break;
            }
        }
    }

    let mut provider_report = None;
    if provider_error.is_none() {
        match invoke_registered(
            &workspace,
            provider_name,
            "invoke",
            params,
            InvocationControl {
                namespace: PluginNamespace::Provider,
                timeout_seconds,
                console_events: !json,
                cancellation: &cancellation,
            },
        ) {
            Ok(report) => {
                record_journal_event(
                    &mut generation_journal,
                    &mut generation_journal_degradation,
                    "provider.completed",
                    report.diagnostic_value(),
                );
                provider_report = Some(report);
            }
            Err(error) => {
                let message = error.to_string();
                record_journal_event(
                    &mut generation_journal,
                    &mut generation_journal_degradation,
                    "provider.error",
                    serde_json::json!({"provider": provider_name, "error": message}),
                );
                provider_error = Some(message);
            }
        }
    }
    let observer_outcome = provider_report.as_ref().map_or_else(
        || {
            if provider_error
                .as_deref()
                .is_some_and(|error| error.contains("observe.begin"))
            {
                "begin_error"
            } else {
                "outcome_unknown"
            }
        },
        |report| match report.summary.outcome.kind {
            InvocationKind::Succeeded => "ok",
            InvocationKind::PluginFailed => "error",
            InvocationKind::ProcessFailed
            | InvocationKind::ProtocolFailed
            | InvocationKind::RuntimeFailed
            | InvocationKind::DeadlineExceeded
            | InvocationKind::Cancelled => "outcome_unknown",
        },
    );
    end_observers(
        &observer_context,
        &mut active,
        observer_outcome,
        &mut evidence,
        &mut observer_errors,
        &mut generation_journal,
        &mut generation_journal_degradation,
    )?;
    let scripts = discover_scripts(&workspace)?;
    let generated_delta = generated_script_delta(&script_baseline, &scripts)?;
    record_journal_event(
        &mut generation_journal,
        &mut generation_journal_degradation,
        "generation.discovered_scripts",
        serde_json::json!({"scripts":scripts, "generated_delta":generated_delta}),
    );
    let provider_ok = provider_report
        .as_ref()
        .is_some_and(|report| report.summary.outcome.is_success())
        && provider_error.is_none();
    let generation_error = (provider_ok && generated_delta.is_empty()).then(|| {
        "provider completed successfully but created or modified no non-empty numbered Haskell entry"
            .to_owned()
    });
    let success = provider_ok && generation_error.is_none() && observer_errors.is_empty();
    let mut generation_outcome = provider_report.as_ref().map_or_else(
        || {
            synthetic_runtime_failure(
                provider_error
                    .as_deref()
                    .unwrap_or("provider was not invoked"),
            )
        },
        |report| report.summary.outcome.clone(),
    );
    if !observer_errors.is_empty() {
        generation_outcome = synthetic_runtime_failure(&format!(
            "observer cleanup failed: {}",
            observer_errors.join("; ")
        ));
    } else if let Some(error) = generation_error.as_deref() {
        generation_outcome = synthetic_runtime_failure(error);
    }
    let generation_state = if success {
        "succeeded"
    } else if provider_report
        .as_ref()
        .is_some_and(|report| outcome_is_unknown(report.summary.outcome.kind))
    {
        "outcome_unknown"
    } else {
        "failed"
    };
    if generation_journal_degradation.is_none()
        && let Err(error) = generation_journal.record_transition(
            "running",
            TransitionTrigger::new(
                TriggerKind::InternalResult,
                "tactus.generate",
                "workflow.generation_completed",
            )
            .with_details(serde_json::json!({
                "provider":provider_name,
                "provider_ok":provider_ok,
                "generated_script_count":generated_delta.len(),
                "observer_error_count":observer_errors.len(),
                "generation_error":generation_error.as_deref(),
            })),
            TransitionGuard::new(
                "generation outcome classified",
                true,
                "Provider, observer, and generated-script results were evaluated.",
            ),
            generation_state,
            Presentation::new(
                PresentationCategory::State,
                match generation_state {
                    "succeeded" => "Workflow generation succeeded.".to_owned(),
                    "failed" => "Workflow generation entered the failed state.".to_owned(),
                    _ => "Workflow generation entered the outcome-unknown state.".to_owned(),
                },
            ),
        )
    {
        note_journal_degradation(&mut generation_journal_degradation, error);
    }
    if generation_state == "outcome_unknown"
        && generation_journal_degradation.is_none()
        && let Err(error) = generation_journal.record_with_presentation(
            "runtime.outcome_unknown",
            serde_json::json!({
                "provider":provider_name,
                "reason":"provider execution did not yield a provable terminal result",
            }),
            Some(Presentation::new(
                PresentationCategory::Warning,
                "The provider may have changed the workspace; Tactus did not retry it automatically.",
            )),
        )
    {
        note_journal_degradation(&mut generation_journal_degradation, error);
    }
    let generation_persisted = generation_journal.is_durable();
    let (generation_summary, final_journal_degradation) = finish_journal_preserving_outcome(
        &mut generation_journal,
        generation_outcome,
        generation_journal_degradation,
    );
    let generation_report = InvocationReport {
        name: "generate".to_owned(),
        run_path: generation_path,
        summary: generation_summary,
        persisted: generation_persisted && final_journal_degradation.is_none(),
    };
    if json {
        print_json(&serde_json::json!({
            "provider": provider_name,
            "ok": success,
            "provider_ok": provider_ok,
            "generated_delta": generated_delta,
            "generation": generation_report,
            "provider_run": provider_report,
            "effects": evidence,
            "observer_errors": observer_errors,
            "error": provider_error,
            "generation_error": generation_error,
            "scripts": scripts,
        }))?;
    } else {
        render_presentation(&Presentation::new(
            PresentationCategory::State,
            if success {
                format!("Workflow generation with {provider_name} succeeded.")
            } else {
                format!("Workflow generation with {provider_name} did not succeed.")
            },
        ));
        if let Some(error) = provider_error.as_deref() {
            render_presentation(&Presentation::new(PresentationCategory::Error, error));
        }
        if let Some(error) = generation_error.as_deref() {
            render_presentation(&Presentation::new(PresentationCategory::Error, error));
        }
        for error in observer_errors {
            render_presentation(&Presentation::new(
                PresentationCategory::Warning,
                format!("An invocation observer failed: {error}"),
            ));
        }
        render_presentation(&Presentation::new(
            PresentationCategory::Info,
            format!(
                "Tactus found {} Haskell sources; {} numbered entries were created or changed. Run `tactus list` for file details.",
                scripts.len(),
                generated_delta.len()
            ),
        ));
    }
    if let Some(diagnostic) = final_journal_degradation.as_deref() {
        render_journal_degradation(diagnostic);
    }
    Ok(if success { 0 } else { 1 })
}

fn script_fingerprints(workspace: &Workspace) -> Result<BTreeMap<String, String>, CliError> {
    let scripts = discover_scripts(workspace)?;
    scripts
        .into_iter()
        .filter(|script| script.runnable)
        .map(|script| {
            let bytes = fs::read(&script.path)?;
            Ok((script.relative_path, format!("{:x}", Sha256::digest(bytes))))
        })
        .collect()
}

fn generated_script_delta(
    before: &BTreeMap<String, String>,
    scripts: &[ScriptInfo],
) -> Result<Vec<String>, CliError> {
    let mut delta = Vec::new();
    for script in scripts.iter().filter(|script| script.runnable) {
        let bytes = fs::read(&script.path)?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let fingerprint = format!("{:x}", Sha256::digest(&bytes));
        if before.get(&script.relative_path) != Some(&fingerprint) {
            delta.push(script.relative_path.clone());
        }
    }
    Ok(delta)
}

#[derive(Debug, Serialize)]
struct ObserverEvidence {
    effect: String,
    phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    report: Option<InvocationReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

struct ActiveObserver {
    name: String,
    options: BTreeMap<String, Value>,
    begin: Value,
}

struct ObserverContext<'a> {
    workspace: &'a Workspace,
    invocation: &'a str,
    context: &'a Value,
    timeout_seconds: u64,
    console_events: bool,
}

fn observer_params(
    context: &ObserverContext<'_>,
    options: &BTreeMap<String, Value>,
    outcome: Option<&str>,
    begin: Option<&Value>,
) -> Result<Map<String, Value>, CliError> {
    let mut params = Map::new();
    params.insert(
        "workspace".to_owned(),
        Value::String(context.workspace.root.display().to_string()),
    );
    params.insert("options".to_owned(), serde_json::to_value(options)?);
    params.insert(
        "invocation".to_owned(),
        Value::String(context.invocation.to_owned()),
    );
    params.insert("context".to_owned(), context.context.clone());
    if let Some(outcome) = outcome {
        params.insert("outcome".to_owned(), Value::String(outcome.to_owned()));
    }
    if let Some(begin) = begin {
        params.insert("begin".to_owned(), begin.clone());
    }
    Ok(params)
}

fn successful_value(report: &InvocationReport) -> Option<Value> {
    if !report.summary.outcome.is_success() {
        return None;
    }
    match &report.summary.outcome.terminal {
        Some(TerminalResult::Success { value }) => Some(value.clone()),
        Some(TerminalResult::Failure { .. }) | None => None,
    }
}

fn end_observers(
    context: &ObserverContext<'_>,
    active: &mut Vec<ActiveObserver>,
    outcome: &str,
    evidence: &mut Vec<ObserverEvidence>,
    errors: &mut Vec<String>,
    journal: &mut RunJournal,
    journal_degradation: &mut Option<String>,
) -> Result<(), CliError> {
    while let Some(observer) = active.pop() {
        let cleanup_cancellation = CancellationToken::new();
        let cleanup_timeout = if context.timeout_seconds == 0 {
            30
        } else {
            context.timeout_seconds.min(30)
        };
        let params = observer_params(
            context,
            &observer.options,
            Some(outcome),
            Some(&observer.begin),
        )?;
        match invoke_registered(
            context.workspace,
            &observer.name,
            "observe.end",
            params,
            InvocationControl {
                namespace: PluginNamespace::Effect,
                timeout_seconds: cleanup_timeout,
                console_events: context.console_events,
                cancellation: &cleanup_cancellation,
            },
        ) {
            Ok(report) => {
                let succeeded = report.summary.outcome.is_success();
                record_journal_event(
                    journal,
                    journal_degradation,
                    "observer.end",
                    report.diagnostic_value(),
                );
                if !succeeded {
                    errors.push(format!("effect {:?} observe.end failed", observer.name));
                }
                evidence.push(ObserverEvidence {
                    effect: observer.name,
                    phase: "observe.end".to_owned(),
                    report: Some(report),
                    error: None,
                });
            }
            Err(error) => {
                let message = error.to_string();
                record_journal_event(
                    journal,
                    journal_degradation,
                    "observer.end_error",
                    serde_json::json!({"effect": observer.name, "error": message}),
                );
                errors.push(message.clone());
                evidence.push(ObserverEvidence {
                    effect: observer.name,
                    phase: "observe.end".to_owned(),
                    report: None,
                    error: Some(message),
                });
            }
        }
    }
    Ok(())
}

fn synthetic_runtime_failure(message: &str) -> ProcessOutcome {
    ProcessOutcome {
        kind: InvocationKind::RuntimeFailed,
        exit_code: None,
        terminal: None,
        frames_seen: 0,
        events_dropped: 0,
        observation_error: None,
        stderr: String::new(),
        stderr_truncated: false,
        error: Some(message.to_owned()),
        elapsed_ms: 0,
    }
}

fn select_scripts(
    workspace: &Workspace,
    explicit: &[PathBuf],
    entries_only: bool,
) -> Result<Vec<PathBuf>, CliError> {
    if !explicit.is_empty() {
        return explicit
            .iter()
            .map(|value| {
                let candidate = if value.is_absolute() {
                    value.clone()
                } else {
                    workspace.root.join(value)
                };
                let resolved = dunce::canonicalize(candidate)?;
                let is_haskell = resolved.is_file()
                    && resolved
                        .extension()
                        .and_then(|suffix| suffix.to_str())
                        .is_some_and(|suffix| {
                            suffix.eq_ignore_ascii_case("hs") || suffix.eq_ignore_ascii_case("lhs")
                        });
                if !is_haskell {
                    return Err(CliError::InvalidArguments(format!(
                        "not a Haskell source: {}",
                        resolved.display()
                    )));
                }
                Ok(resolved)
            })
            .collect();
    }
    Ok(discover_scripts(workspace)?
        .into_iter()
        .filter(|script| !entries_only || script.runnable)
        .map(|script: ScriptInfo| script.path)
        .collect())
}

struct ToolRuntime {
    _lease: fs::File,
    directory: tempfile::TempDir,
    environment: BTreeMap<String, String>,
}

const TOOL_RUNTIME_PREFIX: &str = "agenstro-tactus-";
const TOOL_RUNTIME_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_TOOL_RUNTIME_CANDIDATES: usize = 1_024;
const MAX_TOOL_RUNTIME_REMOVALS: usize = 64;

impl ToolRuntime {
    fn create(workspace: &Workspace) -> Result<Self, CliError> {
        let temporary_root = env::temp_dir();
        if let Some(stale_before) = SystemTime::now().checked_sub(TOOL_RUNTIME_RETENTION) {
            cleanup_stale_tool_runtimes(&temporary_root, stale_before);
        }
        let directory = tempfile::Builder::new()
            .prefix(TOOL_RUNTIME_PREFIX)
            .tempdir_in(&temporary_root)?;
        let lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(directory.path().join(".lease"))?;
        fs4::FileExt::lock(&lease)?;
        let environment = runtime_environment(workspace, &directory.path().join("runtime.json"))?;
        Ok(Self {
            _lease: lease,
            directory,
            environment,
        })
    }
}

/// Best-effort cleanup for crash leftovers. Only old, real directories with
/// the exact private prefix and a canonical parent equal to the system temp
/// directory are eligible. Failure is observational and never blocks a tool.
fn cleanup_stale_tool_runtimes(temporary_root: &Path, stale_before: SystemTime) {
    let Ok(canonical_root) = dunce::canonicalize(temporary_root) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&canonical_root) else {
        return;
    };
    let mut candidates = 0usize;
    let mut removals = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(TOOL_RUNTIME_PREFIX) {
            continue;
        }
        candidates += 1;
        if candidates > MAX_TOOL_RUNTIME_CANDIDATES || removals >= MAX_TOOL_RUNTIME_REMOVALS {
            break;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified > stale_before {
            continue;
        }
        let Ok(canonical_candidate) = dunce::canonicalize(&path) else {
            continue;
        };
        if canonical_candidate.parent() != Some(canonical_root.as_path())
            || !canonical_candidate.starts_with(&canonical_root)
        {
            continue;
        }
        let lease_path = canonical_candidate.join(".lease");
        let Ok(lease_metadata) = fs::symlink_metadata(&lease_path) else {
            continue;
        };
        if !lease_metadata.is_file() || lease_metadata.file_type().is_symlink() {
            continue;
        }
        let Ok(lease) = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lease_path)
        else {
            continue;
        };
        if fs4::FileExt::try_lock(&lease).is_err() {
            continue;
        }
        drop(lease);
        if fs::remove_dir_all(&canonical_candidate).is_ok() {
            removals += 1;
        }
    }
}

const MAX_CLEF_SIDECAR_BYTES: u64 = 16 * 1024 * 1024;
const MAX_CLEF_SIDECAR_LINE_BYTES: usize = 1024 * 1024;
const MAX_CLEF_SIDECAR_RECORDS: usize = 10_000;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ClefSidecarRecord {
    StateTransition {
        code: String,
        level: ClefPresentationLevel,
        message: String,
        subject: String,
        state_before: String,
        trigger: Box<ClefTransitionTrigger>,
        guard: Box<ClefTransitionGuard>,
        state_after: String,
        context: Map<String, Value>,
    },
    Message {
        code: String,
        level: ClefPresentationLevel,
        message: String,
        context: Map<String, Value>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum ClefPresentationLevel {
    State,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClefTransitionTrigger {
    kind: TriggerKind,
    source: String,
    code: String,
    #[serde(default)]
    details: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClefTransitionGuard {
    condition: String,
    passed: bool,
    reason: String,
}

fn is_stable_diagnostic_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'_' | b'-'))
}

#[derive(Debug, Error)]
enum ClefSidecarError {
    #[error("cannot read Clef diagnostic sidecar: {0}")]
    Io(#[source] io::Error),
    #[error("invalid Clef diagnostic sidecar: {0}")]
    Invalid(String),
    #[error(transparent)]
    Journal(#[from] JournalError),
}

fn import_clef_diagnostic_sidecar(
    journal: &mut RunJournal,
    path: &Path,
) -> Result<usize, ClefSidecarError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(ClefSidecarError::Io(error)),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ClefSidecarError::Invalid(
            "path is not a plain diagnostic file".to_owned(),
        ));
    }
    if metadata.len() > MAX_CLEF_SIDECAR_BYTES {
        return Err(ClefSidecarError::Invalid(format!(
            "file exceeds {MAX_CLEF_SIDECAR_BYTES} bytes"
        )));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(ClefSidecarError::Io)?
        .take(MAX_CLEF_SIDECAR_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(ClefSidecarError::Io)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CLEF_SIDECAR_BYTES {
        return Err(ClefSidecarError::Invalid(format!(
            "file exceeds {MAX_CLEF_SIDECAR_BYTES} bytes"
        )));
    }
    let mut records = Vec::new();
    for (index, raw_line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_CLEF_SIDECAR_LINE_BYTES {
            return Err(ClefSidecarError::Invalid(format!(
                "line {} exceeds {MAX_CLEF_SIDECAR_LINE_BYTES} bytes",
                index + 1
            )));
        }
        if records.len() >= MAX_CLEF_SIDECAR_RECORDS {
            return Err(ClefSidecarError::Invalid(format!(
                "record count exceeds {MAX_CLEF_SIDECAR_RECORDS}"
            )));
        }
        let record = serde_json::from_slice::<ClefSidecarRecord>(line).map_err(|error| {
            ClefSidecarError::Invalid(format!(
                "line {} is not valid wire JSON: {error}",
                index + 1
            ))
        })?;
        match &record {
            ClefSidecarRecord::StateTransition {
                code,
                level,
                message,
                subject,
                state_before,
                trigger,
                guard,
                state_after,
                ..
            } => {
                if *level != ClefPresentationLevel::State {
                    return Err(ClefSidecarError::Invalid(format!(
                        "line {} state_transition level must be state",
                        index + 1
                    )));
                }
                let required_text_present = [
                    code.as_str(),
                    message.as_str(),
                    subject.as_str(),
                    state_before.as_str(),
                    state_after.as_str(),
                    trigger.source.as_str(),
                    trigger.code.as_str(),
                    guard.condition.as_str(),
                    guard.reason.as_str(),
                ]
                .into_iter()
                .all(|field| !field.trim().is_empty());
                let stable_identifiers = [
                    code.as_str(),
                    subject.as_str(),
                    state_before.as_str(),
                    state_after.as_str(),
                    trigger.source.as_str(),
                    trigger.code.as_str(),
                ]
                .into_iter()
                .all(is_stable_diagnostic_identifier);
                let bounded_external_text = [
                    message.as_str(),
                    guard.condition.as_str(),
                    guard.reason.as_str(),
                ]
                .into_iter()
                .all(|field| field.len() <= 4_096);
                if !required_text_present
                    || !stable_identifiers
                    || !bounded_external_text
                    || state_before == state_after
                    || !guard.passed
                {
                    return Err(ClefSidecarError::Invalid(format!(
                        "line {} is not a committed state transition",
                        index + 1
                    )));
                }
            }
            ClefSidecarRecord::Message {
                level: ClefPresentationLevel::State,
                ..
            } => {
                return Err(ClefSidecarError::Invalid(format!(
                    "line {} message records cannot claim the state category",
                    index + 1
                )));
            }
            ClefSidecarRecord::Message { code, message, .. }
                if !is_stable_diagnostic_identifier(code)
                    || message.trim().is_empty()
                    || message.len() > 4_096 =>
            {
                return Err(ClefSidecarError::Invalid(format!(
                    "line {} message requires non-empty code and text",
                    index + 1
                )));
            }
            ClefSidecarRecord::Message { .. } => {}
        }
        records.push(record);
    }

    let mut imported = 0;
    for record in records {
        match record {
            ClefSidecarRecord::StateTransition {
                code,
                level,
                message,
                subject,
                state_before,
                trigger,
                guard,
                state_after,
                context,
            } => {
                debug_assert_eq!(level, ClefPresentationLevel::State);
                let trigger = *trigger;
                let guard = *guard;
                let evidence = diagnostic_value_summary(&serde_json::json!({
                    "message":message,
                    "subject":subject,
                    "trigger_details":trigger.details,
                    "guard_condition":guard.condition,
                    "guard_reason":guard.reason,
                    "context":context,
                }));
                let durable_message =
                    format!("Clef recorded transition {code}: {state_before} to {state_after}.");
                journal.record_transition(
                    state_before,
                    TransitionTrigger::new(trigger.kind, trigger.source, trigger.code)
                        .with_details(serde_json::json!({
                            "source":"clef.sidecar",
                            "code":code,
                            "evidence":evidence,
                        })),
                    TransitionGuard::new(
                        "Clef sidecar record passed strict validation",
                        guard.passed,
                        "The imported diagnostic asserted a committed transition.",
                    ),
                    state_after,
                    Presentation::new(PresentationCategory::State, durable_message),
                )?;
            }
            ClefSidecarRecord::Message {
                code,
                level,
                message: _,
                context,
            } => {
                let category = match level {
                    ClefPresentationLevel::State => unreachable!("validated above"),
                    ClefPresentationLevel::Info => PresentationCategory::Info,
                    ClefPresentationLevel::Warning => PresentationCategory::Warning,
                    ClefPresentationLevel::Error => PresentationCategory::Error,
                };
                let durable_message = match category {
                    PresentationCategory::Info => {
                        format!("Clef recorded diagnostic message {code}.")
                    }
                    PresentationCategory::Warning => {
                        format!("Clef recorded warning {code}; inspect live output if available.")
                    }
                    PresentationCategory::Error => {
                        format!("Clef recorded failure {code}; inspect live output if available.")
                    }
                    PresentationCategory::State => unreachable!("validated above"),
                };
                journal.record_with_presentation(
                    "runtime.message",
                    serde_json::json!({
                        "source":"clef.sidecar",
                        "code":code,
                        "context":diagnostic_value_summary(&Value::Object(context)),
                    }),
                    Some(Presentation::new(category, durable_message)),
                )?;
            }
        }
        imported += 1;
    }
    Ok(imported)
}

fn execute_tool(
    workspace: &Workspace,
    command: Vec<String>,
    environment: &BTreeMap<String, String>,
    timeout_seconds: u64,
    cancellation: &CancellationToken,
) -> Result<i32, CliError> {
    let mut spec = ProcessSpec::new(command, &workspace.root);
    spec.environment = environment.clone();
    spec.limits.deadline = (timeout_seconds != 0).then(|| Duration::from_secs(timeout_seconds));
    let outcome = ProcessSupervisor.run_command(&spec, cancellation)?;
    if outcome.kind != crate::process::CommandKind::Exited {
        render_presentation(&Presentation::new(
            PresentationCategory::Error,
            format!(
                "The supervised command ended as {:?}: {}",
                outcome.kind,
                outcome.error.as_deref().unwrap_or("no additional detail")
            ),
        ));
    }
    Ok(outcome.exit_code.unwrap_or(1))
}

#[derive(Debug, Serialize)]
struct InvocationReport {
    name: String,
    run_path: PathBuf,
    summary: RunSummary,
    persisted: bool,
}

impl InvocationReport {
    /// Project one nested invocation into durable controller diagnostics. The
    /// child journal owns the detailed result; parent traces retain only its
    /// correlation identity and the same bounded terminal summary.
    fn diagnostic_value(&self) -> Value {
        serde_json::json!({
            "name":self.name,
            "run_id":self.summary.run_id,
            "persisted":self.persisted,
            "summary":diagnostic_summary(&self.summary),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn plugin_call(
    start: &Path,
    name: &str,
    method: &str,
    params_json: &str,
    namespace: PluginNamespace,
    timeout_seconds: u64,
    json: bool,
) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let cancellation = install_cancellation()?;
    let params = parse_params(params_json)?;
    let report = invoke_registered(
        &workspace,
        name,
        method,
        params,
        InvocationControl {
            namespace,
            timeout_seconds,
            console_events: !json,
            cancellation: &cancellation,
        },
    )?;
    let success = report.summary.outcome.is_success();
    if json {
        print_json(&report)?;
    } else if report.persisted {
        render_presentation(&Presentation::new(
            PresentationCategory::Info,
            format!("Run record {} was saved for {name}.", report.summary.run_id),
        ));
    } else {
        render_presentation(&Presentation::new(
            PresentationCategory::Warning,
            format!(
                "Run record {} could not be persisted for {name}; the execution result is still authoritative.",
                report.summary.run_id
            ),
        ));
    }
    Ok(if success { 0 } else { 1 })
}

fn smoke(start: &Path, names: &[String], live: bool, json: bool) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let cancellation = install_cancellation()?;
    let config = workspace.load_config()?;
    let selected: Vec<(String, String, PluginNamespace)> = if names.is_empty() {
        config
            .providers
            .keys()
            .map(|name| {
                (
                    format!("provider:{name}"),
                    name.clone(),
                    PluginNamespace::Provider,
                )
            })
            .chain(config.effects.keys().map(|name| {
                (
                    format!("effect:{name}"),
                    name.clone(),
                    PluginNamespace::Effect,
                )
            }))
            .chain(config.plugins.keys().map(|name| {
                (
                    format!("plugin:{name}"),
                    name.clone(),
                    PluginNamespace::Plugin,
                )
            }))
            .collect()
    } else {
        names
            .iter()
            .map(|selector| parse_plugin_selector(selector))
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut reports = Vec::new();
    let mut successful = true;
    for (selector, name, namespace) in selected {
        let mut params = Map::new();
        params.insert("live".to_owned(), Value::Bool(live));
        let report = invoke_registered(
            &workspace,
            &name,
            "smoke",
            params,
            InvocationControl {
                namespace,
                timeout_seconds: 60,
                console_events: !json,
                cancellation: &cancellation,
            },
        )?;
        successful &= report.summary.outcome.is_success();
        if !json {
            render_presentation(&Presentation::new(
                PresentationCategory::Info,
                format!(
                    "Smoke check for {selector} ended as {:?}.",
                    report.summary.outcome.kind
                ),
            ));
        }
        reports.push(report);
    }
    if json {
        print_json(&reports)?;
    }
    Ok(if successful { 0 } else { 1 })
}

fn parse_plugin_selector(selector: &str) -> Result<(String, String, PluginNamespace), CliError> {
    let Some((prefix, name)) = selector.split_once(':') else {
        return Ok((
            selector.to_owned(),
            selector.to_owned(),
            PluginNamespace::Auto,
        ));
    };
    if name.is_empty() {
        return Err(CliError::InvalidArguments(format!(
            "empty plugin selector {selector:?}"
        )));
    }
    let namespace = match prefix {
        "provider" => PluginNamespace::Provider,
        "effect" => PluginNamespace::Effect,
        "plugin" => PluginNamespace::Plugin,
        _ => {
            return Err(CliError::InvalidArguments(format!(
                "unknown plugin namespace {prefix:?}"
            )));
        }
    };
    Ok((selector.to_owned(), name.to_owned(), namespace))
}

#[derive(Clone, Copy)]
struct InvocationControl<'a> {
    namespace: PluginNamespace,
    timeout_seconds: u64,
    console_events: bool,
    cancellation: &'a CancellationToken,
}

struct SharedJournal {
    journal: Option<RunJournal>,
    error: Option<JournalError>,
}

impl SharedJournal {
    fn new(journal: RunJournal) -> Self {
        Self {
            journal: Some(journal),
            error: None,
        }
    }

    fn record_frame(&mut self, subject: &str, frame: &PluginFrame) {
        if self.error.is_some() {
            return;
        }
        let Some(journal) = self.journal.as_mut() else {
            return;
        };
        let (kind, data) = diagnostic_frame(frame);
        if let Err(error) =
            journal.record_with_presentation(kind, data, presentation_for_frame(subject, frame))
        {
            self.error = Some(error);
        }
    }
}

fn take_shared_journal(
    shared: &Arc<Mutex<SharedJournal>>,
) -> Result<(RunJournal, Option<JournalError>), CliError> {
    let mut state = match shared.try_lock() {
        Ok(state) => state,
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
        Err(TryLockError::WouldBlock) => {
            return Err(CliError::EventSinkStalled(
                "run journal writer did not return before supervision ended".to_owned(),
            ));
        }
    };
    let journal = state
        .journal
        .take()
        .ok_or_else(|| CliError::EventSinkStalled("run journal was already taken".to_owned()))?;
    Ok((journal, state.error.take()))
}

fn note_journal_degradation(slot: &mut Option<String>, error: impl ToString) {
    append_bounded_diagnostic(slot, &error.to_string());
}

fn record_journal_event(
    journal: &mut RunJournal,
    degradation: &mut Option<String>,
    kind: &str,
    data: Value,
) {
    if degradation.is_none()
        && let Err(error) = journal.record(kind, data)
    {
        note_journal_degradation(degradation, error);
    }
}

fn note_observation_degradation(slot: &mut Option<String>, error: impl ToString) {
    append_bounded_diagnostic(slot, &error.to_string());
}

fn append_bounded_diagnostic(slot: &mut Option<String>, message: &str) {
    const MAX_DIAGNOSTIC_BYTES: usize = 4_096;
    let combined = slot.as_ref().map_or_else(
        || message.to_owned(),
        |existing| format!("{existing}; {message}"),
    );
    if combined.len() <= MAX_DIAGNOSTIC_BYTES {
        *slot = Some(combined);
        return;
    }
    let suffix = "…";
    let mut end = MAX_DIAGNOSTIC_BYTES.saturating_sub(suffix.len());
    while !combined.is_char_boundary(end) {
        end -= 1;
    }
    *slot = Some(format!("{}{suffix}", &combined[..end]));
}

fn preserve_journal_degradation(outcome: &mut ProcessOutcome, diagnostic: &str) {
    let message = format!("run journal degraded: {diagnostic}");
    append_bounded_diagnostic(&mut outcome.observation_error, &message);
}

fn finish_journal_preserving_outcome(
    journal: &mut RunJournal,
    mut outcome: ProcessOutcome,
    mut degradation: Option<String>,
) -> (RunSummary, Option<String>) {
    if let Some(error) = degradation.as_deref() {
        preserve_journal_degradation(&mut outcome, error);
        let fallback = journal.snapshot_summary(outcome.clone());
        return match journal.finish_degraded(outcome) {
            Ok(summary) => (summary, degradation),
            Err(error) => {
                note_journal_degradation(&mut degradation, error);
                (fallback, degradation)
            }
        };
    }

    let fallback_outcome = outcome.clone();
    match journal.finish(outcome) {
        Ok(summary) => (summary, None),
        Err(error) => {
            note_journal_degradation(&mut degradation, error);
            let mut degraded_outcome = fallback_outcome;
            preserve_journal_degradation(
                &mut degraded_outcome,
                degradation.as_deref().unwrap_or("unknown journal failure"),
            );
            let fallback = journal.snapshot_summary(degraded_outcome.clone());
            match journal.finish_degraded(degraded_outcome) {
                Ok(summary) => (summary, degradation),
                Err(error) => {
                    note_journal_degradation(&mut degradation, error);
                    (fallback, degradation)
                }
            }
        }
    }
}

fn in_memory_degraded_summary(
    run_id: &str,
    mut outcome: ProcessOutcome,
    diagnostic: &str,
) -> RunSummary {
    preserve_journal_degradation(&mut outcome, diagnostic);
    let now = current_unix_millis();
    RunSummary {
        api: crate::journal::TRACE_API.to_owned(),
        run_id: run_id.to_owned(),
        started_unix_ms: now,
        finished_unix_ms: now,
        events_recorded: 0,
        outcome,
    }
}

fn render_journal_degradation(diagnostic: &str) {
    render_presentation(&Presentation::new(
        PresentationCategory::Warning,
        format!(
            "Diagnostic persistence degraded, but the known execution result was preserved: {diagnostic}"
        ),
    ));
}

fn create_journal_preserving_execution(
    workspace: &Workspace,
) -> (RunJournal, Option<JournalError>) {
    match RunJournal::create(workspace) {
        Ok(journal) => (journal, None),
        Err(error) => (RunJournal::degraded(workspace), Some(error)),
    }
}

fn current_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn invoke_registered(
    workspace: &Workspace,
    name: &str,
    method: &str,
    mut params: Map<String, Value>,
    control: InvocationControl<'_>,
) -> Result<InvocationReport, CliError> {
    let config = workspace.load_config()?;
    let definition = workspace.resolve_plugin(&config, name, control.namespace)?;
    inject_registered_params(workspace, definition, &mut params)?;
    let registry = match definition {
        ResolvedPlugin::Plugin(_) => "plugin",
        ResolvedPlugin::Provider(_) => "provider",
        ResolvedPlugin::Effect(_) => "effect",
    };
    let command = definition.command();
    let executable = command
        .first()
        .and_then(|value| Path::new(value).file_name())
        .map_or_else(|| "<unknown>".into(), |value| value.to_string_lossy());
    let (mut journal, journal_create_error) = create_journal_preserving_execution(workspace);
    let run_path = journal.run_path().to_path_buf();
    let run_id = journal.run_id().to_owned();
    let started_presentation = Presentation::new(
        PresentationCategory::State,
        format!("{registry}:{name} method {method} started."),
    );
    let initial_journal_error = if let Some(error) = journal_create_error {
        Some(error)
    } else {
        journal
            .record_transition(
                "ready",
                TransitionTrigger::new(
                    TriggerKind::Request,
                    "tactus.cli",
                    "plugin.invocation_requested",
                )
                .with_details(serde_json::json!({
                    "plugin": name,
                    "method": method,
                    "namespace": registry,
                    "executable": executable,
                    "argument_count": command.len().saturating_sub(1),
                })),
                TransitionGuard::new(
                    "plugin resolved and request validated",
                    true,
                    "The configured plugin and invocation request passed runtime validation.",
                ),
                "running",
                started_presentation.clone(),
            )
            .err()
    };
    if control.console_events {
        render_presentation(&started_presentation);
    }
    let request = PluginRequest::new(run_id.clone(), method, params)
        .map_err(|error| CliError::InvalidArguments(error.to_string()))?;
    let tool_runtime = ToolRuntime::create(workspace)?;
    let mut spec = ProcessSpec::new(
        resolve_builtin_command(definition.command())?,
        &workspace.root,
    );
    spec.environment = tool_runtime.environment.clone();
    attach_builtin_provider_to_supervised_group(&mut spec, definition.command());
    spec.limits.deadline =
        (control.timeout_seconds != 0).then(|| Duration::from_secs(control.timeout_seconds));
    let mut shared_journal_state = SharedJournal::new(journal);
    shared_journal_state.error = initial_journal_error;
    let shared_journal = Arc::new(Mutex::new(shared_journal_state));
    let callback_journal = Arc::clone(&shared_journal);
    let display_name = name.to_owned();
    let console_events = control.console_events;
    let invocation =
        ProcessSupervisor.invoke(&spec, &request, control.cancellation, move |frame| {
            callback_journal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .record_frame(&display_name, frame);
            if console_events {
                render_frame(&display_name, frame);
            }
        });
    let (mut journal, mut journal_degradation) = match take_shared_journal(&shared_journal) {
        Ok((journal, error)) => (Some(journal), error.map(|error| error.to_string())),
        Err(error) => (None, Some(error.to_string())),
    };
    let outcome = match invocation {
        Ok(outcome) => outcome,
        Err(error) => {
            let message = error.to_string();
            let failed_presentation = Presentation::new(
                PresentationCategory::State,
                format!("{registry}:{name} entered the failed state before it could start."),
            );
            let synthetic = synthetic_runtime_failure(&message);
            if let Some(journal) = journal.as_mut() {
                if journal_degradation.is_none()
                    && let Err(journal_error) = journal.record_transition(
                        "running",
                        TransitionTrigger::new(
                            TriggerKind::InternalResult,
                            "tactus.process",
                            "plugin.spawn_failed",
                        )
                        .with_details(serde_json::json!({"error":message})),
                        TransitionGuard::new(
                            "process start attempt classified",
                            true,
                            "The process could not be started, so execution is a known failure.",
                        ),
                        "failed",
                        failed_presentation.clone(),
                    )
                {
                    note_journal_degradation(&mut journal_degradation, journal_error);
                }
                let (_, degradation) = finish_journal_preserving_outcome(
                    journal,
                    synthetic,
                    journal_degradation.take(),
                );
                journal_degradation = degradation;
            }
            if control.console_events {
                render_presentation(&failed_presentation);
            }
            if let Some(diagnostic) = journal_degradation.as_deref() {
                render_journal_degradation(diagnostic);
            }
            return Err(CliError::Process(error));
        }
    };
    if let Some(journal) = journal.as_mut()
        && journal_degradation.is_none()
    {
        if let Err(error) = record_outcome_transition(journal, registry, name, method, &outcome) {
            note_journal_degradation(&mut journal_degradation, error);
        }
        if journal_degradation.is_none() && outcome_is_unknown(outcome.kind) {
            let warning_message = Presentation::new(
                PresentationCategory::Warning,
                format!(
                    "{registry}:{name} may have completed externally; Tactus did not retry it automatically."
                ),
            );
            if let Err(error) = journal.record_with_presentation(
                "runtime.outcome_unknown",
                serde_json::json!({
                    "plugin":name,
                    "namespace":registry,
                    "method":method,
                    "cause":outcome_kind_name(outcome.kind),
                    "frames_seen":outcome.frames_seen,
                    "events_dropped":outcome.events_dropped,
                    "diagnostic":outcome.error.as_deref(),
                }),
                Some(warning_message),
            ) {
                note_journal_degradation(&mut journal_degradation, error);
            }
        }
    }
    let journal_was_durable = journal.as_ref().is_some_and(RunJournal::is_durable);
    let (summary, final_degradation) = match journal.as_mut() {
        Some(journal) => {
            finish_journal_preserving_outcome(journal, outcome, journal_degradation.take())
        }
        None => {
            let diagnostic = journal_degradation
                .as_deref()
                .unwrap_or("the journal writer was unavailable");
            (
                in_memory_degraded_summary(&run_id, outcome, diagnostic),
                Some(diagnostic.to_owned()),
            )
        }
    };
    let outcome = &summary.outcome;
    if control.console_events {
        render_presentation(&outcome_presentation(registry, name, method, outcome));
        if let Some(failure) = known_failure_presentation(registry, name, outcome) {
            render_presentation(&failure);
        }
    }
    if outcome_is_unknown(outcome.kind) {
        let warning_message = Presentation::new(
            PresentationCategory::Warning,
            format!(
                "{registry}:{name} may have completed externally; Tactus did not retry it automatically."
            ),
        );
        if control.console_events {
            render_presentation(&warning_message);
        }
    }
    if control.console_events && outcome.events_dropped > 0 {
        render_presentation(&Presentation::new(
            PresentationCategory::Warning,
            format!(
                "Tactus omitted {} low-priority progress events; the terminal result was preserved.",
                outcome.events_dropped
            ),
        ));
    }
    if control.console_events && outcome.observation_error.is_some() {
        render_presentation(&Presentation::new(
            PresentationCategory::Warning,
            "A diagnostic observer stopped responding; execution results were not changed.",
        ));
    }
    if let Some(diagnostic) = final_degradation.as_deref() {
        render_journal_degradation(diagnostic);
    }
    Ok(InvocationReport {
        name: name.to_owned(),
        run_path,
        summary,
        persisted: journal_was_durable && final_degradation.is_none(),
    })
}

fn inject_registered_params(
    workspace: &Workspace,
    definition: ResolvedPlugin<'_>,
    params: &mut Map<String, Value>,
) -> Result<(), CliError> {
    params
        .entry("workspace".to_owned())
        .or_insert_with(|| Value::String(workspace.root.to_string_lossy().into_owned()));

    let configured_options = match definition {
        ResolvedPlugin::Provider(value) => &value.options,
        ResolvedPlugin::Effect(value) => &value.options,
        ResolvedPlugin::Plugin(value) => &value.options,
    };
    let mut options = configured_options
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Map<_, _>>();
    if let Some(provided) = params.remove("options") {
        let provided = provided.as_object().ok_or_else(|| {
            CliError::InvalidArguments("plugin params.options must be a JSON object".to_owned())
        })?;
        options.extend(provided.clone());
    }
    params.insert("options".to_owned(), Value::Object(options));

    if let ResolvedPlugin::Provider(provider) = definition {
        if let Some(model) = provider.model.as_ref() {
            params
                .entry("model".to_owned())
                .or_insert_with(|| Value::String(model.clone()));
        }
        if let Some(effort) = provider.effort.as_ref() {
            params
                .entry("effort".to_owned())
                .or_insert_with(|| Value::String(effort.clone()));
        }
    }
    Ok(())
}

fn runtime_environment(
    workspace: &Workspace,
    runtime_path: &Path,
) -> Result<BTreeMap<String, String>, CliError> {
    let temporary = runtime_path.with_extension(format!("{}.tmp", std::process::id()));
    let encoded = serde_json::to_vec(&runtime_document(workspace)?)?;
    fs::write(&temporary, encoded)?;
    fs::rename(temporary, runtime_path)?;
    Ok(BTreeMap::from([
        (
            "TACTUS_RUNTIME_CONFIG".to_owned(),
            runtime_path.display().to_string(),
        ),
        (
            "TACTUS_WORKSPACE".to_owned(),
            workspace.root.display().to_string(),
        ),
        (
            "PATH".to_owned(),
            effective_path().to_string_lossy().into_owned(),
        ),
    ]))
}

fn runtime_document(workspace: &Workspace) -> Result<Value, CliError> {
    let dispatcher = env::current_exe()?;
    workspace
        .runtime_json_with_dispatcher(&dispatcher)
        .map_err(CliError::Workspace)
}

fn dispatch(
    start: &Path,
    name: &str,
    namespace: PluginNamespace,
    timeout_seconds: u64,
) -> Result<i32, CliError> {
    if namespace == PluginNamespace::Auto {
        return Err(CliError::InvalidArguments(
            "dispatch requires plugin, provider, or effect namespace".to_owned(),
        ));
    }
    let workspace = Workspace::discover(start)?;
    let cancellation = install_cancellation()?;
    let config = workspace.load_config()?;
    let definition = workspace.resolve_plugin(&config, name, namespace)?;
    let mut input = Vec::new();
    io::stdin().take(1024 * 1024 + 1).read_to_end(&mut input)?;
    if input.len() > 1024 * 1024 {
        return Err(CliError::InvalidArguments(
            "plugin request exceeds 1048576 bytes".to_owned(),
        ));
    }
    let mut request =
        decode_request(&input).map_err(|error| CliError::InvalidArguments(error.to_string()))?;
    inject_registered_params(&workspace, definition, &mut request.params)?;
    let (mut journal, journal_create_error) = create_journal_preserving_execution(&workspace);
    let namespace_name = format!("{namespace:?}").to_lowercase();
    let parent_run_id = env::var("TACTUS_RUN_ID").ok();
    let initial_journal_error = if let Some(error) = journal_create_error {
        Some(error)
    } else {
        journal
            .record_transition(
                "ready",
                TransitionTrigger::new(
                    TriggerKind::Request,
                    "tactus.dispatch",
                    "plugin.dispatch_requested",
                )
                .with_details(serde_json::json!({
                    "plugin": name,
                    "namespace": namespace_name,
                    "method": request.method,
                    "parent_run_id":parent_run_id,
                })),
                TransitionGuard::new(
                    "dispatch request and plugin registry entry validated",
                    true,
                    "The request belongs to a configured plugin and passed protocol validation.",
                ),
                "running",
                Presentation::new(
                    PresentationCategory::State,
                    format!("{namespace_name}:{name} method {} started.", request.method),
                ),
            )
            .err()
    };
    let tool_runtime = ToolRuntime::create(&workspace)?;
    let mut spec = ProcessSpec::new(
        resolve_builtin_command(definition.command())?,
        &workspace.root,
    );
    spec.environment = tool_runtime.environment.clone();
    attach_builtin_provider_to_supervised_group(&mut spec, definition.command());
    spec.limits.deadline = (timeout_seconds != 0).then(|| Duration::from_secs(timeout_seconds));

    let cancel_on_write = cancellation.clone();
    let callback_error = Arc::new(Mutex::new(None::<String>));
    let callback_error_sink = Arc::clone(&callback_error);
    let mut shared_journal_state = SharedJournal::new(journal);
    shared_journal_state.error = initial_journal_error;
    let shared_journal = Arc::new(Mutex::new(shared_journal_state));
    let callback_journal = Arc::clone(&shared_journal);
    let display_name = name.to_owned();
    let invocation = ProcessSupervisor.invoke(&spec, &request, &cancellation, move |frame| {
        {
            let mut journal = callback_journal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            journal.record_frame(&display_name, frame);
        }
        if matches!(frame, PluginFrame::Event { .. }) {
            let stdout = io::stdout();
            let mut stdout = stdout.lock();
            if let Err(error) = write_frame(&mut stdout, frame) {
                *callback_error_sink
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(format!("cannot forward plugin event: {error}"));
                cancel_on_write.cancel();
            }
        }
    });
    let spawn_error = invocation.as_ref().err().map(ToString::to_string);
    let outcome = invocation.unwrap_or_else(|error| synthetic_runtime_failure(&error.to_string()));
    if !outcome.stderr.is_empty() {
        render_presentation(&Presentation::new(
            PresentationCategory::Warning,
            format!(
                "The plugin produced {} bytes of native diagnostics; raw content was withheld (sha256 {:x}).",
                outcome.stderr.len(),
                Sha256::digest(outcome.stderr.as_bytes())
            ),
        ));
    }
    let callback_error = callback_error
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let (mut journal, mut journal_degradation) = match take_shared_journal(&shared_journal) {
        Ok((journal, error)) => (Some(journal), error.map(|error| error.to_string())),
        Err(error) => (None, Some(error.to_string())),
    };
    if let Some(journal) = journal.as_mut()
        && journal_degradation.is_none()
    {
        let transition_result = if let Some(error) = spawn_error.as_deref() {
            journal.record_transition(
                "running",
                TransitionTrigger::new(
                    TriggerKind::InternalResult,
                    "tactus.process",
                    "plugin.spawn_failed",
                )
                .with_details(serde_json::json!({"error":error})),
                TransitionGuard::new(
                    "process start attempt classified",
                    true,
                    "The process could not be started, so execution is a known failure.",
                ),
                "failed",
                Presentation::new(
                    PresentationCategory::State,
                    format!(
                        "{namespace_name}:{name} entered the failed state before it could start."
                    ),
                ),
            )
            .map(|_| ())
        } else {
            record_outcome_transition(journal, &namespace_name, name, &request.method, &outcome)
        };
        if let Err(error) = transition_result {
            note_journal_degradation(&mut journal_degradation, error);
        }
        if journal_degradation.is_none()
            && spawn_error.is_none()
            && outcome_is_unknown(outcome.kind)
            && let Err(error) = journal.record_with_presentation(
                "runtime.outcome_unknown",
                serde_json::json!({
                    "plugin":name,
                    "namespace":namespace_name,
                    "method":request.method,
                    "cause":outcome_kind_name(outcome.kind),
                    "diagnostic":outcome.error.as_deref(),
                }),
                Some(Presentation::new(
                    PresentationCategory::Warning,
                    format!(
                        "{namespace_name}:{name} may have completed externally; Tactus did not retry it automatically."
                    ),
                )),
            )
        {
            note_journal_degradation(&mut journal_degradation, error);
        }
    }
    let final_degradation = match journal.as_mut() {
        Some(journal) => {
            let (_, degradation) = finish_journal_preserving_outcome(
                journal,
                outcome.clone(),
                journal_degradation.take(),
            );
            degradation
        }
        None => journal_degradation.take(),
    };
    if let Some(diagnostic) = final_degradation.as_deref() {
        render_journal_degradation(diagnostic);
    }
    let dispatch_succeeded = callback_error.is_none() && outcome.kind == InvocationKind::Succeeded;
    let final_frame = if let Some(error) = callback_error {
        dispatch_failure_frame(&request, "tactus.dispatch_failed", error)
    } else {
        match outcome.kind {
            InvocationKind::Succeeded | InvocationKind::PluginFailed => outcome
                .terminal
                .as_ref()
                .map(|terminal| terminal_frame(&request, terminal))
                .unwrap_or_else(|| {
                    dispatch_failure_frame(
                        &request,
                        "tactus.protocol_failed",
                        "validated invocation had no terminal frame".to_owned(),
                    )
                }),
            _ => dispatch_failure_frame(
                &request,
                outcome_code(outcome.kind),
                outcome
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("plugin ended as {:?}", outcome.kind)),
            ),
        }
    };
    write_frame_bounded(&final_frame, Duration::from_millis(250))?;
    Ok(if dispatch_succeeded { 0 } else { 1 })
}

fn outcome_code(kind: InvocationKind) -> &'static str {
    match kind {
        InvocationKind::Succeeded => "tactus.succeeded",
        InvocationKind::PluginFailed => "tactus.plugin_failed",
        InvocationKind::ProcessFailed => "tactus.process_failed",
        InvocationKind::ProtocolFailed => "tactus.protocol_failed",
        InvocationKind::RuntimeFailed => "tactus.runtime_failed",
        InvocationKind::DeadlineExceeded => "tactus.deadline_exceeded",
        InvocationKind::Cancelled => "tactus.cancelled",
    }
}

fn dispatch_failure_frame(request: &PluginRequest, cause: &str, message: String) -> PluginFrame {
    failure_frame_with_details(
        request,
        "outcome_unknown",
        message,
        Some(serde_json::json!({"cause": cause})),
    )
}

fn failure_frame_with_details(
    request: &PluginRequest,
    code: &str,
    message: String,
    details: Option<Value>,
) -> PluginFrame {
    PluginFrame::Result {
        id: request.id.clone(),
        ok: false,
        value: JsonField::Missing,
        error: JsonField::Present(PluginFailure {
            code: code.to_owned(),
            message,
            details,
        }),
    }
}

fn terminal_frame(request: &PluginRequest, terminal: &TerminalResult) -> PluginFrame {
    match terminal {
        TerminalResult::Success { value } => PluginFrame::Result {
            id: request.id.clone(),
            ok: true,
            value: JsonField::Present(value.clone()),
            error: JsonField::Missing,
        },
        TerminalResult::Failure { error } => PluginFrame::Result {
            id: request.id.clone(),
            ok: false,
            value: JsonField::Missing,
            error: JsonField::Present(error.clone()),
        },
    }
}

fn write_frame(output: &mut impl Write, frame: &PluginFrame) -> io::Result<()> {
    serde_json::to_writer(&mut *output, frame).map_err(io::Error::other)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn write_frame_bounded(frame: &PluginFrame, timeout: Duration) -> io::Result<()> {
    let frame = frame.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        let _ = sender.send(write_frame(&mut stdout, &frame));
    });
    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "stdout did not accept the terminal plugin frame",
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "terminal plugin frame writer stopped unexpectedly",
        )),
    }
}

fn resolve_builtin_command(command: &[String]) -> Result<Vec<String>, CliError> {
    let mut resolved = command.to_vec();
    if resolved.first().is_some_and(|value| value == "tactus")
        && resolved.get(1).is_some_and(|value| {
            matches!(value.as_str(), "provider-host" | "effect-host" | "dispatch")
        })
    {
        resolved[0] = env::current_exe()?.display().to_string();
    }
    Ok(resolved)
}

fn attach_builtin_provider_to_supervised_group(spec: &mut ProcessSpec, command: &[String]) {
    #[cfg(unix)]
    if command.first().is_some_and(|value| {
        let file_name = Path::new(value)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(value);
        file_name == "tactus" || file_name == "tactus.exe"
    }) && command.get(1).is_some_and(|value| value == "provider-host")
    {
        spec.environment
            .insert(SUPERVISED_PROCESS_GROUP_ENV.to_owned(), "attach".to_owned());
    }
    #[cfg(not(unix))]
    let _ = (spec, command, SUPERVISED_PROCESS_GROUP_ENV);
}

fn install_cancellation() -> Result<CancellationToken, CliError> {
    let cancellation = CancellationToken::new();
    let signal_token = cancellation.clone();
    ctrlc::set_handler(move || signal_token.cancel_from_signal()).map_err(CliError::CtrlC)?;
    Ok(cancellation)
}

fn validate_haskell_package(value: &str) -> Result<String, String> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if valid {
        Ok(value.to_owned())
    } else {
        Err("package names must contain only ASCII letters, digits, '-', '_', or '.'".to_owned())
    }
}

fn haskell_packages(additional: &[String]) -> Vec<String> {
    let mut packages = vec!["clef-sdk".to_owned()];
    for package in additional {
        if !packages.contains(package) {
            packages.push(package.clone());
        }
    }
    packages
}

fn parse_params(value: &str) -> Result<Map<String, Value>, CliError> {
    let decoded = decode_json(value.as_bytes())
        .map_err(|error| CliError::InvalidArguments(format!("invalid --params JSON: {error}")))?;
    decoded
        .as_object()
        .cloned()
        .ok_or_else(|| CliError::InvalidArguments("--params must be a JSON object".to_owned()))
}

fn render_frame(name: &str, frame: &PluginFrame) {
    if let Some(presentation) = presentation_for_frame(name, frame) {
        render_presentation(&presentation);
    }
}

fn render_presentation(presentation: &Presentation) {
    eprintln!(
        "[{}] {}",
        presentation.category.label(),
        presentation.message
    );
}

fn presentation_for_frame(name: &str, frame: &PluginFrame) -> Option<Presentation> {
    match frame {
        PluginFrame::Event { event, .. } if event.kind == "effect.warning" => {
            let skipped = event
                .payload
                .get("skipped_paths")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            Some(Presentation::new(
                PresentationCategory::Warning,
                format!(
                    "{name} skipped {skipped} workspace path(s) that could not be inspected; execution continued."
                ),
            ))
        }
        PluginFrame::Event { event, .. }
            if matches!(
                event.kind.as_str(),
                "provider.progress"
                    | "provider.tool.started"
                    | "provider.tool.completed"
                    | "workflow.progress"
                    | "effect.progress"
            ) =>
        {
            Some(Presentation::new(
                PresentationCategory::Info,
                if event.kind == "provider.progress" {
                    format!("{name} is working.")
                } else {
                    format!("{name} reported {}.", event.kind.replace(['.', '_'], " "))
                },
            ))
        }
        PluginFrame::Event { .. } | PluginFrame::Result { .. } => None,
    }
}

fn diagnostic_frame(frame: &PluginFrame) -> (&'static str, Value) {
    match frame {
        PluginFrame::Event { event, .. } => (
            "plugin.event",
            serde_json::json!({
                "event_type":event.kind,
                "payload":event.payload,
            }),
        ),
        PluginFrame::Result {
            ok: true,
            value: JsonField::Present(value),
            ..
        } => {
            let encoded = serde_json::to_vec(value).unwrap_or_default();
            (
                "plugin.result",
                serde_json::json!({
                    "ok":true,
                    "value_summary":{
                        "bytes":encoded.len(),
                        "sha256":format!("{:x}", Sha256::digest(encoded)),
                    }
                }),
            )
        }
        PluginFrame::Result {
            ok: false,
            error: JsonField::Present(error),
            ..
        } => (
            "plugin.result",
            serde_json::json!({
                "ok":false,
                "error":{
                    "code":error.code,
                    "message":error.message,
                    "details":error.details,
                }
            }),
        ),
        PluginFrame::Result { ok, .. } => (
            "plugin.result",
            serde_json::json!({"ok":ok,"protocol_shape":"invalid"}),
        ),
    }
}

fn record_outcome_transition(
    journal: &mut RunJournal,
    namespace: &str,
    name: &str,
    method: &str,
    outcome: &ProcessOutcome,
) -> Result<(), JournalError> {
    let state_after = match outcome.kind {
        InvocationKind::Succeeded => "succeeded",
        InvocationKind::PluginFailed => "failed",
        InvocationKind::DeadlineExceeded => "timed_out",
        InvocationKind::Cancelled => "cancelled",
        InvocationKind::ProcessFailed
        | InvocationKind::ProtocolFailed
        | InvocationKind::RuntimeFailed => "outcome_unknown",
    };
    let presentation = outcome_presentation(namespace, name, method, outcome);
    let (trigger_kind, trigger_source, trigger_code, guard_condition, guard_reason) =
        match outcome.kind {
            InvocationKind::DeadlineExceeded => (
                TriggerKind::Timer,
                "tactus.deadline",
                "plugin.deadline_elapsed",
                "configured deadline elapsed",
                "Monotonic runtime time reached the configured invocation deadline.",
            ),
            InvocationKind::Cancelled => (
                TriggerKind::Control,
                "tactus.cancellation",
                "plugin.cancellation_requested",
                "cancellation request observed",
                "The runtime accepted a cancellation control signal.",
            ),
            InvocationKind::Succeeded
            | InvocationKind::PluginFailed
            | InvocationKind::ProcessFailed
            | InvocationKind::ProtocolFailed
            | InvocationKind::RuntimeFailed => (
                TriggerKind::InternalResult,
                "tactus.process",
                "plugin.supervision_completed",
                "supervisor outcome classified",
                "The process supervisor produced a terminal runtime classification.",
            ),
        };
    journal.record_transition(
        "running",
        TransitionTrigger::new(trigger_kind, trigger_source, trigger_code).with_details(
            serde_json::json!({
                "plugin":name,
                "namespace":namespace,
                "method":method,
                "outcome_kind":outcome_kind_name(outcome.kind),
                "exit_code":outcome.exit_code,
                "frames_seen":outcome.frames_seen,
                "events_dropped":outcome.events_dropped,
                "observation_error":outcome.observation_error.as_deref(),
                "side_effect_certainty":if outcome_is_unknown(outcome.kind) {
                    "unknown"
                } else {
                    "known"
                },
                "stderr_bytes":outcome.stderr.len(),
                "stderr_sha256":if outcome.stderr.is_empty() {
                    None
                } else {
                    Some(format!("{:x}", Sha256::digest(outcome.stderr.as_bytes())))
                },
            }),
        ),
        TransitionGuard::new(guard_condition, true, guard_reason),
        state_after,
        presentation,
    )?;
    if let Some(presentation) = durable_failure_presentation(namespace, name, outcome) {
        let diagnostic = match outcome.terminal.as_ref() {
            Some(TerminalResult::Failure { error }) => serde_json::json!({
                "plugin":name,
                "namespace":namespace,
                "code":error.code,
                "message":error.message,
                "details":error.details,
            }),
            _ => serde_json::json!({
                "plugin":name,
                "namespace":namespace,
                "diagnostic":outcome.error.as_deref(),
            }),
        };
        journal.record_with_presentation(
            "runtime.known_failure",
            diagnostic,
            Some(presentation),
        )?;
    }
    if outcome.events_dropped > 0 {
        journal.record_with_presentation(
            "runtime.progress_dropped",
            serde_json::json!({
                "events_dropped":outcome.events_dropped,
                "reason":"bounded observational delivery",
            }),
            Some(Presentation::new(
                PresentationCategory::Warning,
                format!(
                    "Tactus omitted {} low-priority progress events; the terminal result was preserved.",
                    outcome.events_dropped
                ),
            )),
        )?;
    }
    if let Some(error) = outcome.observation_error.as_deref() {
        journal.record_with_presentation(
            "runtime.observer_degraded",
            serde_json::json!({"diagnostic":error}),
            Some(Presentation::new(
                PresentationCategory::Warning,
                "A diagnostic observer stopped responding; execution results were not changed.",
            )),
        )?;
    }
    Ok(())
}

fn outcome_presentation(
    namespace: &str,
    name: &str,
    method: &str,
    outcome: &ProcessOutcome,
) -> Presentation {
    let message = match outcome.kind {
        InvocationKind::Succeeded => format!("{namespace}:{name} method {method} succeeded."),
        InvocationKind::PluginFailed => {
            format!("{namespace}:{name} method {method} entered the failed state.")
        }
        InvocationKind::DeadlineExceeded => {
            format!("{namespace}:{name} method {method} entered the timed-out state.")
        }
        InvocationKind::Cancelled => {
            format!("{namespace}:{name} method {method} entered the cancelled state.")
        }
        InvocationKind::ProcessFailed
        | InvocationKind::ProtocolFailed
        | InvocationKind::RuntimeFailed => {
            format!("{namespace}:{name} method {method} entered the outcome-unknown state.")
        }
    };
    Presentation::new(PresentationCategory::State, message)
}

fn known_failure_presentation(
    namespace: &str,
    name: &str,
    outcome: &ProcessOutcome,
) -> Option<Presentation> {
    if outcome.kind != InvocationKind::PluginFailed {
        return None;
    }
    let message = match outcome.terminal.as_ref() {
        Some(TerminalResult::Failure { error }) => {
            format!("{namespace}:{name} failed: {}", error.message)
        }
        _ => format!("{namespace}:{name} failed without a valid diagnostic."),
    };
    Some(Presentation::new(PresentationCategory::Error, message))
}

fn durable_failure_presentation(
    namespace: &str,
    name: &str,
    outcome: &ProcessOutcome,
) -> Option<Presentation> {
    if outcome.kind != InvocationKind::PluginFailed {
        return None;
    }
    let message = match outcome.terminal.as_ref() {
        Some(TerminalResult::Failure { error }) => format!(
            "{namespace}:{name} failed with diagnostic code {}.",
            error.code
        ),
        _ => format!("{namespace}:{name} failed without a valid diagnostic."),
    };
    Some(Presentation::new(PresentationCategory::Error, message))
}

fn outcome_is_unknown(kind: InvocationKind) -> bool {
    matches!(
        kind,
        InvocationKind::ProcessFailed
            | InvocationKind::ProtocolFailed
            | InvocationKind::RuntimeFailed
            | InvocationKind::DeadlineExceeded
            | InvocationKind::Cancelled
    )
}

fn outcome_kind_name(kind: InvocationKind) -> &'static str {
    match kind {
        InvocationKind::Succeeded => "succeeded",
        InvocationKind::PluginFailed => "plugin_failed",
        InvocationKind::ProcessFailed => "process_failed",
        InvocationKind::ProtocolFailed => "protocol_failed",
        InvocationKind::RuntimeFailed => "runtime_failed",
        InvocationKind::DeadlineExceeded => "deadline_exceeded",
        InvocationKind::Cancelled => "cancelled",
    }
}

fn print_json(value: &impl Serialize) -> Result<(), CliError> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

/// User-facing CLI failure.
#[derive(Debug, Error)]
pub enum CliError {
    /// Workspace operation failed.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// Process could not be started.
    #[error(transparent)]
    Process(#[from] ProcessError),
    /// Run journal could not be persisted.
    #[error(transparent)]
    Journal(#[from] JournalError),
    /// JSON input or output was invalid.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Runtime config materialization failed.
    #[error("runtime I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// The process interrupt handler could not be installed.
    #[error("cannot install Ctrl-C handler: {0}")]
    CtrlC(#[source] ctrlc::Error),
    /// An event consumer did not release a callback-owned resource in time.
    #[error("event sink stalled: {0}")]
    EventSinkStalled(String),
    /// Command arguments were semantically invalid.
    #[error("invalid arguments: {0}")]
    InvalidArguments(String),
}

/// Run the process CLI and render errors consistently.
pub fn entrypoint() -> i32 {
    match run() {
        Ok(code) => code,
        Err(error) => {
            render_presentation(&Presentation::new(
                PresentationCategory::Error,
                format!("Tactus could not complete the command: {error}"),
            ));
            2
        }
    }
}

/// Return the executable's current directory for small embedding smoke tests.
pub fn current_directory() -> Result<PathBuf, CliError> {
    env::current_dir().map_err(CliError::Io)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn params_reject_duplicate_keys_recursively() {
        let error =
            parse_params(r#"{"region":{"holes":1,"holes":2}}"#).expect_err("duplicate params");
        assert!(error.to_string().contains("duplicate object key"));
    }

    #[test]
    fn workspace_effect_warning_has_a_natural_language_projection() {
        let frame = serde_json::from_value::<PluginFrame>(serde_json::json!({
            "type":"event",
            "id":"warning-test",
            "event":{
                "type":"effect.warning",
                "skipped_paths":3,
                "examples":[{"path":"private.txt", "reason":"permission denied"}]
            }
        }))
        .expect("warning frame");

        let presentation = presentation_for_frame("workspace.paths", &frame).expect("presentation");
        assert_eq!(presentation.category, PresentationCategory::Warning);
        assert_eq!(
            presentation.message,
            "workspace.paths skipped 3 workspace path(s) that could not be inspected; execution continued."
        );
        assert!(!presentation.message.contains("private.txt"));
    }

    #[test]
    fn extension_packages_keep_clef_and_stable_order() {
        assert_eq!(
            haskell_packages(&[
                "segno-flow".to_owned(),
                "clef-sdk".to_owned(),
                "segno-flow".to_owned(),
            ]),
            vec!["clef-sdk".to_owned(), "segno-flow".to_owned()]
        );
    }

    #[test]
    fn package_names_are_bounded_and_cannot_be_ghc_arguments() {
        assert_eq!(
            validate_haskell_package("segno-flow"),
            Ok("segno-flow".to_owned())
        );
        assert!(validate_haskell_package("-package=evil").is_err());
        assert!(validate_haskell_package("package with spaces").is_err());
    }

    #[test]
    fn journal_callback_failure_preserves_known_terminal_outcomes() {
        let outcomes = [
            ProcessOutcome {
                kind: InvocationKind::Succeeded,
                exit_code: Some(0),
                terminal: Some(TerminalResult::Success {
                    value: serde_json::json!({"result":"known success"}),
                }),
                frames_seen: 1,
                events_dropped: 0,
                observation_error: None,
                stderr: String::new(),
                stderr_truncated: false,
                error: None,
                elapsed_ms: 1,
            },
            ProcessOutcome {
                kind: InvocationKind::PluginFailed,
                exit_code: Some(1),
                terminal: Some(TerminalResult::Failure {
                    error: PluginFailure {
                        code: "domain_failure".to_owned(),
                        message: "known failure".to_owned(),
                        details: Some(serde_json::json!({"stage":"fixture"})),
                    },
                }),
                frames_seen: 1,
                events_dropped: 0,
                observation_error: None,
                stderr: String::new(),
                stderr_truncated: false,
                error: None,
                elapsed_ms: 1,
            },
        ];

        for expected in outcomes {
            let temporary = tempdir().expect("temporary directory");
            let workspace = Workspace::at(temporary.path());
            let journal = RunJournal::create(&workspace).expect("journal");
            let mut shared_state = SharedJournal::new(journal);
            shared_state.error = Some(JournalError::Io(io::Error::other(
                "injected callback writer failure",
            )));
            let shared = Arc::new(Mutex::new(shared_state));
            let (mut journal, callback_error) = take_shared_journal(&shared).expect("take journal");
            let (summary, degradation) = finish_journal_preserving_outcome(
                &mut journal,
                expected.clone(),
                callback_error.map(|error| error.to_string()),
            );

            assert_eq!(summary.outcome.kind, expected.kind);
            assert_eq!(summary.outcome.terminal, expected.terminal);
            assert!(
                summary
                    .outcome
                    .observation_error
                    .as_deref()
                    .is_some_and(|message| message.contains("injected callback writer failure"))
            );
            assert!(
                degradation
                    .as_deref()
                    .is_some_and(|message| message.contains("injected callback writer failure"))
            );
            let durable: RunSummary = serde_json::from_slice(
                &fs::read(journal.summary_path()).expect("degraded summary"),
            )
            .expect("summary JSON");
            assert_eq!(durable.outcome.kind, expected.kind);
            assert_eq!(
                durable
                    .outcome
                    .terminal
                    .as_ref()
                    .map(|terminal| match terminal {
                        TerminalResult::Success { .. } => "success",
                        TerminalResult::Failure { .. } => "failure",
                    }),
                expected.terminal.as_ref().map(|terminal| match terminal {
                    TerminalResult::Success { .. } => "success",
                    TerminalResult::Failure { .. } => "failure",
                })
            );
        }
    }

    #[test]
    fn journal_creation_failure_falls_back_without_blocking_execution_setup() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        fs::create_dir_all(&workspace.control).expect("control directory");
        fs::write(&workspace.runs_path, "not a directory").expect("blocked runs path");

        let (mut journal, degradation) = create_journal_preserving_execution(&workspace);
        assert!(degradation.is_some());
        assert!(!journal.is_durable());
        journal
            .record_transition(
                "ready",
                TransitionTrigger::new(TriggerKind::Request, "test", "test.requested"),
                TransitionGuard::new("request accepted", true, "fixture"),
                "running",
                Presentation::new(PresentationCategory::State, "Fixture started."),
            )
            .expect("in-memory transition");
    }

    #[test]
    fn tool_runtime_config_is_ephemeral_and_separate_from_run_journal() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        fs::create_dir_all(&workspace.runs_path).expect("runs directory");
        fs::write(
            &workspace.config_path,
            r#"api = "clef.runtime/v1"
default_provider = "codex"
instructions = ".tactus/PROMPT.md"

[providers.codex]
command = ["tactus", "provider-host", "codex"]
"#,
        )
        .expect("runtime config");
        fs::write(&workspace.prompt_path, "test instructions\n").expect("prompt");
        let journal = RunJournal::create(&workspace).expect("journal");
        let run_path = journal.run_path().to_path_buf();

        let runtime_path = {
            let runtime = ToolRuntime::create(&workspace).expect("tool runtime");
            let path = PathBuf::from(
                runtime
                    .environment
                    .get("TACTUS_RUNTIME_CONFIG")
                    .expect("runtime path"),
            );
            assert!(path.is_file());
            assert!(!path.starts_with(&workspace.runs_path));
            assert!(!path.starts_with(&run_path));
            assert!(!run_path.join("runtime.json").exists());
            path
        };

        assert!(!runtime_path.exists());
        assert!(!run_path.join("runtime.json").exists());
    }

    #[test]
    fn stale_tool_runtime_cleanup_is_bounded_and_prefix_scoped() {
        let temporary = tempdir().expect("temporary directory");
        let stale = temporary.path().join(format!("{TOOL_RUNTIME_PREFIX}stale"));
        let active = temporary
            .path()
            .join(format!("{TOOL_RUNTIME_PREFIX}active"));
        let unrelated = temporary.path().join("unrelated-runtime");
        let matching_file = temporary
            .path()
            .join(format!("{TOOL_RUNTIME_PREFIX}plain-file"));
        fs::create_dir(&stale).expect("stale directory");
        fs::write(stale.join(".lease"), "").expect("stale lease");
        fs::write(stale.join("runtime.json"), "sensitive config").expect("stale config");
        fs::create_dir(&active).expect("active directory");
        let active_lease = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(active.join(".lease"))
            .expect("active lease");
        fs4::FileExt::lock(&active_lease).expect("lock active lease");
        fs::write(active.join("runtime.json"), "active config").expect("active config");
        fs::create_dir(&unrelated).expect("unrelated directory");
        fs::write(&matching_file, "not a directory").expect("matching file");

        cleanup_stale_tool_runtimes(temporary.path(), SystemTime::now() + Duration::from_secs(1));

        assert!(!stale.exists());
        assert!(active.is_dir());
        assert!(unrelated.is_dir());
        assert!(matching_file.is_file());

        fs4::FileExt::unlock(&active_lease).expect("unlock active lease");
        drop(active_lease);
        cleanup_stale_tool_runtimes(temporary.path(), SystemTime::now() + Duration::from_secs(1));
        assert!(!active.exists());
    }

    #[test]
    fn nested_invocation_reports_persist_only_terminal_summaries() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");

        let mut success = script_batch_outcome(&Ok(0), 1, Duration::from_millis(1));
        success.terminal = Some(TerminalResult::Success {
            value: serde_json::json!({"unknown":"DO_NOT_PERSIST_SUCCESS_VALUE"}),
        });
        let success_report = InvocationReport {
            name: "provider".to_owned(),
            run_path: PathBuf::from("DO_NOT_PERSIST_RUN_PATH"),
            summary: RunSummary {
                api: crate::journal::TRACE_API.to_owned(),
                run_id: "child-success".to_owned(),
                started_unix_ms: 1,
                finished_unix_ms: 2,
                events_recorded: 1,
                outcome: success,
            },
            persisted: true,
        };
        journal
            .record("provider.completed", success_report.diagnostic_value())
            .expect("success diagnostic");

        let mut failure = script_batch_outcome(&Ok(1), 1, Duration::from_millis(1));
        failure.terminal = Some(TerminalResult::Failure {
            error: PluginFailure {
                code: "provider.failed".to_owned(),
                message: "DO_NOT_PERSIST_FAILURE_MESSAGE".to_owned(),
                details: Some(serde_json::json!({
                    "unknown":"DO_NOT_PERSIST_FAILURE_DETAILS"
                })),
            },
        });
        let failure_report = InvocationReport {
            name: "observer".to_owned(),
            run_path: PathBuf::from("DO_NOT_PERSIST_OBSERVER_PATH"),
            summary: RunSummary {
                api: crate::journal::TRACE_API.to_owned(),
                run_id: "child-failure".to_owned(),
                started_unix_ms: 1,
                finished_unix_ms: 2,
                events_recorded: 1,
                outcome: failure,
            },
            persisted: true,
        };
        journal
            .record("observer.end", failure_report.diagnostic_value())
            .expect("failure diagnostic");
        journal
            .finish(script_batch_outcome(&Ok(0), 1, Duration::from_millis(1)))
            .expect("finish");

        let durable = fs::read_to_string(journal.event_path()).expect("events");
        assert!(!durable.contains("DO_NOT_PERSIST"), "{durable}");
        assert!(durable.contains("provider.failed"), "{durable}");
        assert!(durable.contains("diagnostic_summary"), "{durable}");
    }

    #[test]
    fn script_batch_outcome_keeps_known_success_and_failure_terminal() {
        let success = script_batch_outcome(&Ok(0), 3, Duration::from_millis(7));
        assert_eq!(success.kind, InvocationKind::Succeeded);
        assert!(matches!(
            success.terminal,
            Some(TerminalResult::Success { .. })
        ));

        let failure = script_batch_outcome(&Ok(9), 3, Duration::from_millis(7));
        assert_eq!(failure.kind, InvocationKind::PluginFailed);
        assert_eq!(failure.exit_code, Some(9));
        assert!(matches!(
            failure.terminal,
            Some(TerminalResult::Failure { .. })
        ));
    }

    #[test]
    fn imports_only_valid_clef_transition_and_message_sidecars() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");
        let sidecar = temporary.path().join("clef.jsonl");
        fs::write(
            &sidecar,
            concat!(
                r#"{"type":"state_transition","code":"workflow.transition","level":"state","message":"DO_NOT_PERSIST_TRANSITION_MESSAGE","subject":"workflow:test","state_before":"domain.ready","trigger":{"kind":"request","source":"clef.workflow","code":"workflow.requested","details":{"unknown":"DO_NOT_PERSIST_TRIGGER_DETAILS"}},"guard":{"condition":"DO_NOT_PERSIST_GUARD_CONDITION","passed":true,"reason":"DO_NOT_PERSIST_GUARD_REASON"},"state_after":"domain.running","context":{"prompt":"DO_NOT_PERSIST_PROMPT","unknown":"DO_NOT_PERSIST_CONTEXT"}}"#,
                "\n",
                r#"{"type":"message","code":"workflow.notice","level":"warning","message":"DO_NOT_PERSIST_MESSAGE_TEXT","context":{"unknown":"DO_NOT_PERSIST_MESSAGE_CONTEXT"}}"#,
                "\n"
            ),
        )
        .expect("sidecar");

        assert_eq!(
            import_clef_diagnostic_sidecar(&mut journal, &sidecar).expect("import"),
            2
        );
        journal
            .finish(script_batch_outcome(&Ok(0), 1, Duration::from_millis(1)))
            .expect("finish");
        let encoded = fs::read_to_string(journal.event_path()).expect("events");
        let events = encoded
            .lines()
            .map(|line| serde_json::from_str::<crate::journal::TraceEvent>(line).expect("event"))
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "runtime.state_transition");
        assert_eq!(events[0].data["state_before"], "domain.ready");
        assert_eq!(events[0].data["trigger"]["kind"], "request");
        assert_eq!(events[0].data["trigger"]["code"], "workflow.requested");
        assert_eq!(events[0].data["guard"]["passed"], true);
        assert_eq!(events[0].data["state_after"], "domain.running");
        assert_eq!(events[1].kind, "runtime.message");
        assert_eq!(
            events[1].presentation.as_ref().map(|value| value.category),
            Some(PresentationCategory::Warning)
        );
        assert!(!encoded.contains("DO_NOT_PERSIST"));
        assert_eq!(
            events[0]
                .presentation
                .as_ref()
                .map(|value| value.message.as_str()),
            Some("Clef recorded transition workflow.transition: domain.ready to domain.running.")
        );
    }

    #[test]
    fn rejects_non_projection_clef_sidecar_without_poisoning_journal() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");
        let sidecar = temporary.path().join("clef-raw.jsonl");
        fs::write(
            &sidecar,
            r#"{"type":"plugin_event","plugin":"fake","event":{"raw":"secret"}}"#,
        )
        .expect("sidecar");

        assert!(matches!(
            import_clef_diagnostic_sidecar(&mut journal, &sidecar),
            Err(ClefSidecarError::Invalid(_))
        ));
        journal
            .record("runtime.after_rejection", serde_json::json!({"ok":true}))
            .expect("journal remains usable");
        let summary = journal
            .finish(script_batch_outcome(&Ok(0), 1, Duration::from_millis(1)))
            .expect("finish");
        assert_eq!(summary.outcome.kind, InvocationKind::Succeeded);
        assert_eq!(summary.events_recorded, 1);
    }
}
