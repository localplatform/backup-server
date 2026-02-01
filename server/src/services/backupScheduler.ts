import { CronJob } from 'cron';
import { backupJobModel } from '../models/backupJob.js';
import { runBackupJob, isJobRunning } from './backupOrchestrator.js';
import { runBackupJobWithAgent, isJobRunning as isAgentJobRunning } from './agentOrchestrator.js';
import { logger } from '../utils/logger.js';

const scheduledJobs = new Map<string, CronJob>();

export function scheduleJob(jobId: string, cronExpression: string): void {
  // Remove existing schedule if any
  unscheduleJob(jobId);

  try {
    const cronJob = new CronJob(cronExpression, async () => {
      const job = backupJobModel.findById(jobId);
      if (!job || !job.enabled) return;

      // Check if job is already running (either rsync or agent)
      if (isJobRunning(jobId) || isAgentJobRunning(jobId)) {
        logger.warn({ jobId }, 'Skipping scheduled run: job already running');
        return;
      }

      // TODO: Add server-level flag to choose between rsync and agent
      // For now, use agent-based backup by default
      const useAgent = true;

      logger.info({ jobId, name: job.name, method: useAgent ? 'agent' : 'rsync' }, 'Starting scheduled backup');
      try {
        if (useAgent) {
          await runBackupJobWithAgent(job);
        } else {
          await runBackupJob(job);
        }
      } catch (err) {
        logger.error({ jobId, error: err instanceof Error ? err.message : String(err) }, 'Scheduled backup failed');
      }
    });

    cronJob.start();
    scheduledJobs.set(jobId, cronJob);
    logger.info({ jobId, cron: cronExpression }, 'Job scheduled');
  } catch (err) {
    logger.error({ jobId, cron: cronExpression, error: err instanceof Error ? err.message : String(err) }, 'Invalid cron expression');
  }
}

export function unscheduleJob(jobId: string): void {
  const existing = scheduledJobs.get(jobId);
  if (existing) {
    existing.stop();
    scheduledJobs.delete(jobId);
  }
}

export function initSchedules(): void {
  const jobs = backupJobModel.findAll();
  for (const job of jobs) {
    if (job.cron_schedule && job.enabled) {
      scheduleJob(job.id, job.cron_schedule);
    }
  }
  logger.info({ count: scheduledJobs.size }, 'Cron schedules initialized');
}

export function stopAllSchedules(): void {
  for (const [id, job] of scheduledJobs) {
    job.stop();
    scheduledJobs.delete(id);
  }
}
