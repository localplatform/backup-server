use crate::error::AppError;
use crate::models::server;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_servers).post(create_server))
        .route("/ping-status", get(get_ping_status))
        .route("/{id}", get(get_server).put(update_server).delete(delete_server))
        .route("/{id}/explore", get(crate::routes::explorer::explore))
}

async fn list_servers(State(state): State<Arc<AppState>>) -> Result<Json<Vec<server::Server>>, AppError> {
    let db = state.db.clone();
    let servers = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        server::find_all(&conn)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    Ok(Json(servers))
}

async fn get_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<server::Server>, AppError> {
    let db = state.db.clone();
    let srv = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        server::find_by_id(&conn, &id)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    match srv {
        Some(s) => Ok(Json(s)),
        None => Err(AppError::NotFound("Server not found".into())),
    }
}

async fn create_server(
    State(state): State<Arc<AppState>>,
    Json(body): Json<server::CreateServerRequest>,
) -> Result<(axum::http::StatusCode, Json<server::Server>), AppError> {
    if body.name.is_empty() || body.hostname.is_empty() {
        return Err(AppError::BadRequest("name and hostname are required".into()));
    }

    let password = body.password.clone();
    let db = state.db.clone();
    let srv = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        server::create(&conn, &body)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    // TODO: Phase 7 - trigger agent deployment with password
    state.ui.broadcast("server:updated", serde_json::json!({ "server": srv }));

    Ok((axum::http::StatusCode::CREATED, Json(srv)))
}

async fn update_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<server::UpdateServerRequest>,
) -> Result<Json<server::Server>, AppError> {
    let db = state.db.clone();
    let id2 = id.clone();
    let srv = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        server::update(&conn, &id2, &body)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    match srv {
        Some(s) => {
            state.ui.broadcast("server:updated", serde_json::json!({ "server": s }));
            Ok(Json(s))
        }
        None => Err(AppError::NotFound("Server not found".into())),
    }
}

async fn delete_server(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    let db = state.db.clone();
    let id2 = id.clone();
    let deleted = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        server::delete(&conn, &id2)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    if deleted {
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound("Server not found".into()))
    }
}

async fn get_ping_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Return agent connection status for all servers
    let db = state.db.clone();
    let agents = state.agents.clone();
    let statuses = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        let servers = server::find_all(&conn)?;
        let mut result = Vec::new();
        for s in servers {
            let connected = agents.is_connected(&s.id);
            result.push(serde_json::json!({
                "serverId": s.id,
                "reachable": connected,
                "latencyMs": null,
                "lastCheckedAt": chrono::Utc::now().to_rfc3339(),
            }));
        }
        Ok::<_, anyhow::Error>(result)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    Ok(Json(statuses))
}
