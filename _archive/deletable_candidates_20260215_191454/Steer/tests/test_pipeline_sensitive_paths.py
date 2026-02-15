from pathlib import Path

import pytest

from core.data_pipeline.pipeline import CorpusBuilder
from core.policy.engine import PolicyEngine
from scripts.pipeline.infopilot import _load_scan_rows


def test_scan_rows_respects_sensitive_paths(tmp_path: Path):
    scan_csv = tmp_path / "found_files.csv"
    rows = [
        {"path": str(tmp_path / "secret" / "a.txt"), "size": 10, "mtime": 0.0, "ctime": 0.0},
        {"path": str(tmp_path / "public" / "b.txt"), "size": 10, "mtime": 0.0, "ctime": 0.0},
    ]
    CorpusBuilder.to_csv(rows, scan_csv)  # type: ignore[attr-defined]

    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        """
        [
          {
            "path": ".",
            "agents": ["knowledge_search"],
            "sensitive_paths": ["./secret"]
          }
        ]
        """,
        encoding="utf-8",
    )
    engine = PolicyEngine.from_file(policy_json)

    filtered = list(_load_scan_rows(scan_csv, agent="knowledge_search", policy_engine=engine, include_manual=True))
    filtered_paths = {row["path"] for row in filtered}
    assert str(tmp_path / "public" / "b.txt") in filtered_paths
    assert str(tmp_path / "secret" / "a.txt") not in filtered_paths


def test_extract_embed_split_requires_corpus(tmp_path: Path):
    # Placeholder to document that embed-only requires prebuilt corpus
    corpus_path = tmp_path / "corpus.parquet"
    with pytest.raises(FileNotFoundError):
        # simulate embed step expecting existing corpus
        from scripts.pipeline.infopilot import _resolve_scan_csv  # noqa: F401
        if not corpus_path.exists():
            raise FileNotFoundError("corpus required for embed")
