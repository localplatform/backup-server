use std::path::Path;

const MAX_BACKUPS: usize = 7;

pub fn backup_database(db_path: &str, data_dir: &Path) -> anyhow::Result<()> {
    let backup_dir = data_dir.join("backups");
    std::fs::create_dir_all(&backup_dir)?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let backup_name = format!("backup-server-{}.db", today);
    let backup_path = backup_dir.join(&backup_name);

    if backup_path.exists() {
        tracing::info!("[DB Backup] Today's backup already exists, skipping");
        return Ok(());
    }

    std::fs::copy(db_path, &backup_path)?;
    tracing::info!("[DB Backup] Created backup: {}", backup_name);

    // Cleanup old backups
    let mut backups: Vec<_> = std::fs::read_dir(&backup_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("backup-server-")
        })
        .collect();

    backups.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

    for old in backups.into_iter().skip(MAX_BACKUPS) {
        let _ = std::fs::remove_file(old.path());
        tracing::info!("[DB Backup] Removed old backup: {}", old.file_name().to_string_lossy());
    }

    Ok(())
}
