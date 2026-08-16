use std::{fmt, str::FromStr};

use sha2::{Digest as _, Sha256};
use thiserror::Error;

const CANONICAL_MAGIC: &[u8] = b"agentro-canonical-v1\0";
const MAX_CANONICAL_DOMAIN_BYTES: usize = 128;
const MAX_CANONICAL_FIELD_NAME_BYTES: usize = 64;

/// Maximum number of fields accepted by one canonical digest operation.
pub const MAX_CANONICAL_FIELDS: u32 = 1_024;
/// Maximum byte length of one inline canonical field.
///
/// Larger content must be hashed separately and supplied by digest reference.
pub const MAX_CANONICAL_FIELD_BYTES: usize = 1024 * 1024;

/// A raw SHA-256 digest with a stable `sha256:<hex>` text representation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Creates a digest value from exactly 32 raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Parses the canonical `sha256:<64 lower-case hex digits>` form.
    ///
    /// # Errors
    ///
    /// Returns [`DigestError::MalformedDigest`] for another algorithm,
    /// length, character set, or case.
    pub fn parse(value: &str) -> Result<Self, DigestError> {
        let Some(encoded) = value.strip_prefix("sha256:") else {
            return Err(DigestError::MalformedDigest);
        };
        if encoded.len() != 64 {
            return Err(DigestError::MalformedDigest);
        }

        let encoded = encoded.as_bytes();
        let mut bytes = [0_u8; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            let high = decode_lower_hex(encoded[index * 2]).ok_or(DigestError::MalformedDigest)?;
            let low =
                decode_lower_hex(encoded[index * 2 + 1]).ok_or(DigestError::MalformedDigest)?;
            *byte = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

fn decode_lower_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for Sha256Digest {
    type Err = DigestError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Streaming entry point for the versioned canonical digest encoding.
///
/// The byte format uses a fixed magic prefix, tagged length-prefixed domain
/// and fields, strict ascending field names, and a terminal field count. This
/// encoding is independent of Protobuf serialization and map iteration order.
pub struct CanonicalHasher {
    inner: Sha256,
    last_field: Option<Box<str>>,
    field_count: u32,
}

impl CanonicalHasher {
    /// Starts a canonical digest in a lower-case namespaced domain.
    ///
    /// # Errors
    ///
    /// Returns [`DigestError::InvalidDomain`] for an empty, oversized, or
    /// malformed domain.
    pub fn new(domain: &str) -> Result<Self, DigestError> {
        if !valid_domain(domain) {
            return Err(DigestError::InvalidDomain);
        }

        let mut inner = Sha256::new();
        inner.update(CANONICAL_MAGIC);
        inner.update([1]);
        inner.update(encoded_u16(domain.len())?);
        inner.update(domain.as_bytes());
        Ok(Self {
            inner,
            last_field: None,
            field_count: 0,
        })
    }

    /// Adds one inline field in strict ascending name order.
    ///
    /// # Errors
    ///
    /// Returns [`DigestError`] when the name or value exceeds its bound, the
    /// name is malformed or out of order, or the field count is exhausted.
    pub fn write_field(&mut self, name: &str, value: &[u8]) -> Result<(), DigestError> {
        if !valid_field_name(name) {
            return Err(DigestError::InvalidFieldName);
        }
        if value.len() > MAX_CANONICAL_FIELD_BYTES {
            return Err(DigestError::FieldTooLarge);
        }
        if self.field_count >= MAX_CANONICAL_FIELDS {
            return Err(DigestError::TooManyFields);
        }
        if self
            .last_field
            .as_deref()
            .is_some_and(|previous| previous >= name)
        {
            return Err(DigestError::FieldsOutOfOrder);
        }

        self.inner.update([2]);
        self.inner.update(encoded_u16(name.len())?);
        self.inner.update(name.as_bytes());
        self.inner.update(encoded_u64(value.len())?);
        self.inner.update(value);
        self.last_field = Some(name.into());
        self.field_count += 1;
        Ok(())
    }

    /// Finishes the canonical stream and returns its SHA-256 digest.
    #[must_use]
    pub fn finish(mut self) -> Sha256Digest {
        self.inner.update([0]);
        self.inner.update(self.field_count.to_be_bytes());
        Sha256Digest(self.inner.finalize().into())
    }
}

fn valid_domain(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CANONICAL_DOMAIN_BYTES
        && value.split('.').count() >= 2
        && value.split('.').all(valid_domain_segment)
}

fn valid_domain_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn valid_field_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !value.is_empty()
        && value.len() <= MAX_CANONICAL_FIELD_NAME_BYTES
        && bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

fn encoded_u16(value: usize) -> Result<[u8; 2], DigestError> {
    u16::try_from(value)
        .map(u16::to_be_bytes)
        .map_err(|_| DigestError::LengthOverflow)
}

fn encoded_u64(value: usize) -> Result<[u8; 8], DigestError> {
    u64::try_from(value)
        .map(u64::to_be_bytes)
        .map_err(|_| DigestError::LengthOverflow)
}

/// A canonical encoding or SHA-256 text invariant violation.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DigestError {
    /// The algorithm-prefixed digest text is malformed or non-canonical.
    #[error("SHA-256 digest text is malformed")]
    MalformedDigest,
    /// The canonical domain is empty, oversized, or malformed.
    #[error("canonical digest domain is invalid")]
    InvalidDomain,
    /// A field name is empty, oversized, or malformed.
    #[error("canonical digest field name is invalid")]
    InvalidFieldName,
    /// Field names are duplicated or not in strict ascending order.
    #[error("canonical digest fields are not in strict ascending order")]
    FieldsOutOfOrder,
    /// The field count exceeds [`MAX_CANONICAL_FIELDS`].
    #[error("canonical digest field count exceeds its limit")]
    TooManyFields,
    /// One inline field exceeds [`MAX_CANONICAL_FIELD_BYTES`].
    #[error("canonical digest field exceeds its byte limit")]
    FieldTooLarge,
    /// A platform length cannot be represented in the versioned encoding.
    #[error("canonical digest length cannot be represented")]
    LengthOverflow,
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{CanonicalHasher, DigestError, Sha256Digest};

    proptest! {
        #[test]
        fn digest_text_round_trips_all_raw_values(bytes in any::<[u8; 32]>()) {
            let digest = Sha256Digest::from_bytes(bytes);
            prop_assert_eq!(Sha256Digest::parse(&digest.to_string()), Ok(digest));
        }

        #[test]
        fn changing_a_framed_value_changes_the_digest(value in prop::collection::vec(any::<u8>(), 0..256)) {
            let mut longer = value.clone();
            longer.push(0);

            let first = digest_one_field(&value);
            let second = digest_one_field(&longer);
            prop_assert!(first.is_ok());
            prop_assert!(second.is_ok());
            prop_assert_ne!(first, second);
        }
    }

    fn digest_one_field(value: &[u8]) -> Result<Sha256Digest, DigestError> {
        let mut hasher = CanonicalHasher::new("agentro.test")?;
        hasher.write_field("payload", value)?;
        Ok(hasher.finish())
    }

    #[test]
    fn field_framing_distinguishes_ambiguous_concatenation() -> Result<(), DigestError> {
        let mut first = CanonicalHasher::new("agentro.test")?;
        first.write_field("a", b"bc")?;
        first.write_field("d", b"")?;

        let mut second = CanonicalHasher::new("agentro.test")?;
        second.write_field("a", b"b")?;
        second.write_field("c", b"d")?;

        assert_ne!(first.finish(), second.finish());
        Ok(())
    }

    #[test]
    fn canonical_encoding_has_cross_platform_golden_digest() -> Result<(), DigestError> {
        let mut hasher = CanonicalHasher::new("agentro.test")?;
        hasher.write_field("payload", b"hello")?;

        assert_eq!(
            hasher.finish().to_string(),
            "sha256:a21d5bb2ec23aea1f86805b9754deca294d3cda55dd83a0a259fc45b784d9b65"
        );
        Ok(())
    }

    #[test]
    fn fields_must_be_strictly_sorted() -> Result<(), DigestError> {
        let mut hasher = CanonicalHasher::new("agentro.test")?;
        hasher.write_field("second", b"2")?;

        assert_eq!(
            hasher.write_field("first", b"1"),
            Err(DigestError::FieldsOutOfOrder)
        );
        assert_eq!(
            hasher.write_field("second", b"2"),
            Err(DigestError::FieldsOutOfOrder)
        );
        Ok(())
    }
}
