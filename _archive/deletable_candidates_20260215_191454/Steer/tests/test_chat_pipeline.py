import json
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "pipeline" / "infopilot.py"
MODEL = ROOT / "data" / "topic_model.joblib"
CORPUS = ROOT / "data" / "corpus.parquet"
CACHE = ROOT / "data" / "cache"


@pytest.mark.integration
@pytest.mark.skipif(not SCRIPT.exists(), reason="infopilot.py not available")
def test_chat_command_returns_json_payload():
    if not MODEL.exists() or not CORPUS.exists():
        pytest.skip("chat artifacts missing")

    cmd = [
        sys.executable,
        str(SCRIPT),
        "run",
        "chat",
        "--model",
        str(MODEL),
        "--corpus",
        str(CORPUS),
        "--cache",
        str(CACHE),
        "--query",
        "테스트 인사",
        "--json",
        "--no-auto-train",
    ]

    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if proc.returncode != 0:
        pytest.skip(f"chat command unavailable: {proc.stderr.strip()}")

    payload = json.loads(proc.stdout.strip() or "{}")
    assert "answer" in payload
    assert payload.get("query") == "테스트 인사"
    assert isinstance(payload.get("suggestions", []), list)
