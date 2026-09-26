---
status: complete
priority: p1
issue_id: "310"
tags: [code-review, security, correctness]
dependencies: []
---

# truncate_summary panics on multi-byte UTF-8 input

## Problem Statement

`truncate_summary` in `agent.rs` uses byte-index slicing (`s[..max_len.saturating_sub(3)]`) which panics at runtime if the cut point falls inside a multi-byte UTF-8 character. Tool inputs and outputs contain arbitrary Unicode (user names, CJK text, emoji in shell output). This crashes the agent loop with no recovery.

Identified by: architecture-strategist, security-sentinel, performance-oracle, agent-native-reviewer

## Findings

- `s.len()` returns byte length; `s[..n]` slices by byte offset
- If `n` falls inside a multi-byte sequence, Rust panics: `byte index N is not a char boundary`
- All existing tests use ASCII-only strings and don't catch this
- The codebase already has the correct pattern in `compaction.rs` lines 64-68: `while !summary_text.is_char_boundary(summary_text.len())`

## Proposed Solutions

### Option A: Use `is_char_boundary` walk-back (Recommended)
```rust
fn truncate_summary(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let cut = max_len.saturating_sub(3);
        let mut boundary = cut;
        while boundary > 0 && !s.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}...", &s[..boundary])
    }
}
```
- Pros: Consistent with existing codebase pattern, zero allocation overhead
- Cons: None
- Effort: Small
- Risk: None

### Option B: Use char_indices iterator
```rust
let boundary = s.char_indices().map(|(i, _)| i).take_while(|&i| i <= cut).last().unwrap_or(0);
```
- Pros: More idiomatic
- Cons: Slightly more overhead for very long strings
- Effort: Small

## Technical Details

- **Affected file:** `crates/mika-agent/src/agent.rs:120-128`
- **Call sites:** `process_tool_calls` (line 600, 614), `tool_calls_metadata_json` fallback (line 145-146), `format_tool_summary_block` (line 175)

## Acceptance Criteria

- [ ] `truncate_summary` uses char-boundary-safe slicing
- [ ] New test with multi-byte input (emoji, CJK) at truncation boundary
- [ ] No panics on arbitrary Unicode strings

## Work Log

- 2026-02-27: Identified during code review of commit 573596b
