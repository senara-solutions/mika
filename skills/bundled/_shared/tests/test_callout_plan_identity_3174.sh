#!/bin/bash
# Suite d'assertions pour l'identité du plan dans le court-circuit d'idempotence
# de `_write_canonical_callout` (mika#3174).
#
# Incident fondateur (2026-11-26, vol 2 de mika#3118) : le groom converge, sort en
# `Outcome: PLAN_GROOMED`, et le corps du ticket ne reçoit aucun callout. Le corps
# portait encore le callout COMPLET du vol 1 — même branche, plan `2026-11-22-002`,
# marqueur `second-pass (GROOMED)` — donc les trois signaux de la porte de dispatch
# étaient présents, et le writer a conclu « déjà estampillé » sans jamais regarder
# QUEL plan le corps nommait.
#
# POURQUOI CETTE SUITE VIT ICI ET PAS DANS `test-dispatch-lib.sh` (mika#3174, vol 2) :
# le fichier agrégat frôle le plafond `LARGE_FILE_LIMIT` (1 048 576 octets) de
# `scripts/check-secrets.sh`. Y ajouter ce bloc l'a fait passer à 1 052 961 octets
# et le job `Secret Scan` a rougi sur une taille, pas sur un secret. Les suites de
# `tests/` existent précisément pour ça : un maillon, un fichier.
#
# Run: bash skills/bundled/_shared/tests/test_callout_plan_identity_3174.sh
# Expected: all assertions pass, exit 0.

set -euo pipefail

# Hermeticité (mika#1772, recopiée de `test-dispatch-lib.sh`) : la sonde ci-dessous
# fait `git init` + `git commit` dans un répertoire temporaire et hériterait de la
# config git de l'hôte. Sur une machine dont la config globale pose
# `commit.gpgsign = true`, la suite s'arrête en 128. `init.defaultBranch` est le
# second couplage : les fixtures `git init` puis référencent `main`.
#
# On surcharge les clés de signature SEULEMENT. Annuler `GIT_CONFIG_GLOBAL` à la
# place retirerait l'identité du committer dont la sonde a besoin.
export GIT_CONFIG_COUNT=5
export GIT_CONFIG_KEY_3=user.name
export GIT_CONFIG_VALUE_3=dispatch-lib test suite
export GIT_CONFIG_KEY_4=user.email
export GIT_CONFIG_VALUE_4=test@localhost
export GIT_CONFIG_KEY_0=commit.gpgsign
export GIT_CONFIG_VALUE_0=false
export GIT_CONFIG_KEY_1=tag.gpgsign
export GIT_CONFIG_VALUE_1=false
export GIT_CONFIG_KEY_2=init.defaultBranch
export GIT_CONFIG_VALUE_2=main

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DISPATCH_LIB="$SCRIPT_DIR/../dispatch-lib.sh"

PASS=0
FAIL=0
# mika#2149: une troisième colonne, pour qu'une garde qui n'a pas pu s'armer ne se
# lise jamais comme un vert. Incrémentée seulement par une sonde qui dit SKIP.
SKIPPED=0

# Le binaire `mika`, résolu par la bibliothèque PARTAGÉE — pas recopiée (mika#2194).
# Depuis mika#2194 `_extract_plan_path` invoque `mika` par son nom, et le prédicat
# ajouté par mika#3174 lui délègue : sans le binaire ce bloc ne mesure rien.
# shellcheck source=skills/bundled/_shared/tests/lib-mika-bin.sh
source "$SCRIPT_DIR/lib-mika-bin.sh"

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected: $expected"
        echo "    actual:   $actual"
    fi
}

assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected to contain: $needle"
        echo "    actual: $haystack"
    fi
}

assert_not_contains() {
    local label="$1" needle="$2" haystack="$3"
    if ! grep -qF -- "$needle" <<<"$haystack"; then
        PASS=$((PASS + 1))
        echo "  ✓ $label"
    else
        FAIL=$((FAIL + 1))
        echo "  ✗ $label"
        echo "    expected NOT to contain: $needle"
        echo "    actual: $haystack"
    fi
}

# Source la lib pour rendre les helpers appelables. `dispatch-lib.sh` fait
# `set -euo pipefail` en tête ; le sourcer le pose aussi pour cette suite — ce qui
# convient, puisqu'on met `|| true` partout où un non-zéro est attendu.
# shellcheck disable=SC1090
source "$DISPATCH_LIB" 2>/dev/null || true

# Source-shape : la source du writer, lue par le volet T4.
WRITER_SRC=$(declare -f _write_canonical_callout)

echo ""
echo "mika#3174 — le court-circuit d'idempotence regarde QUEL plan le corps nomme"
echo "---------------------------------------------------------------------------"

# ----------------------------------------------------------------------------
# mika#3174 — le court-circuit d'idempotence regarde QUEL plan le corps nomme
# ----------------------------------------------------------------------------
# Les assertions de forme de source de `test-dispatch-lib.sh` ne peuvent PAS voir
# ce défaut : le court-circuit est bien présent, c'est sa *question* qui est trop
# faible. Au vol 2 de mika#3118 le corps portait encore le callout COMPLET du
# vol 1 — même branche, plan `2026-11-22-002`, marqueur `second-pass (GROOMED)` —
# donc les trois signaux de la porte étaient là, et le writer a conclu « déjà
# estampillé » sans jamais regarder que le plan nommé n'était pas celui que ce
# groom venait de valider. Aucun callout n'a été écrit.
#
# Les deux cas qui suivent sont donc COMPORTEMENTAUX : on exécute la fonction sur
# un vrai dépôt, avec `gh` mocké, et on lit le corps qu'elle a produit.
T3174_PLAN_REL="docs/plans/2026-11-26-002-fix-3174-callout-plan-identity-plan.md"
T3174_STALE_PLAN_REL="docs/plans/2026-11-22-002-fix-3174-vol1-plan.md"

# Extrait une section du blob de la sonde. Les marqueurs sont seuls sur leur
# ligne, donc un corps d'issue qui contiendrait un `__MOT__` en prose ne peut pas
# être confondu avec une frontière de section.
_t3174_section() {
    printf '%s\n' "$2" | awk -v want="__$1__" '
        /^__[A-Z_]+__$/ { inside = ($0 == want); next }
        inside { print }
    '
}

# Exécute `_write_canonical_callout` sur un corps d'issue donné et rend quatre
# sections séparables : RC, BODY (le corps écrit, ou le marqueur `NO_EDIT` si
# `gh issue edit` n'a jamais été appelé), STDERR, et CALLOUT_PLAN (la valeur de
# `GROOM_CALLOUT_PLAN`, l'invariant mika#2981).
#
# Modelée sur `_t2492_recovery_probe` : c'est l'idiome de la maison pour
# « exécuter dispatch-lib sur un vrai dépôt et lire ce qu'il produit ».
#
# `mika` n'est délibérément PAS mocké : depuis mika#2194 le seul lecteur du
# callout `Plan` est le Rust, et le prédicat ajouté par mika#3174 lui délègue.
# Un mock mesurerait le mock. D'où le `mika_bin_or_skip` qui arme le bloc.
#
# $1 = le corps d'issue du fixture.
_t3174_callout_probe() {
    local t3174_base t3174_wt t3174_fill t3174_rc=0
    t3174_base=$(mktemp -d)
    t3174_wt="$t3174_base/worktree"

    git init -q "$t3174_wt" 2>/dev/null
    mkdir -p "$t3174_wt/docs/plans"
    # > 500 octets : c'est le seuil du `-size +500c` de `_find_issue_plan`. Un
    # plan plus court ne serait pas résolu et la sonde mesurerait le find, pas le
    # court-circuit.
    {
        printf '# Plan du vol courant pour mika#3174\n\n'
        t3174_fill=0
        while [ "$t3174_fill" -lt 12 ]; do
            printf 'Remplissage pour franchir le seuil des 500 octets de _find_issue_plan.\n'
            t3174_fill=$((t3174_fill + 1))
        done
    } > "$t3174_wt/$T3174_PLAN_REL"
    # Le writer lit `git rev-parse --short HEAD` : sans commit, `head_sha` serait
    # vide et le bloc écrit incomparable.
    echo "initial" > "$t3174_wt/file.txt"
    git -C "$t3174_wt" add -A 2>/dev/null
    git -C "$t3174_wt" commit -q -m "initial" 2>/dev/null

    printf '%s' "$1" > "$t3174_base/fixture-body.md"

    (
        # shellcheck disable=SC1091
        source "$DISPATCH_LIB" 2>/dev/null || true

        # `gh` mocké en fonction shell : le writer ne l'invoque jamais via
        # `command`, donc la fonction l'emporte. Aucune sortie réseau.
        gh() {
            local gh_sub="${2:-}" gh_prev="" gh_arg
            case "$gh_sub" in
            view)
                cat "$t3174_base/fixture-body.md"
                return 0
                ;;
            edit)
                for gh_arg in "$@"; do
                    if [ "$gh_prev" = "--body-file" ]; then
                        cp "$gh_arg" "$t3174_base/written-body.md"
                        break
                    fi
                    gh_prev="$gh_arg"
                done
                return 0
                ;;
            esac
            return 1
        }

        WORKTREE_DIR="$t3174_wt"
        REPO="mika"
        ISSUE_NUM="3174"
        BRANCH="fix/3174/probe"
        GROOM_CALLOUT_PLAN=""

        t3174_rc=0
        _write_canonical_callout ready-to-groomed t3174 2>"$t3174_base/stderr.txt" || t3174_rc=$?
        printf '%s' "$t3174_rc" > "$t3174_base/rc.txt"
        printf '%s' "$GROOM_CALLOUT_PLAN" > "$t3174_base/callout-plan.txt"
    ) >/dev/null 2>&1 || true

    printf '__RC__\n'
    cat "$t3174_base/rc.txt" 2>/dev/null || printf 'NO_RC'
    printf '\n__BODY__\n'
    if [ -f "$t3174_base/written-body.md" ]; then
        cat "$t3174_base/written-body.md"
    else
        printf 'NO_EDIT'
    fi
    printf '\n__STDERR__\n'
    cat "$t3174_base/stderr.txt" 2>/dev/null || true
    printf '\n__CALLOUT_PLAN__\n'
    cat "$t3174_base/callout-plan.txt" 2>/dev/null || printf 'NO_CALLOUT_PLAN'
    printf '\n'

    rm -rf "$t3174_base"
}

if mika_bin_or_skip "mika#3174 identité du plan dans le court-circuit"; then
    # --- T1 (AC1) : un callout complet nommant un AUTRE plan fait écrire, et remplace
    #
    # Le fixture porte le callout complet du vol 1, dans la forme réelle qu'émet
    # le writer. Les trois signaux de la porte sont donc tous présents — c'est
    # précisément ce qui faisait taire le producteur.
    T3174_STALE_FIXTURE="> - **Branch:** \`fix/3174/probe\`
> - **Plan:** \`$T3174_STALE_PLAN_REL\` (committed on branch @ \`deadbee\`)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: vol1

## Constat

corps du ticket, intact."

    T3174_STALE_OUT=$(_t3174_callout_probe "$T3174_STALE_FIXTURE")
    T3174_STALE_RC=$(_t3174_section RC "$T3174_STALE_OUT")
    T3174_STALE_BODY=$(_t3174_section BODY "$T3174_STALE_OUT")
    T3174_STALE_ERR=$(_t3174_section STDERR "$T3174_STALE_OUT")
    T3174_STALE_CALLOUT_PLAN=$(_t3174_section CALLOUT_PLAN "$T3174_STALE_OUT")

    # LE rouge-avant : contre le code d'avant le correctif, la fonction rend 0
    # sans rien écrire et `NO_EDIT` apparaît.
    assert_not_contains "T1 (AC1): un callout nommant un AUTRE plan fait ÉCRIRE (pas de NO_EDIT)" \
        "NO_EDIT" "$T3174_STALE_BODY"
    assert_contains "T1 (AC1): le corps écrit nomme le plan de CE groom" \
        "> - **Plan:** \`$T3174_PLAN_REL\`" "$T3174_STALE_BODY"
    assert_not_contains "T1 (AC1): le plan du vol précédent a disparu du corps" \
        "2026-11-22-002" "$T3174_STALE_BODY"
    # Non-empilation MESURÉE plutôt que supposée — c'est la propriété que
    # mika#2012 U3 a payée, et le chemin REPLACE est le seul à la tenir.
    assert_eq "T1 (AC1): exactement une ligne Plan dans le corps écrit" "1" \
        "$(printf '%s\n' "$T3174_STALE_BODY" | grep -c '^> - \*\*Plan:\*\*' || true)"
    assert_eq "T1 (AC1): exactement une ligne Branch dans le corps écrit" "1" \
        "$(printf '%s\n' "$T3174_STALE_BODY" | grep -c '^> - \*\*Branch:\*\*' || true)"
    assert_eq "T1 (AC1): exactement une ligne Grooming history dans le corps écrit" "1" \
        "$(printf '%s\n' "$T3174_STALE_BODY" | grep -c '^> - \*\*Grooming history:\*\*' || true)"
    # Le strip est resté borné au préambule.
    assert_contains "T1 (AC1): le corps du ticket survit (titre)" '## Constat' "$T3174_STALE_BODY"
    assert_contains "T1 (AC1): le corps du ticket survit (prose)" 'corps du ticket, intact.' "$T3174_STALE_BODY"
    # Sans ce diagnostic l'opérateur voit une écriture sans savoir ce qu'elle
    # remplace.
    assert_contains "T1 (AC1): diagnostic nommé pour l'écriture sur plan périmé" \
        'write_canonical_callout_stale_plan' "$T3174_STALE_ERR"
    assert_contains "T1 (AC1): le diagnostic nomme le plan que le CORPS portait" \
        "body_plan=$T3174_STALE_PLAN_REL" "$T3174_STALE_ERR"
    assert_contains "T1 (AC1): le diagnostic nomme le plan que CE groom a validé" \
        "current_plan=$T3174_PLAN_REL" "$T3174_STALE_ERR"
    assert_eq "T1 (AC1): la fonction rend 0" "0" "$T3174_STALE_RC"

    # --- T3 (volet stale) : l'invariant mika#2981 tient sur ce chemin
    assert_eq "T3: GROOM_CALLOUT_PLAN nomme le plan courant sur le chemin réécrit" \
        "$T3174_PLAN_REL" "$T3174_STALE_CALLOUT_PLAN"

    # --- T2 (AC2) : le callout du plan COURANT laisse le corps inchangé
    #
    # Contrôle négatif de T1. Sans lui, « le prédicat discrimine » est
    # indistinguable de « le court-circuit ne tire plus jamais », qui est la
    # régression symétrique : une édition d'issue à chaque passe, et le risque
    # d'empilement que le chemin REPLACE est le seul à contenir.
    T3174_FRESH_FIXTURE="> - **Branch:** \`fix/3174/probe\`
> - **Plan:** \`$T3174_PLAN_REL\` (committed on branch @ \`deadbee\`)
> - **Grooming history:** first-pass (READY) → second-pass (GROOMED) — session-id: vol2

## Constat

corps du ticket, intact."

    T3174_FRESH_OUT=$(_t3174_callout_probe "$T3174_FRESH_FIXTURE")
    T3174_FRESH_RC=$(_t3174_section RC "$T3174_FRESH_OUT")
    T3174_FRESH_BODY=$(_t3174_section BODY "$T3174_FRESH_OUT")
    T3174_FRESH_ERR=$(_t3174_section STDERR "$T3174_FRESH_OUT")
    T3174_FRESH_CALLOUT_PLAN=$(_t3174_section CALLOUT_PLAN "$T3174_FRESH_OUT")

    assert_contains "T2 (AC2): un corps déjà estampillé par CE plan n'est pas réécrit" \
        "NO_EDIT" "$T3174_FRESH_BODY"
    # Mot pour mot : cette ligne est lue dans les journaux de dispatch.
    assert_contains "T2 (AC2): la ligne de log du court-circuit est conservée mot pour mot" \
        'dispatch-gate signals already present in mika#3174 body — skipping (idempotent)' \
        "$T3174_FRESH_ERR"
    assert_not_contains "T2 (AC2): aucun diagnostic de plan périmé sur ce chemin" \
        'write_canonical_callout_stale_plan' "$T3174_FRESH_ERR"
    assert_eq "T2 (AC2): la fonction rend 0" "0" "$T3174_FRESH_RC"

    # --- T3 (volet idempotent) : même preuve des deux côtés du court-circuit
    #
    # L'affectation de `GROOM_CALLOUT_PLAN` est AVANT le court-circuit
    # (mika#2981), donc un corps déjà estampillé produit la même preuve qu'un
    # corps réécrit. Rien dans mika#3174 ne doit le déplacer, et ce test le dit.
    assert_eq "T3: GROOM_CALLOUT_PLAN nomme le plan courant AUSSI sur le court-circuit" \
        "$T3174_PLAN_REL" "$T3174_FRESH_CALLOUT_PLAN"
fi

# --- T4 — les signaux de la porte sont restés en lockstep, et le prédicat délègue
#
# Les assertions de forme de source de `test-dispatch-lib.sh` (`has_branch=`,
# `has_plan=`, `has_verdict=` et les trois motifs de verdict) doivent rester
# vertes SANS retouche : mika#3174 n'ajoute pas un quatrième signal de porte, il
# ajoute un prédicat d'identité du plan, qui n'a pas de contrepartie côté Rust
# parce que `check_grooming_markers` n'a pas de « plan courant » à comparer.
#
# Une seule assertion de forme est ajoutée : le court-circuit lit le plan du
# corps par le LECTEUR UNIQUE, pas par un motif local. Un futur refactor qui
# renverserait cette décision doit rougir ici avant de rougir au lint des jetons
# canoniques.
assert_contains "T4 (mika#3174): le court-circuit délègue la lecture au lecteur unique" \
    '_extract_plan_path "$current_body"' "$WRITER_SRC"
assert_contains "T4 (mika#3174): le court-circuit compare au plan de CE groom" \
    '[ "$body_plan" = "$plan_relpath" ]' "$WRITER_SRC"
# Le fail-safe est dans le bon sens : le court-circuit exige rc = 0, donc
# `rc = 1` (aucun callout lisible) et `rc ≥ 2` (« je n'ai pas pu regarder »)
# écrivent tous les deux. Écrire est idempotent ; un court-circuit de trop est
# le défaut mesuré.
assert_contains "T4 (mika#3174): le court-circuit n'est pris que sur une lecture aboutie" \
    '[ "$body_plan_rc" -eq 0 ]' "$WRITER_SRC"
assert_contains "T4 (mika#3174): l'écriture sur plan périmé porte un motif greppable" \
    'write_canonical_callout_stale_plan' "$WRITER_SRC"

# --- Summary ---

echo ""
echo "========================================"
echo "Results: $PASS passed, $FAIL failed, SKIPPED: $SKIPPED"
echo "========================================"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
