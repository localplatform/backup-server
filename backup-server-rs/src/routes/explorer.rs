use crate::error::AppError;
use crate::models::server;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ExploreQuery {
    pub path: Option<String>,
}

pub async fn explore(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ExploreQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let remote_path = query.path.unwrap_or_else(|| "/".into());

    // Verify server exists
    let db = state.db.clone();
    let id2 = id.clone();
    let srv = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        server::find_by_id(&conn, &id2)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    if srv.is_none() {
        return Err(AppError::NotFound("Server not found".into()));
    }

    if !state.agents.is_connected(&id) {
        return Err(AppError::ServiceUnavailable("Agent is not connected".into()));
    }

    let message = serde_json::json!({
        "type": "fs:browse",
        "payload": { "path": remote_path },
    });

    let result = state
        .agents
        .request_from_agent(&id, message, 30_000)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("Permission denied") || msg.contains("EACCES") {
                AppError::BadRequest(format!("Permission denied: {}", remote_path))
            } else if msg.contains("No such file") || msg.contains("not found") {
                AppError::NotFound(format!("Path not found: {}", remote_path))
            } else {
                AppError::Internal(e)
            }
        })?;

    if let Some(error) = result.get("error") {
        return Err(AppError::Internal(anyhow::anyhow!("{}", error)));
    }

    let entries = result.get("entries").cloned().unwrap_or(serde_json::json!([]));
    Ok(Json(entries))
}
