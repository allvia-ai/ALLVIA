from __future__ import annotations

import json
import zipfile
from pathlib import Path

from core.agents.meeting.dataset_loader import load_transcript_summary_pairs
import pytest


def _write_transcript_pair(root: Path, transcript: str, summary: str) -> None:
    pair_dir = root / "pair"
    pair_dir.mkdir(parents=True, exist_ok=True)
    (pair_dir / "transcript.txt").write_text(transcript, encoding="utf-8")
    (pair_dir / "summary.json").write_text(json.dumps({"summary": summary}, ensure_ascii=False), encoding="utf-8")


def _write_archive(root: Path, archive_name: str) -> None:
    archive_dir = root / "archives"
    archive_dir.mkdir(parents=True, exist_ok=True)
    archive_path = archive_dir / archive_name
    payload = {
        "documents": [
            {
                "text": [{"sentence": "문장 A입니다."}, {"sentence": "문장 B입니다."}],
                "abstractive": ["두 문장을 한 줄로 요약합니다."],
            }
        ]
    }
    with zipfile.ZipFile(archive_path, "w") as zf:
        zf.writestr("train_original.json", json.dumps(payload, ensure_ascii=False))


def _write_special_json(root: Path) -> None:
    json_dir = root / "special"
    json_dir.mkdir(parents=True, exist_ok=True)
    payload = [
        {
            "id": "sample-0",
            "text": "첫 번째 장면의 전체 대사입니다.",
            "scenario": "첫 번째 장면을 요약합니다.",
        },
        {
            "id": "sample-1",
            "text": "두 번째 장면의 전체 대사입니다.",
            "event": "두 번째 장면을 요약합니다.",
        },
    ]
    (json_dir / "ko_text.json").write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")


@pytest.mark.full
def test_load_transcript_summary_pairs_all_formats(tmp_path):
    dataset_root = tmp_path / "dataset"
    dataset_root.mkdir()

    _write_transcript_pair(dataset_root, "회의 전체 내용입니다.", "한 줄 회의 요약.")
    _write_archive(dataset_root, "legal_train.zip")
    _write_special_json(dataset_root)

    transcripts, summaries = load_transcript_summary_pairs(dataset_root)

    assert len(transcripts) == len(summaries)
    assert len(transcripts) == 4

    summary_set = set(summaries)
    assert "한 줄 회의 요약." in summary_set
    assert "두 문장을 한 줄로 요약합니다." in summary_set
    assert "첫 번째 장면을 요약합니다." in summary_set
    assert "두 번째 장면을 요약합니다." in summary_set
