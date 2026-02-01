import { Client } from 'ssh2';
import { sshPool } from './sshConnectionPool.js';
import { Server } from '../models/server.js';
import { logger } from '../utils/logger.js';

function execCommand(client: Client, cmd: string): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    client.exec(cmd, (err, stream) => {
      if (err) return reject(err);
      let stdout = '';
      let stderr = '';
      stream.on('close', (code: number) => {
        resolve({ code, stdout, stderr });
      });
      stream.on('data', (data: Buffer) => { stdout += data.toString(); });
      stream.stderr.on('data', (data: Buffer) => { stderr += data.toString(); });
    });
  });
}

export async function checkRsync(server: Server): Promise<boolean> {
  if (!server.ssh_key_path) throw new Error('Server has no SSH key configured');

  const client = await sshPool.getConnection(server.id, server.hostname, server.port, server.ssh_user, server.ssh_key_path);
  const result = await execCommand(client, 'rsync --version');
  return result.code === 0;
}

export async function installRsync(server: Server): Promise<boolean> {
  if (!server.ssh_key_path) throw new Error('Server has no SSH key configured');

  const client = await sshPool.getConnection(server.id, server.hostname, server.port, server.ssh_user, server.ssh_key_path);

  // Detect package manager
  const checks = [
    { cmd: 'which apt-get', install: 'apt-get update && apt-get install -y rsync' },
    { cmd: 'which yum', install: 'yum install -y rsync' },
    { cmd: 'which dnf', install: 'dnf install -y rsync' },
    { cmd: 'which pacman', install: 'pacman -S --noconfirm rsync' },
    { cmd: 'which apk', install: 'apk add rsync' },
  ];

  for (const check of checks) {
    const result = await execCommand(client, check.cmd);
    if (result.code === 0) {
      logger.info({ serverId: server.id, install: check.install }, 'Installing rsync');
      const installResult = await execCommand(client, check.install);
      if (installResult.code === 0) {
        const verify = await execCommand(client, 'rsync --version');
        return verify.code === 0;
      }
      logger.error({ serverId: server.id, stderr: installResult.stderr }, 'Failed to install rsync');
      return false;
    }
  }

  logger.error({ serverId: server.id }, 'No supported package manager found');
  return false;
}

export async function verifyPathsExist(server: Server, remotePaths: string[]): Promise<string[]> {
  if (!server.ssh_key_path) throw new Error('Server has no SSH key configured');

  const client = await sshPool.getConnection(server.id, server.hostname, server.port, server.ssh_user, server.ssh_key_path);

  const failed: string[] = [];
  for (const remotePath of remotePaths) {
    // Escape single quotes in path and wrap in single quotes
    const escapedPath = remotePath.replace(/'/g, "'\\''");
    const result = await execCommand(client, `test -e '${escapedPath}'`);
    if (result.code !== 0) {
      failed.push(remotePath);
    }
  }
  return failed;
}
