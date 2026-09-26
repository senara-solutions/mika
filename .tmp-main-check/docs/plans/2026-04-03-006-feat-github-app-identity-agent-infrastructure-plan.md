---
title: "feat: GitHub App identity and agent infrastructure"
type: feat
status: completed
date: 2026-04-03
---

# GitHub App Identity and Agent Infrastructure

## Overview

Infrastructure work for separate agent identities and autonomous operation. This umbrella covers four interconnected sub-tasks that complete the GitHub App integration story:

1. **#410** — Cancel running long-running task via PID kill
2. **#422** — Separate GitHub App identities for mika-dev and mika-qa
3. **#416** — Autonomous issue pickup (assignment-based triggering)
4. **#411** — Multi-tenant GitHub webhook routing (repo-to-customer mapping)

The acceptance criteria states "one branch, one PR per sub-task" — these are independent enough to land separately. This plan covers all four with implementation phases.

## Problem Statement / Motivation

1. **Cancel task (#410):** Users cannot cancel running long-running tasks. The `cancel_task` tool only updates DB status without killing the process. PIDs are already stored but never used for user-facing cancellation.

2. **Separate App identities (#422):** mika-dev and mika-qa share one GitHub App identity (`mika-dev-bot`). GitHub blocks self-approval: `Review Can not approve your own pull request`. mika-qa falls back to `--comment` instead of `--approve`, which doesn't satisfy branch protection. This blocks the autonomous dev loop.

3. **Autonomous pickup (#416):** mika-dev only works when explicitly told. Assignment-based triggering via `issues.assigned` webhook is the simplest and most secure mechanism — only repo collaborators can assign, providing built-in authorization.

4. **Multi-tenant routing (#411):** Gateway routes all GitHub webhooks to a single `agent_base_url`. Multi-tenant mika-cloud deployment requires per-customer routing based on repository ownership.

## Proposed Solution

### Phase 1: Cancel Task with PID Kill (#410)

Extend the existing `cancel_task` tool and add HTTP + CLI cancel paths with actual process termination.

#### Changes

**`crates/mika-agent/src/tools/cancel_task.rs`** — Add PID kill logic:
- After DB status update, check `task.process_id`
- If PID is present and process is alive, send SIGTERM
- Wait 5 seconds grace period, then SIGKILL if still alive
- Clear `process_id` in DB after kill
- Use process group kill (`kill -TERM -$pid`) to handle child process trees

**`crates/mika-agent/src/db.rs`** — Add `get_task_for_cancel(task_id, agent_id)` method:
- Returns task with `process_id` field
- Validates task is in cancellable state (`pending`, `in_progress`, `blocked`)
- Atomic status update to `cancelled`

**`crates/mika-agent/src/task_engine/engine.rs`** — Extract PID kill into shared helper:
- `kill_process(pid: i64, use_process_group: bool) -> Result<bool>` 
- Reuse in both `kill_orphan_processes()` and `cancel_task`
- Check `/proc/{pid}/cmdline` before killing to mitigate PID reuse risk

**`crates/mika-agent/src/server/handlers.rs`** — Add `POST /tasks/{id}/cancel`:
- Synchronous (200 with status), matching `/tasks/{id}/complete` pattern
- Internal token auth (same as `/tasks/{id}/complete`)
- Performs PID kill + DB update
- Returns `{ "status": "cancelled", "task_id": "..." }`

**`crates/mika-agent/src/server/mod.rs`** — Register the cancel route.

**`crates/mika-cli/src/commands/tasks.rs`** — Add `mika tasks cancel <id>`:
- Look up task, perform PID kill, update DB
- Print confirmation with task ID and label

#### Race Condition Handling

- PID kill is idempotent — killing an already-dead process returns ESRCH (harmless)
- Terminal-state guard in `cancel_task()` prevents double-cancel
- Background monitor's `kill_orphan_processes()` and manual cancel may race — the first to set terminal status wins (existing `AND status NOT IN (...)` guard)

#### Acceptance Criteria

- [x] `cancel_task` tool kills the process when PID is present
- [x] `POST /tasks/{id}/cancel` endpoint works with internal token auth
- [x] `mika tasks cancel <id>` CLI command works
- [x] Process group kill handles child process trees
- [x] Grace period (5s) between SIGTERM and SIGKILL
- [x] Process alive check before kill (PID reuse mitigation)

### Phase 2: Per-Agent GitHub App Identity (#422)

Enable each agent to have its own GitHub App credentials.

#### Design Decisions

- **Config mechanism:** Per-agent `.env` files at `~/.mika/agents/{agent_name}/.env` with the same `MIKA_GITHUB_APP_*` env var names. `Settings::load_for_agent()` already supports per-agent env loading — no new config schema needed.
- **Fallback chain:** Per-agent App → global App → PAT → no auth (graceful degradation)
- **Token cache:** Per-agent at `{agent_home}/github_app_token.json` instead of global `~/.mika/github_app_token.json`
- **Two separate GitHub App registrations** (not installations of the same App) — required to solve self-approval

#### Changes

**`crates/mika-common/src/github_app.rs`**:
- `installation_token_with_file_cache()` — accept `cache_dir: &Path` parameter instead of hardcoding `~/.mika/`. Cache at `{cache_dir}/github_app_token.json`
- Add `github_app_login` field derived from App slug (e.g., `mika-dev[bot]`) — fetched once at construction via GitHub API `GET /app` or configurable via `MIKA_GITHUB_APP_LOGIN` env var

**`crates/mika-common/src/config.rs`**:
- Add `github_app_login: Option<String>` to `Settings`
- Add `ConfigKeyInfo` entry for the new field
- Manual `Debug` impl already redacts — no changes needed (login is not sensitive)

**`crates/mika-agent/src/server/mod.rs`**:
- Multi-agent path (line 474): Replace single `global_github_app` with per-agent map `HashMap<String, Arc<GitHubApp>>`
- Each agent constructs its own `GitHubApp::from_settings(&agent_settings)` during agent loading
- `AgentState.github_app` already exists — just populate per-agent

**`crates/mika-agent/src/task_engine/dispatcher.rs`**:
- `TaskDispatcher.github_app` → resolve per-agent at dispatch time from the map

**`crates/mika-cli/src/commands/token.rs`** and **`credential_helper.rs`**:
- Accept `--agent` flag to select which agent's App to use
- Default to active agent

#### Backward Compatibility

- When no per-agent config exists, falls back to global `MIKA_GITHUB_APP_*` vars — existing single-App setups work unchanged
- `resolve_github_token()` signature unchanged — it already takes `Option<&GitHubApp>`
- Env var scrubbing: new per-agent `MIKA_*` vars are auto-scrubbed by prefix matching — no `EXTRA_SCRUB_VARS` changes needed

#### Acceptance Criteria

- [x] Per-agent GitHub App credentials via per-agent `.env` files
- [x] Per-agent file-based token cache (no cross-agent corruption)
- [x] Fallback to global App when per-agent config absent
- [x] `mika token github --agent <name>` resolves correct agent's token
- [x] Credential helper uses active agent's App
- [x] mika-qa can approve mika-dev's PRs (separate App identity, verified manually after App creation)

### Phase 3: Autonomous Issue Pickup (#416)

Enable mika-dev to auto-start work when assigned a GitHub issue.

#### Design Decisions

- **Trigger:** `issues.assigned` webhook (already routed to mika-dev by gateway)
- **Assignee filtering:** Agent-level, not gateway-level. The agent checks if the assignee login matches its `github_app_login` config
- **Work start mechanism:** The agent processes the issue assignment as a regular message. The self-dev skill prompt recognizes the format and invokes the development workflow
- **Busy handling:** If the agent is busy (429 from `try_lock`), the message is queued via existing retry mechanism

#### Changes

**`crates/mika-gateway/src/github.rs`**:
- `format_event_text()` for `issues.assigned` — ensure the formatted text includes the assignee login prominently so the agent can filter
- No routing changes needed — `issues.assigned` → `mika-dev` is already correct
- Remove stale CLAUDE.md references to `issues.opened` (documentation-only)

**Agent-side (prompt/skill changes, not code):**
- The self-dev skill prompt in `mika-skills/` should recognize issue assignment messages and auto-start the `/mika` workflow
- The agent checks `github_app_login` against the assignee in the message — if mismatch, responds "This issue was assigned to {assignee}, not me"
- Guard: if the agent already has an active work item for the same `reference_url` (the issue URL), skip with "Already working on this issue"

#### Edge Cases

- **Re-assignment:** Delivery UUID dedup prevents duplicate webhook processing. If unassigned and re-assigned, a new delivery UUID is generated — the agent deduplicates via `reference_url` on work items
- **Assigned to human:** Agent ignores — assignee login doesn't match `github_app_login`
- **Multiple assignments:** Gateway forwards each `issues.assigned` event separately. Agent creates one work item per unique issue URL

#### Acceptance Criteria

- [x] Agent recognizes issue assignment messages
- [x] Assignee filtering prevents processing assignments to others
- [x] Duplicate assignment detection via work item `reference_url`
- [x] Agent busy → message queued (existing behavior)

### Phase 4: Multi-Tenant GitHub Webhook Routing (#411)

Route GitHub webhooks to the correct customer container.

#### Design Decisions

- **Routing key:** `repository.full_name` from webhook payload (simpler than `installation.id`, directly maps to customer)
- **Fallback:** When no `github_repos` match, fall back to `agent_base_url` (backward-compatible single-tenant mode)
- **HMAC secret:** Single shared secret across all installations (acceptable for single-org deployments; per-customer secrets deferred to future)
- **Registration:** Manual DB insert initially; admin API endpoint in future

#### Changes

**`crates/mika-gateway/migrations/004_github_repos.sql`**:

```sql
CREATE TABLE github_repos (
    id SERIAL PRIMARY KEY,
    repo_full_name TEXT NOT NULL,
    customer_id UUID NOT NULL REFERENCES customers(id),
    created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE UNIQUE INDEX idx_github_repos_name ON github_repos(repo_full_name);
```

**`crates/mika-gateway/src/github.rs`**:
- Add `resolve_github_customer(pool, repo_full_name) -> Option<Uuid>` (analogous to Telegram's `resolve_customer()`)
- Update `forward_github_event()` to:
  1. Extract `repository.full_name` from event
  2. Look up `github_repos` → `customer_id`
  3. If found → `container_url(&customer_id, ...)` for FQDN routing
  4. If not found and `agent_base_url` set → fall back (single-tenant)
  5. If not found and no `agent_base_url` → drop with warning log

**`crates/mika-gateway/src/github.rs`** — Update `forward_github_event()` signature to accept `&PgPool`.

#### Security

- Unregistered repos are rejected at gateway level (no forwarding)
- HMAC validation prevents forged events regardless of repo registration
- `repo_full_name` unique index prevents duplicate registrations

#### Acceptance Criteria

- [x] `github_repos` table created via migration 004
- [x] Webhook events routed to correct customer container
- [x] Unregistered repos fall back to `agent_base_url` or are dropped
- [x] Existing single-tenant setups work unchanged (backward compatible)

## Technical Considerations

### Architecture Impacts

- **Per-agent `GitHubApp` map** replaces single global instance — affects server startup, `AgentState`, and `TaskDispatcher`
- **Shared kill helper** extracted from task engine — reused by tool, HTTP handler, and CLI
- **Gateway migration 004** — Postgres schema change, requires migration path for existing deployments

### Security Considerations

- Per-agent App credentials scrubbed from child processes via existing `MIKA_*` prefix matching
- PID reuse risk mitigated by `/proc/{pid}/cmdline` check before kill
- HMAC webhook validation unchanged — single shared secret per deployment
- Unregistered repos rejected at gateway (defense-in-depth alongside HMAC)

### Performance Implications

- Per-agent `GitHubApp` instances have independent token caches — no contention
- `github_repos` lookup adds one Postgres query per webhook event — negligible overhead
- PID kill adds a brief blocking wait (5s max) to cancel operations

## System-Wide Impact

- **Interaction graph:** Cancel tool → DB status update → PID kill → process group SIGTERM → child cleanup. Per-agent App → token resolution per turn → `ToolContext.github_token` → `run_gh`, context injection, work item enrichment
- **Error propagation:** PID kill errors (ESRCH, EPERM) are logged but non-fatal. GitHub App token exchange failure → graceful degradation to PAT → no auth
- **State lifecycle risks:** Cancel + background monitor race → terminal-state guard prevents double-update. Per-agent token cache → per-agent file path prevents corruption
- **API surface parity:** `cancel_task` tool, `POST /tasks/{id}/cancel`, `mika tasks cancel` — all three paths converge on the same kill + DB update logic

## Dependencies & Risks

| Risk | Mitigation |
|------|-----------|
| GitHub App creation is manual (external step) | Phase 2 code is backward-compatible; can deploy before creating the new App |
| PID reuse kills wrong process | `/proc/{pid}/cmdline` check before kill |
| Gateway migration breaks existing deployments | Migration is additive (new table); no existing table changes |
| Self-dev skill prompt changes needed for #416 | Separate PR in mika-skills repo; agent-side code works without it |

## Sources

- Issue #423: [umbrella: GitHub App identity and agent infrastructure](https://github.com/senara-solutions/mika/issues/423)
- Issue #422: [feat: separate GitHub App identities](https://github.com/senara-solutions/mika/issues/422)
- Issue #416: [investigate: autonomous issue pickup](https://github.com/senara-solutions/mika/issues/416)
- Issue #410: [feat: cancel running long-running task via PID kill](https://github.com/senara-solutions/mika/issues/410)
- Issue #411: [feat(gateway): multi-tenant GitHub webhook routing](https://github.com/senara-solutions/mika/issues/411)
- Solution: `docs/solutions/architecture-patterns/github-app-jwt-authentication-module.md`
- Solution: `docs/solutions/security-issues/gh-token-identity-collision-dotenv-leak.md`
- Solution: `docs/solutions/architecture-patterns/dedicated-github-token-agent-operations.md`
- Solution: `docs/solutions/architecture-patterns/github-webhook-endpoint-gateway.md`
- Solution: `docs/solutions/architecture-patterns/config-key-rename-across-layers.md`
- Existing code: `crates/mika-common/src/github_app.rs`, `crates/mika-gateway/src/github.rs`
- Existing todo: `todos/741-complete-p2-consolidate-github-app-instances.md`
