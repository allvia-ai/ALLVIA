#!/bin/bash
set -e

# Load env
export $(cat core/.env | xargs)

mkdir -p scenario_results
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOG_FILE="scenario_results/scenario_4_${TIMESTAMP}.log"

echo "📂 Scenario 4: Finder Navigation"
echo "Logs: $LOG_FILE"

# Run with debug build (faster compile, less resource usage)
cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "Open Finder, list the files in the Downloads folder, and read the first filename." &> "$LOG_FILE"

if [ $? -eq 0 ]; then
    echo "✅ Scenario 4 Complete."
    MESSAGE="✅ *시나리오 4: Finder 탐색*

상태: 성공
결과: 작업 완료
시간: $(date '+%H:%M:%S')"
    
    echo "🔔 Sending Smart Notification..."
    ./send_telegram_notification.sh "$MESSAGE"
else
    echo "❌ Scenario 4 Failed."
    ./send_telegram_notification.sh "❌ *시나리오 4: Finder 탐색* (실패)"
fi
