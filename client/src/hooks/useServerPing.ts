import { useState, useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useWebSocket } from './useWebSocket.js';
import { serversApi } from '../api/endpoints.js';

export interface PingStatus {
  serverId: string;
  reachable: boolean;
  latencyMs: number | null;
  lastCheckedAt: string;
}

export function useServerPingStatus() {
  const { subscribe } = useWebSocket();
  const { data: initialStatuses = [] } = useQuery({
    queryKey: ['ping-status'],
    queryFn: serversApi.pingStatus,
  });

  const [statuses, setStatuses] = useState<Map<string, PingStatus>>(new Map());

  // Seed from REST on initial load
  useEffect(() => {
    if (initialStatuses.length === 0) return;
    const map = new Map<string, PingStatus>();
    for (const s of initialStatuses) map.set(s.serverId, s);
    setStatuses(map);
  }, [initialStatuses]);

  // Subscribe to live updates
  useEffect(() => {
    return subscribe('server:ping', (payload) => {
      const p = payload as unknown as PingStatus;
      setStatuses(prev => {
        const next = new Map(prev);
        next.set(p.serverId, p);
        return next;
      });
    });
  }, [subscribe]);

  return statuses;
}
