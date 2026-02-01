import { Router, Request, Response } from 'express';
import { serverModel } from '../models/server.js';
import { sshKeyManager } from '../services/sshKeyManager.js';
import { broadcast } from '../websocket/server.js';
import { logger } from '../utils/logger.js';

const router = Router();

// POST /api/servers/:id/generate-key
router.post('/:id/generate-key', async (req: Request, res: Response) => {
  try {
    const server = serverModel.findById(req.params.id);
    if (!server) return res.status(404).json({ error: 'Server not found' });

    const { privateKey, publicKey } = await sshKeyManager.generateKey(server.id);

    const updated = serverModel.update(server.id, {
      ssh_key_path: privateKey,
      ssh_status: 'key_generated',
      ssh_error: null,
    });

    broadcast('server:updated', { server: updated });
    res.json({ publicKey });
  } catch (err) {
    logger.error({ err }, 'Failed to generate SSH key');
    res.status(500).json({ error: err instanceof Error ? err.message : 'Key generation failed' });
  }
});

// POST /api/servers/:id/register-key
router.post('/:id/register-key', async (req: Request, res: Response) => {
  try {
    const server = serverModel.findById(req.params.id);
    if (!server) return res.status(404).json({ error: 'Server not found' });
    if (!server.ssh_key_path) return res.status(400).json({ error: 'No SSH key generated' });

    const { password } = req.body;
    if (!password) return res.status(400).json({ error: 'Password required' });

    const publicKey = await sshKeyManager.getPublicKey(server.id);
    await sshKeyManager.registerKey(server.hostname, server.port, server.ssh_user, password, publicKey);

    const updated = serverModel.update(server.id, {
      ssh_status: 'key_registered',
      ssh_error: null,
    });

    broadcast('server:updated', { server: updated });
    res.json({ success: true });
  } catch (err) {
    logger.error({ err }, 'Failed to register SSH key');
    serverModel.update(req.params.id, {
      ssh_status: 'error',
      ssh_error: err instanceof Error ? err.message : 'Registration failed',
    });
    const updated = serverModel.findById(req.params.id);
    broadcast('server:updated', { server: updated });
    res.status(500).json({ error: err instanceof Error ? err.message : 'Registration failed' });
  }
});

// POST /api/servers/:id/test-connection
router.post('/:id/test-connection', async (req: Request, res: Response) => {
  try {
    const server = serverModel.findById(req.params.id);
    if (!server) return res.status(404).json({ error: 'Server not found' });
    if (!server.ssh_key_path) return res.status(400).json({ error: 'No SSH key configured' });

    await sshKeyManager.testConnection(server.hostname, server.port, server.ssh_user, server.ssh_key_path);

    const now = new Date().toISOString();
    const updated = serverModel.update(server.id, {
      ssh_status: 'connected',
      ssh_error: null,
      last_seen_at: now,
    });

    broadcast('server:status', { serverId: server.id, status: 'connected', lastSeenAt: now });
    broadcast('server:updated', { server: updated });
    res.json({ connected: true });
  } catch (err) {
    const errorMsg = err instanceof Error ? err.message : 'Connection failed';
    serverModel.update(req.params.id, { ssh_status: 'error', ssh_error: errorMsg });
    const updated = serverModel.findById(req.params.id);
    broadcast('server:status', { serverId: req.params.id, status: 'error', error: errorMsg });
    broadcast('server:updated', { server: updated });
    res.status(500).json({ error: errorMsg });
  }
});

// POST /api/servers/:id/provision
router.post('/:id/provision', async (req: Request, res: Response) => {
  try {
    const server = serverModel.findById(req.params.id);
    if (!server) return res.status(404).json({ error: 'Server not found' });

    const { checkRsync, installRsync } = await import('../services/remoteProvisioner.js');

    let installed = await checkRsync(server);
    if (!installed) {
      installed = await installRsync(server);
    }

    const updated = serverModel.update(server.id, { rsync_installed: installed ? 1 : 0 });
    broadcast('server:updated', { server: updated });
    res.json({ rsyncInstalled: installed });
  } catch (err) {
    logger.error({ err }, 'Failed to provision server');
    res.status(500).json({ error: err instanceof Error ? err.message : 'Provisioning failed' });
  }
});

export default router;
