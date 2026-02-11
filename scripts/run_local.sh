#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

COLLECTOR_IMPL="${STEER_COLLECTOR_IMPL:-rust}"

if [[ "$COLLECTOR_IMPL" == "python" ]]; then
  export PYTHONPATH="${PYTHONPATH:-}:src"
  exec python -m collector.main --config configs/config.yaml
fi

if [[ ! -x ./core/target/debug/collector_rs ]]; then
  cargo build --manifest-path core/Cargo.toml --bin collector_rs
fi

exec ./core/target/debug/collector_rs
