#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
FIX_URL="${STEER_CORE_PREFLIGHT_FIX_URL:-http://127.0.0.1:5680/api/agent/preflight/fix}"
ENABLE_ISOLATED_MODE="${STEER_DEMO_ENABLE_ISOLATED_MODE:-0}"
RESET_MAIL_OUTGOING="${STEER_DEMO_RESET_MAIL_OUTGOING:-1}"
RESET_NOTES_WINDOWS="${STEER_DEMO_RESET_NOTES_WINDOWS:-1}"
RESET_TEXTEDIT_WINDOWS="${STEER_DEMO_RESET_TEXTEDIT_WINDOWS:-1}"

ok() { echo "✅ $1"; }
warn() { echo "⚠️  $1"; }
fail() { echo "❌ $1"; }

if ! command -v curl >/dev/null 2>&1; then
  fail "curl not found"
  exit 1
fi

run_fix() {
  local action="$1"
  local tmp
  tmp="$(mktemp)"
  local code
  code="$(curl -sS -o "$tmp" -w "%{http_code}" \
    -X POST "$FIX_URL" \
    -H "content-type: application/json" \
    -d "{\"action\":\"$action\"}" || true)"

  if [ "$code" = "200" ]; then
    local message
    message="$(rg -o '"message"\s*:\s*"[^"]*"' "$tmp" | head -n1 | sed -E 's/^"message"\s*:\s*"//; s/"$//')"
    if [ -n "$message" ]; then
      ok "$action -> $message"
    else
      ok "$action -> done"
    fi
    rm -f "$tmp"
    return 0
  fi

  if [ "$code" = "404" ]; then
    warn "preflight fix endpoint not found (legacy core): $FIX_URL"
    rm -f "$tmp"
    return 0
  fi

  local err_msg=""
  if [ -s "$tmp" ]; then
    err_msg="$(rg -o '"error"\s*:\s*"[^"]*"' "$tmp" | head -n1 | sed -E 's/^"error"\s*:\s*"//; s/"$//')"
  fi
  if [ -n "$err_msg" ]; then
    warn "$action failed (http=$code, error=$err_msg)"
  else
    warn "$action failed (http=$code)"
  fi
  rm -f "$tmp"
  return 0
}

echo "=== Demo State Reset ==="
echo "repo: $ROOT_DIR"
echo "fix_url: $FIX_URL"
echo

run_fix "activate_finder"
if [ "$RESET_MAIL_OUTGOING" = "1" ]; then
  run_fix "mail_cleanup_outgoing_windows"
else
  warn "mail outgoing cleanup skipped (set STEER_DEMO_RESET_MAIL_OUTGOING=1 to enable)"
fi

if [ "$RESET_NOTES_WINDOWS" = "1" ]; then
  osascript <<'APPLESCRIPT' >/dev/null 2>&1 || true
tell application "Notes"
  if running then
    try
      set winCount to count of windows
      if winCount > 1 then
        repeat with i from winCount to 2 by -1
          try
            close window i
          end try
        end repeat
      end if
    end try
  end if
end tell
APPLESCRIPT
  ok "notes window cleanup -> done"
else
  warn "notes window cleanup skipped (set STEER_DEMO_RESET_NOTES_WINDOWS=1 to enable)"
fi

if [ "$RESET_TEXTEDIT_WINDOWS" = "1" ]; then
  osascript <<'APPLESCRIPT' >/dev/null 2>&1 || true
tell application "TextEdit"
  if running then
    try
      set winCount to count of windows
      if winCount > 0 then
        repeat with i from winCount to 1 by -1
          try
            close window i saving no
          end try
        end repeat
      end if
    end try
  end if
end tell
APPLESCRIPT
  ok "textedit window cleanup -> done"
else
  warn "textedit window cleanup skipped (set STEER_DEMO_RESET_TEXTEDIT_WINDOWS=1 to enable)"
fi

if [ "$ENABLE_ISOLATED_MODE" = "1" ]; then
  run_fix "prepare_isolated_mode"
else
  warn "isolated mode skipped (set STEER_DEMO_ENABLE_ISOLATED_MODE=1 to enable)"
fi

echo
ok "demo state reset complete"
