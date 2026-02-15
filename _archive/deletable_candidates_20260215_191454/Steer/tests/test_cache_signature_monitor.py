import threading

import pytest

from core.search.retriever import CacheSignatureMonitor


@pytest.mark.full
def test_cache_signature_monitor_triggers_after_stable_change():
    signatures = [
        (1.0, 1.0, 1.0),
        (2.0, 2.0, 2.0),
        (2.0, 2.0, 2.0),
    ]
    index = {"value": 0}

    def compute():
        value = signatures[min(index["value"], len(signatures) - 1)]
        index["value"] += 1
        return value

    observed = []

    monitor = CacheSignatureMonitor(
        compute,
        lambda prev, curr: observed.append((prev, curr)),
        interval=0.01,
        stability_checks=2,
    )
    monitor.prime((1.0, 1.0, 1.0))

    monitor.check_once()  # baseline match
    monitor.check_once()  # pending change
    monitor.check_once()  # confirms change

    assert observed == [((1.0, 1.0, 1.0), (2.0, 2.0, 2.0))]


@pytest.mark.full
def test_cache_signature_monitor_start_stop_background_thread():
    values = [
        (1.0, 1.0, 1.0),
        (1.0, 1.0, 1.0),
        (3.0, 3.0, 3.0),
        (3.0, 3.0, 3.0),
    ]
    index = {"value": 0}
    lock = threading.Lock()
    triggered = threading.Event()

    def compute():
        with lock:
            value = values[min(index["value"], len(values) - 1)]
            index["value"] += 1
            return value

    def on_change(_prev, _curr):
        triggered.set()

    monitor = CacheSignatureMonitor(
        compute,
        on_change,
        interval=0.01,
        stability_checks=2,
    )
    monitor.prime((1.0, 1.0, 1.0))

    monitor.start()
    try:
        assert triggered.wait(timeout=0.5), "background monitor should trigger on change"
    finally:
        monitor.stop()

    assert not monitor.is_running()
