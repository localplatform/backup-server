import api from './client.js';

// Types
export interface Server {
  id: string;
  name: string;
  hostname: string;
  port: number;
  ssh_user: string;
  ssh_key_path: string | null;
  ssh_status: 'pending' | 'key_generated' | 'key_registered' | 'connected' | 'error';
  ssh_error: string | null;
  rsync_installed: number;
  agent_status: 'disconnected' | 'connected' | 'updating' | 'error';
  agent_version: string | null;
  agent_last_seen: string | null;
  last_seen_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface BackupJob {
  id: string;
  server_id: string;
  name: string;
  remote_paths: string; // JSON
  local_path: string;
  cron_schedule: string | null;
  status: 'idle' | 'running' | 'completed' | 'failed' | 'cancelled';
  rsync_options: string;
  max_parallel: number;
  enabled: number;
  last_run_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface BackupLog {
  id: string;
  job_id: string;
  started_at: string;
  finished_at: string | null;
  status: 'running' | 'completed' | 'failed' | 'cancelled';
  bytes_transferred: number;
  files_transferred: number;
  output: string;
  error: string | null;
}

export interface BackupVersion {
  id: string;
  job_id: string;
  log_id: string | null;
  version_timestamp: string;
  local_path: string;
  status: 'running' | 'completed' | 'failed';
  bytes_total: number;
  files_total: number;
  bytes_transferred: number;
  files_transferred: number;
  created_at: string;
  completed_at: string | null;
  backup_type: 'full' | 'incremental';
  files_unchanged: number;
  bytes_unchanged: number;
  files_deleted: number;
}

export interface RemoteEntry {
  name: string;
  path: string;
  type: 'file' | 'directory' | 'symlink' | 'other';
  size: number;
  modifiedAt: string;
  permissions: number;
}

// Server endpoints
export const serversApi = {
  list: () => api.get<Server[]>('/servers').then(r => r.data),
  get: (id: string) => api.get<Server>(`/servers/${id}`).then(r => r.data),
  create: (data: { name: string; hostname: string; port?: number; ssh_user?: string; password?: string }) =>
    api.post<Server>('/servers', data).then(r => r.data),
  update: (id: string, data: Partial<Server>) =>
    api.put<Server>(`/servers/${id}`, data).then(r => r.data),
  delete: (id: string) => api.delete(`/servers/${id}`),
  pingStatus: () =>
    api.get<Array<{ serverId: string; reachable: boolean; latencyMs: number | null; lastCheckedAt: string }>>('/servers/ping-status').then(r => r.data),
  explore: (id: string, path: string) =>
    api.get<RemoteEntry[]>(`/servers/${id}/explore`, { params: { path } }).then(r => r.data),
  // Agent management
  updateAgent: (id: string) =>
    api.post<{ status: string }>(`/agent/update/${id}`).then(r => r.data),
};

// Storage types
export interface BackupMeta {
  server: { name: string; hostname: string; port: number };
  job: { id: string; name: string; remotePaths: string[] };
  createdAt: string;
  lastRunAt: string;
}

export interface LocalEntry {
  name: string;
  path: string;
  type: 'file' | 'directory' | 'symlink' | 'other';
  size: number;
  modifiedAt: string;
  backupMeta?: BackupMeta;
}

export interface DiskUsage {
  total: number;
  used: number;
  available: number;
  usedPercent: number;
}

export interface StorageSettings {
  backup_root: string | null;
}

export interface JobWithVersions {
  id: string;
  name: string;
  remote_paths: string[];
  local_path: string;
  versions: BackupVersion[];
  totalSize: number;
}

export interface ServerWithJobs {
  id: string;
  name: string;
  hostname: string;
  port: number;
  jobs: JobWithVersions[];
  totalVersions: number;
}

export interface StorageHierarchy {
  servers: ServerWithJobs[];
}

// Storage endpoints
export const storageApi = {
  getSettings: () => api.get<StorageSettings>('/storage/settings').then(r => r.data),
  updateSettings: (data: { backup_root: string }) =>
    api.put<StorageSettings>('/storage/settings', data).then(r => r.data),
  browse: (path?: string) =>
    api.get<LocalEntry[]>('/storage/browse', { params: { path } }).then(r => r.data),
  diskUsage: () => api.get<DiskUsage>('/storage/disk-usage').then(r => r.data),
  getHierarchy: () => api.get<StorageHierarchy>('/storage/hierarchy').then(r => r.data),
  browseVersion: (versionId: string, path?: string) =>
    api.get<LocalEntry[]>('/storage/browse-version', { params: { version_id: versionId, path } }).then(r => r.data),
};

// Versions endpoints
export const versionsApi = {
  list: (jobId: string) => api.get<BackupVersion[]>('/versions', { params: { job_id: jobId } }).then(r => r.data),
  get: (id: string) => api.get<BackupVersion>(`/versions/${id}`).then(r => r.data),
  delete: (id: string) => api.delete(`/versions/${id}`),
  deleteByJob: (jobId: string) => api.delete(`/versions/by-job/${jobId}`),
  deleteByServer: (serverId: string) => api.delete(`/versions/by-server/${serverId}`),
};

// Job endpoints
export const jobsApi = {
  list: () => api.get<BackupJob[]>('/jobs').then(r => r.data),
  get: (id: string) => api.get<BackupJob>(`/jobs/${id}`).then(r => r.data),
  create: (data: {
    server_id: string;
    name: string;
    remote_paths: string[];
    cron_schedule?: string | null;
    rsync_options?: string;
    max_parallel?: number;
  }) => api.post<BackupJob>('/jobs', data).then(r => r.data),
  update: (id: string, data: Partial<BackupJob>) =>
    api.put<BackupJob>(`/jobs/${id}`, data).then(r => r.data),
  delete: (id: string) => api.delete(`/jobs/${id}`),
  run: (id: string) => api.post<{ started: boolean }>(`/jobs/${id}/run`).then(r => r.data),
  cancel: (id: string) => api.post<{ cancelled: boolean }>(`/jobs/${id}/cancel`).then(r => r.data),
  logs: (id: string) => api.get<BackupLog[]>(`/jobs/${id}/logs`).then(r => r.data),
};
