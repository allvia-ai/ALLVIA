#!/bin/bash
set -e

# Load environment variables
if [ -f core/.env ]; then
    export $(cat core/.env | xargs)
fi

echo "🚀 Starting Complex Scenario 5 Execution..."
echo "⚠️  PLEASE DO NOT TOUCH THE MOUSE/KEYBOARD DURING EXECUTION"
echo ""

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
    local screenshot="scenario_results/complex_scenario_${scenario_num}_${TIMESTAMP}.png"
    screencapture -x "$screenshot"
    
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

# Scenario 5: Notes -> Safari -> Notes -> Mail
echo "---------------------------------------------------"
echo "🔄 Scenario 5: Notes → Safari → Notes → Mail"
LOG_FILE="scenario_results/complex_scenario_5_${TIMESTAMP}.log"
echo "Goal: Research loop with Notes, Safari, and Mail."
CMD='Notes를 열어 새 메모(Cmd+N)를 만들고 "Research Topic: Rust programming language"를 입력하세요. "Rust programming" 텍스트를 선택해 복사(Cmd+C)하고 Safari를 열어 Google 검색창에 붙여넣기(Cmd+V) 후 검색하세요. 첫 번째 결과의 URL을 복사(Cmd+L, Cmd+C)하고 Notes로 돌아가 메모에 붙여넣으세요(Cmd+V). 메모 전체 선택(Cmd+A) 후 복사(Cmd+C)하고 Mail에서 새 이메일(Cmd+N)을 만들어 제목 "Research Findings"을 입력한 뒤 본문에 붙여넣으세요(Cmd+V).'

if cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "$CMD" &> "$LOG_FILE"; then
    echo "✅ Scenario 5 Complete."
    capture_and_notify 5 "Research Loop" "success" "$LOG_FILE"
else
    echo "❌ Scenario 5 Failed."
    capture_and_notify 5 "Research Loop" "failed" "$LOG_FILE"
fi

echo ""
echo "🎉 Scenario 5 Execution Finished."
