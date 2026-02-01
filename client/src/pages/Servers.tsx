import { useState } from 'react';
import { Link } from 'react-router-dom';
import { Plus, Trash2, Server } from 'lucide-react';
import { useServers, useCreateServer, useDeleteServer } from '../hooks/useServers.js';
import StatusBadge from '../components/StatusBadge.js';
import ServerForm from '../components/ServerForm.js';
import PageHeader from '../components/PageHeader.js';
import './Servers.scss';

export default function Servers() {
  const { data: servers = [], isLoading } = useServers();
  const createServer = useCreateServer();
  const deleteServer = useDeleteServer();
  const [showForm, setShowForm] = useState(false);

  const connected = servers.filter(s => s.ssh_status === 'connected').length;
  const errors = servers.filter(s => s.ssh_status === 'error').length;

  return (
    <div className="servers-page">
      <PageHeader
        icon={<Server size={22} />}
        title="Servers"
        stats={
          servers.length > 0
            ? [
                { label: `${servers.length} total` },
                { label: `${connected} connected` },
                ...(errors > 0 ? [{ label: `${errors} errors`, variant: 'failed' as const }] : []),
              ]
            : undefined
        }
        actions={
          <button className="btn" onClick={() => setShowForm(true)}>
            <Plus size={16} /> Add Server
          </button>
        }
      />

      {showForm && (
        <div className="modal-overlay" onClick={() => setShowForm(false)}>
          <div className="modal" onClick={e => e.stopPropagation()}>
            <h2>Add Server</h2>
            <ServerForm
              onSubmit={data => {
                createServer.mutate(data, { onSuccess: () => setShowForm(false) });
              }}
              onCancel={() => setShowForm(false)}
              loading={createServer.isPending}
            />
          </div>
        </div>
      )}

      {isLoading ? (
        <div className="empty-state">Loading...</div>
      ) : servers.length === 0 ? (
        <div className="empty-state-servers">
          <Server size={48} strokeWidth={1} />
          <p>No servers configured yet.</p>
          <button className="btn" onClick={() => setShowForm(true)}>
            <Plus size={16} /> Add your first server
          </button>
        </div>
      ) : (
        <div className="servers-table">
          <div className="servers-table-head">
            <span className="col-name">Name</span>
            <span className="col-hostname">Hostname</span>
            <span className="col-port">Port</span>
            <span className="col-user">User</span>
            <span className="col-ssh-status">SSH Status</span>
            <span className="col-rsync">Rsync</span>
            <span className="col-actions" />
          </div>
          {servers.map(s => (
            <div key={s.id} className={`server-row ${s.ssh_status === 'error' ? 'status-error' : ''}`}>
              <span className="col-name">
                <Link to={`/servers/${s.id}`}>{s.name}</Link>
              </span>
              <span className="col-hostname">{s.hostname}</span>
              <span className="col-port">{s.port}</span>
              <span className="col-user">{s.ssh_user}</span>
              <span className="col-ssh-status">
                <span className={`status-dot status-${s.ssh_status}`} />
                <StatusBadge status={s.ssh_status} size="sm" />
              </span>
              <span className="col-rsync">{s.rsync_installed ? 'Yes' : 'No'}</span>
              <span className="col-actions" onClick={e => e.stopPropagation()}>
                <button
                  className="btn-icon btn-icon-danger"
                  title="Delete"
                  onClick={() => {
                    if (confirm('Delete this server?')) deleteServer.mutate(s.id);
                  }}
                >
                  <Trash2 size={15} />
                </button>
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
