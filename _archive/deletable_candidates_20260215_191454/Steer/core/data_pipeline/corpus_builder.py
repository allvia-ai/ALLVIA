# corpus_builder.py - Extracted from pipeline.py (Document corpus building)
"""Build document corpus from file rows with text extraction and translation."""

from __future__ import annotations

import os
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Set, Tuple

from core.utils.cli_ui import ProgressLine

# Optional dependencies
try:
    import pandas as pd
except Exception:
    pd = None

try:
    from tqdm import tqdm
except Exception:
    tqdm = None

try:
    from deep_translator import GoogleTranslator
except Exception:
    GoogleTranslator = None

# Import from extractors
from core.data_pipeline.extractors import EXT_MAP

# Import PARQUET_ENGINE detection
PARQUET_ENGINE: Optional[str] = None
if pd is not None:
    import importlib
    for candidate in ("fastparquet", "pyarrow"):
        try:
            importlib.import_module(candidate)
            PARQUET_ENGINE = candidate
            break
        except ImportError:
            continue


def _hash_text(text: str) -> str:
    import hashlib
    return hashlib.sha1(text.encode("utf-8", "ignore")).hexdigest()


def _sanitize_embed_dtype(value: Optional[str]) -> Optional[str]:
    _VALID_EMBED_DTYPES = {"auto", "fp16", "fp32"}
    if not value:
        return None
    v = str(value).strip().lower()
    return v if v in _VALID_EMBED_DTYPES else None


def _prepare_text_frame(df: "pd.DataFrame") -> None:
    """Placeholder - actual implementation in pipeline.py"""
    pass


@dataclass
class ExtractRecord:
    path: str
    ext: str
    ok: bool
    text: str
    text_original: str
    meta: Dict[str, Any]
    size: Optional[int] = None
    mtime: Optional[float] = None
    ctime: Optional[float] = None
    owner: Optional[str] = None
    doc_hash: str = ""
    file_hash: str = ""

class CorpusBuilder:
    MAX_TRANSLATE_CHARS = 4000

    def __init__(
        self,
        max_text_chars: int = 200_000,
        progress: bool = True,
        translate: bool = False,
        max_workers: Optional[int] = None,
        target_embed_dtype: str = "auto",
    ):
        self.max_text_chars = max_text_chars
        self.progress = progress
        self.translate = translate
        self.target_embed_dtype = _sanitize_embed_dtype(target_embed_dtype) or "auto"
        self.translator = None
        if translate:
            if GoogleTranslator is None:
                print("⚠️ 경고: 'deep-translator' 라이브러리를 찾을 수 없어 번역 기능이 비활성화됩니다.")
                print("   해결: pip install deep-translator")
            else:
                try:
                    self.translator = GoogleTranslator(source="auto", target="en")
                except Exception as exc:
                    print("⚠️ 경고: 번역기 초기화에 실패해 번역 기능이 비활성화됩니다.")
                    print(f"   상세: {exc}")
        worker_default = max(1, min(8, (os.cpu_count() or 4)))
        self.max_workers = max_workers or worker_default
        if self.translate:
            # 번역 시 외부 API 호출이 순차 처리되도록 워커 1개만 사용
            self.max_workers = 1

    def build(self, file_rows: List[Dict[str, Any]]):
        if pd is None:
            raise RuntimeError("pandas 필요. pip install pandas")

        total = len(file_rows)
        if total == 0:
            print("ℹ️ 신규/변경 문서가 없어 추출을 건너뜁니다.", flush=True)
            empty = pd.DataFrame(columns=list(ExtractRecord.__annotations__.keys()))
            empty.attrs["target_embed_dtype"] = self.target_embed_dtype
            return empty

        use_tqdm = self.progress and tqdm is not None
        desc = "📥 Extract & Translate" if self.translate else "📥 Extract"
        bar = tqdm(total=total, desc=desc, unit="file") if use_tqdm else ProgressLine(total, "extracting", update_every=max(1, total // 100 or 1))

        recs: List[Optional[ExtractRecord]] = [None] * total
        with ThreadPoolExecutor(max_workers=max(1, self.max_workers)) as executor:
            future_map = {
                executor.submit(self._extract_one, file_rows[idx]): idx
                for idx in range(total)
            }
            for future in as_completed(future_map):
                idx = future_map[future]
                try:
                    rec = future.result()
                except Exception as exc:
                    row = file_rows[idx]
                    rec = ExtractRecord(
                        path=row.get("path", ""),
                        ext=row.get("ext", ""),
                        ok=False,
                        text="",
                        text_original="",
                        meta={"error": f"extract crash: {exc}"},
                        size=row.get("size"),
                        mtime=row.get("mtime"),
                        ctime=row.get("ctime"),
                        owner=row.get("owner"),
                    )
                recs[idx] = rec
                if use_tqdm:
                    bar.update(1)
                else:
                    bar.update(1)

        if use_tqdm and bar is not None:
            bar.close()
        elif not use_tqdm:
            bar.close()

        records = [r.__dict__ for r in recs if r is not None]
        df = pd.DataFrame(records)
        df.attrs["target_embed_dtype"] = self.target_embed_dtype
        _prepare_text_frame(df)
        ok = int(df["ok"].sum()) if len(df) > 0 else 0
        fail = int((~df["ok"]).sum()) if len(df) > 0 else 0
        print(f"✅ Extract 완료: ok={ok}, fail={fail}", flush=True)
        return df

    def _extract_one(self, row: Dict[str, Any]) -> ExtractRecord:
        path = Path(row["path"])
        ext = path.suffix.lower()
        file_hash = str(row.get("hash") or row.get("file_hash") or "").strip()
        mask_pii = bool(row.get("policy_mask_pii") or row.get("mask_pii"))
        ex = EXT_MAP.get(ext)
        if not ex:
            return ExtractRecord(
                str(path),
                ext,
                False,
                "",
                "",
                {"error": "no extractor"},
                row.get("size"),
                row.get("mtime"),
                row.get("ctime"),
                row.get("owner"),
                "",
                file_hash,
            )
        try:
            out = ex.extract(path)
            raw_text = (out.get("text", "") or "")[:self.max_text_chars]
            doc_hash = _hash_text(raw_text)

            if mask_pii and raw_text.strip():
                # Reuse meeting pipeline masking rules for privacy-preserving corpora.
                from core.agents.meeting.pii import mask_text as _mask_text

                original_text = _mask_text(raw_text)
            else:
                original_text = raw_text

            text_for_model = original_text
            if self.translator and original_text.strip():
                text_for_model = self._translate_text(original_text, context=path.name)

            return ExtractRecord(
                str(path),
                ext,
                bool(out.get("ok", False)),
                text_for_model,
                original_text,
                out.get("meta", {}),
                row.get("size"),
                row.get("mtime"),
                row.get("ctime"),
                row.get("owner"),
                doc_hash,
                file_hash,
            )
        except Exception as e:
            return ExtractRecord(
                str(path),
                ext,
                False,
                "",
                "",
                {"error": f"extract crash: {e}"},
                row.get("size"),
                row.get("mtime"),
                row.get("ctime"),
                row.get("owner"),
                "",
                file_hash,
            )

    def _translate_text(self, text: str, *, context: str) -> str:
        if not self.translator:
            return text
        chunks = self._chunk_text(text, self.MAX_TRANSLATE_CHARS)
        try:
            translated_chunks: List[str] = []
            for chunk in chunks:
                translated = self.translator.translate(chunk)
                translated_chunks.append(self._translated_text(translated, fallback=chunk))
            joined = "\n".join(translated_chunks).strip()
            return joined or text
        except Exception as exc:
            self._log_warning(f"\n[경고] '{context}' 번역 실패. 원본 텍스트 사용. 오류: {exc}")
            return text

    @staticmethod
    def _translated_text(result: Any, *, fallback: str) -> str:
        if isinstance(result, str):
            return result
        text = getattr(result, "text", None)
        if isinstance(text, str) and text.strip():
            return text
        return fallback

    @staticmethod
    def _chunk_text(text: str, limit: int) -> List[str]:
        if len(text) <= limit:
            return [text]
        chunks: List[str] = []
        start = 0
        length = len(text)
        while start < length:
            end = min(length, start + limit)
            split = end
            if end < length:
                for sep in ("\n\n", "\n", " "):
                    idx = text.rfind(sep, start, end)
                    if idx != -1 and idx > start:
                        split = idx + len(sep)
                        break
            if split <= start:
                split = end
            chunks.append(text[start:split])
            start = split
        return chunks

    def _log_warning(self, message: str) -> None:
        if tqdm and self.progress:
            tqdm.write(message)
        else:
            print(message)

    @staticmethod
    def save(df, out_path:Path):
        out_path.parent.mkdir(parents=True, exist_ok=True)
        ext = out_path.suffix.lower()
        if ext == ".parquet":
            engine_kwargs = {}
            engine_label = PARQUET_ENGINE or "auto"
            if PARQUET_ENGINE:
                engine_kwargs["engine"] = PARQUET_ENGINE
            try:
                df.to_parquet(out_path, index=False, **engine_kwargs)
                print(f"✅ Parquet 저장({engine_label}): {out_path}")
                return
            except Exception as e:
                csv_path = out_path.with_suffix(".csv")
                df.to_csv(csv_path, index=False, encoding="utf-8")
                print(
                    f"⚠️ Parquet 엔진 실패({engine_label}) → CSV로 저장: {csv_path}\n"
                    f"   상세: {e}"
                )
                return
        df.to_csv(out_path, index=False, encoding="utf-8")
        print(f"✅ CSV 저장: {out_path}")


def _load_existing_corpus(path: Path) -> Optional["pd.DataFrame"]:
    if pd is None:
        return None
    candidates = [path]
    suffix = path.suffix.lower()
    if suffix == ".parquet":
        candidates.append(path.with_suffix(".csv"))
    elif suffix == ".csv":
        candidates.append(path.with_suffix(".parquet"))

    for candidate in candidates:
        if not candidate.exists():
            continue
        try:
            if candidate.suffix.lower() == ".parquet":
                engine_kwargs = {}
                if PARQUET_ENGINE:
                    engine_kwargs["engine"] = PARQUET_ENGINE
                return pd.read_parquet(candidate, **engine_kwargs)
            return pd.read_csv(candidate)
        except Exception as exc:
            engine_label = PARQUET_ENGINE or "auto"
            print(
                f"⚠️ 기존 코퍼스 로드 실패 ({candidate}, engine={engine_label}): {exc}",
                flush=True,
            )
    return None


def _is_cache_fresh(cached: Dict[str, Any], row: Dict[str, Any]) -> bool:
    if not cached.get("ok"):
        return False
    if not cached.get("text"):
        return False
    try:
        cached_size = int(cached.get("size", -1))
        row_size = int(row.get("size", -1))
    except (TypeError, ValueError):
        return False
    if cached_size != row_size:
        return False
    try:
        cached_mtime = float(cached.get("mtime", 0.0))
        row_mtime = float(row.get("mtime", 0.0))
    except (TypeError, ValueError):
        return False
    if abs(cached_mtime - row_mtime) > 1.0:
        return False
    return True


def _split_cache(
    file_rows: List[Dict[str, Any]],
    existing_df: Optional["pd.DataFrame"],
    *,
    force_paths: Optional[Set[str]] = None,
) -> Tuple[List[Dict[str, Any]], Optional["pd.DataFrame"]]:
    if pd is None or existing_df is None or existing_df.empty or "path" not in existing_df.columns:
        return list(file_rows), None

    meta_map: Dict[str, Dict[str, Any]] = {}
    seen_paths: Set[str] = set()
    for rec in existing_df[["path", "size", "mtime"]].drop_duplicates(subset=["path"]).to_dict(orient="records"):
        key = str(rec.get("path") or "")
        if key:
            meta_map[key] = rec
            seen_paths.add(key)

    to_process: List[Dict[str, Any]] = []
    process_paths: Set[str] = set()
    for row in file_rows:
        path = str(row.get("path") or "")
        force = force_paths is not None and path in force_paths
        cached = meta_map.get(path)
        if force or not cached or not _is_cache_fresh(cached, row):
            to_process.append(row)
            if path:
                process_paths.add(path)

    if not process_paths:
        return to_process, existing_df.copy()

    mask = ~existing_df["path"].astype(str).isin(process_paths)
    remainder = existing_df[mask].copy()
    return to_process, remainder


def _collect_existing_rows(
    existing_df: Optional["pd.DataFrame"],
    target_paths: Set[str],
) -> Optional["pd.DataFrame"]:
    if pd is None or existing_df is None or existing_df.empty or not target_paths:
        return None
    mask = existing_df["path"].astype(str).isin({str(p) for p in target_paths})
    subset = existing_df[mask].copy()
    return subset if not subset.empty else None
