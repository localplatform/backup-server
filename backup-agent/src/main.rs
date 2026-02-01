//! Backup Agent - Main entry point
//!
//! Rust-based backup agent with delta-sync capabilities.

use anyhow::Result;
use backup_agent::{api, config::Config, utils, daemon::shutdown::ShutdownCoordinator, ws};
use clap::Parser;
use std::path::PathBuf;
use std::net::SocketAddr;
use tokio_util::sync::CancellationToken;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to configuration file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Port to listen on (overrides config)
    #[arg(short, long)]
    port: Option<u16>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long)]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Load configuration
    let config = if let Some(config_path) = args.config {
        Config::from_file(&config_path)?
    } else {
        Config::default()
    };

    // Initialize logging
    let log_level = args.log_level.as_deref().unwrap_or(&config.log.level);
    utils::logger::init(log_level)?;

    // Initialize start time for uptime tracking
    api::health::init_start_time();

    tracing::info!(
        "Starting backup-agent v{} (agent_id: {})",
        env!("CARGO_PKG_VERSION"),
        config.agent.id
    );

    // Determine port
    let port = args.port.unwrap_or(config.agent.port);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Create shutdown coordinator
    let shutdown_coordinator = ShutdownCoordinator::new();

    // Create shared app state (shared between HTTP router and WS client)
    let app_state = api::create_app_state();

    // Create API router with shared state
    let app = api::create_router_with_state(app_state.clone());

    // Spawn reverse WebSocket client to connect to the backup server
    let ws_shutdown = CancellationToken::new();
    let ws_shutdown_clone = ws_shutdown.clone();
    let server_url = config.server.url.clone();
    let server_id = config.server.server_id.clone();
    let agent_id = config.agent.id.clone();
    let ws_app_state = app_state.clone();

    let ws_client_handle = tokio::spawn(async move {
        let client = ws::client::AgentWsClient::new(
            server_url,
            server_id,
            agent_id,
            ws_app_state,
            ws_shutdown_clone,
        );
        client.run().await;
    });

    tracing::info!("Listening on http://{}", addr);
    tracing::info!("Health endpoint: http://{}/health", addr);
    tracing::info!("Version endpoint: http://{}/version", addr);
    tracing::info!("WebSocket endpoint: ws://{}/ws", addr);
    tracing::info!("Server connection: {}/ws/agent", config.server.url);

    // Start server
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Spawn server task
    let server = axum::serve(listener, app);
    let server_handle = tokio::spawn(async move {
        server.await
    });

    // Wait for shutdown signal
    shutdown_coordinator.wait_for_signal().await;

    // Signal WS client to stop
    ws_shutdown.cancel();

    // Graceful shutdown
    shutdown_coordinator.shutdown().await;

    // Wait for WS client to finish
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), ws_client_handle).await;

    // Wait for server to finish (with timeout)
    match tokio::time::timeout(std::time::Duration::from_secs(5), server_handle).await {
        Ok(Ok(Ok(()))) => tracing::info!("Server shutdown complete"),
        Ok(Ok(Err(e))) => tracing::error!("Server error during shutdown: {}", e),
        Ok(Err(e)) => tracing::error!("Server task panicked: {}", e),
        Err(_) => tracing::warn!("Server shutdown timeout, forcing exit"),
    }

    Ok(())
}
