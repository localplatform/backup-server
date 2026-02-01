//! WebSocket server for real-time progress streaming.
//!
//! This module provides bidirectional communication between the agent and server:
//! - Agent → Server: Progress updates, status changes, logs
//! - Server → Agent: Control commands (pause, resume, cancel)

pub mod client;
pub mod handler;

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, error, info, warn};

/// Maximum number of queued messages per subscriber
const BROADCAST_CAPACITY: usize = 1000;

/// WebSocket event types sent from agent to server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum WsEvent {
    /// Backup job progress update
    #[serde(rename = "backup:progress")]
    BackupProgress(BackupProgressPayload),

    /// Backup job started
    #[serde(rename = "backup:started")]
    BackupStarted { job_id: String },

    /// Backup job completed successfully
    #[serde(rename = "backup:completed")]
    BackupCompleted {
        job_id: String,
        total_bytes: u64,
        total_files: usize,
        #[serde(default)]
        transferred_bytes: u64,
        #[serde(default)]
        transferred_files: usize,
        #[serde(default)]
        unchanged_files: usize,
        #[serde(default)]
        unchanged_bytes: u64,
        #[serde(default)]
        deleted_files: usize,
        #[serde(default)]
        backup_type: String,
    },

    /// Backup job failed
    #[serde(rename = "backup:failed")]
    BackupFailed { job_id: String, error: String },

    /// Agent status update
    #[serde(rename = "agent:status")]
    AgentStatus(AgentStatusPayload),

    /// Log message
    #[serde(rename = "agent:log")]
    LogMessage { level: String, message: String },

    /// Filesystem browse response (sent to server via reverse WS)
    #[serde(rename = "fs:browse:response")]
    FsBrowseResponse {
        request_id: String,
        entries: Vec<crate::api::filesystem::FsEntry>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

/// Progress information for a backup job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupProgressPayload {
    pub job_id: String,
    pub percent: f64,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: u64,
    pub eta_seconds: u64,
    pub current_file: Option<String>,
    pub files_processed: usize,
    pub total_files: usize,
    pub speed: String,
    // Per-file progress (legacy single file)
    pub current_file_bytes: u64,
    pub current_file_total: u64,
    pub current_file_percent: f64,
    // Active parallel transfers
    #[serde(default)]
    pub active_files: Vec<ActiveFileProgress>,
    // Incremental backup stats
    #[serde(default)]
    pub skipped_files: usize,
    #[serde(default)]
    pub skipped_bytes: u64,
    #[serde(default)]
    pub backup_type: String,
}

/// Progress for a single active file transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveFileProgress {
    pub path: String,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub percent: f64,
}

/// Agent status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatusPayload {
    pub status: String, // "idle", "running", "paused"
    pub active_jobs: usize,
    pub uptime_secs: u64,
}

/// WebSocket command types received from server
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum WsCommand {
    /// Pause a backup job
    #[serde(rename = "backup:pause")]
    PauseBackup { job_id: String },

    /// Resume a paused backup job
    #[serde(rename = "backup:resume")]
    ResumeBackup { job_id: String },

    /// Cancel a backup job
    #[serde(rename = "backup:cancel")]
    CancelBackup { job_id: String },

    /// Request agent status
    #[serde(rename = "agent:status")]
    GetStatus,
}

/// Shared WebSocket state
#[derive(Clone)]
pub struct WsState {
    /// Broadcast channel for sending events to all connected clients
    pub tx: broadcast::Sender<WsEvent>,
}

impl WsState {
    /// Create a new WebSocket state
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self { tx }
    }

    /// Broadcast an event to all connected WebSocket clients
    pub fn broadcast(&self, event: WsEvent) {
        match self.tx.send(event.clone()) {
            Ok(count) => {
                debug!("Broadcast event to {} client(s): {:?}", count, event);
            }
            Err(e) => {
                warn!("Failed to broadcast event (no receivers): {:?}", e);
            }
        }
    }

    /// Subscribe to events (for new WebSocket connections)
    pub fn subscribe(&self) -> broadcast::Receiver<WsEvent> {
        self.tx.subscribe()
    }
}

impl Default for WsState {
    fn default() -> Self {
        Self::new()
    }
}

/// WebSocket upgrade handler
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(app_state): axum::extract::State<crate::api::AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, app_state.ws_state))
}

/// Handle a WebSocket connection
async fn handle_socket(socket: WebSocket, state: Arc<RwLock<WsState>>) {
    info!("New WebSocket client connected");

    let (mut sender, mut receiver) = socket.split();

    // Subscribe to broadcast events
    let state_read = state.read().await;
    let mut rx = state_read.subscribe();
    drop(state_read);

    // Spawn task to forward broadcast events to this client
    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = rx.recv().await {
            // Serialize event to JSON
            match serde_json::to_string(&event) {
                Ok(json) => {
                    if sender.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(e) => {
                    error!("Failed to serialize event: {:?}", e);
                }
            }
        }
    });

    // Spawn task to handle incoming commands from client
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                match serde_json::from_str::<WsCommand>(&text) {
                    Ok(command) => {
                        handler::handle_command(command).await;
                    }
                    Err(e) => {
                        warn!("Failed to parse WebSocket command: {:?}", e);
                    }
                }
            }
        }
    });

    // Wait for either task to finish (connection closed or error)
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }

    info!("WebSocket client disconnected");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_state_creation() {
        let state = WsState::new();
        // Should be able to broadcast without panic
        state.broadcast(WsEvent::AgentStatus(AgentStatusPayload {
            status: "idle".to_string(),
            active_jobs: 0,
            uptime_secs: 0,
        }));
    }

    #[test]
    fn test_event_serialization() {
        let event = WsEvent::BackupProgress(BackupProgressPayload {
            job_id: "test-job".to_string(),
            percent: 50.0,
            transferred_bytes: 1024,
            total_bytes: 2048,
            bytes_per_second: 512,
            eta_seconds: 2,
            current_file: Some("test.txt".to_string()),
            files_processed: 5,
            total_files: 10,
            speed: "512 B/s".to_string(),
            current_file_bytes: 512,
            current_file_total: 1024,
            current_file_percent: 50.0,
            active_files: vec![],
            skipped_files: 0,
            skipped_bytes: 0,
            backup_type: "full".to_string(),
        });

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("backup:progress"));
        assert!(json.contains("test-job"));
    }

    #[test]
    fn test_command_deserialization() {
        let json = r#"{"type":"backup:pause","payload":{"job_id":"test"}}"#;
        let command: WsCommand = serde_json::from_str(json).unwrap();

        match command {
            WsCommand::PauseBackup { job_id } => {
                assert_eq!(job_id, "test");
            }
            _ => panic!("Wrong command type"),
        }
    }
}
