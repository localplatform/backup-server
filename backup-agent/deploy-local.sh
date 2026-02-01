#!/bin/bash
set -e

# Local deployment helper - serves files via HTTP for manual download
# Usage: ./deploy-local.sh [port]

PORT="${1:-9090}"
BINARY_PATH="target/release/backup-agent"
CONFIG_PATH="config.production.toml"

echo "=================================================="
echo "Backup Agent Local Deployment Server"
echo "=================================================="
echo ""

# Check if binary exists
if [ ! -f "$BINARY_PATH" ]; then
    echo "ERROR: Binary not found at $BINARY_PATH"
    echo "Run: cargo build --release"
    exit 1
fi

# Create temporary deployment directory
DEPLOY_DIR="/tmp/agent-deploy-$$"
mkdir -p "$DEPLOY_DIR"

# Copy files
cp "$BINARY_PATH" "$DEPLOY_DIR/backup-agent"
cp "$CONFIG_PATH" "$DEPLOY_DIR/config.toml"

# Create installation script
cat > "$DEPLOY_DIR/install.sh" << 'INSTALL_SCRIPT'
#!/bin/bash
set -e

echo "Installing Backup Agent..."

# Create directories
mkdir -p /opt/backup-agent /var/lib/backup-agent

# Move files
mv backup-agent /opt/backup-agent/
mv config.toml /opt/backup-agent/

# Set permissions
chmod +x /opt/backup-agent/backup-agent

# Stop existing agent
pkill -f backup-agent || true
sleep 2

# Start agent
nohup /opt/backup-agent/backup-agent --config /opt/backup-agent/config.toml > /var/log/backup-agent.log 2>&1 &

# Wait for startup
sleep 3

# Test
echo ""
echo "Testing agent..."
if curl -s http://localhost:8080/health | grep -q "ok"; then
    echo "✓ Agent installed and running successfully!"
    echo ""
    echo "Health check:"
    curl -s http://localhost:8080/health | jq .
    echo ""
    echo "Version info:"
    curl -s http://localhost:8080/version | jq .
else
    echo "✗ Agent health check failed"
    echo "Check logs: tail -f /var/log/backup-agent.log"
    exit 1
fi

echo ""
echo "Installation complete!"
echo "  Binary: /opt/backup-agent/backup-agent"
echo "  Config: /opt/backup-agent/config.toml"
echo "  Logs: /var/log/backup-agent.log"
INSTALL_SCRIPT

chmod +x "$DEPLOY_DIR/install.sh"

# Get local IP
LOCAL_IP=$(ip addr show enp2s0 | grep "inet " | awk '{print $2}' | cut -d/ -f1)

echo "Deployment files ready in: $DEPLOY_DIR"
echo ""
echo "Files available:"
echo "  - backup-agent ($(du -h $DEPLOY_DIR/backup-agent | cut -f1))"
echo "  - config.toml"
echo "  - install.sh"
echo ""
echo "=================================================="
echo "Starting HTTP server on port $PORT..."
echo "=================================================="
echo ""
echo "On the remote server (10.10.10.10), run:"
echo ""
echo "  cd /tmp"
echo "  wget http://$LOCAL_IP:$PORT/backup-agent"
echo "  wget http://$LOCAL_IP:$PORT/config.toml"
echo "  wget http://$LOCAL_IP:$PORT/install.sh"
echo "  chmod +x install.sh"
echo "  sudo ./install.sh"
echo ""
echo "Or use the one-liner:"
echo ""
echo "  cd /tmp && wget -q http://$LOCAL_IP:$PORT/install.sh && chmod +x install.sh && sudo ./install.sh"
echo ""
echo "=================================================="
echo "Press Ctrl+C to stop the server"
echo "=================================================="
echo ""

# Start simple HTTP server
cd "$DEPLOY_DIR"
python3 -m http.server $PORT

# Cleanup on exit
trap "rm -rf $DEPLOY_DIR" EXIT
