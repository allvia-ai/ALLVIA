# -*- coding: utf-8 -*-
"""
LNPChat: 자연어 대화로 문서 검색/추천
- Retriever(모델/코퍼스/인덱스)를 사용해 사용자 질의 → 유사 문서 Top-K
- 간단한 대화 히스토리, 진행 스피너, 후속질문 제안 포함
"""
from __future__ import annotations

import json
import os
import re
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Set

from core.utils import get_logger

from core.conversation.prompting import ChatTurn, MemoryStore, PromptManager, ToolRouter
from core.conversation.translation_cache import TranslationCache
from core.policy.engine import PolicyEngine

# New Modules
from core.conversation.chat_ui import ChatUI
from core.conversation.retrieval_strategy import (
    init_retriever, 
    init_llm_client, 
    ensure_offline_transformers
)
from core.conversation.llm_client import LLMClient

try:
    from deep_translator import GoogleTranslator
except ImportError:
    GoogleTranslator = None

LOGGER = get_logger("lnp.chat")

# ──────────────────────────
# Constants
# ──────────────────────────
CONFIRM_POSITIVES = {"응", "네", "예", "맞아", "좋아", "yes", "y", "sure", "ok", "그래", "ㅇㅋ", "넵"}
CONFIRM_NEGATIVES = {"아니", "아니오", "no", "n", "싫어", "괜찮아", "아냐", "안돼", "안해", "노"}

CHIT_CHAT_KEYWORDS = {
    "안녕", "안녕하세요", "반가워", "반갑습니다", "하이", "하이루", "ㅎㅇ", 
    "고마워", "감사", "감사합니다", "수고", "수고했어", "잘했어", "누구니", 
    "너는 누구니", "도움말", "사용법", "hello", "hi", "thanks", "thank you",
    "이름이", "뭐야", "뭐니"
}

SEARCH_INTENT_KEYWORDS = {"찾아", "검색", "보여", "요약", "어디", "문서", "파일", "정리", "알려줘", "구해", "무슨"}

# Slash command routing
SLASH_COMMANDS = {
    "/document": "search",
    "/doc": "search",
    "/문서": "search",
    "/meeting": "meeting",
    "/audio": "meeting",
    "/회의": "meeting",
    "/photo": "photo",
    "/사진": "photo",
}

PROMPT_INSTRUCTION_SMALL_TALK = (
    "당신은 'InfoPilot'이라는 AI 비서입니다. 한국어로 친근하고 자연스럽게 대화하세요.\n"
    "\n"
    "## 핵심 규칙\n"
    "1. 사용자가 '너(AI)의 이름'을 물으면: 'InfoPilot입니다.'\n"
    "2. 사용자가 '내(사용자) 이름'을 물으면: '죄송해요, 아직 이름을 알려주지 않으셨어요. 이름이 뭐예요?'\n"
    "3. 사용자가 자신의 이름을 알려주면: 그 이름을 기억하고 사용하세요.\n"
    "4. 문서 검색이나 파일 관련 질문은 '/document 질문' 형식으로 물어보라고 안내하세요.\n"
    "\n"
    "## 대화 스타일\n"
    "- 짧고 친절하게 답변하세요.\n"
    "- 반말/존댓말은 사용자에 맞춰주세요.\n"
    "- 모르는 건 솔직히 모른다고 하세요.\n"
)

def _split_tokens(text: str) -> List[str]:
    return text.split()

@dataclass
class LNPChat:
    """
    Local Neural Pilot Chat.
    Orchestrates search, translation, policy checks, and LLM summarization.
    """
    model_path: Path
    corpus_path: Path
    
    # Configuration
    cache_dir: Path = Path("data/cache")
    topk: int = 5
    min_similarity: float = 0.75
    translate: bool = False
    rerank: bool = True
    rerank_model: str = "BAAI/bge-reranker-large"
    rerank_depth: int = 40  # Reduced from 80 for speed
    rerank_batch_size: int = 16
    rerank_device: Optional[str] = None
    rerank_min_score: Optional[float] = 0.35
    lexical_weight: float = 0.4  # Increased for better keyword matching
    show_translation: bool = False
    translation_lang: str = "en"
    auto_search: bool = False
    strict_search: bool = False
    
    # LLM Config
    llm_backend: Optional[str] = None
    llm_model: Optional[str] = None
    llm_host: Optional[str] = None
    llm_options: Optional[Dict[str, str]] = None
    llm_health_timeout: Optional[float] = None
    llm_timeout: Optional[float] = None
    
    # Policy Config
    policy_engine: Optional[PolicyEngine] = None
    policy_scope: str = "auto"
    policy_agent: str = "knowledge_search"
    
    # State
    memory: MemoryStore = field(default_factory=lambda: MemoryStore(capacity=30))
    llm_client: Optional[LLMClient] = None
    _translator: Optional[GoogleTranslator] = None
    _translation_cache: Optional[TranslationCache] = None
    
    # Components
    ui: ChatUI = field(default_factory=ChatUI)
    retr: Optional[Any] = None  # HybridRetriever
    tool_router: ToolRouter = field(default_factory=ToolRouter)
    prompt_manager: Optional[PromptManager] = None
    
    # Internal State
    _pending_confirmation: Optional[Dict[str, Any]] = None

    def __post_init__(self):
        self.cache_dir.mkdir(parents=True, exist_ok=True)
        ensure_offline_transformers()
        self._translation_cache = TranslationCache(self.cache_dir / "translation_cache.mb")
        self.prompt_manager = PromptManager(self.memory, tokenizer=_split_tokens)
        
        # Initialize LLM Client
        self._reset_llm_client()

    def _reset_llm_client(self):
        """Re-initialize the LLM client using the Strategy module."""
        self.llm_client = init_llm_client(
            backend=self.llm_backend,
            model=self.llm_model,
            host=self.llm_host,
            options=self.llm_options,
            health_timeout=self.llm_health_timeout
        )

    def ready(self, rebuild: bool = False, *, wait_timeout: float | None = 0.1) -> bool:
        """Initialize retrieval backend."""
        if self.retr is None:
            self._build_retriever(rebuild=rebuild)
            
        if self.retr and wait_timeout is not None:
            return self.retr.wait_until_ready(timeout=wait_timeout)
        return False

    def _build_retriever(self, rebuild: bool = False) -> None:
        """Delegate to retrieval strategy."""
        
        self.retr = init_retriever(
            model_path=self.model_path,
            corpus_path=self.corpus_path,
            cache_dir=self.cache_dir,
            topk=self.topk,
            min_similarity=self.min_similarity,
            use_rerank=self.rerank,
            rerank_model_name=self.rerank_model,
            rerank_depth=self.rerank_depth,
            rerank_batch_size=self.rerank_batch_size,
            rerank_device=self.rerank_device,
            rerank_min_score=self.rerank_min_score,
            lexical_weight=self.lexical_weight,
            rebuild=rebuild
        )
        if self.retr is None:
             LOGGER.warning("Retriever initialization failed or deferred.")

    def _ensure_preview_translator(self):
        if not self.translate or not GoogleTranslator:
            return
        if self._translator is None:
            try:
                self._translator = GoogleTranslator(source="auto", target="ko")
            except Exception as e:
                LOGGER.warning("Translator init failed: %s", e)

    def ask(self, query: str, topk: Optional[int] = None, *, force_action: Optional[str] = None) -> Dict[str, Any]:
        """Main interaction point."""
        k = topk if topk is not None else self.topk
        
        # 1. Command Check
        cmd_res = self._handle_commands(query)
        if cmd_res:
            return cmd_res

        # 2. Pending Confirmation
        confirm_res = self._handle_pending_confirmation(query, k)
        if confirm_res:
             if "answer" in confirm_res:
                 return confirm_res
             # If it returned query/action, we continue with that
             query = confirm_res.get("query", query)
             force_action = confirm_res.get("action", force_action)

        # 3. Action Determination
        action, query_to_use = self._determine_action(query, force_action)
        
        # 4. Execute
        return self._execute_action(action, query_to_use, k)

    def _handle_commands(self, query: str) -> Optional[Dict[str, Any]]:
        query_s = query.strip()
        if query_s in ("!reset", "!clear"):
            self.memory.clear()
            return {"answer": "대화 기록을 초기화했습니다."}
        return None

    def _handle_pending_confirmation(self, query: str, k: int) -> Optional[Dict[str, Any]]:
        if not self._pending_confirmation:
            return None
        
        pending = self._pending_confirmation
        self._pending_confirmation = None # Clear it
        
        q_norm = query.strip().lower()
        if q_norm in CONFIRM_POSITIVES:
            # User confirmed
            return {"query": pending["query"], "action": pending["action"]}
        elif q_norm in CONFIRM_NEGATIVES:
            # User denied
            return {"answer": "알겠습니다. 검색하지 않겠습니다."}
        
        # Ambiguous response -> treat as new query? 
        # For now, let's assume if it's not yes/no, it's a new turn
        pass 
        return None

    def _parse_command(self, query: str) -> tuple[str | None, str]:
        """Parse slash command from query. Returns (action, remaining_query)."""
        query_s = query.strip()
        if not query_s.startswith("/"):
            return None, query
        
        parts = query_s.split(maxsplit=1)
        cmd = parts[0].lower()
        remaining = parts[1] if len(parts) > 1 else ""
        
        action = SLASH_COMMANDS.get(cmd)
        return action, remaining

    def _determine_action(self, query: str, force_action: Optional[str] = None) -> tuple[str, str]:
        """Determine action and return (action, query_to_use)."""
        if force_action:
            return force_action, query
        
        # 1. Check for slash commands first
        cmd_action, remaining_query = self._parse_command(query)
        if cmd_action:
            return cmd_action, remaining_query
        
        # 2. Auto-search mode (if enabled)
        if self.auto_search or self.strict_search:
            return "search", query

        # 3. Default to chat (no search)
        return "chat", query

    def _execute_action(self, action: str, query: str, k: int) -> Dict[str, Any]:
        if action == "search":
            return self._search_and_answer(query, k)
        elif action == "meeting":
            return self._handle_meeting(query)
        elif action == "photo":
            return self._handle_photo(query)
        elif action == "chat":
            return self._chat_only(query)
        return self._chat_only(query)  # Default to chat

    def _handle_meeting(self, query: str) -> Dict[str, Any]:
        """Handle meeting/audio transcription and summarization."""
        # Query should be a file path
        audio_path = query.strip().strip('"').strip("'")
        
        if not audio_path:
            return {"answer": "오디오 파일 경로를 입력해주세요.\n예: /meeting /path/to/audio.mp3", "hits": []}
        
        from pathlib import Path
        path = Path(audio_path)
        
        if not path.exists():
            return {"answer": f"파일을 찾을 수 없습니다: {audio_path}", "hits": []}
        
        try:
            from core.agents.meeting.agent import MeetingAgent, MeetingAgentConfig
            from core.agents import AgentRequest
            
            with self.ui.spinner("회의 전사 및 요약 중..."):
                config = MeetingAgentConfig()
                agent = MeetingAgent(config=config)
                agent.prepare()
                
                request = AgentRequest(
                    query=f"회의 요약",
                    context={"audio_path": str(path)}
                )
                result = agent.run(request)
                
            return {"answer": result.content, "hits": []}
        except ImportError as e:
            return {"answer": f"MeetingAgent를 로드할 수 없습니다: {e}", "hits": []}
        except Exception as e:
            LOGGER.error("Meeting agent failed: %s", e)
            return {"answer": f"회의 처리 중 오류가 발생했습니다: {e}", "hits": []}

    def _handle_photo(self, query: str) -> Dict[str, Any]:
        """Handle photo folder analysis."""
        folder_path = query.strip().strip('"').strip("'")
        
        if not folder_path:
            return {"answer": "사진 폴더 경로를 입력해주세요.\n예: /photo /path/to/photos", "hits": []}
        
        from pathlib import Path
        path = Path(folder_path)
        
        if not path.exists() or not path.is_dir():
            return {"answer": f"폴더를 찾을 수 없습니다: {folder_path}", "hits": []}
        
        try:
            from core.agents.photo.agent import PhotoAgent, PhotoAgentConfig
            from core.agents import AgentRequest
            
            with self.ui.spinner("사진 폴더 분석 중..."):
                config = PhotoAgentConfig()
                agent = PhotoAgent(config=config)
                agent.prepare()
                
                request = AgentRequest(
                    query=f"사진 정리",
                    context={"roots": [str(path)]}
                )
                result = agent.run(request)
            
            return {"answer": result.content, "hits": []}
        except ImportError as e:
            return {"answer": f"PhotoAgent를 로드할 수 없습니다: {e}", "hits": []}
        except Exception as e:
            LOGGER.error("Photo agent failed: %s", e)
            return {"answer": f"사진 처리 중 오류가 발생했습니다: {e}", "hits": []}

    def _search_and_answer(self, query: str, k: int) -> Dict[str, Any]:
        self.ready(wait_timeout=5.0)
        
        if not self.retr:
             return {"answer": "검색 엔진이 준비되지 않았습니다."}
             
        with self.ui.spinner("문서 검색 중..."):
             hits = self.retr.search(query, top_k=k)
        
        if not hits:
            return {"answer": "검색 결과가 없습니다.", "hits": []}
            
        # Summarize with LLM using the helper method (for test compatibility)
        summary = self._summarize_hits(query, hits)
        
        if summary:
            ans = summary
        else:
            context_str = "\n\n".join([h["content"] for h in hits])
            ans = "LLM을 사용할 수 없습니다. 검색 결과:\n" + context_str[:500]
            
        return {"answer": ans, "hits": hits, "suggestions": []}

    def _summarize_hits(self, query: str, hits: List[Dict[str, Any]]) -> Optional[str]:
        """Summarize search hits using the attached LLM."""
        if not self.llm_client:
            return None
            
        context_str = "\n\n".join([
            f"[{h.get('path', 'Unknown')}]\n{h.get('content') or h.get('preview') or ''}"
            for h in hits
        ])
        
        # Improved RAG prompt with structure and citation requirements
        full_prompt = (
            f"## 작업\n"
            f"아래 검색 결과를 핵심 근거로 삼아 사용자의 질문에 답변하세요.\n\n"
            f"## 규칙\n"
            f"1. 답변 내용의 모든 사실은 검색 결과에서 가져와야 합니다.\n"
            f"2. 검색 결과에 없는 내용은 '검색 결과에 관련 정보가 없습니다'라고 답하세요.\n"
            f"3. 답변 중간중간에 출처 파일명을 [파일명] 형태로 명시하세요.\n"
            f"4. 한국어로 자연스럽고 명확하게 답변하세요.\n\n"
            f"## 사용자 질문\n{query}\n\n"
            f"## 검색 결과\n{context_str}"
        )
        
        system_prompt = (
            "당신은 InfoPilot입니다. 사용자의 로컬 문서를 검색하여 정확한 정보를 제공하는 AI 비서입니다. "
            "검색 결과가 없거나 관련성이 낮으면 솔직하게 모른다고 대답합니다."
        )
        
        try:
            response = self.llm_client.generate(full_prompt, system=system_prompt)
            return response
        except Exception as e:
            LOGGER.warning("LLM summarization failed: %s", e)
            return None

    def _chat_only(self, query: str) -> Dict[str, Any]:
        """Multi-turn chat without retrieval for greetings/chit-chat."""
        if not self.llm_client:
             return {"answer": "LLM이 준비되지 않았습니다."}
        
        try:
            # Build messages array from memory for multi-turn
            messages = [{"role": "system", "content": PROMPT_INSTRUCTION_SMALL_TALK}]
            
            # Add conversation history (last N turns)
            for turn in self.memory.recent(limit=10):
                role = "user" if turn.role == "user" else "assistant"
                messages.append({"role": role, "content": turn.text})
            
            # Add current user query
            messages.append({"role": "user", "content": query})
            
            # Store user query in memory
            self.memory.add_turn("user", query)
            
            # Generate response
            with self.ui.spinner("생각 중..."):
                response = self.llm_client.generate_chat(messages)
            
            # Store assistant response in memory
            self.memory.add_turn("assistant", response)
            
            return {"answer": response, "hits": [], "suggestions": []}
        except Exception as e:
            LOGGER.warning("LLM chat failed: %s", e)
            return {"answer": "죄송합니다, 대화 처리 중 문제가 발생했습니다.", "hits": [], "suggestions": []}
