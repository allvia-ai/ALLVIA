import pandas as pd
import pytest

from core.data_pipeline.pipeline import _apply_uniform_chunks


@pytest.mark.full
def test_adaptive_chunk_window_splits_long_documents():
    text = " ".join([f"sentence{i}" for i in range(600)])
    df = pd.DataFrame([{"text": text}])
    chunked = _apply_uniform_chunks(df, min_tokens=64, max_tokens=256)
    assert len(chunked) > 1
    assert chunked.iloc[0]["chunk_tokens"] >= 64


@pytest.mark.full
def test_adaptive_chunk_window_single_chunk_for_short_text():
    df = pd.DataFrame([{"text": "short text with just a few words."}])
    chunked = _apply_uniform_chunks(df, min_tokens=64, max_tokens=256)
    assert len(chunked) == 1


@pytest.mark.full
def test_markdown_heading_chunking_adds_heading_column():
    text = "# Intro\n" + ("word " * 320) + "\n## Details\n" + ("word " * 320)
    df = pd.DataFrame(
        [
            {
                "text": text,
                "text_original": text,
                "ext": ".md",
                "meta": {"format": "markdown"},
            }
        ]
    )
    chunked = _apply_uniform_chunks(df, min_tokens=64, max_tokens=256)
    assert len(chunked) > 1
    assert "heading" in chunked.columns
    headings = {h for h in chunked["heading"].fillna("").astype(str).tolist() if h.strip()}
    assert "Intro" in headings or "Details" in headings
