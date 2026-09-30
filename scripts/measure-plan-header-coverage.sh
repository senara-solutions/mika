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
# Cet instrument SOURCE dispatch-lib.sh et appelle la vraie fonction : le compte
# « lu / non lu » ne peut donc pas diverger du code, ce qui est tout l'intérêt
# (la leçon que mika#2293 a dû épingler pour sa reconstruction de cascade).
#
# EN REVANCHE le discriminant du bas — « ce plan porte-t-il malgré tout une ligne
# à label ? » — est un motif SÉPARÉ et délibérément plus large que celui de la
# fonction : c'est ce qui permet de voir une forme que la fonction n'atteint pas.
# Il ne peut pas être partagé, `hdr` étant un `local`. Prix à connaître : si la
# fonction gagne un préfixe que ce motif-ci ignore (une puce `* **Ticket :**`, par
# exemple, qu'aucun des deux ne lit aujourd'hui), un vrai trou de lecteur serait
# rapporté sous « sans aucune ligne à label », le seau qui veut dire « sain ».
# En toucher un, c'est devoir toucher l'autre.
#
# CE N'EST PAS UN DÉTECTEUR. Sortie de rapport, toujours exit 0, jamais câblé à
# la CI — il asserterait une propriété d'un corpus de documents en croissance,
# pas une propriété du code. Le refus est raisonné en R6 du plan mika#2606 ; la
# régression de code est couverte par les fixtures de tests/test_find_issue_plan.sh.
#
# Le défaut est `docs/plans/*-plan.md` et non `*.md` : c'est la population que
# `_find_issue_plan` globe réellement à ses trois tiers. Mesuré, `*.md` ajoute 26
# fichiers que le lecteur n'ouvre jamais, crédite 12 « en-têtes lus » et gonfle de
# 14 le seau même que le rapport appelle fail-open.
#
# Usage:
#   bash scripts/measure-plan-header-coverage.sh                        # docs/plans/*-plan.md
#   bash scripts/measure-plan-header-coverage.sh 'docs/plans/2026-09-*' # motif entre quotes
#   bash scripts/measure-plan-header-coverage.sh a.md b.md              # chemins explicites

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
    set -- "$REPO_ROOT/docs/plans/*-plan.md"
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
UNREAD_WITH_LABEL_LINE=0
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
    #
    # C'est la copie séparée dont l'en-tête de ce fichier nomme le prix. Elle est
    # PLUS LARGE que le motif de la fonction (aucune exigence de deux-points,
    # citation non bornée) : c'est délibéré, un discriminant plus étroit que le
    # lecteur ne pourrait jamais montrer une forme non lue.
    label_lines=$(head -n 20 "$plan" 2>/dev/null \
        | grep -inE '^[[:space:]]*(>[[:space:]]*)*(-[[:space:]]+)?(\*\*)?(ticket|issue|number)' || true)
    UNREAD_REPORT="${UNREAD_REPORT}
  ${plan#"$REPO_ROOT"/}"
    if [ -n "$label_lines" ]; then
        UNREAD_WITH_LABEL_LINE=$((UNREAD_WITH_LABEL_LINE + 1))
        while IFS= read -r line; do
            UNREAD_REPORT="${UNREAD_REPORT}
      | ${line}"
        done <<< "$label_lines"
    else
        UNREAD_REPORT="${UNREAD_REPORT}
      | (aucune ligne à label dans les 20 premières lignes)"
    fi
done

TOTAL=${#PLANS[@]}
echo "mika#2606 — couverture du lecteur d'en-tête de plan"
echo "  motif ................................ $*"
echo "  plans examinés ....................... $TOTAL"
echo "  en-tête LU (réclame ≥ 1 numéro) ...... $READ_COUNT"
echo "  en-tête NON LU (réclame rien) ........ $UNREAD_COUNT"
echo "    dont porteurs d'une ligne à label .. $UNREAD_WITH_LABEL_LINE"
echo "    dont sans aucune ligne à label ..... $((UNREAD_COUNT - UNREAD_WITH_LABEL_LINE))"

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
