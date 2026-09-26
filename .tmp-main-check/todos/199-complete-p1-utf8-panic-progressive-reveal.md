---
status: complete
priority: p1
issue_id: "199"
tags: [code-review, correctness, tui]
dependencies: []
---

# UTF-8 Panic in Progressive Reveal

## Problem Statement

The progressive reveal in `app.rs` uses byte-level indexing on a UTF-8 `String`. When `reveal_index` lands in the middle of a multi-byte character (emoji, accented names, CJK text), slicing with `&full[..reveal_index]` will panic at runtime, crashing the TUI.

## Findings

- **Source:** performance-oracle (P0), architecture-strategist (9b)
- **Location:** `crates/mika-cli/src/tui/app.rs:150-156`, `crates/mika-cli/src/tui/ui.rs:98`
- **Evidence:** `full.len()` returns bytes; `&full[..app.reveal_index]` panics on non-char boundaries. Any Claude response containing non-ASCII characters will trigger this.
- **Impact:** Runtime panic that crashes the TUI for any non-English response content.

## Proposed Solutions

### Option 1: Use `floor_char_boundary` (Rust 1.82+, available with edition 2024)
- **Pros**: Single-line fix, no O(n) overhead, idiomatic
- **Cons**: None (project uses edition 2024, rustc 1.92)
- **Effort**: Small
- **Risk**: Low

```rust
let next = full.floor_char_boundary(self.reveal_index + 8);
self.reveal_index = next;
// ...
let revealed = &full[..app.reveal_index.min(full.len())];
```

### Option 2: Use char-based indexing with cached char count
- **Pros**: Works on any Rust version
- **Cons**: Store char_count alongside pending_response; `chars().take(n).collect()` allocates per tick
- **Effort**: Small
- **Risk**: Low

## Recommended Action

Option 1 — `floor_char_boundary` is the cleanest approach given edition 2024.

## Technical Details

- **Affected files:** `crates/mika-cli/src/tui/app.rs`, `crates/mika-cli/src/tui/ui.rs`
- **Components:** TUI progressive reveal

## Acceptance Criteria

- [ ] Non-ASCII Claude responses render without panic
- [ ] Progressive reveal advances at a similar visual rate for mixed-encoding text
- [ ] Manual test: send a message that triggers emoji/accented response

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review | Byte vs char indexing is a common Rust pitfall |

## Resources

- Commit: 399ebf0
