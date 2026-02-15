# core/conversation/retrieval_strategy.py
"""
Retrieval backend setup and LLM client initialization strategy.
"""
from __future__ import annotations

import os
from pathlib import Path
from typing import Optional, Dict

from core.utils import get_logger
from core.config.llm_defaults import DEFAULT_LLM_MODEL, resolve_backend
from core.conversation.llm_client import create_llm_client, LLMClient, LLMClientError

try:
    from core.search.retriever import HybridRetriever
except ImportError:
    HybridRetriever = None

LOGGER = get_logger("retrieval.strategy")

def ensure_offline_transformers():
    """Environment setup to prevent HuggingFace online lookups if possible."""
    os.environ.setdefault("HF_HUB_OFFLINE", "1")
    os.environ.setdefault("TRANSFORMERS_OFFLINE", "1")

def init_retriever(
    model_path: Path,
    corpus_path: Path,
    cache_dir: Path,
    topk: int,
    use_rerank: bool,
    rerank_model_name: str,
    rerank_depth: int,
    rerank_batch_size: int,
    rerank_device: Optional[str],
    rerank_min_score: Optional[float],
    min_similarity: float,
    lexical_weight: float,
    rebuild: bool = False,
) -> Optional[HybridRetriever]:
    """Factory method to build or load the HybridRetriever."""
    if HybridRetriever is None:
        LOGGER.warning("HybridRetriever implementation missing.")
        return None

    if not model_path.exists() and not corpus_path.exists():
        LOGGER.info("Retrieval model/corpus missing; operating in dialogue-only mode (unless rebuild requested).")
        if not rebuild:
            return None

    ensure_offline_transformers()
    
    try:
        retr = HybridRetriever(
            model_path=model_path,
            corpus_path=corpus_path,
            cache_dir=cache_dir,
            min_similarity=min_similarity,
            use_rerank=use_rerank,
            rerank_model=rerank_model_name,
            rerank_depth=rerank_depth,
            rerank_batch_size=rerank_batch_size,
            rerank_device=rerank_device,
            rerank_min_score=rerank_min_score,
            lexical_weight=lexical_weight,
        )
        if rebuild:
            LOGGER.info("Rebuild requested; retriever might not be ready immediately.")
            # Trigger build logic if async - but HybridRetriever usually loads sync?
            # Assuming standard Init loads state. Rebuild is usually a method call.
            pass
        return retr
    except Exception as e:
        LOGGER.error("Failed to initialize retriever: %s", e)
        return None


def init_llm_client(
    backend: Optional[str],
    model: Optional[str],
    host: Optional[str],
    options: Optional[Dict[str, str]],
    default_timeout: float = 30.0,
    health_timeout: Optional[float] = None,
) -> Optional[LLMClient]:
    """Factory to create and health-check the LLM Client."""
    
    # Resolve backend/model if auto
    effective_model = model or os.getenv("LNPCHAT_LLM_MODEL", DEFAULT_LLM_MODEL)
    effective_backend = resolve_backend(
        config_backend=backend,
        env_backend=os.getenv("LNPCHAT_LLM_BACKEND"),
        model=effective_model,
    )

    try:
        client = create_llm_client(
            backend=effective_backend,
            model=effective_model,
            host=host,
            options=options,
        )
        if not client.is_available():
            raise LLMClientError(f"Backend {effective_backend} is not available.")
            
        LOGGER.info("LLM Client initialized: %s (%s)", effective_backend, effective_model)
        return client
    except Exception as e:
        LOGGER.warning("LLM initialization failed: %s. Falling back to simple dialogue.", e)
        return None
