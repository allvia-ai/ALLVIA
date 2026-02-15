from __future__ import annotations

from pathlib import Path

import pytest

from core.data_pipeline.pipeline import ExcelLikeExtractor


@pytest.mark.smoke
def test_excel_extractor_skips_large_files(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setenv("INFOPILOT_EXCEL_MAX_BYTES", "1")

    fake_xlsx = tmp_path / "big.xlsx"
    fake_xlsx.write_bytes(b"xx")

    out = ExcelLikeExtractor().extract(fake_xlsx)

    assert out["ok"] is False
    assert "file too large" in (out.get("meta", {}).get("error", "") or "")
    assert out.get("meta", {}).get("size") == 2
    assert out.get("meta", {}).get("max_bytes") == 1
