use std::fmt;

/// A SHA-256 digest with stable lowercase hexadecimal display.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectDigest([u8; 32]);

impl ObjectDigest {
    /// Constructs a digest from exactly 32 SHA-256 bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrows the raw 32-byte digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Encodes the digest as 64 lowercase hexadecimal characters.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut encoded = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write;
            let _ = write!(encoded, "{byte:02x}");
        }
        encoded
    }
}

impl fmt::Debug for ObjectDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ObjectDigest")
            .field(&self.to_hex())
            .finish()
    }
}

impl fmt::Display for ObjectDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}
