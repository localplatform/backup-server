export type WsEventType =
  | 'backup:started'
  | 'backup:progress'
  | 'backup:task-completed'
  | 'backup:completed'
  | 'backup:failed'
  | 'server:status'
  | 'server:ping'
  | 'server:updated'
  | 'job:updated'
  | 'job:created'
  | 'job:deleted'
  | 'version:created'
  | 'version:completed'
  | 'version:deleted';

export interface WsMessage {
  type: WsEventType;
  payload: Record<string, unknown>;
}

export interface BackupStartedPayload {
  jobId: string;
  serverId: string;
  remotePaths: string[];
}

export interface BackupProgressPayload {
  jobId: string;
  percent: number;
  checkedFiles: number;
  totalFiles: number;
  transferredBytes: number;  // NEW: Bytes transferred so far
  totalBytes: number;        // NEW: Total bytes in transfer
  speed: string;
  currentFile: string;
}

export interface BackupTaskCompletedPayload {
  jobId: string;
  taskIndex: number;
  remotePath: string;
  bytes: number;
  files: number;
}

export interface BackupCompletedPayload {
  jobId: string;
  duration: number;
  totalBytes: number;
  totalFiles: number;
}

export interface BackupFailedPayload {
  jobId: string;
  taskIndex?: number;
  error: string;
  remotePath?: string;
}

export interface ServerStatusPayload {
  serverId: string;
  status: string;
  error?: string;
  lastSeenAt?: string;
}

export interface VersionEventPayload {
  versionId: string;
  jobId: string;
}
