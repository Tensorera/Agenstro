use std::{fmt, str::FromStr};

use thiserror::Error;
use uuid::{Uuid, Variant};

/// A failure to parse a stable UUIDv7 identifier.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum IdError {
    /// The input is not a syntactically valid UUID.
    #[error("identifier is not a valid UUID")]
    Malformed,
    /// The input is not the canonical lower-case, hyphenated representation.
    #[error("identifier is not in canonical lower-case UUID form")]
    NonCanonical,
    /// The UUID does not use the RFC variant expected by UUIDv7.
    #[error("identifier does not use the RFC UUID variant")]
    WrongVariant,
    /// The UUID is not version 7.
    #[error("identifier is not UUIDv7")]
    WrongVersion,
}

macro_rules! define_uuid_v7_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a process-local monotonic UUIDv7 using the system clock and RNG.
            #[must_use]
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }

            /// Parses a canonical lower-case, hyphenated UUIDv7.
            ///
            /// # Errors
            ///
            /// Returns [`IdError`] when the value is malformed, non-canonical, uses
            /// a non-RFC variant, or is not version 7.
            pub fn parse(value: &str) -> Result<Self, IdError> {
                let uuid = Uuid::try_parse(value).map_err(|_| IdError::Malformed)?;
                if uuid.get_variant() != Variant::RFC4122 {
                    return Err(IdError::WrongVariant);
                }
                if uuid.get_version_num() != 7 {
                    return Err(IdError::WrongVersion);
                }
                if uuid.hyphenated().to_string() != value {
                    return Err(IdError::NonCanonical);
                }
                Ok(Self(uuid))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.hyphenated().fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

define_uuid_v7_id!(
    /// Identifies one daemon instance and rotates on every launch.
    DaemonInstanceId
);
define_uuid_v7_id!(
    /// Identifies one idempotent command request in its documented scope.
    RequestId
);
define_uuid_v7_id!(
    /// Correlates a bounded diagnostic without exposing internal error text.
    TraceId
);

#[cfg(test)]
mod tests {
    use super::{DaemonInstanceId, IdError};

    #[test]
    fn generated_identifier_round_trips_canonical_text() -> Result<(), IdError> {
        let generated = DaemonInstanceId::generate();
        let parsed = DaemonInstanceId::parse(&generated.to_string())?;

        assert_eq!(parsed, generated);
        Ok(())
    }

    #[test]
    fn parser_rejects_other_uuid_versions() {
        let result = DaemonInstanceId::parse("550e8400-e29b-41d4-a716-446655440000");

        assert_eq!(result, Err(IdError::WrongVersion));
    }

    #[test]
    fn parser_rejects_noncanonical_text() {
        let generated = DaemonInstanceId::generate().to_string().to_uppercase();
        let result = DaemonInstanceId::parse(&generated);

        assert_eq!(result, Err(IdError::NonCanonical));
    }
}
