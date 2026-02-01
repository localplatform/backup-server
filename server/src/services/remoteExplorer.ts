import { Client, SFTPWrapper } from 'ssh2';
import { sshPool } from './sshConnectionPool.js';
import { Server } from '../models/server.js';
import { logger } from '../utils/logger.js';

export interface RemoteEntry {
  name: string;
  path: string;
  type: 'file' | 'directory' | 'symlink' | 'other';
  size: number;
  modifiedAt: string;
  permissions: number;
}

function getSftp(client: Client): Promise<SFTPWrapper> {
  return new Promise((resolve, reject) => {
    client.sftp((err, sftp) => {
      if (err) reject(err);
      else resolve(sftp);
    });
  });
}

export async function explorePath(server: Server, remotePath: string): Promise<RemoteEntry[]> {
  if (!server.ssh_key_path) {
    throw new Error('Server has no SSH key configured');
  }

  try {
    return await doExplorePath(server, remotePath);
  } catch (err: any) {
    if (err?.message?.includes('open failed') || err?.message?.includes('Channel open')) {
      logger.warn({ serverId: server.id }, 'SFTP channel failure, evicting connection and retrying');
      sshPool.evict(server.id);
      return await doExplorePath(server, remotePath);
    }
    throw err;
  }
}

async function doExplorePath(server: Server, remotePath: string): Promise<RemoteEntry[]> {
  const client = await sshPool.getConnection(
    server.id,
    server.hostname,
    server.port,
    server.ssh_user,
    server.ssh_key_path!
  );

  const sftp = await getSftp(client);

  try {
    const list = await new Promise<any[]>((resolve, reject) => {
      sftp.readdir(remotePath, (err, list) => {
        if (err) return reject(err);
        resolve(list);
      });
    });

    const entries: RemoteEntry[] = list.map(item => {
      let type: RemoteEntry['type'] = 'other';
      if (item.attrs.isDirectory()) type = 'directory';
      else if (item.attrs.isFile()) type = 'file';
      else if (item.attrs.isSymbolicLink()) type = 'symlink';

      return {
        name: item.filename,
        path: remotePath === '/' ? `/${item.filename}` : `${remotePath}/${item.filename}`,
        type,
        size: item.attrs.size,
        modifiedAt: new Date(item.attrs.mtime * 1000).toISOString(),
        permissions: item.attrs.mode & 0o777,
      };
    });

    // Sort: directories first, then by name
    entries.sort((a, b) => {
      if (a.type === 'directory' && b.type !== 'directory') return -1;
      if (a.type !== 'directory' && b.type === 'directory') return 1;
      return a.name.localeCompare(b.name);
    });

    return entries;
  } finally {
    sftp.end();
  }
}
