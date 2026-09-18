#!/usr/bin/env bash
# test-guard-shared-checkout.sh — mika#2107
#
# Harnais table-driven de `scripts/guard-shared-checkout`.
#
# LES QUATRE OCCURRENCES MESURÉES SONT DES FIXTURES NOMMÉES ET DATÉES. Un test
# qui n'exercerait que des cas inventés attesterait d'une garde, pas de CELLE
# que le ticket demande.
#
# LES CONTRÔLES NÉGATIFS COMPTENT AUTANT QUE LES POSITIFS : ce hook tourne avant
# CHAQUE appel Bash de CHAQUE session enracinée dans ce dépôt, dispatches
# comprises. Un faux positif ne casse pas un test, il cale la boucle autonome.

set -uo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
GUARD="$SCRIPT_DIR/guard-shared-checkout"

PASS=0
FAIL=0
FAILURES=()

# ---------------------------------------------------------------------------
# TABLE D'EXCEPTIONS — Fire-Disposition (mika#1574)
#
# Disposition retenue : option (a), exception nommée, `allowlist: zero entries`.
# La vacuité est MESURÉE (aucun `.claude/settings.json` suivi avant ce PR,
# aucun `scripts/guard-*` préexistant, les 119 `git -C` de `dispatch-lib.sh`
# visant tous le worktree courant) et elle est ASSERTÉE plus bas — sinon elle ne
# serait qu'une affirmation de plan.
#
# FORME OBLIGATOIRE D'UNE FUTURE ENTRÉE, si jamais il en faut une :
#     "<donnée exacte>|<ticket de suivi>|<condition de péremption>"
#   - donnée exacte : la chaîne de commande ou le chemin, jamais un motif large
#   - ticket de suivi : l'issue qui la retire, jamais « à voir »
#   - condition de péremption : une assertion qui ROUGIT quand le suivi se
#     résout, pour que l'entrée ne survive pas à sa raison d'être
#
# Cette table vit dans le HARNAIS, jamais dans le script de production : une
# allowlist lue au runtime serait un cinquième terme non mesuré devant un
# prédicat dont toute la conception tient au fait qu'il a exactement quatre
# termes conjoints et positifs.
# ---------------------------------------------------------------------------
GUARD_EXCEPTION_ALLOWLIST=()

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

# expect <allow|deny> <name> <project_dir> <cwd> <command>
expect() {
	local want=$1 name=$2 project_dir=$3 cwd=$4 command=$5
	local out status
	out=$(bash "$GUARD" --decide "$project_dir" "$cwd" "$command" 2>&1)
	status=$?
	case $want in
	allow)
		if [ "$status" -eq 0 ]; then ok "$name"; else ko "$name" "attendu allow, obtenu deny: $out"; fi
		;;
	deny)
		if [ "$status" -eq 1 ]; then ok "$name"; else ko "$name" "attendu deny, obtenu allow (exit $status)"; fi
		;;
	esac
}

# ---------------------------------------------------------------------------
# Fixtures : un vrai checkout principal et un vrai worktree lié.
# ---------------------------------------------------------------------------
TMPROOT=$(mktemp -d)
trap 'rm -rf -- "$TMPROOT"' EXIT

build_repo_pair() {
	local root=$1 main_name=$2 wt_name=$3
	local main="$root/$main_name"
	mkdir -p "$main"
	git -C "$main" init -q -b main
	git -C "$main" config user.email guard@test
	git -C "$main" config user.name guard
	printf 'seed\n' >"$main/seed.txt"
	git -C "$main" add seed.txt
	git -C "$main" commit -q -m seed
	git -C "$main" worktree add -q -b guard-branch "$root/$wt_name" >/dev/null 2>&1
	printf '%s' "$main"
}

MAIN=$(build_repo_pair "$TMPROOT" mika wt-mika)
WT="$TMPROOT/wt-mika"
mkdir -p "$WT/sub"
NOT_A_REPO="$TMPROOT/plain-dir"
mkdir -p "$NOT_A_REPO"

# Le journal part vers une destination jetable : aucun test n'écrit dans le
# `~/.mika/state/` de la machine qui lance la CI.
export MIKA_GUARD_SHARED_CHECKOUT_LOG="$TMPROOT/guard.log"

printf '\n== Préconditions de fixture ==\n'
if [ -f "$WT/.git" ]; then
	ok "le worktree lié porte un .git FICHIER (le terme T1 est observable)"
else
	ko "le worktree lié porte un .git FICHIER" "fixture cassée: $WT/.git absent ou répertoire"
fi

# ---------------------------------------------------------------------------
# AC1 — les quatre occurrences mesurées du ticket
# ---------------------------------------------------------------------------
printf '\n== AC1 — les quatre occurrences mesurées (mika#2107) ==\n'
expect deny "occ.1 30/08 matin  mika/ — git reset --hard depuis un cwd dérivé" \
	"$WT" "$MAIN" "git reset --hard"
expect deny "occ.2 30/08 16:54  mika/ — git checkout <branche> depuis un cwd dérivé" \
	"$WT" "$MAIN" "git checkout origin/fix/2031/dispatch-lib-dev-groom-has-no-dirty"
expect deny "occ.3 30/08 20:56  claude-pilot/ — git add via -C vers l'arbre partagé" \
	"$WT" "$WT" "git -C $MAIN add src tests docs CLAUDE.md"
expect deny "occ.4 31/08 09:31  claude-pilot/ — git checkout <sha> depuis un cwd dérivé" \
	"$WT" "$MAIN" "git checkout 4ed6af7d36"

printf '\n== AC1 — le refus nomme l'"'"'arbre visé, le remède et la dérogation ==\n'
REASON=$(bash "$GUARD" --decide "$WT" "$MAIN" "git checkout main" 2>&1)
for needle in "$MAIN" "$WT" "Remède" "MIKA_GUARD_SHARED_CHECKOUT=0"; do
	case $REASON in
	*"$needle"*) ok "le refus nomme « $needle »" ;;
	*) ko "le refus nomme « $needle »" "motif obtenu: $REASON" ;;
	esac
done

# ---------------------------------------------------------------------------
# Autres mécanismes de dérive — la même cause, d'autres formes
# ---------------------------------------------------------------------------
printf '\n== Autres formes de la même dérive ==\n'
expect deny "cd vers l'arbre partagé puis git add" "$WT" "$WT" "cd $MAIN && git add ."
expect deny "--git-dir vers l'arbre partagé" "$WT" "$WT" "git --git-dir=$MAIN/.git tag v9"
expect deny "GIT_WORK_TREE vers l'arbre partagé" "$WT" "$WT" "GIT_WORK_TREE=$MAIN git checkout main"
expect deny "git stash NU (empile, ce n'est pas une lecture)" "$WT" "$WT" "git -C $MAIN stash"
expect deny "git branch -d (option de mutation sur un verbe listant)" "$WT" "$WT" "git -C $MAIN branch -d foo"
expect deny "git worktree add (sous-verbe mutant)" "$WT" "$WT" "git -C $MAIN worktree add /tmp/x"
expect deny "git tag v1 (pose une étiquette, forme non listante)" "$WT" "$WT" "git -C $MAIN tag v1"
expect deny "git config --global user.name x (forme non lisante)" "$WT" "$WT" "git -C $MAIN config user.name x"
expect deny "segment git après && (le premier segment n'est pas git)" "$WT" "$WT" "echo hi && git -C $MAIN commit -m x"
expect deny "segment git après ; " "$WT" "$WT" "ls ; git -C $MAIN rm -rf ."
expect deny "git push depuis un cwd dérivé" "$WT" "$MAIN" "git push --force origin main"
expect deny "git clean -fdx depuis un cwd dérivé" "$WT" "$MAIN" "git clean -fdx"

# ---------------------------------------------------------------------------
# AC3 — allow-list de lecture, pas deny-list de mutation
# ---------------------------------------------------------------------------
printf '\n== AC3 — un verbe non classé visant hors du worktree est refusé ==\n'
expect deny "verbe inconnu « frobnicate » (absent des DEUX listes)" \
	"$WT" "$MAIN" "git frobnicate --wildly"
expect deny "verbe inconnu « sparse-checkout »" "$WT" "$WT" "git -C $MAIN sparse-checkout set x"
expect deny "alias local non résoluble (« st »)" "$WT" "$MAIN" "git st"

# ---------------------------------------------------------------------------
# AC2 — la garde ne coûte rien au nominal.
#
# C'est la moitié qui protège la boucle autonome : ce hook tourne avant chaque
# appel Bash de chaque dispatche.
# ---------------------------------------------------------------------------
printf '\n== AC2 — contrôles négatifs : le nominal ne paie rien ==\n'
expect allow "git add dans le worktree de la session" "$WT" "$WT" "git add ."
expect allow "git commit dans le worktree de la session" "$WT" "$WT" "git commit -m 'fix: x'"
expect allow "git checkout -b dans le worktree de la session" "$WT" "$WT" "git checkout -b feature/x"
expect allow "git reset --hard dans le worktree de la session" "$WT" "$WT" "git reset --hard"
expect allow "git -C <son propre worktree>" "$WT" "$WT" "git -C $WT add ."
expect allow "git -C <sous-répertoire de son worktree>" "$WT" "$WT" "git -C $WT/sub add ."
expect allow "cd dans son propre worktree puis git add" "$WT" "$MAIN" "cd $WT && git add ."
expect allow "commande sans git" "$WT" "$MAIN" "ls -la && cargo build"
expect allow "git --version (aucun verbe)" "$WT" "$MAIN" "git --version"
expect allow "chemin cible inexistant (la commande échouera d'elle-même)" \
	"$WT" "$WT" "git -C $TMPROOT/nope add ."
expect allow "cible qui n'est pas un dépôt git" "$WT" "$WT" "git -C $NOT_A_REPO add ."

printf '\n== AC2 — tous les verbes de lecture, même vers l'"'"'arbre partagé ==\n'
for verb in "status" "status --porcelain" "log --oneline -5" "diff" "diff --cached" \
	"show HEAD" "rev-parse HEAD" "rev-parse --abbrev-ref HEAD" "rev-list --count HEAD" \
	"ls-files" "cat-file -p HEAD" "blame seed.txt" "for-each-ref" "describe --tags" \
	"reflog" "reflog show" "shortlog -sn" "grep -n seed" "show-ref" "merge-base HEAD HEAD" \
	"branch" "branch -a" "branch -vv" "tag -l" "tag" "stash list" "stash show" \
	"worktree list" "remote -v" "remote" "config --get user.name" "config --list" \
	"symbolic-ref HEAD" "submodule status" "notes list" "ls-tree HEAD" "count-objects -v"; do
	expect allow "lecture: git $verb → arbre partagé" "$WT" "$WT" "git -C $MAIN $verb"
done

printf '\n== AC2 — une session qui n'"'"'est PAS enracinée dans un worktree lié ==\n'
expect allow "orchestrateur: project-dir = le checkout principal lui-même" \
	"$MAIN" "$MAIN" "git checkout some-branch"
expect allow "orchestrateur: project-dir = racine de l'espace de travail" \
	"$TMPROOT" "$MAIN" "git reset --hard"
expect allow "project-dir sans .git du tout" \
	"$NOT_A_REPO" "$MAIN" "git checkout main"
expect allow "project-dir vide (ancre indisponible → fail-open)" \
	"" "$MAIN" "git checkout main"

# ---------------------------------------------------------------------------
# AC2 (suite) — NON-RÉGRESSION DES DISPATCHES.
#
# La moitié la plus coûteuse à se tromper. Ce hook tourne avant CHAQUE appel
# Bash de CHAQUE dispatche `dev-pilot` / `dev-groom` : un faux positif ne fait
# pas rougir un test, il cale la boucle autonome. Les commandes ci-dessous sont
# celles qu'un pilote émet réellement pendant une session.
#
# La géométrie d'un worktree de dispatche est `<meta>/<slug>/<sous-repo>`, et
# `$CLAUDE_PROJECT_DIR` peut désigner l'un OU l'autre niveau. Les deux sont
# exercés : dans le second cas le sous-repo est SOUS le project-dir, donc le
# terme T4 est faux et la garde ne coûte pas même un fork.
# ---------------------------------------------------------------------------
printf '\n== AC2 — non-régression des dispatches (le nominal de la boucle) ==\n'
META="$TMPROOT/meta"
mkdir -p "$META"
SUB_MAIN=$(build_repo_pair "$META" sub-repo wt-sub)
SUB_WT="$META/wt-sub"

for cmd in \
	"git status --porcelain" \
	"git add -A" \
	"git commit -m 'fix(mika#2107): garde'" \
	"git rev-parse --abbrev-ref HEAD" \
	"git log --oneline -3" \
	"git diff --stat" \
	"git push -u origin HEAD" \
	"git fetch origin main" \
	"git rebase origin/main" \
	"git checkout -b fix/1234/slug" \
	"cargo build && git add -A && git commit -m wip" \
	"git add pr-body.md && git commit -m 'pr body' && rm pr-body.md" \
	"gh pr create --body-file pr-body.md" \
	"bash scripts/test-guard-shared-checkout.sh" \
	"cargo test -p mika-agent 2>&1 | tail -20"; do
	expect allow "dispatche (project-dir = le sous-repo): $cmd" "$SUB_WT" "$SUB_WT" "$cmd"
done

# Géométrie meta-repo : le project-dir est la racine du worktree, la commande
# vise le sous-repo qui est dessous.
mkdir -p "$META/wt-sub/nested"
expect allow "dispatche (project-dir = racine meta): git -C <sous-repo> commit" \
	"$META" "$META" "git -C $SUB_WT commit -m x"
expect allow "dispatche (project-dir = racine meta): cd <sous-repo> && git add" \
	"$META" "$META" "cd $SUB_WT && git add -A"

# ---------------------------------------------------------------------------
# AC5 — la dérogation fonctionne et se voit
# ---------------------------------------------------------------------------
printf '\n== AC5 — dérogation nommée et journalisée ==\n'
BYPASS_LOG="$TMPROOT/bypass.log"
if MIKA_GUARD_SHARED_CHECKOUT=0 MIKA_GUARD_SHARED_CHECKOUT_LOG="$BYPASS_LOG" \
	bash "$GUARD" --decide "$WT" "$MAIN" "git checkout main" >/dev/null 2>&1; then
	ok "MIKA_GUARD_SHARED_CHECKOUT=0 laisse passer"
else
	ko "MIKA_GUARD_SHARED_CHECKOUT=0 laisse passer" "la dérogation n'a pas été honorée"
fi
if [ -f "$BYPASS_LOG" ] && grep -q 'bypass' "$BYPASS_LOG"; then
	ok "la dérogation est journalisée (un contournement silencieux serait pire que pas de garde)"
else
	ko "la dérogation est journalisée" "aucune ligne « bypass » dans $BYPASS_LOG"
fi

# ---------------------------------------------------------------------------
# AC6 — tout refus est observable
# ---------------------------------------------------------------------------
printf '\n== AC6 — tout refus écrit une ligne exploitable ==\n'
DENY_LOG="$TMPROOT/deny.log"
MIKA_GUARD_SHARED_CHECKOUT_LOG="$DENY_LOG" \
	bash "$GUARD" --decide "$WT" "$MAIN" "git checkout main" >/dev/null 2>&1
if [ -f "$DENY_LOG" ] && grep -q "deny project=$WT" "$DENY_LOG" && grep -q "target=$MAIN" "$DENY_LOG"; then
	ok "le refus journalise project-dir, cible et commande"
else
	ko "le refus journalise project-dir, cible et commande" "contenu: $(cat "$DENY_LOG" 2>/dev/null)"
fi

# ---------------------------------------------------------------------------
# AC9 — fail-open, et le journal n'est JAMAIS un terme du prédicat
# ---------------------------------------------------------------------------
printf '\n== AC9 — la garde tombe en marche du bon côté ==\n'
UNWRITABLE=/proc/guard-shared-checkout-nowhere/x.log
if MIKA_GUARD_SHARED_CHECKOUT_LOG="$UNWRITABLE" \
	bash "$GUARD" --decide "$WT" "$MAIN" "git checkout main" >/dev/null 2>&1; then
	ko "journal non inscriptible → la décision ne change pas" \
		"la garde a laissé passer parce qu'elle n'a pas pu écrire son log (fail-closed déguisé inversé)"
else
	ok "journal non inscriptible → la décision reste deny (journaliser n'est pas un terme du prédicat)"
fi
if MIKA_GUARD_SHARED_CHECKOUT_LOG="$UNWRITABLE" \
	bash "$GUARD" --decide "$WT" "$WT" "git add ." >/dev/null 2>&1; then
	ok "journal non inscriptible → un allow reste un allow"
else
	ko "journal non inscriptible → un allow reste un allow" "la garde a refusé un geste nominal"
fi

# Le cas réel « worktree créé sur une branche antérieure au correctif » : la
# branche ne porte NI le settings.json NI le script. Rien ne tire — ce n'est pas
# une violation préexistante mais une ABSENCE de détecteur. On l'exerce sous la
# seule forme observable : le script introuvable.
MISSING_GUARD="$TMPROOT/absent/guard-shared-checkout"
bash "$MISSING_GUARD" --decide "$WT" "$MAIN" "git checkout main" >/dev/null 2>&1
if [ "$?" -ne 1 ]; then
	ok "script absent (worktree pré-correctif) → aucun refus n'est prononcé"
else
	ko "script absent → aucun refus n'est prononcé" "un script absent a produit un deny"
fi

# ---------------------------------------------------------------------------
# AC8 — agnosticisme du dépôt
# ---------------------------------------------------------------------------
printf '\n== AC8 — aucun chemin ni nom de dépôt en dur ==\n'
OTHER_ROOT="$TMPROOT/other"
mkdir -p "$OTHER_ROOT"
OTHER_MAIN=$(build_repo_pair "$OTHER_ROOT" acme-core wt-acme)
OTHER_WT="$OTHER_ROOT/wt-acme"
expect deny "arborescence portant un autre nom: checkout vers l'arbre partagé" \
	"$OTHER_WT" "$OTHER_MAIN" "git checkout main"
expect allow "arborescence portant un autre nom: geste nominal" \
	"$OTHER_WT" "$OTHER_WT" "git add ."
# Le motif porte sur le CODE, jamais sur la prose : les commentaires de ce
# script nomment délibérément des chemins et des commandes de la machine où les
# quatre occurrences ont été mesurées, et c'est ce qui rend son raisonnement
# relisible. Ce qui est interdit, c'est qu'un chemin gouverne une DÉCISION.
GUARD_CODE=$(sed 's/^[[:space:]]*#.*$//' "$GUARD")
if printf '%s' "$GUARD_CODE" | grep -qE '/data/workspace|senara-solutions|mika-platform'; then
	ko "le script ne code aucun chemin de machine en dur" \
		"un chemin absolu ou un nom d'organisation apparaît hors commentaire dans $GUARD"
else
	ok "le script ne code aucun chemin de machine en dur"
fi

# ---------------------------------------------------------------------------
# Mode hook — le contrat d'E/S réel de production
# ---------------------------------------------------------------------------
printf '\n== Mode hook — contrat JSON ==\n'
hook_payload() {
	jq -nc --arg cwd "$1" --arg cmd "$2" \
		'{session_id:"t",hook_event_name:"PreToolUse",tool_name:"Bash",cwd:$cwd,tool_input:{command:$cmd}}'
}

HOOK_OUT=$(hook_payload "$MAIN" "git checkout main" |
	CLAUDE_PROJECT_DIR="$WT" bash "$GUARD" 2>/dev/null)
HOOK_STATUS=$?
if [ "$HOOK_STATUS" -eq 2 ]; then
	ok "refus en mode hook: code de sortie 2 (canal de blocage inconditionnel)"
else
	ko "refus en mode hook: code de sortie 2" "obtenu $HOOK_STATUS"
fi
if printf '%s' "$HOOK_OUT" | jq -e '.hookSpecificOutput.permissionDecision == "deny"' >/dev/null 2>&1; then
	ok "refus en mode hook: JSON permissionDecision=deny (second canal, M0-b non mesuré)"
else
	ko "refus en mode hook: JSON permissionDecision=deny" "stdout: $HOOK_OUT"
fi

hook_payload "$WT" "git add ." | CLAUDE_PROJECT_DIR="$WT" bash "$GUARD" >/dev/null 2>&1
if [ "$?" -eq 0 ]; then
	ok "geste nominal en mode hook: code de sortie 0, aucune décision imposée"
else
	ko "geste nominal en mode hook: code de sortie 0" "la garde a refusé un geste nominal"
fi

printf 'ceci n est pas du json' | CLAUDE_PROJECT_DIR="$WT" bash "$GUARD" >/dev/null 2>&1
if [ "$?" -eq 0 ]; then
	ok "charge utile illisible → fail-open (exit 0)"
else
	ko "charge utile illisible → fail-open" "une charge cassée a bloqué un appel Bash"
fi

jq -nc --arg cmd "git checkout main" \
	'{hook_event_name:"PreToolUse",tool_name:"Write",cwd:"/",tool_input:{file_path:$cmd}}' |
	CLAUDE_PROJECT_DIR="$WT" bash "$GUARD" >/dev/null 2>&1
if [ "$?" -eq 0 ]; then
	ok "outil autre que Bash → aucune décision"
else
	ko "outil autre que Bash → aucune décision" "la garde a mordu hors de son périmètre"
fi

# Une commande multi-ligne est une seule chaîne JSON : elle doit être
# reconstruite en entier, sinon un segment dangereux au-delà de la première
# ligne serait invisible. Les deux directions sont exercées — une chaîne
# tronquée rendrait le premier cas vert par accident.
MULTILINE_DENY=$(printf 'echo "préparation"\ngit -C %s reset --hard\necho fini' "$MAIN")
hook_payload "$WT" "$MULTILINE_DENY" | CLAUDE_PROJECT_DIR="$WT" bash "$GUARD" >/dev/null 2>&1
if [ "$?" -eq 2 ]; then
	ok "commande multi-ligne: le segment dangereux de la ligne 2 est vu"
else
	ko "commande multi-ligne: le segment dangereux de la ligne 2 est vu" \
		"la chaîne a été tronquée à la première ligne"
fi
MULTILINE_ALLOW=$(printf 'echo "préparation"\ngit add -A\ngit commit -m fini')
hook_payload "$WT" "$MULTILINE_ALLOW" | CLAUDE_PROJECT_DIR="$WT" bash "$GUARD" >/dev/null 2>&1
if [ "$?" -eq 0 ]; then
	ok "commande multi-ligne nominale: aucune décision imposée"
else
	ko "commande multi-ligne nominale: aucune décision imposée" "faux positif sur du multi-ligne nominal"
fi

# ---------------------------------------------------------------------------
# AC4 — preuve d'armement.
#
# Son ABSENCE est l'information : elle dit que le hook n'est pas chargé, et non
# que rien n'a eu à être refusé. C'est la seule chose qui distingue « rien à
# refuser » de « rien n'est armé » — et donc ce qui rend le fail-open ci-dessus
# un arbitrage observable plutôt qu'un désarmement silencieux.
# ---------------------------------------------------------------------------
printf '\n== AC4 — ligne d'"'"'armement SessionStart ==\n'
ARM_LOG="$TMPROOT/arm.log"
jq -nc '{hook_event_name:"SessionStart",source:"startup"}' |
	MIKA_GUARD_SHARED_CHECKOUT_LOG="$ARM_LOG" CLAUDE_PROJECT_DIR="$WT" bash "$GUARD" >/dev/null 2>&1
if [ -f "$ARM_LOG" ] && grep -q 'shared-checkout-guard: armed' "$ARM_LOG"; then
	ok "SessionStart écrit la ligne d'armement"
else
	ko "SessionStart écrit la ligne d'armement" "contenu: $(cat "$ARM_LOG" 2>/dev/null)"
fi
ARM_STDOUT=$(jq -nc '{hook_event_name:"SessionStart",source:"startup"}' |
	MIKA_GUARD_SHARED_CHECKOUT_LOG="$ARM_LOG" CLAUDE_PROJECT_DIR="$WT" bash "$GUARD" 2>/dev/null)
if [ -z "$ARM_STDOUT" ]; then
	ok "SessionStart n'écrit rien sur stdout (ce canal est injecté dans le contexte)"
else
	ko "SessionStart n'écrit rien sur stdout" "stdout: $ARM_STDOUT"
fi

# ---------------------------------------------------------------------------
# Fire-Disposition — la table d'exceptions est vide, et c'est ASSERTÉ
# ---------------------------------------------------------------------------
printf '\n== Fire-Disposition — allowlist: zero entries ==\n'
if [ "${#GUARD_EXCEPTION_ALLOWLIST[@]}" -eq 0 ]; then
	ok "table d'exceptions vide (voir la forme obligatoire d'une entrée en tête de fichier)"
else
	ko "table d'exceptions vide" \
		"${#GUARD_EXCEPTION_ALLOWLIST[@]} entrée(s) — chacune doit porter donnée exacte, ticket de suivi et condition de péremption"
fi
if grep -qE 'ALLOWLIST|EXCEPTION' "$GUARD"; then
	ko "le script de production ne consulte aucune table d'exceptions" \
		"une allowlist au runtime serait un cinquième terme devant un prédicat qui en a quatre"
else
	ok "le script de production ne consulte aucune table d'exceptions"
fi

# ---------------------------------------------------------------------------
printf '\n%s\n' "----------------------------------------"
printf 'guard-shared-checkout: %d ok, %d fail\n' "$PASS" "$FAIL"
if [ "$FAIL" -ne 0 ]; then
	printf '\nÉchecs:\n' >&2
	for f in "${FAILURES[@]}"; do printf '  - %s\n' "$f" >&2; done
	exit 1
fi
exit 0
