#!/bin/bash
set -e

# Load env
export $(cat core/.env | xargs)
export STEER_API_PORT=5690 # Force alternate port to avoid conflict

mkdir -p scenario_results
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOG_FILE="scenario_results/scenario_3_${TIMESTAMP}.log"

echo "🌐 Scenario 3: Web Research"
echo "Logs: $LOG_FILE"

# Run with debug build
cargo run --manifest-path core/Cargo.toml --bin local_os_agent -- surf "Open Safari, search for 'DeepSeek R1', and tell me what it is." &> "$LOG_FILE"

if [ $? -eq 0 ]; then
    echo "✅ Scenario 3 Complete."
    
    # Extract last 40 lines of log to capture the result
    # Filter out verbose lines if needed, but raw log is fine for LLM
    LOG_SUMMARY=$(tail -n 40 "$LOG_FILE")
    
    MESSAGE="✅ Scenario 3 Execution Log:
$LOG_SUMMARY"
    
    echo "🔔 Sending Smart Notification..."
    ./send_telegram_notification.sh "$MESSAGE"
else
    echo "❌ Scenario 3 Failed."
    LOG_SUMMARY=$(tail -n 20 "$LOG_FILE")
    ./send_telegram_notification.sh "❌ Scenario 3 Failed. Log: $LOG_SUMMARY"
fi
