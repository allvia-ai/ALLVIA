"""Generate a summary for a transcript using a fine-tuned model with auto language detection.

Usage (PowerShell example)::

    python scripts\pipeline\run_inference.py `
      --input "C:\\python\\github\\AI-summary\\ami_outputs\\EN2001a.Headset-0\\transcript.txt"

Options:
    --force-model <MODEL>   # 자동 언어 감지를 무시하고 특정 모델 사용

Examples (CMD)::

    python scripts\pipeline\run_inference.py --input "C:\python\github\AI-summary\ami_outputs\EN2001a.Headset-0\transcript.txt"
    python scripts\pipeline\run_inference.py --input "C:\path\to\transcript.txt" --force-model "gogamza/kobart-base-v2"
"""

import argparse
from pathlib import Path

try:
    import torch
    from transformers import AutoModelForSeq2SeqLM, AutoTokenizer
    from langdetect import detect
except ImportError:
    torch = None
    AutoModelForSeq2SeqLM = None
    AutoTokenizer = None
    detect = None

from core.config.paths import PROJECT_ROOT


# 기본 모델 매핑
DEFAULT_KO_MODEL = PROJECT_ROOT / "artifacts" / "kobart_ft" / "checkpoint-42"
MODEL_MAP = {
    "ko": str(DEFAULT_KO_MODEL),  # fine-tuned KoBART
    "en": "facebook/bart-large-cnn",  # English summarizer
}


def run_inference(model_path: str, text: str, num_beams: int = 4, max_new_tokens: int = 128) -> str:
    tokenizer = AutoTokenizer.from_pretrained(model_path)
    model = AutoModelForSeq2SeqLM.from_pretrained(model_path)

    inputs = tokenizer(
        text,
        return_tensors="pt",
        truncation=True,
        max_length=512,
    )

    with torch.no_grad():
        outputs = model.generate(
            **{k: v for k, v in inputs.items() if k != "token_type_ids"},
            max_new_tokens=max_new_tokens,
            num_beams=num_beams,
        )

    return tokenizer.decode(outputs[0], skip_special_tokens=True)


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, help="Path to the transcript file")
    parser.add_argument("--force-model", default=None, help="Force model name or path (bypass auto-detect)")
    parser.add_argument("--num-beams", type=int, default=4)
    parser.add_argument("--max-new-tokens", type=int, default=128)
    return parser


def main() -> None:
    args = _build_parser().parse_args()
    text = Path(args.input).read_text(encoding="utf-8")

    if not text.strip():
        raise ValueError(f"Input transcript is empty: {args.input}")

    # 언어 감지 & 모델 선택
    if args.force_model:
        model_path = args.force_model
        lang = "forced"
    else:
        lang = detect(text)
        model_path = MODEL_MAP.get(lang, MODEL_MAP["en"])  # 기본은 영어 모델

    print(f"\n[INFO] Detected language: {lang} → Using model: {model_path}\n")

    summary = run_inference(model_path, text, args.num_beams, args.max_new_tokens)

    print("\n=== Generated Summary ===\n")
    print(summary)


if __name__ == "__main__":
    main()
