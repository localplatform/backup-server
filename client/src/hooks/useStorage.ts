import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { storageApi } from '../api/endpoints.js';
import toast from 'react-hot-toast';

export function useStorageSettings() {
  return useQuery({
    queryKey: ['storage', 'settings'],
    queryFn: storageApi.getSettings,
  });
}

export function useUpdateStorageSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: storageApi.updateSettings,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['storage'] });
      toast.success('Storage settings saved');
    },
    onError: (err: Error) => toast.error(err.message),
  });
}

export function useStorageBrowse(path?: string) {
  return useQuery({
    queryKey: ['storage', 'browse', path],
    queryFn: () => storageApi.browse(path),
    enabled: path !== undefined,
  });
}

export function useDiskUsage() {
  return useQuery({
    queryKey: ['storage', 'disk-usage'],
    queryFn: storageApi.diskUsage,
  });
}

export function useVersionBrowse(versionId?: string, path?: string) {
  return useQuery({
    queryKey: ['storage', 'version-browse', versionId, path],
    queryFn: () => storageApi.browseVersion(versionId!, path),
    enabled: !!versionId,
  });
}
