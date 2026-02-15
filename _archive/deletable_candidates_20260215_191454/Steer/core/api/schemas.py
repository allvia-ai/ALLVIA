"""Pydantic models shared by the test-facing FastAPI routes."""
from __future__ import annotations

from typing import List, Literal, Optional

from pydantic import BaseModel, Field


class SearchRequest(BaseModel):
    query: str
    top_k: int = 5
    session_id: Optional[str] = None


class SessionSummary(BaseModel):
    recent_queries: List[str]
    preferred_exts: List[str]
    owner_prior: List[str]


class ChatMessage(BaseModel):
    role: Literal["user", "assistant"]
    text: str


class SearchResponse(BaseModel):
    session_id: Optional[str]
    results: List[dict]
    explain: List[List[str]]
    session: SessionSummary
    answer: Optional[str] = None
    answer_source: Literal["llm", "fallback", "none"] = "none"
    history: List[ChatMessage] = Field(default_factory=list)
    llm_error: Optional[str] = None


class FeedbackRequest(BaseModel):
    session_id: Optional[str]
    doc_id: Optional[int] = None
    path: Optional[str] = None
    ext: Optional[str] = None
    owner: Optional[str] = None
    action: Literal["click", "pin", "like", "dislike"]


class FeedbackResponse(BaseModel):
    session_id: Optional[str]
    status: str
    session: Optional[SessionSummary] = None


class ReindexRequest(BaseModel):
    force: bool = False


class ReindexResponse(BaseModel):
    status: str


class SessionResetResponse(BaseModel):
    session_id: Optional[str]
    recent_queries: List[str]
    history: List[ChatMessage] = Field(default_factory=list)
