from __future__ import annotations

import json
from pathlib import Path

import pytest

from core.data_pipeline.pipeline import ExtractRecord, TrainConfig, run_step2


@pytest.mark.smoke
def test_run_step2_skips_paths_marked_cached_by_scan_state(tmp_path: Path) -> None:
    # A path that does not exist on disk. If run_step2 tries to extract it, extraction will fail.
    missing_path = tmp_path / "does_not_exist.txt"

    scan_rows = [
        {
            "path": str(missing_path),
            "ext": ".txt",
            "size": 123,
            "mtime": 1000.0,
        }
    ]

    scan_state_path = tmp_path / "scan_state.json"
    scan_state_path.write_text(
        json.dumps(
            {
                "paths": {str(missing_path): {"size": 123, "mtime": 1000.0}},
                "last_scan_timestamp": 1000.0,
            },
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )

    # Existing corpus is empty (e.g., previous run deduplicated everything away).
    import pandas as pd

    out_corpus = tmp_path / "corpus.parquet"
    empty = pd.DataFrame(columns=list(ExtractRecord.__annotations__.keys()))
    engine = "pyarrow"
    try:
        import fastparquet  # noqa: F401

        engine = "fastparquet"
    except Exception:
        pass
    empty.to_parquet(out_corpus, engine=engine, index=False)

    out_model = tmp_path / "topic_model.joblib"

    df, model = run_step2(
        scan_rows,
        out_corpus=out_corpus,
        out_model=out_model,
        cfg=TrainConfig(use_sentence_transformer=False),
        use_tqdm=False,
        translate=False,
        scan_state_path=scan_state_path,
        chunk_cache_path=None,
    )

    assert model is None
    assert df is not None
