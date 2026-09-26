---
status: complete
priority: p3
issue_id: "449"
tags: [code-review, duplication]
dependencies: []
---

# Iteration Calculation Duplicated in decompose()

## Problem Statement

Identical `if feedback.is_some() { iteration + 1 } else { 1 }` block at lines ~358 and ~382 in `decompose()`. Should extract to a local variable.

## Proposed Fix

Extract to `let iteration = if feedback.is_some() { ... }` before the match.

## Acceptance Criteria

- [ ] Single calculation of iteration in `decompose()`
