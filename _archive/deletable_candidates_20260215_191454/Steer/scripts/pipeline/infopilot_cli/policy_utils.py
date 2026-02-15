# scripts/pipeline/infopilot_cli/policy_utils.py
from __future__ import annotations

import csv
import hashlib
from pathlib import Path
from dataclasses import dataclass
from typing import Any, Dict, List, Set

from core.policy.engine import SmartFolderPolicy

@dataclass
class PolicyArtifacts:
    base_dir: Path
    scan_csv: Path
    corpus: Path
    model: Path
    cache_dir: Path

    def ensure_dirs(self) -> None:
        self.base_dir.mkdir(parents=True, exist_ok=True)
        self.cache_dir.mkdir(parents=True, exist_ok=True)

def policy_slug(policy: SmartFolderPolicy) -> str:
    digest = hashlib.sha1(str(policy.path).encode("utf-8")).hexdigest()[:8]
    candidate = policy.path.name or policy.path.anchor.strip("\\/") or "policy"
    safe = "".join(ch.lower() if ch.isalnum() else "_" for ch in candidate).strip("_") or "policy"
    return f"{safe}-{digest}"

def get_policy_artifacts(root: Path, policy: SmartFolderPolicy) -> PolicyArtifacts:
    slug = policy_slug(policy)
    base_dir = root / slug
    return PolicyArtifacts(
        base_dir=base_dir,
        scan_csv=base_dir / "found_files.csv",
        corpus=base_dir / "corpus.parquet",
        model=base_dir / "topic_model.joblib",
        cache_dir=base_dir / "cache",
    )

def sync_scan_csv(
    scan_csv: Path,
    rows_to_add: List[Dict[str, Any]],
    paths_to_remove: Set[str],
) -> None:
    if not rows_to_add and not paths_to_remove:
        return

    def _normalize_path(raw: Any) -> str:
        return str(raw or "").strip()

    fieldnames = ["path", "size", "mtime", "ctime", "ext", "drive", "owner"]
    additions: Dict[str, Dict[str, Any]] = {}
    for row in rows_to_add:
        path_key = _normalize_path(row.get("path"))
        if not path_key:
            continue
        additions[path_key] = {name: row.get(name) for name in fieldnames}

    removals = {_normalize_path(path) for path in paths_to_remove if _normalize_path(path)}
    removals.difference_update(additions.keys())

    scan_csv.parent.mkdir(parents=True, exist_ok=True)

    if not scan_csv.exists():
        with scan_csv.open("w", encoding="utf-8", newline="") as dst:
            writer = csv.DictWriter(dst, fieldnames=fieldnames)
            writer.writeheader()
            for record in additions.values():
                writer.writerow(record)
        return

    temp_path = scan_csv.with_suffix(scan_csv.suffix + ".tmp")
    with scan_csv.open("r", encoding="utf-8", newline="") as src, temp_path.open(
        "w", encoding="utf-8", newline=""
    ) as dst:
        reader = csv.DictReader(src)
        writer = csv.DictWriter(dst, fieldnames=fieldnames)
        writer.writeheader()

        for row in reader:
            path_key = _normalize_path(row.get("path"))
            if not path_key or path_key in removals or path_key in additions:
                continue
            writer.writerow({name: row.get(name) for name in fieldnames})

        for record in additions.values():
            writer.writerow(record)

    temp_path.replace(scan_csv)
