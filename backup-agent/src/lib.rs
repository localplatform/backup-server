//! Backup Agent Library
//!
//! Rust-based backup agent with delta-sync capabilities using fast_rsync.

pub mod api;
pub mod config;
pub mod daemon;
pub mod executor;
pub mod fs;
pub mod sync;
pub mod transfer;
pub mod utils;
pub mod ws;

// Re-export commonly used types
pub use config::Config;
pub use utils::errors::AgentError;
pub type Result<T> = std::result::Result<T, AgentError>;
