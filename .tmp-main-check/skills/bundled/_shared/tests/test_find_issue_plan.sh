#!/bin/bash
# Test suite for dispatch-lib.sh's _find_issue_plan helper (mika#1421, mika#1617).
#
# Verifies three-tier plan-file discovery:
#   (1) Tier 1: Filename embeds issue number: `2026-06-05-001-fix-1407-...-plan.md`
#   (2) Tier 2: Filename omits issue number, content header references
#       `**Ticket:** mika issue#N` / `**Issue:** mika#N` / YAML `ticket:`/`issue:`
#   (3) Tier 3: Broad issue-number reference in first 50 lines — bare `#N`,
#       `Closes #N`, YAML `number: N`, etc. (mika#1617)
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
# Prose mention of #N is not enough for ANY tier when outside all header zones.
# Tier 2 checks first 20 lines, tier 3 checks first 50 lines.
{
    echo "# Plan: unrelated cleanup"
    echo ""
    for i in $(seq 1 52); do echo "Body padding line $i — approach discussion."; done
    echo "Sister to mika#1234 but ticket is something else"
    for i in $(seq 1 5); do echo "More body line $i."; done
} > "$TMPROOT/docs/plans/2026-06-04-003-unrelated-cleanup-plan.md"
ISSUE_NUM=1234
RESULT=$(_find_issue_plan || true)
assert_empty "prose '#1234' past line 50 → no match (outside all tier zones)" "$RESULT"

echo
echo "Negative (quoted **Ticket:** line in BODY prose past line 50 must NOT match):"
# Regression test for self-test failure: mika#1421's plan QUOTED mika#771's
# `**Ticket:** mika issue#771` header on line 49 (a body quote) to illustrate
# the founding incident. The v1 helper false-positive'd on ISSUE_NUM=771 and
# returned the #1421 plan instead of the actual #771 plan. The header-zone
# scope (first 20 lines for tier 2, first 50 lines for tier 3) closes this
# class. The quoted reference must be past line 50 to avoid tier-3 match.
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
    # Pad to push the quoted Ticket reference past line 50 (tier 3 zone)
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
assert_empty "quoted **Ticket:** mika issue#6060 in body (line ~55) → no false-positive match" "$RESULT"

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
# Tier 3: broad issue-number reference in header zone (mika#1617)
# ============================================================================

echo
echo "Tier 3 (broad issue-number reference in header zone, first 50 lines):"

# Positive: bare #N in H1 (line 1–5)
{
    echo "# Fix for (#1617)"
    echo ""
    echo "Some summary text without a Ticket: or Issue: prefix."
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$TMPROOT/docs/plans/2026-06-28-001-fix-console-agent-image-tag-plan.md"
ISSUE_NUM=1617
RESULT=$(_find_issue_plan)
assert_eq "tier 3: bare #N in H1 → matches" \
    "$TMPROOT/docs/plans/2026-06-28-001-fix-console-agent-image-tag-plan.md" \
    "$RESULT"

echo
echo "Tier 3 positive: YAML number: N without #:"
{
    echo "---"
    echo "number: 3030"
    echo "type: fix"
    echo "---"
    echo ""
    echo "# Some plan title"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$TMPROOT/docs/plans/2026-06-28-002-yaml-number-shape-plan.md"
ISSUE_NUM=3030
RESULT=$(_find_issue_plan)
assert_eq "tier 3: YAML 'number: N' without # → matches" \
    "$TMPROOT/docs/plans/2026-06-28-002-yaml-number-shape-plan.md" \
    "$RESULT"

echo
echo "Tier 3 positive: Closes #N in summary (~line 10):"
{
    echo "# Plan: fix something"
    echo ""
    echo "## Problem"
    echo ""
    echo "Something is broken."
    echo ""
    echo "## Solution"
    echo ""
    echo "Closes #4040 by fixing the root cause."
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$TMPROOT/docs/plans/2026-06-28-003-closes-ref-plan.md"
ISSUE_NUM=4040
RESULT=$(_find_issue_plan)
assert_eq "tier 3: 'Closes #N' in summary → matches" \
    "$TMPROOT/docs/plans/2026-06-28-003-closes-ref-plan.md" \
    "$RESULT"

echo
echo "Tier 3 negative: #N only past line 50:"
{
    echo "# Plan: unrelated topic"
    echo ""
    for i in $(seq 1 55); do echo "Filler line $i with no issue reference."; done
    echo "This references #7070 but it is past line 50."
    echo ""
    for i in $(seq 1 5); do echo "More filler line $i."; done
} > "$TMPROOT/docs/plans/2026-06-28-004-deep-body-ref-plan.md"
ISSUE_NUM=7070
RESULT=$(_find_issue_plan 2>/dev/null || true)
assert_empty "tier 3: #N only past line 50 → no match (zone boundary)" "$RESULT"

echo
echo "Tier 3 priority: tier 1 wins over tier 3:"
# Create a plan with issue number in filename AND bare #N in body
{
    echo "# Plan: something about (#8080)"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$TMPROOT/docs/plans/2026-06-28-005-fix-8080-something-plan.md"
ISSUE_NUM=8080
RESULT=$(_find_issue_plan)
assert_eq "tier 1 wins over tier 3 (filename match returned)" \
    "$TMPROOT/docs/plans/2026-06-28-005-fix-8080-something-plan.md" \
    "$RESULT"

echo
echo "Tier 3 priority: tier 2 wins over tier 3:"
# Create a plan with **Ticket:** in first 20 lines AND bare #N on line 40
{
    echo "# Plan: something"
    echo ""
    echo "**Ticket:** mika issue#9090"
    echo ""
    for i in $(seq 1 40); do echo "Body padding line $i for size."; done
    echo "Also references #9090 here on line ~45."
} > "$TMPROOT/docs/plans/2026-06-28-006-tier2-wins-plan.md"
ISSUE_NUM=9090
RESULT=$(_find_issue_plan)
assert_eq "tier 2 wins over tier 3 (anchored header match returned)" \
    "$TMPROOT/docs/plans/2026-06-28-006-tier2-wins-plan.md" \
    "$RESULT"

# ============================================================================
# mika#2038: tier-1 header refutation
#
# Tier 1 globs `*-${ISSUE_NUM}-*-plan.md` and used to return the first hit.
# For ISSUE_NUM=2026 that pattern matches `rustsec-2026-0097` — a RustSec
# advisory year, not an issue number — so a pilot dispatched for mika#2026
# was launched on an April plan about bumping `rand`.
#
# The fix keeps the glob permissive (only 255 of 745 real plans honour the
# `<date>-<NNN>-<type>-<issue>-` slot, so an exclusive positional filter
# would reopen the false-negative class of mika#1421/#1602/#1617) and
# instead refutes a candidate whose header names a DIFFERENT issue.
# Silence is not refutation: a plan with no ticket header stays eligible.
# ============================================================================

# Each case below gets its own root — the shared TMPROOT above accumulates
# fixtures from earlier sections that would pollute a glob for these numbers.
fresh_root() {
    local d
    d=$(mktemp -d "$TMPROOT/case-XXXXXX")
    mkdir -p "$d/docs/plans"
    printf '%s' "$d"
}

echo
echo "mika#2038 — tier-1 refutation (helper: _plan_header_refutes_issue):"

HELPER_ROOT=$(fresh_root)

# The founding incident's header, verbatim: no `mika` prefix, bare `#N`.
{
    echo "# Plan: Bump rand to 0.9.3+ to clear RUSTSEC-2026-0097"
    echo ""
    echo "**Issue:** #539"
    echo "**Type:** chore (dependency maintenance)"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$HELPER_ROOT/docs/plans/rustsec-header.md"

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/rustsec-header.md" 2026; then
    PASS=$((PASS + 1)); echo "  ✓ '**Issue:** #539' refutes target 2026 (founding incident header)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ '**Issue:** #539' should refute target 2026"
fi

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/rustsec-header.md" 539; then
    FAIL=$((FAIL + 1)); echo "  ✗ '**Issue:** #539' must NOT refute its own target 539"
else
    PASS=$((PASS + 1)); echo "  ✓ '**Issue:** #539' does not refute target 539"
fi

{
    echo "---"
    echo "issue: 1679"
    echo "type: fix"
    echo "---"
    echo ""
    echo "# Plan — fix(dispatch-lib): mika#1383 recovery guards (mika#1679)"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$HELPER_ROOT/docs/plans/yaml-issue-header.md"

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/yaml-issue-header.md" 1383; then
    PASS=$((PASS + 1)); echo "  ✓ YAML 'issue: 1679' refutes target 1383 (slug cites another ticket)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ YAML 'issue: 1679' should refute target 1383"
fi

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/yaml-issue-header.md" 1679; then
    FAIL=$((FAIL + 1)); echo "  ✗ YAML 'issue: 1679' must NOT refute target 1679"
else
    PASS=$((PASS + 1)); echo "  ✓ YAML 'issue: 1679' does not refute target 1679"
fi

# KTD2: silence is not refutation. 13% of the real corpus carries no marker.
{
    for i in $(seq 1 30); do echo "Body padding line $i, no ticket header at all."; done
} > "$HELPER_ROOT/docs/plans/no-header.md"

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/no-header.md" 2026; then
    FAIL=$((FAIL + 1)); echo "  ✗ a header-less plan must NOT be refuted (KTD2)"
else
    PASS=$((PASS + 1)); echo "  ✓ header-less plan is not refuted (silence is not refutation)"
fi

# Multiple claims in the header zone: refute only when NONE of them matches.
{
    echo "# Plan: something"
    echo ""
    echo "- **Ticket:** mika issue#2038"
    echo "- **Issue:** #2026"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$HELPER_ROOT/docs/plans/two-claims.md"

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/two-claims.md" 2038; then
    FAIL=$((FAIL + 1)); echo "  ✗ header naming the target among several must NOT refute"
else
    PASS=$((PASS + 1)); echo "  ✓ header naming the target among several claims does not refute"
fi

# Word boundary: 160 must not be satisfied by #1600 (mika#1602 discipline).
{
    echo "# Plan: something"
    echo ""
    echo "**Ticket:** mika#1600"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$HELPER_ROOT/docs/plans/prefix-collision.md"

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/prefix-collision.md" 160; then
    PASS=$((PASS + 1)); echo "  ✓ '#1600' refutes target 160 (no prefix collision)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ '#1600' should refute target 160"
fi

# Header zone is the first 20 lines, same as tier 2.
{
    echo "# Plan: something"
    for i in $(seq 1 40); do echo "Body padding line $i for size."; done
    echo "**Ticket:** mika#5150"
} > "$HELPER_ROOT/docs/plans/deep-claim.md"
# A frontmatter key that merely ENDS in a refuting label is not a claim.
# Found by the mika#2038 corpus diff: an unanchored `id` matched inside
# `groom_session_id: 557a7808-…` and refuted mika#1469's own plan.
{
    echo "---"
    echo "title: \"fix(engine): webhook_zero_tools guard prefix-narrowing\""
    echo "type: fix"
    echo "origin: GitHub issue mika#1469"
    echo "groom_session_id: 557a7808-17f2-4f7e-bcfe-25e8df3021d9"
    echo "---"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$HELPER_ROOT/docs/plans/session-id-frontmatter.md"

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/session-id-frontmatter.md" 1469; then
    FAIL=$((FAIL + 1)); echo "  ✗ 'groom_session_id:' must NOT be read as an issue claim"
else
    PASS=$((PASS + 1)); echo "  ✓ 'groom_session_id: 557a…' is not an issue claim (label is anchored)"
fi

# A cross-reference is not an ownership claim. `Related issue: #456` names an
# issue the plan RELATES to; reading it as ownership refutes the plan for its
# own ticket and drops it out of tier 1.
{
    echo "# Plan — mika#852 follow-up"
    echo ""
    echo "Related issue: #456"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$HELPER_ROOT/docs/plans/cross-reference.md"

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/cross-reference.md" 852; then
    FAIL=$((FAIL + 1)); echo "  ✗ 'Related issue: #456' must NOT be read as an ownership claim"
else
    PASS=$((PASS + 1)); echo "  ✓ 'Related issue: #456' is a cross-reference, not a claim"
fi

# Prose that happens to contain a label word is not a claim either.
{
    echo "# Plan: something"
    echo ""
    echo "The issue: 3 phases remain before this lands."
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$HELPER_ROOT/docs/plans/prose-label.md"

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/prose-label.md" 852; then
    FAIL=$((FAIL + 1)); echo "  ✗ prose 'The issue: 3 phases' must NOT be read as a claim"
else
    PASS=$((PASS + 1)); echo "  ✓ prose 'The issue: 3 phases' is not a claim"
fi

# Two issue numbers in one label line: both are claims, not just the last.
{
    echo "# Plan: something"
    echo ""
    echo "**Ticket:** mika#1772/#1773"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$HELPER_ROOT/docs/plans/two-numbers-one-line.md"

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/two-numbers-one-line.md" 1772; then
    FAIL=$((FAIL + 1)); echo "  ✗ 'mika#1772/#1773' must claim BOTH numbers, not only the last"
else
    PASS=$((PASS + 1)); echo "  ✓ 'mika#1772/#1773' claims both numbers"
fi

if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/two-numbers-one-line.md" 1773; then
    FAIL=$((FAIL + 1)); echo "  ✗ 'mika#1772/#1773' must not refute its own second number"
else
    PASS=$((PASS + 1)); echo "  ✓ 'mika#1772/#1773' does not refute 1773 either"
fi


if _plan_header_refutes_issue "$HELPER_ROOT/docs/plans/deep-claim.md" 2026; then
    FAIL=$((FAIL + 1)); echo "  ✗ a claim past line 20 must NOT refute (header-zone scope)"
else
    PASS=$((PASS + 1)); echo "  ✓ claim past line 20 does not refute (header-zone scope)"
fi

echo
echo "mika#2038 — tier 1 discards the RustSec false positive:"

CASE_ROOT=$(fresh_root)
RUSTSEC_PLAN="$CASE_ROOT/docs/plans/2026-04-11-003-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md"
{
    echo "# Plan: Bump rand to 0.9.3+ to clear RUSTSEC-2026-0097"
    echo ""
    echo "**Issue:** #539"
    echo "**Type:** chore (dependency maintenance)"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$RUSTSEC_PLAN"

WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=2026
RESULT=$(_find_issue_plan 2>/dev/null || true)
assert_empty "the April rustsec-2026-0097 plan is not returned for #2026" "$RESULT"

# Tier 3 is exercised for real below: a body that cites the target number is
# the shape the real corpus has, and the shape that made the first draft of
# this fix return a foreign plan for #2026.
echo
echo "mika#2038 — the correct plan still wins when both are present:"
CORRECT_PLAN="$CASE_ROOT/docs/plans/2026-08-29-004-obs-loop-pr-origin-plan.md"
{
    echo "# Plan: PR origin is not measurable"
    echo ""
    echo "**Ticket:** mika issue#2026"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$CORRECT_PLAN"

RESULT=$(_find_issue_plan 2>/dev/null)
assert_eq "correct #2026 plan is found via tier 2 once tier 1 refutes the April one" \
    "$CORRECT_PLAN" "$RESULT"

echo
echo "mika#2038 — a refuted candidate is not handed back by tier 3:"

# Every fixture above pads its body with text that never mentions the target,
# so tier 3 structurally cannot fire and a green "not returned" assertion
# proves nothing about it. Real plans cite other tickets in their prose all
# the time — this plan's own Problem Frame names mika#2026 nine times. The
# refutation has to hold at the tier that reads bodies, not just at tier 1.
CASE_ROOT=$(fresh_root)
{
    echo "# Plan: Bump rand to 0.9.3+ to clear RUSTSEC-2026-0097"
    echo ""
    echo "**Issue:** #539"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$CASE_ROOT/docs/plans/2026-04-11-003-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md"
{
    echo "# Plan: an unrelated ticket that merely discusses mika#2026"
    echo ""
    echo "**Ticket:** mika issue#2038"
    echo ""
    echo "A pilot dispatched for mika#2026 was launched on the wrong plan."
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$CASE_ROOT/docs/plans/2026-08-29-002-fix-2038-tier1-refutation-plan.md"

WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=2026
RESULT=$(_find_issue_plan 2>/dev/null || true)
assert_empty "no plan is returned for #2026 when every candidate belongs to another issue" "$RESULT"

echo
echo "mika#2038 — off-slot filenames still resolve at tier 1 (no false negatives):"

# 66% of the real corpus does not honour the <date>-<NNN>-<type>-<issue>- slot.
CASE_ROOT=$(fresh_root)
OFFSLOT_PLAN="$CASE_ROOT/docs/plans/2026-05-19-feat-1150-send-message-guard-cohort-F2-plan.md"
{
    echo "# Plan: post-crash send_message guard (mika#1150 F2)"
    echo ""
    echo "ticket: mika#1150 (F2 + partial F3 only)"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$OFFSLOT_PLAN"

WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=1150
RESULT=$(_find_issue_plan 2>/dev/null)
assert_eq "off-slot filename (no NNN counter) still returned" "$OFFSLOT_PLAN" "$RESULT"

CASE_ROOT=$(fresh_root)
MIKA_PREFIX_PLAN="$CASE_ROOT/docs/plans/2026-06-10-001-fix-mika-1475-deploy-info-off-main-abort-plan.md"
{
    echo "# Plan: deploy info off-main abort"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$MIKA_PREFIX_PLAN"

WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=1475
RESULT=$(_find_issue_plan 2>/dev/null)
assert_eq "header-less, 'mika-' prefixed filename still returned (KTD2 + KTD1)" \
    "$MIKA_PREFIX_PLAN" "$RESULT"

echo
echo "mika#2038 — slot position ranks survivors (KTD4):"

CASE_ROOT=$(fresh_root)
IN_SLOT="$CASE_ROOT/docs/plans/2026-07-01-002-fix-3300-in-slot-plan.md"
OFF_SLOT="$CASE_ROOT/docs/plans/2026-07-02-001-fix-rustsec-3300-0097-plan.md"
for f in "$IN_SLOT" "$OFF_SLOT"; do
    { echo "# Plan: candidate"; echo ""
      for i in $(seq 1 30); do echo "Body padding line $i for size."; done; } > "$f"
done

WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=3300
RESULT=$(_find_issue_plan 2>/dev/null)
assert_eq "in-slot candidate beats the newer off-slot one" "$IN_SLOT" "$RESULT"

CASE_ROOT=$(fresh_root)
OLDER="$CASE_ROOT/docs/plans/2026-07-01-001-fix-3400-older-plan.md"
NEWER="$CASE_ROOT/docs/plans/2026-07-09-001-fix-3400-newer-plan.md"
for f in "$OLDER" "$NEWER"; do
    { echo "# Plan: candidate"; echo ""
      for i in $(seq 1 30); do echo "Body padding line $i for size."; done; } > "$f"
done

WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=3400
RESULT=$(_find_issue_plan 2>/dev/null)
assert_eq "two in-slot candidates → reverse-sort first (most recent) wins" "$NEWER" "$RESULT"

echo
echo "mika#2038 — the selection is logged to stderr (R5):"

CASE_ROOT=$(fresh_root)
DECOY="$CASE_ROOT/docs/plans/2026-04-11-003-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md"
{
    echo "# Plan: Bump rand"
    echo ""
    echo "**Issue:** #539"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$DECOY"
KEEPER="$CASE_ROOT/docs/plans/2026-08-29-005-obs-2026-real-plan.md"
{
    echo "# Plan: the real one"
    echo ""
    echo "**Ticket:** mika issue#2026"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$KEEPER"

WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=2026
STDERR=$(_find_issue_plan 2>&1 >/dev/null || true)

case "$STDERR" in
    *rustsec-2026-0097*539*) PASS=$((PASS + 1)); echo "  ✓ discard is logged with the candidate and the issue its header claimed" ;;
    *) FAIL=$((FAIL + 1)); echo "  ✗ discard line missing the candidate path and claimed issue; got: $STDERR" ;;
esac

case "$STDERR" in
    *2026-08-29-005-obs-2026-real-plan.md*) PASS=$((PASS + 1)); echo "  ✓ selection is logged with the chosen path" ;;
    *) FAIL=$((FAIL + 1)); echo "  ✗ selection line missing the chosen path; got: $STDERR" ;;
esac

echo
echo "mika#2038 — the refutation ledger tells callers WHY nothing was returned (R6):"

# The three PIPELINE FAILURE strings used to assert "no filename match" and send
# the operator after a discovery bug or pilot drift. After a refutation that is
# false: a plan matched and was discarded on purpose. Callers read this global.
CASE_ROOT=$(fresh_root)
{
    echo "# Plan: Bump rand"
    echo ""
    echo "**Issue:** #539"
    echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$CASE_ROOT/docs/plans/2026-04-11-003-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md"

WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=2026
_find_issue_plan >/dev/null 2>&1 || true
case "${FIND_ISSUE_PLAN_REFUTED:-}" in
    *rustsec-2026-0097*539*) PASS=$((PASS + 1)); echo "  ✓ the discarded candidate and its claimed issue reach the caller" ;;
    *) FAIL=$((FAIL + 1)); echo "  ✗ FIND_ISSUE_PLAN_REFUTED did not name the discard; got: '${FIND_ISSUE_PLAN_REFUTED:-}'" ;;
esac

# And it is cleared on entry, so a later clean call cannot inherit a stale claim.
CASE_ROOT=$(fresh_root)
GOOD="$CASE_ROOT/docs/plans/2026-08-29-006-fix-4242-clean-plan.md"
{
    echo "# Plan: clean"; echo ""
    for i in $(seq 1 30); do echo "Body padding line $i for size."; done
} > "$GOOD"
WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=4242
_find_issue_plan >/dev/null 2>&1 || true
if [ -z "${FIND_ISSUE_PLAN_REFUTED:-}" ]; then
    PASS=$((PASS + 1)); echo "  ✓ the ledger is cleared on entry (no stale claim carried forward)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ stale refutation carried into a clean call: '${FIND_ISSUE_PLAN_REFUTED}'"
fi

echo
echo "mika#2038 — empty plans dir is unchanged:"
CASE_ROOT=$(fresh_root)
WORKTREE_DIR="$CASE_ROOT"
ISSUE_NUM=9999
RESULT=$(_find_issue_plan 2>/dev/null || true)
assert_empty "empty docs/plans → no match, no stdout" "$RESULT"

# Restore the shared root for anything appended after this section.
WORKTREE_DIR="$TMPROOT"

# ============================================================================
# Summary
# ============================================================================

echo
echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] || exit 1
