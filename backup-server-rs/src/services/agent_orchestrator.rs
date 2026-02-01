use crate::db::connection::DbPool;
use crate::models::{backup_job, backup_version, server};
use crate::state::AppState;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub async fn run_backup_job(state: Arc<AppState>, job_id: String) -> anyhow::Result<()> {
    // Check if already running
    {
        let mut running = state.running_jobs.lock().await;
        if running.contains(&job_id) {
            anyhow::bail!("Job already running");
        }
        running.insert(job_id.clone());
    }

    let result = run_backup_inner(state.clone(), &job_id).await;

    // Always remove from running set
    {
        let mut running = state.running_jobs.lock().await;
        running.remove(&job_id);
    }

    result
}

async fn run_backup_inner(state: Arc<AppState>, job_id: &str) -> anyhow::Result<()> {
    let db = state.db.clone();
    let jid = job_id.to_string();

    // Load job and server
    let (job, srv) = tokio::task::spawn_blocking({
        let db = db.clone();
        let jid = jid.clone();
        move || {
            let conn = db.get()?;
            let job = backup_job::find_by_id(&conn, &jid)?
                .ok_or_else(|| anyhow::anyhow!("Job not found"))?;
            let srv = server::find_by_id(&conn, &job.server_id)?
                .ok_or_else(|| anyhow::anyhow!("Server not found"))?;
            Ok::<_, anyhow::Error>((job, srv))
        }
    })
    .await??;

    if !state.agents.is_connected(&srv.id) {
        anyhow::bail!("Agent is not connected");
    }

    let remote_paths: Vec<String> = serde_json::from_str(&job.remote_paths).unwrap_or_default();
    if remote_paths.is_empty() {
        anyhow::bail!("No remote paths configured");
    }

    // Update status to running
    let db2 = db.clone();
    let jid2 = jid.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db2.get()?;
        backup_job::update_status(&conn, &jid2, "running")
    })
    .await??;

    // Create log entry
    let db3 = db.clone();
    let jid3 = jid.clone();
    let log = tokio::task::spawn_blocking(move || {
        let conn = db3.get()?;
        backup_job::create_log(&conn, &jid3)
    })
    .await??;

    let start_time = std::time::Instant::now();

    state.ui.broadcast("backup:started", serde_json::json!({
        "jobId": jid,
        "serverId": srv.id,
        "remotePaths": remote_paths,
    }));

    state.ui.broadcast("backup:progress", serde_json::json!({
        "jobId": jid,
        "percent": 0,
        "checkedFiles": 0,
        "totalFiles": 0,
        "transferredBytes": 0,
        "totalBytes": 0,
        "speed": "",
        "currentFile": "Initializing agent backup...",
    }));

    // Generate version timestamp
    let now = chrono::Utc::now();
    let version_timestamp = now.format("%Y-%m-%d_%H-%M-%S").to_string();
    let versions_dir = std::path::PathBuf::from(&job.local_path).join("versions");
    let version_path = versions_dir.join(&version_timestamp);
    tokio::fs::create_dir_all(&version_path).await?;

    // Write backup metadata
    let meta_path = std::path::PathBuf::from(&job.local_path).join(".backup-meta.json");
    let meta = serde_json::json!({
        "server": { "name": srv.name, "hostname": srv.hostname, "port": srv.port },
        "job": { "id": job.id, "name": job.name, "remotePaths": remote_paths },
        "agent": { "enabled": true },
        "createdAt": job.created_at,
        "lastRunAt": chrono::Utc::now().to_rfc3339(),
    });
    tokio::fs::write(&meta_path, serde_json::to_string_pretty(&meta)?).await?;

    // Create version record
    let db4 = db.clone();
    let vp = version_path.to_string_lossy().to_string();
    let jid4 = jid.clone();
    let log_id = log.id.clone();
    let vt = version_timestamp.clone();
    let version = tokio::task::spawn_blocking(move || {
        let conn = db4.get()?;
        backup_version::create(&conn, &backup_version::CreateVersionData {
            job_id: jid4,
            log_id,
            version_timestamp: vt,
            local_path: vp,
        })
    })
    .await??;

    // Acquire semaphores
    let _global_permit = state.global_semaphore.acquire().await?;
    let server_sem = state.get_server_semaphore(&srv.id).await;
    let _server_permit = server_sem.acquire().await?;

    // Determine if incremental backup is possible (automatic: manifest exists → incremental)
    let incremental = {
        let db_inc = db.clone();
        let jid_inc = jid.clone();
        tokio::task::spawn_blocking(move || {
            let conn = db_inc.get()?;
            if let Some(prev) = backup_version::find_latest_completed(&conn, &jid_inc)? {
                let manifest_path = std::path::PathBuf::from(&prev.local_path)
                    .join(".backup-manifest.json");
                Ok::<_, anyhow::Error>(manifest_path.exists())
            } else {
                Ok(false)
            }
        })
        .await
        .unwrap_or(Ok(false))
        .unwrap_or(false)
    };

    let backup_type = if incremental { "incremental" } else { "full" };
    tracing::info!(job_id = %jid, ?remote_paths, backup_type, "Starting agent backup via WebSocket");

    // Send backup start command
    let mut payload = serde_json::json!({
        "job_id": jid,
        "paths": remote_paths,
        "incremental": incremental,
    });
    if incremental {
        payload.as_object_mut().unwrap().insert(
            "manifest_url".to_string(),
            serde_json::json!(format!("/api/files/manifest/{}", jid)),
        );
    }

    let sent = state.agents.send_to_agent(&srv.id, serde_json::json!({
        "type": "backup:start",
        "payload": payload,
    }));

    if !sent {
        fail_backup(&state, &db, &jid, &log.id, &version.id).await;
        anyhow::bail!("Failed to send backup command to agent");
    }

    // Wait for completion via polling (agent messages are forwarded through the WS handler)
    // We use a channel-based approach: listen for completion/failure events
    #[derive(Debug)]
    struct BackupCompletionStats {
        total_bytes: i64,
        total_files: i64,
        transferred_bytes: i64,
        transferred_files: i64,
        unchanged_files: i64,
        unchanged_bytes: i64,
        deleted_files: i64,
        backup_type: String,
    }
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<Result<BackupCompletionStats, String>>();
    let done_tx = Arc::new(tokio::sync::Mutex::new(Some(done_tx)));

    // Spawn a timeout + polling task
    let state2 = state.clone();
    let jid5 = jid.clone();
    let sid = srv.id.clone();
    let db5 = db.clone();
    let done_tx2 = done_tx.clone();
    let monitor = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let timeout = tokio::time::sleep(std::time::Duration::from_secs(3600));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    if let Some(tx) = done_tx2.lock().await.take() {
                        let _ = tx.send(Err("Backup timed out after 1 hour".into()));
                    }
                    break;
                }
                _ = interval.tick() => {
                    // Check if job was cancelled
                    let db = db5.clone();
                    let jid = jid5.clone();
                    if let Ok(Ok(Some(j))) = tokio::task::spawn_blocking(move || {
                        let conn = db.get()?;
                        backup_job::find_by_id(&conn, &jid)
                    }).await {
                        if j.status == "cancelled" {
                            state2.agents.send_to_agent(&sid, serde_json::json!({
                                "type": "backup:cancel",
                                "payload": { "job_id": jid5 },
                            }));
                            if let Some(tx) = done_tx2.lock().await.take() {
                                let _ = tx.send(Err("Job cancelled by user".into()));
                            }
                            break;
                        }
                    }

                    // Check agent connection
                    if !state2.agents.is_connected(&sid) {
                        if let Some(tx) = done_tx2.lock().await.take() {
                            let _ = tx.send(Err("Agent disconnected during backup".into()));
                        }
                        break;
                    }
                }
            }
        }
    });

    // Listen for backup:completed and backup:failed from the UI broadcast
    let mut rx = state.ui.subscribe();
    let jid6 = jid.clone();
    let done_tx3 = done_tx.clone();
    let event_listener = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&msg) {
                let msg_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let payload = parsed.get("payload").cloned().unwrap_or_default();
                let msg_job_id = payload.get("jobId").or(payload.get("job_id"))
                    .and_then(|v| v.as_str()).unwrap_or("");

                if msg_job_id != jid6 {
                    continue;
                }

                match msg_type {
                    "backup:completed" => {
                        let total_bytes = payload.get("totalBytes").or(payload.get("total_bytes"))
                            .and_then(|v| v.as_i64()).unwrap_or(0);
                        let total_files = payload.get("totalFiles").or(payload.get("total_files"))
                            .and_then(|v| v.as_i64()).unwrap_or(0);
                        let transferred_bytes = payload.get("transferredBytes").or(payload.get("transferred_bytes"))
                            .and_then(|v| v.as_i64()).unwrap_or(total_bytes);
                        let transferred_files = payload.get("transferredFiles").or(payload.get("transferred_files"))
                            .and_then(|v| v.as_i64()).unwrap_or(total_files);
                        let unchanged_files = payload.get("unchangedFiles").or(payload.get("unchanged_files"))
                            .and_then(|v| v.as_i64()).unwrap_or(0);
                        let unchanged_bytes = payload.get("unchangedBytes").or(payload.get("unchanged_bytes"))
                            .and_then(|v| v.as_i64()).unwrap_or(0);
                        let deleted_files = payload.get("deletedFiles").or(payload.get("deleted_files"))
                            .and_then(|v| v.as_i64()).unwrap_or(0);
                        let backup_type = payload.get("backupType").or(payload.get("backup_type"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("full")
                            .to_string();
                        if let Some(tx) = done_tx3.lock().await.take() {
                            let _ = tx.send(Ok(BackupCompletionStats {
                                total_bytes,
                                total_files,
                                transferred_bytes,
                                transferred_files,
                                unchanged_files,
                                unchanged_bytes,
                                deleted_files,
                                backup_type,
                            }));
                        }
                        break;
                    }
                    "backup:failed" => {
                        let error = payload.get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Backup failed on agent")
                            .to_string();
                        if let Some(tx) = done_tx3.lock().await.take() {
                            let _ = tx.send(Err(error));
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    });

    // Wait for result
    let result = done_rx.await.map_err(|_| anyhow::anyhow!("Monitor channel closed"))?;

    // Cleanup monitor tasks
    monitor.abort();
    event_listener.abort();

    match result {
        Ok(stats) => {
            let duration_secs = start_time.elapsed().as_secs() as i64;
            let total_bytes = stats.total_bytes;
            let total_files = stats.total_files;
            let backup_type = stats.backup_type.clone();
            let transferred_bytes = stats.transferred_bytes;
            let transferred_files = stats.transferred_files;
            let unchanged_files = stats.unchanged_files;
            let unchanged_bytes = stats.unchanged_bytes;
            let deleted_files = stats.deleted_files;

            // Update job status
            let db_c = db.clone();
            let jid_c = jid.clone();
            let log_id = log.id.clone();
            let vid = version.id.clone();
            let completion_data = backup_version::CompletionData {
                bytes_transferred: stats.transferred_bytes,
                files_transferred: stats.transferred_files,
                backup_type: stats.backup_type.clone(),
                files_unchanged: stats.unchanged_files,
                bytes_unchanged: stats.unchanged_bytes,
                files_deleted: stats.deleted_files,
            };
            tokio::task::spawn_blocking(move || {
                let conn = db_c.get()?;
                backup_job::update_status(&conn, &jid_c, "completed")?;
                backup_job::update_log(&conn, &log_id, &[
                    ("status", &"completed" as &dyn rusqlite::types::ToSql),
                    ("files_transferred", &stats.transferred_files as &dyn rusqlite::types::ToSql),
                    ("bytes_transferred", &stats.transferred_bytes as &dyn rusqlite::types::ToSql),
                    ("finished_at", &chrono::Utc::now().to_rfc3339() as &dyn rusqlite::types::ToSql),
                ])?;
                backup_version::update_completion_incremental(&conn, &vid, &completion_data)?;
                Ok::<_, anyhow::Error>(())
            })
            .await??;

            // Manifest is uploaded by the agent with correct source mtimes.
            // Wait briefly for the upload to complete (agent sends it via HTTP
            // right before the BackupCompleted WebSocket event).
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            // Fallback: generate server-side manifest if agent didn't upload one
            let manifest_vp = version_path.clone();
            let manifest_jid = jid.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let manifest_path = manifest_vp.join(".backup-manifest.json");
                if !manifest_path.exists() {
                    tracing::info!(job_id = %manifest_jid, "No agent manifest found, generating server-side manifest (fallback)");
                    if let Err(e) = generate_manifest(&manifest_vp, &manifest_jid) {
                        tracing::warn!(job_id = %manifest_jid, "Failed to generate manifest: {}", e);
                    }
                }
            }).await;

            // Cleanup old versions
            let max_versions = job.max_versions;
            cleanup_old_versions(db.clone(), jid.clone(), max_versions).await;

            state.ui.broadcast("backup:completed", serde_json::json!({
                "jobId": jid,
                "totalBytes": total_bytes,
                "totalFiles": total_files,
                "duration": duration_secs,
                "backupType": backup_type,
                "unchangedFiles": unchanged_files,
                "unchangedBytes": unchanged_bytes,
                "deletedFiles": deleted_files,
            }));

            state.ui.broadcast("backup:progress", serde_json::json!({
                "jobId": jid,
                "percent": 100,
                "checkedFiles": total_files,
                "totalFiles": total_files,
                "transferredBytes": transferred_bytes,
                "totalBytes": total_bytes,
                "speed": "",
                "currentFile": "Completed",
                "backupType": backup_type,
                "skippedFiles": unchanged_files,
                "skippedBytes": unchanged_bytes,
            }));

            tracing::info!(
                job_id = %jid,
                total_bytes,
                total_files,
                duration_secs,
                "Backup job completed"
            );

            Ok(())
        }
        Err(error_msg) => {
            fail_backup(&state, &db, &jid, &log.id, &version.id).await;
            state.ui.broadcast("backup:failed", serde_json::json!({
                "jobId": jid,
                "error": error_msg,
            }));
            tracing::error!(job_id = %jid, error = %error_msg, "Backup job failed");
            anyhow::bail!("{}", error_msg)
        }
    }
}

async fn fail_backup(state: &AppState, db: &DbPool, job_id: &str, log_id: &str, version_id: &str) {
    let db = db.clone();
    let jid = job_id.to_string();
    let lid = log_id.to_string();
    let vid = version_id.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_job::update_status(&conn, &jid, "failed")?;
        backup_job::update_log(&conn, &lid, &[
            ("status", &"failed" as &dyn rusqlite::types::ToSql),
            ("finished_at", &chrono::Utc::now().to_rfc3339() as &dyn rusqlite::types::ToSql),
        ])?;
        backup_version::update_failed(&conn, &vid)?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
}

pub async fn cancel_backup_job(state: Arc<AppState>, job_id: &str) -> anyhow::Result<()> {
    tracing::info!(job_id, "Cancelling backup job");

    let db = state.db.clone();
    let jid = job_id.to_string();
    let (job, srv) = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        let job = backup_job::find_by_id(&conn, &jid)?
            .ok_or_else(|| anyhow::anyhow!("Job not found"))?;
        let srv = server::find_by_id(&conn, &job.server_id)?
            .ok_or_else(|| anyhow::anyhow!("Server not found"))?;
        Ok::<_, anyhow::Error>((job, srv))
    })
    .await??;

    if state.agents.is_connected(&srv.id) {
        state.agents.send_to_agent(&srv.id, serde_json::json!({
            "type": "backup:cancel",
            "payload": { "job_id": job_id },
        }));
        tracing::info!(job_id, "Sent cancel command to agent");
    }

    let db = state.db.clone();
    let jid = job_id.to_string();
    tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_job::update_status(&conn, &jid, "cancelled")
    })
    .await??;

    state.ui.broadcast("backup:cancelled", serde_json::json!({ "jobId": job_id }));

    {
        let mut running = state.running_jobs.lock().await;
        running.remove(job_id);
    }

    Ok(())
}

/// Generate a `.backup-manifest.json` file in the version directory.
/// The manifest records every file with its relative path, size, and mtime
/// so future incremental backups can diff against it.
fn generate_manifest(version_path: &Path, job_id: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let mut files: HashMap<String, serde_json::Value> = HashMap::new();
    let mut total_files: usize = 0;
    let mut total_bytes: u64 = 0;

    fn walk_recursive(
        dir: &Path,
        root: &Path,
        files: &mut HashMap<String, serde_json::Value>,
        total_files: &mut usize,
        total_bytes: &mut u64,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;

            if file_type.is_dir() {
                walk_recursive(&path, root, files, total_files, total_bytes)?;
            } else if file_type.is_file() {
                let metadata = std::fs::metadata(&path)?;
                let relative = path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string();

                // Skip the manifest file itself
                if relative == ".backup-manifest.json" {
                    continue;
                }

                let size = metadata.len();
                let mtime = metadata.mtime();

                files.insert(relative, serde_json::json!({
                    "size": size,
                    "mtime": mtime,
                }));
                *total_files += 1;
                *total_bytes += size;
            }
        }
        Ok(())
    }

    walk_recursive(version_path, version_path, &mut files, &mut total_files, &mut total_bytes)?;

    let manifest = serde_json::json!({
        "version": 1,
        "job_id": job_id,
        "files": files,
        "total_files": total_files,
        "total_bytes": total_bytes,
    });

    let manifest_path = version_path.join(".backup-manifest.json");
    std::fs::write(&manifest_path, serde_json::to_string(&manifest)?)?;

    tracing::info!(
        job_id = %job_id,
        total_files,
        total_bytes,
        "Generated backup manifest"
    );

    Ok(())
}

/// Backfill `.backup-manifest.json` for completed versions that don't have one.
/// This is a startup migration for versions completed before manifest generation was added.
pub fn backfill_manifests(db: &DbPool) {
    let conn = match db.get() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to get DB connection for manifest backfill: {}", e);
            return;
        }
    };

    let all_jobs = match backup_job::find_all(&conn) {
        Ok(jobs) => jobs,
        Err(e) => {
            tracing::warn!("Failed to list jobs for manifest backfill: {}", e);
            return;
        }
    };

    let mut backfilled = 0;
    for job in &all_jobs {
        let versions = match backup_version::find_by_job_id(&conn, &job.id) {
            Ok(v) => v,
            Err(_) => continue,
        };

        for v in &versions {
            if v.status != "completed" {
                continue;
            }
            let manifest_path = std::path::PathBuf::from(&v.local_path)
                .join(".backup-manifest.json");
            if manifest_path.exists() {
                continue;
            }
            // Version directory must exist
            if !Path::new(&v.local_path).is_dir() {
                continue;
            }
            match generate_manifest(Path::new(&v.local_path), &v.job_id) {
                Ok(()) => {
                    backfilled += 1;
                    tracing::info!(
                        job_id = %v.job_id,
                        version = %v.version_timestamp,
                        "Backfilled manifest for existing version"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        job_id = %v.job_id,
                        version = %v.version_timestamp,
                        "Failed to backfill manifest: {}", e
                    );
                }
            }
        }
    }

    if backfilled > 0 {
        tracing::info!("Backfilled {} manifests for existing versions", backfilled);
    }
}

async fn cleanup_old_versions(db: DbPool, job_id: String, max_versions: i64) {
    let _ = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        let versions = backup_version::find_by_job_id(&conn, &job_id)?;
        let mut completed: Vec<_> = versions.into_iter().filter(|v| v.status == "completed").collect();
        completed.sort_by(|a, b| b.version_timestamp.cmp(&a.version_timestamp));

        if completed.len() as i64 <= max_versions {
            return Ok::<_, anyhow::Error>(());
        }

        let to_delete = &completed[max_versions as usize..];
        for v in to_delete {
            backup_version::delete(&conn, &v.id)?;
            let path = v.local_path.clone();
            // Spawn async removal (best effort)
            std::thread::spawn(move || {
                let _ = std::fs::remove_dir_all(&path);
            });
            tracing::info!(version_id = %v.id, job_id = %job_id, path = %v.local_path, "Deleted old backup version");
        }

        Ok(())
    })
    .await;
}
