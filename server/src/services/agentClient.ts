/**
 * HTTP and WebSocket client for communicating with the Rust backup agent.
 *
 * This client provides:
 * - HTTP API calls (health, version, backup control)
 * - WebSocket connection for real-time progress updates
 * - TypeScript interfaces matching the Rust agent's protocol
 */

import axios, { AxiosInstance } from 'axios';
import WebSocket from 'ws';
import { EventEmitter } from 'events';
import { logger } from '../utils/logger.js';

// ============================================================================
// Type Definitions (matching Rust agent protocol)
// ============================================================================

export interface AgentConfig {
  host: string;
  port: number;
  protocol?: 'http' | 'https';
  timeout?: number;
}

export interface AgentHealth {
  status: string;
  version: string;
  uptime_secs: number;
}

export interface AgentVersion {
  version: string;
  build?: string;
  features?: string[];
}

export interface StartBackupRequest {
  job_id: string;
  paths: string[];
  server_url: string;
  token?: string;
}

export interface StartBackupResponse {
  status: string;
  pid?: number;
}

export interface CancelBackupRequest {
  job_id: string;
}

export interface CancelBackupResponse {
  status: string;
}

// WebSocket Event Types (from Rust agent)

export interface BackupProgressPayload {
  job_id: string;
  percent: number;
  transferred_bytes: number;
  total_bytes: number;
  bytes_per_second: number;
  eta_seconds: number;
  current_file: string | null;
  files_processed: number;
  total_files: number;
}

export interface AgentStatusPayload {
  status: string;
  active_jobs: number;
  uptime_secs: number;
}

export type WsEvent =
  | { type: 'backup:progress'; payload: BackupProgressPayload }
  | { type: 'backup:started'; payload: { job_id: string } }
  | { type: 'backup:completed'; payload: { job_id: string; total_bytes: number } }
  | { type: 'backup:failed'; payload: { job_id: string; error: string } }
  | { type: 'agent:status'; payload: AgentStatusPayload }
  | { type: 'agent:log'; payload: { level: string; message: string } };

export type WsCommand =
  | { type: 'backup:pause'; payload: { job_id: string } }
  | { type: 'backup:resume'; payload: { job_id: string } }
  | { type: 'backup:cancel'; payload: { job_id: string } }
  | { type: 'agent:status'; payload: null };

// ============================================================================
// Agent Client Class
// ============================================================================

export class AgentClient extends EventEmitter {
  private readonly baseURL: string;
  private readonly wsURL: string;
  private readonly http: AxiosInstance;
  private ws: WebSocket | null = null;
  private reconnectTimer: NodeJS.Timeout | null = null;
  private reconnectDelay = 1000;
  private readonly maxReconnectDelay = 30000;

  constructor(config: AgentConfig) {
    super();

    const protocol = config.protocol || 'http';
    const wsProtocol = protocol === 'https' ? 'wss' : 'ws';

    this.baseURL = `${protocol}://${config.host}:${config.port}`;
    this.wsURL = `${wsProtocol}://${config.host}:${config.port}/ws`;

    this.http = axios.create({
      baseURL: this.baseURL,
      timeout: config.timeout || 30000,
      headers: {
        'Content-Type': 'application/json',
      },
    });

    // Setup axios response interceptor for error logging
    this.http.interceptors.response.use(
      (response) => response,
      (error) => {
        logger.error('Agent HTTP error', {
          url: error.config?.url,
          method: error.config?.method,
          status: error.response?.status,
          message: error.message,
        });
        return Promise.reject(error);
      }
    );
  }

  // ==========================================================================
  // HTTP API Methods
  // ==========================================================================

  /**
   * Check agent health status
   */
  async getHealth(): Promise<AgentHealth> {
    const response = await this.http.get<AgentHealth>('/health');
    return response.data;
  }

  /**
   * Get agent version information
   */
  async getVersion(): Promise<AgentVersion> {
    const response = await this.http.get<AgentVersion>('/version');
    return response.data;
  }

  /**
   * Start a backup job on the agent
   */
  async startBackup(request: StartBackupRequest): Promise<StartBackupResponse> {
    const response = await this.http.post<StartBackupResponse>('/backup/start', request);
    return response.data;
  }

  /**
   * Cancel a running backup job
   */
  async cancelBackup(request: CancelBackupRequest): Promise<CancelBackupResponse> {
    const response = await this.http.post<CancelBackupResponse>('/backup/cancel', request);
    return response.data;
  }

  // ==========================================================================
  // WebSocket Methods
  // ==========================================================================

  /**
   * Connect to the agent's WebSocket endpoint
   */
  connectWebSocket(): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      logger.warn('WebSocket already connected');
      return;
    }

    logger.info(`Connecting to agent WebSocket: ${this.wsURL}`);

    this.ws = new WebSocket(this.wsURL);

    this.ws.on('open', () => {
      logger.info('Agent WebSocket connected');
      this.reconnectDelay = 1000; // Reset reconnect delay
      this.emit('ws:connected');
    });

    this.ws.on('message', (data: WebSocket.Data) => {
      try {
        const message: WsEvent = JSON.parse(data.toString());
        this.handleWsEvent(message);
      } catch (error) {
        logger.error('Failed to parse WebSocket message', { error, data: data.toString() });
      }
    });

    this.ws.on('close', (code, reason) => {
      logger.warn('Agent WebSocket closed', { code, reason: reason.toString() });
      this.ws = null;
      this.emit('ws:disconnected', { code, reason });
      this.scheduleReconnect();
    });

    this.ws.on('error', (error) => {
      logger.error('Agent WebSocket error', { error: error.message });
      this.emit('ws:error', error);
    });
  }

  /**
   * Disconnect from the agent's WebSocket
   */
  disconnectWebSocket(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
  }

  /**
   * Send a command to the agent via WebSocket
   */
  sendCommand(command: WsCommand): void {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
      logger.error('Cannot send command: WebSocket not connected');
      throw new Error('WebSocket not connected');
    }

    this.ws.send(JSON.stringify(command));
    logger.debug('Sent WebSocket command', { type: command.type });
  }

  /**
   * Pause a backup job
   */
  pauseBackup(jobId: string): void {
    this.sendCommand({
      type: 'backup:pause',
      payload: { job_id: jobId },
    });
  }

  /**
   * Resume a paused backup job
   */
  resumeBackup(jobId: string): void {
    this.sendCommand({
      type: 'backup:resume',
      payload: { job_id: jobId },
    });
  }

  /**
   * Request agent status
   */
  requestStatus(): void {
    this.sendCommand({
      type: 'agent:status',
      payload: null,
    });
  }

  // ==========================================================================
  // Private Methods
  // ==========================================================================

  /**
   * Handle incoming WebSocket events
   */
  private handleWsEvent(event: WsEvent): void {
    logger.debug('Received WebSocket event', { type: event.type });

    switch (event.type) {
      case 'backup:progress':
        this.emit('backup:progress', event.payload);
        break;

      case 'backup:started':
        this.emit('backup:started', event.payload);
        break;

      case 'backup:completed':
        this.emit('backup:completed', event.payload);
        break;

      case 'backup:failed':
        this.emit('backup:failed', event.payload);
        break;

      case 'agent:status':
        this.emit('agent:status', event.payload);
        break;

      case 'agent:log':
        this.emit('agent:log', event.payload);
        break;

      default:
        logger.warn('Unknown WebSocket event type', { event });
    }
  }

  /**
   * Schedule WebSocket reconnection with exponential backoff
   */
  private scheduleReconnect(): void {
    if (this.reconnectTimer) {
      return; // Already scheduled
    }

    logger.info(`Scheduling WebSocket reconnect in ${this.reconnectDelay}ms`);

    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.reconnectDelay = Math.min(this.reconnectDelay * 2, this.maxReconnectDelay);
      this.connectWebSocket();
    }, this.reconnectDelay);
  }

  /**
   * Check if WebSocket is connected
   */
  isWebSocketConnected(): boolean {
    return this.ws !== null && this.ws.readyState === WebSocket.OPEN;
  }

  /**
   * Cleanup resources
   */
  destroy(): void {
    this.disconnectWebSocket();
    this.removeAllListeners();
  }
}

// ============================================================================
// Helper Functions
// ============================================================================

/**
 * Create an agent client instance
 */
export function createAgentClient(config: AgentConfig): AgentClient {
  return new AgentClient(config);
}

/**
 * Format bytes for human-readable display (matching Rust agent format)
 */
export function formatBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let size = bytes;
  let unitIndex = 0;

  while (size >= 1024 && unitIndex < units.length - 1) {
    size /= 1024;
    unitIndex++;
  }

  return `${size.toFixed(2)} ${units[unitIndex]}`;
}

/**
 * Format speed for human-readable display
 */
export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

/**
 * Format duration for human-readable display (matching Rust agent format)
 */
export function formatDuration(seconds: number): string {
  if (seconds < 60) {
    return `${seconds}s`;
  } else if (seconds < 3600) {
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins}m ${secs}s`;
  } else {
    const hours = Math.floor(seconds / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    return `${hours}h ${mins}m`;
  }
}
