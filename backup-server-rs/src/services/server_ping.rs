use crate::models::server;
use crate::state::AppState;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub fn start_ping_service(state: Arc<AppState>, cancel: CancellationToken) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = interval.tick() => {
                    let db = state.db.clone();
                    let agents = state.agents.clone();
                    let ui = state.ui.clone();

                    if let Ok(result) = tokio::task::spawn_blocking(move || {
                        let conn = db.get()?;
                        let servers = server::find_all(&conn)?;
                        let mut statuses = Vec::new();
                        for s in servers {
                            let connected = agents.is_connected(&s.id);

                            // Update agent_status in DB
                            let new_status = if connected { "connected" } else { "disconnected" };
                            if s.agent_status != new_status {
                                server::update_fields(&conn, &s.id, &[
                                    ("agent_status", &new_status as &dyn rusqlite::types::ToSql),
                                ])?;
                            }

                            statuses.push(serde_json::json!({
                                "serverId": s.id,
                                "reachable": connected,
                                "latencyMs": null,
                                "lastCheckedAt": chrono::Utc::now().to_rfc3339(),
                            }));
                        }
                        Ok::<_, anyhow::Error>(statuses)
                    }).await {
                        if let Ok(statuses) = result {
                            for status in statuses {
                                ui.broadcast("server:ping", status);
                            }
                        }
                    }
                }
            }
        }
        tracing::info!("Ping service stopped");
    });
}
