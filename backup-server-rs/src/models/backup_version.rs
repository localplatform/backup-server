use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupVersion {
    pub id: String,
    pub job_id: String,
    pub log_id: Option<String>,
    pub version_timestamp: String,
    pub local_path: String,
    pub status: String,
    pub bytes_total: i64,
    pub files_total: i64,
    pub bytes_transferred: i64,
    pub files_transferred: i64,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub backup_type: String,
    pub files_unchanged: i64,
    pub bytes_unchanged: i64,
    pub files_deleted: i64,
}

fn row_to_version(row: &Row) -> rusqlite::Result<BackupVersion> {
    Ok(BackupVersion {
        id: row.get("id")?,
        job_id: row.get("job_id")?,
        log_id: row.get("log_id")?,
        version_timestamp: row.get("version_timestamp")?,
        local_path: row.get("local_path")?,
        status: row.get("status")?,
        bytes_total: row.get("bytes_total")?,
        files_total: row.get("files_total")?,
        bytes_transferred: row.get("bytes_transferred")?,
        files_transferred: row.get("files_transferred")?,
        created_at: row.get("created_at")?,
        completed_at: row.get("completed_at")?,
        backup_type: row.get("backup_type").unwrap_or_else(|_| "full".to_string()),
        files_unchanged: row.get("files_unchanged").unwrap_or(0),
        bytes_unchanged: row.get("bytes_unchanged").unwrap_or(0),
        files_deleted: row.get("files_deleted").unwrap_or(0),
    })
}

pub fn find_all(conn: &Connection) -> anyhow::Result<Vec<BackupVersion>> {
    let mut stmt = conn.prepare("SELECT * FROM backup_versions ORDER BY version_timestamp DESC")?;
    let rows = stmt.query_map([], |row| row_to_version(row))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn find_by_id(conn: &Connection, id: &str) -> anyhow::Result<Option<BackupVersion>> {
    let mut stmt = conn.prepare("SELECT * FROM backup_versions WHERE id = ?")?;
    let mut rows = stmt.query_map(params![id], |row| row_to_version(row))?;
    Ok(rows.next().and_then(|r| r.ok()))
}

pub fn find_by_job_id(conn: &Connection, job_id: &str) -> anyhow::Result<Vec<BackupVersion>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM backup_versions WHERE job_id = ? ORDER BY version_timestamp DESC",
    )?;
    let rows = stmt.query_map(params![job_id], |row| row_to_version(row))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn find_latest_completed(conn: &Connection, job_id: &str) -> anyhow::Result<Option<BackupVersion>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM backup_versions WHERE job_id = ? AND status = 'completed' ORDER BY version_timestamp DESC LIMIT 1",
    )?;
    let mut rows = stmt.query_map(params![job_id], |row| row_to_version(row))?;
    Ok(rows.next().and_then(|r| r.ok()))
}

pub struct CreateVersionData {
    pub job_id: String,
    pub log_id: String,
    pub version_timestamp: String,
    pub local_path: String,
}

pub fn create(conn: &Connection, data: &CreateVersionData) -> anyhow::Result<BackupVersion> {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO backup_versions (id, job_id, log_id, version_timestamp, local_path, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, data.job_id, data.log_id, data.version_timestamp, data.local_path, now],
    )?;
    find_by_id(conn, &id)?
        .ok_or_else(|| anyhow::anyhow!("Failed to retrieve created version"))
}

pub struct CompletionData {
    pub bytes_transferred: i64,
    pub files_transferred: i64,
    pub backup_type: String,
    pub files_unchanged: i64,
    pub bytes_unchanged: i64,
    pub files_deleted: i64,
}

pub fn update_completion(conn: &Connection, id: &str, bytes_transferred: i64, files_transferred: i64) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE backup_versions SET status = 'completed', bytes_transferred = ?, files_transferred = ?, completed_at = ? WHERE id = ?",
        params![bytes_transferred, files_transferred, now, id],
    )?;
    Ok(())
}

pub fn update_completion_incremental(conn: &Connection, id: &str, data: &CompletionData) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE backup_versions SET status = 'completed', bytes_transferred = ?, files_transferred = ?, backup_type = ?, files_unchanged = ?, bytes_unchanged = ?, files_deleted = ?, completed_at = ? WHERE id = ?",
        params![data.bytes_transferred, data.files_transferred, data.backup_type, data.files_unchanged, data.bytes_unchanged, data.files_deleted, now, id],
    )?;
    Ok(())
}

pub fn update_failed(conn: &Connection, id: &str) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE backup_versions SET status = 'failed', completed_at = ? WHERE id = ?",
        params![now, id],
    )?;
    Ok(())
}

pub fn update_fields(conn: &Connection, id: &str, fields: &[(&str, &dyn rusqlite::types::ToSql)]) -> anyhow::Result<()> {
    if fields.is_empty() {
        return Ok(());
    }
    let sets: Vec<String> = fields.iter().map(|(k, _)| format!("{} = ?", k)).collect();
    let sql = format!("UPDATE backup_versions SET {} WHERE id = ?", sets.join(", "));
    let mut params: Vec<&dyn rusqlite::types::ToSql> = fields.iter().map(|(_, v)| *v).collect();
    params.push(&id);
    conn.execute(&sql, params.as_slice())?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> anyhow::Result<bool> {
    let changes = conn.execute("DELETE FROM backup_versions WHERE id = ?", params![id])?;
    Ok(changes > 0)
}

pub fn delete_by_job_id(conn: &Connection, job_id: &str) -> anyhow::Result<i64> {
    let versions = find_by_job_id(conn, job_id)?;
    let count = versions.len() as i64;
    conn.execute("DELETE FROM backup_versions WHERE job_id = ?", params![job_id])?;
    Ok(count)
}

pub fn delete_by_server_id(conn: &Connection, server_id: &str) -> anyhow::Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM backup_versions WHERE job_id IN (SELECT id FROM backup_jobs WHERE server_id = ?)",
        params![server_id],
        |row| row.get(0),
    )?;
    conn.execute(
        "DELETE FROM backup_versions WHERE job_id IN (SELECT id FROM backup_jobs WHERE server_id = ?)",
        params![server_id],
    )?;
    Ok(count)
}
