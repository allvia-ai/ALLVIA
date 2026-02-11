#!/bin/bash
set -euo pipefail

# Load environment variables
if [ -f core/.env ]; then
    set -a
    # shellcheck disable=SC1091
    source core/.env
    set +a
fi

echo "🚀 Starting Advanced Scenarios 1-5 Execution..."
echo "⚠️  PLEASE DO NOT TOUCH THE MOUSE/KEYBOARD DURING EXECUTION"
echo ""

require_terminal_context() {
    local require_terminal="${STEER_REQUIRE_TERMINAL:-1}"
    [ "$require_terminal" = "1" ] || return 0

    local term_program="${TERM_PROGRAM:-unknown}"
    local allowed_programs="${STEER_ALLOWED_TERM_PROGRAMS:-Apple_Terminal}"
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

preflight_checks() {
    local failed=0
    local preflight_capture="/tmp/all_scenarios_preflight_$$.png"
    local ax_out=""
    local cap_out=""

    if ! require_terminal_context; then
        return 1
    fi

    if ! ax_out=$(osascript -e 'tell application "System Events" to return name of first application process' 2>&1); then
        echo "❌ Preflight failed: Accessibility permission check failed."
        echo "   Details: $ax_out"
        failed=1
    fi

    if ! cap_out=$(screencapture -x "$preflight_capture" 2>&1); then
        echo "❌ Preflight failed: Screen Recording/display capture unavailable."
        echo "   Details: $cap_out"
        failed=1
    else
        rm -f "$preflight_capture"
    fi

    if [ "$failed" -ne 0 ]; then
        echo "⛔ Preflight checks failed. Aborting scenario run."
        return 1
    fi
    return 0
}

if ! preflight_checks; then
    exit 1
fi

# Send initial notification
send_telegram_safe() {
    if [ ! -x ./send_telegram_notification.sh ] && [ ! -f ./send_telegram_notification.sh ]; then
        echo "ℹ️ Telegram notifier not found, skipping send."
        return 0
    fi
    if [ -z "${TELEGRAM_BOT_TOKEN:-}" ] || [ -z "${TELEGRAM_CHAT_ID:-}" ]; then
        echo "ℹ️ Telegram env missing, skipping send."
        return 0
    fi
    if ! bash ./send_telegram_notification.sh "$@"; then
        echo "⚠️ Telegram send failed (continuing)."
    fi
}

send_telegram_safe "🚀 시나리오 실행 시작"

# Create output directory for results
mkdir -p scenario_results
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Helper function to capture screenshot and send notification
capture_and_notify() {
    local scenario_num=$1
    local scenario_name=$2
    local status=$3
    local log_file=$4
    
    # Capture screenshot
    local screenshot="scenario_results/scenario_${scenario_num}_${TIMESTAMP}.png"
    screencapture -x "$screenshot"
    
    local result_info=""
    local key_line=""
    key_line="$(grep -En "Goal completed by planner|Surf failed|Supervisor escalated|Execution Error|PLAN_REJECTED|LLM Refused|fallback action|FALLBACK_ACTION:" "$log_file" 2>/dev/null | tail -n 1 | sed -E 's/^[0-9]+://')"

    # Send clean notification
    if [ "$status" = "success" ]; then
        result_info="작업 완료"
        if [ -n "$key_line" ]; then
            result_info="${result_info} (${key_line})"
        fi
        local message="✅ *시나리오 ${scenario_num}: ${scenario_name}*

상태: 성공
결과: ${result_info}
시간: $(date '+%H:%M:%S')"
    else
        result_info="실행 중 오류 발생"
        if [ -n "$key_line" ]; then
            result_info="${result_info} (${key_line})"
        fi
        local message="❌ *시나리오 ${scenario_num}: ${scenario_name}*

상태: 실패
결과: ${result_info}
시간: $(date '+%H:%M:%S')

로그 확인: scenario_results/scenario_${scenario_num}_${TIMESTAMP}.log"
    fi
    
    send_telegram_safe "$message" "$screenshot"
}

SUCCESS_COUNT=0
TOTAL_COUNT=0

# Scenario 1: Calendar
echo "---------------------------------------------------"
echo "📅 Scenario 1: Calendar Check"
LOG_FILE="scenario_results/scenario_1_${TIMESTAMP}.log"
echo "Goal: 'Check my calendar for today and tell me the first event.'"
if cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "Check my calendar for today and tell me the first event." &> "$LOG_FILE"; then
    echo "✅ Scenario 1 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 1 "캘린더 확인" "success" "$LOG_FILE"
else
    echo "❌ Scenario 1 Failed."
    capture_and_notify 1 "캘린더 확인" "failed" "$LOG_FILE"
fi
TOTAL_COUNT=$((TOTAL_COUNT + 1))
sleep 5

# Scenario 2: Meeting Summary
echo "---------------------------------------------------"
echo "📝 Scenario 2: Meeting Summary"
LOG_FILE="scenario_results/scenario_2_${TIMESTAMP}.log"
echo "Goal: 'Open Notes app, create a new note titled 'Daily Standup', and write 'All systems go'.'"
if cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "Open Notes app, create a new note titled 'Daily Standup', and write 'All systems go'." &> "$LOG_FILE"; then
    echo "✅ Scenario 2 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 2 "회의 요약 작성" "success" "$LOG_FILE"
else
    echo "❌ Scenario 2 Failed."
    capture_and_notify 2 "회의 요약 작성" "failed" "$LOG_FILE"
fi
TOTAL_COUNT=$((TOTAL_COUNT + 1))
sleep 5

# Scenario 3: Research
echo "---------------------------------------------------"
echo "🌐 Scenario 3: Web Research"
LOG_FILE="scenario_results/scenario_3_${TIMESTAMP}.log"
echo "Goal: 'Open Safari, search for 'DeepSeek R1', and tell me what it is.'"
if cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "Open Safari, search for 'DeepSeek R1', and tell me what it is." &> "$LOG_FILE"; then
    echo "✅ Scenario 3 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 3 "웹 검색" "success" "$LOG_FILE"
else
    echo "❌ Scenario 3 Failed."
    capture_and_notify 3 "웹 검색" "failed" "$LOG_FILE"
fi
TOTAL_COUNT=$((TOTAL_COUNT + 1))
sleep 5

# Scenario 4: Finder Navigation
echo "---------------------------------------------------"
echo "📂 Scenario 4: Finder Navigation"
LOG_FILE="scenario_results/scenario_4_${TIMESTAMP}.log"
echo "Goal: 'Open Finder, list the files in the Downloads folder, and read the first filename.'"
if cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "Open Finder, list the files in the Downloads folder, and read the first filename." &> "$LOG_FILE"; then
    echo "✅ Scenario 4 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 4 "Finder 탐색" "success" "$LOG_FILE"
else
    echo "❌ Scenario 4 Failed."
    capture_and_notify 4 "Finder 탐색" "failed" "$LOG_FILE"
fi
TOTAL_COUNT=$((TOTAL_COUNT + 1))
sleep 5

# Scenario 5: Complex Workflow
echo "---------------------------------------------------"
echo "🔗 Scenario 5: Complex Workflow"
LOG_FILE="scenario_results/scenario_5_${TIMESTAMP}.log"
echo "Goal: 'Open Safari, check the current stock price of Apple (AAPL), then open Notes and record it.'"
if cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "Open Safari, check the current stock price of Apple (AAPL), then open Notes and record it." &> "$LOG_FILE"; then
    echo "✅ Scenario 5 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 5 "복합 워크플로우" "success" "$LOG_FILE"
else
    echo "❌ Scenario 5 Failed."
    capture_and_notify 5 "복합 워크플로우" "failed" "$LOG_FILE"
fi
TOTAL_COUNT=$((TOTAL_COUNT + 1))

echo ""
echo "🎉 All 5 Scenarios Executed."

# Send final summary with judged counts
SUMMARY="🎉 *전체 시나리오 실행 완료*

총 ${TOTAL_COUNT}개 실행
성공 ${SUCCESS_COUNT}개 / 실패 $((TOTAL_COUNT - SUCCESS_COUNT))개
결과: scenario_results/ 폴더 확인"

send_telegram_safe "$SUMMARY"
