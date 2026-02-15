
import sys
import unittest
from pathlib import Path

# Add project root to sys.path
project_root = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(project_root))

from core.data_pipeline.chunking_v2 import SemanticChunker, Chunk

class TestSemanticChunker(unittest.TestCase):
    def setUp(self):
        self.chunker = SemanticChunker(max_tokens=50, overlap_tokens=10)

    def test_simple_split(self):
        text = "Hello world. " * 100  # ~200 tokens
        # Should be split because it's long (>50 tokens)
        chunks = self.chunker.chunk_text(text)
        self.assertTrue(len(chunks) > 1, f"Expected split, got {len(chunks)} chunks. Text len: {len(text)}")
        for c in chunks:
            self.assertTrue(c.token_count <= 50)

    def test_markdown_headers(self):
        # ... (same)
        text = """# Header 1
Content 1.
""" + ("Filler text. " * 30) + """
## Header 2
Content 2.
"""
        chunks = self.chunker.chunk_text(text)
        self.assertTrue(len(chunks) >= 2)
        self.assertIn("Header 1", chunks[0].text)
        self.assertIn("Header 2", chunks[-1].text)
        self.assertEqual(chunks[0].metadata.get("heading"), "Header 1")
        self.assertEqual(chunks[-1].metadata.get("heading"), "Header 2")

    def test_paragraph_split(self):
        text = ("Para 1. " * 10) + "\n\n" + ("Para 2. " * 10) + "\n\n" + ("Para 3. " * 10)
        # Total ~30 tokens. Max 10.
        chunker = SemanticChunker(max_tokens=10, overlap_tokens=0, min_tokens=1)
        chunks = chunker.chunk_text(text)
        self.assertTrue(len(chunks) >= 3)

        # Verification of integration logic is done in checking pipeline output structure
        # ensuring chunking v2 returns list of Chunks
        text = "A" * 100
        chunks = self.chunker.chunk_text(text)
        self.assertTrue(isinstance(chunks[0], Chunk))

if __name__ == "__main__":
    unittest.main()
