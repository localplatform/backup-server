import fs from 'fs/promises';
import { backupVersionModel } from '../models/backupVersion.js';
import { logger } from '../utils/logger.js';

export async function cleanupOldVersions(jobId: string, maxVersions: number): Promise<void> {
  const versions = backupVersionModel.findByJobId(jobId);
  const completedVersions = versions
    .filter(v => v.status === 'completed')
    .sort((a, b) => b.version_timestamp.localeCompare(a.version_timestamp));

  if (completedVersions.length <= maxVersions) {
    return; // Nothing to clean up
  }

  const versionsToDelete = completedVersions.slice(maxVersions);

  for (const version of versionsToDelete) {
    try {
      // Delete from database first (transactional safety)
      backupVersionModel.delete(version.id);

      // Delete from filesystem (async to avoid blocking)
      await fs.rm(version.local_path, { recursive: true, force: true });

      logger.info({ versionId: version.id, jobId, path: version.local_path },
        'Deleted old backup version');
    } catch (err) {
      logger.error({ err, versionId: version.id, path: version.local_path },
        'Failed to delete old version');
    }
  }
}
