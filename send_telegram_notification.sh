#!/bin/bash

# Telegram notification helper script
# Usage: ./send_telegram_notification.sh "message text" [image_path]
# Optional env:
#   TELEGRAM_DUMP_FINAL_PATH=/path/to/file.txt  (stores final text sent to Telegram)
#   TELEGRAM_EXTRA_IMAGE_LIST_FILE=/path/to/list.txt
#     line format: /absolute/or/relative/image.png|caption text
#   NOTIFY_DENY_CHANNELS=telegram,...
#   NOTIFY_ALLOW_CHANNELS=telegram,...
#   NOTIFY_DENY_TARGET_IDS=<chat_id>,...
#   NOTIFY_ALLOW_TARGET_IDS=<chat_id>,...

TELEGRAM_BOT_TOKEN="${TELEGRAM_BOT_TOKEN:-}"
TELEGRAM_CHAT_ID="${TELEGRAM_CHAT_ID:-}"
TELEGRAM_CONNECT_TIMEOUT="${TELEGRAM_CONNECT_TIMEOUT:-5}"
TELEGRAM_MAX_TIME="${TELEGRAM_MAX_TIME:-20}"
TELEGRAM_RETRY_COUNT="${TELEGRAM_RETRY_COUNT:-3}"
TELEGRAM_RETRY_DELAY_SEC="${TELEGRAM_RETRY_DELAY_SEC:-1}"
TELEGRAM_VALIDATE_REPORT="${TELEGRAM_VALIDATE_REPORT:-0}"
TELEGRAM_MAX_TEXT_LEN="${TELEGRAM_MAX_TEXT_LEN:-3800}"

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

validate_report_message() {
    local text="$1"
    local evidence_count
    local has_status
    local has_evidence_header

    has_status=0
    has_evidence_header=0
    if printf '%s\n' "$text" | grep -Eq "^상태:[[:space:]]*(✅|❌)"; then
        has_status=1
    fi
    if printf '%s\n' "$text" | grep -Eq "^근거:"; then
        has_evidence_header=1
    fi
    evidence_count="$(printf '%s\n' "$text" | grep -Ec "^- ")"
    evidence_count="${evidence_count:-0}"

    if [ "$has_status" -eq 0 ] || [ "$has_evidence_header" -eq 0 ] || [ "$evidence_count" -lt 3 ]; then
        echo "❌ Telegram report validation failed (status=$has_status evidence_header=$has_evidence_header bullets=$evidence_count)"
        return 1
    fi
    return 0
}

if [ "$TELEGRAM_VALIDATE_REPORT" = "1" ]; then
    if ! validate_report_message "$MESSAGE"; then
        exit 1
    fi
fi

trim_ws() {
    printf '%s' "$1" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//'
}

message_contains_any_keyword() {
    local text="$1"
    local raw_list="$2"
    local item=""
    IFS=',' read -r -a __keywords <<< "$raw_list"
    for item in "${__keywords[@]}"; do
        item="$(trim_ws "$item")"
        [ -z "$item" ] && continue
        if printf '%s' "$text" | grep -Fqi -- "$item"; then
            return 0
        fi
    done
    return 1
}

csv_contains_exact() {
    local value="$1"
    local raw_list="$2"
    value="$(printf '%s' "$value" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"
    [ -z "$value" ] && return 1
    local item=""
    IFS=',' read -r -a __items <<< "$raw_list"
    for item in "${__items[@]}"; do
        item="$(printf '%s' "$item" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"
        [ -z "$item" ] && continue
        if [ "$item" = "$value" ]; then
            return 0
        fi
    done
    return 1
}

enforce_notify_policy() {
    local title="${TELEGRAM_POLICY_TITLE:-telegram}"
    local channel="${TELEGRAM_POLICY_CHANNEL:-telegram}"
    local target_id="$TELEGRAM_CHAT_ID"
    local policy="${NOTIFY_POLICY:-allow}"
    policy="$(printf '%s' "$policy" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"
    local haystack="${title}"$'\n'"${MESSAGE}"

    if [ "$policy" = "deny" ]; then
        echo "🔕 Telegram suppressed by NOTIFY_POLICY=deny"
        return 1
    fi

    local deny_channels="${NOTIFY_DENY_CHANNELS:-}"
    if [ -n "$deny_channels" ] && csv_contains_exact "$channel" "$deny_channels"; then
        echo "🔕 Telegram suppressed by NOTIFY_DENY_CHANNELS"
        return 1
    fi

    local allow_channels="${NOTIFY_ALLOW_CHANNELS:-}"
    if [ -n "$allow_channels" ] && ! csv_contains_exact "$channel" "$allow_channels"; then
        echo "🔕 Telegram suppressed by NOTIFY_ALLOW_CHANNELS"
        return 1
    fi

    local deny_targets="${NOTIFY_DENY_TARGET_IDS:-${NOTIFY_DENY_CHAT_IDS:-}}"
    if [ -n "$deny_targets" ] && csv_contains_exact "$target_id" "$deny_targets"; then
        echo "🔕 Telegram suppressed by NOTIFY_DENY_TARGET_IDS"
        return 1
    fi

    local allow_targets="${NOTIFY_ALLOW_TARGET_IDS:-${NOTIFY_ALLOW_CHAT_IDS:-}}"
    if [ -n "$allow_targets" ] && ! csv_contains_exact "$target_id" "$allow_targets"; then
        echo "🔕 Telegram suppressed by NOTIFY_ALLOW_TARGET_IDS"
        return 1
    fi

    local deny_keywords="${NOTIFY_DENY_KEYWORDS:-}"
    if [ -n "$deny_keywords" ] && message_contains_any_keyword "$haystack" "$deny_keywords"; then
        echo "🔕 Telegram suppressed by NOTIFY_DENY_KEYWORDS"
        return 1
    fi

    local allow_keywords="${NOTIFY_ALLOW_KEYWORDS:-}"
    if [ -n "$allow_keywords" ] && ! message_contains_any_keyword "$haystack" "$allow_keywords"; then
        echo "🔕 Telegram suppressed by NOTIFY_ALLOW_KEYWORDS (no keyword match)"
        return 1
    fi
    return 0
}

if ! enforce_notify_policy; then
    if [ "${TELEGRAM_REQUIRE_SEND:-0}" = "1" ]; then
        echo "❌ Telegram suppressed by policy while TELEGRAM_REQUIRE_SEND=1"
        exit 1
    fi
    exit 0
fi

# Send one text chunk with retries.
send_message_single() {
    local text="${1:-$MESSAGE}"
    local attempt=1
    local backoff="$TELEGRAM_RETRY_DELAY_SEC"
    local rc=1
    while [ "$attempt" -le "$TELEGRAM_RETRY_COUNT" ]; do
        local resp_file
        resp_file="$(mktemp -t steer_tg_msg.XXXXXX)"
        local http_code=""
        http_code="$(curl -sS -o "$resp_file" -w "%{http_code}" -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendMessage" \
            --connect-timeout "$TELEGRAM_CONNECT_TIMEOUT" --max-time "$TELEGRAM_MAX_TIME" \
            -H "Content-Type: application/json" \
            -d "$(jq -n --arg chat_id "$TELEGRAM_CHAT_ID" --arg text "$text" '{chat_id: $chat_id, text: $text}')" || true)"
        if [ "$http_code" = "200" ] && jq -e '.ok == true' "$resp_file" >/dev/null 2>&1; then
            rm -f "$resp_file"
            return 0
        fi
        rc=1
        if [ "$http_code" = "429" ]; then
            local retry_after
            retry_after="$(jq -r '.parameters.retry_after // empty' "$resp_file" 2>/dev/null || true)"
            if [[ "$retry_after" =~ ^[0-9]+$ ]] && [ "$retry_after" -gt 0 ]; then
                backoff="$retry_after"
            fi
        fi
        rm -f "$resp_file"
        if [ "$attempt" -lt "$TELEGRAM_RETRY_COUNT" ]; then
            sleep "$backoff"
            backoff=$((backoff * 2))
        fi
        attempt=$((attempt + 1))
    done
    return "$rc"
}

# Split long messages into multiple chunks (Telegram hard limit ~= 4096 chars).
send_message() {
    local text="${1:-$MESSAGE}"
    local max_len="$TELEGRAM_MAX_TEXT_LEN"
    if ! [[ "$max_len" =~ ^[0-9]+$ ]]; then
        max_len=3800
    fi

    if [ ${#text} -le "$max_len" ]; then
        send_message_single "$text"
        return $?
    fi

    local chunk=""
    local line=""
    while IFS= read -r line || [ -n "$line" ]; do
        while [ ${#line} -gt "$max_len" ]; do
            local part="${line:0:max_len}"
            if [ -n "$chunk" ]; then
                if ! send_message_single "$chunk"; then
                    return 1
                fi
                chunk=""
            fi
            if ! send_message_single "$part"; then
                return 1
            fi
            line="${line:max_len}"
        done

        local candidate
        if [ -z "$chunk" ]; then
            candidate="$line"
        else
            candidate="${chunk}"$'\n'"$line"
        fi

        if [ ${#candidate} -gt "$max_len" ] && [ -n "$chunk" ]; then
            if ! send_message_single "$chunk"; then
                return 1
            fi
            chunk="$line"
        else
            chunk="$candidate"
        fi
    done <<< "$text"

    if [ -n "$chunk" ]; then
        if ! send_message_single "$chunk"; then
            return 1
        fi
    fi
    return 0
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

    local attempt=1
    local backoff="$TELEGRAM_RETRY_DELAY_SEC"
    local rc=1
    while [ "$attempt" -le "$TELEGRAM_RETRY_COUNT" ]; do
        local resp_file
        resp_file="$(mktemp -t steer_tg_photo.XXXXXX)"
        local http_code=""
        http_code="$(curl -sS -o "$resp_file" -w "%{http_code}" -X POST "https://api.telegram.org/bot${TELEGRAM_BOT_TOKEN}/sendPhoto" \
            --connect-timeout "$TELEGRAM_CONNECT_TIMEOUT" --max-time "$TELEGRAM_MAX_TIME" \
            -F "chat_id=${TELEGRAM_CHAT_ID}" \
            -F "photo=@${image_path}" \
            -F "caption=${caption}" || true)"
        if [ "$http_code" = "200" ] && jq -e '.ok == true' "$resp_file" >/dev/null 2>&1; then
            rm -f "$resp_file"
            return 0
        fi
        rc=1
        if [ "$http_code" = "429" ]; then
            local retry_after
            retry_after="$(jq -r '.parameters.retry_after // empty' "$resp_file" 2>/dev/null || true)"
            if [[ "$retry_after" =~ ^[0-9]+$ ]] && [ "$retry_after" -gt 0 ]; then
                backoff="$retry_after"
            fi
        fi
        rm -f "$resp_file"
        if [ "$attempt" -lt "$TELEGRAM_RETRY_COUNT" ]; then
            sleep "$backoff"
            backoff=$((backoff * 2))
        fi
        attempt=$((attempt + 1))
    done
    return "$rc"
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
