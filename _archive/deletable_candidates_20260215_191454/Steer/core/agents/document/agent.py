"""Document assistant backed by LNPChat."""
from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from types import SimpleNamespace
from typing import Dict, Optional

from core.agents import AgentRequest, AgentResult, ConversationalAgent
from core.agents.supervisor import SummarySupervisor, SupervisorDecision
from core.conversation.lnp_chat import LNPChat
from core.policy.engine import PolicyEngine
from core.config.llm_defaults import DEFAULT_LLM_MODEL, resolve_backend


@dataclass
class DocumentAgentConfig:
    """Configuration for document assistant."""

    model_path: Path
    corpus_path: Path
    cache_dir: Path
    topk: int = 5
    min_similarity: float = 0.75
    translate: bool = False
    rerank: bool = True
    rerank_model: str = "BAAI/bge-reranker-large"
    rerank_depth: int = 80
    rerank_batch_size: int = 16
    rerank_device: Optional[str] = None
    rerank_min_score: Optional[float] = 0.35
    lexical_weight: float = 0.2
    show_translation: bool = False
    translation_lang: str = "en"
    auto_search: bool = False
    strict_search: bool = False
    llm_backend: Optional[str] = None
    llm_model: Optional[str] = None
    llm_host: Optional[str] = None
    llm_options: Optional[Dict[str, str]] = None
    llm_health_timeout: Optional[float] = None
    llm_timeout: Optional[float] = None
    policy_engine: Optional[PolicyEngine] = None
    policy_scope: str = "auto"
    policy_agent: str = "knowledge_search"
    rebuild_index: bool = False


class DocumentAgent(ConversationalAgent):
    """Wraps LNPChat so it can be orchestrated alongside 다른 에이전트."""

    name = "document_search"
    description = "질문에 맞는 문서를 검색하고 필요한 경우 요약합니다."

    def __init__(self, config: DocumentAgentConfig) -> None:
        self.config = config
        llm_options = dict(config.llm_options or {})
        llm_options.setdefault("num_predict", int(os.getenv("DOCUMENT_LLM_NUM_PREDICT", "512")))
        llm_options.setdefault("temperature", float(os.getenv("DOCUMENT_LLM_TEMPERATURE", "0.6")))
        llm_options.setdefault("thinking", False)

        env_backend = os.getenv("DOCUMENT_LLM_BACKEND", "")
        effective_model = config.llm_model or os.getenv("DOCUMENT_LLM_MODEL", DEFAULT_LLM_MODEL)
        effective_backend = resolve_backend(
            config_backend=config.llm_backend,
            env_backend=env_backend,
            model=effective_model,
        )

        self._chat = LNPChat(
            model_path=config.model_path,
            corpus_path=config.corpus_path,
            cache_dir=config.cache_dir,
            topk=config.topk,
            min_similarity=config.min_similarity,
            translate=config.translate,
            rerank=config.rerank,
            rerank_model=config.rerank_model,
            rerank_depth=config.rerank_depth,
            rerank_batch_size=config.rerank_batch_size,
            rerank_device=config.rerank_device,
            rerank_min_score=config.rerank_min_score,
            lexical_weight=config.lexical_weight,
            show_translation=config.show_translation,
            translation_lang=config.translation_lang,
            auto_search=config.auto_search,
            strict_search=config.strict_search,
            llm_backend=effective_backend,
            llm_model=effective_model,
            llm_host=config.llm_host,
            llm_options=llm_options,
            llm_health_timeout=config.llm_health_timeout if config.llm_health_timeout is not None else None,
            llm_timeout=config.llm_timeout if config.llm_timeout is not None else None,
            policy_engine=config.policy_engine,
            policy_scope=config.policy_scope,
            policy_agent=config.policy_agent,
        )
        self._supervisor = SummarySupervisor.from_env("DOCUMENT")
        supervisor_mode_env = (os.getenv("DOCUMENT_SUPERVISOR_MODE") or os.getenv("SUMMARY_SUPERVISOR_MODE") or "manual").strip().lower()
        self._supervisor_mode = supervisor_mode_env if supervisor_mode_env in {"auto", "always", "manual", "off"} else "manual"

    @property
    def chat(self) -> LNPChat:
        return self._chat

    @property
    def llm_client(self):
        return getattr(self._chat, "llm_client", None)

    def prepare(self) -> None:
        wait_timeout = None if self.config.rebuild_index else 0.1
        self._chat.ready(rebuild=self.config.rebuild_index, wait_timeout=wait_timeout)

    def rebuild_index(self) -> None:
        retr = getattr(self._chat, "retr", None)
        if retr is None:
            self._chat.ready(rebuild=True)
            return
        future = retr.index_manager.schedule_rebuild(priority=True)
        future.result()
        retr.wait_until_ready(timeout=None)

    def run(self, request: AgentRequest) -> AgentResult:
        context = request.context or {}
        force_action: Optional[str] = None
        if context:
            forced = context.get("force_action")
            if isinstance(forced, str):
                forced_normalized = forced.strip().lower()
                if forced_normalized in {"dialogue", "search", "search_and_summarize"}:
                    force_action = forced_normalized
            if force_action is None and context.get("force_search"):
                force_action = "search"

        try:
            result = self._chat.ask(request.query, force_action=force_action)
        except TypeError:
            # 백워드 호환: force_action을 지원하지 않는 stub/구버전 챗 구현
            result = self._chat.ask(request.query)
        answer: str = result.get("answer") or ""
        suggestions = result.get("suggestions") or None
        metadata = {
            "hits": result.get("hits", []),
            "agent": self.name,
            "llm_summary": result.get("llm_summary"),
        }
        issues: list[str] = []
        alerts: list[str] = []
        answer_clean = answer.strip()
        if not answer_clean:
            issues.append("answer_missing")
            alerts.append("answer_missing")
        if not metadata["hits"]:
            issues.append("no_hits")
        if len(answer_clean) < 80:
            issues.append("answer_short")

        metrics = {
            "answer_chars": len(answer_clean),
            "hit_count": len(metadata["hits"]),
            "suggestion_count": len(suggestions or []),
        }
        metadata["metrics"] = metrics
        if issues:
            metadata["issues"] = issues

        supervisor_info: Optional[Dict[str, str]] = None
        if self._supervisor.is_enabled() and self._supervisor_mode not in {"off"}:
            decision = self._supervisor.decide(
                agent="document",
                summary=SimpleNamespace(
                    raw_summary=answer,
                    highlights=[],
                    action_items=suggestions or [],
                    decisions=[],
                    structured_summary={"source_hits": metadata["hits"], "issues": issues},
                ),
                metrics=metrics,
                issues=issues,
                alerts=alerts,
            )
            supervisor_info = decision.as_dict()
            metadata["supervisor"] = supervisor_info
            if decision.action in {"escalate", "review"}:
                metadata["requires_manual_review"] = True
                if decision.reason:
                    metadata["supervisor_reason"] = decision.reason
                if decision.notes:
                    metadata["supervisor_notes"] = decision.notes
            if decision.focus_keywords:
                metadata["supervisor_focus"] = decision.focus_keywords

        return AgentResult(content=answer, suggestions=suggestions, metadata=metadata)
