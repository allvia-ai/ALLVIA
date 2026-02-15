from pathlib import Path

from core.agents.photo import PhotoAgent
from core.agents.photo.pipeline import PhotoJobConfig
from core.policy.engine import PolicyEngine


def test_photo_policy_blocks_sensitive(tmp_path: Path):
    root = tmp_path / "photos"
    root.mkdir(parents=True, exist_ok=True)
    allowed = root / "public" / "a.jpg"
    blocked = root / "private" / "b.jpg"
    allowed.parent.mkdir(parents=True, exist_ok=True)
    blocked.parent.mkdir(parents=True, exist_ok=True)
    allowed.write_bytes(b"1")
    blocked.write_bytes(b"2")

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
        roots=[root],
        output_dir=root / ".ai_agent" / "photos",
        policy_engine=engine,
        policy_agent="photo",
    )
    agent = PhotoAgent(job)
    files = agent._collect_files()  # type: ignore[attr-defined]
    paths = {Path(f.path) for f in files}
    assert allowed in paths
    assert blocked not in paths
