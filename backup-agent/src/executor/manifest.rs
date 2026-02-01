//! Manifest types for incremental backup support.
//!
//! A manifest records every file in a backup version with its size and mtime,
//! allowing the agent to diff against it and only transfer changed files.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Backup manifest — serialized as `.backup-manifest.json` in each version directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub job_id: String,
    pub files: HashMap<String, ManifestEntry>,
    pub total_files: usize,
    pub total_bytes: u64,
}

/// Metadata for a single file in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub size: u64,
    pub mtime: i64,
}
