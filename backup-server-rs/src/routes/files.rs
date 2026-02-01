use crate::error::AppError;
use crate::models::backup_version;
use crate::state::AppState;
use axum::extract::{Path as AxumPath, Request, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use futures_util::StreamExt;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

pub fn router(_state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/upload", post(upload_file))
        .route("/manifest/{job_id}", get(get_manifest))
        .route("/hardlink", post(create_hardlinks))
}

async fn upload_file(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Request,
) -> Result<Json<serde_json::Value>, AppError> {
    let job_id = headers
        .get("x-job-id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing x-job-id header".into()))?
        .to_string();

    let relative_path = headers
        .get("x-relative-path")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("Missing x-relative-path header".into()))?
        .to_string();

    let total_size: u64 = headers
        .get("x-total-size")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| AppError::BadRequest("Missing or invalid x-total-size header".into()))?;

    let content_encoding = headers
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    tracing::debug!(job_id = %job_id, relative_path = %relative_path, total_size, "Receiving file upload");

    // Determine destination path
    let db = state.db.clone();
    let jid = job_id.clone();
    let backups_dir = state.config.backups_dir.clone();
    let base_dir = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        let versions = backup_version::find_by_job_id(&conn, &jid)?;
        let running = versions.into_iter().find(|v| v.status == "running");
        Ok::<_, anyhow::Error>(match running {
            Some(v) => PathBuf::from(v.local_path),
            None => backups_dir.join(&jid),
        })
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let dest_path = base_dir.join(&relative_path);
    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent).await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to create directory: {}", e)))?;
    }

    // Stream body to file
    let body = request.into_body();
    let body_stream = body.into_data_stream();

    if content_encoding.as_deref() == Some("zstd") {
        // Collect body, decompress, then write
        let mut compressed = Vec::new();
        let mut stream = body_stream;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| AppError::Internal(anyhow::anyhow!("Read error: {}", e)))?;
            compressed.extend_from_slice(&chunk);
        }
        let decompressed = tokio::task::spawn_blocking(move || {
            zstd::decode_all(compressed.as_slice())
        })
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Zstd decompression failed: {}", e)))?;

        tokio::fs::write(&dest_path, &decompressed).await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Write error: {}", e)))?;
    } else {
        // Stream directly to file
        let mut file = tokio::fs::File::create(&dest_path).await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Create file error: {}", e)))?;
        let mut stream = body_stream;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| AppError::Internal(anyhow::anyhow!("Read error: {}", e)))?;
            file.write_all(&chunk).await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("Write error: {}", e)))?;
        }
        file.flush().await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Flush error: {}", e)))?;
    }

    // Verify file size
    let metadata = tokio::fs::metadata(&dest_path).await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Stat error: {}", e)))?;

    if metadata.len() != total_size {
        tracing::warn!(
            job_id = %job_id,
            relative_path = %relative_path,
            expected = total_size,
            actual = metadata.len(),
            "File size mismatch after upload"
        );
        return Err(AppError::BadRequest(format!(
            "File size mismatch: expected {} got {}",
            total_size,
            metadata.len()
        )));
    }

    tracing::debug!(job_id = %job_id, relative_path = %relative_path, size = metadata.len(), "File upload complete");

    Ok(Json(serde_json::json!({
        "success": true,
        "path": relative_path,
        "size": metadata.len(),
    })))
}

/// Returns the manifest JSON from the latest completed version for a given job.
/// The agent fetches this to determine which files have changed for incremental backups.
async fn get_manifest(
    State(state): State<Arc<AppState>>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = state.db.clone();
    let jid = job_id.clone();

    let prev = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_version::find_latest_completed(&conn, &jid)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let prev = prev.ok_or_else(|| AppError::NotFound("No completed version found".into()))?;
    let manifest_path = PathBuf::from(&prev.local_path).join(".backup-manifest.json");

    let content = tokio::fs::read_to_string(&manifest_path).await
        .map_err(|_| AppError::NotFound("Manifest not found for latest version".into()))?;

    let manifest: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Invalid manifest JSON: {}", e)))?;

    Ok(Json(manifest))
}

#[derive(Deserialize)]
struct HardlinkRequest {
    job_id: String,
    files: Vec<String>,
}

/// Creates hardlinks from the previous completed version to the current running version
/// for files that haven't changed. This avoids re-transferring unchanged files while
/// keeping each version as a complete, browsable snapshot.
async fn create_hardlinks(
    State(state): State<Arc<AppState>>,
    Json(body): Json<HardlinkRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = state.db.clone();
    let jid = body.job_id.clone();

    // Find current running version and previous completed version
    let (current_path, previous_path) = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        let versions = backup_version::find_by_job_id(&conn, &jid)?;
        let running = versions.iter()
            .find(|v| v.status == "running")
            .map(|v| v.local_path.clone());
        let completed = versions.iter()
            .find(|v| v.status == "completed")
            .map(|v| v.local_path.clone());
        Ok::<_, anyhow::Error>((running, completed))
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let current = PathBuf::from(
        current_path.ok_or_else(|| AppError::BadRequest("No running version found".into()))?
    );
    let previous = PathBuf::from(
        previous_path.ok_or_else(|| AppError::BadRequest("No previous completed version".into()))?
    );

    let files = body.files;
    let (linked, failed) = tokio::task::spawn_blocking(move || {
        let mut linked = 0u64;
        let mut failed = 0u64;

        for rel_path in &files {
            let src = previous.join(rel_path);
            let dst = current.join(rel_path);

            if !src.exists() {
                tracing::warn!(path = %rel_path, "Hardlink source does not exist");
                failed += 1;
                continue;
            }

            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            match std::fs::hard_link(&src, &dst) {
                Ok(_) => linked += 1,
                Err(e) => {
                    tracing::warn!(path = %rel_path, error = %e, "Hardlink failed");
                    failed += 1;
                }
            }
        }

        (linked, failed)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))?;

    tracing::info!(
        job_id = %body.job_id,
        linked,
        failed,
        "Hardlink creation completed"
    );

    Ok(Json(serde_json::json!({
        "linked": linked,
        "failed": failed,
    })))
}
