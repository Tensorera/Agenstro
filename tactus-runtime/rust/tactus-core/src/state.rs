use thiserror::Error;

/// Durable state of one execution attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunState {
    /// Durable intent exists but no worker has started.
    Pending,
    /// A supervised worker owns execution.
    Running,
    /// Cancellation was durably requested and is propagating to the worker.
    Cancelling,
    /// Startup reconciliation is examining an in-flight attempt.
    Recovering,
    /// Result checkpoint and terminal state committed together.
    Succeeded,
    /// Execution or checkpoint publication failed.
    Failed,
    /// Explicit cancellation completed.
    Cancelled,
    /// The prior worker cannot be resumed after a crash or protocol loss.
    Interrupted,
}

/// Commands accepted by the run state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunTransition {
    /// Start a supervised worker.
    Start,
    /// Request idempotent cancellation.
    RequestCancel,
    /// Enter daemon restart reconciliation.
    BeginRecovery,
    /// Commit success after checkpoint publication.
    Succeed,
    /// Commit a typed failure.
    Fail,
    /// Finish an accepted cancellation.
    Cancel,
    /// Record loss of a resumable worker.
    Interrupt,
}

/// Durable state of one cell execution, distinct from stable cell identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellState {
    /// The attempt is queued behind a durable run intent.
    Queued,
    /// The cell is executing in a supervised worker.
    Running,
    /// The cell is being reconciled after daemon restart.
    Recovering,
    /// The cell result and checkpoint committed.
    Succeeded,
    /// The cell execution failed.
    Failed,
    /// The cell was explicitly cancelled.
    Cancelled,
    /// The cell worker disappeared and Python memory was discarded.
    Interrupted,
}

/// Commands accepted by the cell-attempt state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CellTransition {
    /// Start execution.
    Start,
    /// Enter crash reconciliation.
    BeginRecovery,
    /// Commit success.
    Succeed,
    /// Commit failure.
    Fail,
    /// Commit cancellation.
    Cancel,
    /// Commit interruption.
    Interrupt,
}

/// Durable workspace transaction state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionState {
    /// Intent and fence exist; baseline publication is incomplete.
    Prepared,
    /// Baseline checkpoint is durable and execution may start.
    Active,
    /// Result checkpoint and run success committed together.
    Committed,
    /// No successful result was published; evidence remains available.
    Abandoned,
    /// Workspace consistency could not be proven.
    Conflict,
}

/// Commands accepted by the workspace transaction state machine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionTransition {
    /// Attach the durable baseline checkpoint.
    Activate,
    /// Commit a result checkpoint.
    Commit,
    /// Stop without publishing a result.
    Abandon,
    /// Preserve evidence and require explicit conflict resolution.
    Conflict,
}

/// An attempted state transition violated a closed state machine.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("invalid {machine} state transition from {state} using {command}")]
pub struct StateTransitionError {
    machine: &'static str,
    state: &'static str,
    command: &'static str,
}

impl RunState {
    /// Applies one run command without permitting a terminal state to reopen.
    ///
    /// # Errors
    ///
    /// Returns [`StateTransitionError`] when the command is invalid.
    pub fn transition(self, command: RunTransition) -> Result<Self, StateTransitionError> {
        let next = match (self, command) {
            (Self::Pending, RunTransition::Start) => Self::Running,
            (Self::Pending | Self::Running, RunTransition::RequestCancel) => Self::Cancelling,
            (Self::Pending | Self::Running | Self::Cancelling, RunTransition::BeginRecovery) => {
                Self::Recovering
            }
            (Self::Running, RunTransition::Succeed) => Self::Succeeded,
            (Self::Pending | Self::Running, RunTransition::Fail) => Self::Failed,
            (Self::Pending | Self::Running | Self::Cancelling, RunTransition::Cancel) => {
                Self::Cancelled
            }
            (
                Self::Pending | Self::Running | Self::Cancelling | Self::Recovering,
                RunTransition::Interrupt,
            ) => Self::Interrupted,
            (Self::Recovering, RunTransition::Fail) => Self::Failed,
            _ => return Err(transition_error("run", self.as_str(), run_command(command))),
        };
        Ok(next)
    }

    /// Reports whether no later state transition is legal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Recovering => "recovering",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "cancelling" => Some(Self::Cancelling),
            "recovering" => Some(Self::Recovering),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }
}

impl CellState {
    /// Applies one cell-attempt command.
    ///
    /// # Errors
    ///
    /// Returns [`StateTransitionError`] when the command is invalid.
    pub fn transition(self, command: CellTransition) -> Result<Self, StateTransitionError> {
        let next = match (self, command) {
            (Self::Queued, CellTransition::Start) => Self::Running,
            (Self::Queued | Self::Running, CellTransition::BeginRecovery) => Self::Recovering,
            (Self::Running, CellTransition::Succeed) => Self::Succeeded,
            (Self::Queued | Self::Running | Self::Recovering, CellTransition::Fail) => Self::Failed,
            (Self::Queued | Self::Running, CellTransition::Cancel) => Self::Cancelled,
            (Self::Queued | Self::Running | Self::Recovering, CellTransition::Interrupt) => {
                Self::Interrupted
            }
            _ => {
                return Err(transition_error(
                    "cell",
                    self.as_str(),
                    cell_command(command),
                ));
            }
        };
        Ok(next)
    }

    /// Reports whether the cell attempt is terminal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Recovering => "recovering",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "recovering" => Some(Self::Recovering),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }
}

impl TransactionState {
    /// Applies one workspace transaction command.
    ///
    /// # Errors
    ///
    /// Returns [`StateTransitionError`] when the command is invalid.
    pub fn transition(self, command: TransactionTransition) -> Result<Self, StateTransitionError> {
        let next = match (self, command) {
            (Self::Prepared, TransactionTransition::Activate) => Self::Active,
            (Self::Active, TransactionTransition::Commit) => Self::Committed,
            (Self::Prepared | Self::Active, TransactionTransition::Abandon) => Self::Abandoned,
            (Self::Prepared | Self::Active, TransactionTransition::Conflict) => Self::Conflict,
            _ => {
                return Err(transition_error(
                    "workspace transaction",
                    self.as_str(),
                    transaction_command(command),
                ));
            }
        };
        Ok(next)
    }

    /// Reports whether the transaction cannot transition again.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Committed | Self::Abandoned | Self::Conflict)
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Active => "active",
            Self::Committed => "committed",
            Self::Abandoned => "abandoned",
            Self::Conflict => "conflict",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "prepared" => Some(Self::Prepared),
            "active" => Some(Self::Active),
            "committed" => Some(Self::Committed),
            "abandoned" => Some(Self::Abandoned),
            "conflict" => Some(Self::Conflict),
            _ => None,
        }
    }
}

const fn transition_error(
    machine: &'static str,
    state: &'static str,
    command: &'static str,
) -> StateTransitionError {
    StateTransitionError {
        machine,
        state,
        command,
    }
}

const fn run_command(command: RunTransition) -> &'static str {
    match command {
        RunTransition::Start => "start",
        RunTransition::RequestCancel => "request_cancel",
        RunTransition::BeginRecovery => "begin_recovery",
        RunTransition::Succeed => "succeed",
        RunTransition::Fail => "fail",
        RunTransition::Cancel => "cancel",
        RunTransition::Interrupt => "interrupt",
    }
}

const fn cell_command(command: CellTransition) -> &'static str {
    match command {
        CellTransition::Start => "start",
        CellTransition::BeginRecovery => "begin_recovery",
        CellTransition::Succeed => "succeed",
        CellTransition::Fail => "fail",
        CellTransition::Cancel => "cancel",
        CellTransition::Interrupt => "interrupt",
    }
}

const fn transaction_command(command: TransactionTransition) -> &'static str {
    match command {
        TransactionTransition::Activate => "activate",
        TransactionTransition::Commit => "commit",
        TransactionTransition::Abandon => "abandon",
        TransactionTransition::Conflict => "conflict",
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn success_requires_a_running_attempt() -> Result<(), StateTransitionError> {
        let running = RunState::Pending.transition(RunTransition::Start)?;
        let succeeded = running.transition(RunTransition::Succeed)?;
        assert_eq!(succeeded, RunState::Succeeded);
        assert!(succeeded.transition(RunTransition::Start).is_err());
        Ok(())
    }

    proptest! {
        #[test]
        fn terminal_runs_never_reopen(command in 0_u8..7) {
            let command = match command {
                0 => RunTransition::Start,
                1 => RunTransition::RequestCancel,
                2 => RunTransition::BeginRecovery,
                3 => RunTransition::Succeed,
                4 => RunTransition::Fail,
                5 => RunTransition::Cancel,
                _ => RunTransition::Interrupt,
            };
            for terminal in [
                RunState::Succeeded,
                RunState::Failed,
                RunState::Cancelled,
                RunState::Interrupted,
            ] {
                prop_assert!(terminal.transition(command).is_err());
            }
        }
    }

    #[test]
    fn prepared_checkpoint_cannot_be_mislabeled_committed() {
        assert!(
            TransactionState::Prepared
                .transition(TransactionTransition::Commit)
                .is_err()
        );
    }
}
