---
module: skills/bundled/_shared/tests/
tags: [scission, suite-bash, plage-contigue, rc-127, set-e, panne-silencieuse, fixture-partagee, mika-3178, mika-3179]
problem_type: test-failure
category: test-failures
date: 2026-11-27
---

# Scinder une suite : une plage contiguë n'est pas auto-suffisante

## Le problème

mika#3178 scinde `skills/bundled/_shared/test-dispatch-lib.sh` (19 080 lignes,
1 041 743 octets, à 6 833 octets du plafond `LARGE_FILE_LIMIT`) en 25 suites
autonomes, par **familles de blocs contigus**. Le découpage se fait par numéros
de ligne, et chaque corps est extrait mot pour mot par `sed -n 'N,Mp'`.

Le plan du vol 2 posait que trois gestes suffisent à produire une suite : écrire
un en-tête, extraire la plage, extraire la queue de résumé. Cette supposition est
fausse, et sa fausseté ne se voit **pas** en relecture : une plage contiguë,
syntaxiquement complète et octet pour octet fidèle à son origine, peut dépendre
d'un état que l'agrégat ne pose qu'une seule fois, ailleurs.

## Le symptôme, et pourquoi il est pire que rouge

Deux classes, mesurées en vol le 2026-11-27.

**(a) Panne silencieuse.** L'agrégat `source` `dispatch-lib.sh` au premier niveau
une seule fois, à sa l. 730 — donc **dans la famille 1**. Extraite seule, la
famille 2 meurt au premier appel de `_escalate_groom` : commande introuvable.
Mais le bloc redirige lui-même son `stderr` vers `/dev/null`, et il tourne sous
`set -euo pipefail` : la suite s'arrête **sans imprimer son bloc `Results:`**.
Rien ne nomme la cause. Trois marqueurs de premier niveau (l. 737-739) sont dans
le même cas, lus par des blocs des familles 2 et 8.

**(b) `rc=127` en série.** L'agrégat annonce lui-même la structure, à ses
l. 1426-1428 :

```
# The five cases below are DEFINED here next to the gate's code-shape
# assertions, but INVOKED near the end of this file — they need
# `_fixture_setup`, which is defined further down.
```

Les douze cas `test_groom_gate_*` sont **définis** en famille 2 (l. 1429-1827),
`_fixture_setup` en **famille 4** (l. 2824-2855), et la boucle qui les **appelle**
en **famille 5** (l. 4950-4988). Trois familles, une seule chaîne
définition → fixture → appel. La famille 5 extraite seule :

```
  ✗ callout present but plan file ABSENT → gate does NOT fire: ABORTED (rc=127)
  ✗ plan committed on branch → gate fires with the path: ABORTED (rc=127)
  … (douze fois)
========================================
Results: 145 passed, 12 failed, SKIPPED: 0
```

L'agrégat, lui, en rend **zéro**. Les 12 échecs sont donc entièrement produits
par la scission, sur des assertions déplacées mot pour mot.

## La cause

Un découpage en plages contiguës préserve l'**ordre** et le **texte**, mais pas
la **portée**. Trois formes d'état survivent à une plage et ne voyagent pas avec
elle :

1. un `source` de premier niveau fait une fois pour tout le fichier ;
2. une constante ou un marqueur défini en tête et lu beaucoup plus bas ;
3. une fonction définie dans une famille et appelée dans une autre.

Aucune des trois n'est visible dans la plage qui en dépend : le texte extrait est
exact, et c'est précisément ce qui rend le défaut invisible à une revue de diff.
`bash -n` ne le voit pas non plus — la syntaxe est bonne. **Seule l'exécution le
dit**, et dans le cas (a) elle le dit par un silence.

## La solution

**Une famille extraite se lance avant d'être crue.** C'est la seule méthode qui
tienne : ni la relecture, ni `bash -n`, ni le `diff` de parité mot pour mot ne
détectent une dépendance de portée. Le `diff` est même **vert par construction**
dans les deux cas ci-dessus.

Et pour l'état partagé, le ticket tranchait d'avance — « un helper utilisé par
plusieurs familles va dans [la] bibliothèque [partagée] ; un helper propre à une
famille reste avec elle » :

- **l'état de premier niveau** (le `source` et les trois marqueurs) vit dans
  l'ossature `tests/lib-dispatch-lib-suite.sh`, que toutes les suites sourcent ;
- **la chaîne définition → fixture → appel** vit dans une bibliothèque dédiée,
  `tests/lib-dispatch-lib-groom-gate-fixtures.sh`, sourcée par la seule suite qui
  en a besoin.

Deux propriétés rendent ce remède compatible avec la preuve de mot pour mot :

- **la bibliothèque est remplie par le même `sed` que les suites**, donc son
  contenu est mot pour mot par construction, pas par relecture. Son nom est ouvert
  dans la permission du pilote en **ajout seulement** (`>>`, jamais `>`), pour que
  la garantie ne dépende pas d'un jugement ;
- **les plages restent AUSSI dans le corps des suites d'origine.** Une
  redéfinition de fonction en bash est sans effet, donc la duplication ne coûte
  rien — et c'est ce qui garde le `diff` de parité des familles 2 et 4 exact.
  Vérifier d'abord que la plage déplacée ne contient **aucune** assertion ni
  `echo` de section au premier niveau : sinon, sourcer la bibliothèque
  compterait, et les compteurs d'une suite cesseraient d'être ceux de sa plage.

## Comment la détecter la prochaine fois

Sur une suite fraîchement extraite, deux signaux et un seul geste :

| signal | ce qu'il nomme |
|---|---|
| `ABORTED (rc=127)` | une fonction définie dans une autre famille |
| bloc `Results:` **absent** de la sortie | un `set -e` sorti sur une commande dont le `stderr` était masqué |

Le geste : **lancer la suite**. Un `FAIL` non nul dont l'agrégat ne rend aucun est
une dépendance de portée, jamais une assertion fautive — les assertions sont
identiques par construction.

## La troisième occurrence (vol 4, 2026-11-28) : la définition est hors de la fenêtre du vol

Le motif est revenu au vol 4, et **son profil a changé**. Deux dépendances y ont
été nommées à l'exécution :

| dépendance | définie à | lue par |
|---|---|---|
| `_t2178_line_of`, `_t2178_after` | l. 7419-7431 (**famille 9**) | famille 13 |
| `_gate_probe` | l. 6858-6920 (**famille 8**) | famille 14 |

Au vol 2, les deux dépendances mesurées traversaient des familles **du même
vol** : lire les cinq plages du vol suffisait à les voir. Au vol 4, les deux
définitions sont **en amont de la fenêtre du vol** — les plages extraites
commencent à la l. 8773, et les familles 8 et 9 sont parties au vol 3.

Ce décalage retire au défaut son dernier lecteur plausible. La scission reste
additive jusqu'au vol 7 : l'agrégat **porte encore** les l. 6858-6920 et
7419-7431, donc le relire ne montre rien de manquant. Les suites du vol 3 les
portent aussi — mais une suite du vol 4 ne `source` pas une suite du vol 3, et
rien dans son texte ne dit qu'elle en aurait besoin. La relecture de l'agrégat
courant est donc **vraie et inutile** : elle atteste une présence qui ne voyage
pas jusqu'à la suite extraite.

**Conséquence sur la fenêtre à balayer.** Chercher les dépendances de portée dans
les plages du vol est insuffisant dès le second vol d'une scission par vols
successifs. La fenêtre est **tout ce qui précède la première ligne du vol**, et
elle s'allonge à chaque vol. Le geste qui la couvre reste le même, et il reste le
seul : lancer chaque famille extraite avant de la croire.

Remède identique à celui du vol 2, et pour les mêmes deux raisons de forme : une
bibliothèque dédiée, `tests/lib-dispatch-lib-shared-probes-fixtures.sh`, remplie
par le même `sed` en ajout seulement, sourcée par les seules familles 13 et 14.
Les deux plages déplacées ne portent que des définitions de fonction — aucune
assertion, aucun `echo` de section au premier niveau — donc les compteurs des
familles 13 et 14 restent ceux de leurs plages.

## Réutilisation

La scission de mika#3178 porte 7 vols ; les vols 5 et 6 extraient 10 familles de
plus. Quatre dépendances sont **mesurées** à ce jour — deux au vol 2, deux au
vol 4, aucune au vol 3 : la liste n'est pas close, et c'est la méthode qui se
réutilise, pas la liste. Ce que les trois occurrences ajoutent l'une à l'autre
n'est pas un inventaire mais une **fenêtre qui s'élargit** : au vol *n*, une
dépendance peut venir de n'importe laquelle des familles des vols 1 à *n*−1.

Voir aussi : le gel de l'agrégat (mika#3179) qui empêche le défaut d'origine de
se reproduire pendant la scission, et
`docs/plans/2026-11-27-003-test-3178-dispatch-lib-scinder-test-dispatch-lib-plan.md`
§ *Note du vol (2026-11-27)*.
