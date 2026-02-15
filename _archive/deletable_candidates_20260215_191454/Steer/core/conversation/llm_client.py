from __future__ import annotations

import json
import os
import socket
import subprocess
import shlex
from dataclasses import dataclass, field
from pathlib import Path
import sys
from typing import Any, Dict, List, Optional
from urllib import error, request

try:
    import torch
except Exception:  # pragma: no cover - optional dependency
    torch = None

try:
    from transformers import AutoModelForCausalLM, AutoTokenizer, pipeline
except Exception:  # pragma: no cover - optional dependency
    AutoModelForCausalLM = None
    AutoTokenizer = None
    pipeline = None

try:
    from llama_cpp import Llama
except Exception:  # pragma: no cover - optional dependency
    Llama = None

class LLMClientError(RuntimeError):
    """Raised when the local LLM backend fails."""


@dataclass
class LLMClient:
    """Abstract base for lightweight local LLM clients."""

    def is_available(self) -> bool:
        raise NotImplementedError

    def generate(self, prompt: str, *, system: Optional[str] = None, timeout: float = 30.0) -> str:
        raise NotImplementedError

    def generate_chat(self, messages: List[Dict[str, str]], *, timeout: float = 30.0) -> str:
        """Generate response from a list of messages for multi-turn conversation.
        
        messages: List of dicts with 'role' and 'content' keys.
                  role can be 'system', 'user', or 'assistant'
        """
        raise NotImplementedError


@dataclass
class OllamaClient(LLMClient):
    """Executes prompts against an Ollama daemon via its HTTP API."""

    model: str = "llama3"
    host: str = ""
    options: Dict[str, str] = field(default_factory=dict)
    _context: Optional[List[int]] = field(init=False, default=None)

    def _resolve_base_url(self) -> str:
        host = (self.host or os.getenv("OLLAMA_HOST") or "127.0.0.1:11434").strip()
        if not host:
            host = "127.0.0.1:11434"
        if host.startswith(("http://", "https://")):
            base = host
        else:
            base = f"http://{host}"
        return base.rstrip("/")

    def is_available(self) -> bool:
        base = self._resolve_base_url()
        try:
            with request.urlopen(f"{base}/api/tags", timeout=2.0) as resp:
                code = getattr(resp, "status", None)
                if code is None:
                    code = resp.getcode()
                return 200 <= int(code) < 400
        except Exception:
            return False

    def generate(self, prompt: str, *, system: Optional[str] = None, timeout: float = 30.0) -> str:
        base = self._resolve_base_url()
        full_prompt = prompt if not system else f"{system.strip()}\n\n{prompt.strip()}"
        payload: Dict[str, Any] = {
            "model": self.model,
            "prompt": full_prompt,
            "stream": False,
        }
        if self._context:
            payload["context"] = self._context

        options = self._prepare_options(system)
        if options:
            payload["options"] = options

        data = json.dumps(payload).encode("utf-8")
        req = request.Request(
            f"{base}/api/generate",
            data=data,
            headers={"Content-Type": "application/json"},
            method="POST",
        )

        try:
            with request.urlopen(req, timeout=timeout) as resp:
                body = resp.read().decode("utf-8", "ignore")
        except socket.timeout as exc:
            raise LLMClientError(f"ollama request timed out after {timeout}s") from exc
        except error.HTTPError as exc:
            detail = exc.read().decode("utf-8", "ignore") if hasattr(exc, "read") else ""
            message = detail.strip() or str(exc)
            raise LLMClientError(f"ollama request failed ({exc.code}): {message}") from exc
        except error.URLError as exc:
            raise LLMClientError(f"ollama connection failed: {exc.reason}") from exc

        try:
            payload = json.loads(body)
        except json.JSONDecodeError as exc:
            raise LLMClientError("ollama response decoding failed") from exc

        if not isinstance(payload, dict):
            raise LLMClientError("ollama response was not a JSON object")
        if "error" in payload:
            raise LLMClientError(f"ollama returned an error: {payload['error']}")

        context = payload.get("context")
        if isinstance(context, list):
            self._context = context
        text = payload.get("response", "")
        if not isinstance(text, str):
            text = ""
        return text.strip()

    def _prepare_options(self, system: Optional[str]) -> Dict[str, Any]:
        options: Dict[str, Any] = {}
        for key, value in (self.options or {}).items():
            if key.lower() == "api_key":
                continue
            options[key] = self._coerce_option_value(value)

        env_default = os.getenv("LNPCHAT_OLLAMA_NUM_PREDICT")
        if env_default:
            options.setdefault("num_predict", self._coerce_option_value(env_default))
        else:
            options.setdefault("num_predict", 512)

        if system and "health check" in system.lower():
            try:
                max_tokens = int(options.get("num_predict", 64))
            except (TypeError, ValueError):
                max_tokens = 64
            options["num_predict"] = max(1, min(64, max_tokens))
            options.setdefault("temperature", 0.0)

        return options

    @staticmethod
    def _coerce_option_value(value: Any) -> Any:
        if isinstance(value, (bool, int, float)):
            return value
        if isinstance(value, str):
            raw = value.strip()
            if not raw:
                return raw
            lowered = raw.lower()
            if lowered in {"true", "false"}:
                return lowered == "true"
            try:
                if "." in raw:
                    return float(raw)
                return int(raw)
            except ValueError:
                return raw
        return value


@dataclass
class LocalGemmaClient(LLMClient):
    """Loads a local Gemma (or any HF causal LM) without running a server."""

    model: str
    device: str = "auto"
    torch_dtype: str = "auto"
    max_new_tokens: int = 512
    _pipe: Any = field(init=False, default=None)

    def __post_init__(self) -> None:
        if pipeline is None or AutoModelForCausalLM is None or AutoTokenizer is None:
            raise LLMClientError("transformers가 필요합니다. `pip install transformers` 후 다시 시도하세요.")
        dtype = self._resolve_dtype(self.torch_dtype)
        model_kwargs: Dict[str, Any] = {}
        if dtype != "auto":
            model_kwargs["torch_dtype"] = dtype
        try:
            self._pipe = pipeline(
                "text-generation",
                model=self.model,
                tokenizer=self.model,
                model_kwargs=model_kwargs,
                device_map=self.device or "auto",
                trust_remote_code=False,
            )
        except Exception as exc:
            raise LLMClientError(f"로컬 모델 로드에 실패했습니다: {exc}") from exc

    def is_available(self) -> bool:
        return self._pipe is not None

    def generate(self, prompt: str, *, system: Optional[str] = None, timeout: float = 30.0) -> str:
        if self._pipe is None:
            raise LLMClientError("로컬 모델이 초기화되지 않았습니다.")
        full_prompt = prompt if not system else f"{system.strip()}\n\n{prompt.strip()}"
        try:
            outputs = self._pipe(
                full_prompt,
                max_new_tokens=max(1, int(self.max_new_tokens)),
                do_sample=False,
                num_return_sequences=1,
            )
        except Exception as exc:
            raise LLMClientError(f"로컬 모델 호출에 실패했습니다: {exc}") from exc

        if not outputs:
            return ""
        first = outputs[0]
        if isinstance(first, dict):
            text = first.get("generated_text", "")
        else:
            text = str(first)
        text = text or ""
        # Best-effort: 제거할 프롬프트가 있으면 잘라낸다.
        if text.startswith(full_prompt):
            text = text[len(full_prompt) :].lstrip()
        return text

    @staticmethod
    def _resolve_dtype(raw: str) -> Any:
        if torch is None:
            return "auto"
        lowered = (raw or "").strip().lower()
        if lowered in {"fp16", "float16", "half"}:
            return torch.float16
        if lowered in {"bf16", "bfloat16"}:
            return torch.bfloat16
        if lowered in {"fp32", "float32"}:
            return torch.float32
        return "auto"


@dataclass
class LocalLlamaCppClient(LLMClient):
    """Loads a local GGUF (e.g., Gemma) via llama.cpp without any server."""

    model: str
    n_ctx: int = 4096
    n_threads: int = 0
    n_gpu_layers: int = -1
    max_new_tokens: int = 512
    _llm: Any = field(init=False, default=None)

    def __post_init__(self) -> None:
        if Llama is None:
            raise LLMClientError("llama-cpp-python이 필요합니다. `pip install llama-cpp-python` 후 다시 시도하세요.")
        try:
            self._llm = Llama(
                model_path=self.model,
                n_ctx=max(256, int(self.n_ctx)),
                n_threads=int(self.n_threads) if self.n_threads else None,
                n_gpu_layers=int(self.n_gpu_layers),
                logits_all=False,
            )
        except Exception as exc:
            raise LLMClientError(f"GGUF 로드에 실패했습니다: {exc}") from exc

    def is_available(self) -> bool:
        return self._llm is not None

    def generate(self, prompt: str, *, system: Optional[str] = None, timeout: float = 30.0) -> str:
        if self._llm is None:
            raise LLMClientError("llama.cpp 모델이 초기화되지 않았습니다.")
        
        # Use chat completion API for proper template handling
        messages = []
        if system:
            messages.append({"role": "system", "content": system.strip()})
        messages.append({"role": "user", "content": prompt.strip()})
        
        try:
            out = self._llm.create_chat_completion(
                messages=messages,
                max_tokens=max(1, int(self.max_new_tokens)),
                temperature=0.0,
            )
        except Exception as exc:
            raise LLMClientError(f"llama.cpp 호출에 실패했습니다: {exc}") from exc
        
        text = ""
        if isinstance(out, dict):
            choices = out.get("choices") or []
            if choices and isinstance(choices[0], dict):
                message = choices[0].get("message", {})
                text = message.get("content", "") or ""
        if not text:
            text = str(out)
        return text.strip()

    def generate_chat(self, messages: List[Dict[str, str]], *, timeout: float = 30.0) -> str:
        """Generate response from a list of messages for multi-turn conversation."""
        if self._llm is None:
            raise LLMClientError("llama.cpp 모델이 초기화되지 않았습니다.")
        
        try:
            out = self._llm.create_chat_completion(
                messages=messages,
                max_tokens=max(1, int(self.max_new_tokens)),
                temperature=0.0,
            )
        except Exception as exc:
            raise LLMClientError(f"llama.cpp 호출에 실패했습니다: {exc}") from exc
        
        text = ""
        if isinstance(out, dict):
            choices = out.get("choices") or []
            if choices and isinstance(choices[0], dict):
                message = choices[0].get("message", {})
                text = message.get("content", "") or ""
        if not text:
            text = str(out)
        return text.strip()


@dataclass
class LocalLlamaCliClient(LLMClient):
    """Executes prompts using the bundled llama.cpp `llama-cli` binary."""

    model: str
    n_ctx: int = 4096
    n_threads: int = 0
    n_gpu_layers: int = 0
    max_new_tokens: int = 512
    cli_path: str = ""
    no_mmap: Optional[bool] = None

    def is_available(self) -> bool:
        return bool(self._resolve_cli_path())

    def _resolve_cli_path(self) -> str:
        override = (self.cli_path or os.getenv("LNPCHAT_LLAMA_CLI_PATH") or "").strip()
        if override:
            print(f"[DEBUG] Using Override Llama-CLI: {override}", file=sys.stderr)
            return override

        here = Path(__file__).resolve()
        repo_root = here.parents[2] # expected: AI-summary
        m4_build = repo_root / "models" / "llama.cpp" / "build_m4" / "bin" / "llama-cli"
        metal = repo_root / "models" / "llama.cpp" / "build_metal" / "bin" / "llama-cli"
        cpu = repo_root / "models" / "llama.cpp" / "build_cpu" / "bin" / "llama-cli"
        
        print(f"[DEBUG] Checking M4 Build at: {m4_build} (Exists: {m4_build.exists()})", file=sys.stderr)
        
        if m4_build.exists():
            print(f"[DEBUG] Using M4-optimized Llama-CLI: {m4_build}", file=sys.stderr)
            return str(m4_build)
        if metal.exists():
            print(f"[DEBUG] Using Standard Metal Llama-CLI: {metal}", file=sys.stderr)
            return str(metal)
        if cpu.exists():
            print(f"[DEBUG] Using CPU Llama-CLI: {cpu}", file=sys.stderr)
            return str(cpu)
        
        print("[DEBUG] No Llama-CLI found!", file=sys.stderr)
        return ""

    def generate(self, prompt: str, *, system: Optional[str] = None, timeout: float = 30.0) -> str:
        cli = self._resolve_cli_path()
        if not cli:
            raise LLMClientError("llama-cli not found (set LNPCHAT_LLAMA_CLI_PATH or build models/llama.cpp)")

        full_prompt = prompt if not system else f"{system.strip()}\n\n{prompt.strip()}"
        model_path = self.model
        if not model_path:
            raise LLMClientError("llama-cli backend requires a model path (GGUF)")

        args = [
            cli,
            "--simple-io",
            "--no-display-prompt",
            "--no-perf",
            # "--log-disable", # Enable logs for debugging and stability
            "-no-cnv",
            "-m",
            model_path,
            "-c",
            str(max(256, int(self.n_ctx))),
            "-n",
            str(max(1, int(self.max_new_tokens))),
            "--temp",
            "0",
            "-ngl",
            str(int(self.n_gpu_layers)),
            "-p",
            full_prompt,
        ]
        if self.n_threads:
            args.extend(["-t", str(int(self.n_threads))])

        resolved_no_mmap = self.no_mmap
        if resolved_no_mmap is None:
            raw = (os.getenv("LNPCHAT_LLAMA_CLI_NO_MMAP") or "").strip().lower()
            if raw in {"1", "true", "yes"}:
                resolved_no_mmap = True
            elif raw in {"0", "false", "no"}:
                resolved_no_mmap = False
            else:
                resolved_no_mmap = "build_metal" in cli
        if resolved_no_mmap:
            args.append("--no-mmap")

        print(f"[DEBUG] Executing Command: {shlex.join(args)}", file=sys.stderr)
        
        try:
            proc = subprocess.run(
                args,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=timeout,
                check=False,
            )
        except subprocess.TimeoutExpired as exc:
            raise LLMClientError(f"llama-cli timed out after {timeout}s") from exc
        except FileNotFoundError as exc:
            raise LLMClientError(f"llama-cli executable not found: {cli}") from exc
        except Exception as exc:
            raise LLMClientError(f"llama-cli invocation failed: {exc}") from exc

        if proc.returncode != 0:
            stderr = proc.stderr.decode("utf-8", "ignore").strip()
            # Special handling for Metal compatibility issues (e.g., M4 / pre-M5 checks)
            # We check for generic Metal errors or the specific tensor API warning which seems to imply failure here
            if ("ggml_metal" in stderr or "tensor API disabled" in stderr) and int(self.n_gpu_layers) != 0:
                # Fallback to CPU
                print("⚠️ Metal backend failed (M4/compatibility issue). Falling back to CPU mode...", file=sys.stderr)

                # Parse args to find -ngl and replace its value safely
                new_args = list(args)
                try:
                    ngl_idx = new_args.index("-ngl")
                    if ngl_idx + 1 < len(new_args):
                        new_args[ngl_idx + 1] = "0"
                except ValueError:
                    new_args.extend(["-ngl", "0"])
                
                args = new_args
                
                # Force disable Metal in environment for fallback
                env = os.environ.copy()
                env["GGML_METAL_DISABLE"] = "1"

                try:
                    proc = subprocess.run(
                        args,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        timeout=timeout,
                        env=env,
                        check=False,
                    )
                    if proc.returncode == 0:
                         text = proc.stdout.decode("utf-8", "ignore").strip()
                         return self._clean_output(text)
                    stderr = proc.stderr.decode("utf-8", "ignore").strip()
                except Exception:
                    pass

            raise LLMClientError(f"llama-cli failed ({proc.returncode}): {stderr[:300]}")

        text = proc.stdout.decode("utf-8", "ignore").strip()
        return self._clean_output(text)

    def _clean_output(self, text: str) -> str:
        if not text:
            return ""
        cleaned = []
        for line in text.splitlines():
            trimmed = line.strip()
            if "EOF by user" in trimmed:
                continue
            if trimmed.startswith(">") and "EOF" in trimmed:
                continue
            cleaned.append(line)
        return "\n".join(cleaned).strip()


@dataclass
class LocalLlamaCppSubprocessClient(LLMClient):
    """Runs llama-cpp-python inside a subprocess (crash-safe), with optional llama-cli fallback."""

    model: str
    n_ctx: int = 4096
    n_threads: int = 0
    n_gpu_layers: int = 0
    max_new_tokens: int = 512
    allow_cli_fallback: bool = True

    def is_available(self) -> bool:
        return True

    def generate(self, prompt: str, *, system: Optional[str] = None, timeout: float = 30.0) -> str:
        model_path = self.model
        if not model_path:
            raise LLMClientError("llama-cpp-python backend requires a model path (GGUF)")

        # Use dedicated chat worker (not meeting summarization worker)
        payload = {
            "prompt": prompt,
            "system": system or "",
            "model_path": model_path,
            "n_ctx": int(self.n_ctx),
            "n_threads": int(self.n_threads),
            "n_gpu_layers": int(self.n_gpu_layers),
            "max_new_tokens": int(self.max_new_tokens),
            "temperature": 0.7,
        }

        try:
            proc = subprocess.run(
                [sys.executable, "-m", "core.agents.meeting.llm.llama_cpp_chat_worker"],
                input=json.dumps(payload, ensure_ascii=False).encode("utf-8"),
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=timeout,
                check=False,
            )
        except subprocess.TimeoutExpired as exc:
            raise LLMClientError(f"llama.cpp chat worker timed out after {timeout}s") from exc
        except Exception as exc:
            raise LLMClientError(f"llama.cpp chat worker launch failed: {exc}") from exc

        if proc.returncode != 0:
            err = (proc.stderr or b"").decode("utf-8", "ignore").strip()
            raise LLMClientError(f"llama.cpp chat worker failed (code={proc.returncode}): {err[:300]}")

        raw = (proc.stdout or b"").decode("utf-8", "ignore").strip()
        try:
            data = json.loads(raw) if raw else {}
        except json.JSONDecodeError as exc:
            raise LLMClientError("llama.cpp chat worker returned invalid JSON") from exc
        
        # Chat worker returns {"response": "..."} 
        text = str(data.get("response") or "").strip()
        return text


def create_llm_client(backend: Optional[str], *, model: str, host: str = "", options: Optional[Dict[str, str]] = None) -> Optional[LLMClient]:
    backend = (backend or "").strip().lower()
    if not backend:
        return None
    if backend in {"none", "off", "disabled"}:
        return None
    if backend == "ollama":
        client = OllamaClient(model=model or "llama3", host=host or "", options=options or {})
        if not client.is_available():
            raise LLMClientError("ollama backend requested but server is not reachable")
        return client
    if backend == "local_gemma":
        opts = options or {}
        device = opts.get("device") or host or "auto"
        torch_dtype = opts.get("torch_dtype") or "auto"
        max_new_tokens = opts.get("max_new_tokens") or opts.get("num_predict") or 512
        return LocalGemmaClient(
            model=model,
            device=str(device),
            torch_dtype=str(torch_dtype),
            max_new_tokens=int(max_new_tokens),
        )
    if backend == "local_llamacpp":
        opts = options or {}
        n_gpu_layers = opts.get("n_gpu_layers")
        if n_gpu_layers is None:
            n_gpu_layers = os.getenv("LNPCHAT_LLAMACPP_GPU_LAYERS", "-1")
        # Default to in-process (0) for faster response - subprocess available via env var
        use_subprocess = os.getenv("LNPCHAT_LLAMACPP_SUBPROCESS", "0").strip().lower() in {
            "1",
            "true",
            "yes",
        }
        fallback = os.getenv("LNPCHAT_LLAMACPP_FALLBACK_CLI", "1").strip().lower() not in {
            "",
            "0",
            "false",
            "no",
        }
        if use_subprocess:
            return LocalLlamaCppSubprocessClient(
                model=model,
                n_ctx=int(opts.get("n_ctx", 4096)),
                n_threads=int(opts.get("n_threads", 0)),
                n_gpu_layers=int(n_gpu_layers),
                max_new_tokens=int(opts.get("max_new_tokens", opts.get("num_predict", 512))),
                allow_cli_fallback=fallback,
            )
        try:
            return LocalLlamaCppClient(
                model=model,
                n_ctx=int(opts.get("n_ctx", 4096)),
                n_threads=int(opts.get("n_threads", 0)),
                n_gpu_layers=int(n_gpu_layers),
                max_new_tokens=int(opts.get("max_new_tokens", opts.get("num_predict", 512))),
            )
        except LLMClientError:
            if not fallback:
                raise
            return LocalLlamaCliClient(
                model=model,
                n_ctx=int(opts.get("n_ctx", 4096)),
                n_threads=int(opts.get("n_threads", 0)),
                n_gpu_layers=int(n_gpu_layers),
                max_new_tokens=int(opts.get("max_new_tokens", opts.get("num_predict", 512))),
            )
    if backend in {"local_llama_cli", "llama_cli", "llama-cli"}:
        opts = options or {}
        n_gpu_layers = opts.get("n_gpu_layers")
        if n_gpu_layers is None:
            n_gpu_layers = os.getenv("LNPCHAT_LLAMACPP_GPU_LAYERS", "-1")
        return LocalLlamaCliClient(
            model=model,
            n_ctx=int(opts.get("n_ctx", 4096)),
            n_threads=int(opts.get("n_threads", 0)),
            n_gpu_layers=int(n_gpu_layers),
            max_new_tokens=int(opts.get("max_new_tokens", opts.get("num_predict", 512))),
            cli_path=str(opts.get("cli_path", "")),
        )
    raise LLMClientError(f"unsupported LLM backend: {backend}")
