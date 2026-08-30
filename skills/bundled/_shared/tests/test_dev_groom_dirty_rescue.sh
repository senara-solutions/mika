#!/bin/bash
# Test suite for the dirty-worktree rescue's dev-groom coverage (mika#2031).
#
# WHY: dispatch-lib's mika#1282 rescue was gated on `$SKILL = dev-pilot`. A
# dev-groom pilot killed after writing its plan but before `git commit` has
# nothing staged, nothing committed, nothing pushed — and `_set_up_worktree`
# force-removes the worktree on the next dispatch of the same branch. Uncommitted
# work is the most fragile form the loss takes: it exists in exactly one place,
# so a rescue that misses it loses it for good.
#
# WHAT THIS SUITE IS FOR. Two halves, and the second is the one that is easy to
# skip: a dirty tree must be PRESERVED AND UNBLOCKED, and a CLEAN tree must
# trigger nothing at all. Testing only the dirty case would let a rescue that
# fires unconditionally pass — indistinguishable, from inside the suite, from one
# that fires when it should.
#
# It calls the REAL `_rescue_dirty_worktree` against a real temp git repo rather
# than reimplementing the rescue in the test — the older rescue suites
# (test_auto_rescue_* in test-dispatch-lib.sh) reimplement it, which means they
# cannot falsify the shipped code.
#
# Run: bash skills/bundled/_shared/tests/test_dev_groom_dirty_rescue.sh
# Expected: all assertions pass, exit 0. No network / cargo / claude-pilot needed.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

# dispatch-lib writes git noise to fd 9 (opened by the trace setup in a real
# dispatch). Open it here so the rescue's `2>&9` redirects have a destination.
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
        echo "    in:      '$(printf '%s' "$hay" | head -c 400)'"
    fi
}

assert_not_contains() {
    local label="$1" needle="$2" hay="$3"
    if grep -qF -- "$needle" <<<"$hay"; then
        FAIL=$((FAIL + 1)); echo "  ✗ $label"
        echo "    unexpectedly present: '$needle'"
    else
        PASS=$((PASS + 1)); echo "  ✓ $label"
    fi
}

# A worktree-shaped temp repo with one commit, so HEAD resolves.
make_repo() {
    local repo
    repo="$(mktemp -d "${TMPDIR:-/tmp}/mika-2031-test.XXXXXX")"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name "Test"
    git -C "$repo" config commit.gpgsign false
    printf 'seed\n' > "$repo/README.md"
    git -C "$repo" add -A
    git -C "$repo" commit -q -m "seed" --no-verify
    printf '%s' "$repo"
}

# Set the globals `_rescue_dirty_worktree` reads, then call it. PRE/POST_RUN_HEAD
# are both set to the repo's current HEAD — the zero-commit shape the rescue is
# scoped to.
run_rescue() {
    local repo="$1" skill="$2"
    WORKTREE_DIR="$repo"
    SKILL="$skill"
    REPO="mika"
    ISSUE_NUM="2031"
    BRANCH="fix/2031/dispatch-lib-dev-groom-has-no-dirty"
    SESSION_ID="sess-test"
    PILOT_EXIT=1
    RESULT="claude-pilot completed (status: terminated)."
    RESCUED_DIRTY_WORKTREE=""
    PRE_RUN_HEAD=$(git -C "$repo" rev-parse HEAD)
    POST_RUN_HEAD="$PRE_RUN_HEAD"
    _rescue_dirty_worktree || true
}

# ============================================================================
# V1 (anti-vacuity): dev-groom, CLEAN tree — the rescue must do nothing
# ============================================================================
echo ""
echo "V1: dev-groom + clean worktree → no rescue at all"
echo "-------------------------------------------------"
REPO_DIR="$(make_repo)"
BEFORE_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
BEFORE_RESULT="claude-pilot completed (status: terminated)."
run_rescue "$REPO_DIR" "dev-groom"
assert_eq "clean tree: HEAD did not move" "$BEFORE_HEAD" "$(git -C "$REPO_DIR" rev-parse HEAD)"
assert_eq "clean tree: RESULT untouched" "$BEFORE_RESULT" "$RESULT"
assert_eq "clean tree: POST_RUN_HEAD untouched" "$BEFORE_HEAD" "$POST_RUN_HEAD"
assert_eq "clean tree: no draft-PR marker" "" "$RESCUED_DIRTY_WORKTREE"
rm -rf "$REPO_DIR"

# ============================================================================
# V2: dev-groom, uncommitted plan — preserved, and the note says where
# ============================================================================
echo ""
echo "V2: dev-groom + uncommitted plan → preserved, named, and reachable"
echo "------------------------------------------------------------------"
REPO_DIR="$(make_repo)"
BEFORE_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
mkdir -p "$REPO_DIR/docs/plans"
printf '# plan for mika#2031\n\nbody\n' > "$REPO_DIR/docs/plans/2026-08-30-001-fix-2031-x-plan.md"
run_rescue "$REPO_DIR" "dev-groom"
AFTER_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
assert_eq "dirty tree: HEAD advanced (content is in a commit)" "1" \
    "$( [ "$BEFORE_HEAD" != "$AFTER_HEAD" ] && echo 1 || echo 0 )"
assert_eq "dirty tree: plan is tracked at HEAD" "1" \
    "$(git -C "$REPO_DIR" ls-tree -r --name-only HEAD | grep -c 'docs/plans/2026-08-30-001-fix-2031-x-plan.md')"
assert_eq "dirty tree: worktree is clean afterwards" "" \
    "$(git -C "$REPO_DIR" status --porcelain)"
assert_eq "dirty tree: POST_RUN_HEAD advanced so _push_branch sees the commit" "$AFTER_HEAD" "$POST_RUN_HEAD"
# R6 — what, where, and how to reach it.
assert_contains "note names the rescued file" "docs/plans/2026-08-30-001-fix-2031-x-plan.md" "$RESULT"
assert_contains "note names the rescue commit sha" \
    "$(git -C "$REPO_DIR" rev-parse --short HEAD)" "$RESULT"
assert_contains "note names the branch" "fix/2031/dispatch-lib-dev-groom-has-no-dirty" "$RESULT"
assert_contains "commit subject says a plan was staged" "plan staged by post-flight recovery (mika#2031)" \
    "$(git -C "$REPO_DIR" log -1 --format=%s)"
assert_contains "prior RESULT is preserved below the note" "claude-pilot completed (status: terminated)." "$RESULT"
rm -rf "$REPO_DIR"

# ============================================================================
# V3: dev-groom rescue must not open a PR, and is not a pipeline failure
# ============================================================================
echo ""
echo "V3: dev-groom rescue → no draft PR, not reported as PIPELINE FAILURE"
echo "--------------------------------------------------------------------"
REPO_DIR="$(make_repo)"
mkdir -p "$REPO_DIR/docs/plans"
printf '# plan\n' > "$REPO_DIR/docs/plans/2026-08-30-001-fix-2031-y-plan.md"
run_rescue "$REPO_DIR" "dev-groom"
assert_eq "dev-groom: RESCUED_DIRTY_WORKTREE not set (no draft PR)" "" "$RESCUED_DIRTY_WORKTREE"
assert_not_contains "dev-groom: successful rescue is not a PIPELINE FAILURE" "PIPELINE FAILURE:" "$RESULT"
rm -rf "$REPO_DIR"

# ============================================================================
# V4: dev-groom, only scaffold paths dirty — nothing to rescue
# ============================================================================
echo ""
echo "V4: dev-groom + scaffold-only dirt → no commit (exclusions hold)"
echo "----------------------------------------------------------------"
REPO_DIR="$(make_repo)"
BEFORE_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
mkdir -p "$REPO_DIR/.claude/commands"
printf 'slash\n' > "$REPO_DIR/.claude/commands/mika.md"
printf '{}\n' > "$REPO_DIR/.claude/claude-pilot.json"
printf '{}\n' > "$REPO_DIR/.claude/settings.local.json"
run_rescue "$REPO_DIR" "dev-groom"
assert_eq "scaffold-only: HEAD did not move" "$BEFORE_HEAD" "$(git -C "$REPO_DIR" rev-parse HEAD)"
assert_eq "scaffold-only: draft-PR marker explicitly cleared" "0" "$RESCUED_DIRTY_WORKTREE"
rm -rf "$REPO_DIR"

# ============================================================================
# V5: dev-pilot regression — unchanged behavior (plus the new sha/branch line)
# ============================================================================
echo ""
echo "V5: dev-pilot + dirty tree → unchanged rescue behavior"
echo "------------------------------------------------------"
# Non-.rs content on purpose: the proactive `cargo fmt` is gated on staged *.rs,
# and this suite must run without a Rust toolchain.
REPO_DIR="$(make_repo)"
BEFORE_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
printf 'impl\n' > "$REPO_DIR/NOTES.md"
run_rescue "$REPO_DIR" "dev-pilot"
assert_eq "dev-pilot: HEAD advanced" "1" \
    "$( [ "$BEFORE_HEAD" != "$(git -C "$REPO_DIR" rev-parse HEAD)" ] && echo 1 || echo 0 )"
assert_eq "dev-pilot: draft-PR marker set" "1" "$RESCUED_DIRTY_WORKTREE"
assert_contains "dev-pilot: still classified PIPELINE FAILURE" "PIPELINE FAILURE:" "$RESULT"
assert_contains "dev-pilot: commit subject unchanged" "impl staged by post-flight recovery (mika#1282)" \
    "$(git -C "$REPO_DIR" log -1 --format=%s)"
assert_contains "dev-pilot: note names commit + branch (mika#2031 R6)" \
    "Rescued into commit $(git -C "$REPO_DIR" rev-parse --short HEAD) on branch fix/2031/dispatch-lib-dev-groom-has-no-dirty" "$RESULT"
rm -rf "$REPO_DIR"

# ============================================================================
# V6 (anti-vacuity): dev-pilot, CLEAN tree — the rescue must do nothing
# ============================================================================
echo ""
echo "V6: dev-pilot + clean worktree → no rescue at all"
echo "-------------------------------------------------"
REPO_DIR="$(make_repo)"
BEFORE_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
run_rescue "$REPO_DIR" "dev-pilot"
assert_eq "clean tree: HEAD did not move" "$BEFORE_HEAD" "$(git -C "$REPO_DIR" rev-parse HEAD)"
assert_eq "clean tree: no draft-PR marker" "" "$RESCUED_DIRTY_WORKTREE"
assert_not_contains "clean tree: no rescue note" "Rescued into commit" "$RESULT"
rm -rf "$REPO_DIR"

# ============================================================================
# V7: unknown skill — the skill guard still holds
# ============================================================================
echo ""
echo "V7: unknown skill + dirty tree → guard holds, nothing committed"
echo "---------------------------------------------------------------"
REPO_DIR="$(make_repo)"
BEFORE_HEAD=$(git -C "$REPO_DIR" rev-parse HEAD)
printf 'stray\n' > "$REPO_DIR/stray.md"
run_rescue "$REPO_DIR" "some-other-skill"
assert_eq "unknown skill: HEAD did not move" "$BEFORE_HEAD" "$(git -C "$REPO_DIR" rev-parse HEAD)"
assert_eq "unknown skill: tree left untouched" "?? stray.md" "$(git -C "$REPO_DIR" status --porcelain)"
rm -rf "$REPO_DIR"

# ============================================================================
# V8 (static): the rescue is reached from _post_flight_recovery, and the
# dev-pilot-only skill gate the ticket reported is gone.
# ============================================================================
echo ""
echo "V8: call site + retired dev-pilot-only gate"
echo "-------------------------------------------"
assert_eq "_post_flight_recovery calls _rescue_dirty_worktree" "1" \
    "$(grep -c '^        _rescue_dirty_worktree$' "$DISPATCH_LIB")"
assert_eq "old dev-pilot-only rescue gate is gone" "0" \
    "$(grep -c 'PRE_RUN_HEAD" = "$POST_RUN_HEAD" \] && \[ "$SKILL" = "dev-pilot"' "$DISPATCH_LIB")"

echo ""
echo "===================================================================="
echo "Passed: $PASS   Failed: $FAIL"
echo "===================================================================="
[ "$FAIL" -eq 0 ] || exit 1
