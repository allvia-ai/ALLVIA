"""Release preparation helper: collects KPIs and writes human-readable summary.

The script keeps external tooling lightweight so it can run inside the
workspace without extra dependencies. Where optional libraries (pyarrow,
pandas) are missing we gracefully fall back to ``None`` metrics so the caller
can still inspect the generated report.
"""
from __future__ import annotations

import argparse
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, Optional

REPO_ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS_DIR = REPO_ROOT / "artifacts"


def run(cmd: list[str]) -> Optional[str]:
    """Run *cmd* inside the repo and return stdout, or ``None`` on failure."""

    try:
        result = subprocess.run(
            cmd,
            cwd=REPO_ROOT,
            capture_output=True,
            text=True,
            check=True,
        )
    except (subprocess.CalledProcessError, OSError):
        return None
    return result.stdout.strip()


def _count_parquet_rows(path: Path) -> Optional[int]:
    if not path.exists():
        return None
    try:
        import pyarrow.parquet as pq  # type: ignore

        return pq.ParquetFile(path).metadata.num_rows
    except ModuleNotFoundError:
        try:
            import pandas as pd  # type: ignore

            return int(pd.read_parquet(path).shape[0])
        except ModuleNotFoundError:
            return None
        except Exception:
            return None
    except Exception:
        return None


def _stat_file(path: Path) -> Dict[str, Any]:
    return {
        "exists": path.exists(),
        "size_bytes": path.stat().st_size if path.exists() else None,
        "modified_at": datetime.fromtimestamp(path.stat().st_mtime, tz=timezone.utc).isoformat()
        if path.exists()
        else None,
    }


def collect_kpis() -> Dict[str, Any]:
    corpus_path = REPO_ROOT / "data" / "corpus.parquet"
    model_path = REPO_ROOT / "data" / "topic_model.joblib"

    doc_rows = _count_parquet_rows(corpus_path)
    git_commit = run(["git", "rev-parse", "HEAD"])
    git_branch = run(["git", "rev-parse", "--abbrev-ref", "HEAD"])

    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "git": {
            "commit": git_commit,
            "branch": git_branch,
        },
        "corpus": {
            "path": str(corpus_path),
            "rows": doc_rows,
            "stats": _stat_file(corpus_path),
        },
        "model": {
            "path": str(model_path),
            "stats": _stat_file(model_path),
        },
        "notes": "Integrate real KPI metrics (latency, accuracy) once telemetry is wired.",
    }


def write_artifacts(kpis: Dict[str, Any]) -> None:
    ARTIFACTS_DIR.mkdir(exist_ok=True)

    json_path = ARTIFACTS_DIR / "kpi.json"
    json_path.write_text(json.dumps(kpis, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    human_path = ARTIFACTS_DIR / "kpi_summary.md"
    human_lines = ["# KPI Snapshot", ""]
    human_lines.append(f"Generated: {kpis.get('generated_at', 'n/a')}")
    git_info = kpis.get("git", {}) or {}
    human_lines.append(
        f"Git: {git_info.get('branch', 'n/a')} @ {git_info.get('commit', 'n/a')}"
    )
    human_lines.append("")

    corpus = kpis.get("corpus", {}) or {}
    corpus_rows = corpus.get("rows")
    human_lines.append("## Corpus")
    human_lines.append(f"Path: {corpus.get('path', 'n/a')}")
    human_lines.append(f"Rows: {corpus_rows if corpus_rows is not None else 'unknown'}")
    human_lines.append("")

    model = kpis.get("model", {}) or {}
    human_lines.append("## Model")
    human_lines.append(f"Path: {model.get('path', 'n/a')}")
    model_stats = model.get("stats", {}) or {}
    size_bytes = model_stats.get("size_bytes")
    human_lines.append(f"Size: {size_bytes if size_bytes is not None else 'unknown'} bytes")
    human_lines.append("")

    human_lines.append("## Notes")
    human_lines.append(kpis.get("notes", ""))
    human_path.write_text("\n".join(human_lines) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description="Collect release KPIs and summaries.")
    parser.add_argument(
        "--print", dest="print_json", action="store_true", help="Print the KPI JSON to stdout."
    )
    args = parser.parse_args()

    kpis = collect_kpis()
    write_artifacts(kpis)

    if args.print_json:
        print(json.dumps(kpis, indent=2, ensure_ascii=False))
    else:
        print("KPI snapshot saved to artifacts/kpi.json and artifacts/kpi_summary.md")


if __name__ == "__main__":
    main()
