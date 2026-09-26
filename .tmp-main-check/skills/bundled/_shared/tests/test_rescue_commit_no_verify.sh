#!/bin/bash
# Test suite for post-flight rescue-commit hook bypass (mika#1685).
#
# WHY: dispatch-lib's post-flight rescue paths (mika#1282 dirty-worktree,
# mika#1383 trailing-content) call `git commit` to SALVAGE a pilot's work into
# a draft PR when the pilot left the worktree dirty. lefthook runs rust-clippy
# on pre-commit; a single clippy nit the pilot left behind would reject the
# rescue commit and strand 29-turn, $4-cost work as a dead block — the modal
# loop-wedge cause (n=3+ on 2026-06-30). The fix adds `--no-verify` to the
# rescue commits so the salvage always lands; CI surfaces clippy on the draft
# PR at the right layer.
#
# This suite proves the MECHANISM the fix relies on (the rescue blocks are
# inline in run_claude_pilot, not callable in isolation, so we exercise the
# git behavior directly against a real temp repo) plus a static guard that the
# three rescue commit sites still carry --no-verify.
#
# Run: bash skills/bundled/_shared/tests/test_rescue_commit_no_verify.sh
# Expected: all assertions pass, exit 0. No network / cargo / clippy required.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

PASS=0
FAIL=0

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: '$expected'"
        echo "    actual:   '$actual'"
    fi
}

# Build a throwaway git repo with a pre-commit hook that always rejects
# (stands in for lefthook's rust-clippy step failing on a leftover nit).
# Echoes the repo dir on stdout.
make_repo_with_failing_hook() {
    local repo
    repo="$(mktemp -d "${TMPDIR:-/tmp}/mika-1685-test.XXXXXX")"
    git -C "$repo" init -q
    git -C "$repo" config user.email test@example.com
    git -C "$repo" config user.name "Test"
    mkdir -p "$repo/.git/hooks"
    cat > "$repo/.git/hooks/pre-commit" <<'HOOK'
#!/bin/bash
# Simulated clippy rejection — the exact wedge condition.
echo "⛔ rust-clippy: leftover lint" >&2
exit 1
HOOK
    chmod +x "$repo/.git/hooks/pre-commit"
    printf 'content\n' > "$repo/file.txt"
    git -C "$repo" add -A
    printf '%s' "$repo"
}

# ============================================================================
# Test 1: bare `git commit` is REJECTED by the failing hook (the wedge)
# ============================================================================
echo ""
echo "Test: bare git commit rejected by pre-commit hook (reproduces wedge)"
echo "--------------------------------------------------------------------"

REPO="$(make_repo_with_failing_hook)"
rc=0
git -C "$REPO" commit -m "wip: rescue" >/dev/null 2>&1 || rc=$?
assert_eq "bare commit: rejected (non-zero exit)" "1" "$( [ "$rc" -ne 0 ] && echo 1 || echo 0 )"
HEAD_COUNT=$(git -C "$REPO" rev-list --count HEAD 2>/dev/null || echo 0)
assert_eq "bare commit: HEAD did not advance (no commit)" "0" "$HEAD_COUNT"
rm -rf "$REPO"

# ============================================================================
# Test 2 (AC3): `git commit --no-verify` SUCCEEDS despite the failing hook
# ============================================================================
echo ""
echo "Test (AC3): git commit --no-verify rescues despite clippy-equivalent nit"
echo "-----------------------------------------------------------------------"

REPO="$(make_repo_with_failing_hook)"
rc=0
git -C "$REPO" commit -m "wip: rescue" --no-verify >/dev/null 2>&1 || rc=$?
assert_eq "no-verify commit: succeeds (exit 0)" "0" "$rc"
HEAD_COUNT=$(git -C "$REPO" rev-list --count HEAD 2>/dev/null || echo 0)
assert_eq "no-verify commit: HEAD advanced (commit landed)" "1" "$HEAD_COUNT"
rm -rf "$REPO"

# ============================================================================
# Test 3 (AC5): no hook present — --no-verify still rescues cleanly (happy path)
# ============================================================================
echo ""
echo "Test (AC5): clean worktree, no hook — --no-verify still commits"
echo "---------------------------------------------------------------"

REPO="$(mktemp -d "${TMPDIR:-/tmp}/mika-1685-clean.XXXXXX")"
git -C "$REPO" init -q
git -C "$REPO" config user.email test@example.com
git -C "$REPO" config user.name "Test"
printf 'content\n' > "$REPO/file.txt"
git -C "$REPO" add -A
rc=0
git -C "$REPO" commit -m "wip: rescue" --no-verify >/dev/null 2>&1 || rc=$?
assert_eq "happy path: --no-verify commits cleanly (exit 0)" "0" "$rc"
HEAD_COUNT=$(git -C "$REPO" rev-list --count HEAD 2>/dev/null || echo 0)
assert_eq "happy path: HEAD advanced" "1" "$HEAD_COUNT"
rm -rf "$REPO"

# ============================================================================
# Test 4 (AC1): static guard — every rescue-path commit carries --no-verify
# ============================================================================
echo ""
echo "Test (AC1): all rescue git-commit sites in dispatch-lib carry --no-verify"
echo "------------------------------------------------------------------------"

# Each rescue commit is `git -C "$WORKTREE_DIR" commit -m "wip(...`. The flag
# may sit on the same line (mika#1383 inline) or on the message's closing line
# (mika#1282 multi-line -m). Pull each commit invocation plus its following
# lines up to the line that opens the redirect/`; then`, and assert --no-verify
# appears in that window. awk: from a rescue commit opener, accumulate lines
# until one contains '2>&' (the redirect that closes the invocation), then
# check the accumulated block for --no-verify.
MISSING=$(awk '
    /git -C "\$WORKTREE_DIR" commit -m "wip\(/ { inblock=1; block=""; }
    inblock { block = block $0 "\n"; }
    inblock && /2>&/ {
        inblock=0;
        if (block !~ /--no-verify/) bad++;
    }
    END { print bad+0 }
' "$DISPATCH_LIB")
assert_eq "static guard: rescue commits missing --no-verify" "0" "$MISSING"

# Belt-and-suspenders: there are exactly 3 rescue commit invocations today.
RESCUE_COUNT=$(grep -c 'git -C "\$WORKTREE_DIR" commit -m "wip(' "$DISPATCH_LIB")
assert_eq "static guard: rescue commit invocation count" "3" "$RESCUE_COUNT"

# ============================================================================
# Test 5: dispatch-lib still parses (the edits did not break shell syntax)
# ============================================================================
echo ""
echo "Test: dispatch-lib.sh passes bash -n"
echo "------------------------------------"

rc=0
bash -n "$DISPATCH_LIB" 2>/dev/null || rc=$?
assert_eq "dispatch-lib: bash -n clean" "0" "$rc"

# ============================================================================
# Summary
# ============================================================================
echo ""
echo "=============================="
echo "Results: $PASS passed, $FAIL failed"
echo "=============================="

[ "$FAIL" -eq 0 ] || exit 1
