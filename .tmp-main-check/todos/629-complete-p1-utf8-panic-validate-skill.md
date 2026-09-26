---
status: complete
priority: p1
issue_id: "629"
tags: [code-review, security, rust]
dependencies: []
---

# UTF-8 Panic in validate_skill() String Slicing

## Problem Statement

`validate_skill()` at `crates/mika-agent/src/skills/index.rs:258` performs byte-based string slicing on `manifest.skill.description`:

```rust
&manifest.skill.description[..manifest.skill.description.len().min(60)]
```

This will **panic at runtime** if the description contains multi-byte UTF-8 characters (e.g., emoji, CJK, accented characters) and the 60-byte boundary falls in the middle of a character.

## Findings

- Identified by: architecture-strategist, pattern-recognition-specialist
- Severity: P1 — runtime panic on valid user input
- The same pattern does NOT exist in `scan_skills_dir()`, so this is new code only

## Proposed Solutions

### Option A: Use `chars().take(60)` (Recommended)
```rust
let desc: String = manifest.skill.description.chars().take(60).collect();
```
- Pros: Simple, correct, idiomatic Rust
- Cons: Allocates a new String
- Effort: Small
- Risk: None

### Option B: Find char boundary
```rust
let end = manifest.skill.description.char_indices().nth(60).map_or(manifest.skill.description.len(), |(i, _)| i);
&manifest.skill.description[..end]
```
- Pros: No allocation
- Cons: More verbose
- Effort: Small
- Risk: None

## Recommended Action

Option A — simplest and clearest.

## Technical Details

- **Affected file:** `crates/mika-agent/src/skills/index.rs:258`

## Acceptance Criteria

- [ ] No panic when skill description contains multi-byte UTF-8 characters
- [ ] Truncation still limits output to ~60 characters

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | — |

## Resources

- [Rust String slicing docs](https://doc.rust-lang.org/std/string/struct.String.html#slicing)
