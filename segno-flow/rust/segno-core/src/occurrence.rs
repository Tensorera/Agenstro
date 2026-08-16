use std::{fmt, num::NonZeroU64};

use agentro_contracts::{CanonicalHasher, DigestError};

use crate::{MisfirePolicy, OrchestrationRunId};

const MAX_ID_BYTES: usize = 128;

/// Stable validated task identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TaskId(Box<str>);

impl TaskId {
    /// Parses a lower-case dash-separated task identifier.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseError::InvalidIdentity`] for malformed values.
    pub fn parse(value: &str) -> Result<Self, LeaseError> {
        let bytes = value.as_bytes();
        let valid = !bytes.is_empty()
            && bytes.len() <= 64
            && bytes.first().is_some_and(u8::is_ascii_lowercase)
            && bytes
                .last()
                .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes
                .iter()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
            && !value.contains("--");
        if !valid {
            return Err(LeaseError::InvalidIdentity);
        }
        Ok(Self(value.into()))
    }

    /// Returns the stable text value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Monotonic task schedule revision.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ScheduleRevision(NonZeroU64);

impl ScheduleRevision {
    /// Creates a non-zero revision.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseError::InvalidRevision`] for zero.
    pub fn new(value: u64) -> Result<Self, LeaseError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(LeaseError::InvalidRevision)
    }

    /// Returns the persisted integer.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0.get()
    }
}

/// UTC millisecond instant used as occurrence identity.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct UtcInstant(i64);

impl UtcInstant {
    /// Creates a UTC instant from Unix epoch milliseconds.
    #[must_use]
    pub const fn from_millis(value: i64) -> Self {
        Self(value)
    }

    /// Returns Unix epoch milliseconds.
    #[must_use]
    pub const fn as_millis(self) -> i64 {
        self.0
    }
}

/// Deterministic logical occurrence identifier.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OccurrenceId(Box<str>);

impl OccurrenceId {
    /// Derives identity from task, schedule revision, and UTC instant.
    ///
    /// # Errors
    ///
    /// Returns a canonical digest error only if an internal field contract is
    /// violated.
    pub fn derive(
        task_id: &TaskId,
        schedule_revision: ScheduleRevision,
        scheduled_for: UtcInstant,
    ) -> Result<Self, DigestError> {
        let revision = schedule_revision.value().to_be_bytes();
        let instant = scheduled_for.as_millis().to_be_bytes();
        let mut hasher = CanonicalHasher::new("segno.occurrence")?;
        hasher.write_field("revision", &revision)?;
        hasher.write_field("scheduled_for", &instant)?;
        hasher.write_field("task_id", task_id.as_str().as_bytes())?;
        Ok(Self(format!("occ-{}", hasher.finish()).into()))
    }

    /// Parses the canonical derived representation.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseError::InvalidIdentity`] for another representation.
    pub fn parse(value: &str) -> Result<Self, LeaseError> {
        let digest = value
            .strip_prefix("occ-")
            .ok_or(LeaseError::InvalidIdentity)?;
        agentro_contracts::Sha256Digest::parse(digest).map_err(|_| LeaseError::InvalidIdentity)?;
        Ok(Self(value.into()))
    }

    /// Returns the stable idempotency key sent to `agentrod`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Bounded lease owner identity.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LeaseOwnerId(Box<str>);

impl LeaseOwnerId {
    /// Parses an opaque printable owner identifier.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseError::InvalidIdentity`] for empty, control, whitespace,
    /// or oversized input.
    pub fn parse(value: &str) -> Result<Self, LeaseError> {
        if value.is_empty()
            || value.len() > MAX_ID_BYTES
            || value
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(LeaseError::InvalidIdentity);
        }
        Ok(Self(value.into()))
    }

    /// Returns the stable owner text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Strictly increasing lease fencing token.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FencingToken(NonZeroU64);

impl FencingToken {
    /// Creates a non-zero token.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseError::FenceExhausted`] for zero.
    pub fn new(value: u64) -> Result<Self, LeaseError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(LeaseError::FenceExhausted)
    }

    /// Returns the persisted integer.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0.get()
    }

    fn next(self) -> Result<Self, LeaseError> {
        self.value()
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(Self)
            .ok_or(LeaseError::FenceExhausted)
    }
}

/// Active occurrence claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lease {
    owner: LeaseOwnerId,
    fencing_token: FencingToken,
    expires_at: UtcInstant,
}

impl Lease {
    /// Returns the owner.
    #[must_use]
    pub const fn owner(&self) -> &LeaseOwnerId {
        &self.owner
    }

    /// Returns the fencing token required by later writes.
    #[must_use]
    pub const fn fencing_token(&self) -> FencingToken {
        self.fencing_token
    }

    /// Returns the wall-clock expiration instant.
    #[must_use]
    pub const fn expires_at(&self) -> UtcInstant {
        self.expires_at
    }
}

/// Durable occurrence state owned by Segno.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OccurrenceState {
    /// Persisted and waiting for dispatch admission.
    Queued,
    /// Claimed and represented by a durable outbox command.
    Dispatching,
    /// Accepted by `agentrod`; workflow ownership has transferred.
    Dispatched,
    /// Bounded workflow summary reports success.
    Succeeded,
    /// Bounded workflow summary reports failure.
    Failed,
    /// External outcome cannot yet be determined safely.
    RecoveryRequired,
    /// Policy intentionally suppressed dispatch.
    Skipped,
}

impl OccurrenceState {
    /// Returns whether the occurrence cannot accept another state transition.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Skipped)
    }
}

/// One occurrence with lease/fence and only an orchestration reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Occurrence {
    /// Stable identity.
    pub id: OccurrenceId,
    /// Owning task.
    pub task_id: TaskId,
    /// Frozen schedule revision.
    pub schedule_revision: ScheduleRevision,
    /// UTC scheduled instant.
    pub scheduled_for: UtcInstant,
    /// Current durable state.
    pub state: OccurrenceState,
    /// Current claim, when any.
    pub lease: Option<Lease>,
    /// External orchestration reference, never Tactus execution detail.
    pub orchestration_run_id: Option<OrchestrationRunId>,
}

impl Occurrence {
    /// Creates a queued occurrence with deterministic identity.
    ///
    /// # Errors
    ///
    /// Returns a canonical digest error only for an internal contract failure.
    pub fn new(
        task_id: TaskId,
        schedule_revision: ScheduleRevision,
        scheduled_for: UtcInstant,
    ) -> Result<Self, DigestError> {
        let id = OccurrenceId::derive(&task_id, schedule_revision, scheduled_for)?;
        Ok(Self {
            id,
            task_id,
            schedule_revision,
            scheduled_for,
            state: OccurrenceState::Queued,
            lease: None,
            orchestration_run_id: None,
        })
    }

    /// Claims queued/recoverable work and advances its fencing token.
    ///
    /// # Errors
    ///
    /// Rejects terminal/dispatched work, an unexpired other owner, invalid TTL,
    /// or token/time arithmetic overflow.
    pub fn claim(
        &mut self,
        owner: LeaseOwnerId,
        now: UtcInstant,
        ttl_ms: u64,
    ) -> Result<FencingToken, LeaseError> {
        if ttl_ms == 0 || ttl_ms > i64::MAX as u64 {
            return Err(LeaseError::InvalidTtl);
        }
        if self.state.is_terminal() || self.state == OccurrenceState::Dispatched {
            return Err(LeaseError::InvalidState);
        }
        if self
            .lease
            .as_ref()
            .is_some_and(|lease| lease.expires_at > now && lease.owner != owner)
        {
            return Err(LeaseError::LeaseConflict);
        }
        let token = match self.lease.as_ref().map(|lease| lease.fencing_token) {
            Some(current) => current.next()?,
            None => FencingToken::new(1)?,
        };
        let ttl = i64::try_from(ttl_ms).map_err(|_| LeaseError::InvalidTtl)?;
        let expires_at = now
            .as_millis()
            .checked_add(ttl)
            .map(UtcInstant::from_millis)
            .ok_or(LeaseError::InvalidTtl)?;
        self.lease = Some(Lease {
            owner,
            fencing_token: token,
            expires_at,
        });
        self.state = OccurrenceState::Dispatching;
        Ok(token)
    }

    /// Records a successful `agentrod` acceptance using the current fence.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseError::FenceRejected`] for a stale owner/token.
    pub fn record_dispatch(
        &mut self,
        owner: &LeaseOwnerId,
        token: FencingToken,
        run_id: OrchestrationRunId,
    ) -> Result<(), LeaseError> {
        let is_current = self
            .lease
            .as_ref()
            .is_some_and(|lease| lease.owner == *owner && lease.fencing_token == token);
        if !is_current || self.state != OccurrenceState::Dispatching {
            return Err(LeaseError::FenceRejected);
        }
        self.orchestration_run_id = Some(run_id);
        self.state = OccurrenceState::Dispatched;
        Ok(())
    }
}

/// Lease, fence, identity, or occurrence transition failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseError {
    /// Persisted/public identity is malformed.
    InvalidIdentity,
    /// Schedule revisions must be non-zero.
    InvalidRevision,
    /// TTL is zero, excessive, or overflows its instant.
    InvalidTtl,
    /// Another unexpired owner holds the occurrence.
    LeaseConflict,
    /// The fencing sequence cannot advance.
    FenceExhausted,
    /// An old owner/token attempted to commit.
    FenceRejected,
    /// Occurrence state does not permit the command.
    InvalidState,
    /// Misfire output bound is invalid or exceeded.
    MisfireLimit,
}

impl fmt::Display for LeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentity => "identity is malformed",
            Self::InvalidRevision => "schedule revision must be non-zero",
            Self::InvalidTtl => "lease TTL is invalid",
            Self::LeaseConflict => "occurrence lease is held by another owner",
            Self::FenceExhausted => "fencing sequence is exhausted",
            Self::FenceRejected => "stale fencing token was rejected",
            Self::InvalidState => "occurrence state does not permit the command",
            Self::MisfireLimit => "misfire output limit is invalid or exceeded",
        })
    }
}

impl std::error::Error for LeaseError {}

/// Selects a bounded deterministic subset of sorted due instants.
///
/// # Errors
///
/// Rejects a zero output bound or a selected policy limit above the caller's
/// bound.
pub fn select_misfires(
    due: &[UtcInstant],
    now: UtcInstant,
    policy: MisfirePolicy,
    max_output: usize,
) -> Result<Vec<UtcInstant>, LeaseError> {
    if max_output == 0 {
        return Err(LeaseError::MisfireLimit);
    }
    let latest_index = due.iter().rposition(|instant| *instant <= now);
    let Some(latest_index) = latest_index else {
        return Ok(Vec::new());
    };
    let latest = due[latest_index];
    match policy {
        MisfirePolicy::Skip { grace_ms } => {
            let lateness = now.as_millis().saturating_sub(latest.as_millis());
            if u64::try_from(lateness).unwrap_or(u64::MAX) <= grace_ms {
                Ok(vec![latest])
            } else {
                Ok(Vec::new())
            }
        }
        MisfirePolicy::Coalesce => Ok(vec![latest]),
        MisfirePolicy::BoundedCatchUp(limit) => {
            let selected = usize::from(limit.get());
            if selected > max_output {
                return Err(LeaseError::MisfireLimit);
            }
            let start = latest_index.saturating_add(1).saturating_sub(selected);
            Ok(due[start..=latest_index].to_vec())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;

    fn occurrence() -> Result<Occurrence, Box<dyn std::error::Error>> {
        Ok(Occurrence::new(
            TaskId::parse("daily-report")?,
            ScheduleRevision::new(1)?,
            UtcInstant::from_millis(1_000),
        )?)
    }

    #[test]
    fn expired_owner_cannot_commit_after_new_fence() -> Result<(), Box<dyn std::error::Error>> {
        let old = LeaseOwnerId::parse("owner-a")?;
        let new = LeaseOwnerId::parse("owner-b")?;
        let mut value = occurrence()?;
        let stale = value.claim(old.clone(), UtcInstant::from_millis(1_000), 10)?;
        let current = value.claim(new.clone(), UtcInstant::from_millis(1_011), 10)?;

        assert!(current > stale);
        assert_eq!(
            value.record_dispatch(&old, stale, OrchestrationRunId::parse("run-stale")?),
            Err(LeaseError::FenceRejected)
        );
        value.record_dispatch(&new, current, OrchestrationRunId::parse("run-current")?)?;
        assert_eq!(value.state, OccurrenceState::Dispatched);
        Ok(())
    }

    #[test]
    fn misfire_selection_is_bounded_and_deterministic() -> Result<(), LeaseError> {
        let due = [1, 2, 3, 4].map(|value| UtcInstant::from_millis(value * 1_000));
        assert_eq!(
            select_misfires(
                &due,
                UtcInstant::from_millis(5_000),
                MisfirePolicy::Coalesce,
                4,
            )?,
            vec![UtcInstant::from_millis(4_000)]
        );
        assert_eq!(
            select_misfires(
                &due,
                UtcInstant::from_millis(5_000),
                MisfirePolicy::BoundedCatchUp(NonZeroU16::new(2).ok_or(LeaseError::MisfireLimit)?),
                2,
            )?,
            vec![
                UtcInstant::from_millis(3_000),
                UtcInstant::from_millis(4_000)
            ]
        );
        Ok(())
    }
}
