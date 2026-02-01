import { useState } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { ArrowLeft, Pencil, Server as ServerIcon, RefreshCw } from 'lucide-react';
import { useServer, useUpdateServer, useDeleteServer } from '../hooks/useServers.js';
import { serversApi } from '../api/endpoints.js';
import StatusBadge from '../components/StatusBadge.js';
import PageHeader from '../components/PageHeader.js';
import ServerForm from '../components/ServerForm.js';
import FileExplorer from '../components/FileExplorer.js';
import toast from 'react-hot-toast';
import './ServerDetail.scss';

function AgentStatusBadge({ status }: { status: string }) {
  const map: Record<string, { variant: 'success' | 'info' | 'warning' | 'danger' | 'muted'; label: string }> = {
    connected: { variant: 'success', label: 'Connected' },
    disconnected: { variant: 'muted', label: 'Disconnected' },
    updating: { variant: 'warning', label: 'Updating...' },
    error: { variant: 'danger', label: 'Error' },
  };
  const { variant, label } = map[status] || { variant: 'muted' as const, label: status };
  return <StatusBadge status={label} variant={variant} size="sm" />;
}

export default function ServerDetail() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const { data: server, isLoading } = useServer(id!);
  const updateServer = useUpdateServer();
  const deleteServer = useDeleteServer();
  const [editing, setEditing] = useState(false);
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

  const agentStatus = (server as any).agent_status || 'disconnected';
  const agentVersion = (server as any).agent_version;
  const agentLastSeen = (server as any).agent_last_seen;

  return (
    <>
      <PageHeader
        icon={<ServerIcon size={22} />}
        title={server.name}
        stats={[
          {
            label: agentStatus,
            variant: agentStatus === 'error' ? ('failed' as const) : 'default',
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
            <dt>Last Seen</dt><dd>{server.last_seen_at ? new Date(server.last_seen_at).toLocaleString() : 'Never'}</dd>
          </dl>
        </div>

        <div className="card">
          <h3>Agent</h3>
          <dl className="detail-list">
            <dt>Status</dt>
            <dd><AgentStatusBadge status={agentStatus} /></dd>
            <dt>Version</dt>
            <dd>{agentVersion || 'Unknown'}</dd>
            <dt>Last Seen</dt>
            <dd>{agentLastSeen ? new Date(agentLastSeen).toLocaleString() : 'Never'}</dd>
          </dl>

          <div className="setup-actions" style={{ marginTop: '1rem' }}>
            {agentStatus === 'connected' && (
              <button
                className="btn btn-sm"
                disabled={busy !== null}
                onClick={() => handleAction('update', () => serversApi.updateAgent(server.id))}
              >
                <RefreshCw size={14} />
                {busy === 'update' ? 'Updating...' : 'Update Agent'}
              </button>
            )}
            {agentStatus === 'disconnected' && (
              <div className="error-box" style={{ marginBottom: '0.5rem' }}>
                Agent is not connected. It may be starting up or the service may need to be restarted on the remote server.
              </div>
            )}
            {agentStatus === 'error' && (
              <div className="error-box" style={{ marginBottom: '0.5rem' }}>
                Agent encountered an error. Check the agent logs on the remote server.
              </div>
            )}
            {agentStatus === 'updating' && (
              <div style={{ color: 'var(--color-warning)' }}>
                Agent is updating, please wait...
              </div>
            )}
          </div>
        </div>
      </div>

        {agentStatus === 'connected' && (
          <div className="card" style={{ marginTop: '1rem' }}>
            <h3>File Explorer</h3>
            <FileExplorer serverId={server.id} />
          </div>
        )}
      </div>
    </>
  );
}
