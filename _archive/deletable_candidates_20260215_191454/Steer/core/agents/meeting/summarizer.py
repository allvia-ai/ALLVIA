"""Summarisation helpers and backends for the meeting agent."""
from __future__ import annotations

import importlib.util
import json
import logging
import os
import re
import shutil
import subprocess
import sys
import textwrap
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Protocol

try:
    from llama_cpp import Llama  # type: ignore
except Exception:  # pragma: no cover - optional dependency
    Llama = None

LOGGER = logging.getLogger(__name__)

SENTENCE_SPLIT = re.compile(r"(?<=[.!?\n])\s+")
DEFAULT_OLLAMA_PROMPT_EN = textwrap.dedent(
    """
    You are a helpful meeting assistant.
    Summarise the following meeting transcript.
    
    Structure your response exactly as follows:
    ## Highlights
    - Key point 1
    - Key point 2

    ## Decisions
    - Decision 1
    - Decision 2

    ## Action Items
    - [Owner] Task description (Due: YYYY-MM-DD or TBD)
    - [Owner] Task description (Due: YYYY-MM-DD or TBD)

    ## Summary
    (Concise summary paragraph)

    Transcript:
    {transcript}
    """
)

DEFAULT_OLLAMA_PROMPT_KO = textwrap.dedent(
    """
    당신은 유능한 회의 비서입니다. 다음 회의 스크립트를 분석하고 한국어로 요약해 주세요.
    
    [화자 1], [화자 2] 등의 표시가 있으면 각 화자별 발언을 구분하여 핵심 내용을 파악해 주세요.
    
    반드시 다음 형식을 정확히 지켜주세요:
    
    ## 📌 회의 주제
    (회의의 주요 목적/주제를 한 문장으로)
    
    ## ⭐ 핵심 논의 사항
    - 주요 논의 내용 1
    - 주요 논의 내용 2
    - 주요 논의 내용 3
    
    ## ✅ 결정 사항
    - 결정된 내용 1
    - 결정된 내용 2
    
    ## 📋 액션 아이템 (할 일)
    - [ ] [담당자] 할 일 내용 (기한: YYYY-MM-DD 또는 미정)
    - [ ] [담당자] 할 일 내용 (기한: YYYY-MM-DD 또는 미정)
    
    ## 📝 요약
    (전체 내용을 3~5문장으로 명확하고 간결하게 요약)
    
    ---
    스크립트:
    {transcript}
    """
)


PROMPTS = {
    "en": DEFAULT_OLLAMA_PROMPT_EN,
    "ko": DEFAULT_OLLAMA_PROMPT_KO,
}


def _int_env(name: str, default: int) -> int:
    raw = os.getenv(name)
    if raw is None:
        return default
    try:
        return int(raw)
    except ValueError:
        LOGGER.warning("Invalid integer for %s=%s; using %s", name, raw, default)
        return default


@dataclass
class SummariserConfig:
    """Common configuration shared by the supported summariser backends."""

    model_name: str = os.getenv("MEETING_SUMMARY_MODEL", "gogamza/kobart-base-v2")
    english_model_name: str = os.getenv("MEETING_SUMMARY_EN_MODEL", "facebook/bart-large-cnn")
    max_length: int = _int_env("MEETING_SUMMARY_MAXLEN", 128)
    min_length: int = _int_env("MEETING_SUMMARY_MINLEN", 32)
    max_new_tokens: int = _int_env("MEETING_SUMMARY_MAX_NEW_TOKENS", 128)
    num_beams: int = _int_env("MEETING_SUMMARY_NUM_BEAMS", 4)
    model_max_input_chars: int = _int_env("MEETING_SUMMARY_MODEL_MAX_CHARS", 1024)
    chunk_char_limit: int = _int_env("MEETING_SUMMARY_CHUNK_CHARS", 1800)

    ollama_model: str = os.getenv("MEETING_SUMMARY_OLLAMA_MODEL", "llama3")
    ollama_host: str = os.getenv("MEETING_SUMMARY_OLLAMA_HOST", "")
    ollama_prompt: str = os.getenv("MEETING_SUMMARY_OLLAMA_PROMPT", DEFAULT_OLLAMA_PROMPT_EN)

    bitnet_model: str = os.getenv("MEETING_SUMMARY_BITNET_MODEL", "bitnet/b1.58-instruct")
    bitnet_max_length: int = _int_env("MEETING_SUMMARY_BITNET_MAXLEN", 220)
    bitnet_min_length: int = _int_env("MEETING_SUMMARY_BITNET_MINLEN", 60)

    llama_model: str = os.getenv("MEETING_SUMMARY_LLAMA_MODEL", "")
    llama_n_ctx: int = _int_env("MEETING_SUMMARY_LLAMA_N_CTX", 4096)
    llama_n_threads: int = _int_env("MEETING_SUMMARY_LLAMA_THREADS", 0)
    llama_gpu_layers: int = _int_env("MEETING_SUMMARY_LLAMA_GPU_LAYERS", 0)
    llama_max_new_tokens: int = _int_env("MEETING_SUMMARY_LLAMA_MAX_NEW_TOKENS", 256)


class BaseSummariser(Protocol):
    """Protocol describing the summariser interface."""

    def summarise(self, text: str) -> str:
        ...

    @staticmethod
    def is_available() -> bool:
        ...


class KoBARTSummariser:
    """Chunked KoBART summarisation with lazy pipeline initialisation."""

    def __init__(self, config: SummariserConfig | None = None) -> None:
        self.config = config or SummariserConfig()
        self._pipelines: Dict[str, Any] = {}
        self._device = self._resolve_device()

    @staticmethod
    def is_available() -> bool:
        return importlib.util.find_spec("transformers") is not None

    def _resolve_device(self) -> int:
        try:
            override = os.getenv("MEETING_SUMMARY_DEVICE")
            if override:
                override = override.strip().lower()
                if override in {"cpu", "-1"}:
                    return -1
                if override in {"cuda", "gpu", "0"}:
                    return 0
                if override.isdigit():
                    return int(override)
            try:
                import torch

                return 0 if torch.cuda.is_available() else -1
            except ImportError:  # pragma: no cover - torch optional
                return -1
        except Exception:  # pragma: no cover - defensive fallback
            return -1

    def _ensure_pipeline(self, flavour: str = "ko"):
        if flavour in self._pipelines:
            return self._pipelines[flavour]

        try:
            from transformers import pipeline  # type: ignore
        except ImportError as exc:  # pragma: no cover - optional dependency
            raise RuntimeError("transformers is required for KoBART summarisation") from exc

        if flavour == "en":
            model_name = self.config.english_model_name
        else:
            model_name = self.config.model_name

        device_label = self._device if isinstance(self._device, int) and self._device >= 0 else "cpu"
        LOGGER.info(
            "Loading %s summariser model: %s on device=%s",  # noqa: G004
            flavour.upper(),
            model_name,
            device_label,
        )
        task = "text2text-generation"
        self._pipelines[flavour] = pipeline(
            task,
            model=model_name,
            tokenizer=model_name,
            device=self._device,
        )
        return self._pipelines[flavour]

    def summarise(self, text: str) -> str:
        text = (text or "").strip()
        if not text:
            return ""

        flavour = "en" if self._is_likely_english(text) else "ko"
        chunks = self._chunk_text(text, self.config.chunk_char_limit)
        summarise_chunk = self._make_chunk_summariser(flavour)
        partials = [summarise_chunk(chunk) for chunk in chunks if chunk.strip()]
        partials = [item for item in partials if item]

        if not partials:
            return ""

        if len(partials) == 1:
            return partials[0]

        combined = " ".join(partials)
        final = summarise_chunk(combined)
        return final or combined

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------
    def _make_chunk_summariser(self, flavour: str) -> Callable[[str], str]:
        transformer_pipeline = self._ensure_pipeline(flavour)
        max_length = self.config.max_length
        min_length = self.config.min_length
        max_new_tokens = self.config.max_new_tokens
        num_beams = self.config.num_beams
        max_input_chars = self.config.model_max_input_chars

        def _summarise_chunk(chunk: str) -> str:
            try:
                trimmed = chunk[:max_input_chars] if max_input_chars > 0 else chunk
                result: List[dict] = transformer_pipeline(
                    trimmed,
                    max_new_tokens=max_new_tokens,
                    max_length=max_length,
                    min_length=min_length,
                    do_sample=False,
                    num_beams=num_beams,
                )
                if not result:
                    return ""
                payload = result[0].get("generated_text") or result[0].get("summary_text") or ""
                return payload.strip()
            except Exception as exc:  # pragma: no cover - inference guard
                LOGGER.exception("KoBART summarisation failed: %s", exc)
                return ""

        return _summarise_chunk

    def _is_likely_english(self, text: str) -> bool:
        # Quick heuristic based on Hangul vs ASCII letter ratio.
        if not text:
            return False

        hangul = sum(1 for ch in text if "가" <= ch <= "힣")
        latin = sum(1 for ch in text if "a" <= ch.lower() <= "z")
        # Treat as English when Hangul is rare and there is a reasonable amount of Latin chars.
        return hangul == 0 and latin > 0 or (latin > 5 and hangul * 5 < latin)

    def _chunk_text(self, text: str, limit: int) -> List[str]:
        if limit <= 0:
            return [text]

        sentences = [sentence.strip() for sentence in SENTENCE_SPLIT.split(text) if sentence.strip()]
        if not sentences:
            return [text]

        chunks: List[str] = []
        current: List[str] = []
        current_len = 0

        for sentence in sentences:
            length = len(sentence)
            if current and current_len + length > limit:
                chunks.append(" ".join(current))
                current = [sentence]
                current_len = length
            else:
                current.append(sentence)
                current_len += length

        if current:
            chunks.append(" ".join(current))

        return chunks or [text]


class OllamaSummariser:
    """Summariser that delegates to a locally running Ollama server/CLI."""

    def __init__(self, config: SummariserConfig | None = None) -> None:
        self.config = config or SummariserConfig()
        self._ollama_cmd = shutil.which("ollama")

    @staticmethod
    def is_available() -> bool:
        return shutil.which("ollama") is not None

    def summarise(self, text: str) -> str:
        text = (text or "").strip()
        if not text:
            return ""
        if not self._ollama_cmd:
            raise RuntimeError("ollama executable not detected")

        # Determine prompt based on content or config
        prompt_template = self.config.ollama_prompt
        # If default is used, try to auto-detect
        if prompt_template == DEFAULT_OLLAMA_PROMPT_EN:
             if any("\uac00" <= char <= "\ud7a3" for char in text[:500]):
                 prompt_template = PROMPTS["ko"]
             else:
                 prompt_template = PROMPTS["en"]

        prompt = prompt_template.format(transcript=text)
        env = os.environ.copy()
        if self.config.ollama_host:
            env["OLLAMA_HOST"] = self.config.ollama_host
        env.setdefault(
            "OLLAMA_NUM_PREDICT",
            os.getenv("MEETING_SUMMARY_OLLAMA_NUM_PREDICT")
            or os.getenv("SUMMARY_OLLAMA_NUM_PREDICT")
            or "192",
        )
        env.setdefault(
            "OLLAMA_TEMPERATURE",
            os.getenv("MEETING_SUMMARY_OLLAMA_TEMPERATURE")
            or os.getenv("SUMMARY_OLLAMA_TEMPERATURE")
            or "0.08",
        )

        try:
            result = subprocess.run(
                ["ollama", "run", self.config.ollama_model],
                input=prompt,
                capture_output=True,
                text=True,
                env=env,
                check=False,
            )
        except FileNotFoundError as exc:  # pragma: no cover - defensive
            raise RuntimeError("ollama executable not found in PATH") from exc

        if result.returncode != 0:
            stderr = (result.stderr or "").strip()
            stdout = (result.stdout or "").strip()
            error_text = stderr or stdout or "unknown error"
            raise RuntimeError(f"ollama run failed ({result.returncode}): {error_text}")

        return (result.stdout or "").strip()


class LlamaCppSummariser:
    """Summariser that runs a local GGUF via llama-cpp-python (no Ollama)."""

    def __init__(self, config: SummariserConfig | None = None) -> None:
        self.config = config or SummariserConfig()
        model_path = (self.config.llama_model or "").strip()
        if not model_path:
            raise RuntimeError("MEETING_SUMMARY_LLAMA_MODEL must point to a GGUF file")
        if not Path(model_path).expanduser().exists():
            raise RuntimeError(f"MEETING_SUMMARY_LLAMA_MODEL not found: {model_path}")

        self._use_subprocess = os.getenv("MEETING_LLAMA_CPP_SUBPROCESS", "1").strip().lower() not in {
            "",
            "0",
            "false",
            "no",
        }
        self._model_path = str(Path(model_path).expanduser())
        if self._use_subprocess:
            self._llm = None
            return

        if Llama is None:
            raise RuntimeError("llama-cpp-python is required for llama backend")
        n_ctx = max(256, int(self.config.llama_n_ctx))
        n_threads = int(self.config.llama_n_threads)
        n_gpu_layers = int(self.config.llama_gpu_layers)
        try:
            self._llm = Llama(
                model_path=model_path,
                n_ctx=n_ctx,
                n_threads=n_threads if n_threads > 0 else None,
                n_gpu_layers=n_gpu_layers,
                logits_all=False,
            )
        except Exception as exc:  # pragma: no cover - optional dependency
            raise RuntimeError(f"llama.cpp model load failed: {exc}") from exc

    @staticmethod
    def is_available() -> bool:
        return Llama is not None

    def summarise(self, text: str) -> str:
        text = (text or "").strip()
        if not text:
            return ""

        if getattr(self, "_use_subprocess", False):
            return self._summarise_auto(text)

        chunks = self._chunk_text(text, self.config.chunk_char_limit)
        parts = [self._summarise_chunk(chunk) for chunk in chunks if chunk.strip()]
        parts = [p for p in parts if p]
        if not parts:
            return ""
        if len(parts) == 1:
            return parts[0]
        combined = " ".join(parts)
        final = self._summarise_chunk(combined)
        return final or combined

    def _summarise_chunk(self, chunk: str) -> str:
        prompt_template = self.config.ollama_prompt
        if prompt_template == DEFAULT_OLLAMA_PROMPT_EN:
             if any("\uac00" <= char <= "\ud7a3" for char in chunk[:500]):
                 prompt_template = PROMPTS["ko"]
             else:
                 prompt_template = PROMPTS["en"]

        prompt = prompt_template.format(transcript=chunk)
        try:
            out = self._llm(
                prompt=prompt,
                max_tokens=max(1, int(self.config.llama_max_new_tokens)),
                temperature=0.0,
                stop=["</s>"],
            )
        except Exception as exc:  # pragma: no cover - inference guard
            LOGGER.exception("llama.cpp summarisation failed: %s", exc)
            return ""
        text = ""
        if isinstance(out, dict):
            choices = out.get("choices") or []
            if choices and isinstance(choices[0], dict):
                text = choices[0].get("text", "") or ""
        if not text:
            text = str(out)
        return text.strip()

    def _summarise_auto(self, transcript: str) -> str:
        mode = os.getenv("MEETING_LLAMA_MODE", "auto").strip().lower()
        if mode in {"auto", "direct", "llama_cpp", "llamacpp"}:
            summary = self._summarise_via_subprocess_direct(transcript)
            if summary:
                return summary
        return self._summarise_via_subprocess_cli(transcript)

    def _summarise_via_subprocess_direct(self, transcript: str) -> str:
        payload = {
            "transcript": transcript,
            "model_path": getattr(self, "_model_path", ""),
            "n_ctx": int(self.config.llama_n_ctx),
            "n_threads": int(self.config.llama_n_threads),
            "n_gpu_layers": int(self.config.llama_gpu_layers),
            "max_new_tokens": int(self.config.llama_max_new_tokens),
            "chunk_char_limit": int(self.config.chunk_char_limit),
            "prompt_template": self.config.ollama_prompt or DEFAULT_OLLAMA_PROMPT,
        }

        try:
            proc = subprocess.run(
                [sys.executable, "-m", "core.agents.meeting.llm.llama_cpp_direct_worker"],
                input=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
        except Exception as exc:  # pragma: no cover - defensive
            LOGGER.warning("llama.cpp direct worker failed to spawn: %s", exc)
            return ""

        if proc.returncode != 0:
            err = (proc.stderr or b"").decode("utf-8", "ignore").strip()
            LOGGER.warning("llama.cpp direct worker failed (code=%s): %s", proc.returncode, err[:300])
            return ""

        raw = (proc.stdout or b"").decode("utf-8", "ignore").strip()
        try:
            data = json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            LOGGER.warning("llama.cpp direct worker returned non-json output")
            return ""
        summary = str(data.get("summary") or "").strip()
        return summary

    def _summarise_via_subprocess_cli(self, transcript: str) -> str:
        payload = {
            "transcript": transcript,
            "model_path": getattr(self, "_model_path", ""),
            "n_ctx": int(self.config.llama_n_ctx),
            "n_threads": int(self.config.llama_n_threads),
            "gpu_layers": int(self.config.llama_gpu_layers),
            "max_new_tokens": int(self.config.llama_max_new_tokens),
            "chunk_char_limit": int(self.config.chunk_char_limit),
            "prompt_template": self.config.ollama_prompt or DEFAULT_OLLAMA_PROMPT,
        }
        try:
            proc = subprocess.run(
                [sys.executable, "-m", "core.agents.meeting.llm.llama_cpp_worker"],
                input=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
        except Exception as exc:  # pragma: no cover - defensive
            LOGGER.warning("llama.cpp cli worker failed to spawn: %s", exc)
            return ""

        if proc.returncode != 0:
            err = (proc.stderr or b"").decode("utf-8", "ignore").strip()
            LOGGER.warning("llama.cpp cli worker failed (code=%s): %s", proc.returncode, err[:300])
            return ""

        raw = (proc.stdout or b"").decode("utf-8", "ignore").strip()
        try:
            data = json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            LOGGER.warning("llama.cpp cli worker returned non-json output")
            return ""
        summary = str(data.get("summary") or "").strip()
        return summary

    def _chunk_text(self, text: str, limit: int) -> List[str]:
        if limit <= 0:
            return [text]

        sentences = [sentence.strip() for sentence in SENTENCE_SPLIT.split(text) if sentence.strip()]
        if not sentences:
            return [text]

        chunks: List[str] = []
        current: List[str] = []
        current_len = 0

        for sentence in sentences:
            length = len(sentence)
            if current and current_len + length > limit:
                chunks.append(" ".join(current))
                current = [sentence]
                current_len = length
            else:
                current.append(sentence)
                current_len += length

        if current:
            chunks.append(" ".join(current))

        return chunks or [text]


class BitNetSummariser:
    """Summariser for BitNet / low-bit quantised models via HuggingFace pipeline."""

    def __init__(self, config: SummariserConfig | None = None) -> None:
        self.config = config or SummariserConfig()
        self._pipeline = None

    @staticmethod
    def is_available() -> bool:
        return importlib.util.find_spec("transformers") is not None

    def _ensure_pipeline(self):
        if self._pipeline is not None:
            return self._pipeline

        try:
            from transformers import pipeline  # type: ignore
        except ImportError as exc:  # pragma: no cover - optional dependency
            raise RuntimeError("transformers is required for BitNet summarisation") from exc

        LOGGER.info("Loading BitNet summariser model: %s", self.config.bitnet_model)
        self._pipeline = pipeline(
            "summarization",
            model=self.config.bitnet_model,
            tokenizer=self.config.bitnet_model,
        )
        return self._pipeline

    def summarise(self, text: str) -> str:
        text = (text or "").strip()
        if not text:
            return ""

        pipeline = self._ensure_pipeline()
        try:
            result: List[dict] = pipeline(
                text,
                max_length=self.config.bitnet_max_length,
                min_length=self.config.bitnet_min_length,
                do_sample=False,
            )
        except Exception as exc:  # pragma: no cover - inference guard
            LOGGER.exception("BitNet summarisation failed: %s", exc)
            return ""

        if not result:
            return ""
        return (result[0].get("summary_text") or "").strip()


_SUMMARY_BACKEND_ALIASES = {
    "kobart": "kobart",
    "kobart_chunk": "kobart",
    "ollama": "ollama",
    "bitnet": "bitnet",
    "llama": "llama",
    "llamacpp": "llama",
    "local_llama": "llama",
    "local_llamacpp": "llama",
}
_SUMMARY_BACKENDS = {
    "kobart": KoBARTSummariser,
    "ollama": OllamaSummariser,
    "bitnet": BitNetSummariser,
    "llama": LlamaCppSummariser,
}


def create_summary_backend(name: str | None, config: SummariserConfig | None = None) -> Optional[BaseSummariser]:
    """Instantiate the configured summary backend, returning ``None`` for heuristics."""

    if not name:
        return None

    key = _SUMMARY_BACKEND_ALIASES.get(name.lower().strip(), name.lower().strip())
    if key in {"heuristic", "none", "placeholder"}:
        return None

    # Safety: llama-cpp-python can segfault on some setups.
    # Default to subprocess mode in `LlamaCppSummariser` and allow it without extra flags.
    # Only allow in-process llama.cpp when explicitly opted in.
    if key == "llama":
        subprocess_enabled = os.getenv("MEETING_LLAMA_CPP_SUBPROCESS", "1").strip().lower() not in {
            "",
            "0",
            "false",
            "no",
        }
        if not subprocess_enabled and os.getenv("MEETING_ALLOW_LLAMA_CPP", "0").strip() != "1":
            LOGGER.warning("llama.cpp in-process backend disabled (set MEETING_ALLOW_LLAMA_CPP=1 to enable)")
            return None
    backend_cls = _SUMMARY_BACKENDS.get(key)
    if backend_cls is None:
        LOGGER.warning("Unknown summary backend '%s'; using heuristic summary", name)
        return None

    try:
        return backend_cls(config)
    except RuntimeError as exc:  # pragma: no cover - optional dependency
        LOGGER.warning("%s summariser unavailable: %s", key, exc)
        return None


def available_summary_backends() -> Dict[str, bool]:
    """Return availability information for the known summary backends."""

    availability: Dict[str, bool] = {"heuristic": True}
    for alias, key in _SUMMARY_BACKEND_ALIASES.items():
        backend_cls = _SUMMARY_BACKENDS.get(key)
        if backend_cls is None:
            availability[alias] = False
            continue
        try:
            availability[alias] = bool(getattr(backend_cls, "is_available")())
        except Exception:  # pragma: no cover - defensive
            availability[alias] = False
    return availability
