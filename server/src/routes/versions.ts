import { Router, Request, Response } from 'express';
import fs from 'fs';
import { backupVersionModel } from '../models/backupVersion.js';
import { backupJobModel } from '../models/backupJob.js';
import { broadcast } from '../websocket/server.js';
import { logger } from '../utils/logger.js';

const router = Router();

// GET /api/versions?job_id=xxx
router.get('/', (req: Request, res: Response) => {
  const jobId = req.query.job_id as string | undefined;

  if (!jobId) {
    return res.status(400).json({ error: 'job_id query parameter required' });
  }

  const versions = backupVersionModel.findByJobId(jobId);
  res.json(versions);
});

// GET /api/versions/:id
router.get('/:id', (req: Request, res: Response) => {
  const version = backupVersionModel.findById(req.params.id);
  if (!version) {
    return res.status(404).json({ error: 'Version not found' });
  }
  res.json(version);
});

// DELETE /api/versions/:id
router.delete('/:id', async (req: Request, res: Response) => {
  const version = backupVersionModel.findById(req.params.id);
  if (!version) {
    return res.status(404).json({ error: 'Version not found' });
  }

  const job = backupJobModel.findById(version.job_id);
  if (!job) {
    return res.status(404).json({ error: 'Job not found' });
  }

  try {
    // Delete from database first (transactional safety)
    backupVersionModel.delete(version.id);

    // Delete from filesystem asynchronously
    fs.rm(version.local_path, { recursive: true, force: true }, (err) => {
      if (err) {
        logger.error({ err, versionId: version.id, path: version.local_path },
          'Failed to delete version files (DB record already deleted)');
      } else {
        logger.info({ versionId: version.id, path: version.local_path },
          'Version files deleted');
      }
    });

    broadcast('version:deleted', { versionId: version.id, jobId: version.job_id });
    res.status(204).end();
  } catch (err) {
    logger.error({ err, versionId: version.id }, 'Failed to delete version');
    res.status(500).json({ error: 'Failed to delete version' });
  }
});

// DELETE /api/versions/by-job/:jobId
router.delete('/by-job/:jobId', async (req: Request, res: Response) => {
  const jobId = req.params.jobId;
  const job = backupJobModel.findById(jobId);
  if (!job) {
    return res.status(404).json({ error: 'Job not found' });
  }

  const allVersions = backupVersionModel.findByJobId(jobId);
  const toDelete = allVersions; // Delete all versions

  if (toDelete.length === 0) {
    return res.status(400).json({ error: 'No versions to delete' });
  }

  try {
    let deletedCount = 0;
    for (const version of toDelete) {
      backupVersionModel.delete(version.id);
      deletedCount++;

      // Delete from filesystem asynchronously
      fs.rm(version.local_path, { recursive: true, force: true }, (err) => {
        if (err) {
          logger.error({ err, versionId: version.id, path: version.local_path },
            'Failed to delete version files (DB record already deleted)');
        } else {
          logger.info({ versionId: version.id, path: version.local_path },
            'Version files deleted');
        }
      });
    }

    broadcast('version:bulk-deleted', { jobId, deletedCount });
    res.json({ deleted: deletedCount, kept: 0 });
  } catch (err) {
    logger.error({ err, jobId }, 'Failed to delete versions by job');
    res.status(500).json({ error: 'Failed to delete versions' });
  }
});

// DELETE /api/versions/by-server/:serverId
router.delete('/by-server/:serverId', async (req: Request, res: Response) => {
  const serverId = req.params.serverId;
  const jobs = backupJobModel.findByServerId(serverId);

  if (jobs.length === 0) {
    return res.status(404).json({ error: 'No jobs found for this server' });
  }

  let totalDeleted = 0;
  const errors: string[] = [];

  try {
    for (const job of jobs) {
      const allVersions = backupVersionModel.findByJobId(job.id);
      const toDelete = allVersions; // Delete all versions for this job

      for (const version of toDelete) {
        try {
          backupVersionModel.delete(version.id);
          totalDeleted++;

          // Delete from filesystem asynchronously
          fs.rm(version.local_path, { recursive: true, force: true }, (err) => {
            if (err) {
              logger.error({ err, versionId: version.id, path: version.local_path },
                'Failed to delete version files (DB record already deleted)');
            } else {
              logger.info({ versionId: version.id, path: version.local_path },
                'Version files deleted');
            }
          });
        } catch (err) {
          errors.push(`Failed to delete version ${version.id}: ${err instanceof Error ? err.message : 'Unknown error'}`);
        }
      }
    }

    broadcast('version:bulk-deleted', { serverId, deletedCount: totalDeleted });

    if (errors.length > 0) {
      res.json({ deleted: totalDeleted, errors });
    } else {
      res.json({ deleted: totalDeleted });
    }
  } catch (err) {
    logger.error({ err, serverId }, 'Failed to delete versions by server');
    res.status(500).json({ error: 'Failed to delete versions' });
  }
});

export default router;
