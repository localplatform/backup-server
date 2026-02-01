/**
 * Agent-based backup orchestrator.
 *
 * Uses the persistent WebSocket connection from the agent registry
 * to send backup commands and receive progress events.
 */

import fs from 'fs';
import path from 'path';
import { BackupJob, backupJobModel, backupLogModel } from '../models/backupJob.js';
import { serverModel } from '../models/server.js';
import { backupVersionModel } from '../models/backupVersion.js';
import { broadcast } from '../websocket/server.js';
import { sendToAgent, isAgentConnected, onAgentMessage, offAgentMessage } from '../websocket/agentRegistry.js';
import { Semaphore } from '../utils/semaphore.js';
import { config } from '../config.js';
import { logger } from '../utils/logger.js';
import { cleanupOldVersions } from './versionCleanup.js';

// ============================================================================
// Concurrency Management
// ============================================================================

const globalSemaphore = new Semaphore(config.maxConcurrentGlobal);
const serverSemaphores = new Map<string, Semaphore>();
const jobSemaphore = new Semaphore(1);

function getServerSemaphore(serverId: string, max: number): Semaphore {
  let sem = serverSemaphores.get(serverId);
  if (!sem) {
    sem = new Semaphore(max);
    serverSemaphores.set(serverId, sem);
  }
  return sem;
}

// ============================================================================
// Job Tracking
// ============================================================================

const runningJobs = new Set<string>();

export function isJobRunning(jobId: string): boolean {
  return runningJobs.has(jobId);
}

// ============================================================================
// Main Backup Orchestration
// ============================================================================

function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec < 1024) return `${bytesPerSec} B/s`;
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(1)} KB/s`;
  if (bytesPerSec < 1024 * 1024 * 1024) return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`;
  return `${(bytesPerSec / (1024 * 1024 * 1024)).toFixed(1)} GB/s`;
}

export async function runBackupJobWithAgent(job: BackupJob, fullBackup = false): Promise<void> {
  await jobSemaphore.acquire();

  let version: ReturnType<typeof backupVersionModel.create> | null = null;

  try {
    if (runningJobs.has(job.id)) {
      logger.warn({ jobId: job.id }, 'Job already running');
      return;
    }

    const server = serverModel.findById(job.server_id);
    if (!server) throw new Error('Server not found');

    if (!isAgentConnected(server.id)) {
      throw new Error('Agent is not connected');
    }

    const remotePaths: string[] = JSON.parse(job.remote_paths);
    if (remotePaths.length === 0) throw new Error('No remote paths configured');

    runningJobs.add(job.id);
    backupJobModel.updateStatus(job.id, 'running');

    const log = backupLogModel.create(job.id);
    const startTime = Date.now();

    broadcast('backup:started', { jobId: job.id, serverId: server.id, remotePaths });
    broadcast('job:updated', { job: backupJobModel.findById(job.id) });

    broadcast('backup:progress', {
      jobId: job.id,
      percent: 0,
      checkedFiles: 0,
      totalFiles: 0,
      transferredBytes: 0,
      totalBytes: 0,
      speed: '',
      currentFile: 'Initializing agent backup...',
    });

    const serverSem = getServerSemaphore(server.id, config.maxConcurrentPerServer);
    let totalBytes = 0;
    let totalFiles = 0;
    let hasError = false;
    let errorMessage = '';

    // Generate version timestamp
    const versionTimestamp = new Date().toISOString()
      .replace(/[:.]/g, '-')
      .replace('T', '_')
      .substring(0, 19);

    const versionsDir = path.join(job.local_path, 'versions');
    const versionPath = path.join(versionsDir, versionTimestamp);
    fs.mkdirSync(versionPath, { recursive: true });

    // Write backup metadata
    const metaPath = path.join(job.local_path, '.backup-meta.json');
    const meta = {
      server: { name: server.name, hostname: server.hostname, port: server.port },
      job: { id: job.id, name: job.name, remotePaths },
      agent: { enabled: true },
      createdAt: job.created_at,
      lastRunAt: new Date().toISOString(),
    };
    fs.writeFileSync(metaPath, JSON.stringify(meta, null, 2));

    version = backupVersionModel.create({
      job_id: job.id,
      log_id: log.id,
      version_timestamp: versionTimestamp,
      local_path: versionPath,
    });

    // Setup progress handlers via agent message handlers
    let lastBroadcastTime = 0;
    let completed = false;

    const progressHandler = (serverId: string, payload: Record<string, unknown>) => {
      if (payload.job_id !== job.id) return;

      const now = Date.now();
      if (now - lastBroadcastTime < 250) return;
      lastBroadcastTime = now;

      broadcast('backup:progress', {
        jobId: job.id,
        percent: Math.min(100, Math.max(0, payload.percent as number)),
        checkedFiles: payload.files_processed,
        totalFiles: payload.total_files,
        transferredBytes: payload.transferred_bytes,
        totalBytes: payload.total_bytes,
        speed: formatSpeed(payload.bytes_per_second as number || 0),
        currentFile: payload.current_file || 'Processing...',
        currentFileBytes: payload.current_file_bytes,
        currentFileTotal: payload.current_file_total,
        currentFilePercent: payload.current_file_percent,
        activeFiles: payload.active_files || [],
      });
    };

    const completedHandler = (serverId: string, payload: Record<string, unknown>) => {
      if (payload.job_id !== job.id) return;
      totalBytes = payload.total_bytes as number;
      if (payload.total_files) totalFiles = payload.total_files as number;
      completed = true;
    };

    const failedHandler = (serverId: string, payload: Record<string, unknown>) => {
      if (payload.job_id !== job.id) return;
      hasError = true;
      errorMessage = payload.error as string || 'Backup failed on agent';
    };

    onAgentMessage('backup:progress', progressHandler);
    onAgentMessage('backup:completed', completedHandler);
    onAgentMessage('backup:failed', failedHandler);

    try {
      await globalSemaphore.acquire();
      await serverSem.acquire();

      logger.info({ jobId: job.id, remotePaths }, 'Starting agent backup via WebSocket');

      // Send backup start command via persistent WebSocket
      const sent = sendToAgent(server.id, {
        type: 'backup:start',
        payload: {
          job_id: job.id,
          paths: remotePaths,
        },
      });

      if (!sent) throw new Error('Failed to send backup command to agent');

      // Wait for completion via WebSocket events
      let attempts = 0;
      const maxAttempts = 3600; // 1 hour max

      while (!completed && attempts < maxAttempts && !hasError) {
        await new Promise(resolve => setTimeout(resolve, 1000));
        attempts++;

        // Check if job was cancelled externally
        const currentJob = backupJobModel.findById(job.id);
        if (currentJob?.status === 'cancelled') {
          sendToAgent(server.id, { type: 'backup:cancel', payload: { job_id: job.id } });
          throw new Error('Job cancelled by user');
        }

        // Check if agent disconnected
        if (!isAgentConnected(server.id)) {
          throw new Error('Agent disconnected during backup');
        }
      }

      if (!completed && !hasError) throw new Error('Backup timed out after 1 hour');
      if (hasError) throw new Error(errorMessage);

    } finally {
      offAgentMessage('backup:progress', progressHandler);
      offAgentMessage('backup:completed', completedHandler);
      offAgentMessage('backup:failed', failedHandler);

      globalSemaphore.release();
      serverSem.release();
    }

    // Update job status
    const duration = Date.now() - startTime;
    const durationSecs = Math.floor(duration / 1000);

    backupJobModel.updateStatus(job.id, 'completed');

    backupLogModel.update(log.id, {
      status: 'completed',
      files_transferred: totalFiles,
      bytes_transferred: totalBytes,
      finished_at: new Date().toISOString(),
    });

    backupVersionModel.updateCompletion(version.id, totalBytes, totalFiles);
    await cleanupOldVersions(job.id, job.max_versions || 7);

    broadcast('backup:completed', { jobId: job.id, totalBytes, totalFiles, duration: durationSecs });
    broadcast('job:updated', { job: backupJobModel.findById(job.id) });

    broadcast('backup:progress', {
      jobId: job.id,
      percent: 100,
      checkedFiles: totalFiles,
      totalFiles,
      transferredBytes: totalBytes,
      totalBytes,
      speed: '',
      currentFile: 'Completed',
    });

    logger.info({ jobId: job.id, totalBytes, totalFiles, duration: durationSecs }, 'Backup job completed');

  } catch (error) {
    const err = error as Error;
    logger.error({ jobId: job.id, error: err.message }, 'Backup job failed');

    backupJobModel.updateStatus(job.id, 'failed');

    if (version) backupVersionModel.updateFailed(version.id);

    broadcast('backup:failed', { jobId: job.id, error: err.message });
    broadcast('job:updated', { job: backupJobModel.findById(job.id) });

    throw error;

  } finally {
    runningJobs.delete(job.id);
    jobSemaphore.release();
  }
}

/**
 * Cancel a running backup job via the agent WebSocket.
 */
export async function cancelBackupJob(jobId: string): Promise<void> {
  logger.info({ jobId }, 'Cancelling backup job');

  const job = backupJobModel.findById(jobId);
  if (!job) throw new Error('Job not found');

  const server = serverModel.findById(job.server_id);
  if (!server) throw new Error('Server not found');

  if (isAgentConnected(server.id)) {
    sendToAgent(server.id, { type: 'backup:cancel', payload: { job_id: jobId } });
    logger.info({ jobId }, 'Sent cancel command to agent');
  }

  backupJobModel.updateStatus(jobId, 'cancelled');
  broadcast('backup:cancelled', { jobId });
  broadcast('job:updated', { job: backupJobModel.findById(jobId) });
  runningJobs.delete(jobId);
}
