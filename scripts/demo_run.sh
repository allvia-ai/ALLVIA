#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CORE_STATUS_URL="${STEER_CORE_STATUS_URL:-http://127.0.0.1:5680/api/status}"
CORE_BOOT_TIMEOUT_SEC="${STEER_DEMO_CORE_BOOT_TIMEOUT_SEC:-45}"
START_CORE_IF_DOWN="${STEER_DEMO_START_CORE_IF_DOWN:-1}"
CORE_AUTOSTART_PID_FILE="${STEER_DEMO_CORE_PID_FILE:-/tmp/steer_demo_core.pid}"
CORE_AUTOSTART_LOG="${STEER_DEMO_CORE_LOG_FILE:-$ROOT_DIR/scenario_results/demo_videos/core_autostart.log}"

ensure_steer_app_installed() {
  local app_id
  app_id="$(osascript <<'APPLESCRIPT' 2>/dev/null || true
try
  return id of application "AllvIa"
on error
  return ""
end try
APPLESCRIPT
)"
  if [ -z "$app_id" ]; then
    echo "❌ AllvIa 앱을 찾을 수 없습니다. 앱 설치 후 다시 실행하세요."
    exit 1
  fi
}

core_status_code() {
  curl -sS -o /tmp/steer_demo_core_status.json -w "%{http_code}" "$CORE_STATUS_URL" 2>/dev/null || echo "000"
}

wait_for_core_ready() {
  local timeout_sec="$1"
  local started_at now code
  started_at="$(date +%s)"
  while true; do
    code="$(core_status_code)"
    if [ "$code" = "200" ]; then
      return 0
    fi
    now="$(date +%s)"
    if [ $((now - started_at)) -ge "$timeout_sec" ]; then
      return 1
    fi
    sleep 1
  done
}

start_core_if_needed() {
  if wait_for_core_ready 2; then
    echo "✅ core API ready ($CORE_STATUS_URL)"
    return 0
  fi

  if [ "$START_CORE_IF_DOWN" != "1" ]; then
    echo "❌ core API not ready ($CORE_STATUS_URL)."
    echo "   core를 먼저 실행한 뒤 다시 시도하세요."
    exit 1
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "❌ core API not ready and cargo not found."
    echo "   cargo 설치 또는 core 수동 실행이 필요합니다."
    exit 1
  fi

  mkdir -p "$(dirname "$CORE_AUTOSTART_LOG")"
  if [ -f "$CORE_AUTOSTART_PID_FILE" ]; then
    local prev_pid
    prev_pid="$(cat "$CORE_AUTOSTART_PID_FILE" 2>/dev/null || true)"
    if [[ "$prev_pid" =~ ^[0-9]+$ ]] && kill -0 "$prev_pid" 2>/dev/null; then
      echo "ℹ️ core autostart pid already running: $prev_pid"
    else
      rm -f "$CORE_AUTOSTART_PID_FILE" 2>/dev/null || true
    fi
  fi

  if [ ! -f "$CORE_AUTOSTART_PID_FILE" ]; then
    echo "🚀 core API down → starting core backend..."
    (
      cd "$ROOT_DIR"
      nohup cargo run --manifest-path core/Cargo.toml --bin core >>"$CORE_AUTOSTART_LOG" 2>&1 &
      echo $! >"$CORE_AUTOSTART_PID_FILE"
    )
  fi

  if ! wait_for_core_ready "$CORE_BOOT_TIMEOUT_SEC"; then
    echo "❌ core API boot timeout (${CORE_BOOT_TIMEOUT_SEC}s): $CORE_STATUS_URL"
    echo "   log: $CORE_AUTOSTART_LOG"
    exit 1
  fi
  echo "✅ core API ready after autostart ($CORE_STATUS_URL)"
}

usage() {
  cat <<'EOF'
Usage:
  scripts/demo_run.sh --preset {news_telegram|news_email|calendar_telegram}
  scripts/demo_run.sh --prompt "자연어 요청문"

Optional env:
  STEER_DEMO_SKIP_PREP=1            # skip demo_prep.sh
  STEER_DEMO_SKIP_STATE_RESET=1     # skip demo_state_reset.sh
  STEER_DEMO_FPS=30
  STEER_DEMO_DISPLAY_INPUT=auto
EOF
}

MODE=""
PRESET=""
PROMPT=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --preset)
      MODE="preset"
      PRESET="${2:-}"
      shift 2
      ;;
    --prompt)
      MODE="prompt"
      PROMPT="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown arg: $1"
      usage
      exit 1
      ;;
  esac
done

if [ -z "$MODE" ]; then
  usage
  exit 1
fi

ensure_steer_app_installed
start_core_if_needed

# Demo-safe defaults: intentionally pinned to avoid env drift during live demos.
export STEER_OPENAI_MODEL="gpt-4o-mini"
export STEER_VISION_MODEL="gpt-4o-mini"
export STEER_OPENAI_VISION_MAX_B64="4000"
export STEER_VISION_MAX_TOKENS="64"
export STEER_VISION_PROMPT_MINIMAL="1"
export STEER_OPENAI_429_RETRY_SEC="6"

export STEER_UI_FALLBACK_RUN="0"
export STEER_UI_SUBMIT_METHOD="both"
export STEER_UI_RETRY_SUBMIT_METHOD="auto"
export STEER_UI_SET_VALUE_MODE="ax"
export STEER_UI_AX_PASTE_FALLBACK="1"
export STEER_UI_REQUIRE_INPUT_MATCH="1"
export STEER_UI_REQUIRE_RUN_DETECTION="1"
export STEER_UI_ENABLE_TYPE_FALLBACK="0"
export STEER_UI_SUBMIT_RETRIES="3"
export STEER_UI_RETRY_INTERVAL_SEC="4"
export STEER_UI_DETECT_WINDOW_SEC="12"
export STEER_UI_RUN_TIMEOUT_SEC="180"
export STEER_UI_MAX_RUN_IDLE_SEC="25"
export STEER_UI_ALLOW_INPUT_UNAVAILABLE="0"
export STEER_UI_MATCH_TASK_RUN_PROMPT="1"
export STEER_UI_PREP_STATE_RESET="1"
export STEER_UI_REQUIRE_SUCCESS_STATUS="1"

export STEER_PAUSE_ON_USER_INPUT="1"
export STEER_USER_INPUT_GUARD_MODE="all"

export STEER_NODE_CAPTURE_ALL="0"
export STEER_TELEGRAM_EXTRA_IMAGE_MAX="0"
export STEER_TELEGRAM_COMPACT_SUCCESS="1"
export STEER_TELEGRAM_COMPACT_FAILURE="1"
export STEER_TELEGRAM_SUPER_COMPACT="1"
export STEER_TELEGRAM_EVIDENCE_MAX_LINES="2"
export STEER_TELEGRAM_REPORT_MAX_CHARS="1600"

if [ "${STEER_DEMO_SKIP_PREP:-0}" != "1" ]; then
  echo "== demo_prep =="
  "$ROOT_DIR/scripts/demo_prep.sh"
fi

if [ "${STEER_DEMO_SKIP_STATE_RESET:-0}" != "1" ]; then
  echo "== demo_state_reset =="
  "$ROOT_DIR/scripts/demo_state_reset.sh"
fi

open -a "AllvIa" >/dev/null 2>&1 || true

echo "== demo_record =="
if [ "$MODE" = "preset" ]; then
  if [ -z "$PRESET" ]; then
    echo "Missing --preset value"
    exit 1
  fi
  exec "$ROOT_DIR/scripts/record_demo_preset.sh" "$PRESET"
else
  if [ -z "$PROMPT" ]; then
    echo "Missing --prompt value"
    exit 1
  fi
  exec "$ROOT_DIR/scripts/record_ui_nl_demo.sh" "$PROMPT"
fi
