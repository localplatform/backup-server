module.exports = {
  apps: [
    {
      name: 'backup-server',
      cwd: './server',
      script: 'node_modules/.bin/tsx',
      args: 'watch src/index.ts',
      env: {
        NODE_ENV: 'development',
        AGENT_PORT: '8080', // Rust backup agent port
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
