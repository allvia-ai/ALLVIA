"""Dataclasses for photo agent inputs and outputs."""
from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Dict, List, Optional


@dataclass
class PhotoJobConfig:
    roots: List[Path]
    output_dir: Path
    policy_tag: Optional[str] = None
    policy_engine: Optional[object] = None
    policy_agent: str = "photo"
    prefer_gpu: bool = False
    organize: bool = False
    dry_run: bool = True
    dest_root: Optional[Path] = None
    organize_strategy: str = "month"
    dedupe: bool = False
    created_at: datetime = field(default_factory=datetime.utcnow)


@dataclass
class PhotoAsset:
    path: Path
    tags: List[str]
    embedding: Optional[List[float]] = None
    score: Optional[float] = None
    capture_date: Optional[datetime] = None
    location: str = "Unknown"
    gps: Optional[tuple] = None
    face_count: int = -1  # -1 = not detected, 0+ = number of faces
    metadata: Dict[str, object] = field(default_factory=dict)


@dataclass
class PhotoRecommendation:
    best_shots: List[PhotoAsset]
    duplicates: List[List[PhotoAsset]]
    similar_groups: List[List[PhotoAsset]]
    report_path: Path
