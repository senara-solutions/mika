---
status: complete
priority: p2
issue_id: 350
tags: [code-review, performance, optimization]
dependencies: []
---

# `strip_prior_images` Unnecessary Allocations and Multi-Pass Iteration

## Problem Statement

`strip_prior_images()` runs on every agent loop step after step 0, iterating over all prior messages. It has two inefficiencies:

1. **Text cloning before image check:** Clones all text strings from `ToolResultBody::Blocks` into a `Vec<String>` before checking if images exist. If there are no images (common case), those clones are wasted.
2. **User image double-scan:** Scans all blocks with `any()` to check for `ContentBlock::Image`, then iterates again to replace them. Can be done in a single pass.

## Findings

- **Source:** performance-oracle, code-simplicity-reviewer (convergent findings)
- **Location:** `crates/mika-agent/src/agent.rs:232-269`
- **Evidence:** Two-pass pattern for tool result blocks; two-pass pattern for user image blocks

## Proposed Solutions

### Option A: Single-pass with lazy text collection (Recommended)
1. For tool result blocks: check `has_images` first, only then collect text with `as_str()` (not `clone()`).
2. For user image blocks: replace in-place in a single pass, no pre-scan needed.
3. Combine both tool result and user image handling into one iteration over `blocks`.

```rust
for block in blocks.iter_mut() {
    match block {
        ContentBlock::ToolResult { content, .. } => {
            if let ToolResultBody::Blocks(inner_blocks) = content {
                let combined = inner_blocks.iter()
                    .filter_map(|b| match b {
                        ToolResultBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let combined = format!("{combined}\n[image(s) from previous turn omitted]");
                *content = ToolResultBody::Text(combined);
            }
        }
        ContentBlock::Image { .. } => {
            *block = ContentBlock::Text {
                text: "[user image from previous turn omitted]".to_string(),
            };
        }
        _ => {}
    }
}
```

- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] Single iteration over `blocks` handles both tool result images and user images
- [ ] Text strings are borrowed (`as_str()`) rather than cloned when possible
- [ ] No pre-scan `any()` check for user images
- [ ] All existing `strip_prior_images` tests pass

## Work Log

| Date | Action | Result |
|------|--------|--------|
| 2026-02-28 | Identified by performance-oracle and code-simplicity-reviewer | Pending |
| 2026-02-28 | Fixed: single-pass match block, as_str() borrows, no pre-scan | Complete |
