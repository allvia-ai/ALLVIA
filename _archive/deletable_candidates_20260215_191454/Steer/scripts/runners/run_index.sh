#!/bin/bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null && pwd)
ROOT_DIR=$(cd -- "$SCRIPT_DIR/.." >/dev/null && pwd)
cd -- "$ROOT_DIR"

# Freeze parallelism and disable GPU inference to avoid FAISS/OpenMP conflicts.
export PYTORCH_MPS_ENABLE=0
export OMP_NUM_THREADS=1
export KMP_DUPLICATE_LIB_OK=TRUE

# Prefer offline transformers/HF hub usage for repeatability.
export HF_HUB_OFFLINE=1
export TRANSFORMERS_OFFLINE=1

python infopilot.py run index \
  --corpus data/corpus.parquet \
  --cache data/cache \
  --model data/topic_model.joblib \
  --policy none

if [[ -f data/cache/doc_index.faiss ]]; then
  echo "Index build succeeded: data/cache/doc_index.faiss exists."
else
  echo "Index build finished but data/cache/doc_index.faiss is missing." >&2
  exit 1
fi
