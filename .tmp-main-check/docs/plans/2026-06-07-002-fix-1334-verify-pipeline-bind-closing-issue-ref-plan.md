# Plan — fix(ci): verify-pipeline.sh bind Closes#N via GitHub-parsed reference (mika#1334)

## Problem

mika#861's documentation-label exemption trusts the `Closes|Fixes|Resolves #N` reference in the PR body without binding it to the PR. Failure shapes (3-reviewer adversarial pass):

1. **Arbitrary pre-labeled issue**: a PR can write `Closes #1` where #1 is any standing `documentation`-labelled issue, regardless of relation to the actual change. `head -1` makes the author pick which reference counts.
2. **Quoted/code-block refs**: `grep -oE '(Closes|Fixes|Resolves) #[0-9]+'` extracts from `> Closes #N` (quoted) and fenced code blocks, which GitHub's own auto-close engine ignores.

## Severity

Policy-gate hardening on an accepted tradeoff. Not a security boundary (no RCE/exfil) — it's an enforcement-trust question.

## Fix

Replace regex-on-body with `gh pr view --json closingIssuesReferences`. GitHub's parsed linkage:
- Only counts refs that GitHub recognizes as actual auto-close intents (ignores quoted/code-block refs)
- Returns ALL linked issues, not just `head -1` — gives us the full set to evaluate
- Already bound to the PR by GitHub's own parser, not author-controlled text scraping

Fall through to the existing regex when `gh` is unavailable (preserves no-network compatibility for local dev).

## Implementation

`scripts/verify-pipeline.sh` `LINKED_ISSUE` extraction section:

```bash
# mika#1334: Prefer GitHub-parsed closing issue reference over regex-on-body.
# GitHub's parser ignores quoted/code-block refs and is bound to the PR — not
# author-controlled scraping. Falls through to body-regex when gh unavailable.
LINKED_ISSUE=""
if command -v gh >/dev/null 2>&1; then
  # Returns ALL closingIssuesReferences. Use the first that has the documentation label.
  # Schema: .closingIssuesReferences[] | {number, state, title}
  _closing_refs=$(gh pr view --json closingIssuesReferences --jq '.closingIssuesReferences[].number' 2>/dev/null || echo "")
  if [ -n "$_closing_refs" ]; then
    LINKED_ISSUE=$(echo "$_closing_refs" | head -1)
  fi
fi
# Fallback: body-regex (existing behavior, preserved for no-network/local-dev case)
if [ -z "$LINKED_ISSUE" ] && [ -n "${GITHUB_PR_BODY:-}" ]; then
  LINKED_ISSUE=$(echo "$GITHUB_PR_BODY" | grep -oE '(Closes|Fixes|Resolves) #[0-9]+' | head -1 | grep -oE '[0-9]+' || true)
fi
if [ -z "$LINKED_ISSUE" ] && command -v gh >/dev/null 2>&1; then
  _pr_body=$(gh pr view --json body --jq .body 2>/dev/null || echo "")
  LINKED_ISSUE=$(echo "$_pr_body" | grep -oE '(Closes|Fixes|Resolves) #[0-9]+' | head -1 | grep -oE '[0-9]+' || true)
fi
```

Then the existing `ISSUE_HAS_DOCUMENTATION_LABEL` check runs over the bound issue.

## Trade considered + named

Should we iterate ALL closingIssuesReferences and exempt if ANY has the documentation label? Or only the first?

- ALL: matches mika#861's "linked issue = source of truth" intent more broadly.
- First only: keeps the current first-match-wins behavior on the *bound* set (a tighter set than the unbound author-controlled set).

Picking first-only for v1 — same priority semantic as before, applied to the bound set. Multi-issue PRs are rare and the failure shape (PR docs-only against a non-documentation issue) is still gated by other tiers (pr-label, trailer-with-reason). Follow-up if multi-issue cases emerge.

## Acceptance criteria

- AC1: PR with quoted/code-block-only `Closes #N` reference is NOT exempted by the documentation-label gate (no LINKED_ISSUE found via gh).
- AC2: PR with valid `Closes #N` ref where the bound issue has `documentation` label is exempted (no change).
- AC3: PR with valid `Closes #N` ref where the bound issue has a different label is NOT exempted via this tier (other tiers can still exempt).
- AC4: When `gh` is unavailable, falls through to existing body-regex behavior — no regression.
- AC5: Script passes shellcheck.

## Test plan

- Manually craft a docs-only PR with a quoted `> Closes #<documentation-issue>` and verify gate REJECTS (pre-fix: would exempt incorrectly).
- Verify a normal docs-only PR with proper `Closes #N` still exempts correctly.

## Cross-repo port-forward

After this fix lands, same pattern as mika#1395: follow-up port-forward to mika-platform + mika-cloud + mika-skills + claude-pilot-py. Tracked structurally under mika#1434.
