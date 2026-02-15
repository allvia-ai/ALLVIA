"""Smart Folder + Policy aware scanner."""
from __future__ import annotations

import os
import time
import hashlib
import platform
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Set, Any

try:
    import pwd
except ImportError:
    pwd = None

# Default extensions from legacy FileFinder
DEFAULT_EXTS = {
    ".hwp", ".doc", ".docx",
    ".xlsx", ".xls", ".xlsm", ".xlsb", ".xltx",
    ".pdf",
    ".ppt", ".pptx",
    ".csv", ".txt", ".md", ".rst", ".json",
}

SKIP_DIRS = {
    "__pycache__", ".git", ".svn", ".hg", ".idea", ".vscode", "node_modules",
    "venv", ".venv", "env", ".env", "dist", "build", "site-packages"
}

@dataclass
class ScanConfig:
    roots: List[Path]
    exts: Optional[Iterable[str]] = None
    allow_hash: bool = False

@dataclass
class ScanResult:
    path: Path
    size: int
    mtime: float
    allowed: bool
    deny_reason: str = ""
    content_hash: str = ""

    def to_row(self) -> Dict[str, object]:
        return {
            "path": str(self.path),
            "size": self.size,
            "mtime": self.mtime,
            "allowed": int(self.allowed),
            "deny_reason": self.deny_reason,
            "hash": self.content_hash,
        }

def _hash_file(path: Path) -> str:
    try:
        data = path.read_bytes()
        return hashlib.sha256(data).hexdigest()
    except OSError:
        return ""

def _resolve_owner(stat_result) -> str:
    if not stat_result or pwd is None:
        return ""
    try:
        return pwd.getpwuid(stat_result.st_uid).pw_name
    except (KeyError, AttributeError):
        return ""

def collect_file_metadata(path: Path, *, allowed_exts: Optional[Iterable[str]] = None) -> Optional[Dict[str, Any]]:
    """Build a single metadata row for a file."""
    try:
        p = Path(path).expanduser().resolve(strict=True)
    except (OSError, RuntimeError):
        return None

    if allowed_exts:
        valid_exts = {e.lower() if e.startswith(".") else f".{e.lower()}" for e in allowed_exts}
        if p.suffix.lower() not in valid_exts:
            return None
    
    try:
        st = p.stat()
    except (FileNotFoundError, PermissionError, OSError):
        return None
        
    return {
        "path": str(p),
        "size": st.st_size,
        "mtime": st.st_mtime,
        "ctime": st.st_ctime,
        "ext": p.suffix.lower(),
        "drive": p.anchor,
        "owner": _resolve_owner(st),
    }

def scan_directory(root: Path, exts: Optional[Iterable[str]] = None) -> List[Dict[str, Any]]:
    """Legacy-compatible scanner function. Returns list of dicts."""
    target_exts = set(exts or DEFAULT_EXTS)
    target_exts = {e.lower() if e.startswith(".") else e.lower() for e in target_exts}
    
    results = []
    
    # Simple recursive walk
    # Use os.walk for simplicity and robustness
    for current_root, dirs, files in os.walk(root):
        # Filter directories in-place
        # SKIP_DIRS 제외 & .으로 시작하는 숨김 폴더 제외 (단, .ai_agent는 허용)
        dirs[:] = [
            d for d in dirs
            if d not in SKIP_DIRS and (not d.startswith(".") or d == ".ai_agent")
        ]
        
        for name in files:
            if name.startswith(".") and name != ".htaccess": # Skip hidden files
                continue
                
            _, ext = os.path.splitext(name)
            if ext.lower() not in target_exts:
                continue
                
            full_path = Path(current_root) / name
            try:
                st = full_path.stat()
                results.append({
                    "path": str(full_path),
                    "size": st.st_size,
                    "mtime": st.st_mtime,
                    "ctime": st.st_ctime,
                    "ext": ext.lower(),
                    "drive": full_path.anchor,
                    "owner": _resolve_owner(st)
                })
            except (OSError, PermissionError):
                continue
                
    return results

def run_scan(cfg: ScanConfig, policy_engine: Any = None) -> List[ScanResult]:
    """
    Run scan using configuration and optional policy engine.
    """
    results: List[ScanResult] = []
    targets = cfg.roots
    exts = cfg.exts or DEFAULT_EXTS
    
    scanned_dicts = []
    for root in targets:
        if root.exists():
            scanned_dicts.extend(scan_directory(root, exts))
            
    for rec in scanned_dicts:
        path = Path(rec["path"])
        allowed = True
        deny_reason = ""
        
        if policy_engine and hasattr(policy_engine, "allows"):
            if not policy_engine.allows(path, agent="knowledge_search", include_manual=True):
                allowed = False
                deny_reason = "policy_denied"
                
        content_hash = ""
        if cfg.allow_hash and allowed:
            content_hash = _hash_file(path)
            
        results.append(ScanResult(
            path=path,
            size=int(rec["size"]),
            mtime=float(rec["mtime"]),
            allowed=allowed,
            deny_reason=deny_reason,
            content_hash=content_hash
        ))
        
    return results

