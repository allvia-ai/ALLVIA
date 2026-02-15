# core/utils/stopwords.py
from __future__ import annotations
from typing import Set

_EN_STOPWORDS: Set[str] = {
    "the", "and", "for", "that", "with", "from", "this", "have", "been", "were",
    "into", "about", "after", "before", "while", "shall", "could", "would", "there",
    "their", "they", "them", "these", "those", "is", "are", "be", "am", "was",
    "it", "its", "as", "to", "or", "an", "a", "so", "if", "not", "no", "do",
    "does", "did", "each", "per", "via", "both", "same", "own", "due",
    "can", "will", "may", "might", "must", "should", "of", "in", "on", "at", "by",
}

_KO_STOPWORDS: Set[str] = {
    "그리고", "그러나", "하지만", "그러면서", "그러므로", "또한", "그러니까", "따라서", "그리고나서",
    "이", "그", "저", "것", "수", "등", "들", "및", "안", "못", "왜", "어떻게",
    "무엇", "어떤", "누구", "언제", "어디", "가", "이", "을", "를", "의", "는", "은", "과", "와",
    "도", "만", "게", "서", "에게", "께", "한테", "보다", "처럼", "만큼", "같이", "부터", "까지",
    "이나", "나", "다", "로", "으로", "해", "해서", "한다", "했다", "이다", "있다", "없다",
    "되다", "보다", "하다", "같다", "생각하다", "만들다", "시키다", "받다", "주다", "오다", "가다",
    "알다", "모르다", "없다", "계시다",
    # Domain specific (safe to include here for general retrieval noise reduction)
    "파일", "문서", "보고서", "첨부", "자료들", "내용", "프로젝트", "관련자료",
}

STOPWORDS: Set[str] = {
    word.lower() for word in (*_EN_STOPWORDS, *_KO_STOPWORDS)
}
