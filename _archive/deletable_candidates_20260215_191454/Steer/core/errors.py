"""Unified error hierarchy (Park David spec alignment)."""
from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Dict


class AIError(Exception):
    """Base error for AI-summary."""

    error_type = "AIError"
    severity = "medium"

    def __init__(
        self,
        message: str = "",
        *,
        hint: str | None = None,
        context: Dict[str, Any] | None = None,
        timestamp: str | None = None,
    ):
        super().__init__(message)
        self.hint = hint
        self.context = context or {}
        self.timestamp = timestamp or datetime.now(timezone.utc).isoformat()

    def to_dict(self) -> Dict[str, Any]:
        return {
            "error_type": self.error_type,
            "message": str(self),
            "hint": self.hint or "",
            "context": self.context,
            "timestamp": self.timestamp,
            "severity": self.severity,
        }


class PolicyError(AIError):
    error_type = "PolicyError"


class AccessDeniedError(PolicyError):
    error_type = "AccessDeniedError"
    severity = "high"


class PolicyViolationError(PolicyError):
    error_type = "PolicyViolationError"
    severity = "high"


class PipelineError(AIError):
    error_type = "PipelineError"


class ScanError(PipelineError):
    error_type = "ScanError"


class ExtractionError(PipelineError):
    error_type = "ExtractionError"


class EmbeddingError(PipelineError):
    error_type = "EmbeddingError"


class TrainingError(PipelineError):
    error_type = "TrainingError"


class DriftError(PipelineError):
    error_type = "DriftError"


class AgentError(AIError):
    error_type = "AgentError"


class STTError(AgentError):
    error_type = "STTError"


class SummarizationError(AgentError):
    error_type = "SummarizationError"


class ActionExtractionError(AgentError):
    error_type = "ActionExtractionError"


class ConversationError(AIError):
    error_type = "ConversationError"


class PromptError(ConversationError):
    error_type = "PromptError"


class CitationError(ConversationError):
    error_type = "CitationError"


class SafetyError(ConversationError):
    error_type = "SafetyError"


class ModelError(AIError):
    error_type = "ModelError"


class ModelLoadError(ModelError):
    error_type = "ModelLoadError"
    severity = "critical"


class ModelInferenceError(ModelError):
    error_type = "ModelInferenceError"


class SystemError(AIError):
    error_type = "SystemError"


class FileIOError(SystemError):
    error_type = "FileIOError"


class CacheCorruptionError(SystemError):
    error_type = "CacheCorruptionError"
    severity = "high"


class UnexpectedRuntimeError(SystemError):
    error_type = "UnexpectedRuntimeError"
    severity = "critical"
