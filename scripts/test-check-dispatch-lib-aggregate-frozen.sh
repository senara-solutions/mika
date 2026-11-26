#!/usr/bin/env bash
#
# Anti-vacuity harness for scripts/check-dispatch-lib-aggregate-frozen.sh (mika#3179).
#
# "Fausse le contrôle ; confirme que le cas rougit."
#
# Le garde compare une seule donnée — `wc -c` de
# `skills/bundled/_shared/test-dispatch-lib.sh` — à la constante
# `AGGREGATE_FROZEN_BYTES` qu'il porte en dur. Les quatre cas du ticket sont
# C1 à C4 ci-dessous, chacun sur une COPIE temporaire : ce harnais ne touche
# jamais le vrai agrégat, qu'il lit en lecture seule.
#
# UN PLAFOND STATIQUE PASSE VERT POUR DEUX MAUVAISES RAISONS, et les deux
# contrôles qui restent sont là pour ça :
#
#   P — population. La constante est extraite du garde par `sed` ancré en
#       colonne zéro, puis comparée à la taille réelle de l'agrégat. Égalité
#       exigée. Sans ce contrôle, C1 (« la copie telle quelle est verte ») est
#       une tautologie : une constante dérivée vers le haut le laisse passer
#       tout en ne refusant plus rien.
#   D — dérive. Personne ne peut ajouter un octet à l'agrégat, mais n'importe
#       qui peut RELEVER la constante. D la borne par le `LARGE_FILE_LIMIT` que
#       `scripts/check-secrets.sh` déclare — lu par `sed`, JAMAIS sourcé, sur le
#       motif de `check-pilot-turn-ceiling-labels.sh` qui parse `dispatch-lib.sh`
#       sans l'exécuter. Une constante montée jusqu'au plafond rend le gel sans
#       objet, puisque `Secret Scan` rougirait d'abord. `check-secrets.sh` est
#       lu et non modifié : c'est sa MODIFICATION qui est hors périmètre.
#
# Sorties : 0 tout vert, 1 un cas rouge, 3 vacuité (une des deux constantes
# n'est plus extractible, ou la constante a dérivé de la taille réelle). Un
# rouge sur un renommage de l'une des deux variables est le lockstep voulu, pas
# un faux positif.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
GUARD="$REPO_ROOT/scripts/check-dispatch-lib-aggregate-frozen.sh"
AGGREGATE_REAL="$REPO_ROOT/skills/bundled/_shared/test-dispatch-lib.sh"
SECRETS_SCAN="$REPO_ROOT/scripts/check-secrets.sh"

PASS=0
FAIL=0
VACUOUS=0

assert_exit() {
    local path="$1" want="$2" name="$3"
    local got=0
    bash "$GUARD" "$path" >/dev/null 2>&1 || got=$?
    if [ "$got" -eq "$want" ]; then
        echo "PASS: $name (exit $got)"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (wanted exit $want, got $got)"
        FAIL=$((FAIL + 1))
    fi
}

assert_output_contains() {
    local path="$1" needle="$2" name="$3"
    local out
    out="$(bash "$GUARD" "$path" 2>&1 || true)"
    if [[ "$out" == *"$needle"* ]]; then
        echo "PASS: $name"
        PASS=$((PASS + 1))
    else
        echo "FAIL: $name (output did not contain: $needle)"
        FAIL=$((FAIL + 1))
    fi
}

vacuous() {
    echo "VACUOUS: $1"
    VACUOUS=$((VACUOUS + 1))
}

# Une seule racine de fixtures, retirée par un `trap`. Les copies pèsent ~1 Mo
# chacune : C2 et C3 DÉRIVENT de la copie de C1 au lieu de recopier le vrai
# fichier quatre fois.
FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT

if [ ! -r "$AGGREGATE_REAL" ]; then
    vacuous "l'agrégat $AGGREGATE_REAL est illisible — les quatre cas seraient bâtis sur rien"
    echo ""
    echo "check-dispatch-lib-aggregate-frozen anti-vacuity: $PASS passed, $FAIL failed, $VACUOUS vacuous"
    exit 3
fi

REAL_SIZE="$(wc -c < "$AGGREGATE_REAL" | tr -d '[:space:]')"

# ── Les deux constantes, extraites et jamais sourcées. Ancrage en colonne zéro :
#    une ligne de commentaire citant le nom de la variable ne doit pas répondre à
#    la place de la déclaration.
FROZEN="$(sed -n 's/^AGGREGATE_FROZEN_BYTES=\([0-9][0-9]*\).*/\1/p' "$GUARD" | tr -d '[:space:]')"
LIMIT="$(sed -n 's/^LARGE_FILE_LIMIT=\([0-9][0-9]*\).*/\1/p' "$SECRETS_SCAN" | tr -d '[:space:]')"

# ── C1 — TAILLE ÉGALE. La copie telle quelle du vrai agrégat est verte, et le
#    message vert nomme la mesure ET la constante (un 0 muet ne dit pas lequel
#    des deux verts il a rendu — la panne silencieuse de mika#2578).
C1="$FIXTURE_ROOT/aggregate-equal.sh"
cp "$AGGREGATE_REAL" "$C1"
assert_exit "$C1" 0 "C1 — une copie de la taille gelée est verte"
assert_output_contains "$C1" "Aggregate frozen:" "C1 — le message vert annonce la mesure"

# ── C2 — UN OCTET DE PLUS. Le rouge que ce lint existe pour rendre. La copie
#    est fabriquée ici, par ce harnais, à partir de celle de C1.
C2="$FIXTURE_ROOT/aggregate-plus-one.sh"
cp "$C1" "$C2"
printf '#' >> "$C2"
C2_SIZE="$(wc -c < "$C2" | tr -d '[:space:]')"
if [ "$C2_SIZE" -ne "$((REAL_SIZE + 1))" ]; then
    vacuous "la fixture +1 octet pèse $C2_SIZE au lieu de $((REAL_SIZE + 1)) — C2 ne mesure pas ce qu'il dit"
else
    assert_exit "$C2" 1 "C2 — un octet de plus est refusé"
    assert_output_contains "$C2" "$C2_SIZE octets" "C2 — l'échec nomme la taille mesurée"
    assert_output_contains "$C2" "#3178" "C2 — l'échec nomme #3178, la seule sortie légitime du plafond"
    assert_output_contains "$C2" "skills/bundled/_shared/tests/" \
        "C2 — l'échec nomme le chemin des suites neuves"
    assert_output_contains "$C2" "test_callout_plan_identity_3174.sh" \
        "C2 — l'échec nomme la suite modèle"
    assert_output_contains "$C2" "test-dispatch-lib" "C2 — l'échec nomme la cible que la CI appelle"
    assert_output_contains "$C2" "LARGE_FILE_ALLOWLIST" "C2 — l'échec nomme le NON-geste"
    if [ -n "$FROZEN" ]; then
        assert_output_contains "$C2" "$FROZEN" "C2 — l'échec nomme la constante ($FROZEN)"
    else
        vacuous "AGGREGATE_FROZEN_BYTES inextractible — l'assertion de constante de C2 ne mesure rien"
    fi
fi

# ── C3 — FICHIER PLUS PETIT. Le prédicat est ≤, pas == : la scission #3178
#    rétrécira l'agrégat avant de le supprimer, et ce chemin-là est vert.
C3="$FIXTURE_ROOT/aggregate-smaller.sh"
head -c "$((REAL_SIZE - 1024))" "$C1" > "$C3"
assert_exit "$C3" 0 "C3 — un fichier plus petit est vert"

# ── C4 — FICHIER ABSENT. #3178 a pu le supprimer : le lint n'a plus rien à
#    mesurer et doit le DIRE, en nommant son propre retrait.
C4="$FIXTURE_ROOT/aggregate-does-not-exist.sh"
assert_exit "$C4" 0 "C4 — un agrégat absent est vert"
assert_output_contains "$C4" "Aggregate absent:" "C4 — le message vert nomme la branche « absent »"
assert_output_contains "$C4" "#3178" "C4 — le vert « absent » renvoie à #3178"

# ── P — POPULATION. C'est ce contrôle qui fait de C1 un vrai vert. Il mesure
#    aussi, à chaque run de CI, que la population d'exceptions est VIDE : la
#    constante vaut la taille réelle, donc l'unique objet surveillé est conforme
#    sans dérogation (mika#2201 — « on déclare, on n'allowliste pas »).
if [[ ! "$FROZEN" =~ ^[0-9]+$ ]] || [ "$FROZEN" -eq 0 ]; then
    vacuous "P — AGGREGATE_FROZEN_BYTES inextractible de $GUARD (lu : '$FROZEN')"
elif [ "$FROZEN" -ne "$REAL_SIZE" ]; then
    vacuous "P — la constante ($FROZEN) a dérivé de la taille réelle de l'agrégat ($REAL_SIZE) : C1 passerait vert sur une tautologie"
else
    echo "PASS: P — la constante gelée ($FROZEN) vaut la taille réelle de l'agrégat"
    PASS=$((PASS + 1))
fi

# ── D — DÉRIVE. Le seul mode de défaillance que le plafond laisse ouvert :
#    relever la constante. D la borne par le plafond de `check-secrets.sh`.
if [[ ! "$LIMIT" =~ ^[0-9]+$ ]] || [ "$LIMIT" -eq 0 ]; then
    vacuous "D — LARGE_FILE_LIMIT inextractible de $SECRETS_SCAN (lu : '$LIMIT')"
elif [[ ! "$FROZEN" =~ ^[0-9]+$ ]] || [ "$FROZEN" -eq 0 ]; then
    vacuous "D — AGGREGATE_FROZEN_BYTES inextractible : la borne de dérive ne mesure rien"
elif [ "$FROZEN" -ge "$LIMIT" ]; then
    echo "FAIL: D — la constante gelée ($FROZEN) atteint le LARGE_FILE_LIMIT de check-secrets.sh ($LIMIT) :"
    echo "      à ce point le gel ne protège plus de rien, puisque \`Secret Scan\` rougirait d'abord."
    echo "      La sortie est #3178 (scinder l'agrégat), pas une constante relevée."
    FAIL=$((FAIL + 1))
else
    echo "PASS: D — la constante gelée ($FROZEN) reste sous le LARGE_FILE_LIMIT de check-secrets.sh ($LIMIT), marge $((LIMIT - FROZEN))"
    PASS=$((PASS + 1))
fi

echo ""
echo "check-dispatch-lib-aggregate-frozen anti-vacuity: $PASS passed, $FAIL failed, $VACUOUS vacuous"
[ "$VACUOUS" -eq 0 ] || exit 3
[ "$FAIL" -eq 0 ] || exit 1
