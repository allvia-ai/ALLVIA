"""Async embedding helpers built on top of SentenceTransformer."""

from __future__ import annotations

import asyncio
from functools import partial
from typing import Iterable, List, Sequence

import numpy as np


class AsyncSentenceEmbedder:
    """Runs SentenceTransformer.encode inside an asyncio pipeline."""

    def __init__(
        self,
        encoder,
        batch_size: int = 32,
        concurrency: int = 1,
        *,
        target_dtype: str = "auto",
        device: str | None = None,
    ):
        self.encoder = encoder
        self.batch_size = max(1, int(batch_size))
        self.concurrency = max(1, int(concurrency))
        self.device = device
        self._device_label = self._normalize_device(device)
        dim_getter = getattr(self.encoder, "get_sentence_embedding_dimension", None)
        self.embedding_dim = int(dim_getter()) if callable(dim_getter) else 0
        if target_dtype not in {"auto", "fp32", "fp16"}:
            target_dtype = "auto"
        self.target_dtype = target_dtype
        self._onnx_backend = self._detect_onnx_backend()

    async def _encode_batch(self, batch: Sequence[str]) -> np.ndarray:
        loop = asyncio.get_running_loop()
        func = partial(
            self.encoder.encode,
            list(batch),
            batch_size=min(self.batch_size, max(1, len(batch))),
            show_progress_bar=False,
            convert_to_numpy=True,
            normalize_embeddings=False,
            device=self.device,
        )
        result = await loop.run_in_executor(None, func)
        return result.astype(self._numpy_dtype(), copy=False)

    async def _runner(self, texts: List[str]) -> np.ndarray:
        pending = []
        results: List[np.ndarray] = []
        for start in range(0, len(texts), self.batch_size):
            batch = texts[start : start + self.batch_size]
            task = asyncio.create_task(self._encode_batch(batch))
            pending.append(task)
            if len(pending) >= self.concurrency:
                completed, pending = await asyncio.wait(pending, return_when=asyncio.FIRST_COMPLETED)
                for item in completed:
                    results.append(item.result())
        if pending:
            completed, _ = await asyncio.wait(pending)
            for item in completed:
                results.append(item.result())
        if not results:
            return np.zeros((0, self.embedding_dim), dtype=self._numpy_dtype())
        stacked = np.vstack(results)
        return stacked.astype(self._numpy_dtype(), copy=False)

    def encode(self, texts: Iterable[str]) -> np.ndarray:
        text_list = [str(t or "") for t in texts]
        if not text_list:
            return np.zeros((0, self.embedding_dim), dtype=self._numpy_dtype())
        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            loop = None
        if loop and loop.is_running():
            return self._encode_direct(text_list)
        return asyncio.run(self._runner(text_list))

    def _encode_direct(self, texts: Sequence[str]) -> np.ndarray:
        """Fallback for environments with a running event loop."""
        result = self.encoder.encode(
            list(texts),
            batch_size=min(self.batch_size, max(1, len(texts))),
            show_progress_bar=False,
            convert_to_numpy=True,
            normalize_embeddings=False,
            device=self.device,
        )
        return np.asarray(result, dtype=self._numpy_dtype(), copy=False)

    def _numpy_dtype(self):
        if self.target_dtype == "fp16":
            return np.float16
        if self.target_dtype == "fp32":
            return np.float32
        return np.float16 if self._should_use_fp16() else np.float32

    def _should_use_fp16(self) -> bool:
        if self.target_dtype == "fp16":
            return True
        if self.target_dtype == "fp32":
            return False
        if self._onnx_backend:
            return False
        if self._device_label and not self._device_label.startswith("cuda"):
            return False
        try:
            import torch
        except Exception:
            return False
        if not torch.cuda.is_available():
            return False
        device_index = self._cuda_device_index()
        if device_index >= torch.cuda.device_count():
            device_index = 0
        try:
            props = torch.cuda.get_device_properties(device_index)
        except Exception:
            return False
        # 기본적으로 FP16 연산이 가능한 GPU(CUDA capability >= 7.0)를 가정
        return getattr(props, "major", 0) >= 7

    def _cuda_device_index(self) -> int:
        if not self._device_label.startswith("cuda"):
            return 0
        parts = self._device_label.split(":", 1)
        if len(parts) == 2:
            try:
                return max(0, int(parts[1]))
            except ValueError:
                return 0
        return 0

    def _normalize_device(self, device) -> str:
        if device is None:
            return ""
        try:
            return str(device).strip().lower()
        except Exception:
            return ""

    def _detect_onnx_backend(self) -> bool:
        encoder = self.encoder
        if encoder is None:
            return False
        try:
            cls = encoder.__class__
        except Exception:
            return False
        name = getattr(cls, "__name__", "")
        module = getattr(cls, "__module__", "")
        lowered = f"{module}.{name}".lower()
        if "onnx" in lowered:
            return True
        # onnxruntime.InferenceSession exposes providers attribute
        if hasattr(encoder, "providers"):
            return True
        if hasattr(encoder, "onnx_model_path") or hasattr(encoder, "onnx_sessions"):
            return True
        return False
