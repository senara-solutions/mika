---
status: complete
priority: p3
issue_id: "421"
tags: [code-review, quality, duplication, reflection]
dependencies: []
---

# Extract check_reflection_evidence() Helper

## Problem Statement

Identical 8-line evidence check block appears in 3 tool files:
- `update_core_memory.rs:77-84`
- `store_fact.rs:68-75`
- `update_fact.rs:56-63`

## Proposed Solutions

Extract to `tools/mod.rs`:
```rust
pub(crate) fn check_reflection_evidence(ctx: &ToolContext<'_>, input: &Value) -> Option<ToolOutput> {
    if ctx.is_reflection {
        let evidence = input["evidence"].as_str().unwrap_or("").trim();
        if evidence.is_empty() {
            return Some(ToolOutput::error(
                "Reflection mode requires an evidence field citing specific conversation content.",
            ));
        }
    }
    None
}
```
Each tool call site becomes: `if let Some(err) = check_reflection_evidence(&ctx, &input) { return Ok(err); }`

- **Effort**: Small (~18 LOC saved)

## Acceptance Criteria

- [ ] Single helper function for evidence check
- [ ] All 3 tools use the helper
- [ ] Existing tests pass
