#!/usr/bin/env bash
# check-shared-checkout-guard-wiring.sh — mika#2107
#
# Test STRUCTUREL du câblage de la garde d'écriture git hors worktree.
#
# POURQUOI IL EXISTE À CÔTÉ DU TEST COMPORTEMENTAL. La régression qu'il attrape
# ne rendrait AUCUNE décision fausse : elle rendrait la garde ABSENTE. Les 94
# assertions de `test-guard-shared-checkout.sh` resteraient vertes pendant que
# plus aucune session ne chargerait le hook — exactement la classe que ce dépôt
# a payée trois fois (mika#2205, un scan silencieusement inactif ; mika#2327, un
# réglage code-owned jamais écrit ; mika#2340, une bibliothèque rafraîchie en
# apparence). Un test comportemental ne peut pas voir cette panne-là.

#
# Usage: check-shared-checkout-guard-wiring.sh [repo-root]
# La racine est un argument pour que l'anti-vacuité puisse l'exercer sur des
# arborescences dégradées — même motif que check-dispatch-seats-declared.sh.

set -uo pipefail

REPO_ROOT=${1:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)}
SETTINGS="$REPO_ROOT/.claude/settings.json"
GUARD_REL="scripts/guard-shared-checkout"
GUARD="$REPO_ROOT/$GUARD_REL"
HARNESS="$REPO_ROOT/scripts/test-guard-shared-checkout.sh"

FAIL=0

fail() {
	FAIL=1
	printf 'ÉCHEC: %s\n' "$1" >&2
	[ "$#" -gt 1 ] && printf '  Fix: %s\n' "$2" >&2
	return 0
}

pass() { printf 'ok   %s\n' "$1"; }

# --- 1. Le script de production existe et reste exécutable -------------------
if [ -f "$GUARD" ]; then
	pass "$GUARD_REL existe"
else
	fail "$GUARD_REL est absent" "restaurer le script; sans lui aucune session n'a de garde"
fi

if [ -x "$GUARD" ]; then
	pass "$GUARD_REL est exécutable sur le disque"
else
	fail "$GUARD_REL n'est pas exécutable" "chmod +x $GUARD_REL"
fi

# Le bit exécutable suivi par git : un chmod local ne survit pas au clone, donc
# c'est le mode DANS L'INDEX qui décide de ce que reçoit un worktree neuf.
if git -C "$REPO_ROOT" ls-files --stage -- "$GUARD_REL" 2>/dev/null | grep -q '^100755'; then
	pass "$GUARD_REL porte le mode 100755 dans l'index git"
else
	fail "$GUARD_REL n'est pas suivi en mode 100755" \
		"git update-index --chmod=+x $GUARD_REL (un clone neuf recevrait un fichier non exécutable)"
fi

if [ -f "$HARNESS" ]; then
	pass "scripts/test-guard-shared-checkout.sh existe"
else
	fail "scripts/test-guard-shared-checkout.sh est absent" \
		"restaurer le harnais; la garde ne serait plus éprouvée par la CI"
fi

# --- 2. Le settings.json suivi porte les deux hooks --------------------------
if [ -f "$SETTINGS" ]; then
	pass ".claude/settings.json existe"
else
	fail ".claude/settings.json est absent" \
		"le hook n'est plus déclaré: aucune session ne charge la garde"
	printf '\ncheck-shared-checkout-guard-wiring: ÉCHEC\n' >&2
	exit 1
fi

if ! git -C "$REPO_ROOT" ls-files --error-unmatch -- .claude/settings.json >/dev/null 2>&1; then
	fail ".claude/settings.json n'est pas suivi par git" \
		"git add .claude/settings.json — un fichier non suivi n'arrive pas avec le checkout, et c'est TOUTE la propriété que ce câblage achète"
else
	pass ".claude/settings.json est suivi par git"
fi

if jq -e . "$SETTINGS" >/dev/null 2>&1; then
	pass ".claude/settings.json est du JSON valide"
else
	fail ".claude/settings.json n'est pas du JSON valide" \
		"un settings.json invalide est ignoré en silence: la garde disparaît sans un mot"
	printf '\ncheck-shared-checkout-guard-wiring: ÉCHEC\n' >&2
	exit 1
fi

if jq -e --arg g "$GUARD_REL" '
  (.hooks.PreToolUse // [])
  | map(select((.matcher // "") | test("Bash")))
  | map(.hooks // []) | add // []
  | map(select((.command // "") | contains($g)))
  | length > 0
' "$SETTINGS" >/dev/null 2>&1; then
	pass "le hook PreToolUse(Bash) invoque $GUARD_REL"
else
	fail "le hook PreToolUse(Bash) n'invoque plus $GUARD_REL" \
		"restaurer l'entrée hooks.PreToolUse — sans elle la garde ne s'interpose plus devant aucune commande"
fi

if jq -e --arg g "$GUARD_REL" '
  (.hooks.SessionStart // [])
  | map(.hooks // []) | add // []
  | map(select((.command // "") | contains($g)))
  | length > 0
' "$SETTINGS" >/dev/null 2>&1; then
	pass "le hook SessionStart invoque $GUARD_REL (preuve d'armement)"
else
	fail "le hook SessionStart n'invoque plus $GUARD_REL" \
		"restaurer l'entrée hooks.SessionStart — sans la ligne d'armement, une garde cassée se lit exactement comme une garde qui n'a jamais eu à firer, et le fail-open devient un désarmement silencieux"
fi

# --- 3. Aucune collision de clé avec le settings.local.json des worktrees -----
# dispatch-lib.sh copie un settings.local.json qui ne porte que `permissions`.
# Si ce fichier-ci se mettait à porter `permissions`, les deux se disputeraient
# la même clé et l'ordre de fusion deviendrait une question ouverte.
if jq -e 'has("permissions")' "$SETTINGS" >/dev/null 2>&1; then
	fail ".claude/settings.json porte une clé 'permissions'" \
		"la retirer: le settings.local.json copié dans chaque worktree ne porte QUE 'permissions', et la disjonction des clés est ce qui rend la cohabitation sûre"
else
	pass ".claude/settings.json ne porte pas 'permissions' (clés disjointes du settings.local.json)"
fi

if [ "$FAIL" -ne 0 ]; then
	printf '\ncheck-shared-checkout-guard-wiring: ÉCHEC\n' >&2
	exit 1
fi
printf '\ncheck-shared-checkout-guard-wiring: ok\n'
exit 0
