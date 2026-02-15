"""Convert STT JSONL metadata into a manifest CSV the training stack can consume."""

from __future__ import annotations

import argparse
import csv
import json
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator, Optional

try:  # Optional but recommended for duration stats.
    import soundfile as sf  # type: ignore
except Exception:  # pragma: no cover - optional dependency
    sf = None  # type: ignore[assignment]


@dataclass
class ManifestRow:
    audio_path: Path
    text: str
    duration_seconds: Optional[float]


def read_jsonl(path: Path) -> Iterator[dict]:
    with path.open("r", encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                yield json.loads(line)
            except json.JSONDecodeError:
                continue


def resolve_audio_path(entry: dict, audio_root: Path) -> Optional[Path]:
    audio_field = entry.get("audio") or entry.get("audio_filepath") or entry.get("wav")
    if not audio_field:
        return None
    raw_path = Path(audio_field)
    candidates = []
    if raw_path.is_absolute():
        candidates.append(raw_path)
    else:
        candidates.append(audio_root / raw_path)
    if raw_path.name and raw_path.name != str(raw_path):
        candidates.append(audio_root / raw_path.name)
    for candidate in candidates:
        resolved = candidate.expanduser().resolve()
        if resolved.exists():
            return resolved
    return None


def compute_duration(audio_path: Path) -> Optional[float]:
    if sf is None:
        return None
    try:
        with sf.SoundFile(str(audio_path)) as handle:  # type: ignore[arg-type]
            frames = handle.frames
            rate = handle.samplerate or 0
            return round(frames / float(rate), 3) if rate else None
    except Exception:
        return None


def build_manifest(jsonl_path: Path, audio_root: Path, *, limit: int, skip_duration: bool) -> list[ManifestRow]:
    rows: list[ManifestRow] = []
    for entry in read_jsonl(jsonl_path):
        text = entry.get("text") or entry.get("transcript") or entry.get("sentence")
        if not isinstance(text, str):
            continue
        audio_path = resolve_audio_path(entry, audio_root)
        if audio_path is None or not audio_path.exists():
            continue
        duration = None if skip_duration else compute_duration(audio_path)
        rows.append(ManifestRow(audio_path=audio_path, text=text.strip(), duration_seconds=duration))
        if limit and len(rows) >= limit:
            break
    return rows


def write_manifest(rows: list[ManifestRow], output_path: Path) -> None:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    with output_path.open("w", encoding="utf-8", newline="") as csvfile:
        writer = csv.writer(csvfile)
        writer.writerow(["audio_path", "text", "duration_seconds"])
        for row in rows:
            writer.writerow([str(row.audio_path), row.text, row.duration_seconds or ""])


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jsonl-path", required=True, help="원본 script.jsonl 경로")
    parser.add_argument(
        "--audio-root",
        help="JSONL에 기록된 상대 경로의 기준 디렉터리 (기본: JSONL이 위치한 디렉터리)",
    )
    parser.add_argument(
        "--output",
        default="data/stt_manifest.csv",
        help="생성할 manifest CSV 경로",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=0,
        help="상위 N건만 변환 (0이면 전체)",
    )
    parser.add_argument(
        "--skip-duration",
        action="store_true",
        help="duration_seconds 계산을 건너뜁니다(soundfile 미설치 시 유용)",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    jsonl_path = Path(args.jsonl_path).expanduser()
    if not jsonl_path.exists():
        raise SystemExit(f"JSONL 경로를 찾을 수 없습니다: {jsonl_path}")

    audio_root = Path(args.audio_root).expanduser() if args.audio_root else jsonl_path.parent
    rows = build_manifest(
        jsonl_path=jsonl_path,
        audio_root=audio_root,
        limit=max(0, int(args.limit)),
        skip_duration=bool(args.skip_duration),
    )
    if not rows:
        raise SystemExit("변환 가능한 항목을 찾지 못했습니다. JSONL 내용을 확인하세요.")

    output_path = Path(args.output).expanduser()
    write_manifest(rows, output_path)

    print(f"✅ manifest 저장 완료: {output_path} (총 {len(rows)}건) ", end="")
    missing = "duration 미포함" if args.skip_duration or sf is None else "duration 포함"
    print(f"[{missing}]")


if __name__ == "__main__":
    main()
