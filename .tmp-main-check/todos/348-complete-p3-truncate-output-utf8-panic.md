---
status: complete
priority: p3
issue_id: 348
tags: [code-review, bug, pre-existing]
dependencies: []
---

# truncate_output Can Panic on Multi-Byte UTF-8

## Problem Statement

Pre-existing bug (not introduced in this PR). `truncate_output()` in executor.rs performs byte-based slicing `&s[..MAX_OUTPUT_LEN]` which panics if the boundary falls within a multi-byte UTF-8 character. The `truncate_summary()` function in the same file already handles this correctly.

## Findings

- **Source:** security-sentinel
- **Location:** `crates/mika-agent/src/skills/executor.rs:322-330`
- **Evidence:** Uses `&s[..MAX_OUTPUT_LEN]` without char boundary check

## Proposed Solutions

### Option A: Use char-boundary-safe truncation (Recommended)
Walk back to valid char boundary before slicing, same as `truncate_summary()`.
- Effort: Small
- Risk: Low

## Acceptance Criteria

- [x] `truncate_output` uses `is_char_boundary()` before slicing

## Work Log

| Date | Action | Result |
|------|--------|--------|
| 2026-02-28 | Identified during code review (pre-existing) | Pending |
| 2026-02-28 | Fixed: added char boundary walk-back before slicing | Complete |
