import { useState } from 'react';
import { Folder, File, ChevronRight, Loader2 } from 'lucide-react';
import { useVersionBrowse } from '../hooks/useStorage.js';
import type { LocalEntry } from '../api/endpoints.js';
import './LocalFileExplorer.scss';

interface Props {
  versionId: string | null;
}

interface DirNode {
  path: string;
  entries: LocalEntry[];
  expanded: boolean;
}

export default function LocalFileExplorer({ versionId }: Props) {
  const [expandedDirs, setExpandedDirs] = useState<Map<string, DirNode>>(new Map([['/', { path: '/', entries: [], expanded: true }]]));
  const rootDir = expandedDirs.get('/');
  const { data: rootEntries = [], isLoading: rootLoading, error: rootError } = useVersionBrowse(versionId ?? undefined, '/');

  // Update root entries when loaded
  if (rootDir && rootEntries && rootEntries.length > 0 && rootDir.entries.length === 0) {
    const newMap = new Map(expandedDirs);
    newMap.set('/', { ...rootDir, entries: rootEntries });
    if (newMap !== expandedDirs) {
      setExpandedDirs(newMap);
    }
  }

  const toggleDir = (path: string) => {
    const node = expandedDirs.get(path);
    if (node?.expanded) {
      // Collapse
      setExpandedDirs(prev => {
        const next = new Map(prev);
        next.set(path, { ...node, expanded: false });
        return next;
      });
    } else {
      // Expand
      setExpandedDirs(prev => {
        const next = new Map(prev);
        next.set(path, { path, entries: node?.entries || [], expanded: true });
        return next;
      });
    }
  };

  const renderEntries = (entries: LocalEntry[], depth: number): React.ReactNode => {
    return entries.map(entry => {
      const isDir = entry.type === 'directory';
      const node = expandedDirs.get(entry.path);

      return (
        <DirEntry
          key={entry.path}
          entry={entry}
          versionId={versionId}
          depth={depth}
          isExpanded={node?.expanded || false}
          onToggle={() => isDir && toggleDir(entry.path)}
          childEntries={node?.entries}
        />
      );
    });
  };

  if (!versionId) {
    return (
      <div className="local-file-explorer">
        <div className="explorer-empty">Select a backup version to browse its contents</div>
      </div>
    );
  }

  return (
    <div className="local-file-explorer">
      {rootError && <div className="explorer-error">Failed to load backup contents</div>}
      {rootLoading && <div className="explorer-loading"><Loader2 size={16} className="spinner" /> Loading...</div>}
      {rootDir?.entries && renderEntries(rootDir.entries, 0)}
    </div>
  );
}

interface DirEntryProps {
  entry: LocalEntry;
  versionId: string | null;
  depth: number;
  isExpanded: boolean;
  onToggle: () => void;
  childEntries?: LocalEntry[];
}

function DirEntry({ entry, versionId, depth, isExpanded, onToggle, childEntries }: DirEntryProps) {
  const isDir = entry.type === 'directory';
  const { data: entries = [], isLoading } = useVersionBrowse(
    versionId ?? undefined,
    isExpanded && isDir ? entry.path : undefined
  );

  // Use fetched entries if available, otherwise use passed childEntries
  const displayEntries = entries.length > 0 ? entries : childEntries || [];

  return (
    <div>
      <div
        className="explorer-row"
        style={{ paddingLeft: `${depth * 1.25 + 0.5}rem` }}
        onClick={onToggle}
      >
        {isDir ? (
          <>
            <ChevronRight size={14} className={`chevron ${isExpanded ? 'open' : ''}`} />
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
        {isLoading && <Loader2 size={14} className="spinner" />}
      </div>
      {isExpanded && isDir && displayEntries.length > 0 && renderEntriesRecursive(displayEntries, versionId, depth + 1)}
    </div>
  );
}

function renderEntriesRecursive(entries: LocalEntry[], versionId: string | null, depth: number): React.ReactNode {
  return entries.map(entry => {
    const isDir = entry.type === 'directory';
    const [isExpanded, setIsExpanded] = useState(false);

    return (
      <DirEntry
        key={entry.path}
        entry={entry}
        versionId={versionId}
        depth={depth}
        isExpanded={isExpanded}
        onToggle={() => isDir && setIsExpanded(!isExpanded)}
      />
    );
  });
}

function formatSize(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}
