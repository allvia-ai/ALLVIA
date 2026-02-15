from __future__ import annotations

import pytest


@pytest.mark.smoke
def test_search_endpoint_returns_results(client):
    resp = client.post("/api/search", json={"query": "테스트", "top_k": 2})
    assert resp.status_code == 200
    data = resp.json()
    assert "results" in data
    assert "session" in data
    assert "answer" in data
    assert data["answer_source"] in {"llm", "fallback", "none"}
    assert isinstance(data.get("history"), list)
    assert data.get("history") and data["history"][-1]["role"] == "assistant"


@pytest.mark.smoke
def test_feedback_endpoint_accepts_actions(client):
    resp = client.post(
        "/api/feedback",
        json={
            "session_id": "test",
            "action": "like",
            "ext": ".pdf",
        },
    )
    assert resp.status_code == 200
    assert resp.json()["status"] == "ok"
