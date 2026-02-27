#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="${STEER_COLLECTOR_PORT:-9100}"
REQUIRE_TOKEN="${STEER_COLLECTOR_REQUIRE_TOKEN:-0}"
LOG_DIR="${ROOT_DIR}/.logs"
LOG_FILE="${LOG_DIR}/collector_rs.log"

mkdir -p "${LOG_DIR}"

if [[ ! -x "${ROOT_DIR}/core/target/release/collector_rs" ]]; then
  cargo build --release --manifest-path "${ROOT_DIR}/core/Cargo.toml" --bin collector_rs >/dev/null
fi

if pgrep -f 'collector_rs' >/dev/null 2>&1; then
  pkill -f 'collector_rs' || true
  sleep 1
fi

nohup env \
  STEER_COLLECTOR_PORT="${PORT}" \
  STEER_COLLECTOR_REQUIRE_TOKEN="${REQUIRE_TOKEN}" \
  "${ROOT_DIR}/core/target/release/collector_rs" \
  >>"${LOG_FILE}" 2>&1 < /dev/null &

sleep 1
curl -fsS "http://127.0.0.1:${PORT}/health" >/dev/null
echo "collector_rs started on http://127.0.0.1:${PORT} (log: ${LOG_FILE})"
