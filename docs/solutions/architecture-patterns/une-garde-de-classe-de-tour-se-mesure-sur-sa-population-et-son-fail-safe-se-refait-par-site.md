---
title: "Une garde de classe de tour se mesure sur sa population, et son fail-safe se refait site par site"
date: 2026-10-05
category: architecture-patterns
module: crates/mika-agent/src/tools/cancel_task.rs
problem_type: architecture_pattern
component: dev-loop
severity: high
ticket: mika#2653
applies_when:
  - "Poser une garde sur un outil en la conditionnant à un booléen de classe de tour de ToolContext"
  - "Réutiliser un booléen existant « sur le modèle de » au lieu d'en créer un nouveau"
  - "Trancher fail-open ou fail-closed pour un verdict de vivacité Unreadable"
  - "Invoquer « un faux refus retire le geste de reprise de l'opérateur » contre une garde"
related_components:
  - crates/mika-agent/src/live_pilot.rs
  - crates/mika-agent/src/webhook_dispatch.rs
  - crates/mika-agent/src/tools/mod.rs
  - crates/mika-agent/src/task_engine/process_kill.rs
tags: [loop-substrate, guard, toolcontext, webhook-turn, live-pilot, fail-closed, fail-open, population-vide, mika2653, mika2649, mika2279]
related:
  - mika#2653
  - mika#2649
  - mika#2279
  - mika#2205
---

# Une garde de classe de tour se mesure sur sa population, et son fail-safe se refait site par site

## Contexte

Le 2026-10-02 (trace `8ec6364c-be71-11f1-908b-e931f18d2c16`), un tour mika-dev
ouvert par un webhook de revue QA sur la PR #2644 a appelé `cancel_task` sur le
pilote Fix-CI en vol `8a3b2082` : 71 tours jetés. La phase A de mika#2653
(PR #2659, en draft au moment de l'écriture) refuse désormais `cancel_task` sur
un pilote vif quand le tour est un tour webhook PR.

Le ticket proposait trois raccourcis. Les trois étaient faux, et chacun l'est
d'une façon qui se rejouera sur la prochaine garde de ce type.

## Leçons

### 1. Un booléen « sur le modèle de » peut valoir `false` exactement sur la population visée

Le ticket disait : « juste un booléen de classe de tour, sur le modèle
d'`is_webhook_fallthrough_turn` ». Or `is_webhook_fallthrough_domain`
(`webhook_dispatch.rs`) **exclut explicitement** les préfixes `[GitHub] PR ` et
`[GitHub] Check suite ` du domaine Fallthrough, parce que `self-dev-webhook-qa`
/ `-ci` y portent des dispatchs légitimes. Réutiliser ce booléen aurait donné
une garde qui ne mord **jamais** sur la population du défaut : la classe
mika#2205, une garde que personne n'exerce se lit comme une garde qui marche.

Le correctif est un cinquième axe, `ToolContext.is_webhook_pr_event_turn`, posé
par `is_webhook_pr_event_domain` au même module que les constantes de préfixe.
Le test `mika2653_les_deux_axes_de_tour_webhook_sont_exclusifs` épingle
l'exclusivité des deux axes, et
`mika2653_le_predicat_de_prefixe_saccorde_avec_la_cible` tient le prédicat
d'accord avec `webhook_event_target`.

**À faire :** avant de conditionner une garde à un booléen existant, écrire un
test qui construit un message **de la population du défaut** et vérifie que le
booléen vaut `true` dessus.

### 2. Le fail-safe d'un verdict partagé ne se transporte pas d'un appelant à l'autre

`live_pilot` (mika#2279) est **fail-open** sur `Unreadable` : un faux `Alive`
gèle un ticket et brûle le budget de re-drive. La garde de `cancel_task` utilise
**la même enum** et est **fail-closed** :

| | faux refus | faux passage |
|---|---|---|
| coût | un appel d'outil refusé, visible dans `tool_calls.output` et `audit_events` | 71 tours détruits, sans trace |
| se rejoue ? | non | oui, à chaque webhook |

La doctrine mika#2277 (« un signal illisible n'est jamais un terme satisfait »)
ne tranche pas seule : mika#2279 y a dérogé à juste titre. Ce qui tranche est
l'asymétrie de coût **de ce site**. Le coût du fail-closed est nommé dans le
doc-comment de `cancel_task.rs` : une ligne avec `process_id` sans
`process_start_time` reste non annulable depuis un tour webhook PR jusqu'au
panic-fallback `timeout_at`.

**À faire :** pour chaque nouvel appelant d'un verdict à trois états, refaire la
table faux-positif / faux-négatif ; ne jamais hériter du choix du module.

### 3. « Un faux refus retire le geste de l'opérateur » se vérifie en énumérant les appelants

Le ticket se sortait du périmètre de mika#2649 en arguant que `cancel_task` est
le geste de reprise le plus court de l'opérateur. `cancel_task_and_kill` a
quatre appelants (`mika tasks cancel`, `POST /tasks/{id}/cancel`, l'outil
`cancel_task`/`cancel_reminder`, les tests eval mika#2335), et **un seul**
traverse un `ToolContext`. Sur celui-là, le booléen ne vaut `true` que si le
message du tour commence par un préfixe GitHub : un opérateur qui écrit
« annule la tâche X » via `mika ask` est hors population. L'argument était faux,
et c'était lui qui pesait contre le fail-closed.

**À faire :** avant de céder sur une garde au nom de l'ergonomie opérateur,
lister tous les appelants de l'action gardée et marquer lesquels passent
réellement par la garde.

## Ce qui reste ouvert

- **Traversée dupliquée — fermé en phase B.** La résolution parent → enfants
  que `live_pilot_for_task` et `cancel_task_and_kill` écrivaient chacun a un site
  unique, `live_pilot::resolve_task_pilot`, qui rend des candidats sans les
  classer ; le scan `mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique` refuse
  un second lecteur. Les deux lectures divergent
  volontairement sur un point à préserver : le chemin de kill **écarte** un
  enfant sans `process_start_time`, le verdict le rend `Unreadable`.
- **Vecteurs voisins non couverts** (plan § R6) : `update_task_status` →
  `cancelled` rend un pilote orphelin sans le tuer ; `run_shell` peut taper
  `mika tasks cancel`. C'est pourquoi le texte du refus ne nomme aucune commande
  de contournement (`mika2653_le_refus_ne_nomme_aucun_contournement`).

## Références

- Plan : `docs/plans/2026-10-03-002-fix-2653-cancel-task-pilote-vif-tour-webhook-plan.md`
- Famille : mika#2649 (vecteur `run_claude_pilot`, fermé), frères mika#2652, mika#2654
