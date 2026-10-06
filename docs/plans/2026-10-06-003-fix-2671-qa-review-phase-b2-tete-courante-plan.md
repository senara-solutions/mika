---
title: "fix(qa-review): une revue par tête stable — phase B2, contrôle de tête courante au tour de revue"
issue: senara-solutions/mika#2671
type: fix
status: active
date: 2026-10-06
phase: B2 (dernière phase ; A #2672 et B1 #2673 mergées)
product_contract_source: ce-plan-bootstrap
---

# fix(qa-review) mika#2671 — phase B2 : tête encore courante au démarrage du tour de revue

## Pourquoi

AC2 exige deux contrôles : avant chaque tour de **callback de build** (livré
par la phase A, #2672) et avant chaque tour de **revue**. B2 livre le second.

Le trou que B1 laisse ouvert, mesurable dans son propre code : l'anti-rebond
(`server::sync_debounce`) retire un `synchronize` A de sa table à l'échéance et
le **verse en file**. Si un `synchronize` B de la même PR arrive pendant
qu'A attend en file, B **ouvre une nouvelle fenêtre** : la table ne contient
plus A, et la coalescence `webhook_queue_v2` ne fusionne qu'entre éléments
en file. A sort de file, et paie un tour de revue complet (0,6 à 1,1 M de
tokens d'entrée, corps de mika#2671) sur une tête que B a déjà remplacée,
puis B est revu à son échéance. Même forme quand le tour d'A attend le verrou
de l'agent derrière un autre tour.

## Le critère : une retenue plus récente que la sienne

Au démarrage d'un tour sur un `synchronize` d'identité `request_id = R`, pour
la clé `pr:{repo}#{pr}` :

> **Périmé** ⇔ le registre `qa_pr_sync_observed` contient une ligne de retenue
> à soi (`after_value = stage=held`, `reasoning = request_id=R`) **et** au
> moins une ligne de retenue d'une **autre** identité, d'`id` strictement
> supérieur.

### KTD1 — Retenues seulement, pas lignes de tour démarré

Une retenue est **durable** (B1, KTD3) : elle reste pendante jusqu'à ce que son
tour démarre, et se rejoue au démarrage du worker comme au balayage. Sauter A
parce qu'une retenue B plus récente existe ne perd donc aucune revue : B sera
revue. Par récurrence, **la retenue la plus récente d'une clé n'est jamais
sautée** (aucune retenue plus récente qu'elle) : la dernière tête est toujours
revue (AC4 tient par construction). Une ligne de tour démarré d'une autre
identité ne donne pas cette garantie (elle ne dit pas que sa tête est plus
récente), donc elle ne compte pas.

### KTD2 — Pas de retenue à soi ⇒ aucun témoin ⇒ le tour tourne

Un `synchronize` peut arriver au tour sans retenue : anti-rebond désarmé,
chemin hérité (`MIKA_WEBHOOK_QUEUE_ENABLED=false`), écriture `stage=held` en
échec (B1 le verse alors en file directement). Sans retenue à soi, on ne sait
pas situer l'événement par rapport aux retenues de la clé : une retenue
**plus ancienne** pendante ne doit pas faire sauter une tête plus récente. Le
tour tourne, comme aujourd'hui.

### KTD3 — Aucun second registre

La lecture passe par une seule requête sur `audit_events`, même `tool_name`,
même clé (`sync_observed_key`), même marqueur (`sync_debounce::request_marker`),
même horizon que la reprise (`RECOVERY_HORIZON_SECS`). Rien n'est écrit au
registre sur un tour sauté : il n'a pas démarré, et la retenue d'A n'est déjà
plus pendante (une retenue plus récente existe). La seule écriture est la
ligne d'observabilité nommée `qa_review_head_superseded` (journal + une
ligne `audit_events` que rien ne lit pour décider), même forme que
`qa_build_callback_superseded` en phase A.

### KTD4 — Fail-safe dans le sens de la revue

Registre illisible ⇒ le tour tourne (WARN `qa_review_head_unreadable`).
Kill-switch `MIKA_QA_STALE_REVIEW_GUARD` (armé par défaut, `parse_switch`
partagé avec A et B1 : `0`/`false`/`off`/`no` désarme, coquille ⇒ WARN + armé).

### KTD5 — Placement : en tête de `run_agent_for_message`

Point de passage des trois chemins qui lancent un tour (drain v2, chemin
hérité, rejeu #528), **avant** `record_pr_sync_observed` et avant toute
création de session ou résolution de jeton : un tour sauté ne crée ni session,
ni appel LLM, ni appel GitHub. Zéro appel LLM (AC2 autorise « au plus un appel
de constat, ou zéro »). Le verrou de l'agent est relâché au retour ; le worker
de drain sort l'événement de l'état « en vol » comme pour un tour normal.

## Ce que B2 ne fait PAS

- Aucun appel GitHub (ni lecture de la tête réelle : même raison que la phase A,
  la garde no-diff #886 du gateway ferait sauter la seule revue d'un amend).
- Ne touche ni l'anti-rebond, ni la garde de callback de la phase A.
- Aucun chemin CODEOWNERS (`perimeter/`, `verdict_handler.rs`,
  `pr_merge_with_gate.rs`, `docs/gate/`, `docs/egress/`, `.github/CODEOWNERS`).
- Aucun prompt modifié, aucun label.

## Unités

### U1 — Requête (`db.rs`, `async_db.rs`)

`newer_audit_holds(agent, tool, held_value, target_key, identity, since)` →
`(Option<i64> own_id, i64 newer)`. `own_id` = `MAX(id)` des retenues portant
`identity` ; `newer` = retenues de la clé d'`id` supérieur et d'identité
différente. `own_id` NULL ⇒ `newer = 0`.

### U2 — Décision pure (`qa_head_supersession.rs`)

`ReviewTurnHead { NotASync, NoWitness, Current, Superseded { target,
newer_holds }, LedgerUnreadable { target } }` et `decide_review_turn(text,
lookup)` ; seul `Superseded` saute. Constantes `REVIEW_HEAD_SUPERSEDED_EVENT`,
`REVIEW_HEAD_UNREADABLE_EVENT`, `STALE_REVIEW_GUARD_ENV`.

### U3 — Garde (`server/handlers.rs`)

`skip_superseded_review_turn(db, text, request_id) -> bool`, appelée en tête de
`run_agent_for_message`. **SOLE WRITER** de `REVIEW_HEAD_SUPERSEDED_EVENT`.

## Tests (tous préfixés `mika2671_`, aucun appel GitHub)

- U1 (base en mémoire) : retenue A puis B ⇒ A voit 1 retenue plus récente, B
  en voit 0 ; sans retenue à soi ⇒ `own_id = None` ; une retenue d'une autre PR
  ou d'un autre agent ne compte pas.
- U2 : chaque variante, et `lookup` jamais appelé hors `synchronize`.
- U3 : `held(A)`, `held(B)` ⇒ le tour d'A est sauté, une ligne
  `qa_review_head_superseded`, aucune ligne de tour démarré pour A ; contrôle
  négatif : le tour de B n'est pas sauté.
- AC2 de bout en bout sur `run_agent_for_message`, LLM bouchonné qui compte :
  tête périmée ⇒ **0 appel LLM** ; contrôle négatif : tête courante ⇒ ≥ 1
  appel, ligne de tour démarré écrite (comportement inchangé).
- Structurel : l'appel vit dans `run_agent_for_message`, avant
  `record_pr_sync_observed` ; un seul écrivain du nom d'audit.

## Estimation

Brut ≈ 70 lignes de code + ≈ 100 de tests ≈ 170 ; marge ×2,5 ≈ 175 annoncée
par le vol pour le code, ≈ 425 tests compris. Réel : rapporté dans la PR.

## Definition of Done

- [ ] `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt
      --check`, `cargo test -p mika-agent --lib mika2671`, `cargo test -p
      mika-common mika2398` verts — un seul build cargo à la fois.
- [ ] Chaque contrôle négatif vérifié rouge par mutation (commit vert d'abord).
- [ ] Aucun appel GitHub réel depuis les tests ; aucun `#[cfg(test)] pub fn`
      ajouté (recensement mika2398).
- [ ] `/ce:compound` traversé ; `cargo clean` au push ; un seul push.

## Acceptance criteria

Transcrits du ticket (mika#2671).

- [x] **AC1. Anti-rebond.** Un `synchronize` ouvre une fenêtre d'attente (~5 min, réglable, trois paliers maison). Seule la **dernière** tête reçue dans la fenêtre déclenche un tour de revue. Tests : trois synchronize dans la fenêtre ⇒ un seul tour, sur la dernière tête ; contrôle négatif : deux synchronize séparés de plus que la fenêtre ⇒ deux tours. — *B1 (livré, #2673)*
- [ ] **AC2. Tête encore courante.** Avant chaque tour de revue **et** chaque tour de callback de build, le moteur vérifie que la tête visée est toujours la tête de la PR. Sinon le tour se termine sans appel LLM complet (au plus un appel de constat, ou zéro) et laisse une ligne d'observabilité nommée. Tests : tête périmée ⇒ aucun tour LLM complet ; contrôle négatif : tête courante ⇒ comportement inchangé. — *A : callback (livré) ; B2 : tour de revue (ce plan)*
- [x] **AC3. Build périmé.** Le callback de build d'une tête remplacée ne relance pas de tour LLM complet, qu'il ait été annulé (mika#2335) ou non. Test avec un callback sur une tête périmée. — *A (livré, #2672)*
- [x] **AC4. Aucune revue perdue.** La dernière tête est toujours revue, et un `opened` / `ready_for_review` / `review_requested` n'est pas retardé par la fenêtre. Contrôle négatif dédié. — *B1 (livré) ; B2 le préserve par construction (KTD1)*
- [x] **AC5. Mesure.** Cible rebasée : sur un épisode de N `synchronize` rapprochés, le coût total ne dépasse pas celui d'un seul cycle revue + build. Sonde post-déploiement documentée (requête `llm_calls` par session). — *B1 (mesure rejouée + sonde dans #2673) ; confirmation post-déploiement*
