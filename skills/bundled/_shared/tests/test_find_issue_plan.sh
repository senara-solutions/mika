#!/bin/bash
# Test suite for dispatch-lib.sh's _find_issue_plan helper (mika#1421, #1617).
#
# Verifies plan-file discovery against the three-tier strategy:
#   (1) Filename embeds issue number: `2026-06-05-001-fix-1407-...-plan.md`
#   (2) Anchored header in first 20 lines: `**Ticket:**`/`**Issue:**`/`ticket:`/`issue:`
#       followed by `mika ... #N`
#   (3) Broad content scan in first 50 lines (mika#1617): bare `#N`, `Closes #N`,
#       YAML `number: N`, etc.
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
echo "Negative (prose-only mention of #N past line 50 must NOT match):"
# Tier 3's 50-line zone is the guard. Prose mentioning #N within 50 lines
# IS a valid tier-3 match (accepted false-positive class per mika#1617).
# This test pushes the reference past line 50 to verify zone enforcement.
{
    echo "# Plan: unrelated cleanup"
    echo ""
    # Pad to push the prose reference past line 50
    for i in $(seq 1 55); do echo "Body padding line $i — discussion of approach."; done
    echo "Sister to mika#1234 but ticket is something else"
    echo ""
    for i in $(seq 1 5); do echo "More padding $i."; done
} > "$TMPROOT/docs/plans/2026-06-04-003-unrelated-cleanup-plan.md"
ISSUE_NUM=1234
RESULT=$(_find_issue_plan || true)
assert_empty "prose '#1234' past line 50 → no match (zone boundary enforced)" "$RESULT"

echo
echo "Negative (quoted **Ticket:** line in BODY past line 50 must NOT match):"
# Regression test for self-test failure: mika#1421's plan QUOTED mika#771's
# `**Ticket:** mika issue#771` header on line 49 (a body quote) to illustrate
# the founding incident. Tier 2's 20-line zone closed this class for anchored
# headers; tier 3's 50-line zone closes it for broad references (mika#1617).
{
    echo "---"
    echo "ticket: mika#5050"  # CANONICAL ticket in YAML frontmatter
    echo "type: fix"
    echo "---"
    echo ""
    echo "# Plan: Some other thing"
    echo ""
    echo "Some preamble text in the header zone."
    echo ""
    # Pad to push the quoted Ticket reference past line 50
    for i in $(seq 1 45); do echo "Body padding line $i — discussion of approach."; done
    # NOW the quoted reference appears in BODY (past line 50)
    echo "Line 55 quotes another plan's header to illustrate a point:"
    echo ""
    echo "**Ticket:** mika issue#6060"  # This is a QUOTE, not the real ticket
    echo ""
    for i in $(seq 1 10); do echo "More body discussion line $i."; done
} > "$TMPROOT/docs/plans/2026-06-04-004-fix-something-with-quoted-example-plan.md"

ISSUE_NUM=6060
RESULT=$(_find_issue_plan || true)
assert_empty "quoted **Ticket:** mika issue#6060 in body (line ~57) → no false-positive match" "$RESULT"

# And verify the canonical issue (5050) still matches via its frontmatter header
ISSUE_NUM=5050
RESULT=$(_find_issue_plan)
assert_eq "canonical YAML 'ticket: mika#5050' in frontmatter (line 2) → matches" \
    "$TMPROOT/docs/plans/2026-06-04-004-fix-something-with-quoted-example-plan.md" \
    "$RESULT"

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
# Tier 3: broad issue-number reference in header zone (first 50 lines)
# (mika#1617, N=5 founding incidents)
# ============================================================================

echo
echo "Tier 3 (bare #N in H1 — no anchored prefix):"
{
    echo "# Fix for (#1617)"
    echo ""
    echo "## Problem"
    echo ""
    echo "Something is broken."
    echo ""
    # Pad to >500 bytes
    for i in $(seq 1 30); do echo "Lorem ipsum dolor sit amet padding line $i."; done
} > "$TMPROOT/docs/plans/2026-06-28-001-fix-console-agent-image-tag-plan.md"
ISSUE_NUM=1617
RESULT=$(_find_issue_plan)
assert_eq "tier 3: bare #1617 in H1 line 1 → matches" \
    "$TMPROOT/docs/plans/2026-06-28-001-fix-console-agent-image-tag-plan.md" \
    "$RESULT"

echo
echo "Tier 3 (YAML 'number: N' without # prefix):"
{
    echo "---"
    echo "number: 1617"
    echo "type: fix"
    echo "---"
    echo ""
    echo "# Some plan title"
    echo ""
    # Pad to >500 bytes
    for i in $(seq 1 30); do echo "Lorem ipsum dolor sit amet padding line $i."; done
} > "$TMPROOT/docs/plans/2026-06-28-002-fix-yaml-number-plan.md"
# Remove the tier-3 fixture that matched above so this one is the only candidate
rm "$TMPROOT/docs/plans/2026-06-28-001-fix-console-agent-image-tag-plan.md"
ISSUE_NUM=1617
RESULT=$(_find_issue_plan)
assert_eq "tier 3: YAML 'number: 1617' in frontmatter → matches" \
    "$TMPROOT/docs/plans/2026-06-28-002-fix-yaml-number-plan.md" \
    "$RESULT"

echo
echo "Tier 3 ('Closes #N' in summary around line 10):"
{
    echo "# Plan: fix something"
    echo ""
    echo "## Problem"
    echo ""
    echo "This is a description of the problem."
    echo ""
    echo "## Solution"
    echo ""
    echo "Closes #7777"
    echo ""
    # Pad to >500 bytes
    for i in $(seq 1 30); do echo "Lorem ipsum dolor sit amet padding line $i."; done
} > "$TMPROOT/docs/plans/2026-06-28-003-fix-closes-ref-plan.md"
ISSUE_NUM=7777
RESULT=$(_find_issue_plan)
assert_eq "tier 3: 'Closes #7777' on line ~9 → matches" \
    "$TMPROOT/docs/plans/2026-06-28-003-fix-closes-ref-plan.md" \
    "$RESULT"

echo
echo "Tier 3 negative (#N only in body past line 50):"
{
    echo "# Plan: unrelated work"
    echo ""
    # 55 lines of padding to push the reference past the 50-line zone
    for i in $(seq 1 55); do echo "Body padding line $i — no issue reference here."; done
    echo ""
    echo "See also #8888 for related context."
    echo ""
    for i in $(seq 1 5); do echo "More padding line $i."; done
} > "$TMPROOT/docs/plans/2026-06-28-004-deep-body-ref-plan.md"
ISSUE_NUM=8888
RESULT=$(_find_issue_plan || true)
assert_empty "tier 3 negative: #8888 on line ~58 (past 50-line zone) → no match" "$RESULT"

echo
echo "Tier priority: tier 1 wins over tier 3:"
# Create a plan with issue number in filename AND bare #N in body
{
    echo "# Plan: fix issue (#9090)"
    echo ""
    for i in $(seq 1 30); do echo "Lorem ipsum dolor sit amet padding line $i."; done
} > "$TMPROOT/docs/plans/2026-06-28-005-fix-9090-something-plan.md"
ISSUE_NUM=9090
RESULT=$(_find_issue_plan)
assert_eq "tier 1 wins: filename-embedded 9090 returned (tier 3 never runs)" \
    "$TMPROOT/docs/plans/2026-06-28-005-fix-9090-something-plan.md" \
    "$RESULT"

echo
echo "Tier priority: tier 2 wins over tier 3:"
# Create a plan with anchored Ticket header in first 20 lines AND bare #N on line 40
{
    echo "# Plan: fix something"
    echo ""
    echo "**Ticket:** mika issue#3030"
    echo ""
    for i in $(seq 1 35); do echo "Body discussion line $i."; done
    echo "Also references #3030 in body."
    for i in $(seq 1 5); do echo "More padding $i."; done
} > "$TMPROOT/docs/plans/2026-06-28-006-fix-tier2-wins-plan.md"
ISSUE_NUM=3030
RESULT=$(_find_issue_plan)
assert_eq "tier 2 wins: anchored **Ticket:** in header returned (tier 3 never runs)" \
    "$TMPROOT/docs/plans/2026-06-28-006-fix-tier2-wins-plan.md" \
    "$RESULT"

# ============================================================================
# Summary
# ============================================================================

echo
echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
