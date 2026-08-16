use std::{fmt, num::NonZeroU64, str::FromStr};

use thiserror::Error;
use uuid::{Uuid, Variant};

/// A malformed stable Tactus identifier or fencing token.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum TactusIdError {
    /// The identifier is not syntactically valid UUID text.
    #[error("identifier is not a valid UUID")]
    Malformed,
    /// The identifier is not canonical lower-case hyphenated text.
    #[error("identifier is not in canonical lower-case UUID form")]
    NonCanonical,
    /// The identifier does not use the RFC UUID variant.
    #[error("identifier does not use the RFC UUID variant")]
    WrongVariant,
    /// The identifier is not UUIDv7.
    #[error("identifier is not UUIDv7")]
    WrongVersion,
    /// Fencing tokens start at one.
    #[error("fencing token must be greater than zero")]
    ZeroFence,
}

macro_rules! define_uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a monotonic UUIDv7 identity.
            #[must_use]
            pub fn generate() -> Self {
                Self(Uuid::now_v7())
            }

            /// Parses canonical lower-case UUIDv7 text.
            ///
            /// # Errors
            ///
            /// Returns [`TactusIdError`] for malformed, non-canonical,
            /// non-RFC, or non-v7 input.
            pub fn parse(value: &str) -> Result<Self, TactusIdError> {
                let uuid = Uuid::try_parse(value).map_err(|_| TactusIdError::Malformed)?;
                if uuid.get_variant() != Variant::RFC4122 {
                    return Err(TactusIdError::WrongVariant);
                }
                if uuid.get_version_num() != 7 {
                    return Err(TactusIdError::WrongVersion);
                }
                if uuid.hyphenated().to_string() != value {
                    return Err(TactusIdError::NonCanonical);
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
            type Err = TactusIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

define_uuid_id!(
    /// Stable identity of one project registration.
    ProjectId
);
define_uuid_id!(
    /// Stable cell identity, independent of source text and script position.
    CellId
);
define_uuid_id!(
    /// Identity of one immutable execution attempt.
    RunId
);
define_uuid_id!(
    /// Identity of one fenced workspace transaction.
    WorkspaceTransactionId
);
define_uuid_id!(
    /// Identity of the process instance claiming a project lease.
    LeaseOwnerId
);

/// A monotonically increasing project writer token.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FencingToken(NonZeroU64);

impl FencingToken {
    /// Creates a non-zero fencing token.
    ///
    /// # Errors
    ///
    /// Returns [`TactusIdError::ZeroFence`] for zero.
    pub fn new(value: u64) -> Result<Self, TactusIdError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(TactusIdError::ZeroFence)
    }

    /// Returns the durable integer representation.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0.get()
    }
}

impl fmt::Display for FencingToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value().fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::{CellId, FencingToken, TactusIdError};

    #[test]
    fn stable_ids_round_trip_only_canonical_uuid_v7() -> Result<(), TactusIdError> {
        let id = CellId::generate();
        assert_eq!(CellId::parse(&id.to_string())?, id);
        assert_eq!(
            CellId::parse(&id.to_string().to_uppercase()),
            Err(TactusIdError::NonCanonical)
        );
        assert_eq!(
            CellId::parse("550e8400-e29b-41d4-a716-446655440000"),
            Err(TactusIdError::WrongVersion)
        );
        Ok(())
    }

    #[test]
    fn fencing_tokens_cannot_be_zero() {
        assert_eq!(FencingToken::new(0), Err(TactusIdError::ZeroFence));
        assert_eq!(FencingToken::new(1).map(FencingToken::value), Ok(1));
    }
}
