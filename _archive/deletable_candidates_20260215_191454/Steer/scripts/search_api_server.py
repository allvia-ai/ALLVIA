"""Lightweight FastAPI server exposing /api/search for the retriever."""
from __future__ import annotations

import sys
from pathlib import Path

import uvicorn

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from core.api.app_factory import create_app
from core.api.settings import Settings
from core.config.paths import CACHE_DIR, CORPUS_PATH, TOPIC_MODEL_PATH
from core.search.retriever import Retriever


_RETRIEVER: Retriever | None = None


def _get_retriever() -> Retriever:
    global _RETRIEVER
    if _RETRIEVER is None:
        _RETRIEVER = Retriever(
            model_path=TOPIC_MODEL_PATH,
            corpus_path=CORPUS_PATH,
            cache_dir=CACHE_DIR,
            auto_refresh=False,
        )
        # 초기 로딩 시 인덱스를 준비해 첫 요청 지연을 줄인다.
        try:
            _RETRIEVER.ready(rebuild=False, wait=True)
        except Exception:
            # 인덱스가 없으면 요청 시 다시 준비하도록 둔다.
            pass
    return _RETRIEVER


app = create_app(settings=Settings(STARTUP_LOAD=False), retriever_provider=_get_retriever)


def main() -> None:
    uvicorn.run(
        "scripts.search_api_server:app",
        host="127.0.0.1",
        port=8080,
        reload=False,
    )


if __name__ == "__main__":
    main()
