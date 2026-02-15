"""Speaker diarization module for meeting transcription."""
from __future__ import annotations

import os
from pathlib import Path
from typing import List, Optional, Tuple

from core.utils import get_logger

LOGGER = get_logger("meeting.diarization")

# Try to import pyannote
try:
    from pyannote.audio import Pipeline
    HAS_PYANNOTE = True
except ImportError:
    HAS_PYANNOTE = False
    LOGGER.warning("pyannote-audio not available. Speaker diarization disabled.")


class SpeakerDiarizer:
    """Speaker diarization using pyannote-audio."""
    
    _instance: Optional["SpeakerDiarizer"] = None
    
    def __init__(self, hf_token: Optional[str] = None):
        if not HAS_PYANNOTE:
            raise ImportError("pyannote-audio not installed. Run: pip install pyannote-audio")
        
        self.hf_token = hf_token or os.getenv("HF_TOKEN") or os.getenv("HUGGINGFACE_TOKEN")
        self.pipeline: Optional[Pipeline] = None
        
    @classmethod
    def get_instance(cls, hf_token: Optional[str] = None) -> "SpeakerDiarizer":
        """Get singleton instance."""
        if cls._instance is None:
            cls._instance = SpeakerDiarizer(hf_token)
        return cls._instance
    
    def load(self) -> None:
        """Load diarization pipeline (lazy loading)."""
        if self.pipeline is not None:
            return
            
        if not self.hf_token:
            raise ValueError(
                "HuggingFace token required for pyannote-audio. "
                "Set HF_TOKEN or HUGGINGFACE_TOKEN environment variable."
            )
        
        LOGGER.info("Loading pyannote diarization pipeline...")
        self.pipeline = Pipeline.from_pretrained(
            "pyannote/speaker-diarization-3.1",
            use_auth_token=self.hf_token
        )
        
        # Use MPS if available
        import torch
        if torch.backends.mps.is_available():
            self.pipeline.to(torch.device("mps"))
            LOGGER.info("Diarization pipeline using MPS (Metal)")
        
        LOGGER.info("Diarization pipeline loaded successfully")
    
    def diarize(self, audio_path: Path) -> List[Tuple[float, float, str]]:
        """Run speaker diarization on audio file.
        
        Args:
            audio_path: Path to audio file
            
        Returns:
            List of (start_time, end_time, speaker_id) tuples
        """
        if not HAS_PYANNOTE:
            return []
        
        try:
            self.load()
            
            LOGGER.info("Running diarization on: %s", audio_path)
            diarization = self.pipeline(str(audio_path))
            
            segments = []
            for turn, _, speaker in diarization.itertracks(yield_label=True):
                segments.append((turn.start, turn.end, speaker))
            
            LOGGER.info("Diarization complete: %d segments, %d speakers", 
                       len(segments), 
                       len(set(s[2] for s in segments)))
            return segments
        except Exception as e:
            LOGGER.error("Diarization failed: %s", e)
            return []


def assign_speakers_to_transcript(
    transcript_segments: List[dict],
    diarization_segments: List[Tuple[float, float, str]]
) -> List[dict]:
    """Assign speaker labels to transcript segments.
    
    Args:
        transcript_segments: List of {"start": float, "end": float, "text": str}
        diarization_segments: List of (start, end, speaker_id)
        
    Returns:
        Transcript segments with "speaker" field added
    """
    if not diarization_segments:
        return transcript_segments
    
    result = []
    for seg in transcript_segments:
        mid_time = (seg.get("start", 0) + seg.get("end", 0)) / 2
        
        # Find the speaker at this time
        speaker = "Unknown"
        for start, end, spk in diarization_segments:
            if start <= mid_time <= end:
                speaker = spk
                break
        
        result.append({
            **seg,
            "speaker": speaker
        })
    
    return result


def format_transcript_with_speakers(segments: List[dict]) -> str:
    """Format transcript with speaker labels.
    
    Args:
        segments: List of {"text": str, "speaker": str, ...}
        
    Returns:
        Formatted string like "[Speaker_1] Hello..."
    """
    lines = []
    current_speaker = None
    current_text = []
    
    for seg in segments:
        speaker = seg.get("speaker", "Unknown")
        text = seg.get("text", "").strip()
        
        if not text:
            continue
        
        if speaker != current_speaker:
            # Flush previous speaker's text
            if current_speaker and current_text:
                label = _format_speaker_label(current_speaker)
                lines.append(f"{label} {' '.join(current_text)}")
            current_speaker = speaker
            current_text = [text]
        else:
            current_text.append(text)
    
    # Flush remaining
    if current_speaker and current_text:
        label = _format_speaker_label(current_speaker)
        lines.append(f"{label} {' '.join(current_text)}")
    
    return "\n\n".join(lines)


def _format_speaker_label(speaker_id: str) -> str:
    """Format speaker ID as readable label."""
    # Convert "SPEAKER_00" -> "[화자 1]"
    if speaker_id.startswith("SPEAKER_"):
        try:
            num = int(speaker_id.split("_")[1]) + 1
            return f"[화자 {num}]"
        except (IndexError, ValueError):
            pass
    return f"[{speaker_id}]"


def diarize_audio(audio_path: Path, hf_token: Optional[str] = None) -> List[Tuple[float, float, str]]:
    """Convenience function to run diarization.
    
    Args:
        audio_path: Path to audio file
        hf_token: Optional HuggingFace token
        
    Returns:
        List of (start, end, speaker_id) tuples
    """
    if not HAS_PYANNOTE:
        LOGGER.warning("pyannote-audio not available, skipping diarization")
        return []
    
    try:
        diarizer = SpeakerDiarizer.get_instance(hf_token)
        return diarizer.diarize(audio_path)
    except Exception as e:
        LOGGER.error("Diarization failed: %s", e)
        return []
