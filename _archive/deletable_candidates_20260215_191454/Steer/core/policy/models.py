"""Policy data models for smart folder configurations."""
from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Sequence

from core.utils import resolve_repo_root


def _normalize_path(path: Path) -> Path:
    path = path.expanduser()
    try:
        return path.resolve(strict=False)
    except TypeError:  # Python <3.9 strict arg not supported
        try:
            return path.resolve()
        except OSError:
            return path
    except OSError:
        return path


def _normalize_ext(value: str) -> str:
    raw = (value or "").strip().lower()
    if not raw:
        return ""
    if not raw.startswith("."):
        raw = f".{raw}"
    return raw


def _normalize_exts(values: object) -> frozenset[str]:
    if not values:
        return frozenset()
    if not isinstance(values, list):
        values = [values]
    normalized = []
    for item in values:
        if item is None:
            continue
        ext = _normalize_ext(str(item))
        if ext:
            normalized.append(ext)
    return frozenset(normalized)


@dataclass(frozen=True)
class AgentRule:
    allow_types: frozenset[str] = frozenset()
    deny_types: frozenset[str] = frozenset()
    max_file_size_mb: int | None = None
    masking: Dict[str, bool] = field(default_factory=dict)


@dataclass(frozen=True)
class SmartFolderPolicy:
    path: Path
    id: str = ""
    label: str = ""
    folder_type: str = ""
    scope: str = ""
    legacy_policy_tag: str = ""
    agents: frozenset[str] = frozenset()
    sensitive_paths: frozenset[Path] = frozenset()
    allow_types: frozenset[str] = frozenset()
    deny_types: frozenset[str] = frozenset()
    max_file_size_mb: int | None = None
    agent_rules: Dict[str, AgentRule] = field(default_factory=dict)
    security: Dict[str, object] = field(default_factory=dict)
    indexing: Dict[str, object] = field(default_factory=dict)
    retention: Dict[str, object] = field(default_factory=dict)
    cache: Dict[str, object] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, data: Dict[str, object], *, base: Path) -> "SmartFolderPolicy":
        if "path" not in data:
            raise ValueError("Smart folder policy requires a 'path' key")
        raw_path_value = data.get("path")
        raw_path_str = str(raw_path_value or "").strip()
        folder_type = str(data.get("type") or "").strip()
        folder_type_lower = folder_type.lower()

        if not raw_path_str:
            if folder_type_lower == "global":
                raw_path = resolve_repo_root()
            else:
                raise ValueError("Smart folder policy requires a non-empty 'path' for non-global entries")
        else:
            raw_path = Path(raw_path_str).expanduser()
            if not raw_path.is_absolute():
                raw_path = base / raw_path
        normalized_path = _normalize_path(raw_path)
        agents = frozenset(str(item) for item in data.get("agents", []) or [])
        allow_types = _normalize_exts(data.get("allow_types"))
        deny_types = _normalize_exts(data.get("deny_types"))
        max_file_size_mb_raw = data.get("max_file_size_mb")
        max_file_size_mb: int | None
        if max_file_size_mb_raw is None or max_file_size_mb_raw == "":
            max_file_size_mb = None
        else:
            try:
                max_file_size_mb = int(max_file_size_mb_raw)  # type: ignore[arg-type]
            except (TypeError, ValueError):
                max_file_size_mb = None

        agent_rules: Dict[str, AgentRule] = {}
        raw_agent_rules = data.get("agent_rules") or {}
        if isinstance(raw_agent_rules, dict):
            for agent_name, raw_rule in raw_agent_rules.items():
                if not agent_name:
                    continue
                if not isinstance(raw_rule, dict):
                    continue
                rule_allow = _normalize_exts(raw_rule.get("allow_types"))
                rule_deny = _normalize_exts(raw_rule.get("deny_types"))
                rule_max_raw = raw_rule.get("max_file_size_mb")
                rule_max: int | None
                if rule_max_raw is None or rule_max_raw == "":
                    rule_max = None
                else:
                    try:
                        rule_max = int(rule_max_raw)  # type: ignore[arg-type]
                    except (TypeError, ValueError):
                        rule_max = None
                masking_raw = raw_rule.get("masking") or {}
                masking: Dict[str, bool] = {}
                if isinstance(masking_raw, dict):
                    for key, value in masking_raw.items():
                        if not key:
                            continue
                        masking[str(key)] = bool(value)
                agent_rules[str(agent_name)] = AgentRule(
                    allow_types=rule_allow,
                    deny_types=rule_deny,
                    max_file_size_mb=rule_max,
                    masking=masking,
                )

        sensitive_paths_raw = data.get("sensitive_paths") or []
        sensitive_paths: List[Path] = []
        for entry in sensitive_paths_raw:
            if entry is None:
                continue
            entry_str = str(entry).strip()
            if not entry_str:
                continue
            entry_path = Path(entry_str).expanduser()
            if not entry_path.is_absolute():
                entry_path = base / entry_path
            sensitive_paths.append(_normalize_path(entry_path))
        security = dict(data.get("security", {}) or {})
        indexing = dict(data.get("indexing", {}) or {})
        retention = dict(data.get("retention", {}) or {})
        cache = dict(data.get("cache", {}) or {})
        return cls(
            id=str(data.get("id") or "").strip(),
            label=str(data.get("label") or "").strip(),
            folder_type=folder_type_lower,
            scope=str(data.get("scope") or "").strip(),
            legacy_policy_tag=str(data.get("policy") or "").strip(),
            path=normalized_path,
            agents=agents,
            sensitive_paths=frozenset(sensitive_paths),
            allow_types=allow_types,
            deny_types=deny_types,
            max_file_size_mb=max_file_size_mb,
            agent_rules=agent_rules,
            security=security,
            indexing=indexing,
            retention=retention,
            cache=cache,
        )

    @property
    def indexing_mode(self) -> str:
        mode = str(self.indexing.get("mode", "realtime") or "realtime").lower()
        if mode not in {"realtime", "scheduled", "manual"}:
            return "realtime"
        return mode

    def allows_agent(self, agent: str) -> bool:
        if not self.agents:
            return True
        return agent in self.agents

    def is_sensitive(self, path: Path) -> bool:
        if not self.sensitive_paths:
            return False
        normalized = _normalize_path(path)
        for sensitive_root in self.sensitive_paths:
            try:
                normalized.relative_to(sensitive_root)
                return True
            except ValueError:
                continue
        return False
