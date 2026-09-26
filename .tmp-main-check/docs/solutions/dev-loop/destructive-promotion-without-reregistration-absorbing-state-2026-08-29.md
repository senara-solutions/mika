---
module: mika-agent/task_engine
tags: [dev-loop, deferred-dispatch, absorbing-state, reaper, self-repair, ready-label, observability]
problem_type: logic_error
category: dev-loop
created: 2026-08-29
ticket: mika#2045
---

# Une promotion destructive sans ré-inscription crée un état absorbant

## Problem

Une issue portant `ready` pouvait devenir définitivement non-dispatchable. L'issue gardait son label, la file la comptait, et rien ne la prenait jamais. Il fallait lire la table `tasks` en base pour le voir. Sept tâches étaient dans cet état le 2026-08-29, dont trois depuis deux à trois jours.

Le compteur qui dit l'ampleur : **1966 `ready_label_task_create_failed` contre 1234 `ready_label_engine_dispatched`** — la création de tâche échouait plus souvent qu'elle ne réussissait.

## Root cause

Trois mécanismes corrects pris séparément composent un piège.

1. `ready_label_handler` pré-crée une tâche parente `manual` portant `reference_url`. Quand le créneau de dispatch de la classe est occupé, `validate_dispatch_readiness` refuse et `register_deferred_callback` inscrit un **wrapper différé** enfant de cette parente. Le wrapper est ce qui représente la tâche dans la file.

2. La promotion est **destructive** : `promote_next_deferred_callback` fait `UPDATE tasks SET status = 'completed'`. Le wrapper quitte l'état `pending`, donc il quitte la file différée définitivement. Rien ne l'y remet.

3. `idx_tasks_manual_active_ref_url` est un index UNIQUE partiel sur `(agent_id, reference_url)` qui exclut `completed/cancelled/failed/delivered`. Tant que la parente reste `pending`, aucune remplaçante ne peut naître pour la même issue.

Il suffit donc que le tour silencieux déclenché par la promotion ne produise pas de dispatch réel : le wrapper est consommé, plus rien ne représente la parente, et l'index interdit qu'on recommence. **État absorbant.** Deux chemins y menaient, et le second était invisible :

- le tour se terminait `Ok` sans avoir appelé l'outil (no-op) ;
- **le tour renvoyait `Err`** — et cette branche ne faisait *rien du tout* : ni `mark_task_delivered`, ni détection, ni événement.

Le second explique l'occurrence mesurée du 2026-08-29 entre 09:10Z et 09:51Z : quatre tâches se sont figées avec **zéro** `deferred_dispatch_noop_completion` dans le journal.

## Solution

Trois couches, dans cet ordre : réparer, expirer, rendre visible. Voir PR sur mika#2045.

### 1. Ré-armement — la réparation, pas le constat

`rearm_deferred_callback` (`crates/mika-agent/src/skills/executor.rs`) inscrit un wrapper de remplacement quand un wrapper est consommé sans avoir produit de callback réel. Appelé depuis **les deux** terminaisons de `dispatch_resume_agent`.

Il n'y a **aucune promotion en ligne**. La cascade que `mika#1124` a fermée promouvait le wrapper *suivant* — celui d'une autre tâche — N fois dans la même pile d'appels. Le ré-armement insère une ligne `pending` pour *sa propre* parente et rend la main au promoteur périodique, qui vérifie le créneau avant de promouvoir. Le trou se referme sans rouvrir la cascade.

### 2. Ramasseur — le filet

`reap_orphaned_pending_issue_tasks` (`crates/mika-agent/src/task_engine/engine.rs`) suit une échelle :

| État | Action |
|---|---|
| wrapper `pending`, ou callback réel actif | rien — la tâche est en file ou travaille |
| orpheline, budget restant | ré-armer (le travail est conservé) |
| refus passager (file pleine) | rien — retenter au passage suivant |
| budget épuisé, ou dispatch non reconstructible | annuler les wrappers résiduels, **puis** expirer |

L'annulation avant expiration n'est pas de la politesse : un wrapper qui survit peut encore être promu et rejouerait un dispatch sur une parente morte pendant qu'une remplaçante vit déjà pour la même issue.

### 3. Sonde — briser le silence

`mika tasks --agent <nom> stuck` liste les issues figées avec leur âge et leur nombre de réparations ; le moteur émet `loop_stuck_pending_tasks` quand le compte est non nul. `ready_label_task_create_failed` nomme désormais l'issue victime et l'âge de la tâche bloquante.

## Le critère de diagnostic — deux états, pas un seuil

Le piège de conception qui a failli passer : **l'âge seul ne discrimine pas.** Une tâche `pending` depuis 33 minutes peut être saine, patientant derrière un créneau légitimement occupé. Un critère sur l'âge l'aurait tuée.

Le CLI distinguait déjà les deux états sans qu'on l'ait remarqué :

```
$ mika tasks --agent mika-dev promote-deferred implement
  No pending deferred wrapper for class 'implement'.        → ORPHELINE (le défaut)
  Cannot promote: dispatch slot ... is occupied by ...       → EN FILE (nominal)
```

Le prédicat retenu est plus fin encore : `register_deferred_callback` fixe `parent_task_id` sur le wrapper, donc « existe-t-il un wrapper `pending` dont `parent_task_id` vaut cette tâche » répond **tâche par tâche**, là où `promote-deferred` ne répond que par classe.

Note sur un faux marqueur écarté en route : `fired_at` ne discrimine rien. **27 lignes renseignées sur 2334** — une tâche parfaitement saine l'a vide aussi.

## Prevention

Trois règles généralisables, chacune payée par ce ticket.

1. **Une détection sans réparation ne guérit rien.** La garde R9 de `mika#1124` détectait exactement ce défaut et se contentait de `warn!`. Elle a averti **792 fois** sans jamais agir. Quand on ajoute un détecteur pour un état qu'on sait absorbant, il faut décider dans le même geste ce qui répare — sinon on a construit un thermomètre pour un incendie.

2. **La branche `Err` d'un chemin de reprise est le silence le plus coûteux.** Le no-op était journalisé ; l'erreur ne l'était pas. C'est donc l'erreur qui a figé quatre tâches sans laisser une seule trace exploitable. En relisant un chemin de récupération, lire la branche d'échec *avant* la branche nominale.

3. **Un refus de réparation booléen confond le passager et le définitif.** `rearm` renvoyait `bool`, donc le ramasseur lisait « la file différée est pleine » comme « cette tâche est irréparable » et détruisait, pour une condition qui se résout seule, du travail qui avait encore du budget. `RearmOutcome::{Rearmed, NotNow, Unrepairable}` sépare les deux ; seul `Unrepairable` autorise la destruction.

## Test anti-vacuité

Un correctif dont les tests passent sans lui ne prouve rien. La preuve exigée ici : neutraliser la clause de wrapper dans le prédicat et vérifier que `test_stuck_pending_reaper_spares_task_queued_behind_busy_slot` **échoue**. Exécutée, elle tombe — au niveau base comme au niveau moteur.

## Lié

- `mika#2044` — la mesure fondatrice du même défaut.
- `mika#1124` — la garde anti-cascade et sa détection R9, qui constatait sans réparer.
- `mika#1011`, `mika#1070`, `mika#1175` — l'histoire de la promotion différée, en ligne puis par classe.
