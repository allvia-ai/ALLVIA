#!/bin/bash
set -euo pipefail

# Background runner for run_nl_request_with_telegram.sh
# Usage:
#   scripts/run_nl_bg.sh "요청문" "작업명"

REQUEST_TEXT="${1:-}"
TASK_NAME="${2:-백그라운드 자연어 실행}"

if [ -z "$REQUEST_TEXT" ]; then
  echo "Usage: $0 \"요청문\" [\"작업명\"]"
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RUN_DIR="$ROOT_DIR/scenario_results/bg_runs"
mkdir -p "$RUN_DIR"

TS="$(date +%Y%m%d_%H%M%S)"
RUN_ID="bg_${TS}"
PID_FILE="$RUN_DIR/${RUN_ID}.pid"
LOG_FILE="$RUN_DIR/${RUN_ID}.driver.log"
META_FILE="$RUN_DIR/${RUN_ID}.meta"
LATEST_FILE="$RUN_DIR/latest_run_id"

{
  echo "run_id=${RUN_ID}"
  echo "request=${REQUEST_TEXT}"
  echo "task=${TASK_NAME}"
  echo "created_at=$(date '+%F %T')"
} > "$META_FILE"

# Fully detach so callers (panel/HTTP) return immediately.
nohup env \
  STEER_PAUSE_ON_USER_INPUT="${STEER_PAUSE_ON_USER_INPUT:-1}" \
  STEER_USER_ACTIVE_APPS="${STEER_USER_ACTIVE_APPS:-Terminal,Codex,iTerm2}" \
  STEER_INPUT_ACTIVE_THRESHOLD_SECONDS="${STEER_INPUT_ACTIVE_THRESHOLD_SECONDS:-1}" \
  STEER_IDLE_RESUME_SECONDS="${STEER_IDLE_RESUME_SECONDS:-3}" \
  STEER_INPUT_POLL_SECONDS="${STEER_INPUT_POLL_SECONDS:-1}" \
  STEER_REQUIRE_TERMINAL="${STEER_REQUIRE_TERMINAL:-0}" \
  STEER_NODE_CAPTURE_ALL="${STEER_NODE_CAPTURE_ALL:-1}" \
  bash -lc "cd \"$ROOT_DIR\" && bash run_nl_request_with_telegram.sh \"\$1\" \"\$2\"" _ \
  "$REQUEST_TEXT" "$TASK_NAME" > "$LOG_FILE" 2>&1 < /dev/null &

PID=$!
disown "$PID" 2>/dev/null || true
echo "$PID" > "$PID_FILE"
echo "$RUN_ID" > "$LATEST_FILE"

echo "started_run_id=$RUN_ID"
echo "pid=$PID"
echo "driver_log=$LOG_FILE"
echo "pid_file=$PID_FILE"
