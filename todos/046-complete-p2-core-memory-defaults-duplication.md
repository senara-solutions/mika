---
status: complete
priority: p2
issue_id: "046"
tags: [code-review, quality, architecture, rust-v2]
dependencies: []
---

# Core Memory Section Names and Defaults Duplicated

## Problem Statement
The 4 core memory section names appear in 5 locations and default values appear in 3 locations. If someone changes one, they must change all others. Drift risk.

**Locations:**
- `db.rs:seed_core_memory` - hardcoded defaults
- `cli.rs:CORE_MEMORY_DEFAULTS` - reset defaults
- `tools/update_core_memory.rs:ALLOWED_SECTIONS` - validation allowlist
- `cli.rs:/reset handler` - allowed blocks list
- `prompt.rs` - implicit ordering

**Reported by:** pattern-recognition-specialist, code-simplicity-reviewer

## Proposed Solutions

### Option A: Single shared constant (Recommended)
Define `CORE_MEMORY_SECTIONS` as a shared constant with names and defaults. All locations reference it.
- **Effort:** Small
- **Risk:** None

## Acceptance Criteria
- [ ] Single source of truth for core memory section names and defaults
- [ ] All locations reference the shared constant

## Work Log
| Date | Action | Learnings |
|------|--------|-----------|
| 2026-02-24 | Created from multi-agent code review | |
