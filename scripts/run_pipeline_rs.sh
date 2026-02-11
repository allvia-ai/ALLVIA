#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

CONFIG_PATH="${1:-configs/config.yaml}"

if [[ ! -x ./core/target/debug/build_sessions_rs || ! -x ./core/target/debug/build_routines_rs || ! -x ./core/target/debug/build_handoff_rs ]]; then
  cargo build --manifest-path core/Cargo.toml \
    --bin build_sessions_rs --bin build_routines_rs --bin build_handoff_rs
fi

./core/target/debug/build_sessions_rs --config "$CONFIG_PATH" --since-hours 6 --use-state
./core/target/debug/build_routines_rs --config "$CONFIG_PATH" --days 3 --min-support 2 --use-state
./core/target/debug/build_handoff_rs --config "$CONFIG_PATH" --skip-unchanged --keep-latest-pending
