#!/bin/bash
set -euo pipefail

# Record a full demo run (video + NL execution).
# Usage:
#   scripts/record_nl_demo.sh "자연어 요청" "작업명"
# Optional env:
#   STEER_DEMO_FPS=30
#   STEER_DEMO_DISPLAY_INPUT=auto
#   STEER_DEMO_OPEN_APP=1
#   STEER_DEMO_DIR=scenario_results/demo_videos
#   STEER_PAUSE_ON_USER_INPUT=0

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 \"자연어 요청\" [\"작업명\"]"
  exit 1
fi

REQUEST_TEXT="${1:-}"
TASK_NAME="${2:-시연 실행}"
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DEMO_DIR="${STEER_DEMO_DIR:-$ROOT_DIR/scenario_results/demo_videos}"
FPS="${STEER_DEMO_FPS:-30}"
DISPLAY_INPUT="${STEER_DEMO_DISPLAY_INPUT:-auto}"
OPEN_APP="${STEER_DEMO_OPEN_APP:-1}"

mkdir -p "$DEMO_DIR"
TS="$(date +%Y%m%d_%H%M%S)"
VIDEO_FILE="$DEMO_DIR/demo_${TS}.mp4"
RUN_LOG="$DEMO_DIR/demo_run_${TS}.log"
FFMPEG_LOG="$DEMO_DIR/demo_ffmpeg_${TS}.log"

if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "❌ ffmpeg not found. Install ffmpeg first."
  exit 1
fi

detect_display_input() {
  local probe
  probe="$(ffmpeg -f avfoundation -list_devices true -i "" 2>&1 || true)"
  local idx=""
  idx="$(printf '%s\n' "$probe" | sed -n 's/.*\[\([0-9][0-9]*\)\] Capture screen.*/\1/p' | head -n 1)"
  if [ -n "$idx" ]; then
    echo "${idx}:none"
    return 0
  fi
  # Common fallback on macOS.
  echo "1:none"
  return 0
}

start_ffmpeg_recording() {
  local input_spec="$1"
  local requested_fps="$2"
  local fps_try="$requested_fps"
  local candidates=()
  candidates+=("$fps_try")
  [ "$fps_try" != "30" ] && candidates+=("30")
  [ "$fps_try" != "60" ] && candidates+=("60")

  for fps_try in "${candidates[@]}"; do
    echo "🎥 Try recording input=${input_spec}, fps=${fps_try}"
    ffmpeg -y \
      -f avfoundation \
      -framerate "$fps_try" \
      -i "$input_spec" \
      -vf "scale=1280:-2" \
      -c:v libx264 \
      -preset ultrafast \
      -pix_fmt yuv420p \
      "$VIDEO_FILE" >"$FFMPEG_LOG" 2>&1 &
    FFMPEG_PID=$!
    sleep 2
    if kill -0 "$FFMPEG_PID" 2>/dev/null; then
      echo "✅ Recording started (fps=${fps_try})"
      return 0
    fi
    wait "$FFMPEG_PID" 2>/dev/null || true
    unset FFMPEG_PID
    echo "⚠️ Recording start failed at fps=${fps_try}, retrying..."
  done
  return 1
}

cleanup() {
  local code=$?
  if [ -n "${FFMPEG_PID:-}" ] && kill -0 "$FFMPEG_PID" 2>/dev/null; then
    kill -INT "$FFMPEG_PID" 2>/dev/null || true
    wait "$FFMPEG_PID" 2>/dev/null || true
  fi
  if [ "$code" -ne 0 ]; then
    echo "❌ Demo run failed (exit=$code)."
    echo "   video=$VIDEO_FILE"
    echo "   run_log=$RUN_LOG"
  fi
  exit "$code"
}
trap cleanup EXIT INT TERM

if [ "$OPEN_APP" = "1" ]; then
  open -a "AllvIa" >/dev/null 2>&1 || true
  sleep 1
fi

if [ "$DISPLAY_INPUT" = "auto" ]; then
  DISPLAY_INPUT="$(detect_display_input)"
fi
echo "🎥 Recording start: $VIDEO_FILE (input=${DISPLAY_INPUT}, fps=${FPS})"
if ! start_ffmpeg_recording "$DISPLAY_INPUT" "$FPS"; then
  echo "❌ Failed to start screen recording."
  echo "   ffmpeg_log=$FFMPEG_LOG"
  exit 1
fi

echo "🚀 Running NL demo..."
(
  cd "$ROOT_DIR"
  STEER_OPENAI_MODEL="${STEER_OPENAI_MODEL:-gpt-4o-mini}" \
  STEER_VISION_MODEL="${STEER_VISION_MODEL:-gpt-4o-mini}" \
  STEER_OPENAI_VISION_MAX_B64="${STEER_OPENAI_VISION_MAX_B64:-4000}" \
  STEER_VISION_MAX_TOKENS="${STEER_VISION_MAX_TOKENS:-64}" \
  STEER_VISION_PROMPT_MINIMAL="${STEER_VISION_PROMPT_MINIMAL:-1}" \
  STEER_OPENAI_429_RETRY_SEC="${STEER_OPENAI_429_RETRY_SEC:-6}" \
  STEER_FORCE_DETERMINISTIC_GOAL_AUTOPLAN="${STEER_FORCE_DETERMINISTIC_GOAL_AUTOPLAN:-1}" \
  STEER_PAUSE_ON_USER_INPUT="${STEER_PAUSE_ON_USER_INPUT:-1}" \
  STEER_USER_INPUT_GUARD_MODE="${STEER_USER_INPUT_GUARD_MODE:-all}" \
  STEER_AUTO_DETECT_CLI_LLM="${STEER_AUTO_DETECT_CLI_LLM:-0}" \
  STEER_CLI_LLM="${STEER_CLI_LLM:-}" \
  STEER_NODE_CAPTURE_ALL="${STEER_NODE_CAPTURE_ALL:-0}" \
  STEER_TELEGRAM_EXTRA_IMAGE_MAX="${STEER_TELEGRAM_EXTRA_IMAGE_MAX:-0}" \
  STEER_TELEGRAM_COMPACT_SUCCESS="${STEER_TELEGRAM_COMPACT_SUCCESS:-1}" \
  STEER_TELEGRAM_COMPACT_FAILURE="${STEER_TELEGRAM_COMPACT_FAILURE:-1}" \
  STEER_TELEGRAM_EVIDENCE_MAX_LINES="${STEER_TELEGRAM_EVIDENCE_MAX_LINES:-2}" \
  STEER_TELEGRAM_REPORT_MAX_CHARS="${STEER_TELEGRAM_REPORT_MAX_CHARS:-1600}" \
  bash run_nl_request_with_telegram.sh "$REQUEST_TEXT" "$TASK_NAME"
) | tee "$RUN_LOG"

kill -INT "$FFMPEG_PID" 2>/dev/null || true
wait "$FFMPEG_PID" 2>/dev/null || true
unset FFMPEG_PID

echo "✅ Demo completed."
echo "   video=$VIDEO_FILE"
echo "   run_log=$RUN_LOG"
echo "   ffmpeg_log=$FFMPEG_LOG"
