mod config;
mod db;
mod error;
mod models;
mod routes;
mod services;
mod state;
mod utils;
mod ws;

use crate::config::AppConfig;
use crate::db::connection::create_pool;
use crate::db::migrate::migrate;
use crate::services::backup_migration::migrate_existing_backups;
use crate::services::backup_scheduler::BackupScheduler;
use crate::services::db_backup::backup_database;
use crate::services::path_migration::migrate_server_folder_names;
use crate::services::server_ping::start_ping_service;
use crate::state::AppState;
use std::sync::Arc;
use tokio::signal;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = AppConfig::from_env();
    tracing::info!("Starting backup server on port {}", config.port);

    // Ensure data directories exist
    std::fs::create_dir_all(&config.data_dir)?;
    std::fs::create_dir_all(&config.keys_dir)?;

    // Initialize database
    let db_path = config.db_path.to_string_lossy().to_string();
    let pool = create_pool(&db_path);
    migrate(&pool, &config.data_dir, &config.keys_dir)?;

    // Daily database backup
    if let Err(e) = backup_database(&db_path, &config.data_dir) {
        tracing::warn!("Failed to create database backup: {}", e);
    }

    // Data migrations (one-time, idempotent)
    if let Err(e) = migrate_existing_backups(&pool) {
        tracing::warn!("Backup migration failed: {}", e);
    }
    if let Err(e) = migrate_server_folder_names(&pool) {
        tracing::warn!("Path migration failed: {}", e);
    }

    // Build application state
    let state = Arc::new(AppState::new(pool, config.clone()));

    // Start ping service
    let cancel = CancellationToken::new();
    start_ping_service(state.clone(), cancel.clone());

    // Initialize cron scheduler
    let scheduler = match BackupScheduler::new(state.clone()).await {
        Ok(s) => {
            if let Err(e) = s.init_schedules().await {
                tracing::warn!("Failed to initialize schedules: {}", e);
            }
            if let Err(e) = s.start().await {
                tracing::warn!("Failed to start scheduler: {}", e);
            }
            Some(s)
        }
        Err(e) => {
            tracing::warn!("Failed to create scheduler: {}", e);
            None
        }
    };

    // Build router
    let app = routes::create_router(state.clone());

    // Start HTTP server
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {}", addr);

    // Graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(cancel.clone()))
        .await?;

    // Cleanup
    tracing::info!("Shutting down...");
    cancel.cancel();

    // Stop scheduler
    if let Some(s) = scheduler {
        if let Err(e) = s.shutdown().await {
            tracing::warn!("Scheduler shutdown error: {}", e);
        }
    }

    // Close database
    db::connection::close_pool(&state.db);
    tracing::info!("Server stopped");

    Ok(())
}

async fn shutdown_signal(cancel: CancellationToken) {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to listen for ctrl+c");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received SIGINT"),
        _ = terminate => tracing::info!("Received SIGTERM"),
    }

    cancel.cancel();
}
