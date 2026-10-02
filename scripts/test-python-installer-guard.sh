#!/usr/bin/env bash
# test-python-installer-guard.sh — mika#2639 R3 (AC3, V1–V5, V7)
#
# Harnais du quatrième bloc de refus de `run_shell`
# (crates/mika-agent/templates/skills/shell-exec/handlers/run.sh) : le
# containment d'installateur Python. Modelé sur
# `scripts/test-shell-exec-guard.sh` (mika#2449) — même `run_handler`, même
# `json_cmd`, même comptage, même `exit 1` final.
#
# LA FIXTURE EST HERMÉTIQUE ET N'INSTALLE RIEN. Des shims `pip`/`pip3`/`uv`/
# `pipx`/`python`/`python3` sur le PATH écrivent une sentinelle sur stdout ; un
# faux `$W/.venv/bin/pip` et un faux `$W/.venv/bin/python` en écrivent une
# autre. Les formes refusées n'atteignent JAMAIS `eval`, donc aucun shim ne
# tourne : le harnais l'atteste par l'ABSENCE de sentinelle. Les contrôles
# positifs atteignent `eval` et le harnais l'atteste par sa PRÉSENCE — sans
# quoi « non refusé » serait indistinguable de « refusé par autre chose »
# (classe mika#2205).
#
# CHAQUE TERME EST VU ROUGE SÉPARÉMENT : les cinq formes d'évasion d'AC1 sont
# asservies une à une, jamais neutralisées d'un coup (leçon mika#2277). Et le
# CONTRÔLE NÉGATIF en fin de fichier retire le bloc et vérifie que le harnais
# rougirait (feedback_verify_pipeline_passes_without_the_fix).
#
# Statut de chaque entrée du corpus : MESURÉE (verbatim du ticket ou de son
# commentaire opérateur) ou RECONSTRUITE (forme énumérée par AC1 / le
# commentaire). Discipline mika#2565 : « fixtures are frozen, not refreshed ».

set -uo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=${SCRIPT_DIR%/scripts}
RUN_SH="$REPO_ROOT/crates/mika-agent/templates/skills/shell-exec/handlers/run.sh"

# Allowlist d'exceptions, LIVRÉE VIDE et épinglée vide (Fire-Disposition,
# option (a)). Motif `HANDLER_ENV_KNOWN_INERT` (mika#2536) et
# `pilot-push-allowlist.txt` (mika#2520). Quand la garde tire, on route la
# commande vers le substitut ; on n'ajoute PAS de ligne ici (doctrine
# mika#2201).
#
# LA POPULATION À EXEMPTER EST MESURÉE VIDE : aucun script, handler, prompt ni
# cible `make` du dépôt ne prescrit une installation dans l'environnement de
# l'HÔTE. Mesure, au `395ec6e5` :
#   grep -rln 'pip install\|pipx install\|uv pip' Makefile scripts/ skills/ crates/ .github/
# rendait zéro. Depuis mika#2639 elle rend quatre fichiers, et aucun n'est une
# prescription à exempter : le handler (qui EST la garde), ce harnais et le job
# CI (qui la TESTENT), et `qa-review/system_prompt.md` (qui prescrit la recette
# PERMISE, un venv sous le worktree). Le jour où un cinquième apparaît, c'est
# lui qu'il faut lire — pas une ligne à ajouter ici.
PYTHON_INSTALLER_ALLOWED_FORMS=()

PASS=0
FAIL=0
FAILURES=()
ok() {
	PASS=$((PASS + 1))
	printf '  ok   %s\n' "$1"
}
ko() {
	FAIL=$((FAIL + 1))
	FAILURES+=("$1")
	printf '  FAIL %s\n' "$1" >&2
	[ "$#" -gt 1 ] && printf '       %s\n' "$2" >&2
	return 0
}

command -v jq >/dev/null 2>&1 || {
	echo "jq requis" >&2
	exit 1
}

TMPROOT=$(mktemp -d)
trap 'rm -rf -- "$TMPROOT"' EXIT

# ---------------------------------------------------------------------------
# Fixture hermétique
# ---------------------------------------------------------------------------
HOST_SENTINEL='__HOST_INSTALLER_RAN__'
VENV_SENTINEL='__VENV_INSTALLER_RAN__'

SHIMS="$TMPROOT/shims"
mkdir -p "$SHIMS"
for bin in pip pip3 uv pipx python python3 cmake make configure; do
	printf '#!/bin/sh\nprintf %%s\\\\n "%s"\nexit 0\n' "$HOST_SENTINEL" >"$SHIMS/$bin"
	chmod +x "$SHIMS/$bin"
done

# Le venv jetable : un `pip` et un `python` sous `<worktree>/.venv/bin/`.
W="$TMPROOT/w"
mkdir -p "$W/.venv/bin"
for bin in pip python; do
	printf '#!/bin/sh\nprintf %%s\\\\n "%s"\nexit 0\n' "$VENV_SENTINEL" >"$W/.venv/bin/$bin"
	chmod +x "$W/.venv/bin/$bin"
done

FAKE_HOME="$TMPROOT/home"
SKILL_DIR="$TMPROOT/skill-dir"
mkdir -p "$FAKE_HOME" "$SKILL_DIR"
# MIKA_PLATFORM_DIR vise un chemin ABSENT : la garde mika#2449 est alors un
# no-op silencieux (son cas D6 « plateforme absente → allow, SANS ligne »), ce
# qui garde le stderr de ce harnais lisible.
NO_PLATFORM="$TMPROOT/nulle-part"

OUT=''
ERR=''
STATUS=0
# run_handler <json> [SCRIPT] → stdout dans $OUT, stderr dans $ERR, code dans $STATUS
run_handler() {
	local json=$1
	local script=${2:-$RUN_SH}
	local errf="$TMPROOT/err.$$"
	OUT=$(cd "$SKILL_DIR" && env -u MIKA_GUARD_SHARED_CHECKOUT \
		HOME="$FAKE_HOME" MIKA_PLATFORM_DIR="$NO_PLATFORM" \
		PATH="$SHIMS:$PATH" sh "$script" <<<"$json" 2>"$errf")
	STATUS=$?
	ERR=$(cat "$errf")
}
json_cmd() { jq -cn --arg c "$1" '{command: $c}'; }

REFUSAL_TOKEN='REFUS (python-installer-guard, mika#2639)'

# refused <nom> <motif attendu> <commande>
# Trois assertions par cas, et la troisième est celle qui compte : le refus doit
# précéder `eval`, donc AUCUN shim ne doit avoir tourné.
refused() {
	local name=$1 motif=$2 cmd=$3
	run_handler "$(json_cmd "$cmd")"
	if [ "$STATUS" -eq 1 ]; then
		ok "$name — exit 1"
	else
		ko "$name — exit 1" "exit=$STATUS out=$OUT err=$ERR"
	fi
	if grep -qF "$REFUSAL_TOKEN: $motif" <<<"$ERR"; then
		ok "$name — stderr porte « $REFUSAL_TOKEN: $motif »"
	else
		ko "$name — motif « $motif » sur stderr" "err=$ERR"
	fi
	if ! grep -qF "$HOST_SENTINEL" <<<"$OUT$ERR" && ! grep -qF "$VENV_SENTINEL" <<<"$OUT$ERR"; then
		ok "$name — refusé AVANT eval (aucun shim n'a tourné)"
	else
		ko "$name — refusé avant eval" "un installateur a tourné : out=$OUT"
	fi
}

# passed <nom> <sentinelle attendue|-> <commande>
# La sentinelle est le contrôle positif : sans elle, « non refusé » et « refusé
# par un autre bloc » rendent les mêmes octets.
passed() {
	local name=$1 sentinel=$2 cmd=$3
	run_handler "$(json_cmd "$cmd")"
	if ! grep -qF "$REFUSAL_TOKEN" <<<"$ERR"; then
		ok "$name — non refusé"
	else
		ko "$name — non refusé" "err=$ERR"
	fi
	if [ "$sentinel" = '-' ]; then
		return 0
	fi
	if grep -qF "$sentinel" <<<"$OUT"; then
		ok "$name — a bien atteint eval (sentinelle $sentinel)"
	else
		ko "$name — atteint eval" "exit=$STATUS out=$OUT err=$ERR"
	fi
}

# Les deux verbatims MESURÉS, gelés. F2 est la commande du ticket, caractère
# pour caractère sauf la branche git (inatteignable depuis le bac à sable) et
# le `…` de l'invocation pytest, remplacés par des équivalents inertes.
F1='pip install -e . -q --break-system-packages'
F2='R=~/workspace/mika-platform/claude-pilot; W=$(mktemp -d); git -C "$R" worktree add --detach "$W" origin/fix272-pilot-scratch-var-cpp 2>&1 && cd "$W" && pip install -e . -q --break-system-packages 2>&1 | tail -3 && python -m pytest -q ; git -C "$R" worktree remove --force "$W"'
F3='cd /tmp/cpp280-review && uv pip install -e . --break-system-packages 2>&1 | tail -3'

printf '\n== V1 — les verbatims MESURÉS sont refusés (ticket + commentaire opérateur) ==\n'
refused 'F1 (MESURÉE, ticket)     pip install -e . --break-system-packages' host_installer "$F1"
refused 'F2 (MESURÉE, ticket)     le one-liner complet du 2026-10-02' host_installer "$F2"
refused 'F3 (MESURÉE, comment)    uv pip install -e .' host_installer "$F3"

printf '\n== AC1 — les formes énumérées par le ticket et son commentaire ==\n'
refused 'F4  (RECONSTRUITE)  pip3 install' host_installer 'pip3 install requests'
refused 'F5  (RECONSTRUITE)  python -m pip install' host_installer 'python -m pip install -e .'
refused 'F6  (RECONSTRUITE)  python3 -m pip install' host_installer 'python3 -m pip install foo'
refused 'F7  (RECONSTRUITE)  pipx install' host_installer 'pipx install black'
refused 'F8  (RECONSTRUITE)  uv pip install --system' host_installer 'uv pip install --system foo'
refused 'F9  (RECONSTRUITE)  pip install --user' host_installer 'pip install --user foo'
refused 'F10 (RECONSTRUITE)  pip install --target' host_installer 'pip install --target /tmp/x foo'
refused 'F11 (RECONSTRUITE)  pip install --prefix' host_installer 'pip install --prefix /usr foo'

printf '\n== V4 — les cinq formes d'"'"'évasion d'"'"'AC1, une par une ==\n'
refused 'F12 évasion  sh -c' host_installer "sh -c 'pip install -e .'"
refused 'F13 évasion  eval' host_installer 'eval "pip install -e ."'
refused 'F14 évasion  chemin absolu' host_installer '/usr/bin/pip install -e .'
refused 'F15 évasion  séparateur ;' host_installer 'cd /tmp && pwd ; pip install -e .'
refused 'F16 évasion  $( )' host_installer 'echo $(pip install -e .)'

printf '\n== Les deux corrections que le prototype a imposées (§ Le prédicat) ==\n'
refused 'F17 guillemet fermant  "/usr/bin/pip" install' host_installer '"/usr/bin/pip" install foo'
refused 'F18 pip de venv + drapeau hôte' host_target_flag "W=$W; \"\$W/.venv/bin/pip\" install --user -e \"\$W\""

printf '\n== Formes adjacentes que la frontière d'"'"'AC1 laisserait passer ==\n'
refused 'F19 adjacence  python -mpip install' host_installer 'python -mpip install foo'
refused 'F20 drapeaux intercalés  pip --quiet install' host_installer 'pip --quiet install foo'

printf '\n== V2 — contrôles POSITIFS d'"'"'AC3 : ils passent ET atteignent eval ==\n'
passed 'P1 uv run pytest' "$HOST_SENTINEL" 'uv run pytest -q'
passed 'P2 "$W/.venv/bin/pip" install -e "$W"' "$VENV_SENTINEL" "W=$W; \"\$W/.venv/bin/pip\" install -e \"\$W\""
passed 'P3 "$W/.venv/bin/python" -m pip install' "$VENV_SENTINEL" "W=$W; \"\$W/.venv/bin/python\" -m pip install -e \"\$W\""
passed 'P4 la recette complète d'"'"'AC2 (venv jetable)' "$VENV_SENTINEL" "W=$W; python -m venv \"\$W/.venv\" >/dev/null && \"\$W/.venv/bin/pip\" install -e \"\$W\""

printf '\n== V3 — contrôles NÉGATIFS de bruit : ils passent ==\n'
passed 'N1 pip list' "$HOST_SENTINEL" 'pip list'
passed 'N2 pip --version' "$HOST_SENTINEL" 'pip --version'
passed 'N3 cmake --prefix /x' "$HOST_SENTINEL" 'cmake --prefix /x'
passed 'N4 echo "pipeline install done"' '-' 'echo "pipeline install done"'
passed 'N5 configure --prefix=… && make install' "$HOST_SENTINEL" 'configure --prefix=/usr >/dev/null && make install'

printf '\n== Allowlist : LIVRÉE VIDE et épinglée vide (Fire-Disposition (a)) ==\n'
if [ "${#PYTHON_INSTALLER_ALLOWED_FORMS[@]}" -eq 0 ]; then
	ok "PYTHON_INSTALLER_ALLOWED_FORMS est vide — rien à exempter, la population est mesurée vide"
else
	ko "allowlist vide" "${#PYTHON_INSTALLER_ALLOWED_FORMS[@]} entrée(s) : quand la garde tire, on route vers le substitut (mika#2201)"
fi

printf '\n== V7 — le jeton de refus a un SITE DE DÉFINITION UNIQUE, et deux motifs figés ==\n'
TOKEN_SITES=$(grep -v '^[[:space:]]*#' "$RUN_SH" | grep -cF "$REFUSAL_TOKEN" || true)
if [ "$TOKEN_SITES" -eq 1 ]; then
	ok "run.sh écrit « $REFUSAL_TOKEN » à exactement UN site de code"
else
	ko "site de définition unique" "$TOKEN_SITES site(s) hors commentaire"
fi
for motif in host_installer host_target_flag; do
	if grep -v '^[[:space:]]*#' "$RUN_SH" | grep -qF "$motif"; then
		ok "le motif « $motif » est un format de fil porté par run.sh"
	else
		ko "motif « $motif » présent" "absent du code de run.sh"
	fi
done
# Un troisième motif rendrait la table de lecture du CLAUDE.md racine fausse en
# silence : les deux valeurs sont le format de fil que les requêtes SQL lisent.
MOTIF_ARGS=$(grep -v '^[[:space:]]*#' "$RUN_SH" | grep -oE '_refuse_python_installer [a-z_]+' | sort -u | wc -l)
if [ "$MOTIF_ARGS" -eq 2 ]; then
	ok "exactement DEUX motifs sont émis (host_installer, host_target_flag)"
else
	ko "deux motifs émis" "$MOTIF_ARGS motif(s) distinct(s) passé(s) à _refuse_python_installer"
fi

printf '\n== V5 — CONTRÔLE NÉGATIF : sans le bloc, le harnais ROUGIT ==\n'
STRIPPED="$TMPROOT/run-stripped.sh"
sed '/^# --- mika#2639: python installer containment ---$/,/^# --- end python installer containment ---$/d' "$RUN_SH" >"$STRIPPED"
if ! grep -qF "$REFUSAL_TOKEN" "$STRIPPED"; then
	ok "le handler dépouillé ne porte plus le jeton (la coupe a pris)"
else
	ko "coupe du bloc" "le jeton est encore présent dans $STRIPPED"
fi
run_handler "$(json_cmd "$F1")" "$STRIPPED"
if ! grep -qF "$REFUSAL_TOKEN" <<<"$ERR"; then
	ok "handler dépouillé : F1 n'est PAS refusé (le harnais rougirait sur V1)"
else
	ko "contrôle négatif : le handler dépouillé refuse encore" "le harnais ne peut pas rougir"
fi
if grep -qF "$HOST_SENTINEL" <<<"$OUT"; then
	ok "…et l'installateur de l'hôte A tourné (le contrôle « avant eval » rougirait aussi)"
else
	ko "contrôle négatif : l'installateur n'a pas tourné" "out=$OUT err=$ERR — la fixture ne mesure rien"
fi

printf '\n%s\n' "----------------------------------------"
printf 'test-python-installer-guard: %d ok, %d fail\n' "$PASS" "$FAIL"
if [ "$FAIL" -ne 0 ]; then
	printf '\nÉchecs:\n' >&2
	for f in "${FAILURES[@]}"; do printf '  - %s\n' "$f" >&2; done
	exit 1
fi
exit 0
