"""Edge-friendly corpus exporter and micro server."""
from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from pathlib import Path
from typing import Any, Dict, List

try:
    import pandas as pd
except Exception as exc:  # pragma: no cover - pandas is required
    raise SystemExit("pandas가 필요합니다. `pip install pandas` 후 다시 시도하세요.") from exc

try:
    from fastapi import FastAPI, HTTPException
    import uvicorn
except Exception:  # pragma: no cover - optional dependency for serve mode
    FastAPI = None  # type: ignore[assignment]
    uvicorn = None  # type: ignore[assignment]


def _resolve_text_column(df: "pd.DataFrame") -> str:
    if "text_model" in df.columns:
        return "text_model"
    if "text" in df.columns:
        return "text"
    raise ValueError("코퍼스에 text/text_model 컬럼이 없습니다.")


def _load_corpus(path: Path) -> "pd.DataFrame":
    if not path.exists():
        raise FileNotFoundError(f"코퍼스 파일을 찾을 수 없습니다: {path}")
    if path.suffix.lower() == ".parquet":
        return pd.read_parquet(path)
    if path.suffix.lower() == ".csv":
        return pd.read_csv(path)
    raise ValueError(f"지원하지 않는 형식: {path.suffix}")


def export_corpus(corpus_path: Path, database: Path, *, force: bool = False) -> None:
    df = _load_corpus(corpus_path)
    text_col = _resolve_text_column(df)
    preview = df["preview"] if "preview" in df.columns else df[text_col].astype(str).str[:200]
    topic = df["topic"].astype(str) if "topic" in df.columns else ""
    updated = df["mtime"] if "mtime" in df.columns else 0.0
    chunk_id = df["chunk_id"] if "chunk_id" in df.columns else 1

    if database.exists() and force:
        database.unlink()
    conn = sqlite3.connect(str(database))
    with conn:
        conn.execute(
            """
            CREATE TABLE IF NOT EXISTS documents (
                path TEXT,
                chunk_id INTEGER,
                text TEXT,
                preview TEXT,
                topic TEXT,
                updated_at REAL,
                PRIMARY KEY(path, chunk_id)
            )
            """
        )
        conn.execute("DELETE FROM documents")
        rows = [
            (
                str(row.path),
                int(chunk_id.iloc[idx]) if len(chunk_id) > idx else 1,
                str(getattr(row, text_col) or ""),
                str(preview.iloc[idx] or ""),
                str(topic.iloc[idx] or ""),
                float(updated.iloc[idx]) if len(updated) > idx else 0.0,
            )
            for idx, row in enumerate(df.itertuples())
        ]
        conn.executemany(
            "INSERT OR REPLACE INTO documents(path, chunk_id, text, preview, topic, updated_at) VALUES(?,?,?,?,?,?)",
            rows,
        )
    conn.close()
    print(f"✅ SQLite 코퍼스 저장 완료 ({len(rows)} rows) → {database}")


class EdgeCorpus:
    def __init__(self, database: Path) -> None:
        if not database.exists():
            raise FileNotFoundError(f"데이터베이스를 찾을 수 없습니다: {database}")
        self.database = database

    def search(self, query: str, limit: int) -> List[Dict[str, Any]]:
        terms = query.strip()
        if not terms:
            return []
        with sqlite3.connect(str(self.database)) as conn:
            conn.row_factory = sqlite3.Row
            rows = conn.execute(
                "SELECT path, preview, topic FROM documents WHERE text LIKE ? ORDER BY updated_at DESC LIMIT ?",
                (f"%{terms}%", max(1, limit)),
            ).fetchall()
        return [dict(row) for row in rows]


def serve_database(database: Path, host: str, port: int) -> None:
    if FastAPI is None or uvicorn is None:
        raise SystemExit("fastapi 및 uvicorn 모듈이 필요합니다. `pip install fastapi uvicorn` 후 다시 시도하세요.")
    corpus = EdgeCorpus(database)
    app = FastAPI(title="Edge Adapter", version="1.0")

    @app.get("/ping")
    def ping() -> Dict[str, Any]:
        return {"status": "ok"}

    @app.get("/search")
    def search(q: str, limit: int = 5) -> Dict[str, Any]:
        items = corpus.search(q, limit)
        if not items:
            raise HTTPException(status_code=404, detail="결과가 없습니다.")
        return {"hits": items}

    uvicorn.run(app, host=host, port=port)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="코퍼스를 Edge/모바일 친화적 SQLite로 변환")
    sub = parser.add_subparsers(dest="command", required=True)

    export_p = sub.add_parser("export", help="corpus → SQLite 변환")
    export_p.add_argument("--corpus", default="data/corpus.parquet", help="출력할 코퍼스 경로")
    export_p.add_argument("--database", default="data/edge_corpus.sqlite", help="생성될 SQLite 파일")
    export_p.add_argument("--force", action="store_true", help="존재 시 삭제 후 재생성")
    export_p.set_defaults(func=lambda args: export_corpus(Path(args.corpus), Path(args.database), force=args.force))

    serve_p = sub.add_parser("serve", help="간단한 검색 서버 실행")
    serve_p.add_argument("--database", default="data/edge_corpus.sqlite", help="SQLite 파일 경로")
    serve_p.add_argument("--host", default="127.0.0.1")
    serve_p.add_argument("--port", type=int, default=9090)
    serve_p.set_defaults(func=lambda args: serve_database(Path(args.database), args.host, args.port))

    return parser


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
