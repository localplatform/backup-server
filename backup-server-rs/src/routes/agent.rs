use crate::error::AppError;
use crate::models::server;
use crate::services::agent_deployer;
use crate::state::AppState;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::sync::Arc;
use tokio_util::io::ReaderStream;

pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/deploy", post(deploy_agent))
        .route("/binary", get(get_binary))
        .route("/update/{server_id}", post(update_agent))
        .route("/status/{server_id}", get(get_status))
}

async fn deploy_agent(
    State(state): State<Arc<AppState>>,
    Json(body): Json<server::CreateServerRequest>,
) -> Result<(axum::http::StatusCode, Json<server::Server>), AppError> {
    if body.name.is_empty() || body.hostname.is_empty() {
        return Err(AppError::BadRequest("name and hostname are required".into()));
    }

    let password = body.password.clone().unwrap_or_default();
    if password.is_empty() {
        return Err(AppError::BadRequest("password is required for deployment".into()));
    }

    // Create server record first
    let db = state.db.clone();
    let srv = tokio::task::spawn_blocking({
        let body = server::CreateServerRequest {
            name: body.name.clone(),
            hostname: body.hostname.clone(),
            port: body.port,
            ssh_user: body.ssh_user.clone(),
            password: None,
        };
        move || {
            let conn = db.get()?;
            server::create(&conn, &body)
        }
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let server_id = srv.id.clone();
    let opts = agent_deployer::DeployOptions {
        hostname: body.hostname.clone(),
        port: body.port as u16,
        username: body.ssh_user.clone(),
        password,
        server_id: srv.id.clone(),
        server_port: state.config.port,
        backup_server_ip: state.config.backup_server_ip.clone(),
    };

    match agent_deployer::deploy_agent(opts, state.agents.clone()).await {
        Ok(()) => {
            let db = state.db.clone();
            let sid = server_id.clone();
            let updated = tokio::task::spawn_blocking(move || {
                let conn = db.get()?;
                server::find_by_id(&conn, &sid)
            })
            .await
            .map_err(|e| anyhow::anyhow!(e))??
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Server disappeared")))?;

            state.ui.broadcast("server:updated", serde_json::json!({ "server": updated }));
            Ok((axum::http::StatusCode::CREATED, Json(updated)))
        }
        Err(e) => {
            let error_msg = e.to_string();
            tracing::error!(hostname = %body.hostname, error = %error_msg, "Agent deployment failed");

            // Delete the failed server record
            let db = state.db.clone();
            let sid = server_id;
            let _ = tokio::task::spawn_blocking(move || {
                let conn = db.get()?;
                server::delete(&conn, &sid)
            })
            .await;

            Err(AppError::Unprocessable(error_msg))
        }
    }
}

async fn get_binary() -> Result<impl IntoResponse, AppError> {
    let binary_path = agent_deployer::get_agent_binary_path();

    if !binary_path.exists() {
        return Err(AppError::NotFound(
            "Agent binary not found. Build with: cd backup-agent && cargo build --release".into(),
        ));
    }

    let file = tokio::fs::File::open(&binary_path)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to open binary: {}", e)))?;

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"backup-agent\"",
            ),
        ],
        body,
    ))
}

async fn update_agent(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let db = state.db.clone();
    let sid = server_id.clone();
    let srv = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        server::find_by_id(&conn, &sid)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    if srv.is_none() {
        return Err(AppError::NotFound("Server not found".into()));
    }

    if !state.agents.is_connected(&server_id) {
        return Err(AppError::Conflict("Agent is not connected".into()));
    }

    let sent = state.agents.send_to_agent(
        &server_id,
        serde_json::json!({
            "type": "agent:update",
            "payload": {
                "download_path": "/api/agent/binary",
                "version": "latest",
            },
        }),
    );

    if !sent {
        return Err(AppError::Internal(anyhow::anyhow!("Failed to send update command")));
    }

    // Mark as updating
    let db = state.db.clone();
    let sid = server_id.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        server::update_fields(&conn, &sid, &[
            ("agent_status", &"updating" as &dyn rusqlite::types::ToSql),
        ])
    })
    .await;

    Ok(Json(serde_json::json!({ "status": "update_initiated" })))
}

async fn get_status(
    State(state): State<Arc<AppState>>,
    Path(server_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let connected = state.agents.is_connected(&server_id);
    let db = state.db.clone();
    let sid = server_id.clone();
    let srv = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        server::find_by_id(&conn, &sid)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    match srv {
        Some(s) => Ok(Json(serde_json::json!({
            "connected": connected,
            "agent_status": s.agent_status,
            "agent_version": s.agent_version,
            "agent_last_seen": s.agent_last_seen,
        }))),
        None => Err(AppError::NotFound("Server not found".into())),
    }
}
