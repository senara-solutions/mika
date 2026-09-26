---
title: "feat: Expose process_id in check_task and list_tasks tool output"
type: feat
status: active
date: 2026-05-08
---

# feat: Expose process_id in check_task and list_tasks tool output

## Overview

Surface the existing `tasks.process_id` column in the agent-facing `check_task` and `list_tasks` tools so Mika can answer "what is the PID of the running session for task N?" using only the DB.

## Problem Frame

The `process_id` column already exists on the `tasks` table (since before v31), is populated at spawn time via `set_task_process_id()` in `executor.rs:1221`, and is cleared on exit via `clear_task_process_id()` in `engine.rs:301` and `process_kill.rs:176`. The callback watchdog (`check_callback_process_liveness`) already reads it for liveness detection. However, neither `check_task` nor `list_tasks` includes `process_id` in their output, so the agent has no query path to retrieve it. This is the sole remaining gap in mika#854's acceptance criteria.

## Requirements Trace

- R1. `check_task` output includes `process_id` when the field is non-NULL
- R2. `list_tasks` output includes `process_id` for tasks that have one (in_progress with active subprocess)
- R3. No schema migration needed — column already exists

## Scope Boundaries

- No new DB column, migration, or schema change
- No changes to the spawn-time write or exit-time clear paths (already working)
- Dashboard API `TaskResponse` parity is a separate concern (out of scope)
- `list_scheduled_tasks` tool is out of scope (scheduled tasks don't spawn subprocesses)
- This ticket is part of milestone#18 (Task lifecycle improvements); cancel-by-PID and revert-cleanup tickets are siblings that build on this exposure.

## Context & Research

### Relevant Code and Patterns

- `check_task` output format: `crates/mika-agent/src/tools/check_task.rs:220-295` — uses `writeln!` with conditional fields (Source, Reference, Completed, Metadata all use `if let Some(ref ...)` guards)
- `list_tasks` output format: `crates/mika-agent/src/tools/list_tasks.rs:129-172` — compact one-liner per task with optional annotations (ref_url, src, task_type, children) using `.map(|x| format!(...)).unwrap_or_default()` pattern
- `Task.process_id: Option<i64>` — field at `crates/mika-agent/src/db.rs:152`
- Existing test patterns: both files have inline `#[cfg(test)] mod tests` with `TestHarness` setup

### Institutional Learnings

- Per `feedback_smoke_before_claiming_done.md`: build + run + paste real output before claiming behavior
- Per the proactive state checking convention (CLAUDE.md): new write tools should have a corresponding query tool — this change closes that gap for `process_id`

## Key Technical Decisions

- **Conditional display only when set**: `process_id` is `None` for most tasks (only set on callback tasks with active subprocesses). Show it only when `Some` to avoid noise. Matches the `check_task` pattern for Source, Reference, Completed.
- **`list_tasks` uses compact annotation**: Follow the existing `ref:`, `src:`, `type:`, `children:` annotation pattern with `pid:` prefix. Only emitted when `process_id.is_some()`.
- **No "liveness" enrichment**: The tools surface the raw PID. Liveness checking (is the process still alive?) is the watchdog's job, not the query tool's.

## Implementation Units

- [ ] **Unit 1: Add process_id to `check_task` output**

  **Goal:** When `check_task` is called on a task with a non-NULL `process_id`, include it in the output.

  **Requirements:** R1

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-agent/src/tools/check_task.rs`

  **Approach:**
  - Add a conditional `writeln!` for `process_id` after the Metadata block and before the GitHub enrichment section, following the same `if let Some(ref ...)` pattern used for Source, Reference, Completed, and Metadata
  - Format: `Process ID: <pid>`

  **Patterns to follow:**
  - `check_task.rs:229-231` — Source field conditional display
  - `check_task.rs:239-241` — Completed field conditional display

  **Pinned insertion point** (verbatim from branch state at `feat/854/store-process-id-when-a-task-triggers-a` HEAD; lines may shift if main is rebased — re-grep `// GitHub enrichment` to relocate):

  ```rust
  // crates/mika-agent/src/tools/check_task.rs:243-251
          if let Some(ref metadata) = task.metadata
              && metadata != "{}"
          {
              writeln!(output, "Metadata: {metadata}").unwrap();
          }

          // GitHub enrichment
          if let Some(ref url) = task.reference_url {
              writeln!(output).unwrap();
  ```

  Insert the new block on the blank line between `}` (line 247) and `// GitHub enrichment` (line 249), so the resulting structure is:

  ```rust
          if let Some(ref metadata) = task.metadata
              && metadata != "{}"
          {
              writeln!(output, "Metadata: {metadata}").unwrap();
          }

          if let Some(pid) = task.process_id {
              writeln!(output, "Process ID: {pid}").unwrap();
          }

          // GitHub enrichment
  ```

  Note: `task.process_id` is `Option<i64>` (a `Copy` type) — use `if let Some(pid)` not `if let Some(ref pid)`.

  **Test scenarios:**
  - Happy path: task with `process_id = Some(12345)` → output contains `Process ID: 12345`
  - Happy path: task with `process_id = None` → output does NOT contain `Process ID`

  **Test setup mechanism:** Tests should set `process_id` via `db.set_task_process_id(&task_id, Some(12345)).await` (the production write path), not via raw SQL. This composes the production write path with the new read path so column-name regressions surface.

  **Verification:**
  - `cargo test -p mika-agent check_task` passes
  - New tests cover both Some and None cases

- [ ] **Unit 2: Add process_id to `list_tasks` output**

  **Goal:** When `list_tasks` displays a task with a non-NULL `process_id`, include it as a compact annotation.

  **Requirements:** R2

  **Dependencies:** None (can be done in parallel with Unit 1)

  **Files:**
  - Modify: `crates/mika-agent/src/tools/list_tasks.rs`

  **Approach:**
  - Add a `pid` annotation using the same `.map(|p| format!(" pid:{p}")).unwrap_or_default()` pattern used for `ref_url`, `src`, `task_type`, and `children`
  - Insert into the format string alongside existing annotations

  **Patterns to follow:**
  - `list_tasks.rs:134-138` — `ref_url` annotation pattern
  - `list_tasks.rs:139-143` — `src` annotation pattern

  **Pinned insertion point** (verbatim from branch state at `feat/854/store-process-id-when-a-task-triggers-a` HEAD; re-grep `format!(\n` inside the for-each task loop to relocate if shifted):

  ```rust
  // crates/mika-agent/src/tools/list_tasks.rs:151-160
              let children = child_count
                  .map(|c| format!(" children:{c}"))
                  .unwrap_or_default();

              lines.push(format!(
                  "- [{status}] {id} {label} (created:{created}{ref_url}{src}{task_type}{children})",
                  status = task.status,
                  id = task.id,
                  label = task.label,
              ));
  ```

  Add the new `pid` binding immediately after the `children` binding, then append `{pid}` at the END of the parenthesised annotation list inside the format string (after `{children}`). Result:

  ```rust
              let children = child_count
                  .map(|c| format!(" children:{c}"))
                  .unwrap_or_default();
              let pid = task
                  .process_id
                  .map(|p| format!(" pid:{p}"))
                  .unwrap_or_default();

              lines.push(format!(
                  "- [{status}] {id} {label} (created:{created}{ref_url}{src}{task_type}{children}{pid})",
                  status = task.status,
                  id = task.id,
                  label = task.label,
              ));
  ```

  Placement rationale: `pid` appended after `children` puts the rarest annotation last — most tasks have neither, so the common case (no annotations) stays compact. `process_id` is `Option<i64>` (a `Copy` type) — `.map(|p| ...)` works directly without `as_deref()` (unlike `ref_url`/`src` which are `Option<String>`).

  **Test scenarios:**
  - Happy path: task with `process_id = Some(42)` in `list_tasks` output → line contains `pid:42`
  - Happy path: task with `process_id = None` → line does NOT contain `pid:`
  - Integration: create a task, call `db.set_task_process_id(&task_id, Some(42)).await` (the production write path, not raw SQL), call `list_tasks`, verify annotation appears.

  **Test setup mechanism:** Tests should set `process_id` via `db.set_task_process_id(&task_id, Some(42)).await`, not via raw SQL. This composes the production write path with the new read path so column-name regressions surface.

  **Verification:**
  - `cargo test -p mika-agent list_tasks` passes
  - New tests cover both Some and None cases

## System-Wide Impact

- **API surface parity:** Dashboard `TaskResponse` does not include `process_id` — this is a known gap but out of scope for this ticket. Could be a follow-up.
- **Unchanged invariants:** No changes to `set_task_process_id`, `clear_task_process_id`, the watchdog, or the `list_scheduled_tasks` tool.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| None significant — additive display-only change | Conditional emission means no output change for the vast majority of tasks |

## Sources & References

- Related issue: senara-solutions/mika#854
- Spawn site: `crates/mika-agent/src/skills/executor.rs:1219-1223`
- Clear site: `crates/mika-agent/src/task_engine/engine.rs:301`
- Watchdog: `crates/mika-agent/src/task_engine/engine.rs:411+`
