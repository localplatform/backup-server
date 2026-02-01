import { Router, Request, Response } from 'express';
import fs from 'fs';
import path from 'path';
import { v4 as uuidv4 } from 'uuid';
import { serverModel, CreateServerSchema } from '../models/server.js';
import { deployAgent, getAgentBinaryPath } from '../services/agentDeployer.js';
import { sendToAgent, isAgentConnected } from '../websocket/agentRegistry.js';
import { broadcast } from '../websocket/server.js';
import { config } from '../config.js';
import { logger } from '../utils/logger.js';

const router = Router();

/**
 * POST /api/agent/deploy
 * Deploy the agent to a remote server, creating the server record.
 */
router.post('/deploy', async (req: Request, res: Response) => {
  const parsed = CreateServerSchema.safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: parsed.error.flatten() });

  const { password, ...serverData } = parsed.data;

  // Create server record first (so we have an ID for the agent config)
  const server = serverModel.create({ ...serverData, ssh_user: serverData.ssh_user });

  try {
    await deployAgent({
      hostname: serverData.hostname,
      port: serverData.port,
      username: serverData.ssh_user,
      password,
      serverId: server.id,
      serverPort: config.port,
    });

    // The agent should have connected by now (or will soon)
    const updated = serverModel.findById(server.id)!;
    broadcast('server:updated', { server: updated });
    res.status(201).json(updated);
  } catch (err) {
    const errorMsg = err instanceof Error ? err.message : 'Deployment failed';
    logger.error({ err, hostname: serverData.hostname }, 'Agent deployment failed');

    // Delete the server record — no point keeping a failed deployment
    serverModel.delete(server.id);

    res.status(422).json({ error: errorMsg });
  }
});

/**
 * GET /api/agent/binary
 * Serve the compiled agent binary for download.
 */
router.get('/binary', (_req: Request, res: Response) => {
  const binaryPath = getAgentBinaryPath();

  if (!fs.existsSync(binaryPath)) {
    return res.status(404).json({ error: 'Agent binary not found. Build with: cd backup-agent && cargo build --release' });
  }

  res.download(binaryPath, 'backup-agent');
});

/**
 * POST /api/agent/update/:serverId
 * Send an update command to a connected agent.
 */
router.post('/update/:serverId', (req: Request, res: Response) => {
  const { serverId } = req.params;

  const server = serverModel.findById(serverId);
  if (!server) return res.status(404).json({ error: 'Server not found' });

  if (!isAgentConnected(serverId)) {
    return res.status(409).json({ error: 'Agent is not connected' });
  }

  // Send just the path — the agent will prepend its configured server URL
  const sent = sendToAgent(serverId, {
    type: 'agent:update',
    payload: {
      download_path: '/api/agent/binary',
      version: 'latest',
    },
  });

  if (!sent) {
    return res.status(500).json({ error: 'Failed to send update command' });
  }

  // Mark as updating
  serverModel.update(serverId, { agent_status: 'updating' } as any);
  broadcast('server:updated', { server: serverModel.findById(serverId) });

  res.json({ status: 'update_initiated' });
});

/**
 * GET /api/agent/status/:serverId
 * Get agent connection status for a server.
 */
router.get('/status/:serverId', (req: Request, res: Response) => {
  const server = serverModel.findById(req.params.serverId);
  if (!server) return res.status(404).json({ error: 'Server not found' });

  res.json({
    connected: isAgentConnected(req.params.serverId),
    agent_status: (server as any).agent_status,
    agent_version: (server as any).agent_version,
    agent_last_seen: (server as any).agent_last_seen,
  });
});

export default router;
