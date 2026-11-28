#!/bin/bash
# Boucle d'itération de grooming (mika#1271) — extrait de test-dispatch-lib.sh par mika#3178, vol 2.
# Structure de la branche quand l'issue est fermée, boucle `_iterate_groom_loop`,
# et le second-pass invoqué deux fois par branche (READY + ITERATE, chacune avec
# sa relance mika#2641).
#
# Plage d'origine : lignes 116–1095 de skills/bundled/_shared/test-dispatch-lib.sh
# à la base 890f3f5f. Corps et queue extraits par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) : mot pour mot par construction.
set -euo pipefail
_SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib-dispatch-lib-suite.sh
source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"

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
# Extract the call sequence. QUATRE termes depuis mika#3121 (V7), et la position
# du quatrième porte sa conséquence : une porte de lancement placée AVANT
# `_handle_dry_run` refuserait un dry-run (qui n'allait jamais partir) ; placée
# APRÈS `_run_claude_pilot`, elle ne refuserait rien — le tour de pilote serait
# déjà consommé. Le `awk` ne garde que le premier champ, parce que le quatrième
# site porte son `|| exit 1` sur la même ligne : c'est `RESULT` posé plus la
# sortie du processus qui font remonter la bannière au callback.
CALL_SEQUENCE=$(printf '%s\n' "$DISPATCH_BODY" | grep -E '^\s+_(set_up_worktree|detect_plan_on_branch|handle_dry_run|confront_launch_plan)\b' | tr -s ' ' | sed 's/^ //' | awk '{print $1}')
EXPECTED_SEQUENCE=$(printf '%s\n' "_set_up_worktree" "_detect_plan_on_branch" "_handle_dry_run" "_confront_launch_plan")
assert_eq "Call ordering: _set_up_worktree -> _detect_plan_on_branch -> _handle_dry_run -> _confront_launch_plan (mika#3121 V7)" "$EXPECTED_SEQUENCE" "$CALL_SEQUENCE"

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

# --- mika#3121 V8 : les deux globales, une fois chacune, dans la MÊME branche ---
#
# `_LAUNCHED_PLAN_PATH` ne vaut comme réponse à « un plan est-il servi ? » que si
# elle est posée là où la fonction réécrit `ENTRY_COMMAND`, et nulle part
# ailleurs : posée plus haut, elle armerait la porte sur un plan que la fonction
# va finalement refuser ; posée deux fois, l'une des deux poserait le mauvais
# chemin. Le compte est donc de UN, et le voisinage est l'assertion qui porte le
# reste — rien entre les deux affectations ne ferme ni ne détourne la branche.
LAUNCHED_SETS=$(printf '%s\n' "$PLAN_FUNC" | grep -v '^\s*#' | grep -c '^\s*_LAUNCHED_PLAN_PATH=' || true)
assert_eq "mika#3121 V8: _LAUNCHED_PLAN_PATH is set exactly once in _detect_plan_on_branch" "1" "$LAUNCHED_SETS"
SIZE_PATH_SETS=$(printf '%s\n' "$PLAN_FUNC" | grep -v '^\s*#' | grep -c '^\s*_PLAN_SIZE_PLAN_PATH=' || true)
assert_eq "mika#3121 V8: _PLAN_SIZE_PLAN_PATH is set exactly once in _detect_plan_on_branch" "1" "$SIZE_PATH_SETS"
# Entre les deux affectations, aucune fermeture ni dérivation de branche : la
# globale de la PORTE est posée dans la même branche que celle de la LIGNE DE
# TAILLE, qui est la branche où `ENTRY_COMMAND` est réécrit.
GLOBALS_SPAN=$(printf '%s\n' "$PLAN_FUNC" | sed -n '/^\s*_PLAN_SIZE_PLAN_PATH=/,/^\s*_LAUNCHED_PLAN_PATH=/p' | grep -v '^\s*#')
assert_not_contains "mika#3121 V8: no branch close between the two globals (fi)" 'fi' "$GLOBALS_SPAN"
assert_not_contains "mika#3121 V8: no branch fork between the two globals (else)" 'else' "$GLOBALS_SPAN"
assert_contains "mika#3121 V8: both globals sit in the ENTRY_COMMAND-override branch" '_LAUNCHED_PLAN_PATH=' "$GLOBALS_SPAN"

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

# Les trois entrées littérales des cas de tier 0 et 0b, définies AVANT la porte
# ci-dessous : la garde de dérive de marqueur (plus bas dans ce fichier) les lit,
# et elle tourne même quand ce bloc skippe. Les laisser à l'intérieur les rendrait
# indéfinies sous `set -u` le jour où le binaire manque — un SKIP qui casse un
# autre bloc n'est plus un SKIP.
WITHHELD_MARKER="Disposition-Withheld: REVIEW-ANCHOR-MISSING"
ENGINE_FINDING_MARKER="(BLOCKING) [mika-engine] review-anchor:"
ENGINE_LINE="F1: $ENGINE_FINDING_MARKER attestation withheld after the corrective re-prompt — anchors_found=3, anchors_valid=2, miss_reason=QuoteNotInBrief: paraphrase."

# mika#2194 (vol 5) : depuis la bascule, les ~30 cas ci-dessous appellent
# `_parse_disposition` / `_parse_verdict`, qui délèguent à `mika arch-disposition`.
# Sans le binaire, chaque enveloppe prend son chemin de refus et rend un jeton
# vide : les cas « aucun match » resteraient verts et tous les autres rougiraient,
# ce qui est exactement le genre de rouge qui ne dit pas sa cause. Le SKIP la dit.
# Forme reprise de la porte de grooming mika#2012/#2608 plus bas — le corps n'est
# pas réindenté, délibérément, pour que le diff de ce vol reste lisible.
if ! mika_bin_or_skip "mika#2194 formes littérales des deux lecteurs (~30 cas)"; then
    : # le SKIP est déjà compté et dit par le helper
else

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
# crates/mika-agent/src/agent_loop/mod.rs. `$WITHHELD_MARKER` est défini en tête
# de ce bloc, au-dessus de la porte du binaire.
#
# mika#2194 (vol 5) : le tier 0 vit désormais dans le moteur, et ces cas
# l'exercent à travers les enveloppes. Ce qu'ils gardent est inchangé — la
# POSITION du tier 0 dans l'ordre des cinq — et ils n'ont pas changé d'attente.
# ---------------------------------------------------------------------------

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
# `$ENGINE_FINDING_MARKER` et `$ENGINE_LINE` sont définis en tête de ce bloc, au-dessus
# de la porte du binaire — la garde de dérive les lit même quand ce bloc skippe.
#
# mika#2194 (vol 5) : le tier 0b a basculé dans le moteur comme les quatre autres,
# mais `_engine_escalation_line` RESTE dans le bash — elle a un second appelant de
# production, `_escalate_groom` (mika#2338), qui n'est pas un lecteur de disposition.
# C'est la seule des cinq fonctions du bloc d'origine que ce vol conserve.
# ---------------------------------------------------------------------------

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

fi # fin de la porte du binaire pour les formes littérales (mika#2194, vol 5)

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

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
