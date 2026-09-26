---
status: complete
priority: p3
issue_id: 296
tags: [code-review, quality, maintainability]
dependencies: []
---

# Bundled skills data has ~170 lines of repetitive struct literals

## Problem Statement

`bundled_skills.rs` lines 28-188 contain 170 lines of repetitive `BundledSkill`/`SkillFile` struct literals. Each entry follows an identical 5-line pattern where only 3 values change (path, template path, executable flag). Adding a 6th skill requires ~20 lines of boilerplate.

## Findings

- **Code Simplicity Reviewer:** A declarative macro would reduce 170 lines to ~50, improving signal-to-noise ratio and making new skill additions trivial.

## Proposed Solutions

### Solution A: Declarative macro

```rust
macro_rules! skill {
    ($name:expr, [ $( $path:expr => $template:expr $(, +x)? );+ $(;)? ]) => {
        BundledSkill {
            name: $name,
            files: &[$(
                SkillFile {
                    path: $path,
                    content: include_str!($template),
                    executable: skill!(@exec $($( +x )?)?),
                },
            )+],
        }
    };
    (@exec +x) => { true };
    (@exec) => { false };
}
```

- **Pros:** ~100 LOC saved, adding new skills becomes 6 lines instead of 20+
- **Cons:** Macro syntax is less immediately readable for Rust newcomers
- **Effort:** Medium
- **Risk:** Low

### Solution B: Keep as-is

The current code is clear and correct. Repetition is mechanical but explicit.

- **Pros:** No macro to understand, every field visible
- **Cons:** 170 lines of boilerplate, adding skills is tedious
- **Effort:** None
- **Risk:** None

## Technical Details

- **Affected files:** `crates/mika-agent/src/bundled_skills.rs`

## Acceptance Criteria

- [ ] Skill declarations are concise and easy to extend
- [ ] All 4 existing tests still pass
- [ ] New skill can be added in <10 lines
