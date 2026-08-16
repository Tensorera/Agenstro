use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::PathBuf,
    time::Duration,
};

use thiserror::Error;

/// Hard maximum number of arguments accepted by one process specification.
pub const MAX_ARGUMENTS: usize = 4_096;
/// Hard maximum encoded bytes across all arguments.
pub const MAX_ARGUMENT_BYTES: u64 = 1_048_576;
/// Hard maximum number of explicitly supplied environment variables.
pub const MAX_ENVIRONMENT_VARIABLES: usize = 256;
/// Hard maximum encoded bytes across environment names and values.
pub const MAX_ENVIRONMENT_BYTES: u64 = 1_048_576;
/// Hard maximum bytes retained from either output stream.
pub const MAX_CAPTURE_BYTES: u64 = 16 * 1_048_576;
/// Hard maximum bytes retained across both output streams.
pub const MAX_TOTAL_CAPTURE_BYTES: u64 = 32 * 1_048_576;
const MAX_PROCESS_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_TERMINATION_GRACE: Duration = Duration::from_secs(60);

/// Invalid or unsafe process specification.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SpecError {
    /// The executable path was not absolute.
    #[error("the executable path must be absolute")]
    ExecutableNotAbsolute,
    /// The working directory path was not absolute.
    #[error("the working directory path must be absolute")]
    WorkingDirectoryNotAbsolute,
    /// An argument or environment item contained a NUL code point.
    #[error("{field} contains a NUL code point")]
    ContainsNul {
        /// Name of the invalid field.
        field: &'static str,
    },
    /// Too many arguments were supplied.
    #[error("argument count {actual} exceeds hard limit {limit}")]
    TooManyArguments {
        /// Observed argument count.
        actual: usize,
        /// Configured hard limit.
        limit: usize,
    },
    /// Encoded argument bytes exceeded the hard limit.
    #[error("argument bytes exceed hard limit {limit}")]
    ArgumentBytesExceeded {
        /// Configured hard limit.
        limit: u64,
    },
    /// Too many environment variables were supplied.
    #[error("environment count {actual} exceeds hard limit {limit}")]
    TooManyEnvironmentVariables {
        /// Observed variable count.
        actual: usize,
        /// Configured hard limit.
        limit: usize,
    },
    /// Encoded environment bytes exceeded the hard limit.
    #[error("environment bytes exceed hard limit {limit}")]
    EnvironmentBytesExceeded {
        /// Configured hard limit.
        limit: u64,
    },
    /// An environment variable name was empty or contained `=`.
    #[error("environment variable name is invalid")]
    InvalidEnvironmentName,
    /// A duration was zero or above its hard limit.
    #[error("{field} must be non-zero and no greater than {maximum:?}")]
    InvalidDuration {
        /// Name of the invalid duration.
        field: &'static str,
        /// Maximum accepted duration.
        maximum: Duration,
    },
    /// An output budget was zero or above its hard limit.
    #[error("{stream} output budget must be non-zero and no greater than {maximum}")]
    InvalidOutputBudget {
        /// Output stream or aggregate name.
        stream: &'static str,
        /// Maximum accepted byte count.
        maximum: u64,
    },
    /// A requested resource limit was zero.
    #[error("{resource} resource limit must be non-zero")]
    ZeroResourceLimit {
        /// Resource whose limit was zero.
        resource: &'static str,
    },
}

/// Per-stream and aggregate in-memory output bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutputBudget {
    stdout_bytes: u64,
    stderr_bytes: u64,
    total_bytes: u64,
}

impl OutputBudget {
    /// Constructs a validated output budget.
    ///
    /// # Errors
    ///
    /// Returns [`SpecError::InvalidOutputBudget`] when any limit is zero or
    /// exceeds the corresponding hard maximum.
    pub fn new(stdout_bytes: u64, stderr_bytes: u64, total_bytes: u64) -> Result<Self, SpecError> {
        validate_budget("stdout", stdout_bytes, MAX_CAPTURE_BYTES)?;
        validate_budget("stderr", stderr_bytes, MAX_CAPTURE_BYTES)?;
        validate_budget("combined", total_bytes, MAX_TOTAL_CAPTURE_BYTES)?;
        Ok(Self {
            stdout_bytes,
            stderr_bytes,
            total_bytes,
        })
    }

    pub(crate) fn stdout_bytes(self) -> u64 {
        self.stdout_bytes
    }

    pub(crate) fn stderr_bytes(self) -> u64 {
        self.stderr_bytes
    }

    pub(crate) fn total_bytes(self) -> u64 {
        self.total_bytes
    }
}

fn validate_budget(stream: &'static str, value: u64, maximum: u64) -> Result<(), SpecError> {
    if value == 0 || value > maximum {
        return Err(SpecError::InvalidOutputBudget { stream, maximum });
    }
    Ok(())
}

/// Absolute run deadline and bounded termination grace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessTimeouts {
    overall: Duration,
    termination_grace: Duration,
}

impl ProcessTimeouts {
    /// Constructs validated process timeouts.
    ///
    /// # Errors
    ///
    /// Returns [`SpecError::InvalidDuration`] for zero or excessive values.
    pub fn new(overall: Duration, termination_grace: Duration) -> Result<Self, SpecError> {
        validate_duration("overall timeout", overall, MAX_PROCESS_TIMEOUT)?;
        validate_duration(
            "termination grace",
            termination_grace,
            MAX_TERMINATION_GRACE,
        )?;
        Ok(Self {
            overall,
            termination_grace,
        })
    }

    pub(crate) fn overall(self) -> Duration {
        self.overall
    }

    pub(crate) fn termination_grace(self) -> Duration {
        self.termination_grace
    }
}

fn validate_duration(
    field: &'static str,
    value: Duration,
    maximum: Duration,
) -> Result<(), SpecError> {
    if value.is_zero() || value > maximum {
        return Err(SpecError::InvalidDuration { field, maximum });
    }
    Ok(())
}

/// Optional hard operating-system resource limits.
///
/// The first slice exposes the contract but the native implementation reports
/// these limits as unsupported rather than silently claiming enforcement.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceLimits {
    memory_bytes: Option<u64>,
    process_count: Option<u32>,
    cpu_time: Option<Duration>,
}

impl ResourceLimits {
    /// Constructs validated optional resource limits.
    ///
    /// # Errors
    ///
    /// Returns [`SpecError::ZeroResourceLimit`] when a supplied limit is zero.
    pub fn new(
        memory_bytes: Option<u64>,
        process_count: Option<u32>,
        cpu_time: Option<Duration>,
    ) -> Result<Self, SpecError> {
        if memory_bytes == Some(0) {
            return Err(SpecError::ZeroResourceLimit { resource: "memory" });
        }
        if process_count == Some(0) {
            return Err(SpecError::ZeroResourceLimit {
                resource: "process count",
            });
        }
        if cpu_time == Some(Duration::ZERO) {
            return Err(SpecError::ZeroResourceLimit {
                resource: "CPU time",
            });
        }
        Ok(Self {
            memory_bytes,
            process_count,
            cpu_time,
        })
    }

    pub(crate) fn first_requested(self) -> Option<&'static str> {
        if self.memory_bytes.is_some() {
            Some("memory")
        } else if self.process_count.is_some() {
            Some("process count")
        } else if self.cpu_time.is_some() {
            Some("CPU time")
        } else {
            None
        }
    }
}

/// A fully validated, shell-free process request.
pub struct ProcessSpec {
    executable: PathBuf,
    arguments: Vec<OsString>,
    working_directory: PathBuf,
    environment: BTreeMap<OsString, OsString>,
    timeouts: ProcessTimeouts,
    output_budget: OutputBudget,
    resource_limits: ResourceLimits,
}

impl ProcessSpec {
    /// Builds a process request from an executable and an argument vector.
    ///
    /// The environment is explicit and the supervisor will clear all inherited
    /// variables before applying it.
    ///
    /// # Errors
    ///
    /// Returns [`SpecError`] when paths, arguments, environment, or hard limits
    /// violate the bounded process contract.
    pub fn new(
        executable: PathBuf,
        arguments: Vec<OsString>,
        working_directory: PathBuf,
        environment: BTreeMap<OsString, OsString>,
        timeouts: ProcessTimeouts,
        output_budget: OutputBudget,
        resource_limits: ResourceLimits,
    ) -> Result<Self, SpecError> {
        if !executable.is_absolute() {
            return Err(SpecError::ExecutableNotAbsolute);
        }
        if !working_directory.is_absolute() {
            return Err(SpecError::WorkingDirectoryNotAbsolute);
        }
        validate_arguments(&arguments)?;
        validate_environment(&environment)?;
        Ok(Self {
            executable,
            arguments,
            working_directory,
            environment,
            timeouts,
            output_budget,
            resource_limits,
        })
    }

    pub(crate) fn executable(&self) -> &PathBuf {
        &self.executable
    }

    pub(crate) fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub(crate) fn working_directory(&self) -> &PathBuf {
        &self.working_directory
    }

    pub(crate) fn environment(&self) -> &BTreeMap<OsString, OsString> {
        &self.environment
    }

    pub(crate) fn timeouts(&self) -> ProcessTimeouts {
        self.timeouts
    }

    pub(crate) fn output_budget(&self) -> OutputBudget {
        self.output_budget
    }

    pub(crate) fn resource_limits(&self) -> ResourceLimits {
        self.resource_limits
    }
}

fn validate_arguments(arguments: &[OsString]) -> Result<(), SpecError> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(SpecError::TooManyArguments {
            actual: arguments.len(),
            limit: MAX_ARGUMENTS,
        });
    }
    let mut encoded_bytes = 0_u64;
    for argument in arguments {
        if contains_nul(argument) {
            return Err(SpecError::ContainsNul { field: "argument" });
        }
        encoded_bytes = encoded_bytes.checked_add(os_str_bytes(argument)).ok_or(
            SpecError::ArgumentBytesExceeded {
                limit: MAX_ARGUMENT_BYTES,
            },
        )?;
        if encoded_bytes > MAX_ARGUMENT_BYTES {
            return Err(SpecError::ArgumentBytesExceeded {
                limit: MAX_ARGUMENT_BYTES,
            });
        }
    }
    Ok(())
}

fn validate_environment(environment: &BTreeMap<OsString, OsString>) -> Result<(), SpecError> {
    if environment.len() > MAX_ENVIRONMENT_VARIABLES {
        return Err(SpecError::TooManyEnvironmentVariables {
            actual: environment.len(),
            limit: MAX_ENVIRONMENT_VARIABLES,
        });
    }
    let mut encoded_bytes = 0_u64;
    for (name, value) in environment {
        if name.is_empty() || os_str_contains(name, '=') {
            return Err(SpecError::InvalidEnvironmentName);
        }
        if contains_nul(name) || contains_nul(value) {
            return Err(SpecError::ContainsNul {
                field: "environment",
            });
        }
        encoded_bytes = encoded_bytes
            .checked_add(os_str_bytes(name))
            .and_then(|current| current.checked_add(os_str_bytes(value)))
            .ok_or(SpecError::EnvironmentBytesExceeded {
                limit: MAX_ENVIRONMENT_BYTES,
            })?;
        if encoded_bytes > MAX_ENVIRONMENT_BYTES {
            return Err(SpecError::EnvironmentBytesExceeded {
                limit: MAX_ENVIRONMENT_BYTES,
            });
        }
    }
    Ok(())
}

fn os_str_contains(value: &OsStr, needle: char) -> bool {
    value.to_string_lossy().contains(needle)
}

#[cfg(windows)]
fn contains_nul(value: &OsStr) -> bool {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().any(|unit| unit == 0)
}

#[cfg(unix)]
fn contains_nul(value: &OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().contains(&0)
}

#[cfg(windows)]
fn os_str_bytes(value: &OsStr) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    value
        .encode_wide()
        .fold(0_u64, |bytes, _| bytes.saturating_add(2))
}

#[cfg(unix)]
fn os_str_bytes(value: &OsStr) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    u64::try_from(value.as_bytes().len()).unwrap_or(u64::MAX)
}

#[cfg(not(any(unix, windows)))]
fn contains_nul(value: &OsStr) -> bool {
    value.to_string_lossy().contains('\0')
}

#[cfg(not(any(unix, windows)))]
fn os_str_bytes(value: &OsStr) -> u64 {
    u64::try_from(value.to_string_lossy().len()).unwrap_or(u64::MAX)
}
