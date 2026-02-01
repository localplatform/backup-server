import net from 'net';
import { serverModel } from '../models/server.js';
import { broadcast } from '../websocket/server.js';
import { logger } from '../utils/logger.js';

export interface PingStatus {
  serverId: string;
  reachable: boolean;
  latencyMs: number | null;
  lastCheckedAt: string;
}

const statuses = new Map<string, PingStatus>();
let intervalHandle: ReturnType<typeof setInterval> | null = null;

function tcpPing(host: string, port: number, timeoutMs = 3000): Promise<number> {
  return new Promise((resolve, reject) => {
    const start = Date.now();
    const socket = new net.Socket();

    socket.setTimeout(timeoutMs);

    socket.on('connect', () => {
      const latency = Date.now() - start;
      socket.destroy();
      resolve(latency);
    });

    socket.on('timeout', () => {
      socket.destroy();
      reject(new Error('timeout'));
    });

    socket.on('error', (err) => {
      socket.destroy();
      reject(err);
    });

    socket.connect(port, host);
  });
}

async function pingAllServers(): Promise<void> {
  const servers = serverModel.findAll();
  const now = new Date().toISOString();

  const results = await Promise.allSettled(
    servers.map(async (server) => {
      try {
        const latency = await tcpPing(server.hostname, server.port);
        return { serverId: server.id, reachable: true, latencyMs: latency, lastCheckedAt: now };
      } catch {
        return { serverId: server.id, reachable: false, latencyMs: null, lastCheckedAt: now };
      }
    })
  );

  for (const result of results) {
    if (result.status === 'fulfilled') {
      const status = result.value;
      statuses.set(status.serverId, status);
      broadcast('server:ping', status as unknown as Record<string, unknown>);
    }
  }
}

export function startPingService(intervalMs = 10000): void {
  if (intervalHandle) return;

  logger.info({ intervalMs }, 'Starting server ping service');

  // Initial ping
  pingAllServers().catch(err => {
    logger.error({ err }, 'Initial ping cycle failed');
  });

  intervalHandle = setInterval(() => {
    pingAllServers().catch(err => {
      logger.error({ err }, 'Ping cycle failed');
    });
  }, intervalMs);
}

export function stopPingService(): void {
  if (intervalHandle) {
    clearInterval(intervalHandle);
    intervalHandle = null;
    logger.info('Server ping service stopped');
  }
}

export function getAllPingStatuses(): PingStatus[] {
  return [...statuses.values()];
}
