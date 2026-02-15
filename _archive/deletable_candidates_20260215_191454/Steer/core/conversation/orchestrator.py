"""High-level conversational orchestrator that routes queries to domain agents."""
from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Dict, Iterable, List, Optional

from core.agents import AgentRequest, AgentResult, ConversationalAgent
from core.conversation.llm_client import LLMClient
from core.utils import get_logger

LOGGER = get_logger("conversation.orchestrator")

MEETING_KEYWORDS = {"meeting", "회의", "녹음", "회의록", "transcribe", "transcription"}
PHOTO_KEYWORDS = {"사진", "photo", "이미지", "앨범", "gallery"}

COMMAND_PREFIXES = {
    "/search": "document_search",
    "/doc": "document_search",
    "/meeting": "meeting_summary",
    "/photo": "photo_manager",
    # Korean UI aliases
    "@검색": "document_search", 
    "@회의": "meeting_summary",
    "@사진": "photo_manager",
    "@문서": "document_search",
}


@dataclass
class OrchestratorResponse:
    message: str
    agent: str
    metadata: Dict[str, object] = field(default_factory=dict)
    suggestions: Optional[List[str]] = None
    reason: Optional[str] = None


class AssistantOrchestrator:
    """Routes conversation turns to specialised agents using deterministic heuristics."""

    def __init__(
        self,
        agents: Iterable[ConversationalAgent],
        *,
        llm_client: Optional[LLMClient] = None, # Kept for signature compatibility but unused for routing
        system_prompt: str = "", # Unused
    ) -> None:
        self._agents: Dict[str, ConversationalAgent] = {agent.name: agent for agent in agents}
        if "document_search" not in self._agents:
            raise ValueError("orchestrator requires a document_search agent as fallback")
        self._history: List[Dict[str, str]] = []
        self._last_reason: Optional[str] = None
        self._last_agent: Optional[str] = None
        self._initialise_agents()

    def _initialise_agents(self) -> None:
        for agent in self._agents.values():
            try:
                agent.prepare()
            except Exception as exc:  # pragma: no cover - defensive
                LOGGER.warning("agent %s failed to prepare: %s", agent.name, exc)

    def attach_llm(self, client: Optional[LLMClient]) -> None:
        # No-op: Orchestrator no longer uses LLM for routing
        pass

    def handle(self, query: str, extra_context: Optional[Dict[str, object]] = None) -> OrchestratorResponse:
        self._history.append({"role": "user", "content": query})

        command_agent, normalized_query, command_context = self._detect_command(query)
        base_context: Dict[str, object] = dict(extra_context or {})
        if command_context:
            base_context.update(command_context)

        # 1. Command-based Routing (Highest Priority)
        if command_agent:
            agent_name = command_agent
            context = base_context
            self._last_reason = "command"
            
            # Check for missing context (e.g. need audio path)
            missing_reason = self._missing_context_reason(agent_name, context)
            if missing_reason:
                return self._create_follow_up(missing_reason)
                
            self._last_agent = agent_name
            
        else:
            # 2. Heuristic Routing (Keyword/Guardrail)
            agent_name, context, reason = self._heuristic_route(query, base_context)
            self._last_reason = reason
            self._last_agent = agent_name

        agent = self._agents.get(agent_name, self._agents["document_search"])
        context_keys = [key for key in context.keys() if isinstance(key, str)]
        
        LOGGER.info(
            "orchestrator executing agent=%s reason=%s context_keys=%s",
            agent.name,
            self._last_reason,
            context_keys,
        )

        try:
            result = agent.run(AgentRequest(query=normalized_query, context=context))
            response_text = result.content.strip() or "결과가 없습니다."
            self._history.append({"role": "assistant", "content": response_text})
            
            return OrchestratorResponse(
                message=response_text,
                agent=agent.name,
                metadata=result.metadata,
                suggestions=result.suggestions,
                reason=self._last_reason,
            )
        except ValueError as exc:
            message = str(exc)
            self._history.append({"role": "assistant", "content": message})
            LOGGER.info("agent %s validation error: %s", agent.name, message)
            return OrchestratorResponse(message=message, agent=agent.name, reason=self._last_reason)
        except Exception as exc:
            LOGGER.exception("agent %s execution failed", agent.name)
            fallback = "요청을 처리하는 중 오류가 발생했습니다."
            self._history.append({"role": "assistant", "content": fallback})
            return OrchestratorResponse(
                message=fallback,
                agent=agent.name,
                metadata={"error": str(exc)},
                reason=self._last_reason,
            )

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------
    def _create_follow_up(self, reason: str) -> OrchestratorResponse:
        self._last_reason = reason
        self._last_agent = "follow_up"
        message = self._default_follow_up_message(reason)
        self._history.append({"role": "assistant", "content": message})
        LOGGER.info("orchestrator follow-up: reason=%s", reason)
        return OrchestratorResponse(message=message, agent="follow_up", reason=reason)

    @staticmethod
    def _default_follow_up_message(reason: Optional[str]) -> str:
        if reason == "needs_audio":
            return "회의 오디오 파일 경로를 알려주세요."
        if reason == "needs_roots":
            return "사진이 들어 있는 폴더 경로를 알려주세요."
        return "추가 정보가 필요합니다."

    def _detect_command(self, query: str) -> tuple[Optional[str], str, Dict[str, object]]:
        stripped = query.lstrip()
        lower = stripped.lower()
        for prefix, agent in COMMAND_PREFIXES.items():
            if lower.startswith(prefix):
                remainder = stripped[len(prefix) :].strip()
                remainder = remainder.replace("[첨부]", "").strip()
                
                context: Dict[str, object] = {}
                # Specific contexts
                if agent == "photo_manager" and remainder:
                    roots = [part.strip() for part in remainder.split(",") if part.strip()]
                    if roots: context["roots"] = roots
                if agent == "meeting_summary" and remainder:
                    context["audio_path"] = remainder
                if agent == "document_search":
                    # If explicitly commanded, force search intent
                    context["force_action"] = "search"
                    
                return agent, remainder or query, context
        return None, query, {}

    def _heuristic_route(self, query: str, extra_context: Dict[str, object]) -> tuple[str, Dict[str, object], Optional[str]]:
        # Default fallback
        agent = "document_search"
        reason = "default"
        context = dict(extra_context)

        # 1. Meeting Keywords (Strict)
        if self._text_contains_keywords(query, MEETING_KEYWORDS):
             # Check if context already has audio
             if context.get("audio_path"):
                 return "meeting_summary", context, "keyword_meeting"
             # If just keyword, maybe just talking about meeting?
             # For now, default to document search unless user clearly commands or implies Action
             # But if user says "Summarize this meeting", we might want meeting agent.
             # However, without audio_path, it triggers follow-up. That is OK.
             # Let's be aggressive on "meeting summary" intent if keywords strongly match action.
             if any(act in query for act in ["요약", "전사", "기록", "summary"]):
                 return "meeting_summary", context, "keyword_meeting_action"

        # 2. Photo Keywords
        if self._text_contains_keywords(query, PHOTO_KEYWORDS):
             if any(act in query for act in ["정리", "분류", "organize", "sort"]):
                 return "photo_manager", context, "keyword_photo_action"

        return agent, context, reason

    def _missing_context_reason(self, agent: str, context: Dict[str, object]) -> Optional[str]:
        if agent == "meeting_summary":
            if not context.get("audio_path"):
                return "needs_audio"
        if agent == "photo_manager":
             # Photo agent might rely on configured policy roots, so missing roots in context is not fatal 
             # unless policy is missing too. But Orchestrator doesn't check policy.
             # We assume if explicit command, we might need roots if not provided?
             # Let's match original logic:
             pass 
        return None

    @staticmethod
    def _text_contains_keywords(text: str, keywords: Iterable[str]) -> bool:
        lowered = text.lower()
        return any(keyword in lowered for keyword in keywords)
