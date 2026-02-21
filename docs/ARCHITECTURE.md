# System Architecture

## 1. High-Level Design
`[Web/Tauri UI] <-> [Rust Core API + Controller] <-> [macOS Native Bindings]`

## 2. Runtime Components

### A. Rust Core (`core/`)
- Role: state management, policy enforcement, planning, execution, verification, API server.
- Entry binary: `local_os_agent`.
- API server: Axum on `127.0.0.1:${STEER_API_PORT:-5680}`.
- Core modules include `api_server`, `controller`, `policy`, `llm_gateway`, `monitor`, `macos`.

### B. Web UI (`web/`)
- Role: operator UI for launch/chat/dashboard/verification.
- Stack: React + Vite + TypeScript.
- Talks to core through `http://127.0.0.1:5680/api` (configurable via `VITE_API_BASE_URL`).

### C. Desktop Wrapper (`web/src-tauri/`)
- Role: desktop shell and native windowing/packaging.
- Stack: Tauri + Vite.
- Hosts the same web front-end and invokes core capabilities.

### D. Native Control Layer
- Implemented through Rust macOS bindings (`core/src/macos/*`, AppleScript helpers).
- Legacy Swift adapter references are historical and not part of the active runtime path.

## 3. Data Flow
1. User action enters via web/Tauri UI or CLI.
2. Core validates request using policy/security layers.
3. Planner/executor performs shell/UI/native actions.
4. Results are stored (SQLite / memory modules) and exposed through API.
5. UI reflects status, logs, and verification outcomes.

## 4. Security Boundaries
- Write lock and risk classification are enforced in core policy/security modules.
- API auth can be enabled with `STEER_API_KEY`.
- Shell and tool allow/deny controls are managed via environment-based policy.
