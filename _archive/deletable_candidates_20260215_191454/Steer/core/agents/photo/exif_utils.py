"""EXIF metadata extraction utilities for photo classification."""
from __future__ import annotations

import os
from datetime import datetime
from pathlib import Path
from typing import Optional, Tuple
from functools import lru_cache

from core.utils import get_logger

LOGGER = get_logger("photo.exif_utils")

# Try to import optional dependencies
try:
    from PIL import Image
    from PIL.ExifTags import TAGS, GPSTAGS
    HAS_PIL = True
except ImportError:
    HAS_PIL = False
    LOGGER.warning("PIL not available. EXIF extraction disabled.")

try:
    from geopy.geocoders import Nominatim
    from geopy.exc import GeocoderTimedOut, GeocoderServiceError
    HAS_GEOPY = True
except ImportError:
    HAS_GEOPY = False
    LOGGER.warning("geopy not available. GPS→Location disabled.")


def extract_date(path: Path) -> Optional[datetime]:
    """Extract capture date from image EXIF data.
    
    Returns:
        datetime if found, None otherwise
    """
    if not HAS_PIL:
        return None
    
    try:
        with Image.open(path) as img:
            exif_data = img._getexif()
            if not exif_data:
                return None
            
            # Look for DateTimeOriginal (36867) or DateTime (306)
            for tag_id, value in exif_data.items():
                tag = TAGS.get(tag_id, tag_id)
                if tag == "DateTimeOriginal":
                    return datetime.strptime(value, "%Y:%m:%d %H:%M:%S")
                elif tag == "DateTime":
                    return datetime.strptime(value, "%Y:%m:%d %H:%M:%S")
    except Exception as e:
        LOGGER.debug("Failed to extract date from %s: %s", path, e)
    
    return None


def extract_gps(path: Path) -> Optional[Tuple[float, float]]:
    """Extract GPS coordinates from image EXIF data.
    
    Returns:
        (latitude, longitude) tuple if found, None otherwise
    """
    if not HAS_PIL:
        return None
    
    try:
        with Image.open(path) as img:
            exif_data = img._getexif()
            if not exif_data:
                return None
            
            gps_info = {}
            for tag_id, value in exif_data.items():
                tag = TAGS.get(tag_id, tag_id)
                if tag == "GPSInfo":
                    for gps_tag_id, gps_value in value.items():
                        gps_tag = GPSTAGS.get(gps_tag_id, gps_tag_id)
                        gps_info[gps_tag] = gps_value
            
            if not gps_info:
                return None
            
            # Parse latitude
            lat = gps_info.get("GPSLatitude")
            lat_ref = gps_info.get("GPSLatitudeRef")
            lon = gps_info.get("GPSLongitude")
            lon_ref = gps_info.get("GPSLongitudeRef")
            
            if not all([lat, lat_ref, lon, lon_ref]):
                return None
            
            lat_deg = _convert_to_degrees(lat)
            lon_deg = _convert_to_degrees(lon)
            
            if lat_ref == "S":
                lat_deg = -lat_deg
            if lon_ref == "W":
                lon_deg = -lon_deg
            
            return (lat_deg, lon_deg)
    except Exception as e:
        LOGGER.debug("Failed to extract GPS from %s: %s", path, e)
    
    return None


def _convert_to_degrees(value) -> float:
    """Convert GPS coordinates to degrees."""
    d = float(value[0])
    m = float(value[1])
    s = float(value[2])
    return d + (m / 60.0) + (s / 3600.0)


# Cache geocoding results to avoid repeated API calls
@lru_cache(maxsize=1000)
def gps_to_location(lat: float, lon: float) -> str:
    """Convert GPS coordinates to location name using reverse geocoding.
    
    Returns:
        City/town name or "Unknown" if geocoding fails
    """
    if not HAS_GEOPY:
        return "Unknown"
    
    # Check for offline mode
    if os.getenv("PHOTO_OFFLINE", "0") == "1":
        return "Unknown"
    
    try:
        geolocator = Nominatim(user_agent="infopilot_photo_agent", timeout=5)
        location = geolocator.reverse(f"{lat}, {lon}", language="ko")
        
        if location and location.raw:
            address = location.raw.get("address", {})
            # Try to get city/town name in order of preference
            for key in ["city", "town", "village", "county", "state", "country"]:
                if key in address:
                    return address[key]
        
        return "Unknown"
    except (GeocoderTimedOut, GeocoderServiceError) as e:
        LOGGER.debug("Geocoding failed for (%s, %s): %s", lat, lon, e)
        return "Unknown"
    except Exception as e:
        LOGGER.debug("Unexpected geocoding error: %s", e)
        return "Unknown"


def get_photo_metadata(path: Path) -> dict:
    """Extract all relevant metadata from a photo.
    
    Returns:
        dict with 'capture_date', 'location', 'gps' keys
    """
    result = {
        "capture_date": None,
        "location": "Unknown",
        "gps": None,
    }
    
    # Extract date
    result["capture_date"] = extract_date(path)
    
    # Extract GPS and convert to location
    gps = extract_gps(path)
    if gps:
        result["gps"] = gps
        lat, lon = gps
        result["location"] = gps_to_location(lat, lon)
    
    return result
