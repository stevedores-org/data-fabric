#!/usr/bin/env bash
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Deploy data-fabric to remote machine via autossh + tmux
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

set -euo pipefail

# Configuration
REMOTE_USER="${REMOTE_USER:-aivcs}"
REMOTE_HOST="${REMOTE_HOST:-aivcs.local}"
TMUX_SESSION="${TMUX_SESSION:-onion}"
REMOTE_WORK_DIR="${REMOTE_WORK_DIR:-/tmp/data-fabric}"
LOCAL_REPO="${LOCAL_REPO:-.}"

echo "🚀 Deploying data-fabric to $REMOTE_USER@$REMOTE_HOST:$REMOTE_WORK_DIR"
echo ""

# Step 1: Copy repository to remote
echo "📦 Copying repository to remote machine..."
ssh "$REMOTE_USER@$REMOTE_HOST" "mkdir -p $(dirname $REMOTE_WORK_DIR)" || true
rsync -avz --delete "$LOCAL_REPO/" "$REMOTE_USER@$REMOTE_HOST:$REMOTE_WORK_DIR/" \
  --exclude=.git \
  --exclude=.wrangler \
  --exclude=build \
  --exclude=target \
  --exclude=node_modules

echo "✅ Repository copied"
echo ""

# Step 2: Set up environment on remote
echo "🔧 Setting up environment on remote machine..."
ssh "$REMOTE_USER@$REMOTE_HOST" bash <<'REMOTE_SETUP'
export PATH="$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:$PATH"

echo "  Checking Rust..."
rustc --version || echo "  ⚠️  Rust not installed"

echo "  Checking Cargo..."
cargo --version || echo "  ⚠️  Cargo not installed"

echo "  Checking Bun..."
bun --version || echo "  ⚠️  Bun not installed"

echo "  Checking tmux..."
tmux -V || echo "  ⚠️  tmux not installed"
REMOTE_SETUP

echo "✅ Environment checked"
echo ""

# Step 3: Start data-fabric in tmux session
echo "🎬 Starting data-fabric in tmux session '$TMUX_SESSION'..."
ssh "$REMOTE_USER@$REMOTE_HOST" bash <<REMOTE_START
export PATH="\$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:\$PATH"

cd $REMOTE_WORK_DIR

# Kill existing session if it exists
tmux kill-session -t $TMUX_SESSION 2>/dev/null || true

# Create new session and run data-fabric
tmux new-session -d -s $TMUX_SESSION -x 200 -y 50

# Run in the session
tmux send-keys -t $TMUX_SESSION "cd $REMOTE_WORK_DIR" Enter
tmux send-keys -t $TMUX_SESSION "export PATH=\$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:\$PATH" Enter
tmux send-keys -t $TMUX_SESSION "bunx wrangler dev --local" Enter

# Show session status
sleep 2
echo "📋 Session info:"
tmux list-sessions
echo ""
echo "📺 Attaching to session (you can detach with Ctrl+B, D):"
REMOTE_START

echo "✅ data-fabric started in tmux session"
echo ""

# Step 4: Show connection info
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🎯 Connection Info"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "To attach to the running session:"
echo "  autossh -M 0 -t $REMOTE_USER@$REMOTE_HOST '/opt/homebrew/bin/tmux attach-session -t $TMUX_SESSION'"
echo ""
echo "Or with shorter command:"
echo "  autossh -M 0 -t $REMOTE_USER@$REMOTE_HOST 'tmux a -t $TMUX_SESSION'"
echo ""
echo "To view logs:"
echo "  ssh $REMOTE_USER@$REMOTE_HOST 'tmux capture-pane -t $TMUX_SESSION -p'"
echo ""
echo "To restart:"
echo "  ssh $REMOTE_USER@$REMOTE_HOST 'tmux send-keys -t $TMUX_SESSION C-c'"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "✅ Deployment complete!"
