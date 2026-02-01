import fs from 'fs';
import path from 'path';
import { execSync } from 'child_process';

export interface BackupMeta {
  server: { name: string; hostname: string; port: number };
  job: { name: string; remotePaths: string[] };
  createdAt: string;
  lastRunAt: string;
}

export interface LocalEntry {
  name: string;
  path: string;
  type: 'file' | 'directory' | 'symlink' | 'other';
  size: number;
  modifiedAt: string;
  backupMeta?: BackupMeta;
}

export interface DiskUsage {
  total: number;
  used: number;
  available: number;
  usedPercent: number;
}

function assertWithinRoot(rootPath: string, targetPath: string): string {
  // Strip leading slash so path.resolve treats it as relative to rootPath
  const relative = targetPath.replace(/^\/+/, '');
  const resolved = path.resolve(rootPath, relative);
  if (!resolved.startsWith(rootPath)) {
    throw new Error('Path outside of backup root');
  }
  return resolved;
}

export async function exploreLocal(rootPath: string, subPath: string = '/'): Promise<LocalEntry[]> {
  const resolved = assertWithinRoot(rootPath, subPath);

  const dirents = await fs.promises.readdir(resolved, { withFileTypes: true });
  const entries: LocalEntry[] = [];

  for (const dirent of dirents) {
    // Hide metadata files from the listing
    if (dirent.name === '.backup-meta.json') continue;

    const fullPath = path.join(resolved, dirent.name);
    const relativePath = path.join(subPath, dirent.name);

    let type: LocalEntry['type'] = 'other';
    if (dirent.isDirectory()) type = 'directory';
    else if (dirent.isFile()) type = 'file';
    else if (dirent.isSymbolicLink()) type = 'symlink';

    let size = 0;
    let modifiedAt = new Date().toISOString();
    try {
      const stat = await fs.promises.stat(fullPath);
      size = stat.size;
      modifiedAt = stat.mtime.toISOString();
    } catch {
      // stat may fail on broken symlinks or permission issues
    }

    const entry: LocalEntry = { name: dirent.name, path: relativePath, type, size, modifiedAt };

    // Attach backup metadata if available
    if (type === 'directory') {
      try {
        const metaContent = await fs.promises.readFile(path.join(fullPath, '.backup-meta.json'), 'utf-8');
        entry.backupMeta = JSON.parse(metaContent);
      } catch {
        // No metadata file or invalid JSON
      }
    }

    entries.push(entry);
  }

  entries.sort((a, b) => {
    if (a.type === 'directory' && b.type !== 'directory') return -1;
    if (a.type !== 'directory' && b.type === 'directory') return 1;
    return a.name.localeCompare(b.name);
  });

  return entries;
}

export function getDiskUsage(dirPath: string): DiskUsage {
  const output = execSync(`df -B1 "${dirPath}"`, { encoding: 'utf-8' });
  const lines = output.trim().split('\n');
  // Second line contains the values
  const parts = lines[1].split(/\s+/);
  const total = parseInt(parts[1], 10);
  const used = parseInt(parts[2], 10);
  const available = parseInt(parts[3], 10);
  const usedPercent = total > 0 ? Math.round((used / total) * 100) : 0;

  return { total, used, available, usedPercent };
}
