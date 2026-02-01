import { useState } from 'react';
import { Plus, Play, Square, Trash2, Clock, Server, FolderOpen, HardDrive } from 'lucide-react';
import { useBackupJobs, useCreateJob, useUpdateJob, useDeleteJob, useRunJob, useCancelJob } from '../hooks/useBackupJobs.js';
import { useServers } from '../hooks/useServers.js';
import { useServerPingStatus } from '../hooks/useServerPing.js';
import { useWebSocket } from '../hooks/useWebSocket.js';
import StatusBadge from '../components/StatusBadge.js';
import PageHeader from '../components/PageHeader.js';
import BackupJobForm from '../components/BackupJobForm.js';
import JobDetailPanel from '../components/JobDetailPanel.js';
import type { BackupJob } from '../api/endpoints.js';
import './BackupJobs.scss';

export default function BackupJobs() {
  const { connected } = useWebSocket(); // Ensure WebSocket connection is maintained
  const { data: jobs = [], isLoading } = useBackupJobs();
  const { data: servers = [] } = useServers();
  const createJob = useCreateJob();
  const updateJob = useUpdateJob();
  const deleteJob = useDeleteJob();
  const runJob = useRunJob();
  const cancelJob = useCancelJob();
  const pingStatuses = useServerPingStatus();
  const [showForm, setShowForm] = useState(false);
  const [editingJob, setEditingJob] = useState<BackupJob | null>(null);
  const [showBackupTypeModal, setShowBackupTypeModal] = useState(false);
  const [pendingJobId, setPendingJobId] = useState<string | null>(null);

  const getServerName = (serverId: string) => {
    const server = servers.find(s => s.id === serverId);
    return server?.name || 'Unknown';
  };

  const running = jobs.filter(j => j.status === 'running').length;
  const failed = jobs.filter(j => j.status === 'failed').length;

  // Group jobs by server
  const grouped = jobs.reduce<Map<string, BackupJob[]>>((map, job) => {
    const list = map.get(job.server_id) || [];
    list.push(job);
    map.set(job.server_id, list);
    return map;
  }, new Map());

  // Determine active job (first running job)
  const activeJob = jobs.find(j => j.status === 'running') || null;

  return (
    <div className="jobs-page">
      <PageHeader
        icon={<HardDrive size={22} />}
        title="Backup Jobs"
        stats={
          jobs.length > 0
            ? [
                { label: `${jobs.length} job${jobs.length > 1 ? 's' : ''}` },
                ...(running > 0 ? [{ label: `${running} running`, variant: 'running' as const }] : []),
                ...(failed > 0 ? [{ label: `${failed} failed`, variant: 'failed' as const }] : []),
              ]
            : undefined
        }
        actions={
          <button className="btn" onClick={() => setShowForm(true)}>
            <Plus size={16} /> New Job
          </button>
        }
      />

      {showForm && (
        <div className="modal-overlay" onClick={() => setShowForm(false)}>
          <div className="modal-wide" onClick={e => e.stopPropagation()}>
            <h2>Create Backup Job</h2>
            <BackupJobForm
              onSubmit={data => {
                createJob.mutate(data, { onSuccess: () => setShowForm(false) });
              }}
              onCancel={() => setShowForm(false)}
              loading={createJob.isPending}
            />
          </div>
        </div>
      )}

      {editingJob && (
        <div className="modal-overlay" onClick={() => setEditingJob(null)}>
          <div className="modal-wide" onClick={e => e.stopPropagation()}>
            <h2>Edit Backup Job</h2>
            <BackupJobForm
              initial={{
                server_id: editingJob.server_id,
                name: editingJob.name,
                remote_paths: JSON.parse(editingJob.remote_paths),
                cron_schedule: editingJob.cron_schedule || '',
                rsync_options: editingJob.rsync_options,
                max_parallel: editingJob.max_parallel,
              }}
              onSubmit={data => {
                updateJob.mutate({ id: editingJob.id, data }, { onSuccess: () => setEditingJob(null) });
              }}
              onCancel={() => setEditingJob(null)}
              loading={updateJob.isPending}
            />
          </div>
        </div>
      )}

      {showBackupTypeModal && pendingJobId && (
        <div className="modal-overlay" onClick={() => setShowBackupTypeModal(false)}>
          <div className="modal" onClick={e => e.stopPropagation()}>
            <h2>Select Backup Type</h2>
            <p style={{ marginBottom: '1.5rem', color: 'var(--text-secondary)' }}>
              Choose how to perform this backup:
            </p>

            <div style={{ display: 'flex', flexDirection: 'column', gap: '0.75rem' }}>
              <button
                className="btn"
                onClick={() => {
                  runJob.mutate({ jobId: pendingJobId, full: false });
                  setShowBackupTypeModal(false);
                }}
                style={{ justifyContent: 'flex-start', textAlign: 'left' }}
              >
                <div>
                  <strong>Incremental Backup</strong>
                  <div style={{ fontSize: '0.85rem', opacity: 0.8, marginTop: '0.25rem' }}>
                    Only transfer files that changed since last backup (faster, recommended)
                  </div>
                </div>
              </button>

              <button
                className="btn btn-secondary"
                onClick={() => {
                  runJob.mutate({ jobId: pendingJobId, full: true });
                  setShowBackupTypeModal(false);
                }}
                style={{ justifyContent: 'flex-start', textAlign: 'left' }}
              >
                <div>
                  <strong>Full Backup</strong>
                  <div style={{ fontSize: '0.85rem', opacity: 0.8, marginTop: '0.25rem' }}>
                    Transfer all files regardless of changes (slower, complete copy)
                  </div>
                </div>
              </button>
            </div>

            <div className="modal-actions">
              <button className="btn btn-secondary" onClick={() => setShowBackupTypeModal(false)}>
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}

      {isLoading ? (
        <div className="empty-state">Loading...</div>
      ) : jobs.length === 0 ? (
        <div className="empty-state-jobs">
          <FolderOpen size={48} strokeWidth={1} />
          <p>No backup jobs configured yet.</p>
          <button className="btn" onClick={() => setShowForm(true)}>
            <Plus size={16} /> Create your first job
          </button>
        </div>
      ) : (
        <div className="jobs-page-split">
          <div className="jobs-list-section">
            <div className="jobs-table">
              <div className="jobs-table-head">
                <span className="col-status">Status</span>
                <span className="col-name">Job</span>
                <span className="col-paths">Paths</span>
                <span className="col-schedule">Schedule</span>
                <span className="col-lastrun">Last Run</span>
                <span className="col-actions" />
              </div>
              {[...grouped.entries()].map(([serverId, serverJobs]) => (
                <div key={serverId} className="server-group">
                  <div className="server-group-header">
                    <Server size={14} />
                    <span className={`ping-dot ${pingStatuses.get(serverId) === undefined ? '' : pingStatuses.get(serverId)?.reachable ? 'ping-ok' : 'ping-fail'}`} />
                    <span>{getServerName(serverId)}</span>
                    <span className="server-group-count">{serverJobs.length}</span>
                  </div>
                  {serverJobs.map(job => (
                    <JobRow
                      key={job.id}
                      job={job}
                      isActive={job.id === activeJob?.id}
                      running={running}
                      onEdit={() => setEditingJob(job)}
                      onRun={(jobId) => {
                        setPendingJobId(jobId);
                        setShowBackupTypeModal(true);
                      }}
                      onCancel={() => cancelJob.mutate(job.id)}
                      onDelete={() => {
                        if (confirm('Delete this job?')) deleteJob.mutate(job.id);
                      }}
                    />
                  ))}
                </div>
              ))}
            </div>
          </div>

          {activeJob && (
            <div className="detail-panel-section">
              <JobDetailPanel
                job={activeJob}
                serverName={getServerName(activeJob.server_id)}
              />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function JobRow({
  job, isActive, running, onEdit, onRun, onCancel, onDelete,
}: {
  job: BackupJob;
  isActive: boolean;
  running: number;
  onEdit: () => void;
  onRun: (jobId: string) => void;
  onCancel: () => void;
  onDelete: () => void;
}) {
  const remotePaths: string[] = JSON.parse(job.remote_paths);

  return (
    <div className={`job-row status-${job.status} ${isActive ? 'active' : ''}`} onClick={onEdit}>
      <span className="col-status">
        <span className={`status-dot status-${job.status}`} />
        <StatusBadge status={job.status} size="sm" />
      </span>
      <span className="col-name">
        <strong>{job.name}</strong>
      </span>
      <span className="col-paths">
        {remotePaths.map(p => <code key={p}>{p}</code>)}
      </span>
      <span className="col-schedule">
        {job.cron_schedule ? (
          <span className="schedule-value"><Clock size={13} /> {job.cron_schedule}</span>
        ) : (
          <span className="no-value">Manual</span>
        )}
      </span>
      <span className="col-lastrun">
        {job.last_run_at ? (
          new Date(job.last_run_at).toLocaleString()
        ) : (
          <span className="no-value">Never</span>
        )}
      </span>
      <span className="col-actions" onClick={e => e.stopPropagation()}>
        {job.status === 'running' ? (
          <button className="btn btn-danger btn-sm" onClick={onCancel}>
            <Square size={14} />
          </button>
        ) : (
          <button
            className="btn btn-success btn-sm"
            onClick={() => onRun(job.id)}
            disabled={running > 0}
            title={running > 0 ? 'Another job is already running' : 'Start backup'}
          >
            <Play size={14} />
          </button>
        )}
        <button className="btn-icon btn-icon-danger" title="Delete" onClick={onDelete}>
          <Trash2 size={15} />
        </button>
      </span>
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
