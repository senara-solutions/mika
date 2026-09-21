#!/usr/bin/env bash
# Verify that the /mika pipeline produced required artifacts before PR creation.
#
# Bucket-comparison logic — categorizes the PR's changed files and rejects
# pathological splits (docs-only or code-only PRs) that mika-platform#17/#18
# established as a recurring failure mode.
#
# Buckets (applied to the union of committed/staged/unstaged diffs vs base):
#   docs    = docs/plans/** or docs/solutions/**
#   source  = everything NOT under docs/, .github/, or .claude/worktrees/
#   other   = the rest (.github/, docs/adr/, docs/brainstorms/, README.md, ...)
#
# Decisions:
#   docs && source           -> pass
#   docs && !source          -> REJECT (docs-only PR)
#   !docs && source          -> REJECT (code-only PR)
#   !docs && !source         -> warn + pass (pure config or no diff)
#
# Exemption mechanisms for docs-only PRs (checked in this order; first match wins):
#
#   1. Issue `documentation` label inheritance (mika#861):
#      If the PR body contains `Closes #N` and issue N carries the
#      `documentation` label, the docs-only rejection is bypassed. This is
#      the PRIMARY classification-driven path — the linked issue is the
#      single source of truth for whether work is docs-only.
#
#   2. PR `pipeline-exempt` label (mika#1067):
#      Set by an operator directly on the PR. Read from
#      `GITHUB_EVENT_PATH` (no gh CLI / token needed). Useful when the
#      linked issue can't be re-labelled (cross-repo, immutable, or
#      already-closed) or for one-off operator overrides.
#
#   3. `Pipeline-Exempt:` commit trailer (mika#860):
#      Any commit in base..HEAD with
#        Pipeline-Exempt: docs-only — <reason>   (preferred, audit trail)
#        Pipeline-Exempt: docs-only               (accepted, warns)
#        Pipeline-Exempt: code-only — <reason>    (preferred)
#        Pipeline-Exempt: code-only               (accepted, warns)
#      Bare form still passes for backwards compat (PR #860 shipped both
#      forms) but emits a warning directing operators to the with-reason
#      form for auditability. Trailer is the residual escape hatch.
#
#   4. Automated PR author (mika#2419):
#      If the PR was opened by a recognized automated author
#      (`dependabot[bot]` / `app/dependabot`, read from
#      `.pull_request.user.login` in `GITHUB_EVENT_PATH`), BOTH bucket
#      rejections are skipped. Such a PR has no plan doc and no
#      `Pipeline-Exempt:` trailer by construction — nothing in its pipeline
#      can produce one.
#
#      Scope: the two bucket checks ONLY. The mika#1600 `## Acceptance
#      criteria` check above still applies — if an automated PR does carry a
#      plan, that plan is still verified. An exemption should be the narrowest
#      one that repairs the measured defect.
#
#      Discriminant is the AUTHOR, never the branch prefix.
#      `.github/workflows/ci.yml` skips this whole job on
#      `startsWith(github.head_ref, 'dependabot/')` because an `if:` expression
#      has nothing else available; this script has `user.login`, and a branch
#      prefix is spoofable (a human naming a branch `dependabot/foo` would walk
#      through the plan gate). The two lists are deliberately NOT parity-linted:
#      they answer different questions ("is a runner worth spending?" vs "is
#      this PR subject to the gate?") and comparing branch prefixes to logins
#      would compare two kinds of thing. The asymmetry of risk is what makes
#      that acceptable — this script stricter than ci.yml yields a visible
#      false `block`; this script more permissive is dangerous, and is bounded
#      by an explicit two-login list no code path builds dynamically.
#
#      FAIL-CLOSED, and fail-closed here means CONTINUE WITHOUT EXEMPTING,
#      never ABORT: no event file, unreadable file, absent field, empty login,
#      or no `jq` on PATH => no exemption, checks proceed normally. A local run
#      therefore never exempts. See the `2>/dev/null || echo ""` note at the
#      resolution site for why the distinction is load-bearing under `set -e`.
#
#   5. Reject — no exemption found, exit 1.
#
# One-directional asymmetry (load-bearing):
#   The `documentation` issue label and the `pipeline-exempt` PR label
#   exempt the source-required check ONLY. They do NOT exempt the
#   docs-required-when-source-changes check. A PR with source changes
#   still needs a plan/solution doc regardless of label. This preserves
#   the protection from mika-platform#17. Only the `Pipeline-Exempt:
#   code-only` trailer bypasses the code-only rejection.
#
# Path-pattern auto-exemption was considered and rejected (mika#861):
#   (1) Silent green CI with multiple exemption paths erodes structural
#       visibility — operators can't tell at a glance which path allowed
#       a green check.
#   (2) Path-touching is an artifact of classification, not the
#       classification itself (a docs-only PR is one whose intent is
#       documentation, not one whose paths happen to start with `docs/`).
#       Gating on the artifact rather than the decision is an inversion
#       that erodes protections over time.
#   (3) Creates a parallel taxonomy alongside `.github/labels.yml`
#       (DRY violation).
#
# Cross-repo `Closes` references (e.g., `Closes senara-solutions/other#N`)
#   are treated as "no linked issue in this repo" — the `gh api` call would
#   404. This is intentional: the cross-repo split pattern is what
#   mika-platform#17 protects against. Use the PR `pipeline-exempt` label
#   or the trailer with reason for legitimate cross-repo docs-only ships.
#
# Note: this script previously enforced an unconditional plan-doc-presence
# AND compound-doc-presence check. Both were strictly subsumed by the bucket
# logic — docs/plans presence is covered by DOCS_BUCKET, source presence by
# SOURCE_BUCKET, compound docs satisfy DOCS_BUCKET, and the exempt trailers
# provide the escape hatch for legitimate docs-only / code-only PRs (e.g.
# standalone /ce:compound shipments). Running the strict checks in addition
# rejected legitimate /ce:compound docs-only PRs with no escape mechanism.
# Aligned with mika-platform/scripts/verify-pipeline.sh.
#
# Usage:
#   ./scripts/verify-pipeline.sh              # local (compares to main)
#   ./scripts/verify-pipeline.sh origin/main  # CI (compares to origin/main or base SHA)
#
# Exit codes:
#   0 - all checks passed (possibly with warnings)
#   1 - missing artifacts or pathological split

set -euo pipefail
cd "$(dirname "$0")/.."

BASE_REF="${1:-main}"
MERGE_BASE=$(git merge-base "$BASE_REF" HEAD 2>/dev/null || echo "$BASE_REF")

# Collect all changed files: committed + staged + unstaged
COMMITTED=$(git diff "$MERGE_BASE" HEAD --name-only 2>/dev/null || true)
STAGED=$(git diff --cached --name-only 2>/dev/null || true)
UNSTAGED=$(git diff --name-only 2>/dev/null || true)
ALL=$(printf '%s\n%s\n%s' "$COMMITTED" "$STAGED" "$UNSTAGED" | sort -u | grep -v '^$' || true)

ERRORS=0

# Capture PLAN purely for the final "passed" message.
PLAN=$(echo "$ALL" | grep '^docs/plans/.*\.md$' || true)
COMPOUND=$(echo "$ALL" | grep '^docs/solutions/.*\.md$' || true)

# --- mika#1600: Acceptance criteria section check ---
if [[ -n "$PLAN" ]]; then
  # Take the first plan file if multiple exist
  PLAN_FILE=$(echo "$PLAN" | head -1)
  if [[ -f "$PLAN_FILE" ]]; then
    # Check for ## Acceptance criteria heading (case-insensitive: the autonomous
    # groom command instructs lowercase, but the LLM frequently title-cases the
    # noun phrase — mika#1639. `grep -qi` / sed start-address `I` flag (both GNU)
    # fold case so `## Acceptance Criteria` is accepted.)
    if ! grep -qi '^## Acceptance criteria' "$PLAN_FILE"; then
      echo "FAIL: Plan '$PLAN_FILE' missing '## Acceptance criteria' section. See mika#1600." >&2
      ERRORS=$((ERRORS + 1))
    else
      # Check for at least one non-blank line after the heading. Only the start
      # address needs the `I` flag; the end address `/^## /` and inner `/^## /d`
      # already match any `## ` heading prefix case-agnostically.
      AC_CONTENT=$(sed -n '/^## Acceptance criteria/I,/^## /{ /^## /d; /^[[:space:]]*$/d; p; }' "$PLAN_FILE")
      if [[ -z "$AC_CONTENT" ]]; then
        echo "FAIL: Plan '$PLAN_FILE' has empty '## Acceptance criteria' section. See mika#1600." >&2
        ERRORS=$((ERRORS + 1))
      fi
    fi
  fi
fi

DOCS_BUCKET=$(echo "$ALL" | grep -E '^docs/(plans|solutions)/' || true)
SOURCE_BUCKET=$(echo "$ALL" \
  | grep -v -E '^docs/' \
  | grep -v -E '^\.github/' \
  | grep -v -E '^\.claude/worktrees/' \
  || true)

# OTHER = ALL minus DOCS_BUCKET minus SOURCE_BUCKET
if [[ -n "$ALL" ]]; then
  EXCLUDE=$(printf '%s\n%s\n' "$DOCS_BUCKET" "$SOURCE_BUCKET" | grep -v '^$' || true)
  if [[ -n "$EXCLUDE" ]]; then
    OTHER_BUCKET=$(echo "$ALL" | grep -v -F -x -f <(echo "$EXCLUDE") || true)
  else
    OTHER_BUCKET="$ALL"
  fi
else
  OTHER_BUCKET=""
fi

# --- mika#861: Label inheritance from linked issue ---
# Parse `Closes #N` from PR body to identify linked issue.
# Sources (priority order): GITHUB_PR_BODY env var (CI), gh pr view (fallback).
# Branch-name fallback intentionally omitted — silent misfire on branches like
# `feature/v2/...` is worse than no fallback (see plan F2).
# mika#1334: Prefer GitHub-parsed closing issue reference over regex-on-body.
# GitHub's parser ignores quoted/code-block refs and is bound to the PR via
# its own auto-close engine — not author-controlled scraping. Falls through
# to body-regex when gh unavailable (no-network / local-dev).
LINKED_ISSUE=""
if command -v gh >/dev/null 2>&1; then
  _closing_refs=$(gh pr view --json closingIssuesReferences --jq '.closingIssuesReferences[].number' 2>/dev/null || echo "")
  if [ -n "$_closing_refs" ]; then
    LINKED_ISSUE=$(echo "$_closing_refs" | head -1)
  fi
fi
# Fallback: body-regex preserved for the no-network / local-dev case.
if [ -z "$LINKED_ISSUE" ] && [ -n "${GITHUB_PR_BODY:-}" ]; then
  LINKED_ISSUE=$(echo "$GITHUB_PR_BODY" | grep -oE '(Closes|Fixes|Resolves) #[0-9]+' | head -1 | grep -oE '[0-9]+' || true)
fi
if [ -z "$LINKED_ISSUE" ] && command -v gh >/dev/null 2>&1; then
  _pr_body=$(gh pr view --json body --jq .body 2>/dev/null || echo "")
  LINKED_ISSUE=$(echo "$_pr_body" | grep -oE '(Closes|Fixes|Resolves) #[0-9]+' | head -1 | grep -oE '[0-9]+' || true)
fi

ISSUE_HAS_DOCUMENTATION_LABEL=false
if [ -n "$LINKED_ISSUE" ]; then
  _repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null || echo "")
  if [ -n "$_repo" ]; then
    _labels=$(gh api "repos/$_repo/issues/$LINKED_ISSUE" --jq '.labels[].name' 2>/dev/null || echo "")
    if grep -qx -- "documentation" <<<"$_labels"; then
      ISSUE_HAS_DOCUMENTATION_LABEL=true
    fi
  fi
fi

# --- Exempt trailers: scan commit messages in base..HEAD ---
COMMIT_BODIES=$(git log --format=%B "${MERGE_BASE}..HEAD" 2>/dev/null || true)
EXEMPT_DOCS_ONLY=0
EXEMPT_CODE_ONLY=0
EXEMPT_DOCS_REASON=""
EXEMPT_CODE_REASON=""

# Dual-form matching: with-reason (preferred) vs bare (backwards compat, warns)
if grep -qE -- '^Pipeline-Exempt: docs-only[[:space:]]+.+$' <<<"$COMMIT_BODIES"; then
  EXEMPT_DOCS_ONLY=1
  EXEMPT_DOCS_REASON=$(echo "$COMMIT_BODIES" | grep -oE '^Pipeline-Exempt: docs-only[[:space:]]+.+$' | head -1 | sed 's/^Pipeline-Exempt: docs-only[[:space:]]*//')
elif grep -qE -- '^Pipeline-Exempt: docs-only[[:space:]]*$' <<<"$COMMIT_BODIES"; then
  EXEMPT_DOCS_ONLY=1
  EXEMPT_DOCS_REASON=""
fi

if grep -qE -- '^Pipeline-Exempt: code-only[[:space:]]+.+$' <<<"$COMMIT_BODIES"; then
  EXEMPT_CODE_ONLY=1
  EXEMPT_CODE_REASON=$(echo "$COMMIT_BODIES" | grep -oE '^Pipeline-Exempt: code-only[[:space:]]+.+$' | head -1 | sed 's/^Pipeline-Exempt: code-only[[:space:]]*//')
elif grep -qE -- '^Pipeline-Exempt: code-only[[:space:]]*$' <<<"$COMMIT_BODIES"; then
  EXEMPT_CODE_ONLY=1
  EXEMPT_CODE_REASON=""
fi

# --- PR `pipeline-exempt` label exemption (mika#1067) ---
# Read PR labels from GitHub Actions event payload. GITHUB_EVENT_PATH is always
# set in GitHub Actions; contains the full event JSON. For pull_request events,
# labels are at .pull_request.labels[].name. No gh CLI or GITHUB_TOKEN needed —
# reads a local file.
EXEMPT_PR_LABEL_DOCS=0
if [[ -n "${GITHUB_EVENT_PATH:-}" ]]; then
  if jq -e '.pull_request.labels[]? | select(.name == "pipeline-exempt")' "$GITHUB_EVENT_PATH" >/dev/null 2>&1; then
    EXEMPT_PR_LABEL_DOCS=1
  fi
fi

# mika#1395: Live API fallback for label-freeze on workflow rerun.
# $GITHUB_EVENT_PATH is a frozen snapshot taken at workflow trigger time;
# labels applied after the workflow starts are invisible to the gate on
# `gh run rerun --failed`. Fall back to a live `gh pr view` fetch when the
# frozen snapshot did not surface the label, so operators don't need to
# push an empty commit just to re-trigger CI for label visibility.
if [[ "$EXEMPT_PR_LABEL_DOCS" == "0" ]] && command -v gh >/dev/null 2>&1 \
   && [[ -n "${GITHUB_EVENT_PATH:-}" ]] && [[ -f "$GITHUB_EVENT_PATH" ]]; then
  _pr_number=$(jq -r '.pull_request.number // empty' "$GITHUB_EVENT_PATH" 2>/dev/null || echo "")
  if [[ -n "$_pr_number" ]]; then
    if gh pr view "$_pr_number" --json labels --jq '.labels[].name' 2>/dev/null | grep -qx "pipeline-exempt"; then
      EXEMPT_PR_LABEL_DOCS=1
    fi
  fi
fi

# --- Automated PR author exemption (mika#2419) ---
# Mirrors the intent of `.github/workflows/ci.yml`'s pipeline-artifacts branch
# exclusion, on the author rather than the branch prefix. See mechanism 4 in
# the header for the full rationale, the scope, and why the two lists are not
# parity-linted.
#
# Exact equality against every entry — never a substring test: an author named
# `not-dependabot[bot]` must not match `dependabot[bot]`. Both forms are listed
# because `gh` renders the identity either way, and qa-review's Step 1.6
# already recognizes both.
AUTOMATED_PR_AUTHORS=("dependabot[bot]" "app/dependabot")
EXEMPT_AUTOMATED_AUTHOR=0
AUTOMATED_AUTHOR=""
if [[ -n "${GITHUB_EVENT_PATH:-}" ]] && [[ -f "$GITHUB_EVENT_PATH" ]]; then
  # `2>/dev/null || echo ""` is load-bearing, not defensive habit. This script
  # runs under `set -euo pipefail` (see the top), so a bare
  # `jq -r '...' "$GITHUB_EVENT_PATH"` ABORTS the whole script when `jq` is
  # absent or the event file is malformed — with jq's exit code. qa-review's
  # Step 2C reads any non-zero exit, without judgment, as `block[pipeline]`,
  # so a PR that passes today would start failing: this exemption would
  # produce the exact symptom it exists to close. Same defensive shape already
  # used twice above (the mika#1395 live-label fallback).
  # Pinned by F7 in scripts/verify-pipeline-test.sh.
  _event_author=$(jq -r '.pull_request.user.login // empty' "$GITHUB_EVENT_PATH" 2>/dev/null || echo "")
  if [[ -n "$_event_author" ]]; then
    for _known_author in "${AUTOMATED_PR_AUTHORS[@]}"; do
      if [[ "$_event_author" == "$_known_author" ]]; then
        EXEMPT_AUTOMATED_AUTHOR=1
        AUTOMATED_AUTHOR="$_event_author"
        break
      fi
    done
  fi
fi

# --- Docs-only check (labels exempt source-required; trailer is escape hatch) ---
if [[ -n "$DOCS_BUCKET" && -z "$SOURCE_BUCKET" ]]; then
  if [[ "$EXEMPT_AUTOMATED_AUTHOR" == "1" ]]; then
    echo "info: [pipeline-exempt: automated-author] $AUTOMATED_AUTHOR: bucket checks skipped (mirrors ci.yml pipeline-artifacts branch exclusion, mika#2010)" >&2
  elif [ "$ISSUE_HAS_DOCUMENTATION_LABEL" = true ]; then
    echo "info: [pipeline-exempt: issue-label] docs-only PR allowed by linked-issue documentation label (#$LINKED_ISSUE)" >&2
  elif [[ "$EXEMPT_PR_LABEL_DOCS" == "1" ]]; then
    echo "info: [pipeline-exempt: pr-label] docs-only PR allowed by pipeline-exempt PR label" >&2
  elif [[ "$EXEMPT_DOCS_ONLY" == "1" ]]; then
    if [ -n "$EXEMPT_DOCS_REASON" ]; then
      echo "info: [pipeline-exempt: trailer] docs-only PR allowed by Pipeline-Exempt trailer with reason: $EXEMPT_DOCS_REASON" >&2
    else
      echo "warn: [pipeline-exempt: trailer] bare Pipeline-Exempt: docs-only trailer detected; prefer 'Pipeline-Exempt: docs-only — <reason>' for audit trail" >&2
    fi
  else
    echo "[pipeline-exempt: none] REJECT: docs-only PR: plan/solution present but no source changes" >&2
    echo "        Add the 'documentation' label to the linked issue (preferred), or apply" >&2
    echo "        the 'pipeline-exempt' label to the PR, or add" >&2
    echo "        'Pipeline-Exempt: docs-only — <reason>' trailer to a commit" >&2
    echo "        if this docs-only ship is intentional (e.g. standalone /ce:compound)." >&2
    ERRORS=$((ERRORS + 1))
  fi
fi

# --- Code-only check (label does NOT exempt docs-required-when-source-changes) ---
if [[ -z "$DOCS_BUCKET" && -n "$SOURCE_BUCKET" ]]; then
  if [[ "$EXEMPT_AUTOMATED_AUTHOR" == "1" ]]; then
    echo "info: [pipeline-exempt: automated-author] $AUTOMATED_AUTHOR: bucket checks skipped (mirrors ci.yml pipeline-artifacts branch exclusion, mika#2010)" >&2
  elif [[ "$EXEMPT_CODE_ONLY" == "1" ]]; then
    if [ -n "$EXEMPT_CODE_REASON" ]; then
      echo "info: [pipeline-exempt: trailer] code-only PR allowed by Pipeline-Exempt trailer with reason: $EXEMPT_CODE_REASON" >&2
    else
      echo "warn: [pipeline-exempt: trailer] bare Pipeline-Exempt: code-only trailer detected; prefer 'Pipeline-Exempt: code-only — <reason>' for audit trail" >&2
    fi
  else
    echo "[pipeline-exempt: none] REJECT: code-only PR: source changes present but no plan/solution doc" >&2
    echo "        Add 'Pipeline-Exempt: code-only — <reason>' trailer to a commit" >&2
    echo "        if this code-only ship is intentional." >&2
    ERRORS=$((ERRORS + 1))
  fi
fi

if [[ -z "$DOCS_BUCKET" && -z "$SOURCE_BUCKET" ]]; then
  if [[ -n "$OTHER_BUCKET" ]]; then
    echo "warn: no docs or source changes, only config/other files" >&2
  else
    echo "warn: no diff against $BASE_REF" >&2
  fi
fi

if [[ $ERRORS -gt 0 ]]; then
  echo "Verification FAILED: $ERRORS missing artifact(s)." >&2
  exit 1
fi

echo "Pipeline verification passed. Plan: ${PLAN:-<none>} Compound: ${COMPOUND:-<none>}"
