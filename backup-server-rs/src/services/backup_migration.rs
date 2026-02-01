use crate::db::connection::DbPool;
use crate::models::{backup_job, backup_version};
use std::path::Path;

pub fn migrate_existing_backups(pool: &DbPool) -> anyhow::Result<()> {
    let conn = pool.get()?;
    let jobs = backup_job::find_all(&conn)?;

    for job in jobs {
        if let Err(e) = migrate_job(&conn, &job) {
            tracing::error!(job_id = %job.id, error = %e, "Failed to migrate backup");
        }
    }

    Ok(())
}

fn migrate_job(conn: &rusqlite::Connection, job: &backup_job::BackupJob) -> anyhow::Result<()> {
    let versions_dir = Path::new(&job.local_path).join("versions");
    if versions_dir.exists() {
        return Ok(()); // Already migrated
    }

    let job_path = Path::new(&job.local_path);
    if !job_path.exists() {
        return Ok(()); // No backup yet
    }

    let entries: Vec<_> = std::fs::read_dir(job_path)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != ".backup-meta.json")
        .collect();

    if entries.is_empty() {
        return Ok(()); // Empty
    }

    tracing::info!(job_id = %job.id, path = %job.local_path, "Migrating existing backup to versioned structure");

    std::fs::create_dir_all(&versions_dir)?;

    let timestamp = job
        .last_run_at
        .as_ref()
        .map(|t| {
            t.replace([':', '.'], "-")
                .replace('T', "_")
                .chars()
                .take(19)
                .collect::<String>()
        })
        .unwrap_or_else(|| "initial-migration".into());

    let version_path = versions_dir.join(&timestamp);
    std::fs::create_dir_all(&version_path)?;

    for entry in std::fs::read_dir(job_path)?.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".backup-meta.json" || name_str == "versions" {
            continue;
        }
        let dest = version_path.join(&name);
        std::fs::rename(entry.path(), dest)?;
    }

    let version = backup_version::create(
        conn,
        &backup_version::CreateVersionData {
            job_id: job.id.clone(),
            log_id: String::new(),
            version_timestamp: timestamp.clone(),
            local_path: version_path.to_string_lossy().to_string(),
        },
    )?;

    let completed_at = job
        .last_run_at
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    backup_version::update_fields(conn, &version.id, &[
        ("status", &"completed" as &dyn rusqlite::types::ToSql),
        ("completed_at", &completed_at as &dyn rusqlite::types::ToSql),
    ])?;

    // Create 'current' symlink
    let current_symlink = Path::new(&job.local_path).join("current");
    let target = Path::new("versions").join(&timestamp);
    let _ = std::os::unix::fs::symlink(&target, &current_symlink);

    tracing::info!(job_id = %job.id, version_id = %version.id, "Migration completed");
    Ok(())
}
