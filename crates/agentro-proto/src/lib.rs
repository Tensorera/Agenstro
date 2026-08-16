//! Generated Protobuf messages and Tonic service bindings for Agentro API v1.
//!
//! The source of truth is the workspace root `proto/` tree. These wire types
//! are intentionally separate from `agentro-contracts` domain values.

#![allow(clippy::doc_markdown)]

include!(concat!(env!("OUT_DIR"), "/agentro_modules.rs"));

/// Encoded `google.protobuf.FileDescriptorSet` for the complete API v1 source.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/agentro_descriptor.bin"));
