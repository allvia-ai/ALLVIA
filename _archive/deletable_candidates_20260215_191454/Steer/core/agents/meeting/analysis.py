# core/agents/meeting/analysis.py
"""
Heuristic analysis and post-processing for meeting transcripts.
Extracts highlights, action items, and decisions using keyword matching.
"""
from __future__ import annotations

import logging
import re
from typing import Dict, List, Optional, Sequence, Tuple, Union

LOGGER = logging.getLogger(__name__)

# Constants extracted from pipeline.py (assumed)
# We will duplicate them here or import if they were in a constants file. 
# Assuming they were inline or imported in pipeline.py.
# Let's define standard sets here for completeness or placeholders.

DEFAULT_LANGUAGE = "en"
LANGUAGE_ALIASES = {
    "ko": "ko", "korean": "ko",
    "en": "en", "english": "en",
    "ja": "ja", "japanese": "ja",
    "zh": "zh", "chinese": "zh",
}

ACTION_KEYWORDS = {
    "en": ["action item", "todo", "to-do", "task", "assign", "will do", "deadline", "due by"],
    "ko": ["\ud5c9\uc77c", "\ud560 \uc77c", "\uacfc\uc81c", "\ub2f4\ub2f9", "\uae30\ud5dd", "\uc608\uc815", "\ud574\uc57c"],
}
DECISION_KEYWORDS = {
    "en": ["decided", "decision", "agreed", "consensus", "conclusion", "approved", "adopted"],
    "ko": ["\uacb0\uc815", "\ud569\uc758", "\uacb0\ub860", "\ud655\uc815", "\uc2b9\uc778", "\ucc44\ud0dd"],
}


def map_language_code(value: Optional[str]) -> Optional[str]:
    if not value:
        return None
    code = value.lower().strip()
    mapped = LANGUAGE_ALIASES.get(code)
    if mapped:
        return mapped
    if code and code.split("-")[0] in LANGUAGE_ALIASES:
        return LANGUAGE_ALIASES[code.split("-")[0]]
    return None


def format_timestamp(seconds: float) -> str:
    """Format seconds as HH:MM:SS."""
    m, s = divmod(int(seconds), 60)
    h, m = divmod(m, 60)
    return f"{h:02d}:{m:02d}:{s:02d}"


def keywords_for(language: str, source: Dict[str, List[str]]) -> List[str]:
    return source.get(language, source["en"])


def score_highlight(text: str, language: str) -> float:
    words = text.split()
    if len(words) < 5:
        return 0.0
    
    # Simple heuristic scoring
    score = len(words) * 0.1
    # Check for "important" keywords
    importance_markers = {
        "en": ["important", "key", "main", "highlight", "significant", "crucial"],
        "ko": ["\uc911\uc694", "\ud3ec\uc778\ud2b8", "\ud575\uc2ec", "\uac15\uc870"],
    }
    markers = keywords_for(language, importance_markers)
    lower = text.lower()
    for m in markers:
        if m in lower:
            score += 2.0
            
    return score


def extract_highlights(segments: Sequence[dict], language: str) -> List[dict]:
    scored: List[Tuple[float, dict]] = []
    for segment in segments:
        text = (segment.get("text") or "").strip()
        if not text:
            continue
        valid_lang = map_language_code(language) or DEFAULT_LANGUAGE
        score = score_highlight(text, valid_lang)
        if score <= 0:
            continue
        scored.append(
            (
                score,
                {
                    "text": text,
                    "ref": format_timestamp(segment.get("start", 0.0)),
                },
            )
        )

    scored.sort(key=lambda item: item[0], reverse=True)
    top_entries = [entry for _score, entry in scored[:3]]
    return top_entries


def extract_action_items(segments: Sequence[dict], language: str) -> List[dict]:
    valid_lang = map_language_code(language) or DEFAULT_LANGUAGE
    keywords = keywords_for(valid_lang, ACTION_KEYWORDS)
    return collect_by_keywords(segments, keywords)


def extract_decisions(segments: Sequence[dict], language: str) -> List[dict]:
    valid_lang = map_language_code(language) or DEFAULT_LANGUAGE
    keywords = keywords_for(valid_lang, DECISION_KEYWORDS)
    return collect_by_keywords(segments, keywords)


def collect_by_keywords(
    segments: Sequence[dict],
    keywords: Sequence[str],
) -> List[dict]:
    lowered_keywords = [kw.lower() for kw in keywords]
    scored: List[Tuple[float, dict]] = []
    
    for segment in segments:
        raw_text = segment.get("text")
        text = (raw_text or "").strip()
        if not text:
            continue
        lowered = text.lower()
        match_count = sum(lowered.count(keyword) for keyword in lowered_keywords)
        if match_count == 0:
            continue
        
        # Simple score based on match density and length
        score = match_count * 1.5 + (len(text.split()) * 0.05)
        
        scored.append(
            (
                score,
                {
                    "text": text,
                    "ref": format_timestamp(segment.get("start", 0.0)),
                },
            )
        )

    scored.sort(key=lambda item: item[0], reverse=True)
    return [entry for _score, entry in scored[:5]]


def parse_and_merge_structure(text: str) -> Dict[str, List[dict]]:
    """Parse markdown sections for Highlights, Decisions, and Action Items."""
    result: Dict[str, List[dict]] = {"highlights": [], "decisions": [], "action_items": []}
    
    # Regex to find sections like '## Highlights' followed by bullet points
    section_pattern = re.compile(r"^\s*##+\s+(.*?)\s*\n((?:\s*[-*]\s+.*(?:\n|$))+)", re.MULTILINE)
    
    for match in section_pattern.finditer(text):
        header = match.group(1).lower().strip()
        content = match.group(2).strip()
        items = []
        for line in content.split('\n'):
            cleaned = line.lstrip("-* ").strip()
            if cleaned:
                items.append({"text": cleaned, "timestamp": 0.0}) # Heuristic timestamp
        
        if "highlight" in header:
            result["highlights"] = items
        elif "decision" in header:
            result["decisions"] = items
        elif "action" in header:
            result["action_items"] = items
            
    return result
