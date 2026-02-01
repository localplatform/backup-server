import { Client } from 'ssh2';
import fs from 'fs/promises';
import { logger } from '../utils/logger.js';
import { retry } from '../utils/retry.js';

interface PooledConnection {
  client: Client;
  serverId: string;
  lastUsed: number;
}

class SshConnectionPool {
  private connections = new Map<string, PooledConnection>();
  private connecting = new Map<string, Promise<Client>>();

  async getConnection(
    serverId: string,
    hostname: string,
    port: number,
    username: string,
    privateKeyPath: string
  ): Promise<Client> {
    const existing = this.connections.get(serverId);
    if (existing) {
      const socket = (existing.client as any)._sock;
      if (socket && !socket.destroyed && socket.writable) {
        existing.lastUsed = Date.now();
        return existing.client;
      }
      logger.warn({ serverId }, 'SSH connection stale, reconnecting');
      this.connections.delete(serverId);
      try { existing.client.end(); } catch { /* ignore */ }
    }

    // Avoid duplicate connections
    const pending = this.connecting.get(serverId);
    if (pending) return pending;

    const promise = this.createConnection(serverId, hostname, port, username, privateKeyPath);
    this.connecting.set(serverId, promise);

    try {
      const client = await promise;
      return client;
    } finally {
      this.connecting.delete(serverId);
    }
  }

  private async createConnection(
    serverId: string,
    hostname: string,
    port: number,
    username: string,
    privateKeyPath: string
  ): Promise<Client> {
    const privateKey = await fs.readFile(privateKeyPath, 'utf-8');

    return retry(async () => {
      return new Promise<Client>((resolve, reject) => {
        const client = new Client();
        client.on('ready', () => {
          this.connections.set(serverId, { client, serverId, lastUsed: Date.now() });
          logger.info({ serverId, hostname }, 'SSH connection established');
          resolve(client);
        });
        client.on('error', (err) => {
          this.connections.delete(serverId);
          reject(err);
        });
        client.on('close', () => {
          this.connections.delete(serverId);
          logger.info({ serverId }, 'SSH connection closed');
        });
        client.connect({ host: hostname, port, username, privateKey });
      });
    }, { maxRetries: 3, baseDelay: 1000, maxDelay: 10000, label: `ssh-${serverId}` });
  }

  evict(serverId: string): void {
    const conn = this.connections.get(serverId);
    if (conn) {
      this.connections.delete(serverId);
      try { conn.client.end(); } catch { /* ignore */ }
      logger.info({ serverId }, 'SSH connection evicted from pool');
    }
  }

  releaseConnection(serverId: string): void {
    const conn = this.connections.get(serverId);
    if (conn) {
      conn.client.end();
      this.connections.delete(serverId);
    }
  }

  closeAll(): void {
    for (const [id, conn] of this.connections) {
      conn.client.end();
      this.connections.delete(id);
    }
  }
}

export const sshPool = new SshConnectionPool();
