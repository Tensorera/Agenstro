//! Minimal bounded `agentrod` application/API composition.
//!
//! This vertical slice owns deterministic workflow compilation, in-memory run
//! resources, normalized adapter sessions, sequenced watch events, cancellation,
//! and publication. Durable SQLite/outbox and authenticated Tonic transport are
//! intentionally later infrastructure layers; callers explicitly drive progress
//! with [`Agentrod::advance_run`] so this library creates no detached tasks.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod api;
mod service;

pub use api::{
    ApiValueError, CancelRunRequest, CompileWorkflowRequest, CompileWorkflowResponse,
    CoordinateError, RunEvent, RunEventBody, RunFailure, RunSnapshot, RunState, ServiceLimits,
    SessionCoordinates, StartRunRequest, TaskSnapshot, WatchRunPage, WatchRunRequest,
    derive_session_coordinates,
};
pub use service::{Agentrod, AgentrodError};
