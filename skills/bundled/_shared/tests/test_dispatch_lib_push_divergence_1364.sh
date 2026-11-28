#!/bin/bash
# Divergence de push et échec de rebase (mika#1364) — extrait de test-dispatch-lib.sh par mika#3178, vol 2.
# La conscience qu'a le push de la divergence avec la base, la remontée d'un
# échec de rebase plutôt que son écrasement, et le scan de cardinalité de
# `_is_scaffold_path` (mika#2144).
#
# Plage d'origine : lignes 2816–3921 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 890f3f5f. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
# ============================================================================
# Test 12: Push divergence awareness + rebase failure surfacing (mika#1364)
# ============================================================================

echo ""
echo "Test 12: _push_branch divergence awareness (mika#1364)"
echo "-------------------------------------------------------"

# Helper: create a throwaway bare repo + working clone, wired with file:// remotes.
# Sets FIXTURE_BARE, FIXTURE_CLONE. Caller must clean up via _fixture_cleanup.
_fixture_setup() {
    FIXTURE_BARE=$(mktemp -d)
    FIXTURE_CLONE=$(mktemp -d)

    git -C "$FIXTURE_BARE" init --bare -q
    git clone -q "file://$FIXTURE_BARE" "$FIXTURE_CLONE"
    git -C "$FIXTURE_CLONE" config user.email "test@mika.local"
    git -C "$FIXTURE_CLONE" config user.name "mika test"

    # Initial commit on main so we have a base.
    echo "base" > "$FIXTURE_CLONE/file.txt"
    git -C "$FIXTURE_CLONE" add file.txt
    git -C "$FIXTURE_CLONE" commit -q -m "initial commit"
    git -C "$FIXTURE_CLONE" push -q origin main
}

_fixture_cleanup() {
    rm -rf "$FIXTURE_BARE" "$FIXTURE_CLONE" 2>/dev/null
}

# Safety assertion: verify fixture remotes are local file:// paths, never the real origin.
_assert_fixture_is_local() {
    local remote_url
    remote_url=$(git -C "$FIXTURE_CLONE" remote get-url origin 2>/dev/null)
    if ! grep -q -- "^file://\|^/" <<<"$remote_url"; then
        echo "SAFETY ABORT: fixture remote is not local: $remote_url" >&2
        _fixture_cleanup
        return 1
    fi
}

# --- Test 12a: First push (no origin/$BRANCH) → plain push, no force ---
test_first_push() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # Create a branch with a commit, never pushed.
    git -C "$FIXTURE_CLONE" checkout -q -b feat/first-push
    echo "new" > "$FIXTURE_CLONE/new.txt"
    git -C "$FIXTURE_CLONE" add new.txt
    git -C "$FIXTURE_CLONE" commit -q -m "new feature"

    # Exercise _push_branch. Set globals directly (not VAR=val func syntax,
    # which does not persist function-internal assignments back to caller).
    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="feat/first-push"
    REPO="mika"
    RESULT=""
    _push_branch

    local failures=""
    # Assert: push succeeded.
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    # Assert: mode is first-push (not diverged).
    if grep -qF -- "mode=diverged" <<<"$RESULT"; then
        failures="${failures}should not be diverged mode for first push; "
    fi
    # Assert: remote ref now exists.
    if ! git -C "$FIXTURE_CLONE" rev-parse --verify "origin/feat/first-push" >/dev/null 2>&1; then
        failures="${failures}origin/feat/first-push should exist after push; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12A=$(test_first_push 2>/dev/null)
if [ "$RESULT_12A" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ First push: plain push, no force"
else
    FAIL=$((FAIL + 1)); echo "  ✗ First push: $RESULT_12A"
fi

# --- Test 12b: Fast-forward (origin/$BRANCH is ancestor of HEAD) → plain push ---
test_fast_forward_push() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # Create and push a branch.
    git -C "$FIXTURE_CLONE" checkout -q -b feat/ff-push
    echo "v1" > "$FIXTURE_CLONE/feature.txt"
    git -C "$FIXTURE_CLONE" add feature.txt
    git -C "$FIXTURE_CLONE" commit -q -m "feature v1"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/ff-push

    # Add one more commit locally (fast-forward case).
    echo "v2" > "$FIXTURE_CLONE/feature.txt"
    git -C "$FIXTURE_CLONE" add feature.txt
    git -C "$FIXTURE_CLONE" commit -q -m "feature v2"

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="feat/ff-push"
    REPO="mika"
    RESULT=""
    _push_branch

    local failures=""
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    if ! grep -qF -- "mode=fast-forward" <<<"$RESULT"; then
        failures="${failures}should be fast-forward mode; "
    fi
    # Remote should match local HEAD.
    local local_head remote_head
    local_head=$(git -C "$FIXTURE_CLONE" rev-parse HEAD)
    remote_head=$(git -C "$FIXTURE_CLONE" rev-parse "origin/feat/ff-push")
    if [ "$local_head" != "$remote_head" ]; then
        failures="${failures}remote HEAD should match local HEAD after ff push; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12B=$(test_fast_forward_push 2>/dev/null)
if [ "$RESULT_12B" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Fast-forward push: plain push, no force"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Fast-forward push: $RESULT_12B"
fi

# --- Test 12c: Diverged (stale remote, branch rebased onto advanced main) → force-with-lease ---
# This is THE title fix — the case that strands work on main today.
test_diverged_force_with_lease() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # 1. Create branch with a commit and push it (the "stale remote tip").
    git -C "$FIXTURE_CLONE" checkout -q -b feat/diverged-push
    echo "branch work" > "$FIXTURE_CLONE/branch.txt"
    git -C "$FIXTURE_CLONE" add branch.txt
    git -C "$FIXTURE_CLONE" commit -q -m "branch work"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/diverged-push

    # 2. Advance main past the branch point (non-conflicting).
    git -C "$FIXTURE_CLONE" checkout -q main
    echo "main advance" > "$FIXTURE_CLONE/main-only.txt"
    git -C "$FIXTURE_CLONE" add main-only.txt
    git -C "$FIXTURE_CLONE" commit -q -m "advance main"
    git -C "$FIXTURE_CLONE" push -q origin main

    # 3. Go back to the feature branch and rebase onto advanced main.
    git -C "$FIXTURE_CLONE" checkout -q feat/diverged-push
    git -C "$FIXTURE_CLONE" fetch -q origin main
    git -C "$FIXTURE_CLONE" rebase origin/main

    # Now local HEAD has new SHAs (rebased), but origin/feat/diverged-push
    # still points at the old pre-rebase tip. This is the diverged state.

    # 4. Add one more commit (simulates pilot work).
    echo "pilot impl" > "$FIXTURE_CLONE/impl.txt"
    git -C "$FIXTURE_CLONE" add impl.txt
    git -C "$FIXTURE_CLONE" commit -q -m "pilot implementation"

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="feat/diverged-push"
    REPO="mika"
    RESULT=""
    _push_branch

    local failures=""
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    if ! grep -qF -- "mode=diverged" <<<"$RESULT"; then
        failures="${failures}should be diverged mode; "
    fi
    # Remote should match local HEAD.
    local local_head remote_head
    local_head=$(git -C "$FIXTURE_CLONE" rev-parse HEAD)
    git -C "$FIXTURE_CLONE" fetch -q origin feat/diverged-push
    remote_head=$(git -C "$FIXTURE_CLONE" rev-parse "origin/feat/diverged-push")
    if [ "$local_head" != "$remote_head" ]; then
        failures="${failures}remote HEAD should match local HEAD after force-with-lease push; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12C=$(test_diverged_force_with_lease 2>/dev/null)
if [ "$RESULT_12C" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Diverged push: force-with-lease succeeds (title fix)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Diverged push: $RESULT_12C"
fi

# --- Test 12d: Structural — force-with-lease uses explicit ref form (not blind --force) ---
# The lease-stale abort is a race condition between fetch and push that cannot be
# reliably reproduced in a single-process test (the fetch inside _push_branch always
# picks up the latest remote state). Instead, verify the safety contract structurally:
# - The diverged path uses --force-with-lease=$BRANCH:origin/$BRANCH (explicit ref form)
# - The diverged path does NOT use plain --force
# - The fast-forward and first-push paths do NOT use any force flag
#
# Note (mika#1857): the push_cmd construction was extracted into
# `_push_with_rebase_retry` (race-recovery helper). Both `_push_branch` (which
# constructs push_cmd for the primary attempt) and `_push_with_rebase_retry`
# (which reconstructs it for retries after rebase) must uphold the safety
# contract — the structural check reads BOTH function bodies as one haystack.
PUSH_FUNC_SRC="$(declare -f _push_branch)
$(declare -f _push_with_rebase_retry)"
# Verify explicit lease form (pins expected remote SHA)
if grep -q -- 'force-with-lease=.*BRANCH.*origin.*BRANCH' <<<"$PUSH_FUNC_SRC"; then
    PASS=$((PASS + 1)); echo "  ✓ Lease form: --force-with-lease uses explicit ref pinning"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Lease form: should use --force-with-lease=\$BRANCH:origin/\$BRANCH"
fi
# Verify no blind --force (without -with-lease)
if grep -q -- '--force[^-]' <<<"$PUSH_FUNC_SRC"; then
    FAIL=$((FAIL + 1)); echo "  ✗ Lease safety: found bare --force (should be --force-with-lease only)"
else
    PASS=$((PASS + 1)); echo "  ✓ Lease safety: no bare --force (only --force-with-lease)"
fi

# --- Test 12e: Conflicting rebase → REBASE_CONFLICT + reason surfaced (AC#2/AC#4) ---
echo ""
echo "Test 12e: Rebase conflict surfacing (mika#1364 AC#2/AC#4)"
echo "-----------------------------------------------------------"

test_rebase_conflict_surfaced() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # 1. Create branch with a change to file.txt and push.
    git -C "$FIXTURE_CLONE" checkout -q -b feat/conflict-rebase
    echo "branch version" > "$FIXTURE_CLONE/file.txt"
    git -C "$FIXTURE_CLONE" add file.txt
    git -C "$FIXTURE_CLONE" commit -q -m "branch change to file.txt"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/conflict-rebase

    # 2. Advance main with a CONFLICTING change to file.txt.
    git -C "$FIXTURE_CLONE" checkout -q main
    echo "main conflicting version" > "$FIXTURE_CLONE/file.txt"
    git -C "$FIXTURE_CLONE" add file.txt
    git -C "$FIXTURE_CLONE" commit -q -m "conflicting change on main"
    git -C "$FIXTURE_CLONE" push -q origin main

    # 3. Go back to the branch. Simulate what _set_up_worktree's rebase guard
    # does: compute BEHIND, then rebase.
    git -C "$FIXTURE_CLONE" checkout -q feat/conflict-rebase
    git -C "$FIXTURE_CLONE" fetch -q origin main

    local BEHIND WORKTREE_DIR BRANCH REPO ISSUE_NUM RESULT
    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="feat/conflict-rebase"
    REPO="mika"
    ISSUE_NUM="1364"
    RESULT=""
    BEHIND=$(git -C "$WORKTREE_DIR" rev-list --count HEAD..origin/main 2>/dev/null || echo 0)

    # The rebase guard logic (inlined from dispatch-lib.sh):
    local rebase_err exit_code=0
    rebase_err=$(mktemp "${TMPDIR:-/tmp}/dispatch-lib-rebase-err.XXXXXX")
    if git -C "$WORKTREE_DIR" rebase origin/main >/dev/null 2>"$rebase_err"; then
        rm -f "$rebase_err"
        # Should NOT succeed — we expect a conflict.
        echo "FAIL: rebase should have conflicted but succeeded"
        _fixture_cleanup
        return 1
    else
        CONFLICTS=$(git -C "$WORKTREE_DIR" diff --name-only --diff-filter=U 2>/dev/null | tr '\n' ' ')
        local rebase_reason rebase_mode
        rebase_reason=$(cat "$rebase_err" 2>/dev/null | head -20)
        if [ -n "$CONFLICTS" ]; then
            rebase_mode="conflict"
        else
            rebase_mode="other"
        fi
        git -C "$WORKTREE_DIR" rebase --abort 2>/dev/null || true
        rm -f "$rebase_err"
        RESULT="STATUS=REBASE_CONFLICT
Branch ${BRANCH} is ${BEHIND} commits behind origin/main.
Rebase failure mode: ${rebase_mode}
Conflicted files: ${CONFLICTS:-<none>}
Rebase stderr: ${rebase_reason:-<empty>}
Resolve manually before re-dispatching ${REPO}#${ISSUE_NUM}."
    fi

    local failures=""
    # Assert: STATUS=REBASE_CONFLICT present.
    if ! grep -qF -- "STATUS=REBASE_CONFLICT" <<<"$RESULT"; then
        failures="${failures}RESULT missing STATUS=REBASE_CONFLICT; "
    fi
    # Assert: conflict mode token present (AC#4).
    if ! grep -qF -- "Rebase failure mode: conflict" <<<"$RESULT"; then
        failures="${failures}RESULT missing 'Rebase failure mode: conflict'; "
    fi
    # Assert: conflicted filename present.
    if ! grep -qF -- "file.txt" <<<"$RESULT"; then
        failures="${failures}RESULT missing conflicted filename 'file.txt'; "
    fi
    # Assert: rebase stderr is non-empty (AC#4 — not /dev/null).
    if grep -qF -- "Rebase stderr: <empty>" <<<"$RESULT"; then
        failures="${failures}RESULT has empty rebase stderr (should be surfaced); "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12E=$(test_rebase_conflict_surfaced 2>/dev/null)
if [ "$RESULT_12E" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Rebase conflict: STATUS=REBASE_CONFLICT + mode + stderr surfaced"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Rebase conflict: $RESULT_12E"
fi

# --- Test 12f: Full _push_branch call chain with dedup-rebase → diverged → force (F3) ---
echo ""
echo "Test 12f: Dedup-rebase → diverged → force-with-lease composition (mika#1364 F3)"
echo "---------------------------------------------------------------------------------"

test_dedup_rebase_diverged_force() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # 1. Create branch with two commits: one that will be patch-equivalent to
    # a main commit (duplicate), and one that is genuinely new.
    git -C "$FIXTURE_CLONE" checkout -q -b feat/dedup-diverge
    echo "duplicate content" > "$FIXTURE_CLONE/dup.txt"
    git -C "$FIXTURE_CLONE" add dup.txt
    git -C "$FIXTURE_CLONE" commit -q -m "add dup.txt"
    echo "unique content" > "$FIXTURE_CLONE/unique.txt"
    git -C "$FIXTURE_CLONE" add unique.txt
    git -C "$FIXTURE_CLONE" commit -q -m "add unique.txt"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/dedup-diverge

    # 2. Cherry-pick the "dup.txt" commit onto main (making it patch-equivalent).
    git -C "$FIXTURE_CLONE" checkout -q main
    # Recreate the same content to make it patch-equivalent.
    echo "duplicate content" > "$FIXTURE_CLONE/dup.txt"
    git -C "$FIXTURE_CLONE" add dup.txt
    git -C "$FIXTURE_CLONE" commit -q -m "add dup.txt on main too"
    # Also advance main with another non-conflicting change.
    echo "main extra" > "$FIXTURE_CLONE/main-extra.txt"
    git -C "$FIXTURE_CLONE" add main-extra.txt
    git -C "$FIXTURE_CLONE" commit -q -m "advance main further"
    git -C "$FIXTURE_CLONE" push -q origin main

    # 3. Go back to the feature branch. Simulate a rebase (as _set_up_worktree does).
    git -C "$FIXTURE_CLONE" checkout -q feat/dedup-diverge
    git -C "$FIXTURE_CLONE" fetch -q origin main
    git -C "$FIXTURE_CLONE" rebase origin/main

    # Now the branch is rebased (diverged from origin/feat/dedup-diverge).
    # _check_duplicate_commits will detect the dup and rebase again, which
    # may further rewrite history. Then _push_branch should force-with-lease.

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="feat/dedup-diverge"
    REPO="mika"
    RESULT=""
    _push_branch

    local failures=""
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    # Remote should match local HEAD.
    local local_head remote_head
    local_head=$(git -C "$FIXTURE_CLONE" rev-parse HEAD)
    git -C "$FIXTURE_CLONE" fetch -q origin feat/dedup-diverge
    remote_head=$(git -C "$FIXTURE_CLONE" rev-parse "origin/feat/dedup-diverge")
    if [ "$local_head" != "$remote_head" ]; then
        failures="${failures}remote HEAD should match local HEAD after push; "
    fi
    # unique.txt should be present (not lost by dedup).
    if [ ! -f "$FIXTURE_CLONE/unique.txt" ]; then
        failures="${failures}unique.txt should still exist; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12F=$(test_dedup_rebase_diverged_force 2>/dev/null)
if [ "$RESULT_12F" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Dedup-rebase → diverged → force-with-lease composition"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Dedup-rebase composition: $RESULT_12F"
fi

# --- Test 12g: Structural assertions on dispatch-lib.sh (mika#1364) ---
echo ""
echo "Test 12g: Structural assertions (mika#1364)"
echo "---------------------------------------------"

# _push_branch uses merge-base --is-ancestor for divergence detection.
# Note (mika#1857): the push_cmd construction was extracted into
# `_push_with_rebase_retry` — the force-with-lease structural check reads both
# function bodies as one haystack (same rationale as test 12d above).
PUSH_FUNC=$(declare -f _push_branch)
PUSH_FUNC_WITH_HELPER="$(declare -f _push_branch)
$(declare -f _push_with_rebase_retry)"
assert_contains "_push_branch uses merge-base --is-ancestor" "merge-base --is-ancestor" "$PUSH_FUNC"
assert_contains "_push_branch (or retry helper) uses force-with-lease" "force-with-lease" "$PUSH_FUNC_WITH_HELPER"
assert_contains "_push_branch tracks push_mode" "push_mode" "$PUSH_FUNC"

# _set_up_worktree rebase captures stderr (no 2>/dev/null on rebase)
# Check that the rebase at the setup site uses a temp file, not /dev/null
SETUP_REBASE_REGION=$(sed -n '/Rebase-or-abort guard/,/^[[:space:]]*fi$/p' "$DISPATCH_LIB" | head -40)
assert_contains "Setup rebase captures stderr to temp file" "dispatch-lib-rebase-err" "$SETUP_REBASE_REGION"
assert_contains "Setup rebase surfaces failure mode token" "rebase_mode" "$SETUP_REBASE_REGION"

# _check_duplicate_commits rebase captures stderr
DEDUP_FUNC=$(declare -f _check_duplicate_commits)
assert_contains "Dedup rebase captures stderr to temp file" "dedup-rebase-err" "$DEDUP_FUNC"
assert_contains "Dedup rebase surfaces reason in RESULT" "Dedup-rebase failed" "$DEDUP_FUNC"

# --- Test 12h: mika#1407 — push decision keyed on the remote-tracking branch ---
echo ""
echo "Test 12h: Stale-main conflation no-op (mika#1407)"
echo "-------------------------------------------------"

# Structural: the three-state rationale is documented in source, and the push
# decision is keyed on origin/$BRANCH..HEAD (the remote-tracking branch), never
# on local main. Comments are stripped by `declare -f`, so read the source file.
PUSH_SRC=$(sed -n '/^_push_branch() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "_push_branch documents mika#1407 three-state rationale" "mika#1407" "$PUSH_SRC"
assert_contains "_push_branch keys push decision on origin/\$BRANCH..HEAD" 'origin/$BRANCH..HEAD' "$PUSH_SRC"

# Behavioral: HEAD == origin/<branch> (nothing to push) WHILE local `main` is
# stale (behind origin/main). _push_branch must no-op — return 0, push nothing,
# emit no divergence/abort text. base-behind-main is orthogonal to the push
# decision; the pilot's old prose diagnostic conflated the two (mika#1407).
test_noop_when_head_equals_remote_stale_main() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # Feature branch at HEAD == origin/<branch> (ahead==0).
    git -C "$FIXTURE_CLONE" checkout -q -b fix/1407-noop
    echo "plan" > "$FIXTURE_CLONE/plan.txt"
    git -C "$FIXTURE_CLONE" add plan.txt
    git -C "$FIXTURE_CLONE" commit -q -m "groom plan (content-only)"
    git -C "$FIXTURE_CLONE" push -q -u origin fix/1407-noop

    # Advance origin/main via a throwaway clone so the working clone's local
    # `main` ref falls behind origin/main — the exact conflation trigger.
    local advance_clone
    advance_clone=$(mktemp -d)
    git clone -q "file://$FIXTURE_BARE" "$advance_clone"
    git -C "$advance_clone" config user.email "test@mika.local"
    git -C "$advance_clone" config user.name "mika test"
    echo "advance" > "$advance_clone/main-advance.txt"
    git -C "$advance_clone" add main-advance.txt
    git -C "$advance_clone" commit -q -m "advance main"
    git -C "$advance_clone" push -q origin main
    rm -rf "$advance_clone"
    # Update remote-tracking refs but leave local `main` behind origin/main.
    git -C "$FIXTURE_CLONE" fetch -q origin

    local behind remote_before
    behind=$(git -C "$FIXTURE_CLONE" rev-list --count main..origin/main 2>/dev/null || echo 0)
    remote_before=$(git -C "$FIXTURE_CLONE" rev-parse "origin/fix/1407-noop")

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="fix/1407-noop"
    REPO="mika"
    RESULT=""
    local rc=0
    _push_branch || rc=$?

    local remote_after
    remote_after=$(git -C "$FIXTURE_CLONE" rev-parse "origin/fix/1407-noop")

    local failures=""
    [ "$rc" -eq 0 ] || failures="${failures}expected return 0 (no-op), got $rc; "
    [ "${behind:-0}" -ge 1 ] || failures="${failures}fixture should leave local main behind origin/main; "
    [ "$remote_before" = "$remote_after" ] || failures="${failures}remote HEAD must be unchanged (nothing pushed); "
    if grep -qiE -- "divergence|abort|reconciliation|Push: pushed|Push: FAILED" <<<"$RESULT"; then
        failures="${failures}RESULT must contain no push/abort/divergence text; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12H=$(test_noop_when_head_equals_remote_stale_main 2>/dev/null)
if [ "$RESULT_12H" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ No-op when HEAD==origin/branch + stale local main (mika#1407)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ No-op on stale-main conflation (mika#1407): $RESULT_12H"
fi

# --- Test 12i: Resume with dirty worktree → cleaned + stashed, rebase succeeds (mika#1414) ---
# A reused worktree carrying UNEXPECTED dirty residue (modified non-scaffold tracked
# file + untracked file) must not crash `git rebase` on the resume path. The real
# _clean_worktree_for_rebase helper must stash the residue (operator-recoverable) and
# leave a clean tree so the rebase proceeds. Calling the real function (not an inline
# copy of the guard) eliminates the Test-12e drift risk called out in the plan.
test_resume_dirty_worktree_cleaned() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # Feature branch with its own commit, so the rebase actually replays work.
    # Commit a .gitignore so we can prove `clean -fd` (no -x) preserves ignored files.
    git -C "$FIXTURE_CLONE" checkout -q -b feat/resume-dirty
    echo "feat" > "$FIXTURE_CLONE/feature.txt"
    printf '*.keep\n' > "$FIXTURE_CLONE/.gitignore"
    git -C "$FIXTURE_CLONE" add feature.txt .gitignore
    git -C "$FIXTURE_CLONE" commit -q -m "feature work"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/resume-dirty

    # Advance origin/main by one NON-conflicting commit (touches a different file)
    # via a throwaway clone, so BEHIND>0 and a clean rebase is possible.
    local advance_clone
    advance_clone=$(mktemp -d)
    git clone -q "file://$FIXTURE_BARE" "$advance_clone"
    git -C "$advance_clone" config user.email "test@mika.local"
    git -C "$advance_clone" config user.name "mika test"
    echo "advance" > "$advance_clone/main-advance.txt"
    git -C "$advance_clone" add main-advance.txt
    git -C "$advance_clone" commit -q -m "advance main"
    git -C "$advance_clone" push -q origin main
    rm -rf "$advance_clone"
    git -C "$FIXTURE_CLONE" fetch -q origin

    # Dirty the worktree with UNEXPECTED residue: a modified non-scaffold tracked
    # file + an untracked file. Neither is a dispatch-lib-owned scaffold path, so
    # both survive the surgical resets and must be stashed by the blanket fallback.
    echo "dirty" >> "$FIXTURE_CLONE/file.txt"
    echo "junk" > "$FIXTURE_CLONE/junk.tmp"
    # A gitignored, uncommitted file (stands in for .claude/*.local.json) — must
    # survive `clean -fd` (no -x), which is the load-bearing reason -x is omitted.
    echo "keepme" > "$FIXTURE_CLONE/config.keep"

    WORKTREE_DIR="$FIXTURE_CLONE"
    LOG_ID="test-1414"
    RESUME_CLEANUP_STASH=""

    # Call the REAL helper.
    _clean_worktree_for_rebase "$FIXTURE_CLONE"

    local status_after
    status_after=$(git -C "$FIXTURE_CLONE" status --porcelain 2>/dev/null)

    # The rebase the helper guards must now succeed on the clean tree.
    local rebase_rc=0
    git -C "$FIXTURE_CLONE" rebase origin/main >/dev/null 2>&1 || rebase_rc=$?

    local failures=""
    # AC1 + AC3: tree clean after cleanup, rebase succeeds (no REBASE_CONFLICT).
    [ -z "$status_after" ] || failures="${failures}worktree not clean after cleanup: [$status_after]; "
    [ "$rebase_rc" -eq 0 ] || failures="${failures}rebase should succeed on clean tree, got rc=$rebase_rc; "
    # AC2: a stash with the descriptive name was created.
    if ! git -C "$FIXTURE_CLONE" stash list 2>/dev/null | grep -qF "dispatch-lib-resume-cleanup-test-1414-"; then
        failures="${failures}expected stash named dispatch-lib-resume-cleanup-test-1414-*; "
    fi
    # AC4: the immutable stash SHA was captured for operator recovery.
    if [ -z "$RESUME_CLEANUP_STASH" ]; then
        failures="${failures}RESUME_CLEANUP_STASH should hold the stash SHA; "
    elif ! git -C "$FIXTURE_CLONE" cat-file -e "$RESUME_CLEANUP_STASH" 2>/dev/null; then
        failures="${failures}RESUME_CLEANUP_STASH ($RESUME_CLEANUP_STASH) is not a valid git object; "
    fi
    # AC5: stashed content is recoverable — both the modified tracked file and the
    # untracked file. The untracked file lives in the stash's third parent (^3),
    # created by `stash push --include-untracked`.
    if [ -n "$RESUME_CLEANUP_STASH" ]; then
        if ! git -C "$FIXTURE_CLONE" stash show "$RESUME_CLEANUP_STASH" 2>/dev/null | grep -qF 'file.txt'; then
            failures="${failures}stash diff should contain the dirtied tracked file (file.txt); "
        fi
        if ! git -C "$FIXTURE_CLONE" ls-tree -r "${RESUME_CLEANUP_STASH}^3" 2>/dev/null | grep -qF 'junk.tmp'; then
            failures="${failures}stash should preserve the untracked file (junk.tmp); "
        fi
    fi
    # clean -fd omits -x: the gitignored file must survive the blanket fallback.
    if [ ! -f "$FIXTURE_CLONE/config.keep" ]; then
        failures="${failures}gitignored file (config.keep) must survive clean -fd (no -x); "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12I=$(test_resume_dirty_worktree_cleaned 2>/dev/null)
if [ "$RESULT_12I" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Resume dirty worktree: cleaned + stashed, rebase succeeds (mika#1414)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Resume dirty worktree cleanup (mika#1414): $RESULT_12I"
fi

# --- Test 12j: Resume with ONLY scaffold dirt → surgical reset, NO stash (mika#1414) ---
# The dominant production case: a stale committed-plan / .claude/commands edit (e.g. a
# `make deploy` stale mika.md). Tier-2 surgical resets must clean it to HEAD WITHOUT
# creating an operator-recovery stash — the plan's accepted-consequence contract (AC2
# reconciliation). A regression that stashes scaffold residue would fill recovery
# stashes with noise; this test locks the no-stash boundary.
test_resume_surgical_only_no_stash() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b feat/surgical-only
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    echo "plan v1" > "$FIXTURE_CLONE/docs/plans/p.md"
    echo "feat" > "$FIXTURE_CLONE/feature.txt"
    git -C "$FIXTURE_CLONE" add docs/plans/p.md feature.txt
    git -C "$FIXTURE_CLONE" commit -q -m "feature + committed plan"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/surgical-only

    # Advance origin/main (non-conflicting) so BEHIND>0.
    local advance_clone
    advance_clone=$(mktemp -d)
    git clone -q "file://$FIXTURE_BARE" "$advance_clone"
    git -C "$advance_clone" config user.email "test@mika.local"
    git -C "$advance_clone" config user.name "mika test"
    echo "advance" > "$advance_clone/main-advance.txt"
    git -C "$advance_clone" add main-advance.txt
    git -C "$advance_clone" commit -q -m "advance main"
    git -C "$advance_clone" push -q origin main
    rm -rf "$advance_clone"
    git -C "$FIXTURE_CLONE" fetch -q origin

    # Dirty ONLY a dispatch-lib-owned scaffold path (a stale committed-plan edit).
    echo "stale edit" >> "$FIXTURE_CLONE/docs/plans/p.md"

    WORKTREE_DIR="$FIXTURE_CLONE"
    LOG_ID="test-1414-surgical"
    RESUME_CLEANUP_STASH=""

    _clean_worktree_for_rebase "$FIXTURE_CLONE"

    local status_after
    status_after=$(git -C "$FIXTURE_CLONE" status --porcelain 2>/dev/null)
    local rebase_rc=0
    git -C "$FIXTURE_CLONE" rebase origin/main >/dev/null 2>&1 || rebase_rc=$?

    local failures=""
    [ -z "$status_after" ] || failures="${failures}worktree not clean after surgical reset: [$status_after]; "
    [ "$rebase_rc" -eq 0 ] || failures="${failures}rebase should succeed, got rc=$rebase_rc; "
    # The accepted-consequence contract: scaffold-only dirt is reset, NOT stashed.
    [ -z "$RESUME_CLEANUP_STASH" ] || failures="${failures}scaffold-only dirt must NOT create a stash (got $RESUME_CLEANUP_STASH); "
    if git -C "$FIXTURE_CLONE" stash list 2>/dev/null | grep -qF "dispatch-lib-resume-cleanup-"; then
        failures="${failures}no resume-cleanup stash should exist for scaffold-only dirt; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12J=$(test_resume_surgical_only_no_stash 2>/dev/null)
if [ "$RESULT_12J" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Resume scaffold-only dirt: surgical reset, no stash (mika#1414)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Resume scaffold-only no-stash (mika#1414): $RESULT_12J"
fi

# --- Test 12k: Resume aborts a half-finished rebase before cleanup (mika#1414 tier 1) ---
# A prior dispatch killed mid-rebase leaves a rebase-in-progress state. Tier 1 must
# `rebase --abort` it; otherwise the stash below fails and the exact crash this fix
# targets recurs. Locks the hardening tier.
test_resume_aborts_half_finished_rebase() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # Commit a shared file on main both sides will edit (forces a rebase conflict).
    echo "base-shared" > "$FIXTURE_CLONE/shared.txt"
    git -C "$FIXTURE_CLONE" add shared.txt
    git -C "$FIXTURE_CLONE" commit -q -m "add shared"
    git -C "$FIXTURE_CLONE" push -q origin main

    git -C "$FIXTURE_CLONE" checkout -q -b feat/half-rebase
    echo "branch-change" > "$FIXTURE_CLONE/shared.txt"
    git -C "$FIXTURE_CLONE" add shared.txt
    git -C "$FIXTURE_CLONE" commit -q -m "branch edits shared"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/half-rebase

    # Advance origin/main with a CONFLICTING edit to the same file.
    local advance_clone
    advance_clone=$(mktemp -d)
    git clone -q "file://$FIXTURE_BARE" "$advance_clone"
    git -C "$advance_clone" config user.email "test@mika.local"
    git -C "$advance_clone" config user.name "mika test"
    git -C "$advance_clone" checkout -q main
    echo "main-change" > "$advance_clone/shared.txt"
    git -C "$advance_clone" add shared.txt
    git -C "$advance_clone" commit -q -m "main edits shared (conflict)"
    git -C "$advance_clone" push -q origin main
    rm -rf "$advance_clone"
    git -C "$FIXTURE_CLONE" fetch -q origin

    # Start a conflicting rebase and LEAVE it mid-flight (do not abort/resolve).
    git -C "$FIXTURE_CLONE" rebase origin/main >/dev/null 2>&1 || true

    local mid_rebase=0
    if [ -d "$FIXTURE_CLONE/.git/rebase-merge" ] || [ -d "$FIXTURE_CLONE/.git/rebase-apply" ]; then
        mid_rebase=1
    fi

    WORKTREE_DIR="$FIXTURE_CLONE"
    LOG_ID="test-1414-abort"
    RESUME_CLEANUP_STASH=""

    _clean_worktree_for_rebase "$FIXTURE_CLONE"

    local failures=""
    [ "$mid_rebase" -eq 1 ] || failures="${failures}precondition: expected a mid-rebase state in the fixture; "
    if [ -d "$FIXTURE_CLONE/.git/rebase-merge" ] || [ -d "$FIXTURE_CLONE/.git/rebase-apply" ]; then
        failures="${failures}tier 1 did not abort the in-progress rebase; "
    fi
    local status_after
    status_after=$(git -C "$FIXTURE_CLONE" status --porcelain 2>/dev/null)
    [ -z "$status_after" ] || failures="${failures}tree not clean after abort+cleanup: [$status_after]; "

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12K=$(test_resume_aborts_half_finished_rebase 2>/dev/null)
if [ "$RESULT_12K" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Resume aborts half-finished rebase (mika#1414 tier 1)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Resume half-finished-rebase abort (mika#1414): $RESULT_12K"
fi

# --- Test 12l: résidu d'échafaudage UNTRACKED seul → aucun stash (mika#2144) ---
#
# Le trou que mika#2144 ferme, et il est étroit : `git checkout HEAD -- docs/plans/`
# restaure les entrées CONNUES de l'arbre et ne supprime jamais un untracked —
# comportement git, pas accident local. Donc un pilote de groom qui écrit son plan
# puis meurt avant de committer (signature mika#2141) laisse un `?? docs/plans/…`
# que le Tier 2 ne peut pas mordre, que le Tier 3 stashe, et que le `clean -fd`
# qui suit supprime de toute façon. Les 11 stashes du seul 2026-09-01 sont de
# cette forme.
#
# Ce test est le contrôle négatif de la correction : contre le Tier 3 NON modifié
# il DOIT échouer (un stash apparaît). Une version verte avant correction ne
# mesure pas le trou et sa verdeur ne prouverait rien (classe mika#2205).
#
# `docs/plans/` porte ici un plan COMMITTÉ en plus de l'untracked, pour que le
# reset chirurgical du Tier 2 réussisse sur son pathspec : le test attesterait
# autrement un `checkout` en échec plutôt que le trou lui-même.
test_resume_untracked_scaffold_no_stash() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b feat/untracked-plan
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    echo "plan v1" > "$FIXTURE_CLONE/docs/plans/committed-plan.md"
    echo "feat" > "$FIXTURE_CLONE/feature.txt"
    git -C "$FIXTURE_CLONE" add docs/plans/committed-plan.md feature.txt
    git -C "$FIXTURE_CLONE" commit -q -m "feature + committed plan"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/untracked-plan

    # Advance origin/main (non-conflicting) so BEHIND>0 and the rebase replays.
    local advance_clone
    advance_clone=$(mktemp -d)
    git clone -q "file://$FIXTURE_BARE" "$advance_clone"
    git -C "$advance_clone" config user.email "test@mika.local"
    git -C "$advance_clone" config user.name "mika test"
    echo "advance" > "$advance_clone/main-advance.txt"
    git -C "$advance_clone" add main-advance.txt
    git -C "$advance_clone" commit -q -m "advance main"
    git -C "$advance_clone" push -q origin main
    rm -rf "$advance_clone"
    git -C "$FIXTURE_CLONE" fetch -q origin

    # Le résidu réellement producteur : un plan de groom jamais committé.
    echo "plan the pilot never committed" \
        > "$FIXTURE_CLONE/docs/plans/2026-09-29-001-chore-2144-untracked-plan.md"

    WORKTREE_DIR="$FIXTURE_CLONE"
    LOG_ID="test-2144-untracked"
    RESUME_CLEANUP_STASH=""

    # stderr dans un FICHIER, jamais `$(… 2>&1)` : une substitution de commande
    # ouvre un sous-shell, et `RESUME_CLEANUP_STASH` — l'effet de bord dont tout
    # ce test dépend — y resterait invisible. Mesuré : le contrôle négatif passait
    # alors sur la mauvaise assertion. Le fichier vit dans le dépôt NU, hors de
    # l'arbre de travail, donc il ne peut pas salir le `status --porcelain` qu'on
    # mesure, et `_fixture_cleanup` l'emporte avec lui.
    local stderr_file="$FIXTURE_BARE/cleanup.stderr" stderr_seen
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    stderr_seen=$(cat "$stderr_file" 2>/dev/null || true)

    local status_after
    status_after=$(git -C "$FIXTURE_CLONE" status --porcelain 2>/dev/null)
    local rebase_rc=0
    git -C "$FIXTURE_CLONE" rebase origin/main >/dev/null 2>&1 || rebase_rc=$?

    local failures=""
    # Le nettoyage est INCHANGÉ : seule l'émission du stash bouge (mika#2144 § 2).
    [ -z "$status_after" ] || failures="${failures}worktree not clean after cleanup: [$status_after]; "
    [ "$rebase_rc" -eq 0 ] || failures="${failures}rebase should succeed on clean tree, got rc=$rebase_rc; "
    # AC1 : résidu entièrement échafaudage → aucun stash, malgré l'untracked.
    [ -z "$RESUME_CLEANUP_STASH" ] || failures="${failures}untracked scaffold must NOT create a stash (got $RESUME_CLEANUP_STASH); "
    if git -C "$FIXTURE_CLONE" stash list 2>/dev/null | grep -qF "dispatch-lib-resume-cleanup-"; then
        failures="${failures}no resume-cleanup stash should exist for untracked scaffold; "
    fi
    # AC2 en négatif : le message de récupération ne doit pas être imprimé pour rien.
    if grep -qF 'resume-cleanup stashed dirty worktree' <<<"$stderr_seen"; then
        failures="${failures}recovery message printed for scaffold-only residue; "
    fi
    # U5 : l'abstention est DITE, sinon sa mesure n'existe pas.
    if ! grep -qF 'resume_cleanup_scaffold_only' <<<"$stderr_seen"; then
        failures="${failures}expected the abstention line resume_cleanup_scaffold_only; "
    fi
    if ! grep -qF 'classes=plans' <<<"$stderr_seen"; then
        failures="${failures}abstention line should name the class (classes=plans); "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12L=$(test_resume_untracked_scaffold_no_stash 2>/dev/null)
if [ "$RESULT_12L" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Resume untracked scaffold: no stash, abstention said (mika#2144)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Resume untracked scaffold no-stash (mika#2144): $RESULT_12L"
fi

# --- Test 12m: AC6 — les deux branches dans un seul test (mika#2144) ---
#
# La lettre d'AC6 : « une exécution réelle de la voie de reprise sur un worktree
# portant du `.iterate/` et rien d'autre ne crée aucun stash, et la même exécution
# avec un fichier de code modifié en crée un. Les deux, dans le même test. »
#
# `.iterate/` arrive bien au Tier 3 dans une fixture, et pas par artifice : le
# `rm -rf "$wt/.iterate"` du Tier 2 est gardé par `_assert_removable_worktree_path`,
# dont le terme T4 exige un chemin sous `/.claude/worktrees/`. Une fixture en
# `mktemp -d` est refusée — la forme exacte que R4 décrit en production (une
# branche antérieure à `ac1ee87e`, où `.iterate/` n'est ni ignoré ni forcément
# hors de l'index).
#
# La phase 2 est le contrôle négatif non négociable d'AC2 : elle doit rester verte
# avant ET après la correction. Si elle rougit, le filet est devenu muet et c'est
# la perte des 16 stashes de code en train de se rejouer.
test_resume_ac6_both_branches() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b feat/ac6-both
    echo "feat" > "$FIXTURE_CLONE/feature.txt"
    git -C "$FIXTURE_CLONE" add feature.txt
    git -C "$FIXTURE_CLONE" commit -q -m "feature work"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/ac6-both

    local failures=""
    # stderr dans un fichier, pour la raison écrite sur le test 12l : une
    # substitution de commande perdrait `RESUME_CLEANUP_STASH`. Le dépôt nu est
    # hors de l'arbre mesuré.
    local stderr_file="$FIXTURE_BARE/cleanup.stderr"

    # ---- Phase 1 : `.iterate/` et rien d'autre → aucun stash ----
    mkdir -p "$FIXTURE_CLONE/.iterate"
    echo "scratch" > "$FIXTURE_CLONE/.iterate/findings.txt"

    WORKTREE_DIR="$FIXTURE_CLONE"
    LOG_ID="test-2144-ac6-iterate"
    RESUME_CLEANUP_STASH=""
    local stderr_phase1
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    stderr_phase1=$(cat "$stderr_file" 2>/dev/null || true)

    [ -z "$RESUME_CLEANUP_STASH" ] || failures="${failures}phase1: .iterate/-only dirt must NOT stash (got $RESUME_CLEANUP_STASH); "
    if git -C "$FIXTURE_CLONE" stash list 2>/dev/null | grep -qF "dispatch-lib-resume-cleanup-test-2144-ac6-iterate"; then
        failures="${failures}phase1: no resume-cleanup stash should exist for .iterate/-only dirt; "
    fi
    if ! grep -qF 'classes=iterate' <<<"$stderr_phase1"; then
        failures="${failures}phase1: abstention line should name classes=iterate; "
    fi
    local status_p1
    status_p1=$(git -C "$FIXTURE_CLONE" status --porcelain 2>/dev/null)
    [ -z "$status_p1" ] || failures="${failures}phase1: tree not clean after cleanup: [$status_p1]; "

    # ---- Phase 2 : un fichier de code modifié → un stash, ET son message ----
    echo "real work the pilot did not commit" >> "$FIXTURE_CLONE/feature.txt"

    LOG_ID="test-2144-ac6-code"
    RESUME_CLEANUP_STASH=""
    local stderr_phase2
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    stderr_phase2=$(cat "$stderr_file" 2>/dev/null || true)

    if [ -z "$RESUME_CLEANUP_STASH" ]; then
        failures="${failures}phase2: a modified code file MUST be stashed (AC2); "
    elif ! git -C "$FIXTURE_CLONE" cat-file -e "$RESUME_CLEANUP_STASH" 2>/dev/null; then
        failures="${failures}phase2: RESUME_CLEANUP_STASH ($RESUME_CLEANUP_STASH) is not a valid git object; "
    elif ! git -C "$FIXTURE_CLONE" stash show "$RESUME_CLEANUP_STASH" 2>/dev/null | grep -qF 'feature.txt'; then
        failures="${failures}phase2: stashed content should hold the code file (feature.txt); "
    fi
    # Le message de récupération est la moitié d'AC2 que le ticket nomme.
    if ! grep -qF 'resume-cleanup stashed dirty worktree' <<<"$stderr_phase2"; then
        failures="${failures}phase2: the recovery message must still be printed; "
    fi
    if grep -qF 'resume_cleanup_scaffold_only' <<<"$stderr_phase2"; then
        failures="${failures}phase2: abstention line must NOT appear when code is present; "
    fi
    local status_p2
    status_p2=$(git -C "$FIXTURE_CLONE" status --porcelain 2>/dev/null)
    [ -z "$status_p2" ] || failures="${failures}phase2: tree not clean after cleanup: [$status_p2]; "

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12M=$(test_resume_ac6_both_branches 2>/dev/null)
if [ "$RESULT_12M" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ AC6: .iterate/-only no stash, code still stashed (mika#2144)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ AC6 both branches (mika#2144): $RESULT_12M"
fi

# --- Test 12n: renommages — le piège `R ` du format -z (mika#2144) ---
#
# `git status --porcelain -z` émet pour un renommage DEUX enregistrements
# NUL-séparés : la destination avec son préfixe `XY `, puis l'origine SANS préfixe.
# Un classificateur qui applique l'offset 3 aux deux tronque l'origine
# (`docs/plans/b.md` → `s/plans/b.md`), qui ne matche plus aucun motif.
#
# Le plan mika#2144 nomme le sens « sortant de docs/plans/ ». Livré seul, ce sens
# ne discrimine RIEN : sa destination est déjà du code, donc le premier
# enregistrement suffit à décider, et le test passerait avec ou sans le traitement
# de l'origine. Les trois volets sont donc livrés ensemble.
#
# Le volet 2 (l'origine est du code) est le contrôle de SÛRETÉ : sans lecture de
# l'origine, un `git mv code/y.rs docs/plans/d.md` verrait une destination
# d'échafaudage et cesserait de stasher du code. Fail-closed ou pas, c'est la
# perte qu'AC2 interdit.
#
# Le volet 3 est le volet DISCRIMINANT, et il vit sous `.iterate/` — pas sous
# `docs/plans/` comme le premier jet l'écrivait. Mesuré : le Tier 2 DÉMONTE un
# renommage interne à `docs/plans/` avant que le Tier 3 ne le voie
# (`checkout HEAD -- docs/plans/` restaure l'origine et laisse la destination
# seule → un seul enregistrement, `paths=1`), donc ce sens-là n'atteint jamais le
# chemin à deux enregistrements. `.iterate/` y arrive, et pas par artifice : le
# `rm -rf` du Tier 2 est refusé sur une fixture hors de `/.claude/worktrees/`, et
# un `.iterate/` TRACKÉ est exactement la population que R4 décrit (une branche
# antérieure à `ac1ee87e`, dont le commit s'intitule *untrack*). Sous un offset 3
# appliqué à tort à l'origine, `terate/a` ne matche plus rien et le résidu est
# stashé : ce volet rougit.
test_resume_rename_records() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b feat/renames
    mkdir -p "$FIXTURE_CLONE/docs/plans" "$FIXTURE_CLONE/code"
    # Contenu identique et assez long pour que la détection de renommage de
    # `git status` (activée par défaut) apparie les deux côtés.
    local filler
    filler=$(printf 'ligne de contenu stable %s\n' 1 2 3 4 5 6 7 8 9 10)
    mkdir -p "$FIXTURE_CLONE/.iterate"
    printf '%s' "$filler" > "$FIXTURE_CLONE/docs/plans/a-plan.md"
    printf '%s' "$filler" > "$FIXTURE_CLONE/code/y.rs"
    # `.iterate/` TRACKÉ : la population de R4 (branche antérieure à `ac1ee87e`).
    printf '%s' "$filler" > "$FIXTURE_CLONE/.iterate/findings-a.txt"
    git -C "$FIXTURE_CLONE" add docs/plans code .iterate
    git -C "$FIXTURE_CLONE" commit -q -m "plans + code + tracked .iterate"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/renames

    WORKTREE_DIR="$FIXTURE_CLONE"
    local failures="" stderr_file="$FIXTURE_BARE/cleanup.stderr"

    # ---- Volet 1 : docs/plans/a-plan.md → code/x.rs  ⇒ STASH ----
    git -C "$FIXTURE_CLONE" mv docs/plans/a-plan.md code/x.rs
    LOG_ID="test-2144-rename-out"
    RESUME_CLEANUP_STASH=""
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    [ -n "$RESUME_CLEANUP_STASH" ] \
        || failures="${failures}volet1: a rename OUT of docs/plans/ must stash; "

    # ---- Volet 2 : code/y.rs → docs/plans/d-plan.md  ⇒ STASH (origine = code) ----
    git -C "$FIXTURE_CLONE" mv code/y.rs docs/plans/d-plan.md
    LOG_ID="test-2144-rename-in"
    RESUME_CLEANUP_STASH=""
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    [ -n "$RESUME_CLEANUP_STASH" ] \
        || failures="${failures}volet2: a rename INTO docs/plans/ from code must stash (the origin is work); "

    # ---- Volet 3 : .iterate/findings-a → .iterate/findings-b  ⇒ AUCUN stash ----
    # Le volet discriminant : les deux enregistrements sont de l'échafaudage, et
    # seul un traitement correct de l'origine sans préfixe le voit.
    git -C "$FIXTURE_CLONE" mv .iterate/findings-a.txt .iterate/findings-b.txt
    LOG_ID="test-2144-rename-internal"
    RESUME_CLEANUP_STASH=""
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    local stderr_internal
    stderr_internal=$(cat "$stderr_file" 2>/dev/null || true)
    [ -z "$RESUME_CLEANUP_STASH" ] \
        || failures="${failures}volet3: a rename WITHIN .iterate/ must NOT stash (got $RESUME_CLEANUP_STASH); "
    if ! grep -qF 'resume_cleanup_scaffold_only' <<<"$stderr_internal"; then
        failures="${failures}volet3: expected the abstention line for an internal rename; "
    fi
    # Les deux enregistrements comptent : destination ET origine.
    if ! grep -qE 'resume_cleanup_scaffold_only paths=2 classes=iterate ' <<<"$stderr_internal"; then
        failures="${failures}volet3: both rename records should be counted (paths=2 classes=iterate), got [$stderr_internal]; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12N=$(test_resume_rename_records 2>/dev/null)
if [ "$RESULT_12N" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Renames: both -z records classified (mika#2144)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Rename -z records (mika#2144): $RESULT_12N"
fi

# --- mika#2144 : `_is_scaffold_path` est le seul site déclarant LA LISTE --------
#
# mika#2157 (R3) écrivait déjà, dans dispatch-lib.sh, que ses deux autorités
# « peuvent diverger ; quand vous ajoutez un chemin ici, ajoutez-le au
# classificateur aussi ». C'était une consigne sans garde. Celle-ci l'arme.
#
# LE PRÉDICAT PORTE SUR LA LISTE, JAMAIS SUR LE LITTÉRAL ISOLÉ — et ce n'est pas
# un assouplissement, c'est la correction d'un premier jet mesuré faux. Un scan
# « un littéral d'échafaudage en position de motif de `case` » accuse
# `_extract_plan_path` (`docs/plans/*)`, dispatch-lib.sh), qui ne déclare pas
# l'échafaudage : il normalise le chemin porté par le callout d'une issue, et son
# second motif `*/docs/plans/*` est la forme PRÉFIXÉE PAR LE DÉPÔT, que le Tier 2
# ne remet justement pas à HEAD. Le router vers `_is_scaffold_path` élargirait la
# liste du Tier 3 à un chemin jamais restauré : très exactement la perte
# silencieuse que R5 refuse. Ce n'est donc pas une exception à allowlister, c'est
# une autre notion — et la notion surveillée ici est l'ENSEMBLE des quatre
# chemins. Un bloc `case` qui en porte DEUX OU PLUS redéclare la liste.
#
# BORNE NOMMÉE, pas découverte (modèle T12j) : un site isolé portant UN SEUL de
# ces chemins pour une raison qui lui est propre est hors population. Le jour où
# quelqu'un scinde la liste en plusieurs `case` d'un motif chacun, ce scan ne le
# voit pas — on élargit alors le prédicat ici, dans le même commit.
#
# Rend : « introuvable », ou le nombre de blocs `case` redéclarant la liste.
_t2144_scaffold_decl_scan() {
    local file="$1" body
    [ -r "$file" ] || { printf 'introuvable'; return 0; }
    # Le corps du site unique est retiré AVANT le comptage : c'est lui la
    # déclaration légitime, et le scan cherche ses concurrents.
    body=$(sed '/^_is_scaffold_path()/,/^}/d' "$file" | grep -v '^[[:space:]]*#' || true)
    printf '%s\n' "$body" | awk '
        BEGIN { inblk = 0; viol = 0 }
        function flush(   k, n) {
            n = 0
            for (k in seen) n++
            if (n >= 2) viol++
            delete seen
        }
        {
            if ($0 ~ /(^|[[:space:];&|(])case[[:space:]]/) inblk = 1
            if (inblk) {
                if ($0 ~ /^[[:space:]]*\.claude\/groom-verdict-trail\.log[^[:space:]]*\)/) seen["trail"] = 1
                if ($0 ~ /^[[:space:]]*\.claude\/commands\/[^[:space:]]*\)/)               seen["commands"] = 1
                if ($0 ~ /^[[:space:]]*\.iterate\/[^[:space:]]*\)/)                        seen["iterate"] = 1
                if ($0 ~ /^[[:space:]]*docs\/plans\/[^[:space:]]*\)/)                      seen["plans"] = 1
            }
            if ($0 ~ /(^|[[:space:];&|])esac([[:space:]]|;|$)/) { flush(); inblk = 0 }
        }
        END { if (inblk) flush(); print viol + 0 }
    '
}

T2144_DECL_SCAN=$(_t2144_scaffold_decl_scan "$DISPATCH_LIB")
assert_eq "mika#2144: dispatch-lib.sh lisible par le scan de déclaration" "lu" \
    "$([ "$T2144_DECL_SCAN" != introuvable ] && echo lu || echo "fichier introuvable: $DISPATCH_LIB")"
assert_eq "mika#2144: aucun second bloc case ne redéclare la liste d'échafaudage" "0" \
    "$T2144_DECL_SCAN"

# Anti-vacuité (i) : le scan viserait un nom mort si la fonction disparaissait, et
# un scan sur un nom mort se lit exactement comme un arbre propre (mika#2205).
assert_eq "mika#2144 anti-vacuité: _is_scaffold_path existe dans dispatch-lib.sh" "1" \
    "$(grep -c '^_is_scaffold_path()' "$DISPATCH_LIB" || true)"
assert_eq "mika#2144 anti-vacuité: le site unique porte bien les QUATRE motifs" "4" \
    "$(sed -n '/^_is_scaffold_path()/,/^}/p' "$DISPATCH_LIB" \
       | grep -cE '^[[:space:]]*(\.claude/groom-verdict-trail\.log|\.claude/commands/|\.iterate/|docs/plans/)[^[:space:]]*\)' || true)"

# Anti-vacuité (ii) : le prédicat MORD, exercé à chaque exécution plutôt qu'une
# fois à la main. Un `case` portant deux des quatre chemins est compté…
assert_eq "mika#2144 anti-vacuité: un case redéclarant deux chemins est vu" "1" \
    "$(_t2144_scaffold_decl_scan <(printf '%s\n' \
        'f() {' '    case "$p" in' '        .iterate/*) ;;' '        docs/plans/*) ;;' '    esac' '}'))"
# …et un `case` n'en portant qu'un seul ne l'est pas : c'est la borne ci-dessus,
# celle qui laisse `_extract_plan_path` tranquille.
assert_eq "mika#2144 borne: un case portant un seul chemin reste hors population" "0" \
    "$(_t2144_scaffold_decl_scan <(printf '%s\n' \
        'f() {' '    case "$p" in' '        docs/plans/*) ;;' '        */docs/plans/*) ;;' '    esac' '}'))"
# …et un fichier illisible ROUGIT au lieu de rendre zéro violation.
assert_eq "mika#2144 anti-vacuité: un fichier absent rend « introuvable »" "introuvable" \
    "$(_t2144_scaffold_decl_scan /nonexistent/mika-2144.sh)"

# Option (a) de la Fire-Disposition : table d'exceptions livrée VIDE, vacuité
# assertée à l'exécution (modèle T2544_HEADING_READER_ALLOWLIST). Quand le scan
# tire, la résolution est de router le site vers `_is_scaffold_path` ; on n'ajoute
# pas de ligne ici (doctrine mika#2201).
T2144_SCAFFOLD_PATTERN_DECLARATION_ALLOWED=()
assert_eq "mika#2144: table d'exceptions du scan de déclaration — zero entries" "0" \
    "${#T2144_SCAFFOLD_PATTERN_DECLARATION_ALLOWED[@]}"

# Les deux consommateurs lisent le site unique, et c'est ce qui rend la garde
# ci-dessus autre chose qu'une assertion sur du texte mort.
assert_eq "mika#2144: _is_scaffold_path a exactement deux consommateurs" "2" \
    "$(grep -v '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '_is_scaffold_path "' || true)"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
