use std::fmt;

use crate::{
    FencingToken, LeaseError, LeaseOwnerId, OccurrenceId, ScheduleRevision, Sha256Digest, TaskId,
};

const MAX_RUN_ID_BYTES: usize = 128;
const MAX_WORKFLOW_SPEC_BYTES: usize = 1024 * 1024;

/// Stable `agentrod` orchestration run reference.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OrchestrationRunId(Box<str>);

impl OrchestrationRunId {
    /// Parses a bounded printable run reference.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseError::InvalidIdentity`] for malformed input.
    pub fn parse(value: &str) -> Result<Self, LeaseError> {
        if value.is_empty()
            || value.len() > MAX_RUN_ID_BYTES
            || value
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(LeaseError::InvalidIdentity);
        }
        Ok(Self(value.into()))
    }

    /// Returns the protocol text value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Frozen Segno workflow compilation request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileWorkflowRequest {
    /// Owning task.
    pub task_id: TaskId,
    /// Immutable revision.
    pub revision: ScheduleRevision,
    /// Published package digest.
    pub package_digest: Sha256Digest,
    /// Canonical execution-spec digest.
    pub workflow_spec_digest: Sha256Digest,
    workflow_spec: Vec<u8>,
}

impl CompileWorkflowRequest {
    /// Creates a bounded compilation request.
    ///
    /// # Errors
    ///
    /// Returns [`PortError::InvalidRequest`] for an empty or oversized spec.
    pub fn new(
        task_id: TaskId,
        revision: ScheduleRevision,
        package_digest: Sha256Digest,
        workflow_spec_digest: Sha256Digest,
        workflow_spec: Vec<u8>,
    ) -> Result<Self, PortError> {
        if workflow_spec.is_empty() || workflow_spec.len() > MAX_WORKFLOW_SPEC_BYTES {
            return Err(PortError::InvalidRequest);
        }
        Ok(Self {
            task_id,
            revision,
            package_digest,
            workflow_spec_digest,
            workflow_spec,
        })
    }

    /// Returns canonical, versioned Clef execution-spec bytes.
    #[must_use]
    pub fn workflow_spec(&self) -> &[u8] {
        &self.workflow_spec
    }
}

/// Successful compilation result persisted before schedule enablement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompileWorkflowResponse {
    /// Frozen Clef plan digest.
    pub plan_digest: Sha256Digest,
}

/// Port that delegates workflow semantics to `agentrod`/Clef.
pub trait CompileWorkflowPort {
    /// Compiles one immutable versioned execution spec.
    ///
    /// Repeating the same spec digest must return the same plan digest.
    ///
    /// # Errors
    ///
    /// Returns a bounded transport or remote rejection.
    fn compile_workflow(
        &mut self,
        request: &CompileWorkflowRequest,
    ) -> Result<CompileWorkflowResponse, PortError>;
}

/// Idempotent dispatch request sent only after durable outbox creation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchRequest {
    /// Idempotency key and occurrence identity.
    pub occurrence_id: OccurrenceId,
    /// Frozen task revision.
    pub revision: ScheduleRevision,
    /// Frozen Clef plan digest.
    pub plan_digest: Sha256Digest,
    /// Current Segno lease owner.
    pub owner: LeaseOwnerId,
    /// Fence required to record the response.
    pub fencing_token: FencingToken,
}

/// Result of starting an idempotent workflow run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchStart {
    /// `agentrod` returned the stable orchestration reference.
    Accepted(OrchestrationRunId),
    /// Transport failed after acceptance may have occurred.
    OutcomeUnknown,
}

/// Result of querying `agentrod` by occurrence idempotency key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchLookup {
    /// The occurrence already maps to this stable run.
    Found(OrchestrationRunId),
    /// No run currently exists for the key; the same start request is safe.
    NotFound,
}

/// Port for idempotent `occurrence_id` to orchestration-run mapping.
pub trait AgentrodPort {
    /// Starts or returns the existing run for the occurrence key.
    ///
    /// # Errors
    ///
    /// Returns only a known pre-accept rejection. Ambiguous transport failure
    /// must be represented as [`DispatchStart::OutcomeUnknown`].
    fn start_workflow(&mut self, request: &DispatchRequest) -> Result<DispatchStart, PortError>;

    /// Queries the durable external mapping before retry after uncertainty.
    ///
    /// # Errors
    ///
    /// Returns a bounded transport/remote error.
    fn query_by_occurrence(
        &mut self,
        occurrence_id: &OccurrenceId,
    ) -> Result<DispatchLookup, PortError>;
}

/// Bounded external port failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortError {
    /// Request violated a local bounded contract.
    InvalidRequest,
    /// External service is unavailable before the operation was accepted.
    Unavailable,
    /// External service rejected the immutable plan or policy.
    Rejected(&'static str),
}

impl fmt::Display for PortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest => formatter.write_str("port request is invalid"),
            Self::Unavailable => formatter.write_str("external service is unavailable"),
            Self::Rejected(code) => write!(formatter, "external service rejected request: {code}"),
        }
    }
}

impl std::error::Error for PortError {}
