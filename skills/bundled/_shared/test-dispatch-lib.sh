#!/bin/bash
# Test suite for dispatch-lib.sh — specifically the closed-issue auto-skip path (mika#988).
#
# Strategy: Extract and test the closed-issue detection logic in isolation.
# We cannot run the full dispatch_claude_pilot() in a test environment because
# it requires real mika CLI, claude-pilot, git, etc. Instead we verify:
#   1. The code structure: the closed-issue branch calls _deliver_callback and exit 0
#   2. The result JSON shape: validates the structured output format
#   3. The absence of crash semantics: no exit 1, no "Reopen first" error
#
# Run: bash skills/bundled/_shared/test-dispatch-lib.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/dispatch-lib.sh"

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
        echo "    expected: $expected"
        echo "    actual:   $actual"
    fi
}

assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if printf '%s' "$haystack" | grep -qF "$needle"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected to contain: $needle"
        echo "    actual: $haystack"
    fi
}

assert_not_contains() {
    local label="$1" needle="$2" haystack="$3"
    if ! printf '%s' "$haystack" | grep -qF "$needle"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected NOT to contain: $needle"
        echo "    actual: $haystack"
    fi
}

# --- Test 1: Code structure verification ---

echo ""
echo "Test 1: dispatch-lib.sh closed-issue branch structure"
echo "------------------------------------------------------"

# Verify the closed-issue branch calls _deliver_callback (not inline mika ask)
CLOSED_BLOCK=$(sed -n '/if \[ "\$ISSUE_STATE" = "CLOSED" \]/,/^[[:space:]]*fi/p' "$DISPATCH_LIB")

assert_contains "Closed branch calls _deliver_callback" "_deliver_callback" "$CLOSED_BLOCK"
assert_contains "Closed branch exits 0" "exit 0" "$CLOSED_BLOCK"

# Check no exit 1 in non-comment lines of the closed block
NON_COMMENT_EXIT1=$(printf '%s\n' "$CLOSED_BLOCK" | grep -v '^\s*#' | grep -c "exit 1" || true)
assert_eq "Closed branch has no exit 1 in code (only comments)" "0" "$NON_COMMENT_EXIT1"

assert_not_contains "No 'Reopen first' error message" "Reopen first" "$CLOSED_BLOCK"
assert_contains "Sets RESULT with auto_skipped status" '"status":"auto_skipped"' "$CLOSED_BLOCK"
assert_contains "Sets RESULT with issue_closed reason" '"reason":"issue_closed"' "$CLOSED_BLOCK"

# --- Test 2: Result JSON shape validation ---

echo ""
echo "Test 2: Auto-skip result JSON shape"
echo "------------------------------------"

# Simulate what the closed-issue branch produces
REPO="mika"
ISSUE_NUM="985"
RESULT=$(printf '{"status":"auto_skipped","reason":"issue_closed","issue":"senara-solutions/%s#%s","note":"Issue was already closed before dispatch fired. Presumed handled."}' "$REPO" "$ISSUE_NUM")

# Validate it's well-formed JSON
if printf '%s' "$RESULT" | jq . >/dev/null 2>&1; then
    PASS=$((PASS + 1))
    echo "  ✓ Result is valid JSON"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Result is not valid JSON: $RESULT"
fi

# Validate individual fields
STATUS=$(printf '%s' "$RESULT" | jq -r '.status')
REASON=$(printf '%s' "$RESULT" | jq -r '.reason')
ISSUE=$(printf '%s' "$RESULT" | jq -r '.issue')
NOTE=$(printf '%s' "$RESULT" | jq -r '.note')

assert_eq "status field is auto_skipped" "auto_skipped" "$STATUS"
assert_eq "reason field is issue_closed" "issue_closed" "$REASON"
assert_eq "issue field has correct format" "senara-solutions/mika#985" "$ISSUE"
assert_contains "note field explains the skip" "already closed" "$NOTE"

# Validate it's a single line (no embedded newlines)
LINE_COUNT=$(printf '%s' "$RESULT" | wc -l)
assert_eq "Result is single-line (no embedded newlines)" "0" "$LINE_COUNT"

# --- Test 3: EXIT trap guard prevents duplicate delivery ---

echo ""
echo "Test 3: EXIT trap CALLBACK_SENT guard (structural)"
echo "---------------------------------------------------"

# Verify the exit trap checks CALLBACK_SENT=1 as its first guard
TRAP_FUNC=$(sed -n '/_dispatch_lib_exit_trap()/,/^}/p' "$DISPATCH_LIB")
assert_contains "EXIT trap checks CALLBACK_SENT" 'CALLBACK_SENT' "$TRAP_FUNC"

# Verify _deliver_callback sets CALLBACK_SENT=1
DELIVER_FUNC=$(sed -n '/^_deliver_callback()/,/^}/p' "$DISPATCH_LIB")
assert_contains "_deliver_callback sets CALLBACK_SENT=1" "CALLBACK_SENT=1" "$DELIVER_FUNC"

# --- Test 4: No regression — open-issue path unchanged ---

echo ""
echo "Test 4: Open-issue path does NOT auto-skip (structural)"
echo "--------------------------------------------------------"

# After the closed-issue fi, the script should continue to branch derivation
# Verify the code after the closed-issue block proceeds to derive-branch-name
AFTER_CLOSED=$(sed -n '/exit 0/,/derive-branch-name/{/derive-branch-name/p}' "$DISPATCH_LIB" | head -1)
assert_contains "Open-issue path continues to branch derivation" "derive-branch-name" "$AFTER_CLOSED"

# Verify there is no unconditional _deliver_callback outside the CLOSED block
# in the issue-state checking region (non-comment lines only)
ISSUE_STATE_REGION=$(sed -n '/ISSUE_STATE=.*jq.*state/,/derive-branch-name/p' "$DISPATCH_LIB")
# Count occurrences of _deliver_callback in non-comment lines
DELIVER_COUNT=$(printf '%s\n' "$ISSUE_STATE_REGION" | grep -v '^\s*#' | grep -c "_deliver_callback" || true)
assert_eq "Only one _deliver_callback call in the issue-state region (non-comment)" "1" "$DELIVER_COUNT"

# --- Test 5: Prompt recognition guidance ---

echo ""
echo "Test 5: self-dev prompt includes auto-skip recognition"
echo "-------------------------------------------------------"

PROMPT_FILE="$SCRIPT_DIR/../self-dev/system_prompt.md"
if [ -f "$PROMPT_FILE" ]; then
    # Use grep directly on the file to avoid shell argument size limits
    if grep -qF "auto_skipped" "$PROMPT_FILE"; then
        PASS=$((PASS + 1)); echo "  ✓ Prompt mentions auto_skipped"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ Prompt mentions auto_skipped"
    fi
    if grep -qF "Do not ask the operator" "$PROMPT_FILE"; then
        PASS=$((PASS + 1)); echo "  ✓ Prompt says not to ask operator"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ Prompt says not to ask operator"
    fi
    if grep -qF "Do not post a status message" "$PROMPT_FILE"; then
        PASS=$((PASS + 1)); echo "  ✓ Prompt says no status message"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ Prompt says no status message"
    fi
    if grep -qF "mika#988" "$PROMPT_FILE"; then
        PASS=$((PASS + 1)); echo "  ✓ Prompt references mika#988"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ Prompt references mika#988"
    fi
else
    FAIL=$((FAIL + 1))
    echo "  ✗ self-dev/system_prompt.md not found at expected path"
fi

# --- Test 6: Plan-on-branch detection (mika#1074) ---

echo ""
echo "Test 6: Plan-on-branch detection (_detect_plan_on_branch)"
echo "----------------------------------------------------------"

# Verify the helper function exists
PLAN_FUNC=$(sed -n '/_detect_plan_on_branch()/,/^}/p' "$DISPATCH_LIB")
assert_contains "Function _detect_plan_on_branch exists" "_detect_plan_on_branch()" "$PLAN_FUNC"

# Verify it guards on dev-pilot skill
assert_contains "Guards on dev-pilot skill" 'dev-pilot' "$PLAN_FUNC"

# Verify it checks ISSUE_BODY is non-empty
assert_contains "Checks ISSUE_BODY is non-empty" 'ISSUE_BODY' "$PLAN_FUNC"

# Verify it checks WORKTREE_DIR is non-empty
assert_contains "Checks WORKTREE_DIR is non-empty" 'WORKTREE_DIR' "$PLAN_FUNC"

# Verify it uses the correct plan callout pattern with docs/plans/ prefix
assert_contains "Uses docs/plans/ prefix in pattern" 'docs/plans/' "$PLAN_FUNC"

# Verify it validates file existence ([ -f or test -f)
assert_contains "Validates plan file exists before overriding" '[ -f' "$PLAN_FUNC"

# Verify it overrides ENTRY_COMMAND to /ce:work
assert_contains "Overrides ENTRY_COMMAND to /ce:work" '/ce:work' "$PLAN_FUNC"

# Verify the function is called in dispatch_claude_pilot after _set_up_worktree
# and before _handle_dry_run
DISPATCH_BODY=$(sed -n '/^dispatch_claude_pilot()/,/^}/p' "$DISPATCH_LIB")
# Extract the call sequence: _set_up_worktree, _detect_plan_on_branch, _handle_dry_run
CALL_SEQUENCE=$(printf '%s\n' "$DISPATCH_BODY" | grep -E '^\s+_(set_up_worktree|detect_plan_on_branch|handle_dry_run)' | tr -s ' ' | sed 's/^ //')
EXPECTED_SEQUENCE=$(printf '%s\n' "_set_up_worktree" "_detect_plan_on_branch" "_handle_dry_run")
assert_eq "Call ordering: _set_up_worktree -> _detect_plan_on_branch -> _handle_dry_run" "$EXPECTED_SEQUENCE" "$CALL_SEQUENCE"

# Verify the case switch is unchanged (dev-pilot still maps to /mika as default)
CASE_BLOCK=$(sed -n '/case "\$SKILL" in/,/esac/p' "$DISPATCH_LIB")
assert_contains "Case switch still maps dev-pilot to /mika" 'dev-pilot)  ENTRY_COMMAND="/mika"' "$CASE_BLOCK"
assert_contains "Case switch still maps dev-groom to /mika-groom-ticket" 'dev-groom)  ENTRY_COMMAND="/mika-groom-ticket"' "$CASE_BLOCK"

# Verify fallback behavior: function returns 0 (no-op) on guard failures
# 4 guards: skill, issue_body, worktree_dir, plan_path empty
NON_COMMENT_RETURNS=$(printf '%s\n' "$PLAN_FUNC" | grep -v '^\s*#' | grep -c 'return 0' || true)
assert_eq "Has 4 guard return statements (skill, body, worktree, plan_path)" "4" "$NON_COMMENT_RETURNS"

# --- Test 7: Plan callout regex extraction (mika#1074) ---

echo ""
echo "Test 7: Plan callout regex extraction (live)"
echo "----------------------------------------------"

# Exercise the grep -oP regex against a canonical callout string
CALLOUT_REGEX='> - \*\*Plan:\*\* `\Kdocs/plans/[^`]+'

# Happy path: canonical callout with trailing context
TEST_BODY='> - **Plan:** `docs/plans/2026-05-11-001-feat-foo-plan.md` (committed on branch @ abc1234)'
EXTRACTED=$(printf '%s\n' "$TEST_BODY" | grep -oP "$CALLOUT_REGEX" | head -1)
assert_eq "Extracts plan path from canonical callout" "docs/plans/2026-05-11-001-feat-foo-plan.md" "$EXTRACTED"

# Edge case: callout with no trailing context (just backtick-close)
TEST_BODY2='> - **Plan:** `docs/plans/short.md`'
EXTRACTED2=$(printf '%s\n' "$TEST_BODY2" | grep -oP "$CALLOUT_REGEX" | head -1)
assert_eq "Extracts plan path from minimal callout" "docs/plans/short.md" "$EXTRACTED2"

# Edge case: prose containing "Plan:" without docs/plans/ prefix — should NOT match
TEST_BODY3='The Plan: is to refactor the module'
EXTRACTED3=$(printf '%s\n' "$TEST_BODY3" | grep -oP "$CALLOUT_REGEX" | head -1 || true)
assert_eq "Prose Plan: without docs/plans/ prefix does not match" "" "$EXTRACTED3"

# Edge case: multiple callouts — first one wins
TEST_BODY4='> - **Plan:** `docs/plans/first.md` (committed on branch @ aaa)
> - **Plan:** `docs/plans/second.md` (committed on branch @ bbb)'
EXTRACTED4=$(printf '%s\n' "$TEST_BODY4" | grep -oP "$CALLOUT_REGEX" | head -1)
assert_eq "Multiple callouts: first one wins" "docs/plans/first.md" "$EXTRACTED4"

# Verify dry_run output includes entry_command field
DRY_RUN_JQ=$(sed -n '/_handle_dry_run()/,/^}/p' "$DISPATCH_LIB")
assert_contains "Dry run output includes entry_command" 'entry_command' "$DRY_RUN_JQ"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
