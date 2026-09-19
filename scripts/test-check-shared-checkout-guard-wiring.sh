#!/usr/bin/env bash
#
# Anti-vacuité de scripts/check-shared-checkout-guard-wiring.sh (mika#2107).
#
# « Supprime la chose que le test protège ; confirme que le test rougit. »
#
# Un guard que personne n'a vu rougir est une décoration — mika#2103 est
# l'incident où un lint est resté vert à travers 26 paniques de production parce
# qu'il ne connaissait qu'une orthographe du défaut. Le check sous test lit du
# JSON à travers des expressions jq dont chaque `// []` est une occasion de
# rendre vrai sur rien : une expression cassée passerait sur un settings.json
# vide aussi bien que sur le bon, et personne ne le saurait.
#
# Chaque cas dégrade UNE propriété et exige le rouge. Le contrôle de bonne foi
# (l'arborescence complète doit être verte) est ce qui distingue « le check
# détecte » de « le check refuse tout ».

set -uo pipefail

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
CHECK="$REPO_ROOT/scripts/check-shared-checkout-guard-wiring.sh"

PASS=0
FAIL=0

assert_exit() {
	local root=$1 want=$2 name=$3
	local got=0
	bash "$CHECK" "$root" >/dev/null 2>&1 || got=$?
	if [ "$got" -eq "$want" ]; then
		printf 'PASS: %s (exit %d)\n' "$name" "$got"
		PASS=$((PASS + 1))
	else
		printf 'FAIL: %s (attendu exit %d, obtenu %d)\n' "$name" "$want" "$got" >&2
		FAIL=$((FAIL + 1))
	fi
}

assert_output_contains() {
	local root=$1 needle=$2 name=$3
	local out
	out=$(bash "$CHECK" "$root" 2>&1 || true)
	case $out in
	*"$needle"*)
		printf 'PASS: %s\n' "$name"
		PASS=$((PASS + 1))
		;;
	*)
		printf 'FAIL: %s (la sortie ne contient pas: %s)\n' "$name" "$needle" >&2
		FAIL=$((FAIL + 1))
		;;
	esac
}

TMPROOT=$(mktemp -d)
trap 'rm -rf -- "$TMPROOT"' EXIT

# Une arborescence conforme, servant de base à chaque dégradation.
make_fixture() {
	local name=$1
	local root="$TMPROOT/$name"
	mkdir -p "$root/scripts" "$root/.claude"
	printf '#!/usr/bin/env bash\nexit 0\n' >"$root/scripts/guard-shared-checkout"
	printf '#!/usr/bin/env bash\nexit 0\n' >"$root/scripts/test-guard-shared-checkout.sh"
	chmod +x "$root/scripts/guard-shared-checkout" "$root/scripts/test-guard-shared-checkout.sh"
	cat >"$root/.claude/settings.json" <<'JSON'
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/scripts/guard-shared-checkout\"" }
        ]
      }
    ],
    "SessionStart": [
      {
        "hooks": [
          { "type": "command", "command": "bash \"$CLAUDE_PROJECT_DIR/scripts/guard-shared-checkout\"" }
        ]
      }
    ]
  }
}
JSON
	git -C "$root" init -q -b main
	git -C "$root" config user.email guard@test
	git -C "$root" config user.name guard
	git -C "$root" add -A
	printf '%s' "$root"
}

# --- Contrôle de bonne foi ---------------------------------------------------
GOOD=$(make_fixture good)
assert_exit "$GOOD" 0 "arborescence conforme: le check passe (sinon il refuse tout)"

# --- Le hook PreToolUse disparaît -------------------------------------------
NO_PRE=$(make_fixture no-pretooluse)
jq 'del(.hooks.PreToolUse)' "$NO_PRE/.claude/settings.json" >"$NO_PRE/.claude/tmp.json"
mv "$NO_PRE/.claude/tmp.json" "$NO_PRE/.claude/settings.json"
assert_exit "$NO_PRE" 1 "PreToolUse retiré: rouge"
assert_output_contains "$NO_PRE" "PreToolUse(Bash) n'invoque plus" "PreToolUse retiré: le message nomme le hook"

# --- Le matcher cesse de couvrir Bash ---------------------------------------
BAD_MATCHER=$(make_fixture bad-matcher)
jq '.hooks.PreToolUse[0].matcher = "Write"' "$BAD_MATCHER/.claude/settings.json" >"$BAD_MATCHER/.claude/tmp.json"
mv "$BAD_MATCHER/.claude/tmp.json" "$BAD_MATCHER/.claude/settings.json"
assert_exit "$BAD_MATCHER" 1 "matcher ne couvrant plus Bash: rouge"

# --- La commande pointe ailleurs --------------------------------------------
BAD_CMD=$(make_fixture bad-command)
jq '.hooks.PreToolUse[0].hooks[0].command = "bash /usr/bin/true"' \
	"$BAD_CMD/.claude/settings.json" >"$BAD_CMD/.claude/tmp.json"
mv "$BAD_CMD/.claude/tmp.json" "$BAD_CMD/.claude/settings.json"
assert_exit "$BAD_CMD" 1 "commande pointant ailleurs que la garde: rouge"

# --- La preuve d'armement disparaît -----------------------------------------
# Le cas le plus sournois: la garde marche encore, mais plus rien ne distingue
# « rien à refuser » de « rien n'est armé ».
NO_ARM=$(make_fixture no-sessionstart)
jq 'del(.hooks.SessionStart)' "$NO_ARM/.claude/settings.json" >"$NO_ARM/.claude/tmp.json"
mv "$NO_ARM/.claude/tmp.json" "$NO_ARM/.claude/settings.json"
assert_exit "$NO_ARM" 1 "SessionStart retiré: rouge"
assert_output_contains "$NO_ARM" "désarmement silencieux" "SessionStart retiré: le message dit ce qu'on perd"

# --- Collision de clé avec le settings.local.json des worktrees --------------
WITH_PERMS=$(make_fixture with-permissions)
jq '.permissions = {"allow":["Bash(git:*)"]}' \
	"$WITH_PERMS/.claude/settings.json" >"$WITH_PERMS/.claude/tmp.json"
mv "$WITH_PERMS/.claude/tmp.json" "$WITH_PERMS/.claude/settings.json"
assert_exit "$WITH_PERMS" 1 "clé 'permissions' ajoutée: rouge (collision avec le settings.local.json)"

# --- JSON invalide : ignoré en silence par le harnais ------------------------
BROKEN=$(make_fixture broken-json)
printf '{ "hooks": { oops\n' >"$BROKEN/.claude/settings.json"
assert_exit "$BROKEN" 1 "settings.json invalide: rouge (sinon la garde disparaît sans un mot)"

# --- Le settings.json disparaît ---------------------------------------------
NO_SETTINGS=$(make_fixture no-settings)
rm -f "$NO_SETTINGS/.claude/settings.json"
assert_exit "$NO_SETTINGS" 1 "settings.json absent: rouge"

# --- Le settings.json existe mais n'est pas suivi ----------------------------
# Un fichier non suivi n'arrive pas avec le checkout — c'est TOUTE la propriété
# que ce câblage achète (taux d'installation de .githooks mesuré à zéro).
UNTRACKED=$(make_fixture untracked-settings)
git -C "$UNTRACKED" rm -q --cached .claude/settings.json >/dev/null 2>&1
assert_exit "$UNTRACKED" 1 "settings.json non suivi par git: rouge"

# --- Le script perd son bit exécutable ---------------------------------------
NOT_EXEC=$(make_fixture not-executable)
chmod -x "$NOT_EXEC/scripts/guard-shared-checkout"
git -C "$NOT_EXEC" update-index --chmod=-x scripts/guard-shared-checkout >/dev/null 2>&1
assert_exit "$NOT_EXEC" 1 "garde non exécutable: rouge"

# --- Le script disparaît -----------------------------------------------------
NO_GUARD=$(make_fixture no-guard)
rm -f "$NO_GUARD/scripts/guard-shared-checkout"
assert_exit "$NO_GUARD" 1 "garde absente: rouge"

# --- La surface s'élargit sans perdre la garde -------------------------------
# Les cas les plus sournois de cette famille: tout ce qui est asserté par
# présence reste vrai, et le fichier fait pourtant exécuter autre chose sur
# chaque machine qui clone le dépôt.
EXTRA_HOOK=$(make_fixture extra-hook)
jq '.hooks.PostToolUse = [{"matcher":"Bash","hooks":[{"type":"command","command":"bash /tmp/whatever"}]}]' \
	"$EXTRA_HOOK/.claude/settings.json" >"$EXTRA_HOOK/.claude/tmp.json"
mv "$EXTRA_HOOK/.claude/tmp.json" "$EXTRA_HOOK/.claude/settings.json"
assert_exit "$EXTRA_HOOK" 1 "évènement de hook supplémentaire: rouge (la garde est intacte, la surface a grandi)"

EXTRA_CMD=$(make_fixture extra-command)
jq '.hooks.PreToolUse[0].hooks += [{"type":"command","command":"bash /tmp/also-this"}]' \
	"$EXTRA_CMD/.claude/settings.json" >"$EXTRA_CMD/.claude/tmp.json"
mv "$EXTRA_CMD/.claude/tmp.json" "$EXTRA_CMD/.claude/settings.json"
assert_exit "$EXTRA_CMD" 1 "commande supplémentaire dans l'entrée de la garde: rouge"

EXTRA_KEY=$(make_fixture extra-toplevel-key)
jq '.env = {"MIKA_GUARD_SHARED_CHECKOUT":"0"}' \
	"$EXTRA_KEY/.claude/settings.json" >"$EXTRA_KEY/.claude/tmp.json"
mv "$EXTRA_KEY/.claude/tmp.json" "$EXTRA_KEY/.claude/settings.json"
assert_exit "$EXTRA_KEY" 1 "clé de premier niveau supplémentaire: rouge (ici, un désarmement par env)"

# --- Le harnais comportemental disparaît -------------------------------------
NO_HARNESS=$(make_fixture no-harness)
rm -f "$NO_HARNESS/scripts/test-guard-shared-checkout.sh"
assert_exit "$NO_HARNESS" 1 "harnais comportemental absent: rouge"

printf '\n%s\n' "----------------------------------------"
printf 'test-check-shared-checkout-guard-wiring: %d PASS, %d FAIL\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
exit 0
