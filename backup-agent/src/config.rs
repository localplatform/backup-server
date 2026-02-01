//! Configuration management for the backup agent.
//!
//! Loads configuration from TOML file with environment variable overrides.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub agent: AgentConfig,
    pub server: ServerConfig,
    pub sync: SyncConfig,
    pub log: LogConfig,
    pub daemon: DaemonConfig,
    pub performance: PerformanceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique agent identifier
    pub id: String,

    /// HTTP/WebSocket server port
    pub port: u16,

    /// Working directory for temporary files
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Backend server URL
    pub url: String,

    /// Pre-shared key or JWT token
    pub token: String,

    /// Server ID this agent is associated with (set during deployment)
    #[serde(default)]
    pub server_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Chunk size in bytes (default: 1MB)
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,

    /// Compression algorithm (zstd, gzip, none)
    #[serde(default = "default_compression")]
    pub compression: String,

    /// Compression level (1-22 for zstd)
    #[serde(default = "default_compression_level")]
    pub compression_level: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub level: String,

    /// Log output (journald, file, stdout)
    #[serde(default = "default_log_output")]
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// PID file location
    pub pid_file: PathBuf,

    /// User to run as
    #[serde(default = "default_user")]
    pub user: String,

    /// Group to run as
    #[serde(default = "default_group")]
    pub group: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// Maximum concurrent backup jobs
    #[serde(default = "default_max_concurrent_jobs")]
    pub max_concurrent_jobs: usize,

    /// Number of I/O worker threads
    #[serde(default = "default_io_threads")]
    pub io_threads: usize,
}

// Default values
fn default_chunk_size() -> usize {
    1024 * 1024 // 1MB
}

fn default_compression() -> String {
    "zstd".to_string()
}

fn default_compression_level() -> i32 {
    3
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_output() -> String {
    "stdout".to_string()
}

fn default_user() -> String {
    "backup".to_string()
}

fn default_group() -> String {
    "backup".to_string()
}

fn default_max_concurrent_jobs() -> usize {
    1
}

fn default_io_threads() -> usize {
    4
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Create a default configuration
    pub fn default() -> Self {
        Config {
            agent: AgentConfig {
                id: hostname::get()
                    .ok()
                    .and_then(|h| h.into_string().ok())
                    .unwrap_or_else(|| "backup-agent-01".to_string()),
                port: 9990,
                data_dir: PathBuf::from("/var/lib/backup-agent"),
            },
            server: ServerConfig {
                url: "http://localhost:3000".to_string(),
                token: "".to_string(),
                server_id: None,
            },
            sync: SyncConfig {
                chunk_size: default_chunk_size(),
                compression: default_compression(),
                compression_level: default_compression_level(),
            },
            log: LogConfig {
                level: default_log_level(),
                output: default_log_output(),
            },
            daemon: DaemonConfig {
                pid_file: PathBuf::from("/var/run/backup-agent.pid"),
                user: default_user(),
                group: default_group(),
            },
            performance: PerformanceConfig {
                max_concurrent_jobs: default_max_concurrent_jobs(),
                io_threads: default_io_threads(),
            },
        }
    }
}
