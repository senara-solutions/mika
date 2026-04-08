---
title: Work Item Metadata Two-Level Shallow Merge
date: 2026-04-08
tags: [work-items, metadata, callback, claude-pilot, shallow-merge]
related_issue: 489
related_pr: TBD
---

# Work Item Metadata Two-Level Shallow Merge

## Problem

Work item `metadata` is enriched from two independent paths:

1. **Engine path** — `task_engine::dispatcher::try_extract_callback_metadata`
   parses claude-pilot's stdout (cost, duration, session, turns) before the
   silent agent runs and persists it under `metadata.claude_pilot.*`.
2. **Agent path** — the agent's later `update_work_item_status` call enriches
   with PR URL, branch, etc. under the same `claude_pilot` key.

Both paths previously implemented a **single-level shallow merge** by hand:

```rust
for (k, v) in new_obj {
    base_obj.insert(k.clone(), v.clone()); // <-- replaces whole object
}
```

When the agent wrote `{"claude_pilot": {"pr_url": "..."}}`, the entire
`claude_pilot` object overwrote the prior one — wiping out the
engine-injected `cost_usd`, `duration_ms`, `session_id`, and `turns` fields.
A real incident on work item `b5f073b7-eb8d-48fc-87e0-2c3deff42b0c`
permanently lost `$6.47` / `1230s` of telemetry, replacing it with `$0.15` /
`118s` from the agent's merge turn.

The CLAUDE.md design intent for #376 ("Engine-level callback metadata
extraction") explicitly promised "shallow merge" enrichment — but the
implementation only honored that at the top level.

## Solution

Introduce a **shared two-level shallow merge helper** in
`crates/mika-agent/src/work_item_metadata.rs`:

```rust
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
```

Semantics:

- Top-level keys from `incoming` are inserted into `base`.
- When **both** sides hold a JSON object for the same top-level key, their
  inner fields are shallow-merged (incoming wins on conflict).
- One level only — recursion stops at depth 1.
- All other top-level conflicts (scalar / array / type mismatch) replace.

Both call sites — `tools/update_work_item_status.rs::merge_and_persist_metadata`
and `task_engine/dispatcher.rs::try_extract_callback_metadata` — now call
the shared helper, guaranteeing identical semantics across the two
enrichment paths.

The tool's input-schema description was updated to explicitly tell the LLM:
"shallow-merged at the top level **and one level deep** for object-valued
fields (e.g. fields under `claude_pilot.*` from a prior callback are
preserved)."

## Why Two Levels And Not More

Two levels matches the actual shape of work-item metadata in practice:
top-level namespaces (`claude_pilot`, `github`, ...) wrapping flat field
bags. Deep recursion would be surprising — array merges, nested-object
identity questions, and field-vs-object collisions all become ambiguous.
The agent's mental model is "I'm adding fields under `claude_pilot`, not
replacing `claude_pilot`" — two levels expresses exactly that.

## Tests

`work_item_metadata.rs` ships 10 unit tests covering inner-field merge,
scalar/object conflicts, type mismatches, no-recursion-past-depth-1, array
replacement, and no-op on non-object inputs.

`update_work_item_status.rs::tests` adds two integration tests through the
tool surface: the #489 reproduction (six-field claude_pilot survives) and
the top-level multi-namespace case (`claude_pilot` and `github` coexist
across two updates).

## Lessons

- **Sibling code paths must share helpers, not duplicate algorithms.** Both
  the engine extractor and the agent tool merge the same JSON shape — they
  drifted only because they were coded separately. The fix is one helper
  imported in two places.
- **Tool descriptions are part of the contract.** The LLM cannot honor a
  semantics it doesn't know about. Updating the input-schema description
  string is as important as updating the code.
- **Missing audit events make data-loss bugs invisible.** This bug was only
  detected because the original $6.47 was visible in claude-pilot's stdout.
  Filing a follow-up: metadata mutations should produce `audit_events`
  rows so before/after history is queryable.

## References

- Issue: #489
- Related: #376 (engine-level callback metadata extraction)
- CLAUDE.md "Engine-level callback metadata extraction (#376)"
- Plan: `docs/plans/2026-04-08-003-fix-update-work-item-status-metadata-shallow-merge-plan.md`
