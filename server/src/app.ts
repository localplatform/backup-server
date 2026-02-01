import express from 'express';
import path from 'path';
import { config } from './config.js';
import serversRouter from './routes/servers.js';
import sshRouter from './routes/ssh.js';
import explorerRouter from './routes/explorer.js';
import backupJobsRouter from './routes/backupJobs.js';
import storageRouter from './routes/storage.js';
import versionsRouter from './routes/versions.js';
import filesRouter from './routes/files.js';

export function createApp(): express.Application {
  const app = express();

  app.use(express.json());

  // API routes
  app.use('/api/servers', serversRouter);
  app.use('/api/servers', sshRouter);
  app.use('/api/servers', explorerRouter);
  app.use('/api/jobs', backupJobsRouter);
  app.use('/api/storage', storageRouter);
  app.use('/api/versions', versionsRouter);
  app.use('/api/files', filesRouter);

  // Serve client static files in production
  app.use(express.static(config.clientDist));

  // SPA fallback
  app.get('*', (_req, res) => {
    res.sendFile(path.join(config.clientDist, 'index.html'));
  });

  return app;
}
