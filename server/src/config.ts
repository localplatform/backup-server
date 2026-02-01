import dotenv from 'dotenv';
import path from 'path';
import { fileURLToPath } from 'url';

dotenv.config();

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const serverRoot = path.resolve(__dirname, '..');

export const config = {
  port: parseInt(process.env.PORT || '3000', 10),
  dataDir: path.join(serverRoot, 'data'),
  dbPath: path.join(serverRoot, 'data', 'backup-server.db'),
  keysDir: path.join(serverRoot, 'data', 'keys'),
  backupsDir: process.env.BACKUPS_DIR || '/backup/data/backups', // HDD mount point
  clientDist: path.resolve(serverRoot, '..', 'client', 'dist'),
  logLevel: process.env.LOG_LEVEL || 'info',
  maxConcurrentGlobal: parseInt(process.env.MAX_CONCURRENT_GLOBAL || '8', 10),
  maxConcurrentPerServer: parseInt(process.env.MAX_CONCURRENT_PER_SERVER || '4', 10),
};
