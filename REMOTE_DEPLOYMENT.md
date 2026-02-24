# Data Fabric — Remote Deployment (Studio Machine)

Deploy and run data-fabric on a remote machine via autossh + tmux.

## Quick Start

### Option 1: Automated Deployment Script

```bash
cd /path/to/data-fabric

# Deploy to studio machine
bash deploy-remote.sh
```

This will:
- Copy repository to remote machine
- Set up environment
- Start data-fabric in tmux session named "onion"

### Option 2: Manual Deployment

#### Step 1: Copy Repository

```bash
rsync -avz --delete ./ aivcs@studio:/tmp/data-fabric/ \
  --exclude=.git \
  --exclude=.wrangler \
  --exclude=build \
  --exclude=target
```

#### Step 2: Connect via autossh

```bash
autossh -M 0 -t aivcs@studio "/opt/homebrew/bin/tmux new-session -A -s onion"
```

#### Step 3: Start data-fabric in the session

```bash
# Inside the tmux session (after autossh connects)
cd /tmp/data-fabric
export PATH="$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:$PATH"
bunx wrangler dev --local
```

## Usage

### View Running Session

```bash
# List sessions
ssh aivcs@studio "tmux list-sessions"

# View logs in session
ssh aivcs@studio "tmux capture-pane -t onion -p"

# Attach to session
autossh -M 0 -t aivcs@studio "/opt/homebrew/bin/tmux attach-session -t onion"
```

### Stop/Restart

```bash
# Stop (from within tmux)
Ctrl+C

# Or remotely
ssh aivcs@studio "tmux send-keys -t onion C-c"

# Restart
ssh aivcs@studio "tmux send-keys -t onion 'bunx wrangler dev --local' Enter"
```

### Kill Session

```bash
ssh aivcs@studio "tmux kill-session -t onion"
```

## Testing Remote Instance

### Health Check

Once running, test from your local machine:

```bash
# If studio machine forwards port 8787 to your machine
curl http://localhost:8787/health

# Or SSH tunnel to it
ssh -L 8787:localhost:8787 aivcs@studio

# Then in another terminal
curl http://localhost:8787/health
```

### With Gemini CLI

```bash
# Assuming remote forwarding is set up
export FABRIC_URL=http://studio.local:8787
gemini extensions add ./mcp-server
gemini
# > Test if data-fabric is healthy
```

## Network Configuration

### Port Forwarding via SSH

```bash
# Keep a tunnel open in background
autossh -M 20000 -N -L 8787:localhost:8787 aivcs@studio

# Then access from your machine
curl http://localhost:8787/health
```

### Or in SSH Config

Add to `~/.ssh/config`:

```
Host studio
    HostName studio.local
    User aivcs
    LocalForward 8787 localhost:8787
    ServerAliveInterval 30
    ServerAliveCountMax 2
```

Then:

```bash
ssh -N studio  # Keep connection open
# In another terminal: curl http://localhost:8787/health
```

## tmux Cheat Sheet

**Inside tmux session:**

| Command | Action |
|---------|--------|
| `Ctrl+B, D` | Detach from session |
| `Ctrl+B, [` | Enter copy mode (scroll logs) |
| `Ctrl+B, ]` | Paste |
| `Ctrl+B, ?` | Show help |

**From outside:**

```bash
# Attach
tmux attach-session -t onion

# Send keys/commands
tmux send-keys -t onion "command" Enter

# Capture output
tmux capture-pane -t onion -p

# Kill window
tmux kill-window -t onion:0

# Kill session
tmux kill-session -t onion
```

## Troubleshooting

### "Connection refused"

```bash
# Check if session is running
ssh aivcs@studio "tmux list-sessions"

# Check if port is being used
ssh aivcs@studio "lsof -i :8787"

# Check logs
ssh aivcs@studio "tmux capture-pane -t onion -p | tail -50"
```

### "autossh connection lost"

```bash
# autossh will automatically reconnect
# But check your SSH config
ssh -v aivcs@studio "echo OK"

# Verify keys are working
ssh -i ~/.ssh/id_rsa aivcs@studio "echo OK"
```

### "Rust not found"

On the remote machine:

```bash
# Check PATH
echo $PATH

# Verify Rust is installed
rustc --version
cargo --version

# Add to PATH in tmux command
export PATH="$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:$PATH"
```

## Integration with oxidizedgraph

Test remote data-fabric with oxidizedgraph:

```bash
# Set remote URL
export FABRIC_URL=http://studio.local:8787  # or localhost:8787 with port forwarding

# Run oxidizedgraph tests against remote instance
cd /path/to/oxidizedgraph
FABRIC_URL=http://studio.local:8787 cargo test --test integration_with_fabric
```

## Security Notes

- ✅ Use autossh for reliable connections
- ✅ Configure SSH keys (not passwords)
- ✅ Use SSH tunneling for port forwarding (encrypted)
- ✅ Restrict access to studio machine
- ✅ Monitor running processes regularly

## Monitoring

### Watch Session

```bash
# Keep watching logs
watch -n 2 'ssh aivcs@studio "tmux capture-pane -t onion -p | tail -20"'
```

### Health Check Loop

```bash
# Monitor health endpoint
while true; do
  curl -s http://localhost:8787/health | jq .
  sleep 5
done
```

---

**Remote data-fabric is now running and ready for testing!** 🚀
