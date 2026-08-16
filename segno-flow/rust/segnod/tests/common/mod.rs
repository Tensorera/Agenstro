#![allow(dead_code)]

use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

use serde_json::json;
use tempfile::TempDir;
use zip::{ZipWriter, write::SimpleFileOptions};

pub enum ExtraEntry {
    File(String, Vec<u8>),
    Symlink(String, String),
}

pub fn test_directory() -> Result<TempDir, Box<dyn std::error::Error>> {
    Ok(tempfile::Builder::new()
        .prefix("segnod-test-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))?)
}

pub fn manifest(overlap: &str, misfire: &str) -> Vec<u8> {
    let overlap = match overlap {
        "allow" => json!({"kind": "allow_with_limit", "limit": 10}),
        "queue" => json!({"kind": "queue_one"}),
        _ => json!({"kind": "forbid"}),
    };
    let misfire = match misfire {
        "catch_up" => json!({"kind": "bounded_catch_up", "limit": 3}),
        "skip" => json!({"kind": "skip", "grace_seconds": 30}),
        _ => json!({"kind": "coalesce"}),
    };
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "id": "test-task",
        "name": "Test task",
        "schedule": {
            "cron_dialect": "unix5",
            "cron": "* * * * *",
            "timezone": "UTC",
            "dst_gap": "skip",
            "dst_fold": "both",
            "misfire": misfire,
            "overlap": overlap,
            "retry": {"kind": "none"},
            "jitter_seconds": 0
        },
        "scripts": {
            "pre": "scripts/pre.py",
            "main": "scripts/main.py",
            "post": "scripts/post.py"
        }
    }))
    .unwrap_or_default()
}

pub fn write_package(
    path: &Path,
    manifest_bytes: &[u8],
    extras: Vec<ExtraEntry>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let file = File::create(path)?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("segno-flow.json", options)?;
    writer.write_all(manifest_bytes)?;
    for stage in ["pre", "main", "post"] {
        writer.start_file(format!("scripts/{stage}.py"), options)?;
        writer.write_all(b"raise RuntimeError('must never execute in Segno')\n")?;
    }
    for extra in extras {
        match extra {
            ExtraEntry::File(name, bytes) => {
                writer.start_file(name, options)?;
                writer.write_all(&bytes)?;
            }
            ExtraEntry::Symlink(name, target) => {
                writer.add_symlink(name, target, options)?;
            }
        }
    }
    writer.finish()?.sync_all()?;
    Ok(path.to_path_buf())
}
