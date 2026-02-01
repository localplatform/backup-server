import { z } from 'zod';
import { v4 as uuidv4 } from 'uuid';
import { getDb } from '../db/connection.js';

export const VersionStatus = z.enum(['running', 'completed', 'failed']);

export const BackupVersionSchema = z.object({
  id: z.string().uuid(),
  job_id: z.string().uuid(),
  log_id: z.string().uuid().nullable(),
  version_timestamp: z.string(),
  local_path: z.string(),
  status: VersionStatus,
  bytes_total: z.number().int(),
  files_total: z.number().int(),
  bytes_transferred: z.number().int(),
  files_transferred: z.number().int(),
  created_at: z.string(),
  completed_at: z.string().nullable(),
});

export type BackupVersion = z.infer<typeof BackupVersionSchema>;

export const backupVersionModel = {
  findAll(): BackupVersion[] {
    return getDb().prepare('SELECT * FROM backup_versions ORDER BY version_timestamp DESC').all() as BackupVersion[];
  },

  findById(id: string): BackupVersion | undefined {
    return getDb().prepare('SELECT * FROM backup_versions WHERE id = ?').get(id) as BackupVersion | undefined;
  },

  findByJobId(jobId: string): BackupVersion[] {
    return getDb().prepare(
      'SELECT * FROM backup_versions WHERE job_id = ? ORDER BY version_timestamp DESC'
    ).all(jobId) as BackupVersion[];
  },

  findLatestCompleted(jobId: string): BackupVersion | undefined {
    return getDb().prepare(
      `SELECT * FROM backup_versions
       WHERE job_id = ? AND status = 'completed'
       ORDER BY version_timestamp DESC
       LIMIT 1`
    ).get(jobId) as BackupVersion | undefined;
  },

  create(data: {
    job_id: string;
    log_id: string;
    version_timestamp: string;
    local_path: string;
  }): BackupVersion {
    const id = uuidv4();
    const now = new Date().toISOString();

    getDb().prepare(
      `INSERT INTO backup_versions
       (id, job_id, log_id, version_timestamp, local_path, created_at)
       VALUES (?, ?, ?, ?, ?, ?)`
    ).run(id, data.job_id, data.log_id, data.version_timestamp, data.local_path, now);

    return this.findById(id)!;
  },

  update(id: string, data: Partial<BackupVersion>): void {
    const fields: string[] = [];
    const values: unknown[] = [];

    for (const [key, value] of Object.entries(data)) {
      if (key === 'id' || key === 'job_id' || key === 'created_at') continue;
      fields.push(`${key} = ?`);
      values.push(value);
    }

    if (fields.length > 0) {
      values.push(id);
      getDb().prepare(`UPDATE backup_versions SET ${fields.join(', ')} WHERE id = ?`).run(...values);
    }
  },

  delete(id: string): boolean {
    const result = getDb().prepare('DELETE FROM backup_versions WHERE id = ?').run(id);
    return result.changes > 0;
  },
};
