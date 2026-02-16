---
status: complete
priority: p2
issue_id: "114"
tags: [code-review, quality, architecture]
dependencies: []
---

# Duplicated Prompt Assembly Between System and Silent Prompts

## Problem Statement

`build_system_prompt` and `build_silent_prompt` repeat the same blocks: soul content injection, identity injection, current time section, and core memory section (~30 lines of duplicated logic). Changes to prompt formatting must be applied in both places.

## Findings

- **Source:** pattern-recognition-specialist (2C)
- **Location:** `crates/mika-agent/src/prompt.rs`
  - Soul: lines 72-76 vs 164-168
  - Identity: line 78 vs 170
  - Time: lines 81-91 vs 173-183
  - Core memory: lines 94-103 vs 186-191
- **Evidence:** ~30 lines of near-identical logic across both functions
- **Risk:** When adding new prompt sections (Phase 2), easy to update one function but forget the other

## Proposed Solutions

### Option 1: Extract shared helper functions (Recommended)
- **Pros**: DRY, both builders compose from same primitives
- **Cons**: Slightly more functions to navigate
- **Effort**: Small

```rust
fn write_soul_section(prompt: &mut String, soul_content: &str) { ... }
fn write_identity_section(prompt: &mut String, identity: &Identity) { ... }
fn write_time_section(prompt: &mut String, current_utc: DateTime<Utc>, timezone: Option<&str>) { ... }
fn write_core_memory_section(prompt: &mut String, core_memory: &[CoreMemoryEntry]) { ... }
```

### Option 2: Shared PromptBuilder struct with method chaining
- **Pros**: Fluent API, harder to forget sections
- **Cons**: Over-engineering for current needs
- **Effort**: Medium
- **Risk**: Low

## Recommended Action

_To be filled during triage_

## Technical Details

- **Affected Files**: `crates/mika-agent/src/prompt.rs`

## Acceptance Criteria

- [ ] No duplicated prompt assembly logic
- [ ] Both system and silent prompts produce identical output for shared sections
- [ ] All prompt tests pass
- [ ] System prompt output is unchanged (regression check)

## Work Log

### 2026-02-24 - Identified in v4 Code Review
**By:** Multi-agent review (pattern-recognition-specialist)

## Resources

- Commit under review: 38a843b
