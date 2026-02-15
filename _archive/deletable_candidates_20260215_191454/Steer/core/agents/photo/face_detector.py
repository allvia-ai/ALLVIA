"""Face detection module using mediapipe for counting faces in photos."""
from __future__ import annotations

import os
from pathlib import Path
from typing import List, Optional, Tuple

from core.utils import get_logger

LOGGER = get_logger("photo.face_detector")

# Try to import mediapipe
try:
    import mediapipe as mp
    from PIL import Image
    import numpy as np
    HAS_MEDIAPIPE = True
except ImportError:
    HAS_MEDIAPIPE = False
    LOGGER.warning("mediapipe not available. Face detection disabled.")


class FaceDetector:
    """Face detector using mediapipe FaceDetection."""
    
    _instance: Optional["FaceDetector"] = None
    
    def __init__(self, min_confidence: float = 0.5):
        if not HAS_MEDIAPIPE:
            raise ImportError("mediapipe not installed. Run: pip install mediapipe")
        
        self.min_confidence = min_confidence
        self.detector = None
        
    @classmethod
    def get_instance(cls, min_confidence: float = 0.5) -> "FaceDetector":
        """Get singleton instance."""
        if cls._instance is None:
            cls._instance = FaceDetector(min_confidence)
        return cls._instance
    
    def load(self) -> None:
        """Load face detection model (lazy loading)."""
        if self.detector is not None:
            return
        
        LOGGER.info("Loading mediapipe face detection model...")
        mp_face = mp.solutions.face_detection
        self.detector = mp_face.FaceDetection(
            model_selection=1,  # 0 for short-range, 1 for full-range
            min_detection_confidence=self.min_confidence
        )
        LOGGER.info("Face detection model loaded successfully")
    
    def count_faces(self, image_path: Path) -> int:
        """Count the number of faces in an image.
        
        Args:
            image_path: Path to image file
            
        Returns:
            Number of faces detected
        """
        if not HAS_MEDIAPIPE:
            return -1  # -1 indicates detection not available
        
        try:
            self.load()
            
            # Load and convert image
            img = Image.open(image_path)
            if img.mode != 'RGB':
                img = img.convert('RGB')
            
            img_array = np.array(img)
            
            # Run face detection
            results = self.detector.process(img_array)
            
            if results.detections:
                count = len(results.detections)
                LOGGER.debug("Detected %d face(s) in %s", count, image_path.name)
                return count
            return 0
            
        except Exception as e:
            LOGGER.error("Face detection failed for %s: %s", image_path, e)
            return -1
    
    def detect_faces(self, image_path: Path) -> List[Tuple[float, float, float, float]]:
        """Detect faces and return bounding boxes.
        
        Args:
            image_path: Path to image file
            
        Returns:
            List of (x, y, width, height) normalized bounding boxes
        """
        if not HAS_MEDIAPIPE:
            return []
        
        try:
            self.load()
            
            img = Image.open(image_path)
            if img.mode != 'RGB':
                img = img.convert('RGB')
            
            img_array = np.array(img)
            results = self.detector.process(img_array)
            
            boxes = []
            if results.detections:
                for detection in results.detections:
                    bbox = detection.location_data.relative_bounding_box
                    boxes.append((
                        bbox.xmin,
                        bbox.ymin,
                        bbox.width,
                        bbox.height
                    ))
            
            return boxes
            
        except Exception as e:
            LOGGER.error("Face detection failed for %s: %s", image_path, e)
            return []


def count_faces(image_path: Path) -> int:
    """Convenience function to count faces in an image.
    
    Args:
        image_path: Path to image file
        
    Returns:
        Number of faces detected, or -1 if detection unavailable
    """
    if not HAS_MEDIAPIPE:
        return -1
    
    try:
        detector = FaceDetector.get_instance()
        return detector.count_faces(image_path)
    except Exception as e:
        LOGGER.error("Face detection failed: %s", e)
        return -1


def get_face_count_label(count: int) -> str:
    """Get human-readable label for face count.
    
    Args:
        count: Number of faces
        
    Returns:
        Korean label like "혼자", "둘이서", etc.
    """
    if count < 0:
        return "알 수 없음"
    elif count == 0:
        return "사람 없음"
    elif count == 1:
        return "혼자"
    elif count == 2:
        return "둘이서"
    elif count <= 5:
        return f"{count}명"
    else:
        return "단체 사진"
