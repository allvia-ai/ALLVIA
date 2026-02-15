"""Load transcript/summary training pairs from heterogeneous datasets."""
from __future__ import annotations

import json
import zipfile
from pathlib import Path
from typing import Iterable, List, Tuple

from core.utils import get_logger

LOGGER = get_logger("meeting.dataset_loader")


def _stringify_summary(raw_summary) -> str:
    """Normalise nested summary payloads into a single string."""

    def _collect_from_sequence(values) -> list[str]:
        collected: list[str] = []
        for item in values:
            text = _stringify_summary(item)
            if text:
                collected.append(text)
        return collected

    if raw_summary is None:
        return ""

    if isinstance(raw_summary, str):
        return raw_summary.strip()

    if isinstance(raw_summary, (list, tuple, set)):
        return " ".join(_collect_from_sequence(raw_summary)).strip()

    if isinstance(raw_summary, dict):
        preferred_keys = (
            "generated_text",
            "summary_text",
            "summary",
            "text",
        )
        structured_keys = (
            "highlights",
            "action_items",
            "decisions",
            "discussion_points",
        )

        candidates: list[str] = []

        for key in preferred_keys:
            value = raw_summary.get(key)
            if value is None:
                continue
            text = _stringify_summary(value)
            if text:
                candidates.append(text)

        for key in structured_keys:
            value = raw_summary.get(key)
            if value is None:
                continue
            text = _stringify_summary(value)
            if text:
                candidates.append(text)

        if not candidates:
            for value in raw_summary.values():
                text = _stringify_summary(value)
                if text:
                    candidates.append(text)

        return " ".join(candidates).strip()

    return str(raw_summary).strip()


def _summary_files(base_dir: Path) -> Iterable[Path]:
    seen: set[Path] = set()
    for path in base_dir.rglob("*"):
        if not path.is_file():
            continue
        if path.stem.lower() != "summary":
            continue
        resolved = path.resolve()
        if resolved in seen:
            continue
        seen.add(resolved)
        yield path


def _find_transcript(summary_file: Path) -> Path | None:
    for candidate in summary_file.parent.iterdir():
        if not candidate.is_file():
            continue
        if candidate.stem.lower() == "transcript":
            return candidate
    return None


def _load_from_transcript_pairs(base_dir: Path) -> Tuple[List[str], List[str]]:
    transcripts: list[str] = []
    summaries: list[str] = []

    for summary_file in _summary_files(base_dir):
        transcript_file = _find_transcript(summary_file)
        if transcript_file is None:
            continue

        try:
            transcript = transcript_file.read_text(encoding="utf-8").strip()
        except Exception as exc:  # pragma: no cover - defensive
            LOGGER.warning("Failed to read transcript %s: %s", transcript_file, exc)
            continue
        if not transcript:
            continue

        try:
            summary_text = summary_file.read_text(encoding="utf-8").strip()
        except Exception as exc:  # pragma: no cover - defensive
            LOGGER.warning("Failed to read summary %s: %s", summary_file, exc)
            continue
        if not summary_text:
            continue

        try:
            summary_payload = json.loads(summary_text)
        except json.JSONDecodeError:
            summary_payload = summary_text

        if isinstance(summary_payload, dict):
            raw_summary = summary_payload.get("summary", summary_payload)
        else:
            raw_summary = summary_payload

        target = _stringify_summary(raw_summary)
        if not target:
            continue

        transcripts.append(transcript)
        summaries.append(target)

    return transcripts, summaries


def _normalise_text_block(block) -> str:
    if isinstance(block, str):
        return block.strip()
    if isinstance(block, dict):
        sentence = block.get("sentence") or block.get("text")
        if isinstance(sentence, str):
            return sentence.strip()
    return ""


def _extract_document_text(entry: dict) -> str:
    text = entry.get("text")
    if isinstance(text, str):
        return text.strip()
    if isinstance(text, list):
        parts = [_normalise_text_block(item) for item in text]
        filtered = [part for part in parts if part]
        if filtered:
            return "\n".join(filtered)
    return ""


def _extract_document_summary(entry: dict) -> str:
    candidates = []
    for key in ("summary", "abstractive", "target", "scenario", "event"):
        if key not in entry:
            continue
        candidates.append(entry[key])
    for candidate in candidates:
        text = _stringify_summary(candidate)
        if text:
            return text
    return ""


def _load_from_json_archive(archive_path: Path) -> Tuple[List[str], List[str]]:
    transcripts: list[str] = []
    summaries: list[str] = []

    try:
        with zipfile.ZipFile(archive_path) as zf:
            json_members = [name for name in zf.namelist() if name.lower().endswith(".json")]
            for member in json_members:
                with zf.open(member) as fh:
                    try:
                        payload = json.load(fh)
                    except json.JSONDecodeError:
                        continue
                documents = []
                if isinstance(payload, dict):
                    documents = payload.get("documents") or []
                elif isinstance(payload, list):
                    documents = payload
                if not isinstance(documents, list):
                    continue
                for doc in documents:
                    if not isinstance(doc, dict):
                        continue
                    source = _extract_document_text(doc)
                    if not source:
                        continue
                    summary = _extract_document_summary(doc)
                    if not summary:
                        continue
                    transcripts.append(source)
                    summaries.append(summary)
    except zipfile.BadZipFile:
        LOGGER.warning("Skipping invalid archive %s", archive_path)
    except Exception as exc:  # pragma: no cover - defensive
        LOGGER.warning("Failed to load archive %s: %s", archive_path, exc)

    return transcripts, summaries


def _load_from_special_json(json_path: Path) -> Tuple[List[str], List[str]]:
    transcripts: list[str] = []
    summaries: list[str] = []

    try:
        payload = json.loads(json_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        LOGGER.warning("Failed to parse %s", json_path)
        return transcripts, summaries

    items: Iterable[dict]
    if isinstance(payload, list):
        items = payload
    elif isinstance(payload, dict) and "documents" in payload:
        maybe_docs = payload["documents"]
        items = maybe_docs if isinstance(maybe_docs, list) else []
    else:
        return transcripts, summaries

    for entry in items:
        if not isinstance(entry, dict):
            continue
        text = entry.get("text")
        if not isinstance(text, str):
            continue
        summary = entry.get("scenario") or entry.get("event") or entry.get("summary")
        if not isinstance(summary, str):
            continue
        text = text.strip()
        summary = summary.strip()
        if not text or not summary:
            continue
        transcripts.append(text)
        summaries.append(summary)

    return transcripts, summaries


def load_transcript_summary_pairs(base_dir: Path) -> Tuple[List[str], List[str]]:
    """Return parallel transcript/summary lists from heterogeneous sources."""
    base_dir = base_dir.resolve()

    transcripts: list[str] = []
    summaries: list[str] = []

    paired_transcripts, paired_summaries = _load_from_transcript_pairs(base_dir)
    transcripts.extend(paired_transcripts)
    summaries.extend(paired_summaries)

    for archive_path in base_dir.rglob("*.zip"):
        archive_transcripts, archive_summaries = _load_from_json_archive(archive_path)
        if archive_transcripts:
            LOGGER.info(
                "Loaded %s pairs from archive %s",
                len(archive_transcripts),
                archive_path,
            )
            transcripts.extend(archive_transcripts)
            summaries.extend(archive_summaries)

    for json_path in base_dir.rglob("ko_text.json"):
        json_transcripts, json_summaries = _load_from_special_json(json_path)
        if json_transcripts:
            LOGGER.info(
                "Loaded %s pairs from JSON %s",
                len(json_transcripts),
                json_path,
            )
            transcripts.extend(json_transcripts)
            summaries.extend(json_summaries)

    if not transcripts:
        raise RuntimeError(f"No transcript/summary pairs found under {base_dir}")

    return transcripts, summaries


__all__ = ["load_transcript_summary_pairs"]
