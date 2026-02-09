import os
import socket
import subprocess
import time
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


def _spawn_core() -> subprocess.Popen:
    env = os.environ.copy()
    env["STEER_LOCK_DISABLED"] = "1"
    env["STEER_DISABLE_EVENT_TAP"] = "1"
    env["STEER_API_PORT"] = str(_find_free_port())
    return subprocess.Popen(
        [_ensure_core_binary()],
        cwd="./core",
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
        env=env,
    )

def test_behavior_analysis():
    process = _spawn_core()
    time.sleep(2)

    if process.poll() is not None:
        stdout, stderr = process.communicate()
        raise AssertionError(f"core exited before test started\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}")

    assert process.stdin is not None
    process.stdin.write("unlock\n")
    process.stdin.flush()
    time.sleep(1)

    for _ in range(25):
        process.stdin.write("fake_log\n")
        process.stdin.flush()
        time.sleep(0.1)

    time.sleep(2)
    process.terminate()
    try:
        stdout, stderr = process.communicate(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate()

    with open("behavior_test_output.txt", "w") as file:
        file.write(stdout)
        file.write("\n\n--- STDERR ---\n")
        file.write(stderr)

    assert "Simulated Log Sent" in stdout
    assert "FATAL" not in stderr

if __name__ == "__main__":
    test_behavior_analysis()
