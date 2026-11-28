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

# L'ÉTAT DE PREMIER NIVEAU QUE PLUSIEURS FAMILLES PARTAGENT (mika#3178, vol 2).
#
# Mesuré au vol 2, et le vol 1 ne l'avait pas vu : l'agrégat source
# `dispatch-lib.sh` au premier niveau une seule fois, à sa ligne 730 — donc DANS
# la famille 1 — et toutes les familles suivantes qui appellent une fonction au
# premier niveau en dépendent. Extraite seule, la famille 2 meurt au premier
# `_escalate_groom` : commande introuvable, stderr déjà redirigé vers /dev/null
# par le bloc lui-même, donc une panne SILENCIEUSE sous `set -e`. C'est le cas
# que le ticket tranche d'avance (« un helper utilisé par plusieurs familles va
# dans cette bibliothèque ») : il vit ici, et la ligne 730 reste en place dans la
# famille 1, où un second `source` est sans effet.
#
# Les trois marqueurs qui suivent sont dans le même cas, et l'agrégat le dit à sa
# ligne 732 : définis en tête de la famille 1 « parce que la garde de dérive de
# marqueur, plus bas dans ce fichier, les lit, et elle tourne même quand ce bloc
# skippe ». Leurs lecteurs hors famille 1 sont mesurés : l. 1139 (famille 2) et
# l. 6802/6820 (famille 8). Copiés mot pour mot de l'agrégat l. 730 et 737-739 ;
# le jour où ils y changent, la garde de dérive de la famille 8 le dit.
# shellcheck disable=SC1090
source "$DISPATCH_LIB" 2>/dev/null || true

# shellcheck disable=SC2034  # lus par les blocs déplacés, pas par ce fichier
WITHHELD_MARKER="Disposition-Withheld: REVIEW-ANCHOR-MISSING"
ENGINE_FINDING_MARKER="(BLOCKING) [mika-engine] review-anchor:"
ENGINE_LINE="F1: $ENGINE_FINDING_MARKER attestation withheld after the corrective re-prompt — anchors_found=3, anchors_valid=2, miss_reason=QuoteNotInBrief: paraphrase."

# DEUX AUTRES VALEURS DE PREMIER NIVEAU QUE PLUSIEURS FAMILLES PARTAGENT
# (mika#3178, vol 3). Même cas que le bloc ci-dessus, et comme lui nommé à
# l'EXÉCUTION : extraites seules, les familles 7 et 9 meurent sur une `unbound
# variable` sous `set -u`, sans imprimer un seul bloc `Results:`. La relecture ne
# les avait pas vues.
#
# `GATE_SRC_3118` : agrégat l. 5977 (famille 6), lu l. 6086/6088/6101
# (famille 7). `REPO_ROOT` : agrégat l. 6758 (famille 8), lu l. 7862/7876
# (famille 9) et plus loin par les familles 13, 18 et 24 — l'agrégat le dit
# lui-même à sa l. 7862 (« Checked here because this suite already resolves
# REPO_ROOT »), donc le promouvoir ici sert aussi les vols 4 à 6.
#
# Copiées mot pour mot de l'agrégat ; leurs deux lignes d'origine y RESTENT en
# place, où la ré-assignation est sans effet — c'est ce qui garde le `diff` de
# déplacement exact, la suite restant sa plage plus l'ossature.
# shellcheck disable=SC2034  # lues par les blocs déplacés, pas par ce fichier
GATE_SRC_3118=$(declare -f _groom_plan_halt_gate)
# shellcheck disable=SC2034  # lue par les blocs déplacés, pas par ce fichier
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

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
