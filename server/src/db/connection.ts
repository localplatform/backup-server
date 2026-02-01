import Database from 'better-sqlite3';
import { config } from '../config.js';

let db: Database.Database | null = null;

export function getDb(): Database.Database {
  if (!db) {
    db = new Database(config.dbPath);
    // Use DELETE mode instead of WAL for better durability with frequent restarts
    db.pragma('journal_mode = DELETE');
    db.pragma('synchronous = FULL');
    db.pragma('foreign_keys = ON');
  }
  return db;
}

export function closeDb(): void {
  if (db) {
    db.close();
    db = null;
  }
}

export function flushDatabase(): void {
  // Force database writes to disk
  // In DELETE mode, this is mostly a no-op, but useful if we switch back to WAL
  if (db) {
    try {
      db.pragma('wal_checkpoint(FULL)');
    } catch {
      // Ignored in DELETE mode (expected to fail)
    }
  }
}
