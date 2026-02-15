# vector_index.py - Extracted from retriever.py (GOD CLASS refactoring)
"""FAISS/HNSW Vector Index management for hybrid search."""

from __future__ import annotations

import json
import hashlib
import math
import logging
import os
import re
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Set, Tuple

try:
    import numpy as np
except Exception:
    np = None  # type: ignore

try:
    import faiss
except ImportError:
    faiss = None

try:
    import hnswlib
except ImportError:
    hnswlib = None

try:
    from rank_bm25 import BM25Okapi
except Exception:
    BM25Okapi = None

from core.utils.stopwords import STOPWORDS
from core.utils.nlp import split_tokens as _split_tokens_util

logger = logging.getLogger(__name__)

MAX_PREVIEW_CHARS = 180
MAX_BM25_TOKENS = 8000


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


@dataclass
class IndexPaths:
    emb_npy: Optional[Path]
    meta_json: Path
    faiss_index: Optional[Path] = None


class VectorIndex:
    LEXICAL_SCHEMA_VERSION = 2.0
    def __init__(self) -> None:
        self.dimension: Optional[int] = None
        self.doc_ids: List[int] = []
        self.entries: Dict[int, Dict[str, Any]] = {}
        self.lexical_tokens: Dict[int, List[str]] = {}
        self.embeddings: Dict[int, np.ndarray] = {}
        self._path_to_id: Dict[str, int] = {}

        self.paths: List[str] = []
        self.exts: List[str] = []
        self.preview: List[str] = []
        self.sizes: List[Optional[int]] = []
        self.mtimes: List[Optional[float]] = []
        self.ctimes: List[Optional[float]] = []
        self.owners: List[Optional[str]] = []
        self.drives: List[Optional[str]] = []
        self.chunk_ids: List[Optional[int]] = []
        self.chunk_counts: List[Optional[int]] = []
        self.chunk_tokens: List[Optional[int]] = []

        self.Z: Optional[np.ndarray] = None
        self.faiss_index = None
        self.lexical_index: Optional[BM25Okapi] = None
        self.ann_index = None
        self._ann_backend_active: Optional[str] = None
        self._ann_hnsw = None

        self._matrix_dirty = True
        self._faiss_dirty = True
        self._lexical_dirty = True
        self._ann_dirty = True

        self.lexical_weight = 0.0
        self.ann_threshold = 50000
        self.ann_m = 32
        self.ann_ef_construction = 80
        self.ann_ef_search = 64
        self.faiss_use_pq = True
        self.faiss_pq_threshold = 2000
        self.faiss_pq_m = 32
        self.faiss_pq_nbits = 8
        self.faiss_pq_min_nlist = 64
        self.faiss_pq_max_nlist = 4096
        self.faiss_pq_nprobe = 64
        self._faiss_pq_active = False

        # v1.1 HNSW Configuration
        self.use_hnsw = True
        self.hnsw_m = 32
        self.hnsw_ef_construction = 200
        self.hnsw_ef_search = 128
        self.hnsw_min_size = 10000 # Use Flat below this, HNSW above

    @staticmethod
    def _normalize_rows(matrix: np.ndarray) -> np.ndarray:
        norms = np.linalg.norm(matrix, axis=1, keepdims=True) + 1e-12
        return (matrix / norms).astype(np.float32, copy=False)

    @staticmethod
    def _normalize_vector(vec: np.ndarray) -> np.ndarray:
        arr = np.asarray(vec, dtype=np.float32).reshape(-1)
        norm = float(np.linalg.norm(arr)) + 1e-12
        return (arr / norm).astype(np.float32, copy=False)

    @staticmethod
    def _ann_backend() -> Optional[str]:
        if os.getenv("DISABLE_ANN") == "1":
            return None
        prefer_faiss = os.getenv("FORCE_FAISS") == "1" or os.getenv("DISABLE_HNSWLIB") == "1"
        if not prefer_faiss and hnswlib is not None:
            return "hnswlib"
        if faiss is not None:
            return "faiss"
        if hnswlib is not None:
            return "hnswlib"
        return None

    @staticmethod
    def _normalize_path(path: str) -> str:
        try:
            return os.path.normcase(str(Path(path).resolve()))
        except Exception:
            return os.path.normcase(str(path))

    @staticmethod
    def _truncate_preview(text: str, limit: int = MAX_PREVIEW_CHARS) -> str:
        src = (text or "").strip()
        return src if len(src) <= limit else f"{src[:limit]}…"

    @staticmethod
    def _generate_doc_ids(paths: Iterable[str]) -> List[int]:
        assigned: Set[int] = set()
        ids: List[int] = []
        mask = (1 << 63) - 1
        for raw_path in paths:
            norm = VectorIndex._normalize_path(raw_path)
            digest = hashlib.sha1(norm.encode("utf-8")).digest()
            candidate = int.from_bytes(digest[:8], byteorder="big") & mask
            while candidate in assigned:
                candidate = (candidate + 1) & mask
            assigned.add(candidate)
            ids.append(candidate)
        return ids

    def _allocate_doc_id(self, path: str) -> int:
        normalized = self._normalize_path(path)
        existing = self._path_to_id.get(normalized)
        if existing is not None:
            return existing
        mask = (1 << 63) - 1
        digest = hashlib.sha1(normalized.encode("utf-8")).digest()
        candidate = int.from_bytes(digest[:8], byteorder="big") & mask
        while candidate in self.entries and self.entries[candidate].get("path") != path:
            candidate = (candidate + 1) & mask
        return candidate

    def _rebuild_lists(self) -> None:
        self.paths = []
        self.exts = []
        self.preview = []
        self.sizes = []
        self.mtimes = []
        self.ctimes = []
        self.owners = []
        self.drives = []
        self.chunk_ids = []
        self.chunk_counts = []
        self.chunk_tokens = []
        for doc_id in self.doc_ids:
            entry = self.entries.get(doc_id, {})
            self.paths.append(entry.get("path", ""))
            self.exts.append(entry.get("ext", ""))
            self.preview.append(entry.get("preview", ""))
            self.sizes.append(entry.get("size"))
            self.mtimes.append(entry.get("mtime"))
            self.ctimes.append(entry.get("ctime"))
            self.owners.append(entry.get("owner"))
            self.drives.append(entry.get("drive"))
            self.chunk_ids.append(entry.get("chunk_id"))
            self.chunk_counts.append(entry.get("chunk_count"))
            self.chunk_tokens.append(entry.get("chunk_tokens"))

    def _mark_faiss_dirty(self) -> None:
        self._faiss_dirty = True

    def _mark_lexical_dirty(self) -> None:
        self._lexical_dirty = True

    def _mark_ann_dirty(self) -> None:
        self._ann_dirty = True
        self._ann_backend_active = None

    def _ensure_matrix(self) -> None:
        if not self._matrix_dirty:
            return
        if not self.embeddings:
            self.Z = None
            self._matrix_dirty = False
            return
        ordered = [self.embeddings[doc_id] for doc_id in self.doc_ids if doc_id in self.embeddings]
        if not ordered:
            self.Z = None
            self._matrix_dirty = False
            return
        self.Z = np.vstack(ordered).astype(np.float32, copy=False)
        self.dimension = self.Z.shape[1]
        self._matrix_dirty = False

    def _ensure_faiss_index(self) -> None:
        if not self._faiss_dirty:
            return
        if faiss is None or self._ann_backend() == "hnswlib":
            self.faiss_index = None
            self._faiss_dirty = False
            return
        if not self.doc_ids:
            self.faiss_index = None
            self._faiss_dirty = False
            return
        if self.Z is None:
            self._ensure_matrix()
        if self.Z is None or self.Z.size == 0:
            self.faiss_index = None
            self._faiss_dirty = False
            return
        dim = self.Z.shape[1]
        use_pq = (
            self.faiss_use_pq
            and len(self.doc_ids) >= max(1, self.faiss_pq_threshold)
        )
        index = None
        index = None
        
        # v1.1 HNSW Logic
        if self.use_hnsw and len(self.doc_ids) >= self.hnsw_min_size:
             index = self._build_hnsw_index(dim)
             
        if index is None and use_pq:
            index = self._build_ivfpq_index(dim)
            
        if index is None:
            base = faiss.IndexFlatIP(dim)
            index = faiss.IndexIDMap(base)
            ids = np.asarray(self.doc_ids, dtype=np.int64)
            index.add_with_ids(self.Z, ids)
            self._faiss_pq_active = False
        else:
            self._faiss_pq_active = True
        self.faiss_index = index
        self.dimension = dim
        self._faiss_dirty = False

    def _ensure_lexical_index(self) -> None:
        if not self._lexical_dirty:
            return
        if BM25Okapi is None or not self.doc_ids:
            self.lexical_index = None
            self._lexical_dirty = False
            return
        corpus = [self.lexical_tokens.get(doc_id, []) for doc_id in self.doc_ids]
        try:
            self.lexical_index = BM25Okapi(corpus) if corpus else None
        except Exception:
            self.lexical_index = None
        self._lexical_dirty = False

    def _resolve_pq_nlist(self) -> int:
        total = len(self.doc_ids)
        if total <= 0:
            return 0
        base = max(self.faiss_pq_min_nlist, total // 8)
        nlist = min(self.faiss_pq_max_nlist, base)
        nlist = max(1, min(nlist, total - 1))
        return nlist

    def _resolve_pq_m(self, dim: int) -> int:
        target = min(max(1, self.faiss_pq_m), dim)
        while target > 1 and dim % target != 0:
            target -= 1
        return max(1, target)

    def _build_ivfpq_index(self, dim: int):
        if faiss is None or self.Z is None or self.Z.size == 0:
            return None
        nlist = self._resolve_pq_nlist()
        if nlist <= 0:
            return None
        subvectors = self._resolve_pq_m(dim)
        if subvectors <= 0:
            return None
        try:
            quantizer = faiss.IndexFlatIP(dim)
            metric = getattr(faiss, "METRIC_INNER_PRODUCT", faiss.METRIC_L2)
            index = faiss.IndexIVFPQ(
                quantizer,
                dim,
                nlist,
                subvectors,
                max(1, int(self.faiss_pq_nbits)),
                metric,
            )
        except Exception:
            return None
        try:
            index.train(self.Z)
        except Exception:
            return None
        ids = np.asarray(self.doc_ids, dtype=np.int64)
        try:
            index.add_with_ids(self.Z, ids)
        except Exception:
            return None
        nprobe = max(1, min(self.faiss_pq_nprobe, nlist))
        try:
            index.nprobe = int(nprobe)
        except Exception:
            pass
        return index

    def _build_hnsw_index(self, dim: int):
        """v1.1 HNSW Index Builder"""
        if faiss is None or self.Z is None or self.Z.size == 0:
            return None
        
        try:
            # IndexHNSWFlat supports inner product via METRIC_INNER_PRODUCT
            metric = getattr(faiss, "METRIC_INNER_PRODUCT", faiss.METRIC_L2)
            index = faiss.IndexHNSWFlat(dim, self.hnsw_m, metric)
            index.hnsw.efConstruction = self.hnsw_ef_construction
            index.hnsw.efSearch = self.hnsw_ef_search
            
            # Train not needed for HNSWFlat (it stores full vectors)
            # But we wrap in IDMap to track IDs
            id_index = faiss.IndexIDMap(index)
            
            ids = np.asarray(self.doc_ids, dtype=np.int64)
            id_index.add_with_ids(self.Z, ids)
            
            logger.info("Built HNSW index for %d vectors", len(self.doc_ids))
            return id_index
        except Exception as e:
            logger.warning("Failed to build HNSW index: %s", e)
            return None

    def _tokenize_entry(self, entry: Dict[str, Any]) -> List[str]:
        corpus = " ".join(
            str(entry.get(field, "")) for field in ("preview", "path", "ext", "owner", "drive")
        )
        return [tok for tok in _split_tokens_util(corpus.lower()) if tok]

    def build(
        self,
        embeddings: np.ndarray,
        paths: List[str],
        exts: List[str],
        preview_texts: List[str],
        *,
        tokens: Optional[List[List[str]]] = None,
        sizes: Optional[List[Optional[int]]] = None,
        mtimes: Optional[List[Optional[float]]] = None,
        ctimes: Optional[List[Optional[float]]] = None,
        owners: Optional[List[Optional[str]]] = None,
        drives: Optional[List[Optional[str]]] = None,
        doc_ids: Optional[List[int]] = None,
        extra_meta: Optional[List[Dict[str, Any]]] = None,
    ) -> None:
        if embeddings.ndim != 2:
            raise ValueError("embeddings는 2차원이어야 합니다.")

        normalized_embeddings = self._normalize_rows(embeddings)
        count = normalized_embeddings.shape[0]
        if doc_ids is None:
            doc_ids = self._generate_doc_ids(paths)
        if len(doc_ids) != count:
            raise ValueError("doc_id 수와 임베딩 수가 다릅니다.")

        self.dimension = normalized_embeddings.shape[1]
        self.doc_ids = [int(doc_id) for doc_id in doc_ids]
        self.entries.clear()
        self.lexical_tokens.clear()
        self.embeddings = {
            doc_id: normalized_embeddings[idx]
            for idx, doc_id in enumerate(self.doc_ids)
        }
        self._path_to_id.clear()

        def _meta_list(values: Optional[List[Any]], fallback: Any) -> List[Any]:
            if values is None:
                return [fallback] * count
            if len(values) != count:
                raise ValueError("메타데이터 길이가 문서 수와 다릅니다.")
            return list(values)

        size_list = _meta_list(sizes, 0)
        mtime_list = _meta_list(mtimes, 0.0)
        ctime_list = _meta_list(ctimes, 0.0)
        owner_list = _meta_list(owners, "")
        drive_list = _meta_list(drives, "")

        for idx, doc_id in enumerate(self.doc_ids):
            entry = {
                "path": paths[idx],
                "ext": exts[idx],
                "preview": preview_texts[idx],
                "size": size_list[idx],
                "mtime": mtime_list[idx],
                "ctime": ctime_list[idx],
                "owner": owner_list[idx],
                "drive": drive_list[idx],
            }
            if extra_meta and idx < len(extra_meta):
                for key, value in extra_meta[idx].items():
                    entry[key] = value
            entry["preview"] = self._truncate_preview(entry.get("preview", ""))
            self.entries[doc_id] = entry
            provided_tokens = tokens[idx] if tokens and idx < len(tokens) else None
            self.lexical_tokens[doc_id] = list(provided_tokens) if provided_tokens else self._tokenize_entry(entry)
            self._path_to_id[self._normalize_path(entry["path"])] = doc_id

        self._rebuild_lists()
        self._matrix_dirty = True
        self._mark_faiss_dirty()
        self._mark_lexical_dirty()
        self._mark_ann_dirty()
        self._ensure_matrix()
        self._ensure_faiss_index()
        self._ensure_lexical_index()

    def save(self, out_dir: Path) -> IndexPaths:
        out_dir.mkdir(parents=True, exist_ok=True)
        emb_path = out_dir / "doc_embeddings.npy"
        meta_path = out_dir / "doc_meta.json"
        faiss_path: Optional[Path] = out_dir / "doc_index.faiss"

        def _tmp_path(final: Path) -> Path:
            suffix = f".tmp.{os.getpid()}.{time.time_ns()}"
            return final.with_name(final.stem + suffix + final.suffix)

        self._ensure_matrix()
        if self.Z is not None:
            tmp_emb = _tmp_path(emb_path)
            np.save(tmp_emb, self.Z.astype(np.float32, copy=False), allow_pickle=False)
            os.replace(tmp_emb, emb_path)
        else:
            try:
                emb_path.unlink()
            except FileNotFoundError:
                pass
            emb_path = None

        tokens_payload = [self.lexical_tokens.get(doc_id, []) for doc_id in self.doc_ids]
        chunk_id_payload = [self.entries.get(doc_id, {}).get("chunk_id") for doc_id in self.doc_ids]
        chunk_count_payload = [self.entries.get(doc_id, {}).get("chunk_count") for doc_id in self.doc_ids]
        chunk_tokens_payload = [self.entries.get(doc_id, {}).get("chunk_tokens") for doc_id in self.doc_ids]
        payload = {
            "schema_version": self.LEXICAL_SCHEMA_VERSION,
            "doc_ids": self.doc_ids,
            "paths": self.paths,
            "exts": self.exts,
            "preview": self.preview,
            "sizes": self.sizes,
            "mtimes": self.mtimes,
            "ctimes": self.ctimes,
            "owners": self.owners,
            "drives": self.drives,
            "tokens": tokens_payload,
            "chunk_id": chunk_id_payload,
            "chunk_count": chunk_count_payload,
            "chunk_tokens": chunk_tokens_payload,
        }

        # Save FAISS before meta so meta acts as the commit marker.
        if faiss is not None:
            self._ensure_faiss_index()
            if self.faiss_index is not None:
                tmp_faiss = _tmp_path(faiss_path)
                faiss.write_index(self.faiss_index, str(tmp_faiss))
                os.replace(tmp_faiss, faiss_path)
            else:
                try:
                    faiss_path.unlink()
                except FileNotFoundError:
                    pass
                faiss_path = None
        else:
            try:
                faiss_path.unlink()
            except FileNotFoundError:
                pass
            faiss_path = None

        tmp_meta = _tmp_path(meta_path)
        with tmp_meta.open("w", encoding="utf-8") as f:
            json.dump(
                payload,
                f,
                ensure_ascii=False,
            )
        os.replace(tmp_meta, meta_path)

        return IndexPaths(emb_npy=emb_path, meta_json=meta_path, faiss_index=faiss_path)

    def load(
        self,
        emb_npy: Optional[Path],
        meta_json: Path,
        *,
        faiss_path: Optional[Path] = None,
        use_mmap: bool = True,
    ) -> None:
        if not meta_json.exists():
            raise FileNotFoundError(f"메타데이터 파일을 찾을 수 없습니다: {meta_json}")

        with meta_json.open("r", encoding="utf-8") as f:
            meta = json.load(f)

        schema_version = float(meta.get("schema_version", 1.0))
        if schema_version < self.LEXICAL_SCHEMA_VERSION:
            raise ValueError(
                f"index schema v{schema_version:.1f} detected; requires v{self.LEXICAL_SCHEMA_VERSION:.1f}"
            )

        paths = meta.get("paths", [])
        exts = meta.get("exts", [])
        previews = meta.get("preview", [])
        doc_ids = [int(x) for x in meta.get("doc_ids", list(range(len(paths))))]
        sizes = meta.get("sizes", [0] * len(paths))
        mtimes = meta.get("mtimes", [0.0] * len(paths))
        ctimes = meta.get("ctimes", [0.0] * len(paths))
        owners = meta.get("owners", [""] * len(paths))
        drives = meta.get("drives", [""] * len(paths))
        tokens_payload = meta.get("tokens", [[] for _ in doc_ids])
        chunk_id_payload = meta.get("chunk_id", [None for _ in doc_ids])
        chunk_count_payload = meta.get("chunk_count", [None for _ in doc_ids])
        chunk_tokens_payload = meta.get("chunk_tokens", [None for _ in doc_ids])

        self.doc_ids = doc_ids
        self.entries.clear()
        self.lexical_tokens.clear()
        self.embeddings.clear()
        self._path_to_id.clear()

        for idx, doc_id in enumerate(self.doc_ids):
            entry = {
                "path": paths[idx] if idx < len(paths) else "",
                "ext": exts[idx] if idx < len(exts) else "",
                "preview": previews[idx] if idx < len(previews) else "",
                "size": sizes[idx] if idx < len(sizes) else 0,
                "mtime": mtimes[idx] if idx < len(mtimes) else 0.0,
                "ctime": ctimes[idx] if idx < len(ctimes) else 0.0,
                "owner": owners[idx] if idx < len(owners) else "",
                "drive": drives[idx] if idx < len(drives) else "",
            }
            def _coerce_optional_int(raw: Any) -> Optional[int]:
                if raw in (None, "", "null"):
                    return None
                if isinstance(raw, int):
                    return raw
                try:
                    as_float = float(raw)
                except (TypeError, ValueError):
                    return None
                if math.isnan(as_float):
                    return None
                return int(as_float)

            if idx < len(chunk_id_payload):
                entry["chunk_id"] = _coerce_optional_int(chunk_id_payload[idx])
            if idx < len(chunk_count_payload):
                entry["chunk_count"] = _coerce_optional_int(chunk_count_payload[idx])
            if idx < len(chunk_tokens_payload):
                entry["chunk_tokens"] = _coerce_optional_int(chunk_tokens_payload[idx])
            entry["preview"] = self._truncate_preview(entry.get("preview", ""))
            self.entries[doc_id] = entry
            provided_tokens = tokens_payload[idx] if idx < len(tokens_payload) else []
            self.lexical_tokens[doc_id] = list(provided_tokens)
            self._path_to_id[self._normalize_path(entry["path"])] = doc_id

        self._rebuild_lists()

        if emb_npy and emb_npy.exists():
            mmap_mode = "r" if use_mmap else None
            matrix = np.load(emb_npy, mmap_mode=mmap_mode)
            if matrix.dtype != np.float32:
                matrix = matrix.astype(np.float32, copy=False)
            self.Z = matrix
            self.dimension = matrix.shape[1] if matrix.ndim == 2 and matrix.size else None
            for idx, doc_id in enumerate(self.doc_ids):
                if idx < matrix.shape[0]:
                    self.embeddings[doc_id] = matrix[idx]
            self._matrix_dirty = False
        else:
            self.Z = None
            self._matrix_dirty = True

        if faiss is not None and faiss_path and faiss_path.exists():
            try:
                self.faiss_index = faiss.read_index(str(faiss_path))
                self.dimension = getattr(self.faiss_index, "d", self.dimension)
                try:
                    self._faiss_pq_active = isinstance(self.faiss_index, faiss.IndexIVFPQ)
                except Exception:
                    self._faiss_pq_active = False
                self._faiss_dirty = False
            except Exception:
                self.faiss_index = None
                self._faiss_dirty = True
        else:
            self.faiss_index = None
            self._faiss_dirty = True

        if self.dimension is None and self.embeddings:
            any_vec = next(iter(self.embeddings.values()))
            self.dimension = len(any_vec)

        self._mark_lexical_dirty()
        self._ensure_lexical_index()
        self._mark_ann_dirty()

    def configure_ann(
        self,
        *,
        threshold: Optional[int] = None,
        ef_search: Optional[int] = None,
        ef_construction: Optional[int] = None,
        m: Optional[int] = None,
    ) -> None:
        rebuild = False
        if threshold is not None:
            new_threshold = max(0, int(threshold))
            if new_threshold != self.ann_threshold:
                self.ann_threshold = new_threshold
                rebuild = True
        if ef_construction is not None:
            new_ef_construction = max(16, int(ef_construction))
            if new_ef_construction != self.ann_ef_construction:
                self.ann_ef_construction = new_ef_construction
                rebuild = True
        if m is not None:
            new_m = max(8, int(m))
            if new_m != self.ann_m:
                self.ann_m = new_m
                rebuild = True
        if rebuild:
            self._mark_ann_dirty()
        if ef_search is not None:
            new_ef_search = max(8, int(ef_search))
            if new_ef_search != self.ann_ef_search:
                self.ann_ef_search = new_ef_search
                if self.ann_index is not None:
                    try:
                        self.ann_index.hnsw.efSearch = self.ann_ef_search
                    except Exception:
                        pass

    def search(
        self,
        qvec: np.ndarray,
        top_k: int = 5,
        *,
        oversample: int = 1,
        lexical_weight: float = 0.0,
        query_tokens: Optional[List[str]] = None,
        min_similarity: float = 0.0,
        use_ann: Optional[bool] = None,
    ) -> List[Dict[str, Any]]:
        if not self.doc_ids:
            return []
        q = self._normalize_vector(qvec)
        fetch = max(1, min(len(self.doc_ids), top_k * max(1, oversample)))

        vector_scores, vector_order = self._vector_scores(q, fetch, use_ann=use_ann)
        lex_weight = max(0.0, min(1.0, lexical_weight))
        use_lexical = lex_weight > 0.0 and bool(query_tokens)

        lexical_scores: Dict[int, float] = {}
        if use_lexical:
            lexical_fetch = min(len(self.doc_ids), max(fetch, top_k * 8))
            lexical_scores = self._lexical_scores(query_tokens, lexical_fetch)

        candidate_ids: Set[int] = set(vector_scores.keys())
        if use_lexical:
            candidate_ids |= set(lexical_scores.keys())
        if not candidate_ids:
            candidate_ids = set(self.doc_ids[:fetch])
        lexical_max = max(lexical_scores.values(), default=0.0)

        def _lexical_component(raw: float) -> float:
            if lexical_max <= 0.0:
                return 0.0
            return float(raw) / float(lexical_max) if lexical_max else 0.0

        threshold = max(0.0, min(1.0, float(min_similarity)))

        results: List[Dict[str, Any]] = []
        for doc_id in candidate_ids:
            entry = self.entries.get(doc_id)
            if not entry:
                continue
            vector_raw = float(vector_scores.get(doc_id, 0.0))
            vector_component = _rescale_inner_product(vector_raw)
            lexical_raw = float(lexical_scores.get(doc_id, 0.0))
            lexical_component = _lexical_component(lexical_raw)
            hybrid_score = ((1.0 - lex_weight) * vector_component) + (lex_weight * lexical_component)

            if threshold > 0.0:
                passes_vector = vector_component >= threshold
                passes_hybrid = hybrid_score >= threshold if use_lexical else False
                if not (passes_vector or passes_hybrid):
                    continue

            hit = {
                "doc_id": doc_id,
                "path": entry.get("path"),
                "ext": entry.get("ext"),
                "preview": entry.get("preview"),
                "size": entry.get("size"),
                "mtime": entry.get("mtime"),
                "ctime": entry.get("ctime"),
                "owner": entry.get("owner"),
                "drive": entry.get("drive"),
                "chunk_id": entry.get("chunk_id"),
                "chunk_count": entry.get("chunk_count"),
                "chunk_tokens": entry.get("chunk_tokens"),
                "vector_similarity": vector_component,
                "vector_raw": vector_raw,
                "lexical_score": lexical_component,
                "lexical_raw": lexical_raw,
                "score": hybrid_score,
                "hybrid_score": hybrid_score,
                "similarity": vector_component,
            }
            results.append(hit)

        if not results:
            return []

        vector_rank = {doc_id: idx for idx, doc_id in enumerate(vector_order)}
        results.sort(
            key=lambda item: (
                item.get("score", 0.0),
                item.get("vector_similarity", 0.0),
                -vector_rank.get(item.get("doc_id"), len(vector_rank)),
            ),
            reverse=True,
        )

        limit = min(len(results), max(top_k, fetch))
        return results[:limit]

    def _vector_scores(
        self,
        qvec: np.ndarray,
        fetch: int,
        *,
        use_ann: Optional[bool] = None,
    ) -> Tuple[Dict[int, float], List[int]]:
        scores: Dict[int, float] = {}
        order: List[int] = []
        ann_choice = use_ann
        if ann_choice is None and self._should_use_ann():
            ann_choice = True
        if ann_choice:
            scores, order = self._ann_scores(qvec, fetch)
            if scores:
                return scores, order
        elif ann_choice is False:
            pass
        self._ensure_faiss_index()
        if self.faiss_index is not None and faiss is not None:
            query = qvec.reshape(1, -1).astype(np.float32, copy=False)
            k = min(fetch, len(self.doc_ids))
            if k <= 0:
                return scores, order
            distances, ids = self.faiss_index.search(query, k)
            for score, doc_id in zip(distances[0], ids[0]):
                if doc_id < 0:
                    continue
                doc_id_int = int(doc_id)
                scores[doc_id_int] = float(score)
                order.append(doc_id_int)
            return scores, order

        self._ensure_matrix()
        if self.Z is None or self.Z.size == 0:
            return scores, order

        # NOTE: On some macOS Python builds (Accelerate + NumPy 2.0.x),
        # `matmul` can emit spurious RuntimeWarnings even with finite float32 inputs.
        # `np.dot` produces identical results without the noisy warnings.
        sims = np.dot(self.Z, qvec)
        limit = min(fetch, sims.shape[0])
        idx = np.argpartition(-sims, limit - 1)[:limit]
        idx = idx[np.argsort(-sims[idx])]
        for pos in idx:
            doc_id = self.doc_ids[pos]
            scores[doc_id] = float(sims[pos])
            order.append(doc_id)
        return scores, order

    def _should_use_ann(self) -> bool:
        backend = self._ann_backend()
        if backend is None:
            return False
        if len(self.doc_ids) < max(1, self.ann_threshold):
            return False
        self._ensure_ann_index()
        return self.ann_index is not None

    def _ensure_ann_index(self) -> None:
        if not self._ann_dirty:
            return
        backend = self._ann_backend()
        if backend is None or not self.doc_ids:
            self.ann_index = None
            self._ann_dirty = False
            self._ann_backend_active = None
            return
        if len(self.doc_ids) < max(1, self.ann_threshold):
            self.ann_index = None
            self._ann_dirty = False
            self._ann_backend_active = None
            return
        self._ensure_matrix()
        if self.Z is None or self.Z.size == 0:
            self.ann_index = None
            self._ann_dirty = False
            self._ann_backend_active = None
            return
        dim = self.Z.shape[1]

        if backend == "faiss":
            try:
                hnsw_index = faiss.IndexHNSWFlat(dim, max(8, int(self.ann_m)))
            except Exception:
                self.ann_index = None
                self._ann_dirty = False
                self._ann_backend_active = None
                return
            hnsw_index.hnsw.efConstruction = max(16, int(self.ann_ef_construction))
            hnsw_index.hnsw.efSearch = max(8, int(self.ann_ef_search))
            ids = np.asarray(self.doc_ids, dtype=np.int64)
            target_index = hnsw_index if hasattr(hnsw_index, "add_with_ids") else faiss.IndexIDMap(hnsw_index)
            try:
                target_index.add_with_ids(self.Z, ids)
            except Exception:
                target_index = faiss.IndexIDMap(hnsw_index)
                target_index.add_with_ids(self.Z, ids)
            self.ann_index = target_index
            self._ann_hnsw = hnsw_index
            self._ann_backend_active = "faiss"
            self._ann_dirty = False
            return

        try:
            index = hnswlib.Index(space="ip", dim=dim)
            index.init_index(
                max_elements=len(self.doc_ids),
                ef_construction=max(16, int(self.ann_ef_construction)),
                M=max(8, int(self.ann_m)),
            )
            ids = np.asarray(self.doc_ids, dtype=np.int64)
            index.add_items(self.Z.astype(np.float32, copy=False), ids)
            index.set_ef(max(8, int(self.ann_ef_search)))
        except Exception:
            self.ann_index = None
            self._ann_backend_active = None
            self._ann_dirty = False
            return

        self.ann_index = index
        self._ann_hnsw = None
        self._ann_backend_active = "hnswlib"
        self._ann_dirty = False

    def _ann_scores(self, qvec: np.ndarray, fetch: int) -> Tuple[Dict[int, float], List[int]]:
        self._ensure_ann_index()
        if self.ann_index is None:
            return {}, []
        k = min(len(self.doc_ids), max(fetch, self.ann_ef_search))
        if k <= 0:
            return {}, []
        backend = self._ann_backend_active or self._ann_backend()
        query_matrix = qvec.reshape(1, -1).astype(np.float32, copy=False)
        try:
            if backend == "faiss":
                hnsw_struct = None
                if hasattr(self.ann_index, "hnsw"):
                    hnsw_struct = self.ann_index.hnsw
                elif hasattr(self, "_ann_hnsw") and hasattr(self._ann_hnsw, "hnsw"):
                    hnsw_struct = self._ann_hnsw.hnsw
                if hnsw_struct is not None:
                    hnsw_struct.efSearch = max(self.ann_ef_search, fetch)
                _, ids = self.ann_index.search(query_matrix, k)
            elif backend == "hnswlib":
                try:
                    self.ann_index.set_ef(max(self.ann_ef_search, fetch))
                except Exception:
                    pass
                ids, _ = self.ann_index.knn_query(query_matrix, k=k)
            else:
                return {}, []
        except Exception:
            return {}, []

        scores: Dict[int, float] = {}
        order: List[int] = []
        if np.size(ids) == 0:
            return scores, order

        candidate_ids = ids[0] if ids.ndim > 1 else ids
        for doc_id in candidate_ids:
            if doc_id < 0:
                continue
            doc_id_int = int(doc_id)
            vec = self.embeddings.get(doc_id_int)
            if vec is None:
                continue
            raw = float(np.dot(vec, qvec))
            scores[doc_id_int] = raw
            order.append(doc_id_int)
            if len(order) >= fetch:
                break
        return scores, order

    def _lexical_scores(self, query_tokens: Optional[List[str]], fetch: int) -> Dict[int, float]:
        if not query_tokens:
            return {}
        self._ensure_lexical_index()
        if self.lexical_index is None:
            return {}
        try:
            scores = self.lexical_index.get_scores(query_tokens)
        except Exception:
            return {}
        scores_arr = np.asarray(scores, dtype=np.float32)
        if scores_arr.size == 0:
            return {}
        limit = min(fetch, scores_arr.shape[0])
        idx = np.argpartition(-scores_arr, limit - 1)[:limit]
        idx = idx[np.argsort(-scores_arr[idx])]
        result: Dict[int, float] = {}
        for pos in idx:
            score = float(scores_arr[pos])
            if score <= 0:
                continue
            doc_id = self.doc_ids[pos]
            result[doc_id] = score
        return result

    def remove_paths(self, paths: Iterable[str]) -> int:
        to_remove: List[int] = []
        for raw in paths:
            doc_id = self._path_to_id.pop(self._normalize_path(raw), None)
            if doc_id is not None:
                to_remove.append(doc_id)

        removed = 0
        for doc_id in to_remove:
            if self._remove_doc_id(doc_id):
                removed += 1

        if removed:
            self._rebuild_lists()
            self._matrix_dirty = True
            self._mark_faiss_dirty()
            self._mark_lexical_dirty()
        return removed

    def _remove_doc_id(self, doc_id: int) -> bool:
        if doc_id not in self.entries:
            return False
        self.entries.pop(doc_id, None)
        self.lexical_tokens.pop(doc_id, None)
        self.embeddings.pop(doc_id, None)
        if doc_id in self.doc_ids:
            self.doc_ids.remove(doc_id)
        return True

    def upsert(
        self,
        *,
        path: str,
        ext: str,
        embedding: np.ndarray,
        preview: str,
        size: Optional[int] = None,
        mtime: Optional[float] = None,
        ctime: Optional[float] = None,
        owner: Optional[str] = None,
        tokens: Optional[List[str]] = None,
    ) -> int:
        return self.upsert_batch([{
            "path": path,
            "ext": ext,
            "embedding": embedding,
            "preview": preview,
            "size": size,
            "mtime": mtime,
            "ctime": ctime,
            "owner": owner,
            "tokens": tokens,
        }])[0]

    def upsert_batch(self, items: List[Dict[str, Any]]) -> List[int]:
        """v1.1 Batch Upsert - optimized to rebuild lists only once."""
        result_ids = []
        
        for item in items:
            path = item["path"]
            doc_id = self._allocate_doc_id(path)
            normalized_path = self._normalize_path(path)
            self._path_to_id[normalized_path] = doc_id
            result_ids.append(doc_id)

            self.embeddings[doc_id] = self._normalize_vector(item["embedding"])
            entry = {
                "path": path,
                "ext": item["ext"],
                "preview": item["preview"],
                "size": item.get("size") or 0,
                "mtime": item.get("mtime") or 0.0,
                "ctime": item.get("ctime") or 0.0,
                "owner": item.get("owner") or "",
            }
            entry["preview"] = self._truncate_preview(entry.get("preview", ""))
            self.entries[doc_id] = entry
            
            tokens = item.get("tokens")
            self.lexical_tokens[doc_id] = list(tokens) if tokens is not None else self._tokenize_entry(entry)

            if doc_id not in self.doc_ids:
                self.doc_ids.append(doc_id)

        self._rebuild_lists()
        self._matrix_dirty = True
        self._mark_faiss_dirty()
        self._mark_lexical_dirty()
        return result_ids
