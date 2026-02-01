use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub port: i64,
    pub ssh_user: String,
    pub ssh_key_path: Option<String>,
    pub ssh_status: String,
    pub ssh_error: Option<String>,
    pub rsync_installed: i64,
    pub use_sudo: i64,
    pub agent_status: String,
    pub agent_version: Option<String>,
    pub agent_last_seen: Option<String>,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateServerRequest {
    pub name: String,
    pub hostname: String,
    #[serde(default = "default_port")]
    pub port: i64,
    #[serde(default = "default_ssh_user")]
    pub ssh_user: String,
    pub password: Option<String>,
}

fn default_port() -> i64 { 22 }
fn default_ssh_user() -> String { "root".into() }

#[derive(Debug, Deserialize)]
pub struct UpdateServerRequest {
    pub name: Option<String>,
    pub hostname: Option<String>,
    pub port: Option<i64>,
    pub ssh_user: Option<String>,
}

fn row_to_server(row: &Row) -> rusqlite::Result<Server> {
    Ok(Server {
        id: row.get("id")?,
        name: row.get("name")?,
        hostname: row.get("hostname")?,
        port: row.get("port")?,
        ssh_user: row.get("ssh_user")?,
        ssh_key_path: row.get("ssh_key_path")?,
        ssh_status: row.get("ssh_status")?,
        ssh_error: row.get("ssh_error")?,
        rsync_installed: row.get("rsync_installed")?,
        use_sudo: row.get("use_sudo")?,
        agent_status: row.get("agent_status")?,
        agent_version: row.get("agent_version")?,
        agent_last_seen: row.get("agent_last_seen")?,
        last_seen_at: row.get("last_seen_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn find_all(conn: &Connection) -> anyhow::Result<Vec<Server>> {
    let mut stmt = conn.prepare("SELECT * FROM source_servers ORDER BY created_at DESC")?;
    let rows = stmt.query_map([], |row| row_to_server(row))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

pub fn find_by_id(conn: &Connection, id: &str) -> anyhow::Result<Option<Server>> {
    let mut stmt = conn.prepare("SELECT * FROM source_servers WHERE id = ?")?;
    let mut rows = stmt.query_map(params![id], |row| row_to_server(row))?;
    Ok(rows.next().and_then(|r| r.ok()))
}

pub fn create(conn: &Connection, data: &CreateServerRequest) -> anyhow::Result<Server> {
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO source_servers (id, name, hostname, port, ssh_user, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, data.name, data.hostname, data.port, data.ssh_user, now, now],
    )?;
    find_by_id(conn, &id)?
        .ok_or_else(|| anyhow::anyhow!("Failed to retrieve created server"))
}

pub fn update(conn: &Connection, id: &str, data: &UpdateServerRequest) -> anyhow::Result<Option<Server>> {
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
    if let Some(ref hostname) = data.hostname {
        sets.push("hostname = ?");
        values.push(Box::new(hostname.clone()));
    }
    if let Some(port) = data.port {
        sets.push("port = ?");
        values.push(Box::new(port));
    }
    if let Some(ref ssh_user) = data.ssh_user {
        sets.push("ssh_user = ?");
        values.push(Box::new(ssh_user.clone()));
    }

    sets.push("updated_at = datetime('now')");
    values.push(Box::new(id.to_string()));

    let sql = format!(
        "UPDATE source_servers SET {} WHERE id = ?",
        sets.join(", ")
    );

    let params: Vec<&dyn rusqlite::types::ToSql> = values.iter().map(|v| v.as_ref()).collect();
    conn.execute(&sql, params.as_slice())?;
    find_by_id(conn, id)
}

pub fn update_fields(conn: &Connection, id: &str, fields: &[(&str, &dyn rusqlite::types::ToSql)]) -> anyhow::Result<()> {
    if fields.is_empty() {
        return Ok(());
    }
    let mut sets: Vec<String> = fields.iter().map(|(k, _)| format!("{} = ?", k)).collect();
    sets.push("updated_at = datetime('now')".into());

    let sql = format!("UPDATE source_servers SET {} WHERE id = ?", sets.join(", "));
    let mut params: Vec<&dyn rusqlite::types::ToSql> = fields.iter().map(|(_, v)| *v).collect();
    params.push(&id);
    conn.execute(&sql, params.as_slice())?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> anyhow::Result<bool> {
    let changes = conn.execute("DELETE FROM source_servers WHERE id = ?", params![id])?;
    Ok(changes > 0)
}
