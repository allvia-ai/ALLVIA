# models.py - Extracted from pipeline.py (ML models for embeddings and topic modeling)
"""Topic modeling and sentence transformer embedding models."""

from __future__ import annotations

import json
import math
import os
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import numpy as np

from core.config.paths import MODELS_DIR
from core.utils.cli_ui import Spinner, ProgressLine

# Lazy imports for optional deps
try:
    from sklearn.feature_extraction.text import TfidfVectorizer
    from sklearn.decomposition import TruncatedSVD
    from sklearn.cluster import MiniBatchKMeans
    from sklearn.pipeline import Pipeline
    from sklearn.preprocessing import FunctionTransformer
except Exception:
    TfidfVectorizer = TruncatedSVD = MiniBatchKMeans = Pipeline = FunctionTransformer = None

try:
    from sentence_transformers import SentenceTransformer
except Exception:
    SentenceTransformer = None

try:
    import joblib
except Exception:
    joblib = None

try:
    from tqdm import tqdm
except Exception:
    tqdm = None

from core.data_pipeline.embedder import AsyncSentenceEmbedder
from core.data_pipeline.evaluate import evaluate_embeddings

# Constants from pipeline.py
DEFAULT_N_COMPONENTS = 128
MODEL_TYPE_SENTENCE_TRANSFORMER = "sentence-transformer"
TOKEN_PATTERN = r"(?u)\b[가-힣a-zA-Z0-9_]{2,}\b"
EMBED_DTYPE_ENV = "INFOPILOT_EMBED_DTYPE"
_VALID_EMBED_DTYPES = {"auto", "fp16", "fp32"}


def _default_embed_model() -> str:
    import platform
    env_model = os.getenv("DEFAULT_EMBED_MODEL")
    if env_model:
        return env_model
    if platform.system() == "Darwin":
        return "models--intfloat--multilingual-e5-small"
    return "BAAI/bge-m3"


DEFAULT_EMBED_MODEL = _default_embed_model()


def _sanitize_embed_dtype(value: Optional[str]) -> Optional[str]:
    if not value:
        return None
    v = str(value).strip().lower()
    return v if v in _VALID_EMBED_DTYPES else None


def _normalize_hf_model_id(value: str) -> str:
    if not value:
        return value
    if value.startswith("models--"):
        parts = value[len("models--"):].split("--", 1)
        if len(parts) == 2:
            return f"{parts[0]}/{parts[1]}"
    return value


def _resolve_sentence_transformer_location(model_name: str) -> str:
    if not model_name:
        return model_name
    if Path(model_name).exists():
        return model_name
    base_dir = MODELS_DIR / "sentence_transformers"
    if base_dir.exists():
        direct = base_dir / model_name
        if direct.exists():
            return str(direct)
        cache_dir = base_dir / f"models--{model_name.replace('/', '--')}"
        snapshots = cache_dir / "snapshots"
        if snapshots.exists():
            candidates = sorted(snapshots.iterdir(), key=lambda item: item.stat().st_mtime, reverse=True)
            for candidate in candidates:
                if any(
                    (candidate / marker).exists()
                    for marker in ("config.json", "modules.json", "config_sentence_transformers.json")
                ):
                    return str(candidate)
    return model_name


def _resolve_kmeans_n_init() -> int:
    try:
        import sklearn
        major, minor = map(int, sklearn.__version__.split(".")[:2])
        if major > 1 or (major == 1 and minor >= 4):
            return "auto"  # type: ignore[return-value]
    except Exception:
        pass
    return 10


@dataclass
class TrainConfig:
    max_features: int = 50_000
    n_components: int = DEFAULT_N_COMPONENTS
    n_clusters: int = 30
    ngram_range: Tuple[int, int] = (1, 2)
    min_df: int = 2
    max_df: float = 0.8
    use_sentence_transformer: bool = True
    embedding_model: str = DEFAULT_EMBED_MODEL
    embedding_batch_size: int = 32
    async_embeddings: bool = True
    embedding_concurrency: int = 1
    embedding_dtype: str = "auto"
    # 대규모 코퍼스 처리용 청크/서브프로세스 임베딩 옵션
    embedding_chunk_size: int = 0  # 0이면 전체 한 번에
    embedding_chunk_start: int = 0  # chunk_size>0일 때 시작 청크 인덱스(포함)
    embedding_chunk_end: int = -1  # chunk_size>0일 때 끝 청크 인덱스(미포함, -1이면 끝까지)
    embedding_subprocess_fallback: bool = True


def default_train_config() -> TrainConfig:
    return TrainConfig()


def _resolve_embed_dtype(cfg: TrainConfig) -> str:
    env_raw = os.getenv(EMBED_DTYPE_ENV)
    env_value = _sanitize_embed_dtype(env_raw)
    if env_raw:
        if env_value is not None:
            print(f"⚙️ 임베딩 dtype 설정: {env_value} ({EMBED_DTYPE_ENV})", flush=True)
            return env_value
        print(f"⚠️ {EMBED_DTYPE_ENV}={env_raw!r} 값이 잘못되어 auto 모드로 유지합니다.", flush=True)
    cfg_value = _sanitize_embed_dtype(getattr(cfg, "embedding_dtype", None))
    return cfg_value or "auto"

class TopicModel:
    def __init__(self, cfg:TrainConfig):
        if any(x is None for x in (TfidfVectorizer, TruncatedSVD, MiniBatchKMeans, Pipeline)):
            raise RuntimeError("scikit-learn 필요. pip install scikit-learn joblib")
        self.cfg=cfg
        self.pipeline:Optional[Pipeline]=None
        self._kmeans_n_init = _resolve_kmeans_n_init()

    def fit(self, df, text_col="text"):
        texts=(df[text_col].fillna("").astype(str)).tolist()
        print("🧠 학습 준비: TF-IDF → SVD → KMeans", flush=True)
        n_docs = len(texts)
        if n_docs <= 0:
            raise ValueError("학습할 문서가 없습니다.")

        effective_min_df = max(1, int(self.cfg.min_df))
        if n_docs < effective_min_df:
            effective_min_df = 1

        effective_max_df: float | int = self.cfg.max_df
        if isinstance(effective_max_df, float) and 0.0 < effective_max_df <= 1.0:
            if int(effective_max_df * n_docs) < effective_min_df:
                effective_max_df = 1.0
            if n_docs < 10 and float(effective_max_df) < 1.0:
                effective_max_df = 1.0
        elif isinstance(effective_max_df, int):
            effective_max_df = max(effective_min_df, effective_max_df)

        spin=Spinner(prefix="  학습 중")
        spin.start()
        try:
            t0=time.time()
            tfidf = TfidfVectorizer(
                token_pattern=TOKEN_PATTERN,
                ngram_range=self.cfg.ngram_range,
                max_features=self.cfg.max_features,
                min_df=effective_min_df,
                max_df=effective_max_df,
            )
            try:
                X = tfidf.fit_transform(texts)
            except ValueError as exc:
                if "After pruning, no terms remain" in str(exc):
                    tfidf = TfidfVectorizer(
                        token_pattern=TOKEN_PATTERN,
                        ngram_range=self.cfg.ngram_range,
                        max_features=self.cfg.max_features,
                        min_df=1,
                        max_df=1.0,
                    )
                    X = tfidf.fit_transform(texts)
                else:
                    raise
            n_features = int(getattr(X, "shape", (0, 0))[1])
            if n_features <= 0:
                raise ValueError("TF-IDF 피처가 0개라 토픽 모델을 학습할 수 없습니다.")

            def _dense(matrix):
                return matrix.toarray() if hasattr(matrix, "toarray") else matrix

            # SVD는 n_components < n_features 조건을 만족해야 한다.
            # 단, 극소 코퍼스에서는 SVD가 실패할 수 있으므로 TF-IDF(dense)로 fallback한다.
            svd_components = min(int(self.cfg.n_components), max(1, n_features - 1))
            if TruncatedSVD is None or FunctionTransformer is None:
                raise RuntimeError("scikit-learn 필요. pip install scikit-learn joblib")
            if n_features <= 1:
                svd = FunctionTransformer(_dense, validate=False)
                Z = svd.fit_transform(X)
            else:
                svd = TruncatedSVD(n_components=svd_components, random_state=42)
                try:
                    Z = svd.fit_transform(X)
                except ValueError:
                    svd = FunctionTransformer(_dense, validate=False)
                    Z = svd.fit_transform(X)

            # KMeans는 n_clusters <= n_samples 조건을 만족해야 한다.
            n_samples = int(getattr(Z, "shape", (0, 0))[0])
            clusters = max(1, min(int(self.cfg.n_clusters), n_samples))
            kmeans = MiniBatchKMeans(
                n_clusters=clusters,
                random_state=42,
                batch_size=2048,
                n_init=self._kmeans_n_init,
            )
            kmeans.fit(Z)

            # Build a pipeline object for downstream predict/transform; steps are already fitted.
            self.pipeline = Pipeline(steps=[
                ("tfidf", tfidf),
                ("svd", svd),
                ("kmeans", kmeans),
            ])
            t1=time.time()
        finally:
            spin.stop()
        print(f"✅ 학습 완료 (docs={n_docs:,}, {t1-t0:.1f}s)", flush=True)
        return self

    def predict(self, df, text_col="text")->List[int]:
        texts=(df[text_col].fillna("").astype(str)).tolist()
        return self.pipeline.predict(texts)

    def transform(self, df, text_col="text"):
        texts=(df[text_col].fillna("").astype(str)).tolist()
        X=self.pipeline.named_steps["tfidf"].transform(texts)
        Z=self.pipeline.named_steps["svd"].transform(X)
        return Z

    def save(self, path:Path):
        if joblib is None: raise RuntimeError("joblib 필요")
        path.parent.mkdir(parents=True, exist_ok=True)
        joblib.dump({"cfg":self.cfg,"pipeline":self.pipeline}, path)


class SentenceBertModel:
    def __init__(self, cfg: TrainConfig):
        if SentenceTransformer is None:
            raise RuntimeError(
                "sentence-transformers 라이브러리가 필요합니다. pip install sentence-transformers"
            )
        self.cfg = cfg
        self.model_name = cfg.embedding_model or DEFAULT_EMBED_MODEL
        resolved = _resolve_sentence_transformer_location(self.model_name)
        if resolved == self.model_name:
            resolved = _normalize_hf_model_id(self.model_name)
        if resolved != self.model_name:
            print(f"🧠 Sentence-BERT 준비: {self.model_name} -> {resolved}", flush=True)
        else:
            print(f"🧠 Sentence-BERT 준비: {self.model_name}", flush=True)
        try:
            self._encoder = SentenceTransformer(resolved)
        except (RuntimeError, NotImplementedError) as exc:
            message = str(exc).lower()
            meta_issue = "meta tensor" in message or "to_empty" in message
            if meta_issue:
                print("⚠️ SentenceTransformer 로드 실패 → CPU 강제 시도", flush=True)
                try:
                    self._encoder = SentenceTransformer(resolved, device="cpu")
                except Exception as inner_exc:
                    raise RuntimeError(
                        "SentenceTransformer 초기화에 실패했습니다.\n"
                        "PyTorch를 README 권장 버전(torch 2.3.0, torchvision 0.18.0, torchaudio 2.3.0)으로 재설치해 주세요."
                    ) from inner_exc
            else:
                raise
        self.embedding_dim = int(self._encoder.get_sentence_embedding_dimension())
        self._target_dtype = (cfg.embedding_dtype or "auto").strip().lower()
        self._np_dtype = np.float16 if self._should_use_fp16() else np.float32
        self.cluster_model: Optional[MiniBatchKMeans] = None
        self.cluster_labels_: Optional[np.ndarray] = None
        self._kmeans_n_init = _resolve_kmeans_n_init()
        self._async_enabled = bool(getattr(self.cfg, "async_embeddings", False))
        self._async_threshold = max(512, int(self.cfg.embedding_batch_size) * 4)
        self._async_embedder = (
            AsyncSentenceEmbedder(
                self._encoder,
                batch_size=max(1, int(self.cfg.embedding_batch_size)),
                concurrency=max(1, int(getattr(self.cfg, "embedding_concurrency", 1))),
                target_dtype=self._target_dtype,
                device=self._encoder_device(),
            )
            if self._async_enabled
            else None
        )

    def _encoder_device(self) -> Optional[str]:
        device = getattr(self._encoder, "device", None)
        if device is None:
            device = getattr(self._encoder, "_target_device", None)
        if device is None:
            return None
        return str(device)

    def _should_use_fp16(self) -> bool:
        if self._target_dtype == "fp16":
            return True
        if self._target_dtype == "fp32":
            return False
        device = self._encoder_device()
        return bool(device and device.startswith("cuda"))

    def encode(self, texts: List[str], *, show_progress: bool = False) -> np.ndarray:
        if not texts:
            return np.zeros((0, self.embedding_dim), dtype=np.float32)
        use_async = (
            self._async_enabled
            and self._async_embedder is not None
            and len(texts) >= self._async_threshold
        )
        if use_async:
            try:
                return self._async_embedder.encode(texts)
            except Exception as exc:
                print(f"⚠️ Async 임베딩 실패 → 동기 모드로 재시도합니다: {exc}", flush=True)
        embeddings = self._encoder.encode(
            texts,
            batch_size=max(1, int(self.cfg.embedding_batch_size)),
            show_progress_bar=show_progress,
            convert_to_numpy=True,
            normalize_embeddings=False,
        )
        if isinstance(embeddings, list):
            embeddings = np.asarray(embeddings, dtype=np.float32)
        embeddings = np.asarray(embeddings, dtype=np.float32)
        if embeddings.dtype != self._np_dtype:
            embeddings = embeddings.astype(self._np_dtype, copy=False)
        return embeddings

    def fit(self, df, text_col: str = "text") -> np.ndarray:
        texts = (df[text_col].fillna("").astype(str)).tolist()
        show_progress = tqdm is not None and len(texts) > 1000
        embeddings = self.encode(texts, show_progress=show_progress)

        can_cluster = (
            MiniBatchKMeans is not None
            and self.cfg.n_clusters > 0
            and embeddings.shape[0] >= max(10, self.cfg.n_clusters)
        )
        if can_cluster:
            print("🔖 클러스터링: MiniBatchKMeans", flush=True)
            self.cluster_model = MiniBatchKMeans(
                n_clusters=self.cfg.n_clusters,
                random_state=42,
                batch_size=2048,
                n_init=self._kmeans_n_init,
            )
            self.cluster_model.fit(embeddings)
            try:
                labels = self.cluster_model.labels_
            except AttributeError:
                labels = self.cluster_model.predict(embeddings)
            self.cluster_labels_ = np.asarray(labels, dtype=np.int32)
        else:
            self.cluster_model = None
            self.cluster_labels_ = None
            if MiniBatchKMeans is None:
                print("⚠️ scikit-learn MiniBatchKMeans 미설치로 토픽 라벨링을 건너뜁니다.", flush=True)
            elif embeddings.shape[0] < max(10, self.cfg.n_clusters):
                print("ℹ️ 문서 수가 적어 토픽 클러스터링을 건너뜁니다.", flush=True)
        return embeddings

    def predict(self, embeddings: np.ndarray) -> np.ndarray:
        if self.cluster_model is None:
            raise RuntimeError("클러스터링 모델이 초기화되지 않았습니다.")
        labels = self.cluster_model.predict(embeddings)
        return np.asarray(labels, dtype=np.int32)

    def save(self, path: Path) -> None:
        if joblib is None:
            raise RuntimeError("joblib 필요. pip install joblib")
        path.parent.mkdir(parents=True, exist_ok=True)
        payload: Dict[str, Any] = {
            "version": 2,
            "model_type": MODEL_TYPE_SENTENCE_TRANSFORMER,
            "model_name": self.model_name,
            "embedding_dim": self.embedding_dim,
            "train_config": self.cfg,
        }
        if self.cluster_model is not None:
            payload["cluster_model"] = self.cluster_model
        joblib.dump(payload, path)


# =========================
# 청크 임베딩 (서브프로세스 GPU→CPU fallback)
# =========================
def _run_embed_chunk_subprocess(
    texts: List[str],
    cfg: TrainConfig,
    chunk_id: int,
    total_chunks: int,
) -> np.ndarray:
    """Embed a chunk of texts in a subprocess to isolate MPS OOM; CPU로 재시도."""
    root = Path(__file__).resolve().parents[2]
    cli_path = root / "scripts" / "pipeline" / "infopilot.py"
    if not cli_path.exists():
        raise RuntimeError(f"embed-chunk 명령을 찾을 수 없습니다: {cli_path}")

    with tempfile.TemporaryDirectory() as td:
        td_path = Path(td)
        input_path = td_path / "chunk.json"
        output_path = td_path / "embeddings.npy"
        input_path.write_text(json.dumps(texts, ensure_ascii=False), encoding="utf-8")

        base_cmd = [
            sys.executable,
            str(cli_path),
            "embed-chunk",
            "--input",
            str(input_path),
            "--output",
            str(output_path),
            "--model",
            cfg.embedding_model,
            "--batch-size",
            str(max(1, int(cfg.embedding_batch_size))),
            "--concurrency",
            str(max(1, int(cfg.embedding_concurrency))),
            "--dtype",
            cfg.embedding_dtype or "auto",
        ]
        # 청크 임베딩은 안정성을 위해 기본적으로 동기 모드로 강제한다.
        base_cmd.append("--no-async")

        def _run(cmd, env=None) -> int:
            proc = subprocess.run(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                env=env,
            )
            if proc.returncode != 0:
                stdout = (proc.stdout or b"").decode(errors="ignore")
                stderr = (proc.stderr or b"").decode(errors="ignore")
                print(
                    f"⚠️ 청크 임베딩 실패(chunk {chunk_id}/{total_chunks-1}, rc={proc.returncode})"
                    f"\nstdout: {stdout[:2000]}\nstderr: {stderr[:2000]}",
                    flush=True,
                )
            return proc.returncode

        rc = _run(base_cmd)
        if rc != 0 and cfg.embedding_subprocess_fallback:
            env = os.environ.copy()
            env["INFOPILOT_FORCE_CPU"] = "1"
            print(f"⚠️ chunk {chunk_id} → CPU로 재시도합니다.", flush=True)
            rc = _run(base_cmd, env=env)

        if rc != 0:
            raise RuntimeError(f"청크 임베딩 실패(chunk {chunk_id})")

        emb = np.load(output_path)
        if emb.dtype != np.float32:
            emb = emb.astype(np.float32, copy=False)
        return emb


def _chunked_sentence_embeddings(texts: List[str], cfg: TrainConfig) -> np.ndarray:
    chunk_size = max(1, int(cfg.embedding_chunk_size))
    start_chunk = max(0, int(getattr(cfg, "embedding_chunk_start", 0) or 0))
    end_chunk = int(getattr(cfg, "embedding_chunk_end", -1) or -1)
    total_chunks = math.ceil(len(texts) / chunk_size) if texts else 0
    embeddings_list: List[np.ndarray] = []
    progress = ProgressLine(total=max(1, total_chunks), label="Chunk 임베딩", update_every=1)

    for chunk_id, start_idx in enumerate(range(0, len(texts), chunk_size)):
        if chunk_id < start_chunk:
            continue
        if end_chunk >= 0 and chunk_id >= end_chunk:
            break
        chunk_texts = texts[start_idx : start_idx + chunk_size]
        emb = _run_embed_chunk_subprocess(chunk_texts, cfg, chunk_id, total_chunks)
        embeddings_list.append(emb)
        progress.update()

    progress.close()
    if not embeddings_list:
        return np.zeros((0, 0), dtype=np.float32)
    return np.concatenate(embeddings_list, axis=0)


def _fit_sentence_transformer_chunked(train_df, text_col: str, cfg: TrainConfig):
    """Chunk + subprocess 기반 임베딩 후 클러스터/메트릭 계산."""
    semantic_model = SentenceBertModel(cfg)
    texts = (train_df[text_col].fillna("").astype(str)).tolist()
    embeddings = _chunked_sentence_embeddings(texts, cfg)

    metrics: Dict[str, float] = {}
    labels: Optional[np.ndarray] = None

    can_cluster = (
        MiniBatchKMeans is not None
        and cfg.n_clusters > 0
        and embeddings.shape[0] >= max(10, max(1, cfg.n_clusters))
    )
    if can_cluster:
        print("🔖 클러스터링: MiniBatchKMeans", flush=True)
        cluster_model = MiniBatchKMeans(
            n_clusters=cfg.n_clusters,
            random_state=42,
            batch_size=2048,
            n_init=_resolve_kmeans_n_init(),
        )
        cluster_model.fit(embeddings)
        try:
            labels = cluster_model.labels_
        except AttributeError:
            labels = cluster_model.predict(embeddings)
        labels = np.asarray(labels, dtype=np.int32)
        semantic_model.cluster_model = cluster_model
        semantic_model.cluster_labels_ = labels
        metrics = evaluate_embeddings(embeddings, labels, topk=min(5, max(1, embeddings.shape[0] - 1)))
    else:
        semantic_model.cluster_model = None
        semantic_model.cluster_labels_ = None
        if MiniBatchKMeans is None:
            print("⚠️ scikit-learn MiniBatchKMeans 미설치로 토픽 라벨링을 건너뜁니다.", flush=True)
        elif embeddings.shape[0] < max(10, max(1, cfg.n_clusters)):
            print("ℹ️ 문서 수가 적어 토픽 클러스터링을 건너뜁니다.", flush=True)

    return embeddings, semantic_model, metrics
