import json
import os
import socket
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _ensure_core_binary() -> str:
    binary = Path("./core/target/debug/local_os_agent")
    if not binary.exists():
        subprocess.run(
            ["cargo", "build", "--manifest-path", "core/Cargo.toml", "--bin", "local_os_agent"],
            check=True,
        )
    return "./target/debug/local_os_agent"


def _http_get(url: str) -> str:
    with urllib.request.urlopen(url, timeout=2) as resp:
        return resp.read().decode("utf-8")


def _collect_output(process: subprocess.Popen, timeout: int = 5) -> tuple[str, str]:
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate()
    return stdout, stderr


def _wait_for_health(port: int, timeout_sec: float = 15.0) -> bool:
    start = time.time()
    url = f"http://127.0.0.1:{port}/api/health"
    while time.time() - start < timeout_sec:
        try:
            body = _http_get(url).strip()
            if body == "ok":
                return True
        except (urllib.error.URLError, TimeoutError):
            pass
        time.sleep(0.4)
    return False

def test_integration():
    port = _find_free_port()
    env = os.environ.copy()
    env["STEER_TEST_MODE"] = "1"
    env["STEER_LOCK_DISABLED"] = "1"
    env["STEER_DISABLE_EVENT_TAP"] = "1"
    env["STEER_DISABLE_DOWNLOAD_WATCHER"] = "1"
    env["STEER_DISABLE_APP_WATCHER"] = "1"
    env["STEER_API_ALLOW_NO_KEY"] = "1"
    env["STEER_API_PORT"] = str(port)

    process = subprocess.Popen(
        [_ensure_core_binary()],
        cwd="./core",
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
        env=env,
    )

    try:
        healthy = _wait_for_health(port)
        if not healthy:
            if process.poll() is None:
                process.terminate()
            stdout, stderr = _collect_output(process, timeout=5)
            raise AssertionError(f"API health check failed\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}")

        raw = _http_get(f"http://127.0.0.1:{port}/api/status")
        status = json.loads(raw)
        assert "cpu_usage" in status
        assert "memory_used" in status
        assert "memory_total" in status
    finally:
        if process.poll() is None and process.stdin is not None:
            try:
                if process.stdin.closed:
                    raise ValueError("stdin already closed")
                process.stdin.write("exit\n")
                process.stdin.flush()
            except (BrokenPipeError, ValueError):
                pass
        if process.poll() is None:
            process.terminate()
            _collect_output(process, timeout=5)

if __name__ == "__main__":
    test_integration()
