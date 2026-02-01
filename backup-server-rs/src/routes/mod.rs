pub mod servers;
pub mod jobs;
pub mod versions;
pub mod storage;
pub mod files;
pub mod agent;
pub mod explorer;

use crate::state::AppState;
use axum::Router;
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};

pub fn create_router(state: Arc<AppState>) -> Router {
    let client_dist = state.config.client_dist.clone();
    let index_html = client_dist.join("index.html");

    Router::new()
        .nest("/api/servers", servers::router(state.clone()))
        .nest("/api/jobs", jobs::router(state.clone()))
        .nest("/api/versions", versions::router(state.clone()))
        .nest("/api/storage", storage::router(state.clone()))
        .nest("/api/files", files::router(state.clone()))
        .nest("/api/agent", agent::router(state.clone()))
        .route("/ws", axum::routing::get(crate::ws::ui::ws_handler))
        .route("/ws/agent", axum::routing::get(crate::ws::agent_registry::ws_handler))
        .fallback_service(
            ServeDir::new(&client_dist)
                .fallback(ServeFile::new(index_html)),
        )
        .with_state(state)
}
