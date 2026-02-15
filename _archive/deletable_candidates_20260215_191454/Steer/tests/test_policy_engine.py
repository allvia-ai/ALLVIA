from pathlib import Path

import pytest

from core.policy.engine import PolicyEngine


def test_policy_sensitive_paths_excluded(tmp_path: Path):
    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        """
        [
          {
            "path": ".",
            "agents": ["knowledge_search"],
            "sensitive_paths": ["./secret", "./hidden/sub"],
            "cache": {"max_bytes": 1024}
          }
        ]
        """,
        encoding="utf-8",
    )
    engine = PolicyEngine.from_file(policy_json)
    assert engine.has_policies

    allowed_root = tmp_path
    secret = (tmp_path / "secret" / "file.txt").resolve()
    nested = (tmp_path / "hidden" / "sub" / "doc.pdf").resolve()
    public = (tmp_path / "public" / "doc.pdf").resolve()

    # Sensitive paths are excluded
    assert not engine.allows(secret, agent="knowledge_search", include_manual=True)
    assert not engine.allows(nested, agent="knowledge_search", include_manual=True)

    # Non-sensitive path under root is allowed
    assert engine.allows(public, agent="knowledge_search", include_manual=True)


def test_policy_cache_limit_parsed(tmp_path: Path):
    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        """
        [
          {
            "path": ".",
            "agents": ["knowledge_search"],
            "cache": {"max_bytes": 2048, "purge_days": 7}
          }
        ]
        """,
        encoding="utf-8",
    )
    engine = PolicyEngine.from_file(policy_json)
    policy = engine.policy_for_path(tmp_path)
    assert policy is not None
    assert policy.cache.get("max_bytes") == 2048
    assert policy.cache.get("purge_days") == 7


def test_policy_allows_respects_type_rules(tmp_path: Path):
    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        """
        [
          {
            "path": ".",
            "agents": ["knowledge_search"],
            "allow_types": ["md", ".txt"],
            "deny_types": ["zip"]
          }
        ]
        """,
        encoding="utf-8",
    )
    engine = PolicyEngine.from_file(policy_json)
    allowed = tmp_path / "doc.md"
    blocked = tmp_path / "secret.pdf"
    denied = tmp_path / "archive.zip"
    allowed.write_text("ok", encoding="utf-8")
    blocked.write_text("no", encoding="utf-8")
    denied.write_text("no", encoding="utf-8")

    assert engine.allows(allowed, agent="knowledge_search", include_manual=True)
    assert not engine.allows(blocked, agent="knowledge_search", include_manual=True)
    assert not engine.allows(denied, agent="knowledge_search", include_manual=True)


def test_policy_allows_respects_size_limit(tmp_path: Path):
    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        """
        [
          {
            "path": ".",
            "agents": ["knowledge_search"],
            "max_file_size_mb": 0
          }
        ]
        """,
        encoding="utf-8",
    )
    engine = PolicyEngine.from_file(policy_json)
    tiny = tmp_path / "tiny.txt"
    tiny.write_text("1", encoding="utf-8")
    assert not engine.allows(tiny, agent="knowledge_search", include_manual=True)


def test_policy_pii_mask_toggle_from_agent_rules(tmp_path: Path):
    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        """
        [
          {
            "path": ".",
            "agents": ["meeting"],
            "agent_rules": {
              "meeting": {
                "masking": {"email": true}
              }
            }
          }
        ]
        """,
        encoding="utf-8",
    )
    engine = PolicyEngine.from_file(policy_json)
    meeting_file = tmp_path / "audio.wav"
    meeting_file.write_text("noop", encoding="utf-8")
    assert engine.pii_mask_enabled_for_path(meeting_file, agent="meeting") is True
