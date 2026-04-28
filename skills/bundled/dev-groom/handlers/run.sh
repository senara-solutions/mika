#!/bin/sh
# Thin convenience handler for the dev-groom skill.
#
# Provides deterministic IO that benefits from scripted centralization:
# - Branch slug derivation from issue title + labels
# - Plan-file sequence number incrementing
# - Worktree idempotency check
#
# This is NOT a long-running exec handler. The grooming flow is LLM-driven
# via system_prompt.md; this handler provides deterministic helpers only.
#
# Input: JSON on stdin with fields:
#   repo        — target repository name (e.g., "mika", "mika-cloud")
#   issue       — issue number
#   branch      — optional explicit branch slug override
#
# Output: JSON with derived values for the LLM to consume.

set -e

export PATH="$HOME/.local/bin:$PATH"

# Dependency checks
command -v jq >/dev/null 2>&1 || { echo '{"error": "jq is required but not installed"}'; exit 1; }
command -v gh >/dev/null 2>&1 || { echo '{"error": "gh CLI is required but not installed"}'; exit 1; }

# Read input JSON from stdin
INPUT=$(cat)

REPO=$(printf '%s\n' "$INPUT" | jq -r '.repo // empty')
ISSUE=$(printf '%s\n' "$INPUT" | jq -r '.issue // empty')
BRANCH_OVERRIDE=$(printf '%s\n' "$INPUT" | jq -r '.branch // empty')

if [ -z "$REPO" ] || [ -z "$ISSUE" ]; then
    echo '{"error": "repo and issue fields are required"}'
    exit 1
fi

# Fetch issue details
ISSUE_JSON=$(gh issue view "$ISSUE" --repo "senara-solutions/$REPO" --json title,body,labels 2>&1) || {
    printf '%s' "$ISSUE_JSON" | jq -Rsc '{ error: ("failed to fetch issue: " + .) }'
    exit 1
}

TITLE=$(printf '%s\n' "$ISSUE_JSON" | jq -r '.title // empty')
LABELS=$(printf '%s\n' "$ISSUE_JSON" | jq -r '[.labels[].name] | join(",")' 2>/dev/null || echo "")
BODY=$(printf '%s\n' "$ISSUE_JSON" | jq -r '.body // empty')

# Check for existing branch callout in issue body
EXISTING_BRANCH=$(printf '%s\n' "$BODY" | sed -n 's/.*> - \*\*Branch:\*\* `\([^`]*\)`.*/\1/p' | head -1)

# Branch slug derivation
if [ -n "$BRANCH_OVERRIDE" ]; then
    BRANCH="$BRANCH_OVERRIDE"
elif [ -n "$EXISTING_BRANCH" ]; then
    BRANCH="$EXISTING_BRANCH"
else
    # Determine type from labels
    case "$LABELS" in
        *enhancement*) TYPE="feat" ;;
        *bug*)         TYPE="fix" ;;
        *)             TYPE="chore" ;;
    esac

    # Check if title already has conventional prefix
    if printf '%s' "$TITLE" | grep -qE '^(feat|fix|chore|docs|eval|test|refactor|perf)(\([^)]+\))?: '; then
        TYPE=$(printf '%s' "$TITLE" | sed -nE 's/^([a-z]+)(\([^)]+\))?: .*$/\1/p')
        SLUG_BODY=$(printf '%s' "$TITLE" | sed -E 's/^[a-z]+(\([^)]+\))?: *//')
    else
        SLUG_BODY="$TITLE"
    fi

    SLUG=$(printf '%s' "$SLUG_BODY" | tr '[:upper:]' '[:lower:]' \
        | LC_ALL=C sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' \
        | cut -c1-45 | sed -E 's/-[^-]*$//; s/-+$//')

    BRANCH="${TYPE}/${ISSUE}/${SLUG}"
fi

# Sanitize branch for worktree path
SANITIZED=$(printf '%s' "$BRANCH" | tr '/' '-')

# Resolve repo root for plan file sequencing
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || pwd)
PLAN_DIR="$REPO_ROOT/docs/plans"

# Plan file sequence number
TODAY=$(date +%Y-%m-%d)

# Extract the slug tail for the plan filename
SLUG_TAIL=$(printf '%s' "$BRANCH" | sed 's|.*/||')

# Find next sequence number for today
EXISTING_COUNT=$(find "$PLAN_DIR" -name "${TODAY}-*" -maxdepth 1 2>/dev/null | wc -l || echo "0")
NEXT_SEQ=$(printf '%03d' $((EXISTING_COUNT + 1)))

# Determine plan type from branch
PLAN_TYPE=$(printf '%s' "$BRANCH" | sed -nE 's|^([^/]+)/.*|\1|p')
: "${PLAN_TYPE:=feat}"

PLAN_FILE="${TODAY}-${NEXT_SEQ}-${PLAN_TYPE}-${SLUG_TAIL}-plan.md"
# Output relative path so the LLM sees docs/plans/... not an absolute path
PLAN_PATH="docs/plans/${PLAN_FILE}"

# Worktree path
WORKTREE_BASE="../../.claude/worktrees/${SANITIZED}/${REPO}"
WORKTREE_EXISTS="false"
if [ -d "$WORKTREE_BASE" ]; then
    WORKTREE_EXISTS="true"
fi

# Output derived values
jq -n \
    --arg branch "$BRANCH" \
    --arg sanitized "$SANITIZED" \
    --arg plan_path "$PLAN_PATH" \
    --arg plan_file "$PLAN_FILE" \
    --arg worktree_path "$WORKTREE_BASE" \
    --arg worktree_exists "$WORKTREE_EXISTS" \
    --arg title "$TITLE" \
    --arg repo "$REPO" \
    --arg issue "$ISSUE" \
    '{
        branch: $branch,
        sanitized_branch: $sanitized,
        plan_path: $plan_path,
        plan_file: $plan_file,
        worktree_path: $worktree_path,
        worktree_exists: ($worktree_exists == "true"),
        title: $title,
        repo: $repo,
        issue: $issue
    }'
