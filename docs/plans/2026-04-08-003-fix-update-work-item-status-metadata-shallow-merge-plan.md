---
title: "fix: update_work_item_status metadata shallow-merges one level deep"
type: fix
status: active
date: 2026-04-08
issue: 489
---

# fix: update_work_item_status metadata shallow-merges one level deep

## Problem

`update_work_item_status` and the engine-level callback metadata extractor
both perform a **single-level** shallow merge of work-item metadata. When the
agent passes `{"claude_pilot": {"branch": "...", "pr_url": "..."}}`, the entire
`claude_pilot` object overwrites the existing one — losing the
engine-injected `cost_usd`, `duration_ms`, `session_id`, and `turns` fields
that came from the prior callback turn.

Real incident (issue #489): work item `b5f073b7-eb8d-48fc-87e0-2c3deff42b0c`
had its `$6.47` / `1230s` claude-pilot telemetry overwritten by a later merge
turn that wrote `$0.15` / `118s`. Cost telemetry permanently lost.

## Root cause

Two call sites implement the same naive single-level merge loop:

1. **`crates/mika-agent/src/tools/update_work_item_status.rs:222-253`** —
   `merge_and_persist_metadata()`. Iterates `new_obj` keys and calls
   `existing_obj.insert(k, v)`, blindly replacing object values.
2. **`crates/mika-agent/src/task_engine/dispatcher.rs:828-845`** —
   `try_extract_callback_metadata()`. Same loop pattern.

Neither inspects whether both sides hold a JSON object for the same top-level
key. CLAUDE.md and the `engine-level-callback-metadata-extraction.md` solution
doc both promise the agent can "enrich with additional fields … shallow merge"
— but the implementation only honors that promise at the top level.

## Fix

Introduce a shared helper that merges two levels deep — top-level keys are
shallow-merged, and when **both** sides hold a JSON object for the same key,
the inner objects are also shallow-merged (one level only, no recursion past
that).

### New module: `crates/mika-agent/src/work_item_metadata.rs`

```rust
use serde_json::{Map, Value};

/// Merge `incoming` into `base` with two-level shallow semantics:
/// - Top-level keys from `incoming` are inserted into `base`.
/// - When both `base[k]` and `incoming[k]` are JSON objects, their fields are
///   shallow-merged (incoming wins on conflict). One level only.
/// - All other top-level conflicts replace the base value.
///
/// `base` is mutated in place. `incoming` must be a JSON object; non-object
/// inputs are no-ops (callers validate shape upstream).
pub fn merge_metadata(base: &mut Value, incoming: &Value) {
    let (Some(base_obj), Some(new_obj)) = (base.as_object_mut(), incoming.as_object()) else {
        return;
    };
    for (k, v) in new_obj {
        match (base_obj.get_mut(k), v) {
            (Some(Value::Object(existing_inner)), Value::Object(new_inner)) => {
                shallow_merge_object(existing_inner, new_inner);
            }
            _ => {
                base_obj.insert(k.clone(), v.clone());
            }
        }
    }
}

fn shallow_merge_object(base: &mut Map<String, Value>, incoming: &Map<String, Value>) {
    for (k, v) in incoming {
        base.insert(k.clone(), v.clone());
    }
}
```

Register the module in `crates/mika-agent/src/lib.rs` (or wherever sibling
modules are declared).

### Wire into `update_work_item_status.rs`

Replace the inline merge loop in `merge_and_persist_metadata()` with:

```rust
let merged = if let Ok(Some(task)) = ctx.db.get_task_unscoped(task_id).await {
    match task.metadata.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()) {
        Some(mut existing) => {
            crate::work_item_metadata::merge_metadata(&mut existing, new_meta);
            existing
        }
        None => new_meta.clone(),
    }
} else {
    new_meta.clone()
};
```

### Wire into `task_engine/dispatcher.rs`

Replace the inline merge loop in `try_extract_callback_metadata()` (lines
828-845) with the same helper call so the engine path and the agent tool path
share identical semantics.

### Tool description tweak

Update `update_work_item_status.rs:84-87` description string from
"shallow-merged with existing metadata" to "shallow-merged at the top level
**and one level deep** for object-valued fields (e.g. `claude_pilot.*`)" so
the LLM understands the contract.

## Acceptance criteria

- [ ] New module `crates/mika-agent/src/work_item_metadata.rs` exposes
      `merge_metadata(&mut Value, &Value)`.
- [ ] `update_work_item_status` tool uses the helper.
- [ ] `task_engine::dispatcher::try_extract_callback_metadata` uses the
      helper.
- [ ] Tool input-schema description string mentions "one level deep".
- [ ] Unit test in `work_item_metadata.rs`: write `{claude_pilot: {a:1,b:2}}`,
      merge `{claude_pilot: {b:3,c:4}}`, expect `{claude_pilot: {a:1,b:3,c:4}}`.
- [ ] Unit test: top-level non-conflicting key from `incoming` is added,
      existing top-level keys preserved.
- [ ] Unit test: top-level scalar conflict — `incoming` value wins.
- [ ] Unit test: top-level type mismatch (existing object, incoming scalar) —
      incoming wins (base overwritten).
- [ ] Unit test: nested-object merge does NOT recurse past one level (e.g.
      `{a:{b:{c:1}}}` merged with `{a:{b:{d:2}}}` produces `{a:{b:{d:2}}}` — `b`
      is replaced because the recursion stops at depth 1).
- [ ] Integration test in `update_work_item_status.rs::tests`: create work
      item, call tool with `{claude_pilot: {cost_usd: "6.47", duration_ms:
      1230514, session_id: "abc", turns: 102}}`, then call again with
      `{claude_pilot: {pr_url: "https://...", branch: "feat/x"}}`, fetch from
      DB and assert all six fields are present on the merged
      `claude_pilot` object.
- [ ] Integration test: a top-level scalar key set in turn 1 is preserved when
      turn 2 sets a different top-level key.
- [ ] `cargo test -p mika-agent` passes.
- [ ] `cargo clippy -p mika-agent --all-targets` clean.

## Out of scope

- Deep merge (recursion past depth 1).
- Audit events for metadata mutations (related telemetry gap noted in #489
  — file as follow-up).
- Migration / repair of work item `b5f073b7…` (per #489: leave it).
- Schema validation on metadata shape.

## Files touched

- `crates/mika-agent/src/work_item_metadata.rs` (new)
- `crates/mika-agent/src/lib.rs` (add `pub mod work_item_metadata;`)
- `crates/mika-agent/src/tools/update_work_item_status.rs` (use helper, update
  description, add merge integration test)
- `crates/mika-agent/src/task_engine/dispatcher.rs` (use helper)

## Sources

- Issue: #489
- Related solution doc:
  `docs/solutions/architecture-patterns/engine-level-callback-metadata-extraction.md`
- Related plan:
  `docs/plans/2026-04-02-002-fix-work-item-metadata-persistence-at-callback-plan.md`
- CLAUDE.md "Engine-level callback metadata extraction (#376)" — design
  intent reference
