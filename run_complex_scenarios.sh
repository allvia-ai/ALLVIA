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

# Validate required runtime permissions/config before running long scenarios.
preflight_checks() {
    local failed=0
    local ax_out=""
    local capture_out=""
    local preflight_capture="scenario_results/preflight_capture_${TIMESTAMP}.png"

    echo "🔎 Running preflight checks..."

    if ! command -v osascript >/dev/null 2>&1; then
        echo "❌ Preflight failed: osascript not found."
        failed=1
    elif ! ax_out=$(osascript -e 'tell application "System Events" to return name of first application process' 2>&1); then
        echo "❌ Preflight failed: Accessibility permission check failed."
        echo "   Details: $ax_out"
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
    elif ! capture_out=$(screencapture -x "$preflight_capture" 2>&1); then
        echo "❌ Preflight failed: Screen Recording/display capture unavailable."
        echo "   Details: $capture_out"
        if echo "$capture_out" | grep -q "could not create image from display"; then
            echo "   Cause: 현재 실행 세션에서 디스플레이 접근이 불가능합니다."
        fi
        echo "   Fix: System Settings > Privacy & Security > Screen Recording에서 Terminal/Codex를 허용하세요."
        failed=1
    else
        echo "✅ Preflight: Screen capture works."
        rm -f "$preflight_capture"
    fi

    if [ -z "${OPENAI_API_KEY:-}" ]; then
        echo "❌ Preflight failed: OPENAI_API_KEY is not set."
        echo "   Fix: core/.env 또는 현재 셸 환경에 OPENAI_API_KEY를 설정하세요."
        failed=1
    else
        echo "✅ Preflight: OPENAI_API_KEY detected."
    fi

    if [ "$failed" -ne 0 ]; then
        echo ""
        echo "⛔ Preflight checks failed. Aborting scenario run."
        return 1
    fi

    echo "✅ Preflight checks passed."
    return 0
}

# Run agent command and detect logical failures from logs as well as exit code.
run_agent_scenario() {
    local prompt=$1
    local log_file=$2
    local fatal_pattern='Failed to acquire lock|thread .* panicked|FATAL ERROR|⛔️|❌|LLM not available for surf mode|Preflight failed'

    if ! STEER_SCENARIO_MODE=1 STEER_LOCK_DISABLED=1 cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$prompt" &> "$log_file"; then
        return 1
    fi

    if grep -Eq "$fatal_pattern" "$log_file"; then
        return 1
    fi

    return 0
}

# Helper function to capture screenshot and send notification
capture_and_notify() {
    local scenario_num=$1
    local scenario_name=$2
    local status=$3
    local log_file=$4
    
    # Capture screenshot
    local screenshot="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}.png"
    if ! screencapture -x "$screenshot"; then
        echo "Warning: failed to capture screenshot for scenario ${scenario_num}" >&2
    fi
    
    # Extract meaningful info from log
    local result_info=""
    if grep -q "✅" "$log_file" 2>/dev/null; then
        result_info="작업 완료"
    elif grep -q "❌" "$log_file" 2>/dev/null; then
        result_info="실행 중 오류 발생"
    else
        result_info="실행됨"
    fi
    
    echo "Scenario ${scenario_num} finished with status: ${status}"
}

if ! preflight_checks; then
    exit 1
fi
echo ""

# Scenario 1: Calendar -> Safari -> Notes -> Mail
echo "---------------------------------------------------"
echo "📅 Scenario 1: Calendar → Safari → Notes → Mail"
LOG_FILE="scenario_results/complex_scenario_1_${TIMESTAMP}.log"
echo "Goal: Multi-app draft chain without screen-reading dependency."
CMD='Calendar를 열고 전면으로 가져오세요. Notes를 열어 새 메모(Cmd+N)를 만들고 제목을 "Today Plan Brief"로 입력한 뒤 아래 3줄을 그대로 입력하세요: "Calendar opened", "Notes draft ready", "Mail prep pending". 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. TextEdit를 열어 새 문서(Cmd+N)에 붙여넣기(Cmd+V)하고 다음 줄에 "Shared via TextEdit"를 입력하세요. 다시 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. Mail을 열어 새 이메일(Cmd+N) 초안을 만들고 제목 "Today Plan Brief"를 입력한 뒤 본문에 붙여넣기(Cmd+V)하세요.'

if run_agent_scenario "$CMD" "$LOG_FILE"; then
    echo "✅ Scenario 1 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 1 "일정 브리핑 체인" "success" "$LOG_FILE"
else
    echo "❌ Scenario 1 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 1 "일정 브리핑 체인" "failed" "$LOG_FILE"
fi
sleep 5

# Scenario 2: Finder -> TextEdit -> Notes
echo "---------------------------------------------------"
echo "📂 Scenario 2: Finder → TextEdit → Notes"
LOG_FILE="scenario_results/complex_scenario_2_${TIMESTAMP}.log"
echo "Goal: Finder/TextEdit/Notes/Mail transfer chain."
CMD='Finder를 열어 Downloads 폴더로 이동하세요. TextEdit를 열어 새 문서(Cmd+N)를 만들고 제목 "Downloads Triage"를 입력한 뒤 아래 3줄을 그대로 입력하세요: "1. invoice.pdf", "2. screenshot.png", "3. notes.txt". 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. Notes를 열어 새 메모(Cmd+N)를 만들고 붙여넣기(Cmd+V)하세요. 다시 전체 선택(Cmd+A) 후 복사(Cmd+C)하고 Mail을 열어 새 이메일(Cmd+N) 초안을 만든 뒤 제목 "Downloads Triage"를 입력하고 본문에 붙여넣기(Cmd+V)하세요.'

if run_agent_scenario "$CMD" "$LOG_FILE"; then
    echo "✅ Scenario 2 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 2 "다운로드 분류 체인" "success" "$LOG_FILE"
else
    echo "❌ Scenario 2 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 2 "다운로드 분류 체인" "failed" "$LOG_FILE"
fi
sleep 5

# Scenario 3: Safari -> Calculator -> Notes
echo "---------------------------------------------------"
echo "📈 Scenario 3: Safari → Calculator → Notes"
LOG_FILE="scenario_results/complex_scenario_3_${TIMESTAMP}.log"
echo "Goal: Browser + calculation + document handoff chain."
CMD='Safari를 열고 https://www.google.com 으로 이동하세요. 새 탭(Cmd+T)을 열고 https://www.wikipedia.org 로 이동하세요. Calculator를 열어 "120*1300=" 을 입력해 계산한 뒤 복사(Cmd+C)하세요. Notes를 열어 새 메모(Cmd+N)를 만들고 제목 "Calc Result"를 입력한 뒤 다음 줄에 "120*1300="를 입력하고 다음 줄에 붙여넣기(Cmd+V)하세요. TextEdit를 열어 새 문서(Cmd+N)에 방금 메모 내용을 붙여넣기(Cmd+V)하고 마지막 줄에 "Done"을 입력하세요.'

if run_agent_scenario "$CMD" "$LOG_FILE"; then
    echo "✅ Scenario 3 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 3 "주가 비교 체인" "success" "$LOG_FILE"
else
    echo "❌ Scenario 3 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 3 "주가 비교 체인" "failed" "$LOG_FILE"
fi
sleep 5

# Scenario 4: Notes -> Safari -> TextEdit
echo "---------------------------------------------------"
echo "🧠 Scenario 4: Notes → Safari → TextEdit"
LOG_FILE="scenario_results/complex_scenario_4_${TIMESTAMP}.log"
echo "Goal: Idea note -> web query -> report -> mail draft chain."
CMD='Notes를 열어 새 메모(Cmd+N)를 만들고 아래 3줄을 그대로 입력하세요: "focus music", "pomodoro timer", "daily review template". 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. Safari를 열고 https://www.google.com 으로 이동한 뒤 붙여넣기(Cmd+V)하고 Enter를 누르세요. 주소창에 포커스(Cmd+L) 후 복사(Cmd+C)하세요. TextEdit를 열어 새 문서(Cmd+N)에 "Productivity Research" 제목을 입력하고 다음 줄에 붙여넣기(Cmd+V)하세요. Mail을 열어 새 이메일(Cmd+N) 초안을 만들고 제목 "Productivity Research"를 입력한 뒤 본문에 붙여넣기(Cmd+V)하세요.'

if run_agent_scenario "$CMD" "$LOG_FILE"; then
    echo "✅ Scenario 4 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 4 "아이디어 리서치 체인" "success" "$LOG_FILE"
else
    echo "❌ Scenario 4 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 4 "아이디어 리서치 체인" "failed" "$LOG_FILE"
fi
sleep 5

# Scenario 5: Safari -> Calculator -> Notes -> Mail
echo "---------------------------------------------------"
echo "💱 Scenario 5: Safari → Calculator → Notes → Mail"
LOG_FILE="scenario_results/complex_scenario_5_${TIMESTAMP}.log"
echo "Goal: Finder/Calculator/Notes/Mail budget draft chain."
CMD='Finder를 열어 Desktop으로 이동하세요. Calculator를 열어 "120*1450=" 을 입력해 계산하고 결과를 복사(Cmd+C)하세요. Notes를 열어 새 메모(Cmd+N)를 만들고 제목 "Budget Check"를 입력한 뒤 다음 줄에 "Base: 120 USD"를 입력하고 다음 줄에 붙여넣기(Cmd+V)하세요. 전체 선택(Cmd+A) 후 복사(Cmd+C)하세요. Mail을 열어 새 이메일(Cmd+N) 초안을 만들고 제목 "Budget Check"를 입력한 다음 본문에 붙여넣기(Cmd+V)하세요.'

if run_agent_scenario "$CMD" "$LOG_FILE"; then
    echo "✅ Scenario 5 Complete."
    SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
    capture_and_notify 5 "환율 예산 체인" "success" "$LOG_FILE"
else
    echo "❌ Scenario 5 Failed."
    FAIL_COUNT=$((FAIL_COUNT + 1))
    capture_and_notify 5 "환율 예산 체인" "failed" "$LOG_FILE"
fi

echo ""
echo "📊 Summary: success=${SUCCESS_COUNT}, failed=${FAIL_COUNT}"
if [ "$FAIL_COUNT" -gt 0 ]; then
    echo "⚠️  Completed with failures."
    exit 1
fi
echo "🎉 All 5 Complex Scenarios Succeeded."
