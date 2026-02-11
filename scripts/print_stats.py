from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from urllib.error import URLError
from urllib.request import Request, urlopen

# Ensure local src is importable when running as a script.
PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.config import load_config


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Fetch /stats from collector")
    parser.add_argument(
        "--config", default="configs/config.yaml", help="path to config file"
    )
    parser.add_argument(
        "--endpoint", default="", help="override stats endpoint URL"
    )
    parser.add_argument(
        "--timeout", type=float, default=5.0, help="request timeout in seconds"
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    config = load_config(args.config)
    endpoint = args.endpoint.strip()
    if not endpoint:
        endpoint = f"http://{config.ingest.host}:{config.ingest.port}/stats"

    try:
        req = Request(endpoint, method="GET")
        with urlopen(req, timeout=args.timeout) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except (URLError, ValueError, json.JSONDecodeError) as exc:
        print(f"failed to fetch stats from {endpoint}: {exc}")
        return

    print(json.dumps(payload, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
