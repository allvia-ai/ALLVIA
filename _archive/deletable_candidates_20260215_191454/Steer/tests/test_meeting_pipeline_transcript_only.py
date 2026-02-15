from __future__ import annotations

from pathlib import Path

import pytest

from core.agents.meeting.models import MeetingJobConfig
from core.agents.meeting.pipeline import MeetingPipeline


@pytest.mark.smoke
def test_meeting_pipeline_runs_with_transcript_file(tmp_path: Path) -> None:
    transcript = tmp_path / "meeting.txt"
    transcript.write_text(
        "오늘 회의 요약입니다.\n결정: 다음주에 배포합니다.\n액션: 문서 정리하기.\n",
        encoding="utf-8",
    )
    output_dir = tmp_path / "out"

    job = MeetingJobConfig(audio_path=transcript, output_dir=output_dir, language="ko")
    pipeline = MeetingPipeline(stt_backend="placeholder", summary_backend="heuristic")
    summary = pipeline.run(job)

    assert summary.raw_summary
    assert (output_dir / "summary.json").exists()
    assert (output_dir / "summary.md").exists()


@pytest.mark.smoke
def test_meeting_pipeline_defaults_to_heuristic_summary_backend(monkeypatch: pytest.MonkeyPatch) -> None:
    for key in (
        "MEETING_SUMMARY_BACKEND",
        "MEETING_SUMMARY_LLAMA_MODEL",
        "MEETING_ALLOW_LLAMA_CPP",
        "MEETING_LLAMA_CPP_SUBPROCESS",
    ):
        monkeypatch.delenv(key, raising=False)
    pipeline = MeetingPipeline(stt_backend="placeholder")
    assert pipeline.summary_backend == "heuristic"
