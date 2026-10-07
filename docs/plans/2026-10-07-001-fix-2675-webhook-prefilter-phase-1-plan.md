---
title: "fix(webhooks): pré-filtre déterministe — phase 1, check_suite verte sur main et label inerte"
issue: senara-solutions/mika#2675
type: fix
status: active
date: 2026-10-07
phase: 1 sur 3 (phase 2 = classe (c), phase 3 = classe (b))
product_contract_source: ce-plan-bootstrap
---

# fix(webhooks) mika#2675 — phase 1 : aucun tour LLM sur une check_suite verte de `main` ni sur un label inerte

## Pourquoi

Mesuré le 2026-10-07 (`messages` × `llm_calls`, fenêtre de 30 h, base de prod) :
chaque `check_suite` verte sur `main` coûte **deux** tours LLM, un par agent de
la diffusion mika#1711. mika-qa répond « CI green on `main` — no review action
needed » (≈ 44 k tokens d'entrée), mika-dev répond « Duplicate. CI green on main.
No action. » (≈ 55 k). Seize de ces sessions en 30 h. Même forme pour les labels
sans effet : `p1-important`, `bug`, `loop-substrate`, `p2-normal` posés sur un
ticket donnent chacun un tour mika-dev (≈ 65 k) qui conclut « Pas de `ready`. Pas
de dispatch. » — ce que le prompt `self-dev` prescrit déjà (« acknowledge, do NOT
dispatch »). Le moteur a tout ce qu'il faut pour le savoir sans modèle.

## Découpage (contrainte du vol : marge ×2,5, seuil 1000)

Le ticket entier, estimé brut, fait ≈ 1 200 lignes tests compris : ≈ 3 000 avec la
marge. Trois phases, chacune sous le seuil :

| phase | classes | pourquoi séparée |
|---|---|---|
| **1 (ce plan)** | (a) `check_suite` verte sur `main` ; (d) label inerte ; la porte, l'interrupteur, l'audit, les contrôles négatifs d'AC2 | aucune donnée de forge : décidable sur le texte et la base locale |
| 2 | (c) `pr_review` sans tâche active, verdict non actionnable | le terme « PR fermée ou mergée » exige un état de forge, donc une couture bouchonnable (AC3 côté forge) |
| 3 | (b) `check_suite` verte sur une PR déjà décidée et notifiée pour cette tête | le SHA n'est pas dans le texte du gateway ; la décision vit dans `ci_success_handler` après `find_open_pr`, qu'il faut rendre bouchonnable |

La porte de la phase 1 est le point d'ancrage des deux suivantes : elles ajoutent
une classe et ses faits, pas un second mécanisme.

## Décisions

### KTD1 — La porte vient APRÈS la chaîne des handlers déterministes, avant `run_agent`

Dans `run_agent_for_message`, entre le dernier handler structurel
(`upstream_close_handler`) et `agent::run_agent`. Raison : les handlers ont des
effets (évaluation de merge, notification opérateur, nettoyage de lignes de
suivi) que le filtre ne doit jamais retirer. Le filtre ne change pas ce que le
moteur **fait** ; il change seulement si l'on **demande ensuite** au modèle. Le
précédent mika#2671 B2 (`skip_superseded_review_turn`) se place en tête parce
qu'il annule le tour entier ; ici les handlers doivent tourner.

### KTD2 — (a) et (d) exigent un texte intact après les handlers

Si un handler a remplacé ou enrichi `req.text`, il a quelque chose à dire au
modèle : le tour a lieu. Sur `main`, l'évaluateur de mika-dev rend
`Passthrough { enrichment: None }` (pas de PR ouverte) et celui de mika-qa sort à
la porte mika#2260 sans toucher au texte : les deux restent filtrables.

### KTD3 — Une fonction pure classe, une recherche d'état décide, fail-safe vers le LLM

`classify(text) -> Option<Candidate>` est pure et lit **la première ligne du
gateway** via les grammaires existantes (`webhook_queue_v2::classify_event`, pas
de troisième regex). La seule recherche d'état de la phase 1 est
`find_active_task_by_branch(branch)` pour (a) : `Ok(None)` filtre, `Ok(Some)` et
`Err` laissent passer (AC3 : on paie, on ne perd rien).

### KTD4 — L'ensemble des labels lus par la boucle est une constante unique, composée de ses lecteurs

`webhook_dispatch::label_read_by_loop(label)` consulte : `READY_LABEL` (déplacé
de `webhook_queue_v2` vers `webhook_dispatch`, lu par la file v2), 
`OPERATOR_HELD_LABELS` (lu par le feeder et `ready_label_handler`), le préfixe
`operator-`, le préfixe de siège `DISPATCH_SEAT_LABEL_PREFIX`, et
`LOOP_AUTOMATION_LABELS` (labels d'automatisation de `.github/labels.yml` lus
par dispatch-lib ou le moteur : `loop-substrate`, `needs-build`, `needs-deploy`,
`wip-rescue`, `human-review-required`, `needs-multi-agent-review`,
`rescue-after-review`, `stale-against-main`, `pipeline-exempt`, et les préfixes
`phase:` et `origin:`). Large à dessein : un label lu à tort comme inerte perd
un événement, un label inerte lu à tort comme actif coûte un tour.

### KTD5 — Interrupteur `MIKA_WEBHOOK_PREFILTER` par `qa_head_supersession::parse_switch`

Même table de vérité que les interrupteurs de mika#2671 : absent/vide/`1`/`true`/
`on`/`yes` → armé ; `0`/`false`/`off`/`no` → désarmé ; une coquille est dite
(WARN `qa_switch_invalid` nommant la variable) et laisse armé. Lu à chaque
événement : pas de redémarrage pour désarmer.

### KTD6 — Une ligne d'audit nommée par événement écarté

`tool_name = "webhook_prefilter_skipped"`, `target_key` = la cible
(`check_suite:{repo}@{branch}` ou `issue:{repo}#{n}`), `after_value` = la classe
(`green_check_suite_default_branch`, `inert_label`), `reasoning` =
`request_id=<uuid>`. Plus un `info!` du même nom. C'est la moitié « filtré » de la
sonde AC5.

## Ce que la phase 1 ne fait PAS

- Aucun filtre sur `pr_review` (phase 2), ni sur une `check_suite` de branche de
  PR (phase 3) : mika-qa y lance réellement ses revues (`qa-review-webhook-success`).
- Ne touche pas `verdict_handler.rs` (CODEOWNERS) ni `ci_success_handler.rs`.
- Une session vide est encore créée avant la porte (les handlers en ont besoin) :
  coût nul en tokens, nommé ici pour qu'on ne le prenne pas pour un tour.

## Unités

- **U1** `server/webhook_prefilter.rs` (neuf) : `PrefilterClass`, `Candidate`,
  `classify`, `decide`, `prefilter_enabled`, `skip_turn` (porte + audit).
- **U2** `webhook_dispatch.rs` : `READY_LABEL`, `LOOP_AUTOMATION_LABELS`,
  `label_read_by_loop`. `webhook_queue_v2` lit `READY_LABEL` de là.
- **U3** `server/handlers.rs` : capture du texte d'origine, appel de la porte.
- **U4** sonde AC5 dans le doc de compound.

## Tests (préfixe `mika2675_`, aucun appel GitHub, LLM bouchonné qui compte)

De bout en bout par `run_agent_for_message`, sur le compteur `calls_made()` du
`MockLlmProvider` (même montage que mika#2671 B2) :

- (a) `[GitHub] Check suite success on senara-solutions/mika (branch: main)` ⇒ 0 appel, 1 ligne d'audit.
- (d) `[GitHub] Issue labeled p1-important on …#2675 — …\n…\nLabeled by: @samidarko` ⇒ 0 appel.
- Contrôles négatifs AC2, un par événement légitime ⇒ ≥ 1 appel : PR `opened`,
  `synchronize`, `ready_for_review`, `review_requested`, `check_suite` `failure`
  et `timed_out`, commentaire (de l'opérateur), label `ready`, PR `closed`
  (`Merged: false`) et `closed` mergée, verdict `pass` du relecteur QA.
- Mutations terme par terme, chacune ⇒ ≥ 1 appel : (a) conclusion `failure` ;
  branche de PR ; tâche active sur la branche ; recherche de tâche en erreur
  (AC3) ; (d) chaque famille de l'ensemble lu (`ready`, `blocked`,
  `operator-*`, `dispatch:*`, automatisation) ; interrupteur `off` (AC4).
- Unitaires : `classify` (grammaire), `label_read_by_loop`, table de
  `parse_switch` pour `MIKA_WEBHOOK_PREFILTER`.

Puis contrôle négatif par mutation du code de production, terme par terme, vu
rouge, sur un commit vert.

## Taille estimée

Brut : ≈ 150 lignes de code (U1 ≈ 110, U2 ≈ 25, U3 ≈ 15) + ≈ 230 de tests ≈ 380 ;
marge ×2,5 ≈ 950, sous le seuil de 1000 : pas de découpage supplémentaire. Réel :
rapporté dans la PR.

Total estimé : 950 lignes

## Definition of Done

- [ ] `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
      `cargo test -p mika-agent`, `cargo test -p mika-common mika2398` verts — un seul
      build cargo à la fois.
- [ ] Chaque terme de (a) et (d) vu rouge par mutation du code de production, commit vert d'abord.
- [ ] Aucun appel GitHub réel depuis les tests ; aucun `#[cfg(test)] pub fn` ajouté.
- [ ] `/ce:compound` traversé (sonde AC5 incluse) ; `cargo clean` au push ; un seul push.

## Acceptance criteria

Transcrits du ticket (mika#2675). Cochés : ce que la phase 1 livre.

- [ ] **AC1. Classes filtrées, décidées sans LLM.** Avant tout tour LLM, une fonction pure classe l'événement. Les classes suivantes sont écartées avec une ligne d'audit nommée (classe et cible) et **zéro appel LLM** :
  - [x] (a) `check_suite` de conclusion `success` sur la branche par défaut, sans tâche active liée au SHA ou à la branche ; — *phase 1*
  - [ ] (b) `check_suite` `success` sur une branche de PR dont l'issue est déjà décidée et notifiée pour **cette tête** ; — *phase 3*
  - [ ] (c) `pr_review` sans tâche active pour la PR, et dont le verdict n'est pas actionnable ; — *phase 2*
  - [x] (d) `issue_labeled` dont le label n'est dans aucun ensemble que la boucle lit. L'ensemble est une constante unique, partagée avec ses lecteurs. — *phase 1*
- [x] **AC2. Les événements légitimes passent toujours.** PR `opened`, `synchronize`, `ready_for_review`, `review_requested`, `check_suite` en échec ou `timed_out`, tout commentaire, le label `ready`, `closed` et `merged`, un verdict `pass` du relecteur QA sur une PR avec tâche active. — *phase 1 pose les contrôles ; 2 et 3 les gardent verts*
- [ ] **AC3. Fail-safe vers le LLM.** Base illisible ⇒ le tour a lieu. — *phase 1 pour la base ; « PR introuvable » avec la phase 2*
- [x] **AC4. Kill-switch** `MIKA_WEBHOOK_PREFILTER` à trois paliers. — *phase 1*
- [ ] **AC5. Mesure.** Sonde documentée ; cible ≥ 80 % de baisse sur (a) à (d). — *sonde en phase 1 ; mesure après la phase 3*
