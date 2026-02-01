import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { AxiosError } from 'axios';
import { serversApi, Server } from '../api/endpoints.js';
import toast from 'react-hot-toast';

function apiError(err: unknown): string {
  if (err instanceof AxiosError && err.response?.data?.error) {
    return typeof err.response.data.error === 'string'
      ? err.response.data.error
      : JSON.stringify(err.response.data.error);
  }
  return err instanceof Error ? err.message : 'Unknown error';
}

export function useServers() {
  return useQuery({
    queryKey: ['servers'],
    queryFn: serversApi.list,
  });
}

export function useServer(id: string) {
  return useQuery({
    queryKey: ['servers', id],
    queryFn: () => serversApi.get(id),
    enabled: !!id,
  });
}

export function useCreateServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: serversApi.create,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['servers'] });
      toast.success('Server added and connected');
    },
    onError: (err) => toast.error(apiError(err)),
  });
}

export function useUpdateServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, data }: { id: string; data: Partial<Server> }) => serversApi.update(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['servers'] });
      toast.success('Server updated');
    },
    onError: (err: Error) => toast.error(err.message),
  });
}

export function useDeleteServer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: serversApi.delete,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['servers'] });
      toast.success('Server deleted');
    },
    onError: (err: Error) => toast.error(err.message),
  });
}
