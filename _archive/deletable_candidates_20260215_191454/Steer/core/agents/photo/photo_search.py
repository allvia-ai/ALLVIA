"""Photo search engine with criteria-based filtering."""
from __future__ import annotations

from datetime import datetime
from pathlib import Path
from typing import List, Optional

from core.utils import get_logger

from .models import PhotoAsset
from .query_parser import PhotoSearchCriteria

LOGGER = get_logger("photo.search")


def search_photos(
    photos: List[PhotoAsset],
    criteria: PhotoSearchCriteria
) -> List[PhotoAsset]:
    """Search photos by criteria.
    
    Args:
        photos: List of PhotoAsset to search
        criteria: Search criteria from query parser
        
    Returns:
        List of matching PhotoAsset
    """
    results = []
    
    for photo in photos:
        if _matches_criteria(photo, criteria):
            results.append(photo)
    
    LOGGER.info(
        "Search found %d/%d photos matching criteria",
        len(results), len(photos)
    )
    
    return results


def _matches_criteria(photo: PhotoAsset, criteria: PhotoSearchCriteria) -> bool:
    """Check if photo matches all criteria."""
    
    # Check date range
    if criteria.date_start or criteria.date_end:
        if not _matches_date(photo, criteria.date_start, criteria.date_end):
            return False
    
    # Check location
    if criteria.location:
        if not _matches_location(photo, criteria.location):
            return False
    
    # Check face count
    if criteria.face_count is not None:
        if not _matches_face_count(photo, criteria.face_count, criteria.face_count_op):
            return False
    
    # Check scene tags
    if criteria.scene_tags:
        if not _matches_tags(photo, criteria.scene_tags):
            return False
    
    return True


def _matches_date(
    photo: PhotoAsset,
    date_start: Optional[datetime],
    date_end: Optional[datetime]
) -> bool:
    """Check if photo date is within range."""
    if photo.capture_date is None:
        return False
    
    if date_start and photo.capture_date < date_start:
        return False
    if date_end and photo.capture_date > date_end:
        return False
    
    return True


def _matches_location(photo: PhotoAsset, location: str) -> bool:
    """Check if photo location matches (case-insensitive, partial match)."""
    if not photo.location or photo.location == "Unknown":
        return False
    
    return location.lower() in photo.location.lower()


def _matches_face_count(photo: PhotoAsset, count: int, op: str) -> bool:
    """Check if photo face count matches.
    
    Args:
        photo: PhotoAsset to check
        count: Expected face count
        op: Comparison operator ('eq', 'gte', 'lte')
    """
    if photo.face_count < 0:  # Detection not run
        return False
    
    if op == "eq":
        return photo.face_count == count
    elif op == "gte":
        return photo.face_count >= count
    elif op == "lte":
        return photo.face_count <= count
    
    return False


def _matches_tags(photo: PhotoAsset, required_tags: List[str]) -> bool:
    """Check if photo has any of the required tags."""
    if not photo.tags:
        return False
    
    photo_tags_lower = [t.lower() for t in photo.tags]
    
    for tag in required_tags:
        if tag.lower() in photo_tags_lower:
            return True
    
    return False


def search_photos_in_folder(
    folder: Path,
    query: str,
    run_detection: bool = True
) -> List[PhotoAsset]:
    """Search photos in folder using natural language query.
    
    This is a convenience function that:
    1. Scans folder for photos
    2. Extracts metadata (EXIF, GPS, faces)
    3. Parses query
    4. Returns matching photos
    
    Args:
        folder: Folder to search
        query: Natural language query
        run_detection: Whether to run face detection
        
    Returns:
        List of matching PhotoAsset
    """
    from .query_parser import parse_photo_query
    from .exif_utils import get_photo_metadata
    from .face_detector import count_faces, HAS_MEDIAPIPE
    
    LOGGER.info("Searching photos in %s with query: %s", folder, query)
    
    # Parse query
    criteria = parse_photo_query(query)
    
    # Scan folder
    extensions = {'.jpg', '.jpeg', '.png', '.heic', '.gif', '.webp'}
    photos = []
    
    for path in folder.rglob('*'):
        if path.suffix.lower() not in extensions:
            continue
        
        # Get metadata
        meta = get_photo_metadata(path)
        
        # Run face detection if needed
        face_count = -1
        if run_detection and HAS_MEDIAPIPE and criteria.face_count is not None:
            face_count = count_faces(path)
        
        asset = PhotoAsset(
            path=path,
            tags=[],
            capture_date=meta.get("capture_date"),
            location=meta.get("location", "Unknown"),
            gps=meta.get("gps"),
            face_count=face_count
        )
        photos.append(asset)
    
    LOGGER.info("Scanned %d photos, running search...", len(photos))
    
    # Search
    results = search_photos(photos, criteria)
    
    return results
