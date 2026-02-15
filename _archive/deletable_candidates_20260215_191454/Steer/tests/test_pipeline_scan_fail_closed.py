from __future__ import annotations

import csv
import json
from pathlib import Path

import pytest

from core.policy.engine import PolicyEngine
from scripts.pipeline.infopilot import _load_scan_rows
from scripts.pipeline.infopilot_cli.scan import run_scan


@pytest.mark.smoke
def test_scan_marks_denied_and_load_scan_rows_filters(tmp_path: Path) -> None:
    in_scope = tmp_path / "in"
    out_scope = tmp_path / "out"
    in_scope.mkdir()
    out_scope.mkdir()

    allowed = in_scope / "a.txt"
    denied_by_type = in_scope / "b.exe"
    denied_out_of_scope = out_scope / "c.txt"
    allowed.write_text("hello", encoding="utf-8")
    denied_by_type.write_text("nope", encoding="utf-8")
    denied_out_of_scope.write_text("nope", encoding="utf-8")

    policy_json = tmp_path / "policy.json"
    policy_json.write_text(
        json.dumps(
            [
                {
                    "path": str(in_scope),
                    "agents": ["knowledge_search"],
                    "allow_types": [".txt", ".exe"],
                    "deny_types": [".exe"],
                    "security": {"pii_filter": True},
                }
            ],
            ensure_ascii=False,
            indent=2,
        ),
        encoding="utf-8",
    )
    engine = PolicyEngine.from_file(policy_json)

    scan_csv = tmp_path / "found_files.csv"
    rows = run_scan(
        scan_csv,
        roots=[in_scope, out_scope],
        policy_engine=engine,
        exts=["txt", "exe"],
        agent="knowledge_search",
        include_denied=True,
        include_hash=True,
    )

    by_path = {row["path"]: row for row in rows}
    assert by_path[str(allowed)]["allowed"] == 1
    assert by_path[str(allowed)]["deny_reason"] == ""
    assert by_path[str(allowed)]["hash"]

    assert by_path[str(denied_by_type)]["allowed"] == 0
    assert by_path[str(denied_by_type)]["deny_reason"] == "type_denied"
    assert by_path[str(denied_by_type)]["hash"] == ""

    assert by_path[str(denied_out_of_scope)]["allowed"] == 0
    assert by_path[str(denied_out_of_scope)]["deny_reason"] == "out_of_scope"
    assert by_path[str(denied_out_of_scope)]["hash"] == ""

    # Downstream steps must be fail-closed: denied rows never re-enter the pipeline.
    filtered = list(_load_scan_rows(scan_csv, agent="knowledge_search", policy_engine=engine, include_manual=True))
    assert [row["path"] for row in filtered] == [str(allowed)]
    assert filtered[0]["policy_mask_pii"] is True

    with scan_csv.open("r", encoding="utf-8", newline="") as handle:
        reader = csv.DictReader(handle)
        assert reader.fieldnames == ["path", "size", "mtime", "allowed", "deny_reason", "hash"]
