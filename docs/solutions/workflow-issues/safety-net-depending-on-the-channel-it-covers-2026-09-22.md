---
module: auto_pull
tags: [auto_pull, stuck_ready, webhook, dispatch, silent_failure, structural_enforcement, audit]
problem_type: silent_dispatch_failure
category: workflow-issues
date: 2026-09-22
---

# Le filet qui dépend du canal qu'il couvre — la Phase 2 de `auto_pull` ré-écrivait `ready` et attendait un webhook mort

## Problem

La Phase 2 de `auto_pull` (mika#1824, `phase2_reconcile_stuck_ready`) existe pour
rattraper les tickets qui portent `ready` mais n'ont jamais été dispatchés —
son propre commentaire nomme le cas d'usage : « *webhook dropped* ». Jusqu'à
mika#2470 son unique action était un churn remove→add du label : GitHub
émettait alors `labeled(ready)`, et **le dispatch attendait ce webhook**.

Le 2026-09-21, câbles Ethernet débranchés (`eno1 carrier=0`, reverse-proxy
Synology sur l'IP Ethernet) : zéro webhook reçu depuis 14:31:49Z. À
17:00:11–17:00:19Z le moteur journalise trois `stuck_ready_reconciled` (#2149,
#2152, #2155). Preuve DB du non-événement :

```sql
select count(*) from tasks where created_at > '2026-09-21T17:00:00Z';  -- 0
```

Trois re-drives plus tard, `auto_pull_redrive_abandoned` « sans progrès
observable ». Le filet a « réussi » trois fois sans rien dispatcher — même
schéma que mika#2449. La seule voie de récupération de la boucle était
structurellement couplée à la panne qu'elle devait couvrir : un cercle.

**La classe.** Un mécanisme de secours dont l'action passe par le composant
dont la défaillance est son cas d'usage. Il est vert exactement tant qu'on n'en
a pas besoin, et sa propre ligne de succès (`stuck_ready_reconciled`) ment
quand on en a besoin.

## Resolution

Le dispatch se fait par **appel direct, in-process, du handler ready-label**
(`server::ready_label_handler::try_handle_ready_label_dispatch_with_fetcher`),
celui-là même que la passerelle appelle à la réception du webhook. Trois
décisions portent la forme (plan `docs/plans/2026-09-22-001-fix-2470-…`) :

1. **Appeler, pas dupliquer (D1).** Deux copies des étapes 9a–9i existaient
   déjà (`ready_label_handler.rs`, `dispatcher.rs::try_dispatch_pilot_after_groom_success`).
   Une troisième aurait été la dérive que mika#2158 a payée. Les quinze portes
   du handler (allowlist, pilote vif, egress, siège, held, groomé→`dev-pilot` /
   non→`dev-groom`, slot + différé, spawn) s'appliquent **par construction**.
   Diff sur le handler : zéro.
2. **Le corps déjà lu est le fetcher (D2).** La Phase 2 tient `Issue { body,
   labels }` pour chaque ticket ; le handler reçoit une closure qui les rend.
   Zéro `gh issue view` par sauvetage, test hermétique, et le handler décide
   sur le **même instantané** que les filtres qui ont déclaré le ticket
   éligible — une lecture fraîche pourrait faire diverger les deux.
3. **Dispatch d'abord, churn ensuite, et le churn reste (D3).** La seule
   fenêtre dangereuse est un `labeled` webhook traité entre la pré-création de
   la parente (étape 7) et l'enregistrement du pgid (9i) : la porte 2c ne voit
   pas encore de pilote, et l'étape 6b tuerait le nouveau-né (mika#2335). En
   dispatchant avant, le `labeled` du churn ne peut arriver qu'après deux
   allers-retours `gh` plus le trajet GitHub → gateway → agent ; 2c le refuse
   en `pilot_in_flight`, refus nominal. Le churn reste parce qu'il remet l'âge
   du label à zéro : c'est le throttle qui espace les re-drives d'un ticket
   dont le pilote meurt vite (mika#1824 D3, budget mika#2020 inchangé).
   **Rectifié en troisième passe (mika-arch, Option A) :** la marge est
   temporelle, pas un invariant (9i est un `tokio::spawn`) ; et quand l'appel
   direct n'a pas dispatché mais que le moteur tient déjà le ticket (parente
   `pending` laissée par 9d/9a/9b), le churn est sauté — ses deux rôles sont
   sans objet et son `labeled` ne produirait qu'une collision à l'étape 7 et un
   tour LLM `Passthrough`. Ligne d'audit : `churn=skipped_in_flight`.

Une ligne d'audit côté `auto_pull` dit ce qu'`auto_pull` a obtenu, jointurable
au `ready_label_outcome` du handler par `trace_id` :

```sql
select after_value, count(*) from audit_events
 where tool_name = 'stuck_ready_direct_dispatch' group by 1;
```

`dispatched` est le succès primaire ; la parente `self_dev` en est la
conséquence topologique, pas le porteur. `handled` / `passthrough` → joindre
`ready_label_outcome` sur `trace_id` pour lire la porte. Une ligne `handled`
porte `task_id=none` même quand une parente `pending` existe (slot occupé,
outil absent) — seule la variante `Dispatched` porte un `task_id` ; la parente
se retrouve par `tasks.reference_url`.

Le contrôle d'ordre est un test de source (`mika2470_direct_dispatch_precedes_the_label_churn`) :
il rougit si un diff futur remet le churn avant le dispatch.

## Prevention

- **Compter la conséquence, pas le message.** Après un `*_reconciled` /
  `*_rescued` / `*_repaired`, compter la ligne que l'action devait produire
  (`tasks`, PR, commit) dans la même fenêtre. Un succès journalisé sans effet
  compté est le symptôme de cette classe. Ici : « `select count(*) from tasks
  where created_at > <ts du reconciled>` = 0 » a suffi à renverser une lecture
  erronée (« Phase 2 réparera à ~17:10Z »).
- **Tracer le chemin d'action d'un filet jusqu'au bout.** Si le trajet
  repasse par le composant dont la panne est le cas d'usage du filet (ici :
  label → GitHub → webhook → passerelle → agent, pour couvrir « webhook
  dropped »), le filet est inerte par construction. La question à poser au
  moment de l'écrire : « quand X est mort, par où passe mon action ? ».
- **Le contexte obligatoire, pas optionnel.** `DirectDispatchCtx` n'est pas
  un `Option` : un `None` serait « le comportement d'avant », réintroduisible
  par un appelant distrait. L'absence du contexte est une erreur de
  compilation, pas un retour silencieux au label-et-attendre.
- **Sonde post-déploiement avec halte.** Au premier `stuck_ready_reconciled`
  réel : `after_value = dispatched` attendu, puis `count(tasks) ≥ 1`. Tant
  qu'on n'a pas vu un `dispatched`, ne pas dire « réparé » (mémoire *déployé ≠
  efficace*).

## Related

- mika#2470 (ce ticket), mika#2449 (même schéma d'abandon), mika#1824 (Phase 2,
  throttle D3), mika#1572 (dispatch moteur), mika#2279 (porte 2c), mika#2335
  (kill à la supersession), mika#2315 D5 (timeline réutilisée), mika#2323
  (lecteur unique de la porte), mika#2020 (budget re-drive), mika#1614 (copie
  n°2 des étapes 9a–9i — fusion à ouvrir quand S1 est verte une fois).
- `docs/solutions/workflow-issues/ready-label-dispatch-handler-regression-2026-04-27.md`
  — la première occurrence de « le label disparaît et rien ne part », côté
  prompt ; celle-ci est la même classe côté filet.
- `docs/solutions/best-practices/a-rescue-mechanism-must-know-how-to-give-up-out-loud-2026-08-30.md`
  (mika#1901) — le budget re-drive de cette même boucle ; il reste incrémenté
  sur le churn réussi, pas sur `Dispatched`.
- `docs/solutions/dev-loop/two-predicates-for-one-concept-livelock-2026-09-03.md`
  (mika#2158) — le précédent de D1 : appeler, ne pas dupliquer.
