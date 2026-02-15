# scripts/pipeline/infopilot_cli/watch.py
"""
Watch command module.
Refactored to delegate logic to `watchers.py` and `pipeline_runner.py`.
"""
from __future__ import annotations

import time
import threading
import queue
from pathlib import Path

from watchdog.observers import Observer
from sentence_transformers import SentenceTransformer

from core.config.paths import CACHE_DIR, CORPUS_PATH
from core.policy.engine import PolicyEngine

from .watchers import WatchEventHandler, PolicyEventHandler
from .pipeline_runner import IncrementalPipeline, load_vector_index, sync_scan_csv, watch_loop

__all__ = [
    "IncrementalPipeline",
    "PolicyEventHandler",
    "WatchEventHandler",
    "load_vector_index",
    "sync_scan_csv",
    "watch_loop",
    "cmd_watch",
]

def cmd_watch(args, knowledge_agent: str):
    """
    Run the incremental indexing watcher.
    """
    # 1. Setup paths
    output_root = Path(args.output_root).expanduser()
    scan_csv = output_root / "scan_results.csv"  # Default convention? Or args?
    # Actually args.scan_csv? Or inferred.
    # In scan command: output_csv = output_root / "scan_results.csv"
    if hasattr(args, "output_csv") and args.output_csv:
        scan_csv = Path(args.output_csv)
    
    # 2. Load policy
    # We borrow loader logic from policy.py if needed, or simple load
    policy_path = getattr(args, "policy", None)
    if policy_path:
        policy_path = Path(policy_path)
    
    policy_engine = None
    if policy_path and policy_path.exists():
        try:
            policy_engine = PolicyEngine.from_file(policy_path)
            print(f"📜 정책 로드: {policy_path}")
        except Exception as e:
            print(f"⚠️ 정책 로드 실패: {e}")

    # 3. Initialize Encoder
    model_name = getattr(args, "model_name", "all-MiniLM-L6-v2")
    print(f"🔌 Encoder 로딩: {model_name}...")
    encoder = SentenceTransformer(model_name)

    # 4. Pipeline Context
    pipeline = IncrementalPipeline(
        encoder=encoder,
        batch_size=getattr(args, "batch_size", 32),
        scan_csv=scan_csv,
        corpus_path=CORPUS_PATH, # Or args.corpus
        cache_dir=CACHE_DIR,     # Or args.cache
        translate=getattr(args, "translate", False),
        policy_engine=policy_engine,
        policy_path=policy_path,
        roots=[Path(r) for r in getattr(args, "target", [])],
        agent=knowledge_agent,
    )

    # 5. Queue & Observer
    event_queue = queue.Queue()
    stop_event = threading.Event()
    debounce = float(getattr(args, "debounce", 1.0))

    observer = Observer()
    
    # Watch targets
    targets = getattr(args, "target", [])
    if not targets:
        # If no targets, maybe rely on policy?
        # But watcher needs explicit directories usually.
        # Fallback to current dir?
        targets = ["."]
    
    for target in targets:
        p = Path(target).expanduser().resolve()
        if not p.exists():
            print(f"⚠️ 경로 없음: {p}")
            continue
        print(f"👀 감시 시작: {p}")
        handler = WatchEventHandler(event_queue, str(p))
        observer.schedule(handler, str(p), recursive=True)

    if policy_path and policy_path.exists():
        # Watch policy file directory? Or just the file?
        # Watchdog watches directories.
        policy_dir = policy_path.parent
        phandler = PolicyEventHandler(event_queue, str(policy_path))
        observer.schedule(phandler, str(policy_dir), recursive=False)

    observer.start()
    print("🚀 감시 루프 시작 (Ctrl+C로 종료)...")

    try:
        watch_loop(event_queue, pipeline, stop_event, debounce)
    except KeyboardInterrupt:
        print("\n🛑 중지 요청...")
        stop_event.set()
    finally:
        observer.stop()
        observer.join()
        print("👋 종료.")
