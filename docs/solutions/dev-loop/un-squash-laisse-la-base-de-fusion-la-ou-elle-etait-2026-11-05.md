---
title: "Un squash laisse la base de fusion là où elle était — l'arbre est le contenu, le diff est une histoire"
date: 2026-11-05
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
component: development_workflow
problem_type: logic_error
severity: high
ticket: mika#3038
symptoms:
  - "une draft de secours affiche `+953 −23` alors que `rev-parse HEAD^{tree}` égale `rev-parse origin/main^{tree}`"
  - "`rescue-diff: carries-work` et `Closes #N` sur une branche dont tout le contenu est déjà dans `main`"
  - "4 drafts de secours ouvertes <20 min après le merge d'une PR de leur propre branche, toutes fermées à la main"
  - "`ready` retiré d'un autre ticket 6 secondes après l'ouverture d'une draft fantôme"
  - "une branche supprimée d'`origin` par le merge de sa PR y réapparaît en `mode=first-push` sans rien publier"
tags:
  - git
  - squash-merge
  - merge-base
  - dispatch-lib
  - rescue
  - fail-closed
---

# Un squash laisse la base de fusion là où elle était

## La mesure

Quatre drafts de secours de fin de vol, entre le 2026-09-04 et le 2026-11-04,
se sont ouvertes sur une tête dont l'arbre était **octet pour octet** celui
d'`origin/main`.

| draft | ouverte | merge précédent sur la branche | écart |
|---|---|---|---|
| #2197 | 2026-09-05 14:41:12Z | #2196, 14:22:33Z | 19 min |
| #2702 | 2026-10-14 20:37:16Z | #2696, 20:31:53Z | 5 min |
| #2899 | 2026-10-23 11:44:19Z | #2898, 11:38:36Z | 6 min |
| #3034 | 2026-11-04 08:07:36Z | #3033, 07:58:23Z | 9 min |

Toutes portaient `rescue-diff: carries-work`, toutes ont été fermées à la main
par l'opérateur. Sur #3034, GitHub affichait `+953 −23` pendant que
`d97b53bdf^{tree}` était exactement `d7de73137^{tree}`.

Le contrôle négatif compte autant : cinq autres secours ouverts sur une branche
déjà mergée (#2198, #2834, #2842, #2920, #3016) portaient un **vrai** diff et
ont été mergés. Le défaut n'était donc pas « secours après un merge » — il était
plus étroit, et c'est cette étroitesse qui a rendu le correctif court.

Une draft fantôme n'est pas inerte : six secondes après l'ouverture de #3034,
`ready` a été retiré de #2194 à son titre.

## Le mécanisme

Un merge **par squash** pose sur `main` un commit dont le *contenu* est celui de
la branche, **sans qu'aucun commit de la branche en soit un ancêtre**. La base
de fusion reste donc exactement là où elle était avant le merge.

```
seed ──── A (branche)          base de fusion : seed
   └───── S (main, squash de A) contenu(S) = contenu(A)

git merge-base main A        → seed     (le merge n'a rien rapproché)
git diff --name-only main...A → tout A   (« le travail reparaît »)
git rev-parse A^{tree}        = S^{tree} (… et il est déjà là)
```

`git diff main...A` (trois points) répond « ce que la branche introduit depuis
la base de fusion ». Comme la base n'a pas bougé, cette réponse est l'intégralité
du travail **déjà mergé**. Le prédicat qui lisait cette forme répondait donc
« cette branche porte du travail », et il avait raison sur la question qu'il
posait — ce n'était pas la question qu'il fallait poser.

## La leçon

**L'arbre est le contenu ; le diff est une histoire.** Pour savoir si une tête
apporte quelque chose, on compare deux SHA d'arbre. Deux arbres égaux, c'est la
preuve que tout fichier de cette tête est déjà dans `main`, à l'octet. Aucune
liste de chemins à classer, donc aucune liste à tenir à jour, et aucune question
de quoting — le défaut `core.quotePath=true`, qui oblige le classifieur de diff
voisin à lire en `-z` sous peine d'échouer *ouvert* sur le premier chemin
accentué, n'a aucune prise sur un SHA.

## Et pourquoi l'intersection, plutôt que simplement l'autre forme

Le réflexe, en découvrant le piège des trois points, est de passer aux deux
points (`git diff origin/main HEAD`, « l'écart à la tête de main »). Il échoue
dans l'autre sens, et il échoue **ouvert** : sur une branche simplement **en
retard**, les fichiers plus récents de `main` sont rapportés comme des
différences, donc une branche qui n'apporte rien lit à nouveau `carries-work`.

Aucune des deux formes ne répond seule à « qu'est-ce que cette branche
**apporte** à `main` ». Un chemin présent dans les **deux** sets est un chemin
que la branche a introduit **et** que la tête de `main` n'a pas déjà : c'est la
définition cherchée. L'intersection n'est donc pas une ceinture-et-bretelles —
chaque forme seule est fausse sur une population que ce dépôt produit, et elles
le sont sur des populations **différentes**.

Le prédicat d'arbre ne remplace pas ce classifieur et ne l'absorbe pas : une
branche peut apporter un fichier réel et neuf déjà mergés, et le marqueur de la
draft qui s'ouvre alors doit dire `carries-work` en ne nommant que le premier.
C'est le cas de #3016.

## Le fetch est portant, pas de l'hygiène

La population mesurée est exactement celle où le merge a eu lieu **pendant** le
vol. La ref de suivi `origin/main` du checkout partagé date alors d'avant ce
merge, et la comparaison rendrait « différents » sur la seule population qui
compte. D'où un `fetch` avant la lecture — **une fois par dispatch**, et
`fetch origin main` **nommément** : un `fetch --prune origin` nu mute les refs de
suivi du *common dir*, partagé par tous les worktrees du checkout, donc il
toucherait aussi celles des dispatches en vol. Le raisonnement complet vit sur
`_remote_branch_exists`, qui l'a écarté le premier (voir
[une-ref-de-suivi-locale-ne-dit-pas-si-le-distant-porte-la-branche.md](une-ref-de-suivi-locale-ne-dit-pas-si-le-distant-porte-la-branche.md)).

## La direction du fail-closed se déduit de ce que le correctif retire

Le prédicat rend **trois** valeurs — égal, différent, **indéterminé** — et
l'indéterminée est *nommée* plutôt que choisie par défaut : « un signal qu'on ne
peut pas lire n'est jamais un terme satisfait » (mika#2277), même contrat que
`_remote_branch_exists` (mika#2626).

Sa direction est l'**inverse** de celle de son voisin `_rescue_diff_carries_work`,
et la règle qui décide des deux est la même : **regarder ce que le correctif
retire.**

| correctif | ce qu'il retire | donc une mesure ratée doit… |
|---|---|---|
| `_rescue_diff_carries_work` (mika#2157) | un `Closes #N` | …aller vers `Refs` — une fermeture silencieuse que personne ne mesure est le pire cas |
| `_branch_tree_equals_main` (mika#3038) | la draft elle-même | …**ouvrir** la draft — le comportement courant est visible et réversible |

« Fail-closed » n'est donc pas une direction absolue : c'est « vers l'issue
qu'un humain peut constater et annuler ». Une draft fantôme se ferme d'un clic ;
une implémentation jamais publiée ne se rattrape pas.

Corollaire mesuré sur le site : le terme de l'AC3 porte sur
`remote_known_absent` (un **fait** : `ls-remote` a répondu « absente ») et non
sur `remote_has_branch` (une **décision**, qui retombe sur la ref de suivi locale
quand la mesure a échoué). Refuser un push sur une décision, c'est refuser de
publier du travail sur un `ls-remote` qui n'a pas répondu.

## Deux pièges de test rencontrés en l'écrivant

**1. `grep -n … | cut` rend une chaîne vide, et `${L:-0}` en fait un zéro.** Le
scan de câblage comparait les lignes de deux refus pour vérifier leur ordre.
Avant que le refus existe, `L3038` était `""`, `${L3038:-0}` valait `0`, et
`0 -lt 14289` est vrai : l'assertion d'ordre **passait au vert contre un fichier
qui ne portait aucun refus**. Mesuré sur le run rouge, pas supposé. Le remède
est de compter d'abord avec `grep -c` et d'affirmer la non-vacuité avant de
comparer.

**2. Un compte unique sur « câblage + prose » apprend à ajuster le nombre.** Le
scan comptait « le prédicat apparaît 4 fois ». Ajouter un renvoi croisé en
commentaire — un changement sans contenu comportemental — l'a fait rougir, et la
leçon qu'un développeur tire de ce rouge est *« bump the number »*. C'est ainsi
qu'un compte cesse d'être lu. Les **sites d'appel** se comptent donc à part
(exactement deux), le total restant affirmé avec son énumération nommée.

## Symptômes à reconnaître

- Une draft de secours dont l'onglet *Files changed* est copieux et dont
  `git rev-parse HEAD^{tree}` égale `git rev-parse origin/main^{tree}`.
- Un secours qui s'ouvre quelques minutes après le merge d'une PR de sa propre
  branche.
- Une branche que le merge a supprimée d'`origin` et qui y réapparaît en
  `mode=first-push` sans rien publier.

La commande qui tranche, et la seule :

```
git -C <worktree> rev-parse HEAD^{tree} origin/main^{tree}
```

## Sites

- `_branch_tree_equals_main`, `_fetch_main_ref_once`,
  `_note_tree_compare_undetermined` — `skills/bundled/_shared/dispatch-lib.sh`
- le refus de l'AC1, posé **avant** celui de mika#2503 (les deux s'excluent par
  le drapeau que le premier à tirer éteint, donc celui qui tourne d'abord nomme
  la raison : le diagnostic le plus spécifique passe devant)
- le saut de push de l'AC3, dans `_push_branch`
- `parse_no_pr_reason` — `crates/mika-agent/src/task_engine/dispatcher.rs` (son
  énumération est la seule qu'un opérateur lise pour savoir ce que la boucle peut
  répondre ; `rescue_scratch_only` y manquait depuis mika#2503)
- la suite — `skills/bundled/_shared/tests/test_rescue_tree_equals_main.sh`,
  atteinte par l'étape nommée du job `dispatch-lib-offline-suites` de `ci.yml`
