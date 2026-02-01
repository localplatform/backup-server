import fs from 'fs';
import { getDb } from './connection.js';
import { config } from '../config.js';

const SCHEMA = `
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
`;

export function migrate(): void {
  console.log('[DB] Starting database migration...');

  // Ensure data directories exist
  fs.mkdirSync(config.dataDir, { recursive: true });
  fs.mkdirSync(config.keysDir, { recursive: true });

  const db = getDb();
  db.exec(SCHEMA);

  // Idempotent migrations for existing databases
  const serverColumns = db.prepare("PRAGMA table_info(source_servers)").all() as { name: string }[];
  const serverColumnNames = serverColumns.map(c => c.name);

  if (!serverColumnNames.includes('use_sudo')) {
    db.exec('ALTER TABLE source_servers ADD COLUMN use_sudo INTEGER NOT NULL DEFAULT 0');
  }
  if (!serverColumnNames.includes('backup_user')) {
    db.exec("ALTER TABLE source_servers ADD COLUMN backup_user TEXT NOT NULL DEFAULT 'backup-agent'");
  }
  if (!serverColumnNames.includes('backup_key_path')) {
    db.exec('ALTER TABLE source_servers ADD COLUMN backup_key_path TEXT');
  }

  // Migration for backup_jobs table
  const jobColumns = db.prepare("PRAGMA table_info(backup_jobs)").all() as { name: string }[];
  const jobColumnNames = jobColumns.map(c => c.name);

  if (!jobColumnNames.includes('max_versions')) {
    db.exec('ALTER TABLE backup_jobs ADD COLUMN max_versions INTEGER NOT NULL DEFAULT 7');
  }

  // Migration: purge all existing data (switch from backup-agent to root SSH)
  // DISABLED: This migration was too aggressive and deleted data on every restart
  // Reason: Running DELETE queries on every migration would wipe user data during development
  // with tsx watch mode. This has been permanently disabled.
  // const serverCount = (db.prepare('SELECT COUNT(*) as count FROM source_servers WHERE backup_user != ?').get('root') as { count: number })?.count ?? 0;
  // if (serverCount > 0) {
  //   db.exec('DELETE FROM backup_logs');
  //   db.exec('DELETE FROM backup_jobs');
  //   db.exec('DELETE FROM source_servers');
  //
  //   // Clean up old SSH key files
  //   const keyFiles = fs.readdirSync(config.keysDir);
  //   for (const file of keyFiles) {
  //     fs.unlinkSync(`${config.keysDir}/${file}`);
  //   }
  // }

  console.log('[DB] Migration completed successfully');
}
