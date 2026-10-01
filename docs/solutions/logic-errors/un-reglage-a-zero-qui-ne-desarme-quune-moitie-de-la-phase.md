---
module: kg/entity_resolver
date: 2026-10-01
problem_type: logic_error
component: database
severity: high
symptoms:
  - "kg_resolver_tick.complete à chaque tick, sur chaque agent : pending 1288-1493, resolved_in_tick 0, duration_ms 26000-48000"
  - "le couple aborted_budget: true ET llm_calls: 0 sur la même ligne"
  - "diagnostic initial du ticket : « graphe de domaine (kg_entities) vide ou incomplet »"
root_cause: logic_error
resolution_type: code_fix
tags: [kg, budget, livelock, configuration, palliatif, zero-value, diagnostic, mika-1833]
ticket: mika#1833
---

# Un réglage « à zéro » qui ne désarme qu'une moitié de la phase

## Problème

Le 2026-07-26, chaque tick du résolveur KG brûlait 26 à 48 s par agent pour
**zéro** résolution, toutes les 30 minutes, sur cinq agents — et affamait la
boucle de développement. Le ticket (mika#1833) en a conclu que le graphe de
domaine était vide. Il ne l'était pas nécessairement, et la cause était **le
palliatif lui-même** : `MIKA_KG_BATCH_BUDGET=0`, posé en croyant désactiver la
phase, puisque la doc le disait (*« `0` disables the phase entirely »*).

## Symptômes

```
pending: 1288-1493  resolved_in_tick: 0  duration_ms: 26000-48000
aborted_budget: true  llm_calls: 0
```

## Ce qui n'a pas marché

**Chercher la cause dans les données.** L'hypothèse « graphe de domaine vide » est
plausible, invérifiable depuis le bac à sable de dispatch (aucune base lisible),
et **non nécessaire** : l'interblocage se produit avec un graphe parfaitement
peuplé. Elle aurait envoyé le correctif sur `domain_builder`, qui n'était pas en
cause.

**Lire la doc du réglage pour savoir ce que fait le réglage.** La doc disait que
`0` désarmait la phase ; le code ne le faisait que pour une de ses deux étapes. Et
une seconde doc, à quelques lignes de là (`crates/mika-agent/CLAUDE.md`), décrivait
le comportement réel **comme une fonctionnalité** (*« even `budget=0` lets exact
matches resolve »*). Deux documents se contredisaient, chacun vrai d'une moitié.

## Solution

### Le diagnostic est dans le couple de compteurs, pas dans les données

`aborted_budget: true` **avec** `llm_calls: 0` n'a qu'une cause possible dans le
code : `aborted_budget` n'est posé que sur `SkippedBudget`, qui exige
`llm_call_allowed == false` ; or
`llm_call_allowed = stats.llm_calls < budget`
(`crates/mika-agent/src/kg/entity_resolver.rs:420`). Avec `llm_calls == 0`, cela
impose `0 < budget` faux, donc **`budget == 0`**. La configuration était la cause,
lisible en une ligne de journal, deux mois avant que quiconque la regarde.

### La chaîne, maillon par maillon — aucun n'est fautif seul

1. L'opérateur pose `MIKA_KG_BATCH_BUDGET=0` pour arrêter la phase.
2. Un modèle de résolution **est** configuré, donc `SkippedNoLlm` — qui écrit une
   ligne `kg_resolutions_log` et **draine** — n'est jamais atteint.
3. Stage-1 trouve un exact match, mais le seuil est `confidence > 0.9`
   **strict** ; à la valeur modale `0.9` qu'émet un LLM pour une extraction sûre,
   l'entité escalade en Stage-2.
4. `0 < 0` est faux ⇒ `SkippedBudget`.
5. `SkippedBudget` fait `continue` **sans** `apply_result` : aucune ligne
   `kg_resolutions_log` n'est écrite.
6. La sélection est `ORDER BY e.id ASC LIMIT …`
   (`entity_resolver.rs:1203`) : les **mêmes** lignes de tête reviennent au tick
   suivant. Retour en 3, indéfiniment.

Le budget nul ne désarmait que **l'étape qui aurait fait avancer la file** ; la
sélection, elle, continuait. Une phase à moitié éteinte n'est pas une phase
éteinte : c'est une file dont la tête ne peut plus sortir.

### Le correctif : zéro désarme la phase entière, avant toute requête

`kg::budget::phase_is_disabled(budget)` est le **lecteur unique** de la question
(`crates/mika-agent/src/kg/budget.rs`, scan de source à allowlist vide) ;
`resolve_pending`, `resolve_doc_entities`, `extract_pending` et les deux phases du
tick court-circuitent **avant** leurs requêtes de comptage — où vivait
l'essentiel des 26-48 s. Le tick émet quand même sa ligne de complétion, avec
`skipped_reason: "zero_budget"` et le champ de comptage **absent** (`null`,
jamais `0`) : sans elle, « la phase est désarmée » et « le tick ne tourne pas »
rendraient des octets identiques.

Le comportement retiré est nommé plutôt que perdu en silence : sous `budget == 0`,
les exact matches Stage-1 ne passent plus « gratuitement ». Ce mode avait **déjà
son levier propre, et c'est celui qui termine** : ne configurer aucun modèle de
résolution — tout ce qui n'est pas au-dessus du seuil draine alors en
`skipped_no_llm`. *Deux orthographes pour une intention, dont une seule termine :
on retire l'autre.*

## Pourquoi ça marche

L'interblocage exige deux ingrédients : une étape aval désarmée **dont le refus ne
laisse aucune trace d'état**, et une étape amont qui **continue de sélectionner**
les mêmes éléments. Retirer le premier ingrédient seulement (écrire une ligne sur
`SkippedBudget`) aurait drainé la file — en la vidant de résultats faux,
`no_match` sur des entités qui en avaient un. Faire de zéro un désarmement total
retire le second : rien n'est sélectionné, rien n'est refusé, rien ne revient.

## Prévention

- **Une valeur « off » d'un réglage désarme la phase entière, et le fait avant
  toute requête.** Si une moitié doit rester active, ce n'est pas la valeur zéro
  du même réglage : c'est un autre réglage, ou l'absence d'une dépendance (ici, du
  modèle). Un zéro qui veut dire « à moitié » sera lu « éteint » par l'opérateur
  qui l'utilise en incident — c'est-à-dire exactement quand il ne lira pas le code.
- **Avant de poser un palliatif de configuration, lire ce que la valeur fait dans
  le code, pas dans la doc.** Le palliatif du 2026-07-26 *était* la cause, et il a
  tenu deux mois parce que rien ne disait qu'il était en vigueur. Le budget résolu
  est désormais dit une fois par agent et par démarrage (`kg_budget_resolved`,
  champs `budget`, `budget_source`, `resolution_armed`, `extraction_armed`) — *un
  réglage qu'on ne peut pas observer n'est pas un réglage, c'est un espoir*
  (mika#2293).
- **Quand un compteur de journal contredit une hypothèse sur les données, croire le
  compteur.** Remonter d'un couple de valeurs (`aborted_budget` × `llm_calls`) à la
  seule branche de code qui le produit coûte une lecture ; tester une hypothèse sur
  les données coûte une base qu'on n'a pas.
- **Test de non-régression qui encode la leçon** :
  `tests/eval/test_kg_zero_budget_1833.rs` — sous budget nul, aucune entité n'est
  touchée, aucune ligne `kg_resolutions_log` n'est écrite, `aborted_budget` reste
  `false` ; et à budget non nul, l'exact match au-dessus du seuil résout toujours
  (contrôle négatif). Le chemin fondateur exact — modèle configuré, entité à `0.9`
  — n'y est pas rejoué : le test épingle le court-circuit, pas l'interblocage.

## Liens

- `dev-loop/two-predicates-for-one-concept-livelock-2026-09-03.md` — même famille :
  un refus qui ne produit aucun effet d'état se répète au lieu de borner. Ici le
  refus était `SkippedBudget`, et ce qui le rendait répétable était la sélection
  amont restée active.
- `database-issues/kg-null-hash-deadlock-and-extraction-lag-2026-05-09.md` — même
  module, autre instance : un prédicat « pending » face à une écriture qui n'écrit
  rien.
- `architecture-patterns/coupled-budgets-only-one-of-which-has-a-knob-2026-09-06.md`
  — deux réglages couplés dont un seul est exposé ; ici, un seul réglage dont la
  valeur zéro couvre deux étapes qui n'auraient jamais dû la partager.
- Hors périmètre, nommé : le seuil `> 0.9` strict continue d'envoyer en Stage-2 des
  exact matches parfaits à la confiance modale — un gaspillage réel, dont la
  précondition est la distribution des confiances d'extraction.
