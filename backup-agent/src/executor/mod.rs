//! Backup job executor - Orchestrates the actual backup process.
//!
//! This module ties together all the Week 2 components:
//! - File system walker
//! - Signature generation
//! - Delta computation
//! - Progress tracking
//! - WebSocket event emission

use crate::fs::walker::{walk_directory, WalkOptions, FileInfo};
use crate::transfer::progress::{ProgressTracker, TransferProgress, format_speed};
use crate::transfer::progress_stream::ProgressStream;
use crate::ws::{WsState, WsEvent, BackupProgressPayload};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{RwLock, broadcast};
use tracing::{info, warn, error};
use tokio_util::io::ReaderStream;

/// Backup job configuration
#[derive(Debug, Clone)]
pub struct BackupJob {
    pub job_id: String,
    pub paths: Vec<PathBuf>,
    pub destination: PathBuf,
    pub server_url: String,
}

/// Backup execution result
#[derive(Debug)]
pub struct BackupResult {
    pub total_files: usize,
    pub total_bytes: u64,
    pub duration_secs: u64,
}

/// Main backup executor
pub struct BackupExecutor {
    ws_state: Arc<RwLock<WsState>>,
    shutdown_rx: Option<broadcast::Receiver<()>>,
}

impl BackupExecutor {
    /// Create a new backup executor
    pub fn new(ws_state: Arc<RwLock<WsState>>) -> Self {
        Self {
            ws_state,
            shutdown_rx: None,
        }
    }

    /// Create a new backup executor with shutdown support
    pub fn with_shutdown(ws_state: Arc<RwLock<WsState>>, shutdown_rx: broadcast::Receiver<()>) -> Self {
        Self {
            ws_state,
            shutdown_rx: Some(shutdown_rx),
        }
    }

    /// Execute a backup job
    pub async fn execute(&mut self, job: BackupJob) -> Result<BackupResult, Box<dyn std::error::Error + Send + Sync>> {
        let start_time = std::time::Instant::now();

        info!("Starting backup execution for job: {}", job.job_id);

        // Send backup:started event
        self.broadcast_event(WsEvent::BackupStarted {
            job_id: job.job_id.clone(),
        }).await;

        // Collect all files to backup
        let mut all_files = Vec::new();
        let mut total_size = 0u64;

        for path in &job.paths {
            match self.scan_path(path, &mut all_files, &mut total_size).await {
                Ok(_) => {
                    info!("Scanned path: {} ({} files, {} bytes)",
                          path.display(), all_files.len(), total_size);
                }
                Err(e) => {
                    error!("Failed to scan path {}: {}", path.display(), e);
                    self.broadcast_event(WsEvent::BackupFailed {
                        job_id: job.job_id.clone(),
                        error: format!("Failed to scan {}: {}", path.display(), e),
                    }).await;
                    return Err(e);
                }
            }
        }

        info!("Total files to backup: {}, total size: {} bytes", all_files.len(), total_size);

        // Create progress tracker
        let mut tracker = ProgressTracker::new(total_size, all_files.len());

        // Process each file
        let mut transferred_bytes = 0u64;
        let mut files_processed = 0usize;
        let mut last_progress_time = std::time::Instant::now();
        const PROGRESS_INTERVAL_MS: u64 = 250; // 4 updates per second

        info!("Starting file processing loop for {} files", all_files.len());

        for file_info in &all_files {
            info!("Processing file: {}", file_info.path.display());
            // Check for shutdown signal before processing each file
            if let Some(ref mut rx) = self.shutdown_rx {
                if rx.try_recv().is_ok() {
                    warn!("Shutdown signal received, stopping backup after current file");
                    self.broadcast_event(WsEvent::BackupFailed {
                        job_id: job.job_id.clone(),
                        error: "Backup interrupted by shutdown signal".to_string(),
                    }).await;
                    return Err("Shutdown signal received".into());
                }
            }

            match self.process_file(&job, file_info, &mut tracker).await {
                Ok(bytes) => {
                    transferred_bytes += bytes;
                    files_processed += 1;

                    // Update progress
                    let progress = tracker.update(transferred_bytes);

                    // Throttle progress events to 4x/second
                    let now = std::time::Instant::now();
                    if now.duration_since(last_progress_time).as_millis() >= PROGRESS_INTERVAL_MS as u128 {
                        self.send_progress(&job.job_id, progress, &file_info.relative_path).await;
                        last_progress_time = now;
                    }
                }
                Err(e) => {
                    warn!("Failed to process file {}: {}", file_info.path.display(), e);
                    // Continue with next file instead of failing entire backup
                }
            }
        }

        // Send final progress update at 100%
        let final_progress = tracker.update(transferred_bytes);
        self.send_progress(&job.job_id, final_progress, &PathBuf::from("Completed")).await;

        let duration = start_time.elapsed();
        let duration_secs = duration.as_secs();

        info!("Backup completed: {} files, {} bytes, {} seconds",
              files_processed, transferred_bytes, duration_secs);

        // Send backup:completed event
        self.broadcast_event(WsEvent::BackupCompleted {
            job_id: job.job_id.clone(),
            total_bytes: transferred_bytes,
        }).await;

        Ok(BackupResult {
            total_files: files_processed,
            total_bytes: transferred_bytes,
            duration_secs,
        })
    }

    /// Scan a source path and collect all files
    async fn scan_path(
        &self,
        path: &Path,
        all_files: &mut Vec<FileInfo>,
        total_size: &mut u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let walk_options = WalkOptions {
            follow_links: false,
            max_depth: None,
            exclude_patterns: vec![
                ".git".to_string(),
                "node_modules".to_string(),
                ".DS_Store".to_string(),
            ],
        };

        // Use blocking task for CPU-intensive directory walk
        let path_owned = path.to_path_buf();
        let files = tokio::task::spawn_blocking(move || {
            walk_directory(&path_owned, walk_options)
        }).await??;

        for file in files {
            if !file.is_dir {
                *total_size += file.size;
                all_files.push(file);
            }
        }

        Ok(())
    }

    /// Process a single file - upload to backup server with compression and real-time progress
    async fn process_file(
        &self,
        job: &BackupJob,
        file_info: &FileInfo,
        tracker: &ProgressTracker,
    ) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
        let file_bytes_transferred = Arc::new(AtomicU64::new(0));
        let total_bytes_before = tracker.progress().transferred_bytes;
        let total_bytes_overall = tracker.progress().total_bytes;
        let total_files = tracker.progress().total_files;
        let files_processed = tracker.progress().files_processed;
        // Open the file for reading
        let file = match tokio::fs::File::open(&file_info.path).await {
            Ok(f) => f,
            Err(e) => {
                error!("Failed to open file {}: {}", file_info.path.display(), e);
                return Err(Box::new(e));
            }
        };

        // Spawn progress monitoring task
        let file_bytes_clone = Arc::clone(&file_bytes_transferred);
        let ws_state_clone = Arc::clone(&self.ws_state);
        let job_id_clone = job.job_id.clone();
        let file_path_clone = file_info.path.clone();
        let total_bytes_clone = file_info.size;

        let monitor_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(250));
            let start_time = std::time::Instant::now();

            loop {
                interval.tick().await;
                let current = file_bytes_clone.load(Ordering::Relaxed);
                if current >= total_bytes_clone {
                    break;
                }

                // Calculate speed
                let elapsed = start_time.elapsed().as_secs_f64();
                let bytes_per_second = if elapsed > 0.0 {
                    (current as f64 / elapsed) as u64
                } else {
                    0
                };

                // Send progress update via WebSocket
                let overall_transferred = total_bytes_before + current;
                let percent = if total_bytes_overall > 0 {
                    ((overall_transferred as f64 / total_bytes_overall as f64) * 100.0).min(100.0)
                } else {
                    0.0
                };

                let eta_seconds = if bytes_per_second > 0 {
                    (total_bytes_clone - current) / bytes_per_second
                } else {
                    0
                };

                // Calculate per-file progress
                let current_file_percent = if total_bytes_clone > 0 {
                    ((current as f64 / total_bytes_clone as f64) * 100.0).min(100.0)
                } else {
                    0.0
                };

                // Format speed as human-readable string
                let speed = format_speed(bytes_per_second);

                let payload = BackupProgressPayload {
                    job_id: job_id_clone.clone(),
                    percent,
                    transferred_bytes: overall_transferred,
                    total_bytes: total_bytes_overall,
                    bytes_per_second,
                    eta_seconds,
                    current_file: Some(file_path_clone.display().to_string()),
                    files_processed,
                    total_files,
                    speed,
                    // Per-file progress
                    current_file_bytes: current,
                    current_file_total: total_bytes_clone,
                    current_file_percent,
                };

                let state = ws_state_clone.read().await;
                state.broadcast(WsEvent::BackupProgress(payload));
            }
        });

        // Only compress files smaller than 500 MB to avoid memory issues
        const MAX_COMPRESS_SIZE: u64 = 500 * 1024 * 1024; // 500 MB
        let use_compression = file_info.size < MAX_COMPRESS_SIZE;

        let client = reqwest::Client::new();
        let upload_url = format!("{}/api/files/upload", job.server_url);

        // Create progress callback
        let file_bytes_for_callback = Arc::clone(&file_bytes_transferred);
        let progress_callback = Arc::new(move |bytes: u64| {
            file_bytes_for_callback.store(bytes, Ordering::Relaxed);
        });

        let response = if use_compression {
            // Compress small/medium files
            use async_compression::tokio::bufread::ZstdEncoder;
            use tokio::io::BufReader;

            let buf_reader = BufReader::new(file);
            let compressed = ZstdEncoder::with_quality(buf_reader, async_compression::Level::Default);
            let stream = ReaderStream::new(compressed);
            let progress_stream = ProgressStream::new(stream, progress_callback.clone());
            let body = reqwest::Body::wrap_stream(progress_stream);

            client
                .post(&upload_url)
                .header("x-job-id", &job.job_id)
                .header("x-relative-path", file_info.relative_path.display().to_string())
                .header("x-total-size", file_info.size.to_string())
                .header("content-encoding", "zstd")
                .body(body)
                .send()
                .await
        } else {
            // Upload large files uncompressed
            let stream = ReaderStream::new(file);
            let progress_stream = ProgressStream::new(stream, progress_callback);
            let body = reqwest::Body::wrap_stream(progress_stream);

            client
                .post(&upload_url)
                .header("x-job-id", &job.job_id)
                .header("x-relative-path", file_info.relative_path.display().to_string())
                .header("x-total-size", file_info.size.to_string())
                .body(body)
                .send()
                .await
        };

        match response {
            Ok(resp) if resp.status().is_success() => {
                // Set final byte count to ensure monitoring task completes
                file_bytes_transferred.store(file_info.size, Ordering::Relaxed);

                // Wait for monitoring task to finish (it will exit when current >= total)
                let _ = monitor_handle.await;

                info!(
                    "Uploaded {} bytes: {} -> {}",
                    file_info.size,
                    file_info.path.display(),
                    upload_url
                );
                Ok(file_info.size)
            }
            Ok(resp) => {
                // Abort monitoring task on failure
                monitor_handle.abort();

                let status = resp.status();
                let error_text = resp.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                error!(
                    "Upload failed with status {}: {} -> {}. Error: {}",
                    status,
                    file_info.path.display(),
                    upload_url,
                    error_text
                );
                Err(format!("Upload failed: {} - {}", status, error_text).into())
            }
            Err(e) => {
                // Abort monitoring task on error
                monitor_handle.abort();

                error!(
                    "Upload request failed: {} -> {}. Error: {}",
                    file_info.path.display(),
                    upload_url,
                    e
                );
                Err(Box::new(e))
            }
        }
    }

    /// Send progress update via WebSocket
    async fn send_progress(&self, job_id: &str, progress: &TransferProgress, current_file: &Path) {
        let payload = BackupProgressPayload {
            job_id: job_id.to_string(),
            percent: progress.percent_complete,
            transferred_bytes: progress.transferred_bytes,
            total_bytes: progress.total_bytes,
            bytes_per_second: progress.bytes_per_second,
            eta_seconds: progress.eta_seconds,
            current_file: Some(current_file.display().to_string()),
            files_processed: progress.files_processed,
            total_files: progress.total_files,
            speed: format_speed(progress.bytes_per_second),
            // No per-file progress between files
            current_file_bytes: 0,
            current_file_total: 0,
            current_file_percent: 0.0,
        };

        self.broadcast_event(WsEvent::BackupProgress(payload)).await;
    }

    /// Broadcast an event to all WebSocket clients
    async fn broadcast_event(&self, event: WsEvent) {
        let state = self.ws_state.read().await;
        state.broadcast(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backup_job_creation() {
        let job = BackupJob {
            job_id: "test-job".to_string(),
            paths: vec![PathBuf::from("/tmp")],
            destination: PathBuf::from("/backup"),
            server_url: "http://localhost:3000".to_string(),
        };

        assert_eq!(job.job_id, "test-job");
        assert_eq!(job.paths.len(), 1);
    }
}
