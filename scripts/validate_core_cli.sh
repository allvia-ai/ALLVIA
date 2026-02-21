#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CORE_DIR="${ROOT_DIR}/core"

PORT=15680
PROFILE="debug"
GOAL=""
KEEP_RUNNING=0

usage() {
  cat <<'EOF'
Usage:
  ./scripts/validate_core_cli.sh [--goal "메모장 열어줘"] [--port 15680] [--release] [--keep-running]

Options:
  --goal TEXT        Run one goal request after server health checks.
  --port N           API port for validation core (default: 15680).
  --release          Build and run release binary instead of debug.
  --keep-running     Do not stop validation core at script end.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --goal)
      shift
      GOAL="${1:-}"
      [[ -n "${GOAL}" ]] || { echo "ERROR: --goal requires text"; exit 1; }
      ;;
    --port)
      shift
      PORT="${1:-}"
      [[ "${PORT}" =~ ^[0-9]+$ ]] || { echo "ERROR: --port must be numeric"; exit 1; }
      ;;
    --release)
      PROFILE="release"
      ;;
    --keep-running)
      KEEP_RUNNING=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown option: $1"
      usage
      exit 1
      ;;
  esac
  shift
done

BUILD_CMD=(cargo build --manifest-path "${CORE_DIR}/Cargo.toml" --bin local_os_agent)
if [[ "${PROFILE}" == "release" ]]; then
  BUILD_CMD+=(--release)
fi

echo "[1/5] Build core (${PROFILE})..."
"${BUILD_CMD[@]}"

BIN="${CORE_DIR}/target/${PROFILE}/local_os_agent"
if [[ ! -x "${BIN}" ]]; then
  echo "ERROR: core binary not found: ${BIN}"
  exit 1
fi

BASE_URL="http://127.0.0.1:${PORT}"
HEALTH_URL="${BASE_URL}/api/system/health"
PREFLIGHT_URL="${BASE_URL}/api/agent/preflight"
LOG_FILE="/tmp/steer_core_validate_${PORT}.log"
PID_FILE="/tmp/steer_core_validate_${PORT}.pid"

cleanup() {
  if [[ "${KEEP_RUNNING}" -eq 1 ]]; then
    echo "Validation core left running on ${BASE_URL} (keep-running enabled)."
    echo "PID: $(cat "${PID_FILE}" 2>/dev/null || echo '?')"
    return
  fi
  if [[ -f "${PID_FILE}" ]]; then
    local pid
    pid="$(cat "${PID_FILE}" || true)"
    if [[ -n "${pid}" ]] && kill -0 "${pid}" 2>/dev/null; then
      kill "${pid}" || true
    fi
  fi
}
trap cleanup EXIT

if lsof -tiTCP:"${PORT}" -sTCP:LISTEN >/dev/null 2>&1; then
  echo "ERROR: port ${PORT} already in use."
  lsof -iTCP:"${PORT}" -sTCP:LISTEN -n -P || true
  exit 1
fi

echo "[2/5] Launch validation core on ${BASE_URL}..."
(
  cd "${CORE_DIR}"
  STEER_API_PORT="${PORT}" \
  STEER_LOCK_SCOPE="validate-${PORT}" \
  STEER_API_ALLOW_NO_KEY=1 \
  STEER_DISABLE_EVENT_TAP=1 \
  STEER_PREFLIGHT_SCREEN_CAPTURE=0 \
  STEER_PREFLIGHT_AX_SNAPSHOT=0 \
  RUST_LOG=info \
  "${BIN}" >"${LOG_FILE}" 2>&1
) &
echo $! > "${PID_FILE}"

for _ in {1..25}; do
  if curl -fsS --max-time 2 "${HEALTH_URL}" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

if ! curl -fsS --max-time 3 "${HEALTH_URL}" >/dev/null 2>&1; then
  echo "ERROR: health check failed (${HEALTH_URL})"
  echo "---- core log (tail) ----"
  tail -n 120 "${LOG_FILE}" || true
  exit 1
fi

echo "[3/5] Health check OK"
curl -sS --max-time 3 "${HEALTH_URL}" || true
echo

echo "[4/5] Preflight probe"
curl -sS --max-time 20 "${PREFLIGHT_URL}" | sed -n '1,2p' || true
echo

if [[ -n "${GOAL}" ]]; then
  echo "[5/5] Goal run probe"
  goal_json="$(printf '%s' "${GOAL}" | sed 's/\\/\\\\/g; s/"/\\"/g')"
  resp="$(curl -sS --max-time 180 -X POST "${BASE_URL}/api/agent/goal/run" \
    -H 'Content-Type: application/json' \
    -d "{\"goal\":\"${goal_json}\"}")"
  echo "${resp}"

  run_id="$(echo "${resp}" | rg -o '"run_id"\s*:\s*"[^"]+"' | head -n1 | sed -E 's/.*"([^"]+)"$/\1/' || true)"
  if [[ -n "${run_id}" ]]; then
    echo "run_id=${run_id}"
    for _ in {1..20}; do
      sleep 2
      detail="$(curl -sS --max-time 5 "${BASE_URL}/api/agent/task-runs/${run_id}" || true)"
      status="$(echo "${detail}" | sed -n 's/.*"status":"\([^"]*\)".*/\1/p' | head -n1)"
      echo "status=${status:-unknown}"
      if [[ "${status}" == "business_completed" || "${status}" == "business_failed" ]]; then
        break
      fi
    done
  fi
fi

echo "Validation done."
