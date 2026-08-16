use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::{
    AgentBackend, AgentContractError, AgentError, AgentErrorCode, AgentEvent, AgentInput,
    AgentSession, CancelReason, CancelResult, OpenSessionRequest, ProbeReport, ProbeRequest,
};

const MAX_FAKE_SESSIONS: usize = 64;
const MAX_FAKE_EVENTS_PER_SESSION: usize = 4_096;

#[derive(Debug, Default)]
struct MetricCounters {
    opened_sessions: AtomicUsize,
    sent_turns: AtomicUsize,
    cancelled_sessions: AtomicUsize,
    closed_sessions: AtomicUsize,
}

/// Thread-safe observable counters used by fake adapter conformance tests.
#[derive(Clone, Debug, Default)]
pub struct FakeBackendMetrics {
    inner: Arc<MetricCounters>,
}

impl FakeBackendMetrics {
    /// Returns successfully opened sessions.
    #[must_use]
    pub fn opened_sessions(&self) -> usize {
        self.inner.opened_sessions.load(Ordering::SeqCst)
    }

    /// Returns accepted turn inputs.
    #[must_use]
    pub fn sent_turns(&self) -> usize {
        self.inner.sent_turns.load(Ordering::SeqCst)
    }

    /// Returns sessions receiving at least one cancellation call.
    #[must_use]
    pub fn cancelled_sessions(&self) -> usize {
        self.inner.cancelled_sessions.load(Ordering::SeqCst)
    }

    /// Returns explicitly closed session owners.
    #[must_use]
    pub fn closed_sessions(&self) -> usize {
        self.inner.closed_sessions.load(Ordering::SeqCst)
    }
}

/// Deterministic in-process fake for the adapter-host contract suite.
///
/// The fake never launches a provider, reads user configuration, accesses the
/// network, or synthesizes capabilities from a provider name.
#[derive(Debug)]
pub struct FakeBackend {
    report: ProbeReport,
    scripts: Mutex<VecDeque<VecDeque<AgentEvent>>>,
    metrics: FakeBackendMetrics,
}

impl FakeBackend {
    /// Creates a fake with one bounded event script per future session.
    ///
    /// # Errors
    ///
    /// Rejects more than 64 sessions or more than 4,096 events in one session.
    pub fn new(
        report: ProbeReport,
        scripts: Vec<Vec<AgentEvent>>,
    ) -> Result<Self, AgentContractError> {
        if scripts.len() > MAX_FAKE_SESSIONS
            || scripts
                .iter()
                .any(|events| events.len() > MAX_FAKE_EVENTS_PER_SESSION)
        {
            return Err(AgentContractError::LimitExceeded);
        }
        Ok(Self {
            report,
            scripts: Mutex::new(
                scripts
                    .into_iter()
                    .map(VecDeque::from)
                    .collect::<VecDeque<_>>(),
            ),
            metrics: FakeBackendMetrics::default(),
        })
    }

    /// Returns observable conformance counters sharing this fake's state.
    #[must_use]
    pub fn metrics(&self) -> FakeBackendMetrics {
        self.metrics.clone()
    }
}

impl AgentBackend for FakeBackend {
    fn probe(&self, _request: ProbeRequest) -> Result<ProbeReport, AgentError> {
        Ok(self.report.clone())
    }

    fn open_session(
        &self,
        request: OpenSessionRequest,
    ) -> Result<Box<dyn AgentSession>, AgentError> {
        if !request
            .capabilities()
            .missing_required(self.report.capabilities())
            .is_empty()
        {
            return Err(AgentError::new(AgentErrorCode::CapabilityMissing));
        }
        if request.effort().is_some() {
            let effort = crate::AgentCapability::ReasoningEffort
                .capability()
                .map_err(|_| AgentError::new(AgentErrorCode::Internal))?;
            if !self.report.capabilities().contains(effort.name()) {
                return Err(AgentError::new(AgentErrorCode::CapabilityMissing));
            }
        }
        let mut scripts = self
            .scripts
            .lock()
            .map_err(|_| AgentError::new(AgentErrorCode::Internal))?;
        let events = scripts
            .pop_front()
            .ok_or_else(|| AgentError::new(AgentErrorCode::Internal))?;
        self.metrics
            .inner
            .opened_sessions
            .fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeSession {
            events,
            metrics: self.metrics.clone(),
            has_sent: false,
            is_cancelled: false,
            saw_terminal: false,
        }))
    }
}

#[derive(Debug)]
struct FakeSession {
    events: VecDeque<AgentEvent>,
    metrics: FakeBackendMetrics,
    has_sent: bool,
    is_cancelled: bool,
    saw_terminal: bool,
}

impl AgentSession for FakeSession {
    fn send(&mut self, _input: AgentInput) -> Result<(), AgentError> {
        if self.is_cancelled {
            return Err(AgentError::new(AgentErrorCode::Cancelled));
        }
        if self.has_sent {
            return Err(AgentError::new(AgentErrorCode::ProtocolViolation));
        }
        self.has_sent = true;
        self.metrics.inner.sent_turns.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn poll_event(&mut self) -> Result<Option<AgentEvent>, AgentError> {
        if !self.has_sent {
            return Err(AgentError::new(AgentErrorCode::ProtocolViolation));
        }
        if self.is_cancelled {
            return Ok(None);
        }
        let event = self.events.pop_front();
        if event
            .as_ref()
            .is_some_and(|candidate| candidate.body().is_terminal())
        {
            self.saw_terminal = true;
        }
        Ok(event)
    }

    fn cancel(&mut self, _reason: CancelReason) -> Result<CancelResult, AgentError> {
        if self.saw_terminal {
            return Ok(CancelResult::AlreadyTerminal);
        }
        if !self.is_cancelled {
            self.is_cancelled = true;
            self.events.clear();
            self.metrics
                .inner
                .cancelled_sessions
                .fetch_add(1, Ordering::SeqCst);
        }
        Ok(CancelResult::Acknowledged)
    }

    fn close(self: Box<Self>) -> Result<(), AgentError> {
        self.metrics
            .inner
            .closed_sessions
            .fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use agentro_contracts::{CapabilitySet, UtcTimestamp};

    use crate::{
        AgentBackend, AgentCapability, AgentEvent, AgentEventBody, AgentInput, AgentSessionId,
        AgentTurnId, CapabilityRequest, OpenSessionRequest, ProbeLevel, ProbeReport, ProbeRequest,
        ProtocolVersion, ProviderName, WorkspaceId,
    };

    use super::FakeBackend;

    #[test]
    fn fake_obeys_probe_send_stream_cancel_and_close_contracts()
    -> Result<(), Box<dyn std::error::Error>> {
        let capabilities = CapabilitySet::from_capabilities([
            AgentCapability::Streaming.capability()?,
            AgentCapability::ReasoningEffort.capability()?,
        ])?;
        let provider = ProviderName::parse("fake")?;
        let session_id = AgentSessionId::parse("session-one")?;
        let turn_id = AgentTurnId::parse("turn-one")?;
        let event = AgentEvent::new(
            ProtocolVersion::V1,
            session_id.clone(),
            turn_id.clone(),
            1,
            UtcTimestamp::new(1_700_000_000, 0)?,
            provider.clone(),
            AgentEventBody::TurnCompleted {
                artifacts: Vec::new(),
            },
        )?;
        let report = ProbeReport::new(provider, "1.0.0", ProtocolVersion::V1, capabilities, true)?;
        let backend = FakeBackend::new(report, vec![vec![event]])?;
        let metrics = backend.metrics();

        assert_eq!(
            backend
                .probe(ProbeRequest::new(ProbeLevel::Protocol))?
                .protocol(),
            ProtocolVersion::V1
        );
        let mut session = backend.open_session(OpenSessionRequest::new(
            session_id,
            WorkspaceId::parse("workspace-one")?,
            CapabilityRequest::default(),
            None,
            8,
        )?)?;
        session.send(AgentInput::new(turn_id, "perform the task")?)?;
        assert!(session.poll_event()?.is_some());
        assert!(session.poll_event()?.is_none());
        session.close()?;

        assert_eq!(metrics.opened_sessions(), 1);
        assert_eq!(metrics.sent_turns(), 1);
        assert_eq!(metrics.closed_sessions(), 1);
        Ok(())
    }
}
