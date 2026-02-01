//! Backup job management endpoints.

use axum::{Json, http::StatusCode, extract::State};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct StartBackupRequest {
    pub job_id: String,
    pub paths: Vec<String>,
    pub server_url: String,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartBackupResponse {
    pub status: String,
    pub pid: u32,
}

#[derive(Debug, Deserialize)]
pub struct CancelBackupRequest {
    pub job_id: String,
}

#[derive(Debug, Serialize)]
pub struct CancelBackupResponse {
    pub status: String,
}

/// POST /backup/start - Start a backup job
pub async fn start_backup(
    State(app_state): State<super::AppState>,
    Json(req): Json<StartBackupRequest>,
) -> Result<Json<StartBackupResponse>, StatusCode> {
    tracing::info!("Starting backup job: {} for paths: {:?}", req.job_id, req.paths);

    // Convert paths to PathBuf
    let paths: Vec<PathBuf> = req.paths.iter().map(PathBuf::from).collect();

    // Create temporary destination (TODO: get from server or config)
    let destination = PathBuf::from(format!("/tmp/backup-{}", req.job_id));

    // Create backup job
    let job = crate::executor::BackupJob {
        job_id: req.job_id.clone(),
        paths,
        destination,
        server_url: req.server_url,
    };

    // Create executor
    let mut executor = crate::executor::BackupExecutor::new(app_state.ws_state.clone());

    let job_id = req.job_id.clone();
    let tracker = app_state.job_tracker.clone();

    // Spawn backup task in background and track it
    let handle = tokio::spawn(async move {
        match executor.execute(job).await {
            Ok(result) => {
                tracing::info!(
                    "Backup completed successfully: {} files, {} bytes, {} seconds",
                    result.total_files,
                    result.total_bytes,
                    result.duration_secs
                );
                // Remove from tracker when completed
                tracker.complete(&job_id).await;
            }
            Err(e) => {
                tracing::error!("Backup failed: {}", e);
                // Remove from tracker when failed
                tracker.complete(&job_id).await;
            }
        }
    });

    // Register the job with its abort handle
    app_state.job_tracker.register(req.job_id.clone(), handle.abort_handle()).await;

    Ok(Json(StartBackupResponse {
        status: "started".to_string(),
        pid: std::process::id(),
    }))
}

/// POST /backup/cancel - Cancel a backup job
pub async fn cancel_backup(
    State(app_state): State<super::AppState>,
    Json(req): Json<CancelBackupRequest>,
) -> Result<Json<CancelBackupResponse>, StatusCode> {
    tracing::info!("Cancelling backup job: {}", req.job_id);

    let cancelled = app_state.job_tracker.cancel(&req.job_id).await;

    if cancelled {
        tracing::info!("Successfully cancelled backup job: {}", req.job_id);
        Ok(Json(CancelBackupResponse {
            status: "cancelled".to_string(),
        }))
    } else {
        tracing::warn!("Job not found or already completed: {}", req.job_id);
        Err(StatusCode::NOT_FOUND)
    }
}
