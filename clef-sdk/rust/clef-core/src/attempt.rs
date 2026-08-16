use std::{fmt, num::NonZeroU32};

use crate::TaskId;

/// One-based immutable task-attempt number.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AttemptNumber(NonZeroU32);

impl AttemptNumber {
    /// Creates a non-zero attempt number.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError::InvalidAttemptNumber`] for zero.
    pub const fn new(value: u32) -> Result<Self, TransitionError> {
        match NonZeroU32::new(value) {
            Some(value) => Ok(Self(value)),
            None => Err(TransitionError::InvalidAttemptNumber),
        }
    }

    /// Returns the one-based numeric value.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0.get()
    }
}

/// Closed lifecycle states for one task attempt.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AttemptState {
    /// The attempt exists but has not started.
    Planned,
    /// Policy requires an approval before backend work starts.
    WaitingForApproval,
    /// Backend work is in progress.
    Running,
    /// Deterministic checks are evaluating candidate outputs.
    Verifying,
    /// The publish gate is evaluating accepted outputs.
    Publishing,
    /// Outputs were published successfully.
    Succeeded,
    /// The attempt failed or approval was denied.
    Failed,
    /// Explicit cancellation reached the attempt.
    Cancelled,
    /// The owner disappeared before a clean terminal result.
    Interrupted,
}

impl AttemptState {
    /// Returns whether the state cannot transition further.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

/// Command accepted by the attempt state machine.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AttemptCommand {
    /// Starts the attempt, optionally stopping at the approval gate.
    Begin {
        /// Whether policy requires an approval first.
        approval_required: bool,
    },
    /// Accepts a pending approval and starts backend work.
    Approve,
    /// Denies a pending approval and fails the attempt.
    Deny,
    /// Moves backend output into deterministic verification.
    BeginVerification,
    /// Moves verified output into the publish gate.
    BeginPublishing,
    /// Commits the successful terminal state after publication.
    Complete,
    /// Records a typed failure at a non-terminal stage.
    Fail,
    /// Cancels a planned or active attempt.
    Cancel,
    /// Records loss of the active owner.
    Interrupt,
}

/// One task attempt whose state changes only through [`Attempt::apply`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attempt {
    task_id: TaskId,
    number: AttemptNumber,
    state: AttemptState,
}

impl Attempt {
    /// Creates a planned attempt.
    #[must_use]
    pub const fn new(task_id: TaskId, number: AttemptNumber) -> Self {
        Self {
            task_id,
            number,
            state: AttemptState::Planned,
        }
    }

    /// Applies one command without changing state on failure.
    ///
    /// Allowed transitions are:
    ///
    /// - `Planned -> WaitingForApproval | Running | Cancelled`
    /// - `WaitingForApproval -> Running | Failed | Cancelled`
    /// - `Running -> Verifying | Failed | Cancelled | Interrupted`
    /// - `Verifying -> Publishing | Failed | Cancelled | Interrupted`
    /// - `Publishing -> Succeeded | Failed | Cancelled | Interrupted`
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError::InvalidTransition`] for any other pair.
    pub fn apply(&mut self, command: AttemptCommand) -> Result<AttemptState, TransitionError> {
        let next = match (self.state, command) {
            (
                AttemptState::Planned,
                AttemptCommand::Begin {
                    approval_required: true,
                },
            ) => AttemptState::WaitingForApproval,
            (
                AttemptState::Planned,
                AttemptCommand::Begin {
                    approval_required: false,
                },
            )
            | (AttemptState::WaitingForApproval, AttemptCommand::Approve) => AttemptState::Running,
            (AttemptState::WaitingForApproval, AttemptCommand::Deny) => AttemptState::Failed,
            (AttemptState::Running, AttemptCommand::BeginVerification) => AttemptState::Verifying,
            (AttemptState::Verifying, AttemptCommand::BeginPublishing) => AttemptState::Publishing,
            (AttemptState::Publishing, AttemptCommand::Complete) => AttemptState::Succeeded,
            (
                AttemptState::Planned
                | AttemptState::WaitingForApproval
                | AttemptState::Running
                | AttemptState::Verifying
                | AttemptState::Publishing,
                AttemptCommand::Cancel,
            ) => AttemptState::Cancelled,
            (
                AttemptState::WaitingForApproval
                | AttemptState::Running
                | AttemptState::Verifying
                | AttemptState::Publishing,
                AttemptCommand::Fail,
            ) => AttemptState::Failed,
            (
                AttemptState::Running | AttemptState::Verifying | AttemptState::Publishing,
                AttemptCommand::Interrupt,
            ) => AttemptState::Interrupted,
            (state, command) => {
                return Err(TransitionError::InvalidTransition { state, command });
            }
        };
        self.state = next;
        Ok(next)
    }

    /// Returns the owning task.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the immutable one-based attempt number.
    #[must_use]
    pub const fn number(&self) -> AttemptNumber {
        self.number
    }

    /// Returns the current closed state.
    #[must_use]
    pub const fn state(&self) -> AttemptState {
        self.state
    }
}

/// An invalid attempt number or lifecycle command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionError {
    /// Attempt numbers are one-based.
    InvalidAttemptNumber,
    /// The command is not allowed from the current state.
    InvalidTransition {
        /// State that rejected the command.
        state: AttemptState,
        /// Rejected command.
        command: AttemptCommand,
    },
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAttemptNumber => formatter.write_str("attempt number must be non-zero"),
            Self::InvalidTransition { state, command } => {
                write!(
                    formatter,
                    "attempt command {command:?} is invalid from {state:?}"
                )
            }
        }
    }
}

impl std::error::Error for TransitionError {}

#[cfg(test)]
mod tests {
    use crate::TaskId;

    use super::{Attempt, AttemptCommand, AttemptNumber, AttemptState};

    #[test]
    fn successful_attempt_must_cross_verification_and_publish()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut attempt = Attempt::new(TaskId::parse("task")?, AttemptNumber::new(1)?);
        attempt.apply(AttemptCommand::Begin {
            approval_required: false,
        })?;
        assert!(attempt.apply(AttemptCommand::Complete).is_err());
        attempt.apply(AttemptCommand::BeginVerification)?;
        attempt.apply(AttemptCommand::BeginPublishing)?;
        attempt.apply(AttemptCommand::Complete)?;

        assert_eq!(attempt.state(), AttemptState::Succeeded);
        assert!(attempt.apply(AttemptCommand::Cancel).is_err());
        assert_eq!(attempt.state(), AttemptState::Succeeded);
        Ok(())
    }
}
