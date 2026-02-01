use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::state::AppState;

const BROADCAST_CAPACITY: usize = 256;
const MAX_QUEUE_PER_JOB: usize = 100;

#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub event_type: String,
    pub payload: Value,
    pub timestamp: i64,
}

#[derive(Clone)]
pub struct UiBroadcaster {
    tx: broadcast::Sender<String>,
    queue: Arc<DashMap<String, VecDeque<QueuedMessage>>>,
}

impl UiBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            tx,
            queue: Arc::new(DashMap::new()),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, event_type: &str, payload: Value) {
        let msg = serde_json::json!({
            "type": event_type,
            "payload": payload,
        });
        let msg_str = msg.to_string();

        // Queue backup-related messages for replay
        if event_type.starts_with("backup:") {
            if let Some(job_id) = payload.get("jobId").and_then(|v| v.as_str()) {
                let mut entry = self.queue.entry(job_id.to_string()).or_insert_with(VecDeque::new);
                entry.push_back(QueuedMessage {
                    event_type: event_type.to_string(),
                    payload: payload.clone(),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                });
                if entry.len() > MAX_QUEUE_PER_JOB {
                    entry.pop_front();
                }
            }
        }

        let _ = self.tx.send(msg_str);
    }

    pub fn get_queued_messages(&self, job_id: &str, since: i64) -> Vec<QueuedMessage> {
        self.queue
            .get(job_id)
            .map(|q| q.iter().filter(|m| m.timestamp > since).cloned().collect())
            .unwrap_or_default()
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ui_socket(socket, state))
}

async fn handle_ui_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.ui.subscribe();

    // Spawn a task to forward broadcasts to this client
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages from client (e.g., replay:request)
    let ui = state.ui.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Text(text) = msg {
                if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                    if parsed.get("type").and_then(|t| t.as_str()) == Some("replay:request") {
                        if let Some(payload) = parsed.get("payload") {
                            let job_id = payload.get("jobId").and_then(|v| v.as_str()).unwrap_or("");
                            let since = payload.get("since").and_then(|v| v.as_i64()).unwrap_or(0);
                            let _messages = ui.get_queued_messages(job_id, since);
                            // Replay messages are sent via the broadcast channel
                            // The client will receive them through the normal broadcast path
                            for m in _messages {
                                let replay = serde_json::json!({
                                    "type": m.event_type,
                                    "payload": m.payload,
                                });
                                let _ = ui.tx.send(replay.to_string());
                            }
                        }
                    }
                }
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }
}

// Make tx accessible for replay
impl UiBroadcaster {
    fn send_raw(&self, msg: String) {
        let _ = self.tx.send(msg);
    }
}
