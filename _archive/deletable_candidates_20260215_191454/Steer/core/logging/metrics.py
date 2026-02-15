from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Optional

from core.config.paths import METRICS_PATH


def _atomic_write_json(path: Path, payload: Dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
    tmp.replace(path)


@dataclass
class MetricsStore:
    path: Path = METRICS_PATH

    def load(self) -> Dict[str, Any]:
        if not self.path.exists():
            return {}
        try:
            return json.loads(self.path.read_text(encoding="utf-8"))
        except Exception:
            return {}

    def update(self, updates: Dict[str, Any]) -> Dict[str, Any]:
        current = self.load()
        current.update(updates)
        _atomic_write_json(self.path, current)
        return current

    def increment(self, key: str, value: float = 1.0) -> float:
        current = self.load()
        existing = current.get(key, 0.0)
        try:
            existing_value = float(existing)
        except Exception:
            existing_value = 0.0
        new_value = existing_value + float(value)
        current[key] = new_value
        _atomic_write_json(self.path, current)
        return new_value

