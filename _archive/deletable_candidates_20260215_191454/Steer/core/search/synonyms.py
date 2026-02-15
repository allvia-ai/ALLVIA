# synonyms.py - Extension and Domain Synonyms for Search
"""
Synonym mappings for document types, extensions, and domain keywords.
Extracted from retriever.py for modularity.
"""
from __future__ import annotations

from typing import Dict, Iterable, Set, Tuple

from .scoring import _normalize_ext

# ---------------------------------------------------------------------------
# Token splitting (local minimal version to avoid circular import)
# ---------------------------------------------------------------------------
import re
_TOKEN_PATTERN = re.compile(r"(?u)(?:[가-힣]{1,}|[A-Za-z0-9]{2,})")


def _split_tokens_local(source: str) -> list:
    if not source:
        return []
    return _TOKEN_PATTERN.findall(str(source).lower())


# ---------------------------------------------------------------------------
# Document Type Synonyms
# ---------------------------------------------------------------------------
_DOC_SYNONYMS: Set[str] = {
    "워드",
    "ms word",
    "microsoft word",
    "ms-word",
    "msword",
    "word file",
    "word document",
    "word doc",
    "워드 문서",
    "워드파일",
}

_PPT_SYNONYMS: Set[str] = {
    "ppt",
    "파워포인트",
    "파워 포인트",
    "powerpoint",
    "power point",
    "power-point",
    "presentation deck",
    "presentation file",
    "slide deck",
    "슬라이드",
}

_EXCEL_SYNONYMS: Set[str] = {
    "excel",
    "엑셀",
    "excel file",
    "excel sheet",
    "spreadsheet",
    "스프레드시트",
    "엑셀 시트",
}

_CSV_SYNONYMS: Set[str] = {
    "csv",
    "comma separated",
    "comma-separated",
    "comma separated values",
    "씨에스브이",
    "쉼표 구분",
}

EXT_SYNONYMS: Dict[str, Set[str]] = {
    ".pdf": {"pdf", "피디에프", "acrobat", "portable document", "포터블 문서"},
    ".hwp": {"hwp", "한글", "한컴", "한컴오피스", "hanword", "han word", "hangul", "hangeul"},
    ".doc": set(_DOC_SYNONYMS),
    ".docx": set(_DOC_SYNONYMS),
    ".ppt": set(_PPT_SYNONYMS),
    ".pptx": set(_PPT_SYNONYMS) | {"pptx"},
    ".xlsx": set(_EXCEL_SYNONYMS),
    ".xls": set(_EXCEL_SYNONYMS),
    ".xlsm": set(_EXCEL_SYNONYMS),
    ".xlsb": set(_EXCEL_SYNONYMS),
    ".xltx": set(_EXCEL_SYNONYMS),
    ".csv": set(_CSV_SYNONYMS),
}

DOMAIN_EXT_HINTS: Dict[str, Set[str]] = {
    "보고서": {".pdf", ".docx", ".hwp"},
    "회의록": {".docx", ".hwp", ".pdf"},
    "계약서": {".pdf", ".hwp"},
    "사업계획서": {".pdf", ".docx", ".hwp", ".pptx"},
    "사업계획": {".pdf", ".docx", ".hwp", ".pptx"},
    "이력서": {".pdf", ".docx", ".hwp"},
    "예산": {".xlsx", ".xls", ".xlsm"},
    "세금": {".xlsx", ".xls", ".pdf"},
    "레퍼런스": {".pdf", ".xlsx"},
    "참고": {".pdf", ".xlsx"},
    "초안": {".doc", ".docx", ".hwp"},
    "참고문헌": {".docx", ".xlsx", ".csv"},
    "ir": {".pdf", ".pptx"},
    "피치": {".pdf", ".pptx"},
    "피치덱": {".pdf", ".pptx"},
}

DOMAIN_KEYWORDS_BY_EXT: Dict[str, Set[str]] = {}
for keyword, exts in DOMAIN_EXT_HINTS.items():
    for ext in exts:
        norm = _normalize_ext(ext)
        if not norm:
            continue
        DOMAIN_KEYWORDS_BY_EXT.setdefault(norm, set()).add(keyword)

SEMANTIC_SYNONYMS: Dict[str, Set[str]] = {
    "보고서": {"report", "document", "summary"},
    "자료": {"material", "resource", "document"},
    "계약서": {"contract", "agreement"},
    "회의록": {"meeting", "minutes", "meeting minutes"},
    "예산": {"budget", "financial plan"},
    "발표": {"presentation", "slide", "deck"},
    "제안서": {"proposal", "pitch", "offer"},
    "사업계획서": {"business plan", "businessplan", "plan", "proposal", "pitch deck", "ir", "investment"},
    "사업계획": {"business plan", "businessplan", "plan", "strategy", "roadmap"},
    "이력서": {"resume", "cv", "curriculum vitae", "profile"},
    "경력기술서": {"resume", "cv", "work experience", "profile"},
    "계획": {"plan", "planning"},
    "정리": {"summary", "overview"},
    "ml": {"machine learning", "머신러닝", "머신 러닝", "기계학습", "기계 학습"},
    "머신러닝": {"machine learning", "ml", "기계학습", "ai"},
    "기계학습": {"machine learning", "ml", "머신러닝"},
    "ai": {"artificial intelligence", "인공지능", "머신러닝"},
    "인공지능": {"ai", "artificial intelligence", "machine learning", "머신러닝"},
}


# ---------------------------------------------------------------------------
# Keyword Form Generation
# ---------------------------------------------------------------------------
def _keyword_forms(keyword: str) -> Set[str]:
    base = str(keyword).strip().lower()
    if not base:
        return set()
    forms: Set[str] = {base}
    tokens = _split_tokens_local(base)
    if tokens:
        forms.add("".join(tokens))
        forms.add(" ".join(tokens))
        forms.update(tokens)
    return {form for form in forms if form}


# ---------------------------------------------------------------------------
# Pre-computed Keyword Maps
# ---------------------------------------------------------------------------
EXTENSION_KEYWORD_MAP: Dict[str, Set[str]] = {}
for ext, synonyms in EXT_SYNONYMS.items():
    normalized_ext = _normalize_ext(ext)
    if not normalized_ext:
        continue
    keyword_pool = set(synonyms)
    keyword_pool.add(normalized_ext)
    keyword_pool.add(normalized_ext.lstrip('.'))
    for keyword in keyword_pool:
        for form in _keyword_forms(keyword):
            bucket = EXTENSION_KEYWORD_MAP.setdefault(form, set())
            bucket.add(normalized_ext)

DOMAIN_KEYWORD_MAP: Dict[str, Set[str]] = {}
for keyword, exts in DOMAIN_EXT_HINTS.items():
    for form in _keyword_forms(keyword):
        mapped_exts = {_normalize_ext(ext) for ext in exts if _normalize_ext(ext)}
        if mapped_exts:
            DOMAIN_KEYWORD_MAP.setdefault(form, set()).update(mapped_exts)


# ---------------------------------------------------------------------------
# Lexical Keyword Hints
# ---------------------------------------------------------------------------
LEXICAL_WEIGHT = 0.35

LEXICAL_KEYWORD_HINTS_RAW: Tuple[Tuple[Set[str], float], ...] = (
    (
        {
            "법령",
            "법률",
            "법규",
            "조례",
            "규정",
            "규칙",
            "지침",
            "세칙",
            "훈령",
            "행정규칙",
        },
        0.55,
    ),
    (
        {
            "공고",
            "공문",
            "공지",
            "입찰",
            "계약",
            "발주",
            "고시",
            "제안요청서",
        },
        0.5,
    ),
    (
        {
            "이력서",
            "resume",
            "cv",
            "경력기술서",
            "포트폴리오",
            "사업계획서",
            "사업계획",
            "비즈니스플랜",
            "사업제안서",
            "투자제안서",
            "ir",
            "피치",
            "피치덱",
        },
        0.65,
    ),
)


def build_keyword_hint_forms(keywords: Iterable[str]) -> Set[str]:
    forms: Set[str] = set()
    for keyword in keywords:
        forms.update(_keyword_forms(keyword))
    return {form for form in forms if form}


LEXICAL_KEYWORD_HINTS: Tuple[Tuple[Set[str], float], ...] = tuple(
    (build_keyword_hint_forms(keywords), weight)
    for keywords, weight in LEXICAL_KEYWORD_HINTS_RAW
)


# Backward compatibility aliases
_DOC_SYNONYMS = _DOC_SYNONYMS
_PPT_SYNONYMS = _PPT_SYNONYMS
_EXCEL_SYNONYMS = _EXCEL_SYNONYMS
_CSV_SYNONYMS = _CSV_SYNONYMS
_EXT_SYNONYMS = EXT_SYNONYMS
_DOMAIN_EXT_HINTS = DOMAIN_EXT_HINTS
_DOMAIN_KEYWORDS_BY_EXT = DOMAIN_KEYWORDS_BY_EXT
_SEMANTIC_SYNONYMS = SEMANTIC_SYNONYMS
_EXTENSION_KEYWORD_MAP = EXTENSION_KEYWORD_MAP
_DOMAIN_KEYWORD_MAP = DOMAIN_KEYWORD_MAP
_LEXICAL_WEIGHT = LEXICAL_WEIGHT
_LEXICAL_KEYWORD_HINTS_RAW = LEXICAL_KEYWORD_HINTS_RAW
_LEXICAL_KEYWORD_HINTS = LEXICAL_KEYWORD_HINTS
_build_keyword_hint_forms = build_keyword_hint_forms
