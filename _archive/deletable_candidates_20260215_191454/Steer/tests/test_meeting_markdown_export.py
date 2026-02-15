from __future__ import annotations

import json
from pathlib import Path

import pytest

from core.agents.meeting.models import MeetingJobConfig, MeetingSummary, MeetingTranscriptionResult
from core.agents.meeting.persistence import export_integrations


@pytest.mark.smoke
def test_export_integrations_writes_summary_markdown(tmp_path: Path) -> None:
    audio_path = tmp_path / "meeting.txt"
    audio_path.write_text("hello", encoding="utf-8")
    output_dir = tmp_path / "out"
    output_dir.mkdir()

    job = MeetingJobConfig(audio_path=audio_path, output_dir=output_dir)
    transcription = MeetingTranscriptionResult(
        text="hello",
        segments=[{"start": 0.0, "end": 1.0, "text": "hello"}],
        duration_seconds=1.0,
        language="ko",
    )
    summary = MeetingSummary(
        id="dummy",
        highlights=["h"],
        action_items=["a"],
        decisions=["d"],
        raw_summary="요약 테스트",
        transcript_path=output_dir / "transcript.txt",
        structured_summary={
            "highlights": [{"text": "h", "ref": "00:00"}],
            "action_items": [{"text": "a", "ref": "00:00"}],
            "decisions": [{"text": "d", "ref": "00:00"}],
        },
    )

    (output_dir / "summary.json").write_text(json.dumps({"attachments": {}}, ensure_ascii=False), encoding="utf-8")
    attachments = export_integrations(job, transcription, summary)

    assert attachments["summary_md"] == "summary.md"
    content = (output_dir / "summary.md").read_text(encoding="utf-8")
    assert "## Summary" in content
    assert "요약 테스트" in content

