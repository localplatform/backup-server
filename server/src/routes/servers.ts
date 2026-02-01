import { Router, Request, Response } from 'express';
import { serverModel, CreateServerSchema, UpdateServerSchema } from '../models/server.js';
import { sshKeyManager } from '../services/sshKeyManager.js';
import { checkRsync, installRsync } from '../services/remoteProvisioner.js';
import { getAllPingStatuses } from '../services/serverPingService.js';
import { broadcast } from '../websocket/server.js';
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

// POST /api/servers
router.post('/', async (req: Request, res: Response) => {
  const parsed = CreateServerSchema.safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: parsed.error.flatten() });

  const { password, ...serverData } = parsed.data;

  // Validate SSH connectivity before persisting
  let privateKeyPath: string;
  let publicKey: string;

  try {
    // 1. Generate SSH key in a temp location using a temp ID
    const tempId = serverData.hostname.replace(/[^a-zA-Z0-9]/g, '_') + '_' + Date.now();
    const keyResult = await sshKeyManager.generateKey(tempId);
    privateKeyPath = keyResult.privateKey;
    publicKey = keyResult.publicKey;

    // 2. Register public key on root's authorized_keys via sudo (using admin user + password)
    await sshKeyManager.registerKey(serverData.hostname, serverData.port, serverData.ssh_user, password, publicKey);
    logger.info({ hostname: serverData.hostname }, 'SSH key registered for root, testing connection...');

    // 3. Test connection as root with the key
    await sshKeyManager.testConnection(serverData.hostname, serverData.port, 'root', privateKeyPath);
    logger.info({ hostname: serverData.hostname }, 'SSH connection as root verified');
  } catch (err) {
    const errorMsg = err instanceof Error ? err.message : 'SSH setup failed';
    logger.error({ err, hostname: serverData.hostname }, 'SSH setup failed, server not created');
    return res.status(422).json({ error: errorMsg });
  }

  // SSH OK — persist the server with ssh_user='root'
  const server = serverModel.create({ ...serverData, ssh_user: 'root' });

  // Rename key files to use the real server ID
  const { renameSync } = await import('fs');
  const finalPaths = sshKeyManager.getKeyPaths(server.id);
  renameSync(privateKeyPath, finalPaths.privateKey);
  renameSync(privateKeyPath + '.pub', finalPaths.publicKey);

  const now = new Date().toISOString();
  serverModel.update(server.id, {
    ssh_key_path: finalPaths.privateKey,
    ssh_status: 'connected',
    last_seen_at: now,
  });

  // Check/install rsync as root
  try {
    const updatedServer = serverModel.findById(server.id)!;
    const hasRsync = await checkRsync(updatedServer);
    if (!hasRsync) {
      await installRsync(updatedServer);
    }
    serverModel.update(server.id, { rsync_installed: 1 });
  } catch (err) {
    logger.warn({ err, serverId: server.id }, 'rsync check/install failed (non-fatal)');
  }

  const updated = serverModel.findById(server.id)!;
  broadcast('server:updated', { server: updated });
  res.status(201).json(updated);
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
