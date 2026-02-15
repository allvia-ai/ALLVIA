"""Shared constants for the meeting agent."""
from __future__ import annotations

import re

LANGUAGE_ALIASES = {
    "ko": "ko",
    "kor": "ko",
    "korean": "ko",
    "ko-kr": "ko",
    "kr": "ko",
    "en": "en",
    "eng": "en",
    "english": "en",
    "en-us": "en",
    "en-gb": "en",
    "ja": "ja",
    "jpn": "ja",
    "japanese": "ja",
    "zh": "zh",
    "zh-cn": "zh",
    "zh-tw": "zh",
    "cmn": "zh",
    "chi": "zh",
    "mandarin": "zh",
}

DEFAULT_LANGUAGE = "ko"

ACTION_KEYWORDS = {
    "default": ["action", "todo", "follow", "follow-up"],
    "ko": ["action", "todo", "follow", "해야", "요청", "담당", "액션", "아이템", "후속"],
    "en": ["action", "todo", "follow", "follow-up", "owner", "next step"],
    "ja": ["対応", "タスク", "宿題", "確認", "引き続き"],
    "zh": ["行动", "待办", "跟进", "负责人", "任务"],
}

DECISION_KEYWORDS = {
    "default": ["decision", "decide", "approved", "agreed"],
    "ko": ["결정", "승인", "확정", "정리", "합의"],
    "en": ["decision", "approved", "agreed", "final"],
    "ja": ["決定", "合意", "承認", "確定"],
    "zh": ["决定", "批准", "确认", "定案"],
}

HIGHLIGHT_KEYWORDS = {
    "default": ["key", "highlight", "important", "summary", "note"],
    "ko": ["핵심", "하이라이트", "중요", "요약", "결론", "중점"],
    "en": ["key", "highlight", "important", "summary", "notable"],
    "ja": ["重要", "ハイライト", "要点", "まとめ"],
    "zh": ["重点", "亮点", "总结", "重要"],
}

HIGHLIGHT_FALLBACK = {
    "ko": "회의 주요 내용을 식별하지 못했습니다.",
    "en": "No key highlights detected.",
    "ja": "主要なハイライトを検出できませんでした。",
    "zh": "未检测到关键要点。",
}

GENERIC_FALLBACK = {
    "ko": "관련 항목이 발견되지 않았습니다.",
    "en": "No related items found.",
    "ja": "該当する項目が見つかりませんでした。",
    "zh": "未找到相关条目。",
}

PII_EMAIL_RE = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
PII_PHONE_RE = re.compile(r"\+?\d[\d\s\-]{7,}\d")
PII_RRN_RE = re.compile(r"\b\d{6}-?[1-8]\d{6}\b")

# NOTE: 주소(주소 문자열)는 형태가 매우 다양하므로, 과도한 오탐을 피하기 위해
# 한국어 주소에서 자주 등장하는 "시/도/구/군/읍/면/동 + 로/길 + 번지" 패턴을 보수적으로 탐지한다.
PII_ADDRESS_RE = re.compile(
    r"(?:(?:[가-힣]{2,}(?:시|도))\s*)?"
    r"(?:[가-힣0-9]{1,}(?:시|구|군))\s+"
    r"(?:[가-힣0-9]{1,}(?:구|군|읍|면|동))\s+"
    r"(?:[가-힣0-9]+(?:로|길|번길))\s+"
    r"\d+(?:-\d+)?"
)

AVERAGE_SPEECH_WPM = 130

QUESTION_STOP_WORDS = {
    "the",
    "a",
    "an",
    "is",
    "are",
    "was",
    "were",
    "what",
    "who",
    "when",
    "where",
    "why",
    "how",
    "do",
    "does",
    "did",
    "will",
    "can",
    "should",
    "could",
    "would",
    "please",
    "누가",
    "무엇",
    "언제",
    "어디",
    "왜",
    "어떻게",
    "무슨",
    "어느",
    "가능",
}
