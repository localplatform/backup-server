import { Router, Request, Response } from 'express';
import { serverModel } from '../models/server.js';
import { explorePath } from '../services/remoteExplorer.js';
import { logger } from '../utils/logger.js';

const router = Router();

// GET /api/servers/:id/explore?path=/
router.get('/:id/explore', async (req: Request, res: Response) => {
  try {
    const server = serverModel.findById(req.params.id);
    if (!server) return res.status(404).json({ error: 'Server not found' });

    const remotePath = (req.query.path as string) || '/';
    const entries = await explorePath(server, remotePath);
    res.json(entries);
  } catch (err: any) {
    logger.error({ err, serverId: req.params.id, path: req.query.path }, 'Failed to explore remote path');

    const message = err instanceof Error ? err.message : 'Exploration failed';

    if (err?.code === 3 || message.includes('Permission denied') || message.includes('EACCES')) {
      return res.status(403).json({ error: 'Permission denied', code: 'PERMISSION_DENIED', path: req.query.path });
    }

    if (err?.code === 2 || message.includes('No such file')) {
      return res.status(404).json({ error: 'Path not found', code: 'PATH_NOT_FOUND', path: req.query.path });
    }

    res.status(500).json({ error: message });
  }
});

export default router;
