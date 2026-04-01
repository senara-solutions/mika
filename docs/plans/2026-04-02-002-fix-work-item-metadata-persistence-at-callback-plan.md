---
title: "fix: persist work item metadata at callback time"
type: fix
status: completed
date: 2026-04-02
issue: "#376"
---

# fix: persist work item metadata at callback time

## Overview

When claude-pilot completes and mika-dev receives the callback, the work item's `metadata` field remains empty. No `session_id`, `cost_usd`, `duration_ms`, `turns`, `branch`, or `pr_url` is recorded. This breaks audit commands, per-task cost tracking, and dashboard task detail pages.

## Problem Statement

Work item metadata persistence is currently **entirely agent-driven** — the engine never writes metadata automatically. The self-dev skill prompt instructs the agent to call `update_work_item_status` with metadata in its close-out step (Step 6), but this step is the **last** action in the callback flow. When the 20-step tool budget is exhausted by earlier actions (QA delegation, retry loops, notifications), Step 6 is dropped and metadata is lost.

The `max_steps_exceeded` continuation turn runs with tools disabled, so even if the agent "wants" to persist metadata at that point, it cannot.

## Proposed Solution

**Combined approach: deterministic engine-level extraction + prompt restructuring.**

### Part 1: Engine-Level Metadata Extraction (Primary Fix)

Add a `try_extract_callback_metadata()` function in `dispatcher.rs` that runs **before** `run_silent_agent()` in `dispatch_resume_agent()`. This function:

1. Checks if `task.parent_task_id` is `Some` and the parent is a `trigger_type = 'manual'` work item
2. Parses the callback result text for structured fields using line-based regex
3. Writes extracted metadata to the parent work item via `update_work_item_metadata()`
4. Uses JSON shallow merge (same pattern as the tool) to avoid clobbering any existing metadata

**Extractable fields** (from callback result text):
- `session_id` — from `"Session: ..."` line
- `cost_usd` — from `"Cost: $..."` line
- `duration_ms` — from `"Duration: ...ms"` line
- `turns` — from `"Turns: ..."` line

**Non-extractable fields** (agent-enriched via self-dev prompt):
- `branch` — derived from worktree context, not in callback result
- `pr_url` — discovered via `gh pr list`, not in callback result

This is deterministic — no LLM involvement. It guarantees at minimum 4 of 6 fields are persisted even if the silent agent exhausts its step budget.

### Part 2: Self-Dev Prompt Restructuring (Complementary)

Move the metadata persistence call from Step 6 (close-out) to immediately after metadata extraction in Step 3 (callback entry). The agent writes all 6 fields (including `branch` and `pr_url`) early, before consuming steps on QA delegation and retry loops. Step 6 still calls `update_work_item_status` for the status transition with any additional metadata discovered during QA.

The shallow merge in `merge_and_persist_metadata()` means the agent's later call enriches (never clobbers) the engine-written base.

## Technical Approach

### Engine-Level Changes

**File: `crates/mika-agent/src/task_engine/dispatcher.rs`**

Add new function `try_extract_callback_metadata()`:

```rust
/// Extract metadata from callback result text and persist to parent work item.
///
/// This is a best-effort, fire-and-forget operation. Failures are logged
/// but do not block the callback dispatch.
async fn try_extract_callback_metadata(
    db: &AsyncDatabase,
    task: &Task,
) {
    // 1. Check parent_task_id exists
    let parent_id = match &task.parent_task_id {
        Some(id) => id.clone(),
        None => return,
    };

    // 2. Verify parent is a manual work item
    let parent = match db.get_task_unscoped(&parent_id).await {
        Ok(Some(t)) if t.trigger_type == "manual" => t,
        _ => return,
    };

    // 3. Parse result text
    let result = match &task.result {
        Some(r) if !r.is_empty() => r,
        _ => return,
    };

    let extracted = extract_callback_fields(result);
    if extracted.is_null() {
        return;
    }

    // 4. Shallow merge with existing metadata
    let merged = match &parent.metadata {
        Some(existing) => {
            if let Ok(mut base) = serde_json::from_str::<serde_json::Value>(existing) {
                if let Some(base_obj) = base.as_object_mut() {
                    if let Some(new_obj) = extracted.as_object() {
                        for (k, v) in new_obj {
                            base_obj.insert(k.clone(), v.clone());
                        }
                    }
                }
                base
            } else {
                extracted
            }
        }
        None => extracted,
    };

    // 5. Persist
    match db
        .update_work_item_metadata(&parent_id, &merged.to_string())
        .await
    {
        Ok(true) => info!(
            parent_task_id = %parent_id,
            callback_task_id = %task.id,
            "engine: persisted callback metadata to work item"
        ),
        Ok(false) => warn!(
            parent_task_id = %parent_id,
            "engine: parent work item not found for metadata write"
        ),
        Err(e) => warn!(
            parent_task_id = %parent_id,
            error = %e,
            "engine: failed to persist callback metadata"
        ),
    }
}

/// Parse structured fields from callback result text.
///
/// Expected format (lines from claude-pilot run.sh):
///   Session: <session_id>
///   Turns: <N>
///   Cost: $<amount>
///   Duration: <N>ms
fn extract_callback_fields(result: &str) -> serde_json::Value {
    use regex::Regex;
    use std::sync::LazyLock;

    static RE_SESSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Session:\s*(\S+)").unwrap());
    static RE_TURNS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Turns:\s*(\d+)").unwrap());
    static RE_COST: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Cost:\s*\$([0-9]+(?:\.[0-9]+)?)").unwrap());
    static RE_DURATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Duration:\s*(\d+)ms").unwrap());

    let mut map = serde_json::Map::new();

    if let Some(cap) = RE_SESSION.captures(result) {
        map.insert("session_id".into(), cap[1].into());
    }
    if let Some(cap) = RE_TURNS.captures(result) {
        if let Ok(n) = cap[1].parse::<u64>() {
            map.insert("turns".into(), n.into());
        }
    }
    if let Some(cap) = RE_COST.captures(result) {
        map.insert("cost_usd".into(), cap[1].into());
    }
    if let Some(cap) = RE_DURATION.captures(result) {
        if let Ok(n) = cap[1].parse::<u64>() {
            map.insert("duration_ms".into(), n.into());
        }
    }

    if map.is_empty() {
        serde_json::Value::Null
    } else {
        // Nest under "claude_pilot" key to match self-dev prompt schema
        serde_json::json!({ "claude_pilot": map })
    }
}
```

**Integration point** in `dispatch_resume_agent()`:

```rust
// After constructing trigger, before run_silent_agent
if is_callback {
    try_extract_callback_metadata(&self.db, task).await;
}
```

### Self-Dev Prompt Changes

**File: `mika-skills/self-dev/system_prompt.md`**

Restructure the callback entry point to persist metadata immediately after extraction (Step 3), not defer to Step 6:

**Before (current):**
- Step 3: Extract metadata fields, remember for Step 6
- Steps 4-5: Delegate QA, handle retries
- Step 6: Close out — call `update_work_item_status` with metadata (often dropped)

**After (proposed):**
- Step 3: Extract metadata fields, **immediately** call `update_work_item_status` with `claude_pilot` metadata (status unchanged, metadata-only update)
- Steps 4-5: Delegate QA, handle retries (as before)
- Step 6: Close out — call `update_work_item_status` with status transition + any additional metadata from QA

## System-Wide Impact

### Interaction Graph

1. Callback task completes → `dispatch_resume_agent()` fires
2. NEW: `try_extract_callback_metadata()` reads parent work item, writes metadata
3. `run_silent_agent()` runs with self-dev skill
4. Agent calls `update_work_item_status` (shallow merge enriches engine-written metadata)
5. Dashboard reads `tasks.metadata` for display

### Error Propagation

- Engine extraction is fire-and-forget: DB errors logged at `warn!`, never block callback dispatch
- Agent metadata writes use existing `merge_and_persist_metadata()` error handling (returns error to agent)
- `update_work_item_metadata()` returns `false` if task not found — logged, not retried

### State Lifecycle Risks

- **Race condition:** Engine writes metadata, then agent overwrites with shallow merge → safe (merge preserves engine-written keys unless agent writes the same keys, which is fine since values should be identical)
- **Terminal state:** `update_work_item_metadata()` only checks `trigger_type = 'manual'`, not status → works on completed/cancelled items
- **Missing parent:** If `parent_task_id` is `None` or parent is not a work item → early return, no-op

### API Surface Parity

- No new tools or API endpoints
- Dashboard task detail page already renders `metadata` JSON — no changes needed
- `check_work_item` tool already returns metadata — no changes needed

## Acceptance Criteria

- [x] Engine-level extraction persists `claude_pilot.session_id`, `claude_pilot.cost_usd`, `claude_pilot.duration_ms`, `claude_pilot.turns` to parent work item before silent agent runs
- [x] Self-dev prompt restructured to persist metadata immediately after extraction (Step 3)
- [x] Metadata shallow merge preserves existing keys (e.g., `pipeline_retry_count`)
- [x] No-op when `parent_task_id` is `None` or parent is not a manual work item
- [x] Extraction failures logged at `warn!` but never block callback dispatch
- [x] Unit tests for `extract_callback_fields()` covering success format, pipeline failure format, missing fields, and empty/garbage input
- [x] Integration test verifying engine metadata write in dispatcher with mock DB

## MVP

### `crates/mika-agent/src/task_engine/dispatcher.rs` (engine extraction)

```rust
// In dispatch_resume_agent(), after trigger construction:
if is_callback {
    try_extract_callback_metadata(&self.db, task).await;
}
```

### `crates/mika-agent/src/task_engine/dispatcher.rs` (extraction function + tests)

```rust
fn extract_callback_fields(result: &str) -> serde_json::Value { /* regex parsing */ }
async fn try_extract_callback_metadata(db: &AsyncDatabase, task: &Task) { /* load parent, merge, persist */ }

#[cfg(test)]
mod tests {
    #[test]
    fn test_extract_success_format() { /* "Session: abc\nTurns: 91\nCost: $7.07\nDuration: 996000ms" */ }
    #[test]
    fn test_extract_pipeline_failure() { /* "PIPELINE FAILURE: ...\nSession: abc\n..." */ }
    #[test]
    fn test_extract_missing_fields() { /* partial matches */ }
    #[test]
    fn test_extract_garbage() { /* no matches → Null */ }
}
```

### `mika-skills/self-dev/system_prompt.md` (prompt restructuring)

Move metadata persistence to Step 3 callback entry point.

## Sources

- Related issue: #376
- Architecture pattern: `docs/solutions/architecture-patterns/generic-callback-framing-parent-task-id.md`
- Architecture pattern: `docs/solutions/architecture-patterns/callback-turn-work-item-context-injection.md`
- Key files:
  - `crates/mika-agent/src/task_engine/dispatcher.rs:278` — `dispatch_resume_agent()`
  - `crates/mika-agent/src/db.rs:6678` — `update_work_item_metadata()`
  - `crates/mika-agent/src/async_db.rs:1580` — async wrapper
  - `crates/mika-agent/src/tools/update_work_item_status.rs:222` — `merge_and_persist_metadata()`
  - `mika-skills/self-dev/system_prompt.md` — callback entry point
