---
title: "Une affirmation d'ordre sur une tâche spawnée est une affirmation sur la primitive de spawn — la citer, ou la mesurer"
date: 2026-09-22
category: best-practices
module: agent-core
problem_type: best_practice
component: dispatch
severity: high
applies_when:
  - Un plan ou un commentaire dit « X est écrit avant que Y n'arrive » et X est produit par une tâche lancée (spawn, thread, job)
  - Un argument de sûreté repose sur « le handler a rendu, donc l'effet est posé »
  - Une décision ratifiée accepte un coût nommé et l'implémentation en révèle un d'une autre classe
tags: [tokio-spawn, ordering, timing-margin, ready-label, auto_pull, stuck-ready, architect-ratified, grooming]
---

# Une affirmation d'ordre sur une tâche spawnée est une affirmation sur la primitive de spawn — la citer, ou la mesurer

## Context

mika#2470 : la Phase 2 d'`auto_pull` dispatche désormais un ticket sauvé par
appel direct du handler ready-label, **avant** le churn remove→add du label.
L'argument de sûreté de la décision D3, ratifié par mika-arch en deux passes :
« en dispatchant d'abord, le `labeled` renvoyé par le churn ne peut arriver
qu'après deux allers-retours `gh` plus le trajet GitHub → gateway → agent :
**le pgid est écrit depuis plusieurs secondes**, la porte 2c refuse en
`pilot_in_flight` ».

Deux lecteurs ont accepté cette phrase sans la vérifier : l'architecte au
groom, puis l'orchestrateur qui a rédigé le brief de revue de code — ce brief
demandait explicitement de vérifier que « le spawn est synchrone dans l'appel
du handler ». Le relecteur adversarial a lu la ligne :
`spawn_long_running_exec` (`crates/mika-agent/src/skills/executor.rs`) est un
`tokio::spawn` ; `cmd.spawn()` puis `db.set_task_process_id(...)` s'exécutent
sur la **tâche détachée**, après que le handler a rendu `Dispatched`. Au moment
où l'appelant reprend la main, le parent est `in_progress` avec `process_id
NULL`, l'enfant callback n'a pas de pgid, et la porte 2c exige
`child.process_id IS NOT NULL`.

La sûreté réelle est une **marge temporelle** — des millisecondes de spawn
contre des secondes de trajet réseau — pas un invariant d'ordre. Ce n'est pas
une erreur d'implémentation : c'est une prémisse fausse qui a traversé deux
revues parce qu'elle avait la forme d'un fait.

## Guidance

1. **Toute phrase « X est posé avant que Y n'arrive » où X sort d'une tâche
   lancée doit citer la primitive.** `tokio::spawn`, `std::thread::spawn`, un
   job de fond : l'appelant reprend la main *avant* que le corps n'ait tourné.
   Si la phrase ne cite pas la ligne du spawn et le point où l'effet est écrit,
   elle n'est pas vérifiée — elle est plausible. Écrire « marge temporelle »
   quand c'en est une.

2. **Distinguer les deux garanties dans le texte.** « Le handler a rendu » ≠
   « l'effet est persisté ». Quand un appel rend un verdict (`Dispatched`) et
   que l'effet observable (pgid, ligne DB, fichier) est produit par une tâche
   détachée, nommer les deux instants séparément, et dire lequel la porte en
   aval lit.

3. **Une marge se mesure, elle ne se raisonne pas.** Sonde post-déploiement
   qui rendrait la perte de course visible (ici S2 : deux `gate=dispatched` sur
   le même ticket dans la même minute), plutôt qu'une attente ajoutée dans le
   chemin pour « fermer » une fenêtre dont on ne connaît pas la largeur
   (ratification mika-arch A4 : mesurer avant de durcir).

4. **Un coût accepté est accepté pour sa classe.** D3 acceptait « une ligne
   `pilot_in_flight` par sauvetage ». L'implémentation a montré, sur la branche
   non-`Dispatched`, un tour LLM `Passthrough` par sauvetage différé — un coût
   d'une autre classe. Une décision qui budgète X ne couvre pas Y : c'est
   remonté à l'architecte en choix forcé, jamais glissé dans le diff. Sa règle
   : « on ne budgète pas un gaspillage certain, on le supprime ».

## Why This Matters

Un invariant faux dans un plan groomé devient un invariant faux dans le
commentaire de la boucle, puis dans le `CLAUDE.md` du crate, puis dans le
prochain plan qui s'y adosse. Ici la chaîne avait déjà deux maillons (plan +
brief de revue) avant que quelqu'un lise la ligne du spawn. Le coût de la
vérification est une ligne de `grep` ; le coût de l'oubli est un mécanisme
de kill (mika#2335, étape 6b) réarmé sur une fenêtre que le texte disait
fermée.

## When to Apply

- Au groom, dès qu'un plan écrit « avant/après » sur deux effets dont l'un est
  produit hors du fil d'appel.
- En revue de code, quand le brief lui-même contient une hypothèse d'ordre :
  la vérifier avant de la transmettre aux relecteurs (ici le brief l'a
  transmise, et c'est le relecteur qui l'a démentie).
- Quand une porte en aval lit un effet (`process_id IS NOT NULL`) : chercher
  qui l'écrit et sur quel fil.

## Examples

Avant (D3, plan v3) : « le pgid est écrit depuis plusieurs secondes, la porte
2c refuse en `pilot_in_flight` ». Après (D3 rectifié, plan v6, commentaire de
`phase2_reconcile_stuck_ready`) : « l'étape 9i est un `tokio::spawn` : le pgid
est écrit par la tâche détachée *après* le retour du handler ; dispatcher
d'abord est une marge temporelle (millisecondes de spawn contre secondes de
trajet), mesurée par S2, pas durcie ici ».

## Related

- `docs/solutions/workflow-issues/safety-net-depending-on-the-channel-it-covers-2026-09-22.md`
  — le ticket lui-même (la classe « filet qui dépend du canal qu'il couvre »).
- `docs/plans/2026-09-22-001-fix-2470-phase2-dispatch-direct-in-process-plan.md`
  § D3, encadré de rectification daté (troisième passe mika-arch, Option A).
- mika#2335 (kill à la supersession — la fenêtre 7 → 9i), mika#2279 (porte
  2c), mika#2470.
- Mémoire orchestrateur `feedback_implementer_finds_contradiction_architect_chooses_which_resolution_yields`
  — CARRY / ROUTE : l'implémenteur remonte, l'architecte tranche.
