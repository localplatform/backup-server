//! HTTP API module for the backup agent.

pub mod auth;
pub mod backup;
pub mod health;
pub mod job_tracker;

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub ws_state: Arc<RwLock<crate::ws::WsState>>,
    pub job_tracker: job_tracker::JobTracker,
}

/// Create the API router with all endpoints
pub fn create_router() -> Router {
    // Create shared state
    let state = AppState {
        ws_state: Arc::new(RwLock::new(crate::ws::WsState::new())),
        job_tracker: job_tracker::JobTracker::new(),
    };

    Router::new()
        // Health endpoints
        .route("/health", get(health::health))
        .route("/version", get(health::version))
        // Backup endpoints
        .route("/backup/start", post(backup::start_backup))
        .route("/backup/cancel", post(backup::cancel_backup))
        // WebSocket endpoint
        .route("/ws", get(crate::ws::ws_handler))
        .with_state(state)
}
