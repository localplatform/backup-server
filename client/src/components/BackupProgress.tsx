import { useState, useEffect } from 'react';
import { useWebSocket } from '../hooks/useWebSocket.js';
import './BackupProgress.scss';

interface ProgressData {
  percent: number;
  checkedFiles: number;
  totalFiles: number;
  speed: string;
  currentFile: string;
}

interface Props {
  jobId: string;
}

export default function BackupProgress({ jobId }: Props) {
  const { subscribe } = useWebSocket();
  const [progress, setProgress] = useState<ProgressData | null>(null);
  const [status, setStatus] = useState<'running' | 'completed' | 'failed'>('running');
  const [summary, setSummary] = useState<{ duration: number; totalBytes: number; totalFiles: number } | null>(null);

  useEffect(() => {
    const unsubs = [
      subscribe('backup:progress', (payload) => {
        if ((payload as { jobId: string }).jobId !== jobId) return;
        const p = payload as unknown as ProgressData & { jobId: string };
        setProgress({
          percent: p.percent,
          checkedFiles: p.checkedFiles,
          totalFiles: p.totalFiles,
          speed: p.speed,
          currentFile: p.currentFile,
        });
      }),
      subscribe('backup:completed', (payload) => {
        if ((payload as { jobId: string }).jobId !== jobId) return;
        const p = payload as { duration: number; totalBytes: number; totalFiles: number };
        setStatus('completed');
        setSummary(p);
      }),
      subscribe('backup:failed', (payload) => {
        if ((payload as { jobId: string }).jobId !== jobId) return;
        setStatus('failed');
      }),
    ];

    return () => unsubs.forEach(fn => fn());
  }, [jobId, subscribe]);

  if (!progress && status === 'running') {
    return (
      <div className="backup-progress">
        <div className="progress-header">
          <span className="progress-label">Initializing...</span>
        </div>
        <div className="progress-bar">
          <div className="progress-fill" style={{ width: '0%' }} />
        </div>
        <div className="progress-info">
          <span className="progress-file">Connecting to server...</span>
        </div>
      </div>
    );
  }

  const percent = status === 'completed' ? 100 : (progress?.percent ?? 0);

  return (
    <div className="backup-progress">
      <div className="progress-header">
        <span className="progress-label">
          {status === 'completed' ? 'Completed' : status === 'failed' ? 'Failed' : `${percent}%`}
        </span>
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
      {progress && status === 'running' && (
        <div className="progress-info">
          {progress.currentFile && (
            <span className="progress-file" title={progress.currentFile}>{progress.currentFile}</span>
          )}
          <span className="progress-count">
            {progress.checkedFiles}/{progress.totalFiles} files
          </span>
        </div>
      )}
      {summary && (
        <div className="progress-summary">
          {formatBytes(summary.totalBytes)} in {summary.totalFiles} files ({(summary.duration / 1000).toFixed(1)}s)
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
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}
