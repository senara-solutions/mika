---
status: pending
priority: p3
issue_id: 263
tags: [code-review, quality, simplification]
dependencies: []
---

# Reduce Prompt Builder Verbosity

## Problem Statement

All 4 prompt builders in prompt.rs use individual `writeln!(ctx, ...).unwrap()` calls for each line. This is verbose compared to using `format!()` with multi-line string literals. The pattern contributes ~140 lines of boilerplate across the module.

## Findings

- Each prompt builder function constructs a `String` by appending one line at a time via `writeln!(ctx, "...").unwrap()`.
- This approach makes the prompt text harder to read as a whole because the actual content is interleaved with Rust formatting machinery.
- Multi-line string literals with `format!()` would make the prompt templates more readable and maintainable, as the template structure would be visible at a glance.
- All 4 prompt builders follow the same verbose pattern.

## Proposed Solutions

Convert prompt builders from `writeln!` chains to `format!()` with multi-line string literals.

**Before:**
```rust
fn build_some_prompt(name: &str, task: &str) -> String {
    let mut ctx = String::new();
    writeln!(ctx, "You are {}", name).unwrap();
    writeln!(ctx, "").unwrap();
    writeln!(ctx, "Your task is:").unwrap();
    writeln!(ctx, "{}", task).unwrap();
    writeln!(ctx, "").unwrap();
    writeln!(ctx, "Follow these rules:").unwrap();
    writeln!(ctx, "1. Be concise").unwrap();
    writeln!(ctx, "2. Be accurate").unwrap();
    ctx
}
```

**After:**
```rust
fn build_some_prompt(name: &str, task: &str) -> String {
    format!(
        "You are {name}\n\
         \n\
         Your task is:\n\
         {task}\n\
         \n\
         Follow these rules:\n\
         1. Be concise\n\
         2. Be accurate\n"
    )
}
```

Estimated reduction from ~220 lines of builder code to ~80 lines.

## Technical Details

**Files affected:**
- `crates/mika-agent/src/teams/prompt.rs`

**Considerations:**
- Ensure no trailing whitespace differences between `writeln!` and `format!` output
- Dynamic sections (conditional blocks) may still need separate `format!` or `if` blocks appended
- Compare output strings in tests to ensure byte-for-byte equivalence

## Acceptance Criteria

- [ ] All 4 prompt builders converted to `format!()` with multi-line string literals
- [ ] Prompt output is identical to previous implementation
- [ ] `use std::fmt::Write` import removed if no longer needed
- [ ] All tests pass (`cargo test`)
- [ ] Code is more readable and maintainable

## Work Log

| Date | Note |
|------|------|
| 2026-02-25 | Created from code review of PR #13 |

## Resources

- PR: https://github.com/senara-solutions/mika/pull/13
