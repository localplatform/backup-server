import { useEffect, useState } from 'react';
import { Server, Activity, FileText } from 'lucide-react';
import { useWebSocket } from '../hooks/useWebSocket.js';
import type { BackupJob } from '../api/endpoints.js';
import './JobDetailPanel.scss';

interface ActiveFile {
  path: string;
  transferred_bytes: number;
  total_bytes: number;
  percent: number;
}

interface ProgressData {
  percent: number;
  checkedFiles: number;
  totalFiles: number;
  speed: string;
  currentFile: string;
  // Per-file progress (legacy)
  currentFileBytes: number;
  currentFileTotal: number;
  currentFilePercent: number;
  // Active parallel transfers
  activeFiles: ActiveFile[];
}

interface JobDetailPanelProps {
  job: BackupJob | null;
  serverName: string;
}

export default function JobDetailPanel({ job, serverName }: JobDetailPanelProps) {
  const { subscribe } = useWebSocket();
  const [progress, setProgress] = useState<ProgressData | null>(null);
  const [status, setStatus] = useState<'running' | 'completed' | 'failed'>('running');

  useEffect(() => {
    if (!job) {
      setProgress(null);
      setStatus('running');
      return;
    }

    const unsubs = [
      subscribe('backup:progress', (payload) => {
        if ((payload as { jobId: string }).jobId !== job.id) return;
        const p = payload as any;
        setProgress({
          percent: p.percent,
          checkedFiles: p.checkedFiles,
          totalFiles: p.totalFiles,
          speed: p.speed,
          currentFile: p.currentFile,
          currentFileBytes: p.currentFileBytes || 0,
          currentFileTotal: p.currentFileTotal || 0,
          currentFilePercent: p.currentFilePercent || 0,
          activeFiles: p.activeFiles || [],
        });
        setStatus('running');
      }),
      subscribe('backup:completed', (payload) => {
        if ((payload as { jobId: string }).jobId !== job.id) return;
        setStatus('completed');
      }),
      subscribe('backup:failed', (payload) => {
        if ((payload as { jobId: string }).jobId !== job.id) return;
        setStatus('failed');
      }),
    ];

    return () => unsubs.forEach(fn => fn());
  }, [job?.id, subscribe]);

  // Empty state: no active job
  if (!job) {
    return (
      <div className="job-detail-panel empty">
        <div className="empty-content">
          <Activity size={48} strokeWidth={1.5} />
          <h3>No Active Backup</h3>
          <p>Start a job to see real-time progress</p>
        </div>
      </div>
    );
  }

  const percent = status === 'completed' ? 100 : (progress?.percent ?? 0);
  const remotePaths: string[] = JSON.parse(job.remote_paths);
  const activeFiles = progress?.activeFiles ?? [];

  return (
    <div className="job-detail-panel">
      {/* Header with job info */}
      <div className="panel-header">
        <div className="job-info">
          <h3>{job.name}</h3>
          <div className="job-meta">
            <Server size={14} />
            <span>{serverName}</span>
          </div>
        </div>
      </div>

      {/* Overall progress */}
      <div className="panel-section">
        <h4>Overall Progress</h4>
        <div className="overall-progress">
          <div className="progress-header">
            <span className="progress-percent">{percent.toFixed(2)}%</span>
            {progress?.speed && status === 'running' && (
              <span className="progress-speed">{progress.speed}</span>
            )}
          </div>
          <div className="progress-bar">
            <div
              className={`progress-fill ${status}`}
              style={{ width: `${percent}%` }}
            />
          </div>
          {progress && (
            <div className="progress-stats">
              <span>{progress.checkedFiles.toLocaleString()} / {progress.totalFiles.toLocaleString()} files</span>
            </div>
          )}
        </div>
      </div>

      {/* Active files (parallel transfers) */}
      {activeFiles.length > 0 && status === 'running' && (
        <div className="panel-section">
          <h4>Active Transfers ({activeFiles.length})</h4>
          <div className="active-files-list">
            {[...activeFiles].sort((a, b) => b.total_bytes - a.total_bytes).map((file, idx) => (
              <div key={idx} className="active-file-item">
                <div className="active-file-header">
                  <FileText size={14} />
                  <span className="active-file-name" title={file.path}>
                    {file.path.split('/').pop() || file.path}
                  </span>
                  <span className="active-file-percent">{file.percent.toFixed(1)}%</span>
                </div>
                {file.total_bytes >= 10 * 1024 * 1024 && (
                  <div className="active-file-progress">
                    <div className="progress-bar">
                      <div
                        className="progress-fill running"
                        style={{ width: `${file.percent}%` }}
                      />
                    </div>
                    <span className="active-file-size">
                      {formatBytes(file.transferred_bytes)} / {formatBytes(file.total_bytes)}
                    </span>
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Fallback: single file (when no active_files array) */}
      {activeFiles.length === 0 && progress?.currentFile && status === 'running' && (
        <div className="panel-section">
          <h4>Current File</h4>
          <div className="current-file">
            <FileText size={16} />
            <span title={progress.currentFile}>{progress.currentFile}</span>
          </div>
          {progress.currentFileTotal > 0 && (
            <div className="file-progress">
              <div className="file-progress-header">
                <span className="file-progress-percent">{progress.currentFilePercent.toFixed(1)}%</span>
                <span className="file-progress-size">
                  {formatBytes(progress.currentFileBytes)} / {formatBytes(progress.currentFileTotal)}
                </span>
              </div>
              <div className="progress-bar">
                <div
                  className="progress-fill running"
                  style={{ width: `${progress.currentFilePercent}%` }}
                />
              </div>
            </div>
          )}
        </div>
      )}

      {/* Transfer paths */}
      <div className="panel-section">
        <h4>Transfer Paths</h4>
        <div className="transfer-paths">
          {remotePaths.map((path, idx) => (
            <div key={idx} className="path-item">
              <code>{path}</code>
            </div>
          ))}
        </div>
      </div>

      {/* Status messages */}
      {!progress && status === 'running' && (
        <div className="panel-section">
          <div className="status-message initializing">
            Initializing backup...
          </div>
        </div>
      )}
      {status === 'completed' && (
        <div className="panel-section">
          <div className="status-message completed">
            Backup completed successfully
          </div>
        </div>
      )}
      {status === 'failed' && (
        <div className="panel-section">
          <div className="status-message failed">
            Backup failed
          </div>
        </div>
      )}
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}
