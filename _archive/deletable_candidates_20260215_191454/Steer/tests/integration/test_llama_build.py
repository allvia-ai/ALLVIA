import os

import pytest

llama_cpp = pytest.importorskip("llama_cpp")

DEFAULT_MODEL_PATH = "models/gguf/tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf"
MODEL_PATH = os.path.expanduser(os.getenv("LLAMA_GGUF_PATH", DEFAULT_MODEL_PATH))


@pytest.mark.integration
@pytest.mark.skipif(
    not os.path.exists(MODEL_PATH),
    reason="LLM model file not found; set LLAMA_GGUF_PATH or place gguf in models/gguf",
)
def test_llama_build_generates_basic_text():
    """Integration guardrail: confirm llama.cpp build can load and generate."""
    llm = llama_cpp.Llama(
        model_path=MODEL_PATH,
        n_gpu_layers=-1,
        n_ctx=512,
        verbose=False,
    )

    prompt = (
        "<|system|>\nYou are a concise assistant.\n</s>\n"
        "<|user|>\nSay hello in one short sentence.\n</s>\n"
        "<|assistant|>\n"
    )
    result = llm(prompt, max_tokens=32, stop=["</s>"], echo=False)

    # ensure at least one token was generated and text is non-empty
    assert "choices" in result and result["choices"], "no generation choices returned"
    text = result["choices"][0].get("text", "").strip()
    assert text, "empty generation from llama.cpp"
