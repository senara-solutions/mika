#!/usr/bin/env bash
# Verify that the /mika pipeline produced required artifacts before PR creation.
#
# Checks:
#   1. A plan doc exists in docs/plans/*.md (in the branch diff)
#   2. Source code changes exist beyond the plan doc
#
# Usage:
#   ./scripts/verify-pipeline.sh              # local (compares to main)
#   ./scripts/verify-pipeline.sh origin/main  # CI (compares to origin/main)
#
# Exit codes:
#   0 - all checks passed
#   1 - missing artifacts

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

# Check 1: Plan doc in docs/plans/
PLAN=$(echo "$ALL" | grep '^docs/plans/.*\.md$' || true)
if [[ -z "$PLAN" ]]; then
  echo "MISSING: No plan doc in docs/plans/. Run /ce:plan." >&2
  ERRORS=$((ERRORS + 1))
fi

# Check 2: Source changes beyond plan doc and .claude/ config
CODE=$(echo "$ALL" | grep -v '^docs/plans/' | grep -v '^\.claude/' || true)
if [[ -z "$CODE" ]]; then
  echo "MISSING: No source changes beyond plan doc. Run /ce:work." >&2
  ERRORS=$((ERRORS + 1))
fi

if [[ $ERRORS -gt 0 ]]; then
  echo "Verification FAILED: $ERRORS missing artifact(s)." >&2
  exit 1
fi

echo "Pipeline verification passed. Plan: $PLAN"
