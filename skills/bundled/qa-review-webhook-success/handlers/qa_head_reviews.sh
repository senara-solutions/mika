#!/usr/bin/env bash
set -euo pipefail

# qa_head_reviews — the reviews that carry THIS head's commit_id, and nothing else.
#
# `gh pr view --json reviews` does not render `commit_id` — the field is absent
# from that schema. The only endpoint that carries it is
# `GET /repos/{owner}/{repo}/pulls/{n}/reviews`, which is outside mika-qa's
# `run_gh` scope (`validate_qa_review_gh_scope` allows `gh api` for
# `GET /advisories` only). Widening that gate would make every
# `pulls/*/reviews` path reachable from any mika-qa turn to close a need of one
# turn; a handler gives the datum instead — remove capability, don't prohibit it.
#
# Why this tool exists at all (mika#2934): judging a head on a PR-level state
# (`reviewDecision`, `mergeStateStatus`) is judging it on something that
# survives a force-push. On 2026-10-27 that read declared three freshly-pushed
# heads already ruled on, 9 to 17 minutes before the first verdict on each.
# A review's `commit_id` is the only field that names the head it ruled on.
#
# Input: JSON on stdin with "repo" (owner/name), "pr" (integer), "head_sha" (40 hex)
# Output: JSON array on stdout of {author, state, submitted_at, carries_verdict_line}
#
# An absent or malformed field is an ERROR, never an empty array: an empty array
# is the answer "no review on this head", and conflating the two is the
# mika#2205 class. The review body is deliberately NOT rendered — step 4 needs
# the fact, not the text.
#
# See: docs/solutions/prompt-engineering/2026-04-08-remove-capability-not-prohibit-it.md

command -v gh &>/dev/null || { echo "ERROR: gh CLI not found on PATH" >&2; exit 1; }
command -v jq &>/dev/null || { echo "ERROR: jq not found on PATH" >&2; exit 1; }

INPUT=$(cat)

REPO=$(printf '%s\n' "$INPUT" | jq -r '.repo // empty')
PR=$(printf '%s\n' "$INPUT" | jq -r '.pr // empty')
HEAD_SHA=$(printf '%s\n' "$INPUT" | jq -r '.head_sha // empty')

# Each field is validated BEFORE it reaches the command line — the three
# together build the API path, so a stray `--flag` or `../` would be option
# injection or path traversal.
if [[ ! "$REPO" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]]; then
  echo "ERROR: repo is required and must be owner/name (got: '${REPO}')" >&2
  exit 1
fi
if [[ ! "$PR" =~ ^[0-9]+$ ]]; then
  echo "ERROR: pr is required and must be a PR number (got: '${PR}')" >&2
  exit 1
fi
if [[ ! "$HEAD_SHA" =~ ^[0-9a-f]{40}$ ]]; then
  echo "ERROR: head_sha is required and must be a 40-character lowercase hex SHA (got: '${HEAD_SHA}')" >&2
  exit 1
fi

# `--jq '.[]'` unrolls each page into a stream of reviews; the head filter runs
# in the second jq because `--paginate` concatenates one array per page and
# `gh api` has no way to pass a `--arg` to its own `--jq`. The SHA therefore
# reaches jq as data, never as interpolated program text.
gh api "repos/${REPO}/pulls/${PR}/reviews" --paginate --jq '.[]' \
  | jq -s --arg head_sha "$HEAD_SHA" '
      [ .[]
        | select(.commit_id == $head_sha)
        | {
            author: (.user.login // ""),
            state: (.state // ""),
            submitted_at: (.submitted_at // ""),
            carries_verdict_line: ((.body // "") | test("VERDICT:[ \t*]*(pass|block)"))
          }
      ]'
