from __future__ import annotations

import json
from pathlib import Path

import pytest

from core.agents import AgentRequest
from core.agents.photo.agent import PhotoAgent, PhotoAgentConfig
from core.policy.engine import PolicyEngine


@pytest.mark.smoke
def test_photo_agent_can_infer_roots_from_policy(tmp_path: Path) -> None:
    photo_root = tmp_path / "photos"
    photo_root.mkdir()
    (photo_root / "a.jpg").write_bytes(b"fake")

    policy_path = tmp_path / "smart_folders.json"
    policy_path.write_text(
        json.dumps(
            [
                {
                    "id": "photos-default",
                    "label": "Photos",
                    "type": "photos",
                    "path": str(photo_root),
                    "scope": "policy",
                    "policy": "core/config/hybrid.yaml",
                }
            ],
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )
    engine = PolicyEngine.from_file(policy_path)
    out_root = tmp_path / "out"
    agent = PhotoAgent(PhotoAgentConfig(output_root=out_root, policy_engine=engine))
    agent.prepare()

    result = agent.run(AgentRequest(query="사진 정리", context={"policy_engine": engine}))
    assert result.metadata.get("report_path")
    assert "사진" in result.content
