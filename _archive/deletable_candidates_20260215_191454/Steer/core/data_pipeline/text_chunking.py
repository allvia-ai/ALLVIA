# text_chunking.py - Text Chunking Utilities for Pipeline
"""
Text chunking functions extracted from pipeline.py.
These are legacy chunking functions; SemanticChunker in chunking_v2.py is preferred.
"""
from __future__ import annotations

import re
from typing import Any, Dict, List, Tuple

from core.utils.stopwords import STOPWORDS as _STOPWORDS

TOKEN_PATTERN = r'(?u)(?:[가-힣]{1,}|[A-Za-z0-9]{2,})'
_TOKEN_REGEX = re.compile(TOKEN_PATTERN)

DEFAULT_CHUNK_MIN_TOKENS = 200
DEFAULT_CHUNK_MAX_TOKENS = 500


def remove_stopwords(text: str) -> str:
    if not text:
        return ""
    kept: List[str] = []
    for match in _TOKEN_REGEX.finditer(text):
        token = match.group(0)
        token_norm = token.lower()
        if token_norm in _STOPWORDS:
            continue
        if token_norm.isdigit():
            continue
        if len(set(token_norm)) == 1 and len(token_norm) <= 3:
            continue
        kept.append(token)
    if not kept:
        return text.strip()
    return " ".join(kept)


def slice_text_by_ratio(source: str, start_char: int, end_char: int, base_len: int) -> str:
    if not source:
        return ""
    if base_len <= 0:
        return source.strip()
    length = len(source)
    start_ratio = max(0.0, min(1.0, float(start_char) / float(base_len)))
    end_ratio = max(start_ratio, min(1.0, float(end_char) / float(base_len)))
    start_idx = int(round(start_ratio * length))
    end_idx = int(round(end_ratio * length))
    if end_idx <= start_idx:
        end_idx = min(length, max(start_idx + 1, end_idx))
    return source[start_idx:end_idx].strip()


def token_chunk_spans(text: str, *, min_tokens: int, max_tokens: int) -> List[Tuple[int, int, int]]:
    if not text or not text.strip():
        cleaned = (text or "").strip()
        return [(0, len(text), 0)] if cleaned else []

    matches = list(_TOKEN_REGEX.finditer(text))
    total_tokens = len(matches)
    if total_tokens == 0:
        cleaned = text.strip()
        return [(0, len(text), 0)] if cleaned else []
    if total_tokens <= max_tokens:
        return [(0, len(text), total_tokens)]

    spans: List[Tuple[int, int, int]] = []
    start_index = 0
    prev_char = 0
    text_len = len(text)

    while start_index < total_tokens:
        end_index = min(start_index + max_tokens, total_tokens)
        remaining = total_tokens - end_index
        if remaining and remaining < min_tokens:
            end_index = total_tokens
        next_start_char = matches[end_index].start() if end_index < total_tokens else text_len
        span_start = prev_char
        span_end = next_start_char
        token_count = end_index - start_index
        chunk = text[span_start:span_end].strip()
        if chunk:
            spans.append((span_start, span_end, token_count))
        prev_char = next_start_char
        start_index = end_index

    if len(spans) >= 2 and spans[-1][2] < min_tokens:
        prev_start, _prev_end, prev_tokens = spans[-2]
        spans[-2] = (prev_start, spans[-1][1], prev_tokens + spans[-1][2])
        spans.pop()

    if spans and spans[-1][1] < text_len:
        start, _, tokens = spans[-1]
        spans[-1] = (start, text_len, tokens)

    return spans


def token_chunk_spans_with_overlap(
    text: str,
    *,
    min_tokens: int,
    max_tokens: int,
    overlap_tokens: int,
) -> List[Tuple[int, int, int]]:
    min_tokens = max(1, int(min_tokens))
    max_tokens = max(min_tokens, int(max_tokens))
    overlap = max(0, int(overlap_tokens))

    matches = list(_TOKEN_REGEX.finditer(text))
    if not matches:
        cleaned = (text or "").strip()
        return [(0, len(text), 0)] if cleaned else []

    total_tokens = len(matches)
    if total_tokens <= max_tokens:
        return [(0, len(text), total_tokens)]

    spans: List[Tuple[int, int, int]] = []
    start_index = 0
    text_len = len(text)
    while start_index < total_tokens:
        end_index = min(start_index + max_tokens, total_tokens)
        remaining = total_tokens - end_index
        if remaining and remaining < min_tokens:
            end_index = total_tokens

        start_char = matches[start_index].start()
        end_char = matches[end_index - 1].end() if end_index > start_index else min(text_len, start_char)
        chunk = text[start_char:end_char].strip()
        if chunk:
            spans.append((start_char, end_char, end_index - start_index))

        if end_index >= total_tokens:
            break
        next_start = end_index - overlap if overlap else end_index
        if next_start <= start_index:
            next_start = end_index
        start_index = next_start

    if spans and spans[-1][1] < text_len:
        start, _, tokens = spans[-1]
        spans[-1] = (start, text_len, tokens)

    return spans


_MD_HEADING_RE = re.compile(r"(?m)^(#{1,6})\s+(.+?)\s*$")
_MD_NUMBERED_HEADING_RE = re.compile(r"(?m)^\s*(\d+(?:\.\d+)*[\).])\\s+(.+?)\s*$")


def iter_markdown_sections(text: str) -> List[Tuple[int, int, str]]:
    """Return (start_char, end_char, heading_title) for markdown-ish sections."""
    if not text:
        return []
    headings: List[Tuple[int, str]] = []
    for match in _MD_HEADING_RE.finditer(text):
        title = (match.group(2) or "").strip()
        headings.append((match.start(), title))
    for match in _MD_NUMBERED_HEADING_RE.finditer(text):
        title = (match.group(2) or "").strip()
        headings.append((match.start(), title))
    if not headings:
        return [(0, len(text), "")]

    headings.sort(key=lambda item: item[0])
    sections: List[Tuple[int, int, str]] = []
    for idx, (start, title) in enumerate(headings):
        end = headings[idx + 1][0] if idx + 1 < len(headings) else len(text)
        sections.append((start, end, title))
    if sections and sections[0][0] > 0:
        sections.insert(0, (0, sections[0][0], ""))
    return [(s, e, t) for (s, e, t) in sections if s < e]


def is_markdown_record(record: Dict[str, Any]) -> bool:
    ext = str(record.get("ext") or "").lower()
    if ext == ".md":
        return True
    meta = record.get("meta")
    if isinstance(meta, dict) and str(meta.get("format") or "").lower() == "markdown":
        return True
    return False


def adaptive_chunk_window(text: str, base_min: int, base_max: int) -> Tuple[int, int]:
    base_min = max(16, int(base_min))
    base_max = max(base_min + 16, int(base_max))
    approx_tokens = len(_TOKEN_REGEX.findall(text)) or max(1, len(text) // 4)

    if approx_tokens <= base_max:
        min_tokens = max(16, int(base_min * 0.5))
        max_tokens = max(min_tokens + 24, int(base_max * 0.75))
        return min_tokens, max_tokens

    if approx_tokens <= base_max * 3:
        return base_min, base_max

    scale = min(2.0, approx_tokens / float(base_max * 3))
    min_tokens = int(base_min * (1.0 + (scale * 0.5)))
    max_tokens = int(base_max * (1.0 + (scale * 0.5)))
    min_tokens = max(base_min, min(min_tokens, 320))
    max_tokens = max(min_tokens + 32, min(1200, max_tokens))
    remainder = approx_tokens - max_tokens
    if remainder and remainder < min_tokens:
        adjustment = min_tokens - remainder
        max_tokens = max(min_tokens + 32, max_tokens - adjustment)
    return min_tokens, max_tokens


# Backward compatibility aliases
_remove_stopwords = remove_stopwords
_slice_text_by_ratio = slice_text_by_ratio
_token_chunk_spans = token_chunk_spans
_token_chunk_spans_with_overlap = token_chunk_spans_with_overlap
_iter_markdown_sections = iter_markdown_sections
_is_markdown_record = is_markdown_record
_adaptive_chunk_window = adaptive_chunk_window
