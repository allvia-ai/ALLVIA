"""Natural language query parser for photo search."""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import List, Optional, Tuple

from core.utils import get_logger

LOGGER = get_logger("photo.query_parser")


@dataclass
class PhotoSearchCriteria:
    """Parsed search criteria from natural language query."""
    date_start: Optional[datetime] = None
    date_end: Optional[datetime] = None
    location: Optional[str] = None
    face_count: Optional[int] = None  # None = any, 0 = no people, 1 = alone, etc.
    face_count_op: str = "eq"  # eq, gte, lte
    self_filter: Optional[str] = None  # None, "with_me", or "me_alone"
    scene_tags: List[str] = field(default_factory=list)
    object_tags: List[str] = field(default_factory=list)  # YOLO detected objects
    raw_query: str = ""


# Korean date keywords
DATE_KEYWORDS = {
    "올해": lambda: (datetime(datetime.now().year, 1, 1), datetime.now()),
    "작년": lambda: (datetime(datetime.now().year - 1, 1, 1), datetime(datetime.now().year - 1, 12, 31)),
    "재작년": lambda: (datetime(datetime.now().year - 2, 1, 1), datetime(datetime.now().year - 2, 12, 31)),
    "이번달": lambda: (datetime(datetime.now().year, datetime.now().month, 1), datetime.now()),
    "지난달": lambda: _last_month(),
    "오늘": lambda: (datetime.now().replace(hour=0, minute=0, second=0), datetime.now()),
    "어제": lambda: _yesterday(),
}

# Season keywords
SEASON_KEYWORDS = {
    "봄": (3, 5),
    "여름": (6, 8),
    "가을": (9, 11),
    "겨울": (12, 2),
}

# Face count keywords
FACE_KEYWORDS = {
    "혼자": 1,
    "나만": 1,
    "솔로": 1,
    "둘이": 2,
    "커플": 2,
    "셋이": 3,
    "단체": -1,  # -1 means 5+ people
    "가족": -2,  # -2 means unknown, but multiple
}

# Self-identification keywords (requires face registration)
SELF_KEYWORDS = {
    "내가 나온": "with_me",
    "나 나온": "with_me",
    "내 사진": "with_me",
    "내가 혼자": "me_alone",
    "나만 나온": "me_alone",
    "나 혼자": "me_alone",
}

# Scene keywords (Korean to English for CLIP)
SCENE_KEYWORDS = {
    "바다": "beach",
    "해변": "beach",
    "산": "mountain",
    "여행": "travel",
    "음식": "food",
    "맛집": "food",
    "카페": "cafe",
    "공원": "park",
    "도시": "city",
    "밤": "night",
    "야경": "night cityscape",
    "일출": "sunrise",
    "일몰": "sunset",
    "꽃": "flowers",
    "벚꽃": "cherry blossom",
    "눈": "snow",
    "비": "rain",
    "셀카": "selfie",
    "풍경": "landscape",
}

# Object keywords (Korean to COCO class names for YOLO)
OBJECT_KEYWORDS = {
    "강아지": "dog",
    "개": "dog",
    "고양이": "cat",
    "자동차": "car",
    "차": "car",
    "자전거": "bicycle",
    "새": "bird",
    "말": "horse",
    "핸드폰": "cell phone",
    "노트북": "laptop",
    "책": "book",
    "피자": "pizza",
    "케이크": "cake",
    "소파": "couch",
    "의자": "chair",
    "TV": "tv",
    "인형": "teddy bear",
}


def _last_month() -> Tuple[datetime, datetime]:
    """Get last month date range."""
    today = datetime.now()
    if today.month == 1:
        start = datetime(today.year - 1, 12, 1)
        end = datetime(today.year - 1, 12, 31)
    else:
        start = datetime(today.year, today.month - 1, 1)
        end = start.replace(day=28) + timedelta(days=4)
        end = end - timedelta(days=end.day)
    return start, end


def _yesterday() -> Tuple[datetime, datetime]:
    """Get yesterday date range."""
    yesterday = datetime.now() - timedelta(days=1)
    start = yesterday.replace(hour=0, minute=0, second=0, microsecond=0)
    end = yesterday.replace(hour=23, minute=59, second=59)
    return start, end


def _parse_year_season(query: str) -> Optional[Tuple[datetime, datetime]]:
    """Parse year + season patterns like '작년 겨울', '올해 여름'."""
    year = None
    
    # Find year reference
    if "올해" in query:
        year = datetime.now().year
    elif "작년" in query:
        year = datetime.now().year - 1
    elif "재작년" in query:
        year = datetime.now().year - 2
    
    # Find season
    for season_name, (start_month, end_month) in SEASON_KEYWORDS.items():
        if season_name in query:
            if year is None:
                year = datetime.now().year
            
            if season_name == "겨울":
                # Winter spans two years
                start = datetime(year, 12, 1)
                end = datetime(year + 1, 2, 28)
            else:
                start = datetime(year, start_month, 1)
                # Get last day of end month
                if end_month == 12:
                    end = datetime(year, 12, 31)
                else:
                    end = datetime(year, end_month + 1, 1) - timedelta(days=1)
            
            return start, end
    
    return None


def _parse_location(query: str) -> Optional[str]:
    """Extract location from query."""
    # Common travel destinations
    locations = [
        "후쿠오카", "도쿄", "오사카", "교토", "삿포로", "나고야", "오키나와",
        "서울", "부산", "제주", "강릉", "경주", "전주",
        "방콕", "파타야", "싱가포르", "홍콩", "대만", "타이페이",
        "파리", "런던", "뉴욕", "LA", "하와이",
        # English versions
        "Fukuoka", "Tokyo", "Osaka", "Kyoto", "Seoul", "Busan", "Jeju",
    ]
    
    for loc in locations:
        if loc.lower() in query.lower():
            return loc
    
    return None


def _parse_face_count(query: str) -> Tuple[Optional[int], str]:
    """Parse face count from query.
    
    Returns:
        (count, operator) where operator is 'eq', 'gte', 'lte'
    """
    for keyword, count in FACE_KEYWORDS.items():
        if keyword in query:
            if count == -1:  # 단체
                return 5, "gte"
            elif count == -2:  # 가족 등
                return 2, "gte"
            return count, "eq"
    return None, "eq"


def _parse_scene_tags(query: str) -> List[str]:
    """Extract scene tags from query."""
    tags = []
    for ko_word, en_tag in SCENE_KEYWORDS.items():
        if ko_word in query:
            tags.append(en_tag)
    return tags


def parse_photo_query(query: str) -> PhotoSearchCriteria:
    """Parse natural language photo search query.
    
    Examples:
        "작년 겨울 후쿠오카 혼자 사진" 
        → date: 2023-12~2024-02, location: Fukuoka, face_count: 1
        
        "올해 여름 바다 사진"
        → date: 2024-06~2024-08, scene_tags: ["beach"]
    
    Args:
        query: Natural language query in Korean
        
    Returns:
        PhotoSearchCriteria with parsed values
    """
    criteria = PhotoSearchCriteria(raw_query=query)
    
    # Parse date range
    # First try year + season pattern
    date_range = _parse_year_season(query)
    if date_range:
        criteria.date_start, criteria.date_end = date_range
    else:
        # Try individual date keywords
        for keyword, date_fn in DATE_KEYWORDS.items():
            if keyword in query:
                criteria.date_start, criteria.date_end = date_fn()
                break
    
    # Parse location
    criteria.location = _parse_location(query)
    
    # Parse face count
    criteria.face_count, criteria.face_count_op = _parse_face_count(query)
    
    # Parse self-identification filter (requires face registration)
    for keyword, filter_type in SELF_KEYWORDS.items():
        if keyword in query:
            criteria.self_filter = filter_type
            break
    
    # Parse scene tags
    criteria.scene_tags = _parse_scene_tags(query)
    
    # Parse object tags (YOLO classes)
    for ko_word, en_class in OBJECT_KEYWORDS.items():
        if ko_word in query:
            criteria.object_tags.append(en_class)
    
    LOGGER.info(
        "Parsed query '%s' → date: %s~%s, location: %s, faces: %s (%s), self: %s, scenes: %s, objects: %s",
        query,
        criteria.date_start.strftime("%Y-%m-%d") if criteria.date_start else None,
        criteria.date_end.strftime("%Y-%m-%d") if criteria.date_end else None,
        criteria.location,
        criteria.face_count,
        criteria.face_count_op,
        criteria.self_filter,
        criteria.scene_tags,
        criteria.object_tags
    )
    
    return criteria
