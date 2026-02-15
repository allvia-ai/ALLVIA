
import sys
import unittest
import tempfile
import shutil
import time
from pathlib import Path

# Setup paths
project_root = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(project_root))

from core.data_pipeline.drift import DriftDetector
from core.data_pipeline.incremental import update_scan_state, filter_incremental_rows

class TestDriftDetection(unittest.TestCase):
    def setUp(self):
        self.test_dir = Path(tempfile.mkdtemp())
        self.cache_dir = self.test_dir / "cache"
        self.cache_dir.mkdir()
        self.cache_path = self.cache_dir / "chunk_cache.json"
        
        # Create dummy cache file
        self.cache_path.write_text("{}", encoding="utf-8")
        
        # Helper to create mock scanned files
        self.mock_files = []

    def tearDown(self):
        shutil.rmtree(self.test_dir)

    def test_drift_logic(self):
        # 1. Initial State: Cache has File A
        detector = DriftDetector(self.cache_path)
        # Manually populate cache for testing
        detector.cache._entries = {
            "/tmp/fileA.txt": type('obj', (object,), {'doc_hash': 'aaa', 'chunk_count': 1, 'updated_at': 0.0, 'path': '/tmp/fileA.txt'})()
        }
        # Hack to populate known paths since we mocked _entries
        # but know_paths uses keys so we are good.
        
        # 2. Current State: File A (Unchanged), File B (New)
        scanned_files = [
            {"path": "/tmp/fileA.txt", "mtime": 100, "size": 10},
            {"path": "/tmp/fileB.txt", "mtime": 200, "size": 20},
        ]
        
        # 3. Detect
        state = detector.detect(scanned_files)
        
        # Expect: Added=[File B], Unchanged=[File A]
        self.assertIn("/tmp/fileB.txt", state.added)
        self.assertIn("/tmp/fileA.txt", state.unchanged)
        self.assertEqual(len(state.modified), 0)
        self.assertEqual(len(state.deleted), 0)

    def test_deletion(self):
        detector = DriftDetector(self.cache_path)
        detector.cache._entries = {
            "/tmp/fileA.txt": type('obj', (object,), {'doc_hash': 'aaa', 'chunk_count': 1, 'updated_at': 0.0, 'path': '/tmp/fileA.txt'})()
        }
        
        # Scan result is empty (File A deleted)
        scanned_files = [] 
        
        state = detector.detect(scanned_files)
        self.assertIn("/tmp/fileA.txt", state.deleted)

    def test_incremental_filter_integration(self):
        # Scan state logic test
        scan_state = {}
        files = [{"path": "/tmp/f1", "mtime": 100.0, "size": 10}]
        
        # First pass: update logic
        scan_state = update_scan_state(scan_state, files)
        self.assertIn("/tmp/f1", scan_state["paths"])
        
        # Second pass: same file
        to_process, cached = filter_incremental_rows(files, scan_state)
        # Should be empty to_process because it matches state (size=10, mtime=100)
        self.assertEqual(len(to_process), 0)
        self.assertEqual(len(cached), 1)
        
        # Third pass: modified file
        files_mod = [{"path": "/tmp/f1", "mtime": 200.0, "size": 10}]
        to_process, cached = filter_incremental_rows(files_mod, scan_state)
        self.assertEqual(len(to_process), 1)

if __name__ == "__main__":
    unittest.main()
