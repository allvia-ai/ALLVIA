from __future__ import annotations

from pathlib import Path

from core.data_pipeline.pipeline import run_step2, TrainConfig


def test_large_corpus_chunking(tmp_path: Path) -> None:
    rows = []
    for idx in range(500):
        rows.append(
            {
                "path": str(tmp_path / f"doc_{idx}.txt"),
                "ext": ".txt",
                "size": 1024,
                "mtime": 0.0,
                "ctime": 0.0,
                "drive": "tmp",
                "owner": "tester",
                "text": "sample text " * 50,
            }
        )
    corpus = tmp_path / "corpus.parquet"
    model = tmp_path / "model.joblib"
    cfg = TrainConfig(
        max_features=1000,
        n_components=64,
        n_clusters=10,
        ngram_range=(1, 2),
        min_df=1,
        max_df=1.0,
        use_sentence_transformer=False,
        embedding_model="",
        embedding_batch_size=16,
    )
    df, _ = run_step2(rows, out_corpus=corpus, out_model=model, cfg=cfg, use_tqdm=False, translate=False)
    assert len(df) == len(rows)
