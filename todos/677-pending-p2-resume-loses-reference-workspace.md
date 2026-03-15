---
status: pending
priority: p2
issue_id: "677"
tags: [code-review, architecture, data-loss]
dependencies: []
---

# new_for_resume hardcodes None for reference workspace

## Problem Statement

`TeamEngine::new_for_resume()` at line 192 of `engine.rs` always passes `None` for the reference workspace parameter to `init_resources()`. This means suspended team runs that were originally started with `--run-id` lose their reference workspace context when resumed. Agents that could read reference files during the initial run will lose that ability after suspend/resume.

## Findings

- **Architecture Strategist**: Medium severity. If reference workspace is important enough to provide at run start, it should survive suspension.

**Affected files:**
- `crates/mika-agent/src/teams/engine.rs` (`new_for_resume`, line ~192)
- `crates/mika-agent/src/teams/types.rs` (`TeamRun` struct — needs `reference_run_id` field)

## Proposed Solutions

### Option A: Persist reference_run_id in TeamRun (Recommended)
Add `reference_run_id: Option<String>` to `TeamRun`, persist in DB `team_runs` table, reconstruct reference workspace path on resume.
- **Pros:** Complete fix, reference workspace survives suspend/resume
- **Cons:** Schema change (new column on `team_runs`), migration needed
- **Effort:** Medium
- **Risk:** Low (additive schema change)

### Option B: Accept the limitation and document it
Document that `--run-id` context is lost on suspend/resume. Users can re-provide `--run-id` on the next `mika ask --team` invocation.
- **Pros:** No code change
- **Cons:** Surprising behavior, may confuse users
- **Effort:** None
- **Risk:** UX degradation

## Recommended Action

Option A in a follow-up PR — this is not a blocker for the current feature.

## Acceptance Criteria

- [ ] `TeamRun` stores `reference_run_id`
- [ ] `new_for_resume` reconstructs reference workspace from stored run_id
- [ ] Test: suspended run with --run-id resumes with reference workspace intact

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-15 | Created from code review | Architecture strategist flagged |

## Resources

- Architecture strategist finding B
