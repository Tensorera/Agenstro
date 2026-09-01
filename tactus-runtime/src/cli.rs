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
    executable::effective_path,
    journal::{
        JournalError, Presentation, PresentationCategory, RunJournal, RunSummary, TransitionGuard,
        TransitionTrigger, TriggerKind, diagnostic_failure_details, diagnostic_summary,
        diagnostic_value_summary,
    },
    limits::RuntimeLimits,
    outcome::{
        OutcomeContext, OutcomeState, OutcomeUnknownDiagnostic, classify_outcome,
        outcome_is_unknown,
    },
    process::{
        CancellationToken, CommandKind, CommandOutcome, InvocationKind, InvocationPhase,
        InvocationProgress, ProcessError, ProcessOutcome, ProcessSpec, ProcessSupervisor,
    },
    protocol::{
        JsonField, PluginFailure, PluginFrame, PluginRequest, TerminalResult, decode_json,
        decode_request,
    },
    session::SessionControlFailure,
    studio::{ControlFailure, ControlSuccess},
    workspace::{
        PluginNamespace, ResolvedPlugin, ScriptInfo, Workspace, WorkspaceError, discover_scripts,
        doctor, initialize_workspace,
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
        /// Explicit sources. Repeat as positional arguments to preserve order.
        scripts: Vec<PathBuf>,
        /// Select every discovered Haskell source explicitly.
        #[arg(long)]
        all: bool,
        /// Select numbered entries at or after this three-digit order.
        #[arg(long, value_parser = validate_entry_order)]
        from: Option<u16>,
        /// Select numbered entries at or before this three-digit order.
        #[arg(long, value_parser = validate_entry_order)]
        through: Option<u16>,
        /// Continue after a source fails.
        #[arg(long)]
        keep_going: bool,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Override the workspace deadline for each Cabal/GHC process; zero disables it.
        #[arg(long)]
        timeout_seconds: Option<u64>,
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
        /// Select every numbered entry explicitly.
        #[arg(long)]
        all: bool,
        /// Select numbered entries at or after this three-digit order.
        #[arg(long, value_parser = validate_entry_order)]
        from: Option<u16>,
        /// Select numbered entries at or before this three-digit order.
        #[arg(long, value_parser = validate_entry_order)]
        through: Option<u16>,
        /// Continue after an entry fails.
        #[arg(long)]
        keep_going: bool,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Override the workspace deadline for each Cabal/runghc process; zero disables it.
        #[arg(long)]
        timeout_seconds: Option<u64>,
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
        /// Override the workspace outer provider deadline; zero disables it.
        #[arg(long)]
        timeout_seconds: Option<u64>,
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
        /// Override the workspace invocation deadline; zero disables it.
        #[arg(long)]
        timeout_seconds: Option<u64>,
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
        /// Override the workspace dispatch deadline; zero disables it.
        #[arg(long)]
        timeout_seconds: Option<u64>,
    },
    /// Call the `smoke` method on selected unambiguous plugins.
    Smoke {
        /// Plugin selectors. With none, checks every provider, effect, and generic plugin.
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
    /// Query and conservatively maintain local run journals.
    Runs {
        /// Run-journal operation.
        #[command(subcommand)]
        command: RunsCommand,
    },
    /// Read and answer durable human-in-the-loop sessions.
    Session {
        /// Session control operation.
        #[command(subcommand)]
        command: SessionCommand,
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
        /// Refuse upward discovery when the supplied folder is not the workspace root.
        #[arg(long)]
        exact_root: bool,
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

#[derive(Debug, Subcommand)]
enum RunsCommand {
    /// List active run journals newest first.
    List {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Canonical state or specific invocation kind.
        #[arg(long)]
        state: Option<String>,
        /// Unix milliseconds or a relative age such as `24h` or `7d`.
        #[arg(long)]
        since: Option<String>,
        /// Maximum records returned after filtering.
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Emit one JSON result.
        #[arg(long)]
        json: bool,
    },
    /// Aggregate active runs by canonical state and invocation kind.
    Summarize {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Canonical state or specific invocation kind.
        #[arg(long)]
        state: Option<String>,
        /// Unix milliseconds or a relative age such as `24h` or `7d`.
        #[arg(long)]
        since: Option<String>,
        /// Emit one JSON result.
        #[arg(long)]
        json: bool,
    },
    /// List runs that have no atomically published terminal summary.
    Unfinished {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Unix milliseconds or a relative age such as `24h` or `7d`.
        #[arg(long)]
        since: Option<String>,
        /// Maximum records returned after filtering.
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Emit one JSON result.
        #[arg(long)]
        json: bool,
    },
    /// Show one run and a bounded event page.
    Show {
        /// Opaque run identifier returned by `runs list`.
        run_id: String,
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Return events whose sequence is greater than this cursor.
        #[arg(long, default_value_t = 0)]
        after: u64,
        /// Maximum events in this page.
        #[arg(long, default_value_t = 250)]
        limit: usize,
        /// Maximum retained event bytes in this page.
        #[arg(long, default_value_t = 4 * 1024 * 1024)]
        max_bytes: usize,
        /// Emit one JSON result including redacted event payloads.
        #[arg(long)]
        json: bool,
    },
    /// Move eligible old runs into `.tactus/runs/archive`.
    Archive {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Upper time bound as Unix milliseconds or a relative age.
        #[arg(long)]
        before: String,
        /// Apply the previewed moves. Without this flag nothing changes.
        #[arg(long)]
        yes: bool,
        /// Emit one JSON result.
        #[arg(long)]
        json: bool,
    },
    /// Delete eligible runs already moved into the archive.
    Gc {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Optional upper time bound as Unix milliseconds or a relative age.
        #[arg(long)]
        before: Option<String>,
        /// Apply the previewed deletions. Without this flag nothing changes.
        #[arg(long)]
        yes: bool,
        /// Emit one JSON result.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    /// List validated sessions, newest first.
    List {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Maximum number of sessions to return.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Show one validated session.
    Show {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Opaque session identifier.
        #[arg(long)]
        session: String,
    },
    /// Apply one answer using the current turn as a compare-and-set token.
    Answer {
        /// Start path for upward workspace discovery.
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Opaque session identifier.
        #[arg(long)]
        session: String,
        /// Delivered-brief turn token.
        #[arg(long)]
        turn: String,
        /// Stable pending question axis.
        #[arg(long)]
        axis: String,
        /// One option present in the pending question.
        #[arg(long)]
        option: String,
        /// Optional bounded human note.
        #[arg(long, allow_hyphen_values = true)]
        note: Option<String>,
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
            all,
            from,
            through,
            keep_going,
            root,
            timeout_seconds,
            packages,
        } => check(
            &root,
            ScriptSelection::new(&scripts, all, from, through),
            &packages,
            keep_going,
            timeout_seconds,
        ),
        Command::Run {
            scripts,
            all,
            from,
            through,
            keep_going,
            root,
            timeout_seconds,
            packages,
            arguments,
        } => run_scripts_command(
            &root,
            ScriptSelection::new(&scripts, all, from, through),
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
        Command::Runs { command } => runs(command),
        Command::Session { command } => session(command),
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
        StudioCommand::Inspect {
            root,
            exact_root,
            run_limit,
        } => {
            let inspected = if exact_root {
                crate::studio::inspect_exact(&root, run_limit)
            } else {
                crate::studio::inspect(&root, run_limit)
            };
            match inspected {
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

fn runs(command: RunsCommand) -> Result<i32, CliError> {
    match command {
        RunsCommand::List {
            root,
            state,
            since,
            limit,
            json,
        } => present_runs_result(
            "runs.list",
            json,
            crate::runs::list(&root, state.as_deref(), since.as_deref(), limit),
            render_run_list,
        ),
        RunsCommand::Summarize {
            root,
            state,
            since,
            json,
        } => present_runs_result(
            "runs.summarize",
            json,
            crate::runs::summarize(&root, state.as_deref(), since.as_deref()),
            render_run_aggregate,
        ),
        RunsCommand::Unfinished {
            root,
            since,
            limit,
            json,
        } => present_runs_result(
            "runs.unfinished",
            json,
            crate::runs::unfinished(&root, since.as_deref(), limit),
            render_run_list,
        ),
        RunsCommand::Show {
            run_id,
            root,
            after,
            limit,
            max_bytes,
            json,
        } => present_runs_result(
            "runs.show",
            json,
            crate::runs::show(&root, &run_id, after, limit, max_bytes),
            render_run_show,
        ),
        RunsCommand::Archive {
            root,
            before,
            yes,
            json,
        } => present_runs_result(
            "runs.archive",
            json,
            crate::runs::archive(&root, &before, yes),
            render_maintenance_report,
        ),
        RunsCommand::Gc {
            root,
            before,
            yes,
            json,
        } => present_runs_result(
            "runs.gc",
            json,
            crate::runs::gc(&root, before.as_deref(), yes),
            render_maintenance_report,
        ),
    }
}

fn present_runs_result<T: Serialize>(
    command: &'static str,
    json: bool,
    result: Result<T, crate::runs::RunsError>,
    render_human: impl FnOnce(&T),
) -> Result<i32, CliError> {
    match result {
        Ok(value) => {
            if json {
                print_json(&value)?;
            } else {
                render_human(&value);
            }
            Ok(0)
        }
        Err(error) if json => {
            print_json(&serde_json::json!({
                "api":crate::runs::RUNS_API,
                "command":command,
                "status":"error",
                "error":{
                    "code":error.code(),
                    "message":error.public_message(),
                }
            }))?;
            Ok(1)
        }
        Err(error) => Err(error.into()),
    }
}

fn render_run_list(result: &crate::runs::RunList) {
    render_presentation(&Presentation::new(
        PresentationCategory::Info,
        format!(
            "{} run journal(s) matched; {} shown.",
            result.matched,
            result.runs.len()
        ),
    ));
    for run in &result.runs {
        render_presentation(&Presentation::new(
            run_record_category(run),
            format!(
                "{}: {} (started {}, {} event(s), integrity {:?}).",
                run.run_id, run.state, run.started_unix_ms, run.events_recorded, run.integrity
            ),
        ));
    }
}

fn render_run_aggregate(result: &crate::runs::RunAggregate) {
    render_presentation(&Presentation::new(
        PresentationCategory::Info,
        format!(
            "{} run journal(s) matched; {} are protected from maintenance.",
            result.matched, result.protected
        ),
    ));
    for (state, count) in &result.states {
        render_presentation(&Presentation::new(
            if matches!(state.as_str(), "outcome_unknown" | "open" | "corrupt") {
                PresentationCategory::Warning
            } else {
                PresentationCategory::Info
            },
            format!("{state}: {count}"),
        ));
    }
}

fn render_run_show(result: &crate::runs::RunShow) {
    render_presentation(&Presentation::new(
        run_record_category(&result.run),
        format!(
            "{}: {} (integrity {:?}, {} event(s)).",
            result.run.run_id, result.run.state, result.run.integrity, result.run.events_recorded
        ),
    ));
    for event in &result.page.events {
        if let Some(presentation) = event.presentation.as_ref() {
            render_presentation(presentation);
        } else {
            render_presentation(&Presentation::new(
                PresentationCategory::Info,
                format!(
                    "Event #{} at {}: {}.",
                    event.seq, event.at_unix_ms, event.kind
                ),
            ));
        }
    }
    if !result.page.complete {
        render_presentation(&Presentation::new(
            PresentationCategory::Warning,
            format!(
                "More or partial event evidence remains; continue after sequence {}.",
                result.page.next_after
            ),
        ));
    }
}

fn render_maintenance_report(result: &crate::runs::MaintenanceReport) {
    render_presentation(&Presentation::new(
        PresentationCategory::State,
        if result.dry_run {
            format!(
                "Run {} preview selected {} action(s); no files changed.",
                result.operation,
                result.actions.len()
            )
        } else {
            format!(
                "Run {} applied {} action(s).",
                result.operation,
                result.actions.len()
            )
        },
    ));
    for action in &result.actions {
        render_presentation(&Presentation::new(
            PresentationCategory::Info,
            format!("{}: {}.", action.run_id, action.action),
        ));
    }
    for protected in &result.protected {
        render_presentation(&Presentation::new(
            PresentationCategory::Warning,
            format!("{} was preserved: {}.", protected.run_id, protected.reason),
        ));
    }
}

fn run_record_category(run: &crate::runs::RunRecord) -> PresentationCategory {
    if matches!(run.state.as_str(), "outcome_unknown" | "open" | "corrupt") {
        PresentationCategory::Warning
    } else {
        PresentationCategory::Info
    }
}

fn session(command: SessionCommand) -> Result<i32, CliError> {
    let (name, result) = match command {
        SessionCommand::List { root, limit } => (
            "session.list",
            crate::session::list(&root, limit).and_then(|value| {
                serde_json::to_value(value).map_err(|error| {
                    crate::session::SessionError::Corrupt(format!(
                        "cannot encode session list: {error}"
                    ))
                })
            }),
        ),
        SessionCommand::Show { root, session } => (
            "session.show",
            crate::session::show(&root, &session).and_then(|value| {
                serde_json::to_value(value).map_err(|error| {
                    crate::session::SessionError::Corrupt(format!("cannot encode session: {error}"))
                })
            }),
        ),
        SessionCommand::Answer {
            root,
            session,
            turn,
            axis,
            option,
            note,
        } => (
            "session.answer",
            crate::session::answer(&root, &session, &turn, &axis, &option, note.as_deref())
                .and_then(|value| {
                    serde_json::to_value(value).map_err(|error| {
                        crate::session::SessionError::Corrupt(format!(
                            "cannot encode session: {error}"
                        ))
                    })
                }),
        ),
    };
    match result {
        Ok(data) => {
            print_json(&ControlSuccess::new(name, data))?;
            Ok(0)
        }
        Err(error) => {
            print_json(&SessionControlFailure::new(name, &error))?;
            Ok(1)
        }
    }
}

fn runtime_json(start: &Path) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    print_json(&runtime_document(&workspace)?)?;
    Ok(0)
}

#[derive(Clone, Copy)]
struct ScriptSelection<'a> {
    explicit: &'a [PathBuf],
    all: bool,
    from: Option<u16>,
    through: Option<u16>,
}

impl<'a> ScriptSelection<'a> {
    fn new(explicit: &'a [PathBuf], all: bool, from: Option<u16>, through: Option<u16>) -> Self {
        Self {
            explicit,
            all,
            from,
            through,
        }
    }
}

fn check(
    start: &Path,
    selection: ScriptSelection<'_>,
    additional_packages: &[String],
    keep_going: bool,
    timeout_seconds: Option<u64>,
) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let timeout_seconds =
        timeout_seconds.unwrap_or(workspace.load_config()?.limits.check_timeout_seconds);
    let cancellation = install_cancellation()?;
    let scripts = select_scripts(&workspace, selection, false)?;
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
    if !build.is_success() {
        return Ok(command_exit_code(&build));
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
        let outcome = execute_tool(
            &workspace,
            command,
            environment,
            timeout_seconds,
            &cancellation,
        )?;
        let status = command_exit_code(&outcome);
        if !outcome.is_success() {
            first_failure = first_failure.max(status);
            if outcome.kind != CommandKind::Exited {
                break;
            }
            if !keep_going {
                break;
            }
        }
    }
    Ok(first_failure)
}

fn run_scripts_command(
    start: &Path,
    selection: ScriptSelection<'_>,
    additional_packages: &[String],
    arguments: &[String],
    keep_going: bool,
    timeout_seconds: Option<u64>,
) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let timeout_seconds =
        timeout_seconds.unwrap_or(workspace.load_config()?.limits.script_timeout_seconds);
    let cancellation = install_cancellation()?;
    let scripts = select_scripts(&workspace, selection, true)?;
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
    let mut script_results = Vec::new();
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
                        ClefSidecarFacts::default()
                    }
                    ScriptBatchObservation::Diagnostics(diagnostic_path) => {
                        let authoritative = inspect_clef_sidecar_facts(diagnostic_path);
                        match import_clef_diagnostic_sidecar(&mut journal, diagnostic_path) {
                            Ok(mut facts) => {
                                facts.outcome_unknown |= authoritative.outcome_unknown;
                                facts.observation_ambiguous |= authoritative.observation_ambiguous;
                                facts
                            }
                            Err(ClefSidecarError::Journal(error)) => {
                                note_journal_degradation(&mut journal_degradation, error);
                                authoritative
                            }
                            Err(error) => {
                                note_observation_degradation(&mut clef_observation_error, error);
                                ClefSidecarFacts {
                                    observation_ambiguous: true,
                                    ..authoritative
                                }
                            }
                        }
                    }
                    ScriptBatchObservation::ScriptFinished {
                        script,
                        outcome,
                        clef_outcome_unknown,
                    } => {
                        let result = ScriptExecutionResult {
                            script: workspace_relative_script(&workspace, script),
                            exit_code: outcome.exit_code,
                            command_kind: outcome.kind,
                            outcome_unknown: clef_outcome_unknown
                                || outcome.kind != CommandKind::Exited
                                || outcome.exit_code.is_none(),
                        };
                        if journal_degradation.is_none()
                            && let Err(error) = journal.record_with_presentation(
                                if outcome.is_success() && !clef_outcome_unknown {
                                    "script.completed"
                                } else if result.outcome_unknown {
                                    "script.outcome_unknown"
                                } else {
                                    "script.failed"
                                },
                                serde_json::to_value(&result)
                                    .expect("script result serialization is infallible"),
                                Some(Presentation::new(
                                    if outcome.is_success() && !clef_outcome_unknown {
                                        PresentationCategory::Info
                                    } else if result.outcome_unknown {
                                        PresentationCategory::Warning
                                    } else {
                                        PresentationCategory::Error
                                    },
                                    format!(
                                        "Script {} ended as {:?} with exit code {:?}.",
                                        result.script, result.command_kind, result.exit_code
                                    ),
                                )),
                            )
                        {
                            note_journal_degradation(&mut journal_degradation, error);
                        }
                        script_results.push(result);
                        ClefSidecarFacts::default()
                    }
                },
            )
        }
        Err(error) => Err(error),
    };
    let mut outcome =
        script_batch_outcome(&result, scripts.len(), &script_results, started.elapsed());
    let batch_exit_code = script_batch_exit_code(&result);
    outcome.observation_error = clef_observation_error.clone();
    let state_after = classify_outcome(&outcome).as_str();
    let terminal_presentation = Presentation::new(
        PresentationCategory::State,
        match state_after {
            "succeeded" => format!("Script batch {run_id} succeeded."),
            "outcome_unknown" => {
                format!("Script batch {run_id} entered the outcome-unknown state.")
            }
            _ => format!("Script batch {run_id} entered the failed state."),
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
                } else if state_after == "outcome_unknown" {
                    "script_batch.outcome_unknown"
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
                "scripts":script_results,
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
    if journal_degradation.is_none()
        && outcome_is_unknown(&outcome)
        && let Err(error) = journal.record_with_presentation(
            "runtime.outcome_unknown",
            outcome_unknown_diagnostic_value("workflow", "script_batch", "run", &outcome),
            Some(Presentation::new(
                PresentationCategory::Warning,
                "A workflow script may have completed externally; Tactus did not retry it automatically.",
            )),
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
    result.map(|_| batch_exit_code)
}

enum ScriptBatchObservation<'a> {
    Prepared,
    Diagnostics(&'a Path),
    ScriptFinished {
        script: &'a Path,
        outcome: &'a CommandOutcome,
        clef_outcome_unknown: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct ScriptExecutionResult {
    script: String,
    exit_code: Option<i32>,
    command_kind: CommandKind,
    outcome_unknown: bool,
}

#[derive(Clone, Debug)]
struct ScriptBatchExecution {
    command: CommandOutcome,
    script_started: bool,
    clef_outcome_unknown: bool,
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
    mut on_observation: impl FnMut(ScriptBatchObservation<'_>) -> ClefSidecarFacts,
) -> Result<ScriptBatchExecution, CliError> {
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
    if !build.is_success() {
        return Ok(ScriptBatchExecution {
            command: build,
            script_started: false,
            clef_outcome_unknown: false,
        });
    }
    on_observation(ScriptBatchObservation::Prepared);
    let include = format!("-i{}", workspace.scripts_path.display());
    let mut first_failure = 0;
    let mut elapsed_ms = build.elapsed_ms;
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
            "TACTUS_WORKFLOW_NAME".to_owned(),
            workspace_relative_script(workspace, script),
        );
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
        let facts = on_observation(ScriptBatchObservation::Diagnostics(&diagnostic_path));
        let status = status?;
        let clef_outcome_unknown =
            facts.outcome_unknown || (facts.observation_ambiguous && !status.is_success());
        elapsed_ms = elapsed_ms.saturating_add(status.elapsed_ms);
        on_observation(ScriptBatchObservation::ScriptFinished {
            script,
            outcome: &status,
            clef_outcome_unknown,
        });
        if status.kind != CommandKind::Exited || status.exit_code.is_none() || clef_outcome_unknown
        {
            return Ok(ScriptBatchExecution {
                command: status,
                script_started: true,
                clef_outcome_unknown,
            });
        }
        let exit_code = status.exit_code.unwrap_or(1);
        if exit_code != 0 {
            first_failure = first_failure.max(exit_code);
            if !keep_going {
                break;
            }
        }
    }
    Ok(ScriptBatchExecution {
        command: CommandOutcome {
            kind: CommandKind::Exited,
            exit_code: Some(first_failure),
            error: None,
            elapsed_ms,
        },
        script_started: true,
        clef_outcome_unknown: false,
    })
}

fn workspace_relative_script(workspace: &Workspace, script: &Path) -> String {
    script
        .strip_prefix(&workspace.root)
        .unwrap_or(script)
        .to_string_lossy()
        .replace('\\', "/")
}

fn script_batch_outcome(
    result: &Result<ScriptBatchExecution, CliError>,
    script_count: usize,
    script_results: &[ScriptExecutionResult],
    elapsed: Duration,
) -> ProcessOutcome {
    let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(execution) if execution.command.is_success() && !execution.clef_outcome_unknown => {
            ProcessOutcome {
                kind: InvocationKind::Succeeded,
                exit_code: Some(0),
                terminal: Some(TerminalResult::Success {
                    value: serde_json::json!({
                        "script_count":script_count,
                        "completed_script_count":script_results.len(),
                        "scripts":script_results,
                        "exit_code":0
                    }),
                }),
                frames_seen: 0,
                events_dropped: 0,
                observation_error: None,
                stderr: String::new(),
                stderr_truncated: false,
                error: None,
                elapsed_ms,
                progress: None,
            }
        }
        Ok(execution) if !execution.script_started => ProcessOutcome {
            kind: InvocationKind::PluginFailed,
            exit_code: execution.command.exit_code,
            terminal: Some(TerminalResult::Failure {
                error: PluginFailure {
                    code: "script_preparation_failed".to_owned(),
                    message: "script batch preparation did not complete successfully".to_owned(),
                    details: Some(serde_json::json!({
                        "script_count":script_count,
                        "completed_script_count":script_results.len(),
                        "scripts":script_results,
                        "command_kind":execution.command.kind,
                        "exit_code":execution.command.exit_code,
                    })),
                },
            }),
            frames_seen: 0,
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: execution.command.error.clone(),
            elapsed_ms,
            progress: None,
        },
        Ok(execution) if execution.clef_outcome_unknown => ProcessOutcome {
            kind: InvocationKind::PluginFailed,
            exit_code: execution.command.exit_code,
            terminal: Some(TerminalResult::Failure {
                error: PluginFailure {
                    code: "outcome_unknown".to_owned(),
                    message: "a Clef workflow reported an ambiguous external result".to_owned(),
                    details: Some(serde_json::json!({
                        "script_count":script_count,
                        "completed_script_count":script_results.len(),
                        "scripts":script_results,
                    })),
                },
            }),
            frames_seen: 1,
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: execution.command.error.clone(),
            elapsed_ms,
            progress: Some(script_command_progress(
                &execution.command,
                InvocationPhase::TerminalReceived,
            )),
        },
        Ok(execution) => {
            let kind = match execution.command.kind {
                CommandKind::Exited if execution.command.exit_code.is_none() => {
                    InvocationKind::RuntimeFailed
                }
                CommandKind::Exited => InvocationKind::PluginFailed,
                CommandKind::DeadlineExceeded => InvocationKind::DeadlineExceeded,
                CommandKind::Cancelled => InvocationKind::Cancelled,
                CommandKind::RuntimeFailed => InvocationKind::RuntimeFailed,
            };
            let terminal = (execution.command.kind == CommandKind::Exited
                && execution.command.exit_code.is_some())
            .then(|| TerminalResult::Failure {
                error: PluginFailure {
                    code: "script_batch_failed".to_owned(),
                    message: format!(
                        "script batch exited with code {}",
                        execution.command.exit_code.unwrap_or(1)
                    ),
                    details: Some(serde_json::json!({
                        "script_count":script_count,
                        "completed_script_count":script_results.len(),
                        "scripts":script_results,
                    })),
                },
            });
            ProcessOutcome {
                kind,
                exit_code: execution.command.exit_code,
                terminal,
                frames_seen: 0,
                events_dropped: 0,
                observation_error: None,
                stderr: String::new(),
                stderr_truncated: false,
                error: execution.command.error.clone().or_else(|| {
                    (execution.command.kind == CommandKind::Exited
                        && execution.command.exit_code.is_none())
                    .then(|| "script process terminated without an exit code".to_owned())
                }),
                elapsed_ms,
                progress: (execution.command.kind != CommandKind::Exited
                    || execution.command.exit_code.is_none())
                .then(|| script_command_progress(&execution.command, InvocationPhase::Dispatched)),
            }
        }
        Err(error) => ProcessOutcome {
            kind: InvocationKind::PluginFailed,
            exit_code: Some(1),
            terminal: Some(TerminalResult::Failure {
                error: PluginFailure {
                    code: "script_supervisor_start_failed".to_owned(),
                    message: error.to_string(),
                    details: None,
                },
            }),
            frames_seen: 0,
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: None,
            elapsed_ms,
            progress: None,
        },
    }
}

fn script_batch_exit_code(result: &Result<ScriptBatchExecution, CliError>) -> i32 {
    match result {
        Ok(execution) if execution.command.is_success() && !execution.clef_outcome_unknown => 0,
        Ok(execution) if execution.command.kind == CommandKind::Exited => execution
            .command
            .exit_code
            .filter(|code| *code != 0)
            .unwrap_or(1),
        Ok(_) | Err(_) => 1,
    }
}

fn script_command_progress(command: &CommandOutcome, phase: InvocationPhase) -> InvocationProgress {
    InvocationProgress {
        phase,
        dispatched_unix_ms: current_unix_millis().saturating_sub(command.elapsed_ms),
        first_response_unix_ms: None,
        last_event_unix_ms: None,
        cleanup_completed: matches!(
            command.kind,
            CommandKind::Exited | CommandKind::DeadlineExceeded | CommandKind::Cancelled
        ),
    }
}

fn generate(
    start: &Path,
    goal: &str,
    selected_provider: Option<&str>,
    timeout_seconds: Option<u64>,
    json: bool,
) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let cancellation = install_cancellation()?;
    let config = workspace.load_config()?;
    let timeout_seconds = timeout_seconds.unwrap_or(config.limits.provider_outer_timeout_seconds);
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
        timeout_seconds: config.limits.plugin_timeout_seconds,
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
                timeout_seconds: config.limits.plugin_timeout_seconds,
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
    let observer_outcome = generation_observer_outcome(
        provider_report.as_ref(),
        &evidence,
        provider_error.as_deref(),
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
    let (scripts, generated_delta, workspace_inspection_error) = match discover_scripts(&workspace)
    {
        Ok(scripts) => match generated_script_delta(&script_baseline, &scripts) {
            Ok(delta) => (scripts, delta, None),
            Err(error) => (scripts, Vec::new(), Some(error.to_string())),
        },
        Err(error) => (Vec::new(), Vec::new(), Some(error.to_string())),
    };
    record_journal_event(
        &mut generation_journal,
        &mut generation_journal_degradation,
        "generation.discovered_scripts",
        serde_json::json!({
            "scripts":scripts,
            "generated_delta":generated_delta,
            "inspection_error":workspace_inspection_error,
        }),
    );
    let provider_ok = provider_report
        .as_ref()
        .is_some_and(|report| report.summary.outcome.is_success())
        && provider_error.is_none();
    let generation_error = (provider_ok
        && workspace_inspection_error.is_none()
        && generated_delta.is_empty())
    .then(|| {
        "provider completed successfully but created or modified no non-empty numbered Haskell entry"
            .to_owned()
    });
    // Ambiguity has priority over later, known cleanup/no-delta failures.  An
    // observer is an external effect too, so losing its terminal fact makes
    // the aggregate unsafe to retry even when the provider itself was known.
    let unknown_outcome = provider_report
        .as_ref()
        .map(|report| &report.summary.outcome)
        .filter(|outcome| outcome_is_unknown(outcome))
        .or_else(|| {
            evidence.iter().find_map(|item| {
                item.report
                    .as_ref()
                    .map(|report| &report.summary.outcome)
                    .filter(|outcome| outcome_is_unknown(outcome))
            })
        });
    let inspection_unknown = workspace_inspection_error.as_deref().and_then(|error| {
        provider_report
            .as_ref()
            .map(|report| post_execution_inspection_unknown(error, &report.summary.outcome))
    });
    let generation_outcome = if let Some(outcome) = unknown_outcome {
        outcome.clone()
    } else if let Some(outcome) = inspection_unknown {
        outcome
    } else if !observer_errors.is_empty() {
        known_runtime_failure(
            "observer_cleanup_failed",
            &format!("observer cleanup failed: {}", observer_errors.join("; ")),
        )
    } else if let Some(error) = generation_error.as_deref() {
        known_runtime_failure("generation_produced_no_script", error)
    } else if let Some(error) = workspace_inspection_error.as_deref() {
        known_runtime_failure("workspace_inspection_failed", error)
    } else if let Some(report) = provider_report.as_ref() {
        report.summary.outcome.clone()
    } else {
        known_runtime_failure(
            "provider_invocation_failed",
            provider_error
                .as_deref()
                .unwrap_or("provider was not invoked"),
        )
    };
    let generation_state = classify_outcome(&generation_outcome).as_str();
    let success = provider_ok
        && generation_error.is_none()
        && workspace_inspection_error.is_none()
        && observer_errors.is_empty()
        && generation_state == "succeeded";
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
                "workspace_inspection_error":workspace_inspection_error.as_deref(),
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
            outcome_unknown_diagnostic_value(
                "workflow",
                "generate",
                "run",
                &generation_outcome,
            ),
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
            "workspace_inspection_error": workspace_inspection_error,
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
        if let Some(error) = workspace_inspection_error.as_deref() {
            render_presentation(&Presentation::new(
                PresentationCategory::Error,
                format!("Workspace inspection failed after generation: {error}"),
            ));
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

fn generation_observer_outcome(
    provider_report: Option<&InvocationReport>,
    evidence: &[ObserverEvidence],
    provider_error: Option<&str>,
) -> &'static str {
    let begin_reports = evidence.iter().filter(|item| item.phase == "observe.begin");
    if begin_reports
        .clone()
        .filter_map(|item| item.report.as_ref())
        .any(|report| classify_outcome(&report.summary.outcome) == OutcomeState::OutcomeUnknown)
    {
        return "outcome_unknown";
    }
    if begin_reports.clone().any(|item| {
        item.error.is_some()
            || item.report.as_ref().is_some_and(|report| {
                classify_outcome(&report.summary.outcome) != OutcomeState::Succeeded
            })
    }) {
        return "begin_error";
    }
    if let Some(report) = provider_report {
        return match classify_outcome(&report.summary.outcome) {
            OutcomeState::Succeeded => "ok",
            OutcomeState::Failed => "error",
            OutcomeState::OutcomeUnknown => "outcome_unknown",
        };
    }
    debug_assert!(provider_error.is_some());
    "error"
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

fn known_runtime_failure(code: &str, message: &str) -> ProcessOutcome {
    ProcessOutcome {
        kind: InvocationKind::PluginFailed,
        exit_code: Some(1),
        terminal: Some(TerminalResult::Failure {
            error: PluginFailure {
                code: code.to_owned(),
                message: message.to_owned(),
                details: None,
            },
        }),
        frames_seen: 0,
        events_dropped: 0,
        observation_error: None,
        stderr: String::new(),
        stderr_truncated: false,
        error: None,
        elapsed_ms: 0,
        progress: None,
    }
}

fn post_execution_inspection_unknown(message: &str, basis: &ProcessOutcome) -> ProcessOutcome {
    ProcessOutcome {
        kind: InvocationKind::RuntimeFailed,
        exit_code: None,
        terminal: None,
        frames_seen: basis.frames_seen,
        events_dropped: basis.events_dropped,
        observation_error: basis.observation_error.clone(),
        stderr: String::new(),
        stderr_truncated: basis.stderr_truncated || !basis.stderr.is_empty(),
        error: Some(format!(
            "workspace inspection failed after provider execution: {message}"
        )),
        elapsed_ms: basis.elapsed_ms,
        progress: basis.progress.clone().or_else(|| {
            Some(InvocationProgress {
                phase: InvocationPhase::TerminalReceived,
                dispatched_unix_ms: current_unix_millis().saturating_sub(basis.elapsed_ms),
                first_response_unix_ms: None,
                last_event_unix_ms: None,
                cleanup_completed: true,
            })
        }),
    }
}

fn process_error_outcome(error: &ProcessError) -> ProcessOutcome {
    if matches!(error, ProcessError::MissingPipe(_)) {
        ProcessOutcome {
            kind: InvocationKind::RuntimeFailed,
            exit_code: None,
            terminal: None,
            frames_seen: 0,
            events_dropped: 0,
            observation_error: None,
            stderr: String::new(),
            stderr_truncated: false,
            error: Some(error.to_string()),
            elapsed_ms: 0,
            progress: Some(InvocationProgress {
                phase: InvocationPhase::Dispatched,
                dispatched_unix_ms: current_unix_millis(),
                first_response_unix_ms: None,
                last_event_unix_ms: None,
                cleanup_completed: true,
            }),
        }
    } else {
        known_runtime_failure("plugin_start_failed", &error.to_string())
    }
}

fn select_scripts(
    workspace: &Workspace,
    selection: ScriptSelection<'_>,
    entries_only: bool,
) -> Result<Vec<PathBuf>, CliError> {
    let ScriptSelection {
        explicit,
        all,
        from,
        through,
    } = selection;
    let range_selected = from.is_some() || through.is_some();
    if !explicit.is_empty() && (all || range_selected) {
        return Err(CliError::InvalidArguments(
            "explicit script paths cannot be combined with --all, --from, or --through".to_owned(),
        ));
    }
    if all && range_selected {
        return Err(CliError::InvalidArguments(
            "--all cannot be combined with --from or --through".to_owned(),
        ));
    }
    if from.is_some_and(|value| value > 999) || through.is_some_and(|value| value > 999) {
        return Err(CliError::InvalidArguments(
            "entry orders must be between 000 and 999".to_owned(),
        ));
    }
    if from
        .zip(through)
        .is_some_and(|(from, through)| from > through)
    {
        return Err(CliError::InvalidArguments(
            "--from must not be greater than --through".to_owned(),
        ));
    }
    if explicit.is_empty() && !all && !range_selected {
        return Err(CliError::InvalidArguments(
            if entries_only {
                "select at least one entry with --script, --all, --from, or --through"
            } else {
                "select at least one source path, --all, --from, or --through"
            }
            .to_owned(),
        ));
    }
    let discovered = discover_scripts(workspace)?;
    if !explicit.is_empty() {
        let mut selected = Vec::with_capacity(explicit.len());
        for value in explicit {
            let candidate = if value.is_absolute() {
                value.clone()
            } else {
                workspace.root.join(value)
            };
            let named_metadata = fs::symlink_metadata(&candidate)?;
            if named_metadata.file_type().is_symlink() {
                return Err(CliError::InvalidArguments(format!(
                    "script path must not be a symbolic link: {}",
                    value.display()
                )));
            }
            let resolved = dunce::canonicalize(candidate)?;
            let Some(script) = discovered.iter().find(|script| script.path == resolved) else {
                return Err(CliError::InvalidArguments(format!(
                    "script must be a discovered Haskell source below .tactus/scripts: {}",
                    value.display()
                )));
            };
            if entries_only && !script.runnable {
                return Err(CliError::InvalidArguments(format!(
                    "run accepts only numbered NNN_slug.hs or NNN_slug.lhs entries: {}",
                    value.display()
                )));
            }
            if !selected.contains(&resolved) {
                selected.push(resolved);
            }
        }
        return Ok(selected);
    }
    Ok(discovered
        .into_iter()
        .filter(|script| {
            if range_selected {
                script.order.is_some_and(|order| {
                    from.is_none_or(|lower| order >= lower)
                        && through.is_none_or(|upper| order <= upper)
                })
            } else {
                all && (!entries_only || script.runnable)
            }
        })
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ClefSidecarFacts {
    imported: usize,
    outcome_unknown: bool,
    observation_ambiguous: bool,
}

/// Classify an authoritative Clef workflow transition independently from
/// journal projection.  A degraded journal or malformed trailing observation
/// must never turn an already-recorded ambiguous external result into a known
/// failure.
fn inspect_clef_sidecar_facts(path: &Path) -> ClefSidecarFacts {
    let mut facts = ClefSidecarFacts::default();
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return facts;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        facts.observation_ambiguous = true;
        return facts;
    }
    let Ok(file) = fs::File::open(path) else {
        facts.observation_ambiguous = true;
        return facts;
    };
    if metadata.len() > MAX_CLEF_SIDECAR_BYTES {
        facts.observation_ambiguous = true;
    }
    let mut bytes = Vec::new();
    if file
        .take(MAX_CLEF_SIDECAR_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        facts.observation_ambiguous = true;
        return facts;
    }
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CLEF_SIDECAR_BYTES {
        facts.observation_ambiguous = true;
    }
    let mut records_seen = 0_usize;
    for raw_line in bytes.split(|byte| *byte == b'\n') {
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        if records_seen >= MAX_CLEF_SIDECAR_RECORDS || line.len() > MAX_CLEF_SIDECAR_LINE_BYTES {
            facts.observation_ambiguous = true;
            break;
        }
        records_seen += 1;
        let Ok(record) = serde_json::from_slice::<ClefSidecarRecord>(line) else {
            facts.observation_ambiguous = true;
            break;
        };
        if let ClefSidecarRecord::StateTransition {
            code,
            level,
            message,
            subject,
            state_before,
            trigger,
            guard,
            state_after,
            context,
        } = record
            && level == ClefPresentationLevel::State
            && !message.trim().is_empty()
            && message.len() <= 4_096
            && !subject.trim().is_empty()
            && is_stable_diagnostic_identifier(&subject)
            && is_stable_diagnostic_identifier(&state_before)
            && is_stable_diagnostic_identifier(&state_after)
            && guard.passed
            && state_before != state_after
            && clef_transition_is_outcome_unknown(&code, &state_after, &trigger, &context)
        {
            facts.outcome_unknown = true;
        }
    }
    facts
}

fn import_clef_diagnostic_sidecar(
    journal: &mut RunJournal,
    path: &Path,
) -> Result<ClefSidecarFacts, ClefSidecarError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ClefSidecarFacts::default());
        }
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

    let mut facts = ClefSidecarFacts::default();
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
                facts.outcome_unknown |=
                    clef_transition_is_outcome_unknown(&code, &state_after, &trigger, &context);
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
                let structured_context =
                    project_clef_transition_context(&code, &state_after, &context);
                let durable_message =
                    format!("Clef recorded transition {code}: {state_before} to {state_after}.");
                journal.record_transition(
                    state_before,
                    TransitionTrigger::new(trigger.kind, trigger.source, trigger.code)
                        .with_details(serde_json::json!({
                            "source":"clef.sidecar",
                            "code":code,
                            "evidence":evidence,
                            "structured_context":structured_context,
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
                        "context":project_clef_message_context(&code, &context),
                    }),
                    Some(Presentation::new(category, durable_message)),
                )?;
            }
        }
        facts.imported += 1;
    }
    Ok(facts)
}

fn clef_transition_is_outcome_unknown(
    code: &str,
    state_after: &str,
    trigger: &ClefTransitionTrigger,
    context: &Map<String, Value>,
) -> bool {
    code == "workflow.result.error"
        && state_after == "outcome_unknown"
        && trigger.code == "workflow.result.error"
        && trigger.source == "clef.workflow"
        && context
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            == Some("plugin.outcome_unknown")
}

fn project_clef_transition_context(
    code: &str,
    state_after: &str,
    context: &Map<String, Value>,
) -> Value {
    if code != "workflow.result.error" || !matches!(state_after, "failed" | "outcome_unknown") {
        return diagnostic_value_summary(&Value::Object(context.clone()));
    }
    let Some(error) = context.get("error").and_then(Value::as_object) else {
        return diagnostic_value_summary(&Value::Object(context.clone()));
    };
    match error.get("code").and_then(Value::as_str) {
        Some("workflow.validation_failed") => project_clef_validation_error(error),
        Some("plugin.outcome_unknown") => project_clef_outcome_unknown_error(error),
        _ => None,
    }
    .unwrap_or_else(|| diagnostic_value_summary(&Value::Object(context.clone())))
}

fn project_clef_message_context(code: &str, context: &Map<String, Value>) -> Value {
    if code == "plugin.outcome_unknown"
        && let Some(cause) = context.get("cause").and_then(Value::as_object)
        && let Some(projected) = project_clef_outcome_unknown_cause(cause)
    {
        return projected;
    }
    diagnostic_value_summary(&Value::Object(context.clone()))
}

fn project_clef_validation_error(error: &Map<String, Value>) -> Option<Value> {
    let failures = error.get("validation_failed")?.as_array()?;
    if failures.is_empty() || failures.len() > 128 {
        return None;
    }
    let mut projected_failures = Vec::with_capacity(failures.len());
    for failure in failures {
        let fields = failure.as_object()?;
        let stage = fields.get("stage")?.as_str()?;
        if !matches!(stage, "structure" | "readability" | "domain" | "reviewer") {
            return None;
        }
        let severity = fields.get("severity")?.as_str()?;
        if !is_stable_diagnostic_identifier(severity) {
            return None;
        }
        let mut projected = Map::new();
        projected.insert("stage".to_owned(), Value::String(stage.to_owned()));
        projected.insert("severity".to_owned(), Value::String(severity.to_owned()));
        if let Some(rule) = fields
            .get("rule")
            .and_then(Value::as_str)
            .filter(|value| is_stable_diagnostic_identifier(value))
        {
            projected.insert("rule".to_owned(), Value::String(rule.to_owned()));
        }
        for name in ["expected", "observed", "provenance"] {
            projected.insert(name.to_owned(), fields.get(name)?.clone());
        }
        projected_failures.push(Value::Object(projected));
    }
    let source = Value::Object(error.clone());
    let mut projected = diagnostic_failure_details(&serde_json::json!({
        "code":"workflow.validation_failed",
        "validation_failed":projected_failures,
    }));
    if let Value::Object(fields) = &mut projected {
        fields.insert(
            "source_withheld".to_owned(),
            diagnostic_value_summary(&source),
        );
    }
    Some(projected)
}

fn project_clef_outcome_unknown_error(error: &Map<String, Value>) -> Option<Value> {
    let cause = error.get("cause")?.as_object()?;
    let mut projected = project_clef_outcome_unknown_cause(cause)?;
    let Value::Object(fields) = &mut projected else {
        return None;
    };
    fields.insert(
        "code".to_owned(),
        Value::String("plugin.outcome_unknown".to_owned()),
    );
    fields.insert(
        "source_withheld".to_owned(),
        diagnostic_value_summary(&Value::Object(error.clone())),
    );
    Some(projected)
}

fn project_clef_outcome_unknown_cause(cause: &Map<String, Value>) -> Option<Value> {
    let cause_code = cause.get("code")?.as_str()?;
    if !is_stable_diagnostic_identifier(cause_code) {
        return None;
    }
    let details = cause.get("details")?.as_object()?;
    let mut projected_details = Map::new();
    for name in ["frames_seen", "last_event_unix_ms"] {
        if let Some(Value::Number(value)) = details.get(name) {
            projected_details.insert(name.to_owned(), Value::Number(value.clone()));
        }
    }
    for name in ["external_effect_possible", "reported_details_withheld"] {
        if let Some(Value::Bool(value)) = details.get(name) {
            projected_details.insert(name.to_owned(), Value::Bool(*value));
        }
    }
    if let Some(phase) = details
        .get("phase")
        .and_then(Value::as_str)
        .filter(|value| is_stable_diagnostic_identifier(value))
    {
        projected_details.insert("phase".to_owned(), Value::String(phase.to_owned()));
    }
    if let Some(progress) = details.get("progress").and_then(Value::as_object) {
        let mut projected_progress = Map::new();
        if let Some(Value::Number(value)) = progress.get("event_frames_seen") {
            projected_progress.insert("event_frames_seen".to_owned(), Value::Number(value.clone()));
        }
        if let Some(Value::Bool(value)) = progress.get("terminal_frame_seen") {
            projected_progress.insert("terminal_frame_seen".to_owned(), Value::Bool(*value));
        }
        projected_details.insert("progress".to_owned(), Value::Object(projected_progress));
    }
    if let Some(last_event) = details.get("last_event").and_then(Value::as_object)
        && let Some(kind) = last_event
            .get("type")
            .and_then(Value::as_str)
            .filter(|value| is_stable_diagnostic_identifier(value))
    {
        projected_details.insert("last_event".to_owned(), serde_json::json!({"type":kind}));
    }
    if let Some(reconciliation) = details.get("reconciliation").and_then(Value::as_object) {
        let mut projected_reconciliation = Map::new();
        for name in ["required", "automatic_retry_safe"] {
            if let Some(Value::Bool(value)) = reconciliation.get(name) {
                projected_reconciliation.insert(name.to_owned(), Value::Bool(*value));
            }
        }
        projected_details.insert(
            "reconciliation".to_owned(),
            Value::Object(projected_reconciliation),
        );
    }
    let source = Value::Object(cause.clone());
    let mut projected = diagnostic_failure_details(&serde_json::json!({
        "cause":{
            "code":cause_code,
            "details":projected_details,
        }
    }));
    if let Value::Object(fields) = &mut projected {
        fields.insert(
            "source_withheld".to_owned(),
            diagnostic_value_summary(&source),
        );
    }
    Some(projected)
}

fn execute_tool(
    workspace: &Workspace,
    command: Vec<String>,
    environment: &BTreeMap<String, String>,
    timeout_seconds: u64,
    cancellation: &CancellationToken,
) -> Result<CommandOutcome, CliError> {
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
    Ok(outcome)
}

fn command_exit_code(outcome: &CommandOutcome) -> i32 {
    if outcome.is_success() {
        0
    } else if outcome.kind == CommandKind::Exited {
        outcome.exit_code.filter(|code| *code != 0).unwrap_or(1)
    } else {
        1
    }
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
    timeout_seconds: Option<u64>,
    json: bool,
) -> Result<i32, CliError> {
    let workspace = Workspace::discover(start)?;
    let cancellation = install_cancellation()?;
    let config = workspace.load_config()?;
    let definition = workspace.resolve_plugin(&config, name, namespace)?;
    let timeout_seconds =
        timeout_seconds.unwrap_or(default_invocation_timeout(&config.limits, definition, true));
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
        let definition = workspace.resolve_plugin(&config, &name, namespace)?;
        let timeout_seconds = default_invocation_timeout(&config.limits, definition, false);
        let mut params = Map::new();
        params.insert("live".to_owned(), Value::Bool(live));
        let report = invoke_registered(
            &workspace,
            &name,
            "smoke",
            params,
            InvocationControl {
                namespace,
                timeout_seconds,
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
    inject_registered_params(
        workspace,
        definition,
        &config.limits,
        control.timeout_seconds,
        method,
        &mut params,
    )?;
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
        format!(
            "{} started.",
            safe_invocation_subject(registry, name, method)
        ),
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
    apply_transport_limits(&mut spec, &config.limits);
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
    let outcome = invocation.unwrap_or_else(|error| process_error_outcome(&error));
    if let Some(journal) = journal.as_mut()
        && journal_degradation.is_none()
    {
        if let Err(error) = record_outcome_transition(journal, registry, name, method, &outcome) {
            note_journal_degradation(&mut journal_degradation, error);
        }
        if journal_degradation.is_none() && outcome_is_unknown(&outcome) {
            let warning_message = Presentation::new(
                PresentationCategory::Warning,
                format!(
                    "{registry}:{name} may have completed externally; Tactus did not retry it automatically."
                ),
            );
            if let Err(error) = journal.record_with_presentation(
                "runtime.outcome_unknown",
                outcome_unknown_diagnostic_value(registry, name, method, &outcome),
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
    if outcome_is_unknown(outcome) {
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
    limits: &RuntimeLimits,
    supervisor_timeout_seconds: u64,
    method: &str,
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
    if matches!(definition, ResolvedPlugin::Provider(_)) {
        for (name, value) in [
            ("native_max_line_bytes", limits.native_max_line_bytes),
            ("native_max_stdout_bytes", limits.native_max_stdout_bytes),
            ("native_max_result_bytes", limits.native_max_result_bytes),
            ("native_max_stderr_bytes", limits.native_max_stderr_bytes),
            (
                "native_output_queue_bound",
                limits.native_output_queue_bound,
            ),
        ] {
            options
                .entry(name.to_owned())
                .or_insert_with(|| Value::from(u64::try_from(value).unwrap_or(u64::MAX)));
        }
    }
    if let Some(provided) = params.remove("options") {
        let provided = provided.as_object().ok_or_else(|| {
            CliError::InvalidArguments("plugin params.options must be a JSON object".to_owned())
        })?;
        options.extend(provided.clone());
    }
    if matches!(definition, ResolvedPlugin::Provider(_)) {
        let configured_native_timeout = limits.provider_timeout_seconds.saturating_sub(60);
        let supervised_native_timeout = if supervisor_timeout_seconds == 0 {
            configured_native_timeout
        } else {
            supervisor_timeout_seconds
                .checked_sub(60)
                .filter(|timeout| *timeout > 0)
                .ok_or_else(|| {
                    CliError::InvalidArguments(
                        "provider supervisor timeout must be at least 61 seconds".to_owned(),
                    )
                })?
        };
        let mut native_timeout = configured_native_timeout.min(supervised_native_timeout);
        if method == "smoke" {
            // Provider health commands are expected to be quick. Do not let a
            // workspace's multi-hour generation budget replace the adapter's
            // short smoke deadline, while retaining supervisor cleanup room.
            native_timeout = native_timeout.min(20);
        }
        match options.get("timeout_seconds") {
            None => {
                options.insert("timeout_seconds".to_owned(), Value::from(native_timeout));
            }
            Some(Value::Number(value))
                if method == "smoke"
                    && value
                        .as_f64()
                        .is_some_and(|seconds| seconds > native_timeout as f64) =>
            {
                options.insert("timeout_seconds".to_owned(), Value::from(native_timeout));
            }
            Some(_) => {}
        }
        limits
            .validate_provider_options(&options, supervisor_timeout_seconds)
            .map_err(CliError::InvalidArguments)?;
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

fn apply_transport_limits(spec: &mut ProcessSpec, limits: &RuntimeLimits) {
    spec.limits.max_request_bytes = limits.max_request_bytes;
    spec.limits.max_frame_bytes = limits.max_frame_bytes;
    spec.limits.max_stdout_bytes = limits.max_stdout_bytes;
    spec.limits.max_frames = limits.max_event_frames;
    spec.limits.max_stderr_bytes = limits.max_stderr_bytes;
    spec.limits.event_queue_bound = limits.event_queue_bound;
}

fn default_invocation_timeout(
    limits: &RuntimeLimits,
    definition: ResolvedPlugin<'_>,
    provider_outer: bool,
) -> u64 {
    match definition {
        ResolvedPlugin::Provider(_) if provider_outer => limits.provider_outer_timeout_seconds,
        ResolvedPlugin::Provider(_) => limits.provider_timeout_seconds,
        ResolvedPlugin::Effect(_) | ResolvedPlugin::Plugin(_) => limits.plugin_timeout_seconds,
    }
}

fn dispatch(
    start: &Path,
    name: &str,
    namespace: PluginNamespace,
    timeout_seconds: Option<u64>,
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
    let timeout_seconds = timeout_seconds.unwrap_or(default_invocation_timeout(
        &config.limits,
        definition,
        false,
    ));
    let mut input = Vec::new();
    io::stdin()
        .take(u64::try_from(config.limits.max_request_bytes).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut input)?;
    if input.len() > config.limits.max_request_bytes {
        return Err(CliError::InvalidArguments(format!(
            "plugin request exceeds {} bytes",
            config.limits.max_request_bytes
        )));
    }
    let mut request =
        decode_request(&input).map_err(|error| CliError::InvalidArguments(error.to_string()))?;
    inject_registered_params(
        &workspace,
        definition,
        &config.limits,
        timeout_seconds,
        &request.method,
        &mut request.params,
    )?;
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
                    format!(
                        "{} started.",
                        safe_invocation_subject(&namespace_name, name, &request.method)
                    ),
                ),
            )
            .err()
    };
    let tool_runtime = ToolRuntime::create(&workspace)?;
    let mut spec = ProcessSpec::new(
        resolve_builtin_command(definition.command())?,
        &workspace.root,
    );
    apply_transport_limits(&mut spec, &config.limits);
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
    let outcome = invocation.unwrap_or_else(|error| process_error_outcome(&error));
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
        let transition_result =
            record_outcome_transition(journal, &namespace_name, name, &request.method, &outcome);
        if let Err(error) = transition_result {
            note_journal_degradation(&mut journal_degradation, error);
        }
        if journal_degradation.is_none()
            && outcome_is_unknown(&outcome)
            && let Err(error) = journal.record_with_presentation(
                "runtime.outcome_unknown",
                outcome_unknown_diagnostic_value(&namespace_name, name, &request.method, &outcome),
                Some(Presentation::new(
                    PresentationCategory::Warning,
                    format!(
                        "{} may have completed externally; Tactus did not retry it automatically.",
                        safe_presentation_identifier(name)
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

fn validate_entry_order(value: &str) -> Result<u16, String> {
    let order = value
        .parse::<u16>()
        .map_err(|_| "entry order must be an integer between 000 and 999".to_owned())?;
    if order > 999 {
        return Err("entry order must be between 000 and 999".to_owned());
    }
    Ok(order)
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
                    "details_summary":error.details.as_ref().map(diagnostic_value_summary),
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
    let state_after = classify_outcome(outcome).as_str();
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
                "side_effect_certainty":if outcome_is_unknown(outcome) {
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
                "details_summary":error.details.as_ref().map(diagnostic_value_summary),
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
    let subject = safe_invocation_subject(namespace, name, method);
    let message = match classify_outcome(outcome) {
        OutcomeState::Succeeded => format!("{subject} succeeded."),
        OutcomeState::Failed => format!("{subject} entered the failed state."),
        OutcomeState::OutcomeUnknown => {
            format!("{subject} entered the outcome-unknown state.")
        }
    };
    Presentation::new(PresentationCategory::State, message)
}

fn known_failure_presentation(
    namespace: &str,
    name: &str,
    outcome: &ProcessOutcome,
) -> Option<Presentation> {
    if classify_outcome(outcome) != OutcomeState::Failed {
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
    if classify_outcome(outcome) != OutcomeState::Failed {
        return None;
    }
    let subject = format!(
        "{}:{}",
        safe_presentation_identifier(namespace),
        safe_presentation_identifier(name)
    );
    let message = match outcome.terminal.as_ref() {
        Some(TerminalResult::Failure { .. }) => {
            format!("{subject} failed with a structured diagnostic.")
        }
        _ => format!("{subject} failed without a valid diagnostic."),
    };
    Some(Presentation::new(PresentationCategory::Error, message))
}

fn safe_invocation_subject(namespace: &str, name: &str, method: &str) -> String {
    format!(
        "{}:{} method {}",
        safe_presentation_identifier(namespace),
        safe_presentation_identifier(name),
        safe_presentation_method(method)
    )
}

fn safe_presentation_method(value: &str) -> String {
    if matches!(
        value,
        "invoke"
            | "smoke"
            | "observe.begin"
            | "observe.end"
            | "snapshot"
            | "diff"
            | "commit"
            | "rollback"
            | "run"
    ) {
        value.to_owned()
    } else {
        "<redacted-method>".to_owned()
    }
}

fn safe_presentation_identifier(value: &str) -> String {
    if is_stable_diagnostic_identifier(value) {
        value.to_owned()
    } else {
        "<redacted-identifier>".to_owned()
    }
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

fn outcome_unknown_diagnostic_value(
    namespace: &str,
    name: &str,
    method: &str,
    outcome: &ProcessOutcome,
) -> Value {
    let context = OutcomeContext {
        workflow: bounded_environment_context("TACTUS_WORKFLOW_NAME", 1_024),
        task: bounded_environment_context("TACTUS_TASK_NAME", 512),
        business_key_sha256: bounded_environment_context("TACTUS_BUSINESS_KEY_SHA256", 64)
            .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(|value| value.to_ascii_lowercase()),
        occurrence_id: bounded_environment_context("TACTUS_OCCURRENCE_ID", 512),
        provider: (namespace == "provider")
            .then(|| bounded_context_value(name, 128))
            .flatten(),
    };
    serde_json::to_value(OutcomeUnknownDiagnostic::from_outcome(
        context, namespace, method, outcome,
    ))
    .expect("typed outcome-unknown diagnostic serialization is infallible")
}

fn bounded_environment_context(name: &str, max_bytes: usize) -> Option<String> {
    env::var(name)
        .ok()
        .and_then(|value| bounded_context_value(&value, max_bytes))
}

fn bounded_context_value(value: &str, max_bytes: usize) -> Option<String> {
    (!value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
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
    /// Run-journal query or maintenance failed.
    #[error(transparent)]
    Runs(#[from] crate::runs::RunsError),
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

    fn batch_execution(
        kind: CommandKind,
        exit_code: Option<i32>,
        script_started: bool,
        clef_outcome_unknown: bool,
    ) -> Result<ScriptBatchExecution, CliError> {
        Ok(ScriptBatchExecution {
            command: CommandOutcome {
                kind,
                exit_code,
                error: None,
                elapsed_ms: 1,
            },
            script_started,
            clef_outcome_unknown,
        })
    }

    fn exited_batch(exit_code: i32) -> Result<ScriptBatchExecution, CliError> {
        batch_execution(CommandKind::Exited, Some(exit_code), true, false)
    }

    fn provider_definition(
        options: BTreeMap<String, Value>,
    ) -> crate::workspace::ProviderDefinition {
        crate::workspace::ProviderDefinition {
            command: vec![
                "tactus".to_owned(),
                "provider-host".to_owned(),
                "codex".to_owned(),
            ],
            model: None,
            effort: None,
            options,
        }
    }

    fn invocation_report(name: &str, outcome: ProcessOutcome) -> InvocationReport {
        InvocationReport {
            name: name.to_owned(),
            run_path: PathBuf::from("fixture-run"),
            summary: RunSummary {
                api: crate::journal::TRACE_API.to_owned(),
                run_id: format!("run-{name}"),
                started_unix_ms: 1,
                finished_unix_ms: 2,
                events_recorded: 1,
                outcome,
            },
            persisted: true,
        }
    }

    fn injected_provider_timeout(
        supervisor_timeout_seconds: u64,
        method: &str,
        definition: &crate::workspace::ProviderDefinition,
        provided_options: Option<Value>,
    ) -> Result<u64, CliError> {
        let workspace = Workspace::at("fixture");
        let mut params = Map::new();
        if let Some(options) = provided_options {
            params.insert("options".to_owned(), options);
        }
        inject_registered_params(
            &workspace,
            ResolvedPlugin::Provider(definition),
            &RuntimeLimits::default(),
            supervisor_timeout_seconds,
            method,
            &mut params,
        )?;
        params["options"]["timeout_seconds"]
            .as_u64()
            .ok_or_else(|| CliError::InvalidArguments("missing injected timeout".to_owned()))
    }

    #[test]
    fn provider_timeout_derivation_respects_workspace_policy_and_actual_supervisor() {
        let provider = provider_definition(BTreeMap::new());
        assert_eq!(
            injected_provider_timeout(14_400, "invoke", &provider, None).expect("default outer"),
            13_440
        );
        assert_eq!(
            injected_provider_timeout(7_200, "invoke", &provider, None).expect("short outer"),
            7_140
        );
        assert_eq!(
            injected_provider_timeout(20_000, "invoke", &provider, None).expect("long outer"),
            13_440
        );
        assert_eq!(
            injected_provider_timeout(0, "invoke", &provider, None).expect("unbounded outer"),
            13_440
        );
        assert_eq!(
            injected_provider_timeout(61, "invoke", &provider, None).expect("minimum outer"),
            1
        );

        let configured = provider_definition(BTreeMap::from([(
            "timeout_seconds".to_owned(),
            Value::from(7_000_u64),
        )]));
        assert_eq!(
            injected_provider_timeout(7_200, "invoke", &configured, None)
                .expect("registry override"),
            7_000
        );
        assert_eq!(
            injected_provider_timeout(
                7_200,
                "invoke",
                &configured,
                Some(serde_json::json!({"timeout_seconds":7_140})),
            )
            .expect("call override"),
            7_140
        );
        assert!(
            injected_provider_timeout(
                7_200,
                "invoke",
                &configured,
                Some(serde_json::json!({"timeout_seconds":7_150})),
            )
            .expect_err("call override without cleanup headroom")
            .to_string()
            .contains("leave at least 60 seconds")
        );
        assert!(
            injected_provider_timeout(60, "invoke", &provider, None)
                .expect_err("too-short supervisor")
                .to_string()
                .contains("at least 61 seconds")
        );
    }

    #[test]
    fn provider_smoke_timeout_stays_short_and_keeps_supervisor_headroom() {
        let provider = provider_definition(BTreeMap::new());
        assert_eq!(
            injected_provider_timeout(14_400, "smoke", &provider, None).expect("default smoke"),
            20
        );
        assert_eq!(
            injected_provider_timeout(70, "smoke", &provider, None).expect("short supervisor"),
            10
        );
        assert_eq!(
            injected_provider_timeout(0, "smoke", &provider, None).expect("unbounded supervisor"),
            20
        );

        let configured_short = provider_definition(BTreeMap::from([(
            "timeout_seconds".to_owned(),
            Value::from(5_u64),
        )]));
        assert_eq!(
            injected_provider_timeout(14_400, "smoke", &configured_short, None)
                .expect("short registry timeout"),
            5
        );

        let configured_long = provider_definition(BTreeMap::from([(
            "timeout_seconds".to_owned(),
            Value::from(7_000_u64),
        )]));
        assert_eq!(
            injected_provider_timeout(14_400, "smoke", &configured_long, None)
                .expect("long registry timeout is clamped"),
            20
        );
        assert_eq!(
            injected_provider_timeout(
                14_400,
                "smoke",
                &configured_long,
                Some(serde_json::json!({"timeout_seconds":8})),
            )
            .expect("short call timeout"),
            8
        );
    }

    #[test]
    fn supervised_command_never_reports_timeout_or_cancellation_as_success() {
        for kind in [
            CommandKind::DeadlineExceeded,
            CommandKind::Cancelled,
            CommandKind::RuntimeFailed,
        ] {
            let outcome = CommandOutcome {
                kind,
                exit_code: Some(0),
                error: None,
                elapsed_ms: 1,
            };
            assert_eq!(command_exit_code(&outcome), 1);
        }
        assert_eq!(
            command_exit_code(&CommandOutcome {
                kind: CommandKind::Exited,
                exit_code: Some(0),
                error: None,
                elapsed_ms: 1,
            }),
            0
        );
    }

    #[test]
    fn observer_cleanup_receives_the_factual_upstream_classification() {
        let unknown = script_batch_outcome(
            &batch_execution(CommandKind::DeadlineExceeded, None, true, false),
            1,
            &[],
            Duration::from_millis(1),
        );
        let begin_unknown = vec![ObserverEvidence {
            effect: "second".to_owned(),
            phase: "observe.begin".to_owned(),
            report: Some(invocation_report("begin-unknown", unknown.clone())),
            error: None,
        }];
        assert_eq!(
            generation_observer_outcome(None, &begin_unknown, Some("begin failed")),
            "outcome_unknown"
        );

        let known_failure =
            script_batch_outcome(&exited_batch(1), 1, &[], Duration::from_millis(1));
        let begin_failed = vec![ObserverEvidence {
            effect: "second".to_owned(),
            phase: "observe.begin".to_owned(),
            report: Some(invocation_report("begin-failed", known_failure)),
            error: None,
        }];
        assert_eq!(
            generation_observer_outcome(None, &begin_failed, Some("begin failed")),
            "begin_error"
        );
        assert_eq!(
            generation_observer_outcome(None, &[], Some("provider spawn failed")),
            "error"
        );
        let provider_unknown = invocation_report("provider-unknown", unknown);
        assert_eq!(
            generation_observer_outcome(Some(&provider_unknown), &[], None),
            "outcome_unknown"
        );
    }

    #[test]
    fn post_provider_workspace_inspection_failure_is_not_safe_to_retry() {
        let provider_success =
            script_batch_outcome(&exited_batch(0), 1, &[], Duration::from_millis(1));
        let outcome = post_execution_inspection_unknown("fixture read failed", &provider_success);
        assert_eq!(classify_outcome(&outcome), OutcomeState::OutcomeUnknown);
        assert!(outcome.progress.is_some());
        assert!(
            outcome
                .error
                .as_deref()
                .is_some_and(|error| error.contains("inspection failed"))
        );
    }

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

    fn selection_fixture() -> (tempfile::TempDir, Workspace) {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        fs::create_dir_all(&workspace.scripts_path).expect("scripts directory");
        for (name, source) in [
            ("010_first.hs", "main = pure ()"),
            ("020_second.hs", "main = pure ()"),
            ("Support.hs", "module Support where"),
        ] {
            fs::write(workspace.scripts_path.join(name), source).expect("script");
        }
        (temporary, workspace)
    }

    #[test]
    fn script_selection_requires_an_explicit_mode() {
        let (_temporary, workspace) = selection_fixture();
        let none = ScriptSelection::new(&[], false, None, None);
        assert!(select_scripts(&workspace, none, false).is_err());
        assert!(select_scripts(&workspace, none, true).is_err());

        let all = ScriptSelection::new(&[], true, None, None);
        let checked = select_scripts(&workspace, all, false).expect("explicit check all");
        let run = select_scripts(&workspace, all, true).expect("explicit run all");
        assert_eq!(checked.len(), 3);
        assert_eq!(run.len(), 2);
    }

    #[test]
    fn script_selection_ranges_are_ordered_and_mutually_exclusive() {
        let (_temporary, workspace) = selection_fixture();
        let ranged = select_scripts(
            &workspace,
            ScriptSelection::new(&[], false, Some(15), Some(20)),
            true,
        )
        .expect("range");
        assert_eq!(
            ranged[0].file_name().and_then(|name| name.to_str()),
            Some("020_second.hs")
        );
        assert!(
            select_scripts(
                &workspace,
                ScriptSelection::new(&[], false, Some(21), Some(20)),
                true,
            )
            .is_err()
        );
        assert!(
            select_scripts(
                &workspace,
                ScriptSelection::new(&[], true, Some(10), None),
                true,
            )
            .is_err()
        );
        let explicit = [PathBuf::from(".tactus/scripts/010_first.hs")];
        assert!(
            select_scripts(
                &workspace,
                ScriptSelection::new(&explicit, true, None, None),
                true,
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_run_selection_rejects_helpers_and_workspace_escapes() {
        let (temporary, workspace) = selection_fixture();
        let helper = [PathBuf::from(".tactus/scripts/Support.hs")];
        assert!(
            select_scripts(
                &workspace,
                ScriptSelection::new(&helper, false, None, None),
                true,
            )
            .is_err()
        );
        let outside = temporary.path().join("outside.hs");
        fs::write(&outside, "main = pure ()").expect("outside fixture");
        assert!(
            select_scripts(
                &workspace,
                ScriptSelection::new(&[outside], false, None, None),
                false,
            )
            .is_err()
        );
        assert_eq!(validate_entry_order("999"), Ok(999));
        assert!(validate_entry_order("1000").is_err());
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
                progress: None,
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
                progress: None,
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

        let mut success = script_batch_outcome(&exited_batch(0), 1, &[], Duration::from_millis(1));
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

        let mut failure = script_batch_outcome(&exited_batch(1), 1, &[], Duration::from_millis(1));
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
            .finish(script_batch_outcome(
                &exited_batch(0),
                1,
                &[],
                Duration::from_millis(1),
            ))
            .expect("finish");

        let durable = fs::read_to_string(journal.event_path()).expect("events");
        assert!(!durable.contains("DO_NOT_PERSIST"), "{durable}");
        assert!(!durable.contains("provider.failed"), "{durable}");
        assert!(durable.contains("invalid_identifier."), "{durable}");
        assert!(durable.contains("diagnostic_summary"), "{durable}");
    }

    #[test]
    fn script_batch_outcome_keeps_known_success_and_failure_terminal() {
        let scripts = vec![
            ScriptExecutionResult {
                script: ".tactus/scripts/010_first.hs".to_owned(),
                exit_code: Some(0),
                command_kind: CommandKind::Exited,
                outcome_unknown: false,
            },
            ScriptExecutionResult {
                script: ".tactus/scripts/020_second.hs".to_owned(),
                exit_code: Some(9),
                command_kind: CommandKind::Exited,
                outcome_unknown: false,
            },
        ];
        let success =
            script_batch_outcome(&exited_batch(0), 2, &scripts[..1], Duration::from_millis(7));
        assert_eq!(success.kind, InvocationKind::Succeeded);
        match success.terminal {
            Some(TerminalResult::Success { value }) => {
                assert_eq!(value["completed_script_count"], 1);
                assert_eq!(value["scripts"][0]["script"], scripts[0].script);
            }
            other => panic!("unexpected success terminal: {other:?}"),
        }

        let failure = script_batch_outcome(&exited_batch(9), 3, &scripts, Duration::from_millis(7));
        assert_eq!(failure.kind, InvocationKind::PluginFailed);
        assert_eq!(failure.exit_code, Some(9));
        match failure.terminal {
            Some(TerminalResult::Failure { error }) => {
                let details = error.details.expect("batch details");
                assert_eq!(details["completed_script_count"], 2);
                assert_eq!(details["scripts"][1]["exit_code"], 9);
            }
            other => panic!("unexpected failure terminal: {other:?}"),
        }
    }

    #[test]
    fn script_batch_preserves_timeout_signal_exit_and_clef_ambiguity() {
        let deadline = batch_execution(CommandKind::DeadlineExceeded, Some(0), true, false);
        let deadline_outcome = script_batch_outcome(&deadline, 1, &[], Duration::from_millis(7));
        assert_eq!(
            classify_outcome(&deadline_outcome),
            OutcomeState::OutcomeUnknown
        );
        assert_eq!(script_batch_exit_code(&deadline), 1);
        assert!(deadline_outcome.progress.is_some());

        let signalled = batch_execution(CommandKind::Exited, None, true, false);
        let signalled_outcome = script_batch_outcome(&signalled, 1, &[], Duration::from_millis(7));
        assert_eq!(signalled_outcome.kind, InvocationKind::RuntimeFailed);
        assert_eq!(
            classify_outcome(&signalled_outcome),
            OutcomeState::OutcomeUnknown
        );
        assert_eq!(script_batch_exit_code(&signalled), 1);
        assert!(signalled_outcome.terminal.is_none());
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let mut journal = RunJournal::create(&workspace).expect("journal");
        journal
            .finish(signalled_outcome)
            .expect("signalled outcome remains persistable");

        let clef_unknown = batch_execution(CommandKind::Exited, Some(0), true, true);
        let clef_outcome = script_batch_outcome(&clef_unknown, 1, &[], Duration::from_millis(7));
        assert_eq!(
            classify_outcome(&clef_outcome),
            OutcomeState::OutcomeUnknown
        );
        assert_eq!(script_batch_exit_code(&clef_unknown), 1);
        match clef_outcome.terminal {
            Some(TerminalResult::Failure { error }) => {
                assert_eq!(error.code, "outcome_unknown");
            }
            other => panic!("unexpected Clef outcome: {other:?}"),
        }

        let build_timeout = batch_execution(CommandKind::DeadlineExceeded, Some(1), false, false);
        assert_eq!(
            classify_outcome(&script_batch_outcome(
                &build_timeout,
                1,
                &[],
                Duration::from_millis(7),
            )),
            OutcomeState::Failed,
            "preparation never dispatched workflow code"
        );
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
            import_clef_diagnostic_sidecar(&mut journal, &sidecar)
                .expect("import")
                .imported,
            2
        );
        journal
            .finish(script_batch_outcome(
                &exited_batch(0),
                1,
                &[],
                Duration::from_millis(1),
            ))
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
    fn clef_outcome_unknown_classification_survives_redaction_and_a_bad_tail() {
        let temporary = tempdir().expect("temporary directory");
        let workspace = Workspace::at(temporary.path());
        let sidecar = temporary.path().join("clef-unknown.jsonl");
        let transition = r#"{"type":"state_transition","code":"workflow.result.error","level":"state","message":"Workflow entered outcome unknown.","subject":"workflow:1","state_before":"running","trigger":{"kind":"internal_result","source":"clef.workflow","code":"workflow.result.error"},"guard":{"condition":"typed workflow result","passed":true,"reason":"typed error decides the state"},"state_after":"outcome_unknown","context":{"provider":"DO_NOT_PERSIST_PROVIDER","error":{"code":"plugin.outcome_unknown","message":"DO_NOT_PERSIST_MESSAGE","cause":{"code":"plugin.deadline_exceeded","message":"DO_NOT_PERSIST_CAUSE","details":{"phase":"awaiting_terminal","frames_seen":3,"progress":{"event_frames_seen":2,"terminal_frame_seen":false},"last_event":{"type":"provider.progress","secret":"DO_NOT_PERSIST_EVENT"},"last_event_unix_ms":42,"external_effect_possible":true,"reported_details_withheld":true,"reconciliation":{"required":true,"automatic_retry_safe":false,"steps":["DO_NOT_PERSIST_STEP"]}}}}}}"#;
        fs::write(&sidecar, format!("{transition}\n{{bad tail\n")).expect("sidecar with bad tail");
        let facts = inspect_clef_sidecar_facts(&sidecar);
        assert!(facts.outcome_unknown);

        let mut rejected_journal = RunJournal::create(&workspace).expect("journal");
        assert!(matches!(
            import_clef_diagnostic_sidecar(&mut rejected_journal, &sidecar),
            Err(ClefSidecarError::Invalid(_))
        ));
        let batch = batch_execution(CommandKind::Exited, Some(1), true, facts.outcome_unknown);
        assert_eq!(
            classify_outcome(&script_batch_outcome(
                &batch,
                1,
                &[],
                Duration::from_millis(1),
            )),
            OutcomeState::OutcomeUnknown
        );

        fs::write(&sidecar, format!("{transition}\n")).expect("valid sidecar");
        let mut journal = RunJournal::create(&workspace).expect("second journal");
        let imported = import_clef_diagnostic_sidecar(&mut journal, &sidecar).expect("import");
        assert!(imported.outcome_unknown);
        journal
            .finish(script_batch_outcome(
                &batch,
                1,
                &[],
                Duration::from_millis(1),
            ))
            .expect("finish");
        let durable = fs::read_to_string(journal.event_path()).expect("events");
        assert!(!durable.contains("DO_NOT_PERSIST"), "{durable}");
        assert!(durable.contains("plugin.outcome_unknown"), "{durable}");
        assert!(durable.contains("awaiting_terminal"), "{durable}");
        assert!(durable.contains("source_withheld"), "{durable}");
    }

    #[test]
    fn an_unobservable_oversized_clef_sidecar_makes_a_failed_script_ambiguous() {
        let temporary = tempdir().expect("temporary directory");
        let sidecar = temporary.path().join("oversized-clef.jsonl");
        fs::write(
            &sidecar,
            vec![b' '; usize::try_from(MAX_CLEF_SIDECAR_BYTES).unwrap() + 1],
        )
        .expect("oversized sidecar");
        let facts = inspect_clef_sidecar_facts(&sidecar);
        assert!(facts.observation_ambiguous);

        let execution = batch_execution(
            CommandKind::Exited,
            Some(1),
            true,
            facts.outcome_unknown || facts.observation_ambiguous,
        );
        assert_eq!(
            classify_outcome(&script_batch_outcome(
                &execution,
                1,
                &[],
                Duration::from_millis(1),
            )),
            OutcomeState::OutcomeUnknown
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
            .finish(script_batch_outcome(
                &exited_batch(0),
                1,
                &[],
                Duration::from_millis(1),
            ))
            .expect("finish");
        assert_eq!(summary.outcome.kind, InvocationKind::Succeeded);
        assert_eq!(summary.events_recorded, 1);
    }
}
