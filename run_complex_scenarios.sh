#!/bin/bash
set -e

# Load environment variables
if [ -f core/.env ]; then
    set -a
    # shellcheck disable=SC1091
    source core/.env
    set +a
fi

echo "🚀 Starting Complex Scenarios 1-5 Execution..."
echo "⚠️  PLEASE DO NOT TOUCH THE MOUSE/KEYBOARD DURING EXECUTION"
echo ""

# Create output directory for results
mkdir -p scenario_results
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
SUCCESS_COUNT=0
FAIL_COUNT=0
SCENARIO_MODE_VALUE="${STEER_SCENARIO_MODE:-0}"
NODE_CAPTURE_ALL_VALUE="${STEER_NODE_CAPTURE_ALL:-1}"
CLI_LLM_VALUE="${STEER_CLI_LLM-}"
FAIL_ON_FALLBACK_VALUE="${STEER_FAIL_ON_FALLBACK:-1}"
NOTIFIER_TIMEOUT_SEC="${STEER_NOTIFIER_TIMEOUT_SEC:-25}"
REQUIRE_PRIMARY_PLANNER_VALUE="${STEER_REQUIRE_PRIMARY_PLANNER:-1}"
LOCK_DISABLED_VALUE="${STEER_LOCK_DISABLED:-0}"
MAIL_TO_TARGET="${STEER_DEFAULT_MAIL_TO:-$(git config --get user.email 2>/dev/null || true)}"

SUBJECT_S1="Today Plan Brief S1_${TIMESTAMP}"
SUBJECT_S2="Downloads Triage S2_${TIMESTAMP}"
SUBJECT_S3="Calc Result S3_${TIMESTAMP}"
SUBJECT_S4="Productivity Research S4_${TIMESTAMP}"
SUBJECT_S5="Budget Check S5_${TIMESTAMP}"
MARKER_S1="RUN_SCOPE_S1_${TIMESTAMP}"
MARKER_S2="RUN_SCOPE_S2_${TIMESTAMP}"
MARKER_S3="RUN_SCOPE_S3_${TIMESTAMP}"
MARKER_S4="RUN_SCOPE_S4_${TIMESTAMP}"
MARKER_S5="RUN_SCOPE_S5_${TIMESTAMP}"
CURRENT_SCENARIO_MARKER=""

if [ "$REQUIRE_PRIMARY_PLANNER_VALUE" = "1" ] && [ "$SCENARIO_MODE_VALUE" = "1" ] && [ "${STEER_ALLOW_SCENARIO_MODE:-0}" != "1" ]; then
    echo "❌ 정책 위반: STEER_SCENARIO_MODE=1 이지만 STEER_ALLOW_SCENARIO_MODE=1 승인 없이 fallback 모드 실행은 금지됩니다."
    echo "   운영 검증은 STEER_SCENARIO_MODE=0으로 실행하거나, 테스트 목적일 때만 STEER_ALLOW_SCENARIO_MODE=1을 설정하세요."
    exit 1
fi

echo "🔧 STEER_SCENARIO_MODE=${SCENARIO_MODE_VALUE} (0=LLM planning, 1=fallback scenario mode)"
echo "📸 STEER_NODE_CAPTURE=1, STEER_NODE_CAPTURE_ALL=${NODE_CAPTURE_ALL_VALUE}"
echo "🧯 STEER_FAIL_ON_FALLBACK=${FAIL_ON_FALLBACK_VALUE} (1=mark failed on fallback action)"
if [ -n "$CLI_LLM_VALUE" ]; then
    echo "🤖 STEER_CLI_LLM=${CLI_LLM_VALUE}"
else
echo "🤖 STEER_CLI_LLM=disabled (using default OpenAI path)"
fi
echo ""

require_terminal_context() {
    local require_terminal="${STEER_REQUIRE_TERMINAL:-1}"
    [ "$require_terminal" = "1" ] || return 0

    local term_program="${TERM_PROGRAM:-unknown}"
    local allowed_programs="${STEER_ALLOWED_TERM_PROGRAMS:-Apple_Terminal,unknown}"
    local allowed_match=0
    IFS=',' read -r -a _allowed_arr <<< "$allowed_programs"
    for entry in "${_allowed_arr[@]}"; do
        entry="$(echo "$entry" | tr -d ' ')"
        if [ "$term_program" = "$entry" ]; then
            allowed_match=1
            break
        fi
    done

    if [ "$allowed_match" -ne 1 ]; then
        echo "❌ 실행 환경 고정 위반: TERM_PROGRAM=${term_program}"
        echo "   이 스크립트는 Terminal(기본: Apple_Terminal)에서만 실행하도록 설정됨."
        echo "   필요 시 STEER_ALLOWED_TERM_PROGRAMS로 허용 목록을 조정하세요."
        return 1
    fi

    local pid="$$"
    local hop=0
    while [ "$hop" -lt 20 ]; do
        local cmd=""
        cmd="$(ps -o command= -p "$pid" 2>/dev/null || true)"
        if echo "$cmd" | grep -Eiq 'Antigravity\.app|Antigravity Helper'; then
            echo "❌ Antigravity 프로세스 경유 실행 감지됨."
            echo "   Terminal 앱에서 직접 실행해 주세요."
            return 1
        fi
        local ppid=""
        ppid="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ' || true)"
        [ -z "$ppid" ] && break
        [ "$ppid" = "1" ] && break
        pid="$ppid"
        hop=$((hop + 1))
    done
    return 0
}

RUN_TIMEOUT_STDOUT=""
RUN_TIMEOUT_STDERR=""
RUN_TIMEOUT_EXIT=0

run_cmd_with_timeout_capture() {
    local timeout_sec="$1"
    shift

    RUN_TIMEOUT_STDOUT=""
    RUN_TIMEOUT_STDERR=""
    RUN_TIMEOUT_EXIT=0

    local tmp_out=""
    local tmp_err=""
    local cmd_pid=""
    local elapsed=0
    tmp_out="$(mktemp -t steer_cmd_out.XXXXXX)"
    tmp_err="$(mktemp -t steer_cmd_err.XXXXXX)"

    "$@" >"$tmp_out" 2>"$tmp_err" &
    cmd_pid=$!

    while kill -0 "$cmd_pid" 2>/dev/null; do
        if [ "$elapsed" -ge "$timeout_sec" ]; then
            kill -9 "$cmd_pid" 2>/dev/null || true
            wait "$cmd_pid" 2>/dev/null || true
            RUN_TIMEOUT_STDOUT="$(cat "$tmp_out" 2>/dev/null || true)"
            RUN_TIMEOUT_STDERR="$(cat "$tmp_err" 2>/dev/null || true)"
            rm -f "$tmp_out" "$tmp_err"
            RUN_TIMEOUT_EXIT=124
            return 124
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    wait "$cmd_pid"
    RUN_TIMEOUT_EXIT=$?
    RUN_TIMEOUT_STDOUT="$(cat "$tmp_out" 2>/dev/null || true)"
    RUN_TIMEOUT_STDERR="$(cat "$tmp_err" 2>/dev/null || true)"
    rm -f "$tmp_out" "$tmp_err"
    return "$RUN_TIMEOUT_EXIT"
}

semantic_location_missing() {
    case "$1" in
        NOT_FOUND|CHECK_ERROR|CHECK_TIMEOUT|MARKER_REQUIRED|"")
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

normalize_semantic_token() {
    printf '%s' "$1" | tr '\r\n\t' '   ' | sed -E 's/[[:space:]]+/ /g; s/^[[:space:]]+//; s/[[:space:]]+$//'
}

# Validate required runtime permissions/config before running long scenarios.
preflight_checks() {
    local failed=0
    local ax_out=""
    local capture_out=""
    local preflight_capture="scenario_results/preflight_capture_${TIMESTAMP}.png"
    local preflight_timeout="${STEER_PREFLIGHT_TIMEOUT_SEC:-6}"

    echo "🔎 Running preflight checks..."

    if ! require_terminal_context; then
        return 1
    fi

    if ! command -v osascript >/dev/null 2>&1; then
        echo "❌ Preflight failed: osascript not found."
        failed=1
    elif ! run_cmd_with_timeout_capture "$preflight_timeout" osascript -e 'tell application "System Events" to return name of first application process'; then
        ax_out="${RUN_TIMEOUT_STDERR:-$RUN_TIMEOUT_STDOUT}"
        echo "❌ Preflight failed: Accessibility permission check failed."
        if [ "$RUN_TIMEOUT_EXIT" -eq 124 ]; then
            echo "   Cause: 접근성 검사 타임아웃(${preflight_timeout}s)."
        fi
        [ -n "$ax_out" ] && echo "   Details: $ax_out"
        if echo "$ax_out" | grep -q -- "-10827"; then
            echo "   Cause: 접근성 권한이 없거나 현재 실행 세션에서 UI 자동화가 차단되었습니다."
        elif echo "$ax_out" | grep -Eq "Connection Invalid|-1728"; then
            echo "   Cause: GUI 세션에 연결되지 않아 AppleScript 앱 제어가 불가능합니다."
        fi
        echo "   Fix: System Settings > Privacy & Security > Accessibility에서 Terminal/Codex를 허용하세요."
        failed=1
    else
        echo "✅ Preflight: Accessibility permission looks available."
    fi

    if ! command -v screencapture >/dev/null 2>&1; then
        echo "❌ Preflight failed: screencapture command not found."
        failed=1
    elif ! run_cmd_with_timeout_capture "$preflight_timeout" screencapture -x "$preflight_capture"; then
        capture_out="${RUN_TIMEOUT_STDERR:-$RUN_TIMEOUT_STDOUT}"
        echo "❌ Preflight failed: Screen Recording/display capture unavailable."
        if [ "$RUN_TIMEOUT_EXIT" -eq 124 ]; then
            echo "   Cause: 화면 캡처 검사 타임아웃(${preflight_timeout}s)."
        fi
        [ -n "$capture_out" ] && echo "   Details: $capture_out"
        if echo "$capture_out" | grep -q "could not create image from display"; then
            echo "   Cause: 현재 실행 세션에서 디스플레이 접근이 불가능합니다."
        fi
        echo "   Fix: System Settings > Privacy & Security > Screen Recording에서 Terminal/Codex를 허용하세요."
        failed=1
    else
        echo "✅ Preflight: Screen capture works."
        rm -f "$preflight_capture"
    fi

    if [ "$SCENARIO_MODE_VALUE" = "0" ] && [ -z "$CLI_LLM_VALUE" ] && [ -z "${OPENAI_API_KEY:-}" ]; then
        echo "❌ Preflight failed: OPENAI_API_KEY is not set."
        echo "   Fix: 기본 OpenAI 경로를 쓰려면 core/.env 또는 현재 셸에 OPENAI_API_KEY를 설정하세요."
        echo "   대안: STEER_CLI_LLM 설정 또는 STEER_SCENARIO_MODE=1(테스트 전용) 사용."
        failed=1
    elif [ -n "${OPENAI_API_KEY:-}" ]; then
        echo "✅ Preflight: OPENAI_API_KEY detected."
    else
        echo "ℹ️ Preflight: OPENAI_API_KEY not required in current mode (CLI/scenario path)."
    fi

    if [ "${STEER_REQUIRE_MAIL_SEND:-1}" = "1" ] && [ -z "$MAIL_TO_TARGET" ]; then
        echo "❌ Preflight failed: mail send target is empty."
        echo "   Fix: STEER_DEFAULT_MAIL_TO 또는 git user.email 을 설정하세요."
        failed=1
    fi

    if [ "$failed" -ne 0 ]; then
        echo ""
        echo "⛔ Preflight checks failed. Aborting scenario run."
        return 1
    fi

    echo "✅ Preflight checks passed."
    return 0
}

send_telegram_with_timeout() {
    local timeout_sec="$1"
    shift
    if run_cmd_with_timeout_capture "$timeout_sec" "$@"; then
        return 0
    fi
    if [ "$RUN_TIMEOUT_EXIT" -eq 124 ]; then
        echo "Warning: Telegram notification timed out (${timeout_sec}s)." >&2
    else
        local err_detail="${RUN_TIMEOUT_STDERR:-$RUN_TIMEOUT_STDOUT}"
        if [ -n "$err_detail" ]; then
            echo "Warning: Telegram notification failed: $err_detail" >&2
        else
            echo "Warning: Telegram notification failed." >&2
        fi
    fi
    return 1
}

mail_outgoing_count() {
    local timeout_sec="${STEER_OSASCRIPT_TIMEOUT_SEC:-8}"
    if run_cmd_with_timeout_capture "$timeout_sec" \
        osascript -e 'tell application "Mail" to return count of outgoing messages'; then
        printf '%s' "${RUN_TIMEOUT_STDOUT}" | tr -d '[:space:]'
        return 0
    fi
    echo "-1"
    return 1
}

get_idle_seconds() {
    local idle_raw
    idle_raw=$(ioreg -c IOHIDSystem 2>/dev/null | awk '/HIDIdleTime/ {print $NF; exit}')
    if [ -z "$idle_raw" ]; then
        return 1
    fi
    # HIDIdleTime is nanoseconds.
    echo $((idle_raw / 1000000000))
    return 0
}

get_frontmost_app() {
    osascript -e 'tell application "System Events" to get name of first process whose frontmost is true' 2>/dev/null || true
}

is_user_active_front_app() {
    local app="$1"
    local user_apps_csv="${STEER_USER_ACTIVE_APPS:-Terminal,Codex,iTerm2}"
    local oldifs="$IFS"
    IFS=','
    read -r -a apps <<< "$user_apps_csv"
    IFS="$oldifs"
    for item in "${apps[@]}"; do
        local trimmed
        trimmed="$(echo "$item" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
        [ -z "$trimmed" ] && continue
        if [ "$app" = "$trimmed" ]; then
            return 0
        fi
    done
    return 1
}

run_surf_with_input_guard() {
    local prompt="$1"
    local log_file="$2"
    local node_dir="$3"
    local use_guard="${STEER_PAUSE_ON_USER_INPUT:-1}"
    if [ "$use_guard" != "1" ]; then
        if [ -n "$CLI_LLM_VALUE" ]; then
            STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
                STEER_CLI_LLM="$CLI_LLM_VALUE" \
                STEER_NODE_CAPTURE=1 \
                STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
                STEER_NODE_CAPTURE_DIR="$node_dir" \
                STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
                STEER_TEST_MODE=1 \
                cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$prompt" &> "$log_file"
        else
            STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
                STEER_NODE_CAPTURE=1 \
                STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
                STEER_NODE_CAPTURE_DIR="$node_dir" \
                STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
                STEER_TEST_MODE=1 \
                cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$prompt" &> "$log_file"
        fi
        return $?
    fi

    local active_threshold="${STEER_INPUT_ACTIVE_THRESHOLD_SECONDS:-1}"
    local resume_idle="${STEER_IDLE_RESUME_SECONDS:-3}"
    local poll_interval="${STEER_INPUT_POLL_SECONDS:-1}"
    local paused=0
    local pause_count=0
    local run_pid=""

    echo "🛡️ User-input guard enabled (apps=${STEER_USER_ACTIVE_APPS:-Terminal,Codex,iTerm2}, active<=${active_threshold}s, resume>=${resume_idle}s)"

    if [ -n "$CLI_LLM_VALUE" ]; then
        STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
            STEER_CLI_LLM="$CLI_LLM_VALUE" \
            STEER_NODE_CAPTURE=1 \
            STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
            STEER_NODE_CAPTURE_DIR="$node_dir" \
            STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
            STEER_TEST_MODE=1 \
            cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$prompt" &> "$log_file" &
    else
        STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
            STEER_NODE_CAPTURE=1 \
            STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
            STEER_NODE_CAPTURE_DIR="$node_dir" \
            STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
            STEER_TEST_MODE=1 \
            cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$prompt" &> "$log_file" &
    fi
    run_pid=$!

    while kill -0 "$run_pid" 2>/dev/null; do
        local idle_sec=""
        idle_sec="$(get_idle_seconds || true)"
        if [ -n "$idle_sec" ]; then
            local front_app=""
            front_app="$(get_frontmost_app)"
            if [ "$paused" -eq 0 ] && [ "$idle_sec" -le "$active_threshold" ] && is_user_active_front_app "$front_app"; then
                kill -STOP "$run_pid" >/dev/null 2>&1 || true
                pkill -STOP -P "$run_pid" >/dev/null 2>&1 || true
                paused=1
                pause_count=$((pause_count + 1))
                echo "⏸️ [InputGuard] Paused run (front_app=${front_app}, idle=${idle_sec}s, count=${pause_count})"
                echo "⏸️ [InputGuard] Paused run (front_app=${front_app}, idle=${idle_sec}s, count=${pause_count})" >> "$log_file"
            elif [ "$paused" -eq 1 ] && [ "$idle_sec" -ge "$resume_idle" ]; then
                kill -CONT "$run_pid" >/dev/null 2>&1 || true
                pkill -CONT -P "$run_pid" >/dev/null 2>&1 || true
                paused=0
                echo "▶️ [InputGuard] Resumed run (idle=${idle_sec}s)"
                echo "▶️ [InputGuard] Resumed run (idle=${idle_sec}s)" >> "$log_file"
            fi
        fi
        sleep "$poll_interval"
    done

    wait "$run_pid"
    local exit_code=$?
    echo "🧾 [InputGuard] pause_count=${pause_count}"
    echo "🧾 [InputGuard] pause_count=${pause_count}" >> "$log_file"
    return $exit_code
}

token_presence_location() {
    local token="$1"
    local marker="${2:-}"
    local require_marker="${STEER_SEMANTIC_REQUIRE_MARKER:-1}"
    local scan_limit="${STEER_MAIL_SENT_SCAN_LIMIT:-120}"
    local result=""
    local timeout_sec="${STEER_OSASCRIPT_TIMEOUT_SEC:-8}"
    local tmp_out=""
    local tmp_err=""
    local osa_pid=""

    if [ "$require_marker" = "1" ] && [ -z "$marker" ]; then
        printf '%s\n' "MARKER_REQUIRED"
        return 0
    fi
    tmp_out="$(mktemp -t steer_osa_out.XXXXXX)"
    tmp_err="$(mktemp -t steer_osa_err.XXXXXX)"

    (
        osascript - "$token" "$marker" "$scan_limit" <<'APPLESCRIPT'
on run argv
    set tokenText to item 1 of argv
    set markerText to ""
    if (count of argv) > 1 then set markerText to item 2 of argv
    set scanLimit to 120
    if (count of argv) > 2 then
        try
            set scanLimit to (item 3 of argv) as integer
        on error
            set scanLimit to 120
        end try
    end if
    if scanLimit < 10 then set scanLimit to 10

    try
        tell application "Notes"
            if (count of accounts) > 0 then
                repeat with ac in accounts
                    repeat with f in folders of ac
                        repeat with n in notes of f
                            try
                                set nName to name of n as text
                            on error
                                set nName to ""
                            end try
                            try
                                set nBody to body of n as text
                            on error
                                set nBody to ""
                            end try
                            set scopeOk to (markerText is "" or nBody contains markerText or nName contains markerText)
                            if scopeOk and nName contains tokenText then return "NOTE_TITLE"
                            if scopeOk and nBody contains tokenText then return "NOTE_BODY"
                        end repeat
                    end repeat
                end repeat
            end if
        end tell
    on error
        return "CHECK_ERROR"
    end try

    try
        tell application "Mail"
            set draftCount to count of outgoing messages
            if draftCount > 0 then
                set lowerDraft to draftCount - scanLimit
                if lowerDraft < 1 then set lowerDraft to 1
                repeat with idx from draftCount to lowerDraft by -1
                    set m to item idx of outgoing messages
                    try
                        set s to subject of m as text
                    on error
                        set s to ""
                    end try

                    try
                        set c to content of m as text
                    on error
                        set c to ""
                    end try
                    set scopeOk to (markerText is "" or c contains markerText or s contains markerText)
                    if scopeOk and s contains tokenText then return "MAIL_SUBJECT"
                    if scopeOk and c contains tokenText then return "MAIL_BODY"
                end repeat
            end if

            repeat with ac in accounts
                try
                    set sentMbx to sent mailbox of ac
                    if sentMbx is not missing value then
                        set sentCount to count of messages of sentMbx
                        if sentCount > 0 then
                            set lowerBound to sentCount - scanLimit
                            if lowerBound < 1 then set lowerBound to 1
                            repeat with idx from sentCount to lowerBound by -1
                                set sm to message idx of sentMbx
                                set ss to ""
                                set sc to ""
                                try
                                    set ss to subject of sm as text
                                end try
                                try
                                    set sc to content of sm as text
                                end try
                                set sentScopeOk to (markerText is "" or sc contains markerText or ss contains markerText)
                                if sentScopeOk and ss contains tokenText then return "MAIL_SENT_SUBJECT"
                                if sentScopeOk and sc contains tokenText then return "MAIL_SENT_BODY"
                            end repeat
                        end if
                    end if
                end try
            end repeat
        end tell
    on error
        return "CHECK_ERROR"
    end try

    try
        tell application "TextEdit"
            set docCount to count of documents
            if docCount > 0 then
                set lowerDoc to docCount - scanLimit
                if lowerDoc < 1 then set lowerDoc to 1
                repeat with idx from docCount to lowerDoc by -1
                    set d to item idx of documents
                    try
                        set t to text of d as text
                    on error
                        set t to ""
                    end try
                    set scopeOk to (markerText is "" or t contains markerText)
                    if scopeOk and t contains tokenText then return "TEXTEDIT_BODY"
                end repeat
            end if
        end tell
    on error
        return "CHECK_ERROR"
    end try

    return "NOT_FOUND"
end run
APPLESCRIPT
) >"$tmp_out" 2>"$tmp_err" &
    osa_pid=$!

    local elapsed=0
    while kill -0 "$osa_pid" 2>/dev/null; do
        if [ "$elapsed" -ge "$timeout_sec" ]; then
            kill -9 "$osa_pid" 2>/dev/null || true
            wait "$osa_pid" 2>/dev/null || true
            result="CHECK_TIMEOUT"
            break
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    if [ -z "$result" ]; then
        wait "$osa_pid" 2>/dev/null || true
        result="$(cat "$tmp_out" 2>/dev/null || true)"
    fi

    rm -f "$tmp_out" "$tmp_err"

    if [ -z "$result" ]; then
        result="CHECK_ERROR"
    fi
    printf '%s\n' "$result"
}

mail_sent_recipient_location() {
    local recipient="$1"
    local marker="${2:-}"
    local require_marker="${STEER_SEMANTIC_REQUIRE_MARKER:-1}"
    local scan_limit="${STEER_MAIL_SENT_SCAN_LIMIT:-120}"
    local result=""
    local timeout_sec="${STEER_OSASCRIPT_TIMEOUT_SEC:-8}"
    local tmp_out=""
    local tmp_err=""
    local osa_pid=""

    if [ -z "$recipient" ]; then
        printf '%s\n' "RECIPIENT_EMPTY"
        return 0
    fi
    if [ "$require_marker" = "1" ] && [ -z "$marker" ]; then
        printf '%s\n' "MARKER_REQUIRED"
        return 0
    fi

    tmp_out="$(mktemp -t steer_mail_recipient_out.XXXXXX)"
    tmp_err="$(mktemp -t steer_mail_recipient_err.XXXXXX)"

    (
        osascript - "$recipient" "$marker" "$scan_limit" <<'APPLESCRIPT'
on run argv
    set recipientText to item 1 of argv
    set markerText to ""
    if (count of argv) > 1 then set markerText to item 2 of argv
    set scanLimit to 120
    if (count of argv) > 2 then
        try
            set scanLimit to (item 3 of argv) as integer
        on error
            set scanLimit to 120
        end try
    end if
    if scanLimit < 10 then set scanLimit to 10

    try
        tell application "Mail"
            repeat with ac in accounts
                try
                    set sentMbx to sent mailbox of ac
                    if sentMbx is not missing value then
                        set sentCount to count of messages of sentMbx
                        if sentCount > 0 then
                            set lowerBound to sentCount - scanLimit
                            if lowerBound < 1 then set lowerBound to 1
                            repeat with idx from sentCount to lowerBound by -1
                                set sm to message idx of sentMbx
                                set ss to ""
                                set sc to ""
                                set recipientsText to ""
                                try
                                    set ss to subject of sm as text
                                end try
                                try
                                    set sc to content of sm as text
                                end try
                                try
                                    repeat with r in to recipients of sm
                                        try
                                            set recipientsText to recipientsText & " " & (address of r as text)
                                        end try
                                    end repeat
                                end try
                                set scopeOk to (markerText is "" or sc contains markerText or ss contains markerText)
                                if scopeOk and recipientsText contains recipientText then return "MAIL_SENT_RECIPIENT"
                            end repeat
                        end if
                    end if
                end try
            end repeat
        end tell
    on error
        return "CHECK_ERROR"
    end try
    return "NOT_FOUND"
end run
APPLESCRIPT
    ) >"$tmp_out" 2>"$tmp_err" &
    osa_pid=$!

    local elapsed=0
    while kill -0 "$osa_pid" 2>/dev/null; do
        if [ "$elapsed" -ge "$timeout_sec" ]; then
            kill -9 "$osa_pid" 2>/dev/null || true
            wait "$osa_pid" 2>/dev/null || true
            result="CHECK_TIMEOUT"
            break
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    if [ -z "$result" ]; then
        wait "$osa_pid" 2>/dev/null || true
        result="$(cat "$tmp_out" 2>/dev/null || true)"
    fi

    rm -f "$tmp_out" "$tmp_err"
    if [ -z "$result" ]; then
        result="CHECK_ERROR"
    fi
    printf '%s\n' "$result"
}

# Run agent command and detect logical failures from logs as well as exit code.
run_agent_scenario() {
    local prompt=$1
    local log_file=$2
    local scenario_num=$3
    local fatal_pattern='Failed to acquire lock|thread .* panicked|FATAL ERROR|⛔️|LLM not available for surf mode|Preflight failed|Surf failed|Supervisor escalated|Execution Error|SCHEMA_ERROR|PLAN_REJECTED|LLM Refused'
    local node_dir="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}_nodes"

    if ! run_surf_with_input_guard "$prompt" "$log_file" "$node_dir"; then
        return 1
    fi

    if grep -Eq "$fatal_pattern" "$log_file"; then
        return 1
    fi

    if [ "$FAIL_ON_FALLBACK_VALUE" = "1" ] && grep -Eiq "fallback action|FALLBACK_ACTION:" "$log_file"; then
        return 1
    fi

    return 0
}

# Helper function to collect step-level node captures and send notification
capture_and_notify() {
    local scenario_num=$1
    local scenario_name=$2
    local status=$3
    local log_file=$4
    local scenario_goal=$5
    local fallback_screenshot="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}.png"
    local telegram_main_image=""

    local semantic_lines=""
    local semantic_missing=0
    local mail_subject_for_verify=""
    if [ "${STEER_SEMANTIC_VERIFY:-1}" = "1" ]; then
        local expected_tokens=()
        case "$scenario_num" in
            1)
                expected_tokens=("$SUBJECT_S1" "Calendar opened" "Notes draft ready" "Mail prep pending" "Shared via TextEdit" "$MARKER_S1")
                mail_subject_for_verify="$SUBJECT_S1"
                ;;
            2)
                expected_tokens=("$SUBJECT_S2" "1. invoice.pdf" "2. screenshot.png" "3. notes.txt" "$MARKER_S2")
                mail_subject_for_verify="$SUBJECT_S2"
                ;;
            3)
                expected_tokens=("$SUBJECT_S3" "120*1300=" "Done" "$MARKER_S3")
                mail_subject_for_verify="$SUBJECT_S3"
                ;;
            4)
                expected_tokens=("$SUBJECT_S4" "focus music" "pomodoro timer" "daily review template" "$MARKER_S4")
                mail_subject_for_verify="$SUBJECT_S4"
                ;;
            5)
                expected_tokens=("$SUBJECT_S5" "Base: 120 USD" "$MARKER_S5")
                mail_subject_for_verify="$SUBJECT_S5"
                ;;
        esac

        local semantic_checked=0
        for token in "${expected_tokens[@]}"; do
            [ -z "$token" ] && continue
            semantic_checked=$((semantic_checked + 1))
            normalized_token="$(normalize_semantic_token "$token")"
            location="$(token_presence_location "$token" "$CURRENT_SCENARIO_MARKER")"
            if semantic_location_missing "$location" && [ -n "$normalized_token" ] && [ "$normalized_token" != "$token" ]; then
                location="$(token_presence_location "$normalized_token" "$CURRENT_SCENARIO_MARKER")"
            fi
            if semantic_location_missing "$location"; then
                semantic_missing=$((semantic_missing + 1))
                semantic_lines="${semantic_lines}- 의미검증 ❌ \"${token}\" (location=${location})"$'\n'
            else
                semantic_lines="${semantic_lines}- 의미검증 ✅ \"${token}\" (location=${location})"$'\n'
            fi
        done
        semantic_lines="${semantic_lines}- 의미검증 토큰 수: ${semantic_checked}"$'\n'
        if [ -n "$CURRENT_SCENARIO_MARKER" ]; then
            semantic_lines="${semantic_lines}- 의미검증 run-scope marker: ${CURRENT_SCENARIO_MARKER}"$'\n'
        fi

        if [ "$semantic_missing" -gt 0 ]; then
            status="failed"
            semantic_lines="${semantic_lines}- 의미검증 실패로 최종 상태를 failed로 강등"$'\n'
        fi
    else
        semantic_lines="${semantic_lines}- 의미검증 비활성(STEER_SEMANTIC_VERIFY=0)"$'\n'
    fi

    if [ "${STEER_REQUIRE_MAIL_SEND:-1}" = "1" ]; then
        local mail_send_logged=0
        if grep -Eiq "Shortcut 'd'.*shift.*Mail sent|Mail send completed|\"send_status\"[[:space:]]*:[[:space:]]*\"sent_confirmed\"|mail sent" "$log_file"; then
            mail_send_logged=1
        fi
        local outgoing_count
        outgoing_count="$(mail_outgoing_count || echo -1)"
        local mail_verify_token="${CURRENT_SCENARIO_MARKER:-}"
        if [ -z "$mail_verify_token" ] && [ -n "$mail_subject_for_verify" ]; then
            mail_verify_token="$mail_subject_for_verify"
        fi
        local mail_sent_location="NOT_CHECKED"
        if [ -n "$mail_verify_token" ]; then
            mail_sent_location="$(token_presence_location "$mail_verify_token" "$CURRENT_SCENARIO_MARKER")"
        fi
        local mail_sent_ok=0
        case "$mail_sent_location" in
            MAIL_SENT_SUBJECT|MAIL_SENT_BODY)
                mail_sent_ok=1
                ;;
        esac
        local expected_recipient
        expected_recipient="$(printf '%s' "${STEER_EXPECT_MAIL_RECIPIENT:-$MAIL_TO_TARGET}" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"
        local mail_recipient_location="RECIPIENT_UNSET"
        local mail_recipient_ok=0
        if printf '%s' "$expected_recipient" | grep -Eq '.+@.+\..+'; then
            mail_recipient_location="$(mail_sent_recipient_location "$expected_recipient" "$CURRENT_SCENARIO_MARKER")"
            if [ "$mail_recipient_location" = "MAIL_SENT_RECIPIENT" ]; then
                mail_recipient_ok=1
            fi
        fi
        if [ "$mail_send_logged" -eq 1 ] && [ "$mail_sent_ok" -eq 1 ] && [ "$mail_recipient_ok" -eq 1 ]; then
            semantic_lines="${semantic_lines}- 메일 발송 검증 ✅ (send-action 로그 + sent mailbox 확인 + recipient=${expected_recipient}, outgoing=${outgoing_count})"$'\n'
        else
            semantic_lines="${semantic_lines}- 메일 발송 검증 ❌ (send-action 로그=${mail_send_logged}, outgoing=${outgoing_count}, sent_location=${mail_sent_location}, recipient=${expected_recipient:-none}, recipient_location=${mail_recipient_location}, token=${mail_verify_token:-none})"$'\n'
            status="failed"
        fi
    fi
    
    # Derive result from judged status, not emoji presence in logs.
    local result_info="요청 체인이 완료 판정되었습니다."
    if [ "$status" != "success" ]; then
        result_info="요청 체인이 실패 판정되었습니다."
    fi

    # Build concise evidence lines from log for detailed Telegram report.
    local key_logs=""
    key_logs=$(grep -En "Goal completed by planner|Surf failed|Supervisor escalated|Preflight failed|Execution Error|SCHEMA_ERROR|PLAN_REJECTED|LLM Refused|fallback action|FALLBACK_ACTION:|Node evidence" "$log_file" 2>/dev/null | tail -n 8 | sed -E 's/^[0-9]+://')
    if [ -z "$key_logs" ]; then
        key_logs=$(tail -n 3 "$log_file" 2>/dev/null | sed -E 's/^[[:space:]]+//')
    fi

    local evidence_lines=""
    local fallback_hit=0
    if grep -Eiq "fallback action|FALLBACK_ACTION:" "$log_file" 2>/dev/null; then
        fallback_hit=1
    fi
    while IFS= read -r line; do
        if [ -n "$line" ]; then
            evidence_lines="${evidence_lines}- ${line}"$'\n'
        fi
    done <<< "$key_logs"
    if [ -z "$evidence_lines" ]; then
        evidence_lines="- (핵심 로그 없음)"$'\n'
    fi
    evidence_lines="${evidence_lines}- 판정 기준: 종료코드 + 치명 로그 패턴 검사"$'\n'
    evidence_lines="${evidence_lines}- STEER_SCENARIO_MODE=${SCENARIO_MODE_VALUE}"$'\n'
    evidence_lines="${evidence_lines}- STEER_NODE_CAPTURE_ALL=${NODE_CAPTURE_ALL_VALUE}"$'\n'
    if [ "$fallback_hit" -eq 1 ]; then
        evidence_lines="${evidence_lines}- fallback 액션 감지됨(fallback action/FALLBACK_ACTION)"$'\n'
        if [ "$FAIL_ON_FALLBACK_VALUE" = "1" ]; then
            evidence_lines="${evidence_lines}- 정책상 fallback 감지 시 실패 처리(STEER_FAIL_ON_FALLBACK=1)"$'\n'
        fi
    fi
    evidence_lines="${evidence_lines}${semantic_lines}"

    local node_dir="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}_nodes"
    local node_count=0
    if [ -d "$node_dir" ]; then
        node_count=$(find "$node_dir" -maxdepth 1 -type f -name '*.png' | wc -l | tr -d ' ')
    fi
    evidence_lines="${evidence_lines}- 노드 캡처 수: ${node_count}"$'\n'
    evidence_lines="${evidence_lines}- 노드 캡처 폴더: $(basename "$node_dir")"$'\n'
    local node_image_list_file="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}.telegram.node_images.txt"
    : > "$node_image_list_file"
    local node_step_summary=""
    local node_step_count=0

    if [ "$node_count" -gt 0 ] && [ -f "$log_file" ]; then
        local node_last_rows=""
        node_last_rows=$(awk '
            /Node evidence:/ {
                line = $0
                path = app = step = action = phase = note = ""
                sub(/^.*Node evidence: /, "", line)
                split(line, parts, " \\| ")
                path = parts[1]
                meta = parts[2]

                n = split(meta, kv, " ")
                for (i = 1; i <= n; i++) {
                    if (index(kv[i], "step=") == 1) step = substr(kv[i], 6)
                    else if (index(kv[i], "action=") == 1) action = substr(kv[i], 8)
                    else if (index(kv[i], "phase=") == 1) phase = substr(kv[i], 7)
                    else if (index(kv[i], "front_app=") == 1) app = substr(kv[i], 11)
                    else if (index(kv[i], "note=") == 1) note = substr(kv[i], 6)
                }
                gsub(/^ +| +$/, "", path)
                if (path != "") {
                    step_num = step + 0
                    key = sprintf("%06d_%s", step_num, action)
                    idx[key] = NR
                    payload[key] = path "|" step "|" action "|" phase "|" app "|" note
                }
            }
            END {
                for (key in idx) {
                    print key "|" idx[key] "|" payload[key]
                }
            }
        ' "$log_file" | sort -t'|' -k1,1 -k2,2n)

        if [ -n "$node_last_rows" ]; then
            while IFS= read -r row; do
                [ -z "$row" ] && continue
                IFS='|' read -r _step_key _ord path step action phase app note <<< "$row"
                local node_status="✅ 실행"
                if [[ "$phase" == *error* ]] || [[ "$note" == *failed* ]]; then
                    node_status="❌ 실행오류"
                fi
                node_step_count=$((node_step_count + 1))
                local node_label="step ${step}, action ${action}"
                if [ -n "$app" ]; then
                    node_label="${node_label}, app ${app}"
                fi
                node_step_summary="${node_step_summary}- ${node_label}: ${node_status}"$'\n'
                if [ -f "$path" ]; then
                    telegram_main_image="$path"
                    local node_caption
                    node_caption="단계 최종결과 | 시나리오:${scenario_num} | step:${step} | action:${action} | app:${app:-unknown} | ${node_status}"
                    printf '%s|%s\n' "$path" "$node_caption" >> "$node_image_list_file"
                fi
            done <<< "$node_last_rows"
        fi
    fi

    if [ -n "$node_step_summary" ]; then
        evidence_lines="${evidence_lines}- 단계별 마지막 결과"$'\n'"${node_step_summary}"
    fi
    evidence_lines="${evidence_lines}- 단계별 요약 수: ${node_step_count}"$'\n'
    evidence_lines="${evidence_lines}- 단계 상태는 '액션 실행 여부' 기준이며, 내용 충족 여부는 의미검증 라인 기준"$'\n'

    if [ -s "$node_image_list_file" ]; then
        telegram_main_image=""
        evidence_lines="${evidence_lines}- 단계별 실제 앱 캡처를 텔레그램에 첨부"$'\n'
    else
        telegram_main_image=""
        if run_cmd_with_timeout_capture "${STEER_SCREENSHOT_TIMEOUT_SEC:-6}" screencapture -x "$fallback_screenshot"; then
            telegram_main_image="$fallback_screenshot"
            evidence_lines="${evidence_lines}- 단계 캡처가 없어 fallback 전체화면 캡처를 첨부"$'\n'
        else
            evidence_lines="${evidence_lines}- 단계 캡처/ fallback 캡처 모두 실패"$'\n'
        fi
    fi

    local status_label="❌ 실패"
    if [ "$status" = "success" ]; then
        status_label="✅ 성공"
    fi

    local telegram_message
    telegram_message=$(cat <<EOF
작업: 시나리오 ${scenario_num} - ${scenario_name}
요청: ${scenario_goal}
수행: 자동 시나리오 실행 및 결과 캡처/검증
결과: ${result_info}
상태: ${status_label}
근거:
${evidence_lines}- 로그: $(basename "$log_file")
EOF
)

    # Keep local audit copy of the raw pre-rewrite message.
    local raw_message_file="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}.telegram.raw.txt"
    printf '%s\n' "$telegram_message" > "$raw_message_file"

    # Path where send script writes the final rewritten text actually sent.
    local final_message_file="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}.telegram.final.txt"

    # Send Telegram notification if helper exists and env vars are configured.
    local notifier="./send_telegram_notification.sh"
    if [ -f "$notifier" ]; then
        if [ -n "${TELEGRAM_BOT_TOKEN:-}" ] && [ -n "${TELEGRAM_CHAT_ID:-}" ]; then
            local telegram_send_ok=1
            local notify_env=()
            if [ -s "$node_image_list_file" ]; then
                notify_env=(TELEGRAM_EXTRA_IMAGE_LIST_FILE="$node_image_list_file")
            fi
            if [ -n "$telegram_main_image" ] && [ -f "$telegram_main_image" ]; then
                if ! send_telegram_with_timeout "$NOTIFIER_TIMEOUT_SEC" \
                    env TELEGRAM_DUMP_FINAL_PATH="$final_message_file" TELEGRAM_SKIP_REWRITE=1 "${notify_env[@]}" \
                    bash "$notifier" "$telegram_message" "$telegram_main_image" >/dev/null 2>&1; then
                    telegram_send_ok=0
                fi
            else
                if ! send_telegram_with_timeout "$NOTIFIER_TIMEOUT_SEC" \
                    env TELEGRAM_DUMP_FINAL_PATH="$final_message_file" TELEGRAM_SKIP_REWRITE=1 "${notify_env[@]}" \
                    bash "$notifier" "$telegram_message" >/dev/null 2>&1; then
                    telegram_send_ok=0
                fi
            fi
            if [ "$telegram_send_ok" -ne 1 ]; then
                printf '%s\n- 텔레그램 전송 실패(타임아웃/오류)\n' "$telegram_message" > "$final_message_file"
                status="failed"
            fi
        else
            echo "Warning: TELEGRAM_BOT_TOKEN/TELEGRAM_CHAT_ID not set; skipped Telegram notification." >&2
        fi
    else
        echo "Warning: send_telegram_notification.sh not found; skipped Telegram notification." >&2
    fi
    
    echo "Scenario ${scenario_num} finished with status: ${status}"
    echo "  - telegram raw: ${raw_message_file}"
    echo "  - telegram final: ${final_message_file}"
}

if ! preflight_checks; then
    exit 1
fi
echo ""

# Scenario 1: Calendar -> Safari -> Notes -> Mail
echo "---------------------------------------------------"
echo "📅 Scenario 1: Calendar → Safari → Notes → Mail"
LOG_FILE="scenario_results/complex_scenario_1_${TIMESTAMP}.log"
SCENARIO_GOAL="Multi-app draft chain without screen-reading dependency."
CURRENT_SCENARIO_MARKER="$MARKER_S1"
echo "Goal: ${SCENARIO_GOAL}"
CMD="Calendar를 열고 전면으로 가져오세요. Notes를 열어 새 메모(Cmd+N)를 만들고 제목을 \"${SUBJECT_S1}\"로 입력한 뒤 아래 3줄을 그대로 입력하세요: \"Calendar opened\", \"Notes draft ready\", \"Mail prep pending\". 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. TextEdit를 열어 새 문서(Cmd+N)에 붙여넣기(Cmd+V)하고 다음 줄에 \"Shared via TextEdit\"를 입력하세요. 다음 줄에 \"${MARKER_S1}\"를 정확히 입력하세요. 다시 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. Mail을 열어 새 이메일(Cmd+N) 초안을 만들고 제목 \"${SUBJECT_S1}\"를 입력한 뒤 본문에 붙여넣기(Cmd+V)하세요. 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요."

if run_agent_scenario "$CMD" "$LOG_FILE" 1; then
    echo "✅ Scenario 1 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 1 "일정 브리핑 체인" "success" "$LOG_FILE" "$SCENARIO_GOAL"
else
    echo "❌ Scenario 1 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 1 "일정 브리핑 체인" "failed" "$LOG_FILE" "$SCENARIO_GOAL"
fi
sleep 5

# Scenario 2: Finder -> TextEdit -> Notes
echo "---------------------------------------------------"
echo "📂 Scenario 2: Finder → TextEdit → Notes"
LOG_FILE="scenario_results/complex_scenario_2_${TIMESTAMP}.log"
SCENARIO_GOAL="Finder/TextEdit/Notes/Mail transfer chain."
CURRENT_SCENARIO_MARKER="$MARKER_S2"
echo "Goal: ${SCENARIO_GOAL}"
CMD="Finder를 열어 Downloads 폴더로 이동하세요. TextEdit를 열어 새 문서(Cmd+N)를 만들고 제목 \"${SUBJECT_S2}\"를 입력한 뒤 아래 3줄을 그대로 입력하세요: \"1. invoice.pdf\", \"2. screenshot.png\", \"3. notes.txt\". 다음 줄에 \"${MARKER_S2}\"를 정확히 입력하세요. 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. Notes를 열어 새 메모(Cmd+N)를 만들고 붙여넣기(Cmd+V)하세요. 다시 전체 선택(Cmd+A) 후 복사(Cmd+C)하고 Mail을 열어 새 이메일(Cmd+N) 초안을 만든 뒤 제목 \"${SUBJECT_S2}\"를 입력하고 본문에 붙여넣기(Cmd+V)하세요. 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요."

if run_agent_scenario "$CMD" "$LOG_FILE" 2; then
    echo "✅ Scenario 2 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 2 "다운로드 분류 체인" "success" "$LOG_FILE" "$SCENARIO_GOAL"
else
    echo "❌ Scenario 2 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 2 "다운로드 분류 체인" "failed" "$LOG_FILE" "$SCENARIO_GOAL"
fi
sleep 5

# Scenario 3: Safari -> Calculator -> Notes
echo "---------------------------------------------------"
echo "📈 Scenario 3: Safari → Calculator → Notes"
LOG_FILE="scenario_results/complex_scenario_3_${TIMESTAMP}.log"
SCENARIO_GOAL="Browser + calculation + document handoff chain."
CURRENT_SCENARIO_MARKER="$MARKER_S3"
echo "Goal: ${SCENARIO_GOAL}"
CMD="Safari를 열고 https://www.google.com 으로 이동하세요. 새 탭(Cmd+T)을 열고 https://www.wikipedia.org 로 이동하세요. Calculator를 열어 \"120*1300=\" 을 입력해 계산한 뒤 복사(Cmd+C)하세요. Notes를 열어 새 메모(Cmd+N)를 만들고 제목 \"${SUBJECT_S3}\"를 입력한 뒤 다음 줄에 \"120*1300=\"를 입력하고 다음 줄에 붙여넣기(Cmd+V)하세요. TextEdit를 열어 새 문서(Cmd+N)에 방금 메모 내용을 붙여넣기(Cmd+V)하고 마지막 줄에 \"Done\"을 입력하세요. 다음 줄에 \"${MARKER_S3}\"를 정확히 입력하세요. Mail을 열어 새 이메일(Cmd+N) 초안을 만들고 제목 \"${SUBJECT_S3}\"를 입력한 뒤 본문에 붙여넣기(Cmd+V)하세요. 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요."

if run_agent_scenario "$CMD" "$LOG_FILE" 3; then
    echo "✅ Scenario 3 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 3 "주가 비교 체인" "success" "$LOG_FILE" "$SCENARIO_GOAL"
else
    echo "❌ Scenario 3 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 3 "주가 비교 체인" "failed" "$LOG_FILE" "$SCENARIO_GOAL"
fi
sleep 5

# Scenario 4: Notes -> Safari -> TextEdit
echo "---------------------------------------------------"
echo "🧠 Scenario 4: Notes → Safari → TextEdit"
LOG_FILE="scenario_results/complex_scenario_4_${TIMESTAMP}.log"
SCENARIO_GOAL="Idea note -> web query -> report -> mail draft chain."
CURRENT_SCENARIO_MARKER="$MARKER_S4"
echo "Goal: ${SCENARIO_GOAL}"
CMD="Notes를 열어 새 메모(Cmd+N)를 만들고 아래 3줄을 그대로 입력하세요: \"focus music\", \"pomodoro timer\", \"daily review template\". 다음 줄에 \"${MARKER_S4}\"를 정확히 입력하세요. 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. Safari를 열고 https://www.google.com 으로 이동한 뒤 붙여넣기(Cmd+V)하고 Enter를 누르세요. 주소창에 포커스(Cmd+L) 후 복사(Cmd+C)하세요. TextEdit를 열어 새 문서(Cmd+N)에 \"${SUBJECT_S4}\" 제목을 입력하고 다음 줄에 붙여넣기(Cmd+V)하세요. Mail을 열어 새 이메일(Cmd+N) 초안을 만들고 제목 \"${SUBJECT_S4}\"를 입력한 뒤 본문에 붙여넣기(Cmd+V)하세요. 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요."

if run_agent_scenario "$CMD" "$LOG_FILE" 4; then
    echo "✅ Scenario 4 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 4 "아이디어 리서치 체인" "success" "$LOG_FILE" "$SCENARIO_GOAL"
else
    echo "❌ Scenario 4 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 4 "아이디어 리서치 체인" "failed" "$LOG_FILE" "$SCENARIO_GOAL"
fi
sleep 5

# Scenario 5: Safari -> Calculator -> Notes -> Mail
echo "---------------------------------------------------"
echo "💱 Scenario 5: Safari → Calculator → Notes → Mail"
LOG_FILE="scenario_results/complex_scenario_5_${TIMESTAMP}.log"
SCENARIO_GOAL="Finder/Calculator/Notes/Mail budget draft chain."
CURRENT_SCENARIO_MARKER="$MARKER_S5"
echo "Goal: ${SCENARIO_GOAL}"
CMD="Finder를 열어 Desktop으로 이동하세요. Calculator를 열어 \"120*1450=\" 을 입력해 계산하고 결과를 복사(Cmd+C)하세요. Notes를 열어 새 메모(Cmd+N)를 만들고 제목 \"${SUBJECT_S5}\"를 입력한 뒤 다음 줄에 \"Base: 120 USD\"를 입력하고 다음 줄에 붙여넣기(Cmd+V)하세요. 다음 줄에 \"${MARKER_S5}\"를 정확히 입력하세요. 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. Mail을 열어 새 이메일(Cmd+N) 초안을 만들고 제목 \"${SUBJECT_S5}\"를 입력한 다음 본문에 붙여넣기(Cmd+V)하세요. 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요."

if run_agent_scenario "$CMD" "$LOG_FILE" 5; then
    echo "✅ Scenario 5 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 5 "환율 예산 체인" "success" "$LOG_FILE" "$SCENARIO_GOAL"
else
    echo "❌ Scenario 5 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 5 "환율 예산 체인" "failed" "$LOG_FILE" "$SCENARIO_GOAL"
fi

echo ""
echo "📊 Summary: success=${SUCCESS_COUNT}, failed=${FAIL_COUNT}"
if [ "$FAIL_COUNT" -gt 0 ]; then
    echo "⚠️  Completed with failures."
    exit 1
fi
echo "🎉 All 5 Complex Scenarios Succeeded."
