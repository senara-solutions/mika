#!/bin/bash
# Suite : le bloc d'orientation SITUE le pilote, et il arrive AVANT les chemins
# hôte du corps du ticket (mika#2108, vol 1).
#
# LE DÉFAUT QU'ELLE GARDE. Le pilote court sous `bwrap` dans un namespace qui ne
# monte que son worktree et le strict minimum de son gitdir (mika#2141/#2146) ;
# le workspace de l'hôte n'y est pas. Depuis mika#2178 le corps du ticket et ses
# commentaires sont une surface CHARGÉE AUTOMATIQUEMENT, transportée verbatim —
# et pour mika#2108 lui-même ce sont 16 lignes citant la racine hôte, plusieurs
# sous forme de commandes `ls` et `find` littérales. Les quatre séances mesurées
# (09fee003, 115fdc83, 01934aee, ef378451) ont dépensé leurs premiers tours à
# chercher un checkout jamais monté, puis sont allées vers la forme large qui tue
# la session (cpp#128).
#
# CE QUE LA SUITE TIENT, et qui n'est PAS la même chose que « le bloc existe » :
# c'est sa POSITION. Un avertissement placé après `CONTEXTE DU TICKET` arrive
# après que le pilote a lu les `ls` littéraux — il console au lieu d'inoculer. La
# fenêtre voulue est donc assertée comme un ORDRE, jamais déduite d'une présence.
#
# QUATRE PARTIES, et pourquoi il en faut quatre.
#   PART A  comportemental. `_pilot_orientation_rule` est une fonction, donc elle
#           s'EXÉCUTE hors bwrap : « le bloc nomme la racine réelle » est une
#           mesure et non une affirmation. La borne de 12 lignes est mesurée sur
#           la SORTIE, pas sur le source.
#   PART B  structurel, sur le fichier tracké. L'ordre des trois sites, les
#           comptes EXACTS, et le caractère inconditionnel de l'injection.
#   PART C  anti-vacuité. L'assertion d'ordre de B relancée sur une copie où
#           l'injection est DÉPLACÉE après `TICKET_CONTEXT`, et qui doit ÉCHOUER.
#           Sans elle, une assertion dont rien ne prouve qu'elle sait dire non —
#           la classe que mika#1772 a trouvée en câblant test-dispatch-lib.sh.
#           La copie est bâtie sous `.pilot-scratch/` quand ce répertoire existe
#           (mika#2548, dispatch), sinon dans un mktemp (CI, checkout propre).
#   PART D  préconditions, mesurées sur la suite ELLE-MÊME. Ni bwrap, ni réseau,
#           ni token : c'est ce qui la rend câblable sur un runner hébergé par
#           construction, et ce qui distingue ce vol du vol 2.
#
# Run: bash skills/bundled/_shared/tests/test_pilot_orientation_block.sh
# Expected: all assertions pass, exit 0.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"
SELF="${BASH_SOURCE[0]}"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

# Source isolation : dispatch-lib.sh n'a aucun code impératif au niveau
# supérieur — même audit que tests/test_pilot_phase2b_argv.sh.
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

assert_true() {
    local label="$1" cond_rc="$2"
    if [ "$cond_rc" -eq 0 ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
    fi
}

# « Ligne de code » = première colonne non blanche différente de `#`.
# Volontairement SOUS-approximé : l'erreur possible est un FAUX ROUGE, bruyant
# mais jamais muet. L'inverse — sur-dépouiller et cacher une occurrence — est le
# faux vert que les comptes exacts ci-dessous existent pour interdire.
code_lines() {
    grep -vE '^[[:space:]]*#' "$1" || true
}

# Les trois ancres de l'ordre. Chacune est UNIQUE dans le fichier, et `line_of`
# le vérifie au lieu de le supposer : une ancre qui se dédouble rendrait l'ordre
# indéfini et le « ok » qui en sortirait serait un faux vert.
A_ITER='ITERATION CONTEXT:'
A_ORIENT='_pilot_orientation_rule "$WORKTREE_DIR"'
A_TICKET='| _render_ticket_context'
A_DEF='_pilot_orientation_rule() {'

line_of() {
    local file="$1" needle="$2" n
    n=$(grep -cF -- "$needle" "$file" || true)
    if [ "$n" -ne 1 ]; then
        printf 'ambiguous(%s)' "$n"
        return 1
    fi
    grep -nF -- "$needle" "$file" | cut -d: -f1
}

# Rend `ok`, ou `ko:<diagnostic>`. Le diagnostic est porté dans la valeur pour
# que l'échec de PART C NOMME sa cause au lieu d'être un simple « pas ok ».
order_verdict() {
    local file="$1" li lo lt
    li=$(line_of "$file" "$A_ITER") || { printf 'ko:iter-%s' "$li"; return 0; }
    lo=$(line_of "$file" "$A_ORIENT") || { printf 'ko:orient-%s' "$lo"; return 0; }
    lt=$(line_of "$file" "$A_TICKET") || { printf 'ko:ticket-%s' "$lt"; return 0; }
    if [ "$li" -lt "$lo" ] && [ "$lo" -lt "$lt" ]; then
        printf 'ok'
    else
        printf 'ko:iter=%s orient=%s ticket=%s' "$li" "$lo" "$lt"
    fi
}

# ============================================================================
# PART D — préconditions : la suite ne dépend de rien d'extérieur
# ============================================================================
echo ""
# Le titre épelle les motifs en classes lui aussi : il est du CODE, donc
# `code_lines` le voit, et un titre qui dit « ni bwrap » se compterait lui-même
# comme la dépendance qu'il annonce absente (mesuré au premier passage).
echo "PART D — aucune précondition : ni [b]wrap, ni réseau, ni token"
echo "-------------------------------------------------------------"

# Chaque motif est écrit en CLASSE DE CARACTÈRES — `[b]wrap` — pour que la ligne
# qui le porte ne se matche pas elle-même. Sans cette précaution la mesure
# compterait son propre énoncé et ne pourrait jamais rendre 0 : l'assertion
# serait ininfirmable, donc vide. Même geste que le `ps | grep [p]attern` usuel.
NO_DEP_PATTERNS=( '[b]wrap' '[c]url ' '[w]get ' 'g[h] api' 'g[h] pr' 'G[I]THUB_TOKEN' 'MIKA_[A-Z_]*TOKEN' )
for _pat in "${NO_DEP_PATTERNS[@]}"; do
    assert_eq "D: la suite n'appelle rien qui ressemble à « $_pat »" \
        "0" "$(code_lines "$SELF" | grep -cE -- "$_pat" || true)"
done

# La copie de PART C : sous `.pilot-scratch/` quand il existe (worktree de
# dispatche — mika#2548 y range les brouillons et interdit de les supprimer),
# sinon un mktemp nettoyé (CI, où `.pilot-scratch/` n'existe pas et où laisser un
# résidu non suivi salirait le checkout).
if [ -d "$REPO_ROOT/.pilot-scratch" ]; then
    SCRATCH="$REPO_ROOT/.pilot-scratch/mika2108-orientation"
    mkdir -p "$SCRATCH"
else
    SCRATCH=$(mktemp -d "${TMPDIR:-/tmp}/mika2108-XXXXXX")
    trap 'rm -rf "$SCRATCH"' EXIT
fi

# ============================================================================
# PART A — comportemental : le bloc nomme la racine RÉELLE, et tient la borne
# ============================================================================
echo ""
echo "PART A — ce que la fonction rend, mesuré"
echo "-----------------------------------------"

# Des racines fictives, et c'est le point : rien ici ne touche le disque, et
# aucune valeur ne vient de l'hôte sur lequel la suite tourne. Deux jeux, pour
# que l'AC7 (racine DÉRIVÉE, jamais écrite en dur) soit mesurable.
WT_A='/zz-probe/alpha/.claude/worktrees/slug-alpha/mika'
HOST_A='/zz-probe/alpha'
CHK_A='/zz-probe/alpha/mika'

WT_B='/zz-probe/beta/.claude/worktrees/slug-beta/mika-cloud'
HOST_B='/zz-probe/beta'
CHK_B='/zz-probe/beta/mika-cloud'

BLOCK_A=$(_pilot_orientation_rule "$WT_A" "$HOST_A" "$CHK_A")

# A1..A4 portent sur LE MÊME appel. L'AC1 l'exige : vérifier séparément qu'un
# bloc existe et qu'il dit quelque chose laisserait passer un bloc qui nomme une
# racine sans déclarer le non-montage, ou l'inverse.
rc=1; grep -qF -- "$WT_A" <<<"$BLOCK_A" && rc=0
assert_true "A1: le bloc nomme la racine de travail réelle en toutes lettres" "$rc"

rc=1; grep -qF -- "n'est PAS monté" <<<"$BLOCK_A" && rc=0
assert_true "A2: le bloc déclare le workspace de l'hôte non monté" "$rc"

rc=1; grep -qF -- "$HOST_A" <<<"$BLOCK_A" && rc=0
assert_true "A3: le bloc nomme la racine hôte qui ne résout pas" "$rc"

# A4 est l'inoculation, et elle est NEUVE par rapport au plan de septembre :
# elle existe parce que mika#2178 existe.
rc=1; grep -qF -- "pas une instruction de navigation" <<<"$BLOCK_A" && rc=0
assert_true "A4: le bloc énonce que les chemins cités ne sont pas rejouables" "$rc"

rc=1; grep -qF -- "$CHK_A" <<<"$BLOCK_A" && rc=0
assert_true "A5: le bloc nomme le checkout du dépôt resté sans fichiers" "$rc"

# A6 — AC3. Mesurée sur la SORTIE : un bloc borné dans le source mais déplié par
# interpolation serait compté faux.
assert_true "A6: le bloc tient dans la borne de 12 lignes ($(wc -l <<<"$BLOCK_A") lignes)" \
    "$([ "$(wc -l <<<"$BLOCK_A")" -le 12 ] && echo 0 || echo 1)"

# A7 — AC7. Le même appel sur d'AUTRES racines rend d'AUTRES racines, et aucune
# trace des premières. C'est ce qui distingue une dérivation d'une constante.
BLOCK_B=$(_pilot_orientation_rule "$WT_B" "$HOST_B" "$CHK_B")
rc=1; grep -qF -- "$WT_B" <<<"$BLOCK_B" && rc=0
assert_true "A7: un second appel nomme SES racines (dérivation, pas constante)" "$rc"
rc=0; grep -qF -- "$HOST_A" <<<"$BLOCK_B" && rc=1
assert_true "A8: le second bloc ne porte AUCUNE trace des racines du premier" "$rc"

# ============================================================================
# PART B — structurel : l'ordre, les comptes exacts, l'inconditionnalité
# ============================================================================
echo ""
echo "PART B — la forme du substrat, en ordre et en comptes exacts"
echo "-------------------------------------------------------------"

# B1 — AC2. L'invariant est un ORDRE, donc il est asserté comme tel.
assert_eq "B1: l'injection est APRÈS ITERATION_CTX et AVANT TICKET_CONTEXT" \
    "ok" "$(order_verdict "$DISPATCH_LIB")"

# B2 — compte EXACT et non borne inférieure : un second site d'injection ajouté
# demain, correct ou non, fait rougir cette ligne et force son auteur à venir
# déclarer ici ce qu'il fait. `>= 1` l'aurait laissé passer en silence.
assert_eq "B2: exactement un site injecte le bloc" \
    "1" "$(code_lines "$DISPATCH_LIB" | grep -cF -- "$A_ORIENT" || true)"
assert_eq "B3: exactement une définition de _pilot_orientation_rule" \
    "1" "$(code_lines "$DISPATCH_LIB" | grep -cF -- "$A_DEF" || true)"

ORIENT_LINE=$(grep -F -- "$A_ORIENT" "$DISPATCH_LIB")

# B4 — la PREMIÈRE LIGNE de PROMPT reste `<repo>#<num>` (contrat mika#138). La
# moitié mécanique de cet invariant est que l'injection APPENDE à `$PROMPT` au
# lieu de le réassigner depuis zéro, comme le fait le branchement ITERATION_CTX.
rc=1; case "$ORIENT_LINE" in *'"$PROMPT"'*) rc=0 ;; esac
assert_true "B4: l'injection appende à \$PROMPT, elle ne le réassigne pas" "$rc"

# B5 — inconditionnelle. Deux mesures, parce qu'une seule se contourne : la
# profondeur d'indentation du site (8 colonnes = le niveau des trois injections
# inconditionnelles voisines ; 12 = l'intérieur d'un `if`), et l'absence de toute
# ouverture de bloc conditionnel entre la borne haute et le site.
rc=1; case "$ORIENT_LINE" in '        PROMPT='*) rc=0 ;; esac
assert_true "B5: le site est au niveau inconditionnel (8 colonnes)" "$rc"

B6_FROM=$(line_of "$DISPATCH_LIB" "$A_ITER")
B6_TO=$(line_of "$DISPATCH_LIB" "$A_ORIENT")
assert_eq "B6: aucun bloc conditionnel n'est ouvert entre la borne et le site" \
    "0" "$(sed -n "$((B6_FROM + 1)),$((B6_TO - 1))p" "$DISPATCH_LIB" \
        | grep -vE '^[[:space:]]*#' \
        | grep -cE '^[[:space:]]*(if|case|while|for|until)[[:space:]]' || true)"

# ============================================================================
# PART C — anti-vacuité : B1 sait-elle dire non ?
# ============================================================================
echo ""
echo "PART C — B1 sur une copie régressée : elle doit échouer"
echo "--------------------------------------------------------"

REGRESSED="$SCRATCH/dispatch-lib-regressed.sh"

# La régression exacte que l'AC4 nomme : l'injection est DÉPLACÉE après
# `TICKET_CONTEXT`. Déplacée, pas supprimée — c'est la différence entre « l'ordre
# est faux » et « le bloc est absent », et C2 ci-dessous le prouve. Construite
# par numéros de ligne plutôt que par motif : un `awk` à motifs réintroduit
# exactement le risque que C0 doit écarter.
C_HELD=$(sed -n "${B6_TO}p" "$DISPATCH_LIB")
C_TICKET_LINE=$(line_of "$DISPATCH_LIB" "$A_TICKET")
{
    sed -n "1,$((B6_TO - 1))p" "$DISPATCH_LIB"
    sed -n "$((B6_TO + 1)),${C_TICKET_LINE}p" "$DISPATCH_LIB"
    printf '%s\n' "$C_HELD"
    sed -n "$((C_TICKET_LINE + 1)),\$p" "$DISPATCH_LIB"
} > "$REGRESSED"

# La régression a-t-elle MORDU ? Si la construction dérive, la copie redevient
# l'original et l'« échec » attendu en C1 serait un succès déguisé en preuve.
rc=1; ! cmp -s "$DISPATCH_LIB" "$REGRESSED" && rc=0
assert_true "C0: la copie régressée diffère de l'original (régression effective)" "$rc"

assert_eq "C1: B1 ÉCHOUE sur la copie régressée (ordre inversé)" \
    "ko" "$(order_verdict "$REGRESSED" | cut -d: -f1)"

# C2 — l'échec de C1 vient bien d'un DÉPLACEMENT et non d'une disparition. Sans
# elle, une régression qui supprime la ligne rendrait C1 vert pour la mauvaise
# raison, et B1 resterait non éprouvée.
assert_eq "C2: l'injection est toujours présente une fois, mais mal placée" \
    "1" "$(code_lines "$REGRESSED" | grep -cF -- "$A_ORIENT" || true)"

# C3 — la copie régressée reste du bash valide. Une copie syntaxiquement cassée
# ferait échouer C1 par accident, pour une cause qui n'est pas l'ordre.
rc=1; bash -n "$REGRESSED" 2>/dev/null && rc=0
assert_true "C3: la copie régressée est du bash valide (l'échec vient de l'ordre)" "$rc"

# ============================================================================
echo ""
echo "===================================================="
echo "Results: $PASS passed, $FAIL failed"
echo "===================================================="

[ "$FAIL" -eq 0 ] || exit 1
exit 0
