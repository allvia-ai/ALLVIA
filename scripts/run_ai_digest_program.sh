#!/bin/bash
set -euo pipefail

# Usage:
#   ./scripts/run_ai_digest_program.sh ["요청 문구"]
#
# This calls core API endpoint:
#   POST /api/automation/ai-digest

REQUEST_TEXT="${1:-뉴스 5개 요약해서 노션에 정리해줘. 유튜브 링크 포함.}"
API_BASE="${STEER_API_BASE_URL:-http://127.0.0.1:5680/api}"
URL="${API_BASE%/}/automation/ai-digest"

if command -v jq >/dev/null 2>&1; then
  PAYLOAD="$(jq -nc --arg text "$REQUEST_TEXT" '{text:$text}')"
  curl -sS -X POST "$URL" \
    -H "Content-Type: application/json" \
    -d "$PAYLOAD" | jq .
else
  ESCAPED="${REQUEST_TEXT//\"/\\\"}"
  curl -sS -X POST "$URL" \
    -H "Content-Type: application/json" \
    -d "{\"text\":\"$ESCAPED\"}"
fi
