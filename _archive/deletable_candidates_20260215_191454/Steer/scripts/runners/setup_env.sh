#!/usr/bin/env bash
set -euo pipefail

python_bin="${PYTHON_BIN:-python3}"
root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

cd "$root_dir"

if [ ! -f .env ] && [ -f .env.example ]; then
  cp .env.example .env
  echo "Created .env from .env.example. Update values as needed." >&2
fi

if [ ! -d .venv ]; then
  "$python_bin" -m venv .venv
fi

# shellcheck disable=SC1091
source .venv/bin/activate

pip install --upgrade pip
pip install -r requirements.txt

if command -v pytest >/dev/null 2>&1; then
  pytest -q || true
else
  echo "pytest not installed; skipping test run" >&2
fi
