from __future__ import annotations

from pathlib import Path

from core.agents.photo import PhotoJobConfig, PhotoPipeline


def test_photo_pipeline_scans_and_reports(tmp_path: Path) -> None:
    root = tmp_path / "photos"
    root.mkdir()
    (root / "a.jpg").write_bytes(b"a")
    (root / "b.jpg").write_bytes(b"b")
    output = tmp_path / "out"

    config = PhotoJobConfig(roots=[root], output_dir=output)
    pipeline = PhotoPipeline()
    recommendation = pipeline.run(config)

    assert recommendation.report_path.exists()
    assert recommendation.best_shots
    assert recommendation.best_shots[0].path.exists()
