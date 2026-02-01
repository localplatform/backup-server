import fs from 'fs';
import path from 'path';
import { BackupJob, backupJobModel, backupLogModel } from '../models/backupJob.js';
import { serverModel } from '../models/server.js';
import { backupVersionModel } from '../models/backupVersion.js';
import { runRsync, cancelAllForJob } from './rsyncRunner.js';
import { broadcast } from '../websocket/server.js';
import { Semaphore } from '../utils/semaphore.js';
import { config } from '../config.js';
import { logger } from '../utils/logger.js';
import { cleanupOldVersions } from './versionCleanup.js';

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

const runningJobs = new Set<string>();

export function isJobRunning(jobId: string): boolean {
  return runningJobs.has(jobId);
}

export async function runBackupJob(job: BackupJob, fullBackup = false): Promise<void> {
  // Acquire job-level semaphore to ensure only 1 job runs at a time
  await jobSemaphore.acquire();

  try {
    if (runningJobs.has(job.id)) {
      logger.warn({ jobId: job.id }, 'Job already running');
      return;
    }

    const server = serverModel.findById(job.server_id);
    if (!server || !server.ssh_key_path) {
      throw new Error('Server not found or SSH key not configured');
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

    // Send initial progress event immediately
    broadcast('backup:progress', {
      jobId: job.id,
      percent: 0,
      checkedFiles: 0,
      totalFiles: 0,
      speed: '',
      currentFile: 'Initializing backup...',
    });

    const serverSem = getServerSemaphore(server.id, config.maxConcurrentPerServer);
    let totalBytes = 0;
    let totalFiles = 0;
    let hasError = false;

    // Generate version timestamp
    const versionTimestamp = new Date().toISOString()
      .replace(/[:.]/g, '-')
      .replace('T', '_')
      .substring(0, 19); // YYYY-MM-DD_HHMMSS

    // Create version directory structure
    const versionsDir = path.join(job.local_path, 'versions');
    const versionPath = path.join(versionsDir, versionTimestamp);
    fs.mkdirSync(versionPath, { recursive: true });

    // Write backup metadata for recovery/identification
    const metaPath = path.join(job.local_path, '.backup-meta.json');
    const meta = {
      server: { name: server.name, hostname: server.hostname, port: server.port },
      job: { id: job.id, name: job.name, remotePaths },
      createdAt: job.created_at,
      lastRunAt: new Date().toISOString(),
    };
    fs.writeFileSync(metaPath, JSON.stringify(meta, null, 2));

    // Create version record in DB
    const version = backupVersionModel.create({
      job_id: job.id,
      log_id: log.id,
      version_timestamp: versionTimestamp,
      local_path: versionPath,
    });

    // Find previous successful version for --link-dest (skip if full backup requested)
    const previousVersion = fullBackup ? undefined : backupVersionModel.findLatestCompleted(job.id);

    // Aggregated progress tracking across all tasks
    const taskProgress = new Map<number, {
      totalFiles: number;
      checkedFiles: number;
      totalBytes: number;        // NEW: Total bytes in transfer
      transferredBytes: number;  // NEW: Bytes transferred so far
      speed: string;
      currentFile: string;
      percentComplete: number;   // NEW: Percentage from rsync
    }>();
    let lastBroadcastPercent = 0;
    let lastBroadcastTime = 0;

    const tasks = remotePaths.map((remotePath, taskIndex) => {
      return (async () => {
        await globalSemaphore.acquire();
        await serverSem.acquire();

        try {
          const processKey = `${job.id}-${taskIndex}`;
          const result = await runRsync(
            remotePath,
            versionPath,
            server.ssh_key_path!,
            server.hostname,
            server.ssh_user,
            server.port,
            job.rsync_options,
            processKey,
            previousVersion?.local_path,
            (progress) => {
              // Store progress for this task (preserve last known speed)
              const existing = taskProgress.get(taskIndex);
              taskProgress.set(taskIndex, {
                ...progress,
                speed: progress.speed || existing?.speed || '',
              });

              // Throttle broadcasts to 4x/second (250ms) for smooth updates
              const now = Date.now();
              if (now - lastBroadcastTime < 250) return;
              lastBroadcastTime = now;

              // Aggregate progress across all tasks
              let globalTotalFiles = 0;
              let globalCheckedFiles = 0;
              let globalTotalBytes = 0;
              let globalTransferredBytes = 0;
              let latestSpeed = '';
              let latestFile = '';

              for (const [, tp] of taskProgress) {
                globalTotalFiles += tp.totalFiles;
                globalCheckedFiles += tp.checkedFiles;
                globalTotalBytes += tp.totalBytes || 0;
                globalTransferredBytes += tp.transferredBytes || 0;
                if (tp.speed) latestSpeed = tp.speed;
                if (tp.currentFile) latestFile = tp.currentFile;
              }

              // Calculate percentage: prefer byte-based, fallback to file-based
              let percent = 0;
              if (globalTotalBytes > 0) {
                percent = Math.round((globalTransferredBytes / globalTotalBytes) * 100);
              } else if (globalTotalFiles > 0) {
                percent = Math.round((globalCheckedFiles / globalTotalFiles) * 100);
              }

              // Clamp to prevent decreasing percentages
              const clampedPercent = Math.max(percent, lastBroadcastPercent);
              lastBroadcastPercent = clampedPercent;

              broadcast('backup:progress', {
                jobId: job.id,
                percent: clampedPercent,
                checkedFiles: globalCheckedFiles,
                totalFiles: globalTotalFiles,
                transferredBytes: globalTransferredBytes,  // NEW
                totalBytes: globalTotalBytes,              // NEW
                speed: latestSpeed,
                currentFile: latestFile,
              });
            }
          );

          if (result.code === 0 || result.code === 23 || result.code === 24) {
            // 23 = partial transfer, 24 = some files vanished (ok for backups)
            totalBytes += result.bytesTransferred;
            totalFiles += result.filesTransferred;

            broadcast('backup:task-completed', {
              jobId: job.id,
              taskIndex,
              remotePath,
              bytes: result.bytesTransferred,
              files: result.filesTransferred,
            });
          } else {
            hasError = true;
            logger.error({ jobId: job.id, taskIndex, remotePath, code: result.code, output: result.output }, 'Rsync exited with error');
            broadcast('backup:failed', {
              jobId: job.id,
              taskIndex,
              error: `rsync exited with code ${result.code}`,
              remotePath,
            });
          }
        } catch (err) {
          hasError = true;
          const msg = err instanceof Error ? err.message : String(err);
          broadcast('backup:failed', {
            jobId: job.id,
            taskIndex,
            error: msg,
            remotePath,
          });
          logger.error({ jobId: job.id, taskIndex, remotePath, error: msg }, 'Rsync task failed');
        } finally {
          serverSem.release();
          globalSemaphore.release();
        }
      })();
    });

    await Promise.allSettled(tasks);

    const duration = Date.now() - startTime;
    const finalStatus = hasError ? 'failed' : 'completed';

    // Update version status
    backupVersionModel.update(version.id, {
      status: finalStatus,
      bytes_transferred: totalBytes,
      files_transferred: totalFiles,
      completed_at: new Date().toISOString(),
    });

    // Write version metadata
    const versionMetaPath = path.join(versionPath, '.version-meta.json');
    fs.writeFileSync(versionMetaPath, JSON.stringify({
      version_id: version.id,
      timestamp: versionTimestamp,
      bytes_transferred: totalBytes,
      files_transferred: totalFiles,
      status: finalStatus,
    }, null, 2));

    if (finalStatus === 'completed') {
      // Update 'current' symlink to point to latest version
      const currentSymlink = path.join(job.local_path, 'current');
      try {
        if (fs.existsSync(currentSymlink)) {
          fs.unlinkSync(currentSymlink);
        }
        fs.symlinkSync(path.join('versions', versionTimestamp), currentSymlink);
      } catch (err) {
        logger.warn({ err, jobId: job.id }, 'Failed to update current symlink');
      }

      // Apply retention policy
      await cleanupOldVersions(job.id, job.max_versions);
    }

    backupJobModel.updateStatus(job.id, finalStatus === 'failed' ? 'failed' : 'idle');
    backupJobModel.update(job.id, { last_run_at: new Date().toISOString() } as never);

    backupLogModel.update(log.id, {
      finished_at: new Date().toISOString(),
      status: finalStatus,
      bytes_transferred: totalBytes,
      files_transferred: totalFiles,
    });

    runningJobs.delete(job.id);

    broadcast('backup:completed', {
      jobId: job.id,
      duration,
      totalBytes,
      totalFiles,
    });
    broadcast('job:updated', { job: backupJobModel.findById(job.id) });

    logger.info({ jobId: job.id, duration, totalBytes, totalFiles, status: finalStatus }, 'Backup job finished');
  } finally {
    // Always release the job semaphore, even on error
    jobSemaphore.release();
  }
}

export function cancelBackupJob(jobId: string): boolean {
  if (!runningJobs.has(jobId)) return false;

  cancelAllForJob(jobId);
  runningJobs.delete(jobId);
  backupJobModel.updateStatus(jobId, 'cancelled');
  broadcast('job:updated', { job: backupJobModel.findById(jobId) });
  return true;
}
