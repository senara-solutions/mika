---
title: Un pré-filtre de webhook se place après les handlers déterministes, pas avant
date: 2026-10-07
category: best-practices
module: crates/mika-agent/src/server
problem_type: best_practice
component: agent-core
severity: medium
tags: [webhooks, llm-cost, prefilter, mika-2675]
applies_when:
  - "Vouloir écarter un événement webhook sans tour LLM"
  - "Ajouter une garde qui court-circuite run_agent_for_message"
  - "Mesurer les tours LLM « à vide » par classe d'événement"
---

# Un pré-filtre de webhook se place après les handlers déterministes, pas avant

## Le problème

Mesuré le 2026-10-07 sur `messages` × `llm_calls` (30 h) : chaque `check_suite`
verte sur `main` coûtait deux tours LLM (mika-qa ≈ 44 k tokens d'entrée,
mika-dev ≈ 55 k), seize sessions sur la fenêtre, toutes concluant « CI green on
main. No action ». Les labels `bug`, `p1-important`, `p2-normal`,
`loop-substrate` coûtaient chacun un tour mika-dev (≈ 65 k) qui concluait « pas
de `ready`, pas de dispatch » — ce que le prompt `self-dev` prescrit déjà.

## La décision qui ne va pas de soi : APRÈS les handlers

Le réflexe est de filtrer en tête de `run_agent_for_message`, comme la garde de
tête périmée de mika#2671 B2. C'est faux ici : B2 annule un tour **entier**,
alors qu'un événement « sans suite » pour le modèle peut encore avoir une suite
pour le moteur. Les handlers structurels (`ci_success_handler` évalue le merge,
`verdict_handler` notifie l'opérateur d'un `hold[review]`, `upstream_close`
nettoie des lignes) doivent tourner. La porte vit donc entre le dernier handler
et `agent::run_agent` : **le filtre ne change pas ce que le moteur fait, il
change seulement si l'on demande ensuite au modèle.**

Corollaire : les classes qui ne dépendent pas d'un handler exigent un **texte
intact**. Si un handler a remplacé ou enrichi `req.text`, il a quelque chose à
dire au modèle, et le tour a lieu. Sur `main`, l'évaluateur de mika-dev rend
`Passthrough { enrichment: None }` (pas de PR ouverte) et celui de mika-qa sort
à la porte mika#2260 sans toucher au texte : les deux restent filtrables.

## Ce qui reste fail-safe

Interrupteur `MIKA_WEBHOOK_PREFILTER` désarmé (trois paliers, via
`qa_head_supersession::parse_switch`), texte touché, recherche de tâche en
erreur : le tour a lieu. Un faux « écarter » perd un événement ; un faux
« laisser passer » coûte un tour. L'asymétrie décide.

## L'ensemble des labels lus est composé, pas recopié

`webhook_dispatch::label_read_by_loop` compose `READY_LABEL`,
`OPERATOR_HELD_LABELS`, le préfixe de siège `DISPATCH_SEAT_LABEL_PREFIX`, le
préfixe `operator-` et les labels d'automatisation. Large à dessein : un lecteur
qui fait grandir sa liste fait grandir l'ensemble dans la même édition.

Le critère est l'**événement d'ajout**, pas l'existence du label comme état :
`p1-important` ou `bug` sont lus comme état par le feeder et dispatch-lib, mais
leur ajout ne met rien en mouvement — d'où leur filtrage. Écrire « un label lu
n'est jamais filtré » serait faux d'après le code ; la revue l'a relevé.

## La sonde (AC5)

Les écarts, par classe et par agent :

```sql
SELECT agent_id, after_value AS classe, count(*)
  FROM audit_events
 WHERE tool_name = 'webhook_prefilter_skipped'
   AND created_at > strftime('%Y-%m-%dT%H:%M:%SZ','now','-18 hours')
 GROUP BY 1, 2;
```

Les tours à vide RESTANTS, par type d'événement (mêmes classes, même fenêtre),
à comparer à la mesure fondatrice :

```sql
WITH ev AS (
  SELECT m.session_id, s.agent_id, substr(m.content, 1, 60) AS head
    FROM messages m JOIN sessions s ON s.id = m.session_id
   WHERE m.role = 'user'
     AND m.created_at > strftime('%Y-%m-%dT%H:%M:%SZ','now','-18 hours')
     AND (m.content LIKE '[GitHub] Check suite success on % (branch: main)%'
          OR m.content LIKE '[GitHub] Issue labeled %')
)
SELECT ev.agent_id, ev.head, count(DISTINCT l.session_id) AS sessions,
       sum(l.input_tokens) AS tokens_entree
  FROM ev JOIN llm_calls l ON l.session_id = ev.session_id
 GROUP BY 1, 2 ORDER BY tokens_entree DESC;
```

Cible : ≥ 80 % de baisse des tokens d'entrée des classes filtrées. Un label lu
par la boucle (`ready`, `blocked`, `operator-*`, …) doit, lui, continuer
d'apparaître dans la seconde requête : c'est le contrôle négatif.

## Découpage

Phase 1 (cette PR) : (a) `check_suite` verte sur `main`, (d) label inerte.
Phase 2 : (c) `pr_review` non actionnable sans tâche — exige l'état de la PR,
donc une couture de forge bouchonnable. Phase 3 : (b) `check_suite` verte sur
une PR déjà décidée et notifiée pour cette tête — le SHA n'est pas dans le texte
du gateway ; la décision vit dans `ci_success_handler` après `find_open_pr`.
