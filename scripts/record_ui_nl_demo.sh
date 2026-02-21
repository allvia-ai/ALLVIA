#!/bin/bash
set -euo pipefail

# Record a demo that starts from the Steer OS app UI input box.
# Usage:
#   scripts/record_ui_nl_demo.sh "자연어 요청"
#
# Optional env:
#   STEER_DEMO_DIR=scenario_results/demo_videos
#   STEER_DEMO_FPS=30
#   STEER_DEMO_DISPLAY_INPUT=auto
#   STEER_UI_RUN_TIMEOUT_SEC=180
#   STEER_UI_SUBMIT_METHOD=button|enter|both
#   STEER_UI_RETRY_SUBMIT_METHOD=auto|enter|button|both
#   STEER_UI_ENABLE_TYPE_FALLBACK=0|1
#   STEER_UI_ALLOW_INPUT_UNAVAILABLE=0|1
#   STEER_UI_MAX_RUN_IDLE_SEC=25
#   STEER_UI_DETECT_WINDOW_SEC=12

if [ "$#" -lt 1 ]; then
  echo "Usage: $0 \"자연어 요청\""
  exit 1
fi

REQUEST_TEXT="$1"
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DEMO_DIR="${STEER_DEMO_DIR:-$ROOT_DIR/scenario_results/demo_videos}"
FPS="${STEER_DEMO_FPS:-30}"
DISPLAY_INPUT="${STEER_DEMO_DISPLAY_INPUT:-auto}"
RUN_TIMEOUT_SEC="${STEER_UI_RUN_TIMEOUT_SEC:-180}"
FALLBACK_RUN="${STEER_UI_FALLBACK_RUN:-0}"
CLICLICK_BIN="$(command -v cliclick || true)"
CORE_API_BASE="${STEER_CORE_API_BASE:-http://127.0.0.1:5680/api}"
TASK_RUNS_URL="${CORE_API_BASE%/}/agent/task-runs?limit=20"
TASK_RUN_URL_BASE="${CORE_API_BASE%/}/agent/task-runs"
CORE_STATUS_URL="${STEER_CORE_STATUS_URL:-http://127.0.0.1:5680/api/status}"
PRIMARY_SUBMIT_METHOD="${STEER_UI_SUBMIT_METHOD:-both}"
RETRY_SUBMIT_METHOD="${STEER_UI_RETRY_SUBMIT_METHOD:-auto}"
PREP_STATE_RESET="${STEER_UI_PREP_STATE_RESET:-1}"
DETECT_WINDOW_SEC="${STEER_UI_DETECT_WINDOW_SEC:-12}"
MAX_SUBMIT_RETRIES="${STEER_UI_SUBMIT_RETRIES:-3}"
RETRY_INTERVAL_SEC="${STEER_UI_RETRY_INTERVAL_SEC:-4}"
SET_VALUE_MODE="${STEER_UI_SET_VALUE_MODE:-ax}"          # hybrid|ax|paste|type
INPUT_VERIFY_RETRIES="${STEER_UI_INPUT_VERIFY_RETRIES:-3}"
REQUIRE_RUN_DETECTION="${STEER_UI_REQUIRE_RUN_DETECTION:-1}"
TYPE_FALLBACK_NONASCII="${STEER_UI_TYPE_FALLBACK_NONASCII:-0}"
MATCH_TASK_RUN_PROMPT="${STEER_UI_MATCH_TASK_RUN_PROMPT:-1}"
ENABLE_TYPE_FALLBACK="${STEER_UI_ENABLE_TYPE_FALLBACK:-0}"
REQUIRE_INPUT_MATCH="${STEER_UI_REQUIRE_INPUT_MATCH:-1}"
ALLOW_INPUT_UNAVAILABLE="${STEER_UI_ALLOW_INPUT_UNAVAILABLE:-0}"
MAX_RUN_IDLE_SEC="${STEER_UI_MAX_RUN_IDLE_SEC:-25}"
REQUIRE_SUCCESS_STATUS="${STEER_UI_REQUIRE_SUCCESS_STATUS:-1}"

PRIMARY_SUBMIT_METHOD="$(printf '%s' "$PRIMARY_SUBMIT_METHOD" | tr '[:upper:]' '[:lower:]')"
RETRY_SUBMIT_METHOD="$(printf '%s' "$RETRY_SUBMIT_METHOD" | tr '[:upper:]' '[:lower:]')"
SET_VALUE_MODE="$(printf '%s' "$SET_VALUE_MODE" | tr '[:upper:]' '[:lower:]')"
if ! [[ "$DETECT_WINDOW_SEC" =~ ^[0-9]+$ ]]; then
  DETECT_WINDOW_SEC=12
fi
if ! [[ "$MAX_SUBMIT_RETRIES" =~ ^[0-9]+$ ]]; then
  MAX_SUBMIT_RETRIES=2
fi
if [ "$MAX_SUBMIT_RETRIES" -lt 1 ]; then
  MAX_SUBMIT_RETRIES=1
fi
if ! [[ "$RETRY_INTERVAL_SEC" =~ ^[0-9]+$ ]]; then
  RETRY_INTERVAL_SEC=4
fi
if [ "$RETRY_INTERVAL_SEC" -lt 3 ]; then
  RETRY_INTERVAL_SEC=3
fi
if ! [[ "$MAX_RUN_IDLE_SEC" =~ ^[0-9]+$ ]]; then
  MAX_RUN_IDLE_SEC=25
fi
if [ "$MAX_RUN_IDLE_SEC" -lt 15 ]; then
  MAX_RUN_IDLE_SEC=15
fi
if ! [[ "$INPUT_VERIFY_RETRIES" =~ ^[0-9]+$ ]]; then
  INPUT_VERIFY_RETRIES=3
fi
if [ "$INPUT_VERIFY_RETRIES" -lt 1 ]; then
  INPUT_VERIFY_RETRIES=1
fi

wait_for_core_api() {
  local timeout_sec="${1:-20}"
  local start now code
  start="$(date +%s)"
  while true; do
    code="$(curl -sS -o /tmp/steer_ui_demo_core_status.json -w "%{http_code}" "$CORE_STATUS_URL" 2>/dev/null || echo "000")"
    if [ "$code" = "200" ]; then
      return 0
    fi
    now="$(date +%s)"
    if [ $((now - start)) -ge "$timeout_sec" ]; then
      return 1
    fi
    sleep 1
  done
}

mkdir -p "$DEMO_DIR"
TS="$(date +%Y%m%d_%H%M%S)"
VIDEO_FILE="$DEMO_DIR/ui_demo_${TS}.mp4"
RUN_LOG_CAPTURE="$DEMO_DIR/ui_demo_capture_${TS}.log"
RUN_STATUS_CAPTURE="$DEMO_DIR/ui_demo_run_${TS}.json"
FFMPEG_LOG="$DEMO_DIR/ui_demo_ffmpeg_${TS}.log"
LOCK_FILE="${STEER_UI_DEMO_LOCK_FILE:-/tmp/steer_ui_nl_demo.lock}"

if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "❌ ffmpeg not found."
  exit 1
fi

acquire_demo_lock() {
  if [ -f "$LOCK_FILE" ]; then
    local existing_pid
    existing_pid="$(cat "$LOCK_FILE" 2>/dev/null || true)"
    if [[ "$existing_pid" =~ ^[0-9]+$ ]] && kill -0 "$existing_pid" 2>/dev/null; then
      echo "❌ UI demo recorder already running (pid=$existing_pid, lock=$LOCK_FILE)."
      exit 2
    fi
    rm -f "$LOCK_FILE" 2>/dev/null || true
  fi
  echo "$$" >"$LOCK_FILE"
}

detect_display_input() {
  local probe
  probe="$(ffmpeg -f avfoundation -list_devices true -i "" 2>&1 || true)"
  local idx=""
  idx="$(printf '%s\n' "$probe" | sed -n 's/.*\[\([0-9][0-9]*\)\] Capture screen.*/\1/p' | head -n 1)"
  if [ -n "$idx" ]; then
    echo "${idx}:none"
    return 0
  fi
  echo "1:none"
  return 0
}

start_recording() {
  local input_spec="$1"
  local fps="$2"
  ffmpeg -y \
    -f avfoundation \
    -framerate "$fps" \
    -i "$input_spec" \
    -vf "scale=1280:-2" \
    -c:v libx264 \
    -preset ultrafast \
    -pix_fmt yuv420p \
    "$VIDEO_FILE" >"$FFMPEG_LOG" 2>&1 &
  FFMPEG_PID=$!
  sleep 2
  if ! kill -0 "$FFMPEG_PID" 2>/dev/null; then
    wait "$FFMPEG_PID" 2>/dev/null || true
    unset FFMPEG_PID
    return 1
  fi
  return 0
}

cleanup() {
  local code=$?
  if [ -n "${FFMPEG_PID:-}" ] && kill -0 "$FFMPEG_PID" 2>/dev/null; then
    kill -INT "$FFMPEG_PID" 2>/dev/null || true
    wait "$FFMPEG_PID" 2>/dev/null || true
  fi
  if [ -f "$LOCK_FILE" ]; then
    local lock_pid
    lock_pid="$(cat "$LOCK_FILE" 2>/dev/null || true)"
    if [ -z "$lock_pid" ] || [ "$lock_pid" = "$$" ]; then
      rm -f "$LOCK_FILE" 2>/dev/null || true
    fi
  fi
  [ -n "${BASELINE_LIST:-}" ] && rm -f "$BASELINE_LIST" 2>/dev/null || true
  [ -n "${BASELINE_RUN_IDS:-}" ] && rm -f "$BASELINE_RUN_IDS" 2>/dev/null || true
  exit "$code"
}
trap cleanup EXIT INT TERM

acquire_demo_lock

if ! wait_for_core_api 20; then
  echo "❌ core API not ready: $CORE_STATUS_URL"
  echo "   UI 시연을 시작하지 않습니다. (정지 화면 녹화 방지)"
  exit 2
fi

if [ "$DISPLAY_INPUT" = "auto" ]; then
  DISPLAY_INPUT="$(detect_display_input)"
fi

SCENARIO_DIR="$ROOT_DIR/scenario_results"
mkdir -p "$SCENARIO_DIR"
BASELINE_LIST="$(mktemp)"
ls -1 "$SCENARIO_DIR"/nl_request_*.log 2>/dev/null | sort >"$BASELINE_LIST" || true
BASELINE_RUN_IDS="$(mktemp)"

if command -v python3 >/dev/null 2>&1; then
  TASK_RUNS_SNAPSHOT="$(mktemp)"
  if curl -fsS --max-time 5 "$TASK_RUNS_URL" >"$TASK_RUNS_SNAPSHOT" 2>/dev/null; then
    python3 - "$TASK_RUNS_SNAPSHOT" >"$BASELINE_RUN_IDS" <<'PY'
import json, sys
path = sys.argv[1]
try:
    data = json.load(open(path, "r", encoding="utf-8"))
except Exception:
    data = []
if isinstance(data, list):
    for item in data:
        rid = item.get("run_id") if isinstance(item, dict) else None
        if isinstance(rid, str) and rid.strip():
            print(rid.strip())
PY
  else
    : >"$BASELINE_RUN_IDS"
  fi
  rm -f "$TASK_RUNS_SNAPSHOT"
else
  : >"$BASELINE_RUN_IDS"
fi

if [ "$PREP_STATE_RESET" = "1" ] && [ -x "$ROOT_DIR/scripts/demo_state_reset.sh" ]; then
  echo "🧹 Applying demo state reset..."
  "$ROOT_DIR/scripts/demo_state_reset.sh" >/dev/null 2>&1 || true
fi

echo "🎥 UI demo recording start: $VIDEO_FILE (input=${DISPLAY_INPUT}, fps=${FPS})"
if ! start_recording "$DISPLAY_INPUT" "$FPS"; then
  echo "❌ Failed to start recording. See $FFMPEG_LOG"
  exit 1
fi

echo "🚀 Opening Steer OS and submitting prompt from UI..."
open -a "Steer OS" >/dev/null 2>&1 || true
sleep 0.6
osascript <<'APPLESCRIPT' >/dev/null 2>&1
tell application "Steer OS" to activate
delay 1.0
APPLESCRIPT

STEER_PROCESS_READY="$(osascript <<'APPLESCRIPT' 2>/dev/null || true
tell application "System Events"
  if exists process "Steer OS" then return "1"
end tell
return "0"
APPLESCRIPT
)"
if [ "$STEER_PROCESS_READY" != "1" ]; then
  echo "❌ Steer OS process not found. 앱이 실행되지 않아 시연을 시작할 수 없습니다."
  exit 1
fi

ensure_nl_mode() {
  osascript <<'APPLESCRIPT' 2>/dev/null || true
tell application "System Events"
  if not (exists process "Steer OS") then return "NO_PROCESS"
  tell process "Steer OS"
    set frontmost to true
    try
      repeat with w in windows
        try
          if exists button "자연어" of w then
            click button "자연어" of w
            return "OK"
          end if
        end try
      end repeat
    on error
      return "ERR"
    end try
  end tell
end tell
return "NOT_FOUND"
APPLESCRIPT
}
NL_MODE_STATE="$(ensure_nl_mode)"
if [ "$NL_MODE_STATE" = "NO_PROCESS" ] || [ "$NL_MODE_STATE" = "NOT_FOUND" ]; then
  echo "❌ 자연어 탭을 찾지 못했습니다(NL mode state=${NL_MODE_STATE})."
  exit 2
fi
sleep 0.2

wait_for_launcher_input_ready() {
  local timeout_sec="${1:-20}"
  local start
  local ready
  start="$(date +%s)"
  while true; do
    ready="$(osascript <<'APPLESCRIPT' 2>/dev/null || true
tell application "System Events"
  if not (exists process "Steer OS") then return "0"
  tell process "Steer OS"
    try
      repeat with w in windows
        try
          if exists text field 1 of w then return "1"
        end try
      end repeat
    end try
  end tell
end tell
return "0"
APPLESCRIPT
)"
    if [ "$ready" = "1" ]; then
      return 0
    fi
    if [ $(( $(date +%s) - start )) -ge "$timeout_sec" ]; then
      return 1
    fi
    sleep 0.4
  done
}

if ! wait_for_launcher_input_ready 20; then
  echo "❌ Steer OS 런처 입력창을 찾지 못했습니다(타임아웃)."
  exit 2
fi

# Read Steer OS launcher window geometry.
WINDOW_GEOMETRY="$(osascript <<'APPLESCRIPT'
tell application "System Events"
  if not (exists process "Steer OS") then return ""
  tell process "Steer OS"
    set frontmost to true
    try
      set p to position of window 1
      set s to size of window 1
      set px to item 1 of p
      set py to item 2 of p
      set sw to item 1 of s
      set sh to item 2 of s
      return (px as text) & "|" & (py as text) & "|" & (sw as text) & "|" & (sh as text)
    on error
      return ""
    end try
  end tell
end tell
APPLESCRIPT
)"

INPUT_POINT=""
SEND_POINT=""
if [ -n "$WINDOW_GEOMETRY" ]; then
  IFS='|' read -r WIN_X WIN_Y WIN_W WIN_H <<<"$WINDOW_GEOMETRY"
  if [ -n "${WIN_X:-}" ] && [ -n "${WIN_Y:-}" ] && [ -n "${WIN_W:-}" ] && [ -n "${WIN_H:-}" ]; then
    INPUT_X=$((WIN_X + (WIN_W / 2)))
    INPUT_Y=$((WIN_Y + 70))
    INPUT_POINT="${INPUT_X},${INPUT_Y}"
    SEND_X=$((WIN_X + WIN_W - 38))
    SEND_Y=$((WIN_Y + 70))
    SEND_POINT="${SEND_X},${SEND_Y}"
  fi
fi

normalize_for_compare() {
  printf '%s' "${1:-}" \
    | tr -d '\r' \
    | sed -E 's/[[:space:]]+/ /g; s/^ //; s/ $//'
}

read_ui_input_value() {
  osascript <<'APPLESCRIPT' 2>/dev/null || true
tell application "System Events"
  if not (exists process "Steer OS") then return "__UNAVAILABLE__"
  tell process "Steer OS"
    set frontmost to true
    try
      repeat with w in windows
        try
          if exists text field 1 of w then
            return (value of text field 1 of w) as text
          end if
        end try
      end repeat
    on error
      return "__UNAVAILABLE__"
    end try
  end tell
end tell
return "__UNAVAILABLE__"
APPLESCRIPT
}

set_prompt_ax() {
  osascript - "$REQUEST_TEXT" <<'APPLESCRIPT' >/dev/null 2>&1 || true
on run argv
  set promptText to item 1 of argv
  tell application "System Events"
    if not (exists process "Steer OS") then return "NO_PROCESS"
    tell process "Steer OS"
      set frontmost to true
      repeat with w in windows
        try
          if exists text field 1 of w then
            set value of text field 1 of w to promptText
            return "OK"
          end if
        end try
      end repeat
    end tell
  end tell
  return "NO_TEXT_FIELD"
end run
APPLESCRIPT
}

set_prompt_paste() {
  osascript - "$REQUEST_TEXT" <<'APPLESCRIPT' >/dev/null 2>&1 || true
on run argv
  set promptText to item 1 of argv
  set the clipboard to promptText
  delay 0.2
  tell application "System Events"
    keystroke "a" using command down
    delay 0.08
    key code 51
    delay 0.08
    keystroke "v" using command down
  end tell
end run
APPLESCRIPT
}

focus_input_point() {
  if [ -n "$INPUT_POINT" ] && [ -n "$CLICLICK_BIN" ]; then
    "$CLICLICK_BIN" "c:${INPUT_POINT}" >/dev/null 2>&1 || true
    sleep 0.2
  fi
}

clear_prompt_input() {
  osascript <<'APPLESCRIPT' >/dev/null 2>&1 || true
tell application "System Events"
  keystroke "a" using command down
  delay 0.06
  key code 51
end tell
APPLESCRIPT
}

set_prompt_type() {
  osascript - "$REQUEST_TEXT" <<'APPLESCRIPT' >/dev/null 2>&1 || true
on run argv
  set promptText to item 1 of argv
  tell application "System Events"
    keystroke promptText
  end tell
end run
APPLESCRIPT
}

is_ascii_prompt() {
  # True when request text is plain ASCII; false for CJK/emoji/other composed input.
  printf '%s' "$REQUEST_TEXT" | LC_ALL=C grep -q '^[ -~]*$'
}

try_apply_method_once() {
  local method="$1"
  clear_prompt_input
  case "$method" in
    ax)
      set_prompt_ax
      ;;
    paste)
      set_prompt_paste
      ;;
    type)
      set_prompt_type
      ;;
    *)
      return 1
      ;;
  esac
  sleep 0.22
  return 0
}

apply_prompt_with_verify() {
  local tries="${INPUT_VERIFY_RETRIES}"
  local attempt=1
  local want got want_norm got_norm
  local methods=()
  local method=""
  local allow_type=0
  want="$REQUEST_TEXT"
  want_norm="$(normalize_for_compare "$want")"

  if [ "$ENABLE_TYPE_FALLBACK" = "1" ]; then
    allow_type=1
    if [ "$TYPE_FALLBACK_NONASCII" != "1" ] && ! is_ascii_prompt; then
      allow_type=0
    fi
  fi

  case "$SET_VALUE_MODE" in
    ax)
      methods=("ax")
      ;;
    paste)
      methods=("paste")
      ;;
    type)
      methods=("type")
      ;;
    hybrid|*)
      methods=("ax" "paste")
      if [ "$allow_type" = "1" ]; then
        methods+=("type")
      fi
      ;;
  esac

  while [ "$attempt" -le "$tries" ]; do
    focus_input_point
    for method in "${methods[@]}"; do
      try_apply_method_once "$method"
      got="$(read_ui_input_value)"
      if [ "$got" = "__UNAVAILABLE__" ]; then
        # Some builds expose no AX text field. In demo mode keep strict by default.
        if [ "$ALLOW_INPUT_UNAVAILABLE" = "1" ]; then
          return 0
        fi
        continue
      fi
      got_norm="$(normalize_for_compare "$got")"
      if [ "$got_norm" = "$want_norm" ]; then
        return 0
      fi
    done
    attempt=$((attempt + 1))
  done
  return 1
}

submit_prompt_from_ui() {
  local method="${1:-enter}"
  if ! apply_prompt_with_verify; then
    echo "⚠️ 입력값 검증 실패: 현재 UI 입력 텍스트를 정확히 맞추지 못했습니다."
    if [ "$REQUIRE_INPUT_MATCH" = "1" ]; then
      return 2
    fi
  fi

  local mode_state
  mode_state="$(ensure_nl_mode)"
  if [ "$mode_state" = "NO_PROCESS" ] || [ "$mode_state" = "NOT_FOUND" ]; then
    echo "❌ 제출 직전 자연어 탭 확인 실패(state=${mode_state})"
    return 2
  fi
  focus_input_point

  case "$method" in
    enter)
      osascript <<'APPLESCRIPT' >/dev/null 2>&1
tell application "System Events"
  key code 36
end tell
APPLESCRIPT
      ;;
    button)
      if [ -n "$SEND_POINT" ] && [ -n "$CLICLICK_BIN" ]; then
        "$CLICLICK_BIN" "c:${SEND_POINT}" >/dev/null 2>&1 || true
      else
        osascript <<'APPLESCRIPT' >/dev/null 2>&1
tell application "System Events"
  key code 36
end tell
APPLESCRIPT
      fi
      ;;
    both)
      osascript <<'APPLESCRIPT' >/dev/null 2>&1
tell application "System Events"
  key code 36
end tell
APPLESCRIPT
      if [ -n "$SEND_POINT" ] && [ -n "$CLICLICK_BIN" ]; then
        sleep 0.12
        "$CLICLICK_BIN" "c:${SEND_POINT}" >/dev/null 2>&1 || true
      fi
      ;;
    *)
      osascript <<'APPLESCRIPT' >/dev/null 2>&1
tell application "System Events"
  key code 36
end tell
APPLESCRIPT
      ;;
  esac
  LAST_SUBMIT_METHOD="$method"
  return 0
}

pick_retry_submit_method() {
  local configured="${1:-auto}"
  local last="${2:-enter}"
  case "$configured" in
    auto)
      case "$last" in
        enter) echo "button" ;;
        button) echo "enter" ;;
        both) echo "enter" ;;
        *) echo "enter" ;;
      esac
      ;;
    *)
      echo "$configured"
      ;;
  esac
}

LAST_SUBMIT_METHOD="$PRIMARY_SUBMIT_METHOD"
if ! submit_prompt_from_ui "$PRIMARY_SUBMIT_METHOD"; then
  echo "❌ 초기 UI 제출 실패(method=${PRIMARY_SUBMIT_METHOD})."
  exit 2
fi
SUBMIT_ATTEMPTS=1

START_EPOCH="$(date +%s)"
NEW_LOG=""
DETECTED_RUN_ID=""
SOURCE_HINT=""

find_new_task_run_id() {
  if ! command -v python3 >/dev/null 2>&1; then
    return 1
  fi
  local snapshot_file
  snapshot_file="$(mktemp)"
  if ! curl -fsS --max-time 5 "$TASK_RUNS_URL" >"$snapshot_file" 2>/dev/null; then
    rm -f "$snapshot_file"
    return 1
  fi
  python3 - "$snapshot_file" "$BASELINE_RUN_IDS" "$REQUEST_TEXT" "$MATCH_TASK_RUN_PROMPT" <<'PY'
import json, sys
snapshot_path = sys.argv[1]
baseline_path = sys.argv[2]
request_text = (sys.argv[3] or "").strip()
strict_prompt_match = (sys.argv[4] or "1").strip() == "1"
baseline = set()
try:
    with open(baseline_path, "r", encoding="utf-8") as f:
        baseline = {line.strip() for line in f if line.strip()}
except Exception:
    baseline = set()
try:
    data = json.load(open(snapshot_path, "r", encoding="utf-8"))
except Exception:
    data = []

def norm(text: str) -> str:
    return " ".join((text or "").strip().lower().split())

want = norm(request_text)

def prompt_matches(item_prompt: str) -> bool:
    got = norm(item_prompt)
    if not want:
        return True
    if got == want:
        return True
    # Small tolerance for punctuation/spacing differences.
    got_compact = "".join(got.split())
    want_compact = "".join(want.split())
    return bool(got_compact) and bool(want_compact) and (
        got_compact in want_compact or want_compact in got_compact
    )

candidates = []
if isinstance(data, list):
    for item in data:
        if not isinstance(item, dict):
            continue
        run_id = item.get("run_id")
        if not (isinstance(run_id, str) and run_id.strip()):
            continue
        run_id = run_id.strip()
        if run_id in baseline:
            continue
        candidates.append(item)

# Prefer prompt-matching runs first, otherwise keep waiting in strict mode.
for item in candidates:
    rid = str(item.get("run_id", "")).strip()
    if prompt_matches(str(item.get("prompt", ""))):
        print(rid)
        raise SystemExit(0)

if not strict_prompt_match and candidates:
    print(str(candidates[0].get("run_id", "")).strip())
PY
  rm -f "$snapshot_file"
}

fetch_task_run_status() {
  local run_id="$1"
  local detail_file
  detail_file="$(mktemp)"
  if ! curl -fsS --max-time 5 "$TASK_RUN_URL_BASE/$run_id" >"$detail_file" 2>/dev/null; then
    rm -f "$detail_file"
    return 1
  fi
  cp "$detail_file" "$RUN_STATUS_CAPTURE" >/dev/null 2>&1 || true
  python3 - "$detail_file" <<'PY'
import json, sys
path = sys.argv[1]
try:
    data = json.load(open(path, "r", encoding="utf-8"))
except Exception:
    print("|||")
    raise SystemExit(0)
status = data.get("status", "")
planner = str(data.get("planner_complete", ""))
execution = str(data.get("execution_complete", ""))
business = str(data.get("business_complete", ""))
print(f"{status}|{planner}|{execution}|{business}")
PY
  rm -f "$detail_file"
}

while [ -z "$DETECTED_RUN_ID" ] && [ -z "$NEW_LOG" ]; do
  DETECTED_RUN_ID="$(find_new_task_run_id || true)"
  if [ -n "$DETECTED_RUN_ID" ]; then
    break
  fi

  CURRENT_LIST="$(mktemp)"
  ls -1 "$SCENARIO_DIR"/nl_request_*.log 2>/dev/null | sort >"$CURRENT_LIST" || true
  NEW_LOG="$(comm -13 "$BASELINE_LIST" "$CURRENT_LIST" | tail -n 1 || true)"
  rm -f "$CURRENT_LIST"
  if [ -n "$NEW_LOG" ]; then
    break
  fi

  NOW="$(date +%s)"
  ELAPSED=$((NOW - START_EPOCH))
  if [ "$SUBMIT_ATTEMPTS" -lt "$MAX_SUBMIT_RETRIES" ] && [ "$ELAPSED" -ge $((SUBMIT_ATTEMPTS * RETRY_INTERVAL_SEC)) ]; then
    CURRENT_UI_INPUT="$(read_ui_input_value)"
    CURRENT_UI_INPUT_NORM="$(normalize_for_compare "$CURRENT_UI_INPUT")"
    if [ "$CURRENT_UI_INPUT" != "__UNAVAILABLE__" ] && [ -z "$CURRENT_UI_INPUT_NORM" ]; then
      SUBMIT_ATTEMPTS=$((SUBMIT_ATTEMPTS + 1))
      sleep 1
      continue
    fi
    NEXT_METHOD="$(pick_retry_submit_method "$RETRY_SUBMIT_METHOD" "$LAST_SUBMIT_METHOD")"
    echo "⚠️ UI submit 재시도(${SUBMIT_ATTEMPTS}/${MAX_SUBMIT_RETRIES}, method=${NEXT_METHOD}) ..."
    if ! submit_prompt_from_ui "$NEXT_METHOD"; then
      echo "❌ UI submit 재시도 실패(method=${NEXT_METHOD})."
      exit 2
    fi
    SUBMIT_ATTEMPTS=$((SUBMIT_ATTEMPTS + 1))
  fi
  if [ $((NOW - START_EPOCH)) -gt "$DETECT_WINDOW_SEC" ]; then
    echo "⚠️ No new run detected within ${DETECT_WINDOW_SEC}s."
    break
  fi
  sleep 1
done

FINAL_STATUS="unknown"
if [ -n "$DETECTED_RUN_ID" ]; then
  echo "📄 Detected task run: $DETECTED_RUN_ID"
  SOURCE_HINT="run_id:${DETECTED_RUN_ID}"
  END_DEADLINE=$((START_EPOCH + RUN_TIMEOUT_SEC))
  LAST_RUN_SIGNATURE=""
  LAST_RUN_PROGRESS_EPOCH="$(date +%s)"
  : >"$RUN_LOG_CAPTURE"
  while true; do
    SNAPSHOT="$(fetch_task_run_status "$DETECTED_RUN_ID" || true)"
    if [ -n "$SNAPSHOT" ]; then
      IFS='|' read -r RUN_STATUS RUN_PLANNER RUN_EXECUTION RUN_BUSINESS <<<"$SNAPSHOT"
      RUN_SIGNATURE="${RUN_STATUS}|${RUN_PLANNER}|${RUN_EXECUTION}|${RUN_BUSINESS}"
      if [ "$RUN_SIGNATURE" != "$LAST_RUN_SIGNATURE" ]; then
        LAST_RUN_SIGNATURE="$RUN_SIGNATURE"
        LAST_RUN_PROGRESS_EPOCH="$(date +%s)"
      fi
      printf '%s|run_id=%s|status=%s|planner=%s|execution=%s|business=%s\n' \
        "$(date -u +%FT%TZ)" "$DETECTED_RUN_ID" "${RUN_STATUS:-unknown}" \
        "${RUN_PLANNER:-}" "${RUN_EXECUTION:-}" "${RUN_BUSINESS:-}" >>"$RUN_LOG_CAPTURE"
      case "${RUN_STATUS:-}" in
        completed|success|error|failed|blocked|manual_required|approval_required)
          FINAL_STATUS="$RUN_STATUS"
          break
          ;;
      esac
    fi
    NOW="$(date +%s)"
    if [ $((NOW - LAST_RUN_PROGRESS_EPOCH)) -ge "$MAX_RUN_IDLE_SEC" ]; then
      FINAL_STATUS="stalled"
      echo "⚠️ run_id=${DETECTED_RUN_ID} 상태 변화 없음(${MAX_RUN_IDLE_SEC}s) → 시연 정지 처리"
      break
    fi
    if [ "$NOW" -ge "$END_DEADLINE" ]; then
      FINAL_STATUS="timeout"
      break
    fi
    sleep 2
  done
elif [ -n "$NEW_LOG" ] && [ -f "$NEW_LOG" ]; then
  echo "📄 Detected run log: $NEW_LOG"
  SOURCE_HINT="log:${NEW_LOG}"
  END_DEADLINE=$((START_EPOCH + RUN_TIMEOUT_SEC))
  while true; do
    if rg -n "^Done\\.|^- status: " "$NEW_LOG" >/dev/null 2>&1; then
      FINAL_STATUS="$(rg -n "^- status: " "$NEW_LOG" | tail -n 1 | sed 's/.*- status: //')"
      [ -n "$FINAL_STATUS" ] || FINAL_STATUS="finished"
      break
    fi
    NOW="$(date +%s)"
    if [ "$NOW" -ge "$END_DEADLINE" ]; then
      FINAL_STATUS="timeout"
      break
    fi
    sleep 2
  done
  cp "$NEW_LOG" "$RUN_LOG_CAPTURE" || true
else
  if [ "$FALLBACK_RUN" = "1" ]; then
    echo "⚠️ UI submit 감지 실패. 동일 요청을 엔진 fallback으로 실행해 시연을 완료합니다."
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
      bash run_nl_request_with_telegram.sh "$REQUEST_TEXT" "UI Demo ${TS}"
    ) | tee -a "$RUN_LOG_CAPTURE" >/dev/null 2>&1 || true

    CURRENT_LIST="$(mktemp)"
    ls -1 "$SCENARIO_DIR"/nl_request_*.log 2>/dev/null | sort >"$CURRENT_LIST" || true
    NEW_LOG="$(comm -13 "$BASELINE_LIST" "$CURRENT_LIST" | tail -n 1 || true)"
    rm -f "$CURRENT_LIST"
    if [ -n "$NEW_LOG" ] && [ -f "$NEW_LOG" ]; then
      FINAL_STATUS="$(rg -n "^- status: " "$NEW_LOG" | tail -n 1 | sed 's/.*- status: //')"
      [ -n "$FINAL_STATUS" ] || FINAL_STATUS="finished"
      cp "$NEW_LOG" "$RUN_LOG_CAPTURE" || true
      SOURCE_HINT="fallback_log:${NEW_LOG}"
    else
      FINAL_STATUS="fallback_done_but_log_not_found"
      SOURCE_HINT="fallback:no_log"
    fi
  else
    FINAL_STATUS="no_run_detected"
    SOURCE_HINT="none"
  fi
fi

sleep 1
kill -INT "$FFMPEG_PID" 2>/dev/null || true
wait "$FFMPEG_PID" 2>/dev/null || true
unset FFMPEG_PID

echo "✅ UI demo capture done."
echo "   video=$VIDEO_FILE"
echo "   log_copy=$RUN_LOG_CAPTURE"
echo "   source=${SOURCE_HINT:-none}"
echo "   final_status=$FINAL_STATUS"
echo "   run_status_json=$RUN_STATUS_CAPTURE"
echo "   ffmpeg_log=$FFMPEG_LOG"

if [ "$REQUIRE_RUN_DETECTION" = "1" ] && [[ "$FINAL_STATUS" =~ ^(no_run_detected|timeout|stalled)$ ]]; then
    echo "❌ UI 데모 실패: 실행 상태가 안정적으로 완료되지 않았습니다(final_status=${FINAL_STATUS})."
    exit 2
fi

if [ "$REQUIRE_SUCCESS_STATUS" = "1" ]; then
    case "$FINAL_STATUS" in
        completed|success|finished)
            ;;
        *)
            echo "❌ UI 데모 실패: 실행은 감지됐지만 성공 상태가 아닙니다(final_status=${FINAL_STATUS})."
            exit 2
            ;;
    esac
fi
