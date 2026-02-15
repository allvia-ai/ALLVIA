"""Hash/semantic drift heuristics for the local corpus."""

from __future__ import annotations

import csv
import json
import time
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import Dict, List, Optional, Tuple

try:
    import numpy as np
except Exception:  # pragma: no cover - optional dep
    np = None  # type: ignore[assignment]

try:
    import pandas as pd
except Exception:  # pragma: no cover - optional dep
    pd = None  # type: ignore[assignment]


@dataclass
class FileMeta:
    path: str
    size: int = 0
    mtime: float = 0.0
    doc_hash: str = ""
    file_hash: str = ""

    def signature(self) -> str:
        """Return a lightweight hash that can be compared across runs."""
        if self.file_hash:
            return self.file_hash
        if self.size > 0 or self.mtime > 0.0:
            return f"{self.size}:{int(round(self.mtime))}"
        return self.doc_hash or ""


@dataclass
class DriftReport:
    timestamp: float
    scan_rows: int
    corpus_rows: int
    new_files: List[str]
    missing_files: List[str]
    changed_files: List[str]
    hash_drift_ratio: float
    semantic_shift: float
    semantic_sample_size: int
    recommendations: List[str]
    reembed_candidates: List[str]

    def to_json(self) -> str:
        return json.dumps(asdict(self), ensure_ascii=False)


def _safe_int(value, default: int = 0) -> int:
    try:
        if value is None:
            return default
        return int(float(value))
    except Exception:
        return default


def _safe_float(value, default: float = 0.0) -> float:
    try:
        if value is None:
            return default
        return float(value)
    except Exception:
        return default


def _load_scan_meta(scan_csv: Path) -> Dict[str, FileMeta]:
    if not scan_csv.exists():
        return {}
    meta: Dict[str, FileMeta] = {}
    with scan_csv.open("r", encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f)
        for row in reader:
            allowed = str(row.get("allowed") or "").strip().lower()
            if allowed in {"0", "false", "no"}:
                continue
            path = str(row.get("path") or "").strip()
            if not path:
                continue
            meta[path] = FileMeta(
                path=path,
                size=_safe_int(row.get("size")),
                mtime=_safe_float(row.get("mtime")),
                doc_hash="",
                file_hash=str(row.get("hash") or row.get("file_hash") or "").strip(),
            )
    return meta


def _load_corpus_meta(corpus_path: Path) -> Dict[str, FileMeta]:
    if not corpus_path.exists() or pd is None:
        return {}
    try:
        try:
            df = pd.read_parquet(corpus_path, columns=["path", "size", "mtime", "doc_hash", "file_hash"])
        except Exception:
            df = pd.read_parquet(corpus_path)
    except Exception:
        return {}
    if df is None or df.empty or "path" not in df.columns:
        return {}

    columns = set(df.columns)
    has_size = "size" in columns
    has_mtime = "mtime" in columns
    has_hash = "doc_hash" in columns
    has_file_hash = "file_hash" in columns

    meta: Dict[str, FileMeta] = {}
    for idx in range(len(df)):
        path = str(df["path"].iloc[idx] or "").strip()
        if not path or path in meta:
            continue
        meta[path] = FileMeta(
            path=path,
            size=_safe_int(df["size"].iloc[idx]) if has_size else 0,
            mtime=_safe_float(df["mtime"].iloc[idx]) if has_mtime else 0.0,
            doc_hash=str(df["doc_hash"].iloc[idx] or "") if has_hash else "",
            file_hash=str(df["file_hash"].iloc[idx] or "") if has_file_hash else "",
        )
    return meta


def _semantic_shift(
    cache_dir: Optional[Path],
    baseline_path: Optional[Path],
    *,
    sample_size: int = 2048,
) -> Tuple[float, int]:
    if cache_dir is None or np is None:
        return 0.0, 0
    emb_path = cache_dir / "doc_embeddings.npy"
    if not emb_path.exists():
        return 0.0, 0
    try:
        embeddings = np.load(str(emb_path), mmap_mode="r")
    except Exception:
        return 0.0, 0
    if embeddings is None or embeddings.size == 0:
        return 0.0, 0

    embeddings = np.asarray(embeddings, dtype=np.float32)
    total = embeddings.shape[0]
    if sample_size and total > sample_size:
        # Uniform subsample to avoid loading everything into memory.
        idx = np.linspace(0, total - 1, sample_size, dtype=int)
        work = embeddings[idx]
    else:
        work = embeddings

    mean_vec = work.mean(axis=0)
    norm = float(np.linalg.norm(mean_vec)) if mean_vec is not None else 0.0
    if not norm:
        return 0.0, work.shape[0]
    mean_vec = mean_vec / norm

    baseline = _load_semantic_baseline(baseline_path)
    shift = _cosine_distance(mean_vec, baseline) if baseline is not None else 0.0
    _save_semantic_baseline(baseline_path, mean_vec)
    return float(shift), work.shape[0]


def _load_semantic_baseline(path: Optional[Path]) -> Optional["np.ndarray"]:
    if path is None or np is None or not path.exists():
        return None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        data = payload.get("mean")
        if not isinstance(data, list):
            return None
        arr = np.asarray(data, dtype=np.float32)
        norm = float(np.linalg.norm(arr))
        if norm:
            arr = arr / norm
        return arr
    except Exception:
        return None


def _save_semantic_baseline(path: Optional[Path], vector: "np.ndarray") -> None:
    if path is None or np is None or vector is None:
        return
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = {"mean": vector.tolist(), "updated_at": time.time()}
        path.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")
    except Exception:
        pass


def _cosine_distance(current: "np.ndarray", baseline: Optional["np.ndarray"]) -> float:
    if baseline is None or current is None:
        return 0.0
    if baseline.shape != current.shape:
        return 0.0
    similarity = float(np.dot(current, baseline))
    similarity = max(-1.0, min(1.0, similarity))
    return max(0.0, 1.0 - similarity)


def check_drift(
    scan_csv: Path,
    corpus_path: Path,
    *,
    cache_dir: Optional[Path] = None,
    log_path: Optional[Path] = None,
    alert_threshold: float = 0.1,
    semantic_baseline: Optional[Path] = None,
    semantic_threshold: float = 0.15,
    semantic_sample: int = 2048,
    policy_engine=None,
    policy_agent: str = "",
    log_policy_metadata: bool = False,
    cache_action: Optional[str] = None,
    policy_id: Optional[str] = None,
) -> DriftReport:
    """Compare scan CSV vs corpus metadata and compute semantic shift."""

    scan_meta = _load_scan_meta(scan_csv)
    corpus_meta = _load_corpus_meta(corpus_path)
    scan_paths = set(scan_meta.keys())
    corpus_paths = set(corpus_meta.keys())

    new_files = sorted(scan_paths - corpus_paths)
    missing_files = sorted(corpus_paths - scan_paths)

    changed_files: List[str] = []
    for path in sorted(scan_paths & corpus_paths):
        if scan_meta[path].signature() != corpus_meta[path].signature():
            changed_files.append(path)

    base = max(1, len(corpus_paths))
    ratio = round((len(new_files) + len(changed_files)) / base, 4)

    semantic_shift, semantic_sample_size = _semantic_shift(
        cache_dir,
        semantic_baseline,
        sample_size=max(256, semantic_sample),
    )

    recommendations: List[str] = []
    if ratio >= alert_threshold and (new_files or changed_files):
        recommendations.append("reembed_new")
    if changed_files:
        recommendations.append("reembed_changed")
    if missing_files:
        recommendations.append("cleanup_index")
    if not corpus_paths:
        recommendations.append("full_train")
    if semantic_shift >= semantic_threshold and semantic_sample_size:
        recommendations.append("semantic_review")

    reembed_candidates = (new_files + changed_files)[:200]

    report = DriftReport(
        timestamp=time.time(),
        scan_rows=len(scan_paths),
        corpus_rows=len(corpus_paths),
        new_files=new_files[:100],
        missing_files=missing_files[:100],
        changed_files=changed_files[:100],
        hash_drift_ratio=ratio,
        semantic_shift=round(float(semantic_shift), 4),
        semantic_sample_size=int(semantic_sample_size),
        recommendations=recommendations,
        reembed_candidates=reembed_candidates,
    )

    if log_path:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        with log_path.open("a", encoding="utf-8") as f:
            entry = json.loads(report.to_json())
            if log_policy_metadata:
                entry["policy_id"] = policy_id
                entry["policy_source"] = getattr(policy_engine, "source", None) if policy_engine else None
                entry["policy_agent"] = policy_agent
                entry["cache_action"] = cache_action
            f.write(json.dumps(entry, ensure_ascii=False))
            f.write("\n")

    return report
