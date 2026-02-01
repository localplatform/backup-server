# Backup Agent Deployment Guide

## Quick Deploy to 10.10.10.10

### Option 1: Automated Deployment (requires SSH key)

First, set up SSH access from the backup server:

```bash
# 1. Copy the backup server's public key
cat ~/.ssh/id_ed25519.pub

# 2. On the remote server (10.10.10.10), add it to authorized_keys:
ssh 10.10.10.10
mkdir -p ~/.ssh
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHqNvzp5FO7r21D8ghMz/YazU0f7dHQ9nsRVMZIxz/ml backup-server@backup" >> ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
exit

# 3. Run automated deployment:
cd /backup-server/backup-agent
./deploy.sh 10.10.10.10
```

### Option 2: Manual Deployment

```bash
# On the backup server:
cd /backup-server/backup-agent

# 1. Copy binary to remote server
scp target/release/backup-agent root@10.10.10.10:/opt/backup-agent/
scp config.production.toml root@10.10.10.10:/opt/backup-agent/config.toml

# 2. On remote server (10.10.10.10):
ssh root@10.10.10.10

# Create directories
mkdir -p /opt/backup-agent /var/lib/backup-agent

# Make binary executable
chmod +x /opt/backup-agent/backup-agent

# Start the agent
nohup /opt/backup-agent/backup-agent --config /opt/backup-agent/config.toml > /var/log/backup-agent.log 2>&1 &

# Verify it's running
curl http://localhost:8080/health

# Should return: {"status":"ok","version":"...","uptime_secs":...}
```

### Option 3: Copy-Paste Deployment (no SSH required)

**On the backup server**, run:

```bash
cd /backup-server/backup-agent
base64 target/release/backup-agent > /tmp/agent.b64
echo "Binary encoded. Size: $(wc -c < /tmp/agent.b64) bytes"
```

**On 10.10.10.10**, run:

```bash
# Paste the base64 content and decode:
cat > /tmp/agent.b64 << 'EOF'
[PASTE BASE64 CONTENT HERE]
EOF

base64 -d /tmp/agent.b64 > /opt/backup-agent/backup-agent
chmod +x /opt/backup-agent/backup-agent

# Create config:
cat > /opt/backup-agent/config.toml << 'EOF'
[agent]
id = "agent-10.10.10.10"
port = 8080
data_dir = "/var/lib/backup-agent"

[server]
url = "http://10.10.10.1:3000"
token = ""

[sync]
chunk_size = 1048576
compression = "none"
compression_level = 3

[log]
level = "info"
output = "stdout"

[daemon]
pid_file = "/var/run/backup-agent.pid"
user = "root"
group = "root"

[performance]
max_concurrent_jobs = 1
io_threads = 4
EOF

# Create data directory
mkdir -p /var/lib/backup-agent

# Start agent
nohup /opt/backup-agent/backup-agent --config /opt/backup-agent/config.toml > /var/log/backup-agent.log 2>&1 &

# Test
curl http://localhost:8080/health
```

## Verification

After deployment, verify the agent is working:

```bash
# Health check
curl http://10.10.10.10:8080/health

# Version info
curl http://10.10.10.10:8080/version

# View logs
ssh root@10.10.10.10 'tail -f /var/log/backup-agent.log'
```

## Integration with Backup Server

Once the agent is deployed, you need to:

1. **Update server configuration** to use agents instead of rsync
2. **Test a backup job** from the UI
3. **Monitor WebSocket connection** for real-time progress

The backup server will automatically connect to the agent at `http://10.10.10.10:8080` when you configure a server to use the agent-based backup method.

## Troubleshooting

### Agent won't start
```bash
# Check logs
tail -50 /var/log/backup-agent.log

# Check if port is already in use
netstat -tulpn | grep 8080

# Check file permissions
ls -la /opt/backup-agent/backup-agent
```

### Can't connect from backup server
```bash
# Test from backup server
curl http://10.10.10.10:8080/health

# Check firewall (on 10.10.10.10)
iptables -L -n | grep 8080
```

### WebSocket not working
```bash
# Test WebSocket upgrade (should return 426 for HTTP request)
curl -I http://10.10.10.10:8080/ws
```
