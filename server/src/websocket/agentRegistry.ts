/**
 * Agent WebSocket Registry
 *
 * Accepts persistent WebSocket connections from remote backup agents.
 * Each agent connects to /ws/agent and registers with its server_id.
 * All backup commands, file browsing, and updates go through this channel.
 */

import { WebSocketServer, WebSocket } from 'ws';
import { v4 as uuidv4 } from 'uuid';
import { serverModel } from '../models/server.js';
import { broadcast } from './server.js';
import { logger } from '../utils/logger.js';

const PING_INTERVAL = 30000;
const REQUEST_TIMEOUT = 30000;

interface AgentConnection {
  serverId: string;
  ws: WebSocket;
  hostname: string;
  version: string;
  connectedAt: Date;
  lastPingAt: Date;
}

type AgentMessageHandler = (serverId: string, payload: Record<string, unknown>) => void;

const agents = new Map<string, AgentConnection>();
const pendingRequests = new Map<string, { resolve: (value: unknown) => void; reject: (err: Error) => void; timer: ReturnType<typeof setTimeout> }>();
const messageHandlers = new Map<string, Set<AgentMessageHandler>>();

let agentWss: WebSocketServer | null = null;

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

export function setupAgentWebSocket(): WebSocketServer {
  agentWss = new WebSocketServer({ noServer: true });

  agentWss.on('connection', (ws) => {
    logger.info('Agent WebSocket connection opened, waiting for registration...');

    let registered = false;
    let serverId: string | null = null;

    // Ping/pong keep-alive
    let isAlive = true;
    ws.on('pong', () => { isAlive = true; });

    const pingInterval = setInterval(() => {
      if (!isAlive) {
        ws.terminate();
        return;
      }
      isAlive = false;
      ws.ping();

      if (serverId) {
        const conn = agents.get(serverId);
        if (conn) conn.lastPingAt = new Date();
      }
    }, PING_INTERVAL);

    ws.on('message', (data) => {
      try {
        const msg = JSON.parse(data.toString());
        const type = msg.type as string;
        const payload = (msg.payload ?? {}) as Record<string, unknown>;

        // Registration handshake
        if (type === 'agent:register') {
          serverId = handleRegistration(ws, payload);
          if (serverId) registered = true;
          return;
        }

        if (!registered || !serverId) {
          logger.warn('Agent sent message before registration, ignoring');
          return;
        }

        // Response to a pending request (fs:browse, etc.)
        const requestId = payload.request_id as string | undefined;
        if (requestId && pendingRequests.has(requestId)) {
          const pending = pendingRequests.get(requestId)!;
          clearTimeout(pending.timer);
          pendingRequests.delete(requestId);
          pending.resolve(payload);
          return;
        }

        // Dispatch to registered message handlers
        const handlers = messageHandlers.get(type);
        if (handlers) {
          for (const handler of handlers) {
            try { handler(serverId, payload); } catch (err) {
              logger.error({ err, type }, 'Error in agent message handler');
            }
          }
        }

        // Also forward backup events to UI broadcast
        if (type.startsWith('backup:') || type.startsWith('agent:')) {
          // Handlers in agentOrchestrator will call broadcast() themselves
          // so we don't double-broadcast here
        }

      } catch (err) {
        logger.warn({ err }, 'Failed to parse agent WebSocket message');
      }
    });

    ws.on('close', () => {
      clearInterval(pingInterval);
      if (serverId) {
        handleDisconnection(serverId);
      }
    });

    ws.on('error', (err) => {
      clearInterval(pingInterval);
      logger.error({ err, serverId }, 'Agent WebSocket error');
      if (serverId) {
        handleDisconnection(serverId);
      }
    });
  });

  logger.info('Agent WebSocket server initialized on /ws/agent');
  return agentWss;
}

// ---------------------------------------------------------------------------
// Registration & Disconnection
// ---------------------------------------------------------------------------

function handleRegistration(ws: WebSocket, payload: Record<string, unknown>): string | null {
  const hostname = payload.hostname as string;
  const version = payload.version as string;
  const serverId = payload.server_id as string;

  if (!serverId || !hostname) {
    logger.warn({ payload }, 'Agent registration missing required fields');
    ws.send(JSON.stringify({ type: 'agent:register:error', payload: { error: 'Missing server_id or hostname' } }));
    return null;
  }

  // Verify server exists in DB
  const server = serverModel.findById(serverId);
  if (!server) {
    logger.warn({ serverId }, 'Agent registered with unknown server_id');
    ws.send(JSON.stringify({ type: 'agent:register:error', payload: { error: 'Unknown server_id' } }));
    return null;
  }

  // Close existing connection if any (agent reconnected)
  const existing = agents.get(serverId);
  if (existing) {
    logger.info({ serverId }, 'Agent reconnected, closing old connection');
    try { existing.ws.close(); } catch { /* ignore */ }
  }

  const conn: AgentConnection = {
    serverId,
    ws,
    hostname,
    version: version || 'unknown',
    connectedAt: new Date(),
    lastPingAt: new Date(),
  };

  agents.set(serverId, conn);

  // Update DB
  const now = new Date().toISOString();
  serverModel.update(serverId, {
    agent_status: 'connected',
    agent_version: version || 'unknown',
    agent_last_seen: now,
    last_seen_at: now,
  } as any);

  // Notify UI clients
  const updated = serverModel.findById(serverId);
  broadcast('server:updated', { server: updated });

  // Confirm registration to agent
  ws.send(JSON.stringify({ type: 'agent:register:ok', payload: { server_id: serverId } }));

  logger.info({ serverId, hostname, version }, 'Agent registered successfully');
  return serverId;
}

function handleDisconnection(serverId: string): void {
  agents.delete(serverId);

  // Update DB
  try {
    const server = serverModel.findById(serverId);
    if (server) {
      // Preserve 'updating' status during agent restart for self-update
      const currentStatus = (server as any).agent_status;
      const newStatus = currentStatus === 'updating' ? 'updating' : 'disconnected';

      serverModel.update(serverId, {
        agent_status: newStatus,
        agent_last_seen: new Date().toISOString(),
      } as any);

      const updated = serverModel.findById(serverId);
      broadcast('server:updated', { server: updated });
    }
  } catch (err) {
    logger.error({ err, serverId }, 'Error updating server on agent disconnect');
  }

  logger.info({ serverId }, 'Agent disconnected');
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export function isAgentConnected(serverId: string): boolean {
  const conn = agents.get(serverId);
  return !!conn && conn.ws.readyState === WebSocket.OPEN;
}

export function getConnectedAgent(serverId: string): AgentConnection | null {
  const conn = agents.get(serverId);
  if (!conn || conn.ws.readyState !== WebSocket.OPEN) return null;
  return conn;
}

export function getConnectedAgents(): Map<string, AgentConnection> {
  return agents;
}

export function sendToAgent(serverId: string, message: Record<string, unknown>): boolean {
  const conn = agents.get(serverId);
  if (!conn || conn.ws.readyState !== WebSocket.OPEN) {
    logger.warn({ serverId }, 'Cannot send to agent: not connected');
    return false;
  }

  conn.ws.send(JSON.stringify(message));
  return true;
}

/**
 * Send a request to an agent and wait for a response with matching request_id.
 */
export function requestFromAgent(serverId: string, message: Record<string, unknown>, timeoutMs = REQUEST_TIMEOUT): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const requestId = uuidv4();
    const fullMessage = { ...message, payload: { ...(message.payload as Record<string, unknown> ?? {}), request_id: requestId } };

    const timer = setTimeout(() => {
      pendingRequests.delete(requestId);
      reject(new Error(`Agent request timeout (${timeoutMs}ms)`));
    }, timeoutMs);

    pendingRequests.set(requestId, { resolve, reject, timer });

    const sent = sendToAgent(serverId, fullMessage);
    if (!sent) {
      clearTimeout(timer);
      pendingRequests.delete(requestId);
      reject(new Error('Agent not connected'));
    }
  });
}

/**
 * Register a handler for a specific message type from agents.
 */
export function onAgentMessage(type: string, handler: AgentMessageHandler): void {
  let handlers = messageHandlers.get(type);
  if (!handlers) {
    handlers = new Set();
    messageHandlers.set(type, handlers);
  }
  handlers.add(handler);
}

/**
 * Remove a handler for a specific message type.
 */
export function offAgentMessage(type: string, handler: AgentMessageHandler): void {
  const handlers = messageHandlers.get(type);
  if (handlers) {
    handlers.delete(handler);
  }
}

/**
 * Cleanup all agent connections (for shutdown).
 */
export function closeAllAgentConnections(): void {
  for (const [serverId, conn] of agents) {
    try { conn.ws.close(); } catch { /* ignore */ }
  }
  agents.clear();

  // Clear pending requests
  for (const [id, pending] of pendingRequests) {
    clearTimeout(pending.timer);
    pending.reject(new Error('Agent registry shutting down'));
  }
  pendingRequests.clear();
}

export function getAgentWss(): WebSocketServer | null {
  return agentWss;
}
