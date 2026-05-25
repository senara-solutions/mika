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

# --- Test 8: Recovery shim fallback removed (mika#1144) ---

echo ""
echo "Test 8: _verify_and_write_body_callout has no unscoped fallback"
echo "-----------------------------------------------------------------"

VERIFY_FUNC=$(sed -n '/_verify_and_write_body_callout()/,/^}/p' "$DISPATCH_LIB")

# Assertion 1 (POSITIVE — F2 paired-test discipline): the issue-scoped find is
# retained. If a refactor accidentally removes BOTH finds, this assertion fires.
SCOPED_FIND_COUNT=$(printf '%s\n' "$VERIFY_FUNC" \
    | grep -cE 'find.*-name *"\*-\$\{issue_num\}-\*-plan\.md"' || true)
assert_eq "Issue-scoped find call retained" "1" "$SCOPED_FIND_COUNT"

# Assertion 2 (NEGATIVE — the core mika#1144 regression guard): the unscoped
# fallback find is gone. Detection shape: an assignment to plan_file with a
# `find ... -name "*-plan.md"` glob that does NOT contain `${issue_num}`.
# The scoped find above DOES contain `${issue_num}` and is filtered out by the
# `grep -v issue_num` step.
UNSCOPED_FALLBACK_COUNT=$(printf '%s\n' "$VERIFY_FUNC" \
    | grep -E 'plan_file=.*find.*-name *"\*-plan\.md"' \
    | grep -vc 'issue_num' || true)
assert_eq "No unscoped fallback find call (mika#1144 strip)" "0" "$UNSCOPED_FALLBACK_COUNT"

# Assertion 3: the new stderr log key is present (signals the skip path is
# explicit, not silent).
assert_contains "Skipped-recovery stderr log present" \
    'body_callout_drift_recovery_skipped' "$VERIFY_FUNC"

# Assertion 4: the function still has the early-return guard after the
# issue-scoped find — no plan file → return 0, now with a log line on stderr.
SKIP_BLOCK=$(printf '%s\n' "$VERIFY_FUNC" | sed -n '/if \[ -z "\$plan_file" \]/,/^[[:space:]]*fi$/p' | head -10)
assert_contains "Skip block returns 0" "return 0" "$SKIP_BLOCK"
assert_contains "Skip block logs to stderr" '>&2' "$SKIP_BLOCK"

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

# --- Test 11: Class D drift fix — cat-file guard and fallback callouts (mika#1204) ---

echo ""
echo "Test 11: Class D drift fix — cat-file guard and fallback callouts (mika#1204)"
echo "-------------------------------------------------------------------------------"

VERIFY_FUNC=$(sed -n '/_verify_and_write_body_callout()/,/^}/p' "$DISPATCH_LIB")

# (a) cat-file -e guard exists — the core fix. HEAD's tree is checked before
# stamping a SHA, so a plan-on-disk-but-not-in-HEAD state is detected.
assert_contains "cat-file -e guard present" \
    'cat-file -e "HEAD:${plan_relpath}"' "$VERIFY_FUNC"

# (b) Recovery commit uses pathspec-limited commit (not git add + git commit).
# This prevents capturing other staged files from a partial pilot run.
assert_contains "Recovery commit is pathspec-limited" \
    'git -C "$worktree_dir" commit -m' "$VERIFY_FUNC"
# Check for pathspec separator followed by plan_relpath in the commit command.
# Cannot use assert_contains here because the needle starts with '--' which
# grep interprets as an option. Use a direct grep -F with -- end-of-options.
if printf '%s\n' "$VERIFY_FUNC" | grep -qF -- '-- "$plan_relpath"'; then
    PASS=$((PASS + 1))
    echo "  ✓ Pathspec targets plan file only"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Pathspec targets plan file only"
fi

# (c) Recovery commit message follows wip() convention with issue reference.
assert_contains "Recovery commit message has wip() prefix" \
    'wip(${repo}#${issue_num})' "$VERIFY_FUNC"

# (d) Commit failure fallback stamps as uncommitted, not with a fabricated SHA.
# The (uncommitted ...) format does NOT match "committed on branch @" — safe for
# downstream parsers (check_grooming_markers, _detect_plan_on_branch).
assert_contains "Commit-failure fallback uses uncommitted callout" \
    '(uncommitted on branch' "$VERIFY_FUNC"

# (e) Push failure fallback stamps differently from commit failure — operator
# can distinguish the two states.
assert_contains "Push-failure fallback uses committed-locally callout" \
    '(committed locally, push failed' "$VERIFY_FUNC"

# (f) head_sha capture is AFTER the cat-file guard — not before. This ensures
# the SHA is only stamped when the plan is verifiably in HEAD.
# Strategy: extract line numbers and confirm ordering.
CATFILE_LINE=$(printf '%s\n' "$VERIFY_FUNC" | grep -n 'cat-file -e "HEAD:${plan_relpath}"' | head -1 | cut -d: -f1)
HEADSHA_LINE=$(printf '%s\n' "$VERIFY_FUNC" | grep -n 'head_sha=$(git -C "$worktree_dir" rev-parse --short HEAD' | head -1 | cut -d: -f1)
if [ -n "$CATFILE_LINE" ] && [ -n "$HEADSHA_LINE" ] && [ "$CATFILE_LINE" -lt "$HEADSHA_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ cat-file guard (line $CATFILE_LINE) fires before head_sha capture (line $HEADSHA_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ cat-file guard must fire before head_sha capture"
    echo "    catfile_line=$CATFILE_LINE headsha_line=$HEADSHA_LINE"
fi

# (g) Push of recovery commit is present — SHA must be reachable from origin.
assert_contains "Recovery commit is pushed to origin" \
    'git -C "$worktree_dir" push origin "$branch"' "$VERIFY_FUNC"

# (h) Both fallback paths return 0 (recovery is best-effort, not fatal).
# Use a wider capture (30 lines) since fallback blocks include heredoc + gh issue edit.
COMMIT_FAIL_BLOCK=$(printf '%s\n' "$VERIFY_FUNC" | sed -n '/post_flight_class_d_commit_failed/,/return 0/p' | head -30)
assert_contains "Commit-failure fallback returns 0" "return 0" "$COMMIT_FAIL_BLOCK"
PUSH_FAIL_BLOCK=$(printf '%s\n' "$VERIFY_FUNC" | sed -n '/post_flight_class_d_push_failed/,/return 0/p' | head -30)
assert_contains "Push-failure fallback returns 0" "return 0" "$PUSH_FAIL_BLOCK"

# (i) Stderr diagnostic keys are present for observability.
assert_contains "Class D recovery start log key" \
    'post_flight_class_d_recovery' "$VERIFY_FUNC"
assert_contains "Commit failure log key" \
    'post_flight_class_d_commit_failed' "$VERIFY_FUNC"
assert_contains "Push failure log key" \
    'post_flight_class_d_push_failed' "$VERIFY_FUNC"

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

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
