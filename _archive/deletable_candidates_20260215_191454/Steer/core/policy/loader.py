"""Helpers to load and validate smart folder policies."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Dict, Iterable, List

from core.utils import get_logger, resolve_repo_root

LOGGER = get_logger("policy.loader")


def _require_jsonschema():
    try:
        import jsonschema  # type: ignore
    except ModuleNotFoundError as exc:
        raise RuntimeError(
            "Policy schema validation requires `jsonschema`. "
            "Install dependencies (e.g. `pip install -r requirements.txt`) and retry."
        ) from exc
    return jsonschema


def load_policy(path: Path) -> Dict[str, Any]:
    """Load a single smart folder policy."""
    policies = load_policy_file(path)
    if not policies:
        raise ValueError(f"No policy found in {path}")
    return policies[0]


def load_policy_file(path: Path) -> List[Dict[str, Any]]:
    """Load one or more smart folder policies from a JSON file."""
    raw = json.loads(path.read_text(encoding="utf-8"))
    if isinstance(raw, dict):
        items: Iterable[Dict[str, Any]] = [raw]
    elif isinstance(raw, list):
        items = raw
    else:
        raise ValueError("Policy file must contain a JSON object or array of objects")

    schema = _load_schema()
    jsonschema = _require_jsonschema()
    policies: List[Dict[str, Any]] = []
    for idx, data in enumerate(items):
        try:
            jsonschema.validate(data, schema)
            policies.append(data)
        except Exception as exc:
             LOGGER.warning(f"Validation failed for policy item {idx} in {path}: {exc}")
             continue
             
    LOGGER.debug("Loaded %d policies from %s", len(policies), path)
    return policies


def _load_schema() -> Dict[str, Any]:
    # Updated path to new schema location
    schema_path = resolve_repo_root() / "core" / "policy" / "schema" / "smart_folder_policy.schema.json"
    try:
        return json.loads(schema_path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        LOGGER.error("Policy schema missing at %s", schema_path)
        raise exc
