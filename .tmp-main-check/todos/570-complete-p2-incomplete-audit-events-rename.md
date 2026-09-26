---
status: complete
priority: p2
issue_id: "570"
tags: [code-review, naming, consistency, observability]
dependencies: []
---

# Incomplete memory_events → audit_events rename in prompt.rs and docs

## Problem Statement

The table and struct were renamed from `memory_events` to `audit_events`, but several code identifiers and documentation references still use the old naming:

1. `SilentPromptContext.recent_memory_events` field (prompt.rs:405)
2. `memory_events_digest` local variable (agent.rs:1196)
3. Prompt text still says "Recent Memory Changes" with `<memory-events>` XML tags
4. `crates/mika-agent/docs/architecture.md` references old table names at lines 154, 717-720

## Findings

- **Source:** Agent-Native Reviewer, Code Simplicity Reviewer (converged independently)
- **Files:** `crates/mika-agent/src/prompt.rs`, `crates/mika-agent/src/agent.rs`, `crates/mika-agent/docs/architecture.md`
- **Impact:** Naming inconsistency confuses which convention to follow

## Proposed Solutions

### Option A: Complete the rename (Recommended)
- Rename `recent_memory_events` → `recent_audit_events` in `SilentPromptContext`
- Rename `memory_events_digest` → `audit_events_digest` in agent.rs
- Update prompt text: "Recent Audit Events" with `<audit-events>` tags
- Update architecture.md references

- **Effort:** Small (15 min)
- **Risk:** None

## Acceptance Criteria

- [ ] No references to `memory_events` in Rust code identifiers
- [ ] Documentation updated to reference `audit_events`
- [ ] `cargo test` passes

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-08 | Created from PR #88 code review | Half-finished rename |
