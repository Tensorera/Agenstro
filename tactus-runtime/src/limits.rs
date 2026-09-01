//! Validated workspace-owned resource limits.
//!
//! The native provider stream, the plugin protocol, and the workflow process
//! have deliberately separate budgets.  In particular, a large native stdout
//! allowance is a cumulative drain limit; it is never an instruction to retain
//! that many bytes in memory.

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};

/// Largest supported configured timeout: seven days.
pub const MAX_TIMEOUT_SECONDS: u64 = 7 * 24 * 60 * 60;
/// Largest supported terminal/plugin frame.
pub const MAX_FRAME_BYTES_CEILING: usize = 32 * 1024 * 1024;
/// Largest supported retained provider result.
pub const MAX_PROVIDER_RESULT_BYTES_CEILING: usize = 5 * 1024 * 1024;
/// Largest supported cumulative native stdout drain. On a 32-bit target the
/// address-space-sized ceiling is used so the configuration remains
/// representable without weakening 64-bit builds.
pub const MAX_NATIVE_STDOUT_BYTES_CEILING: usize = if usize::BITS >= 64 {
    4_294_967_296_u64 as usize
} else {
    usize::MAX
};
/// Largest possible resident native-output queue configured by one workspace.
pub const MAX_NATIVE_QUEUE_BYTES: usize = 256 * 1024 * 1024;

/// Resource policy serialized below `[limits]` in `.tactus/tactus.toml` and
/// forwarded to Clef in `runtime.json`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeLimits {
    /// Maximum provider processes admitted concurrently by one Clef runtime.
    pub max_concurrent_provider_calls: usize,
    /// Default timeout for one check compiler process.
    pub check_timeout_seconds: u64,
    /// Default timeout for one complete workflow entry process.
    pub script_timeout_seconds: u64,
    /// Default timeout for a non-provider plugin call.
    pub plugin_timeout_seconds: u64,
    /// Deadline applied by the Tactus provider dispatcher.
    pub provider_timeout_seconds: u64,
    /// Clef's outer provider deadline, including dispatcher cleanup headroom.
    pub provider_outer_timeout_seconds: u64,
    /// Maximum encoded plugin request.
    pub max_request_bytes: usize,
    /// Maximum one-line plugin-v1 frame.
    pub max_frame_bytes: usize,
    /// Maximum cumulative plugin-v1 stdout drained for one invocation.
    pub max_stdout_bytes: usize,
    /// Maximum observational event frames delivered for one invocation.
    pub max_event_frames: u64,
    /// Maximum retained plugin stderr.
    pub max_stderr_bytes: usize,
    /// Number of decoded plugin frames waiting at each bounded handoff.
    pub event_queue_bound: usize,
    /// Maximum one-line native agent event.
    pub native_max_line_bytes: usize,
    /// Maximum cumulative native agent stdout drained for one invocation.
    pub native_max_stdout_bytes: usize,
    /// Maximum native agent terminal text retained in memory.
    pub native_max_result_bytes: usize,
    /// Maximum retained native agent stderr.
    pub native_max_stderr_bytes: usize,
    /// Number of native stdout lines waiting between reader and parser.
    pub native_output_queue_bound: usize,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_concurrent_provider_calls: 4,
            check_timeout_seconds: 30 * 60,
            // One provider call may use 3h45m and Clef may spend another 15m
            // proving cleanup.  The entry process owns a final 15m margin.
            script_timeout_seconds: 4 * 60 * 60 + 15 * 60,
            plugin_timeout_seconds: 60 * 60,
            provider_timeout_seconds: 3 * 60 * 60 + 45 * 60,
            provider_outer_timeout_seconds: 4 * 60 * 60,
            max_request_bytes: 1024 * 1024,
            max_frame_bytes: 32 * 1024 * 1024,
            max_stdout_bytes: 64 * 1024 * 1024,
            max_event_frames: 10_000,
            max_stderr_bytes: 1024 * 1024,
            event_queue_bound: 4,
            native_max_line_bytes: 8 * 1024 * 1024,
            native_max_stdout_bytes: 1024 * 1024 * 1024,
            native_max_result_bytes: 4 * 1024 * 1024,
            native_max_stderr_bytes: 1024 * 1024,
            native_output_queue_bound: 8,
        }
    }
}

impl RuntimeLimits {
    /// Validate bounds and the nested-deadline ordering before any process is
    /// started.  The error is safe to show as configuration diagnostics.
    pub fn validate(&self) -> Result<(), String> {
        require_range(
            "max_concurrent_provider_calls",
            self.max_concurrent_provider_calls,
            1,
            32,
        )?;
        for (name, seconds) in [
            ("check_timeout_seconds", self.check_timeout_seconds),
            ("script_timeout_seconds", self.script_timeout_seconds),
            ("plugin_timeout_seconds", self.plugin_timeout_seconds),
            ("provider_timeout_seconds", self.provider_timeout_seconds),
            (
                "provider_outer_timeout_seconds",
                self.provider_outer_timeout_seconds,
            ),
        ] {
            require_range(name, seconds, 1, MAX_TIMEOUT_SECONDS)?;
        }
        if self.provider_timeout_seconds < 61 {
            return Err(
                "provider_timeout_seconds must be at least 61 seconds so the native provider can be cleaned up"
                    .to_owned(),
            );
        }
        if self.provider_outer_timeout_seconds < self.provider_timeout_seconds + 60 {
            return Err(
                "provider_outer_timeout_seconds must leave at least 60 seconds after provider_timeout_seconds"
                    .to_owned(),
            );
        }
        if self.script_timeout_seconds < self.provider_outer_timeout_seconds + 60 {
            return Err(
                "script_timeout_seconds must leave at least 60 seconds after provider_outer_timeout_seconds"
                    .to_owned(),
            );
        }
        if self.script_timeout_seconds < self.plugin_timeout_seconds {
            return Err(
                "script_timeout_seconds must not be shorter than plugin_timeout_seconds".to_owned(),
            );
        }

        require_range(
            "max_request_bytes",
            self.max_request_bytes,
            1,
            16 * 1024 * 1024,
        )?;
        require_range(
            "max_frame_bytes",
            self.max_frame_bytes,
            1,
            MAX_FRAME_BYTES_CEILING,
        )?;
        require_range(
            "max_stdout_bytes",
            self.max_stdout_bytes,
            self.max_frame_bytes,
            512 * 1024 * 1024,
        )?;
        require_range("max_event_frames", self.max_event_frames, 1, 1_000_000)?;
        require_range(
            "max_stderr_bytes",
            self.max_stderr_bytes,
            1,
            16 * 1024 * 1024,
        )?;
        require_range("event_queue_bound", self.event_queue_bound, 1, 128)?;
        let plugin_queue_bytes = self
            .max_frame_bytes
            .checked_mul(self.event_queue_bound)
            .and_then(|value| value.checked_mul(2))
            .ok_or_else(|| "plugin frame queue size overflows usize".to_owned())?;
        if plugin_queue_bytes > MAX_NATIVE_QUEUE_BYTES {
            return Err(format!(
                "2 * max_frame_bytes * event_queue_bound must not exceed {MAX_NATIVE_QUEUE_BYTES} bytes"
            ));
        }
        require_range(
            "native_max_line_bytes",
            self.native_max_line_bytes,
            1,
            MAX_FRAME_BYTES_CEILING,
        )?;
        require_range(
            "native_max_stdout_bytes",
            self.native_max_stdout_bytes,
            self.native_max_line_bytes,
            MAX_NATIVE_STDOUT_BYTES_CEILING,
        )?;
        require_range(
            "native_max_result_bytes",
            self.native_max_result_bytes,
            1,
            MAX_PROVIDER_RESULT_BYTES_CEILING,
        )?;
        require_range(
            "native_max_stderr_bytes",
            self.native_max_stderr_bytes,
            1,
            16 * 1024 * 1024,
        )?;
        require_range(
            "native_output_queue_bound",
            self.native_output_queue_bound,
            1,
            128,
        )?;
        let queue_bytes = self
            .native_max_line_bytes
            .checked_mul(self.native_output_queue_bound)
            .ok_or_else(|| "native output queue size overflows usize".to_owned())?;
        if queue_bytes > MAX_NATIVE_QUEUE_BYTES {
            return Err(format!(
                "native_max_line_bytes * native_output_queue_bound must not exceed {MAX_NATIVE_QUEUE_BYTES} bytes"
            ));
        }
        // Provider text is nested as a JSON string. A control byte can expand
        // to six bytes (`\u00xx`), and the echoed request id is already bounded
        // by the encoded request budget. Prove the worst case before any
        // external operation starts.
        let required_provider_frame = self
            .native_max_result_bytes
            .checked_mul(6)
            .and_then(|value| value.checked_add(self.max_request_bytes))
            .and_then(|value| value.checked_add(64 * 1024))
            .ok_or_else(|| "provider terminal frame budget overflows usize".to_owned())?;
        if self.max_frame_bytes < required_provider_frame {
            return Err(
                "max_frame_bytes must cover 6 * native_max_result_bytes + max_request_bytes + 65536 bytes"
                    .to_owned(),
            );
        }
        Ok(())
    }

    /// Validate provider option overrides against both hard adapter ceilings
    /// and the actual supervising deadline before a native process is spawned.
    pub fn validate_provider_options(
        &self,
        options: &JsonMap<String, JsonValue>,
        supervisor_timeout_seconds: u64,
    ) -> Result<(), String> {
        let max_line_bytes = provider_usize_option(
            options,
            "native_max_line_bytes",
            self.native_max_line_bytes,
            MAX_FRAME_BYTES_CEILING,
        )?;
        let max_stdout_bytes = provider_usize_option(
            options,
            "native_max_stdout_bytes",
            self.native_max_stdout_bytes,
            MAX_NATIVE_STDOUT_BYTES_CEILING,
        )?;
        let max_result_bytes = provider_usize_option(
            options,
            "native_max_result_bytes",
            self.native_max_result_bytes,
            MAX_PROVIDER_RESULT_BYTES_CEILING,
        )?;
        provider_usize_option(
            options,
            "native_max_stderr_bytes",
            self.native_max_stderr_bytes,
            16 * 1024 * 1024,
        )?;
        let output_queue_bound = provider_usize_option(
            options,
            "native_output_queue_bound",
            self.native_output_queue_bound,
            128,
        )?;
        if max_stdout_bytes < max_line_bytes {
            return Err(
                "provider native_max_stdout_bytes must not be smaller than native_max_line_bytes"
                    .to_owned(),
            );
        }
        let queue_bytes = max_line_bytes
            .checked_mul(output_queue_bound)
            .ok_or_else(|| "provider native output queue size overflows usize".to_owned())?;
        if queue_bytes > MAX_NATIVE_QUEUE_BYTES {
            return Err(format!(
                "provider native_max_line_bytes * native_output_queue_bound must not exceed {MAX_NATIVE_QUEUE_BYTES} bytes"
            ));
        }
        let required_frame = max_result_bytes
            .checked_mul(6)
            .and_then(|value| value.checked_add(self.max_request_bytes))
            .and_then(|value| value.checked_add(64 * 1024))
            .ok_or_else(|| "provider terminal frame budget overflows usize".to_owned())?;
        if self.max_frame_bytes < required_frame {
            return Err(format!(
                "provider native_max_result_bytes requires at least {required_frame} max_frame_bytes after JSON escaping"
            ));
        }
        let native_timeout = match options.get("timeout_seconds") {
            None => self.provider_timeout_seconds.saturating_sub(60) as f64,
            Some(JsonValue::Number(value)) => value.as_f64().ok_or_else(|| {
                "provider options.timeout_seconds must be a finite number".to_owned()
            })?,
            Some(_) => {
                return Err("provider options.timeout_seconds must be a number".to_owned());
            }
        };
        if !native_timeout.is_finite()
            || native_timeout <= 0.0
            || native_timeout > MAX_TIMEOUT_SECONDS as f64
        {
            return Err(format!(
                "provider options.timeout_seconds must be between 0 and {MAX_TIMEOUT_SECONDS}"
            ));
        }
        if supervisor_timeout_seconds != 0
            && native_timeout + 60.0 > supervisor_timeout_seconds as f64
        {
            return Err(
                "provider options.timeout_seconds must leave at least 60 seconds for supervisor cleanup"
                    .to_owned(),
            );
        }
        Ok(())
    }
}

fn provider_usize_option(
    options: &JsonMap<String, JsonValue>,
    name: &str,
    default: usize,
    maximum: usize,
) -> Result<usize, String> {
    match options.get(name) {
        None => Ok(default),
        Some(JsonValue::Number(value)) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value >= 1 && *value <= maximum)
            .ok_or_else(|| format!("provider options.{name} must be between 1 and {maximum}")),
        Some(_) => Err(format!("provider options.{name} must be an integer")),
    }
}

fn require_range<T>(name: &str, value: T, minimum: T, maximum: T) -> Result<(), String>
where
    T: Copy + Ord + std::fmt::Display,
{
    if value < minimum || value > maximum {
        Err(format!(
            "limits.{name} must be between {minimum} and {maximum}, received {value}"
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_keep_nested_deadline_headroom() {
        let limits = RuntimeLimits::default();
        limits.validate().expect("default limits");
        assert!(limits.provider_timeout_seconds < limits.provider_outer_timeout_seconds);
        assert!(limits.provider_outer_timeout_seconds < limits.script_timeout_seconds);
        assert_eq!(limits.native_max_stdout_bytes, 1024 * 1024 * 1024);
        assert_eq!(limits.max_concurrent_provider_calls, 4);
    }

    #[test]
    fn rejects_a_resident_queue_larger_than_the_hard_budget() {
        let limits = RuntimeLimits {
            native_output_queue_bound: 33,
            ..RuntimeLimits::default()
        };
        assert!(
            limits
                .validate()
                .expect_err("oversized queue")
                .contains("native_max_line_bytes * native_output_queue_bound")
        );
    }

    #[test]
    fn rejects_an_outer_deadline_that_can_preempt_provider_cleanup() {
        let limits = RuntimeLimits {
            provider_outer_timeout_seconds: RuntimeLimits::default().provider_timeout_seconds,
            ..RuntimeLimits::default()
        };
        assert!(
            limits
                .validate()
                .expect_err("missing headroom")
                .contains("leave at least 60 seconds")
        );
    }

    #[test]
    fn partial_toml_uses_safe_defaults() {
        let limits: RuntimeLimits = toml::from_str(
            "max_concurrent_provider_calls = 2\nnative_max_result_bytes = 2097152\n",
        )
        .expect("partial limits");
        assert_eq!(limits.max_concurrent_provider_calls, 2);
        assert_eq!(limits.native_max_result_bytes, 2 * 1024 * 1024);
        assert_eq!(limits.native_max_stdout_bytes, 1024 * 1024 * 1024);
        limits.validate().expect("partial limits valid");
    }

    #[test]
    fn provider_overrides_are_checked_against_frame_and_deadline_budgets() {
        let limits = RuntimeLimits {
            max_frame_bytes: 30 * 1024 * 1024,
            ..RuntimeLimits::default()
        };
        limits
            .validate()
            .expect("base limits still fit their default result");
        let oversized_result = JsonMap::from_iter([(
            "native_max_result_bytes".to_owned(),
            JsonValue::from(5 * 1024 * 1024_u64),
        )]);
        assert!(
            limits
                .validate_provider_options(&oversized_result, 14_400)
                .expect_err("escaped terminal frame must fit")
                .contains("after JSON escaping")
        );

        let too_close =
            JsonMap::from_iter([("timeout_seconds".to_owned(), JsonValue::from(7_150_u64))]);
        assert!(
            RuntimeLimits::default()
                .validate_provider_options(&too_close, 7_200)
                .expect_err("cleanup headroom")
                .contains("leave at least 60 seconds")
        );
        let valid =
            JsonMap::from_iter([("timeout_seconds".to_owned(), JsonValue::from(7_140_u64))]);
        RuntimeLimits::default()
            .validate_provider_options(&valid, 7_200)
            .expect("exact cleanup headroom");
    }

    #[test]
    fn worst_case_json_escaping_is_proved_before_spawn() {
        let limits = RuntimeLimits::default();
        let worst_case_encoded = limits
            .native_max_result_bytes
            .checked_mul(6)
            .and_then(|bytes| bytes.checked_add(limits.max_request_bytes))
            .and_then(|bytes| bytes.checked_add(64 * 1024))
            .expect("representable default budget");
        assert!(limits.max_frame_bytes >= worst_case_encoded);
    }
}
