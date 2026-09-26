---
status: complete
priority: p2
issue_id: 542
tags: [code-review, logic-error, teams, duplication]
dependencies: [539]
---

# execute_from_phase Must Respect Critic Rejection Like execute_inner

## Problem Statement

After resume, if the critic rejects, the code logs "critic rejected on resume, proceeding anyway" and delivers regardless. The regular `execute_inner` flow supports re-decomposition iterations with critic feedback. The critic exists for a reason — it should be respected on all code paths, including resume.

**Severity:** P2 — Silently delivers rejected work on the resume path.

## Findings

- `crates/mika-agent/src/teams/engine.rs` — `execute_from_phase` ignores critic rejection
- `crates/mika-agent/src/teams/engine.rs` — `execute_inner` has re-iteration logic that handles rejection correctly

## Proposed Solutions

1. **Extract shared re-iteration logic, use on both paths**
   - Extract the critic rejection → re-execute → re-review loop from `execute_inner` into a shared method
   - `execute_from_phase` calls the same shared method after injecting child results
   - On critic rejection: re-run execute phase with critic's feedback (up to `max_iterations`)
   - If still rejected after `max_iterations`: deliver with a warning that critic flagged issues
   - Remove the "proceeding anyway" log — never silently deliver rejected work
   - Pros: Single source of truth, consistent quality enforcement
   - Cons: Slightly larger refactor
   - Effort: Medium
   - Risk: Low

## Acceptance Criteria

- [ ] `execute_from_phase` handles critic rejection the same way `execute_inner` does
- [ ] Re-iteration logic extracted from `execute_inner` and shared by both paths (no duplication)
- [ ] On rejection: re-execute with critic feedback up to `max_iterations`
- [ ] After `max_iterations` exhausted: deliver with warning, not silent acceptance
- [ ] "Proceeding anyway" log removed
