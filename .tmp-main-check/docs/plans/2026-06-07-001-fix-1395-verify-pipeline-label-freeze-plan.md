# Plan — fix(ci): verify-pipeline.sh label-freeze (mika#1395)

## Problem

`scripts/verify-pipeline.sh:181` reads PR labels from `$GITHUB_EVENT_PATH` — a frozen snapshot taken at workflow trigger time. Labels applied after workflow start are invisible on rerun. Operator workaround: push an empty commit to fire a fresh workflow with fresh event payload.

## Evidence

- Reproduced today (2026-06-03) on mika PR#1390 (compound doc shipped to #905). Same pattern affected #1392 narrowly.
- Cost per occurrence: ~2-3 min + one extra commit on every docs-only PR with post-applied label.

## Fix

Add a live API fetch fallback. Priority: prefer the frozen event payload (free, no network), fall back to `gh api repos/{owner}/{repo}/pulls/{N}` when label not found in payload OR when payload is empty.

The fall-through ensures:
- Normal case (label present at trigger time): no behavior change, no network call.
- Rerun-after-label-applied case: live fetch picks up the new label.

## Implementation

`scripts/verify-pipeline.sh` around line 181:

```bash
EXEMPT_PR_LABEL_DOCS=0
if [[ -n "${GITHUB_EVENT_PATH:-}" ]]; then
  if jq -e '.pull_request.labels[]? | select(.name == "pipeline-exempt")' "$GITHUB_EVENT_PATH" >/dev/null 2>&1; then
    EXEMPT_PR_LABEL_DOCS=1
  fi
fi

# Live API fallback: if frozen-snapshot didn't find the label, re-fetch.
# Handles label-applied-after-workflow-start case (rerun does not refresh event payload).
if [[ "$EXEMPT_PR_LABEL_DOCS" == "0" ]] && command -v gh >/dev/null 2>&1; then
  _pr_number=$(jq -r '.pull_request.number // empty' "$GITHUB_EVENT_PATH" 2>/dev/null)
  if [[ -n "$_pr_number" ]]; then
    if gh pr view "$_pr_number" --json labels --jq '.labels[].name' 2>/dev/null | grep -qx "pipeline-exempt"; then
      EXEMPT_PR_LABEL_DOCS=1
    fi
  fi
fi
```

Same shape extended to the `documentation` label read at the linked-issue level (separate read site earlier in the script that fetches issue labels via `gh api repos/$_repo/issues/$LINKED_ISSUE`). That site already uses `gh api` so it doesn't have the freeze bug — just the `pipeline-exempt` PR label has it.

## Acceptance criteria

- AC1: Empty-commit workaround no longer needed for post-workflow-start `pipeline-exempt` label.
- AC2: No behavior change when label is applied at PR-create time (label in frozen event payload).
- AC3: Network failure on `gh pr view` falls through gracefully (`EXEMPT_PR_LABEL_DOCS=0`, original frozen-snapshot read still authoritative).
- AC4: Script passes existing shellcheck.

## Test plan

Manual rerun verification on an existing PR:
1. Open a docs-only PR without `pipeline-exempt` label.
2. Confirm Pipeline Artifacts fails.
3. Apply `pipeline-exempt` label via `gh pr edit`.
4. Rerun the failed Pipeline Artifacts job via `gh run rerun --failed`.
5. With fix: passes. Without fix (pre-merge state): still fails — operator needs empty commit.

## Cross-repo port-forward (follow-up, not this PR)

Today's substrate-pivot shipped canonical verify-pipeline.sh to mika-cloud + mika-skills (mika-cloud#109, mika-skills#171). After this fix lands in mika, follow-up port-forwards to mika-platform + mika-cloud + mika-skills + claude-pilot-py keep the canonical script aligned. Tracking question: mika#1434 (CI script sync class observation).
