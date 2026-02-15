from __future__ import annotations

import pytest

from core.infra import ModelManager

pytestmark = pytest.mark.smoke


def test_model_manager_caches_and_releases() -> None:
    loads: list[str] = []
    disposed: list[str] = []

    def loader(name: str) -> dict[str, str]:
        loads.append(name)
        return {"name": name}

    def disposer(model: dict[str, str]) -> None:
        disposed.append(model["name"])

    manager = ModelManager(loader, disposer=disposer)

    model_a = manager.get("foo")
    assert loads == ["foo"]
    assert manager.stats()["foo"] == 1

    same_model = manager.get("foo")
    assert same_model is model_a
    assert loads == ["foo"]
    assert manager.stats()["foo"] == 2

    manager.release("foo")
    assert manager.stats()["foo"] == 1

    manager.release("foo")
    assert "foo" not in manager.stats()
    assert disposed == ["foo"]

    manager.get("bar")
    manager.clear()
    assert disposed[-1] == "bar"

    with pytest.raises(ValueError):
        manager.get("  ")
