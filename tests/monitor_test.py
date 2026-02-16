import socket
import subprocess
import time
import os
import selectors
import shutil
import tempfile
import urllib.error
import urllib.request
from pathlib import Path


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _spawn_core(extra_env: dict[str, str] | None = None) -> tuple[subprocess.Popen, int]:
    binary = Path("./core/target/debug/local_os_agent")
    if not binary.exists():
        subprocess.run(
            ["cargo", "build", "--manifest-path", "core/Cargo.toml", "--bin", "local_os_agent"],
            check=True,
        )

    env = os.environ.copy()
    env["STEER_TEST_MODE"] = "1"
    env["STEER_LOCK_DISABLED"] = "1"
    env["STEER_DISABLE_EVENT_TAP"] = "1"
    env["STEER_DISABLE_APP_WATCHER"] = "1"
    env["STEER_API_ALLOW_NO_KEY"] = "1"
    port = _find_free_port()
    env["STEER_API_PORT"] = str(port)
    if extra_env:
        env.update(extra_env)
    process = subprocess.Popen(
        ["./target/debug/local_os_agent"],
        cwd="./core",
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
        env=env,
    )
    return process, port


def _wait_for_health(port: int, timeout_sec: float = 15.0) -> bool:
    start = time.time()
    url = f"http://127.0.0.1:{port}/api/health"
    while time.time() - start < timeout_sec:
        try:
            with urllib.request.urlopen(url, timeout=2) as resp:
                if resp.read().decode("utf-8").strip() == "ok":
                    return True
        except (urllib.error.URLError, TimeoutError):
            pass
        time.sleep(0.3)
    return False


def _collect_output(process: subprocess.Popen, timeout: int = 5) -> tuple[str, str]:
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate()
    return stdout, stderr


def _drain_stdout_nonblocking(process: subprocess.Popen, timeout_sec: float = 0.3) -> str:
    if process.stdout is None:
        return ""
    collected: list[str] = []
    fd = process.stdout.fileno()
    try:
        os.set_blocking(fd, False)
    except OSError:
        pass
    selector = selectors.DefaultSelector()
    try:
        selector.register(process.stdout, selectors.EVENT_READ)
        end = time.time() + timeout_sec
        while time.time() < end:
            events = selector.select(timeout=0.1)
            if not events:
                continue
            for _key, _ in events:
                try:
                    chunk = os.read(fd, 4096)
                except BlockingIOError:
                    continue
                if not chunk:
                    continue
                collected.append(chunk.decode("utf-8", errors="replace"))
    finally:
        selector.close()
    return "".join(collected)


def _wait_for_stdout_pattern(
    process: subprocess.Popen,
    patterns: list[str],
    timeout_sec: float = 8.0,
) -> tuple[bool, str]:
    start = time.time()
    collected = ""
    while time.time() - start < timeout_sec:
        collected += _drain_stdout_nonblocking(process, timeout_sec=0.25)
        if any(p in collected for p in patterns):
            return True, collected
        if process.poll() is not None:
            break
        time.sleep(0.05)
    return any(p in collected for p in patterns), collected


def test_monitoring():
    # Use isolated watcher path to avoid touching real ~/Downloads.
    downloads = tempfile.mkdtemp(prefix="steer-monitor-test-")
    test_file = os.path.join(downloads, "agent_monitor_test.txt")
    process, port = _spawn_core({"STEER_DOWNLOADS_DIR": downloads})
    streamed_stdout = ""
    try:
        if not _wait_for_health(port, timeout_sec=20):
            if process.poll() is None:
                process.terminate()
            stdout, stderr = _collect_output(process, timeout=5)
            raise AssertionError(f"core failed to become healthy\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}")

        if process.poll() is not None:
            stdout, stderr = process.communicate()
            raise AssertionError(f"core exited before monitor test\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}")

        with open(test_file, "w") as f:
            f.write("Hello Agent")

        matched, watcher_stdout = _wait_for_stdout_pattern(
            process,
            ["agent_monitor_test.txt", "Watching for changes in"],
            timeout_sec=10,
        )
        streamed_stdout += watcher_stdout
        if not matched:
            streamed_stdout += _drain_stdout_nonblocking(process, timeout_sec=0.4)
        process.terminate()
        try:
            stdout, stderr = process.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, stderr = process.communicate()
        stdout = f"{streamed_stdout}{stdout}"

        watcher_ok = ("agent_monitor_test.txt" in stdout) or ("Watching for changes in" in stdout)

        assert watcher_ok
        assert "FATAL" not in stderr
    finally:
        shutil.rmtree(downloads, ignore_errors=True)

if __name__ == "__main__":
    test_monitoring()
