---
status: pending
priority: p2
issue_id: "631"
tags: [code-review, testing]
dependencies: []
---

# No Unit Tests for validate_skill()

## Problem Statement

`validate_skill()` in `crates/mika-agent/src/skills/index.rs:206-351` has multiple branches (missing files, oversized files, legacy detection, TOML parse errors, exec handler permission checks, no-op/never-activates warnings) with zero dedicated test coverage.

## Findings

- Identified by: architecture-strategist
- The function has ~145 lines with at least 8 distinct code paths
- Existing tests cover `scan_skills_dir()` and `is_legacy_format()` but not `validate_skill()`

## Proposed Solutions

### Option A: Add targeted tests for each diagnostic path
Test cases needed:
1. Valid skill → returns OK diagnostics only
2. Missing skill.toml → returns ERR
3. Oversized skill.toml → returns ERR
4. Legacy format skill.toml → returns ERR
5. Invalid TOML → returns ERR
6. Missing handler in tools.json → returns ERR
7. Non-existent exec command → returns ERR
8. No-op skill (no tools, no always_on) → returns WARN
9. Never-activates skill (no triggers, no always_on) → returns WARN

- Effort: Medium
- Risk: None

## Acceptance Criteria

- [ ] Each error/warning path in `validate_skill()` has at least one test

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | — |
