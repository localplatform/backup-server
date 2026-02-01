/**
 * Agent-based backup orchestrator - replaces rsync SSH pipeline.
 *
 * This orchestrator uses the Rust backup agent for:
 * - Delta-sync with byte-level progress tracking
 * - Real-time WebSocket progress streaming
 * - Native file system operations (no SSH overhead)
 */

import fs from 'fs';
import path from 'path';
import { BackupJob, backupJobModel, backupLogModel } from '../models/backupJob.js';
import { serverModel } from '../models/server.js';
import { backupVersionModel } from '../models/backupVersion.js';
import { broadcast } from '../websocket/server.js';
import { Semaphore } from '../utils/semaphore.js';
import { config } from '../config.js';
import { logger } from '../utils/logger.js';
import { cleanupOldVersions } from './versionCleanup.js';
import {
  createAgentClient,
  AgentClient,
  BackupProgressPayload,
  formatBytes,
  formatSpeed,
  formatDuration,
} from './agentClient.js';

// ============================================================================
// Concurrency Management
// ============================================================================

const globalSemaphore = new Semaphore(config.maxConcurrentGlobal);
const serverSemaphores = new Map<string, Semaphore>();
const jobSemaphore = new Semaphore(1); // Limit to 1 concurrent job

function getServerSemaphore(serverId: string, max: number): Semaphore {
  let sem = serverSemaphores.get(serverId);
  if (!sem) {
    sem = new Semaphore(max);
    serverSemaphores.set(serverId, sem);
  }
  return sem;
}

// ============================================================================
// Agent Connection Management
// ============================================================================

const agentClients = new Map<string, AgentClient>();

/**
 * Get or create an agent client for a server
 */
function getAgentClient(serverId: string, hostname: string, port: number = 8080): AgentClient {
  let client = agentClients.get(serverId);

  if (!client) {
    logger.info({ serverId, hostname, port }, 'Creating new agent client');

    client = createAgentClient({
      host: hostname,
      port,
      protocol: 'http', // TODO: Support HTTPS from server config
      timeout: 60000, // 60 seconds for long-running operations
    });

    // Connect WebSocket for progress streaming
    client.connectWebSocket();

    // Handle WebSocket lifecycle
    client.on('ws:connected', () => {
      logger.info({ serverId }, 'Agent WebSocket connected');
    });

    client.on('ws:disconnected', ({ code, reason }) => {
      logger.warn({ serverId, code, reason }, 'Agent WebSocket disconnected');
    });

    client.on('ws:error', (error) => {
      logger.error({ serverId, error: error.message }, 'Agent WebSocket error');
    });

    agentClients.set(serverId, client);
  }

  return client;
}

/**
 * Cleanup agent client connections
 */
export function cleanupAgentClient(serverId: string): void {
  const client = agentClients.get(serverId);
  if (client) {
    logger.info({ serverId }, 'Cleaning up agent client');
    client.destroy();
    agentClients.delete(serverId);
  }
}

/**
 * Cleanup all agent connections
 */
export function cleanupAllAgentClients(): void {
  logger.info('Cleaning up all agent clients');
  for (const [serverId, client] of agentClients) {
    client.destroy();
    agentClients.delete(serverId);
  }
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

export async function runBackupJobWithAgent(job: BackupJob, fullBackup = false): Promise<void> {
  // Acquire job-level semaphore to ensure only 1 job runs at a time
  await jobSemaphore.acquire();

  let version: ReturnType<typeof backupVersionModel.create> | null = null;

  try {
    if (runningJobs.has(job.id)) {
      logger.warn({ jobId: job.id }, 'Job already running');
      return;
    }

    const server = serverModel.findById(job.server_id);
    if (!server) {
      throw new Error('Server not found');
    }

    const remotePaths: string[] = JSON.parse(job.remote_paths);
    if (remotePaths.length === 0) {
      throw new Error('No remote paths configured');
    }

    runningJobs.add(job.id);
    backupJobModel.updateStatus(job.id, 'running');

    const log = backupLogModel.create(job.id);
    const startTime = Date.now();

    broadcast('backup:started', {
      jobId: job.id,
      serverId: server.id,
      remotePaths,
    });
    broadcast('job:updated', { job: backupJobModel.findById(job.id) });

    // Send initial progress event
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

    // Get agent client (assumes agent runs on same hostname as SSH server)
    const agentPort = parseInt(process.env.AGENT_PORT || '8080');
    const agent = getAgentClient(server.id, server.hostname, agentPort);

    // Check agent health
    try {
      const health = await agent.getHealth();
      logger.info({ serverId: server.id, health }, 'Agent health check passed');
    } catch (error) {
      logger.error({ serverId: server.id, error }, 'Agent health check failed');
      throw new Error('Agent is not available or not responding');
    }

    const serverSem = getServerSemaphore(server.id, config.maxConcurrentPerServer);
    let totalBytes = 0;
    let totalFiles = 0;
    let hasError = false;

    // Generate version timestamp
    const versionTimestamp = new Date().toISOString()
      .replace(/[:.]/g, '-')
      .replace('T', '_')
      .substring(0, 19); // YYYY-MM-DD_HH-MM-SS

    // Create version directory structure
    const versionsDir = path.join(job.local_path, 'versions');
    const versionPath = path.join(versionsDir, versionTimestamp);
    fs.mkdirSync(versionPath, { recursive: true });

    // Write backup metadata
    const metaPath = path.join(job.local_path, '.backup-meta.json');
    const meta = {
      server: { name: server.name, hostname: server.hostname, port: server.port },
      job: { id: job.id, name: job.name, remotePaths },
      agent: { enabled: true, version: await agent.getVersion().catch(() => ({ version: 'unknown' })) },
      createdAt: job.created_at,
      lastRunAt: new Date().toISOString(),
    };
    fs.writeFileSync(metaPath, JSON.stringify(meta, null, 2));

    // Create version record in DB
    version = backupVersionModel.create({
      job_id: job.id,
      log_id: log.id,
      version_timestamp: versionTimestamp,
      local_path: versionPath,
    });

    // Progress tracking (aggregated across paths)
    const taskProgress = new Map<number, BackupProgressPayload>();
    let lastBroadcastTime = 0;

    // Setup WebSocket progress listener
    const progressHandler = (progress: BackupProgressPayload) => {
      // Only handle progress for this job
      if (progress.job_id !== job.id) return;

      // Throttle broadcasts to 4x/second (250ms)
      const now = Date.now();
      if (now - lastBroadcastTime < 250) return;
      lastBroadcastTime = now;

      // Broadcast to UI with formatted data
      broadcast('backup:progress', {
        jobId: job.id,
        percent: Math.min(100, Math.max(0, progress.percent)),
        checkedFiles: progress.files_processed,
        totalFiles: progress.total_files,
        transferredBytes: progress.transferred_bytes,
        totalBytes: progress.total_bytes,
        speed: formatSpeed(progress.bytes_per_second),
        currentFile: progress.current_file || 'Processing...',
        // Per-file progress (legacy single file)
        currentFileBytes: progress.current_file_bytes,
        currentFileTotal: progress.current_file_total,
        currentFilePercent: progress.current_file_percent,
        // Active parallel transfers
        activeFiles: progress.active_files || [],
      });

      logger.debug({ jobId: job.id, progress: `${progress.percent.toFixed(1)}%` }, 'Backup progress');
    };

    // Setup completion handlers
    let completed = false;
    const completedHandler = (payload: { job_id: string; total_bytes: number; total_files?: number }) => {
      if (payload.job_id !== job.id) return;
      logger.info({ jobId: job.id, totalBytes: payload.total_bytes }, 'Backup completed on agent');
      totalBytes = payload.total_bytes;
      if (payload.total_files) totalFiles = payload.total_files;
      completed = true;
    };

    const failedHandler = (payload: { job_id: string; error: string }) => {
      if (payload.job_id !== job.id) return;
      logger.error({ jobId: job.id, error: payload.error }, 'Backup failed on agent');
      hasError = true;
    };

    agent.on('backup:progress', progressHandler);
    agent.on('backup:completed', completedHandler);
    agent.on('backup:failed', failedHandler);

    // Execute backup on agent
    try {
      await globalSemaphore.acquire();
      await serverSem.acquire();

      logger.info({ jobId: job.id, remotePaths }, 'Starting agent backup');

      // Start backup on the Rust agent
      // Use 10.10.10.100 (backup server IP) instead of localhost, since agent runs remotely
      const serverUrl = `http://10.10.10.100:${config.port || 3000}`;
      const result = await agent.startBackup({
        job_id: job.id,
        paths: remotePaths,
        server_url: serverUrl,
        token: undefined, // TODO: Implement auth tokens
      });

      logger.info({ jobId: job.id, result }, 'Agent backup started');

      // Wait for completion via WebSocket events (completed/failed handlers set the flag)
      let attempts = 0;
      const maxAttempts = 3600; // 1 hour max

      while (!completed && attempts < maxAttempts && !hasError) {
        await new Promise(resolve => setTimeout(resolve, 1000));
        attempts++;

        // Check if job was cancelled externally
        const currentJob = backupJobModel.findById(job.id);
        if (currentJob?.status === 'cancelled') {
          logger.info({ jobId: job.id }, 'Job cancelled by user');
          await agent.cancelBackup({ job_id: job.id });
          throw new Error('Job cancelled by user');
        }
      }

      if (!completed && !hasError) {
        throw new Error('Backup timed out after 1 hour');
      }

      if (hasError) {
        throw new Error('Backup failed on agent');
      }

    } finally {
      // Cleanup listeners
      agent.off('backup:progress', progressHandler);
      agent.off('backup:completed', completedHandler);
      agent.off('backup:failed', failedHandler);

      globalSemaphore.release();
      serverSem.release();
    }

    // Update job status
    const duration = Date.now() - startTime;
    const durationSecs = Math.floor(duration / 1000);

    backupJobModel.updateStatus(job.id, hasError ? 'failed' : 'completed');

    backupLogModel.update(log.id, {
      status: hasError ? 'failed' : 'completed',
      files_transferred: totalFiles,
      bytes_transferred: totalBytes,
      finished_at: new Date().toISOString(),
    });

    // Update version record
    backupVersionModel.updateCompletion(version.id, totalBytes, totalFiles);

    // Cleanup old versions
    await cleanupOldVersions(job.id, job.max_versions || 7);

    // Send completion broadcast
    broadcast('backup:completed', {
      jobId: job.id,
      totalBytes,
      totalFiles,
      duration: durationSecs,
    });
    broadcast('job:updated', { job: backupJobModel.findById(job.id) });

    // Send final 100% progress
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

    logger.info({
      jobId: job.id,
      totalBytes,
      totalFiles,
      duration: durationSecs,
      bytesPerSec: totalBytes / Math.max(durationSecs, 1),
    }, 'Backup job completed');

  } catch (error) {
    const err = error as Error;
    logger.error({ jobId: job.id, error: err.message, stack: err.stack }, 'Backup job failed');

    backupJobModel.updateStatus(job.id, 'failed');

    // Mark version as failed if it was created
    if (version) {
      backupVersionModel.updateFailed(version.id);
    }

    broadcast('backup:failed', {
      jobId: job.id,
      error: err.message,
    });
    broadcast('job:updated', { job: backupJobModel.findById(job.id) });

    throw error;

  } finally {
    runningJobs.delete(job.id);
    jobSemaphore.release();
  }
}

/**
 * Cancel a running backup job
 */
export async function cancelBackupJob(jobId: string): Promise<void> {
  logger.info({ jobId }, 'Cancelling backup job');

  const job = backupJobModel.findById(jobId);
  if (!job) {
    throw new Error('Job not found');
  }

  const server = serverModel.findById(job.server_id);
  if (!server) {
    throw new Error('Server not found');
  }

  // Get agent client and send cancel command
  const agent = agentClients.get(server.id);
  if (agent) {
    try {
      await agent.cancelBackup({ job_id: jobId });
      logger.info({ jobId }, 'Sent cancel command to agent');
    } catch (error) {
      logger.error({ jobId, error }, 'Failed to cancel job on agent');
    }
  }

  // Update job status
  backupJobModel.updateStatus(jobId, 'cancelled');

  broadcast('backup:cancelled', { jobId });
  broadcast('job:updated', { job: backupJobModel.findById(jobId) });

  runningJobs.delete(jobId);
}

/**
 * Get agent status for a server
 */
export async function getAgentStatus(serverId: string): Promise<any> {
  const server = serverModel.findById(serverId);
  if (!server) {
    throw new Error('Server not found');
  }

  const agentPort = parseInt(process.env.AGENT_PORT || '8080');
  const agent = getAgentClient(server.id, server.hostname, agentPort);

  const [health, version] = await Promise.all([
    agent.getHealth(),
    agent.getVersion(),
  ]);

  return {
    connected: agent.isWebSocketConnected(),
    health,
    version,
  };
}
