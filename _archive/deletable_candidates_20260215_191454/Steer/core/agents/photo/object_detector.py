"""Object detection module using YOLO for photo auto-tagging.

Detects objects in photos and returns labels like:
- person, dog, cat, car, bicycle, food, etc.

Uses COCO dataset labels (80 classes).
"""
from __future__ import annotations

import os
from pathlib import Path
from typing import Dict, List, Optional, Tuple

from core.utils import get_logger

LOGGER = get_logger("photo.object_detector")

# Try to import ultralytics YOLO
try:
    from ultralytics import YOLO
    HAS_YOLO = True
except ImportError:
    HAS_YOLO = False
    LOGGER.warning("ultralytics not available. Object detection disabled.")


# COCO class names (80 classes)
COCO_CLASSES = [
    "person", "bicycle", "car", "motorcycle", "airplane", "bus", "train", "truck",
    "boat", "traffic light", "fire hydrant", "stop sign", "parking meter", "bench",
    "bird", "cat", "dog", "horse", "sheep", "cow", "elephant", "bear", "zebra",
    "giraffe", "backpack", "umbrella", "handbag", "tie", "suitcase", "frisbee",
    "skis", "snowboard", "sports ball", "kite", "baseball bat", "baseball glove",
    "skateboard", "surfboard", "tennis racket", "bottle", "wine glass", "cup",
    "fork", "knife", "spoon", "bowl", "banana", "apple", "sandwich", "orange",
    "broccoli", "carrot", "hot dog", "pizza", "donut", "cake", "chair", "couch",
    "potted plant", "bed", "dining table", "toilet", "tv", "laptop", "mouse",
    "remote", "keyboard", "cell phone", "microwave", "oven", "toaster", "sink",
    "refrigerator", "book", "clock", "vase", "scissors", "teddy bear", "hair drier",
    "toothbrush"
]

# Korean to English mapping for search
OBJECT_KEYWORDS_KO = {
    "사람": "person",
    "강아지": "dog",
    "개": "dog",
    "고양이": "cat",
    "자동차": "car",
    "차": "car",
    "자전거": "bicycle",
    "새": "bird",
    "말": "horse",
    "소": "cow",
    "양": "sheep",
    "코끼리": "elephant",
    "곰": "bear",
    "기린": "giraffe",
    "가방": "backpack",
    "우산": "umbrella",
    "넥타이": "tie",
    "스키": "skis",
    "서핑": "surfboard",
    "병": "bottle",
    "와인": "wine glass",
    "컵": "cup",
    "바나나": "banana",
    "사과": "apple",
    "샌드위치": "sandwich",
    "오렌지": "orange",
    "피자": "pizza",
    "도넛": "donut",
    "케이크": "cake",
    "의자": "chair",
    "소파": "couch",
    "식물": "potted plant",
    "침대": "bed",
    "테이블": "dining table",
    "TV": "tv",
    "노트북": "laptop",
    "핸드폰": "cell phone",
    "스마트폰": "cell phone",
    "책": "book",
    "시계": "clock",
    "꽃병": "vase",
    "인형": "teddy bear",
}


class ObjectDetector:
    """Object detection using YOLO model."""
    
    _instance: Optional["ObjectDetector"] = None
    
    DEFAULT_MODEL = "yolov8n.pt"  # Nano model (fastest)
    CONFIDENCE_THRESHOLD = 0.5
    
    def __init__(self, model_name: Optional[str] = None, confidence: float = 0.5):
        if not HAS_YOLO:
            raise ImportError("ultralytics not installed. Run: pip install ultralytics")
        
        self.model_name = model_name or self.DEFAULT_MODEL
        self.confidence = confidence
        self.model: Optional[YOLO] = None
        
    @classmethod
    def get_instance(cls, model_name: Optional[str] = None) -> "ObjectDetector":
        """Get singleton instance."""
        if cls._instance is None:
            cls._instance = ObjectDetector(model_name)
        return cls._instance
    
    def load(self) -> None:
        """Load YOLO model (lazy loading)."""
        if self.model is not None:
            return
        
        LOGGER.info("Loading YOLO model: %s", self.model_name)
        self.model = YOLO(self.model_name)
        LOGGER.info("YOLO model loaded successfully")
    
    def detect(self, image_path: Path) -> List[str]:
        """Detect objects in image and return unique labels.
        
        Args:
            image_path: Path to image file
            
        Returns:
            List of detected object labels
        """
        if not HAS_YOLO:
            return []
        
        try:
            self.load()
            
            # Run inference
            results = self.model(str(image_path), conf=self.confidence, verbose=False)
            
            # Extract unique class names
            labels = set()
            for result in results:
                if result.boxes is not None:
                    for box in result.boxes:
                        cls_id = int(box.cls[0])
                        if cls_id < len(COCO_CLASSES):
                            labels.add(COCO_CLASSES[cls_id])
            
            LOGGER.debug("Detected objects in %s: %s", image_path.name, list(labels))
            return list(labels)
            
        except Exception as e:
            LOGGER.error("Object detection failed for %s: %s", image_path, e)
            return []
    
    def detect_with_counts(self, image_path: Path) -> Dict[str, int]:
        """Detect objects and return counts per class.
        
        Args:
            image_path: Path to image file
            
        Returns:
            Dict of {label: count}
        """
        if not HAS_YOLO:
            return {}
        
        try:
            self.load()
            
            results = self.model(str(image_path), conf=self.confidence, verbose=False)
            
            counts: Dict[str, int] = {}
            for result in results:
                if result.boxes is not None:
                    for box in result.boxes:
                        cls_id = int(box.cls[0])
                        if cls_id < len(COCO_CLASSES):
                            label = COCO_CLASSES[cls_id]
                            counts[label] = counts.get(label, 0) + 1
            
            return counts
            
        except Exception as e:
            LOGGER.error("Object detection failed: %s", e)
            return {}


def detect_objects(image_path: Path) -> List[str]:
    """Convenience function to detect objects in an image.
    
    Args:
        image_path: Path to image file
        
    Returns:
        List of detected object labels
    """
    if not HAS_YOLO:
        return []
    
    try:
        detector = ObjectDetector.get_instance()
        return detector.detect(image_path)
    except Exception as e:
        LOGGER.error("Object detection failed: %s", e)
        return []


def translate_object_query(query: str) -> List[str]:
    """Translate Korean object keywords to English COCO labels.
    
    Args:
        query: Korean search query
        
    Returns:
        List of matching COCO class names
    """
    objects = []
    query_lower = query.lower()
    
    for ko, en in OBJECT_KEYWORDS_KO.items():
        if ko in query_lower:
            objects.append(en)
    
    return objects


def get_korean_label(english_label: str) -> str:
    """Get Korean label for display.
    
    Args:
        english_label: COCO class name in English
        
    Returns:
        Korean label if available, else English
    """
    for ko, en in OBJECT_KEYWORDS_KO.items():
        if en == english_label:
            return ko
    return english_label
