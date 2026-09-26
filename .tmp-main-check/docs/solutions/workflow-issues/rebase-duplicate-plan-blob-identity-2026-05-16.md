---
title: Rebase-duplicate plan blob identity — third disposition for grooming-session recovery
module: mika-arch
date: 2026-05-16
problem_type: workflow_issue
component: development_workflow
severity: medium
tags:
  - mika-platform
  - grooming
  - mika-arch
  - workflow
  - git
  - rebase
  - recovery
related_components:
  - claude-pilot
  - dev-groom
applies_when:
  - "A grooming session resumes after the branch was rebased onto a newer main"
  - "Architect's prior verdict cites a SHA that is no longer in current branch ancestry"
  - "Branch and main both moved (sibling-PR landed) but the plan content did not"
---

# Rebase-duplicate plan blob identity — third disposition for grooming-session recovery

## Symptom

mika-arch resumes a grooming session whose prior verdict was `GROOMED`. Its recovery logic runs the ancestry check ("is the prior-verdict SHA in the current branch ancestry?"), gets `NO`, and concludes "no prior verdict applies — re-groom from scratch." It then burns ~4 turns and ~$0.30 to re-derive the same `GROOMED` verdict it already issued.

The plan file's text never changed. The branch was rebased.

## How it happens

mika-arch's session_store records verdicts as "GROOMED at parent SHA `A`, plan blob hash `X`." The recovery frame stored in the grooming spec is binary:

| Ancestry check | Recovery action |
|----------------|-----------------|
| `A` is in `git log HEAD`'s ancestry | Prior verdict still applies — no re-review |
| `A` is NOT in `git log HEAD`'s ancestry | Re-engage architect from scratch |

This frame fails on rebase. When a sibling PR lands on `main` and the grooming branch is rebased to pick up the new base, git **replays** the same commits onto the new parent. Result:

- Branch tip SHA changes (`A` → `B`) — `A` survives only in the reflog, not in `git log B`'s ancestry.
- Plan file content does not change — git stores blobs by content hash, so `git ls-tree --object-only B -- <path>` returns the **same blob OID** as `git ls-tree --object-only A -- <path>`.

mika-arch reviewed the plan's *content* (the words on the page), not the parent commit SHA. Content identity is provable. The ancestry check is a proxy that drops accuracy on rebase; blob identity is the real invariant.

## How to verify (the recovery recipe)

Compute four blob hashes in a 4-row table. The first pair targets the plan file itself; the second pair targets each Phase 0 pin file the plan cites by SHA.

```bash
# Plan file pair
git ls-tree --object-only <prior-verdict-sha> -- docs/plans/<plan-file>.md
git ls-tree --object-only <current-tip-sha>   -- docs/plans/<plan-file>.md

# Each Phase 0 pin pair (repeat per pinned file)
git ls-tree --object-only <pin-sha-cited-in-plan> -- <path/to/pinned/file>
git ls-tree --object-only <current-main-tip-sha>  -- <path/to/pinned/file>
```

| Row | SHA | In current ancestry? | Blob hash for cited path |
|-----|-----|----------------------|--------------------------|
| 1 | prior-verdict SHA | typically NO (rebased) | hash of plan file at that SHA |
| 2 | current-tip SHA | YES | hash of plan file at HEAD |
| 3 | pin-SHA cited in plan | typically NO (main moved) | hash of pinned file at pin SHA |
| 4 | current `origin/main` tip | YES | hash of pinned file at main HEAD |

Compare pairwise: row 1 vs row 2, row 3 vs row 4 (one row 3/row 4 pair per Phase 0 pin).

## Decision rule

| Comparison result | Disposition |
|-------------------|-------------|
| All paired blob hashes match | Prior verdict transitively applies. **No re-review.** Resume at the next pipeline step. |
| Any paired blob hash differs | Re-engage architect on what changed. Limit re-review to the diverged paths, not the whole plan. |

The rule extends the binary ancestry frame with a third disposition: **out-of-ancestry but blob-identical → still transitively binding.**

## Canonical example: mika#794, session 1b6ae3cc, 2026-05-16

- **Issue:** `mika#794` (PR merge gate as tagged union)
- **Grooming session:** `1b6ae3cc-...` on 2026-05-16
- **Prior-`GROOMED` tip:** `5a5eb6a4` (reflog only — not in current branch ancestry)
- **Current branch tip:** `1b44e053` (HEAD on `origin/feat/794-pr-merge-gate-tagged-union`)

Plan file `docs/plans/2026-05-15-001-feat-pr-merge-gate-tagged-union-plan.md`:

| SHA | In ancestry of `1b44e053`? | Plan blob hash |
|-----|---------------------------|----------------|
| `5a5eb6a4` | NO | `2a30d63cff880b187e76ce86e63418715be52eee` |
| `1b44e053` | YES | `2a30d63cff880b187e76ce86e63418715be52eee` |

Initial-plan blob (commits `71fefb00` vs `bfca3245`):

| SHA | In ancestry of `1b44e053`? | Initial-plan blob hash |
|-----|---------------------------|------------------------|
| `71fefb00` | NO | `fbf4a1a5faea946b29420240fe031566f85bd5ed` |
| `bfca3245` | YES | `fbf4a1a5faea946b29420240fe031566f85bd5ed` |

Phase 0 pins (`crates/mika-agent/.../pr_merge_with_gate.rs` and `skills/bundled/self-dev-webhook-qa/system_prompt.md`) at pin SHA `8731102d` and at current `origin/main` HEAD:

| File | Blob at pin SHA `8731102d` | Blob at current `origin/main` HEAD |
|------|----------------------------|------------------------------------|
| `pr_merge_with_gate.rs` | identical | identical |
| `self-dev-webhook-qa/system_prompt.md` | identical | identical |

All four blob comparisons match. Histories are rebase duplicates onto a different main base. The prior architect's `GROOMED` verdict is transitively binding under blob identity — no re-review was warranted.

## Callout for grooming-spec authors

The recovery flowchart in `mika-arch-groom-ticket` (and any sibling skill that resumes a verdict) currently has two branches:

1. Prior-verdict SHA in ancestry → verdict applies.
2. Prior-verdict SHA not in ancestry → re-groom from scratch.

This is incomplete. The third branch is:

3. Prior-verdict SHA not in ancestry, **but** every cited path has identical blob hash at prior-verdict SHA and at current tip → verdict transitively applies, no re-review needed.

Recommended amendment: insert the blob-identity check **before** declaring "re-groom from scratch." The check is cheap (`git ls-tree --object-only` is a constant-time index lookup, no working-tree materialization) and it exactly captures the architect's actual review object — file content, not commit topology. Apply the same check to Phase 0 pins, since pins are content claims about specific files at specific SHAs, and rebase-duplicate main tips preserve them too.

## Why this matters

Architects are expensive — Sonnet 4.6 grooming runs at ~$0.30 per re-engagement, and the operator pays the same dollars whether the verdict changes or not. The ancestry check is a topology proxy for a content question; rebase decouples them. Without the third disposition, every sibling-PR-merge-then-rebase costs an unnecessary re-review.

The pattern generalizes beyond architect verdicts. Any cached judgment that cites a commit SHA but reasons about file content is vulnerable to the same false-invalidation on rebase. Blob identity is the right invariant for content-anchored caches. Commit SHA is the right invariant only when the cache reasons about *history*.

## Related

- `mika-platform/feedback_check_code_when_asked_about_code.md` (memory) — adjacent rule: code claims need PRAGMA + grep + file:line, not commit-SHA citations alone. Same shape — the SHA is a pointer to content, not the content itself.
- `mika-platform/feedback_plan_rebase_risk_calibration.md` (memory) — sibling tickets co-editing the same function make rebase expected, not rare. This doc is the recovery primitive for the rebase outcome that memory entry warns about.
- `mika/docs/solutions/workflow-issues/grooming-branch-callout-required-2026-04-25.md` — companion grooming-recovery doc covering the upstream case (branch picked at dispatch). This doc covers the downstream case (verdict resumed after rebase).
- `senara-solutions/mika#794` — canonical instance.
