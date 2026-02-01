//! Self-update mechanism for the backup agent.
//!
//! Downloads a new binary from the backup server and restarts via systemd.

use tracing::{info, error};

/// Download a new agent binary and restart the service.
pub async fn self_update(download_url: &str, version: &str) {
    info!("Starting self-update to version {}", version);

    let tmp_path = "/tmp/backup-agent-new";
    let install_path = "/usr/local/bin/backup-agent";

    // Download new binary
    match download_binary(download_url, tmp_path).await {
        Ok(()) => {
            info!("Downloaded new binary to {}", tmp_path);
        }
        Err(e) => {
            error!("Failed to download update: {}", e);
            return;
        }
    }

    // Make executable
    if let Err(e) = std::process::Command::new("chmod")
        .args(["+x", tmp_path])
        .status()
    {
        error!("Failed to chmod new binary: {}", e);
        return;
    }

    // Replace current binary
    if let Err(e) = std::fs::rename(tmp_path, install_path) {
        // rename may fail across filesystems, try copy+remove
        if let Err(e2) = std::fs::copy(tmp_path, install_path) {
            error!("Failed to install new binary: {} (rename: {})", e2, e);
            return;
        }
        let _ = std::fs::remove_file(tmp_path);
    }

    info!("Installed new binary, restarting service...");

    // Restart via systemd
    match std::process::Command::new("systemctl")
        .args(["restart", "backup-agent"])
        .status()
    {
        Ok(status) if status.success() => {
            info!("Service restart initiated");
        }
        Ok(status) => {
            error!("systemctl restart exited with: {}", status);
        }
        Err(e) => {
            error!("Failed to restart service: {}", e);
        }
    }
}

async fn download_binary(url: &str, dest: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}: {}", response.status(), url).into());
    }

    let bytes = response.bytes().await?;
    std::fs::write(dest, &bytes)?;

    Ok(())
}
