# PROJECT_BIBLE.md

## 1. Project Overview

- Project Name: Local OS Super Agent
- Role: Local execution agent for OS tasks and workflows
- Core Philosophy: "LLM plans, Rust enforces, native layer executes"
- Critical Constraint: LLM output is never executed directly; every action passes policy/security checks.

---

## 2. Current Source of Truth

This repository has evolved from the early Rust+Swift adapter concept.
The active architecture is:

```text
local-os-agent/
├── core/                  # Rust core engine + API server + policy + automation
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── api_server.rs
│       ├── controller/
│       ├── policy.rs
│       ├── security.rs
│       └── macos/
├── web/                   # React/Vite operator UI
├── desktop/               # Desktop frontend shell
├── desktop/src-tauri/     # Tauri wrapper and native packaging
└── docs/                  # Product/architecture/security docs
```

Notes:
- Legacy Swift `adapter/` layout in older docs is not the active runtime path.
- Primary binary is `local_os_agent` (from `core` crate).

---

## 3. Security Model (Live)

1. Zero Trust:
- All model outputs are treated as untrusted.
- Action execution is gated by policy/security classification.

2. Write Lock:
- Default state is locked for write-like actions.
- Unlock is explicit and time/flow controlled.

3. Command/Tool Guardrails:
- Shell risk classification (`Safe`, `Warning`, `Critical`).
- Tool allowlist/denylist and execution approval flows.

4. Fail-safe:
- Runtime can continue even if optional subsystems fail (e.g., API bind conflict in CLI-only flows).

---

## 4. Validation Baseline

Minimum validation before accepting changes:

1. `cargo check` in `core/`
2. `cargo test --no-run` in `core/`
3. `npm run lint` and `npm run build` in `web/`
4. `pytest -q tests` with:
- `STEER_LOCK_DISABLED=1`
- unique `STEER_API_PORT` for isolated test runs

---

## 5. Operational Notes

- Never hardcode production secrets in scripts or source.
- Prefer env-based configuration for tokens/keys.
- Keep docs aligned with actual executable paths and binary names.
- Treat this document as a living snapshot of current architecture.

---

## 6. User Goal Snapshot (2026-02-08)

- Primary goal: local global-permission execution agent that can complete complex natural-language scenarios end-to-end.
- Validation standard: app-result-based verification (not terminal-only success), with node-level evidence.
- Reporting standard: Telegram report must include per-node output evidence, success/failure reasoning, and concise summary.
- Behavior loop: collect user execution logs, detect patterns, and proactively suggest automation workflows.
- Automation output: propose executable workflow designs and support either n8n integration or direct custom workflow construction.
- Current development constraint: real log-collection/workflow pipeline is being handled on another machine; this repo proceeds with mock workflow input to keep implementation moving.
