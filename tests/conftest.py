import os


def pytest_configure() -> None:
    # Tests rely on test-only runtime flags such as STEER_LOCK_DISABLED.
    os.environ.setdefault("STEER_TEST_MODE", "1")
