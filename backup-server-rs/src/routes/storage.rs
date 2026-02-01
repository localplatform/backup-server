use crate::error::AppError;
use crate::models::{backup_job, backup_version, server, settings};
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub fn router(state: Arc<AppState>) -> Router<Arc<AppState>> {
    Router::new()
        .route("/hierarchy", get(get_hierarchy))
        .route("/settings", get(get_settings).put(update_settings))
        .route("/browse", get(browse))
        .route("/disk-usage", get(disk_usage))
        .route("/browse-version", get(browse_version))
}

// ── Settings ──

async fn get_settings(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, AppError> {
    let db = state.db.clone();
    let backup_root = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        Ok::<_, anyhow::Error>(settings::get(&conn, "backup_root")?)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;
    Ok(Json(serde_json::json!({ "backup_root": backup_root })))
}

#[derive(Deserialize)]
struct UpdateSettingsBody {
    backup_root: String,
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpdateSettingsBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.backup_root.is_empty() {
        return Err(AppError::BadRequest("backup_root is required".into()));
    }

    let path = PathBuf::from(&body.backup_root);
    if !path.is_dir() {
        return Err(AppError::BadRequest("Path does not exist or is not a directory".into()));
    }

    let db = state.db.clone();
    let new_root = body.backup_root.clone();
    tokio::task::spawn_blocking(move || {
        let conn = db.get()?;

        let old_root = settings::get(&conn, "backup_root")?;

        // Move data if root changed
        if let Some(ref old) = old_root {
            if old != &new_root {
                std::fs::create_dir_all(&new_root).ok();
                if let Ok(entries) = std::fs::read_dir(old) {
                    for entry in entries.flatten() {
                        let dest = PathBuf::from(&new_root).join(entry.file_name());
                        let _ = std::fs::rename(entry.path(), dest);
                    }
                }

                // Update job paths
                let jobs = backup_job::find_all(&conn)?;
                for job in jobs {
                    if job.local_path.starts_with(old) {
                        let relative = &job.local_path[old.len()..];
                        let new_path = format!("{}{}", new_root, relative);
                        backup_job::update(&conn, &job.id, &backup_job::UpdateBackupJobRequest {
                            name: None,
                            remote_paths: None,
                            local_path: Some(new_path),
                            cron_schedule: None,
                            rsync_options: None,
                            max_parallel: None,
                            enabled: None,
                            max_versions: None,
                        })?;
                    }
                }
            }
        }

        settings::set(&conn, "backup_root", &new_root)?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    Ok(Json(serde_json::json!({ "backup_root": body.backup_root })))
}

// ── Browse ──

#[derive(Deserialize)]
struct BrowseQuery {
    path: Option<String>,
}

#[derive(Serialize)]
struct LocalEntry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    size: u64,
    #[serde(rename = "modifiedAt")]
    modified_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "backupMeta")]
    backup_meta: Option<serde_json::Value>,
}

fn assert_within_root(root: &str, sub_path: &str) -> Result<PathBuf, AppError> {
    let relative = sub_path.trim_start_matches('/');
    let resolved = PathBuf::from(root).join(relative).canonicalize()
        .map_err(|_| AppError::BadRequest("Path does not exist".into()))?;
    let root_canonical = PathBuf::from(root).canonicalize()
        .map_err(|_| AppError::BadRequest("Backup root does not exist".into()))?;
    if !resolved.starts_with(&root_canonical) {
        return Err(AppError::BadRequest("Access denied".into()));
    }
    Ok(resolved)
}

fn explore_local(root: &str, sub_path: &str) -> Result<Vec<LocalEntry>, AppError> {
    let resolved = assert_within_root(root, sub_path)?;

    let mut entries = Vec::new();
    let read_dir = std::fs::read_dir(&resolved)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to read directory: {}", e)))?;

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".backup-meta.json" {
            continue;
        }

        let ft = entry.file_type().unwrap_or_else(|_| std::fs::metadata(entry.path()).unwrap().file_type());
        let entry_type = if ft.is_dir() {
            "directory"
        } else if ft.is_file() {
            "file"
        } else if ft.is_symlink() {
            "symlink"
        } else {
            "other"
        };

        let (size, modified_at) = match std::fs::metadata(entry.path()) {
            Ok(meta) => {
                let modified = meta.modified()
                    .ok()
                    .map(|t| {
                        let dt: chrono::DateTime<chrono::Utc> = t.into();
                        dt.to_rfc3339()
                    })
                    .unwrap_or_default();
                (meta.len(), modified)
            }
            Err(_) => (0, String::new()),
        };

        let rel_path = format!("{}/{}", sub_path.trim_end_matches('/'), name);

        let backup_meta = if entry_type == "directory" {
            let meta_path = entry.path().join(".backup-meta.json");
            std::fs::read_to_string(meta_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
        } else {
            None
        };

        entries.push(LocalEntry {
            name,
            path: rel_path,
            entry_type: entry_type.into(),
            size,
            modified_at,
            backup_meta,
        });
    }

    entries.sort_by(|a, b| {
        let a_dir = a.entry_type == "directory";
        let b_dir = b.entry_type == "directory";
        match (a_dir, b_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    Ok(entries)
}

async fn browse(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BrowseQuery>,
) -> Result<Json<Vec<LocalEntry>>, AppError> {
    let sub_path = query.path.unwrap_or_else(|| "/".into());
    let db = state.db.clone();
    let backup_root = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        settings::get(&conn, "backup_root")
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let backup_root = backup_root.ok_or_else(|| AppError::BadRequest("Backup root not configured".into()))?;

    let entries = tokio::task::spawn_blocking(move || explore_local(&backup_root, &sub_path))
        .await
        .map_err(|e| anyhow::anyhow!(e))??;

    Ok(Json(entries))
}

// ── Disk Usage ──

#[derive(Serialize)]
struct DiskUsage {
    total: u64,
    used: u64,
    available: u64,
    #[serde(rename = "usedPercent")]
    used_percent: u64,
}

async fn disk_usage(State(state): State<Arc<AppState>>) -> Result<Json<DiskUsage>, AppError> {
    let db = state.db.clone();
    let backup_root = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        settings::get(&conn, "backup_root")
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let backup_root = backup_root.ok_or_else(|| AppError::BadRequest("Backup root not configured".into()))?;

    let usage = tokio::task::spawn_blocking(move || -> anyhow::Result<DiskUsage> {
        let output = std::process::Command::new("df")
            .args(["-B1", &backup_root])
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.trim().lines().collect();
        if lines.len() < 2 {
            anyhow::bail!("Unexpected df output");
        }
        let parts: Vec<&str> = lines[1].split_whitespace().collect();
        let total: u64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let used: u64 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        let available: u64 = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
        let used_percent = if total > 0 { (used * 100) / total } else { 0 };
        Ok(DiskUsage { total, used, available, used_percent })
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    Ok(Json(usage))
}

// ── Browse Version ──

#[derive(Deserialize)]
struct BrowseVersionQuery {
    version_id: Option<String>,
    path: Option<String>,
}

async fn browse_version(
    State(state): State<Arc<AppState>>,
    Query(query): Query<BrowseVersionQuery>,
) -> Result<Json<Vec<LocalEntry>>, AppError> {
    let version_id = query.version_id.ok_or_else(|| AppError::BadRequest("version_id query parameter required".into()))?;
    let sub_path = query.path.unwrap_or_else(|| "/".into());

    let db = state.db.clone();
    let vid = version_id.clone();
    let version = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        backup_version::find_by_id(&conn, &vid)
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    let version = version.ok_or_else(|| AppError::NotFound("Version not found".into()))?;
    let local_path = version.local_path;

    let entries = tokio::task::spawn_blocking(move || explore_local(&local_path, &sub_path))
        .await
        .map_err(|e| anyhow::anyhow!(e))??;

    Ok(Json(entries))
}

// ── Hierarchy ──

async fn get_hierarchy(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, AppError> {
    let db = state.db.clone();
    let hierarchy = tokio::task::spawn_blocking(move || {
        let conn = db.get()?;
        let servers = server::find_all(&conn)?;
        let mut result = Vec::new();

        for s in servers {
            let jobs = backup_job::find_by_server_id(&conn, &s.id)?;
            let mut jobs_with_versions = Vec::new();

            for job in jobs {
                let remote_paths: Vec<String> = serde_json::from_str(&job.remote_paths).unwrap_or_default();
                let versions = backup_version::find_by_job_id(&conn, &job.id)?;
                let total_size: i64 = versions.iter().map(|v| v.bytes_transferred).sum();

                let version_vals: Vec<serde_json::Value> = versions.iter().map(|v| {
                    serde_json::json!({
                        "id": v.id,
                        "job_id": v.job_id,
                        "version_timestamp": v.version_timestamp,
                        "local_path": v.local_path,
                        "status": v.status,
                        "bytes_transferred": v.bytes_transferred,
                        "files_transferred": v.files_transferred,
                        "created_at": v.created_at,
                        "completed_at": v.completed_at,
                    })
                }).collect();

                jobs_with_versions.push(serde_json::json!({
                    "id": job.id,
                    "name": job.name,
                    "remote_paths": remote_paths,
                    "local_path": job.local_path,
                    "versions": version_vals,
                    "totalSize": total_size,
                }));
            }

            let total_versions: usize = jobs_with_versions.iter()
                .map(|j| j["versions"].as_array().map(|a| a.len()).unwrap_or(0))
                .sum();

            result.push(serde_json::json!({
                "id": s.id,
                "name": s.name,
                "hostname": s.hostname,
                "port": s.port,
                "jobs": jobs_with_versions,
                "totalVersions": total_versions,
            }));
        }

        Ok::<_, anyhow::Error>(serde_json::json!({ "servers": result }))
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))??;

    Ok(Json(hierarchy))
}
