#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Load environment variables
if [ -f core/.env ]; then
    set -a
    # shellcheck disable=SC1091
    source core/.env
    set +a
fi

# Semantic contract policy defaults: keep complex scenarios aligned with NL request runner.
: "${STEER_SEMANTIC_FAIL_ON_TRUNCATION:=1}"
: "${STEER_SEMANTIC_REQUIRE_APP_SCOPE:=1}"

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
NOTIFIER_TIMEOUT_SEC="${STEER_NOTIFIER_TIMEOUT_SEC:-120}"
REQUIRE_PRIMARY_PLANNER_VALUE="${STEER_REQUIRE_PRIMARY_PLANNER:-1}"
LOCK_DISABLED_VALUE="${STEER_LOCK_DISABLED:-0}"
APPROVAL_ASK_FALLBACK_VALUE="${STEER_APPROVAL_ASK_FALLBACK:-deny}"
TEST_MODE_VALUE="${STEER_TEST_MODE:-0}"

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
REQUIRE_TELEGRAM_REPORT_VALUE="${STEER_REQUIRE_TELEGRAM_REPORT:-1}"
DETERMINISTIC_GOAL_AUTOPLAN_VALUE="${STEER_DETERMINISTIC_GOAL_AUTOPLAN:-}"
SCENARIO_IDS_RAW="${STEER_SCENARIO_IDS:-1,2,3,4,5}"
if [ -z "$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" ]; then
    DETERMINISTIC_GOAL_AUTOPLAN_VALUE="1"
fi
REQUIRE_MAIL_BODY_VALUE="${STEER_REQUIRE_MAIL_BODY:-1}"
REQUIRE_MAIL_SUBJECT_VALUE="${STEER_REQUIRE_MAIL_SUBJECT:-1}"
REQUIRE_SENT_MAILBOX_EVIDENCE_VALUE="${STEER_REQUIRE_SENT_MAILBOX_EVIDENCE:-1}"
MAIL_TO_TARGET="${STEER_DEFAULT_MAIL_TO:-$(git config --get user.email 2>/dev/null || true)}"
OPENAI_PREFLIGHT_REQUIRED_VALUE="${STEER_PREFLIGHT_REQUIRE_OPENAI_KEY:-0}"

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
CURRENT_SCENARIO_START_EPOCH=0
CURRENT_LOG_FILE=""
CURRENT_INPUT_GUARD_ABORTED=0
CURRENT_INPUT_GUARD_ABORT_REASON=""
SELECTED_SCENARIO_IDS=""
SELECTED_SCENARIO_COUNT=0

CONTRACT_FILE_RAW="${STEER_SCENARIO_CONTRACT_FILE:-configs/complex_scenario_contracts.sh}"
if [ -z "${CONTRACT_FILE_RAW#/}" ]; then
    CONTRACT_FILE="$CONTRACT_FILE_RAW"
else
    CONTRACT_FILE="${SCRIPT_DIR}/${CONTRACT_FILE_RAW}"
fi
if [ ! -f "$CONTRACT_FILE" ]; then
    echo "❌ 시나리오 계약 파일을 찾을 수 없습니다: $CONTRACT_FILE"
    exit 1
fi
# shellcheck disable=SC1090
source "$CONTRACT_FILE"

normalize_scenario_ids() {
    local raw="$1"
    local normalized=""
    local token=""
    for token in $(printf '%s' "$raw" | tr ',/' ' '); do
        token="$(printf '%s' "$token" | tr -d '[:space:]')"
        case "$token" in
            1|2|3|4|5)
                if [[ " $normalized " != *" $token "* ]]; then
                    normalized="${normalized} ${token}"
                fi
                ;;
            *)
                ;;
        esac
    done
    printf '%s\n' "${normalized# }"
}

should_run_scenario() {
    local id="$1"
    [[ " ${SELECTED_SCENARIO_IDS} " == *" ${id} "* ]]
}

SELECTED_SCENARIO_IDS="$(normalize_scenario_ids "$SCENARIO_IDS_RAW")"
if [ -z "$SELECTED_SCENARIO_IDS" ]; then
    echo "❌ 유효한 STEER_SCENARIO_IDS 값이 없습니다: '${SCENARIO_IDS_RAW}'"
    echo "   허용 값: 1,2,3,4,5 (예: STEER_SCENARIO_IDS=1,3,5)"
    exit 1
fi
SELECTED_SCENARIO_COUNT="$(printf '%s\n' "$SELECTED_SCENARIO_IDS" | wc -w | tr -d ' ')"

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

echo "🔧 STEER_SCENARIO_MODE=${SCENARIO_MODE_VALUE} (0=LLM planning, 1=fallback scenario mode)"
echo "📸 STEER_NODE_CAPTURE=1, STEER_NODE_CAPTURE_ALL=${NODE_CAPTURE_ALL_VALUE}"
echo "🧪 STEER_TEST_MODE=${TEST_MODE_VALUE}"
echo "📨 STEER_REQUIRE_MAIL_BODY=${REQUIRE_MAIL_BODY_VALUE}"
echo "📝 STEER_REQUIRE_MAIL_SUBJECT=${REQUIRE_MAIL_SUBJECT_VALUE}"
echo "📬 STEER_REQUIRE_SENT_MAILBOX_EVIDENCE=${REQUIRE_SENT_MAILBOX_EVIDENCE_VALUE}"
echo "🧩 STEER_SCENARIO_IDS=${SELECTED_SCENARIO_IDS}"
echo "🧯 STEER_FAIL_ON_FALLBACK=${FAIL_ON_FALLBACK_VALUE} (1=mark failed on fallback action)"
echo "🧭 STEER_DETERMINISTIC_GOAL_AUTOPLAN=${DETERMINISTIC_GOAL_AUTOPLAN_VALUE}"
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

semantic_log_location_allowed_as_app_scope() {
    case "${1:-}" in
        LOG_MAIL_SUBJECT|LOG_MAIL_RECIPIENT|LOG_MAIL_BODY|LOG_NOTE_BODY|LOG_TEXTEDIT_BODY|LOG_MAIL_SEND|LOG_MAIL_WRITE_SUBJECT|LOG_MAIL_WRITE_RECIPIENT|LOG_MAIL_WRITE_BODY_LEN|LOG_MAIL_FLOW_DRAFT)
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

extract_expected_recipients_from_request() {
    local source_text="${1:-}"
    [ -z "$source_text" ] && return 0

    local rust_recipients=""
    if rust_recipients="$(extract_semantic_contract_with_rust "recipients" "$source_text")"; then
        if [ -n "$rust_recipients" ]; then
            printf '%s\n' "$rust_recipients" | awk 'NF > 0 && !seen[$0]++'
            return 0
        fi
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
    local source_text="${1:-}"
    [ -z "$source_text" ] && return 1
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
    recipients="$(extract_expected_recipients_from_request "$source_text" || true)"
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

selected_scenarios_require_mail() {
    local sid=""
    for sid in ${SELECTED_SCENARIO_IDS}; do
        if complex_scenario_required_artifacts "$sid" | grep -Fxq "mail_send"; then
            return 0
        fi
    done
    return 1
}

SEMANTIC_CONTRACT_RUST_BIN=""

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
    case "${STEER_SEMANTIC_REQUIRE_RUST_CONTRACT:-1}" in
        0|false|FALSE|no|NO|off|OFF)
            return 1
            ;;
        *)
            return 0
            ;;
    esac
}

semantic_allow_scenario_fallback() {
    case "${STEER_SEMANTIC_SCHEMA_ONLY:-1}" in
        1|true|TRUE|yes|YES|on|ON)
            return 1
            ;;
    esac
    case "${STEER_SEMANTIC_ALLOW_SCENARIO_FALLBACK:-0}" in
        1|true|TRUE|yes|YES|on|ON)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
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

    if [ "${STEER_PREFLIGHT_FOCUS_HANDOFF:-1}" = "1" ]; then
        local focus_activate_out=""
        local focus_front_out=""
        local focus_front=""
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

    if [ "$OPENAI_PREFLIGHT_REQUIRED_VALUE" = "1" ] && [ "$SCENARIO_MODE_VALUE" = "0" ] && [ -z "$CLI_LLM_VALUE" ] && ! has_openai_key_configured; then
        echo "❌ Preflight failed: OPENAI_API_KEY is not set."
        echo "   Fix: 기본 OpenAI 경로를 쓰려면 .env/core/.env 또는 현재 셸에 OPENAI_API_KEY를 설정하세요."
        echo "   대안: STEER_CLI_LLM 설정 또는 STEER_SCENARIO_MODE=1(테스트 전용) 사용."
        failed=1
    elif [ "$SCENARIO_MODE_VALUE" = "0" ] && [ -z "$CLI_LLM_VALUE" ] && ! has_openai_key_configured; then
        echo "ℹ️ OPENAI_API_KEY 미설정: preflight 강제는 비활성(STEER_PREFLIGHT_REQUIRE_OPENAI_KEY=0)."
        echo "   필요하면 STEER_CLI_LLM을 지정하거나 STEER_PREFLIGHT_REQUIRE_OPENAI_KEY=1로 엄격 모드를 켜세요."
    elif [ "$SCENARIO_MODE_VALUE" = "0" ] && [ -z "$CLI_LLM_VALUE" ]; then
        echo "✅ Preflight: OPENAI_API_KEY detected (env or .env)."
    else
        echo "ℹ️ Preflight: OPENAI_API_KEY not required in current mode (CLI/scenario path)."
    fi

    local require_mail_send_preflight=0
    if [ "${STEER_REQUIRE_MAIL_SEND:-0}" = "1" ] || selected_scenarios_require_mail; then
        require_mail_send_preflight=1
    fi
    if [ "$require_mail_send_preflight" -eq 1 ] && [ -z "$MAIL_TO_TARGET" ]; then
        echo "❌ Preflight failed: mail send target is empty."
        echo "   Fix: STEER_DEFAULT_MAIL_TO 또는 git user.email 을 설정하세요."
        failed=1
    fi

    if semantic_require_rust_contract; then
        if [ "${STEER_USE_RUST_SEMANTIC_CONTRACT:-1}" != "1" ]; then
            echo "❌ Preflight failed: STEER_SEMANTIC_REQUIRE_RUST_CONTRACT=1 이면 STEER_USE_RUST_SEMANTIC_CONTRACT=1 이어야 합니다."
            failed=1
        elif ! resolve_semantic_contract_rust_bin >/dev/null 2>&1; then
            echo "❌ Preflight failed: semantic_contract_rs 바이너리를 찾거나 빌드할 수 없습니다."
            echo "   Fix: core에서 cargo build --bin semantic_contract_rs 실행 또는 STEER_SEMANTIC_CONTRACT_AUTO_BUILD=1 확인."
            failed=1
        else
            echo "✅ Preflight: Rust semantic contract parser available."
        fi
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
    local max_evidence_lines="${STEER_TELEGRAM_EVIDENCE_MAX_LINES:-18}"
    if ! [[ "$max_chars" =~ ^[0-9]+$ ]]; then
        max_chars=3300
    fi
    if ! [[ "$max_evidence_lines" =~ ^[0-9]+$ ]]; then
        max_evidence_lines=18
    fi
    local compressed="$message"
    if [ "${#compressed}" -gt "$max_chars" ]; then
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
    fi
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
    local log_file="$1"
    local phase="$2"
    local status="$3"
    local details="$4"
    [ -z "$log_file" ] && return 0
    local ts
    ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'RUN_ATTEMPT|phase=%s|status=%s|details=%s|ts=%s\n' \
        "$phase" "$status" "$details" "$ts" >> "$log_file"
    if command -v jq >/dev/null 2>&1; then
        local payload
        payload="$(jq -cn \
            --arg phase "$phase" \
            --arg status "$status" \
            --arg details "$details" \
            --arg ts "$ts" \
            '{type:"run.attempt",phase:$phase,status:$status,details:$details,ts:$ts}')"
        printf 'RUN_ATTEMPT_JSON|%s\n' "$payload" >> "$log_file"
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

mail_outgoing_count() {
    local timeout_sec="${STEER_SEMANTIC_OSASCRIPT_TIMEOUT_SEC:-30}"
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
    local prompt="$1"
    local log_file="$2"
    local node_dir="$3"
    local use_guard="${STEER_PAUSE_ON_USER_INPUT:-1}"
    CURRENT_INPUT_GUARD_ABORTED=0
    CURRENT_INPUT_GUARD_ABORT_REASON=""
    if [ "$use_guard" != "1" ]; then
        if [ -n "$CLI_LLM_VALUE" ]; then
            STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
                STEER_CLI_LLM="$CLI_LLM_VALUE" \
                STEER_NODE_CAPTURE=1 \
                STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
                STEER_NODE_CAPTURE_DIR="$node_dir" \
                STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
                STEER_APPROVAL_ASK_FALLBACK="$APPROVAL_ASK_FALLBACK_VALUE" \
                STEER_TEST_MODE="$TEST_MODE_VALUE" \
                STEER_DETERMINISTIC_GOAL_AUTOPLAN="$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" \
                cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$prompt" &> "$log_file"
        else
            STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
                STEER_NODE_CAPTURE=1 \
                STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
                STEER_NODE_CAPTURE_DIR="$node_dir" \
                STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
                STEER_APPROVAL_ASK_FALLBACK="$APPROVAL_ASK_FALLBACK_VALUE" \
                STEER_TEST_MODE="$TEST_MODE_VALUE" \
                STEER_DETERMINISTIC_GOAL_AUTOPLAN="$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" \
                cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$prompt" &> "$log_file"
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
    local run_pid=""

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
            STEER_NODE_CAPTURE_DIR="$node_dir" \
            STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
            STEER_APPROVAL_ASK_FALLBACK="$APPROVAL_ASK_FALLBACK_VALUE" \
            STEER_TEST_MODE="$TEST_MODE_VALUE" \
            STEER_DETERMINISTIC_GOAL_AUTOPLAN="$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" \
            cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$prompt" &> "$log_file" &
    else
        STEER_SCENARIO_MODE="$SCENARIO_MODE_VALUE" \
            STEER_NODE_CAPTURE=1 \
            STEER_NODE_CAPTURE_ALL="$NODE_CAPTURE_ALL_VALUE" \
            STEER_NODE_CAPTURE_DIR="$node_dir" \
            STEER_LOCK_DISABLED="$LOCK_DISABLED_VALUE" \
            STEER_APPROVAL_ASK_FALLBACK="$APPROVAL_ASK_FALLBACK_VALUE" \
            STEER_TEST_MODE="$TEST_MODE_VALUE" \
            STEER_DETERMINISTIC_GOAL_AUTOPLAN="$DETERMINISTIC_GOAL_AUTOPLAN_VALUE" \
            cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$prompt" &> "$log_file" &
    fi
    run_pid=$!

    while kill -0 "$run_pid" 2>/dev/null; do
        if [ -f "$log_file" ]; then
            local new_item_live_count=0
            new_item_live_count="$(grep -Eic "$live_new_item_pattern" "$log_file" 2>/dev/null || true)"
            if [ "${new_item_live_count:-0}" -gt "$live_new_item_limit" ]; then
                CURRENT_INPUT_GUARD_ABORTED=1
                CURRENT_INPUT_GUARD_ABORT_REASON="new_item_flood(${new_item_live_count}>${live_new_item_limit})"
                echo "⛔️ [InputGuard] Abort run: ${CURRENT_INPUT_GUARD_ABORT_REASON}"
                echo "⛔️ [InputGuard] Abort run: ${CURRENT_INPUT_GUARD_ABORT_REASON}" >> "$log_file"
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
            local front_app=""
            front_app="$(get_frontmost_app)"
            if [ "$paused" -eq 0 ] && [ "$idle_sec" -le "$active_threshold" ] && should_pause_for_user_input "$front_app"; then
                kill -STOP "$run_pid" >/dev/null 2>&1 || true
                pkill -STOP -P "$run_pid" >/dev/null 2>&1 || true
                paused=1
                pause_count=$((pause_count + 1))
                pause_started_epoch="$(date +%s)"
                echo "⏸️ [InputGuard] Paused run (front_app=${front_app}, idle=${idle_sec}s, count=${pause_count})"
                echo "⏸️ [InputGuard] Paused run (front_app=${front_app}, idle=${idle_sec}s, count=${pause_count})" >> "$log_file"
                if [ "$max_pauses" -gt 0 ] && [ "$pause_count" -ge "$max_pauses" ]; then
                    CURRENT_INPUT_GUARD_ABORTED=1
                    CURRENT_INPUT_GUARD_ABORT_REASON="max_pauses_exceeded(${pause_count}/${max_pauses})"
                    echo "⛔️ [InputGuard] Abort run: ${CURRENT_INPUT_GUARD_ABORT_REASON}"
                    echo "⛔️ [InputGuard] Abort run: ${CURRENT_INPUT_GUARD_ABORT_REASON}" >> "$log_file"
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
                echo "▶️ [InputGuard] Resumed run (idle=${idle_sec}s)" >> "$log_file"
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
                    CURRENT_INPUT_GUARD_ABORTED=1
                    CURRENT_INPUT_GUARD_ABORT_REASON="max_pause_seconds_exceeded(${total_paused_seconds}+${current_pause}/${max_pause_seconds})"
                    echo "⛔️ [InputGuard] Abort run: ${CURRENT_INPUT_GUARD_ABORT_REASON}"
                    echo "⛔️ [InputGuard] Abort run: ${CURRENT_INPUT_GUARD_ABORT_REASON}" >> "$log_file"
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
    echo "🧾 [InputGuard] pause_count=${pause_count}" >> "$log_file"
    echo "🧾 [InputGuard] paused_seconds=${total_paused_seconds}"
    echo "🧾 [InputGuard] paused_seconds=${total_paused_seconds}" >> "$log_file"
    if [ "${CURRENT_INPUT_GUARD_ABORTED:-0}" = "1" ] && [ "$exit_code" -eq 0 ]; then
        exit_code=124
    fi
    return $exit_code
}

extract_latest_notes_target_from_log() {
    local log_file="$1"
    [ -f "$log_file" ] || return 0
    local line=""
    local note_id=""
    local note_name=""
    line="$(grep -E 'note_id=|"note_id"|note_name=|"note_name"' "$log_file" 2>/dev/null | tail -n 1 || true)"
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
    local log_file="$1"
    [ -f "$log_file" ] || return 0
    local line=""
    local doc_id=""
    local doc_name=""
    line="$(grep -E 'doc_id=|"doc_id"|doc_name=|"doc_name"' "$log_file" 2>/dev/null | tail -n 1 || true)"
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

token_presence_location_scoped_docs() {
    local token="$1"
    local marker="$2"
    local log_file="$3"
    local notes_target=""
    local textedit_target=""
    local note_id=""
    local note_name=""
    local doc_id=""
    local doc_name=""

    [ -z "$token" ] && return 0
    [ -z "$marker" ] && return 0
    [ -f "$log_file" ] || return 0

    notes_target="$(extract_latest_notes_target_from_log "$log_file" || true)"
    textedit_target="$(extract_latest_textedit_target_from_log "$log_file" || true)"

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
    local skip_sent_mail_scan="${STEER_SEMANTIC_DISABLE_SENT_MAIL_SCAN:-0}"
    local allow_log_evidence="${STEER_SEMANTIC_ALLOW_LOG_EVIDENCE:-0}"

    if [ "$require_marker" = "1" ] && [ -z "$marker" ]; then
        printf '%s\n' "MARKER_REQUIRED"
        return 0
    fi

    if [ -n "${CURRENT_LOG_FILE:-}" ]; then
        scoped_doc_location="$(token_presence_location_scoped_docs "$token" "$marker" "$CURRENT_LOG_FILE")"
        if [ -n "$scoped_doc_location" ] && ! semantic_location_missing "$scoped_doc_location"; then
            printf '%s\n' "$scoped_doc_location"
            return 0
        fi
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
            -- TextEdit may not be running; continue to Mail checks.
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
    if semantic_location_missing "$result" && [ "$allow_log_evidence" = "1" ] && [ -n "${CURRENT_LOG_FILE:-}" ]; then
        log_location="$(token_presence_location_from_log "$token" "$CURRENT_LOG_FILE" "$marker" "$require_marker")"
        if [ -n "$log_location" ]; then
            result="$log_location"
        fi
    fi
    printf '%s\n' "$result"
}

token_presence_location_from_log() {
    local token="$1"
    local log_file="$2"
    local marker="${3:-}"
    local require_marker="${4:-1}"
    [ -z "$token" ] && return 0
    [ -f "$log_file" ] || return 0

    local lines=""
    lines="$(grep -F -- "$token" "$log_file" 2>/dev/null | tail -n 200 || true)"
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
    local timeout_sec="${STEER_OSASCRIPT_TIMEOUT_SEC:-15}"
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

# Run agent command and detect logical failures from logs as well as exit code.
run_agent_scenario() {
    local prompt=$1
    local log_file=$2
    local scenario_num=$3
    local hard_fatal_pattern='Failed to acquire lock|thread .* panicked|FATAL ERROR|⛔️|LLM not available for surf mode|Preflight failed|Surf failed|Execution Error|SCHEMA_ERROR'
    local soft_fatal_pattern='Supervisor escalated|PLAN_REJECTED|LLM Refused'
    local fatal_pattern="$hard_fatal_pattern"
    if [ "${STEER_FATAL_STRICT:-0}" = "1" ]; then
        fatal_pattern="${hard_fatal_pattern}|${soft_fatal_pattern}"
    fi
    local node_dir="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}_nodes"

    CURRENT_SCENARIO_START_EPOCH="$(date +%s)"
    if ! run_surf_with_input_guard "$prompt" "$log_file" "$node_dir"; then
        if [ "${CURRENT_INPUT_GUARD_ABORTED:-0}" = "1" ]; then
            log_run_attempt \
                "$log_file" \
                "input_guard_abort" \
                "failed" \
                "${CURRENT_INPUT_GUARD_ABORT_REASON:-unknown}"
        fi
        return 1
    fi

    if grep -Eq "$fatal_pattern" "$log_file"; then
        return 1
    fi

    if [ "$FAIL_ON_FALLBACK_VALUE" = "1" ] && grep -Eiq "fallback action|FALLBACK_ACTION:" "$log_file"; then
        return 1
    fi

    local terminal_status=""
    for status_name in blocked approval_required manual_required; do
        if run_attempt_phase_status_hit "$log_file" "execution_end" "$status_name"; then
            terminal_status="$status_name"
            break
        fi
    done
    if [ -n "$terminal_status" ]; then
        log_run_attempt \
            "$log_file" \
            "scenario_terminal_status" \
            "$terminal_status" \
            "execution_end=${terminal_status}"
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
    local scenario_request="${6:-$scenario_goal}"
    local fallback_screenshot="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}.png"
    local telegram_main_image=""
    CURRENT_LOG_FILE="$log_file"
    local terminal_status=""

    local semantic_lines=""
    local semantic_missing=0
    local mail_subject_for_verify=""
    local mail_proof_status=""
    local mail_proof_recipient=""
    local mail_proof_subject=""
    local mail_proof_body_len="-1"
    local mail_proof_draft_id=""
    local expected_tokens=()
    local merged_tokens=()
    local required_artifacts=()
    local require_semantic_tokens=0
    local require_mail_send=0
    local require_node_capture=0
    local semantic_contract_rust_error=0
    local semantic_contract_rust_error_detail=""
    local semantic_contract_rust_warning_detail=""

    local allow_static_scenario_contract=0
    case "${STEER_SEMANTIC_ALLOW_SCENARIO_CONTRACT_TOKENS:-0}" in
        1|true|TRUE|yes|YES|on|ON)
            allow_static_scenario_contract=1
            ;;
    esac
    if [ "$allow_static_scenario_contract" -eq 1 ]; then
        while IFS= read -r token; do
            [ -z "$token" ] && continue
            expected_tokens+=("$token")
        done < <(complex_scenario_expected_tokens "$scenario_num")
    fi
    local rust_tokens=""
    local allow_scenario_fallback=0
    if semantic_allow_scenario_fallback; then
        allow_scenario_fallback=1
    fi
    if rust_tokens="$(extract_semantic_contract_with_rust "tokens" "$scenario_request")"; then
        if [ -n "$rust_tokens" ]; then
            while IFS= read -r token; do
                [ -z "$token" ] && continue
                expected_tokens+=("$token")
            done < <(printf '%s\n' "$rust_tokens")
        elif semantic_require_rust_contract; then
            if [ "$allow_scenario_fallback" -eq 1 ] && [ "${#expected_tokens[@]}" -gt 0 ]; then
                semantic_contract_rust_warning_detail="semantic_contract_rs returned empty tokens (fallback to scenario contract tokens)"
            else
                semantic_contract_rust_error=1
                semantic_contract_rust_error_detail="semantic_contract_rs returned empty tokens"
            fi
        fi
    elif semantic_require_rust_contract; then
        if [ "$allow_scenario_fallback" -eq 1 ] && [ "${#expected_tokens[@]}" -gt 0 ]; then
            semantic_contract_rust_warning_detail="semantic_contract_rs unavailable (fallback to scenario contract tokens)"
        else
            semantic_contract_rust_error=1
            if [ "${STEER_USE_RUST_SEMANTIC_CONTRACT:-1}" != "1" ]; then
                semantic_contract_rust_error_detail="STEER_USE_RUST_SEMANTIC_CONTRACT=1 required"
            else
                semantic_contract_rust_error_detail="semantic_contract_rs unavailable"
            fi
        fi
    fi
    mail_subject_for_verify="$(complex_scenario_mail_subject "$scenario_num")"
    while IFS= read -r artifact; do
        [ -z "$artifact" ] && continue
        required_artifacts+=("$artifact")
    done < <(complex_scenario_required_artifacts "$scenario_num")

    for token in "${expected_tokens[@]}"; do
        [ -z "$token" ] && continue
        token="$(normalize_semantic_token "$token")"
        [ -z "$token" ] && continue
        if is_noise_token "$token"; then
            continue
        fi
        merged_tokens+=("$token")
    done
    expected_tokens=()
    while IFS= read -r token; do
        [ -z "$token" ] && continue
        expected_tokens+=("$token")
    done < <(printf '%s\n' "${merged_tokens[@]}" | awk 'NF > 0 && !seen[$0]++')
    local token_truncated=0
    local default_token_cap=384
    local request_len=${#scenario_request}
    local token_cap=0
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
    if [ "$token_cap" -gt 0 ] && [ "${#expected_tokens[@]}" -gt "$token_cap" ]; then
        token_truncated=1
        expected_tokens=("${expected_tokens[@]:0:$token_cap}")
    fi
    if [ -n "$CURRENT_SCENARIO_MARKER" ]; then
        local marker_kept=0
        for token in "${expected_tokens[@]}"; do
            if [ "$token" = "$CURRENT_SCENARIO_MARKER" ]; then
                marker_kept=1
                break
            fi
        done
        if [ "$marker_kept" -eq 0 ]; then
            if [ "$token_cap" -gt 0 ] && [ "${#expected_tokens[@]}" -ge "$token_cap" ]; then
                expected_tokens=("${expected_tokens[@]:0:$((token_cap - 1))}")
            fi
            expected_tokens+=("$CURRENT_SCENARIO_MARKER")
        fi
    fi
    for artifact in "${required_artifacts[@]}"; do
        case "$artifact" in
            semantic_tokens)
                require_semantic_tokens=1
                ;;
            mail_send)
                require_mail_send=1
                ;;
            node_capture)
                require_node_capture=1
                ;;
        esac
    done

    if proof_line="$(mail_send_proof_from_log "$log_file")"; then
        IFS='|' read -r mail_proof_status mail_proof_recipient mail_proof_subject mail_proof_body_len mail_proof_draft_id <<< "$proof_line"
    fi

    if [ "${STEER_SEMANTIC_VERIFY:-1}" = "1" ]; then
        if [ "$semantic_contract_rust_error" -eq 1 ]; then
            status="failed"
            semantic_lines="${semantic_lines}- 의미검증 계약 위반: Rust semantic contract 추출 실패 (${semantic_contract_rust_error_detail:-unknown})"$'\n'
        elif [ -n "$semantic_contract_rust_warning_detail" ]; then
            semantic_lines="${semantic_lines}- 의미검증 계약 경고: ${semantic_contract_rust_warning_detail}"$'\n'
        fi
        local semantic_checked=0
        for token in "${expected_tokens[@]}"; do
            [ -z "$token" ] && continue
            semantic_checked=$((semantic_checked + 1))
            normalized_token="$(normalize_semantic_token "$token")"
            location=""
            if [ -n "$mail_proof_subject" ] && [ "$mail_proof_status" = "sent_confirmed" ] && [ "$token" = "$mail_proof_subject" ]; then
                location="LOG_MAIL_SUBJECT"
            elif [ -n "$mail_proof_recipient" ] && [ "$mail_proof_status" = "sent_confirmed" ] && [ "$token" = "$mail_proof_recipient" ]; then
                location="LOG_MAIL_RECIPIENT"
            else
                location="$(token_presence_location "$token" "$CURRENT_SCENARIO_MARKER" "$CURRENT_SCENARIO_START_EPOCH")"
            fi
            if semantic_location_missing "$location" && [ -n "$normalized_token" ] && [ "$normalized_token" != "$token" ]; then
                if [ -n "$mail_proof_subject" ] && [ "$mail_proof_status" = "sent_confirmed" ] && [ "$normalized_token" = "$mail_proof_subject" ]; then
                    location="LOG_MAIL_SUBJECT"
                elif [ -n "$mail_proof_recipient" ] && [ "$mail_proof_status" = "sent_confirmed" ] && [ "$normalized_token" = "$mail_proof_recipient" ]; then
                    location="LOG_MAIL_RECIPIENT"
                else
                    location="$(token_presence_location "$normalized_token" "$CURRENT_SCENARIO_MARKER" "$CURRENT_SCENARIO_START_EPOCH")"
                fi
            fi
            if [ "${STEER_SEMANTIC_REQUIRE_APP_SCOPE:-1}" = "1" ] && semantic_location_is_log "$location"; then
                if ! semantic_log_location_allowed_as_app_scope "$location"; then
                    location="LOG_ONLY_BLOCKED(${location})"
                fi
            fi
            if semantic_location_missing "$location"; then
                semantic_missing=$((semantic_missing + 1))
                semantic_lines="${semantic_lines}- 의미검증 ❌ \"${token}\" (location=${location})"$'\n'
            else
                semantic_lines="${semantic_lines}- 의미검증 ✅ \"${token}\" (location=${location})"$'\n'
            fi
        done
        semantic_lines="${semantic_lines}- 의미검증 토큰 수: ${semantic_checked}"$'\n'
        if [ "$token_truncated" -eq 1 ]; then
            semantic_lines="${semantic_lines}- 의미검증 토큰이 상한(${token_cap})으로 잘렸습니다(STEER_SEMANTIC_MAX_TOKENS 조정 필요)"$'\n'
            if [ "${STEER_SEMANTIC_FAIL_ON_TRUNCATION:-1}" = "1" ]; then
                status="failed"
                semantic_lines="${semantic_lines}- 계약 위반: 토큰 절단 발생으로 최종 상태를 failed로 강등"$'\n'
            fi
        fi
        if [ "$require_semantic_tokens" -eq 1 ] && [ "$semantic_checked" -eq 0 ]; then
            status="failed"
            semantic_lines="${semantic_lines}- 계약 위반: semantic token 계약이 비어 있습니다"$'\n'
        fi
        if [ -n "$CURRENT_SCENARIO_MARKER" ]; then
            semantic_lines="${semantic_lines}- 의미검증 run-scope marker: ${CURRENT_SCENARIO_MARKER}"$'\n'
        fi

        if [ "$semantic_missing" -gt 0 ]; then
            status="failed"
            semantic_lines="${semantic_lines}- 의미검증 실패로 최종 상태를 failed로 강등"$'\n'
        fi
    else
        semantic_lines="${semantic_lines}- 의미검증 비활성(STEER_SEMANTIC_VERIFY=0)"$'\n'
        if [ "$require_semantic_tokens" -eq 1 ]; then
            status="failed"
            semantic_lines="${semantic_lines}- 계약 위반: semantic_tokens가 required인데 의미검증이 비활성입니다"$'\n'
        fi
    fi

    local request_requires_mail=0
    if request_requires_mail_send "$scenario_request"; then
        request_requires_mail=1
    fi
    if [ "$require_mail_send" -eq 1 ] || [ "${STEER_REQUIRE_MAIL_SEND:-0}" = "1" ] || [ "$request_requires_mail" -eq 1 ]; then
        local mail_send_logged=0
        local mail_log_status="${mail_proof_status:-}"
        local mail_log_recipient="${mail_proof_recipient:-}"
        local mail_log_subject="${mail_proof_subject:-}"
        local mail_log_body_len="${mail_proof_body_len:--1}"
        local mail_log_draft_id="${mail_proof_draft_id:-}"
        local mail_write_recipient_seen=0
        local mail_write_subject_seen=0
        local mail_write_body_len="-1"
        if [ -n "$mail_log_draft_id" ]; then
            local write_line=""
            if write_line="$(mail_write_evidence_for_draft "$log_file" "$mail_log_draft_id")"; then
                IFS='|' read -r mail_write_recipient_seen mail_write_subject_seen mail_write_body_len <<< "$write_line"
            fi
        fi
        if [ "$mail_log_status" = "sent_confirmed" ]; then
            mail_send_logged=1
        elif grep -Eiq "Shortcut 'd'.*shift.*Mail sent|Mail send completed|\"send_status\"[[:space:]]*:[[:space:]]*\"sent_confirmed\"|MAIL_SEND_PROOF\\|status=sent_confirmed|EVIDENCE\\|target=mail\\|event=send\\|status=sent_confirmed" "$log_file"; then
            mail_send_logged=1
        fi
        local outgoing_count
        outgoing_count="$(mail_outgoing_count || echo -1)"
        local mail_verify_token="${CURRENT_SCENARIO_MARKER:-}"
        if [ -z "$mail_verify_token" ] && [ -n "$mail_subject_for_verify" ]; then
            mail_verify_token="$mail_subject_for_verify"
        fi
        local mail_sent_location="NOT_CHECKED"
        local mail_sent_ok=0
        if [ "$mail_send_logged" -eq 1 ]; then
            mail_sent_ok=1
            mail_sent_location="LOG_MAIL_SEND"
        fi
        if [ -n "$mail_verify_token" ]; then
            if [ "$mail_sent_ok" -ne 1 ]; then
                mail_sent_location="$(token_presence_location "$mail_verify_token" "$CURRENT_SCENARIO_MARKER" "$CURRENT_SCENARIO_START_EPOCH")"
                case "$mail_sent_location" in
                    MAIL_SENT_SUBJECT|MAIL_SENT_BODY)
                        mail_sent_ok=1
                        ;;
                esac
            fi
        fi
        local mail_subject_ok=1
        local mail_subject_location="SUBJECT_NOT_REQUIRED"
        if [ "$REQUIRE_MAIL_SUBJECT_VALUE" = "1" ]; then
            mail_subject_ok=0
            local trimmed_mail_subject=""
            trimmed_mail_subject="$(printf '%s' "${mail_log_subject:-}" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"
            if [ -n "$trimmed_mail_subject" ]; then
                mail_subject_ok=1
                mail_subject_location="LOG_MAIL_SUBJECT"
                if [ -n "$CURRENT_SCENARIO_MARKER" ] && ! printf '%s' "$trimmed_mail_subject" | grep -Fq "$CURRENT_SCENARIO_MARKER"; then
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
        local expected_recipients_raw
        expected_recipients_raw="${STEER_EXPECT_MAIL_RECIPIENTS:-}"
        if [ -z "$expected_recipients_raw" ]; then
            expected_recipients_raw="${STEER_EXPECT_MAIL_RECIPIENT:-$MAIL_TO_TARGET}"
        fi
        local expected_recipients=()
        while IFS= read -r recipient; do
            [ -z "$recipient" ] && continue
            expected_recipients+=("$recipient")
        done < <(
            printf '%s\n' "$expected_recipients_raw" \
                | tr ',;' '\n' \
                | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//' \
                | tr '[:upper:]' '[:lower:]' \
                | tr -d '[:space:]' \
                | awk 'NF > 0 && !seen[$0]++'
        )
        local expected_recipients_label="optional"
        if [ "${#expected_recipients[@]}" -gt 0 ]; then
            expected_recipients_label="$(printf '%s' "${expected_recipients[*]}" | tr ' ' ',')"
        fi
        local mail_recipient_location="RECIPIENT_NOT_REQUIRED"
        local mail_recipient_ok=1
        if [ "${#expected_recipients[@]}" -gt 0 ]; then
            local normalized_log_recipient
            normalized_log_recipient="$(printf '%s' "$mail_log_recipient" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"
            local missing_recipients=()
            for expected_recipient in "${expected_recipients[@]}"; do
                if ! printf '%s' "$expected_recipient" | grep -Eq '.+@.+\..+'; then
                    continue
                fi
                local recipient_single_ok=0
                local recipient_single_location="NOT_FOUND"
                if [ -n "$normalized_log_recipient" ] && [ "$normalized_log_recipient" = "$expected_recipient" ]; then
                    recipient_single_ok=1
                    recipient_single_location="LOG_MAIL_RECIPIENT"
                elif [ "${mail_write_recipient_seen:-0}" = "1" ]; then
                    recipient_single_ok=1
                    recipient_single_location="LOG_MAIL_WRITE_RECIPIENT"
                else
                    recipient_single_location="$(mail_sent_recipient_location "$expected_recipient" "$CURRENT_SCENARIO_MARKER" "$CURRENT_SCENARIO_START_EPOCH")"
                    if [ "$recipient_single_location" = "MAIL_SENT_RECIPIENT" ]; then
                        recipient_single_ok=1
                    fi
                fi
                if [ "$recipient_single_ok" -ne 1 ]; then
                    mail_recipient_ok=0
                    missing_recipients+=("${expected_recipient}@${recipient_single_location}")
                fi
            done
            if [ "$mail_recipient_ok" -eq 1 ]; then
                if [ -n "$normalized_log_recipient" ]; then
                    mail_recipient_location="LOG_MAIL_RECIPIENT"
                elif [ "${mail_write_recipient_seen:-0}" = "1" ]; then
                    mail_recipient_location="LOG_MAIL_WRITE_RECIPIENT"
                else
                    mail_recipient_location="MAIL_SENT_RECIPIENT"
                fi
            elif [ "${#missing_recipients[@]}" -gt 0 ]; then
                mail_recipient_location="MISSING[$(printf '%s' "${missing_recipients[*]}" | tr ' ' ',')]"
            fi
        fi
        local mail_body_ok=1
        local mail_body_location="BODY_NOT_REQUIRED"
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
                if ! semantic_log_location_allowed_as_app_scope "$mail_sent_location"; then
                    mail_sent_ok=0
                    mail_sent_location="LOG_ONLY_BLOCKED(${mail_sent_location})"
                fi
            fi
            if [ "$mail_subject_ok" -eq 1 ] && semantic_location_is_log "$mail_subject_location"; then
                if ! semantic_log_location_allowed_as_app_scope "$mail_subject_location"; then
                    mail_subject_ok=0
                    mail_subject_location="LOG_ONLY_BLOCKED(${mail_subject_location})"
                fi
            fi
            if [ "$mail_recipient_ok" -eq 1 ] && semantic_location_is_log "$mail_recipient_location"; then
                if ! semantic_log_location_allowed_as_app_scope "$mail_recipient_location"; then
                    mail_recipient_ok=0
                    mail_recipient_location="LOG_ONLY_BLOCKED(${mail_recipient_location})"
                fi
            fi
            if [ "$mail_body_ok" -eq 1 ] && semantic_location_is_log "$mail_body_location"; then
                if ! semantic_log_location_allowed_as_app_scope "$mail_body_location"; then
                    mail_body_ok=0
                    mail_body_location="LOG_ONLY_BLOCKED(${mail_body_location})"
                fi
            fi
        fi
        local mail_mailbox_evidence_ok=0
        local mail_mailbox_evidence_location="$mail_sent_location"
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
            semantic_lines="${semantic_lines}- 메일 발송 검증 ✅ (send-action 로그/증거 + recipients=${expected_recipients_label}, outgoing=${outgoing_count}, sent_location=${mail_sent_location}, mailbox_evidence=${mail_mailbox_evidence_location}, subject_location=${mail_subject_location}, body_location=${mail_body_location}, body_len=${mail_log_body_len:-n/a}, draft_id=${mail_log_draft_id:-n/a}, write_recipient=${mail_write_recipient_seen:-0}, write_subject=${mail_write_subject_seen:-0}, write_body_len=${mail_write_body_len:--1}, subject=${mail_log_subject:-n/a})"$'\n'
        else
            semantic_lines="${semantic_lines}- 메일 발송 검증 ❌ (send-action 로그=${mail_send_logged}, outgoing=${outgoing_count}, sent_location=${mail_sent_location}, mailbox_required=${REQUIRE_SENT_MAILBOX_EVIDENCE_VALUE}, mailbox_location=${mail_mailbox_evidence_location}, subject_required=${REQUIRE_MAIL_SUBJECT_VALUE}, subject_location=${mail_subject_location}, body_required=${REQUIRE_MAIL_BODY_VALUE}, body_location=${mail_body_location}, body_len=${mail_log_body_len:-n/a}, draft_id=${mail_log_draft_id:-n/a}, write_recipient=${mail_write_recipient_seen:-0}, write_subject=${mail_write_subject_seen:-0}, write_body_len=${mail_write_body_len:--1}, recipients=${expected_recipients_label}, recipient_location=${mail_recipient_location}, token=${mail_verify_token:-none})"$'\n'
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
    key_logs=$(grep -En "Goal completed by planner|Surf failed|Supervisor escalated|Preflight failed|Execution Error|SCHEMA_ERROR|PLAN_REJECTED|LLM Refused|fallback action|FALLBACK_ACTION:|Node evidence|MAIL_SEND_PROOF\\||EVIDENCE\\|" "$log_file" 2>/dev/null | tail -n 8 | sed -E 's/^[0-9]+://')
    if [ -z "$key_logs" ]; then
        key_logs=$(tail -n 3 "$log_file" 2>/dev/null | sed -E 's/^[[:space:]]+//')
    fi

    local evidence_lines=""
    local fallback_hit=0
    local cmd_n_guard_count=0
    local cmd_n_window_flood_guard_count=0
    if grep -Eiq "fallback action|FALLBACK_ACTION:" "$log_file" 2>/dev/null; then
        fallback_hit=1
    fi
    cmd_n_guard_count=$(grep -Ec 'cmd_n_loop_guard_block' "$log_file" 2>/dev/null || true)
    cmd_n_window_flood_guard_count=$(grep -Ec 'cmd_n_window_flood_block' "$log_file" 2>/dev/null || true)
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
    evidence_lines="${evidence_lines}- STEER_TEST_MODE=${TEST_MODE_VALUE}"$'\n'
    evidence_lines="${evidence_lines}- STEER_REQUIRE_MAIL_BODY=${REQUIRE_MAIL_BODY_VALUE}"$'\n'
    evidence_lines="${evidence_lines}- STEER_REQUIRE_MAIL_SUBJECT=${REQUIRE_MAIL_SUBJECT_VALUE}"$'\n'
    evidence_lines="${evidence_lines}- STEER_REQUIRE_SENT_MAILBOX_EVIDENCE=${REQUIRE_SENT_MAILBOX_EVIDENCE_VALUE}"$'\n'
    evidence_lines="${evidence_lines}- STEER_SEMANTIC_FAIL_ON_TRUNCATION=${STEER_SEMANTIC_FAIL_ON_TRUNCATION}"$'\n'
    evidence_lines="${evidence_lines}- STEER_SEMANTIC_REQUIRE_APP_SCOPE=${STEER_SEMANTIC_REQUIRE_APP_SCOPE}"$'\n'
    local diag_lines=""
    diag_lines="$(collect_diagnostic_event_lines || true)"
    if [ -n "$diag_lines" ]; then
        evidence_lines="${evidence_lines}- diagnostics tail (${STEER_DIAGNOSTIC_EVENTS_PATH:-scenario_results/diagnostic_events.jsonl})"$'\n'"${diag_lines}"$'\n'
    fi
    if [ "$fallback_hit" -eq 1 ]; then
        evidence_lines="${evidence_lines}- fallback 액션 감지됨(fallback action/FALLBACK_ACTION)"$'\n'
        if [ "$FAIL_ON_FALLBACK_VALUE" = "1" ]; then
            evidence_lines="${evidence_lines}- 정책상 fallback 감지 시 실패 처리(STEER_FAIL_ON_FALLBACK=1)"$'\n'
        fi
    fi
    if [ "$cmd_n_guard_count" -gt 0 ]; then
        status="failed"
        evidence_lines="${evidence_lines}- cmd+n 루프 가드 발동 횟수=${cmd_n_guard_count}"$'\n'
    fi
    if [ "$cmd_n_window_flood_guard_count" -gt 0 ]; then
        status="failed"
        evidence_lines="${evidence_lines}- cmd+n 창 폭증 가드 발동 횟수=${cmd_n_window_flood_guard_count}"$'\n'
    fi
    if [ "${CURRENT_INPUT_GUARD_ABORTED:-0}" = "1" ]; then
        evidence_lines="${evidence_lines}- 입력 가드 중단: ${CURRENT_INPUT_GUARD_ABORT_REASON:-unknown}"$'\n'
    fi
    evidence_lines="${evidence_lines}${semantic_lines}"

    local node_dir="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}_nodes"
    local node_count=0
    if [ -d "$node_dir" ]; then
        node_count=$(find "$node_dir" -maxdepth 1 -type f -name '*.png' | wc -l | tr -d ' ')
    fi
    if [ "$require_node_capture" -eq 1 ] && [ "$node_count" -eq 0 ]; then
        status="failed"
        evidence_lines="${evidence_lines}- 계약 위반: node_capture required인데 노드 캡처가 없습니다"$'\n'
    fi
    evidence_lines="${evidence_lines}- 노드 캡처 수: ${node_count}"$'\n'
    evidence_lines="${evidence_lines}- 노드 캡처 폴더: $(basename "$node_dir")"$'\n'
    local node_image_list_file="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}.telegram.node_images.txt"
    : > "$node_image_list_file"
    local node_step_summary=""
    local node_step_count=0
    local node_error_steps=""

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

        node_error_steps=$(awk '
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
        ' "$log_file" | sort -n)

        if [ -n "$node_last_rows" ]; then
            while IFS= read -r row; do
                [ -z "$row" ] && continue
                IFS='|' read -r _step_key _ord path step action phase app note <<< "$row"
                local node_status="✅ 실행"
                local step_has_error=0
                if [ -n "$node_error_steps" ] && printf '%s\n' "$node_error_steps" | grep -qx "$step"; then
                    step_has_error=1
                fi
                if [[ "$phase" == *error* ]] || [[ "$note" == *failed* ]] || [ "$step_has_error" -eq 1 ]; then
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
    evidence_lines="${evidence_lines}- 단계 상태는 노드 캡처 + Execution Error 로그 기준이며, 내용 충족 여부는 의미검증 라인 기준"$'\n'

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

    local judgement_summary="semantic_missing=${semantic_missing:-0}, mail_proof=${mail_proof_status:-none}, node_capture_required=${require_node_capture}, node_count=${node_count}"
    local fail_primary_reason="none"
    local retry_guide="동일 시나리오를 재실행해도 됩니다."
    if [ "$status" != "success" ]; then
        if [ -n "$terminal_status" ]; then
            fail_primary_reason="terminal_status=${terminal_status}"
            retry_guide="승인/수동 단계 해소 후 같은 시나리오를 다시 실행하세요."
        elif [ "$semantic_missing" -gt 0 ]; then
            fail_primary_reason="semantic_missing_tokens=${semantic_missing}"
            retry_guide="요구 토큰이 실제 앱 결과에 남도록 단계 입력을 보강하세요."
        elif [ "${fallback_hit:-0}" -eq 1 ]; then
            fail_primary_reason="fallback_detected"
            retry_guide="fallback 유도 원인(포커스/권한/플랜)을 먼저 제거하세요."
        elif [ "${cmd_n_window_flood_guard_count:-0}" -gt 0 ]; then
            fail_primary_reason="cmd_n_window_flood_guard"
            retry_guide="생성된 새 창을 정리한 뒤 재실행하세요."
        elif [ "${cmd_n_guard_count:-0}" -gt 0 ]; then
            fail_primary_reason="cmd_n_loop_guard"
            retry_guide="cmd+n 연속 시도를 줄이도록 플랜/앱 상태를 정리한 뒤 재실행하세요."
        else
            fail_primary_reason="evidence_or_runtime_failure"
            retry_guide="실패 근거 라인 기준으로 실패 단계만 수정 후 재실행하세요."
        fi
    fi

    local brief_mail_result_line="- 메일 발송 증거: 확인 필요"
    if [ "${mail_proof_status:-}" = "sent_confirmed" ]; then
        brief_mail_result_line="- 메일 정상 발송 완료 (${mail_proof_recipient:-unknown})"
    fi
    local brief_final_line="- 문제가 있어 재실행 필요 -> 실패"
    if [ "$status" = "success" ]; then
        brief_final_line="- 문제 없음 -> 성공"
    fi

    local telegram_message
    telegram_message=$(cat <<EOF
📌 시나리오 ${scenario_num} - 쉽게 말한 요약

🔄 뭘 했는지
- 요청한 자동 실행 체인을 끝까지 수행했고
- 단계별 캡처/실행 증거를 수집했고
- 결과를 검증 규칙으로 최종 판정했어요.

✅ 결과
- ${result_info}
${brief_mail_result_line}
${brief_final_line}

상태: ${status_label}
판정:
- ${judgement_summary}
- fail_reason=${fail_primary_reason}
재실행 가이드:
- ${retry_guide}
근거:
${evidence_lines}- 로그: $(basename "$log_file")
EOF
)
    telegram_message="$(compress_telegram_report "$telegram_message")"

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
            local node_image_count=0
            if [ -s "$node_image_list_file" ]; then
                notify_env=(TELEGRAM_EXTRA_IMAGE_LIST_FILE="$node_image_list_file")
                node_image_count="$(grep -Ec '^[^|]+' "$node_image_list_file" || true)"
                node_image_count="${node_image_count:-0}"
            fi
            local notifier_timeout
            notifier_timeout="$(compute_notifier_timeout "$NOTIFIER_TIMEOUT_SEC" "$node_image_count")"
            if [ -n "$telegram_main_image" ] && [ -f "$telegram_main_image" ]; then
                if ! send_telegram_with_timeout "$notifier_timeout" \
                    env TELEGRAM_DUMP_FINAL_PATH="$final_message_file" TELEGRAM_SKIP_REWRITE=1 TELEGRAM_VALIDATE_REPORT=1 TELEGRAM_REQUIRE_SEND="$REQUIRE_TELEGRAM_REPORT_VALUE" ${notify_env[@]+"${notify_env[@]}"} \
                    bash "$notifier" "$telegram_message" "$telegram_main_image" >/dev/null 2>&1; then
                    telegram_send_ok=0
                fi
            else
                if ! send_telegram_with_timeout "$notifier_timeout" \
                    env TELEGRAM_DUMP_FINAL_PATH="$final_message_file" TELEGRAM_SKIP_REWRITE=1 TELEGRAM_VALIDATE_REPORT=1 TELEGRAM_REQUIRE_SEND="$REQUIRE_TELEGRAM_REPORT_VALUE" ${notify_env[@]+"${notify_env[@]}"} \
                    bash "$notifier" "$telegram_message" >/dev/null 2>&1; then
                    telegram_send_ok=0
                fi
            fi
            if [ "$telegram_send_ok" -ne 1 ]; then
                printf '%s\n- 텔레그램 전송 실패(타임아웃/오류)\n' "$telegram_message" > "$final_message_file"
                status="failed"
            fi
        else
            if [ "$REQUIRE_TELEGRAM_REPORT_VALUE" = "1" ]; then
                echo "❌ Telegram report required but TELEGRAM_BOT_TOKEN/TELEGRAM_CHAT_ID is missing." >&2
                printf '%s\n- 텔레그램 전송 필수인데 TELEGRAM_BOT_TOKEN/TELEGRAM_CHAT_ID가 없어 실패 처리되었습니다.\n' "$telegram_message" > "$final_message_file"
                status="failed"
            else
                echo "Warning: TELEGRAM_BOT_TOKEN/TELEGRAM_CHAT_ID not set; skipped Telegram notification." >&2
            fi
        fi
    else
        if [ "$REQUIRE_TELEGRAM_REPORT_VALUE" = "1" ]; then
            echo "❌ Telegram report required but send_telegram_notification.sh is missing." >&2
            printf '%s\n- 텔레그램 전송 필수인데 notifier 스크립트가 없어 실패 처리되었습니다.\n' "$telegram_message" > "$final_message_file"
            status="failed"
        else
            echo "Warning: send_telegram_notification.sh not found; skipped Telegram notification." >&2
        fi
    fi
    
    log_run_attempt \
        "$log_file" \
        "scenario_${scenario_num}_final_judgement" \
        "$status" \
        "semantic_missing=${semantic_missing:-0},mail_proof=${mail_proof_status:-none},node_capture_required=${require_node_capture},telegram_required=${REQUIRE_TELEGRAM_REPORT_VALUE}"

    echo "Scenario ${scenario_num} finished with status: ${status}"
    echo "  - telegram raw: ${raw_message_file}"
    echo "  - telegram final: ${final_message_file}"
    if [ "$status" = "success" ]; then
        CURRENT_LOG_FILE=""
        return 0
    fi
    CURRENT_LOG_FILE=""
    return 1
}

if ! preflight_checks; then
    exit 1
fi
echo ""

# Scenario 1: Calendar -> Safari -> Notes -> Mail
if should_run_scenario 1; then
    echo "---------------------------------------------------"
    echo "📅 Scenario 1: Calendar → Safari → Notes → Mail"
    LOG_FILE="scenario_results/complex_scenario_1_${TIMESTAMP}.log"
    SCENARIO_GOAL="Multi-app draft chain without screen-reading dependency."
    CURRENT_SCENARIO_MARKER="$MARKER_S1"
    echo "Goal: ${SCENARIO_GOAL}"
    CMD="Calendar를 열고 전면으로 가져오세요. Notes를 열어 새 메모(Cmd+N)를 만들고 제목을 \"${SUBJECT_S1}\"로 입력한 뒤 아래 3줄을 그대로 입력하세요: \"Calendar opened\", \"Notes draft ready\", \"Mail prep pending\". 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. TextEdit를 열어 새 문서(Cmd+N)에 붙여넣기(Cmd+V)하고 다음 줄에 \"Shared via TextEdit\"를 입력하세요. 다음 줄에 \"${MARKER_S1}\"를 정확히 입력하세요. 다시 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. Mail을 열어 새 이메일(Cmd+N) 초안을 만들고 제목 \"${SUBJECT_S1}\"를 입력한 뒤 본문에 붙여넣기(Cmd+V)하세요. 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요."

    scenario_status="failed"
    if run_agent_scenario "$CMD" "$LOG_FILE" 1; then
        scenario_status="success"
    fi
    if capture_and_notify 1 "일정 브리핑 체인" "$scenario_status" "$LOG_FILE" "$SCENARIO_GOAL" "$CMD"; then
        echo "✅ Scenario 1 Complete."
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    else
        echo "❌ Scenario 1 Failed."
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
    sleep 5
else
    echo "⏭️  Scenario 1 skipped (STEER_SCENARIO_IDS=${SELECTED_SCENARIO_IDS})"
fi

# Scenario 2: Finder -> Notes -> TextEdit -> Mail
if should_run_scenario 2; then
    echo "---------------------------------------------------"
    echo "📂 Scenario 2: Finder → Notes → TextEdit → Mail"
    LOG_FILE="scenario_results/complex_scenario_2_${TIMESTAMP}.log"
    SCENARIO_GOAL="Finder/Notes/TextEdit/Mail transfer chain."
    CURRENT_SCENARIO_MARKER="$MARKER_S2"
    echo "Goal: ${SCENARIO_GOAL}"
    CMD="아래 순서를 정확히 지키세요. 1) Finder를 열고 Downloads 폴더를 전면으로 가져오세요. 2) Notes를 열고 새 메모(Cmd+N)를 만든 뒤 제목을 \"${SUBJECT_S2}\"로 입력하세요. 3) 본문에 다음 4줄을 그대로 입력하세요: \"1. invoice.pdf\", \"2. screenshot.png\", \"3. notes.txt\", \"${MARKER_S2}\". 4) 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. 5) TextEdit를 열고 새 문서(Cmd+N)에 붙여넣기(Cmd+V)한 뒤 다음 줄에 \"Shared via Notes\"를 입력하세요. 6) 다시 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. 7) Mail을 열고 새 이메일(Cmd+N) 초안을 만든 뒤 제목 \"${SUBJECT_S2}\"를 입력하고 본문에 붙여넣기(Cmd+V)하세요. 8) 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요. 9) 전송이 끝나면 done으로 종료하세요."

    scenario_status="failed"
    if run_agent_scenario "$CMD" "$LOG_FILE" 2; then
        scenario_status="success"
    fi
    if capture_and_notify 2 "다운로드 분류 체인" "$scenario_status" "$LOG_FILE" "$SCENARIO_GOAL" "$CMD"; then
        echo "✅ Scenario 2 Complete."
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    else
        echo "❌ Scenario 2 Failed."
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
    sleep 5
else
    echo "⏭️  Scenario 2 skipped (STEER_SCENARIO_IDS=${SELECTED_SCENARIO_IDS})"
fi

# Scenario 3: Calculator -> Notes -> TextEdit -> Mail
if should_run_scenario 3; then
    echo "---------------------------------------------------"
    echo "📈 Scenario 3: Calculator → Notes → TextEdit → Mail"
    LOG_FILE="scenario_results/complex_scenario_3_${TIMESTAMP}.log"
    SCENARIO_GOAL="Calculation + document handoff + mail send chain."
    CURRENT_SCENARIO_MARKER="$MARKER_S3"
    echo "Goal: ${SCENARIO_GOAL}"
    CMD="아래 순서를 정확히 지키세요. 1) Calculator를 열고 \"120*1300=\" 를 입력해 계산 화면을 준비하세요. 2) Notes를 열고 새 메모(Cmd+N)를 만든 뒤 제목을 \"${SUBJECT_S3}\"로 입력하세요. 3) 본문에 다음 3줄을 그대로 입력하세요: \"120*1300=\", \"Done\", \"${MARKER_S3}\". 4) 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. 5) TextEdit를 열고 새 문서(Cmd+N)에 붙여넣기(Cmd+V)한 뒤 다음 줄에 \"Calc verified\"를 입력하세요. 6) 다시 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. 7) Mail을 열고 새 이메일(Cmd+N) 초안을 만든 뒤 제목 \"${SUBJECT_S3}\"를 입력하고 본문에 붙여넣기(Cmd+V)하세요. 8) 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요. 9) 전송이 끝나면 done으로 종료하세요."

    scenario_status="failed"
    if run_agent_scenario "$CMD" "$LOG_FILE" 3; then
        scenario_status="success"
    fi
    if capture_and_notify 3 "주가 비교 체인" "$scenario_status" "$LOG_FILE" "$SCENARIO_GOAL" "$CMD"; then
        echo "✅ Scenario 3 Complete."
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    else
        echo "❌ Scenario 3 Failed."
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
    sleep 5
else
    echo "⏭️  Scenario 3 skipped (STEER_SCENARIO_IDS=${SELECTED_SCENARIO_IDS})"
fi

# Scenario 4: Calendar -> Notes -> TextEdit -> Mail
if should_run_scenario 4; then
    echo "---------------------------------------------------"
    echo "🧠 Scenario 4: Calendar → Notes → TextEdit → Mail"
    LOG_FILE="scenario_results/complex_scenario_4_${TIMESTAMP}.log"
    SCENARIO_GOAL="Idea note -> report -> mail send chain."
    CURRENT_SCENARIO_MARKER="$MARKER_S4"
    echo "Goal: ${SCENARIO_GOAL}"
    CMD="아래 순서를 정확히 지키세요. 1) Calendar를 열어 전면으로 가져오세요. 2) Notes를 열고 새 메모(Cmd+N)를 만든 뒤 제목을 \"${SUBJECT_S4}\"로 입력하세요. 3) 본문에 다음 4줄을 그대로 입력하세요: \"focus music\", \"pomodoro timer\", \"daily review template\", \"${MARKER_S4}\". 4) 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. 5) TextEdit를 열고 새 문서(Cmd+N)에 붙여넣기(Cmd+V)한 뒤 다음 줄에 \"Research shortlist ready\"를 입력하세요. 6) 다시 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. 7) Mail을 열고 새 이메일(Cmd+N) 초안을 만든 뒤 제목 \"${SUBJECT_S4}\"를 입력하고 본문에 붙여넣기(Cmd+V)하세요. 8) 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요. 9) 전송이 끝나면 done으로 종료하세요."

    scenario_status="failed"
    if run_agent_scenario "$CMD" "$LOG_FILE" 4; then
        scenario_status="success"
    fi
    if capture_and_notify 4 "아이디어 리서치 체인" "$scenario_status" "$LOG_FILE" "$SCENARIO_GOAL" "$CMD"; then
        echo "✅ Scenario 4 Complete."
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    else
        echo "❌ Scenario 4 Failed."
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
    sleep 5
else
    echo "⏭️  Scenario 4 skipped (STEER_SCENARIO_IDS=${SELECTED_SCENARIO_IDS})"
fi

# Scenario 5: Finder -> Calculator -> Notes -> TextEdit -> Mail
if should_run_scenario 5; then
    echo "---------------------------------------------------"
    echo "💱 Scenario 5: Finder → Calculator → Notes → TextEdit → Mail"
    LOG_FILE="scenario_results/complex_scenario_5_${TIMESTAMP}.log"
    SCENARIO_GOAL="Finder/Calculator/Notes/TextEdit/Mail budget draft chain."
    CURRENT_SCENARIO_MARKER="$MARKER_S5"
    echo "Goal: ${SCENARIO_GOAL}"
    CMD="아래 순서를 정확히 지키세요. 1) Finder를 열고 Desktop을 전면으로 가져오세요. 2) Calculator를 열고 \"120*1450=\" 를 입력해 계산 화면을 준비하세요. 3) Notes를 열고 새 메모(Cmd+N)를 만든 뒤 제목을 \"${SUBJECT_S5}\"로 입력하세요. 4) 본문에 다음 3줄을 그대로 입력하세요: \"Base: 120 USD\", \"120*1450=\", \"${MARKER_S5}\". 5) 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. 6) TextEdit를 열고 새 문서(Cmd+N)에 붙여넣기(Cmd+V)한 뒤 다음 줄에 \"Budget draft ready\"를 입력하세요. 7) 다시 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. 8) Mail을 열고 새 이메일(Cmd+N) 초안을 만든 뒤 제목 \"${SUBJECT_S5}\"를 입력하고 본문에 붙여넣기(Cmd+V)하세요. 9) 받는 사람에 \"${MAIL_TO_TARGET}\"를 입력하고 보내기(Cmd+Shift+D)로 발송하세요. 10) 전송이 끝나면 done으로 종료하세요."

    scenario_status="failed"
    if run_agent_scenario "$CMD" "$LOG_FILE" 5; then
        scenario_status="success"
    fi
    if capture_and_notify 5 "환율 예산 체인" "$scenario_status" "$LOG_FILE" "$SCENARIO_GOAL" "$CMD"; then
        echo "✅ Scenario 5 Complete."
        SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    else
        echo "❌ Scenario 5 Failed."
        FAIL_COUNT=$((FAIL_COUNT + 1))
    fi
else
    echo "⏭️  Scenario 5 skipped (STEER_SCENARIO_IDS=${SELECTED_SCENARIO_IDS})"
fi

echo ""
echo "📊 Summary: selected=${SELECTED_SCENARIO_COUNT}, success=${SUCCESS_COUNT}, failed=${FAIL_COUNT}"
if [ "$FAIL_COUNT" -gt 0 ]; then
    echo "⚠️  Completed with failures."
    exit 1
fi
echo "🎉 All selected complex scenarios succeeded."
