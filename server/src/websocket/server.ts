import { WebSocketServer, WebSocket } from 'ws';
import type { Server as HttpServer } from 'http';
import { logger } from '../utils/logger.js';
import type { WsEventType } from './events.js';

let wss: WebSocketServer | null = null;
const PING_INTERVAL = 30000; // 30 seconds

// Message queue for reconnection resilience
interface QueuedMessage {
  type: WsEventType;
  payload: Record<string, unknown>;
  timestamp: number;
}

const MESSAGE_QUEUE_SIZE = 100;
const messageQueues = new Map<string, QueuedMessage[]>(); // key = jobId

function queueMessage(type: WsEventType, payload: Record<string, unknown>): void {
  // Only queue backup-related messages
  if (!type.startsWith('backup:')) return;

  const jobId = (payload as { jobId?: string }).jobId;
  if (!jobId) return;

  let queue = messageQueues.get(jobId);
  if (!queue) {
    queue = [];
    messageQueues.set(jobId, queue);
  }

  queue.push({ type, payload, timestamp: Date.now() });

  // Keep only last MESSAGE_QUEUE_SIZE messages (LRU cache)
  if (queue.length > MESSAGE_QUEUE_SIZE) {
    queue.shift();
  }
}

function getQueuedMessages(jobId: string, since: number): QueuedMessage[] {
  const queue = messageQueues.get(jobId);
  if (!queue) return [];

  return queue.filter(msg => msg.timestamp > since);
}

function cleanupQueue(jobId: string): void {
  messageQueues.delete(jobId);
}

export function setupWebSocket(server: HttpServer): WebSocketServer {
  wss = new WebSocketServer({ server, path: '/ws' });

  wss.on('connection', (ws) => {
    logger.info('WebSocket client connected');

    // Handle incoming messages (replay requests)
    ws.on('message', (data) => {
      try {
        const msg = JSON.parse(data.toString());

        if (msg.type === 'replay:request') {
          const { jobId, since } = msg.payload;
          const queued = getQueuedMessages(jobId, since);

          queued.forEach(qm => {
            ws.send(JSON.stringify({ type: qm.type, payload: qm.payload }));
          });

          logger.info({ jobId, count: queued.length }, 'Replayed queued messages');
        }
      } catch (err) {
        logger.warn({ err }, 'Failed to parse WebSocket message');
      }
    });

    // Setup ping/pong to keep connection alive
    let isAlive = true;
    ws.on('pong', () => {
      isAlive = true;
    });

    const pingInterval = setInterval(() => {
      if (!isAlive) {
        ws.terminate();
        return;
      }
      isAlive = false;
      ws.ping();
    }, PING_INTERVAL);

    ws.on('close', () => {
      clearInterval(pingInterval);
      logger.info('WebSocket client disconnected');
    });

    ws.on('error', (err) => {
      clearInterval(pingInterval);
      logger.error({ err }, 'WebSocket error');
    });
  });

  logger.info('WebSocket server initialized');
  return wss;
}

export function broadcast(type: WsEventType, payload: Record<string, unknown>): void {
  if (!wss) return;

  // Queue message for reconnection resilience
  queueMessage(type, payload);

  const message = JSON.stringify({ type, payload });

  wss.clients.forEach((client) => {
    if (client.readyState === WebSocket.OPEN) {
      client.send(message);
    }
  });

  // Cleanup queue 5 minutes after job completion
  if (type === 'backup:completed' || type === 'backup:failed') {
    const jobId = (payload as { jobId?: string }).jobId;
    if (jobId) {
      setTimeout(() => cleanupQueue(jobId), 5 * 60 * 1000);
    }
  }
}

export function getWss(): WebSocketServer | null {
  return wss;
}
