from __future__ import annotations

import json
import os
from pathlib import Path

import pytest

from core.agents.photo.models import PhotoJobConfig
from core.agents.photo.pipeline import PhotoPipeline


@pytest.mark.smoke
def test_photo_organize_dry_run_does_not_move_files(tmp_path: Path) -> None:
    root = tmp_path / "photos"
    root.mkdir()
    src = root / "a.jpg"
    src.write_bytes(b"fake-jpg")
    os.utime(src, (1700000000, 1700000000))

    dest_root = root / "organized"
    output_dir = tmp_path / "out"

    job = PhotoJobConfig(
        roots=[root],
        output_dir=output_dir,
        organize=True,
        dry_run=True,
        dest_root=dest_root,
        organize_strategy="month",
    )
    pipeline = PhotoPipeline()
    rec = pipeline.run(job)

    assert src.exists()
    report = json.loads(rec.report_path.read_text(encoding="utf-8"))
    organize = report["organize"]
    assert organize["dry_run"] is True
    assert len(organize["planned"]) >= 1
    assert len(organize["applied"]) >= 1


@pytest.mark.smoke
def test_photo_organize_apply_moves_and_handles_collisions(tmp_path: Path) -> None:
    root = tmp_path / "photos"
    (root / "sub").mkdir(parents=True)
    src1 = root / "same.jpg"
    src2 = root / "sub" / "same.jpg"
    src1.write_bytes(b"one")
    src2.write_bytes(b"two")
    os.utime(src1, (1700000000, 1700000000))
    os.utime(src2, (1700000000, 1700000000))

    dest_root = root / "organized"
    output_dir = tmp_path / "out"

    job = PhotoJobConfig(
        roots=[root],
        output_dir=output_dir,
        organize=True,
        dry_run=False,
        dest_root=dest_root,
        organize_strategy="month",
    )
    pipeline = PhotoPipeline()
    rec = pipeline.run(job)

    assert not src1.exists()
    assert not src2.exists()
    report = json.loads(rec.report_path.read_text(encoding="utf-8"))
    organize = report["organize"]
    moved = [Path(item["dst"]) for item in organize["applied"]]
    assert len(moved) >= 2
    assert moved[0].exists()
    assert moved[1].exists()
    assert moved[0].name != moved[1].name

