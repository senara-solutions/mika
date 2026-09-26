---
status: complete
priority: p2
issue_id: "415"
tags: [code-review, quality, reflection, audit]
dependencies: []
---

# Evidence Not Persisted in Audit Logs for store_fact and update_fact

## Problem Statement

In reflection mode, `update_core_memory` persists evidence in the audit log:
```rust
format!("{reasoning} [evidence] {evidence}")
```

But `store_fact` and `update_fact` validate the evidence field and then **discard it**. All `log_memory_event` calls in those tools pass `None` for the reasoning parameter. The evidence is checked but never recorded.

This makes the audit trail incomplete — you can't verify whether reflection-mode fact changes were grounded in real conversation evidence.

## Findings

- **Agent-native reviewer**: "The audit log for fact mutations during reflection will be indistinguishable from normal conversation mutations. The evidence requirement becomes security theater."
- **Architecture reviewer**: "This should be fixed to match update_core_memory's pattern"
- **Pattern recognition**: Confirmed asymmetry across the three tools

## Proposed Solutions

### Option A: Thread evidence through to log_memory_event (Recommended)
At each `log_memory_event` call site in `store_fact.rs` and `update_fact.rs`:
```rust
let reasoning = if ctx.is_reflection {
    input["evidence"].as_str().map(|e| format!("[evidence] {e}"))
} else {
    None
};
ctx.db.log_memory_event(ctx.session_id, "store_fact", &target, None, &after, reasoning.as_deref()).await?;
```
- **Effort**: Small (modify 5 call sites)
- **Risk**: Low

## Technical Details

- **Affected files**: `crates/mika-agent/src/tools/store_fact.rs` (4 call sites), `update_fact.rs` (1 call site)

## Acceptance Criteria

- [ ] Evidence stored in reasoning field for all memory tools in reflection mode
- [ ] Audit log entries from reflection have `[evidence]` prefix marker
- [ ] Test verifying evidence is persisted for store_fact and update_fact in reflection mode
