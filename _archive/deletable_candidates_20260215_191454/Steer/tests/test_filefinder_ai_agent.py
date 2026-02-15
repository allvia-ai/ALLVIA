from __future__ import annotations

from pathlib import Path

import pytest

from core.data_pipeline.scanner import scan_directory


@pytest.mark.smoke
def test_filefinder_includes_ai_agent_dir_even_when_hidden(tmp_path: Path) -> None:
    # .ai_agent 폴더는 숨김 폴더지만 스캔 대상에 포함되어야 함 (Legacy FileFinder behavior)
    # 현재 scanner.py가 이를 지원하는지 확인 필요. 지원하지 않는다면 이 테스트는 실패할 것임.
    (tmp_path / ".ai_agent" / "meetings" / "run1").mkdir(parents=True)
    transcript = tmp_path / ".ai_agent" / "meetings" / "run1" / "transcript.txt"
    transcript.write_text("hello", encoding="utf-8")

    (tmp_path / ".secret").mkdir()
    hidden_doc = tmp_path / ".secret" / "leak.txt"
    hidden_doc.write_text("should not be scanned", encoding="utf-8")

    # scanner.py의 scan_directory 사용
    rows = scan_directory(tmp_path, exts=[".txt"])
    paths = {Path(row["path"]) for row in rows if row.get("path")}

    # NOTE: 만약 scanner.py가 모든 dot-dir을 무시한다면 이 assert는 실패할 수 있음.
    # 그 경우 scanner.py를 수정하거나 의도를 확인해야 함.
    # 일단 ModuleNotFoundError를 해결하는 것이 우선.
    assert transcript in paths
    assert hidden_doc not in paths

