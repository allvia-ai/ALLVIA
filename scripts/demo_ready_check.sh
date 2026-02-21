#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CORE_ENV_FILE="$ROOT_DIR/core/.env"
STATUS_URL="${STEER_CORE_STATUS_URL:-http://127.0.0.1:5680/api/status}"
PREFLIGHT_URL="${STEER_CORE_PREFLIGHT_URL:-http://127.0.0.1:5680/api/agent/preflight}"
GOAL_RUN_URL="${STEER_CORE_GOAL_RUN_URL:-http://127.0.0.1:5680/api/agent/goal/run}"
LEGACY_INTENT_URL="${STEER_CORE_LEGACY_INTENT_URL:-http://127.0.0.1:5680/api/agent/intent}"
LEGACY_PLAN_URL="${STEER_CORE_LEGACY_PLAN_URL:-http://127.0.0.1:5680/api/agent/plan}"
LEGACY_EXECUTE_URL="${STEER_CORE_LEGACY_EXECUTE_URL:-http://127.0.0.1:5680/api/agent/execute}"
LEGACY_GOAL_URL="${STEER_CORE_LEGACY_GOAL_URL:-http://127.0.0.1:5680/api/agent/goal}"

ok() { echo "✅ $1"; }
warn() { echo "⚠️  $1"; }
fail() { echo "❌ $1"; }

is_endpoint_reachable_code() {
  local code="${1:-000}"
  case "$code" in
    200|400|401|403|409|422|429|500|503)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

check_cmd() {
  local name="$1"
  if command -v "$name" >/dev/null 2>&1; then
    ok "command available: $name ($(command -v "$name"))"
    return 0
  fi
  fail "missing command: $name"
  return 1
}

check_env_key() {
  local key="$1"
  if [ ! -f "$CORE_ENV_FILE" ]; then
    warn "core/.env not found ($CORE_ENV_FILE)"
    return 0
  fi
  if rg -n "^${key}=.+" "$CORE_ENV_FILE" >/dev/null 2>&1; then
    ok "core/.env has $key"
  else
    warn "core/.env missing $key"
  fi
}

check_steer_app() {
  local app_id
  app_id="$(osascript <<'APPLESCRIPT' 2>/dev/null || true
try
  return id of application "Steer OS"
on error
  return ""
end try
APPLESCRIPT
)"
  if [ -n "$app_id" ]; then
    ok "Steer OS app installed ($app_id)"
    return 0
  fi
  fail "Steer OS app not found. 앱 번들이 설치되어 있어야 UI 시연이 가능합니다."
  return 1
}

check_steer_process() {
  local running
  running="$(osascript <<'APPLESCRIPT' 2>/dev/null || true
tell application "System Events"
  if exists process "Steer OS" then return "1"
end tell
return "0"
APPLESCRIPT
)"
  if [ "$running" = "1" ]; then
    ok "Steer OS process running"
  else
    warn "Steer OS process not running now (시연 시작 시 자동 실행됨)"
  fi
}

echo "=== Steer Demo Ready Check ==="
echo "repo: $ROOT_DIR"
echo

TOOL_FAIL=0
check_steer_app || TOOL_FAIL=1
check_steer_process
check_cmd ffmpeg || TOOL_FAIL=1
check_cmd osascript || TOOL_FAIL=1
if ! check_cmd cliclick; then
  warn "cliclick missing: UI automation reliability will be lower"
fi

echo
HTTP_CODE="$(curl -sS -o /tmp/steer_demo_status.json -w "%{http_code}" "$STATUS_URL" || true)"
if [ "$HTTP_CODE" = "200" ]; then
  ok "core status API reachable ($STATUS_URL)"
else
  fail "core status API not ready ($STATUS_URL, http=$HTTP_CODE)"
fi

PREFLIGHT_CODE="$(curl -sS -o /tmp/steer_demo_preflight.json -w "%{http_code}" "$PREFLIGHT_URL" || true)"
if [ "$PREFLIGHT_CODE" = "200" ]; then
  ok "preflight API reachable ($PREFLIGHT_URL)"
elif [ "$PREFLIGHT_CODE" = "404" ]; then
  warn "preflight API 404 (legacy core mode). launcher will continue without hard preflight gate."
else
  warn "preflight API status=$PREFLIGHT_CODE ($PREFLIGHT_URL)"
fi

GOAL_RUN_CODE="$(curl -sS -X POST -H 'Content-Type: application/json' -d '{"goal":"demo probe"}' -o /tmp/steer_demo_goal_run_probe.json -w "%{http_code}" "$GOAL_RUN_URL" || true)"
case "$GOAL_RUN_CODE" in
  200|400|422|500|503)
    ok "goal-run endpoint reachable (POST status=$GOAL_RUN_CODE, $GOAL_RUN_URL)"
    ;;
  404|405|501)
    INTENT_CODE="$(curl -sS -X POST -H 'Content-Type: application/json' -d '{"text":"demo probe"}' -o /tmp/steer_demo_intent_probe.json -w "%{http_code}" "$LEGACY_INTENT_URL" || true)"
    PLAN_CODE="$(curl -sS -X POST -H 'Content-Type: application/json' -d '{"session_id":"demo_probe"}' -o /tmp/steer_demo_plan_probe.json -w "%{http_code}" "$LEGACY_PLAN_URL" || true)"
    EXECUTE_CODE="$(curl -sS -X POST -H 'Content-Type: application/json' -d '{"plan_id":"demo_probe"}' -o /tmp/steer_demo_execute_probe.json -w "%{http_code}" "$LEGACY_EXECUTE_URL" || true)"
    LEGACY_GOAL_CODE="$(curl -sS -X POST -H 'Content-Type: application/json' -d '{"goal":"demo probe"}' -o /tmp/steer_demo_legacy_goal_probe.json -w "%{http_code}" "$LEGACY_GOAL_URL" || true)"
    if is_endpoint_reachable_code "$INTENT_CODE" && is_endpoint_reachable_code "$PLAN_CODE" && is_endpoint_reachable_code "$EXECUTE_CODE"; then
      ok "goal-run 미지원 코어 감지, legacy intent/plan/execute 경로 사용 가능 (intent=$INTENT_CODE, plan=$PLAN_CODE, execute=$EXECUTE_CODE)"
    elif is_endpoint_reachable_code "$LEGACY_GOAL_CODE"; then
      ok "goal-run 미지원 코어 감지, legacy goal 경로 사용 가능 (goal=$LEGACY_GOAL_CODE)"
    else
      warn "goal-run 미지원 + legacy 체인 일부 미확인 (intent=$INTENT_CODE, plan=$PLAN_CODE, execute=$EXECUTE_CODE, goal=$LEGACY_GOAL_CODE)"
    fi
    ;;
  *)
    warn "goal-run endpoint probe uncertain (status=$GOAL_RUN_CODE, $GOAL_RUN_URL)"
    ;;
esac

echo
check_env_key OPENAI_API_KEY
if [ -f "$CORE_ENV_FILE" ] && rg -n "^STEER_OPENAI_MODEL=.+" "$CORE_ENV_FILE" >/dev/null 2>&1; then
  ok "core/.env has STEER_OPENAI_MODEL"
else
  ok "STEER_OPENAI_MODEL not set (default model will be used)"
fi
check_env_key TELEGRAM_BOT_TOKEN
check_env_key TELEGRAM_CHAT_ID

if [ -n "${VITE_API_BASE_URL:-}" ]; then
  if [[ "${VITE_API_BASE_URL%/}" == */api ]]; then
    ok "VITE_API_BASE_URL suffix looks good (/api)"
  else
    warn "VITE_API_BASE_URL should end with /api (current: ${VITE_API_BASE_URL})"
  fi
fi

echo
if [ "$TOOL_FAIL" -eq 0 ] && [ "$HTTP_CODE" = "200" ]; then
  ok "demo baseline READY (tools + core status)"
else
  fail "demo baseline NOT READY. fix the red items first."
  exit 1
fi
