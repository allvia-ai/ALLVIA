
"""
Drift Detection Module
Identifies file changes (drift) by comparing current file system state against the Chunk Cache.
"""
import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Dict, Set, Any, Optional

from core.data_pipeline.cache_manager import ChunkCache, SQLiteChunkCache

@dataclass
class DriftState:
    added: List[str] = field(default_factory=list)
    modified: List[str] = field(default_factory=list)
    deleted: List[str] = field(default_factory=list)
    unchanged: List[str] = field(default_factory=list)
    
    @property
    def has_changes(self) -> bool:
        return bool(self.added or self.modified or self.deleted)
        
    def summary(self) -> str:
        return f"DriftState(added={len(self.added)}, modified={len(self.modified)}, deleted={len(self.deleted)}, unchanged={len(self.unchanged)})"


class DriftDetector:
    def __init__(self, cache_path: Path, use_sqlite: bool = False):
        if use_sqlite:
            self.cache = SQLiteChunkCache(cache_path)
        else:
            self.cache = ChunkCache(cache_path)
            
    def detect(self, scanned_files: List[Dict[str, Any]]) -> DriftState:
        """
        Compare scanned files against cache to detect drift.
        scanned_files: List of dicts with 'path', 'mtime', 'size', etc.
        """
        # 1. Build map of current files
        current_map: Dict[str, Dict[str, Any]] = {}
        for f in scanned_files:
            p = str(f.get("path") or "")
            if p:
                current_map[p] = f
                
        # 2. Get cached paths
        # Accessing private _entries is allowed for this friend class helper
        # But SQLite cache doesn't expose _entries directly. 
        # Ideally we rely on known_paths() or similar public API if possible.
        # SQLiteChunkCache has known_paths(). ChunkCache has it too.
        
        cached_paths = self.cache.known_paths()
        
        added: List[str] = []
        modified: List[str] = []
        unchanged: List[str] = []
        
        # 3. Check for Added and Modified
        # Ideally we need mtime check for "modified candidate" 
        # But real confirmation comes from Hash which is expensive.
        # So we use mtime heuristic first as "Modified Candidate".
        # IF hash later proves same, it moves to unchanged.
        # BUT DriftDetector here is pre-hash phase (usually).
        # OR is this post-scan?
        # If we have hashes in scanned_files, we use them.
        
        # Assuming scanned_files comes from filefinder/scanner which might NOT have content hash yet.
        # If we don't have content hash, we rely on mtime/size.
        
        if hasattr(self.cache, "_entries") and isinstance(self.cache, ChunkCache):
            # Optimised map access for JSON cache
            cache_map = self.cache._entries
            for path, meta in current_map.items():
                if path not in cache_map:
                    added.append(path)
                    continue
                    
                cached_entry = cache_map[path]
                
                # Check 1: Mtime/Size from scan vs Cache Updated At?
                # Cache entry stores 'updated_at' which is usually scan time or mtime?
                # ChunkCache entry has: path, doc_hash, chunk_count, updated_at (time.time())
                # It does NOT store file mtime! This is a gap in CacheEntry schema if we want pure mtime check.
                # However, incremental.py stores mtime.
                
                # Strategy: 
                # If path exists in cache:
                #   We assume it is UNCHANGED unless mtime logic (external) says otherwise?
                #   But ChunkCache is unrelated to mtime. It is related to CONTENT HASH.
                
                # So DriftDetector must essentially just separate "New Paths" vs "Known Paths".
                # To detect ACTUAL modification without reading file, we need mtime cache (incremental.py).
                
                # Assuming DriftDetector is used AFTER `incremental.py` filters `to_process`.
                # If so, `scanned_files` passed here are only the ones `incremental.py` thinks changed?
                # No, that would be circular.
                
                # Let's align:
                # 1. Scanner finds all files.
                # 2. Incremental filters based on mtime (incremental.py). -> Returns `to_process` (Dirty candidates).
                # 3. DriftDetector should classify these `to_process` into Added vs Modified?
                #    And identify Deleted.
                
                pass

        # Let's simplify: DriftDetector detects structure changes (Add/Del).
        # Modification is detected if it's in both but flagged dirty by Scanner/Incremental.
        
        # Identify Deleted
        deleted = [p for p in cached_paths if p not in current_map]
        
        # Identify Added/Modified based solely on cache presence
        for path in current_map:
            if path in cached_paths:
                # It is "Known". Whether it is modified depends on caller (hashing).
                # But for the purpose of "Indexing List", we treat as modified candidate or unchanged.
                # We'll put in 'unchanged' bucket here if we don't check hash.
                unchanged.append(path) 
            else:
                added.append(path)
                
        # Re-shuffle: If caller provides dirty_paths (from incremental scan), we move them from unchanged to modified.
        return DriftState(
            added=added,
            modified=[], # Caller fills this or we assume Mtime diff logic here?
            deleted=deleted,
            unchanged=unchanged
        )

    def detect_with_incremental(self, scanned_files: List[Dict[str, Any]], dirty_candidates: List[Dict[str, Any]]) -> DriftState:
        """
        Combine scan results with incremental candidates to give full picture.
        scanned_files: All files currently on disk.
        dirty_candidates: Files that incremental logic thinks are changed (mtime/size).
        """
        base_drift = self.detect(scanned_files)
        
        dirty_paths = {str(d.get("path")) for d in dirty_candidates}
        
        # Move dirty from unchanged to modified
        final_unchanged = []
        for p in base_drift.unchanged:
            if p in dirty_paths:
                base_drift.modified.append(p)
            else:
                final_unchanged.append(p)
        base_drift.unchanged = final_unchanged
        
        return base_drift
