use crate::config::AppConfig;
use crate::ws::agent_registry::AgentRegistry;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

const REMOTE_BINARY_PATH: &str = "/usr/local/bin/backup-agent";
const REMOTE_CONFIG_DIR: &str = "/etc/backup-agent";
const REMOTE_CONFIG_PATH: &str = "/etc/backup-agent/config.toml";
const SYSTEMD_SERVICE_PATH: &str = "/etc/systemd/system/backup-agent.service";

pub struct DeployOptions {
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub server_id: String,
    pub server_port: u16,
    pub backup_server_ip: Option<String>,
}

pub fn get_agent_binary_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../backup-agent/target/release/backup-agent")
}

pub async fn deploy_agent(
    opts: DeployOptions,
    agents: Arc<AgentRegistry>,
) -> anyhow::Result<()> {
    let binary_path = get_agent_binary_path();
    if !binary_path.exists() {
        anyhow::bail!(
            "Agent binary not found at {}. Build with: cd backup-agent && cargo build --release",
            binary_path.display()
        );
    }

    tracing::info!(hostname = %opts.hostname, "Starting agent deployment");

    // Run SSH operations in a blocking task
    let result = tokio::task::spawn_blocking(move || {
        deploy_via_ssh(&opts, &binary_path)
    })
    .await??;

    // Wait for agent to connect
    let server_id = result;
    wait_for_agent_connection(&server_id, agents, 30_000).await;

    tracing::info!("Agent deployed and connected");
    Ok(())
}

fn deploy_via_ssh(opts: &DeployOptions, binary_path: &PathBuf) -> anyhow::Result<String> {
    // Connect via SSH
    let tcp = std::net::TcpStream::connect(format!("{}:{}", opts.hostname, opts.port))?;
    let mut sess = ssh2::Session::new()?;
    sess.set_tcp_stream(tcp);
    sess.handshake()?;

    // Authenticate with password
    sess.userauth_password(&opts.username, &opts.password)
        .map_err(|e| anyhow::anyhow!("SSH authentication failed: {}", e))?;

    if !sess.authenticated() {
        anyhow::bail!("SSH authentication failed");
    }

    // 1. Upload binary via SFTP
    tracing::info!(hostname = %opts.hostname, "Uploading agent binary...");
    let tmp_path = format!("/tmp/backup-agent-upload-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());

    let binary_data = std::fs::read(binary_path)?;
    let sftp = sess.sftp()?;
    let mut remote_file = sftp.create(std::path::Path::new(&tmp_path))?;
    std::io::Write::write_all(&mut remote_file, &binary_data)?;
    drop(remote_file);
    drop(sftp);

    exec_ssh(&sess, &opts.password, &format!("sudo mv {} {}", tmp_path, REMOTE_BINARY_PATH))?;
    exec_ssh(&sess, &opts.password, &format!("sudo chmod +x {}", REMOTE_BINARY_PATH))?;

    // 2. Detect server IP
    let detected_ip = detect_source_ip(&sess, &opts.password, &opts.backup_server_ip)?;
    let server_url = format!("http://{}:{}", detected_ip, opts.server_port);
    tracing::info!(hostname = %opts.hostname, server_url = %server_url, "Detected backup server URL");

    // 3. Write config
    tracing::info!(hostname = %opts.hostname, "Writing agent config...");
    let config_content = generate_config(&opts.hostname, &server_url, &opts.server_id);
    exec_ssh(&sess, &opts.password, &format!("sudo mkdir -p {}", REMOTE_CONFIG_DIR))?;
    write_remote_file(&sess, &opts.password, REMOTE_CONFIG_PATH, &config_content)?;

    // 4. Create systemd service
    tracing::info!(hostname = %opts.hostname, "Creating systemd service...");
    let service_content = generate_systemd_service();
    write_remote_file(&sess, &opts.password, SYSTEMD_SERVICE_PATH, &service_content)?;

    // 5. Stop existing agent
    tracing::info!(hostname = %opts.hostname, "Stopping existing agent...");
    let _ = exec_ssh(&sess, &opts.password, "sudo systemctl stop backup-agent || true");
    let _ = exec_ssh(&sess, &opts.password, "sudo fuser -k 9990/tcp || true");
    std::thread::sleep(std::time::Duration::from_secs(1));

    // 6. Start service
    tracing::info!(hostname = %opts.hostname, "Starting agent service...");
    exec_ssh(&sess, &opts.password, "sudo systemctl daemon-reload")?;
    exec_ssh(&sess, &opts.password, "sudo systemctl enable backup-agent")?;
    exec_ssh(&sess, &opts.password, "sudo systemctl restart backup-agent")?;

    // 7. Verify
    std::thread::sleep(std::time::Duration::from_secs(2));
    let status = exec_ssh(&sess, &opts.password, "sudo systemctl is-active backup-agent")?;
    if status.trim() != "active" {
        let journal = exec_ssh(&sess, &opts.password, "sudo journalctl -u backup-agent -n 30 --no-pager").unwrap_or_default();
        tracing::error!(hostname = %opts.hostname, status = %status.trim(), journal = %journal, "Agent service failed to start");
        anyhow::bail!("Agent service failed to start (status: {})", status.trim());
    }

    tracing::info!(hostname = %opts.hostname, "Agent service is active, waiting for WS connection...");
    Ok(opts.server_id.clone())
}

fn exec_ssh(sess: &ssh2::Session, password: &str, cmd: &str) -> anyhow::Result<String> {
    let sudo_cmd = cmd.replace("sudo", "sudo -S");
    let mut channel = sess.channel_session()?;
    channel.exec(&sudo_cmd)?;

    if sudo_cmd.contains("sudo -S") {
        std::io::Write::write_all(&mut channel, format!("{}\n", password).as_bytes())?;
    }

    let mut stdout = String::new();
    channel.read_to_string(&mut stdout)?;
    channel.wait_close()?;
    Ok(stdout)
}

fn write_remote_file(sess: &ssh2::Session, password: &str, remote_path: &str, content: &str) -> anyhow::Result<()> {
    let tmp_path = format!("/tmp/backup-agent-deploy-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());

    let sftp = sess.sftp()?;
    let mut file = sftp.create(std::path::Path::new(&tmp_path))?;
    std::io::Write::write_all(&mut file, content.as_bytes())?;
    drop(file);
    drop(sftp);

    exec_ssh(sess, password, &format!("sudo mv {} {}", tmp_path, remote_path))?;
    Ok(())
}

fn detect_source_ip(sess: &ssh2::Session, password: &str, fallback_ip: &Option<String>) -> anyhow::Result<String> {
    if let Ok(output) = exec_ssh(sess, password, "echo $SSH_CONNECTION") {
        let parts: Vec<&str> = output.trim().split_whitespace().collect();
        if let Some(ip) = parts.first() {
            let ip = ip.trim();
            if ip.split('.').count() == 4 && ip.split('.').all(|p| p.parse::<u8>().is_ok()) {
                return Ok(ip.to_string());
            }
        }
    }

    if let Some(ip) = fallback_ip {
        return Ok(ip.clone());
    }

    Ok("127.0.0.1".into())
}

async fn wait_for_agent_connection(server_id: &str, agents: Arc<AgentRegistry>, timeout_ms: u64) {
    let start = std::time::Instant::now();
    while start.elapsed().as_millis() < timeout_ms as u128 {
        if agents.is_connected(server_id) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    tracing::warn!(server_id, "Agent did not connect within timeout, it may connect later");
}

fn generate_config(hostname: &str, server_url: &str, server_id: &str) -> String {
    format!(
        r#"[agent]
id = "{hostname}"
port = 9990
data_dir = "/var/lib/backup-agent"

[server]
url = "{server_url}"
token = ""
server_id = "{server_id}"

[sync]
chunk_size = 1048576
compression = "zstd"
compression_level = 3

[log]
level = "info"
output = "stdout"

[daemon]
pid_file = "/var/run/backup-agent.pid"
user = "root"
group = "root"

[performance]
max_concurrent_jobs = 1
io_threads = 4
"#
    )
}

fn generate_systemd_service() -> String {
    r#"[Unit]
Description=Backup Agent
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/backup-agent --config /etc/backup-agent/config.toml
Restart=always
RestartSec=5
User=root
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
"#
    .to_string()
}
