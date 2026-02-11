from __future__ import annotations

import argparse
import sys
from pathlib import Path

# Ensure local src is importable when running as a script.
PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.config import load_config
from collector.retention import retention_result_json, run_retention
from collector.store import SQLiteStore


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run retention cleanup")
    parser.add_argument(
        "--config", default="configs/config.yaml", help="path to config file"
    )
    parser.add_argument(
        "--force-vacuum",
        action="store_true",
        help="force VACUUM regardless of size threshold",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    config = load_config(args.config)
    store = SQLiteStore(
        config.db_path,
        wal_mode=config.wal_mode,
        busy_timeout_ms=config.store.busy_timeout_ms,
    )
    store.connect()
    store.migrate(config.migrations_path)

    result = run_retention(store, config.retention, force_vacuum=args.force_vacuum)
    print(retention_result_json(result))
    store.close()


if __name__ == "__main__":
    main()
