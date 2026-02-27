#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BINARY="${ROOT_DIR}/core/target/release/collector_rs"
LABEL="com.allvia.collector_rs"
PLIST_DIR="${HOME}/Library/LaunchAgents"
PLIST_PATH="${PLIST_DIR}/${LABEL}.plist"
LOG_DIR="${ROOT_DIR}/.logs"

mkdir -p "${PLIST_DIR}" "${LOG_DIR}"

if [[ ! -x "${BINARY}" ]]; then
  cargo build --release --manifest-path "${ROOT_DIR}/core/Cargo.toml" --bin collector_rs >/dev/null
fi

cat > "${PLIST_PATH}" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LABEL}</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ProgramArguments</key>
  <array>
    <string>${BINARY}</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>STEER_COLLECTOR_PORT</key>
    <string>9100</string>
    <key>STEER_COLLECTOR_REQUIRE_TOKEN</key>
    <string>0</string>
  </dict>
  <key>StandardOutPath</key>
  <string>${LOG_DIR}/collector_launchd.out.log</string>
  <key>StandardErrorPath</key>
  <string>${LOG_DIR}/collector_launchd.err.log</string>
</dict>
</plist>
PLIST

launchctl bootout "gui/$(id -u)/${LABEL}" >/dev/null 2>&1 || true
launchctl bootstrap "gui/$(id -u)" "${PLIST_PATH}"
launchctl kickstart -k "gui/$(id -u)/${LABEL}"

sleep 1
curl -fsS "http://127.0.0.1:9100/health" >/dev/null
echo "installed ${LABEL} (${PLIST_PATH})"
