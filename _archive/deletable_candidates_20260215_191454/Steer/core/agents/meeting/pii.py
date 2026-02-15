"""PII masking helpers for meeting artifacts."""
from __future__ import annotations

from typing import List, Optional

# Delegate to the centralized masking patterns
from .masking_patterns import mask_content as mask_text


def mask_segments(segments: Optional[List[dict]]) -> List[dict]:
    """Return a new list with text fields masked."""
    if not segments:
        return []
    masked_segments: List[dict] = []
    for segment in segments:
        if not isinstance(segment, dict):
            continue
        masked_segments.append(
            {
                **segment,
                "text": mask_text(segment.get("text") or ""),
            }
        )
    return masked_segments


__all__ = ["mask_text", "mask_segments"]
