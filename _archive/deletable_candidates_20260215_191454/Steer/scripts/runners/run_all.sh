#!/usr/bin/env bash
# 하나의 명령으로 FastAPI 백엔드 + React 프런트를 함께 실행합니다.
# 종료(Ctrl+C) 시 백엔드를 함께 종료합니다.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
API_PID=""

cleanup() {
  if [[ -n "${API_PID}" ]]; then
    if kill -0 "${API_PID}" >/dev/null 2>&1; then
      kill "${API_PID}" >/dev/null 2>&1 || true
    fi
  fi
}
trap cleanup EXIT INT TERM

cd "${ROOT_DIR}"

echo "Starting FastAPI server (scripts/search_api_server.py)…"
python scripts/search_api_server.py &
API_PID=$!
echo "FastAPI PID=${API_PID}"

echo "Starting webapp dev server (npm run dev)…"
cd "${ROOT_DIR}/webapp"
npm run dev -- --host 127.0.0.1 --port 5173
