---
title: "fix(qa-review): une revue par tête stable — phase B1, anti-rebond durable sur synchronize"
issue: senara-solutions/mika#2671
type: fix
status: active
date: 2026-10-06
phase: B1 (sur B1 + B2)
product_contract_source: ce-plan-bootstrap
---

# fix(qa-review) mika#2671 — phase B1 : anti-rebond durable sur `synchronize`

## Pourquoi

La phase A (#2672, 2c0faae2) a supprimé le tour LLM du callback de build d'une
tête remplacée. Elle ne touche pas à la cause amont : **chaque `synchronize`
déclenche un tour de revue**, parce que la coalescence de `webhook_queue_v2`
(clé `pr_sync:{repo}:{pr}`) ne fusionne que des événements **encore en file**,
et que la file de mika-qa est presque toujours vide (corps de mika#2671).

Épisode de référence (mika#2659, 2026-10-06, mesuré dans `llm_calls`, détail
dans le plan de phase A) : trois `synchronize` à 14:21:21, 14:22:30, 14:32:12 ;
trois revues (0,58 / 0,71 / 1,06 M) et deux callbacks (1,03 / 1,16 M) ≈ 4,5 M.

**Cible AC5 rebasée** (dernier commentaire du ticket, 2026-10-06T18:01Z) : sur
un épisode de N `synchronize` rapprochés, le coût total ne dépasse pas celui
**d'un seul cycle revue + build**.

## Découpage (contrainte du vol : marge ×2,5, phases au-delà de 1000 lignes)

Estimation brute du périmètre B demandé (AC1, AC2-revue, AC4 complet, AC5) :
≈ 490 lignes (code ≈ 270, tests ≈ 220), ×2,5 ≈ 1 225 > 1 000. **Découpage** :

- **B1 (ce plan)** — AC1 (anti-rebond), AC4 complet (aucun retard pour
  `opened`/`ready_for_review`/`review_requested` ; la dernière tête est revue,
  **y compris à travers un redémarrage pendant la fenêtre**), le couplage avec
  la garde de phase A, et la mesure AC5 rejouée sur l'épisode de référence.
  Brut ≈ 390, ×2,5 ≈ 975.
- **B2 (plan séparé, branche neuve depuis main après B1)** — AC2 côté tour de
  revue : au démarrage d'un tour sur un `synchronize`, si une tête plus récente
  est **retenue** par l'anti-rebond, le tour se termine sans appel LLM et laisse
  une ligne nommée. Brut ≈ 70, ×2,5 ≈ 175.

**Pourquoi cette frontière et pas l'inverse.** La durabilité ne peut pas partir
en B2 : la fenêtre **crée** le risque de perdre la revue de la dernière tête à
un redémarrage (plan de phase A, « Découpage »), et les redémarrages tombent
précisément sur ces épisodes — un merge de PR empilée produit à la fois un
`synchronize` sur la PR fille et un `make deploy`. AC2-revue, lui, ne fait
qu'économiser un tour **sans jamais en perdre** : sans B2, une tête périmée
déjà en file est revue pour rien, comme aujourd'hui. B1 est donc sûre seule ;
B1 sans durabilité ne le serait pas.

La PR B1 porte `Part of #2671` : le ticket reste ouvert pour B2.

## Décisions

### KTD1 — Fenêtre fixe ouverte par le premier `synchronize`, dernier gagnant

Un `synchronize` pour une PR sans fenêtre ouverte **ouvre** une fenêtre de W
secondes ; tout `synchronize` reçu pour la même PR pendant la fenêtre
**remplace** l'événement retenu. À l'échéance, le dernier retenu est versé dans
`webhook_queue_v2`, exactement comme s'il venait d'arriver. La fenêtre n'est
**pas** réarmée à chaque événement : un débit continu de pushes ne peut pas
retarder la revue indéfiniment (latence bornée à W).

Clé de fenêtre = `qa_head_supersession::sync_observed_key(repo, pr)` — la clé du
registre de phase A, écrite une fois (leçon mika#2158).

### KTD2 — Seul `synchronize` est retenu (AC4, premier volet)

La décision de retenir passe par `webhook_queue_v2::classify_event` :
seul `PullRequestSync` est retenu. `opened`, `ready_for_review`,
`review_requested`, `closed` sont classés `Other` et suivent le chemin actuel —
la fenêtre ne peut pas les retarder par construction. Une revue (`PR review`),
un `check_suite`, un label : inchangés.

### KTD3 — Durabilité : réutiliser le registre `qa_pr_sync_observed`, pas en créer un second

Consigne du vol. Chaque `synchronize` retenu écrit une ligne dans le registre de
phase A, **même `tool_name`, même clé**, distinguée par
`after_value = "stage=held"` et portant le texte de l'événement dans
`before_value`. Les lignes de phase A (tour démarré) gardent `after_value`
NULL : pas de migration, les lignes déjà en base restent lisibles.

**Pendant** = la dernière ligne du registre pour une clé est une ligne
`stage=held` (aucun tour démarré depuis, aucune retenue plus récente), dans un
horizon de 24 h. Au démarrage du worker de drain
(`spawn_webhook_drain_worker`, point unique des deux sites de démarrage —
`server/mod.rs` et la résolution paresseuse de `server/state.rs`), chaque
pendant est remis en fenêtre. Un redémarrage pendant la fenêtre, ou après
l'échéance mais avant le démarrage du tour, ne perd donc pas la dernière tête.

Résiduel nommé : un événement **versé** dans la file puis évincé par le
drop-oldest (file pleine à 64) reste « pendant » et n'est rejoué qu'au prochain
démarrage. Il exige 64 événements distincts en file pour mika-qa ; c'est le
même résiduel que la file v2 a aujourd'hui, borné cette fois par un rejeu.

### KTD4 — La garde de phase A compte les retenues (révision assumée de « démarré, pas reçu »)

La phase A ne saute un callback que si la revue de la tête suivante a
**démarré**, parce qu'un événement « reçu » pouvait être perdu (file en mémoire,
drop-oldest, redémarrage). Une retenue de B1 n'est pas un simple « reçu » : elle
est **durable** (KTD3) et se rejoue jusqu'à ce que son tour démarre. Elle
satisfait donc la propriété que la phase A exigeait : *la revue de la tête
suivante est garantie*.

Sans ce couplage, B1 **dégraderait** l'épisode de référence : la revue de
14:32:12 serait retenue jusqu'à 14:37:12, après le callback du build précédent
(~14:34), que la garde de phase A ne sauterait plus. Replay : ≈ 3,95 M, contre
≈ 3,5 M pour la phase A seule. Avec le couplage : ≈ 2,9 M (voir AC5).

Coût du couplage : **zéro ligne** dans la garde — `count_recent_audit_events_for_target`
compte déjà toutes les lignes du registre pour la clé. Ce qui change est la
documentation de `qa_head_supersession` et ses gardes structurelles.

### KTD5 — Réglage : trois paliers maison + kill-switch

- `MIKA_QA_SYNC_DEBOUNCE_SECS` — défaut `300`. Absent/vide → défaut ; illisible,
  `0`, négatif ou > 3600 → défaut + WARN nommant la valeur. Le `0` ne désarme
  pas (c'est le rôle du kill-switch).
- `MIKA_QA_SYNC_DEBOUNCE` — kill-switch, armé par défaut, `0`/`false`/`off`/`no`
  désarme, coquille → WARN + armé (forme de `MIKA_QA_STALE_BUILD_GUARD`).
  Désarmé : `synchronize` suit le chemin actuel ; les pendants éventuels sont
  versés immédiatement au démarrage (fenêtre nulle) plutôt que perdus.

Lecture d'environnement, pas de clé `config.toml` : même forme que la phase A,
aucun changement à `mika-common`.

## Ce que B1 ne fait PAS

- Pas de contrôle de tête au démarrage du tour de revue (AC2-revue) : B2.
- Pas de retenue sur le chemin hérité (`MIKA_WEBHOOK_QUEUE_ENABLED=false`) ni
  sur le rejeu #528 : `synchronize` n'est pas corrélé par #528, et le chemin
  hérité est un kill-switch de repli.
- Ne touche aucun chemin CODEOWNERS (`perimeter/`, `verdict_handler.rs`,
  `pr_merge_with_gate.rs`, `docs/gate/`, `docs/egress/`, `.github/CODEOWNERS`).
- Aucun prompt modifié, aucun appel GitHub ajouté.

## Unités

### U1 — Module `server/sync_debounce.rs`

État par agent `SyncDebounce` (table clé → dernier `MessageRequest` retenu,
verrou synchrone court) ; `hold` rend `Opened` ou `Replaced` ; `take` retire.
`debounce_key(text)` rend la clé pour un `synchronize`, `None` sinon.
`admit(debounce, queue, key, req, window)` retient et, sur `Opened`, lance la
tâche d'échéance qui `take` puis `enqueue` dans `WebhookQueue`. Parse des deux
variables (KTD5), fonctions pures testables sans base.

Champ `sync_debounce: Arc<SyncDebounce>` sur `AgentState` (`server/state.rs`),
deux sites de construction (`init_agent` et `test_state_full`).

**Tests** (`server/sync_debounce.rs`, temps tokio en pause) :
- AC1 : trois `synchronize` de la même PR dans la fenêtre ⇒ après W, **un**
  élément en file, et c'est le troisième texte.
- AC1 contrôle négatif : deux `synchronize` séparés de plus que W ⇒ deux
  éléments, dans l'ordre.
- Isolement : deux PR différentes dans la même fenêtre ⇒ deux éléments.
- Avant l'échéance : rien en file.
- KTD5 : paliers de la fenêtre (absent, vide, `0`, `-5`, `abc`, `99999` ⇒ 300 ;
  `120` ⇒ 120) ; kill-switch (absent armé, coquille armée, `off` désarmé).
- AC4 : `debounce_key` est `None` pour `opened`, `ready_for_review`,
  `review_requested`, `closed`, une revue, un check-suite ; `Some` pour
  `synchronize` (contrôle négatif).

### U2 — Ingestion dans `handle_message` (`server/handlers.rs`)

Sur le chemin v2, avant `webhook_queue_v2.enqueue` : si armé et
`debounce_key` rend une clé, écrire la ligne `stage=held` (fonction
`record_pr_sync_held`, à côté de `record_pr_sync_observed` — même fichier, le
SOLE WRITER de phase A reste `server/handlers.rs`), `admit`, audit
`webhook_queue_debounced` (`outcome=opened|replaced window_secs=`), 202
`accepted` — réponse identique pour le gateway.

**Tests** (routeur, `server/mod.rs`, aucun worker de drain, aucun appel
GitHub) :
- AC1 : POST d'un `synchronize` ⇒ 202, file v2 **vide**, une retenue, une
  ligne `stage=held` portant le texte.
- AC4 contrôle négatif dédié : une retenue en cours pour la PR, puis POST d'un
  `opened`, d'un `ready_for_review`, d'un `review_requested` pour la **même**
  PR ⇒ chacun est en file v2 immédiatement (profondeur 3), la retenue intacte.

### U3 — Pendants et reprise au démarrage (`db.rs`, `async_db.rs`, `handlers.rs`)

Requête `list_pending_held_syncs(tool, held_value, since)` : lignes
`stage=held` postérieures à `since` dont aucune ligne plus récente (`id`
supérieur) n'existe pour la même clé. Reprise dans
`spawn_webhook_drain_worker`, avant la boucle : chaque pendant est reconstruit
en `MessageRequest` (`channel = "github"`, agent courant, `request_id =
"held-replay-<uuid>"`) et passé à `admit` avec la fenêtre courante (nulle si
désarmé). Ligne `info!` `qa_sync_debounce_recovered count=`.

**Tests** :
- `db` : retenue seule ⇒ pendante ; retenue puis ligne de tour démarré ⇒ non
  pendante (contrôle négatif) ; deux retenues ⇒ seule la dernière ; autre agent
  ou hors horizon ⇒ rien.
- AC4 à travers un redémarrage : une ligne `stage=held` en base, un
  `SyncDebounce` neuf (redémarrage) ⇒ la reprise remet l'événement en fenêtre ;
  après W il est en file avec son texte. Contrôle négatif : la même ligne suivie
  d'un tour démarré ⇒ rien repris.

### U4 — Couplage phase A (`qa_head_supersession.rs`)

Documentation du module (KTD4) ; gardes structurelles mises à jour : le SOLE
WRITER de `SYNC_OBSERVED_TOOL` reste `server/handlers.rs` ; le placement « au
démarrage du tour » de `record_pr_sync_observed` reste vrai ; nouveau
placement : `record_pr_sync_held` n'est appelé que depuis `handle_message`.

**Tests** :
- AC5/couplage : build créé à `T`, retenue écrite après `T` ⇒ la décision de
  phase A rend `Superseded` (base SQLite réelle en mémoire). Contrôle négatif :
  retenue **antérieure** à `T` ⇒ `Current`.

## Definition of Done

- [ ] `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt
      --check`, tests du crate `mika-agent` verts — un seul build cargo à la fois.
- [ ] Chaque AC de B1 porte un test positif **et** un contrôle négatif, chaque
      contrôle négatif vérifié rouge par mutation (commit vert d'abord).
- [ ] Aucun appel GitHub réel depuis les tests.
- [ ] Mesure AC5 rejouée dans le corps de PR, sonde post-déploiement documentée.
- [ ] `/ce:compound` traversé ; `cargo clean` au push.

## Acceptance criteria

Transcrits du ticket (mika#2671). Phase de chaque case précisée.

- [ ] **AC1. Anti-rebond.** Un `synchronize` ouvre une fenêtre d'attente (~5 min, réglable, trois paliers maison). Seule la **dernière** tête reçue dans la fenêtre déclenche un tour de revue. Tests : trois synchronize dans la fenêtre ⇒ un seul tour, sur la dernière tête ; contrôle négatif : deux synchronize séparés de plus que la fenêtre ⇒ deux tours. — *B1*
- [ ] **AC2. Tête encore courante.** Avant chaque tour de revue **et** chaque tour de callback de build, le moteur vérifie que la tête visée est toujours la tête de la PR. Sinon le tour se termine sans appel LLM complet (au plus un appel de constat, ou zéro) et laisse une ligne d'observabilité nommée. Tests : tête périmée ⇒ aucun tour LLM complet ; contrôle négatif : tête courante ⇒ comportement inchangé. — *A : callback de build (livré) ; B2 : tour de revue*
- [x] **AC3. Build périmé.** Le callback de build d'une tête remplacée ne relance pas de tour LLM complet, qu'il ait été annulé (mika#2335) ou non. Test avec un callback sur une tête périmée. — *A (livré, #2672)*
- [ ] **AC4. Aucune revue perdue.** La dernière tête est toujours revue, et un `opened` / `ready_for_review` / `review_requested` n'est pas retardé par la fenêtre. Contrôle négatif dédié. — *B1*
- [ ] **AC5. Mesure.** Cible rebasée : sur un épisode de N `synchronize` rapprochés, le coût total ne dépasse pas celui d'un seul cycle revue + build. Sonde post-déploiement documentée (requête `llm_calls` par session). — *B1 : mesure rejouée + sonde ; confirmation post-déploiement*

## Mesure AC5 rejouée (épisode de référence, W = 300 s)

Durées tirées de l'épisode : revue ≈ 1,5 min avant lancement du build, build
6,5 à 7,5 min.

| heure | événement | B1 | coût |
|---|---|---|---|
| 14:21:21 | sync 1 | ouvre la fenêtre → 14:26:21 | — |
| 14:22:30 | sync 2 | remplace la retenue | — |
| 14:26:21 | échéance | revue de la tête 2 | 0,71 M |
| ~14:27:50 | build | lancé | — |
| 14:32:12 | sync 3 | **retenue durable** → fenêtre → 14:37:12 | — |
| ~14:34:30 | callback build tête 2 | **sauté** (retenue postérieure, KTD4) | 0 |
| 14:37:12 | échéance | revue de la tête 3 + build + callback | 1,06 + 1,16 M |

Total ≈ **2,9 M** (contre 4,5 M avant, 3,5 M phase A seule). L'écart de
9 min 42 s entre sync 2 et sync 3 dépasse la fenêtre : à W = 300 s, ce sont
**deux épisodes** au sens de la cible rebasée, chacun ≤ un cycle (le premier
coûte une revue seule, son callback est sauté). Les deux premiers `synchronize`,
eux, forment l'épisode « rapproché » type : une revue au lieu de deux.
Faire tenir l'épisode de 12 min entier dans un seul cycle (≈ 2,2 M) exige
W ≥ 651 s — réglable, mais c'est un choix de latence de revue qui revient à
l'opérateur, pas à ce plan.

### Sonde post-déploiement

```sql
-- retenues et rejeux par PR
SELECT created_at, target_key, COALESCE(after_value, 'stage=started') AS stage
FROM audit_events WHERE agent_id = 'mika-qa' AND tool_name = 'qa_pr_sync_observed'
ORDER BY created_at DESC LIMIT 40;
-- coût par session mika-qa sur la fenêtre d'un épisode
SELECT session_id, MIN(created_at), COUNT(*), SUM(input_tokens)
FROM llm_calls WHERE agent_id = 'mika-qa' AND created_at BETWEEN :t0 AND :t1
GROUP BY session_id ORDER BY 2;
```

Lecture attendue : pour N `synchronize` dans W, N lignes `stage=held` puis
**une** ligne `stage=started` ; dans `llm_calls`, une session de revue et au
plus une session de callback.

## Assumptions

- Les `synchronize` ne sont routés qu'à mika-qa (constat de phase A) ; la
  retenue s'applique à tout agent qui en reçoit un, sans effet ailleurs.
- Un rejeu au démarrage d'une PR fermée entre-temps coûte au pire un tour de
  revue sur une PR fermée ; horizon 24 h.

## Deferred to Follow-Up Work

- **B2** — AC2 côté tour de revue (lit `SyncDebounce` au démarrage du tour).
- Confirmation AC5 sur un épisode réel après déploiement.
