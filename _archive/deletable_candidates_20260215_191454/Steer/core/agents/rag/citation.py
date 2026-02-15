from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set, Tuple

_REF_RE = re.compile(r"\[ref:\s*([^\]]+)\]", flags=re.IGNORECASE)


def format_ref_id(path: str, chunk_id: Any) -> str:
    """Format a stable citation identifier for a hit."""
    name = ""
    try:
        name = Path(str(path)).name
    except Exception:
        name = str(path or "").strip()
    if not name:
        name = "unknown"
    stem = Path(name).stem or name
    try:
        chunk = int(chunk_id)
    except (TypeError, ValueError):
        chunk = 0
    return f"{stem}#{chunk}"


def build_allowed_refs(hits: Sequence[Dict[str, Any]]) -> Set[str]:
    allowed: Set[str] = set()
    for hit in hits:
        if not isinstance(hit, dict):
            continue
        path = hit.get("path") or hit.get("file") or ""
        chunk_id = hit.get("chunk_id")
        allowed.add(format_ref_id(str(path), chunk_id))
    return allowed


def extract_refs(text: str) -> List[str]:
    if not text:
        return []
    refs: List[str] = []
    for match in _REF_RE.finditer(text):
        value = (match.group(1) or "").strip()
        if value:
            refs.append(value)
    return refs


@dataclass(frozen=True)
class CitationCheck:
    has_any: bool
    unknown_refs: Tuple[str, ...]


def check_citations(text: str, *, allowed_refs: Set[str]) -> CitationCheck:
    refs = extract_refs(text)
    unknown = [ref for ref in refs if ref not in allowed_refs]
    return CitationCheck(has_any=bool(refs), unknown_refs=tuple(dict.fromkeys(unknown)))


def _sources_block(hits: Sequence[Dict[str, Any]], *, limit: int = 5) -> str:
    lines = ["", "Sources:"]
    for hit in list(hits)[: max(1, int(limit))]:
        if not isinstance(hit, dict):
            continue
        ref_id = format_ref_id(str(hit.get("path") or ""), hit.get("chunk_id"))
        try:
            file_name = Path(str(hit.get("path") or "")).name
        except Exception:
            file_name = str(hit.get("path") or "")
        lines.append(f"- {file_name} [{ref_id}]")
    return "\n".join(lines).rstrip()


def ensure_citations_or_append_sources(
    text: str,
    hits: Sequence[Dict[str, Any]],
    *,
    allowed_refs: Optional[Set[str]] = None,
    sources_limit: int = 5,
) -> str:
    """Ensure the response contains citations; append a sources section if missing or invalid."""
    allowed = allowed_refs if allowed_refs is not None else build_allowed_refs(hits)
    check = check_citations(text, allowed_refs=allowed)
    if check.has_any and not check.unknown_refs:
        return text
    suffix = _sources_block(hits, limit=sources_limit)
    if not check.has_any:
        return (text.rstrip() + "\n" + suffix).strip() + "\n"
    warning = ""
    if check.unknown_refs:
        unknown = ", ".join(check.unknown_refs[:6])
        warning = f"\n\n(⚠️ 일부 인용 표기가 유효하지 않습니다: {unknown})"
    return (text.rstrip() + warning + "\n" + suffix).strip() + "\n"

