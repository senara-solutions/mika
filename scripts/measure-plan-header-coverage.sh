#!/usr/bin/env bash
# mika#2606 — quel volume de nos plans le lecteur d'en-tête lit-il réellement ?
#
# `_plan_header_claimed_issues` (dispatch-lib.sh) alimente la garde de
# réfutation de mika#2038 : un plan dont l'en-tête réclame un AUTRE ticket que
# la cible est écarté du tier 1 de `_find_issue_plan`. Quand le lecteur ne voit
# pas l'en-tête, la garde ne réfute rien — elle n'échoue pas, elle est muette.
# C'est la classe mika#2205 : une garde que personne n'a exercée se lit
# exactement comme une garde qui marche.
#
# Cet instrument SOURCE dispatch-lib.sh et appelle la vraie fonction. Il n'en
# redéfinit aucune copie : une sonde qui reporte son propre motif divergerait du
# code au premier changement, et c'est très exactement le défaut qu'elle mesure
# (la leçon que mika#2293 a dû épingler pour sa reconstruction de cascade).
#
# CE N'EST PAS UN DÉTECTEUR. Sortie de rapport, toujours exit 0, jamais câblé à
# la CI — il asserterait une propriété d'un corpus de documents en croissance,
# pas une propriété du code. Le refus est raisonné en R6 du plan mika#2606 ; la
# régression de code est couverte par les fixtures de tests/test_find_issue_plan.sh.
#
# Usage:
#   scripts/measure-plan-header-coverage.sh                        # docs/plans/*.md
#   scripts/measure-plan-header-coverage.sh 'docs/plans/2026-09-*' # motif entre quotes
#   scripts/measure-plan-header-coverage.sh a.md b.md              # chemins explicites

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DISPATCH_LIB="$REPO_ROOT/skills/bundled/_shared/dispatch-lib.sh"

if [ ! -r "$DISPATCH_LIB" ]; then
    echo "measure-plan-header-coverage: dispatch-lib.sh introuvable ($DISPATCH_LIB)" >&2
    exit 0
fi

# shellcheck source=skills/bundled/_shared/dispatch-lib.sh
source "$DISPATCH_LIB"

# Un argument est pris comme chemin quand il existe, comme motif sinon — de
# sorte que la commande marche que le shell appelant ait développé le glob ou
# l'ait passé entre quotes.
PLANS=()
if [ "$#" -eq 0 ]; then
    set -- "$REPO_ROOT/docs/plans/*.md"
fi
for arg in "$@"; do
    if [ -f "$arg" ]; then
        PLANS+=("$arg")
        continue
    fi
    case "$arg" in
        /*) pattern="$arg" ;;
        *)  pattern="$REPO_ROOT/$arg" ;;
    esac
    for hit in $pattern; do
        [ -f "$hit" ] && PLANS+=("$hit")
    done
done

if [ "${#PLANS[@]}" -eq 0 ]; then
    echo "measure-plan-header-coverage: aucun plan ne correspond à: $*"
    exit 0
fi

READ_COUNT=0
UNREAD_COUNT=0
UNREAD_WITH_LABEL_WORD=0
UNREAD_REPORT=""

for plan in "${PLANS[@]}"; do
    claimed=$(_plan_header_claimed_issues "$plan" | tr '\n' ' ')
    claimed="${claimed% }"
    if [ -n "$claimed" ]; then
        READ_COUNT=$((READ_COUNT + 1))
        continue
    fi
    UNREAD_COUNT=$((UNREAD_COUNT + 1))

    # Le discriminant de M6 : le plan porte-t-il malgré tout une ligne qui
    # COMMENCE par un mot de label ? Ancré comme le lecteur, mais agnostique du
    # séparateur — c'est ce qui sépare « aucun marqueur », la population
    # fail-open légitime de mika#2038, de « un en-tête que le lecteur n'atteint
    # pas », qui est le défaut. Une recherche non ancrée ne trancherait rien :
    # mesurée, elle rend 122 des 126 non-lus, parce que la prose de nos plans
    # dit « ticket » à longueur de paragraphe.
    label_words=$(head -n 20 "$plan" 2>/dev/null \
        | grep -inE '^[[:space:]]*(>[[:space:]]*)*(-[[:space:]]+)?(\*\*)?(ticket|issue|number)' || true)
    UNREAD_REPORT="${UNREAD_REPORT}
  ${plan#"$REPO_ROOT"/}"
    if [ -n "$label_words" ]; then
        UNREAD_WITH_LABEL_WORD=$((UNREAD_WITH_LABEL_WORD + 1))
        while IFS= read -r line; do
            UNREAD_REPORT="${UNREAD_REPORT}
      | ${line}"
        done <<< "$label_words"
    else
        UNREAD_REPORT="${UNREAD_REPORT}
      | (aucun mot de label dans les 20 premières lignes)"
    fi
done

TOTAL=${#PLANS[@]}
echo "mika#2606 — couverture du lecteur d'en-tête de plan"
echo "  motif ................................ $*"
echo "  plans examinés ....................... $TOTAL"
echo "  en-tête LU (réclame ≥ 1 numéro) ...... $READ_COUNT"
echo "  en-tête NON LU (réclame rien) ........ $UNREAD_COUNT"
echo "    dont porteurs d'une ligne à label .. $UNREAD_WITH_LABEL_WORD"
echo "    dont sans aucune ligne à label ..... $((UNREAD_COUNT - UNREAD_WITH_LABEL_WORD))"

if [ "$UNREAD_COUNT" -gt 0 ]; then
    echo
    echo "Plans dont le lecteur ne réclame rien. Les lignes citées commencent par"
    echo "un mot de label (ancré, séparateur agnostique) : un plan qui n'en porte"
    echo "aucune est la population fail-open légitime de mika#2038 ; un plan qui"
    echo "en porte une et n'est pas lu est le défaut, ou de la prose enroulée."
    printf '%s\n' "$UNREAD_REPORT"
fi

# Instrument, pas détecteur : toujours 0.
exit 0
