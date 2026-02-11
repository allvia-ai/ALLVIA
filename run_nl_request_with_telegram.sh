#!/bin/bash
set -e

# Usage:
#   ./run_nl_request_with_telegram.sh "자연어 요청" ["작업 이름"]
#
# Behavior:
# - Runs local_os_agent surf with the given request
# - Stores run log/screenshot
# - Builds detailed Korean report
# - Sends Telegram notification (with final sent text audit file)

REQUEST_TEXT="$1"
TASK_NAME="${2:-자연어 요청 실행}"

if [ -z "$REQUEST_TEXT" ]; then
    echo "Usage: $0 \"자연어 요청\" [\"작업 이름\"]"
    exit 1
fi

# Load environment variables
if [ -f core/.env ]; then
    set -a
    # shellcheck disable=SC1091
    source core/.env
    set +a
fi

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
        NOT_FOUND|CHECK_ERROR|CHECK_TIMEOUT|"")
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

preflight_checks() {
    local failed=0
    local ax_out=""
    local capture_out=""
    local preflight_capture="/tmp/nl_preflight_$$.png"
    local preflight_timeout="${STEER_PREFLIGHT_TIMEOUT_SEC:-6}"

    if ! require_terminal_context; then
        return 1
    fi

    if ! command -v osascript >/dev/null 2>&1; then
        echo "❌ Preflight failed: osascript not found."
        failed=1
    elif ! run_cmd_with_timeout_capture "$preflight_timeout" osascript -e 'tell application "System Events" to return name of first application process'; then
        ax_out="${RUN_TIMEOUT_STDERR:-$RUN_TIMEOUT_STDOUT}"
        if [ "$RUN_TIMEOUT_EXIT" -eq 124 ]; then
            echo "❌ Preflight failed: Accessibility permission check timed out (${preflight_timeout}s)."
        else
            echo "❌ Preflight failed: Accessibility permission check failed."
        fi
        [ -n "$ax_out" ] && echo "   Details: $ax_out"
        failed=1
    fi

    if ! command -v screencapture >/dev/null 2>&1; then
        echo "❌ Preflight failed: screencapture command not found."
        failed=1
    elif ! run_cmd_with_timeout_capture "$preflight_timeout" screencapture -x "$preflight_capture"; then
        capture_out="${RUN_TIMEOUT_STDERR:-$RUN_TIMEOUT_STDOUT}"
        if [ "$RUN_TIMEOUT_EXIT" -eq 124 ]; then
            echo "❌ Preflight failed: Screen Recording/display capture timed out (${preflight_timeout}s)."
        else
            echo "❌ Preflight failed: Screen Recording/display capture unavailable."
        fi
        [ -n "$capture_out" ] && echo "   Details: $capture_out"
        failed=1
    else
        rm -f "$preflight_capture"
    fi

    if [ "$failed" -ne 0 ]; then
        echo "⛔ Preflight checks failed. Aborting run."
        return 1
    fi
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

extract_expected_tokens_from_request() {
    {
        printf '%s\n' "$REQUEST_TEXT" | perl -ne '
            while (/"([^"]+)"|'\''([^'\'']+)'\''/g) {
                my $s = defined($1) && $1 ne "" ? $1 : $2;
                $s =~ s/^\s+|\s+$//g;
                next if length($s) < 3;
                print "$s\n";
            }
        '
        # Also capture non-quoted status/value style requirements.
        printf '%s\n' "$REQUEST_TEXT" | perl -ne '
            while (/(?:status|상태)\s*:\s*([A-Za-z0-9 _-]{3,80})/ig) {
                my $s = $1;
                $s =~ s/^\s+|\s+$//g;
                next if length($s) < 3;
                print "$s\n";
            }
        '
    } | awk '!seen[$0]++'
}

is_noise_token() {
    local token="$1"
    if [[ "$token" =~ ^(Cmd\+|cmd\+|command\+|shortcut|done|Done)$ ]]; then
        return 0
    fi
    if [[ "$token" =~ ^https?:// ]]; then
        return 0
    fi
    return 1
}

token_presence_location() {
    local token="$1"
    local result=""
    local timeout_sec="${STEER_OSASCRIPT_TIMEOUT_SEC:-8}"
    local tmp_out=""
    local tmp_err=""
    local osa_pid=""
    tmp_out="$(mktemp -t steer_osa_out.XXXXXX)"
    tmp_err="$(mktemp -t steer_osa_err.XXXXXX)"

    (
        osascript - "$token" <<'APPLESCRIPT'
on run argv
    set tokenText to item 1 of argv

    try
        tell application "Notes"
            if (count of accounts) > 0 then
                set latestNote to missing value
                set latestDate to date "January 1, 1970 at 00:00:00"
                repeat with ac in accounts
                    repeat with f in folders of ac
                        repeat with n in notes of f
                            try
                                set modDate to modification date of n
                            on error
                                set modDate to current date
                            end try
                            if latestNote is missing value or modDate > latestDate then
                                set latestNote to n
                                set latestDate to modDate
                            end if
                        end repeat
                    end repeat
                end repeat

                if latestNote is not missing value then
                    try
                        set nName to name of latestNote as text
                    on error
                        set nName to ""
                    end try
                    if nName contains tokenText then return "NOTE_TITLE"

                    try
                        set nBody to body of latestNote as text
                    on error
                        set nBody to ""
                    end try
                    if nBody contains tokenText then return "NOTE_BODY"
                end if
            end if
        end tell
    on error
        return "CHECK_ERROR"
    end try

    try
        tell application "Mail"
            set draftCount to count of outgoing messages
            if draftCount > 0 then
                set m to last outgoing message
                try
                    set s to subject of m as text
                on error
                    set s to ""
                end try
                if s contains tokenText then return "MAIL_SUBJECT"

                try
                    set c to content of m as text
                on error
                    set c to ""
                end try
                if c contains tokenText then return "MAIL_BODY"
            end if
        end tell
    on error
        return "CHECK_ERROR"
    end try

    try
        tell application "TextEdit"
            if (count of documents) > 0 then
                set d to front document
                try
                    set t to text of d as text
                on error
                    set t to ""
                end try
                if t contains tokenText then return "TEXTEDIT_BODY"
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

if ! preflight_checks; then
    exit 1
fi

mkdir -p scenario_results
TS=$(date +%Y%m%d_%H%M%S)
LOG_FILE="scenario_results/nl_request_${TS}.log"
FALLBACK_SCREENSHOT_FILE="scenario_results/nl_request_${TS}.png"
RAW_MSG_FILE="scenario_results/nl_request_${TS}.telegram.raw.txt"
FINAL_MSG_FILE="scenario_results/nl_request_${TS}.telegram.final.txt"
NODE_IMAGE_LIST_FILE="scenario_results/nl_request_${TS}.telegram.node_images.txt"
SCENARIO_MODE_VALUE="${STEER_SCENARIO_MODE:-0}"
NODE_CAPTURE_ALL_VALUE="${STEER_NODE_CAPTURE_ALL:-1}"
NODE_DIR="scenario_results/nl_request_${TS}_nodes"
CLI_LLM_VALUE="${STEER_CLI_LLM-}"
FAIL_ON_FALLBACK_VALUE="${STEER_FAIL_ON_FALLBACK:-1}"
NOTIFIER_TIMEOUT_SEC="${STEER_NOTIFIER_TIMEOUT_SEC:-25}"
REQUIRE_PRIMARY_PLANNER_VALUE="${STEER_REQUIRE_PRIMARY_PLANNER:-1}"
LOCK_DISABLED_VALUE="${STEER_LOCK_DISABLED:-0}"

if [ "$REQUIRE_PRIMARY_PLANNER_VALUE" = "1" ] && [ "$SCENARIO_MODE_VALUE" = "1" ] && [ "${STEER_ALLOW_SCENARIO_MODE:-0}" != "1" ]; then
    echo "❌ 정책 위반: STEER_SCENARIO_MODE=1 이지만 STEER_ALLOW_SCENARIO_MODE=1 승인 없이 fallback 모드 실행은 금지됩니다."
    echo "   운영 검증은 STEER_SCENARIO_MODE=0으로 실행하거나, 테스트 목적일 때만 STEER_ALLOW_SCENARIO_MODE=1을 설정하세요."
    exit 1
fi

FATAL_PATTERN='Failed to acquire lock|thread .* panicked|FATAL ERROR|⛔️|❌|LLM not available for surf mode|Preflight failed|Surf failed|Supervisor escalated|Execution Error|SCHEMA_ERROR|PLAN_REJECTED|LLM Refused'

echo "🚀 Running NL request..."
echo "Task: ${TASK_NAME}"
echo "Mode: STEER_SCENARIO_MODE=${SCENARIO_MODE_VALUE}"
echo "Node Capture: STEER_NODE_CAPTURE=1, STEER_NODE_CAPTURE_ALL=${NODE_CAPTURE_ALL_VALUE}"
echo "Fallback Policy: STEER_FAIL_ON_FALLBACK=${FAIL_ON_FALLBACK_VALUE}"
if [ -n "$CLI_LLM_VALUE" ]; then
    echo "CLI LLM: STEER_CLI_LLM=${CLI_LLM_VALUE}"
else
    echo "CLI LLM: disabled (using default OpenAI path)"
fi

get_idle_seconds() {
    local idle_raw
    idle_raw=$(ioreg -c IOHIDSystem 2>/dev/null | awk '/HIDIdleTime/ {print $NF; exit}')
    if [ -z "$idle_raw" ]; then
        return 1
    fi
    # HIDIdleTime is in nanoseconds.
    echo $((idle_raw / 1000000000))
    return 0
}

get_frontmost_app() {
    osascript -e 'tell application "System Events" to get name of first process whose frontmost is true' 2>/dev/null || true
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
    local use_guard="${STEER_PAUSE_ON_USER_INPUT:-1}"
    if [ "$use_guard" != "1" ]; then
        if [ -n "$CLI_LLM_VALUE" ]; then
            STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
                STEER_CLI_LLM="$CLI_LLM_VALUE" \
                STEER_NODE_CAPTURE=1 \
                STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
                STEER_NODE_CAPTURE_DIR="$NODE_DIR" \
                STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
                cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$REQUEST_TEXT" &> "$LOG_FILE"
        else
            STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
                STEER_NODE_CAPTURE=1 \
                STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
                STEER_NODE_CAPTURE_DIR="$NODE_DIR" \
                STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
                cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$REQUEST_TEXT" &> "$LOG_FILE"
        fi
        return $?
    fi

    local active_threshold="${STEER_INPUT_ACTIVE_THRESHOLD_SECONDS:-1}"
    local resume_idle="${STEER_IDLE_RESUME_SECONDS:-3}"
    local poll_interval="${STEER_INPUT_POLL_SECONDS:-1}"
    local paused=0
    local pause_count=0
    local run_pid

    echo "🛡️ User-input guard enabled (apps=${STEER_USER_ACTIVE_APPS:-Terminal,Codex,iTerm2}, active<=${active_threshold}s, resume>=${resume_idle}s)"

    if [ -n "$CLI_LLM_VALUE" ]; then
        STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
            STEER_CLI_LLM="$CLI_LLM_VALUE" \
            STEER_NODE_CAPTURE=1 \
            STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
            STEER_NODE_CAPTURE_DIR="$NODE_DIR" \
            STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
            cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$REQUEST_TEXT" &> "$LOG_FILE" &
    else
        STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
            STEER_NODE_CAPTURE=1 \
            STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
            STEER_NODE_CAPTURE_DIR="$NODE_DIR" \
            STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
            cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$REQUEST_TEXT" &> "$LOG_FILE" &
    fi
    run_pid=$!

    while kill -0 "$run_pid" 2>/dev/null; do
        local idle_sec=""
        idle_sec="$(get_idle_seconds || true)"
        if [ -n "$idle_sec" ]; then
            local front_app
            front_app="$(get_frontmost_app)"
            if [ "$paused" -eq 0 ] && [ "$idle_sec" -le "$active_threshold" ] && is_user_active_front_app "$front_app"; then
                # Pause root process and immediate children to avoid race with cargo/local_os_agent.
                kill -STOP "$run_pid" >/dev/null 2>&1 || true
                pkill -STOP -P "$run_pid" >/dev/null 2>&1 || true
                paused=1
                pause_count=$((pause_count + 1))
                echo "⏸️ [InputGuard] Paused run (front_app=${front_app}, idle=${idle_sec}s, count=${pause_count})"
                echo "⏸️ [InputGuard] Paused run (front_app=${front_app}, idle=${idle_sec}s, count=${pause_count})" >> "$LOG_FILE"
            elif [ "$paused" -eq 1 ] && [ "$idle_sec" -ge "$resume_idle" ]; then
                kill -CONT "$run_pid" >/dev/null 2>&1 || true
                pkill -CONT -P "$run_pid" >/dev/null 2>&1 || true
                paused=0
                echo "▶️ [InputGuard] Resumed run (idle=${idle_sec}s)"
                echo "▶️ [InputGuard] Resumed run (idle=${idle_sec}s)" >> "$LOG_FILE"
            fi
        fi
        sleep "$poll_interval"
    done

    wait "$run_pid"
    local exit_code=$?
    echo "🧾 [InputGuard] pause_count=${pause_count}"
    echo "🧾 [InputGuard] pause_count=${pause_count}" >> "$LOG_FILE"
    return $exit_code
}

STATUS="success"
if ! run_surf_with_input_guard; then
    STATUS="failed"
fi

if grep -Eq "$FATAL_PATTERN" "$LOG_FILE"; then
    STATUS="failed"
fi

FALLBACK_HIT=0
if grep -Eiq "fallback action|FALLBACK_ACTION:" "$LOG_FILE"; then
    FALLBACK_HIT=1
    if [ "$FAIL_ON_FALLBACK_VALUE" = "1" ]; then
        STATUS="failed"
    fi
fi

SEMANTIC_LINES=""
if [ "${STEER_SEMANTIC_VERIFY:-1}" = "1" ]; then
    RAW_TOKENS=()
    while IFS= read -r token; do
        RAW_TOKENS+=("$token")
    done < <(extract_expected_tokens_from_request)
    FILTERED_TOKENS=()
    for token in "${RAW_TOKENS[@]}"; do
        [ -z "$token" ] && continue
        if is_noise_token "$token"; then
            continue
        fi
        FILTERED_TOKENS+=("$token")
    done
    if [ "${#FILTERED_TOKENS[@]}" -gt 12 ]; then
        FILTERED_TOKENS=("${FILTERED_TOKENS[@]:0:12}")
    fi

    missing_count=0
    checked_count=0
    if [ "${#FILTERED_TOKENS[@]}" -eq 0 ]; then
        SEMANTIC_LINES="${SEMANTIC_LINES}- 의미 검증 토큰 없음(요청에서 추출된 핵심 문자열 기준)"$'\n'
    else
        for token in "${FILTERED_TOKENS[@]}"; do
            checked_count=$((checked_count + 1))
            normalized_token="$(normalize_semantic_token "$token")"
            location="$(token_presence_location "$token")"
            if semantic_location_missing "$location" && [ -n "$normalized_token" ] && [ "$normalized_token" != "$token" ]; then
                location="$(token_presence_location "$normalized_token")"
            fi
            if semantic_location_missing "$location"; then
                missing_count=$((missing_count + 1))
                SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 ❌ \"${token}\" (location=${location})"$'\n'
            else
                SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 ✅ \"${token}\" (location=${location})"$'\n'
            fi
        done
        SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 토큰 수: ${checked_count}"$'\n'
    fi

    if [ "$missing_count" -gt 0 ]; then
        STATUS="failed"
        SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 실패로 최종 상태를 failed로 강등"$'\n'
    fi
else
    SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 비활성(STEER_SEMANTIC_VERIFY=0)"$'\n'
fi

if printf '%s' "$REQUEST_TEXT" | grep -Eiq '보내|발송|send'; then
    mail_send_logged=0
    if grep -Eiq "Shortcut 'd'.*shift|send.*mail|mail sent" "$LOG_FILE"; then
        mail_send_logged=1
    fi
    outgoing_count="$(mail_outgoing_count || echo -1)"
    if [ "$mail_send_logged" -eq 1 ] && [ "$outgoing_count" = "0" ]; then
        SEMANTIC_LINES="${SEMANTIC_LINES}- 메일 발송 검증 ✅ (send-action 로그 + outgoing=0)"$'\n'
    else
        SEMANTIC_LINES="${SEMANTIC_LINES}- 메일 발송 검증 ❌ (send-action 로그=${mail_send_logged}, outgoing=${outgoing_count})"$'\n'
        STATUS="failed"
    fi
fi

KEY_LOGS=$(grep -En "Goal completed by planner|Surf failed|Supervisor escalated|Preflight failed|Execution Error|SCHEMA_ERROR|PLAN_REJECTED|LLM Refused|fallback action|FALLBACK_ACTION:|Node evidence" "$LOG_FILE" 2>/dev/null | tail -n 10 | sed -E 's/^[0-9]+://')
if [ -z "$KEY_LOGS" ]; then
    KEY_LOGS=$(tail -n 4 "$LOG_FILE" 2>/dev/null | sed -E 's/^[[:space:]]+//')
fi

EVIDENCE_LINES=""
while IFS= read -r line; do
    if [ -n "$line" ]; then
        EVIDENCE_LINES="${EVIDENCE_LINES}- ${line}"$'\n'
    fi
done <<< "$KEY_LOGS"
if [ -z "$EVIDENCE_LINES" ]; then
    EVIDENCE_LINES="- (핵심 로그 없음)"$'\n'
fi
EVIDENCE_LINES="${EVIDENCE_LINES}- 판정 기준: 종료코드 + 치명 로그 패턴 검사"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_SCENARIO_MODE=${SCENARIO_MODE_VALUE}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_NODE_CAPTURE_ALL=${NODE_CAPTURE_ALL_VALUE}"$'\n'
if [ "$FALLBACK_HIT" -eq 1 ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- fallback 액션 감지됨(fallback action/FALLBACK_ACTION)"$'\n'
    if [ "$FAIL_ON_FALLBACK_VALUE" = "1" ]; then
        EVIDENCE_LINES="${EVIDENCE_LINES}- 정책상 fallback 감지 시 실패 처리(STEER_FAIL_ON_FALLBACK=1)"$'\n'
    fi
fi
EVIDENCE_LINES="${EVIDENCE_LINES}${SEMANTIC_LINES}"

NODE_COUNT=0
if [ -d "$NODE_DIR" ]; then
    NODE_COUNT=$(find "$NODE_DIR" -maxdepth 1 -type f -name '*.png' | wc -l | tr -d ' ')
fi
EVIDENCE_LINES="${EVIDENCE_LINES}- 노드 캡처 수: ${NODE_COUNT}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- 노드 캡처 폴더: $(basename "$NODE_DIR")"$'\n'

NODE_STEP_SUMMARY=""
NODE_STEP_COUNT=0
TELEGRAM_MAIN_IMAGE=""
: > "$NODE_IMAGE_LIST_FILE"
if [ -f "$LOG_FILE" ]; then
    NODE_LAST_ROWS=$(awk '
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
    ' "$LOG_FILE" | sort -t'|' -k1,1 -k2,2n)

    if [ -n "$NODE_LAST_ROWS" ]; then
        while IFS= read -r row; do
            [ -z "$row" ] && continue
            IFS='|' read -r _step_key _ord path step action phase app note <<< "$row"
            node_status="✅ 실행"
            if [[ "$phase" == *error* ]] || [[ "$note" == *failed* ]]; then
                node_status="❌ 실행오류"
            fi
            NODE_STEP_COUNT=$((NODE_STEP_COUNT + 1))
            node_label="step ${step}, action ${action}"
            if [ -n "$app" ]; then
                node_label="${node_label}, app ${app}"
            fi
            NODE_STEP_SUMMARY="${NODE_STEP_SUMMARY}- ${node_label}: ${node_status}"$'\n'
            if [ -f "$path" ]; then
                TELEGRAM_MAIN_IMAGE="$path"
                node_caption="단계 최종결과 | step:${step} | action:${action} | app:${app:-unknown} | ${node_status}"
                printf '%s|%s\n' "$path" "$node_caption" >> "$NODE_IMAGE_LIST_FILE"
            fi
        done <<< "$NODE_LAST_ROWS"
    fi
fi

if [ -n "$NODE_STEP_SUMMARY" ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- 단계별 마지막 결과"$'\n'"${NODE_STEP_SUMMARY}"
fi
EVIDENCE_LINES="${EVIDENCE_LINES}- 단계별 요약 수: ${NODE_STEP_COUNT}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- 단계 상태는 '액션 실행 여부' 기준이며, 내용 충족 여부는 의미검증 라인 기준"$'\n'

if [ -s "$NODE_IMAGE_LIST_FILE" ]; then
    TELEGRAM_MAIN_IMAGE=""
    EVIDENCE_LINES="${EVIDENCE_LINES}- 단계별 실제 앱 캡처를 텔레그램에 첨부"$'\n'
else
    TELEGRAM_MAIN_IMAGE=""
    if run_cmd_with_timeout_capture "${STEER_SCREENSHOT_TIMEOUT_SEC:-6}" screencapture -x "$FALLBACK_SCREENSHOT_FILE"; then
        TELEGRAM_MAIN_IMAGE="$FALLBACK_SCREENSHOT_FILE"
        EVIDENCE_LINES="${EVIDENCE_LINES}- 단계 캡처가 없어 fallback 전체화면 캡처를 첨부"$'\n'
    else
        EVIDENCE_LINES="${EVIDENCE_LINES}- 단계 캡처/ fallback 캡처 모두 실패"$'\n'
    fi
fi

RESULT_TEXT="요청 실행이 완료 판정되었습니다."
STATUS_LABEL="✅ 성공"
if [ "$STATUS" != "success" ]; then
    RESULT_TEXT="요청 실행이 실패 판정되었습니다."
    STATUS_LABEL="❌ 실패"
fi

REQUEST_PREVIEW="$REQUEST_TEXT"
if [ ${#REQUEST_PREVIEW} -gt 480 ]; then
    REQUEST_PREVIEW="${REQUEST_PREVIEW:0:480}..."
fi

TELEGRAM_MESSAGE=$(cat <<EOF
작업: ${TASK_NAME}
요청: ${REQUEST_PREVIEW}
수행: 자연어 요청 실행 및 결과 캡처/검증
결과: ${RESULT_TEXT}
상태: ${STATUS_LABEL}
근거:
${EVIDENCE_LINES}- 로그: $(basename "$LOG_FILE")
EOF
)

printf '%s\n' "$TELEGRAM_MESSAGE" > "$RAW_MSG_FILE"

if [ -n "${TELEGRAM_BOT_TOKEN:-}" ] && [ -n "${TELEGRAM_CHAT_ID:-}" ] && [ -f "./send_telegram_notification.sh" ]; then
    TELEGRAM_SEND_OK=1
    EXTRA_NODE_ENV=()
    if [ -s "$NODE_IMAGE_LIST_FILE" ]; then
        EXTRA_NODE_ENV=(TELEGRAM_EXTRA_IMAGE_LIST_FILE="$NODE_IMAGE_LIST_FILE")
    fi

    if [ -n "$TELEGRAM_MAIN_IMAGE" ] && [ -f "$TELEGRAM_MAIN_IMAGE" ]; then
        if ! send_telegram_with_timeout "$NOTIFIER_TIMEOUT_SEC" \
            env TELEGRAM_DUMP_FINAL_PATH="$FINAL_MSG_FILE" TELEGRAM_SKIP_REWRITE=1 "${EXTRA_NODE_ENV[@]}" \
            bash ./send_telegram_notification.sh "$TELEGRAM_MESSAGE" "$TELEGRAM_MAIN_IMAGE" >/dev/null 2>&1; then
            TELEGRAM_SEND_OK=0
        fi
    else
        if ! send_telegram_with_timeout "$NOTIFIER_TIMEOUT_SEC" \
            env TELEGRAM_DUMP_FINAL_PATH="$FINAL_MSG_FILE" TELEGRAM_SKIP_REWRITE=1 "${EXTRA_NODE_ENV[@]}" \
            bash ./send_telegram_notification.sh "$TELEGRAM_MESSAGE" >/dev/null 2>&1; then
            TELEGRAM_SEND_OK=0
        fi
    fi
    if [ "$TELEGRAM_SEND_OK" -ne 1 ]; then
        STATUS="failed"
        printf '%s\n- 텔레그램 전송 실패(타임아웃/오류)\n' "$TELEGRAM_MESSAGE" > "$FINAL_MSG_FILE"
    fi
else
    echo "Warning: Telegram env or notifier missing. Skipped Telegram send." >&2
fi

echo ""
echo "Done."
echo "- status: ${STATUS}"
echo "- log: ${LOG_FILE}"
if [ -n "$TELEGRAM_MAIN_IMAGE" ]; then
    echo "- screenshot: ${TELEGRAM_MAIN_IMAGE}"
else
    echo "- screenshot: (none, node captures only)"
fi
echo "- telegram raw: ${RAW_MSG_FILE}"
echo "- telegram final: ${FINAL_MSG_FILE}"

if [ "$STATUS" = "success" ]; then
    exit 0
fi
exit 1
