---
status: complete
priority: p3
issue_id: "635"
tags: [code-review, consistency]
dependencies: []
---

# DiagnosticLevel::Error vs doctor.rs CheckStatus::Fail Inconsistency

## Problem Statement

`validate` uses `[ERR]` tags while `doctor` uses `[FAIL]`. Users running both see inconsistent output terminology.

## Findings

- Identified by: pattern-recognition-specialist

## Proposed Solutions

### Option A: Align naming — use [FAIL] to match doctor
### Option B: Extract shared diagnostic type used by both

- Effort: Small
- Risk: None

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-11 | Created from code review | — |
| 2026-03-11 | Resolved: renamed DiagnosticLevel::Error to Fail, tag [ERR] to [FAIL], constructor error() to fail(), updated all call sites | — |
