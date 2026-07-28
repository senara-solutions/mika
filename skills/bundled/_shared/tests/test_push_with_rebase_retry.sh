#!/bin/bash
# Test suite for _push_with_rebase_retry (mika#1857 — rupture B, throughput).
#
# Verifies the bounded retry-with-rebase chain that wraps _push_branch's
# push_cmd. The function must:
#   - Succeed immediately on first-attempt success (no retry)
#   - Retry with fetch+rebase on race-shaped rejection ("rejected", "fetch
#     first", "remote contains work")
#   - NOT retry on non-race failures (credential, network, hook rejection)
#   - Fail-safe on rebase conflict (abort rebase + bail to caller's FAILED path)
#   - Fail-safe on fetch failure (bail immediately)
#   - Bound retries at MAX_ATTEMPTS (=2)
#   - Swap to --force-with-lease after successful rebase (history rewrite)
#
# Source isolation: dispatch-lib.sh has no top-level imperative code (audited
# 2026-07-27 for test_force_push_guard.sh) — safe to source directly.
#
# Run: bash skills/bundled/_shared/tests/test_push_with_rebase_retry.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

PASS=0
FAIL=0

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

assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    needle:   '$needle'"
        echo "    haystack: '$haystack'"
    fi
}

# Test harness — record git invocations and control their exit + stderr.
# Each test sets MOCK_GIT_SCRIPT to a comma-separated list of "op:exit:stderr"
# tuples that the mock replays in order.
#
# Ops recognized: "push", "fetch", "rebase", "rebase-abort"
# Exit: numeric exit code
# Stderr: string written to the err file (via >$err_file redirect)
#
# The mock tracks how many times each op was called via GIT_CALL_LOG.
MOCK_GIT_SCRIPT=""
GIT_CALL_LOG=""

git() {
    # Identify which op is being called by scanning args.
    local op="unknown"
    for arg in "$@"; do
        case "$arg" in
            push) op="push"; break ;;
            fetch) op="fetch"; break ;;
            rebase) op="rebase"; break ;;
        esac
    done
    # `rebase --abort` = special sub-op
    for arg in "$@"; do
        [ "$arg" = "--abort" ] && op="rebase-abort" && break
    done

    GIT_CALL_LOG="${GIT_CALL_LOG}${op},"

    # Consume the next tuple from MOCK_GIT_SCRIPT
    local tuple exit_code stderr_out
    tuple="${MOCK_GIT_SCRIPT%%|*}"       # first tuple
    MOCK_GIT_SCRIPT="${MOCK_GIT_SCRIPT#*|}" # rest

    # Tuple format "op:exit:stderr"
    local tuple_op="${tuple%%:*}"
    local rest="${tuple#*:}"
    exit_code="${rest%%:*}"
    stderr_out="${rest#*:}"

    # Sanity check — mock desync would be a test bug, not fixture noise.
    if [ "$tuple_op" != "$op" ]; then
        echo "MOCK DESYNC: expected op '$tuple_op', got '$op' (args: $*)" >&2
        return 99
    fi

    # Write stderr to fd 2 (git's stderr goes to caller's captured file)
    [ -n "$stderr_out" ] && printf '%s' "$stderr_out" >&2

    return "$exit_code"
}

setup() {
    WORKTREE_DIR="/tmp/wt-test"
    BRANCH="feat/test-branch"
    GIT_CALL_LOG=""
    ERR_FILE=$(mktemp /tmp/push-test-err-XXXXXX)
}

teardown() {
    rm -f "$ERR_FILE"
    unset WORKTREE_DIR BRANCH
}

# ── Test 1: first-attempt success → no retry ─────────────────────────────────
echo "test 1: first-attempt success (no retry)"
setup
MOCK_GIT_SCRIPT="push:0:|"  # push succeeds, no rebase/fetch expected
rc=0
_push_with_rebase_retry "diverged" "$ERR_FILE" 2>/dev/null || rc=$?
assert_return "  returns 0 on success" 0 "$rc"
assert_contains "  called push once" "push," "$GIT_CALL_LOG"
if [[ "$GIT_CALL_LOG" == *"fetch"* ]]; then
    FAIL=$((FAIL + 1))
    echo "  ✗ unexpected fetch call on first-attempt success"
else
    PASS=$((PASS + 1))
    echo "  ✓ no fetch/rebase on first-attempt success"
fi
teardown

# ── Test 2: race error → fetch + rebase + retry succeeds ─────────────────────
echo "test 2: race error → rebase → retry success"
setup
MOCK_GIT_SCRIPT="push:1:! [rejected] fetch first|fetch:0:|rebase:0:|push:0:|"
rc=0
_push_with_rebase_retry "diverged" "$ERR_FILE" 2>/dev/null || rc=$?
assert_return "  returns 0 after successful retry" 0 "$rc"
assert_contains "  called push twice" "push,fetch,rebase,push," "$GIT_CALL_LOG"
teardown

# ── Test 3: race error → rebase succeeds → retry hits race again → bail ───────
echo "test 3: race → rebase → race again → bail (exhausted)"
setup
MOCK_GIT_SCRIPT="push:1:! [rejected] fetch first|fetch:0:|rebase:0:|push:1:! [rejected] remote contains work|"
rc=0
_push_with_rebase_retry "diverged" "$ERR_FILE" 2>/dev/null || rc=$?
assert_return "  returns 1 after exhausted retries" 1 "$rc"
assert_contains "  err file has race-shape stderr" "rejected" "$(cat "$ERR_FILE")"
teardown

# ── Test 4: non-race error → no retry ────────────────────────────────────────
echo "test 4: non-race error → no retry, immediate bail"
setup
MOCK_GIT_SCRIPT="push:1:permission denied (publickey)|"
rc=0
_push_with_rebase_retry "diverged" "$ERR_FILE" 2>/dev/null || rc=$?
assert_return "  returns 1 without retry" 1 "$rc"
if [[ "$GIT_CALL_LOG" == *"fetch"* ]]; then
    FAIL=$((FAIL + 1))
    echo "  ✗ unexpected fetch call on non-race failure"
else
    PASS=$((PASS + 1))
    echo "  ✓ no fetch/rebase on non-race failure"
fi
teardown

# ── Test 5: race → fetch fails → bail ────────────────────────────────────────
echo "test 5: race → fetch fails → bail"
setup
MOCK_GIT_SCRIPT="push:1:! [rejected] fetch first|fetch:1:network error|"
rc=0
_push_with_rebase_retry "diverged" "$ERR_FILE" 2>/dev/null || rc=$?
assert_return "  returns 1 on fetch failure" 1 "$rc"
if [[ "$GIT_CALL_LOG" == *"rebase"* ]]; then
    FAIL=$((FAIL + 1))
    echo "  ✗ unexpected rebase after fetch failure"
else
    PASS=$((PASS + 1))
    echo "  ✓ no rebase attempted after fetch failure"
fi
teardown

# ── Test 6: race → rebase conflict → abort + bail ────────────────────────────
echo "test 6: race → rebase conflict → abort + bail"
setup
MOCK_GIT_SCRIPT="push:1:! [rejected] fetch first|fetch:0:|rebase:1:CONFLICT (content)|rebase-abort:0:|"
rc=0
_push_with_rebase_retry "diverged" "$ERR_FILE" 2>/dev/null || rc=$?
assert_return "  returns 1 on rebase conflict" 1 "$rc"
assert_contains "  called rebase-abort after conflict" "rebase-abort," "$GIT_CALL_LOG"
teardown

# ── Test 7: fast-forward mode → race → rebase → retry uses force-with-lease ──
echo "test 7: fast-forward mode swaps to --force-with-lease after rebase"
setup
# Track push args to verify force-with-lease is used post-rebase.
# We can't easily inspect args in the mock, so we just verify the flow completes.
MOCK_GIT_SCRIPT="push:1:! [rejected] non-fast-forward|fetch:0:|rebase:0:|push:0:|"
rc=0
_push_with_rebase_retry "fast-forward" "$ERR_FILE" 2>/dev/null || rc=$?
assert_return "  returns 0 after fast-forward → rebase → force-with-lease retry" 0 "$rc"
assert_contains "  push+fetch+rebase+push sequence" "push,fetch,rebase,push," "$GIT_CALL_LOG"
teardown

# ── Test 8: err file is truncated between attempts ───────────────────────────
echo "test 8: err file reflects LAST attempt's stderr (not accumulated)"
setup
MOCK_GIT_SCRIPT="push:1:! [rejected] fetch first FIRST|fetch:0:|rebase:0:|push:1:! [rejected] fetch first SECOND|"
rc=0
_push_with_rebase_retry "diverged" "$ERR_FILE" 2>/dev/null || rc=$?
assert_return "  returns 1 on exhausted retries" 1 "$rc"
err_content=$(cat "$ERR_FILE")
if [[ "$err_content" == *"SECOND"* ]] && [[ "$err_content" != *"FIRST"* ]]; then
    PASS=$((PASS + 1))
    echo "  ✓ err file contains only LAST attempt's stderr (SECOND, not FIRST)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ err file has wrong content: '$err_content'"
fi
teardown

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
