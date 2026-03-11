---
status: pending
priority: p3
issue_id: "634"
tags: [code-review, architecture, simplicity]
dependencies: []
---

# Duplicated Validation Logic Between scan_skills_dir() and validate_skill()

## Problem Statement

Both `scan_skills_dir()` and `validate_skill()` independently implement the same check sequence: existence, size limit, read, legacy detection, TOML parse. ~25 lines of duplication that could drift over time.

## Findings

- Identified by: architecture-strategist, code-simplicity-reviewer, pattern-recognition-specialist

## Proposed Solutions

### Option A: Extract shared manifest-parsing helper
- Pros: Single source of truth, ~25 lines saved
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [ ] Single shared helper for file-read/size-check/legacy-check/parse sequence

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | — |
