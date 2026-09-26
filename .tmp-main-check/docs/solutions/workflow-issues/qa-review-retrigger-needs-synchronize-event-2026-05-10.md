---
module: qa-review-dispatch
date: 2026-05-10
problem_type: workflow_issue
component: development_workflow
severity: medium
applies_when:
  - "After deploying a fix to qa-review skill prompt that changes verdict logic"
  - "PRs blocked by the OLD qa-review verdict need to be re-evaluated under the new logic"
  - "Toggling labels (remove + add `ready` or `pipeline-exempt`) does NOT trigger mika-qa to re-review"
tags:
  - mika-qa
  - dispatch
  - webhook
  - synchronize-event
  - retrigger
  - debounce
related_components:
  - github-webhook
  - self-dev-webhook-qa
---

# After deploying qa-review skill fixes, blocked PRs need a `synchronize` event (push) to re-trigger review — label toggles alone are debounced

## Context

When mika-qa's skill prompt is updated (e.g., new exemption logic, changed verdict criteria) and deployed via `make deploy`, PRs that were blocked under the OLD logic do NOT automatically get re-reviewed under the new logic. The `pull_request.labeled` webhook event has an effective debounce — toggling a label on a recently-reviewed PR does NOT fire mika-qa again. Only a `pull_request.synchronize` event (push to PR head) reliably triggers a fresh review.

This means the workflow "deploy fix → re-evaluate previously-blocked PRs" requires explicit operator action: push an empty commit to each blocked PR's branch.

## Guidance

After deploying any qa-review skill change that affects verdict logic, **for each PR previously blocked under the old logic**:

```bash
WT=<path-to-worktree-or-checkout-of-pr-branch>
cd $WT
git pull origin main --no-rebase --no-edit  # bring branch up to date if needed
git commit --allow-empty -m "trigger: re-trigger mika-qa after <skill-fix> deploy

Empty commit to fire pull_request.synchronize webhook — label toggles
don't re-trigger mika-qa after recent verdicts (effective debounce).
<deploy-context>"
git push
```

mika-qa will re-review within ~30 seconds. Verify via `gh pr view <N> --repo senara-solutions/<repo> --json reviews --jq '.reviews | last'` — the new review's `submittedAt` should be after your push timestamp.

**Do NOT use `mika ask --agent mika-qa "review <PR>"` direct-dispatch** as a substitute (per `feedback_ready_label_dispatch_canonical` — always ready-label/synchronize-event for canonical mechanism, never direct mika ask).

## Why This Matters

Without this discipline, deploys of qa-review fixes appear to silently fail — the operator believes the new gate logic is live, but the dashboard still shows old blocking verdicts. Two failure modes:

1. **Operator confusion:** "I deployed the fix; why are the PRs still blocked?" → wastes time re-checking the deploy, the binary, the skill prompt diff.
2. **Silent stagnation:** Without observability into "which previously-blocked PRs did the new fix actually unblock?", a deploy may technically succeed but its consumer effect (PRs flipping from blocked → approved) never lands.

Empirical evidence (2026-05-10):

- Three docs-only PRs (mika#1062, mika#1063, mika-platform#99) were blocked by mika-qa's pre-fix verdict (`block[pipeline]: "no Pipeline-Exempt label"` — fabricated label name; the real label `pipeline-exempt` lowercase wasn't actually checked by the prompt).
- After PR #1065 shipped + v0.12.4 deployed (qa-review skill now honors `pipeline-exempt` label):
  - **Toggle-only attempt:** Removed `ready` label, waited 5 seconds, re-added. Timeline confirmed both events fired (`unlabeled ready` at 17:31:15Z, `labeled ready` at 17:31:38Z). Mika-qa did NOT fire — the latest review still showed the old `block[pipeline]` verdict from 14:48Z.
  - **Synchronize-event attempt:** Pushed an empty commit to each PR's branch. Mika-qa fired immediately, re-reviewed, and emitted `VERDICT: pass` within 30 seconds.

The label-toggle path appears suppressed by webhook handler debounce (avoids re-reviewing the same PR after a recent verdict). The synchronize-event path is the reliable retrigger.

## When to Apply

After ANY mika-qa skill prompt deploy that changes verdict logic. Examples:

- Adding/removing exemption labels (the case that surfaced this lesson)
- Changing AC verification criteria
- Updating diff-review heuristics
- Modifying `block[pipeline]` / `block[ac]` / `pass` decision tree

Also applies to **PRs that were ESCALATED via `block[ac]`** if the underlying AC interpretation changed in the deploy.

Does **NOT** apply to:

- New PRs opened after the deploy (those automatically get the new logic on first review).
- mika-qa-build-callback verdicts (different trigger path; rebuilds happen on `build_mika` callback completion, not on label/synchronize events).

## Examples

### Concrete walkthrough — mika#1062 (2026-05-10 17:00Z)

**State:** PR opened earlier today; received `block[pipeline]` verdict at 14:12Z under old qa-review logic. PR #1065 shipped at 16:16Z + v0.12.4 deployed at 17:49 local (15:49 UTC). Operator wants to retry mika-qa with the new label-honor logic.

**Failed attempt — label toggle:**

```bash
gh pr edit 1062 --repo senara-solutions/mika --remove-label ready
sleep 5
gh pr edit 1062 --repo senara-solutions/mika --add-label ready
# Wait 60s, check reviews — still showing 14:12Z verdict. No new review.
```

Timeline events on the PR confirmed both label changes fired in GitHub's event stream. mika-qa just didn't react.

**Successful attempt — synchronize event:**

```bash
WT=/data/workspace/mika-platform/.claude/worktrees/docs-1058-compound-doc-enrichment/mika
cd $WT
git commit --allow-empty -m "trigger: re-trigger mika-qa after v0.12.4 deploy"
git push
# Within ~30s: mika-qa re-reviewed, emitted VERDICT: pass.
```

`gh pr view 1062 --json reviews --jq '.reviews | last'` showed new APPROVED review submittedAt `2026-05-10T16:58:21Z`.

### Counter-example — when label apply DOES work

Applying `ready` to a fresh issue (never reviewed before) DOES trigger mika-qa correctly. The debounce is specifically on **re-review of recently-reviewed PRs**, not first-review.

## Related

- mika#1064 / PR #1065 — the qa-review fix that surfaced this re-trigger workflow lesson
- `docs/solutions/prompt-engineering/qa-review-docs-only-pipeline-exempt-gate-2026-05-10.md` — the fix doc PR #1065 wrote (this lesson is about HOW to operate after that fix lands, not the fix itself)
- `feedback_ready_label_dispatch_canonical` (orchestrator memory) — always ready-label/canonical-trigger, never direct mika ask
- mika#1067 — follow-up to extend `pipeline-exempt` label honor to CI verify-pipeline-artifacts (currently uses commit-trailer mechanism — different exemption mechanism for the same intent)
