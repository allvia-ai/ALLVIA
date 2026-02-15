from __future__ import annotations

from pathlib import Path

import pandas as pd

from core.data_pipeline.pipeline import CorpusBuilder


def test_pipeline_extract_masks_pii_when_policy_enabled(tmp_path: Path) -> None:
    sample = (
        "연락처: 010-1234-5678\n"
        "주민번호: 900101-1234567\n"
        "이메일: test.user@example.com\n"
        "주소: 서울특별시 강남구 테헤란로 123\n"
    )
    doc_path = tmp_path / "note.txt"
    doc_path.write_text(sample, encoding="utf-8")

    builder = CorpusBuilder(progress=False, translate=False, max_workers=1)
    df = builder.build(
        [
            {
                "path": str(doc_path),
                "size": doc_path.stat().st_size,
                "mtime": doc_path.stat().st_mtime,
                "ext": ".txt",
                "policy_mask_pii": True,
            }
        ]
    )

    assert isinstance(df, pd.DataFrame)
    assert len(df) == 1
    text = str(df.loc[0, "text"])
    original = str(df.loc[0, "text_original"])

    for value in (text, original):
        assert "010-1234-5678" not in value
        assert "900101-1234567" not in value
        assert "test.user@example.com" not in value
        assert "[REDACTED_PHONE]" in value
        assert "[REDACTED_RRN]" in value
        assert "[REDACTED_EMAIL]" in value
