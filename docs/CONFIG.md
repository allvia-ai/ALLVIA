# Configuration Reference

This document summarizes the optional environment variables introduced across phases.

## Core Safety & Execution
- `SHELL_ALLOWLIST` / `SHELL_DENYLIST`: Comma-separated allow/deny rules for shell commands.
- `SHELL_ALLOW_COMPOSITES`: Allow composite shell operators (`&&`, `||`, `;`). Default `false`.
- `SHELL_ALLOW_SUBSTITUTION`: Allow command substitution (`$()`/`` `...` ``). Default `false`.
- `TOOL_ALLOWLIST` / `TOOL_DENYLIST`: Tool-level allow/deny rules (supports `ui.*`, `shell.exec`, `*`).

## Context Pruning
- `CONTEXT_PRUNE_MAX_MESSAGES`: Max chat history messages to pass to the LLM (default `8`).
- `CONTEXT_PRUNE_TTL_SECONDS`: Drop messages older than this TTL (disabled by default).

## Project Scanner
- `PROJECT_SCAN_MAX_FILES`: Max files to list (default `200`).
- `PROJECT_SCAN_MAX_FILE_SIZE`: Max bytes to include for key files (default `20000`).
- `PROJECT_SCAN_IGNORED_DIRS`: Comma-separated ignored directories.
- `KEY_FILE_NAMES`: Comma-separated list of key files to include.

## Runtime Verification
- `RUN_BACKEND_PORT`: Optional default backend port (API request can override).
- `RUN_FRONTEND_PORT`: Optional default frontend port (API request can override).

## Performance Verification
- `PERF_MAX_FILES`: Max file count threshold (default `300`).
- `PERF_MAX_CODE_BYTES`: Max code bytes threshold (default `5_000_000`).
- `PERF_MAX_DEPS`: Max dependency count threshold (default `200`).

## Replanning
- `EXECUTOR_MAX_REPLANS`: Max replans per goal (default `1`).
- `EXECUTOR_MAX_RETRIES`: Max retries per step (default `2`).

## Chat Gate (optional)
- `CHAT_GATE_ENABLED`: Enable channel gating (default `false`).
- `CHAT_REQUIRE_MENTION`: Require mention (default `false`).
- `CHAT_ALLOWED_CHANNELS`: Allowed channels (comma-separated).
- `CHAT_ALLOWED_CHAT_TYPES`: Allowed chat types (comma-separated).
- `CHAT_ALLOWED_SENDERS`: Allowed senders (comma-separated).

## NL Automation
- `STEER_NL_SESSION_TTL_SECONDS`: NL session TTL in seconds (default `3600`). Set `0` to disable cleanup.
- `STEER_APPROVAL_REQUIRE_MEDIUM`: Require approval for medium-risk actions (default `false`).

## Notifications
- `NOTIFY_POLICY_RULES`: JSON rules for notification gating (send_policy).

## n8n Runtime
- `STEER_N8N_RUNTIME`: n8n runtime mode (`docker` | `npx` | `manual`). Default `docker`.
- `STEER_N8N_AUTO_START`: Auto-start n8n when unreachable. Docker mode default `true`.
- `STEER_N8N_COMPOSE_FILE`: Absolute path to docker-compose file used in docker mode.
- `STEER_N8N_ALLOW_CLI_FALLBACK`: Allow CLI import fallback when API fails. Default `false` in docker mode, `true` in npx mode.
