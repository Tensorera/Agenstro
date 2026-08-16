//! Tactus-specific SQLite schema, storage values, codecs, and writer owner.
//!
//! This module deliberately has no dependency on `tactus-core`. It owns the
//! immutable Tactus v1 migration and the persistence-facing representation
//! needed by later repository method groups.

/// Closed codecs for values crossing the SQLite boundary.
pub mod codec;
/// Immutable Tactus v1 schema and migration definition.
pub mod migration;
/// Persistence-facing keys, enums, and data-transfer records.
pub mod model;
/// Single-writer Tactus owner and narrow typed handle.
pub mod repository;
