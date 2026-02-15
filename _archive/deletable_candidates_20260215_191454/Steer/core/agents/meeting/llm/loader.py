"""On-device model loader scaffolding for meeting agent."""
from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

from core.utils import get_logger

LOGGER = get_logger("meeting.llm.loader")


@dataclass
class OnDeviceModelConfig:
    """Configuration describing an on-device LLM candidate."""

    name: str
    path: Path
    device: str = "cpu"
    mmap: bool = True

    @classmethod
    def from_env(cls) -> Optional["OnDeviceModelConfig"]:
        path_env = os.getenv("MEETING_ONDEVICE_MODEL_PATH")
        if not path_env:
            return None
        path = Path(path_env).expanduser()
        if not path.exists():
            LOGGER.warning("on-device model path not found: %s", path)
            return None
        name = os.getenv("MEETING_ONDEVICE_MODEL_NAME", path.stem)
        device = os.getenv("MEETING_ONDEVICE_DEVICE", "cpu")
        mmap = os.getenv("MEETING_ONDEVICE_MMAP", "1").strip().lower() not in {"0", "false", "no"}
        return cls(name=name, path=path, device=device, mmap=mmap)


class OnDeviceModel:
    """Placeholder runtime handle for on-device models."""

    def __init__(self, config: OnDeviceModelConfig) -> None:
        self.config = config

    def unload(self) -> None:
        LOGGER.info("unloading on-device model: %s", self.config.name)


class OnDeviceModelLoader:
    """Best-effort loader that records configuration for later use."""

    def __init__(self, config: Optional[OnDeviceModelConfig]) -> None:
        self._config = config
        self._model: Optional[OnDeviceModel] = None

    @classmethod
    def from_env(cls) -> "OnDeviceModelLoader":
        return cls(OnDeviceModelConfig.from_env())

    def is_configured(self) -> bool:
        return self._config is not None

    def load(self) -> Optional[OnDeviceModel]:
        if self._model is not None:
            return self._model
        if self._config is None:
            LOGGER.debug("on-device model not configured; skipping load")
            return None
        LOGGER.info(
            "loading on-device model: name=%s path=%s device=%s mmap=%s",
            self._config.name,
            self._config.path,
            self._config.device,
            self._config.mmap,
        )
        self._model = OnDeviceModel(self._config)
        return self._model

    def unload(self) -> None:
        if self._model is None:
            return
        self._model.unload()
        self._model = None

    def get_config(self) -> Optional[OnDeviceModelConfig]:
        return self._config
