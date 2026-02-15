from __future__ import annotations

import csv

import pandas as pd

from core.monitor.drift_checker import check_drift


def test_drift_checker_prefers_file_hash(tmp_path):
    scan_csv = tmp_path / "scan.csv"
    with scan_csv.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=["path", "size", "mtime", "allowed", "hash"])
        writer.writeheader()
        writer.writerow({"path": str(tmp_path / "a.txt"), "size": 10, "mtime": 1.0, "allowed": 1, "hash": "h1"})

    corpus_path = tmp_path / "corpus.parquet"
    df = pd.DataFrame([{"path": str(tmp_path / "a.txt"), "size": 10, "mtime": 1.0, "doc_hash": "d1", "file_hash": "h2"}])
    df.to_parquet(corpus_path, index=False)

    report = check_drift(scan_csv, corpus_path)
    assert report.changed_files == [str(tmp_path / "a.txt")]

