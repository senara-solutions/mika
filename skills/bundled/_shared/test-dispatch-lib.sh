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
#
# Historical note (mika#1412): the previous form of this test asserted four
# specific strings (`auto_skipped`, `Do not ask the operator`, `Do not post
# a status message`, `mika#988`) about the `self-dev/system_prompt.md`
# auto-skip section. That section has been refactored out of the prompt
# entirely as the auto-skip handling moved to engine-level guards
# (validate_dispatch_readiness in `mika-agent`) — the prompt now describes
# the rejection-handling shape via typed `pr_merge_with_gate` variants
# (mika#1326-era refactor) and the `dispatch_task_has_open_pr` rejection
# (verified at prompt line 87). The four old assertions are removed per
# mika#1412 AC1 ("removed if it asserts retired behavior").
#
# Current invariant worth asserting: the prompt still requires the agent
# to consult the operator before retrying after a dispatch rejection — the
# engine guard is authoritative, but the prompt-level discipline backs it.

echo ""
echo "Test 5: self-dev prompt — dispatch-rejection handling discipline (mika#1412 refresh)"
echo "-----------------------------------------------------------------------------------"

PROMPT_FILE="$SCRIPT_DIR/../self-dev/system_prompt.md"
if [ -f "$PROMPT_FILE" ]; then
    # Invariant: prompt instructs the agent to wait for explicit operator
    # direction after a dispatch rejection — defense-in-depth against
    # auto-retry loops on rejected dispatches.
    if grep -qF "Wait for explicit instructions" "$PROMPT_FILE"; then
        PASS=$((PASS + 1)); echo "  ✓ Prompt requires explicit operator direction after dispatch rejection"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ Prompt requires explicit operator direction after dispatch rejection"
    fi

    # Invariant: prompt cites the structural enforcement point so the
    # prompt-level discipline names where the authoritative gate lives.
    if grep -qF "validate_dispatch_readiness" "$PROMPT_FILE"; then
        PASS=$((PASS + 1)); echo "  ✓ Prompt names the authoritative engine guard (validate_dispatch_readiness)"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ Prompt names the authoritative engine guard"
    fi

    # Invariant: prompt enumerates the typed rejection sub-variants the
    # agent must branch on exhaustively (mika#1326-era structural fix).
    if grep -qF "dispatch_task_has_open_pr" "$PROMPT_FILE"; then
        PASS=$((PASS + 1)); echo "  ✓ Prompt enumerates dispatch_task_has_open_pr rejection variant"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ Prompt enumerates dispatch_task_has_open_pr rejection variant"
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

# Verify it overrides ENTRY_COMMAND to /ce-work
# (compound-engineering 3.x renamed `ce:work` → `ce-work` per plugin CHANGELOG #503; mika#1345)
assert_contains "Overrides ENTRY_COMMAND to /ce-work" '/ce-work' "$PLAN_FUNC"

# Verify the function is called in dispatch_claude_pilot after _set_up_worktree
# and before _handle_dry_run
DISPATCH_BODY=$(sed -n '/^dispatch_claude_pilot()/,/^}/p' "$DISPATCH_LIB")
# Extract the call sequence: _set_up_worktree, _detect_plan_on_branch, _handle_dry_run
CALL_SEQUENCE=$(printf '%s\n' "$DISPATCH_BODY" | grep -E '^\s+_(set_up_worktree|detect_plan_on_branch|handle_dry_run)' | tr -s ' ' | sed 's/^ //')
EXPECTED_SEQUENCE=$(printf '%s\n' "_set_up_worktree" "_detect_plan_on_branch" "_handle_dry_run")
assert_eq "Call ordering: _set_up_worktree -> _detect_plan_on_branch -> _handle_dry_run" "$EXPECTED_SEQUENCE" "$CALL_SEQUENCE"

# Verify the case switch maps both skills correctly. As of mika#1271 sub-PR 8
# (referenced in dispatch-lib.sh around line 1685) the case block is now
# multi-line per skill and dev-groom maps to `/mika-groom-plan-only`
# (content-only) — the iterate-groom-loop + canonical body-callout writer
# took over the architect-convergence work that `/mika-groom-ticket` used to
# handle in the autonomous flow. Operator-facing `/mika-groom-ticket` is
# unchanged but is invoked by hand, not by dev-groom. (mika#1412)
CASE_BLOCK=$(sed -n '/case "\$SKILL" in/,/esac/p' "$DISPATCH_LIB")
assert_contains "Case switch still maps dev-pilot to /mika" 'ENTRY_COMMAND="/mika"' "$CASE_BLOCK"
assert_contains "Case switch maps dev-groom to /mika-groom-plan-only (mika#1271 sub-PR 8)" 'ENTRY_COMMAND="/mika-groom-plan-only"' "$CASE_BLOCK"
# Defensive: the old single-line shape `dev-pilot)  ENTRY_COMMAND="/mika"`
# should be absent — if a refactor re-collapses the case block to one-liners
# this regression test should fire on the structural shape, not just the
# string match above.
assert_not_contains "Case block is multi-line (no inlined dev-groom mapping to /mika-groom-ticket)" 'dev-groom)  ENTRY_COMMAND="/mika-groom-ticket"' "$CASE_BLOCK"

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
# even if cleanup fails) AND callout is non-fatal (|| <fallback>). The
# non-fatal shape evolved (mika#1412): older code used a `|| true`
# fall-through; current code uses `|| echo "WARN: canonical_callout_failed ..."`
# so the operator sees a diagnostic if the writer fails. Assertion updated
# to match the current canonical_callout_failed warning string.
assert_contains "ready-to-groomed callout non-fatal on write failure (canonical_callout_failed warn)" 'canonical_callout_failed' "$ITERATE_NOW"

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

# Test C (mika#1419): claude-pilot.json scaffold exclusion — relay-config
# file is cp'd from $PLATFORM_DIR by _set_up_worktree at line 489 and is NOT
# pilot-authored content. The rescue commit must exclude it via the same
# pathspec mechanism mika#1288 uses for .claude/commands/. Without this
# exclusion, PR #1348's intentional deletion (mika#1193 Phase C) is silently
# re-introduced on every WIP-rescue flow — the ping-pong on the file's git
# log is the founding incident for mika#1419.
test_auto_rescue_excludes_claude_pilot_json() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # 1. claude-pilot.json scaffold file (untracked, cp'd from meta-repo) —
    #    must NOT be staged by the rescue commit.
    mkdir -p "$test_dir/.claude"
    echo '{"command":"mika","args":["--agent","mika-relay","ask"]}' > "$test_dir/.claude/claude-pilot.json"

    # 2. .claude/commands/ scaffold file — must NOT be staged (mika#1288).
    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold" > "$test_dir/.claude/commands/mika-groom-ticket.md"

    # 3. Pilot-authored new file — must be staged.
    mkdir -p "$test_dir/src"
    echo "fn main() {}" > "$test_dir/src/new_feature.rs"

    # Exercise: the mika#1419 pathspec exclusion.
    git -C "$test_dir" add -A -- ':!.claude/commands/' ':!.claude/claude-pilot.json' 2>/dev/null

    local staged
    staged=$(git -C "$test_dir" diff --cached --name-only)

    # claude-pilot.json must NOT appear in staged files (mika#1419 regression).
    if echo "$staged" | grep -q '.claude/claude-pilot.json'; then
        echo "FAIL: .claude/claude-pilot.json was staged (mika#1419 ping-pong reintroduced)"
        return 1
    fi

    # .claude/commands/ scaffold must NOT appear (mika#1288 regression).
    if echo "$staged" | grep -q '.claude/commands/'; then
        echo "FAIL: .claude/commands/ scaffold was staged (mika#1288 regression)"
        return 1
    fi

    # Pilot-authored file MUST be staged.
    if ! echo "$staged" | grep -q 'src/new_feature.rs'; then
        echo "FAIL: pilot-authored file was not staged"
        return 1
    fi

    echo "PASS"
}

RESULT_C=$(test_auto_rescue_excludes_claude_pilot_json 2>/dev/null)
if [ "$RESULT_C" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ claude-pilot.json scaffold excluded (mika#1419 ping-pong regression)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ claude-pilot.json scaffold exclusion: $RESULT_C"
fi

# Test D (mika#1419): scaffold-only worktree empty-index guard includes
# claude-pilot.json. Verifies the empty-index path correctly skips the rescue
# commit when both scaffold types are the only dirty content.
test_auto_rescue_empty_index_guard_with_claude_pilot_json() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # Only scaffold dirt: both commands/ AND claude-pilot.json — no pilot content.
    mkdir -p "$test_dir/.claude/commands"
    echo "# scaffold" > "$test_dir/.claude/commands/mika.md"
    echo '{"command":"mika","args":["--agent","mika-relay","ask"]}' > "$test_dir/.claude/claude-pilot.json"

    git -C "$test_dir" add -A -- ':!.claude/commands/' ':!.claude/claude-pilot.json' 2>/dev/null

    if ! git -C "$test_dir" diff --cached --quiet 2>/dev/null; then
        echo "FAIL: index is not empty despite only scaffold-class files being dirty"
        return 1
    fi

    echo "PASS"
}

RESULT_D=$(test_auto_rescue_empty_index_guard_with_claude_pilot_json 2>/dev/null)
if [ "$RESULT_D" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Empty-index guard covers both scaffold classes (mika#1288 + mika#1419)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Empty-index guard with both scaffolds: $RESULT_D"
fi

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
    # mika#1341: mirror the production scratch path (mktemp), not the old "$WORKTREE_DIR/.git/"
    # location. This harness uses a non-linked `git init` repo (where .git is a directory), so
    # either path would open here — but the mirror should track production shape for fidelity.
    local RESCUE_COMMIT_ERR="$(mktemp)"
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

# Test C — structural assertion (mika#1341): the rescue scratch path MUST NOT live
# under "$WORKTREE_DIR/.git/". In a linked worktree (every autonomous dev-pilot run)
# ".git" is a FILE, so a redirect into "$WORKTREE_DIR/.git/<name>" fails (ENOTDIR),
# the rescue `git commit` never runs, and the loop wedges. The active assignment must
# use mktemp (off the working tree, valid in linked + non-linked checkouts).
# This assertion FAILS against the pre-mika#1341 ".git/"-scratch code.
RESCUE_ASSIGN=$(grep 'RESCUE_COMMIT_ERR=' "$DISPATCH_LIB" | grep -v '^\s*#')
ACTIVE_ASSIGN=$(printf '%s\n' "$RESCUE_ASSIGN" | grep 'mktemp' || true)
GIT_FILE_ASSIGN=$(printf '%s\n' "$RESCUE_ASSIGN" | grep 'WORKTREE_DIR/.git/mika-rescue-commit-err' || true)
if [ -n "$ACTIVE_ASSIGN" ] && [ -z "$GIT_FILE_ASSIGN" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Rescue scratch path uses mktemp, not \$WORKTREE_DIR/.git/ (mika#1341)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Rescue scratch path must use mktemp and must NOT use \$WORKTREE_DIR/.git/ (mika#1341)"
    echo "    active(mktemp): ${ACTIVE_ASSIGN:-<none>}"
    echo "    git-file path:  ${GIT_FILE_ASSIGN:-<none>}"
fi

# Verify no .iterate/rescue-commit-err reference exists (regression guard)
ITERATE_SCRATCH=$(grep -c '.iterate/rescue-commit-err' "$DISPATCH_LIB" || true)
assert_eq "No .iterate/rescue-commit-err reference in dispatch-lib.sh" "0" "$ITERATE_SCRATCH"

# Test D — live invariant (mika#1341): the rescue commit must succeed inside a real
# LINKED worktree. Reproduces the root cause as an executable artifact: in a linked
# worktree ".git" is a file, a redirect into "<wt>/.git/<name>" fails (ENOTDIR), but
# a commit capturing into an mktemp path lands and advances HEAD.
#
# This is a standalone root-cause PROOF — it reimplements the rescue commit with its own
# mktemp scratch and does NOT source dispatch-lib.sh, so it passes regardless of the
# production code. The source-coupled regression guard is Test C above (which asserts the
# production RESCUE_COMMIT_ERR assignment uses mktemp, not "$WORKTREE_DIR/.git/"). Do not
# delete Test C assuming Test D covers the regression — Test D would still pass on buggy code.
test_rescue_linked_worktree_invariant() {
    local base_dir wt_dir
    base_dir=$(mktemp -d)
    wt_dir="${base_dir}-wt"
    trap "git -C '$base_dir' worktree remove --force '$wt_dir' 2>/dev/null; rm -rf '$base_dir' '$wt_dir'" RETURN

    # Base repo with an initial commit + an identity so commits work in CI.
    git -C "$base_dir" init -q
    git -C "$base_dir" config user.email "test@mika.local"
    git -C "$base_dir" config user.name "mika test"
    git -C "$base_dir" commit --allow-empty -m "initial" -q

    # Create a LINKED worktree on a new branch.
    git -C "$base_dir" worktree add -q -b rescue-test "$wt_dir" 2>/dev/null

    local failures=""

    # (1) In a linked worktree, .git is a FILE, not a directory.
    if [ ! -f "$wt_dir/.git" ]; then
        failures="${failures}linked worktree .git is not a file; "
    fi

    # (2) The OLD path construction fails to open (documents the ENOTDIR root cause).
    if ( echo probe > "$wt_dir/.git/mika-rescue-commit-err" ) 2>/dev/null; then
        failures="${failures}redirect into <wt>/.git/<name> unexpectedly succeeded; "
    fi

    # (3) The FIXED approach: capture into an mktemp path; the rescue commit lands.
    local pre_head post_head scratch
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)
    echo "pilot wrote this but never committed" > "$wt_dir/impl.txt"
    git -C "$wt_dir" add -A -- ':!.claude/commands/' 2>/dev/null
    scratch="$(mktemp)"
    if git -C "$wt_dir" commit -m "wip(mika#1341): rescue in linked worktree" > "$scratch" 2>&1; then
        post_head=$(git -C "$wt_dir" rev-parse HEAD)
        [ "$pre_head" != "$post_head" ] || failures="${failures}HEAD did not advance after rescue commit; "
        git -C "$wt_dir" diff --quiet HEAD -- impl.txt || failures="${failures}impl.txt not committed; "
    else
        failures="${failures}rescue commit failed with mktemp scratch path; "
    fi
    rm -f "$scratch"
    [ ! -f "$scratch" ] || failures="${failures}scratch not cleaned; "

    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_LWT=$(test_rescue_linked_worktree_invariant 2>/dev/null)
if [ "$RESULT_LWT" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Linked-worktree rescue: .git is a file, mktemp scratch lands the commit (mika#1341)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Linked-worktree rescue invariant: $RESULT_LWT"
fi

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
    if ! printf '%s' "$remote_url" | grep -q "^file://\|^/"; then
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
    if ! printf '%s' "$RESULT" | grep -qF "Push: pushed"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    # Assert: mode is first-push (not diverged).
    if printf '%s' "$RESULT" | grep -qF "mode=diverged"; then
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
    if ! printf '%s' "$RESULT" | grep -qF "Push: pushed"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    if ! printf '%s' "$RESULT" | grep -qF "mode=fast-forward"; then
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
    if ! printf '%s' "$RESULT" | grep -qF "Push: pushed"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    if ! printf '%s' "$RESULT" | grep -qF "mode=diverged"; then
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
PUSH_FUNC_SRC=$(declare -f _push_branch)
# Verify explicit lease form (pins expected remote SHA)
if printf '%s' "$PUSH_FUNC_SRC" | grep -q 'force-with-lease=.*BRANCH.*origin.*BRANCH'; then
    PASS=$((PASS + 1)); echo "  ✓ Lease form: --force-with-lease uses explicit ref pinning"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Lease form: should use --force-with-lease=\$BRANCH:origin/\$BRANCH"
fi
# Verify no blind --force (without -with-lease)
if printf '%s' "$PUSH_FUNC_SRC" | grep -q -- '--force[^-]'; then
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
    if ! printf '%s' "$RESULT" | grep -qF "STATUS=REBASE_CONFLICT"; then
        failures="${failures}RESULT missing STATUS=REBASE_CONFLICT; "
    fi
    # Assert: conflict mode token present (AC#4).
    if ! printf '%s' "$RESULT" | grep -qF "Rebase failure mode: conflict"; then
        failures="${failures}RESULT missing 'Rebase failure mode: conflict'; "
    fi
    # Assert: conflicted filename present.
    if ! printf '%s' "$RESULT" | grep -qF "file.txt"; then
        failures="${failures}RESULT missing conflicted filename 'file.txt'; "
    fi
    # Assert: rebase stderr is non-empty (AC#4 — not /dev/null).
    if printf '%s' "$RESULT" | grep -qF "Rebase stderr: <empty>"; then
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
    if ! printf '%s' "$RESULT" | grep -qF "Push: pushed"; then
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

# _push_branch uses merge-base --is-ancestor for divergence detection
PUSH_FUNC=$(declare -f _push_branch)
assert_contains "_push_branch uses merge-base --is-ancestor" "merge-base --is-ancestor" "$PUSH_FUNC"
assert_contains "_push_branch uses force-with-lease" "force-with-lease" "$PUSH_FUNC"
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
    if printf '%s' "$RESULT" | grep -qiE "divergence|abort|reconciliation|Push: pushed|Push: FAILED"; then
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

# ============================================================================
# Pre-flight stale-relic cleanup + dual-failure diagnostic (mika#1472)
# ============================================================================

echo ""
echo "Test: Pre-flight stale-relic cleanup (mika#1472 U1)"
echo "---------------------------------------------------"

# Extract the _set_up_worktree function body for structural assertions
SET_UP_WT_FUNC=$(sed -n '/^_set_up_worktree()/,/^}/p' "$DISPATCH_LIB")

# U1: Pre-flight block contains worktree list --porcelain detection
assert_contains "Pre-flight uses worktree list --porcelain" \
    'worktree list --porcelain' "$SET_UP_WT_FUNC"

# U1: Pre-flight block contains the branch-comparison awk script
assert_contains "Pre-flight awk matches refs/heads/\$BRANCH" \
    'refs/heads/$BRANCH' "$SET_UP_WT_FUNC"

# U1: Pre-flight block contains the directory-exists guard
assert_contains "Pre-flight has directory-exists guard for existing_wt" \
    '[ -d "$existing_wt" ]' "$SET_UP_WT_FUNC"

# U1: Pre-flight block contains status --porcelain check
assert_contains "Pre-flight checks dirty state via status --porcelain" \
    'status --porcelain' "$SET_UP_WT_FUNC"

# U1: Pre-flight block contains stash push -u -m with descriptive name pattern
assert_contains "Pre-flight stashes with descriptive name (stash push --include-untracked -m)" \
    'stash push --include-untracked -m "$stash_name"' "$SET_UP_WT_FUNC"

# U1: Pre-flight block contains the stale-worktree-cleanup naming convention
assert_contains "Pre-flight stash name uses dispatch-lib-stale-worktree-cleanup prefix" \
    'dispatch-lib-stale-worktree-cleanup' "$SET_UP_WT_FUNC"

# U1: Pre-flight block contains worktree remove --force for the existing_wt
assert_contains "Pre-flight removes non-canonical worktree with --force" \
    'worktree remove --force "$existing_wt"' "$SET_UP_WT_FUNC"

# U1: Call ordering — pre-flight cleanup runs BEFORE the existing dashed-path
# collision check (the "Reuse existing worktree if valid" block).
PREFLIGHT_LINE=$(printf '%s\n' "$SET_UP_WT_FUNC" | grep -n 'worktree list --porcelain' | head -1 | cut -d: -f1)
COLLISION_LINE=$(printf '%s\n' "$SET_UP_WT_FUNC" | grep -n 'Reuse existing worktree if valid' | head -1 | cut -d: -f1)
if [ -n "$PREFLIGHT_LINE" ] && [ -n "$COLLISION_LINE" ] && [ "$PREFLIGHT_LINE" -lt "$COLLISION_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Pre-flight cleanup (line $PREFLIGHT_LINE) runs before dashed-path collision check (line $COLLISION_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Pre-flight cleanup must run before dashed-path collision check"
    echo "    preflight_line=$PREFLIGHT_LINE collision_line=$COLLISION_LINE"
fi

echo ""
echo "Test: Dual-failure diagnostic (mika#1472 U2)"
echo "---------------------------------------------"

# U2: Wrapped block contains stderr-capture redirects
assert_contains "Worktree add captures stderr to wt-add-1-err temp file" \
    'wt_err_1' "$SET_UP_WT_FUNC"
assert_contains "Worktree add captures stderr to wt-add-2-err temp file" \
    'wt_err_2' "$SET_UP_WT_FUNC"

# U2: Wrapped block contains the worktree_setup_failed: diagnostic prefix
assert_contains "Dual-failure emits worktree_setup_failed: structured diagnostic" \
    'worktree_setup_failed:' "$SET_UP_WT_FUNC"

# U2: Diagnostic includes both attempts' stderr content
assert_contains "Diagnostic includes attempt 1 stderr (cat wt_err_1)" \
    'cat "$wt_err_1"' "$SET_UP_WT_FUNC"
assert_contains "Diagnostic includes attempt 2 stderr (cat wt_err_2)" \
    'cat "$wt_err_2"' "$SET_UP_WT_FUNC"

# U2: Temp files cleaned up in both success and dual-failure paths
CLEANUP_COUNT=$(printf '%s\n' "$SET_UP_WT_FUNC" | grep -c 'rm -f "$wt_err_1" "$wt_err_2"')
if [ "$CLEANUP_COUNT" -ge 2 ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Temp files cleaned up in both success and dual-failure paths ($CLEANUP_COUNT occurrences)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Temp file cleanup must appear in both success and failure paths"
    echo "    found $CLEANUP_COUNT occurrences, expected >= 2"
fi

# U2: Call ordering — worktree_setup_failed: appears AFTER the pre-flight cleanup
DIAGNOSTIC_LINE=$(printf '%s\n' "$SET_UP_WT_FUNC" | grep -n 'worktree_setup_failed:' | head -1 | cut -d: -f1)
if [ -n "$PREFLIGHT_LINE" ] && [ -n "$DIAGNOSTIC_LINE" ] && [ "$PREFLIGHT_LINE" -lt "$DIAGNOSTIC_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Pre-flight (line $PREFLIGHT_LINE) runs before dual-failure diagnostic (line $DIAGNOSTIC_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Pre-flight must run before dual-failure diagnostic"
    echo "    preflight_line=$PREFLIGHT_LINE diagnostic_line=$DIAGNOSTIC_LINE"
fi

echo ""
echo "Test: Doc comment on _set_up_worktree (mika#1472 U3)"
echo "----------------------------------------------------"

# U3: Doc comment exists above _set_up_worktree citing mika#1472
DOC_COMMENT=$(sed -n '/^# Pre-flight cleanup (mika#1472)/,/^_set_up_worktree()/p' "$DISPATCH_LIB")
assert_contains "Doc comment cites mika#1472" 'mika#1472' "$DOC_COMMENT"
assert_contains "Doc comment cites sibling mika#1414" 'mika#1414' "$DOC_COMMENT"
assert_contains "Doc comment cites sibling mika#1364" 'mika#1364' "$DOC_COMMENT"
assert_contains "Doc comment mentions worktree_setup_failed:" 'worktree_setup_failed:' "$DOC_COMMENT"

# --- Test: _label_to_type mapping (mika#1515) ---

echo ""
echo "Test: _label_to_type mapping (mika#1515)"
echo "------------------------------------------"

assert_eq "_label_to_type enhancement" "feat" "$(_label_to_type "enhancement")"
assert_eq "_label_to_type feature" "feat" "$(_label_to_type "feature")"
assert_eq "_label_to_type bug" "fix" "$(_label_to_type "bug")"
assert_eq "_label_to_type infrastructure" "chore" "$(_label_to_type "infrastructure")"
assert_eq "_label_to_type documentation" "docs" "$(_label_to_type "documentation")"
assert_eq "_label_to_type refactor" "refactor" "$(_label_to_type "refactor")"
assert_eq "_label_to_type test" "test" "$(_label_to_type "test")"
assert_eq "_label_to_type unknown label defaults to chore" "chore" "$(_label_to_type "priority:high")"
assert_eq "_label_to_type empty string defaults to chore" "chore" "$(_label_to_type "")"
assert_eq "_label_to_type comma-separated with enhancement" "feat" "$(_label_to_type "priority:high,enhancement")"
assert_eq "_label_to_type comma-separated with bug" "fix" "$(_label_to_type "component:agent,bug,ready")"

# --- Test: _derive_recovery_pr_title (mika#1515) ---

echo ""
echo "Test: _derive_recovery_pr_title (mika#1515)"
echo "----------------------------------------------"

# Set up a temporary git repo for commit-pushed-no-pr tests
_TEST_REPO_DIR=$(mktemp -d)
git -C "$_TEST_REPO_DIR" init -q 2>/dev/null
git -C "$_TEST_REPO_DIR" config user.email "test@test.com"
git -C "$_TEST_REPO_DIR" config user.name "Test"
echo "content" > "$_TEST_REPO_DIR/file.txt"
git -C "$_TEST_REPO_DIR" add file.txt
git -C "$_TEST_REPO_DIR" commit -q -m "feat(agent): add dispatch recovery (mika#1515)" 2>/dev/null

# commit-pushed-no-pr: should return the commit subject
assert_eq "commit-pushed-no-pr returns commit subject" \
    "feat(agent): add dispatch recovery (mika#1515)" \
    "$(_derive_recovery_pr_title "commit-pushed-no-pr" "$_TEST_REPO_DIR" "mika" "1515" "enhancement" "Some issue title")"

# dirty-worktree with plan file that has H1
mkdir -p "$_TEST_REPO_DIR/docs/plans"
echo '# PR titles on recovery should carry the conventional-commit subject' > "$_TEST_REPO_DIR/docs/plans/2026-06-14-005-1515-dispatch-lib-pr-titles-on-recovery-plan.md"

assert_eq "dirty-worktree with plan H1 constructs conventional title" \
    "feat: PR titles on recovery should carry the conventional-commit subject (mika#1515)" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "1515" "enhancement" "Some issue title")"

# dirty-worktree with plan H1 that already has conventional-commit format
echo '# feat(dispatch-lib): PR titles on recovery' > "$_TEST_REPO_DIR/docs/plans/2026-06-14-005-1515-dispatch-lib-pr-titles-on-recovery-plan.md"

assert_eq "dirty-worktree with conventional-commit H1 passes through" \
    "feat(dispatch-lib): PR titles on recovery" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "1515" "enhancement" "Some issue title")"

# dirty-worktree with no plan file → fallback to issue title
rm -rf "$_TEST_REPO_DIR/docs/plans"

assert_eq "dirty-worktree without plan falls back to issue title" \
    "feat: Some issue title (mika#1515)" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "1515" "enhancement" "Some issue title")"

# dirty-worktree with bug label → fix prefix
assert_eq "dirty-worktree with bug label uses fix prefix" \
    "fix: Fix broken dispatch (mika#42)" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "42" "bug" "Fix broken dispatch")"

# issue title already has conventional-commit format → pass through
assert_eq "issue title with conventional-commit passes through" \
    "fix(cli): handle edge case" \
    "$(_derive_recovery_pr_title "dirty-worktree" "$_TEST_REPO_DIR" "mika" "99" "bug" "fix(cli): handle edge case")"

# Cleanup
rm -rf "$_TEST_REPO_DIR"

# --- Test: Recovery block uses _derive_recovery_pr_title (mika#1515, structural) ---

echo ""
echo "Test: Recovery block uses _derive_recovery_pr_title (mika#1515, structural)"
echo "-----------------------------------------------------------------------------"

RECOVERY_BLOCK=$(sed -n '/Recovery classes:/,/RESCUED_PR_URL/p' "$DISPATCH_LIB")

assert_contains "dirty-worktree uses _derive_recovery_pr_title" \
    '_derive_recovery_pr_title "dirty-worktree"' "$RECOVERY_BLOCK"
assert_contains "commit-pushed-no-pr uses _derive_recovery_pr_title" \
    '_derive_recovery_pr_title "commit-pushed-no-pr"' "$RECOVERY_BLOCK"
assert_not_contains "No hardcoded wip() rescue title" \
    'rescued impl (dispatch-lib recovery)' "$RECOVERY_BLOCK"
assert_not_contains "No hardcoded pilot impl rescue title" \
    'pilot impl (dispatch-lib PR-create recovery' "$RECOVERY_BLOCK"

# --- Test 13: policy-deny disambiguation (drift-misdiagnosis fix) ---
# Mirrors investigation in docs/solutions/workflow-issues/
# 2026-06-14-dev-groom-drift-misdiagnosis-policy-deny-halt.md.
# Verifies the new POLICY_DENY pre-check exists, precedes the existing
# drift-detection chain, reads from the persistent stderr path, and emits
# a distinct message that does NOT conflate the failure with LLM drift.

echo ""
echo "Test 13: Policy-deny disambiguation precedes drift detection (structural)"
echo "-------------------------------------------------------------------------"

DRIFT_BLOCK=$(sed -n '/Post-flight plan validation/,/Issue #138: Discover/p' "$DISPATCH_LIB")

assert_contains "POLICY_DENY variable initialized" \
    'POLICY_DENY=""' "$DRIFT_BLOCK"
assert_contains "PERSISTENT_STDERR_PATH uses LOG_ID convention" \
    'PERSISTENT_STDERR_PATH="/var/log/claude-pilot/${LOG_ID}.stderr"' "$DRIFT_BLOCK"
assert_contains "Reads from persistent stderr (mika#1097 channel)" \
    '"$PERSISTENT_STDERR_PATH"' "$DRIFT_BLOCK"
assert_contains "Strips ANSI before grep (UI ANSI shouldn't break match)" \
    'sed' "$DRIFT_BLOCK"
assert_contains "Searches for [policy:deny] marker" \
    '[policy:deny]' "$DRIFT_BLOCK"
assert_contains "Class C message identifies policy halt explicitly" \
    'halted by claude-pilot policy deny' "$DRIFT_BLOCK"
assert_contains "Class C message explicitly NOT drift" \
    'not LLM drift' "$DRIFT_BLOCK"
assert_contains "Class C message links to investigation doc" \
    'drift-misdiagnosis-policy-deny-halt' "$DRIFT_BLOCK"

# Branch ordering: the POLICY_DENY branch must be the FIRST elif/if, so it wins
# over the drift messages when both conditions could fire.
POLICY_DENY_LINE=$(echo "$DRIFT_BLOCK" | grep -n 'if \[ -n "\$POLICY_DENY" \]' | head -1 | cut -d: -f1)
DRIFT_MSG_LINE=$(echo "$DRIFT_BLOCK" | grep -n 'Session drifted into executor mode' | head -1 | cut -d: -f1)
if [ -n "$POLICY_DENY_LINE" ] && [ -n "$DRIFT_MSG_LINE" ] && [ "$POLICY_DENY_LINE" -lt "$DRIFT_MSG_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ POLICY_DENY branch precedes drift message in source"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ POLICY_DENY branch must precede drift message (POLICY_DENY_LINE=$POLICY_DENY_LINE, DRIFT_MSG_LINE=$DRIFT_MSG_LINE)"
fi

# Behavioral test — extract the policy-deny check logic into a function and
# exercise it with synthetic stderr fixtures.
echo ""
echo "Test 13b: Policy-deny check (behavioral, synthetic stderr fixtures)"
echo "-------------------------------------------------------------------"

# Minimal reproduction of the policy-deny check from dispatch-lib.sh.
# Kept in sync with the version in dispatch-lib.sh via the structural
# assertions above (mirrors the mika#1364 self-test approach).
_test_policy_deny_check() {
    local stderr_path="$1"
    local policy_deny=""
    if [ -f "$stderr_path" ] && [ -r "$stderr_path" ]; then
        policy_deny=$(sed 's/\x1b\[[0-9;]*[mK]//g' "$stderr_path" 2>/dev/null \
            | grep -m1 '\[policy:deny\]' || true)
    fi
    printf '%s' "$policy_deny"
}

POLICY_DENY_FIXTURE_DIR=$(mktemp -d)
trap "rm -rf '$POLICY_DENY_FIXTURE_DIR'" EXIT

# Fixture 1: stderr with a real ANSI-coded policy-deny line (today's mika#624 shape).
printf '\x1b[31m[policy:deny]\x1b[0m \x1b[1mBash\x1b[0m: gh auth status 2>&1 | head -10\n' \
    > "$POLICY_DENY_FIXTURE_DIR/with_deny.stderr"
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/with_deny.stderr")
assert_contains "ANSI-stripped deny line extracted" \
    '[policy:deny] Bash: gh auth status' "$RESULT"

# Fixture 2: stderr without any policy-deny lines (healthy session).
printf '[init] Session abc123\n[done] Success | 5 turns | $0.20\n' \
    > "$POLICY_DENY_FIXTURE_DIR/clean.stderr"
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/clean.stderr")
assert_eq "Clean stderr yields empty POLICY_DENY" "" "$RESULT"

# Fixture 3: stderr missing entirely (fail-open, fall through to drift checks).
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/does_not_exist.stderr")
assert_eq "Missing stderr yields empty POLICY_DENY (fail-open)" "" "$RESULT"

# Fixture 4: deny line with rule-id suffix (today's mika#96 shape).
printf '\x1b[31m[policy:deny]\x1b[0m \x1b[1mBash\x1b[0m: grep -r "x" /tmp/ [bash-grep]\n' \
    > "$POLICY_DENY_FIXTURE_DIR/deny_with_rule.stderr"
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/deny_with_rule.stderr")
assert_contains "Deny line with rule-id suffix extracted" \
    '[bash-grep]' "$RESULT"

# Fixture 5: multiple deny lines — only the first should be extracted (head -1
# behavior). Operators get the actionable signal without spam.
printf '[policy:deny] Bash: cmd1\n[policy:deny] Bash: cmd2\n[policy:deny] Bash: cmd3\n' \
    > "$POLICY_DENY_FIXTURE_DIR/multi_deny.stderr"
RESULT=$(_test_policy_deny_check "$POLICY_DENY_FIXTURE_DIR/multi_deny.stderr")
assert_contains "First deny extracted" "cmd1" "$RESULT"
assert_not_contains "Subsequent denies not included" "cmd2" "$RESULT"

# --- Test 14: policy-deny disambiguation extended to dev-pilot ---
# Companion to mika#1534 (dev-groom disambiguation). The Class C policy-deny
# check now also fires on dev-pilot post-flight before the generic "Zero new
# commits" / "HEAD unchanged" message. Validates the structural placement.

echo ""
echo "Test 14: Policy-deny disambiguation extended to dev-pilot (structural)"
echo "----------------------------------------------------------------------"

POSTFLIGHT_BLOCK=$(sed -n '/Post-flight diff check: detect zero-commit/,/Unit 1 (mika#1282)/p' "$DISPATCH_LIB")

assert_contains "Class C check fires on HEAD-unchanged for ALL skills (not just dev-groom)" \
    'Policy-deny pre-check (Class C disambiguation, extended to dev-pilot' "$POSTFLIGHT_BLOCK"
assert_contains "POLICY_DENY variable set in HEAD-unchanged path" \
    'POLICY_DENY=""' "$POSTFLIGHT_BLOCK"
assert_contains "Reads persistent stderr at LOG_ID path" \
    'PERSISTENT_STDERR_PATH="/var/log/claude-pilot/${LOG_ID}.stderr"' "$POSTFLIGHT_BLOCK"
assert_contains "Strips ANSI before grep" \
    'sed' "$POSTFLIGHT_BLOCK"
assert_contains "Searches for [policy:deny] marker" \
    '[policy:deny]' "$POSTFLIGHT_BLOCK"
assert_contains "Class C message — halted by policy deny, NOT generic exit" \
    'halted by policy deny — not generic exit' "$POSTFLIGHT_BLOCK"
assert_contains "Links to investigation doc" \
    'drift-misdiagnosis-policy-deny-halt' "$POSTFLIGHT_BLOCK"

# Branch ordering: POLICY_DENY must precede BOTH the dev-groom-re-dispatch
# Note AND the generic "Zero new commits" message in source order.
POLICY_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n 'if \[ -n "\$POLICY_DENY" \]' | head -1 | cut -d: -f1)
GROOM_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n 'HEAD unchanged on dev-groom re-dispatch' | head -1 | cut -d: -f1)
ZERO_COMMITS_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n 'Zero new commits produced' | head -1 | cut -d: -f1)
if [ -n "$POLICY_LINE" ] && [ -n "$GROOM_LINE" ] && [ -n "$ZERO_COMMITS_LINE" ] \
    && [ "$POLICY_LINE" -lt "$GROOM_LINE" ] && [ "$POLICY_LINE" -lt "$ZERO_COMMITS_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ POLICY_DENY branch precedes both dev-groom-note and zero-commits messages"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ POLICY_DENY branch must precede both messages in HEAD-unchanged block"
    echo "    (POLICY_LINE=$POLICY_LINE, GROOM_LINE=$GROOM_LINE, ZERO_COMMITS_LINE=$ZERO_COMMITS_LINE)"
fi

# --- Test 15: mika#1383 structural completion gate (HEAD-changed + missing PR) ---
#
# mika#1383: dispatch-lib now auto-creates a PR when the pilot committed but
# didn't reach `gh pr create`. This is the structural completion gate for the
# tail-loss failure modes (Mode 1: bare /ce-work, Mode 2: full /mika tail).
# Tests validate the placement, scope (dev-pilot only), decision flow.

echo ""
echo "Test 15: mika#1383 structural completion gate (HEAD-changed + no PR)"
echo "--------------------------------------------------------------------"

# Block extraction: from the gate's marker comment to the next major section.
GATE_BLOCK=$(sed -n '/mika#1383: structural completion gate/,/Post-flight plan validation/p' "$DISPATCH_LIB")

assert_contains "Gate references mika#1271 (content/workflow split)" \
    'mika#1271' "$GATE_BLOCK"
assert_contains "Gate references mika#1282 (companion handler for HEAD-unchanged + dirty)" \
    'mika#1282' "$GATE_BLOCK"
assert_contains "Gate scoped to dev-pilot only (groom intentionally has no PR)" \
    'SKILL" = "dev-pilot"' "$GATE_BLOCK"
assert_contains "Gate fires only when HEAD has advanced (PRE != POST)" \
    'PRE_RUN_HEAD" != "$POST_RUN_HEAD"' "$GATE_BLOCK"
assert_contains "Gate uses gh pr list --head <branch> for PR existence check" \
    'gh pr list --repo "$REPO" --head "$BRANCH"' "$GATE_BLOCK"
assert_contains "Phase A: trailing dirty rescue with wip() prefix" \
    'wip(${REPO}#${ISSUE_NUM}): trailing content after pilot end_turn (mika#1383)' "$GATE_BLOCK"
assert_contains "Phase A: same scaffold-path exclusion as mika#1282 (.claude/commands, claude-pilot.json)" \
    ":!.claude/commands/" "$GATE_BLOCK"
assert_contains "Phase B: gh pr create with --base main --head BRANCH" \
    'gh pr create' "$GATE_BLOCK"
assert_contains "Phase B: PR title derived from latest commit subject" \
    'git -C "$WORKTREE_DIR" log -1 --format=' "$GATE_BLOCK"
assert_contains "Phase B: Closes #ISSUE_NUM in body" \
    'Closes #${ISSUE_NUM}' "$GATE_BLOCK"
assert_contains "PR-create failure surfaces structured manual recovery command" \
    'PIPELINE FAILURE: pilot produced commits on' "$GATE_BLOCK"
assert_contains "Manual recovery names the gh pr create command verbatim" \
    'Manual recovery:' "$GATE_BLOCK"

# Structural placement: the gate must fire AFTER the mika#1282 dirty-rescue
# (which only handles HEAD-unchanged) and BEFORE the dev-groom-specific
# post-flight plan validation. Both bounds verified by source order.
GATE_LINE=$(grep -n 'mika#1383: structural completion gate' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
M1282_LINE=$(grep -n 'Unit 1 (mika#1282): detect dirty worktree' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
GROOM_PLAN_LINE=$(grep -n 'Post-flight plan validation (mika#1033' "$DISPATCH_LIB" | head -1 | cut -d: -f1)
if [ -n "$GATE_LINE" ] && [ -n "$M1282_LINE" ] && [ -n "$GROOM_PLAN_LINE" ] \
    && [ "$M1282_LINE" -lt "$GATE_LINE" ] && [ "$GATE_LINE" -lt "$GROOM_PLAN_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ Gate placement: after mika#1282 dirty-rescue, before dev-groom plan validation"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Gate placement violated source-order invariant"
    echo "    (GATE_LINE=$GATE_LINE, M1282_LINE=$M1282_LINE, GROOM_PLAN_LINE=$GROOM_PLAN_LINE)"
fi

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
