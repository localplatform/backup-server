use crate::db::connection::DbPool;
use crate::models::{backup_job, backup_version, server, settings};
use std::path::Path;

fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn migrate_server_folder_names(pool: &DbPool) -> anyhow::Result<()> {
    let conn = pool.get()?;
    let backup_root = match settings::get(&conn, "backup_root")? {
        Some(r) => r,
        None => {
            tracing::info!("[PATH_MIGRATION] No backup root configured, skipping");
            return Ok(());
        }
    };

    tracing::info!("[PATH_MIGRATION] Starting server folder name migration...");

    let servers = server::find_all(&conn)?;
    let jobs = backup_job::find_all(&conn)?;
    let mut migrated = 0;
    let mut skipped = 0;

    for srv in &servers {
        let server_jobs: Vec<_> = jobs.iter().filter(|j| j.server_id == srv.id).collect();
        if server_jobs.is_empty() {
            continue;
        }

        let old_slug = slug(&srv.hostname);
        let new_slug = slug(&srv.name);

        if old_slug == new_slug {
            continue;
        }

        let old_path = Path::new(&backup_root).join(&old_slug);
        let new_path = Path::new(&backup_root).join(&new_slug);

        if !old_path.exists() {
            continue;
        }

        if new_path.exists() {
            tracing::warn!(
                server_id = %srv.id,
                old_path = %old_path.display(),
                new_path = %new_path.display(),
                "[PATH_MIGRATION] Target path exists, skipping"
            );
            skipped += 1;
            continue;
        }

        tracing::info!(
            server_id = %srv.id,
            old_path = %old_path.display(),
            new_path = %new_path.display(),
            "[PATH_MIGRATION] Migrating server folder"
        );

        match std::fs::rename(&old_path, &new_path) {
            Ok(()) => {
                let old_str = old_path.to_string_lossy().to_string();
                let new_str = new_path.to_string_lossy().to_string();

                for job in &server_jobs {
                    if job.local_path.starts_with(&old_str) {
                        let new_job_path = job.local_path.replace(&old_str, &new_str);
                        let _ = backup_job::update(&conn, &job.id, &backup_job::UpdateBackupJobRequest {
                            name: None,
                            remote_paths: None,
                            local_path: Some(new_job_path),
                            cron_schedule: None,
                            rsync_options: None,
                            max_parallel: None,
                            enabled: None,
                            max_versions: None,
                        });

                        if let Ok(versions) = backup_version::find_by_job_id(&conn, &job.id) {
                            for v in versions {
                                if v.local_path.starts_with(&old_str) {
                                    let new_v_path = v.local_path.replace(&old_str, &new_str);
                                    let _ = backup_version::update_fields(&conn, &v.id, &[
                                        ("local_path", &new_v_path as &dyn rusqlite::types::ToSql),
                                    ]);
                                }
                            }
                        }
                    }
                }

                migrated += 1;
            }
            Err(e) => {
                tracing::error!(
                    server_id = %srv.id,
                    error = %e,
                    "[PATH_MIGRATION] Failed to rename folder"
                );
                skipped += 1;
            }
        }
    }

    tracing::info!(migrated, skipped, "[PATH_MIGRATION] Completed");
    Ok(())
}
