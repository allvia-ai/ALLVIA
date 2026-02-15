from __future__ import annotations

import sys

import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

from typing import Dict, Optional
from datetime import datetime

from core.policy.engine import PolicyEngine
from core.config.paths import PROJECT_ROOT, DATA_DIR
from .pipeline import MeetingPipeline, LOGGER, TaskCancelled
from .models import MeetingJobConfig, MeetingSummary
import re
from core.agents.base import ConversationalAgent, AgentRequest, AgentResult
from core.tasks.store import TaskStore, Task


@dataclass
class MeetingAgentConfig:
    """Runtime configuration for the meeting assistant."""

    output_root: Path = DATA_DIR / "ami_outputs"
    language: str = "ko"
    policy_tag: Optional[str] = None
    # Policy loading
    policy_path: Path = PROJECT_ROOT / "core" / "config" / "smart_folders.json"


class MeetingAgent(ConversationalAgent):
    """Wraps ``MeetingPipeline`` to integrate with the orchestrator."""

    name = "meeting_summary"
    description = "회의 오디오를 전사하고 요약합니다. audio_path가 필요합니다."

    def __init__(
        self,
        config: Optional[MeetingAgentConfig] = None,
        policy_engine: Optional[PolicyEngine] = None,
    ) -> None:
        self.config = config or MeetingAgentConfig()
        self.pipeline: Optional[MeetingPipeline] = None
        self._policy_engine: Optional[PolicyEngine] = policy_engine

    def prepare(self) -> None:
        self.config.output_root.mkdir(parents=True, exist_ok=True)
        # Load policies
        if self.config.policy_path and self.config.policy_path.exists():
            self._policy_engine = PolicyEngine.from_file(self.config.policy_path)
            LOGGER.info("MeetingAgent loaded policies from %s", self.config.policy_path)
        else:
            LOGGER.warning("MeetingAgent could not find policy file at %s", self.config.policy_path)

    def run(self, request: AgentRequest) -> AgentResult:
        if self.pipeline is None:
            self.pipeline = MeetingPipeline()
        
        # Ensure policy engine is loaded if prepare() wasn't called explicitly
        if self._policy_engine is None and self.config.policy_path.exists():
             self._policy_engine = PolicyEngine.from_file(self.config.policy_path)

        context = dict(request.context or {})
        progress_callback = context.pop("__progress_callback", None)
        cancel_event = context.pop("__cancel_event", None)
        audio_path_raw = context.get("audio_path")
        if not audio_path_raw:
            raise ValueError("회의 요약을 실행하려면 audio_path(오디오 파일 경로)가 필요합니다.")
        audio_path = Path(audio_path_raw).expanduser()
        if not audio_path.exists():
            raise ValueError(f"오디오 파일을 찾을 수 없습니다: {audio_path}")

        # Policy Check
        if self._policy_engine:
            allowed, reason = self._policy_engine.check(audio_path, agent="meeting")
            if not allowed:
                 raise PermissionError(f"정책에 의해 접근이 거부되었습니다 ({reason}): {audio_path}")

        output_root = Path(context.get("output_dir") or self._default_output_root())
        
        # Enforce YYYY-MM-DD_TITLE format
        date_str = datetime.now().strftime("%Y-%m-%d")
        safe_title = "".join(c for c in audio_path.stem if c.isalnum() or c in (' ', '_', '-')).strip().replace(' ', '_')
        output_dir = output_root / f"{date_str}_{safe_title}"
        language = context.get("language") or self.config.language
        context_dirs = [Path(p).expanduser() for p in context.get("context_dirs", [])]
        enable_resume = bool(context.get("enable_resume", False))
        job = MeetingJobConfig(
            audio_path=audio_path,
            output_dir=output_dir,
            language=language,
            policy_tag=context.get("policy_tag") or self.config.policy_tag,
            context_dirs=context_dirs,
            enable_resume=enable_resume,
            speaker_count=context.get("speaker_count"),
        )
        LOGGER.info("meeting agent running job: %s", job)
        try:
            summary = self.pipeline.run(
                job,
                progress_callback=progress_callback,
                cancel_event=cancel_event,
            )
            
            # Auto-save extracted tasks
            if summary.action_items_structured:
                self._save_tasks(summary.action_items_structured, audio_path.stem)
                
        except TaskCancelled as exc:
            LOGGER.info("meeting agent cancelled: %s", exc)
            raise ValueError("회의 요약이 취소되었습니다.") from exc

        events = self.pipeline.last_events()
        return AgentResult(
            content=self._format_summary(summary),
            metadata={
                "agent": self.name,
                "output_dir": str(output_dir),
                "transcript_path": str(summary.transcript_path),
                "language": summary.structured_summary.get("language"),
                "stages": events,
            },
        )

    def _default_output_root(self) -> Path:
        env_value = os.getenv("MEETING_OUTPUT_DIR")
        if env_value:
            return Path(env_value).expanduser()
        return self.config.output_root

    @staticmethod
    def _format_summary(summary: MeetingSummary) -> str:
        lines = ["🗂️ 회의 요약"]
        if summary.highlights:
            lines.append("\n핵심 요약:")
            for item in summary.highlights:
                lines.append(f"- {item}")
        if summary.decisions:
            lines.append("\n결정 사항:")
            for item in summary.decisions:
                lines.append(f"- {item}")
        if summary.action_items:
            lines.append("\n액션 아이템:")
            for item in summary.action_items:
                lines.append(f"- {item}")
        lines.append(f"\n원문 전체 요약:\n{summary.raw_summary.strip()}")
        lines.append(f"\n전사 파일: {summary.transcript_path}")
        return "\n".join(lines)

    def _save_tasks(self, action_items: list[dict], meeting_id: str) -> None:
        """Parse and save action items to the Task Store."""
        try:
            store = TaskStore()
            for item in action_items:
                text = item.get("text", "").strip()
                if not text:
                    continue
                
                # Heuristic parsing: [Owner] Task (Due: Date)
                owner = None
                due_date = None
                
                # Extract Owner: [David] ... or Action: [David] ...
                # Look for brackets near the start
                owner_match = re.search(r"^[(\w+\s*:)?\s*]*\[(.*?)\].*", text)
                if owner_match:
                    owner = owner_match.group(1).strip()
                    # Remove owner tag from text if desired, or keep it.
                    # text = text[owner_match.end():].strip()
                
                # Extract Due Date: ... (Due: YYYY-MM-DD)
                due_match = re.search(r"\(Due:\s*(.*?)\)", text, re.IGNORECASE)
                if due_match:
                    due_date = due_match.group(1).strip()
                
                task = Task(
                    content=text,
                    source_meeting_id=meeting_id,
                    owner=owner,
                    due_date=due_date
                )
                store.add_task(task)
            
            count = len(action_items)
            LOGGER.info("Saved %d tasks for meeting %s", count, meeting_id)
            
            if count > 0:
                self._send_notification(
                    "Action Items Extracted", 
                    f"{count} new tasks saved from meeting."
                )

        except Exception as exc:
            LOGGER.warning("Failed to auto-save tasks: %s", exc)

    def _send_notification(self, title: str, message: str) -> None:
        """Send MacOS notification."""
        if sys.platform != "darwin":
            return
        try:
            script = f'display notification "{message}" with title "{title}"'
            subprocess.run(["osascript", "-e", script], check=False)
        except Exception as e:
            LOGGER.debug("Failed to send notification: %s", e)


