//! Backup job executor - Orchestrates the actual backup process.
//!
//! This module ties together all the Week 2 components:
//! - File system walker
//! - Signature generation
//! - Delta computation
//! - Progress tracking
//! - WebSocket event emission

use crate::fs::walker::{walk_directory, WalkOptions, FileInfo};
use crate::transfer::progress::format_speed;
use crate::transfer::progress_stream::ProgressStream;
use crate::ws::{WsState, WsEvent, BackupProgressPayload, ActiveFileProgress};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::{RwLock, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn, error};
use tokio_util::io::ReaderStream;

/// Concurrency weight budget — total permits in the semaphore.
/// Small files take 1 permit (up to 64 concurrent), large files take all 64 (sequential).
const CONCURRENCY_BUDGET: usize = 64;

/// Returns the number of semaphore permits a file should acquire based on its size.
/// This automatically adapts parallelism: many small files run concurrently,
/// large files limit concurrency to avoid bandwidth saturation.
///
/// | File size       | Permits | Effective concurrency |
/// |-----------------|---------|----------------------|
/// | < 10 MB         | 1       | 64                   |
/// | 10 – 100 MB     | 2       | 32                   |
/// | 100 – 500 MB    | 16      | 4                    |
/// | 500 MB – 1 GB   | 32      | 2                    |
/// | > 1 GB          | 64      | 1                    |
fn concurrency_weight(file_size: u64) -> u32 {
    match file_size {
        0..=10_485_759              => 1,  // < 10 MB
        10_485_760..=104_857_599    => 2,  // 10 – 100 MB
        104_857_600..=524_287_999   => 16, // 100 – 500 MB
        524_288_000..=1_073_741_823 => 32, // 500 MB – 1 GB
        _ => 64,                           // > 1 GB
    }
}

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

/// Tracks an active file transfer (shared between upload task and progress broadcaster)
struct ActiveFileState {
    path: String,
    total_bytes: u64,
    transferred: AtomicU64,
}

/// Main backup executor
pub struct BackupExecutor {
    ws_state: Arc<RwLock<WsState>>,
    cancel_token: CancellationToken,
}

impl BackupExecutor {
    /// Create a new backup executor (no cancellation support)
    pub fn new(ws_state: Arc<RwLock<WsState>>) -> Self {
        Self {
            ws_state,
            cancel_token: CancellationToken::new(),
        }
    }

    /// Create a new backup executor with cancellation support
    pub fn with_cancel(ws_state: Arc<RwLock<WsState>>, cancel_token: CancellationToken) -> Self {
        Self {
            ws_state,
            cancel_token,
        }
    }

    /// Execute a backup job
    pub async fn execute(&mut self, job: BackupJob) -> Result<BackupResult, Box<dyn std::error::Error + Send + Sync>> {
        let start_time = std::time::Instant::now();

        info!("Starting backup execution for job: {} (adaptive concurrency, budget: {})", job.job_id, CONCURRENCY_BUDGET);

        // Send backup:started event
        self.broadcast_event(WsEvent::BackupStarted {
            job_id: job.job_id.clone(),
        }).await;

        // Collect all files to backup
        let mut all_files = Vec::new();
        let mut total_size = 0u64;

        for path in &job.paths {
            // Check cancellation before scanning
            if self.cancel_token.is_cancelled() {
                return Err("Backup cancelled".into());
            }

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

        // Sort files smallest-first for optimal concurrency:
        // small files process with high parallelism, large files later with lower parallelism
        all_files.sort_by_key(|f| f.size);

        let total_files_count = all_files.len();
        info!("Total files to backup: {}, total size: {} bytes (sorted by size, smallest first)", total_files_count, total_size);

        // Shared counters for completed work
        let completed_bytes = Arc::new(AtomicU64::new(0));
        let completed_files = Arc::new(AtomicUsize::new(0));

        // Shared map of currently active file transfers (for progress reporting)
        let active_files: Arc<RwLock<HashMap<usize, Arc<ActiveFileState>>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Weighted semaphore: total budget = CONCURRENCY_BUDGET permits.
        // Each file acquires 1-16 permits based on size (see concurrency_weight()).
        let semaphore = Arc::new(Semaphore::new(CONCURRENCY_BUDGET));

        // Spawn a single progress broadcast task that reads all shared state
        let progress_ws = Arc::clone(&self.ws_state);
        let progress_job_id = job.job_id.clone();
        let progress_completed_bytes = Arc::clone(&completed_bytes);
        let progress_completed_files = Arc::clone(&completed_files);
        let progress_active_files = Arc::clone(&active_files);
        let progress_cancel = self.cancel_token.clone();
        let progress_total_bytes = total_size;
        let progress_total_files = total_files_count;

        let progress_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(250));
            let mut last_total = 0u64;
            let mut last_time = std::time::Instant::now();

            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = progress_cancel.cancelled() => { break; }
                }

                if progress_cancel.is_cancelled() {
                    break;
                }

                let done_bytes = progress_completed_bytes.load(Ordering::Relaxed);
                let done_files = progress_completed_files.load(Ordering::Relaxed);

                // Read active files and compute in-flight bytes
                let active_map = progress_active_files.read().await;
                let mut inflight_bytes = 0u64;
                let mut file_list: Vec<ActiveFileProgress> = Vec::with_capacity(active_map.len());

                for (_id, state) in active_map.iter() {
                    let transferred = state.transferred.load(Ordering::Relaxed);
                    inflight_bytes += transferred;
                    file_list.push(ActiveFileProgress {
                        path: state.path.clone(),
                        transferred_bytes: transferred,
                        total_bytes: state.total_bytes,
                        percent: if state.total_bytes > 0 {
                            ((transferred as f64 / state.total_bytes as f64) * 100.0).min(100.0)
                        } else {
                            0.0
                        },
                    });
                }
                drop(active_map);

                let total_transferred = done_bytes + inflight_bytes;

                // Calculate speed
                let now = std::time::Instant::now();
                let elapsed_window = now.duration_since(last_time).as_secs_f64();
                let bytes_per_second = if elapsed_window > 0.1 {
                    let diff = total_transferred.saturating_sub(last_total);
                    (diff as f64 / elapsed_window) as u64
                } else {
                    0
                };
                last_total = total_transferred;
                last_time = now;

                let percent = if progress_total_bytes > 0 {
                    ((total_transferred as f64 / progress_total_bytes as f64) * 100.0).min(100.0)
                } else {
                    0.0
                };

                let eta_seconds = if bytes_per_second > 0 {
                    progress_total_bytes.saturating_sub(total_transferred) / bytes_per_second
                } else {
                    0
                };

                // Use first active file as "current file" for legacy compatibility
                let current_file = file_list.first().map(|f| f.path.clone());
                let (cf_bytes, cf_total, cf_percent) = file_list.first()
                    .map(|f| (f.transferred_bytes, f.total_bytes, f.percent))
                    .unwrap_or((0, 0, 0.0));

                let payload = BackupProgressPayload {
                    job_id: progress_job_id.clone(),
                    percent,
                    transferred_bytes: total_transferred,
                    total_bytes: progress_total_bytes,
                    bytes_per_second,
                    eta_seconds,
                    current_file,
                    files_processed: done_files,
                    total_files: progress_total_files,
                    speed: format_speed(bytes_per_second),
                    current_file_bytes: cf_bytes,
                    current_file_total: cf_total,
                    current_file_percent: cf_percent,
                    active_files: file_list,
                };

                let state = progress_ws.read().await;
                state.broadcast(WsEvent::BackupProgress(payload));
            }
        });

        // Spawn parallel file upload tasks with adaptive concurrency
        info!("Starting parallel file processing: {} files, adaptive concurrency (budget: {})", total_files_count, CONCURRENCY_BUDGET);

        let mut handles = Vec::with_capacity(all_files.len());

        for (idx, file_info) in all_files.into_iter().enumerate() {
            let sem = Arc::clone(&semaphore);
            let job_id = job.job_id.clone();
            let server_url = job.server_url.clone();
            let global_completed_bytes = Arc::clone(&completed_bytes);
            let global_completed_files = Arc::clone(&completed_files);
            let active_map = Arc::clone(&active_files);
            let cancel = self.cancel_token.clone();

            let handle = tokio::spawn(async move {
                // Check cancellation before acquiring permit
                if cancel.is_cancelled() {
                    return Err::<u64, Box<dyn std::error::Error + Send + Sync>>(
                        "Cancelled".into()
                    );
                }

                // Acquire weighted semaphore permits based on file size
                let weight = concurrency_weight(file_info.size);
                let permit = tokio::select! {
                    result = sem.acquire_many(weight) => {
                        result.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                            Box::new(std::io::Error::new(std::io::ErrorKind::Other, format!("Semaphore closed: {}", e)))
                        })?
                    }
                    _ = cancel.cancelled() => {
                        return Err("Cancelled".into());
                    }
                };

                // Check cancellation after acquiring permit
                if cancel.is_cancelled() {
                    drop(permit);
                    return Err("Cancelled".into());
                }

                // Register in active files map
                let file_state = Arc::new(ActiveFileState {
                    path: file_info.path.display().to_string(),
                    total_bytes: file_info.size,
                    transferred: AtomicU64::new(0),
                });
                {
                    let mut map = active_map.write().await;
                    map.insert(idx, Arc::clone(&file_state));
                }

                info!("Processing file: {}", file_info.path.display());

                let result = upload_file(
                    &job_id,
                    &server_url,
                    &file_info,
                    &file_state,
                    &cancel,
                ).await;

                // Remove from active files map
                {
                    let mut map = active_map.write().await;
                    map.remove(&idx);
                }

                // Drop permit to allow next task
                drop(permit);

                match result {
                    Ok(bytes) => {
                        global_completed_bytes.fetch_add(bytes, Ordering::Relaxed);
                        global_completed_files.fetch_add(1, Ordering::Relaxed);
                        Ok(bytes)
                    }
                    Err(e) => {
                        warn!("Failed to process file {}: {}", file_info.path.display(), e);
                        global_completed_files.fetch_add(1, Ordering::Relaxed);
                        Err(e)
                    }
                }
            });

            handles.push(handle);
        }

        // Wait for all tasks to complete
        let mut total_processed = 0usize;
        for handle in handles {
            match handle.await {
                Ok(Ok(_bytes)) => {
                    total_processed += 1;
                }
                Ok(Err(e)) => {
                    if self.cancel_token.is_cancelled() {
                        info!("Task cancelled: {}", e);
                    } else {
                        warn!("File upload task failed: {}", e);
                    }
                }
                Err(e) => {
                    if e.is_cancelled() {
                        info!("Task was aborted");
                    } else {
                        warn!("File upload task panicked: {}", e);
                    }
                }
            }
        }

        // Check if cancellation was requested by the user (before we cancel the progress task)
        let was_cancelled_by_user = self.cancel_token.is_cancelled();

        // Stop the progress broadcast task
        self.cancel_token.cancel(); // No-op if not already cancelled
        let _ = progress_task.await;

        // Read final values from atomics
        let final_transferred = completed_bytes.load(Ordering::Relaxed);
        let final_files = completed_files.load(Ordering::Relaxed);

        let duration = start_time.elapsed();
        let duration_secs = duration.as_secs();

        // Check if we were cancelled by the user
        if was_cancelled_by_user && total_processed < total_files_count {
            info!("Backup cancelled: {} files processed out of {}", total_processed, total_files_count);

            self.broadcast_event(WsEvent::BackupFailed {
                job_id: job.job_id.clone(),
                error: "Backup cancelled by user".to_string(),
            }).await;

            return Err("Backup cancelled by user".into());
        }

        info!("Backup completed: {} files ({} successful), {} bytes, {} seconds",
              final_files, total_processed, final_transferred, duration_secs);

        // Send final 100% progress
        {
            let payload = BackupProgressPayload {
                job_id: job.job_id.clone(),
                percent: 100.0,
                transferred_bytes: final_transferred,
                total_bytes: total_size,
                bytes_per_second: 0,
                eta_seconds: 0,
                current_file: Some("Completed".to_string()),
                files_processed: final_files,
                total_files: total_files_count,
                speed: "0.00 B/s".to_string(),
                current_file_bytes: 0,
                current_file_total: 0,
                current_file_percent: 0.0,
                active_files: vec![],
            };
            let state = self.ws_state.read().await;
            state.broadcast(WsEvent::BackupProgress(payload));
        }

        // Send backup:completed event
        self.broadcast_event(WsEvent::BackupCompleted {
            job_id: job.job_id.clone(),
            total_bytes: final_transferred,
            total_files: final_files,
        }).await;

        Ok(BackupResult {
            total_files: final_files,
            total_bytes: final_transferred,
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

    /// Broadcast an event to all WebSocket clients
    async fn broadcast_event(&self, event: WsEvent) {
        let state = self.ws_state.read().await;
        state.broadcast(event);
    }
}

/// Upload a single file to the backup server
async fn upload_file(
    job_id: &str,
    server_url: &str,
    file_info: &FileInfo,
    file_state: &Arc<ActiveFileState>,
    cancel: &CancellationToken,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    // Open the file for reading
    let file = match tokio::fs::File::open(&file_info.path).await {
        Ok(f) => f,
        Err(e) => {
            error!("Failed to open file {}: {}", file_info.path.display(), e);
            return Err(Box::new(e));
        }
    };

    // Only compress files smaller than 500 MB to avoid memory issues
    const MAX_COMPRESS_SIZE: u64 = 500 * 1024 * 1024;
    let use_compression = file_info.size < MAX_COMPRESS_SIZE;

    let client = reqwest::Client::new();
    let upload_url = format!("{}/api/files/upload", server_url);

    // Progress callback updates the shared atomic
    let file_state_clone = Arc::clone(file_state);
    let progress_callback = Arc::new(move |bytes: u64| {
        file_state_clone.transferred.store(bytes, Ordering::Relaxed);
    });

    // Build the request (with or without compression)
    let request_future = if use_compression {
        use async_compression::tokio::bufread::ZstdEncoder;
        use tokio::io::BufReader;

        let buf_reader = BufReader::new(file);
        let compressed = ZstdEncoder::with_quality(buf_reader, async_compression::Level::Default);
        let stream = ReaderStream::new(compressed);
        let progress_stream = ProgressStream::new(stream, progress_callback);
        let body = reqwest::Body::wrap_stream(progress_stream);

        client
            .post(&upload_url)
            .header("x-job-id", job_id)
            .header("x-relative-path", file_info.relative_path.display().to_string())
            .header("x-total-size", file_info.size.to_string())
            .header("content-encoding", "zstd")
            .body(body)
            .send()
    } else {
        let stream = ReaderStream::new(file);
        let progress_stream = ProgressStream::new(stream, progress_callback);
        let body = reqwest::Body::wrap_stream(progress_stream);

        client
            .post(&upload_url)
            .header("x-job-id", job_id)
            .header("x-relative-path", file_info.relative_path.display().to_string())
            .header("x-total-size", file_info.size.to_string())
            .body(body)
            .send()
    };

    // Execute with cancellation support
    let response = tokio::select! {
        result = request_future => result,
        _ = cancel.cancelled() => {
            info!("Upload cancelled for {}", file_info.path.display());
            return Err("Cancelled".into());
        }
    };

    match response {
        Ok(resp) if resp.status().is_success() => {
            // Mark file as fully transferred
            file_state.transferred.store(file_info.size, Ordering::Relaxed);

            info!(
                "Uploaded {} bytes: {}",
                file_info.size,
                file_info.path.display(),
            );
            Ok(file_info.size)
        }
        Ok(resp) => {
            let status = resp.status();
            let error_text = resp.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            error!(
                "Upload failed with status {}: {}. Error: {}",
                status,
                file_info.path.display(),
                error_text
            );
            Err(format!("Upload failed: {} - {}", status, error_text).into())
        }
        Err(e) => {
            error!(
                "Upload request failed: {}. Error: {}",
                file_info.path.display(),
                e
            );
            Err(Box::new(e))
        }
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

    #[test]
    fn test_concurrency_weight() {
        assert_eq!(concurrency_weight(0), 1);              // 0 bytes → 1 permit (64 concurrent)
        assert_eq!(concurrency_weight(512_000), 1);        // 500 KB → 1 permit (64 concurrent)
        assert_eq!(concurrency_weight(1_048_576), 1);      // 1 MB → 1 permit (64 concurrent)
        assert_eq!(concurrency_weight(5_000_000), 1);      // 5 MB → 1 permit (64 concurrent)
        assert_eq!(concurrency_weight(50_000_000), 2);     // 50 MB → 2 permits (32 concurrent)
        assert_eq!(concurrency_weight(200_000_000), 16);   // 200 MB → 16 permits (4 concurrent)
        assert_eq!(concurrency_weight(800_000_000), 32);   // 800 MB → 32 permits (2 concurrent)
        assert_eq!(concurrency_weight(2_000_000_000), 64); // 2 GB → 64 permits (1 concurrent)
    }
}
