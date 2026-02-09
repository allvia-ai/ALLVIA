#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

HOST="${STEER_BG_PANEL_HOST:-127.0.0.1}"
PORT="${STEER_BG_PANEL_PORT:-8787}"

echo "Starting Steer BG panel at http://${HOST}:${PORT}"
exec python3 scripts/run_nl_bg_panel.py
