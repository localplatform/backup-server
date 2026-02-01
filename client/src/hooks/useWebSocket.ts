import { useEffect, useRef, useCallback, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';

export interface WsMessage {
  type: string;
  payload: Record<string, unknown>;
}

type WsListener = (payload: Record<string, unknown>) => void;

const listeners = new Map<string, Set<WsListener>>();

let globalWs: WebSocket | null = null;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let reconnectDelay = 1000;
const MAX_DELAY = 30000;

// Track reconnection state for message replay
let lastConnectedTime = Date.now();
let isReconnecting = false;
const activeJobSubscriptions = new Set<string>();

function getWsUrl(): string {
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${window.location.host}/ws`;
}

function notifyListeners(type: string, payload: Record<string, unknown>) {
  const set = listeners.get(type);
  if (set) {
    set.forEach(fn => fn(payload));
  }
}

function connect(queryClient: ReturnType<typeof useQueryClient>, onStatusChange: (connected: boolean) => void) {
  if (globalWs && (globalWs.readyState === WebSocket.OPEN || globalWs.readyState === WebSocket.CONNECTING)) {
    // If already connected, immediately update the status for this new subscriber
    if (globalWs.readyState === WebSocket.OPEN) {
      onStatusChange(true);
    }
    return;
  }

  const ws = new WebSocket(getWsUrl());
  globalWs = ws;

  ws.onopen = () => {
    reconnectDelay = 1000;
    onStatusChange(true);

    // Request message replay if reconnecting
    if (isReconnecting && activeJobSubscriptions.size > 0) {
      const disconnectedAt = lastConnectedTime;
      activeJobSubscriptions.forEach(jobId => {
        ws.send(JSON.stringify({
          type: 'replay:request',
          payload: { jobId, since: disconnectedAt }
        }));
      });
      isReconnecting = false;
    }

    lastConnectedTime = Date.now();

    // Only invalidate on reconnect (not every progress event)
    queryClient.invalidateQueries({ queryKey: ['servers'] });
    queryClient.invalidateQueries({ queryKey: ['jobs'] });
  };

  ws.onmessage = (event) => {
    try {
      const msg: WsMessage = JSON.parse(event.data);
      notifyListeners(msg.type, msg.payload);

      // Auto-invalidate react-query caches
      if (msg.type.startsWith('server:') && msg.type !== 'server:ping') {
        queryClient.invalidateQueries({ queryKey: ['servers'] });
      }

      // FIXED: Only invalidate on data-changing events, NOT progress updates
      // Progress is handled by component state, not React Query cache
      if (msg.type === 'job:updated' ||
          msg.type === 'job:created' ||
          msg.type === 'job:deleted' ||
          msg.type === 'backup:completed' ||
          msg.type === 'backup:failed' ||
          msg.type === 'backup:started') {
        queryClient.invalidateQueries({ queryKey: ['jobs'] });
      }

      // REMOVED: msg.type.startsWith('backup:') - was causing 50+ invalidations per job
      // backup:progress events are now ignored for cache invalidation
    } catch {
      // ignore malformed messages
    }
  };

  ws.onclose = () => {
    onStatusChange(false);
    globalWs = null;
    isReconnecting = true;  // Mark as reconnecting for message replay
    reconnectTimer = setTimeout(() => {
      reconnectDelay = Math.min(reconnectDelay * 2, MAX_DELAY);
      connect(queryClient, onStatusChange);
    }, reconnectDelay);
  };

  ws.onerror = () => {
    ws.close();
  };
}

export function useWebSocket() {
  const queryClient = useQueryClient();
  const [connected, setConnected] = useState(false);

  useEffect(() => {
    connect(queryClient, setConnected);
    return () => {
      if (reconnectTimer) clearTimeout(reconnectTimer);
    };
  }, [queryClient]);

  const subscribe = useCallback((type: string, listener: WsListener) => {
    if (!listeners.has(type)) {
      listeners.set(type, new Set());
    }
    listeners.get(type)!.add(listener);

    return () => {
      listeners.get(type)?.delete(listener);
      // Cleanup empty sets to prevent memory leaks
      if (listeners.get(type)?.size === 0) {
        listeners.delete(type);
      }
    };
  }, []);  // Empty deps = stable reference (won't cause re-renders)

  return { connected, subscribe };
}
