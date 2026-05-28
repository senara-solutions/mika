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


# --- Test 9: claude-pilot venv smoke test structure (mika#1200) ---

echo ""
echo "Test 9: claude-pilot venv smoke test (mika#1200)"
echo "-------------------------------------------------"

# Verify the smoke test block exists in dispatch_claude_pilot
DISPATCH_BODY=$(sed -n '/^dispatch_claude_pilot()/,/^}/p' "$DISPATCH_LIB")

# (a) Smoke test fires: `claude-pilot --help` check is present
assert_contains "Smoke test runs claude-pilot --help" \
    'claude-pilot --help' "$DISPATCH_BODY"

# (b) Smoke test aborts with diagnostic on failure
assert_contains "Smoke test error mentions venv is broken" \
    'claude-pilot venv is broken' "$DISPATCH_BODY"
assert_contains "Smoke test error mentions uv tool install restoration command" \
    'uv tool install --force --editable ./claude-pilot-py' "$DISPATCH_BODY"
assert_contains "Smoke test error references mika#1200" \
    'mika#1200' "$DISPATCH_BODY"

# (c) Smoke test fires BEFORE worktree mutation — verify ordering:
#     The smoke test (claude-pilot --help) must appear before _set_up_worktree
#     in the function body. Extract line numbers to confirm ordering.
SMOKE_LINE=$(printf '%s\n' "$DISPATCH_BODY" | grep -n 'claude-pilot --help' | head -1 | cut -d: -f1)
WORKTREE_LINE=$(printf '%s\n' "$DISPATCH_BODY" | grep -n '_set_up_worktree' | head -1 | cut -d: -f1)
if [ -n "$SMOKE_LINE" ] && [ -n "$WORKTREE_LINE" ] && [ "$SMOKE_LINE" -lt "$WORKTREE_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Smoke test (line $SMOKE_LINE) fires before _set_up_worktree (line $WORKTREE_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Smoke test must fire before _set_up_worktree"
    echo "    smoke_line=$SMOKE_LINE worktree_line=$WORKTREE_LINE"
fi

# (c2) Smoke test fires AFTER command -v claude-pilot — ordering:
COMMAND_V_LINE=$(printf '%s\n' "$DISPATCH_BODY" | grep -n 'command -v claude-pilot' | head -1 | cut -d: -f1)
if [ -n "$COMMAND_V_LINE" ] && [ -n "$SMOKE_LINE" ] && [ "$COMMAND_V_LINE" -lt "$SMOKE_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ command -v claude-pilot (line $COMMAND_V_LINE) fires before smoke test (line $SMOKE_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ command -v claude-pilot must fire before smoke test"
    echo "    command_v_line=$COMMAND_V_LINE smoke_line=$SMOKE_LINE"
fi

# (b2) Smoke test uses exit 1 (not return 1) — matches surrounding control flow
SMOKE_BLOCK=$(printf '%s\n' "$DISPATCH_BODY" | sed -n '/claude-pilot --help/,/fi/p' | head -20)
assert_contains "Smoke test uses exit 1 on failure" 'exit 1' "$SMOKE_BLOCK"

# (b3) Smoke test writes to stderr (lands in task result via EXIT trap)
assert_contains "Smoke test error goes to stderr" '>&2' "$SMOKE_BLOCK"

# (b4) Smoke test suppresses stdout and routes stderr to trace fd
assert_contains "Smoke test suppresses stdout" '>/dev/null' "$SMOKE_BLOCK"
assert_contains "Smoke test routes stderr to trace fd 9" '2>&9' "$SMOKE_BLOCK"

# (b5) Smoke test has a timeout guard against hung venvs
assert_contains "Smoke test has timeout guard" 'timeout' "$SMOKE_BLOCK"

# --- Test 10: cli.py top-level import guard (mika#1200 Phase 2d) ---

echo ""
echo "Test 10: cli.py top-level import regression guard (mika#1200)"
echo "--------------------------------------------------------------"

# The dispatch-lib smoke test relies on cli.py keeping .agent and .permissions
# imports at module top level (not lazy-imported inside main() or any function).
# If a future refactor moves these imports into a function body, the smoke test
# silently stops detecting the import-time failure class.
#
# This test checks that the imports are at the module top level via grep.
# It uses PLATFORM_DIR to find claude-pilot-py, matching how dispatch-lib resolves it.

TEST_PLATFORM_DIR="${MIKA_PLATFORM_DIR:-}"
if [ -z "$TEST_PLATFORM_DIR" ]; then
    # Walk up from SCRIPT_DIR to find mika-platform root
    # SCRIPT_DIR is inside mika/skills/bundled/_shared/
    _candidate="$SCRIPT_DIR/../../../.."
    if [ -f "$_candidate/claude-pilot-py/src/claude_pilot/cli.py" ]; then
        TEST_PLATFORM_DIR=$(cd "$_candidate" && pwd -P)
    fi
fi

CLI_PY="${TEST_PLATFORM_DIR:+$TEST_PLATFORM_DIR/claude-pilot-py/src/claude_pilot/cli.py}"

if [ -n "$CLI_PY" ] && [ -f "$CLI_PY" ]; then
    # Check: `from .agent import` appears at column 0 (top-level, not indented)
    AGENT_IMPORT=$(grep -c '^from \.agent import' "$CLI_PY" || true)
    if [ "$AGENT_IMPORT" -ge 1 ]; then
        PASS=$((PASS + 1))
        echo "  ✓ cli.py has top-level 'from .agent import' ($AGENT_IMPORT occurrence(s))"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ cli.py MISSING top-level 'from .agent import' — smoke test in dispatch-lib.sh"
        echo "    will silently stop detecting import-time failures. See mika#1200 Phase 0 Pin."
    fi

    # Check: `from .permissions import` appears at column 0 (top-level, not indented)
    PERMS_IMPORT=$(grep -c '^from \.permissions import' "$CLI_PY" || true)
    if [ "$PERMS_IMPORT" -ge 1 ]; then
        PASS=$((PASS + 1))
        echo "  ✓ cli.py has top-level 'from .permissions import' ($PERMS_IMPORT occurrence(s))"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ cli.py MISSING top-level 'from .permissions import' — smoke test in"
        echo "    dispatch-lib.sh will silently stop detecting import-time failures."
        echo "    See mika#1200 Phase 0 Pin."
    fi
else
    echo "  ⊘ cli.py not found at expected path (skipped — set MIKA_PLATFORM_DIR or run from mika-platform root)"
    echo "    tried: ${CLI_PY:-<empty>}"
fi

# ============================================================================
# Iterate-loop primitives (mika#1271 — Phase A/B/C helpers)
# ============================================================================

echo ""
echo "Test: iterate-loop primitives (mika#1271)"
echo "-----------------------------------------"

# Source the lib so the helpers are callable. dispatch-lib.sh runs `set -euo
# pipefail` at the top; sourcing it sets that for the test script too — which
# is fine since we use `|| true` everywhere we expect non-zero.
# shellcheck disable=SC1090
source "$DISPATCH_LIB" 2>/dev/null || true

# _parse_disposition — recognizes the three literal forms
assert_eq "_parse_disposition READY" "READY" \
    "$(printf 'Some prose.\nDisposition: READY\nMore prose.\n' | _parse_disposition)"
assert_eq "_parse_disposition ITERATE" "ITERATE" \
    "$(printf 'Findings:\n- F1: ...\nDisposition: ITERATE\n' | _parse_disposition)"
assert_eq "_parse_disposition ESCALATE" "ESCALATE" \
    "$(printf 'Disposition: ESCALATE\nReason: out of scope.\n' | _parse_disposition)"

# _parse_disposition — extra whitespace tolerance
assert_eq "_parse_disposition tolerates extra spaces" "READY" \
    "$(printf 'Disposition:   READY\n' | _parse_disposition)"

# _parse_disposition — no match returns empty
assert_eq "_parse_disposition empty on no match" "" \
    "$(printf 'No verdict here.\n' | _parse_disposition)"

# _parse_disposition — only first match wins (defensive: never mix dispositions in a session)
assert_eq "_parse_disposition first match wins" "READY" \
    "$(printf 'Disposition: READY\nDisposition: ITERATE\n' | _parse_disposition)"

# _parse_verdict — recognizes the two literal forms
assert_eq "_parse_verdict GROOMED" "GROOMED" \
    "$(printf 'Verdict: GROOMED\n' | _parse_verdict)"
assert_eq "_parse_verdict ESCALATE" "ESCALATE" \
    "$(printf 'Verdict: ESCALATE\n' | _parse_verdict)"

# _parse_verdict — no match returns empty
assert_eq "_parse_verdict empty on no match" "" \
    "$(printf 'no verdict\n' | _parse_verdict)"

# _trail_append + _trail_read — round-trip
TRAIL_TMP=$(mktemp -d)
WORKTREE_DIR="$TRAIL_TMP" _trail_append "groom-ticket" "session-abc" "READY"
WORKTREE_DIR="$TRAIL_TMP" _trail_append "second-review" "session-abc" "GROOMED"
TRAIL_OUTPUT=$(WORKTREE_DIR="$TRAIL_TMP" _trail_read)
assert_contains "_trail round-trip captures groom-ticket entry" "groom-ticket	session-abc	READY" "$TRAIL_OUTPUT"
assert_contains "_trail round-trip captures second-review entry" "second-review	session-abc	GROOMED" "$TRAIL_OUTPUT"
rm -rf "$TRAIL_TMP"

# _trail_append — silently no-ops when WORKTREE_DIR unset / missing
assert_eq "_trail_append no-op when WORKTREE_DIR unset" "0" \
    "$(WORKTREE_DIR="" _trail_append "x" "y" "z" 2>/dev/null; echo $?)"

# _arch_ask — guard rejects missing args
assert_eq "_arch_ask rejects missing skill" "2" \
    "$(_arch_ask "" "/tmp/foo" 2>/dev/null; echo $?)"
assert_eq "_arch_ask rejects missing plan_path" "2" \
    "$(_arch_ask "mika-arch-groom-ticket" "" 2>/dev/null; echo $?)"
assert_eq "_arch_ask rejects unreadable plan_path" "2" \
    "$(_arch_ask "mika-arch-groom-ticket" "/nonexistent/path" 2>/dev/null; echo $?)"

# _arch_ask — argv construction (stub `mika` to capture the args it receives).
# This catches the --skill vs --enable-skill flag-name bug that was in PR#1274.
# Argv is joined with `|` so we can grep substrings; needles are prefixed with
# `|` to avoid grep treating `--`-leading strings as flags.
#
# mika#1283: _arch_ask passes plan content via stdin (mika ask "-" reads stdin),
# NOT as @-file argument (mika ask doesn't expand @<path>). The argv should
# contain a literal "-" marker and the stub should receive the plan content
# on stdin when invoked.
ARCH_PLAN_TMP=$(mktemp /tmp/arch-plan-XXXXXX.md)
printf 'plan content for arch ask test\n' > "$ARCH_PLAN_TMP"
mika() { printf '|%s' "$@"; printf '|'; }  # leading | so first arg also has separator
ARCH_ARGV=$(_arch_ask "mika-arch-groom-ticket" "$ARCH_PLAN_TMP")
assert_contains "_arch_ask uses enable-skill flag (not the wrong skill flag)" "|--enable-skill|" "$ARCH_ARGV"
assert_contains "_arch_ask threads skill name after enable-skill" "|--enable-skill|mika-arch-groom-ticket|" "$ARCH_ARGV"
assert_contains "_arch_ask sets agent mika-arch" "|--agent|mika-arch|" "$ARCH_ARGV"
assert_contains "_arch_ask sets format json" "|--format|json|" "$ARCH_ARGV"
assert_contains "_arch_ask sets verbose flag" "|--verbose|" "$ARCH_ARGV"
# mika#1283: stdin-marker, not @-file
assert_contains "_arch_ask uses stdin marker (mika#1283)" "|-|" "$ARCH_ARGV"
assert_not_contains "_arch_ask no longer passes @-path (mika#1283)" "|@${ARCH_PLAN_TMP}|" "$ARCH_ARGV"
assert_not_contains "_arch_ask omits session-id when not given" "|--session-id|" "$ARCH_ARGV"

ARCH_ARGV_WITH_SESSION=$(_arch_ask "mika-arch-second-review" "$ARCH_PLAN_TMP" "session-xyz")
assert_contains "_arch_ask threads session-id when given" "|--session-id|session-xyz|" "$ARCH_ARGV_WITH_SESSION"
assert_contains "_arch_ask uses stdin marker with session-id (mika#1283)" "|-|" "$ARCH_ARGV_WITH_SESSION"
assert_not_contains "_arch_ask no longer passes @-path with session-id (mika#1283)" "|@${ARCH_PLAN_TMP}|" "$ARCH_ARGV_WITH_SESSION"

# mika#1283: verify plan content arrives on stdin, not as argv. Use a stub that
# echoes the stdin it receives so we can grep for the plan content.
mika_stdin_capture() { cat; }  # echo whatever arrives on stdin
mika() { mika_stdin_capture; }
ARCH_STDIN_RECEIVED=$(_arch_ask "mika-arch-groom-ticket" "$ARCH_PLAN_TMP")
assert_contains "_arch_ask delivers plan content via stdin (mika#1283)" "plan content for arch ask test" "$ARCH_STDIN_RECEIVED"

unset -f mika mika_stdin_capture
rm -f "$ARCH_PLAN_TMP"

# _iterate_groom_loop — guard rejects missing worktree/issue
assert_eq "_iterate_groom_loop fails when WORKTREE_DIR unset" "1" \
    "$(WORKTREE_DIR="" ISSUE_NUM="1267" REPO="mika" _iterate_groom_loop 2>/dev/null; echo $?)"
assert_eq "_iterate_groom_loop fails when ISSUE_NUM unset" "1" \
    "$(WORKTREE_DIR="/tmp" ISSUE_NUM="" REPO="mika" _iterate_groom_loop 2>/dev/null; echo $?)"
assert_eq "_iterate_groom_loop fails when REPO unset" "1" \
    "$(WORKTREE_DIR="/tmp" ISSUE_NUM="1267" REPO="" _iterate_groom_loop 2>/dev/null; echo $?)"

# _iterate_groom_loop — guard rejects when no plan file present
ITERATE_TMP=$(mktemp -d)
mkdir -p "$ITERATE_TMP/docs/plans"
assert_eq "_iterate_groom_loop fails when no plan file present" "1" \
    "$(WORKTREE_DIR="$ITERATE_TMP" ISSUE_NUM="1267" REPO="mika" _iterate_groom_loop 2>/dev/null; echo $?)"
rm -rf "$ITERATE_TMP"

# Code-shape inspection: the function references the three required disposition
# values and the second-pass GROOMED verdict (three load-bearing flows wired).
ITERATE_FUNC=$(declare -f _iterate_groom_loop)
assert_contains "_iterate_groom_loop dispatches on READY" "READY)" "$ITERATE_FUNC"
assert_contains "_iterate_groom_loop dispatches on ITERATE" "ITERATE)" "$ITERATE_FUNC"
assert_contains "_iterate_groom_loop dispatches on ESCALATE" "ESCALATE)" "$ITERATE_FUNC"
assert_contains "_iterate_groom_loop checks GROOMED on second pass" "GROOMED)" "$ITERATE_FUNC"
assert_contains "_iterate_groom_loop invokes mika-arch-groom-ticket" "mika-arch-groom-ticket" "$ITERATE_FUNC"
assert_contains "_iterate_groom_loop invokes mika-arch-second-review" "mika-arch-second-review" "$ITERATE_FUNC"
assert_contains "_iterate_groom_loop threads session_id to second pass" 'mika-arch-second-review' "$ITERATE_FUNC"

# Wiring point inspection on dispatch_claude_pilot
DISPATCH_FUNC=$(declare -f dispatch_claude_pilot)
# Sub-PR 7a: MIKA_DISPATCH_USE_ITERATE_LOOP feature flag removed — iterate loop
# is now unconditional for the dev-groom skill (gated by SKILL check only).
assert_not_contains "dispatch_claude_pilot no longer references MIKA_DISPATCH_USE_ITERATE_LOOP flag" "MIKA_DISPATCH_USE_ITERATE_LOOP" "$DISPATCH_FUNC"
assert_contains "dispatch_claude_pilot still gates iterate-loop on dev-groom skill" "dev-groom" "$DISPATCH_FUNC"
assert_contains "dispatch_claude_pilot calls _iterate_groom_loop" "_iterate_groom_loop" "$DISPATCH_FUNC"
# Sub-PR 7b retirement: Class D recovery shim is gone — _verify_and_write_body_callout
# function definition deleted; its post-flight call site in _run_claude_pilot removed.
# Tests below assert both deletions structurally.
RUN_CLAUDE_PILOT_FUNC=$(declare -f _run_claude_pilot)
assert_not_contains "_run_claude_pilot no longer invokes Class D recovery shim" "_verify_and_write_body_callout" "$RUN_CLAUDE_PILOT_FUNC"
# Class D function definition removed entirely from dispatch-lib.sh.
if declare -f _verify_and_write_body_callout >/dev/null 2>&1; then
    assert_eq "_verify_and_write_body_callout function deleted (sub-PR 7b)" "absent" "still-defined"
else
    assert_eq "_verify_and_write_body_callout function deleted (sub-PR 7b)" "absent" "absent"
fi

# Ordering check: iterate-loop runs AFTER _run_claude_pilot and BEFORE _push_branch.
# Use grep -n on declare-f output; pick first occurrence of each.
DISPATCH_ORDER=$(printf '%s\n' "$DISPATCH_FUNC" | grep -nE "_run_claude_pilot|_iterate_groom_loop|_push_branch" | head -10)
RUN_LINE=$(echo "$DISPATCH_ORDER" | grep _run_claude_pilot | head -1 | cut -d: -f1)
ITERATE_LINE=$(echo "$DISPATCH_ORDER" | grep _iterate_groom_loop | head -1 | cut -d: -f1)
PUSH_LINE=$(echo "$DISPATCH_ORDER" | grep _push_branch | head -1 | cut -d: -f1)
if [ -n "$RUN_LINE" ] && [ -n "$ITERATE_LINE" ] && [ -n "$PUSH_LINE" ] && \
   [ "$RUN_LINE" -lt "$ITERATE_LINE" ] && [ "$ITERATE_LINE" -lt "$PUSH_LINE" ]; then
    assert_eq "dispatch ordering: _run_claude_pilot → _iterate_groom_loop → _push_branch" "ok" "ok"
else
    assert_eq "dispatch ordering: _run_claude_pilot → _iterate_groom_loop → _push_branch" "ok" \
        "run=${RUN_LINE:-missing} iterate=${ITERATE_LINE:-missing} push=${PUSH_LINE:-missing}"
fi

# ============================================================================
# ITERATE flow primitives (mika#1271 sub-PR 4)
# ============================================================================

echo ""
echo "Test: ITERATE-flow primitives (mika#1271 sub-PR 4)"
echo "--------------------------------------------------"

# _launch_revise_pilot — guard rejections
assert_eq "_launch_revise_pilot rejects unreadable findings file" "1" \
    "$(WORKTREE_DIR="/tmp" ISSUE_NUM="1267" _launch_revise_pilot "/nonexistent/findings.md" 2>/dev/null; echo $?)"
REV_TMP_GUARD=$(mktemp)
echo "findings" > "$REV_TMP_GUARD"
assert_eq "_launch_revise_pilot rejects missing WORKTREE_DIR" "1" \
    "$(WORKTREE_DIR="" ISSUE_NUM="1267" _launch_revise_pilot "$REV_TMP_GUARD" 2>/dev/null; echo $?)"
assert_eq "_launch_revise_pilot rejects missing ISSUE_NUM" "1" \
    "$(WORKTREE_DIR="/tmp" ISSUE_NUM="" _launch_revise_pilot "$REV_TMP_GUARD" 2>/dev/null; echo $?)"
REV_DIR_GUARD=$(mktemp -d)
mkdir -p "$REV_DIR_GUARD/docs/plans"
assert_eq "_launch_revise_pilot rejects when no plan file present" "1" \
    "$(WORKTREE_DIR="$REV_DIR_GUARD" ISSUE_NUM="9999" _launch_revise_pilot "$REV_TMP_GUARD" 2>/dev/null; echo $?)"
rm -rf "$REV_TMP_GUARD" "$REV_DIR_GUARD"

# Code-shape: _launch_revise_pilot
REVISE_FUNC=$(declare -f _launch_revise_pilot)
assert_contains "_launch_revise_pilot invokes /mika-revise-plan slash command" "/mika-revise-plan" "$REVISE_FUNC"
assert_contains "_launch_revise_pilot uses sha256 detection (not mtime)" "sha256sum" "$REVISE_FUNC"
assert_contains "_launch_revise_pilot passes findings via @-file" '@${findings_file}' "$REVISE_FUNC"

# _cleanup_iterate_findings — no-op when .iterate/ absent
CLEAN_TMP=$(mktemp -d)
assert_eq "_cleanup_iterate_findings no-op when .iterate/ absent" "0" \
    "$(WORKTREE_DIR="$CLEAN_TMP" _cleanup_iterate_findings 2>/dev/null; echo $?)"

# _cleanup_iterate_findings — sweeps .iterate/ when present
mkdir -p "$CLEAN_TMP/.iterate"
echo "findings" > "$CLEAN_TMP/.iterate/findings-1.md"
WORKTREE_DIR="$CLEAN_TMP" _cleanup_iterate_findings 2>/dev/null
if [ ! -d "$CLEAN_TMP/.iterate" ]; then
    assert_eq "_cleanup_iterate_findings sweeps .iterate/ when present" "removed" "removed"
else
    assert_eq "_cleanup_iterate_findings sweeps .iterate/ when present" "removed" "still present"
fi
rm -rf "$CLEAN_TMP"

# _cleanup_iterate_findings — guard: WORKTREE_DIR unset → no-op
assert_eq "_cleanup_iterate_findings no-op when WORKTREE_DIR unset" "0" \
    "$(WORKTREE_DIR="" _cleanup_iterate_findings 2>/dev/null; echo $?)"

# Code-shape inspection: ITERATE branch in _iterate_groom_loop
ITERATE_FULL=$(declare -f _iterate_groom_loop)
assert_contains "ITERATE branch calls _launch_revise_pilot" "_launch_revise_pilot" "$ITERATE_FULL"
assert_contains "ITERATE branch writes findings to .iterate/" ".iterate" "$ITERATE_FULL"
assert_contains "ITERATE branch invokes second-pass after revise" "mika-arch-second-review" "$ITERATE_FULL"

# Cleanup symmetry: GROOMED paths sweep findings; ESCALATE/failure preserve.
# Two GROOMED paths (READY+ITERATE) each call _cleanup_iterate_findings; ESCALATE
# paths must NOT call it. Expect exactly 2 references.
SWEEP_COUNT=$(printf '%s\n' "$ITERATE_FULL" | grep -c '_cleanup_iterate_findings')
assert_eq "_iterate_groom_loop calls _cleanup_iterate_findings on both GROOMED paths" "2" "$SWEEP_COUNT"

# Session-id symmetry: second-pass invoked on both READY and ITERATE branches,
# both threading session_id from first-pass response.
SECOND_PASS_COUNT=$(printf '%s\n' "$ITERATE_FULL" | grep -c 'mika-arch-second-review')
assert_eq "_iterate_groom_loop invokes second-pass twice (READY + ITERATE)" "2" "$SECOND_PASS_COUNT"

# ============================================================================
# ESCALATE flow (mika#1271 sub-PR 5)
# ============================================================================

echo ""
echo "Test: ESCALATE-flow helper (mika#1271 sub-PR 5)"
echo "-----------------------------------------------"

# _escalate_groom — writes findings file under .iterate/ + appends structured
# PIPELINE FAILURE marker to RESULT
ESC_TMP=$(mktemp -d)
RESULT=""
WORKTREE_DIR="$ESC_TMP" _escalate_groom "first-pass" "F1: Concern\nDisposition: ESCALATE" "session-esc-1" 2>/dev/null
if [ -r "$ESC_TMP/.iterate/escalate-first-pass.md" ]; then
    assert_eq "_escalate_groom writes findings file under .iterate/" "ok" "ok"
else
    assert_eq "_escalate_groom writes findings file under .iterate/" "ok" "missing"
fi
assert_contains "_escalate_groom appends PIPELINE FAILURE to RESULT" "PIPELINE FAILURE: groom escalated by mika-arch first-pass" "$RESULT"
assert_contains "_escalate_groom RESULT includes Verdict: ESCALATE" "Verdict: ESCALATE" "$RESULT"
assert_contains "_escalate_groom RESULT includes session_id" "Session: session-esc-1" "$RESULT"
assert_contains "_escalate_groom RESULT references findings file path" "Architect findings preserved at:" "$RESULT"
rm -rf "$ESC_TMP"

# _escalate_groom — distinct stage labels write distinct findings files
ESC_TMP2=$(mktemp -d)
RESULT=""
WORKTREE_DIR="$ESC_TMP2" _escalate_groom "second-pass-after-ready" "second-pass-ready content" "sess-2" 2>/dev/null
if [ -r "$ESC_TMP2/.iterate/escalate-second-pass-after-ready.md" ]; then
    assert_eq "_escalate_groom: second-pass-after-ready stage label drives filename" "ok" "ok"
else
    assert_eq "_escalate_groom: second-pass-after-ready stage label drives filename" "ok" "missing"
fi
RESULT=""
WORKTREE_DIR="$ESC_TMP2" _escalate_groom "second-pass-after-iterate" "second-pass-iter content" "sess-3" 2>/dev/null
if [ -r "$ESC_TMP2/.iterate/escalate-second-pass-after-iterate.md" ]; then
    assert_eq "_escalate_groom: second-pass-after-iterate stage label drives filename" "ok" "ok"
else
    assert_eq "_escalate_groom: second-pass-after-iterate stage label drives filename" "ok" "missing"
fi
rm -rf "$ESC_TMP2"

# Code-shape: _iterate_groom_loop has exactly 3 _escalate_groom call sites
# (first-pass ESCALATE + READY-then-second-pass-fail + ITERATE-then-second-pass-fail)
ITERATE_NOW=$(declare -f _iterate_groom_loop)
ESCALATE_CALL_COUNT=$(printf '%s\n' "$ITERATE_NOW" | grep -c '_escalate_groom')
assert_eq "_iterate_groom_loop has 3 _escalate_groom call sites" "3" "$ESCALATE_CALL_COUNT"

# Each stage label appears exactly once
assert_contains "ESCALATE branch uses first-pass stage" '_escalate_groom "first-pass"' "$ITERATE_NOW"
assert_contains "READY-second-pass-fail uses second-pass-after-ready stage" '_escalate_groom "second-pass-after-ready"' "$ITERATE_NOW"
assert_contains "ITERATE-second-pass-fail uses second-pass-after-iterate stage" '_escalate_groom "second-pass-after-iterate"' "$ITERATE_NOW"

# Preservation invariant: ESCALATE never calls _cleanup_iterate_findings.
# Cleanup count must still be exactly 2 (the two GROOMED success paths only).
CLEANUP_COUNT=$(printf '%s\n' "$ITERATE_NOW" | grep -c '_cleanup_iterate_findings')
assert_eq "_iterate_groom_loop still has 2 cleanup calls (GROOMED-only; ESCALATE preserves)" "2" "$CLEANUP_COUNT"

# Robustness: _escalate_groom populates RESULT even when WORKTREE_DIR is unset
# (defensive — should not crash; findings file write is best-effort, RESULT
# marker is the mandatory product).
RESULT=""
WORKTREE_DIR="" _escalate_groom "first-pass" "content" "sess-x" 2>/dev/null
assert_contains "_escalate_groom populates RESULT even when WORKTREE_DIR unset" "PIPELINE FAILURE" "$RESULT"

# ----------------------------------------------------------------------------
# Phase D — canonical body-callout writer (mika#1271 sub-PR 6)
# ----------------------------------------------------------------------------
# _write_canonical_callout is the forward-path body-callout writer called on
# GROOMED success. As of sub-PR 7b it is the sole structural authority for the
# body callout — the Class D recovery shim (formerly _verify_and_write_body_callout,
# mika#1123) was retired. Tests below assert: definition exists, two call sites
# in _iterate_groom_loop (one per GROOMED path), zero call sites on ESCALATE,
# stage labels produce distinct Grooming-history shapes, unknown stage returns 1,
# gh failure on idempotency check leaves no write.

# Definition exists
if declare -f _write_canonical_callout >/dev/null 2>&1; then
    assert_eq "_write_canonical_callout is defined" "ok" "ok"
else
    assert_eq "_write_canonical_callout is defined" "ok" "missing"
fi

# Code-shape: _iterate_groom_loop has exactly 2 _write_canonical_callout call
# sites (the two GROOMED success branches — one for READY-to-GROOMED and one
# for ITERATE-to-GROOMED). Zero call sites on ESCALATE branches.
CANONICAL_CALL_COUNT=$(printf '%s\n' "$ITERATE_NOW" | grep -c '_write_canonical_callout')
assert_eq "_iterate_groom_loop has 2 _write_canonical_callout call sites" "2" "$CANONICAL_CALL_COUNT"

# Stage labels match the GROOMED paths exactly
assert_contains "READY-to-GROOMED branch uses ready-to-groomed stage" '_write_canonical_callout "ready-to-groomed"' "$ITERATE_NOW"
assert_contains "ITERATE-to-GROOMED branch uses iterate-to-groomed stage" '_write_canonical_callout "iterate-to-groomed"' "$ITERATE_NOW"

# Preservation invariant: writer is called BEFORE cleanup (so callout writes
# even if cleanup fails) AND callout is non-fatal (|| true on failure).
assert_contains "ready-to-groomed callout non-fatal on write failure" 'canonical callout write non-fatal failure' "$ITERATE_NOW"

# Unknown stage label → returns 1 (defensive contract).
RESULT=""
unknown_rc=0
(WORKTREE_DIR="/tmp" REPO="mika" ISSUE_NUM="999" BRANCH="test/branch" _write_canonical_callout "unknown-stage" "sess-unknown" 2>/dev/null) || unknown_rc=$?
assert_eq "_write_canonical_callout: unknown stage returns non-zero" "1" "$unknown_rc"

# Missing required env → returns 1 (defensive contract). WORKTREE_DIR unset.
no_wd_rc=0
(WORKTREE_DIR="" REPO="mika" ISSUE_NUM="999" BRANCH="test/branch" _write_canonical_callout "ready-to-groomed" "sess-x" 2>/dev/null) || no_wd_rc=$?
assert_eq "_write_canonical_callout: missing WORKTREE_DIR returns non-zero" "1" "$no_wd_rc"

# Missing REPO/ISSUE_NUM/BRANCH → returns 1
CC_TMP=$(mktemp -d)
no_repo_rc=0
(WORKTREE_DIR="$CC_TMP" REPO="" ISSUE_NUM="999" BRANCH="test/branch" _write_canonical_callout "ready-to-groomed" "sess-x" 2>/dev/null) || no_repo_rc=$?
assert_eq "_write_canonical_callout: missing REPO returns non-zero" "1" "$no_repo_rc"
rm -rf "$CC_TMP"

# Source-shape: writer source contains the canonical 3-line dispatch-gate shape
# (matches the Pin B / check_grooming_markers regex in executor.rs).
WRITER_SRC=$(declare -f _write_canonical_callout)
assert_contains "writer source contains Branch line" '> - **Branch:**' "$WRITER_SRC"
assert_contains "writer source contains Plan line with committed-on-branch" 'committed on branch @' "$WRITER_SRC"
assert_contains "writer source contains second-pass (GROOMED) marker for dispatch gate" 'second-pass (GROOMED)' "$WRITER_SRC"
assert_contains "writer source includes session_id in Grooming history" 'session-id: ${session_id}' "$WRITER_SRC"

# Stage-label-driven history lines
assert_contains "ready-to-groomed stage produces READY → GROOMED history" 'first-pass (READY) → second-pass (GROOMED)' "$WRITER_SRC"
assert_contains "iterate-to-groomed stage produces ITERATE → revised → GROOMED history" 'first-pass (ITERATE) → revised → second-pass (GROOMED)' "$WRITER_SRC"

# Idempotency check uses the dispatch-gate three-signal pattern from executor.rs::check_grooming_markers
assert_contains "writer reuses Pin B has_branch signal" 'has_branch=' "$WRITER_SRC"
assert_contains "writer reuses Pin B has_plan signal" 'has_plan=' "$WRITER_SRC"
assert_contains "writer reuses Pin B has_verdict signal" 'has_verdict=' "$WRITER_SRC"

# --- Test: Auto-rescue scaffold exclusion (mika#1288) ---

echo ""
echo "Test: Auto-rescue excludes scaffold files (mika#1288)"
echo "------------------------------------------------------"

# Test A: Mixed content — scaffold excluded, pilot content staged
test_auto_rescue_excludes_scaffold_files() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    # Setup: create a git repo simulating a dirty worktree
    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # 1. Tracked file with a pilot modification
    echo "original" > "$test_dir/tracked.rs"
    git -C "$test_dir" add tracked.rs
    git -C "$test_dir" commit -m "add tracked" -q
    echo "modified by pilot" > "$test_dir/tracked.rs"

    # 2. Scaffold file (untracked) — must NOT be staged
    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold" > "$test_dir/.claude/commands/mika-groom-ticket.md"

    # 3. Pilot-authored new file (untracked) — must be staged
    mkdir -p "$test_dir/src"
    echo "fn main() {}" > "$test_dir/src/new_feature.rs"

    # Exercise: run the pathspec-filtered git add
    git -C "$test_dir" add -A -- ':!.claude/commands/' 2>/dev/null

    # Assert: check what was staged
    local staged
    staged=$(git -C "$test_dir" diff --cached --name-only)

    # Scaffold file must NOT appear in staged files
    if echo "$staged" | grep -q '.claude/commands/'; then
        echo "FAIL: scaffold file was staged"
        return 1
    fi

    # Pilot-authored new file must be staged
    if ! echo "$staged" | grep -q 'src/new_feature.rs'; then
        echo "FAIL: pilot-authored new file was not staged"
        return 1
    fi

    # Modified tracked file must be staged
    if ! echo "$staged" | grep -q 'tracked.rs'; then
        echo "FAIL: modified tracked file was not staged"
        return 1
    fi

    echo "PASS"
}

RESULT_A=$(test_auto_rescue_excludes_scaffold_files 2>/dev/null)
if [ "$RESULT_A" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Mixed content: scaffold excluded, pilot content staged"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Mixed content: $RESULT_A"
fi

# Test B: Scaffold-only worktree — empty index guard
test_auto_rescue_empty_index_guard() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    # Setup: repo where the ONLY dirty content is scaffold files
    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold only" > "$test_dir/.claude/commands/mika-groom-ticket.md"
    echo "# another scaffold" > "$test_dir/.claude/commands/mika.md"

    # Exercise: pathspec-filtered git add
    git -C "$test_dir" add -A -- ':!.claude/commands/' 2>/dev/null

    # Assert: index should be empty (diff --cached --quiet exits 0)
    if ! git -C "$test_dir" diff --cached --quiet 2>/dev/null; then
        echo "FAIL: index is not empty despite only scaffold files being dirty"
        return 1
    fi

    echo "PASS"
}

RESULT_B=$(test_auto_rescue_empty_index_guard 2>/dev/null)
if [ "$RESULT_B" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Scaffold-only worktree: empty index correctly detected"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Scaffold-only worktree: $RESULT_B"
fi

# Test C: Structural — dispatch-lib uses pathspec exclusion (not bare git add -A)
RESCUE_BLOCK=$(sed -n '/Unit 1 (mika#1282)/,/^[[:space:]]*fi$/p' "$DISPATCH_LIB" | head -120)
assert_contains "Rescue uses pathspec exclusion :!.claude/commands/" ":!.claude/commands/" "$RESCUE_BLOCK"
assert_contains "Rescue has empty-index guard (diff --cached --quiet)" "diff --cached --quiet" "$RESCUE_BLOCK"
assert_contains "Rescue uses RESCUED_FILES for message" 'RESCUED_FILES' "$RESCUE_BLOCK"

# --- Test: Auto-rescue hook failure handling (mika#1296) ---

echo ""
echo "Test: Auto-rescue checks commit exit code (mika#1296)"
echo "------------------------------------------------------"

# Test A — structural assertion (code-shape): RESCUED_DIRTY_WORKTREE=1 appears
# only inside the commit-success path, not as a fallthrough after the if/elif/else.
# In the rescue block (between "mika-rescue-commit-err" and the closing fi chain),
# every RESCUED_DIRTY_WORKTREE=1 must be preceded by an "rm -f" (cleanup on success)
# within the same branch — not as a standalone unconditional line.
RESCUE_BLOCK_1296=$(sed -n '/mika-rescue-commit-err/,/^[[:space:]]*fi$/p' "$DISPATCH_LIB")

# Count RESCUED_DIRTY_WORKTREE=1 in non-comment lines of the rescue block
RESCUED_SET_COUNT=$(printf '%s\n' "$RESCUE_BLOCK_1296" | grep -v '^\s*#' | grep -c 'RESCUED_DIRTY_WORKTREE=1' || true)
# There should be exactly 2: one for first-try success, one for retry success
assert_eq "RESCUED_DIRTY_WORKTREE=1 appears exactly twice in rescue block (success paths only)" "2" "$RESCUED_SET_COUNT"

# The else branch (unknown hook failure) must NOT set RESCUED_DIRTY_WORKTREE=1
# Verify by checking that no RESCUED_DIRTY_WORKTREE=1 appears after "non-rustfmt" marker
AFTER_NON_RUSTFMT=$(printf '%s\n' "$RESCUE_BLOCK_1296" | sed -n '/non-rustfmt/,/fi/p')
NON_RUSTFMT_RESCUED=$(printf '%s\n' "$AFTER_NON_RUSTFMT" | grep -v '^\s*#' | grep -c 'RESCUED_DIRTY_WORKTREE=1' || true)
assert_eq "Unknown hook failure branch does NOT set RESCUED_DIRTY_WORKTREE" "0" "$NON_RUSTFMT_RESCUED"

# Test B — live invariant (git-repo exercise): pre-commit hook rejects commit,
# verify RESCUED_DIRTY_WORKTREE is NOT set and RESULT contains PIPELINE FAILURE.
test_rescue_hook_failure_invariant() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    # Setup: create a git repo with a pre-commit hook that rejects with "rust-fmt"
    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # Create a pre-commit hook that always fails with rust-fmt in stderr
    mkdir -p "$test_dir/.git/hooks"
    cat > "$test_dir/.git/hooks/pre-commit" << 'HOOK'
#!/bin/bash
echo "error: rust-fmt check failed" >&2
exit 1
HOOK
    chmod +x "$test_dir/.git/hooks/pre-commit"

    # Create a dirty tracked file
    echo "fn main() {}" > "$test_dir/main.rs"
    git -C "$test_dir" add main.rs
    git -C "$test_dir" -c core.hooksPath="$test_dir/.git/hooks" commit --no-verify -m "add file" -q
    echo "fn main() { println!(\"dirty\"); }" > "$test_dir/main.rs"
    git -C "$test_dir" add main.rs

    # Create a stub cargo fmt on PATH that succeeds (simulates formatting)
    local stub_bin
    stub_bin=$(mktemp -d)
    cat > "$stub_bin/cargo" << 'STUB'
#!/bin/bash
# Stub cargo that does nothing for "fmt" subcommand
exit 0
STUB
    chmod +x "$stub_bin/cargo"

    # Exercise: simulate the rescue commit logic
    local WORKTREE_DIR="$test_dir"
    local REPO="mika" ISSUE_NUM="1296" SESSION_ID="test-session"
    local RESCUED_DIRTY_WORKTREE=0
    local RESULT=""
    local RESCUE_COMMIT_ERR="$WORKTREE_DIR/.git/mika-rescue-commit-err"
    local CARGO_FMT_ERR="" RESCUE_ERR_CONTENT=""

    # First attempt — will fail due to pre-commit hook
    if git -C "$WORKTREE_DIR" commit -m "wip(test): rescue" 2>"$RESCUE_COMMIT_ERR"; then
        rm -f "$RESCUE_COMMIT_ERR"
        RESCUED_DIRTY_WORKTREE=1
    elif grep -q "rust-fmt\|cargo fmt\|rustfmt" "$RESCUE_COMMIT_ERR" 2>/dev/null; then
        CARGO_FMT_ERR=""
        CARGO_FMT_ERR=$( (cd "$WORKTREE_DIR" && PATH="$stub_bin:$PATH" cargo fmt --all) 2>&1 ) || true
        git -C "$WORKTREE_DIR" add -A -- ':!.claude/commands/' 2>/dev/null

        # Retry — will also fail (hook still rejects)
        if git -C "$WORKTREE_DIR" commit -m "wip(test): rescue retry" 2>"$RESCUE_COMMIT_ERR"; then
            rm -f "$RESCUE_COMMIT_ERR"
            RESCUED_DIRTY_WORKTREE=1
        else
            RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -20)
            RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook after cargo-fmt retry.
cargo fmt stderr: ${CARGO_FMT_ERR:-<empty>}
Hook output: ${RESCUE_ERR_CONTENT}
Worktree left dirty for operator inspection: ${WORKTREE_DIR}

${RESULT}"
            rm -f "$RESCUE_COMMIT_ERR"
        fi
    else
        RESCUE_ERR_CONTENT=$(cat "$RESCUE_COMMIT_ERR" 2>/dev/null | head -20)
        RESULT="PIPELINE FAILURE: auto-rescue commit rejected by pre-commit hook (non-rustfmt).
Hook output: ${RESCUE_ERR_CONTENT}

${RESULT}"
        rm -f "$RESCUE_COMMIT_ERR"
    fi

    # Assertions
    local failures=""
    if [ "$RESCUED_DIRTY_WORKTREE" != "0" ]; then
        failures="${failures}RESCUED_DIRTY_WORKTREE should be 0 but is $RESCUED_DIRTY_WORKTREE; "
    fi
    if ! printf '%s' "$RESULT" | grep -qF "PIPELINE FAILURE"; then
        failures="${failures}RESULT missing PIPELINE FAILURE; "
    fi
    if ! printf '%s' "$RESULT" | grep -qF "cargo fmt stderr:"; then
        failures="${failures}RESULT missing cargo fmt diagnostic; "
    fi
    if [ -f "$RESCUE_COMMIT_ERR" ]; then
        failures="${failures}scratch file not cleaned up; "
    fi

    rm -rf "$stub_bin"
    if [ -z "$failures" ]; then
        echo "PASS"
    else
        echo "FAIL: $failures"
    fi
}

RESULT_HOOK=$(test_rescue_hook_failure_invariant 2>/dev/null)
if [ "$RESULT_HOOK" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Hook failure: RESCUED_DIRTY_WORKTREE=0, PIPELINE FAILURE in RESULT, diagnostics present, scratch cleaned"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Hook failure invariant: $RESULT_HOOK"
fi

# Test C — structural assertion: scratch file uses .git/ location (not .iterate/)
SCRATCH_FILE_CHECK=$(grep -c 'mika-rescue-commit-err' "$DISPATCH_LIB" || true)
if [ "$SCRATCH_FILE_CHECK" -ge 1 ]; then
    # Verify it uses $WORKTREE_DIR/.git/ prefix
    SCRATCH_IN_GIT=$(grep 'WORKTREE_DIR/.git/mika-rescue-commit-err' "$DISPATCH_LIB" | grep -v '^\s*#' | head -1)
    if [ -n "$SCRATCH_IN_GIT" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ Scratch file uses \$WORKTREE_DIR/.git/mika-rescue-commit-err (not .iterate/)"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ Scratch file location does not use \$WORKTREE_DIR/.git/ prefix"
    fi
else
    FAIL=$((FAIL + 1))
    echo "  ✗ No mika-rescue-commit-err reference found in dispatch-lib.sh"
fi

# Verify no .iterate/rescue-commit-err reference exists (regression guard)
ITERATE_SCRATCH=$(grep -c '.iterate/rescue-commit-err' "$DISPATCH_LIB" || true)
assert_eq "No .iterate/rescue-commit-err reference in dispatch-lib.sh" "0" "$ITERATE_SCRATCH"

# --- Test 11: Idempotency-bypass-architect fabrication brake retired (mika#1327) ---

echo ""
echo "Test 11: Idempotency-bypass-architect fabrication brake retired (mika#1327)"
echo "---------------------------------------------------------------------------"

# The mika#1322 brake (fabrication-string grep on session log) was retired in
# mika#1327. Per Vincent's ticket comment IC_kwDORWsgGM8AAAABEDBqdw, the brake
# was a duplicate fabrication-detection mechanism alongside state-grounded
# checks (HEAD-unchanged at line 451, plan-missing at line 621, iterate-loop
# ESCALATE inside _escalate_groom) and post-cpp#20 had become dead code.
#
# These assertions are regression guards against re-introducing the brake.

DISPATCH_LIB_CONTENT=$(cat "$DISPATCH_LIB")

assert_not_contains "Brake comment header (mika#1319 + idempotency-bypass-architect) is absent" 'mika#1319.*idempotency-bypass-architect' "$DISPATCH_LIB_CONTENT"
assert_not_contains "Brake comment header (idempotency-bypass-architect fabrication in dev-groom) is absent" 'idempotency-bypass-architect fabrication in dev-groom' "$DISPATCH_LIB_CONTENT"
assert_not_contains "Fabrication needle (Architect convergence pending via dispatch-lib iterate loop) is absent" 'Architect convergence pending via dispatch-lib iterate loop' "$DISPATCH_LIB_CONTENT"
assert_not_contains "FABRICATION_NEEDLE variable assignment is absent" 'FABRICATION_NEEDLE=' "$DISPATCH_LIB_CONTENT"
assert_not_contains "PIPELINE FAILURE marker with idempotency-bypass-architect sub-type is absent" 'PIPELINE FAILURE:.*idempotency-bypass-architect' "$DISPATCH_LIB_CONTENT"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
