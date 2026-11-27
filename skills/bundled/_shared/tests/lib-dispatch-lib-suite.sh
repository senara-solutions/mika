#!/bin/bash
# Ossature commune des suites issues de la scission de `test-dispatch-lib.sh`
# (mika#3178, vol 1).
#
# POURQUOI CE FICHIER EXISTE. L'agrégat `_shared/test-dispatch-lib.sh` frôle le
# plafond `LARGE_FILE_LIMIT` (1 048 576 octets) de `scripts/check-secrets.sh` :
# le prochain bloc ajouté y fait rougir `Secret Scan` sur une taille et non sur
# un secret (incident #3177). mika#3178 le scinde en 25 suites autonomes sous
# `tests/`. Son prélude — herméticité git, compteurs, `assert_*` — est commun aux
# 25 : il vit ici, SOURCÉ, jamais recopié. Recopié, il divergerait suite par
# suite, et une suite gardant un `assert_*` d'hier compterait faux sans le dire.
#
# Ce fichier n'est PAS une suite : son nom ne porte aucun des cinq préfixes que
# `scripts/check-suite-wiring.sh` reconnaît (`test-`, `test_`, `check-`,
# `verify-`, `measure-`), exactement comme ses voisins `lib-mika-bin.sh` et
# `lib-fake-egress-relay.sh`. Il n'a donc rien à déclarer dans
# `scripts/suite-wiring.tsv`, et rien ne le lance directement.
#
# Usage, depuis une suite de `_shared/tests/` :
#     _SUITE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
#     source "$_SUITE_DIR/lib-dispatch-lib-suite.sh"
#
# CE QU'IL NE PORTE PAS, et c'est mesuré. Pas de fonction de résumé, et pas de
# `trap … EXIT` qui l'imprimerait : l'agrégat porte lui-même un
# `trap "rm -rf …" EXIT` au premier niveau (l. 4238, famille 5), qui écraserait
# le nôtre dans la suite extraite de cette famille. Elle sortirait 0 malgré ses
# FAIL, sans imprimer son résumé — une panne muette, dans la seule suite où elle
# ne se verrait pas. Chaque suite porte donc sa queue de résumé en clair, en fin
# de fichier, extraite mot pour mot de l'agrégat.

set -euo pipefail

# Herméticité (mika#1772, prélude de `test-dispatch-lib.sh` l. 26-49) : les
# sondes des suites font `git init` + `git commit` dans des répertoires
# temporaires et hériteraient de la config git de l'hôte. Sur une machine dont
# la config globale pose `commit.gpgsign = true`, une suite s'arrête en 128 et
# fuit ses répertoires temporaires (mesuré).
#
# On surcharge les clés de signature SEULEMENT. Annuler `GIT_CONFIG_GLOBAL` à la
# place retirerait l'identité du committer dont plusieurs fixtures dépendent :
# mesuré, 8 échecs à rc=128 sur les fixtures mika#1341/#1364/#1407/#1414.
# `init.defaultBranch` est le second couplage : les fixtures `git init` puis
# référencent `main` ; sans lui elles obtiennent `master` et sept fixtures de
# rebase échouent en 128.
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

# `SCRIPT_DIR` désigne `_shared/`, PAS `tests/` — et c'est la décision de
# conception qui rend la scission mot pour mot possible.
#
# `${BASH_SOURCE[0]}` est CE fichier, donc `dirname` donne `tests/` et `/..`
# donne `_shared/`. Raison : les blocs déplacés contiennent 26 chemins relatifs
# à `$SCRIPT_DIR` ancrés sur `_shared/`, répartis dans 11 familles —
# `"$SCRIPT_DIR/../self-dev/system_prompt.md"`, `"$SCRIPT_DIR/../../../.."`, les
# fixtures de mika#2194/#2608/#2636/#2693/#2641/#3122… Ancrer `SCRIPT_DIR` ici
# sur `_shared/` les laisse octet pour octet identiques : aucune de ces 26
# lignes n'est réécrite. Un `SCRIPT_DIR` pointant sur `tests/` exigerait de les
# réécrire toutes les 26, ce que mika#3178 interdit (« les blocs sont déplacés
# mot pour mot »).
#
# Il est calculé depuis CE fichier et jamais depuis l'appelant, comme dans
# `lib-mika-bin.sh` : une résolution relative à l'appelant marcherait pour une
# suite et chercherait un cran trop haut pour l'autre.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC2034  # lu par les blocs déplacés, pas par ce fichier
DISPATCH_LIB="$SCRIPT_DIR/dispatch-lib.sh"

PASS=0
FAIL=0
# mika#2149: a third column, so a guard that could not arm is never read as a
# bare green. Incremented only by a probe that says SKIP out loud.
SKIPPED=0

# Le binaire `mika` est résolu une fois, par une bibliothèque commune : depuis
# mika#2194 `_extract_plan_path` invoque `mika` par son nom, et depuis mika#2608
# les deux lecteurs bash du callout y délèguent. Les suites de la porte de
# grooming en dépendent donc aussi. Sourcée et non recopiée, pour la même raison
# que ce fichier l'est (mika#2608 R4, déplacé par mika#2194 vol 5 R6).
# shellcheck source=skills/bundled/_shared/tests/lib-mika-bin.sh
source "$SCRIPT_DIR/tests/lib-mika-bin.sh"

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
