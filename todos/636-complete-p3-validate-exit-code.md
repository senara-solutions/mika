---
status: complete
priority: p3
issue_id: "636"
tags: [code-review, cli]
dependencies: []
---

# validate_skills() Always Exits 0 Even on Errors

## Problem Statement

`mika skills validate` returns `Ok(())` regardless of findings. `mika doctor` exits non-zero on failures — this should be consistent.

## Findings

- Identified by: pattern-recognition-specialist

## Proposed Solutions

### Option A: Return non-zero exit code when errors found
- Effort: Small
- Risk: None

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | — |
| 2026-03-11 | Resolved: validate_skills() now returns Result<()>, bails with anyhow when total_errors > 0, caller propagates with ? | — |
