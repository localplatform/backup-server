import { Router, Request, Response } from 'express';
import fs from 'fs';
import { execSync } from 'child_process';
import { z } from 'zod';
import { settingsModel } from '../models/settings.js';
import { backupJobModel } from '../models/backupJob.js';
import { serverModel } from '../models/server.js';
import { backupVersionModel } from '../models/backupVersion.js';
import { exploreLocal, getDiskUsage } from '../services/localExplorer.js';
import { logger } from '../utils/logger.js';

const router = Router();

const UpdateSettingsSchema = z.object({
  backup_root: z.string().min(1),
});

// GET /api/storage/hierarchy
router.get('/hierarchy', (_req: Request, res: Response) => {
  try {
    const servers = serverModel.findAll();
    const hierarchy = servers.map(server => {
      const jobs = backupJobModel.findByServerId(server.id);
      const jobsWithVersions = jobs.map(job => {
        const remotePaths = JSON.parse(job.remote_paths);
        const versions = backupVersionModel.findByJobId(job.id);
        const totalSize = versions.reduce((sum, v) => sum + v.bytes_transferred, 0);
        return {
          id: job.id,
          name: job.name,
          remote_paths: remotePaths,
          local_path: job.local_path,
          versions: versions.map(v => ({
            id: v.id,
            job_id: v.job_id,
            version_timestamp: v.version_timestamp,
            local_path: v.local_path,
            status: v.status,
            bytes_transferred: v.bytes_transferred,
            files_transferred: v.files_transferred,
            created_at: v.created_at,
            completed_at: v.completed_at,
          })),
          totalSize,
        };
      });
      const totalVersions = jobsWithVersions.reduce((sum, j) => sum + j.versions.length, 0);
      return {
        id: server.id,
        name: server.name,
        hostname: server.hostname,
        port: server.port,
        jobs: jobsWithVersions,
        totalVersions,
      };
    });
    res.json({ servers: hierarchy });
  } catch (err) {
    logger.error({ err }, 'Failed to fetch storage hierarchy');
    res.status(500).json({ error: 'Failed to fetch storage hierarchy' });
  }
});

// GET /api/storage/settings
router.get('/settings', (_req: Request, res: Response) => {
  const backupRoot = settingsModel.get('backup_root') ?? null;
  res.json({ backup_root: backupRoot });
});

// PUT /api/storage/settings
router.put('/settings', (req: Request, res: Response) => {
  const parsed = UpdateSettingsSchema.safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: parsed.error.flatten() });

  const { backup_root } = parsed.data;

  try {
    fs.accessSync(backup_root, fs.constants.R_OK);
    const stat = fs.statSync(backup_root);
    if (!stat.isDirectory()) {
      return res.status(400).json({ error: 'Path is not a directory' });
    }
  } catch {
    return res.status(400).json({ error: 'Path does not exist or is not readable' });
  }

  const oldRoot = settingsModel.get('backup_root');

  // Move existing data if root changed
  if (oldRoot && oldRoot !== backup_root) {
    try {
      fs.mkdirSync(backup_root, { recursive: true });
      const entries = fs.readdirSync(oldRoot);
      if (entries.length > 0) {
        execSync(`mv "${oldRoot}"/* "${backup_root}"/`, { stdio: 'pipe' });
      }
    } catch (err) {
      logger.warn({ err, oldRoot, newRoot: backup_root }, 'Failed to move backup data');
    }

    // Update all jobs' local_path
    const jobs = backupJobModel.findAll();
    for (const job of jobs) {
      if (job.local_path.startsWith(oldRoot)) {
        const relative = job.local_path.slice(oldRoot.length);
        backupJobModel.update(job.id, { local_path: backup_root + relative });
      }
    }
  }

  settingsModel.set('backup_root', backup_root);
  res.json({ backup_root });
});

// GET /api/storage/browse?path=/sub/dir
router.get('/browse', async (req: Request, res: Response) => {
  const backupRoot = settingsModel.get('backup_root');
  if (!backupRoot) return res.status(400).json({ error: 'Backup root not configured' });

  const subPath = (req.query.path as string) || '/';

  try {
    const entries = await exploreLocal(backupRoot, subPath);
    res.json(entries);
  } catch (err) {
    if (err instanceof Error && err.message === 'Path outside of backup root') {
      return res.status(403).json({ error: 'Access denied' });
    }
    logger.error({ err, subPath }, 'Failed to browse storage');
    res.status(500).json({ error: 'Failed to browse directory' });
  }
});

// GET /api/storage/disk-usage
router.get('/disk-usage', (_req: Request, res: Response) => {
  const backupRoot = settingsModel.get('backup_root');
  if (!backupRoot) return res.status(400).json({ error: 'Backup root not configured' });

  try {
    const usage = getDiskUsage(backupRoot);
    res.json(usage);
  } catch (err) {
    logger.error({ err }, 'Failed to get disk usage');
    res.status(500).json({ error: 'Failed to get disk usage' });
  }
});

// GET /api/storage/browse-version?version_id=xxx&path=/sub/path
router.get('/browse-version', async (req: Request, res: Response) => {
  const versionId = req.query.version_id as string | undefined;
  const subPath = (req.query.path as string) || '/';

  if (!versionId) {
    return res.status(400).json({ error: 'version_id query parameter required' });
  }

  const version = backupVersionModel.findById(versionId);
  if (!version) {
    return res.status(404).json({ error: 'Version not found' });
  }

  try {
    // Explore within the version's local_path
    // The exploreLocal function expects a root and a subPath
    // We'll pass the version's local_path as the root
    const entries = await exploreLocal(version.local_path, subPath);
    res.json(entries);
  } catch (err) {
    if (err instanceof Error && err.message === 'Path outside of backup root') {
      return res.status(403).json({ error: 'Access denied' });
    }
    logger.error({ err, versionId, subPath }, 'Failed to browse version');
    res.status(500).json({ error: 'Failed to browse version directory' });
  }
});

export default router;
