# core/utils/nlp.py
from __future__ import annotations
import re
from typing import List, Set
from core.utils.stopwords import STOPWORDS

_TOKEN_PATTERN = r'(?u)(?:[가-힣]{1,}|[A-Za-z0-9]{2,})'
_TOKEN_REGEX = re.compile(_TOKEN_PATTERN)
_WHITESPACE_REGEX = re.compile(r"\s+")

class TextCleaner:
    @staticmethod
    def clean(s: str) -> str:
        return _WHITESPACE_REGEX.sub(" ", s).strip()

def split_tokens(source: str) -> List[str]:
    """Split string into tokens, filtering stopwords."""
    if not source:
        return []
    
    # Simple regex based tokenization
    candidates = _TOKEN_REGEX.findall(source)
    tokens = []
    
    for t in candidates:
        lowered = t.lower()
        if lowered in STOPWORDS:
            continue
        tokens.append(t)  # Preserve original case if needed? Original logic seemed to mix handling.
                          # BM25 usually prefers raw terms or lowercased.
                          # Original _split_tokens in retriever did: 
                          # tokens = [t for t in _TOKEN_REGEX.findall(text) if t.lower() not in _STOPWORDS]
                          # It returned original case tokens.
    
    return tokens

# Helper for unique set of tokens
def unique_tokens(source: str) -> Set[str]:
    return set(split_tokens(source))
