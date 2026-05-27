#!/bin/bash
# Test suite for dispatch-lib.sh fuzzy disposition/verdict parsers (mika#1272).
#
# Tests two-tier parsing: literal (tier 1) and fuzzy (tier 2) matching for
# both _parse_disposition() and _parse_verdict(). Verifies disambiguation
# priority (ESCALATE > ITERATE > READY for dispositions, ESCALATE > GROOMED
# for verdicts) and the _disposition_was_fuzzy() side-channel.
#
# Source isolation audit (2026-05-27): dispatch-lib.sh has no top-level
# imperative code — all `set -e`, `trap`, and env var references are inside
# function bodies. Safe to source directly without a guard variable.
#
# Run: bash skills/bundled/_shared/tests/test_parse_disposition.sh
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

# Helper: check fuzzy flag and return "1" or "0" as a string for assert_eq
check_fuzzy() {
    _disposition_was_fuzzy && echo "1" || echo "0"
}

# Cleanup tmpfile on exit
trap 'rm -f "$_DISPOSITION_FUZZY_FILE"' EXIT

# ============================================================================
# _parse_disposition — tier 1 (literal)
# ============================================================================
echo ""
echo "Test: _parse_disposition — literal (tier 1)"
echo "----------------------------------------------"

result=$(printf 'Disposition: READY' | _parse_disposition 2>/dev/null)
assert_eq "literal READY" "READY" "$result"
assert_eq "literal READY sets fuzzy=0" "0" "$(check_fuzzy)"

result=$(printf 'Disposition: ITERATE' | _parse_disposition 2>/dev/null)
assert_eq "literal ITERATE" "ITERATE" "$result"
assert_eq "literal ITERATE sets fuzzy=0" "0" "$(check_fuzzy)"

result=$(printf 'Disposition: ESCALATE' | _parse_disposition 2>/dev/null)
assert_eq "literal ESCALATE" "ESCALATE" "$result"
assert_eq "literal ESCALATE sets fuzzy=0" "0" "$(check_fuzzy)"

# With surrounding text
result=$(printf 'Some preamble.\n\nDisposition: READY\n\nSome trailing text.' | _parse_disposition 2>/dev/null)
assert_eq "literal READY with surrounding text" "READY" "$result"

# ============================================================================
# _parse_disposition — tier 2 (fuzzy)
# ============================================================================
echo ""
echo "Test: _parse_disposition — fuzzy (tier 2)"
echo "--------------------------------------------"

# READY paraphrases
result=$(printf 'The plan looks great. Proceed with implementation.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy READY: proceed" "READY" "$result"
assert_eq "fuzzy READY sets fuzzy=1" "1" "$(check_fuzzy)"

result=$(printf 'This is solid work. Ship it to the dev team.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy READY: ship it" "READY" "$result"

result=$(printf 'Looks good to go for second review.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy READY: good to go" "READY" "$result"

result=$(printf 'The plan is clean and well-structured.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy READY: plan is clean" "READY" "$result"

result=$(printf 'Ready to dispatch to dev-pilot.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy READY: dispatch" "READY" "$result"

# ITERATE paraphrases
result=$(printf 'The plan needs revision on section 3.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ITERATE: needs revision" "ITERATE" "$result"
assert_eq "fuzzy ITERATE sets fuzzy=1" "1" "$(check_fuzzy)"

result=$(printf 'I would like another pass on the error handling section.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ITERATE: another pass" "ITERATE" "$result"

result=$(printf 'Please revise the implementation units.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ITERATE: revise" "ITERATE" "$result"

result=$(printf 'Please address the following concerns before proceeding.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ITERATE: address the following" "ITERATE" "$result"

result=$(printf 'There are concerns that require attention in Unit 2.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ITERATE: concerns that require" "ITERATE" "$result"

# ESCALATE paraphrases
result=$(printf 'This needs to escalate to the operator for review.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ESCALATE: escalate" "ESCALATE" "$result"
assert_eq "fuzzy ESCALATE sets fuzzy=1" "1" "$(check_fuzzy)"

result=$(printf 'This requires human review before we can proceed.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ESCALATE: human review" "ESCALATE" "$result"

result=$(printf 'We cannot proceed with this approach.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ESCALATE: cannot proceed" "ESCALATE" "$result"

result=$(printf 'There is a fundamental issue with the architecture.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ESCALATE: fundamental" "ESCALATE" "$result"

result=$(printf 'This is out of scope for the current ticket.' | _parse_disposition 2>/dev/null)
assert_eq "fuzzy ESCALATE: out of scope for" "ESCALATE" "$result"

# ============================================================================
# _parse_disposition — disambiguation priority
# ============================================================================
echo ""
echo "Test: _parse_disposition — disambiguation (ESCALATE > ITERATE > READY)"
echo "------------------------------------------------------------------------"

# ESCALATE + READY → ESCALATE wins
result=$(printf 'Proceed with caution but there is a fundamental problem here.' | _parse_disposition 2>/dev/null)
assert_eq "ESCALATE > READY: fundamental + proceed" "ESCALATE" "$result"

# ESCALATE + ITERATE → ESCALATE wins
result=$(printf 'Needs revision but the fundamental design is wrong.' | _parse_disposition 2>/dev/null)
assert_eq "ESCALATE > ITERATE: fundamental + needs revision" "ESCALATE" "$result"

# ITERATE + READY → ITERATE wins
result=$(printf 'I would proceed but the plan needs revision first.' | _parse_disposition 2>/dev/null)
assert_eq "ITERATE > READY: needs revision + proceed" "ITERATE" "$result"

# All three → ESCALATE wins
result=$(printf 'Proceed after revision but there is a fundamental blocker.' | _parse_disposition 2>/dev/null)
assert_eq "All three → ESCALATE wins" "ESCALATE" "$result"

# ============================================================================
# _parse_disposition — no match
# ============================================================================
echo ""
echo "Test: _parse_disposition — no match"
echo "--------------------------------------"

result=$(printf 'This is a completely unrelated paragraph about the weather.' | _parse_disposition 2>/dev/null)
assert_eq "unrelated text → empty" "" "$result"
assert_eq "no match keeps fuzzy=0" "0" "$(check_fuzzy)"

result=$(printf '' | _parse_disposition 2>/dev/null)
assert_eq "empty input → empty" "" "$result"

# ============================================================================
# _parse_verdict — tier 1 (literal)
# ============================================================================
echo ""
echo "Test: _parse_verdict — literal (tier 1)"
echo "------------------------------------------"

result=$(printf 'Verdict: GROOMED' | _parse_verdict 2>/dev/null)
assert_eq "literal GROOMED" "GROOMED" "$result"
assert_eq "literal GROOMED sets fuzzy=0" "0" "$(check_fuzzy)"

result=$(printf 'Verdict: ESCALATE' | _parse_verdict 2>/dev/null)
assert_eq "literal ESCALATE" "ESCALATE" "$result"
assert_eq "literal ESCALATE sets fuzzy=0" "0" "$(check_fuzzy)"

# With surrounding text
result=$(printf 'After careful review.\n\nVerdict: GROOMED\n\nEnd.' | _parse_verdict 2>/dev/null)
assert_eq "literal GROOMED with surrounding text" "GROOMED" "$result"

# ============================================================================
# _parse_verdict — tier 2 (fuzzy)
# ============================================================================
echo ""
echo "Test: _parse_verdict — fuzzy (tier 2)"
echo "----------------------------------------"

# GROOMED paraphrases
result=$(printf 'The plan has been groomed and is ready for implementation.' | _parse_verdict 2>/dev/null)
assert_eq "fuzzy GROOMED: groomed" "GROOMED" "$result"
assert_eq "fuzzy GROOMED sets fuzzy=1" "1" "$(check_fuzzy)"

result=$(printf 'The revised plan is approved for dispatch.' | _parse_verdict 2>/dev/null)
assert_eq "fuzzy GROOMED: approved" "GROOMED" "$result"

result=$(printf 'The plan is ready for the dev-pilot.' | _parse_verdict 2>/dev/null)
assert_eq "fuzzy GROOMED: plan is ready" "GROOMED" "$result"

result=$(printf 'Ship it to dev-pilot.' | _parse_verdict 2>/dev/null)
assert_eq "fuzzy GROOMED: ship it" "GROOMED" "$result"

# Verify "ship" as a substring does NOT false-positive (e.g., "relationship")
result=$(printf 'The relationship between components is well-defined.' | _parse_verdict 2>/dev/null)
assert_eq "no false positive: ship in relationship" "" "$result"

result=$(printf 'I have no remaining concerns with this plan.' | _parse_verdict 2>/dev/null)
assert_eq "fuzzy GROOMED: no remaining concerns" "GROOMED" "$result"

# ESCALATE paraphrases
result=$(printf 'I need to escalate this to the operator.' | _parse_verdict 2>/dev/null)
assert_eq "fuzzy ESCALATE: escalate" "ESCALATE" "$result"
assert_eq "fuzzy ESCALATE sets fuzzy=1" "1" "$(check_fuzzy)"

result=$(printf 'I cannot approve this plan in its current state.' | _parse_verdict 2>/dev/null)
assert_eq "fuzzy ESCALATE: cannot approve" "ESCALATE" "$result"

result=$(printf 'Human review needed for this architectural decision.' | _parse_verdict 2>/dev/null)
assert_eq "fuzzy ESCALATE: human review needed" "ESCALATE" "$result"

result=$(printf 'Fundamental issues remain after the revision.' | _parse_verdict 2>/dev/null)
assert_eq "fuzzy ESCALATE: fundamental issues remain" "ESCALATE" "$result"

# ============================================================================
# _parse_verdict — disambiguation priority
# ============================================================================
echo ""
echo "Test: _parse_verdict — disambiguation (ESCALATE > GROOMED)"
echo "-------------------------------------------------------------"

# ESCALATE + GROOMED → ESCALATE wins
result=$(printf 'The plan is groomed but I need to escalate the security concern.' | _parse_verdict 2>/dev/null)
assert_eq "ESCALATE > GROOMED: escalate + groomed" "ESCALATE" "$result"

result=$(printf 'Approved overall but fundamental issues remain with auth.' | _parse_verdict 2>/dev/null)
assert_eq "ESCALATE > GROOMED: fundamental issues remain + approved" "ESCALATE" "$result"

# ============================================================================
# _parse_verdict — no match
# ============================================================================
echo ""
echo "Test: _parse_verdict — no match"
echo "----------------------------------"

result=$(printf 'This paragraph is about something entirely different.' | _parse_verdict 2>/dev/null)
assert_eq "unrelated text → empty" "" "$result"

result=$(printf '' | _parse_verdict 2>/dev/null)
assert_eq "empty input → empty" "" "$result"

# ============================================================================
# Case insensitivity
# ============================================================================
echo ""
echo "Test: fuzzy matching is case-insensitive"
echo "-------------------------------------------"

result=$(printf 'PROCEED with the implementation.' | _parse_disposition 2>/dev/null)
assert_eq "PROCEED (uppercase) → READY" "READY" "$result"

result=$(printf 'Needs Revision on the plan.' | _parse_disposition 2>/dev/null)
assert_eq "Needs Revision (mixed case) → ITERATE" "ITERATE" "$result"

result=$(printf 'GROOMED and ready to ship.' | _parse_verdict 2>/dev/null)
assert_eq "GROOMED (uppercase, fuzzy) → GROOMED" "GROOMED" "$result"

# ============================================================================
# Summary
# ============================================================================
echo ""
echo "======================================"
echo "Results: $PASS passed, $FAIL failed"
echo "======================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
