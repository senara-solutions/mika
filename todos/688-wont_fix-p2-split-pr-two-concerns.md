---
status: pending
priority: p2
issue_id: 688
tags: [code-review, architecture]
dependencies: []
---

# Split PR #193 into Two Separate PRs

## Problem Statement

PR #193 bundles two completely unrelated concerns: (1) LLM API key unification (Rust, 29+ files) and (2) UI package extraction (TypeScript/React). If the API key rename introduces a regression, reverting the PR also reverts the UI extraction, and vice versa. This violates the single-responsibility principle for PRs.

## Findings

- Commit `2100f2b` renames `MIKA_ANTHROPIC_API_KEY` to `MIKA_LLM_API_KEY` across Rust code, docs, CI, scripts
- Commit `40c81ec` extracts React components into `packages/ui/`
- Zero overlap between the two changes
- The simplicity reviewer recommends shipping the API key change immediately and deferring/reconsidering the UI extraction

## Proposed Solutions

### Option A: Split into two PRs
- Cherry-pick `2100f2b` into its own branch/PR
- Keep `40c81ec` on a separate branch
- **Pros:** Clean git history, independent revert/deploy
- **Cons:** Extra branch management
- **Effort:** Small
- **Risk:** Low

### Option B: Accept as-is with clear commit separation
- Keep both commits on one PR, ensure they remain separate commits
- **Pros:** Less work
- **Cons:** Coupled revert, muddled PR scope
- **Effort:** None
- **Risk:** Low

## Recommended Action

*(To be filled during triage)*

## Technical Details

- **Affected files:** All 36 changed files in PR #193
- **Components:** mika-common, mika-agent, mika-cli, dashboard, packages/ui

## Acceptance Criteria

- [ ] API key unification and UI extraction are on separate PRs (or decision documented to keep together)

## Work Log

| Date | Action | Learnings |
|------|--------|-----------|
| 2026-03-17 | Created from code review of PR #193 | |

## Resources

- [PR #193](https://github.com/senara-solutions/mika/pull/193)
