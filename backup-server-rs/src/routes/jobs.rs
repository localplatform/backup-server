use crate::error::AppError;
use crate::models::backup_job;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_jobs).post(create_job))
        .route("/{id}", get(get_job).put(update_job).delete(delete_job))
        .route("/{id}/run", post(run_job))
        .route("/{id}/cancel", post(cancel_job))
        .route("/{id}/logs", get(get_job_logs))
}

async fn list_jobs(State(state): State<Arc<AppState>>) -> Result<Json<Vec<backup_job::BackupJob>>, AppError> {
    let db = state.db.clone();
    let jobs = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_job::find_all(&conn)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    Ok(Json(jobs))
}

async fn get_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<backup_job::BackupJob>, AppError> {
    let db = state.db.clone();
    let job = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_job::find_by_id(&conn, &id)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    match job {
        Some(j) => Ok(Json(j)),
        None => Err(AppError::NotFound("Job not found".into())),
    }
}

async fn create_job(
    State(state): State<Arc<AppState>>,
    Json(body): Json<backup_job::CreateBackupJobRequest>,
) -> Result<(axum::http::StatusCode, Json<backup_job::BackupJob>), AppError> {
    if body.name.is_empty() {
        return Err(AppError::BadRequest("name is required".into()));
    }
    if body.remote_paths.is_empty() {
        return Err(AppError::BadRequest("remote_paths must not be empty".into()));
    }

    let db = state.db.clone();
    let ui = state.ui.clone();
    let job = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_job::create(&conn, &body)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    ui.broadcast("job:created", serde_json::json!({ "jobId": job.id }));

    // TODO: Phase 8 - schedule cron if cron_schedule is set

    Ok((axum::http::StatusCode::CREATED, Json(job)))
}

async fn update_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<backup_job::UpdateBackupJobRequest>,
) -> Result<Json<backup_job::BackupJob>, AppError> {
    let db = state.db.clone();
    let id2 = id.clone();
    let job = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_job::update(&conn, &id2, &body)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    match job {
        Some(j) => {
            state.ui.broadcast("job:updated", serde_json::json!({ "jobId": j.id }));
            // TODO: Phase 8 - reschedule cron
            Ok(Json(j))
        }
        None => Err(AppError::NotFound("Job not found".into())),
    }
}

async fn delete_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    // TODO: Phase 6 - cancel if running
    let db = state.db.clone();
    let id2 = id.clone();
    let deleted = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_job::delete(&conn, &id2)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    if deleted {
        state.ui.broadcast("job:deleted", serde_json::json!({ "jobId": id }));
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("Job not found".into()))
    }
}

#[derive(Deserialize)]
pub struct RunJobQuery {
    pub full: Option<bool>,
}

async fn run_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<RunJobQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let full = query.full.unwrap_or(false);
    let state2 = state.clone();
    let id2 = id.clone();

    // Spawn the backup as a background task
    tokio::spawn(async move {
        if let Err(e) = crate::services::agent_orchestrator::run_backup_job(state2, id2, full).await {
            tracing::error!("Backup job failed: {:#}", e);
        }
    });

    Ok(Json(serde_json::json!({ "started": true })))
}

async fn cancel_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    crate::services::agent_orchestrator::cancel_backup_job(state, &id)
        .await
        .map_err(|e| AppError::Internal(e))?;
    Ok(Json(serde_json::json!({ "cancelled": true })))
}

#[derive(Deserialize)]
pub struct LogsQuery {
    pub limit: Option<i64>,
}

async fn get_job_logs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<Vec<backup_job::BackupLog>>, AppError> {
    let limit = query.limit.unwrap_or(50);
    let db = state.db.clone();
    let logs = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_job::find_logs_by_job_id(&conn, &id, limit)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    Ok(Json(logs))
}
