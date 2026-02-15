"""Path helpers shared across InfoPilot modules."""
from __future__ import annotations

from pathlib import Path
from functools import lru_cache


@lru_cache(maxsize=1)
def resolve_repo_root(start: Path | None = None) -> Path:
    """Return repository root by walking up until a .git directory is found."""
    current = Path(start or Path(__file__).resolve()).parent
    for parent in [current] + list(current.parents):
        if (parent / ".git").exists():
            return parent
    return current
