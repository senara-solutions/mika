---
title: "fix(webhooks): pré-filtre déterministe, phase 3 — check_suite verte sur une tête déjà décidée et notifiée (mika#2675)"
type: fix
status: active
date: 2026-10-07
issue: senara-solutions/mika#2675
---

# Phase 3 — classe (b) : `check_suite` verte sur une tête déjà décidée et notifiée

## Pourquoi

Phases 1 (#2676) et 2 (#2678) mergées : classes (a), (c), (d), trait
`PrefilterState`, module `server/webhook_prefilter.rs`. Reste AC1(b) : une
`check_suite` `success` sur une branche de PR dont l'issue est **déjà décidée et
notifiée pour cette tête** (même PR, même SHA, même décision) ne doit produire
aucun tour LLM.

## Mesure : quels retours de `ci_success_handler` relèvent vraiment de (b)

Mesuré le 2026-10-07 sur `~/.mika/data/mika.db` (3 jours, `mika-dev`),
`audit_events` × `llm_calls` joints par `trace_id` (= `request_id` de
l'événement) :

| retour du handler | trace durable de la décision pour CETTE tête ? | trace de la notification ? | verdict |
|---|---|---|---|
| 1b — agent non dispatcher (`mika-qa`) | non (la décision n'est pas la sienne) | — | garde son tour |
| pas de jeton (`enrichment: Some`) | non | — | garde son tour |
| pas de PR ouverte / erreur `find_open_pr` | non | — | garde son tour |
| 2b — dédup mémoire / 2c — dédup audit | **non** : le marqueur `ci_success_handler_processed` est écrit **avant** l'évaluation ; il atteste « traitement commencé », pas son issue | non | garde son tour |
| 3 — pas de `VERDICT: pass` / 4 — verdict périmé | non (rien n'est notifié) | non | garde son tour |
| 5 — checks en attente / en échec | non | non | garde son tour |
| 5b — **DECISION-CORE** (`Handled`) | partielle : `ci_success_handler_human_gate_required` cible `pr:{repo}#{n}` **sans SHA** | **non** : l'issue de `sender.send` n'est que journalisée | **la classe (b)** — une fois la trace complétée |
| 5c — en retard sur `main` (`enrichment`) | non | non | garde son tour |
| 6 — signal merge-ready (`Handled`) | oui, mais le tour suivant est l'acteur de merge | — | garde son tour |

Observé : les traitements d'une même tête sont espacés de **plusieurs minutes**
(fenêtre de dédup : 60 s), si bien que le cas (b) réel est le **retraitement
complet** qui redécide DECISION-CORE : `mika#2672@4ab024d` (16:54:58 puis
17:06:51), `mika#2674@a634a46` (20:54:07 puis 21:05:18). Chacun renotifie
l'opérateur, rend `Handled`, et le tour LLM conclut « Already notified » /
« Duplicate » (≈ 65 k tokens d'entrée chacun).

Les dédups 2b/2c **ne relèvent pas de (b)** : rien n'y atteste ce que la tête a
décidé. Un `hold` DECISION-CORE n'est atteint qu'une fois **tous** les checks
terminés, donc une dédup < 60 s *après* un hold est structurellement improbable ;
le seul tour de dédup observé (`mika#2673@84975ef`, 19:18:59, 30 s après le
traitement) suivait un « pas de verdict », non notifié. Ils gardent leur tour.

## Décision

1. **Compléter la trace dans `ci_success_handler`** (seul changement de ce
   fichier, DECISION-CORE) : dans la branche 5b, **après** l'envoi de la
   notification, écrire une ligne `ci_success_handler_decision_core_hold`,
   cible `pr:{repo}#{n}@{head_sha}` (la forme de `ci_success_handler_processed`,
   factorisée dans `head_key`), `after_value` = `notified` si l'envoi a rendu
   `Delivered`, `not_notified` sinon, `reasoning` = l'issue précise. La ligne
   `ci_success_handler_human_gate_required` existante est inchangée (forme,
   place). Le handler ne change ni ses décisions, ni sa notification, ni son
   retour.
2. **Classe (b) dans le module des phases 1 et 2** : variante
   `PrefilterClass::GreenCheckSuiteHeadDecided`
   (`green_check_suite_head_decided`), candidat `GreenPrBranch` (toute
   `check_suite success` hors branche par défaut), cible d'audit
   `check_suite:{repo}#{n}@{sha}`.
3. **Trois termes, une fonction pure** (`head_decided_and_notified`) :
   - *décision attestée* : CET événement (lignes de son `trace_id`) a écrit une
     ligne `ci_success_handler_decision_core_hold` — il a lui-même redécidé
     DECISION-CORE ; le marqueur `processed` ne suffit pas ;
   - *même tête* : une ligne `…_decision_core_hold` d'un **autre** événement
     porte exactement la même cible `pr:{repo}#{n}@{sha}` (la recherche est
     faite sur le préfixe de PR ; l'égalité de SHA est dans la fonction pure) ;
   - *notification attestée* : cette ligne antérieure porte `notified`.
   - *antériorité* (ajouté en revue) : cette ligne a un `id` strictement
     inférieur à celle de cet événement — un premier traitement lent n'est pas
     sauté parce qu'un traitement plus tardif l'a devancé dans l'audit.
4. **Texte** : (b) exige que seul `ci_success_handler` ait touché le texte
   (`Touched::outside_ci_success_handler`), comme (c) l'exige du
   `verdict_handler`. Un handler ultérieur qui y touche garde le tour.
5. **Couture d'état** : `PrefilterState` gagne deux recherches (lignes d'audit
   de cet événement ; lignes `…_decision_core_hold` de la PR). `LiveState` porte
   le `request_id`. Bouchonnées en test ; aucun appel GitHub réel.
6. **Fail-safe (AC3)** : une recherche en erreur ⇒ le tour a lieu.
7. **AC2 inchangé** : `failure`/`timed_out` ne sont pas des candidats ;
   nouvelle tête ⇒ aucune ligne antérieure sur cette cible ⇒ tour ; premier
   traitement d'une tête ⇒ aucune ligne antérieure ⇒ tour.

## Estimation

Code + tests ≈ 380 lignes (handler ≈ 40, base ≈ 35, module ≈ 90,
`handlers.rs` ≈ 15, tests unitaires ≈ 110, tests d'intégration ≈ 90). Marge
×2,5 : ≈ 950 < 1000 — pas de découpage. Docs (plan, compound) en sus.

## Fichiers

- `crates/mika-agent/src/server/ci_success_handler.rs` — `head_key`, ligne
  `…_decision_core_hold` après la notification.
- `crates/mika-agent/src/evidence/audit.rs`, `async_db.rs` — lignes d'un outil
  par préfixe de cible.
- `crates/mika-agent/src/server/webhook_prefilter.rs` — classe (b).
- `crates/mika-agent/src/server/handlers.rs` — texte avant/après
  `ci_success_handler`, `request_id` dans `LiveState`.
- `crates/mika-agent/src/server/mod.rs` — tests d'intégration.
- `crates/mika-agent/tests/eval/test_ci_success_handler.rs` — ordre épinglé
  (notification puis attestation).

## Definition of Done

- Classe (b) écartée avec ligne d'audit nommée et zéro appel LLM.
- Mutations terme par terme (même tête, décision attestée, notification
  attestée) vues rouges, consignées dans le corps de la PR.
- `cargo test -p mika-agent` et `cargo test -p mika-common mika2398` verts.
- Sonde AC5 étendue à (b) dans la doc compound.
- PR : périmètre DECISION-CORE dit, merge par Vincent ; `Closes #2675`.

## Acceptance criteria

- [ ] AC1(b) — une `check_suite` `success` sur une branche de PR dont la tête
  (même PR, même SHA) a déjà été décidée DECISION-CORE **et** notifiée, et que
  cet événement redécide DECISION-CORE, ne produit aucun tour LLM et écrit
  `webhook_prefilter_skipped` / `green_check_suite_head_decided`.
- [ ] AC2 — `check_suite` en échec ou `timed_out` jamais filtrée ; nouvelle tête
  (SHA différent) jamais filtrée ; premier traitement d'une tête inchangé ;
  dédup 2b/2c et tout autre retour du handler gardent leur tour.
- [ ] AC3 — lignes d'audit illisibles : le tour a lieu.
- [ ] AC4 — `MIKA_WEBHOOK_PREFILTER` désarmé : rien n'est filtré.
- [ ] AC5 — sonde `llm_calls` × `audit_events` documentée pour (b).
- [ ] Chaque terme a sa mutation vue rouge.

## Hors périmètre

- Supprimer la renotification opérateur au retraitement d'une tête déjà
  notifiée (le handler renotifie aujourd'hui ; ce module ne change pas ce que
  le moteur fait). Candidat à un ticket séparé.
- Les tours `check_suite` de `mika-qa` et les « pas de verdict » de `mika-dev` :
  aucune décision notifiée ne les ferme.
- Router vers un seul agent.
