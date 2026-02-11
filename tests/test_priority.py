from __future__ import annotations

import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(PROJECT_ROOT / "src"))

from collector.priority import _classify_priority


def test_priority_mappings() -> None:
    assert _classify_priority("outlook.send_clicked", "P1") == "P0"
    assert _classify_priority("excel.export_pdf", "P2") == "P0"
    assert _classify_priority("outlook.compose_started", "P2") == "P1"
    assert _classify_priority("outlook.attachment_added_meta", "P2") == "P1"
    assert _classify_priority("unknown.event", "P2") == "P2"
