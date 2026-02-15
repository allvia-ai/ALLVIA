"""Scaffolding for syncing action items with external tools."""
from __future__ import annotations

import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, Optional

from core.utils import get_logger

LOGGER = get_logger("meeting.integrations.sync")


@dataclass
class IntegrationConfig:
    provider: str
    config_path: Optional[Path]


def load_provider_config() -> Optional[IntegrationConfig]:
    provider = os.getenv("MEETING_INTEGRATIONS_PROVIDER")
    if not provider:
        return None
    config_path_env = os.getenv("MEETING_INTEGRATIONS_CONFIG")
    config_path = Path(config_path_env).expanduser() if config_path_env else None
    if config_path and not config_path.exists():
        LOGGER.warning("integration config not found: %s", config_path)
        config_path = None
    return IntegrationConfig(provider=provider, config_path=config_path)


def sync_action_items(
    items: Iterable[Dict[str, object]],
    config: IntegrationConfig,
    *,
    output_dir: Optional[Path] = None,
) -> None:
    """Best-effort sync stub that logs payloads for later integration."""

    payload = {
        "provider": config.provider,
        "items": list(items),
    }
    if config.config_path:
        payload["config_path"] = str(config.config_path)

    # Placeholder behaviour: write payload to a diagnostics file or log
    if output_dir is not None:
        diagnostics_dir = output_dir
    else:
        diagnostics_dir = Path(os.getenv("MEETING_INTEGRATIONS_OUT", "artifacts/integrations"))
    diagnostics_dir.mkdir(parents=True, exist_ok=True)
    if config.provider.lower() == "local":
        output_path = diagnostics_dir / "action_items.json"
        output_path.write_text(json.dumps(payload["items"], ensure_ascii=False, indent=2), encoding="utf-8")
        LOGGER.info("action items saved locally: %s", output_path)
    else:
        output_path = diagnostics_dir / f"sync_{config.provider}.json"
        output_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        LOGGER.info(
            "queued integration sync: provider=%s items=%s output=%s",
            config.provider,
            len(payload["items"]),
            output_path,
        )
