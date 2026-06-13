#!/usr/bin/env bash
# CI gate: validate PR body for closure-consistency and follow-up tracking.
# See: https://github.com/senara-solutions/mika/issues/527
#
# Check A (closure-consistency): When PR body says `Closes #N`, walk #N's
# formal sub-issue list (GitHub GraphQL trackedIssues); if any are OPEN,
# hard-fail unless the PR body explicitly acknowledges with `Tracked in: <ref>`.
#
# Check B (follow-up tracker): When PR body contains a follow-up-deferral
# trigger phrase, require a `Tracked in: <ref>` line naming the tracker.
#
# Usage: bash scripts/check-pr-body-consistency.sh <PR_NUMBER>
# Env:   GH_TOKEN (or GITHUB_TOKEN) must be set for `gh` CLI auth.
#        GITHUB_REPOSITORY must be set (format: owner/repo).

set -euo pipefail

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

PR_NUMBER="${1:-}"
if [[ -z "$PR_NUMBER" ]]; then
    echo "Usage: $0 <PR_NUMBER>" >&2
    exit 2
fi

# GITHUB_REPOSITORY is set automatically in GitHub Actions (owner/repo).
REPO="${GITHUB_REPOSITORY:-}"
if [[ -z "$REPO" ]]; then
    # Fallback: derive from git remote.
    REPO="$(git remote get-url origin 2>/dev/null \
        | sed -E 's#.*github\.com[:/]##; s/\.git$//')" || true
fi
if [[ -z "$REPO" ]]; then
    echo "ERROR: Could not determine repository (set GITHUB_REPOSITORY)." >&2
    exit 2
fi

OWNER="${REPO%%/*}"
REPO_NAME="${REPO##*/}"

VIOLATIONS=0
ERROR_MESSAGES=""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

add_error() {
    ERROR_MESSAGES+="$1"$'\n'
    VIOLATIONS=$((VIOLATIONS + 1))
}

# get_open_sub_issues <parent_number>
#
# Queries GitHub GraphQL for OPEN tracked (sub) issues of a parent issue.
# Prints one issue number per line. Returns 0 on success, 1 on API error.
get_open_sub_issues() {
    local parent_number="$1"
    local query
    query='query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      trackedIssues(first: 50, states: OPEN) {
        totalCount
        nodes { number }
      }
    }
  }
}'

    local result
    result="$(gh api graphql \
        -F owner="$OWNER" \
        -F name="$REPO_NAME" \
        -F number="$parent_number" \
        -f query="$query" 2>&1)" || {
        echo "ERROR: GraphQL query failed for issue #${parent_number}: ${result}" >&2
        return 1
    }

    # Check if issue exists.
    local issue_node
    issue_node="$(echo "$result" | jq -r '.data.repository.issue')"
    if [[ "$issue_node" == "null" ]]; then
        echo "ERROR: Issue #${parent_number} not found in ${REPO}." >&2
        return 1
    fi

    local total_count
    total_count="$(echo "$result" | jq -r '.data.repository.issue.trackedIssues.totalCount')"
    if [[ "$total_count" -gt 50 ]]; then
        echo "WARNING: Issue #${parent_number} has ${total_count} open sub-issues; only the first 50 are checked." >&2
    fi

    echo "$result" | jq -r '.data.repository.issue.trackedIssues.nodes[].number'
}

# ---------------------------------------------------------------------------
# Fetch PR body
# ---------------------------------------------------------------------------

PR_BODY="$(gh pr view "$PR_NUMBER" --repo "$REPO" --json body --jq .body 2>&1)" || {
    echo "ERROR: Could not fetch PR #${PR_NUMBER} body: ${PR_BODY}" >&2
    exit 2
}

if [[ -z "$PR_BODY" ]]; then
    echo "PR #${PR_NUMBER} has an empty body — nothing to validate."
    exit 0
fi

# ---------------------------------------------------------------------------
# Check A: Closure-consistency
# ---------------------------------------------------------------------------

# Extract all close-keyword references: Closes, Fixes, Resolves (case-insensitive).
# Matches: "Closes #123", "fixes #456", "RESOLVES #789"
# Also matches "Close #N", "Fix #N", "Resolve #N" (GitHub accepts both forms).
CLOSE_REFS="$(echo "$PR_BODY" \
    | grep -oiE '(close[sd]?|fix(es|ed)?|resolve[sd]?) #[0-9]+' \
    | grep -oE '#[0-9]+' \
    | tr -d '#' \
    | sort -un)" || true

if [[ -n "$CLOSE_REFS" ]]; then
    while IFS= read -r parent_number; do
        [[ -z "$parent_number" ]] && continue

        open_subs="$(get_open_sub_issues "$parent_number")" || {
            add_error "ERROR: Failed to query sub-issues for #${parent_number}. Cannot verify closure-consistency."
            continue
        }

        if [[ -z "$open_subs" ]]; then
            continue
        fi

        # Check each open sub-issue for a Tracked in: acknowledgment.
        unacked=""
        while IFS= read -r sub_number; do
            [[ -z "$sub_number" ]] && continue
            # Match: "Tracked in: #N" or "Tracked in: owner/repo#N"
            if ! echo "$PR_BODY" | grep -qiE "Tracked in:.*#${sub_number}(\\b|$)"; then
                # Also check if the PR itself closes the sub-issue.
                if ! echo "$CLOSE_REFS" | grep -qx "$sub_number"; then
                    unacked+="  - #${sub_number}"$'\n'
                fi
            fi
        done <<< "$open_subs"

        if [[ -n "$unacked" ]]; then
            add_error "$(cat <<EOF
ERROR: PR closes #${parent_number}, but the following sub-issues are still OPEN and not acknowledged via \`Tracked in: <ref>\`:
${unacked}
Fix one of:
  1. Close the open sub-issue(s) in this PR (add \`Closes #<number>\` to the body).
  2. Acknowledge tracking in this PR's body: add a line \`Tracked in: #<new-followup-number>\` for each.
  3. Re-scope: don't close the parent (#${parent_number}) if the sub-issues will be done in follow-ups.
EOF
)"
        fi
    done <<< "$CLOSE_REFS"
fi

# ---------------------------------------------------------------------------
# Check B: Follow-up tracker
# ---------------------------------------------------------------------------

# Trigger phrases (case-insensitive).
TRIGGER_PATTERN='follow-up PR|will be (handled|done|fixed) in a (separate|follow-up|follow up) (PR|issue)|deferred to a separate (PR|issue)|tracked in a follow-up|addressed in a follow-up'

MATCHED_TRIGGER="$(echo "$PR_BODY" | grep -oiE "$TRIGGER_PATTERN" | head -1)" || true

if [[ -n "$MATCHED_TRIGGER" ]]; then
    # Check for a valid Tracked in: line with a GitHub ref pattern.
    # Valid: "Tracked in: #N" or "Tracked in: owner/repo#N"
    if ! echo "$PR_BODY" | grep -qiE 'Tracked in:.*#[0-9]+'; then
        # Check if there's a malformed Tracked in: line (no #N).
        if echo "$PR_BODY" | grep -qiE 'Tracked in:'; then
            add_error "$(cat <<EOF
ERROR: PR body has a \`Tracked in:\` line but the reference does not match \`#N\` or \`<owner>/<repo>#N\` form.

Add a line to the PR body in the form:
  Tracked in: ${REPO}#<issue-or-PR-number>

The trigger phrase that fired:
  > ${MATCHED_TRIGGER}
EOF
)"
        else
            add_error "$(cat <<EOF
ERROR: PR body indicates deferred work ("${MATCHED_TRIGGER}") but no \`Tracked in: <ref>\` line is present.

Add a line to the PR body in the form:
  Tracked in: ${REPO}#<issue-or-PR-number>

The trigger phrase that fired:
  > ${MATCHED_TRIGGER}
EOF
)"
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Result
# ---------------------------------------------------------------------------

if [[ $VIOLATIONS -gt 0 ]]; then
    echo ""
    echo "$ERROR_MESSAGES"
    echo "Found ${VIOLATIONS} PR body validation issue(s)."
    exit 1
fi

echo "PR body validation passed."
exit 0
