# WebSocket API Documentation

## Overview

The backup agent provides a WebSocket endpoint at `/ws` for real-time bidirectional communication between the agent and server.

## Connection

Connect to the WebSocket endpoint:
```
ws://agent-host:8080/ws
```

## Message Format

All messages are JSON with a `type` and `payload` structure.

### Events (Agent → Server)

#### Backup Progress
```json
{
  "type": "backup:progress",
  "payload": {
    "job_id": "abc-123",
    "percent": 45.5,
    "transferred_bytes": 1048576,
    "total_bytes": 2097152,
    "bytes_per_second": 524288,
    "eta_seconds": 2,
    "current_file": "/data/file.txt",
    "files_processed": 10,
    "total_files": 25
  }
}
```

#### Backup Started
```json
{
  "type": "backup:started",
  "payload": {
    "job_id": "abc-123"
  }
}
```

#### Backup Completed
```json
{
  "type": "backup:completed",
  "payload": {
    "job_id": "abc-123",
    "total_bytes": 2097152
  }
}
```

#### Backup Failed
```json
{
  "type": "backup:failed",
  "payload": {
    "job_id": "abc-123",
    "error": "Network timeout"
  }
}
```

#### Agent Status
```json
{
  "type": "agent:status",
  "payload": {
    "status": "running",
    "active_jobs": 2,
    "uptime_secs": 3600
  }
}
```

### Commands (Server → Agent)

#### Pause Backup
```json
{
  "type": "backup:pause",
  "payload": {
    "job_id": "abc-123"
  }
}
```

#### Resume Backup
```json
{
  "type": "backup:resume",
  "payload": {
    "job_id": "abc-123"
  }
}
```

#### Cancel Backup
```json
{
  "type": "backup:cancel",
  "payload": {
    "job_id": "abc-123"
  }
}
```

#### Get Status
```json
{
  "type": "agent:status",
  "payload": null
}
```

## Example Usage (JavaScript)

```javascript
const ws = new WebSocket('ws://localhost:8080/ws');

ws.onopen = () => {
  console.log('Connected to backup agent');
};

ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  
  if (message.type === 'backup:progress') {
    const { percent, bytes_per_second, current_file } = message.payload;
    console.log(`Progress: ${percent}% - ${current_file} - ${bytes_per_second} B/s`);
  }
};

// Send a command
ws.send(JSON.stringify({
  type: 'backup:pause',
  payload: { job_id: 'abc-123' }
}));
```

## Architecture

- **Broadcasting**: Progress updates are broadcast to all connected clients via tokio::sync::broadcast channel
- **Concurrency**: Each WebSocket connection spawns two tasks: one for sending events, one for receiving commands
- **State Management**: Shared WsState with Arc<RwLock<>> for thread-safe access
- **Error Handling**: Graceful disconnection on client/server errors

## Integration

To broadcast an event from anywhere in the agent:

```rust
use crate::ws::{WsEvent, BackupProgressPayload};

// Get the WsState from app state
let ws_state = /* get from router state */;

// Broadcast progress
ws_state.broadcast(WsEvent::BackupProgress(BackupProgressPayload {
    job_id: "abc-123".to_string(),
    percent: 50.0,
    transferred_bytes: 1024,
    total_bytes: 2048,
    bytes_per_second: 512,
    eta_seconds: 2,
    current_file: Some("test.txt".to_string()),
    files_processed: 5,
    total_files: 10,
}));
```
