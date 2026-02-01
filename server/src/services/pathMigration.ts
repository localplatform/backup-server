import fs from 'fs';
import path from 'path';
import { serverModel } from '../models/server.js';
import { backupJobModel } from '../models/backupJob.js';
import { backupVersionModel } from '../models/backupVersion.js';
import { settingsModel } from '../models/settings.js';
import { logger } from '../utils/logger.js';

function slug(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
}

export async function migrateServerFolderNames(): Promise<void> {
  const backupRoot = settingsModel.get('backup_root');
  if (!backupRoot) {
    logger.info('[PATH_MIGRATION] No backup root configured, skipping migration');
    return;
  }

  logger.info('[PATH_MIGRATION] Starting server folder name migration...');

  const servers = serverModel.findAll();
  const jobs = backupJobModel.findAll();

  let migratedCount = 0;
  let skippedCount = 0;

  for (const server of servers) {
    const serverJobs = jobs.filter(j => j.server_id === server.id);
    if (serverJobs.length === 0) {
      logger.debug({ serverId: server.id, serverName: server.name },
        '[PATH_MIGRATION] No jobs for this server, skipping');
      continue;
    }

    const oldSlug = slug(server.hostname);
    const newSlug = slug(server.name);

    if (oldSlug === newSlug) {
      logger.debug({ serverId: server.id, serverName: server.name, slug: oldSlug },
        '[PATH_MIGRATION] Server already uses name-based path, skipping');
      continue;
    }

    const oldPath = path.join(backupRoot, oldSlug);
    const newPath = path.join(backupRoot, newSlug);

    if (!fs.existsSync(oldPath)) {
      logger.debug({ serverId: server.id, serverName: server.name, oldPath },
        '[PATH_MIGRATION] Old path does not exist, skipping');
      continue;
    }

    if (fs.existsSync(newPath)) {
      logger.warn({ serverId: server.id, serverName: server.name, oldPath, newPath },
        '[PATH_MIGRATION] Target path already exists, skipping migration (manual intervention required)');
      skippedCount++;
      continue;
    }

    logger.info({ serverId: server.id, serverName: server.name, oldPath, newPath },
      '[PATH_MIGRATION] Migrating server folder from hostname-based to name-based path');

    try {
      // 1. Rename the folder on disk (atomic operation)
      fs.renameSync(oldPath, newPath);

      // 2. Update all jobs for this server
      for (const job of serverJobs) {
        if (job.local_path.startsWith(oldPath)) {
          const newJobPath = job.local_path.replace(oldPath, newPath);

          logger.debug({ jobId: job.id, oldJobPath: job.local_path, newJobPath },
            '[PATH_MIGRATION] Updating job path');

          backupJobModel.update(job.id, { local_path: newJobPath } as never);

          // 3. Update all version records for this job
          const versions = backupVersionModel.findByJobId(job.id);
          for (const version of versions) {
            if (version.local_path.startsWith(oldPath)) {
              const newVersionPath = version.local_path.replace(oldPath, newPath);

              logger.debug({ versionId: version.id, oldVersionPath: version.local_path, newVersionPath },
                '[PATH_MIGRATION] Updating version path');

              backupVersionModel.update(version.id, { local_path: newVersionPath });
            }
          }
        }
      }

      logger.info({ serverId: server.id, serverName: server.name },
        '[PATH_MIGRATION] Successfully migrated server folder');
      migratedCount++;

    } catch (err) {
      logger.error({ err, serverId: server.id, serverName: server.name, oldPath, newPath },
        '[PATH_MIGRATION] Failed to migrate server folder');
      skippedCount++;
    }
  }

  logger.info({ migratedCount, skippedCount },
    '[PATH_MIGRATION] Server folder name migration completed');
}
