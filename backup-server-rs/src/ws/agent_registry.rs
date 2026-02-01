use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

use crate::state::AppState;

#[derive(Debug)]
pub struct AgentConnection {
    pub server_id: String,
    pub hostname: String,
    pub version: String,
    pub tx: mpsc::UnboundedSender<String>,
}

pub struct AgentRegistry {
    agents: DashMap<String, AgentConnection>,
    pending_requests: DashMap<String, oneshot::Sender<Value>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
            pending_requests: DashMap::new(),
        }
    }

    pub fn register(&self, server_id: String, hostname: String, version: String, tx: mpsc::UnboundedSender<String>) {
        // Close old connection if exists
        if let Some((_, old)) = self.agents.remove(&server_id) {
            // drop old tx, which will close the old connection
            drop(old);
        }
        self.agents.insert(
            server_id.clone(),
            AgentConnection { server_id, hostname, version, tx },
        );
    }

    pub fn unregister(&self, server_id: &str) {
        self.agents.remove(server_id);
    }

    pub fn is_connected(&self, server_id: &str) -> bool {
        self.agents.contains_key(server_id)
    }

    pub fn get_connected_agents(&self) -> Vec<(String, String, String)> {
        self.agents
            .iter()
            .map(|entry| {
                let conn = entry.value();
                (conn.server_id.clone(), conn.hostname.clone(), conn.version.clone())
            })
            .collect()
    }

    pub fn send_to_agent(&self, server_id: &str, message: Value) -> bool {
        if let Some(agent) = self.agents.get(server_id) {
            agent.tx.send(message.to_string()).is_ok()
        } else {
            false
        }
    }

    pub async fn request_from_agent(
        &self,
        server_id: &str,
        mut message: Value,
        timeout_ms: u64,
    ) -> anyhow::Result<Value> {
        let request_id = uuid::Uuid::new_v4().to_string();

        // Inject request_id into message payload
        if let Some(payload) = message.get_mut("payload") {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("request_id".into(), Value::String(request_id.clone()));
            }
        }

        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(request_id.clone(), tx);

        if !self.send_to_agent(server_id, message) {
            self.pending_requests.remove(&request_id);
            anyhow::bail!("Agent not connected");
        }

        match tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            rx,
        )
        .await
        {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                self.pending_requests.remove(&request_id);
                anyhow::bail!("Agent request cancelled")
            }
            Err(_) => {
                self.pending_requests.remove(&request_id);
                anyhow::bail!("Agent request timed out")
            }
        }
    }

    pub fn resolve_request(&self, request_id: &str, response: Value) {
        if let Some((_, tx)) = self.pending_requests.remove(request_id) {
            let _ = tx.send(response);
        }
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_agent_socket(socket, state))
}

async fn handle_agent_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    let mut server_id: Option<String> = None;

    // Forward outgoing messages to the agent
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Handle incoming messages from the agent
    while let Some(Ok(msg)) = receiver.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Ping(_) => continue,
            Message::Pong(_) => continue,
            Message::Close(_) => break,
            _ => continue,
        };

        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Support both formats:
        // 1. {"type": "event_name", "payload": {...}} (standard)
        // 2. {"event_name": {...}} (serde externally-tagged enum from agent)
        let (msg_type, msg_payload) = if let Some(t) = parsed.get("type").and_then(|t| t.as_str()) {
            (t.to_string(), parsed.get("payload").cloned())
        } else if let Some(obj) = parsed.as_object() {
            if let Some((key, value)) = obj.iter().next() {
                if obj.len() == 1 {
                    (key.clone(), Some(value.clone()))
                } else {
                    (String::new(), None)
                }
            } else {
                (String::new(), None)
            }
        } else {
            (String::new(), None)
        };

        match msg_type.as_str() {
            "agent:register" => {
                let payload = msg_payload.clone().unwrap_or_default();
                let sid = payload.get("server_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let hostname = payload.get("hostname").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let version = payload.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string();

                if sid.is_empty() {
                    let err_msg = serde_json::json!({
                        "type": "agent:register:error",
                        "payload": { "error": "server_id is required" }
                    });
                    let _ = tx.send(err_msg.to_string());
                    continue;
                }

                // Verify server exists
                let db = state.db.clone();
                let sid2 = sid.clone();
                let exists = tokio::task::spawn_blocking(move || {
                    let conn = db.get()?;
                    Ok::<_, anyhow::Error>(crate::models::server::find_by_id(&conn, &sid2)?.is_some())
                })
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or(false);

                if !exists {
                    let err_msg = serde_json::json!({
                        "type": "agent:register:error",
                        "payload": { "error": "Server not found in database" }
                    });
                    let _ = tx.send(err_msg.to_string());
                    continue;
                }

                tracing::info!("Agent registered: server_id={}, hostname={}, version={}", sid, hostname, version);
                state.agents.register(sid.clone(), hostname.clone(), version.clone(), tx.clone());
                server_id = Some(sid.clone());

                // Update DB
                let db = state.db.clone();
                let sid3 = sid.clone();
                let ver = version.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let conn = db.get()?;
                    let now = chrono::Utc::now().to_rfc3339();
                    crate::models::server::update_fields(&conn, &sid3, &[
                        ("agent_status", &"connected" as &dyn rusqlite::types::ToSql),
                        ("agent_version", &ver as &dyn rusqlite::types::ToSql),
                        ("agent_last_seen", &now as &dyn rusqlite::types::ToSql),
                    ])
                }).await;

                let ok_msg = serde_json::json!({
                    "type": "agent:register:ok",
                    "payload": { "server_id": sid }
                });
                let _ = tx.send(ok_msg.to_string());

                state.ui.broadcast("agent:connected", serde_json::json!({
                    "serverId": sid,
                    "version": version,
                }));
            }
            _ => {
                // Check if this is a response to a pending request
                if let Some(ref payload) = msg_payload {
                    if let Some(request_id) = payload.get("request_id").and_then(|v| v.as_str()) {
                        state.agents.resolve_request(request_id, payload.clone());
                        continue;
                    }
                }

                // Forward other agent messages as UI broadcasts if relevant
                if msg_type.starts_with("backup:") {
                    if let Some(payload) = msg_payload {
                        let camel = snake_to_camel_keys(payload);
                        state.ui.broadcast(&msg_type, camel);
                    }
                }
            }
        }
    }

    // Cleanup on disconnect
    if let Some(sid) = &server_id {
        tracing::info!("Agent disconnected: server_id={}", sid);
        state.agents.unregister(sid);

        // Update DB
        let db = state.db.clone();
        let sid = sid.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let conn = db.get()?;
            crate::models::server::update_fields(&conn, &sid, &[
                ("agent_status", &"disconnected" as &dyn rusqlite::types::ToSql),
            ])
        }).await;

        state.ui.broadcast("agent:disconnected", serde_json::json!({
            "serverId": server_id,
        }));
    }

    send_task.abort();
}

/// Convert all keys in a JSON Value from snake_case to camelCase (recursive)
fn snake_to_camel_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                let camel = snake_to_camel(&k);
                new_map.insert(camel, snake_to_camel_keys(v));
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(snake_to_camel_keys).collect()),
        other => other,
    }
}

fn snake_to_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;
    for ch in s.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}
