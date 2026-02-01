module.exports = {
  apps: [
    {
      name: 'backup-server',
      cwd: './server',
      script: 'node_modules/.bin/tsx',
      args: 'watch src/index.ts',
      env: {
        NODE_ENV: 'development',
        AGENT_PORT: '9990', // Rust backup agent port
        BACKUP_SERVER_IP: '10.10.10.100', // IP accessible by remote agents
      },
      kill_timeout: 10000, // 10s for graceful shutdown (DB flush + cleanup)
      max_restarts: 10, // Limit infinite restart loops
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
