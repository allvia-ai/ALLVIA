"""Hybrid storage/offloading strategies."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Optional

from core.utils import get_logger

LOGGER = get_logger("infra.offload")


@dataclass
class OffloadStrategy:
    local_root: Path
    cloud_bucket: Optional[str] = None
    cold_after_days: int = 30

    def should_offload(self, path: Path) -> bool:
        if not path.exists():
            return False
        age_days = (path.stat().st_mtime - path.stat().st_ctime) / 86400
        return age_days >= self.cold_after_days

    def offload(self, path: Path) -> None:
        if not self.cloud_bucket:
            LOGGER.info("offload skipped (%s) – cloud bucket not configured", path)
            return
        LOGGER.info("offloading %s to bucket=%s", path, self.cloud_bucket)
        # Placeholder for S3/Azure upload integration

    def recall(self, path: Path) -> None:
        LOGGER.info("recall requested for %s", path)
        # Placeholder for cloud → local recall
