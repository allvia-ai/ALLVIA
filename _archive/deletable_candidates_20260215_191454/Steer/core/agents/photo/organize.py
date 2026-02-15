"""Photo organization planner/applicator (offline, filesystem-only).

The goal is to make photo cleanup usable without requiring any remote vision models:
- Group photos by capture date (EXIF when available, otherwise filesystem mtime).
- Move files into a deterministic folder structure.
- Support dry-run by default and write an action log for auditing/undo.
"""

from __future__ import annotations

import hashlib
import os
import shutil
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Sequence, Tuple

from core.utils import get_logger

LOGGER = get_logger("photo.organize")

try:
    from PIL import Image
except Exception:  # pragma: no cover - optional dependency
    Image = None  # type: ignore


SUPPORTED_EXTS = {".jpg", ".jpeg", ".png", ".heic"}


@dataclass(frozen=True)
class MoveOperation:
    src: Path
    dst: Path
    reason: str
    capture_date: str
    duplicate_of: Optional[Path] = None


@dataclass(frozen=True)
class OrganizePlan:
    dest_root: Path
    operations: Tuple[MoveOperation, ...]
    skipped: Tuple[str, ...]
    strategy: str
    dedupe: bool


@dataclass(frozen=True)
class ApplyResult:
    applied: Tuple[MoveOperation, ...]
    skipped: Tuple[str, ...]


def _safe_stat_mtime(path: Path) -> Optional[datetime]:
    try:
        ts = path.stat().st_mtime
    except OSError:
        return None
    return datetime.fromtimestamp(ts)


def _parse_exif_datetime(value: object) -> Optional[datetime]:
    if value is None:
        return None
    if isinstance(value, bytes):
        try:
            value = value.decode("utf-8", "ignore")
        except Exception:
            return None
    text = str(value).strip()
    if not text:
        return None
    for fmt in ("%Y:%m:%d %H:%M:%S", "%Y-%m-%d %H:%M:%S"):
        try:
            return datetime.strptime(text, fmt)
        except ValueError:
            continue
    return None


def extract_capture_datetime(path: Path) -> Optional[datetime]:
    """Return capture datetime using EXIF when available, otherwise None."""
    suffix = path.suffix.lower()
    if suffix not in {".jpg", ".jpeg"}:
        return None
    if Image is None:
        return None
    try:
        with Image.open(path) as img:
            exif = getattr(img, "getexif", None)
            if not callable(exif):
                return None
            data = exif()
            # DateTimeOriginal=36867, DateTime=306
            return _parse_exif_datetime(data.get(36867) or data.get(306))
    except Exception:
        return None


def _bucket_for_date(dt: datetime, *, strategy: str) -> Path:
    if strategy == "year":
        return Path(f"{dt.year:04d}")
    # default: year/month
    return Path(f"{dt.year:04d}") / f"{dt.month:02d}"


def _sanitize_folder_name(name: str) -> str:
    """Sanitize folder name by removing/replacing invalid characters."""
    # Replace problematic characters
    for char in ['/', '\\', ':', '*', '?', '"', '<', '>', '|']:
        name = name.replace(char, '_')
    return name.strip()[:50]  # Limit length


def _bucket_for_smart(
    dt: datetime,
    *,
    location: str = "Unknown",
    tag: str = "기타",
) -> Path:
    """Create smart folder name: YYYY-MM-DD_Location_Tag.
    
    Example: 2024-03-22_Tokyo_여행
    """
    date_str = dt.strftime("%Y-%m-%d")
    location_clean = _sanitize_folder_name(location)
    tag_clean = _sanitize_folder_name(tag)
    
    folder_name = f"{date_str}_{location_clean}_{tag_clean}"
    return Path(folder_name)



def _unique_destination(dst: Path, *, reserved: Optional[set[str]] = None) -> Path:
    reserved_set = reserved or set()
    dst_key = dst.as_posix()
    if dst_key not in reserved_set and not dst.exists():
        reserved_set.add(dst_key)
        return dst
    stem = dst.stem
    suffix = dst.suffix
    parent = dst.parent
    for idx in range(1, 10000):
        candidate = parent / f"{stem}__{idx}{suffix}"
        key = candidate.as_posix()
        if key in reserved_set:
            continue
        if not candidate.exists():
            reserved_set.add(key)
            return candidate
    raise RuntimeError(f"destination collision: {dst}")


def _sha256(path: Path, *, chunk_size: int = 1024 * 1024) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            buf = handle.read(chunk_size)
            if not buf:
                break
            h.update(buf)
    return h.hexdigest()


def _detect_duplicates(paths: Sequence[Path]) -> Dict[Path, Path]:
    """Return mapping duplicate_path -> canonical_path."""
    by_size: Dict[int, List[Path]] = {}
    for path in paths:
        try:
            size = path.stat().st_size
        except OSError:
            continue
        by_size.setdefault(int(size), []).append(path)

    duplicate_of: Dict[Path, Path] = {}
    for bucket in by_size.values():
        if len(bucket) < 2:
            continue
        hashes: Dict[str, Path] = {}
        for path in bucket:
            try:
                digest = _sha256(path)
            except OSError:
                continue
            canonical = hashes.get(digest)
            if canonical is None:
                hashes[digest] = path
            else:
                duplicate_of[path] = canonical
    return duplicate_of


def build_plan(
    files: Iterable[Path],
    *,
    dest_root: Path,
    strategy: str = "month",
    dedupe: bool = False,
) -> OrganizePlan:
    resolved_root = dest_root.expanduser().resolve()
    skipped: List[str] = []
    candidates: List[Path] = []
    for path in files:
        if path.suffix.lower() not in SUPPORTED_EXTS:
            continue
        try:
            candidates.append(path.resolve())
        except OSError:
            skipped.append(f"unreadable_path: {path}")

    duplicate_map: Dict[Path, Path] = _detect_duplicates(candidates) if dedupe else {}

    operations: List[MoveOperation] = []
    reserved: set[str] = set()
    for src in candidates:
        if not src.exists():
            skipped.append(f"missing: {src}")
            continue
        if str(src).startswith(str(resolved_root)):
            skipped.append(f"already_in_dest: {src}")
            continue

        capture_dt = extract_capture_datetime(src) or _safe_stat_mtime(src)
        if capture_dt is None:
            skipped.append(f"no_timestamp: {src}")
            continue

        capture_date = capture_dt.date().isoformat()
        bucket = _bucket_for_date(capture_dt, strategy=strategy)
        filename = src.name

        duplicate_of = duplicate_map.get(src)
        if duplicate_of is not None:
            target_dir = resolved_root / "_duplicates" / bucket
            reason = "duplicate"
        else:
            target_dir = resolved_root / bucket
            reason = "date_bucket"

        dst = _unique_destination(target_dir / filename, reserved=reserved)
        operations.append(
            MoveOperation(
                src=src,
                dst=dst,
                reason=reason,
                capture_date=capture_date,
                duplicate_of=duplicate_of,
            )
        )

    operations.sort(key=lambda op: (op.dst.as_posix(), op.src.as_posix()))
    return OrganizePlan(
        dest_root=resolved_root,
        operations=tuple(operations),
        skipped=tuple(skipped),
        strategy=strategy,
        dedupe=dedupe,
    )


def apply_plan(plan: OrganizePlan, *, dry_run: bool = True) -> ApplyResult:
    skipped: List[str] = list(plan.skipped)
    applied: List[MoveOperation] = []
    reserved: set[str] = set()

    for op in plan.operations:
        src = op.src
        dst = op.dst
        if not src.exists():
            skipped.append(f"missing: {src}")
            continue
        try:
            unique_dst = _unique_destination(dst, reserved=reserved)
        except Exception:
            skipped.append(f"destination_exists: {dst}")
            continue
        if unique_dst != dst:
            dst = unique_dst
            op = MoveOperation(
                src=op.src,
                dst=dst,
                reason=op.reason,
                capture_date=op.capture_date,
                duplicate_of=op.duplicate_of,
            )
        if dry_run:
            applied.append(op)
            continue
        try:
            dst.parent.mkdir(parents=True, exist_ok=True)
        except OSError as exc:
            skipped.append(f"mkdir_failed: {dst.parent} ({exc})")
            continue
        try:
            src.rename(dst)
        except OSError:
            try:
                shutil.move(str(src), str(dst))
            except OSError as exc:
                skipped.append(f"move_failed: {src} -> {dst} ({exc})")
                continue
        applied.append(op)

    return ApplyResult(applied=tuple(applied), skipped=tuple(skipped))


def build_plan_from_assets(
    assets: List,  # List[PhotoAsset]
    *,
    dest_root: Path,
    strategy: str = "smart",
    dedupe: bool = False,
) -> OrganizePlan:
    """Build organize plan from PhotoAssets with metadata.
    
    Uses YYYY-MM-DD_Location_Tag folder format when strategy='smart'.
    """
    from .models import PhotoAsset
    
    resolved_root = dest_root.expanduser().resolve()
    skipped: List[str] = []
    operations: List[MoveOperation] = []
    reserved: set[str] = set()
    
    # Detect duplicates if requested
    duplicate_map: Dict[Path, Path] = {}
    if dedupe:
        paths = [a.path for a in assets if a.path.exists()]
        duplicate_map = _detect_duplicates(paths)
    
    for asset in assets:
        src = asset.path
        if not src.exists():
            skipped.append(f"missing: {src}")
            continue
        if str(src).startswith(str(resolved_root)):
            skipped.append(f"already_in_dest: {src}")
            continue
        
        # Determine capture date
        capture_dt = asset.capture_date or _safe_stat_mtime(src)
        if capture_dt is None:
            skipped.append(f"no_timestamp: {src}")
            continue
        
        capture_date = capture_dt.date().isoformat()
        
        # Determine bucket based on strategy
        if strategy == "smart":
            location = getattr(asset, 'location', 'Unknown') or 'Unknown'
            tags = getattr(asset, 'tags', []) or []
            tag = tags[0] if tags else '기타'
            bucket = _bucket_for_smart(capture_dt, location=location, tag=tag)
        else:
            bucket = _bucket_for_date(capture_dt, strategy=strategy)
        
        filename = src.name
        
        duplicate_of = duplicate_map.get(src)
        if duplicate_of is not None:
            target_dir = resolved_root / "_duplicates" / bucket
            reason = "duplicate"
        else:
            target_dir = resolved_root / bucket
            reason = "smart_bucket" if strategy == "smart" else "date_bucket"
        
        dst = _unique_destination(target_dir / filename, reserved=reserved)
        operations.append(
            MoveOperation(
                src=src,
                dst=dst,
                reason=reason,
                capture_date=capture_date,
                duplicate_of=duplicate_of,
            )
        )
    
    operations.sort(key=lambda op: (op.dst.as_posix(), op.src.as_posix()))
    return OrganizePlan(
        dest_root=resolved_root,
        operations=tuple(operations),
        skipped=tuple(skipped),
        strategy=strategy,
        dedupe=dedupe,
    )
