/**
 * Agent Deployer - Deploys the backup agent to remote servers via SSH.
 *
 * Flow:
 * 1. Connect to remote server via SSH with password
 * 2. Upload agent binary via SFTP
 * 3. Write config.toml with server URL and server_id
 * 4. Create systemd service
 * 5. Start the service
 * 6. Wait for agent to connect back via WebSocket
 */

import { Client, SFTPWrapper } from 'ssh2';
import fs from 'fs';
import path from 'path';
import { config } from '../config.js';
import { logger } from '../utils/logger.js';
import { isAgentConnected } from '../websocket/agentRegistry.js';

const AGENT_BINARY_PATH = path.resolve(config.dataDir, '..', '..', 'backup-agent', 'target', 'release', 'backup-agent');
const REMOTE_BINARY_PATH = '/usr/local/bin/backup-agent';
const REMOTE_CONFIG_DIR = '/etc/backup-agent';
const REMOTE_CONFIG_PATH = '/etc/backup-agent/config.toml';
const SYSTEMD_SERVICE_PATH = '/etc/systemd/system/backup-agent.service';

interface DeployOptions {
  hostname: string;
  port: number;
  username: string;
  password: string;
  serverId: string;
  serverPort: number;
}

/**
 * Deploy the backup agent to a remote server.
 */
export async function deployAgent(opts: DeployOptions): Promise<void> {
  logger.info({ hostname: opts.hostname }, 'Starting agent deployment');

  // Check if binary exists
  if (!fs.existsSync(AGENT_BINARY_PATH)) {
    throw new Error(`Agent binary not found at ${AGENT_BINARY_PATH}. Build the agent first with: cd backup-agent && cargo build --release`);
  }

  const conn = await connectSSH(opts);

  try {
    // 1. Upload binary (to /tmp first, then sudo mv to final location)
    logger.info({ hostname: opts.hostname }, 'Uploading agent binary...');
    const tmpBinaryPath = `/tmp/backup-agent-upload-${Date.now()}`;
    await uploadFile(conn, AGENT_BINARY_PATH, tmpBinaryPath);
    await execCommand(conn, opts.password, `sudo mv ${tmpBinaryPath} ${REMOTE_BINARY_PATH}`);
    await execCommand(conn, opts.password, `sudo chmod +x ${REMOTE_BINARY_PATH}`);

    // 2. Detect backup server IP as seen from the remote host
    const detectedIp = await detectSourceIp(conn, opts.password);
    const serverUrl = `http://${detectedIp}:${opts.serverPort}`;
    logger.info({ hostname: opts.hostname, serverUrl }, 'Detected backup server URL from SSH connection');

    // 3. Create config directory and write config
    logger.info({ hostname: opts.hostname }, 'Writing agent config...');
    const configContent = generateConfig({ ...opts, serverUrl });
    await execCommand(conn, opts.password, `sudo mkdir -p ${REMOTE_CONFIG_DIR}`);
    await writeRemoteFile(conn, opts.password, REMOTE_CONFIG_PATH, configContent);

    // 3. Create systemd service
    logger.info({ hostname: opts.hostname }, 'Creating systemd service...');
    const serviceContent = generateSystemdService();
    await writeRemoteFile(conn, opts.password, SYSTEMD_SERVICE_PATH, serviceContent);

    // 4. Stop any existing agent and free port 9990
    logger.info({ hostname: opts.hostname }, 'Stopping existing agent (if any)...');
    await execCommand(conn, opts.password, 'sudo systemctl stop backup-agent || true');
    await execCommand(conn, opts.password, 'sudo fuser -k 9990/tcp || true');
    await new Promise(resolve => setTimeout(resolve, 1000));

    // 5. Start the service
    logger.info({ hostname: opts.hostname }, 'Starting agent service...');
    await execCommand(conn, opts.password, 'sudo systemctl daemon-reload');
    await execCommand(conn, opts.password, 'sudo systemctl enable backup-agent');
    await execCommand(conn, opts.password, 'sudo systemctl restart backup-agent');

    // 6. Verify the service started successfully
    await new Promise(resolve => setTimeout(resolve, 2000));
    const statusResult = await execCommand(conn, opts.password, 'sudo systemctl is-active backup-agent');
    const isActive = statusResult.stdout.trim() === 'active';

    if (!isActive) {
      const journalResult = await execCommand(conn, opts.password, 'sudo journalctl -u backup-agent -n 30 --no-pager');
      const statusDetail = await execCommand(conn, opts.password, 'sudo systemctl status backup-agent --no-pager');
      logger.error(
        { hostname: opts.hostname, status: statusResult.stdout.trim(), journal: journalResult.stdout, statusDetail: statusDetail.stdout },
        'Agent service failed to start'
      );
      throw new Error(`Agent service failed to start (status: ${statusResult.stdout.trim()}). Check server logs for journal output.`);
    }

    logger.info({ hostname: opts.hostname }, 'Agent service is active, waiting for WS connection...');
  } finally {
    conn.end();
  }

  // 5. Wait for agent to connect back via WebSocket
  await waitForAgentConnection(opts.serverId, 30000);

  logger.info({ hostname: opts.hostname }, 'Agent deployed and connected');
}

/**
 * Update the agent on a remote server by sending the update command via WebSocket.
 * The agent will download the new binary from the server and restart.
 */
export function getAgentBinaryPath(): string {
  return AGENT_BINARY_PATH;
}

// ---------------------------------------------------------------------------
// SSH helpers
// ---------------------------------------------------------------------------

function connectSSH(opts: DeployOptions): Promise<Client> {
  return new Promise((resolve, reject) => {
    const conn = new Client();
    conn.on('ready', () => resolve(conn));
    conn.on('error', (err) => reject(err));
    conn.on('keyboard-interactive', (_name, _instructions, _lang, _prompts, finish) => {
      finish([opts.password]);
    });
    conn.connect({
      host: opts.hostname,
      port: opts.port,
      username: opts.username,
      password: opts.password,
      tryKeyboard: true,
      readyTimeout: 30000,
    });
  });
}

function uploadFile(conn: Client, localPath: string, remotePath: string): Promise<void> {
  return new Promise((resolve, reject) => {
    conn.sftp((err, sftp) => {
      if (err) return reject(err);

      const readStream = fs.createReadStream(localPath);
      const writeStream = sftp.createWriteStream(remotePath);

      writeStream.on('close', () => {
        sftp.end();
        resolve();
      });
      writeStream.on('error', (err: Error) => {
        sftp.end();
        reject(err);
      });

      readStream.pipe(writeStream);
    });
  });
}

function execCommand(conn: Client, password: string, cmd: string): Promise<{ code: number; stdout: string; stderr: string }> {
  const sudoCmd = cmd.replace(/\bsudo\b/g, 'sudo -S');

  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      conn.end();
      reject(new Error(`Command timed out: ${cmd.slice(0, 80)}`));
    }, 30000);

    conn.exec(sudoCmd, (err, stream) => {
      if (err) {
        clearTimeout(timeout);
        return reject(err);
      }

      let stdout = '';
      let stderr = '';

      if (sudoCmd.includes('sudo -S')) {
        stream.write(password + '\n');
      }

      stream.on('data', (data: Buffer) => { stdout += data.toString(); });
      stream.stderr.on('data', (data: Buffer) => {
        const text = data.toString();
        if (text.includes('[sudo] password') || text.includes('Password:')) return;
        stderr += text;
      });
      stream.on('close', (code: number | null) => {
        clearTimeout(timeout);
        resolve({ code: code ?? 1, stdout, stderr });
      });
    });
  });
}

function writeRemoteFile(conn: Client, password: string, remotePath: string, content: string): Promise<void> {
  // Write to /tmp first, then sudo mv to final location
  const tmpPath = `/tmp/backup-agent-deploy-${Date.now()}`;
  return new Promise((resolve, reject) => {
    conn.sftp(async (err, sftp) => {
      if (err) return reject(err);

      const writeStream = sftp.createWriteStream(tmpPath);
      writeStream.on('close', async () => {
        sftp.end();
        try {
          await execCommand(conn, password, `sudo mv ${tmpPath} ${remotePath}`);
          resolve();
        } catch (e) {
          reject(e);
        }
      });
      writeStream.on('error', (err: Error) => {
        sftp.end();
        reject(err);
      });
      writeStream.end(content);
    });
  });
}

async function waitForAgentConnection(serverId: string, timeoutMs: number): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (isAgentConnected(serverId)) {
      return;
    }
    await new Promise(resolve => setTimeout(resolve, 1000));
  }
  logger.warn({ serverId }, 'Agent did not connect within timeout, it may connect later');
}

/**
 * Detect the backup server's IP as seen from the remote host.
 * Uses SSH_CONNECTION env var which contains: client_ip client_port server_ip server_port
 * Falls back to BACKUP_SERVER_IP env var, then to network interface detection.
 */
async function detectSourceIp(conn: Client, password: string): Promise<string> {
  try {
    // SSH_CONNECTION = "client_ip client_port server_ip server_port"
    // The client_ip is the backup server's IP as seen by the remote host
    const result = await execCommand(conn, password, 'echo $SSH_CONNECTION');
    const parts = result.stdout.trim().split(/\s+/);
    if (parts.length >= 1 && parts[0]) {
      const ip = parts[0];
      // Validate it looks like an IP
      if (/^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$/.test(ip)) {
        return ip;
      }
    }
  } catch {
    // Fall through to fallback
  }

  // Fallback: use env var or first non-loopback interface
  if (process.env.BACKUP_SERVER_IP) {
    return process.env.BACKUP_SERVER_IP;
  }

  try {
    const os = await import('os');
    const nets = os.networkInterfaces();
    for (const name of Object.keys(nets)) {
      for (const net of nets[name]!) {
        if (net.family === 'IPv4' && !net.internal) {
          return net.address;
        }
      }
    }
  } catch {
    // ignore
  }

  return '127.0.0.1';
}

// ---------------------------------------------------------------------------
// Config / Service generators
// ---------------------------------------------------------------------------

function generateConfig(opts: DeployOptions & { serverUrl: string }): string {
  return `[agent]
id = "${opts.hostname}"
port = 9990
data_dir = "/var/lib/backup-agent"

[server]
url = "${opts.serverUrl}"
token = ""
server_id = "${opts.serverId}"

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
`;
}

function generateSystemdService(): string {
  return `[Unit]
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
`;
}
