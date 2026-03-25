---
status: pending
priority: p2
issue_id: 727
tags: [code-review, security]
---

# Strip newlines in sanitize_label to prevent prompt structure manipulation

## Problem Statement

The `sanitize_label()` function in `prompt.rs` strips `<` and `>` characters but does not strip newlines (`\n`, `\r`). A crafted task label or preference value containing embedded newlines could create fake entries in the `<task-health>` or `<stored-preferences>` prompt blocks, potentially confusing the LLM's parsing.

## Findings

- `sanitize_label()` strips `<>` and truncates to 200 chars
- Does NOT strip `\n` or `\r`
- Task labels come from `create_work_item` tool calls — user-influenced via LLM
- Preference values also user-influenced
- Risk is low (LLM generates these values) but defense-in-depth is cheap

## Proposed Solutions

```rust
fn sanitize_label(s: &str) -> String {
    s.chars()
        .take(200)
        .filter(|c| *c != '<' && *c != '>' && *c != '\n' && *c != '\r')
        .collect()
}
```

This also eliminates the double-allocation (collect then replace) — single-pass filtering.

## Acceptance Criteria

- [ ] `sanitize_label()` strips `\n` and `\r` in addition to `<>`
- [ ] Single-pass implementation (no intermediate String allocation)
- [ ] Same change applied to `sanitize_ref_url()`
