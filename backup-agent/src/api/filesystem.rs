//! Filesystem browsing endpoint.
//!
//! Provides directory listing for the remote file explorer UI,
//! replacing SSH-based file browsing.

use axum::{extract::Query, Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::error;

#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub size: u64,
}

#[derive(Debug, Serialize)]
pub struct BrowseResponse {
    pub entries: Vec<FsEntry>,
}

/// GET /fs/browse?path=/ - Browse local filesystem
pub async fn browse(
    Query(query): Query<BrowseQuery>,
) -> Result<Json<BrowseResponse>, StatusCode> {
    match browse_path(&query.path) {
        Ok(entries) => Ok(Json(BrowseResponse { entries })),
        Err(e) => {
            error!("Failed to browse path {}: {}", query.path, e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Browse a directory and return its entries.
/// Used both by the HTTP endpoint and the WebSocket command handler.
pub fn browse_path(dir: &str) -> std::io::Result<Vec<FsEntry>> {
    let path = Path::new(dir);
    let mut entries = Vec::new();

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let name = entry.file_name().to_string_lossy().to_string();
        let full_path = entry.path().to_string_lossy().to_string();

        let entry_type = if metadata.is_dir() {
            "directory"
        } else if metadata.is_symlink() {
            "symlink"
        } else {
            "file"
        };

        entries.push(FsEntry {
            name,
            path: full_path,
            entry_type: entry_type.to_string(),
            size: if metadata.is_file() { metadata.len() } else { 0 },
        });
    }

    // Sort: directories first, then files, alphabetically
    entries.sort_by(|a, b| {
        let a_is_dir = a.entry_type == "directory";
        let b_is_dir = b.entry_type == "directory";
        b_is_dir.cmp(&a_is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browse_root() {
        let entries = browse_path("/tmp").unwrap();
        // /tmp should exist and be browsable
        assert!(entries.iter().all(|e| !e.name.is_empty()));
    }

    #[test]
    fn test_browse_nonexistent() {
        let result = browse_path("/nonexistent_path_12345");
        assert!(result.is_err());
    }
}
