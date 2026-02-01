import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { versionsApi } from '../api/endpoints.js';

export function useVersions(jobId: string | undefined) {
  return useQuery({
    queryKey: ['versions', jobId],
    queryFn: () => versionsApi.list(jobId!),
    enabled: !!jobId,
  });
}

export function useDeleteVersion() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: versionsApi.delete,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['versions'] });
      queryClient.invalidateQueries({ queryKey: ['storage', 'hierarchy'] });
    },
  });
}

export function useDeleteVersionsByJob() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: versionsApi.deleteByJob,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['versions'] });
      queryClient.invalidateQueries({ queryKey: ['storage', 'hierarchy'] });
    },
  });
}

export function useDeleteVersionsByServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: versionsApi.deleteByServer,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['versions'] });
      queryClient.invalidateQueries({ queryKey: ['storage', 'hierarchy'] });
    },
  });
}
