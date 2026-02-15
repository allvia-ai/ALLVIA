
import sys
import unittest
from pathlib import Path
import tempfile
import shutil
import os

# Add project root
project_root = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(project_root))

class TestScannerRefactor(unittest.TestCase):
    def setUp(self):
        self.test_dir = Path(tempfile.mkdtemp())
        (self.test_dir / "file1.txt").write_text("content")
        (self.test_dir / "file2.md").write_text("# Markdown")
        (self.test_dir / "ignore_me.pyc").write_text("")
        (self.test_dir / ".hidden").write_text("")

    def tearDown(self):
        shutil.rmtree(self.test_dir)

    def test_scanner_import(self):
        """Verify we can import the new scanner module."""
        import core.data_pipeline.scanner as scanner
        self.assertTrue(hasattr(scanner, "run_scan"))
        self.assertTrue(hasattr(scanner, "ScanConfig"))

    def test_scanner_functionality(self):
        """Verify the new scanner actually finds files."""
        import core.data_pipeline.scanner as scanner
        cfg = scanner.ScanConfig(
            roots=[self.test_dir],
            exts=[".txt", ".md"],
            allow_hash=False
        )
        results = scanner.run_scan(cfg)
        paths = [r.path.name for r in results]
        self.assertIn("file1.txt", paths)
        self.assertIn("file2.md", paths)
        self.assertNotIn("ignore_me.pyc", paths)
        self.assertNotIn(".hidden", paths)

    def test_cli_scan_import(self):
        """Verify scripts/pipeline/infopilot_cli/scan.py imports correctly."""
        try:
            import scripts.pipeline.infopilot_cli.scan as cli_scan
        except ImportError as e:
            self.fail(f"Failed to import infopilot_cli.scan: {e}")
        except ModuleNotFoundError as e:
             self.fail(f"Module not found: {e}")

if __name__ == "__main__":
    unittest.main()
