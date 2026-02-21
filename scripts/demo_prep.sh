#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

echo "=== Steer Demo Prep ==="
echo "repo: $ROOT_DIR"
echo

echo "[1/6] Demo readiness check"
"$ROOT_DIR/scripts/demo_ready_check.sh"
echo

echo "[2/6] Demo state reset"
"$ROOT_DIR/scripts/demo_state_reset.sh"
echo

echo "[3/6] Web build"
(cd "$ROOT_DIR/web" && npm run -s build)
echo

echo "[4/6] State reset script syntax check"
bash -n "$ROOT_DIR/scripts/demo_state_reset.sh"
echo "✅ demo_state_reset.sh syntax ok"
echo

echo "[5/6] UI recorder script syntax check"
bash -n "$ROOT_DIR/scripts/record_ui_nl_demo.sh"
echo "✅ record_ui_nl_demo.sh syntax ok"
echo

echo "[6/7] Preset recorder script syntax check"
bash -n "$ROOT_DIR/scripts/record_demo_preset.sh"
echo "✅ record_demo_preset.sh syntax ok"
echo

echo "[7/7] One-click demo script syntax check"
bash -n "$ROOT_DIR/scripts/demo_run.sh"
echo "✅ demo_run.sh syntax ok"
echo

echo "✅ Demo prep completed."
echo "next:"
echo "  cd \"$ROOT_DIR\""
echo "  ./scripts/demo_run.sh --preset news_telegram"
