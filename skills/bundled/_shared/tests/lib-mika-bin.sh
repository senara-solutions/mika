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

# Trois portes, dans l'ordre : le PATH (le cas production), puis les deux cibles
# cargo (le cas dev, et CI où `cargo test` précède les suites bash).
#
# Le PATH est augmenté pour TOUT le fichier appelant plutôt que par bloc : `mika`
# est appelé par son nom au fond de dispatch-lib, donc un bloc qui oublierait de
# l'augmenter skipperait sans le dire — l'inverse du contrat de la troisième
# colonne. Aucune variable de production n'est introduite : c'est le PATH.
MIKA_BIN=""
if command -v mika >/dev/null 2>&1; then
    MIKA_BIN="$(command -v mika)"
else
    for _mika_cand in "$_MIKA_BIN_REPO_ROOT/target/debug/mika" "$_MIKA_BIN_REPO_ROOT/target/release/mika"; do
        if [ -x "$_mika_cand" ]; then MIKA_BIN="$_mika_cand"; break; fi
    done
fi
if [ -n "$MIKA_BIN" ]; then
    PATH="$(cd "$(dirname "$MIKA_BIN")" && pwd -P):$PATH"
    export PATH
fi

# `mika_bin_or_skip <étiquette>` — rend 0 si le binaire est là, sinon compte un
# SKIP BRUYANT et rend 1. Le SKIP est bruyant parce qu'un bloc de porte qui n'a
# pas pu s'armer ne doit pas se lire comme un vert (mika#2149, troisième
# colonne). CI construit avant (`cargo test` précède les suites bash dans
# `ci.yml`), donc le SKIP est le cas dev, pas le cas CI.
mika_bin_or_skip() {
    [ -n "$MIKA_BIN" ] && return 0
    SKIPPED=$((SKIPPED + 1))
    echo "  ⊘ SKIP $1 — binaire 'mika' introuvable (PATH, target/debug, target/release)."
    echo "    Depuis mika#2194 les lecteurs bash du callout ET de la disposition"
    echo "    délèguent au moteur : sans le binaire ce bloc ne peut RIEN mesurer."
    echo "    Remède : cargo build -p mika-cli --bin mika"
    return 1
}
