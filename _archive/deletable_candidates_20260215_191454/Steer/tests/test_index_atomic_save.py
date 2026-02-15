from __future__ import annotations

import json
from pathlib import Path

import numpy as np

from core.search.retriever import VectorIndex


def _build_small_index(tmp_path: Path, *, preview: str) -> VectorIndex:
    index = VectorIndex()
    Z = np.asarray([[1.0, 0.0], [0.0, 1.0]], dtype=np.float32)
    index.build(
        Z,
        paths=[str(tmp_path / "a.md"), str(tmp_path / "b.md")],
        exts=[".md", ".md"],
        preview_texts=[preview, preview],
        tokens=[["a"], ["b"]],
        sizes=[1, 1],
        mtimes=[1.0, 1.0],
        ctimes=[1.0, 1.0],
        owners=["", ""],
        drives=["", ""],
        extra_meta=[{"chunk_id": 1}, {"chunk_id": 2}],
    )
    return index


def test_vector_index_save_writes_meta_last(tmp_path: Path) -> None:
    cache = tmp_path / "cache"
    index = _build_small_index(tmp_path, preview="first")
    index.save(cache)

    emb = cache / "doc_embeddings.npy"
    meta = cache / "doc_meta.json"
    assert emb.exists()
    assert meta.exists()
    assert meta.stat().st_mtime >= emb.stat().st_mtime

    before_meta = json.loads(meta.read_text(encoding="utf-8"))
    assert before_meta["preview"][0] == "first"

    index2 = _build_small_index(tmp_path, preview="second")
    index2.save(cache)

    after_meta = json.loads(meta.read_text(encoding="utf-8"))
    assert after_meta["preview"][0] == "second"

