import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { jobsApi } from '../api/endpoints.js';
import toast from 'react-hot-toast';

export function useBackupJobs() {
  return useQuery({
    queryKey: ['jobs'],
    queryFn: jobsApi.list,
  });
}

export function useBackupJob(id: string) {
  return useQuery({
    queryKey: ['jobs', id],
    queryFn: () => jobsApi.get(id),
    enabled: !!id,
  });
}

export function useCreateJob() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: jobsApi.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['jobs'] });
      toast.success('Backup job created');
    },
    onError: (err: Error) => toast.error(err.message),
  });
}

export function useUpdateJob() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> }) => jobsApi.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['jobs'] });
      toast.success('Job updated');
    },
    onError: (err: Error) => toast.error(err.message),
  });
}

export function useDeleteJob() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: jobsApi.delete,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['jobs'] });
      toast.success('Job deleted');
    },
    onError: (err: Error) => toast.error(err.message),
  });
}

export function useRunJob() {
  return useMutation({
    mutationFn: (jobId: string) => jobsApi.run(jobId),
    onSuccess: () => toast.success('Backup started'),
    onError: (err: Error) => toast.error(err.message),
  });
}

export function useCancelJob() {
  return useMutation({
    mutationFn: jobsApi.cancel,
    onSuccess: () => toast.success('Backup cancelled'),
    onError: (err: Error) => toast.error(err.message),
  });
}

export function useJobLogs(jobId: string) {
  return useQuery({
    queryKey: ['jobs', jobId, 'logs'],
    queryFn: () => jobsApi.logs(jobId),
    enabled: !!jobId,
  });
}
