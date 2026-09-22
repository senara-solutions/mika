#!/usr/bin/env bash
# test-shell-exec-guard.sh — mika#2449 U4.4
#
# Harnais du BRANCHEMENT de la garde du checkout principal dans le handler
# `run_shell` (crates/mika-agent/templates/skills/shell-exec/handlers/run.sh).
# `scripts/test-guard-shared-checkout.sh` atteste le prédicat ; ce fichier
# atteste que le handler l'appelle, relaie son refus, et se tait ou parle aux
# bons moments. Un handler qui n'appelle rien est vert exactement comme un
# handler qui refuse : d'où le CONTRÔLE NÉGATIF en fin de fichier, qui retire
# le branchement et vérifie que le harnais rougit
# (feedback_verify_pipeline_passes_without_the_fix).
#
# LE TEST GIT EST LE CONTRÔLE QUI COMPTE : les trois commandes de M0 visent une
# branche de fixture qui existe et dont l'extraction SALIRAIT l'arbre. Le
# harnais vérifie l'arbre, pas seulement le code de sortie.

set -uo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=${SCRIPT_DIR%/scripts}
RUN_SH="$REPO_ROOT/crates/mika-agent/templates/skills/shell-exec/handlers/run.sh"
GUARD="$REPO_ROOT/scripts/guard-shared-checkout"

PASS=0
FAIL=0
FAILURES=()
ok() { PASS=$((PASS + 1)); printf '  ok   %s\n' "$1"; }
ko() {
	FAIL=$((FAIL + 1))
	FAILURES+=("$1")
	printf '  FAIL %s\n' "$1" >&2
	[ "$#" -gt 1 ] && printf '       %s\n' "$2" >&2
	return 0
}

command -v jq >/dev/null 2>&1 || { echo "jq requis" >&2; exit 1; }

# Hermeticity (mika#1772) : pas de signature, identité posée, `main` par défaut.
export GIT_CONFIG_COUNT=5
export GIT_CONFIG_KEY_0=commit.gpgsign GIT_CONFIG_VALUE_0=false
export GIT_CONFIG_KEY_1=tag.gpgsign GIT_CONFIG_VALUE_1=false
export GIT_CONFIG_KEY_2=init.defaultBranch GIT_CONFIG_VALUE_2=main
export GIT_CONFIG_KEY_3=user.name GIT_CONFIG_VALUE_3=guard
export GIT_CONFIG_KEY_4=user.email GIT_CONFIG_VALUE_4=guard@test

TMPROOT=$(mktemp -d)
trap 'rm -rf -- "$TMPROOT"' EXIT

# ---------------------------------------------------------------------------
# Fixture : une plateforme avec un checkout principal `mika` (qui porte la
# garde dans scripts/, comme en production), un worktree lié, une branche
# `feat` dont l'extraction salirait l'arbre, et un `~/workspace` symlink.
# ---------------------------------------------------------------------------
mkdir -p "$TMPROOT/data/workspace/mika-platform" "$TMPROOT/home"
ln -s ../data/workspace "$TMPROOT/home/workspace"
PLATFORM=$(cd -- "$TMPROOT/data/workspace/mika-platform" && pwd -P)
FAKE_HOME=$(cd -- "$TMPROOT/home" && pwd -P)
PRIMARY="$PLATFORM/mika"
mkdir -p "$PRIMARY/scripts" "$PRIMARY/site"
cp "$GUARD" "$PRIMARY/scripts/guard-shared-checkout"
chmod +x "$PRIMARY/scripts/guard-shared-checkout"
printf 'main\n' >"$PRIMARY/scripts/check-landing-tokens.sh"
printf 'main\n' >"$PRIMARY/site/index.html"
git -C "$PRIMARY" init -q -b main
git -C "$PRIMARY" add -A
git -C "$PRIMARY" commit -q -m seed
git -C "$PRIMARY" checkout -q -b feat
printf 'feat\n' >"$PRIMARY/scripts/check-landing-tokens.sh"
printf 'feat\n' >"$PRIMARY/site/index.html"
git -C "$PRIMARY" commit -q -am feat
git -C "$PRIMARY" checkout -q main
FEAT_SHA=$(git -C "$PRIMARY" rev-parse feat)
HEAD_BEFORE=$(git -C "$PRIMARY" rev-parse HEAD)
mkdir -p "$PLATFORM/.claude/worktrees/x"
git -C "$PRIMARY" worktree add -q "$PLATFORM/.claude/worktrees/x/mika" feat >/dev/null 2>&1
LINKED="$PLATFORM/.claude/worktrees/x/mika"
SKILL_DIR="$TMPROOT/skill-dir"
mkdir -p "$SKILL_DIR"
# run.sh scrube TOUTES les MIKA_* — MIKA_GUARD_SHARED_CHECKOUT_LOG comprise —
# donc la garde appelée par le handler journalise au chemin par défaut, sous
# le HOME de la fixture. C'est là qu'on lit.
GUARD_LOG="$FAKE_HOME/.mika/state/shared-checkout-guard.log"

# run_handler <json> [ENV=VAL…] → stdout dans $OUT, stderr dans $ERR, code dans $STATUS
OUT=''; ERR=''; STATUS=0
run_handler() {
	local json=$1
	shift
	local errf="$TMPROOT/err.$$"
	OUT=$(cd "$SKILL_DIR" && env HOME="$FAKE_HOME" MIKA_PLATFORM_DIR="$PLATFORM" "$@" sh "$RUN_SH" <<<"$json" 2>"$errf")
	STATUS=$?
	ERR=$(cat "$errf")
}
json_cmd() { jq -cn --arg c "$1" '{command: $c}'; }

tree_intact() {
	local name=$1
	local st
	st=$(git -C "$PRIMARY" status --porcelain)
	if [ -z "$st" ] && [ "$(git -C "$PRIMARY" rev-parse HEAD)" = "$HEAD_BEFORE" ] && [ "$(git -C "$PRIMARY" rev-parse --abbrev-ref HEAD)" = main ]; then
		ok "$name — arbre du principal intact (porcelain vide, HEAD inchangé, sur main)"
	else
		ko "$name — arbre du principal intact" "porcelain: $st ; HEAD: $(git -C "$PRIMARY" rev-parse HEAD)"
		git -C "$PRIMARY" checkout -q -- . 2>/dev/null; git -C "$PRIMARY" reset -q --hard "$HEAD_BEFORE" 2>/dev/null; git -C "$PRIMARY" checkout -q main 2>/dev/null
	fi
}

M0_1="cd ~/workspace/mika-platform/mika && git -C . checkout $FEAT_SHA -- scripts/check-landing-tokens.sh site/ && bash scripts/check-landing-tokens.sh"
M0_2="cd ~/workspace/mika-platform/mika && git checkout feat -- scripts/check-landing-tokens.sh && bash scripts/check-landing-tokens.sh; git checkout -- scripts/check-landing-tokens.sh"
M0_3="cd ~/workspace/mika-platform/mika && git fetch origin feat 2>/dev/null; git checkout feat -- scripts/check-landing-tokens.sh site/index.html && make test-dispatch-lib"

printf '\n== AC5 — les trois commandes de M0 sont REFUSÉES par run_shell, et l'"'"'arbre reste intact ==\n'
i=0
for cmd in "$M0_1" "$M0_2" "$M0_3"; do
	i=$((i + 1))
	run_handler "$(json_cmd "$cmd")"
	case $OUT in
	'REFUS (shared-checkout-guard, mika#2449)'*) ok "M0_$i — la sortie commence par le jeton de refus" ;;
	*) ko "M0_$i — la sortie commence par le jeton de refus" "exit=$STATUS out=$OUT err=$ERR" ;;
	esac
	if [ "$STATUS" -eq 1 ]; then ok "M0_$i — exit 1"; else ko "M0_$i — exit 1" "exit=$STATUS"; fi
	tree_intact "M0_$i"
done

printf '\n== Contrôle positif de la fixture : la même commande NON gardée salirait bien l'"'"'arbre ==\n'
# Exécutée directement (sans le handler), M0_1 doit modifier l'index : sinon
# « l'arbre intact » ci-dessus ne prouverait rien (classe mika#2205).
(cd "$PRIMARY" && git checkout -q "$FEAT_SHA" -- scripts/check-landing-tokens.sh site/) 2>/dev/null
if [ -n "$(git -C "$PRIMARY" status --porcelain)" ]; then ok "sans la garde, checkout <ref> -- <paths> salit l'arbre (staged)"; else ko "contrôle positif : la commande de fixture ne salit rien"; fi
git -C "$PRIMARY" reset -q --hard "$HEAD_BEFORE"

printf '\n== AC6 — une lecture et les formes de synchronisation passent, et s'"'"'exécutent ==\n'
run_handler "$(json_cmd "git -C $PRIMARY log --oneline -1")"
if [ "$STATUS" -eq 0 ] && grep -q seed <<<"$OUT"; then ok "git log : exit 0 et la sortie est celle de git"; else ko "git log passe" "exit=$STATUS out=$OUT err=$ERR"; fi
run_handler "$(json_cmd "cd ~/workspace/mika-platform/mika && git fetch origin 2>/dev/null; git merge --ff-only main && echo SYNC_OK")"
if [ "$STATUS" -eq 0 ] && grep -q SYNC_OK <<<"$OUT"; then ok "fetch && merge --ff-only : exit 0, exécuté"; else ko "merge --ff-only passe" "exit=$STATUS out=$OUT"; fi
run_handler "$(json_cmd "git -C $PRIMARY show feat:scripts/check-landing-tokens.sh")"
if [ "$STATUS" -eq 0 ] && grep -q '^feat$' <<<"$OUT"; then ok "show <branch>:<path> (la recette qa-review) : exit 0, contenu rendu"; else ko "show passe" "exit=$STATUS out=$OUT"; fi
tree_intact "AC6"

printf '\n== V7 — hors population : worktree lié et hors plateforme passent (et s'"'"'exécutent) ==\n'
run_handler "$(json_cmd "cd $LINKED && git checkout main -- scripts/check-landing-tokens.sh && git status --porcelain | wc -l")"
if [ "$STATUS" -eq 0 ] && ! grep -q REFUS <<<"$OUT"; then ok "checkout <ref> -- <paths> dans un worktree lié : non refusé"; else ko "worktree lié non refusé" "exit=$STATUS out=$OUT"; fi
git -C "$LINKED" checkout -q -- . 2>/dev/null
OUTSIDE="$TMPROOT/outside"
git -C "$(mkdir -p "$OUTSIDE" && echo "$OUTSIDE")" init -q -b main && git -C "$OUTSIDE" commit -q --allow-empty -m x
run_handler "$(json_cmd "cd $OUTSIDE && git stash list && git checkout main")"
if [ "$STATUS" -eq 0 ] && ! grep -q REFUS <<<"$OUT"; then ok "checkout <branche> hors plateforme : non refusé"; else ko "hors plateforme non refusé" "exit=$STATUS out=$OUT"; fi

printf '\n== D6 — plateforme absente : allow, SANS ligne ==\n'
OUT=$(cd "$SKILL_DIR" && env HOME="$FAKE_HOME" MIKA_PLATFORM_DIR="$TMPROOT/nulle-part" sh "$RUN_SH" <<<"$(json_cmd 'echo hello')" 2>"$TMPROOT/err2"); STATUS=$?; ERR=$(cat "$TMPROOT/err2")
if [ "$STATUS" -eq 0 ] && [ "$OUT" = hello ] && ! grep -q 'shared-checkout guard' <<<"$ERR"; then ok "plateforme absente → exit 0, aucune ligne de garde sur stderr"; else ko "plateforme absente" "exit=$STATUS out=$OUT err=$ERR"; fi

printf '\n== D6 — plateforme présente SANS script : allow AVEC la ligne fail-open ==\n'
NOSCRIPT="$TMPROOT/plat-sans-script"; mkdir -p "$NOSCRIPT/mika"
OUT=$(cd "$SKILL_DIR" && env HOME="$FAKE_HOME" MIKA_PLATFORM_DIR="$NOSCRIPT" sh "$RUN_SH" <<<"$(json_cmd 'echo hello')" 2>"$TMPROOT/err3"); STATUS=$?; ERR=$(cat "$TMPROOT/err3")
if [ "$STATUS" -eq 0 ] && [ "$OUT" = hello ]; then ok "plateforme sans script → exit 0, la commande tourne"; else ko "plateforme sans script → exit 0" "exit=$STATUS out=$OUT"; fi
if grep -q "shell-exec: shared-checkout guard not found at $NOSCRIPT/mika/scripts/guard-shared-checkout (fail-open)" <<<"$ERR"; then ok "la ligne fail-open nomme le chemin cherché"; else ko "ligne fail-open" "err=$ERR"; fi

printf '\n== F3 — MIKA_GUARD_SHARED_CHECKOUT=0 : la commande passe ET la dérogation est dite à chaque appel ==\n'
rm -f "$GUARD_LOG"
run_handler "$(json_cmd "git -C $PRIMARY stash && echo BYPASSED")" MIKA_GUARD_SHARED_CHECKOUT=0
if ! grep -q REFUS <<<"$OUT" && grep -q BYPASSED <<<"$OUT"; then ok "stash (classe refusée) passe sous la dérogation"; else ko "dérogation : stash passe" "exit=$STATUS out=$OUT"; fi
if grep -q 'shell-exec: shared-checkout guard disarmed by MIKA_GUARD_SHARED_CHECKOUT=0 (operator override)' <<<"$ERR"; then ok "la ligne « disarmed … (operator override) » est sur stderr"; else ko "ligne disarmed" "err=$ERR"; fi
run_handler "$(json_cmd 'echo second')" MIKA_GUARD_SHARED_CHECKOUT=0
if grep -q 'disarmed' <<<"$ERR"; then ok "…et à CHAQUE appel, y compris sans git (le handler n'a pas de « premier »)"; else ko "disarmed à chaque appel" "err=$ERR"; fi
if grep -q 'mode=primary bypass MIKA_GUARD_SHARED_CHECKOUT=0' "$GUARD_LOG" 2>/dev/null; then ok "le bypass est journalisé par la garde (mode=primary bypass, ~/.mika/state/shared-checkout-guard.log)"; else ko "bypass journalisé" "$(cat "$GUARD_LOG" 2>/dev/null)"; fi
run_handler "$(json_cmd "git -C $PRIMARY stash")"
if grep -q "mode=primary deny platform=$PLATFORM target=$PRIMARY verb=stash" "$GUARD_LOG" 2>/dev/null; then ok "un refus est journalisé (mode=primary deny, plateforme, cible, verbe) au même fichier que mika#2107"; else ko "refus journalisés" "$(cat "$GUARD_LOG" 2>/dev/null)"; fi
git -C "$PRIMARY" stash drop -q 2>/dev/null || true
git -C "$PRIMARY" reset -q --hard "$HEAD_BEFORE"

printf '\n== V8 — run.sh lit MIKA_PLATFORM_DIR AVANT le scrub, et vise CE chemin ==\n'
# Une seconde plateforme, désignée par la variable ; la première (celle que
# ~/workspace désigne) n'est alors PAS protégée : c'est la preuve que la garde
# vise la variable et non $HOME/workspace/mika-platform.
PLAT2="$TMPROOT/plat2/mika-platform"; mkdir -p "$PLAT2/mika/scripts"
cp "$GUARD" "$PLAT2/mika/scripts/guard-shared-checkout"
git -C "$PLAT2/mika" init -q -b main && git -C "$PLAT2/mika" add -A && git -C "$PLAT2/mika" commit -q -m seed
PLAT2=$(cd -- "$PLAT2" && pwd -P)
OUT=$(cd "$SKILL_DIR" && env HOME="$FAKE_HOME" MIKA_PLATFORM_DIR="$PLAT2" sh "$RUN_SH" <<<"$(json_cmd "git -C $PLAT2/mika stash")" 2>/dev/null); STATUS=$?
if [ "$STATUS" -eq 1 ] && grep -q 'REFUS (shared-checkout-guard, mika#2449)' <<<"$OUT"; then ok "la plateforme désignée par MIKA_PLATFORM_DIR est protégée"; else ko "MIKA_PLATFORM_DIR est lu" "exit=$STATUS out=$OUT"; fi
OUT=$(cd "$SKILL_DIR" && env HOME="$FAKE_HOME" MIKA_PLATFORM_DIR="$PLAT2" sh "$RUN_SH" <<<"$(json_cmd "git -C $PRIMARY stash list")" 2>/dev/null); STATUS=$?
OUT=$(cd "$SKILL_DIR" && env HOME="$FAKE_HOME" MIKA_PLATFORM_DIR="$PLAT2" sh "$RUN_SH" <<<"$(json_cmd "cd ~/workspace/mika-platform/mika && git stash && echo NOT_GUARDED")" 2>/dev/null); STATUS=$?
if grep -q NOT_GUARDED <<<"$OUT"; then ok "…et ~/workspace/mika-platform n'est PAS protégée quand la variable désigne ailleurs (la variable décide, pas \$HOME)"; else ko "la variable décide" "exit=$STATUS out=$OUT"; fi
git -C "$PRIMARY" stash drop -q 2>/dev/null || true
git -C "$PRIMARY" reset -q --hard "$HEAD_BEFORE"
# La variable ne doit pas atteindre la commande exécutée (le scrub tient).
run_handler "$(json_cmd 'echo "P=${MIKA_PLATFORM_DIR:-scrubbed} G=${MIKA_GUARD_SHARED_CHECKOUT:-scrubbed}"')"
if [ "$OUT" = "P=scrubbed G=scrubbed" ]; then ok "MIKA_PLATFORM_DIR et MIKA_GUARD_SHARED_CHECKOUT restent scrubbés pour la commande"; else ko "scrub intact" "out=$OUT"; fi

printf '\n== U5 — le handler ne porte pas le jeton de refus : SOLE WRITER = la garde ==\n'
NONCOMMENT_TOKEN=$(grep -v '^[[:space:]]*#' "$RUN_SH" | grep -c 'REFUS (shared-checkout-guard' || true)
if [ "$NONCOMMENT_TOKEN" -eq 0 ]; then ok "aucune ligne de code de run.sh n'écrit « REFUS (shared-checkout-guard »"; else ko "sole writer" "$NONCOMMENT_TOKEN ligne(s)"; fi
GUARD_TOKEN_SITES=$(grep -c "REFUS (%s, mika#2449)" "$GUARD" || true)
if [ "$GUARD_TOKEN_SITES" -eq 1 ]; then ok "la garde écrit le jeton mika#2449 à exactement UN site"; else ko "un site dans la garde" "$GUARD_TOKEN_SITES"; fi

printf '\n== V5 — CONTRÔLE NÉGATIF : sans le branchement, le harnais ROUGIT ==\n'
STRIPPED="$TMPROOT/run-stripped.sh"
sed '/^# --- mika#2449: shared-checkout guard — the primary/,/^# --- end shared-checkout guard ---$/d' "$RUN_SH" >"$STRIPPED"
if ! grep -q 'decide-primary' "$STRIPPED"; then ok "le handler dépouillé ne contient plus l'appel (la coupe a pris)"; else ko "coupe du branchement" "decide-primary encore présent"; fi
OUT=$(cd "$SKILL_DIR" && env HOME="$FAKE_HOME" MIKA_PLATFORM_DIR="$PLATFORM" sh "$STRIPPED" <<<"$(json_cmd "$M0_1")" 2>/dev/null); STATUS=$?
case $OUT in
'REFUS (shared-checkout-guard, mika#2449)'*) ko "contrôle négatif : le handler dépouillé refuse encore — le harnais ne peut pas rougir" ;;
*) ok "handler dépouillé : M0_1 n'est PAS refusé (le harnais rougirait sur AC5)" ;;
esac
if [ -n "$(git -C "$PRIMARY" status --porcelain)" ]; then ok "…et l'arbre du principal EST sali (le contrôle git aussi rougirait)"; else ko "contrôle négatif : arbre non sali" "la commande M0_1 n'a pas produit l'effet mesuré"; fi
git -C "$PRIMARY" reset -q --hard "$HEAD_BEFORE"

printf '\n%s\n' "----------------------------------------"
printf 'test-shell-exec-guard: %d ok, %d fail\n' "$PASS" "$FAIL"
if [ "$FAIL" -ne 0 ]; then
	printf '\nÉchecs:\n' >&2
	for f in "${FAILURES[@]}"; do printf '  - %s\n' "$f" >&2; done
	exit 1
fi
exit 0
