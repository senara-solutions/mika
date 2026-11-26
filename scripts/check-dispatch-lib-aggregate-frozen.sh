#!/usr/bin/env bash
# CI lint: `skills/bundled/_shared/test-dispatch-lib.sh` ne grossit plus.
#
# LE FAIT MESURÉ. L'agrégat pèse 1 041 743 octets. Le plafond `LARGE_FILE_LIMIT`
# de `scripts/check-secrets.sh` est de 1 048 576 octets (1 Mo) : il reste moins de
# 7 Ko de marge, et le fichier a pris ~200 Ko en un mois.
#
# L'INCIDENT FONDATEUR est #3177. Un bloc de 238 lignes ajouté à l'agrégat a fait
# rougir le job `Secret Scan` (run 45856244085) APRÈS le push. Le pilote n'avait
# aucun moyen de l'apprendre avant la CI, et une itération entière a servi à
# déplacer le bloc dans `skills/bundled/_shared/tests/test_callout_plan_identity_3174.sh`.
#
# POURQUOI LE MESSAGE DE `check-secrets.sh` NE SUFFIT PAS. Il est écrit pour un
# fixture binaire, et il dit :
#   ERROR: <f> is 1028KB — exceeds 1MB limit (add to LARGE_FILE_ALLOWLIST ...)
# Lu par un pilote, il suggère l'EXACT CONTRAIRE de ce qu'il faut faire : une
# entrée en allowlist, la PR passe, l'agrégat continue de croître, et #3178
# devient d'autant plus coûteuse. C'est ce coût-là qui justifie un lint dédié,
# plus que le gain de latence.
#
# ON NE RELÈVE PAS LA CONSTANTE. Le geste correct est une suite neuve sous
# `skills/bundled/_shared/tests/`, un maillon par fichier. La seule sortie
# légitime de ce plafond est la scission complète de l'agrégat, #3178, ouverte en
# dormeur en attente de deux décisions de l'opérateur.
#
# UN PLAFOND STATIQUE, PAS UN DIFF CONTRE LA BASE. Un diff contre `origin/main`
# mesurerait la croissance introduite par la PR, mais dépendrait de la profondeur
# du checkout et de la présence de la ref : muet en local sur une branche fraîche,
# muet dans `make test` hors d'un clone complet. Une taille absolue est une seule
# donnée sans dépendance — la même mesure en local, dans `make test` et en CI.
#
# Usage: check-dispatch-lib-aggregate-frozen.sh [aggregate-path]
#   Le défaut est le vrai fichier. L'argument n'existe QUE pour que le harnais
#   anti-vacuité pointe le garde vers une copie temporaire
#   (scripts/test-check-dispatch-lib-aggregate-frozen.sh). La constante reste
#   interne : si le harnais pouvait la passer, le cas vert ne mesurerait plus la
#   vraie constante et deviendrait une tautologie.
#
# Exit 0 si la taille est ≤ la constante, 1 si elle la dépasse, 0 si le fichier
# n'existe plus (#3178 a pu le supprimer — le message vert DIT que c'est cette
# branche-là qui a été prise, parce qu'un 0 muet est un vert sur un fichier
# renommé, la panne silencieuse de mika#2578), 2 si le fichier est là mais que la
# mesure est impossible. Un garde qui ne peut pas mesurer ne passe pas
# (docs/solutions/best-practices/structural-guard-fails-open-parser-fixture-harness.md).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
AGGREGATE="${1:-$REPO_ROOT/skills/bundled/_shared/test-dispatch-lib.sh}"

# La taille de l'agrégat au dernier commit de mika#3179. Ancrée en colonne zéro :
# le contrôle P du harnais l'extrait par `sed` sur `^AGGREGATE_FROZEN_BYTES=`.
AGGREGATE_FROZEN_BYTES=1041743

TESTS_DIR="skills/bundled/_shared/tests"
MODEL_SUITE="$TESTS_DIR/test_callout_plan_identity_3174.sh"

# ── La branche « absent ». #3178 a pu scinder puis supprimer l'agrégat : à ce
#    moment-là ce lint n'a plus rien à mesurer, et il doit le dire au lieu de
#    rendre un 0 indistinguable du cas nominal.
if [[ ! -e "$AGGREGATE" ]]; then
    echo "Aggregate absent: $AGGREGATE"
    echo "Rien à mesurer. Si c'est #3178 qui a scindé puis supprimé l'agrégat, ce lint"
    echo "n'a plus d'objet : retirez \`scripts/check-dispatch-lib-aggregate-frozen.sh\`,"
    echo "son harnais, et leurs lignes des cibles \`test\` et \`test-dispatch-lib\` du"
    echo "Makefile. Si le fichier a seulement été RENOMMÉ, ce vert est faux : pointez le"
    echo "garde sur le nouveau chemin dans le même commit."
    exit 0
fi

# ── La mesure. Un fichier présent mais illisible, un répertoire, ou un `wc` dont
#    la sortie n'est pas un entier : le garde sort en 2, jamais en 0.
if [[ ! -f "$AGGREGATE" || ! -r "$AGGREGATE" ]]; then
    echo "ERROR: aggregate present but not a readable regular file: $AGGREGATE" >&2
    echo "Un garde qui ne peut pas mesurer ne rend pas un vert." >&2
    exit 2
fi

MEASURED=""
MEASURED="$(wc -c < "$AGGREGATE" 2>/dev/null | tr -d '[:space:]')" || MEASURED=""
if [[ ! "$MEASURED" =~ ^[0-9]+$ ]] || [[ "$MEASURED" -eq 0 ]]; then
    echo "ERROR: could not measure $AGGREGATE (wc -c returned: '${MEASURED}')" >&2
    echo "Un garde qui ne peut pas mesurer ne rend pas un vert." >&2
    exit 2
fi

# ── La violation que ce lint existe pour refuser.
if [[ "$MEASURED" -gt "$AGGREGATE_FROZEN_BYTES" ]]; then
    DELTA=$((MEASURED - AGGREGATE_FROZEN_BYTES))
    echo "ERROR: l'agrégat a grossi de $DELTA octet(s)." >&2
    echo "  fichier  : $AGGREGATE" >&2
    echo "  mesuré   : $MEASURED octets" >&2
    echo "  gelé à   : $AGGREGATE_FROZEN_BYTES octets (AGGREGATE_FROZEN_BYTES, mika#3179)" >&2
    echo "" >&2
    echo "LE GESTE — votre bloc d'assertions va dans une suite NEUVE :" >&2
    echo "  $TESTS_DIR/test_<sujet>_<ticket>.sh" >&2
    echo "sur le modèle de \`$MODEL_SUITE\`, dont" >&2
    echo "l'en-tête explique pourquoi elle ne vit pas dans l'agrégat. Puis câblez-la" >&2
    echo "dans LES DEUX cibles du Makefile : \`test-dispatch-lib\` (celle que la CI" >&2
    echo "nomme, ci.yml) et \`test\`. Une suite câblée dans \`test\` seulement n'est" >&2
    echo "pas une porte de merge (mika#2578)." >&2
    echo "" >&2
    echo "LE NON-GESTE — ne relevez pas AGGREGATE_FROZEN_BYTES, et n'ajoutez pas" >&2
    echo "d'entrée à LARGE_FILE_ALLOWLIST de \`scripts/check-secrets.sh\`. Le message de" >&2
    echo "ce scan suggère l'allowlist parce qu'il est écrit pour un fixture binaire ;" >&2
    echo "l'appliquer ici laisse l'agrégat croître jusqu'au plafond de 1 048 576 octets." >&2
    echo "" >&2
    echo "La SEULE sortie légitime de ce plafond est #3178, la scission de l'agrégat." >&2
    exit 1
fi

MARGIN=$((AGGREGATE_FROZEN_BYTES - MEASURED))
echo "Aggregate frozen: $MEASURED octets ≤ $AGGREGATE_FROZEN_BYTES (AGGREGATE_FROZEN_BYTES), marge $MARGIN."
exit 0
