#!/bin/bash
set -e

# Deployment script for backup-agent
# Usage: ./deploy.sh <host> [port]

HOST="${1:-10.10.10.10}"
PORT="${2:-22}"
AGENT_PORT="8080"
BINARY_PATH="target/release/backup-agent"
CONFIG_PATH="config.production.toml"
REMOTE_DIR="/opt/backup-agent"
REMOTE_DATA_DIR="/var/lib/backup-agent"

echo "=================================================="
echo "Backup Agent Deployment to $HOST"
echo "=================================================="

# Check if binary exists
if [ ! -f "$BINARY_PATH" ]; then
    echo "ERROR: Binary not found at $BINARY_PATH"
    echo "Run: cargo build --release"
    exit 1
fi

# Check if config exists
if [ ! -f "$CONFIG_PATH" ]; then
    echo "ERROR: Config not found at $CONFIG_PATH"
    exit 1
fi

echo ""
echo "Step 1: Testing SSH connectivity..."
if ! ssh -p "$PORT" -o ConnectTimeout=5 "root@$HOST" "echo 'SSH connection OK'"; then
    echo "ERROR: Cannot connect to $HOST:$PORT via SSH"
    exit 1
fi

echo ""
echo "Step 2: Creating remote directories..."
ssh -p "$PORT" "root@$HOST" "mkdir -p $REMOTE_DIR $REMOTE_DATA_DIR"

echo ""
echo "Step 3: Copying binary ($(du -h $BINARY_PATH | cut -f1))..."
scp -P "$PORT" "$BINARY_PATH" "root@$HOST:$REMOTE_DIR/backup-agent"

echo ""
echo "Step 4: Copying configuration..."
scp -P "$PORT" "$CONFIG_PATH" "root@$HOST:$REMOTE_DIR/config.toml"

echo ""
echo "Step 5: Setting permissions..."
ssh -p "$PORT" "root@$HOST" "chmod +x $REMOTE_DIR/backup-agent"

echo ""
echo "Step 6: Stopping existing agent (if running)..."
ssh -p "$PORT" "root@$HOST" "pkill -f backup-agent || true"
sleep 2

echo ""
echo "Step 7: Starting agent..."
ssh -p "$PORT" "root@$HOST" "nohup $REMOTE_DIR/backup-agent --config $REMOTE_DIR/config.toml > /var/log/backup-agent.log 2>&1 &"

echo ""
echo "Step 8: Waiting for agent to start..."
sleep 3

echo ""
echo "Step 9: Checking agent health..."
if ssh -p "$PORT" "root@$HOST" "curl -s http://localhost:$AGENT_PORT/health" | grep -q "ok"; then
    echo "✓ Agent is running and healthy!"
else
    echo "✗ Agent health check failed"
    echo ""
    echo "Logs from remote server:"
    ssh -p "$PORT" "root@$HOST" "tail -20 /var/log/backup-agent.log"
    exit 1
fi

echo ""
echo "Step 10: Verifying WebSocket endpoint..."
if ssh -p "$PORT" "root@$HOST" "curl -s -I http://localhost:$AGENT_PORT/ws" | grep -q "426"; then
    echo "✓ WebSocket endpoint is accessible (426 Upgrade Required is expected for HTTP)"
else
    echo "✗ WebSocket endpoint check failed"
fi

echo ""
echo "=================================================="
echo "Deployment completed successfully!"
echo "=================================================="
echo ""
echo "Agent Details:"
echo "  Host: $HOST"
echo "  HTTP API: http://$HOST:$AGENT_PORT"
echo "  WebSocket: ws://$HOST:$AGENT_PORT/ws"
echo "  Binary: $REMOTE_DIR/backup-agent"
echo "  Config: $REMOTE_DIR/config.toml"
echo "  Logs: /var/log/backup-agent.log"
echo ""
echo "Next steps:"
echo "  1. Test health: curl http://$HOST:$AGENT_PORT/health"
echo "  2. Test version: curl http://$HOST:$AGENT_PORT/version"
echo "  3. View logs: ssh root@$HOST 'tail -f /var/log/backup-agent.log'"
echo "  4. Configure a backup job to use this agent"
echo ""
