"""Context adapter that augments meeting summaries with auxiliary documents."""
from __future__ import annotations

import json
import logging
import os
import shutil
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, List, Optional

LOGGER = logging.getLogger(__name__)

SUPPORTED_EXTENSIONS = {".txt", ".md", ".json"}


def _ensure_list(path_value: Optional[str]) -> List[Path]:
    if not path_value:
        return []
    paths: List[Path] = []
    for raw in path_value.split(os.pathsep):
        candidate = Path(raw).expanduser()
        if candidate.exists():
            paths.append(candidate)
        else:
            LOGGER.debug("context directory ignored (missing): %s", candidate)
    return paths


@dataclass
class ContextDocument:
    """Metadata describing a collected context document."""

    source: Path
    target_name: str
    kind: str
    preview: Optional[str] = None


@dataclass
class ContextBundle:
    """Container returned by the context adapter."""

    summary_prompt: Optional[str]
    documents: List[ContextDocument] = field(default_factory=list)


class MeetingContextAdapter:
    """Collects meeting context files and prepares prompt augmentations."""

    def __init__(
        self,
        *,
        pre_dirs: Iterable[Path] | None = None,
        post_dirs: Iterable[Path] | None = None,
        max_preview_chars: int = 1200,
    ) -> None:
        env_pre = _ensure_list(os.getenv("MEETING_CONTEXT_PRE_DIR"))
        env_post = _ensure_list(os.getenv("MEETING_CONTEXT_POST_DIR"))

        self._pre_dirs: List[Path] = list(pre_dirs or []) + env_pre
        self._post_dirs: List[Path] = list(post_dirs or []) + env_post
        self._max_preview_chars = max_preview_chars

    def collect(
        self,
        *,
        job_audio: Path,
        output_dir: Path,
        extra_dirs: Iterable[Path] | None = None,
        allowed_roots: Iterable[Path] | None = None,
    ) -> ContextBundle:
        """Collect context documents and copy them into the meeting output."""

        attachments_dir = output_dir / "attachments"
        attachments_dir.mkdir(parents=True, exist_ok=True)

        normalized_allowed: List[Path] = []
        if allowed_roots:
            normalized_allowed = [
                root.expanduser().resolve(strict=False) for root in allowed_roots
            ]

        directories: List[Path] = []
        directories.extend(self._pre_dirs)
        directories.extend(self._post_dirs)
        if extra_dirs:
            for path in extra_dirs:
                if path.exists():
                    directories.append(path)

        # Allow co-located documents (same directory as the audio file)
        audio_dir = job_audio.parent
        if audio_dir not in directories and audio_dir.exists():
            directories.append(audio_dir)

        validated_directories: List[Path] = []
        disallowed: List[Path] = []
        for directory in directories:
            candidate = directory.expanduser().resolve(strict=False)
            if normalized_allowed and not any(
                candidate == root or candidate.is_relative_to(root)
                for root in normalized_allowed
            ):
                disallowed.append(candidate)
                continue
            validated_directories.append(candidate)

        if disallowed:
            raise PermissionError(
                "context directory outside allowed scope: "
                + ", ".join(str(path) for path in disallowed)
            )

        collected: List[ContextDocument] = []
        for directory in validated_directories:
            collected.extend(self._scan_directory(directory, attachments_dir, job_audio.stem))

        if not collected:
            return ContextBundle(summary_prompt=None, documents=[])

        prompt_sections: List[str] = []
        for doc in collected:
            preview = (doc.preview or "").strip()
            if not preview:
                continue
            prompt_sections.append(f"[{doc.kind.upper()}] {doc.target_name}\n{preview}")

        prompt_text = "\n\n".join(prompt_sections) if prompt_sections else None
        return ContextBundle(summary_prompt=prompt_text, documents=collected)

    # ------------------------------------------------------------------
    # Helpers
    # ------------------------------------------------------------------
    def _scan_directory(
        self,
        directory: Path,
        attachments_dir: Path,
        stem: str,
    ) -> List[ContextDocument]:
        documents: List[ContextDocument] = []
        if not directory.exists():
            return documents

        for path in sorted(directory.glob("*")):
            if not path.is_file():
                continue
            if path.suffix.lower() not in SUPPORTED_EXTENSIONS:
                continue
            if stem and stem not in path.name and not path.name.startswith(stem):
                # Allow generic context files by name pattern
                if not any(keyword in path.name.lower() for keyword in ("agenda", "brief", "notes", "context")):
                    continue
            target = attachments_dir / path.name
            try:
                shutil.copy2(path, target)
            except Exception as exc:  # pragma: no cover - IO guard
                LOGGER.warning("failed to copy context document %s: %s", path, exc)
                continue

            documents.append(
                ContextDocument(
                    source=target,
                    target_name=target.name,
                    kind=self._classify_document(path.name),
                    preview=self._render_preview(target),
                )
            )

        return documents

    def _classify_document(self, name: str) -> str:
        lowered = name.lower()
        if "agenda" in lowered or "brief" in lowered:
            return "pre"
        if "notes" in lowered or "summary" in lowered:
            return "post"
        return "context"

    def _render_preview(self, path: Path) -> Optional[str]:
        try:
            text = path.read_text(encoding="utf-8")
        except Exception:  # pragma: no cover - binary or unreadable
            return None

        if path.suffix.lower() == ".json":
            try:
                parsed = json.loads(text)
                text = json.dumps(parsed, ensure_ascii=False, indent=2)
            except Exception:  # pragma: no cover - json fallback
                pass

        text = text.strip()
        if len(text) > self._max_preview_chars:
            text = text[: self._max_preview_chars].rstrip() + "…"
        return text
