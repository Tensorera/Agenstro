use std::{collections::BTreeMap, fmt};

use crate::{ExecutionPlan, TaskId};

/// Runtime scheduling state of one task in a compiled plan.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TaskScheduleState {
    /// Dependencies or concurrency admission are still pending.
    Pending,
    /// The scheduler has granted one bounded concurrency slot.
    Running,
    /// The task completed and published successfully.
    Succeeded,
    /// The task failed.
    Failed,
    /// The active task was cancelled.
    Cancelled,
    /// The task cannot run because admission stopped or a dependency failed.
    Skipped,
}

impl TaskScheduleState {
    /// Returns whether no later scheduling transition is allowed.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Skipped
        )
    }
}

/// Result supplied when an admitted task releases its concurrency slot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TaskOutcome {
    /// The task published successfully.
    Succeeded,
    /// The task failed.
    Failed,
    /// The task was cancelled.
    Cancelled,
}

/// Aggregate scheduler outcome derived from task states.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum WorkflowScheduleState {
    /// No task has been admitted yet.
    Pending,
    /// At least one task is running or may still be admitted.
    Running,
    /// Every task succeeded.
    Succeeded,
    /// At least one task failed or was dependency-skipped.
    Failed,
    /// Cancellation ended the workflow without a task failure.
    Cancelled,
}

impl WorkflowScheduleState {
    /// Returns whether the workflow has no schedulable work left.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

/// Deterministic ready-set scheduler with an explicit concurrency bound.
#[derive(Clone, Debug)]
pub struct ReadyScheduler {
    plan: ExecutionPlan,
    states: BTreeMap<TaskId, TaskScheduleState>,
    running: u16,
    has_failure: bool,
}

impl ReadyScheduler {
    /// Creates a scheduler owning an immutable copy of the compiled plan.
    #[must_use]
    pub fn new(plan: &ExecutionPlan) -> Self {
        let states = plan
            .tasks()
            .iter()
            .map(|task| (task.spec().id().clone(), TaskScheduleState::Pending))
            .collect();
        Self {
            plan: plan.clone(),
            states,
            running: 0,
            has_failure: false,
        }
    }

    /// Returns currently ready tasks in deterministic plan order without
    /// changing admission state.
    #[must_use]
    pub fn ready_set(&self) -> Vec<TaskId> {
        if self.has_failure && self.plan.policy().is_fail_fast() {
            return Vec::new();
        }
        self.plan
            .tasks()
            .iter()
            .filter_map(|task| {
                let task_id = task.spec().id();
                if self.states.get(task_id) != Some(&TaskScheduleState::Pending) {
                    return None;
                }
                let all_succeeded = self.plan.predecessors(task_id).is_some_and(|predecessors| {
                    predecessors.iter().all(|predecessor| {
                        self.states.get(predecessor) == Some(&TaskScheduleState::Succeeded)
                    })
                });
                all_succeeded.then(|| task_id.clone())
            })
            .collect()
    }

    /// Admits at most the remaining workflow concurrency permits.
    ///
    /// Returned task IDs are already marked [`TaskScheduleState::Running`].
    pub fn admit_ready(&mut self) -> Vec<TaskId> {
        self.propagate_blocked();
        let available = self
            .plan
            .policy()
            .max_concurrency()
            .saturating_sub(self.running) as usize;
        let admitted: Vec<TaskId> = self.ready_set().into_iter().take(available).collect();
        for task_id in &admitted {
            if let Some(state) = self.states.get_mut(task_id) {
                *state = TaskScheduleState::Running;
                self.running = self.running.saturating_add(1);
            }
        }
        admitted
    }

    /// Releases one running task's permit and records its terminal outcome.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerError`] for an unknown task or a task not currently
    /// owned by a running slot. State is unchanged on failure.
    pub fn finish(&mut self, task_id: &TaskId, outcome: TaskOutcome) -> Result<(), SchedulerError> {
        let Some(current) = self.states.get(task_id).copied() else {
            return Err(SchedulerError::UnknownTask);
        };
        if current != TaskScheduleState::Running {
            return Err(SchedulerError::TaskNotRunning { current });
        }
        let next = match outcome {
            TaskOutcome::Succeeded => TaskScheduleState::Succeeded,
            TaskOutcome::Failed => TaskScheduleState::Failed,
            TaskOutcome::Cancelled => TaskScheduleState::Cancelled,
        };
        if let Some(state) = self.states.get_mut(task_id) {
            *state = next;
        }
        self.running = self.running.saturating_sub(1);
        if outcome == TaskOutcome::Failed {
            self.has_failure = true;
            if self.plan.policy().is_fail_fast() {
                for state in self.states.values_mut() {
                    if *state == TaskScheduleState::Pending {
                        *state = TaskScheduleState::Skipped;
                    }
                }
            }
        }
        self.propagate_blocked();
        Ok(())
    }

    /// Cancels every active task and skips every task not yet admitted.
    ///
    /// Returns IDs that were running so their application-owned resources can
    /// be cancelled and closed exactly once.
    pub fn cancel_all(&mut self) -> Vec<TaskId> {
        let mut running = Vec::new();
        for task in self.plan.tasks() {
            let task_id = task.spec().id();
            if let Some(state) = self.states.get_mut(task_id) {
                match *state {
                    TaskScheduleState::Running => {
                        running.push(task_id.clone());
                        *state = TaskScheduleState::Cancelled;
                    }
                    TaskScheduleState::Pending => *state = TaskScheduleState::Skipped,
                    _ => {}
                }
            }
        }
        self.running = 0;
        running
    }

    /// Returns one task's current scheduler state.
    #[must_use]
    pub fn state(&self, task_id: &TaskId) -> Option<TaskScheduleState> {
        self.states.get(task_id).copied()
    }

    /// Returns all task states in stable task-ID order.
    #[must_use]
    pub const fn states(&self) -> &BTreeMap<TaskId, TaskScheduleState> {
        &self.states
    }

    /// Returns the number of currently owned concurrency slots.
    #[must_use]
    pub const fn running_count(&self) -> u16 {
        self.running
    }

    /// Derives the aggregate workflow scheduling state.
    #[must_use]
    pub fn workflow_state(&self) -> WorkflowScheduleState {
        if self
            .states
            .values()
            .all(|state| *state == TaskScheduleState::Pending)
        {
            return WorkflowScheduleState::Pending;
        }
        if self.states.values().any(|state| {
            matches!(
                state,
                TaskScheduleState::Pending | TaskScheduleState::Running
            )
        }) {
            return WorkflowScheduleState::Running;
        }
        if self
            .states
            .values()
            .all(|state| *state == TaskScheduleState::Succeeded)
        {
            return WorkflowScheduleState::Succeeded;
        }
        if self.states.values().any(|state| {
            matches!(
                state,
                TaskScheduleState::Failed | TaskScheduleState::Skipped
            )
        }) {
            return WorkflowScheduleState::Failed;
        }
        WorkflowScheduleState::Cancelled
    }

    fn propagate_blocked(&mut self) {
        loop {
            let blocked: Vec<TaskId> = self
                .plan
                .tasks()
                .iter()
                .filter_map(|task| {
                    let task_id = task.spec().id();
                    if self.states.get(task_id) != Some(&TaskScheduleState::Pending) {
                        return None;
                    }
                    let is_blocked = self.plan.predecessors(task_id).is_some_and(|predecessors| {
                        predecessors.iter().any(|predecessor| {
                            self.states.get(predecessor).is_some_and(|state| {
                                matches!(
                                    state,
                                    TaskScheduleState::Failed
                                        | TaskScheduleState::Cancelled
                                        | TaskScheduleState::Skipped
                                )
                            })
                        })
                    });
                    is_blocked.then(|| task_id.clone())
                })
                .collect();
            if blocked.is_empty() {
                break;
            }
            for task_id in blocked {
                if let Some(state) = self.states.get_mut(&task_id) {
                    *state = TaskScheduleState::Skipped;
                }
            }
        }
    }
}

/// A scheduler command targeted an unknown or non-running task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    /// The task ID does not belong to the compiled plan.
    UnknownTask,
    /// Only a currently running task owns a permit that can be released.
    TaskNotRunning {
        /// Current state that rejected completion.
        current: TaskScheduleState,
    },
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTask => formatter.write_str("scheduler task is unknown"),
            Self::TaskNotRunning { current } => {
                write!(formatter, "scheduler task is not running: {current:?}")
            }
        }
    }
}

impl std::error::Error for SchedulerError {}
