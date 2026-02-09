#!/bin/bash
set -euo pipefail

# Control script for background NL runs.
# Usage:
#   scripts/run_nl_bg_ctl.sh status [run_id]
#   scripts/run_nl_bg_ctl.sh pause [run_id]
#   scripts/run_nl_bg_ctl.sh resume [run_id]
#   scripts/run_nl_bg_ctl.sh stop [run_id]
#   scripts/run_nl_bg_ctl.sh tail [run_id]

ACTION="${1:-status}"
RUN_ID="${2:-}"

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
RUN_DIR="$ROOT_DIR/scenario_results/bg_runs"
LATEST_FILE="$RUN_DIR/latest_run_id"

if [ -z "$RUN_ID" ] && [ -f "$LATEST_FILE" ]; then
  RUN_ID="$(cat "$LATEST_FILE")"
fi

if [ -z "$RUN_ID" ]; then
  echo "No run_id provided and no latest run found."
  exit 1
fi

PID_FILE="$RUN_DIR/${RUN_ID}.pid"
META_FILE="$RUN_DIR/${RUN_ID}.meta"
LOG_FILE="$RUN_DIR/${RUN_ID}.driver.log"

if [ ! -f "$PID_FILE" ]; then
  echo "PID file not found: $PID_FILE"
  exit 1
fi

PID="$(cat "$PID_FILE")"

is_alive() {
  kill -0 "$PID" 2>/dev/null
}

case "$ACTION" in
  status)
    echo "run_id=$RUN_ID"
    echo "pid=$PID"
    [ -f "$META_FILE" ] && cat "$META_FILE"
    if is_alive; then
      # Also print process state when possible.
      STATE="$(ps -o state= -p "$PID" 2>/dev/null | tr -d ' ' || true)"
      echo "alive=1"
      echo "state=${STATE:-unknown}"
    else
      echo "alive=0"
      echo "state=exited"
    fi
    echo "driver_log=$LOG_FILE"
    ;;
  pause)
    if is_alive; then
      kill -STOP "$PID"
      pkill -STOP -P "$PID" >/dev/null 2>&1 || true
      echo "paused run_id=$RUN_ID pid=$PID"
    else
      echo "process not alive"
      exit 1
    fi
    ;;
  resume)
    if is_alive; then
      kill -CONT "$PID"
      pkill -CONT -P "$PID" >/dev/null 2>&1 || true
      echo "resumed run_id=$RUN_ID pid=$PID"
    else
      echo "process not alive"
      exit 1
    fi
    ;;
  stop)
    if is_alive; then
      kill "$PID" >/dev/null 2>&1 || true
      pkill -P "$PID" >/dev/null 2>&1 || true
      sleep 1
      if is_alive; then
        kill -9 "$PID" >/dev/null 2>&1 || true
      fi
      echo "stopped run_id=$RUN_ID pid=$PID"
    else
      echo "process already exited"
    fi
    ;;
  tail)
    if [ -f "$LOG_FILE" ]; then
      tail -n 80 "$LOG_FILE"
    else
      echo "driver log missing: $LOG_FILE"
      exit 1
    fi
    ;;
  *)
    echo "Unknown action: $ACTION"
    echo "Usage: $0 {status|pause|resume|stop|tail} [run_id]"
    exit 1
    ;;
esac
