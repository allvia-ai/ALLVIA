#!/bin/bash
set -euo pipefail

# Legacy alias kept for compatibility.
# The canonical runner is run_complex_scenarios.sh.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_SCRIPT="${SCRIPT_DIR}/run_complex_scenarios.sh"

if [ ! -f "$TARGET_SCRIPT" ]; then
    echo "❌ Missing target script: $TARGET_SCRIPT" >&2
    exit 1
fi

echo "ℹ️ run_all_scenarios.sh is deprecated. Delegating to run_complex_scenarios.sh"
exec bash "$TARGET_SCRIPT" "$@"
