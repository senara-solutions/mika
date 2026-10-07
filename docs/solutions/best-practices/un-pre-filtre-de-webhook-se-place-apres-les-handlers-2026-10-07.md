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

## Phase 2 : « texte intact » est relatif au handler qui a déjà parlé

La règle de la phase 1 excluait la classe (c) par construction : sur un
`pr_review` sans tâche, le `verdict_handler` **enrichit ou remplace toujours**
le texte (`hold_review_without_task`, `refuse_pass_from_non_reviewer`, le
pré-digest du verdict illisible). Ce n'est pas un signal « il a quelque chose à
dire au modèle » : ce qu'il dit est précisément « rien d'actionnable ». La porte
compare donc, pour (c), le texte final au texte **juste après** le
`verdict_handler` — un handler ultérieur qui y touche garde le tour. Une seule
règle, paramétrée par le handler responsable de la classe ; pas une exception.

Trois décisions qui ne vont pas de soi :

- **Une revue sans ligne `VERDICT:` n'est pas un verdict.** `Verdict::Missing`
  confond « ligne illisible » et « pas de ligne » ; seule la première est dans
  (c). La seconde est la forme d'une consigne écrite en revue, et l'identité
  GitHub de l'opérateur est partagée (AC2). `verdict_raw_value` sépare les deux.
- **« Sans tâche active » = `find_active_task_by_pr_url` rend `None`**, tout
  statut non terminal comptant comme actif — plus large que
  `find_task_for_verdict` (`in_progress` seul). Un faux « actif » coûte un tour,
  un faux « inactif » perdrait un événement.
- **La forge en dernier.** Les trois termes lus dans le texte (`hold[review]`
  déjà remis à l'opérateur, auteur ≠ relecteur QA, valeur illisible) décident
  sans `gh` ; `gh pr view --json state` n'est payé que si aucun ne tient.
- **« Non actionnable » = « déjà refermé par quelqu'un ».** Le terme
  d'identité ne couvre que le `pass` d'un non-relecteur, parce que c'est le
  seul que le `verdict_handler` refuse et audite. Un `block[*]` d'une autre
  identité ne déclenche aucune notification ; l'écarter aussi aurait fait
  disparaître sans trace une consigne de l'opérateur (identité partagée).
  Avant d'écarter, vérifier qu'un autre site a déjà dit à quelqu'un ce qui se
  passe — la revue l'a attrapé, pas les tests, qui épinglaient la version large. État
  inconnu, pas de jeton, délai dépassé : le tour a lieu (AC3).

La couture d'état devient un trait (`PrefilterState`) : trois recherches,
bouchonnées en test, aucun appel GitHub réel.

Sonde de la classe (c), par terme :

```sql
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'webhook_prefilter_skipped'
   AND after_value LIKE 'verdict_%'
 GROUP BY 1;
```

Contrôle négatif : `grep webhook_prefilter_state_unreadable` avec
`what = "pr_state"` — non vide en continu signifie que la forge n'est jamais
lisible (jeton), donc que le terme « PR fermée » est inerte et ne compte rien.

## Découpage

Phase 1 : (a) `check_suite` verte sur `main`, (d) label inerte.
Phase 2 (livrée) : (c) `pr_review` non actionnable sans tâche. Phase 3 : (b) `check_suite` verte sur
une PR déjà décidée et notifiée pour cette tête — le SHA n'est pas dans le texte
du gateway ; la décision vit dans `ci_success_handler` après `find_open_pr`.
