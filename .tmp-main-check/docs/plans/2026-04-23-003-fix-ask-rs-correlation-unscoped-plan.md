---
title: "fix: Swap ask.rs correlation branch from agent-scoped to unscoped task lookup"
type: fix
status: active
date: 2026-04-23
issue: 752
---

# fix: Swap ask.rs correlation branch from agent-scoped to unscoped task lookup

## Overview

The `mika ask --task-id` correlation-only path uses `get_task()` (agent-scoped) when it should use `get_task_unscoped()`. This causes cross-agent relay failures when mika-relay correlates tasks owned by mika-dev. One-line swap plus bidirectional doc comments and a regression test.

## Problem Frame

`ask.rs:183` calls `ctx.async_db.get_task(tid)` inside a code path explicitly labeled "correlation-only." The intent is an existence check for observability metadata — not ownership validation. `get_task()` enforces `WHERE agent_id = ?`, so when mika-relay (the caller) differs from the task owner (mika-dev), the lookup returns `None` and bails with "Task not found."

Exposed by the claude-pilot routing fix (commit `8b6df69`) that correctly routed relay to mika-relay per #721's intent. 65+ relay denials in production logs. Milestone #16 is blocked.

## Requirements Trace

- R1. Correlation path at `ask.rs:183` must use unscoped task lookup
- R2. Doc comment on `validate_task_exists` must cross-link `get_task_unscoped` with ownership-vs-correlation distinction
- R3. Reciprocal doc comment on `get_task_unscoped` must name `validate_task_exists` and list callers
- R4. Regression test: task owned by agent A, correlation path invoked as agent B, must succeed

## Scope Boundaries

- The `--task-complete` path at line 129 correctly uses agent-scoped `get_task()` — it mutates task state and ownership check is correct there
- `validate_task_exists` in `tools/mod.rs` is correctly agent-scoped for its consumers — no change
- No architectural changes, no new helpers, no enum parameters

## Context & Research

### Relevant Code and Patterns

- `crates/mika-cli/src/commands/ask.rs:183` — the one-line fix site
- `crates/mika-agent/src/async_db.rs:1624` — `get_task_unscoped()` already exists on `AsyncDatabase`
- `crates/mika-agent/src/db.rs:7122` — `Database::get_task_unscoped()` — no agent_id parameter
- `crates/mika-agent/src/tools/mod.rs:311` — `validate_task_exists()` — agent-scoped, correct for its callers
- Existing test patterns: `TestHarness::with_agent("agent-a")` for cross-agent test setup (see `tools/mod.rs:914`)

### Institutional Learnings

- Cross-agent correlation is a known pattern — `get_task_unscoped` was introduced for the dashboard and is documented as "without agent_id scoping"

## Key Technical Decisions

- **Use `get_task_unscoped` not a new helper:** The primitive already exists and has the exact semantics needed — no agent_id filter, returns `Option<Task>`
- **Keep completion path agent-scoped:** Line 129's `get_task()` is correct — completing a task is a state mutation that requires ownership

## Implementation Units

- [ ] **Unit 1: Swap to unscoped lookup + doc comments**

**Goal:** Fix the correlation path and add bidirectional documentation

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Modify: `crates/mika-cli/src/commands/ask.rs`
- Modify: `crates/mika-agent/src/tools/mod.rs`
- Modify: `crates/mika-agent/src/db.rs`

**Approach:**
- Replace `ctx.async_db.get_task(tid)` with `ctx.async_db.get_task_unscoped(tid)` at line 183
- Add doc comment above `validate_task_exists` (~line 310 in tools/mod.rs) naming the ownership-vs-correlation distinction and pointing to `Database::get_task_unscoped`
- Add reciprocal doc comment above `get_task_unscoped` (~line 7122 in db.rs) pointing back to `validate_task_exists` and listing current callers

**Patterns to follow:**
- Existing doc comment style in `tools/mod.rs` (see lines 300-310)
- `get_task_unscoped` already has a one-line doc comment — extend it

**Test expectation:** none — behavioral verification is in Unit 2

**Verification:**
- `cargo clippy -p mika-cli -p mika-agent` clean
- `cargo build -p mika-cli` succeeds

- [ ] **Unit 2: Cross-agent correlation regression test**

**Goal:** Prove the correlation path works across agent boundaries

**Requirements:** R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-cli/src/commands/ask.rs` (test module)

**Approach:**
- Create an `AsyncDatabase` with agent "agent-a", seed a callback task
- Create a second `AsyncDatabase` with agent "agent-b"
- Call `get_task_unscoped(tid)` on the agent-b database — assert `Ok(Some(task))` and task fields match
- Verify agent-scoped `get_task(tid)` on agent-b returns `Ok(None)` — proves the distinction

**Patterns to follow:**
- `TestHarness::with_agent("agent-a")` pattern from `tools/mod.rs:916`
- Existing async test pattern in `tools/mod.rs` tests
- Note: ask.rs tests currently only test serialization (non-async `#[test]`). The new test will be the first `#[tokio::test]` in this module — may need to add `use mika_agent::...` imports

**Test scenarios:**
- Happy path: task owned by agent-a, `get_task_unscoped` from agent-b context returns `Ok(Some(task))` with correct label and id
- Contrast: same task, agent-scoped `get_task` from agent-b context returns `Ok(None)` — confirms the fix is necessary
- Edge case: `get_task_unscoped` with non-existent UUID returns `Ok(None)` (not an error)

**Verification:**
- `cargo test -p mika-cli` passes including the new test
- `cargo test -p mika-agent` passes (no regressions)

## System-Wide Impact

- **Interaction graph:** Only the correlation path in `ask.rs` is affected. The completion path (line 129) and all `validate_task_exists` callers in tools remain agent-scoped
- **Error propagation:** The `Ok(None)` bail at line 200-202 still fires for genuinely non-existent tasks — unscoped lookup doesn't suppress real errors
- **Unchanged invariants:** All agent-scoped task operations (`update_task_status`, `cancel_task`, `complete_task`) continue to enforce ownership via `validate_task_exists`

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Unscoped lookup reveals task existence across agents | Correlation path only logs/warns — no task data returned to external callers. Same exposure as dashboard endpoint which already uses `get_task_unscoped` |

## Sources & References

- Related issue: #752
- Related PRs: #751 (shipped despite relay denials), #721 (introduced mika-relay)
- Related commits: `8b6df69` (claude-pilot routing fix that exposed the latent bug)
