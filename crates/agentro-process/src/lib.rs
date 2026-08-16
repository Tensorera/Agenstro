//! Bounded process execution with explicit ownership and native tree containment.
//!
//! Commands are always an executable plus an argument vector. The native
//! supervisor clears the child environment, drains both output pipes
//! concurrently, and owns the child tree until it has been terminated and
//! reaped. Windows uses a Job Object and Linux uses a process group through
//! the maintained `command-group` crate.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod cancellation;
mod spec;
mod supervisor;

pub use cancellation::{CancellationSource, CancellationToken};
pub use spec::{
    MAX_ARGUMENT_BYTES, MAX_ARGUMENTS, MAX_CAPTURE_BYTES, MAX_ENVIRONMENT_BYTES,
    MAX_ENVIRONMENT_VARIABLES, MAX_TOTAL_CAPTURE_BYTES, OutputBudget, ProcessSpec, ProcessTimeouts,
    ResourceLimits, SpecError,
};
pub use supervisor::{
    ContainmentKind, GracefulTermination, NativeProcessSupervisor, OutputStream,
    ProcessCapabilities, ProcessError, ProcessOutput, ProcessSupervisor, TerminationReason,
};
