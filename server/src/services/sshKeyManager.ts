import { execFile } from 'child_process';
import { promisify } from 'util';
import fs from 'fs/promises';
import path from 'path';
import { Client } from 'ssh2';
import { config } from '../config.js';
import { logger } from '../utils/logger.js';

const execFileAsync = promisify(execFile);

export const sshKeyManager = {
  getKeyPaths(serverId: string) {
    const privateKey = path.join(config.keysDir, `${serverId}`);
    const publicKey = path.join(config.keysDir, `${serverId}.pub`);
    return { privateKey, publicKey };
  },

  async generateKey(serverId: string): Promise<{ privateKey: string; publicKey: string }> {
    const paths = this.getKeyPaths(serverId);

    // Remove existing keys if any
    await fs.rm(paths.privateKey, { force: true });
    await fs.rm(paths.publicKey, { force: true });

    await execFileAsync('ssh-keygen', [
      '-t', 'ed25519',
      '-f', paths.privateKey,
      '-N', '',
      '-C', `backup-server-${serverId}`,
    ]);

    // Set proper permissions
    await fs.chmod(paths.privateKey, 0o600);
    await fs.chmod(paths.publicKey, 0o644);

    const publicKeyContent = await fs.readFile(paths.publicKey, 'utf-8');
    logger.info({ serverId }, 'SSH key pair generated');

    return { privateKey: paths.privateKey, publicKey: publicKeyContent.trim() };
  },

  async getPublicKey(serverId: string): Promise<string> {
    const paths = this.getKeyPaths(serverId);
    return (await fs.readFile(paths.publicKey, 'utf-8')).trim();
  },

  async registerKey(
    hostname: string,
    port: number,
    username: string,
    password: string,
    publicKey: string
  ): Promise<void> {
    const escapedPubKey = publicKey.replace(/'/g, "'\\''");
    const cmd = `sudo sh -c 'mkdir -p /root/.ssh && chmod 700 /root/.ssh && echo "'"${escapedPubKey}"'" >> /root/.ssh/authorized_keys && chmod 600 /root/.ssh/authorized_keys'`;
    const result = await this.execWithPassword(hostname, port, username, password, cmd);
    if (result.code !== 0) {
      throw new Error(`Key registration failed (code ${result.code}): ${result.stderr}`);
    }
    logger.info({ hostname }, 'Public key registered for root');
  },

  async execWithPassword(
    hostname: string,
    port: number,
    username: string,
    password: string,
    cmd: string
  ): Promise<{ code: number; stdout: string; stderr: string }> {
    // Replace `sudo` with `sudo -S` so it reads the password from stdin
    const sudoCmd = cmd.replace(/\bsudo\b/g, 'sudo -S');

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        conn.end();
        reject(new Error(`Command timed out after 30s: ${cmd.slice(0, 80)}`));
      }, 30000);

      const conn = new Client();
      conn.on('ready', () => {
        conn.exec(sudoCmd, (err, stream) => {
          if (err) {
            clearTimeout(timeout);
            conn.end();
            return reject(err);
          }
          let stdout = '';
          let stderr = '';

          // Proactively write password to stdin for sudo -S
          // sudo -S reads from stdin immediately, no need to wait for prompt
          if (sudoCmd.includes('sudo -S')) {
            stream.write(password + '\n');
          }

          stream.on('data', (data: Buffer) => { stdout += data.toString(); });
          stream.stderr.on('data', (data: Buffer) => {
            const text = data.toString();
            // Filter out the sudo password prompt from stderr
            if (text.includes('[sudo] password') || text.includes('Password:') || text.includes('password for')) {
              return;
            }
            stderr += text;
          });
          stream.on('close', (code: number | null) => {
            clearTimeout(timeout);
            conn.end();
            resolve({ code: code ?? 1, stdout, stderr });
          });
        });
      });
      conn.on('error', (err) => {
        clearTimeout(timeout);
        reject(err);
      });
      conn.on('keyboard-interactive', (_name, _instructions, _instructionsLang, _prompts, finish) => {
        finish([password]);
      });
      conn.connect({ host: hostname, port, username, password, tryKeyboard: true, readyTimeout: 30000 });
    });
  },

  async testConnection(
    hostname: string,
    port: number,
    username: string,
    privateKeyPath: string
  ): Promise<boolean> {
    const privateKey = await fs.readFile(privateKeyPath, 'utf-8');
    return new Promise((resolve, reject) => {
      const conn = new Client();
      conn.on('ready', () => {
        conn.end();
        resolve(true);
      });
      conn.on('error', (err) => {
        reject(err);
      });
      conn.connect({ host: hostname, port, username, privateKey, readyTimeout: 30000 });
    });
  },
};
