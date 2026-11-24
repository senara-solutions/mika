#!/bin/bash
# Test suite for _check_pilot_force_push (mika#1318).
#
# Tests the post-flight pilot push guard that detects scope-of-authority
# violations when a dev-groom pilot pushes to the remote.
#
# mika#3128 — la garde lit TROIS issues de chaque côté, pas deux. Le
# 2026-11-01, sur #3004, l'opérateur a fermé une draft et supprimé la branche
# distante 66 secondes après le lancement d'un groom ; la garde a lu
# `post_remote=<none>`, son terme unique d'égalité était faux, et le dispatch
# est rendu `STRUCTURAL VIOLATION: pilot push detected`. Le pilote n'avait rien
# poussé, et le site d'appel court-circuite `_iterate_groom_loop` et
# `_push_branch` : un vol de groom entier perdu sur un geste humain.
#
# Les CONTRÔLES NÉGATIFS de cette suite portent autant que les cas positifs —
# ils sont ce qui sépare « la garde distingue » de « la garde est désarmée ».
# Trois d'entre eux ne doivent jamais changer de verdict : une ref qui AVANCE
# reste une violation (cas 2), une branche APPARUE reste une violation (cas 6),
# et une branche absente des deux côtés reste propre (cas 5).
#
# Source isolation audit: dispatch-lib.sh has no top-level imperative code —
# all `set -e`, `trap`, and env var references are inside function bodies.
# Safe to source directly without a guard variable.
#
# Run: bash skills/bundled/_shared/tests/test_force_push_guard.sh
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

# Here-string, jamais `printf … | grep -q` : sous `pipefail`, le `grep -q` sort
# au premier match et le producteur meurt sur SIGPIPE, ce qui fait rougir
# l'assertion qui vient de RÉUSSIR (mika#2055, et `scripts/verify-no-sigpipe-grep.sh`
# refuse la forme en pipeline).
assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected to contain: '$needle'"
        echo "    actual:              '$haystack'"
    fi
}

assert_return() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label (exit=$actual)"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected exit: $expected"
        echo "    actual exit:   $actual"
    fi
}

# Override git to simulate ls-remote responses without network access.
#
# TROIS issues et non deux, et c'est le seul changement structurel du harnais
# pour mika#3128 : le rc de `ls-remote --exit-code` est CONTRACTUEL
# (`git ls-remote --help` : « Exit with status "2" when no matching refs are
# found in the remote repository »), donc le mock doit rendre `2` pour
# « absente » et `128` pour « le distant n'a pas répondu ». Avant ce vol, tout
# `MOCK_LS_REMOTE_EXIT` non nul était un échec indifférencié et l'absence se
# simulait par un `stdout` vide avec `rc=0` — la confusion même que la garde
# faisait.
#
#   MOCK_LS_REMOTE_EXIT=0   + MOCK_LS_REMOTE_OUTPUT="<sha>\trefs/heads/<br>"  → PRÉSENTE
#   MOCK_LS_REMOTE_EXIT=2                                                     → ABSENTE
#   MOCK_LS_REMOTE_EXIT=128                                                   → INDÉTERMINÉE
#
# Un rc non nul n'imprime RIEN, et ce n'est pas une économie : `git` n'écrit
# aucune ref sur un `--exit-code` qui échoue, donc un mock qui fournirait un
# SHA avec `rc=2` laisserait passer un code qui lit le `stdout` sans regarder
# le rc — exactement le défaut corrigé.
MOCK_LS_REMOTE_OUTPUT=""
MOCK_LS_REMOTE_EXIT=0
git() {
    # Intercept: git -C <dir> ls-remote [--exit-code] origin refs/heads/<branch>
    local is_ls_remote=0
    for arg in "$@"; do
        if [ "$arg" = "ls-remote" ]; then
            is_ls_remote=1
            break
        fi
    done
    if [ "$is_ls_remote" = "1" ]; then
        if [ "$MOCK_LS_REMOTE_EXIT" -ne 0 ]; then
            return "$MOCK_LS_REMOTE_EXIT"
        fi
        printf '%s' "$MOCK_LS_REMOTE_OUTPUT"
        return 0
    fi
    # Fall through to real git for anything else
    command git "$@"
}

# Toute l'evidence produite par la suite, accumulée à CHAQUE appel de la garde.
# C'est le support de l'assertion d'absence de `post_remote=<none>` (mika#3128) :
# accumuler dans le lanceur plutôt que cas par cas est ce qui empêche un cas
# ajouté plus tard d'échapper à l'assertion en silence.
ALL_EVIDENCE=""

# Lance la garde en conservant ses effets de bord (`RESULT`,
# `PUSH_VIOLATION_DETECTED` sont des globaux : un `$( … )` les perdrait dans le
# sous-shell), puis la relance dans un sous-shell pour lire son `stderr`. La
# garde est idempotente sur des mocks, et la seconde lecture n'a aucun autre
# effet observable — ses écritures de globaux meurent avec le sous-shell.
GUARD_RC=0
GUARD_STDERR=""
run_guard() {
    GUARD_RC=0
    _check_pilot_force_push 2>/dev/null || GUARD_RC=$?
    ALL_EVIDENCE="${ALL_EVIDENCE}${PUSH_VIOLATION_EVIDENCE}
"
    GUARD_STDERR=$({ _check_pilot_force_push >/dev/null; } 2>&1) || true
}

# ============================================================================
# Test 1: dev-groom, no pilot push (clean)
# ============================================================================
echo ""
echo "Test: dev-groom, no pilot push — guard returns 0"
echo "---------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD="abc123"
PRE_RUN_REMOTE_STATE="present"
MOCK_LS_REMOTE_EXIT=0
MOCK_LS_REMOTE_OUTPUT="abc123	refs/heads/$BRANCH"
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "clean: returns 0" "0" "$GUARD_RC"
assert_eq "clean: no violation flag" "" "$PUSH_VIOLATION_DETECTED"

# ============================================================================
# Test 2: dev-groom, pilot pushed (violation)
#
# CONTRÔLE NÉGATIF de mika#3128, laissé mot pour mot : une ref distante qui
# AVANCE vers un autre SHA pendant le vol reste une violation. C'est ce cas qui
# sépare « la garde distingue une suppression d'un push » de « la garde a été
# désarmée ».
# ============================================================================
echo ""
echo "Test: dev-groom, pilot pushed — guard returns 1"
echo "---------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD="abc123"
PRE_RUN_REMOTE_STATE="present"
MOCK_LS_REMOTE_EXIT=0
MOCK_LS_REMOTE_OUTPUT="def456	refs/heads/$BRANCH"
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "violation: returns 1" "1" "$GUARD_RC"
assert_eq "violation: flag set" "1" "$PUSH_VIOLATION_DETECTED"
assert_eq "violation: evidence captured" "pre_remote=abc123 post_remote=def456" "$PUSH_VIOLATION_EVIDENCE"

# ============================================================================
# Test 3: dev-pilot — guard returns 0 regardless of remote state (R5)
# ============================================================================
echo ""
echo "Test: dev-pilot — guard early-returns 0 (R5)"
echo "---------------------------------------------------"

SKILL="dev-pilot"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD="abc123"
PRE_RUN_REMOTE_STATE="present"
MOCK_LS_REMOTE_EXIT=0
MOCK_LS_REMOTE_OUTPUT="def456	refs/heads/$BRANCH"
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "dev-pilot: returns 0" "0" "$GUARD_RC"
assert_eq "dev-pilot: no violation flag" "" "$PUSH_VIOLATION_DETECTED"

# ============================================================================
# Test 4: no worktree (free-text mode) — guard returns 0
# ============================================================================
echo ""
echo "Test: no worktree — guard returns 0"
echo "---------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR=""
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD=""
PRE_RUN_REMOTE_STATE=""
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "no-worktree: returns 0" "0" "$GUARD_RC"

# ============================================================================
# Test 5: branch not on remote pre-run, still not on remote post-run (clean)
#
# CONTRÔLE NÉGATIF : le vol nominal d'un premier groom. Adapté au mock à trois
# issues (`rc=2` au lieu d'un `stdout` vide), verdict attendu INCHANGÉ.
# ============================================================================
echo ""
echo "Test: branch absent from the remote on both sides — guard returns 0"
echo "-----------------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/new-branch"
PRE_RUN_REMOTE_HEAD=""
PRE_RUN_REMOTE_STATE="absent"
MOCK_LS_REMOTE_EXIT=2
MOCK_LS_REMOTE_OUTPUT=""
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "new-branch-clean: returns 0" "0" "$GUARD_RC"
assert_eq "new-branch-clean: no violation flag" "" "$PUSH_VIOLATION_DETECTED"
assert_eq "new-branch-clean: no Vanished-ref line — nothing vanished" "" "$RESULT"

# ============================================================================
# Test 6: branch not on remote pre-run, appeared post-run (pilot created it)
#
# CONTRÔLE NÉGATIF : `pre=absente` + `post=présente`, c'est le pilote qui a
# créé la branche, et ça reste une violation. L'evidence conserve son
# `pre_remote=<none>` — c'est le côté GAUCHE, et il désigne une vraie
# violation ; seul le `post_remote=<none>` était la phrase fausse de #3004.
# ============================================================================
echo ""
echo "Test: branch appeared on remote — violation detected"
echo "------------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/new-branch"
PRE_RUN_REMOTE_HEAD=""
PRE_RUN_REMOTE_STATE="absent"
MOCK_LS_REMOTE_EXIT=0
MOCK_LS_REMOTE_OUTPUT="def456	refs/heads/$BRANCH"
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "new-branch-violation: returns 1" "1" "$GUARD_RC"
assert_eq "new-branch-violation: flag set" "1" "$PUSH_VIOLATION_DETECTED"
assert_eq "new-branch-violation: evidence" "pre_remote=<none> post_remote=def456" "$PUSH_VIOLATION_EVIDENCE"

# ============================================================================
# Test 7: network failure on ls-remote — guard returns 0 (fail-open)
# ============================================================================
echo ""
echo "Test: post-flight network failure — guard returns 0 (fail-open)"
echo "------------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD="abc123"
PRE_RUN_REMOTE_STATE="present"
MOCK_LS_REMOTE_EXIT=128
MOCK_LS_REMOTE_OUTPUT=""
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "network-failure: returns 0" "0" "$GUARD_RC"
assert_eq "network-failure: no violation flag" "" "$PUSH_VIOLATION_DETECTED"

# ============================================================================
# Test 8: la ref distante a DISPARU pendant le vol — ce n'est pas un push
#
# Le cas de #3004, rejoué avec ses valeurs : la branche était sur `origin` à
# `8da087abd` au lancement, l'opérateur a fermé la draft et l'a supprimée
# 66 secondes plus tard, et le post-vol lit « absente ».
# ============================================================================
echo ""
echo "Test: the remote ref vanished mid-flight — not a pilot push (mika#3128)"
echo "------------------------------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/3004/dispatch-un-tour-pr-review-de-mika-dev"
PRE_RUN_REMOTE_HEAD="8da087abd"
PRE_RUN_REMOTE_STATE="present"
MOCK_LS_REMOTE_EXIT=2
MOCK_LS_REMOTE_OUTPUT=""
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "vanished-ref: returns 0" "0" "$GUARD_RC"
assert_eq "vanished-ref: no violation flag" "" "$PUSH_VIOLATION_DETECTED"
assert_eq "vanished-ref: no evidence captured" "" "$PUSH_VIOLATION_EVIDENCE"

# Le fait est NOMMÉ dans le `RESULT`, et c'est la surface portante : le stderr
# de cette fonction est le `Stdio::piped()` de `spawn_long_running_exec`, que
# l'exécuteur ne lit QUE dans `if !status.success()` — sur un dispatch qui
# réussit le tube est jeté sans être lu (Signal M, mesuré par mika#2050).
assert_contains "vanished-ref: le RESULT porte la ligne Vanished-ref:" \
    "Vanished-ref:" "$RESULT"
assert_contains "vanished-ref: la ligne NOMME la branche" \
    "origin/$BRANCH" "$RESULT"
assert_contains "vanished-ref: la ligne porte le SHA de lancement" \
    "8da087abd" "$RESULT"
assert_contains "vanished-ref: la ligne dit que ce n'est PAS attribué au pilote" \
    "NOT counted as a pilot push" "$RESULT"

# Et le `RESULT` ne porte AUCUNE des deux phrases que #3004 a produites. Ces
# deux-là ne sont pas écrites par la garde mais par son site d'appel, sur son
# `rc=1` : les asserter ici mesure la conséquence réelle du verdict rendu.
assert_eq "vanished-ref: le RESULT ne porte pas STRUCTURAL VIOLATION" "0" \
    "$(printf '%s' "$RESULT" | grep -cF 'STRUCTURAL VIOLATION' || true)"
assert_eq "vanished-ref: le RESULT ne porte pas PIPELINE_INCOMPLETE — push violation" "0" \
    "$(printf '%s' "$RESULT" | grep -cF 'PIPELINE_INCOMPLETE — push violation' || true)"

# Le marqueur `stderr`, convenance et non sonde (Signal M) : même préfixe que
# ses deux sœurs, troisième sous-événement de la famille.
assert_contains "vanished-ref: le marqueur stderr porte le préfixe maison" \
    "pilot_push_guard.branch_deleted" "$GUARD_STDERR"

# ============================================================================
# Test 9: le PRÉ-vol n'a pas été mesuré — la garde n'accuse personne
#
# La moitié symétrique du même défaut : `PRE_RUN_REMOTE_HEAD` était vide dans
# DEUX cas — la branche n'était pas sur le distant, et l'`ls-remote` de pré-vol
# n'a pas répondu — donc un échec réseau d'avant-vol faisait lire « la branche
# est apparue ⇒ violation » à une branche présente depuis le début. Règle
# maison mika#2277 : un signal qu'on ne peut pas lire n'est jamais un terme
# satisfait.
# ============================================================================
echo ""
echo "Test: the pre-flight read failed — no attribution (mika#3128)"
echo "---------------------------------------------------------------"

SKILL="dev-groom"
WORKTREE_DIR="/tmp/test-worktree"
BRANCH="fix/1318/test-branch"
PRE_RUN_REMOTE_HEAD=""
PRE_RUN_REMOTE_STATE="unread"
MOCK_LS_REMOTE_EXIT=0
MOCK_LS_REMOTE_OUTPUT="def456	refs/heads/$BRANCH"
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "pre-unread: returns 0" "0" "$GUARD_RC"
assert_eq "pre-unread: no violation flag" "" "$PUSH_VIOLATION_DETECTED"
assert_eq "pre-unread: no evidence captured" "" "$PUSH_VIOLATION_EVIDENCE"

# Un état jamais initialisé — le mode texte libre le pose vide — est traité
# comme « non mesurée » et non comme « absente ». Un état qu'on n'a pas posé
# n'accuse personne.
PRE_RUN_REMOTE_STATE=""
PUSH_VIOLATION_DETECTED=""
PUSH_VIOLATION_EVIDENCE=""
RESULT=""

run_guard
assert_return "pre-uninitialised: returns 0" "0" "$GUARD_RC"
assert_eq "pre-uninitialised: no violation flag" "" "$PUSH_VIOLATION_DETECTED"

# ============================================================================
# Test 10: `post_remote=<none>` ne se forme sur AUCUN cas de la suite
#
# C'est la forme exacte de l'evidence de #3004, et son absence structurelle est
# l'assertion qui dit que le défaut est fermé : le bras de violation n'est
# atteignable que sur un post-vol PRÉSENT, donc `post_remote` y porte toujours
# un SHA.
# ============================================================================
echo ""
echo "Test: post_remote=<none> cannot form on any case (mika#3128)"
echo "-------------------------------------------------------------"

# Anti-vacuité, et c'est une ASSERTION et non un commentaire : un accumulateur
# vide rendrait l'absence vraie en ne regardant rien (classe mika#2205).
assert_contains "absence: l'accumulateur a bien vu de l'evidence" \
    "post_remote=def456" "$ALL_EVIDENCE"

# Contrôle de bonne foi : le prédicat d'absence DOIT accuser la chaîne de
# #3004. Sans ce sens, « elle n'apparaît nulle part » est indistinguable d'un
# `grep` qui ne mord sur rien.
assert_eq "absence (contrôle négatif): le prédicat accuse bien la chaîne de #3004" "1" \
    "$(printf '%s' 'pre_remote=8da087abd post_remote=<none>' | grep -cF 'post_remote=<none>' || true)"

assert_eq "absence: post_remote=<none> sur aucun cas de la suite" "0" \
    "$(printf '%s' "$ALL_EVIDENCE" | grep -cF 'post_remote=<none>' || true)"

# Le côté GAUCHE, lui, reste — il désigne la branche créée par le pilote, qui
# est une vraie violation (cas 6). L'asserter est ce qui empêche de « fermer »
# le défaut en supprimant les deux formes.
assert_eq "absence: pre_remote=<none> reste, lui — c'est une vraie violation" "1" \
    "$(printf '%s' "$ALL_EVIDENCE" | grep -cF 'pre_remote=<none> post_remote=def456' || true)"

# Reset mock
MOCK_LS_REMOTE_EXIT=0

# ============================================================================
# Summary
# ============================================================================
echo ""
echo "=============================="
echo "Results: $PASS passed, $FAIL failed"
echo "=============================="

[ "$FAIL" -eq 0 ] || exit 1
