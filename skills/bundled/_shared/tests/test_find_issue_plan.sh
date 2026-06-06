#!/bin/bash
# Test suite for dispatch-lib.sh's _find_issue_plan helper (mika#1421).
#
# Verifies plan-file discovery against the two filename conventions the
# pilot has been observed to produce:
#   (1) Filename embeds issue number: `2026-06-05-001-fix-1407-...-plan.md`
#   (2) Filename omits issue number: `2026-06-06-003-feat-post-condition-...-plan.md`
#       (the pilot writes this when its slug-tail doesn't naturally include
#       the issue number; content header references `**Ticket:** mika issue#N`)
#
# Founding incident: mika#1421 bound the architect-convergence class at n=2
# on 2026-06-06. The brittle filename-only pattern in three callsites caused
# `_iterate_groom_loop` to return non-zero with the plan committed but the
# architect never called.
#
# Run: bash skills/bundled/_shared/tests/test_find_issue_plan.sh
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

assert_empty() {
    local label="$1" actual="$2"
    if [ -z "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label (expected empty; got '$actual')"
    fi
}

# Set up a temp worktree directory for fixtures
TMPROOT=$(mktemp -d)
trap 'rm -rf "$TMPROOT"' EXIT
mkdir -p "$TMPROOT/docs/plans"

# Helper: write a plan file >500 bytes
write_plan() {
    local path="$1" ticket_line="${2:-}"
    {
        if [ -n "$ticket_line" ]; then
            echo "# Plan: example"
            echo ""
            echo "$ticket_line"
            echo ""
        fi
        # Pad to >500 bytes
        for i in $(seq 1 30); do echo "Lorem ipsum dolor sit amet padding line $i."; done
    } > "$path"
}

WORKTREE_DIR="$TMPROOT"

# ============================================================================
# Primary: filename embeds issue number
# ============================================================================

echo "Primary (filename-embedded issue number):"

write_plan "$TMPROOT/docs/plans/2026-06-05-001-fix-1407-pilot-push-diagnosis-plan.md" \
    "**Ticket:** mika#1407"
ISSUE_NUM=1407
RESULT=$(_find_issue_plan)
assert_eq "filename embeds 1407 → matches" \
    "$TMPROOT/docs/plans/2026-06-05-001-fix-1407-pilot-push-diagnosis-plan.md" \
    "$RESULT"

# ============================================================================
# Fallback: filename omits issue number, content has the Ticket reference
# ============================================================================

echo
echo "Fallback (content-grep for **Ticket:** mika issue#N — current pilot shape):"

# This is the EXACT filename mika#771 wrote on 2026-06-06 17:22Z
write_plan "$TMPROOT/docs/plans/2026-06-06-003-feat-post-condition-guard-send-message-plan.md" \
    "**Ticket:** mika issue#771"
ISSUE_NUM=771
RESULT=$(_find_issue_plan)
assert_eq "mika#771 founding incident — filename without issue number, content matches" \
    "$TMPROOT/docs/plans/2026-06-06-003-feat-post-condition-guard-send-message-plan.md" \
    "$RESULT"

echo
echo "Fallback (older **Ticket:** mika#N shape):"
write_plan "$TMPROOT/docs/plans/2026-06-04-001-fix-something-cleanup-plan.md" \
    "**Ticket:** mika#999"
ISSUE_NUM=999
RESULT=$(_find_issue_plan)
assert_eq "older Ticket shape (mika#999 — no 'issue' word) matches" \
    "$TMPROOT/docs/plans/2026-06-04-001-fix-something-cleanup-plan.md" \
    "$RESULT"

echo
echo "Fallback (YAML frontmatter 'ticket:' shape):"
write_plan "$TMPROOT/docs/plans/2026-06-04-002-feat-something-else-plan.md" \
    "ticket: mika#888"
ISSUE_NUM=888
RESULT=$(_find_issue_plan)
assert_eq "YAML frontmatter ticket: mika#N matches" \
    "$TMPROOT/docs/plans/2026-06-04-002-feat-something-else-plan.md" \
    "$RESULT"

# ============================================================================
# Negative: prose mentioning #N is NOT enough (anchor discipline)
# ============================================================================

echo
echo "Negative (prose-only mention of #N must NOT match):"
write_plan "$TMPROOT/docs/plans/2026-06-04-003-unrelated-cleanup-plan.md" \
    "Sister to mika#1234 but ticket is something else"
ISSUE_NUM=1234
RESULT=$(_find_issue_plan || true)
assert_empty "prose '#1234' in line 3 (not anchored as **Ticket:**) → no match" "$RESULT"

# ============================================================================
# Most-recent-wins when both shapes exist for the same issue
# ============================================================================

echo
echo "Sort order (sort -r over found candidates):"
# Add a second plan with later date for the same issue (#1407) — content shape
write_plan "$TMPROOT/docs/plans/2026-06-07-001-revised-fix-plan.md" \
    "**Ticket:** mika issue#1407"
ISSUE_NUM=1407
RESULT=$(_find_issue_plan)
# Primary pattern matches the 2026-06-05 file (has "-1407-" in name) BEFORE
# the fallback runs. Primary wins; the 2026-06-05 file is returned even
# though 2026-06-07 sorts higher in the fallback path.
assert_eq "primary pattern wins over fallback (returns 2026-06-05 not 2026-06-07)" \
    "$TMPROOT/docs/plans/2026-06-05-001-fix-1407-pilot-push-diagnosis-plan.md" \
    "$RESULT"

# Remove the primary-matching file; rerun. Now only fallback applies.
rm "$TMPROOT/docs/plans/2026-06-05-001-fix-1407-pilot-push-diagnosis-plan.md"
RESULT=$(_find_issue_plan)
assert_eq "fallback-only: most-recent match wins (2026-06-07 returned)" \
    "$TMPROOT/docs/plans/2026-06-07-001-revised-fix-plan.md" \
    "$RESULT"

# ============================================================================
# Guard: unset / missing inputs
# ============================================================================

echo
echo "Input guards:"
ISSUE_NUM=""
RESULT=$(_find_issue_plan 2>/dev/null || true)
assert_empty "ISSUE_NUM unset → returns empty" "$RESULT"
ISSUE_NUM=1407

WORKTREE_DIR_SAVED="$WORKTREE_DIR"
WORKTREE_DIR=""
RESULT=$(_find_issue_plan 2>/dev/null || true)
assert_empty "WORKTREE_DIR unset → returns empty" "$RESULT"
WORKTREE_DIR="$WORKTREE_DIR_SAVED"

# Non-existent docs/plans directory
WORKTREE_DIR="$TMPROOT/nonexistent"
ISSUE_NUM=2222
RESULT=$(_find_issue_plan 2>/dev/null || true)
assert_empty "docs/plans absent → returns empty (no crash)" "$RESULT"
WORKTREE_DIR="$WORKTREE_DIR_SAVED"

# Plan under 500 bytes is excluded
echo "small content for #5555" > "$TMPROOT/docs/plans/2026-06-04-small-plan.md"
echo "**Ticket:** mika issue#5555" >> "$TMPROOT/docs/plans/2026-06-04-small-plan.md"
ISSUE_NUM=5555
RESULT=$(_find_issue_plan 2>/dev/null || true)
assert_empty "<500-byte plan filtered by size guard" "$RESULT"

# ============================================================================
# Summary
# ============================================================================

echo
echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
