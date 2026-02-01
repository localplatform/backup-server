import { useState } from 'react';
import { Database, Settings } from 'lucide-react';
import { useStorageSettings, useUpdateStorageSettings, useDiskUsage } from '../hooks/useStorage.js';
import { useStorageHierarchy } from '../hooks/useStorageHierarchy.js';
import PageHeader from '../components/PageHeader.js';
import StorageHierarchyTree from '../components/StorageHierarchyTree.js';
import LocalFileExplorer from '../components/LocalFileExplorer.js';
import './Storage.scss';

export default function Storage() {
  const { data: settings, isLoading: loadingSettings } = useStorageSettings();
  const updateSettings = useUpdateStorageSettings();
  const [showForm, setShowForm] = useState(false);
  const [formPath, setFormPath] = useState('');
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);

  const backupRoot = settings?.backup_root;
  const { data: diskUsage } = useDiskUsage();
  const { data: hierarchy } = useStorageHierarchy();

  const handleSaveSettings = () => {
    if (!formPath.trim()) return;
    updateSettings.mutate(
      { backup_root: formPath.trim() },
      {
        onSuccess: () => {
          setShowForm(false);
        },
      }
    );
  };

  // Find the selected version details for the panel header
  const selectedVersionData = hierarchy?.servers
    .flatMap(s => s.jobs)
    .flatMap(j => j.versions)
    .find(v => v.id === selectedVersion);

  if (loadingSettings) {
    return (
      <div className="page">
        <div className="empty-state">Loading...</div>
      </div>
    );
  }

  return (
    <div className="storage-page">
      <PageHeader
        icon={<Database size={22} />}
        title="Storage"
        stats={
          diskUsage
            ? [
                { label: `${formatBytes(diskUsage.used)} / ${formatBytes(diskUsage.total)}` },
                {
                  label: `${diskUsage.usedPercent}% used`,
                  variant: diskUsage.usedPercent > 90 ? ('failed' as const) : 'default',
                },
              ]
            : undefined
        }
        actions={
          <button
            className="btn"
            onClick={() => {
              setFormPath(backupRoot || '');
              setShowForm(true);
            }}
          >
            <Settings size={16} /> {backupRoot ? 'Change Path' : 'Configure'}
          </button>
        }
      />

      {showForm && (
        <div className="modal-overlay" onClick={() => setShowForm(false)}>
          <div className="modal" onClick={e => e.stopPropagation()}>
            <h2>Backup Storage Path</h2>
            <div className="form-group">
              <label>Root directory</label>
              <input
                type="text"
                value={formPath}
                onChange={e => setFormPath(e.target.value)}
                placeholder="/mnt/backups"
                autoFocus
              />
              <span className="form-hint">Absolute path to the directory where backups are stored.</span>
            </div>
            <div className="modal-actions">
              <button className="btn btn-secondary" onClick={() => setShowForm(false)}>
                Cancel
              </button>
              <button className="btn" onClick={handleSaveSettings} disabled={updateSettings.isPending}>
                {updateSettings.isPending ? 'Saving...' : 'Save'}
              </button>
            </div>
          </div>
        </div>
      )}

      {!backupRoot ? (
        <div className="empty-state-storage">
          <Database size={48} strokeWidth={1} />
          <p>No storage location configured yet.</p>
          <p>Set the root directory to browse your backups.</p>
          <button className="btn" onClick={() => setShowForm(true)}>
            <Settings size={16} /> Configure Storage
          </button>
        </div>
      ) : (
        <>
          {diskUsage && (
            <div className="storage-disk-banner">
              <div className="disk-banner-left">
                <code>{backupRoot}</code>
                <div className="disk-bar">
                  <div
                    className={`disk-bar-fill ${diskUsage.usedPercent > 90 ? 'danger' : diskUsage.usedPercent > 70 ? 'warning' : ''}`}
                    style={{ width: `${diskUsage.usedPercent}%` }}
                  />
                </div>
              </div>
              <div className="disk-banner-right">
                <span className="disk-available">Available: {formatBytes(diskUsage.available)}</span>
              </div>
            </div>
          )}

          <div className="storage-split-view">
            <div className="hierarchy-panel">
              <StorageHierarchyTree
                onVersionSelect={setSelectedVersion}
                selectedVersion={selectedVersion}
              />
            </div>

            {selectedVersion && selectedVersionData && (
              <div className="file-explorer-panel">
                <div className="panel-header">
                  <h3>Backup Contents</h3>
                  <div className="version-details">
                    <span className="detail-item">
                      <strong>Version:</strong> {new Date(selectedVersionData.version_timestamp).toLocaleString()}
                    </span>
                    <span className="detail-item">
                      <strong>Status:</strong> <span className={`status-badge status-${selectedVersionData.status}`}>{selectedVersionData.status}</span>
                    </span>
                    <span className="detail-item">
                      <strong>Size:</strong> {formatBytes(selectedVersionData.bytes_transferred)}
                    </span>
                    <span className="detail-item">
                      <strong>Files:</strong> {selectedVersionData.files_transferred.toLocaleString()}
                    </span>
                  </div>
                </div>
                <LocalFileExplorer versionId={selectedVersion} />
              </div>
            )}
          </div>
        </>
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
