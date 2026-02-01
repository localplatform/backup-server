//! Reverse WebSocket client — connects to the backup server.
//!
//! The agent initiates and maintains a persistent WebSocket connection to the
//! backup server at `ws://{server_url}/ws/agent`. This is the primary
//! communication channel for:
//! - Registration handshake (agent identity)
//! - Receiving backup commands (start, cancel)
//! - Receiving filesystem browse requests
//! - Receiving update commands
//! - Forwarding local WsEvent broadcasts to the server (progress, completion)

use crate::api::AppState;
use crate::api::filesystem;
use crate::ws::WsEvent;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::path::PathBuf;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Commands received from the backup server via WebSocket.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerCommand {
    #[serde(rename = "backup:start")]
    StartBackup(StartBackupPayload),

    #[serde(rename = "backup:cancel")]
    CancelBackup { job_id: String },

    #[serde(rename = "fs:browse")]
    BrowseFilesystem {
        path: String,
        request_id: String,
    },

    #[serde(rename = "agent:update")]
    UpdateAgent {
        download_path: String,
        version: String,
    },

    /// Registration acknowledgment from server
    #[serde(rename = "agent:register:ok")]
    RegisterOk { server_id: String },

    #[serde(rename = "agent:register:error")]
    RegisterError { error: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartBackupPayload {
    pub job_id: String,
    pub paths: Vec<String>,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
}

/// Reverse WebSocket client that connects to the backup server.
pub struct AgentWsClient {
    server_url: String,
    server_id: Option<String>,
    agent_id: String,
    app_state: AppState,
    shutdown: CancellationToken,
}

impl AgentWsClient {
    pub fn new(
        server_url: String,
        server_id: Option<String>,
        agent_id: String,
        app_state: AppState,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            server_url,
            server_id,
            agent_id,
            app_state,
            shutdown,
        }
    }

    /// Run the WebSocket client with automatic reconnection.
    pub async fn run(&self) {
        let mut backoff_ms: u64 = 1000;
        let max_backoff_ms: u64 = 30000;

        loop {
            if self.shutdown.is_cancelled() {
                info!("WS client shutting down");
                return;
            }

            match self.connect_and_run().await {
                Ok(()) => {
                    info!("WS client connection closed normally");
                    backoff_ms = 1000; // Reset backoff on clean disconnect
                }
                Err(e) => {
                    warn!("WS client connection error: {}", e);
                }
            }

            if self.shutdown.is_cancelled() {
                return;
            }

            info!("Reconnecting in {}ms...", backoff_ms);
            tokio::select! {
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)) => {}
                _ = self.shutdown.cancelled() => return,
            }

            backoff_ms = (backoff_ms * 2).min(max_backoff_ms);
        }
    }

    async fn connect_and_run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Build WebSocket URL
        let ws_url = self.server_url
            .replace("http://", "ws://")
            .replace("https://", "wss://");
        let url = format!("{}/ws/agent", ws_url);

        info!("Connecting to server WebSocket: {}", url);

        let (ws_stream, _) = connect_async(&url).await?;
        let (mut write, mut read) = ws_stream.split();

        info!("Connected to server WebSocket");

        // Send registration handshake
        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| "unknown".to_string());

        let register_msg = serde_json::json!({
            "type": "agent:register",
            "payload": {
                "hostname": hostname,
                "version": env!("CARGO_PKG_VERSION"),
                "server_id": self.server_id,
                "agent_id": self.agent_id,
            }
        });

        write.send(Message::Text(register_msg.to_string().into())).await?;
        info!("Registration handshake sent");

        // Subscribe to local broadcast channel to forward events to server
        let ws_state = self.app_state.ws_state.read().await;
        let mut rx = ws_state.tx.subscribe();
        drop(ws_state);

        let app_state = self.app_state.clone();
        let shutdown = self.shutdown.clone();

        loop {
            tokio::select! {
                // Forward local events to the server
                event = rx.recv() => {
                    match event {
                        Ok(ws_event) => {
                            if let Ok(json) = serde_json::to_string(&ws_event) {
                                if write.send(Message::Text(json.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            warn!("WS client lagged by {} messages", n);
                        }
                        Err(_) => break,
                    }
                }

                // Handle incoming messages from server
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            handle_server_message(&text, &app_state, &self.server_url).await;
                        }
                        Some(Ok(Message::Ping(data))) => {
                            if write.send(Message::Pong(data)).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => {
                            info!("Server closed WebSocket connection");
                            break;
                        }
                        Some(Err(e)) => {
                            error!("WebSocket read error: {}", e);
                            break;
                        }
                        _ => {}
                    }
                }

                _ = shutdown.cancelled() => {
                    info!("Shutdown signal received, closing WS client");
                    let _ = write.send(Message::Close(None)).await;
                    break;
                }
            }
        }

        Ok(())
    }
}

/// Handle a message received from the backup server.
async fn handle_server_message(text: &str, app_state: &AppState, server_url: &str) {
    let parsed: Result<ServerCommand, _> = serde_json::from_str(text);

    match parsed {
        Ok(ServerCommand::StartBackup(payload)) => {
            handle_start_backup(payload, app_state).await;
        }
        Ok(ServerCommand::CancelBackup { job_id }) => {
            handle_cancel_backup(&job_id, app_state).await;
        }
        Ok(ServerCommand::BrowseFilesystem { path, request_id }) => {
            handle_browse_filesystem(&path, &request_id, app_state).await;
        }
        Ok(ServerCommand::UpdateAgent { download_path, version }) => {
            let download_url = format!("{}{}", server_url.trim_end_matches('/'), download_path);
            handle_update_agent(&download_url, &version).await;
        }
        Ok(ServerCommand::RegisterOk { server_id }) => {
            info!("Registration confirmed for server_id: {}", server_id);
        }
        Ok(ServerCommand::RegisterError { error }) => {
            error!("Registration failed: {}", error);
        }
        Err(e) => {
            warn!("Failed to parse server command: {} (raw: {})", e, text);
        }
    }
}

async fn handle_start_backup(payload: StartBackupPayload, app_state: &AppState) {
    info!("Received backup:start command for job: {}", payload.job_id);

    let paths: Vec<PathBuf> = payload.paths.iter().map(PathBuf::from).collect();
    let destination = PathBuf::from(format!("/tmp/backup-{}", payload.job_id));
    let server_url = payload.server_url.unwrap_or_default();

    let job = crate::executor::BackupJob {
        job_id: payload.job_id.clone(),
        paths,
        destination,
        server_url,
    };

    let cancel_token = CancellationToken::new();
    let executor_token = cancel_token.clone();

    let mut executor = crate::executor::BackupExecutor::with_cancel(
        app_state.ws_state.clone(),
        executor_token,
    );

    let job_id = payload.job_id.clone();
    let tracker = app_state.job_tracker.clone();

    let handle = tokio::spawn(async move {
        match executor.execute(job).await {
            Ok(result) => {
                info!(
                    "Backup completed: {} files, {} bytes, {}s",
                    result.total_files, result.total_bytes, result.duration_secs
                );
                tracker.complete(&job_id).await;
            }
            Err(e) => {
                error!("Backup failed: {}", e);
                tracker.complete(&job_id).await;
            }
        }
    });

    app_state
        .job_tracker
        .register(payload.job_id, handle.abort_handle(), cancel_token)
        .await;
}

async fn handle_cancel_backup(job_id: &str, app_state: &AppState) {
    info!("Received backup:cancel command for job: {}", job_id);
    let cancelled = app_state.job_tracker.cancel(job_id).await;
    if cancelled {
        info!("Job {} cancelled successfully", job_id);
    } else {
        warn!("Job {} not found or already completed", job_id);
    }
}

async fn handle_browse_filesystem(path: &str, request_id: &str, app_state: &AppState) {
    info!("Received fs:browse for path: {}", path);

    let result = filesystem::browse_path(path);

    let (entries, error) = match result {
        Ok(entries) => (entries, None),
        Err(e) => (vec![], Some(e.to_string())),
    };

    // Broadcast the response — the WS client forwards all WsEvents to the server
    let ws_state = app_state.ws_state.read().await;
    ws_state.broadcast(WsEvent::FsBrowseResponse {
        request_id: request_id.to_string(),
        entries,
        error,
    });
}

async fn handle_update_agent(download_url: &str, version: &str) {
    info!("Received agent:update command: version={}, url={}", version, download_url);
    crate::update::self_update(download_url, version).await;
}
