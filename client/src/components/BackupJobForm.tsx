import { useState } from 'react';
import { useServers } from '../hooks/useServers.js';
import { X } from 'lucide-react';
import CronInput from './CronInput.js';
import FileExplorer from './FileExplorer.js';

interface Props {
  initial?: {
    server_id: string;
    name: string;
    remote_paths: string[];
    cron_schedule: string;
    rsync_options: string;
    max_parallel: number;
  };
  onSubmit: (data: {
    server_id: string;
    name: string;
    remote_paths: string[];
    cron_schedule: string | null;
    rsync_options: string;
    max_parallel: number;
  }) => void;
  onCancel: () => void;
  loading?: boolean;
}

export default function BackupJobForm({ initial, onSubmit, onCancel, loading }: Props) {
  const { data: servers = [] } = useServers();
  const [serverId, setServerId] = useState(initial?.server_id || '');
  const [name, setName] = useState(initial?.name || '');
  const [remotePaths, setRemotePaths] = useState<string[]>(initial?.remote_paths || []);
  const [cronSchedule, setCronSchedule] = useState(initial?.cron_schedule || '');
  const [maxParallel, setMaxParallel] = useState(initial?.max_parallel || 4);
  const [pathInput, setPathInput] = useState('');

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onSubmit({
      server_id: serverId,
      name,
      remote_paths: remotePaths,
      cron_schedule: cronSchedule || null,
      rsync_options: '',
      max_parallel: maxParallel,
    });
  };

  const addPath = (path: string) => {
    if (path && !remotePaths.includes(path)) {
      setRemotePaths([...remotePaths, path]);
    }
  };

  const selectedServer = servers.find(s => s.id === serverId);

  return (
    <form onSubmit={handleSubmit}>
      <div className="job-form-grid">
        {/* Colonne 1 : Server Selection */}
        <div className="server-selection-column">
          <label>Select Server</label>
          <div className="server-cards">
            {servers.map(server => (
              <div
                key={server.id}
                className={`server-card ${serverId === server.id ? 'selected' : ''}`}
                onClick={() => setServerId(server.id)}
              >
                <div className="server-card-header">
                  <strong>{server.name}</strong>
                  <div className={`status-badge ssh-${server.ssh_status}`}>
                    {server.ssh_status}
                  </div>
                </div>
                <div className="server-card-body">
                  <code>{server.hostname}</code>
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* Colonne 2 : Form Fields */}
        <div className="form-fields-column">
          <div className="form-group">
            <label>Job Name</label>
            <input value={name} onChange={e => setName(e.target.value)} placeholder="Daily backup" required />
          </div>

          <div className="form-group">
            <label>Remote Paths</label>
            <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '0.5rem' }}>
              <input
                value={pathInput}
                onChange={e => setPathInput(e.target.value)}
                placeholder="/var/www"
                style={{ flex: 1 }}
              />
              <button type="button" className="btn btn-sm" onClick={() => { addPath(pathInput); setPathInput(''); }}>
                Add
              </button>
            </div>
            {remotePaths.length > 0 && (
              <div style={{ display: 'flex', flexWrap: 'wrap', gap: '0.25rem' }}>
                {remotePaths.map(p => (
                  <span key={p} className="btn btn-secondary btn-sm" style={{ gap: '0.25rem' }}>
                    {p}
                    <X size={12} style={{ cursor: 'pointer' }} onClick={() => setRemotePaths(remotePaths.filter(x => x !== p))} />
                  </span>
                ))}
              </div>
            )}
          </div>

          <div className="form-group">
            <label>Schedule (optional)</label>
            <CronInput value={cronSchedule} onChange={setCronSchedule} />
          </div>

          <div className="form-group">
            <label>Max Parallel Transfers</label>
            <div style={{ display: 'flex', gap: '0.5rem' }}>
              {[1, 2, 3, 4].map(n => (
                <button
                  key={n}
                  type="button"
                  className={`btn btn-sm ${maxParallel === n ? '' : 'btn-secondary'}`}
                  onClick={() => setMaxParallel(n)}
                >
                  {n}
                </button>
              ))}
            </div>
          </div>
        </div>

        {/* Colonne 3 : File Browser */}
        <div className="file-browser-column">
          <label>Browse Remote Files</label>
          {!serverId ? (
            <div className="browser-placeholder">
              <p>Select a server to browse files</p>
            </div>
          ) : selectedServer?.ssh_status !== 'connected' ? (
            <div className="browser-placeholder warning">
              <p>Server not connected</p>
              <small>SSH status: {selectedServer?.ssh_status}</small>
            </div>
          ) : (
            <FileExplorer
              serverId={serverId}
              selectable
              selectedPaths={remotePaths}
              onSelect={addPath}
            />
          )}
        </div>
      </div>

      <div className="modal-actions">
        <button type="button" className="btn btn-secondary" onClick={onCancel}>Cancel</button>
        <button type="submit" className="btn" disabled={loading || remotePaths.length === 0}>
          {loading ? 'Saving...' : initial ? 'Update' : 'Create Job'}
        </button>
      </div>
    </form>
  );
}
