use crate::error::AppError;
use crate::models::backup_version;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_versions))
        .route("/{id}", get(get_version).delete(delete_version))
        .route("/by-job/{job_id}", delete(delete_by_job))
        .route("/by-server/{server_id}", delete(delete_by_server))
}

#[derive(Deserialize)]
pub struct VersionsQuery {
    pub job_id: Option<String>,
}

async fn list_versions(
    State(state): State<Arc<AppState>>,
    Query(query): Query<VersionsQuery>,
) -> Result<Json<Vec<backup_version::BackupVersion>>, AppError> {
    let db = state.db.clone();
    let versions = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        match query.job_id {
            Some(job_id) => backup_version::find_by_job_id(&conn, &job_id),
            None => backup_version::find_all(&conn),
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    Ok(Json(versions))
}

async fn get_version(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<backup_version::BackupVersion>, AppError> {
    let db = state.db.clone();
    let version = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_version::find_by_id(&conn, &id)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    match version {
        Some(v) => Ok(Json(v)),
        None => Err(AppError::NotFound("Version not found".into())),
    }
}

async fn delete_version(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    let db = state.db.clone();
    let id2 = id.clone();

    // Get version path for filesystem cleanup before deleting from DB
    let version = tokio::task::spawn_blocking({
        let db = db.clone();
        let id = id.clone();
        move || {
            let conn = db.get()?;
            backup_version::find_by_id(&conn, &id)
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let job_id = version.as_ref().map(|v| v.job_id.clone());
    let local_path = version.as_ref().map(|v| v.local_path.clone());

    let deleted = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_version::delete(&conn, &id2)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    if deleted {
        // Async filesystem cleanup
        if let Some(path) = local_path {
            tokio::spawn(async move {
                if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                    tracing::warn!("Failed to remove version directory {}: {}", path, e);
                }
            });
        }
        if let Some(job_id) = job_id {
            state.ui.broadcast("version:deleted", serde_json::json!({
                "versionId": id,
                "jobId": job_id,
            }));
        }
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("Version not found".into()))
    }
}

async fn delete_by_job(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = state.db.clone();
    let jid = job_id.clone();

    // Get paths before deleting
    let versions = tokio::task::spawn_blocking({
        let db = db.clone();
        let jid = job_id.clone();
        move || {
            let conn = db.get()?;
            backup_version::find_by_job_id(&conn, &jid)
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let paths: Vec<String> = versions.iter().map(|v| v.local_path.clone()).collect();

    let count = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_version::delete_by_job_id(&conn, &jid)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    // Async cleanup
    for path in paths {
        tokio::spawn(async move {
            let _ = tokio::fs::remove_dir_all(&path).await;
        });
    }

    state.ui.broadcast("version:bulk-deleted", serde_json::json!({
        "jobId": job_id,
        "deletedCount": count,
    }));

    Ok(Json(serde_json::json!({ "deleted": count, "kept": 0 })))
}

async fn delete_by_server(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = state.db.clone();
    let sid = server_id.clone();

    let count = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_version::delete_by_server_id(&conn, &sid)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    Ok(Json(serde_json::json!({ "deleted": count, "kept": 0 })))
}
