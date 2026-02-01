use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── BackupJob ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupJob {
    pub id: String,
    pub server_id: String,
    pub name: String,
    pub remote_paths: String, // JSON array stored as text
    pub local_path: String,
    pub cron_schedule: Option<String>,
    pub status: String,
    pub rsync_options: String,
    pub max_parallel: i64,
    pub enabled: i64,
    pub max_versions: i64,
    pub last_run_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateBackupJobRequest {
    pub server_id: String,
    pub name: String,
    pub remote_paths: Vec<String>,
    #[serde(default)]
    pub local_path: String,
    pub cron_schedule: Option<String>,
    #[serde(default)]
    pub rsync_options: String,
    #[serde(default = "default_max_parallel")]
    pub max_parallel: i64,
    #[serde(default = "default_enabled")]
    pub enabled: i64,
    #[serde(default = "default_max_versions")]
    pub max_versions: i64,
}

fn default_max_parallel() -> i64 { 4 }
fn default_enabled() -> i64 { 1 }
fn default_max_versions() -> i64 { 7 }

#[derive(Debug, Deserialize)]
pub struct UpdateBackupJobRequest {
    pub name: Option<String>,
    pub remote_paths: Option<Vec<String>>,
    pub local_path: Option<String>,
    pub cron_schedule: Option<Option<String>>,
    pub rsync_options: Option<String>,
    pub max_parallel: Option<i64>,
    pub enabled: Option<i64>,
    pub max_versions: Option<i64>,
}

fn row_to_job(row: &Row) -> rusqlite::Result<BackupJob> {
    Ok(BackupJob {
        id: row.get("id")?,
        server_id: row.get("server_id")?,
        name: row.get("name")?,
        remote_paths: row.get("remote_paths")?,
        local_path: row.get("local_path")?,
        cron_schedule: row.get("cron_schedule")?,
        status: row.get("status")?,
        rsync_options: row.get("rsync_options")?,
        max_parallel: row.get("max_parallel")?,
        enabled: row.get("enabled")?,
        max_versions: row.get("max_versions")?,
        last_run_at: row.get("last_run_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn find_all(conn: &Connection) -> anyhow::Result<Vec<BackupJob>> {
    let mut stmt = conn.prepare("SELECT * FROM backup_jobs ORDER BY created_at DESC")?;
    let rows = stmt.query_map([], |row| row_to_job(row))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn find_by_id(conn: &Connection, id: &str) -> anyhow::Result<Option<BackupJob>> {
    let mut stmt = conn.prepare("SELECT * FROM backup_jobs WHERE id = ?")?;
    let mut rows = stmt.query_map(params![id], |row| row_to_job(row))?;
    Ok(rows.next().and_then(|r| r.ok()))
}

pub fn find_by_server_id(conn: &Connection, server_id: &str) -> anyhow::Result<Vec<BackupJob>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM backup_jobs WHERE server_id = ? ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map(params![server_id], |row| row_to_job(row))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn create(conn: &Connection, data: &CreateBackupJobRequest) -> anyhow::Result<BackupJob> {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let remote_paths_json = serde_json::to_string(&data.remote_paths)?;
    conn.execute(
        "INSERT INTO backup_jobs (id, server_id, name, remote_paths, local_path, cron_schedule, rsync_options, max_parallel, enabled, max_versions, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            data.server_id,
            data.name,
            remote_paths_json,
            data.local_path,
            data.cron_schedule,
            data.rsync_options,
            data.max_parallel,
            data.enabled,
            data.max_versions,
            now,
            now,
        ],
    )?;
    find_by_id(conn, &id)?
        .ok_or_else(|| anyhow::anyhow!("Failed to retrieve created job"))
}

pub fn update(conn: &Connection, id: &str, data: &UpdateBackupJobRequest) -> anyhow::Result<Option<BackupJob>> {
    let existing = find_by_id(conn, id)?;
    if existing.is_none() {
        return Ok(None);
    }

    let mut sets = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref name) = data.name {
        sets.push("name = ?");
        values.push(Box::new(name.clone()));
    }
    if let Some(ref remote_paths) = data.remote_paths {
        sets.push("remote_paths = ?");
        values.push(Box::new(serde_json::to_string(remote_paths).unwrap()));
    }
    if let Some(ref local_path) = data.local_path {
        sets.push("local_path = ?");
        values.push(Box::new(local_path.clone()));
    }
    if let Some(ref cron_schedule) = data.cron_schedule {
        sets.push("cron_schedule = ?");
        values.push(Box::new(cron_schedule.clone()));
    }
    if let Some(ref rsync_options) = data.rsync_options {
        sets.push("rsync_options = ?");
        values.push(Box::new(rsync_options.clone()));
    }
    if let Some(max_parallel) = data.max_parallel {
        sets.push("max_parallel = ?");
        values.push(Box::new(max_parallel));
    }
    if let Some(enabled) = data.enabled {
        sets.push("enabled = ?");
        values.push(Box::new(enabled));
    }
    if let Some(max_versions) = data.max_versions {
        sets.push("max_versions = ?");
        values.push(Box::new(max_versions));
    }

    if sets.is_empty() {
        return find_by_id(conn, id);
    }

    sets.push("updated_at = datetime('now')");
    values.push(Box::new(id.to_string()));

    let sql = format!("UPDATE backup_jobs SET {} WHERE id = ?", sets.join(", "));
    let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
    conn.execute(&sql, params.as_slice())?;
    find_by_id(conn, id)
}

pub fn update_status(conn: &Connection, id: &str, status: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE backup_jobs SET status = ?, updated_at = datetime('now') WHERE id = ?",
        params![status, id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> anyhow::Result<bool> {
    let changes = conn.execute("DELETE FROM backup_jobs WHERE id = ?", params![id])?;
    Ok(changes > 0)
}

// ── BackupLog ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupLog {
    pub id: String,
    pub job_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub bytes_transferred: i64,
    pub files_transferred: i64,
    pub output: String,
    pub error: Option<String>,
}

fn row_to_log(row: &Row) -> rusqlite::Result<BackupLog> {
    Ok(BackupLog {
        id: row.get("id")?,
        job_id: row.get("job_id")?,
        started_at: row.get("started_at")?,
        finished_at: row.get("finished_at")?,
        status: row.get("status")?,
        bytes_transferred: row.get("bytes_transferred")?,
        files_transferred: row.get("files_transferred")?,
        output: row.get("output")?,
        error: row.get("error")?,
    })
}

pub fn find_logs_by_job_id(conn: &Connection, job_id: &str, limit: i64) -> anyhow::Result<Vec<BackupLog>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM backup_logs WHERE job_id = ? ORDER BY started_at DESC LIMIT ?",
    )?;
    let rows = stmt.query_map(params![job_id, limit], |row| row_to_log(row))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn create_log(conn: &Connection, job_id: &str) -> anyhow::Result<BackupLog> {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO backup_logs (id, job_id, started_at) VALUES (?1, ?2, ?3)",
        params![id, job_id, now],
    )?;
    let mut stmt = conn.prepare("SELECT * FROM backup_logs WHERE id = ?")?;
    let mut rows = stmt.query_map(params![id], |row| row_to_log(row))?;
    rows.next()
        .ok_or_else(|| anyhow::anyhow!("Failed to retrieve created log"))?
        .map_err(Into::into)
}

pub fn update_log(conn: &Connection, id: &str, fields: &[(&str, &dyn rusqlite::types::ToSql)]) -> anyhow::Result<()> {
    if fields.is_empty() {
        return Ok(());
    }
    let sets: Vec<String> = fields.iter().map(|(k, _)| format!("{} = ?", k)).collect();
    let sql = format!("UPDATE backup_logs SET {} WHERE id = ?", sets.join(", "));
    let mut params: Vec<&dyn rusqlite::types::ToSql> = fields.iter().map(|(_, v)| *v).collect();
    params.push(&id);
    conn.execute(&sql, params.as_slice())?;
    Ok(())
}
