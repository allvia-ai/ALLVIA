"""Load optional domain-specific metadata for document enrichment."""
from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path
from typing import Dict, Optional, Tuple

from core.config.paths import DATA_DIR, PROJECT_ROOT
from core.utils import get_logger

LOGGER = get_logger("data_pipeline.metadata")


def _metadata_sources(base: Path) -> Tuple[Path, ...]:
    if not base.exists():
        return ()
    sources = []
    root_file = base / "metadata.json"
    if root_file.exists():
        sources.append(root_file)
    for candidate in base.rglob("metadata.json"):
        if candidate == root_file:
            continue
        sources.append(candidate)
    # preserve order while removing duplicates
    ordered = tuple(dict.fromkeys(sources))
    return ordered


@lru_cache(maxsize=1)
def _load_metadata_indexes() -> Tuple[Dict[str, str], Dict[str, str]]:
    """Return lookup dictionaries keyed by basename and resolved path."""
    base = DATA_DIR / "정답지"
    sources = _metadata_sources(base)
    if not sources:
        return {}, {}

    by_name: Dict[str, str] = {}
    by_resolved: Dict[str, str] = {}
    for path in sources:
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except Exception as exc:  # pragma: no cover - defensive guard
            LOGGER.warning("Failed to load metadata from %s: %s", path, exc)
            continue

        source_dir = path.parent

        for entry in data if isinstance(data, list) else []:
            if not isinstance(entry, dict):
                continue
            file_name = str(entry.get("file_name") or "").strip()
            title = str(entry.get("document_title") or "").strip()
            description = str(entry.get("description") or "").strip()
            if not file_name:
                continue

            parts = [part for part in (title, description) if part]
            if not parts:
                continue
            text = "\n".join(parts)

            by_name[file_name] = text
            by_name[file_name.casefold()] = text

            full_path = source_dir / file_name
            for candidate in _path_candidates(full_path):
                by_resolved[candidate] = text
                by_resolved[candidate.casefold()] = text

    return by_name, by_resolved


def _path_candidates(path: Path) -> Tuple[str, ...]:
    """Derive multiple path string variants for robust lookup."""
    candidates = []
    try:
        candidates.append(str(path.resolve()))
    except Exception:
        candidates.append(str(path))
    candidates.append(str(path))
    try:
        candidates.append(str(path.relative_to(PROJECT_ROOT)))
    except Exception:
        pass
    return tuple(dict.fromkeys(candidates))  # preserve order, drop duplicates


def get_metadata_for_path(path_str: str) -> Optional[str]:
    """Return descriptive metadata text for the given file path, if any."""
    if not path_str:
        return None

    try:
        path = Path(path_str)
    except Exception:
        path = None
    by_name, by_resolved = _load_metadata_indexes()

    if path:
        name = path.name
        if name:
            direct = by_name.get(name)
            if direct:
                return direct
            folded = name.casefold()
            if folded:
                hit = by_name.get(folded)
                if hit:
                    return hit
        for candidate in _path_candidates(path):
            meta = by_resolved.get(candidate)
            if meta:
                return meta
            folded = candidate.casefold()
            if folded:
                meta_cf = by_resolved.get(folded)
                if meta_cf:
                    return meta_cf
    # fallback to basename lookup even if Path creation failed
    fallback = by_name.get(path_str) or by_name.get(path_str.casefold())
    return fallback


__all__ = ["get_metadata_for_path"]
