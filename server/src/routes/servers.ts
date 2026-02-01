import { Router, Request, Response } from 'express';
import { serverModel, CreateServerSchema, UpdateServerSchema } from '../models/server.js';
import { deployAgent } from '../services/agentDeployer.js';
import { getAllPingStatuses } from '../services/serverPingService.js';
import { broadcast } from '../websocket/server.js';
import { config } from '../config.js';
import { logger } from '../utils/logger.js';

const router = Router();

// GET /api/servers
router.get('/', (_req: Request, res: Response) => {
  const servers = serverModel.findAll();
  res.json(servers);
});

// GET /api/servers/ping-status (must be before /:id)
router.get('/ping-status', (_req: Request, res: Response) => {
  res.json(getAllPingStatuses());
});

// GET /api/servers/:id
router.get('/:id', (req: Request, res: Response) => {
  const server = serverModel.findById(req.params.id);
  if (!server) return res.status(404).json({ error: 'Server not found' });
  res.json(server);
});

// POST /api/servers — Create server and deploy agent
router.post('/', async (req: Request, res: Response) => {
  const parsed = CreateServerSchema.safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: parsed.error.flatten() });

  const { password, ...serverData } = parsed.data;

  // Create server record first (agent needs the server_id)
  const server = serverModel.create(serverData);

  try {
    await deployAgent({
      hostname: serverData.hostname,
      port: serverData.port,
      username: serverData.ssh_user,
      password,
      serverId: server.id,
      serverPort: config.port,
    });

    const updated = serverModel.findById(server.id)!;
    broadcast('server:updated', { server: updated });
    res.status(201).json(updated);
  } catch (err) {
    const errorMsg = err instanceof Error ? err.message : 'Agent deployment failed';
    logger.error({ err, hostname: serverData.hostname }, 'Agent deployment failed');

    // Delete the server record — no point keeping a failed deployment
    serverModel.delete(server.id);

    res.status(422).json({ error: errorMsg });
  }
});

// PUT /api/servers/:id
router.put('/:id', (req: Request, res: Response) => {
  const parsed = UpdateServerSchema.safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: parsed.error.flatten() });

  const server = serverModel.update(req.params.id, parsed.data as Partial<import('../models/server.js').Server>);
  if (!server) return res.status(404).json({ error: 'Server not found' });

  broadcast('server:updated', { server });
  res.json(server);
});

// DELETE /api/servers/:id
router.delete('/:id', (req: Request, res: Response) => {
  const deleted = serverModel.delete(req.params.id);
  if (!deleted) return res.status(404).json({ error: 'Server not found' });
  res.status(204).end();
});

export default router;
