# scripts/pipeline/infopilot_cli/model_utils.py
from __future__ import annotations

import os
import joblib
from pathlib import Path
from typing import Optional, Tuple
from core.config.paths import MODELS_DIR
from core.infra.models import ModelManager
from core.data_pipeline.pipeline import DEFAULT_EMBED_MODEL

try:
    from sentence_transformers import SentenceTransformer
except ImportError:
    SentenceTransformer = None

_SENTENCE_ENCODER_MANAGER: Optional[ModelManager] = None

def _get_sentence_encoder_manager() -> ModelManager:
    global _SENTENCE_ENCODER_MANAGER
    if _SENTENCE_ENCODER_MANAGER is None:
        def _load(model_name: str):
            if SentenceTransformer is None:
                raise RuntimeError("sentence-transformers 패키지가 필요합니다. pip install sentence-transformers")
            local_dir = MODELS_DIR / "sentence_transformers" / model_name
            if local_dir.exists():
                return SentenceTransformer(str(local_dir))
            return SentenceTransformer(model_name)

        _SENTENCE_ENCODER_MANAGER = ModelManager(loader=_load)
    return _SENTENCE_ENCODER_MANAGER # type: ignore

def disable_mps_for_inference(reason: str = "") -> None:
    """Force CPU fallback for sentence-transformers when MPS OOM/errors occur."""
    if os.getenv("PYTORCH_MPS_ENABLE", "1") == "0":
        return
    os.environ["PYTORCH_MPS_ENABLE"] = "0"
    os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")
    if reason:
        print(f"⚠️ MPS 비활성화 후 CPU로 재시도합니다: {reason}", flush=True)

def load_sentence_encoder(model_path: Path) -> Tuple[Optional[SentenceTransformer], int, str]:
    model_name = DEFAULT_EMBED_MODEL
    batch_size = 32

    if joblib is not None and model_path.exists():
        try:
            payload = joblib.load(model_path)
            model_name = payload.get("model_name", model_name)
            cfg = payload.get("train_config")
            if cfg and hasattr(cfg, "embedding_batch_size"):
                batch_size = int(getattr(cfg, "embedding_batch_size", batch_size) or batch_size)
        except Exception as exc:
            print(f"⚠️ 임베딩 모델 메타 로드 실패 → 기본값 사용({model_name}): {exc}")

    try:
        manager = _get_sentence_encoder_manager()
    except RuntimeError as exc:
        print(f"⚠️ SentenceTransformer 로더 초기화 실패: {exc}")
        return None, batch_size, model_name

    try:
        encoder = manager.get(model_name)
    except Exception as exc:
        print(f"⚠️ SentenceTransformer 모델 로드 실패({model_name}): {exc}")
        # MPS OOM/호환 문제일 수 있으니 CPU 강제 후 한 번만 재시도
        try:
            disable_mps_for_inference(str(exc))
            # manager는 캐시를 갖고 있어 재생성이 필요하다
            global _SENTENCE_ENCODER_MANAGER
            _SENTENCE_ENCODER_MANAGER = None
            manager = _get_sentence_encoder_manager()
            encoder = manager.get(model_name)
        except Exception as exc_retry:
            print(f"⚠️ CPU 강제 재시도도 실패({model_name}): {exc_retry}")
            return None, batch_size, model_name
    return encoder, batch_size, model_name
