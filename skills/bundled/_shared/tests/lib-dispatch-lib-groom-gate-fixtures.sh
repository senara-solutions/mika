#!/bin/bash
# Fixtures de la porte de grooming, partagées entre familles (mika#3178, vol 2).
#
# POURQUOI CE FICHIER EXISTE, et c'est l'agrégat qui le dit. Ses lignes 1426-1428
# annoncent la structure que la scission rencontre :
#
#     # The five cases below are DEFINED here next to the gate's code-shape
#     # assertions, but INVOKED near the end of this file — they need
#     # `_fixture_setup`, which is defined further down.
#
# Les douze cas `test_groom_gate_*` sont donc définis dans la famille 2
# (l. 1429-1827), `_fixture_setup` dans la famille 4 (l. 2826), et la boucle qui
# les appelle dans la famille 5 (l. 4950-4988). Trois familles, une seule chaîne
# définition → fixture → appel : AUCUN découpage en plages contiguës ne la garde
# entière. Mesuré au vol 2 — la famille 5 extraite seule rendait 12 `rc=127`,
# quand l'agrégat en rend 0.
#
# C'est le cas que le ticket tranche d'avance : « un helper utilisé par plusieurs
# familles va dans cette bibliothèque ; un helper propre à une famille reste avec
# elle. » Les deux plages extraites ici ne contiennent AUCUNE assertion et aucun
# `echo` de section — seulement des définitions de fonctions et les constantes
# qu'elles lisent. Sourcer ce fichier ne compte donc rien : les compteurs d'une
# suite restent ceux de sa propre plage. Vérifié ligne à ligne avant extraction.
#
# Plages d'origine, à la base 890f3f5f, extraites par `sed -n 'N,Mp'` (mika#3178
# D1(a), permission claude-pilot#325) — mot pour mot par construction :
#   - l. 2824-2855 : `_fixture_setup`, `_fixture_cleanup`,
#     `_assert_fixture_is_local` (famille 4). D'abord, car les douze cas les
#     appellent.
#   - l. 1426-1827 : les douze cas `test_groom_gate_*`,
#     `_fixture_foreign_plan_on_main`, et les quatre constantes de corps et de
#     chemin qu'ils lisent (famille 2).
#
# Les deux plages restent AUSSI dans le corps des suites 2 et 4, qui copient leur
# famille mot pour mot : une redéfinition de fonction est sans effet, et c'est ce
# qui garde le `diff` d'AC3 de ces deux suites exact. L'agrégat n'est pas touché à
# ce vol ; au vol 7, qui le supprime, ce fichier devient l'unique définition.
#
# Comme ses voisins `lib-mika-bin.sh` et `lib-dispatch-lib-suite.sh`, son nom ne
# porte aucun des cinq préfixes que `scripts/check-suite-wiring.sh` reconnaît :
# il n'est pas une suite, n'a rien à déclarer dans `scripts/suite-wiring.tsv`, et
# rien ne le lance directement.
