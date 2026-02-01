use crate::config::AppConfig;
use crate::db::connection::DbPool;
use crate::ws::ui::UiBroadcaster;
use crate::ws::agent_registry::AgentRegistry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct AppState {
    pub db: DbPool,
    pub config: AppConfig,
    pub ui: UiBroadcaster,
    pub agents: Arc<AgentRegistry>,
    pub running_jobs: Arc<Mutex<HashSet<String>>>,
    pub global_semaphore: Arc<tokio::sync::Semaphore>,
    pub server_semaphores: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>>,
}

impl AppState {
    pub fn new(db: DbPool, config: AppConfig) -> Self {
        let max_global = config.max_concurrent_global;
        let max_per_server = config.max_concurrent_per_server;
        Self {
            db,
            config,
            ui: UiBroadcaster::new(),
            agents: Arc::new(AgentRegistry::new()),
            running_jobs: Arc::new(Mutex::new(HashSet::new())),
            global_semaphore: Arc::new(tokio::sync::Semaphore::new(max_global)),
            server_semaphores: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn get_server_semaphore(&self, server_id: &str) -> Arc<tokio::sync::Semaphore> {
        let mut map = self.server_semaphores.lock().await;
        map.entry(server_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrent_per_server)))
            .clone()
    }
}
