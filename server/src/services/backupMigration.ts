import fs from 'fs';
import path from 'path';
import { backupJobModel } from '../models/backupJob.js';
import { backupVersionModel } from '../models/backupVersion.js';
import { logger } from '../utils/logger.js';

export async function migrateExistingBackups(): Promise<void> {
  const jobs = backupJobModel.findAll();

  for (const job of jobs) {
    try {
      // Check if already migrated (has versions/ subdirectory)
      const versionsDir = path.join(job.local_path, 'versions');
      if (fs.existsSync(versionsDir)) {
        continue; // Already migrated
      }

      // Check if backup directory has content (not just .backup-meta.json)
      if (!fs.existsSync(job.local_path)) {
        continue; // No backup yet
      }

      const entries = fs.readdirSync(job.local_path);
      const hasContent = entries.some(e => e !== '.backup-meta.json');

      if (!hasContent) {
        continue; // Empty backup, skip migration
      }

      logger.info({ jobId: job.id, path: job.local_path }, 'Migrating existing backup to versioned structure');

      // Create versions directory
      fs.mkdirSync(versionsDir, { recursive: true });

      // Create initial version timestamp based on last_run_at or current time
      const timestamp = job.last_run_at
        ? new Date(job.last_run_at).toISOString().replace(/[:.]/g, '-').replace('T', '_').substring(0, 19)
        : 'initial-migration';

      const versionPath = path.join(versionsDir, timestamp);

      // Move all content (except .backup-meta.json) into version directory
      fs.mkdirSync(versionPath, { recursive: true });
      for (const entry of entries) {
        if (entry === '.backup-meta.json' || entry === 'versions') continue;
        const src = path.join(job.local_path, entry);
        const dest = path.join(versionPath, entry);
        fs.renameSync(src, dest);
      }

      // Create version record
      const version = backupVersionModel.create({
        job_id: job.id,
        log_id: '', // No log for migrated backups
        version_timestamp: timestamp,
        local_path: versionPath,
      });

      backupVersionModel.update(version.id, {
        status: 'completed',
        completed_at: job.last_run_at || new Date().toISOString(),
      });

      // Create 'current' symlink
      const currentSymlink = path.join(job.local_path, 'current');
      fs.symlinkSync(path.join('versions', timestamp), currentSymlink);

      logger.info({ jobId: job.id, versionId: version.id }, 'Migration completed');
    } catch (err) {
      logger.error({ err, jobId: job.id }, 'Failed to migrate backup');
    }
  }
}
