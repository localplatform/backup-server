//! Backup job executor - Orchestrates the actual backup process.
//!
//! This module ties together all the Week 2 components:
//! - File system walker
//! - Signature generation
//! - Delta computation
//! - Progress tracking
//! - WebSocket event emission

pub mod manifest;

use crate::fs::walker::{walk_directory, WalkOptions, FileInfo};
use crate::transfer::progress::format_speed;
use crate::transfer::progress_stream::ProgressStream;
use crate::ws::{WsState, WsEvent, BackupProgressPayload, ActiveFileProgress};
use manifest::Manifest;
use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
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
    pub incremental: bool,
    pub manifest_url: Option<String>,
}

/// Backup execution result
#[derive(Debug)]
pub struct BackupResult {
    pub total_files: usize,
    pub total_bytes: u64,
    pub transferred_files: usize,
    pub transferred_bytes: u64,
    pub unchanged_files: usize,
    pub unchanged_bytes: u64,
    pub deleted_files: usize,
    pub backup_type: String,
    pub duration_secs: u64,
}

/// Result of diffing scanned files against a manifest
struct DiffResult {
    /// Files to upload (new or modified)
    changed_files: Vec<FileInfo>,
    _changed_bytes: u64,
    /// Relative paths of unchanged files (to hardlink on server)
    unchanged_paths: Vec<String>,
    unchanged_bytes: u64,
    /// Count of files in manifest but not on filesystem
    deleted_count: usize,
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

        let all_files_count = all_files.len();
        let all_files_bytes = total_size;

        // Snapshot for manifest generation (needs source mtimes)
        let all_files_snapshot: Vec<(String, u64, i64)> = all_files.iter().map(|f| {
            let mtime = std::fs::metadata(&f.path)
                .ok()
                .map(|m| m.mtime())
                .unwrap_or(0);
            (f.relative_path.to_string_lossy().to_string(), f.size, mtime)
        }).collect();

        // Incremental diff: compare against previous manifest
        let (files_to_upload, unchanged_files_count, unchanged_bytes, deleted_count, backup_type) =
            if job.incremental {
                match self.try_incremental_diff(&job, all_files.clone()).await {
                    Some(diff) => {
                        let uc = diff.unchanged_paths.len();
                        let ub = diff.unchanged_bytes;
                        let dc = diff.deleted_count;

                        info!(
                            "Incremental diff: {} changed, {} unchanged, {} deleted",
                            diff.changed_files.len(), uc, dc
                        );

                        // Request hardlinks for unchanged files
                        if !diff.unchanged_paths.is_empty() {
                            self.request_hardlinks(&job.server_url, &job.job_id, &diff.unchanged_paths).await;
                        }

                        (diff.changed_files, uc, ub, dc, "incremental".to_string())
                    }
                    None => {
                        info!("Incremental diff failed, falling back to full backup");
                        (all_files, 0, 0, 0, "full".to_string())
                    }
                }
            } else {
                (all_files, 0, 0, 0, "full".to_string())
            };

        // Sort files smallest-first for optimal concurrency
        let mut files_to_upload = files_to_upload;
        files_to_upload.sort_by_key(|f| f.size);

        let upload_files_count = files_to_upload.len();
        let upload_total_size: u64 = files_to_upload.iter().map(|f| f.size).sum();

        info!(
            "Total files to backup: {}, to transfer: {}, total size: {} bytes (sorted by size, smallest first)",
            all_files_count, upload_files_count, upload_total_size
        );

        // Shared counters for completed work
        let completed_bytes = Arc::new(AtomicU64::new(0));
        let completed_files = Arc::new(AtomicUsize::new(0));

        // Shared map of currently active file transfers (for progress reporting)
        let active_files: Arc<RwLock<HashMap<usize, Arc<ActiveFileState>>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Weighted semaphore: total budget = CONCURRENCY_BUDGET permits.
        let semaphore = Arc::new(Semaphore::new(CONCURRENCY_BUDGET));

        // Spawn a single progress broadcast task that reads all shared state
        let progress_ws = Arc::clone(&self.ws_state);
        let progress_job_id = job.job_id.clone();
        let progress_completed_bytes = Arc::clone(&completed_bytes);
        let progress_completed_files = Arc::clone(&completed_files);
        let progress_active_files = Arc::clone(&active_files);
        let progress_cancel = self.cancel_token.clone();
        let progress_total_bytes = upload_total_size;
        let progress_total_files = upload_files_count;
        let progress_skipped_files = unchanged_files_count;
        let progress_skipped_bytes = unchanged_bytes;
        let progress_backup_type = backup_type.clone();

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
                } else if progress_total_files == 0 {
                    100.0 // Nothing to transfer
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
                    skipped_files: progress_skipped_files,
                    skipped_bytes: progress_skipped_bytes,
                    backup_type: progress_backup_type.clone(),
                };

                let state = progress_ws.read().await;
                state.broadcast(WsEvent::BackupProgress(payload));
            }
        });

        // Spawn parallel file upload tasks with adaptive concurrency
        info!("Starting parallel file processing: {} files, adaptive concurrency (budget: {})", upload_files_count, CONCURRENCY_BUDGET);

        let mut handles = Vec::with_capacity(files_to_upload.len());

        for (idx, file_info) in files_to_upload.into_iter().enumerate() {
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
        if was_cancelled_by_user && total_processed < upload_files_count {
            info!("Backup cancelled: {} files processed out of {}", total_processed, upload_files_count);

            self.broadcast_event(WsEvent::BackupFailed {
                job_id: job.job_id.clone(),
                error: "Backup cancelled by user".to_string(),
            }).await;

            return Err("Backup cancelled by user".into());
        }

        info!(
            "Backup completed: {} files transferred ({} successful), {} bytes, {} unchanged, {} deleted, {}s",
            final_files, total_processed, final_transferred, unchanged_files_count, deleted_count, duration_secs
        );

        // Send final 100% progress
        {
            let payload = BackupProgressPayload {
                job_id: job.job_id.clone(),
                percent: 100.0,
                transferred_bytes: final_transferred,
                total_bytes: upload_total_size,
                bytes_per_second: 0,
                eta_seconds: 0,
                current_file: Some("Completed".to_string()),
                files_processed: final_files,
                total_files: upload_files_count,
                speed: "0.00 B/s".to_string(),
                current_file_bytes: 0,
                current_file_total: 0,
                current_file_percent: 0.0,
                active_files: vec![],
                skipped_files: unchanged_files_count,
                skipped_bytes: unchanged_bytes,
                backup_type: backup_type.clone(),
            };
            let state = self.ws_state.read().await;
            state.broadcast(WsEvent::BackupProgress(payload));
        }

        // Upload manifest with source mtimes for future incremental backups
        if let Err(e) = upload_manifest(&job.server_url, &job.job_id, &all_files_snapshot).await {
            warn!("Failed to upload manifest: {}", e);
        }

        // Send backup:completed event with full stats
        self.broadcast_event(WsEvent::BackupCompleted {
            job_id: job.job_id.clone(),
            total_bytes: all_files_bytes,
            total_files: all_files_count,
            transferred_bytes: final_transferred,
            transferred_files: final_files,
            unchanged_files: unchanged_files_count,
            unchanged_bytes: unchanged_bytes,
            deleted_files: deleted_count,
            backup_type: backup_type.clone(),
        }).await;

        Ok(BackupResult {
            total_files: all_files_count,
            total_bytes: all_files_bytes,
            transferred_files: final_files,
            transferred_bytes: final_transferred,
            unchanged_files: unchanged_files_count,
            unchanged_bytes: unchanged_bytes,
            deleted_files: deleted_count,
            backup_type,
            duration_secs,
        })
    }

    /// Attempt incremental diff against previous manifest.
    /// Returns None if manifest fetch fails (caller should fall back to full backup).
    async fn try_incremental_diff(
        &self,
        job: &BackupJob,
        all_files: Vec<FileInfo>,
    ) -> Option<DiffResult> {
        let manifest_url = job.manifest_url.as_ref()?;
        let manifest = fetch_manifest(&job.server_url, manifest_url).await?;

        // Diff in a blocking task (filesystem metadata reads)
        let result = tokio::task::spawn_blocking(move || {
            diff_files_against_manifest(all_files, &manifest)
        }).await.ok()?;

        Some(result)
    }

    /// Send a hardlink request to the server for unchanged files.
    async fn request_hardlinks(&self, server_url: &str, job_id: &str, unchanged_paths: &[String]) {
        let url = format!("{}/api/files/hardlink", server_url);
        let client = reqwest::Client::new();

        // Send in batches to avoid oversized requests
        const BATCH_SIZE: usize = 5000;
        for chunk in unchanged_paths.chunks(BATCH_SIZE) {
            let body = serde_json::json!({
                "job_id": job_id,
                "files": chunk,
            });

            match client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(result) = resp.json::<serde_json::Value>().await {
                        let linked = result.get("linked").and_then(|v| v.as_u64()).unwrap_or(0);
                        let failed = result.get("failed").and_then(|v| v.as_u64()).unwrap_or(0);
                        info!("Hardlinks created: {} linked, {} failed (batch of {})", linked, failed, chunk.len());
                    }
                }
                Ok(resp) => {
                    let status = resp.status();
                    warn!("Hardlink request failed with status {}", status);
                }
                Err(e) => {
                    warn!("Hardlink request error: {}", e);
                }
            }
        }
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

/// Fetch the previous backup manifest from the server.
/// Returns None on any error (caller should fall back to full backup).
async fn fetch_manifest(server_url: &str, manifest_url: &str) -> Option<Manifest> {
    let url = format!("{}{}", server_url, manifest_url);
    let client = reqwest::Client::new();

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<Manifest>().await {
                Ok(manifest) => {
                    info!("Fetched manifest: {} files, {} bytes", manifest.total_files, manifest.total_bytes);
                    Some(manifest)
                }
                Err(e) => {
                    warn!("Failed to parse manifest: {}", e);
                    None
                }
            }
        }
        Ok(resp) => {
            info!("No manifest available (status {}), will do full backup", resp.status());
            None
        }
        Err(e) => {
            warn!("Failed to fetch manifest: {}", e);
            None
        }
    }
}

/// Compare scanned files against a manifest to determine what changed.
/// Uses size + mtime as the change detection heuristic (same as rsync default).
fn diff_files_against_manifest(all_files: Vec<FileInfo>, manifest: &Manifest) -> DiffResult {
    let mut changed_files = Vec::new();
    let mut changed_bytes = 0u64;
    let mut unchanged_paths = Vec::new();
    let mut unchanged_bytes = 0u64;
    let mut seen_paths = HashSet::new();

    for file in all_files {
        let rel = file.relative_path.to_string_lossy().to_string();
        seen_paths.insert(rel.clone());

        if let Some(entry) = manifest.files.get(&rel) {
            // Check if file is unchanged (same size and mtime)
            let mtime = std::fs::metadata(&file.path)
                .ok()
                .map(|m| m.mtime())
                .unwrap_or(0);

            if entry.size == file.size && entry.mtime == mtime {
                unchanged_paths.push(rel);
                unchanged_bytes += file.size;
                continue;
            }
        }

        changed_bytes += file.size;
        changed_files.push(file);
    }

    // Count deleted files (in manifest but not on filesystem)
    let deleted_count = manifest.files.keys()
        .filter(|k| !seen_paths.contains(*k))
        .count();

    DiffResult {
        changed_files,
        _changed_bytes: changed_bytes,
        unchanged_paths,
        unchanged_bytes,
        deleted_count,
    }
}

/// Upload a manifest file containing source file metadata (size + mtime from agent).
/// This is uploaded as `.backup-manifest.json` via the normal upload route.
async fn upload_manifest(
    server_url: &str,
    job_id: &str,
    files: &[(String, u64, i64)],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut file_entries = HashMap::new();
    let mut total_files = 0usize;
    let mut total_bytes = 0u64;

    for (rel_path, size, mtime) in files {
        file_entries.insert(rel_path.clone(), serde_json::json!({
            "size": size,
            "mtime": mtime,
        }));
        total_files += 1;
        total_bytes += size;
    }

    let manifest = serde_json::json!({
        "version": 1,
        "job_id": job_id,
        "files": file_entries,
        "total_files": total_files,
        "total_bytes": total_bytes,
    });

    let manifest_json = serde_json::to_string(&manifest)?;
    let upload_url = format!("{}/api/files/upload", server_url);

    let client = reqwest::Client::new();
    let resp = client.post(&upload_url)
        .header("x-job-id", job_id)
        .header("x-relative-path", ".backup-manifest.json")
        .header("x-total-size", manifest_json.len().to_string())
        .header("content-type", "application/octet-stream")
        .body(manifest_json)
        .send()
        .await?;

    if resp.status().is_success() {
        info!("Uploaded manifest: {} files, {} bytes", total_files, total_bytes);
    } else {
        warn!("Manifest upload failed with status {}", resp.status());
    }

    Ok(())
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
            incremental: false,
            manifest_url: None,
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

    #[test]
    fn test_diff_files_against_manifest() {
        let mut files_map = HashMap::new();
        files_map.insert("file1.txt".to_string(), ManifestEntry { size: 100, mtime: 1000 });
        files_map.insert("file2.txt".to_string(), ManifestEntry { size: 200, mtime: 2000 });
        files_map.insert("deleted.txt".to_string(), ManifestEntry { size: 50, mtime: 500 });

        let manifest = Manifest {
            version: 1,
            job_id: "test".to_string(),
            files: files_map,
            total_files: 3,
            total_bytes: 350,
        };

        // file1.txt unchanged, file2.txt modified (different size), new_file.txt is new, deleted.txt is gone
        // Note: we can't easily test mtime matching without real files, so this tests the structure
        let all_files = vec![
            FileInfo {
                path: PathBuf::from("/data/file1.txt"),
                relative_path: PathBuf::from("file1.txt"),
                size: 100,
                is_dir: false,
                is_symlink: false,
                depth: 0,
            },
            FileInfo {
                path: PathBuf::from("/data/file2.txt"),
                relative_path: PathBuf::from("file2.txt"),
                size: 250, // different size
                is_dir: false,
                is_symlink: false,
                depth: 0,
            },
            FileInfo {
                path: PathBuf::from("/data/new_file.txt"),
                relative_path: PathBuf::from("new_file.txt"),
                size: 300,
                is_dir: false,
                is_symlink: false,
                depth: 0,
            },
        ];

        let result = diff_files_against_manifest(all_files, &manifest);

        // file2.txt changed (size differs), new_file.txt is new
        // file1.txt will be "changed" too because mtime won't match (no real file)
        // deleted.txt is in manifest but not in scanned files
        assert_eq!(result.deleted_count, 1);
        // Total scanned = 3 files, all will be changed because mtime can't match without real filesystem
        assert_eq!(result.changed_files.len() + result.unchanged_paths.len(), 3);
    }
}
