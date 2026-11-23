#!/usr/bin/env bash
set -euo pipefail

# qa_head_reviews — the reviews that RULED ON this head, and nothing else.
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
#
# Why `commit_id` alone is not the answer either (mika#2939): `commit_id` names
# the head the forge attributes a review to **today**, which a force-push can
# rewrite (measured on #2920: a review submitted at 07:16:49Z carries a
# `commit_id` created at 12:14Z — five hours after the review). The head a
# review *ruled on* is therefore `commit_id` **and** a submission not predating
# the head.
#
# The reference point is `head_first_seen_at = MIN(check_suites[].created_at)`
# on the SHA — a server-side instant (a committer date is set locally, so it is
# no reference at all), defined by construction (the turn is triggered BY a
# check_suite on this head), and semantically exact: the first suite is the
# first instant the forge attests the SHA exists. A review submitted before the
# forge had seen the SHA cannot have ruled on it, whatever path made it the
# head afterwards. Beyond 100 suites the MIN is taken over a subset, hence it is
# >= the true MIN, hence the filter is STRICTER — fail-safe, which is why this
# handler deliberately does not paginate that call.
#
# Input: JSON on stdin with "repo" (owner/name), "pr" (integer), "head_sha" (40 hex)
# Output: one JSON object on stdout:
#   {head_evidence, head_sha, head_first_seen_at,
#    reviews: [{author, state, submitted_at, carries_verdict_line}],
#    dropped_predating_head: <int>}
#
# An absent or malformed field is an ERROR, never an empty `reviews`: an empty
# `reviews` is the answer "no review ever ruled on this head", and conflating
# the two is the mika#2205 class. The review body is deliberately NOT rendered —
# step 4 needs the fact, not the text.
#
# `head_evidence` is a CONTRACT READ BY THE ENGINE (mika#3124), not a
# convenience. Closed vocabulary, exactly two values:
#
#     head_evidence ∈ { "head_was_ruled_on", "head_never_ruled_on" }
#
# `qa_reviewed_head::preuve_du_tour_manquante` searches this turn's tool history
# for the **quoted** literal and refuses a `record_reviewed_head` write with a
# `stop_*` outcome unless THIS head's own call came back `head_was_ruled_on`.
# Until mika#3124 the engine only inspected the call's INPUT, so a turn that had
# asked the question and been told "zero reviews" could still write
# `stop_already_reviewed_sha`: measured on #3070, head `24e1f25b2`, 2026-11-10
# at 17:02:06Z — which then discarded that head's green check_suite for 30 days.
#
# Three properties of the key, all three load-bearing, all three held by
# `scripts/test-qa-head-reviews-handler.sh`:
#   1. derived from the SAME `$kept` as `reviews` — never from `$on_head`, whose
#      reviews may predate the head and have ruled on nothing;
#   2. emitted FIRST, because the engine truncates the output it keeps to
#      `planning::policy::OUTPUT_SUMMARY_MAX` bytes and the tail is lost;
#   3. the two values are lexically disjoint — neither is a substring of the
#      other, so searching one can never match the other.
# Renaming a value, moving the key, or deriving it from anything else breaks the
# engine's read in silence. Rust-side source of truth for the two spellings:
# `HEAD_EVIDENCE_RULED_ON` / `HEAD_EVIDENCE_NEVER_RULED_ON`.
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

# The reference point, read BEFORE the reviews so a failure here writes nothing
# at all on stdout — a partial object would be read as a review list.
#
# `VAR=$(…) || { … }` and not `if ! VAR=$(…); then`: the mika#2536 scan
# (`skills::executor` T1) reads an assignment at the HEAD of a command segment,
# and `if ! ` in front of it hides the write — the variable then reads as an
# inert environment lookup and the guard reddens. Same form as the three `jq`
# reads above. `set -e` is satisfied either way: `||` makes this a compound list.
FIRST_SEEN=$(gh api "repos/${REPO}/commits/${HEAD_SHA}/check-suites?per_page=100" \
  --jq '[(.check_suites // [])[].created_at] | min // empty') || {
  echo "ERROR: could not read the check suites of ${HEAD_SHA} in ${REPO}" >&2
  exit 1
}
if [[ ! "$FIRST_SEEN" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$ ]]; then
  echo "ERROR: no readable check_suite timestamp for ${HEAD_SHA} (got: '${FIRST_SEEN}') — head_first_seen_at is undeterminable, which is NOT an empty review list" >&2
  exit 1
fi

# `--jq '.[]'` unrolls each page into a stream of reviews; the filters run in the
# second jq because `--paginate` concatenates one array per page and `gh api` has
# no way to pass a `--arg` to its own `--jq`. The SHA and the reference instant
# therefore reach jq as data, never as interpolated program text.
#
# The comparison is lexicographic, and that is correct: both sides are
# fixed-width ISO 8601 UTC suffixed `Z`. The predicate is `>=`, inclusive — a
# review submitted in the same second as the first suite is kept, because the
# reference point bounds what the review could see, it does not arbitrate
# sub-seconds.
#
# The filter PARTITIONS rather than selects: both halves are counted, so
# `dropped_predating_head` measures how often the forge reattributes a review.
gh api "repos/${REPO}/pulls/${PR}/reviews" --paginate --jq '.[]' \
  | jq -s --arg head_sha "$HEAD_SHA" --arg first_seen "$FIRST_SEEN" '
      [ .[] | select(.commit_id == $head_sha) ] as $on_head
      | ($on_head | map(select((.submitted_at // "") >= $first_seen))) as $kept
      | {
          head_evidence: (if ($kept | length) > 0
                          then "head_was_ruled_on"
                          else "head_never_ruled_on"
                          end),
          head_sha: $head_sha,
          head_first_seen_at: $first_seen,
          reviews: ($kept | map({
            author: (.user.login // ""),
            state: (.state // ""),
            submitted_at: (.submitted_at // ""),
            carries_verdict_line: ((.body // "") | test("VERDICT:[ \t*]*(pass|block)"))
          })),
          dropped_predating_head: (($on_head | length) - ($kept | length))
        }'
