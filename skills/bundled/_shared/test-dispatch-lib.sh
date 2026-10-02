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

# Hermeticity (mika#1772): the probes below run `git init` + `git commit` in temp
# dirs and inherit the host's git config. On a machine whose global config sets
# `commit.gpgsign = true` the suite aborts partway with exit 128 and leaks its
# temp dirs (measured). Now that CI gates on this suite — on a shared
# self-hosted runner — that ambient coupling is a merge blocker waiting to happen.
#
# Override the signing keys only. Nulling GIT_CONFIG_GLOBAL instead strips the
# committer identity several existing fixtures rely on: measured, that produces
# 8 failures at rc=128 across the mika#1341/#1364/#1407/#1414 fixtures.
# `init.defaultBranch` is the second coupling: the fixtures `git init` and then
# reference `main`. A host without it gets `master` and seven rebase fixtures
# fail at rc=128. It is set on this developer machine, which is why the suite
# passed here while never having been run anywhere else.
export GIT_CONFIG_COUNT=5
export GIT_CONFIG_KEY_3=user.name
export GIT_CONFIG_VALUE_3=dispatch-lib test suite
export GIT_CONFIG_KEY_4=user.email
export GIT_CONFIG_VALUE_4=test@localhost
export GIT_CONFIG_KEY_0=commit.gpgsign
export GIT_CONFIG_VALUE_0=false
export GIT_CONFIG_KEY_1=tag.gpgsign
export GIT_CONFIG_VALUE_1=false
export GIT_CONFIG_KEY_2=init.defaultBranch
export GIT_CONFIG_VALUE_2=main

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/dispatch-lib.sh"

PASS=0
FAIL=0
# mika#2149: a third column, so a guard that could not arm is never read as a
# bare green. Incremented only by a probe that says SKIP out loud.
SKIPPED=0

# ─────────────────────────────────────────────────────────────────────────────
# Le binaire `mika`, résolu UNE fois et PARTAGÉ (mika#2608 R4)
#
# Depuis mika#2194 `_extract_plan_path` invoque `mika` par son nom, et depuis
# mika#2608 les deux lecteurs bash du callout délèguent à cette fonction. Donc
# les douze cas fonctionnels de la porte de grooming (`test_groom_gate_*`,
# invoqués vers la ligne 4210) en dépendent AUSSI — et ils tournent bien avant
# le bloc mika#2194, qui portait cette résolution en local.
#
# Trois portes, dans l'ordre : le PATH (le cas production), puis les deux cibles
# cargo (le cas dev, et CI où `cargo test` précède `make test-dispatch-lib`).
#
# Le PATH est augmenté pour TOUT le fichier plutôt que par bloc : `mika` est
# appelé par son nom au fond de dispatch-lib, donc un bloc qui oublierait de
# l'augmenter skipperait sans le dire — l'inverse du contrat de la troisième
# colonne. Aucune variable de production n'est introduite : c'est le PATH.
MIKA_BIN=""
if command -v mika >/dev/null 2>&1; then
    MIKA_BIN="$(command -v mika)"
else
    for _mika_cand in "$SCRIPT_DIR/../../../target/debug/mika" "$SCRIPT_DIR/../../../target/release/mika"; do
        if [ -x "$_mika_cand" ]; then MIKA_BIN="$_mika_cand"; break; fi
    done
fi
if [ -n "$MIKA_BIN" ]; then
    PATH="$(cd "$(dirname "$MIKA_BIN")" && pwd -P):$PATH"
    export PATH
fi

# `mika_bin_or_skip <étiquette>` — rend 0 si le binaire est là, sinon compte un
# SKIP BRUYANT et rend 1. Le SKIP est bruyant parce qu'un bloc de porte qui n'a
# pas pu s'armer ne doit pas se lire comme un vert (mika#2149, troisième
# colonne). CI construit avant (`cargo test` précède `make test-dispatch-lib`
# dans `ci.yml`), donc le SKIP est le cas dev, pas le cas CI.
mika_bin_or_skip() {
    [ -n "$MIKA_BIN" ] && return 0
    SKIPPED=$((SKIPPED + 1))
    echo "  ⊘ SKIP $1 — binaire 'mika' introuvable (PATH, target/debug, target/release)."
    echo "    Depuis mika#2608 les deux lecteurs bash du callout délèguent à"
    echo "    'mika plan-callout' : sans le binaire ce bloc ne peut RIEN mesurer."
    echo "    Remède : cargo build -p mika-cli --bin mika"
    return 1
}

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
    if grep -qF -- "$needle" <<<"$haystack"; then
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
    if ! grep -qF -- "$needle" <<<"$haystack"; then
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

    # mika#2506 — the prompt must NAME the deterministic gesture.
    #
    # The assertion is POSITIVE, and deliberately so. The negative half of
    # AC10 ("no longer prescribes route 1a") is not greppable: the assertion
    # directly above REQUIRES `dispatch_task_has_open_pr` to stay mentioned,
    # because the guard does still fire on an ACTIVE task carrying a pr_url.
    # A denylist on that token would therefore contradict its own sibling.
    # What separates "the prompt describes the guard" from "the prompt
    # prescribes the broken 2-step route" is whether the deterministic
    # command is named as the route to take — so that is what is asserted.
    #
    # Its failure mode is the one this ticket exists to close: a prescriber
    # that prescribes a dead route manufactures the next occurrence. Per
    # feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate,
    # this prompt half does NOT hold on its own — the structural half is the
    # `mika iterate` command itself (mika#2506 R1). This guard only stops the
    # prompt from silently drifting back, which no behavioural test can see:
    # re-prescribing route 1a makes no decision wrong on the day it is
    # written, it just re-creates the mis-route measured on mika#2503.
    if grep -qF "mika iterate" "$PROMPT_FILE"; then
        PASS=$((PASS + 1)); echo "  ✓ Prompt names the deterministic iterate gesture (mika#2506 AC10)"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ Prompt names the deterministic iterate gesture (mika#2506 AC10)"
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
#
# Cinq sorties no-op depuis mika#2194, et la cinquième est un livrable (AC4) :
# skill, issue_body, worktree_dir, callout illisible (code ≥2 — REFUS NOMMÉ),
# puis aucun callout (code 1). La quatrième est la population que la migration
# CRÉE : avant elle, le corps était en variable et il n'y avait pas de fichier à
# ne pas pouvoir lire. La confondre avec « aucun callout » fabriquerait un
# silence qui n'existait pas.
#
# Le compte reste une assertion de FORME : une sixième sortie ajoutée sans
# raison écrite fait rougir ce test, ce qui est son objet.
NON_COMMENT_RETURNS=$(printf '%s\n' "$PLAN_FUNC" | grep -v '^\s*#' | grep -c 'return 0' || true)
assert_eq "Has 5 guard return statements (skill, body, worktree, unreadable, no-callout)" "5" "$NON_COMMENT_RETURNS"

# --- Test 7: Plan callout regex extraction (mika#1074) ---

echo ""
echo "Test 7: Plan callout regex extraction (live)"
echo "----------------------------------------------"

# Les cas d'extraction eux-mêmes vivent dans la section mika#2120, plus bas :
# ils y appellent `_extract_plan_path`, la fonction de production, au lieu d'en
# recopier le motif ici. La copie qui vivait à cet endroit a survécu au
# changement de motif de mika#2120 sans rien signaler — c'est la classe même que
# ce ticket ferme, rejouée dans son propre harnais de test. Un test qui recopie
# ce qu'il vérifie ne le vérifie pas.

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
# mika#2634: `--reinstall` joins the command, and it is not cosmetic. The
# remedy now has to cover TWO measured causes — a stale dependency set
# (mika#1200) and an entry point taken over by a pip-user editable install
# (mika#2634, which leaves the uv-tool venv intact and only rewrites the
# shebang, so a plain `--force` can leave it in place).
assert_contains "Smoke test error mentions uv tool install restoration command" \
    'uv tool install --reinstall --force --editable ./claude-pilot-py' "$DISPATCH_BODY"
assert_contains "Smoke test error references mika#1200" \
    'mika#1200' "$DISPATCH_BODY"
# mika#2634: …and the second cause, with its own reference, so a reader who
# lands on a broken shebang is not sent to re-sync dependencies that are fine.
assert_contains "Smoke test error references mika#2634 (shebang takeover)" \
    'mika#2634' "$DISPATCH_BODY"

# (c) Smoke test fires BEFORE worktree mutation — verify ordering:
#     The smoke test (claude-pilot --help) must appear before _set_up_worktree
#     in the function body. Extract line numbers to confirm ordering.
#
# mika#2634 — the `_set_up_worktree` needle is ANCHORED on the call, and that
# repair is load-bearing rather than cosmetic. The bare `grep -n
# '_set_up_worktree'` matched the mika#2155 COMMENT twenty-five lines above the
# call ("it must exist even when `_set_up_worktree` was never reached"), so this
# probe compared the smoke test against a mention in prose. It passed only
# because the smoke test happened to sit above that comment too; moving the
# block thirty-nine lines down — the whole of mika#2634 — turned it red while
# the property it claims to measure (smoke test before any worktree mutation)
# stayed TRUE. A probe that reads prose is the mika#2050 class, and here it was
# one line away from blocking a fix for the defect it exists to prevent.
SMOKE_LINE=$(printf '%s\n' "$DISPATCH_BODY" | grep -n 'claude-pilot --help' | head -1 | cut -d: -f1)
WORKTREE_LINE=$(printf '%s\n' "$DISPATCH_BODY" | grep -nE '^[[:space:]]*_set_up_worktree( |$)' | head -1 | cut -d: -f1)
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

# (b2..b5) The block itself. Bounded on the `exit` rather than on `/fi/`:
# mika#2634 made the body long enough that `head -20` truncated it, and `/fi/`
# matches the first line merely CONTAINING "fi" — the word "fired" in a comment
# does it. Both bounds were loose, and a loose bound on an assertion block turns
# it green by accident, which is what (b2) was before this repair (it matched
# `exit 1` inside a comment while the code no longer exits 1 at all).
SMOKE_BLOCK=$(printf '%s\n' "$DISPATCH_BODY" | sed -n '/claude-pilot --help/,/^[[:space:]]*exit /p')

# (b2) mika#2634 — the pre-flight exits on the DEDICATED code, never 1.
#
# `exit 1` was what made this failure indistinguishable from every other handler
# crash, and the engine had nothing to classify on. The literal is asserted here
# AND cross-checked against Rust by
# `skills::executor::tests::mika2634::*`, in both directions: this value is
# written twice across two languages and no single-source exists for it (the
# duplication mika#2520 had to accept for `GIT_OPS_PROTECTED_BRANCHES`).
assert_contains "Smoke test exits on the dedicated launcher-dead code, not 1" \
    'exit "$_EXIT_LAUNCHER_DEAD"' "$SMOKE_BLOCK"
assert_contains "…and that code is 79" \
    '_EXIT_LAUNCHER_DEAD=79' "$(cat "$DISPATCH_LIB")"
# Negative control: the bare `exit 1` is GONE from the block. Without this, the
# assertion above is satisfied by a block that carries both.
M2634_SMOKE_EXITS=$(printf '%s\n' "$SMOKE_BLOCK" \
    | grep -cE '^[[:space:]]*exit 1[[:space:]]*$' || true)
assert_eq "mika#2634 (contrôle négatif): plus aucun \`exit 1\` nu dans le bloc" \
    "0" "$M2634_SMOKE_EXITS"

# (b3) mika#2634 — the diagnostic does NOT go to fd 2, and that is the fix.
#
# This probe used to assert `>&2` with the comment "lands in task result via
# EXIT trap". Both halves were false. `exec 9>>"$TRACE_FILE" 2>/dev/null` at the
# top of `dispatch_claude_pilot` is an `exec` WITHOUT a command, so its
# redirections apply to the current shell permanently: fd 2 has been /dev/null
# ever since (mika#903, stated textually in `_halt_family`'s own comment). The
# diagnostic was written into the void — which is why the three measured
# dispatches of 2026-10-02 reported `stderr_bytes: 0`, read as "there was
# nothing to say". The channels that work are fd 9 (the trace the trap appends)
# and `RESULT`.
assert_contains "Smoke test marker goes to the trace fd, not to /dev/null fd 2" \
    '>&9' "$SMOKE_BLOCK"
assert_contains "Smoke test poses RESULT itself (the channel that reaches the callback)" \
    'RESULT="LAUNCHER DEAD' "$SMOKE_BLOCK"

# (b3-bis) mika#2634 — the launcher's OWN stderr reaches the RESULT.
#
# This is the half of AC3 that carries the CAUSE rather than the remedy, and it
# is captured to a file this block owns instead of riding the trace window.
# Measured: with `set -x` active, the `tail -50` the trap appends is consumed by
# the trace of this block plus the trap's own commands, so a `2>&9` traceback
# falls out of the window. The assertion is on the composition, not on the
# intention — a `RESULT` that names the remedy and drops the cause looks
# delivered and is not.
assert_contains "Smoke test captures the launcher's own stderr" \
    '2>"$_SMOKE_ERR_FILE"' "$SMOKE_BLOCK"
assert_contains "…and that capture reaches the RESULT" \
    '${_SMOKE_STDERR}' "$SMOKE_BLOCK"
# …and the capture file is cleaned on BOTH paths. The failure path removes it
# before composing the RESULT; the nominal path removes the empty leftover. A
# per-dispatch temp file nobody removes is a slow leak in /tmp.
M2634_SMOKE_CLEANUP=$(printf '%s\n' "$DISPATCH_BODY" \
    | grep -cE '^[[:space:]]*rm -f "\$_SMOKE_ERR_FILE"[[:space:]]*$' || true)
assert_eq "mika#2634: le fichier de capture est nettoyé sur les DEUX chemins" \
    "2" "$M2634_SMOKE_CLEANUP"

# (b4) Smoke test suppresses stdout
assert_contains "Smoke test suppresses stdout" '>/dev/null' "$SMOKE_BLOCK"

# (b5) Smoke test has a timeout guard against hung venvs
assert_contains "Smoke test has timeout guard" 'timeout' "$SMOKE_BLOCK"

# --- Test 9-bis: le lanceur mort est estampillé, terminal, et borné (mika#2634) ---

echo ""
echo "Test 9-bis: launcher-dead stamping and terminality (mika#2634)"
echo "---------------------------------------------------------------"

# (A) L'ORDRE, et c'est le défaut mesuré.
#
# Le smoke test doit tourner APRÈS `trap '_dispatch_lib_exit_trap' EXIT`. Avant,
# il sortait treize lignes plus haut : aucun callback n'était livré, la tâche
# restait non terminale, et le moteur écrivait `Process Exit code: 1: ` avec un
# stderr vide. Trois dispatches sont morts comme ça en 2 h 45 le 2026-10-02 sans
# qu'aucune alarme ne se lève.
#
# L'aiguille du trap est ancrée sur l'INSTALLATION (`trap '…' EXIT`), jamais sur
# le nom de la fonction : celui-ci apparaît aussi en commentaire, et c'est très
# exactement le faux positif que la réparation de (c) ci-dessus a dû corriger.
M2634_TRAP_LINE=$(printf '%s\n' "$DISPATCH_BODY" \
    | grep -nE "^[[:space:]]*trap '_dispatch_lib_exit_trap' EXIT" | head -1 | cut -d: -f1)
assert_eq "mika#2634 (anti-vacuité): l'installation du trap EXIT est trouvée" \
    "yes" "$([ -n "$M2634_TRAP_LINE" ] && echo yes || echo no)"
if [ -n "$M2634_TRAP_LINE" ] && [ -n "$SMOKE_LINE" ] && [ "$M2634_TRAP_LINE" -lt "$SMOKE_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ mika#2634: le trap EXIT (ligne $M2634_TRAP_LINE) est armé AVANT le smoke test (ligne $SMOKE_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ mika#2634: le smoke test doit tourner APRÈS l'armement du trap EXIT"
    echo "    trap_line=$M2634_TRAP_LINE smoke_line=$SMOKE_LINE"
fi

# (B) Les trois `command -v` restent AVANT le trap, et ce n'est pas un détail
# d'ordonnancement. Le trap livre son callback par le CLI `mika` et son corps
# emploie `jq` : les déplacer après donnerait un trap qui échoue en silence sur
# un hôte où l'un des deux manque — une panne muette échangée contre une autre.
M2634_CMDV_JQ=$(printf '%s\n' "$DISPATCH_BODY" | grep -n 'command -v jq' | head -1 | cut -d: -f1)
if [ -n "$M2634_CMDV_JQ" ] && [ -n "$M2634_TRAP_LINE" ] && [ "$M2634_CMDV_JQ" -lt "$M2634_TRAP_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ mika#2634: \`command -v jq\` (ligne $M2634_CMDV_JQ) reste avant le trap (ligne $M2634_TRAP_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ mika#2634: les dépendances du trap doivent être vérifiées AVANT son armement"
    echo "    jq_line=$M2634_CMDV_JQ trap_line=$M2634_TRAP_LINE"
fi

# (C) LE RESULT EST TERMINAL — il ne matche aucun motif de la famille retryable.
#
# C'est la moitié vérifiable d'U2, et le plan la demande explicitement plutôt
# que de la laisser à une lecture. `HANDLER CRASH` générique serait un piège
# mesuré : il est dans la population que `self-dev-callback/system_prompt.md`
# invite à rejouer, et que `_gate_non_empty_cycle` grepe à côté de
# `PIPELINE FAILURE:`. Un lanceur cassé rejoué est la boucle que mika#2545 a dû
# refermer pour l'`ESCALATE` de groom : chaque rejeu meurt identiquement jusqu'à
# réparation de l'HÔTE, et aucun budget ne la borne.
#
# Le RESULT est extrait du source (de son assignation jusqu'à sa ligne
# `Outcome:`), puis confronté à l'alternation EXACTE de `_gate_non_empty_cycle`.
M2634_RESULT_BLOCK=$(printf '%s\n' "$DISPATCH_BODY" \
    | sed -n '/RESULT="LAUNCHER DEAD/,/^Outcome: LAUNCHER_DEAD"$/p')
assert_eq "mika#2634 (anti-vacuité): le RESULT du lanceur mort est trouvé" \
    "yes" "$([ -n "$M2634_RESULT_BLOCK" ] && echo yes || echo no)"
M2634_RETRYABLE=$(printf '%s\n' "$M2634_RESULT_BLOCK" \
    | grep -cE '(PIPELINE FAILURE:|STRUCTURAL VIOLATION:|HANDLER CRASH|^STATUS=CANCELLED|^Outcome: PIPELINE_INCOMPLETE|^Outcome: ESCALATE)' || true)
assert_eq "mika#2634: le RESULT ne matche AUCUN motif de la famille retryable" \
    "0" "$M2634_RETRYABLE"
# Contrôle de bonne foi : l'alternation ci-dessus accuse bel et bien quand on
# lui donne un membre de la famille. Sans lui, « le RESULT est propre » est
# indistinguable de « le prédicat ne regarde rien ».
M2634_CTRL=$(printf '%s\n' 'HANDLER CRASH (exit code 79).' \
    | grep -cE '(PIPELINE FAILURE:|STRUCTURAL VIOLATION:|HANDLER CRASH|^STATUS=CANCELLED|^Outcome: PIPELINE_INCOMPLETE|^Outcome: ESCALATE)' || true)
assert_eq "mika#2634 (contrôle de bonne foi): l'alternation accuse un membre de la famille" \
    "1" "$M2634_CTRL"

# (D) La ligne `Outcome:` est ANCRÉE en début de ligne — les lecteurs le sont
# tous (`grep -qE '^Outcome: '`), et une ligne posée en milieu de ligne est le
# piège que mika#2590 a dû désamorcer.
#
# Le `"` terminal fait partie de l'aiguille parce que ce prédicat lit le SOURCE,
# où ce guillemet ferme la chaîne ; à l'exécution la ligne du RESULT n'en porte
# pas. Dit ici plutôt que découvert : l'ancrage à droite est aussi ce qui
# garantit que la ligne est bien la DERNIÈRE du RESULT, donc qu'aucun texte ne
# la suit sur sa ligne.
M2634_OUTCOME=$(printf '%s\n' "$M2634_RESULT_BLOCK" | grep -cE '^Outcome: LAUNCHER_DEAD"$' || true)
assert_eq "mika#2634: exactement une ligne \`Outcome: LAUNCHER_DEAD\` ancrée" \
    "1" "$M2634_OUTCOME"

# (E) Le RESULT mène par `LAUNCHER DEAD`, ce dont dépend le `case` du trap qui
# append la queue de trace. Sans cette tête, le bras `*)` SUPPRIME le fichier et
# la cause (`ModuleNotFoundError`, déjà écrite sur fd 9 par le smoke test) est
# perdue exactement sur le chemin qui a été ouvert pour l'avoir perdue.
assert_contains "mika#2634: le trap append la trace sur le marqueur LAUNCHER DEAD" \
    '"HANDLER CRASH"*|"LAUNCHER DEAD"*' "$(sed -n '/^_dispatch_lib_exit_trap() {/,/^}/p' "$DISPATCH_LIB")"

# (E-bis) LE TRAP SURVIT À CE NOUVEAU POINT D'ENTRÉE, et c'est le risque
# principal du déplacement : un trap qui plante sur ce chemin remplacerait une
# panne muette par une autre, ce qui est pire que le défaut.
#
# Quatre préconditions, épinglées plutôt que raisonnées — le raisonnement est
# juste mais il n'est pas une garde.
#
# Les deux variables que le trap lit sans guard (`[ "$CALLBACK_SENT" -eq 1 ]`
# échouerait sur une variable non définie) doivent être posées AVANT lui.
M2634_CBSENT=$(printf '%s\n' "$DISPATCH_BODY" | grep -nE '^[[:space:]]*CALLBACK_SENT=0[[:space:]]*$' | head -1 | cut -d: -f1)
M2634_SEAT=$(printf '%s\n' "$DISPATCH_BODY" | grep -nE '^[[:space:]]*ISSUE_SEAT_CLAIMED=0[[:space:]]*$' | head -1 | cut -d: -f1)
M2634_PARSE=$(printf '%s\n' "$DISPATCH_BODY" | grep -nE '^[[:space:]]*_parse_input_json[[:space:]]*$' | head -1 | cut -d: -f1)
for _pair in "CALLBACK_SENT:$M2634_CBSENT" "ISSUE_SEAT_CLAIMED:$M2634_SEAT" "_parse_input_json (pose TASK_ID):$M2634_PARSE"; do
    _name="${_pair%%:*}"; _line="${_pair##*:}"
    if [ -n "$_line" ] && [ -n "$M2634_TRAP_LINE" ] && [ "$_line" -lt "$M2634_TRAP_LINE" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ mika#2634: $_name (ligne $_line) est posé avant le trap (ligne $M2634_TRAP_LINE)"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ mika#2634: $_name doit être posé AVANT le trap — sinon le trap plante sur ce chemin"
        echo "    line=$_line trap_line=$M2634_TRAP_LINE"
    fi
done

# La quatrième : le trap découvre la PR, et sur ce chemin `REPO`/`BRANCH` sont
# vides. `_classify_no_pr_reason` doit court-circuiter sur un repo vide SANS
# appeler `gh` — sinon le trap pendrait sur un hôte sans réseau, au moment
# précis où il porte le seul diagnostic disponible.
M2634_NOPR_SRC=$(sed -n '/^_classify_no_pr_reason() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "mika#2634: _classify_no_pr_reason court-circuite sur un repo vide" \
    'repo_unset' "$M2634_NOPR_SRC"
M2634_NOPR_GH=$(printf '%s\n' "$M2634_NOPR_SRC" | grep -c 'gh ' || true)
assert_eq "mika#2634: …et elle n'appelle aucun \`gh\`" "0" "$M2634_NOPR_GH"

# (F) Le smoke test NE PEUT PAS être le producteur d'un faux `empty_completion`.
# `PILOT_RAN=1` vit dans `_run_claude_pilot`, donc à ce point il est non défini
# et `_gate_non_empty_cycle` court-circuite. Cette assertion épingle la prémisse
# plutôt que de la supposer : si `PILOT_RAN` était un jour posé plus tôt, le
# lanceur mort serait EN PLUS accusé de n'avoir rien produit.
M2634_PILOT_RAN_SETTERS=$(grep -cE '^[[:space:]]*PILOT_RAN=1[[:space:]]*$' "$DISPATCH_LIB" || true)
assert_eq "mika#2634: un seul site pose PILOT_RAN=1" "1" "$M2634_PILOT_RAN_SETTERS"
assert_contains "mika#2634: …et ce site est dans _run_claude_pilot (donc après le smoke test)" \
    "PILOT_RAN=1" "$(sed -n '/^_run_claude_pilot() {/,/^}/p' "$DISPATCH_LIB")"

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

# ---------------------------------------------------------------------------
# Tier 0 — withheld disposition (mika#2037)
#
# The engine strips a disposition it refused to let stand and substitutes this
# literal marker. No tier may then derive a verdict from the response: it is an
# absence of verdict, not an approval.
#
# The tier-0 POSITION is what these assertions actually guard. After tier 1a the
# marker would still be honoured; after tier 2 it would not, because the fuzzy pass
# matches paraphrases anywhere in the text — so a response whose disposition line
# was merely removed could still yield READY out of its own body. The third and
# sixth assertions below are the ones that fail if tier 0 ever moves down.
#
# The literal must stay in sync with DISPOSITION_WITHHELD_MARKER in
# crates/mika-agent/src/agent_loop/mod.rs.
# ---------------------------------------------------------------------------
WITHHELD_MARKER="Disposition-Withheld: REVIEW-ANCHOR-MISSING"

assert_eq "tier 0 beats tier 1a (literal Disposition present)" "" \
    "$(printf 'Some review prose.\n%s\nDisposition: READY\n' "$WITHHELD_MARKER" | _parse_disposition 2>/dev/null)"
assert_eq "tier 0 beats tier 1b (Verdict carry-over present)" "" \
    "$(printf '%s\nVerdict: GROOMED\n' "$WITHHELD_MARKER" | _parse_disposition 2>/dev/null)"
assert_eq "tier 0 beats tier 2 (fuzzy READY paraphrases in the body)" "" \
    "$(printf 'The plan is clean, good to go — proceed with the dispatch.\n%s\n' "$WITHHELD_MARKER" | _parse_disposition 2>/dev/null)"

assert_eq "verdict tier 0 beats tier 1a" "" \
    "$(printf '%s\nVerdict: GROOMED\n' "$WITHHELD_MARKER" | _parse_verdict 2>/dev/null)"
assert_eq "verdict tier 0 beats tier 1b" "" \
    "$(printf '%s\nDisposition: READY\n' "$WITHHELD_MARKER" | _parse_verdict 2>/dev/null)"
assert_eq "verdict tier 0 beats tier 2 (fuzzy GROOMED paraphrases)" "" \
    "$(printf 'Approved, no remaining concerns, ship it.\n%s\n' "$WITHHELD_MARKER" | _parse_verdict 2>/dev/null)"

# A response that QUOTES the marker mid-line must not be suppressed: the three arch prompts
# now teach the literal to the model, so an echo is an ordinary shape. Only a line the engine
# wrote counts (mika#2037 F4).
assert_eq "quoted marker mid-line does not suppress a genuine ITERATE" "ITERATE" \
    "$(printf 'If the attestation were missing the engine would emit %s here.\nDisposition: ITERATE\n' "$WITHHELD_MARKER" | _parse_disposition 2>/dev/null)"
assert_eq "quoted marker mid-line does not suppress a genuine GROOMED" "GROOMED" \
    "$(printf 'The engine substitutes %s in that case.\nVerdict: GROOMED\n' "$WITHHELD_MARKER" | _parse_verdict 2>/dev/null)"
assert_eq "marker at the very start of the response still suppresses" "" \
    "$(printf '%s\nDisposition: READY\n' "$WITHHELD_MARKER" | _parse_disposition 2>/dev/null)"

# Non-regression: without the marker every tier behaves exactly as before.
assert_eq "no marker — tier 1a still returns READY" "READY" \
    "$(printf 'Some review prose.\nDisposition: READY\n' | _parse_disposition 2>/dev/null)"
assert_eq "no marker — tier 2 still returns READY" "READY" \
    "$(printf 'The plan is clean, good to go — proceed.\n' | _parse_disposition 2>/dev/null)"
assert_eq "no marker — verdict tier 1a still returns GROOMED" "GROOMED" \
    "$(printf 'Verdict: GROOMED\n' | _parse_verdict 2>/dev/null)"

# ---------------------------------------------------------------------------
# Tier 0b — engine escalation line (mika#2338)
#
# Since mika#2338 the engine no longer withholds an unattested disposition into the
# marker above: it rewrites the response into a terminal ESCALATE preceded by a
# finding line of fixed shape, `F<n>: (BLOCKING) [mika-engine] review-anchor: …`.
# Tier 0b recognizes that exact shape anchored at the START OF A LINE and returns
# ESCALATE before any textual tier runs. Two things are guarded here:
#
# - POSITION: tier 1a / verdict tier 1 grep the first `Disposition:`/`Verdict:`
#   ANYWHERE in the text, unanchored. The engine also rewrites every inline
#   mention, but the shell must be fail-closed on its own — the two layers each
#   suffice alone.
# - SHAPE: `[mika-engine]` alone is not the signal. Every corrective re-prompt
#   begins with it, the model relays it in its session, and the prompts teach it.
#   Only the full line the engine composes counts; an echo in prose does not.
#
# The literal must stay in sync with REVIEW_ANCHOR_ENGINE_FINDING_MARKER in
# crates/mika-agent/src/agent_loop/mod.rs (drift guard below, next to the marker's).
# ---------------------------------------------------------------------------
ENGINE_FINDING_MARKER="(BLOCKING) [mika-engine] review-anchor:"
ENGINE_LINE="F1: $ENGINE_FINDING_MARKER attestation withheld after the corrective re-prompt — anchors_found=3, anchors_valid=2, miss_reason=QuoteNotInBrief: paraphrase."

assert_eq "tier 0b — engine line + Disposition: ESCALATE parses ESCALATE" "ESCALATE" \
    "$(printf 'A1: x\nA2: y\n\n%s\n\nDisposition: ESCALATE\n' "$ENGINE_LINE" | _parse_disposition 2>/dev/null)"
assert_eq "tier 0b — verdict parser reads ESCALATE from the engine line" "ESCALATE" \
    "$(printf 'A1: x\n\n%s\n\nVerdict: ESCALATE\n' "$ENGINE_LINE" | _parse_verdict 2>/dev/null)"
assert_eq "tier 0b beats tier 1a — READY quoted inline BEFORE the engine line" "ESCALATE" \
    "$(printf 'Je confirme que ma réponse se termine par Disposition: READY comme demandé.\nA1: x\n\n%s\n\nDisposition: ESCALATE\n' "$ENGINE_LINE" | _parse_disposition 2>/dev/null)"
assert_eq "tier 0b beats verdict tier 1 — GROOMED quoted inline BEFORE the engine line" "ESCALATE" \
    "$(printf 'Mon verdict est bien Verdict: GROOMED.\n\n%s\n\nVerdict: ESCALATE\n' "$ENGINE_LINE" | _parse_verdict 2>/dev/null)"
assert_eq "tier 0b — indented engine line still counts" "ESCALATE" \
    "$(printf 'prose\n   %s\nDisposition: READY\n' "$ENGINE_LINE" | _parse_disposition 2>/dev/null)"
assert_eq "tier 0b — F2 prefix (any finding index) counts" "ESCALATE" \
    "$(printf 'F2: %s counters\nDisposition: READY\n' "$ENGINE_FINDING_MARKER" | _parse_disposition 2>/dev/null)"

# An echo of `[mika-engine]` in prose is not the engine line: the architect's own verdict stands.
assert_eq "echo of [mika-engine] in prose does not suppress a genuine READY" "READY" \
    "$(printf 'The engine re-prompt began with [mika-engine] and I addressed it.\nA1: x\nDisposition: READY\n' | _parse_disposition 2>/dev/null)"
assert_eq "echo of the marker mid-line does not force ESCALATE on a genuine ITERATE" "ITERATE" \
    "$(printf 'Were the anchors missing the engine would write %s here.\nDisposition: ITERATE\n' "$ENGINE_FINDING_MARKER" | _parse_disposition 2>/dev/null)"
assert_eq "verdict parser — echo of [mika-engine] in prose keeps a genuine GROOMED" "GROOMED" \
    "$(printf 'I saw a [mika-engine] re-prompt earlier.\nA1: x\nVerdict: GROOMED\n' | _parse_verdict 2>/dev/null)"

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
# mika#2363: the selection flag is now `--only-skill`, not `--enable-skill`. The
# latter was measured to be abandoned in transit since mika#1727 (`mika ask` is a
# thin A2A client; the flag configures a local registry that is no longer the
# execution surface), so mika-arch received all three of its always_on prompts on
# every turn. `--only-skill` is the subtractive half of that channel and it does
# reach spirit. The two are mutually exclusive in clap, so this is a replacement:
# keeping both would fail every architect call at argument parsing.
#
# mika#1283: _arch_ask passes plan content via stdin (mika ask "-" reads stdin),
# NOT as @-file argument (mika ask doesn't expand @<path>). The argv should
# contain a literal "-" marker and the stub should receive the plan content
# on stdin when invoked.
ARCH_PLAN_TMP=$(mktemp /tmp/arch-plan-XXXXXX.md)
printf 'plan content for arch ask test\n' > "$ARCH_PLAN_TMP"
mika() { printf '|%s' "$@"; printf '|'; }  # leading | so first arg also has separator
ARCH_ARGV=$(_arch_ask "mika-arch-groom-ticket" "$ARCH_PLAN_TMP")
assert_contains "_arch_ask uses only-skill flag (not the wrong skill flag) — mika#2363" "|--only-skill|" "$ARCH_ARGV"
assert_contains "_arch_ask threads skill name after only-skill" "|--only-skill|mika-arch-groom-ticket|" "$ARCH_ARGV"
assert_not_contains "_arch_ask no longer passes --enable-skill (exclusive with --only-skill, mika#2363)" "|--enable-skill|" "$ARCH_ARGV"
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
#
# mika#1943: le fixture porte désormais la forme RÉELLE d'un worktree de
# dispatch (`…/.claude/worktrees/<slug>/<repo>`) au lieu d'un `mktemp -d` nu.
# Ce n'était pas un détail cosmétique : la garde de suppression refuse un
# chemin qui ne prouve pas être un worktree géré, et le fixture exerçait donc
# une forme que la production ne produit jamais. Le répertoire reste sous
# `mktemp -d`, donc l'hermétisme de la suite (mika#1772) est intact.
CLEAN_ROOT=$(mktemp -d)
CLEAN_TMP="$CLEAN_ROOT/.claude/worktrees/chore-1271-iterate-fixture/mika"
mkdir -p "$CLEAN_TMP"
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
rm -rf "$CLEAN_ROOT"

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
#
# mika#2641 — le compte passe de 2 à 4, et c'est la SYMÉTRIE qui est épinglée,
# pas le nombre : chaque branche porte désormais un essai initial PLUS une
# relance corrective (le patron mika#1823 de la première passe, porté sur la
# seconde). Deux par branche, donc quatre, et un compte impair dirait qu'un seul
# des deux sites a reçu la relance — ce qui se lirait exactement comme deux.
SECOND_PASS_COUNT=$(printf '%s\n' "$ITERATE_FULL" | grep -c 'mika-arch-second-review')
assert_eq "_iterate_groom_loop invokes second-pass twice per branch (READY + ITERATE, chacune avec sa relance mika#2641)" \
    "4" "$SECOND_PASS_COUNT"

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
# mika#2545: the marker changed CLASS, not substance. `PIPELINE FAILURE:` is what
# put a terminal grooming verdict into `self-dev-callback`'s retryable population
# and got the groom of mika#2542 replayed eight times; the producer now stamps its
# own terminal fact instead. The negative control below is the half that matters —
# without it, "the terminal marker is written" would not say the retryable one is
# gone.
assert_contains "_escalate_groom stamps the terminal marker on RESULT (mika#2545)" "GROOM ESCALATED (terminal): mika-arch escalated at first-pass" "$RESULT"
assert_not_contains "_escalate_groom no longer writes the retryable marker (mika#2545)" "PIPELINE FAILURE:" "$RESULT"
assert_contains "_escalate_groom RESULT includes Verdict: ESCALATE" "Verdict: ESCALATE" "$RESULT"
assert_contains "_escalate_groom RESULT includes session_id" "Session: session-esc-1" "$RESULT"
assert_contains "_escalate_groom RESULT references findings file path" "Architect findings preserved at:" "$RESULT"
rm -rf "$ESC_TMP"

# _escalate_groom — engine escalation line (mika#2338): the cause travels into RESULT and
# the failure reason names the engine, not the architect.
ESC_TMP_ENG=$(mktemp -d)
RESULT=""
GROOM_LOOP_FAILURE_REASON="architect ESCALATE (first-pass)"
ENGINE_CONTENT="A1: x
A2: y

$ENGINE_LINE

Disposition: ESCALATE"
WORKTREE_DIR="$ESC_TMP_ENG" _escalate_groom "first-pass" "$ENGINE_CONTENT" "session-esc-eng" 2>/dev/null
assert_contains "_escalate_groom copies the engine line into RESULT as Engine reason" "Engine reason: $ENGINE_LINE" "$RESULT"
assert_eq "_escalate_groom names the engine in the failure reason" \
    "engine ESCALATE (first-pass): review-anchor attestation withheld" "$GROOM_LOOP_FAILURE_REASON"
rm -rf "$ESC_TMP_ENG"

# An architect ESCALATE whose prose echoes `[mika-engine]` is NOT relabelled as an engine refusal.
ESC_TMP_ECHO=$(mktemp -d)
RESULT=""
GROOM_LOOP_FAILURE_REASON="architect ESCALATE (first-pass)"
WORKTREE_DIR="$ESC_TMP_ECHO" _escalate_groom "first-pass" "F1: (BLOCKING) The [mika-engine] re-prompt was addressed but the plan contradicts ADR-008.
Disposition: ESCALATE" "session-esc-echo" 2>/dev/null
if grep -q "Engine reason:" <<<"$RESULT"; then
    assert_eq "_escalate_groom ignores an echo of [mika-engine] in architect prose" "no engine reason" "engine reason present"
else
    assert_eq "_escalate_groom ignores an echo of [mika-engine] in architect prose" "no engine reason" "no engine reason"
fi
assert_eq "_escalate_groom keeps the architect reason on an echo" "architect ESCALATE (first-pass)" "$GROOM_LOOP_FAILURE_REASON"
rm -rf "$ESC_TMP_ECHO"
GROOM_LOOP_FAILURE_REASON=""

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

# Code-shape: _iterate_groom_loop has exactly 5 _escalate_groom call sites.
#
# mika#2641 — le compte passe de 3 à 5. Les deux bras de seconde passe qui
# escaladaient indistinctement (`*)`) se sont scindés en DEUX chacun :
# `ESCALATE)` (un refus réel, cause `verdict`, `RESULT` inchangé à l'octet près)
# et `*)` (chaîne vide, cause `architect_unreadable`, AUCUNE ligne `Verdict:`).
# C'est cette scission qui EST le correctif : avant, une absence de signal
# empruntait le bras d'un verdict.
#
#   1. first-pass ESCALATE                     → verdict
#   2. after-ready, verdict ESCALATE           → verdict
#   3. after-ready, verdict illisible ×2       → architect_unreadable
#   4. after-iterate, verdict ESCALATE         → verdict
#   5. after-iterate, verdict illisible ×2     → architect_unreadable
#
# Le scan de cardinalité du bloc mika#2641 (S-c) exige en plus que chacun des
# cinq passe sa cause par la CONSTANTE : un site qui l'omettrait tomberait dans
# le défaut et produirait un `RESULT` plausible.
ITERATE_NOW=$(declare -f _iterate_groom_loop)
ESCALATE_CALL_COUNT=$(printf '%s\n' "$ITERATE_NOW" | grep -c '_escalate_groom')
assert_eq "_iterate_groom_loop has 5 _escalate_groom call sites (mika#2641)" "5" "$ESCALATE_CALL_COUNT"

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
assert_contains "_escalate_groom populates RESULT even when WORKTREE_DIR unset" "GROOM ESCALATED (terminal)" "$RESULT"
assert_contains "_escalate_groom stamps its Outcome line even when WORKTREE_DIR unset (mika#2545)" \
    "Outcome: ESCALATE — first-pass" "$RESULT"

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

# --- Single-pass grooming exit (mika#2012) ---
#
# The first-pass READY disposition is a legitimate grooming exit
# (/mika-groom-ticket Phase 3 step 10) that had no stage before mika#2012:
# _write_canonical_callout fell into `*)` and returned 1 silently, so no verdict
# was written and the ticket re-groomed forever. The history line deliberately
# does NOT claim `second-pass (GROOMED)` — no second pass ran, and a body that
# lies is a debt the next reader inherits. executor.rs::SINGLE_PASS_GROOMED_RE
# is the paired recognizer.
assert_contains "ready-single-pass stage exists" 'ready-single-pass)' "$WRITER_SRC"
assert_contains "ready-single-pass produces truthful single-pass history" 'first-pass (READY, single-pass GROOMED)' "$WRITER_SRC"
assert_contains "ready-single-pass does not fabricate a second pass" 'no second pass required' "$WRITER_SRC"

# Unknown stage must be operator-visible, not a silent return 1 — the silent
# failure IS the mika#2012 root cause.
CC_UNKNOWN_TMP=$(mktemp -d)
unknown_stderr=$( (WORKTREE_DIR="$CC_UNKNOWN_TMP" REPO="mika" ISSUE_NUM="999" BRANCH="test/branch" _write_canonical_callout "bogus-stage" "sess-unknown" 2>&1 >/dev/null) || true )
assert_contains "unknown stage emits greppable operator diagnostic" 'write_canonical_callout_unknown_stage' "$unknown_stderr"
assert_contains "unknown stage diagnostic names the consequence" 'NO VERDICT WRITTEN' "$unknown_stderr"
rm -rf "$CC_UNKNOWN_TMP"

# --- Callout replacement + path discipline (mika#2012 U3) ---

# Pin: the writer's idempotency pattern must stay in lockstep with executor.rs's
# three verdict regexes. Drift is not cosmetic — a form the gate accepts but this
# check misses makes the writer prepend a SECOND callout on every pass.
assert_contains "idempotency check knows canonical GROOMED (delimiter-tolerant)" 'second-pass \(GROOMED[[:space:]).,;:—-]' "$WRITER_SRC"
assert_contains "idempotency check knows paraphrased GROOMED" 'second-pass \(READY, paraphrased GROOMED' "$WRITER_SRC"
assert_contains "idempotency check knows single-pass GROOMED" 'first-pass \(READY, single-pass GROOMED' "$WRITER_SRC"

# The delimiter-tolerant form is the load-bearing fix: the OLD pattern was
# `second-pass \(GROOMED\)`, which required an immediate closing paren and so
# missed `second-pass (GROOMED — session-id: …)` — the exact shape this same
# function emits. That mismatch is the callout-stacking mechanism.
VERDICT_RE='second-pass \(GROOMED[[:space:])._,;:—-]|second-pass \(READY, paraphrased GROOMED|first-pass \(READY, single-pass GROOMED'
assert_eq "verdict pattern matches canonical em-dash session-id shape" "1" \
    "$(printf '%s' '> - **Grooming history:** first-pass (READY) → second-pass (GROOMED — session-id: abc)' | grep -cE "$VERDICT_RE" || true)"
assert_eq "verdict pattern matches single-pass shape" "1" \
    "$(printf '%s' '> - **Grooming history:** first-pass (READY, single-pass GROOMED) — no second pass required' | grep -cE "$VERDICT_RE" || true)"
assert_eq "verdict pattern does not match bare first-pass READY" "0" \
    "$(printf '%s' '> - **Grooming history:** first-pass (READY) — awaiting second pass' | grep -cE "$VERDICT_RE" || true)"
assert_eq "old pattern would have MISSED the em-dash shape (regression witness)" "0" \
    "$(printf '%s' '> - **Grooming history:** second-pass (GROOMED — session-id: abc)' | grep -cE 'second-pass \(GROOMED\)' || true)"

# Replacement, not stacking: existing callout lines are stripped before prepend.
assert_contains "writer strips existing callout lines" 'stripped_body=' "$WRITER_SRC"
assert_contains "writer strips all three callout line kinds" 'Branch|Plan|Grooming history' "$WRITER_SRC"
assert_contains "writer prepends onto the STRIPPED body, not the raw one" 'new_body=$(printf '"'"'%s\n\n%s'"'"' "$callout_block" "$stripped_body")' "$WRITER_SRC"

# Functional: a body carrying TWO stacked callout blocks (the mika#1962 shape)
# must come out with zero callout lines left.
STACKED_BODY='> - **Branch:** `fix/1962/new`
> - **Plan:** `docs/plans/new-plan.md` (committed on branch @ `bbb2222`)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED — session-id: two)

> - **Branch:** `fix/1962/old`
> - **Plan:** `docs/plans/stale-plan.md` (committed on branch @ `aaa1111`)
> - **Grooming history:** body callout recovered by post-flight (mika#1123)

## Symptom
Real content that must survive.'
STRIPPED_OUT=$(printf '%s' "$STACKED_BODY" | grep -vE '^> - \*\*(Branch|Plan|Grooming history):\*\*' | sed '/./,$!d')
assert_eq "stripping removes every callout line from a stacked body" "0" \
    "$(printf '%s' "$STRIPPED_OUT" | grep -cE '^> - \*\*(Branch|Plan|Grooming history):\*\*' || true)"
assert_contains "stripping preserves the real issue content" '## Symptom' "$STRIPPED_OUT"
assert_contains "stripping preserves body prose" 'Real content that must survive.' "$STRIPPED_OUT"
assert_not_contains "stripping drops the stale plan path" 'stale-plan.md' "$STRIPPED_OUT"

# Path discipline: the body must carry a repo-relative path that exists.
assert_contains "writer refuses a plan outside the worktree" 'write_canonical_callout_plan_outside_worktree' "$WRITER_SRC"
assert_contains "writer refuses a plan file that is absent" 'write_canonical_callout_plan_missing' "$WRITER_SRC"

# Review finding: the character class must mirror Rust's exactly. An extra
# member is not harmless — a body the writer thinks is stamped but the gate
# rejects re-groom-loops the ticket, which is #2012 in mirror image.
assert_not_contains "verdict class carries no member Rust lacks (underscore)" 'GROOMED[[:space:])._' "$WRITER_SRC"

# Review finding: the strip is preamble-scoped. A ticket that DOCUMENTS the
# callout format quotes these exact lines lower in the body — mika#2012's own
# issue does. A body-wide grep -v would delete that documentation.
DOCUMENTING_BODY='> - **Branch:** `fix/2012/gate`
> - **Plan:** `docs/plans/p.md` (committed on branch @ `abc1234`)
> - **Grooming history:** first-pass (READY, single-pass GROOMED) — session-id: x

## Symptom
The writer emits this shape:

```
> - **Branch:** `<branch>`
> - **Plan:** `<path>`
> - **Grooming history:** <verdict>
```

That block must survive body rewrites.'
PREAMBLE_STRIPPED=$(printf '%s' "$DOCUMENTING_BODY" | awk '
    BEGIN { preamble = 1 }
    preamble && /^> - \*\*(Branch|Plan|Grooming history):\*\*/ { next }
    preamble && /^[[:space:]]*$/ { next }
    { preamble = 0 }
    { print }
')
assert_eq "fixture carries 6 callout-shaped lines (3 preamble + 3 documented)" "6" \
    "$(printf '%s' "$DOCUMENTING_BODY" | grep -cE '^> - \*\*(Branch|Plan|Grooming history):\*\*' || true)"
assert_eq "documented callout lines inside the body SURVIVE the strip" "3" \
    "$(printf '%s' "$PREAMBLE_STRIPPED" | grep -cE '^> - \*\*(Branch|Plan|Grooming history):\*\*' || true)"
assert_contains "documenting body keeps its prose" 'must survive body rewrites' "$PREAMBLE_STRIPPED"
assert_contains "writer uses the preamble-scoped strip" 'preamble = 1' "$WRITER_SRC"

# The stacked-body case must still fully strip under the preamble-scoped rule.
STACKED_PREAMBLE_OUT=$(printf '%s' "$STACKED_BODY" | awk '
    BEGIN { preamble = 1 }
    preamble && /^> - \*\*(Branch|Plan|Grooming history):\*\*/ { next }
    preamble && /^[[:space:]]*$/ { next }
    { preamble = 0 }
    { print }
')
assert_eq "preamble strip still clears BOTH stacked blocks" "0" \
    "$(printf '%s' "$STACKED_PREAMBLE_OUT" | grep -cE '^> - \*\*(Branch|Plan|Grooming history):\*\*' || true)"
assert_contains "preamble strip preserves content after stacked blocks" '## Symptom' "$STACKED_PREAMBLE_OUT"

# Review finding: the gate must not read FETCH_HEAD. It is a single file shared
# by every process on the checkout, and mika#1001 allows a concurrent
# implement+groom pair on the same sub-repo. A sibling fetch between our fetch
# and our cat-file would test the wrong branch's tree.
PLAN_GATE_SRC=$(declare -f _committed_plan_on_branch)
assert_not_contains "gate does not read the shared FETCH_HEAD" 'FETCH_HEAD' "$PLAN_GATE_SRC"
assert_contains "gate fetches into a branch-named ref" 'refs/dispatch-gate/' "$PLAN_GATE_SRC"

# ----------------------------------------------------------------------------
# Redundant-groom refusal gate (mika#2012)
# ----------------------------------------------------------------------------
# _committed_plan_on_branch is the gate's evidence step: it answers "is there a
# committed plan on the dispatch branch?", NOT "does the body mention a plan?".
# The distinction is the whole ticket. A body-only grep would refuse grooming
# for a ticket whose plan was never pushed or was deleted — stranding it
# forever, which is a worse failure than the loop it fixes.

# The five cases below are DEFINED here next to the gate's code-shape
# assertions, but INVOKED near the end of this file — they need
# `_fixture_setup`, which is defined further down.
CALLOUT_PRESENT_BODY='## Symptom
Some text.

> - **Branch:** `fix/2012/plan-gate`
> - **Plan:** `docs/plans/2026-08-27-001-plan.md` (committed on branch @ `deadbeef`)
> - **Grooming history:** first-pass (READY, single-pass GROOMED) — no second pass required — session-id: t1
'

# --- Case 1 (the discriminating one): callout present, file ABSENT on branch.
# The gate must NOT fire — this ticket genuinely needs grooming.
test_groom_gate_callout_present_file_absent() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    # Branch exists on the remote but carries NO plan file.
    git -C "$FIXTURE_CLONE" checkout -q -b fix/2012/plan-gate
    echo "code" > "$FIXTURE_CLONE/src.txt"
    git -C "$FIXTURE_CLONE" add src.txt
    git -C "$FIXTURE_CLONE" commit -q -m "work, no plan"
    git -C "$FIXTURE_CLONE" push -q origin fix/2012/plan-gate

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/plan-gate" "$CALLOUT_PRESENT_BODY" "mika" >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then echo "PASS"; else echo "FAIL: gate fired on a callout whose plan file is absent — would strand the ticket"; fi
}

# --- Case 2: callout present AND file committed on branch → gate fires.
test_groom_gate_plan_committed() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b fix/2012/plan-gate
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    echo "# plan" > "$FIXTURE_CLONE/docs/plans/2026-08-27-001-plan.md"
    git -C "$FIXTURE_CLONE" add docs/plans/2026-08-27-001-plan.md
    git -C "$FIXTURE_CLONE" commit -q -m "commit plan"
    git -C "$FIXTURE_CLONE" push -q origin fix/2012/plan-gate

    local out rc=0
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/plan-gate" "$CALLOUT_PRESENT_BODY" "mika" 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "docs/plans/2026-08-27-001-plan.md" ]; then
        echo "PASS"
    else
        echo "FAIL: expected rc=0 and the plan path, got rc=$rc out='$out'"
    fi
}

# --- Case 3: repo-prefixed callout form (`mika/docs/plans/...`) still resolves.
# Tickets groomed before U3's path normalization carry this shape.
test_groom_gate_repo_prefixed_path() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b fix/2012/prefixed
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    echo "# plan" > "$FIXTURE_CLONE/docs/plans/legacy-plan.md"
    git -C "$FIXTURE_CLONE" add docs/plans/legacy-plan.md
    git -C "$FIXTURE_CLONE" commit -q -m "commit plan"
    git -C "$FIXTURE_CLONE" push -q origin fix/2012/prefixed

    local prefixed_body out rc=0
    prefixed_body='> - **Plan:** `mika/docs/plans/legacy-plan.md` (committed on branch @ `abc1234`)'
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/prefixed" "$prefixed_body" "mika" 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "docs/plans/legacy-plan.md" ]; then
        echo "PASS"
    else
        echo "FAIL: repo-prefixed callout should resolve to the relative path, got rc=$rc out='$out'"
    fi
}

# --- Case 4: no Plan callout at all → gate never fires (ungroomed ticket).
test_groom_gate_no_callout() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "main" "## Symptom
Plain ungroomed body, no callout." "mika" >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then echo "PASS"; else echo "FAIL: gate fired on a body with no Plan callout"; fi
}

# --- Case 5: branch does not exist on the remote → gate must not fire.
test_groom_gate_branch_absent() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "fix/2012/never-pushed" "$CALLOUT_PRESENT_BODY" "mika" >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then echo "PASS"; else echo "FAIL: gate fired although the dispatch branch does not exist on the remote"; fi
}

# ----------------------------------------------------------------------------
# Issue binding in the dispatch gate (mika#2034)
# ----------------------------------------------------------------------------
# Cases 1–5 above all run against a fixture whose `main` carries no plan files.
# That is precisely why they were green while the defect was live: the fixture
# could not express the production shape, where every dispatch branch descends
# from a `main` carrying 769 plans and therefore resolves ANY valid plan path.
# The gate read the path out of the ticket's own callout and asked only whether
# it resolved — an attestation produced by the claim it was meant to check.
#
# Measured 2026-08-30, both stranded by that gate:
#   mika#1887 → a plan whose header reads `issue: senara-solutions/mika#1933`
#   mika#2026 → a plan whose header reads `**Issue:** #539`
#
# Case 6 is the one that would have caught them.

# Writes a plan file on `main` whose header claims $2, and pushes. The branch in
# $3 is then created on top and adds NOTHING — so the plan resolves on it purely
# by inheritance, exactly as in production.
_fixture_foreign_plan_on_main() {
    local plan_rel="$1" claims_issue="$2" branch="$3"
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    {
        printf -- '---\n'
        printf 'issue: senara-solutions/mika#%s\n' "$claims_issue"
        printf 'type: fix\n'
        printf -- '---\n\n'
        printf '# Plan — a plan belonging to another ticket\n\n'
        printf 'Padding so the file clears any size filter. %s\n' \
            "$(head -c 600 /dev/zero | tr '\0' 'x')"
    } > "$FIXTURE_CLONE/$plan_rel"
    git -C "$FIXTURE_CLONE" add "$plan_rel"
    git -C "$FIXTURE_CLONE" commit -q -m "plan for another ticket, on main"
    git -C "$FIXTURE_CLONE" push -q origin main

    git -C "$FIXTURE_CLONE" checkout -q -b "$branch"
    echo "code" > "$FIXTURE_CLONE/src.txt"
    git -C "$FIXTURE_CLONE" add src.txt
    git -C "$FIXTURE_CLONE" commit -q -m "work, no plan of its own"
    git -C "$FIXTURE_CLONE" push -q origin "$branch"
}

FOREIGN_PLAN_REL='docs/plans/2026-08-21-002-fix-1933-reader-completed-section-plan.md'
FOREIGN_PLAN_BODY='## Symptom
Some text.

> - **Branch:** `feat/1887/research-peer-b`
> - **Plan:** `docs/plans/2026-08-21-002-fix-1933-reader-completed-section-plan.md` (committed on branch @ `deadbeef`)
'

# --- Case 6 (the discriminating one): the plan the body names belongs to
# ANOTHER ticket and the branch merely inherited it from main. The gate must NOT
# fire — this ticket has never been groomed.
test_groom_gate_foreign_plan_inherited_from_main() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    local rc=0 err
    err=$(_committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 1887 2>&1 >/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ]; then
        echo "FAIL: gate fired on a plan whose header claims issue 1933 while grooming 1887 — this is the mika#1887 / mika#2026 strand"
    elif ! grep -q -- 'dispatch_gate_groom_plan_refuted' <<<"$err"; then
        echo "FAIL: gate declined but emitted no greppable refusal diagnostic; got: $err"
    elif ! grep -q -- '1933' <<<"$err"; then
        echo "FAIL: refusal diagnostic does not name the issue the plan claims; got: $err"
    else
        echo "PASS"
    fi
}

# --- Case 7: same file, but its header claims the TARGET issue → gate fires.
# Without this, case 6 could be satisfied by refusing everything.
test_groom_gate_plan_header_claims_target_issue() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1887 "feat/1887/research-peer-b"

    local out rc=0
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 1887 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "$FOREIGN_PLAN_REL" ]; then
        echo "PASS"
    else
        echo "FAIL: a plan whose header claims the target issue must still fire the gate, got rc=$rc out='$out'"
    fi
}

# --- Case 8: the plan carries NO issue marker at all → gate still fires.
# Refutation, not confirmation: 95 of the 745 plans in docs/plans/ name no
# issue, and demanding a positive match would strand every one of them.
test_groom_gate_plan_without_issue_marker_still_fires() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    mkdir -p "$FIXTURE_CLONE/docs/plans"
    printf '# Plan — no issue marker anywhere in this header\n\nBody.\n' \
        > "$FIXTURE_CLONE/$FOREIGN_PLAN_REL"
    git -C "$FIXTURE_CLONE" add "$FOREIGN_PLAN_REL"
    git -C "$FIXTURE_CLONE" commit -q -m "markerless plan on main"
    git -C "$FIXTURE_CLONE" push -q origin main
    git -C "$FIXTURE_CLONE" checkout -q -b feat/1887/research-peer-b
    git -C "$FIXTURE_CLONE" push -q origin feat/1887/research-peer-b

    local out rc=0
    out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 1887 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "$FOREIGN_PLAN_REL" ]; then
        echo "PASS"
    else
        echo "FAIL: silence is not refutation — a markerless plan must still fire the gate, got rc=$rc out='$out'"
    fi
}

# --- Case 9: the four existing call shapes keep working with no 5th argument.
# KTD2 — the parameter is optional; a required one would break all five cases
# above and both production call sites.
test_groom_gate_issue_arg_is_optional() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    # No 5th argument and no ambient ISSUE_NUM: the binding check is skipped
    # rather than guessed, so the gate behaves exactly as it did before mika#2034.
    local out rc=0
    out=$(ISSUE_NUM='' _committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "$FOREIGN_PLAN_BODY" "mika" 2>/dev/null) || rc=$?

    _fixture_cleanup
    if [ "$rc" -eq 0 ] && [ "$out" = "$FOREIGN_PLAN_REL" ]; then
        echo "PASS"
    else
        echo "FAIL: the 5th argument must be optional, got rc=$rc out='$out'"
    fi
}

# --- Case 10: provenance is measured, not asserted. A blob identical to main's
# is reported as inherited; one the branch actually added is reported as
# committed. This is the sentence the operator reads.
test_plan_provenance_distinguishes_inherited_from_committed() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    # Prime the gate ref the same way _committed_plan_on_branch does.
    git -C "$FIXTURE_CLONE" fetch --quiet --force origin \
        "refs/heads/feat/1887/research-peer-b:refs/dispatch-gate/feat/1887/research-peer-b" 2>/dev/null

    local inherited committed own_plan='docs/plans/2026-08-30-001-fix-2034-own-plan.md'
    inherited=$(_plan_provenance "$FIXTURE_CLONE" "feat/1887/research-peer-b" "$FOREIGN_PLAN_REL")

    printf -- '---\nissue: senara-solutions/mika#1887\n---\n\nOwn plan.\n' \
        > "$FIXTURE_CLONE/$own_plan"
    git -C "$FIXTURE_CLONE" add "$own_plan"
    git -C "$FIXTURE_CLONE" commit -q -m "this branch's own plan"
    git -C "$FIXTURE_CLONE" push -q origin feat/1887/research-peer-b
    git -C "$FIXTURE_CLONE" fetch --quiet --force origin \
        "refs/heads/feat/1887/research-peer-b:refs/dispatch-gate/feat/1887/research-peer-b" 2>/dev/null
    committed=$(_plan_provenance "$FIXTURE_CLONE" "feat/1887/research-peer-b" "$own_plan")

    _fixture_cleanup
    if ! grep -q -- 'inherited unchanged from main' <<<"$inherited"; then
        echo "FAIL: a blob identical to main's must be reported as inherited, got '$inherited'"
    elif ! grep -q -- 'committed on the dispatch branch' <<<"$committed"; then
        echo "FAIL: a blob the branch added must be reported as committed, got '$committed'"
    else
        echo "PASS"
    fi
}

# --- Case 11: a callout naming a DIRECTORY must not fire the gate.
# Written to pin the unbindable-candidate path and it found a real one instead:
# `cat-file -e` is satisfied by a tree, and `git show` on a tree prints a
# listing — non-empty, carrying no issue header, therefore "not refuted". A
# ticket whose callout said `docs/plans` was refused grooming on the strength of
# a directory. The gate now demands a blob.
test_groom_gate_declines_on_directory_candidate() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    _fixture_foreign_plan_on_main "$FOREIGN_PLAN_REL" 1933 "feat/1887/research-peer-b"

    local rc=0
    _committed_plan_on_branch "$FIXTURE_CLONE" "feat/1887/research-peer-b" \
        "> - **Plan:** \`docs/plans\` (committed on branch @ \`deadbeef\`)" \
        "mika" 1887 >/dev/null 2>&1 || rc=$?

    _fixture_cleanup
    if [ "$rc" -ne 0 ]; then
        echo "PASS"
    else
        echo "FAIL: a directory is not a plan — the gate must not fire on one"
    fi
}

# ----------------------------------------------------------------------------
# mika#2158 — le croisement contractuel Rust ↔ Bash (AC7)
# ----------------------------------------------------------------------------
#
# Trois composants répondent à « ce ticket est-il groomé ? » : `is_groomed`
# (promotion, Rust), `check_grooming_markers` (routage du dispatch, Rust) et
# cette garde (refus, Bash). Les deux premiers sont croisés entre eux dans
# `grooming_marker::tests::ac7_both_rust_predicates_agree_on_the_frozen_bodies`.
# Le troisième ne peut pas l'être au même endroit : il mesure un artefact git
# que le Rust ne peut pas évaluer sans I/O.
#
# Le croisement testable est donc unidirectionnel, et c'est celui qui compte :
#
#     si la garde refuse pour `already_groomed`, alors le Rust doit dire groomé.
#
# C'est exactement l'implication qui était fausse avant mika#2158, et c'est ce
# qui faisait le livelock : la garde refusait le grooming de #2108/#2127 parce
# que leur plan était sur la branche, pendant que le Rust les déclarait non
# groomés et les redemandait en grooming toutes les dix minutes. Douze refus en
# trois heures et demie, sans qu'aucun état ne change entre deux tours.
#
# L'implication INVERSE n'est pas vraie et ne doit pas être testée : un ticket
# peut être groomé et son plan absent de la branche (jamais poussé), et c'est le
# cas que la branche `elif` du site d'appel traite déjà.
#
# La moitié Rust est vérifiée ici structurellement, pas paraphrasée : le test
# échoue si la fixture n'est plus dans la table Rust. Retirer `2108.md` de
# `grooming_marker.rs` casse ce test — ce qui est le point.
GROOMING_FIXTURE_DIR="$SCRIPT_DIR/../../../crates/mika-agent/tests/fixtures/grooming_bodies"
GROOMING_MARKER_RS="$SCRIPT_DIR/../../../crates/mika-agent/src/grooming_marker.rs"

test_groom_gate_refusal_implies_rust_says_groomed() {
    local ticket fixture body plan_path branch rc out problems=""

    if [ ! -d "$GROOMING_FIXTURE_DIR" ]; then
        echo "FAIL: fixture dir absent: $GROOMING_FIXTURE_DIR"
        return 0
    fi
    if [ ! -f "$GROOMING_MARKER_RS" ]; then
        echo "FAIL: grooming_marker.rs absent: $GROOMING_MARKER_RS"
        return 0
    fi

    # Les deux corps que la garde refusait pendant que le Rust les redemandait :
    # #2108 (première passe READY, AC1) et #2127 (GROOMED après arbitrage, AC3).
    for ticket in 2108 2127; do
        fixture="$GROOMING_FIXTURE_DIR/$ticket.md"
        if [ ! -f "$fixture" ]; then
            problems="$problems; fixture $ticket.md absente"
            continue
        fi
        # Moitié Rust : la fixture doit être dans la table du module.
        if ! grep -qF "grooming_bodies/$ticket.md" "$GROOMING_MARKER_RS"; then
            problems="$problems; $ticket.md n'est plus dans la table Rust de grooming_marker.rs"
            continue
        fi

        body=$(cat "$fixture")
        plan_path=$(printf '%s\n' "$body" \
            | sed -n 's/^> - \*\*Plan:\*\* *`\([^`]*\)`.*/\1/p' | head -1)
        branch=$(printf '%s\n' "$body" \
            | sed -n 's/^> - \*\*Branch:\*\* *`\([^`]*\)`.*/\1/p' | head -1)
        if [ -z "$plan_path" ] || [ -z "$branch" ]; then
            problems="$problems; $ticket.md ne porte pas les callouts Branch/Plan"
            continue
        fi

        _fixture_setup
        _assert_fixture_is_local || return 1

        git -C "$FIXTURE_CLONE" checkout -q -b "$branch"
        mkdir -p "$FIXTURE_CLONE/$(dirname "$plan_path")"
        printf '# Plan — mika#%s\n' "$ticket" > "$FIXTURE_CLONE/$plan_path"
        git -C "$FIXTURE_CLONE" add "$plan_path"
        git -C "$FIXTURE_CLONE" commit -q -m "commit plan for $ticket"
        git -C "$FIXTURE_CLONE" push -q origin "$branch"

        rc=0
        out=$(_committed_plan_on_branch "$FIXTURE_CLONE" "$branch" "$body" "mika" "$ticket" 2>/dev/null) || rc=$?
        _fixture_cleanup

        if [ "$rc" -ne 0 ] || [ "$out" != "$plan_path" ]; then
            problems="$problems; $ticket: la garde n'a pas refusé (rc=$rc out='$out', attendu '$plan_path')"
        fi
    done

    if [ -z "$problems" ]; then
        echo "PASS"
    else
        echo "FAIL${problems}"
    fi
}

# Code-shape: the binding must not be quietly dropped by a later edit. The gate
# is the only thing standing between a false body callout and a permanent
# strand, so its call to the refutation helper is an assertion of its own.
PLAN_GATE_SRC_2034=$(declare -f _committed_plan_on_branch)
assert_contains "gate binds the candidate to the issue before believing it" \
    '_plan_header_refutes_issue' "$PLAN_GATE_SRC_2034"
assert_contains "gate names the issue the refuted plan claims" \
    '_plan_header_claimed_issues' "$PLAN_GATE_SRC_2034"
assert_contains "gate emits a greppable refusal diagnostic" \
    'dispatch_gate_groom_plan_refuted' "$PLAN_GATE_SRC_2034"
# Provenance lives outside the gate on purpose: every call site invokes the gate
# in a command substitution, and a subshell cannot set a variable in its parent.
assert_not_contains "gate keeps stdout to the plan path alone" \
    'COMMITTED_PLAN_PROVENANCE=' "$PLAN_GATE_SRC_2034"
PLAN_PROVENANCE_SRC=$(declare -f _plan_provenance)
assert_contains "provenance compares the branch blob against main's" \
    'origin/main:' "$PLAN_PROVENANCE_SRC"

# --- mika#2606: the reader's two patterns have ONE definition site ---
# `_plan_header_claimed_issues` reads a plan header by two branches: a `grep`
# for `#N`, and a `sed` that strips the prefix so a `grep` can see a BARE
# numeric value (`issue: 1679`, mika#1617). Each used to carry its own copy of
# the prefix. Widening one alone leaves the other blind on exactly the lines the
# first just admitted — measured on mika#2606: `issue : 1679`, `> issue: 1679`
# and `> issue : 1679` all return empty while EVERY `#N` fixture stays green.
# A half-fix is therefore green on the whole `#N` population and silently broken
# only on the bare-numeric shapes, which is this ticket's own failure class one
# notch later. A behavioural test cannot catch the class: recopying the prefix
# makes no decision wrong on the day it is written.
# `|| true` so a rename produces a NAMED failure instead of an unexplained
# abort: under `set -e`, `declare -f` on a missing function kills the suite
# before the anti-vacuity assertion below can say what went wrong. Verified by
# renaming the function — without it the run stops with exit 1 and no diagnostic.
CLAIMED_SRC_2606=$(declare -f _plan_header_claimed_issues || true)

# Anti-vacuity FIRST. Without it, a rename or deletion makes every assertion
# below pass against an empty string, and a scan that looks at nothing reads
# exactly like a clean tree — literally the mika#2205 class this ticket closes.
assert_contains "mika#2606: the header reader still exists (anti-vacuity)" \
    '_plan_header_claimed_issues (' "$CLAIMED_SRC_2606"

assert_contains "mika#2606: the line prefix has one definition" \
    'local hdr=' "$CLAIMED_SRC_2606"
assert_contains "mika#2606: the label/value separator has one definition" \
    'local sep=' "$CLAIMED_SRC_2606"
assert_contains "mika#2606: the grep interpolates both, never its own copy" \
    '${hdr}(ticket|issue|number)${sep}' "$CLAIMED_SRC_2606"
assert_contains "mika#2606: the sed interpolates both, never its own copy" \
    '${hdr}[A-Za-z]+${sep}' "$CLAIMED_SRC_2606"

# The teeth: each literal may appear exactly once — inside its own definition.
# A future editor who inlines the prefix back into either pattern goes red here
# instead of reopening the desynchronisation.
assert_eq "mika#2606: the dash-prefix literal appears once (in hdr alone)" \
    "1" "$(grep -oF -- '(-[[:space:]]+)?' <<<"$CLAIMED_SRC_2606" | wc -l | tr -d '[:space:]')"
assert_eq "mika#2606: the colon literal appears once (in sep alone)" \
    "1" "$(grep -oF -- '(:\*\*|:)' <<<"$CLAIMED_SRC_2606" | wc -l | tr -d '[:space:]')"

# Code-shape: the gate must use mika#988 exit semantics — _deliver_callback +
# exit 0, never exit 1. An `exit 1` here is wrapped as HANDLER CRASH by the EXIT
# trap and stalls the loop (7 h on 2026-05-06).
SETUP_WT_SRC=$(declare -f _set_up_worktree)
assert_contains "groom refusal gate is scoped to dev-groom" '[ "$SKILL" = "dev-groom" ]' "$SETUP_WT_SRC"
assert_contains "groom refusal gate calls _committed_plan_on_branch" '_committed_plan_on_branch' "$SETUP_WT_SRC"
assert_contains "groom refusal delivers a structured callback" 'already_groomed' "$SETUP_WT_SRC"
assert_contains "groom refusal emits a greppable operator diagnostic" 'dispatch_gate_groom_refused' "$SETUP_WT_SRC"

# --- Re-grooming visibility (mika#2012 U4) ---
# A second grooming of the same ticket must not read like a first one. When the
# gate correctly declines to fire but the body still carries a Plan callout, the
# run is a RE-groom on a stale claim — it proceeds, but says so distinctly so a
# grep separates the two populations.
assert_contains "allowed-but-stale re-groom emits its own signal" 'dispatch_gate_groom_allowed_stale_callout' "$SETUP_WT_SRC"
assert_not_contains "the stale-callout signal is not the refusal signal reused" \
    'dispatch_gate_groom_refused: repo=${REPO} issue=${ISSUE_NUM} branch=${BRANCH} — issue body carries' "$SETUP_WT_SRC"

# The refusal RESULT must be machine-readable: mika-dev's callback turn and the
# audit dashboard both consume it. Rebuild it with the same printf and prove jq
# can reach every field.
REFUSAL_JSON=$(printf '{"status":"auto_skipped","reason":"already_groomed","issue":"senara-solutions/%s#%s","branch":"%s","plan":"%s","note":"A committed plan already exists on the dispatch branch. Re-grooming would re-derive it and stack a second body callout. Do NOT dispatch dev-pilot: the provenance gate refuses it. Remove the plan from the branch AND the grooming callouts from the issue body, then let the loop re-groom it."}' \
    "mika" "2012" "fix/2012/plan-gate" "docs/plans/2026-08-27-001-plan.md")
assert_eq "refusal RESULT is valid JSON" "0" "$(printf '%s' "$REFUSAL_JSON" | jq -e . >/dev/null 2>&1; echo $?)"
assert_eq "refusal RESULT exposes .reason to jq" "already_groomed" "$(printf '%s' "$REFUSAL_JSON" | jq -r '.reason')"
assert_eq "refusal RESULT exposes .status to jq" "auto_skipped" "$(printf '%s' "$REFUSAL_JSON" | jq -r '.status')"
assert_eq "refusal RESULT exposes .plan to jq" "docs/plans/2026-08-27-001-plan.md" "$(printf '%s' "$REFUSAL_JSON" | jq -r '.plan')"
assert_eq "refusal RESULT exposes .branch to jq" "fix/2012/plan-gate" "$(printf '%s' "$REFUSAL_JSON" | jq -r '.branch')"
assert_contains "the refusal printf in the source matches the shape tested here" \
    '{"status":"auto_skipped","reason":"already_groomed"' "$SETUP_WT_SRC"

# --- mika#2484 U5 : la note ne prescrit plus une route morte ---
# Le texte disait « Dispatch dev-pilot to implement ». Depuis mika#2287 cette
# moitié mène droit à `dispatch_grooming_not_verified` : la porte exige un
# callback groom terminé portant `Outcome: PLAN_GROOMED`, et un
# `already_groomed` n'en frappe aucun — délibérément (une garde qui lit sa
# preuve de la revendication ne peut pas la réfuter). Un texte de remède qui
# nomme une route morte coûte un tour de boucle et une lecture.
assert_not_contains "the already_groomed note no longer prescribes dev-pilot" \
    'Dispatch dev-pilot to implement' "$SETUP_WT_SRC"
assert_contains "the already_groomed note names the gesture that works" \
    'remove the plan from the branch AND the grooming callouts from the issue body' \
    "$SETUP_WT_SRC"

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
    if grep -q -- '.claude/commands/' <<<"$staged"; then
        echo "FAIL: scaffold file was staged"
        return 1
    fi

    # Pilot-authored new file must be staged
    if ! grep -q -- 'src/new_feature.rs' <<<"$staged"; then
        echo "FAIL: pilot-authored new file was not staged"
        return 1
    fi

    # Modified tracked file must be staged
    if ! grep -q -- 'tracked.rs' <<<"$staged"; then
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
    if grep -q -- '.claude/claude-pilot.json' <<<"$staged"; then
        echo "FAIL: .claude/claude-pilot.json was staged (mika#1419 ping-pong reintroduced)"
        return 1
    fi

    # .claude/commands/ scaffold must NOT appear (mika#1288 regression).
    if grep -q -- '.claude/commands/' <<<"$staged"; then
        echo "FAIL: .claude/commands/ scaffold was staged (mika#1288 regression)"
        return 1
    fi

    # Pilot-authored file MUST be staged.
    if ! grep -q -- 'src/new_feature.rs' <<<"$staged"; then
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

# Test E (mika#1552): settings.local.json + .claude/*.local.* exclusion —
# permission allowlist file written by claude-pilot (or worktree setup) MUST NOT
# ship as pilot-authored content. cm#5 dispatch (2026-06-16) produced PR #16
# whose ONLY changed file was a 143-line .claude/settings.local.json leak,
# zero adapter implementation — the founding incident.
test_auto_rescue_excludes_settings_local_json() {
    local test_dir
    test_dir=$(mktemp -d)
    trap "rm -rf '$test_dir'" RETURN

    git -C "$test_dir" init -q
    git -C "$test_dir" commit --allow-empty -m "initial" -q

    # 1. settings.local.json — must NOT be staged (mika#1552 explicit case).
    mkdir -p "$test_dir/.claude"
    echo '{"permissions": ["Bash(ls)"]}' > "$test_dir/.claude/settings.local.json"

    # 2. hooks.local.json — must NOT be staged (mika#1552 wildcard case via .local.*).
    echo '{"PreToolUse": []}' > "$test_dir/.claude/hooks.local.json"

    # 3. Pilot-authored new file — must be staged.
    mkdir -p "$test_dir/src"
    echo "fn main() {}" > "$test_dir/src/new_feature.rs"

    # Exercise: the mika#1552 extended pathspec exclusion.
    git -C "$test_dir" add -A -- \
        ':!.claude/commands/' \
        ':!.claude/claude-pilot.json' \
        ':!.claude/settings.local.json' \
        ':!.claude/*.local.*' 2>/dev/null

    local staged
    staged=$(git -C "$test_dir" diff --cached --name-only)

    # settings.local.json must NOT appear (mika#1552 explicit exclusion).
    if grep -q -- '.claude/settings.local.json' <<<"$staged"; then
        echo "FAIL: .claude/settings.local.json was staged (mika#1552 leak)"
        return 1
    fi

    # hooks.local.json must NOT appear (mika#1552 wildcard exclusion).
    if grep -q -- '.claude/hooks.local.json' <<<"$staged"; then
        echo "FAIL: .claude/hooks.local.json was staged (mika#1552 wildcard gap)"
        return 1
    fi

    # Pilot-authored file MUST be staged.
    if ! grep -q -- 'src/new_feature.rs' <<<"$staged"; then
        echo "FAIL: pilot-authored file was not staged"
        return 1
    fi

    echo "PASS"
}

RESULT_E=$(test_auto_rescue_excludes_settings_local_json 2>/dev/null)
if [ "$RESULT_E" = "PASS" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ settings.local.json + *.local.* excluded (mika#1552 founding case)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ settings.local.json exclusion: $RESULT_E"
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
# mika#2031 moved the rescue body out of _post_flight_recovery into
# _rescue_dirty_worktree(), so the extraction anchors on the function rather than
# on the inline `Unit 1` comment. Same assertions, same block — new address.
RESCUE_BLOCK=$(sed -n '/^_rescue_dirty_worktree() {/,/^}$/p' "$DISPATCH_LIB")
# mika#2348 D2 hoisted the literal exclusions into the shared
# RESCUE_EXCLUDE_PATHSPEC array (one list, every add site). The intent is
# unchanged — scaffold paths never land in a rescue commit — so the assertion
# follows the array: the add line must expand it, and the definition must still
# carry the scaffold exclusions.
RESCUE_EXCLUDE_DEF=$(grep -E '^RESCUE_EXCLUDE_PATHSPEC=\(' "$DISPATCH_LIB" | head -1)
assert_contains "Rescue git add expands RESCUE_EXCLUDE_PATHSPEC (mika#2348 D2)" \
    'add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}"' "$RESCUE_BLOCK"
assert_contains "RESCUE_EXCLUDE_PATHSPEC excludes :!.claude/commands/" \
    ":!.claude/commands/" "$RESCUE_EXCLUDE_DEF"
assert_contains "RESCUE_EXCLUDE_PATHSPEC excludes :!.claude/claude-pilot.json" \
    ":!.claude/claude-pilot.json" "$RESCUE_EXCLUDE_DEF"
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
    if ! grep -qF -- "PIPELINE FAILURE" <<<"$RESULT"; then
        failures="${failures}RESULT missing PIPELINE FAILURE; "
    fi
    if ! grep -qF -- "cargo fmt stderr:" <<<"$RESULT"; then
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
    if ! grep -q -- "^file://\|^/" <<<"$remote_url"; then
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
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    # Assert: mode is first-push (not diverged).
    if grep -qF -- "mode=diverged" <<<"$RESULT"; then
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
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    if ! grep -qF -- "mode=fast-forward" <<<"$RESULT"; then
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
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}RESULT missing 'Push: pushed'; "
    fi
    if ! grep -qF -- "mode=diverged" <<<"$RESULT"; then
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
#
# Note (mika#1857): the push_cmd construction was extracted into
# `_push_with_rebase_retry` (race-recovery helper). Both `_push_branch` (which
# constructs push_cmd for the primary attempt) and `_push_with_rebase_retry`
# (which reconstructs it for retries after rebase) must uphold the safety
# contract — the structural check reads BOTH function bodies as one haystack.
PUSH_FUNC_SRC="$(declare -f _push_branch)
$(declare -f _push_with_rebase_retry)"
# Verify explicit lease form (pins expected remote SHA)
if grep -q -- 'force-with-lease=.*BRANCH.*origin.*BRANCH' <<<"$PUSH_FUNC_SRC"; then
    PASS=$((PASS + 1)); echo "  ✓ Lease form: --force-with-lease uses explicit ref pinning"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Lease form: should use --force-with-lease=\$BRANCH:origin/\$BRANCH"
fi
# Verify no blind --force (without -with-lease)
if grep -q -- '--force[^-]' <<<"$PUSH_FUNC_SRC"; then
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
    if ! grep -qF -- "STATUS=REBASE_CONFLICT" <<<"$RESULT"; then
        failures="${failures}RESULT missing STATUS=REBASE_CONFLICT; "
    fi
    # Assert: conflict mode token present (AC#4).
    if ! grep -qF -- "Rebase failure mode: conflict" <<<"$RESULT"; then
        failures="${failures}RESULT missing 'Rebase failure mode: conflict'; "
    fi
    # Assert: conflicted filename present.
    if ! grep -qF -- "file.txt" <<<"$RESULT"; then
        failures="${failures}RESULT missing conflicted filename 'file.txt'; "
    fi
    # Assert: rebase stderr is non-empty (AC#4 — not /dev/null).
    if grep -qF -- "Rebase stderr: <empty>" <<<"$RESULT"; then
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
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
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

# _push_branch uses merge-base --is-ancestor for divergence detection.
# Note (mika#1857): the push_cmd construction was extracted into
# `_push_with_rebase_retry` — the force-with-lease structural check reads both
# function bodies as one haystack (same rationale as test 12d above).
PUSH_FUNC=$(declare -f _push_branch)
PUSH_FUNC_WITH_HELPER="$(declare -f _push_branch)
$(declare -f _push_with_rebase_retry)"
assert_contains "_push_branch uses merge-base --is-ancestor" "merge-base --is-ancestor" "$PUSH_FUNC"
assert_contains "_push_branch (or retry helper) uses force-with-lease" "force-with-lease" "$PUSH_FUNC_WITH_HELPER"
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
    if grep -qiE -- "divergence|abort|reconciliation|Push: pushed|Push: FAILED" <<<"$RESULT"; then
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

# --- Test 12l: résidu d'échafaudage UNTRACKED seul → aucun stash (mika#2144) ---
#
# Le trou que mika#2144 ferme, et il est étroit : `git checkout HEAD -- docs/plans/`
# restaure les entrées CONNUES de l'arbre et ne supprime jamais un untracked —
# comportement git, pas accident local. Donc un pilote de groom qui écrit son plan
# puis meurt avant de committer (signature mika#2141) laisse un `?? docs/plans/…`
# que le Tier 2 ne peut pas mordre, que le Tier 3 stashe, et que le `clean -fd`
# qui suit supprime de toute façon. Les 11 stashes du seul 2026-09-01 sont de
# cette forme.
#
# Ce test est le contrôle négatif de la correction : contre le Tier 3 NON modifié
# il DOIT échouer (un stash apparaît). Une version verte avant correction ne
# mesure pas le trou et sa verdeur ne prouverait rien (classe mika#2205).
#
# `docs/plans/` porte ici un plan COMMITTÉ en plus de l'untracked, pour que le
# reset chirurgical du Tier 2 réussisse sur son pathspec : le test attesterait
# autrement un `checkout` en échec plutôt que le trou lui-même.
test_resume_untracked_scaffold_no_stash() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b feat/untracked-plan
    mkdir -p "$FIXTURE_CLONE/docs/plans"
    echo "plan v1" > "$FIXTURE_CLONE/docs/plans/committed-plan.md"
    echo "feat" > "$FIXTURE_CLONE/feature.txt"
    git -C "$FIXTURE_CLONE" add docs/plans/committed-plan.md feature.txt
    git -C "$FIXTURE_CLONE" commit -q -m "feature + committed plan"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/untracked-plan

    # Advance origin/main (non-conflicting) so BEHIND>0 and the rebase replays.
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

    # Le résidu réellement producteur : un plan de groom jamais committé.
    echo "plan the pilot never committed" \
        > "$FIXTURE_CLONE/docs/plans/2026-09-29-001-chore-2144-untracked-plan.md"

    WORKTREE_DIR="$FIXTURE_CLONE"
    LOG_ID="test-2144-untracked"
    RESUME_CLEANUP_STASH=""

    # stderr dans un FICHIER, jamais `$(… 2>&1)` : une substitution de commande
    # ouvre un sous-shell, et `RESUME_CLEANUP_STASH` — l'effet de bord dont tout
    # ce test dépend — y resterait invisible. Mesuré : le contrôle négatif passait
    # alors sur la mauvaise assertion. Le fichier vit dans le dépôt NU, hors de
    # l'arbre de travail, donc il ne peut pas salir le `status --porcelain` qu'on
    # mesure, et `_fixture_cleanup` l'emporte avec lui.
    local stderr_file="$FIXTURE_BARE/cleanup.stderr" stderr_seen
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    stderr_seen=$(cat "$stderr_file" 2>/dev/null || true)

    local status_after
    status_after=$(git -C "$FIXTURE_CLONE" status --porcelain 2>/dev/null)
    local rebase_rc=0
    git -C "$FIXTURE_CLONE" rebase origin/main >/dev/null 2>&1 || rebase_rc=$?

    local failures=""
    # Le nettoyage est INCHANGÉ : seule l'émission du stash bouge (mika#2144 § 2).
    [ -z "$status_after" ] || failures="${failures}worktree not clean after cleanup: [$status_after]; "
    [ "$rebase_rc" -eq 0 ] || failures="${failures}rebase should succeed on clean tree, got rc=$rebase_rc; "
    # AC1 : résidu entièrement échafaudage → aucun stash, malgré l'untracked.
    [ -z "$RESUME_CLEANUP_STASH" ] || failures="${failures}untracked scaffold must NOT create a stash (got $RESUME_CLEANUP_STASH); "
    if git -C "$FIXTURE_CLONE" stash list 2>/dev/null | grep -qF "dispatch-lib-resume-cleanup-"; then
        failures="${failures}no resume-cleanup stash should exist for untracked scaffold; "
    fi
    # AC2 en négatif : le message de récupération ne doit pas être imprimé pour rien.
    if grep -qF 'resume-cleanup stashed dirty worktree' <<<"$stderr_seen"; then
        failures="${failures}recovery message printed for scaffold-only residue; "
    fi
    # U5 : l'abstention est DITE, sinon sa mesure n'existe pas.
    if ! grep -qF 'resume_cleanup_scaffold_only' <<<"$stderr_seen"; then
        failures="${failures}expected the abstention line resume_cleanup_scaffold_only; "
    fi
    if ! grep -qF 'classes=plans' <<<"$stderr_seen"; then
        failures="${failures}abstention line should name the class (classes=plans); "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12L=$(test_resume_untracked_scaffold_no_stash 2>/dev/null)
if [ "$RESULT_12L" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Resume untracked scaffold: no stash, abstention said (mika#2144)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Resume untracked scaffold no-stash (mika#2144): $RESULT_12L"
fi

# --- Test 12m: AC6 — les deux branches dans un seul test (mika#2144) ---
#
# La lettre d'AC6 : « une exécution réelle de la voie de reprise sur un worktree
# portant du `.iterate/` et rien d'autre ne crée aucun stash, et la même exécution
# avec un fichier de code modifié en crée un. Les deux, dans le même test. »
#
# `.iterate/` arrive bien au Tier 3 dans une fixture, et pas par artifice : le
# `rm -rf "$wt/.iterate"` du Tier 2 est gardé par `_assert_removable_worktree_path`,
# dont le terme T4 exige un chemin sous `/.claude/worktrees/`. Une fixture en
# `mktemp -d` est refusée — la forme exacte que R4 décrit en production (une
# branche antérieure à `ac1ee87e`, où `.iterate/` n'est ni ignoré ni forcément
# hors de l'index).
#
# La phase 2 est le contrôle négatif non négociable d'AC2 : elle doit rester verte
# avant ET après la correction. Si elle rougit, le filet est devenu muet et c'est
# la perte des 16 stashes de code en train de se rejouer.
test_resume_ac6_both_branches() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b feat/ac6-both
    echo "feat" > "$FIXTURE_CLONE/feature.txt"
    git -C "$FIXTURE_CLONE" add feature.txt
    git -C "$FIXTURE_CLONE" commit -q -m "feature work"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/ac6-both

    local failures=""
    # stderr dans un fichier, pour la raison écrite sur le test 12l : une
    # substitution de commande perdrait `RESUME_CLEANUP_STASH`. Le dépôt nu est
    # hors de l'arbre mesuré.
    local stderr_file="$FIXTURE_BARE/cleanup.stderr"

    # ---- Phase 1 : `.iterate/` et rien d'autre → aucun stash ----
    mkdir -p "$FIXTURE_CLONE/.iterate"
    echo "scratch" > "$FIXTURE_CLONE/.iterate/findings.txt"

    WORKTREE_DIR="$FIXTURE_CLONE"
    LOG_ID="test-2144-ac6-iterate"
    RESUME_CLEANUP_STASH=""
    local stderr_phase1
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    stderr_phase1=$(cat "$stderr_file" 2>/dev/null || true)

    [ -z "$RESUME_CLEANUP_STASH" ] || failures="${failures}phase1: .iterate/-only dirt must NOT stash (got $RESUME_CLEANUP_STASH); "
    if git -C "$FIXTURE_CLONE" stash list 2>/dev/null | grep -qF "dispatch-lib-resume-cleanup-test-2144-ac6-iterate"; then
        failures="${failures}phase1: no resume-cleanup stash should exist for .iterate/-only dirt; "
    fi
    if ! grep -qF 'classes=iterate' <<<"$stderr_phase1"; then
        failures="${failures}phase1: abstention line should name classes=iterate; "
    fi
    local status_p1
    status_p1=$(git -C "$FIXTURE_CLONE" status --porcelain 2>/dev/null)
    [ -z "$status_p1" ] || failures="${failures}phase1: tree not clean after cleanup: [$status_p1]; "

    # ---- Phase 2 : un fichier de code modifié → un stash, ET son message ----
    echo "real work the pilot did not commit" >> "$FIXTURE_CLONE/feature.txt"

    LOG_ID="test-2144-ac6-code"
    RESUME_CLEANUP_STASH=""
    local stderr_phase2
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    stderr_phase2=$(cat "$stderr_file" 2>/dev/null || true)

    if [ -z "$RESUME_CLEANUP_STASH" ]; then
        failures="${failures}phase2: a modified code file MUST be stashed (AC2); "
    elif ! git -C "$FIXTURE_CLONE" cat-file -e "$RESUME_CLEANUP_STASH" 2>/dev/null; then
        failures="${failures}phase2: RESUME_CLEANUP_STASH ($RESUME_CLEANUP_STASH) is not a valid git object; "
    elif ! git -C "$FIXTURE_CLONE" stash show "$RESUME_CLEANUP_STASH" 2>/dev/null | grep -qF 'feature.txt'; then
        failures="${failures}phase2: stashed content should hold the code file (feature.txt); "
    fi
    # Le message de récupération est la moitié d'AC2 que le ticket nomme.
    if ! grep -qF 'resume-cleanup stashed dirty worktree' <<<"$stderr_phase2"; then
        failures="${failures}phase2: the recovery message must still be printed; "
    fi
    if grep -qF 'resume_cleanup_scaffold_only' <<<"$stderr_phase2"; then
        failures="${failures}phase2: abstention line must NOT appear when code is present; "
    fi
    local status_p2
    status_p2=$(git -C "$FIXTURE_CLONE" status --porcelain 2>/dev/null)
    [ -z "$status_p2" ] || failures="${failures}phase2: tree not clean after cleanup: [$status_p2]; "

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12M=$(test_resume_ac6_both_branches 2>/dev/null)
if [ "$RESULT_12M" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ AC6: .iterate/-only no stash, code still stashed (mika#2144)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ AC6 both branches (mika#2144): $RESULT_12M"
fi

# --- Test 12n: renommages — le piège `R ` du format -z (mika#2144) ---
#
# `git status --porcelain -z` émet pour un renommage DEUX enregistrements
# NUL-séparés : la destination avec son préfixe `XY `, puis l'origine SANS préfixe.
# Un classificateur qui applique l'offset 3 aux deux tronque l'origine
# (`docs/plans/b.md` → `s/plans/b.md`), qui ne matche plus aucun motif.
#
# Le plan mika#2144 nomme le sens « sortant de docs/plans/ ». Livré seul, ce sens
# ne discrimine RIEN : sa destination est déjà du code, donc le premier
# enregistrement suffit à décider, et le test passerait avec ou sans le traitement
# de l'origine. Les trois volets sont donc livrés ensemble.
#
# Le volet 2 (l'origine est du code) est le contrôle de SÛRETÉ : sans lecture de
# l'origine, un `git mv code/y.rs docs/plans/d.md` verrait une destination
# d'échafaudage et cesserait de stasher du code. Fail-closed ou pas, c'est la
# perte qu'AC2 interdit.
#
# Le volet 3 est le volet DISCRIMINANT, et il vit sous `.iterate/` — pas sous
# `docs/plans/` comme le premier jet l'écrivait. Mesuré : le Tier 2 DÉMONTE un
# renommage interne à `docs/plans/` avant que le Tier 3 ne le voie
# (`checkout HEAD -- docs/plans/` restaure l'origine et laisse la destination
# seule → un seul enregistrement, `paths=1`), donc ce sens-là n'atteint jamais le
# chemin à deux enregistrements. `.iterate/` y arrive, et pas par artifice : le
# `rm -rf` du Tier 2 est refusé sur une fixture hors de `/.claude/worktrees/`, et
# un `.iterate/` TRACKÉ est exactement la population que R4 décrit (une branche
# antérieure à `ac1ee87e`, dont le commit s'intitule *untrack*). Sous un offset 3
# appliqué à tort à l'origine, `terate/a` ne matche plus rien et le résidu est
# stashé : ce volet rougit.
test_resume_rename_records() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    git -C "$FIXTURE_CLONE" checkout -q -b feat/renames
    mkdir -p "$FIXTURE_CLONE/docs/plans" "$FIXTURE_CLONE/code"
    # Contenu identique et assez long pour que la détection de renommage de
    # `git status` (activée par défaut) apparie les deux côtés.
    local filler
    filler=$(printf 'ligne de contenu stable %s\n' 1 2 3 4 5 6 7 8 9 10)
    mkdir -p "$FIXTURE_CLONE/.iterate"
    printf '%s' "$filler" > "$FIXTURE_CLONE/docs/plans/a-plan.md"
    printf '%s' "$filler" > "$FIXTURE_CLONE/code/y.rs"
    # `.iterate/` TRACKÉ : la population de R4 (branche antérieure à `ac1ee87e`).
    printf '%s' "$filler" > "$FIXTURE_CLONE/.iterate/findings-a.txt"
    git -C "$FIXTURE_CLONE" add docs/plans code .iterate
    git -C "$FIXTURE_CLONE" commit -q -m "plans + code + tracked .iterate"
    git -C "$FIXTURE_CLONE" push -q -u origin feat/renames

    WORKTREE_DIR="$FIXTURE_CLONE"
    local failures="" stderr_file="$FIXTURE_BARE/cleanup.stderr"

    # ---- Volet 1 : docs/plans/a-plan.md → code/x.rs  ⇒ STASH ----
    git -C "$FIXTURE_CLONE" mv docs/plans/a-plan.md code/x.rs
    LOG_ID="test-2144-rename-out"
    RESUME_CLEANUP_STASH=""
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    [ -n "$RESUME_CLEANUP_STASH" ] \
        || failures="${failures}volet1: a rename OUT of docs/plans/ must stash; "

    # ---- Volet 2 : code/y.rs → docs/plans/d-plan.md  ⇒ STASH (origine = code) ----
    git -C "$FIXTURE_CLONE" mv code/y.rs docs/plans/d-plan.md
    LOG_ID="test-2144-rename-in"
    RESUME_CLEANUP_STASH=""
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    [ -n "$RESUME_CLEANUP_STASH" ] \
        || failures="${failures}volet2: a rename INTO docs/plans/ from code must stash (the origin is work); "

    # ---- Volet 3 : .iterate/findings-a → .iterate/findings-b  ⇒ AUCUN stash ----
    # Le volet discriminant : les deux enregistrements sont de l'échafaudage, et
    # seul un traitement correct de l'origine sans préfixe le voit.
    git -C "$FIXTURE_CLONE" mv .iterate/findings-a.txt .iterate/findings-b.txt
    LOG_ID="test-2144-rename-internal"
    RESUME_CLEANUP_STASH=""
    _clean_worktree_for_rebase "$FIXTURE_CLONE" 2>"$stderr_file" >/dev/null || true
    local stderr_internal
    stderr_internal=$(cat "$stderr_file" 2>/dev/null || true)
    [ -z "$RESUME_CLEANUP_STASH" ] \
        || failures="${failures}volet3: a rename WITHIN .iterate/ must NOT stash (got $RESUME_CLEANUP_STASH); "
    if ! grep -qF 'resume_cleanup_scaffold_only' <<<"$stderr_internal"; then
        failures="${failures}volet3: expected the abstention line for an internal rename; "
    fi
    # Les deux enregistrements comptent : destination ET origine.
    if ! grep -qE 'resume_cleanup_scaffold_only paths=2 classes=iterate ' <<<"$stderr_internal"; then
        failures="${failures}volet3: both rename records should be counted (paths=2 classes=iterate), got [$stderr_internal]; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_12N=$(test_resume_rename_records 2>/dev/null)
if [ "$RESULT_12N" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ Renames: both -z records classified (mika#2144)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Rename -z records (mika#2144): $RESULT_12N"
fi

# --- mika#2144 : `_is_scaffold_path` est le seul site déclarant LA LISTE --------
#
# mika#2157 (R3) écrivait déjà, dans dispatch-lib.sh, que ses deux autorités
# « peuvent diverger ; quand vous ajoutez un chemin ici, ajoutez-le au
# classificateur aussi ». C'était une consigne sans garde. Celle-ci l'arme.
#
# LE PRÉDICAT PORTE SUR LA LISTE, JAMAIS SUR LE LITTÉRAL ISOLÉ — et ce n'est pas
# un assouplissement, c'est la correction d'un premier jet mesuré faux. Un scan
# « un littéral d'échafaudage en position de motif de `case` » accuse
# `_extract_plan_path` (`docs/plans/*)`, dispatch-lib.sh), qui ne déclare pas
# l'échafaudage : il normalise le chemin porté par le callout d'une issue, et son
# second motif `*/docs/plans/*` est la forme PRÉFIXÉE PAR LE DÉPÔT, que le Tier 2
# ne remet justement pas à HEAD. Le router vers `_is_scaffold_path` élargirait la
# liste du Tier 3 à un chemin jamais restauré : très exactement la perte
# silencieuse que R5 refuse. Ce n'est donc pas une exception à allowlister, c'est
# une autre notion — et la notion surveillée ici est l'ENSEMBLE des quatre
# chemins. Un bloc `case` qui en porte DEUX OU PLUS redéclare la liste.
#
# BORNE NOMMÉE, pas découverte (modèle T12j) : un site isolé portant UN SEUL de
# ces chemins pour une raison qui lui est propre est hors population. Le jour où
# quelqu'un scinde la liste en plusieurs `case` d'un motif chacun, ce scan ne le
# voit pas — on élargit alors le prédicat ici, dans le même commit.
#
# Rend : « introuvable », ou le nombre de blocs `case` redéclarant la liste.
_t2144_scaffold_decl_scan() {
    local file="$1" body
    [ -r "$file" ] || { printf 'introuvable'; return 0; }
    # Le corps du site unique est retiré AVANT le comptage : c'est lui la
    # déclaration légitime, et le scan cherche ses concurrents.
    body=$(sed '/^_is_scaffold_path()/,/^}/d' "$file" | grep -v '^[[:space:]]*#' || true)
    printf '%s\n' "$body" | awk '
        BEGIN { inblk = 0; viol = 0 }
        function flush(   k, n) {
            n = 0
            for (k in seen) n++
            if (n >= 2) viol++
            delete seen
        }
        {
            if ($0 ~ /(^|[[:space:];&|(])case[[:space:]]/) inblk = 1
            if (inblk) {
                if ($0 ~ /^[[:space:]]*\.claude\/groom-verdict-trail\.log[^[:space:]]*\)/) seen["trail"] = 1
                if ($0 ~ /^[[:space:]]*\.claude\/commands\/[^[:space:]]*\)/)               seen["commands"] = 1
                if ($0 ~ /^[[:space:]]*\.iterate\/[^[:space:]]*\)/)                        seen["iterate"] = 1
                if ($0 ~ /^[[:space:]]*docs\/plans\/[^[:space:]]*\)/)                      seen["plans"] = 1
            }
            if ($0 ~ /(^|[[:space:];&|])esac([[:space:]]|;|$)/) { flush(); inblk = 0 }
        }
        END { if (inblk) flush(); print viol + 0 }
    '
}

T2144_DECL_SCAN=$(_t2144_scaffold_decl_scan "$DISPATCH_LIB")
assert_eq "mika#2144: dispatch-lib.sh lisible par le scan de déclaration" "lu" \
    "$([ "$T2144_DECL_SCAN" != introuvable ] && echo lu || echo "fichier introuvable: $DISPATCH_LIB")"
assert_eq "mika#2144: aucun second bloc case ne redéclare la liste d'échafaudage" "0" \
    "$T2144_DECL_SCAN"

# Anti-vacuité (i) : le scan viserait un nom mort si la fonction disparaissait, et
# un scan sur un nom mort se lit exactement comme un arbre propre (mika#2205).
assert_eq "mika#2144 anti-vacuité: _is_scaffold_path existe dans dispatch-lib.sh" "1" \
    "$(grep -c '^_is_scaffold_path()' "$DISPATCH_LIB" || true)"
assert_eq "mika#2144 anti-vacuité: le site unique porte bien les QUATRE motifs" "4" \
    "$(sed -n '/^_is_scaffold_path()/,/^}/p' "$DISPATCH_LIB" \
       | grep -cE '^[[:space:]]*(\.claude/groom-verdict-trail\.log|\.claude/commands/|\.iterate/|docs/plans/)[^[:space:]]*\)' || true)"

# Anti-vacuité (ii) : le prédicat MORD, exercé à chaque exécution plutôt qu'une
# fois à la main. Un `case` portant deux des quatre chemins est compté…
assert_eq "mika#2144 anti-vacuité: un case redéclarant deux chemins est vu" "1" \
    "$(_t2144_scaffold_decl_scan <(printf '%s\n' \
        'f() {' '    case "$p" in' '        .iterate/*) ;;' '        docs/plans/*) ;;' '    esac' '}'))"
# …et un `case` n'en portant qu'un seul ne l'est pas : c'est la borne ci-dessus,
# celle qui laisse `_extract_plan_path` tranquille.
assert_eq "mika#2144 borne: un case portant un seul chemin reste hors population" "0" \
    "$(_t2144_scaffold_decl_scan <(printf '%s\n' \
        'f() {' '    case "$p" in' '        docs/plans/*) ;;' '        */docs/plans/*) ;;' '    esac' '}'))"
# …et un fichier illisible ROUGIT au lieu de rendre zéro violation.
assert_eq "mika#2144 anti-vacuité: un fichier absent rend « introuvable »" "introuvable" \
    "$(_t2144_scaffold_decl_scan /nonexistent/mika-2144.sh)"

# Option (a) de la Fire-Disposition : table d'exceptions livrée VIDE, vacuité
# assertée à l'exécution (modèle T2544_HEADING_READER_ALLOWLIST). Quand le scan
# tire, la résolution est de router le site vers `_is_scaffold_path` ; on n'ajoute
# pas de ligne ici (doctrine mika#2201).
T2144_SCAFFOLD_PATTERN_DECLARATION_ALLOWED=()
assert_eq "mika#2144: table d'exceptions du scan de déclaration — zero entries" "0" \
    "${#T2144_SCAFFOLD_PATTERN_DECLARATION_ALLOWED[@]}"

# Les deux consommateurs lisent le site unique, et c'est ce qui rend la garde
# ci-dessus autre chose qu'une assertion sur du texte mort.
assert_eq "mika#2144: _is_scaffold_path a exactement deux consommateurs" "2" \
    "$(grep -v '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '_is_scaffold_path "' || true)"

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
# mika#2165 moved the directory behind the single resolver `_pilot_log_dir`;
# the LOG_ID convention this assertion exists for is unchanged, only its
# spelling is. The default literal now lives at exactly one site, pinned below.
# The resolver is called ON THE SAME LINE as the read — an assigning accessor
# (not a printing one, see the mika#2039 collision pinned below) can go stale,
# and co-location is what stops it.
assert_contains "PERSISTENT_STDERR_PATH uses LOG_ID convention" \
    '_pilot_log_dir; PERSISTENT_STDERR_PATH="$_PILOT_LOG_DIR/${LOG_ID}.stderr"' "$DRIFT_BLOCK"
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
# mika#2312: the remedies paragraph used to send the reader straight to
# "widen the policy" / "rewrite the dispatch context" without ever saying the
# deny names a refused CALL and a RULE. That reading is what produced mika#2312's
# inference that the CONTENT of a system_prompt.md was protected.
assert_contains "Class C message teaches rule-id reading first (dev-groom site)" \
    "Read the halt event's bracketed [rule-id] FIRST" "$DRIFT_BLOCK"
# The absent-rule-id semantics are the half mika#2312's first attempt got
# BACKWARDS: rule_id=None is claude-pilot's policy DEFAULT deny (policy.py
# returns rule_id=None when no rule matched; ui.py renders no tag for it), and
# permissions.yaml's default reason prescribes widening. Pin the corrected
# claim, not merely the sentence's presence — a message that is present and
# false is what this ticket exists to repair.
assert_contains "Class C message says an absent rule-id is the policy DEFAULT (dev-groom site)" \
    'NO [rule-id] means the policy DEFAULT fired' "$DRIFT_BLOCK"
assert_not_contains "Class C message does not blame the judgment stage (dev-groom site)" \
    'canUseTool judgment stage' "$DRIFT_BLOCK"

# Branch ordering: the POLICY_DENY branch must be the FIRST elif/if, so it wins
# over the drift messages when both conditions could fire.
POLICY_DENY_LINE=$(echo "$DRIFT_BLOCK" | grep -n 'if \[ -n "\$POLICY_DENY" \]' | head -1 | cut -d: -f1)
DRIFT_MSG_LINE=$(echo "$DRIFT_BLOCK" | grep -n 'pilot drifted into executor mode' | head -1 | cut -d: -f1)
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
    '_pilot_log_dir; PERSISTENT_STDERR_PATH="$_PILOT_LOG_DIR/${LOG_ID}.stderr"' "$POSTFLIGHT_BLOCK"
assert_contains "Strips ANSI before grep" \
    'sed' "$POSTFLIGHT_BLOCK"
assert_contains "Searches for [policy:deny] marker" \
    '[policy:deny]' "$POSTFLIGHT_BLOCK"
assert_contains "Class C message — halted by policy deny, NOT generic exit" \
    'halted by policy deny — not generic exit' "$POSTFLIGHT_BLOCK"
assert_contains "Links to investigation doc" \
    'drift-misdiagnosis-policy-deny-halt' "$POSTFLIGHT_BLOCK"
# mika#2312 — companion of the dev-groom assertions in Test 13. Same phrases,
# second site: the two class-C blocks are NOT copies of one another (their
# headers and remedies paragraphs legitimately differ), so a single assertion
# would only bite on one of them.
assert_contains "Class C message teaches rule-id reading first (dev-pilot site)" \
    "Read the halt event's bracketed [rule-id] FIRST" "$POSTFLIGHT_BLOCK"
assert_contains "Class C message says an absent rule-id is the policy DEFAULT (dev-pilot site)" \
    'NO [rule-id] means the policy DEFAULT fired' "$POSTFLIGHT_BLOCK"
assert_not_contains "Class C message does not blame the judgment stage (dev-pilot site)" \
    'canUseTool judgment stage' "$POSTFLIGHT_BLOCK"

# Reading ORDER, not just presence (mika#2312): the rule-id instruction must
# come BEFORE the allow-list-gap paragraph — a reader who reaches "widen the
# policy" first goes hunting for a gap without knowing which rule fired, which
# is the sequence that produced this ticket. assert_contains cannot see order,
# so compare source line numbers the way the POLICY_DENY branch-ordering check
# below already does.
RULEID_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n "bracketed \[rule-id\] FIRST" | head -1 | cut -d: -f1)
ALLOWLIST_LINE=$(echo "$POSTFLIGHT_BLOCK" | grep -n 'Likely a tier1 or tier2 allow-list gap' | head -1 | cut -d: -f1)
if [ -n "$RULEID_LINE" ] && [ -n "$ALLOWLIST_LINE" ] && [ "$RULEID_LINE" -lt "$ALLOWLIST_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ rule-id instruction precedes the allow-list-gap paragraph (dev-pilot site)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ rule-id instruction must precede the allow-list-gap paragraph (dev-pilot site)"
fi

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

# --- Test 15: mika#1383 structural completion gate — Phase A only, no PR (mika#1679) ---
#
# mika#1383 originally auto-created a PR when the pilot committed but didn't
# reach `gh pr create`. mika#1679 OVERTURNED that: the gate opened a NON-draft
# PR and set the global PR_URL, which SHADOWED the mika#1396 commit-pushed-no-pr
# rescue (Path B) — letting a non-draft PR bypass the mika#1613 recovery guards
# (evidence mika#PR1678/#PR1683). Under R2 the gate keeps only Phase A (trailing
# dirty rescue) and defers ALL PR creation to Path B (single source of truth).
# These tests assert the gate no longer creates a PR and the deferral is wired.

echo ""
echo "Test 15: mika#1383 gate — Phase A retained, PR creation deferred to Path B (mika#1679)"
echo "------------------------------------------------------------------------------------"

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
assert_contains "Phase A: trailing dirty rescue with wip() prefix" \
    'wip(${REPO}#${ISSUE_NUM}): trailing content after pilot end_turn (mika#1383)' "$GATE_BLOCK"
# mika#2348 D2: the exclusion is the shared RESCUE_EXCLUDE_PATHSPEC array, not
# a literal on the add line — assert the gate expands it, and that the array
# still carries the two scaffold exclusions (same list as mika#1282, by
# construction rather than by copy).
GATE_EXCLUDE_DEF=$(grep -E '^RESCUE_EXCLUDE_PATHSPEC=\(' "$DISPATCH_LIB" | head -1)
assert_contains "Phase A: same scaffold-path exclusion as mika#1282 (git add expands RESCUE_EXCLUDE_PATHSPEC)" \
    'add -A -- "${RESCUE_EXCLUDE_PATHSPEC[@]}"' "$GATE_BLOCK"
assert_contains "Phase A: RESCUE_EXCLUDE_PATHSPEC excludes :!.claude/commands/" \
    ":!.claude/commands/" "$GATE_EXCLUDE_DEF"
assert_contains "Phase A: RESCUE_EXCLUDE_PATHSPEC excludes :!.claude/claude-pilot.json" \
    ":!.claude/claude-pilot.json" "$GATE_EXCLUDE_DEF"

# mika#1679 (AC1): the gate must NOT create a PR. Path A creating a PR is what
# set PR_URL and shadowed Path B. The negative checks run against the CODE only
# (comment lines stripped) — the comments legitimately document the removed
# behavior and the shadow history, so they must not trip the regression net.
GATE_CODE=$(printf '%s\n' "$GATE_BLOCK" | grep -v '^[[:space:]]*#')
assert_not_contains "AC1: gate code no longer invokes gh pr create (deferred to Path B)" \
    'gh pr create' "$GATE_CODE"
assert_not_contains "AC1: gate code no longer lists PRs for an existence check" \
    'gh pr list --repo "senara-solutions/$REPO" --head "$BRANCH"' "$GATE_CODE"
assert_not_contains "AC1: gate code no longer emits the auto-created-PR result line" \
    'auto-created PR' "$GATE_CODE"
assert_not_contains "AC1: gate code no longer surfaces the manual-recovery PIPELINE FAILURE" \
    'PIPELINE FAILURE: pilot produced commits on' "$GATE_CODE"

# mika#1679: deferral to Path B (mika#1396 commit-pushed-no-pr) is documented in
# the gate so the shadow cannot be silently reintroduced.
assert_contains "Gate documents deferral to the mika#1396 commit-pushed-no-pr rescue" \
    'mika#1396' "$GATE_BLOCK"
assert_contains "Gate names mika#1679 as the reason PR creation moved out" \
    'mika#1679' "$GATE_BLOCK"

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

# --- Test 15b: Path B (mika#1396) owns the commit-pushed-no-pr rescue PR (mika#1679) ---
#
# After mika#1679, the mika#1383 trigger flows to Path B's commit-pushed-no-pr
# branch. Path B is the single source of truth for the rescue-PR shape: --draft,
# rescue header, RECOVERY_PENDING marker, wip-rescue label, canonical PR: line,
# plus (mika#1679 Edit 2 / AC6) a wip(mika#1383) marker commit so Guard 2's
# `isDraft AND ^wip\(` conjunction fires.

echo ""
echo "Test 15b: Path B owns commit-pushed-no-pr rescue + Guard 2 marker commit (mika#1679)"
echo "-----------------------------------------------------------------------------------"

# Extract Path B (the mika#1282 + mika#1396 draft-PR rescue) up to its callback.
PATHB_BLOCK=$(sed -n '/Unit 2 (mika#1282 + mika#1396): open a draft PR/,/^    _deliver_callback/p' "$DISPATCH_LIB")

# Needle avoids a leading '--' so grep doesn't parse it as a flag; 'draft \'
# uniquely matches the `--draft \` line of the gh pr create invocation.
assert_contains "AC3: Path B opens the rescue PR as draft" \
    'draft \' "$PATHB_BLOCK"
# mika#2157 extracted the body out of the inline heredoc into
# `_compose_rescue_pr_body` so the closing-reference decision could be tested
# against real git repositories instead of a stubbed `gh` argv. The property
# these two assertions pin is unchanged — Path B's PR body carries the rescue
# header and the verification marker — so they now follow it across the
# extraction: Path B must route through the composer, and the composer must
# emit both. Splitting the assertion is what keeps it measuring the shipped
# body rather than a heredoc that no longer exists.
assert_contains "AC3: Path B composes its body through _compose_rescue_pr_body (mika#2157)" \
    '_compose_rescue_pr_body "$WORKTREE_DIR"' "$PATHB_BLOCK"
RESCUE_BODY_FN=$(awk '/^_compose_rescue_pr_body\(\) \{/,/^\}/' "$DISPATCH_LIB")
assert_contains "AC3: Path B writes the Auto-rescued PR rescue header (qa-review Step 1.5)" \
    '## Auto-rescued PR (dispatch-lib recovery, class: ${recovery_class})' "$RESCUE_BODY_FN"
# mika#2354: the marker is no longer the hard-coded literal `no` — it is the
# measured verdict, interpolated. Asserting `rescue-pipeline-verified: no` here
# would pin the very defect that ticket exists to remove, so the assertion
# follows the property (a marker IS emitted, and it carries the measured value)
# rather than the literal. The two values it can take are covered end-to-end in
# `tests/test_rescue_pipeline_verified.sh` against real repositories.
assert_contains "AC3: Path B emits the rescue-pipeline-verified marker (mika#2354: measured, not literal)" \
    'rescue-pipeline-verified: ${verified}' "$RESCUE_BODY_FN"
assert_not_contains "AC3/mika#2354: no hard-coded 'no' literal survives in the composer" \
    'rescue-pipeline-verified: no -->' "$RESCUE_BODY_FN"
assert_contains "AC3/mika#2354: Path B measures the pipeline before opening the PR" \
    '_measure_pipeline_verified "$WORKTREE_DIR"' "$PATHB_BLOCK"
assert_contains "AC3/mika#2354: the measurement is behind the kill-switch" \
    'if _rescue_verify_enabled; then' "$PATHB_BLOCK"
assert_contains "AC3/mika#2354: the measured verdict is passed to the composer" \
    '"$_rescue_verified" "$_rescue_verify_term" "$_rescue_verify_excerpt"' "$PATHB_BLOCK"

# mika#2354 AC10: term 5 is invoked with `origin/main`, never the script's own
# `main` default. A dispatch worktree's local `main` can be days stale, so the
# docs/source bucket split would be computed on a diff that is not the one the
# PR publishes. Structural assertion — the wrong base ref produces a plausible
# verdict, not an error, so no behavioural test can see it.
MEASURE_FN=$(awk '/^_measure_pipeline_verified\(\) \{/,/^\}/' "$DISPATCH_LIB")
assert_contains "AC10: verify-pipeline.sh is invoked against origin/main" \
    './scripts/verify-pipeline.sh origin/main' "$MEASURE_FN"
assert_not_contains "AC10: never against the worktree's local main" \
    'verify-pipeline.sh main' "$MEASURE_FN"
assert_contains "AC3: Path B emits RECOVERY_PENDING: true (Guard 1)" \
    'RECOVERY_PENDING: true' "$PATHB_BLOCK"
assert_contains "AC3: Path B tags the rescued PR with the wip-rescue label" \
    'add-label "wip-rescue"' "$PATHB_BLOCK"
assert_contains "AC3: Path B emits the canonical PR: line (mika#1352)" \
    'PR: ${PR_URL}' "$PATHB_BLOCK"

# AC6 (Edit 2): the wip(mika#1383) marker commit makes the head-commit headline
# match Guard 2's ^wip\( regex — and must be guarded to commit-pushed-no-pr ONLY
# (the dirty-worktree class is already wip()-prefixed; a second empty commit there
# would be wrong).
assert_contains "AC6: Path B adds an empty wip(mika#1383) marker commit for Guard 2" \
    'commit --allow-empty --no-verify -m "wip(mika#1383): auto-PR-create rescue' "$PATHB_BLOCK"

# Structural: the marker commit must sit INSIDE a commit-pushed-no-pr class guard.
# Assert the guard opens before the marker commit and that the dirty-worktree
# arm does not contain the marker.
MARKER_GUARD_BLOCK=$(printf '%s\n' "$PATHB_BLOCK" | sed -n '/RECOVERY_CLASS" = "commit-pushed-no-pr"/,/RESCUED_PR_URL=\$(gh pr create/p')
assert_contains "AC6: marker commit is scoped under the commit-pushed-no-pr guard" \
    'wip(mika#1383): auto-PR-create rescue' "$MARKER_GUARD_BLOCK"
assert_contains "AC6: the marker-commit guard tests RECOVERY_CLASS = commit-pushed-no-pr" \
    'RECOVERY_CLASS" = "commit-pushed-no-pr"' "$MARKER_GUARD_BLOCK"

# AC5 regression: the dirty-worktree class still flows through Path B unchanged
# (its title/fact branch and the shared draft body remain).
assert_contains "AC5: dirty-worktree class still handled by Path B" \
    'RECOVERY_CLASS="dirty-worktree"' "$PATHB_BLOCK"

# AC6 structural: the marker commit appears EXACTLY ONCE in Path B (only in the
# commit-pushed-no-pr arm; the dirty-worktree class is already wip()-prefixed and
# must not get a second empty commit). A count > 1 means it leaked into both arms.
MARKER_COUNT=$(printf '%s\n' "$PATHB_BLOCK" | grep -c 'commit --allow-empty --no-verify -m "wip(mika#1383): auto-PR-create rescue')
if [ "$MARKER_COUNT" -eq 1 ]; then
    PASS=$((PASS + 1))
    echo "  ✓ AC6: wip(mika#1383) marker commit appears exactly once (commit-pushed-no-pr arm only)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ AC6: expected exactly 1 wip(mika#1383) marker commit in Path B, found $MARKER_COUNT"
fi

# mika#1679 hardening (review follow-up): the marker commit is idempotent on
# re-dispatch (skip when HEAD is already a wip(mika#1383) marker) and the push
# failure is surfaced as an observable signal rather than silently swallowed
# (so an unpushed marker = unarmed Guard 2 is visible to operator/telemetry).
assert_contains "Hardening: marker commit is idempotent (skip when HEAD already a wip(mika#1383) marker)" \
    "grep -qF 'wip(mika#1383): auto-PR-create rescue'" "$MARKER_GUARD_BLOCK"
assert_contains "Hardening: marker push failure is surfaced (rescue_marker_push.failed), not silenced" \
    'rescue_marker_push.failed' "$PATHB_BLOCK"
assert_not_contains "Hardening: marker push no longer uses a bare '|| true' silent swallow" \
    'push origin "$BRANCH" 2>&9 || true' "$MARKER_GUARD_BLOCK"

# --- Test: repo#number parse normalizes an optional owner/ prefix (mika#1593) ---
echo ""
echo "Test: _set_up_worktree prompt parse — owner-prefix normalization (mika#1593)"

# (a) Static: the live parser carries the broadened regex + owner-strip, so the
#     behavioral replica below cannot silently drift from the real code.
PARSE_REGION=$(sed -n '/--- Parse repo#number format ---/,/^    if \[ -n "\$REPO" \]/p' "$DISPATCH_LIB")
assert_contains "Parser regex accepts an optional owner/ segment" \
    '^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$' "$PARSE_REGION"
assert_contains "Parser strips the owner/ prefix to the bare basename" \
    "sed 's#.*/##'" "$PARSE_REGION"

# (b) Behavioral: replicate the exact two-line parse and assert normalization.
#     Mirrors the harness convention of testing extracted logic in isolation
#     (the full dispatch_claude_pilot needs git/gh/claude-pilot).
_parse_prompt() {
    local PROMPT="$1" REPO="" ISSUE_NUM=""
    if grep -qE -- '^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$' <<<"$PROMPT"; then
        REPO=$(printf '%s' "$PROMPT" | sed 's/#.*//' | sed 's#.*/##')
        ISSUE_NUM=$(printf '%s' "$PROMPT" | sed 's/.*#//')
    fi
    printf '%s|%s' "$REPO" "$ISSUE_NUM"
}
assert_eq "Bare repo#number parses unchanged" "mika|214" "$(_parse_prompt 'mika#214')"
assert_eq "Owner-qualified ref normalizes to bare basename" \
    "mika|1576" "$(_parse_prompt 'senara-solutions/mika#1576')"
assert_eq "Hyphenated repo basename is preserved" \
    "mika-cloud|50" "$(_parse_prompt 'senara-solutions/mika-cloud#50')"
assert_eq "Bare hyphenated repo parses unchanged" \
    "mika-skills|8" "$(_parse_prompt 'mika-skills#8')"
assert_eq "Free-text prompt with embedded # stays free-text (empty REPO)" \
    "|" "$(_parse_prompt 'fix the foo#bar thing and more')"

# --- Test 16: _find_issue_plan header-shape discovery (mika#1602, n=3; tier 3 added in mika#1617) ---
#
# Behavioral test: source dispatch-lib.sh (verified side-effect-free — function
# definitions only, no top-level execution) and call the real _find_issue_plan
# against temp `docs/plans/` fixtures. Proves AC1–AC4 for tier-2 shapes:
#   AC1 — `**Issue:** mika#N` (and `issue: mika#N`) headers are discoverable.
#   AC2 — the legacy `**Ticket:** mika#N` and `ticket: mika#N` shapes still match.
#   AC3 — the primary filename pass (`*-N-*-plan.md`) still matches.
#   AC4 — the `**Issue:**` case FAILS on the pre-fix regex and PASSES after.
# Tier-3 broad content scan tests live in tests/test_find_issue_plan.sh (mika#1617).
#
# Each fixture is padded > 500 bytes to satisfy the mika#1033 size filter.

echo ""
echo "Test 16: _find_issue_plan header-shape discovery (mika#1602)"
echo "------------------------------------------------------------"

# _fip_probe <issue_num> <header_line> <filename_slug> [header_offset_lines]
# Builds a one-off plan fixture and returns _find_issue_plan's verdict as
# "FOUND <basename>" (exit 0) or "NOTFOUND" (exit non-zero). Sourcing happens
# in this same subshell so $WORKTREE_DIR / $ISSUE_NUM scope to the call only.
_fip_probe() {
    local issue_num="$1" header_line="$2" slug="$3" offset="${4:-2}"
    local tmp result rc i
    tmp=$(mktemp -d)
    mkdir -p "$tmp/docs/plans"
    local plan="$tmp/docs/plans/$slug"
    {
        echo "# Plan: synthetic fixture"
        # Pad with $offset blank/filler lines BEFORE the header so callers can
        # push the header above or below the 20-line header-zone boundary.
        for ((i = 1; i < offset; i++)); do echo ""; done
        echo "$header_line"
        echo ""
        # >500 bytes of body so the size filter does not reject the fixture.
        for i in $(seq 1 12); do
            echo "Body line $i — padding padding padding padding padding padding."
        done
    } > "$plan"
    result=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        WORKTREE_DIR="$tmp" ISSUE_NUM="$issue_num" _find_issue_plan
    ) && rc=0 || rc=$?
    rm -rf "$tmp"
    if [ "$rc" -eq 0 ] && [ -n "$result" ]; then
        printf 'FOUND %s' "$(basename "$result")"
    else
        printf 'NOTFOUND'
    fi
}

# AC1 — the n=3 case: **Issue:** header, filename has NO -1602- token.
assert_eq "AC1: **Issue:** mika#1602 header found despite unrelated filename" \
    "FOUND 2026-06-27-006-fix-unrelated-slug-plan.md" \
    "$(_fip_probe 1602 '**Issue:** mika#1602' '2026-06-27-006-fix-unrelated-slug-plan.md')"

# AC1 variant — `issue:` YAML frontmatter shape.
assert_eq "AC1: issue: mika#1602 YAML header found" \
    "FOUND 2026-06-27-007-yaml-issue-shape-plan.md" \
    "$(_fip_probe 1602 'issue: mika#1602' '2026-06-27-007-yaml-issue-shape-plan.md')"

# AC2 regression — legacy **Ticket:** shape still matches.
assert_eq "AC2: **Ticket:** mika#771 header still found (no regression)" \
    "FOUND 2026-06-06-003-feat-some-other-slug-plan.md" \
    "$(_fip_probe 771 '**Ticket:** mika#771' '2026-06-06-003-feat-some-other-slug-plan.md')"

# AC2 regression — legacy ticket: YAML shape still matches.
assert_eq "AC2: ticket: mika#771 YAML header still found (no regression)" \
    "FOUND 2026-06-06-004-feat-yaml-ticket-plan.md" \
    "$(_fip_probe 771 'ticket: mika#771' '2026-06-06-004-feat-yaml-ticket-plan.md')"

# AC3 regression — primary filename pass (issue number in filename, no content header).
assert_eq "AC3: filename-embedded issue number found via primary pass" \
    "FOUND 2026-06-06-003-fix-1407-pilot-push-plan.md" \
    "$(_fip_probe 1407 '## No matching header here' '2026-06-06-003-fix-1407-pilot-push-plan.md')"

# Negative — wrong issue number must NOT match (guards against over-broad union).
assert_eq "Negative: **Issue:** mika#9999 not matched for ISSUE_NUM=1602" \
    "NOTFOUND" \
    "$(_fip_probe 1602 '**Issue:** mika#9999' '2026-06-27-008-wrong-number-plan.md')"

# mika#2038 — tier 1 refutes a candidate whose header names another issue.
# The founding incident, verbatim: the glob `*-2026-*-plan.md` matches the
# RustSec advisory id in this filename, and the old tier 1 returned it first,
# so a pilot dispatched for mika#2026 ran /ce-work on an April `rand` bump.
# The header says #539, so the candidate is refuted and no tier picks it up.
assert_eq "mika#2038: rustsec-2026-0097 filename not returned for ISSUE_NUM=2026" \
    "NOTFOUND" \
    "$(_fip_probe 2026 '**Issue:** #539' '2026-04-11-003-chore-deps-bump-rand-clear-rustsec-2026-0097-plan.md')"

# mika#2038 — a header-less, off-slot filename must STILL resolve at tier 1.
# 95 of 745 real plans carry no issue marker and 490 do not honour the
# `<date>-<NNN>-<type>-<issue>-` slot; refutation must not turn one false
# positive into that many false negatives (mika#1421 / #1602 / #1617 lineage).
assert_eq "mika#2038: header-less 'mika-' prefixed filename still found" \
    "FOUND 2026-06-10-001-fix-mika-1475-deploy-info-off-main-abort-plan.md" \
    "$(_fip_probe 1475 '## No matching header here' '2026-06-10-001-fix-mika-1475-deploy-info-off-main-abort-plan.md')"

# mika#2038 — a plan whose slug cites another ticket is refuted by its header.
assert_eq "mika#2038: plan for #1679 not returned for the #1383 it merely cites" \
    "NOTFOUND" \
    "$(_fip_probe 1383 'issue: 1679' '2026-06-30-011-fix-1679-dispatch-lib-mika-1383-recovery-guards-plan.md')"

# Negative — header below the 50-line zone must NOT match (tier-3 zone boundary).
# Pre-mika#1617 this was offset 30 (tier-2's 20-line zone); now tier 3 extends
# the scan to 50 lines, so the offset must be past 50 to remain a negative case.
assert_eq "Negative: **Issue:** mika#1602 on line 55 not matched (past tier-3 zone)" \
    "NOTFOUND" \
    "$(_fip_probe 1602 '**Issue:** mika#1602' '2026-06-27-009-deep-header-plan.md' 55)"

# --- Test 17: Post-flight recovery fires on all exit paths (mika#1615) ---

echo ""
echo "Test 17: Post-flight recovery fires on all exit paths (mika#1615)"
echo "-----------------------------------------------------------------"

# Structural: POST_RUN_HEAD computed unconditionally (before the three-branch if/elif/else)
# The computation must appear BEFORE the `if [ -n "$STATUS" ]` line that opens Branch A.
POST_RUN_HEAD_LINE=$(grep -n 'POST_RUN_HEAD=$(git -C "$WORKTREE_DIR" rev-parse HEAD' "$DISPATCH_LIB" \
    | grep -v '#' | grep -v 'rescue' | grep -v 'trailing' | head -1 | cut -d: -f1)
BRANCH_A_LINE=$(grep -n 'if \[ -n "$STATUS" \]; then' "$DISPATCH_LIB" | head -1 | cut -d: -f1)

if [ -n "$POST_RUN_HEAD_LINE" ] && [ -n "$BRANCH_A_LINE" ] && [ "$POST_RUN_HEAD_LINE" -lt "$BRANCH_A_LINE" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ POST_RUN_HEAD computed before Branch A (line $POST_RUN_HEAD_LINE < $BRANCH_A_LINE)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ POST_RUN_HEAD must be computed before Branch A (POST_RUN_HEAD=$POST_RUN_HEAD_LINE, Branch A=$BRANCH_A_LINE)"
fi

# Structural: _post_flight_recovery function exists
assert_contains "_post_flight_recovery function defined" \
    "_post_flight_recovery()" \
    "$(grep '_post_flight_recovery()' "$DISPATCH_LIB")"

# Structural: _post_flight_recovery called from _run_claude_pilot after three-branch fi
RUN_CLAUDE_PILOT_BODY=$(sed -n '/_run_claude_pilot()/,/^}/p' "$DISPATCH_LIB")
assert_contains "_post_flight_recovery called in _run_claude_pilot" \
    "_post_flight_recovery" \
    "$RUN_CLAUDE_PILOT_BODY"

# Structural: dirty-worktree rescue is inside _post_flight_recovery, NOT inside Branch A
BRANCH_A_BLOCK=$(sed -n '/if \[ -n "\$STATUS" \]; then/,/elif \[ "\$PILOT_EXIT" -eq 0 \]/p' "$DISPATCH_LIB")
assert_not_contains "Dirty-worktree rescue NOT inside Branch A" \
    "Unit 1 (mika#1282): detect dirty worktree" \
    "$BRANCH_A_BLOCK"

RECOVERY_FUNC=$(sed -n '/_post_flight_recovery()/,/^}/p' "$DISPATCH_LIB")
assert_contains "Dirty-worktree rescue inside _post_flight_recovery" \
    "Unit 1 (mika#1282): detect dirty worktree" \
    "$RECOVERY_FUNC"

# Structural: dev-groom plan validation is inside _post_flight_recovery
assert_contains "Dev-groom plan validation inside _post_flight_recovery" \
    "Post-flight plan validation" \
    "$RECOVERY_FUNC"
assert_not_contains "Dev-groom plan validation NOT inside Branch A" \
    "Post-flight plan validation" \
    "$BRANCH_A_BLOCK"

# Structural: PR-existence check is inside _post_flight_recovery
assert_contains "PR-existence check inside _post_flight_recovery" \
    "Discover actual PR URL" \
    "$RECOVERY_FUNC"

# Structural: outcome classification is inside _post_flight_recovery
assert_contains "Outcome classification inside _post_flight_recovery" \
    "outcome classification line" \
    "$RECOVERY_FUNC"

# Structural: POST_RUN_HEAD initialization (empty string) alongside RESCUED_DIRTY_WORKTREE
assert_contains "POST_RUN_HEAD initialized to empty" \
    'POST_RUN_HEAD=""' \
    "$RUN_CLAUDE_PILOT_BODY"

# Structural: the unconditional POST_RUN_HEAD computation is guarded by PRE_RUN_HEAD and WORKTREE_DIR
UNCONDITIONAL_HEAD_BLOCK=$(sed -n '/Compute POST_RUN_HEAD unconditionally/,/fi/p' "$DISPATCH_LIB" | head -10)
assert_contains "POST_RUN_HEAD guard checks PRE_RUN_HEAD" \
    'PRE_RUN_HEAD' \
    "$UNCONDITIONAL_HEAD_BLOCK"
assert_contains "POST_RUN_HEAD guard checks WORKTREE_DIR" \
    'WORKTREE_DIR' \
    "$UNCONDITIONAL_HEAD_BLOCK"

# Behavioral: verify recovery fires when STATUS is empty (Branch B scenario)
# Simulates a dirty-worktree rescue path with no structured JSON output.
test_recovery_fires_on_branch_b() {
    local base_dir wt_dir pre_head
    base_dir=$(mktemp -d)
    wt_dir="$base_dir/worktree"

    # Set up a git repo to simulate dirty worktree
    git init -q "$wt_dir" 2>/dev/null
    git -C "$wt_dir" config user.email "test@test.com"
    git -C "$wt_dir" config user.name "Test"
    echo "initial" > "$wt_dir/file.txt"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "initial" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)

    # Create dirty file (pilot-authored content)
    echo "pilot work" > "$wt_dir/new_feature.rs"

    # Source dispatch-lib and set up variables as _run_claude_pilot would
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB"

        # Simulate Branch B state: STATUS is empty, exit 0, dirty worktree
        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD="$pre_head"  # Same as PRE — no commits
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-pilot"
        REPO="mika"
        BRANCH="test-branch"
        ISSUE_NUM="9999"
        SESSION_ID="test-session"
        LOG_ID="test-log"
        STATUS=""  # Branch B — no structured JSON output
        RESULT="claude-pilot completed (exit 0) but output was not structured JSON."
        RESCUED_DIRTY_WORKTREE=0

        # Redirect fd 9 to stderr for the rescue block
        exec 9>&2

        # Run the recovery function
        _post_flight_recovery 2>/dev/null

        # Check results
        post_head=$(git -C "$wt_dir" rev-parse HEAD)
        if [ "$pre_head" != "$post_head" ] && [ "$RESCUED_DIRTY_WORKTREE" -eq 1 ]; then
            echo "OK"
        else
            echo "FAIL: pre=$pre_head post=$post_head rescued=$RESCUED_DIRTY_WORKTREE"
        fi
    )

    rm -rf "$base_dir"
}

RESULT_17A=$(test_recovery_fires_on_branch_b 2>/dev/null)
assert_eq "Branch B (non-JSON exit 0): dirty-worktree rescue fires and commits" "OK" "$RESULT_17A"

# Behavioral: verify recovery fires when exit code is non-zero (Branch C scenario)
test_recovery_fires_on_branch_c() {
    local base_dir wt_dir pre_head
    base_dir=$(mktemp -d)
    wt_dir="$base_dir/worktree"

    git init -q "$wt_dir" 2>/dev/null
    git -C "$wt_dir" config user.email "test@test.com"
    git -C "$wt_dir" config user.name "Test"
    echo "initial" > "$wt_dir/file.txt"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "initial" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)

    # Create dirty file
    echo "pilot crash content" > "$wt_dir/partial_impl.rs"

    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB"

        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD="$pre_head"
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-pilot"
        REPO="mika"
        BRANCH="test-branch"
        ISSUE_NUM="9999"
        SESSION_ID="test-session"
        LOG_ID="test-log"
        STATUS=""  # Branch C — non-zero exit, no structured output
        RESULT="claude-pilot FAILED (exit code 1)."
        RESCUED_DIRTY_WORKTREE=0

        exec 9>&2

        _post_flight_recovery 2>/dev/null

        post_head=$(git -C "$wt_dir" rev-parse HEAD)
        if [ "$pre_head" != "$post_head" ] && [ "$RESCUED_DIRTY_WORKTREE" -eq 1 ]; then
            echo "OK"
        else
            echo "FAIL: pre=$pre_head post=$post_head rescued=$RESCUED_DIRTY_WORKTREE"
        fi
    )

    rm -rf "$base_dir"
}

RESULT_17B=$(test_recovery_fires_on_branch_c 2>/dev/null)
assert_eq "Branch C (non-zero exit): dirty-worktree rescue fires and commits" "OK" "$RESULT_17B"

# Behavioral: verify POST_RUN_HEAD is available for outcome classification
# when STATUS is empty (previously Outcome was never emitted for Branch B/C)
test_outcome_emitted_on_branch_b() {
    local base_dir wt_dir pre_head
    base_dir=$(mktemp -d)
    wt_dir="$base_dir/worktree"

    git init -q "$wt_dir" 2>/dev/null
    git -C "$wt_dir" config user.email "test@test.com"
    git -C "$wt_dir" config user.name "Test"
    echo "initial" > "$wt_dir/file.txt"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "initial" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)

    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB"

        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD="$pre_head"
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-pilot"
        REPO="mika"
        BRANCH="test-branch"
        ISSUE_NUM="9999"
        SESSION_ID="test-session"
        LOG_ID="test-log"
        STATUS=""
        RESULT="claude-pilot completed (exit 0) but output was not structured JSON."
        RESCUED_DIRTY_WORKTREE=0

        exec 9>&2

        _post_flight_recovery 2>/dev/null

        # Outcome classification should have run (PIPELINE_INCOMPLETE for zero-commit)
        if grep -qF -- "Outcome:" <<<"$RESULT"; then
            echo "OK"
        else
            echo "FAIL: no Outcome line in RESULT"
        fi
    )

    rm -rf "$base_dir"
}

RESULT_17C=$(test_outcome_emitted_on_branch_b 2>/dev/null)
assert_eq "Branch B: outcome classification fires (Outcome: line present)" "OK" "$RESULT_17C"

# --- Test: wip-rescue label application (mika#1631) ---

echo ""
echo "Test: wip-rescue label applied to rescued PRs (mika#1631)"
echo "----------------------------------------------------------"

# Verify the rescue flow applies the wip-rescue label after PR creation.
# The `gh pr edit ... --add-label "wip-rescue"` call is inside the deeply-nested
# `if [ -n "$RESCUED_PR_URL" ]` block of _post_flight_recovery. We grep the
# file directly rather than extracting by sed range (nested braces defeat the
# simple /pattern/,/^}/p extraction; full-file cat hits shell string limits).

if grep -qF -- '--add-label "wip-rescue"' "$DISPATCH_LIB"; then
    PASS=$((PASS + 1)); echo "  ✓ Rescue flow applies wip-rescue label"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Rescue flow applies wip-rescue label"
fi

# Verify the label application targets RESCUED_PR_URL
if grep -qF 'gh pr edit "$RESCUED_PR_URL" --add-label "wip-rescue"' "$DISPATCH_LIB"; then
    PASS=$((PASS + 1)); echo "  ✓ Label applied to RESCUED_PR_URL"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Label applied to RESCUED_PR_URL"
fi

# Verify it's fault-tolerant (|| true) — label failure must not break the rescue
if grep -qF 'wip-rescue" 2>&9 || true' "$DISPATCH_LIB"; then
    PASS=$((PASS + 1)); echo "  ✓ Label application is fault-tolerant (|| true)"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Label application is fault-tolerant (|| true)"
fi

# --- Regression: no bare $REPO in gh --repo arguments (mika#1643) ---

echo ""
echo "=== Regression: no bare \$REPO in gh --repo (mika#1643) ==="

# All gh --repo call sites must use senara-solutions/$REPO, never bare $REPO.
BARE_REPO_HITS=$(grep -n 'gh.*--repo[[:space:]]*"\$REPO"' "$DISPATCH_LIB" | grep -v 'senara-solutions/' || true)
BARE_REPO_HITS_UNQUOTED=$(grep -n 'gh.*--repo[[:space:]]*\${REPO}' "$DISPATCH_LIB" | grep -v 'senara-solutions/' || true)

if [ -z "$BARE_REPO_HITS" ] && [ -z "$BARE_REPO_HITS_UNQUOTED" ]; then
    PASS=$((PASS + 1)); echo "  ✓ No bare \$REPO in gh --repo arguments"
else
    FAIL=$((FAIL + 1)); echo "  ✗ Found bare \$REPO in gh --repo arguments:"
    [ -n "$BARE_REPO_HITS" ] && echo "$BARE_REPO_HITS"
    [ -n "$BARE_REPO_HITS_UNQUOTED" ] && echo "$BARE_REPO_HITS_UNQUOTED"
fi

# --- Redundant-groom refusal gate: functional cases (mika#2012) ---
# Defined above next to the gate's code-shape assertions; invoked here because
# they depend on _fixture_setup.

echo ""
echo "Test: redundant-groom refusal gate (mika#2012)"
echo "-----------------------------------------------"

# mika#2608 : ces douze cas appellent `_committed_plan_on_branch`, qui délègue
# désormais à `mika plan-callout`. Sans le binaire ils ne mesurent rien, et le
# dire est le contrat de la troisième colonne (mika#2149) — un cas de porte qui
# n'a pas pu s'armer ne doit pas se lire comme un vert.
if ! mika_bin_or_skip "porte de grooming mika#2012 (12 cas)"; then
    : # le SKIP est déjà compté et dit par le helper
else
for gate_case in \
    "test_groom_gate_callout_present_file_absent|callout present but plan file ABSENT → gate does NOT fire" \
    "test_groom_gate_plan_committed|plan committed on branch → gate fires with the path" \
    "test_groom_gate_repo_prefixed_path|repo-prefixed callout resolves to relative path" \
    "test_groom_gate_no_callout|no Plan callout → gate does NOT fire" \
    "test_groom_gate_branch_absent|branch absent from remote → gate does NOT fire" \
    "test_groom_gate_foreign_plan_inherited_from_main|mika#2034: plan claims another issue, inherited from main → gate does NOT fire" \
    "test_groom_gate_plan_header_claims_target_issue|mika#2034: plan header claims the target issue → gate fires" \
    "test_groom_gate_plan_without_issue_marker_still_fires|mika#2034: plan with no issue marker → gate still fires" \
    "test_groom_gate_issue_arg_is_optional|mika#2034: the 5th argument is optional" \
    "test_plan_provenance_distinguishes_inherited_from_committed|mika#2034: provenance separates inherited from committed" \
    "test_groom_gate_declines_on_directory_candidate|mika#2034: callout naming a directory → gate does NOT fire" \
    "test_groom_gate_refusal_implies_rust_says_groomed|mika#2158 AC7: la garde refuse ⇒ le Rust dit groomé (six corps figés)"
do
    gate_fn="${gate_case%%|*}"
    gate_label="${gate_case#*|}"
    gate_result=$("$gate_fn" 2>/dev/null) || gate_result="ABORTED (rc=$?)"
    if [ "$gate_result" = "PASS" ]; then
        PASS=$((PASS + 1)); echo "  ✓ $gate_label"
    else
        FAIL=$((FAIL + 1)); echo "  ✗ $gate_label: $gate_result"
    fi
done
fi

# ===========================================================================
# mika#1772 — an honest dev-groom callback
#
# Founding incident: tasks f4fff3ff-4e57-4005-b4b6-bda13d68872d (2026-08-28
# 18:08Z) and 74504478-184d-4af4-8d1f-cadb1b1fdce9 (19:08Z), both dispatched
# on mika#2013. claude-pilot returned `status: terminated` at Turns 2 after
# `[guardrail] idle_timeout: No meaningful progress for 300s`, having made
# zero write tool calls. dispatch-lib ran the whole content-validation chain
# on that empty session and emitted a callback with THREE false statements
# ahead of the one true one:
#   1. "Plan exists on branch but architect verdict is missing" — no plan
#      existed and the architect was never reached.
#   2. "no /ce:plan invocation detected in session log" — the session log
#      file did not exist, so nothing was detected either way.
#   3. "plan already committed from prior run" — the guard matched ANY
#      *-plan.md in docs/plans/, of which main carries 769.
# These assertions lock each one shut.
# ===========================================================================

echo ""
echo "Test: dev-groom callback honesty (mika#1772)"
echo "---------------------------------------------"

ITERATE_SRC_1772=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    declare -f _iterate_groom_loop
)
DISPATCH_SRC_1772=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    declare -f dispatch_claude_pilot
)
RCP_SRC_1772=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    declare -f _run_claude_pilot
)
PFR_SRC_1772=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    declare -f _post_flight_recovery
)

# --- U1: the loop names its own failure reason -----------------------------

assert_contains "U1: _iterate_groom_loop initializes GROOM_LOOP_FAILURE_REASON" \
    'GROOM_LOOP_FAILURE_REASON=' "$ITERATE_SRC_1772"

# KTD1: the reason must reach the caller, so the variable is global. A `local`
# declaration would silently restore the "no reason recorded" fallback on
# every failure.
assert_not_contains "U1: GROOM_LOOP_FAILURE_REASON is not declared local" \
    'local GROOM_LOOP_FAILURE_REASON' "$ITERATE_SRC_1772"

# The invariant that every `return 1` carries a reason is asserted per site,
# further down. A totals comparison was tried first and proved complaisant:
# with one reason setter deleted it still reported 21 >= 18 and passed.

# The three _escalate_groom exits are the cases where the architect genuinely
# refused the plan — the class R2 must keep distinguishable from a guard trip.
assert_contains "U1: first-pass ESCALATE exit records an architect-refusal reason" \
    'architect ESCALATE (first-pass)' "$ITERATE_SRC_1772"

# --- U1: the caller stops inventing a diagnosis ----------------------------

assert_not_contains "U1: dispatch_claude_pilot no longer asserts a plan on the branch" \
    'Plan exists on branch' "$DISPATCH_SRC_1772"
assert_not_contains "U1: dispatch_claude_pilot no longer hardcodes the convergence phrase" \
    'architect convergence did not complete' "$DISPATCH_SRC_1772"
assert_contains "U1: dispatch_claude_pilot emits the recorded reason" \
    'GROOM_LOOP_FAILURE_REASON' "$DISPATCH_SRC_1772"
assert_contains "U1: dispatch_claude_pilot has a fallback when no reason was recorded" \
    'no reason recorded' "$DISPATCH_SRC_1772"

# R2 wants the plan-on-branch claim MEASURED, not deleted: on the 2026-07-04
# class (mika#1723) a plan really was on the branch and that fact is the most
# useful line in the message. _committed_plan_on_branch is the file's existing
# authority for the question.
assert_contains "U1: plan-on-branch is measured via _committed_plan_on_branch" \
    '_committed_plan_on_branch' "$DISPATCH_SRC_1772"

# Behavioral: a guard trip records a reason naming the guard, not convergence.
_iterate_reason_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="" ISSUE_NUM="1267" REPO="mika"
        _iterate_groom_loop >/dev/null 2>&1
        printf '%s' "$GROOM_LOOP_FAILURE_REASON"
    )
}
ITERATE_REASON_1772=$(_iterate_reason_probe) || ITERATE_REASON_1772=""
assert_contains "U1: WORKTREE_DIR guard trip records a guard reason" \
    "WORKTREE_DIR" "$ITERATE_REASON_1772"
assert_not_contains "U1: guard trip does not blame architect convergence" \
    "architect convergence" "$ITERATE_REASON_1772"

# Behavioral: no issue-scoped plan names the plan lookup, not the architect.
_iterate_noplan_probe() {
    local tmp
    tmp=$(mktemp -d)
    mkdir -p "$tmp/docs/plans"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$tmp" ISSUE_NUM="1267" REPO="mika"
        _iterate_groom_loop >/dev/null 2>&1
        printf '%s' "$GROOM_LOOP_FAILURE_REASON"
    )
    rm -rf "$tmp"
}
ITERATE_NOPLAN_1772=$(_iterate_noplan_probe) || ITERATE_NOPLAN_1772=""
assert_contains "U1: missing issue plan names the plan lookup" \
    "plan" "$ITERATE_NOPLAN_1772"
assert_not_contains "U1: missing issue plan does not blame architect convergence" \
    "architect convergence" "$ITERATE_NOPLAN_1772"

# --- U2: a terminated session is classified before content validation ------

# KTD5: dispatch_claude_pilot and _run_claude_pilot need a real pilot and CLI,
# so the classification lives in its own callable function the harness can run
# with an injected environment — the same shape as _find_issue_plan's probes.
# mika#2149: a sixth argument carries `.api_error_status` (cpp#54), and a
# seventh selects which stream the probe returns — `stdout` (default) or
# `stderr`, because the halt_family.unknown line (R-4) lives on stderr and the
# 2>/dev/null here used to make that stream unobservable.
_classify_probe() {
    local guardrail_line="$1" turns="${2:-2}" subtype="${3:-}" reason="${4:-}" mode="${5:-full}"
    local api_error_status="${6:-}" stream="${7:-stdout}"
    local tmp log_id
    tmp=$(mktemp -d)
    log_id="probe-1772"
    if [ -n "$guardrail_line" ]; then
        printf '%s\n' "$guardrail_line" > "$tmp/${log_id}.stderr"
    fi
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        STATUS="terminated"
        TURNS="$turns"
        DURATION="602410"
        SESSION_ID="0dab1700-5ca9-4145-87b0-6f618a047220"
        LOG_ID="$log_id"
        PILOT_LOG_DIR="$tmp"
        STDERR_FILE=""
        SUBTYPE="$subtype"
        TERMINATION_REASON="$reason"
        API_ERROR_STATUS="$api_error_status"
        if [ "$stream" = "stderr" ]; then
            _classify_terminated_session "$mode" 2>&1 >/dev/null
        else
            _classify_terminated_session "$mode" 2>/dev/null
        fi
    )
    rm -rf "$tmp"
}

CLASSIFY_WITH_GUARDRAIL=$(_classify_probe '[guardrail] idle_timeout: No meaningful progress for 300s') || CLASSIFY_WITH_GUARDRAIL=""
assert_contains "U2: terminated classification names the termination" \
    "terminated" "$CLASSIFY_WITH_GUARDRAIL"
assert_contains "U2: terminated classification names the guardrail" \
    "idle_timeout" "$CLASSIFY_WITH_GUARDRAIL"
assert_contains "U2: terminated classification names the turn count" \
    "Turns: 2" "$CLASSIFY_WITH_GUARDRAIL"
assert_not_contains "U2: terminated classification carries no plan-lookup diagnosis" \
    "_find_issue_plan" "$CLASSIFY_WITH_GUARDRAIL"
assert_not_contains "U2: terminated classification carries no convergence diagnosis" \
    "architect convergence" "$CLASSIFY_WITH_GUARDRAIL"
assert_contains "U2: terminated classification points at the upstream stall lineage" \
    "mika#1901" "$CLASSIFY_WITH_GUARDRAIL"

# R7: one Outcome line. The old path could stack two — _post_flight_recovery
# appended one and the iterate-loop else-branch rewrote another.
CLASSIFY_OUTCOME_COUNT=$(printf '%s\n' "$CLASSIFY_WITH_GUARDRAIL" | grep -c '^Outcome:' || true)
assert_eq "U2: terminated classification carries exactly one Outcome line" \
    "1" "$CLASSIFY_OUTCOME_COUNT"
assert_contains "U2: terminated classification is PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE" "$CLASSIFY_WITH_GUARDRAIL"

# KTD3: stderr only enriches. A missing file degrades the text, never the
# classification — the 2026-08-28 tasks had no .log file at all, and a
# fail-closed read there would have hidden the whole class.
CLASSIFY_NO_STDERR=$(_classify_probe '') || CLASSIFY_NO_STDERR=""
assert_contains "U2: classification still fires when stderr is absent" \
    "terminated" "$CLASSIFY_NO_STDERR"
assert_contains "U2: classification without stderr is still PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE" "$CLASSIFY_NO_STDERR"

# Structural: the guard has to sit INSIDE _run_claude_pilot, because
# _post_flight_recovery is called from there (dispatch-lib.sh:1280) and not
# from dispatch_claude_pilot. A guard placed after _run_claude_pilot returns
# can only prefix text that is already false.
RCP_FIRST_1772=$(printf '%s\n' "$RCP_SRC_1772" \
    | grep -nE '_classify_terminated_session|_post_flight_recovery' | head -1)
assert_contains "U2: terminated guard precedes _post_flight_recovery in _run_claude_pilot" \
    "_classify_terminated_session" "$RCP_FIRST_1772"

# The mika#1615 invariant stays: the recovery call remains in _run_claude_pilot,
# it only becomes conditional.
assert_contains "U2: _post_flight_recovery is still called from _run_claude_pilot" \
    "_post_flight_recovery" "$RCP_SRC_1772"

# Structural: the terminated short-circuit precedes convergence.
DISPATCH_FIRST_1772=$(printf '%s\n' "$DISPATCH_SRC_1772" \
    | grep -nE 'PILOT_SESSION_TERMINATED|_iterate_groom_loop' | head -1)
assert_contains "U2: PILOT_SESSION_TERMINATED is tested before the iterate loop" \
    "PILOT_SESSION_TERMINATED" "$DISPATCH_FIRST_1772"

# The pilot push guard must NOT be gated on termination: it compares the remote
# ref before and after and already returns 0 when nothing moved, so gating it
# bought a no-op and blinded the one check that catches a terminated pilot which
# had already pushed.
assert_not_contains "U2: the pilot push guard is not gated on PILOT_SESSION_TERMINATED" \
    'PILOT_SESSION_TERMINATED:-0}" != "1" ] && ! _check_pilot_force_push' "$DISPATCH_SRC_1772"

# R6: skip the push only when the branch carries nothing. _push_branch pushes
# "any local-ahead commits regardless of pilot exit code" (dispatch-lib.sh:1801)
# and the worktree is force-removed on the next dispatch, so a blanket skip
# would destroy the work of a session killed AFTER committing — the late-hang
# shape mika#1901 describes.
assert_contains "U2: the push skip is conditioned on an empty branch, not on termination alone" \
    'PRE_RUN_HEAD' "$DISPATCH_SRC_1772"

# --- U3: the remaining false statements ------------------------------------

# (a) "unknown" means the log could not be read. Claiming a detection was made
# is a different statement from making one and coming up empty.
assert_contains "U3a: an unreadable session log gets its own message" \
    'session log was not readable' "$PFR_SRC_1772"

# (b) the re-dispatch note must key on THIS issue's plan. The old glob matched
# any *-plan.md, and main carries 769 of them, so the note always fired.
assert_not_contains "U3b: the re-dispatch note no longer globs for any plan file" \
    'find "$WORKTREE_DIR/docs/plans" -name "*-plan.md"' \
    "$PFR_SRC_1772"

# (c) the zero-commit message must not assert an exit code it never read.
# Both 2026-08-28 tasks carried PILOT_EXIT=1.
assert_not_contains "U3c: the zero-commit message no longer hardcodes 'exited 0'" \
    'claude-pilot exited 0 but HEAD unchanged' "$PFR_SRC_1772"
assert_contains "U3c: the zero-commit message reads the real exit code" \
    'PILOT_EXIT' "$PFR_SRC_1772"

# Behavioral fixture — the 2026-08-28 class end to end: a worktree whose
# docs/plans/ holds other tickets' plans and none for this issue, HEAD
# unchanged, PILOT_EXIT=1.
_pfr_probe_1772() {
    local issue_num="$1" plant_own_plan="$2" pilot_exit="$3"
    local base wt head
    base=$(mktemp -d)
    wt="$base/worktree"
    git init -q "$wt" 2>/dev/null
    git -C "$wt" config user.email "test@test.com"
    git -C "$wt" config user.name "Test"
    mkdir -p "$wt/docs/plans"
    # Other tickets' plans — the 769-file condition on main, in miniature.
    for other in 1111 2222; do
        {
            echo "# Plan for another ticket"
            echo "**Ticket:** mika issue#${other}"
            for i in $(seq 1 12); do
                echo "Body line $i — padding padding padding padding padding."
            done
        } > "$wt/docs/plans/2026-08-01-001-fix-${other}-other-plan.md"
    done
    if [ "$plant_own_plan" = "yes" ]; then
        {
            echo "# Plan for this ticket"
            echo "**Ticket:** mika issue#${issue_num}"
            for i in $(seq 1 12); do
                echo "Body line $i — padding padding padding padding padding."
            done
        } > "$wt/docs/plans/2026-08-29-001-fix-${issue_num}-own-plan.md"
    fi
    echo "initial" > "$wt/file.txt"
    git -C "$wt" add -A 2>/dev/null
    git -C "$wt" commit -q -m "initial" 2>/dev/null
    head=$(git -C "$wt" rev-parse HEAD)
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        PRE_RUN_HEAD="$head"
        POST_RUN_HEAD="$head"
        WORKTREE_DIR="$wt"
        SKILL="dev-groom"
        REPO="mika"
        BRANCH="test-branch"
        ISSUE_NUM="$issue_num"
        SESSION_ID="s"
        LOG_ID="probe-1772-absent-log"
        PILOT_EXIT="$pilot_exit"
        STATUS="terminated"
        RESULT="claude-pilot completed (status: terminated)."
        RESCUED_DIRTY_WORKTREE=0
        exec 9>&2
        _post_flight_recovery 2>/dev/null
        printf '%s' "$RESULT"
    )
    rm -rf "$base"
}

PFR_NO_OWN_PLAN=$(_pfr_probe_1772 2013 no 1) || PFR_NO_OWN_PLAN=""
# Positive anchor first: without it the three assert_not_contains below would all
# pass on an empty string if the probe ever failed to run.
assert_contains "U3: the probe produced a real callback to assert against" \
    "PIPELINE FAILURE" "$PFR_NO_OWN_PLAN"
assert_not_contains "U3b: no plan for this issue → no re-dispatch note" \
    "HEAD unchanged on dev-groom re-dispatch" "$PFR_NO_OWN_PLAN"
assert_not_contains "U3c: PILOT_EXIT=1 → the message does not claim 'exited 0'" \
    "exited 0" "$PFR_NO_OWN_PLAN"
assert_not_contains "U3a: an absent session log → no claim that /ce:plan was undetected" \
    "no /ce:plan invocation detected in session log" "$PFR_NO_OWN_PLAN"

PFR_OWN_PLAN=$(_pfr_probe_1772 2013 yes 1) || PFR_OWN_PLAN=""
assert_contains "U3b: a plan for this issue → the re-dispatch note fires" \
    "HEAD unchanged on dev-groom re-dispatch" "$PFR_OWN_PLAN"
assert_contains "U3b: the re-dispatch note names the plan it found" \
    "2026-08-29-001-fix-2013-own-plan.md" "$PFR_OWN_PLAN"

# --- mika#1772 review round: the two populations of `terminated` -----------
#
# `status: terminated` is set both by a guardrail abort and by an SDK limit.
# The first usually kills a session that did nothing; the second often kills
# one that did a great deal. Treating them alike would skip the mika#1282
# dirty-worktree rescue for the second and tell the operator "nothing was
# written" about a branch carrying commits — the exact defect class this ticket
# closes, reintroduced by its own fix.
#
# The subtype vocabulary is deliberately NOT listed here (mika#2149): upstream
# it is `GuardrailAbortReason.guardrail` in claude-pilot's types.py plus
# `SDK_TERMINATION_SUBTYPES` in agent.py; downstream it is `_halt_family` in
# dispatch-lib.sh. The drift guard further down reads the former and checks the
# latter — a comment nobody executes went stale by five values in eighteen days.

echo ""
echo "Test: terminated sessions that left work behind (mika#1772 review)"
echo "-------------------------------------------------------------------"

# _pilot_left_no_work is the measurement the split turns on.
_left_no_work_probe() {
    local dirty="$1" moved="$2"
    local base wt pre post
    base=$(mktemp -d); wt="$base/wt"
    git init -q "$wt" 2>/dev/null
    echo initial > "$wt/f"; git -C "$wt" add -A 2>/dev/null
    git -C "$wt" commit -q -m initial --no-verify 2>/dev/null
    pre=$(git -C "$wt" rev-parse HEAD)
    post="$pre"
    if [ "$moved" = "yes" ]; then
        echo more > "$wt/g"; git -C "$wt" add -A 2>/dev/null
        git -C "$wt" commit -q -m second --no-verify 2>/dev/null
        post=$(git -C "$wt" rev-parse HEAD)
    fi
    [ "$dirty" = "yes" ] && echo uncommitted > "$wt/pilot-wrote-this.rs"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$wt" PRE_RUN_HEAD="$pre" POST_RUN_HEAD="$post"
        if _pilot_left_no_work; then echo "NO_WORK"; else echo "HAS_WORK"; fi
    )
    rm -rf "$base"
}

assert_eq "clean tree, HEAD unmoved -> no work (the 2026-08-28 shape)" \
    "NO_WORK" "$(_left_no_work_probe no no)"
# The mika#1282 rescue exists for exactly this state; skipping it would lose the
# files, and the worktree is force-removed on the next dispatch.
assert_eq "dirty tree, HEAD unmoved -> work present" \
    "HAS_WORK" "$(_left_no_work_probe yes no)"
assert_eq "HEAD moved -> work present (the mika#1901 late-hang shape)" \
    "HAS_WORK" "$(_left_no_work_probe no yes)"
assert_eq "dirty AND moved -> work present" \
    "HAS_WORK" "$(_left_no_work_probe yes yes)"

# Banner mode: says only what was measured, and carries NO Outcome line — the
# recovery chain ran and owns that.
CLASSIFY_BANNER=$(_classify_probe '' 12 'error_max_turns' 'SDK limit reached: error_max_turns' 'banner') \
    || CLASSIFY_BANNER=""
assert_contains "banner names the SDK limit, not a guardrail" \
    "error_max_turns" "$CLASSIFY_BANNER"
assert_contains "banner names the commit range it measured" \
    "Commits:" "$CLASSIFY_BANNER"
assert_not_contains "banner never claims nothing was written" \
    "nothing was written to the branch" "$CLASSIFY_BANNER"
BANNER_OUTCOME_COUNT=$(printf '%s\n' "$CLASSIFY_BANNER" | grep -c '^Outcome:' || true)
assert_eq "banner carries no Outcome line (recovery owns it)" "0" "$BANNER_OUTCOME_COUNT"

# Full mode still earns its absolute claim, because the caller only reaches it
# when _pilot_left_no_work said so.
CLASSIFY_SDK_FULL=$(_classify_probe '' 2 'idle_timeout' 'No meaningful progress for 300s' 'full') \
    || CLASSIFY_SDK_FULL=""
assert_contains "full mode prefers the structured subtype over the stderr scrape" \
    "idle_timeout" "$CLASSIFY_SDK_FULL"
assert_contains "full mode states the measurement behind its claim" \
    "HEAD did not move and the worktree is clean" "$CLASSIFY_SDK_FULL"

# No subtype and no stderr: the message must say the cause is unrecorded rather
# than name one it did not read.
CLASSIFY_NO_CAUSE=$(_classify_probe '' 2 '' '' 'full') || CLASSIFY_NO_CAUSE=""
assert_contains "an unrecorded cause is reported as unrecorded" \
    "cause not recorded" "$CLASSIFY_NO_CAUSE"

# --- mika#2149: every halt motif gets a family and a retry hint downstream ---
#
# The subtype vocabulary is owned upstream by `GuardrailAbortReason.guardrail`
# (claude-pilot/src/claude_pilot/types.py) plus `SDK_TERMINATION_SUBTYPES` and
# the cpp#187 transport halt in agent.py. Downstream, `_halt_family` in
# dispatch-lib.sh is the ONLY place that enumerates it — a `case` with a `*)`
# arm, so a value added upstream lands on stderr as `halt_family.unknown` the
# first time it is seen instead of silently joining the prose. This comment
# deliberately lists no value: the two tables are the source of truth, and T6
# below checks them against each other.

echo ""
echo "Test: halt motifs are classified downstream (mika#2149)"
echo "--------------------------------------------------------"

# T1 — table-driven: one row per known subtype, both lines present.
while IFS='|' read -r subtype family hint; do
    T1_OUT=$(_classify_probe '' 2 "$subtype" 'x' 'full') || T1_OUT=""
    assert_contains "T1: $subtype -> Halt class: $family" \
        "Halt class: $family" "$T1_OUT"
    assert_contains "T1: $subtype -> Retry hint: $hint" \
        "Retry hint: $hint" "$T1_OUT"
done <<'T1_TABLE'
rate_limited|quota_throttled|transient
awaiting_model|model_never_resumed|transient
awaiting_tool|tool_never_returned|investigate
idle_timeout|session_silent|investigate
stream_stalled|model_stalled|transient
stall_detected|model_unproductive|investigate
empty_response|model_unproductive|investigate
watchdog_error|pilot_bug|investigate
prompt_cache_dead|substrate|investigate
error_max_turns|budget_exhausted|deterministic
error_max_budget_usd|budget_exhausted|deterministic
transport_message_too_large|transport|investigate
T1_TABLE

# T2 — a subtype outside the table is classed unknown/investigate AND said on
# stderr; a known one is not (negative control, same call shape).
T2_OUT=$(_classify_probe '' 2 'foo_bar' 'x' 'full') || T2_OUT=""
assert_contains "T2: unknown subtype -> Halt class: unknown" \
    "Halt class: unknown" "$T2_OUT"
assert_contains "T2: unknown subtype -> Retry hint: investigate" \
    "Retry hint: investigate" "$T2_OUT"
T2_ERR=$(_classify_probe '' 2 'foo_bar' 'x' 'full' '' stderr) || T2_ERR=""
assert_contains "T2: unknown subtype is named on stderr" \
    "halt_family.unknown subtype=foo_bar" "$T2_ERR"
T2_ERR_KNOWN=$(_classify_probe '' 2 'idle_timeout' 'x' 'full' '' stderr) || T2_ERR_KNOWN=""
assert_not_contains "T2 (negative control): a known subtype is silent on stderr" \
    "halt_family.unknown" "$T2_ERR_KNOWN"

# T3 — api_error_status (cpp#54) is a qualifier on the Halt: line, present
# only when the JSON carried it.
T3_OUT=$(_classify_probe '' 2 'rate_limited' 'backoff exhausted' 'full' '429') || T3_OUT=""
assert_contains "T3: api_error_status renders as (HTTP n) on the Halt: line" \
    "Halt: rate_limited (HTTP 429)" "$T3_OUT"
T3_NONE=$(_classify_probe '' 2 'rate_limited' 'backoff exhausted' 'full' '') || T3_NONE=""
assert_not_contains "T3 (negative control): no status -> no (HTTP" \
    "(HTTP" "$T3_NONE"
assert_contains "T3 (negative control): the Halt: line is otherwise intact" \
    "Halt: rate_limited — backoff exhausted" "$T3_NONE"

# T4 — fallback path: no subtype on the JSON, the `[guardrail]` line in stderr
# (ANSI-coloured, as ui.py:113 writes it) feeds the same table.
T4_LINE=$(printf '\033[38;5;208m[guardrail]\033[0m \033[1mawaiting_model\033[0m: model-wait ceiling 900s exceeded')
T4_OUT=$(_classify_probe "$T4_LINE" 2 '' '' 'full') || T4_OUT=""
assert_contains "T4: a stderr-only halt is classified like a JSON one" \
    "Halt class: model_never_resumed" "$T4_OUT"
assert_contains "T4: the Halt: line keeps the scraped text" \
    "Halt: [guardrail] awaiting_model:" "$T4_OUT"
T4_NONE=$(_classify_probe 'no guardrail line here' 2 '' '' 'full') || T4_NONE=""
assert_contains "T4 (negative control): no [guardrail] line -> unknown" \
    "Halt class: unknown" "$T4_NONE"
assert_contains "T4 (negative control): 'cause not recorded' is preserved" \
    "cause not recorded" "$T4_NONE"
T4_NONE_ERR=$(_classify_probe 'no guardrail line here' 2 '' '' 'full' '' stderr) || T4_NONE_ERR=""
assert_not_contains "T4 (negative control): an unrecorded cause is not reported as upstream drift" \
    "halt_family.unknown" "$T4_NONE_ERR"

# T5 — banner mode carries both lines too.
T5_OUT=$(_classify_probe '' 12 'awaiting_tool' 'tool-wait ceiling exceeded' 'banner') || T5_OUT=""
assert_contains "T5: banner mode carries Halt class:" \
    "Halt class: tool_never_returned" "$T5_OUT"
assert_contains "T5: banner mode carries Retry hint:" \
    "Retry hint: investigate" "$T5_OUT"

# T6 — the drift guard (C-5): read the upstream Literal, demand a family for
# every value. Runs on the dispatch host, where claude-pilot is always present
# (it is what this file dispatches). Elsewhere it SKIPs and says so.
#
# T6-arm (F1, first architect pass): a SKIP buried in thousands of output lines
# is indistinguishable from a green to the eye that reads the last line. So the
# block emits EXACTLY ONE `DRIFT-GUARD:` marker on stdout, and a companion
# assertion reads that stdout back — a bare green with no marker is a red. The
# block runs in the current shell (so PASS/FAIL/SKIPPED survive) with its stdout
# duplicated into a capture file by process substitution.
_t6_locate_types_py() {
    # An explicit variable is authoritative — including when it points nowhere,
    # which is how T6-arm control (a) forces the SKIP branch.
    if [ -n "${CLAUDE_PILOT_TYPES:-}" ]; then
        [ -r "$CLAUDE_PILOT_TYPES" ] && printf '%s\n' "$CLAUDE_PILOT_TYPES"
        return 0
    fi
    # <meta>/mika and <meta>/.claude/worktrees/<slug>/mika both have
    # claude-pilot/ one or three levels above the repo root.
    local top c
    # From the script's own repo, never the caller's cwd (mika#2149 review, #6):
    # launched from the meta-repo root, a bare rev-parse resolved the wrong
    # toplevel and the guard SKIPped on the very host it exists to protect.
    top=$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null) || return 0
    for c in "$top/../claude-pilot/src/claude_pilot/types.py" \
             "$top/../../../../claude-pilot/src/claude_pilot/types.py"; do
        if [ -r "$c" ]; then
            (cd "$(dirname "$c")" && printf '%s/%s\n' "$(pwd -P)" "$(basename "$c")")
            return 0
        fi
    done
    return 0
}

_t6_drift_guard() {
    local types_py values n v family
    types_py=$(_t6_locate_types_py)
    if [ -z "$types_py" ]; then
        echo "DRIFT-GUARD: SKIP — types.py unreachable (set CLAUDE_PILOT_TYPES)"
        SKIPPED=$((SKIPPED + 1))
        return 0
    fi
    # The `guardrail: Literal[ ... ]` block, EVERY quoted token — then fail
    # closed on any token outside the shape a subtype can have (mika#2149
    # review, #3): a charset filter in the extraction excluded a digit-bearing
    # name instead of failing on it, so the guard counted it out and stayed
    # green. The shape mirrors the runtime scrape in dispatch-lib.sh.
    values=$(sed -n '/guardrail: Literal\[/,/\]/p' "$types_py" | grep -o '"[^"]*"' | tr -d '"' || true)
    n=$(printf '%s\n' "$values" | grep -c . || true)
    echo "DRIFT-GUARD: armed against $types_py ($n values)"
    if [ "$n" -eq 0 ]; then
        FAIL=$((FAIL + 1))
        echo "  ✗ T6: the Literal block was found empty — the sed anchor no longer matches types.py"
        return 0
    fi
    for v in $values; do
        if ! grep -Eq -- '^[a-z][a-z0-9_]*$' <<<"$v"; then
            FAIL=$((FAIL + 1))
            echo "  ✗ T6: upstream token '$v' is outside the subtype shape [a-z][a-z0-9_]* — widen the runtime scrape and this guard together"
            continue
        fi
        family=$( ( source "$DISPATCH_LIB" 2>/dev/null || true; _halt_family "$v" 2>/dev/null | cut -d'|' -f1 ) )
        # Positive control first: an empty family (function renamed, source
        # aborted in the subshell) must not pass the drift check vacuously.
        assert_eq "T6 drift: upstream value '$v' resolved to a non-empty family" \
            "yes" "$([ -n "$family" ] && echo yes || echo no)"
        assert_not_contains "T6 drift: upstream value '$v' has a downstream family (got: $family)" \
            "unknown" "$family"
    done
}

T6_CAPTURE=$(mktemp)
_t6_drift_guard > >(tee "$T6_CAPTURE")
wait $! 2>/dev/null || true
T6_MARKER_COUNT=$(grep -c '^DRIFT-GUARD: ' "$T6_CAPTURE" || true)
assert_eq "T6-arm: exactly one DRIFT-GUARD marker was emitted on stdout" \
    "1" "$T6_MARKER_COUNT"
rm -f "$T6_CAPTURE"

# T6 fixtures — the guard's own branches, driven in a subshell so its
# counters do not leak into this run's (mika#2149 review, #3 and testing gaps).
# Each fixture is a minimal types.py; the guard's stdout is the assertion
# surface, and its FAIL count is printed last so the fail-closed direction is
# pinned by number, not by prose.
_t6_fixture_probe() {
    local body="$1" fx
    fx=$(mktemp)
    printf '%s\n' "$body" > "$fx"
    (
        PASS=0; FAIL=0; SKIPPED=0
        CLAUDE_PILOT_TYPES="$fx" _t6_drift_guard
        echo "FAIL=$FAIL SKIPPED=$SKIPPED"
    )
    rm -f "$fx"
}
# (a) a digit-bearing name is iterated, not silently dropped: it reaches the
#     table and comes back unknown, and the guard names it.
T6_FX_DIGIT=$(_t6_fixture_probe '    guardrail: Literal[
        "idle_timeout",
        "http_529",
    ]')
assert_contains "T6 fixture: a digit-bearing upstream value is counted" \
    "(2 values)" "$T6_FX_DIGIT"
assert_contains "T6 fixture: a digit-bearing unknown value is named red" \
    "✗ T6 drift: upstream value 'http_529' has a downstream family (got: unknown)" "$T6_FX_DIGIT"
assert_contains "T6 fixture: exactly one red for the one unknown value" \
    "FAIL=1 SKIPPED=0" "$T6_FX_DIGIT"
# (b) a token outside the subtype shape fails closed, by name.
T6_FX_SHAPE=$(_t6_fixture_probe '    guardrail: Literal[
        "idle_timeout",
        "Bad-Token",
    ]')
assert_contains "T6 fixture: an out-of-shape token is refused by name" \
    "upstream token 'Bad-Token' is outside the subtype shape" "$T6_FX_SHAPE"
assert_contains "T6 fixture: the refusal counts as a FAIL" \
    "FAIL=1 SKIPPED=0" "$T6_FX_SHAPE"
# (c) a matched-but-empty Literal block is a red, not a vacuous green.
T6_FX_EMPTY=$(_t6_fixture_probe '    guardrail: Literal[
    ]')
assert_contains "T6 fixture: an empty Literal block is named" \
    "the Literal block was found empty" "$T6_FX_EMPTY"
assert_contains "T6 fixture: an empty Literal block is a FAIL" \
    "FAIL=1 SKIPPED=0" "$T6_FX_EMPTY"
# (d) the SKIP branch, in-file: marker + SKIPPED, zero content assertions.
T6_FX_SKIP=$( ( PASS=0; FAIL=0; SKIPPED=0; CLAUDE_PILOT_TYPES=/nonexistent _t6_drift_guard; echo "FAIL=$FAIL SKIPPED=$SKIPPED PASS=$PASS" ) )
assert_contains "T6 fixture: unreachable types.py emits the SKIP marker" \
    "DRIFT-GUARD: SKIP" "$T6_FX_SKIP"
assert_contains "T6 fixture: unreachable types.py counts SKIPPED and asserts nothing" \
    "FAIL=0 SKIPPED=1 PASS=0" "$T6_FX_SKIP"

# T2-sink — the drift line through the PRODUCTION channel (mika#2149 review,
# #1). The probe mirrors dispatch_claude_pilot: fd 2 is /dev/null (the
# `exec 9>>"$TRACE_FILE" 2>/dev/null` at its top), and the two sinks are the
# files the callback tail and the persisted .stderr are built from. A bare
# `>&2` passes the earlier T2 (which merges fd 2 in the probe) and lands
# nowhere here — that is the defect this probe exists to keep closed.
_classify_sink_probe() {
    local subtype="$1" tmp
    tmp=$(mktemp -d)
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        exec 2>/dev/null
        STATUS="terminated"; TURNS=2; DURATION=1; SESSION_ID=s; LOG_ID="probe-sink"
        PILOT_LOG_DIR="$tmp"
        STDERR_FILE="$tmp/stderr.tmp"; : > "$STDERR_FILE"
        PERSISTENT_STDERR="$tmp/probe-sink.stderr"; : > "$PERSISTENT_STDERR"
        SUBTYPE="$subtype"; TERMINATION_REASON="x"; API_ERROR_STATUS=""
        _classify_terminated_session >/dev/null
        printf 'tail:%s\n' "$(cat "$STDERR_FILE")"
        printf 'persisted:%s\n' "$(cat "$PERSISTENT_STDERR")"
    )
    rm -rf "$tmp"
}
T2_SINK=$(_classify_sink_probe foo_bar) || T2_SINK=""
assert_contains "T2-sink: the drift line reaches the callback-tail source (STDERR_FILE)" \
    "tail:dispatch-lib: halt_family.unknown subtype=foo_bar" "$T2_SINK"
assert_contains "T2-sink: the drift line reaches the persisted .stderr" \
    "persisted:dispatch-lib: halt_family.unknown subtype=foo_bar" "$T2_SINK"
T2_SINK_KNOWN=$(_classify_sink_probe idle_timeout) || T2_SINK_KNOWN=""
assert_not_contains "T2-sink (negative control): a known subtype writes nothing to either sink" \
    "halt_family.unknown" "$T2_SINK_KNOWN"

# The caller must route on the measurement, not on STATUS alone.
assert_contains "the terminated branch is gated on _pilot_left_no_work" \
    "_pilot_left_no_work" "$RCP_SRC_1772"
assert_contains "a terminated session with work still runs _post_flight_recovery" \
    "_post_flight_recovery" "$RCP_SRC_1772"

# The plan line prefers the measurement that can answer on a first grooming.
# _committed_plan_on_branch asks the remote AND needs a body callout, so on the
# run this line exists for it is silent; VALID_PLAN is the worktree answer.
VALID_FIRST=$(printf '%s\n' "$DISPATCH_SRC_1772" \
    | grep -nE 'VALID_PLAN:-|_committed_plan_on_branch' | head -1)
assert_contains "plan line consults the worktree measurement before the remote one" \
    "VALID_PLAN" "$VALID_FIRST"

# Reason coverage, per site rather than by total. The earlier count comparison
# carried four units of slack, so four future un-reasoned exits could land
# without tripping it.
_reason_pairing_check() {
    local body line prev1="" prev2="" bad=0 n=0
    body=$(printf '%s\n' "$ITERATE_SRC_1772")
    while IFS= read -r line; do
        case "$line" in
            *"return 1"*)
                n=$((n + 1))
                case "$line$prev1$prev2" in
                    # mika#2296: `_groom_warn_empty_content` is listed by name,
                    # not covered by relaxing the pattern to `*_groom_warn*`.
                    # It delegates to `_groom_warn` and therefore genuinely sets
                    # the reason — but a wildcard prefix would also accept any
                    # future `_groom_warn_*` that does not, which is how a
                    # detector stops detecting.
                    *_groom_warn\ *|*_groom_warn_empty_content\ *|*GROOM_LOOP_FAILURE_REASON=*) ;;
                    *) bad=$((bad + 1)); echo "    unreasoned exit: $(printf '%s' "$line" | sed 's/^ *//')" >&2 ;;
                esac
                ;;
        esac
        case "$line" in
            "") ;;
            *) prev2="$prev1"; prev1="$line" ;;
        esac
    done <<<"$body"
    echo "${bad}/${n}"
}
REASON_PAIRING=$(_reason_pairing_check 2>/dev/null)
assert_eq "every return 1 in the loop has a reason setter within two lines" \
    "0/${REASON_PAIRING#*/}" "$REASON_PAIRING"

# ===========================================================================
# mika#1772 — the 2026-07-04 signature, replayed against the real loop
#
# The incident this ticket was opened for: on mika#1723 a plan DID exist on the
# branch and `_iterate_groom_loop` still returned non-zero, while a manual
# `/mika-ask-arch` converged GROOMED the same day. The historical trace is gone
# — the worktree (and its `groom-verdict-trail.log`) was destroyed,
# GROOM_LOOP_FAILURE_REASON did not exist yet, and dispatch-lib's stderr has no
# durable sink. So the class is pinned here by construction instead: an
# architect first pass that answers plausibly WITHOUT the `Disposition:` line.
#
# Two states, because one of them alone proves nothing:
#   A. the corrective retry of mika#1823 (f8b63530, 2026-07-25) recovers, and
#      the loop converges — this is the mechanism that closed the class, and
#      nothing until now failed when it was removed;
#   B. the architect misses the line TWICE. The loop still returns 1 — correctly,
#      two failed attempts at a verdict are a real non-convergence — and the
#      question is only what it tells the operator it did.
#
# These run the real `_iterate_groom_loop` with `_arch_ask` stubbed at the
# process boundary. A structural grep would pass over both.
# ===========================================================================

echo ""
echo "Test: the 2026-07-04 UNPARSED signature (mika#1772 / #1823)"
echo "------------------------------------------------------------"

# Architect prose that is genuinely unparsable: no `Disposition:` line, no
# `Verdict:` line, and none of the fourteen paraphrase patterns tier 2 matches.
# Checked against `_parse_disposition_fuzzy` — a body that accidentally said
# "proceed" or "revise" would be parsed, and the fixture would replay a
# different class than the one it names.
UNPARSED_1772='I have read the document end to end. The framing is coherent and the
acceptance criteria tie back to the ticket. My notes on section three are
minor and mostly editorial. Nothing in here surprises me.'

# $1 = what the architect replies to the mika#1823 corrective retry.
# Emits `key=value` lines plus `trail:` / `stderr:` prefixed lines.
_groom_signature_probe_1772() {
    local retry_reply="$1"
    local tmp wt rc
    tmp=$(mktemp -d)
    wt="$tmp/wt"
    mkdir -p "$wt/docs/plans"
    {
        echo "# Plan for the replayed signature"
        echo "**Ticket:** mika issue#1723"
        for i in $(seq 1 14); do
            echo "Body line $i — padding padding padding padding padding."
        done
    } > "$wt/docs/plans/2026-07-01-001-fix-1723-signature-plan.md"

    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true

        ARCH_CALLS="$tmp/arch-calls"
        printf '0' > "$ARCH_CALLS"

        # Stub at the process boundary: the parsers, the retry envelope, the
        # trail and the terminal states all stay real.
        _arch_ask() {
            local n body
            n=$(cat "$ARCH_CALLS")
            n=$((n + 1))
            printf '%s' "$n" > "$ARCH_CALLS"
            case "$n" in
                1) body="$UNPARSED_1772" ;;   # first pass — the 2026-07-04 shape
                2) body="$retry_reply" ;;     # the mika#1823 corrective retry
                *) body='Verdict: GROOMED' ;; # second review
            esac
            jq -n --arg c "$body" \
                '{content:$c, metadata:{session_id:"probe-session-1723"}}'
        }
        # Real one shells out to `gh issue edit`.
        _write_canonical_callout() { printf 'written\n' > "$tmp/callout"; return 0; }

        WORKTREE_DIR="$wt" ISSUE_NUM="1723" REPO="mika" BRANCH="probe-1723"
        if _iterate_groom_loop >/dev/null 2>"$tmp/err"; then rc=0; else rc=1; fi

        printf 'rc=%s\n' "$rc"
        printf 'arch_calls=%s\n' "$(cat "$ARCH_CALLS")"
        printf 'reason=%s\n' "${GROOM_LOOP_FAILURE_REASON:-<none>}"
        printf 'callout=%s\n' "$([ -f "$tmp/callout" ] && echo yes || echo no)"
        # Field 4 of the trail is the outcome, in call order. Joined rather than
        # emitted line-per-line so an assertion can pin the whole sequence: the
        # individual tokens overlap (`UNPARSED` is a prefix of
        # `UNPARSED-after-retry`) and match each other on a substring test.
        printf 'trail_outcomes=%s\n' \
            "$(cut -f4 "$wt/.claude/groom-verdict-trail.log" 2>/dev/null | paste -sd, -)"
        sed 's/^/stderr: /' "$tmp/err" 2>/dev/null
    )
    rm -rf "$tmp"
}

# --- A: the retry recovers, and the loop converges -------------------------

SIG_RECOVERED_1772=$(_groom_signature_probe_1772 'Apologies — restating with the required line.

Disposition: READY') || SIG_RECOVERED_1772=""

# Anti-vacuity anchor, first: without it every assertion below would pass on an
# empty string if the probe ever stopped running.
assert_contains "A: the probe drove the loop through all three architect calls" \
    "arch_calls=3" "$SIG_RECOVERED_1772"
assert_contains "A: the 2026-07-04 signature converges (loop returns zero)" \
    "rc=0" "$SIG_RECOVERED_1772"
assert_contains "A: convergence is announced for the issue" \
    "converged on GROOMED for mika#1723" "$SIG_RECOVERED_1772"
assert_contains "A: the canonical callout is written on convergence" \
    "callout=yes" "$SIG_RECOVERED_1772"
assert_contains "A: a converged run records no failure reason" \
    "reason=<none>" "$SIG_RECOVERED_1772"
# The trail is what distinguishes "converged" from "converged BECAUSE of the
# retry". Pinned as the whole sequence: a loop that never needed a second
# attempt would read `READY,GROOMED` and satisfy any per-token check.
assert_contains "A: the trail is first-pass UNPARSED, then a retry, then GROOMED" \
    "trail_outcomes=UNPARSED,READY-after-retry,GROOMED" "$SIG_RECOVERED_1772"
assert_contains "A: the retry announces itself as the mika#1823 mechanism" \
    "disposition recovered on retry (READY) — mika#1823" "$SIG_RECOVERED_1772"

# --- B: the architect misses the line twice --------------------------------

SIG_DOUBLE_1772=$(_groom_signature_probe_1772 "$UNPARSED_1772") || SIG_DOUBLE_1772=""

assert_contains "B: the retry envelope is bounded at two attempts" \
    "arch_calls=2" "$SIG_DOUBLE_1772"
assert_contains "B: a double miss is a non-convergence (loop returns non-zero)" \
    "rc=1" "$SIG_DOUBLE_1772"
assert_contains "B: the trail records both attempts, the second as a retry" \
    "trail_outcomes=UNPARSED,UNPARSED-after-retry" "$SIG_DOUBLE_1772"

# The reason travels to `tasks.result` (PR#2028) and is the only thing the
# operator sees. This exit is the loop's designed, bounded double-UNPARSED
# terminal — `_parse_disposition` emits READY|ITERATE|ESCALATE or nothing, so
# the `*)` arm fires on empty and on nothing else. Calling that an invariant
# violation sends the operator hunting a bug in the loop instead of reading
# "the architect never emitted the line, twice" — which is the mika#1772 defect
# class exactly, surviving PR#2028 at the one site its author believed
# unreachable.
#
# Asserted against the `reason=` line ALONE, not against the probe's whole
# output: the loop's own stderr already carries both "disposition UNPARSED" and
# "mika#1823" from the FIRST attempt, so a whole-output match reported these as
# satisfied while the reason itself said none of it. Measured — the two needles
# below passed against the pre-fix reason before this narrowing.
SIG_DOUBLE_REASON_1772=$(printf '%s\n' "$SIG_DOUBLE_1772" | grep '^reason=' || true)
assert_contains "B: the reason names the unparsed disposition" \
    "disposition UNPARSED" "$SIG_DOUBLE_REASON_1772"
assert_contains "B: the reason says both attempts missed" \
    "2 attempts" "$SIG_DOUBLE_REASON_1772"
assert_contains "B: the reason names the retry that already ran" \
    "mika#1823" "$SIG_DOUBLE_REASON_1772"
assert_not_contains "B: the designed terminal is not reported as an invariant violation" \
    "invariant violation" "$SIG_DOUBLE_REASON_1772"

# --- Test: the egress launch guard must constate, not affirm (mika#2041) ---
#
# Founding incident (2026-08-29 06:07-06:11 CEST): the host egress proxy was
# killed. A kill does not unlink a unix socket, so the path survived as an
# orphan. dispatch-lib respawned a proxy on every dispatch, every proxy died,
# and dispatch-lib reported success each time -- the pilot went out with no
# egress and no `fs-only` line to say so.
#
# The wait guard asked `[ -S "$sock" ]` -- "is there a file of type socket" --
# where the question is "is anyone listening". An orphan satisfies the first
# and fails the second, so the loop exited on its first iteration and the
# fs-only fallback became unreachable in exactly the scenario that needs it.
#
# ANTI-VACUITY (plan KTD6): the `ghost` assertions below FAIL against the
# pre-fix guard -- measured rc=0 / launched-ok where the fix gives rc=1 /
# fs-only. A structural grep would have passed over dead code; this exercises
# the real `_ensure_pilot_egress_proxy`.

echo ""
echo "Test: egress launch guard tests connectability, not file existence (mika#2041)"
echo "------------------------------------------------------------"

# The operational proxy log is the diagnostic surface this whole ticket rests
# on. A suite that appends fake-proxy output into it destroys the evidence it
# exists to make legible, so the probe redirects the log and we assert the real
# file never grew (plan R8).
#
# Assert on the fixture's own line, not on the file's size: on a host where the
# proxy is alive and serving, the log grows from real traffic while the suite
# runs, so a size comparison would fail for reasons that have nothing to do
# with this test. The fixture string can only appear there if the override broke.
_REAL_EGRESS_LOG="/var/log/mika/pilot-egress-proxy.log"
_egress_log_fixture_hits() {
    # -r, not -f: a deploy run as root leaves a log this suite cannot read, and
    # `grep -c` then exits 2 printing nothing, so the assertion would compare
    # "0" against "" and fail for a reason unrelated to the test.
    [ -r "$_REAL_EGRESS_LOG" ] || { echo 0; return 0; }
    # `grep -c` already prints 0 on no-match and exits 1; `|| true` swallows the
    # status without printing a second count.
    grep -c "mika#2041 test fixture" "$_REAL_EGRESS_LOG" 2>/dev/null || true
}

# _egress_guard_probe <sock_state> <bin_state> [sock_basename]
#   sock_state: ghost  -- bind then close without unlink (the orphan shape)
#               live   -- a real listener held open
#               absent -- nothing at the path
#   bin_state:  dies   -- an executable proxy that exits before binding
#               missing-- no proxy binary at all
# Echoes "rc=<n> launched=<yes|no> msg=<fs-only|phase2b|launched-ok|none>".
_egress_guard_probe() {
    local sock_state="$1" bin_state="$2" sock_name="${3:-mika-pilot-egress.sock}"
    local tmp sock bin logdir marker out rc listener_pid launched msg i
    tmp=$(mktemp -d)
    sock="$tmp/$sock_name"
    bin="$tmp/fake-proxy"
    logdir="$tmp/logs"
    marker="$tmp/launched"
    mkdir -p "$logdir"

    # A proxy that records its own launch and then dies WITHOUT binding --
    # the incident shape. Why it died is still unknown (mika#2041 comment);
    # the guard must notice either way.
    cat > "$bin" <<'FAKE_PROXY'
#!/bin/bash
# The marker lives next to this script, not in an environment variable: the
# launcher starts the relay under `env -i`, so nothing exported by the test
# reaches it.
touch "$(dirname "$0")/launched"
# Noisy on purpose: a proxy dying before bind() prints a traceback, and that
# output goes wherever the launcher points its log. If the log override ever
# regresses, this line lands in the operational log and the size assertion at
# the end of the block catches it. A silent fake would make that check vacuous.
echo "fake-proxy: dying before bind (mika#2041 test fixture)" >&2
exit 1
FAKE_PROXY
    chmod +x "$bin"
    [ "$bin_state" = "missing" ] && rm -f "$bin"

    listener_pid=""
    case "$sock_state" in
        ghost)
            python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.close()
' "$sock"
            ;;
        live)
            python3 -c '
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.listen(1)
time.sleep(30)
' "$sock" &
            listener_pid=$!
            i=0
            while [ $i -lt 50 ] && [ ! -S "$sock" ]; do sleep 0.1; i=$((i + 1)); done
            ;;
    esac

    out=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        _PILOT_EGRESS_SOCK="$sock"
        _PILOT_EGRESS_PROXY_BIN="$bin"
        MIKA_PILOT_EGRESS_LOG_DIR="$logdir"
        _ensure_pilot_egress_proxy 2>&1 >/dev/null
    ) && rc=0 || rc=$?

    [ -n "$listener_pid" ] && kill "$listener_pid" 2>/dev/null
    [ -n "$listener_pid" ] && wait "$listener_pid" 2>/dev/null

    # The launch is a detached `nohup ... &`, so the marker can land after the
    # guard returns. Without this bounded wait the assertion races the fork and
    # reports launched=no for a proxy that did start.
    launched=no
    i=0
    while [ $i -lt 20 ] && [ ! -f "$marker" ]; do sleep 0.05; i=$((i + 1)); done
    [ -f "$marker" ] && launched=yes
    # mika#2049 re-tokenised these two lines. Pre-2049 BOTH ended in "(falling
    # back to fs-only)" and only one carried a stable token, so this `case` had
    # to be ordered defensively to keep them apart -- and an operator using
    # `pilot_egress_guard.unreachable` as THE predicate read a nominal regime on
    # a fleet whose proxy binary was never deployed (the gap mika#2050 had to
    # document). Each cause now carries its own token, so the two arms are
    # disjoint by construction rather than by ordering.
    #
    # The strings no longer say "falling back" because nothing falls back any
    # more: the caller refuses. Leaving them would have made Signal S (mika#2050)
    # count a population that can no longer exist.
    case "$out" in
        *"pilot_egress_guard.binary_missing"*) msg=binary-missing ;;
        *"pilot_egress_guard.unreachable"*) msg=unreachable ;;
        *"pilot-egress-proxy launched"*) msg=launched-ok ;;
        *) msg=none ;;
    esac
    rm -rf "$tmp"
    printf 'rc=%s launched=%s msg=%s' "$rc" "$launched" "$msg"
}

# THE regression. Pre-fix this is "rc=0 launched=yes msg=launched-ok": the
# orphan file satisfies [ -S ], the guard affirms a launch that never happened.
assert_eq "orphan socket + proxy that dies before binding => guard reports unreachable" \
    "rc=1 launched=yes msg=unreachable" \
    "$(_egress_guard_probe ghost dies)"

# The liveness probe must still recognise a real listener after the probe was
# factored out -- and must not relaunch behind its back.
assert_eq "live listener => already-alive, proxy not relaunched" \
    "rc=0 launched=no msg=none" \
    "$(_egress_guard_probe live dies)"

# No file at the path: this already worked pre-fix. Locks it against regression.
assert_eq "no socket at path + proxy that dies => guard reports unreachable" \
    "rc=1 launched=yes msg=unreachable" \
    "$(_egress_guard_probe absent dies)"

# The two causes must stay distinguishable: a missing binary is a deploy state
# (remedy: `make install`), an unreachable socket is a runtime failure (remedy:
# restart the relay). Same rc, different token, different gesture -- which is
# why mika#2049 gave each one a token of its own.
assert_eq "missing proxy binary => binary_missing, not unreachable" \
    "rc=1 launched=no msg=binary-missing" \
    "$(_egress_guard_probe ghost missing)"

# The probe passes the path as argv, never interpolated into python source
# (plan KTD2). A quote in the path used to be a syntax error waiting to happen.
assert_eq "socket path containing a single quote does not break the probe" \
    "rc=1 launched=yes msg=unreachable" \
    "$(_egress_guard_probe ghost dies "mika'\''egress.sock")"

# R8: no fake-proxy output may reach the operational proxy log.
assert_eq "fake-proxy output never reaches the operational proxy log" \
    "0" "$(_egress_log_fixture_hits)"

# --- mika#2051: the refusal line names the pid it just launched -------------
#
# A SISTER function rather than an extension of `_egress_guard_probe`, and the
# reason is the probe's contract: it CLASSIFIES the output into one token
# (`msg=unreachable`) and then throws the text away. Five assertions above
# compare its `rc=… launched=… msg=…` string by strict equality, so widening it
# by one field would redden all five for a need that concerns one. The two
# functions say what they each do: one tests the DECISION, this one the TEXT.
#
# Same fake proxy, same log redirect -- the redirect is not incidental. Without
# it the fake proxy's traceback lands in the operational log, the very file this
# ticket exists to make legible, and the assertion at the end of the block above
# is what catches that regression.
# _egress_guard_line [bin_state]   bin_state: dies (default) | missing
_egress_guard_line() {
    local bin_state="${1:-dies}"
    local tmp sock bin logdir out
    tmp=$(mktemp -d)
    sock="$tmp/mika-pilot-egress.sock"
    bin="$tmp/fake-proxy"
    logdir="$tmp/logs"
    mkdir -p "$logdir"

    cat > "$bin" <<'FAKE_PROXY'
#!/bin/bash
echo "fake-proxy: dying before bind (mika#2041 test fixture)" >&2
exit 1
FAKE_PROXY
    chmod +x "$bin"
    [ "$bin_state" = "missing" ] && rm -f "$bin"

    # The orphan shape: bind then close without unlink. `[ -S ]` is satisfied,
    # nothing listens -- the population mika#2041 measured.
    python3 -c '
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.close()
' "$sock"

    out=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        _PILOT_EGRESS_SOCK="$sock"
        _PILOT_EGRESS_PROXY_BIN="$bin"
        MIKA_PILOT_EGRESS_LOG_DIR="$logdir"
        _ensure_pilot_egress_proxy 2>&1 >/dev/null
    ) || true
    rm -rf "$tmp"
    printf '%s' "$out"
}

_EGRESS_UNREACHABLE_LINE=$(_egress_guard_line)

# The joint. `pilot_egress_startup.begin pid=<pid>` is the proxy's first line
# (contract held by test_startup_emits_a_begin_breadcrumb_before_bind); the
# launcher's SUCCESS line already carried the pid and its FAILURE line -- the
# only one this ticket exists to diagnose -- did not. Without it the operator
# joins the dispatch `.stderr` to a cumulative proxy log by timestamp, which is
# the friction that made the 2026-08-29 diagnosis expensive.
assert_eq "mika#2051: the unreachable refusal names the pid it launched" \
    "yes" \
    "$(grep -qE 'pid [0-9]+' <<<"$_EGRESS_UNREACHABLE_LINE" && echo yes || echo no)"

# The published predicates grep `^dispatch-lib: ` + a contiguous token (Signal S
# anchor, runbook §2). Assert THAT invariant, never the pid's position: pinning
# the position would freeze a wording choice where what matters is what the
# operator greps bite on.
assert_eq "mika#2051: the refusal keeps its anchor and contiguous token" \
    "yes" \
    "$(grep -qE '^dispatch-lib: pilot_egress_guard\.unreachable ' <<<"$_EGRESS_UNREACHABLE_LINE" && echo yes || echo no)"

# The Signal S predicate (CLAUDE.md) stops at the token FAMILY, `pilot_egress_guard.`,
# precisely so it covers both causes -- mika#2050's lesson being that an operator
# who takes `unreachable` for THE predicate reads a nominal regime on a fleet
# whose proxy binary was never deployed. A predicate published over two tokens
# needs both to be witnessed, or half of it is an assumption.
_EGRESS_BINARY_MISSING_LINE=$(_egress_guard_line missing)

assert_eq "mika#2051: the Signal S family predicate bites on unreachable" \
    "yes" \
    "$(grep -qE '^dispatch-lib: pilot_egress_guard\.' <<<"$_EGRESS_UNREACHABLE_LINE" && echo yes || echo no)"

assert_eq "mika#2051: the Signal S family predicate bites on binary_missing" \
    "yes" \
    "$(grep -qE '^dispatch-lib: pilot_egress_guard\.binary_missing ' <<<"$_EGRESS_BINARY_MISSING_LINE" && echo yes || echo no)"

# binary_missing launched NO proxy, so it has no pid to name. Stamping one would
# be a false statement, and this is the negative control that keeps the pid from
# being spread to the line where it would be a lie.
assert_eq "mika#2051: binary_missing names no pid, because it launched none" \
    "no" \
    "$(grep -qE 'pid [0-9]+' <<<"$_EGRESS_BINARY_MISSING_LINE" && echo yes || echo no)"

# R8 again, and it must be HERE rather than only above: the sister function
# launches its own fake proxy, so the earlier assertion -- which runs before
# these calls -- cannot witness it. A log override that regressed only in
# `_egress_guard_line` would leave the first check green while this suite wrote
# fake-proxy tracebacks into the operational log, i.e. destroyed the very
# evidence surface mika#2051 exists to make legible.
assert_eq "mika#2051: the sister function does not reach the operational proxy log either" \
    "0" "$(_egress_log_fixture_hits)"

# ============================================================================
# Dispatchable-repo allowlist: shell defense in depth (mika#2062)
# ============================================================================
# Follow-up of #2046 (KTD5). The Rust tool-boundary guard is load-bearing on the
# nominal loop path; _set_up_worktree adds a shell copy of the same allowlist for
# out-of-engine callers (manual runs, scripts, recovery paths) that never reach
# the Rust guard. These tests are anti-vacuity in both directions: an allowlisted
# repo must pass the gate, a non-allowlisted one must be refused.

echo ""
echo "Test: dispatchable-repo allowlist gate (mika#2062)"
echo "--------------------------------------------------"

# --- Predicate _is_dispatchable_repo: pure, both directions ---
assert_eq "_is_dispatchable_repo accepts mika"          "0" "$(_is_dispatchable_repo mika; echo $?)"
assert_eq "_is_dispatchable_repo accepts mika-cloud"    "0" "$(_is_dispatchable_repo mika-cloud; echo $?)"
assert_eq "_is_dispatchable_repo accepts mika-skills"   "0" "$(_is_dispatchable_repo mika-skills; echo $?)"
assert_eq "_is_dispatchable_repo accepts mika-platform" "0" "$(_is_dispatchable_repo mika-platform; echo $?)"
assert_eq "_is_dispatchable_repo refuses control-monitor" "1" "$(_is_dispatchable_repo control-monitor; echo $?)"
assert_eq "_is_dispatchable_repo refuses claude-pilot"    "1" "$(_is_dispatchable_repo claude-pilot; echo $?)"
assert_eq "_is_dispatchable_repo refuses empty"           "1" "$(_is_dispatchable_repo '' 2>/dev/null; echo $?)"

# --- Integration: the gate inside _set_up_worktree, both directions ---
# Hermetic probe (no gh, no network). PLATFORM_DIR is an EMPTY temp dir, so an
# allowlisted repo clears the gate and then trips the "is not a git repository"
# check that immediately follows SUB_REPO_DIR resolution — proof it got past the
# gate. A non-allowlisted repo is refused AT the gate with `repo_not_dispatchable`
# and never reaches that check. Were the gate removed, control-monitor would also
# reach the git check (verdict "passed-gate") and this test would fail — which is
# the anti-vacuity guarantee.
PROBE_PLATFORM=$(mktemp -d)

_repo_gate_verdict() {
    local out
    out=$(
        (
            PROMPT="$1"
            PLATFORM_DIR="$PROBE_PLATFORM"
            PLATFORM_REPO_NAME="mika-platform"  # so the PLATFORM_DIR self-branch is not taken for mika/mika-cloud
            TASK_ID="probe-2062"
            _set_up_worktree
        ) 2>&1 || true
    )
    if grep -q -- 'repo_not_dispatchable' <<<"$out"; then
        printf 'refused'
    elif grep -q -- 'is not a git repository' <<<"$out"; then
        printf 'passed-gate'
    else
        printf 'unexpected'
    fi
}

assert_eq "mika#N clears the gate, reaches worktree setup"       "passed-gate" "$(_repo_gate_verdict 'mika#214')"
assert_eq "mika-cloud#N clears the gate, reaches worktree setup" "passed-gate" "$(_repo_gate_verdict 'mika-cloud#50')"
assert_eq "control-monitor#N is refused at the gate"             "refused"     "$(_repo_gate_verdict 'control-monitor#159')"
assert_eq "claude-pilot#N is refused at the gate"                "refused"     "$(_repo_gate_verdict 'claude-pilot#119')"

# The refusal names the repo in a machine-readable field (mika#2062), consistent
# with `repo_not_dispatchable` on the Rust side.
REFUSAL_OUT=$(
    (
        PROMPT="control-monitor#159"
        PLATFORM_DIR="$PROBE_PLATFORM"
        PLATFORM_REPO_NAME="mika-platform"
        TASK_ID="probe-2062"
        _set_up_worktree
    ) 2>&1 || true
)
assert_contains "refusal is named repo_not_dispatchable" 'repo_not_dispatchable' "$REFUSAL_OUT"
assert_contains "refusal carries the repo in a machine-readable field" '"repo":"senara-solutions/control-monitor"' "$REFUSAL_OUT"

rm -rf "$PROBE_PLATFORM"

# --- Drift guard: the shell copy must equal the Rust DISPATCHABLE_REPOS ---
# The list is duplicated across two languages by necessity (the Rust side is a
# compile-time &[&str], not a runtime-readable data file). Two mute copies are
# exactly the drift webhook_dispatch.rs exists to prevent, so this fails if they
# diverge (mika#2062).
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
RUST_SRC="$REPO_ROOT/crates/mika-agent/src/webhook_dispatch.rs"
if [ -f "$RUST_SRC" ]; then
    RUST_LIST=$(sed -n '/const DISPATCHABLE_REPOS/,/\];/p' "$RUST_SRC" \
        | grep -oE '"senara-solutions/[^"]+"' \
        | sed 's#"senara-solutions/##; s#"##' \
        | sort | paste -sd' ' -)
    SHELL_LIST=$(printf '%s\n' "${DISPATCHABLE_REPO_BASENAMES[@]}" | sort | paste -sd' ' -)
    assert_eq "shell allowlist matches Rust DISPATCHABLE_REPOS (no drift)" "$RUST_LIST" "$SHELL_LIST"
fi

# --- Drift guard: the withheld-disposition marker must equal the Rust constant ---
# Same necessity as the list above: the engine side is a compile-time `&str`, not a
# runtime-readable data file, so the literal exists in both languages. Here the stakes
# are higher than drift-as-noise — if the two copies diverge, the engine still strips
# the disposition but tier 0 stops recognizing the substitute, and every tier below it
# resumes deriving a verdict from the body. The refusal would fail OPEN, silently
# (mika#2037).
ANCHOR_RUST_SRC="$REPO_ROOT/crates/mika-agent/src/agent_loop/mod.rs"
if [ -f "$ANCHOR_RUST_SRC" ]; then
    RUST_MARKER=$(grep -oE 'const DISPOSITION_WITHHELD_MARKER: &str = "[^"]+"' "$ANCHOR_RUST_SRC" \
        | sed 's/.*= "//; s/"$//')
    assert_eq "shell tier-0 marker matches Rust DISPOSITION_WITHHELD_MARKER (no drift)" \
        "$RUST_MARKER" "$WITHHELD_MARKER"
    # The shell parser must actually contain the literal, not merely compare equal to a
    # variable this test defined.
    if grep -qF "$RUST_MARKER" "$DISPATCH_LIB"; then
        assert_eq "dispatch-lib carries the marker literal" "yes" "yes"
    else
        assert_eq "dispatch-lib carries the marker literal" "yes" "no"
    fi
    # mika#2338 — same necessity, same shape, for the engine escalation line. If the two
    # copies diverge, tier 0b stops recognizing the engine's line and the escalation falls
    # through to the unanchored textual tiers.
    RUST_ENGINE_MARKER=$(grep -oE 'const REVIEW_ANCHOR_ENGINE_FINDING_MARKER: &str = "[^"]+"' "$ANCHOR_RUST_SRC" \
        | sed 's/.*= "//; s/"$//')
    assert_eq "shell tier-0b marker matches Rust REVIEW_ANCHOR_ENGINE_FINDING_MARKER (no drift)" \
        "$RUST_ENGINE_MARKER" "$ENGINE_FINDING_MARKER"
    if grep -qF "$RUST_ENGINE_MARKER" "$DISPATCH_LIB"; then
        assert_eq "dispatch-lib carries the engine marker literal" "yes" "yes"
    else
        assert_eq "dispatch-lib carries the engine marker literal" "yes" "no"
    fi
    # The literal check above is satisfied by a comment; the RECOGNIZER stores the marker
    # as an escaped regex. Bind the regex itself to the Rust constant by feeding a line
    # composed exactly as escalate_unattested_disposition composes it (`<prefix> <MARKER> …`)
    # through _engine_escalation_line — a regex-only edit can no longer stay green.
    RUST_COMPOSED_LINE="F1: $RUST_ENGINE_MARKER attestation withheld after the corrective re-prompt — anchors_found=3, anchors_valid=2, miss_reason=QuoteNotInBrief: x"
    assert_eq "tier-0b regex recognizes a line composed from the Rust constant" "$RUST_COMPOSED_LINE" \
        "$(printf 'prose\n%s\nDisposition: ESCALATE\n' "$RUST_COMPOSED_LINE" | _engine_escalation_line)"
    assert_eq "tier-0b regex rejects the Rust constant when it is not at line start" "" \
        "$(printf 'the engine would write %s here\nDisposition: READY\n' "$RUST_ENGINE_MARKER" | _engine_escalation_line)"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ Rust source not found at $RUST_SRC — cannot verify allowlist drift"
fi

# ============================================================================
# Non-empty output gate (mika#1996)
# ============================================================================
#
# A cycle that produced nothing may no longer report success — and, just as
# load-bearing, a cycle that produced something must come through the gate
# untouched. A gate that only ever fails is satisfied by "always fail", which
# is worth exactly as much as the silent success it replaces. Both directions
# are asserted below, plus the third verdict (`undetermined`) that keeps the
# gate from inventing a red when it has no ground to measure.

echo ""
echo "Test: non-empty output gate — measurement (mika#1996)"
echo "-------------------------------------------------------------------"

# Builds the exact state the gate reads: a real git worktree, a real HEAD range,
# a real stderr file with a real number of `[tool:request]` lines. Echoes either
# the verdict (mode=verdict) or the resulting RESULT (mode=result).
_gate_probe() {
    local mode="$1" dirty="$2" moved="$3" pr="$4" tools="$5" result_in="$6" wt_mode="${7:-real}" pilot_ran="${8:-1}"
    local base wt logdir pre post i
    base=$(mktemp -d "${TMPDIR:-/tmp}/mika-1996-test.XXXXXX")
    wt="$base/wt"; logdir="$base/logs"
    mkdir -p "$logdir"
    git init -q "$wt" 2>/dev/null
    echo initial > "$wt/f"
    git -C "$wt" add -A 2>/dev/null
    git -C "$wt" commit -q -m initial --no-verify 2>/dev/null
    pre=$(git -C "$wt" rev-parse HEAD)
    post="$pre"
    case "$moved" in
        content)
            echo more > "$wt/g"
            git -C "$wt" add -A 2>/dev/null
            git -C "$wt" commit -q -m "feat: real work" --no-verify 2>/dev/null
            post=$(git -C "$wt" rev-parse HEAD)
            ;;
        empty)
            # The wip(mika#1383) rescue marker is an --allow-empty commit by
            # construction: it moves HEAD without producing anything.
            git -C "$wt" commit -q --allow-empty --no-verify \
                -m "wip(mika#1383): auto-PR-create rescue" 2>/dev/null
            post=$(git -C "$wt" rev-parse HEAD)
            ;;
    esac
    if [ "$dirty" = "yes" ]; then
        echo uncommitted > "$wt/pilot-wrote-this.rs"
    fi
    if [ "$tools" != "none" ]; then
        : > "$logdir/probe-1996.stderr"
        i=0
        while [ "$i" -lt "$tools" ]; do
            echo "[tool:request] Bash {\"command\":\"ls\"}" >> "$logdir/probe-1996.stderr"
            i=$((i + 1))
        done
    fi
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$wt"
        [ "$wt_mode" = "absent" ] && WORKTREE_DIR="$base/never-created"
        PRE_RUN_HEAD="$pre"
        POST_RUN_HEAD="$post"
        PR_URL="$pr"
        LOG_ID="probe-1996"
        PILOT_LOG_DIR="$logdir"
        RESULT="$result_in"
        TASK_ID="task-1996"
        SKILL="dev-pilot"
        SESSION_ID="sess-1996"
        PILOT_RAN="$pilot_ran"
        if [ "$mode" = "verdict" ]; then
            _measure_cycle_output 2>/dev/null
            printf '%s' "$CYCLE_OUTPUT_VERDICT"
        else
            _gate_non_empty_cycle 2>/dev/null
            printf '%s' "$RESULT"
        fi
    )
    rm -rf "$base"
}

SUCCESS_RESULT="claude-pilot completed (status: success).
Session: sess-1996
Turns: 2
Cost: \$0.31
Duration: 600000ms"

# --- Direction 1: producing nothing is no longer a success -------------------

assert_eq "clean tree, HEAD unmoved, zero tool calls -> empty (the 102-of-120 shape)" \
    "empty" "$(_gate_probe verdict no none "" 0 "$SUCCESS_RESULT")"
assert_eq "an --allow-empty commit moves HEAD but produces nothing -> empty" \
    "empty" "$(_gate_probe verdict no empty "" 0 "$SUCCESS_RESULT")"
# Reading is not producing: the tool-call count can never rescue a cycle that
# left no trace outside its own process.
assert_eq "40 tool calls with nothing written -> empty" \
    "empty" "$(_gate_probe verdict no none "" 40 "$SUCCESS_RESULT")"
# PIPELINE_INCOMPLETE is a non-conclusion, not a terminal disposition.
assert_eq "a non-conclusive Outcome does not satisfy P4" \
    "empty" "$(_gate_probe verdict no none "" 5 "Outcome: PIPELINE_INCOMPLETE — manual recovery needed.")"
# P4 is a conjunction. A disposition with no tool call is text about work.
assert_eq "a terminal disposition with zero tool calls -> empty" \
    "empty" "$(_gate_probe verdict no none "" 0 "Outcome: PLAN_GROOMED")"

# --- Direction 2: producing something still succeeds -------------------------

assert_eq "commits carrying a non-empty diff -> produced" \
    "produced" "$(_gate_probe verdict no content "" 0 "$SUCCESS_RESULT")"
assert_eq "a dirty worktree -> produced (the mika#1282 rescue shape)" \
    "produced" "$(_gate_probe verdict yes none "" 0 "$SUCCESS_RESULT")"
assert_eq "an open PR -> produced" \
    "produced" "$(_gate_probe verdict no none "https://github.com/senara-solutions/mika/pull/1" 0 "$SUCCESS_RESULT")"
# The legitimately short cycle: a grooming that concludes, having actually acted.
assert_eq "a terminal disposition backed by tool calls -> produced (no false red)" \
    "produced" "$(_gate_probe verdict no none "" 3 "Outcome: PLAN_GROOMED")"
assert_eq "the crash path's PR: line is read when PR_URL was never set" \
    "produced" "$(_gate_probe verdict no none "" 0 "HANDLER CRASH (exit code 1).
PR: https://github.com/senara-solutions/mika/pull/2")"

# --- Third verdict: no ground to measure is not a failure --------------------

assert_eq "an unreadable worktree -> undetermined, never empty" \
    "undetermined" "$(_gate_probe verdict no none "" 0 "$SUCCESS_RESULT" absent)"

echo ""
echo "Test: non-empty output gate — effect on the callback (mika#1996)"
echo "-------------------------------------------------------------------"

GATE_EMPTY_OUT=$(_gate_probe result no none "" 0 "$SUCCESS_RESULT")
assert_contains "an empty cycle is banner-marked as a pipeline failure" \
    "PIPELINE FAILURE: empty_completion" "$GATE_EMPTY_OUT"
assert_contains "the banner carries the measurement, not an assertion" \
    "HEAD did not move" "$GATE_EMPTY_OUT"
assert_contains "the banner names the tool-call count it read" \
    "tool calls: 0" "$GATE_EMPTY_OUT"
assert_contains "the banner states the definition it applied" \
    "observable outside its own process" "$GATE_EMPTY_OUT"
assert_contains "an empty cycle gets a PIPELINE_INCOMPLETE outcome" \
    "Outcome: PIPELINE_INCOMPLETE — empty_completion:" "$GATE_EMPTY_OUT"
GATE_EMPTY_OUTCOMES=$(grep -c '^Outcome: ' <<<"$GATE_EMPTY_OUT" || true)
assert_eq "an empty cycle carries exactly one Outcome line" "1" "$GATE_EMPTY_OUTCOMES"

# The positive direction is an invariant, not an optimisation: byte for byte.
for _case in "no content  0" "yes none  0" "no none  3"; do
    set -- $_case
    _d="$1"; _m="$2"; _t="$3"
    GATE_PRODUCED_IN="$SUCCESS_RESULT
Outcome: PR_OPENED — https://github.com/senara-solutions/mika/pull/3"
    GATE_PRODUCED_OUT=$(_gate_probe result "$_d" "$_m" "" "$_t" "$GATE_PRODUCED_IN")
    assert_eq "a producing cycle leaves RESULT byte-for-byte identical (dirty=$_d moved=$_m tools=$_t)" \
        "$GATE_PRODUCED_IN" "$GATE_PRODUCED_OUT"
done

# Undetermined says so and leaves the cycle's own outcome alone.
GATE_UNDET_OUT=$(_gate_probe result no none "" 0 "$SUCCESS_RESULT
Outcome: PLAN_COMMITTED — docs/plans/x.md" absent)
assert_contains "an undetermined measurement is reported as such" \
    "Measurement: cycle output undetermined" "$GATE_UNDET_OUT"
assert_not_contains "an undetermined measurement never fabricates a failure" \
    "PIPELINE FAILURE" "$GATE_UNDET_OUT"
assert_contains "an undetermined measurement leaves the cycle's outcome intact" \
    "Outcome: PLAN_COMMITTED" "$GATE_UNDET_OUT"

# A cycle that is already red keeps its own, more specific diagnosis.
ALREADY_RED="PIPELINE FAILURE: the claude-pilot session was terminated before it produced any work.

Outcome: PIPELINE_INCOMPLETE — pilot session terminated by claude-pilot before producing work."
GATE_RED_OUT=$(_gate_probe result no none "" 0 "$ALREADY_RED")
GATE_RED_COUNT=$(grep -c 'PIPELINE FAILURE:' <<<"$GATE_RED_OUT" || true)
assert_eq "an already-red callback does not get a second banner stacked on it" \
    "1" "$GATE_RED_COUNT"
assert_contains "an already-red callback keeps its own diagnosis" \
    "terminated before it produced any work" "$GATE_RED_OUT"

# Idempotence: _deliver_callback and the EXIT trap can both fire in one process.
_gate_twice_probe() {
    local base wt logdir pre
    base=$(mktemp -d "${TMPDIR:-/tmp}/mika-1996-twice.XXXXXX")
    wt="$base/wt"; logdir="$base/logs"; mkdir -p "$logdir"
    git init -q "$wt" 2>/dev/null
    echo initial > "$wt/f"; git -C "$wt" add -A 2>/dev/null
    git -C "$wt" commit -q -m initial --no-verify 2>/dev/null
    pre=$(git -C "$wt" rev-parse HEAD)
    : > "$logdir/probe-1996.stderr"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$wt" PRE_RUN_HEAD="$pre" POST_RUN_HEAD="$pre" PR_URL=""
        LOG_ID="probe-1996" PILOT_LOG_DIR="$logdir"
        RESULT="claude-pilot completed (status: success)."
        PILOT_RAN=1
        _gate_non_empty_cycle 2>/dev/null
        _gate_non_empty_cycle 2>/dev/null
        printf '%s' "$RESULT"
    )
    rm -rf "$base"
}
GATE_TWICE_OUT=$(_gate_twice_probe)
GATE_TWICE_COUNT=$(grep -c 'PIPELINE FAILURE: empty_completion' <<<"$GATE_TWICE_OUT" || true)
assert_eq "two gate passes produce exactly one banner" "1" "$GATE_TWICE_COUNT"

# The rewrite is line-anchored and hits the first Outcome only: a quoted
# `Outcome:` inside the pilot's own text is data, not the cycle's verdict.
GATE_QUOTED_OUT=$(_gate_probe result no none "" 0 "claude-pilot completed (status: success).
    Outcome: PLAN_GROOMED   <- quoted from the transcript, not the verdict
Outcome: UNKNOWN — inspect worktree manually.")
assert_contains "the quoted Outcome line survives the rewrite" \
    "    Outcome: PLAN_GROOMED   <- quoted from the transcript" "$GATE_QUOTED_OUT"
assert_not_contains "the cycle's own UNKNOWN outcome is replaced" \
    "Outcome: UNKNOWN" "$GATE_QUOTED_OUT"

# No Outcome line at all: one is appended rather than silently dropped.
GATE_NOOUTCOME_OUT=$(_gate_probe result no none "" 0 "claude-pilot completed (exit 0) but output was not structured JSON.")
assert_contains "a callback with no Outcome line gets one" \
    "Outcome: PIPELINE_INCOMPLETE — empty_completion:" "$GATE_NOOUTCOME_OUT"

# The dispatcher's own deliberate exits are decisions, not cycles. Their
# callbacks are a JSON document mika-dev and the audit dashboard parse; a banner
# prefixed onto one would both invent a failure and break the parse.
AUTOSKIP_IN='{"status":"auto_skipped","reason":"issue_closed","issue":"senara-solutions/mika#1","note":"Issue was already closed before dispatch fired."}'
AUTOSKIP_OUT=$(_gate_probe result no none "" 0 "$AUTOSKIP_IN" real 0)
assert_eq "an auto-skip callback (no pilot ran) is left byte-for-byte identical" \
    "$AUTOSKIP_IN" "$AUTOSKIP_OUT"
if jq -e . >/dev/null 2>&1 <<<"$AUTOSKIP_OUT"; then
    PASS=$((PASS + 1)); echo "  ✓ the auto-skip callback is still valid JSON after the gate"
else
    FAIL=$((FAIL + 1)); echo "  ✗ the auto-skip callback is still valid JSON after the gate"
fi

# A push violation already names a more specific cause than "nothing produced".
PUSHVIOL_IN="STRUCTURAL VIOLATION: pilot push detected (mika#1318).

Outcome: PIPELINE_INCOMPLETE — push violation"
PUSHVIOL_OUT=$(_gate_probe result no none "" 0 "$PUSHVIOL_IN")
assert_eq "a push-violation callback keeps its own diagnosis untouched" \
    "$PUSHVIOL_IN" "$PUSHVIOL_OUT"

# An operator cancel must keep STATUS= as the first line the parser sees.
CANCEL_OUT=$(_gate_probe result no none "" 0 "STATUS=CANCELLED_BY_OPERATOR

Original exit code: 143
claude-pilot completed (status: success).")
assert_eq "a cancelled cycle still leads with its STATUS= line (mika#749 parser contract)" \
    "STATUS=CANCELLED_BY_OPERATOR" "$(head -1 <<<"$CANCEL_OUT")"
assert_not_contains "a cancelled cycle is not relabelled as an empty completion" \
    "empty_completion" "$CANCEL_OUT"

echo ""
echo "Test: the gate cannot be bypassed (mika#1996, CONTROL-MUST-BE-UNAVOIDABLE)"
echo "-------------------------------------------------------------------"

# A guarantee exists only if EVERY path producing the guarded effect crosses the
# control point. The guarded effect is "a cycle verdict reaches mika-dev". This
# guard fails if a future delivery site is ever added without the gate — the
# exact way an invariant of this shape gets lost.
_delivery_blocks() {
    local want="$1"   # all | ungated
    awk -v want="$want" '
        /^[A-Za-z_][A-Za-z0-9_]*\(\)[[:space:]]*\{/ {
            fn = $1; sub(/\(\).*/, "", fn); body = ""; inbody = 1; next
        }
        inbody && /^\}/ {
            if (body ~ /--task-complete/) {
                if (want == "all") print fn
                else if (body !~ /_gate_non_empty_cycle/) print fn
            }
            inbody = 0; body = ""; next
        }
        inbody { body = body "\n" $0 }
    ' "$DISPATCH_LIB"
}
UNGATED=$(_delivery_blocks ungated)
assert_eq "every function that delivers a callback calls the gate" "" "$UNGATED"
DELIVERY_FNS=$(_delivery_blocks all | sort | paste -sd' ' -)
assert_eq "the delivery sites are the two we know about" \
    "_deliver_callback _dispatch_lib_exit_trap" "$DELIVERY_FNS"

# The gate must be the FIRST executable statement of _deliver_callback: an early
# return added above it would be a hole opened without anyone noticing.
DELIVER_FIRST=$(sed -n '/^_deliver_callback() {/,/^}/p' "$DISPATCH_LIB" \
    | sed '1d' | grep -vE '^[[:space:]]*(#|$)' | head -1)
assert_contains "the gate is the first executable statement of _deliver_callback" \
    "_gate_non_empty_cycle" "$DELIVER_FIRST"

# The measurement stays side-effect free: it is what makes the positive
# direction provable. If it ever writes RESULT, the byte-for-byte invariant
# above becomes untestable rather than false, which is worse.
MEASURE_SRC=$(sed -n '/^_measure_cycle_output() {/,/^}/p' "$DISPATCH_LIB")
assert_not_contains "the measurement never writes RESULT" \
    "RESULT=" "$MEASURE_SRC"

# PILOT_RAN is what separates "the cycle produced nothing" from "no cycle ran".
# A second writer would blur the two, which is how a gate starts excusing the
# population it exists to catch.
PILOT_RAN_WRITERS=$(grep -c '^[[:space:]]*PILOT_RAN=1[[:space:]]*$' "$DISPATCH_LIB" || true)
assert_eq "PILOT_RAN has exactly one writer" "1" "$PILOT_RAN_WRITERS"
PILOT_RAN_FN=$(awk '
    /^[A-Za-z_][A-Za-z0-9_]*\(\)[[:space:]]*\{/ { fn = $1; sub(/\(\).*/, "", fn); next }
    /^[[:space:]]*PILOT_RAN=1[[:space:]]*$/ { print fn }
' "$DISPATCH_LIB")
assert_eq "PILOT_RAN is set by the function that launches the pilot" \
    "_run_claude_pilot" "$PILOT_RAN_FN"

# In the EXIT trap the gate has to run BEFORE the cancel prefix, or the banner
# displaces the STATUS= line mika-dev's parser reads first (mika#749).
TRAP_ORDER=$(sed -n '/^_dispatch_lib_exit_trap() {/,/^}/p' "$DISPATCH_LIB" \
    | grep -nE '_gate_non_empty_cycle|Cancel discriminator envelope prefix' | head -1)
assert_contains "in the EXIT trap the gate precedes the cancel prefix" \
    "_gate_non_empty_cycle" "$TRAP_ORDER"


# --- Unknown-flag guard: dispatch-lib may only build flags claude-pilot accepts (mika#2043) ---
#
# Founding defect: dispatch-lib built a `--trace` flag from CLAUDE_PILOT_TRACE
# citing mika#1097 Step 0-B, but claude-pilot never accepted `--trace` — only
# Step 0-B's dispatch-lib half ever shipped. Measured against the installed CLI
# with dispatch-lib's exact argv, an unknown flag is NOT swallowed by the
# REMAINDER positional: argparse exits 2 with `unrecognized arguments` before
# the session starts. Nothing broke only because no skill ever set the env var;
# the first one to follow the comment's invitation would have killed every
# dispatch of that skill.
#
# Two passes, and the second is what keeps the first honest:
#   A. every flag dispatch-lib builds is in the accepted list  — hermetic, always runs
#   B. every entry of that list is really in `claude-pilot --help` — needs the binary
# Without B, pass A is just another unverified claim about claude-pilot's
# interface, written once and never reconfronted with the world — which is
# exactly the defect being closed here.

echo ""
echo "Test: claude-pilot flag-surface guard (mika#2043)"
echo "-------------------------------------------------"

# Flags accepted by claude-pilot's `_build_parser` (src/claude_pilot/cli.py).
# Pass B below is what keeps this list from drifting into a lie.
CP_ACCEPTED_FLAGS="--task-id --no-relay --relay-config --cwd --log-dir --command --verbose -i --interactive --max-turns --max-budget --stall-threshold --empty-threshold --idle-timeout --min-detection-turns --no-guardrails -h --help"

# A flag is anchored on any non-word character, NOT just whitespace: dispatch-lib
# writes `CWD_ARGS="--cwd $DIR"` and the dead flag was `TRACE_FLAG="--trace"`, both
# glued to `="`. A whitespace-only anchor silently missed both — caught by running
# the guard against a deliberately reintroduced --trace, not by reading it.
#
# Short flags count too: `-X` aborts argparse exactly like `--trace`. They are
# only safe to extract AFTER quoted segments are stripped (see CP_UNQUOTED
# below) — an earlier version scanned raw text, where `"${LOG_ID}-revise-$(date
# +%s)"` reads as a flag named `-revise-`.
CP_FLAG_RE='(^|[^A-Za-z0-9_-])--?[A-Za-z][A-Za-z0-9-]*'

# Every capture below ends in `|| true`: the suite runs under `set -euo pipefail`,
# where a grep that matches nothing kills the whole run. Measured — breaking the
# invocation pattern on purpose ended the suite mid-section with no summary and
# no failure. A guard that vanishes when its subject changes shape is worse than
# no guard: the run goes quiet exactly when it should be shouting. The
# site-count assertion below is what turns an empty capture into a failure.

# Join backslash continuations (the revise-pilot invocation spans four lines),
# then drop comment lines (the mika#2043 comment names --trace on purpose, as a
# warning).
CP_JOINED=$(awk '
    { line = line $0 }
    /\\$/ { sub(/\\$/, " ", line); next }
    { print line; line = "" }
' "$DISPATCH_LIB" | grep -v '^[[:space:]]*#' || true)

# Anchor on the launcher, NOT on the shape of the argv. Anchoring on
# `claude-pilot -...` looked equivalent and was not: moving $CWD_ARGS ahead of
# the first literal flag — an unremarkable reordering — dropped the main
# dispatch invocation out of the guard's sight entirely, silently, while all
# three assertions stayed green (measured). Every real launch goes through
# `_run_pilot_sandboxed`, plus the venv smoke test; that is the chokepoint.
#
# mika#2165 widened the anchor from start-of-line to start-of-COMMAND (`^` or a
# preceding `;`). The launcher is still the anchor; what changed is that it no
# longer has to be the first command on its line. This was not a preference:
# mika#2165 co-locates `_pilot_log_dir;` with the reads of $_PILOT_LOG_DIR, and
# the launch lines are two of those reads. Under the old anchor both real launch
# sites dropped out of sight at once — the site count fell 3 → 1 and said so,
# which is precisely the "renamed, reordered, or added launch point turns red
# instead of evaporating" property below doing its job. Widening here strictly
# increases what the guard can see; it never lets a launch line hide.
CP_INVOCATIONS=$(printf '%s\n' "$CP_JOINED" \
    | grep -E '(^|;)[[:space:]]*(_run_pilot_sandboxed[[:space:]]+claude-pilot|(if ! )?timeout[[:space:]]+[0-9]+[[:space:]]+claude-pilot)([[:space:]]|$)' || true)

# Quoted segments carry payload, not argv words: `ENTRY_COMMAND="/mika"` and the
# prompt text cannot word-split into flags. Scanning them made the guard accuse
# claude-pilot of `--no-verify` when the inner slash-command grew an argument
# (measured). Only unquoted interpolation can inject a flag, so strip quotes
# before looking for either flags or variables.
CP_UNQUOTED=$(printf '%s\n' "$CP_INVOCATIONS" | sed -E 's/"[^"]*"//g; s/'"'"'[^'"'"']*'"'"'//g' || true)

# Literal flags on those lines.
CP_LITERAL_FLAGS=$(printf '%s\n' "$CP_UNQUOTED" | grep -oE -- "$CP_FLAG_RE" | sed -E 's/^[^-]*//' | sort -u || true)

# Flags reaching the CLI through an interpolated variable. This half is not
# optional: $TRACE_FLAG is exactly how the dead flag got in, and a guard reading
# only literals would have missed it. The assignment pattern is anchored on a
# word boundary rather than start-of-line, so `export`/`declare`/`readonly`, a
# mid-line `[ cond ] && VAR=--flag`, and `VAR+=` are all covered — the one-line
# conditional being the most natural rewrite of the five lines just deleted, and
# invisible to a start-of-line anchor (measured).
CP_VARS=$(printf '%s\n' "$CP_UNQUOTED" | grep -oE '\$\{?[A-Za-z_][A-Za-z0-9_]*\}?' | tr -d '${}' | sort -u || true)
CP_VAR_FLAGS=""
for v in $CP_VARS; do
    _assigns=$(grep -E "(^|[^A-Za-z0-9_-])${v}\+?=" "$DISPATCH_LIB" || true)
    _found=$(printf '%s\n' "$_assigns" | grep -oE -- "$CP_FLAG_RE" | sed -E 's/^[^-]*//' || true)
    [ -n "$_found" ] && CP_VAR_FLAGS="$CP_VAR_FLAGS $_found"
done
CP_VAR_FLAGS=$(printf '%s\n' $CP_VAR_FLAGS | sort -u || true)

# `--` is the prompt separator, not a flag.
CP_BUILT_FLAGS=$(printf '%s\n%s\n' "$CP_LITERAL_FLAGS" "$CP_VAR_FLAGS" | grep -vE '^(--)?$' | sort -u || true)

# Known limit, stated rather than coded around: one level of indirection
# (`A="--trace"; B="$A"`) and a composed flag (`F="--$name"`) both slip through.
# Closing them would mean evaluating shell, which changes what this guard is.

# Count SITES, not flags. A flag floor cannot notice a lost invocation — the
# surviving one carries seven flags on its own, so the total never drops
# (measured). Pinning the site count means a renamed, reordered, or added
# launch point turns red instead of evaporating.
#
# Count moved 3 → 4 on 2026-09-21 (mika#2306): the Fire-Disposition retry in
# `_fd_retry_if_section_still_missing` is a fourth launch point, deliberately a
# direct invocation rather than a recursion into `_launch_revise_pilot` — the
# recursion would make the retry's own findings-file the predicate's input and
# render its first term true by construction.
#
# Count moved 4 → 5 on 2026-10-02 (mika#2636): the plan-size retry in
# `_plan_size_retry_if_section_still_missing` is a fifth launch point, a
# structural copy of the fourth and a direct invocation for the same reason —
# a recursion into `_launch_revise_pilot` would make the retry's own
# findings-file the predicate's input and render its first term true by
# construction.
CP_SITE_COUNT=$(printf '%s\n' "$CP_INVOCATIONS" | grep -c . || true)
if [ "$CP_SITE_COUNT" -eq 5 ]; then
    PASS=$((PASS + 1))
    echo "  ✓ all 5 claude-pilot launch sites are in the guard's sight"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ expected 5 claude-pilot launch sites, saw $CP_SITE_COUNT"
    echo "    A site the guard cannot see is a site it cannot police. If a launch"
    echo "    point was legitimately added or removed, update this count."
fi

# --- Pass A: every built flag is accepted (hermetic) ---
CP_UNKNOWN=""
for f in $CP_BUILT_FLAGS; do
    case " $CP_ACCEPTED_FLAGS " in
        *" $f "*) ;;
        *) CP_UNKNOWN="$CP_UNKNOWN $f" ;;
    esac
done
if [ -z "$CP_UNKNOWN" ]; then
    PASS=$((PASS + 1))
    echo "  ✓ every flag dispatch-lib builds is accepted by claude-pilot"
else
    FAIL=$((FAIL + 1))
    echo "  ✗ dispatch-lib builds flag(s) claude-pilot does not accept:$CP_UNKNOWN"
    echo "    claude-pilot exits 2 on an unrecognized argument, before the session starts."
    echo "    Either the flag is wrong, or claude-pilot gained it and this list is stale."
fi

# --- Pass B: the accepted list is not stale (needs the binary) ---
if command -v claude-pilot >/dev/null 2>&1 && CP_HELP=$(timeout 20 claude-pilot --help 2>&1); then
    CP_MISSING=""
    for f in $CP_ACCEPTED_FLAGS; do
        grep -qE -- "(^|[^A-Za-z0-9-])${f}([^A-Za-z0-9-]|$)" <<<"$CP_HELP" || CP_MISSING="$CP_MISSING $f"
    done
    if [ -z "$CP_MISSING" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ the accepted-flag list matches the installed claude-pilot --help"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ accepted-flag list is stale — not in claude-pilot --help:$CP_MISSING"
        echo "    Pass A is now asserting against a list the CLI does not back."
    fi
else
    echo "  - skipped: claude-pilot not runnable here, cannot confirm the accepted-flag list"
    echo "    (pass A still ran; only the freshness check of the list is skipped)"
fi

# ============================================================================
# mika#2121 (U1): the callback always names its PR state
# ============================================================================

echo ""
echo "Test: NO_PR total-output contract (mika#2121 U1)"
echo "------------------------------------------------"

# The lib was sourced at the iterate-loop section above, so the helpers are
# callable here. _classify_no_pr_reason is pure; _set_pr_status_line mutates the
# global RESULT, so every call below runs inside a subshell that owns its own
# RESULT — the mutation never escapes the assertion.

# --- _classify_no_pr_reason: one token per state, unset diagnosed first ---
assert_eq "reason: repo unset (before exit code)" "repo_unset" \
    "$(_classify_no_pr_reason '' 'some-branch' 1)"
assert_eq "reason: branch unset (before exit code)" "branch_unset" \
    "$(_classify_no_pr_reason 'mika' '' 1)"
assert_eq "reason: gh query failed" "gh_query_failed" \
    "$(_classify_no_pr_reason 'mika' 'some-branch' 1)"
assert_eq "reason: no pr on branch (clean query, empty result)" "no_pr_on_branch" \
    "$(_classify_no_pr_reason 'mika' 'some-branch' 0)"

# --- _set_pr_status_line: exactly one PR-status line, never two ---
# The site-2-then-site-3 sequence: main path finds no PR (writes NO_PR), the
# rescue then opens one (writes PR). The delivered callback must carry ONLY the
# later, truer line.
assert_eq "totality: NO_PR then PR keeps only PR" "PR: https://github.com/x/pull/1" \
    "$(RESULT='some prefix line'; _set_pr_status_line 'NO_PR: no_pr_on_branch'; _set_pr_status_line 'PR: https://github.com/x/pull/1'; printf '%s' "$RESULT" | grep -E '^(PR|NO_PR): ' || true)"
assert_eq "totality: exactly one PR-status line after two emissions" "1" \
    "$(RESULT='some prefix line'; _set_pr_status_line 'NO_PR: no_pr_on_branch'; _set_pr_status_line 'PR: https://github.com/x/pull/1'; printf '%s' "$RESULT" | grep -cE '^(PR|NO_PR): ' || true)"
# The reverse (rescue-fail supersedes site-2 no_pr_on_branch with the specific
# rescue_pr_create_failed) must also leave exactly one line.
assert_eq "totality: NO_PR then more-specific NO_PR keeps only the latter" "NO_PR: rescue_pr_create_failed" \
    "$(RESULT='x'; _set_pr_status_line 'NO_PR: no_pr_on_branch'; _set_pr_status_line 'NO_PR: rescue_pr_create_failed'; printf '%s' "$RESULT" | grep -E '^(PR|NO_PR): ' || true)"
# A non-status prefix line survives untouched (the strip is anchored, not greedy).
assert_contains "totality: strip does not eat non-status lines" "Outcome: PR_OPENED" \
    "$(RESULT='Outcome: PR_OPENED'; _set_pr_status_line 'PR: https://github.com/x/pull/1'; printf '%s' "$RESULT")"

# --- Structural guard: no emission site can go silent (jamais zéro) ---
# Each of the three sites must be able to emit a NO_PR line. If a future edit
# drops an `else`, the corresponding assert turns red instead of the site
# silently reverting to the pre-mika#2121 absence-means-no-PR behaviour that
# produced 306 mute failures.
TRAP_SRC=$(sed -n '/^_dispatch_lib_exit_trap() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "site 1 (crash trap) can emit NO_PR" 'NO_PR:' "$TRAP_SRC"

PFR_SRC=$(sed -n '/^_post_flight_recovery() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "site 2 (main path) can emit NO_PR" 'NO_PR:' "$PFR_SRC"

DCP_SRC=$(sed -n '/^dispatch_claude_pilot() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "site 3 (rescue) can emit NO_PR: rescue_pr_create_failed" 'rescue_pr_create_failed' "$DCP_SRC"

# === mika#2178 — the ticket context reaches the pilot (T1–T9) ===
#
# What these nine detectors measure: before mika#2178, `PROMPT` was exactly
# `<repo>#<num>`, and claude-pilot builds its opening prompt by plain
# concatenation, `f"{ns.command} {opening}"` (claude_pilot/cli.py:290
# interactive, :251 headless). On the plan-callout path the input became
# `/ce-work docs/plans/x.md mika#N`: neither the ticket body nor its comments
# reached the implementer. The seven readers of `ISSUE_BODY` are all internal to
# dispatch-lib and none of them writes into the prompt.
#
# Hermeticity (T9): no fixture calls `gh`. Everything is literal JSON handed to
# the helper on stdin — the suite runs on a runner with no network and no token.

echo ""
echo "Test: mika#2178 — ticket context in the pilot's opening prompt (T1–T9)"
echo "-----------------------------------------------------------------------"

# --- Section-local tooling ---------------------------------------------------

# Reproduces claude-pilot's opening-prompt assembly
# (`opening_prompt = f"{ns.command} {opening}"`). T1/T2 assert on THIS STRING —
# never on ISSUE_JSON nor on any dispatch-lib internal variable. That is the
# definition of AC3 (anti-vacuity).
_t2178_pilot_input() {
    printf '%s %s' "$1" "$2"
}

# Replays dispatch-lib's injection site. T3 pins the literal source line this
# mirror reproduces, so the mirror cannot drift silently.
_t2178_inject() {
    local repo="$1" issue_num="$2" issue_json="$3"
    local prompt ticket_context
    prompt="${repo}#${issue_num}"
    ticket_context=$(printf '%s' "$issue_json" | _render_ticket_context "$repo" "$issue_num" 2>/dev/null || true)
    if [ -n "$ticket_context" ]; then
        prompt=$(printf '%s\n\n%s' "$prompt" "$ticket_context")
    fi
    printf '%s' "$prompt"
}

_t2178_render() {
    printf '%s' "$1" | _render_ticket_context "${2:-mika}" "${3:-2178}" 2>/dev/null || true
}

_t2178_line_of() {
    grep -nF -- "$1" "$DISPATCH_LIB" 2>/dev/null | head -1 | cut -d: -f1
}

# "yes" when $1 appears further down the source than $2, "no" otherwise —
# including when either is absent: a position invariant over a site that does
# not exist is not satisfied.
_t2178_after() {
    local a b
    a=$(_t2178_line_of "$1")
    b=$(_t2178_line_of "$2")
    if [ -n "$a" ] && [ -n "$b" ] && [ "$a" -gt "$b" ]; then printf 'yes'; else printf 'no'; fi
}

_t2178_lineno_in() {
    printf '%s\n' "$2" | grep -nF -- "$1" | head -1 | cut -d: -f1
}

_t2178_assert_le() {
    local label="$1" limit="$2" actual="$3"
    if [ -n "$actual" ] && [ "$actual" -le "$limit" ] 2>/dev/null; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: <= $limit"
        echo "    actual:   $actual"
    fi
}

T2178_SUW_SRC=$(sed -n '/^_set_up_worktree() {/,/^}/p' "$DISPATCH_LIB")
T2178_HELPER_SRC=$(sed -n '/^_render_ticket_context() {/,/^}/p' "$DISPATCH_LIB")

# --- T1: anti-vacuity, against the pilot's real input (AC3) ------------------

T2178_BODY_PHRASE="CONSIGNE-CORPS-2178 le plan se commite depuis un fichier deja present"
T2178_COMMENT_PHRASE="CONSIGNE-COMMENTAIRE-2178 aucune chaine cat redirigee vers un fichier temporaire"

# Excerpt from the mika#2158 grooming-enrichment comment of 2026-09-04T09:43:28Z.
# RECONSTRUCTED, not literal: the session that wrote these tests had no `gh`
# access (no token, no `gh auth`), and plan §1.3 is the only available source on
# that comment's content. The measured-vs-reconstructed distinction is the one
# crates/mika-agent/tests/fixtures/grooming_bodies/README.md already carries.
# What is load-bearing for AC3 is that one instruction lives in a comment and
# another in the body — not the literalness of the excerpt.
T2178_C2158_EXCERPT="Enrichissement de grooming (reconstruit) : la forme refusee par le
classifieur de permissions est la chaine \`cat > /tmp/plan.md <<EOF\`. Ecris le
plan avec l'outil d'ecriture de fichier, puis commite-le."

T2178_FIXTURE=$(jq -n \
    --arg body "Contexte du ticket.

${T2178_BODY_PHRASE}

> - **Plan:** \`docs/plans/x.md\` (committed on branch @ deadbee)" \
    --arg c1 "Premier commentaire, sans consigne particuliere." \
    --arg c2 "${T2178_COMMENT_PHRASE}

${T2178_C2158_EXCERPT}" \
    '{state:"OPEN", title:"le contexte du ticket atteint le pilote",
      labels:[{name:"ready"}], body:$body,
      comments:[
        {author:{login:"mika-platform-dev"}, createdAt:"2026-09-04T08:12:00Z", body:$c1},
        {author:{login:"samidarko"},         createdAt:"2026-09-04T09:43:28Z", body:$c2}
      ]}')

T2178_PROMPT=$(_t2178_inject "mika" "2178" "$T2178_FIXTURE")
T2178_INPUT_A=$(_t2178_pilot_input "/ce-work docs/plans/x.md" "$T2178_PROMPT")

assert_contains "T1: the BODY instruction reaches the pilot's real input (/ce-work)" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_A"
assert_contains "T1: the COMMENT instruction reaches the pilot's real input (/ce-work)" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_A"

# Red-before, term by term: neutralising ONE term does not pin the others, so
# three separate negative controls.

# (a) `--json` without `comments` — the comment phrase disappears, the body
#     phrase stays. This is what separates "we widened the fetch" from "we
#     widened it AND forwarded it".
T2178_FIXTURE_NO_COMMENTS=$(printf '%s' "$T2178_FIXTURE" | jq 'del(.comments)')
T2178_INPUT_NC=$(_t2178_pilot_input "/ce-work docs/plans/x.md" \
    "$(_t2178_inject "mika" "2178" "$T2178_FIXTURE_NO_COMMENTS")")
assert_not_contains "T1 negative control (a): without the comments field, the comment instruction is gone" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_NC"
assert_contains "T1 negative control (a): without the comments field, the body instruction remains" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_NC"

# (b) helper neutralised (renders the empty string) — both phrases disappear.
#     The neutralisation is real: the helper is shadowed by a subshell-local
#     redefinition, not simulated with a hand-written string. That proves the
#     append is fed by the helper and by nothing else.
T2178_INPUT_NOHELPER=$(
    _render_ticket_context() { :; }
    _t2178_pilot_input "/ce-work docs/plans/x.md" \
        "$(_t2178_inject "mika" "2178" "$T2178_FIXTURE")"
)
assert_not_contains "T1 negative control (b): helper neutralised, the body instruction is gone" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_NOHELPER"
assert_not_contains "T1 negative control (b): helper neutralised, the comment instruction is gone" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_NOHELPER"

# (c) helper intact but the append removed — the helper does render text, yet
#     the pilot's input stays `/ce-work docs/plans/x.md mika#2178`.
T2178_RENDER_ONLY=$(_t2178_render "$T2178_FIXTURE")
T2178_INPUT_NOAPPEND=$(_t2178_pilot_input "/ce-work docs/plans/x.md" "mika#2178")
assert_eq "T1 negative control (c): without the append, the pilot input is exactly the old one" \
    "/ce-work docs/plans/x.md mika#2178" "$T2178_INPUT_NOAPPEND"
assert_not_contains "T1 negative control (c): without the append, the body instruction never reaches the pilot" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_NOAPPEND"
assert_not_contains "T1 negative control (c): without the append, the comment instruction never reaches the pilot" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_NOAPPEND"
assert_contains "T1 negative control (c): the helper itself does render the comment instruction" \
    "$T2178_COMMENT_PHRASE" "$T2178_RENDER_ONLY"

# --- T2: symmetry of the two paths (AC1, AC2) --------------------------------

T2178_INPUT_B=$(_t2178_pilot_input "/mika" "$T2178_PROMPT")
assert_contains "T2: the BODY instruction also reaches the no-callout path (/mika)" \
    "$T2178_BODY_PHRASE" "$T2178_INPUT_B"
assert_contains "T2: the COMMENT instruction also reaches the no-callout path (/mika)" \
    "$T2178_COMMENT_PHRASE" "$T2178_INPUT_B"

# The injection is unconditional: at the point where it happens the entry
# command is not arbitrated yet (_detect_plan_on_branch runs after). A single
# occurrence IN CODE would reintroduce the asymmetry this ticket closes.
#
# The count excludes comment lines, like this suite's pre-existing structural
# guards (NON_COMMENT_EXIT1, NON_COMMENT_RETURNS): the comment explaining WHY
# the site cannot branch on the entry command is precisely what stops a future
# editor from branching there. An assertion demanding its silence would remove
# the reason to keep the rule.
assert_eq "T2: unconditional injection — no code-level branch on ENTRY_COMMAND in _set_up_worktree" \
    "0" "$(printf '%s\n' "$T2178_SUW_SRC" | grep -v '^[[:space:]]*#' | grep -c 'ENTRY_COMMAND' || true)"

# --- T3: position invariants (plan §3.3) -------------------------------------

T2178_APPEND_EXPR='PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$TICKET_CONTEXT")'

# Invariant 1 — after the ITERATION_CTX branch, which REASSIGNS PROMPT from
# scratch instead of appending: injecting before it would overwrite the context
# silently whenever an iteration is in flight.
assert_eq "T3 invariant 1: the injection sits AFTER the ITERATION CONTEXT reassignment" "yes" \
    "$(_t2178_after 'PROMPT" "$TICKET_CONTEXT")' 'ITERATION CONTEXT:')"

# Invariant 2 — after the anchored repo#N parse
# (`^([a-zA-Z0-9_-]+/)?[a-zA-Z0-9_-]+#[0-9]+$`): injecting before it makes the
# regex miss, dispatch falls into free-text mode and NO worktree is created.
assert_eq "T3 invariant 2: the injection sits AFTER the anchored repo#N parse" "yes" \
    "$(_t2178_after 'PROMPT" "$TICKET_CONTEXT")' '[a-zA-Z0-9_-]+#[0-9]+')"

# Invariant 3 — the injection lives INSIDE _set_up_worktree, hence before
# _detect_plan_on_branch and _handle_dry_run. The call order itself
# (_set_up_worktree → _detect_plan_on_branch → _handle_dry_run) is already
# asserted earlier in this suite ("Call ordering: ..."); T3 references that
# rather than duplicating it, and adds only the missing half here: the injection
# site is indeed inside the first of the three.
assert_contains "T3 invariant 3: the injection site lives inside _set_up_worktree" \
    '_render_ticket_context' "$T2178_SUW_SRC"

# T1's mirror is only legitimate if the append expression it replays is
# literally the one in the source.
assert_contains "T3: the source append expression is the one T1 replays" \
    "$T2178_APPEND_EXPR" "$(cat "$DISPATCH_LIB")"

# mika#138 contract + the anchored parse in _set_up_worktree: the FIRST LINE of
# PROMPT stays exactly `<repo>#<num>`. Strict equality, not assert_contains.
assert_eq "T3: the first line of PROMPT stays exactly mika#2178" "mika#2178" \
    "$(printf '%s' "$T2178_PROMPT" | head -1)"

# --- T4: readable rendering (AC1) --------------------------------------------

T2178_FX_ROLES=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"corps du ticket de reference",
  comments:[
    {author:{login:"mika-platform-dev"}, createdAt:"2026-09-04T08:00:00Z",
     body:"Verdict QA: block[pipeline] — la CI est rouge sur le job clippy."},
    {author:{login:"samidarko"}, createdAt:"2026-09-04T09:43:28Z",
     body:"Consigne operateur: refais le plan avant de reimplementer."}
  ]}')
T2178_RENDER_ROLES=$(_t2178_render "$T2178_FX_ROLES")

assert_contains "T4: bot comment header — login, bot role, ISO timestamp" \
    "--- commentaire 1/2 · mika-platform-dev (bot) · 2026-09-04T08:00:00Z ---" "$T2178_RENDER_ROLES"
assert_contains "T4: human comment header — login, human role, ISO timestamp" \
    "--- commentaire 2/2 · samidarko (humain) · 2026-09-04T09:43:28Z ---" "$T2178_RENDER_ROLES"
assert_contains "T4: the ticket body sits under its own separator" \
    "--- corps du ticket ---" "$T2178_RENDER_ROLES"
assert_contains "T4: the header says what to do with the block, not just that it exists" \
    "Lis les deux avant d'agir." "$T2178_RENDER_ROLES"
assert_contains "T4: the bot's QA verdict is forwarded verbatim" \
    "block[pipeline]" "$T2178_RENDER_ROLES"

# --- T5: bounds --------------------------------------------------------------

# (a) 15 SHORT comments (≤ 200 B each) so that only the COUNT cap bites, never
#     the block cap. Without that size constraint on the fixture, a correct
#     implementation could render fewer than 10 (block-cap eviction) and the
#     test would go red for the wrong reason.
T2178_FX_15=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"corps court",
  comments:[range(1;16) | {author:{login:"samidarko"},
                           createdAt:"2026-09-04T00:00:00Z",
                           body:"COMMENTAIRE-NUMERO-\(.)"}]}')
T2178_RENDER_15=$(_t2178_render "$T2178_FX_15")

assert_eq "T5(a): exactly 10 comments rendered out of 15" "10" \
    "$(printf '%s\n' "$T2178_RENDER_15" | grep -c '^--- commentaire ' || true)"
assert_contains "T5(a): the omission line carries the count 5" \
    "[5 commentaire(s) plus ancien(s) omis]" "$T2178_RENDER_15"
assert_contains "T5(a): the oldest rendered is 6/15" \
    "--- commentaire 6/15 · " "$T2178_RENDER_15"
assert_contains "T5(a): the most recent rendered is 15/15" \
    "--- commentaire 15/15 · " "$T2178_RENDER_15"
assert_not_contains "T5(a): 5/15 is indeed evicted" \
    "--- commentaire 5/15 · " "$T2178_RENDER_15"
assert_eq "T5(a): ascending chronological order (6/15 before 15/15)" "yes" \
    "$(a=$(_t2178_lineno_in '--- commentaire 6/15 · ' "$T2178_RENDER_15"); \
       b=$(_t2178_lineno_in '--- commentaire 15/15 · ' "$T2178_RENDER_15"); \
       if [ -n "$a" ] && [ -n "$b" ] && [ "$a" -lt "$b" ]; then printf 'yes'; else printf 'no'; fi)"

# (b) an 8 KiB comment → truncated to 4096 B with a marker.
T2178_FX_8K=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"corps",
  comments:[{author:{login:"samidarko"}, createdAt:"2026-09-04T09:00:00Z", body:("Y" * 8192)}]}')
T2178_RENDER_8K=$(_t2178_render "$T2178_FX_8K")
assert_contains "T5(b): comment truncation marker present" \
    "[… tronqué à 4096 o]" "$T2178_RENDER_8K"
# Uppercase 'Y' appears nowhere else in the rendering (header, markers,
# timestamps), so the 'Y' count IS the length of the retained body.
assert_eq "T5(b): the comment body is cut at 4096 B" "4096" \
    "$(printf '%s' "$T2178_RENDER_8K" | tr -cd 'Y' | wc -c)"

# (c) 10 comments of 4 KiB → block ≤ 16384 B AND an omission line present.
T2178_FX_10x4K=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"corps",
  comments:[range(1;11) | {author:{login:"samidarko"},
                           createdAt:"2026-09-04T00:00:00Z",
                           body:("W" * 4096)}]}')
T2178_RENDER_10x4K=$(_t2178_render "$T2178_FX_10x4K")
T2178_BLOCK_10x4K=$(printf '%s\n' "$T2178_RENDER_10x4K" | sed -n '/^--- commentaire /,$p')
_t2178_assert_le "T5(c): the comment block stays under 16384 B" "16384" \
    "$(printf '%s' "$T2178_BLOCK_10x4K" | wc -c)"
assert_eq "T5(c): omission line present when the block cap bites" "1" \
    "$(printf '%s\n' "$T2178_RENDER_10x4K" | grep -cE '^\[[0-9]+ commentaire\(s\) plus ancien\(s\) omis\]$' || true)"

# (d) a 32 KiB body → truncated to 16384 B with a marker.
T2178_FX_32K=$(jq -n '{state:"OPEN", title:"t", labels:[], body:("Q" * 32768), comments:[]}')
T2178_RENDER_32K=$(_t2178_render "$T2178_FX_32K")
assert_contains "T5(d): body truncation marker present" \
    "[… corps tronqué à 16384 o]" "$T2178_RENDER_32K"
assert_eq "T5(d): the body is cut at 16384 B" "16384" \
    "$(printf '%s' "$T2178_RENDER_32K" | tr -cd 'Q' | wc -c)"

# (e) payload beyond the pipe buffer (64 KiB on Linux). This is not a variant of
#     (b)/(d): it is the only case that separates a truncation written as
#     `printf … | head -c N` from one written without a pipe. With the pipe,
#     head exits after N bytes while printf is still writing, printf dies of
#     SIGPIPE, and under dispatch-lib's `set -euo pipefail` the substitution
#     fails and the whole dispatch aborts — the helper would render empty.
#     A GitHub issue body caps at 65 536 CHARACTERS, hence well past 64 KiB in
#     UTF-8: reachable on a real maximal ticket.
T2178_FX_HUGE=$(jq -n '{state:"OPEN", title:"t", labels:[], body:("Q" * 204800),
  comments:[{author:{login:"samidarko"}, createdAt:"2026-09-04T09:00:00Z", body:("Y" * 204800)}]}')
T2178_RENDER_HUGE=$(_t2178_render "$T2178_FX_HUGE")
assert_contains "T5(e): a payload > pipe buffer does not abort the helper (body)" \
    "[… corps tronqué à 16384 o]" "$T2178_RENDER_HUGE"
assert_contains "T5(e): a payload > pipe buffer does not abort the helper (comment)" \
    "[… tronqué à 4096 o]" "$T2178_RENDER_HUGE"
assert_eq "T5(e): the 200 KiB body is cut at 16384 B" "16384" \
    "$(printf '%s' "$T2178_RENDER_HUGE" | tr -cd 'Q' | wc -c)"
assert_eq "T5(e): the 200 KiB comment is cut at 4096 B" "4096" \
    "$(printf '%s' "$T2178_RENDER_HUGE" | tr -cd 'Y' | wc -c)"

# --- T6: degenerate inputs ---------------------------------------------------

T2178_FX_NO_COMMENTS=$(jq -n '{state:"OPEN", title:"t", labels:[],
  body:"CORPS-SEUL-DISTINCTIF-2178", comments:[]}')
T2178_RENDER_NO_COMMENTS=$(_t2178_render "$T2178_FX_NO_COMMENTS")
assert_not_contains "T6(a): no comment separator when comments is empty" \
    "--- commentaire " "$T2178_RENDER_NO_COMMENTS"
assert_not_contains "T6(a): no omission line when comments is empty" \
    "plus ancien(s) omis" "$T2178_RENDER_NO_COMMENTS"
assert_contains "T6(a): the body alone is present" \
    "CORPS-SEUL-DISTINCTIF-2178" "$T2178_RENDER_NO_COMMENTS"

T2178_FX_EMPTY=$(jq -n '{state:"OPEN", title:"t", labels:[], body:"", comments:[]}')
assert_eq "T6(b): with no body and no comment, the helper renders the empty string" "" \
    "$(_t2178_render "$T2178_FX_EMPTY")"
assert_eq "T6(b): with no body and no comment, PROMPT is STRICTLY equal to mika#2178" "mika#2178" \
    "$(_t2178_inject "mika" "2178" "$T2178_FX_EMPTY")"

# --- T7: non-regression of the seven ISSUE_BODY readers (AC4) ----------------
#
# INVARIANCE detector: it must be GREEN on unmodified main. If it goes red
# before any fix, the reading of the code the mika#2178 plan rests on is wrong —
# halt-and-surface, no allowlist (Fire-Disposition, option (c)).

assert_eq "T7: exactly 7 ISSUE_BODY readers (AC4 freezes them)" "7" \
    "$(grep -c 'ISSUE_BODY' "$DISPATCH_LIB" || true)"

T2178_ISSUE_BODY_LINES=$(grep -h 'ISSUE_BODY' "$DISPATCH_LIB" | sed 's/^[[:space:]]*//')
assert_contains "T7: reader 1 — ISSUE_BODY derived from ISSUE_JSON" \
    "ISSUE_BODY=\$(printf '%s' \"\$ISSUE_JSON\" | jq -r '.body // empty')" "$T2178_ISSUE_BODY_LINES"
assert_contains "T7: reader 2 — derive-branch-name --body-callout" \
    '--body-callout "$ISSUE_BODY")' "$T2178_ISSUE_BODY_LINES"
assert_contains "T7: reader 3 — anti-re-groom gate _committed_plan_on_branch (mika#2012)" \
    'if existing_plan=$(_committed_plan_on_branch "$SUB_REPO_DIR" "$BRANCH" "$ISSUE_BODY" "$REPO" "$ISSUE_NUM"); then' \
    "$T2178_ISSUE_BODY_LINES"
# Reader 4 a changé de forme avec mika#2608, et le lecteur est resté unique.
#
# Il portait `grep -qE -- '^> - \*\*Plan:\*\*'`, c'est-à-dire le second des deux
# lecteurs bash que la phase 1 de mika#2194 avait DÉLIBÉRÉMENT laissés en place.
# La phase 2 le fait déléguer : la distinction 0/1 de `mika plan-callout` EST la
# réponse booléenne que ce site cherchait, donc aucune sous-commande ni drapeau
# n'a été ajouté. Le site ne décide toujours rien — il journalise
# `dispatch_gate_groom_allowed_stale_callout` — et sa place dans la chaîne est
# inchangée (mika#2012 U4).
#
# Ce détecteur est un détecteur d'INVARIANCE : il doit rougir quand la forme d'un
# lecteur bouge, et c'est exactement ce qu'il a fait ici. L'attendu est donc mis à
# jour avec sa raison, jamais relâché en sous-chaîne.
assert_contains "T7: reader 4 — stale callout, délégué depuis mika#2608" \
    'elif _extract_plan_path "$ISSUE_BODY" >/dev/null 2>&1; then' \
    "$T2178_ISSUE_BODY_LINES"
assert_contains "T7: reader 5 — _detect_plan_on_branch guard" \
    '[ -n "$ISSUE_BODY" ] || return 0' "$T2178_ISSUE_BODY_LINES"
# Reader 6 a changé de forme deux fois, et le lecteur est resté unique.
#
# mika#2120 : le motif a quitté `_detect_plan_on_branch` pour
# `_extract_plan_path`, qui accepte les deux écritures du callout et normalise.
#
# mika#2194 : `_extract_plan_path` ne porte plus de motif du tout — elle délègue
# au lecteur Rust — et son code de retour est désormais CAPTURÉ au lieu d'être
# écrasé par `|| return 0`, parce qu'il porte trois valeurs et non deux
# (0 lu · 1 aucun callout · ≥2 illisible). C'est le livrable d'AC4 : confondre
# les deux dernières fabriquerait un silence que la migration vient de créer.
#
# Ce que T7 fige reste vrai et inchangé : il y a UN lecteur, et il lit
# `$ISSUE_BODY`.
assert_contains "T7: reader 6 — plan-path extraction" \
    'PLAN_PATH=$(_extract_plan_path "$ISSUE_BODY") || _plan_rc=$?' \
    "$T2178_ISSUE_BODY_LINES"
assert_contains "T7: reader 7 — plan-line rescue in _iterate_groom_loop" \
    'elif _groom_plan_path=$(_committed_plan_on_branch "$SUB_REPO_DIR" "$BRANCH" "$ISSUE_BODY" "$REPO" "$ISSUE_NUM" 2>/dev/null); then' \
    "$T2178_ISSUE_BODY_LINES"

# The helper reads ISSUE_JSON, never ISSUE_BODY — a shared access would couple
# the two surfaces and turn any change to the rendering into a change to the
# seven readers.
assert_not_contains "T7: _render_ticket_context never reads ISSUE_BODY" \
    "ISSUE_BODY" "$T2178_HELPER_SRC"

# --- T8: structure -----------------------------------------------------------

assert_eq "T8: --json state,title,labels,body,comments present at exactly ONE site" "1" \
    "$(grep -cF -- '--json state,title,labels,body,comments' "$DISPATCH_LIB" || true)"
assert_eq "T8: _render_ticket_context() defined exactly once" "1" \
    "$(grep -c '^_render_ticket_context() {' "$DISPATCH_LIB" || true)"
assert_contains "T8: the injection reads ISSUE_JSON (no second network round trip)" \
    'ISSUE_JSON" | _render_ticket_context' "$T2178_SUW_SRC"
# A second `gh issue view` would open a TOCTOU window between the state read by
# the issue-close gate and the comments — the same one the /mika-groom-ticket
# spec closed at its step 5a.
assert_eq "T8: exactly one gh issue view call in _set_up_worktree" "1" \
    "$(printf '%s\n' "$T2178_SUW_SRC" | grep -c 'gh issue view' || true)"

# --- T9: hermeticity ---------------------------------------------------------
#
# INVARIANCE detector: green on unmodified main. No test in this section calls
# `gh` — the fixtures are literal JSON handed to the helper on stdin, so the
# suite runs on a runner with no network and no token.

T2178_SECTION=$(sed -n '/^# === mika#2178 — the ticket context reaches the pilot/,$p' "${BASH_SOURCE[0]}")
# Guard the guard: if the section marker above ever drifts from the real header,
# the sed returns nothing and the two greps below pass on an empty haystack — a
# silent false green on the very check that certifies hermeticity.
assert_eq "T9: the section extraction actually found the section (guards the guard)" "yes" \
    "$(if [ -n "$T2178_SECTION" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "T9: no gh invocation at command head in the mika#2178 section" "0" \
    "$(printf '%s\n' "$T2178_SECTION" | grep -cE '^[[:space:]]*gh[[:space:]]' || true)"
assert_eq "T9: no gh invocation in a substitution in the mika#2178 section" "0" \
    "$(printf '%s\n' "$T2178_SECTION" | grep -cE '\$\(gh[[:space:]]' || true)"
assert_not_contains "T9: _render_ticket_context does not call gh (it reads stdin)" \
    "gh issue" "$T2178_HELPER_SRC"

# =============================================================================
# mika#2211 — the PR-body containment rule reaches the pilot
# =============================================================================
#
# Why a structural test and not a behavioural one: the defect is a pilot writing
# `/tmp/pr-body-<N>.md` on its own initiative, which no test in this repo can
# reproduce without launching a model. What CAN be pinned is the thing that was
# missing — that the rule is present, and present on the ONE channel the pilot
# actually reads. A test asserting only that some file somewhere contains the
# sentence would have stayed green through the exact failure this closes:
# mika-dev's own system prompt never reaches the pilot process.

T2211_SUW_SRC=$(sed -n '/^_set_up_worktree() {/,/^}/p' "$DISPATCH_LIB")

# --- The rule exists and is non-empty ---------------------------------------
assert_eq "mika#2211: _PR_BODY_CONTAINMENT_RULE is defined exactly once" "1" \
    "$(grep -c '^_PR_BODY_CONTAINMENT_RULE=' "$DISPATCH_LIB" || true)"

# --- It is injected into the prompt the pilot receives ----------------------
assert_contains "mika#2211: the rule is appended to PROMPT inside _set_up_worktree" \
    '_PR_BODY_CONTAINMENT_RULE' "$T2211_SUW_SRC"

# The same position invariant mika#2178 documents: the ITERATION_CTX branch
# REASSIGNS PROMPT from scratch, so an injection above it is dropped silently on
# every iteration — the failure would then reappear on exactly the reruns that
# follow a first attempt.
assert_eq "mika#2211: the injection sits AFTER the ITERATION CONTEXT reassignment" "yes" \
    "$(_t2178_after 'PROMPT" "$_PR_BODY_CONTAINMENT_RULE")' 'ITERATION CONTEXT:')"

# The mika#138 contract: the first line of PROMPT stays `<repo>#<num>`. The
# anchored parse in _set_up_worktree depends on it, so appending must never
# prepend.
assert_eq "mika#2211: the rule is APPENDED (repo#N stays the first line)" "yes" \
    "$(if printf '%s' "$T2211_SUW_SRC" \
        | grep -qF 'PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$_PR_BODY_CONTAINMENT_RULE")'; \
       then printf 'yes'; else printf 'no'; fi)"

# --- The rule says the three things that make it actionable -----------------
# Stated positively first: a pilot told only "not /tmp" still has to invent a
# replacement, and the one it reaches for next (a heredoc) breaks on a body
# containing its own delimiter.
T2211_RULE=$(sed -n '/^_PR_BODY_CONTAINMENT_RULE=/,/^$/p' "$DISPATCH_LIB")

assert_contains "mika#2211: the rule names the working form (--body-file under the worktree)" \
    '--body-file pr-body.md' "$T2211_RULE"
assert_contains "mika#2211: the rule forbids the measured failure path (/tmp)" \
    '/tmp' "$T2211_RULE"
assert_contains "mika#2211: the rule forbids the heredoc the pilot would reach for next" \
    "heredoc" "$T2211_RULE"
assert_contains "mika#2211: the rule forbids ending the session on a question" \
    "session morte" "$T2211_RULE"

# --- The target repo's own command file carries the canonical form ----------
# AC1. Checked here because this suite already resolves REPO_ROOT and runs on
# every PR; a prompt-only fix with no gate is how the pre-fix inline instruction
# survived unexamined.
T2211_MIKA_CMD="$REPO_ROOT/.claude/commands/mika.md"
if [ -f "$T2211_MIKA_CMD" ]; then
    T2211_CMD_SRC=$(cat "$T2211_MIKA_CMD")
    assert_contains "mika#2211 AC1: mika.md PR step uses --body-file under the worktree" \
        '--body-file pr-body.md' "$T2211_CMD_SRC"
    assert_contains "mika#2211 AC1: mika.md PR step forbids writing the body outside the worktree" \
        'Never write the PR body outside the worktree' "$T2211_CMD_SRC"
    # The residue guard: the ignore is what keeps a failed `gh pr create` from
    # leaving a file that `git status --porcelain` reports, which is precisely
    # the probe that triggers the mika#1282 wip-rescue draft.
    assert_contains "mika#2211: pr-body.md is gitignored (a residue must not read as dirty)" \
        'pr-body.md' "$(cat "$REPO_ROOT/.gitignore" 2>/dev/null || true)"
fi
# =============================================================================
# mika#2548 — le scratch du pilote : désigné, vide, invisible à git
# =============================================================================
#
# Fonctionnel, pas seulement structurel : la propriété qui compte est ce que
# `git status` rend dans un worktree LIÉ, et elle n'est lisible qu'en exécutant
# git. Le worktree est créé sous un chemin qui porte `/.claude/worktrees/`, la
# forme réelle d'un worktree de dispatch — sans quoi la garde mika#1943
# refuserait la remise à zéro et le test mesurerait autre chose que la prod.

_t2548_run() {
    local root base wt out=""
    root=$(mktemp -d)
    base="$root/repo"
    wt="$root/.claude/worktrees/fix-2548-probe/mika"
    mkdir -p "$base" "$(dirname "$wt")"
    git -C "$base" init -q
    git -C "$base" commit --allow-empty -q -m initial
    git -C "$base" worktree add -q -b t2548 "$wt" 2>/dev/null

    # S1 — le répertoire existe après le seed.
    _seed_pilot_scratch_dir "$wt" 2>/dev/null
    [ -d "$wt/.pilot-scratch" ] && out="${out}exists;"

    # S2 — un fixture écrit dedans (non vide, en sous-répertoire) est invisible.
    mkdir -p "$wt/.pilot-scratch/sub" && printf 'x\n' > "$wt/.pilot-scratch/sub/f"
    [ -z "$(git -C "$wt" status --porcelain)" ] && out="${out}scratch_invisible;"

    # S3 — contrôle positif du prédicat : un fichier HORS scratch reste visible,
    # sinon S2 prouverait seulement que `git status` ne rend jamais rien.
    printf 'y\n' > "$wt/visible.txt"
    [ -n "$(git -C "$wt" status --porcelain)" ] && out="${out}outside_visible;"
    rm -f "$wt/visible.txt"

    # S4 — idempotence : un second seed laisse UNE seule ligne d'exclusion.
    _seed_pilot_scratch_dir "$wt" 2>/dev/null
    out="${out}lines=$(grep -cxF '.pilot-scratch/' "$base/.git/info/exclude");"

    # S5 — remise à zéro : le résidu d'une session précédente a disparu, le
    # répertoire est là et vide (réutilisation de worktree, KTD6).
    printf 'stale\n' > "$wt/.pilot-scratch/stale"
    _seed_pilot_scratch_dir "$wt" 2>/dev/null
    if [ -d "$wt/.pilot-scratch" ] && [ -z "$(ls -A "$wt/.pilot-scratch")" ]; then
        out="${out}reset;"
    fi

    # S6 — un exclude préexistant sans saut de ligne final n'est pas concaténé.
    printf 'foo' > "$base/.git/info/exclude"
    _seed_pilot_scratch_dir "$wt" 2>/dev/null
    grep -qxF 'foo' "$base/.git/info/exclude" && grep -qxF '.pilot-scratch/' "$base/.git/info/exclude" \
        && out="${out}newline_guard;"

    # S7 — non-régression mika#1415 après la factorisation : un fichier semé par
    # _seed_worktree_slash_commands reste exclu.
    local plat="$root/platform"
    mkdir -p "$plat/.claude/commands"
    printf 'meta\n' > "$plat/.claude/commands/mika-groom-ticket.md"
    _seed_worktree_slash_commands "$plat" "$wt"
    [ -f "$wt/.claude/commands/mika-groom-ticket.md" ] && [ -z "$(git -C "$wt" status --porcelain)" ] \
        && out="${out}cmds_seed_ok;"

    git -C "$base" worktree remove --force "$wt" 2>/dev/null
    rm -rf "$root"
    printf '%s' "$out"
}

# Les jetons sont choisis pour qu'aucun ne soit sous-chaîne d'un autre :
# assert_contains est un test de sous-chaîne, et un premier jet (`clean;` contre
# `commands_still_clean;`) passait vert sans l'exclusion — vu au contrôle négatif.
T2548_OUT=$(_t2548_run)
assert_contains "mika#2548: .pilot-scratch existe après le seed" "exists;" "$T2548_OUT"
assert_contains "mika#2548: un fixture sous .pilot-scratch/ est invisible à git status" "scratch_invisible;" "$T2548_OUT"
assert_contains "mika#2548: contrôle positif — un fichier hors scratch reste visible" "outside_visible;" "$T2548_OUT"
assert_contains "mika#2548: deux seeds laissent une seule ligne d'exclusion" "lines=1;" "$T2548_OUT"
assert_contains "mika#2548: un résidu est vidé à la préparation suivante" "reset;" "$T2548_OUT"
assert_contains "mika#2548: un exclude sans saut de ligne final n'est pas concaténé" "newline_guard;" "$T2548_OUT"
assert_contains "mika#2548: le seeding des commandes (mika#1415) reste propre" "cmds_seed_ok;" "$T2548_OUT"

# --- Structure : le seed est câblé dans _set_up_worktree, et sa suppression
# passe par la garde mika#1943 (un rm -rf nu sur un chemin non prouvé est la
# classe que cette garde existe pour fermer).
T2548_SUW_SRC=$(sed -n '/^_set_up_worktree() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "mika#2548: _set_up_worktree appelle _seed_pilot_scratch_dir" \
    '_seed_pilot_scratch_dir "$WORKTREE_DIR"' "$T2548_SUW_SRC"
T2548_SEED_SRC=$(sed -n '/^_seed_pilot_scratch_dir() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "mika#2548: la remise à zéro passe par _assert_removable_worktree_path" \
    '_assert_removable_worktree_path "$scratch" seed_pilot_scratch_dir' "$T2548_SEED_SRC"

# --- U2 : la règle atteint le pilote, au bon endroit ------------------------
assert_eq "mika#2548: _PILOT_SCRATCH_RULE est définie exactement une fois" "1" \
    "$(grep -c '^_PILOT_SCRATCH_RULE=' "$DISPATCH_LIB" || true)"
assert_contains "mika#2548: la règle est APPENDUE au PROMPT (repo#N reste la 1re ligne)" \
    'PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$_PILOT_SCRATCH_RULE")' "$T2548_SUW_SRC"
assert_eq "mika#2548: l'injection suit la réaffectation ITERATION CONTEXT" "yes" \
    "$(_t2178_after 'PROMPT" "$_PILOT_SCRATCH_RULE")' 'ITERATION CONTEXT:')"
assert_eq "mika#2548: l'injection suit la règle mika#2211" "yes" \
    "$(_t2178_after 'PROMPT" "$_PILOT_SCRATCH_RULE")' 'PROMPT" "$_PR_BODY_CONTAINMENT_RULE")')"
# La règle Fire-Disposition doit rester la plus récente pour le groomeur.
assert_eq "mika#2548: la règle Fire-Disposition (mika#2306) reste APRÈS la règle scratch" "yes" \
    "$(_t2178_after 'PROMPT" "$_FIRE_DISPOSITION_RULE")' 'PROMPT" "$_PILOT_SCRATCH_RULE")')"
# Inconditionnelle : l'injection n'est pas dans le bloc `if [ "$SKILL" = "dev-groom" ]`.
T2548_GROOM_BLOCK=$(printf '%s\n' "$T2548_SUW_SRC" | sed -n '/if \[ "\$SKILL" = "dev-groom" \]; then/,/^        fi/p')
assert_not_contains "mika#2548: l'injection n'est pas conditionnée au skill" \
    '_PILOT_SCRATCH_RULE' "$T2548_GROOM_BLOCK"

T2548_RULE=$(bash -c 'source "$1" 2>/dev/null; printf "%s" "$_PILOT_SCRATCH_RULE"' _ "$DISPATCH_LIB")
assert_contains "mika#2548: la règle porte son étiquette de ticket" "mika#2548" "$T2548_RULE"
assert_contains "mika#2548: la règle nomme le lieu du scratch" ".pilot-scratch/" "$T2548_RULE"
assert_contains "mika#2548: la règle interdit rm nu" 'pas de `rm`' "$T2548_RULE"
assert_contains "mika#2548: la règle interdit rmdir" '`rmdir`' "$T2548_RULE"
assert_contains "mika#2548: la règle interdit rm -rf" '`rm -rf`' "$T2548_RULE"
assert_contains "mika#2548: la règle dit que le refus est terminal" "TERMINAL" "$T2548_RULE"
assert_contains "mika#2548: la règle écarte /tmp comme lieu de fixture" "/tmp" "$T2548_RULE"
# La commande qui a réellement échoué dans l'incident fondateur (mika#2054) était un
# `git show … -- > … 2>/dev/null; wc -l`, refusé pour sa forme et non pour sa cible :
# la règle nomme la seule forme que la politique autorise (cpp#35, bash-git-show-redirect).
assert_contains "mika#2548: la règle nomme la forme git show autorisée" \
    'git show <ref>:<chemin> > .pilot-scratch/<chemin>' "$T2548_RULE"
# L'interdit porte sur le scratch, pas sur tout fichier : sinon il contredit la
# règle mika#2211 (« puis supprime-le ») et un pr-body.md laissé à la racine est
# commité par le rescue dans les dépôts qui ne l'ignorent pas.
assert_contains "mika#2548: l'interdit de suppression est borné au scratch" \
    'Ne supprime JAMAIS un brouillon de `.pilot-scratch/`' "$T2548_RULE"
assert_contains "mika#2548: pr-body.md (mika#2211) est l'exception nommée" 'pr-body.md' "$T2548_RULE"
assert_not_contains "mika#2548: plus d'interdit général sur tout brouillon" \
    'Ne supprime JAMAIS un brouillon,' "$T2548_RULE"

# ============================================================================
# mika#2120 — le second lecteur du callout `Plan` tolère le préfixe de dépôt
# ============================================================================
#
# Parité avec `auto_pull::extract_plan_path` : les deux écritures du callout
# rendent le **même** chemin normalisé, relatif à la racine du sous-dépôt. La
# divergence entre ces deux lecteurs est la cause racine du ticket, rejouée si
# elle revient — d'où un test, pas un commentaire.

echo ""
echo "Test: _extract_plan_path — les deux écritures du callout (mika#2120)"
echo "---------------------------------------------------------------------"

# ─────────────────────────────────────────────────────────────────────────────
# mika#2194 — CES FIXTURES INLINE SONT PARTIES DANS LE CORPUS COMMUN
#
# Ce bloc portait **17 assertions à fixtures inline**, et le corpus Rust voisin
# portait **6 corps d'issue réels** : deux jeux soignés, tous les deux, et
# **aucune entrée commune**. Personne n'avait jamais exécuté les deux lecteurs
# sur la même entrée — c'est le défaut central que mika#2194 M3 relève, et il
# n'était ni dans le corps du ticket ni dans le TSV de mika#2201.
#
# Les cas sont donc rapatriés dans
# `crates/mika-agent/tests/fixtures/plan_callout_bodies/`, où les DEUX lecteurs
# les lisent. Le bloc « parité du callout Plan sur le corpus commun » en fin de
# fichier est leur nouveau site d'exécution côté bash ; `plan_callout_parity.rs`
# est l'autre. Correspondance, pour que rien ne se perde :
#
#   forme nue                       → bare-callout.md
#   forme préfixée + parité         → 1680.md … 1949.md (six corps MESURÉS)
#   préfixe ≠ `mika/`               → other-repo-prefix.md
#   quatre contrôles négatifs       → neg-brainstorms / neg-solutions /
#                                     neg-parent-traversal / neg-two-segments
#   ancrage en début de ligne       → neg-inline-prose.md
#   callout canonique / minimal     → bare-callout.md / minimal-callout.md
#   prose « Plan: » sans docs/plans → neg-prose-plan-no-dir.md
#   deux callouts, le premier gagne → double-callout.md
#
# Ne pas les réécrire ici : deux jeux de mesure pour un prédicat, c'est
# exactement la configuration qui a permis à la divergence de vivre.
#
# Les assertions **structurelles** de mika#2120 restent, elles : elles ne
# mesurent pas une entrée, elles mesurent qu'il n'y a qu'un lecteur.

# Un seul lecteur : `_detect_plan_on_branch` ne doit pas reporter sa propre
# extraction. C'est la copie-à-côté-du-lecteur qui a fait diverger les deux
# prédicats Rust pendant des mois (mika#2158) ; même discipline ici.
MIKA2120_DETECT_SRC=$(sed -n '/^_detect_plan_on_branch()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2120: _detect_plan_on_branch a bien été trouvée (guards the guard)" "yes" \
    "$(if [ -n "$MIKA2120_DETECT_SRC" ]; then printf 'yes'; else printf 'no'; fi)"
assert_contains "mika#2120: _detect_plan_on_branch appelle le lecteur partagé" \
    "_extract_plan_path" "$MIKA2120_DETECT_SRC"
assert_eq "mika#2120: _detect_plan_on_branch ne porte plus de motif de callout" "0" \
    "$(printf '%s\n' "$MIKA2120_DETECT_SRC" | grep -cF 'grep -oP' || true)"

echo ""
echo "Test: _pilot_log_dir — un seul point de vérité, résolu tard (mika#2165)"
echo "----------------------------------------------------------------------"
# Trois parties doivent nommer le MÊME répertoire : le pilote depuis l'intérieur
# du bac à sable (--log-dir), le bind bwrap, et l'hôte en post-flight. Ce bloc
# épingle le résolveur unique — et surtout le MOMENT de sa résolution.
#
# Pourquoi le moment est l'assertion principale. La première coupe de mika#2165
# figeait le répertoire dans une variable assignée au chargement de la lib
# (`_PILOT_LOG_DIR="${PILOT_LOG_DIR:-...}"`). Trois sondes de cette suite posent
# PILOT_LOG_DIR APRÈS le `source` ; elles se sont mises à lire
# /var/log/claude-pilot au lieu de leur tmpdir — sans erreur, comme la panne que
# le ticket répare. Une surcharge d'environnement lue une seule fois répond le
# défaut à tout appelant qui la pose trop tard, et le fait en silence.

MIKA2165_RESOLVER_SRC=$(sed -n '/^_pilot_log_dir()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2165: _pilot_log_dir a bien été trouvée (guards the guard)" "yes" \
    "$(if [ -n "$MIKA2165_RESOLVER_SRC" ]; then printf 'yes'; else printf 'no'; fi)"

# Le défaut littéral n'apparaît en CODE qu'à un seul endroit : le résolveur.
# Toute autre occurrence exécutable est une quatrième épellation, exactement ce
# que mika#2165 a fermé. Les lignes de commentaire sont exclues à dessein — la
# prose doit pouvoir nommer le chemin (l'en-tête du bac à sable et la note sur
# le .stderr le font) sans que nommer devienne réintroduire.
assert_eq "mika#2165: le défaut /var/log/claude-pilot n'est écrit qu'une fois en code" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '/var/log/claude-pilot' || true)"

# Aucun site ne doit revenir à une variable gelée au chargement. La forme
# dangereuse est l'assignation en COLONNE ZÉRO — au chargement de la lib, donc
# avant toute surcharge posée par l'appelant. L'assignation *dans* le résolveur
# est au contraire le mécanisme : elle s'exécute à chaque appel.
assert_eq "mika#2165: pas de variable _PILOT_LOG_DIR figée au source" "0" \
    "$(grep -cE '^_PILOT_LOG_DIR=' "$DISPATCH_LIB" || true)"
assert_eq "mika#2165: la seule assignation du résolveur est dans son corps" "1" \
    "$(printf '%s\n' "$MIKA2165_RESOLVER_SRC" | grep -cE '^[[:space:]]+_PILOT_LOG_DIR=' || true)"

# COLLISION mika#2039 — pourquoi le résolveur ASSIGNE au lieu d'IMPRIMER.
#
# La deuxième coupe de mika#2165 exposait `printf` et se lisait `$(_pilot_log_dir)`.
# Sous `set -x` (tout le dispatch tourne ainsi, BASH_XTRACEFD vers $TRACE_FILE que
# _emit_callback renvoie à l'appelant), cette forme écrit `++ printf %s <valeur>`.
# `_scrub_secrets_from_output` ne réécrit que `NAME=value` et les formes de token :
# cette ligne-là n'est couverte par rien. La garde R8 de mika#2039
# (test_sandbox_no_secret_in_argv.sh) interdit donc la FORME, sans regarder la
# valeur — et elle est passée rouge, 34/35, sur ce résolveur.
#
# La valeur en jeu ici est un chemin, jamais un identifiant. C'est précisément
# pourquoi la garde ne raisonne pas par valeur : une garde qui ferait l'exception
# pour ce printf-ci laisserait passer le prochain écrivain. Les deux autres issues
# ont été écartées — assouplir la garde échange une propriété structurelle contre
# une commodité, et emballer le printf dans `set +x` (le bracket de
# _stage_pilot_gh_token) masquerait la ligne au lieu de cesser de la produire.
#
# Cette assertion est ce qui empêche un futur contributeur de « simplifier » le
# résolveur en accesseur imprimant : la garde #2039 redeviendrait rouge, mais
# dans une autre suite, et le lien avec ce ticket serait perdu.
assert_eq "mika#2165 × mika#2039: le résolveur n'imprime pas (aucun printf/echo dans son corps)" "0" \
    "$(printf '%s\n' "$MIKA2165_RESOLVER_SRC" | grep -cE '^[[:space:]]*(printf|echo)[[:space:]]' || true)"
assert_eq "mika#2165 × mika#2039: aucune substitution \$(_pilot_log_dir) ne subsiste" "0" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '$(_pilot_log_dir)' || true)"

# CO-LOCATION — le coût de l'accesseur assignant, refermé structurellement.
#
# Un accesseur qui assigne peut être lu périmé : un appelant qui lit
# $_PILOT_LOG_DIR sans appeler le résolveur juste avant obtient la valeur d'un
# appel précédent (ou rien). Ce serait une nouvelle panne silencieuse, la classe
# exacte que ce ticket referme — donc elle n'est pas laissée à la discipline.
#
# Règle : toute ligne de code qui LIT $_PILOT_LOG_DIR doit aussi APPELER
# _pilot_log_dir. Les lignes du corps du résolveur et les commentaires sont hors
# sujet ; les déclarations de tableau `_PILOT_LOG_BIND_ARGS` portent un autre nom
# et ne matchent pas.
MIKA2165_UNCOLOCATED=$(grep -nE '\$\{?_PILOT_LOG_DIR\b' "$DISPATCH_LIB" \
    | grep -vE '^[0-9]+:[[:space:]]*#' \
    | grep -vE '_pilot_log_dir' \
    || true)
assert_eq "mika#2165: chaque lecture de \$_PILOT_LOG_DIR appelle le résolveur sur la même ligne" "" \
    "$MIKA2165_UNCOLOCATED"

# `--log-dir` valué, jamais nu : nu, argparse retombe sur le `const` Python
# (/var/log/claude-pilot) et le pilote écrit ailleurs que là où l'hôte bind et
# relit — l'invariant à trois têtes casse sans qu'aucun test structurel ne bouge.
assert_eq "mika#2165: aucun --log-dir nu ne subsiste" "0" \
    "$(grep -cE -- '--log-dir([[:space:]]+--|[[:space:]]*$)' "$DISPATCH_LIB" || true)"
#
# Compte passé de 2 à 3 le 2026-09-21 (mika#2306) : la relance Fire-Disposition
# est un troisième lancement, et elle reproduit la forme co-localisée ci-dessus
# plutôt que de lire une valeur héritée du lancement nominal.
#
# Compte passé de 3 à 4 le 2026-10-02 (mika#2636) : la relance de taille est un
# quatrième lancement, copie structurelle du troisième, et elle reproduit la
# même forme co-localisée.
assert_eq "mika#2165: chaque --log-dir est valué par le résolveur" "4" \
    "$(grep -cF -- '--log-dir "$_PILOT_LOG_DIR"' "$DISPATCH_LIB" || true)"

# L'assertion de comportement : la surcharge posée APRÈS le source est honorée.
# C'est celle qui aurait attrapé la régression ci-dessus ; les assertions
# structurelles au-dessus n'y suffisent pas.
# Les sondes impriment $_PILOT_LOG_DIR elles-mêmes : le résolveur assigne, il ne
# rend rien sur stdout (voir la collision mika#2039 ci-dessus). Ce `printf` vit
# dans la suite de tests, pas dans le dispatch tracé — la garde R8 ne le voit pas
# et n'a pas à le voir.
_mika2165_late_override_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        PILOT_LOG_DIR="/tmp/mika2165-late-override"
        _pilot_log_dir; printf '%s' "$_PILOT_LOG_DIR"
    )
}
assert_eq "mika#2165: PILOT_LOG_DIR posé APRÈS le source est honoré" \
    "/tmp/mika2165-late-override" "$(_mika2165_late_override_probe)"

_mika2165_default_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PILOT_LOG_DIR
        _pilot_log_dir; printf '%s' "$_PILOT_LOG_DIR"
    )
}
assert_eq "mika#2165: sans surcharge, le défaut opérationnel est rendu" \
    "/var/log/claude-pilot" "$(_mika2165_default_probe)"

# Le bind suit le résolveur, pas une copie. Un bind qui nommerait un autre
# répertoire que celui passé à --log-dir serait vert partout et vide : le
# pilote écrirait dans le tmpfs, exactement la panne d'origine.
_mika2165_bind_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        PILOT_LOG_DIR=$(mktemp -d "${TMPDIR:-/tmp}/mika2165-bind.XXXXXX")
        _PILOT_LOG_BIND_ARGS=()
        _pilot_log_bind_args 2>/dev/null
        printf '%s' "${_PILOT_LOG_BIND_ARGS[*]}"
        rm -rf "$PILOT_LOG_DIR"
    )
}
MIKA2165_BIND_OUT=$(_mika2165_bind_probe)
assert_contains "mika#2165: le bind est rw (--bind, pas --ro-bind)" \
    "--bind " "$MIKA2165_BIND_OUT"
assert_eq "mika#2165: le bind nomme le répertoire du résolveur, source == cible" "yes" \
    "$(set -- $MIKA2165_BIND_OUT; if [ "${2:-}" = "${3:-x}" ] && [ -n "${2:-}" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "mika#2165: le bind ne s'élargit pas à /var/log" "no" \
    "$(set -- $MIKA2165_BIND_OUT; if [ "${2:-}" = "/var/log" ]; then printf 'yes'; else printf 'no'; fi)"

# Le repli est une liste VIDE, jamais un bind rigide : `--bind` dont la source
# manque fait échouer bwrap en entier. Le défaut d'aujourd'hui perd un journal ;
# un bind rigide perdrait la session.
_mika2165_unwritable_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        PILOT_LOG_DIR="/proc/mika2165-cannot-exist"
        _PILOT_LOG_BIND_ARGS=(sentinel)
        _pilot_log_bind_args 2>/dev/null
        printf '%s' "${_PILOT_LOG_BIND_ARGS[*]}"
    )
}
assert_eq "mika#2165: un répertoire impossible laisse la liste vide (bwrap survit)" \
    "" "$(_mika2165_unwritable_probe)"
assert_contains "mika#2165: et le repli est bruyant — la moitié hôte d'AC3" \
    "pilot_log_guard." "$(
        (
            # shellcheck disable=SC1090
            source "$DISPATCH_LIB" 2>/dev/null || true
            PILOT_LOG_DIR="/proc/mika2165-cannot-exist"
            _PILOT_LOG_BIND_ARGS=()
            _pilot_log_bind_args 2>&1 >/dev/null
        )
    )"

echo ""
echo "Test: le plafond de tours du pilote est armé à la source (mika#2496)"
echo "---------------------------------------------------------------------"
# Aucun test comportemental ne peut voir la classe que ce bloc garde. Un
# quatrième site de lancement écrit demain sans `--max-turns` ne rend AUCUNE
# décision fausse : il tourne simplement sans borne, et toutes les assertions
# existantes restent vertes. C'est la forme exacte du défaut que mika#2496
# ferme — trois sites de lancement, zéro drapeau, depuis toujours — reproduite
# un cran plus tard. D'où un scan de source.
#
# LE MOTIF EST FIGÉ, parce qu'un scan sur « tout appel `claude-pilot` » est faux
# dans les deux directions. La chaîne apparaît ~47 fois dans le fichier et une
# seule famille est un lancement : commentaires, `--ro-bind-try
# "$HOME/.local/bin/claude-pilot"`, `--relay-config …/claude-pilot.json`,
# `command -v claude-pilot`, le smoke test `timeout 15 claude-pilot --help`, et
# de la prose dans les corps `RESULT=`. Un scan naïf rougirait sur les ~44
# autres ; resserré sur la LIGNE PHYSIQUE, il manquerait deux des trois sites
# réels, écrits sur plusieurs lignes avec continuation `\` — `--max-turns` y vit
# légitimement sur une ligne que le grep ne regarde pas.
#
# Cinq termes, dont le cinquième est une correction MESURÉE du plan :
#
#   1. unité d'analyse = l'INVOCATION LOGIQUE, obtenue en recollant les
#      continuations `\`, jamais la ligne physique. C'est le terme que la forme
#      des deux sites de revise impose (fixture N2).
#   2. les lignes de commentaire sont retirées AVANT le recollement.
#   3. `claude-pilot` en POSITION DE COMMANDE : précédé d'un début d'invocation
#      (début de ligne, `;`, `|`, `&`, `(`, `{`) ou du nom du wrapper
#      `_run_pilot_sandboxed`. Un `/` n'est pas dans ce jeu, ce qui exclut les
#      chemins (`--ro-bind-try "…/bin/claude-pilot"`).
#   4. le token suivant est exigé d'être un DRAPEAU (`[[:space:]]+-`). Ça borne
#      le jeton par la droite — `claude-pilot.json` et `claude-pilot-py` ne
#      peuvent plus matcher — et ça exclut la prose.
#   5. la CARDINALITÉ est assertée à 3. Sans elle, un prédicat devenu trop
#      étroit passerait au vert en ne regardant plus rien (classe mika#2205),
#      et c'est la seule forme de panne qu'aucune des cinq fixtures ne voit.
#
# Le terme 4 N'EST PAS dans le plan, et il a été ajouté sur une mesure : le plan
# nomme « de la prose dans les corps `RESULT=` » parmi ce que le scan doit
# exclure, mais ses quatre termes ne l'excluaient pas. `dispatch-lib.sh` porte
# `claude-pilot FAILED (exit code ${PILOT_EXIT}).` EN COLONNE ZÉRO, à
# l'intérieur d'une chaîne `RESULT="…"` multi-lignes — donc « début de ligne »,
# donc candidat, donc cardinalité 4 au lieu de 3. Le terme 4 l'écarte sans rien
# coûter : un lancement porte toujours des drapeaux. Fixture N5.
#
# Le terme 3 est délibérément une DISJONCTION (le wrapper *ou* la position de
# commande) et non le wrapper seul : un site futur qui appellerait le binaire
# sans passer par `_run_pilot_sandboxed` échapperait à un prédicat ancré sur le
# wrapper — et ce site-là est aussi celui qui perdrait le confinement réseau
# (mika#2049), donc il doit rougir ici plutôt que passer. Fixture N4.

# L'allowlist est **livrée vide**. Quand ce scan tire, la résolution est d'ARMER
# le site, jamais de l'allowlister : doctrine mika#2201, « on déclare, on
# n'allowliste pas ». Un site de lancement qu'on ne veut pas armer est un site
# qu'il faut supprimer.
MIKA2496_LAUNCH_EXCEPTIONS=()

# Rend une invocation logique par ligne : commentaires retirés, continuations
# recollées. Le `sed` est l'idiome standard de recollement (`N` + `ta`).
_mika2496_logical_invocations() {
    grep -vE '^[[:space:]]*#' "$1" | sed -e :a -e '/\\$/N; s/\\\n/ /; ta'
}

# Les invocations logiques qui LANCENT claude-pilot (termes 3 et 4).
_mika2496_launch_candidates() {
    _mika2496_logical_invocations "$1" \
        | grep -E '(^|[;|&(){}]|_run_pilot_sandboxed)[[:space:]]*claude-pilot[[:space:]]+-' \
        || true
}

# Celles qui le lancent SANS plafond — la population que le scan refuse.
_mika2496_unbounded_launches() {
    local line
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        local skip=""
        local exc
        for exc in "${MIKA2496_LAUNCH_EXCEPTIONS[@]+"${MIKA2496_LAUNCH_EXCEPTIONS[@]}"}"; do
            case "$line" in *"$exc"*) skip=1 ;; esac
        done
        [ -n "$skip" ] && continue
        case "$line" in *--max-turns*) ;; *) printf '%s\n' "$line" ;; esac
    done < <(_mika2496_launch_candidates "$1")
}

# Terme 5 — la cardinalité. Anti-vacuité : un scan qui ne trouve PERSONNE se lit
# exactement comme un scan propre.
# Compte passé de 3 à 4 le 2026-10-02 (mika#2636) : la relance de taille est un
# quatrième site de lancement, et il porte `--max-turns` comme les trois autres.
assert_eq "mika#2496: le scan voit exactement les quatre sites de lancement" "4" \
    "$(_mika2496_launch_candidates "$DISPATCH_LIB" | wc -l | tr -d ' ')"

# L'assertion elle-même : aucun lancement sans son plafond.
assert_eq "mika#2496: aucun site de lancement ne part sans --max-turns" "" \
    "$(_mika2496_unbounded_launches "$DISPATCH_LIB" | cut -c1-100)"

assert_eq "mika#2496: l'allowlist des lancements non bornés est livrée vide" "0" \
    "${#MIKA2496_LAUNCH_EXCEPTIONS[@]}"

# --- Contrôles négatifs N1–N5, sur les formes d'appel RÉELLEMENT présentes ---
#
# Un scan vert par vacuité sur les formes qu'il ne reconnaît pas est
# indistinguable d'un scan qui couvre (mika#2205). Chaque fixture est donc
# calquée sur une forme qui existe dans `dispatch-lib.sh`, jamais inventée, et
# chacune est vue rouge ou vue verte explicitement.
MIKA2496_FIXDIR=$(mktemp -d "${TMPDIR:-/tmp}/mika2496-fixtures.XXXXXX")

# N1 — mono-ligne, la forme du dispatch principal, sans le drapeau. ROUGE.
printf '%s\n' \
    '    _pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose --log-dir "$D" --task-id "$I" --command "$C" -- "$P"' \
    > "$MIKA2496_FIXDIR/n1"

# N2 — multi-lignes avec continuations, la forme des deux pilotes de revise,
# sans le drapeau. ROUGE — atteste le recollement du terme 1.
printf '%s\n' \
    '    _pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose --log-dir "$D" --task-id "$I" \' \
    '        --command "/mika-revise-plan" $CWD_ARGS \' \
    '        -- "@$F"' \
    > "$MIKA2496_FIXDIR/n2"

# N3 — même forme, drapeau présent mais SUR UNE LIGNE DE CONTINUATION. VERT.
# Le miroir de N2 : sans lui, « le scan lit l'invocation » ne se distingue pas
# de « le scan rougit sur tout multi-ligne ».
printf '%s\n' \
    '    _pilot_log_dir; _run_pilot_sandboxed claude-pilot --verbose --log-dir "$D" --task-id "$I" \' \
    '        --max-turns 120 \' \
    '        --command "/mika-revise-plan" $CWD_ARGS' \
    > "$MIKA2496_FIXDIR/n3"

# N4 — appel direct, SANS `_run_pilot_sandboxed`, sans le drapeau. ROUGE —
# atteste la disjonction du terme 3.
printf '%s\n' \
    '    claude-pilot --verbose --log-dir "$D" --task-id "$I" --command "$C"' \
    > "$MIKA2496_FIXDIR/n4"

# N5 — bonne foi : un commentaire, un chemin bindé, la config du relais, et la
# prose `RESULT=` en colonne zéro. VERT — sans quoi le scan serait rouge en
# permanence, donc désarmé.
printf '%s\n' \
    '# on lancera claude-pilot --verbose ici un jour' \
    '            --ro-bind-try "$HOME/.local/bin/claude-pilot" "$HOME/.local/bin/claude-pilot" \' \
    '            --ro-bind-try "$HOME/.local/share/uv/tools/claude-pilot" "$X" \' \
    '        CWD_ARGS="$CWD_ARGS --relay-config $W/.claude/claude-pilot.json"' \
    '    command -v claude-pilot >/dev/null 2>&1 || exit 1' \
    '    if ! timeout 15 claude-pilot --help >/dev/null 2>&9; then' \
    'claude-pilot FAILED (exit code ${PILOT_EXIT}).' \
    > "$MIKA2496_FIXDIR/n5"

for _mika2496_red in n1 n2 n4; do
    assert_eq "mika#2496: fixture $_mika2496_red est VUE ROUGE (lancement sans plafond)" "1" \
        "$(_mika2496_unbounded_launches "$MIKA2496_FIXDIR/$_mika2496_red" | wc -l | tr -d ' ')"
done
assert_eq "mika#2496: fixture n3 est VUE VERTE (drapeau sur une continuation)" "0" \
    "$(_mika2496_unbounded_launches "$MIKA2496_FIXDIR/n3" | wc -l | tr -d ' ')"
assert_eq "mika#2496: fixture n3 EST bien un candidat (le vert n'est pas de la vacuité)" "1" \
    "$(_mika2496_launch_candidates "$MIKA2496_FIXDIR/n3" | wc -l | tr -d ' ')"
assert_eq "mika#2496: fixture n5 (bonne foi) n'est candidate à rien" "0" \
    "$(_mika2496_launch_candidates "$MIKA2496_FIXDIR/n5" | wc -l | tr -d ' ')"

rm -rf "$MIKA2496_FIXDIR"

# --- CO-LOCATION, le coût de l'accesseur assignant, refermé structurellement ---
#
# Même mécanisme et même raison que pour `$_PILOT_LOG_DIR` (mika#2165) : un
# accesseur qui ASSIGNE peut être lu périmé, et un site de lancement qui lirait
# `$_PILOT_MAX_TURNS` sans appeler `_pilot_max_turns` juste avant hériterait du
# plafond d'un appel précédent — ou de rien, c'est-à-dire d'un dispatch non
# borné qui se lirait comme borné.
# La co-location se lit sur l'INVOCATION LOGIQUE, pas sur la ligne physique —
# même unité d'analyse que le scan ci-dessus, et pour la même raison : deux des
# trois sites portent le drapeau sur une ligne de continuation, donc la lecture
# de `$_PILOT_MAX_TURNS` et l'appel du résolveur y vivent sur des lignes
# physiques différentes tout en appartenant à la même commande. Un prédicat à la
# ligne les accuserait toutes les deux.
MIKA2496_UNCOLOCATED=$(_mika2496_logical_invocations "$DISPATCH_LIB" \
    | grep -E '\$\{?_PILOT_MAX_TURNS\b' \
    | grep -vE '_pilot_max_turns' \
    || true)
assert_eq "mika#2496: chaque lecture de \$_PILOT_MAX_TURNS appelle le résolveur sur la même ligne" "" \
    "$MIKA2496_UNCOLOCATED"

# Le résolveur ASSIGNE, il n'imprime pas — collision mika#2039, identique à
# celle que `_pilot_log_dir` documente : un accesseur imprimant se lirait
# `$(_pilot_max_turns)` et poserait `++ printf %s <valeur>` dans la trace
# `set -x`, forme qu'aucun scrubber ne couvre.
MIKA2496_RESOLVER_SRC=$(sed -n '/^_pilot_max_turns()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2496: _pilot_max_turns a bien été trouvée (guards the guard)" "yes" \
    "$(if [ -n "$MIKA2496_RESOLVER_SRC" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "mika#2496 × mika#2039: le résolveur n'imprime pas" "0" \
    "$(printf '%s\n' "$MIKA2496_RESOLVER_SRC" | grep -cE '^[[:space:]]*(printf|echo)[[:space:]]' || true)"
assert_eq "mika#2496 × mika#2039: aucune substitution \$(_pilot_max_turns) ne subsiste" "0" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '$(_pilot_max_turns)' || true)"

# --- AC7 : rien n'annonce un budget dollars qui n'existe pas ---
#
# `--max-budget` serait accepté, validé, résolu, porté jusqu'à
# `_sdk_guardrail_kwargs` — et ignoré (son `if config.maxBudgetUsd > 0:` se
# termine sur `pass`). Le passer ferait paraître le budget fermé dans l'argv.
assert_eq "mika#2496 (AC7): aucun site ne passe --max-budget" "0" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF -- '--max-budget' || true)"

# --- V4 : le comportement du résolveur, les trois paliers ---
# `$2` (optionnel, mika#2542) : le CSV des labels du ticket, tel que les sites de
# lancement le passent au résolveur.
_mika2496_resolve_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PILOT_MAX_TURNS
        if [ "$1" != "__UNSET__" ]; then export PILOT_MAX_TURNS="$1"; fi
        _pilot_max_turns "${2:-}"
        printf '%s|%s|%s' "$_PILOT_MAX_TURNS" "$_PILOT_MAX_TURNS_SOURCE" "$_PILOT_MAX_TURNS_INVALID"
    )
}
# mika#2542 (D2) : le défaut de flotte est ARMÉ à 150 — V2 de mika#2496 rapportée.
# Le diff de cette assertion (`|default|` → `150|default|`) EST la décision.
assert_eq "mika#2542 (AC9): sans surcharge, le défaut de flotte est armé à 150" \
    "150|default|" "$(_mika2496_resolve_probe __UNSET__)"
assert_eq "mika#2496: une valeur entière est honorée, provenance env" \
    "120|env|" "$(_mika2496_resolve_probe 120)"
# Contrôle négatif du rollback (AC4) : `0` omet le drapeau, ce qui rend le
# comportement d'avant mika#2496 — claude-pilot retombe sur maxTurns=200.
assert_eq "mika#2496 (AC4): PILOT_MAX_TURNS=0 est le ROLLBACK, pas une borne" \
    "|env|" "$(_mika2496_resolve_probe 0)"
assert_eq "mika#2496 (AC4): une valeur vide vaut rollback" \
    "|env|" "$(_mika2496_resolve_probe '')"
# Un désarmement par coquille sur un frein de coût serait la panne silencieuse
# que tout ceci ferme : il retombe au défaut ET il est DIT.
assert_eq "mika#2496: une valeur illisible retombe au défaut et est nommée" \
    "150|default|abc" "$(_mika2496_resolve_probe abc)"
assert_eq "mika#2496: une valeur négative retombe au défaut et est nommée" \
    "150|default|-5" "$(_mika2496_resolve_probe '-5')"

# --- V4 : le drapeau atteint l'argv, avec la valeur résolue ---
#
# C'est LE contrat côté mika. « Le budget tue avant 120 tours » est exécuté par
# le SDK, dans un autre processus, avec un vrai fournisseur — la moitié
# comportementale est la sonde S2, pas ce harnais.
_mika2496_argv_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PILOT_MAX_TURNS
        if [ "$1" != "__UNSET__" ]; then export PILOT_MAX_TURNS="$1"; fi
        _pilot_max_turns "${2:-}"
        # shellcheck disable=SC2086
        printf '%s' "claude-pilot --verbose ${_PILOT_MAX_TURNS:+--max-turns $_PILOT_MAX_TURNS} --command /mika"
    )
}
assert_contains "mika#2496 (AC1): armé, l'argv porte --max-turns avec la valeur résolue" \
    '--max-turns 120' "$(_mika2496_argv_probe 120)"
# Depuis mika#2542 le défaut est armé : le seul désarmement est le rollback `0`.
assert_not_contains "mika#2496 (AC4): désarmé, l'argv ne porte AUCUN --max-turns" \
    '--max-turns' "$(_mika2496_argv_probe 0)"
assert_contains "mika#2542 (AC2): sans surcharge, l'argv porte le défaut armé" \
    '--max-turns 150' "$(_mika2496_argv_probe __UNSET__)"

# --- U2/AC2 : le budget en vigueur est dit, et la ligne lit l'ARGV ---
_mika2496_budget_line() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PILOT_MAX_TURNS
        if [ "$1" != "__UNSET__" ]; then export PILOT_MAX_TURNS="$1"; fi
        _pilot_max_turns "${2:-}"
        # shellcheck disable=SC2086
        _emit_pilot_budget_line claude-pilot --verbose ${_PILOT_MAX_TURNS:+--max-turns $_PILOT_MAX_TURNS} --command /mika 2>&1
    )
}
assert_contains "mika#2496 (AC2): armé par l'env, la ligne dit la valeur et sa provenance" \
    'pilot_budget_armed max_turns=120 source=env cost_bound=absent_upstream' \
    "$(_mika2496_budget_line 120)"
assert_contains "mika#2496 (AC2): désarmé, la ligne dit max_turns=none — jamais une borne inventée" \
    'pilot_budget_armed max_turns=none source=env cost_bound=absent_upstream' \
    "$(_mika2496_budget_line 0)"
assert_contains "mika#2542 (AC2): sans surcharge, la ligne dit le défaut armé et sa provenance" \
    'pilot_budget_armed max_turns=150 source=default cost_bound=absent_upstream' \
    "$(_mika2496_budget_line __UNSET__)"
assert_contains "mika#2496: une coquille est nommée entre guillemets" \
    'pilot_budget_invalid PILOT_MAX_TURNS="oops"' \
    "$(_mika2496_budget_line oops)"
# AC7 : la ligne REFUSE d'annoncer un budget dollars, parce qu'il n'en existe
# aucun en amont. Elle nomme l'absence au lieu de la taire.
assert_not_contains "mika#2496 (AC7): la ligne n'annonce jamais un budget dollars" \
    'cost_bound=40' "$(_mika2496_budget_line 120)"
# Ancrage obligatoire à la lecture (mika#2050) : le `.stderr` porte aussi la
# prose du pilote, et une session discutant du signal s'est déjà lue comme une
# émission.
assert_contains "mika#2496: la ligne est ancrable sur '^dispatch-lib: '" \
    'dispatch-lib: pilot_budget_armed' "$(_mika2496_budget_line 120)"

# La ligne est émise depuis `_run_pilot_sandboxed`, donc SOUS la redirection
# `2>"$STDERR_FILE"` du site de lancement — donc dans le sillon forensique
# per-dispatch. L'émettre avant la ligne de lancement l'enverrait sur le stderr
# propre de dispatch-lib, que l'exécuteur ne lit QUE sur `if !status.success()` :
# sur un dispatch qui réussit, le tuyau est jeté sans être lu (Signal M,
# mesuré par mika#2050). Cette assertion est ce qui empêche un futur
# contributeur de « simplifier » en déplaçant l'appel sur la ligne de lancement.
assert_eq "mika#2496: la ligne est émise depuis _run_pilot_sandboxed, pas avant le lancement" "1" \
    "$(sed -n '/^_run_pilot_sandboxed()/,/^}/p' "$DISPATCH_LIB" | grep -cF '_emit_pilot_budget_line' || true)"
# Un seul APPEL en production — la définition est exclue par sa parenthèse, pas
# par son indentation : un appel écrit en colonne zéro resterait compté.
assert_eq "mika#2496: et depuis nulle part ailleurs" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -E '_emit_pilot_budget_line' | grep -cvE '_emit_pilot_budget_line\(\)' || true)"

# --- mika#2542 : le plafond de tours se résout depuis le label du ticket ---
echo ""
echo "Test: le plafond de tours se résout depuis le label du ticket (mika#2542)"
echo "-------------------------------------------------------------------------"
# Trois implements `loop-substrate` coupés à 151 tours sous le plafond 150. Le
# ticket porteur du label reçoit 200 ; tout autre reste à 150. L'ORDRE de la
# cascade est le livrable : l'hôte de production porte `PILOT_MAX_TURNS=150`,
# donc un palier label placé sous l'env serait inerte partout où il compte
# (classe mika#2205). Les probes réutilisent celles de mika#2496, étendues d'un
# second argument (le CSV des labels).

# Les deux allowlists sont LIVRÉES VIDES (doctrine mika#2201 : on déclare, on
# n'allowliste pas). Quand G1 tire : déclarer le label dans labels.yml. Quand G2
# tire : passer "${LABELS:-}" au résolveur.
MIKA2542_CEILING_LABEL_EXCEPTIONS=()
MIKA2542_UNARGUMENTED_SITES=()
assert_eq "mika#2542: l'allowlist des labels non déclarés est livrée vide" "0" \
    "${#MIKA2542_CEILING_LABEL_EXCEPTIONS[@]}"
assert_eq "mika#2542: l'allowlist des sites sans argument est livrée vide" "0" \
    "${#MIKA2542_UNARGUMENTED_SITES[@]}"

# --- G3 / V1 : la cascade, palier par palier (`valeur|source|invalid`) ---
assert_eq "mika#2542 (AC1): un ticket loop-substrate reçoit 200, provenance label" \
    "200|label|" "$(_mika2496_resolve_probe __UNSET__ loop-substrate)"
assert_eq "mika#2542 (AC2): sans label, une valeur d'env reste honorée" \
    "120|env|" "$(_mika2496_resolve_probe 120 '')"
# R1-bis — le cas qui décide si le ticket a un effet en production.
assert_eq "mika#2542 (AC3): le label BAT l'env (sinon inerte sur l'hôte de prod)" \
    "200|label|" "$(_mika2496_resolve_probe 150 loop-substrate)"
assert_eq "mika#2542 (AC3): le label bat l'env même quand l'env vaut plus bas" \
    "200|label|" "$(_mika2496_resolve_probe 120 loop-substrate)"
# Le rollback mika#2496 reste un rollback, label ou pas.
assert_eq "mika#2542 (AC4): PILOT_MAX_TURNS=0 reste le ROLLBACK, le label n'a pas voix" \
    "|env|" "$(_mika2496_resolve_probe 0 loop-substrate)"
assert_eq "mika#2542 (AC4): une valeur vide reste le rollback sur un ticket substrat" \
    "|env|" "$(_mika2496_resolve_probe '' loop-substrate)"
# L'invalidité est dite indépendamment du palier qui décide.
assert_eq "mika#2542: une coquille sur un ticket substrat — le label décide ET la coquille est nommée" \
    "200|label|abc" "$(_mika2496_resolve_probe abc loop-substrate)"
assert_eq "mika#2542: une coquille sans label retombe au défaut armé, nommée" \
    "150|default|abc" "$(_mika2496_resolve_probe abc '')"

# --- G3 / V2 : appariement EXACT sur un élément du CSV (AC6) ---
#
# Fixtures négatives d'abord : un glob de sous-chaîne (`*loop-substrate*`, la
# forme de `_label_to_type`) relèverait chacune d'elles. Vues rouges avant
# d'être vues vertes — c'est la Halte 4 du plan tenue à l'unité.
for _mika2542_near in not-loop-substrate loop-substrate-v2 xloop-substrate loop-substratex \
    'bug,loop-substrate-v2' 'not-loop-substrate,p1-important' 'LOOP-SUBSTRATE' ' loop-substrate'; do
    assert_eq "mika#2542 (AC6): '$_mika2542_near' ne relève RIEN" \
        "150|default|" "$(_mika2496_resolve_probe __UNSET__ "$_mika2542_near")"
done
# Contrôle de bonne foi : le label relève en tête, au milieu et en queue de CSV.
for _mika2542_pos in 'loop-substrate,p1-important' 'ready,loop-substrate,p1-important' \
    'p1-important,loop-substrate'; do
    assert_eq "mika#2542 (AC6): '$_mika2542_pos' relève bien" \
        "200|label|" "$(_mika2496_resolve_probe __UNSET__ "$_mika2542_pos")"
done

# --- G3 / V3 : le drapeau atteint l'argv avec la valeur du label ---
assert_contains "mika#2542 (AC1): un ticket loop-substrate porte --max-turns 200 dans l'argv" \
    '--max-turns 200' "$(_mika2496_argv_probe 150 loop-substrate)"
assert_not_contains "mika#2542 (AC4): rollback sur un ticket substrat — aucun --max-turns" \
    '--max-turns' "$(_mika2496_argv_probe 0 loop-substrate)"

# --- G3 / V5 : la ligne d'observabilité ---
assert_contains "mika#2542 (AC1): la ligne dit source=label et QUEL label a décidé" \
    'pilot_budget_armed max_turns=200 source=label label=loop-substrate cost_bound=absent_upstream' \
    "$(_mika2496_budget_line 150 loop-substrate)"
# Contrôle négatif : aucun champ `label=` quand le label n'a pas décidé — un
# champ vide se lirait comme un label nommé « vide ».
assert_not_contains "mika#2542 (AC2): sans label, aucun champ label= n'est émis" \
    'label=' "$(_mika2496_budget_line 150 '')"
assert_not_contains "mika#2542 (AC4): rollback sur un ticket substrat, aucun champ label=" \
    'label=' "$(_mika2496_budget_line 0 loop-substrate)"
assert_not_contains "mika#2542 (AC7 mika#2496): la ligne relevée n'annonce pas de budget dollars" \
    'cost_bound=4' "$(_mika2496_budget_line __UNSET__ loop-substrate)"

# --- G4 : la valeur du défaut in-file, pinnée littéralement ---
#
# Lue dans la source, pas au comportement : l'assertion comportementale
# (`150|default|`) rougirait aussi, mais celle-ci nomme LE site à relire.
assert_eq "mika#2542 (AC9): le défaut in-file de _pilot_max_turns vaut 150" "1" \
    "$(sed -n '/^_pilot_max_turns()/,/^}/p' "$DISPATCH_LIB" | grep -cE '^[[:space:]]*local _default="150"$' || true)"

# --- AC5 : un seul site nommé, et aucun second lecteur de label ne décide ---
assert_eq "mika#2542 (AC5): PILOT_LABEL_TURN_CEILINGS est déclarée exactement une fois" "1" \
    "$(grep -cE '^PILOT_LABEL_TURN_CEILINGS=\(' "$DISPATCH_LIB" || true)"
assert_eq "mika#2542 (AC5): la table porte loop-substrate=200" "1" \
    "$(sed -n '/^PILOT_LABEL_TURN_CEILINGS=(/,/^)/p' "$DISPATCH_LIB" | grep -cxE '[[:space:]]*"loop-substrate=200"' || true)"
# Le seul consommateur de la table est le helper ; le seul appelant du helper
# est le résolveur. Un second lecteur ferait deux sites de décision.
assert_eq "mika#2542 (AC5): la table n'est itérée que par _pilot_label_turn_ceiling" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '"${PILOT_LABEL_TURN_CEILINGS[@]}"' || true)"
assert_eq "mika#2542 (AC5): _pilot_label_turn_ceiling n'est appelée que depuis le résolveur" "1" \
    "$(sed -n '/^_pilot_max_turns()/,/^}/p' "$DISPATCH_LIB" | grep -cF '_pilot_label_turn_ceiling' || true)"
assert_eq "mika#2542 (AC5): …et depuis nulle part ailleurs" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -F '_pilot_label_turn_ceiling' | grep -cvE '_pilot_label_turn_ceiling\(\)' || true)"
# Le helper assigne, il n'imprime pas (mika#2039).
assert_eq "mika#2542 × mika#2039: le helper de label n'imprime pas" "0" \
    "$(sed -n '/^_pilot_label_turn_ceiling()/,/^}/p' "$DISPATCH_LIB" | grep -cE '^[[:space:]]*(printf|echo)[[:space:]]' || true)"

# --- G2 / V4 : les trois sites passent les labels au résolveur (AC10) ---
#
# Sur les invocations LOGIQUES (continuations recollées), même unité que le scan
# mika#2496. Un appel `_pilot_max_turns` suivi d'un `;` ou d'une fin de ligne est
# un site qui a oublié l'argument : il résout « aucun label », donc 150 — fail-
# safe, mais silencieusement hors du mécanisme.
# L'amorce est « tout caractère hors identifiant », pas une liste de ponctuations :
# un appel écrit après un mot-clé (`then _pilot_max_turns …`) doit être vu aussi.
_mika2542_resolver_calls() {
    _mika2496_logical_invocations "$1" \
        | grep -E '(^|[^A-Za-z0-9_])_pilot_max_turns([^A-Za-z0-9_]|$)' \
        | grep -vE '^[[:space:]]*_pilot_max_turns\(\)' \
        || true
}
_mika2542_unargumented_calls() {
    local line exc skip
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        skip=""
        for exc in "${MIKA2542_UNARGUMENTED_SITES[@]+"${MIKA2542_UNARGUMENTED_SITES[@]}"}"; do
            case "$line" in *"$exc"*) skip=1 ;; esac
        done
        [ -n "$skip" ] && continue
        if ! grep -qE '_pilot_max_turns[[:space:]]+"\$\{LABELS:-\}"' <<<"$line"; then
            printf '%s\n' "$line"
        fi
    done < <(_mika2542_resolver_calls "$1")
}
# Compte passé de 3 à 4 le 2026-10-02 (mika#2636) : la relance de taille appelle
# le résolveur sur son propre site, en lui passant `"${LABELS:-}"` comme les
# trois autres — un site qui l'oublierait résoudrait « aucun label », donc le
# défaut, donc fail-safe vers 150 et jamais vers 200.
assert_eq "mika#2542 (AC10): le scan voit exactement quatre appels du résolveur" "4" \
    "$(_mika2542_resolver_calls "$DISPATCH_LIB" | wc -l | tr -d ' ')"
assert_eq "mika#2542 (AC10): chaque appel du résolveur passe \"\${LABELS:-}\"" "" \
    "$(_mika2542_unargumented_calls "$DISPATCH_LIB" | cut -c1-100)"

# Contrôles négatifs de G2, calqués sur la forme réelle des sites.
MIKA2542_FIXDIR=$(mktemp -d "${TMPDIR:-/tmp}/mika2542-fixtures.XXXXXX")
printf '%s\n' \
    '    _pilot_log_dir; _pilot_max_turns; _run_pilot_sandboxed claude-pilot --verbose ${_PILOT_MAX_TURNS:+--max-turns $_PILOT_MAX_TURNS} --command "$C"' \
    > "$MIKA2542_FIXDIR/g2_red"
printf '%s\n' \
    '    _pilot_log_dir; _pilot_max_turns "${LABELS:-}"; _run_pilot_sandboxed claude-pilot --verbose --task-id "$I" \' \
    '        ${_PILOT_MAX_TURNS:+--max-turns $_PILOT_MAX_TURNS} \' \
    '        --command "/mika-revise-plan"' \
    > "$MIKA2542_FIXDIR/g2_green"
assert_eq "mika#2542: fixture g2_red (site sans argument) est VUE ROUGE" "1" \
    "$(_mika2542_unargumented_calls "$MIKA2542_FIXDIR/g2_red" | wc -l | tr -d ' ')"
assert_eq "mika#2542: fixture g2_green (argument passé, multi-lignes) est VUE VERTE" "0" \
    "$(_mika2542_unargumented_calls "$MIKA2542_FIXDIR/g2_green" | wc -l | tr -d ' ')"
assert_eq "mika#2542: fixture g2_green EST bien un appel vu (le vert n'est pas de la vacuité)" "1" \
    "$(_mika2542_resolver_calls "$MIKA2542_FIXDIR/g2_green" | wc -l | tr -d ' ')"
printf '%s\n' \
    '    if [ -n "$X" ]; then _pilot_max_turns; fi' \
    > "$MIKA2542_FIXDIR/g2_keyword"
assert_eq "mika#2542: fixture g2_keyword (appel après then, sans argument) est VUE ROUGE" "1" \
    "$(_mika2542_unargumented_calls "$MIKA2542_FIXDIR/g2_keyword" | wc -l | tr -d ' ')"
rm -rf "$MIKA2542_FIXDIR"

# Auto-nettoyage des allowlists (exigence (a) de mika#1574) : une entrée qui
# n'apparie plus rien fait rougir. Livrées vides, elles le restent sans qu'un
# relecteur ait à y penser.
_mika2542_stale=""
for _mika2542_exc in "${MIKA2542_UNARGUMENTED_SITES[@]+"${MIKA2542_UNARGUMENTED_SITES[@]}"}"; do
    _mika2542_resolver_calls "$DISPATCH_LIB" | grep -qF -- "$_mika2542_exc" \
        || _mika2542_stale+="$_mika2542_exc;"
done
for _mika2542_exc in "${MIKA2542_CEILING_LABEL_EXCEPTIONS[@]+"${MIKA2542_CEILING_LABEL_EXCEPTIONS[@]}"}"; do
    sed -n '/^PILOT_LABEL_TURN_CEILINGS=(/,/^)/p' "$DISPATCH_LIB" | grep -qF -- "\"$_mika2542_exc=" \
        || _mika2542_stale+="$_mika2542_exc;"
done
assert_eq "mika#2542: aucune entrée d'allowlist n'est périmée" "" "$_mika2542_stale"

# G1 (table ↔ labels.yml) vit dans scripts/check-pilot-turn-ceiling-labels.sh,
# branché en CI (`pilot-turn-ceiling-labels-lint`). On le rejoue ici pour que
# `make test-dispatch-lib` ne puisse pas être vert avec une table non déclarée.
assert_eq "mika#2542 (AC7): G1 — chaque clé de la table est déclarée dans labels.yml" "0" \
    "$(bash "$(dirname "$DISPATCH_LIB")/../../../scripts/check-pilot-turn-ceiling-labels.sh" >/dev/null 2>&1; echo $?)"

# --- mika#2296: un `.content` vide et un `session_id` absent ne se lisent plus pareil ---
#
# Avant ce ticket, les deux échouaient sur un seul message nommant un champ JSON
# manquant. La cause mesurée du premier est un budget de sortie épuisé par le
# raisonnement du modèle : le fournisseur répond 200, l'enveloppe est bien
# formée, et son `content` est authentiquement vide. Lire « champ manquant » là
# où il y a « réponse vide » a coûté trois essais.

_mika2296_empty_content_reason() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        _groom_warn_empty_content "first-pass" >/dev/null 2>&1
        printf '%s' "$GROOM_LOOP_FAILURE_REASON"
    )
}
MIKA2296_EMPTY=$(_mika2296_empty_content_reason) || MIKA2296_EMPTY=""

assert_contains "mika#2296: le message du content vide dit que la réponse est VIDE" \
    "EMPTY .content" "$MIKA2296_EMPTY"
assert_contains "mika#2296: il nomme la cause probable (budget de raisonnement)" \
    "output budget" "$MIKA2296_EMPTY"
assert_contains "mika#2296: il nomme le grep serveur qui confirme (AC3/AC4)" \
    "llm_reasoning_budget_exhausted" "$MIKA2296_EMPTY"
assert_contains "mika#2296: il nomme la remédiation" \
    "llm_max_tokens" "$MIKA2296_EMPTY"
# Le point du ticket : il ne se lit plus comme une enveloppe incomplète.
assert_not_contains "mika#2296: il n'accuse plus un champ JSON manquant" \
    "missing .content" "$MIKA2296_EMPTY"

# La garde de première passe teste désormais les deux conditions séparément —
# sans quoi les deux messages existeraient sans que personne ne puisse les voir.
MIKA2296_GUARD=$(awk '/first-pass response missing \.metadata\.session_id/{found=1} END{print found+0}' "$DISPATCH_LIB")
assert_eq "mika#2296: la garde session_id a son propre message" \
    "1" "$MIKA2296_GUARD"
assert_contains "mika#2296: le message session_id se démarque du cas content vide" \
    "NOT the mika#2296 empty-content case" "$(grep -A 2 'first-pass response missing \.metadata\.session_id' "$DISPATCH_LIB")"

# Les trois passes architecte qui lisent un `.content` passent par le même
# message : n'en corriger qu'une laisserait la même énigme aux deux autres.
MIKA2296_CALLSITES=$(grep -c '_groom_warn_empty_content "' "$DISPATCH_LIB" || true)
assert_eq "mika#2296: les trois sites de lecture de .content partagent le message" \
    "3" "$MIKA2296_CALLSITES"

# ============================================================================
# mika#2363 — `_arch_ask` déclare la passe qu'il exécute
# ============================================================================
#
# mika-arch porte trois skills `always_on` (39 798 o de prompt) et un tour n'en
# exécute qu'une : ~23,5 Ko de chaque prompt système architecte décrivaient deux
# tâches que le tour ne fait pas. `--only-skill` est le canal — strictement
# soustractif — qui le dit au serveur.
#
# Test comportemental : on éclipse `mika` par une fonction shell qui rend son
# argv, puis on appelle le vrai `_arch_ask`. Une assertion sur le texte du
# fichier prouverait que la ligne existe ; celle-ci prouve ce que la commande
# *émet*.

echo ""
echo "Test: _arch_ask — déclaration de passe (mika#2363)"
echo "---------------------------------------------------"

# _arch_ask_argv <skill> [session_id] → l'argv que `_arch_ask` remet à `mika`.
_arch_ask_argv() {
    local skill="$1" session_id="${2:-}"
    local tmp plan
    tmp=$(mktemp -d)
    plan="$tmp/plan.md"
    echo "# Plan: fixture mika#2363" > "$plan"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        # Éclipse le binaire : une fonction shell l'emporte sur $PATH.
        mika() { printf '%s\n' "$*"; }
        _arch_ask "$skill" "$plan" "$session_id"
    )
    rm -rf "$tmp"
}

# Idem, mais à travers le wrapper de retry (mika#2278). Sert à prouver que le
# wrapper transmet ses trois arguments verbatim : les gardes mika#2305 portent
# désormais sur des appels au wrapper, et sans cette assertion elles
# épingleraient une session qui pourrait ne jamais atteindre `mika ask`.
_arch_ask_argv_via_wrapper() {
    local skill="$1" session_id="${2:-}"
    local tmp plan
    tmp=$(mktemp -d)
    plan="$tmp/plan.md"
    echo "# Plan: fixture mika#2278" > "$plan"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        mika() { printf '%s\n' "$*"; }
        _arch_ask_with_retry "$skill" "$plan" "$session_id"
    )
    rm -rf "$tmp"
}

# Les trois passes, une par une. Le test échouerait aussi bien sur une skill
# oubliée que sur un `--only-skill` retiré.
for _MIKA2363_SKILL in mika-arch-groom-ticket mika-arch-second-review mika-arch-groom-milestone; do
    _MIKA2363_ARGV=$(_arch_ask_argv "$_MIKA2363_SKILL")
    assert_contains "mika#2363: _arch_ask déclare $_MIKA2363_SKILL" \
        "--only-skill $_MIKA2363_SKILL" "$_MIKA2363_ARGV"

    # `--enable-skill` et `--only-skill` sont exclusifs côté clap : les garder
    # tous les deux ferait mourir chaque appel architecte au parsing d'arguments.
    assert_not_contains "mika#2363: $_MIKA2363_SKILL — plus de --enable-skill (exclusif de --only-skill)" \
        "--enable-skill" "$_MIKA2363_ARGV"

    # La passe demandée est la seule nommée. Le vocabulaire des trois skills
    # vit dans MIKA_ARCH_SKILL_ALLOWLIST (well_known_agents.rs) ; une seconde
    # copie en shell dériverait en silence.
    for _MIKA2363_SISTER in mika-arch-groom-ticket mika-arch-second-review mika-arch-groom-milestone; do
        [ "$_MIKA2363_SISTER" = "$_MIKA2363_SKILL" ] && continue
        assert_not_contains "mika#2363: $_MIKA2363_SKILL n'énumère pas $_MIKA2363_SISTER" \
            "$_MIKA2363_SISTER" "$_MIKA2363_ARGV"
    done
done

# La continuation de session reste orthogonale à la déclaration de passe : la
# seconde passe relit la première dans la même session architecte.
_MIKA2363_ARGV_SESSION=$(_arch_ask_argv "mika-arch-second-review" "sess-2363")
assert_contains "mika#2363: --session-id survit à la déclaration de passe" \
    "--session-id sess-2363" "$_MIKA2363_ARGV_SESSION"
assert_contains "mika#2363: --only-skill survit à --session-id" \
    "--only-skill mika-arch-second-review" "$_MIKA2363_ARGV_SESSION"

# =============================================================================
# mika#2305 — le contrat de session des passes architecte, épinglé
# =============================================================================
#
# Le ticket soupçonnait une FUITE : « le contexte de mika-arch semble porté d'une
# passe à l'autre SANS --session-id explicite ». La lecture du code dit l'inverse
# sur cet axe — le portage intra-invocation est explicite, délibéré et commenté,
# et c'est même le contrat que la « Piste » du ticket appelle de ses vœux. La
# fuite réelle vivait dans l'assembleur de prompt (HistoryScope::Agent, refermée
# par mika#2295 + mika#2330), pas ici.
#
# D'où ces assertions : elles n'ajoutent aucun comportement, elles empêchent
# qu'une prochaine lecture « corrige » le portage en croyant fermer #2305. Le
# retry UNPARSED en particulier ne re-demande pas la revue — il demande à
# l'architecte de COMPLÉTER sa propre réponse en y ajoutant la ligne
# `Disposition:` manquante. Sans la session, la demande serait inintelligible :
# le portage y est la condition de correction du mécanisme, pas sa contamination.

# --- Moitié 1 : _arch_ask ne pose --session-id que si $3 est non vide ---

_MIKA2305_ARGV_FRESH=$(_arch_ask_argv "mika-arch-groom-ticket")
assert_not_contains "mika#2305: session neuve par défaut (pas de \$3 → pas de --session-id)" \
    "--session-id" "$_MIKA2305_ARGV_FRESH"

_MIKA2305_ARGV_CONT=$(_arch_ask_argv "mika-arch-groom-ticket" "sess-2305")
assert_contains "mika#2305: continuation sur demande explicite (\$3 → --session-id)" \
    "--session-id sess-2305" "$_MIKA2305_ARGV_CONT"

# --- Moitié 2 : les trois usages de _iterate_groom_loop ---
#
# `_iterate_groom_loop` ne s'exécute pas en isolation (il veut gh, un plan, un
# ticket), donc le contrat est lu sur la source. Le prédicat porte sur la
# PRÉSENCE d'un troisième argument à l'appel, ce qui est exactement ce que
# `_arch_ask` teste à la ligne 4688.

_MIKA2305_BODY=$(awk '/^_iterate_groom_loop\(\) \{/,/^\}/' "$DISPATCH_LIB")
assert_contains "mika#2305: le corps de _iterate_groom_loop est lisible" \
    "_arch_ask" "$_MIKA2305_BODY"

# mika#2278 a interposé `_arch_ask_with_retry` entre la boucle et `_arch_ask` :
# le prédicat porte donc sur `_arch_ask*` et non plus sur `_arch_ask` seul. Le
# contrat épinglé, lui, est inchangé — c'est le TROISIÈME argument qui compte,
# pas le nom de la fonction. Le wrapper transmet `$3` verbatim (couvert par le
# test de sa propre section), donc porter la session à son niveau la porte
# jusqu'à `mika ask`.
_MIKA2305_ANY_CALL='_arch_ask\(_with_retry\)\? "'

# 1ʳᵉ passe : session neuve. C'est le SEUL appel architecte de la boucle qui ne
# nomme pas "$session_id" — d'où un prédicat sur l'absence plutôt que sur la
# forme exacte de la fin de ligne.
_MIKA2305_CALLS=$(printf '%s\n' "$_MIKA2305_BODY" | grep "$_MIKA2305_ANY_CALL" || true)
_MIKA2305_FRESH_CALLS=$(printf '%s\n' "$_MIKA2305_CALLS" | grep -v '"\$session_id"' || true)
assert_eq "mika#2305: un seul appel architecte sans session portée" \
    "1" "$(printf '%s\n' "$_MIKA2305_FRESH_CALLS" | grep -c "$_MIKA2305_ANY_CALL" || true)"
assert_contains "mika#2305: et c'est la 1ʳᵉ passe groom-ticket sur le plan" \
    '_arch_ask_with_retry "mika-arch-groom-ticket" "$plan_path"' "$_MIKA2305_FRESH_CALLS"

# Retry UNPARSED : session portée, pour que l'architecte voie sa propre réponse
# et puisse la compléter.
_MIKA2305_RETRY=$(printf '%s\n' "$_MIKA2305_BODY" \
    | grep -c '_arch_ask_with_retry "mika-arch-groom-ticket" "\$retry_prompt" "\$session_id"' || true)
assert_eq "mika#2305: le retry UNPARSED continue la session (D6)" \
    "1" "$_MIKA2305_RETRY"

# 2ᵉ passe : session portée, sur les deux branches (après READY et après ITERATE).
# Le contrat de continuité de session est déclaré par mika-arch-second-review.
_MIKA2305_SECOND=$(printf '%s\n' "$_MIKA2305_BODY" \
    | grep -c '_arch_ask_with_retry "mika-arch-second-review" "\$plan_path" "\$session_id"' || true)
assert_eq "mika#2305: les deux branches de 2ᵉ passe continuent la session" \
    "2" "$_MIKA2305_SECOND"

# Retry UNPARSED de SECONDE passe (mika#2641) : session portée, sur les deux
# branches. LA DÉCISION, prise ici parce que le bloc ci-dessous l'exige — « un
# cinquième appel est un halt-and-surface ; il porte ou ne porte pas la session,
# et c'est une décision ».
#
# Elle porte la session, et pour la raison EXACTE déjà écrite pour le retry de
# première passe : la relance ne re-demande pas la revue, elle demande à
# l'architecte de COMPLÉTER sa propre réponse en y ajoutant la ligne `Verdict:`
# manquante. Sans la session la demande serait inintelligible — le portage y est
# la condition de correction du mécanisme, pas sa contamination. C'est aussi ce
# que le contrat de continuité de `mika-arch-second-review` exige.
#
# Le prédicat ne nomme pas le fichier temporaire ($retry2 / $retry2i) mais la
# POSITION du troisième argument : ce qui est épinglé est le portage, pas le nom
# de la variable locale.
_MIKA2641_SECOND_RETRY=$(printf '%s\n' "$_MIKA2305_BODY" \
    | grep -cE '_arch_ask_with_retry "mika-arch-second-review" "\$retry2i?" "\$session_id"' || true)
assert_eq "mika#2641: les deux relances de 2ᵉ passe continuent la session (D6)" \
    "2" "$_MIKA2641_SECOND_RETRY"

# Aucun appel architecte n'échappe à l'inventaire ci-dessus : 1 + 1 + 2 + 2 = 6.
# Un septième appel est un halt-and-surface — il porte ou ne porte pas la
# session, et c'est une décision, pas une ligne à ajouter au compte.
_MIKA2305_TOTAL=$(printf '%s\n' "$_MIKA2305_BODY" | grep -c "$_MIKA2305_ANY_CALL" || true)
assert_eq "mika#2305: inventaire clos des appels _arch_ask (1 neuf + 5 continués)" \
    "6" "$_MIKA2305_TOTAL"

# mika#2278 : la boucle passe TOUJOURS par le wrapper. Un appel direct à
# `_arch_ask` y serait un site sans retry et sans capture stderr — exactement
# les deux défauts composés que ce ticket ferme, réintroduits sur un seul site
# pendant que les trois autres restent corrects.
_MIKA2305_BARE=$(printf '%s\n' "$_MIKA2305_BODY" | grep -c '_arch_ask "' || true)
assert_eq "mika#2278: aucun appel direct à _arch_ask ne subsiste dans la boucle" \
    "0" "$_MIKA2305_BARE"

# Et le wrapper transmet le troisième argument verbatim : sans ça, les
# assertions ci-dessus épingleraient une session qui n'atteint pas `mika ask`.
_MIKA2278_WRAPPER_FRESH=$(_arch_ask_argv_via_wrapper "mika-arch-groom-ticket")
assert_not_contains "mika#2278: le wrapper préserve la session neuve" \
    "--session-id" "$_MIKA2278_WRAPPER_FRESH"
_MIKA2278_WRAPPER_CONT=$(_arch_ask_argv_via_wrapper "mika-arch-second-review" "sess-2278")
assert_contains "mika#2278: le wrapper transmet la session continuée" \
    "--session-id sess-2278" "$_MIKA2278_WRAPPER_CONT"

# ============================================================================
# mika#2278 — un brief architecte tué par un restart est ré-émis
# ============================================================================
#
# Mesuré le 2026-09-10 sur le groom de #2276 : un restart de `mika-spirit` a tué
# la passe architecte en vol, et rien ne l'a reprise. `mika ask` sort désormais
# en 75 (EX_TEMPFAIL) sur un échec de classe transport, et `_arch_ask_with_retry`
# réessaie une fois — et UNE SEULE FOIS, et SEULEMENT sur 75.
#
# Test comportemental : on éclipse `mika` par une fonction shell qui compte ses
# invocations et rend une séquence de codes imposée, puis on appelle le vrai
# `_arch_ask_with_retry`. Le compte d'invocations est ce qui porte la preuve :
# une assertion sur le seul code de sortie final passerait aussi bien sur un
# wrapper qui ne réessaie jamais.

echo ""
echo "Test: _arch_ask_with_retry — retry borné sur 75 (mika#2278)"
echo "------------------------------------------------------------"

# _mika2278_probe <codes-séparés-par-virgule> <MIKA_ARCH_ASK_RETRY>
# → "status=<code final> calls=<nombre d'invocations de mika>"
_mika2278_probe() {
    local codes="$1" retry_flag="$2"
    local tmp plan calls
    tmp=$(mktemp -d)
    plan="$tmp/plan.md"
    echo "# Plan: fixture mika#2278" > "$plan"
    calls="$tmp/calls"
    printf '0' > "$calls"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB"
        # Le délai réel est de 30 s ; la sonde n'en mesure pas la valeur, elle
        # mesure le nombre d'appels. 1 s garde la suite rapide sans changer ce
        # qui est asserté.
        export MIKA_ARCH_ASK_RETRY_DELAY_SECS=1
        export MIKA_ARCH_ASK_RETRY="$retry_flag"
        # Éclipse le binaire : une fonction shell l'emporte sur $PATH.
        mika() {
            local n code
            n=$(cat "$calls")
            n=$((n + 1))
            printf '%s' "$n" > "$calls"
            code=$(printf '%s' "$codes" | cut -d, -f"$n")
            [ -n "$code" ] || code=0
            if [ "$code" -eq 0 ]; then
                printf '{"content":"ok","metadata":{"session_id":"s-2278"}}'
            else
                echo "mika ask to http://127.0.0.1:8080/a2a/mika-arch failed: unreachable" >&2
            fi
            return "$code"
        }
        local out status
        if out=$(_arch_ask_with_retry "mika-arch-groom-ticket" "$plan" 2>/dev/null); then
            status=0
        else
            status=$?
        fi
        printf 'status=%s calls=%s out=%s' "$status" "$(cat "$calls")" "$out"
    )
    rm -rf "$tmp"
}

# Cas positif : un 75 puis un 0 → deux invocations, succès final, stdout intact.
_MIKA2278_RETRIED=$(_mika2278_probe "75,0" "1")
assert_contains "mika#2278: un 75 est réessayé exactement une fois" \
    "calls=2" "$_MIKA2278_RETRIED"
assert_contains "mika#2278: et le retry aboutit (statut final 0)" \
    "status=0" "$_MIKA2278_RETRIED"
assert_contains "mika#2278: le stdout du retry remonte intact à l'appelant" \
    'out={"content":"ok"' "$_MIKA2278_RETRIED"

# Contrôle négatif — c'est celui qui porte la preuve. Sans lui, un wrapper qui
# réessaierait sur n'importe quel code non nul passerait le cas positif.
_MIKA2278_CONTRACT=$(_mika2278_probe "1,0" "1")
assert_contains "mika#2278: un échec de contrat (1) n'est JAMAIS réessayé" \
    "calls=1" "$_MIKA2278_CONTRACT"
assert_contains "mika#2278: et son code de sortie est propagé tel quel" \
    "status=1" "$_MIKA2278_CONTRACT"

# Fail-safe D4 : un code qu'on ne sait pas lire est traité comme définitif.
# L'inverse ferait d'un futur mode d'échec imprévu une boucle de réessais.
_MIKA2278_UNKNOWN=$(_mika2278_probe "2,0" "1")
assert_contains "mika#2278: un code inconnu (2) n'est pas retryable (D4)" \
    "calls=1" "$_MIKA2278_UNKNOWN"

# Contrôle négatif de désarmement (R7) : la variable doit mordre.
_MIKA2278_DISARMED=$(_mika2278_probe "75,0" "0")
assert_contains "mika#2278: MIKA_ARCH_ASK_RETRY=0 désarme le retry" \
    "calls=1" "$_MIKA2278_DISARMED"
assert_contains "mika#2278: et le 75 est alors propagé à l'appelant" \
    "status=75" "$_MIKA2278_DISARMED"

# Budget de UN : deux 75 d'affilée ne produisent pas un troisième appel.
_MIKA2278_EXHAUSTED=$(_mika2278_probe "75,75,0" "1")
assert_contains "mika#2278: le budget est de un retry, pas d'une boucle" \
    "calls=2" "$_MIKA2278_EXHAUSTED"
assert_contains "mika#2278: budget épuisé → le 75 est propagé" \
    "status=75" "$_MIKA2278_EXHAUSTED"

# Lecteurs d'environnement, trois paliers (R4).
_MIKA2278_DELAY_DEFAULT=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; unset MIKA_ARCH_ASK_RETRY_DELAY_SECS; _arch_ask_retry_delay_secs
)
assert_eq "mika#2278: délai par défaut = 30 s" "30" "$_MIKA2278_DELAY_DEFAULT"
_MIKA2278_DELAY_SET=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; MIKA_ARCH_ASK_RETRY_DELAY_SECS=45 _arch_ask_retry_delay_secs
)
assert_eq "mika#2278: délai réglé honoré" "45" "$_MIKA2278_DELAY_SET"
# `0` ne désarme PAS — c'est le rôle du kill-switch ; lu autrement, une coquille
# sur le délai restaurerait en silence le défaut que ce ticket ferme.
_MIKA2278_DELAY_ZERO=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; MIKA_ARCH_ASK_RETRY_DELAY_SECS=0 _arch_ask_retry_delay_secs 2>/dev/null
)
assert_eq "mika#2278: délai 0 retombe sur le défaut (ne désarme pas)" \
    "30" "$_MIKA2278_DELAY_ZERO"
_MIKA2278_DELAY_ABSURD=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; MIKA_ARCH_ASK_RETRY_DELAY_SECS=99999 _arch_ask_retry_delay_secs 2>/dev/null
)
assert_eq "mika#2278: délai absurde borné par le haut" "30" "$_MIKA2278_DELAY_ABSURD"

# Garde structurelle : le retour d'un `2>/dev/null` sur un appel architecte.
#
# Un test comportemental ne peut pas voir cette classe. La régression ne rendrait
# aucune décision fausse — le retry continuerait de fonctionner, toutes les
# assertions ci-dessus resteraient vertes — elle rendrait le diagnostic aveugle :
# l'opérateur relirait `first-pass _arch_ask failed` sans jamais apprendre
# pourquoi. Même motif que mika#2131.
_MIKA2278_LOOP_BODY=$(awk '/^_iterate_groom_loop\(\) \{/,/^\}/' "$DISPATCH_LIB")
_MIKA2278_SWALLOWED=$(printf '%s\n' "$_MIKA2278_LOOP_BODY" \
    | grep -c '_arch_ask[a-z_]* .*2>/dev/null' || true)
assert_eq "mika#2278: aucun appel architecte ne jette plus stderr (D7)" \
    "0" "$_MIKA2278_SWALLOWED"

# Et les SIX sites font remonter ce message dans leur WARN d'échec (R5).
#
# mika#2641 — le compte passe de 4 à 6 : les deux relances de seconde passe sont
# deux appels architecte de plus, donc deux sites d'échec de plus, et l'invariant
# R5 s'applique à eux à l'identique. Un WARN de relance qui ne porterait pas le
# message du CLI laisserait l'opérateur relire « second-pass retry _arch_ask
# failed » sans jamais apprendre pourquoi — exactement la cécité que ce test
# existe pour refuser.
_MIKA2278_SUFFIXED=$(printf '%s\n' "$_MIKA2278_LOOP_BODY" \
    | grep -c '_groom_warn ".*_arch_ask failed.*_arch_ask_error_suffix' || true)
assert_eq "mika#2278: les six WARN d'échec portent le message du CLI (R5)" \
    "6" "$_MIKA2278_SUFFIXED"

# ===========================================================================
# mika#2522 — les deux moitiés du code de sortie retryable sont le même nombre
# ===========================================================================
#
# Le retry de mika#2278 ne rejoue que sur `75`, et mika#2522 fait arriver un `75`
# là où arrivait un `1` : la classe `transport` attestée par le serveur devient
# `FailureClass::Transport` côté CLI, donc `EXIT_TRANSPORT_FAILURE`. Ce ticket ne
# touche PAS `dispatch-lib.sh` — tout ce que le shell doit garantir, c'est que la
# constante qu'il lit est bien celle que le CLI écrit.
#
# Le côté Rust épingle déjà le littéral (`assert_eq!(EXIT_TRANSPORT_FAILURE, 75)`
# dans `remote_ask.rs`). Aucun des deux tests ne voit l'autre : deux moitiés
# épinglées séparément sur deux littéraux peuvent diverger en silence, et le seul
# symptôme serait un retry qui cesse de s'armer — c'est-à-dire exactement le
# défaut que mika#2522 ferme, revenu par la porte de derrière. Cette assertion est
# la jointure.
_MIKA2522_REMOTE_ASK_RS="$SCRIPT_DIR/../../../crates/mika-cli/src/remote_ask.rs"
_MIKA2522_CLI_EXIT=$(grep -oE 'pub const EXIT_TRANSPORT_FAILURE: i32 = [0-9]+' \
    "$_MIKA2522_REMOTE_ASK_RS" 2>/dev/null | grep -oE '[0-9]+$' || true)
# Anti-vacuité : source déplacée, constante renommée, `grep` qui ne rend rien —
# chacun donnerait une chaîne vide, et comparer deux vides passerait. Un prédicat
# qui ne lit rien se lit exactement comme un prédicat sain (classe mika#2205).
assert_eq "mika#2522: la constante du CLI a bien été extraite de sa source" \
    "extracted" "$([ -n "$_MIKA2522_CLI_EXIT" ] && echo extracted || echo empty)"
_MIKA2522_SHELL_EXIT=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB"; printf '%s' "$_ARCH_ASK_RETRYABLE_EXIT"
)
assert_eq "mika#2522: _ARCH_ASK_RETRYABLE_EXIT == EXIT_TRANSPORT_FAILURE du CLI" \
    "$_MIKA2522_CLI_EXIT" "$_MIKA2522_SHELL_EXIT"

# ===========================================================================
# mika#1943 — un chemin qu'on ne peut pas prouver worktree n'est pas supprimé
# ===========================================================================
#
# L'incident fondateur (28/07) est une suppression automatisée qui a emporté
# `/data/workspace/bbytaa`, un répertoire qui n'était protégé par aucune liste —
# il était protégé par btrbk. La garde posée ici est une **allowlist positive**
# alignée terme pour terme sur `worktree_reaper::is_managed_worktree_path`
# (mika#2420) : un chemin doit *prouver* qu'il est un worktree géré, au lieu
# d'être absent d'une denylist. Une denylist est fausse le jour où un répertoire
# précieux n'y figure pas — c'est-à-dire le jour où elle servirait.
#
# L'asymétrie qui décide de tout : un faux négatif laisse un worktree résiduel
# sur le disque (le reaper mika#2420 le ramasse au tick suivant, coût borné) ;
# un faux positif supprime un répertoire qui n'est pas un worktree, et c'est
# irréversible. Donc tout terme illisible conserve.

echo ""
echo "Test: garde de suppression — allowlist worktree (mika#1943)"
echo "------------------------------------------------------------"

_MIKA1943_MANAGED_ROOT='/data/workspace/mika-platform/.claude/worktrees'

# Rend REFUSED / ACCEPTED, et sur un refus le jeton `term=` émis sur stderr.
_mika1943_probe() {
    local lib="$1" path="$2" err rc
    err=$(
        # shellcheck disable=SC1090
        source "$lib" 2>/dev/null || true
        _assert_removable_worktree_path "$path" probe 2>&1 >/dev/null
        printf '\nRC=%s' "$?"
    )
    rc=${err##*RC=}
    if [ "$rc" = "0" ]; then
        printf 'ACCEPTED'
        return 0
    fi
    # Extraction du jeton `term=` par expansion bash, sans pipe : un
    # `sed … | head -1` sous `set -o pipefail` (en tête de ce fichier) rend 141
    # dès que `head` ferme le tuyau avant la fin de `sed`. Ça ne se produit pas
    # sur une ligne courte, donc ça passerait — et rougirait le jour où le
    # message du refus s'allonge, pour une raison sans rapport avec ce qu'il
    # teste.
    local term=""
    case "$err" in *term=*) term=${err#*term=}; term=${term%% *} ;; esac
    printf 'REFUSED:%s' "$term"
}

# --- Contrôles positifs : chaque chemin est refusé, et par le bon terme ------

assert_eq "mika#1943: chaîne vide refusée (la racine du défaut)" \
    "REFUSED:empty" "$(_mika1943_probe "$DISPATCH_LIB" "")"
assert_eq "mika#1943: /data/workspace/foo refusé (fixture de l'AC2)" \
    "REFUSED:outside_managed_root" "$(_mika1943_probe "$DISPATCH_LIB" "/data/workspace/foo")"
assert_eq "mika#1943: /data/workspace/bbytaa refusé (le répertoire de l'incident)" \
    "REFUSED:outside_managed_root" "$(_mika1943_probe "$DISPATCH_LIB" "/data/workspace/bbytaa")"
assert_eq "mika#1943: chemin relatif refusé" \
    "REFUSED:not_absolute" "$(_mika1943_probe "$DISPATCH_LIB" "relatif/x")"
assert_eq "mika#1943: traversée par .. refusée malgré le segment gardé" \
    "REFUSED:parent_dir_component" \
    "$(_mika1943_probe "$DISPATCH_LIB" "$_MIKA1943_MANAGED_ROOT/../../../etc")"
assert_eq "mika#1943: / refusé" \
    "REFUSED:outside_managed_root" "$(_mika1943_probe "$DISPATCH_LIB" "/")"

# --- Contrôle négatif : c'est lui qui donne un sens aux positifs -------------
#
# Doctrine mika#2420 : sans ce contrôle, une fonction qui refuse *tout* passerait
# la suite en vert tout en cassant chaque dispatch.
assert_eq "mika#1943: un vrai worktree de dispatch est accepté (contrôle négatif)" \
    "ACCEPTED" \
    "$(_mika1943_probe "$DISPATCH_LIB" "$_MIKA1943_MANAGED_ROOT/chore-1943-platform-hygiene-re-file-p0-28-07/mika")"

# --- Le refus est DIT ------------------------------------------------------
#
# Sans cette ligne, un refus se lirait exactement comme une absence de travail
# (classe mika#2205) : le dispatch continuerait, le worktree résiduel resterait,
# et rien dans les journaux ne nommerait la cause.
_MIKA1943_REFUSAL_STDERR=$(
    # shellcheck disable=SC1090
    source "$DISPATCH_LIB" 2>/dev/null || true
    _assert_removable_worktree_path "/data/workspace/bbytaa" site_probe 2>&1 >/dev/null || true
)
assert_contains "mika#1943: le refus émet l'événement nommé" \
    "dispatch_lib_unsafe_removal_refused" "$_MIKA1943_REFUSAL_STDERR"
assert_contains "mika#1943: le refus nomme le chemin" \
    "/data/workspace/bbytaa" "$_MIKA1943_REFUSAL_STDERR"
assert_contains "mika#1943: le refus nomme le terme qui a échoué" \
    "term=outside_managed_root" "$_MIKA1943_REFUSAL_STDERR"
assert_contains "mika#1943: le refus nomme le site appelant" \
    "site=site_probe" "$_MIKA1943_REFUSAL_STDERR"

# --- Vérification par INJECTION, exigée par l'AC2 ---------------------------
#
# Neutraliser les quatre termes d'un coup ne prouverait rien : une conjonction ne
# se teste pas en désarmant tous ses termes ensemble (leçon mika#2277). Chaque
# terme porte donc un marqueur de fin de ligne `# mika1943:T<n>`, et l'injection
# remplace **une** ligne à la fois par un no-op.
#
# `MUTATION_ABSENTE` est le garde-fou du garde-fou : si quelqu'un renomme un
# marqueur ou fusionne deux termes, le `sed` ne mord plus et le test passerait au
# vert en n'ayant rien vérifié.
_mika1943_inject() {
    local term="$1" path="$2" tmp
    tmp=$(mktemp)
    sed "s|^.*# mika1943:${term}\$|    :|" "$DISPATCH_LIB" >"$tmp"
    if cmp -s "$DISPATCH_LIB" "$tmp"; then
        rm -f "$tmp"
        printf 'MUTATION_ABSENTE'
        return 0
    fi
    _mika1943_probe "$tmp" "$path"
    rm -f "$tmp"
}

# T4 — LE terme de l'AC2. Le tuer laisse passer le répertoire de l'incident.
assert_eq "mika#1943 injection T4: sans l'allowlist, /data/workspace/bbytaa passe" \
    "ACCEPTED" "$(_mika1943_inject T4 "/data/workspace/bbytaa")"

# T3 — sans lui, le segment gardé peut être traversé jusqu'à n'importe quoi.
assert_eq "mika#1943 injection T3: sans le refus de .., la traversée passe" \
    "ACCEPTED" "$(_mika1943_inject T3 "$_MIKA1943_MANAGED_ROOT/../../../etc")"

# T2 — sans lui, un chemin relatif portant le segment passe. `git -C` et `rm -rf`
# le résoudraient depuis le cwd du moment, qui n'est pas une garantie.
assert_eq "mika#1943 injection T2: sans l'exigence d'absolu, le relatif passe" \
    "ACCEPTED" "$(_mika1943_inject T2 "mika-platform/.claude/worktrees/foo/mika")"

# T1 — cas particulier, et il faut dire pourquoi il ne rend pas ACCEPTED.
# La chaîne vide est aussi non-absolue : T2 la rattrape. Ce que T1 porte n'est
# donc pas la sûreté mais le **diagnostic** — et c'est exactement la racine de
# mika#1943, une `WORKTREE_DIR` vide propagée en silence. La nommer
# `not_absolute` enverrait l'opérateur chercher une faute de frappe de chemin
# au lieu d'une dérivation qui a échoué.
assert_eq "mika#1943 injection T1: sans lui, le vide est mal nommé (T2 le rattrape)" \
    "REFUSED:not_absolute" "$(_mika1943_inject T1 "")"

# --- Lecteur unique --------------------------------------------------------
#
# Cinq sites destructifs, donc cinq occasions de diverger. C'est la leçon que
# `grooming_marker` (mika#2158) a dû engraver une fois dans ce dépôt : deux
# prédicats répondant différemment à « ce chemin est-il supprimable ».
_MIKA1943_GUARD_BODY=$(awk '/^_assert_removable_worktree_path\(\) \{/,/^\}/' "$DISPATCH_LIB")
_MIKA1943_MARKERS_TOTAL=$(grep -c '# mika1943:T[0-9]$' "$DISPATCH_LIB" || true)
_MIKA1943_MARKERS_IN_GUARD=$(printf '%s\n' "$_MIKA1943_GUARD_BODY" | grep -c '# mika1943:T[0-9]$' || true)
assert_eq "mika#1943: quatre termes, et pas un de plus" "4" "$_MIKA1943_MARKERS_TOTAL"
assert_eq "mika#1943: les quatre termes vivent dans la seule fonction qui décide" \
    "4" "$_MIKA1943_MARKERS_IN_GUARD"

# --- U2: la racine est fermée ----------------------------------------------
#
# `derive-worktree-path` vit dans mika-platform et le fichier n'a ni `set -e` ni
# `set -u`. Sans vérification, un script absent ou en échec rend une chaîne vide
# qui se propage en silence jusqu'aux sites de suppression — et jusqu'à la
# comparaison d'égalité qui ÉLIT une cible.
_MIKA1943_SETUP_BODY=$(awk '/^_set_up_worktree\(\) \{/,/^\}/' "$DISPATCH_LIB")
assert_contains "mika#1943 U2: l'échec de dérivation est nommé et abandonne le dispatch" \
    "worktree_path_derivation_failed" "$_MIKA1943_SETUP_BODY"
assert_contains "mika#1943 U2: la comparaison d'élection exige une cible non vide" \
    '[ -n "$WORKTREE_DIR" ] && [ -n "$existing_wt" ]' "$_MIKA1943_SETUP_BODY"

# --- U2: les cinq sites destructifs appellent la garde ----------------------
#
# Le plan en nomme quatre plus la racine ; `_handle_dry_run` est le cinquième,
# trouvé à la lecture. L'invariant du Product Contract porte sur *tout* site de
# suppression, pas sur la liste énumérée.
_mika1943_guard_calls_in() {
    awk -v fn="^$1\\\\(\\\\) \\\\{" '$0 ~ fn, /^\}/' "$DISPATCH_LIB" \
        | grep -c '_assert_removable_worktree_path' || true
}
assert_eq "mika#1943 U2: _set_up_worktree garde ses deux suppressions" \
    "2" "$(_mika1943_guard_calls_in _set_up_worktree)"
assert_eq "mika#1943 U2: _handle_dry_run garde la sienne" \
    "1" "$(_mika1943_guard_calls_in _handle_dry_run)"
assert_eq "mika#1943 U2: _clean_worktree_for_rebase garde son rm -rf" \
    "1" "$(_mika1943_guard_calls_in _clean_worktree_for_rebase)"
assert_eq "mika#1943 U2: _cleanup_iterate_findings garde son rm -rf" \
    "1" "$(_mika1943_guard_calls_in _cleanup_iterate_findings)"
assert_eq "mika#1943 U2: _seed_pilot_scratch_dir garde son rm -rf (mika#2548)" \
    "1" "$(_mika1943_guard_calls_in _seed_pilot_scratch_dir)"

# --- U2: la garde est CÂBLÉE au site, et elle n'y casse rien ----------------
#
# Les assertions structurelles ci-dessus prouvent qu'un appel existe, jamais
# qu'il décide. Et sur ce site précis le reset chirurgical est plus porteur
# qu'il n'y paraît : `.iterate/` est gitignored (`.gitignore:53`), donc le
# tier 3 ne le voit pas (`status --porcelain` ignore les fichiers ignorés) et
# `clean -fd` ne le supprime pas (pas de `-x`). Ce `rm -rf` est la SEULE chose
# qui le retire. Une garde trop stricte y ferait s'accumuler les findings d'une
# passe de grooming à l'autre, en silence.
#
# D'où les deux moitiés : un chemin de la forme réelle doit être balayé, un
# chemin non géré doit être épargné ET dit.
_mika1943_clean_probe() {
    local wt="$1" out
    mkdir -p "$wt"
    git -C "$wt" init -q
    printf '.iterate/\n' >"$wt/.gitignore"
    git -C "$wt" add .gitignore
    git -C "$wt" commit -q -m "base"
    mkdir -p "$wt/.iterate"
    printf 'findings\n' >"$wt/.iterate/findings-1.md"
    out=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        LOG_ID="test-1943" _clean_worktree_for_rebase "$wt" 2>&1 >/dev/null
    )
    if [ -d "$wt/.iterate" ]; then printf 'KEPT|%s' "$out"; else printf 'SWEPT|%s' "$out"; fi
}

_MIKA1943_CLEAN_ROOT=$(mktemp -d)
_MIKA1943_CLEAN_MANAGED=$(_mika1943_clean_probe \
    "$_MIKA1943_CLEAN_ROOT/.claude/worktrees/chore-1943-fixture/mika")
_MIKA1943_CLEAN_FOREIGN=$(_mika1943_clean_probe "$_MIKA1943_CLEAN_ROOT/pas-un-worktree")
rm -rf "$_MIKA1943_CLEAN_ROOT"

assert_contains "mika#1943 U2: sur un worktree géré, .iterate est bien balayé (contrôle négatif)" \
    "SWEPT|" "$_MIKA1943_CLEAN_MANAGED"
assert_not_contains "mika#1943 U2: et le balayage nominal n'émet aucun refus" \
    "dispatch_lib_unsafe_removal_refused" "$_MIKA1943_CLEAN_MANAGED"
assert_contains "mika#1943 U2: sur un chemin non géré, .iterate est épargné" \
    "KEPT|" "$_MIKA1943_CLEAN_FOREIGN"
assert_contains "mika#1943 U2: et l'épargne est DITE, pas silencieuse" \
    "term=outside_managed_root" "$_MIKA1943_CLEAN_FOREIGN"

# Et aucun site de suppression n'y échappe : tout `rm -rf` portant une variable
# de worktree, et tout `worktree remove`, sont précédés de la garde. Un test
# comportemental ne peut pas voir cette classe — un sixième site ajouté demain
# ne rendrait aucune décision fausse, il rouvrirait le trou en silence.
#
# Le recensement porte sur les lignes de CODE : compter aussi les commentaires
# ferait passer le test au vert pour la mauvaise raison — le premier jet le
# faisait, et il comptait les commentaires que ce même correctif venait
# d'ajouter.
_MIKA1943_CODE_LINES=$(grep -v '^[[:space:]]*#' "$DISPATCH_LIB" || true)
_MIKA1943_WT_REMOVES=$(printf '%s\n' "$_MIKA1943_CODE_LINES" | grep -c 'worktree remove --force' || true)
assert_eq "mika#1943 U2: exactement trois 'worktree remove --force' recensés" \
    "3" "$_MIKA1943_WT_REMOVES"
_MIKA1943_RMRF_WT=$(printf '%s\n' "$_MIKA1943_CODE_LINES" | grep -cE 'rm -rf "\$(wt|WORKTREE_DIR|findings_dir|scratch)' || true)
assert_eq "mika#1943 U2: exactement trois 'rm -rf' sur un chemin de worktree" \
    "3" "$_MIKA1943_RMRF_WT"

# ============================================================================
# mika#2306 — la section `## Fire-Disposition` a un site de production (T1–T11)
# ============================================================================
#
# Ce que ces onze détecteurs mesurent. Avant mika#2306, aucun canal que CE dépôt
# contrôle ne prescrivait `## Fire-Disposition` : `/ce:plan` est un plugin tiers
# qui ignore mika#1574, les trois commandes de groom vivent dans mika-platform,
# et `grep -n "Fire-Disposition" .claude/commands/*.md` rendait zéro. Un plan
# neuf livrant un détecteur arrivait donc devant mika-arch sans la section, et
# l'unique itération de la boucle était dépensée sur un motif formel — puis le
# gate de seconde passe, sans recours, rendait ESCALATE.
#
# Les deux moitiés sont livrées ensemble à dessein : la prescription (T1–T4)
# ne tiendrait pas seule, et le rattrapage (T5–T11) ne tiendrait pas seul non
# plus. Les quatre contrôles négatifs (T3, T6, T7, T11) sont porteurs, pas
# décoratifs : sans eux, une garde qui relance TOUJOURS passerait T5 en vert
# tout en doublant le coût de chaque grooming du dépôt.

echo ""
echo 'Test: mika#2306 — site de production de `## Fire-Disposition` (T1–T11)'
echo "-----------------------------------------------------------------------"

# --- Outillage local de section ---------------------------------------------

# Réplique le site d'injection de `_set_up_worktree` : contexte de ticket, puis
# règle de corps de PR (inconditionnelle), puis règle Fire-Disposition
# (conditionnée au skill). T2/T3 assertent sur CETTE chaîne ; T4 asserte que
# l'expression source est littéralement celle répliquée ici, donc le miroir ne
# peut pas dériver en silence (patron mika#2178 T3).
_t2306_inject() {
    local skill="$1" repo="${2:-mika}" issue_num="${3:-2306}"
    local prompt="${repo}#${issue_num}"
    prompt=$(printf '%s\n\n%s' "$prompt" "$_PR_BODY_CONTAINMENT_RULE")
    if [ "$skill" = "dev-groom" ]; then
        prompt=$(printf '%s\n\n%s' "$prompt" "$_FIRE_DISPOSITION_RULE")
    fi
    printf '%s' "$prompt"
}

# Sonde comportementale de `_launch_revise_pilot`.
#
# Elle exerce la VRAIE fonction : seul `_run_pilot_sandboxed` est neutralisé
# (par redéfinition locale au sous-shell, jamais par une chaîne écrite à la
# main), ce qui est la seule façon de prouver que le compte de relances vient de
# la garde et de rien d'autre. Chaque invocation du pilote simulé incrémente un
# compteur sur disque et applique la mutation demandée au plan — modifier le
# plan est indispensable, sans quoi `pre_hash == post_hash` et le flot tombe
# dans la branche d'échec avant d'atteindre la garde.
#
# $1 = contenu de .iterate/findings-1.md
# $2 = le plan initial porte-t-il déjà la section ? (yes|no), ou sous quelle
#      forme de titre (mika#2544) : numbered (`## 3.`), upper (`## 3.` en
#      capitales), ish (`## Fire-Disposition-ish`), notes (`## 11. Notes on …`),
#      subsub (`## 1.1 …`)
# $3 = comportement du pilote simulé :
#      never  — ne l'ajoute jamais
#      second — l'ajoute à la deuxième invocation
#      second-numbered — l'ajoute NUMÉROTÉE à la deuxième invocation (mika#2544)
#      wipe   — supprime le findings-file de première passe (fail-safe en vol)
# $4 = (optionnel) "residual-fd" dépose un findings-1-fd.md résiduel
# Rend : "<rc>|<nombre d'invocations du pilote>|<stderr>"
_t2306_revise_probe() {
    local t2306_findings="$1" t2306_has_section="$2" t2306_behaviour="$3" t2306_extra="${4:-}"
    local t2306_root t2306_wt t2306_plan t2306_counter t2306_err t2306_rc
    t2306_root=$(mktemp -d)
    t2306_wt="$t2306_root/.claude/worktrees/fix-2306-probe/mika"
    mkdir -p "$t2306_wt/docs/plans" "$t2306_wt/.iterate"
    t2306_plan="$t2306_wt/docs/plans/2026-09-21-001-fix-2306-sonde-plan.md"

    # > 500 octets : `_find_issue_plan` filtre les plans plus courts (mika#1033).
    # Le remplissage est volontairement large — un plan qui tombe SOUS le seuil
    # fait rendre « no plan file to revise », c'est-à-dire un vert qui n'a rien
    # exercé de la garde. C'est le premier défaut que cette sonde a eu.
    {
        printf '# mika#2306 — plan de sonde\n\n**Ticket :** mika issue#2306\n\n## Problème\n\n'
        printf 'Corps de remplissage pour franchir le filtre des 500 octets applique par\n'
        printf '_find_issue_plan a tous ses tiers de decouverte. Ce texte ne porte aucune\n'
        printf 'signification pour la sonde : seules comptent sa longueur, et la presence\n'
        printf "ou l'absence de la section testee.\n\n"
        printf 'Le seuil existe pour ecarter les fichiers-fantomes et les ebauches vides ;\n'
        printf "il s'applique identiquement aux trois tiers de decouverte, donc un plan\n"
        printf 'trop court est invisible quelle que soit la façon dont il est nomme. Une\n'
        printf 'sonde qui passe sous ce seuil ne mesure rien et le dit mal : elle rend\n'
        printf 'exactement le meme « plan introuvable » que la garde de fonction.\n\n'
        printf '## Implementation Units\n\nU1 — un livrable detecteur quelconque.\n\n'
        # mika#2544 — cinq formes de titre en plus des deux d'origine. La ligne
        # `## Acceptance criteria` qui suit reste HORS du `case` : l'emporter en
        # réécrivant ce bloc casserait tout futur gate AC sur ce fixture.
        case "$t2306_has_section" in
            yes)      printf '## Fire-Disposition\n\nOption (a) — exception nommee, table vide.\n\n' ;;
            numbered) printf '## 3. Fire-Disposition\n\nOption (a) — exception nommee, table vide.\n\n' ;;
            upper)    printf '## 3. FIRE-DISPOSITION\n\nOption (a) — exception nommee, table vide.\n\n' ;;
            ish)      printf '## Fire-Disposition-ish\n\nOption (a) — exception nommee, table vide.\n\n' ;;
            notes)    printf '## 11. Notes on Fire-Disposition\n\nRemarques, pas la section.\n\n' ;;
            subsub)   printf '## 1.1 Fire-Disposition\n\nOption (a) — exception nommee, table vide.\n\n' ;;
        esac
        printf '## Acceptance criteria\n\nAC1 — la sonde tourne.\n'
    } > "$t2306_plan"

    printf '%s\n' "$t2306_findings" > "$t2306_wt/.iterate/findings-1.md"
    if [ "$t2306_extra" = "residual-fd" ]; then
        printf 'Fire-Disposition — residu laisse par un dispatch anterieur.\n' \
            > "$t2306_wt/.iterate/findings-1-fd.md"
    fi

    t2306_counter="$t2306_root/pilot-invocations"
    printf '0\n' > "$t2306_counter"

    # UNE seule exécution : le rc transite par un fichier plutôt que par une
    # seconde passe. Rejouer la fonction avec un pilote inerte donnerait
    # `pre_hash == post_hash`, donc un rc de 1 sans rapport avec ce qui est
    # mesuré — un faux vert sur la moitié fail-safe.
    t2306_err=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        # Neutralisation RÉELLE : seul le lancement du pilote est shadowé, tout
        # le reste de `_launch_revise_pilot` s'exécute tel quel. C'est la seule
        # façon de prouver que le compte de relances vient de la garde.
        _run_pilot_sandboxed() {
            local n; n=$(( $(cat "$t2306_counter") + 1 ))
            printf '%s\n' "$n" > "$t2306_counter"
            case "$t2306_behaviour" in
                wipe) rm -f "$t2306_wt/.iterate/findings-1.md" ;;
                second) [ "$n" -ge 2 ] && printf '\n## Fire-Disposition\n\nOption (a).\n' >> "$t2306_plan" ;;
                second-numbered) [ "$n" -ge 2 ] && printf '\n## 3. Fire-Disposition\n\nOption (a).\n' >> "$t2306_plan" ;;
            esac
            # Mutation inconditionnelle : sans elle le sha ne bouge pas, et la
            # branche de succès — donc la garde — n'est jamais atteinte.
            printf '\n<!-- revise %s -->\n' "$n" >> "$t2306_plan"
            return 0
        }
        set +e
        WORKTREE_DIR="$t2306_wt" ISSUE_NUM="2306" REPO="mika" LOG_ID="t2306" \
            CWD_ARGS="--cwd $t2306_wt" \
            _launch_revise_pilot "$t2306_wt/.iterate/findings-1.md" 2>&1 >/dev/null
        printf '%s\n' "$?" > "$t2306_root/rc"
    )
    t2306_rc=$(cat "$t2306_root/rc")
    printf '%s|%s|%s' "$t2306_rc" "$(cat "$t2306_counter")" "$t2306_err"
    rm -rf "$t2306_root"
}

# Le champ 3 est du stderr libre — multiligne, et pouvant contenir des `|`. Un
# `cut` nu le traiterait ligne par ligne et rendrait, pour les champs 1 et 2,
# les lignes de stderr qui ne portent aucun délimiteur. Deuxième défaut que
# cette sonde a eu, et il produisait des `expected: 1 / actual: 1` illisibles.
_t2306_field() { printf '%s' "$2" | head -1 | cut -d'|' -f"$1"; }
_t2306_err()   { printf '%s' "$1" | sed '1s/^[0-9]*|[0-9]*|//'; }

T2306_FINDINGS_WITH_FD="F1 [BLOQUANT] — le plan livre des tests et ne porte pas de section
\`## Fire-Disposition\`. Ajoute-la en nommant l'une des trois options de mika#1574.

Disposition: ITERATE"
T2306_FINDINGS_WITHOUT_FD="F1 [BLOQUANT] — l'unité U2 laisse le choix de la structure de
données à l'implémenteur. Tranche-le dans le plan.

Disposition: ITERATE"

T2306_LIB_SRC=$(cat "$DISPATCH_LIB")
T2306_SUW_SRC=$(sed -n '/^_set_up_worktree() {/,/^}/p' "$DISPATCH_LIB")
T2306_REVISE_SRC=$(declare -f _launch_revise_pilot)
T2306_GUARD_SRC=$(declare -f _fd_retry_if_section_still_missing)

# --- T1 : la règle existe et dit quoi écrire ---------------------------------
#
# Une règle tronquée qui prescrit la section sans nommer ses options ferait
# inventer une quatrième disposition au groomeur — pire que le silence, parce
# qu'elle passerait le `grep` du gate et échouerait sur son contenu.

assert_eq "T1: _FIRE_DISPOSITION_RULE est définie et non vide" "non-vide" \
    "$([ -n "${_FIRE_DISPOSITION_RULE:-}" ] && echo non-vide || echo vide)"
assert_contains "T1: la règle nomme la section exacte" \
    '## Fire-Disposition' "${_FIRE_DISPOSITION_RULE:-}"
assert_contains "T1: la règle nomme l'option (a)" "(a)" "${_FIRE_DISPOSITION_RULE:-}"
assert_contains "T1: la règle nomme l'option (b)" "(b)" "${_FIRE_DISPOSITION_RULE:-}"
assert_contains "T1: la règle nomme l'option (c)" "(c)" "${_FIRE_DISPOSITION_RULE:-}"
assert_contains "T1: la règle cite la doctrine par référence plutôt que de la reformuler" \
    "mika#1574" "${_FIRE_DISPOSITION_RULE:-}"
# La conditionnalité est la moitié que l'option (a) du gate rend explicite :
# sans elle, un groomeur ajouterait la section à tout plan, ce qui contredit
# « Plan has no detector-class deliverables ⇒ gate is N/A ».
assert_contains "T1: la règle dit que la section n'est PAS requise sans détecteur" \
    "N/A" "${_FIRE_DISPOSITION_RULE:-}"

# --- T2 : la prescription atteint le groomeur --------------------------------

T2306_PROMPT_GROOM=$(_t2306_inject "dev-groom")
assert_contains "T2: la règle est injectée dans le PROMPT pour SKILL=dev-groom" \
    '## Fire-Disposition' "$T2306_PROMPT_GROOM"
assert_contains "T2: et le site d'injection existe bien dans _set_up_worktree" \
    '_FIRE_DISPOSITION_RULE' "$T2306_SUW_SRC"

# --- T3 : contrôle négatif — dev-pilot ne la reçoit pas ----------------------
#
# Porteur, pas décoratif : le site voisin (`_PR_BODY_CONTAINMENT_RULE`) est
# inconditionnel dans cette branche, donc la condition se PERD en copiant le
# voisin. C'est exactement l'écart que ce test attrape — et il ne rendrait
# aucune décision fausse, il mettrait juste du bruit dans le prompt de tout
# pilote d'implémentation.

T2306_PROMPT_PILOT=$(_t2306_inject "dev-pilot")
assert_not_contains "T3: la règle n'est PAS injectée pour SKILL=dev-pilot" \
    '## Fire-Disposition' "$T2306_PROMPT_PILOT"
assert_contains "T3: mais dev-pilot reçoit bien la règle inconditionnelle voisine" \
    "RÈGLE DE DISPATCH (mika#2211)" "$T2306_PROMPT_PILOT"
# La garde est lue par POSITION dans les lignes de code, pas par un `grep -B`
# à fenêtre fixe : une ligne de commentaire ajoutée demain entre les deux
# déplacerait la fenêtre et rendrait le test vert pour la mauvaise raison.
T2306_SUW_CODE=$(printf '%s\n' "$T2306_SUW_SRC" | grep -v '^[[:space:]]*#')
T2306_FD_INJ_LN=$(printf '%s\n' "$T2306_SUW_CODE" | grep -n '_FIRE_DISPOSITION_RULE' | head -1 | cut -d: -f1)
T2306_FD_PREV_LN=""
if [ -n "$T2306_FD_INJ_LN" ] && [ "$T2306_FD_INJ_LN" -gt 1 ]; then
    T2306_FD_PREV_LN=$(printf '%s\n' "$T2306_SUW_CODE" | sed -n "$((T2306_FD_INJ_LN - 1))p")
fi
assert_contains "T3: la ligne de code qui précède l'injection est la garde sur le skill" \
    'if [ "$SKILL" = "dev-groom" ]; then' "$T2306_FD_PREV_LN"

# --- T4 : contrat mika#138 et fidélité du miroir -----------------------------
#
# Invariant de position 2 : si la première ligne de PROMPT cesse d'être
# `<repo>#<num>`, la regex ancrée de `_set_up_worktree` manque, le dispatch
# tombe en mode free-text et AUCUN worktree n'est créé. Régression de premier
# ordre, et invisible à toute assertion sur le contenu de la règle.

assert_eq "T4: la première ligne du PROMPT reste exactement mika#2306" "mika#2306" \
    "$(printf '%s' "$T2306_PROMPT_GROOM" | head -1)"
assert_eq "T4: idem sur le chemin dev-pilot" "mika#2306" \
    "$(printf '%s' "$T2306_PROMPT_PILOT" | head -1)"
assert_contains "T4: l'expression source est littéralement celle que le miroir T2/T3 réplique" \
    'PROMPT=$(printf '"'"'%s\n\n%s'"'"' "$PROMPT" "$_FIRE_DISPOSITION_RULE")' "$T2306_LIB_SRC"
# La règle est appendue APRÈS celle de mika#2211 : l'inverse laisserait la règle
# de corps de PR en dernier sur le chemin dev-groom, or la recency est le seul
# levier qu'a une règle en fin d'un prompt de 16 KiB.
assert_eq "T4: l'injection FD suit celle de mika#2211" "yes" \
    "$(_t2178_after '"$PROMPT" "$_FIRE_DISPOSITION_RULE")' '"$PROMPT" "$_PR_BODY_CONTAINMENT_RULE")')"

# --- T5 : le cœur du correctif ----------------------------------------------
#
# Findings réclamant la section + plan révisé qui ne la porte toujours pas
# ⇒ exactement UNE relance (deux invocations du pilote : la nominale et elle).

T2306_T5=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "no" "never")
assert_eq "T5: findings réclamant FD + section absente ⇒ exactement une relance" \
    "2" "$(_t2306_field 2 "$T2306_T5")"
assert_contains "T5: et la relance est DITE, pas silencieuse" \
    "fire_disposition_revise_retried" "$T2306_T5"
assert_eq "T5: la valeur de retour reste 0 — la garde ajoute une tentative, pas un mode d'échec" \
    "0" "$(_t2306_field 1 "$T2306_T5")"

# Et quand la seconde tentative réussit, elle se tait : l'événement d'échec est
# réservé à l'échec, sinon il ne mesure plus rien.
T2306_T5B=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "no" "second")
assert_eq "T5: seconde tentative réussie ⇒ toujours une seule relance" \
    "2" "$(_t2306_field 2 "$T2306_T5B")"
assert_not_contains "T5: et aucun événement d'échec n'est émis quand la section arrive" \
    "fire_disposition_still_missing_after_retry" "$T2306_T5B"

# --- T6 : contrôle négatif — l'architecte n'a rien demandé -------------------

T2306_T6=$(_t2306_revise_probe "$T2306_FINDINGS_WITHOUT_FD" "no" "never")
assert_eq "T6: findings sans mention de FD ⇒ zéro relance" \
    "1" "$(_t2306_field 2 "$T2306_T6")"
assert_not_contains "T6: et rien n'est journalisé" \
    "fire_disposition_revise_retried" "$T2306_T6"

# --- T7 : contrôle négatif — le revise a fait son travail -------------------

T2306_T7=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "yes" "never")
assert_eq "T7: section déjà présente ⇒ zéro relance (le chemin nominal ne paie rien)" \
    "1" "$(_t2306_field 2 "$T2306_T7")"
assert_not_contains "T7: et rien n'est journalisé" \
    "fire_disposition_revise_retried" "$T2306_T7"

# --- T8 : fail-safe — une information illisible sort de la population --------
#
# Deux moitiés. (a) À l'entrée : la garde préexistante rend 1, et la garde FD
# n'y change rien. (b) EN VOL : le pilote de revise supprime le findings-file
# entre-temps. C'est la seule forme que l'illisibilité peut réellement prendre
# ici — l'entrée est déjà gardée — et elle doit sortir le dispatch de la
# population, jamais l'y faire entrer.

assert_eq "T8 (a): findings illisible à l'entrée ⇒ retour 1, inchangé" "1" \
    "$(WORKTREE_DIR="/tmp" ISSUE_NUM="2306" _launch_revise_pilot "/nonexistent/findings-2306.md" 2>/dev/null; echo $?)"

T2306_T8=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "no" "wipe")
assert_eq "T8 (b): findings disparu en vol ⇒ zéro relance" \
    "1" "$(_t2306_field 2 "$T2306_T8")"
assert_eq "T8 (b): et la valeur de retour reste celle d'avant le correctif" \
    "0" "$(_t2306_field 1 "$T2306_T8")"

# --- T9 : le budget architecte est intact ------------------------------------
#
# Structurel, et il l'est pour la raison qui rend ces scans nécessaires ailleurs
# dans ce dépôt : la régression qu'il attrape ne rendrait AUCUNE décision
# fausse. Un futur éditeur qui « améliorerait » la garde en redemandant l'avis
# de l'architecte doublerait le budget LLM par grooming sans qu'un seul test de
# comportement ne rougisse.

assert_eq "T9: zéro appel _arch_ask dans _launch_revise_pilot" "0" \
    "$(printf '%s\n' "$T2306_REVISE_SRC" | grep -c '_arch_ask' || true)"
assert_eq "T9: zéro appel _arch_ask dans la garde de rattrapage" "0" \
    "$(printf '%s\n' "$T2306_GUARD_SRC" | grep -c '_arch_ask' || true)"

# Fire-Disposition, option (a) — la table d'exceptions de CE scan est vide, et
# la vacuité est assertée À L'EXÉCUTION (modèle scripts/test-guard-shared-checkout.sh).
# Le test rougit donc LE JOUR où une exception est ajoutée, pas seulement quand
# elle devient stale. Une table vide assertée vide est aussi ce qui distingue
# « aucune violation » de « le scan ne regarde rien ».
#
# Toute entrée future doit porter LES TROIS propriétés que l'option (a) exige —
# (1) nommer la donnée précise, (2) référencer un ticket de suivi, (3) porter une
# assertion auto-nettoyante — et non la seule troisième.
T2306_ARCH_ASK_ALLOWLIST=()
assert_eq "T9: table d'exceptions du scan _arch_ask — zero entries" "0" \
    "${#T2306_ARCH_ASK_ALLOWLIST[@]}"
# Contre-vacuité : le scan doit voir quelque chose. `_iterate_groom_loop`, la
# fonction voisine, en contient bien — sinon un `grep -c` qui rend 0 partout
# prouverait seulement que la source n'est pas lue.
_T2306_ITERATE_SRC=$(declare -f _iterate_groom_loop)
if [ "$(printf '%s\n' "$_T2306_ITERATE_SRC" | grep -c '_arch_ask' || true)" -gt 0 ]; then
    assert_eq "T9 contre-vacuité: le scan voit bien les _arch_ask de _iterate_groom_loop" "ok" "ok"
else
    assert_eq "T9 contre-vacuité: le scan voit bien les _arch_ask de _iterate_groom_loop" "ok" \
        "le scan ne trouve aucun _arch_ask nulle part — il ne regarde rien"
fi

# --- T10 : terminaison, et l'échec de second tour est DIT --------------------
#
# Sans ce test, une garde qui rendrait la main en silence sur ce chemin
# passerait T5 en vert : l'échec de second tour deviendrait indistinguable d'un
# succès — précisément l'angle mort que ce correctif reproche au critère sha256.

assert_contains "T10: seconde tentative échouée ⇒ fire_disposition_still_missing_after_retry" \
    "fire_disposition_still_missing_after_retry" "$T2306_T5"
assert_eq "T10: émis exactement une fois" "1" \
    "$(_t2306_err "$T2306_T5" | grep -c 'fire_disposition_still_missing_after_retry' || true)"
assert_eq "T10: et AUCUNE troisième relance" "2" "$(_t2306_field 2 "$T2306_T5")"
# La terminaison est lisible sans dérouler le flot : un compteur explicite, armé
# AVANT toute action, donc un échec en aval ne peut pas rouvrir le budget.
assert_contains "T10: le compteur de garde existe et est armé avant l'action" \
    "_FD_REVISE_RETRIED=1" "$T2306_GUARD_SRC"
assert_contains "T10: et il est remis à zéro à chaque entrée de _launch_revise_pilot" \
    "_FD_REVISE_RETRIED=0" "$T2306_REVISE_SRC"

# --- T11 : contrôle de source — le prédicat auto-entretenu ------------------
#
# La seule défaillance de cette famille qu'aucun autre test ne voit. Un
# implémenteur qui lit le findings CIBLÉ (qui contient nécessairement la chaîne,
# c'est son objet) rend le premier terme vrai par construction : la garde
# relance même quand l'architecte n'a rien demandé, le compteur borne la boucle,
# et T5, T6, T8 restent TOUS verts. T6 exerce un findings de première passe
# propre, pas la lecture du mauvais fichier.

# (a) forme de code : aucune ligne qui manipule le findings ciblé ne l'interroge,
#     et la relance n'est pas une récursion.
T2306_FD_VAR_LINES=$(printf '%s\n' "$T2306_GUARD_SRC" | grep 'fd_findings_file' || true)
assert_eq "T11 (a): aucune ligne touchant findings-1-fd.md ne l'interroge par grep" "0" \
    "$(printf '%s\n' "$T2306_FD_VAR_LINES" | grep -c 'grep' || true)"
assert_eq "T11 (a): et la garde ne se relance pas par récursion sur _launch_revise_pilot" "0" \
    "$(printf '%s\n' "$T2306_GUARD_SRC" | grep -c '_launch_revise_pilot' || true)"
# Contre-vacuité de la moitié (a) : la garde manipule réellement ce fichier.
assert_eq "T11 (a) contre-vacuité: la garde écrit bien un findings ciblé" "yes" \
    "$([ -n "$T2306_FD_VAR_LINES" ] && echo yes || echo non-trouvé)"
# Et le terme 1 lit bien le paramètre reçu, pas un chemin reconstruit.
assert_contains "T11 (a): le terme 1 interroge le findings de première passe reçu en argument" \
    'first_pass_findings' "$(printf '%s\n' "$T2306_GUARD_SRC" | grep 'Fire-Disposition' | grep 'grep -qF' || true)"

# (b) comportemental : un findings-1-fd.md résiduel laissé par un dispatch
#     antérieur ne rend aucun terme vrai.
T2306_T11=$(_t2306_revise_probe "$T2306_FINDINGS_WITHOUT_FD" "no" "never" "residual-fd")
assert_eq "T11 (b): un findings-1-fd.md résiduel ne déclenche aucune relance" \
    "1" "$(_t2306_field 2 "$T2306_T11")"
assert_not_contains "T11 (b): et rien n'est journalisé" \
    "fire_disposition_revise_retried" "$T2306_T11"

# ============================================================================
# mika#2544 — les lecteurs `Fire-Disposition` tolèrent la numérotation (T12)
# ============================================================================
#
# `/ce:plan` numérote ses titres de section (`## 3. Fire-Disposition`). Le gate
# `Acceptance criteria` le tolère depuis mika#2516 ; les deux lecteurs FD de
# `_fd_retry_if_section_still_missing` ne le toléraient pas. Sur un plan
# CONFORME à titre numéroté, le rattrapage relançait un pilote de revise pour
# rien, avec un finding synthétique faux, et `fire_disposition_still_missing_after_retry`
# (régime attendu : zéro) était émis à tort — classe mika#2205, un instrument
# qui affirme un état qui n'a pas eu lieu.
#
# T12a est la fixture porteuse. T12e est obligatoire : avec le seul terme 2
# élargi, T12a passe au vert et le re-test de journalisation émet toujours
# l'événement faux. T12c/T12d sont les contrôles négatifs de la tolérance, et
# T7 (plus haut) reste celui de la forme non numérotée.

echo ""
echo 'Test: mika#2544 — lecteurs `Fire-Disposition` tolérant la numérotation (T12)'
echo "-----------------------------------------------------------------------------"

# --- T12a–T12g : comportement de la vraie fonction ---------------------------

T2544_T12A=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "numbered" "never")
assert_eq "T12a: findings réclamant FD + section NUMÉROTÉE ⇒ zéro relance" \
    "1" "$(_t2306_field 2 "$T2544_T12A")"
assert_not_contains "T12a: et rien n'est journalisé" \
    "fire_disposition_revise_retried" "$T2544_T12A"

T2544_T12B=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "upper" "never")
assert_eq "T12b: section numérotée en capitales ⇒ zéro relance (casse tolérée, D2)" \
    "1" "$(_t2306_field 2 "$T2544_T12B")"

# Contrôles négatifs : la tolérance n'a pas avalé les refus. Vus verts AVANT le
# correctif aussi — c'est ce qui prouve un élargissement et non un désarmement.
T2544_T12C=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "notes" "never")
assert_eq "T12c: seul '## 11. Notes on Fire-Disposition' ⇒ une relance (refus tenu)" \
    "2" "$(_t2306_field 2 "$T2544_T12C")"
T2544_T12D=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "subsub" "never")
assert_eq "T12d: '## 1.1 Fire-Disposition' ⇒ une relance (borne du préfixe assumée, D2)" \
    "2" "$(_t2306_field 2 "$T2544_T12D")"

# La moitié oubliée : le revise AJOUTE la section, numérotée. La relance est
# légitime (le plan initial ne la porte pas) ; l'événement d'échec, non.
T2544_T12E=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "no" "second-numbered")
assert_eq "T12e: section ajoutée numérotée à la relance ⇒ toujours une seule relance" \
    "2" "$(_t2306_field 2 "$T2544_T12E")"
assert_not_contains "T12e: et AUCUN fire_disposition_still_missing_after_retry (re-test de journalisation)" \
    "fire_disposition_still_missing_after_retry" "$T2544_T12E"

# Chaîne C2 : le plan porte déjà la section numérotée et l'architecte en
# critique le CONTENU — le terme 1 est vrai sans ITERATE préalable sur la
# section elle-même.
T2544_FINDINGS_FD_CONTENT="F1 [BLOQUANT] — la section Fire-Disposition nomme l'option (a) sans
assertion auto-nettoyante. Précise-la.

Disposition: ITERATE"
T2544_T12F=$(_t2306_revise_probe "$T2544_FINDINGS_FD_CONTENT" "numbered" "never")
assert_eq "T12f: findings critiquant le contenu + section numérotée ⇒ zéro relance" \
    "1" "$(_t2306_field 2 "$T2544_T12F")"

# Le coût de D3, rendu lisible : pas d'ancre de fin (alignée sur AC_HEADING_RE),
# donc un titre `## Fire-Disposition-ish` est lu comme la section.
T2544_T12G=$(_t2306_revise_probe "$T2306_FINDINGS_WITH_FD" "ish" "never")
assert_eq "T12g: '## Fire-Disposition-ish' en tête de ligne ⇒ lu présent, zéro relance (D3)" \
    "1" "$(_t2306_field 2 "$T2544_T12G")"

# --- T12h : la matrice du motif, ligne par ligne ------------------------------
#
# Les cas négatifs du ticket (`###`, prose, backticks) et les bornes de D2, lus
# par la constante elle-même avec le drapeau des deux sites. Un motif vide
# matcherait tout : la non-vacuité est assertée d'abord.

assert_eq "T12h: _FD_HEADING_RE est définie et non vide" "non-vide" \
    "$([ -n "${_FD_HEADING_RE:-}" ] && echo non-vide || echo vide)"
_t2544_reads() {
    if [ -n "${_FD_HEADING_RE:-}" ] && grep -qiE -- "$_FD_HEADING_RE" <<<"$1"; then
        echo present
    else
        echo absent
    fi
}
while IFS='|' read -r t2544_want t2544_line; do
    assert_eq "T12h: « $t2544_line » ⇒ $t2544_want" "$t2544_want" "$(_t2544_reads "$t2544_line")"
done <<'T2544_MATRIX'
present|## Fire-Disposition
present|## 3. Fire-Disposition
present|## 3 Fire-Disposition
present|## 11. Fire-Disposition
present|## fire-disposition
present|##   Fire-Disposition
present|## Fire-Disposition (option a)
present|## Fire-Disposition-ish
absent|### Fire-Disposition
absent|# Fire-Disposition
absent|##Fire-Disposition
absent|la section `## Fire-Disposition` est requise
absent|une section Fire-Disposition-ish dans une ligne de prose
absent|## Notes on Fire-Disposition
absent|## 1.1 Fire-Disposition
T2544_MATRIX

# --- T12i : la clause de vocabulaire des deux gates architecte ---------------
#
# Assertion de PRÉSENCE, pas de comportement : le verdict est rendu par un LLM,
# que ce harnais ne peut pas exercer. Le contrat côté mika est « la clause est
# dans le prompt servi ».

for t2544_prompt in mika-arch-groom-ticket mika-arch-second-review; do
    t2544_pf="$SCRIPT_DIR/../$t2544_prompt/system_prompt.md"
    if [ -r "$t2544_pf" ]; then
        t2544_gate=$(sed -n '/^### Fire-Disposition Gate/,/^### /p' "$t2544_pf")
        assert_contains "T12i: $t2544_prompt — le gate FD dit qu'un titre numéroté est la section" \
            '`## 3. Fire-Disposition`' "$t2544_gate"
    else
        assert_eq "T12i: $t2544_prompt/system_prompt.md lisible" "lisible" "fichier introuvable: $t2544_pf"
    fi
done

# --- T12j : lecteur unique par section, et co-mutation FD ⇔ AC ---------------
#
# Le commentaire de `AC_HEADING_RE` promettait une garde de lecteur unique dans
# scripts/verify-pipeline-test.sh ; elle n'y était pas, et ce harnais-là n'est
# lancé par aucun job CI. Elle vit ici, dans le seul harnais shell câblé en CI,
# et couvre les deux sections.
#
# Prédicat POSITIONNEL, jamais lexical : une ligne de code (hors commentaire)
# qui invoque `grep`/`sed` ET porte, entre quotes, un littéral de titre
# (`#…<section>`). Les `echo "FAIL: … '## Acceptance criteria' …"` de
# verify-pipeline.sh et la prose de `_FIRE_DISPOSITION_RULE` citent le littéral
# sans être des lecteurs : ils sont hors population par construction. Le terme 1
# de la garde FD (`grep -qF -- 'Fire-Disposition'` sur les FINDINGS, sans `#`)
# l'est aussi — autre population, délibérément grossière.
#
# Borne nommée, pas découverte : la population est celle des lignes `grep`/`sed`.
# Un lecteur `awk`, `[[ =~ ]]`, `case`, un motif en variable ou sur une ligne de
# continuation échappe au scan. Aucun n'existe dans les deux fichiers ; le jour
# où l'un est écrit, on élargit l'alternance de commandes ici, dans le même
# commit.
#
# Contre-vacuité en deux moitiés : (i) au moins un lecteur via la constante par
# section ; (ii) un fichier illisible ROUGIT avec « fichier introuvable », au
# lieu de sauter comme le `if [ -f ]` de T2211 — un scan qui ne regarde rien se
# lit exactement comme un arbre propre (mika#2205).
#
# Rend : « introuvable », ou « <littéraux en position de motif>|<lecteurs via la constante> ».
_t2544_heading_scan() {
    local file="$1" section="$2" const="$3" code violations readers
    [ -r "$file" ] || { printf 'introuvable'; return 0; }
    code=$(grep -v '^[[:space:]]*#' "$file" \
        | grep -E '(^|[^[:alnum:]_-])(grep|sed)([[:space:]]|$)' || true)
    violations=$(printf '%s\n' "$code" \
        | grep -iE "['\"][^'\"]*#[^'\"]*${section}" | grep -c . || true)
    readers=$(printf '%s\n' "$code" | grep -F "\$${const}" | grep -c . || true)
    printf '%s|%s' "$violations" "$readers"
}

T2544_FD_SCAN=$(_t2544_heading_scan "$DISPATCH_LIB" "Fire-Disposition" "_FD_HEADING_RE")
T2544_AC_SCAN=$(_t2544_heading_scan "$REPO_ROOT/scripts/verify-pipeline.sh" "Acceptance criteria" "AC_HEADING_RE")

assert_eq "T12j (FD): dispatch-lib.sh lisible par le scan" "lu" \
    "$([ "$T2544_FD_SCAN" != introuvable ] && echo lu || echo "fichier introuvable: $DISPATCH_LIB")"
assert_eq "T12j (FD): aucun littéral de titre Fire-Disposition en position de motif" "0" \
    "${T2544_FD_SCAN%%|*}"
assert_eq "T12j (FD): exactement deux lecteurs, tous deux via \$_FD_HEADING_RE" "2" \
    "${T2544_FD_SCAN##*|}"
# Le drapeau combiné, pas `-E` seul : écrire `-qi` en remplaçant `-qE` perd
# l'ERE en silence, le motif ne matche plus rien et le rattrapage tire sur tout.
assert_eq "T12j (FD): les deux lecteurs portent -qiE" "2" \
    "$(grep -v '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF 'grep -qiE "$_FD_HEADING_RE"' || true)"

assert_eq "T12j (AC): verify-pipeline.sh lisible par le scan" "lu" \
    "$([ "$T2544_AC_SCAN" != introuvable ] && echo lu || echo "fichier introuvable: $REPO_ROOT/scripts/verify-pipeline.sh")"
assert_eq "T12j (AC): aucun littéral de titre Acceptance criteria en position de motif" "0" \
    "${T2544_AC_SCAN%%|*}"
assert_eq "T12j (AC): exactement deux lecteurs, tous deux via \$AC_HEADING_RE" "2" \
    "${T2544_AC_SCAN##*|}"

# Contre-vacuité (ii), exercée à chaque exécution plutôt qu'une fois à la main.
assert_eq "T12j contre-vacuité: un fichier absent rend « introuvable », pas zéro violation" \
    "introuvable" "$(_t2544_heading_scan "/nonexistent/mika-2544.sh" "Fire-Disposition" "_FD_HEADING_RE")"
# Et le prédicat positionnel mord : un troisième lecteur littéral est vu.
assert_eq "T12j contre-vacuité: un grep littéral '^## Fire-Disposition' est compté" "1|0" \
    "$(_t2544_heading_scan <(printf '%s\n' "    ! grep -qE '^## Fire-Disposition' \"\$p\" || return 0") \
        "Fire-Disposition" "_FD_HEADING_RE")"

# Co-mutation : aucun fichier n'est lisible par les deux gates au même instant
# (verify-pipeline.sh est lu depuis le dépôt, dispatch-lib.sh est une projection
# du binaire — mika#2340), donc le préfixe est doublé et tenu ici. Le jour où
# l'un des deux évolue, ce test rougit au lieu de laisser les gates rediverger.
T2544_AC_RE=$(sed -nE "s/^[[:space:]]*AC_HEADING_RE='(.*)'[[:space:]]*\$/\\1/p" \
    "$REPO_ROOT/scripts/verify-pipeline.sh" 2>/dev/null | head -1)
assert_eq "T12j (co-mutation): AC_HEADING_RE extraite de verify-pipeline.sh" "non-vide" \
    "$([ -n "$T2544_AC_RE" ] && echo non-vide || echo vide)"
T2544_FD_RE="${_FD_HEADING_RE:-}"
assert_eq "T12j (co-mutation): préfixe de _FD_HEADING_RE == préfixe de AC_HEADING_RE" \
    "${T2544_AC_RE%Acceptance criteria}" "${T2544_FD_RE%Fire-Disposition}"

# Fire-Disposition, option (a) — table d'exceptions du scan vide, vacuité
# assertée à l'exécution (modèle T2306_ARCH_ASK_ALLOWLIST). Quand le scan tire,
# on route le site vers la constante ; on n'ajoute pas de ligne ici (mika#2201).
T2544_HEADING_READER_ALLOWLIST=()
assert_eq "T12j: table d'exceptions du scan de lecteurs — zero entries" "0" \
    "${#T2544_HEADING_READER_ALLOWLIST[@]}"

# --- mika#2449: aucun message de récupération de stash ne nomme le checkout principal ---
#
# Deux sites de dispatch-lib stashent un worktree sale et impriment une
# consigne « recover with: git -C <dir> stash apply <sha> ». Le site A
# (_clean_worktree_for_rebase) nomme le worktree ; le site B (_set_up_worktree,
# relic non canonique) nommait `$SUB_REPO_DIR` — le checkout PRINCIPAL. Exécutée,
# cette consigne dépose le contenu non committé d'un worktree dans le checkout de
# déploiement, exactement la signature du sinistre du 2026-09-21 (staged +
# modified sur main, rebuild bloqué). Le lecteur de ce message est un acteur non
# contenu (opérateur, mika-dev lisant un dispatch en échec), donc une
# prescription écrite est un producteur latent.
#
# Scan de source à ALLOWLIST VIDE : toute ligne portant à la fois `stash apply`
# et `recover with` est un site ; aucun ne peut nommer SUB_REPO_DIR. Quand il
# tire, la résolution est de corriger le message, jamais d'ajouter une entrée.
# Contrôle de bonne foi : ≥ 2 sites trouvés, sinon le scan est vert parce qu'il
# ne regarde rien (classe mika#2205).
T2449_RECOVER_SITES=$(grep -n 'stash apply' "$DISPATCH_LIB" | grep 'recover with' || true)
T2449_SITE_COUNT=$(printf '%s\n' "$T2449_RECOVER_SITES" | grep -c 'recover with' || true)
assert_eq "mika#2449 bonne foi: le scan trouve ≥ 2 sites « recover with … stash apply »" "yes" \
    "$([ "$T2449_SITE_COUNT" -ge 2 ] && echo yes || echo "non ($T2449_SITE_COUNT)")"
assert_eq "mika#2449: aucun site « recover with » ne nomme SUB_REPO_DIR (allowlist vide)" "0" \
    "$(printf '%s\n' "$T2449_RECOVER_SITES" | grep -c 'SUB_REPO_DIR' || true)"
# Le site B doit nommer le worktree CANONIQUE, pas le relic : celui-ci est
# supprimé quelques lignes plus bas (`worktree remove --force "$existing_wt"`),
# donc une consigne qui le nommerait échouerait au moment où l'opérateur la lit.
T2449_SITE_B=$(printf '%s\n' "$T2449_RECOVER_SITES" | grep 'stale-worktree-cleanup\|existing_wt' || true)
assert_contains "mika#2449: le site B (relic) prescrit le worktree canonique \$WORKTREE_DIR" \
    'git -C $WORKTREE_DIR stash apply' "$T2449_SITE_B"
assert_not_contains "mika#2449: le site B ne prescrit pas le relic \$existing_wt (supprimé plus bas)" \
    'git -C $existing_wt stash apply' "$T2449_SITE_B"
assert_contains "mika#2449: le site B dit explicitement de ne pas appliquer dans le checkout principal" \
    'NOT in the primary checkout' "$T2449_SITE_B"

# --- mika#2492: la PR nominale de la boucle n'est pas une épave ---
#
# Tout ticket groomé est dispatché sous `/ce-work <plan>` (override mika#1074),
# dont le périmètre est « implementation and local verification only, without
# the shipping tail ». Ce pilote n'ouvre donc JAMAIS de PR — c'est son périmètre,
# pas une troncature — et dispatch-lib classait pourtant sa PR en épave
# (`commit-pushed-no-pr`), ce qui armait trois gardes contre la PR même que la
# boucle existe pour produire.
#
# Le croisement a DEUX axes et une seule case change :
#   A périmètre (present/absent) × B session (conclue/tronquée)
# Les deux états de B existent dans la population mesurée du 2026-09-22 :
# #2425 (`[done] Success | 142 turns`) contre #2484 (`[guardrail]
# error_max_turns`), tous deux sous `/ce-work`.

echo ""
echo "Test mika#2492: classe no-shipping-tail — le chemin nominal cesse d'être une épave"
echo "---------------------------------------------------------------------------------"

# --- T1/T2/T3/T4 : le prédicat, sur ses quatre croisements (comportemental) ---
#
# Sonde pure : source la lib, pose les deux axes, interroge le prédicat.
# $1 = PILOT_SHIPPING_TAIL, $2 = STATUS, $3 = SKILL
_t2492_predicate() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        PILOT_SHIPPING_TAIL="$1"
        STATUS="$2"
        SKILL="$3"
        if _pilot_had_no_shipping_tail; then echo "yes"; else echo "no"; fi
    )
}

assert_eq "T1: périmètre absent + session conclue ⇒ classe nouvelle" \
    "yes" "$(_t2492_predicate absent success dev-pilot)"
assert_eq "T2 (contrôle négatif, axe B): périmètre absent + session tronquée ⇒ épave" \
    "no" "$(_t2492_predicate absent terminated dev-pilot)"
assert_eq "T3 (non-régression, axe A): périmètre présent + session conclue ⇒ inchangé" \
    "no" "$(_t2492_predicate present success dev-pilot)"
assert_eq "T3 (non-régression): périmètre présent + session tronquée ⇒ inchangé" \
    "no" "$(_t2492_predicate present terminated dev-pilot)"
assert_eq "T4 (fail-safe R4): estampille vide ⇒ comportement d'avant le ticket" \
    "no" "$(_t2492_predicate '' success dev-pilot)"
assert_eq "T4 (fail-safe R4): statut vide ⇒ comportement d'avant le ticket" \
    "no" "$(_t2492_predicate absent '' dev-pilot)"
assert_eq "T4 (fail-safe R4): dev-groom n'entre jamais dans la classe" \
    "no" "$(_t2492_predicate absent success dev-groom)"

# --- T7b/T8 : la garde a PRIS, et la ligne Outcome de la fenêtre (comportemental) ---
#
# Un scan de présence ne distingue pas « la garde est écrite » de « la garde est
# écrite dans le bon sens » : retirer le `!` de la conjonction du bloc mika#940
# Unit 1 est une régression d'UN caractère qui laisse T7a vert. Cette sonde
# exécute `_post_flight_recovery` sur un vrai dépôt et lit le RESULT produit.
#
# $1 = PILOT_SHIPPING_TAIL, $2 = STATUS. Rend le RESULT complet.
_t2492_recovery_probe() {
    local base_dir wt_dir pre_head
    base_dir=$(mktemp -d)
    wt_dir="$base_dir/worktree"

    git init -q "$wt_dir" 2>/dev/null
    echo "initial" > "$wt_dir/file.txt"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "initial" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)
    # Le pilote a commité : HEAD avance, worktree propre.
    echo "impl" > "$wt_dir/feature.rs"
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "fix(2492): the pilot's own implementation commit" 2>/dev/null

    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        # Stub : aucune sortie réseau depuis le harness. `gh` n'est jamais
        # invoqué via `command`, donc la fonction l'emporte.
        gh() { return 1; }

        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD=$(git -C "$wt_dir" rev-parse HEAD)
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-pilot"
        REPO="mika"
        BRANCH="fix/2492/probe"
        ISSUE_NUM="2492"
        SESSION_ID="t2492"
        LOG_ID="t2492"
        RESULT="claude-pilot completed (status: ${2})."
        RESCUED_DIRTY_WORKTREE=0
        PILOT_SHIPPING_TAIL="$1"
        STATUS="$2"

        exec 9>&2
        _post_flight_recovery 2>/dev/null
        printf '%s' "$RESULT"
    )

    rm -rf "$base_dir"
}

T2492_NOMINAL=$(_t2492_recovery_probe absent success 2>/dev/null)
assert_not_contains "T7b: périmètre absent + conclue ⇒ AUCUN PIPELINE FAILURE (la garde a pris)" \
    "PIPELINE FAILURE:" "$T2492_NOMINAL"
assert_contains "T8: la fenêtre avant Path B porte un motif nommé, jamais UNKNOWN" \
    "Outcome: PIPELINE_INCOMPLETE — no_shipping_tail: dispatch-lib did not reach PR creation." \
    "$T2492_NOMINAL"
assert_not_contains "T8: et jamais le UNKNOWN générique (moins actionnable qu'aujourd'hui)" \
    "Outcome: UNKNOWN" "$T2492_NOMINAL"
assert_eq "T8: exactement une ligne Outcome dans la fenêtre" \
    "1" "$(printf '%s\n' "$T2492_NOMINAL" | grep -c '^Outcome: ' || true)"

# Contrôle négatif de la même sonde : le périmètre présent garde son
# PIPELINE FAILURE, mot pour mot. Sans lui, T7b ne distingue pas « la garde
# discrimine » de « le bloc mika#940 ne fire plus du tout ».
T2492_PRESENT=$(_t2492_recovery_probe present success 2>/dev/null)
assert_contains "T7b (contrôle négatif): périmètre présent ⇒ PIPELINE FAILURE inchangé" \
    "PIPELINE FAILURE: claude-pilot produced commits" "$T2492_PRESENT"
assert_contains "T3: et sa ligne Outcome reste PIPELINE_INCOMPLETE — manual recovery needed" \
    "Outcome: PIPELINE_INCOMPLETE — manual recovery needed." "$T2492_PRESENT"
assert_eq "T3: une seule ligne Outcome sur ce croisement aussi" \
    "1" "$(printf '%s\n' "$T2492_PRESENT" | grep -c '^Outcome: ' || true)"

# Le fail-safe, vu de bout en bout : estampille vide ⇒ chemin d'avant le ticket.
T2492_UNSTAMPED=$(_t2492_recovery_probe '' success 2>/dev/null)
assert_contains "T4 (bout en bout): estampille vide ⇒ PIPELINE FAILURE, comme avant" \
    "PIPELINE FAILURE: claude-pilot produced commits" "$T2492_UNSTAMPED"

# --- T8 (suite) : _set_outcome_line rend le contrat vrai par construction ---
_t2492_outcome_line_probe() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        RESULT="body line

Outcome: PIPELINE_INCOMPLETE — no_shipping_tail: dispatch-lib did not reach PR creation."
        _set_outcome_line "Outcome: PR_OPENED — https://example/pr/1"
        printf '%s' "$RESULT"
    )
}
T2492_OUTCOME=$(_t2492_outcome_line_probe 2>/dev/null)
assert_eq "T8: après réécriture, exactement une ligne Outcome" \
    "1" "$(printf '%s\n' "$T2492_OUTCOME" | grep -c '^Outcome: ' || true)"
assert_contains "T8: et c'est la plus vraie des deux" \
    "Outcome: PR_OPENED — https://example/pr/1" "$T2492_OUTCOME"
assert_not_contains "T8: l'ancienne valeur de la fenêtre a disparu" \
    "no_shipping_tail" "$T2492_OUTCOME"
assert_contains "T8: le corps du RESULT est préservé" \
    "body line" "$T2492_OUTCOME"

# --- T5 : l'estampille est écrite à UN site par valeur (scan, allowlist vide) ---
#
# « On déclare, on n'allowliste pas » (mika#2201) : quand ce scan tire, la
# résolution est de RETIRER le second site, jamais d'y ajouter une entrée. La
# lecture couvre l'autre moitié — qu'aucun chemin d'entrée n'atteigne la
# sélection de classe sans estamper — et aucun scan ne peut l'atteindre :
# `SKILL` a un site d'écriture unique, le `case` de `dispatch_claude_pilot` est
# exhaustif et fatal (`*) exit 1`), la suite est linéaire, et la fonction n'a
# que deux appelants (dev-pilot/handlers/run.sh, dev-groom/handlers/run.sh).
SHIPPING_TAIL_WRITERS_ALLOWED=""   # mika#2492: livrée VIDE, et un test le tient
assert_eq "T5: allowlist des écrivains de l'estampille — zero entries" \
    "" "$SHIPPING_TAIL_WRITERS_ALLOWED"
assert_eq "T5: PILOT_SHIPPING_TAIL=\"absent\" n'est écrit qu'à un seul site" \
    "1" "$(grep -c 'PILOT_SHIPPING_TAIL="absent"' "$DISPATCH_LIB" || true)"
assert_eq "T5: PILOT_SHIPPING_TAIL=\"present\" n'est écrit qu'à un seul site" \
    "1" "$(grep -c 'PILOT_SHIPPING_TAIL="present"' "$DISPATCH_LIB" || true)"
# Le site `absent` doit être celui de l'override — pas un site voisin qui
# devinerait le périmètre après coup.
T2492_DETECT_FN=$(sed -n '/^_detect_plan_on_branch() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "T5: le site « absent » est la ligne même qui pose l'override /ce-work" \
    'PILOT_SHIPPING_TAIL="absent"' "$T2492_DETECT_FN"
assert_contains "T5: et il est adjacent à l'override qu'il décrit" \
    'ENTRY_COMMAND="/ce-work $PLAN_PATH"' "$T2492_DETECT_FN"
# Un seul lecteur décisionnel de l'estampille : le prédicat. Tout autre lecteur
# ferait diverger deux jugements d'un même fait.
assert_eq "T5: l'estampille n'a qu'un lecteur décisionnel (le prédicat)" \
    "1" "$(grep -c 'PILOT_SHIPPING_TAIL:-' "$DISPATCH_LIB" || true)"

# --- T7a : la conjonction du bloc mika#940 Unit 1 porte la NÉGATION ---
#
# Cherchée sur la LIGNE de la conjonction, pas dans le fichier : à l'échelle du
# fichier, l'appel légitime — et sans `!` — de la sélection de classe satisfait
# un scan de présence naïf.
UNIT1_GUARD_EXEMPT=""   # mika#2492: livrée VIDE
assert_eq "T7a: allowlist d'exemption du guard Unit 1 — zero entries" \
    "" "$UNIT1_GUARD_EXEMPT"
T2492_UNIT1_LINE=$(grep -n 'STATUS" = "success" \] && \[ "\$SKILL" = "dev-pilot"' "$DISPATCH_LIB" | head -1)
assert_contains "T7a: la conjonction Unit 1 porte « ! _pilot_had_no_shipping_tail » (négation incluse)" \
    '! _pilot_had_no_shipping_tail' "$T2492_UNIT1_LINE"

# --- T6 : le bras de la classe nouvelle n'émet JAMAIS RECOVERY_PENDING ---
NO_TAIL_RECOVERY_PENDING_ALLOWED=""   # mika#2492: livrée VIDE
assert_eq "T6: allowlist RECOVERY_PENDING de la classe nouvelle — zero entries" \
    "" "$NO_TAIL_RECOVERY_PENDING_ALLOWED"
PATHB_2492=$(sed -n '/Unit 2 (mika#1282 + mika#1396): open a draft PR/,/^    _deliver_callback/p' "$DISPATCH_LIB")
# Ancré sur le `if` à douze espaces — l'`elif` du bras titre/fact porte la même
# comparaison et ouvrirait la plage trop tôt. Les commentaires sont retirés : le
# scan porte sur le CODE, et le commentaire de ce bras nomme légitimement le
# marqueur qu'il n'émet pas (même geste que GATE_CODE au test 15).
T2492_NOTAIL_ARM=$(printf '%s\n' "$PATHB_2492" \
    | sed -n '/^            if \[ "\$RECOVERY_CLASS" = "no-shipping-tail"/,/^            else$/p' \
    | grep -v '^[[:space:]]*#')
assert_contains "T6 (bonne foi): le bras de la classe nouvelle existe dans Path B" \
    'PR: ${PR_URL}' "$T2492_NOTAIL_ARM"
assert_not_contains "T6: et il n'émet jamais RECOVERY_PENDING: true (Guard 1 désarmée)" \
    "RECOVERY_PENDING: true" "$T2492_NOTAIL_ARM"
assert_contains "T6: il réécrit la ligne Outcome en PR_OPENED" \
    '_set_outcome_line "Outcome: PR_OPENED — ${PR_URL}"' "$T2492_NOTAIL_ARM"
# R3 : l'autre bras garde son marqueur, mot pour mot.
assert_contains "T3 (R3): le bras des épaves émet toujours RECOVERY_PENDING: true" \
    "RECOVERY_PENDING: true" "$PATHB_2492"

# --- Sélection de classe : le bras nouveau est AVANT l'épave, et gardé ---
T2492_SELECT=$(printf '%s\n' "$PATHB_2492" | sed -n '/local RECOVERY_CLASS=""/,/^    fi$/p')
assert_contains "U3b: le bras no-shipping-tail est gardé par le prédicat" \
    '_pilot_had_no_shipping_tail; then' "$T2492_SELECT"
assert_contains "U3b: la garde RESCUED_DIRTY_WORKTREE reste première" \
    'RESCUED_DIRTY_WORKTREE:-}" = "1" ]; then' "$T2492_SELECT"
T2492_NOTAIL_POS=$(printf '%s\n' "$T2492_SELECT" | grep -n 'RECOVERY_CLASS="no-shipping-tail"' | head -1 | cut -d: -f1)
T2492_WRECK_POS=$(printf '%s\n' "$T2492_SELECT" | grep -n 'RECOVERY_CLASS="commit-pushed-no-pr"' | head -1 | cut -d: -f1)
assert_eq "U3b: le bras no-shipping-tail précède le bras commit-pushed-no-pr" "yes" \
    "$([ -n "$T2492_NOTAIL_POS" ] && [ -n "$T2492_WRECK_POS" ] && [ "$T2492_NOTAIL_POS" -lt "$T2492_WRECK_POS" ] && echo yes || echo "non ($T2492_NOTAIL_POS vs $T2492_WRECK_POS)")"

# --- Le commit marqueur reste réservé aux épaves (égalité stricte) ---
# La borne de fin est `RESCUED_PR_URL=` seul : écrire le motif complet de
# l'invocation ferait de cette ligne une fausse positive du détecteur
# d'herméticité mika#2178 T9, qui balaie tout ce qui suit sa propre section.
T2492_MARKER_GUARD=$(printf '%s\n' "$PATHB_2492" | sed -n '/RECOVERY_CLASS" = "commit-pushed-no-pr"/,/^        RESCUED_PR_URL=/p')
assert_not_contains "U3c: le commit marqueur wip(mika#1383) n'atteint pas la classe nouvelle" \
    "no-shipping-tail" "$T2492_MARKER_GUARD"
# L'intention de cette assertion est que la réécriture en PR_OPENED n'ait qu'un
# site — c'est la valeur, non l'helper, qui doit être unique. Le prédicat comptait
# sur le fichier entier parce qu'il n'existait alors qu'un seul appel ; mika#2545
# en a ajouté un second, pour `Outcome: ESCALATE`, et mika#2590 un troisième pour
# `Outcome: PLAN_GROOMED`. Chaque valeur est donc nommée et comptée à un site, ce
# qui est plus fort que l'ancien compte global : une quatrième valeur écrite par
# un helper partagé fait rougir la cardinalité ci-dessous au lieu de passer
# inaperçue. **La cardinalité a mordu à l'arrivée de la troisième** — c'est ce
# qui la rend crédible, et le geste juste est de nommer la valeur neuve, jamais
# de relâcher le compte.
assert_eq "U3c: la réécriture Outcome: PR_OPENED n'est appelée qu'au seul bras de la classe nouvelle" \
    "1" "$(grep -c '_set_outcome_line "Outcome: PR_OPENED' "$DISPATCH_LIB" || true)"
assert_eq "U3c (mika#2545): Outcome: ESCALATE n'est estampillé qu'à un site" \
    "1" "$(grep -c '_set_outcome_line "Outcome: ESCALATE' "$DISPATCH_LIB" || true)"
assert_eq "U3c (mika#2590): Outcome: PLAN_GROOMED n'est estampillé qu'à un site" \
    "1" "$(grep -c '_set_outcome_line "Outcome: PLAN_GROOMED' "$DISPATCH_LIB" || true)"
assert_eq "U3c: et _set_outcome_line n'a que ces trois valeurs déclarées" \
    "3" "$(grep -c '_set_outcome_line "Outcome:' "$DISPATCH_LIB" || true)"

# --- D5 : la classe nouvelle passe par le même producteur de marqueur ---
assert_contains "D5: _measure_pipeline_verified couvre toutes les classes de Path B" \
    '_measure_pipeline_verified "$WORKTREE_DIR"' "$PATHB_2492"
# D3 : le label est CONSERVÉ — le retirer sortirait la PR de wip_rescue.rs,
# seul mécanisme qui la rebase, la passe à clippy et la sort du draft, sans que
# qa_review_reconcile (qui exige isDraft == false) la rattrape.
assert_contains "D3: le label wip-rescue reste appliqué (sinon la PR sort de tous les filets)" \
    'add-label "wip-rescue"' "$PATHB_2492"

# --- Le titre de la classe nouvelle vient du vrai commit d'implémentation ---
_t2492_title_probe() {
    local base_dir wt_dir
    base_dir=$(mktemp -d); wt_dir="$base_dir/wt"
    git init -q "$wt_dir" 2>/dev/null
    echo x > "$wt_dir/a"; git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "fix(2492): le vrai sujet du commit du pilote" 2>/dev/null
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        _derive_recovery_pr_title "no-shipping-tail" "$wt_dir" "mika" "2492" "" "titre d'issue"
    )
    rm -rf "$base_dir"
}
assert_eq "U3c: le titre de la classe nouvelle est le sujet du commit d'implémentation" \
    "fix(2492): le vrai sujet du commit du pilote" "$(_t2492_title_probe 2>/dev/null)"

# =============================================================================
# mika#2493 — un deny non-terminal survécu est une NOTE, jamais un « halted »
# =============================================================================
#
# Ce que ces détecteurs mesurent. Le champ `result` préfixait « PIPELINE
# FAILURE: … halted by policy deny » alors que le refus n'avait pas arrêté la
# session et qu'elle avait livré. Le libellé a fait conclure « échec » à tort
# deux fois — opérateur ET orchestrateur — dans l'incident de la nuit du
# 2026-09-22. Preuves re-mesurées (M0) : sessions `98b60020` (2 refus) et
# `a0886164` (5 refus), **zéro terminal**, toutes deux `status: success`.
#
# Trois unités, trois populations de test :
#   U1 — la garde `[ -z "$VALID_PLAN" ]` (T8, T9, T15, T16)
#   U2 — le prédicat de létalité + la note annexée (T1–T7, T10, T11)
#   U3 — la dé-troncature de l'événement rapporté (T13, T14)
#
# Herméticité : aucune fixture ne touche le réseau ni la forge. Les deux sondes
# bout-en-bout redéfinissent `_pr_list_url` dans leur sous-shell.

echo ""
echo "Test mika#2493 : le libellé suit la létalité du deny"
echo "----------------------------------------------------"

T2493_FIXTURE_DIR=$(mktemp -d)

# --- T1–T7 : le prédicat de létalité ----------------------------------------
#
# D2 : le prédicat porte sur le FICHIER, jamais sur la ligne capturée. Trois
# raisons mesurées — le marqueur sort de la première ligne dans 23 % des cas
# (M2, dont la preuve `98b60020`) ; une session porte plusieurs refus ; et un
# refus terminal termine sa session (M4), donc « au moins un terminal » et « le
# dernier est terminal » coïncident, la première formulation seule survivant à
# la troncature.

_t2493_lethality() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        _policy_deny_lethality "$1"
    )
}

# T1 — refus non-terminal mono-ligne (forme la plus fréquente : 1093 des 1185
# marqueurs mesurés depuis cpp#151).
printf '\x1b[31m[policy:deny]\x1b[0m Bash: env | head -3 [bash-env] (non-terminal)\n' \
    > "$T2493_FIXTURE_DIR/nonterm.stderr"
assert_eq "T1: refus non-terminal mono-ligne ⇒ non-terminal" \
    "non-terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/nonterm.stderr")"

# T2 — refus terminal. La population que le verbe « halted » doit garder.
printf '[policy:deny] Bash: rm -rf /etc [bash-destructive] (terminal)\n' \
    > "$T2493_FIXTURE_DIR/term.stderr"
assert_eq "T2: refus terminal ⇒ terminal" \
    "terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/term.stderr")"

# T3 — forme antérieure à cpp#151 (2026-09-04) : aucun marqueur nulle part.
# D3 : rien n'est affirmé. Replier sur `non-terminal` poserait l'affirmation
# fausse dans l'autre sens ; replier sur `terminal` reconduirait le défaut.
printf '[policy:deny] Bash: gh auth status 2>&1 | head -10\n' \
    > "$T2493_FIXTURE_DIR/undeclared.stderr"
assert_eq "T3: refus sans marqueur (pré-cpp#151) ⇒ undeclared" \
    "undeclared" "$(_t2493_lethality "$T2493_FIXTURE_DIR/undeclared.stderr")"

# T4 — LE test qui sépare « lit le fichier » de « lit la ligne ». C'est la forme
# exacte de la preuve `98b60020` du ticket : `<detail>` multi-ligne, marqueur en
# 3ᵉ ligne. Toute implémentation qui lirait la ligne capturée par un
# `grep -m1` rougit ici — et serait donc fausse sur une des deux preuves.
printf '%s\n' \
    '[2026-09-22T17:35:39.081Z] [policy:deny] Bash: cd /data/workspace/mika-platform/.claude/worktrees/test-2471/mika \' \
    '  && git log --oneline -5 \' \
    '  && echo done [bash-cd] (non-terminal)' \
    > "$T2493_FIXTURE_DIR/multiline.stderr"
assert_eq "T4: refus multi-ligne, marqueur en 3ᵉ ligne (forme 98b60020) ⇒ non-terminal" \
    "non-terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/multiline.stderr")"

# T5 — plusieurs refus : la question est « un terminal existe-t-il ? », pas
# « le premier l'était-il ? ». `a0886164` en porte cinq.
printf '%s\n' \
    '[policy:deny] Bash: a [r1] (non-terminal)' \
    '[policy:deny] Bash: b [r2] (non-terminal)' \
    '[policy:deny] Bash: c [r3] (non-terminal)' \
    '[policy:deny] Bash: d [r4] (non-terminal)' \
    '[policy:deny] Bash: e [r5] (terminal)' \
    > "$T2493_FIXTURE_DIR/four_then_term.stderr"
assert_eq "T5: quatre non-terminaux suivis d'un terminal ⇒ terminal" \
    "terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/four_then_term.stderr")"

# T6 — CONTRÔLE NÉGATIF. `(non-terminal)` ne contient pas la sous-chaîne
# `(terminal)` : la parenthèse ouvrante qu'elle exige est occupée par le `-`.
# Inverser cette discrimination reclasserait d'un coup les 1093 refus
# non-terminaux mesurés — c'est l'erreur que ce test existe pour attraper, et
# elle ne se voit sur aucune des fixtures positives ci-dessus.
printf '%s\n' \
    '[policy:deny] Bash: x [r1] (non-terminal)' \
    '[policy:deny] Bash: y [r2] (non-terminal)' \
    > "$T2493_FIXTURE_DIR/only_nonterm.stderr"
assert_eq "T6 (contrôle négatif): un fichier de non-terminaux ne rend JAMAIS terminal" \
    "non-terminal" "$(_t2493_lethality "$T2493_FIXTURE_DIR/only_nonterm.stderr")"
# Et la vérification directe de la propriété de sous-chaîne dont tout dépend.
assert_eq "T6: la sous-chaîne (terminal) n'est pas contenue dans (non-terminal)" \
    "1" "$(printf '%s\n' 'x (non-terminal)' 'y (terminal)' | grep -c -F '(terminal)' || true)"

# T7 — fail-open préservé, dans les deux formes d'indisponibilité.
assert_eq "T7: fichier absent ⇒ undeclared (fail-open)" \
    "undeclared" "$(_t2493_lethality "$T2493_FIXTURE_DIR/does_not_exist.stderr")"
printf '[policy:deny] Bash: z [r] (terminal)\n' > "$T2493_FIXTURE_DIR/unreadable.stderr"
chmod 000 "$T2493_FIXTURE_DIR/unreadable.stderr" 2>/dev/null || true
if [ -r "$T2493_FIXTURE_DIR/unreadable.stderr" ]; then
    # root ignore les bits de permission : la sonde ne peut pas s'armer, et le
    # dire est ce qui empêche de lire un vert à vide (mika#2149).
    SKIPPED=$((SKIPPED + 1))
    echo "  SKIP T7: fichier illisible — le processus lit malgré chmod 000 (root ?)"
else
    assert_eq "T7: fichier illisible ⇒ undeclared (fail-open)" \
        "undeclared" "$(_t2493_lethality "$T2493_FIXTURE_DIR/unreadable.stderr")"
fi
chmod 644 "$T2493_FIXTURE_DIR/unreadable.stderr" 2>/dev/null || true

# --- T13–T14 : l'événement rapporté est dé-tronqué et borné (U3) -------------
#
# U3 est la condition de VÉRIFIABILITÉ de U2 : sans lui, le message affirme
# « non-terminal » en joignant une preuve où le marqueur n'apparaît pas (23 %
# des cas) — un troisième « croire sur parole » dans un ticket dont le sujet est
# un libellé qu'on a cru sur parole.

_t2493_excerpt() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        sed 's/\x1b\[[0-9;]*[mK]//g' "$1" | _policy_deny_excerpt
    )
}

T2493_EXCERPT_MULTILINE=$(_t2493_excerpt "$T2493_FIXTURE_DIR/multiline.stderr")
assert_contains "T13: l'extrait porte le [rule-id] (perdu par grep -m1)" \
    "[bash-cd]" "$T2493_EXCERPT_MULTILINE"
assert_contains "T13: l'extrait porte le marqueur de létalité" \
    "(non-terminal)" "$T2493_EXCERPT_MULTILINE"
assert_contains "T13: et il porte toujours la première ligne du refus" \
    "[policy:deny] Bash: cd /data/workspace" "$T2493_EXCERPT_MULTILINE"
# Contrôle de bonne foi : ce que l'ancien prédicat rendait sur cette même
# fixture. Si ce test rougit, c'est que la fixture ne reproduit plus la forme
# de M2 et que T13 ne mesure plus rien.
assert_not_contains "T13 (bonne foi): grep -m1 sur la même fixture PERD le rule-id" \
    "[bash-cd]" "$(sed 's/\x1b\[[0-9;]*[mK]//g' "$T2493_FIXTURE_DIR/multiline.stderr" | grep -m1 '\[policy:deny\]' || true)"

# T14 — bornes de D6 : un refus mono-ligne sans marqueur suivi de bruit de
# journal s'arrête à la première ligne ouvrant un autre événement.
printf '%s\n' \
    '[policy:deny] Bash: gh auth status' \
    '[debug] tool_use id=abc' \
    '[debug] tool_result bytes=1200' \
    '[done] Success | 5 turns' \
    > "$T2493_FIXTURE_DIR/deny_then_noise.stderr"
T2493_EXCERPT_NOISE=$(_t2493_excerpt "$T2493_FIXTURE_DIR/deny_then_noise.stderr")
assert_contains "T14: l'extrait porte le refus" \
    "[policy:deny] Bash: gh auth status" "$T2493_EXCERPT_NOISE"
assert_not_contains "T14: et n'emporte pas le bruit [debug] qui suit" \
    "[debug]" "$T2493_EXCERPT_NOISE"
assert_not_contains "T14: ni la ligne de fin de session" \
    "[done]" "$T2493_EXCERPT_NOISE"
# Plafond : sans marqueur ET sans autre événement, la capture reste bornée.
{
    printf '[policy:deny] Bash: long-command \\\n'
    for _i in $(seq 1 40); do printf '  arg%s \\\n' "$_i"; done
} > "$T2493_FIXTURE_DIR/deny_unbounded.stderr"
assert_eq "T14: sans marqueur ni autre événement, la capture est plafonnée" \
    "12" "$(_t2493_excerpt "$T2493_FIXTURE_DIR/deny_unbounded.stderr" | grep -c . || true)"

# --- T8 : garde de préemption, formulée sur la POPULATION des sites ----------
#
# Formulé sur la population — toute branche qui déclare un halt sur un deny —
# et non sur deux lignes nommées, pour qu'un troisième site futur soit vu.
# M5 est la mesure qui requalifie U1 : les trois autres branches de la chaîne
# du site dev-groom portaient DÉJÀ `[ -z "$VALID_PLAN" ]`. La branche de refus
# était la seule à ne pas la porter — et c'est elle qui est en tête. U1 rend à
# cette branche la condition que ses sœurs ont ; il n'en invente pas une.
#
# « On déclare, on n'allowliste pas » (mika#2201, mika#2323) : quand ce scan
# tire, la résolution est d'ajouter la garde au site fautif, JAMAIS d'ajouter
# une entrée ici.
POLICY_DENY_UNGUARDED_ALLOWED=""   # mika#2493 : LIVRÉE VIDE, et un test le tient
assert_eq "T8: allowlist des sites de halt sans garde — zero entries" \
    "" "$POLICY_DENY_UNGUARDED_ALLOWED"

# Le scan associe chaque libellé de halt à la condition de la branche qui
# l'englobe, et vérifie que cette condition porte les DEUX gardes. La portée de
# `cond` est bornée à 15 lignes pour qu'un libellé lointain n'hérite pas de la
# condition d'un autre bloc.
T2493_HALT_SCAN=$(awk '
    /^[[:space:]]*(el)?if .*\$POLICY_DENY/ { cond = $0; age = 0; next }
    cond != "" { age++; if (age > 15) cond = "" }
    /RESULT="PIPELINE FAILURE:.*halted by.*policy deny/ {
        seen++
        if (cond !~ /-z "\$VALID_PLAN"/) bad_plan++
        if (cond !~ /POLICY_DENY_LETHALITY" = "terminal"/) bad_leth++
    }
    END { printf "%d:%d:%d", seen + 0, bad_plan + 0, bad_leth + 0 }
' "$DISPATCH_LIB")
# Bonne foi d'abord : un scan qui ne trouve aucun site passerait vert à vide.
assert_eq "T8 (bonne foi): le scan voit les deux sites de halt existants" \
    "yes" "$([ "${T2493_HALT_SCAN%%:*}" -ge 2 ] && echo yes || echo "non ($T2493_HALT_SCAN)")"
assert_eq "T8: tout site de halt sur deny porte la garde [ -z \"\$VALID_PLAN\" ]" \
    "0" "$(printf '%s' "$T2493_HALT_SCAN" | cut -d: -f2)"
assert_eq "T8: tout site de halt sur deny est réservé à la létalité terminal" \
    "0" "$(printf '%s' "$T2493_HALT_SCAN" | cut -d: -f3)"

# L'ORDRE DES CONJOINTS est porteur (D1) : les deux assertions d'ordre
# existantes (Test 13, Test 14) cherchent la sous-chaîne littérale
# `if [ -n "$POLICY_DENY" ]`. Mettre la garde en tête les ferait rougir pour un
# résultat identique — et donnerait l'apparence d'un fix qui corrige ses tests.
assert_eq "D1: les deux sites gardent POLICY_DENY en conjoint de tête" \
    "2" "$(grep -c 'if \[ -n "\$POLICY_DENY" \] && \[ -z "\$VALID_PLAN" \]' "$DISPATCH_LIB" || true)"

# --- T9, T10, T11, T15, T16 : comportement bout-en-bout ---------------------
#
# Les deux sens du DoD, sur la forme exacte des deux preuves : dev-groom,
# HEAD inchangé, un refus dans le stderr — et `VALID_PLAN` peuplé ou vide.

_t2493_e2e() {
    # $1 = chemin d'une fixture stderr ; $2 = "plan" pour peupler VALID_PLAN.
    local base_dir wt_dir log_dir pre_head
    base_dir=$(mktemp -d); wt_dir="$base_dir/wt"; log_dir="$base_dir/logs"
    mkdir -p "$log_dir" "$wt_dir/docs/plans"
    git init -q "$wt_dir" 2>/dev/null
    printf 'seed\n' > "$wt_dir/seed.txt"
    if [ "${2:-}" = "plan" ]; then
        # >500 octets (seuil mika#1033) + en-tête nommant l'issue (tier 2).
        {
            printf '# Plan mika issue#9493\n\n**Ticket:** mika issue#9493\n\n'
            for _i in $(seq 1 40); do printf 'Ligne de corps du plan, pour franchir le seuil de 500 octets.\n'; done
        } > "$wt_dir/docs/plans/2026-09-23-001-fix-9493-sonde-plan.md"
    fi
    git -C "$wt_dir" add -A 2>/dev/null
    git -C "$wt_dir" commit -q -m "seed" 2>/dev/null
    pre_head=$(git -C "$wt_dir" rev-parse HEAD)
    cp "$1" "$log_dir/t2493probe.stderr"

    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB"

        # Herméticité : aucune requête forge. Le détecteur mika#2178 T9 balaie
        # tout ce qui suit sa section — ces redéfinitions sont ce qui permet à
        # la sonde de tourner sans réseau ni jeton.
        _pr_list_url() { _LAST_PR_QUERY_RC=0; printf ''; }
        _stamp_pr_origin() { return 0; }

        PILOT_LOG_DIR="$log_dir"
        PRE_RUN_HEAD="$pre_head"
        POST_RUN_HEAD="$pre_head"     # HEAD inchangé — la forme des deux preuves
        WORKTREE_DIR="$wt_dir"
        SKILL="dev-groom"
        REPO="mika"
        BRANCH="fix/9493/sonde"
        ISSUE_NUM="9493"
        SESSION_ID="sonde-2493"
        LOG_ID="t2493probe"
        STATUS="success"
        PILOT_EXIT=0
        RESULT="claude-pilot session completed."
        RESCUED_DIRTY_WORKTREE=0

        exec 9>&2
        _post_flight_recovery 2>/dev/null

        printf '%s' "$RESULT"
    )

    rm -rf "$base_dir"
}

# T9 — DoD point 1. La population fondatrice : dev-groom qui a livré son plan,
# refus non-terminal survécu. Ni « PIPELINE FAILURE » ni « halted ».
T2493_E2E_SURVIVED=$(_t2493_e2e "$T2493_FIXTURE_DIR/nonterm.stderr" plan 2>/dev/null)
assert_not_contains "T9 (DoD-1): plan livré + deny non-terminal ⇒ pas de PIPELINE FAILURE" \
    "PIPELINE FAILURE:" "$T2493_E2E_SURVIVED"
assert_not_contains "T9 (DoD-1): plan livré + deny non-terminal ⇒ pas de « halted »" \
    "halted" "$T2493_E2E_SURVIVED"
# AC4 — le refus est REQUALIFIÉ, jamais supprimé : il reste visible en annexe.
assert_contains "AC4: le refus non-terminal reste visible dans le result (note)" \
    "the session continued past it" "$T2493_E2E_SURVIVED"
assert_contains "AC4: et la note joint la preuve du refus" \
    "Observed deny:" "$T2493_E2E_SURVIVED"
assert_contains "AC4: la note nomme la létalité lue" \
    "Lethality marker: (non-terminal)" "$T2493_E2E_SURVIVED"
# Idempotence : les deux sites peuvent tirer sur un même dispatch dev-groom.
assert_eq "La note n'est annexée qu'une fois, même quand les deux sites tirent" \
    "1" "$(grep -c 'the session continued past it' <<<"$T2493_E2E_SURVIVED" || true)"

# T15 — LE TEST DE L'EFFET, et non de la formulation (M6). Le faux
# `PIPELINE FAILURE:` ne se contentait pas de mal nommer : il CAPTURAIT le
# classificateur de la ligne `Outcome:`. T9 assertte une ABSENCE de sous-chaîne,
# qu'une reformulation malheureuse pourrait satisfaire sans rien réparer ; T15
# assertte la PRÉSENCE du bon classement, qu'on ne peut pas obtenir par accident.
assert_contains "T15 (AC6): la session est classée PLAN_COMMITTED" \
    "Outcome: PLAN_COMMITTED" "$T2493_E2E_SURVIVED"
assert_not_contains "T15 (AC6): et non PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE" "$T2493_E2E_SURVIVED"

# T16 — CONTRÔLE NÉGATIF de T15. Même fixture, `VALID_PLAN` vide : une session
# qui n'a réellement rien produit continue d'être classée PIPELINE_INCOMPLETE.
# C'est ce qui distingue « le correctif répare le faux positif » de « le
# correctif a désarmé le classificateur ».
T2493_E2E_NOPLAN=$(_t2493_e2e "$T2493_FIXTURE_DIR/nonterm.stderr" 2>/dev/null)
assert_contains "T16 (contrôle négatif): sans plan, la session reste PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE" "$T2493_E2E_NOPLAN"
assert_contains "T16: et le diagnostic de la branche qui s'applique garde la parole" \
    "_find_issue_plan returned empty" "$T2493_E2E_NOPLAN"
# D4 : le refus est annexé, il n'ÉVINCE pas le vrai diagnostic. Remplacer
# « pas de plan trouvé, causes probables (a) dérive (b) bug de découverte » par
# « halted by policy deny » déplacerait le mensonge du ticket au lieu de le
# fermer — on substituerait au vrai diagnostic un faux, sous couvert de précision.
assert_contains "D4: sans plan non plus, le deny non-terminal n'est qu'une annexe" \
    "the session continued past it" "$T2493_E2E_NOPLAN"
assert_not_contains "D4: et le verbe « halted » n'apparaît pas pour autant" \
    "halted" "$T2493_E2E_NOPLAN"

# T10 — DoD point 2, le SENS INVERSE. Un refus terminal produit toujours le
# libellé, augmenté de la mention de sa létalité. Le verbe reste disponible
# pour les vrais halts : c'est la moitié qu'un correctif trop large casserait.
T2493_E2E_TERMINAL=$(_t2493_e2e "$T2493_FIXTURE_DIR/term.stderr" 2>/dev/null)
assert_contains "T10 (DoD-2): deny terminal ⇒ le result porte « halted »" \
    "halted" "$T2493_E2E_TERMINAL"
assert_contains "T10 (DoD-2): et la mention (terminal)" \
    "(terminal)" "$T2493_E2E_TERMINAL"
assert_contains "T10: le libellé historique du site dev-groom est conservé (REQ7)" \
    "halted by claude-pilot policy deny" "$T2493_E2E_TERMINAL"
assert_contains "T10: et la session est bien classée en échec" \
    "Outcome: PIPELINE_INCOMPLETE" "$T2493_E2E_TERMINAL"
# AC3 : les deux sens sont deux tests distincts, et T5/T6 tiennent la frontière.
assert_not_contains "T10: un halt terminal n'annexe pas en plus la note de survie" \
    "the session continued past it" "$T2493_E2E_TERMINAL"

# T11 — D5, contrainte DURE sur la note, assertée sur le TEXTE PRODUIT et non
# sur la constante. `dispatch-lib.sh` grep le contenu de `RESULT` pour cinq
# jetons ; une note d'information qui en introduirait un reclasserait la session
# en échec — exactement le défaut réparé, reconstruit par le correctif.
T2493_NOTE_ONLY=$(
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        RESULT=""
        _annex_policy_deny_note "non-terminal" "[policy:deny] Bash: x [r] (non-terminal)"
        printf '%s' "$RESULT"
    )
)
assert_contains "T11 (bonne foi): la sonde produit bien la note" \
    "Observed deny:" "$T2493_NOTE_ONLY"
for _tok in "PIPELINE FAILURE:" "STRUCTURAL VIOLATION:" "HANDLER CRASH"; do
    assert_not_contains "T11 (D5): la note ne porte pas le jeton « $_tok »" \
        "$_tok" "$T2493_NOTE_ONLY"
done
assert_eq "T11 (D5): la note n'ouvre aucune ligne par STATUS=CANCELLED" \
    "0" "$(grep -c '^STATUS=CANCELLED' <<<"$T2493_NOTE_ONLY" || true)"
assert_eq "T11 (D5): la note n'ouvre aucune ligne par Outcome: PIPELINE_INCOMPLETE" \
    "0" "$(grep -c '^Outcome: PIPELINE_INCOMPLETE' <<<"$T2493_NOTE_ONLY" || true)"
# Le même prédicat que le lecteur de reclassement applique, appliqué à la note.
assert_eq "T11 (D5): le prédicat de reclassement de dispatch-lib ne mord pas sur la note" \
    "0" "$(grep -cE '(PIPELINE FAILURE:|STRUCTURAL VIOLATION:|HANDLER CRASH|^STATUS=CANCELLED|^Outcome: PIPELINE_INCOMPLETE)' <<<"$T2493_NOTE_ONLY" || true)"

# T3 bout-en-bout — REQ4/AC5 : une létalité non déclarée n'affirme rien, et le
# message dit POURQUOI, en nommant le build du pilote plutôt que la lecture.
T2493_E2E_UNDECLARED=$(_t2493_e2e "$T2493_FIXTURE_DIR/undeclared.stderr" plan 2>/dev/null)
assert_contains "AC5: létalité non déclarée ⇒ ni terminal ni non-terminal affirmé" \
    "Lethality marker: undeclared" "$T2493_E2E_UNDECLARED"
assert_contains "AC5: et le message impute l'absence au build du pilote (cpp#151)" \
    "cpp#151" "$T2493_E2E_UNDECLARED"
assert_not_contains "AC5: un undeclared ne produit pas « halted »" \
    "halted" "$T2493_E2E_UNDECLARED"

# --- Herméticité de cette section (patron mika#2178 T9) ---------------------
T2493_SECTION=$(sed -n '/^# mika#2493 — un deny non-terminal survécu est une NOTE/,$p' "${BASH_SOURCE[0]}")
assert_eq "Herméticité: l'extraction de la section mika#2493 a trouvé la section" \
    "yes" "$(if [ -n "$T2493_SECTION" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "Herméticité: aucune invocation de forge en tête de commande" \
    "0" "$(printf '%s\n' "$T2493_SECTION" | grep -cE '^[[:space:]]*gh[[:space:]]' || true)"
assert_eq "Herméticité: aucune invocation de forge en substitution" \
    "0" "$(printf '%s\n' "$T2493_SECTION" | grep -cE '\$\(gh[[:space:]]' || true)"

rm -rf "$T2493_FIXTURE_DIR"

# ============================================================================
# mika#2539 — le jeton de cause DÉRIVE de _halt_family, il ne la redéclare pas
#
# Ces scans sont structurels et non comportementaux, et c'est une nécessité :
# une seconde table de classification ne rendrait AUCUNE décision fausse le jour
# où on l'écrit. Elle rendrait `_rescue_cause_token` juste, et la laisserait
# dériver silencieusement ensuite — un sous-type ajouté en amont ferait rougir le
# drift guard T6 et passerait sans un mot dans le message de sauvetage. C'est la
# panne que mika#2158 a mesurée sur `auto_pull.rs` : une regex commentée
# « Mirrors GROOMED_VERDICT_RE » qui n'a suivi aucun des deux élargissements
# suivants, et promotion et routage ont répondu différemment à la même question
# pendant des mois. Aucune suite comportementale ne voit cette classe.
#
# T6 n'est ni touché ni dupliqué : il couvre déjà le jeton par construction,
# puisque le jeton EST la sortie de `_halt_family`.
# ============================================================================
echo ""
echo "mika#2539: le jeton de cause du sauvetage dérive de la table de halte"
echo "---------------------------------------------------------------------"

T2539_TOKEN_FN=$(sed -n '/^_rescue_cause_token() {$/,/^}$/p' "$DISPATCH_LIB")
T2539_RESOLVER_FN=$(sed -n '/^_resolve_halt_subtype() {$/,/^}$/p' "$DISPATCH_LIB")

# S3 (anti-vacuité) — AVANT tout le reste. Un scan qui regarde une fonction
# disparue ou vide se lit exactement comme un arbre propre (classe mika#2205),
# et les scans S1/S2 ci-dessous seraient vacuement verts sur une chaîne vide.
assert_eq "S3 (anti-vacuité): _rescue_cause_token existe et son corps est non vide" "yes" \
    "$(if [ "$(printf '%s\n' "$T2539_TOKEN_FN" | grep -c .)" -gt 3 ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "S3 (anti-vacuité): _resolve_halt_subtype existe et son corps est non vide" "yes" \
    "$(if [ "$(printf '%s\n' "$T2539_RESOLVER_FN" | grep -c .)" -gt 3 ]; then printf 'yes'; else printf 'no'; fi)"

# S1 — la délégation, mesurée. Sans cet appel, le jeton porte une table à soi.
assert_contains "S1: _rescue_cause_token consulte _halt_family" \
    "_halt_family" "$T2539_TOKEN_FN"
assert_contains "S1: et il en tire la famille (premier champ de la ligne)" \
    '%%|*' "$T2539_TOKEN_FN"

# S2 — aucun nom de sous-type amont dans le corps. C'est R-3 rendu exécutable :
# la seule forme que prendrait une seconde table.
#
# La population des sous-types interdits est DÉRIVÉE de `_halt_family` elle-même,
# jamais recopiée ici. Une liste en dur aurait exactement le défaut qu'elle
# prétend interdire : elle cesserait de couvrir le prochain sous-type ajouté à la
# table, donc le scan rétrécirait en silence pendant que l'arbre reste vert.
T2539_SUBTYPES=$(sed -n '/^_halt_family() {$/,/^}$/p' "$DISPATCH_LIB" \
    | grep -oE '^        [a-z][a-z0-9_]*\)' | tr -d ' )' || true)
# Anti-vacuité de la dérivation : la table en compte douze aujourd'hui. Si
# l'extraction en rend une poignée, l'ancre `case` ne matche plus et S2 ne
# regarde plus rien.
assert_eq "S2 (anti-vacuité): la population de sous-types est dérivée de la table" "yes" \
    "$(if [ "$(printf '%s\n' "$T2539_SUBTYPES" | grep -c .)" -ge 12 ]; then printf 'yes'; else printf 'no'; fi)"

# Une entrée = un site qui énumère un sous-type de halte hors de _halt_family,
# au format `<sous-type>|<raison + ticket de suivi>`. LIVRÉE VIDE : les deux
# fonctions scannées sont créées par mika#2539, donc aucune violation
# préexistante ne peut exister.
#
# QUAND CE SCAN TIRE, LA RÉSOLUTION EST DE ROUTER LE SITE VERS _halt_family —
# JAMAIS d'ajouter une ligne ici (doctrine mika#2201). Un site qu'on ne veut pas
# router est un site à supprimer.
RESCUE_CAUSE_SUBTYPE_ALLOWED=()

T2539_LEAKED=""
for _t2539_st in $T2539_SUBTYPES; do
    grep -qF -- "$_t2539_st" <<<"$T2539_TOKEN_FN" || continue
    _t2539_excused=no
    for _t2539_entry in ${RESCUE_CAUSE_SUBTYPE_ALLOWED+"${RESCUE_CAUSE_SUBTYPE_ALLOWED[@]}"}; do
        [ "${_t2539_entry%%|*}" = "$_t2539_st" ] && _t2539_excused=yes && break
    done
    [ "$_t2539_excused" = "yes" ] || T2539_LEAKED="${T2539_LEAKED}${_t2539_st} "
done
assert_eq "S2: aucun sous-type amont énuméré dans _rescue_cause_token" "" \
    "$(printf '%s' "$T2539_LEAKED" | sed 's/ *$//')"

# Le double sens — l'assertion auto-nettoyante. Une entrée qui ne matche plus
# rien fait rougir le build le jour de la réparation, pas des mois après.
T2539_STALE=""
for _t2539_entry in ${RESCUE_CAUSE_SUBTYPE_ALLOWED+"${RESCUE_CAUSE_SUBTYPE_ALLOWED[@]}"}; do
    _t2539_st="${_t2539_entry%%|*}"
    grep -qF -- "$_t2539_st" <<<"$T2539_TOKEN_FN" \
        || T2539_STALE="${T2539_STALE}${_t2539_st} "
done
assert_eq "S2 (double sens): aucune entrée d'allowlist périmée" "" \
    "$(printf '%s' "$T2539_STALE" | sed 's/ *$//')"

# S4 — un seul résolveur. Un troisième site qui résoudrait le sous-type à la
# main rouvrirait la divergence que `_resolve_halt_subtype` existe pour fermer :
# la bannière du callback dirait `Halt class: session_silent` pendant que le
# commit dirait `rescue no_halt_signal`, pour une même session, l'un des deux
# gravé dans l'historique git pour toujours.
#
# Le compte est fait hors définition et hors commentaire — seuls les appels.
T2539_RESOLVER_CALLS=$(grep -n '_resolve_halt_subtype' "$DISPATCH_LIB" \
    | grep -v '_resolve_halt_subtype() {' \
    | grep -vE ':[[:space:]]*#' | grep -c . || true)
assert_eq "S4: _resolve_halt_subtype a exactement deux appelants de production" "2" \
    "$T2539_RESOLVER_CALLS"
assert_contains "S4: l'un est la bannière du callback" \
    '_resolve_halt_subtype' "$(sed -n '/^_classify_terminated_session() {$/,/^}$/p' "$DISPATCH_LIB")"
assert_contains "S4: l'autre est le compositeur du sujet de sauvetage" \
    '_resolve_halt_subtype' "$T2539_TOKEN_FN"

# Le mémo est réinitialisé au site qui lit le sous-type d'une NOUVELLE session,
# et pas chez l'un des deux appelants : « une session, une réponse » est ainsi
# une propriété de la construction et non de la mémoire du prochain éditeur.
assert_eq "S4: le mémo est réinitialisé exactement une fois, au site de SUBTYPE" "1" \
    "$(grep -c '^    unset _HALT_SUBTYPE_RESOLVED _HALT_GUARDRAIL_LINE$' "$DISPATCH_LIB")"

# S5 — dispatch-lib parse toujours.
T2539_RC=0
bash -n "$DISPATCH_LIB" 2>/dev/null || T2539_RC=$?
assert_eq "S5: dispatch-lib.sh passe bash -n" "0" "$T2539_RC"

# --- Herméticité de cette section (patron mika#2178 T9 / mika#2493) ---------
T2539_SECTION=$(sed -n '/^# mika#2539 — le jeton de cause DÉRIVE de _halt_family/,$p' "${BASH_SOURCE[0]}")
assert_eq "Herméticité: l'extraction de la section mika#2539 a trouvé la section" \
    "yes" "$(if [ -n "$T2539_SECTION" ]; then printf 'yes'; else printf 'no'; fi)"
assert_eq "Herméticité: aucune invocation de forge en tête de commande" \
    "0" "$(printf '%s\n' "$T2539_SECTION" | grep -cE '^[[:space:]]*gh[[:space:]]' || true)"
assert_eq "Herméticité: aucune invocation de forge en substitution" \
    "0" "$(printf '%s\n' "$T2539_SECTION" | grep -cE '\$\(gh[[:space:]]' || true)"

# ============================================================================
# mika#2545 — un ESCALATE de groom est TERMINAL, et son producteur l'estampille
# ============================================================================
#
# Le défaut mesuré : `_escalate_groom` écrivait `PIPELINE FAILURE:`, marqueur
# qui fait entrer le tour dans la population RETRYABLE de `self-dev-callback`.
# Un verdict de halte — terminal par contrat de `/mika-groom-ticket` — était
# donc rejoué : huit dispatches sur mika#2542 le 2026-09-26, chacun ESCALATE
# sur la même cause.
#
# Le producteur pose désormais son propre fait : `Outcome: ESCALATE — <stage>`,
# écrit par `_set_outcome_line` (mika#2492) donc unique par construction, et le
# marqueur retryable disparaît. Les assertions ci-dessous couvrent les trois
# stages, la non-régression de la population retryable LÉGITIME, et la
# non-régression du gate mika#1996 — dont le terme de reclassement est la part
# fragile de ce correctif : `PIPELINE FAILURE:` était aussi ce qui sortait le
# cycle de `empty_completion`.

echo ""
echo "Test: un ESCALATE de groom est terminal (mika#2545)"
echo "-------------------------------------------------------------------"

# Sonde comportementale : `_escalate_groom` est appelable hors contexte — elle
# ne lit que WORKTREE_DIR, RESULT et ses trois arguments.
_t2545_escalate_probe() {
    local stage="$1" content="$2" result_in="${3:-}"
    local base wt
    base=$(mktemp -d "${TMPDIR:-/tmp}/mika-2545-test.XXXXXX")
    wt="$base/wt"; mkdir -p "$wt"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        WORKTREE_DIR="$wt"
        RESULT="$result_in"
        _escalate_groom "$stage" "$content" "sess-2545" 2>/dev/null
        printf '%s' "$RESULT"
    )
    rm -rf "$base"
}

T2545_BASE_RESULT="claude-pilot completed (status: success).
Session: sess-2545
Turns: 12"

# --- Les trois stages : le fait est posé, le marqueur retryable a disparu ----
for _t2545_stage in first-pass second-pass-after-ready second-pass-after-iterate; do
    T2545_OUT=$(_t2545_escalate_probe "$_t2545_stage" "F1: le plan ne cite rien." "$T2545_BASE_RESULT")
    assert_eq "AC3 ($_t2545_stage): exactement une ligne Outcome: ESCALATE" \
        "1" "$(printf '%s\n' "$T2545_OUT" | grep -c "^Outcome: ESCALATE" || true)"
    assert_eq "AC3 ($_t2545_stage): exactement une ligne Outcome, toutes valeurs confondues" \
        "1" "$(printf '%s\n' "$T2545_OUT" | grep -c '^Outcome: ' || true)"
    assert_not_contains "AC3 ($_t2545_stage): plus aucun marqueur de la population retryable" \
        "PIPELINE FAILURE:" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): la ligne Outcome nomme le stage" \
        "Outcome: ESCALATE — $_t2545_stage" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): le corps nomme sa classe terminale" \
        "GROOM ESCALATED (terminal):" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): la session reste lisible" \
        "Session: sess-2545" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): le verdict reste lisible" \
        "Verdict: ESCALATE" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): le chemin des findings est nommé" \
        "escalate-${_t2545_stage}.md" "$T2545_OUT"
    assert_contains "AC3 ($_t2545_stage): le corps du RESULT antérieur est préservé" \
        "Turns: 12" "$T2545_OUT"
done

# Une ligne `Outcome:` déjà posée par la validation de plan est REMPLACÉE, pas
# doublée — c'est la propriété que `_set_outcome_line` garantit par construction
# et la seule raison de ne pas concaténer à la main ici.
T2545_OVERWRITE=$(_t2545_escalate_probe first-pass "F1: rien." "$T2545_BASE_RESULT

Outcome: PIPELINE_INCOMPLETE — no_shipping_tail: dispatch-lib did not reach PR creation.")
assert_eq "AC3: une ligne Outcome préexistante est remplacée, jamais doublée" \
    "1" "$(printf '%s\n' "$T2545_OVERWRITE" | grep -c '^Outcome: ' || true)"
assert_contains "AC3: et c'est l'ESCALATE qui reste" \
    "Outcome: ESCALATE — first-pass" "$T2545_OVERWRITE"

# La cause moteur de mika#2338 voyage toujours : ce ticket change la CLASSE du
# marqueur, pas la substance transmise.
T2545_ENGINE=$(_t2545_escalate_probe first-pass \
    "F1: (BLOCKING) [mika-engine] review-anchor: attestation withheld after the corrective re-prompt." \
    "$T2545_BASE_RESULT")
assert_contains "mika#2338 (non-régression): la cause moteur voyage toujours dans RESULT" \
    "Engine reason:" "$T2545_ENGINE"

# --- Cardinalité : un seul site d'écriture du fait --------------------------
#
# Sans cette assertion, un prédicat devenu trop étroit passerait en ne regardant
# rien (classe mika#2205). Elle est aussi ce qui refuse un second écrivain : le
# fait doit être posé par son producteur, et par lui seul.
T2545_OUTCOME_WRITERS_ALLOWED=""   # mika#2545: livrée VIDE, et l'assertion suivante le tient
assert_eq "allowlist des écrivains de Outcome: ESCALATE — zero entries" \
    "" "$T2545_OUTCOME_WRITERS_ALLOWED"
assert_eq "exactement un site d'écriture de Outcome: ESCALATE dans dispatch-lib" \
    "1" "$(grep -c '_set_outcome_line "Outcome: ESCALATE' "$DISPATCH_LIB" || true)"
# Les commentaires sont retirés avant le scan : la prose de cette fonction nomme
# légitimement le marqueur qu'elle n'écrit plus — c'est le faux positif que
# mika#2050 a mesuré sur le Signal S, et le même geste que GATE_CODE au test 15.
T2545_ESCALATE_FN=$(sed -n '/^_escalate_groom() {$/,/^}$/p' "$DISPATCH_LIB" | grep -v '^[[:space:]]*#')
assert_contains "et ce site est _escalate_groom elle-même" \
    '_set_outcome_line "Outcome: ESCALATE' "$T2545_ESCALATE_FN"
assert_not_contains "_escalate_groom n'écrit plus le marqueur de la population retryable" \
    "PIPELINE FAILURE:" "$T2545_ESCALATE_FN"
assert_contains "bonne foi: l'extraction du corps de _escalate_groom a bien trouvé son code" \
    "GROOM ESCALATED (terminal):" "$T2545_ESCALATE_FN"

# --- Le gate mika#1996 ne reclasse PAS un ESCALATE en cycle vide ------------
#
# La part fragile du correctif. `PIPELINE FAILURE:` sortait le cycle de
# `empty_completion` ; le retirer exposerait un ESCALATE à un faux rouge sur une
# sortie délibérée. Deux termes le couvrent et il faut les DEUX — le second cas
# ci-dessous (compte d'appels d'outils NON MESURÉ) est celui que P4 ne peut pas
# rattraper, puisque sa conjonction exige `CYCLE_TOOL_CALLS >= 1`.
T2545_ESCALATE_RESULT="claude-pilot completed (status: success).
GROOM ESCALATED (terminal): mika-arch escalated at first-pass.
Verdict: ESCALATE — human review required.

Outcome: ESCALATE — first-pass"

T2545_GATE_UNMEASURED=$(_gate_probe result no none "" none "$T2545_ESCALATE_RESULT")
assert_not_contains "AC7: un ESCALATE dont le compte d'outils n'est pas mesuré n'est pas reclassé vide" \
    "PIPELINE FAILURE: empty_completion" "$T2545_GATE_UNMEASURED"
assert_contains "AC7: et il garde sa propre disposition" \
    "Outcome: ESCALATE — first-pass" "$T2545_GATE_UNMEASURED"

T2545_GATE_ZERO=$(_gate_probe result no none "" 0 "$T2545_ESCALATE_RESULT")
assert_not_contains "AC7: un ESCALATE mesuré à zéro appel d'outil n'est pas reclassé vide non plus" \
    "PIPELINE FAILURE: empty_completion" "$T2545_GATE_ZERO"

# Contrôle POSITIF du gate : P4 continue de reconnaître l'ESCALATE comme une
# disposition motivée quand le compte est mesuré. Sans ce contrôle, « le terme
# ajouté mord » ne se distingue pas de « P4 ne voit plus rien ».
T2545_GATE_VERDICT=$(_gate_probe verdict no none "" 3 "$T2545_ESCALATE_RESULT")
assert_eq "AC7 (contrôle positif): un ESCALATE avec des appels d'outils est 'produced' par P4" \
    "produced" "$T2545_GATE_VERDICT"

# Contrôle NÉGATIF du gate : la population `empty_completion` existe toujours.
T2545_GATE_STILL_EMPTY=$(_gate_probe result no none "" 0 "claude-pilot completed (status: success).")
assert_contains "AC7 (contrôle négatif): un cycle réellement vide est toujours banni" \
    "PIPELINE FAILURE: empty_completion" "$T2545_GATE_STILL_EMPTY"

# --- Non-régression : la population retryable LÉGITIME est intacte ----------
T2545_REAL_FAILURE="PIPELINE FAILURE: the claude-pilot session was terminated before it produced any work.

Outcome: PIPELINE_INCOMPLETE — pilot session terminated by claude-pilot before producing work."
T2545_GATE_REAL=$(_gate_probe result no none "" 0 "$T2545_REAL_FAILURE")
assert_contains "AC6: un PIPELINE FAILURE authentique garde son marqueur" \
    "PIPELINE FAILURE: the claude-pilot session was terminated" "$T2545_GATE_REAL"
assert_contains "AC6: et son Outcome: PIPELINE_INCOMPLETE" \
    "Outcome: PIPELINE_INCOMPLETE — pilot session terminated" "$T2545_GATE_REAL"
assert_eq "AC6: sans bannière empilée" \
    "1" "$(printf '%s\n' "$T2545_GATE_REAL" | grep -c 'PIPELINE FAILURE:' || true)"

# --- La branche `else` de dispatch_claude_pilot PRÉSERVE un ESCALATE --------
#
# Son texte — « grooming did not converge » — reste juste pour ses dix-sept
# autres sorties ; il est FAUX pour un ESCALATE, qui a convergé, sur un verdict
# de halte. Scan de source : ce bras n'est pas atteignable hors d'un dispatch
# complet (worktree, pilote, architecte).
T2545_CONVERGENCE_BLOCK=$(sed -n '/^        if _iterate_groom_loop; then$/,/^    fi$/p' "$DISPATCH_LIB")
assert_eq "AC3 (bonne foi): le bloc de convergence de dispatch_claude_pilot a été extrait" \
    "yes" "$(if [ -n "$T2545_CONVERGENCE_BLOCK" ]; then printf 'yes'; else printf 'no'; fi)"
assert_contains "AC3: un bras consulte la ligne Outcome: ESCALATE avant toute réécriture" \
    "elif grep -qE '^Outcome: ESCALATE'" "$T2545_CONVERGENCE_BLOCK"
assert_contains "AC3: et l'écriture PIPELINE FAILURE reste présente pour les autres sorties" \
    "PIPELINE FAILURE: grooming did not converge" "$T2545_CONVERGENCE_BLOCK"
# L'ordre est porteur : le bras ESCALATE doit précéder celui qui réécrit
# `Outcome: .*` au sed, sinon il n'est jamais atteint.
T2545_ESC_POS=$(printf '%s\n' "$T2545_CONVERGENCE_BLOCK" | grep -n "elif grep -qE '\^Outcome: ESCALATE'" | head -1 | cut -d: -f1)
T2545_SED_POS=$(printf '%s\n' "$T2545_CONVERGENCE_BLOCK" | grep -n 's/Outcome: .\*/Outcome: PIPELINE_INCOMPLETE' | head -1 | cut -d: -f1)
assert_eq "AC3: le bras ESCALATE précède la réécriture sed de la ligne Outcome" "yes" \
    "$([ -n "$T2545_ESC_POS" ] && [ -n "$T2545_SED_POS" ] && [ "$T2545_ESC_POS" -lt "$T2545_SED_POS" ] && echo yes || echo "non ($T2545_ESC_POS vs $T2545_SED_POS)")"

# Le chemin GROOMED est inchangé à l'octet près — c'est la moitié qui garantit
# que ce ticket n'a pas déplacé le succès en même temps que l'échec.
T2545_GROOMED_ARM=$(sed -n '/mika#1394: Architect converged on GROOMED/,/^        elif grep -qE/p' "$DISPATCH_LIB")
assert_contains "AC6: le chemin GROOMED écrit toujours Outcome: PLAN_GROOMED" \
    "Outcome: PLAN_GROOMED" "$T2545_GROOMED_ARM"
assert_contains "AC6: et strippe toujours les marqueurs PIPELINE FAILURE périmés" \
    "sed '/^PIPELINE FAILURE:/d'" "$T2545_GROOMED_ARM"

# ===========================================================================
# mika#2578 (D2) — le chemin de staging du jeton a UN site de déclaration,
# conditionnel.
# ===========================================================================
# Aucun test comportemental ne peut voir cette classe : une régression vers
# l'affectation inconditionnelle (`_PILOT_GH_TOKEN_FILE="$HOME/…"`) laisse
# TOUTES les assertions de staging vertes — le défaut est que la pose d'un
# harness redevient silencieusement inerte au `source`, donc que le canari se
# remet à écraser la credential GitHub de l'hôte sans que rien ne rougisse.

# Anti-vacuité d'abord : un scan dont le chemin pourrit se lit exactement comme
# un arbre propre (classe mika#2205).
T2578_BYTES=$(wc -c < "$DISPATCH_LIB" 2>/dev/null || echo 0)
assert_eq "mika#2578 anti-vacuité: dispatch-lib.sh est bien le vrai fichier" "yes" \
    "$([ "${T2578_BYTES:-0}" -gt 100000 ] && echo yes || echo "non ($T2578_BYTES octets)")"

# Un seul passage de filtrage : dispatch-lib.sh pèse ~537 Ko, et le compte comme
# la ligne se lisent du même résultat.
T2578_DECL_LINES=$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -E '_PILOT_GH_TOKEN_FILE:?=' || true)
T2578_DECL_COUNT=$(printf '%s' "$T2578_DECL_LINES" | grep -c . || true)
assert_eq "mika#2578: exactement un site de déclaration de _PILOT_GH_TOKEN_FILE" \
    "1" "${T2578_DECL_COUNT:-0}"

T2578_DECL=$(printf '%s\n' "$T2578_DECL_LINES" | head -1)
assert_contains "mika#2578: ce site est la forme conditionnelle (une valeur de harness survit au source)" \
    ': "${_PILOT_GH_TOKEN_FILE:=' "$T2578_DECL"
assert_contains "mika#2578: et son défaut est INCHANGÉ (un dispatch réel stache toujours sous \$HOME)" \
    '$HOME/.mika/pilot-gh-token' "$T2578_DECL"

# ===========================================================================
# mika#2590 — le producteur n'écrit plus le marqueur hors position, et la note
# d'un refus ne le cite plus.
# ===========================================================================
#
# Le lecteur Rust passe à une lecture ANCRÉE sur la ligne. Appliqué seul, cet
# ancrage INTRODUIT un faux négatif (KTD3) : le producteur écrivait
# `sed 's/Outcome: .*/Outcome: PLAN_GROOMED/'`, **non ancré**, donc sur un RESULT
# portant `Outcome: ` en milieu de ligne il posait le marqueur hors position — et
# le filet qui suivait (`grep -qF`, non ancré lui aussi) le voyait et n'ajoutait
# pas la ligne canonique. Un groom RÉELLEMENT convergé aurait alors été refusé.
# Les deux moitiés voyagent dans le même binaire (`skills/bundled/` est une
# projection du binaire, mika#2340), donc elles ne peuvent pas être servies
# séparément — c'est ce qui rend KTD3 sûr.

# --- U8f(a), contrôle négatif PORTEUR : la forme d'avant produit le piège ----
#
# Reproduction explicite du couple sed+grep retiré par U5a, sur le RESULT piège.
# Sans cette moitié, « le bras écrit une ligne ancrée » serait indistinguable de
# « le RESULT d'entrée n'a jamais porté de piège ».
T2590_TRAP_IN="claude-pilot completed (status: success). Outcome: PLAN_COMMITTED — plan posé.

Session: sess-2590"
T2590_OLD_FORM=$(printf '%s' "$T2590_TRAP_IN" | sed 's/Outcome: .*/Outcome: PLAN_GROOMED/')
assert_eq "mika#2590 (contrôle négatif): la forme non ancrée ne produit AUCUNE ligne Outcome ancrée" \
    "0" "$(printf '%s\n' "$T2590_OLD_FORM" | grep -c '^Outcome: PLAN_GROOMED' || true)"
assert_contains "mika#2590 (contrôle négatif): elle pose bien le marqueur, mais en milieu de ligne" \
    "success). Outcome: PLAN_GROOMED" "$T2590_OLD_FORM"

# --- U8f(b), comportemental : _set_outcome_line rend la ligne ancrée ---------
_t2590_groomed_outcome_probe() {
    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true
        RESULT="claude-pilot completed (status: success). Outcome: PLAN_COMMITTED — plan posé.

Session: sess-2590"
        _set_outcome_line "Outcome: PLAN_GROOMED"
        printf '%s' "$RESULT"
    )
}
T2590_NEW_FORM=$(_t2590_groomed_outcome_probe 2>/dev/null)
assert_eq "mika#2590: _set_outcome_line rend EXACTEMENT une ligne Outcome ancrée" \
    "1" "$(printf '%s\n' "$T2590_NEW_FORM" | grep -c '^Outcome: PLAN_GROOMED' || true)"
assert_contains "mika#2590: et le corps du RESULT est préservé" \
    "Session: sess-2590" "$T2590_NEW_FORM"

# --- U8f(c), scan de source : le bras GROOMED passe par _set_outcome_line ----
#
# C'est cette assertion qui rougit avant U5a. Aucun test comportemental ne peut
# atteindre le bras lui-même : il vit dans `dispatch_claude_pilot`, inatteignable
# hors d'un dispatch complet (worktree, pilote, architecte).
T2590_GROOMED_ARM=$(sed -n '/mika#1394: Architect converged on GROOMED/,/^        elif grep -qE/p' "$DISPATCH_LIB")
assert_eq "mika#2590 (bonne foi): le bras GROOMED a été extrait" \
    "yes" "$(if [ -n "$T2590_GROOMED_ARM" ]; then printf 'yes'; else printf 'no'; fi)"
assert_contains "mika#2590 (U5a): le bras GROOMED pose sa ligne par _set_outcome_line" \
    '_set_outcome_line "Outcome: PLAN_GROOMED"' "$T2590_GROOMED_ARM"
assert_not_contains "mika#2590 (U5a): et le sed NON ANCRÉ a disparu (c'est lui, le faux négatif KTD3)" \
    's/Outcome: .*/Outcome: PLAN_GROOMED/' "$T2590_GROOMED_ARM"
assert_contains "mika#2590 (U5a): le strip des PIPELINE FAILURE périmés est CONSERVÉ" \
    "sed '/^PIPELINE FAILURE:/d'" "$T2590_GROOMED_ARM"

# --- U7b (R8/D7) : aucune construction de RESULT auto_skipped ne cite le jeton -
#
# C'est la moitié STRUCTURELLE de R8. La moitié intention (la note reformulée)
# ne tient pas seule au substrat de la boucle
# (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`) : un
# futur éditeur recopie la phrase depuis le commentaire voisin et le défaut
# revient. Le lecteur corrigé le RATTRAPERAIT (l'enveloppe JSON est refusée par
# R2), et c'est exactement pourquoi cette garde vaut le coup — elle empêche la
# régression de se cacher derrière une défense en profondeur.
T2590_AUTOSKIP_LINES=$(grep -n '"status":"auto_skipped"' "$DISPATCH_LIB" || true)
T2590_AUTOSKIP_COUNT=$(printf '%s' "$T2590_AUTOSKIP_LINES" | grep -c . || true)
assert_eq "mika#2590 anti-vacuité: des RESULT auto_skipped existent bien dans dispatch-lib.sh" \
    "yes" "$([ "${T2590_AUTOSKIP_COUNT:-0}" -ge 1 ] && echo yes || echo "non ($T2590_AUTOSKIP_COUNT)")"

T2590_AUTOSKIP_OFFENDERS=$(printf '%s\n' "$T2590_AUTOSKIP_LINES" | grep -c 'Outcome: PLAN_GROOMED' || true)
assert_eq "mika#2590 (U7b): aucune ligne construisant un RESULT auto_skipped ne cite le marqueur" \
    "0" "${T2590_AUTOSKIP_OFFENDERS:-0}"

# Contrôle négatif de l'assertion elle-même : sur une fixture portant le
# littéral, le prédicat DOIT accuser. Sans lui, « l'assertion tient » et
# « l'assertion ne regarde rien » rendent les mêmes octets (classe mika#2205).
T2590_FIXTURE='RESULT=$(printf '"'"'{"status":"auto_skipped","reason":"x","note":"… carrying Outcome: PLAN_GROOMED exists …"}'"'"')'
assert_eq "mika#2590 (contrôle négatif U7b): le prédicat accuse une fixture qui cite le jeton" \
    "1" "$(printf '%s\n' "$T2590_FIXTURE" | grep '"status":"auto_skipped"' | grep -c 'Outcome: PLAN_GROOMED' || true)"

# ============================================================================
# mika#2194 — UN corpus, DEUX lecteurs, UNE parité (R4) + le scan anti-copie (R5)
# ============================================================================
#
# Le lecteur BASH du corpus doré. Son jumeau est
# `crates/mika-agent/tests/plan_callout_parity.rs`, et les deux lisent le MÊME
# fichier d'attendus. Avant mika#2194 chacun des deux lecteurs du callout avait
# un corpus soigné et **aucune entrée n'était commune** : personne n'avait
# jamais exécuté les deux sur la même entrée.
#
# Cette passe-ci est la NON-RÉGRESSION, pas la preuve. La preuve de parité
# (« mêmes entrées → mêmes sorties que le bash ») se mesure contre le bash
# d'AVANT la bascule, et elle a été faite une fois, à l'implémentation : après
# R3 le bash appelle le Rust, donc un lecteur comparé à lui-même est toujours
# d'accord avec lui-même. D'où la colonne `phase` du TSV, et d'où le fait que
# ce bloc n'exerce que les lignes `both`.

echo ""
echo "Test: mika#2194 — parité du callout Plan sur le corpus commun"
echo "-------------------------------------------------------------"

M2194_CORPUS="$SCRIPT_DIR/../../../crates/mika-agent/tests/fixtures/plan_callout_bodies"
M2194_TSV="$M2194_CORPUS/expectations.tsv"

# ── Anti-vacuité, en trois termes. C'est le mode de panne que R4 nomme : une
#    parité verte sur zéro cas. Le motif dominant du harnais est
#    `source "$DISPATCH_LIB" 2>/dev/null || true`, donc un dispatch-lib qui
#    cesse de sourcer laisserait les assertions vertes sur des fonctions
#    inexistantes — éliminatoire pour un test de parité.
assert_eq "mika#2194 (anti-vacuité 1/3): le corpus d'attendus existe" "yes" \
    "$(if [ -f "$M2194_TSV" ]; then printf 'yes'; else printf 'no'; fi)"

M2194_DECLARED=0
if [ -f "$M2194_TSV" ]; then
    M2194_DECLARED=$(grep -cv '^[[:space:]]*\(#\|$\)' "$M2194_TSV" || true)
fi
assert_eq "mika#2194 (anti-vacuité 2/3): le corpus n'est pas vide" "yes" \
    "$(if [ "$M2194_DECLARED" -gt 0 ]; then printf 'yes'; else printf 'no'; fi)"

assert_eq "mika#2194 (anti-vacuité 3/3): _extract_plan_path est définie après le sourcing" "yes" \
    "$(if declare -f _extract_plan_path >/dev/null; then printf 'yes'; else printf 'no'; fi)"

# ── Le binaire est résolu EN TÊTE DE FICHIER depuis mika#2608 (R4) : les douze
#    cas fonctionnels de la porte de grooming en dépendent aussi et tournent
#    bien avant ce bloc, donc une résolution locale ici les laissait sans.
if [ -z "$MIKA_BIN" ] || [ "$M2194_DECLARED" -eq 0 ]; then
    mika_bin_or_skip "mika#2194/#2608 parité du callout" || true
else
    M2194_CASES=0
    M2194_BAD=0
    while IFS=$'\t' read -r m_file m_rc m_raw m_norm m_parity m_phase; do
        case "$m_file" in ''|'#'*) continue ;; esac
        [ -n "${m_rc:-}" ] || continue
        # Seules les lignes `both` sont exerçables après la bascule. Une ligne
        # `pre-switch` décrit le bash d'AVANT, qui n'existe plus ici.
        [ "$m_phase" = "both" ] || continue

        m_body_file="$M2194_CORPUS/$m_file"
        if [ ! -f "$m_body_file" ]; then
            echo "    ✗ $m_file — corps introuvable"
            M2194_BAD=$((M2194_BAD + 1)); continue
        fi
        m_body=$(cat "$m_body_file")
        M2194_CASES=$((M2194_CASES + 1))

        m_got_rc=0
        m_got=$(_extract_plan_path "$m_body") || m_got_rc=$?

        if [ "$m_rc" = "0" ]; then
            # Le bash veut le chemin NORMALISÉ : sa question est quel fichier
            # ouvrir, et il le résout contre "$WORKTREE_DIR".
            if [ "$m_got_rc" != "0" ] || [ "$m_got" != "$m_norm" ]; then
                echo "    ✗ $m_file — attendu rc=0 '$m_norm', lu rc=$m_got_rc '$m_got'"
                M2194_BAD=$((M2194_BAD + 1))
            fi
        else
            if [ "$m_got_rc" = "0" ] || [ -n "$m_got" ]; then
                echo "    ✗ $m_file — attendu un refus, lu rc=$m_got_rc '$m_got'"
                M2194_BAD=$((M2194_BAD + 1))
            fi
        fi
    done < "$M2194_TSV"

    assert_eq "mika#2194: parité bash↔Rust sur le corpus commun (aucun écart)" \
        "0" "$M2194_BAD"
    # Le compte de cas EST une assertion, pas un affichage : c'est le seul
    # moyen qu'a ce lecteur de dire qu'il a regardé quelque chose.
    assert_eq "mika#2194: le lecteur bash nomme son compte de cas (non nul)" "yes" \
        "$(if [ "$M2194_CASES" -gt 0 ]; then printf 'yes'; else printf 'no'; fi)"
    echo "    → mika#2194 parité bash : $M2194_CASES cas exercés (phase=both)"
fi

# ============================================================================
# mika#2608 — PARITÉ PAR SITE (AC3) et ses contrôles négatifs (AC4)
# ============================================================================
#
# Le bloc mika#2194 ci-dessus exerce `_extract_plan_path`, c'est-à-dire le canal.
# Il ne dit RIEN des deux sites que la phase 2 bascule : un site qui cesserait
# d'appeler le canal — ou qui rétablirait son ancienne expression — le laisserait
# entièrement vert. C'est l'exigence propre à l'AC3, et c'est ce que ces deux
# passages mesurent.
#
# Les deux sites posent des questions différentes, donc chacun a sa forme :
#
#   SITE 2 (`_set_up_worktree`, le `elif`) — question BOOLÉENNE. Le site est un
#   `elif` au milieu d'une fonction de ~400 lignes qu'on ne peut pas appeler
#   isolément (elle `fetch`, `worktree add`, `gh issue view`…), donc ce qui est
#   exercé est l'EXPRESSION que le site porte, extraite de sa source. Le
#   contrôle négatif est ce qui rend ça honnête : il réintroduit le `grep -qE`
#   d'avant la bascule et EXIGE que la parité rougisse.
#
#   SITE 1 (`_committed_plan_on_branch`) — question de CHEMIN. Celui-là est
#   appelable, et il est exercé pour de vrai : contre un clone fixture où le plan
#   du corps est réellement committé sur la branche.
#
# Chaque passage nomme son compte de cas et ÉCHOUE sur zéro (anti-vacuité) : une
# parité verte sur zéro cas est le mode de panne que R4 nomme, et c'est le seul
# que ni l'un ni l'autre des lecteurs ne verrait tout seul.

echo ""
echo "Test: mika#2608 — parité PAR SITE des deux lecteurs bascules"
echo "------------------------------------------------------------"

if [ -z "$MIKA_BIN" ] || [ "$M2194_DECLARED" -eq 0 ]; then
    mika_bin_or_skip "mika#2608 parité par site" || true
else
    # ── SITE 2 — le prédicat booléen du `elif`, lu DANS la source du site.
    #
    # L'expression est extraite plutôt que recopiée : une copie littérale ici
    # resterait verte le jour où le site changerait, ce qui est exactement la
    # classe que ce ticket ferme. `M2608_SITE2_EXPR` est donc la ligne réelle.
    M2608_WT_SRC=$(sed -n '/^_set_up_worktree()/,/^}/p' "$DISPATCH_LIB")
    M2608_SITE2_EXPR=$(printf '%s\n' "$M2608_WT_SRC" \
        | grep -E '^[[:space:]]*elif .*_extract_plan_path' | head -1)

    assert_eq "mika#2608 (site 2, anti-vacuité): le \`elif\` délègue au canal" "yes" \
        "$(if [ -n "$M2608_SITE2_EXPR" ]; then printf 'yes'; else printf 'no'; fi)"

    # Le prédicat du site, isolé dans une fonction qui reproduit sa forme exacte
    # — `>/dev/null 2>&1`, donc seul le code de sortie décide.
    m2608_site2_predicate() { _extract_plan_path "$1" >/dev/null 2>&1; }

    M2608_S2_CASES=0
    M2608_S2_BAD=0
    while IFS=$'\t' read -r m_file m_rc m_raw m_norm m_parity m_phase; do
        case "$m_file" in ''|'#'*) continue ;; esac
        [ -n "${m_rc:-}" ] || continue
        [ "$m_phase" = "both" ] || continue
        m_body_file="$M2194_CORPUS/$m_file"
        [ -f "$m_body_file" ] || continue
        m_body=$(cat "$m_body_file")
        M2608_S2_CASES=$((M2608_S2_CASES + 1))

        # La colonne `rc` du TSV EST la réponse booléenne attendue : le site
        # écrit sa ligne de journal si et seulement si un callout a été lu.
        m2608_got=0
        m2608_site2_predicate "$m_body" || m2608_got=1
        m2608_want=$([ "$m_rc" = "0" ] && printf 0 || printf 1)
        if [ "$m2608_got" != "$m2608_want" ]; then
            echo "    ✗ site 2 / $m_file — attendu booléen=$m2608_want, lu $m2608_got"
            M2608_S2_BAD=$((M2608_S2_BAD + 1))
        fi
    done < "$M2194_TSV"

    assert_eq "mika#2608 (site 2): le booléen du \`elif\` == la colonne rc du TSV" \
        "0" "$M2608_S2_BAD"
    assert_eq "mika#2608 (site 2): le passage nomme son compte de cas (non nul)" "yes" \
        "$(if [ "$M2608_S2_CASES" -gt 0 ]; then printf 'yes'; else printf 'no'; fi)"
    echo "    → mika#2608 site 2 : $M2608_S2_CASES cas exercés"

    # ── CONTRÔLE NÉGATIF DU SITE 2 (AC4) — la mutation DOIT rougir.
    #
    # Le `grep -qE` d'avant la bascule, réintroduit. Il accepte
    # `gate-non-plan-path.md` (il n'exige pas `docs/plans/`) et
    # `gate-double-space.md` (son motif s'arrête avant l'espace), là où le canal
    # les refuse — donc la parité rougit sur au moins ces deux cas. Sans ce
    # contrôle, « la parité mesure le site » serait indistinguable de « la parité
    # rougit sur tout » ET de « la parité ne regarde rien » (classe mika#2205).
    m2608_site2_mutant() { grep -qE -- '^> - \*\*Plan:\*\*' <<<"$1"; }
    M2608_S2_MUT_BAD=0
    while IFS=$'\t' read -r m_file m_rc m_raw m_norm m_parity m_phase; do
        case "$m_file" in ''|'#'*) continue ;; esac
        [ -n "${m_rc:-}" ] || continue
        [ "$m_phase" = "both" ] || continue
        m_body_file="$M2194_CORPUS/$m_file"
        [ -f "$m_body_file" ] || continue
        m_body=$(cat "$m_body_file")
        m2608_got=0
        m2608_site2_mutant "$m_body" || m2608_got=1
        m2608_want=$([ "$m_rc" = "0" ] && printf 0 || printf 1)
        [ "$m2608_got" = "$m2608_want" ] || M2608_S2_MUT_BAD=$((M2608_S2_MUT_BAD + 1))
    done < "$M2194_TSV"

    assert_eq "mika#2608 (site 2, contrôle négatif): la mutation du lecteur fait rougir la parité" \
        "yes" "$(if [ "$M2608_S2_MUT_BAD" -gt 0 ]; then printf 'yes'; else printf "no ($M2608_S2_MUT_BAD écarts)"; fi)"
    echo "    → mika#2608 site 2 mutant : $M2608_S2_MUT_BAD écarts (doit être > 0)"

    # ── SITE 1 — `_committed_plan_on_branch`, exercé POUR DE VRAI contre un
    #    clone fixture où les plans du corpus sont réellement committés.
    #
    #    Un seul clone pour tous les cas : `_fixture_setup` clone un dépôt git,
    #    donc un clone par corps coûterait une vingtaine de clones pour aucune
    #    isolation utile — le site ne mute rien dans l'arbre.
    #
    #    `ISSUE_NUM=''` désactive la liaison mika#2034 : elle a ses douze cas
    #    fonctionnels à elle, et ce qui est mesuré ici est le chemin que le site
    #    rend, pas la réfutation par en-tête.
    # Rend `"<écarts> <cas>"` sur STDOUT et ses messages d'écart sur STDERR.
    #
    # Les deux moitiés de cette signature sont des pièges mesurés à
    # l'implémentation, et ce sont les mêmes deux que `_plan_provenance` documente
    # en production dans dispatch-lib : un appel en substitution de commande
    # tourne dans un SOUS-SHELL, donc (a) une assignation de variable globale
    # faite ici ne remonte PAS chez l'appelant — le compte de cas lisait 0, et
    # l'anti-vacuité rougissait sur un passage qui avait pourtant tout exercé —
    # et (b) tout ce qui va sur stdout est capturé, donc un message d'écart y
    # atterrissait DANS le compte, que `[` refusait ensuite comme non entier.
    # D'où : les deux nombres voyagent par la valeur de retour, et les messages
    # par stderr.
    m2608_site1_corpus_parity() { # $1 = étiquette (pour le message d'écart)
        local label="$1" bad=0 cases=0
        local m_file m_rc m_raw m_norm m_parity m_phase m_body out rc

        while IFS=$'\t' read -r m_file m_rc m_raw m_norm m_parity m_phase; do
            case "$m_file" in ''|'#'*) continue ;; esac
            [ -n "${m_rc:-}" ] || continue
            [ "$m_phase" = "both" ] || continue
            [ -f "$M2194_CORPUS/$m_file" ] || continue
            m_body=$(cat "$M2194_CORPUS/$m_file")
            cases=$((cases + 1))

            rc=0
            out=$(ISSUE_NUM='' _committed_plan_on_branch \
                "$FIXTURE_CLONE" "fix/2608/site1" "$m_body" "mika" 2>/dev/null) || rc=$?

            if [ "$m_rc" = "0" ]; then
                if [ "$rc" -ne 0 ] || [ "$out" != "$m_norm" ]; then
                    echo "    ✗ $label / $m_file — attendu rc=0 '$m_norm', lu rc=$rc '$out'" >&2
                    bad=$((bad + 1))
                fi
            else
                if [ "$rc" -eq 0 ]; then
                    echo "    ✗ $label / $m_file — attendu un refus, lu rc=0 '$out'" >&2
                    bad=$((bad + 1))
                fi
            fi
        done < "$M2194_TSV"

        # Le RETOUR À LA LIGNE est load-bearing : ce fichier tourne sous
        # `set -euo pipefail`, et `read` rend 1 sur une ligne non terminée — donc
        # un `printf` sans `\n` tuait le script entier au milieu de ce bloc, sans
        # résumé et sans aucune assertion en échec. Mesuré à l'implémentation.
        printf '%s %s\n' "$bad" "$cases"
    }

    _fixture_setup
    if ! _assert_fixture_is_local; then
        SKIPPED=$((SKIPPED + 1))
        echo "  ⊘ SKIP mika#2608 parité site 1 — fixture non locale."
    else
        git -C "$FIXTURE_CLONE" checkout -q -b fix/2608/site1
        mkdir -p "$FIXTURE_CLONE/docs/plans"

        # Les plans que le corpus nomme, aux chemins NORMALISÉS, plus le
        # `README.md` qui EST le faux positif latent de M3 : sans lui, le
        # contrôle négatif du site 1 ne pourrait pas diverger — l'ancien `sed`
        # extrairait `README.md` et `cat-file -t` ne rendrait pas `blob`, donc le
        # mutant refuserait comme le site, et la mutation se lirait comme
        # inoffensive alors qu'en production tout dépôt a un README.
        echo "# readme, pas un plan" > "$FIXTURE_CLONE/README.md"
        while IFS=$'\t' read -r m_file m_rc m_raw m_norm m_parity m_phase; do
            case "$m_file" in ''|'#'*) continue ;; esac
            [ "$m_rc" = "0" ] || continue
            [ "$m_phase" = "both" ] || continue
            mkdir -p "$FIXTURE_CLONE/$(dirname "$m_norm")"
            printf '# plan de corpus (aucun en-tête d'"'"'issue)\n' > "$FIXTURE_CLONE/$m_norm"
        done < "$M2194_TSV"
        # Et les deux chemins que SEUL l'ancien `sed` extrait : ils doivent
        # résoudre pour que la mutation soit visible plutôt que masquée par un
        # `cat-file` négatif.
        printf '# plan de corpus\n' > "$FIXTURE_CLONE/docs/plans/2026-09-30-001-fix-2608-deux-espaces-plan.md"
        git -C "$FIXTURE_CLONE" add -A >/dev/null 2>&1
        git -C "$FIXTURE_CLONE" commit -q -m "corpus plans for the site-1 parity pass"
        git -C "$FIXTURE_CLONE" push -q origin fix/2608/site1

        # Les écarts du passage NOMINAL doivent être visibles (stderr non
        # masqué) : ce sont eux qu'un mainteneur lira si la parité casse.
        read -r M2608_S1_BAD M2608_S1_CASES < <(m2608_site1_corpus_parity "site 1")

        assert_eq "mika#2608 (site 1): le chemin rendu == la colonne \`normalized\` du TSV" \
            "0" "$M2608_S1_BAD"
        assert_eq "mika#2608 (site 1): le passage nomme son compte de cas (non nul)" "yes" \
            "$(if [ "$M2608_S1_CASES" -gt 0 ]; then printf 'yes'; else printf 'no'; fi)"
        echo "    → mika#2608 site 1 : $M2608_S1_CASES cas exercés"

        # ── CONTRÔLE NÉGATIF DU SITE 1 (AC4) — la mutation DOIT rougir.
        #
        # `_extract_plan_path` est remplacée par le `sed` d'avant la bascule,
        # puis le VRAI site est rappelé : la mutation traverse le site réel, donc
        # ce qui est attesté est bien que la parité mesure CE SITE et pas
        # seulement le canal. Restauration par re-sourcing plutôt que par `eval`.
        #
        # Une mutation à la fois (leçon mika#2277) : seul l'extracteur change,
        # `cat-file`, la liaison et la normalisation restent ceux de production.
        _extract_plan_path() {
            local p
            p=$(printf '%s\n' "$1" \
                | sed -n 's/^> - \*\*Plan:\*\* *`\([^`]*\)`.*/\1/p' | head -1)
            [ -n "$p" ] || return 1
            printf '%s\n' "$p"
        }
        # Ici stderr EST masqué : les écarts sont ATTENDUS, et les afficher ferait
        # lire onze lignes `✗` sur une passe dont l'échec serait le vert.
        read -r M2608_S1_MUT_BAD _ \
            < <(m2608_site1_corpus_parity "site 1 MUTANT (attendu: des écarts)" 2>/dev/null)
        # shellcheck source=dispatch-lib.sh
        source "$DISPATCH_LIB" 2>/dev/null || true

        assert_eq "mika#2608 (site 1, contrôle négatif): la mutation du lecteur fait rougir la parité" \
            "yes" "$(if [ "$M2608_S1_MUT_BAD" -gt 0 ]; then printf 'yes'; else printf "no ($M2608_S1_MUT_BAD écarts)"; fi)"
        echo "    → mika#2608 site 1 mutant : $M2608_S1_MUT_BAD écarts (doit être > 0)"

        # La restauration a bien eu lieu : sans cette assertion, un re-sourcing
        # raté laisserait le mutant en place pour la suite du fichier — et le
        # motif dominant du harnais est `source … || true`, donc l'échec serait
        # muet (limite que la phase 1 nomme déjà).
        assert_contains "mika#2608: le canal est restauré après le contrôle négatif" \
            "mika plan-callout --body-file" "$(declare -f _extract_plan_path)"

        _fixture_cleanup
    fi
fi

# ── R5 — le scan anti-copie, étendu de UNE fonction à TOUT dispatch-lib.
#
# Le scan de mika#2120 couvrait `_detect_plan_on_branch` seule. Après la
# bascule, plus aucun site de dispatch-lib ne doit **lire** le callout : le
# lecteur est en Rust, et un second motif écrit ici rejouerait exactement la
# divergence que ce ticket ferme.
#
# ── PREMIER PIÈGE, déjà mesuré deux fois dans ce dépôt (mika#2050 sur le
#    Signal S, mika#2201 § R4) : ce fichier CITE le motif dans ses commentaires
#    de doctrine. Le prédicat porte donc sur les lignes de COMMANDE,
#    commentaires retirés d'abord — sans quoi il serait rouge à la naissance,
#    donc désarmé.
#
# ── SECOND PIÈGE, trouvé en exécutant ce scan : dispatch-lib **écrit** aussi le
#    callout (`_write_canonical_callout`, qui compose le corps d'issue que le
#    grooming publie). L'écrivain n'est pas un lecteur, et l'accuser rendrait ce
#    scan rouge sur du code sain — donc désarmé, donc inutile.
#
#    Ce n'est PAS une allowlist d'exemption : c'est la définition de la
#    population. Le TSV de mika#2201 fait déjà exactement cette distinction en
#    ne déclarant que des *sites de match*. La doctrine « on route, on
#    n'allowliste pas » porte sur les lecteurs ; un écrivain n'a rien à router.
#
#    Le discriminant est donc la présence d'un OUTIL DE LECTURE sur la même
#    ligne. Limite nommée : un futur lecteur écrit par expansion de paramètre
#    (`${body##*Plan:}`) passerait sous ce prédicat. Le test Rust
#    `mika2194_aucun_motif_de_callout_hors_de_ce_module` couvre l'autre moitié
#    de l'arbre, et c'est ensemble qu'ils bornent la classe.
M2194_CMD_LINES=$(grep -v '^[[:space:]]*#' "$DISPATCH_LIB" || true)
M2194_EXTRACT_SRC_EARLY=$(sed -n '/^_extract_plan_path()/,/^}/p' "$DISPATCH_LIB")

# L'aiguille couvre les DEUX écritures du motif — le littéral `**Plan:**` et sa
# forme échappée pour une regex `\*\*Plan:\*\*`. La première version de ce scan
# ne portait que la première, donc elle n'aurait PAS vu revenir le `grep -oP`
# qu'elle existe pour refuser : son propre contrôle négatif l'a montré.
M2194_NEEDLE='Plan:(\\)?\*(\\)?\*'
M2194_READER='grep|sed|awk|=~|case[[:space:]]|expr[[:space:]]'

m2194_read_sites() { # lit sur stdin, rend le compte des lignes de LECTURE portant le motif
    grep -v '^[[:space:]]*#' \
        | grep -E -- "$M2194_NEEDLE" \
        | grep -Ec -- "$M2194_READER" || true
}

# ── L'assertion est désormais un ZÉRO, et ce zéro est DEVENU atteignable.
#
#    La phase 1 comptait deux parce qu'elle gardait délibérément deux lecteurs,
#    et son commentaire disait pourquoi un zéro aurait été « rouge à la
#    naissance, donc désarmé ». La phase 2 (mika#2608) retire ces deux lecteurs :
#    `_committed_plan_on_branch` et le `elif` de `_set_up_worktree` délèguent
#    tous deux à `_extract_plan_path`. L'inventaire fermé devient donc le zéro
#    que R5 demandait — ce n'est pas un durcissement gratuit, c'est la forme que
#    le scan aurait prise si la phase 1 avait pu la prendre.
#
#    Le zéro rend les TROIS contrôles de bonne foi ci-dessous PLUS nécessaires,
#    pas moins : ce sont eux qui distinguent « zéro lecteur » de « le prédicat ne
#    regarde rien » (classe mika#2205), et sur un zéro il n'y a plus de
#    population positive pour le faire à leur place.
M2194_READ_COUNT=$(printf '%s\n' "$M2194_CMD_LINES" | m2194_read_sites)
assert_eq "mika#2608 (R7): ZÉRO lecteur bash du callout — les deux de phase 2 ont migré" \
    "0" "$M2194_READ_COUNT"

# Et ce sont bien ces deux-là qui ont migré : un zéro global ne dit pas lesquels,
# et une fonction qui aurait cessé d'exister satisferait le zéro sans avoir
# migré. Les deux assertions de la phase 1 s'INVERSENT plutôt que de disparaître.
M2194_COMMITTED_SRC=$(sed -n '/^_committed_plan_on_branch()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2608 (R7): _committed_plan_on_branch ne lit plus le motif" "0" \
    "$(printf '%s\n' "$M2194_COMMITTED_SRC" | m2194_read_sites)"
assert_contains "mika#2608 (R7): …et elle délègue au canal" \
    "_extract_plan_path" "$M2194_COMMITTED_SRC"
M2194_WORKTREE_SRC=$(sed -n '/^_set_up_worktree()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2608 (R7): _set_up_worktree ne lit plus le motif" "0" \
    "$(printf '%s\n' "$M2194_WORKTREE_SRC" | m2194_read_sites)"
assert_contains "mika#2608 (R7): …et son \`elif\` délègue au canal" \
    "_extract_plan_path" "$M2194_WORKTREE_SRC"

# Et le site que ce ticket bascule n'en est PLUS un : c'est le livrable de R3.
assert_eq "mika#2194 (R5): _extract_plan_path ne lit plus le motif" "0" \
    "$(printf '%s\n' "$M2194_EXTRACT_SRC_EARLY" | m2194_read_sites)"

# Contrôle de bonne foi n°1 : sur une fixture portant le motif ÉCHAPPÉ en
# position de lecture, le prédicat DOIT accuser. Sans lui, « aucun site » est
# indistinguable de « le prédicat ne regarde rien » (classe mika#2205) — et ce
# scan est précisément celui que R5 élargit, donc son inertie coûterait le
# livrable.
M2194_FX_PCRE='    path=$(printf "%s\n" "$body" | grep -oP "^> - \*\*Plan:\*\* `\K[^`]+")'
assert_eq "mika#2194 (R5, contrôle négatif a): le prédicat accuse un PCRE échappé" "1" \
    "$(printf '%s\n' "$M2194_FX_PCRE" | m2194_read_sites)"

# Contrôle de bonne foi n°2 : la forme NON échappée aussi (un `case` sur le
# littéral, la façon dont un futur lecteur bash le plus simple s'écrirait).
M2194_FX_LITERAL='    case "$body" in *"> - **Plan:**"*) printf yes ;; esac'
assert_eq "mika#2194 (R5, contrôle négatif b): le prédicat accuse un littéral lu" "1" \
    "$(printf '%s\n' "$M2194_FX_LITERAL" | m2194_read_sites)"

# Contrôle de bonne foi n°3 — celui qui borne la population : l'ÉCRIVAIN du
# callout porte le motif et ne doit PAS être accusé. Sans ce contrôle, la
# réparation tentante serait d'élargir le prédicat jusqu'à ce qu'il n'attrape
# plus rien.
M2194_FX_WRITER='> - **Plan:** \`${plan_relpath}\` (committed on branch @ \`${head_sha}\`)'
assert_eq "mika#2194 (R5, contrôle négatif c): l'écrivain du callout est hors population" "0" \
    "$(printf '%s\n' "$M2194_FX_WRITER" | m2194_read_sites)"

# Et l'anti-vacuité de la population elle-même : le motif DOIT encore apparaître
# dans dispatch-lib. Le seuil passe de 3 à 1 avec la phase 2, et cette
# population est désormais L'ÉCRIVAIN SEUL — `_write_canonical_callout`, qui
# compose le corps d'issue que le grooming publie. Elle est NOMMÉE parce qu'un
# jeton entièrement disparu (renommage du callout, écrivain retiré) rendrait
# zéro offender, donc un vert sur rien : le zéro de R7 ne vaut que si le motif
# existe encore quelque part.
M2194_NEEDLE_COUNT=$(printf '%s\n' "$M2194_CMD_LINES" | grep -cE -- "$M2194_NEEDLE" || true)
assert_eq "mika#2608 (R7, anti-vacuité a): le motif a une population non vide (l'écrivain)" "yes" \
    "$(if [ "$M2194_NEEDLE_COUNT" -ge 1 ]; then printf 'yes'; else printf "no ($M2194_NEEDLE_COUNT)"; fi)"
# Et c'est bien l'écrivain, nommé : un compte non nul porté par un lecteur qui
# serait revenu satisferait l'anti-vacuité tout en cassant R7, et les deux
# assertions se contrediraient sans que rien ne dise laquelle croire.
M2194_WRITER_SRC=$(sed -n '/^_write_canonical_callout()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2608 (R7, anti-vacuité b): la population est l'écrivain _write_canonical_callout" "yes" \
    "$(if printf '%s\n' "$M2194_WRITER_SRC" | grep -v '^[[:space:]]*#' | grep -qE -- "$M2194_NEEDLE"; then printf 'yes'; else printf 'no'; fi)"

# Et le contrôle positif que le scan a bien vu le fichier : un dispatch-lib
# introuvable rendrait zéro ligne de commande, donc zéro offender, donc un vert
# sur rien.
assert_eq "mika#2194 (R5, anti-vacuité c): le scan a lu des lignes de commande" "yes" \
    "$(if [ "$(printf '%s\n' "$M2194_CMD_LINES" | wc -l)" -gt 100 ]; then printf 'yes'; else printf 'no'; fi)"

# Un seul lecteur : `_extract_plan_path` délègue, et `_detect_plan_on_branch`
# ne reporte pas sa propre extraction (l'assertion de mika#2120, conservée).
M2194_EXTRACT_SRC=$(sed -n '/^_extract_plan_path()/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2194: _extract_plan_path a bien été trouvée (guards the guard)" "yes" \
    "$(if [ -n "$M2194_EXTRACT_SRC" ]; then printf 'yes'; else printf 'no'; fi)"
assert_contains "mika#2194: _extract_plan_path délègue à la sous-commande" \
    "mika plan-callout --body-file" "$M2194_EXTRACT_SRC"
assert_not_contains "mika#2194: _extract_plan_path ne porte plus de PCRE" \
    "grep -oP" "$M2194_EXTRACT_SRC"
# Le corps passe par un FICHIER : un argument ré-introduirait la classe de
# panne de portée de guillemets dans le geste qui prétend la fermer.
assert_contains "mika#2194: le corps passe par un fichier temporaire" \
    "mktemp" "$M2194_EXTRACT_SRC"

# Les trois codes de sortie sont traités, et `≥2` ne devient pas un PLAN_PATH
# vide — c'est le livrable d'AC4.
M2194_DETECT_SRC=$(sed -n '/^_detect_plan_on_branch()/,/^}/p' "$DISPATCH_LIB")
assert_contains "mika#2194 (AC4): _detect_plan_on_branch distingue le code ≥2" \
    '_plan_rc" -ge 2' "$M2194_DETECT_SRC"
assert_contains "mika#2194 (AC4): le refus est nommé plutôt que silencieux" \
    "REFUSED (plan-callout, mika#2194)" "$M2194_DETECT_SRC"
# Le motif voyage jusqu'au RESULT : le stderr d'avant-pilote est perdu sur un
# dispatch qui réussit (classe mika#2050), et ce refus laisse le dispatch
# réussir. Une surface que personne ne lit reproduirait le défaut du Signal M.
assert_contains "mika#2194: le refus atteint le RESULT du callback" \
    "_PLAN_CALLOUT_REFUSAL" "$M2194_DETECT_SRC"
M2194_DELIVER_SRC=$(sed -n '/^_deliver_callback()/,/^}/p' "$DISPATCH_LIB")
assert_contains "mika#2194: _deliver_callback annexe le refus" \
    "_annex_plan_callout_refusal" "$M2194_DELIVER_SRC"

# ============================================================================
# mika#2608 R6 — l'inventaire du jeton dans `canonical-tokens.tsv`, fermé à UNE
# ============================================================================
#
# Pourquoi cette assertion existe, et pourquoi aucune garde mika#2201 ne la
# remplace : `canonical-tokens-survey.sh --check` ne vérifie qu'UNE direction —
# tout site strict de l'arbre est déclaré — et son propre commentaire nomme ce
# qu'il laisse passer, mot pour mot : « a row whose file and symbol both still
# exist but which no longer reads the token ». Les deux fonctions bascules
# EXISTENT encore, donc `mika2201_every_declared_symbol_still_exists` serait
# resté vert sur deux lignes devenues fausses. La lecture naïve de l'AC5 — « les
# deux gardes sont vertes » — n'était donc pas une preuve.
#
# La direction générale « toute ligne déclarée matche encore » est REFUSÉE avec
# sa mesure par le survey lui-même : le TSV porte légitimement des lignes qu'il
# ne peut pas produire (une alternance `(?i)`, un palier flou, un lecteur par
# `contains`), et la comparer ainsi ferait rougir le build sur des lignes
# CORRECTES — 14 fausses contre 2 vraies, mesuré. D'où une assertion bornée AU
# JETON plutôt qu'une comparaison à deux sens.
M2608_TOKENS_TSV="$SCRIPT_DIR/../../../scripts/canonical-tokens.tsv"

assert_eq "mika#2608 (R6, anti-vacuité): le TSV des jetons canoniques existe" "yes" \
    "$(if [ -f "$M2608_TOKENS_TSV" ]; then printf 'yes'; else printf 'no'; fi)"

if [ -f "$M2608_TOKENS_TSV" ]; then
    # Les lignes de DÉCLARATION du jeton, commentaires exclus : le fichier cite
    # le jeton dans sa prose de doctrine (y compris pour dire ce qui a migré),
    # donc un compte sur le fichier entier serait faux dans les deux sens.
    M2608_PLAN_ROWS=$(grep -v '^[[:space:]]*#' "$M2608_TOKENS_TSV" \
        | grep -cF -- '> - **Plan:**	' || true)
    assert_eq "mika#2608 (R6): EXACTEMENT une ligne déclarée pour le jeton \`> - **Plan:**\`" \
        "1" "$M2608_PLAN_ROWS"

    # Et c'est bien celle-là : un compte de un porté par une autre ligne
    # satisferait l'inventaire tout en déclarant le mauvais site.
    M2608_PLAN_SITE=$(grep -v '^[[:space:]]*#' "$M2608_TOKENS_TSV" \
        | grep -F -- '> - **Plan:**	' | cut -f3)
    assert_eq "mika#2608 (R6): et c'est le lecteur unique, plan_callout.rs::PLAN_CALLOUT_RE" \
        "crates/mika-agent/src/plan_callout.rs::PLAN_CALLOUT_RE" "$M2608_PLAN_SITE"

    # Les deux sites bascules ne doivent plus être déclarés pour CE jeton. Un
    # zéro global ne le dirait pas : ils restent légitimement déclarés pour
    # d'autres jetons, donc le test porte sur la conjonction (jeton, site).
    M2608_STALE_ROWS=$(grep -v '^[[:space:]]*#' "$M2608_TOKENS_TSV" \
        | grep -F -- '> - **Plan:**	' \
        | grep -cE 'dispatch-lib\.sh::(_committed_plan_on_branch|_set_up_worktree)' || true)
    assert_eq "mika#2608 (R6): aucune ligne du jeton ne déclare plus un site bascule" \
        "0" "$M2608_STALE_ROWS"

    # Contrôle de bonne foi : le prédicat DOIT accuser une ligne remise. Sans
    # lui, « aucun site bascule » est indistinguable de « le prédicat ne regarde
    # rien » — et la tabulation de séparation est exactement le genre de détail
    # dont une faute rendrait ce scan muet (classe mika#2205).
    M2608_FX_STALE_ROW='> - **Plan:**	B	skills/bundled/_shared/dispatch-lib.sh::_set_up_worktree	exact:literal'
    assert_eq "mika#2608 (R6, contrôle négatif): le prédicat accuse une ligne bascule remise" "1" \
        "$(printf '%s\n' "$M2608_FX_STALE_ROW" \
            | grep -F -- '> - **Plan:**	' \
            | grep -cE 'dispatch-lib\.sh::(_committed_plan_on_branch|_set_up_worktree)' || true)"
fi

# ============================================================================
# Test mika#2626 — `_push_branch` cesse de déduire l'existence DISTANTE d'une
# ref de suivi LOCALE
# ============================================================================
#
# Incident fondateur : 2026-10-01, pilote `624656b1` (mika#1960 phase 2). Le
# rescue a commité le travail du pilote (90,40 USD, ~770 lignes), l'a rebasé
# sur main, puis :
#
#   Push: FAILED — remote advanced since fetch (lease aborted); commits remain
#   local-only on test/1960/eval-propose-surface-doctrine-regression
#
# Or la branche n'existait PAS sur origin : celle de la phase 1 avait été
# supprimée au merge de PR #2620, et personne n'avait « avancé » le distant. Le
# checkout partagé gardait `origin/test/1960/…` à l'ancienne tête, jamais
# élaguée — `fetch origin "$BRANCH"` ne la touche pas, et `--prune` n'y change
# rien (mesuré : git échoue d'abord sur `couldn't find remote ref`, donc le
# prune n'est jamais atteint).
#
# Le fixture reproduit cet état exact. La branche est supprimée **dans le
# bare**, par `update-ref -d` — JAMAIS par `git push origin --delete` depuis le
# clone : git y supprimerait aussi la ref de suivi locale, c'est-à-dire
# précisément le fixture qu'on veut construire. C'est aussi la forme la plus
# fidèle à l'incident : GitHub supprime la branche, le checkout partagé ne le
# sait pas.
echo ""
echo "Test mika#2626: une ref origin/\$BRANCH orpheline ne produit plus 'diverged'"
echo "----------------------------------------------------------------------------"

# Helper de fixture : amène le clone dans l'état de l'incident et rend 0, ou
# écrit la raison sur stdout et rend 1. Paramètre $1 = nom de branche,
# $2 = "rewrite" pour réécrire l'historique (le rescue a rebasé), "keep" pour
# le laisser intact (le cas ahead==0 de V6).
_t2626_make_orphaned_ref() {
    local br="$1" history="$2"

    # 1. La phase 1 : une branche poussée. `origin/$br` existe des deux côtés.
    git -C "$FIXTURE_CLONE" checkout -q -b "$br"
    echo "phase 1" > "$FIXTURE_CLONE/phase1.txt"
    git -C "$FIXTURE_CLONE" add phase1.txt
    git -C "$FIXTURE_CLONE" commit -q -m "phase 1"
    git -C "$FIXTURE_CLONE" push -q -u origin "$br"

    # 2. Le merge de la PR de la phase 1 : la forge supprime la branche. Le
    #    checkout partagé ne le sait pas, sa ref de suivi reste.
    git -C "$FIXTURE_BARE" update-ref -d "refs/heads/$br"

    if [ "$history" = "rewrite" ]; then
        # 3. main avance, puis le rescue rebase la branche : HEAD est réécrite,
        #    donc la ref orpheline n'en est plus un ancêtre → `diverged`.
        git -C "$FIXTURE_CLONE" checkout -q main
        echo "main advance" > "$FIXTURE_CLONE/main-only.txt"
        git -C "$FIXTURE_CLONE" add main-only.txt
        git -C "$FIXTURE_CLONE" commit -q -m "advance main"
        git -C "$FIXTURE_CLONE" push -q origin main
        git -C "$FIXTURE_CLONE" checkout -q "$br"
        git -C "$FIXTURE_CLONE" fetch -q origin main
        git -C "$FIXTURE_CLONE" rebase origin/main >/dev/null 2>&1 \
            || { echo "fixture: rebase échoué; "; return 1; }
        echo "phase 2" > "$FIXTURE_CLONE/phase2.txt"
        git -C "$FIXTURE_CLONE" add phase2.txt
        git -C "$FIXTURE_CLONE" commit -q -m "phase 2 (pilot work)"
    fi

    # Préconditions : la ref de suivi locale existe, le distant ne l'a plus.
    # Sans ces deux assertions, un fixture cassé se lirait comme un correctif
    # qui marche (le piège que V1 existe pour fermer).
    local pre=""
    git -C "$FIXTURE_CLONE" rev-parse --verify "origin/$br" >/dev/null 2>&1 \
        || pre="${pre}fixture: la ref de suivi locale devrait exister; "
    if git -C "$FIXTURE_CLONE" ls-remote --exit-code origin "refs/heads/$br" >/dev/null 2>&1; then
        pre="${pre}fixture: le distant ne devrait PAS porter la branche; "
    fi
    if [ -n "$pre" ]; then echo "$pre"; return 1; fi
    return 0
}

# --- AC1/AC2/V2 : ref locale périmée + branche absente du distant → first-push
test_2626_orphaned_ref_is_first_push() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local br="test/1960/phase"
    local failures=""
    local pre_failures
    pre_failures=$(_t2626_make_orphaned_ref "$br" rewrite) || failures="$pre_failures"

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="$br"
    REPO="mika"
    RESULT=""
    _push_branch

    # AC1 : une ref locale orpheline ne peut plus produire `diverged`.
    if grep -qF -- "mode=diverged" <<<"$RESULT"; then
        failures="${failures}LE DÉFAUT: mode=diverged sur une ref orpheline; "
    fi
    if ! grep -qF -- "mode=first-push" <<<"$RESULT"; then
        failures="${failures}attendu mode=first-push; "
    fi
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}RESULT sans 'Push: pushed'; "
    fi
    # AC4, première moitié : aucune avance distante n'a eu lieu, donc le RESULT
    # ne peut pas l'affirmer (classe mika#2304).
    if grep -qF -- "remote advanced since fetch" <<<"$RESULT"; then
        failures="${failures}RESULT affirme une avance distante qui n'a pas eu lieu; "
    fi
    # AC4, seconde moitié : la ref orpheline est NOMMÉE, même sur le chemin de
    # succès — un checkout partagé qui en porte une est une anomalie d'hygiène
    # qui mérite d'être comptée.
    if ! grep -qF -- "Stale-ref: origin/$br" <<<"$RESULT"; then
        failures="${failures}RESULT ne nomme pas la ref orpheline; "
    fi
    # V2 : la tête distante égale la HEAD locale. Lue par `ls-remote` et non par
    # `rev-parse origin/...` : la ref de suivi locale est précisément ce dont on
    # refuse de croire la parole.
    local local_head remote_head
    local_head=$(git -C "$FIXTURE_CLONE" rev-parse HEAD)
    remote_head=$(git -C "$FIXTURE_CLONE" ls-remote origin "refs/heads/$br" 2>/dev/null | cut -f1)
    [ "$local_head" = "$remote_head" ] \
        || failures="${failures}tête distante ≠ HEAD (local=$local_head remote=${remote_head:-<none>}); "

    # Le mode observé voyage dans le message d'échec : V1 exige de vérifier que
    # le rouge d'avant le correctif dit bien `mode=diverged` / `Push: FAILED`.
    local observed
    observed=$(grep -oE 'mode=[a-z-]+|Push: FAILED[^;]*' <<<"$RESULT" | tr '\n' ' ')

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures[observé: ${observed:-<rien>}]"; fi
}

RESULT_2626A=$(test_2626_orphaned_ref_is_first_push 2>/dev/null)
if [ "$RESULT_2626A" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ mika#2626 (AC1/AC2): ref orpheline → first-push, push réussi"
else
    FAIL=$((FAIL + 1)); echo "  ✗ mika#2626 (AC1/AC2): $RESULT_2626A"
fi

# --- V6 : le court-circuit `ahead == 0` ne s'applique plus à une ref orpheline
#
# Seconde moitié SILENCIEUSE du défaut, et elle est distincte de V2 : sur un
# distant absent, `origin/$BRANCH..HEAD` compte contre une ref orpheline et peut
# rendre 0 alors qu'il y a TOUT à pousser. Un correctif qui ne déplacerait que
# le choix de mode laisserait cette population en `return 0` muet — un no-op qui
# se lit comme « rien à pousser ».
test_2626_zero_ahead_against_orphan_still_pushes() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local br="test/1960/zero-ahead"
    local failures=""
    local pre_failures
    pre_failures=$(_t2626_make_orphaned_ref "$br" keep) || failures="$pre_failures"

    # Précondition propre à V6 : HEAD == la ref orpheline, donc ahead == 0.
    local ahead
    ahead=$(git -C "$FIXTURE_CLONE" rev-list "origin/$br..HEAD" --count 2>/dev/null || echo -1)
    [ "${ahead:-0}" -eq 0 ] \
        || failures="${failures}fixture: ahead devrait être 0 contre la ref orpheline (vu $ahead); "

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="$br"
    REPO="mika"
    RESULT=""
    local rc=0
    _push_branch || rc=$?

    [ "$rc" -eq 0 ] || failures="${failures}attendu rc=0, vu $rc; "
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}LE DÉFAUT: no-op silencieux, rien n'a été poussé; "
    fi
    local local_head remote_head
    local_head=$(git -C "$FIXTURE_CLONE" rev-parse HEAD)
    remote_head=$(git -C "$FIXTURE_CLONE" ls-remote origin "refs/heads/$br" 2>/dev/null | cut -f1)
    [ "$local_head" = "$remote_head" ] \
        || failures="${failures}tête distante ≠ HEAD (local=$local_head remote=${remote_head:-<none>}); "

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_2626B=$(test_2626_zero_ahead_against_orphan_still_pushes 2>/dev/null)
if [ "$RESULT_2626B" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ mika#2626 (V6): ahead==0 contre une ref orpheline pousse quand même"
else
    FAIL=$((FAIL + 1)); echo "  ✗ mika#2626 (V6): $RESULT_2626B"
fi

# --- AC4/V4 : la branche FAILED cesse d'affirmer une avance distante
#
# Le faux énoncé de l'incident vit dans la branche FAILED, pas dans le chemin de
# succès, donc il faut un push qui ÉCHOUE sur un distant qui ne porte pas la
# branche. Un hook `pre-receive` qui décline produit exactement cette
# conjonction : le push est rejeté, `failed to push` matche le prédicat de
# classification mika#1364, et aucune ref distante n'existe pour avoir avancé.
test_2626_failed_push_names_the_orphan_not_a_remote_advance() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local br="test/1960/declined"
    local failures=""
    local pre_failures
    pre_failures=$(_t2626_make_orphaned_ref "$br" rewrite) || failures="$pre_failures"

    # Le hook est posé APRÈS la construction du fixture : les pushes de mise en
    # place doivent aboutir, c'est le push mesuré qui doit être refusé.
    printf '#!/bin/sh\nexit 1\n' > "$FIXTURE_BARE/hooks/pre-receive"
    chmod +x "$FIXTURE_BARE/hooks/pre-receive"

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="$br"
    REPO="mika"
    RESULT=""
    _push_branch || true

    if ! grep -qF -- "Push: FAILED" <<<"$RESULT"; then
        failures="${failures}fixture: le push aurait dû échouer (hook pre-receive); "
    fi
    # AC4 : plus aucune invention d'avance distante sur un distant sans branche.
    if grep -qF -- "remote advanced since fetch" <<<"$RESULT"; then
        failures="${failures}LE DÉFAUT: 'remote advanced since fetch' sur un distant sans branche; "
    fi
    # …et le diagnostic nomme ce qui est vrai : la ref orpheline.
    if ! grep -qF -- "stale tracking ref" <<<"$RESULT"; then
        failures="${failures}le diagnostic FAILED ne nomme pas la ref orpheline; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_2626C=$(test_2626_failed_push_names_the_orphan_not_a_remote_advance 2>/dev/null)
if [ "$RESULT_2626C" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ mika#2626 (AC4): un push refusé sur distant absent nomme la ref orpheline"
else
    FAIL=$((FAIL + 1)); echo "  ✗ mika#2626 (AC4): $RESULT_2626C"
fi

# --- AC3 : le distant PRÉSENT et réellement avancé garde son lease ----------
#
# Contrôle négatif du prédicat. Le Test 12c couvre déjà la divergence nominale
# et reste le domicile de cette non-régression (V3) ; ce qui est vérifié ICI est
# plus étroit et n'existe nulle part ailleurs : que la nouvelle consultation du
# distant ne renverse pas le verdict quand la branche existe bel et bien. Sans
# lui, « `diverged` est préservé » et « le prédicat ne regarde rien » rendraient
# les mêmes octets (classe mika#2205).
test_2626_present_remote_keeps_diverged() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local br="test/1960/present"
    local failures=""

    git -C "$FIXTURE_CLONE" checkout -q -b "$br"
    echo "work" > "$FIXTURE_CLONE/work.txt"
    git -C "$FIXTURE_CLONE" add work.txt
    git -C "$FIXTURE_CLONE" commit -q -m "branch work"
    git -C "$FIXTURE_CLONE" push -q -u origin "$br"

    git -C "$FIXTURE_CLONE" checkout -q main
    echo "main advance" > "$FIXTURE_CLONE/main-only.txt"
    git -C "$FIXTURE_CLONE" add main-only.txt
    git -C "$FIXTURE_CLONE" commit -q -m "advance main"
    git -C "$FIXTURE_CLONE" push -q origin main

    git -C "$FIXTURE_CLONE" checkout -q "$br"
    git -C "$FIXTURE_CLONE" fetch -q origin main
    git -C "$FIXTURE_CLONE" rebase origin/main >/dev/null 2>&1 \
        || failures="${failures}fixture: rebase échoué; "
    echo "pilot" > "$FIXTURE_CLONE/impl.txt"
    git -C "$FIXTURE_CLONE" add impl.txt
    git -C "$FIXTURE_CLONE" commit -q -m "pilot implementation"

    # Ici le distant PORTE la branche — c'est tout le discriminant.
    git -C "$FIXTURE_CLONE" ls-remote --exit-code origin "refs/heads/$br" >/dev/null 2>&1 \
        || failures="${failures}fixture: le distant devrait porter la branche; "

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="$br"
    REPO="mika"
    RESULT=""
    _push_branch

    if ! grep -qF -- "mode=diverged" <<<"$RESULT"; then
        failures="${failures}AC3 cassé: le lease a été perdu sur une divergence légitime; "
    fi
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}RESULT sans 'Push: pushed'; "
    fi
    # Un distant présent n'est pas une ref orpheline : pas de ligne de stale-ref.
    if grep -qF -- "Stale-ref:" <<<"$RESULT"; then
        failures="${failures}faux positif: ligne stale-ref sur un distant présent; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_2626D=$(test_2626_present_remote_keeps_diverged 2>/dev/null)
if [ "$RESULT_2626D" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ mika#2626 (AC3): un distant présent et avancé garde son force-with-lease"
else
    FAIL=$((FAIL + 1)); echo "  ✗ mika#2626 (AC3): $RESULT_2626D"
fi

# --- L'issue INDÉTERMINÉE retombe sur le prédicat d'aujourd'hui -------------
#
# Troisième issue du lecteur, et la plus délicate : le distant n'a pas répondu
# (rc 128 — réseau, authentification, remote injoignable). Elle est NOMMÉE
# plutôt que choisie par défaut, et elle ne devient PAS « absente » : traiter un
# réseau coupé comme une branche absente serait affirmer une mesure qu'on n'a
# pas faite, et pousser en first-push sur une branche que le distant porte
# peut-être. Le repli est le prédicat local, donc exactement le comportement
# d'avant ce correctif — coût borné, et c'est ce qui rend la disposition
# acceptable. Sans ce test, « le repli marche » ne serait pas établi : les
# quatre cas ci-dessus n'exercent que rc 0 et rc 2.
test_2626_undetermined_falls_back_to_the_local_ref() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local br="test/1960/undetermined"
    local failures=""

    # Une divergence légitime d'abord : branche poussée, main avancée, rebase.
    git -C "$FIXTURE_CLONE" checkout -q -b "$br"
    echo "work" > "$FIXTURE_CLONE/work.txt"
    git -C "$FIXTURE_CLONE" add work.txt
    git -C "$FIXTURE_CLONE" commit -q -m "branch work"
    git -C "$FIXTURE_CLONE" push -q -u origin "$br"
    git -C "$FIXTURE_CLONE" checkout -q main
    echo "advance" > "$FIXTURE_CLONE/main-only.txt"
    git -C "$FIXTURE_CLONE" add main-only.txt
    git -C "$FIXTURE_CLONE" commit -q -m "advance main"
    git -C "$FIXTURE_CLONE" push -q origin main
    git -C "$FIXTURE_CLONE" checkout -q "$br"
    git -C "$FIXTURE_CLONE" fetch -q origin main
    git -C "$FIXTURE_CLONE" rebase origin/main >/dev/null 2>&1 \
        || failures="${failures}fixture: rebase échoué; "

    # Puis le distant devient injoignable. Les refs de suivi LOCALES survivent —
    # c'est précisément l'état où le prédicat local est tout ce qui reste.
    git -C "$FIXTURE_CLONE" remote set-url origin "file://$FIXTURE_BARE-does-not-exist"
    git -C "$FIXTURE_CLONE" rev-parse --verify "origin/$br" >/dev/null 2>&1 \
        || failures="${failures}fixture: la ref de suivi locale devrait survivre; "

    local stderr_file="$FIXTURE_BARE/push-stderr"
    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="$br"
    REPO="mika"
    RESULT=""
    _push_branch 2>"$stderr_file" || true

    # Le repli est DIT, pas silencieux : un repli muet se lirait comme une mesure.
    if ! grep -qF -- "remote existence undetermined" "$stderr_file"; then
        failures="${failures}le repli indéterminé n'est pas annoncé; "
    fi
    # …et il est dit sur une surface que QUELQU'UN LIT. Le stderr d'avant-pilote
    # est perdu sur un dispatch qui réussit (classe mika#2050), donc l'assertion
    # ci-dessus seule attesterait un echo, pas une surface.
    if ! grep -qF -- "Remote-probe: existence of origin/$br" <<<"$RESULT"; then
        failures="${failures}le repli n'atteint pas le RESULT; "
    fi
    # Et il ne se fait pas passer pour une absence mesurée.
    if grep -qF -- "Stale-ref:" <<<"$RESULT"; then
        failures="${failures}une absence est affirmée sans avoir été mesurée; "
    fi
    if grep -qF -- "remote has no branch" <<<"$RESULT"; then
        failures="${failures}l'indéterminé a été traité comme 'absente'; "
    fi
    # Comportement d'avant le correctif : le push est tenté et échoue sur le
    # réseau, pas court-circuité en first-push.
    if ! grep -qF -- "Push: FAILED" <<<"$RESULT"; then
        failures="${failures}RESULT sans 'Push: FAILED'; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_2626E=$(test_2626_undetermined_falls_back_to_the_local_ref 2>/dev/null)
if [ "$RESULT_2626E" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ mika#2626: l'issue indéterminée retombe sur le prédicat local, et le dit"
else
    FAIL=$((FAIL + 1)); echo "  ✗ mika#2626 (indéterminé): $RESULT_2626E"
fi

# --- Un VRAI first-push rejeté : aucune ref de suivi locale ------------------
#
# Contrôle négatif du diagnostic AC4, sur la SEULE population où
# `local_ref_sha` est vide en atteignant le chemin FAILED : une branche jamais
# poussée, refusée par la forge. Il n'y a pas de ref orpheline ici, donc le
# diagnostic ne doit pas en inventer une — lui en prêter une serait le défaut
# mika#2304 (affirmer avec autorité ce qui n'a pas eu lieu) inversé, et c'est
# pour cela que le qualificatif est conditionnel plutôt que constant.
#
# Partout ailleurs dans ce bloc la ref de suivi existe, donc aucun autre cas ne
# ferait rougir un qualificatif rendu inconditionnel.
#
# `_push_branch` est appelée NUE : la fonction se termine par `rm -f` et rend
# donc 0 même en FAILED (contrat préexistant — elle ne rend non-zéro que sur le
# guard dedup), il n'y a aucun rc à capturer, et la preuve est la composition du
# RESULT.
test_2626_true_first_push_rejected_invents_no_orphan() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local br="test/1960/never-pushed"
    local failures=""

    # Une branche JAMAIS poussée : aucune ref `origin/$br`, ni locale ni distante.
    git -C "$FIXTURE_CLONE" checkout -q -b "$br"
    echo "work" > "$FIXTURE_CLONE/work.txt"
    git -C "$FIXTURE_CLONE" add work.txt
    git -C "$FIXTURE_CLONE" commit -q -m "branch work"

    if git -C "$FIXTURE_CLONE" rev-parse --verify "origin/$br" >/dev/null 2>&1; then
        failures="${failures}fixture: aucune ref de suivi ne devrait exister; "
    fi

    # Le push est refusé par la forge, pas par le réseau : c'est la conjonction
    # « distant absent + push échoué + aucune ref locale ».
    printf '#!/bin/sh\nexit 1\n' > "$FIXTURE_BARE/hooks/pre-receive"
    chmod +x "$FIXTURE_BARE/hooks/pre-receive"

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="$br"
    REPO="mika"
    RESULT=""
    _push_branch

    if ! grep -qF -- "Push: FAILED" <<<"$RESULT"; then
        failures="${failures}le RESULT n'a pas été composé; "
    fi
    # Aucune ref orpheline n'existe : ni la ligne de détection, ni le qualificatif.
    if grep -qF -- "stale tracking ref" <<<"$RESULT"; then
        failures="${failures}une ref orpheline est inventée sur un vrai first-push; "
    fi
    if grep -qF -- "Stale-ref:" <<<"$RESULT"; then
        failures="${failures}ligne stale-ref sur un vrai first-push; "
    fi
    # Et toujours pas d'avance distante affirmée (AC4).
    if grep -qF -- "remote advanced since fetch" <<<"$RESULT"; then
        failures="${failures}avance distante affirmée sur une branche jamais poussée; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_2626F=$(test_2626_true_first_push_rejected_invents_no_orphan 2>/dev/null)
if [ "$RESULT_2626F" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ mika#2626: un vrai first-push rejeté n'invente aucune ref orpheline"
else
    FAIL=$((FAIL + 1)); echo "  ✗ mika#2626 (vrai first-push): $RESULT_2626F"
fi

# --- INDÉTERMINÉ x aucune ref locale : l'absence non mesurée n'est pas affirmée
#
# La cellule que la revue adversariale a trouvée, et la seule où le diagnostic
# FAILED pourrait affirmer un fait distant qu'aucune mesure ne porte. Les deux
# autres fixtures ne l'atteignent pas : celle du distant injoignable pousse la
# branche d'abord (donc `local_ref_sha` est toujours peuplé), et celle sans ref
# de suivi a un distant joignable.
#
# C'est pour cette cellule que `remote_known_absent` existe à côté de
# `remote_has_branch` : le premier est un FAIT (l'absence a été mesurée), le
# second une DÉCISION (faut-il traiter le distant comme portant la branche).
# Les confondre reproduirait le défaut mika#2304 que ce ticket ferme, une
# cellule plus loin — et sans ce test, rien ne le dirait.
test_2626_undetermined_without_local_ref_asserts_nothing() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local br="test/1960/undetermined-no-ref"
    local failures=""

    # Une branche jamais poussée : aucune ref de suivi.
    git -C "$FIXTURE_CLONE" checkout -q -b "$br"
    echo "work" > "$FIXTURE_CLONE/work.txt"
    git -C "$FIXTURE_CLONE" add work.txt
    git -C "$FIXTURE_CLONE" commit -q -m "branch work"

    # …et un distant injoignable : `ls-remote` ne peut rien mesurer.
    git -C "$FIXTURE_CLONE" remote set-url origin "file://$FIXTURE_BARE-does-not-exist"

    if git -C "$FIXTURE_CLONE" rev-parse --verify "origin/$br" >/dev/null 2>&1; then
        failures="${failures}fixture: aucune ref de suivi ne devrait exister; "
    fi

    local stderr_file="$FIXTURE_BARE/push-stderr-no-ref"
    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="$br"
    REPO="mika"
    RESULT=""
    _push_branch 2>"$stderr_file" || true

    if ! grep -qF -- "remote existence undetermined" "$stderr_file"; then
        failures="${failures}le repli indéterminé n'est pas annoncé; "
    fi
    if ! grep -qF -- "Remote-probe: existence of origin/$br" <<<"$RESULT"; then
        failures="${failures}le repli n'atteint pas le RESULT; "
    fi
    if ! grep -qF -- "Push: FAILED" <<<"$RESULT"; then
        failures="${failures}RESULT sans 'Push: FAILED'; "
    fi
    # LE POINT DU TEST : rien n'a été mesuré, donc rien n'est affirmé.
    if grep -qF -- "remote has no branch" <<<"$RESULT"; then
        failures="${failures}une absence NON MESURÉE est affirmée; "
    fi
    if grep -qF -- "Stale-ref:" <<<"$RESULT"; then
        failures="${failures}ligne stale-ref sans ref de suivi; "
    fi

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_2626G=$(test_2626_undetermined_without_local_ref_asserts_nothing 2>/dev/null)
if [ "$RESULT_2626G" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ mika#2626: indéterminé sans ref locale n'affirme aucune absence"
else
    FAIL=$((FAIL + 1)); echo "  ✗ mika#2626 (indéterminé sans ref): $RESULT_2626G"
fi

# --- DISTANT PRÉSENT x ref de suivi ABSENTE : le second terme de la conjonction
#
# `if [ "$remote_has_branch" -eq 1 ] && [ -n "$local_ref_sha" ]` porte deux
# termes, et celui de droite n'était testé par rien. Il est pourtant porteur :
# tout le bloc qu'il garde LIT la ref de suivi (`origin/$BRANCH..HEAD`,
# `merge-base --is-ancestor`), donc sans lui un distant qui porte la branche
# sans ref locale correspondante entrerait dans un bloc qui compte contre une
# ref absente — `rev-list` échoue, `|| echo 0` rend 0, et `return 0` rend un
# no-op MUET sur une branche qui a tout à pousser. C'est la forme même du
# défaut fondateur, atteinte par l'autre bout.
#
# Le fixture retire le refspec de fetch avant de supprimer la ref de suivi :
# sans refspec, le `git fetch origin "$BRANCH"` de `_push_branch` n'écrit que
# FETCH_HEAD et ne peut pas la recréer, donc la cellule est déterministe. Le
# distant, lui, porte bien la branche.
test_2626_present_remote_without_tracking_ref_still_pushes() {
    _fixture_setup
    _assert_fixture_is_local || return 1

    local br="test/1960/no-tracking-ref"
    local failures=""

    git -C "$FIXTURE_CLONE" checkout -q -b "$br"
    echo "work" > "$FIXTURE_CLONE/work.txt"
    git -C "$FIXTURE_CLONE" add work.txt
    git -C "$FIXTURE_CLONE" commit -q -m "branch work"
    git -C "$FIXTURE_CLONE" push -q -u origin "$br"

    # Un commit de plus, pour qu'il y ait réellement quelque chose à pousser.
    echo "more" > "$FIXTURE_CLONE/more.txt"
    git -C "$FIXTURE_CLONE" add more.txt
    git -C "$FIXTURE_CLONE" commit -q -m "more work"

    # Le distant garde la branche ; la ref de suivi LOCALE disparaît, et le
    # refspec est retiré pour qu'aucun fetch ne la ressuscite.
    git -C "$FIXTURE_CLONE" config --unset remote.origin.fetch
    git -C "$FIXTURE_CLONE" update-ref -d "refs/remotes/origin/$br"

    git -C "$FIXTURE_CLONE" ls-remote --exit-code origin "refs/heads/$br" >/dev/null 2>&1 \
        || failures="${failures}fixture: le distant devrait porter la branche; "
    if git -C "$FIXTURE_CLONE" rev-parse --verify "origin/$br" >/dev/null 2>&1; then
        failures="${failures}fixture: la ref de suivi locale devrait être absente; "
    fi

    WORKTREE_DIR="$FIXTURE_CLONE"
    BRANCH="$br"
    REPO="mika"
    RESULT=""
    _push_branch

    # LE POINT DU TEST : pas de no-op muet.
    if ! grep -qF -- "Push: pushed" <<<"$RESULT"; then
        failures="${failures}no-op muet: rien n'a été poussé alors que le distant porte la branche; "
    fi
    # Aucune ref orpheline ici : le distant est présent et la ref locale absente.
    if grep -qF -- "Stale-ref:" <<<"$RESULT"; then
        failures="${failures}ligne stale-ref sur un distant présent sans ref locale; "
    fi
    local local_head remote_head
    local_head=$(git -C "$FIXTURE_CLONE" rev-parse HEAD)
    remote_head=$(git -C "$FIXTURE_CLONE" ls-remote origin "refs/heads/$br" 2>/dev/null | cut -f1)
    [ "$local_head" = "$remote_head" ] \
        || failures="${failures}tête distante ≠ HEAD (local=$local_head remote=${remote_head:-<none>}); "

    _fixture_cleanup
    if [ -z "$failures" ]; then echo "PASS"; else echo "FAIL: $failures"; fi
}

RESULT_2626H=$(test_2626_present_remote_without_tracking_ref_still_pushes 2>/dev/null)
if [ "$RESULT_2626H" = "PASS" ]; then
    PASS=$((PASS + 1)); echo "  ✓ mika#2626: distant présent sans ref de suivi pousse quand même"
else
    FAIL=$((FAIL + 1)); echo "  ✗ mika#2626 (présent sans ref de suivi): $RESULT_2626H"
fi

# --- V5 : lecteur unique du fait « le distant porte cette branche » ---------
#
# Deux prédicats divergents sur un même fait, c'est le motif que la maison a
# documenté deux fois : mika#2158 (promotion et routage répondaient
# différemment, pendant des mois, sans que rien casse) et mika#2484 (une
# divergence à quatre étapes d'écart dans le même handler). Dans l'incident,
# `_set_up_worktree` a répondu « absente » — c'est pourquoi le worktree a été
# basé sur origin/main — et `_push_branch`, même fichier et même dispatch, a
# répondu « présente ».
#
# Le prédicat porte sur `--exit-code`, la question d'EXISTENCE, et non sur
# `ls-remote` en général : les deux autres sites du fichier lisent un **SHA**
# (`PRE_RUN_REMOTE_HEAD`, `_check_pilot_force_push`), question différente,
# prédicat différent — et un scan qui les dénoncerait à tort se fait désarmer.
echo ""
echo "Test mika#2626 (V5): lecteur unique de l'existence distante"
echo "-----------------------------------------------------------"

M2626_READER_SRC=$(sed -n '/^_remote_branch_exists() {/,/^}/p' "$DISPATCH_LIB")
assert_eq "mika#2626 (V5, anti-vacuité): le lecteur \`_remote_branch_exists\` existe" "yes" \
    "$(if [ -n "$M2626_READER_SRC" ]; then printf 'yes'; else printf 'no'; fi)"

# Les trois issues sont contractuelles, pas observées par accident :
# `git ls-remote --help` — « Exit with status "2" when no matching refs are
# found ». L'issue INDÉTERMINÉE (rc 128, le distant n'a pas répondu) retombe sur
# le prédicat d'aujourd'hui : un signal qu'on ne peut pas lire n'est jamais un
# terme satisfait (mika#2277), appliqué dans les DEUX sens — traiter un réseau
# coupé comme « absente » serait affirmer une mesure qu'on n'a pas faite.
M2626_DOC_SRC=$(sed -n '/^# `_remote_branch_exists /,/^_remote_branch_exists() {/p' "$DISPATCH_LIB")
assert_contains "mika#2626 (V5): le doc-comment porte la table de rc" \
    "rc de ls-remote --exit-code" "$M2626_DOC_SRC"
assert_contains "mika#2626 (V5): et il NOMME l'issue indéterminée plutôt que de la subir" \
    "INDÉTERMINÉE" "$M2626_DOC_SRC"

# Un seul site LIT `ls-remote --exit-code`, et c'est le lecteur.
#
# Le prédicat exclut les lignes de commentaire, et ce terme est load-bearing :
# le fichier MENTIONNE légitimement la forme en prose — la table de rc du
# doc-comment et la note de délégation de `_set_up_worktree` — et un grep nu
# compte ces deux mentions comme des lecteurs. Mesuré pendant cette
# implémentation : 3 occurrences, dont 2 en prose. C'est le faux positif que
# mika#2050 a mesuré sur le Signal S, où la prose d'un pilote qui *discutait* le
# signal se lisait comme une émission.
#
# Le prédicat est défini UNE fois et appelé aux quatre sites, contrôles de
# bonne foi compris. Le recomposer à la main dans les contrôles les rendrait
# muets au resserrement du prédicat réel : ils attesteraient leur propre copie,
# ce qui est la forme exacte d'un contrôle de bonne foi qui ne contrôle rien.
_m2626_count_exitcode_readers() {
    grep -v '^[[:space:]]*#' | grep -cF -- 'ls-remote --exit-code' || true
}

M2626_EXITCODE_SITES=$(_m2626_count_exitcode_readers < "$DISPATCH_LIB")
assert_eq "mika#2626 (V5): EXACTEMENT un site lit \`ls-remote --exit-code\`" "1" \
    "$M2626_EXITCODE_SITES"

M2626_EXITCODE_IN_READER=$(printf '%s\n' "$M2626_READER_SRC" | _m2626_count_exitcode_readers)
assert_eq "mika#2626 (V5): et ce site est DANS le lecteur" "1" \
    "$M2626_EXITCODE_IN_READER"

# Anti-vacuité, et c'est une ASSERTION, pas un commentaire : un prédicat devenu
# trop étroit passerait sinon en ne regardant rien (classe mika#2205).
# EXACTEMENT deux appelants — l.3457 (`_set_up_worktree`, qui délègue) et
# `_push_branch` (nouveau). Les deux n'opèrent pas sur le même répertoire
# (`$SUB_REPO_DIR` contre `$WORKTREE_DIR`), d'où l'argument de répertoire.
#
# Le filtre de commentaires est load-bearing ici AUSSI, et son absence a été
# mesurée : le compte brut rend 3 — les deux appelants (l.3463, l.5879) plus la
# première ligne du doc-comment, qui porte `_remote_branch_exists <repo_dir>`.
# La soustraction d'une ligne de définition ramenait à 2 par COÏNCIDENCE : la
# ligne `_remote_branch_exists() {` est suivie de `(`, donc le regex ne l'a
# jamais comptée, et deux erreurs s'annulaient. Le guard aurait été vert avec
# TROIS vrais appelants — un détecteur d'anti-vacuité vert par accident, qui est
# la classe de panne qu'il existe pour empêcher (mika#2205).
M2626_CALLERS=$(grep -v '^[[:space:]]*#' "$DISPATCH_LIB" \
    | grep -cE '^[[:space:]]*(if |local |[A-Za-z_]+=)?.*_remote_branch_exists[[:space:]]' || true)
assert_eq "mika#2626 (V5, anti-vacuité): EXACTEMENT deux appelants du lecteur" "2" \
    "$M2626_CALLERS"

# Contrôle de bonne foi, DANS LES DEUX SENS, et chaque sens exerce le prédicat
# réel (filtre de commentaires compris) plutôt qu'une approximation.
#
# Sens 1 : un second site remis DOIT être accusé. Sans lui, « exactement un
# site » est indistinguable de « le prédicat ne regarde rien » (mika#2205).
M2626_FX_SECOND_READER='    if git -C "$WORKTREE_DIR" ls-remote --exit-code origin "refs/heads/$BRANCH" >/dev/null 2>&1; then'
assert_eq "mika#2626 (V5, contrôle négatif): le prédicat accuse un second lecteur remis" "1" \
    "$(printf '%s\n' "$M2626_FX_SECOND_READER" | _m2626_count_exitcode_readers)"

# Sens 2 : une MENTION en prose ne doit PAS l'être. C'est le terme qui vient
# d'être ajouté, et sans ce contrôle on ne saurait pas qu'il mord — un scan
# permanemment rouge sur du travail légitime se fait désarmer.
M2626_FX_PROSE_MENTION='            # faux, exactement comme l échec en rc 128 d `ls-remote --exit-code`'
assert_eq "mika#2626 (V5, contrôle négatif): une mention en prose n'est PAS accusée" "0" \
    "$(printf '%s\n' "$M2626_FX_PROSE_MENTION" | _m2626_count_exitcode_readers)"

# Structurel : la décision de mode consulte le distant, et le court-circuit
# `ahead == 0` ne s'applique plus qu'à un distant présent. `declare -f` strippe
# les commentaires, donc on lit la source.
M2626_PUSH_SRC=$(sed -n '/^_push_branch() {/,/^}/p' "$DISPATCH_LIB")
assert_contains "mika#2626: \`_push_branch\` consulte le lecteur avant de choisir son mode" \
    "_remote_branch_exists" "$M2626_PUSH_SRC"

# ============================================================================
# mika#2636 — la section `## Taille estimée` a un site de production (S1–S16)
# ============================================================================
#
# Ce que ces seize détecteurs mesurent. Deux implements consécutifs ont été
# coupés au plafond de tours sur le seul critère du VOLUME — mika#2161 (pilote
# `73720a14`, 151 tours, ≈ 1 470 lignes hors `docs/`) et mika#2633 (pilote
# `2bd6fca0`, 151 tours, ≈ 1 170 lignes) — et AUCUN de leurs plans ne portait
# d'estimation de taille. La pratique « découper en phases au groom » existait
# depuis le 2026-10-01 et RIEN ne la vérifiait : la seconde passe architecte a
# validé le plan de mika#2161 en `PLAN_GROOMED` sans que quoi que ce soit
# demande la taille.
#
# La répartition est celle de la maison — DÉTECTION PERMISSIVE, DÉCISION
# STRICTE : le `grep` dit si la section est PRÉSENTE et si le total est
# PARSABLE, l'architecte seul juge si le total est CRÉDIBLE et si le découpage
# en phases est RÉEL. Aucun prédicat lexical ne peut rendre ces deux jugements,
# et la moitié architecte seule serait de l'enforcement de prompt, que
# `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` borne.
#
# Les quatre contrôles négatifs (S5, S6, S7, S13) sont PORTEURS, pas
# décoratifs : sans eux, une garde qui relance TOUJOURS passerait S1 en vert
# tout en doublant le coût de chaque grooming du dépôt.

echo ""
echo 'Test: mika#2636 — site de production de `## Taille estimée` (S1–S16)'
echo "-----------------------------------------------------------------------"

T2636_PLAN="$REPO_ROOT/docs/plans/2026-10-02-002-feat-2636-estimation-de-taille-au-groom-plan.md"
T2636_SUW_SRC=$(sed -n '/^_set_up_worktree() {/,/^}/p' "$DISPATCH_LIB")
T2636_EMIT_SRC=$(sed -n '/^_emit_pilot_budget_line() {/,/^}/p' "$DISPATCH_LIB")
T2636_GUARD_SRC=$(declare -f _plan_size_retry_if_section_still_missing 2>/dev/null || true)
T2636_REVISE_SRC=$(declare -f _launch_revise_pilot 2>/dev/null || true)
T2636_DETECT_SRC=$(sed -n '/^_detect_plan_on_branch() {/,/^}/p' "$DISPATCH_LIB")

# --- Outillage local de section ---------------------------------------------

# Réplique le site d'injection de `_set_up_worktree` dans son ORDRE réel :
# contexte de ticket, règle de corps de PR (inconditionnelle), règle du scratch
# (inconditionnelle), règle Fire-Disposition puis règle de taille (toutes deux
# conditionnées au skill). S13 asserte sur CETTE chaîne ; une assertion sur la
# source tient le miroir contre la dérive (patron mika#2178 T3 / mika#2306 T4).
_t2636_inject() {
    local skill="$1" repo="${2:-mika}" issue_num="${3:-2636}"
    local prompt="${repo}#${issue_num}"
    prompt=$(printf '%s\n\n%s' "$prompt" "$_PR_BODY_CONTAINMENT_RULE")
    prompt=$(printf '%s\n\n%s' "$prompt" "$_PILOT_SCRATCH_RULE")
    if [ "$skill" = "dev-groom" ]; then
        prompt=$(printf '%s\n\n%s' "$prompt" "$_FIRE_DISPOSITION_RULE")
        _plan_size_rule; prompt=$(printf '%s\n\n%s' "$prompt" "$_PLAN_SIZE_RULE")
    fi
    printf '%s' "$prompt"
}

# Sonde du résolveur de seuil. `__UNSET__` = variable absente.
_t2636_threshold_probe() {
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PLAN_SIZE_MAX_LOC
        if [ "$1" != "__UNSET__" ]; then export PLAN_SIZE_MAX_LOC="$1"; fi
        _plan_size_max_loc
        printf '%s|%s|%s' "$_PLAN_SIZE_MAX_LOC" "${_PLAN_SIZE_MAX_LOC_SOURCE:-}" \
            "${_PLAN_SIZE_MAX_LOC_INVALID:-}"
    )
}

# Sonde du lecteur de total. `$1` = corps du plan (écrit tel quel dans un
# fichier temporaire). Rend ce que `_plan_size_total_loc` imprime.
_t2636_total_probe() {
    local t2636_dir t2636_f t2636_out
    t2636_dir=$(mktemp -d)
    t2636_f="$t2636_dir/plan.md"
    printf '%s\n' "$1" > "$t2636_f"
    t2636_out=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        _plan_size_total_loc "$t2636_f"
    )
    rm -rf "$t2636_dir"
    printf '%s' "$t2636_out"
}

# Sonde du lecteur de PRÉSENCE de section. Rend `oui`/`non`.
_t2636_present_probe() {
    local t2636_dir t2636_f t2636_out
    t2636_dir=$(mktemp -d)
    t2636_f="$t2636_dir/plan.md"
    printf '%s\n' "$1" > "$t2636_f"
    t2636_out=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        if _plan_size_section_present "$t2636_f"; then printf 'oui'; else printf 'non'; fi
    )
    rm -rf "$t2636_dir"
    printf '%s' "$t2636_out"
}

# Sonde de la ligne de journal (porte 3). `$1` = valeur de `_PLAN_SIZE_TOTAL`,
# `$2` = surcharge d'environnement du seuil, `$3` = chemin de plan détecté
# (vide ⇒ aucun plan-on-branch, donc AUCUNE ligne `plan_size_estimate`).
_t2636_emit_probe() {
    # `${3-…}` et NON `${3:-…}` : le troisième argument vide est la valeur qui
    # exprime « aucun plan-on-branch détecté », donc le `:-` le remplacerait par
    # le défaut et le contrôle négatif de porte 3 ne mesurerait rien.
    local t2636_total="$1" t2636_env="${2:-__UNSET__}" t2636_path="${3-docs/plans/sonde-plan.md}"
    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        unset PLAN_SIZE_MAX_LOC
        if [ "$t2636_env" != "__UNSET__" ]; then export PLAN_SIZE_MAX_LOC="$t2636_env"; fi
        _PLAN_SIZE_PLAN_PATH="$t2636_path"
        _PLAN_SIZE_TOTAL="$t2636_total"
        _emit_pilot_budget_line claude-pilot --max-turns 150 2>&1 >/dev/null
    )
}

# Sonde comportementale de `_launch_revise_pilot`, calquée terme pour terme sur
# `_t2306_revise_probe` : seul `_run_pilot_sandboxed` est neutralisé, tout le
# reste de la fonction s'exécute tel quel — la seule façon de prouver que le
# compte de relances vient de la garde et de rien d'autre.
#
# $1 = contenu de .iterate/findings-1.md
# $2 = forme de la section de taille dans le plan initial :
#      no | yes | numbered | fenced (le titre n'existe QUE dans un bloc clôturé)
# $3 = comportement du pilote simulé : never | second | wipe
# $4 = (optionnel) "residual-size" dépose un findings-1-size.md résiduel
# Rend : "<rc>|<nombre d'invocations du pilote>|<stderr>"
_t2636_revise_probe() {
    local t2636_findings="$1" t2636_form="$2" t2636_behaviour="$3" t2636_extra="${4:-}"
    local t2636_root t2636_wt t2636_plan t2636_counter t2636_err t2636_rc
    t2636_root=$(mktemp -d)
    t2636_wt="$t2636_root/.claude/worktrees/feat-2636-probe/mika"
    mkdir -p "$t2636_wt/docs/plans" "$t2636_wt/.iterate"
    t2636_plan="$t2636_wt/docs/plans/2026-10-02-002-feat-2636-sonde-plan.md"

    # > 500 octets : `_find_issue_plan` filtre les plans plus courts (mika#1033).
    # Un plan sous le seuil fait rendre « no plan file to revise », c'est-à-dire
    # un vert qui n'a rien exercé de la garde.
    {
        printf '# mika#2636 — plan de sonde\n\n**Ticket :** mika issue#2636\n\n## Problème\n\n'
        printf 'Corps de remplissage pour franchir le filtre des 500 octets applique par\n'
        printf '_find_issue_plan a tous ses tiers de decouverte. Ce texte ne porte aucune\n'
        printf 'signification pour la sonde : seules comptent sa longueur, et la presence\n'
        printf "ou l'absence de la section testee.\n\n"
        printf 'Le seuil existe pour ecarter les fichiers-fantomes et les ebauches vides ;\n'
        printf "il s'applique identiquement aux trois tiers de decouverte, donc un plan\n"
        printf 'trop court est invisible quelle que soit la façon dont il est nomme.\n\n'
        printf '## Livrables\n\nL1 — un livrable quelconque.\n\n'
        case "$t2636_form" in
            yes)      printf '## Taille estimée\n\nTotal estimé : 400 lignes\n\n' ;;
            numbered) printf '## 6. Taille estimée\n\nTotal estimé : 400 lignes\n\n' ;;
            fenced)   printf '```markdown\n## Taille estimée\n\nTotal estimé : 395 lignes\n```\n\n' ;;
        esac
        printf '## Acceptance criteria\n\nAC1 — la sonde tourne.\n'
    } > "$t2636_plan"

    printf '%s\n' "$t2636_findings" > "$t2636_wt/.iterate/findings-1.md"
    if [ "$t2636_extra" = "residual-size" ]; then
        printf 'Taille estimée — residu laisse par un dispatch anterieur.\n' \
            > "$t2636_wt/.iterate/findings-1-size.md"
    fi

    t2636_counter="$t2636_root/pilot-invocations"
    printf '0\n' > "$t2636_counter"

    t2636_err=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        _run_pilot_sandboxed() {
            local n; n=$(( $(cat "$t2636_counter") + 1 ))
            printf '%s\n' "$n" > "$t2636_counter"
            case "$t2636_behaviour" in
                wipe) rm -f "$t2636_wt/.iterate/findings-1.md" ;;
                second) [ "$n" -ge 2 ] && printf '\n## 6. Taille estimée\n\nTotal estimé : 400 lignes\n' >> "$t2636_plan" ;;
            esac
            # Mutation inconditionnelle : sans elle le sha ne bouge pas et la
            # branche de succès — donc la garde — n'est jamais atteinte.
            printf '\n<!-- revise %s -->\n' "$n" >> "$t2636_plan"
            return 0
        }
        set +e
        WORKTREE_DIR="$t2636_wt" ISSUE_NUM="2636" REPO="mika" LOG_ID="t2636" \
            CWD_ARGS="--cwd $t2636_wt" \
            _launch_revise_pilot "$t2636_wt/.iterate/findings-1.md" 2>&1 >/dev/null
        printf '%s\n' "$?" > "$t2636_root/rc"
    )
    t2636_rc=$(cat "$t2636_root/rc")
    printf '%s|%s|%s' "$t2636_rc" "$(cat "$t2636_counter")" "$t2636_err"
    rm -rf "$t2636_root"
}

# Le champ 3 est du stderr libre — multiligne, pouvant contenir des `|`.
_t2636_field() { printf '%s' "$2" | head -1 | cut -d'|' -f"$1"; }
_t2636_err()   { printf '%s' "$1" | sed '1s/^[0-9]*|[0-9]*|//'; }

T2636_FINDINGS_WITH_SIZE="F1 [BLOQUANT] — le plan ne porte pas de section \`## Taille estimée\`.
Ajoute-la : lignes de code hors \`docs/\` par livrable, puis la ligne \`Total estimé :\`.

Disposition: ITERATE"
T2636_FINDINGS_WITHOUT_SIZE="F1 [BLOQUANT] — l'unité L2 laisse le choix de la structure de
données à l'implémenteur. Tranche-le dans le plan.

Disposition: ITERATE"
T2636_FINDINGS_BOTH="F1 [BLOQUANT] — le plan livre des détecteurs et ne porte pas de section
\`## Fire-Disposition\`.
F2 [BLOQUANT] — le plan ne porte pas de section \`## Taille estimée\`.

Disposition: ITERATE"

# --- S1 : le cœur du correctif ----------------------------------------------
#
# Findings réclamant la section + plan révisé qui ne la porte toujours pas
# ⇒ exactement UNE relance (deux invocations du pilote : la nominale et elle).
# VU ROUGE avant le correctif : la fonction n'existait pas, donc zéro relance.

T2636_S1=$(_t2636_revise_probe "$T2636_FINDINGS_WITH_SIZE" "no" "never")
assert_eq "S1: findings réclamant la taille + section absente ⇒ exactement une relance" \
    "2" "$(_t2636_field 2 "$T2636_S1")"
assert_contains "S1: et la relance est DITE, pas silencieuse" \
    "plan_size_revise_retried" "$T2636_S1"
assert_eq "S1: la valeur de retour reste 0 — la garde ajoute une tentative, pas un mode d'échec" \
    "0" "$(_t2636_field 1 "$T2636_S1")"
assert_contains "S1: l'échec de second tour est DIT" \
    "plan_size_still_missing_after_retry" "$T2636_S1"

# Et quand la seconde tentative réussit, elle se tait : l'événement d'échec est
# réservé à l'échec, sinon il ne mesure plus rien.
T2636_S1B=$(_t2636_revise_probe "$T2636_FINDINGS_WITH_SIZE" "no" "second")
assert_eq "S1: seconde tentative réussie ⇒ toujours une seule relance" \
    "2" "$(_t2636_field 2 "$T2636_S1B")"
assert_not_contains "S1: et aucun événement d'échec quand la section arrive" \
    "plan_size_still_missing_after_retry" "$T2636_S1B"

# --- S2 : un plan sous le seuil passe ---------------------------------------
#
# 515 lignes, la valeur d'AC4, mesurée sur mika#1960 phase 2 (qui n'est pas mort
# de volume). Verdict `under_threshold`, et ZÉRO relance sur un plan conforme —
# le contrôle négatif que mika#2544 a dû livrer après qu'un rattrapage a relancé
# un pilote pour rien sur un plan numéroté conforme.

T2636_S2=$(_t2636_emit_probe "515")
assert_contains "S2: un plan à 515 lignes rend verdict=under_threshold" \
    "verdict=under_threshold" "$T2636_S2"
assert_contains "S2: et la ligne porte le total et le seuil" \
    "total_loc=515 threshold_loc=1000" "$T2636_S2"
T2636_S2B=$(_t2636_revise_probe "$T2636_FINDINGS_WITH_SIZE" "numbered" "never")
assert_eq "S2: section déjà présente ⇒ zéro relance (le chemin nominal ne paie rien)" \
    "1" "$(_t2636_field 2 "$T2636_S2B")"
assert_not_contains "S2: et rien n'est journalisé" \
    "plan_size_revise_retried" "$T2636_S2B"

# --- S3 : un plan au-dessus du seuil est nommé ------------------------------
#
# 1 400 lignes, la valeur d'AC4. VU ROUGE avant le correctif : aucune ligne
# `plan_size_estimate` n'existait, donc le dispatch ne disait rien du volume.
# Le REFUS, lui, vit chez l'architecte (S14 / L2 / L3) : la porte 3 journalise
# et ne refuse JAMAIS, sans quoi la file groomée entière serait gelée (R3).

T2636_S3=$(_t2636_emit_probe "1400")
assert_contains "S3: un plan à 1 400 lignes rend verdict=over_threshold" \
    "verdict=over_threshold" "$T2636_S3"
assert_contains "S3: et le total est nommé" "total_loc=1400" "$T2636_S3"
# Le seuil décide, pas le nombre : le même total sous un seuil relevé passe.
T2636_S3B=$(_t2636_emit_probe "1400" "2000")
assert_contains "S3: le SEUIL décide — 1 400 sous un seuil de 2 000 passe" \
    "verdict=under_threshold" "$T2636_S3B"
assert_contains "S3: et la provenance du seuil est dite" \
    "threshold_loc=2000 source=env" "$T2636_S3B"
# Porte 3 ne refuse rien : aucun vocabulaire de refus sur cette ligne.
assert_not_contains "S3: la porte 3 ne refuse jamais (R3)" "REFUSED" "$T2636_S3"

# --- S4 : la numérotation ne change pas la section --------------------------
#
# `/ce:plan` numérote ses titres. C'est le défaut que mika#2544 a dû corriger
# après qu'un rattrapage Fire-Disposition a tiré à tort sur un plan conforme.
# VU ROUGE si le motif est un littéral `^## Taille estimée`.

assert_eq "S4: _PLAN_SIZE_HEADING_RE est définie et non vide" "non-vide" \
    "$([ -n "${_PLAN_SIZE_HEADING_RE:-}" ] && echo non-vide || echo vide)"
_t2636_heading_match() {
    if [ -n "${_PLAN_SIZE_HEADING_RE:-}" ] && grep -qiE -- "$_PLAN_SIZE_HEADING_RE" <<<"$1"; then
        printf 'match'
    else
        printf 'no-match'
    fi
}
assert_eq "S4: '## Taille estimée' apparie" "match" \
    "$(_t2636_heading_match '## Taille estimée')"
assert_eq "S4: '## 7. Taille estimée' apparie (numérotation, mika#2544)" "match" \
    "$(_t2636_heading_match '## 7. Taille estimée')"
assert_eq "S4: '## 7 Taille estimée' apparie (sans point)" "match" \
    "$(_t2636_heading_match '## 7 Taille estimée')"
assert_eq "S4: '##   Taille estimée' apparie (espaces multiples)" "match" \
    "$(_t2636_heading_match '##   Taille estimée')"
assert_eq "S4: '## 6. TAILLE ESTIMÉE' apparie (casse repliée)" "match" \
    "$(_t2636_heading_match '## 6. TAILLE ESTIMÉE')"
assert_eq "S4: '## Taille estimée (par phase)' apparie (pas d'ancre \$)" "match" \
    "$(_t2636_heading_match '## Taille estimée (par phase)')"
# Contrôles négatifs du motif : le texte reste ancré juste après le préfixe.
assert_eq "S4: '### Taille estimée' n'apparie PAS" "no-match" \
    "$(_t2636_heading_match '### Taille estimée')"
assert_eq "S4: '## Notes sur la Taille estimée' n'apparie PAS" "no-match" \
    "$(_t2636_heading_match '## Notes sur la Taille estimée')"
assert_eq "S4: '##Taille estimée' n'apparie PAS (pas un titre Markdown)" "no-match" \
    "$(_t2636_heading_match '##Taille estimée')"
# Hors périmètre NOMMÉ : `## 1.1 …` n'est pas couvert, exactement comme
# `_FD_HEADING_RE`. Élargir sur une devinette est refusé (mika#2544) ; le jour
# où la forme est mesurée, l'élargissement bouge les DEUX motifs.
assert_eq "S4: '## 1.1 Taille estimée' n'apparie PAS (borne héritée de mika#2544)" "no-match" \
    "$(_t2636_heading_match '## 1.1 Taille estimée')"
# Co-mutation avec son jumeau : le préfixe de numérotation est le MÊME.
assert_eq "S4 (co-mutation): préfixe de _PLAN_SIZE_HEADING_RE == préfixe de _FD_HEADING_RE" \
    "${_FD_HEADING_RE%Fire-Disposition}" "${_PLAN_SIZE_HEADING_RE%Taille estimée}"

# --- S5 : contrôle négatif — l'architecte n'a rien demandé ------------------

T2636_S5=$(_t2636_revise_probe "$T2636_FINDINGS_WITHOUT_SIZE" "no" "never")
assert_eq "S5: findings sans mention de la taille ⇒ zéro relance" \
    "1" "$(_t2636_field 2 "$T2636_S5")"
assert_not_contains "S5: et rien n'est journalisé" \
    "plan_size_revise_retried" "$T2636_S5"

# --- S6 : fail-safe — une information illisible SORT de la population -------

T2636_S6=$(_t2636_revise_probe "$T2636_FINDINGS_WITH_SIZE" "no" "wipe")
assert_eq "S6: findings disparu en vol ⇒ zéro relance" \
    "1" "$(_t2636_field 2 "$T2636_S6")"
assert_eq "S6: et la valeur de retour reste celle d'avant le correctif" \
    "0" "$(_t2636_field 1 "$T2636_S6")"
assert_not_contains "S6: et aucun événement n'est émis" \
    "plan_size_revise_retried" "$T2636_S6"
assert_eq "S6: un plan illisible rend 'absent', jamais un total inventé" "absent" \
    "$( (source "$DISPATCH_LIB" 2>/dev/null || true; _plan_size_total_loc /nonexistent/mika-2636.md) )"

# --- S7 : relance UNIQUE, terminaison lisible sans dérouler le flot ---------

assert_contains "S7: le compteur de garde existe et est armé AVANT l'action" \
    "_PLAN_SIZE_REVISE_RETRIED=1" "$T2636_GUARD_SRC"
assert_contains "S7: et il est remis à zéro à chaque entrée de _launch_revise_pilot" \
    "_PLAN_SIZE_REVISE_RETRIED=0" "$T2636_REVISE_SRC"

# Le no-op du compteur se mesure sur une paire où TOUS LES AUTRES TERMES SONT
# VRAIS, sinon l'assertion est vide. La première rédaction passait ce plan-ci
# des deux côtés : S16 asserte qu'il porte la section, donc le terme 2
# court-circuitait AVANT le compteur et l'assertion ne distinguait pas un
# compteur armé d'un compteur absent (vérifié : retirer la ligne de budget
# laissait le test vert). La sonde construit donc une paire dont les deux
# `grep` sont vrais, et porte son CONTRÔLE POSITIF — compteur à zéro ⇒ une
# relance — sans lequel « le compteur arrête » serait indistinguable de « la
# garde n'a jamais rien fait ».
_t2636_budget_probe() {
    local t2636_armed="$1" t2636_dir t2636_out
    t2636_dir=$(mktemp -d)
    printf 'F1 — ajoute une section `## Taille estimée`.\n' > "$t2636_dir/findings-1.md"
    printf '# plan\n\n## Livrables\n\nL1 — rien.\n' > "$t2636_dir/plan.md"
    t2636_out=$(
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true
        _run_pilot_sandboxed() { return 0; }
        _PLAN_SIZE_REVISE_RETRIED="$t2636_armed"
        LOG_ID=t2636-budget REPO=mika ISSUE_NUM=2636 CWD_ARGS="" \
            _plan_size_retry_if_section_still_missing \
                "$t2636_dir/findings-1.md" "$t2636_dir/plan.md" 2>&1 >/dev/null \
          | grep -c 'plan_size_revise_retried' || true
    )
    rm -rf "$t2636_dir"
    printf '%s' "$t2636_out"
}
assert_eq "S7: compteur déjà armé ⇒ aucune relance (le budget arrête)" "0" \
    "$(_t2636_budget_probe 1)"
assert_eq "S7 contrôle positif: compteur à zéro ⇒ une relance (la garde mord)" "1" \
    "$(_t2636_budget_probe 0)"
assert_eq "S7: et AUCUNE troisième relance sur le chemin S1" "2" "$(_t2636_field 2 "$T2636_S1")"
assert_eq "S7: l'événement d'échec est émis exactement une fois" "1" \
    "$(_t2636_err "$T2636_S1" | grep -c 'plan_size_still_missing_after_retry' || true)"
# Le budget ARCHITECTE est intact : aucun `_arch_ask` sur ce chemin. La
# régression qu'il attrape ne rendrait AUCUNE décision fausse — elle doublerait
# le budget LLM par grooming sans qu'un test de comportement ne rougisse.
assert_eq "S7: zéro appel _arch_ask dans la garde de rattrapage" "0" \
    "$(printf '%s\n' "$T2636_GUARD_SRC" | grep -c '_arch_ask' || true)"

# --- S8 : contrôle de source — le prédicat auto-entretenu -------------------
#
# La seule défaillance de cette famille qu'aucun autre test ne voit. Un
# implémenteur qui lit le findings CIBLÉ (`findings-1-size.md`, qui contient
# nécessairement la chaîne — c'est son objet) rend le premier terme vrai par
# construction : la garde relance même quand l'architecte n'a rien demandé, le
# compteur borne la boucle, et S1, S5, S6 restent TOUS verts.

T2636_SIZE_VAR_LINES=$(printf '%s\n' "$T2636_GUARD_SRC" | grep 'size_findings_file' || true)
assert_eq "S8: aucune ligne touchant findings-1-size.md ne l'interroge par grep" "0" \
    "$(printf '%s\n' "$T2636_SIZE_VAR_LINES" | grep -c 'grep' || true)"
assert_eq "S8: et la garde ne se relance pas par récursion sur _launch_revise_pilot" "0" \
    "$(printf '%s\n' "$T2636_GUARD_SRC" | grep -c '_launch_revise_pilot' || true)"
assert_eq "S8 contre-vacuité: la garde écrit bien un findings ciblé" "yes" \
    "$([ -n "$T2636_SIZE_VAR_LINES" ] && echo yes || echo non-trouvé)"
assert_contains "S8: le terme 1 interroge le findings de première passe REÇU EN ARGUMENT" \
    'first_pass_findings' \
    "$(printf '%s\n' "$T2636_GUARD_SRC" | grep 'Taille estimée' | grep 'grep -qF' || true)"
# Comportemental : un findings-1-size.md résiduel ne rend aucun terme vrai.
T2636_S8B=$(_t2636_revise_probe "$T2636_FINDINGS_WITHOUT_SIZE" "no" "never" "residual-size")
assert_eq "S8: un findings-1-size.md résiduel ne déclenche aucune relance" \
    "1" "$(_t2636_field 2 "$T2636_S8B")"

# --- S9 : trois issues distinctes, et `absent` n'est JAMAIS `0` -------------
#
# Confondre « le plan n'a pas été dimensionné » et « le plan a été dimensionné
# dans une forme que le lecteur ne sait pas lire » rendrait les deux
# indiscernables, alors que les remèdes sont OPPOSÉS (groomer vs réparer le
# motif). Et `0` serait un total plausible, présenté avec autorité, faux —
# *un `null` n'est jamais un `0`* (mika#2331).

assert_eq "S9: section absente ⇒ 'absent'" "absent" \
    "$(_t2636_total_probe '# Plan

## Livrables

L1 — rien.')"
assert_eq "S9: section présente sans total ⇒ 'unparsable'" "unparsable" \
    "$(_t2636_total_probe '# Plan

## 6. Taille estimée

| livrable | lignes |
|---|---|
| `a.rs` | 120 |')"
assert_eq "S9: total apparié ⇒ le nombre" "395" \
    "$(_t2636_total_probe '# Plan

## Taille estimée

Total estimé : 395 lignes')"
assert_eq "S9: un total nu (sans unité) apparie aussi" "42" \
    "$(_t2636_total_probe '# Plan

## Taille estimée

Total estimé : 42')"
assert_eq "S9: le DERNIER match gagne — un total par phase précède le total global" "900" \
    "$(_t2636_total_probe '# Plan

## Taille estimée

Total estimé : 300 lignes (phase 1)

Total estimé : 900 lignes')"
# Hors périmètre NOMMÉ : séparateur de milliers non apparié, forme non
# prescrite, élargissement sur devinette refusé (mika#2544).
assert_eq "S9: un séparateur de milliers n'apparie pas ⇒ 'unparsable'" "unparsable" \
    "$(_t2636_total_probe '# Plan

## Taille estimée

Total estimé : 1 400 lignes')"
# Et la ligne de journal distingue bien les trois. Chaque sonde `source` la
# bibliothèque entière, donc on la hisse une fois et on asserte plusieurs fois.
T2636_S9_ABSENT=$(_t2636_emit_probe "absent")
assert_contains "S9: la ligne porte 'absent', jamais 0" "total_loc=absent" "$T2636_S9_ABSENT"
assert_contains "S9: 'absent' rend verdict=unknown" "verdict=unknown" "$T2636_S9_ABSENT"
T2636_S9_UNPARSABLE=$(_t2636_emit_probe "unparsable")
assert_contains "S9: 'unparsable' est distinct d''absent' sur la ligne" "total_loc=unparsable" \
    "$T2636_S9_UNPARSABLE"
assert_contains "S9: et 'unparsable' rend verdict=unknown lui aussi" "verdict=unknown" \
    "$T2636_S9_UNPARSABLE"
# Contrôle négatif de porte 3 : aucun plan détecté ⇒ AUCUNE ligne d'estimation.
# Et TOUJOURS une ligne quand il l'a été, sinon zéro ligne se lirait « tous les
# plans sont dimensionnés » alors qu'elle voudrait dire « aucun ne l'est »
# (classe mika#2205 appliquée à la sonde de ce ticket).
T2636_S9_NOPLAN=$(_t2636_emit_probe "530" "__UNSET__" "")
assert_not_contains "S9: aucun plan-on-branch ⇒ aucune ligne plan_size_estimate" \
    "plan_size_estimate" "$T2636_S9_NOPLAN"
assert_contains "S9: mais le seuil est DIT même sans plan (doctrine mika#2293)" \
    "plan_size_threshold_resolved" "$T2636_S9_NOPLAN"

# --- S10 : le SINK, et c'est la leçon la plus coûteuse de ce ticket ---------
#
# Émettre avant la ligne de lancement enverrait la ligne sur le stderr propre de
# `dispatch-lib`, que l'exécuteur ne lit QUE dans sa branche
# `if !status.success()` : sur un dispatch qui réussit — et un dispatch implement
# réussit — le tuyau est jeté sans être lu et la ligne n'atterrit dans AUCUN
# fichier. C'est le Signal M, mesuré par mika#2050.

assert_contains "S10: la ligne plan_size_estimate est émise depuis _emit_pilot_budget_line" \
    "plan_size_estimate" "$T2636_EMIT_SRC"
assert_contains "S10: idem pour plan_size_threshold_resolved" \
    "plan_size_threshold_resolved" "$T2636_EMIT_SRC"
assert_eq "S10: et nulle part ailleurs dans le fichier (site unique d'émission)" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF 'plan_size_estimate total_loc=' || true)"
# Contre-vacuité : l'émetteur a bien été trouvé.
assert_eq "S10 contre-vacuité: _emit_pilot_budget_line a été trouvée" "yes" \
    "$([ -n "$T2636_EMIT_SRC" ] && echo yes || echo no)"
# Et le sink lui-même : l'émetteur est appelé depuis `_run_pilot_sandboxed`,
# donc sous la redirection `2>"$STDERR_FILE"` du site de lancement.
assert_contains "S10: l'émetteur est appelé depuis _run_pilot_sandboxed" \
    "_emit_pilot_budget_line" "$(sed -n '/^_run_pilot_sandboxed() {/,/^}/p' "$DISPATCH_LIB")"

# --- S10-bis : le PRODUCTEUR de la porte 3, et pas seulement son émetteur ----
#
# L'émetteur ne peut rien dire si personne ne pose les deux globales. La
# première rédaction extrayait `_detect_plan_on_branch` dans `T2636_DETECT_SRC`
# et ne l'assertait JAMAIS : supprimer les deux lignes de pose laissait les
# 1355 assertions vertes et la porte 3 muette, parce que `_t2636_emit_probe`
# pose `_PLAN_SIZE_PLAN_PATH` / `_PLAN_SIZE_TOTAL` À LA MAIN. Même forme que
# mika#2492 T5, qui tient la pose sœur `PILOT_SHIPPING_TAIL` de cette même
# fonction.
assert_eq "S10-bis contre-vacuité: _detect_plan_on_branch a bien été extraite" "non-vide" \
    "$([ -n "$T2636_DETECT_SRC" ] && echo non-vide || echo vide)"
assert_contains "S10-bis: le producteur pose le discriminant d'émission" \
    '_PLAN_SIZE_PLAN_PATH="$PLAN_PATH"' "$T2636_DETECT_SRC"
assert_contains "S10-bis: et il pose le total depuis le lecteur unique" \
    '_PLAN_SIZE_TOTAL=$(_plan_size_total_loc' "$T2636_DETECT_SRC"
# Site unique de pose : une seconde pose ailleurs ferait émettre la ligne sur un
# dispatch qui n'a pas de plan, ou la ferait porter le total d'un autre.
assert_eq "S10-bis: la pose du discriminant a un site unique" "1" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '_PLAN_SIZE_PLAN_PATH="$PLAN_PATH"' || true)"
# Et elle vit sur la branche où le fichier plan est CONFIRMÉ dans le worktree :
# posée sur le bras `else`, elle annoncerait un plan que le pilote ne lira pas.
assert_eq "S10-bis: la pose suit la bascule de l'ENTRY_COMMAND" "yes" \
    "$(_t2178_after '_PLAN_SIZE_PLAN_PATH="$PLAN_PATH"' 'ENTRY_COMMAND="/ce-work $PLAN_PATH"')"

# --- S11 : co-location, le coût de l'accesseur assignant -------------------
#
# Même mécanisme et même raison que `$_PILOT_LOG_DIR` (mika#2165) et
# `$_PILOT_MAX_TURNS` (mika#2496) : un accesseur qui ASSIGNE peut être lu
# périmé. Lue sur l'INVOCATION LOGIQUE, pas la ligne physique.

# Le joint d'invocations logiques est un `sed` à boucle sur tout le fichier : il
# est hissé une fois, et le scan comme son contrôle de vacuité lisent la même
# sortie.
T2636_LOGICAL=$(_mika2496_logical_invocations "$DISPATCH_LIB")
T2636_READS=$(printf '%s\n' "$T2636_LOGICAL" | grep -E '\$\{?_PLAN_SIZE_MAX_LOC\b' || true)
T2636_UNCOLOCATED=$(printf '%s\n' "$T2636_READS" | grep -vE '_plan_size_max_loc' | grep -E . || true)
assert_eq "S11: chaque lecture de \$_PLAN_SIZE_MAX_LOC appelle le résolveur sur la même ligne" "" \
    "$T2636_UNCOLOCATED"
# Contre-vacuité : le scan voit bien des lectures.
assert_eq "S11 contre-vacuité: au moins une lecture de \$_PLAN_SIZE_MAX_LOC existe" "yes" \
    "$( [ -n "$T2636_READS" ] && echo yes || echo non-trouvé)"
# Le résolveur ASSIGNE, il n'imprime pas — collision mika#2039.
T2636_RESOLVER_SRC=$(sed -n '/^_plan_size_max_loc()/,/^}/p' "$DISPATCH_LIB")
assert_eq "S11: _plan_size_max_loc a bien été trouvée (guards the guard)" "yes" \
    "$([ -n "$T2636_RESOLVER_SRC" ] && echo yes || echo no)"
assert_eq "S11 × mika#2039: le résolveur n'imprime pas" "0" \
    "$(printf '%s\n' "$T2636_RESOLVER_SRC" | grep -cE '^[[:space:]]*(printf|echo)[[:space:]]' || true)"
assert_eq "S11 × mika#2039: aucune substitution \$(_plan_size_max_loc) ne subsiste" "0" \
    "$(grep -vE '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '$(_plan_size_max_loc)' || true)"

# --- S12 : les trois paliers du seuil, et le `0` ne désarme pas ------------
#
# Sur une garde dont le rôle est de borner un coût, une coquille ne doit pas
# être un désarmement silencieux.

T2636_THRESHOLD_UNSET=$(_t2636_threshold_probe __UNSET__)
assert_eq "S12: absent ⇒ le défaut in-file, provenance default" "1000|default|" \
    "$T2636_THRESHOLD_UNSET"
assert_eq "S12: vide ⇒ le défaut, provenance default" "1000|default|" \
    "$(_t2636_threshold_probe "")"
assert_eq "S12: entier positif ⇒ cette valeur, provenance env" "500|env|" \
    "$(_t2636_threshold_probe 500)"
assert_eq "S12: '0' ⇒ le défaut, et la valeur est NOMMÉE" "1000|default|0" \
    "$(_t2636_threshold_probe 0)"
assert_eq "S12: négatif ⇒ le défaut, et la valeur est NOMMÉE" "1000|default|-5" \
    "$(_t2636_threshold_probe "-5")"
assert_eq "S12: illisible ⇒ le défaut, et la valeur est NOMMÉE" "1000|default|mille" \
    "$(_t2636_threshold_probe mille)"
assert_contains "S12: et l'événement nomme la valeur fautive entre guillemets" \
    'plan_size_threshold_invalid PLAN_SIZE_MAX_LOC="mille"' \
    "$(_t2636_emit_probe "530" "mille")"
assert_contains "S12: la provenance du seuil est journalisée (doctrine mika#2293)" \
    "plan_size_threshold_resolved threshold_loc=1000 source=default" \
    "$(_t2636_emit_probe "530")"

# --- S13 : la règle est injectée au groomeur, et à lui SEUL ----------------
#
# Contrôle négatif PORTEUR : le site voisin (`_PR_BODY_CONTAINMENT_RULE`) est
# inconditionnel dans cette branche, donc la condition se PERD en copiant le
# voisin. Cet écart ne rendrait aucune décision fausse — il mettrait du bruit
# dans le prompt de tout pilote d'implémentation (patron mika#2306 T3).

assert_eq "S13: _PLAN_SIZE_RULE_TEMPLATE est définie et non vide" "non-vide" \
    "$([ -n "${_PLAN_SIZE_RULE_TEMPLATE:-}" ] && echo non-vide || echo vide)"
_plan_size_rule 2>/dev/null || true
assert_contains "S13: la règle nomme la section exacte" \
    '## Taille estimée' "${_PLAN_SIZE_RULE:-}"
assert_contains "S13: la règle nomme la ligne de total" \
    'Total estimé' "${_PLAN_SIZE_RULE:-}"
assert_contains "S13: la règle interpole le seuil en vigueur" "1000" "${_PLAN_SIZE_RULE:-}"
assert_not_contains "S13: et le gabarit a bien été substitué" \
    "__PLAN_SIZE_MAX_LOC__" "${_PLAN_SIZE_RULE:-}"
assert_contains "S13: la règle dit la conduite au-dessus du seuil" \
    "DÉCOUPE EN PHASES" "${_PLAN_SIZE_RULE:-}"
assert_contains "S13: et elle dit où la suite est renvoyée" \
    "ticket de suivi" "${_PLAN_SIZE_RULE:-}"
assert_contains "S13: la règle cite le ticket par référence" "mika#2636" "${_PLAN_SIZE_RULE:-}"

# S13-bis — LE FORMAT PRESCRIT A UN SITE UNIQUE, et c'est le scan qui le tient.
#
# Deux textes prescrivent ce format : la règle injectée au groomeur, et le
# finding synthétique du rattrapage. La première rédaction de ce correctif les
# avait déjà laissés DIVERGER (deux jeux de lignes d'exemple différents), et
# rien ne l'aurait dit : S13 n'asserte qu'une présence de sous-chaîne dans la
# règle, S14 n'épingle que le NOMBRE contre les deux prompts architecte. Or ce
# format straddle le contrat des deux motifs — `## Taille estimée` est ce que
# `$_PLAN_SIZE_HEADING_RE` apparie, `Total estimé : <n>` ce que
# `$_PLAN_SIZE_TOTAL_RE` extrait — donc un changement de motif doit bouger un
# seul texte, et le finding est précisément le message envoyé quand le format
# n'a PAS été suivi la première fois.
assert_eq "S13-bis: _PLAN_SIZE_FORMAT_SPEC est définie et non vide" "non-vide" \
    "$([ -n "${_PLAN_SIZE_FORMAT_SPEC:-}" ] && echo non-vide || echo vide)"
assert_contains "S13-bis: le spec porte le titre que \$_PLAN_SIZE_HEADING_RE apparie" \
    '## Taille estimée' "${_PLAN_SIZE_FORMAT_SPEC:-}"
assert_contains "S13-bis: et la ligne de total que \$_PLAN_SIZE_TOTAL_RE extrait" \
    'Total estimé : 215 lignes' "${_PLAN_SIZE_FORMAT_SPEC:-}"
assert_contains "S13-bis: la règle du groomeur interpole le spec" \
    "${_PLAN_SIZE_FORMAT_SPEC:-ABSENT}" "${_PLAN_SIZE_RULE:-}"
# Les deux sites d'interpolation, et PAS un de plus : la règle, et le finding
# synthétique. Un troisième texte qui re-taperait le format est la régression
# que ce scan refuse — et on l'y route vers la constante, on n'ajoute pas
# d'exception (doctrine mika#2201).
assert_eq "S13-bis: exactement deux sites interpolent \$_PLAN_SIZE_FORMAT_SPEC" "2" \
    "$(grep -v '^[[:space:]]*#' "$DISPATCH_LIB" | grep -cF '${_PLAN_SIZE_FORMAT_SPEC}' || true)"
assert_contains "S13-bis: le finding synthétique du rattrapage est l'un des deux" \
    '${_PLAN_SIZE_FORMAT_SPEC}' \
    "$(sed -n '/^_plan_size_retry_if_section_still_missing() {/,/^}/p' "$DISPATCH_LIB")"
# Contre-vacuité : le scan voit bien la constante, et il la voit DÉFINIE une fois.
assert_eq "S13-bis contre-vacuité: la constante a un site de définition unique" "1" \
    "$(grep -cE '^_PLAN_SIZE_FORMAT_SPEC=' "$DISPATCH_LIB" || true)"

T2636_PROMPT_GROOM=$(_t2636_inject "dev-groom")
T2636_PROMPT_PILOT=$(_t2636_inject "dev-pilot")
assert_contains "S13: la règle est injectée dans le PROMPT pour SKILL=dev-groom" \
    '## Taille estimée' "$T2636_PROMPT_GROOM"
assert_not_contains "S13: la règle n'est PAS injectée pour SKILL=dev-pilot" \
    'RÈGLE DE GROOMING (mika#2636)' "$T2636_PROMPT_PILOT"
assert_contains "S13: mais dev-pilot reçoit bien la règle inconditionnelle voisine" \
    "RÈGLE DE DISPATCH (mika#2211)" "$T2636_PROMPT_PILOT"
# Contrat mika#138, invariant de position 2 : si la première ligne cesse d'être
# `<repo>#<num>`, la regex ancrée manque, le dispatch tombe en mode free-text
# et AUCUN worktree n'est créé. Régression de premier ordre.
assert_eq "S13: la première ligne du PROMPT reste exactement mika#2636" "mika#2636" \
    "$(printf '%s' "$T2636_PROMPT_GROOM" | head -1)"
assert_eq "S13: idem sur le chemin dev-pilot" "mika#2636" \
    "$(printf '%s' "$T2636_PROMPT_PILOT" | head -1)"
# LA RÉCENCE EST LE SEUL LEVIER d'une règle en fin d'un prompt de 16 KiB : la
# règle de taille doit être la DERNIÈRE que lit un groomeur.
T2636_LN_FD=$(printf '%s\n' "$T2636_PROMPT_GROOM" | grep -n 'RÈGLE DE GROOMING (mika#2306)' | head -1 | cut -d: -f1)
T2636_LN_PS=$(printf '%s\n' "$T2636_PROMPT_GROOM" | grep -n 'RÈGLE DE GROOMING (mika#2636)' | head -1 | cut -d: -f1)
assert_eq "S13: la règle de taille est la DERNIÈRE règle du prompt du groomeur" "yes" \
    "$( [ -n "$T2636_LN_FD" ] && [ -n "$T2636_LN_PS" ] && [ "$T2636_LN_PS" -gt "$T2636_LN_FD" ] && echo yes || echo no)"
assert_eq "S13: et l'injection source suit celle de mika#2306" "yes" \
    "$(_t2178_after '"$PROMPT" "$_PLAN_SIZE_RULE")' '"$PROMPT" "$_FIRE_DISPOSITION_RULE")')"
# La conditionnalité se lit par APPARTENANCE AU BLOC, jamais par un préfixe.
# La première rédaction scannait « la garde apparaît-elle AVANT l'injection ? » :
# un préfixe, donc vert même en déplaçant l'injection d'une ligne APRÈS le `fi`
# — c'est-à-dire sur la régression exacte que S13 existe pour attraper (vérifié
# par mutation : verdict `yes`, ligne précédente devenue `fi`). Et la moitié
# comportementale ne peut pas la voir non plus, `_t2636_inject` étant un miroir
# qui porte la condition en dur. Forme reprise du bloc mika#2548 déjà en place
# plus haut dans ce fichier.
T2636_GROOM_BLOCK=$(printf '%s\n' "$T2636_SUW_SRC" \
    | sed -n '/if \[ "\$SKILL" = "dev-groom" \]; then/,/^        fi/p')
assert_eq "S13 contre-vacuité: le bloc gardé sur le skill a bien été extrait" "non-vide" \
    "$([ -n "$T2636_GROOM_BLOCK" ] && echo non-vide || echo vide)"
assert_contains "S13: l'injection vit DANS la branche gardée sur le skill" \
    '_PLAN_SIZE_RULE' "$T2636_GROOM_BLOCK"
assert_contains "S13: et son aînée mika#2306 y vit aussi" \
    '_FIRE_DISPOSITION_RULE' "$T2636_GROOM_BLOCK"

# --- S14 : le seuil vit DEUX fois, et c'est le test qui tient R4 -----------
#
# Le jugement vit chez l'architecte, dont le prompt est un `system_prompt.md`
# STATIQUE (aucune interpolation au dispatch). Le seuil vit donc littéral dans
# les deux prompts architecte et en variable dans dispatch-lib. Doublon assumé,
# du même type que `_FD_HEADING_RE` / `AC_HEADING_RE`, et tenu par le même
# moyen. Sans ce test, baisser le défaut shell laisserait les prompts architecte
# sur 1 000 et la divergence serait MUETTE.
#
# Conséquence écrite plutôt que découverte : un opérateur qui baisse
# `PLAN_SIZE_MAX_LOC` change ce que le groomeur VISE, pas ce que l'architecte
# REFUSE.

T2636_SHELL_DEFAULT=$(_t2636_field 1 "${T2636_THRESHOLD_UNSET}|x|x")
T2636_GATE_FIRST=""
T2636_GATE_SECOND=""
for t2636_prompt in mika-arch-groom-ticket mika-arch-second-review; do
    t2636_pf="$SCRIPT_DIR/../$t2636_prompt/system_prompt.md"
    if [ -r "$t2636_pf" ]; then
        t2636_gate=$(sed -n '/^### Plan-Size Gate/,/^### /p' "$t2636_pf")
        case "$t2636_prompt" in
            mika-arch-groom-ticket)   T2636_GATE_FIRST="$t2636_gate" ;;
            mika-arch-second-review)  T2636_GATE_SECOND="$t2636_gate" ;;
        esac
        assert_eq "S14: $t2636_prompt porte un Plan-Size Gate" "non-vide" \
            "$([ -n "$t2636_gate" ] && echo non-vide || echo vide)"
        assert_contains "S14: $t2636_prompt — le gate nomme la section exacte" \
            '`## Taille estimée`' "$t2636_gate"
        assert_contains "S14: $t2636_prompt — le gate dit qu'un titre numéroté est la section" \
            '`## 6. Taille estimée`' "$t2636_gate"
        assert_contains "S14: $t2636_prompt — le gate dit qu'un exemple clôturé n'est pas la section" \
            'fenced' "$t2636_gate"
        t2636_lit=$(sed -nE 's/^\*\*Threshold:\*\* ([0-9]+) lines.*/\1/p' "$t2636_pf" | head -1)
        assert_eq "S14: $t2636_prompt — le littéral de seuil égale le défaut shell" \
            "$T2636_SHELL_DEFAULT" "$t2636_lit"
    else
        assert_eq "S14: $t2636_prompt/system_prompt.md lisible" "lisible" \
            "fichier introuvable: $t2636_pf"
    fi
done
# Les verdicts sont ceux qui existent déjà : aucun mode d'échec neuf dans la
# boucle. Première passe ITERATE, seconde passe ESCALATE (sans recours). Les
# deux blocs viennent de la boucle ci-dessus, donc ces trois assertions sont
# sous la même garde de lisibilité qu'elle — les ré-extraire ici les en sortait.
assert_contains "S14: la première passe refuse en ITERATE" "ITERATE" "$T2636_GATE_FIRST"
assert_contains "S14: la seconde passe refuse en ESCALATE" "ESCALATE" "$T2636_GATE_SECOND"
assert_not_contains "S14: et la seconde passe n'offre PAS d'ITERATE (limite des deux passes)" \
    "return \`ITERATE\`" "$T2636_GATE_SECOND"

# --- S15 : les deux rattrapages COMPOSENT, dans l'ordre figé ---------------
#
# Si les findings de première passe réclament à la fois `Fire-Disposition` et
# `Taille estimée`, les deux rattrapages tirent dans la même invocation de
# `_launch_revise_pilot`, à la suite, chacun avec son propre compteur. Coût :
# jusqu'à deux pilotes de revise supplémentaires sur un plan qui ignore deux
# sections. Acceptable (chacun est borné à un, et l'alternative est un ESCALATE
# sans recours), et NOMMÉ plutôt que découvert. L'ordre est arbitraire et figé
# ici pour que la composition soit reproductible.

T2636_S15=$(_t2636_revise_probe "$T2636_FINDINGS_BOTH" "no" "never")
assert_eq "S15: findings réclamant les DEUX sections ⇒ trois invocations du pilote" \
    "3" "$(_t2636_field 2 "$T2636_S15")"
assert_contains "S15: le rattrapage Fire-Disposition a tiré" \
    "fire_disposition_revise_retried" "$T2636_S15"
assert_contains "S15: le rattrapage de taille a tiré" \
    "plan_size_revise_retried" "$T2636_S15"
assert_eq "S15: et l'ordre est Fire-Disposition PUIS Taille estimée" "yes" \
    "$(_t2178_after '_plan_size_retry_if_section_still_missing "$findings_file" "$plan_path"' \
                    '_fd_retry_if_section_still_missing "$findings_file" "$plan_path"')"

# --- S16 : CE PLAN est le fixture, et c'est délibéré -----------------------
#
# Son §3 documente le format dans un bloc clôturé qui contient un titre
# `## Taille estimée` ET une ligne `Total estimé : 395 lignes` — l'exemple, pas
# la mesure. Un lecteur naïf qui prend le premier match rapporterait 395 pour un
# plan qui en annonce 530 : un nombre plausible, présenté avec autorité, FAUX.
# Pire, le terme 2 du rattrapage apparierait le titre cité et conclurait que la
# section est présente — un plan qui DOCUMENTE le format sans le remplir
# passerait la garde.
#
# Les deux lecteurs strippent donc les blocs clôturés d'abord, exactement comme
# `auto_pull::is_groomed` le fait pour les trois prédicats de callout
# (mika#2120) : une ligne légitimement citée à l'intérieur d'un fence n'est pas
# une déclaration.
#
# Ce plan est le SEUL fichier de l'arbre à porter le format à la fois en exemple
# clôturé et en mesure réelle, donc le seul qui distingue un lecteur qui strippe
# d'un lecteur qui ne strippe pas. Il est en plus AUTO-NETTOYANT : si un futur
# éditeur retire l'exemple du §3, l'assertion de contre-vacuité rougit et nomme
# ce qu'elle a perdu. VU ROUGE sans le strip : rend 395.

if [ -r "$T2636_PLAN" ]; then
    # Contre-vacuité : le fixture porte bien les DEUX formes. Sans ça, un plan
    # dont l'exemple a disparu rendrait 530 pour la mauvaise raison. Le contenu
    # DES fences est extrait une fois (le complément de ce que le lecteur
    # strippe), puis compté deux fois.
    T2636_FENCED=$(awk '/^[[:space:]]*```/ { inf = 1 - inf; next } inf' "$T2636_PLAN")
    assert_eq "S16 contre-vacuité: le fixture porte le titre dans un bloc clôturé" "1" \
        "$(printf '%s\n' "$T2636_FENCED" | grep -cE '^##[[:space:]]+Taille estimée' || true)"
    assert_eq "S16 contre-vacuité: le fixture porte un total d'exemple dans ce bloc" "1" \
        "$(printf '%s\n' "$T2636_FENCED" | grep -cE '^Total estimé' || true)"
    assert_eq "S16: le lecteur de total rend 530, jamais 395" "530" \
        "$( (source "$DISPATCH_LIB" 2>/dev/null || true; _plan_size_total_loc "$T2636_PLAN") )"
    assert_eq "S16: et la section est lue PRÉSENTE (titre numéroté, hors fence)" "oui" \
        "$( (source "$DISPATCH_LIB" 2>/dev/null || true
             if _plan_size_section_present "$T2636_PLAN"; then printf 'oui'; else printf 'non'; fi) )"
else
    assert_eq "S16: le plan de ce ticket est lisible" "lisible" "fichier introuvable: $T2636_PLAN"
fi

# Un plan qui ne porte le titre QUE dans un fence est lu « section absente ».
T2636_FENCED_ONLY='# Plan

## Format prescrit

```markdown
## Taille estimée

Total estimé : 395 lignes
```

## Livrables

L1 — rien.'
assert_eq "S16: un titre uniquement dans un fence ⇒ section ABSENTE" "non" \
    "$(_t2636_present_probe "$T2636_FENCED_ONLY")"
assert_eq "S16: et le total y est 'absent', jamais 395" "absent" \
    "$(_t2636_total_probe "$T2636_FENCED_ONLY")"
# Et le rattrapage tire bien sur ce plan-là : il DOCUMENTE le format sans le
# remplir, donc l'architecte a raison de l'avoir réclamé.
T2636_S16B=$(_t2636_revise_probe "$T2636_FINDINGS_WITH_SIZE" "fenced" "never")
assert_eq "S16: un plan qui documente le format sans le remplir ⇒ relance" \
    "2" "$(_t2636_field 2 "$T2636_S16B")"
# Fence NON TERMINÉ : rien n'est strippé, le corps entier est évalué. Le sens de
# l'arbitrage est celui de mika#2120 — un faux positif coûte une relance de
# revise, un faux négatif a coûté quinze heures de boucle.
assert_eq "S16: un fence non terminé ne strippe RIEN (le corps entier est évalué)" "395" \
    "$(_t2636_total_probe '# Plan

## Taille estimée

```markdown
Total estimé : 395 lignes')"

# Un fence en TILDES est une clôture Markdown valide, donc un plan qui
# documente le format en `~~~markdown` le documente sans le remplir, exactement
# comme en accents graves. Mesuré avant le correctif : la même entrée rendait
# `section=présente` et `total=395`, donc le rattrapage ne tirait pas ET
# l'exemple 395 entrait dans la distribution de calibration.
T2636_TILDE_ONLY='# Plan

## Format prescrit

~~~markdown
## Taille estimée

Total estimé : 395 lignes
~~~

## Livrables

L1 — rien.'
assert_eq "S16: un fence en TILDES est strippé comme un fence en backticks" "non" \
    "$(_t2636_present_probe "$T2636_TILDE_ONLY")"
assert_eq "S16: et son total d'exemple n'entre pas dans la mesure" "absent" \
    "$(_t2636_total_probe "$T2636_TILDE_ONLY")"

# --- S17 : le lecteur ne se tait pas sur un GROS plan (régression SIGPIPE) ---
#
# Mesuré : `_plan_size_strip_fences … | grep -qiE …` faisait sortir `grep` au
# premier match, le producteur prenait SIGPIPE, `pipefail` promouvait le 141, et
# la fonction rendait « section ABSENTE » pour un plan qui la porte — rc=141 sur
# un plan de 320 Ko, déterministe dès ~48 Ko de queue après le titre. Aucune
# fixture du bloc ne pouvait le voir : toutes tiennent dans un tampon de pipe.
# C'est le décile supérieur de la distribution que ce ticket existe pour
# mesurer, donc la population la plus coûteuse à perdre.
_t2636_big_plan_probe() {
    local t2636_dir t2636_f t2636_i t2636_out
    t2636_dir=$(mktemp -d)
    t2636_f="$t2636_dir/big.md"
    {
        printf '# Gros plan\n\n## 6. Taille estimée\n\nTotal estimé : 530 lignes\n\n## Suite\n\n'
        t2636_i=0
        while [ "$t2636_i" -lt 4000 ]; do
            printf 'Ligne de remplissage %s, pour depasser le tampon de pipe du noyau.\n' "$t2636_i"
            t2636_i=$(( t2636_i + 1 ))
        done
    } > "$t2636_f"
    t2636_out=$(
        # shellcheck disable=SC1090
        set -o pipefail
        source "$DISPATCH_LIB" 2>/dev/null || true
        printf '%s|' "$(_plan_size_total_loc "$t2636_f")"
        if _plan_size_section_present "$t2636_f"; then printf 'oui'; else printf 'non'; fi
    )
    rm -rf "$t2636_dir"
    printf '%s' "$t2636_out"
}
T2636_S17=$(_t2636_big_plan_probe)
assert_eq "S17: un plan de ~320 Ko rend son total, jamais 'absent'" "530" "${T2636_S17%%|*}"
assert_eq "S17: et sa section est lue PRÉSENTE sous pipefail" "oui" "${T2636_S17##*|}"

# --- S18 : le total est cherché DANS la section, pas dans tout le document ---
#
# La règle injectée ORDONNE, au-dessus du seuil, de découper en phases et de
# renvoyer la suite à un ticket de suivi ; le gate architecte fait passer ce
# plan en jugeant le périmètre de CETTE PR. Un plan conforme porte donc un
# second `Total estimé :` hors section. Lu sur tout le document avec « dernier
# match gagne », le lecteur rapportait le total différé : mesuré, 850 en section
# et 1900 hors périmètre rendaient 1900, donc `verdict=over_threshold` sur un
# périmètre de 850.
assert_eq "S18: un total différé hors section ne gagne pas" "850" \
    "$(_t2636_total_probe '# Plan

## 6. Taille estimée

| livrable | lignes |
|---|---|
| `a.rs` | 850 |

Total estimé : 850 lignes

## 7. Hors périmètre, délibérément

La phase 2 est renvoyée à un ticket de suivi.

Total estimé : 1900 lignes')"
# DANS la section, le dernier match gagne toujours — un sous-total par livrable
# peut précéder le total qui conclut.
assert_eq "S18: mais dans la section, le dernier match gagne toujours" "900" \
    "$(_t2636_total_probe '# Plan

## Taille estimée

Total estimé : 300 lignes (livrable 1)

Total estimé : 900 lignes

## Autre chose

rien')"

# --- Fire-Disposition, option (a) : les trois tables d'exceptions sont VIDES -
#
# Vacuité assertée À L'EXÉCUTION (modèle T2306_ARCH_ASK_ALLOWLIST) : le test
# rougit donc LE JOUR où une exception est ajoutée, pas seulement quand elle
# devient stale. Une table vide assertée vide est aussi ce qui distingue
# « aucune violation » de « le scan ne regarde rien ». Quand l'un de ces scans
# tire, on ARME le site ; on n'ajoute pas de ligne ici (doctrine mika#2201).
T2636_SOURCE_SCAN_ALLOWLIST=()
assert_eq "mika#2636: table d'exceptions des trois scans de source (S8, S10, S11) — zero entries" \
    "0" "${#T2636_SOURCE_SCAN_ALLOWLIST[@]}"

# ===========================================================================
# mika#2641 — une réponse d'architecte sans disposition lisible est RELANCÉE
#
# Le défaut mesuré le 2026-10-02 (groom de mika#2617) : la 2e passe rend un
# préambule seul, `_parse_verdict` rend une chaîne vide, et le bras `*)` du
# `case` l'escalade au MÊME titre qu'un `Verdict: ESCALATE` explicite —
# `Outcome: ESCALATE`, terminal, gel du re-dispatch par mika#2545.
#
# La première passe, elle, relance depuis mika#1823. Ce bloc porte le même
# patron sur les DEUX sites de seconde passe et épingle les deux moitiés :
# l'illisible relance, l'explicite reste terminal.
#
# Les sondes font tourner le VRAI `_iterate_groom_loop` avec `_arch_ask` stubbé
# au bord de processus (gabarit `_groom_signature_probe_1772`) : les parseurs,
# l'enveloppe de retry, le trail et les états terminaux restent réels. Un grep
# structurel passerait au-dessus des deux.
# ===========================================================================

echo ""
echo "Test: une 2e passe illisible est relancée (mika#2641)"
echo "------------------------------------------------------------"

# Le verbatim du ticket. Vérifié contre les cinq tiers de `_parse_verdict` :
# ni `escalate`/`cannot approve`/`human review needed`/`fundamental issues
# remain`, ni `groomed`/`approved`/`plan is ready`/`ship it`/`no remaining
# concerns` — le texte est en français, donc le fuzzy ne matche rien et le
# verdict est VIDE. Sans cette vérification la fixture rejouerait une autre
# classe que celle qu'elle nomme.
M2641_UNREADABLE='Je relis le plan révisé mika#2617 pour second passe. Analyse en cours des contrats de sortie et de la résolution des 12 rectifications.'

# $1 = chemin  : "ready" (1re passe READY) | "iterate" (1re passe ITERATE + revise)
# $2 = réponse de la 2e passe        (appel architecte n°2)
# $3 = réponse de la relance         (appel architecte n°3)
# $4 = optionnel : le mode d'altération, sous la forme `<mode><n>` où `<n>` est
#      le NUMÉRO D'APPEL architecte visé — `transport2`, `empty2` (contrôles
#      négatifs de V5), `transport3`, `empty3` (les chemins d'échec de la
#      RELANCE). Clé sur le numéro d'appel et non imbriqué sous `2)` : la
#      première version ne pouvait faire échouer que l'appel n°2, donc deux
#      chemins de sortie neufs — le WARN d'échec transport de la relance, et
#      `_groom_warn_empty_content "second-pass retry"`, seul consommateur de la
#      valeur de relance de `_pass_label2` — n'étaient exercés par aucun test.
_groom_second_pass_probe_2641() {
    local probe_path="$1" reply2="$2" reply3="$3" mode="${4:-normal}"
    local tmp wt rc
    tmp=$(mktemp -d)
    wt="$tmp/wt"
    mkdir -p "$wt/docs/plans"
    {
        echo "# Plan rejouant la signature mika#2641"
        echo "**Ticket:** mika issue#2641"
        for i in $(seq 1 14); do
            echo "Ligne de corps $i — rembourrage rembourrage rembourrage."
        done
    } > "$wt/docs/plans/2026-10-02-002-fix-2641-signature-plan.md"

    (
        # shellcheck disable=SC1090
        source "$DISPATCH_LIB" 2>/dev/null || true

        # Le retry transport est désarmé : son délai nominal est de 30 s, et
        # V5 n'interroge pas l'enveloppe transport — elle interroge le fait
        # qu'un échec transport ne produit PAS d'unreadable.
        export MIKA_ARCH_ASK_RETRY=0

        ARCH_CALLS="$tmp/arch-calls"
        printf '0' > "$ARCH_CALLS"

        M2641_FIRST="Disposition: READY"
        if [ "$probe_path" = "iterate" ]; then
            M2641_FIRST="F1: un constat de première passe.

Disposition: ITERATE"
        fi

        _arch_ask() {
            local n body
            n=$(cat "$ARCH_CALLS")
            n=$((n + 1))
            printf '%s' "$n" > "$ARCH_CALLS"
            # Le payload de chaque appel est conservé : V4 mesure la taille du
            # prompt correctif (appel n°3) contre le seuil du manifeste.
            cp -- "$2" "$tmp/prompt-$n" 2>/dev/null || true
            # La session portée est un contrat (D6) : l'architecte doit voir son
            # propre préambule pour le compléter.
            printf '%s' "${3:-<none>}" > "$tmp/session-$n"
            # Le mode est consulté AVANT le choix du corps, et il porte le
            # numéro d'appel qu'il vise : c'est ce qui rend les chemins d'échec
            # de la relance (appel n°3) atteignables.
            case "$mode" in
                "transport$n") return 75 ;;
                "empty$n")     printf '{"content":"","metadata":{"session_id":"probe-session-2641"}}'; return 0 ;;
            esac
            case "$n" in
                1) body="$M2641_FIRST" ;;
                2) body="$reply2" ;;
                3) body="$reply3" ;;
                *) body="APPEL INATTENDU $n" ;;
            esac
            jq -n --arg c "$body" \
                '{content:$c, metadata:{session_id:"probe-session-2641"}}'
        }
        # Les vrais shellent vers `gh` / claude-pilot.
        _write_canonical_callout() { printf 'written\n' > "$tmp/callout"; return 0; }
        _launch_revise_pilot() { return 0; }

        WORKTREE_DIR="$wt" ISSUE_NUM="2641" REPO="mika" BRANCH="probe-2641"
        RESULT=""
        if _iterate_groom_loop >/dev/null 2>"$tmp/err"; then rc=0; else rc=1; fi

        printf 'rc=%s\n' "$rc"
        printf 'arch_calls=%s\n' "$(cat "$ARCH_CALLS")"
        printf 'reason=%s\n' "${GROOM_LOOP_FAILURE_REASON:-<none>}"
        printf 'callout=%s\n' "$([ -f "$tmp/callout" ] && echo yes || echo no)"
        # Champ 4 du trail, joint en une chaîne — la raison est écrite une seule
        # fois, au site d'origine (`_groom_signature_probe_1772`) : la recopier
        # ici ferait vivre une note de correction en deux exemplaires libres de
        # diverger (classe mika#2158).
        printf 'trail_outcomes=%s\n' \
            "$(cut -f4 "$wt/.claude/groom-verdict-trail.log" 2>/dev/null | paste -sd, - || true)"
        printf 'retry_prompt_bytes=%s\n' \
            "$(if [ -f "$tmp/prompt-3" ]; then wc -c < "$tmp/prompt-3" | tr -d ' '; else echo 0; fi)"
        printf 'retry_session=%s\n' "$(cat "$tmp/session-3" 2>/dev/null || echo '<no-call>')"
        printf 'verdict_lines=%s\n' \
            "$(printf '%s\n' "$RESULT" | grep -c '^Verdict:' || true)"
        printf 'halt_cause_lines=%s\n' \
            "$(printf '%s\n' "$RESULT" | grep -c '^Groom-halt-cause:' || true)"
        printf 'outcome_escalate_lines=%s\n' \
            "$(printf '%s\n' "$RESULT" | grep -c '^Outcome: ESCALATE' || true)"
        printf 'terminal_marker_lines=%s\n' \
            "$(printf '%s\n' "$RESULT" | grep -c '^GROOM ESCALATED (terminal):' || true)"
        # Le fichier de constats préservé : combien de sections de relance il
        # porte (0 = seule la dernière réponse a été écrite).
        printf 'findings_retry_sections=%s\n' \
            "$(cat "$wt"/.iterate/escalate-*.md 2>/dev/null | grep -c '^--- retry attempt' || true)"
        printf 'RESULT_BEGIN\n%s\nRESULT_END\n' "$RESULT"
        sed 's/^/stderr: /' "$tmp/err" 2>/dev/null || true
    )
    rm -rf "$tmp"
}

# --- V1 (AC1/AC4) : le verbatim relance, et la relance aboutit --------------

M2641_RECOVERED=$(_groom_second_pass_probe_2641 ready "$M2641_UNREADABLE" 'A1: « Ligne de corps 3 — rembourrage rembourrage rembourrage. »

Verdict: GROOMED') || M2641_RECOVERED=""

# Ancre d'anti-vacuité, en PREMIER (V9) : sans elle chaque assertion ci-dessous
# passerait sur une chaîne vide si la sonde cessait de tourner.
assert_contains "mika#2641 (V1): la sonde a mené la boucle jusqu'au 3e appel architecte" \
    "arch_calls=3" "$M2641_RECOVERED"
assert_contains "mika#2641 (V1): le verbatim CONVERGE au lieu d'escalader" \
    "rc=0" "$M2641_RECOVERED"
assert_contains "mika#2641 (V1): aucune ligne de disposition ESCALATE n'est posée" \
    "outcome_escalate_lines=0" "$M2641_RECOVERED"
assert_contains "mika#2641 (V1): le callout canonique est écrit sur convergence" \
    "callout=yes" "$M2641_RECOVERED"
assert_contains "mika#2641 (V1): un run convergé n'enregistre aucun motif d'échec" \
    "reason=<none>" "$M2641_RECOVERED"
# Le trail est ce qui distingue « convergé » de « convergé GRÂCE à la relance ».
# Épinglé comme séquence entière : une boucle qui n'aurait jamais eu besoin d'un
# second essai lirait `READY,GROOMED` et satisferait tout test par jeton.
assert_contains "mika#2641 (V1): le trail porte UNPARSED puis le verdict de relance" \
    "trail_outcomes=READY,UNPARSED,GROOMED-after-retry" "$M2641_RECOVERED"
# D6 — la session est portée : l'architecte doit voir son propre préambule.
assert_contains "mika#2641 (V1): la relance porte la session de la 2e passe" \
    "retry_session=probe-session-2641" "$M2641_RECOVERED"

# --- V2 (AC3) : contrôle positif — un ESCALATE explicite reste terminal -----

M2641_EXPLICIT=$(_groom_second_pass_probe_2641 ready 'F1: le plan ne porte pas de contrat de sortie.
F2: la section de vérification est absente.

Verdict: ESCALATE' 'JAMAIS ATTEINT') || M2641_EXPLICIT=""

assert_contains "mika#2641 (V2): un verdict explicite ne déclenche AUCUNE relance" \
    "arch_calls=2" "$M2641_EXPLICIT"
assert_contains "mika#2641 (V2): un ESCALATE explicite reste terminal" \
    "rc=1" "$M2641_EXPLICIT"
# Le RESULT de ce chemin est inchangé à l'octet près — c'est AC3.
assert_contains "mika#2641 (V2): le marqueur terminal est inchangé" \
    "GROOM ESCALATED (terminal): mika-arch escalated at second-pass-after-ready." \
    "$M2641_EXPLICIT"
assert_contains "mika#2641 (V2): la ligne Verdict: est TOUJOURS posée sur un refus réel" \
    "Verdict: ESCALATE — human review required." "$M2641_EXPLICIT"
assert_contains "mika#2641 (V2): un refus réel ne porte PAS de motif architect_unreadable" \
    "halt_cause_lines=0" "$M2641_EXPLICIT"
assert_contains "mika#2641 (V2): le motif d'échec d'un refus réel est inchangé" \
    "reason=architect refused on second pass after a READY first pass" "$M2641_EXPLICIT"
assert_contains "mika#2641 (V2): sans relance, le fichier de constats n'a qu'une tentative" \
    "findings_retry_sections=0" "$M2641_EXPLICIT"

# --- V2-bis : la relance rend un ESCALATE explicite -------------------------
#
# Le refus est réel, donc cause `verdict` (ligne Verdict:, aucun motif) — et la
# 1ʳᵉ tentative, la plus riche, reste dans le fichier de constats au lieu d'être
# écrasée par la réponse au prompt correctif.
M2641_RETRY_ESC=$(_groom_second_pass_probe_2641 ready "$M2641_UNREADABLE" 'F1: le plan ne porte pas de contrat de sortie.

Verdict: ESCALATE') || M2641_RETRY_ESC=""

assert_contains "mika#2641 (V2-bis): la sonde a mené la boucle jusqu'au 3e appel" \
    "arch_calls=3" "$M2641_RETRY_ESC"
assert_contains "mika#2641 (V2-bis): un refus obtenu à la relance reste terminal" \
    "rc=1" "$M2641_RETRY_ESC"
assert_contains "mika#2641 (V2-bis): le trail porte UNPARSED puis ESCALATE-after-retry" \
    "trail_outcomes=READY,UNPARSED,ESCALATE-after-retry" "$M2641_RETRY_ESC"
assert_contains "mika#2641 (V2-bis): la ligne Verdict: est posée (refus réel)" \
    "verdict_lines=1" "$M2641_RETRY_ESC"
assert_contains "mika#2641 (V2-bis): aucun motif architect_unreadable" \
    "halt_cause_lines=0" "$M2641_RETRY_ESC"
assert_contains "mika#2641 (V2-bis): le fichier de constats garde les deux tentatives" \
    "findings_retry_sections=1" "$M2641_RETRY_ESC"

# --- V10 (AC1) : une ITERATE explicite n'est PAS un illisible ---------------
#
# AC1 : « ni READY, ni ITERATE, ni ESCALATE explicite ». `_parse_verdict` n'a
# pas de bras ITERATE, donc sans terme dédié une objection lisible serait
# relancée puis étiquetée « absence de signal » — l'inverse du défaut fondateur.
M2641_ITER_EXPL=$(_groom_second_pass_probe_2641 ready 'F1: le contrat de sortie manque encore.

Disposition: ITERATE' 'JAMAIS ATTEINT') || M2641_ITER_EXPL=""

assert_contains "mika#2641 (V10): une ITERATE explicite ne déclenche AUCUNE relance" \
    "arch_calls=2" "$M2641_ITER_EXPL"
assert_contains "mika#2641 (V10): elle reste terminale" \
    "rc=1" "$M2641_ITER_EXPL"
assert_contains "mika#2641 (V10): cause verdict — la ligne Verdict: est posée" \
    "verdict_lines=1" "$M2641_ITER_EXPL"
assert_contains "mika#2641 (V10): aucun motif architect_unreadable" \
    "halt_cause_lines=0" "$M2641_ITER_EXPL"
assert_contains "mika#2641 (V10): le motif d'échec est celui d'un refus, comme avant" \
    "reason=architect refused on second pass after a READY first pass" "$M2641_ITER_EXPL"

M2641_ITER_EXPL_I=$(_groom_second_pass_probe_2641 iterate '**Verdict:** ITERATE' 'JAMAIS ATTEINT') \
    || M2641_ITER_EXPL_I=""
assert_contains "mika#2641 (V10): after-iterate — aucune relance sur une ITERATE explicite" \
    "arch_calls=2" "$M2641_ITER_EXPL_I"
assert_contains "mika#2641 (V10): after-iterate — cause verdict, aucun motif" \
    "halt_cause_lines=0" "$M2641_ITER_EXPL_I"
assert_contains "mika#2641 (V10): after-iterate — le motif d'échec est inchangé" \
    "reason=architect refused on second pass after an ITERATE revise" "$M2641_ITER_EXPL_I"

# Contrôle négatif de l'ANCRAGE : un préambule qui CITE la disposition de 1ʳᵉ
# passe en prose n'est pas une ligne de disposition — il garde sa relance.
M2641_ITER_PROSE=$(_groom_second_pass_probe_2641 ready 'Ma première passe rendait Disposition: ITERATE ; je relis le plan révisé.' 'A1: « Ligne de corps 3 — rembourrage rembourrage rembourrage. »

Verdict: GROOMED') || M2641_ITER_PROSE=""
assert_contains "mika#2641 (V10): une ITERATE citée en prose est relancée" \
    "arch_calls=3" "$M2641_ITER_PROSE"
assert_contains "mika#2641 (V10): et la relance converge" \
    "rc=0" "$M2641_ITER_PROSE"

# --- V3 (AC2) : deux illisibles d'affilée → ESCALATE à motif distinct -------

M2641_DOUBLE=$(_groom_second_pass_probe_2641 ready "$M2641_UNREADABLE" "$M2641_UNREADABLE") \
    || M2641_DOUBLE=""

assert_contains "mika#2641 (V3): l'enveloppe de relance est bornée à deux tentatives" \
    "arch_calls=3" "$M2641_DOUBLE"
assert_contains "mika#2641 (V3): un double illisible est une non-convergence" \
    "rc=1" "$M2641_DOUBLE"
assert_contains "mika#2641 (V3): le trail enregistre les deux tentatives" \
    "trail_outcomes=READY,UNPARSED,UNPARSED-after-retry" "$M2641_DOUBLE"
# D2 — `Outcome: ESCALATE` RESTE la ligne de disposition : `_measure_cycle_output`
# P4 n'énumère que quatre valeurs, et un mot hors liste ferait tomber le cycle
# en `empty`, c'est-à-dire un FAUX ROUGE sur un run qui a produit une décision.
assert_contains "mika#2641 (V3): la ligne de disposition reste Outcome: ESCALATE" \
    "outcome_escalate_lines=1" "$M2641_DOUBLE"
assert_contains "mika#2641 (V3): le marqueur terminal est posé" \
    "terminal_marker_lines=1" "$M2641_DOUBLE"
# Le cœur d'AC2 : « un motif distinct, ET NON UN VERDICT ». La ligne
# `Verdict: ESCALATE — human review required` affirmait un verdict que
# l'architecte n'a jamais rendu.
assert_contains "mika#2641 (V3): AUCUNE ligne Verdict: n'est affirmée" \
    "verdict_lines=0" "$M2641_DOUBLE"
assert_contains "mika#2641 (V3): le motif architect_unreadable est posé, ancré" \
    "halt_cause_lines=1" "$M2641_DOUBLE"
assert_contains "mika#2641 (V3): le motif est nommé dans le RESULT" \
    "Groom-halt-cause: architect_unreadable" "$M2641_DOUBLE"
# R-f — le motif voyage dans `tasks.result` et c'est TOUT ce que l'opérateur
# voit (PR#2028). Dire « l'architecte a refusé » l'envoie lire une objection qui
# n'existe pas : c'est la classe mika#1772 exactement.
M2641_DOUBLE_REASON=$(printf '%s\n' "$M2641_DOUBLE" | grep '^reason=' || true)
assert_contains "mika#2641 (V3): le motif nomme l'absence de ligne de verdict" \
    "no parsable verdict line" "$M2641_DOUBLE_REASON"
assert_contains "mika#2641 (V3): le motif dit que les deux tentatives ont manqué" \
    "2 attempts" "$M2641_DOUBLE_REASON"
assert_not_contains "mika#2641 (V3): le motif ne dit PAS que l'architecte a refusé" \
    "refused" "$M2641_DOUBLE_REASON"

# --- V6 : le MÊME traitement sur le site after-iterate ----------------------
#
# Deux `case` distincts, donc un seul corrigé se lit exactement comme deux.

M2641_ITER=$(_groom_second_pass_probe_2641 iterate "$M2641_UNREADABLE" 'A1: « Ligne de corps 5 — rembourrage rembourrage rembourrage. »

Verdict: GROOMED') || M2641_ITER=""

assert_contains "mika#2641 (V6): le chemin after-iterate atteint lui aussi 3 appels" \
    "arch_calls=3" "$M2641_ITER"
assert_contains "mika#2641 (V6): le chemin after-iterate converge sur relance" \
    "rc=0" "$M2641_ITER"
assert_contains "mika#2641 (V6): le trail after-iterate porte la relance" \
    "trail_outcomes=ITERATE,UNPARSED,GROOMED-after-retry" "$M2641_ITER"

M2641_ITER_DOUBLE=$(_groom_second_pass_probe_2641 iterate "$M2641_UNREADABLE" "$M2641_UNREADABLE") \
    || M2641_ITER_DOUBLE=""

assert_contains "mika#2641 (V6): un double illisible after-iterate est borné" \
    "arch_calls=3" "$M2641_ITER_DOUBLE"
assert_contains "mika#2641 (V6): after-iterate pose le motif et pas de Verdict:" \
    "halt_cause_lines=1" "$M2641_ITER_DOUBLE"
assert_contains "mika#2641 (V6): after-iterate n'affirme aucun verdict" \
    "verdict_lines=0" "$M2641_ITER_DOUBLE"
assert_contains "mika#2641 (V6): le stage after-iterate est nommé dans le RESULT" \
    "second-pass-after-iterate" "$M2641_ITER_DOUBLE"

# --- V5 : contrôles NÉGATIFS — transport et `.content` vide ne sont PAS -----
#         des unreadable (D5)
#
# « N'a pas répondu » et « a répondu sans disposition » restent deux
# populations. Les deux replient vers le rejeu, mais par deux chemins
# différents, et fondre le second dans le premier effacerait un diagnostic qui
# a coûté trois tentatives et quatre tickets à établir (mika#2296).

M2641_TRANSPORT=$(_groom_second_pass_probe_2641 ready 'inatteignable' 'inatteignable' transport2) \
    || M2641_TRANSPORT=""

assert_contains "mika#2641 (V5): un échec transport ne déclenche aucune relance d'illisible" \
    "arch_calls=2" "$M2641_TRANSPORT"
assert_contains "mika#2641 (V5): un échec transport reste une non-convergence" \
    "rc=1" "$M2641_TRANSPORT"
assert_contains "mika#2641 (V5): un échec transport ne pose AUCUN motif unreadable" \
    "halt_cause_lines=0" "$M2641_TRANSPORT"
assert_contains "mika#2641 (V5): un échec transport n'escalade pas" \
    "outcome_escalate_lines=0" "$M2641_TRANSPORT"
M2641_TRANSPORT_REASON=$(printf '%s\n' "$M2641_TRANSPORT" | grep '^reason=' || true)
assert_contains "mika#2641 (V5): le motif transport est inchangé" \
    "_arch_ask failed" "$M2641_TRANSPORT_REASON"

M2641_EMPTY=$(_groom_second_pass_probe_2641 ready 'inatteignable' 'inatteignable' empty2) \
    || M2641_EMPTY=""

assert_contains "mika#2641 (V5): un .content vide ne déclenche aucune relance d'illisible" \
    "arch_calls=2" "$M2641_EMPTY"
assert_contains "mika#2641 (V5): un .content vide reste une non-convergence" \
    "rc=1" "$M2641_EMPTY"
assert_contains "mika#2641 (V5): un .content vide ne pose AUCUN motif unreadable" \
    "halt_cause_lines=0" "$M2641_EMPTY"
M2641_EMPTY_REASON=$(printf '%s\n' "$M2641_EMPTY" | grep '^reason=' || true)
assert_contains "mika#2641 (V5): le motif budget (mika#2296) est inchangé" \
    "EMPTY .content" "$M2641_EMPTY_REASON"

# --- V5-bis : les deux chemins d'échec de la RELANCE elle-même --------------
#
# Le scan R5 de mika#2278 compte les six WARN d'échec DANS LA SOURCE ; sans ces
# deux cas, rien n'assertait qu'ils FIRENT. Et `_pass_label2` a une valeur de
# relance dont le second cas est le seul consommateur.

M2641_RETRY_TRANSPORT=$(_groom_second_pass_probe_2641 ready "$M2641_UNREADABLE" 'inatteignable' transport3) \
    || M2641_RETRY_TRANSPORT=""

assert_contains "mika#2641 (V5-bis): la relance est bien tentée avant d'échouer" \
    "arch_calls=3" "$M2641_RETRY_TRANSPORT"
assert_contains "mika#2641 (V5-bis): un transport mort sur la relance reste une non-convergence" \
    "rc=1" "$M2641_RETRY_TRANSPORT"
assert_contains "mika#2641 (V5-bis): et ne pose AUCUN motif unreadable (le transport n'en est pas un)" \
    "halt_cause_lines=0" "$M2641_RETRY_TRANSPORT"
M2641_RETRY_TRANSPORT_REASON=$(printf '%s\n' "$M2641_RETRY_TRANSPORT" | grep '^reason=' || true)
assert_contains "mika#2641 (V5-bis): le WARN de la relance se nomme comme tel" \
    "second-pass retry _arch_ask failed" "$M2641_RETRY_TRANSPORT_REASON"

M2641_RETRY_EMPTY=$(_groom_second_pass_probe_2641 ready "$M2641_UNREADABLE" 'inatteignable' empty3) \
    || M2641_RETRY_EMPTY=""

assert_contains "mika#2641 (V5-bis): un .content vide sur la relance est tenté puis refusé" \
    "arch_calls=3" "$M2641_RETRY_EMPTY"
assert_contains "mika#2641 (V5-bis): et reste une non-convergence sans motif unreadable" \
    "halt_cause_lines=0" "$M2641_RETRY_EMPTY"
M2641_RETRY_EMPTY_REASON=$(printf '%s\n' "$M2641_RETRY_EMPTY" | grep '^reason=' || true)
# Seul consommateur de la valeur de relance de `_pass_label2`.
assert_contains "mika#2641 (V5-bis): le motif nomme la RELANCE, pas la 2e passe" \
    "second-pass retry returned an EMPTY .content" "$M2641_RETRY_EMPTY_REASON"

# --- V4 (R5/D7) : le prompt de relance est COURT, et le seuil est LU --------
#
# `mika-arch-second-review` déclare `review_anchor_min_brief_chars` : en dessous,
# la garde d'ancrage mika#2037 ne s'arme pas. Un prompt de relance au-dessus
# l'armerait — et l'architecte, qui n'a pas le brief sous les yeux dans un
# re-prompt, ne peut produire aucune ancre ; le moteur retire alors la
# disposition (tier 0) ou réécrit la réponse en escalade (tier 0b), et
# `_parse_verdict` rend ESCALATE. Le correctif produirait exactement le terminal
# qu'il existe pour empêcher.
#
# Le seuil est lu DEPUIS LE MANIFESTE, jamais recopié : un seuil durci dans le
# `skill.toml` sans que le prompt rétrécisse doit faire rougir ce test.
M2641_ANCHOR_MANIFEST="$SCRIPT_DIR/../mika-arch-second-review/skill.toml"
M2641_ANCHOR_THRESHOLD=$(grep -E '^[[:space:]]*review_anchor_min_brief_chars[[:space:]]*=' \
    "$M2641_ANCHOR_MANIFEST" 2>/dev/null | head -1 | sed 's/.*=[[:space:]]*//' | tr -d ' \r' || true)

if [ -z "$M2641_ANCHOR_THRESHOLD" ]; then
    echo "SKIP: mika#2641 (V4) — review_anchor_min_brief_chars absent de $M2641_ANCHOR_MANIFEST"
    SKIPPED=$((SKIPPED + 1))
else
    M2641_RETRY_BYTES=$(printf '%s\n' "$M2641_RECOVERED" | grep '^retry_prompt_bytes=' \
        | head -1 | cut -d= -f2 || true)
    # Anti-vacuité : un prompt de 0 octet satisferait l'inégalité en ne
    # mesurant rien (la sonde n'aurait pas tourné, ou le fichier aurait disparu).
    if [ "${M2641_RETRY_BYTES:-0}" -gt 0 ] 2>/dev/null; then
        PASS=$((PASS + 1))
        echo "  ✓ mika#2641 (V4): le prompt de relance a été mesuré (${M2641_RETRY_BYTES} octets)"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ mika#2641 (V4): le prompt de relance n'a PAS été mesuré (vide ou sonde muette)"
    fi
    if [ "${M2641_RETRY_BYTES:-0}" -lt "$M2641_ANCHOR_THRESHOLD" ] 2>/dev/null; then
        PASS=$((PASS + 1))
        echo "  ✓ mika#2641 (V4): le prompt de relance (${M2641_RETRY_BYTES}) est sous le seuil d'ancrage (${M2641_ANCHOR_THRESHOLD})"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ mika#2641 (V4): le prompt de relance (${M2641_RETRY_BYTES}) ATTEINT le seuil d'ancrage (${M2641_ANCHOR_THRESHOLD}) — il armerait la garde mika#2037"
    fi
fi

# --- S-c : scan de cardinalité des appels à `_escalate_groom` ---------------
#
# Chaque site doit passer une cause EXPLICITE. Un appel qui oublie l'argument
# tomberait dans le défaut `verdict` et produirait un `RESULT` plausible — donc
# aucun test comportemental ne peut voir cette classe.
#
# La cardinalité est ASSERTÉE, pas seulement la couverture : un scan qui
# compterait sans borner passerait en ne regardant rien le jour où le nom de la
# fonction change (classe mika#2205, même raison qui a fait asserter
# `cardinality == 3` dans le scan de mika#2496).
#
# Il y a DEUX assertions de cardinalité à 5, et leur redondance est délibérée :
# celle de la section ESCALATE lit le corps de `_iterate_groom_loop` par
# `declare -f`, celle-ci lit le FICHIER ENTIER. Elles peuvent donc divergir — un
# appel placé hors de la boucle rougit celle-ci et pas l'autre, un appel écrit
# sans guillemets l'inverse — et c'est exactement pourquoi les deux restent.
_m2641_escalate_call_sites() {
    # Les APPELS, jamais la définition ni la prose : ligne dont le premier mot
    # est `_escalate_groom` suivi d'un espace et d'un argument.
    grep -nE '^[[:space:]]*_escalate_groom[[:space:]]+"' "$DISPATCH_LIB" || true
}
M2641_CALL_LINES=$(_m2641_escalate_call_sites)
M2641_CALL_COUNT=$(printf '%s' "$M2641_CALL_LINES" | grep -c . || true)
assert_eq "mika#2641 (S-c): exactement cinq sites appellent _escalate_groom" \
    "5" "$M2641_CALL_COUNT"
# Chaque site nomme sa cause PAR LA CONSTANTE, jamais par un littéral recopié.
# C'est le prédicat plus fort des deux : il attrape à la fois le site qui omet
# l'argument ET celui qui retape la valeur de fil à la main — or le format de
# fil n'est exact que tant qu'il a un site de définition unique (D4).
M2641_CAUSED_COUNT=$(printf '%s\n' "$M2641_CALL_LINES" \
    | grep -cE '"\$GROOM_HALT_CAUSE_(VERDICT|ARCHITECT_UNREADABLE)"[[:space:]]*$' || true)
assert_eq "mika#2641 (S-c): les cinq sites passent une cause explicite, par la constante" \
    "5" "$M2641_CAUSED_COUNT"

# Le format de fil a un site de définition unique de chaque côté (D4). Côté
# shell : une affectation, pas deux.
M2641_WIRE_DEFS=$(grep -cE '^GROOM_HALT_CAUSE_ARCHITECT_UNREADABLE=' "$DISPATCH_LIB" || true)
assert_eq "mika#2641 (D4): le motif de fil a une définition shell unique" \
    "1" "$M2641_WIRE_DEFS"
M2641_PREFIX_DEFS=$(grep -cE '^GROOM_HALT_CAUSE_LINE_PREFIX=' "$DISPATCH_LIB" || true)
assert_eq "mika#2641 (D4): le préfixe de ligne a une définition shell unique" \
    "1" "$M2641_PREFIX_DEFS"

# --- La prémisse qui rend la population de tier 0 VIDE, épinglée ------------
#
# `_parse_verdict` tier 0 (`Disposition-Withheld: REVIEW-ANCHOR-MISSING`) rend
# RIEN, donc il atteindrait le chemin `architect_unreadable` et la relance
# re-demanderait un verdict avec un prompt trop court pour armer la garde
# d'ancrage mika#2037 — un contournement.
#
# Ce que ça ne peut pas faire aujourd'hui : son propre commentaire dit que
# depuis mika#2338 tier 0 est le REPLI, et que tier 0b (qui rend ESCALATE, donc
# ne passe PAS par la relance) tire dès que le skill déclare un `ESCALATE` de la
# famille de la ligne retirée. Le manifeste livré le déclare, donc la population
# de tier 0 sur la seconde passe est vide.
#
# C'est une PRÉMISSE, pas une garantie du prédicat : si quelqu'un retire cette
# déclaration, le contournement s'ouvre. Cette assertion la rend fausse
# BRUYAMMENT plutôt qu'en silence.
M2641_SUFFIX_DECL=$(grep -cE '^[[:space:]]*required_suffix_lines[[:space:]]*=.*Verdict: ESCALATE' \
    "$M2641_ANCHOR_MANIFEST" 2>/dev/null || true)
assert_eq "mika#2641: le manifeste déclare Verdict: ESCALATE (tier 0b tire, tier 0 reste vide)" \
    "1" "$M2641_SUFFIX_DECL"

# R-b : une SEULE définition du prompt correctif de seconde passe. Deux copies
# divergeraient, et c'est la leçon que `grooming_marker.rs` a dû engraver une
# fois (mika#2158) : un prédicat recopié prend du retard sans que rien ne rougisse.
M2641_HELPER_DEFS=$(grep -cE '^_second_pass_retry_prompt\(\)' "$DISPATCH_LIB" || true)
assert_eq "mika#2641 (R-b): le prompt correctif a une définition unique" \
    "1" "$M2641_HELPER_DEFS"
# Les appels, jamais la définition : `$(_second_pass_retry_prompt)` en
# substitution de commande. La forme réelle est `local x; x=$(...)` sur une
# ligne, donc le prédicat porte sur la substitution et non sur le début de ligne.
M2641_HELPER_CALLS=$(grep -cE '\$\(_second_pass_retry_prompt\)' "$DISPATCH_LIB" || true)
assert_eq "mika#2641 (R-b): les deux sites de seconde passe l'appellent" \
    "2" "$M2641_HELPER_CALLS"

# --- dispatch-lib parse toujours -------------------------------------------
T2545_RC=0
bash -n "$DISPATCH_LIB" 2>/dev/null || T2545_RC=$?
assert_eq "dispatch-lib.sh passe bash -n" "0" "$T2545_RC"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
