# OpenClaw / Moltbot Upgrade Plan (Code-Tracked)

This document records what has already been ported into this repository and what can be added next with minimal regression risk.

## Already Ported (In Use)

1. Retry/backoff with retry_after support
- Local module: `core/src/retry_policy.rs`
- Applied in:
  - `core/src/n8n_api.rs`
  - `core/src/telegram_transport.rs`

2. Structured diagnostic events (jsonl)
- Local module: `core/src/diagnostic_events.rs`
- Emitted by:
  - `core/src/execution_controller.rs`
  - `core/src/n8n_api.rs`
  - `core/src/telegram_transport.rs`
  - `core/src/api_server.rs`
  - `core/src/singleton_lock.rs`

3. Outbound policy gate (mail send)
- Local module: `core/src/outbound_policy.rs`
- Enforced in:
  - `core/src/controller/actions.rs`

4. Cross-context outbound policy (Telegram transport)
- Local modules:
  - `core/src/outbound_policy.rs`
  - `core/src/send_policy.rs`
- Enforced in:
  - `core/src/telegram_transport.rs`
  - `core/src/integrations/telegram.rs`

5. Run-attempt structured traces for false-fail debugging
- Emitted by:
  - `core/src/execution_controller.rs`
  - `run_nl_request_with_telegram.sh`
  - `run_complex_scenarios.sh`

6. Approval fallback hardening (allow-once test-only by default)
- Local module: `core/src/approval_gate.rs`
- Behavior:
  - `STEER_APPROVAL_ASK_FALLBACK=allow-once` is blocked outside test/CI unless
    `STEER_APPROVAL_ALLOW_ONCE_NON_TEST=1` is explicitly set.
  - Emits `approval.ask_fallback` diagnostic events.

7. Resume token wiring (API + Launcher)
- Local modules:
  - `core/src/api_server.rs`
  - `web/src/lib/api.ts`
  - `web/src/features/launcher/Launcher.tsx`
- Behavior:
  - `/api/agent/execute` now accepts `resume_token` / `resume_from`.
  - `resume_token` is parsed/validated (plan_id + step index range).
  - Launcher resume/approval flows pass the last `resume_token`.

8. Gateway-lock telemetry surfaced to UI/API
- Local modules:
  - `core/src/singleton_lock.rs`
  - `core/src/api_server.rs`
  - `web/src/lib/api.ts`
  - `web/src/features/launcher/Launcher.tsx`
- Behavior:
  - lock acquire/bypass/blocked/stale-recovered/rejected counters are recorded in-process.
  - `/api/system/lock-metrics` exposes current snapshot.
  - Launcher diagnostics renders Singleton Lock Telemetry to aid false-fail forensics.

9. API no-key development mode hardened (explicit secret required)
- Local module:
  - `core/src/api_server.rs`
- Behavior:
  - no-key mode now requires explicit `STEER_API_DEV_HEADER_VALUE` configuration.
  - if not configured, local no-key requests are denied and diagnostic reason is recorded.

10. Mail draft flood guard for auto-draft selection
- Local module:
  - `core/src/controller/actions.rs`
- Behavior:
  - before auto-selecting Mail draft, outgoing draft count is checked.
  - if drafts exceed `STEER_MAIL_MAX_OUTGOING_FOR_AUTO_DRAFT` (default `8`), it attempts run-scope cleanup first.
  - still over limit -> action fails with `ambiguous_draft` to prevent wrong-send/compose-window flood.

11. Complex scenario semantic verification parity (NL runner policy-level)
- Local module:
  - `run_complex_scenarios.sh`
- Behavior:
  - token truncation is tracked with `STEER_SEMANTIC_MAX_TOKENS` and can fail hard via `STEER_SEMANTIC_FAIL_ON_TRUNCATION=1`.
  - app-scope enforcement blocks log-only proof (`LOG_*`) when `STEER_SEMANTIC_REQUIRE_APP_SCOPE=1`.
  - mail DoD verification now applies the same log-only blocking policy used in NL runner.

12. n8n npx runtime/tunnel hardening
- Local module:
  - `core/src/n8n_api.rs`
- Behavior:
  - `npx` CLI fallback outside test/CI is blocked by default (`STEER_N8N_ALLOW_NPX_CLI_NON_TEST=1` required).
  - `npx` CLI fallback for remote `N8N_API_URL` is blocked by default (`STEER_N8N_ALLOW_NPX_CLI_REMOTE=1` required).
  - `npx --tunnel` is test/CI-only by default (`STEER_N8N_ALLOW_NPX_TUNNEL_NON_TEST=1` required outside test).
  - blocked/start events are emitted to diagnostic events.

13. AX snapshot focus recovery hardening
- Local module:
  - `core/src/macos/accessibility.rs`
- Behavior:
  - when focused app is missing, fallback app activation (`STEER_AX_SNAPSHOT_FALLBACK_APP`) is attempted.
  - focused window recovery retries before failing.
  - missing focus app/window now emits diagnostic events (`ax.snapshot.focus_missing`) for forensic trace.

## Next Safe Imports (Recommended)

1. Approval policy matrix persistence hardening
- Goal: enforce explicit allow-once/allow-always/deny with expiration windows and audit labels.
- Target files:
  - `core/src/approval_gate.rs`
  - `core/src/db.rs`

2. Gateway lock telemetry + stale lock auto-healing metrics dashboard
- Goal: show lock contention, stale recovery, and bypass counts in API/launcher.
- Target files:
  - `core/src/singleton_lock.rs`
  - `core/src/api_server.rs`
  - `web/src/features/launcher/Launcher.tsx`

3. LLM tool-call schema strict mode for side-effect actions
- Goal: refuse action plans unless fully schema-valid for write/send operations.
- Target files:
  - `core/src/action_schema.rs`
  - `core/src/controller/planner.rs`
  - `core/src/semantic_contract.rs`

4. Resume token checkpoints for long scenarios
- Goal: deterministic resume from known safe checkpoint after user approval/manual recovery.
- Target files:
  - `core/src/execution_controller.rs`
  - `core/src/api_server.rs`
  - `run_nl_request_with_telegram.sh`

5. Cross-context outbound policy extension (Slack/Discord adapters)
- Goal: extend the same strictness now used in Mail + Telegram to other outbound channels.
- Target files:
  - `core/src/outbound_policy.rs`
  - future `core/src/integrations/slack.rs`
  - future `core/src/integrations/discord.rs`

## Do Not Copy As-Is

1. Full OpenClaw repo trees under archive
- Current archive location:
  - `_archive/deletable_candidates_20260215_191454/moltbot/openclaw`
- Reason: contains unrelated products/docs/packaging and increases merge/conflict surface.

2. Unscoped skills/commands that bypass local policy gate
- Requirement: all imported behavior must route through local approval + outbound + diagnostic policy layers.

## Acceptance Criteria For Any Future Import

1. No direct side-effect execution path without `approval_gate`.
2. Every retry path emits diagnostic event with attempt count.
3. Every outbound message path enforces policy and records evidence.
4. Scenario scripts (`run_nl_request_with_telegram.sh`, `run_complex_scenarios.sh`) parse new failure proofs deterministically.
5. Unit tests + script syntax checks + web build remain green.
