use std::{
    path::PathBuf,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{Parser, Subcommand};
use segno_core::{OccurrenceState, Sha256Digest, UtcInstant};
use segnod::{ArchiveBudget, SchedulerConfig, Segnod, StaticCompiler};
use serde_json::json;

#[derive(Parser)]
#[command(
    name = "segnod",
    version,
    about = "Durable Segno scheduler composition"
)]
struct Cli {
    /// Absolute Segno local-state root.
    #[arg(long)]
    root: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Preflight, stage, publish, and register a ZIP revision.
    Import {
        /// Portable task ZIP.
        package: PathBuf,
    },
    /// Enable an exact revision using a plan digest returned by agentrod.
    Enable {
        /// Stable task ID.
        task_id: String,
        /// Expected current revision.
        #[arg(long)]
        revision: u64,
        /// Verified `sha256:<hex>` Clef plan digest.
        #[arg(long)]
        plan_digest: String,
    },
    /// List a bounded stable page of tasks.
    List {
        /// Exclusive task-ID cursor.
        #[arg(long)]
        after: Option<String>,
        /// Page size, 1 through 200.
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Persist a manual occurrence; dispatch remains owned by the daemon loop.
    Run {
        /// Stable task ID.
        task_id: String,
        /// Deterministic UTC milliseconds for diagnostics/tests.
        #[arg(long)]
        at_ms: Option<i64>,
    },
    /// Return bounded scheduler/orchestration status.
    Status {
        /// Stable occurrence ID.
        occurrence_id: String,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let mut daemon = Segnod::open(
        &cli.root,
        ArchiveBudget::default(),
        SchedulerConfig::default(),
    )?;
    let now = now()?;
    match cli.command {
        Command::Import { package } => {
            let result = daemon.import_package(&package, now)?;
            println!(
                "{}",
                json!({
                    "task_id": result.task_id,
                    "revision": result.revision,
                    "package_digest": result.package_digest,
                    "workflow_spec_digest": result.workflow_spec_digest,
                    "enabled": false,
                })
            );
        }
        Command::Enable {
            task_id,
            revision,
            plan_digest,
        } => {
            let digest = Sha256Digest::parse(&plan_digest)?;
            let mut compiler = StaticCompiler::new(digest);
            let compiled = daemon.enable(&task_id, revision, &mut compiler, now)?;
            println!(
                "{}",
                json!({
                    "task_id": task_id,
                    "revision": revision,
                    "plan_digest": compiled.to_string(),
                    "enabled": true,
                })
            );
        }
        Command::List { after, limit } => {
            let page = daemon.list_tasks(after.as_deref(), limit)?;
            let tasks: Vec<_> = page
                .tasks
                .iter()
                .map(|task| {
                    json!({
                        "task_id": task.task_id,
                        "revision": task.revision,
                        "enabled": task.enabled,
                        "package_digest": task.package_digest,
                        "plan_digest": task.plan_digest,
                    })
                })
                .collect();
            println!("{}", json!({"tasks": tasks, "next_after": page.next_after}));
        }
        Command::Run { task_id, at_ms } => {
            let scheduled_for = UtcInstant::from_millis(at_ms.unwrap_or(now.as_millis()));
            let occurrence = daemon.run_now(&task_id, scheduled_for)?;
            println!(
                "{}",
                json!({
                    "task_id": task_id,
                    "occurrence_id": occurrence.as_str(),
                    "state": "queued",
                })
            );
        }
        Command::Status { occurrence_id } => {
            let status = daemon.status(&occurrence_id)?;
            println!(
                "{}",
                json!({
                    "occurrence_id": status.occurrence_id,
                    "task_id": status.task_id,
                    "revision": status.revision,
                    "scheduled_for_ms": status.scheduled_for_ms,
                    "state": state_name(status.state),
                    "orchestration_run_id": status.orchestration_run_id,
                    "summary_code": status.summary_code,
                })
            );
        }
    }
    daemon.shutdown()?;
    Ok(())
}

fn now() -> Result<UtcInstant, std::time::SystemTimeError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let millis = i64::try_from(duration.as_millis()).unwrap_or(i64::MAX);
    Ok(UtcInstant::from_millis(millis))
}

const fn state_name(state: OccurrenceState) -> &'static str {
    match state {
        OccurrenceState::Queued => "queued",
        OccurrenceState::Dispatching => "dispatching",
        OccurrenceState::Dispatched => "dispatched",
        OccurrenceState::Succeeded => "succeeded",
        OccurrenceState::Failed => "failed",
        OccurrenceState::RecoveryRequired => "recovery_required",
        OccurrenceState::Skipped => "skipped",
    }
}
