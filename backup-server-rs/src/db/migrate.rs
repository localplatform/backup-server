use crate::db::connection::DbPool;
use std::fs;
use std::path::Path;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS source_servers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  hostname TEXT NOT NULL,
  port INTEGER NOT NULL DEFAULT 22,
  ssh_user TEXT NOT NULL DEFAULT 'root',
  ssh_key_path TEXT,
  ssh_status TEXT NOT NULL DEFAULT 'pending' CHECK(ssh_status IN ('pending','key_generated','key_registered','connected','error')),
  ssh_error TEXT,
  rsync_installed INTEGER NOT NULL DEFAULT 0,
  use_sudo INTEGER NOT NULL DEFAULT 0,
  last_seen_at TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS backup_jobs (
  id TEXT PRIMARY KEY,
  server_id TEXT NOT NULL REFERENCES source_servers(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  remote_paths TEXT NOT NULL DEFAULT '[]',
  local_path TEXT NOT NULL,
  cron_schedule TEXT,
  status TEXT NOT NULL DEFAULT 'idle' CHECK(status IN ('idle','running','completed','failed','cancelled')),
  rsync_options TEXT NOT NULL DEFAULT '',
  max_parallel INTEGER NOT NULL DEFAULT 4,
  enabled INTEGER NOT NULL DEFAULT 1,
  max_versions INTEGER NOT NULL DEFAULT 7,
  last_run_at TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS backup_logs (
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES backup_jobs(id) ON DELETE CASCADE,
  started_at TEXT NOT NULL DEFAULT (datetime('now')),
  finished_at TEXT,
  status TEXT NOT NULL DEFAULT 'running' CHECK(status IN ('running','completed','failed','cancelled')),
  bytes_transferred INTEGER NOT NULL DEFAULT 0,
  files_transferred INTEGER NOT NULL DEFAULT 0,
  output TEXT NOT NULL DEFAULT '',
  error TEXT
);

CREATE TABLE IF NOT EXISTS backup_versions (
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL REFERENCES backup_jobs(id) ON DELETE CASCADE,
  log_id TEXT REFERENCES backup_logs(id) ON DELETE SET NULL,
  version_timestamp TEXT NOT NULL,
  local_path TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'running' CHECK(status IN ('running','completed','failed')),
  bytes_total INTEGER NOT NULL DEFAULT 0,
  files_total INTEGER NOT NULL DEFAULT 0,
  bytes_transferred INTEGER NOT NULL DEFAULT 0,
  files_transferred INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_backup_versions_job_id ON backup_versions(job_id);
CREATE INDEX IF NOT EXISTS idx_backup_versions_timestamp ON backup_versions(version_timestamp DESC);
"#;

pub fn migrate(pool: &DbPool, data_dir: &Path, keys_dir: &Path) -> anyhow::Result<()> {
    tracing::info!("[DB] Starting database migration...");

    fs::create_dir_all(data_dir)?;
    fs::create_dir_all(keys_dir)?;

    let conn = pool.get()?;
    conn.execute_batch(SCHEMA)?;

    // Idempotent migrations for existing databases
    let has_column = |table: &str, column: &str| -> bool {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({})", table))
            .unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        columns.contains(&column.to_string())
    };

    // source_servers migrations
    if !has_column("source_servers", "use_sudo") {
        conn.execute_batch(
            "ALTER TABLE source_servers ADD COLUMN use_sudo INTEGER NOT NULL DEFAULT 0",
        )?;
    }
    if !has_column("source_servers", "backup_user") {
        conn.execute_batch(
            "ALTER TABLE source_servers ADD COLUMN backup_user TEXT NOT NULL DEFAULT 'backup-agent'",
        )?;
    }
    if !has_column("source_servers", "backup_key_path") {
        conn.execute_batch("ALTER TABLE source_servers ADD COLUMN backup_key_path TEXT")?;
    }
    if !has_column("source_servers", "agent_status") {
        conn.execute_batch(
            "ALTER TABLE source_servers ADD COLUMN agent_status TEXT NOT NULL DEFAULT 'disconnected'",
        )?;
    }
    if !has_column("source_servers", "agent_version") {
        conn.execute_batch("ALTER TABLE source_servers ADD COLUMN agent_version TEXT")?;
    }
    if !has_column("source_servers", "agent_last_seen") {
        conn.execute_batch("ALTER TABLE source_servers ADD COLUMN agent_last_seen TEXT")?;
    }

    // backup_jobs migrations
    if !has_column("backup_jobs", "max_versions") {
        conn.execute_batch(
            "ALTER TABLE backup_jobs ADD COLUMN max_versions INTEGER NOT NULL DEFAULT 7",
        )?;
    }

    tracing::info!("[DB] Migration completed successfully");
    Ok(())
}
