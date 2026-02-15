from pathlib import Path

import pytest

from scripts.run_meeting_agent import _discover_meeting_source, _load_policy_engine, MEETING_AGENT


def test_meeting_policy_blocks_sensitive(tmp_path: Path):
    audio_dir = tmp_path / "folder"
    audio_dir.mkdir(parents=True, exist_ok=True)
    allowed = audio_dir / "call.m4a"
    blocked = audio_dir / "secret" / "call.m4a"
    blocked.parent.mkdir(parents=True, exist_ok=True)
    allowed.write_bytes(b"ok")
    blocked.write_bytes(b"no")

    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        """
        [
          {
            "path": ".",
            "agents": ["meeting"],
            "sensitive_paths": ["./folder/secret"]
          }
        ]
        """,
        encoding="utf-8",
    )
    engine = _load_policy_engine(str(policy_json))

    picked = _discover_meeting_source(audio_dir, debug=False, policy_engine=engine)
    assert picked == allowed

    # If only sensitive file exists, discovery should fail
    allowed.unlink()
    with pytest.raises(FileNotFoundError):
        _discover_meeting_source(audio_dir, debug=False, policy_engine=engine)
