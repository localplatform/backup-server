module.exports = {
  apps: [
    {
      name: 'backup-server',
      script: './target/release/backup-server',
      env: {
        PORT: '3000',
        BACKUPS_DIR: '/backup/data/backups',
        BACKUP_SERVER_IP: '10.10.10.100',
        RUST_LOG: 'info',
      },
      kill_timeout: 10000,
      max_restarts: 10,
    },
    {
      name: 'backup-client',
      cwd: './client',
      script: 'node_modules/.bin/vite',
      args: '--host',
      env: {
        NODE_ENV: 'development',
      },
    },
  ],
};
