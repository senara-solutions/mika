---
title: "Un état qui porte deux sens se lit par la présence de sa transition inverse, pas par une date"
date: 2026-09-30
category: dev-loop
module: crates/mika-agent/src/wip_rescue.rs
component: development_workflow
problem_type: logic_error
severity: high
symptoms:
  - "PR #2589 remise en draft de hold, sortie du brouillon par `wip_rescue` deux fois en 35 minutes (2026-09-29)"
  - "merge autonome empêché seulement parce que le diff touchait des fichiers decision-core"
  - "un draft de départ (rescue) et un draft de hold (opérateur) se lisaient tous deux `isDraft: true`"
root_cause: logic_error
resolution_type: code_fix
related_components:
  - background_job
tags:
  - loop-substrate
  - wip-rescue
  - draft
  - hold
  - predicate-design
  - fail-closed
  - selection-vs-chain
  - starvation
ticket: mika#2597
---

# Un état qui porte deux sens se lit par la présence de sa transition inverse, pas par une date

Le détail du module (prédicat, trois états, sondes S1–S4, haltes) vit dans
`crates/mika-agent/CLAUDE.md` § *Un hold explicite tient contre `wip_rescue`
(mika#2597)* et dans `CLAUDE.md` racine § *Signal N-bis*. Ce document garde
seulement ce qui se transpose à **toute** garde qui doit séparer deux usages d'un
même drapeau. Correctif : PR mika#2598 (en attente de merge au moment d'écrire).

## Le problème

Un démon a pour raison d'être de **faire sortir** un état (ici : promouvoir un
brouillon de rescue). Un opérateur utilise **le même état** pour dire l'inverse
(« tiens cette PR »). Le drapeau (`isDraft: true`) ne distingue pas les deux, donc
le démon viole le hold. C'est arrivé deux fois en 35 minutes sur #2589 ; la
défense en profondeur n'a tenu que par coïncidence de périmètre.

## Deux pistes intuitives, toutes deux fausses

1. **« Hold postérieur au dernier push. »** Le démon **pousse lui-même** :
   `prepare_branch` fait `--force-with-lease` à l'étape 4, avant l'un-draft de
   l'étape 7, dans la même chaîne. Un prédicat daté contre une action que
   *l'acteur gardé* accomplit s'annule par le geste même qu'il doit empêcher. Et
   de toute façon, un push ultérieur ne lève pas une décision humaine.
2. **Un nouveau label porteur.** Il faut apprendre un nouveau geste à l'opérateur,
   alors que celui qu'il a déjà fait deux fois est « je remets en draft ». Dans ce
   dépôt, il faut en plus le déclarer dans `.github/labels.yml`, sinon
   `delete-other-labels: true` le supprime en silence.

## Le prédicat qui tient

```text
held(pr) := pr est brouillon MAINTENANT (filtre --draft du listing)
          ∧ la timeline porte ≥ 1 ConvertToDraftEvent
```

Pourquoi la simple **présence** suffit : l'objet naît dans l'état ambigu (`gh pr
create --draft`), donc aucun `ConvertToDraftEvent` à la naissance. Il n'y a que
deux transitions (`ConvertToDraftEvent` et `ReadyForReviewEvent`). Si l'objet est
brouillon *maintenant* et porte au moins une transition *vers* le brouillon, sa
dernière bascule est forcément une remise en brouillon, donc un hold. Il n'y a
**aucune date à comparer**, aucun ordre, aucune pagination (`last: 1` en GraphQL).
Le hold se lève tout seul quand l'opérateur fait sortir la PR du brouillon, sans
état à nettoyer.

**Fragilité à épingler :** l'équivalence repose sur le filtre d'état courant.
Retirer `--draft` du listing la casse. Un test de source l'épingle
(`mika2597_le_listing_filtre_toujours_draft`).

## Deux corollaires qui font partie du correctif

- **Trois états, jamais un `bool`** (`HoldVerdict::{NotHeld, Held, Unreadable}`).
  `Held` et `Unreadable` excluent tous les deux, mais leurs remèdes sont
  opposés : dans un cas le mécanisme marche, dans l'autre le jeton ou l'API gèle
  le démon. Chacun a son propre nom d'événement
  (`wip_rescue_hold_respected` / `wip_rescue_hold_unreadable`). La règle est
  fail-closed : un terme illisible ne vaut jamais « pas tenu ».
- **L'exclusion se fait à la sélection, pas dans la chaîne de traitement.** Le
  scan prend un seul candidat par tick, le plus vieux d'abord
  (`select_eligible`). Une garde placée dans `resume_chain` consommerait ce
  créneau à chaque tick. Or un hold est vieux par nature : il resterait le
  premier candidat pour toujours et affamerait la file. C'est la même forme de
  livelock que mika#2199 puis mika#2286.

## Prévention — la question à poser à toute garde de ce type

Avant d'écrire une garde qui sépare deux sens d'un même état :

1. **L'objet naît-il déjà dans cet état ?** Si oui, la présence de la transition
   *vers* cet état, filtrée sur l'état courant, discrimine sans horloge.
2. **La date de référence est-elle une action que l'acteur gardé peut faire
   lui-même ?** Si oui, le prédicat s'annule par construction ; le rejeter.
3. **La file est-elle à créneau borné ?** Si oui, exclure à la sélection, sinon
   l'objet exclu mais éligible occupe le créneau indéfiniment.
