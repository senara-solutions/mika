---
title: Un saut se justifie par un remplaçant démarré, pas par un remplaçant reçu
date: 2026-10-06
category: best-practices
module: mika-agent/task_engine, mika-agent/server
problem_type: best_practice
component: qa-review
severity: high
applies_when:
  - On saute (ou annule) un travail au motif qu'un travail plus récent le rend inutile
  - Le signal « plus récent » est un événement qui transite par une file en mémoire
  - Le travail sauté ne peut pas être rejoué ensuite (verdict jamais posté, tour perdu)
tags:
  - supersession
  - webhook-queue
  - fail-safe
  - qa-review
  - mika-2671
---

# Un saut se justifie par un remplaçant démarré, pas par un remplaçant reçu

## Contexte

mika#2671 (phase A) saute le tour LLM d'un callback de build QA quand la tête
de la PR a été remplacée : mesuré le 2026-10-06, un callback a payé 1,03 M de
tokens pour verdicter une tête remplacée huit minutes plus tôt.

Le discriminant choisi est local et ne fait aucun appel GitHub : une ligne
`audit_events` (`qa_pr_sync_observed`, clé `pr:{repo}#{n}`) postérieure au
lancement du build. Il a été préféré à la vérité GitHub (`headRefOid`) parce que
la garde no-diff du gateway (#886) supprime certains `synchronize` : avec la
vérité GitHub, une tête déplacée par un amend de trailer aurait rendu le build
« périmé » sans qu'aucun événement ne déclenche la revue suivante.

## Le piège, que la revue de code a trouvé trois fois

La première version écrivait la ligne **à la réception** du `synchronize`, dans
`handle_message`, avant la file webhook. Trois relecteurs indépendants
(correctness, reliability, adversarial) ont convergé sur le même défaut :

- la file v2 (mika#1870) est **en mémoire et bornée** ; son drop-oldest évince
  un événement **déjà acquitté 202**, donc le gateway ne le relivre jamais ;
- un redémarrage (`make deploy`) vide la file ;
- la ligne SQLite, elle, survit.

Résultat possible : callback sauté (la ligne dit « plus récent reçu »), nouvelle
tête jamais revue, et le scan de réconciliation (mika#2334) ne repasse pas sur
une PR qui porte déjà une revue. La PR reste sans verdict sur sa dernière tête —
exactement la perte que la conception prétendait exclure « par construction ».

## La règle

Le signal qui autorise à **sauter** un travail doit attester que le travail de
**remplacement a commencé**, pas qu'il a été **reçu**. Entre « reçu » et
« démarré » se trouve tout ce qui peut perdre un événement (file bornée, crash,
coalescence, rejet 429), et c'est précisément là que le saut transforme une
perte rare en revue manquante.

Ici : la ligne est écrite dans `run_agent_for_message`, le point de passage
unique des trois chemins qui lancent un tour (drain v2, chemin hérité, rejeu
#528). Un `synchronize` encore en file au moment du callback n'est pas inscrit :
le callback tourne (on paie), ce qui est la direction fail-safe.

## Ce qui le tient

- `qa_head_supersession::tests::mika2671_le_registre_est_ecrit_au_demarrage_du_tour`
  — scan de source : un seul appel de production, et il vit dans
  `run_agent_for_message`. Vu rouge en remettant l'appel dans `handle_message`.
  Aucun test comportemental ne voit cette classe : les tests du dispatcher
  amorcent le registre directement et ne distinguent pas « reçu » de « démarré ».
- `mika2671_les_noms_daudit_ont_un_seul_ecrivain` — un seul écrivain par nom.

## Résidu nommé

Le prompt qa-review dit d'attendre le callback d'un `build_mika` en vol. Si un
tour de revue de la tête suivante attendait le build de la tête précédente au
lieu de lancer le sien, sauter ce callback laisserait la nouvelle tête sans
verdict. Mesuré sur l'épisode fondateur : les deux tours suivants ont chacun
lancé leur propre build. Résidu non fermé par cette phase.
