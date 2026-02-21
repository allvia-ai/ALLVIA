#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NEW_SCRIPT="${SCRIPT_DIR}/rebuild_and_deploy.sh"

echo "[DEPRECATED] scripts/build_release.sh"
echo "Use: ./scripts/rebuild_and_deploy.sh"
echo

if [[ ! -x "${NEW_SCRIPT}" ]]; then
  echo "ERROR: ${NEW_SCRIPT} not found or not executable"
  exit 1
fi

exec "${NEW_SCRIPT}" "$@"
