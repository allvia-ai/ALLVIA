from pathlib import Path

import pytest

from scripts.run_meeting_agent import _discover_meeting_source, _load_policy_engine, MeetingPipeline, MeetingJobConfig
from core.policy.engine import PolicyEngine
from core.agents.photo import PhotoAgent
from core.agents.photo.pipeline import PhotoJobConfig


def test_meeting_e2e_smoke(tmp_path: Path):
    fixtures = Path("tests/fixtures")
    audio_dir = tmp_path / "meeting"
    audio_dir.mkdir(parents=True, exist_ok=True)
    sample = fixtures / "meeting_sample.wav"
    target = audio_dir / "sample.wav"
    target.write_bytes(sample.read_bytes())

    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        """
        [
          {
            "path": ".",
            "agents": ["meeting"],
            "sensitive_paths": ["./secret"]
          }
        ]
        """,
        encoding="utf-8",
    )
    engine = _load_policy_engine(str(policy_json))
    picked = _discover_meeting_source(audio_dir, debug=False, policy_engine=engine)
    assert picked == target

    # E2E: run pipeline with txt as placeholder (pipeline expects audio but here we validate policy + run flow)
    job = MeetingJobConfig(
        audio_path=target,
        output_dir=audio_dir / ".ai_agent" / "meetings",
        context_dirs=[audio_dir],
        policy_tag=str(policy_json),
        enable_resume=False,
    )
    pipeline = MeetingPipeline()
    pipeline.run(job)  # noqa: B904


def test_photo_e2e_smoke(tmp_path: Path):
    fixtures = Path("tests/fixtures")
    photos_dir = tmp_path / "photos"
    photos_dir.mkdir(parents=True, exist_ok=True)
    sample = fixtures / "photo_sample.jpg"
    allowed = photos_dir / "public" / "a.jpg"
    blocked = photos_dir / "private" / "b.jpg"
    allowed.parent.mkdir(parents=True, exist_ok=True)
    blocked.parent.mkdir(parents=True, exist_ok=True)
    allowed.write_bytes(sample.read_bytes())
    blocked.write_bytes(sample.read_bytes())

    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        """
        [
          {
            "path": ".",
            "agents": ["photo"],
            "sensitive_paths": ["./photos/private"]
          }
        ]
        """,
        encoding="utf-8",
    )
    engine = PolicyEngine.from_file(policy_json)
    job = PhotoJobConfig(
        roots=[photos_dir],
        output_dir=photos_dir / ".ai_agent" / "photos",
        policy_engine=engine,
        policy_agent="photo",
    )
    agent = PhotoAgent(job)
    files = agent._collect_files()  # type: ignore[attr-defined]
    paths = {Path(f.path) for f in files}
    assert allowed in paths
    assert blocked not in paths
