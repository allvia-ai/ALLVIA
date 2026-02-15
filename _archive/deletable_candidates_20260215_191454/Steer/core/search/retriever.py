# retriever.py  (Step3: 검색기)
from __future__ import annotations

import json
import logging
import math
import os
import platform
import re
import sys
import threading
import time
import types
import unicodedata
import weakref
import importlib
import calendar
import hashlib
import copy
import tempfile
from collections import deque, OrderedDict
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from functools import lru_cache
from pathlib import Path, PurePath
from typing import (
    Any,
    Callable,
    Deque,
    Dict,
    Iterable,
    List,
    Optional,
    Sequence,
    Set,
    Tuple,
    Union,
    cast,
)

# New Utils
from core.utils.stopwords import STOPWORDS
from core.utils.nlp import split_tokens as _split_tokens_util

from core.config.paths import MODELS_DIR

# ---------------------------------------------------------------------------
# Default environment guards for macOS/CPU PyTorch compatibility.
# These avoid repeated user-side exports for shared memory warnings.
# ---------------------------------------------------------------------------
os.environ.setdefault("KMP_DUPLICATE_LIB_OK", "TRUE")
os.environ.setdefault("OMP_NUM_THREADS", "1")

try:
    import numpy as np
except Exception:  # pragma: no cover - defensive fallback for minimal envs
    class _NumpyStub:
        ndarray = list
        float32 = float
        int64 = int
        _is_stub = True

        def __getattr__(self, name: str) -> Any:
            raise ModuleNotFoundError(
                "numpy 모듈이 필요합니다. pip install numpy 로 설치 후 다시 시도해 주세요."
            )

    np = _NumpyStub()  # type: ignore[assignment]

try:
    import pandas as pd
except Exception:
    pd = None

try:
    import joblib
except Exception:
    joblib = None

try:
    from sentence_transformers import CrossEncoder, SentenceTransformer
except Exception:
    CrossEncoder = None
    SentenceTransformer = None

TORCH_META_HINT = """\
SentenceTransformer 초기화 중 torch 메타 텐서 오류가 발생했습니다.
현재 설치된 PyTorch 빌드가 SentenceTransformer와 호환되지 않습니다.

macOS/CPU 환경에서는 아래 명령으로 권장 버전을 설치한 뒤 다시 시도해 주세요.

  pip install --upgrade --no-cache-dir \\
      torch==2.3.0 torchvision==0.18.0 torchaudio==2.3.0 \\
      --index-url https://download.pytorch.org/whl/cpu

설치 후 `python infopilot.py run train` 또는 데스크톱 앱을 다시 실행하면 문제를 해결할 수 있습니다.
"""

try:
    import torch
except Exception:
    torch = None

try:
    import faiss  # noqa: F401
except ImportError:
    faiss = None
    # We will log a warning only if user explicitly tries to use FAISS features later
    # to avoid noise in pure-lexical environments.

try:
    import hnswlib  # noqa: F401
except ImportError:
    hnswlib = None
else:
    if hasattr(faiss, "omp_set_num_threads"):
        try:
            requested = os.environ.get("FAISS_OMP_NUM_THREADS") or os.environ.get("OMP_NUM_THREADS")
            threads = max(1, int(requested)) if requested else None
        except Exception:
            threads = None
        if threads is None and sys.platform == "darwin":
            threads = 1
        if threads is not None:
            try:
                faiss.omp_set_num_threads(max(1, int(threads)))
            except Exception:
                pass
# Allow forcing faiss off (e.g., macOS stability, project-local faiss.py stub).
if os.getenv("FORCE_HNSWLIB", "1") == "1":
    faiss = None

try:
    import hnswlib  # type: ignore
except Exception:
    hnswlib = None

try:
    from rank_bm25 import BM25Okapi
except Exception:
    BM25Okapi = None

from .index_manager import IndexManager

# Backward compatibility: VectorIndex extracted to vector_index.py
from .vector_index import VectorIndex, IndexPaths

# Backward compatibility: SessionState extracted to session.py
from .session import SessionState

# Backward compatibility: QueryEncoder extracted to query_encoder.py  
from .query_encoder import QueryEncoder, DEFAULT_EMBED_MODEL as _QE_DEFAULT_EMBED_MODEL

# Backward compatibility: MetadataFilters extracted to query_parser.py
from .query_parser import MetadataFilters, _apply_metadata_filters, _extract_metadata_filters

# Backward compatibility: scoring helpers extracted to scoring.py
from .scoring import (
    _compute_extension_bonus,
    _compute_owner_bonus,
    _format_human_time,
    _format_size,
    _minmax_scale,
    _normalize_ext,
    _prioritize_ext_hits,
    _similarity_to_percent,
)

# Backward compatibility: reranker extracted to reranker.py
from .reranker import CrossEncoderReranker, EarlyStopConfig, EarlyStopState

# Backward compatibility: cache classes extracted to cache.py
from .cache import CacheSignatureMonitor, QueryResultCache, SemanticQueryCache

# Backward compatibility: synonyms extracted to synonyms.py
from .synonyms import (
    EXT_SYNONYMS as _EXT_SYNONYMS,
    DOMAIN_EXT_HINTS as _DOMAIN_EXT_HINTS,
    DOMAIN_KEYWORDS_BY_EXT as _DOMAIN_KEYWORDS_BY_EXT,
    SEMANTIC_SYNONYMS as _SEMANTIC_SYNONYMS,
    EXTENSION_KEYWORD_MAP as _EXTENSION_KEYWORD_MAP,
    DOMAIN_KEYWORD_MAP as _DOMAIN_KEYWORD_MAP,
    LEXICAL_WEIGHT as _LEXICAL_WEIGHT,
    LEXICAL_KEYWORD_HINTS as _LEXICAL_KEYWORD_HINTS,
    LEXICAL_KEYWORD_HINTS_RAW as _LEXICAL_KEYWORD_HINTS_RAW,
    _keyword_forms,
    build_keyword_hint_forms as _build_keyword_hint_forms,
)

# Backward compatibility: strict_search extracted to strict_search.py
from .strict_search import (
    EXACT_TERM_STRIP_SUFFIXES as _EXACT_TERM_STRIP_SUFFIXES,
    EXACT_TERM_STOPWORDS as _EXACT_TERM_STOPWORDS,
    STRICT_KEYWORDS as _STRICT_KEYWORDS,
    STRICT_INTENT_TOKENS as _STRICT_INTENT_TOKENS,
    path_parts_lower as _path_parts_lower,
    looks_like_identifier as _looks_like_identifier,
    should_apply_strict_search as _should_apply_strict_search,
    extract_strict_keywords as _extract_strict_keywords,
    apply_strict_filter as _apply_strict_filter,
    extract_exact_query_terms as _extract_exact_query_terms,
)

MODEL_TEXT_COLUMN = "text_model"
DEFAULT_EMBED_MODEL = _QE_DEFAULT_EMBED_MODEL
MAX_BM25_TOKENS = 8000
MAX_PREVIEW_CHARS = 180
DEFAULT_MMR_LAMBDA = 0.7
RRF_DEFAULT_K = 60
TEMPORAL_HALF_LIFE_DAYS = 365.0
TEMPORAL_WEIGHT_FLOOR = 0.72
TEMPORAL_WEIGHT_CEILING = 1.15
_SESSION_HISTORY_LIMIT = 50
_SESSION_CHAT_HISTORY_LIMIT = 20
_SESSION_PREF_DECAY = 0.85
_SESSION_CLICK_WEIGHT = 0.35
_SESSION_PIN_WEIGHT = 0.6
_SESSION_LIKE_WEIGHT = 0.45
_SESSION_DISLIKE_WEIGHT = -0.5
_META_SPLIT_RE = re.compile(r"[^0-9A-Za-z가-힣]+")
logger = logging.getLogger(__name__)
if not logger.handlers:
    logger.addHandler(logging.NullHandler())


def _split_tokens(source: Any) -> List[str]:
    if not source:
        return []
    return _split_tokens_util(str(source))


def _temporal_weight(mtime: Any, ctime: Any = None) -> float:
    timestamp = None
    for candidate in (mtime, ctime):
        if candidate:
            try:
                ts = float(candidate)
            except (TypeError, ValueError):
                continue
            if ts > 0:
                timestamp = ts
                break
    if timestamp is None:
        return 1.0
    now = time.time()
    age_days = max(0.0, (now - timestamp) / 86400.0)
    decay = math.exp(-age_days / TEMPORAL_HALF_LIFE_DAYS)
    weight = TEMPORAL_WEIGHT_FLOOR + (TEMPORAL_WEIGHT_CEILING - TEMPORAL_WEIGHT_FLOOR) * decay
    return max(TEMPORAL_WEIGHT_FLOOR, min(TEMPORAL_WEIGHT_CEILING, weight))



def _clamp(val: float, min_val: float, max_val: float) -> float:
    return max(min_val, min(val, max_val))


def _mask_path(path: str) -> str:
    if not path:
        return ""
    try:
        return Path(path).name
    except Exception:
        return "<invalid-path>"


def _looks_like_extension(token: str) -> bool:
    if not token:
        return False
    if token.startswith('.'):
        return True
    if len(token) > 5:
        return False
    ascii_token = token.replace('-', '')
    return ascii_token.isascii() and ascii_token.isalnum()


def _rescale_inner_product(value: float) -> float:
    if value is None:
        return 0.0
    try:
        if math.isnan(value):
            return 0.0
    except TypeError:
        return 0.0
    scaled = 0.5 * (float(value) + 1.0)
    if scaled < 0.0:
        return 0.0
    if scaled > 1.0:
        return 1.0
    return scaled


def _looks_like_identifier(token: str) -> bool:
    if not token:
        return False
    token = token.strip()
    if not token:
        return False
    if re.search(r"\d{3,}", token):
        return True
    if re.search(r"[A-Za-z]{2,}\d{2,}", token):
        return True
    if "-" in token or "_" in token:
        digits = sum(ch.isdigit() for ch in token)
        letters = sum(ch.isalpha() for ch in token)
        if digits >= 2 and (letters >= 1 or "-" in token):
            return True
    return False


_NEGATIVE_TEMPLATE_HINTS: Tuple[Tuple[str, float], ...] = (
    ("목차", 0.35),
    ("차례", 0.3),
    ("table of contents", 0.35),
    ("contents page", 0.3),
    ("cover page", 0.25),
    ("표지", 0.25),
    ("copyright", 0.2),
    ("signature block", 0.2),
)


def _negative_template_penalty(hit: Dict[str, Any]) -> Tuple[float, List[str]]:
    preview = str(hit.get("preview") or "").lower()
    path = str(hit.get("path") or "").lower()
    matched: List[str] = []
    penalty = 0.0
    for keyword, weight in _NEGATIVE_TEMPLATE_HINTS:
        key_lower = keyword.lower()
        if key_lower in preview or key_lower in path:
            matched.append(keyword)
            penalty = max(penalty, weight)
    return penalty, matched


def _iter_query_units(lowered: str) -> Set[str]:
    tokens = _split_tokens(lowered)
    units: Set[str] = set(tokens)
    units.add(lowered)
    for token in tokens:
        for segment in re.findall(r"[a-z0-9]{1,5}", token):
            if segment:
                units.add(segment)
    length = len(tokens)
    for n in (2, 3):
        if length < n:
            continue
        for i in range(length - n + 1):
            segment = tokens[i:i + n]
            units.add(" ".join(segment))
            units.add("".join(segment))
    return units


def _token_contains(unit: str, keyword: str) -> bool:
    if not unit or not keyword:
        return False
    unit_clean = str(unit).strip().lower()
    keyword_clean = str(keyword).strip().lower()
    if not unit_clean or not keyword_clean:
        return False
    if unit_clean == keyword_clean:
        return True
    unit_tokens = set(_split_tokens(unit_clean))
    if keyword_clean in unit_tokens:
        return True
    keyword_tokens = _split_tokens(keyword_clean)
    if keyword_tokens and all(tok in unit_tokens for tok in keyword_tokens):
        return True
    return False


def _extract_query_exts(query: str, *, available_exts: Set[str]) -> Set[str]:
    if not query or not available_exts:
        return set()
    lowered = query.lower()
    units = _iter_query_units(lowered)
    requested: Set[str] = set()

    for unit in units:
        normalized = _normalize_ext(unit)
        if normalized and normalized in available_exts:
            requested.add(normalized)

    if requested:
        return requested

    for unit in units:
        mapped = _EXTENSION_KEYWORD_MAP.get(unit)
        if mapped:
            requested.update(mapped & available_exts)
    if requested:
        return requested

    for unit in units:
        for ext, keywords in _EXT_SYNONYMS.items():
            norm_ext = _normalize_ext(ext)
            if norm_ext not in available_exts:
                continue
            if any(_token_contains(unit, keyword) for keyword in keywords):
                requested.add(norm_ext)
    if requested:
        return requested

    for unit in units:
        mapped = _DOMAIN_KEYWORD_MAP.get(unit)
        if mapped:
            requested.update(mapped & available_exts)

    if requested:
        return requested

    for keyword, hinted_exts in _DOMAIN_EXT_HINTS.items():
        if any(_token_contains(unit, keyword) for unit in units):
            for ext in hinted_exts:
                norm = _normalize_ext(ext)
                if norm in available_exts:
                    requested.add(norm)
    return requested


def _dynamic_oversample(
    top_k: int,
    *,
    has_ext_pref: bool,
    filters_active: bool,
    corpus_size: int,
) -> int:
    capped_top_k = max(1, int(top_k))
    base = 2
    if has_ext_pref:
        base += 2
    if filters_active:
        base = max(base, 6)
    if capped_top_k <= 3:
        base = max(base, 4)

    # hard upper bound so that oversample * top_k stays reasonably small
    upper_by_limit = max(1, min(10, 200 // capped_top_k))

    if corpus_size > 0:
        max_batches = max(1, (corpus_size + capped_top_k - 1) // capped_top_k)
        upper_by_limit = min(upper_by_limit, max_batches)

    oversample = min(base, upper_by_limit)
    return max(1, oversample)


def _classify_query(
    query: str,
    *,
    metadata_filters: "MetadataFilters",
    requested_exts: Set[str],
) -> str:
    token_count = _token_count_lower(query)
    if metadata_filters.is_active() or requested_exts:
        return "narrow"
    if token_count <= 3:
        return "narrow"
    if token_count >= 12:
        return "broad"
    return "broad"


def _dynamic_search_params(
    query: str,
    top_k: int,
    *,
    metadata_filters: "MetadataFilters",
    requested_exts: Set[str],
) -> Dict[str, int]:
    classification = _classify_query(
        query,
        metadata_filters=metadata_filters,
        requested_exts=requested_exts,
    )
    base_top_k = max(1, int(top_k))
    token_count = _token_count_lower(query)
    if classification == "narrow":
        oversample = min(6, max(2, base_top_k))
        rerank_depth = max(base_top_k * 2, 30 + (token_count * 2))
        fusion_depth = max(base_top_k * 2, base_top_k + 10)
    else:
        oversample = min(12, max(3, base_top_k * 2))
        rerank_depth = max(base_top_k * 3, 80 + (token_count * 3))
        fusion_depth = max(base_top_k * 3, base_top_k + 25)
    return {
        "oversample": max(1, oversample),
        "rerank_depth": max(base_top_k, rerank_depth),
        "fusion_depth": max(base_top_k, fusion_depth),
    }


def _should_expand_query(
    query: str,
    *,
    metadata_filters: "MetadataFilters",
    requested_exts: Set[str],
) -> bool:
    tokens = _split_tokens((query or "").lower())
    if len(tokens) > 6:
        return False
    if metadata_filters.is_active():
        return False
    if any(_looks_like_extension(tok) for tok in tokens):
        return False
    lowered = (query or "").lower()
    if any(keyword in lowered for keyword in ("확장자", "extension", "file type")):
        return False
    return True


def _rrf(rank_lists: Sequence[List[Dict[str, Any]]], *, k: int = RRF_DEFAULT_K) -> List[Dict[str, Any]]:
    if not rank_lists:
        return []
    cumulative: Dict[Tuple[Any, Any], float] = {}
    keeper: Dict[Tuple[Any, Any], Dict[str, Any]] = {}
    for rlist in rank_lists:
        for rank, hit in enumerate(rlist, start=1):
            key = (hit.get("doc_id"), hit.get("path"))
            if key not in keeper:
                keeper[key] = hit
            cumulative[key] = cumulative.get(key, 0.0) + 1.0 / (k + rank)
    ordered = sorted(cumulative.items(), key=lambda item: item[1], reverse=True)
    return [keeper[key] for key, _ in ordered]


def _mmr(
    index: "VectorIndex",
    candidates: List[Dict[str, Any]],
    qvec: Optional[np.ndarray],
    top_k: int,
    *,
    lambda_: float = DEFAULT_MMR_LAMBDA,
) -> List[Dict[str, Any]]:
    if not candidates or top_k <= 0:
        return []

    if qvec is None:
        return candidates[:top_k]

    valid_hits: List[Dict[str, Any]] = []
    doc_vectors: List[np.ndarray] = []
    for hit in candidates:
        doc_id = hit.get("doc_id")
        if doc_id is None:
            continue
        vec = index.embeddings.get(int(doc_id)) if hasattr(index, "embeddings") else None
        if vec is None:
            continue
        valid_hits.append(hit)
        doc_vectors.append(vec)

    if not valid_hits:
        return candidates[:top_k]

    D = np.vstack(doc_vectors).astype(np.float32, copy=False)
    q = VectorIndex._normalize_vector(np.asarray(qvec, dtype=np.float32))
    # NOTE: On some macOS Python builds (Accelerate + NumPy 2.0.x),
    # `matmul` can emit spurious RuntimeWarnings; use `np.dot` for stability.
    sim_to_query = np.dot(D, q)

    selected_indices: List[int] = []
    chosen_hits: List[Dict[str, Any]] = []
    remaining = set(range(len(valid_hits)))

    while remaining and len(chosen_hits) < min(top_k, len(valid_hits)):
        if not selected_indices:
            best = int(max(remaining, key=lambda idx: sim_to_query[idx]))
            selected_indices.append(best)
            remaining.discard(best)
            chosen_hits.append(valid_hits[best])
            continue

        selected_matrix = D[selected_indices]
        inter = np.dot(D, selected_matrix.T)
        max_inter = inter.max(axis=1)
        mmr_scores = lambda_ * sim_to_query - (1.0 - lambda_) * max_inter
        for idx in selected_indices:
            mmr_scores[idx] = -1e9
        best = int(max(remaining, key=lambda idx: mmr_scores[idx]))
        selected_indices.append(best)
        remaining.discard(best)
        chosen_hits.append(valid_hits[best])

    if len(chosen_hits) < top_k:
        # fill with remaining candidates preserving order
        seen = {id(hit) for hit in chosen_hits}
        for hit in valid_hits:
            if len(chosen_hits) >= top_k:
                break
            if id(hit) in seen:
                continue
            chosen_hits.append(hit)

    return chosen_hits[:top_k]


def _summarize_metadata_filters(filters: MetadataFilters) -> List[str]:
    if not filters.is_active():
        return []
    summary: List[str] = []
    if filters.mtime_from is not None or filters.mtime_to is not None:
        summary.append("수정일 조건 일치")
    if filters.ctime_from is not None or filters.ctime_to is not None:
        summary.append("생성일 조건 일치")
    if filters.size_min is not None or filters.size_max is not None:
        summary.append("파일 크기 조건 일치")
    if filters.owners:
        owners = ", ".join(sorted(filters.owners))
        summary.append(f"작성자 조건: {owners}")
    return summary


def _annotate_hits(
    hits: List[Dict[str, Any]],
    *,
    desired_exts: Set[str],
    raw_query_tokens: Set[str],
    expanded_query_tokens: Set[str],
    metadata_filters: MetadataFilters,
    lexical_weight: float,
) -> List[Dict[str, Any]]:
    synonym_tokens = {tok for tok in expanded_query_tokens if tok not in raw_query_tokens}
    metadata_summary = _summarize_metadata_filters(metadata_filters)

    def _safe_float(value: Any) -> Optional[float]:
        if value is None:
            return None
        try:
            return float(value)
        except (TypeError, ValueError):
            return None

    def _unique(items: Iterable[str]) -> List[str]:
        seen: Set[str] = set()
        ordered: List[str] = []
        for item in items:
            if item and item not in seen:
                seen.add(item)
                ordered.append(item)
        return ordered

    for hit in hits:
        hit_tokens = _collect_hit_tokens(hit)

        matched_terms = sorted(tok for tok in raw_query_tokens if tok in hit_tokens)
        synonym_matches = sorted(tok for tok in synonym_tokens if tok in hit_tokens)

        breakdown: Dict[str, float] = {}
        reasons: List[str] = []

        vector_score = _safe_float(hit.get("vector_similarity"))
        if vector_score is not None:
            breakdown["vector"] = round(vector_score, 4)
            reasons.append(f"임베딩 유사도 {breakdown['vector']:.2f}")

        lexical_score = _safe_float(hit.get("lexical_score"))
        if lexical_score is not None:
            breakdown["lexical"] = round(lexical_score, 4)
            if lexical_weight > 0.0 and lexical_score > 0.0:
                reasons.append(
                    f"키워드 일치 점수 {breakdown['lexical']:.2f} (가중치 {lexical_weight:.2f})"
                )
            elif lexical_score > 0.0:
                reasons.append(f"키워드 일치 점수 {breakdown['lexical']:.2f}")

        temporal_weight = _safe_float(hit.get("temporal_weight"))
        if temporal_weight is not None and abs(temporal_weight - 1.0) > 0.01:
            breakdown["recency"] = round(temporal_weight, 4)
            if temporal_weight > 1.0:
                reasons.append(f"최신 문서 가중치 {temporal_weight:.2f}")
            else:
                reasons.append(f"오래된 문서 가중치 {temporal_weight:.2f}")

        rerank_score = _safe_float(hit.get("rerank_score"))
        if rerank_score is not None:
            breakdown["rerank"] = round(rerank_score, 4)
            reasons.append(f"Cross-Encoder 점수 {breakdown['rerank']:.2f}")

        negative_penalty = _safe_float(hit.get("negative_penalty"))
        if negative_penalty is not None and negative_penalty > 0.0:
            breakdown["negative_penalty"] = round(negative_penalty, 4)
            negative_labels = hit.get("negative_reasons", [])
            if isinstance(negative_labels, list) and negative_labels:
                label_str = ", ".join(str(lbl) for lbl in negative_labels[:3])
                reasons.append(f"서식/목차 패널티 -{negative_penalty:.2f} ({label_str})")
                hit["negative_matches"] = list(negative_labels)
            else:
                reasons.append(f"서식/목차 패널티 -{negative_penalty:.2f}")

        total_score = _safe_float(hit.get("combined_score", hit.get("score")))
        if total_score is not None:
            breakdown["final"] = round(total_score, 4)

        ext = _normalize_ext(hit.get("ext"))
        desired_ext_bonus = _safe_float(hit.get("desired_extension_bonus")) or 0.0
        session_ext_bonus = _safe_float(hit.get("session_ext_bonus")) or 0.0
        session_owner_bonus = _safe_float(hit.get("session_owner_bonus")) or 0.0

        breakdown["extension_bonus"] = round(desired_ext_bonus, 4) if desired_ext_bonus else 0.0
        if desired_ext_bonus > 0 and ext:
            reasons.append(f"요청 확장자 {ext} 우선")
        if session_ext_bonus:
            breakdown["session_ext"] = round(session_ext_bonus, 4)
            if session_ext_bonus > 0:
                reasons.append("세션 선호 확장자 가중치")
            else:
                reasons.append("세션 비선호 확장자 페널티")
        if session_owner_bonus:
            breakdown["session_owner"] = round(session_owner_bonus, 4)
            if session_owner_bonus > 0:
                reasons.append("세션 선호 작성자 가중치")
            else:
                reasons.append("세션 비선호 작성자 페널티")

        meeting_bonus = _safe_float(hit.get("meeting_artifact_bonus"))
        if meeting_bonus is not None and meeting_bonus > 0.0:
            breakdown["meeting_bonus"] = round(meeting_bonus, 4)
            reasons.append("회의 산출물(요약/전사) 우선")

        metadata_reasons = metadata_summary if metadata_summary else []

        if matched_terms:
            snippet = ", ".join(matched_terms[:4])
            reasons.append(f"질문 키워드 일치: {snippet}")
        if synonym_matches:
            snippet = ", ".join(synonym_matches[:4])
            reasons.append(f"확장/동의어 매칭: {snippet}")
        reasons.extend(metadata_reasons)

        exact_terms = hit.get("exact_terms_matched")
        if isinstance(exact_terms, list) and exact_terms:
            snippet = ", ".join(str(t) for t in exact_terms[:3] if t)
            if snippet:
                reasons.append(f"정확 일치: {snippet}")

        filename_terms = hit.get("filename_terms_matched")
        if isinstance(filename_terms, list) and filename_terms:
            snippet = ", ".join(str(t) for t in filename_terms[:3] if t)
            if snippet:
                reasons.append(f"파일명 일치: {snippet}")

        hit["score_breakdown"] = breakdown
        hit["matched_terms"] = matched_terms
        hit["matched_synonyms"] = synonym_matches
        hit["metadata_matches"] = list(metadata_reasons)
        hit["match_reasons"] = _unique(reasons)

    return hits


@lru_cache(maxsize=256)
def _extension_related_tokens_cached(ext: str) -> Tuple[str, ...]:
    normalized = _normalize_ext(ext)
    if not normalized:
        return ()
    related: Set[str] = set()
    base = normalized.lstrip('.')
    if base:
        related.add(base)
    related.add(normalized)
    for keyword in _EXT_SYNONYMS.get(normalized, set()):
        related.update(_keyword_forms(keyword))
    return tuple(sorted({tok for tok in related if tok}))


def _extension_related_tokens(ext: str) -> Set[str]:
    return set(_extension_related_tokens_cached(ext))


def _expand_query_text(query: str) -> str:
    lowered = query.lower()
    tokens = {tok for tok in _split_tokens(lowered) if tok}
    expansions: Set[str] = set()

    for token in tokens:
        ext_token = None
        if _looks_like_extension(token):
            ext_token = _normalize_ext(token)
        if ext_token:
            expansions.update(_extension_related_tokens(ext_token))
            continue

        mapped_exts = _EXTENSION_KEYWORD_MAP.get(token)
        if mapped_exts:
            for ext in mapped_exts:
                expansions.update(_extension_related_tokens(ext))
            continue

        if token in _SEMANTIC_SYNONYMS:
            expansions.update(_SEMANTIC_SYNONYMS[token])

    for keyword, hinted_exts in _DOMAIN_EXT_HINTS.items():
        if keyword in lowered or keyword in tokens:
            expansions.add(keyword)
            for ext in hinted_exts:
                expansions.update(_extension_related_tokens(ext))

    cleaned_expansions = [word for word in expansions if word and word not in tokens]
    if not cleaned_expansions:
        return query
    return f"{query} {' '.join(sorted(set(cleaned_expansions)))}"


def _time_tokens(epoch: Optional[float]) -> List[str]:
    if not epoch:
        return []
    try:
        dt = datetime.fromtimestamp(float(epoch))
    except Exception:
        return []
    return [
        dt.strftime("%Y"),
        dt.strftime("%Y-%m"),
        dt.strftime("%Y-%m-%d"),
        dt.strftime("%B"),
        dt.strftime("%m"),
    ]


def _size_bucket(size: Optional[int]) -> Optional[str]:
    if size is None:
        return None
    try:
        size = int(size)
    except (TypeError, ValueError):
        return None
    if size <= 0:
        return None
    if size < 10 * 1024:
        return "size:tiny"
    if size < 1 * 1024 * 1024:
        return "size:small"
    if size < 10 * 1024 * 1024:
        return "size:medium"
    if size < 50 * 1024 * 1024:
        return "size:large"
    return "size:huge"


def _clean_token(token: str) -> str:
    if not token:
        return ""
    cleaned = re.sub(r"\s+", " ", str(token)).strip().lower()
    return cleaned


def _metadata_text(
    path: str,
    ext: str,
    drive: str,
    *,
    size: Optional[int] = None,
    mtime: Optional[float] = None,
    ctime: Optional[float] = None,
    owner: Optional[str] = None,
) -> str:
    tokens: List[str] = []
    if path:
        try:
            p = Path(path)
        except Exception:
            p = None
        if p:
            name = p.name
            if name:
                tokens.append(name)
            stem = p.stem
            if stem and stem != name:
                tokens.append(stem)
            tokens.extend(_split_tokens(stem))
            parent_name = p.parent.name if p.parent else ""
            if parent_name:
                tokens.append(parent_name)
                tokens.extend(_split_tokens(parent_name))
        else:
            tokens.append(str(path))
    if ext:
        ext_clean = str(ext).strip()
        if ext_clean:
            tokens.append(ext_clean)
            ext_no_dot = ext_clean.lstrip(".")
            if ext_no_dot:
                tokens.append(ext_no_dot)
            tokens.extend(_extension_related_tokens(ext_clean))
    if drive:
        drive_str = str(drive)
        tokens.append(drive_str)
        tokens.extend(_split_tokens(drive_str))
    for epoch in (mtime, ctime):
        tokens.extend(_time_tokens(epoch))
    bucket = _size_bucket(size)
    if bucket:
        tokens.append(bucket)
    if owner:
        tokens.append(str(owner))
        tokens.extend(_split_tokens(str(owner)))

    seen: Set[str] = set()
    normalized: List[str] = []
    for token in tokens:
        cleaned = _clean_token(token)
        if not cleaned:
            continue
        if cleaned not in seen:
            seen.add(cleaned)
            normalized.append(cleaned)
    return " ".join(normalized)


def _compose_model_text(base_text: str, metadata: str) -> str:
    base_text = base_text or ""
    metadata = metadata or ""
    if metadata and base_text:
        return f"{base_text}\n\n{metadata}"
    if metadata:
        return metadata
    return base_text


def _ensure_unique_paths(
    selected: List[Dict[str, Any]],
    pool: List[Dict[str, Any]],
    top_k: int,
) -> List[Dict[str, Any]]:
    """Ensure top-k results contain unique `path` entries, filling from pool when needed."""
    if not selected or top_k <= 0:
        return selected[: max(top_k, 0)]
    seen: Set[str] = set()
    unique: List[Dict[str, Any]] = []
    for hit in list(selected) + list(pool):
        path = str(hit.get("path") or "")
        if not path or path in seen:
            continue
        seen.add(path)
        unique.append(hit)
        if len(unique) >= top_k:
            break
    return unique


def _collect_hit_tokens(hit: Dict[str, Any]) -> Set[str]:
    tokens: Set[str] = set()
    path = hit.get("path")
    if path:
        path_text = str(path).lower()
        tokens.add(path_text)
        tokens.update(_split_tokens(path_text))
    preview = hit.get("preview")
    if preview:
        preview_text = str(preview).lower()
        tokens.add(preview_text)
        tokens.update(_split_tokens(preview_text))
    tokens.update(_extension_related_tokens(hit.get("ext", "")))
    return {tok for tok in tokens if tok}


def _lexical_overlap_score(query_tokens: Set[str], hit_tokens: Set[str]) -> float:
    if not query_tokens or not hit_tokens:
        return 0.0
    total = 0.0
    for token in query_tokens:
        if token in hit_tokens:
            total += 1.0
            continue
        for candidate in hit_tokens:
            if token in candidate or candidate in token:
                total += 0.5
                break
    return total / max(len(query_tokens), 1)


def _rerank_hits(
    raw_query: str,
    expanded_query: str,
    hits: List[Dict[str, Any]],
    *,
    desired_exts: Set[str],
    top_k: int,
    session: Optional[SessionState] = None,
) -> List[Dict[str, Any]]:
    if not hits:
        return []

    query_lower = (raw_query or "").strip().lower()
    meeting_intent = False

    base_tokens = {tok for tok in _split_tokens(raw_query.lower()) if tok}
    expanded_tokens = {tok for tok in _split_tokens(expanded_query.lower()) if tok}
    query_tokens = base_tokens or expanded_tokens
    synonym_tokens = expanded_tokens - base_tokens
    exact_terms = _extract_exact_query_terms(raw_query)
    exact_bonus_env = os.getenv("INFOPILOT_EXACT_MATCH_BONUS", "").strip()
    exact_bonus = 0.75
    if exact_bonus_env:
        try:
            exact_bonus = float(exact_bonus_env)
        except ValueError:
            pass
    filename_bonus_env = os.getenv("INFOPILOT_FILENAME_MATCH_BONUS", "").strip()
    filename_bonus = 0.25
    if filename_bonus_env:
        try:
            filename_bonus = float(filename_bonus_env)
        except ValueError:
            pass
    scored_hits: List[Dict[str, Any]] = []
    use_lexical_overlap = _LEXICAL_WEIGHT > 0.0

    for raw_hit in hits:
        hit = dict(raw_hit)
        hit_tokens = _collect_hit_tokens(hit)
        lexical_score = 0.0
        if use_lexical_overlap:
            existing_lexical = hit.get("lexical_score")
            if existing_lexical is None:
                lexical_score = _lexical_overlap_score(query_tokens, hit_tokens)
                if synonym_tokens:
                    synonym_score = _lexical_overlap_score(synonym_tokens, hit_tokens)
                    lexical_score = max(lexical_score, synonym_score)
                hit["lexical_score"] = lexical_score
            else:
                try:
                    lexical_score = float(existing_lexical)
                except (TypeError, ValueError):
                    lexical_score = 0.0
        else:
            hit.setdefault("lexical_score", 0.0)

        base_similarity = float(hit.get("vector_similarity", hit.get("similarity", 0.0)))
        if "vector_similarity" not in hit:
            hit["vector_similarity"] = base_similarity

        base_score = hit.get("score")
        if base_score is None:
            base_score = base_similarity
            if use_lexical_overlap:
                base_score += float(lexical_score) * _LEXICAL_WEIGHT

        negative_penalty, negative_matches = _negative_template_penalty(hit)
        if negative_penalty > 0.0:
            hit["negative_penalty"] = float(negative_penalty)
            if negative_matches:
                hit["negative_reasons"] = negative_matches
            base_score = max(0.0, float(base_score) - negative_penalty)
        else:
            hit["negative_penalty"] = 0.0

        ext_raw = hit.get("ext")
        ext_norm = _normalize_ext(ext_raw)
        total_ext_bonus, desired_ext_bonus, session_ext_bonus = _compute_extension_bonus(
            ext_norm,
            desired_exts,
            session,
        )
        owner_bonus = _compute_owner_bonus(hit.get("owner"), session)
        temporal_factor = _temporal_weight(hit.get("mtime"), hit.get("ctime"))
        matched_exact_terms: List[str] = []
        if exact_terms:
            matched_exact_terms = [term for term in exact_terms if term in hit_tokens]
        final_score = (float(base_score) * temporal_factor) + total_ext_bonus + owner_bonus
        if matched_exact_terms and exact_bonus:
            final_score += float(exact_bonus)
            hit["exact_terms_matched"] = matched_exact_terms
            path = str(hit.get("path") or "")
            if path:
                try:
                    filename = Path(path).name.lower()
                except Exception:
                    filename = ""
                filename_terms_matched = [term for term in matched_exact_terms if term.lower() in filename] if filename else []
                if filename_terms_matched and filename_bonus:
                    final_score += float(filename_bonus)
                    hit["filename_terms_matched"] = filename_terms_matched

        meeting_bonus = 0.0
        if meeting_intent:
            path = str(hit.get("path") or "")
            if _is_user_facing_meeting_artifact(path):
                parts = _path_parts_lower(path)
                if parts:
                    meeting_bonus = float(_MEETING_ARTIFACT_SCORE_BONUS.get(parts[-1], 0.0))
                    if meeting_bonus:
                        final_score += meeting_bonus
                        hit["meeting_artifact_bonus"] = meeting_bonus
        hit["score"] = final_score
        hit["temporal_weight"] = temporal_factor
        if "vector_similarity" in hit:
            hit["similarity"] = float(hit.get("vector_similarity", 0.0))
        else:
            hit["similarity"] = final_score
        hit["desired_extension_bonus"] = float(desired_ext_bonus)
        hit["session_ext_bonus"] = float(session_ext_bonus)
        hit["session_owner_bonus"] = float(owner_bonus)
        scored_hits.append(hit)

    scored_hits.sort(key=lambda item: item.get("score", item.get("similarity", 0.0)), reverse=True)
    return _prioritize_ext_hits(scored_hits, desired_exts=desired_exts, top_k=top_k)


try:
    import pandas as pd  # noqa: F811 - re-import for static analyzers
except Exception:  # pragma: no cover - already handled
    pd = None

PARQUET_ENGINE: Optional[str] = None
if pd is not None:
    for candidate in ("fastparquet", "pyarrow"):
        try:
            importlib.import_module(candidate)
            PARQUET_ENGINE = candidate
            break
        except ImportError:
            continue


try:
    import joblib  # noqa: F811 - re-import for static analyzers
except Exception:  # pragma: no cover
    joblib = None

def _pick_rerank_device(requested: Optional[str]) -> str:
    if requested:
        return str(requested)
    if torch is not None:
        try:
            if torch.cuda.is_available():  # type: ignore[attr-defined]
                return "cuda"
        except Exception:
            pass
    return "cpu"



def _token_count_lower(query: str) -> int:
    return len(_split_tokens((query or "").lower()))


class Retriever:
    def __init__(
        self,
        model_path: Path,
        corpus_path: Path,
        cache_dir: Path = Path("./index_cache"),
        *,
        search_wait_timeout: float = 0.5,
        use_rerank: bool = True,
        rerank_model: str = "BAAI/bge-reranker-large",
        rerank_depth: int = 80,
        rerank_batch_size: int = 16,
        rerank_device: Optional[str] = None,
        rerank_min_score: Optional[float] = 0.35,
        lexical_weight: float = 0.0,
        min_similarity: float = 0.35,
        strict_search: bool = False,
        auto_refresh: bool = True,
        refresh_interval: float = 1.5,
        refresh_stability_checks: int = 2,
        result_cache_size: int = 128,
        semantic_cache_size: int = 64,
        semantic_cache_threshold: float = 0.97,
    ):
        self.model_path = Path(model_path)
        self.corpus_path = Path(corpus_path)
        self.cache_dir = Path(cache_dir)
        self.encoder = QueryEncoder(self.model_path)
        self.search_wait_timeout = search_wait_timeout
        depth = max(0, int(rerank_depth))
        self.rerank_depth = depth
        self.rerank_model = rerank_model
        self.rerank_batch_size = max(1, int(rerank_batch_size) if rerank_batch_size else 1)
        self.rerank_device = rerank_device or None
        self.use_rerank = bool(use_rerank and depth > 0)
        base_weight = max(0.0, min(1.0, float(lexical_weight)))
        self.base_lexical_weight = base_weight
        self.lexical_weight = base_weight
        self.min_similarity = max(0.0, min(1.0, float(min_similarity)))
        self.strict_search = bool(strict_search)
        try:
            self.rerank_min_score = float(rerank_min_score) if rerank_min_score is not None else None
        except (TypeError, ValueError):
            self.rerank_min_score = None
        self._reranker: Optional[CrossEncoderReranker] = None
        self.index_manager = IndexManager(
            loader=self._load_cached_index,
            builder=self._rebuild_index,
        )

        self._cache_signature: Optional[Tuple[float, float, float, float]] = self._compute_cache_signature()
        self._cache_monitor: Optional[CacheSignatureMonitor] = None
        self._auto_refresh = bool(auto_refresh)
        self._refresh_interval = max(0.1, float(refresh_interval)) if refresh_interval else 0.0
        self._refresh_stability_checks = max(1, int(refresh_stability_checks))

        self._result_cache = QueryResultCache(result_cache_size)
        self._semantic_cache = None
        if semantic_cache_size > 0 and semantic_cache_threshold > 0.0:
            self._semantic_cache = SemanticQueryCache(semantic_cache_size, semantic_cache_threshold)
        self._semantic_cache_initial_threshold = semantic_cache_threshold
        self._cache_stats = {
            "result_hits": 0,
            "result_misses": 0,
            "semantic_hits": 0,
            "semantic_misses": 0,
        }

        if self._auto_refresh and self._refresh_interval > 0.0:
            self._cache_monitor = CacheSignatureMonitor(
                self._compute_cache_signature,
                self._on_cache_signature_change,
                interval=self._refresh_interval,
                stability_checks=self._refresh_stability_checks,
            )
            if self._cache_signature is not None:
                self._cache_monitor.prime(self._cache_signature)
            self._cache_monitor.start()

        self._finalizer = weakref.finalize(self, self._shutdown_background_tasks)

    def ready(self, rebuild: bool = False, *, wait: bool = True) -> bool:
        if rebuild:
            self.index_manager.schedule_rebuild(priority=True)
        else:
            self.index_manager.ensure_loaded()
        if wait:
            self.index_manager.wait_until_ready()
        return self.index_manager.get_index(wait=False) is not None

    def wait_until_ready(self, timeout: Optional[float] = None) -> bool:
        return self.index_manager.wait_until_ready(timeout=timeout)

    def shutdown(self) -> None:
        finalizer = getattr(self, "_finalizer", None)
        if finalizer is not None and getattr(finalizer, "alive", False):
            finalizer()

    def _shutdown_background_tasks(self) -> None:
        monitor = getattr(self, "_cache_monitor", None)
        if monitor is not None:
            try:
                monitor.stop()
            except Exception:
                pass
        if hasattr(self, "index_manager") and self.index_manager is not None:
            try:
                self.index_manager.shutdown()
            except Exception:
                pass

    def _ensure_index(self) -> Optional[VectorIndex]:
        self._refresh_if_cache_changed()
        index = self.index_manager.get_index(wait=False)
        if index is not None:
            return index
        self.index_manager.ensure_loaded()
        if not self.index_manager.wait_until_ready(timeout=self.search_wait_timeout):
            return None
        return self.index_manager.get_index(wait=False)

    def search(
        self,
        query: str,
        top_k: int = 5,
        *,
        session: Optional[SessionState] = None,
        use_ann: Optional[bool] = None,
    ) -> List[Dict[str, Any]]:
        cache_key: Optional[Tuple[str, int, bool, float]] = None
        result_cache = getattr(self, "_result_cache", None)
        if session is None and result_cache is not None:
            cache_key = self._make_cache_key(query, top_k)
            cached = result_cache.get(cache_key)
            if cached is not None:
                self._record_cache_event("result", hit=True)
                return cached

        index = self._ensure_index()
        if index is None:
            return []

        if session is not None:
            session.add_query(query)

        available_exts: Set[str] = set()
        for ext in getattr(index, "exts", []):
            normalized = _normalize_ext(ext)
            if normalized:
                available_exts.add(normalized)
        requested_exts = _extract_query_exts(query, available_exts=available_exts)
        metadata_filters = _extract_metadata_filters(query)
        corpus_size = len(getattr(index, "doc_ids", [])) if hasattr(index, "doc_ids") else 0

        search_params = _dynamic_search_params(
            query,
            top_k,
            metadata_filters=metadata_filters,
            requested_exts=requested_exts,
        )

        oversample = max(
            search_params["oversample"],
            _dynamic_oversample(
                top_k,
                has_ext_pref=bool(requested_exts),
                filters_active=metadata_filters.is_active(),
                corpus_size=corpus_size,
            ),
        )

        should_expand = _should_expand_query(
            query,
            metadata_filters=metadata_filters,
            requested_exts=requested_exts,
        )
        vector_query = _expand_query_text(query) if should_expand else query
        query_lower = (query or "").lower()
        meeting_intent = False
        raw_query_tokens_set: Set[str] = {tok for tok in _split_tokens(query_lower) if tok}
        expanded_query_tokens_set: Set[str] = {
            tok for tok in _split_tokens(vector_query.lower()) if tok
        }
        q = self.encoder.encode_query(vector_query)
        q_vector: Optional[np.ndarray]
        try:
            q_array = np.asarray(q, dtype=np.float32)
            if q_array.ndim == 0:
                q_vector = q_array.reshape(1)
            else:
                q_vector = q_array.reshape(-1)
        except Exception:
            q_vector = None
        query_tokens: Optional[List[str]] = None
        if getattr(self, "base_lexical_weight", 0.0) > 0.0:
            query_tokens = list(expanded_query_tokens_set)
        configured_rerank_depth = int(getattr(self, "rerank_depth", 0) or 0)
        effective_rerank_depth = max(search_params["rerank_depth"], configured_rerank_depth)
        use_rerank = bool(getattr(self, "use_rerank", False) and effective_rerank_depth > 0)
        fusion_depth = search_params["fusion_depth"]
        search_top_k = max(top_k, 1, effective_rerank_depth if use_rerank else top_k)
        search_oversample = oversample
        if use_rerank:
            search_top_k = max(search_top_k, effective_rerank_depth)
            search_oversample = max(1, min(oversample, 2))
        adaptive_lex_weight = self._dynamic_lexical_weight(query_tokens, filters_active=metadata_filters.is_active())
        self._last_lexical_weight = adaptive_lex_weight
        semantic_cache = getattr(self, "_semantic_cache", None)
        semantic_cached: Optional[List[Dict[str, Any]]] = None
        can_use_semantic_cache = (
            session is None
            and semantic_cache is not None
            and q_vector is not None
            and not metadata_filters.is_active()
            and not requested_exts
        )

        if can_use_semantic_cache:
            semantic_cached = semantic_cache.match(q_vector)
            if semantic_cached is not None:
                self._record_cache_event("semantic", hit=True)
                return self._return_cached(cache_key, semantic_cached, session)

        if hasattr(index, "configure_ann"):
            ann_ef = max(32, search_top_k * max(1, search_oversample))
            try:
                index.configure_ann(ef_search=ann_ef)
            except Exception:
                pass
        raw_hits = index.search(
            q,
            top_k=search_top_k,
            oversample=search_oversample,
            lexical_weight=adaptive_lex_weight,
            query_tokens=query_tokens,
            min_similarity=self.min_similarity,
            use_ann=use_ann,
        )
        filtered_hits = _apply_metadata_filters(raw_hits, metadata_filters)
        # [Refactor] Meeting noise filtering removed.
        # filtered_hits = _filter_meeting_ai_agent_noise(filtered_hits, query)

        if not filtered_hits:
            return self._return_cached(cache_key, [], session)

        lexical_limit = max(top_k, 1)
        if use_rerank and effective_rerank_depth:
            lexical_limit = max(lexical_limit, min(effective_rerank_depth, len(filtered_hits)))

        lexical_ranking = _rerank_hits(
            query,
            vector_query,
            filtered_hits,
            desired_exts=requested_exts,
            top_k=lexical_limit,
            session=session,
        )

        if not use_rerank:
            mmr_limit = max(top_k, min(len(lexical_ranking), fusion_depth))
            mmr_candidates = lexical_ranking[:mmr_limit]
            strict_fallback = False
            if getattr(self, "strict_search", False) and _should_apply_strict_search(query):
                mmr_candidates, strict_fallback = _apply_strict_filter(query, mmr_candidates)
            use_meeting_bias = meeting_intent and any(
                _is_user_facing_meeting_artifact(str(hit.get("path") or "")) for hit in mmr_candidates
            )
            if use_meeting_bias:
                final_hits = mmr_candidates[: max(1, int(top_k))]
            else:
                final_hits = _mmr(index, mmr_candidates, q_vector, top_k)
            final_hits = _ensure_unique_paths(final_hits, mmr_candidates, top_k)
            annotated = _annotate_hits(
                final_hits,
                desired_exts=requested_exts,
                raw_query_tokens=raw_query_tokens_set,
                expanded_query_tokens=expanded_query_tokens_set,
                metadata_filters=metadata_filters,
                lexical_weight=adaptive_lex_weight,
            )
            if strict_fallback and annotated:
                note = "정확 검색 조건에 맞는 문서가 없어 일반 검색 결과를 표시합니다."
                for hit in annotated:
                    reasons = hit.setdefault("match_reasons", [])
                    if note not in reasons:
                        reasons.append(note)
            if can_use_semantic_cache and semantic_cache is not None and q_vector is not None:
                semantic_cache.store(q_vector, annotated)
                self._record_cache_event("semantic", hit=False)
            self._record_cache_event("result", hit=False)
            return self._return_cached(cache_key, annotated, session)

        reranker = self._ensure_reranker()
        if reranker is None:
            mmr_limit = max(top_k, min(len(lexical_ranking), fusion_depth))
            mmr_candidates = lexical_ranking[:mmr_limit]
            final_hits = _mmr(index, mmr_candidates, q_vector, top_k)
            annotated = _annotate_hits(
                final_hits,
                desired_exts=requested_exts,
                raw_query_tokens=raw_query_tokens_set,
                expanded_query_tokens=expanded_query_tokens_set,
                metadata_filters=metadata_filters,
                lexical_weight=adaptive_lex_weight,
            )
            if can_use_semantic_cache and semantic_cache is not None and q_vector is not None:
                semantic_cache.store(q_vector, annotated)
                self._record_cache_event("semantic", hit=False)
            self._record_cache_event("result", hit=False)
            return self._return_cached(cache_key, annotated, session)

        reranked = reranker.rerank(
            query,
            lexical_ranking,
            desired_exts=requested_exts,
            session=session,
        )
        rerank_pruned_all = False
        rerank_threshold: Optional[float] = None
        if self.rerank_min_score is not None:
            filtered: List[Dict[str, Any]] = []
            threshold = float(self.rerank_min_score)
            rerank_threshold = threshold

            for hit in reranked:
                raw_score = hit.get("rerank_score", hit.get("score", hit.get("similarity", 0.0)))
                try:
                    value = float(raw_score)
                except (TypeError, ValueError):
                    continue
                if value >= threshold:
                    filtered.append(hit)

            if filtered:
                reranked = filtered
            else:
                rerank_pruned_all = True
                reranked = []
                logger.info(
                    "rerank threshold filtered all candidates (threshold=%.2f); falling back to vector/lexical ranking",
                    threshold,
                )

        rank_sources: List[List[Dict[str, Any]]] = []
        if lexical_ranking:
            rank_sources.append(lexical_ranking[:fusion_depth])
        if reranked:
            rank_sources.append(reranked[:fusion_depth])
        fused_candidates = _rrf(rank_sources) if rank_sources else []

        if not fused_candidates:
            fused_candidates = reranked[:fusion_depth]
        mmr_pool_size = max(top_k * 2, fusion_depth)
        mmr_candidates = fused_candidates[:mmr_pool_size]
        strict_fallback = False
        if getattr(self, "strict_search", False) and _should_apply_strict_search(query):
            mmr_candidates, strict_fallback = _apply_strict_filter(query, mmr_candidates)
        use_meeting_bias = meeting_intent and any(
            _is_user_facing_meeting_artifact(str(hit.get("path") or "")) for hit in mmr_candidates
        )
        if use_meeting_bias:
            final_hits = mmr_candidates[: max(1, int(top_k))]
        else:
            final_hits = _mmr(index, mmr_candidates, q_vector, top_k)
        final_hits = _ensure_unique_paths(final_hits, mmr_candidates, top_k)
        annotated = _annotate_hits(
            final_hits,
            desired_exts=requested_exts,
            raw_query_tokens=raw_query_tokens_set,
            expanded_query_tokens=expanded_query_tokens_set,
            metadata_filters=metadata_filters,
            lexical_weight=adaptive_lex_weight,
        )
        if strict_fallback and annotated:
            note = "정확 검색 조건에 맞는 문서가 없어 일반 검색 결과를 표시합니다."
            for hit in annotated:
                reasons = hit.setdefault("match_reasons", [])
                if note not in reasons:
                    reasons.append(note)
        if rerank_pruned_all and annotated:
            note = (
                "Cross-Encoder 임계값 미달 후보는 제외되어 임베딩/키워드 순위로 대체했습니다."
                if rerank_threshold is None
                else f"Cross-Encoder 임계값 {rerank_threshold:.2f} 미만 후보는 제외되어 임베딩/키워드 순위로 대체했습니다."
            )
            for hit in annotated:
                reasons = hit.setdefault("match_reasons", [])
                if note not in reasons:
                    reasons.append(note)
        if can_use_semantic_cache and semantic_cache is not None and q_vector is not None:
            semantic_cache.store(q_vector, annotated)
            self._record_cache_event("semantic", hit=False)
        self._record_cache_event("result", hit=False)
        return self._return_cached(cache_key, annotated, session)

    def _make_cache_key(self, query: str, top_k: int) -> Tuple[str, int, bool, float]:
        normalized = (query or "").strip().lower()
        return (
            normalized,
            max(1, int(top_k or 1)),
            bool(getattr(self, "use_rerank", False)),
            round(float(getattr(self, "base_lexical_weight", 0.0) or 0.0), 3),
        )

    def _return_cached(
        self,
        cache_key: Optional[Tuple[str, int, bool, float]],
        hits: List[Dict[str, Any]],
        session: Optional[SessionState],
    ) -> List[Dict[str, Any]]:
        result_cache = getattr(self, "_result_cache", None)
        if cache_key is not None and session is None and result_cache is not None:
            result_cache.set(cache_key, hits)
        return hits

    def _record_cache_event(self, kind: str, hit: bool) -> None:
        stats = getattr(self, "_cache_stats", None)
        if stats is None:
            return
        key = f"{kind}_{'hits' if hit else 'misses'}"
        if key not in stats:
            stats[key] = 0
        stats[key] += 1
        total = stats.get("result_hits", 0) + stats.get("result_misses", 0)
        if total > 0 and total % 100 == 0:
            logger.debug(
                "cache stats: result_hit_rate=%.2f semantic_hit_rate=%.2f (total=%d)",
                stats.get("result_hits", 0) / max(1, total),
                stats.get("semantic_hits", 0) / max(1, stats.get("semantic_hits", 0) + stats.get("semantic_misses", 0)),
                total,
            )
        if kind == "semantic":
            semantic_total = stats.get("semantic_hits", 0) + stats.get("semantic_misses", 0)
            if semantic_total and semantic_total % 200 == 0:
                self._auto_tune_caches(semantic_total)

    def _auto_tune_caches(self, semantic_total: int) -> None:
        stats = getattr(self, "_cache_stats", None)
        if stats is None:
            return
        semantic_cache = getattr(self, "_semantic_cache", None)
        if semantic_cache is None:
            return
        hits = stats.get("semantic_hits", 0)
        misses = stats.get("semantic_misses", 0)
        total = hits + misses
        if total == 0:
            return
        hit_rate = hits / float(total)
        current_threshold = getattr(semantic_cache, "threshold", self._semantic_cache_initial_threshold)
        target = current_threshold
        if hit_rate < 0.15:
            target = max(0.80, current_threshold - 0.02)
        elif hit_rate < 0.3:
            target = max(0.85, current_threshold - 0.01)
        elif hit_rate > 0.65:
            target = min(0.995, current_threshold + 0.01)
        elif hit_rate > 0.5:
            target = min(0.99, current_threshold + 0.005)

        if abs(target - current_threshold) >= 1e-4:
            semantic_cache.set_threshold(target)
            logger.info(
                "semantic cache threshold tuned from %.3f to %.3f (hit_rate=%.2f, samples=%d)",
                current_threshold,
                target,
                hit_rate,
                total,
            )

    def _ensure_reranker(self) -> Optional[CrossEncoderReranker]:
        if not getattr(self, "use_rerank", False):
            return None
        if getattr(self, "_reranker", None) is not None:
            return self._reranker
        try:
            device = _pick_rerank_device(self.rerank_device)
            self._reranker = CrossEncoderReranker(
                self.rerank_model,
                device=device,
                batch_size=self.rerank_batch_size,
            )
        except Exception as exc:
            logger.warning("reranker load failed; disabling rerank: %s", exc)
            self.use_rerank = False
            self._reranker = None
        return getattr(self, "_reranker", None)

    def _dynamic_lexical_weight(self, query_tokens: Optional[List[str]], *, filters_active: bool = False) -> float:
        base = max(0.0, float(getattr(self, "base_lexical_weight", 0.0)))
        if base <= 0.0:
            return 0.0
        tokens = [str(tok) for tok in (query_tokens or []) if str(tok).strip()]
        if not tokens:
            return base

        weight = base
        lowered_tokens = {tok.lower() for tok in tokens if tok}

        hint_applied = False
        if any(_looks_like_identifier(tok) for tok in tokens):
            weight = max(weight, 0.7)
            hint_applied = True
        else:
            for keyword_forms, hint_weight in _LEXICAL_KEYWORD_HINTS:
                if lowered_tokens & keyword_forms:
                    weight = max(weight, hint_weight)
                    hint_applied = True
                    break

        distinct = len(lowered_tokens)
        if distinct <= 2:
            weight = min(0.75, max(weight, 0.45))
        elif distinct >= 8 and not hint_applied:
            weight = max(0.15, min(weight, 0.30))

        if filters_active:
            weight = max(weight, 0.35)
            weight = min(weight, 0.85)

        return float(max(0.0, min(0.9, weight)))

    def _load_cached_index(self) -> Optional[VectorIndex]:
        emb_npy = self.cache_dir / "doc_embeddings.npy"
        meta_json = self.cache_dir / "doc_meta.json"
        faiss_path = self.cache_dir / "doc_index.faiss"
        if not meta_json.exists():
            return None

        index = VectorIndex()
        try:
            index.load(
                emb_npy if emb_npy.exists() else None,
                meta_json,
                faiss_path=faiss_path if faiss_path.exists() else None,
                use_mmap=True,
            )
        except Exception as exc:
            logger.warning("index load failed; rebuild scheduled: %s", exc)
            return None

        if not self._index_matches_model(index):
            logger.warning("index dimension mismatch detected; triggering rebuild")
            return None

        logger.info("index loaded: cache=%s", _mask_path(str(self.cache_dir)))
        return index

    def _rebuild_index(self) -> VectorIndex:
        if pd is None:
            raise RuntimeError("pandas 필요. pip install pandas")

        df = self._load_corpus().copy()
        _prepare_text_frame(df)

        if MODEL_TEXT_COLUMN not in df.columns:
            raise RuntimeError("코퍼스에 학습 텍스트 컬럼이 없습니다.")

        mask = df[MODEL_TEXT_COLUMN].str.len() > 0
        work = df.loc[mask].copy()
        if work.empty:
            raise RuntimeError("유효 텍스트 문서가 없습니다.")

        logger.info("encoding documents for index build: docs=%d", len(work))
        Z = self.encoder.encode_docs(work[MODEL_TEXT_COLUMN].tolist())

        preview_source = work.get("preview")
        if preview_source is None:
            preview_source = work.get("text_original")
        if preview_source is None:
            preview_source = work.get("text")
        if preview_source is None:
            preview_source = work[MODEL_TEXT_COLUMN]
        preview_list = preview_source.fillna("").astype(str).tolist()

        token_source = work.get(MODEL_TEXT_COLUMN)
        if token_source is None:
            token_source = work.get("text")
        if token_source is None:
            token_source = work.get("preview")
        token_texts = token_source.fillna("").astype(str).tolist()

        token_lists: Optional[List[List[str]]] = None
        if BM25Okapi is not None:
            tokens_raw = work.get("tokens")
            if tokens_raw is not None:
                token_lists = [tokens_raw.iloc[i] if isinstance(tokens_raw.iloc[i], list) else _split_tokens(tokens_raw.iloc[i]) for i in range(len(work))]
            else:
                token_lists = [
                    [tok for tok in _split_tokens(token_texts[idx].lower()) if tok]
                    for idx in range(len(work))
                ]
                total_tokens = sum(len(tokens) for tokens in token_lists)
                if total_tokens > MAX_BM25_TOKENS:
                    factor = MAX_BM25_TOKENS / total_tokens
                    limited_tokens: List[List[str]] = []
                    truncated = 0
                    for tokens in token_lists:
                        keep = max(1, int(len(tokens) * factor))
                        if keep < len(tokens):
                            truncated += len(tokens) - keep
                        limited_tokens.append(tokens[:keep])
                    token_lists = limited_tokens
                    if truncated:
                        logger.info(
                            "bm25 tokens truncated: removed=%d limit=%d",
                            truncated,
                            MAX_BM25_TOKENS,
                        )

        size_list = work["size"].fillna(0).astype(int).tolist() if "size" in work.columns else [0] * len(work)
        mtime_list = work["mtime"].fillna(0.0).astype(float).tolist() if "mtime" in work.columns else [0.0] * len(work)
        ctime_list = work["ctime"].fillna(0.0).astype(float).tolist() if "ctime" in work.columns else [0.0] * len(work)
        owner_list = work["owner"].fillna("").astype(str).tolist() if "owner" in work.columns else [""] * len(work)
        drive_list = work["drive"].fillna("").astype(str).tolist() if "drive" in work.columns else [""] * len(work)

        row_count = len(work)
        extra_meta_payload: List[Dict[str, Any]] = [{} for _ in range(row_count)]
        extra_fields_present = False
        if "chunk_id" in work.columns:
            for i in range(row_count):
                value = work["chunk_id"].iloc[i]
                if pd.notna(value):
                    extra_meta_payload[i]["chunk_id"] = int(value)
                    extra_fields_present = True
        if "chunk_count" in work.columns:
            for i in range(row_count):
                value = work["chunk_count"].iloc[i]
                if pd.notna(value):
                    extra_meta_payload[i]["chunk_count"] = int(value)
                    extra_fields_present = True
        if "chunk_tokens" in work.columns:
            for i in range(row_count):
                value = work["chunk_tokens"].iloc[i]
                if pd.notna(value):
                    extra_meta_payload[i]["chunk_tokens"] = int(value)
                    extra_fields_present = True
        if "doc_tags" in work.columns:
            for i in range(row_count):
                value = work["doc_tags"].iloc[i]
                if isinstance(value, list) and value:
                    tags = [str(tag).strip() for tag in value if str(tag).strip()]
                    if tags:
                        extra_meta_payload[i]["doc_tags"] = tags
                        extra_fields_present = True
        if "doc_primary_tag" in work.columns:
            for i in range(row_count):
                value = work["doc_primary_tag"].iloc[i]
                if pd.notna(value) and str(value).strip():
                    extra_meta_payload[i]["doc_primary_tag"] = str(value).strip()
                    extra_fields_present = True

        extra_meta: Optional[List[Dict[str, Any]]]
        if extra_fields_present:
            extra_meta = extra_meta_payload
        else:
            extra_meta = None

        index = VectorIndex()
        index.build(
            Z,
            work["path"].tolist(),
            work["ext"].tolist(),
            preview_list,
            tokens=token_lists,
            sizes=size_list,
            mtimes=mtime_list,
            ctimes=ctime_list,
            owners=owner_list,
            drives=drive_list,
            extra_meta=extra_meta,
        )
        paths = index.save(self.cache_dir)
        saved_files = [str(paths.meta_json)]
        if paths.emb_npy:
            saved_files.append(str(paths.emb_npy))
        if paths.faiss_index:
            saved_files.append(str(paths.faiss_index))
        logger.info(
            "index saved: %s",
            ", ".join(_mask_path(path) for path in saved_files),
        )

        fresh = VectorIndex()
        fresh.load(
            paths.emb_npy,
            paths.meta_json,
            faiss_path=paths.faiss_index,
            use_mmap=True,
        )
        self._cache_signature = self._compute_cache_signature()
        return fresh

    def _load_corpus(self):
        logger.info("loading corpus: path=%s", _mask_path(str(self.corpus_path)))
        if self.corpus_path.suffix.lower() == ".parquet":
            engine_kwargs = {}
            engine_label = PARQUET_ENGINE or "auto"
            if PARQUET_ENGINE:
                engine_kwargs["engine"] = PARQUET_ENGINE
            try:
                return pd.read_parquet(self.corpus_path, **engine_kwargs)
            except Exception as exc:
                logger.warning(
                    "parquet load failed (engine=%s); retrying via CSV: %s",
                    engine_label,
                    exc,
                )
                return pd.read_csv(self.corpus_path.with_suffix(".csv"))
        return pd.read_csv(self.corpus_path)

    def _index_matches_model(self, index: VectorIndex) -> bool:
        if index.Z is None:
            return False
        cached_dim = index.Z.shape[1]
        model_dim = getattr(self.encoder, "embedding_dim", None)
        if model_dim is None and getattr(self.encoder, "svd", None) is not None:
            model_dim = getattr(self.encoder.svd, "n_components", None)
            if model_dim is None:
                components = getattr(self.encoder.svd, "components_", None)
                if components is not None:
                    model_dim = components.shape[0]
        if cached_dim and model_dim and cached_dim != model_dim:
            return False
        return True

    def _compute_cache_signature(self) -> Tuple[float, float, float, float]:
        def _mtime(path: Path) -> float:
            try:
                return path.stat().st_mtime
            except OSError:
                return 0.0

        emb = self.cache_dir / "doc_embeddings.npy"
        meta = self.cache_dir / "doc_meta.json"
        faiss_path = self.cache_dir / "doc_index.faiss"
        return (VectorIndex.LEXICAL_SCHEMA_VERSION, _mtime(meta), _mtime(emb), _mtime(faiss_path))

    def _refresh_if_cache_changed(self) -> None:
        current = self._compute_cache_signature()
        previous = getattr(self, "_cache_signature", None)
        if previous is None:
            self._cache_signature = current
            monitor = getattr(self, "_cache_monitor", None)
            if monitor is not None:
                monitor.prime(current)
            return
        if current != previous:
            self._on_cache_signature_change(previous, current)

    def _on_cache_signature_change(
        self,
        previous: Tuple[float, float, float, float],
        current: Tuple[float, float, float, float],
    ) -> None:
        self._cache_signature = current
        result_cache = getattr(self, "_result_cache", None)
        if result_cache is not None:
            result_cache.clear()
        semantic_cache = getattr(self, "_semantic_cache", None)
        if semantic_cache is not None:
            semantic_cache.clear()
        try:
            self.index_manager.clear()
            loaded = self.index_manager.ensure_loaded()
            if loaded is None:
                self.index_manager.schedule_rebuild(priority=True)
        except Exception as exc:
            logger.warning("index refresh after cache change failed: %s", exc)

    @staticmethod
    def format_results(query: str, results: List[Dict[str, Any]]) -> str:
        if not results:
            return f"“{query}”와 유사한 문서를 찾지 못했습니다."
        lines = [f"‘{query}’와 유사한 문서 Top {len(results)}:"]
        for i, r in enumerate(results, 1):
            vector_similarity = r.get("vector_similarity", r.get("similarity"))
            similarity_label = _similarity_to_percent(vector_similarity)
            final_score = r.get("combined_score", r.get("score", 0.0))
            try:
                final_score = float(final_score)
            except (TypeError, ValueError):
                final_score = 0.0
            path_raw = str(r.get("path", "") or "")
            name = _mask_path(path_raw) or path_raw or "<unknown>"
            ext_label = str(r.get("ext", "") or "")
            ext_display = f" [{ext_label}]" if ext_label else ""
            lines.append(
                f"{i}. {name}{ext_display}  유사도={similarity_label}  종합점수={final_score:.3f}"
            )
            meta_bits: List[str] = []
            mod_date = _format_human_time(r.get("mtime") or r.get("ctime"))
            if mod_date:
                meta_bits.append(f"수정일 {mod_date}")
            size_label = _format_size(r.get("size"))
            if size_label:
                meta_bits.append(size_label)
            owner = str(r.get("owner") or "").strip()
            if owner:
                meta_bits.append(f"작성자 {owner}")
            drive_label = str(r.get("drive") or "").strip()
            if drive_label:
                meta_bits.append(f"드라이브 {drive_label}")
            if meta_bits:
                lines.append("   메타: " + ", ".join(meta_bits))
            if r.get("preview"):
                lines.append(f"   미리보기: {r['preview']}")
        return "\n".join(lines)


def _prepare_text_frame(df: "pd.DataFrame") -> "pd.DataFrame":
    if pd is None or df is None:
        return df
    if df.empty:
        if MODEL_TEXT_COLUMN not in df.columns:
            df[MODEL_TEXT_COLUMN] = pd.Series(dtype=str)
        return df

    for column in ("text", "text_original"):
        if column in df.columns:
            df[column] = df[column].fillna("").astype(str)

    if "text" not in df.columns:
        df["text"] = ""

    paths = df.get("path")
    if paths is None:
        paths = pd.Series([""] * len(df))
    else:
        paths = paths.fillna("").astype(str)

    exts = df.get("ext")
    if exts is None:
        exts = pd.Series([""] * len(df))
    else:
        exts = exts.fillna("").astype(str)

    drives = df.get("drive")
    if drives is None:
        drives = pd.Series([""] * len(df))
    else:
        drives = drives.fillna("").astype(str)

    sizes = df.get("size")
    if sizes is None:
        sizes = pd.Series([0] * len(df))
    else:
        sizes = sizes.fillna(0).astype(int)

    mtimes = df.get("mtime")
    if mtimes is None:
        mtimes = pd.Series([0.0] * len(df))
    else:
        mtimes = mtimes.fillna(0.0).astype(float)

    ctimes = df.get("ctime")
    if ctimes is None:
        ctimes = pd.Series([0.0] * len(df))
    else:
        ctimes = ctimes.fillna(0.0).astype(float)

    owners = df.get("owner")
    if owners is None:
        owners = pd.Series([""] * len(df))
    else:
        owners = owners.fillna("").astype(str)

    base_texts = df["text"].tolist()
    metadata_list = [
        _metadata_text(
            paths.iat[idx],
            exts.iat[idx],
            drives.iat[idx],
            size=sizes.iat[idx],
            mtime=mtimes.iat[idx],
            ctime=ctimes.iat[idx],
            owner=owners.iat[idx],
        )
        for idx in range(len(df))
    ]
    df[MODEL_TEXT_COLUMN] = [
        _compose_model_text(base_texts[idx], metadata_list[idx])
        for idx in range(len(df))
    ]
    return df

# Backward compatibility alias
HybridRetriever = Retriever
