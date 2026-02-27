#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CORE_DIR="$ROOT_DIR/core"
BIN_PATH="$CORE_DIR/target/debug/local_os_agent"
RUNTIME_PROFILE="${STEER_RUNTIME_PROFILE:-dev}"

LOG_DIR="$HOME/.local-os-agent"
LOG_PATH="$LOG_DIR/manual_agent.log"
PID_PATH="$LOG_DIR/manual_agent.pid"
PRIMARY_PLIST="$HOME/Library/LaunchAgents/com.steer.local-os-agent.plist"
LEGACY_PLIST="$HOME/Library/LaunchAgents/com.antigravity.agent.plist"
LOCK_DIR="$HOME/.steer/locks"

API_STATUS_URL="http://127.0.0.1:5680/api/status"
API_PREFLIGHT_URL="http://127.0.0.1:5680/api/agent/preflight"

resolve_api_allow_no_key() {
  if [[ "$RUNTIME_PROFILE" == "prod" ]]; then
    if [[ -z "${STEER_API_KEY:-}" ]]; then
      echo "❌ prod profile requires STEER_API_KEY"
      exit 1
    fi
    echo "0"
    return
  fi
  echo "${STEER_API_ALLOW_NO_KEY:-1}"
}

fail_with_logs() {
  echo "❌ Runtime recovery failed."
  if [[ -n "${AGENT_PID:-}" ]]; then
    echo "   - pid: $AGENT_PID"
  fi
  echo "   - log: $LOG_PATH"
  tail -n 120 "$LOG_PATH" 2>/dev/null || true
  exit 1
}

cleanup_stale_locks() {
  if [[ ! -d "$LOCK_DIR" ]]; then
    return
  fi

  local lock_file owner_pid
  for lock_file in "$LOCK_DIR"/steer.*.lock; do
    [[ -f "$lock_file" ]] || continue
    owner_pid="$(sed -n 's/.*"pid"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' "$lock_file" | head -n 1)"
    if [[ -z "$owner_pid" ]] || ! kill -0 "$owner_pid" 2>/dev/null; then
      rm -f "$lock_file" || true
    fi
  done
}

stop_conflicting_processes() {
  local plist old_pid

  for plist in "$PRIMARY_PLIST" "$LEGACY_PLIST"; do
    if [[ -f "$plist" ]]; then
      launchctl unload "$plist" 2>/dev/null || true
    fi
  done

  if [[ -f "$PID_PATH" ]]; then
    old_pid="$(cat "$PID_PATH" 2>/dev/null || true)"
    if [[ -n "$old_pid" ]]; then
      kill "$old_pid" 2>/dev/null || true
    fi
  fi

  pkill -f "[l]ocal_os_agent" 2>/dev/null || true
  pkill -f "/target/.*/core($| )" 2>/dev/null || true
  sleep 2
  pkill -9 -f "[l]ocal_os_agent" 2>/dev/null || true
  pkill -9 -f "/target/.*/core($| )" 2>/dev/null || true

  local port_pids
  port_pids="$(lsof -ti tcp:5680 2>/dev/null || true)"
  if [[ -n "$port_pids" ]]; then
    kill $port_pids 2>/dev/null || true
    sleep 1
    port_pids="$(lsof -ti tcp:5680 2>/dev/null || true)"
    if [[ -n "$port_pids" ]]; then
      kill -9 $port_pids 2>/dev/null || true
    fi
  fi
  sleep 1
}

wait_for_health() {
  local status_code="000"
  for _ in $(seq 1 40); do
    if ! kill -0 "$AGENT_PID" 2>/dev/null; then
      return 1
    fi
    status_code="$(curl -s -o /dev/null -w '%{http_code}' "$API_STATUS_URL" || true)"
    if [[ "$status_code" == "200" ]]; then
      return 0
    fi
    sleep 1
  done
  return 1
}

echo "[1/6] Building debug runtime binary..."
(cd "$CORE_DIR" && cargo build --bin local_os_agent >/dev/null)

echo "[2/6] Stopping conflicting agent processes..."
stop_conflicting_processes

echo "[3/6] Cleaning stale lock files..."
cleanup_stale_locks

echo "[4/6] Starting local agent in background..."
mkdir -p "$LOG_DIR"
: >"$LOG_PATH"
API_ALLOW_NO_KEY="$(resolve_api_allow_no_key)"
if command -v setsid >/dev/null 2>&1; then
  nohup setsid env \
    STEER_API_ALLOW_NO_KEY="$API_ALLOW_NO_KEY" \
    STEER_DISABLE_EVENT_TAP=1 \
    STEER_COLLECTOR_HANDOFF_AUTOCONSUME=0 \
    "$BIN_PATH" >"$LOG_PATH" 2>&1 < /dev/null &
else
  nohup env \
    STEER_API_ALLOW_NO_KEY="$API_ALLOW_NO_KEY" \
    STEER_DISABLE_EVENT_TAP=1 \
    STEER_COLLECTOR_HANDOFF_AUTOCONSUME=0 \
    "$BIN_PATH" >"$LOG_PATH" 2>&1 < /dev/null &
fi
AGENT_PID=$!
echo "$AGENT_PID" >"$PID_PATH"

echo "[5/6] Waiting for API health..."
if ! wait_for_health; then
  fail_with_logs
fi

echo "[6/6] Stability check..."
sleep 8
if ! kill -0 "$AGENT_PID" 2>/dev/null; then
  fail_with_logs
fi

STATUS_CODE="$(curl -s -o /dev/null -w '%{http_code}' "$API_STATUS_URL" || true)"
if [[ "$STATUS_CODE" != "200" ]]; then
  fail_with_logs
fi
PREFLIGHT_CODE="$(curl -s -o /dev/null -w '%{http_code}' "$API_PREFLIGHT_URL" || true)"

echo "Recovery complete."
echo "✅ Agent PID: $AGENT_PID"
echo "✅ /api/status: $STATUS_CODE"
echo "✅ /api/agent/preflight: $PREFLIGHT_CODE"
echo "📄 Log file: $LOG_PATH"
