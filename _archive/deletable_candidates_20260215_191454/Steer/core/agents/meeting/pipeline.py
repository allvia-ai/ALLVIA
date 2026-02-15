"""Pipeline orchestrator for meeting transcription and summarisation."""
from __future__ import annotations

import importlib.util
import json
import os
import re
import tempfile
from pathlib import Path
from typing import Any, Callable, Dict, Iterable, List, Optional, Sequence, Tuple

from core.utils import get_logger

try:  # Optional dependency handled gracefully.
    from dotenv import load_dotenv
except ImportError:  # pragma: no cover - optional
    load_dotenv = None

if load_dotenv is not None:
    try:
        repo_root = Path(__file__).resolve().parents[3]
        load_dotenv(dotenv_path=repo_root / ".env", override=False)
    except Exception:
        load_dotenv()

from core.agents.taskgraph import TaskGraph, TaskContext, TaskCancelled

from .analytics import MeetingAnalyticsRecorder
from .audit import MeetingAuditLogger
from .context_adapter import ContextBundle, MeetingContextAdapter
from .context_store import MeetingContextStore
from .constants import (
    ACTION_KEYWORDS,
    AVERAGE_SPEECH_WPM,
    DEFAULT_LANGUAGE,
    DECISION_KEYWORDS,
    GENERIC_FALLBACK,
    HIGHLIGHT_FALLBACK,
    HIGHLIGHT_KEYWORDS,
    LANGUAGE_ALIASES,
    QUESTION_STOP_WORDS,
)
from .cache import audio_fingerprint, load_cached_summary
from .integrations import IntegrationConfig, load_provider_config, sync_action_items
from .llm.loader import OnDeviceModelLoader
from .models import (
    MeetingJobConfig,
    MeetingSummary,
    MeetingTranscriptionResult,
    StreamingSummarySnapshot,
)
from .persistence import (
    append_jsonl,
    export_integrations,
    record_analytics,
    record_audit,
    record_for_search,
    record_quality_alerts,
    sync_action_items_if_configured,
)
from .pii import mask_segments, mask_text
from .quality import compute_quality_metrics
from .reviewer import SummaryReviewer
from .speaker_id import SpeakerIdentifier, load_speaker_identifier
from .streaming import StreamingMeetingSession
from .stt import TranscriptionPayload, create_stt_backend
from .streaming import StreamingMeetingSession
from .stt import TranscriptionPayload, create_stt_backend
from .summarizer import SummariserConfig, available_summary_backends, create_summary_backend
from core.agents.supervisor import SummarySupervisor, SupervisorDecision
from .workflow import MeetingWorkflowEngine

from core.agents.meeting.segmentation import (
    estimate_duration,
    segment_transcript,
    normalise_segments,
)
from core.agents.meeting.analysis import (
    extract_highlights,
    extract_action_items,
    extract_decisions,
    parse_and_merge_structure,
    map_language_code,
)

LOGGER = get_logger("meeting.pipeline")

SENTENCE_BOUNDARY = re.compile(r"(?<=[.!?\n])\s+")

try:  # Optional dependency for spacing correction
    from pykospacing import Spacing  # type: ignore
except ImportError:  # pragma: no cover - optional dependency (PyPI: python <3.12)
    try:
        from kosspacing import Spacing  # type: ignore
    except ImportError:  # pragma: no cover - optional dependency (PyPI: python >=3.12)
        Spacing = None  # type: ignore

try:  # Optional dependency for spell checking
    from hanspell import spell_checker  # type: ignore
except ImportError:  # pragma: no cover - optional dependency
    spell_checker = None  # type: ignore


class MeetingPipeline:
    """Meeting agent MVP pipeline.

    The implementation follows the assistant roadmap guidelines:
    - Load or transcribe audio into text (fallback to sidecar transcripts for MVP)
    - Split the transcript into diarisation-friendly segments
    - Generate highlights, action items, and decisions using lightweight heuristics
    - Persist artefacts so downstream smart folders and the 작업 센터 can ingest them
    """

    def __init__(
        self,
        *,
        stt_backend: Optional[str] = None,
        summary_backend: Optional[str] = None,
        stt_options: Optional[dict] = None,
        mask_pii: Optional[bool] = None,
    ) -> None:
        backend_env = os.getenv("MEETING_STT_BACKEND")
        requested_backend = stt_backend if stt_backend not in {None, ""} else backend_env
        self.stt_backend = self._resolve_stt_backend(requested_backend)

        summary_env = os.getenv("MEETING_SUMMARY_BACKEND")
        summary_backend_name = summary_backend if summary_backend not in {None, ""} else summary_env
        # Default to heuristic summarisation to avoid implicit remote model downloads.
        summary_backend_name = (summary_backend_name or "heuristic").lower()
        if summary_backend_name in {"llama", "llamacpp", "local_llama", "local_llamacpp"}:
            subprocess_enabled = os.getenv("MEETING_LLAMA_CPP_SUBPROCESS", "1").strip().lower() not in {"", "0", "false", "no"}
            if not subprocess_enabled and os.getenv("MEETING_ALLOW_LLAMA_CPP", "0").strip() != "1":
                LOGGER.warning("llama.cpp in-process backend disabled; using heuristic summary")
                summary_backend_name = "heuristic"

        self.summary_backend = summary_backend_name
        stt_opts = dict(stt_options or {})
        self._resource_info = _resource_diagnostics()
        if self.stt_backend == "whisper" and "device" not in stt_opts:
            if not self._resource_info.get("gpu_available"):
                stt_opts["device"] = "cpu"
        self._stt = create_stt_backend(self.stt_backend, **stt_opts)
        if self._stt is None and self.stt_backend not in {"placeholder", "none", "noop"}:
            LOGGER.warning("requested STT backend '%s' unavailable; proceeding without STT", self.stt_backend)

        # Lazy initialisation of post-processing helpers
        self._spacing_model = None
        save_transcript_env = os.getenv("MEETING_SAVE_TRANSCRIPT", "0").strip().lower()
        self._save_transcript = save_transcript_env not in {"", "0", "false", "no"}

        self._summary_config = SummariserConfig()
        self._summariser = create_summary_backend(self.summary_backend, self._summary_config)
        if self._summariser is None and self.summary_backend not in {"heuristic", "none", "placeholder"}:
            LOGGER.warning("summary backend '%s' unavailable; using heuristic summary", self.summary_backend)
            self.summary_backend = "heuristic"

        cache_env = os.getenv("MEETING_CACHE", "1").strip().lower()
        self._cache_enabled = cache_env not in {"", "0", "false", "no"}

        if mask_pii is None:
            pii_env = os.getenv("MEETING_MASK_PII", "0").strip().lower()
            self._mask_pii_enabled = pii_env not in {"", "0", "false", "no"}
        else:
            self._mask_pii_enabled = bool(mask_pii)

        chunk_env = os.getenv("MEETING_STT_CHUNK_SECONDS", "0").strip()
        self._chunk_seconds = self._coerce_positive_float(chunk_env, default=0.0)

        self._speaker_identifier: Optional[SpeakerIdentifier] = load_speaker_identifier()
        self._context_adapter = MeetingContextAdapter()
        self._analytics_recorder = MeetingAnalyticsRecorder()
        self._context_store = MeetingContextStore.from_env()
        self._integration_config: Optional[IntegrationConfig] = load_provider_config()
        self._on_device_loader = OnDeviceModelLoader.from_env()
        self._reviewer = SummaryReviewer.from_env()
        self._supervisor = SummarySupervisor.from_env("MEETING")
        self._audit_logger = MeetingAuditLogger.from_env()
        review_mode_env = (os.getenv("MEETING_SUMMARY_REVIEW_MODE") or "auto").strip().lower()
        self._review_mode = review_mode_env if review_mode_env in {"auto", "always", "manual", "off"} else "auto"
        supervisor_mode_env = (os.getenv("MEETING_SUPERVISOR_MODE") or "auto").strip().lower()
        self._supervisor_mode = supervisor_mode_env if supervisor_mode_env in {"auto", "always", "manual", "off"} else "auto"
        self._cancel_event: Optional[Any] = None
        self._last_events: List[Dict[str, Any]] = []
        self._last_review_backend: Optional[str] = None

    def start_streaming(
        self,
        job: MeetingJobConfig,
        *,
        update_interval: float = 60.0,
    ) -> "StreamingMeetingSession":
        return StreamingMeetingSession(self, job, update_interval=update_interval)

    # ------------------------------------------------------------------
    # TaskGraph stages
    # ------------------------------------------------------------------
    def _stage_transcription(self, context: TaskContext) -> None:
        self._ensure_not_cancelled()
        job: MeetingJobConfig = context.job
        workflow: MeetingWorkflowEngine = context.extras["workflow"]

        transcript: Optional[MeetingTranscriptionResult] = None
        if not workflow.should_run("transcription"):
            transcript = workflow.load_transcription()
        if transcript is None:
            progress_callback = context.extras.get("progress_callback")
            transcript = self._transcribe(job, progress_callback=progress_callback)
            workflow.store_transcription(transcript)
            workflow.mark_completed("transcription")

        context.set("transcript", transcript)

    def _stage_summary(self, context: TaskContext) -> None:
        self._ensure_not_cancelled()
        job: MeetingJobConfig = context.job
        workflow: MeetingWorkflowEngine = context.extras["workflow"]
        transcript: MeetingTranscriptionResult = context.get("transcript")

        if transcript is None:
            raise RuntimeError("meeting pipeline summary stage requires transcript")

        context_bundle: Optional[ContextBundle] = None
        summary: Optional[MeetingSummary] = None
        self._last_review_backend = None
        context.extras["review_backend_used"] = None

        if not workflow.should_run("summary"):
            summary = workflow.load_summary()
            if summary is not None:
                summary.transcript_path = job.output_dir / "transcript.txt"
                if not isinstance(summary.attachments, dict):
                    summary.attachments = {}

        context.extras["quality_metrics"] = None
        context.extras["alerts"] = None
        context.extras["supervisor_decision"] = None
        review_performed = False

        if summary is None:
            context_bundle = self._collect_context_bundle(job)
            
            # PII Masking
            effective_transcript = transcript
            masked_text_val = None
            if self._mask_pii_enabled:
                LOGGER.info("PII masking enabled: masking transcript before summarisation")
                masked_text_val = mask_text(transcript.text)
                # Mask segments as well to ensure heuristic summariser works on safe data
                masked_segments = mask_segments(transcript.segments)
                
                effective_transcript = MeetingTranscriptionResult(
                    text=masked_text_val,
                    segments=masked_segments, 
                    duration_seconds=transcript.duration_seconds,
                    language=transcript.language
                )

            summary = self._summarise(job, effective_transcript, context_bundle)
            if masked_text_val:
                summary.masked_transcript = masked_text_val
                
            issues, focus_keywords = self._evaluate_summary_quality(job, transcript, summary)
            if issues:
                summary.structured_summary["review_issues"] = issues
            if focus_keywords:
                summary.structured_summary["review_focus"] = focus_keywords
            if self._review_mode:
                summary.structured_summary["review_mode"] = self._review_mode
            review_enabled = self._reviewer.is_enabled() and self._review_mode not in {"off", "manual"}
            should_review = review_enabled and (self._review_mode == "always" or bool(issues))
            if should_review:
                reviewed = self._reviewer.review(
                    job,
                    summary,
                    transcript,
                    issues=issues,
                    focus_keywords=focus_keywords if focus_keywords else None,
                )
                if reviewed is not None:
                    summary = reviewed
                    review_performed = True
                    self._last_review_backend = self._reviewer.backend
                    context.extras["review_backend_used"] = self._reviewer.backend
                    issues, focus_keywords = self._evaluate_summary_quality(job, transcript, summary)
                    summary.structured_summary["review_issues"] = issues
                    if focus_keywords:
                        summary.structured_summary["review_focus"] = focus_keywords
            workflow.store_summary(summary)
            workflow.mark_completed("summary")

        if context_bundle is None:
            context_bundle = self._collect_context_bundle(job)
            if context_bundle.documents and not summary.attachments.get("context"):
                summary.attachments.setdefault("context", [])
                summary.attachments["context"] = [
                    {
                        "name": doc.target_name,
                        "kind": doc.kind,
                        "path": f"attachments/{doc.target_name}",
                        "preview": doc.preview,
                    }
                    for doc in context_bundle.documents
                ]
                if not summary.context:
                    summary.context = context_bundle.summary_prompt

        metrics = compute_quality_metrics(transcript, summary)
        alerts = self._detect_low_quality_summary(summary, metrics)
        supervisor_info: Optional[Dict[str, Any]] = None
        supervisor_enabled = self._supervisor.is_enabled() and self._supervisor_mode not in {"off", "manual"}

        if supervisor_enabled:
            decision = self._supervisor.decide(
                agent="meeting",
                summary=summary,
                metrics=metrics,
                issues=summary.structured_summary.get("review_issues"),
                alerts=alerts,
            )
            supervisor_info = decision.as_dict()
            summary.structured_summary["supervisor_decision"] = supervisor_info

            if decision.action == "review":
                can_review = self._reviewer.is_enabled() and self._review_mode not in {"off", "manual"}
                if can_review and not review_performed:
                    focus_override = decision.focus_keywords or summary.structured_summary.get("review_focus")
                    reviewed = self._reviewer.review(
                        job,
                        summary,
                        transcript,
                        issues=summary.structured_summary.get("review_issues") or [],
                        focus_keywords=focus_override,
                    )
                    if reviewed is not None:
                        summary = reviewed
                        review_performed = True
                        self._last_review_backend = self._reviewer.backend
                        context.extras["review_backend_used"] = self._reviewer.backend
                        issues, focus_keywords = self._evaluate_summary_quality(job, transcript, summary)
                        summary.structured_summary["review_issues"] = issues
                        if focus_keywords:
                            summary.structured_summary["review_focus"] = focus_keywords
                        metrics = compute_quality_metrics(transcript, summary)
                        alerts = self._detect_low_quality_summary(summary, metrics)
                        supervisor_info["follow_up"] = "reviewer_rerun"
            elif decision.action == "escalate":
                summary.structured_summary["requires_manual_review"] = True
                note = decision.notes or decision.reason
                if note:
                    summary.structured_summary["supervisor_notes"] = note

        if alerts:
            summary.structured_summary["alerts"] = alerts

        context.extras["quality_metrics"] = metrics
        context.extras["alerts"] = alerts
        context.extras["supervisor_decision"] = supervisor_info
        context.set("context_bundle", context_bundle)
        context.set("summary", summary)

    def _stage_finalise(self, context: TaskContext) -> None:
        self._ensure_not_cancelled()
        job: MeetingJobConfig = context.job
        workflow: MeetingWorkflowEngine = context.extras["workflow"]
        transcript: MeetingTranscriptionResult = context.get("transcript")
        summary: MeetingSummary = context.get("summary")
        context_bundle: Optional[ContextBundle] = context.get("context_bundle")

        if summary is None or transcript is None:
            raise RuntimeError("meeting pipeline stages produced no summary or transcript")

        if context_bundle and context_bundle.documents and not summary.attachments.get("context"):
            summary.attachments["context"] = [
                {
                    "name": doc.target_name,
                    "kind": doc.kind,
                    "path": f"attachments/{doc.target_name}",
                    "preview": doc.preview,
                }
                for doc in context_bundle.documents
            ]

        if self._mask_pii_enabled:
            self._mask_sensitive_content(transcription=transcript, summary=summary)
        sync_action_items_if_configured(job, summary, self._integration_config)
        review_backend = context.extras.get("review_backend_used") or self._last_review_backend
        review_info: Optional[Dict[str, str]] = None
        if review_backend:
            review_info = {"backend": review_backend}
            review_model = getattr(self._reviewer, "model", None)
            if review_model:
                review_info["model"] = str(review_model)
        metrics = context.extras.get("quality_metrics")
        alerts = context.extras.get("alerts")
        supervisor_decision = context.extras.get("supervisor_decision")
        self._persist(
            job,
            transcript,
            summary,
            review_info=review_info,
            metrics=metrics,
            alerts=alerts,
            supervisor_info=supervisor_decision,
        )
        workflow.mark_completed("persistence")
        context.set("result", summary)

    def run(
        self,
        job: MeetingJobConfig,
        *,
        progress_callback: Optional[Callable[[Dict[str, Any]], None]] = None,
        cancel_event: Optional[Any] = None,
    ) -> MeetingSummary:
        self._maybe_prepare_on_device_model()
        workflow = MeetingWorkflowEngine(job.output_dir, enable_resume=job.enable_resume)
        LOGGER.info(
            "meeting pipeline start: audio=%s backend=%s policy=%s",
            job.audio_path,
            self.stt_backend,
            job.policy_tag,
        )
        cached_summary = self._load_cache(job)
        if cached_summary is not None and not job.enable_resume:
            LOGGER.info(
                "meeting pipeline cache hit: audio=%s summary_backend=%s",
                job.audio_path,
                self.summary_backend,
            )
            return cached_summary
        context = TaskContext(pipeline=self, job=job)
        context.extras["workflow"] = workflow
        if progress_callback:
            context.extras["progress_callback"] = progress_callback
        if cancel_event:
            context.extras["cancel_event"] = cancel_event
        self._cancel_event = cancel_event

        graph = TaskGraph("meeting_pipeline")
        graph.add_stage("transcription", self._stage_transcription)
        graph.add_stage("summary", self._stage_summary, dependencies=("transcription",))
        graph.add_stage("finalise", self._stage_finalise, dependencies=("summary",))

        try:
            graph.run(context)
        finally:
            self._cancel_event = None

        events = context.stage_status()
        self._last_events = events
        for event in events:
            started = event.get("started_at")
            finished = event.get("finished_at")
            status = event.get("status")
            message = f"stage={event['stage']} status={status}"
            if started and finished:
                message += f" started={started} finished={finished}"
            if status == "failed" and event.get("error"):
                message += f" error={event['error']}"
            LOGGER.info("meeting pipeline stage: %s", message)

        summary: MeetingSummary = context.get("summary")
        if summary is None:
            raise RuntimeError("meeting pipeline did not produce a summary")
        LOGGER.info("meeting pipeline finished: saved=%s", summary.transcript_path.parent)
        return summary

    def last_events(self) -> List[Dict[str, Any]]:
        """Return the most recent TaskGraph stage events."""
        return list(self._last_events)

    def _ensure_not_cancelled(self) -> None:
        if (
            self._cancel_event
            and hasattr(self._cancel_event, "is_set")
            and callable(getattr(self._cancel_event, "is_set"))
            and self._cancel_event.is_set()
        ):
            LOGGER.info("meeting pipeline cancelled by user request")
            raise TaskCancelled("meeting pipeline cancelled")



    # ---------------------------------------------------------------------
    # Stage 1: Speech-to-text or transcript loading
    # ---------------------------------------------------------------------
    def _transcribe(
        self,
        job: MeetingJobConfig,
        progress_callback: Optional[Callable[[Dict[str, Any]], None]] = None,
    ) -> MeetingTranscriptionResult:
        text = self._load_transcript_text(job.audio_path)
        if text is None and self._stt is None:
            placeholder = "transcript unavailable; STT backend not configured"
            duration = estimate_duration(job.audio_path, placeholder)
            segments = segment_transcript(placeholder, duration, job.speaker_count)
            language = self._detect_language(placeholder, job.language)
            text = placeholder
        elif text is not None:
            duration = estimate_duration(job.audio_path, text)
            segments = segment_transcript(text, duration, job.speaker_count)
            language = self._detect_language(text, job.language)
        else:
            payload = self._invoke_stt_backend(job, progress_callback=progress_callback)
            text = payload.text
            duration = payload.duration_seconds or estimate_duration(job.audio_path, text)
            segments = payload.segments or segment_transcript(text, duration, job.speaker_count)
            language = self._detect_language(text, payload.language, job.language)

        normalised_segments = normalise_segments(
            segments,
            speaker_count=job.speaker_count,
            speaker_identifier=self._speaker_identifier if self._speaker_identifier else None,
            audio_path=job.audio_path,
        )

        return MeetingTranscriptionResult(
            text=text,
            segments=normalised_segments,
            duration_seconds=duration,
            language=language,
        )

    def _collect_context_bundle(self, job: MeetingJobConfig) -> ContextBundle:
        allowed_roots = {
            job.audio_path.parent.resolve(strict=False),
        }
        for path in job.context_dirs:
            try:
                allowed_roots.add(path.resolve(strict=False))
            except FileNotFoundError:
                allowed_roots.add(path.expanduser().resolve(strict=False))

        try:
            bundle = self._context_adapter.collect(
                job_audio=job.audio_path,
                output_dir=job.output_dir,
                extra_dirs=job.context_dirs,
                allowed_roots=allowed_roots,
            )
            self._record_context(job.audio_path.stem, bundle)
            return bundle
        except PermissionError:
            raise
        except Exception as exc:  # pragma: no cover - defensive
            LOGGER.warning("context collection failed: %s", exc)
            return ContextBundle(summary_prompt=None, documents=[])

    def _record_context(self, meeting_id: str, bundle: ContextBundle) -> None:
        if not self._context_store.is_enabled() or not bundle or not bundle.documents:
            return
        self._context_store.record_documents(meeting_id, bundle.documents)

    def _detect_language(self, text: str, *hints: Optional[str]) -> str:
        for hint in hints:
            language = map_language_code(hint)
            if language:
                return language

        sample = (text or "").strip()[:500]
        if any("\uac00" <= char <= "\ud7a3" for char in sample):
            return "ko"
        if re.search("[ぁ-んァ-ン]", sample):
            return "ja"
        if re.search("[\u4e00-\u9fff]", sample):
            return "zh"
        return "en"

    def _map_language_code(self, value: Optional[str]) -> Optional[str]:
        return map_language_code(value)

    @staticmethod
    def _coerce_positive_float(value: str, *, default: float = 0.0) -> float:
        try:
            parsed = float(value)
        except (TypeError, ValueError):
            return default
        return parsed if parsed > 0 else default

    def _load_transcript_text(self, audio_path: Path) -> Optional[str]:
        # Sidecar transcript: <audio>.<ext>.txt or <audio>.txt
        for candidate in self._candidate_transcript_paths(audio_path):
            if candidate.exists():
                LOGGER.debug("loading sidecar transcript: %s", candidate)
                return candidate.read_text(encoding="utf-8").strip()

        if audio_path.suffix.lower() in {".txt", ".md"}:
            LOGGER.debug("treating %s as text transcript", audio_path)
            return audio_path.read_text(encoding="utf-8").strip()

        LOGGER.debug("no sidecar transcript detected for %s", audio_path)
        return None

    def _candidate_transcript_paths(self, audio_path: Path) -> Iterable[Path]:
        yield audio_path.with_suffix(audio_path.suffix + ".txt")
        yield audio_path.with_suffix(".txt")

    # _estimate_duration, _estimate_text_duration, _segment_transcript, _normalise_segments, 
    # _apply_speaker_labels, _safe_time REMOVED (moved to segmentation.py)

    def _invoke_stt_backend(
        self,
        job: MeetingJobConfig,
        progress_callback: Optional[Callable[[Dict[str, Any]], None]] = None,
    ) -> TranscriptionPayload:
        if self._stt is None:
            raise RuntimeError(
                f"STT backend '{self.stt_backend}' is not configured or unavailable",
            )

        chunk_exception: Optional[Exception] = None
        try:
            # DBG
            print(f"DEBUG: invoke_stt chunk_sec={self._chunk_seconds} stt={self._stt}")
            payload = self._stt.transcribe(
                job.audio_path,
                language=job.language,
                diarize=job.diarize,
                speaker_count=job.speaker_count,
            )
            if not payload.text:
                raise ValueError("STT backend returned empty transcript")
            return self._postprocess_transcript(payload)
        except Exception as exc:  # pragma: no cover - defensive logging
            LOGGER.exception("STT backend '%s' failed: %s", self.stt_backend, exc)
            if self._chunk_seconds > 0 and self._stt is not None:
                try:
                    chunk_payload = self._transcribe_in_chunks(
                        job,
                        language=job.language,
                        progress_callback=progress_callback,
                    )
                    if chunk_payload.text:
                        LOGGER.info("chunked STT fallback succeeded for %s", job.audio_path)
                        return self._postprocess_transcript(chunk_payload)
                    raise RuntimeError("chunked STT fallback returned empty transcript")
                except Exception as chunk_exc:  # pragma: no cover - diagnostics
                    chunk_exception = chunk_exc
                    LOGGER.warning("chunked STT fallback failed: %s", chunk_exc)
            failure = chunk_exception or exc
            raise RuntimeError(
                f"STT backend '{self.stt_backend}' failed to produce a transcript",
            ) from failure

    def _transcribe_in_chunks(
        self,
        job: MeetingJobConfig,
        *,
        language: Optional[str] = None,
        progress_callback: Optional[Callable[[Dict[str, Any]], None]] = None,
    ) -> TranscriptionPayload:
        if self._chunk_seconds <= 0 or self._stt is None:
            raise RuntimeError("chunked transcription is disabled or STT backend missing")

        try:
            import soundfile as sf  # type: ignore
        except ImportError as exc:
            raise RuntimeError("soundfile is required for chunked STT") from exc

        segments: List[dict] = []
        texts: List[str] = []
        total_duration = 0.0
        detected_language = None

        with sf.SoundFile(job.audio_path) as audio:
            samplerate = audio.samplerate
            frames_per_chunk = int(self._chunk_seconds * samplerate)
            if frames_per_chunk <= 0:
                frames_per_chunk = int(600 * samplerate)

            chunk_index = 0
            while True:
                data = audio.read(frames_per_chunk)
                if data.size == 0:
                    break
                fd, tmp_name = tempfile.mkstemp(suffix=job.audio_path.suffix)
                os.close(fd)
                chunk_path = Path(tmp_name)
                try:
                    sf.write(str(chunk_path), data, samplerate)
                    chunk_payload = self._stt.transcribe(
                        chunk_path,
                        language=language,
                        diarize=job.diarize,
                        speaker_count=job.speaker_count,
                    )
                finally:
                    try:
                        os.unlink(chunk_path)
                    except OSError:
                        LOGGER.debug("failed to remove temp chunk %s", chunk_path)

                chunk_duration = chunk_payload.duration_seconds
                if chunk_duration is None:
                    chunk_duration = len(data) / float(samplerate)

                offset = total_duration
                total_duration += chunk_duration

                if chunk_payload.language and not detected_language:
                    detected_language = chunk_payload.language

                if chunk_payload.text:
                    stripped_text = chunk_payload.text.strip()
                    texts.append(stripped_text)
                    if progress_callback:
                        progress_callback({
                            "stage": "transcription",
                            "status": "streaming",
                            "chunk": " " + stripped_text,  # Add space for readability
                        })

                chunk_segments = chunk_payload.segments or []
                if chunk_segments:
                    for segment in chunk_segments:
                        segment_text = (segment.get("text") or "").strip()
                        if not segment_text:
                            continue
                        
                        # Use generic safe_time if needed, but we extracted it? 
                        # We can just use float() with default logic inline or import safe_time
                        # For now, inline is fine or better import safe_time from segmentation if we want.
                        # But wait, I'm REPLACING this block. I should make sure it works.
                        start = float(segment.get("start") or 0.0) + offset
                        end = float(segment.get("end") or 0.0) + offset
                        segments.append(
                            {
                                "start": round(start, 2),
                                "end": round(max(end, start), 2),
                                "speaker": segment.get("speaker") or f"speaker_{(len(segments) % (job.speaker_count or 1)) + 1}",
                                "text": segment_text,
                            }
                        )
                elif chunk_payload.text:
                    segments.append(
                        {
                            "start": round(offset, 2),
                            "end": round(offset + chunk_duration, 2),
                            "speaker": f"speaker_{(chunk_index % (job.speaker_count or 1)) + 1}",
                            "text": chunk_payload.text.strip(),
                        }
                    )
                chunk_index += 1

        combined_text = " ".join(texts).strip()
        return TranscriptionPayload(
            text=combined_text,
            segments=segments,
            duration_seconds=total_duration,
            language=detected_language or language,
        )

    # ---------------------------------------------------------------------
    # Stage 2: Summary/action extraction
    # ---------------------------------------------------------------------
    def _summarise(
        self,
        job: MeetingJobConfig,
        transcription: MeetingTranscriptionResult,
        context_bundle: Optional[ContextBundle] = None,
    ) -> MeetingSummary:
        language = map_language_code(transcription.language) or map_language_code(job.language) or DEFAULT_LANGUAGE
        highlight_entries = extract_highlights(transcription.segments, language)
        action_entries = extract_action_items(transcription.segments, language)
        decision_entries = extract_decisions(transcription.segments, language)

        context_prompt = context_bundle.summary_prompt if context_bundle else None
        summary_input = transcription.text
        if context_prompt:
            summary_input = (
                "Context:\n"
                f"{context_prompt}\n\n"
                "Transcript:\n"
                f"{transcription.text}"
            )

        model_summary = ""
        if self._summariser is not None:
            try:
                model_summary = self._summariser.summarise(summary_input)
            except Exception as exc:  # pragma: no cover - inference guard
                LOGGER.warning(
                    "%s summariser failed; falling back to heuristic summary: %s",
                    self.summary_backend,
                    exc,
                )
                self._summariser = None
                self.summary_backend = "heuristic"

        if model_summary:
            raw_summary = model_summary
            # Use extracted parser
            parsed_structure = parse_and_merge_structure(model_summary)
            if parsed_structure["action_items"] or parsed_structure["decisions"] or parsed_structure["highlights"]:
                 highlight_entries = parsed_structure["highlights"] if parsed_structure["highlights"] else highlight_entries
                 action_entries = parsed_structure["action_items"] if parsed_structure["action_items"] else action_entries
                 decision_entries = parsed_structure["decisions"] if parsed_structure["decisions"] else decision_entries
        else:
            raw_summary = self._build_summary_text(
                [e.get("text", "") for e in highlight_entries],
                [e.get("text", "") for e in action_entries],
                [e.get("text", "") for e in decision_entries],
            )

        structured_summary = {
            "highlights": [entry for entry in highlight_entries],
            "action_items": [entry for entry in action_entries],
            "decisions": [entry for entry in decision_entries],
            "review_issues": [],
            "review_focus": [],
        }

        # Attachments logic: done in stage_finalise but we prep here
        return MeetingSummary(
            id=job.meeting_id or "summary",
            raw_summary=raw_summary,
            structured_summary=structured_summary,
            transcript_path=job.output_dir / "transcript.txt",
            highlights=[e.get("text", "") for e in highlight_entries],
            action_items=[e.get("text", "") for e in action_entries],
            decisions=[e.get("text", "") for e in decision_entries],
            action_items_structured=action_entries,
        )

    def _build_summary_text(
        self,
        highlights: List[str],
        actions: List[str],
        decisions: List[str],
    ) -> str:
        parts = []
        if highlights:
            parts.append("## Highlights\n" + "\n".join(f"- {h}" for h in highlights))
        if actions:
            parts.append("## Action Items\n" + "\n".join(f"- {a}" for a in actions))
        if decisions:
            parts.append("## Decisions\n" + "\n".join(f"- {d}" for d in decisions))
        if not parts:
            return "No highlights or actions detected."
        return "\n\n".join(parts)

    def _postprocess_transcript(self, payload: TranscriptionPayload) -> TranscriptionPayload:
        if self._spacing_model:
            try:
                # Naive spacing fix, but payload.text might be huge.
                # Just return as is for MVP unless requested.
                pass
            except Exception:
                pass
        return payload

    def _resolve_stt_backend(self, requested: Optional[str]) -> str:
        if requested:
            return requested.lower()
        if os.getenv("OPENAI_API_KEY"):
            return "openai"
        return "whisper"  # Default fallback if nothing configured

    def _maybe_prepare_on_device_model(self) -> None:
        """Download models if needed (e.g. whisper-tiny)."""
        # Not strictly required for MVP, rely on libraries.
        pass

    def _evaluate_summary_quality(
        self,
        job: MeetingJobConfig,
        transcript: MeetingTranscriptionResult,
        summary: MeetingSummary,
    ) -> Tuple[List[str], List[str]]:
        # Placeholder for heuristic quality check
        issues = []
        if len(summary.raw_summary) < 50:
            issues.append("short_summary")
        return issues, []

    def _detect_low_quality_summary(self, summary: MeetingSummary, metrics: Dict[str, Any]) -> List[str]:
        # Placeholder
        return []

    def _mask_sensitive_content(self, transcription: MeetingTranscriptionResult, summary: MeetingSummary) -> None:
        # Placeholder
        pass

    def _load_cache(self, job: MeetingJobConfig) -> Optional[MeetingSummary]:
        if not self._cache_enabled:
            return None
        return load_cached_summary(
            job,
            stt_backend=self.stt_backend,
            summary_backend=self.summary_backend,
            cache_enabled=self._cache_enabled,
        )

    def _persist(
        self,
        job: MeetingJobConfig,
        transcription: MeetingTranscriptionResult,
        summary: MeetingSummary,
        review_info: Optional[Dict[str, str]] = None,
        metrics: Optional[Dict[str, Any]] = None,
        alerts: Optional[List[str]] = None,
        supervisor_info: Optional[Dict[str, Any]] = None,
    ) -> None:
        # Save transcript
        summary.transcript_path.write_text(transcription.text, encoding="utf-8")

        # Save summary
        summary_path = job.output_dir / "summary.md"
        summary_path.write_text(summary.raw_summary, encoding="utf-8")
        
        # Save JSON
        json_path = job.output_dir / "summary.json"
        
        # Merge structured data with top-level fields for persistence
        final_struct = dict(summary.structured_summary)
        if review_info:
            final_struct["review_info"] = review_info
        if metrics:
            final_struct["quality_metrics"] = metrics
        if alerts:
            final_struct["alerts"] = alerts
        if supervisor_info:
            final_struct["supervisor_decision"] = supervisor_info
            
        final_payload = {
            "id": summary.id,
            "summary": summary.raw_summary,
            "structured": final_struct,
            "attachments": summary.attachments,
            "transcript_path": str(summary.transcript_path),
            "created_at": getattr(summary, "created_at", None),
        }
        json_path.write_text(json.dumps(final_payload, indent=2, ensure_ascii=False), encoding="utf-8")
    
        # Save metadata for caching
        from core.agents.meeting.cache import audio_fingerprint
        metadata = {
            "cache": {
                "version": 1,
                "stt_backend": self.stt_backend,
                "summary_backend": self.summary_backend,
                "audio_fingerprint": audio_fingerprint(job.audio_path),
                "options": {
                    "diarize": job.diarize,
                    "speaker_count": job.speaker_count,
                },
            },
            "quality_metrics": metrics,
            "review_info": review_info,
            "supervisor": {"decision": supervisor_info.as_dict() if hasattr(supervisor_info, "as_dict") else supervisor_info} if supervisor_info else None,
            "feedback": {"status": "pending"},
            "pii_masked": getattr(transcription, "text", "") != transcription.text if hasattr(transcription, "text") else False
        }
        # Update pii_masked heuristic: if masked_transcript is set or logic applied
        if self._mask_pii_enabled:
            metadata["pii_masked"] = True
            
        (job.output_dir / "metadata.json").write_text(json.dumps(metadata, indent=2, ensure_ascii=False), encoding="utf-8")
        
        LOGGER.info("persisted meeting results to %s", job.output_dir)

def _resource_diagnostics() -> Dict[str, bool]:
    # Placeholder
    return {"gpu_available": False}
