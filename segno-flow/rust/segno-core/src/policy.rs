use std::{fmt, num::NonZeroU16};

const MAX_CRON_BYTES: usize = 200;
const MAX_TIMEZONE_BYTES: usize = 100;

/// Supported persisted cron syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CronDialect {
    /// Strict Unix five-field minute/hour/day/month/weekday syntax.
    UnixFiveField,
}

/// A bounded five-field cron expression.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CronExpression(Box<str>);

impl CronExpression {
    /// Validates the transport-independent cron envelope.
    ///
    /// Infrastructure performs full grammar parsing with the selected cron
    /// implementation before a task revision is imported.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyError::InvalidCron`] for non-five-field, control, or
    /// oversized input.
    pub fn parse(value: &str) -> Result<Self, PolicyError> {
        if value.is_empty()
            || value.len() > MAX_CRON_BYTES
            || value.chars().any(char::is_control)
            || value.split_ascii_whitespace().count() != 5
        {
            return Err(PolicyError::InvalidCron);
        }
        Ok(Self(value.into()))
    }

    /// Returns the original validated expression.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A bounded explicit IANA timezone name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IanaTimeZone(Box<str>);

impl IanaTimeZone {
    /// Validates the portable timezone name envelope.
    ///
    /// # Errors
    ///
    /// Rejects `local`, whitespace/control input, and oversized names. The
    /// infrastructure timezone database must also recognize the value.
    pub fn parse(value: &str) -> Result<Self, PolicyError> {
        if value.is_empty()
            || value.eq_ignore_ascii_case("local")
            || value.len() > MAX_TIMEZONE_BYTES
            || value.chars().any(|character| {
                character.is_whitespace()
                    || character.is_control()
                    || !(character.is_ascii_alphanumeric()
                        || matches!(character, '/' | '_' | '-' | '+'))
            })
        {
            return Err(PolicyError::InvalidTimeZone);
        }
        Ok(Self(value.into()))
    }

    /// Returns the IANA database key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Policy for a cron civil time absent during a spring-forward transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DstGapPolicy {
    /// Do not create an occurrence for the absent civil time.
    Skip,
    /// Move the occurrence to the first valid civil minute after the gap.
    NextValid,
}

/// Policy for an ambiguous civil time during a fall-back transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DstFoldPolicy {
    /// Select the earlier UTC instant.
    First,
    /// Select the later UTC instant.
    Second,
    /// Create occurrences for both distinct UTC instants.
    Both,
}

/// Explicit downtime/misfire behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MisfirePolicy {
    /// Admit only the newest due instant when it is within the grace window.
    Skip {
        /// Maximum accepted lateness in milliseconds.
        grace_ms: u64,
    },
    /// Admit only the newest due instant regardless of lateness.
    Coalesce,
    /// Admit at most the newest configured number of missed instants.
    BoundedCatchUp(NonZeroU16),
}

/// Explicit same-task overlap behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlapPolicy {
    /// Keep later occurrences queued while one run is active.
    Forbid,
    /// Retain at most one queued successor while one run is active.
    QueueOne,
    /// Permit a bounded number of active runs for the task.
    AllowWithLimit(NonZeroU16),
}

/// Explicit scheduler-level dispatch retry behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryPolicy {
    /// Never create another dispatch attempt after a known failure.
    None,
    /// Retry only a task declared idempotent, with bounded attempts and delay.
    BoundedIdempotent {
        /// Total dispatch attempts including the first.
        max_attempts: NonZeroU16,
        /// Delay before another dispatch attempt, in milliseconds.
        delay_ms: u64,
    },
}

/// Complete persisted schedule semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulePolicy {
    /// Cron syntax selector.
    pub dialect: CronDialect,
    /// Five-field expression.
    pub cron: CronExpression,
    /// IANA timezone database key.
    pub timezone: IanaTimeZone,
    /// Missing civil-time behavior.
    pub dst_gap: DstGapPolicy,
    /// Ambiguous civil-time behavior.
    pub dst_fold: DstFoldPolicy,
    /// Downtime behavior.
    pub misfire: MisfirePolicy,
    /// Same-task concurrency behavior.
    pub overlap: OverlapPolicy,
    /// Dispatch retry behavior.
    pub retry: RetryPolicy,
    /// Maximum deterministic jitter in milliseconds.
    pub jitter_ms: u64,
}

/// Invalid persisted schedule policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    /// Cron envelope is not strict five-field syntax.
    InvalidCron,
    /// Timezone is not an explicit portable IANA key envelope.
    InvalidTimeZone,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCron => "cron expression must contain five bounded fields",
            Self::InvalidTimeZone => "timezone must be an explicit bounded IANA name",
        })
    }
}

impl std::error::Error for PolicyError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_and_timezone_require_explicit_portable_values() {
        assert!(CronExpression::parse("0 8 * * *").is_ok());
        assert!(CronExpression::parse("0 0 8 * * *").is_err());
        assert!(IanaTimeZone::parse("America/New_York").is_ok());
        assert!(IanaTimeZone::parse("local").is_err());
    }
}
