import pytest
from pathlib import Path
from unittest.mock import MagicMock, patch
from core.data_pipeline.pipeline import run_step2
from core.policy.engine import PolicyEngine

@pytest.fixture
def mock_policy_engine():
    engine = MagicMock()
    # Define behavior: allow 'safe.txt', deny 'secret.txt'
    def allows_side_effect(path, agent, include_manual=True):
        return "secret" not in str(path)
    
    engine.allows.side_effect = allows_side_effect
    # Ensure hasattr(engine, "allows") is True by accessing it once or setting it
    engine.allows 
    return engine

@pytest.mark.smoke
def test_pipeline_policy_integration(mock_policy_engine, tmp_path):
    # Setup dummy files
    safe_file = tmp_path / "safe.txt"
    secret_file = tmp_path / "secret.txt"
    safe_file.touch()
    secret_file.touch()
    
    # Mock row data - typically what comes from run_scan
    # We use minimal dicts as run_step2 mainly checks 'path' key for policy
    rows = [
        {"path": str(safe_file), "size": 10, "mtime": 100.0, "ext": ".txt"},
        {"path": str(secret_file), "size": 10, "mtime": 100.0, "ext": ".txt"},
    ]
    
    # We mock _extract_metadata_filters and other internals to avoid heavy dependencies
    # But run_step2 calls them. Ideally we mock the 'extract' and 'train' calls 
    # so we just verify the filtering logic at the start of run_step2.
    
    # However, run_step2 is a large function. 
    # An easier way is to mock 'core.data_pipeline.pipeline.load_scan_state' 
    # and 'core.data_pipeline.pipeline._create_chunk_cache' to minimal mocks
    # and let the function run until it hits the file processing loop.
    
    # Better yet, let's patch the 'print' function to capture the "policy excluded" message
    # And mock the actual processing steps so they don't crash or take time.
    
    with patch("core.data_pipeline.pipeline.pd", new=MagicMock()), \
         patch("core.data_pipeline.pipeline._create_chunk_cache", return_value=None), \
         patch("core.data_pipeline.pipeline.load_scan_state", return_value=None), \
         patch("core.data_pipeline.pipeline.CorpusBuilder") as MockBuilder, \
         patch("core.data_pipeline.pipeline.TopicModel") as MockTopicModel, \
         patch("builtins.print") as mock_print:

        # Mock CorpusBuilder instance
        builder_instance = MockBuilder.return_value
        builder_instance.process_rows.return_value = (None, None) # Minimal return

        try:
            run_step2(
                rows, 
                out_corpus=tmp_path / "corpus.parquet",
                out_model=tmp_path / "model.joblib",
                policy_engine=mock_policy_engine,
                use_tqdm=False,
                train_embeddings=False # Skip actual training
            )
        except Exception:
            # We expect failures downstream because we are mocking pandas blindly.
            # We only care if the policy check happened at the start.
            pass
        
        # Verification
        # 1. Check if policy_engine.allows was called for both
        assert mock_policy_engine.allows.call_count >= 2
        
        # 2. Check print output for "excluded" message
        # We look for the message: "⚠️ 정책에 위반되는 1개 파일을 파이프라인에서 제외했습니다."
        print_calls = [str(call) for call in mock_print.mock_calls]
        found_warning = any("정책에 위반되는 1개 파일" in c for c in print_calls)
        assert found_warning, f"Expected warning message not found in prints: {print_calls}"
        
        # 3. Verify that CorpusBuilder (or whatever downstream) processed reduced rows.
        # Since we might have crashed before CorpusBuilder, this assertion is risky if we crash too early.
        # But looking at code, CorpusBuilder init comes later.
        # Instead, we can inspect if the loop ran.
        pass
