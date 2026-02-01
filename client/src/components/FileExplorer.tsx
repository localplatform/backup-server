import { useState, useEffect } from 'react';
import { Folder, File, ChevronRight, Loader2 } from 'lucide-react';
import { serversApi, RemoteEntry } from '../api/endpoints.js';
import './FileExplorer.scss';

interface Props {
  serverId: string;
  onSelect?: (path: string) => void;
  selectable?: boolean;
  selectedPaths?: string[];
}

interface DirNode {
  path: string;
  entries: RemoteEntry[];
  expanded: boolean;
  loading: boolean;
  error?: string | null;
}

export default function FileExplorer({ serverId, onSelect, selectable, selectedPaths = [] }: Props) {
  const [dirs, setDirs] = useState<Map<string, DirNode>>(new Map());

  const loadDir = async (path: string) => {
    const existing = dirs.get(path);
    if (existing?.expanded) {
      setDirs(prev => {
        const next = new Map(prev);
        next.set(path, { ...existing, expanded: false });
        return next;
      });
      return;
    }

    setDirs(prev => {
      const next = new Map(prev);
      next.set(path, { path, entries: existing?.entries || [], expanded: true, loading: true });
      return next;
    });

    try {
      const entries = await serversApi.explore(serverId, path);
      setDirs(prev => {
        const next = new Map(prev);
        next.set(path, { path, entries, expanded: true, loading: false });
        return next;
      });
    } catch (err: any) {
      const errorMsg = err?.response?.data?.error || (err instanceof Error ? err.message : 'Failed to load');
      setDirs(prev => {
        const next = new Map(prev);
        next.set(path, { path, entries: [], expanded: true, loading: false, error: errorMsg });
        return next;
      });
    }
  };

  // Auto-load root directory when component mounts or serverId changes
  useEffect(() => {
    setDirs(new Map());
    loadDir('/');
  }, [serverId]);

  const renderEntries = (entries: RemoteEntry[], depth: number): React.ReactNode => {
    return entries.map(entry => {
      const isDir = entry.type === 'directory';
      const node = dirs.get(entry.path);
      const isSelected = selectedPaths.includes(entry.path);

      return (
        <div key={entry.path}>
          <div
            className={`explorer-row ${isSelected ? 'selected' : ''}`}
            style={{ paddingLeft: `${depth * 1.25 + 0.5}rem` }}
            onClick={() => isDir && loadDir(entry.path)}
          >
            {isDir ? (
              <>
                <ChevronRight size={14} className={`chevron ${node?.expanded ? 'open' : ''}`} />
                <Folder size={16} className="icon folder" />
              </>
            ) : (
              <>
                <span style={{ width: 14 }} />
                <File size={16} className="icon" />
              </>
            )}
            <span className="entry-name">{entry.name}</span>
            {!isDir && <span className="entry-size">{formatSize(entry.size)}</span>}
            {selectable && isDir && onSelect && (
              <button
                type="button"
                className="btn btn-sm select-btn"
                onClick={e => { e.stopPropagation(); onSelect(entry.path); }}
              >
                Select
              </button>
            )}
            {node?.loading && <Loader2 size={14} className="spinner" />}
          </div>
          {node?.expanded && node.error && (
            <div
              className="explorer-row-error"
              style={{ paddingLeft: `${(depth + 1) * 1.25 + 0.5}rem` }}
            >
              {node.error}
            </div>
          )}
          {node?.expanded && !node.error && node.entries.length > 0 && renderEntries(node.entries, depth + 1)}
        </div>
      );
    });
  };

  const rootNode = dirs.get('/');

  return (
    <div className="file-explorer">
      {rootNode?.error && <div className="explorer-error">{rootNode.error}</div>}
      {rootNode?.loading && <div className="explorer-loading"><Loader2 size={16} className="spinner" /> Loading...</div>}
      {rootNode?.entries && renderEntries(rootNode.entries, 0)}
    </div>
  );
}

function formatSize(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}
