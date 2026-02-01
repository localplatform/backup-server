import { useState } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { Key, Wifi, WifiOff, Package, ArrowLeft, Pencil, Server as ServerIcon } from 'lucide-react';
import { useServer, useUpdateServer, useDeleteServer } from '../hooks/useServers.js';
import { serversApi } from '../api/endpoints.js';
import StatusBadge from '../components/StatusBadge.js';
import PageHeader from '../components/PageHeader.js';
import ServerForm from '../components/ServerForm.js';
import FileExplorer from '../components/FileExplorer.js';
import toast from 'react-hot-toast';
import './ServerDetail.scss';

export default function ServerDetail() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const { data: server, isLoading } = useServer(id!);
  const updateServer = useUpdateServer();
  const deleteServer = useDeleteServer();
  const [editing, setEditing] = useState(false);
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState<string | null>(null);

  if (isLoading) return <div className="page"><div className="empty-state">Loading...</div></div>;
  if (!server) return <div className="page"><div className="empty-state">Server not found</div></div>;

  const handleAction = async (action: string, fn: () => Promise<unknown>) => {
    setBusy(action);
    try {
      await fn();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Action failed');
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <PageHeader
        icon={<ServerIcon size={22} />}
        title={server.name}
        stats={[
          {
            label: server.ssh_status,
            variant: server.ssh_status === 'error' ? ('failed' as const) : 'default',
          },
        ]}
        leftExtra={
          <button className="btn btn-secondary btn-sm" onClick={() => navigate('/servers')}>
            <ArrowLeft size={14} />
          </button>
        }
        actions={
          <>
            <button className="btn btn-secondary btn-sm" onClick={() => setEditing(true)}>
              <Pencil size={14} /> Edit
            </button>
            <button
              className="btn btn-danger btn-sm"
              onClick={() => {
                if (confirm('Delete this server?')) {
                  deleteServer.mutate(server.id, { onSuccess: () => navigate('/servers') });
                }
              }}
            >
              Delete
            </button>
          </>
        }
      />

      <div className="page">
        {editing && (
        <div className="modal-overlay" onClick={() => setEditing(false)}>
          <div className="modal" onClick={e => e.stopPropagation()}>
            <h2>Edit Server</h2>
            <ServerForm
              initial={server}
              onSubmit={(data) => {
                updateServer.mutate({ id: server.id, data }, { onSuccess: () => setEditing(false) });
              }}
              onCancel={() => setEditing(false)}
              loading={updateServer.isPending}
            />
          </div>
        </div>
      )}

      <div className="grid cols-2">
        <div className="card">
          <h3>Connection Info</h3>
          <dl className="detail-list">
            <dt>Hostname</dt><dd>{server.hostname}</dd>
            <dt>Port</dt><dd>{server.port}</dd>
            <dt>SSH User</dt><dd>{server.ssh_user}</dd>
            <dt>Rsync</dt><dd>{server.rsync_installed ? 'Installed' : 'Not installed'}</dd>
            <dt>Last Seen</dt><dd>{server.last_seen_at ? new Date(server.last_seen_at).toLocaleString() : 'Never'}</dd>
          </dl>
          {server.ssh_error && (
            <div className="error-box">{server.ssh_error}</div>
          )}
        </div>

        <div className="card">
          <h3>SSH Setup</h3>
          <div className="setup-actions">
            <div className="setup-step">
              <span className="step-num">1</span>
              <button
                className="btn btn-sm"
                disabled={busy !== null}
                onClick={() => handleAction('generate', () => serversApi.generateKey(server.id))}
              >
                <Key size={14} />
                {busy === 'generate' ? 'Generating...' : 'Generate SSH Key'}
              </button>
              {(server.ssh_status !== 'pending') && <StatusBadge status="key_generated" size="sm" />}
            </div>

            <div className="setup-step">
              <span className="step-num">2</span>
              <div className="register-key-row">
                <input
                  type="password"
                  placeholder="SSH password (one-time)"
                  value={password}
                  onChange={e => setPassword(e.target.value)}
                  style={{ flex: 1 }}
                />
                <button
                  className="btn btn-sm"
                  disabled={busy !== null || !password || server.ssh_status === 'pending'}
                  onClick={() => handleAction('register', async () => {
                    await serversApi.registerKey(server.id, password);
                    setPassword('');
                  })}
                >
                  {busy === 'register' ? 'Registering...' : 'Register Key'}
                </button>
              </div>
            </div>

            <div className="setup-step">
              <span className="step-num">3</span>
              <button
                className="btn btn-sm btn-success"
                disabled={busy !== null || server.ssh_status === 'pending'}
                onClick={() => handleAction('test', () => serversApi.testConnection(server.id))}
              >
                {busy === 'test' ? <><Wifi size={14} /> Testing...</> : <><Wifi size={14} /> Test Connection</>}
              </button>
            </div>

            <div className="setup-step">
              <span className="step-num">4</span>
              <button
                className="btn btn-sm"
                disabled={busy !== null || server.ssh_status !== 'connected'}
                onClick={() => handleAction('provision', () => serversApi.provision(server.id))}
              >
                <Package size={14} />
                {busy === 'provision' ? 'Provisioning...' : 'Install Rsync'}
              </button>
            </div>
          </div>
        </div>
      </div>

        {server.ssh_status === 'connected' && (
          <div className="card" style={{ marginTop: '1rem' }}>
            <h3>File Explorer</h3>
            <FileExplorer serverId={server.id} />
          </div>
        )}
      </div>
    </>
  );
}
