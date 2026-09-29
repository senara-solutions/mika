#!/usr/bin/env bash
# CI lint (mika#2408) — lockstep bidirectionnel manifeste↔code sur les sinks
# réseau sortants. Un chemin d'egress neuf ne peut plus apparaître sans qu'une
# ligne humainement relisible dise quelle donnée part, vers où, et si elle est
# journalisée — et une telle ligne ne peut pas être écrite pour un sink qui
# n'existe pas.
#
# Ce que les trois lints egress voisins NE font pas, et pourquoi celui-ci
# existe : ils vérifient des propriétés de sinks DÉJÀ décidés — le confinement
# d'un host connu (`verify-egress-uniqueness.sh`), la forme de la requête Brave
# (`verify-egress-request-shape.sh`), l'absence de journalisation dans le
# substrat de recherche (`verify-egress-no-log.sh`). Aucun ne tire quand une
# NOUVELLE destination est ajoutée. Si Brave avait été ajouté comme un simple
# appel reqwest vers un host neuf — la forme qu'a Telegram — zéro check
# n'aurait tiré.
#
# Discipline analog : `scripts/verify-egress-uniqueness.sh` (mika#1807) —
# construire l'incapacité, ne pas promettre la retenue. Même forme que
# `scripts/check-byte-slices.sh` (#764) et `scripts/check-loop-select.sh`
# (#848).
#
# Le moteur (inventaire mécanique + les quatre directions) vit dans
# `scripts/lib/egress_manifest_lint.py`. Bash pour l'orchestration, python3
# pour le parsing TOML (`tomllib`, stdlib ≥ 3.11) : parser du TOML à la main en
# bash est exactement la fragilité que `check-pilot-turn-ceiling-labels.sh` a dû
# border par un exit 3 « forme non auditable », et on ne la réintroduit pas.
#
# Usage :
#   bash scripts/verify-egress-manifest.sh [--report] [<racine>] [<manifeste>]
#
# L'argument de racine optionnel suit le modèle de `verify-egress-no-log.sh` :
# c'est ce qui rend le test négatif (`scripts/test-verify-egress-manifest.sh`)
# possible sans jamais toucher l'arbre vivant.
#
# Codes de sortie :
#   0 — aucune violation
#   1 — violation(s) trouvée(s)
#   2 — manifeste illisible, absent ou vide, ou inventaire vide (fail-closed)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENGINE="$REPO_ROOT/scripts/lib/egress_manifest_lint.py"

if [[ ! -f "$ENGINE" ]]; then
    echo "ERROR (egress-manifest): moteur introuvable à $ENGINE" >&2
    echo "  Le lint ne peut rien confronter. Un lint absent qui sortirait 0 se" >&2
    echo "  lirait exactement comme un arbre propre (classe mika#2205)." >&2
    exit 2
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR (egress-manifest): python3 introuvable" >&2
    echo "  Requis pour lire le manifeste TOML (tomllib, stdlib >= 3.11)." >&2
    exit 2
fi

# Aucun argument : la racine du dépôt. `python3 -B` — pas de __pycache__ dans
# l'arbre, même forme que les deux autres jobs CI qui lancent du python.
ARGS=()
SAW_PATH=0
for arg in "$@"; do
    if [[ "$arg" == --* ]]; then
        ARGS+=("$arg")
    else
        ARGS+=("$arg")
        SAW_PATH=1
    fi
done
if [[ $SAW_PATH -eq 0 ]]; then
    ARGS+=("$REPO_ROOT")
fi

exec python3 -B "$ENGINE" "${ARGS[@]}"
