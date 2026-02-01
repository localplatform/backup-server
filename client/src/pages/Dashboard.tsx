import { Server, FolderSync, Activity, AlertTriangle, LayoutDashboard } from 'lucide-react';
import { Link } from 'react-router-dom';
import { useServers } from '../hooks/useServers.js';
import { useBackupJobs } from '../hooks/useBackupJobs.js';
import StatusBadge from '../components/StatusBadge.js';
import PageHeader from '../components/PageHeader.js';
import './Dashboard.scss';

export default function Dashboard() {
  const { data: servers = [] } = useServers();
  const { data: jobs = [] } = useBackupJobs();

  const connectedServers = servers.filter(s => s.ssh_status === 'connected').length;
  const errorServers = servers.filter(s => s.ssh_status === 'error').length;
  const runningJobs = jobs.filter(j => j.status === 'running').length;
  const failedJobs = jobs.filter(j => j.status === 'failed').length;

  return (
    <>
      <PageHeader icon={<LayoutDashboard size={22} />} title="Dashboard" />

      <div className="page">
        <div className="grid cols-4">
        <div className="card stat-card">
          <div className="stat-icon blue">
            <Server size={20} />
          </div>
          <div className="stat-info">
            <span className="stat-value">{servers.length}</span>
            <span className="stat-label">Servers</span>
          </div>
          <span className="stat-detail">{connectedServers} connected</span>
        </div>
        <div className="card stat-card">
          <div className="stat-icon green">
            <FolderSync size={20} />
          </div>
          <div className="stat-info">
            <span className="stat-value">{jobs.length}</span>
            <span className="stat-label">Backup Jobs</span>
          </div>
          <span className="stat-detail">{jobs.filter(j => j.enabled).length} enabled</span>
        </div>
        <div className="card stat-card">
          <div className="stat-icon cyan">
            <Activity size={20} />
          </div>
          <div className="stat-info">
            <span className="stat-value">{runningJobs}</span>
            <span className="stat-label">Running</span>
          </div>
          <span className="stat-detail">active now</span>
        </div>
        <div className="card stat-card">
          <div className="stat-icon red">
            <AlertTriangle size={20} />
          </div>
          <div className="stat-info">
            <span className="stat-value">{errorServers + failedJobs}</span>
            <span className="stat-label">Issues</span>
          </div>
          <span className="stat-detail">
            {errorServers} servers, {failedJobs} jobs
          </span>
        </div>
      </div>

      <div className="grid cols-2" style={{ marginTop: '1.5rem' }}>
        <div className="card dashboard-table dashboard-servers-table">
          <h3>Servers</h3>
          {servers.length === 0 ? (
            <p className="empty-state">
              No servers configured. <Link to="/servers">Add one</Link>
            </p>
          ) : (
            <div className="table-grid">
              <div className="table-grid-head">
                <span>Name</span>
                <span>Host</span>
                <span>Status</span>
              </div>
              {servers.slice(0, 10).map(s => (
                <div key={s.id} className="table-grid-row">
                  <span>
                    <Link to={`/servers/${s.id}`}>{s.name}</Link>
                  </span>
                  <span className="col-secondary">{s.hostname}</span>
                  <span>
                    <StatusBadge status={s.ssh_status} size="sm" />
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="card dashboard-table dashboard-jobs-table">
          <h3>Recent Jobs</h3>
          {jobs.length === 0 ? (
            <p className="empty-state">
              No backup jobs configured. <Link to="/jobs">Create one</Link>
            </p>
          ) : (
            <div className="table-grid">
              <div className="table-grid-head">
                <span>Name</span>
                <span>Status</span>
                <span>Last Run</span>
              </div>
              {jobs.slice(0, 10).map(j => (
                <div key={j.id} className="table-grid-row">
                  <span>{j.name}</span>
                  <span>
                    <StatusBadge status={j.status} size="sm" />
                  </span>
                  <span className="col-secondary">
                    {j.last_run_at ? new Date(j.last_run_at).toLocaleString() : '-'}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
      </div>
    </>
  );
}
