CREATE TABLE IF NOT EXISTS source_servers (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  hostname TEXT NOT NULL,
  port INTEGER NOT NULL DEFAULT 22,
  ssh_user TEXT NOT NULL,
  ssh_key_path TEXT,
  ssh_status TEXT NOT NULL DEFAULT 'pending' CHECK(ssh_status IN ('pending','key_generated','key_registered','connected','error')),
  ssh_error TEXT,
  rsync_installed INTEGER NOT NULL DEFAULT 0,
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
