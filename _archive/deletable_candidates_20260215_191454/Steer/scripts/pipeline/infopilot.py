# infopilot.py
from __future__ import annotations
import sys
import os
from pathlib import Path
from types import SimpleNamespace

ROOT_DIR = Path(__file__).resolve().parents[2]
if str(ROOT_DIR) not in sys.path:
    sys.path.insert(0, str(ROOT_DIR))

try:
    from dotenv import load_dotenv
    load_dotenv(dotenv_path=ROOT_DIR / ".env", override=False)
except Exception:
    pass

import click
from scripts.pipeline.infopilot_cli.scan import cmd_scan as _cmd_scan_impl
from scripts.pipeline.infopilot_cli.chat import cmd_chat as _cmd_chat_impl
from scripts.pipeline.infopilot_cli.schedule import cmd_schedule as _cmd_schedule_impl
from scripts.pipeline.infopilot_cli.watch import cmd_watch as _cmd_watch_impl

from scripts.pipeline.infopilot_cli.scan import cmd_scan as run_scan_cmd
from scripts.pipeline.infopilot_cli.chat import cmd_chat as run_chat_cmd
from scripts.pipeline.infopilot_cli.scan_rows import load_scan_rows

from core.data_pipeline.pipeline import run_step2
from scripts.pipeline.infopilot_cli.schedule import register_policy_jobs

_load_scan_rows = load_scan_rows
_run_scan = run_scan_cmd
_register_policy_jobs = register_policy_jobs

KNOWLEDGE_AGENT = "knowledge_search"
DEFAULT_POLICY_PATH = ROOT_DIR / "core" / "config" / "smart_folders.json"
DEFAULT_FOUND_FILES = ROOT_DIR / "data" / "found_files.csv"
DEFAULT_CHUNK_CACHE = ROOT_DIR / "data" / "cache" / "chunk_cache.json"
DEFAULT_SCAN_STATE = ROOT_DIR / "data" / "cache" / "scan_state.json"

@click.group()
@click.option("--no-mlflow", is_flag=True, hidden=True, help="Disable MLflow tracking (deprecated, no-op)")
def cli(no_mlflow):
    """InfoPilot CLI"""
    # --no-mlflow is now a no-op for backward compatibility
    pass

# --- SCAN ---
@cli.command()
@click.option("--out", default="data/found_files.csv", help="Output CSV path")
@click.option("--root", "roots", multiple=True, help="Root directories")
@click.option("--policy", default=str(DEFAULT_POLICY_PATH), help="Policy file path")
def scan(out, roots, policy):
    args = SimpleNamespace(out=out, roots=roots, policy=policy, exts=None)
    run_scan_cmd(args, default_policy_path=DEFAULT_POLICY_PATH, agent=KNOWLEDGE_AGENT)

# Backward compatibility for scripts
cmd_index = scan
cmd_scan = scan

# Dummy train command for legacy scripts
@click.command()
def noop_train():
    print("Training is now handled implicitly or via dedicated scripts.")
cmd_train = noop_train

# --- CHAT ---
@cli.command()
@click.option("--model", default="data/topic_model.joblib")
@click.option("--corpus", default="data/corpus.parquet")
@click.option("--scan_csv", default="data/found_files.csv")
@click.option("--cache", default="data/cache")
@click.option("--json", "json_mode", is_flag=True, help="JSON output mode")
@click.option("--topk", default=10)
def chat(model, corpus, scan_csv, cache, json_mode, topk):
    args = SimpleNamespace(
        model=model, corpus=corpus, scan_csv=scan_csv, cache=cache,
        json=json_mode, topk=topk, translate=False, auto_train=False,
        rerank=True, rerank_model="BAAI/bge-reranker-large",
        rerank_depth=50, rerank_batch_size=16, rerank_device=None,
        rerank_min_score=0.35, lexical_weight=0.4,
        show_translation=False, translation_lang="en",
        min_similarity=0.35, strict=False, policy=str(DEFAULT_POLICY_PATH),
        scope="auto",
        llm_backend=None, llm_model=None, llm_host=None
    )
    run_chat_cmd(args, default_policy_path=DEFAULT_POLICY_PATH, policy_agent=KNOWLEDGE_AGENT)

# --- SCHEDULE ---
@cli.command()
@click.option("--agent", default=KNOWLEDGE_AGENT)
@click.option("--policy", default=str(DEFAULT_POLICY_PATH))
@click.option("--output_root", default="data/scheduled")
@click.option("--poll_seconds", default=60)
@click.option("--once", is_flag=True)
def schedule(agent, policy, output_root, poll_seconds, once):
    args = SimpleNamespace(
        agent=agent, policy=policy, output_root=output_root, 
        poll_seconds=poll_seconds, once=once, translate=False
    )
    _cmd_schedule_impl(args, knowledge_agent=KNOWLEDGE_AGENT)

# --- WATCH ---
@cli.command()
@click.option("--output-root", default="data/watch_index", help="Index root directory")
@click.option("--target", multiple=True, help="Target directories to watch")
@click.option("--policy", default=str(DEFAULT_POLICY_PATH), help="Policy file path")
@click.option("--debounce", default=1.0, help="Debounce seconds")
def watch(output_root, target, policy, debounce):
    args = SimpleNamespace(
        output_root=output_root, target=target, policy=policy, debounce=debounce,
        model_name="all-MiniLM-L6-v2", batch_size=32, translate=False
    )
    _cmd_watch_impl(args, knowledge_agent=KNOWLEDGE_AGENT)

if __name__ == "__main__":
    cli()
