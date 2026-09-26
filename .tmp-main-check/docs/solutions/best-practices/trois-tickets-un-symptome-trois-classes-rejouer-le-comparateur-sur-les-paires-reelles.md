---
title: "Trois tickets, un symptôme, trois classes : rejouer le mécanisme hors moteur sur les paires réelles avant de réparer"
date: 2026-09-16
category: best-practices
module: agent-core
problem_type: best_practice
component: tooling
severity: high
issue: senara-solutions/mika#2338
tags: [guards, review-anchor, grooming, mika-arch, diagnosis, dispatch-lib, fail-visible, markdown]
applies_when:
  - "Un guard fail-closed refuse plusieurs runs avec la même raison de refus (ici `QuoteNotInBrief`) et le ticket compte les occurrences comme une seule classe"
  - "Le refus d'un comparateur porte sur un texte que le modèle a lu rendu (markdown) alors que le comparateur lit la source"
  - "Un mécanisme qui retient un verdict fait dire au consommateur « verdict missing » sans cause lisible"
  - "Avant de toucher au seuil ou à la config d'un guard : la porte se répare par la cause, jamais par la config"
---

## Le contexte

mika#2338 arrivait avec « n=3 grooms substrat ne reconvergent pas : l'arch émet READY à 2/3 ancres, le guard retient, PIPELINE FAILURE opaque ». Le périmètre (0) du ticket demandait une **preuve**, pas une hypothèse : le brief comparé est-il le texte montré à l'architecte ? Et il réservait le seul fix de marqueur autorisé au cas où cette preuve exposerait un écart de normalisation.

Le n=3 était faux — pas dans le compte, dans la classe. Les trois grooms cités avaient trois causes différentes, et une seule relevait du comparateur.

## La méthode qui a séparé les classes

Tout est lisible sans rejouer un seul appel LLM. Le guard écrit ses compteurs dans `/var/log/mika/server.log` (`event = "guard.review_anchor"` puis, au second échec, `guard.review_anchor_withheld`), et la base garde le texte des deux côtés de la comparaison.

1. **Lister les événements du guard**, avec session et compteurs :
   `grep 'guard.review_anchor' /var/log/mika/server.log | jq -r '[.timestamp,.event,.session_id,.anchors_found,.anchors_valid,.miss_reason]|@tsv'`
2. **Récupérer le brief exact** (le message utilisateur du tour — c'est ce que `run_loop` reçoit comme `review_brief` — voir `crates/mika-agent/src/agent_loop/mod.rs`, appel en mode conversation) :
   `sqlite3 -readonly "file:$HOME/.mika/data/mika.db?mode=ro" "select content from messages where session_id='<s>' and role='user' order by id"`
3. **Récupérer la réponse de l'architecte** (la table `llm_calls` conserve `response_text` même quand la tâche a été annulée avant la sauvegarde du message assistant) :
   `sqlite3 -readonly … "select response_text from llm_calls where session_id='<s>' and stop_reason='EndTurn' order by created_at"`
4. **Rejouer le comparateur hors moteur** — même algorithme que `verify_review_anchors` (`crates/mika-agent/src/agent_loop/review_anchor.rs`) : fenêtre glissante de 40 caractères sur chaque ligne d'ancre, normalisée, cherchée dans le brief normalisé. Vingt lignes de Python suffisent, et elles disent **quelle** ancre rate et **pourquoi**.
5. **Vérifier que le brief comparé est bien le brief montré** : la fenêtre de contexte (mika#2295) n'élide jamais le message du tour (`truncate_history_to_token_budget`, test `mika2295_the_turn_s_own_question_is_never_elided`) et `save_message_with_task_context` insère le texte tel quel. Aucune transformation entre les deux lectures — la différence est donc dans la citation, pas dans le brief.

## Ce que le rejeu a rendu

| Groom | Ce que le log disait | Ce que la paire réelle disait | Classe |
|---|---|---|---|
| #2335, session `975d44d0` | `anchors_found=3 anchors_valid=2 QuoteNotInBrief` | A1 citait mot pour mot la ligne 63 du brief ; le brief porte `**Le mécanisme de kill, lui, est correct** : \`kill_process_gracefully\``, l'arch a écrit `Le mécanisme de kill, lui, est correct : kill_process_gracefully`. « Le mécanisme de kill, lui, est correct » fait 38 caractères avant le premier écart — aucune fenêtre de 40 ne survit à `**` et deux backticks. Rejoué en retirant `*` et `` ` `` des deux côtés : 3/3. | **Balisage markdown** — la seule classe du comparateur |
| #2296, session `09b565bb` | `anchors_found=3 anchors_valid=0` puis withheld | Les trois ancres citaient exactement… le brief de **#2293**, présent huit fois dans la fenêtre agent-wide de l'arch ce jour-là. Le guard a refusé une attestation sur le mauvais ticket : il a fait son travail. | **Contamination cross-ticket** — fermée par #2295 (`[context.history] scope = "session"`) |
| #2331, session `06615128` | aucun événement `guard.*` | Sept appels d'outils entre 07:07 et 07:10Z, puis plus aucun `EndTurn` jusqu'au cancel à 07:33Z. Le verdict n'est pas refusé : il n'est jamais rendu. | **Tour arch sans réponse après outils** — hors périmètre, à ficher à part si n=2 |

Réparer « le guard » sur la foi du n=3 aurait abaissé un seuil pour une classe qui n'existait pas deux fois sur trois, et laissé les deux autres intactes.

## Ce que ça a coûté au fix, et pourquoi il a la forme qu'il a

La classe réelle avait une mesure derrière elle : sur les 245 briefs de plus de 2 000 caractères reçus par mika-arch entre le 1er et le 16 septembre, 237 portent `**` et 243 des backticks. Un modèle qui lit un brief le lit rendu ; un comparateur qui exige les octets refuse tout lecteur honnête. D'où `normalize_for_anchor_match` dans `review_anchor.rs` : retrait de `*` et `` ` ``, repli de l'apostrophe typographique, puis repli des espaces — **symétrique** (brief et ancre), et le seuil de 40 se mesure **après**, pour que le balisage n'achète pas de longueur. Une paraphrase, ou une citation exacte d'un autre brief (le cas #2296), reste refusée : les tests du module portent les deux paires réelles.

Le second échec ne retient plus la disposition : il la réécrit en `ESCALATE` de la même famille avec une ligne `F1: (BLOCKING) [mika-engine] review-anchor: … anchors_found=… anchors_valid=… miss_reason=…`, que `dispatch-lib` lit au tier 0b avant tout grep textuel et recopie dans `RESULT`. Fail-closed n'est pas fail-visible : le withhold laissait le consommateur dire « verdict missing », et c'est l'opérateur qui lisait `server.log`. Voir `escalate_unattested_disposition` (`crates/mika-agent/src/agent_loop/mod.rs`) et `_engine_escalation_line` (`skills/bundled/_shared/dispatch-lib.sh`).

## Quand appliquer

- Un guard refuse plusieurs fois **avec la même raison** : avant de toucher quoi que ce soit, tirer la paire réelle (texte comparé, texte cité) de chaque occurrence et rejouer le mécanisme. Le log dit *combien* ; seule la paire dit *quoi*.
- Le compte d'un ticket est une hypothèse de classe, pas une mesure. Trois occurrences d'un symptôme sont trois enquêtes tant que le rejeu n'a pas montré la même cause.
- Un comparateur « verbatim » sur une source markdown compare des octets à un rendu. Normaliser des deux côtés est un fix de marqueur légitime ; abaisser le seuil ne l'est jamais (bearing Prime sur #2338 : « une porte se répare par la cause »).

## Voir aussi

- `an-exempted-disposition-is-the-attack-surface-2026-08-30.md` — la naissance du guard (mika#2037) et la raison du fail-closed.
- `a-guard-anchored-on-the-shape-of-its-subject-loses-sight-of-it-2026-08-30.md` — le tier 0b ancre sur un littéral à un seul émetteur ; garder ce site unique.
- `../architecture-patterns/guard-parser-must-be-as-permissive-as-downstream-consumer-2026-08-29.md` — le drift guard Rust↔shell qui lie la regex à la constante, pas le commentaire.
- `feedback_n_equals_2_is_the_signal` et `feedback_estimated_counts_undercount_measured` (mémoire orchestrateur) — mesurer, ne pas raconter : ici c'est le compte lui-même qui racontait.
