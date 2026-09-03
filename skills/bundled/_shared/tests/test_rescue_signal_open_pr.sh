#!/bin/bash
# Test suite for the rescue-into-open-PR signal (mika#2151).
#
# WHY: dispatch-lib's rescue net (mika#1282 dirty-worktree, mika#1383 trailing
# content) commits pilot work and pushes it. When the branch already carries an
# open PR, that content enters a PR a reviewer may already have stamped — and
# the net said nothing. PR#2147 is the measured case: two mika#1282 rescues in
# one lineage, a formal QA approval comment at 00:46:51Z, a second rescue at
# 04:40:45Z, and a human writing the invalidation notice BY HAND 128 seconds
# later. The mechanism had the proof and spent none of it.
#
# WHAT THIS SUITE IS FOR. The trigger under test is "a PR is open on this
# branch", NOT "a PR is approved". At 04:40:45Z on the real incident no review
# had ever reached the APPROVED state (the first one is 9 minutes LATER, at
# 04:49:33Z) — a design keyed on reviewDecision would have stayed silent on the
# very incident that motivated it. T7 freezes that shape; T7b freezes the
# approved variant. Both must pass, and only T7b may dismiss.
#
# The suite is equally about the SILENCES the signal must keep: T1b proves a
# dispatch with no rescue makes no network call at all, and T1 proves a rescue
# with no open PR mutates nothing. A signal that fires unconditionally would
# pass every positive assertion here and still be wrong.
#
# It calls the REAL functions against a real temp git repo with a `gh` function
# stub, rather than reimplementing them — a suite that reimplements the code
# cannot falsify it.
#
# Run: bash skills/bundled/_shared/tests/test_rescue_signal_open_pr.sh
# Expected: all assertions pass, exit 0. No network / cargo / gh needed.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

# dispatch-lib writes gh/git noise to fd 9 (opened by the trace setup in a real
# dispatch). Open it here so the `2>&9` redirects have a destination.
exec 9>/dev/null

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

assert_contains() {
    local label="$1" needle="$2" hay="$3"
    if grep -qF -- "$needle" <<<"$hay"; then
        PASS=$((PASS + 1)); echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    missing: '$needle'"
        echo "    in:      '$(printf '%s' "$hay" | head -c 500)'"
    fi
}

assert_not_contains() {
    local label="$1" needle="$2" hay="$3"
    if grep -qF -- "$needle" <<<"$hay"; then
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    unexpectedly present: '$needle'"
        echo "    in: '$(printf '%s' "$hay" | head -c 500)'"
    else
        PASS=$((PASS + 1)); echo "  ✓ $label"
    fi
}

TMPROOT="$(mktemp -d "${TMPDIR:-/tmp}/mika-2151-test.XXXXXX")"
trap 'rm -rf "$TMPROOT"' EXIT

# ---------------------------------------------------------------------------
# `gh` stub. Records every invocation (one line per call) so the suite can
# assert on the ABSENCE of calls as precisely as on their presence — the AC2
# assertions are all negative. Idiom: test_finalize_pr_gate.sh's shell-function
# stub, extended with a call log.
# ---------------------------------------------------------------------------
GH_LOG="$TMPROOT/gh-calls.log"
GH_PR_LIST_JSON=""     # what `gh pr list --json … --jq '.[0] // empty'` yields
GH_REVIEWS_IDS=""      # ids `gh api …/reviews --jq …` yields (one per line)
GH_COMMENT_RC=0
GH_LABEL_RC=0
GH_DISMISS_RC=0

gh() {
    printf '%s\n' "$*" >> "$GH_LOG"
    case "${1:-}" in
        pr)
            case "${2:-}" in
                list)    printf '%s' "$GH_PR_LIST_JSON"; return 0 ;;
                comment) return "$GH_COMMENT_RC" ;;
                edit)    return "$GH_LABEL_RC" ;;
            esac
            return 0
            ;;
        api)
            case "$*" in
                *dismissals*) return "$GH_DISMISS_RC" ;;
                *reviews*)    printf '%s\n' "$GH_REVIEWS_IDS"; return 0 ;;
            esac
            return 0
            ;;
    esac
    return 0
}

gh_calls() { cat "$GH_LOG" 2>/dev/null || true; }
# `grep -c` prints 0 AND exits 1 on no match, so a `|| echo 0` fallback would
# print it twice. Capture, then default.
gh_call_count() {
    local n
    n=$(grep -c . "$GH_LOG" 2>/dev/null || true)
    printf '%s' "${n:-0}"
}

# Build a real temp repo carrying one rescue commit on top of an origin/main
# ref, so `git log`, `git diff --shortstat <sha>^ <sha>` and
# `git diff --shortstat origin/main...HEAD` all have something true to say.
# Echoes "<repo_dir> <rescue_sha>".
make_repo_with_rescue() {
    local branch="${1:-fix/2151/probe}" repo
    repo="$(mktemp -d "$TMPROOT/repo.XXXXXX")"
    git -C "$repo" init -q -b main
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name "Test"
    printf 'base\n' > "$repo/base.txt"
    git -C "$repo" add -A
    git -C "$repo" commit -q -m "feat: base"
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
    git -C "$repo" checkout -q -b "$branch"
    printf 'rescued line one\nrescued line two\n' > "$repo/rescued.txt"
    git -C "$repo" add -A
    git -C "$repo" commit -q -m "wip(mika#2151): impl staged by post-flight recovery (mika#1282)"
    printf '%s %s' "$repo" "$(git -C "$repo" rev-parse HEAD)"
}

# Reset every input the function reads, plus the stub's log and responses.
reset_case() {
    : > "$GH_LOG"
    GH_PR_LIST_JSON=""
    GH_REVIEWS_IDS=""
    GH_COMMENT_RC=0
    GH_LABEL_RC=0
    GH_DISMISS_RC=0
    REPO="mika"
    ISSUE_NUM="2151"
    SESSION_ID="sess-2151-abcdef"
    LOG_ID="log-2151"
    RESULT=""
    RESCUE_COMMITS=""
    RESCUE_COMMITS_SIGNALLED=""
}

# ===========================================================================
# T1 (AC2) — a rescue happened, no PR is open: one read, zero mutation.
# ===========================================================================
echo ""
echo "T1 (AC2): rescue + no open PR — one gh read, zero mutating call"
echo "---------------------------------------------------------------"

reset_case
read -r WORKTREE_DIR RESCUE_SHA <<< "$(make_repo_with_rescue)"
BRANCH="fix/2151/probe"
RESCUE_COMMITS="$RESCUE_SHA"
GH_PR_LIST_JSON=""   # `--jq '.[0] // empty'` on an empty array yields nothing

STDERR_T1="$TMPROOT/t1.stderr"
rc=0
_signal_rescue_into_open_pr 2>"$STDERR_T1" || rc=$?
assert_eq "T1 returns 0" "0" "$rc"
assert_contains "T1 queried gh pr list" "pr list" "$(gh_calls)"
assert_not_contains "T1 posted no comment" "pr comment" "$(gh_calls)"
assert_not_contains "T1 applied no label" "pr edit" "$(gh_calls)"
assert_not_contains "T1 called no api endpoint" "api" "$(gh_calls)"
assert_eq "T1 RESULT untouched" "" "$RESULT"
assert_contains "T1 named the silence on stderr" "rescue_signal.no_open_pr" "$(cat "$STDERR_T1")"
assert_eq "T1 marked the sha signalled" "$RESCUE_SHA" "$RESCUE_COMMITS_SIGNALLED"

# ===========================================================================
# T1b (AC2 / F4) — no rescue at all: not a single gh call, not even the read.
# This is the majority case of a healthy loop; its cost must be exactly zero.
# ===========================================================================
echo ""
echo "T1b (AC2/F4): no rescue commits — zero gh invocations of any kind"
echo "-----------------------------------------------------------------"

reset_case
read -r WORKTREE_DIR _ <<< "$(make_repo_with_rescue)"
BRANCH="fix/2151/probe"
RESCUE_COMMITS=""    # the pilot committed its own work — nothing was rescued

rc=0
_signal_rescue_into_open_pr 2>/dev/null || rc=$?
assert_eq "T1b returns 0" "0" "$rc"
assert_eq "T1b made zero gh calls" "0" "$(gh_call_count)"
assert_eq "T1b RESULT untouched" "" "$RESULT"

# The same must hold when the entry guard's other legs are unset.
reset_case
RESCUE_COMMITS="deadbeef"
REPO=""; BRANCH=""; WORKTREE_DIR=""
rc=0
_signal_rescue_into_open_pr 2>/dev/null || rc=$?
assert_eq "T1b (unset REPO/BRANCH/WORKTREE) returns 0" "0" "$rc"
assert_eq "T1b (unset REPO/BRANCH/WORKTREE) made zero gh calls" "0" "$(gh_call_count)"

# ===========================================================================
# T2 (AC1) — an open PR: a comment naming the sha and its shortstat.
# ===========================================================================
echo ""
echo "T2 (AC1): open PR — comment posted naming the rescue sha + shortstat"
echo "--------------------------------------------------------------------"

reset_case
read -r WORKTREE_DIR RESCUE_SHA <<< "$(make_repo_with_rescue)"
BRANCH="fix/2151/probe"
RESCUE_COMMITS="$RESCUE_SHA"
GH_PR_LIST_JSON='{"number":2147,"reviewDecision":null}'

# Capture the body dispatch-lib writes by intercepting --body-file.
BODY_SEEN="$TMPROOT/t2-body.md"
gh() {
    printf '%s\n' "$*" >> "$GH_LOG"
    case "${1:-}" in
        pr)
            case "${2:-}" in
                list)    printf '%s' "$GH_PR_LIST_JSON"; return 0 ;;
                comment)
                    local prev=""
                    for a in "$@"; do
                        [ "$prev" = "--body-file" ] && cp "$a" "$BODY_SEEN"
                        prev="$a"
                    done
                    return "$GH_COMMENT_RC" ;;
                edit)    return "$GH_LABEL_RC" ;;
            esac
            return 0 ;;
        api)
            case "$*" in
                *dismissals*) return "$GH_DISMISS_RC" ;;
                *reviews*)    printf '%s\n' "$GH_REVIEWS_IDS"; return 0 ;;
            esac
            return 0 ;;
    esac
    return 0
}

rc=0
_signal_rescue_into_open_pr 2>/dev/null || rc=$?
assert_eq "T2 returns 0" "0" "$rc"
assert_contains "T2 posted a comment on PR 2147" "pr comment 2147" "$(gh_calls)"
BODY="$(cat "$BODY_SEEN" 2>/dev/null || true)"
assert_contains "T2 body names the rescue sha" "$(git -C "$WORKTREE_DIR" rev-parse --short "$RESCUE_SHA")" "$BODY"
assert_contains "T2 body carries the per-commit shortstat" "2 insertions(+)" "$BODY"
assert_contains "T2 body carries the cumulative branch delta" "origin/main...HEAD" "$BODY"
assert_contains "T2 body names the pilot session" "sess-2151-abcdef" "$BODY"
assert_contains "T2 body names the log id" "log-2151" "$BODY"
assert_contains "T2 body names the ticket" "mika#2151" "$BODY"
assert_contains "T2 body states the approval does not cover it" "ne les couvre pas" "$BODY"
assert_contains "T2 applied the rescue-after-review label" "rescue-after-review" "$(gh_calls)"
assert_contains "T2 RESULT carries the fact for the callback" "Rescue-signal (mika#2151)" "$RESULT"
assert_contains "T2 RESULT names the PR" "#2147" "$RESULT"

# ===========================================================================
# T3 (AC1) — reviewDecision APPROVED: one PUT …/dismissals per APPROVED review.
# ===========================================================================
echo ""
echo "T3 (AC1): APPROVED PR — one dismissal per APPROVED review"
echo "---------------------------------------------------------"

reset_case
read -r WORKTREE_DIR RESCUE_SHA <<< "$(make_repo_with_rescue)"
BRANCH="fix/2151/probe"
RESCUE_COMMITS="$RESCUE_SHA"
GH_PR_LIST_JSON='{"number":2147,"reviewDecision":"APPROVED"}'
GH_REVIEWS_IDS=$'111\n222'

rc=0
_signal_rescue_into_open_pr 2>/dev/null || rc=$?
assert_eq "T3 returns 0" "0" "$rc"
DISMISSALS=$(grep -c 'dismissals' "$GH_LOG" || true)
assert_eq "T3 dismissed both APPROVED reviews" "2" "$DISMISSALS"
assert_contains "T3 dismissed review 111" "/pulls/2147/reviews/111/dismissals" "$(gh_calls)"
assert_contains "T3 dismissed review 222" "/pulls/2147/reviews/222/dismissals" "$(gh_calls)"
assert_contains "T3 used PUT, not gh pr review" "--method PUT" "$(gh_calls)"
assert_not_contains "T3 never used gh pr review --request-changes" "--request-changes" "$(gh_calls)"
assert_contains "T3 comment still posted before the dismissal" "pr comment 2147" "$(gh_calls)"
assert_contains "T3 RESULT reports the dismissals" "2 stale approval" "$RESULT"

# ===========================================================================
# T4 (AC1) — open but NOT approved: comment, and NO dismissal. This is the
# shape of the real incident; the comment is what carries AC1 on it.
# ===========================================================================
echo ""
echo "T4 (AC1): open, not approved — comment posted, nothing dismissed"
echo "----------------------------------------------------------------"

reset_case
read -r WORKTREE_DIR RESCUE_SHA <<< "$(make_repo_with_rescue)"
BRANCH="fix/2151/probe"
RESCUE_COMMITS="$RESCUE_SHA"
GH_PR_LIST_JSON='{"number":2147,"reviewDecision":"REVIEW_REQUIRED"}'
GH_REVIEWS_IDS=$'111'   # present, and must still not be touched

rc=0
_signal_rescue_into_open_pr 2>/dev/null || rc=$?
assert_eq "T4 returns 0" "0" "$rc"
assert_contains "T4 posted the comment" "pr comment 2147" "$(gh_calls)"
assert_contains "T4 applied the label" "rescue-after-review" "$(gh_calls)"
assert_not_contains "T4 dismissed nothing" "dismissals" "$(gh_calls)"
assert_not_contains "T4 did not even list reviews" "/reviews" "$(gh_calls)"

# ===========================================================================
# T5 (AC4) — accumulator, not a boolean. Two shas in one comment; a second
# call with nothing new stays silent; a fresh dispatch signals again.
# ===========================================================================
echo ""
echo "T5 (AC4): two rescues → one comment naming both; no repeat; fresh run re-signals"
echo "-------------------------------------------------------------------------------"

reset_case
read -r WORKTREE_DIR RESCUE_SHA <<< "$(make_repo_with_rescue)"
BRANCH="fix/2151/probe"
# Second rescue of the same dispatch — exactly PR#2147's e3fe1724 → 628099ef.
printf 'second rescue\n' > "$WORKTREE_DIR/rescued2.txt"
git -C "$WORKTREE_DIR" add -A
git -C "$WORKTREE_DIR" commit -q -m "wip(mika#2151): impl staged by post-flight recovery (mika#1282)"
RESCUE_SHA2="$(git -C "$WORKTREE_DIR" rev-parse HEAD)"
RESCUE_COMMITS="${RESCUE_SHA}
${RESCUE_SHA2}"
GH_PR_LIST_JSON='{"number":2147,"reviewDecision":null}'
: > "$BODY_SEEN"

_signal_rescue_into_open_pr 2>/dev/null || true
BODY="$(cat "$BODY_SEEN" 2>/dev/null || true)"
assert_contains "T5 one comment names the first sha" \
    "$(git -C "$WORKTREE_DIR" rev-parse --short "$RESCUE_SHA")" "$BODY"
assert_contains "T5 one comment names the second sha" \
    "$(git -C "$WORKTREE_DIR" rev-parse --short "$RESCUE_SHA2")" "$BODY"
assert_eq "T5 exactly one comment was posted" "1" "$(grep -c 'pr comment' "$GH_LOG" || true)"

# Second call, nothing new — must be completely silent, including the read.
: > "$GH_LOG"
_signal_rescue_into_open_pr 2>/dev/null || true
assert_eq "T5 second call made zero gh calls" "0" "$(gh_call_count)"

# A later dispatch starts with a fresh SIGNALLED set and must speak again.
: > "$GH_LOG"
RESCUE_COMMITS_SIGNALLED=""
_signal_rescue_into_open_pr 2>/dev/null || true
assert_eq "T5 fresh dispatch signals again" "1" "$(grep -c 'pr comment' "$GH_LOG" || true)"

# ===========================================================================
# T6 (AC3) — the comment fails: the function still returns 0, names the
# failure, and the rescue commit is still reachable in the repo. Nothing in
# the signalling path may cost the work.
# ===========================================================================
echo ""
echo "T6 (AC3): gh pr comment fails — fail-open, work intact"
echo "------------------------------------------------------"

reset_case
read -r WORKTREE_DIR RESCUE_SHA <<< "$(make_repo_with_rescue)"
BRANCH="fix/2151/probe"
RESCUE_COMMITS="$RESCUE_SHA"
GH_PR_LIST_JSON='{"number":2147,"reviewDecision":null}'
GH_COMMENT_RC=1
GH_LABEL_RC=1

STDERR_T6="$TMPROOT/t6.stderr"
rc=0
_signal_rescue_into_open_pr 2>"$STDERR_T6" || rc=$?
assert_eq "T6 returns 0 despite the failed comment" "0" "$rc"
T6_ERR="$(cat "$STDERR_T6")"
assert_contains "T6 stderr names the comment failure" "rescue_signal.comment_failed" "$T6_ERR"
assert_contains "T6 stderr names the label failure" "rescue_signal.label_failed" "$T6_ERR"
OBJ_RC=0
git -C "$WORKTREE_DIR" cat-file -e "${RESCUE_SHA}^{commit}" 2>/dev/null || OBJ_RC=$?
assert_eq "T6 rescue commit still reachable in the repo" "0" "$OBJ_RC"
assert_eq "T6 rescue commit is still the branch tip" \
    "$RESCUE_SHA" "$(git -C "$WORKTREE_DIR" rev-parse HEAD)"

# ===========================================================================
# T7 (AC5) — PR#2147 frozen AS MEASURED: reviewDecision absent at the moment
# of the 04:40:45Z rescue (the stamp in force was a QA prose comment from
# 00:46:51Z; the first APPROVED review is 04:49:33Z, nine minutes LATER).
# A design keyed on APPROVED would print nothing here. It must comment.
# ===========================================================================
echo ""
echo "T7 (AC5): PR#2147 as it actually happened — comment, no dismissal"
echo "-----------------------------------------------------------------"

reset_case
BRANCH="fix/2121/task-engine-306-dispatches-marqu-s"
read -r WORKTREE_DIR RESCUE_SHA <<< "$(make_repo_with_rescue "$BRANCH")"
RESCUE_COMMITS="$RESCUE_SHA"
GH_PR_LIST_JSON='{"number":2147,"reviewDecision":null}'
GH_REVIEWS_IDS=""
: > "$BODY_SEEN"

rc=0
_signal_rescue_into_open_pr 2>/dev/null || rc=$?
assert_eq "T7 returns 0" "0" "$rc"
assert_contains "T7 commented on PR#2147 with no APPROVED state present" \
    "pr comment 2147" "$(gh_calls)"
assert_not_contains "T7 dismissed nothing (there was nothing to dismiss)" \
    "dismissals" "$(gh_calls)"
assert_contains "T7 comment names the real branch" \
    "fix/2121/task-engine-306-dispatches-marqu-s" "$(cat "$BODY_SEEN")"

# ===========================================================================
# T7b (AC5, AC1) — same scenario once the stamp travels through the GitHub
# review state instead of prose: comment AND dismissal.
# ===========================================================================
echo ""
echo "T7b (AC5/AC1): same branch, reviewDecision APPROVED — comment + dismissal"
echo "-------------------------------------------------------------------------"

reset_case
BRANCH="fix/2121/task-engine-306-dispatches-marqu-s"
read -r WORKTREE_DIR RESCUE_SHA <<< "$(make_repo_with_rescue "$BRANCH")"
RESCUE_COMMITS="$RESCUE_SHA"
GH_PR_LIST_JSON='{"number":2147,"reviewDecision":"APPROVED"}'
GH_REVIEWS_IDS=$'909'

rc=0
_signal_rescue_into_open_pr 2>/dev/null || rc=$?
assert_eq "T7b returns 0" "0" "$rc"
assert_contains "T7b commented" "pr comment 2147" "$(gh_calls)"
assert_contains "T7b dismissed the approval" "/pulls/2147/reviews/909/dismissals" "$(gh_calls)"

# ===========================================================================
# T8 (AC1, AC4) — static guard. A future rescue site added without wiring, or
# a push site that stops signalling, must break this test rather than
# reintroduce the silence.
# ===========================================================================
echo ""
echo "T8 (AC1/AC4): static guard — every rescue site feeds, every push site signals"
echo "----------------------------------------------------------------------------"

# The rescue-commit site count is pinned by test_rescue_commit_no_verify.sh at
# 3. Re-assert it here so the two counts below are known to be about the SAME
# three sites: a 4th rescue commit added without a recorder trips this pair.
RESCUE_SITES=$(grep -c 'git -C "\$WORKTREE_DIR" commit -m "wip(' "$DISPATCH_LIB" || true)
assert_eq "T8 rescue-commit sites in dispatch-lib" "3" "$RESCUE_SITES"

RECORDERS=$(grep -cE '^[[:space:]]*_record_rescue_commit[[:space:]]*$' "$DISPATCH_LIB" || true)
assert_eq "T8 one _record_rescue_commit call per rescue site" "$RESCUE_SITES" "$RECORDERS"

SIGNAL_SITES=$(grep -cE '^[[:space:]]*_signal_rescue_into_open_pr[[:space:]]*$' "$DISPATCH_LIB" || true)
assert_eq "T8 _signal_rescue_into_open_pr called at both push sites" "2" "$SIGNAL_SITES"

# The trigger is "a PR is open", never "a PR is approved" — the correction the
# hour-by-hour measurement of PR#2147 forced onto the plan. Assert the gh query
# asks for open PRs and does not filter on reviewDecision.
assert_contains "T8 the query asks for open PRs on the branch" \
    '--state open' "$(sed -n '/^_signal_rescue_into_open_pr()/,/^}/p' "$DISPATCH_LIB")"

# Step 4 — the third silence. The `gh pr create` rescue site lives inline in
# dispatch_claude_pilot and is not callable in isolation (same constraint as the
# rescue blocks in test_rescue_commit_no_verify.sh), so it is pinned statically.
# What must hold: the empty-RESCUED_PR_URL arm asks whether a PR already exists
# BEFORE settling on the generic create-failed motif, and reports the existing
# PR when there is one. A regression to an unconditional
# `NO_PR: rescue_pr_create_failed` puts back the misreport mika#2151 removes.
CREATE_FAIL_ARM=$(sed -n '/mika#2151 (the third silence)/,/NO_PR: rescue_pr_create_failed/p' "$DISPATCH_LIB")
assert_contains "T8 create-failed arm asks whether a PR already exists" \
    '_pr_list_url "$REPO" "$BRANCH"' "$CREATE_FAIL_ARM"
assert_contains "T8 create-failed arm reports the existing PR truthfully" \
    '_set_pr_status_line "PR: ${_existing_pr}"' "$CREATE_FAIL_ARM"
assert_contains "T8 create-failed arm keeps the generic motif for the real failure" \
    'NO_PR: rescue_pr_create_failed' "$CREATE_FAIL_ARM"

rc=0
bash -n "$DISPATCH_LIB" 2>/dev/null || rc=$?
assert_eq "T8 dispatch-lib passes bash -n" "0" "$rc"

# ===========================================================================
echo ""
echo "=============================="
echo "Results: $PASS passed, $FAIL failed"
echo "=============================="

[ "$FAIL" -eq 0 ] || exit 1
