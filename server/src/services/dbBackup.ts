import fs from 'fs';
import path from 'path';
import { config } from '../config.js';
import { logger } from '../utils/logger.js';

const BACKUP_DIR = path.join(config.dataDir, 'backups');
const MAX_BACKUPS = 7; // Keep 7 daily backups

export function backupDatabase(): void {
  try {
    // Create backup directory if it doesn't exist
    if (!fs.existsSync(BACKUP_DIR)) {
      fs.mkdirSync(BACKUP_DIR, { recursive: true });
    }

    // Generate backup filename with current date (YYYY-MM-DD)
    const timestamp = new Date().toISOString().split('T')[0];
    const backupPath = path.join(BACKUP_DIR, `backup-server-${timestamp}.db`);
    const dbPath = path.join(config.dataDir, 'backup-server.db');

    // Copy database file if it exists
    if (fs.existsSync(dbPath)) {
      fs.copyFileSync(dbPath, backupPath);
      logger.info({ backupPath }, '[DB BACKUP] Created database backup');

      // Clean up old backups
      cleanOldBackups();
    } else {
      logger.warn('[DB BACKUP] Database file not found, skipping backup');
    }
  } catch (error) {
    logger.error({ error }, '[DB BACKUP] Failed to create backup');
  }
}

function cleanOldBackups(): void {
  try {
    const files = fs.readdirSync(BACKUP_DIR)
      .filter(f => f.startsWith('backup-server-') && f.endsWith('.db'))
      .map(f => ({
        name: f,
        path: path.join(BACKUP_DIR, f),
        mtime: fs.statSync(path.join(BACKUP_DIR, f)).mtime
      }))
      .sort((a, b) => b.mtime.getTime() - a.mtime.getTime());

    // Delete backups beyond MAX_BACKUPS
    const toDelete = files.slice(MAX_BACKUPS);
    toDelete.forEach(f => {
      fs.unlinkSync(f.path);
      logger.info({ backup: f.name }, '[DB BACKUP] Deleted old backup');
    });

    if (toDelete.length > 0) {
      logger.info({ kept: MAX_BACKUPS, deleted: toDelete.length }, '[DB BACKUP] Cleanup completed');
    }
  } catch (error) {
    logger.error({ error }, '[DB BACKUP] Failed to clean old backups');
  }
}
