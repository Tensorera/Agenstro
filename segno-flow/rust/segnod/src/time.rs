use std::str::FromStr;

use chrono::{DateTime, Datelike, LocalResult, NaiveDateTime, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use cron::Schedule;
use segno_core::{DstFoldPolicy, DstGapPolicy, SchedulePolicy, UtcInstant};
use thiserror::Error;

const MAX_SEARCH_MINUTES: usize = 5_000_000;
const MAX_GAP_MINUTES: usize = 180;

/// Parsed strict five-field cron and fixed IANA timezone engine.
pub struct CronEngine {
    timezone: Tz,
    pattern: CronPattern,
    gap: DstGapPolicy,
    fold: DstFoldPolicy,
}

impl CronEngine {
    /// Parses the cron grammar and IANA timezone from a complete schedule.
    ///
    /// # Errors
    ///
    /// Rejects unsupported grammar/ranges or an unknown IANA timezone.
    pub fn new(policy: &SchedulePolicy) -> Result<Self, TimeError> {
        let timezone = Tz::from_str(policy.timezone.as_str()).map_err(|_| TimeError::TimeZone)?;
        let validation = format!("0 {}", policy.cron.as_str());
        Schedule::from_str(&validation).map_err(|_| TimeError::Cron)?;
        let pattern = CronPattern::parse(policy.cron.as_str())?;
        Ok(Self {
            timezone,
            pattern,
            gap: policy.dst_gap,
            fold: policy.dst_fold,
        })
    }

    /// Returns the next one or two UTC identities strictly after `after`.
    ///
    /// Fold policy `both` may return two instants for one civil minute. Search
    /// work has a fixed hard minute bound.
    ///
    /// # Errors
    ///
    /// Returns a timestamp or bounded-search failure.
    pub fn next_after(&self, after: UtcInstant) -> Result<Vec<UtcInstant>, TimeError> {
        let after_utc = DateTime::<Utc>::from_timestamp_millis(after.as_millis())
            .ok_or(TimeError::Timestamp)?;
        let local = after_utc.with_timezone(&self.timezone);
        let mut candidate = local
            .date_naive()
            .and_hms_opt(local.hour(), local.minute(), 0)
            .ok_or(TimeError::Timestamp)?;
        for _ in 0..MAX_SEARCH_MINUTES {
            if self.pattern.matches(candidate) {
                let resolved = resolve_local_tz(self.timezone, candidate, self.gap, self.fold)?;
                let future: Vec<UtcInstant> = resolved
                    .into_iter()
                    .filter(|instant| *instant > after)
                    .collect();
                if !future.is_empty() {
                    return Ok(future);
                }
            }
            candidate = candidate
                .checked_add_signed(chrono::TimeDelta::minutes(1))
                .ok_or(TimeError::SearchExhausted)?;
        }
        Err(TimeError::SearchExhausted)
    }
}

/// Resolves one civil time according to explicit gap/fold policy.
///
/// # Errors
///
/// Rejects an unknown timezone, malformed timestamp, or a gap larger than the
/// fixed search bound.
pub fn resolve_local(
    timezone: &str,
    civil: NaiveDateTime,
    gap: DstGapPolicy,
    fold: DstFoldPolicy,
) -> Result<Vec<UtcInstant>, TimeError> {
    let timezone = Tz::from_str(timezone).map_err(|_| TimeError::TimeZone)?;
    resolve_local_tz(timezone, civil, gap, fold)
}

fn resolve_local_tz(
    timezone: Tz,
    mut civil: NaiveDateTime,
    gap: DstGapPolicy,
    fold: DstFoldPolicy,
) -> Result<Vec<UtcInstant>, TimeError> {
    for attempt in 0..=MAX_GAP_MINUTES {
        match timezone.from_local_datetime(&civil) {
            LocalResult::Single(value) => return Ok(vec![to_instant(value)]),
            LocalResult::Ambiguous(first, second) => {
                let mut values = [to_instant(first), to_instant(second)];
                values.sort();
                return Ok(match fold {
                    DstFoldPolicy::First => vec![values[0]],
                    DstFoldPolicy::Second => vec![values[1]],
                    DstFoldPolicy::Both => values.to_vec(),
                });
            }
            LocalResult::None if gap == DstGapPolicy::Skip => return Ok(Vec::new()),
            LocalResult::None if attempt < MAX_GAP_MINUTES => {
                civil = civil
                    .checked_add_signed(chrono::TimeDelta::minutes(1))
                    .ok_or(TimeError::Timestamp)?;
            }
            LocalResult::None => return Err(TimeError::SearchExhausted),
        }
    }
    Err(TimeError::SearchExhausted)
}

fn to_instant(value: DateTime<Tz>) -> UtcInstant {
    UtcInstant::from_millis(value.with_timezone(&Utc).timestamp_millis())
}

#[derive(Clone)]
struct Field {
    minimum: u32,
    allowed: Vec<bool>,
    is_wildcard: bool,
}

impl Field {
    fn parse(
        value: &str,
        minimum: u32,
        maximum: u32,
        sunday_alias: bool,
    ) -> Result<Self, TimeError> {
        let width = usize::try_from(maximum - minimum + 1).map_err(|_| TimeError::Cron)?;
        let mut allowed = vec![false; width];
        let is_wildcard = value == "*";
        for item in value.split(',') {
            if item.is_empty() {
                return Err(TimeError::Cron);
            }
            let stepped = item.contains('/');
            let (range, step) = item.split_once('/').map_or((item, 1), |(range, step)| {
                (range, step.parse::<u32>().unwrap_or(0))
            });
            if step == 0 {
                return Err(TimeError::Cron);
            }
            let (start, end) = if range == "*" {
                (minimum, maximum)
            } else if let Some((start, end)) = range.split_once('-') {
                (
                    parse_field_number(start, minimum, maximum, sunday_alias)?,
                    parse_field_number(end, minimum, maximum, sunday_alias)?,
                )
            } else {
                let number = parse_field_number(range, minimum, maximum, sunday_alias)?;
                (number, if stepped { maximum } else { number })
            };
            if start > end {
                return Err(TimeError::Cron);
            }
            let mut current = start;
            while current <= end {
                let index = usize::try_from(current - minimum).map_err(|_| TimeError::Cron)?;
                allowed[index] = true;
                let Some(next) = current.checked_add(step) else {
                    break;
                };
                current = next;
            }
        }
        if !allowed.iter().any(|value| *value) {
            return Err(TimeError::Cron);
        }
        Ok(Self {
            minimum,
            allowed,
            is_wildcard,
        })
    }

    fn contains(&self, value: u32) -> bool {
        value
            .checked_sub(self.minimum)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.allowed.get(index))
            .copied()
            .unwrap_or(false)
    }
}

fn parse_field_number(
    value: &str,
    minimum: u32,
    maximum: u32,
    sunday_alias: bool,
) -> Result<u32, TimeError> {
    let number = value.parse::<u32>().map_err(|_| TimeError::Cron)?;
    if sunday_alias && number == 7 {
        return Ok(0);
    }
    if number < minimum || number > maximum {
        Err(TimeError::Cron)
    } else {
        Ok(number)
    }
}

struct CronPattern {
    minute: Field,
    hour: Field,
    day: Field,
    month: Field,
    weekday: Field,
}

impl CronPattern {
    fn parse(value: &str) -> Result<Self, TimeError> {
        let fields: Vec<&str> = value.split_ascii_whitespace().collect();
        if fields.len() != 5 {
            return Err(TimeError::Cron);
        }
        Ok(Self {
            minute: Field::parse(fields[0], 0, 59, false)?,
            hour: Field::parse(fields[1], 0, 23, false)?,
            day: Field::parse(fields[2], 1, 31, false)?,
            month: Field::parse(fields[3], 1, 12, false)?,
            weekday: Field::parse(fields[4], 0, 6, true)?,
        })
    }

    fn matches(&self, value: NaiveDateTime) -> bool {
        let day_matches = self.day.contains(value.day());
        let weekday = match value.weekday() {
            Weekday::Sun => 0,
            Weekday::Mon => 1,
            Weekday::Tue => 2,
            Weekday::Wed => 3,
            Weekday::Thu => 4,
            Weekday::Fri => 5,
            Weekday::Sat => 6,
        };
        let weekday_matches = self.weekday.contains(weekday);
        let calendar_day_matches = if self.day.is_wildcard || self.weekday.is_wildcard {
            day_matches && weekday_matches
        } else {
            day_matches || weekday_matches
        };
        self.minute.contains(value.minute())
            && self.hour.contains(value.hour())
            && self.month.contains(value.month())
            && calendar_day_matches
    }
}

/// Cron, timezone, timestamp, or bounded-search failure.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum TimeError {
    /// Cron syntax/range is invalid.
    #[error("cron expression is invalid")]
    Cron,
    /// IANA timezone is unknown to the fixed dependency version.
    #[error("IANA timezone is unknown")]
    TimeZone,
    /// UTC/civil timestamp is not representable.
    #[error("timestamp is outside the supported range")]
    Timestamp,
    /// No result was found within the fixed search budget.
    #[error("time search budget was exhausted")]
    SearchExhausted,
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    #[test]
    fn new_york_gap_and_fold_follow_explicit_policy() -> Result<(), Box<dyn std::error::Error>> {
        let gap = NaiveDate::from_ymd_opt(2026, 3, 8)
            .and_then(|date| date.and_hms_opt(2, 30, 0))
            .ok_or(TimeError::Timestamp)?;
        assert!(
            resolve_local(
                "America/New_York",
                gap,
                DstGapPolicy::Skip,
                DstFoldPolicy::First
            )?
            .is_empty()
        );
        let moved = resolve_local(
            "America/New_York",
            gap,
            DstGapPolicy::NextValid,
            DstFoldPolicy::First,
        )?;
        assert_eq!(moved[0].as_millis(), 1_772_953_200_000);

        let fold = NaiveDate::from_ymd_opt(2026, 11, 1)
            .and_then(|date| date.and_hms_opt(1, 30, 0))
            .ok_or(TimeError::Timestamp)?;
        let both = resolve_local(
            "America/New_York",
            fold,
            DstGapPolicy::Skip,
            DstFoldPolicy::Both,
        )?;
        assert_eq!(both.len(), 2);
        assert_eq!(both[1].as_millis() - both[0].as_millis(), 3_600_000);
        Ok(())
    }
}
