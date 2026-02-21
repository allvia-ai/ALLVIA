#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

CORE_CRATE_DIR="${ROOT_DIR}/core"
WEB_DIR="${ROOT_DIR}/web"
EXTERNAL_CORE_BIN="${WEB_DIR}/src-tauri/binaries/core-aarch64-apple-darwin"
BUNDLE_APP="${WEB_DIR}/src-tauri/target/release/bundle/macos/Steer OS.app"
APP_DST="/Applications/Steer OS.app"
HEALTH_URL="http://127.0.0.1:5680/api/system/health"

echo "[1/6] Build core server binary (local_os_agent)..."
cargo build --manifest-path "${CORE_CRATE_DIR}/Cargo.toml" --release

echo "[2/6] Sync externalBin core payload..."
cp "${CORE_CRATE_DIR}/target/release/local_os_agent" "${EXTERNAL_CORE_BIN}"
chmod +x "${EXTERNAL_CORE_BIN}"

echo "[3/6] Build Tauri bundle..."
(
  cd "${WEB_DIR}"
  npm run tauri build
)

if [[ ! -d "${BUNDLE_APP}" ]]; then
  echo "ERROR: bundle not found: ${BUNDLE_APP}"
  exit 1
fi

echo "[4/6] Stop running Steer OS processes..."
pkill -f "Steer OS.app/Contents/MacOS/app" || true
pkill -f "Steer OS.app/Contents/MacOS/core" || true
sleep 1

echo "[5/6] Replace /Applications app (clean deploy)..."
rm -rf "${APP_DST}"
ditto "${BUNDLE_APP}" "${APP_DST}"

echo "[6/6] Launch and health-check..."
open -a "${APP_DST}"

ok=0
for _ in {1..20}; do
  if curl -fsS --max-time 2 "${HEALTH_URL}" >/dev/null 2>&1; then
    ok=1
    break
  fi
  sleep 1
done

if [[ "${ok}" -ne 1 ]]; then
  echo "ERROR: health check failed: ${HEALTH_URL}"
  echo "Debug:"
  ps aux | rg -i "/Applications/Steer OS.app/Contents/MacOS/(app|core)" || true
  lsof -iTCP:5680 -sTCP:LISTEN -n -P || true
  exit 1
fi

echo "OK: deploy complete"
ps aux | rg -i "/Applications/Steer OS.app/Contents/MacOS/(app|core)" || true
curl -sS --max-time 3 "${HEALTH_URL}" || true
