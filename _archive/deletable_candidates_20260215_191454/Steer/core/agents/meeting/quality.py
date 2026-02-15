"""Quality metric helpers for meeting summaries."""
from __future__ import annotations

import re
from collections import Counter
from typing import Dict, List, Optional, Sequence, Tuple

from .constants import QUESTION_STOP_WORDS
from .models import MeetingSummary, MeetingTranscriptionResult


def compute_quality_metrics(
    transcription: MeetingTranscriptionResult,
    summary: MeetingSummary,
) -> Dict[str, float | int | str]:
    transcript_chars = len(transcription.text or "")
    summary_chars = len(summary.raw_summary or "")
    compression = summary_chars / transcript_chars if transcript_chars else 0.0
    rouge_scores = compute_rouge_metrics(transcription.text, summary.raw_summary)
    lfqa_scores = estimate_lfqa_metrics(transcription.text, summary.raw_summary)

    metrics: Dict[str, float | int | str] = {
        "transcript_chars": transcript_chars,
        "summary_chars": summary_chars,
        "compression_ratio": round(compression, 4) if compression else 0.0,
        "highlight_count": len(summary.highlights),
        "action_count": len(summary.action_items),
        "decision_count": len(summary.decisions),
    }
    metrics.update(rouge_scores)
    metrics.update(lfqa_scores)
    return metrics


def compute_rouge_metrics(reference: Optional[str], summary: Optional[str]) -> Dict[str, float]:
    reference_tokens = _tokenize_for_metrics(reference)
    summary_tokens = _tokenize_for_metrics(summary)
    if not reference_tokens or not summary_tokens:
        return {
            "rouge1_precision": 0.0,
            "rouge1_recall": 0.0,
            "rouge1_f": 0.0,
            "rougeL_precision": 0.0,
            "rougeL_recall": 0.0,
            "rougeL_f": 0.0,
        }

    rouge1 = _rouge_n(reference_tokens, summary_tokens, n=1)
    rouge_l = _rouge_l(reference_tokens, summary_tokens)

    return {
        "rouge1_precision": round(rouge1[0], 4),
        "rouge1_recall": round(rouge1[1], 4),
        "rouge1_f": round(rouge1[2], 4),
        "rougeL_precision": round(rouge_l[0], 4),
        "rougeL_recall": round(rouge_l[1], 4),
        "rougeL_f": round(rouge_l[2], 4),
    }


def estimate_lfqa_metrics(transcript: Optional[str], summary: Optional[str]) -> Dict[str, float | int]:
    questions = _extract_question_keywords(transcript)
    if not questions:
        return {
            "lfqa_question_count": 0,
            "lfqa_coverage": 1.0,
        }

    summary_tokens = set(_tokenize_for_metrics(summary))
    covered = 0
    for keywords in questions:
        if not keywords:
            covered += 1
            continue
        if any(token in summary_tokens for token in keywords):
            covered += 1

    coverage = covered / len(questions) if questions else 0.0
    return {
        "lfqa_question_count": len(questions),
        "lfqa_coverage": round(coverage, 4),
    }


def _extract_question_keywords(text: Optional[str]) -> List[List[str]]:
    if not text:
        return []
    raw_questions = re.findall(r"[^?\n]+\?", text)
    questions: List[List[str]] = []
    for question in raw_questions:
        tokens = [token for token in _tokenize_for_metrics(question) if token not in QUESTION_STOP_WORDS]
        questions.append(tokens)
    return questions


def _tokenize_for_metrics(text: Optional[str]) -> List[str]:
    if not text:
        return []
    return re.findall(r"[\w']+", text.lower())


def _rouge_n(
    reference: Sequence[str],
    summary: Sequence[str],
    *,
    n: int,
) -> Tuple[float, float, float]:
    if n <= 0:
        return (0.0, 0.0, 0.0)

    def ngrams(tokens: Sequence[str]) -> Counter:
        return Counter(tuple(tokens[i : i + n]) for i in range(len(tokens) - n + 1))

    ref_counts = ngrams(reference)
    sum_counts = ngrams(summary)
    if not ref_counts or not sum_counts:
        return (0.0, 0.0, 0.0)

    overlap = sum((ref_counts & sum_counts).values())
    precision = overlap / max(sum(sum_counts.values()), 1)
    recall = overlap / max(sum(ref_counts.values()), 1)
    f_score = _safe_f1(precision, recall)
    return (precision, recall, f_score)


def _rouge_l(reference: Sequence[str], summary: Sequence[str]) -> Tuple[float, float, float]:
    lcs = _lcs_length(reference, summary)
    if lcs == 0:
        return (0.0, 0.0, 0.0)
    precision = lcs / len(summary) if summary else 0.0
    recall = lcs / len(reference) if reference else 0.0
    f_score = _safe_f1(precision, recall)
    return (precision, recall, f_score)


def _lcs_length(reference: Sequence[str], summary: Sequence[str]) -> int:
    if not reference or not summary:
        return 0
    prev_row = [0] * (len(summary) + 1)
    for ref_token in reference:
        current = [0]
        for idx, sum_token in enumerate(summary, start=1):
            if ref_token == sum_token:
                current.append(prev_row[idx - 1] + 1)
            else:
                current.append(max(prev_row[idx], current[-1]))
        prev_row = current
    return prev_row[-1]


def _safe_f1(precision: float, recall: float) -> float:
    if precision + recall == 0:
        return 0.0
    return 2 * precision * recall / (precision + recall)


__all__ = [
    "compute_quality_metrics",
    "compute_rouge_metrics",
    "estimate_lfqa_metrics",
]
