#!/bin/bash
set -euo pipefail

# Preset demo recorder.
# Usage:
#   scripts/record_demo_preset.sh news_telegram
#   scripts/record_demo_preset.sh news_email
#   scripts/record_demo_preset.sh calendar_telegram
#
# Optional env:
#   STEER_DEMO_USE_UI=1  -> use UI-input recorder (default)
#   STEER_DEMO_USE_UI=0  -> use engine-run recorder

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 {news_telegram|news_email|calendar_telegram}"
  exit 1
fi

PRESET="$1"
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
USE_UI="${STEER_DEMO_USE_UI:-1}"
if [ "$USE_UI" = "1" ]; then
  RECORDER="$ROOT_DIR/scripts/record_ui_nl_demo.sh"
else
  RECORDER="$ROOT_DIR/scripts/record_nl_demo.sh"
fi

if [ ! -x "$RECORDER" ]; then
  echo "Recorder not executable: $RECORDER"
  exit 1
fi

case "$PRESET" in
  news_telegram)
    PROMPT="오늘 AI 뉴스 중 가장 중요한 5가지를 찾아서 핵심만 쉬운 한국어로 요약하고 텔레그램으로 보내줘."
    TASK="AI 뉴스 5개 텔레그램 요약"
    ;;
  news_email)
    PROMPT="오늘 AI 뉴스 중 가장 중요한 5가지를 찾아서 핵심만 쉬운 한국어로 요약하고 메일로 보내줘."
    TASK="AI 뉴스 5개 이메일 요약"
    ;;
  calendar_telegram)
    PROMPT="캘린더를 열어 오늘 할 일을 확인하고 핵심만 5줄로 정리해서 텔레그램으로 보내줘."
    TASK="오늘 일정 텔레그램 요약"
    ;;
  *)
    echo "Unknown preset: $PRESET"
    echo "Available presets: news_telegram, news_email, calendar_telegram"
    exit 1
    ;;
esac

if [ "$USE_UI" = "1" ]; then
  echo "🎬 UI preset demo mode (Steer launcher input path)"
  export STEER_UI_FALLBACK_RUN="${STEER_UI_FALLBACK_RUN:-0}"
  export STEER_UI_SUBMIT_METHOD="${STEER_UI_SUBMIT_METHOD:-both}"
  export STEER_UI_RETRY_SUBMIT_METHOD="${STEER_UI_RETRY_SUBMIT_METHOD:-auto}"
  export STEER_UI_SET_VALUE_MODE="${STEER_UI_SET_VALUE_MODE:-ax}"
  export STEER_UI_REQUIRE_RUN_DETECTION="${STEER_UI_REQUIRE_RUN_DETECTION:-1}"
  export STEER_UI_REQUIRE_SUCCESS_STATUS="${STEER_UI_REQUIRE_SUCCESS_STATUS:-1}"
  export STEER_UI_REQUIRE_INPUT_MATCH="${STEER_UI_REQUIRE_INPUT_MATCH:-1}"
  export STEER_UI_INPUT_VERIFY_RETRIES="${STEER_UI_INPUT_VERIFY_RETRIES:-4}"
  export STEER_UI_ENABLE_TYPE_FALLBACK="${STEER_UI_ENABLE_TYPE_FALLBACK:-0}"
  export STEER_UI_ALLOW_INPUT_UNAVAILABLE="${STEER_UI_ALLOW_INPUT_UNAVAILABLE:-0}"
  export STEER_UI_SUBMIT_RETRIES="${STEER_UI_SUBMIT_RETRIES:-3}"
  export STEER_UI_RETRY_INTERVAL_SEC="${STEER_UI_RETRY_INTERVAL_SEC:-4}"
  export STEER_UI_DETECT_WINDOW_SEC="${STEER_UI_DETECT_WINDOW_SEC:-12}"
  export STEER_UI_RUN_TIMEOUT_SEC="${STEER_UI_RUN_TIMEOUT_SEC:-180}"
  export STEER_UI_MAX_RUN_IDLE_SEC="${STEER_UI_MAX_RUN_IDLE_SEC:-25}"
  export STEER_NODE_CAPTURE_ALL="${STEER_NODE_CAPTURE_ALL:-0}"
  export STEER_TELEGRAM_EXTRA_IMAGE_MAX="${STEER_TELEGRAM_EXTRA_IMAGE_MAX:-0}"
  export STEER_TELEGRAM_COMPACT_SUCCESS="${STEER_TELEGRAM_COMPACT_SUCCESS:-1}"
  export STEER_TELEGRAM_COMPACT_FAILURE="${STEER_TELEGRAM_COMPACT_FAILURE:-1}"
  export STEER_TELEGRAM_SUPER_COMPACT="${STEER_TELEGRAM_SUPER_COMPACT:-1}"
  export STEER_TELEGRAM_EVIDENCE_MAX_LINES="${STEER_TELEGRAM_EVIDENCE_MAX_LINES:-2}"
  export STEER_TELEGRAM_REPORT_MAX_CHARS="${STEER_TELEGRAM_REPORT_MAX_CHARS:-1600}"
  exec "$RECORDER" "$PROMPT"
else
  echo "🎬 Engine preset demo mode (script path)"
  export STEER_NODE_CAPTURE_ALL="${STEER_NODE_CAPTURE_ALL:-0}"
  export STEER_TELEGRAM_EXTRA_IMAGE_MAX="${STEER_TELEGRAM_EXTRA_IMAGE_MAX:-0}"
  export STEER_TELEGRAM_COMPACT_SUCCESS="${STEER_TELEGRAM_COMPACT_SUCCESS:-1}"
  export STEER_TELEGRAM_COMPACT_FAILURE="${STEER_TELEGRAM_COMPACT_FAILURE:-1}"
  export STEER_TELEGRAM_SUPER_COMPACT="${STEER_TELEGRAM_SUPER_COMPACT:-1}"
  export STEER_TELEGRAM_EVIDENCE_MAX_LINES="${STEER_TELEGRAM_EVIDENCE_MAX_LINES:-2}"
  export STEER_TELEGRAM_REPORT_MAX_CHARS="${STEER_TELEGRAM_REPORT_MAX_CHARS:-1600}"
  exec "$RECORDER" "$PROMPT" "$TASK"
fi
