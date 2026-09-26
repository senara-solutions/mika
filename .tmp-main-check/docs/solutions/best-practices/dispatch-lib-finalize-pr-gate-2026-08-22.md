---
module: skills/bundled/_shared/dispatch-lib.sh
tags: [dispatch-lib, finalize-pr, structural-invariant, mika1941, ac6, ac7, ac8]
problem_type: workflow-drift
category: best-practices
---

# Structural finalize-pr gate for PR-out-of-draft transitions (mika#1941)

## Problem

Between 2026-08-21 and 2026-08-22, three distinct dispatch-lib metadata-drift failures fired in the same n=3 class:

| Founding incident | Class | Cost of drift |
|-------------------|-------|----------------|
| **mika#1676** — dispatch Agent skipped `/ce:review` despite explicit brief instruction | skip-review | MPC caught via correctif Agent post-hoc |
| **PR#1935** — squash-merged as `docs(plans): DoD ... (#1935)` when actual content was `fix(engine): phantom sweep` | stale-title | `git log` on main permanently misleads |
| **PR#1939** — dispatch Agent's textual summary hand-listed 10 files (actual 13, ~700-line under-count) | under-count | ~30 min sami-MPC roundtrip, false merge-refusal |

All three symptoms shared the same causal shape: **the dispatch Agent's textual / metadata output diverged from ground truth at the PR-out-of-draft boundary**, and no structural gate caught it. Sami's directive: *« Une instruction se réinterprète ; un invariant structurel non. »*

## Solution

`_finalize_pr_gate` in `skills/bundled/_shared/dispatch-lib.sh` — a composable structural gate that applies three checks before a PR may leave draft or wip-rescue.

### AC6 — verbatim ground-truth block

Appends a signed markdown block (`## AC6 verbatim ground truth (dispatch-lib finalize gate, mika#1941)`) to the PR body carrying the verbatim output of `git diff --stat origin/main..HEAD` and `gh pr view --json changedFiles,additions,deletions`. Two-pass awk strip refreshes the block on re-run (idempotent on unchanged content, correctly refreshes after rebase / new commits).

Reviewers treat this block as authoritative. Any hand-listed `Files changed` counts elsewhere in the body are informal excerpts.

### AC7 — formal multi-agent review presence

Detects via one of two paths:
- **Signature keywords** (case-insensitive substring in review body): `/ce:review`, `p1/p2/p3`, `adversarial`, `multi-agent`, `multi agent`.
- **Trusted reviewer identities** (`.user.login`): `mika-platform-qa`, `ce-code-review-bot`, `mika-arch`, `mika-qa` (plus `[bot]` suffixed variants).

When absent, adds the `needs-multi-agent-review` label and returns exit 1. Caller (wip_rescue auto-resume, correctif Agent, operator) decides whether to auto-heal or bail-to-human.

### AC8 — PR title matches fix intent

Rewrites the PR title to the most-recent conventional-commit subject (`^(fix|feat|refactor|chore|perf|test|docs|ci|build|style|revert)(\([^)]+\))?!?: `). Rejects `wip(` prefix and non-conforming subjects. No-op when no conventional commit is found in the last 30 commits.

Optimizes for the founding case (PR opened with wip title, fix commit lands last, squash-merge picks stale title). Iterative PRs with `feat` followed by `fix` will see the fix title win — acceptable friction, operator can force-title if the feat is truly the primary contribution.

## Usage

Standalone CLI (dogfood mode):

```
skills/bundled/_shared/finalize-pr <repo> <pr_num> [worktree_dir]
```

Sourced from dispatch-lib.sh (integration mode):

```bash
source skills/bundled/_shared/dispatch-lib.sh
_finalize_pr_gate "$REPO" "$PR_NUM" "$WORKTREE_DIR"
case $? in
    0) echo "green — un-draft allowed" ;;
    1) echo "AC7 review missing — auto-heal or bail-to-human" ;;
    2) echo "invalid args" ;;
    3) echo "gh CLI failure" ;;
esac
```

## Integration points (v1 scope-out — follow-up work)

Callable but not yet wired at:
- `crates/mika-agent/src/wip_rescue.rs` un-draft path (F1 step): should invoke gate before `gh pr ready`; on rc=1 bail-to-human with `needs-multi-agent-review` reason.
- `crates/mika-agent/src/skills/builtin_handlers.rs` mika#1682 guard: should extend the wip-rescue signature check to also gate on AC7 review absence, not just wip-rescue signature.

These integrations layer atop the gate without changing its shape.

## Companion patterns

- `feedback_never_skip_ce_review` — prompt-level rule this gate structurally enforces.
- `feedback_prompt_enforcement_fragile` — the class this whole ticket addresses.
- `feedback_estimated_counts_undercount_measured` — the empirical basis for AC6.
- `feedback_claim_type_stratifies_verification_reliability` — behavioral claims (agent summary) are 1/7 reliable; measured evidence (git-stat) is 9/10.
- `mika/crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` — precedent for structural greps enforcing invariants. AC7 gate is the runtime analog.

## Tests

`skills/bundled/_shared/tests/test_finalize_pr_gate.sh` — 30 assertions:
- AC8: single conv-commit picked, most-recent wins, wip skipped, no-match empty, non-git empty, bang accepted.
- AC6: header signature stable, non-git sentinel, git-stat command echoed, empty-arg sentinel, strip-then-refresh removes prior block cleanly.
- AC7: 4 signature-keyword variants + 2 trusted-reviewer identities detected; empty array + informal-only rejected; malformed payload + missing args error.
- Structural: 4 function-existence guards catch removal regressions.

Uses `gh` function-stub for hermetic AC7 testing (no network).

## Dogfooding on the founding PR (#1946)

The finalize-pr gate was applied to its own PR:
- AC8 rewrote the initial verbose title to the AC8-derived conventional-commit form.
- AC6 appended the verbatim block; a stale-block regression surfaced after rebase and drove the `refresh-in-place` fix (commit `de40433f`).
- AC7 gated on the missing formal multi-agent review; the label was applied and cleared once the review posted.

The self-invariant loop closed cleanly. Dette process n=3 payée.
