use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub port: u16,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub keys_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub client_dist: PathBuf,
    pub log_level: String,
    pub max_concurrent_global: usize,
    pub max_concurrent_per_server: usize,
    pub backup_server_ip: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Self {
        let _ = dotenvy::dotenv();

        let server_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../server");
        let data_dir = server_root.join("data");

        Self {
            port: std::env::var("PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3000),
            db_path: data_dir.join("backup-server.db"),
            keys_dir: data_dir.join("keys"),
            data_dir,
            backups_dir: PathBuf::from(
                std::env::var("BACKUPS_DIR").unwrap_or_else(|_| "/backup/data/backups".into()),
            ),
            client_dist: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../client/dist"),
            log_level: std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".into()),
            max_concurrent_global: std::env::var("MAX_CONCURRENT_GLOBAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8),
            max_concurrent_per_server: std::env::var("MAX_CONCURRENT_PER_SERVER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            backup_server_ip: std::env::var("BACKUP_SERVER_IP").ok(),
        }
    }
}
