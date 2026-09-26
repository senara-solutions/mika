---
status: complete
priority: p3
issue_id: "659"
tags:
  - code-review
  - architecture
  - skills
dependencies: []
---

# Consolidate validate_dependencies with apply_overrides to prevent call-site divergence

## Problem Statement

`validate_dependencies()` is called at 7 locations, always immediately after `apply_overrides()`. The coupling is implicit — a future call site that adds `apply_overrides()` but forgets `validate_dependencies()` will have no compiler error. The simplicity reviewer also notes validate_dependencies is warn-only and could be removed entirely.

## Findings

- **Source**: architecture-strategist, code-simplicity-reviewer
- **Evidence**: 7 call sites across 5 files, all follow `apply_overrides(); validate_dependencies();` pattern

## Proposed Solutions

### Option A: Combine into apply_overrides_and_validate()
- Single method that calls both, migrate all 7 sites
- **Pros**: Prevents divergence, reduces boilerplate
- **Effort**: Small

### Option B: Remove validate_dependencies() entirely
- The matcher already handles missing deps gracefully (silent skip)
- Move validation to `mika skills validate` CLI diagnostic
- **Pros**: 75 LOC removed, 5 fewer files touched
- **Effort**: Small

## Technical Details

- **Affected files**: `crates/mika-agent/src/skills/mod.rs` + 7 call sites

## Acceptance Criteria

- [ ] No implicit coupling between apply_overrides and validate_dependencies

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-13 | Created from code review of PR #134 | Architecture and simplicity reviewers had complementary suggestions |
