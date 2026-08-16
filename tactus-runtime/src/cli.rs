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
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    adapters::{SUPERVISED_PROCESS_GROUP_ENV, run_provider_host, run_workspace_paths_host},
    journal::{JournalError, RunJournal, RunSummary},
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
        } => check(&root, &scripts, keep_going, timeout_seconds),
        Command::Run {
            scripts,
            keep_going,
            root,
            timeout_seconds,
            arguments,
        } => run_scripts_command(&root, &scripts, &arguments, keep_going, timeout_seconds),
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
        println!("Tactus workspace: {}", report.workspace.root.display());
        for path in report.created {
            println!("created   {path}");
        }
        for path in report.preserved {
            println!("preserved {path}");
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
            println!("{order} {kind}  {}", script.relative_path);
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
            println!(
                "{:<4} {:<24} {}",
                if check.ok { "ok" } else { "fail" },
                check.name,
                check.detail
            );
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
    let build = execute_tool(
        &workspace,
        vec![
            "cabal".to_owned(),
            "build".to_owned(),
            "--project-dir".to_owned(),
            project.clone(),
            "lib:clef-sdk".to_owned(),
        ],
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
        let status = execute_tool(
            &workspace,
            vec![
                "cabal".to_owned(),
                "exec".to_owned(),
                "--project-dir".to_owned(),
                project.clone(),
                "--".to_owned(),
                "ghc".to_owned(),
                "-fno-code".to_owned(),
                "-package".to_owned(),
                "clef-sdk".to_owned(),
                include.clone(),
                script.display().to_string(),
            ],
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
    let tool_runtime = ToolRuntime::create(&workspace)?;
    let environment = &tool_runtime.environment;
    let project = workspace.control.display().to_string();
    let build = execute_tool(
        &workspace,
        vec![
            "cabal".to_owned(),
            "build".to_owned(),
            "--project-dir".to_owned(),
            project.clone(),
            "lib:clef-sdk".to_owned(),
        ],
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
            "runghc".to_owned(),
            "--ghc-arg=-package=clef-sdk".to_owned(),
            format!("--ghc-arg={include}"),
            script.display().to_string(),
        ];
        command.extend_from_slice(arguments);
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
    let generation_prompt = format!(
        "{instructions}\n\n# Generation goal\n\n{goal}\n\nCreate a multi-step workflow: begin with the smallest atomic scripts, then compose them into a final complete program. Only write DSL files; do not build or run them.\n"
    );
    let mut params = Map::new();
    params.insert("prompt".to_owned(), Value::String(generation_prompt));
    params.insert(
        "workspace".to_owned(),
        Value::String(workspace.root.display().to_string()),
    );
    let script_baseline = script_fingerprints(&workspace)?;

    let mut generation_journal = RunJournal::create(&workspace)?;
    let generation_path = generation_journal.run_path().to_path_buf();
    let invocation = generation_journal.run_id().to_owned();
    let context = serde_json::json!({
        "source": "tactus.generate",
        "provider": provider_name,
        "goal": goal,
    });
    generation_journal.record(
        "generation.started",
        serde_json::json!({"provider": provider_name, "goal": goal}),
    )?;
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
                generation_journal.record("observer.begin", serde_json::to_value(&report)?)?;
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
                generation_journal.record(
                    "observer.begin_error",
                    serde_json::json!({"effect": effect_name, "error": message}),
                )?;
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
                generation_journal.record("provider.completed", serde_json::to_value(&report)?)?;
                provider_report = Some(report);
            }
            Err(error) => {
                let message = error.to_string();
                generation_journal.record(
                    "provider.error",
                    serde_json::json!({"provider": provider_name, "error": message}),
                )?;
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
    )?;
    let scripts = discover_scripts(&workspace)?;
    let generated_delta = generated_script_delta(&script_baseline, &scripts)?;
    generation_journal.record(
        "generation.discovered_scripts",
        serde_json::json!({"scripts":scripts, "generated_delta":generated_delta}),
    )?;
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
    let generation_summary = generation_journal.finish(generation_outcome)?;
    let generation_report = InvocationReport {
        name: "generate".to_owned(),
        run_path: generation_path,
        summary: generation_summary,
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
        println!(
            "provider {provider_name}: {}",
            if success { "ok" } else { "failed" }
        );
        if let Some(error) = provider_error.as_deref() {
            eprintln!("  {error}");
        }
        if let Some(error) = generation_error.as_deref() {
            eprintln!("  {error}");
        }
        if let Some(report) = provider_report.as_ref() {
            if let Some(TerminalResult::Failure { error }) = &report.summary.outcome.terminal {
                eprintln!("  {}: {}", error.code, error.message);
            }
            if let Some(error) = report
                .summary
                .outcome
                .error
                .as_deref()
                .filter(|error| !error.is_empty())
            {
                eprintln!("  {error}");
            }
        }
        for error in observer_errors {
            eprintln!("  observer failure: {error}");
        }
        for script in scripts {
            let kind = if script.runnable { "entry " } else { "helper" };
            let order = script
                .order
                .map_or_else(|| "---".to_owned(), |value| format!("{value:03}"));
            println!("{order} {kind}  {}", script.relative_path);
        }
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
                journal.record("observer.end", serde_json::to_value(&report)?)?;
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
                journal.record(
                    "observer.end_error",
                    serde_json::json!({"effect": observer.name, "error": message}),
                )?;
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
    directory: PathBuf,
    environment: BTreeMap<String, String>,
}

impl ToolRuntime {
    fn create(workspace: &Workspace) -> Result<Self, CliError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let directory = workspace
            .runs_path
            .join(format!(".tool-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory)?;
        let environment = runtime_environment(workspace, &directory.join("runtime.json"))?;
        Ok(Self {
            directory,
            environment,
        })
    }
}

impl Drop for ToolRuntime {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
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
        eprintln!(
            "tactus: command ended as {:?}: {}",
            outcome.kind,
            outcome.error.as_deref().unwrap_or("no additional detail")
        );
    }
    Ok(outcome.exit_code.unwrap_or(1))
}

#[derive(Debug, Serialize)]
struct InvocationReport {
    name: String,
    run_path: PathBuf,
    summary: RunSummary,
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
    } else {
        println!(
            "plugin {name}: {:?} (run {})",
            report.summary.outcome.kind, report.summary.run_id
        );
        if let Some(error) = &report.summary.outcome.error {
            eprintln!("  {error}");
        }
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
            println!("{selector}: {:?}", report.summary.outcome.kind);
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

    fn record_frame(&mut self, frame: &PluginFrame) {
        if self.error.is_some() {
            return;
        }
        let Some(journal) = self.journal.as_mut() else {
            return;
        };
        match serde_json::to_value(frame) {
            Ok(value) => {
                if let Err(error) = journal.record("plugin.frame", value) {
                    self.error = Some(error);
                }
            }
            Err(error) => {
                self.error = Some(JournalError::Json(error));
            }
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
    let mut journal = RunJournal::create(workspace)?;
    let run_path = journal.run_path().to_path_buf();
    let run_id = journal.run_id().to_owned();
    journal.record(
        "invocation.started",
        serde_json::json!({
            "plugin": name,
            "method": method,
            "namespace": registry,
            "executable": executable,
            "argument_count": command.len().saturating_sub(1),
        }),
    )?;
    let request = PluginRequest::new(run_id, method, params)
        .map_err(|error| CliError::InvalidArguments(error.to_string()))?;
    let mut spec = ProcessSpec::new(
        resolve_builtin_command(definition.command())?,
        &workspace.root,
    );
    spec.environment = runtime_environment(workspace, &run_path.join("runtime.json"))?;
    attach_builtin_provider_to_supervised_group(&mut spec, definition.command());
    spec.limits.deadline =
        (control.timeout_seconds != 0).then(|| Duration::from_secs(control.timeout_seconds));
    let shared_journal = Arc::new(Mutex::new(SharedJournal::new(journal)));
    let callback_journal = Arc::clone(&shared_journal);
    let display_name = name.to_owned();
    let console_events = control.console_events;
    let invocation =
        ProcessSupervisor.invoke(&spec, &request, control.cancellation, move |frame| {
            callback_journal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .record_frame(frame);
            if console_events {
                render_frame(&display_name, frame);
            }
        });
    let (mut journal, journal_error) = take_shared_journal(&shared_journal)?;
    if let Some(error) = journal_error {
        return Err(CliError::Journal(error));
    }
    let outcome = match invocation {
        Ok(outcome) => outcome,
        Err(error) => {
            let message = error.to_string();
            journal.record(
                "invocation.runtime_error",
                serde_json::json!({"error":message}),
            )?;
            journal.finish(synthetic_runtime_failure(&message))?;
            return Err(CliError::Process(error));
        }
    };
    let summary = journal.finish(outcome)?;
    Ok(InvocationReport {
        name: name.to_owned(),
        run_path,
        summary,
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
    let mut journal = RunJournal::create(&workspace)?;
    let run_path = journal.run_path().to_path_buf();
    journal.record(
        "dispatch.started",
        serde_json::json!({
            "plugin": name,
            "namespace": format!("{namespace:?}").to_lowercase(),
            "method": request.method,
        }),
    )?;
    let mut spec = ProcessSpec::new(
        resolve_builtin_command(definition.command())?,
        &workspace.root,
    );
    spec.environment = runtime_environment(&workspace, &run_path.join("runtime.json"))?;
    attach_builtin_provider_to_supervised_group(&mut spec, definition.command());
    spec.limits.deadline = (timeout_seconds != 0).then(|| Duration::from_secs(timeout_seconds));

    let cancel_on_write = cancellation.clone();
    let callback_error = Arc::new(Mutex::new(None::<String>));
    let callback_error_sink = Arc::clone(&callback_error);
    let shared_journal = Arc::new(Mutex::new(SharedJournal::new(journal)));
    let callback_journal = Arc::clone(&shared_journal);
    let invocation = ProcessSupervisor.invoke(&spec, &request, &cancellation, move |frame| {
        {
            let mut journal = callback_journal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            journal.record_frame(frame);
            if let Some(error) = journal.error.as_ref() {
                *callback_error_sink
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error.to_string());
                cancel_on_write.cancel();
                return;
            }
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
        let _ = io::stderr().write_all(outcome.stderr.as_bytes());
    }
    let callback_error = callback_error
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let journal_result = take_shared_journal(&shared_journal)
        .map_err(|error| JournalError::Io(io::Error::other(error.to_string())))
        .and_then(|(mut journal, error)| {
            if let Some(error) = error {
                return Err(error);
            }
            if let Some(error) = spawn_error.as_deref() {
                journal.record("dispatch.runtime_error", serde_json::json!({"error":error}))?;
            }
            journal.finish(outcome.clone())
        });
    let dispatch_succeeded = callback_error.is_none()
        && journal_result.is_ok()
        && outcome.kind == InvocationKind::Succeeded;
    let final_frame = if let Some(error) = callback_error {
        dispatch_failure_frame(&request, "tactus.dispatch_failed", error)
    } else if let Err(error) = journal_result {
        dispatch_failure_frame(&request, "tactus.journal_failed", error.to_string())
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

fn parse_params(value: &str) -> Result<Map<String, Value>, CliError> {
    let decoded = decode_json(value.as_bytes())
        .map_err(|error| CliError::InvalidArguments(format!("invalid --params JSON: {error}")))?;
    decoded
        .as_object()
        .cloned()
        .ok_or_else(|| CliError::InvalidArguments("--params must be a JSON object".to_owned()))
}

fn render_frame(name: &str, frame: &PluginFrame) {
    match frame {
        PluginFrame::Event { event, .. } => {
            eprintln!(
                "[{name}] {} {}",
                event.kind,
                Value::Object(event.payload.clone())
            );
        }
        PluginFrame::Result { ok, .. } => eprintln!("[{name}] terminal ok={ok}"),
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
            eprintln!("tactus: {error}");
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
    use super::*;

    #[test]
    fn params_reject_duplicate_keys_recursively() {
        let error =
            parse_params(r#"{"region":{"holes":1,"holes":2}}"#).expect_err("duplicate params");
        assert!(error.to_string().contains("duplicate object key"));
    }
}
