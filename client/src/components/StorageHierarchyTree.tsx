import { useState } from 'react';
import { ChevronRight, Server, HardDrive, Database, Trash2, AlertTriangle } from 'lucide-react';
import { useStorageHierarchy } from '../hooks/useStorageHierarchy.js';
import { useDeleteVersion, useDeleteVersionsByJob, useDeleteVersionsByServer } from '../hooks/useVersions.js';
import { useWebSocket } from '../hooks/useWebSocket.js';
import { useQueryClient } from '@tanstack/react-query';
import { useEffect } from 'react';
import toast from 'react-hot-toast';
import './StorageHierarchyTree.scss';

interface Props {
  onVersionSelect: (versionId: string | null) => void;
  selectedVersion: string | null;
}

export default function StorageHierarchyTree({ onVersionSelect, selectedVersion }: Props) {
  const { data: hierarchy, isLoading, error } = useStorageHierarchy();
  const [expandedServers, setExpandedServers] = useState<Set<string>>(new Set());
  const [expandedJobs, setExpandedJobs] = useState<Set<string>>(new Set());
  const [deleteConfirm, setDeleteConfirm] = useState<{
    type: 'version' | 'job' | 'server';
    id: string;
    name: string;
  } | null>(null);

  const deleteVersion = useDeleteVersion();
  const deleteVersionsByJob = useDeleteVersionsByJob();
  const deleteVersionsByServer = useDeleteVersionsByServer();

  const { subscribe } = useWebSocket();
  const queryClient = useQueryClient();

  // Subscribe to WebSocket events
  useEffect(() => {
    const unsubDeleted = subscribe('version:deleted', () => {
      queryClient.invalidateQueries({ queryKey: ['storage', 'hierarchy'] });
    });
    const unsubBulkDeleted = subscribe('version:bulk-deleted', () => {
      queryClient.invalidateQueries({ queryKey: ['storage', 'hierarchy'] });
    });
    const unsubCompleted = subscribe('backup:completed', () => {
      queryClient.invalidateQueries({ queryKey: ['storage', 'hierarchy'] });
    });
    const unsubFailed = subscribe('backup:failed', () => {
      queryClient.invalidateQueries({ queryKey: ['storage', 'hierarchy'] });
    });
    return () => {
      unsubDeleted();
      unsubBulkDeleted();
      unsubCompleted();
      unsubFailed();
    };
  }, [subscribe, queryClient]);

  const toggleServer = (serverId: string) => {
    setExpandedServers(prev => {
      const next = new Set(prev);
      if (next.has(serverId)) {
        next.delete(serverId);
      } else {
        next.add(serverId);
      }
      return next;
    });
  };

  const toggleJob = (jobId: string) => {
    setExpandedJobs(prev => {
      const next = new Set(prev);
      if (next.has(jobId)) {
        next.delete(jobId);
      } else {
        next.add(jobId);
      }
      return next;
    });
  };

  const handleDelete = () => {
    if (!deleteConfirm) return;

    switch (deleteConfirm.type) {
      case 'version':
        deleteVersion.mutate(deleteConfirm.id, {
          onSuccess: () => {
            toast.success('Version deleted');
            if (selectedVersion === deleteConfirm.id) {
              onVersionSelect(null);
            }
          },
          onError: (err: Error) => toast.error(err.message || 'Failed to delete version'),
        });
        break;
      case 'job':
        deleteVersionsByJob.mutate(deleteConfirm.id, {
          onSuccess: () => toast.success('Job backups deleted'),
          onError: (err: Error) => toast.error(err.message || 'Failed to delete job backups'),
        });
        break;
      case 'server':
        deleteVersionsByServer.mutate(deleteConfirm.id, {
          onSuccess: () => toast.success('Server backups deleted'),
          onError: (err: Error) => toast.error(err.message || 'Failed to delete server backups'),
        });
        break;
    }

    setDeleteConfirm(null);
  };

  if (isLoading) {
    return <div className="hierarchy-loading">Loading storage hierarchy...</div>;
  }

  if (error) {
    return <div className="hierarchy-error">Failed to load storage hierarchy</div>;
  }

  if (!hierarchy || hierarchy.servers.length === 0) {
    return (
      <div className="hierarchy-empty">
        <Database size={48} strokeWidth={1} />
        <p>No backup data yet</p>
        <p>Run a backup job to see data here</p>
      </div>
    );
  }

  return (
    <>
      <div className="storage-hierarchy-tree">
        {hierarchy.servers.map(server => (
          <div key={server.id} className="hierarchy-server-group">
            <div className="hierarchy-server-row" onClick={() => toggleServer(server.id)}>
              <ChevronRight size={14} className={`chevron ${expandedServers.has(server.id) ? 'open' : ''}`} />
              <Server size={16} className="icon server-icon" />
              <span className="server-name">{server.name}</span>
              <span className="server-info">
                {server.hostname} · {server.jobs.length} job{server.jobs.length !== 1 ? 's' : ''} · {server.totalVersions} version{server.totalVersions !== 1 ? 's' : ''}
              </span>
              {server.totalVersions > 0 && (
                <button
                  className="btn-icon btn-danger-ghost"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDeleteConfirm({ type: 'server', id: server.id, name: server.name });
                  }}
                  title="Delete all backups for this server"
                >
                  <Trash2 size={14} />
                </button>
              )}
            </div>

            {expandedServers.has(server.id) && (
              <div className="jobs-container">
                {server.jobs.map(job => (
                  <div key={job.id} className="hierarchy-job-group">
                    <div className="hierarchy-job-row" onClick={() => toggleJob(job.id)}>
                      <ChevronRight size={14} className={`chevron ${expandedJobs.has(job.id) ? 'open' : ''}`} />
                      <HardDrive size={16} className="icon job-icon" />
                      <span className="job-name">{job.name}</span>
                      <span className="job-info">
                        {job.versions.length} version{job.versions.length !== 1 ? 's' : ''} · {formatBytes(job.totalSize)}
                      </span>
                      {job.versions.length > 0 && (
                        <button
                          className="btn-icon btn-danger-ghost"
                          onClick={(e) => {
                            e.stopPropagation();
                            setDeleteConfirm({ type: 'job', id: job.id, name: job.name });
                          }}
                          title="Delete all backups for this job"
                        >
                          <Trash2 size={14} />
                        </button>
                      )}
                    </div>

                    {expandedJobs.has(job.id) && (
                      <div className="versions-container">
                        {job.versions.length === 0 ? (
                          <div className="no-versions">No backups yet</div>
                        ) : (
                          job.versions.map(version => {
                            const isSelected = selectedVersion === version.id;

                            return (
                              <div
                                key={version.id}
                                className={`hierarchy-version-row ${isSelected ? 'selected' : ''}`}
                                onClick={() => onVersionSelect(version.id)}
                              >
                                <div className="version-info">
                                  <span className="version-timestamp">
                                    {parseVersionTimestamp(version.version_timestamp)}
                                  </span>
                                  <span className={`version-status status-${version.status}`}>
                                    {version.status}
                                  </span>
                                  <span className="version-size">
                                    {formatBytes(version.bytes_transferred)} · {version.files_transferred.toLocaleString()} files
                                  </span>
                                </div>
                                {version.status === 'completed' && (
                                  <button
                                    className="btn-icon btn-danger-ghost"
                                    onClick={(e) => {
                                      e.stopPropagation();
                                      setDeleteConfirm({ type: 'version', id: version.id, name: parseVersionTimestamp(version.version_timestamp) });
                                    }}
                                    title="Delete this version"
                                  >
                                    <Trash2 size={14} />
                                  </button>
                                )}
                              </div>
                            );
                          })
                        )}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>

      {deleteConfirm && (
        <div className="modal-overlay" onClick={() => setDeleteConfirm(null)}>
          <div className="modal" onClick={e => e.stopPropagation()}>
            <h2>
              <AlertTriangle size={20} style={{ verticalAlign: 'text-bottom', marginRight: '0.5rem', color: 'var(--warning)' }} />
              Confirm Deletion
            </h2>
            <p>
              Are you sure you want to delete {deleteConfirm.type === 'version' ? 'this backup version' : `all backups for ${deleteConfirm.type}`} <strong>{deleteConfirm.name}</strong>?
            </p>
            {deleteConfirm.type !== 'version' && (
              <p style={{ color: 'var(--warning)', fontSize: '0.85rem', fontWeight: 500 }}>
                This will delete ALL backups for this {deleteConfirm.type}. This action cannot be undone.
              </p>
            )}
            <div className="modal-actions">
              <button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)}>
                Cancel
              </button>
              <button className="btn btn-danger" onClick={handleDelete}>
                Delete
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

/** Parse version timestamp format "YYYY-MM-DD_HH-MM-SS" into a readable date string */
function parseVersionTimestamp(ts: string): string {
  // Convert "2025-01-15_14-30-22" → "2025-01-15T14:30:22"
  const iso = ts.replace('_', 'T').replace(/-(\d{2})-(\d{2})$/, ':$1:$2');
  const date = new Date(iso);
  if (isNaN(date.getTime())) return ts; // fallback to raw string
  return date.toLocaleString();
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}
