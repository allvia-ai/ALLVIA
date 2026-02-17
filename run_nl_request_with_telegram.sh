#!/bin/bash
set -euo pipefail

# Usage:
#   ./run_nl_request_with_telegram.sh "자연어 요청" ["작업 이름"]
#
# Behavior:
# - Runs local_os_agent surf with the given request
# - Stores run log/screenshot
# - Builds detailed Korean report
# - Sends Telegram notification (with final sent text audit file)

if [ "$#" -lt 1 ]; then
    echo "Usage: $0 \"자연어 요청\" [\"작업 이름\"]"
    exit 1
fi

REQUEST_TEXT="${1:-}"
TASK_NAME="${2:-자연어 요청 실행}"
REQUEST_TEXT_EXEC="$REQUEST_TEXT"
REQUEST_TEXT_FOR_VERIFY="$REQUEST_TEXT"
RUN_SCOPE_MARKER=""

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

# Semantic contract policy defaults: Rust contract is the primary source of truth.
: "${STEER_SEMANTIC_SCHEMA_ONLY:=1}"
: "${STEER_SEMANTIC_REQUIRE_RUST_CONTRACT:=1}"
: "${STEER_SEMANTIC_ALLOW_HEURISTIC_FALLBACK:=0}"
: "${STEER_SEMANTIC_ENFORCE_RUST_ONLY:=1}"
: "${STEER_SEMANTIC_FAIL_ON_TRUNCATION:=1}"
: "${STEER_SEMANTIC_REQUIRE_APP_SCOPE:=1}"
COMPACT_SUCCESS_REPORT_VALUE="${STEER_TELEGRAM_COMPACT_SUCCESS:-1}"

require_terminal_context() {
    local require_terminal="${STEER_REQUIRE_TERMINAL:-1}"
    [ "$require_terminal" = "1" ] || return 0

    local term_program="${TERM_PROGRAM:-unknown}"
    local strict_term_program="${STEER_REQUIRE_TERMINAL_STRICT:-0}"
    if [ "$term_program" = "unknown" ] && [ "$strict_term_program" != "1" ]; then
        echo "⚠️ TERM_PROGRAM=unknown; terminal allowlist strict check skipped (set STEER_REQUIRE_TERMINAL_STRICT=1 to enforce)."
        return 0
    fi
    local allowed_programs="${STEER_ALLOWED_TERM_PROGRAMS:-Apple_Terminal,iTerm.app}"
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
        NOT_FOUND|CHECK_ERROR|CHECK_TIMEOUT|MARKER_REQUIRED|LOG_ONLY_BLOCKED*|"")
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

semantic_location_is_log() {
    case "${1:-}" in
        LOG_*)
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
    local focus_activate_out=""
    local focus_front_out=""
    local focus_front=""
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

    if [ "${STEER_PREFLIGHT_FOCUS_HANDOFF:-1}" = "1" ]; then
        local focus_retries="${STEER_PREFLIGHT_FOCUS_RETRIES:-3}"
        local focus_retry_sleep="${STEER_PREFLIGHT_FOCUS_RETRY_SLEEP_SEC:-0.25}"
        local focus_attempt=1
        local focus_ok=0
        if ! [[ "$focus_retries" =~ ^[0-9]+$ ]] || [ "$focus_retries" -lt 1 ]; then
            focus_retries=3
        fi

        while [ "$focus_attempt" -le "$focus_retries" ]; do
            if ! run_cmd_with_timeout_capture "$preflight_timeout" osascript -e 'tell application "Finder" to activate'; then
                focus_activate_out="${RUN_TIMEOUT_STDERR:-$RUN_TIMEOUT_STDOUT}"
            elif ! run_cmd_with_timeout_capture "$preflight_timeout" osascript -e 'tell application "System Events" to return name of first application process whose frontmost is true'; then
                focus_front_out="${RUN_TIMEOUT_STDERR:-$RUN_TIMEOUT_STDOUT}"
            else
                focus_front="$(printf '%s' "${RUN_TIMEOUT_STDOUT}" | tr -d '\r' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
                if [ "$focus_front" = "Finder" ]; then
                    focus_ok=1
                    break
                fi
                focus_front_out="expected=Finder actual=${focus_front:-unknown}"
            fi

            if [ "$focus_attempt" -lt "$focus_retries" ]; then
                sleep "$focus_retry_sleep"
            fi
            focus_attempt=$((focus_attempt + 1))
        done

        if [ "$focus_ok" -eq 1 ]; then
            echo "✅ Preflight: Focus handoff works (frontmost=Finder, attempt=${focus_attempt}/${focus_retries})."
        else
            echo "❌ Preflight failed: Focus handoff check failed after ${focus_retries} attempts."
            [ -n "$focus_activate_out" ] && echo "   activate details: $focus_activate_out"
            [ -n "$focus_front_out" ] && echo "   frontmost details: $focus_front_out"
            echo "   Fix: 실행 중 전면 앱 충돌을 막기 위해 전용 데스크톱/사용자 세션에서 실행하세요."
            failed=1
        fi
    else
        echo "ℹ️ Preflight: Focus handoff check disabled (STEER_PREFLIGHT_FOCUS_HANDOFF=0)."
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

compute_notifier_timeout() {
    local base_timeout="$1"
    local image_count="$2"
    local per_image_sec="${STEER_NOTIFIER_PER_IMAGE_SEC:-4}"

    if ! [[ "$base_timeout" =~ ^[0-9]+$ ]]; then
        base_timeout=120
    fi
    if ! [[ "$image_count" =~ ^[0-9]+$ ]]; then
        image_count=0
    fi
    if ! [[ "$per_image_sec" =~ ^[0-9]+$ ]]; then
        per_image_sec=4
    fi

    echo $((base_timeout + (image_count * per_image_sec)))
}

compress_telegram_report() {
    local message="$1"
    local max_chars="${STEER_TELEGRAM_REPORT_MAX_CHARS:-3300}"
    local max_evidence_lines="${STEER_TELEGRAM_EVIDENCE_MAX_LINES:-4}"
    if ! [[ "$max_chars" =~ ^[0-9]+$ ]]; then
        max_chars=3300
    fi
    if ! [[ "$max_evidence_lines" =~ ^[0-9]+$ ]]; then
        max_evidence_lines=4
    fi
    local compressed="$message"
    compressed="$(printf '%s\n' "$compressed" | awk -v max_lines="$max_evidence_lines" '
BEGIN { in_evidence=0; evidence_lines=0 }
{
    if ($0 ~ /^근거:/) { in_evidence=1; print; next }
    if (in_evidence == 0) { print; next }
    if (evidence_lines < max_lines) { print; evidence_lines++; next }
}
END {
    if (in_evidence == 1 && evidence_lines >= max_lines) {
        print "- ...(근거 축약, 상세는 로그/캡처 파일 참조)"
    }
}')"
    if [ "${#compressed}" -gt "$max_chars" ]; then
        compressed="${compressed:0:max_chars}"$'\n'"- ...(메시지 길이 축약)"
    fi
    printf '%s' "$compressed"
}

collect_diagnostic_event_lines() {
    local limit="${STEER_DIAGNOSTIC_EVENT_TAIL:-8}"
    local diag_path="${STEER_DIAGNOSTIC_EVENTS_PATH:-scenario_results/diagnostic_events.jsonl}"

    if ! [[ "$limit" =~ ^[0-9]+$ ]]; then
        limit=8
    fi
    if [ "$limit" -le 0 ]; then
        return 0
    fi
    if [ ! -f "$diag_path" ]; then
        return 0
    fi

    if command -v jq >/dev/null 2>&1; then
        tail -n 240 "$diag_path" 2>/dev/null \
            | jq -r '
                select(
                    .type == "run.attempt"
                    or .type == "telegram.send.retry"
                    or .type == "telegram.send.error"
                    or .type == "n8n.http.retry"
                    or .type == "scheduler.start.skipped"
                )
                | "- diag[" + (.type | tostring) + "] " + ((.payload | tostring) // "{}")
            ' 2>/dev/null \
            | tail -n "$limit"
    else
        tail -n "$limit" "$diag_path" 2>/dev/null | sed -E 's/^/- diag.raw /'
    fi
}

log_run_attempt() {
    local phase="$1"
    local status="$2"
    local details="$3"
    [ -z "${LOG_FILE:-}" ] && return 0
    local ts
    ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'RUN_ATTEMPT|phase=%s|status=%s|details=%s|ts=%s\n' \
        "$phase" "$status" "$details" "$ts" >> "$LOG_FILE"
    if command -v jq >/dev/null 2>&1; then
        local payload
        payload="$(jq -cn \
            --arg phase "$phase" \
            --arg status "$status" \
            --arg details "$details" \
            --arg ts "$ts" \
            '{type:"run.attempt",phase:$phase,status:$status,details:$details,ts:$ts}')"
        printf 'RUN_ATTEMPT_JSON|%s\n' "$payload" >> "$LOG_FILE"
    fi
}

run_attempt_phase_status_hit() {
    local log_file="$1"
    local phase="$2"
    local status="$3"
    [ -z "$log_file" ] && return 1
    [ ! -f "$log_file" ] && return 1

    if command -v jq >/dev/null 2>&1; then
        if awk 'index($0, "RUN_ATTEMPT_JSON|") == 1 { sub(/^RUN_ATTEMPT_JSON\|/, "", $0); print }' "$log_file" \
            | jq -er --arg phase "$phase" --arg status "$status" \
                'select(.type == "run.attempt" and .phase == $phase and .status == $status) | 1' >/dev/null 2>&1; then
            return 0
        fi
    fi

    grep -Eiq "^RUN_ATTEMPT\\|phase=${phase}\\|status=${status}(\\||$)" "$log_file"
}

SEMANTIC_CONTRACT_RUST_BIN=""
SEMANTIC_CONTRACT_RUST_ERROR=0
SEMANTIC_CONTRACT_RUST_ERROR_DETAIL=""

resolve_semantic_contract_rust_bin() {
    if [ -n "$SEMANTIC_CONTRACT_RUST_BIN" ] && [ -x "$SEMANTIC_CONTRACT_RUST_BIN" ]; then
        printf '%s\n' "$SEMANTIC_CONTRACT_RUST_BIN"
        return 0
    fi

    local candidates=(
        "./core/target/debug/semantic_contract_rs"
        "./core/target/release/semantic_contract_rs"
    )
    local candidate=""
    for candidate in "${candidates[@]}"; do
        if [ -x "$candidate" ]; then
            SEMANTIC_CONTRACT_RUST_BIN="$candidate"
            printf '%s\n' "$SEMANTIC_CONTRACT_RUST_BIN"
            return 0
        fi
    done

    if [ "${STEER_SEMANTIC_CONTRACT_AUTO_BUILD:-1}" != "1" ]; then
        return 1
    fi

    if (cd core && cargo build --quiet --bin semantic_contract_rs >/dev/null 2>&1); then
        if [ -x "./core/target/debug/semantic_contract_rs" ]; then
            SEMANTIC_CONTRACT_RUST_BIN="./core/target/debug/semantic_contract_rs"
            printf '%s\n' "$SEMANTIC_CONTRACT_RUST_BIN"
            return 0
        fi
    fi
    return 1
}

extract_semantic_contract_with_rust() {
    local mode="$1"
    local source_text="$2"
    if [ "${STEER_USE_RUST_SEMANTIC_CONTRACT:-1}" != "1" ]; then
        return 1
    fi
    local bin=""
    if ! bin="$(resolve_semantic_contract_rust_bin)"; then
        return 1
    fi
    "$bin" --mode "$mode" --request "$source_text" 2>/dev/null
}

semantic_require_rust_contract() {
    case "${STEER_SEMANTIC_SCHEMA_ONLY:-1}" in
        1|true|TRUE|yes|YES|on|ON)
            return 0
            ;;
    esac
    case "${STEER_SEMANTIC_REQUIRE_RUST_CONTRACT:-1}" in
        0|false|FALSE|no|NO|off|OFF)
            return 1
            ;;
        *)
            return 0
            ;;
    esac
}

semantic_allow_heuristic_fallback() {
    case "${STEER_SEMANTIC_ENFORCE_RUST_ONLY:-1}" in
        1|true|TRUE|yes|YES|on|ON)
            return 1
            ;;
    esac
    case "${STEER_SEMANTIC_SCHEMA_ONLY:-1}" in
        1|true|TRUE|yes|YES|on|ON)
            return 1
            ;;
    esac
    case "${STEER_SEMANTIC_ALLOW_HEURISTIC_FALLBACK:-0}" in
        1|true|TRUE|yes|YES|on|ON)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

SEMANTIC_HEURISTIC_HELPER="./scripts/semantic_tokens_heuristic.sh"
if [ -f "$SEMANTIC_HEURISTIC_HELPER" ]; then
    # shellcheck disable=SC1090
    source "$SEMANTIC_HEURISTIC_HELPER"
fi
if ! declare -f extract_expected_tokens_heuristic >/dev/null 2>&1; then
    extract_expected_tokens_heuristic() {
        return 0
    }
fi

extract_expected_tokens_from_request() {
    local source_text="${REQUEST_TEXT_FOR_VERIFY:-$REQUEST_TEXT}"
    SEMANTIC_CONTRACT_RUST_ERROR=0
    SEMANTIC_CONTRACT_RUST_ERROR_DETAIL=""
    local rust_tokens=""
    if rust_tokens="$(extract_semantic_contract_with_rust "tokens" "$source_text")"; then
        if [ -n "$rust_tokens" ]; then
            printf '%s\n' "$rust_tokens" | awk 'NF > 0 && !seen[$0]++'
            return 0
        fi
    fi
    if semantic_require_rust_contract; then
        SEMANTIC_CONTRACT_RUST_ERROR=1
        if [ "${STEER_USE_RUST_SEMANTIC_CONTRACT:-1}" != "1" ]; then
            SEMANTIC_CONTRACT_RUST_ERROR_DETAIL="STEER_USE_RUST_SEMANTIC_CONTRACT=1 required"
        elif ! resolve_semantic_contract_rust_bin >/dev/null 2>&1; then
            SEMANTIC_CONTRACT_RUST_ERROR_DETAIL="semantic_contract_rs unavailable"
        else
            SEMANTIC_CONTRACT_RUST_ERROR_DETAIL="semantic_contract_rs returned empty tokens"
        fi
        return 0
    fi
    if ! semantic_allow_heuristic_fallback; then
        SEMANTIC_CONTRACT_RUST_ERROR=1
        SEMANTIC_CONTRACT_RUST_ERROR_DETAIL="semantic_contract_rs unavailable and heuristic fallback disabled"
        return 0
    fi
    extract_expected_tokens_heuristic "$source_text"
}

extract_expected_tokens_override() {
    local raw="${STEER_SEMANTIC_EXPECT:-}"
    [ -z "$raw" ] && return 0
    printf '%s\n' "$raw" | perl -pe 's/\|\|/\n/g' \
        | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' \
        | awk 'NF > 0 && !seen[$0]++'
}

preflight_validate_semantic_tokens() {
    if [ "${STEER_SEMANTIC_VERIFY:-1}" != "1" ]; then
        return 0
    fi
    if [ "${STEER_SEMANTIC_REQUIRE_NONEMPTY:-1}" != "1" ]; then
        return 0
    fi

    local precheck_stream=""
    local token_count=0
    local token=""
    precheck_stream="$(extract_expected_tokens_from_request || true)"

    if [ "${SEMANTIC_CONTRACT_RUST_ERROR:-0}" = "1" ]; then
        echo "❌ Preflight failed: semantic contract token extraction failed (${SEMANTIC_CONTRACT_RUST_ERROR_DETAIL:-unknown})."
        return 1
    fi

    while IFS= read -r token; do
        [ -z "$token" ] && continue
        token="$(normalize_semantic_token "$token")"
        [ -z "$token" ] && continue
        if is_noise_token "$token"; then
            continue
        fi
        token_count=$((token_count + 1))
    done <<< "$precheck_stream"

    if [ -n "${RUN_SCOPE_MARKER:-}" ]; then
        token_count=$((token_count + 1))
    fi

    if [ "$token_count" -le 0 ]; then
        echo "❌ Preflight failed: 의미검증 토큰이 0개입니다 (요청/계약 확인 필요)."
        return 1
    fi

    echo "✅ Preflight: semantic token contract count=${token_count}"
    return 0
}

is_noise_token() {
    local token="$1"
    if [ "${#token}" -gt 120 ]; then
        return 0
    fi
    if [[ "$token" =~ ^(Cmd\+|cmd\+|command\+|shortcut|done)$ ]]; then
        return 0
    fi
    if [[ "$token" =~ ^https?:// ]]; then
        return 0
    fi
    if [[ "$token" =~ (열고|열어|붙여넣|복사|입력하|작성하|보내기|발송|하세요|해라|실행해) ]]; then
        return 0
    fi
    return 1
}

extract_expected_recipients_from_request() {
    local source_text="${REQUEST_TEXT_FOR_VERIFY:-$REQUEST_TEXT}"
    local rust_recipients=""
    if rust_recipients="$(extract_semantic_contract_with_rust "recipients" "$source_text")"; then
        if [ -n "$rust_recipients" ]; then
            printf '%s\n' "$rust_recipients" | awk 'NF > 0 && !seen[$0]++'
            return 0
        fi
        if semantic_require_rust_contract; then
            # Rust contract is the single source in schema mode.
            return 0
        fi
    elif semantic_require_rust_contract; then
        # In schema mode, avoid heuristic drift when Rust contract is unavailable.
        return 0
    fi
    printf '%s\n' "$source_text" | perl -ne '
        while (/[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}/g) {
            my $e = $&;
            $e =~ s/^[<\(\["'\'']+//;
            $e =~ s/[>\)\]"'\'',;:.]+$//;
            print lc($e), "\n";
        }
    ' | awk '!seen[$0]++'
}

request_requires_mail_send() {
    local source_text="${REQUEST_TEXT_FOR_VERIFY:-$REQUEST_TEXT}"
    local lower_text
    lower_text="$(printf '%s' "$source_text" | tr '[:upper:]' '[:lower:]')"

    local has_mail_context=0
    local has_send_intent=0
    local has_non_mail_send_context=0

    if printf '%s' "$lower_text" | grep -Eiq 'mail|gmail|email|이메일|메일|전자메일'; then
        has_mail_context=1
    fi
    if printf '%s' "$lower_text" | grep -Eiq '보내|발송|send'; then
        has_send_intent=1
    fi
    if printf '%s' "$lower_text" | grep -Eiq 'telegram|텔레그램|slack|디스코드|discord|notion|노션'; then
        has_non_mail_send_context=1
    fi

    local recipients=""
    recipients="$(extract_expected_recipients_from_request || true)"
    if [ -n "$recipients" ]; then
        return 0
    fi

    if [ "$has_mail_context" = "1" ] && [ "$has_send_intent" = "1" ]; then
        return 0
    fi
    if [ "$has_send_intent" = "1" ] && [ "$has_non_mail_send_context" = "0" ] && [ "$has_mail_context" = "1" ]; then
        return 0
    fi
    return 1
}

extract_latest_notes_target_from_log() {
    [ -f "$LOG_FILE" ] || return 0
    local line=""
    local note_id=""
    local note_name=""
    line="$(grep -E 'note_id=|"note_id"|note_name=|"note_name"' "$LOG_FILE" 2>/dev/null | tail -n 1 || true)"
    [ -z "$line" ] && return 0
    note_id="$(printf '%s\n' "$line" | perl -ne '
        if (/note_id=([^|[:space:]]+)/) { print $1; exit }
        if (/"note_id"\s*:\s*"([^"]+)"/) { print $1; exit }
    ')"
    note_name="$(printf '%s\n' "$line" | perl -ne '
        if (/note_name=([^|]+)/) { my $v=$1; $v =~ s/[[:space:]]+$//; print $v; exit }
        if (/"note_name"\s*:\s*"([^"]+)"/) { print $1; exit }
    ')"
    if [ -n "$note_id" ] || [ -n "$note_name" ]; then
        printf '%s|%s\n' "$note_id" "$note_name"
    fi
}

extract_latest_textedit_target_from_log() {
    [ -f "$LOG_FILE" ] || return 0
    local line=""
    local doc_id=""
    local doc_name=""
    line="$(grep -E 'doc_id=|"doc_id"|doc_name=|"doc_name"' "$LOG_FILE" 2>/dev/null | tail -n 1 || true)"
    [ -z "$line" ] && return 0
    doc_id="$(printf '%s\n' "$line" | perl -ne '
        if (/doc_id=([^|[:space:]]+)/) { print $1; exit }
        if (/"doc_id"\s*:\s*"([^"]+)"/) { print $1; exit }
    ')"
    doc_name="$(printf '%s\n' "$line" | perl -ne '
        if (/doc_name=([^|]+)/) { my $v=$1; $v =~ s/[[:space:]]+$//; print $v; exit }
        if (/"doc_name"\s*:\s*"([^"]+)"/) { print $1; exit }
    ')"
    if [ -n "$doc_id" ] || [ -n "$doc_name" ]; then
        printf '%s|%s\n' "$doc_id" "$doc_name"
    fi
}

notes_target_presence_location() {
    local note_id_target="${1:-}"
    local note_name_target="${2:-}"
    local marker="${3:-}"

    [ -z "$note_id_target" ] && [ -z "$note_name_target" ] && return 0

    osascript - "$note_id_target" "$note_name_target" "$marker" <<'APPLESCRIPT' 2>/dev/null || true
on run argv
    set noteIdTarget to item 1 of argv
    set noteNameTarget to item 2 of argv
    set markerText to item 3 of argv

    try
        tell application "Notes"
            if (count of accounts) > 0 then
                repeat with ac in accounts
                    repeat with fd in folders of ac
                        repeat with n in notes of fd
                            set cId to ""
                            set cName to ""
                            set cBody to ""
                            try
                                set cId to id of n as text
                            end try
                            try
                                set cName to name of n as text
                            end try
                            try
                                set cBody to body of n as text
                            end try
                            set idMatch to (noteIdTarget is not "" and cId is noteIdTarget)
                            set nameMatch to (noteNameTarget is not "" and cName is noteNameTarget)
                            if idMatch or nameMatch then
                                set scopeOk to (markerText is "" or cBody contains markerText or cName contains markerText)
                                if scopeOk then
                                    if idMatch then
                                        return "NOTE_TARGET_ID"
                                    else
                                        return "NOTE_TARGET_NAME"
                                    end if
                                end if
                            end if
                        end repeat
                    end repeat
                end repeat
            end if
        end tell
    end try

    return "NOT_FOUND"
end run
APPLESCRIPT
}

textedit_target_presence_location() {
    local doc_id_target="${1:-}"
    local doc_name_target="${2:-}"
    local marker="${3:-}"

    [ -z "$doc_id_target" ] && [ -z "$doc_name_target" ] && return 0

    osascript - "$doc_id_target" "$doc_name_target" "$marker" <<'APPLESCRIPT' 2>/dev/null || true
on run argv
    set docIdTarget to item 1 of argv
    set docNameTarget to item 2 of argv
    set markerText to item 3 of argv

    try
        tell application "TextEdit"
            set docCount to count of documents
            if docCount > 0 then
                repeat with idx from docCount to 1 by -1
                    set d to item idx of documents
                    set cId to ""
                    set cName to ""
                    set cText to ""
                    try
                        set cId to id of d as text
                    end try
                    try
                        set cName to name of d as text
                    end try
                    try
                        set cText to text of d as text
                    end try
                    set idMatch to (docIdTarget is not "" and cId is docIdTarget)
                    set nameMatch to (docNameTarget is not "" and cName is docNameTarget)
                    if idMatch or nameMatch then
                        set scopeOk to (markerText is "" or cText contains markerText)
                        if scopeOk then
                            if idMatch then
                                return "TEXTEDIT_TARGET_ID"
                            else
                                return "TEXTEDIT_TARGET_NAME"
                            end if
                        end if
                    end if
                end repeat
            end if
        end tell
    end try

    return "NOT_FOUND"
end run
APPLESCRIPT
}

token_presence_location_scoped_docs() {
    local token="$1"
    local marker="$2"
    local notes_target=""
    local textedit_target=""
    local note_id=""
    local note_name=""
    local doc_id=""
    local doc_name=""

    [ -z "$token" ] && return 0
    [ -z "$marker" ] && return 0

    notes_target="$(extract_latest_notes_target_from_log || true)"
    textedit_target="$(extract_latest_textedit_target_from_log || true)"

    if [ -n "$notes_target" ]; then
        IFS='|' read -r note_id note_name <<< "$notes_target"
    fi
    if [ -n "$textedit_target" ]; then
        IFS='|' read -r doc_id doc_name <<< "$textedit_target"
    fi

    if [ -z "$note_id" ] && [ -z "$note_name" ] && [ -z "$doc_id" ] && [ -z "$doc_name" ]; then
        return 0
    fi

    osascript - "$token" "$marker" "$note_id" "$note_name" "$doc_id" "$doc_name" <<'APPLESCRIPT' 2>/dev/null || true
on run argv
    set tokenText to item 1 of argv
    set markerText to item 2 of argv
    set noteIdTarget to item 3 of argv
    set noteNameTarget to item 4 of argv
    set docIdTarget to item 5 of argv
    set docNameTarget to item 6 of argv

    if noteIdTarget is not "" or noteNameTarget is not "" then
        try
            tell application "Notes"
                if (count of accounts) > 0 then
                    repeat with ac in accounts
                        repeat with fd in folders of ac
                            repeat with n in notes of fd
                                set cId to ""
                                set cName to ""
                                set cBody to ""
                                try
                                    set cId to id of n as text
                                end try
                                try
                                    set cName to name of n as text
                                end try
                                try
                                    set cBody to body of n as text
                                end try
                                set idMatch to (noteIdTarget is not "" and cId is noteIdTarget)
                                set nameMatch to (noteNameTarget is not "" and cName is noteNameTarget)
                                if idMatch or nameMatch then
                                    set scopeOk to (markerText is "" or cBody contains markerText or cName contains markerText)
                                    if scopeOk and cName contains tokenText then return "NOTE_ID_TITLE"
                                    if scopeOk and cBody contains tokenText then return "NOTE_ID_BODY"
                                end if
                            end repeat
                        end repeat
                    end repeat
                end if
            end tell
        end try
    end if

    if docIdTarget is not "" or docNameTarget is not "" then
        try
            tell application "TextEdit"
                set docCount to count of documents
                if docCount > 0 then
                    repeat with idx from docCount to 1 by -1
                        set d to item idx of documents
                        set cId to ""
                        set cName to ""
                        set cText to ""
                        try
                            set cId to id of d as text
                        end try
                        try
                            set cName to name of d as text
                        end try
                        try
                            set cText to text of d as text
                        end try
                        set idMatch to (docIdTarget is not "" and cId is docIdTarget)
                        set nameMatch to (docNameTarget is not "" and cName is docNameTarget)
                        if idMatch or nameMatch then
                            set scopeOk to (markerText is "" or cText contains markerText)
                            if scopeOk and cText contains tokenText then return "TEXTEDIT_ID_BODY"
                        end if
                    end repeat
                end if
            end tell
        end try
    end if

    return "NOT_FOUND"
end run
APPLESCRIPT
}

token_presence_location() {
    local token="$1"
    local marker="${2:-}"
    local run_start_epoch="${3:-0}"
    local require_marker="${STEER_SEMANTIC_REQUIRE_MARKER:-1}"
    local scan_limit="${STEER_SEMANTIC_SCAN_LIMIT:-40}"
    local result=""
    local timeout_sec="${STEER_SEMANTIC_OSASCRIPT_TIMEOUT_SEC:-30}"
    local tmp_out=""
    local tmp_err=""
    local osa_pid=""
    local log_location=""
    local scoped_doc_location=""
    local skip_global_doc_scan="${STEER_SEMANTIC_DISABLE_GLOBAL_DOC_SCAN:-1}"
    local skip_sent_mail_scan="${STEER_SEMANTIC_DISABLE_SENT_MAIL_SCAN:-1}"
    local allow_log_evidence="${STEER_SEMANTIC_ALLOW_LOG_EVIDENCE:-0}"

    if [ "$require_marker" = "1" ] && [ -z "$marker" ]; then
        printf '%s\n' "MARKER_REQUIRED"
        return 0
    fi

    scoped_doc_location="$(token_presence_location_scoped_docs "$token" "$marker")"
    if [ -n "$scoped_doc_location" ] && ! semantic_location_missing "$scoped_doc_location"; then
        printf '%s\n' "$scoped_doc_location"
        return 0
    fi
    tmp_out="$(mktemp -t steer_osa_out.XXXXXX)"
    tmp_err="$(mktemp -t steer_osa_err.XXXXXX)"

    (
        osascript - "$token" "$marker" "$scan_limit" "$run_start_epoch" "$skip_global_doc_scan" "$skip_sent_mail_scan" <<'APPLESCRIPT'
on run argv
    set tokenText to item 1 of argv
    set markerText to ""
    if (count of argv) > 1 then set markerText to item 2 of argv
    set scanLimit to 40
    if (count of argv) > 2 then
        try
            set scanLimit to (item 3 of argv) as integer
        on error
            set scanLimit to 40
        end try
    end if
    set runStartEpoch to 0
    if (count of argv) > 3 then
        try
            set runStartEpoch to (item 4 of argv) as integer
        on error
            set runStartEpoch to 0
        end try
    end if
    set skipDocScan to false
    if (count of argv) > 4 then
        set scanArg to item 5 of argv
        if scanArg is "1" or scanArg is "true" or scanArg is "yes" or scanArg is "on" then
            set skipDocScan to true
        end if
    end if
    set skipSentScan to false
    if (count of argv) > 5 then
        set sentArg to item 6 of argv
        if sentArg is "1" or sentArg is "true" or sentArg is "yes" or sentArg is "on" then
            set skipSentScan to true
        end if
    end if
    set nowEpoch to 0
    if runStartEpoch > 0 then
        try
            set nowEpoch to (do shell script "date +%s") as integer
        on error
            set nowEpoch to 0
        end try
    end if
    if scanLimit < 10 then set scanLimit to 10

    if skipDocScan is false then
        try
            tell application "Notes"
            if (count of accounts) > 0 then
                repeat with ac in accounts
                    repeat with f in folders of ac
                        set noteCount to count of notes of f
                        if noteCount > 0 then
                            -- Some Notes providers expose newest items at the beginning.
                            -- Scan both head and tail windows to avoid false negatives.
                            set headLimit to scanLimit
                            if headLimit > noteCount then set headLimit to noteCount
                            repeat with noteIdx from 1 to headLimit by 1
                                set n to item noteIdx of notes of f
                                set timeOk to true
                                if runStartEpoch > 0 then
                                    set timeOk to false
                                    try
                                        set modifiedAt to modification date of n
                                        set modifiedAgeSeconds to ((current date) - modifiedAt)
                                        if modifiedAgeSeconds < 0 then set modifiedAgeSeconds to 0
                                        set modifiedEpoch to nowEpoch - (round modifiedAgeSeconds rounding down)
                                        if modifiedEpoch ≥ runStartEpoch then set timeOk to true
                                    on error
                                        -- Notes metadata access can vary by account/provider.
                                        -- Degrade gracefully instead of forcing a false negative.
                                        set timeOk to true
                                    end try
                                end if
                                if timeOk is false and markerText is not "" then
                                    -- Marker scope is a stronger per-run signal than provider timestamps.
                                    set timeOk to true
                                end if
                                if timeOk then
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
                                end if
                            end repeat
                            if noteCount > headLimit then
                                set tailLower to noteCount - scanLimit + 1
                                if tailLower < (headLimit + 1) then set tailLower to (headLimit + 1)
                                repeat with noteIdx from noteCount to tailLower by -1
                                    set n to item noteIdx of notes of f
                                    set timeOk to true
                                    if runStartEpoch > 0 then
                                        set timeOk to false
                                        try
                                            set modifiedAt to modification date of n
                                            set modifiedAgeSeconds to ((current date) - modifiedAt)
                                            if modifiedAgeSeconds < 0 then set modifiedAgeSeconds to 0
                                            set modifiedEpoch to nowEpoch - (round modifiedAgeSeconds rounding down)
                                            if modifiedEpoch ≥ runStartEpoch then set timeOk to true
                                        on error
                                            set timeOk to true
                                        end try
                                    end if
                                    if timeOk is false and markerText is not "" then
                                        set timeOk to true
                                    end if
                                    if timeOk then
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
                                    end if
                                end repeat
                            end if
                        end if
                    end repeat
                end repeat
            end if
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
            -- TextEdit may not be active; continue to Mail scan.
        end try
    end if

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

            if skipSentScan is false then
                repeat with ac in accounts
                    try
                        set sentBoxes to {}
                        repeat with sentName in {"Sent Messages", "Sent Mail", "Sent", "보낸 편지함", "All Mail"}
                            try
                                set end of sentBoxes to (mailbox (sentName as text) of ac)
                            end try
                        end repeat
                        if (count of sentBoxes) = 0 then
                            try
                                set sentMbx to sent mailbox of ac
                                if sentMbx is not missing value then set end of sentBoxes to sentMbx
                            end try
                        end if
                        repeat with sentMbx in sentBoxes
                            set sentCount to count of messages of sentMbx
                            if sentCount > 0 then
                                set lowerBound to sentCount - scanLimit
                                if lowerBound < 1 then set lowerBound to 1
                                repeat with idx from sentCount to lowerBound by -1
                                    set sm to message idx of sentMbx
                                    set ss to ""
                                    set sc to ""
                                    set timeOk to true
                                    try
                                        set ss to subject of sm as text
                                    end try
                                    try
                                        set sc to content of sm as text
                                    end try
                                    if runStartEpoch > 0 then
                                        set timeOk to false
                                        set sentAt to missing value
                                        try
                                            set sentAt to date sent of sm
                                        on error
                                            try
                                                set sentAt to date received of sm
                                            end try
                                        end try
                                        if sentAt is not missing value and nowEpoch > 0 then
                                            set sentAgeSeconds to ((current date) - sentAt)
                                            if sentAgeSeconds < 0 then set sentAgeSeconds to 0
                                            set sentEpoch to nowEpoch - (round sentAgeSeconds rounding down)
                                            if sentEpoch ≥ runStartEpoch then set timeOk to true
                                        end if
                                    end if
                                    set sentScopeOk to (timeOk and (markerText is "" or sc contains markerText or ss contains markerText))
                                    if sentScopeOk and ss contains tokenText then return "MAIL_SENT_SUBJECT"
                                    if sentScopeOk and sc contains tokenText then return "MAIL_SENT_BODY"
                                end repeat
                            end if
                        end repeat
                    end try
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
    if semantic_location_missing "$result" && [ "$allow_log_evidence" = "1" ]; then
        log_location="$(token_presence_location_from_log "$token" "$marker" "$require_marker")"
        if [ -n "$log_location" ]; then
            result="$log_location"
        fi
    fi
    printf '%s\n' "$result"
}

token_presence_location_from_log() {
    local token="$1"
    local marker="${2:-}"
    local require_marker="${3:-1}"
    [ -z "$token" ] && return 0
    [ -f "$LOG_FILE" ] || return 0

    local lines=""
    lines="$(grep -F -- "$token" "$LOG_FILE" 2>/dev/null | tail -n 200 || true)"
    [ -z "$lines" ] && return 0
    if [ -n "$marker" ]; then
        lines="$(printf '%s\n' "$lines" | grep -F -- "$marker" || true)"
        [ -z "$lines" ] && return 0
    elif [ "$require_marker" = "1" ]; then
        return 0
    fi

    if printf '%s\n' "$lines" | grep -Eiq "MAIL_SEND_PROOF\\|.*subject=|EVIDENCE\\|target=mail\\|event=(send|write)\\|.*subject=|\\(mail subject\\)|MAIL_SUBJECT"; then
        printf '%s\n' "LOG_MAIL_SUBJECT"
        return 0
    fi
    if printf '%s\n' "$lines" | grep -Eiq "MAIL_SEND_PROOF\\|.*recipient=|EVIDENCE\\|target=mail\\|event=(send|write)\\|.*recipient=|\"recipient\"[[:space:]]*:"; then
        printf '%s\n' "LOG_MAIL_RECIPIENT"
        return 0
    fi
    if printf '%s\n' "$lines" | grep -Eiq "MAIL_SEND_PROOF\\|.*body_len=|EVIDENCE\\|target=mail\\|event=(send|write)\\|.*body_len=|\\(mail body\\)|MAIL_BODY"; then
        printf '%s\n' "LOG_MAIL_BODY"
        return 0
    fi
    if printf '%s\n' "$lines" | grep -Eiq "EVIDENCE\\|target=textedit\\|event=write\\|.*body_len=|\\(textedit body\\)|textedit_append_text|TEXTEDIT_BODY"; then
        printf '%s\n' "LOG_TEXTEDIT_BODY"
        return 0
    fi
    if printf '%s\n' "$lines" | grep -Eiq "EVIDENCE\\|target=notes\\|event=write\\|.*body_len=|\\(notes body\\)|notes_write_text|NOTE_BODY"; then
        printf '%s\n' "LOG_NOTE_BODY"
        return 0
    fi
    return 0
}

mail_sent_recipient_location() {
    local recipient="$1"
    local marker="${2:-}"
    local run_start_epoch="${3:-0}"
    local require_marker="${STEER_SEMANTIC_REQUIRE_MARKER:-1}"
    local scan_limit="${STEER_SEMANTIC_SCAN_LIMIT:-40}"
    local result=""
    local timeout_sec="${STEER_SEMANTIC_OSASCRIPT_TIMEOUT_SEC:-30}"
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
        osascript - "$recipient" "$marker" "$scan_limit" "$run_start_epoch" <<'APPLESCRIPT'
on run argv
    set recipientText to item 1 of argv
    set markerText to ""
    if (count of argv) > 1 then set markerText to item 2 of argv
    set scanLimit to 40
    if (count of argv) > 2 then
        try
            set scanLimit to (item 3 of argv) as integer
        on error
            set scanLimit to 40
        end try
    end if
    set runStartEpoch to 0
    if (count of argv) > 3 then
        try
            set runStartEpoch to (item 4 of argv) as integer
        on error
            set runStartEpoch to 0
        end try
    end if
    set nowEpoch to 0
    if runStartEpoch > 0 then
        try
            set nowEpoch to (do shell script "date +%s") as integer
        on error
            set nowEpoch to 0
        end try
    end if
    if scanLimit < 10 then set scanLimit to 10

    try
        tell application "Mail"
            repeat with ac in accounts
                try
                    set sentBoxes to {}
                    repeat with sentName in {"Sent Messages", "Sent Mail", "Sent", "보낸 편지함", "All Mail"}
                        try
                            set end of sentBoxes to (mailbox (sentName as text) of ac)
                        end try
                    end repeat
                    if (count of sentBoxes) = 0 then
                        try
                            set sentMbx to sent mailbox of ac
                            if sentMbx is not missing value then set end of sentBoxes to sentMbx
                        end try
                    end if
                    repeat with sentMbx in sentBoxes
                        set sentCount to count of messages of sentMbx
                        if sentCount > 0 then
                            set lowerBound to sentCount - scanLimit
                            if lowerBound < 1 then set lowerBound to 1
                            repeat with idx from sentCount to lowerBound by -1
                                set sm to message idx of sentMbx
                                set ss to ""
                                set sc to ""
                                set hasRecipient to false
                                set timeOk to true
                                try
                                    set ss to subject of sm as text
                                end try
                                try
                                    set sc to content of sm as text
                                end try
                                try
                                    repeat with r in to recipients of sm
                                        try
                                            set recipientAddress to (address of r as text)
                                            set recipientNorm to do shell script "printf %s " & quoted form of recipientAddress & " | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]'"
                                            set expectedNorm to do shell script "printf %s " & quoted form of recipientText & " | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]'"
                                            if recipientNorm is expectedNorm then
                                                set hasRecipient to true
                                                exit repeat
                                            end if
                                        end try
                                    end repeat
                                end try
                                if runStartEpoch > 0 then
                                    set timeOk to false
                                    set sentAt to missing value
                                    try
                                        set sentAt to date sent of sm
                                    on error
                                        try
                                            set sentAt to date received of sm
                                        end try
                                    end try
                                    if sentAt is not missing value and nowEpoch > 0 then
                                        set sentAgeSeconds to ((current date) - sentAt)
                                        if sentAgeSeconds < 0 then set sentAgeSeconds to 0
                                        set sentEpoch to nowEpoch - (round sentAgeSeconds rounding down)
                                        if sentEpoch ≥ runStartEpoch then set timeOk to true
                                    end if
                                end if
                                set scopeOk to (timeOk and (markerText is "" or sc contains markerText or ss contains markerText))
                                if scopeOk and hasRecipient then return "MAIL_SENT_RECIPIENT"
                            end repeat
                        end if
                    end repeat
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

mail_send_proof_from_log() {
    local log_file="$1"
    local line=""
    line="$(grep -E 'EVIDENCE\|target=mail\|event=send\|' "$log_file" 2>/dev/null | tail -n 1)"
    if [ -n "$line" ]; then
        local status=""
        local recipient=""
        local subject=""
        local body_len=""
        local draft_id=""
        status="$(printf '%s\n' "$line" | perl -ne 'if (/(?:^|\|)status=([^|]*)/) { print $1; exit }')"
        recipient="$(printf '%s\n' "$line" | perl -ne 'if (/(?:^|\|)recipient=([^|]*)/) { print $1; exit }')"
        subject="$(printf '%s\n' "$line" | perl -ne 'if (/(?:^|\|)subject=([^|]*)/) { print $1; exit }')"
        body_len="$(printf '%s\n' "$line" | perl -ne 'if (/(?:^|\|)body_len=([0-9-]+)/) { print $1; exit }')"
        draft_id="$(printf '%s\n' "$line" | perl -ne 'if (/(?:^|\|)draft_id=([^|]*)/) { print $1; exit }')"
        if [ -n "$status" ]; then
            printf '%s|%s|%s|%s|%s\n' "$status" "$recipient" "$subject" "${body_len:--1}" "$draft_id"
            return 0
        fi
    fi

    line="$(grep -E 'MAIL_SEND_PROOF\|' "$log_file" 2>/dev/null | tail -n 1)"
    if [ -n "$line" ]; then
        local status=""
        local recipient=""
        local subject=""
        local body_len=""
        local draft_id=""
        status="$(printf '%s\n' "$line" | perl -ne 'if (/status=([^|]*)/) { print $1; exit }')"
        recipient="$(printf '%s\n' "$line" | perl -ne 'if (/recipient=([^|]*)/) { print $1; exit }')"
        subject="$(printf '%s\n' "$line" | perl -ne 'if (/subject=([^|]*)/) { print $1; exit }')"
        body_len="$(printf '%s\n' "$line" | perl -ne 'if (/body_len=([0-9-]+)/) { print $1; exit }')"
        draft_id="$(printf '%s\n' "$line" | perl -ne 'if (/draft_id=([^|]*)/) { print $1; exit }')"
        printf '%s|%s|%s|%s|%s\n' "$status" "$recipient" "$subject" "${body_len:--1}" "$draft_id"
        return 0
    fi

    line="$(grep -E '"proof"[[:space:]]*:[[:space:]]*"mail_send"' "$log_file" 2>/dev/null | tail -n 1)"
    [ -z "$line" ] && return 1
    local status=""
    local recipient=""
    local subject=""
    local body_len=""
    local draft_id=""
    status="$(printf '%s\n' "$line" | perl -ne 'if (/"send_status"\s*:\s*"([^"]*)"/) { print $1; exit }')"
    recipient="$(printf '%s\n' "$line" | perl -ne 'if (/"recipient"\s*:\s*"([^"]*)"/) { print $1; exit }')"
    subject="$(printf '%s\n' "$line" | perl -ne 'if (/"subject"\s*:\s*"([^"]*)"/) { print $1; exit }')"
    body_len="$(printf '%s\n' "$line" | perl -ne 'if (/"body_len"\s*:\s*([0-9-]+)/) { print $1; exit }')"
    draft_id="$(printf '%s\n' "$line" | perl -ne 'if (/"draft_id"\s*:\s*"([^"]*)"/) { print $1; exit }')"
    printf '%s|%s|%s|%s|%s\n' "$status" "$recipient" "$subject" "${body_len:--1}" "$draft_id"
    return 0
}

mail_write_evidence_for_draft() {
    local log_file="$1"
    local draft_id="$2"
    [ -f "$log_file" ] || {
        printf '0|0|-1\n'
        return 0
    }
    [ -n "$draft_id" ] || {
        printf '0|0|-1\n'
        return 0
    }

    local lines=""
    local recipient_seen=0
    local subject_seen=0
    local max_body_len="-1"
    lines="$(grep -E 'EVIDENCE\|target=mail\|event=write\|' "$log_file" 2>/dev/null | grep -F "draft_id=${draft_id}" || true)"
    [ -n "$lines" ] || {
        printf '0|0|-1\n'
        return 0
    }
    if printf '%s\n' "$lines" | grep -Eiq '(?:^|\|)recipient='; then
        recipient_seen=1
    fi
    if printf '%s\n' "$lines" | grep -Eiq '(?:^|\|)subject='; then
        subject_seen=1
    fi
    max_body_len="$(printf '%s\n' "$lines" | perl -ne '
        if (/(?:^|\|)body_len=([0-9-]+)/) {
            my $v = $1;
            if (!defined($max) || $v > $max) { $max = $v; }
        }
        END { if (defined($max)) { print $max; } else { print "-1"; } }
    ')"
    printf '%s|%s|%s\n' "$recipient_seen" "$subject_seen" "${max_body_len:--1}"
}

resolve_runtime_db_path() {
    local from_env="${STEER_DB_PATH:-}"
    if [ -n "$from_env" ]; then
        printf '%s\n' "$from_env"
        return 0
    fi
    printf '%s\n' "./steer.db"
}

extract_run_id_from_log() {
    [ -f "$LOG_FILE" ] || return 0
    local run_id=""
    run_id="$(grep -Eo 'run_id[=:][A-Za-z0-9._:-]+' "$LOG_FILE" 2>/dev/null | tail -n 1 | sed -E 's/^run_id[=:]//')"
    if [ -n "$run_id" ]; then
        printf '%s\n' "$run_id"
        return 0
    fi
    run_id="$(grep -Eo '"run_id"[[:space:]]*:[[:space:]]*"[^"]+"' "$LOG_FILE" 2>/dev/null | tail -n 1 | sed -E 's/^.*"run_id"[[:space:]]*:[[:space:]]*"([^"]+)".*$/\1/')"
    if [ -n "$run_id" ]; then
        printf '%s\n' "$run_id"
    fi
}

collect_artifact_top3_lines() {
    local run_id="$1"
    local db_path=""
    local summary=""

    db_path="$(resolve_runtime_db_path)"
    if [ -n "$run_id" ] && command -v sqlite3 >/dev/null 2>&1 && [ -f "$db_path" ]; then
        local escaped_run_id=""
        escaped_run_id="$(printf '%s' "$run_id" | sed "s/'/''/g")"
        summary="$(sqlite3 -readonly "$db_path" "
            SELECT artifact_key || '=' || value
            FROM task_run_artifacts
            WHERE run_id = '${escaped_run_id}'
            ORDER BY created_at DESC
            LIMIT 40;
        " 2>/dev/null \
            | awk -F'=' '
                {
                    key=$1; val=substr($0, length($1)+2);
                    if (key == \"\") next;
                    count[key] += 1;
                    if (!(key in latest)) latest[key]=val;
                }
                END {
                    for (k in count) {
                        printf \"%06d|%s|%s\\n\", count[k], k, latest[k];
                    }
                }
            ' \
            | sort -r \
            | head -n 3 \
            | awk -F'|' '{ sub(/^0+/, \"\", $1); if ($1 == \"\") $1=\"0\"; printf \"- artifact_top%d: %s=%s (hits=%s)\\n\", NR, $2, $3, $1 }')"
    fi

    if [ -z "$summary" ] && [ -f "$LOG_FILE" ]; then
        summary="$(grep -Eo 'artifact\\.[A-Za-z0-9_.-]+' "$LOG_FILE" 2>/dev/null \
            | awk 'NF>0 {count[$1]+=1} END {for (k in count) printf "%06d|%s\n", count[k], k}' \
            | sort -r \
            | head -n 3 \
            | awk -F'|' '{ sub(/^0+/, "", $1); if ($1 == "") $1="0"; printf "- artifact_top%d: %s (hits=%s)\n", NR, $2, $1 }')"
    fi

    printf '%s\n' "$summary"
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
NOTIFIER_TIMEOUT_SEC="${STEER_NOTIFIER_TIMEOUT_SEC:-120}"
REQUIRE_PRIMARY_PLANNER_VALUE="${STEER_REQUIRE_PRIMARY_PLANNER:-1}"
LOCK_DISABLED_VALUE="${STEER_LOCK_DISABLED:-0}"
APPROVAL_ASK_FALLBACK_VALUE="${STEER_APPROVAL_ASK_FALLBACK:-deny}"
RUN_SCOPE_ENABLED="${STEER_SEMANTIC_RUN_SCOPE:-1}"
REQUIRE_TELEGRAM_REPORT_VALUE="${STEER_REQUIRE_TELEGRAM_REPORT:-1}"
TEST_MODE_VALUE="${STEER_TEST_MODE:-0}"
DETERMINISTIC_GOAL_AUTOPLAN_VALUE="${STEER_DETERMINISTIC_GOAL_AUTOPLAN:-}"
if [ -z "$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" ]; then
    DETERMINISTIC_GOAL_AUTOPLAN_VALUE="1"
fi
REQUIRE_MAIL_BODY_VALUE="${STEER_REQUIRE_MAIL_BODY:-1}"
REQUIRE_MAIL_SUBJECT_VALUE="${STEER_REQUIRE_MAIL_SUBJECT:-1}"
REQUIRE_SENT_MAILBOX_EVIDENCE_VALUE="${STEER_REQUIRE_SENT_MAILBOX_EVIDENCE:-1}"
REQUIRE_NOTES_WRITE_VALUE="${STEER_REQUIRE_NOTES_WRITE:-1}"
REQUIRE_TEXTEDIT_WRITE_VALUE="${STEER_REQUIRE_TEXTEDIT_WRITE:-1}"
REQUIRE_NODE_CAPTURE_VALUE="${STEER_REQUIRE_NODE_CAPTURE:-1}"
OPENAI_PREFLIGHT_REQUIRED_VALUE="${STEER_PREFLIGHT_REQUIRE_OPENAI_KEY:-0}"
REQUIRE_SEMANTIC_NONEMPTY_VALUE="${STEER_SEMANTIC_REQUIRE_NONEMPTY:-1}"
RUN_STARTED_EPOCH=0

is_truthy() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|on|ON)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

detect_cli_llm_provider() {
    local preferred="${STEER_CLI_LLM_AUTO_ORDER:-codex,gemini,claude}"
    local oldifs="$IFS"
    IFS=','
    read -r -a providers <<< "$preferred"
    IFS="$oldifs"
    for provider in "${providers[@]}"; do
        local p
        p="$(echo "$provider" | tr '[:upper:]' '[:lower:]' | tr -d ' ')"
        [ -z "$p" ] && continue
        if command -v "$p" >/dev/null 2>&1; then
            printf '%s\n' "$p"
            return 0
        fi
    done
    return 1
}

has_openai_key_configured() {
    if [ -n "${OPENAI_API_KEY:-}" ]; then
        return 0
    fi

    local env_files=(".env" "core/.env")
    local env_file=""
    for env_file in "${env_files[@]}"; do
        [ -f "$env_file" ] || continue
        if grep -Eq '^[[:space:]]*OPENAI_API_KEY[[:space:]]*=' "$env_file"; then
            local key_line
            key_line="$(grep -E '^[[:space:]]*OPENAI_API_KEY[[:space:]]*=' "$env_file" | tail -n 1 || true)"
            local key_value="${key_line#*=}"
            key_value="$(printf '%s' "$key_value" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//; s/^["'"'"']|["'"'"']$//g')"
            if [ -n "$key_value" ]; then
                return 0
            fi
        fi
    done
    return 1
}

if [ -z "$CLI_LLM_VALUE" ] && [ "${STEER_AUTO_DETECT_CLI_LLM:-1}" = "1" ]; then
    if detected="$(detect_cli_llm_provider)"; then
        CLI_LLM_VALUE="$detected"
        echo "🤖 Auto-detected CLI LLM provider: ${CLI_LLM_VALUE}"
    fi
fi

if [ "$RUN_SCOPE_ENABLED" = "1" ]; then
    RUN_SCOPE_MARKER="RUN_SCOPE_${TS}"
    REQUEST_TEXT_EXEC="${REQUEST_TEXT} 마지막 줄에 \"${RUN_SCOPE_MARKER}\"를 정확히 입력하세요."
    REQUEST_TEXT_FOR_VERIFY="$REQUEST_TEXT_EXEC"
fi

if [ "$REQUIRE_PRIMARY_PLANNER_VALUE" = "1" ] && [ "$SCENARIO_MODE_VALUE" = "1" ] && [ "${STEER_ALLOW_SCENARIO_MODE:-0}" != "1" ]; then
    echo "❌ 정책 위반: STEER_SCENARIO_MODE=1 이지만 STEER_ALLOW_SCENARIO_MODE=1 승인 없이 fallback 모드 실행은 금지됩니다."
    echo "   운영 검증은 STEER_SCENARIO_MODE=0으로 실행하거나, 테스트 목적일 때만 STEER_ALLOW_SCENARIO_MODE=1을 설정하세요."
    exit 1
fi

if is_truthy "$LOCK_DISABLED_VALUE"; then
    if ! is_truthy "$TEST_MODE_VALUE" && ! is_truthy "${CI:-0}" && ! is_truthy "${STEER_ALLOW_LOCK_DISABLED_NON_TEST:-0}"; then
        echo "❌ 안전정책 위반: STEER_LOCK_DISABLED=1 은 테스트/CI 전용입니다."
        echo "   운영 실행에서는 STEER_LOCK_DISABLED=0으로 설정하세요."
        echo "   예외 허용이 꼭 필요하면 STEER_ALLOW_LOCK_DISABLED_NON_TEST=1을 명시적으로 설정하세요."
        exit 1
    fi
fi

if [ "$OPENAI_PREFLIGHT_REQUIRED_VALUE" = "1" ] && [ "$SCENARIO_MODE_VALUE" = "0" ] && [ -z "$CLI_LLM_VALUE" ] && ! has_openai_key_configured; then
    echo "❌ Preflight failed: OPENAI_API_KEY is not set."
    echo "   Fix: 기본 OpenAI 경로를 쓰려면 .env/core/.env 또는 현재 셸에 OPENAI_API_KEY를 설정하세요."
    echo "   대안: STEER_CLI_LLM 설정 또는 STEER_SCENARIO_MODE=1(테스트 전용) 사용."
    exit 1
elif [ "$SCENARIO_MODE_VALUE" = "0" ] && [ -z "$CLI_LLM_VALUE" ] && ! has_openai_key_configured; then
    echo "ℹ️ OPENAI_API_KEY 미설정: preflight 강제는 비활성(STEER_PREFLIGHT_REQUIRE_OPENAI_KEY=0)."
    echo "   필요하면 STEER_CLI_LLM을 지정하거나 STEER_PREFLIGHT_REQUIRE_OPENAI_KEY=1로 엄격 모드를 켜세요."
fi

if semantic_require_rust_contract; then
    if [ "${STEER_USE_RUST_SEMANTIC_CONTRACT:-1}" != "1" ]; then
        echo "❌ Preflight failed: STEER_SEMANTIC_REQUIRE_RUST_CONTRACT=1 이면 STEER_USE_RUST_SEMANTIC_CONTRACT=1 이어야 합니다."
        exit 1
    elif ! resolve_semantic_contract_rust_bin >/dev/null 2>&1; then
        echo "❌ Preflight failed: semantic_contract_rs 바이너리를 찾거나 빌드할 수 없습니다."
        echo "   Fix: core에서 cargo build --bin semantic_contract_rs 실행 또는 STEER_SEMANTIC_CONTRACT_AUTO_BUILD=1 확인."
        exit 1
    else
        echo "✅ Preflight: Rust semantic contract parser available."
    fi
fi

if ! preflight_validate_semantic_tokens; then
    exit 1
fi

# Hard blockers that should always fail the run.
HARD_FATAL_PATTERN='Failed to acquire lock|thread .* panicked|FATAL ERROR|⛔️|LLM not available for surf mode|Preflight failed|Surf failed|Execution Error|SCHEMA_ERROR'
# Recovery-possible signals. Include only in strict mode.
SOFT_FATAL_PATTERN='Supervisor escalated|PLAN_REJECTED|LLM Refused'
FATAL_PATTERN="$HARD_FATAL_PATTERN"
if [ "${STEER_FATAL_STRICT:-0}" = "1" ]; then
    FATAL_PATTERN="${HARD_FATAL_PATTERN}|${SOFT_FATAL_PATTERN}"
fi

echo "🚀 Running NL request..."
echo "Task: ${TASK_NAME}"
echo "Mode: STEER_SCENARIO_MODE=${SCENARIO_MODE_VALUE}"
echo "Node Capture: STEER_NODE_CAPTURE=1, STEER_NODE_CAPTURE_ALL=${NODE_CAPTURE_ALL_VALUE}"
echo "Test Mode: STEER_TEST_MODE=${TEST_MODE_VALUE}"
echo "Mail Body Required: STEER_REQUIRE_MAIL_BODY=${REQUIRE_MAIL_BODY_VALUE}"
echo "Mail Subject Required: STEER_REQUIRE_MAIL_SUBJECT=${REQUIRE_MAIL_SUBJECT_VALUE}"
echo "Sent Mailbox Evidence Required: STEER_REQUIRE_SENT_MAILBOX_EVIDENCE=${REQUIRE_SENT_MAILBOX_EVIDENCE_VALUE}"
echo "Fallback Policy: STEER_FAIL_ON_FALLBACK=${FAIL_ON_FALLBACK_VALUE}"
echo "Deterministic Autoplan: STEER_DETERMINISTIC_GOAL_AUTOPLAN=${DETERMINISTIC_GOAL_AUTOPLAN_VALUE}"
if [ -n "$RUN_SCOPE_MARKER" ]; then
    echo "Semantic Scope Marker: ${RUN_SCOPE_MARKER}"
fi
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
    local timeout_sec="${STEER_OSASCRIPT_TIMEOUT_SEC:-15}"
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

should_pause_for_user_input() {
    local front_app="$1"
    local guard_mode="${STEER_USER_INPUT_GUARD_MODE:-all}"
    case "$guard_mode" in
        all)
            return 0
            ;;
        app_list|allowlist)
            is_user_active_front_app "$front_app"
            return $?
            ;;
        none)
            return 1
            ;;
        *)
            is_user_active_front_app "$front_app"
            return $?
            ;;
    esac
}

run_surf_with_input_guard() {
    local use_guard="${STEER_PAUSE_ON_USER_INPUT:-1}"
    INPUT_GUARD_ABORTED=0
    INPUT_GUARD_ABORT_REASON=""
    if [ "$use_guard" != "1" ]; then
        if [ -n "$CLI_LLM_VALUE" ]; then
            STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
                STEER_CLI_LLM="$CLI_LLM_VALUE" \
                STEER_NODE_CAPTURE=1 \
                STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
                STEER_NODE_CAPTURE_DIR="$NODE_DIR" \
                STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
                STEER_APPROVAL_ASK_FALLBACK="$APPROVAL_ASK_FALLBACK_VALUE" \
                STEER_TEST_MODE="$TEST_MODE_VALUE" \
                STEER_DETERMINISTIC_GOAL_AUTOPLAN="$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" \
                cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$REQUEST_TEXT_EXEC" &> "$LOG_FILE"
        else
            STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
                STEER_NODE_CAPTURE=1 \
                STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
                STEER_NODE_CAPTURE_DIR="$NODE_DIR" \
                STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
                STEER_APPROVAL_ASK_FALLBACK="$APPROVAL_ASK_FALLBACK_VALUE" \
                STEER_TEST_MODE="$TEST_MODE_VALUE" \
                STEER_DETERMINISTIC_GOAL_AUTOPLAN="$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" \
                cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$REQUEST_TEXT_EXEC" &> "$LOG_FILE"
        fi
        return $?
    fi

    local active_threshold="${STEER_INPUT_ACTIVE_THRESHOLD_SECONDS:-1}"
    local resume_idle="${STEER_IDLE_RESUME_SECONDS:-3}"
    local poll_interval="${STEER_INPUT_POLL_SECONDS:-1}"
    local max_pauses="${STEER_INPUT_GUARD_MAX_PAUSES:-40}"
    local max_pause_seconds="${STEER_INPUT_GUARD_MAX_PAUSE_SECONDS:-300}"
    local live_new_item_limit="${STEER_INPUT_GUARD_MAX_NEW_ITEMS:-${STEER_MAX_NEW_ITEM_ACTIONS:-6}}"
    local live_new_item_pattern="Shortcut 'n'.*Created new item|mail_draft_ready|shortcut cmd\\+n"
    local paused=0
    local pause_count=0
    local pause_started_epoch=0
    local total_paused_seconds=0
    local run_pid

    if ! [[ "$live_new_item_limit" =~ ^[0-9]+$ ]]; then
        live_new_item_limit=6
    fi
    if [ "$live_new_item_limit" -lt 1 ]; then
        live_new_item_limit=1
    fi

    echo "🛡️ User-input guard enabled (mode=${STEER_USER_INPUT_GUARD_MODE:-all}, apps=${STEER_USER_ACTIVE_APPS:-Terminal,Codex,iTerm2}, active<=${active_threshold}s, resume>=${resume_idle}s, max_pauses=${max_pauses}, max_pause_seconds=${max_pause_seconds})"
    echo "🛡️ Window flood guard enabled (new_item_limit=${live_new_item_limit})"

    if [ -n "$CLI_LLM_VALUE" ]; then
        STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
            STEER_CLI_LLM="$CLI_LLM_VALUE" \
            STEER_NODE_CAPTURE=1 \
            STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
            STEER_NODE_CAPTURE_DIR="$NODE_DIR" \
            STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
            STEER_APPROVAL_ASK_FALLBACK="$APPROVAL_ASK_FALLBACK_VALUE" \
            STEER_TEST_MODE="$TEST_MODE_VALUE" \
            STEER_DETERMINISTIC_GOAL_AUTOPLAN="$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" \
            cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$REQUEST_TEXT_EXEC" &> "$LOG_FILE" &
    else
        STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
            STEER_NODE_CAPTURE=1 \
            STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
            STEER_NODE_CAPTURE_DIR="$NODE_DIR" \
            STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
            STEER_APPROVAL_ASK_FALLBACK="$APPROVAL_ASK_FALLBACK_VALUE" \
            STEER_TEST_MODE="$TEST_MODE_VALUE" \
            STEER_DETERMINISTIC_GOAL_AUTOPLAN="$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" \
            cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$REQUEST_TEXT_EXEC" &> "$LOG_FILE" &
    fi
    run_pid=$!

    while kill -0 "$run_pid" 2>/dev/null; do
        if [ -f "$LOG_FILE" ]; then
            local new_item_live_count=0
            new_item_live_count="$(grep -Eic "$live_new_item_pattern" "$LOG_FILE" 2>/dev/null || true)"
            if [ "${new_item_live_count:-0}" -gt "$live_new_item_limit" ]; then
                INPUT_GUARD_ABORTED=1
                INPUT_GUARD_ABORT_REASON="new_item_flood(${new_item_live_count}>${live_new_item_limit})"
                echo "⛔️ [InputGuard] Abort run: ${INPUT_GUARD_ABORT_REASON}"
                echo "⛔️ [InputGuard] Abort run: ${INPUT_GUARD_ABORT_REASON}" >> "$LOG_FILE"
                kill -TERM "$run_pid" >/dev/null 2>&1 || true
                pkill -TERM -P "$run_pid" >/dev/null 2>&1 || true
                sleep 1
                kill -KILL "$run_pid" >/dev/null 2>&1 || true
                pkill -KILL -P "$run_pid" >/dev/null 2>&1 || true
                break
            fi
        fi
        local idle_sec=""
        idle_sec="$(get_idle_seconds || true)"
        if [ -n "$idle_sec" ]; then
            local front_app
            front_app="$(get_frontmost_app)"
            if [ "$paused" -eq 0 ] && [ "$idle_sec" -le "$active_threshold" ] && should_pause_for_user_input "$front_app"; then
                # Pause root process and immediate children to avoid race with cargo/local_os_agent.
                kill -STOP "$run_pid" >/dev/null 2>&1 || true
                pkill -STOP -P "$run_pid" >/dev/null 2>&1 || true
                paused=1
                pause_count=$((pause_count + 1))
                pause_started_epoch="$(date +%s)"
                echo "⏸️ [InputGuard] Paused run (front_app=${front_app}, idle=${idle_sec}s, count=${pause_count})"
                echo "⏸️ [InputGuard] Paused run (front_app=${front_app}, idle=${idle_sec}s, count=${pause_count})" >> "$LOG_FILE"
                if [ "$max_pauses" -gt 0 ] && [ "$pause_count" -ge "$max_pauses" ]; then
                    INPUT_GUARD_ABORTED=1
                    INPUT_GUARD_ABORT_REASON="max_pauses_exceeded(${pause_count}/${max_pauses})"
                    echo "⛔️ [InputGuard] Abort run: ${INPUT_GUARD_ABORT_REASON}"
                    echo "⛔️ [InputGuard] Abort run: ${INPUT_GUARD_ABORT_REASON}" >> "$LOG_FILE"
                    kill -TERM "$run_pid" >/dev/null 2>&1 || true
                    pkill -TERM -P "$run_pid" >/dev/null 2>&1 || true
                    sleep 1
                    kill -KILL "$run_pid" >/dev/null 2>&1 || true
                    pkill -KILL -P "$run_pid" >/dev/null 2>&1 || true
                    break
                fi
            elif [ "$paused" -eq 1 ] && [ "$idle_sec" -ge "$resume_idle" ]; then
                kill -CONT "$run_pid" >/dev/null 2>&1 || true
                pkill -CONT -P "$run_pid" >/dev/null 2>&1 || true
                paused=0
                if [ "$pause_started_epoch" -gt 0 ]; then
                    local resume_epoch
                    resume_epoch="$(date +%s)"
                    if [ "$resume_epoch" -gt "$pause_started_epoch" ]; then
                        total_paused_seconds=$((total_paused_seconds + resume_epoch - pause_started_epoch))
                    fi
                fi
                pause_started_epoch=0
                echo "▶️ [InputGuard] Resumed run (idle=${idle_sec}s)"
                echo "▶️ [InputGuard] Resumed run (idle=${idle_sec}s)" >> "$LOG_FILE"
            fi
            if [ "$paused" -eq 1 ] && [ "$max_pause_seconds" -gt 0 ] && [ "$pause_started_epoch" -gt 0 ]; then
                local now_epoch
                now_epoch="$(date +%s)"
                local current_pause
                current_pause=$((now_epoch - pause_started_epoch))
                if [ "$current_pause" -lt 0 ]; then
                    current_pause=0
                fi
                if [ $((total_paused_seconds + current_pause)) -ge "$max_pause_seconds" ]; then
                    INPUT_GUARD_ABORTED=1
                    INPUT_GUARD_ABORT_REASON="max_pause_seconds_exceeded(${total_paused_seconds}+${current_pause}/${max_pause_seconds})"
                    echo "⛔️ [InputGuard] Abort run: ${INPUT_GUARD_ABORT_REASON}"
                    echo "⛔️ [InputGuard] Abort run: ${INPUT_GUARD_ABORT_REASON}" >> "$LOG_FILE"
                    kill -TERM "$run_pid" >/dev/null 2>&1 || true
                    pkill -TERM -P "$run_pid" >/dev/null 2>&1 || true
                    sleep 1
                    kill -KILL "$run_pid" >/dev/null 2>&1 || true
                    pkill -KILL -P "$run_pid" >/dev/null 2>&1 || true
                    break
                fi
            fi
        fi
        sleep "$poll_interval"
    done

    wait "$run_pid"
    local exit_code=$?
    if [ "$paused" -eq 1 ] && [ "$pause_started_epoch" -gt 0 ]; then
        local end_epoch
        end_epoch="$(date +%s)"
        if [ "$end_epoch" -gt "$pause_started_epoch" ]; then
            total_paused_seconds=$((total_paused_seconds + end_epoch - pause_started_epoch))
        fi
    fi
    echo "🧾 [InputGuard] pause_count=${pause_count}"
    echo "🧾 [InputGuard] pause_count=${pause_count}" >> "$LOG_FILE"
    echo "🧾 [InputGuard] paused_seconds=${total_paused_seconds}"
    echo "🧾 [InputGuard] paused_seconds=${total_paused_seconds}" >> "$LOG_FILE"
    if [ "${INPUT_GUARD_ABORTED:-0}" = "1" ] && [ "$exit_code" -eq 0 ]; then
        exit_code=124
    fi
    return $exit_code
}

STATUS="success"
RUN_STARTED_EPOCH="$(date +%s)"
INPUT_GUARD_ABORTED=0
INPUT_GUARD_ABORT_REASON=""
if ! run_surf_with_input_guard; then
    STATUS="failed"
fi
if [ "${INPUT_GUARD_ABORTED:-0}" = "1" ]; then
    STATUS="failed"
fi

if grep -Eq "$FATAL_PATTERN" "$LOG_FILE"; then
    STATUS="failed"
fi

RUN_TERMINAL_BLOCK_STATUS=""
for status_name in blocked approval_required manual_required; do
    if run_attempt_phase_status_hit "$LOG_FILE" "execution_end" "$status_name"; then
        RUN_TERMINAL_BLOCK_STATUS="$status_name"
        STATUS="failed"
        break
    fi
done

FALLBACK_HIT=0
if grep -Eiq "fallback action|FALLBACK_ACTION:" "$LOG_FILE"; then
    FALLBACK_HIT=1
    if [ "$FAIL_ON_FALLBACK_VALUE" = "1" ]; then
        STATUS="failed"
    fi
fi

WINDOW_FLOOD_HIT=0
MAX_NEW_ITEM_ACTIONS_VALUE="${STEER_MAX_NEW_ITEM_ACTIONS:-6}"
if ! [[ "$MAX_NEW_ITEM_ACTIONS_VALUE" =~ ^[0-9]+$ ]]; then
    MAX_NEW_ITEM_ACTIONS_VALUE=6
fi
NEW_ITEM_ACTION_COUNT="$(grep -Eic "Shortcut 'n'.*Created new item|mail_draft_ready|shortcut cmd\\+n" "$LOG_FILE" || true)"
if [ "$NEW_ITEM_ACTION_COUNT" -gt "$MAX_NEW_ITEM_ACTIONS_VALUE" ]; then
    WINDOW_FLOOD_HIT=1
    STATUS="failed"
fi

PLANNER_DONE=0
if grep -Eiq "Goal completed by planner|planner goal completed|planner complete" "$LOG_FILE"; then
    PLANNER_DONE=1
fi

MAIL_PROOF_STATUS=""
MAIL_PROOF_RECIPIENT=""
MAIL_PROOF_SUBJECT=""
MAIL_PROOF_BODY_LEN="-1"
MAIL_PROOF_DRAFT_ID=""
mail_dod_required=0
mail_send_logged=0
mail_sent_ok=0
mail_subject_ok=1
mail_recipient_ok=1
mail_body_ok=1
mail_mailbox_evidence_ok=1
if proof_line="$(mail_send_proof_from_log "$LOG_FILE")"; then
    IFS='|' read -r MAIL_PROOF_STATUS MAIL_PROOF_RECIPIENT MAIL_PROOF_SUBJECT MAIL_PROOF_BODY_LEN MAIL_PROOF_DRAFT_ID <<< "$proof_line"
fi

SEMANTIC_LINES=""
FILTERED_TOKENS=()
if [ "${STEER_SEMANTIC_VERIFY:-1}" = "1" ]; then
    RAW_TOKENS=()
    RAW_TOKEN_STREAM=""
    while IFS= read -r token; do
        [ -z "$token" ] && continue
        RAW_TOKEN_STREAM="${RAW_TOKEN_STREAM}${token}"$'\n'
    done < <(extract_expected_tokens_from_request)
    while IFS= read -r token; do
        [ -z "$token" ] && continue
        RAW_TOKEN_STREAM="${RAW_TOKEN_STREAM}${token}"$'\n'
    done < <(extract_expected_tokens_override)
    if [ "${SEMANTIC_CONTRACT_RUST_ERROR:-0}" = "1" ]; then
        STATUS="failed"
        SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 계약 위반: Rust semantic contract 추출 실패 (${SEMANTIC_CONTRACT_RUST_ERROR_DETAIL:-unknown})"$'\n'
    fi
    if [ -n "$RUN_SCOPE_MARKER" ]; then
        RAW_TOKEN_STREAM="${RAW_TOKEN_STREAM}${RUN_SCOPE_MARKER}"$'\n'
    fi
    while IFS= read -r token; do
        [ -z "$token" ] && continue
        RAW_TOKENS+=("$token")
    done < <(printf '%s' "$RAW_TOKEN_STREAM" | awk 'NF > 0 && !seen[$0]++')
    FILTERED_TOKENS=()
    token_truncated=0
    for token in "${RAW_TOKENS[@]}"; do
        [ -z "$token" ] && continue
        if is_noise_token "$token"; then
            continue
        fi
        FILTERED_TOKENS+=("$token")
    done
    default_token_cap=384
    request_len=${#REQUEST_TEXT_FOR_VERIFY}
    if [ "$request_len" -gt 2400 ]; then
        default_token_cap=640
    fi
    token_cap="${STEER_SEMANTIC_MAX_TOKENS:-$default_token_cap}"
    if ! [[ "$token_cap" =~ ^[0-9]+$ ]]; then
        token_cap="$default_token_cap"
    fi
    if [ "$token_cap" -lt 0 ]; then
        token_cap=0
    fi
    if [ "$token_cap" -gt 0 ] && [ "${#FILTERED_TOKENS[@]}" -gt "$token_cap" ]; then
        token_truncated=1
        FILTERED_TOKENS=("${FILTERED_TOKENS[@]:0:$token_cap}")
    fi
    if [ -n "$RUN_SCOPE_MARKER" ]; then
        marker_kept=0
        for token in "${FILTERED_TOKENS[@]}"; do
            if [ "$token" = "$RUN_SCOPE_MARKER" ]; then
                marker_kept=1
                break
            fi
        done
        if [ "$marker_kept" -eq 0 ]; then
            if [ "$token_cap" -gt 0 ] && [ "${#FILTERED_TOKENS[@]}" -ge "$token_cap" ]; then
                FILTERED_TOKENS=("${FILTERED_TOKENS[@]:0:$((token_cap - 1))}")
            fi
            FILTERED_TOKENS+=("$RUN_SCOPE_MARKER")
        fi
    fi

    missing_count=0
    checked_count=0
    if [ "${#FILTERED_TOKENS[@]}" -eq 0 ]; then
        SEMANTIC_LINES="${SEMANTIC_LINES}- 의미 검증 토큰 없음(요청에서 추출된 핵심 문자열 기준)"$'\n'
        if [ "$REQUIRE_SEMANTIC_NONEMPTY_VALUE" = "1" ]; then
            STATUS="failed"
            SEMANTIC_LINES="${SEMANTIC_LINES}- 계약 위반: 의미검증 토큰이 0개라 최종 상태를 failed로 강등"$'\n'
        fi
    else
        for token in "${FILTERED_TOKENS[@]}"; do
            checked_count=$((checked_count + 1))
            normalized_token="$(normalize_semantic_token "$token")"
            location=""
            if [ -n "$MAIL_PROOF_SUBJECT" ] && [ "$MAIL_PROOF_STATUS" = "sent_confirmed" ] && [ "$token" = "$MAIL_PROOF_SUBJECT" ]; then
                location="LOG_MAIL_SUBJECT"
            elif [ -n "$MAIL_PROOF_RECIPIENT" ] && [ "$MAIL_PROOF_STATUS" = "sent_confirmed" ] && [ "$token" = "$MAIL_PROOF_RECIPIENT" ]; then
                location="LOG_MAIL_RECIPIENT"
            else
                location="$(token_presence_location "$token" "$RUN_SCOPE_MARKER" "$RUN_STARTED_EPOCH")"
            fi
            if semantic_location_missing "$location" && [ -n "$normalized_token" ] && [ "$normalized_token" != "$token" ]; then
                if [ -n "$MAIL_PROOF_SUBJECT" ] && [ "$MAIL_PROOF_STATUS" = "sent_confirmed" ] && [ "$normalized_token" = "$MAIL_PROOF_SUBJECT" ]; then
                    location="LOG_MAIL_SUBJECT"
                elif [ -n "$MAIL_PROOF_RECIPIENT" ] && [ "$MAIL_PROOF_STATUS" = "sent_confirmed" ] && [ "$normalized_token" = "$MAIL_PROOF_RECIPIENT" ]; then
                    location="LOG_MAIL_RECIPIENT"
                else
                    location="$(token_presence_location "$normalized_token" "$RUN_SCOPE_MARKER" "$RUN_STARTED_EPOCH")"
                fi
            fi
            if [ "${STEER_SEMANTIC_REQUIRE_APP_SCOPE:-1}" = "1" ] && semantic_location_is_log "$location"; then
                location="LOG_ONLY_BLOCKED(${location})"
            fi
            if semantic_location_missing "$location"; then
                missing_count=$((missing_count + 1))
                SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 ❌ \"${token}\" (location=${location})"$'\n'
            else
                SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 ✅ \"${token}\" (location=${location})"$'\n'
            fi
        done
        SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 토큰 수: ${checked_count}"$'\n'
        if [ "$token_truncated" -eq 1 ]; then
            SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 토큰이 상한(${token_cap})으로 잘렸습니다(STEER_SEMANTIC_MAX_TOKENS 조정 필요)"$'\n'
            if [ "${STEER_SEMANTIC_FAIL_ON_TRUNCATION:-1}" = "1" ]; then
                STATUS="failed"
                SEMANTIC_LINES="${SEMANTIC_LINES}- 계약 위반: 토큰 절단 발생으로 최종 상태를 failed로 강등"$'\n'
            fi
        fi
        if [ -n "$RUN_SCOPE_MARKER" ]; then
            SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 run-scope marker: ${RUN_SCOPE_MARKER}"$'\n'
        fi
    fi

    if [ "$missing_count" -gt 0 ]; then
        STATUS="failed"
        SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 실패로 최종 상태를 failed로 강등"$'\n'
    fi
else
    SEMANTIC_LINES="${SEMANTIC_LINES}- 의미검증 비활성(STEER_SEMANTIC_VERIFY=0)"$'\n'
fi

if request_requires_mail_send; then
    mail_dod_required=1
    mail_send_logged=0
    mail_log_status="${MAIL_PROOF_STATUS:-}"
    mail_log_recipient="${MAIL_PROOF_RECIPIENT:-}"
    mail_log_subject="${MAIL_PROOF_SUBJECT:-}"
    mail_log_body_len="${MAIL_PROOF_BODY_LEN:--1}"
    mail_log_draft_id="${MAIL_PROOF_DRAFT_ID:-}"
    mail_write_recipient_seen=0
    mail_write_subject_seen=0
    mail_write_body_len="-1"
    if [ -n "$mail_log_draft_id" ]; then
        if write_line="$(mail_write_evidence_for_draft "$LOG_FILE" "$mail_log_draft_id")"; then
            IFS='|' read -r mail_write_recipient_seen mail_write_subject_seen mail_write_body_len <<< "$write_line"
        fi
    fi
    if [ "$mail_log_status" = "sent_confirmed" ]; then
        mail_send_logged=1
    elif grep -Eiq "Shortcut 'd'.*shift.*Mail sent|Mail send completed|\"send_status\"[[:space:]]*:[[:space:]]*\"sent_confirmed\"|MAIL_SEND_PROOF\\|status=sent_confirmed|EVIDENCE\\|target=mail\\|event=send\\|status=sent_confirmed" "$LOG_FILE"; then
        mail_send_logged=1
    fi
    outgoing_count="$(mail_outgoing_count || echo -1)"

    mail_verify_token=""
    if [ -n "$RUN_SCOPE_MARKER" ]; then
        mail_verify_token="$RUN_SCOPE_MARKER"
    elif [ "${#FILTERED_TOKENS[@]}" -gt 0 ]; then
        mail_verify_token="${FILTERED_TOKENS[0]}"
    fi

    mail_sent_location="NOT_CHECKED"
    mail_sent_ok=0
    if [ "$mail_send_logged" -eq 1 ]; then
        mail_sent_ok=1
        mail_sent_location="LOG_MAIL_SEND"
    fi
    if [ -n "$mail_verify_token" ]; then
        if [ "$mail_sent_ok" -ne 1 ]; then
            mail_sent_location="$(token_presence_location "$mail_verify_token" "$RUN_SCOPE_MARKER" "$RUN_STARTED_EPOCH")"
            case "$mail_sent_location" in
                MAIL_SENT_SUBJECT|MAIL_SENT_BODY)
                    mail_sent_ok=1
                    ;;
            esac
        fi
    fi

    mail_subject_ok=1
    mail_subject_location="SUBJECT_NOT_REQUIRED"
    if [ "$REQUIRE_MAIL_SUBJECT_VALUE" = "1" ]; then
        mail_subject_ok=0
        trimmed_mail_subject="$(printf '%s' "${mail_log_subject:-}" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"
        if [ -n "$trimmed_mail_subject" ]; then
            mail_subject_ok=1
            mail_subject_location="LOG_MAIL_SUBJECT"
            if [ -n "$RUN_SCOPE_MARKER" ] && ! printf '%s' "$trimmed_mail_subject" | grep -Fq "$RUN_SCOPE_MARKER"; then
                mail_subject_ok=0
                mail_subject_location="SUBJECT_MISSING_SCOPE_MARKER"
            fi
        else
            mail_subject_location="SUBJECT_EMPTY"
        fi
        if [ "$mail_subject_ok" -ne 1 ] && [ "${mail_write_subject_seen:-0}" = "1" ]; then
            mail_subject_ok=1
            mail_subject_location="LOG_MAIL_WRITE_SUBJECT"
        fi
    fi

    expected_recipients_raw="${STEER_EXPECT_MAIL_RECIPIENTS:-}"
    if [ -z "$expected_recipients_raw" ] && [ -n "${STEER_EXPECT_MAIL_RECIPIENT:-}" ]; then
        expected_recipients_raw="${STEER_EXPECT_MAIL_RECIPIENT}"
    fi
    if [ -z "$expected_recipients_raw" ]; then
        expected_recipients_raw="$(extract_expected_recipients_from_request)"
    fi
    if [ -z "$expected_recipients_raw" ] && [ -n "${STEER_DEFAULT_MAIL_TO:-}" ]; then
        expected_recipients_raw="${STEER_DEFAULT_MAIL_TO}"
    fi
    EXPECTED_RECIPIENTS=()
    while IFS= read -r recipient; do
        [ -z "$recipient" ] && continue
        EXPECTED_RECIPIENTS+=("$recipient")
    done < <(
        printf '%s\n' "$expected_recipients_raw" \
            | tr ',;' '\n' \
            | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' \
            | tr '[:upper:]' '[:lower:]' \
            | tr -d '[:space:]' \
            | awk 'NF > 0 && !seen[$0]++'
    )
    mail_recipient_location="RECIPIENT_NOT_REQUIRED"
    mail_recipient_ok=1
    expected_recipients_label="optional"
    if [ "${#EXPECTED_RECIPIENTS[@]}" -gt 0 ]; then
        expected_recipients_label="$(printf '%s' "${EXPECTED_RECIPIENTS[*]}" | tr ' ' ',')"
        local_mail_log_recipient="$(printf '%s' "$mail_log_recipient" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"
        MISSING_RECIPIENTS=()
        for expected_recipient in "${EXPECTED_RECIPIENTS[@]}"; do
            if ! printf '%s' "$expected_recipient" | grep -Eq '.+@.+\..+'; then
                continue
            fi
            recipient_single_ok=0
            recipient_single_location="NOT_FOUND"
            if [ -n "$local_mail_log_recipient" ] && [ "$local_mail_log_recipient" = "$expected_recipient" ]; then
                recipient_single_ok=1
                recipient_single_location="LOG_MAIL_RECIPIENT"
            elif [ "${mail_write_recipient_seen:-0}" = "1" ]; then
                recipient_single_ok=1
                recipient_single_location="LOG_MAIL_WRITE_RECIPIENT"
            else
                recipient_single_location="$(mail_sent_recipient_location "$expected_recipient" "$RUN_SCOPE_MARKER" "$RUN_STARTED_EPOCH")"
                if [ "$recipient_single_location" = "MAIL_SENT_RECIPIENT" ]; then
                    recipient_single_ok=1
                fi
            fi
            if [ "$recipient_single_ok" -ne 1 ]; then
                mail_recipient_ok=0
                MISSING_RECIPIENTS+=("${expected_recipient}@${recipient_single_location}")
            fi
        done
        if [ "$mail_recipient_ok" -eq 1 ]; then
            if [ -n "$local_mail_log_recipient" ]; then
                mail_recipient_location="LOG_MAIL_RECIPIENT"
            elif [ "${mail_write_recipient_seen:-0}" = "1" ]; then
                mail_recipient_location="LOG_MAIL_WRITE_RECIPIENT"
            else
                mail_recipient_location="MAIL_SENT_RECIPIENT"
            fi
        elif [ "${#MISSING_RECIPIENTS[@]}" -gt 0 ]; then
            mail_recipient_location="MISSING[$(printf '%s' "${MISSING_RECIPIENTS[*]}" | tr ' ' ',')]"
        fi
    fi

    mail_body_ok=1
    mail_body_location="BODY_NOT_REQUIRED"
    if [ "$REQUIRE_MAIL_BODY_VALUE" = "1" ]; then
        mail_body_ok=0
        mail_body_location="${mail_sent_location}"
        if [ "${mail_write_body_len:--1}" -gt 2 ] 2>/dev/null; then
            mail_body_ok=1
            mail_body_location="LOG_MAIL_WRITE_BODY_LEN"
        elif [ "$mail_sent_location" = "MAIL_SENT_BODY" ]; then
            mail_body_ok=1
            mail_body_location="MAIL_SENT_BODY"
        elif [ "${mail_log_body_len:-0}" -gt 2 ] 2>/dev/null; then
            mail_body_ok=1
            mail_body_location="LOG_MAIL_BODY_LEN"
        fi
    fi
    if [ "${STEER_SEMANTIC_REQUIRE_APP_SCOPE:-1}" = "1" ]; then
        if semantic_location_is_log "$mail_sent_location"; then
            mail_sent_ok=0
            mail_sent_location="LOG_ONLY_BLOCKED(${mail_sent_location})"
        fi
        if [ "$mail_subject_ok" -eq 1 ] && semantic_location_is_log "$mail_subject_location"; then
            mail_subject_ok=0
            mail_subject_location="LOG_ONLY_BLOCKED(${mail_subject_location})"
        fi
        if [ "$mail_recipient_ok" -eq 1 ] && semantic_location_is_log "$mail_recipient_location"; then
            mail_recipient_ok=0
            mail_recipient_location="LOG_ONLY_BLOCKED(${mail_recipient_location})"
        fi
        if [ "$mail_body_ok" -eq 1 ] && semantic_location_is_log "$mail_body_location"; then
            mail_body_ok=0
            mail_body_location="LOG_ONLY_BLOCKED(${mail_body_location})"
        fi
    fi
    mail_mailbox_evidence_ok=0
    mail_mailbox_evidence_location="$mail_sent_location"
    case "$mail_sent_location" in
        MAIL_SENT_SUBJECT|MAIL_SENT_BODY)
            mail_mailbox_evidence_ok=1
            ;;
    esac
    if [ "$mail_mailbox_evidence_ok" -ne 1 ] && [ "$mail_recipient_location" = "MAIL_SENT_RECIPIENT" ]; then
        mail_mailbox_evidence_ok=1
        mail_mailbox_evidence_location="MAIL_SENT_RECIPIENT"
    fi
    if [ "$mail_mailbox_evidence_ok" -ne 1 ] \
        && [ "$mail_sent_location" = "LOG_MAIL_SEND" ] \
        && [ -n "$mail_log_draft_id" ] \
        && [ "${mail_write_body_len:--1}" -gt 2 ] 2>/dev/null \
        && { [ "${mail_write_recipient_seen:-0}" = "1" ] || [ -n "$mail_log_recipient" ]; }; then
        mail_mailbox_evidence_ok=1
        mail_mailbox_evidence_location="LOG_MAIL_FLOW_DRAFT"
    fi
    if [ "$REQUIRE_SENT_MAILBOX_EVIDENCE_VALUE" != "1" ]; then
        mail_mailbox_evidence_ok=1
    fi

    if [ "$mail_send_logged" -eq 1 ] && [ "$mail_sent_ok" -eq 1 ] && [ "$mail_recipient_ok" -eq 1 ] && [ "$mail_body_ok" -eq 1 ] && [ "$mail_subject_ok" -eq 1 ] && [ "$mail_mailbox_evidence_ok" -eq 1 ]; then
        SEMANTIC_LINES="${SEMANTIC_LINES}- 메일 발송 검증 ✅ (send-action 로그/증거 + recipients=${expected_recipients_label}, outgoing=${outgoing_count}, sent_location=${mail_sent_location}, mailbox_evidence=${mail_mailbox_evidence_location}, subject_location=${mail_subject_location}, body_location=${mail_body_location}, body_len=${mail_log_body_len:-n/a}, draft_id=${mail_log_draft_id:-n/a}, write_recipient=${mail_write_recipient_seen:-0}, write_subject=${mail_write_subject_seen:-0}, write_body_len=${mail_write_body_len:--1}, subject=${mail_log_subject:-n/a})"$'\n'
    else
        SEMANTIC_LINES="${SEMANTIC_LINES}- 메일 발송 검증 ❌ (send-action 로그=${mail_send_logged}, outgoing=${outgoing_count}, sent_location=${mail_sent_location}, mailbox_required=${REQUIRE_SENT_MAILBOX_EVIDENCE_VALUE}, mailbox_location=${mail_mailbox_evidence_location}, subject_required=${REQUIRE_MAIL_SUBJECT_VALUE}, subject_location=${mail_subject_location}, body_required=${REQUIRE_MAIL_BODY_VALUE}, body_location=${mail_body_location}, body_len=${mail_log_body_len:-n/a}, draft_id=${mail_log_draft_id:-n/a}, write_recipient=${mail_write_recipient_seen:-0}, write_subject=${mail_write_subject_seen:-0}, write_body_len=${mail_write_body_len:--1}, recipients=${expected_recipients_label}, recipient_location=${mail_recipient_location}, token=${mail_verify_token:-none})"$'\n'
        STATUS="failed"
    fi
fi

notes_required=0
notes_write_seen=0
notes_target_seen=0
notes_target_app_seen=0
notes_target_location="NOT_CHECKED"
notes_write_ok=1
if printf '%s' "$REQUEST_TEXT_FOR_VERIFY" | grep -Eiq 'notes|메모|노트'; then
    notes_required=1
fi
if grep -Eiq 'EVIDENCE\|target=notes\|event=write\|.*body_len=' "$LOG_FILE"; then
    notes_write_seen=1
fi
if notes_target="$(extract_latest_notes_target_from_log || true)"; then
    if [ -n "$notes_target" ]; then
        IFS='|' read -r notes_target_id notes_target_name <<< "$notes_target"
        if [ -n "${notes_target_id:-}" ] || [ -n "${notes_target_name:-}" ]; then
            notes_target_seen=1
            notes_target_location="$(notes_target_presence_location "${notes_target_id:-}" "${notes_target_name:-}" "$RUN_SCOPE_MARKER")"
            case "$notes_target_location" in
                NOTE_TARGET_ID|NOTE_TARGET_NAME)
                    notes_target_app_seen=1
                    ;;
            esac
        fi
    fi
fi
if [ "$notes_required" = "1" ] && [ "$REQUIRE_NOTES_WRITE_VALUE" = "1" ]; then
    if [ "$notes_write_seen" -ne 1 ] || [ "$notes_target_seen" -ne 1 ] || [ "$notes_target_app_seen" -ne 1 ]; then
        notes_write_ok=0
        STATUS="failed"
        SEMANTIC_LINES="${SEMANTIC_LINES}- 노트 작성 검증 ❌ (write_seen=${notes_write_seen}, target_seen=${notes_target_seen}, target_app_seen=${notes_target_app_seen}, target_location=${notes_target_location})"$'\n'
    else
        SEMANTIC_LINES="${SEMANTIC_LINES}- 노트 작성 검증 ✅ (write_seen=${notes_write_seen}, target_seen=${notes_target_seen}, target_app_seen=${notes_target_app_seen}, target_location=${notes_target_location})"$'\n'
    fi
fi

textedit_required=0
textedit_write_seen=0
textedit_target_seen=0
textedit_target_app_seen=0
textedit_target_location="NOT_CHECKED"
textedit_write_ok=1
if printf '%s' "$REQUEST_TEXT_FOR_VERIFY" | grep -Eiq 'textedit|텍스트에디트|텍스트 편집'; then
    textedit_required=1
fi
if grep -Eiq 'EVIDENCE\|target=textedit\|event=write\|.*body_len=' "$LOG_FILE"; then
    textedit_write_seen=1
fi
if textedit_target="$(extract_latest_textedit_target_from_log || true)"; then
    if [ -n "$textedit_target" ]; then
        IFS='|' read -r textedit_target_id textedit_target_name <<< "$textedit_target"
        if [ -n "${textedit_target_id:-}" ] || [ -n "${textedit_target_name:-}" ]; then
            textedit_target_seen=1
            textedit_target_location="$(textedit_target_presence_location "${textedit_target_id:-}" "${textedit_target_name:-}" "$RUN_SCOPE_MARKER")"
            case "$textedit_target_location" in
                TEXTEDIT_TARGET_ID|TEXTEDIT_TARGET_NAME)
                    textedit_target_app_seen=1
                    ;;
            esac
        fi
    fi
fi
if [ "$textedit_required" = "1" ] && [ "$REQUIRE_TEXTEDIT_WRITE_VALUE" = "1" ]; then
    if [ "$textedit_write_seen" -ne 1 ] || [ "$textedit_target_seen" -ne 1 ] || [ "$textedit_target_app_seen" -ne 1 ]; then
        textedit_write_ok=0
        STATUS="failed"
        SEMANTIC_LINES="${SEMANTIC_LINES}- 텍스트에디트 작성 검증 ❌ (write_seen=${textedit_write_seen}, target_seen=${textedit_target_seen}, target_app_seen=${textedit_target_app_seen}, target_location=${textedit_target_location})"$'\n'
    else
        SEMANTIC_LINES="${SEMANTIC_LINES}- 텍스트에디트 작성 검증 ✅ (write_seen=${textedit_write_seen}, target_seen=${textedit_target_seen}, target_app_seen=${textedit_target_app_seen}, target_location=${textedit_target_location})"$'\n'
    fi
fi

KEY_LOGS=$(grep -En "Goal completed by planner|Surf failed|Supervisor escalated|Preflight failed|Execution Error|SCHEMA_ERROR|PLAN_REJECTED|LLM Refused|fallback action|FALLBACK_ACTION:|MAIL_SEND_PROOF\\||EVIDENCE\\|target=mail\\|event=send\\|" "$LOG_FILE" 2>/dev/null | tail -n 6 | sed -E 's/^[0-9]+://')
if [ -z "$KEY_LOGS" ]; then
    KEY_LOGS=$(tail -n 4 "$LOG_FILE" 2>/dev/null | sed -E 's/^[[:space:]]+//')
fi
LOOP_BLOCKED_COUNT=$(grep -Ec 'LOOP_BLOCKED: high_risk_repeated_plan' "$LOG_FILE" 2>/dev/null || true)
CMD_N_GUARD_COUNT=$(grep -Ec 'cmd_n_loop_guard_block' "$LOG_FILE" 2>/dev/null || true)
CMD_N_WINDOW_FLOOD_GUARD_COUNT=$(grep -Ec 'cmd_n_window_flood_block' "$LOG_FILE" 2>/dev/null || true)

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
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_TEST_MODE=${TEST_MODE_VALUE}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_REQUIRE_MAIL_BODY=${REQUIRE_MAIL_BODY_VALUE}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_REQUIRE_MAIL_SUBJECT=${REQUIRE_MAIL_SUBJECT_VALUE}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_REQUIRE_SENT_MAILBOX_EVIDENCE=${REQUIRE_SENT_MAILBOX_EVIDENCE_VALUE}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_REQUIRE_NOTES_WRITE=${REQUIRE_NOTES_WRITE_VALUE}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_REQUIRE_TEXTEDIT_WRITE=${REQUIRE_TEXTEDIT_WRITE_VALUE}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_SEMANTIC_FAIL_ON_TRUNCATION=${STEER_SEMANTIC_FAIL_ON_TRUNCATION}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_SEMANTIC_ENFORCE_RUST_ONLY=${STEER_SEMANTIC_ENFORCE_RUST_ONLY}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_SEMANTIC_REQUIRE_APP_SCOPE=${STEER_SEMANTIC_REQUIRE_APP_SCOPE}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- STEER_MAX_NEW_ITEM_ACTIONS=${MAX_NEW_ITEM_ACTIONS_VALUE}"$'\n'
semantic_contract_source="heuristic"
if [ "${STEER_SEMANTIC_ENFORCE_RUST_ONLY:-1}" = "1" ]; then
    semantic_contract_source="rust_enforced"
elif semantic_require_rust_contract; then
    semantic_contract_source="rust_only"
elif semantic_allow_heuristic_fallback; then
    semantic_contract_source="rust_with_fallback"
fi
EVIDENCE_LINES="${EVIDENCE_LINES}- semantic_contract_source=${semantic_contract_source}"$'\n'
RUN_ID_FROM_LOG="$(extract_run_id_from_log || true)"
if [ -n "$RUN_ID_FROM_LOG" ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- run_id=${RUN_ID_FROM_LOG}"$'\n'
fi
ARTIFACT_TOP3_LINES="$(collect_artifact_top3_lines "$RUN_ID_FROM_LOG" || true)"
if [ -n "$ARTIFACT_TOP3_LINES" ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- artifact evidence top3"$'\n'"${ARTIFACT_TOP3_LINES}"$'\n'
else
    EVIDENCE_LINES="${EVIDENCE_LINES}- artifact evidence top3: (no artifact summary found)"$'\n'
fi
DIAG_LINES="$(collect_diagnostic_event_lines || true)"
if [ -n "$DIAG_LINES" ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- diagnostics tail (${STEER_DIAGNOSTIC_EVENTS_PATH:-scenario_results/diagnostic_events.jsonl})"$'\n'"${DIAG_LINES}"$'\n'
fi
if [ "$notes_required" = "1" ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- notes_target_app_seen=${notes_target_app_seen} (${notes_target_location})"$'\n'
fi
if [ "$textedit_required" = "1" ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- textedit_target_app_seen=${textedit_target_app_seen} (${textedit_target_location})"$'\n'
fi
if [ "$FALLBACK_HIT" -eq 1 ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- fallback 액션 감지됨(fallback action/FALLBACK_ACTION)"$'\n'
    if [ "$FAIL_ON_FALLBACK_VALUE" = "1" ]; then
        EVIDENCE_LINES="${EVIDENCE_LINES}- 정책상 fallback 감지 시 실패 처리(STEER_FAIL_ON_FALLBACK=1)"$'\n'
    fi
fi
if [ "$WINDOW_FLOOD_HIT" -eq 1 ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- 창 생성 과다 감지(new_item_count=${NEW_ITEM_ACTION_COUNT}, limit=${MAX_NEW_ITEM_ACTIONS_VALUE})"$'\n'
fi
if [ "$LOOP_BLOCKED_COUNT" -gt 0 ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- 고위험 반복 액션 차단 횟수=${LOOP_BLOCKED_COUNT} (open_app/cmd+n 즉시 반복 방지)"$'\n'
fi
if [ "$CMD_N_GUARD_COUNT" -gt 0 ]; then
    STATUS="failed"
    EVIDENCE_LINES="${EVIDENCE_LINES}- cmd+n 루프 가드 발동 횟수=${CMD_N_GUARD_COUNT} (동일 앱 새 창 과다 시도 차단)"$'\n'
fi
if [ "$CMD_N_WINDOW_FLOOD_GUARD_COUNT" -gt 0 ]; then
    STATUS="failed"
    EVIDENCE_LINES="${EVIDENCE_LINES}- cmd+n 창 폭증 가드 발동 횟수=${CMD_N_WINDOW_FLOOD_GUARD_COUNT} (최근 new item 누적 과다 차단)"$'\n'
fi
if [ "${INPUT_GUARD_ABORTED:-0}" = "1" ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- 입력 가드 중단: ${INPUT_GUARD_ABORT_REASON:-unknown}"$'\n'
fi
if [ -n "$RUN_TERMINAL_BLOCK_STATUS" ]; then
    EVIDENCE_LINES="${EVIDENCE_LINES}- 실행 종료 상태: ${RUN_TERMINAL_BLOCK_STATUS}(RUN_ATTEMPT_JSON execution_end 기준)"$'\n'
fi
EVIDENCE_LINES="${EVIDENCE_LINES}${SEMANTIC_LINES}"

NODE_COUNT=0
if [ -d "$NODE_DIR" ]; then
    NODE_COUNT=$(find "$NODE_DIR" -maxdepth 1 -type f -name '*.png' | wc -l | tr -d ' ')
fi
if [ "$REQUIRE_NODE_CAPTURE_VALUE" = "1" ] && [ "$NODE_COUNT" -eq 0 ]; then
    STATUS="failed"
    EVIDENCE_LINES="${EVIDENCE_LINES}- 계약 위반: node_capture required인데 노드 캡처가 없습니다"$'\n'
fi
EVIDENCE_LINES="${EVIDENCE_LINES}- 노드 캡처 수: ${NODE_COUNT}"$'\n'
EVIDENCE_LINES="${EVIDENCE_LINES}- 노드 캡처 폴더: $(basename "$NODE_DIR")"$'\n'
log_run_attempt \
    "final_judgement" \
    "$STATUS" \
    "fallback=${FALLBACK_HIT:-0},semantic_missing=${missing_count:-0},mail_proof=${MAIL_PROOF_STATUS:-none},node_count=${NODE_COUNT},telegram_required=${REQUIRE_TELEGRAM_REPORT_VALUE}"

NODE_STEP_SUMMARY=""
NODE_STEP_COUNT=0
TELEGRAM_MAIN_IMAGE=""
: > "$NODE_IMAGE_LIST_FILE"
NODE_ERROR_STEPS=""
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

    NODE_ERROR_STEPS=$(awk '
        {
            if (match($0, /\[Step [0-9]+\/[0-9]+\]/)) {
                stepLine = substr($0, RSTART, RLENGTH)
                gsub(/^\[Step /, "", stepLine)
                gsub(/\/[0-9]+\]$/, "", stepLine)
                current_step = stepLine
            }
            if ($0 ~ /Execution Error:/ && current_step != "") {
                err[current_step] = 1
            }
        }
        END {
            for (s in err) {
                print s
            }
        }
    ' "$LOG_FILE" | sort -n)

    if [ -n "$NODE_LAST_ROWS" ]; then
        while IFS= read -r row; do
            [ -z "$row" ] && continue
            IFS='|' read -r _step_key _ord path step action phase app note <<< "$row"
            node_status="✅ 실행"
            step_has_error=0
            if [ -n "$NODE_ERROR_STEPS" ] && printf '%s\n' "$NODE_ERROR_STEPS" | grep -qx "$step"; then
                step_has_error=1
            fi
            if [[ "$phase" == *error* ]] || [[ "$note" == *failed* ]] || [ "$step_has_error" -eq 1 ]; then
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
EVIDENCE_LINES="${EVIDENCE_LINES}- 단계 상태는 노드 캡처 + Execution Error 로그 기준이며, 내용 충족 여부는 의미검증 라인 기준"$'\n'

DOD_FAIL_COUNT=0
DOD_LINES=""
if [ "$PLANNER_DONE" = "1" ]; then
    DOD_LINES="${DOD_LINES}- [planner] planner_complete ✅ (Goal completed by planner 로그 확인)"$'\n'
else
    DOD_FAIL_COUNT=$((DOD_FAIL_COUNT + 1))
    STATUS="failed"
    DOD_LINES="${DOD_LINES}- [planner] planner_complete ❌ (완료 로그 미확인)"$'\n'
fi
if [ "${STEER_SEMANTIC_VERIFY:-1}" = "1" ]; then
    semantic_stage_ok=1
    if [ "${missing_count:-0}" -gt 0 ]; then
        semantic_stage_ok=0
    fi
    if [ "$semantic_stage_ok" = "1" ]; then
        DOD_LINES="${DOD_LINES}- [semantic] required_tokens ✅ (missing=${missing_count:-0})"$'\n'
    else
        DOD_FAIL_COUNT=$((DOD_FAIL_COUNT + 1))
        DOD_LINES="${DOD_LINES}- [semantic] required_tokens ❌ (missing=${missing_count:-0})"$'\n'
    fi
else
    DOD_LINES="${DOD_LINES}- [semantic] required_tokens ⏭️ (비활성)"$'\n'
fi
if [ "$mail_dod_required" = "1" ]; then
    if [ "$mail_send_logged" -eq 1 ] && [ "$mail_sent_ok" -eq 1 ] && [ "$mail_recipient_ok" -eq 1 ] && [ "$mail_body_ok" -eq 1 ] && [ "$mail_subject_ok" -eq 1 ] && [ "$mail_mailbox_evidence_ok" -eq 1 ]; then
        DOD_LINES="${DOD_LINES}- [mail] send_completed ✅ (recipient/subject/body/mailbox evidence 확인)"$'\n'
    else
        DOD_FAIL_COUNT=$((DOD_FAIL_COUNT + 1))
        DOD_LINES="${DOD_LINES}- [mail] send_completed ❌ (send=${mail_send_logged}, sent=${mail_sent_ok}, recipient=${mail_recipient_ok}, subject=${mail_subject_ok}, body=${mail_body_ok}, mailbox=${mail_mailbox_evidence_ok})"$'\n'
    fi
fi
if [ "$notes_required" = "1" ] && [ "$REQUIRE_NOTES_WRITE_VALUE" = "1" ]; then
    if [ "$notes_write_ok" = "1" ]; then
        DOD_LINES="${DOD_LINES}- [notes] write_saved ✅ (write=${notes_write_seen}, target=${notes_target_seen}, app_target=${notes_target_app_seen})"$'\n'
    else
        DOD_FAIL_COUNT=$((DOD_FAIL_COUNT + 1))
        DOD_LINES="${DOD_LINES}- [notes] write_saved ❌ (write=${notes_write_seen}, target=${notes_target_seen}, app_target=${notes_target_app_seen})"$'\n'
    fi
fi
if [ "$textedit_required" = "1" ] && [ "$REQUIRE_TEXTEDIT_WRITE_VALUE" = "1" ]; then
    if [ "$textedit_write_ok" = "1" ]; then
        DOD_LINES="${DOD_LINES}- [textedit] write_saved ✅ (write=${textedit_write_seen}, target=${textedit_target_seen}, app_target=${textedit_target_app_seen})"$'\n'
    else
        DOD_FAIL_COUNT=$((DOD_FAIL_COUNT + 1))
        DOD_LINES="${DOD_LINES}- [textedit] write_saved ❌ (write=${textedit_write_seen}, target=${textedit_target_seen}, app_target=${textedit_target_app_seen})"$'\n'
    fi
fi
if [ "$REQUIRE_NODE_CAPTURE_VALUE" = "1" ]; then
    if [ "$NODE_COUNT" -gt 0 ]; then
        DOD_LINES="${DOD_LINES}- [capture] node_capture ✅ (count=${NODE_COUNT})"$'\n'
    else
        DOD_FAIL_COUNT=$((DOD_FAIL_COUNT + 1))
        DOD_LINES="${DOD_LINES}- [capture] node_capture ❌ (count=${NODE_COUNT})"$'\n'
    fi
fi
EVIDENCE_LINES="${EVIDENCE_LINES}- Stage DoD"$'\n'"${DOD_LINES}"

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

JUDGEMENT_SUMMARY="planner_done=${PLANNER_DONE}, semantic_missing=${missing_count:-0}, mail_proof=${MAIL_PROOF_STATUS:-none}, node_count=${NODE_COUNT}, dod_fail=${DOD_FAIL_COUNT}"
FAIL_PRIMARY_REASON="none"
RETRY_GUIDE="동일 요청을 재실행해도 됩니다."
if [ "$STATUS" != "success" ]; then
    if [ -n "${RUN_TERMINAL_BLOCK_STATUS:-}" ]; then
        FAIL_PRIMARY_REASON="terminal_status=${RUN_TERMINAL_BLOCK_STATUS}"
        RETRY_GUIDE="승인/수동 단계 해소 후 재실행하세요."
    elif [ -n "${INPUT_GUARD_ABORT_REASON:-}" ]; then
        FAIL_PRIMARY_REASON="input_guard=${INPUT_GUARD_ABORT_REASON}"
        RETRY_GUIDE="사용자 입력 충돌이 없는 상태에서 재실행하세요."
    elif [ "${missing_count:-0}" -gt 0 ]; then
        FAIL_PRIMARY_REASON="semantic_missing_tokens=${missing_count}"
        RETRY_GUIDE="요구 토큰이 실제 앱 결과에 남도록 단계 입력을 보강하세요."
    elif [ "${FALLBACK_HIT:-0}" -eq 1 ]; then
        FAIL_PRIMARY_REASON="fallback_detected"
        RETRY_GUIDE="fallback 없이 계획이 끝나도록 단계/권한/포커스를 점검하세요."
    elif [ "${WINDOW_FLOOD_HIT:-0}" -eq 1 ]; then
        FAIL_PRIMARY_REASON="new_item_window_flood"
        RETRY_GUIDE="Mail 초안창 정리 후, 중복 Cmd+N 없이 재실행하세요."
    elif [ "${CMD_N_WINDOW_FLOOD_GUARD_COUNT:-0}" -gt 0 ]; then
        FAIL_PRIMARY_REASON="cmd_n_window_flood_guard"
        RETRY_GUIDE="최근 생성된 새 창을 정리한 뒤 재실행하세요."
    else
        FAIL_PRIMARY_REASON="evidence_or_runtime_failure"
        RETRY_GUIDE="실패 근거 라인을 기준으로 해당 단계만 보강 후 재실행하세요."
    fi
fi

BRIEF_MAIL_RESULT_LINE="- 메일 발송 증거: 확인 필요"
if [ "${MAIL_PROOF_STATUS:-}" = "sent_confirmed" ]; then
    BRIEF_MAIL_RESULT_LINE="- 메일 정상 발송 완료 (${MAIL_PROOF_RECIPIENT:-unknown})"
fi
BRIEF_FINAL_LINE="- 문제가 있어 재실행 필요 -> 실패"
if [ "$STATUS" = "success" ]; then
    BRIEF_FINAL_LINE="- 문제 없음 -> 성공"
fi

if [ "$STATUS" = "success" ] && [ "$COMPACT_SUCCESS_REPORT_VALUE" = "1" ]; then
TELEGRAM_MESSAGE=$(cat <<EOF
📌 ${TASK_NAME} - 성공 요약

🔄 워크플로우
- 자연어 요청 플랜 실행 -> 앱 액션 수행 -> 검증 -> 텔레그램 보고

✅ 결과
- ${RESULT_TEXT}
${BRIEF_MAIL_RESULT_LINE}
- 문제 없음 -> 성공
EOF
)
else
TELEGRAM_MESSAGE=$(cat <<EOF
📌 ${TASK_NAME} - 쉽게 말한 요약

🔄 뭘 했는지
- 사용자 명령을 실행 플랜으로 만들고
- 단계별 실행/캡처 증거를 수집했고
- 결과를 검증 규칙으로 최종 판정했어요.

✅ 결과
- ${RESULT_TEXT}
${BRIEF_MAIL_RESULT_LINE}
${BRIEF_FINAL_LINE}

상태: ${STATUS_LABEL}
판정:
- ${JUDGEMENT_SUMMARY}
- fail_reason=${FAIL_PRIMARY_REASON}
재실행 가이드:
- ${RETRY_GUIDE}
근거:
${EVIDENCE_LINES}- 로그: $(basename "$LOG_FILE")
EOF
)
fi
TELEGRAM_MESSAGE="$(compress_telegram_report "$TELEGRAM_MESSAGE")"

printf '%s\n' "$TELEGRAM_MESSAGE" > "$RAW_MSG_FILE"

if [ -n "${TELEGRAM_BOT_TOKEN:-}" ] && [ -n "${TELEGRAM_CHAT_ID:-}" ] && [ -f "./send_telegram_notification.sh" ]; then
    TELEGRAM_SEND_OK=1
    EXTRA_NODE_ENV=()
    NODE_IMAGE_COUNT=0
    if [ -s "$NODE_IMAGE_LIST_FILE" ]; then
        EXTRA_NODE_ENV=(TELEGRAM_EXTRA_IMAGE_LIST_FILE="$NODE_IMAGE_LIST_FILE")
        NODE_IMAGE_COUNT="$(grep -Ec '^[^|]+' "$NODE_IMAGE_LIST_FILE" || true)"
        NODE_IMAGE_COUNT="${NODE_IMAGE_COUNT:-0}"
    fi
    TELEGRAM_TIMEOUT_EFFECTIVE="$(compute_notifier_timeout "$NOTIFIER_TIMEOUT_SEC" "$NODE_IMAGE_COUNT")"

    if [ -n "$TELEGRAM_MAIN_IMAGE" ] && [ -f "$TELEGRAM_MAIN_IMAGE" ]; then
        if ! send_telegram_with_timeout "$TELEGRAM_TIMEOUT_EFFECTIVE" \
            env TELEGRAM_DUMP_FINAL_PATH="$FINAL_MSG_FILE" TELEGRAM_SKIP_REWRITE=1 TELEGRAM_VALIDATE_REPORT=1 TELEGRAM_REQUIRE_SEND="$REQUIRE_TELEGRAM_REPORT_VALUE" ${EXTRA_NODE_ENV[@]+"${EXTRA_NODE_ENV[@]}"} \
            bash ./send_telegram_notification.sh "$TELEGRAM_MESSAGE" "$TELEGRAM_MAIN_IMAGE" >/dev/null 2>&1; then
            TELEGRAM_SEND_OK=0
        fi
    else
        if ! send_telegram_with_timeout "$TELEGRAM_TIMEOUT_EFFECTIVE" \
            env TELEGRAM_DUMP_FINAL_PATH="$FINAL_MSG_FILE" TELEGRAM_SKIP_REWRITE=1 TELEGRAM_VALIDATE_REPORT=1 TELEGRAM_REQUIRE_SEND="$REQUIRE_TELEGRAM_REPORT_VALUE" ${EXTRA_NODE_ENV[@]+"${EXTRA_NODE_ENV[@]}"} \
            bash ./send_telegram_notification.sh "$TELEGRAM_MESSAGE" >/dev/null 2>&1; then
            TELEGRAM_SEND_OK=0
        fi
    fi
    if [ "$TELEGRAM_SEND_OK" -ne 1 ]; then
        STATUS="failed"
        printf '%s\n- 텔레그램 전송 실패(타임아웃/오류)\n' "$TELEGRAM_MESSAGE" > "$FINAL_MSG_FILE"
    fi
else
    if [ "$REQUIRE_TELEGRAM_REPORT_VALUE" = "1" ]; then
        STATUS="failed"
        printf '%s\n- 텔레그램 전송 필수인데 설정/스크립트가 없어 실패 처리되었습니다.\n' "$TELEGRAM_MESSAGE" > "$FINAL_MSG_FILE"
        echo "❌ Telegram report is required but TELEGRAM_BOT_TOKEN/TELEGRAM_CHAT_ID/notifier is missing." >&2
    else
        echo "Warning: Telegram env or notifier missing. Skipped Telegram send." >&2
    fi
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
