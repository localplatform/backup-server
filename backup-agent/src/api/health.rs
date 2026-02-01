//! Health check endpoints.

use axum::{Json, response::IntoResponse};
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};

static START_TIME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

pub fn init_start_time() {
    START_TIME.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    });
}

/// GET /health - Health check endpoint
pub async fn health() -> impl IntoResponse {
    let uptime = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - START_TIME.get().unwrap_or(&0);

    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime,
        "active_jobs": 0,
    }))
}

/// GET /version - Version information endpoint
pub async fn version() -> impl IntoResponse {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "build": "dev",
        "features": ["zstd"],
    }))
}
