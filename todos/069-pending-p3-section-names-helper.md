---
status: pending
priority: p3
issue_id: "069"
tags: [code-review, quality, rust-v2]
dependencies: []
---

# CORE_MEMORY_SECTIONS Name Extraction Repeated 5 Times

## Problem Statement

The pattern `CORE_MEMORY_SECTIONS.iter().map(|(k, _)| *k).collect::<Vec<&str>>()` appears in 5 locations across 3 files. A small helper would reduce repetition.

## Findings

- **Source:** pattern-recognition-specialist, architecture-strategist
- **Locations:**
  - `crates/mika-agent/src/tools/update_core_memory.rs:22`
  - `crates/mika-agent/src/tools/update_core_memory.rs:76`
  - `crates/mika-agent/src/prompt.rs:51`
  - `crates/mika-agent/src/prompt.rs:94`
  - `crates/mika-agent/src/cli.rs:177`

## Proposed Solutions

### Option A: Add helper function next to constant (Recommended)
- `pub fn core_memory_section_names() -> Vec<&'static str>` in `db.rs`
- **Effort:** Small
- **Risk:** None

### Option B: Add parallel constant
- `pub const CORE_MEMORY_SECTION_NAMES: &[&str] = &[...]` — zero allocation but manual sync
- **Effort:** Small
- **Risk:** Low (drift between constants)

## Acceptance Criteria

- [ ] Helper function or constant exists
- [ ] All 5 call sites use the helper
- [ ] All tests pass

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from code review of commit 3619d13 | Repeated iterator patterns signal a missing abstraction |
