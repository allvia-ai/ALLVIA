import socket
import subprocess
import time
import os
from pathlib import Path


def _find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _spawn_core() -> subprocess.Popen:
    binary = Path("./core/target/debug/local_os_agent")
    if not binary.exists():
        subprocess.run(
            ["cargo", "build", "--manifest-path", "core/Cargo.toml", "--bin", "local_os_agent"],
            check=True,
        )

    env = os.environ.copy()
    env["STEER_LOCK_DISABLED"] = "1"
    env["STEER_DISABLE_EVENT_TAP"] = "1"
    env["STEER_API_PORT"] = str(_find_free_port())
    return subprocess.Popen(
        ["./target/debug/local_os_agent"],
        cwd="./core",
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
        env=env,
    )


def test_monitoring():
    # Ensure Downloads exists
    home = str(Path.home())
    downloads = os.path.join(home, "Downloads")
    test_file = os.path.join(downloads, "agent_monitor_test.txt")
    
    if os.path.exists(test_file):
        os.remove(test_file)

    process = _spawn_core()
    time.sleep(3)
    if process.poll() is not None:
        stdout, stderr = process.communicate()
        raise AssertionError(f"core exited before monitor test\nSTDOUT:\n{stdout}\nSTDERR:\n{stderr}")

    assert process.stdin is not None
    process.stdin.write("status\n")
    process.stdin.flush()
    time.sleep(1)

    with open(test_file, "w") as f:
        f.write("Hello Agent")
    
    time.sleep(6)
    process.terminate()
    try:
        stdout, stderr = process.communicate(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        stdout, stderr = process.communicate()

    status_ok = ("System Status" in stdout) or ("Top Apps" in stdout)
    watcher_ok = ("agent_monitor_test.txt" in stdout) or ("Watching for changes in" in stdout)

    # Cleanup
    if os.path.exists(test_file):
        os.remove(test_file)

    assert status_ok
    assert watcher_ok
    assert "FATAL" not in stderr

if __name__ == "__main__":
    test_monitoring()
