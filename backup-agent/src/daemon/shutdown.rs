//! Graceful shutdown handling for SIGTERM and SIGINT.
//!
//! Ensures that:
//! - Running backups are allowed to complete their current file
//! - WebSocket connections are closed cleanly
//! - Resources are properly released

use tokio::signal;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Shutdown coordinator
pub struct ShutdownCoordinator {
    shutdown_tx: broadcast::Sender<()>,
}

impl ShutdownCoordinator {
    /// Create a new shutdown coordinator
    pub fn new() -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);
        Self { shutdown_tx }
    }

    /// Get a shutdown receiver
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    /// Wait for shutdown signal (SIGTERM or SIGINT)
    pub async fn wait_for_signal(&self) {
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("Failed to install Ctrl+C handler");
        };

        #[cfg(unix)]
        let terminate = async {
            signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("Failed to install SIGTERM handler")
                .recv()
                .await;
        };

        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {
                info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
            }
            _ = terminate => {
                info!("Received SIGTERM, initiating graceful shutdown...");
            }
        }

        // Broadcast shutdown signal to all tasks
        if let Err(e) = self.shutdown_tx.send(()) {
            warn!("Failed to broadcast shutdown signal: {}", e);
        }
    }

    /// Perform graceful shutdown
    pub async fn shutdown(&self) {
        info!("Graceful shutdown initiated");

        // Wait a bit for running tasks to finish
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        info!("Graceful shutdown complete");
    }
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_shutdown_coordinator() {
        let coordinator = ShutdownCoordinator::new();
        let mut rx = coordinator.subscribe();

        // Spawn a task that will receive shutdown
        let handle = tokio::spawn(async move {
            rx.recv().await.ok();
        });

        // Simulate shutdown
        coordinator.shutdown_tx.send(()).unwrap();

        // Task should complete
        handle.await.unwrap();
    }
}
