import { useQuery } from '@tanstack/react-query';
import { storageApi } from '../api/endpoints.js';

export function useStorageHierarchy() {
  return useQuery({
    queryKey: ['storage', 'hierarchy'],
    queryFn: () => storageApi.getHierarchy(),
  });
}
