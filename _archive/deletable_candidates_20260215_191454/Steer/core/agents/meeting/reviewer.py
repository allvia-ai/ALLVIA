"""Post-processing reviewer for meeting summaries."""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import textwrap
from dataclasses import dataclass
from typing import Any, Dict, List, Optional

from core.utils import get_logger

from .models import MeetingJobConfig, MeetingSummary, MeetingTranscriptionResult

LOGGER = get_logger("meeting.reviewer")

DEFAULT_REVIEW_PROMPT = textwrap.dedent(
    """
    You are an experienced meeting notes editor. A first-pass summary has already been
    created. Review the material and improve it only when necessary.

    - Keep the tone concise and professional.
    - If something is unclear, make the best effort guess rather than inventing facts.
    - Always return strict JSON with the following keys:
        "summary": string (optional refined overall summary, omit or set to "" to keep existing)
        "highlights": array of short bullet strings
        "decisions": array of short bullet strings
        "action_items": array of short bullet strings
        "notes": string explaining adjustments (optional)

    Input payload:
    {payload}
    """
).strip()


def _int_env(name: str, default: int) -> int:
    raw = os.getenv(name)
    if raw is None:
        return default
    try:
        return int(raw)
    except ValueError:
        LOGGER.warning("Invalid integer for %s=%s; using %s", name, raw, default)
        return default


def _float_env(name: str, default: float) -> float:
    raw = os.getenv(name)
    if raw is None:
        return default
    try:
        return float(raw)
    except ValueError:
        LOGGER.warning("Invalid float for %s=%s; using %s", name, raw, default)
        return default


@dataclass
class ReviewConfig:
    backend: str
    model: str
    host: str
    timeout: float
    max_chars: int
    max_segments: int
    max_segment_chars: int
    prompt: str
    num_predict: int
    temperature: float

    @classmethod
    def from_env(cls) -> "ReviewConfig":
        backend = os.getenv("MEETING_SUMMARY_REVIEW_BACKEND", "").strip().lower()
        prompt = os.getenv("MEETING_SUMMARY_REVIEW_PROMPT") or DEFAULT_REVIEW_PROMPT
        num_predict_env = os.getenv("MEETING_SUMMARY_REVIEW_NUM_PREDICT") or os.getenv("SUMMARY_REVIEW_NUM_PREDICT") or "192"
        temperature_env = os.getenv("MEETING_SUMMARY_REVIEW_TEMPERATURE") or os.getenv("SUMMARY_REVIEW_TEMPERATURE") or "0.1"
        try:
            num_predict = max(64, int(num_predict_env))
        except ValueError:
            num_predict = 512
        try:
            temperature = float(temperature_env)
        except ValueError:
            temperature = 0.2
        raw_model = os.getenv("MEETING_SUMMARY_REVIEW_MODEL", "").strip()
        return cls(
            backend=backend,
            model=raw_model,
            host=os.getenv("MEETING_SUMMARY_REVIEW_HOST", ""),
            timeout=max(0.0, _float_env("MEETING_SUMMARY_REVIEW_TIMEOUT", 45.0)),
            max_chars=max(0, _int_env("MEETING_SUMMARY_REVIEW_MAX_CHARS", 1200)),
            max_segments=max(1, _int_env("MEETING_SUMMARY_REVIEW_MAX_SEGMENTS", 4)),
            max_segment_chars=max(0, _int_env("MEETING_SUMMARY_REVIEW_SEGMENT_CHARS", 320)),
            prompt=prompt.strip(),
            num_predict=num_predict,
            temperature=max(0.0, temperature),
        )


class SummaryReviewer:
    """Lightweight controller that optionally refines meeting summaries with an LLM."""

    def __init__(self, config: ReviewConfig | None = None) -> None:
        self.config = config or ReviewConfig.from_env()
        self.backend = self.config.backend
        self._enabled = bool(self.backend)
        self._ollama_cmd = None
        self.model: Optional[str] = None
        if self.backend == "ollama":
            self._ollama_cmd = shutil.which("ollama")
            if not self._ollama_cmd:
                LOGGER.warning("ollama backend requested but executable not found; disabling reviewer")
                self._enabled = False
            else:
                self.model = self._resolve_model(self.config.model)
                if self.model:
                    LOGGER.info("summary reviewer configured: backend=ollama model=%s", self.model)
                else:
                    LOGGER.info("summary reviewer backend=ollama (model will be resolved at runtime)")
        elif self.backend:
            self.model = self.config.model or None

    @classmethod
    def from_env(cls) -> "SummaryReviewer":
        return cls(ReviewConfig.from_env())

    def is_enabled(self) -> bool:
        return self._enabled

    def review(
        self,
        job: MeetingJobConfig,
        summary: MeetingSummary,
        transcription: MeetingTranscriptionResult,
        *,
        issues: Optional[List[str]] = None,
        focus_keywords: Optional[List[str]] = None,
    ) -> Optional[MeetingSummary]:
        if not self._enabled:
            return None

        payload = self._build_payload(summary, transcription, issues=issues, focus_keywords=focus_keywords)
        if not payload:
            return None

        prompt = self._build_prompt(payload)
        raw_response = self._invoke_backend(prompt)
        if raw_response is None:
            return None

        parsed = self._parse_response(raw_response)
        if parsed is None:
            LOGGER.warning("review backend returned non-JSON payload; keeping original summary")
            return None

        updated = self._apply_updates(summary, parsed)
        if not updated:
            LOGGER.debug("review backend produced no actionable updates; keeping original summary")
            return None
        LOGGER.info(
            "meeting summary reviewer applied updates: backend=%s meeting=%s",
            self.backend,
            job.audio_path.stem,
        )
        return summary

    # ------------------------------------------------------------------
    # Prompt construction helpers
    # ------------------------------------------------------------------
    def _build_payload(
        self,
        summary: MeetingSummary,
        transcription: MeetingTranscriptionResult,
        *,
        issues: Optional[List[str]] = None,
        focus_keywords: Optional[List[str]] = None,
    ) -> Dict[str, Any]:
        snippets = self._collect_snippets(transcription, focus_keywords=focus_keywords)
        return {
            "raw_summary": summary.raw_summary,
            "highlights": summary.highlights,
            "decisions": summary.decisions,
            "action_items": summary.action_items,
            "language": transcription.language,
            "context": summary.context or "",
            "transcript_snippets": snippets,
            "issues": issues or [],
        }

    def _build_prompt(self, payload: Dict[str, Any]) -> str:
        payload_json = json.dumps(payload, ensure_ascii=False, indent=2)
        prompt_template = self.config.prompt or DEFAULT_REVIEW_PROMPT
        return prompt_template.format(payload=payload_json).strip()

    def _collect_snippets(
        self,
        transcription: MeetingTranscriptionResult,
        *,
        focus_keywords: Optional[List[str]] = None,
    ) -> List[Dict[str, Any]]:
        snippets: List[Dict[str, Any]] = []
        remaining_chars = self.config.max_chars
        focus_lower: List[str] = []
        if focus_keywords:
            focus_lower = [kw.lower() for kw in focus_keywords if kw]

        def _add_snippet(segment: dict) -> bool:
            nonlocal remaining_chars
            text = str(segment.get("text") or "").strip()
            if not text:
                return False
            if self.config.max_segment_chars and len(text) > self.config.max_segment_chars:
                text = text[: self.config.max_segment_chars].strip()
            if remaining_chars:
                if len(text) > remaining_chars:
                    text = text[:remaining_chars].strip()
            snippet = {
                "text": text,
                "speaker": segment.get("speaker"),
                "start": segment.get("start"),
                "end": segment.get("end"),
            }
            snippets.append(snippet)
            if remaining_chars:
                remaining_chars = max(0, remaining_chars - len(text))
            return len(snippets) >= self.config.max_segments or (remaining_chars == 0 and self.config.max_chars)

        segments = transcription.segments or []
        if segments:
            # prioritise segments containing focus keywords
            if focus_lower:
                for segment in segments:
                    text = str(segment.get("text") or "")
                    lowered = text.lower()
                    if any(keyword in lowered for keyword in focus_lower):
                        if _add_snippet(segment):
                            break
                if len(snippets) >= self.config.max_segments:
                    return snippets
            # fill remaining slots sequentially
            for segment in segments:
                if _add_snippet(segment):
                    break

        if not snippets:
            text = (transcription.text or "").strip()
            if self.config.max_chars and len(text) > self.config.max_chars:
                text = text[: self.config.max_chars].strip()
            if text:
                snippets.append({"text": text})
        return snippets

    # ------------------------------------------------------------------
    # Backend invocation
    # ------------------------------------------------------------------
    def _invoke_backend(self, prompt: str) -> Optional[str]:
        if self.backend == "ollama":
            return self._invoke_ollama(prompt)
        LOGGER.warning("Unknown review backend '%s'; disabling reviewer", self.backend)
        self._enabled = False
        return None

    def _invoke_ollama(self, prompt: str) -> Optional[str]:
        if not self._ollama_cmd:
            return None
        model = self.model or self._resolve_model(self.config.model)
        if not model:
            LOGGER.warning("ollama reviewer has no model configured; skipping review")
            return None
        env = os.environ.copy()
        if self.config.host:
            env["OLLAMA_HOST"] = self.config.host
        env.setdefault("OLLAMA_NUM_PREDICT", str(self.config.num_predict))
        env.setdefault("OLLAMA_TEMPERATURE", str(self.config.temperature))
        try:
            result = subprocess.run(
                ["ollama", "run", model],
                input=prompt,
                capture_output=True,
                text=True,
                env=env,
                timeout=self.config.timeout or None,
                check=False,
            )
        except subprocess.TimeoutExpired:
            LOGGER.warning("ollama review timed out after %.1fs", self.config.timeout)
            return None
        except FileNotFoundError:
            LOGGER.warning("ollama executable not found during review run")
            self._enabled = False
            return None

        if result.returncode != 0:
            stderr = (result.stderr or "").strip()
            stdout = (result.stdout or "").strip()
            error_text = stderr or stdout or "unknown error"
            LOGGER.warning("ollama review failed (%s): %s", result.returncode, error_text)
            return None
        return (result.stdout or "").strip()

    def _resolve_model(self, raw: str) -> Optional[str]:
        candidates = self._candidate_models(raw)
        if self.backend == "ollama" and self._ollama_cmd:
            available = self._list_ollama_models()
            if available:
                for candidate in candidates:
                    if candidate in available:
                        return candidate
        for candidate in candidates:
            if candidate:
                return candidate
        return None

    def _candidate_models(self, raw: str) -> List[str]:
        candidates: List[str] = []
        if raw:
            candidates.extend(
                [item.strip() for item in raw.replace(";", ",").split(",") if item.strip()]
            )
        fallback_sources = [
            "eeve_korean_v2",
            os.getenv("MEETING_SUMMARY_OLLAMA_MODEL", "").strip(),
            os.getenv("LNPCHAT_LLM_MODEL", "").strip(),
            "llama3",
        ]
        for source in fallback_sources:
            if not source:
                continue
            for item in source.replace(";", ",").split(","):
                candidate = item.strip()
                if candidate:
                    candidates.append(candidate)
        unique: List[str] = []
        for candidate in candidates:
            if candidate not in unique:
                unique.append(candidate)
        return unique

    def _list_ollama_models(self) -> List[str]:
        if not self._ollama_cmd:
            return []
        env = os.environ.copy()
        if self.config.host:
            env["OLLAMA_HOST"] = self.config.host
        try:
            result = subprocess.run(
                ["ollama", "list", "--format", "json"],
                capture_output=True,
                text=True,
                env=env,
                timeout=5.0,
                check=False,
            )
        except Exception:
            return []
        if result.returncode != 0:
            return []
        try:
            payload = json.loads(result.stdout or "[]")
        except json.JSONDecodeError:
            return []
        models: List[str] = []
        if isinstance(payload, list):
            for item in payload:
                if isinstance(item, dict):
                    name = str(item.get("name") or "").strip()
                    if name:
                        models.append(name)
        return models

    # ------------------------------------------------------------------
    # Response parsing
    # ------------------------------------------------------------------
    def _parse_response(self, text: str) -> Optional[Dict[str, Any]]:
        cleaned = text.strip()
        if not cleaned:
            return None
        if cleaned.startswith("```"):
            cleaned = self._strip_code_fence(cleaned)
        try:
            return json.loads(cleaned)
        except json.JSONDecodeError:
            pass

        match = re.search(r"\{.*\}", cleaned, re.DOTALL)
        if match:
            candidate = match.group(0)
            try:
                return json.loads(candidate)
            except json.JSONDecodeError:
                return None
        return None

    @staticmethod
    def _strip_code_fence(text: str) -> str:
        parts = re.split(r"```(?:json)?", text, flags=re.IGNORECASE)
        if len(parts) < 2:
            return text
        candidate = parts[1]
        candidate = candidate.split("```", 1)[0]
        return candidate.strip()

    # ------------------------------------------------------------------
    # Summary updates
    # ------------------------------------------------------------------
    def _apply_updates(self, summary: MeetingSummary, payload: Dict[str, Any]) -> bool:
        updated = False

        new_summary = payload.get("summary")
        if isinstance(new_summary, str) and new_summary.strip():
            summary.raw_summary = new_summary.strip()
            updated = True

        highlight_list = self._normalise_string_list(payload.get("highlights"))
        if highlight_list:
            summary.highlights = highlight_list
            summary.structured_summary["highlights"] = [{"text": item} for item in highlight_list]
            updated = True

        decision_list = self._normalise_string_list(payload.get("decisions"))
        if decision_list:
            summary.decisions = decision_list
            summary.structured_summary["decisions"] = [{"text": item} for item in decision_list]
            updated = True

        action_list = self._normalise_string_list(payload.get("action_items"))
        if action_list:
            summary.action_items = action_list
            summary.structured_summary["action_items"] = [{"text": item} for item in action_list]
            updated = True

        notes = payload.get("notes")
        if isinstance(notes, str) and notes.strip():
            summary.structured_summary["review_notes"] = notes.strip()
            updated = True

        return updated

    @staticmethod
    def _normalise_string_list(value: Any) -> List[str]:
        if value is None:
            return []
        items: List[str] = []
        if isinstance(value, str):
            candidate = value.strip()
            if candidate:
                items.append(candidate)
            return items
        if isinstance(value, dict):
            text = str(value.get("text") or "").strip()
            if text:
                items.append(text)
            return items
        if not isinstance(value, list):
            return items
        for entry in value:
            if isinstance(entry, str):
                candidate = entry.strip()
                if candidate:
                    items.append(candidate)
            elif isinstance(entry, dict):
                text = str(entry.get("text") or "").strip()
                owner = str(entry.get("owner") or "").strip()
                due = str(entry.get("due") or "").strip()
                combined = text
                details: List[str] = []
                if owner:
                    details.append(f"담당: {owner}")
                if due:
                    details.append(f"기한: {due}")
                if details:
                    combined = f"{text} ({', '.join(details)})".strip()
                if combined:
                    items.append(combined)
        return items
