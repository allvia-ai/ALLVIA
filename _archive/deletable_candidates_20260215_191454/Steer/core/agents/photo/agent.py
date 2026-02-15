"""Conversational wrapper for the photo pipeline."""
from __future__ import annotations

import os
from dataclasses import dataclass
import json
from pathlib import Path
from typing import Iterable, List, Optional

from core.agents import AgentRequest, AgentResult, ConversationalAgent
from core.agents.taskgraph import TaskCancelled
from core.config.paths import DATA_DIR
from core.utils import get_logger

from .models import PhotoJobConfig, PhotoRecommendation
from .pipeline import PhotoPipeline

LOGGER = get_logger("photo.agent")


@dataclass
class PhotoAgentConfig:
    output_root: Path = DATA_DIR / "photo_outputs"
    policy_tag: Optional[str] = None
    policy_engine: Optional[object] = None
    policy_agent: str = "photo"
    default_folder_type: str = "photos"


class PhotoAgent(ConversationalAgent):
    name = "photo_manager"
    description = "사진 폴더를 분석해 베스트샷과 중복을 정리합니다. roots가 필요합니다."

    def __init__(self, config: Optional[PhotoAgentConfig | PhotoJobConfig] = None) -> None:
        self._preset_job: Optional[PhotoJobConfig] = None
        if isinstance(config, PhotoJobConfig):
            self._preset_job = config
            self.config = PhotoAgentConfig()
        else:
            self.config = config or PhotoAgentConfig()
        self.pipeline = PhotoPipeline()

    def prepare(self) -> None:
        self.config.output_root.mkdir(parents=True, exist_ok=True)

    def run(self, request: AgentRequest) -> AgentResult:
        context = dict(request.context or {})
        progress_callback = context.pop("__progress_callback", None)
        cancel_event = context.pop("__cancel_event", None)
        policy_engine = context.get("policy_engine") or self.config.policy_engine
        roots_raw = context.get("roots")
        roots: List[Path] = []
        if roots_raw:
            roots = self._normalise_roots(roots_raw)
        else:
            roots = self._infer_roots_from_policy(policy_engine) if policy_engine else []
        if not roots:
            raise ValueError(
                "유효한 사진 폴더 경로를 찾을 수 없습니다. "
                "(/photo <폴더경로> 또는 core/config/smart_folders.json 의 photos 스코프 확인)"
            )
        output_dir = Path(context.get("output_dir") or self._default_output_root())
        output_dir.mkdir(parents=True, exist_ok=True)
        query_text = (request.query or "").strip()
        query_lower = query_text.lower()
        wants_report_only = any(token in query_text for token in ("리포트", "report", "결과", "로그")) and not any(
            token in query_text for token in ("정리", "이동", "organize", "move")
        )
        if wants_report_only:
            latest = self._latest_report_path(output_dir)
            if latest is None:
                raise ValueError("사진 리포트를 찾지 못했습니다. 먼저 사진 정리를 실행하세요. (예: /photo)")
            return AgentResult(
                content=self._format_report(latest),
                metadata={"agent": self.name, "report_path": str(latest)},
            )
        organize_mode = any(token in query_text for token in ("정리", "이동", "organize", "move")) or query_lower == "/photo"
        apply_mode = any(token in query_text for token in ("적용", "실행", "apply"))
        if apply_mode and os.getenv("PHOTO_ALLOW_APPLY", "0").strip() != "1":
            raise ValueError("사진 이동(적용) 모드는 안전을 위해 비활성화되어 있습니다. `PHOTO_ALLOW_APPLY=1`을 설정한 뒤 다시 시도하세요.")
        dest_root = context.get("dest_root")
        dest_root_path = Path(dest_root).expanduser() if dest_root else (roots[0] / "organized")
        job = PhotoJobConfig(
            roots=roots,
            output_dir=output_dir,
            policy_tag=context.get("policy_tag") or self.config.policy_tag,
            policy_engine=policy_engine,
            policy_agent=context.get("policy_agent") or self.config.policy_agent,
            prefer_gpu=bool(context.get("prefer_gpu", False)),
            organize=organize_mode,
            dry_run=not apply_mode,
            dest_root=dest_root_path,
            organize_strategy=str(context.get("strategy") or "month"),
            dedupe=bool(context.get("dedupe", False)),
        )
        LOGGER.info("photo agent running job: %s", job)
        try:
            recommendation = self.pipeline.run(
                job,
                progress_callback=progress_callback,
                cancel_event=cancel_event,
            )
        except TaskCancelled as exc:
            LOGGER.info("photo agent cancelled: %s", exc)
            raise ValueError("사진 정리가 취소되었습니다.") from exc

        events = self.pipeline.last_events()
        report_path = recommendation.report_path
        formatted = self._format_report(report_path) if report_path.exists() else self._format_recommendation(recommendation)
        return AgentResult(
            content=formatted,
            metadata={
                "agent": self.name,
                "report_path": str(recommendation.report_path),
                "best_shots": [str(asset.path) for asset in recommendation.best_shots],
                "stages": events,
            },
        )

    def _collect_files(self, job: Optional[PhotoJobConfig] = None):
        """테스트/도구용: 정책을 적용한 파일 스캔 결과를 반환합니다."""
        target = job or self._preset_job
        if target is None:
            raise ValueError("스캔할 PhotoJobConfig가 필요합니다.")
        return self.pipeline._scan(target)  # type: ignore[attr-defined]

    def _default_output_root(self) -> Path:
        env_value = os.getenv("PHOTO_OUTPUT_DIR")
        if env_value:
            return Path(env_value).expanduser()
        return self.config.output_root

    def _infer_roots_from_policy(self, policy_engine: object) -> List[Path]:
        if not getattr(policy_engine, "has_policies", False):
            return []
        roots: List[Path] = []
        try:
            roots = list(policy_engine.roots_for_type(self.config.default_folder_type, include_manual=True))
        except Exception:
            roots = []
        if roots:
            return roots
        try:
            candidates = list(policy_engine.roots_for_agent(self.config.policy_agent, include_manual=True))
        except Exception:
            candidates = []
        for root in candidates:
            name = root.name.lower()
            if "사진" in name or "photo" in name or name == "photos":
                roots.append(root)
        return roots

    @staticmethod
    def _latest_report_path(output_root: Path) -> Optional[Path]:
        if not output_root.exists():
            return None
        candidates = list(output_root.rglob("photo_report.json"))
        if not candidates:
            return None
        candidates.sort(key=lambda p: p.stat().st_mtime, reverse=True)
        return candidates[0]

    @staticmethod
    def _format_report(report_path: Path) -> str:
        try:
            payload = json.loads(report_path.read_text(encoding="utf-8"))
        except Exception:
            return f"📷 사진 리포트: {report_path}"
        lines = ["📷 사진 정리 리포트", f"- report: {report_path}"]
        organize = payload.get("organize")
        if isinstance(organize, dict):
            dry_run = bool(organize.get("dry_run", True))
            planned = organize.get("planned") or []
            applied = organize.get("applied") or []
            skipped = organize.get("skipped") or []
            lines.append(f"- mode: {'DRY-RUN' if dry_run else 'APPLY'}")
            lines.append(f"- planned: {len(planned)} | applied: {len(applied)} | skipped: {len(skipped)}")
            for item in list(applied)[:5]:
                if not isinstance(item, dict):
                    continue
                src = item.get("src")
                dst = item.get("dst")
                if src and dst:
                    lines.append(f"  - {src} → {dst}")
        else:
            best = payload.get("best_shots") or []
            dups = payload.get("duplicates") or []
            lines.append(f"- best_shots: {len(best)} | duplicates: {len(dups)}")
        return "\n".join(lines)

    @staticmethod
    def _normalise_roots(raw: Iterable[str | Path]) -> List[Path]:
        roots: List[Path] = []
        for item in raw:
            path = Path(item).expanduser()
            if path.exists():
                roots.append(path)
        return roots

    @staticmethod
    def _format_recommendation(recommendation: PhotoRecommendation) -> str:
        lines = ["📷 사진 정리 결과"]
        if recommendation.best_shots:
            lines.append("\n베스트 샷:")
            for asset in recommendation.best_shots[:10]:
                lines.append(f"- {asset.path}")
            if len(recommendation.best_shots) > 10:
                lines.append(f"... 총 {len(recommendation.best_shots)}장")
        if recommendation.duplicates:
            lines.append("\n중복 그룹:")
            for group in recommendation.duplicates[:5]:
                joined = " / ".join(str(asset.path) for asset in group[:3])
                lines.append(f"- {joined}")
        lines.append(f"\n리포트 파일: {recommendation.report_path}")
        return "\n".join(lines)
