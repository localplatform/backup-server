import http from 'http';
import { createApp } from './app.js';
import { config } from './config.js';
import { migrate } from './db/migrate.js';
import { closeDb, flushDatabase } from './db/connection.js';
import { setupWebSocket, getWss } from './websocket/server.js';
import { setupAgentWebSocket, closeAllAgentConnections, getAgentWss } from './websocket/agentRegistry.js';
import { initSchedules, stopAllSchedules } from './services/backupScheduler.js';
import { startPingService, stopPingService } from './services/serverPingService.js';

import { logger } from './utils/logger.js';
import { backupDatabase } from './services/dbBackup.js';
import { migrateExistingBackups } from './services/backupMigration.js';
import { migrateServerFolderNames } from './services/pathMigration.js';

// Initialize database
migrate();

// Create daily database backup
backupDatabase();

// Migrate existing backups to versioned structure
await migrateExistingBackups();

// Migrate server folders from hostname to server name
await migrateServerFolderNames();

const app = createApp();
const server = http.createServer(app);

// Setup WebSocket servers (noServer mode — manual upgrade routing)
const uiWss = setupWebSocket();
const agWss = setupAgentWebSocket();

server.on('upgrade', (request, socket, head) => {
  const { pathname } = new URL(request.url || '/', `http://${request.headers.host}`);

  if (pathname === '/ws/agent') {
    agWss.handleUpgrade(request, socket, head, (ws) => {
      agWss.emit('connection', ws, request);
    });
  } else if (pathname === '/ws') {
    uiWss.handleUpgrade(request, socket, head, (ws) => {
      uiWss.emit('connection', ws, request);
    });
  } else {
    socket.destroy();
  }
});

// Initialize cron schedules
initSchedules();

// Start periodic server ping (every 10s)
startPingService(10000);

server.listen(config.port, () => {
  logger.info({ port: config.port }, 'Backup server started');
});

// Graceful shutdown
function shutdown(signal: string) {
  logger.info({ signal }, 'Starting graceful shutdown...');

  try {
    // 1. Stop accepting new work
    logger.info('[SHUTDOWN] Stopping scheduler...');
    stopAllSchedules();

    logger.info('[SHUTDOWN] Stopping ping service...');
    stopPingService();

    // 2. Close agent WebSocket connections
    logger.info('[SHUTDOWN] Closing agent connections...');
    closeAllAgentConnections();

    // 4. Close UI WebSocket connections
    const wss = getWss();
    if (wss) {
      logger.info('[SHUTDOWN] Closing WebSocket connections...');
      wss.clients.forEach((client) => {
        client.close();
      });
    }
    const agentWss = getAgentWss();
    if (agentWss) {
      agentWss.clients.forEach((client) => {
        client.close();
      });
    }

    // 4. Flush and close database (CRITICAL for persistence)
    logger.info('[SHUTDOWN] Flushing and closing database...');
    flushDatabase();
    closeDb();

    // 5. Close HTTP server
    logger.info('[SHUTDOWN] Closing HTTP server...');
    server.close(() => {
      logger.info('[SHUTDOWN] Graceful shutdown completed');
      process.exit(0);
    });

    // Force exit after 8s if graceful shutdown hangs
    setTimeout(() => {
      logger.error('[SHUTDOWN] Forcing exit after timeout');
      process.exit(1);
    }, 8000);

  } catch (error) {
    logger.error({ error }, '[SHUTDOWN] Error during shutdown');
    process.exit(1);
  }
}

process.on('SIGTERM', () => shutdown('SIGTERM'));
process.on('SIGINT', () => shutdown('SIGINT'));
