#!/bin/bash
# Test suite for pr-push-guard.sh (mika#2520).
#
# The founding incident: a `resolve-pr-conflicts` pilot rewound `origin/main`
# with a BARE `git push --force-with-lease`. The guard's whole job is that the
# push carries an explicit destination refspec and a pre-session lease, and that
# a resolution aiming at a protected branch is REFUSED rather than pushed.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY V9 IS NOT INTERCHANGEABLE WITH V1–V6
#
# Asserting only the return code would pass on a function that returns 1 *after*
# having pushed. So every refusal path additionally asserts on the fake `git`'s
# CALL LOG: the token `push` does not appear in it. That is AC3's "refused, not
# pushed" read literally — an absence of invocation, not a non-zero exit.
#
# ─────────────────────────────────────────────────────────────────────────────
# WHY V7 ASSERTS ON THE ARGV
#
# mika#2304 measured what an intention field costs: a surface that asserted, with
# authority, the override that had not happened. So V7 reads the argv the fake
# `git` actually received, never a variable the code meant to use. Same shape as
# `pilot_budget_armed` (mika#2496): read the argv, never the resolver.
#
# ─────────────────────────────────────────────────────────────────────────────
# NEGATIVE CONTROLS (N1–N3)
#
# Without them, "the test checks the form" is indistinguishable from "the test
# checks nothing". Each one mutates a copy of the guard, sources it in a
# subshell, and asserts the matching assertion goes RED.
#
# Source isolation: pr-push-guard.sh has no top-level imperative code (function
# definitions and constants only — the contract stated in its header) so direct
# sourcing is safe.
#
# Run: bash skills/bundled/_shared/tests/test_pr_push_guard.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUARD="$SCRIPT_DIR/../pr-push-guard.sh"
HANDLER="$SCRIPT_DIR/../../resolve-pr-conflicts/handlers/run.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

# shellcheck source=skills/bundled/_shared/pr-push-guard.sh
source "$GUARD"

PASS=0
FAIL=0

TMPROOT=$(mktemp -d /tmp/pr-push-guard-test-XXXXXX)
trap 'rm -rf "$TMPROOT"' EXIT

ok() { PASS=$((PASS + 1)); echo "  ✓ $1"; }
ko() {
    FAIL=$((FAIL + 1))
    echo "  ✗ $1"
    shift
    for line in "$@"; do echo "    $line"; done
}

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        ok "$label"
    else
        ko "$label" "expected: '$expected'" "actual:   '$actual'"
    fi
}

assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        ok "$label"
    else
        ko "$label" "needle:   '$needle'" "haystack: '$haystack'"
    fi
}

assert_not_contains() {
    local label="$1" needle="$2" haystack="$3"
    if [[ "$haystack" != *"$needle"* ]]; then
        ok "$label"
    else
        ko "$label" "must NOT contain: '$needle'" "haystack:         '$haystack'"
    fi
}

# Asserts the refusal is well-formed AND that its reason is one of the declared
# wire-format tokens. A reason invented at a call site would split an operator's
# `GROUP BY` without saying so.
assert_refused() {
    local label="$1" reason="$2" rc="$3"

    if [ "$rc" -eq 0 ]; then
        ko "$label — returns non-zero" "actual rc: $rc"
    else
        ok "$label — returns non-zero"
    fi

    assert_contains "$label — refusal names '$reason'" \
        "— ${reason}:" "$PR_PUSH_GUARD_REFUSAL"
    assert_contains "$label — refusal is ticket-prefixed" \
        "REFUSED (mika#2520)" "$PR_PUSH_GUARD_REFUSAL"

    local declared=0 token
    for token in $PR_PUSH_GUARD_REFUSAL_REASONS; do
        [ "$token" = "$reason" ] && declared=1
    done
    if [ "$declared" -eq 1 ]; then
        ok "$label — '$reason' is a declared wire-format token"
    else
        ko "$label — '$reason' is a declared wire-format token" \
           "PR_PUSH_GUARD_REFUSAL_REASONS: $PR_PUSH_GUARD_REFUSAL_REASONS"
    fi
}

# ── Fakes ────────────────────────────────────────────────────────────────────
#
# `git` and `gh` are shell functions, so they shadow the real binaries for every
# call the guard makes. `jq` is left REAL: the guard's JSON parsing is part of
# what R1/R3 must get right, and faking it would test the fake.

MOCK_GH_JSON=""
MOCK_GH_RC=0
MOCK_HEAD=""
MOCK_HEAD_RC=0
MOCK_GIT_DIR=""
MOCK_STATUS=""
MOCK_STATUS_RC=0
MOCK_LS_REMOTE=""
MOCK_PUSH_RC=0
MOCK_ORIGIN_URL="https://github.com/senara-solutions/mika.git"

# The journal is a FILE, not a variable, and that is load-bearing. Most of the
# guard's git calls sit inside a command substitution (`sha=$(git … ls-remote …)`),
# which runs the fake in a subshell — a variable written there is discarded on
# return. A journal that silently loses the very calls V9 reasons about would
# make "no push was invoked" indistinguishable from "nothing was recorded"
# (the mika#2205 class). Hence the positive control in V7.
GIT_JOURNAL="$TMPROOT/git-calls.log"
: > "$GIT_JOURNAL"

gh() {
    printf '%s' "$MOCK_GH_JSON"
    return "$MOCK_GH_RC"
}

git() {
    # Skip a leading `-C <dir>` so the op is read from the real first verb.
    local -a argv=("$@")
    if [ "${argv[0]:-}" = "-C" ]; then
        argv=("${argv[@]:2}")
    fi
    local op="${argv[0]:-}"
    printf '%s\t%s\n' "$op" "$*" >> "$GIT_JOURNAL"

    case "$op" in
        symbolic-ref)
            printf '%s\n' "$MOCK_HEAD"
            return "$MOCK_HEAD_RC"
            ;;
        rev-parse)
            [ -z "$MOCK_GIT_DIR" ] && return 1
            printf '%s\n' "$MOCK_GIT_DIR"
            return 0
            ;;
        status)
            printf '%s' "$MOCK_STATUS"
            return "$MOCK_STATUS_RC"
            ;;
        ls-remote)
            [ -z "$MOCK_LS_REMOTE" ] && return 0
            printf '%s\trefs/heads/x\n' "$MOCK_LS_REMOTE"
            return 0
            ;;
        remote)
            [ -z "$MOCK_ORIGIN_URL" ] && return 1
            printf '%s\n' "$MOCK_ORIGIN_URL"
            return 0
            ;;
        push)
            return "$MOCK_PUSH_RC"
            ;;
        *) return 0 ;;
    esac
}

# Ops recorded this scenario, comma-terminated (`symbolic-ref,status,push,`).
git_ops() { cut -f1 "$GIT_JOURNAL" | tr '\n' ',' ; }

# Full argv of the first call to <op>, or the empty string if it never happened.
# The needle carries the TAB so `push` cannot match a `push`-prefixed op name.
git_argv_of() { grep -m1 -F "$(printf '%s\t' "$1")" "$GIT_JOURNAL" 2>/dev/null | cut -f2- ; }

# A healthy worktree on `<branch>`, with `<sha>` on the remote.
healthy_worktree() {
    local branch="$1" sha="${2:-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}"
    MOCK_HEAD="$branch"
    MOCK_HEAD_RC=0
    MOCK_GIT_DIR="$TMPROOT/clean.git"
    mkdir -p "$MOCK_GIT_DIR"
    MOCK_STATUS=""
    MOCK_STATUS_RC=0
    MOCK_LS_REMOTE="$sha"
    MOCK_PUSH_RC=0
    MOCK_ORIGIN_URL="https://github.com/senara-solutions/mika.git"
}

reset_state() {
    PR_PUSH_GUARD_BRANCH=""
    PR_PUSH_GUARD_BASE=""
    PR_PUSH_GUARD_REPO="senara-solutions/mika"
    PR_PUSH_GUARD_EXPECTED_SHA=""
    PR_PUSH_GUARD_REFUSAL=""
    MOCK_GH_JSON=""
    MOCK_GH_RC=0
    MOCK_ORIGIN_URL="https://github.com/senara-solutions/mika.git"
    : > "$GIT_JOURNAL"
}

# ── V1 — protected branches are refused (AC2, R2) ────────────────────────────
echo "V1: protected branch"
for protected in main master; do
    reset_state
    PR_PUSH_GUARD_BRANCH="$protected"
    PR_PUSH_GUARD_BASE="develop"
    rc=0
    pr_push_guard_check_target 2>/dev/null || rc=$?
    assert_refused "V1[$protected]" "protected_branch" "$rc"
done

# ── V2 — the target is the PR's own base (AC2, R3) ───────────────────────────
echo "V2: branch == base"
reset_state
PR_PUSH_GUARD_BRANCH="feat/x"
PR_PUSH_GUARD_BASE="feat/x"
rc=0
pr_push_guard_check_target 2>/dev/null || rc=$?
assert_refused "V2[stacked base feat/x]" "branch_is_base" "$rc"

# A non-protected base is exactly the case a static list cannot see. Positive
# control right beside it: a genuine feature branch onto `main` passes.
reset_state
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
rc=0
pr_push_guard_check_target 2>/dev/null || rc=$?
assert_eq "V2[nominal fix/2135 onto main] — accepted" "0" "$rc"

# ── V3 — an unreadable base refuses (fail-closed half of R3) ─────────────────
echo "V3: baseRefName unreadable"
reset_state
MOCK_GH_JSON='{"headRefName":"fix/2135"}'
rc=0
pr_push_guard_resolve_target "senara-solutions/mika" "2435" 2>/dev/null || rc=$?
assert_refused "V3[base missing from gh output]" "unresolved_base" "$rc"

# ── V4 — the branch cannot be resolved (R1) ──────────────────────────────────
echo "V4: headRefName unresolvable"
reset_state
MOCK_GH_JSON='{"headRefName":"","baseRefName":"main"}'
rc=0
pr_push_guard_resolve_target "senara-solutions/mika" "2435" 2>/dev/null || rc=$?
assert_refused "V4[empty headRefName]" "unresolved_branch" "$rc"

reset_state
MOCK_GH_JSON=""
MOCK_GH_RC=1
rc=0
pr_push_guard_resolve_target "senara-solutions/mika" "2435" 2>/dev/null || rc=$?
assert_refused "V4[gh fails]" "unresolved_branch" "$rc"

reset_state
rc=0
pr_push_guard_resolve_target "" "" 2>/dev/null || rc=$?
assert_refused "V4[no PR coordinates]" "no_pr_url" "$rc"

# Positive control: a well-formed answer resolves both halves in ONE call.
reset_state
MOCK_GH_JSON='{"headRefName":"fix/2135","baseRefName":"main"}'
rc=0
pr_push_guard_resolve_target "senara-solutions/mika" "2435" || rc=$?
assert_eq "V4[nominal] — resolves" "0" "$rc"
assert_eq "V4[nominal] — branch from PR head" "fix/2135" "$PR_PUSH_GUARD_BRANCH"
assert_eq "V4[nominal] — base from PR" "main" "$PR_PUSH_GUARD_BASE"

# ── V5 — HEAD is not on the resolved branch (R4) ─────────────────────────────
echo "V5: worktree HEAD"
reset_state
healthy_worktree "main"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V5[HEAD on another branch]" "head_mismatch" "$rc"

reset_state
healthy_worktree "fix/2135"
MOCK_HEAD=""
MOCK_HEAD_RC=1
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V5[detached HEAD]" "head_mismatch" "$rc"

# ── V6 — the worktree is not publishable (R5) ────────────────────────────────
echo "V6: worktree state"
reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
MOCK_GIT_DIR="$TMPROOT/rebasing.git"
mkdir -p "$MOCK_GIT_DIR/rebase-merge"
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V6[rebase in progress]" "not_publishable" "$rc"

reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
MOCK_STATUS=" M src/lib.rs"
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V6[dirty tracked tree]" "not_publishable" "$rc"

reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
MOCK_LS_REMOTE=""
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V6[remote branch gone]" "not_publishable" "$rc"

reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
rc=0
pr_push_guard_capture_lease "/wt" || rc=$?
MOCK_LS_REMOTE=""
rc2=0
pr_push_guard_capture_lease "/wt" 2>/dev/null || rc2=$?
assert_refused "V6[no lease to take]" "not_publishable" "$rc2"

# Untracked files must NOT refuse: the handler itself copies
# `.claude/claude-pilot.json` into the worktree, so an `-uall` predicate would
# refuse every nominal dispatch — a guard that fires on its own setup gets
# disarmed. `--untracked-files=no` is asserted on the argv the fake receives.
reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
rc=0
pr_push_guard_check_worktree "/wt" || rc=$?
assert_eq "V6[clean worktree] — accepted" "0" "$rc"
status_argv="$(git_argv_of status)"
assert_contains "V6[clean worktree] — status ignores untracked files" \
    "--untracked-files=no" "$status_argv"
# The handler copies the relay config into the worktree before spawning the
# pilot, and that path is TRACKED here. Counting it would refuse every nominal
# dispatch — a guard that fires on its own setup gets disarmed.
assert_contains "V6[clean worktree] — status exempts the handler-owned relay config" \
    ":(exclude).claude/claude-pilot.json" "$status_argv"
assert_contains "V6[clean worktree] — the exclusions carry a positive pathspec" \
    "-- ." "$status_argv"

# ── V11 — the worktree's origin must be the PR's repository (R6) ─────────────
#
# Without this term the branch comes from the PR while the repository comes from
# whatever `origin` the worktree carries — and the branch name is chosen by
# whoever opened the PR.
echo "V11: repository binding"
reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
PR_PUSH_GUARD_REPO="senara-solutions/mika"
MOCK_ORIGIN_URL="https://github.com/attacker/mika.git"
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V11[foreign origin]" "repo_mismatch" "$rc"

reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
MOCK_ORIGIN_URL=""
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V11[origin unreadable]" "repo_mismatch" "$rc"

reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
MOCK_ORIGIN_URL="not-a-url"
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V11[origin unparseable]" "repo_mismatch" "$rc"

# Every transport spelling of the SAME repository is accepted — otherwise the
# term would refuse the nominal dispatch on an ssh checkout.
for origin_form in \
    "https://github.com/senara-solutions/mika.git" \
    "https://github.com/senara-solutions/mika" \
    "git@github.com:senara-solutions/mika.git" \
    "ssh://git@github.com/senara-solutions/mika.git" \
    "https://github.com/Senara-Solutions/Mika.git"
do
    reset_state
    healthy_worktree "fix/2135"
    PR_PUSH_GUARD_BRANCH="fix/2135"
    PR_PUSH_GUARD_BASE="main"
    MOCK_ORIGIN_URL="$origin_form"
    rc=0
    pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
    assert_eq "V11[accepts $origin_form]" "0" "$rc"
done

# ── V12 — unreadable signals refuse (fail-closed, the inverse of mika#2420) ──
echo "V12: unreadable signals"
reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
MOCK_STATUS_RC=1
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V12[status unreadable]" "not_publishable" "$rc"

reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
MOCK_GIT_DIR=""
rc=0
pr_push_guard_check_worktree "/wt" 2>/dev/null || rc=$?
assert_refused "V12[git dir unreadable]" "not_publishable" "$rc"

# The base half of R3 evaluated inside check_target, not only in resolve_target.
reset_state
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE=""
rc=0
pr_push_guard_check_target 2>/dev/null || rc=$?
assert_refused "V12[base missing at check time]" "unresolved_base" "$rc"

# ── V13 — a genuine push failure is propagated, not swallowed ────────────────
echo "V13: push failure"
reset_state
healthy_worktree "fix/2135" "1111111111111111111111111111111111111111"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
pr_push_guard_capture_lease "/wt"
MOCK_PUSH_RC=1
rc=0
pr_push_guard_push "/wt" 2>/dev/null || rc=$?
assert_eq "V13 — a rejected push returns non-zero" "1" "$rc"
assert_eq "V13 — and is NOT reported as a refusal" "" "$PR_PUSH_GUARD_REFUSAL"
assert_contains "V13 — the push really was attempted" "push," "$(git_ops)"

# ── V7 — the exact argv (AC1) ────────────────────────────────────────────────
echo "V7: push argv"
reset_state
healthy_worktree "fix/2135" "1111111111111111111111111111111111111111"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
pr_push_guard_capture_lease "/wt"
rc=0
pr_push_guard_push "/wt" || rc=$?
push_argv="$(git_argv_of push)"
assert_eq "V7 — push succeeds on the nominal path" "0" "$rc"
assert_eq "V7 — argv is AC1's form, word for word" \
    "-C /wt push --force-with-lease=fix/2135:1111111111111111111111111111111111111111 origin HEAD:refs/heads/fix/2135" \
    "$push_argv"
# A bare lease is `--force-with-lease` followed by a separator, never by `=`.
assert_not_contains "V7 — the lease is never bare" \
    "--force-with-lease " "$push_argv "
assert_contains "V7 — the destination refspec is explicit" \
    "origin HEAD:refs/heads/fix/2135" "$push_argv"
# POSITIVE CONTROL for V9. Without it, "no push was invoked" on the refusal
# paths would be indistinguishable from a journal that records nothing.
assert_contains "V7 — the journal does record a push when one happens" \
    "push," "$(git_ops)"

# ── V8 — the lease is the PRE-session SHA, not one re-read afterwards ────────
#
# This is the second axis of the founding defect: a pilot `git fetch` refreshes
# `origin/<branch>`, so a lease that resolves at push time protects nothing.
echo "V8: lease captured before the session"
reset_state
healthy_worktree "fix/2135" "aaaa000000000000000000000000000000000000"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
pr_push_guard_capture_lease "/wt"
# The remote moves while the pilot runs — the lease must NOT follow it.
MOCK_LS_REMOTE="bbbb111111111111111111111111111111111111"
pr_push_guard_push "/wt"
push_argv="$(git_argv_of push)"
assert_contains "V8 — lease pinned to the pre-session SHA" \
    "--force-with-lease=fix/2135:aaaa000000000000000000000000000000000000" "$push_argv"
assert_not_contains "V8 — lease did not follow the remote" \
    "bbbb1111" "$push_argv"

# Asking to push with no captured lease is itself a refusal.
reset_state
healthy_worktree "fix/2135"
PR_PUSH_GUARD_BRANCH="fix/2135"
PR_PUSH_GUARD_BASE="main"
PR_PUSH_GUARD_EXPECTED_SHA=""
rc=0
pr_push_guard_push "/wt" 2>/dev/null || rc=$?
assert_refused "V8[no lease captured]" "no_lease" "$rc"
assert_not_contains "V8[no lease captured] — git push was NOT invoked" \
    "push," "$(git_ops)"

# ── V9 — no `git push` is invoked on ANY refusal path (AC3) ──────────────────
#
# The form that counts. A return code alone would pass on a function that
# returns 1 after having pushed.
echo "V9: refusal paths invoke no push"

v9_case() {
    local label="$1" branch="$2" base="$3" head="$4" head_rc="$5" \
          gitdir="$6" status="$7" lsremote="$8"
    reset_state
    PR_PUSH_GUARD_BRANCH="$branch"
    PR_PUSH_GUARD_BASE="$base"
    PR_PUSH_GUARD_EXPECTED_SHA="cafe000000000000000000000000000000000000"
    MOCK_HEAD="$head"
    MOCK_HEAD_RC="$head_rc"
    MOCK_GIT_DIR="$gitdir"
    MOCK_STATUS="$status"
    MOCK_STATUS_RC=0
    MOCK_LS_REMOTE="$lsremote"
    local rc=0
    pr_push_guard_push "/wt" 2>/dev/null || rc=$?
    if [ "$rc" -eq 0 ]; then
        ko "V9[$label] — refuses" "expected non-zero, got 0"
    else
        ok "V9[$label] — refuses"
    fi
    assert_not_contains "V9[$label] — NO git push invocation" "push," "$(git_ops)"
    assert_eq "V9[$label] — no push argv recorded" "" "$(git_argv_of push)"
}

CLEAN_GITDIR="$TMPROOT/v9-clean.git"
mkdir -p "$CLEAN_GITDIR"
REBASING_GITDIR="$TMPROOT/v9-rebasing.git"
mkdir -p "$REBASING_GITDIR/rebase-apply"
SHA_OK="dead000000000000000000000000000000000000"

# R2 — the founding shape: the resolution aims at `main` and HEAD is on `main`,
# so R4 passes and R2 is the term that must catch it.
v9_case "R2 protected target" "main" "develop" "main" 0 "$CLEAN_GITDIR" "" "$SHA_OK"
v9_case "R3 branch is base"   "feat/x" "feat/x" "feat/x" 0 "$CLEAN_GITDIR" "" "$SHA_OK"
v9_case "R1 unresolved"       "" "main" "fix/2135" 0 "$CLEAN_GITDIR" "" "$SHA_OK"
v9_case "R4 head mismatch"    "fix/2135" "main" "main" 0 "$CLEAN_GITDIR" "" "$SHA_OK"
v9_case "R4 detached head"    "fix/2135" "main" "" 1 "$CLEAN_GITDIR" "" "$SHA_OK"
v9_case "R5 rebase running"   "fix/2135" "main" "fix/2135" 0 "$REBASING_GITDIR" "" "$SHA_OK"
v9_case "R5 dirty tree"       "fix/2135" "main" "fix/2135" 0 "$CLEAN_GITDIR" " M a.rs" "$SHA_OK"
v9_case "R5 remote gone"      "fix/2135" "main" "fix/2135" 0 "$CLEAN_GITDIR" "" ""

# ── V10 — the refusal reaches a DURABLE surface, not just stderr ─────────────
#
# stderr written before the pilot launches is structurally lost on a dispatch
# that succeeds (the mika#2050 class). `tasks.result` is the surface an operator
# actually reads, so the handler must route the refusal into RESULT.
echo "V10: refusal surface"
reset_state
PR_PUSH_GUARD_BRANCH="main"
PR_PUSH_GUARD_BASE="develop"
rc=0
pr_push_guard_check_target 2>/dev/null || rc=$?
if [ -n "$PR_PUSH_GUARD_REFUSAL" ]; then
    ok "V10 — refusal is exposed as a variable, not only printed"
else
    ko "V10 — refusal is exposed as a variable, not only printed"
fi
assert_contains "V10 — refusal names its lifting gesture" \
    "before re-dispatching" "$PR_PUSH_GUARD_REFUSAL"

if grep -q 'RESULT="\$PR_PUSH_GUARD_REFUSAL"' "$HANDLER"; then
    ok "V10 — the pre-spawn refusals reach RESULT (tasks.result)"
else
    ko "V10 — the pre-spawn refusals reach RESULT (tasks.result)" \
       "expected an assignment RESULT=\"\$PR_PUSH_GUARD_REFUSAL\" in $HANDLER"
fi

# The POST-session refusals use a different form — they PREPEND to an existing
# RESULT — so the grep above is structurally blind to them. Asserted separately,
# and on the leading position specifically: the operator query published in
# CLAUDE.md anchors on the prefix, and RESULT is truncated at 92000 bytes, so a
# refusal trailing a large pilot log is one the query cannot find.
post_session_refusals=$(grep -c 'RESULT="\${PR_PUSH_GUARD_REFUSAL}' "$HANDLER" || true)
if [ "${post_session_refusals:-0}" -ge 2 ]; then
    ok "V10 — both post-session refusals LEAD the result ($post_session_refusals sites)"
else
    ko "V10 — both post-session refusals LEAD the result" \
       "expected >=2 sites assigning RESULT=\"\${PR_PUSH_GUARD_REFUSAL}...\" in $HANDLER" \
       "found: ${post_session_refusals:-0}"
fi

# REGRESSION GUARD for the defect this harness missed once. Running the guard
# inside `$( … )` puts it in a SUBSHELL, so PR_PUSH_GUARD_REFUSAL never reaches
# the handler and the refusal branch becomes dead code — every post-session
# refusal is then reported as a concurrent-push failure, under a reason it
# never had. The call must be a plain statement.
if grep -qE '=\$\(pr_push_guard_push' "$HANDLER"; then
    ko "V10 — pr_push_guard_push is NOT called in a command substitution" \
       "a subshell discards PR_PUSH_GUARD_REFUSAL; redirect to a file instead"
else
    ok "V10 — pr_push_guard_push is NOT called in a command substitution"
fi

# And the documented operator query anchors on the prefix, so a refusal that
# trails a large pilot log is a refusal the query cannot find (and that the
# 92000-byte truncation can cut off).
if grep -q 'REFUSED (mika#2520)%' "$REPO_ROOT/CLAUDE.md"; then
    ok "V10 — CLAUDE.md publishes the anchored operator query"
else
    ko "V10 — CLAUDE.md publishes the anchored operator query" \
       "the documented SQL must match the shape the handler actually writes"
fi

# ── Negative controls ────────────────────────────────────────────────────────
#
# Each mutates a copy of the guard and asserts the matching assertion goes RED.
echo "N1–N3: negative controls"

# Writes the mutated copy and returns its path. A sed expression that matched
# nothing would produce a variant identical to the guard, and every negative
# control would then "pass" while proving nothing — so that case fails loudly in
# the caller (a `ko` inside a command substitution would be lost with the
# subshell).
VARIANT_PATH=""
mutate_guard() {
    local name="$1" expr="$2"
    VARIANT_PATH=""
    local out="$TMPROOT/$name.sh"
    sed "$expr" "$GUARD" > "$out"
    if cmp -s "$out" "$GUARD"; then
        ko "N[$name] — the mutation actually changed the guard" \
           "sed expression matched nothing: $expr"
        return 1
    fi
    VARIANT_PATH="$out"
    return 0
}

# N1 — the push site builds a BARE `--force-with-lease`: V7's argv assertion
# must go red. This is the founding defect, re-created.
if mutate_guard "n1-bare-lease" \
        's|--force-with-lease="${PR_PUSH_GUARD_BRANCH}:${PR_PUSH_GUARD_EXPECTED_SHA}" \\|--force-with-lease \\|'; then
    variant="$VARIANT_PATH"
    argv=$(
        # shellcheck disable=SC1090
        source "$variant"
        GIT_JOURNAL="$TMPROOT/n1-calls.log"
        : > "$GIT_JOURNAL"
        PR_PUSH_GUARD_BRANCH="fix/2135"
        PR_PUSH_GUARD_BASE="main"
        PR_PUSH_GUARD_EXPECTED_SHA="1111111111111111111111111111111111111111"
        MOCK_HEAD="fix/2135"; MOCK_HEAD_RC=0
        MOCK_GIT_DIR="$CLEAN_GITDIR"; MOCK_STATUS=""; MOCK_STATUS_RC=0
        MOCK_LS_REMOTE="$SHA_OK"; MOCK_PUSH_RC=0
        pr_push_guard_push "/wt" >/dev/null 2>&1
        git_argv_of push
    )
    # Assert the POSITIVE shape, not the absence. "No pinned lease" is also
    # true of a variant that pushed nothing at all — which would make this
    # control pass while proving nothing.
    if [[ "$argv" == *"--force-with-lease "* ]] && [[ "$argv" != *"--force-with-lease=fix/2135:"* ]]; then
        ok "N1 — a bare lease makes V7 go red (variant argv: $argv)"
    else
        ko "N1 — a bare lease makes V7 go red" \
           "expected the variant to push with a BARE --force-with-lease" \
           "variant argv: ${argv:-<none — the variant pushed nothing, so this control proved nothing>}"
    fi
fi

# N2 — the R2 term is removed: V1 must go red.
if mutate_guard "n2-no-protected" \
        's|if \[ "$branch" = "$protected" \]; then|if false; then|'; then
    variant="$VARIANT_PATH"
    # Emit the return code too: "no protected_branch in the message" is also
    # true when a DIFFERENT term refused, or when the variant failed to load.
    outcome=$(
        # shellcheck disable=SC1090
        source "$variant"
        PR_PUSH_GUARD_BRANCH="main"
        PR_PUSH_GUARD_BASE="develop"
        rc=0
        pr_push_guard_check_target >/dev/null 2>&1 || rc=$?
        printf '%s|%s' "$rc" "$PR_PUSH_GUARD_REFUSAL"
    )
    if [ "$outcome" = "0|" ]; then
        ok "N2 — removing R2 makes V1 go red (variant accepted 'main')"
    else
        ko "N2 — removing R2 makes V1 go red" \
           "expected the variant to ACCEPT 'main' (rc=0, no refusal)" \
           "variant rc|refusal: $outcome"
    fi
fi

# N3 — the R4 term is removed: V5 must go red.
if mutate_guard "n3-no-head-check" \
        's|if \[ "$head" != "$branch" \]; then|if false; then|'; then
    variant="$VARIANT_PATH"
    outcome=$(
        # shellcheck disable=SC1090
        source "$variant"
        PR_PUSH_GUARD_BRANCH="fix/2135"
        PR_PUSH_GUARD_BASE="main"
        PR_PUSH_GUARD_REPO="senara-solutions/mika"
        MOCK_HEAD="main"; MOCK_HEAD_RC=0
        MOCK_GIT_DIR="$CLEAN_GITDIR"; MOCK_STATUS=""; MOCK_STATUS_RC=0
        MOCK_LS_REMOTE="$SHA_OK"
        MOCK_ORIGIN_URL="https://github.com/senara-solutions/mika.git"
        rc=0
        pr_push_guard_check_worktree "/wt" >/dev/null 2>&1 || rc=$?
        printf '%s|%s' "$rc" "$PR_PUSH_GUARD_REFUSAL"
    )
    if [ "$outcome" = "0|" ]; then
        ok "N3 — removing R4 makes V5 go red (variant accepted a mismatched HEAD)"
    else
        ko "N3 — removing R4 makes V5 go red" \
           "expected the variant to ACCEPT HEAD on 'main' (rc=0, no refusal)" \
           "variant rc|refusal: $outcome"
    fi
fi

echo ""
echo "─────────────────────────────────────────"
echo "Passed: $PASS"
echo "Failed: $FAIL"
if [ "$FAIL" -eq 0 ]; then
    echo "ALL TESTS PASS ✓"
    exit 0
else
    echo "SOME TESTS FAILED ✗"
    exit 1
fi
