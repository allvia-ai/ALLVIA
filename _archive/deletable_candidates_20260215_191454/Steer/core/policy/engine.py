"""Policy engine for smart folder configurations."""
from __future__ import annotations

from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Tuple

from core.policy.loader import load_policy_file
from core.policy.models import (
    SmartFolderPolicy,
    _normalize_ext,
    _normalize_path,
)
from core.utils import get_logger, resolve_repo_root

LOGGER = get_logger("policy.engine")


class PolicyEngine:
    def __init__(self, policies: Sequence[SmartFolderPolicy], *, source: Optional[Path] = None) -> None:
        self._policies = sorted(policies, key=lambda p: len(p.path.parts), reverse=True)
        self.source = source

    @classmethod
    def empty(cls) -> "PolicyEngine":
        return cls((), source=None)

    @classmethod
    def from_file(cls, path: Path) -> "PolicyEngine":
        repo_root = resolve_repo_root()
        if not path.is_absolute():
            path = (repo_root / path).resolve()
        if not path.exists():
            LOGGER.info("Policy file not found at %s; continuing without policies", path)
            return cls.empty()
        
        raw_policies = load_policy_file(path)
        # Use from_dict from models
        policies = [SmartFolderPolicy.from_dict(entry, base=path.parent) for entry in raw_policies]
        LOGGER.info("Loaded %d smart folder policies from %s", len(policies), path)
        return cls(policies, source=path)

    def __len__(self) -> int:
        return len(self._policies)

    @property
    def has_policies(self) -> bool:
        return bool(self._policies)

    def roots_for_agent(self, agent: str, *, include_manual: bool = True) -> List[Path]:
        if not self._policies:
            return []
        roots: List[Path] = []
        for policy in self._policies:
            if not policy.allows_agent(agent):
                continue
            if not include_manual and policy.indexing_mode == "manual":
                continue
            roots.append(policy.path)
        # remove duplicates while preserving order
        seen = set()
        unique: List[Path] = []
        for root in roots:
            key = str(root)
            if key in seen:
                continue
            seen.add(key)
            unique.append(root)
        return unique

    def roots_for_type(self, folder_type: str, *, include_manual: bool = True) -> List[Path]:
        """Return roots for policies matching `folder_type` (case-insensitive)."""
        if not self._policies:
            return []
        wanted = str(folder_type or "").strip().lower()
        if not wanted:
            return []
        roots: List[Path] = []
        for policy in self._policies:
            if not include_manual and policy.indexing_mode == "manual":
                continue
            if str(getattr(policy, "folder_type", "") or "").lower() != wanted:
                continue
            roots.append(policy.path)
        seen = set()
        unique: List[Path] = []
        for root in roots:
            key = str(root)
            if key in seen:
                continue
            seen.add(key)
            unique.append(root)
        return unique

    def iter_policies(self) -> Sequence[SmartFolderPolicy]:
        return tuple(self._policies)

    def policy_for_path(self, path: Path) -> Optional[SmartFolderPolicy]:
        if not self._policies:
            return None
        normalized = _normalize_path(path)
        for policy in self._policies:
            try:
                normalized.relative_to(policy.path)
                return policy
            except ValueError:
                continue
        return None

    def check(self, path: Path, *, agent: str, include_manual: bool = True) -> Tuple[bool, str]:
        if not self._policies:
            return True, "no_policies"
        policy = self.policy_for_path(path)
        if policy is None:
            return False, "out_of_scope"
        if policy.is_sensitive(path):
            return False, "sensitive_path"
        if not policy.allows_agent(agent):
            return False, "agent_denied"
        if not include_manual and policy.indexing_mode == "manual":
            return False, "manual_policy"

        effective_allow = policy.allow_types
        effective_deny = policy.deny_types
        max_size_mb = policy.max_file_size_mb
        rule = policy.agent_rules.get(agent)
        if rule:
            if rule.allow_types:
                effective_allow = rule.allow_types
            if rule.deny_types:
                effective_deny = frozenset(set(effective_deny) | set(rule.deny_types))
            if rule.max_file_size_mb is not None:
                max_size_mb = rule.max_file_size_mb

        if path.is_file():
            ext = _normalize_ext(path.suffix)
            if effective_deny and ext in effective_deny:
                return False, "type_denied"
            if effective_allow and ext not in effective_allow:
                return False, "type_not_allowed"
            if max_size_mb is not None:
                try:
                    size_bytes = path.stat().st_size
                except OSError:
                    return False, "stat_failed"
                if size_bytes > int(max_size_mb) * 1024 * 1024:
                    return False, "file_too_large"
        return True, "ok"

    def allows(self, path: Path, *, agent: str, include_manual: bool = True) -> bool:
        allowed, _ = self.check(path, agent=agent, include_manual=include_manual)
        return allowed

    def masking_rules_for_path(self, path: Path, *, agent: str) -> Dict[str, bool]:
        """Return policy masking rules for a given path/agent."""
        policy = self.policy_for_path(path)
        if policy is None:
            return {}
        rule = policy.agent_rules.get(agent)
        if rule and rule.masking:
            return dict(rule.masking)
        return {}

    def pii_mask_enabled_for_path(self, path: Path, *, agent: str) -> bool:
        """Best-effort toggle to enable meeting-style PII masking."""
        policy = self.policy_for_path(path)
        if policy is None:
            return False
        rules = self.masking_rules_for_path(path, agent=agent)
        if any(bool(value) for value in rules.values()):
            return True
        security = policy.security or {}
        return bool(security.get("pii_filter", False))

    def filter_records(
        self,
        records: Iterable[Dict[str, object]],
        *,
        agent: str,
        include_manual: bool = True,
    ) -> List[Dict[str, object]]:
        if not self._policies:
            return list(records)
        filtered: List[Dict[str, object]] = []
        for record in records:
            path_str = record.get("path") if isinstance(record, dict) else None
            if not path_str:
                continue
            if self.allows(Path(str(path_str)), agent=agent, include_manual=include_manual):
                filtered.append(record)
        return filtered
