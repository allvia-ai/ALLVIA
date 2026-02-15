"""CLIP-based scene tagging for photos."""
from __future__ import annotations

import os
from pathlib import Path
from typing import List, Optional, Tuple
from functools import lru_cache

from core.utils import get_logger

LOGGER = get_logger("photo.clip_tagger")

# Scene categories for classification
SCENE_CATEGORIES = [
    "여행",      # travel
    "음식",      # food
    "가족",      # family
    "인물",      # portrait/people
    "풍경",      # landscape
    "도시",      # city/urban
    "해변",      # beach
    "산",        # mountain
    "동물",      # animal
    "실내",      # indoor
    "기타",      # other
]

# English labels for CLIP (CLIP is trained on English)
SCENE_LABELS_EN = [
    "travel vacation trip holiday",
    "food meal dish restaurant cooking",
    "family gathering celebration party",
    "portrait person people selfie face",
    "landscape nature scenery outdoor view",
    "city urban building architecture street",
    "beach ocean sea coast water",
    "mountain hiking nature peak",
    "animal pet dog cat bird",
    "indoor room home interior",
    "miscellaneous other general",
]

# Try to import optional dependencies
try:
    import torch
    from PIL import Image
    from transformers import CLIPProcessor, CLIPModel
    HAS_CLIP = True
except ImportError:
    HAS_CLIP = False
    LOGGER.warning("CLIP dependencies not available. Scene tagging disabled.")


class CLIPTagger:
    """CLIP-based image scene tagger."""
    
    _instance: Optional["CLIPTagger"] = None
    
    def __init__(self, model_name: str = "openai/clip-vit-base-patch32"):
        if not HAS_CLIP:
            raise ImportError("CLIP dependencies not installed. Run: pip install transformers torch pillow")
        
        self.model_name = model_name
        self.model: Optional[CLIPModel] = None
        self.processor: Optional[CLIPProcessor] = None
        self.device = "mps" if torch.backends.mps.is_available() else "cpu"
        LOGGER.info("CLIPTagger will use device: %s", self.device)
    
    @classmethod
    def get_instance(cls) -> "CLIPTagger":
        """Get singleton instance."""
        if cls._instance is None:
            cls._instance = CLIPTagger()
        return cls._instance
    
    def load(self) -> None:
        """Load CLIP model (lazy loading)."""
        if self.model is not None:
            return
        
        LOGGER.info("Loading CLIP model: %s", self.model_name)
        self.model = CLIPModel.from_pretrained(self.model_name)
        self.processor = CLIPProcessor.from_pretrained(self.model_name)
        self.model.to(self.device)
        self.model.eval()
        LOGGER.info("CLIP model loaded successfully")
    
    def tag_image(self, path: Path, top_k: int = 2) -> List[str]:
        """Classify image into scene categories.
        
        Args:
            path: Path to image file
            top_k: Number of top categories to return
            
        Returns:
            List of Korean scene tags
        """
        if not HAS_CLIP:
            return ["기타"]
        
        try:
            self.load()
            
            image = Image.open(path).convert("RGB")
            inputs = self.processor(
                text=SCENE_LABELS_EN,
                images=image,
                return_tensors="pt",
                padding=True
            )
            
            # Move to device
            inputs = {k: v.to(self.device) for k, v in inputs.items()}
            
            with torch.no_grad():
                outputs = self.model(**inputs)
                logits_per_image = outputs.logits_per_image
                probs = logits_per_image.softmax(dim=1)
            
            # Get top-k categories
            top_indices = probs[0].argsort(descending=True)[:top_k].tolist()
            tags = [SCENE_CATEGORIES[i] for i in top_indices]
            
            return tags
        except Exception as e:
            LOGGER.debug("CLIP tagging failed for %s: %s", path, e)
            return ["기타"]
    
    def get_embedding(self, path: Path) -> Optional[List[float]]:
        """Get image embedding vector for similarity comparison.
        
        Returns:
            Normalized embedding vector or None on failure
        """
        if not HAS_CLIP:
            return None
        
        try:
            self.load()
            
            image = Image.open(path).convert("RGB")
            inputs = self.processor(images=image, return_tensors="pt")
            inputs = {k: v.to(self.device) for k, v in inputs.items()}
            
            with torch.no_grad():
                image_features = self.model.get_image_features(**inputs)
                # Normalize
                image_features = image_features / image_features.norm(dim=-1, keepdim=True)
            
            return image_features[0].cpu().tolist()
        except Exception as e:
            LOGGER.debug("CLIP embedding failed for %s: %s", path, e)
            return None


def tag_photo(path: Path) -> Tuple[List[str], Optional[List[float]]]:
    """Tag a photo with scene categories and get embedding.
    
    Returns:
        (tags, embedding) tuple
    """
    if not HAS_CLIP or os.getenv("PHOTO_SKIP_CLIP", "0") == "1":
        return (["기타"], None)
    
    try:
        tagger = CLIPTagger.get_instance()
        tags = tagger.tag_image(path)
        embedding = tagger.get_embedding(path)
        return (tags, embedding)
    except Exception as e:
        LOGGER.warning("Photo tagging failed: %s", e)
        return (["기타"], None)
