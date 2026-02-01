import { spawn, ChildProcess } from 'child_process';
import { logger } from '../utils/logger.js';

export interface RsyncProgress {
  totalFiles: number;
  checkedFiles: number;
  totalBytes: number;        // NEW: Total bytes in transfer
  transferredBytes: number;  // NEW: Bytes transferred so far
  speed: string;
  currentFile: string;
  percentComplete: number;   // NEW: Calculated percentage (0-100)
}

export interface RsyncResult {
  code: number;
  bytesTransferred: number;
  filesTransferred: number;
  output: string;
}

// Parse rsync --info=progress2 output (byte-level progress)
// Example: "  1,234,567  45%  12.34MB/s    0:00:05 (xfr#1, to-chk=99/100)"
// Supports both comma (1,234,567) and European period (6.459.772.491) separators
const progress2Regex = /^\s*([\d,.]+)\s+(\d+)%\s+([\d.]+[kMGT]?B\/s)/;
const toChkRegex = /to-chk=(\d+)\/(\d+)\)/;
const speedRegex = /([\d.]+[kMGT]?B\/s)/;

// Track running processes for cancellation
const runningProcesses = new Map<string, ChildProcess>();

export function cancelRsync(key: string): boolean {
  const proc = runningProcesses.get(key);
  if (proc) {
    proc.kill('SIGTERM');
    runningProcesses.delete(key);
    return true;
  }
  return false;
}

export function cancelAllForJob(jobId: string): void {
  for (const [key, proc] of runningProcesses) {
    if (key.startsWith(jobId)) {
      proc.kill('SIGTERM');
      runningProcesses.delete(key);
    }
  }
}

export function runRsync(
  remotePath: string,
  localPath: string,
  sshKeyPath: string,
  hostname: string,
  sshUser: string,
  sshPort: number,
  extraOptions: string,
  processKey: string,
  previousVersionPath: string | undefined,
  onProgress: (progress: RsyncProgress) => void
): Promise<RsyncResult> {
  return new Promise((resolve, reject) => {
    const sshCmd = `ssh -i ${sshKeyPath} -p ${sshPort} -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null`;
    const args = [
      '-az',
      '--info=progress2',      // CHANGED: Byte-level progress (requires rsync 3.1.0+)
      '--no-inc-recursive',    // NEW: Scan full file list upfront for accurate progress
      '--outbuf=L',            // NEW: Force line-buffering (supplements stdbuf)
      '--stats',
      '--partial',
      '-e', sshCmd,
      ...(previousVersionPath ? ['--link-dest', previousVersionPath] : []),
      ...(extraOptions ? extraOptions.split(/\s+/).filter(Boolean) : []),
      `${sshUser}@${hostname}:${remotePath}`,
      localPath,
    ];

    logger.info({ args: ['rsync', ...args] }, 'Starting rsync');

    // Use stdbuf to unbuffer output for real-time progress updates
    // Fallback to direct rsync if stdbuf not available
    let proc: ChildProcess;
    try {
      proc = spawn('stdbuf', ['-o0', 'rsync', ...args]);
      runningProcesses.set(processKey, proc);

      // If stdbuf fails to start, fall back to direct rsync
      proc.on('error', (err: NodeJS.ErrnoException) => {
        if (err.code === 'ENOENT') {
          logger.warn('stdbuf not available, falling back to direct rsync spawn');
          runningProcesses.delete(processKey);
          proc = spawn('rsync', args);
          runningProcesses.set(processKey, proc);
        }
      });
    } catch (err) {
      logger.warn({ err }, 'Failed to spawn with stdbuf, using direct rsync');
      proc = spawn('rsync', args);
      runningProcesses.set(processKey, proc);
    }

    let output = '';
    let lastFile = '';
    let totalBytes = 0;
    let totalFiles = 0;
    let transferredBytes = 0;

    proc.stdout?.on('data', (data: Buffer) => {
      const text = data.toString();
      output += text;

      const lines = text.split('\n');
      for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed) continue;

        // Try to match progress2 format first (byte-level progress)
        const prog2Match = progress2Regex.exec(trimmed);
        if (prog2Match) {
          // Remove ALL separators (both commas and periods) for European/US format compatibility
          transferredBytes = parseInt(prog2Match[1].replace(/[.,]/g, ''), 10);
          const percent = parseInt(prog2Match[2], 10);
          const speed = prog2Match[3];

          // Extract to-chk if present on same line for file counts
          const chkMatch = toChkRegex.exec(trimmed);
          if (chkMatch) {
            const remaining = parseInt(chkMatch[1], 10);
            const total = parseInt(chkMatch[2], 10);
            totalFiles = total;

            onProgress({
              totalFiles: total,
              checkedFiles: total - remaining,
              totalBytes: totalBytes || 0,  // Won't have this until completion
              transferredBytes: transferredBytes,
              speed: speed,
              currentFile: lastFile,
              percentComplete: percent,
            });
          } else {
            // progress2 without to-chk info
            onProgress({
              totalFiles: totalFiles,
              checkedFiles: 0,
              totalBytes: totalBytes || 0,
              transferredBytes: transferredBytes,
              speed: speed,
              currentFile: lastFile,
              percentComplete: percent,
            });
          }
          continue;
        }

        // Fallback: file-based progress (to-chk pattern only)
        const chkMatch = toChkRegex.exec(trimmed);
        if (chkMatch) {
          const remaining = parseInt(chkMatch[1], 10);
          const total = parseInt(chkMatch[2], 10);
          const spdMatch = speedRegex.exec(trimmed);
          totalFiles = total;

          onProgress({
            totalFiles: total,
            checkedFiles: total - remaining,
            totalBytes: totalBytes || 0,
            transferredBytes: transferredBytes,
            speed: spdMatch ? spdMatch[1] : '',
            currentFile: lastFile,
            percentComplete: total > 0 ? Math.round(((total - remaining) / total) * 100) : 0,
          });
        } else if (
          !trimmed.match(/^\s*[\d,]+\s+\d+%/) &&
          !trimmed.startsWith('sending') &&
          !trimmed.startsWith('total') &&
          !trimmed.startsWith('Number') &&
          !trimmed.startsWith('Total') &&
          !trimmed.startsWith('Literal') &&
          !trimmed.startsWith('Matched') &&
          !trimmed.startsWith('File list') &&
          !trimmed.startsWith('sent ')
        ) {
          lastFile = trimmed;
        }
      }
    });

    proc.stderr?.on('data', (data: Buffer) => {
      output += data.toString();
    });

    proc.on('close', (code) => {
      runningProcesses.delete(processKey);

      // Parse final stats from output
      const filesMatch = /Number of regular files transferred: ([\d,]+)/.exec(output);
      if (filesMatch) {
        totalFiles = parseInt(filesMatch[1].replace(/,/g, ''), 10);
      }

      // Parse total file size from rsync stats
      const bytesMatch = /Total file size: ([\d,]+)/.exec(output);
      if (bytesMatch) {
        totalBytes = parseInt(bytesMatch[1].replace(/,/g, ''), 10);
      } else {
        // Fallback: try "Total transferred file size"
        const transferredMatch = /Total transferred file size: ([\d,]+)/.exec(output);
        if (transferredMatch) {
          totalBytes = parseInt(transferredMatch[1].replace(/,/g, ''), 10);
        }
      }

      // Use final transferredBytes if we tracked it, otherwise use totalBytes
      const finalBytes = transferredBytes > 0 ? transferredBytes : totalBytes;

      resolve({
        code: code ?? 1,
        bytesTransferred: finalBytes,
        filesTransferred: totalFiles,
        output,
      });
    });

    proc.on('error', (err) => {
      runningProcesses.delete(processKey);
      reject(err);
    });
  });
}
