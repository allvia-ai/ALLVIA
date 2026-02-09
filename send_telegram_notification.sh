#!/bin/bash

# Telegram notification helper script
# Usage: ./send_telegram_notification.sh "message text" [image_path]
# Optional env:
#   TELEGRAM_DUMP_FINAL_PATH=/path/to/file.txt  (stores final text sent to Telegram)
#   TELEGRAM_EXTRA_IMAGE_LIST_FILE=/path/to/list.txt
#     line format: /absolute/or/relative/image.png|caption text

TELEGRAM_BOT_TOKEN="${TELEGRAM_BOT_TOKEN:-}"
TELEGRAM_CHAT_ID="${TELEGRAM_CHAT_ID:-}"
TELEGRAM_CONNECT_TIMEOUT="${TELEGRAM_CONNECT_TIMEOUT:-5}"
TELEGRAM_MAX_TIME="${TELEGRAM_MAX_TIME:-10}"

MESSAGE="$1"
IMAGE_PATH="$2"

if [ -z "$MESSAGE" ]; then
    echo "Usage: $0 'message' [image_path]"
    exit 1
fi

if [ -z "$TELEGRAM_BOT_TOKEN" ] || [ -z "$TELEGRAM_CHAT_ID" ]; then
    echo "TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID must be set"
    exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
    echo "jq is required to send Telegram notifications"
    exit 1
fi

# [Smart Notification Upgrade]
# Use Rust binary for rewriting (Prioritize DEBUG build for speed)
BINARY_PATH="./core/target/debug/local_os_agent" 
SKIP_REWRITE="${TELEGRAM_SKIP_REWRITE:-0}"

if [ "$SKIP_REWRITE" != "1" ] && [ -f "$BINARY_PATH" ]; then
    # echo "🤖 Refining message with Steer Intelligence (Rust)..."
    REFINED_MSG=$($BINARY_PATH rewrite "$MESSAGE" 2>/dev/null)
    
    if [ -n "$REFINED_MSG" ] && [ "$REFINED_MSG" != "$MESSAGE" ]; then
        MESSAGE="$REFINED_MSG"
    fi
fi

# Persist final outgoing text if requested (for audit/debug).
if [ -n "${TELEGRAM_DUMP_FINAL_PATH:-}" ]; then
    mkdir -p "$(dirname "$TELEGRAM_DUMP_FINAL_PATH")"
    printf '%s\n' "$MESSAGE" > "$TELEGRAM_DUMP_FINAL_PATH"
fi

# Send text message using JSON payload
send_message() {
    local text="${1:-$MESSAGE}"
    curl -fsS -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendMessage" \
        --connect-timeout "$TELEGRAM_CONNECT_TIMEOUT" --max-time "$TELEGRAM_MAX_TIME" \
        -H "Content-Type: application/json" \
        -d "$(jq -n --arg chat_id "$TELEGRAM_CHAT_ID" --arg text "$text" '{chat_id: $chat_id, text: $text}')" > /dev/null
}

# Send one photo with caption.
send_photo_with_caption() {
    local image_path="$1"
    local caption="$2"
    if [ ! -f "$image_path" ]; then
        echo "Image file not found: $image_path"
        return 1
    fi

    if [ ${#caption} -gt 900 ]; then
        caption="${caption:0:900}..."
    fi

    curl -fsS -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendPhoto" \
        --connect-timeout "$TELEGRAM_CONNECT_TIMEOUT" --max-time "$TELEGRAM_MAX_TIME" \
        -F "chat_id=${TELEGRAM_CHAT_ID}" \
        -F "photo=@${image_path}" \
        -F "caption=${caption}" > /dev/null
}

send_extra_images() {
    local list_file="${TELEGRAM_EXTRA_IMAGE_LIST_FILE:-}"
    if [ -z "$list_file" ] || [ ! -f "$list_file" ]; then
        return 0
    fi

    while IFS= read -r line || [ -n "$line" ]; do
        [ -z "$line" ] && continue
        local image_path="${line%%|*}"
        local caption="${line#*|}"
        if [ "$image_path" = "$line" ]; then
            caption="노드 결과 스크린샷"
        fi
        if ! send_photo_with_caption "$image_path" "$caption"; then
            echo "Failed to send extra node image: $image_path"
            return 1
        fi
    done < "$list_file"
    return 0
}

# Send main content first.
if [ -n "$IMAGE_PATH" ]; then
    if ! send_photo_with_caption "$IMAGE_PATH" "$MESSAGE"; then
        echo "❌ Telegram notification failed"
        exit 1
    fi

    # If message was truncated as caption, send full text separately.
    if [ ${#MESSAGE} -gt 900 ]; then
        if ! send_message "$MESSAGE"; then
            echo "❌ Telegram notification failed"
            exit 1
        fi
    fi
else
    if ! send_message "$MESSAGE"; then
        echo "❌ Telegram notification failed"
        exit 1
    fi
fi

# Send per-node summary images if provided.
if ! send_extra_images; then
    echo "❌ Telegram extra node images failed"
    exit 1
fi

echo "✅ Telegram notification sent"
