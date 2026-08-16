use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::ObjectDigest;

/// Hard maximum entries materialized by one in-memory manifest build.
pub const MAX_MANIFEST_ENTRIES: usize = 100_000;
/// Hard maximum bytes in one canonical manifest encoding.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1_048_576;
/// Hard maximum UTF-8 bytes in one portable manifest path.
pub const MAX_MANIFEST_PATH_BYTES: usize = 4_096;
const MAX_MANIFEST_COMPONENT_BYTES: usize = 255;
const MANIFEST_MAGIC: &[u8] = b"agentro.cas.manifest\0";
const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Invalid path, manifest budget, duplicate, or canonical encoding.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum ManifestError {
    /// A portable path was empty, absolute, traversing, or otherwise unsafe.
    #[error("manifest path is not a strict portable project-relative path")]
    InvalidPath,
    /// A path exceeded the component or total byte hard limit.
    #[error("manifest path exceeds a hard byte limit")]
    PathTooLong,
    /// A manifest budget was zero or above its hard maximum.
    #[error("invalid manifest budget: {field}")]
    InvalidBudget {
        /// Name of the invalid budget field.
        field: &'static str,
    },
    /// The caller supplied more entries than its budget.
    #[error("manifest entry budget {maximum} exceeded")]
    EntryBudgetExceeded {
        /// Declared maximum entries.
        maximum: usize,
    },
    /// Canonical encoding exceeded the caller byte budget.
    #[error("manifest encoded byte budget {maximum} exceeded")]
    EncodedBudgetExceeded {
        /// Declared maximum encoded bytes.
        maximum: u64,
    },
    /// Two entries used the same exact path.
    #[error("manifest contains duplicate path {path}")]
    DuplicatePath {
        /// Conflicting path.
        path: String,
    },
    /// Two entries differ only by portable ASCII case.
    #[error("manifest contains a portable case collision at {path}")]
    PortableCaseCollision {
        /// Conflicting path.
        path: String,
    },
    /// An executable bit was attached to a non-file entry.
    #[error("only regular-file manifest entries can be executable")]
    InvalidEntry,
}

/// A strict slash-separated UTF-8 project-relative manifest path.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ManifestPath(String);

impl ManifestPath {
    /// Parses a strict portable path suitable for canonical cross-platform data.
    ///
    /// # Errors
    ///
    /// Rejects absolute paths, empty/`.`/`..` components, backslashes, colons,
    /// controls, Windows reserved names, trailing dots/spaces, and byte excess.
    pub fn parse(value: impl Into<String>) -> Result<Self, ManifestError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_MANIFEST_PATH_BYTES {
            return Err(if value.len() > MAX_MANIFEST_PATH_BYTES {
                ManifestError::PathTooLong
            } else {
                ManifestError::InvalidPath
            });
        }
        if value.starts_with('/') || value.ends_with('/') || value.contains(['\\', ':', '\0']) {
            return Err(ManifestError::InvalidPath);
        }
        for component in value.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Err(ManifestError::InvalidPath);
            }
            if component.len() > MAX_MANIFEST_COMPONENT_BYTES {
                return Err(ManifestError::PathTooLong);
            }
            if component.chars().any(char::is_control)
                || component.ends_with(['.', ' '])
                || is_windows_reserved(component)
            {
                return Err(ManifestError::InvalidPath);
            }
        }
        Ok(Self(value))
    }

    /// Returns the canonical slash-separated string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_windows_reserved(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Kind of immutable object referenced by a manifest entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ManifestEntryKind {
    /// Regular file content.
    File,
    /// Canonical child-directory manifest.
    Directory,
    /// Stored symbolic-link target bytes.
    Symlink,
}

impl ManifestEntryKind {
    fn code(self) -> u8 {
        match self {
            Self::File => 1,
            Self::Directory => 2,
            Self::Symlink => 3,
        }
    }
}

/// One immutable manifest edge to a content-addressed object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestEntry {
    path: ManifestPath,
    kind: ManifestEntryKind,
    digest: ObjectDigest,
    length: u64,
    is_executable: bool,
}

impl ManifestEntry {
    /// Constructs a manifest entry from validated values.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::InvalidEntry`] when a non-file is executable.
    pub fn new(
        path: ManifestPath,
        kind: ManifestEntryKind,
        digest: ObjectDigest,
        length: u64,
        is_executable: bool,
    ) -> Result<Self, ManifestError> {
        if is_executable && kind != ManifestEntryKind::File {
            return Err(ManifestError::InvalidEntry);
        }
        Ok(Self {
            path,
            kind,
            digest,
            length,
            is_executable,
        })
    }

    /// Returns the entry's project-relative path.
    #[must_use]
    pub fn path(&self) -> &ManifestPath {
        &self.path
    }
}

/// Caller-selected bounds under the manifest hard maxima.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestBudget {
    max_entries: usize,
    max_encoded_bytes: u64,
}

impl ManifestBudget {
    /// Constructs a validated in-memory manifest budget.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::InvalidBudget`] for zero or excessive values.
    pub fn new(max_entries: usize, max_encoded_bytes: u64) -> Result<Self, ManifestError> {
        if max_entries == 0 || max_entries > MAX_MANIFEST_ENTRIES {
            return Err(ManifestError::InvalidBudget {
                field: "entry count",
            });
        }
        if max_encoded_bytes == 0 || max_encoded_bytes > MAX_MANIFEST_BYTES {
            return Err(ManifestError::InvalidBudget {
                field: "encoded bytes",
            });
        }
        Ok(Self {
            max_entries,
            max_encoded_bytes,
        })
    }
}

/// Sorted entries plus their versioned canonical byte representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Manifest {
    entries: Vec<ManifestEntry>,
    canonical: Vec<u8>,
    digest: ObjectDigest,
}

impl Manifest {
    /// Sorts, validates, and canonically encodes a bounded manifest.
    ///
    /// # Errors
    ///
    /// Returns typed entry, collision, or encoded-byte budget failures.
    pub fn build(
        entries: impl IntoIterator<Item = ManifestEntry>,
        budget: ManifestBudget,
    ) -> Result<Self, ManifestError> {
        let mut sorted = BTreeMap::new();
        let mut portable_names = BTreeSet::new();
        for entry in entries {
            if sorted.len() >= budget.max_entries {
                return Err(ManifestError::EntryBudgetExceeded {
                    maximum: budget.max_entries,
                });
            }
            let path = entry.path.as_str().to_owned();
            if sorted.contains_key(&path) {
                return Err(ManifestError::DuplicatePath { path });
            }
            if !portable_names.insert(path.to_ascii_lowercase()) {
                return Err(ManifestError::PortableCaseCollision { path });
            }
            sorted.insert(path, entry);
        }
        let entries: Vec<_> = sorted.into_values().collect();
        let canonical = encode(&entries, budget.max_encoded_bytes)?;
        let digest_bytes: [u8; 32] = Sha256::digest(&canonical).into();
        Ok(Self {
            entries,
            canonical,
            digest: ObjectDigest::from_bytes(digest_bytes),
        })
    }

    /// Returns entries in canonical path order.
    #[must_use]
    pub fn entries(&self) -> &[ManifestEntry] {
        &self.entries
    }

    /// Returns the versioned canonical bytes used for hashing.
    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical
    }

    /// Returns the SHA-256 digest of the canonical representation.
    #[must_use]
    pub fn digest(&self) -> ObjectDigest {
        self.digest
    }
}

fn encode(entries: &[ManifestEntry], maximum: u64) -> Result<Vec<u8>, ManifestError> {
    let mut encoded = Vec::new();
    append(&mut encoded, MANIFEST_MAGIC, maximum)?;
    append(
        &mut encoded,
        &MANIFEST_SCHEMA_VERSION.to_be_bytes(),
        maximum,
    )?;
    append(
        &mut encoded,
        &u64::try_from(entries.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
        maximum,
    )?;
    for entry in entries {
        let path = entry.path.as_str().as_bytes();
        let path_length = u32::try_from(path.len()).map_err(|_| ManifestError::PathTooLong)?;
        append(&mut encoded, &path_length.to_be_bytes(), maximum)?;
        append(&mut encoded, path, maximum)?;
        append(
            &mut encoded,
            &[entry.kind.code(), u8::from(entry.is_executable)],
            maximum,
        )?;
        append(&mut encoded, &entry.length.to_be_bytes(), maximum)?;
        append(&mut encoded, entry.digest.as_bytes(), maximum)?;
    }
    Ok(encoded)
}

fn append(target: &mut Vec<u8>, value: &[u8], maximum: u64) -> Result<(), ManifestError> {
    let next = u64::try_from(target.len())
        .unwrap_or(u64::MAX)
        .checked_add(u64::try_from(value.len()).unwrap_or(u64::MAX))
        .ok_or(ManifestError::EncodedBudgetExceeded { maximum })?;
    if next > maximum {
        return Err(ManifestError::EncodedBudgetExceeded { maximum });
    }
    target.extend_from_slice(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    fn digest(byte: u8) -> ObjectDigest {
        ObjectDigest::from_bytes([byte; 32])
    }

    fn entry(path: &str, byte: u8) -> Result<ManifestEntry, ManifestError> {
        ManifestEntry::new(
            ManifestPath::parse(path)?,
            ManifestEntryKind::File,
            digest(byte),
            1,
            false,
        )
    }

    #[test]
    fn input_order_does_not_change_manifest_digest() -> Result<(), Box<dyn Error>> {
        let budget = ManifestBudget::new(10, 4_096)?;
        let first = Manifest::build([entry("z.txt", 2)?, entry("a.txt", 1)?], budget)?;
        let second = Manifest::build([entry("a.txt", 1)?, entry("z.txt", 2)?], budget)?;
        assert_eq!(first.digest(), second.digest());
        assert_eq!(first.canonical_bytes(), second.canonical_bytes());
        assert_eq!(first.entries()[0].path().as_str(), "a.txt");
        Ok(())
    }

    #[test]
    fn content_change_changes_manifest_digest() -> Result<(), Box<dyn Error>> {
        let budget = ManifestBudget::new(10, 4_096)?;
        let first = Manifest::build([entry("a.txt", 1)?], budget)?;
        let second = Manifest::build([entry("a.txt", 2)?], budget)?;
        assert_ne!(first.digest(), second.digest());
        Ok(())
    }

    #[test]
    fn unsafe_and_case_colliding_paths_are_rejected() -> Result<(), Box<dyn Error>> {
        assert!(matches!(
            ManifestPath::parse("../escape"),
            Err(ManifestError::InvalidPath)
        ));
        let result = Manifest::build(
            [entry("File.txt", 1)?, entry("file.txt", 2)?],
            ManifestBudget::new(10, 4_096)?,
        );
        assert!(matches!(
            result,
            Err(ManifestError::PortableCaseCollision { .. })
        ));
        Ok(())
    }

    #[test]
    fn encoded_bytes_are_hard_bounded() -> Result<(), Box<dyn Error>> {
        let result = Manifest::build([entry("a.txt", 1)?], ManifestBudget::new(10, 8)?);
        assert!(matches!(
            result,
            Err(ManifestError::EncodedBudgetExceeded { maximum: 8 })
        ));
        Ok(())
    }
}
