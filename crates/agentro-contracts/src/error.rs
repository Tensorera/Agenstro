use std::{fmt, str::FromStr};

use thiserror::Error;

use crate::TraceId;

/// Maximum encoded length of a stable machine error code.
pub const MAX_ERROR_CODE_BYTES: usize = 64;

/// The authority that classified an error.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum ErrorDomain {
    /// Normalized agent orchestration.
    Agent,
    /// Supervised process execution.
    Process,
    /// Python or Jupyter kernel execution.
    Kernel,
    /// Workspace checkpoint and restore.
    Checkpoint,
    /// Durable storage.
    Storage,
    /// Schedule, occurrence, lease, or dispatch.
    Scheduler,
    /// Versioned configuration.
    Config,
    /// Cross-process protocol validation or compatibility.
    Protocol,
    /// A bounded resource, queue, stream, or quota.
    Resource,
}

/// Typed retry guidance that does not require parsing an error message.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RetryAdvice {
    /// Automatic retry is not safe.
    Never,
    /// A bounded, idempotent retry may occur after backoff.
    AfterBackoff,
    /// Retry requires an explicit user correction or approval.
    AfterUserAction,
}

/// A validated, stable upper-case machine error code.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ErrorCode(Box<str>);

impl ErrorCode {
    /// Parses an upper-case machine code such as `PROTOCOL_VERSION_UNSUPPORTED`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCodeError`] for empty, oversized, or malformed input.
    pub fn parse(value: &str) -> Result<Self, ErrorCodeError> {
        if value.is_empty() {
            return Err(ErrorCodeError::Empty);
        }
        if value.len() > MAX_ERROR_CODE_BYTES {
            return Err(ErrorCodeError::TooLong);
        }

        let bytes = value.as_bytes();
        if !bytes[0].is_ascii_uppercase()
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
            || bytes.last() == Some(&b'_')
            || bytes.windows(2).any(|pair| pair == b"__")
        {
            return Err(ErrorCodeError::InvalidFormat);
        }

        Ok(Self(value.into()))
    }

    /// Returns the stable wire representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ErrorCode {
    type Err = ErrorCodeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// A failure to validate a stable machine error code.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ErrorCodeError {
    /// The code is empty.
    #[error("error code must not be empty")]
    Empty,
    /// The code exceeds [`MAX_ERROR_CODE_BYTES`].
    #[error("error code exceeds its byte limit")]
    TooLong,
    /// The code does not match the upper-case underscore-separated grammar.
    #[error("error code has an invalid format")]
    InvalidFormat,
}

/// Stable machine fields attached to a transport-specific error status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorDescriptor {
    domain: ErrorDomain,
    code: ErrorCode,
    retry: RetryAdvice,
    trace_id: TraceId,
}

impl ErrorDescriptor {
    /// Creates a stable error descriptor.
    #[must_use]
    pub const fn new(
        domain: ErrorDomain,
        code: ErrorCode,
        retry: RetryAdvice,
        trace_id: TraceId,
    ) -> Self {
        Self {
            domain,
            code,
            retry,
            trace_id,
        }
    }

    /// Returns the authority that classified the error.
    #[must_use]
    pub const fn domain(&self) -> ErrorDomain {
        self.domain
    }

    /// Returns the stable machine code.
    #[must_use]
    pub const fn code(&self) -> &ErrorCode {
        &self.code
    }

    /// Returns typed retry guidance.
    #[must_use]
    pub const fn retry(&self) -> RetryAdvice {
        self.retry
    }

    /// Returns the opaque diagnostic trace identifier.
    #[must_use]
    pub const fn trace_id(&self) -> TraceId {
        self.trace_id
    }
}

#[cfg(test)]
mod tests {
    use super::{ErrorCode, ErrorCodeError};

    #[test]
    fn machine_code_grammar_is_stable() -> Result<(), ErrorCodeError> {
        let code = ErrorCode::parse("PROTOCOL_VERSION_UNSUPPORTED")?;

        assert_eq!(code.as_str(), "PROTOCOL_VERSION_UNSUPPORTED");
        assert_eq!(
            ErrorCode::parse("protocol_version_unsupported"),
            Err(ErrorCodeError::InvalidFormat)
        );
        assert_eq!(
            ErrorCode::parse("PROTOCOL__VERSION"),
            Err(ErrorCodeError::InvalidFormat)
        );
        Ok(())
    }
}
