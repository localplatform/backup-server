import { useState } from 'react';
import { Folder, File, ChevronRight, Loader2 } from 'lucide-react';
import { useVersionBrowse } from '../hooks/useStorage.js';
import type { LocalEntry } from '../api/endpoints.js';
import './LocalFileExplorer.scss';

interface Props {
  versionId: string | null;
}

export default function LocalFileExplorer({ versionId }: Props) {
  const { data: rootEntries = [], isLoading, error } = useVersionBrowse(versionId ?? undefined, '/');

  if (!versionId) {
    return (
      <div className="local-file-explorer">
        <div className="explorer-empty">Select a backup version to browse its contents</div>
      </div>
    );
  }

  return (
    <div className="local-file-explorer">
      {error && <div className="explorer-error">Failed to load backup contents</div>}
      {isLoading && <div className="explorer-loading"><Loader2 size={16} className="spinner" /> Loading...</div>}
      {rootEntries.map(entry => (
        <DirEntry key={entry.path} entry={entry} versionId={versionId} depth={0} />
      ))}
    </div>
  );
}

interface DirEntryProps {
  entry: LocalEntry;
  versionId: string;
  depth: number;
}

function DirEntry({ entry, versionId, depth }: DirEntryProps) {
  const isDir = entry.type === 'directory';
  const [isExpanded, setIsExpanded] = useState(false);
  const { data: entries = [], isLoading } = useVersionBrowse(
    versionId,
    isExpanded && isDir ? entry.path : undefined
  );

  return (
    <div>
      <div
        className="explorer-row"
        style={{ paddingLeft: `${depth * 1.25 + 0.5}rem` }}
        onClick={() => isDir && setIsExpanded(!isExpanded)}
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
      {isExpanded && isDir && entries.map(child => (
        <DirEntry key={child.path} entry={child} versionId={versionId} depth={depth + 1} />
      ))}
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
