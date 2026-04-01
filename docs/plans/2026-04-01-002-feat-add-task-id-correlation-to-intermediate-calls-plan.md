---
title: "feat: Add task-id correlation to intermediate long-running skill calls"
type: feat
status: active
date: 2026-04-01
origin: docs/brainstorms/2026-03-31-task-correlation-on-intermediate-calls-brainstorm.md
issue: 358
---

# Add task-id correlation to intermediate long-running skill calls

## Overview

When a long-running skill (e.g., claude-pilot) runs, it may issue multiple intermediate `mika ask` calls (permission requests, status queries) before the final completion callback. Today, `--task-id` on `mika ask` means "complete this task" — so intermediate calls cannot carry the task ID without triggering completion. This creates an observability blind spot: you can't answer "what happened during task X?" without manually correlating timestamps.

This plan splits `--task-id` into a correlation-only default and adds `--task-complete` as an explicit completion signal. Intermediate calls record the task ID in session metadata and trace spans for observability without modifying the task state.

## Problem Statement

- **Today:** `mika ask --task-id <uuid>` = complete this callback task with the message as the result
- **Problem:** Intermediate interactions (claude-pilot `canUseTool` permission relays) arrive as `mika ask --agent mika-dev` with **no task correlation** — invisible orphans in traces and dashboard
- **Root cause:** `--task-id` overloads two concepts: "which task this relates to" (correlation) and "this task is done" (completion)

## Proposed Solution

Split the semantic:

```bash
# Intermediate call — correlate only, run full agent loop
mika ask --task-id $TASK_ID --agent mika-dev -- "approve Edit on src/main.rs?"

# Final call — complete the task (existing behavior + explicit flag)
mika ask --task-id $TASK_ID --task-complete --agent mika-dev -- "$RESULT"
```

- **`--task-id` without `--task-complete`:** Record task_id in session metadata and trace context. Run the full agent loop normally. Do not modify the task row.
- **`--task-id` with `--task-complete`:** Existing completion path — validate callback task, `update_task_completed()`, sibling reconciliation, early return.
- **`--task-complete` without `--task-id`:** Clap validation error (`requires("task_id")`).

## Technical Approach

### Phase 1: CLI flag split (`crates/mika-cli/`)

**File: `crates/mika-cli/src/cli.rs`**

Add `--task-complete` flag to `AskArgs`:

```rust
/// Signal that the task should be marked as completed (requires --task-id)
#[arg(long, requires = "task_id")]
pub task_complete: bool,
```

**File: `crates/mika-cli/src/commands/ask.rs`**

Split the existing `--task-id` handler (currently lines 89-153) into two paths:

1. **Completion path** (when both `--task-id` and `--task-complete`): Keep existing early-return logic — validate callback task, `update_task_completed()`, `try_complete_parent_on_sibling_done()`, end session, return. Keep the 100KB result size limit on this path only.

2. **Correlation path** (when `--task-id` without `--task-complete`): Remove the early-return. Instead:
   - Validate the task exists (existence check only — no trigger_type or status validation, since intermediate calls should work regardless of task state)
   - Store `task_id` in session metadata via `create_session_with_metadata()` with `{"task_id": "<uuid>"}`
   - Pass `task_id` into `AgentParams` as a new `Option<String>` field for trace threading
   - Run the full agent loop (same as the no-task-id path)
   - Do NOT set `is_callback_turn` or `is_task_context` — intermediate calls are regular conversation turns

3. **Move the 100KB size limit** from the `task_id.is_some()` gate to the `task_complete` gate. Intermediate calls may carry large code diffs and should not be size-limited.

4. **Deprecation bridge:** When `--task-id` is used without `--task-complete` and the task has `trigger_type='callback'` + `status IN ('pending', 'in_progress')`, emit a stderr warning:
   ```
   [mika] WARNING: --task-id without --task-complete no longer completes callback tasks. Add --task-complete to preserve completion behavior.
   ```
   This gives existing callers a clear signal during the transition window. Log the warning at `warn!` level for server-side visibility.

### Phase 2: AgentParams threading (`crates/mika-agent/`)

**File: `crates/mika-agent/src/agent.rs`**

Add `correlated_task_id: Option<String>` to `AgentParams`:

```rust
pub correlated_task_id: Option<String>,
```

At `run_agent()` entry, if `correlated_task_id` is `Some`, set it as a span attribute on the `agent_turn` tracing span:

```rust
if let Some(ref task_id) = params.correlated_task_id {
    tracing::Span::current().record("correlated_task_id", task_id.as_str());
}
```

This enables OTel/Langfuse queries by task_id. The `correlated_task_id` field follows the same `Option<String>` propagation pattern as `trace_id` (see brainstorm: docs/brainstorms/2026-03-31-task-correlation-on-intermediate-calls-brainstorm.md).

### Phase 3: Session metadata storage

**File: `crates/mika-cli/src/commands/ask.rs`**

When `--task-id` is present (correlation mode), use `create_session_with_metadata()` instead of `create_session()`:

```rust
let metadata = task_id.as_ref().map(|tid| format!(r#"{{"task_id":"{}"}}"#, tid));
// Use create_session_with_metadata when metadata is present
```

When `--session-id` reuses an existing session, update the session metadata if it doesn't already contain a `task_id`. This handles the pattern where the first call in a session establishes correlation.

**Dashboard query support:** Add optional `task_id` query parameter to `GET /api/v1/sessions` that filters using `json_extract(metadata, '$.task_id')`. No schema migration needed — the metadata column already exists.

### Phase 4: JSON output extension

**File: `crates/mika-cli/src/commands/ask.rs`**

Add `task_id` to `AskJsonResponse`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub task_id: Option<String>,
```

Populated from the `--task-id` argument when `--format json` is used. Echoes back the correlated task ID for caller confirmation.

### Phase 5: Skill handler updates (`mika-skills/`)

**Breaking change migration:** All skill handler scripts that call `mika ask --task-id` for completion must add `--task-complete`. These must be updated in the same PR to avoid silent breakage.

**File: `mika-skills/claude-pilot/handlers/run.sh`** (in mika-skills repo)

- Final delivery call: add `--task-complete` flag
  ```bash
  # Before: mika ask --task-id "$TASK_ID" --agent "$AGENT" -- "$RESULT"
  # After:  mika ask --task-id "$TASK_ID" --task-complete --agent "$AGENT" -- "$RESULT"
  ```

- Intermediate relay calls: add `--task-id "$TASK_ID"` (where `TASK_ID` = `__mika_task_id` from env)
  ```bash
  mika ask --task-id "$TASK_ID" --agent "$AGENT" -- "$PERMISSION_QUESTION"
  ```

**Note:** The claude-pilot relay transport changes (threading task-id through `canUseTool` callbacks) are in the `claude-pilot/` repo and are out of scope for this PR. They can be a follow-up since the CLI protocol is the bottleneck today.

### Phase 6: Dashboard API filter

**File: `crates/mika-agent/src/server/handlers.rs`**

Add `task_id: Option<String>` to `SessionsQuery` params. When present, filter with:

```sql
AND json_extract(metadata, '$.task_id') = ?
```

This enables the dashboard to show "all sessions correlated to task X" without a schema migration.

## System-Wide Impact

- **Interaction graph:** `mika ask --task-id X` → session created with metadata → agent loop runs → LLM call spans carry `correlated_task_id` attribute → dashboard can query by task_id. No callbacks, middleware, or observers affected.
- **Error propagation:** Task existence validation failure returns a user-facing error via `bail!()`. No retry needed — the caller should fix the task ID.
- **State lifecycle risks:** Intermediate calls do NOT modify the task row, so there is no partial-state risk. The only state written is session metadata (idempotent JSON field).
- **API surface parity:** `POST /tasks/{id}/complete` is unchanged. `POST /message` does not yet support task correlation (server-mode asymmetry — acceptable since claude-pilot uses CLI mode). Can be a follow-up.
- **Integration test scenarios:** (1) Intermediate call stores metadata → completion call works → dashboard shows both sessions. (2) Multiple intermediate calls with `--session-id` reuse → single session with task_id metadata. (3) `--task-complete` without `--task-id` → clap validation error.

## Acceptance Criteria

- [x] `--task-complete` flag added to `mika ask` with `requires("task_id")` constraint
- [x] `--task-id` without `--task-complete` stores task_id in session metadata and runs full agent loop
- [x] `--task-id` with `--task-complete` triggers existing completion path (backward compatible)
- [x] 100KB size limit applies only to `--task-complete` path, not correlation path
- [x] Deprecation warning emitted when `--task-id` targets a completable callback task without `--task-complete`
- [x] `correlated_task_id` field added to `AgentParams` and recorded as tracing span attribute
- [x] `AskJsonResponse` includes `task_id` field (skip_serializing_if none)
- [x] `GET /api/v1/sessions` supports `task_id` query parameter filtering via `json_extract`
- [ ] claude-pilot `run.sh` final delivery uses `--task-complete`
- [x] All existing tests pass; new tests cover both correlation and completion paths
- [x] `mika ask --task-complete` without `--task-id` produces a clap validation error

## Key Decisions

1. **No schema migration** — uses existing `sessions.metadata` JSON column for task_id storage. A dedicated column + index can be added later if query performance requires it.
2. **Existence-only validation** on intermediate calls — no trigger_type or status check. Avoids race conditions where intermediate calls fail after the task completes.
3. **`is_callback_turn` and `is_task_context` remain `false`** for intermediate calls — they are regular conversation turns with metadata annotation, not callback processing.
4. **Deprecation bridge over hard break** — warn on the old behavior pattern rather than silently changing semantics.
5. **Server-mode parity deferred** — `POST /message` does not yet support task correlation. Follow-up issue if needed.

## Dependencies & Risks

- **Risk: Partial rollout** — If CLI is updated but skill handler scripts are not, all long-running tasks silently stop completing. Mitigated by the deprecation warning and updating `run.sh` in the same PR.
- **Risk: mika-skills repo coordination** — The `claude-pilot/handlers/run.sh` change is in a different repo. Must be coordinated. For this PR, we update the script; the relay transport threading is a follow-up.
- **Dependency: clap** — `requires("task_id")` is standard clap; no version constraints.

## Sources & References

- **Origin brainstorm:** [docs/brainstorms/2026-03-31-task-correlation-on-intermediate-calls-brainstorm.md](docs/brainstorms/2026-03-31-task-correlation-on-intermediate-calls-brainstorm.md) — key decisions: `--task-id` becomes correlation-only, `--task-complete` boolean for completion, intermediate calls correlate only
- **Pattern precedent:** [docs/solutions/architecture-patterns/generic-callback-framing-parent-task-id.md](docs/solutions/architecture-patterns/generic-callback-framing-parent-task-id.md) — `Option<String>` field propagation pattern
- **Trace ID threading:** [docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md](docs/solutions/architecture-patterns/trace-id-structural-linkage-delegate-silent-callback.md) — `unwrap_or_else(generate)` idiom
- **CLI flag naming:** [docs/solutions/architecture-patterns/cli-flag-id-suffix-convention.md](docs/solutions/architecture-patterns/cli-flag-id-suffix-convention.md) — `--{noun}-id` for opaque identifiers
- Related issue: #358
