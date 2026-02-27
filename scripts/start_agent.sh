#!/bin/bash
# Steer OS Agent - Terminal Launcher
# Run this from Terminal to inherit Screen Recording permission
set -e

AGENT_BIN="/Users/david/Desktop/python/github/Allrounder/Steer/local-os-agent/core/target/release/local_os_agent"
WORKDIR="/Users/david/Desktop/python/github/Allrounder/Steer/local-os-agent/core"
LOG_DIR="$HOME/.local-os-agent"
RUNTIME_PROFILE="${STEER_RUNTIME_PROFILE:-dev}"

mkdir -p "$LOG_DIR"

graceful_stop_local_os_agent() {
    local pids=""
    pids="$(pgrep -f "[l]ocal_os_agent" 2>/dev/null || true)"
    if [ -z "$pids" ]; then
        return 0
    fi
    kill $pids 2>/dev/null || true
    sleep 2
    pids="$(pgrep -f "[l]ocal_os_agent" 2>/dev/null || true)"
    if [ -n "$pids" ]; then
        kill -9 $pids 2>/dev/null || true
    fi
}

# Unload launchd service if running (avoid port conflict)
launchctl unload ~/Library/LaunchAgents/com.steer.local-os-agent.plist 2>/dev/null || true
graceful_stop_local_os_agent
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
export STEER_DISABLE_EVENT_TAP=1
export STEER_COLLECTOR_HANDOFF_AUTOCONSUME=0

if [ "$RUNTIME_PROFILE" = "prod" ]; then
    export STEER_API_ALLOW_NO_KEY=0
    if [ -z "${STEER_API_KEY:-}" ]; then
        echo "❌ prod profile requires STEER_API_KEY"
        exit 1
    fi
else
    export STEER_API_ALLOW_NO_KEY="${STEER_API_ALLOW_NO_KEY:-1}"
fi

exec "$AGENT_BIN"
