#!/bin/bash
# Steer OS Agent - Terminal Launcher
# Run this from Terminal to inherit Screen Recording permission
set -e

AGENT_BIN="/Users/david/Desktop/python/github/Allrounder/Steer/local-os-agent/core/target/release/local_os_agent"
WORKDIR="/Users/david/Desktop/python/github/Allrounder/Steer/local-os-agent/core"
LOG_DIR="$HOME/.local-os-agent"

mkdir -p "$LOG_DIR"

# Unload launchd service if running (avoid port conflict)
launchctl unload ~/Library/LaunchAgents/com.steer.local-os-agent.plist 2>/dev/null || true
killall local_os_agent 2>/dev/null || true
sleep 1

echo "🚀 Starting Steer Agent from Terminal (Screen Recording inherited)..."
echo "   Binary: $AGENT_BIN"
echo "   WorkDir: $WORKDIR"
echo "   Logs: $LOG_DIR"
echo ""
echo "   Press Ctrl+C to stop"
echo "==========================================="

cd "$WORKDIR"

export RUST_LOG=info
export PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:$PATH"
export STEER_API_ALLOW_NO_KEY=1
export STEER_DISABLE_EVENT_TAP=1
export STEER_COLLECTOR_HANDOFF_AUTOCONSUME=0

exec "$AGENT_BIN"
