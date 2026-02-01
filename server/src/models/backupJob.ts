import { z } from 'zod';
import { v4 as uuidv4 } from 'uuid';
import { getDb } from '../db/connection.js';

export const JobStatus = z.enum(['idle', 'running', 'completed', 'failed', 'cancelled']);
export type JobStatus = z.infer<typeof JobStatus>;

export const BackupJobSchema = z.object({
  id: z.string().uuid(),
  server_id: z.string().uuid(),
  name: z.string().min(1),
  remote_paths: z.string(), // JSON array
  local_path: z.string().min(1),
  cron_schedule: z.string().nullable().default(null),
  status: JobStatus.default('idle'),
  rsync_options: z.string().default(''),
  max_parallel: z.number().int().min(1).max(32).default(4),
  enabled: z.number().int().default(1),
  last_run_at: z.string().nullable().default(null),
  created_at: z.string(),
  updated_at: z.string(),
});

export type BackupJob = z.infer<typeof BackupJobSchema>;

export const CreateBackupJobSchema = z.object({
  server_id: z.string().uuid(),
  name: z.string().min(1),
  remote_paths: z.array(z.string().min(1)).min(1),
  local_path: z.string().default(''),
  cron_schedule: z.string().nullable().default(null),
  rsync_options: z.string().default(''),
  max_parallel: z.number().int().min(1).max(32).default(4),
  enabled: z.number().int().default(1),
});

export const UpdateBackupJobSchema = z.object({
  name: z.string().min(1).optional(),
  remote_paths: z.array(z.string().min(1)).min(1).optional(),
  local_path: z.string().min(1).optional(),
  cron_schedule: z.string().nullable().optional(),
  rsync_options: z.string().optional(),
  max_parallel: z.number().int().min(1).max(32).optional(),
  enabled: z.number().int().optional(),
});

export type CreateBackupJob = z.infer<typeof CreateBackupJobSchema>;
export type UpdateBackupJob = z.infer<typeof UpdateBackupJobSchema>;

export const LogStatus = z.enum(['running', 'completed', 'failed', 'cancelled']);

export const BackupLogSchema = z.object({
  id: z.string().uuid(),
  job_id: z.string().uuid(),
  started_at: z.string(),
  finished_at: z.string().nullable(),
  status: LogStatus,
  bytes_transferred: z.number().int(),
  files_transferred: z.number().int(),
  output: z.string(),
  error: z.string().nullable(),
});

export type BackupLog = z.infer<typeof BackupLogSchema>;

export const backupJobModel = {
  findAll(): BackupJob[] {
    return getDb().prepare('SELECT * FROM backup_jobs ORDER BY created_at DESC').all() as BackupJob[];
  },

  findById(id: string): BackupJob | undefined {
    return getDb().prepare('SELECT * FROM backup_jobs WHERE id = ?').get(id) as BackupJob | undefined;
  },

  findByServerId(serverId: string): BackupJob[] {
    return getDb().prepare('SELECT * FROM backup_jobs WHERE server_id = ? ORDER BY created_at DESC').all(serverId) as BackupJob[];
  },

  create(data: CreateBackupJob): BackupJob {
    const id = uuidv4();
    const now = new Date().toISOString();
    const remotePaths = JSON.stringify(data.remote_paths);
    getDb().prepare(
      `INSERT INTO backup_jobs (id, server_id, name, remote_paths, local_path, cron_schedule, rsync_options, max_parallel, enabled, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
    ).run(id, data.server_id, data.name, remotePaths, data.local_path, data.cron_schedule, data.rsync_options, data.max_parallel, data.enabled, now, now);
    return this.findById(id)!;
  },

  update(id: string, data: UpdateBackupJob): BackupJob | undefined {
    const existing = this.findById(id);
    if (!existing) return undefined;

    const fields: string[] = [];
    const values: unknown[] = [];
    for (const [key, value] of Object.entries(data)) {
      if (key === 'id' || key === 'created_at') continue;
      if (key === 'remote_paths') {
        fields.push('remote_paths = ?');
        values.push(JSON.stringify(value));
      } else {
        fields.push(`${key} = ?`);
        values.push(value);
      }
    }
    fields.push("updated_at = datetime('now')");
    values.push(id);

    getDb().prepare(`UPDATE backup_jobs SET ${fields.join(', ')} WHERE id = ?`).run(...values);
    return this.findById(id)!;
  },

  updateStatus(id: string, status: string): void {
    getDb().prepare("UPDATE backup_jobs SET status = ?, updated_at = datetime('now') WHERE id = ?").run(status, id);
  },

  delete(id: string): boolean {
    const result = getDb().prepare('DELETE FROM backup_jobs WHERE id = ?').run(id);
    return result.changes > 0;
  },
};

export const backupLogModel = {
  findByJobId(jobId: string, limit = 50): BackupLog[] {
    return getDb().prepare('SELECT * FROM backup_logs WHERE job_id = ? ORDER BY started_at DESC LIMIT ?').all(jobId, limit) as BackupLog[];
  },

  create(jobId: string): BackupLog {
    const id = uuidv4();
    const now = new Date().toISOString();
    getDb().prepare(
      `INSERT INTO backup_logs (id, job_id, started_at) VALUES (?, ?, ?)`
    ).run(id, jobId, now);
    return getDb().prepare('SELECT * FROM backup_logs WHERE id = ?').get(id) as BackupLog;
  },

  update(id: string, data: Partial<BackupLog>): void {
    const fields: string[] = [];
    const values: unknown[] = [];
    for (const [key, value] of Object.entries(data)) {
      if (key === 'id' || key === 'job_id' || key === 'started_at') continue;
      fields.push(`${key} = ?`);
      values.push(value);
    }
    values.push(id);
    if (fields.length > 0) {
      getDb().prepare(`UPDATE backup_logs SET ${fields.join(', ')} WHERE id = ?`).run(...values);
    }
  },
};
