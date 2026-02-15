# strict_search.py - Strict/Exact Search Filtering for Retriever
"""
Strict search filtering utilities for handling exact-match queries.
Extracted from retriever.py for modularity.
"""
from __future__ import annotations

import re
from pathlib import PurePath
from typing import Any, Dict, List, Set, Tuple

# ---------------------------------------------------------------------------
# Constants for Strict Search
# ---------------------------------------------------------------------------
_EXACT_TERM_RE = re.compile(r"^[가-힣]{2,4}$")

EXACT_TERM_STRIP_SUFFIXES: Tuple[str, ...] = (
    "관련문서",
    "관련자료",
    "관련",
    "문서",
    "자료",
    "찾아줘",
    "찾아",
    "검색",
    "찾기",
    "요약",
    "정리",
)

EXACT_TERM_STOPWORDS: Set[str] = {
    "관련",
    "문서",
    "자료",
    "검색",
    "찾아",
    "찾아줘",
    "찾기",
    "요약",
    "정리",
    "이력서",
    "resume",
    "cv",
    "경력기술서",
    "포트폴리오",
    "사업계획서",
    "사업계획",
    "비즈니스플랜",
    "business",
    "businessplan",
    "business plan",
    "pitch",
    "deck",
    "ir",
    "proposal",
    "제안서",
    "계약서",
    "견적서",
}

STRICT_KEYWORDS: Tuple[str, ...] = (
    "이력서",
    "resume",
    "cv",
    "경력기술서",
    "포트폴리오",
    "사업계획서",
    "사업계획",
    "비즈니스플랜",
    "business",
    "businessplan",
    "business plan",
    "pitch",
    "deck",
    "ir",
    "proposal",
    "제안서",
    "계약서",
    "견적서",
)

STRICT_INTENT_TOKENS: Tuple[str, ...] = (
    "찾아",
    "찾아줘",
    "찾기",
    "검색",
    "검색해",
    "검색해줘",
    "파일",
    "문서",
    "자료",
    "원본",
    "pdf",
    "doc",
    "docx",
    "hwp",
    "ppt",
    "pptx",
    "xlsx",
    "xls",
    "csv",
    "template",
    "템플릿",
    "양식",
    "샘플",
    "예시",
    "서식",
)


# ---------------------------------------------------------------------------
# Token Splitting (minimal local version)
# ---------------------------------------------------------------------------
_TOKEN_PATTERN = re.compile(r"(?u)(?:[가-힣]{1,}|[A-Za-z0-9]{2,})")


def _split_tokens_local(source: Any) -> List[str]:
    if not source:
        return []
    return _TOKEN_PATTERN.findall(str(source).lower())


# ---------------------------------------------------------------------------
# Path Utilities
# ---------------------------------------------------------------------------
def path_parts_lower(path: str) -> Tuple[str, ...]:
    if not path:
        return tuple()
    try:
        parts = PurePath(path).parts
    except Exception:
        return tuple()
    return tuple(str(part).lower() for part in parts if part)


def looks_like_identifier(token: str) -> bool:
    if not token:
        return False
    token = token.strip()
    if not token:
        return False
    if re.search(r"\d{3,}", token):
        return True
    if re.fullmatch(r"[A-Za-z]{2,6}-\d{2,}", token):
        return True
    return False


# ---------------------------------------------------------------------------
# Strict Search Functions
# ---------------------------------------------------------------------------
def should_apply_strict_search(
    query: str,
    split_tokens_fn=None,
    extract_exact_fn=None,
    extract_strict_fn=None,
) -> bool:
    """Heuristic: apply strict filtering only when the user likely wants a specific file."""
    if not query:
        return False
    lowered = str(query).strip().lower()
    
    split_fn = split_tokens_fn or _split_tokens_local
    tokens = [tok for tok in split_fn(lowered) if tok]
    if not tokens:
        return False

    # Avoid strict filtering for broad/long queries; semantic is better there.
    if len(tokens) >= 10:
        return False

    has_intent = any(tok in lowered for tok in STRICT_INTENT_TOKENS)
    
    exact_fn = extract_exact_fn or extract_exact_query_terms
    strict_fn = extract_strict_fn or extract_strict_keywords
    
    has_exact = bool(exact_fn(query))
    has_strict_kw = bool(strict_fn(query))
    has_identifier = any(looks_like_identifier(tok) for tok in tokens)
    return bool(has_intent and (has_exact or has_strict_kw or has_identifier))


def extract_strict_keywords(query: str) -> List[str]:
    if not query:
        return []
    lowered = str(query).strip().lower()
    tokens = {tok for tok in _split_tokens_local(lowered) if tok}
    matches: List[str] = []
    for kw in STRICT_KEYWORDS:
        kw_l = kw.lower()
        if kw_l in lowered or kw_l in tokens:
            matches.append(kw_l)
            # Expand intent keywords to likely synonyms found in filenames/content.
            if kw_l in {"이력서", "resume", "cv", "경력기술서", "포트폴리오"}:
                matches.extend(["이력서", "resume", "cv", "경력기술서", "포트폴리오", "portfolio", "profile"])
            if kw_l in {"사업계획서", "사업계획", "비즈니스플랜", "business plan", "businessplan"}:
                matches.extend(["사업계획서", "사업계획", "비즈니스플랜", "business", "businessplan", "plan", "proposal", "pitch", "deck", "ir"])
    seen: Set[str] = set()
    ordered: List[str] = []
    for kw in matches:
        if kw and kw not in seen:
            seen.add(kw)
            ordered.append(kw)
    return ordered[:5]


def apply_strict_filter(
    query: str,
    hits: List[Dict[str, Any]],
    collect_hit_tokens_fn=None,
) -> Tuple[List[Dict[str, Any]], bool]:
    """Filter hits to those that contain exact terms / strict keywords; returns (hits, fallback_used).

    When no candidates match, returns the original hits and sets fallback_used=True so the caller can
    annotate that strict filtering did not apply.
    """
    if not hits:
        return hits, False
    exact_terms = extract_exact_query_terms(query)
    strict_keywords = extract_strict_keywords(query)
    if not exact_terms and not strict_keywords:
        return hits, False

    # Default collect_hit_tokens just uses path
    def _default_collect_tokens(hit: Dict[str, Any]) -> Set[str]:
        text = str(hit.get("path", "") or "").lower()
        text += " " + str(hit.get("text", "") or "").lower()
        return set(_split_tokens_local(text))
    
    collect_fn = collect_hit_tokens_fn or _default_collect_tokens

    filtered: List[Dict[str, Any]] = []
    for hit in hits:
        hit_tokens = collect_fn(hit)
        if exact_terms and not all(term in hit_tokens for term in exact_terms):
            continue
        if strict_keywords and not any(kw in hit_tokens for kw in strict_keywords):
            continue
        filtered.append(hit)

    if filtered:
        return filtered, False
    return hits, True


def extract_exact_query_terms(query: str) -> List[str]:
    """Extract short literal terms (e.g., Korean names) that users likely expect as exact matches.

    This improves UX for queries like "홍길동 관련 문서 찾아줘" where semantic similarity may
    return unrelated documents with high embedding scores.
    """
    if not query:
        return []
    tokens = _split_tokens_local(str(query).strip())
    if not tokens:
        return []

    def _strip_suffixes(token: str) -> str:
        token = token.strip()
        if not token:
            return ""
        changed = True
        while changed:
            changed = False
            for suffix in EXACT_TERM_STRIP_SUFFIXES:
                if token.endswith(suffix) and len(token) > len(suffix):
                    token = token[: -len(suffix)]
                    changed = True
        return token.strip()

    extracted: List[str] = []
    for token in tokens:
        candidate = token.strip()
        if not candidate:
            continue
        if _EXACT_TERM_RE.fullmatch(candidate) and candidate not in EXACT_TERM_STOPWORDS:
            extracted.append(candidate)
            continue
        stripped = _strip_suffixes(candidate)
        if stripped and _EXACT_TERM_RE.fullmatch(stripped) and stripped not in EXACT_TERM_STOPWORDS:
            extracted.append(stripped)

    # Preserve order while de-duplicating.
    seen: Set[str] = set()
    ordered: List[str] = []
    for term in extracted:
        if term and term not in seen:
            seen.add(term)
            ordered.append(term)
    return ordered[:3]


# Backward compatibility aliases
_EXACT_TERM_STRIP_SUFFIXES = EXACT_TERM_STRIP_SUFFIXES
_EXACT_TERM_STOPWORDS = EXACT_TERM_STOPWORDS
_STRICT_KEYWORDS = STRICT_KEYWORDS
_STRICT_INTENT_TOKENS = STRICT_INTENT_TOKENS
_path_parts_lower = path_parts_lower
_looks_like_identifier = looks_like_identifier
_should_apply_strict_search = should_apply_strict_search
_extract_strict_keywords = extract_strict_keywords
_apply_strict_filter = apply_strict_filter
_extract_exact_query_terms = extract_exact_query_terms
