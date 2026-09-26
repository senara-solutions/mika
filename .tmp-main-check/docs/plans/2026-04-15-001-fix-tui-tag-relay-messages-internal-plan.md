---
title: "fix: Tag mika-ask relay messages as internal using existing --task-id flag"
type: fix
status: active
date: 2026-04-15
---

# fix: Tag mika-ask relay messages as internal using existing --task-id flag

## Overview

PilotEvent relay messages sent via `mika ask --agent mika-dev --task-id <uuid> "message"` are visible in the TUI inbox because they are never tagged `internal: true`. The TUI inbox filtering (introduced in #494 / PR #552) works correctly — the messages just aren't tagged. This fix threads the `internal` flag through `AgentParams` using the existing `--task-id` / `--task-complete` signal, requiring no new CLI flags.

## Problem Frame

The autonomous dev loop sends relay messages through `mika ask --task-id <uuid>`. These are agent-to-agent traffic that should be hidden from the human-facing TUI inbox. Two save paths hardcode `internal: false`:
1. User message save (`agent.rs:1137`) — uses `save_message()` which defaults to `internal = 0`
2. Assistant response save (`agent.rs:891`) — uses `save_message_with_metadata(..., false)`

The signal for "this is a relay" already exists: `--task-id` present WITHOUT `--task-complete` means relay/correlation. With `--task-complete` means callback delivery (should remain visible).

## Requirements Trace

- R1. `mika ask --task-id <uuid> "message"` saves both user and assistant messages with `internal: true`
- R2. `mika ask --task-id <uuid> --task-complete "result"` does NOT tag as internal
- R3. `mika ask "message"` (no task-id) does NOT tag as internal
- R4. PilotEvent relay messages are hidden in TUI inbox mode
- R5. Callback results remain visible in TUI

## Scope Boundaries

- No new CLI flags — the `--task-id` / `--task-complete` combination is the signal
- No schema changes — `messages.internal` column already exists (schema v22)
- No TUI filtering changes — inbox mode filtering already works correctly
- No changes to `save_message()` or `save_message_with_metadata()` signatures

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/delegate_task.rs:264,305` — reference implementation that already saves messages with `internal: true`
- `crates/mika-agent/src/agent.rs:1068` — `AgentParams` struct (24 fields, no `internal` yet)
- `crates/mika-agent/src/agent.rs:552` — `run_loop()` takes 15 individual params, called from 3 sites
- `crates/mika-agent/src/agent.rs:891` — assistant response save with hardcoded `false`
- `crates/mika-agent/src/agent.rs:1137` — user message save via `save_message()` (no internal param)
- `crates/mika-agent/src/agent.rs:1489` — max-steps continuation save with hardcoded `false`

### Institutional Learnings

- **#494 internal-message-tagging**: `internal` flag set by server-side code only, never exposed in tool input. `save_message_with_metadata()` accepts `internal: bool`. `save_message()` omits the column (defaults to 0).
- **#358 task-id-correlation**: `AgentParams` propagation pattern — add field, default `false` at all construction sites, set from CLI. Audit with grep after implementation.

## Key Technical Decisions

- **`bool` not `Option<bool>`**: The field has a clear default (`false`), no need for `Option` wrapper. Matches the existing `internal: bool` parameter in `save_message_with_metadata()`.
- **Thread `internal` through `run_loop`**: `run_loop()` takes individual params (not `AgentParams`). Adding `internal: bool` as a new parameter is consistent with the existing pattern. Silent and team callers pass `false`.
- **Replace `save_message` with `save_message_with_metadata` for user message**: At line 1137, switch from `save_message()` to `save_message_with_metadata()` to pass the `internal` flag. The metadata parameter can be `None` — only the `internal` flag matters here.

## Implementation Units

- [x] **Unit 1: Add `internal` field to AgentParams and set it in all construction sites**

**Goal:** Thread the `internal: bool` field through `AgentParams` so it's available during the agent loop.

**Requirements:** R1, R2, R3

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (AgentParams struct)
- Modify: `crates/mika-cli/src/commands/ask.rs` (set `internal: task_id.is_some() && !task_complete`)
- Modify: `crates/mika-cli/src/commands/chat.rs` (2 sites, both `false`)
- Modify: `crates/mika-agent/src/server/handlers.rs` (set `false`)
- Modify: `crates/mika-agent/src/server/a2a.rs` (set `false`)
- Modify: `crates/mika-agent/tests/eval/harness.rs` (set `false`)

**Approach:**
- Add `pub internal: bool` to `AgentParams`
- In `ask.rs`, compute `internal: task_id.is_some() && !task_complete` when building `AgentParams`
- All other construction sites set `internal: false`
- Note: the `--task-complete` path in `ask.rs` returns early before constructing `AgentParams`, so the `!task_complete` guard is defense-in-depth

**Patterns to follow:**
- `AgentParams` field additions from #358 (`correlated_task_id`)
- Existing boolean fields in `AgentParams` (e.g., `cli_mode`)

**Test expectation:** None for this unit alone — behavioral verification in Unit 3.

**Verification:**
- `cargo build` succeeds with no errors
- `grep -n 'internal:' crates/mika-agent/src/agent.rs crates/mika-cli/src/commands/ask.rs crates/mika-cli/src/commands/chat.rs crates/mika-agent/src/server/handlers.rs crates/mika-agent/src/server/a2a.rs crates/mika-agent/tests/eval/harness.rs` shows the field set at every construction site

- [x] **Unit 2: Thread `internal` through `run_loop` and use it in message save paths**

**Goal:** Pass `params.internal` to all three message save sites so relay messages are persisted with `internal: true`.

**Requirements:** R1, R4

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/agent.rs` (run_loop signature, 3 call sites, 3 save sites)

**Approach:**
- Add `internal: bool` parameter to `run_loop()` function signature
- Update 3 call sites of `run_loop()`:
  - `run_agent_inner()` (~line 1466): pass `params.internal`
  - `run_silent_inner()` (~line 2181): pass `false`
  - `run_team_agent_inner_impl()` (~line 2481): pass `false`
- Update 3 message save sites:
  - User message (~line 1137): switch from `save_message()` to `save_message_with_metadata()` with `params.internal`
  - Assistant EndTurn (~line 891 inside `run_loop`): replace hardcoded `false` with the new `internal` parameter
  - Max-steps continuation (~line 1489): replace hardcoded `false` with `params.internal`

**Patterns to follow:**
- `delegate_task.rs:264,305` — reference for `save_message_with_metadata(..., internal: true)`
- Existing parameter threading in `run_loop()`

**Test expectation:** None for this unit alone — behavioral verification in Unit 3.

**Verification:**
- `cargo build` succeeds
- No remaining hardcoded `false` for `internal` in the three save paths

- [x] **Unit 3: Add tests for internal message tagging**

**Goal:** Verify relay messages are tagged internal and non-relay messages are not.

**Requirements:** R1, R2, R3, R5

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/mika-agent/tests/eval/harness.rs` (add `internal` setter if needed)
- Create or Modify: `crates/mika-agent/tests/eval/internal_tagging.rs` or inline in existing eval test file
- Test: `crates/mika-agent/tests/eval/`

**Approach:**
- Use `EvalHarness` with `MockLlmProvider` to exercise `run_agent()` with different `AgentParams.internal` values
- Query the DB after agent run to verify `messages.internal` column values

**Test scenarios:**
- Happy path: `AgentParams { internal: true, .. }` — both user message and assistant response have `internal = 1` in DB
- Happy path: `AgentParams { internal: false, .. }` (default) — both messages have `internal = 0` in DB
- Integration: Verify `load_recent_messages_filtered(limit, true)` excludes internal messages and `load_recent_messages_filtered(limit, false)` includes them

**Verification:**
- `cargo test -p mika-agent --test eval` passes
- `cargo test -p mika-agent` passes (no regressions)

## System-Wide Impact

- **Interaction graph:** Only `ask.rs` → `AgentParams` → `run_agent()` → `run_loop()` → `save_message_with_metadata()` path is affected. No callbacks, middleware, or observers touched.
- **Error propagation:** No new error paths — `save_message_with_metadata()` already exists and handles errors identically to `save_message()`.
- **State lifecycle risks:** None — the `internal` column already exists, default is `0`, no migration needed.
- **API surface parity:** HTTP `/message` handler and A2A handler both set `internal: false`, preserving existing behavior.
- **Unchanged invariants:** TUI inbox filtering logic, `save_message()` signature, `save_message_with_metadata()` signature, schema v22, watermark advancement logic.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Missing an AgentParams construction site | Grep audit after implementation per #358 learnings |
| `run_loop` has 3 callers — must update all | Compiler enforces (new required param) |

## Sources & References

- Related issue: #557
- Related PRs: #552 (internal message tagging, schema v22)
- Parent feature: #494
- Institutional learnings: `docs/solutions/architecture-patterns/internal-message-tagging-tui-inbox-mode.md`, `docs/solutions/architecture-patterns/task-id-correlation-intermediate-calls.md`
