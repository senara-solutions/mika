---
title: "Async Callbacks Code Review Findings Batch (522-542)"
date: 2026-03-06
status: solved
category: code-review-patterns
component:
  - mika-agent/skills/executor
  - mika-agent/task_engine/dispatcher
  - mika-agent/task_engine/engine
  - mika-agent/server/handlers
  - mika-agent/teams/engine
  - mika-agent/teams/types
  - mika-agent/agent
  - mika-agent/db
  - mika-agent/prompt
severity: P1-P3 (4 critical, 10 important, 7 nice-to-have)
tags:
  - env-var-leakage
  - race-condition
  - missing-indexes
  - label-mismatch
  - typed-errors
  - query-correctness
  - stdout-collection
  - retry-backoff
  - code-duplication
  - checkpoint-versioning
  - sql-injection
  - orphan-processes
  - prompt-guidance
  - critic-rejection
symptoms:
  - MIKA_* env vars leaked to long-running subprocess children
  - Label mismatch on team resume (team-agent-analyst vs analyst)
  - Race condition leaving orphan task trees on synchronous team completion
  - Missing indexes on parent_task_id and created_by_session columns causing full table scans
  - String-matching on "agent is busy" error message for retry logic
  - Expired children query returning tasks from unrelated parents
  - Unbounded stdout collection from long-running processes
  - No retry backoff when agent is busy (immediate re-queue)
  - Sibling completion check duplicated across 3 call sites
  - Raw status string literals in server handlers
  - execute_from_phase ignoring next_phase parameter
  - Silent let _ = on child task completion errors
  - dispatch_skill_by_name silently succeeding when agent busy
  - No checkpoint version envelope for forward compatibility
  - LIKE pattern injection via session prefix query
  - Orphan processes not killed on task expiry
  - Team engine constructor duplication between new() and new_for_resume()
  - execute_from_phase duplicating finalization logic
  - SkillRun prompt framing too terse for agent guidance
  - No system prompt guidance for long-running tool behavior
  - Resume path not respecting critic rejection (review always accepted)
related_issues:
  - todos/522-complete-p1-env-vars-leaked-to-long-running-subprocess.md
  - todos/523-complete-p1-label-mismatch-in-team-resume.md
  - todos/524-complete-p1-race-condition-sync-team-run-task-tree.md
  - todos/525-complete-p1-missing-indexes-parent-task-id-session.md
  - todos/526-complete-p2-string-matching-agent-busy.md
  - todos/527-complete-p2-expired-children-query-returns-all.md
  - todos/528-complete-p2-unbounded-stdout-collection.md
  - todos/529-complete-p2-no-retry-backoff-agent-busy.md
  - todos/530-complete-p2-sibling-completion-check-duplicated.md
  - todos/531-complete-p2-raw-status-strings-in-handlers.md
  - todos/532-complete-p2-wire-next-phase-parameter.md
  - todos/533-complete-p2-silent-let-underscore-on-child-completion.md
  - todos/534-complete-p2-dispatch-skill-silently-succeeds-on-busy.md
  - todos/535-complete-p3-checkpoint-version-envelope.md
  - todos/536-complete-p3-like-pattern-injection-session-prefix.md
  - todos/537-complete-p3-orphan-processes-not-killed-on-expiry.md
  - todos/538-complete-p3-team-engine-new-for-resume-duplication.md
  - todos/539-complete-p3-execute-from-phase-duplicates-finalization.md
  - todos/540-complete-p3-skillrun-prompt-framing-terse.md
  - todos/541-complete-p3-no-system-prompt-for-long-running-skills.md
  - todos/542-complete-p2-execute-from-phase-respect-critic-rejection.md
related_docs:
  - docs/solutions/architecture/callback-resume-agent-lifecycle.md
  - docs/solutions/logic-errors/team-engine-code-review-findings-batch.md
  - docs/solutions/security-issues/env-var-leakage-exec-handler-child-processes.md
  - docs/solutions/database-issues/consolidate-per-agent-team-dbs-into-single-container-db.md
  - docs/solutions/code-review-patterns/background-agent-mode-design-checklist.md
---

# Async Callbacks Code Review Findings Batch (522-542)

## Problem Statement

A multi-agent code review of the `feat/unified-task-engine` branch identified 21 findings across the long-running skill executor, task engine dispatcher, team engine suspend/resume, server handlers, and agent loop. The review covered the "team-aware async callbacks with long_running skill flag" feature (44 files, ~2844 insertions).

Four findings were critical (security leak, data mismatch, race condition, missing indexes), ten were important (typed errors, query correctness, code duplication, retry logic, feature completeness), and seven were nice-to-have (versioning, process cleanup, prompt quality, code deduplication).

## Root Cause Analysis

### P1-522: MIKA_* Env Vars Leaked to Long-Running Subprocess

`spawn_long_running_exec` did not scrub MIKA_* environment variables before launching subprocesses. The existing `execute_exec` function had scrubbing logic but it was not shared with the new long-running path. Background scripts could access `MIKA_ANTHROPIC_API_KEY` and other secrets.

### P1-523: Label Mismatch in Team Resume

`dispatch_invoke_orchestrator` matched child task labels (e.g., `team-agent-analyst`) against agent assignment keys (e.g., `analyst`). The `team-agent-` prefix prevented matches, causing all child results to be silently dropped during team resume.

### P1-524: Race Condition on Synchronous Team Completion

When all team agents completed synchronously (no long_running tools), both the sibling-completion check and the team engine's own continuation path could fire the parent `invoke_orchestrator` task. The parent task tree was also left dangling — never cancelled when the team run completed without suspension.

### P1-525: Missing Indexes on parent_task_id and created_by_session

`try_complete_parent_on_sibling_done()` queries `WHERE parent_task_id = ?` and `count_pending_callback_tasks_by_session_prefix` queries `WHERE created_by_session LIKE ?` — both without indexes. Full table scans on every sibling check and team suspension detection.

### P2-526: String Matching on "Agent is Busy"

`dispatch()` returned `anyhow::Error` and callers matched on `err.to_string().contains("agent is busy")` — fragile string matching that breaks if the error message changes.

### P2-527: Expired Children Query Returns Unrelated Tasks

`get_expired_child_task_ids` selected expired tasks without joining on the parent task, returning tasks from unrelated parents.

### P2-528: Unbounded Stdout Collection

`spawn_long_running_exec` used `wait_with_output()`, buffering potentially gigabytes of stdout from long-running processes into memory.

### P2-529: No Retry Backoff for Busy Agent

When `dispatch_resume_agent` failed because the agent was busy, the task was immediately re-queued with no delay — creating a tight retry loop.

### P2-530: Sibling Completion Check Duplicated

Three call sites (engine.rs fire_task, server handlers.rs, CLI ask.rs) each independently called `try_complete_parent_on_sibling_done` + `dispatch` — identical 10-line blocks.

### P2-531: Raw Status Strings in Handlers

Server handlers used string literals like `"completed"`, `"pending"` instead of the `task_status::*` constants defined in `types.rs`.

### P2-532: execute_from_phase Ignoring next_phase

`execute_from_phase` hardcoded Review -> Deliver regardless of the `next_phase` parameter, making it impossible to resume from the execute phase.

### P2-542: Resume Path Not Respecting Critic Rejection

`execute_from_phase` always accepted review results and proceeded to deliver. If the critic rejected, there was no re-iteration loop — unlike `execute_inner` which re-decomposed and re-executed.

### P2-533/534: Silent Failures on Busy Agent

Child task completion errors were silently discarded with `let _ =`. `dispatch_skill_by_name` returned `Ok(())` when the agent was busy instead of propagating the error.

### P3-535: No Checkpoint Version Envelope

Checkpoint serialization had no version marker, making it impossible to evolve the format without breaking in-flight team runs.

### P3-536: LIKE Pattern Injection via Session Prefix

`count_pending_callback_tasks_by_session_prefix` used `LIKE 'team-{run_id}-%'` — if `run_id` contained `%` or `_`, it would match unrelated sessions.

### P3-537: Orphan Processes Not Killed on Expiry

When callback tasks expired, the associated background process continued running indefinitely. No cleanup mechanism existed.

### P3-538/539: Constructor and Finalization Duplication

`TeamEngine::new()` and `new_for_resume()` duplicated resource initialization. `execute_from_phase` duplicated the deliver + finalize logic from `execute_inner`.

### P3-540: SkillRun Prompt Framing Too Terse

`SilentTrigger::SkillRun` produced a minimal prompt with no actionable guidance for the agent.

### P3-541: No System Prompt for Long-Running Tools

The system prompt had no guidance about long-running tools returning task IDs instead of immediate results, causing agents to retry or hallucinate results.

## Solution

### P1-522: Shared Env Scrubbing Helper

Extracted `scrub_mika_env_vars()` helper used by both `execute_exec` and `spawn_long_running_exec`:

```rust
fn scrub_mika_env_vars(cmd: &mut Command) {
    for (key, _) in std::env::vars() {
        if key.starts_with("MIKA_") {
            cmd.env_remove(&key);
        }
    }
}
```

**File:** `crates/mika-agent/src/skills/executor.rs`

### P1-523: Strip Label Prefix on Match

```rust
// Before: exact match fails (label="team-agent-analyst", key="analyst")
c.label.as_deref() == Some(agent_name)

// After: strip prefix for matching
c.label.as_deref()
    .map(|l| l.strip_prefix("team-agent-").unwrap_or(l))
    == Some(agent_name)
```

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

### P1-524: Race Condition Guard + Parent Cancellation

Two fixes: (1) `dispatch_invoke_orchestrator` verifies team run is in `suspended` status before proceeding. (2) Parent task cancelled when team completes synchronously:

```rust
// After synchronous completion — cancel the parent task tree
if !has_pending_grandchildren {
    self.db.cancel_task(&parent_task_id).await?;
}
```

**File:** `crates/mika-agent/src/teams/engine.rs`, `dispatcher.rs`

### P1-525: Partial Indexes

Added to `migrate_v1()`:

```sql
CREATE INDEX IF NOT EXISTS idx_tasks_parent
    ON tasks(parent_task_id, agent_id) WHERE parent_task_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_tasks_session
    ON tasks(created_by_session) WHERE created_by_session IS NOT NULL;
```

**File:** `crates/mika-agent/src/db.rs`

### P2-526: Typed DispatchError

Replaced `anyhow::Error` with typed enum:

```rust
pub enum DispatchError {
    AgentBusy(String),
    Other(anyhow::Error),
}
```

Callers now match on `DispatchError::AgentBusy(_)` instead of string inspection.

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

### P2-527: Scoped Expired Children Query

Added JOIN to filter by parent status:

```sql
SELECT c.id FROM tasks c
JOIN tasks p ON c.parent_task_id = p.id
WHERE c.status = 'expired' AND c.agent_id = ?1
  AND p.status = 'pending' AND p.agent_id = ?1
```

**File:** `crates/mika-agent/src/db.rs`

### P2-528: Null Stdout + Capped Stderr

```rust
// Stdout → /dev/null (background process output not needed)
cmd.stdout(Stdio::null());

// Stderr capped on failure
let mut stderr_buf = Vec::new();
child.stderr.take().unwrap().take(MAX_OUTPUT_LEN as u64)
    .read_to_end(&mut stderr_buf).await?;
```

**File:** `crates/mika-agent/src/skills/executor.rs`

### P2-529: Retry Backoff with Timeout Guard

```rust
DispatchError::AgentBusy(_) => {
    if task.timeout_at.map_or(false, |t| t < now + 30) {
        db.update_task_failed(&task.id, "Agent busy, deadline exceeded").await?;
    } else {
        db.set_task_next_fire_at(&task.id, now + 30).await?;
    }
}
```

30-second delay between retries. Tasks past their deadline are failed instead of re-queued.

**Files:** `engine.rs`, `handlers.rs`

### P2-530: Centralized check_and_dispatch_parent Helper

```rust
async fn check_and_dispatch_parent(&self, task_id: &str) -> Result<()> {
    if let Some(parent_id) = self.db.try_complete_parent_on_sibling_done(task_id).await? {
        if let Some(parent) = self.db.get_task(&parent_id).await? {
            let config = parent.action_config_json();
            self.dispatch(&parent, &config).await.ok();
        }
    }
    Ok(())
}
```

All three call sites reduced to `self.check_and_dispatch_parent(&task_id).await?`.

**File:** `crates/mika-agent/src/task_engine/dispatcher.rs`

### P2-531: Status Constants

```rust
// Before
if task.status == "completed" { ... }

// After
if task.status == task_status::COMPLETED { ... }
```

**File:** `crates/mika-agent/src/server/handlers.rs`

### P2-532: Wire next_phase Parameter

```rust
pub async fn execute_from_phase(&mut self, next_phase: &str, child_results: &str) -> Result<()> {
    match next_phase {
        "execute" => { /* re-run execute with injected context */ }
        "review" => { /* run review_and_iterate */ }
        "deliver" => { /* skip straight to deliver */ }
        _ => return Err(anyhow!("unknown phase: {next_phase}")),
    }
}
```

**File:** `crates/mika-agent/src/teams/engine.rs`

### P2-542: Shared review_and_iterate Loop

Extracted from `execute_inner` so both primary and resume paths share critic rejection handling:

```rust
async fn review_and_iterate(&mut self, agent_results: &str) -> Result<String> {
    for iteration in 0..self.max_iterations {
        let review = self.run_review(agent_results).await?;
        if review.accepted { return Ok(review.feedback); }
        // Re-decompose and re-execute with critic feedback
        let new_results = self.run_execute_phase(&review.feedback).await?;
    }
    Ok("Max iterations reached — delivering with warning".into())
}
```

**File:** `crates/mika-agent/src/teams/engine.rs`

### P3-535: Checkpoint Version Envelope

```rust
const CHECKPOINT_VERSION: u32 = 1;

pub fn serialize_checkpoint(data: &TeamRunCheckpoint) -> Result<String> {
    let envelope = json!({ "version": CHECKPOINT_VERSION, "data": data });
    Ok(serde_json::to_string(&envelope)?)
}

pub fn deserialize_checkpoint(s: &str) -> Result<TeamRunCheckpoint> {
    let v: Value = serde_json::from_str(s)?;
    match v.get("version") {
        Some(_) => serde_json::from_value(v["data"].clone()),
        None => serde_json::from_str(s), // legacy fallback
    }
}
```

`#[serde(default)]` on all checkpoint fields for forward compatibility.

**File:** `crates/mika-agent/src/teams/types.rs`

### P3-536: Exact team_run_id Match

Renamed to `count_pending_callback_tasks_by_team_run` with exact equality:

```sql
WHERE team_run_id = ?1 AND trigger_type = 'callback' AND status = 'pending'
```

**File:** `crates/mika-agent/src/db.rs`

### P3-537: Orphan Process Cleanup

```rust
fn kill_orphan_processes(&self, expired_tasks: &[(String, i64)]) {
    for (task_id, pid) in expired_tasks {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
        self.db.clear_task_process_id(task_id);
    }
}
```

Called in `startup_recovery()` and periodic tick scan via `expire_timed_out_tasks`.

**File:** `crates/mika-agent/src/task_engine/engine.rs`

### P3-538/539: Shared Helpers

Extracted `init_resources()` (shared by `new()` and `new_for_resume()`), `deliver_phase()`, and `finalize_and_shutdown()` to eliminate duplication.

**File:** `crates/mika-agent/src/teams/engine.rs`

### P3-540: Expanded SkillRun Prompt

```rust
SilentTrigger::SkillRun { skill_name } => format!(
    "You are running the '{}' skill autonomously. Execute the skill's purpose, \
     use available tools to complete the work, and store any results. \
     If the skill produces output for the user, use send_message.",
    skill_name
),
```

**File:** `crates/mika-agent/src/agent.rs`

### P3-541: System Prompt Guidance

Added to prompt assembly:

> Some tools are long-running and return a task ID instead of immediate results. When this happens, inform the user that a background task is running and you'll follow up when results arrive. Do not retry the tool.

**File:** `crates/mika-agent/src/prompt.rs`

## Prevention Strategies

### 1. Environment Variable Scrubbing

Always use a shared helper for env var cleanup. The `scrub_mika_env_vars()` pattern ensures consistency across all subprocess launch sites. When adding new subprocess spawn points, grep for `scrub_mika_env_vars` and apply it.

### 2. Typed Error Enums Over String Matching

Never match on error message strings. Define typed error enums (`DispatchError`) with explicit variants. This gives compile-time safety when adding new error conditions.

### 3. Index Coverage for Query Patterns

Every `WHERE` clause pattern used in hot paths (task completion checks, sibling queries) must have a corresponding index. Use partial indexes (`WHERE col IS NOT NULL`) to minimize storage overhead.

### 4. Centralize Repeated Logic

When the same 5+ line pattern appears at 3+ call sites, extract a helper method. The `check_and_dispatch_parent()` and `review_and_iterate()` patterns demonstrate this — one bug fix in the helper fixes all callers.

### 5. Checkpoint Versioning from Day One

Always wrap serialized state in a version envelope. Adding `#[serde(default)]` to all fields enables forward compatibility. The cost is minimal; the migration pain avoided is significant.

### 6. Subprocess Resource Management

- Redirect stdout to `/dev/null` when output is not consumed
- Cap stderr reads with `AsyncReadExt::take()`
- Record PIDs and kill orphans on task expiry
- Send SIGTERM (not SIGKILL) to allow graceful cleanup

### 7. Prompt Engineering for Tool Behavior

When tools have non-obvious behavior (async return, task IDs instead of results), add explicit guidance in the system prompt. Agents cannot infer tool semantics from the tool schema alone.

### 8. Race Condition Prevention in State Machines

When multiple paths can reach the same state transition (synchronous completion vs. async callback), use status guards: check the expected pre-condition status atomically before proceeding.

## Verification

- **Tests:** 895 passing (no failures)
- **Clippy:** Clean (0 warnings, including too_many_arguments fix via `ToolDispatchCtx`)
- **Formatting:** Clean (`cargo fmt --check` passes)
- **Branch:** `feat/unified-task-engine`, commits `6fcddaf`..`22d9aaa`
