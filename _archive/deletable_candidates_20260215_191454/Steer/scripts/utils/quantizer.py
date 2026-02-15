"""SentenceTransformer → ONNX(+optional int8) exporter."""

from __future__ import annotations

import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Optional

import torch
from transformers import AutoModel, AutoTokenizer

try:
    from onnxruntime.quantization import QuantType, quantize_dynamic
except Exception:  # pragma: no cover - optional dependency
    QuantType = None  # type: ignore[assignment]
    quantize_dynamic = None  # type: ignore[assignment]


@dataclass
class QuantizeResult:
    source: str
    output: Path
    quantized: bool
    opset: int
    sequence_length: int
    file_size: int


def _prepare_inputs(tokenizer, sequence_length: int) -> Dict[str, torch.Tensor]:
    dummy = tokenizer(
        "AI-summary document embedding export",
        return_tensors="pt",
        max_length=sequence_length,
        padding="max_length",
        truncation=True,
    )
    return dummy


def export_to_onnx(
    model_name_or_path: str,
    *,
    output_path: Path,
    sequence_length: int = 384,
    opset: int = 17,
    quantize_int8: bool = True,
) -> QuantizeResult:
    """Export a HuggingFace encoder to ONNX (+optional int8 quantisation)."""

    model = AutoModel.from_pretrained(model_name_or_path)
    tokenizer = AutoTokenizer.from_pretrained(model_name_or_path)
    model.eval()

    dummy_inputs = _prepare_inputs(tokenizer, sequence_length)
    input_names = ["input_ids", "attention_mask"]
    inputs = (dummy_inputs["input_ids"], dummy_inputs["attention_mask"])
    dynamic_axes = {
        "input_ids": {0: "batch", 1: "sequence"},
        "attention_mask": {0: "batch", 1: "sequence"},
        "last_hidden_state": {0: "batch", 1: "sequence"},
    }
    if "token_type_ids" in dummy_inputs:
        input_names.append("token_type_ids")
        inputs = (*inputs, dummy_inputs["token_type_ids"])
        dynamic_axes["token_type_ids"] = {0: "batch", 1: "sequence"}

    output_path.parent.mkdir(parents=True, exist_ok=True)
    torch.onnx.export(
        model,
        inputs,
        str(output_path),
        input_names=input_names,
        output_names=["last_hidden_state"],
        dynamic_axes=dynamic_axes,
        opset_version=opset,
    )

    did_quantize = False
    if quantize_int8 and quantize_dynamic and QuantType is not None:
        tmp_out = output_path.with_suffix(".tmp.onnx")
        try:
            quantize_dynamic(
                str(output_path),
                str(tmp_out),
                weight_type=QuantType.QInt8,
                optimize_model=True,
            )
            shutil.move(str(tmp_out), str(output_path))
            did_quantize = True
        finally:
            if tmp_out.exists():
                try:
                    tmp_out.unlink()
                except FileNotFoundError:
                    pass
    elif quantize_int8:
        print("⚠️ onnxruntime.quantization 모듈을 찾지 못해 FP32 ONNX만 생성합니다.")

    size = output_path.stat().st_size if output_path.exists() else 0
    return QuantizeResult(
        source=model_name_or_path,
        output=output_path,
        quantized=did_quantize,
        opset=opset,
        sequence_length=sequence_length,
        file_size=size,
    )
