from __future__ import annotations

from collections import deque
from dataclasses import dataclass, field
from typing import Callable, Deque, Iterable, List, Optional, Sequence, Set, Tuple


@dataclass
class ChatTurn:
    role: str
    text: str
    hits: List[dict] = field(default_factory=list)


class MemoryStore:
    def __init__(self, capacity: int = 10) -> None:
        self.capacity = max(1, capacity)
        self._turns: Deque[ChatTurn] = deque()

    def add(self, turn: ChatTurn) -> None:
        self._turns.append(turn)
        while len(self._turns) > self.capacity:
            self._turns.popleft()

    def add_turn(self, role: str, text: str, *, hits: Optional[List[dict]] = None) -> ChatTurn:
        turn = ChatTurn(role=role, text=text, hits=hits or [])
        self.add(turn)
        return turn

    def recent(self, limit: Optional[int] = None) -> List[ChatTurn]:
        if limit is None or limit >= len(self._turns):
            return list(self._turns)
        if limit <= 0:
            return []
        return list(self._turns)[-limit:]

    def last_user_text(self) -> Optional[str]:
        for turn in reversed(self._turns):
            if turn.role == "user":
                return turn.text
        return None

    def last_assistant_text(self) -> Optional[str]:
        for turn in reversed(self._turns):
            if turn.role == "assistant":
                return turn.text
        return None

    def build_prompt_history(self, limit: Optional[int] = None) -> str:
        turns = self.recent(limit)
        buf = []
        for turn in turns:
            role_label = "User" if turn.role == "user" else "Assistant"
            buf.append(f"{role_label}: {turn.text}")
        return "\n".join(buf)


class PromptManager:
    def __init__(
        self,
        memory: MemoryStore,
        tokenizer: Callable[[str], Iterable[str]],
        *,
        pronoun_markers: Optional[Set[str]] = None,
        follow_markers: Optional[Set[str]] = None,
        type_markers: Optional[Set[str]] = None,
    ) -> None:
        self.memory = memory
        self.tokenizer = tokenizer
        self.pronoun_markers = pronoun_markers or {
            "그",
            "그거",
            "그것",
            "그문서",
            "그파일",
            "해당",
            "이전",
            "앞",
            "방금",
            "previous",
            "earlier",
            "above",
            "that",
            "those",
        }
        self.follow_markers = follow_markers or {
            "추가",
            "또",
            "다른",
            "같은",
            "비슷",
            "관련",
            "계속",
            "이어",
            "더",
        }
        self.type_markers = type_markers or {
            "pdf",
            "ppt",
            "pptx",
            "doc",
            "docx",
            "hwp",
            "xls",
            "xlsx",
            "csv",
            "파일",
            "문서",
            "자료",
            "형식",
            "버전",
        }

    def rewrite_query(
        self,
        query: str,
        tokens: Set[str],
        *,
        last_query: Optional[str],
        context_terms: Sequence[str],
    ) -> Tuple[str, bool]:
        if not last_query:
            return query, False

        lowered_tokens = {tok.lower() for tok in tokens if tok}
        if not lowered_tokens:
            lowered_tokens = {tok.lower() for tok in self.tokenizer(query)}
        last_tokens = {tok.lower() for tok in self.tokenizer(last_query)}

        follow_token = bool(lowered_tokens & (self.pronoun_markers | self.follow_markers))
        disjoint_from_last = last_tokens.isdisjoint(lowered_tokens)
        type_followup = (
            disjoint_from_last and len(lowered_tokens) <= 4 and bool(lowered_tokens & self.type_markers)
        )
        if follow_token and disjoint_from_last and not type_followup and len(lowered_tokens) > 2:
            follow_token = False

        if not (follow_token or type_followup):
            return query, False

        pieces: List[str] = [query]
        if context_terms:
            pieces.append(" ".join(context_terms))
        pieces.append(last_query)
        rewritten = " ".join(piece for piece in pieces if piece).strip()
        return (rewritten or query), True


class ToolRouter:
    def select_action(
        self,
        query: str,
        *,
        use_translation: bool,
        policy_active: bool,
        llm_available: bool,
    ) -> str:
        trimmed = query.strip()
        if not trimmed:
            return "dialogue" if llm_available else "search"

        lowered = trimmed.lower()
        has_command = lowered.startswith(("/search", "/doc", "/audio"))
        keywords_summary = {
            "요약", "정리", "정리해줘", "설명해줘", "핵심", "한줄", "결론",
            "summarize", "summary", "explain", "tl;dr", "overview",
        }
        keywords_list = {"목록", "리스트", "파일", "문서", "보여줘", "검색"}
        doc_terms = {
            "문서", "자료", "파일", "보고서", "리포트", "레포트", "policy", "document", "documents",
            "보고", "자료집", "dataset", "데이터셋", "pdf", "ppt", "pptx", "정책", "규정",
        }
        search_triggers = {
            "찾아", "찾아줘", "검색", "search", "scan", "살펴", "추려", "추천", "list", "show", "요약",
            "정리", "요약해", "정리해", "분석", "요약본",
        }

        def _contains_any(source: str, vocab: Set[str]) -> bool:
            return any(token in source for token in vocab)

        ask_summary = _contains_any(trimmed, keywords_summary) or _contains_any(lowered, keywords_summary)
        ask_list = _contains_any(trimmed, keywords_list) or _contains_any(lowered, keywords_list)
        doc_terms_hit = _contains_any(trimmed, doc_terms) or _contains_any(lowered, doc_terms)
        search_terms_hit = _contains_any(trimmed, search_triggers) or _contains_any(lowered, search_triggers)

        # has_command가 아니더라도 검색 의도가 있으면 search로 라우팅한다.
        # 단, 의도가 불분명하면 대화(dialogue)로 보낸다.
        if ask_summary or ask_list or search_terms_hit or doc_terms_hit:
            return "search_and_summarize" if ask_summary else "search"
            
        return "dialogue" if llm_available else "search"

    def build_gemma_prompt(self, instruction: str, history_text: str, query: str) -> str:
        return (
            f"<start_of_turn>user\n"
            f"{instruction}\n\n"
            f"Context:\n{history_text}\n"
            f"Current Query:\n{query}<end_of_turn>\n"
            f"<start_of_turn>model\n"
        )
