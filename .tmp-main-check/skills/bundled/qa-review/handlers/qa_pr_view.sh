#!/usr/bin/env bash
set -euo pipefail

# qa_pr_view — fetch PR metadata with CI-related fields stripped.
#
# CI fields (statusCheckRollup, statusChecks, mergeStateStatus, mergeable,
# commits) are excluded so the reviewing LLM never sees CI data.
#
# Input: JSON on stdin with "pr_url" field
# Output: PR metadata JSON on stdout
#
# See: docs/solutions/prompt-engineering/2026-04-08-remove-capability-not-prohibit-it.md

command -v gh &>/dev/null || { echo "ERROR: gh CLI not found on PATH" >&2; exit 1; }
command -v jq &>/dev/null || { echo "ERROR: jq not found on PATH" >&2; exit 1; }

INPUT=$(cat)

PR_URL=$(printf '%s\n' "$INPUT" | jq -r '.pr_url // empty')
if [[ -z "$PR_URL" ]]; then
  echo "ERROR: pr_url is required" >&2
  exit 1
fi

# Validate pr_url format to prevent option injection (e.g., --json statusCheckRollup).
# Accepts: https://github.com/owner/repo/pull/123 or owner/repo#123
if [[ ! "$PR_URL" =~ ^https://github\.com/[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+/pull/[0-9]+$ ]] && \
   [[ ! "$PR_URL" =~ ^[a-zA-Z0-9._-]+/[a-zA-Z0-9._-]+#[0-9]+$ ]]; then
  echo "ERROR: pr_url must be a GitHub PR URL (https://github.com/owner/repo/pull/N) or owner/repo#N format" >&2
  exit 1
fi

# Hardcoded safe fields — NO CI metadata
SAFE_FIELDS="title,body,additions,deletions,files,labels,state,headRefName,headRefOid,baseRefName,author"

gh pr view "$PR_URL" --json "$SAFE_FIELDS"
