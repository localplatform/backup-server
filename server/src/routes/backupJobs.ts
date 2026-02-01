import path from 'path';
import { Router, Request, Response } from 'express';
import { z } from 'zod';
import { backupJobModel, backupLogModel, CreateBackupJobSchema, UpdateBackupJobSchema } from '../models/backupJob.js';
import { serverModel } from '../models/server.js';
import { settingsModel } from '../models/settings.js';
import { runBackupJobWithAgent, cancelBackupJob, isJobRunning } from '../services/agentOrchestrator.js';
import { scheduleJob, unscheduleJob } from '../services/backupScheduler.js';
import { isAgentConnected, requestFromAgent } from '../websocket/agentRegistry.js';
import { broadcast } from '../websocket/server.js';
import { logger } from '../utils/logger.js';

function slug(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
}

function uniqueLocalPath(backupRoot: string, hostname: string, jobName: string, excludeJobId?: string): string {
  const serverSlug = slug(hostname);
  const jobSlug = slug(jobName);
  let candidate = path.join(backupRoot, serverSlug, jobSlug);

  const allJobs = backupJobModel.findAll();
  const existing = new Set(
    allJobs
      .filter(j => !excludeJobId || j.id !== excludeJobId)
      .map(j => j.local_path)
  );

  if (!existing.has(candidate)) return candidate;

  let suffix = 2;
  while (existing.has(path.join(backupRoot, serverSlug, `${jobSlug}-${suffix}`))) {
    suffix++;
  }
  return path.join(backupRoot, serverSlug, `${jobSlug}-${suffix}`);
}

const RunBackupJobSchema = z.object({
  full: z.boolean().optional().default(false),
});

const router = Router();

// GET /api/jobs
router.get('/', (_req: Request, res: Response) => {
  const jobs = backupJobModel.findAll();
  res.json(jobs);
});

// GET /api/jobs/:id
router.get('/:id', (req: Request, res: Response) => {
  const job = backupJobModel.findById(req.params.id);
  if (!job) return res.status(404).json({ error: 'Job not found' });
  res.json(job);
});

// POST /api/jobs
router.post('/', async (req: Request, res: Response) => {
  const backupRoot = settingsModel.get('backup_root');
  if (!backupRoot) return res.status(400).json({ error: 'Backup root not configured. Please configure storage first.' });

  const parsed = CreateBackupJobSchema.safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: parsed.error.flatten() });

  const server = serverModel.findById(parsed.data.server_id);
  if (!server) return res.status(400).json({ error: 'Server not found' });

  if (!isAgentConnected(server.id)) {
    return res.status(400).json({ error: 'Agent is not connected on this server.' });
  }

  // Verify paths exist on remote via agent
  try {
    const failed: string[] = [];
    for (const remotePath of parsed.data.remote_paths) {
      try {
        await requestFromAgent(server.id, {
          type: 'fs:browse',
          payload: { path: remotePath },
        });
      } catch {
        failed.push(remotePath);
      }
    }
    if (failed.length > 0) {
      return res.status(422).json({
        error: `The following paths do not exist on the remote server: ${failed.join(', ')}`,
      });
    }
  } catch (err) {
    logger.warn({ err }, 'Path verification failed, proceeding anyway');
  }

  // Generate unique local path
  parsed.data.local_path = uniqueLocalPath(backupRoot, server.name, parsed.data.name);

  const job = backupJobModel.create(parsed.data);

  if (job.cron_schedule && job.enabled) {
    scheduleJob(job.id, job.cron_schedule);
  }

  broadcast('job:created', { job });
  res.status(201).json(job);
});

// PUT /api/jobs/:id
router.put('/:id', async (req: Request, res: Response) => {
  const parsed = UpdateBackupJobSchema.safeParse(req.body);
  if (!parsed.success) return res.status(400).json({ error: parsed.error.flatten() });

  const existing = backupJobModel.findById(req.params.id);
  if (!existing) return res.status(404).json({ error: 'Job not found' });

  // If remote_paths changed, verify via agent
  if (parsed.data.remote_paths) {
    const server = serverModel.findById(existing.server_id);
    if (server && isAgentConnected(server.id)) {
      const failed: string[] = [];
      for (const remotePath of parsed.data.remote_paths) {
        try {
          await requestFromAgent(server.id, {
            type: 'fs:browse',
            payload: { path: remotePath },
          });
        } catch {
          failed.push(remotePath);
        }
      }
      if (failed.length > 0) {
        return res.status(422).json({
          error: `The following paths do not exist on the remote server: ${failed.join(', ')}`,
        });
      }
    }
  }

  // If name changed, update local_path
  if (parsed.data.name && parsed.data.name !== existing.name) {
    const server = serverModel.findById(existing.server_id);
    const backupRoot = settingsModel.get('backup_root');
    if (server && backupRoot) {
      parsed.data.local_path = uniqueLocalPath(backupRoot, server.name, parsed.data.name, existing.id);
    }
  }

  const job = backupJobModel.update(req.params.id, parsed.data);
  if (!job) return res.status(404).json({ error: 'Job not found' });

  if (job.cron_schedule && job.enabled) {
    scheduleJob(job.id, job.cron_schedule);
  } else {
    unscheduleJob(job.id);
  }

  broadcast('job:updated', { job });
  res.json(job);
});

// DELETE /api/jobs/:id
router.delete('/:id', async (req: Request, res: Response) => {
  const id = req.params.id;

  if (isJobRunning(id)) {
    await cancelBackupJob(id);
  }

  unscheduleJob(id);

  const deleted = backupJobModel.delete(id);
  if (!deleted) return res.status(404).json({ error: 'Job not found' });

  broadcast('job:deleted', { jobId: id });
  res.status(204).end();
});

// POST /api/jobs/:id/run
router.post('/:id/run', async (req: Request, res: Response) => {
  try {
    const parsed = RunBackupJobSchema.safeParse(req.body);
    if (!parsed.success) return res.status(400).json({ error: parsed.error.flatten() });

    const job = backupJobModel.findById(req.params.id);
    if (!job) return res.status(404).json({ error: 'Job not found' });

    if (isJobRunning(job.id)) {
      return res.status(409).json({ error: 'Job already running' });
    }

    // Fire and forget - Agent-based backup
    runBackupJobWithAgent(job, parsed.data.full).catch(err => {
      logger.error({ jobId: job.id, err }, 'Agent backup run failed');
    });

    res.json({ started: true });
  } catch (err) {
    res.status(500).json({ error: err instanceof Error ? err.message : 'Failed to start job' });
  }
});

// POST /api/jobs/:id/cancel
router.post('/:id/cancel', async (req: Request, res: Response) => {
  try {
    const jobId = req.params.id;

    if (!isJobRunning(jobId)) {
      return res.status(404).json({ error: 'Job not running' });
    }

    await cancelBackupJob(jobId);
    res.json({ cancelled: true });
  } catch (err) {
    logger.error({ jobId: req.params.id, err }, 'Failed to cancel job');
    res.status(500).json({ error: err instanceof Error ? err.message : 'Failed to cancel job' });
  }
});

// GET /api/jobs/:id/logs
router.get('/:id/logs', (req: Request, res: Response) => {
  const job = backupJobModel.findById(req.params.id);
  if (!job) return res.status(404).json({ error: 'Job not found' });

  const limit = parseInt(req.query.limit as string) || 50;
  const logs = backupLogModel.findByJobId(req.params.id, limit);
  res.json(logs);
});

export default router;
