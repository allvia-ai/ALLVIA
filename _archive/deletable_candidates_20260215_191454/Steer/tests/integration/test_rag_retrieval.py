import pytest
import shutil
from pathlib import Path
from core.search.retriever import Retriever

@pytest.fixture
def rag_test_env(tmp_path):
    # Setup directories
    corpus_dir = tmp_path / "corpus"
    corpus_dir.mkdir()
    model_dir = tmp_path / "model"
    model_dir.mkdir()
    cache_dir = tmp_path / "cache"
    cache_dir.mkdir()
    
    # Create mock documents
    doc1 = corpus_dir / "project_alpha.txt"
    doc1.write_text("Project Alpha is a secret initiative to build a flying car.", encoding="utf-8")
    
    doc2 = corpus_dir / "project_beta.txt"
    doc2.write_text("Project Beta focuses on underwater exploration and deep sea mining.", encoding="utf-8")
    
    doc3 = corpus_dir / "gundam_specs.md"
    doc3.write_text("RX-78-2 Gundam has a beam rifle and beam saber.", encoding="utf-8")

    return {
        "corpus": corpus_dir,
        "model": model_dir,
        "cache": cache_dir
    }

@pytest.mark.smoke
def test_retriever_hybrid_search(rag_test_env):
    """
    Verify that the retriever can find documents based on keywords.
    Note: Since we are not running a full training step here, and the Retriever usually requires
    pre-built indices (vector/lexical), we need to see if we can trigger an index build or specific search mode.
    The Retriever class has an IndexManager which loads/builds indices.
    For this test, we might need to skip the complex vector index build and rely on a simpler mock or 
    force a rebuild if the retriever supports it easily.
    
    However, building a real embedding index might be slow or require models.
    Let's check if we can verify at least the lexical part or if we need to mock the underlying index.
    
    Actually, `Retriever` uses `IndexManager`. If we can't easily build a real index in a unit test 
    without heavy deps, we should mock `IndexManager` or `VectorIndex`.
    """
    
    # For a reliable unit test without heavy ML model loading, we should mock the internal search index.
    # But to test "Hybrid Search Logic", we want to test the `Retriever._rerank_hits` and combination logic.
    
    from unittest.mock import MagicMock, patch
    
    with patch("core.search.retriever.IndexManager") as MockIndexManager, \
         patch("core.search.retriever.QueryEncoder") as MockEncoder, \
         patch("core.search.retriever.CrossEncoderReranker") as MockReranker:
         
        # Setup Mock Index
        mock_index = MagicMock()
        MockIndexManager.return_value.get_index.return_value = mock_index
        mock_index.doc_ids = [0, 1, 2]
        mock_index.exts = [".txt", ".txt", ".md"]
        
        # Setup Mock Search Results (Vector)
        # Query: "flying car" -> matches doc1
        mock_index.search.return_value = [
            {"doc_id": 0, "path": str(rag_test_env["corpus"] / "project_alpha.txt"), "preview": "build a flying car", "score": 0.9, "ext": ".txt"},
            {"doc_id": 2, "path": str(rag_test_env["corpus"] / "gundam_specs.md"), "preview": "beam rifle", "score": 0.1, "ext": ".md"}
        ]
        
        retriever = Retriever(
            model_path=rag_test_env["model"],
            corpus_path=rag_test_env["corpus"],
            cache_dir=rag_test_env["cache"],
            use_rerank=False # Disable reranker for basic hybrid test first
        )
        
        # Run Search
        hits = retriever.search("flying car", top_k=2)
        
        # Verify
        assert len(hits) > 0
        assert "project_alpha" in str(hits[0]["path"])
        item = hits[0]
        assert item["score"] > 0.0
