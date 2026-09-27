#!/usr/bin/env bash
# The annotation PATCH targets the COMMENT, never the issue (mika#2552).
#
# Usage:
#   annotate-issue-token-comment.sh <owner/repo> <issue-number> <clean> <lint-output-file>
#
#   <clean> is "true" when the body carries canonical machine tokens (withdraw a
#   previous annotation), anything else when it does not (post or rewrite one).
#
# ─────────────────────────────────────────────────────────────────────────────
# THE DEFECT THIS EXISTS TO CLOSE, AND ITS PIVOT IS LINK 3
#
# Until mika#2552 this logic lived inside the workflow's `run:` block and
# resolved its PATCH target by string surgery on an HTML URL:
#
#   PRIOR=$(gh issue view … --json comments --jq '… | .url')   # HTML url
#   gh api -X PATCH "${PRIOR/https:\/\/github.com/https:\/\/api.github.com\/repos}"
#
# `gh issue view --json comments` yields the HTML url,
# `https://github.com/O/R/issues/N#issuecomment-ID`. After the substitution the
# target reads `https://api.github.com/repos/O/R/issues/N#issuecomment-ID` — and
# **the fragment is not transmitted over HTTP**, so the effective target is
# `PATCH /repos/O/R/issues/N`: the ISSUE-edit route. `-f body=` then replaced the
# issue body with the annotation text.
#
# The target was never malformed. It was VALID AND DESIGNATED SOMETHING ELSE —
# that is link 3, and it is why no amount of care around the substitution would
# have helped. The `>/dev/null 2>&1 || true` on the call is what made the
# destruction emit nothing at all.
#
# Measured on mika#2544: body posted by the engine groom at 2026-09-27T08:52:00Z
# (3934 bytes, `Branch` / `Plan` / `Grooming history` callouts), then edited by
# `github-actions` at 08:52:13Z down to 979 bytes — the annotation alone. At the
# 09:00:55Z `ready` relabel, `check_grooming_markers` returned `MarkersMissing`
# and the engine routed a SECOND groom instead of the implement, while the
# `Outcome: PLAN_GROOMED` proof already existed in the database.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE TARGET IS RESOLVED FROM THE REST API, NEVER REWRITTEN FROM THE HTML
#
# `GET /repos/{o}/{r}/issues/{n}/comments` returns, per comment, BOTH
# `html_url` (what `gh issue view` was handing us) and `url`, which is already
# `https://api.github.com/repos/{o}/{r}/issues/comments/{id}`. That value is
# patched AS IS. No string substitution survives anywhere in the tree: the class
# disappears rather than being corrected.
#
# Two traps come with that route, and both are load-bearing:
#
#   * `--paginate` is REQUIRED. The route pages at 30. Without it, a long ticket
#     — this house produces them — resolves an empty prior and the workflow
#     STACKS a comment on every edit. Not destructive, but a silent breach of the
#     "ONE COMMENT PER BODY STATE" contract the workflow header claims.
#   * `| last` inside the jq filter is WRONG under `--paginate`. `gh api
#     --paginate --jq` applies the filter PAGE BY PAGE and concatenates the
#     outputs, so `[…] | last` emits one value per page — a multi-line `PRIOR`
#     that `[ -n … ]` accepts and `gh api -X PATCH` receives as an absurd URL.
#     The filter therefore emits 0..n lines and the LAST ONE IS TAKEN IN SHELL.
#
# The MARKER is interpolated into the jq filter exactly as the pre-mika#2552 code
# did: the constant contains neither a double quote nor a `$`, and keeping the
# form avoids depending on `$ENV` support in whichever jq `gh` was built against.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE GUARD IS FAIL-CLOSED, AND THAT IS THE INVERSE OF ITS NEIGHBOUR
#
# `permissions: issues: write` is NECESSARY for `gh issue comment`, and GitHub
# offers no "comment but do not edit the body" granularity. There is therefore no
# version of this workflow that can annotate without being able to destroy. What
# bounds it can only ever be A PREDICATE ON THE TARGET — never a privilege
# reduction. That is why the guard is the structural half of this ticket.
#
# Any target that is not an issue-comment API route — non-conforming, empty, or
# unreadable — REFUSES the PATCH, loudly. This is the inverse of the fail-safe in
# the mika#2420 reaper (where an unreadable signal PRESERVES), and the inversion
# is reasoned exactly as `_shared/pr-push-guard.sh` (mika#2520) writes it for its
# own case: HERE THE ACTION *IS* THE REMOTE WRITE.
#
#   A wrongful refusal costs one missing annotation — visible in Actions,
#   recoverable at the next edit, bounded.
#   A wrongful pass overwrites an issue body and, with it, the grooming proof the
#   loop reads — irreversible without human intervention.
#
# The trade-off is LOCAL AND DOES NOT TRANSPORT.
#
# ─────────────────────────────────────────────────────────────────────────────
# THE PATCH IS NO LONGER SILENT
#
# `>/dev/null 2>&1 || true` is gone from both sites. A failing PATCH — or a
# target the guard refuses — makes the job RED. The workflow stays non-blocking
# by construction (GitHub offers no gate on an issue), so what turns red is the
# Actions tab, never a PR. That is the only instrument this surface has, and its
# absence is precisely what let eleven bytes of URL fragment through.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY THIS IS A SCRIPT AND NOT A `run:` BLOCK
#
# AC3 ("a guard refuses any PATCH whose target lacks `/issues/comments/`") and
# the ticket's third remedy ("negative test") are satisfiable by NOTHING while
# the predicate lives in YAML: no harness in this repo can execute a workflow
# step. Same motif, already applied and not invented here: `_shared/cwd-guard.sh`
# (mika#2536) and `_shared/pr-push-guard.sh` (mika#2520) both left their
# prescriber so a test could see them.
#
# The file is executable AND sourceable: sourcing it defines the predicate
# without running anything, which is what lets
# `scripts/test-annotate-issue-token-comment.sh` exercise the guard exhaustively
# with no network and no stub.

# `-e` and `pipefail` are both load-bearing, not habit. Without them a failing
# `gh api … | tail -n1` leaves `prior` empty, the script falls through to
# `gh issue comment`, and the workflow STACKS a duplicate — the silent contract
# breach described above, produced by the very error handling meant to be safe.
set -euo pipefail

ANNOTATION_MARKER='<!-- canonical-token-annotation (mika#2201) -->'
ANNOTATION_TICKET='mika#2552'

# ─────────────────────────────────────────────────────────────────────────────
# The guard. Pure, network-free, anchored at both ends.
#
# ANCHORED rather than a bare `contains "/issues/comments/"`. The literal
# predicate of AC3 is enough for the measured defect — the pre-fix target
# `…/issues/2544#issuecomment-1234` does not contain that substring — and the
# anchored form costs not one line more while additionally refusing
# `/pulls/comments/` (a PR review comment, a different object), a foreign host, a
# `..` traversal, and a query string. It is a robustness choice, not a widening
# of scope.
issue_comment_patch_target_is_valid() {
    [[ "$1" =~ ^https://api\.github\.com/repos/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+/issues/comments/[0-9]+$ ]]
}

# The one refusal composer, so the grep token has a single definition site.
refuse_patch_target() {
    echo "ERROR: issue_comment_patch_target_refused ($ANNOTATION_TICKET)" >&2
    echo "       target=$1" >&2
    echo "       A PATCH is only ever sent to an issue-comment API route," >&2
    echo "       https://api.github.com/repos/<owner>/<repo>/issues/comments/<id>." >&2
    echo "       Anything else can reach the ISSUE-edit route and replace the body," >&2
    echo "       taking the Branch / Plan / Grooming history callouts with it." >&2
    echo "       Resolve the target from \`.url\` on" >&2
    echo "       GET /repos/{owner}/{repo}/issues/{number}/comments — never by" >&2
    echo "       rewriting an html_url." >&2
}

# ─────────────────────────────────────────────────────────────────────────────
# The prior annotation's API url, or empty.
#
# A failure to list is NOT swallowed: without the listing we cannot tell whether
# a prior comment exists, and both possible guesses are wrong (stacking a
# duplicate, or patching nothing). `set -o pipefail` therefore lets the failure
# out, and the job goes red.
resolve_prior_annotation_url() {
    local repo="$1" num="$2"
    gh api --paginate "repos/$repo/issues/$num/comments" \
        --jq ".[] | select(.body | contains(\"$ANNOTATION_MARKER\")) | .url" \
        | tail -n1
}

# ─────────────────────────────────────────────────────────────────────────────
# The two PATCH sites. Each guards the very variable it patches, in its own
# body — the co-location `scripts/check-issue-comment-patch-guard.sh` asserts.

withdraw_prior_annotation() {
    local prior="$1"
    if ! issue_comment_patch_target_is_valid "$prior"; then
        refuse_patch_target "$prior"
        return 1
    fi
    gh api --silent -X PATCH "$prior" \
        -f body="$ANNOTATION_MARKER"$'\n\n_Resolved: the body now carries canonical machine tokens._'
}

rewrite_prior_annotation() {
    local prior="$1" body="$2"
    if ! issue_comment_patch_target_is_valid "$prior"; then
        refuse_patch_target "$prior"
        return 1
    fi
    gh api --silent -X PATCH "$prior" -f body="$body"
}

compose_annotation_body() {
    local lint_out="$1"
    printf '%s\n\n%s\n\n```\n%s\n```\n\n%s\n' \
        "$ANNOTATION_MARKER" \
        "**Non-canonical machine token in this issue body.** This is an annotation, not a gate — GitHub offers no gate on an issue." \
        "$(cat "$lint_out")" \
        "The tolerance of every reader is declared in \`scripts/canonical-tokens.tsv\`. If a reader does read this form, the fix is that file, not this body."
}

annotate_issue_token_comment() {
    if [ "$#" -ne 4 ]; then
        echo "usage: $0 <owner/repo> <issue-number> <clean> <lint-output-file>" >&2
        return 2
    fi

    local repo="$1" num="$2" clean="$3" lint_out="$4" prior

    prior="$(resolve_prior_annotation_url "$repo" "$num")"

    if [ "$clean" = "true" ]; then
        # The body was repaired: withdraw rather than leave a stale accusation
        # standing. A comment that outlives its cause is the junk-drawer failure
        # on a surface with no gate to lose.
        [ -n "$prior" ] || return 0
        withdraw_prior_annotation "$prior"
        return
    fi

    if [ -z "$prior" ]; then
        gh issue comment "$num" --repo "$repo" --body "$(compose_annotation_body "$lint_out")"
        return
    fi

    rewrite_prior_annotation "$prior" "$(compose_annotation_body "$lint_out")"
}

# Sourced (by the harness) it defines the predicate and nothing runs; executed,
# it does the work.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    annotate_issue_token_comment "$@"
fi
