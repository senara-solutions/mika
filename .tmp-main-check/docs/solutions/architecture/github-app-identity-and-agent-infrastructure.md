---
title: GitHub App Identity and Agent Infrastructure
problem_type: architecture
component: task-engine, server, gateway, cli, config
severity: medium
tags:
  - github-app
  - per-agent-identity
  - process-management
  - multi-tenant
  - webhook-routing
  - cancel-task
symptoms:
  - Users cannot cancel running long-running tasks (only DB status update, no PID kill)
  - mika-dev and mika-qa share one GitHub App identity causing self-approval blocks
  - Gateway routes all GitHub webhooks to a single agent_base_url (no multi-tenant support)
  - No HTTP endpoint to cancel tasks from external systems
related_modules:
  - crates/mika-agent/src/tools/cancel_task.rs
  - crates/mika-agent/src/task_engine/process_kill.rs
  - crates/mika-agent/src/task_engine/engine.rs
  - crates/mika-agent/src/server/handlers.rs
  - crates/mika-agent/src/server/mod.rs
  - crates/mika-agent/src/server/types.rs
  - crates/mika-cli/src/commands/tasks.rs
  - crates/mika-cli/src/commands/token.rs
  - crates/mika-cli/src/commands/credential_helper.rs
  - crates/mika-common/src/config.rs
  - crates/mika-gateway/src/github.rs
  - crates/mika-gateway/migrations/004_github_repos.sql
---

## Context

Four interconnected sub-tasks needed to complete the GitHub App integration story:

1. **Cancel task with PID kill (#410):** `cancel_task` tool only updated DB status without terminating the actual process. PIDs were stored but unused for cancellation.

2. **Per-agent GitHub App identity (#422):** mika-dev and mika-qa shared one App identity (`mika-dev-bot`). GitHub blocks self-approval (`Review Can not approve your own pull request`), breaking the autonomous dev loop.

3. **Autonomous issue pickup (#416):** Agent needed `github_app_login` config to filter assignee matches on `issues.assigned` webhook events.

4. **Multi-tenant webhook routing (#411):** Gateway forwarded all GitHub webhooks to a single `agent_base_url`, blocking multi-customer deployments.

## Solution

### Process Kill Infrastructure

Extracted `cancel_task_and_kill()` shared helper in `task_engine/process_kill.rs`:
- Process group kill (`kill(-pid, SIGTERM)`) to handle child process trees
- 5-second grace period, then SIGKILL if still alive
- `/proc/{pid}/cmdline` existence check before kill to mitigate PID reuse risk
- Idempotent — killing already-dead processes returns ESRCH (harmless)
- Terminal-state guard prevents double-cancel

Three convergent cancel paths:
- **Tool:** `cancel_task` tool calls `cancel_task_and_kill()`
- **HTTP:** `POST /tasks/{id}/cancel` endpoint (internal token auth, 200 sync)
- **CLI:** `mika tasks cancel <id>` subcommand

### Per-Agent GitHub App Credentials

**Config:** Per-agent `.env` files at `~/.mika/agents/{name}/.env` with same `MIKA_GITHUB_APP_*` vars. `Settings::load_for_agent()` already supported per-agent env loading.

**Fallback chain:** Per-agent App → global App → PAT → no auth.

**Token cache:** Per-agent at `{agent_home}/github_app_token.json` — prevents cross-agent corruption.

**New setting:** `MIKA_GITHUB_APP_LOGIN` (optional) — bot login string (e.g., `mika-dev[bot]`) for assignee filtering in autonomous issue pickup.

**CLI:** `mika token github --agent <name>` and `mika credential-helper` both accept `--agent` to resolve agent-specific App credentials.

### Multi-Tenant GitHub Webhook Routing

**Gateway migration 004:** `github_repos` table maps `repo_full_name` → `customer_id`.

**Routing logic in `resolve_github_container_url()`:**
1. Extract `repository.full_name` from webhook payload
2. Look up `github_repos` → `customer_id`
3. If found → construct FQDN via `container_url()`
4. If not found and `agent_base_url` set → fall back (single-tenant)
5. If neither → drop with warning log

## Key Design Decisions

1. **Shared kill helper, not duplicate logic:** `cancel_task_and_kill()` used by tool, HTTP handler, and CLI — single implementation, three entry points.

2. **Per-agent `.env` files, not new config schema:** Reuses existing `Settings::load_for_agent()` pattern. Same env var names, different files.

3. **Single HMAC secret across installations:** Acceptable for single-org deployments. Per-customer webhook secrets deferred to future.

4. **Gateway-level routing, agent-level filtering:** Gateway routes by repo, agent filters by assignee login — separation of concerns.

## Gotchas for Future Work

- **Process group kill on macOS:** `kill(-pid, sig)` requires the calling process to be in the same session. Works in containerized Linux deployments.
- **PID reuse window:** `/proc/{pid}/cmdline` check is best-effort, not atomic. The window is microseconds — acceptable for task cancellation use case.
- **Two separate GitHub Apps required:** mika-dev and mika-qa need distinct App registrations (not installations of the same App) to solve self-approval.
- **`github_repos` table requires manual population:** No admin API yet — insert rows directly into Postgres for multi-tenant setup.
