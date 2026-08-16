//! Streaming SHA-256 content-addressed storage and canonical manifests.
//!
//! Object writers stream into a same-filesystem temporary file, enforce a hard
//! byte budget, synchronize file contents, and publish without replacing an
//! immutable object. Manifests use a versioned, deterministic binary encoding;
//! ordinary JSON or Protobuf serialization is never treated as canonical.

#![deny(missing_docs)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod digest;
mod manifest;
mod object;

pub use digest::ObjectDigest;
pub use manifest::{
    MAX_MANIFEST_BYTES, MAX_MANIFEST_ENTRIES, MAX_MANIFEST_PATH_BYTES, Manifest, ManifestBudget,
    ManifestEntry, ManifestEntryKind, ManifestError, ManifestPath,
};
pub use object::{
    Cas, CasError, MAX_OBJECT_BYTES, ObjectPut, PublishDurability, PublishOutcome, PutBudget,
};
