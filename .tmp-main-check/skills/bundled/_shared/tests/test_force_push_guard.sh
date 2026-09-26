#!/bin/bash
# Test suite for _check_pilot_force_push (mika#1318).
#
# Tests the post-flight pilot push guard that detects scope-of-authority
# violations when a dev-groom pilot pushes to the remote.
#
# Source isolation audit: dispatch-lib.sh has no top-level imperative code —
# all `set -e`, `trap`, and env var references are inside function bodies.
# Safe to source directly without a guard variable.
#
# Run: bash skills/bundled/_shared/tests/test_force_push_guard.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

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

assert_return() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label (exit=$actual)"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected exit: $expected"
        echo "    actual exit:   $actual"
    fi
}

# Override git to simulate ls-remote responses without network access.
# Tests set MOCK_LS_REMOTE_OUTPUT to control what the mock returns.
MOCK_LS_REMOTE_OUTPUT=""
MOCK_LS_REMOTE_EXIT=0
git() {
    # Intercept: git -C <dir> ls-remote origin refs/heads/<branch>
    # Positional: $1=-C, $2=<dir>, $3=ls-remote
    local is_ls_remote=0
    for arg in "$@"; do
        if [ "$arg" = "ls-remote" ]; then
            is_ls_remote=1
            break
        fi
    done
    if [ "$is_ls_remote" = "1" ]; then
        if [ "$MOCK_LS_REMOTE_EXIT" -ne 0 ]; then
            return "$MOCK_LS_REMOTE_EXIT"
        fi
        printf '%s' "$MOCK_LS_REMOTE_OUTPUT"
        return 0
    fi
    # Fall through to real git for anything else
    command git "$@"
}

# ============================================================================
# Test 1: dev-groom, no pilot push (clean)
# ============================================================================
echo ""
echo "Test: dev-groom, no pilot push — guard returns 0"
echo "---------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD="abc123"
MOCK_LS_REMOTE_OUTPUT="abc123	refs/heads/$BRANCH"
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""

rc=0
_check_pilot_force_push 2>/dev/null || rc=$?
assert_return "clean: returns 0" "0" "$rc"
assert_eq "clean: no violation flag" "" "$PUSH_VIOLATION_DETECTED"

# ============================================================================
# Test 2: dev-groom, pilot pushed (violation)
# ============================================================================
echo ""
echo "Test: dev-groom, pilot pushed — guard returns 1"
echo "---------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD="abc123"
MOCK_LS_REMOTE_OUTPUT="def456	refs/heads/$BRANCH"
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""

rc=0
_check_pilot_force_push 2>/dev/null || rc=$?
assert_return "violation: returns 1" "1" "$rc"
assert_eq "violation: flag set" "1" "$PUSH_VIOLATION_DETECTED"
assert_eq "violation: evidence captured" "pre_remote=abc123 post_remote=def456" "$PUSH_VIOLATION_EVIDENCE"

# ============================================================================
# Test 3: dev-pilot — guard returns 0 regardless of remote state (R5)
# ============================================================================
echo ""
echo "Test: dev-pilot — guard early-returns 0 (R5)"
echo "---------------------------------------------------"

SKILL="dev-pilot"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD="abc123"
MOCK_LS_REMOTE_OUTPUT="def456	refs/heads/$BRANCH"
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""

rc=0
_check_pilot_force_push 2>/dev/null || rc=$?
assert_return "dev-pilot: returns 0" "0" "$rc"
assert_eq "dev-pilot: no violation flag" "" "$PUSH_VIOLATION_DETECTED"

# ============================================================================
# Test 4: no worktree (free-text mode) — guard returns 0
# ============================================================================
echo ""
echo "Test: no worktree — guard returns 0"
echo "---------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR=""
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD=""
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""

rc=0
_check_pilot_force_push 2>/dev/null || rc=$?
assert_return "no-worktree: returns 0" "0" "$rc"

# ============================================================================
# Test 5: branch not on remote pre-run, still not on remote post-run (clean)
# ============================================================================
echo ""
echo "Test: branch not on remote (both empty) — guard returns 0"
echo "-----------------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/new-branch"
PRE_RUN_REMOTE_HEAD=""
MOCK_LS_REMOTE_OUTPUT=""
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""

rc=0
_check_pilot_force_push 2>/dev/null || rc=$?
assert_return "new-branch-clean: returns 0" "0" "$rc"

# ============================================================================
# Test 6: branch not on remote pre-run, appeared post-run (pilot created it)
# ============================================================================
echo ""
echo "Test: branch appeared on remote — violation detected"
echo "------------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/new-branch"
PRE_RUN_REMOTE_HEAD=""
MOCK_LS_REMOTE_OUTPUT="def456	refs/heads/$BRANCH"
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""

rc=0
_check_pilot_force_push 2>/dev/null || rc=$?
assert_return "new-branch-violation: returns 1" "1" "$rc"
assert_eq "new-branch-violation: flag set" "1" "$PUSH_VIOLATION_DETECTED"
assert_eq "new-branch-violation: evidence" "pre_remote=<none> post_remote=def456" "$PUSH_VIOLATION_EVIDENCE"

# ============================================================================
# Test 7: network failure on ls-remote — guard returns 0 (fail-open)
# ============================================================================
echo ""
echo "Test: network failure — guard returns 0 (fail-open)"
echo "------------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD="abc123"
MOCK_LS_REMOTE_EXIT=128
MOCK_LS_REMOTE_OUTPUT=""
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""

rc=0
_check_pilot_force_push 2>/dev/null || rc=$?
assert_return "network-failure: returns 0" "0" "$rc"
assert_eq "network-failure: no violation flag" "" "$PUSH_VIOLATION_DETECTED"

# Reset mock
MOCK_LS_REMOTE_EXIT=0

# ============================================================================
# Summary
# ============================================================================
echo ""
echo "=============================="
echo "Results: $PASS passed, $FAIL failed"
echo "=============================="

[ "$FAIL" -eq 0 ] || exit 1
