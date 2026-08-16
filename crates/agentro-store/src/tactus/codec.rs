use agentro_contracts::{RequestId, Sha256Digest};
use thiserror::Error;

use super::model::{
    BlobRef, CellKey, CellState, CheckpointBackend, CheckpointEntryKind, CheckpointKey,
    LeaseOwnerKey, OutputStream, ProjectKey, RollbackFidelity, RunKey, RunState, TransactionState,
    WorkspaceTransactionKey,
};

/// A persisted Tactus value violates the frozen schema or codec contract.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum CorruptStorageError {
    /// Text was not one of a column's closed enum values.
    #[error("unknown durable enum value in {column}")]
    UnknownEnum {
        /// Stable column name.
        column: &'static str,
    },
    /// A non-negative durable integer was negative.
    #[error("negative durable integer in {column}")]
    NegativeInteger {
        /// Stable column name.
        column: &'static str,
    },
    /// A positive durable integer was zero.
    #[error("zero durable integer in {column}")]
    ZeroInteger {
        /// Stable column name.
        column: &'static str,
    },
    /// An SQLite integer used as a Boolean was neither zero nor one.
    #[error("invalid durable Boolean in {column}")]
    InvalidBoolean {
        /// Stable column name.
        column: &'static str,
    },
    /// An identifier column was not canonical UUIDv7 text.
    #[error("invalid durable identifier in {column}")]
    InvalidIdentifier {
        /// Stable column name.
        column: &'static str,
    },
    /// A digest column was not canonical SHA-256 text.
    #[error("invalid durable digest in {column}")]
    InvalidDigest {
        /// Stable column name.
        column: &'static str,
    },
    /// A nullable blob digest and length did not have matching presence.
    #[error("partial durable blob reference in {column}")]
    PartialBlobReference {
        /// Stable digest column name.
        column: &'static str,
    },
}

/// A valid in-memory value cannot be represented by SQLite's signed integer.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("value for {column} exceeds SQLite integer range")]
pub struct IntegerOverflowError {
    /// Stable destination column name.
    pub column: &'static str,
}

macro_rules! string_codec {
    ($type:ty, {$($variant:path => $text:literal),+ $(,)?}) => {
        impl $type {
            /// Returns the frozen SQLite text representation.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($variant => $text),+
                }
            }

            /// Decodes one frozen SQLite text representation.
            ///
            /// # Errors
            ///
            /// Returns [`CorruptStorageError::UnknownEnum`] for unknown text.
            pub fn decode(
                column: &'static str,
                value: &str,
            ) -> Result<Self, CorruptStorageError> {
                match value {
                    $($text => Ok($variant)),+,
                    _ => Err(CorruptStorageError::UnknownEnum { column }),
                }
            }
        }
    };
}

string_codec!(RunState, {
    RunState::Pending => "pending",
    RunState::Running => "running",
    RunState::Cancelling => "cancelling",
    RunState::Recovering => "recovering",
    RunState::Succeeded => "succeeded",
    RunState::Failed => "failed",
    RunState::Cancelled => "cancelled",
    RunState::Interrupted => "interrupted",
});
string_codec!(CellState, {
    CellState::Queued => "queued",
    CellState::Running => "running",
    CellState::Recovering => "recovering",
    CellState::Succeeded => "succeeded",
    CellState::Failed => "failed",
    CellState::Cancelled => "cancelled",
    CellState::Interrupted => "interrupted",
});
string_codec!(TransactionState, {
    TransactionState::Prepared => "prepared",
    TransactionState::Active => "active",
    TransactionState::Committed => "committed",
    TransactionState::Abandoned => "abandoned",
    TransactionState::Conflict => "conflict",
});
string_codec!(OutputStream, {
    OutputStream::Stdout => "stdout",
    OutputStream::Stderr => "stderr",
    OutputStream::Display => "display",
});
string_codec!(CheckpointBackend, {
    CheckpointBackend::NonGit => "non_git",
    CheckpointBackend::GitAware => "git_aware",
});
string_codec!(RollbackFidelity, {
    RollbackFidelity::FullManifest => "full_manifest",
    RollbackFidelity::DeclaredPaths => "declared_paths",
});
string_codec!(CheckpointEntryKind, {
    CheckpointEntryKind::File => "file",
    CheckpointEntryKind::Symlink => "symlink",
});

/// Decodes an SQLite integer that may be zero.
///
/// # Errors
///
/// Returns [`CorruptStorageError::NegativeInteger`] for negative values.
pub fn decode_non_negative(column: &'static str, value: i64) -> Result<u64, CorruptStorageError> {
    u64::try_from(value).map_err(|_| CorruptStorageError::NegativeInteger { column })
}

/// Decodes an SQLite integer that must be positive.
///
/// # Errors
///
/// Returns a typed negative or zero corruption error.
pub fn decode_positive(column: &'static str, value: i64) -> Result<u64, CorruptStorageError> {
    let value = decode_non_negative(column, value)?;
    if value == 0 {
        Err(CorruptStorageError::ZeroInteger { column })
    } else {
        Ok(value)
    }
}

/// Decodes SQLite's exact zero/one Boolean representation.
///
/// # Errors
///
/// Returns [`CorruptStorageError::InvalidBoolean`] for every other integer.
pub fn decode_boolean(column: &'static str, value: i64) -> Result<bool, CorruptStorageError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(CorruptStorageError::InvalidBoolean { column }),
    }
}

/// Encodes an unsigned value into SQLite's signed integer range.
///
/// # Errors
///
/// Returns [`IntegerOverflowError`] when the value is too large.
pub fn encode_integer(column: &'static str, value: u64) -> Result<i64, IntegerOverflowError> {
    i64::try_from(value).map_err(|_| IntegerOverflowError { column })
}

macro_rules! uuid_decoder {
    ($function:ident, $type:ty) => {
        /// Decodes one canonical UUIDv7 storage key.
        ///
        /// # Errors
        ///
        /// Returns [`CorruptStorageError::InvalidIdentifier`] for invalid text.
        pub fn $function(column: &'static str, value: &str) -> Result<$type, CorruptStorageError> {
            <$type>::parse(value).map_err(|_| CorruptStorageError::InvalidIdentifier { column })
        }
    };
}

uuid_decoder!(decode_project_key, ProjectKey);
uuid_decoder!(decode_cell_key, CellKey);
uuid_decoder!(decode_run_key, RunKey);
uuid_decoder!(decode_transaction_key, WorkspaceTransactionKey);
uuid_decoder!(decode_lease_owner_key, LeaseOwnerKey);

/// Decodes a stable request identifier.
///
/// # Errors
///
/// Returns [`CorruptStorageError::InvalidIdentifier`] for invalid text.
pub fn decode_request_id(
    column: &'static str,
    value: &str,
) -> Result<RequestId, CorruptStorageError> {
    RequestId::parse(value).map_err(|_| CorruptStorageError::InvalidIdentifier { column })
}

/// Decodes canonical SHA-256 text.
///
/// # Errors
///
/// Returns [`CorruptStorageError::InvalidDigest`] for invalid text.
pub fn decode_digest(
    column: &'static str,
    value: &str,
) -> Result<Sha256Digest, CorruptStorageError> {
    Sha256Digest::parse(value).map_err(|_| CorruptStorageError::InvalidDigest { column })
}

/// Decodes a content-derived checkpoint key.
///
/// # Errors
///
/// Returns [`CorruptStorageError::InvalidDigest`] for invalid text.
pub fn decode_checkpoint_key(
    column: &'static str,
    value: &str,
) -> Result<CheckpointKey, CorruptStorageError> {
    decode_digest(column, value).map(CheckpointKey::from_digest)
}

/// Decodes nullable blob columns while enforcing matching presence.
///
/// # Errors
///
/// Returns a typed digest, integer, or partial-reference corruption error.
pub fn decode_optional_blob(
    digest_column: &'static str,
    digest: Option<&str>,
    length_column: &'static str,
    length: Option<i64>,
) -> Result<Option<BlobRef>, CorruptStorageError> {
    match (digest, length) {
        (Some(digest), Some(length)) => Ok(Some(BlobRef {
            digest: decode_digest(digest_column, digest)?,
            length: decode_non_negative(length_column, length)?,
        })),
        (None, None) => Ok(None),
        _ => Err(CorruptStorageError::PartialBlobReference {
            column: digest_column,
        }),
    }
}
