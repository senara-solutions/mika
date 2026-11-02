#!/bin/bash
# Résolution du binaire `mika` pour les suites bash de `_shared/` (mika#2194).
#
# POURQUOI CE FICHIER EXISTE, et pourquoi il est PARTAGÉ et non recopié.
#
# Depuis le vol 5 de mika#2194, `_parse_disposition` et `_parse_verdict` ne
# décident plus rien : elles écrivent la réponse de l'architecte dans un fichier
# et appellent `mika arch-disposition`. Deux suites les exercent —
# `_shared/test-dispatch-lib.sh` et `_shared/tests/test_parse_disposition.sh` —
# et elles ont donc exactement le même besoin : trouver le binaire, ou compter un
# SKIP qui se dit. C'est une seule décision ; une seule copie la porte.
#
# La recopier serait reproduire, dans le geste qui retire une seconde
# implémentation du bash, le défaut que ce geste existe pour fermer : deux
# résolutions divergeraient, et la suite qui garderait l'ancienne skipperait sur
# une CI où le binaire est là (ou, pire, le chercherait là où il n'est plus).
#
# Ce fichier n'est PAS une suite : son nom ne porte aucun des cinq préfixes que
# `scripts/check-suite-wiring.sh` reconnaît (`test-`, `test_`, `check-`,
# `verify-`, `measure-`), exactement comme son voisin `lib-fake-egress-relay.sh`.
# Il n'a donc rien à déclarer dans `scripts/suite-wiring.tsv`.
#
# Usage, depuis une suite :
#     source "$SCRIPT_DIR/tests/lib-mika-bin.sh"      # depuis _shared/
#     source "$SCRIPT_DIR/lib-mika-bin.sh"            # depuis _shared/tests/
#     mika_bin_or_skip "<étiquette du bloc>" || …
#
# Les chemins candidats sont calculés depuis CE fichier, jamais depuis
# `$SCRIPT_DIR` de l'appelant : les deux suites ne vivent pas à la même
# profondeur, et une résolution relative à l'appelant marcherait pour l'une et
# chercherait `target/` un cran trop haut pour l'autre.

# `SKIPPED` est le compteur de la troisième colonne. Les deux suites ne le
# déclarent pas au même endroit, et `set -u` est vivant dans les deux : on le
# crée ici s'il n'existe pas, sans jamais écraser celui de l'appelant.
: "${SKIPPED:=0}"

_MIKA_BIN_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_MIKA_BIN_REPO_ROOT="$(cd "$_MIKA_BIN_LIB_DIR/../../../.." && pwd)"

: "${FAIL:=0}"

# Trois portes, dans l'ordre : les deux cibles cargo de CET arbre, puis le PATH.
#
# L'arbre d'abord, et c'est la correction de la revue du vol 5. Ces suites
# mesurent le code sous revue, pas le binaire installé : un `mika` du PATH
# (`~/.local/bin`, posé par `make deploy`) est celui d'un commit antérieur.
# Mesuré le 2026-11-02 : le `mika` déployé rendait 2 sur `arch-disposition
# --help`, donc chaque enveloppe aurait pris son chemin de refus et les suites
# auraient rougi sur une cause qui n'est pas la leur ; ou, sur un binaire
# périmé qui connaîtrait la sous-commande, verdi sur l'ancien moteur. Le PATH
# reste la dernière porte, pour une copie du dépôt sans `target/`.
#
# Le PATH est augmenté pour TOUT le fichier appelant plutôt que par bloc : `mika`
# est appelé par son nom au fond de dispatch-lib, donc un bloc qui oublierait de
# l'augmenter skipperait sans le dire — l'inverse du contrat de la troisième
# colonne. Aucune variable de production n'est introduite : c'est le PATH.
MIKA_BIN=""
for _mika_cand in "$_MIKA_BIN_REPO_ROOT/target/debug/mika" "$_MIKA_BIN_REPO_ROOT/target/release/mika"; do
    if [ -x "$_mika_cand" ]; then MIKA_BIN="$_mika_cand"; break; fi
done
if [ -z "$MIKA_BIN" ] && command -v mika >/dev/null 2>&1; then
    MIKA_BIN="$(command -v mika)"
fi
if [ -n "$MIKA_BIN" ]; then
    PATH="$(cd "$(dirname "$MIKA_BIN")" && pwd -P):$PATH"
    export PATH
fi

# `mika_bin_or_skip <étiquette>` : rend 0 si le binaire est là, sinon rend 1 en
# le DISANT. Le SKIP est bruyant parce qu'un bloc de porte qui n'a pas pu
# s'armer ne doit pas se lire comme un vert (mika#2149, troisième colonne).
#
# En CI (`CI` ou `GITHUB_ACTIONS` posé), ce n'est plus un SKIP mais un ÉCHEC
# compté. CI construit avant (`cargo test` précède les suites bash dans
# `ci.yml`), donc un binaire absent y est une panne de câblage, et un SKIP
# sorti à 0 serait le vert sur rien que le déplacement de l'étape vers `Check`
# existe pour empêcher (revue du vol 5).
mika_bin_or_skip() {
    [ -n "$MIKA_BIN" ] && return 0
    if [ -n "${CI:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
        FAIL=$((FAIL + 1))
        echo "  ✗ FAIL $1 : binaire 'mika' introuvable EN CI (target/debug, target/release, PATH)."
        echo "    En CI, \`cargo test\` doit l'avoir construit avant cette suite : c'est"
        echo "    une panne de câblage, pas un cas à skipper."
        return 1
    fi
    SKIPPED=$((SKIPPED + 1))
    echo "  ⊘ SKIP $1 : binaire 'mika' introuvable (target/debug, target/release, PATH)."
    echo "    Depuis mika#2194 les lecteurs bash du callout ET de la disposition"
    echo "    délèguent au moteur : sans le binaire ce bloc ne peut RIEN mesurer."
    echo "    Remède : cargo build -p mika-cli --bin mika"
    return 1
}
