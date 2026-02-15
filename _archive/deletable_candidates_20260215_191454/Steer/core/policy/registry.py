"""Registry for managing smart folder configurations."""
from __future__ import annotations

import json
import uuid
from pathlib import Path
from typing import Any, Dict, List, Optional

from core.policy.loader import load_policy_file
from core.policy.models import _normalize_path
from core.utils import get_logger, resolve_repo_root

LOGGER = get_logger("policy.registry")


class SmartFolderRegistry:
    def __init__(self, config_path: Optional[Path] = None):
        if config_path is None:
            config_path = resolve_repo_root() / "core" / "config" / "smart_folders.json"
        self.config_path = config_path
        self._items: List[Dict[str, Any]] = []
        self._reload()

    def _reload(self):
        if self.config_path.exists():
            try:
                self._items = load_policy_file(self.config_path)
            except Exception as e:
                LOGGER.error(f"Failed to load smart folders from {self.config_path}: {e}")
                self._items = []
        else:
            self._items = []

    def save(self):
        """Persist registry to disk safely."""
        self.config_path.parent.mkdir(parents=True, exist_ok=True)
        temp_path = self.config_path.with_suffix(".tmp")
        try:
            with temp_path.open("w", encoding="utf-8") as f:
                json.dump(self._items, f, indent=2, ensure_ascii=False)
            temp_path.replace(self.config_path)
            LOGGER.info(f"Saved {len(self._items)} smart folders to {self.config_path}")
        except Exception as e:
            LOGGER.error(f"Failed to save smart folders: {e}")
            if temp_path.exists():
                temp_path.unlink()
            raise

    def list_folders(self) -> List[Dict[str, Any]]:
        return list(self._items)

    def add_folder(self, path: Path, *, label: str = "", folder_type: str = "general") -> Dict[str, Any]:
        """Register a new smart folder."""
        abs_path = _normalize_path(path)
        
        # Check if already exists
        for item in self._items:
            existing_path = item.get("path")
            if existing_path and _normalize_path(Path(existing_path)) == abs_path:
                LOGGER.warning(f"Smart folder already exists: {abs_path}")
                return item

        new_entry = {
            "id": str(uuid.uuid4()),
            "path": str(abs_path),
            "label": label or abs_path.name,
            "type": folder_type,
            "scope": "policy",
            "agents": ["meeting", "photo", "knowledge_search"],  # Default to all, policy can restrict
            "allow_types": [],  # Empty means all allowed by default agent rules (or none?)
                                # Actually engine allows everything if list is empty?
                                # Let's refer to engine logic. 
            "security": {
                 "pii_filter": True
            },
            "indexing": {
                "mode": "realtime"
            }
        }
        self._items.append(new_entry)
        self.save()
        return new_entry

    def remove_folder(self, path: Path) -> bool:
        """Remove a smart folder by path."""
        abs_path = _normalize_path(path)
        initial_count = len(self._items)
        self._items = [
            item for item in self._items
            if _normalize_path(Path(item.get("path", ""))) != abs_path
        ]
        if len(self._items) < initial_count:
            self.save()
            return True
        return False

    def get_by_path(self, path: Path) -> Optional[Dict[str, Any]]:
        abs_path = _normalize_path(path)
        for item in self._items:
             if _normalize_path(Path(item.get("path", ""))) == abs_path:
                 return item
        return None
