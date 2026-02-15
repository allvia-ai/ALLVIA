from __future__ import annotations

from core.infra import ModelSelector


def test_model_selector_prefers_policy_and_gpu():
    selector = ModelSelector(
        registry={
            "default": "base",
            "foo": "foo-cpu",
            "foo:gpu": "foo-gpu",
        },
        default_model="base",
    )
    assert selector.pick() == "base"
    assert selector.pick("foo") == "foo-cpu"
    assert selector.pick("foo", prefer_gpu=True) == "foo-gpu"
    assert selector.pick("bar") == "base"
