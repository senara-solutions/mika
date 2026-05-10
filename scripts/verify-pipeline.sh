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
# Exemptions:
#   pipeline-exempt PR label    -> bypass docs-only rejection (preferred, mika#1067)
#   Pipeline-Exempt: docs-only  -> bypass docs-only rejection (trailer fallback)
#   Pipeline-Exempt: code-only  -> bypass code-only rejection
#
# Note: this script previously enforced an unconditional plan-doc-presence
# AND compound-doc-presence check. Both were strictly subsumed by the bucket
# logic — docs/plans presence is covered by DOCS_BUCKET, source presence by
# SOURCE_BUCKET, compound docs satisfy DOCS_BUCKET, and the exempt trailers
# provide the escape hatch for legitimate docs-only / code-only PRs (e.g.
# standalone /ce:compound shipments). Running the strict checks in addition
# rejected legitimate /ce:compound docs-only PRs with no escape mechanism.
# Aligned with mika-platform/scripts/verify-pipeline.sh; mika#861 tracks
# layering label-inheritance on top of this for the as-above-so-below path.
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

# Exempt trailers: scan commit messages in base..HEAD
COMMIT_BODIES=$(git log --format=%B "${MERGE_BASE}..HEAD" 2>/dev/null || true)
EXEMPT_DOCS_ONLY=0
EXEMPT_CODE_ONLY=0
if echo "$COMMIT_BODIES" | grep -qE '^Pipeline-Exempt: docs-only(\s.*)?$'; then
  EXEMPT_DOCS_ONLY=1
fi
if echo "$COMMIT_BODIES" | grep -qE '^Pipeline-Exempt: code-only(\s.*)?$'; then
  EXEMPT_CODE_ONLY=1
fi

# Label-based exemption: read PR labels from GitHub Actions event payload
# GITHUB_EVENT_PATH is always set in GitHub Actions; contains the full event JSON.
# For pull_request events, labels are at .pull_request.labels[].name.
# No gh CLI or GITHUB_TOKEN needed — reads a local file.
EXEMPT_LABEL_DOCS=0
if [[ -n "${GITHUB_EVENT_PATH:-}" ]]; then
  if jq -e '.pull_request.labels[]? | select(.name == "pipeline-exempt")' "$GITHUB_EVENT_PATH" >/dev/null 2>&1; then
    EXEMPT_LABEL_DOCS=1
  fi
fi

if [[ -n "$DOCS_BUCKET" && -z "$SOURCE_BUCKET" ]]; then
  if [[ "$EXEMPT_DOCS_ONLY" == "1" || "$EXEMPT_LABEL_DOCS" == "1" ]]; then
    if [[ "$EXEMPT_LABEL_DOCS" == "1" ]]; then
      echo "warn: docs-only PR allowed by pipeline-exempt label" >&2
    else
      echo "warn: docs-only PR allowed by Pipeline-Exempt: docs-only trailer" >&2
    fi
  else
    echo "REJECT: docs-only PR: plan/solution present but no source changes" >&2
    echo "        Apply the 'pipeline-exempt' label to the PR (preferred), or add" >&2
    echo "        'Pipeline-Exempt: docs-only — <reason>' trailer to a commit." >&2
    ERRORS=$((ERRORS + 1))
  fi
fi

if [[ -z "$DOCS_BUCKET" && -n "$SOURCE_BUCKET" ]]; then
  if [[ "$EXEMPT_CODE_ONLY" == "1" ]]; then
    echo "warn: code-only PR allowed by Pipeline-Exempt: code-only trailer" >&2
  else
    echo "REJECT: code-only PR: source changes present but no plan/solution doc" >&2
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
