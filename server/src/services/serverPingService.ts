import { serverModel } from '../models/server.js';
import { isAgentConnected } from '../websocket/agentRegistry.js';
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

function checkAllServers(): void {
  const servers = serverModel.findAll();
  const now = new Date().toISOString();

  for (const server of servers) {
    const reachable = isAgentConnected(server.id);
    const status: PingStatus = {
      serverId: server.id,
      reachable,
      latencyMs: reachable ? 0 : null,
      lastCheckedAt: now,
    };
    statuses.set(server.id, status);
    broadcast('server:ping', status as unknown as Record<string, unknown>);
  }
}

export function startPingService(intervalMs = 10000): void {
  if (intervalHandle) return;

  logger.info({ intervalMs }, 'Starting server ping service');

  // Initial check
  checkAllServers();

  intervalHandle = setInterval(() => {
    checkAllServers();
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
