"""Face recognition module using insightface for self-identification.

This module allows users to:
1. Register their face ("이게 나야" feature)
2. Find photos containing only them ("내가 혼자 나온 사진")
3. Find photos with them and others ("내가 나온 사진")
"""
from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Dict, List, Optional, Tuple

import numpy as np

from core.utils import get_logger

LOGGER = get_logger("photo.face_recognition")

# Try to import insightface
try:
    import insightface
    from insightface.app import FaceAnalysis
    HAS_INSIGHTFACE = True
except ImportError:
    HAS_INSIGHTFACE = False
    LOGGER.warning("insightface not available. Face recognition disabled.")


# Default path for storing face embeddings
DEFAULT_FACES_DIR = Path.home() / ".infopilot" / "faces"


class FaceRecognizer:
    """Face recognition using insightface ArcFace embeddings."""
    
    _instance: Optional["FaceRecognizer"] = None
    
    SIMILARITY_THRESHOLD = 0.5  # Cosine similarity threshold for face matching
    
    def __init__(self, faces_dir: Optional[Path] = None):
        if not HAS_INSIGHTFACE:
            raise ImportError("insightface not installed. Run: pip install insightface onnxruntime")
        
        self.faces_dir = faces_dir or DEFAULT_FACES_DIR
        self.faces_dir.mkdir(parents=True, exist_ok=True)
        
        self.app: Optional[FaceAnalysis] = None
        self.registered_faces: Dict[str, np.ndarray] = {}
        
    @classmethod
    def get_instance(cls, faces_dir: Optional[Path] = None) -> "FaceRecognizer":
        """Get singleton instance."""
        if cls._instance is None:
            cls._instance = FaceRecognizer(faces_dir)
        return cls._instance
    
    def load(self) -> None:
        """Load face analysis model (lazy loading)."""
        if self.app is not None:
            return
        
        LOGGER.info("Loading insightface model...")
        self.app = FaceAnalysis(
            name="buffalo_l",
            providers=['CPUExecutionProvider']
        )
        self.app.prepare(ctx_id=0, det_size=(640, 640))
        LOGGER.info("Insightface model loaded successfully")
        
        # Load registered faces
        self._load_registered_faces()
    
    def _load_registered_faces(self) -> None:
        """Load previously registered faces from disk."""
        faces_file = self.faces_dir / "registered_faces.json"
        embeddings_dir = self.faces_dir / "embeddings"
        
        if not faces_file.exists():
            return
        
        try:
            with open(faces_file) as f:
                face_meta = json.load(f)
            
            for name, info in face_meta.items():
                emb_file = embeddings_dir / f"{name}.npy"
                if emb_file.exists():
                    embedding = np.load(emb_file)
                    self.registered_faces[name] = embedding
                    LOGGER.info("Loaded registered face: %s", name)
        except Exception as e:
            LOGGER.error("Failed to load registered faces: %s", e)
    
    def _save_registered_faces(self) -> None:
        """Save registered faces to disk."""
        faces_file = self.faces_dir / "registered_faces.json"
        embeddings_dir = self.faces_dir / "embeddings"
        embeddings_dir.mkdir(parents=True, exist_ok=True)
        
        face_meta = {}
        for name, embedding in self.registered_faces.items():
            emb_file = embeddings_dir / f"{name}.npy"
            np.save(emb_file, embedding)
            face_meta[name] = {
                "name": name,
                "embedding_file": str(emb_file)
            }
        
        with open(faces_file, 'w') as f:
            json.dump(face_meta, f, ensure_ascii=False, indent=2)
    
    def get_face_embedding(self, image_path: Path) -> Optional[np.ndarray]:
        """Extract face embedding from image.
        
        Args:
            image_path: Path to image file
            
        Returns:
            Face embedding vector, or None if no face found
        """
        if not HAS_INSIGHTFACE:
            return None
        
        try:
            self.load()
            
            import cv2
            img = cv2.imread(str(image_path))
            if img is None:
                return None
            
            faces = self.app.get(img)
            
            if not faces:
                return None
            
            # Return the first (largest) face embedding
            return faces[0].embedding
            
        except Exception as e:
            LOGGER.error("Failed to get face embedding: %s", e)
            return None
    
    def get_all_face_embeddings(self, image_path: Path) -> List[np.ndarray]:
        """Extract all face embeddings from image.
        
        Args:
            image_path: Path to image file
            
        Returns:
            List of face embedding vectors
        """
        if not HAS_INSIGHTFACE:
            return []
        
        try:
            self.load()
            
            import cv2
            img = cv2.imread(str(image_path))
            if img is None:
                return []
            
            faces = self.app.get(img)
            return [face.embedding for face in faces]
            
        except Exception as e:
            LOGGER.error("Failed to get face embeddings: %s", e)
            return []
    
    def register_face(self, name: str, image_path: Path) -> bool:
        """Register a face with a name.
        
        Args:
            name: Name for this face (e.g., "나", "me")
            image_path: Path to image containing the face
            
        Returns:
            True if registration successful
        """
        embedding = self.get_face_embedding(image_path)
        if embedding is None:
            LOGGER.error("No face found in image for registration")
            return False
        
        self.registered_faces[name] = embedding
        self._save_registered_faces()
        LOGGER.info("Registered face: %s", name)
        return True
    
    def compare_faces(
        self,
        embedding1: np.ndarray,
        embedding2: np.ndarray
    ) -> float:
        """Compare two face embeddings.
        
        Args:
            embedding1: First face embedding
            embedding2: Second face embedding
            
        Returns:
            Cosine similarity (0.0 to 1.0)
        """
        # Normalize embeddings
        e1 = embedding1 / np.linalg.norm(embedding1)
        e2 = embedding2 / np.linalg.norm(embedding2)
        
        # Cosine similarity
        similarity = np.dot(e1, e2)
        return float(similarity)
    
    def is_same_person(
        self,
        embedding1: np.ndarray,
        embedding2: np.ndarray,
        threshold: Optional[float] = None
    ) -> bool:
        """Check if two embeddings are the same person.
        
        Args:
            embedding1: First face embedding
            embedding2: Second face embedding
            threshold: Similarity threshold
            
        Returns:
            True if likely the same person
        """
        threshold = threshold or self.SIMILARITY_THRESHOLD
        similarity = self.compare_faces(embedding1, embedding2)
        return similarity >= threshold
    
    def find_person_in_photo(
        self,
        name: str,
        image_path: Path,
        threshold: Optional[float] = None
    ) -> Tuple[bool, int]:
        """Check if a registered person is in the photo.
        
        Args:
            name: Registered face name
            image_path: Path to image
            threshold: Similarity threshold
            
        Returns:
            (found: bool, total_faces: int)
        """
        if name not in self.registered_faces:
            LOGGER.warning("Face '%s' not registered", name)
            return False, 0
        
        threshold = threshold or self.SIMILARITY_THRESHOLD
        target_embedding = self.registered_faces[name]
        
        embeddings = self.get_all_face_embeddings(image_path)
        if not embeddings:
            return False, 0
        
        for emb in embeddings:
            if self.is_same_person(target_embedding, emb, threshold):
                return True, len(embeddings)
        
        return False, len(embeddings)
    
    def find_photos_with_person(
        self,
        name: str,
        photos: List[Path],
        alone_only: bool = False
    ) -> List[Path]:
        """Find photos containing a specific person.
        
        Args:
            name: Registered face name
            photos: List of photo paths to search
            alone_only: If True, only return photos where person is alone
            
        Returns:
            List of matching photo paths
        """
        if name not in self.registered_faces:
            LOGGER.warning("Face '%s' not registered", name)
            return []
        
        results = []
        for photo in photos:
            found, total_faces = self.find_person_in_photo(name, photo)
            
            if found:
                if alone_only:
                    if total_faces == 1:
                        results.append(photo)
                else:
                    results.append(photo)
        
        LOGGER.info("Found %d photos with '%s' (alone_only=%s)", len(results), name, alone_only)
        return results


def register_my_face(image_path: Path) -> bool:
    """Convenience function to register user's face as "나".
    
    Args:
        image_path: Path to image containing user's face
        
    Returns:
        True if successful
    """
    if not HAS_INSIGHTFACE:
        LOGGER.error("insightface not available")
        return False
    
    recognizer = FaceRecognizer.get_instance()
    return recognizer.register_face("나", image_path)


def find_my_photos(photos: List[Path], alone_only: bool = False) -> List[Path]:
    """Find photos containing the registered user.
    
    Args:
        photos: List of photo paths
        alone_only: If True, only photos where user is alone
        
    Returns:
        List of matching photo paths
    """
    if not HAS_INSIGHTFACE:
        return []
    
    recognizer = FaceRecognizer.get_instance()
    return recognizer.find_photos_with_person("나", photos, alone_only=alone_only)
