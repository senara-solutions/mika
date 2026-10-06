---
title: "fix(qa-review): une revue par tête stable — phase A, le callback de build d'une tête remplacée ne paie plus de tour LLM"
issue: senara-solutions/mika#2671
type: fix
status: active
date: 2026-10-06
phase: A (sur B)
---

# fix(qa-review) mika#2671 — phase A : pas de tour LLM pour un build périmé

## Pourquoi

**Mesure (mika.db, `llm_calls` + `tasks`, 2026-10-06).** PR mika#2659, trois
`pull_request.synchronize` :

| heure | événement | coût du tour |
|---|---|---|
| 14:21:21 | sync 1 → revue (session `7860b7a0`) | 11 appels, 0,58 M |
| 14:22:20 | build 1 lancé (tâche `long_running:build_mika`) | — |
| 14:22:30 | sync 2 → revue (`97aac38d`) | 13 appels, 0,71 M |
| 14:24:03 | build 2 lancé (annulé à 14:32:20, aucun tour) | — |
| 14:28:49 | **callback du build 1** (`callback-1d1b6aa5`) | 21 appels, **1,03 M** |
| 14:32:12 | sync 3 → revue (`1f74732a`) | 20 appels, 1,06 M |
| 14:32:29 | build 3 lancé | — |
| 14:39:51 | callback du build 3 (`callback-a42638fd`) | 21 appels, 1,16 M |

Total ≈ 4,5 M de tokens d'entrée. Le callback de 14:28:49 revoit une tête que
GitHub avait déjà remplacée **huit minutes plus tôt** (sync 2, 14:22:30, dix
secondes après le lancement du build 1). Ce tour est le plus cher de l'épisode
après le dernier, et il ne pouvait rien produire d'utile : son verdict porte sur
une tête morte.

Les gardes existantes ne le voient pas (corps de mika#2671) : la coalescence de
`webhook_queue_v2` ne fusionne que des événements **encore en file** ; la
supersession de mika#2335 annule un **sous-processus**, pas les tours déjà payés
ni le callback d'un build qui a fini.

## Découpage (contrainte du vol : estimation ×2,5, phases au-delà de 1000 lignes)

Estimation brute du ticket entier ≈ 1 400 lignes (code + doc-comments + tests),
×2,5 ≈ 3 500. Deux phases, découpées sur la frontière **dispatcher / file** :

- **Phase A (ce plan)** — côté dispatcher : AC3, et la moitié « callback de
  build » d'AC2. Brut ≈ 400, ×2,5 ≈ 1 000.
- **Phase B (plan séparé)** — côté file webhook : AC1 (anti-rebond), AC4 (aucune
  revue perdue, `opened`/`ready_for_review`/`review_requested` non retardés), la
  moitié « tour de revue » d'AC2, et la durabilité de l'événement retenu pendant
  la fenêtre (un redémarrage pendant les ~5 min d'attente ne doit pas perdre la
  revue de la dernière tête — c'est un risque que la fenêtre **crée**, donc il
  part avec elle).

La phase A ne ferme pas le ticket : la PR porte `Part of #2671`.

## Décision centrale : « périmé » = un `synchronize` plus récent a été **reçu**

Deux définitions possibles de « la tête visée n'est plus la tête de la PR » :

1. **Vérité GitHub** — au moment du callback, lire `headRefOid` et comparer au
   SHA construit.
2. **Événement observé** — un `pull_request.synchronize` pour la même PR a été
   reçu par mika-qa **après** le lancement du build.

**On prend (2).** Raison décisive : (1) perd des revues. Le gateway supprime les
`synchronize` sans changement de fichiers (garde no-diff, #886 — amend de
trailer). Avec (1), une tête déplacée par un amend de trailer rendrait le build
« périmé », le callback serait sauté, **et aucun autre événement ne
déclencherait la revue de la nouvelle tête** : la PR resterait sans verdict.
Avec (2), on ne saute un tour **que si l'événement qui déclenchera la revue de
la tête suivante a déjà été reçu**. AC4 (« la dernière tête est toujours
revue ») tient alors par construction, pas par vigilance. (2) ne fait en outre
aucun appel GitHub — ni en production, ni à bouchonner en test.

Corollaire assumé : un `synchronize` supprimé par la garde no-diff n'est pas
« observé » ; le callback tourne alors sur l'ancienne tête, dont le contenu est
identique. C'est le bon résultat.

**Fail-safe dans le sens de la revue** (borne du ticket) : registre illisible,
cible PR illisible, métadonnée absente, garde désarmée → le tour tourne, comme
aujourd'hui.

## Mécanisme

### 1. Le registre : une ligne d'audit par tour de revue démarré sur un `synchronize`

Dans `server::handlers::run_agent_for_message` — point de passage des trois
chemins qui lancent un tour (drain v2, chemin hérité, rejeu #528) — si le canal
est `github` et que `classify_event(text)` rend `PullRequestSync { repo, pr }` : `log_audit_event(tool_name =
"qa_pr_sync_observed", target_key = "pr:{repo}#{pr}")`. **Révisé en revue de code** : la
première version écrivait à la réception, dans `handle_message` ; trois
relecteurs ont montré qu'un `synchronize` évincé par la file bornée
(drop-oldest après un 202) ou perdu au redémarrage aurait alors fait sauter le
callback sans revue de la nouvelle tête. Écrit au démarrage du tour, le
registre atteste que la revue suivante a commencé. Best-effort : un échec
d'écriture est journalisé et n'arrête pas le tour (fail-safe : pas de ligne
⇒ pas de supersession ⇒ le tour tourne).

Pas de nouvelle table, pas de migration : la lecture passe par
`count_recent_audit_events_for_target` (déjà utilisée par
`qa_review_reconcile` et le disjoncteur de `verdict_handler`). Le registre est
**par agent** (`agent_id` dans la requête) : seuls les tours de mika-qa le
lisent, et les `synchronize` ne sont routés qu'à mika-qa — un build de mika-dev
ne peut donc jamais être déclaré périmé.

### 2. La garde : avant le tour de callback de build

Dans `TaskDispatcher::dispatch_resume_agent`, après la classification du
chemin et **avant** l'acquisition du verrou d'agent (un tour qu'on ne lance pas
n'a pas à attendre le verrou, ni à compter un report `AgentBusy`) :

- terme 1 : `is_callback` et `is_build_callback_label(&task.label)` ;
- terme 2 : garde armée (`MIKA_QA_STALE_BUILD_GUARD`, armée par défaut, `0` /
  `false` / `off` / `no` désarme, valeur inconnue ⇒ WARN + armée — même forme
  que `MIKA_QA_CALLBACK_VERDICT_NET`) ;
- terme 3 : `read_qa_review_pr_target(task.metadata)` lisible ;
- terme 4 : `count_recent_audit_events_for_target("qa_pr_sync_observed",
  "pr:{repo}#{pr}", task.created_at) > 0` — comparaison stricte, résolution à la
  seconde des deux côtés (`strftime('%Y-%m-%dT%H:%M:%SZ')`) ; une égalité à la
  seconde se lit « non périmé » (fail-safe).

Les quatre termes vrais ⇒ **zéro appel LLM** :

- `mark_task_delivered(task.id)` — la ligne sort de la population des callbacks
  non livrés (`alert_undelivered_build_verdicts` ne la signalera pas, le scan ne
  la re-sélectionnera pas) ;
- ligne d'observabilité nommée : `warn!`/`info!` `event =
  "qa_build_callback_superseded"` + ligne d'audit du même nom, `target_key =
  task:{id}`, `after_value = pr:{repo}#{pr} later_syncs={n}` ;
- retour `Ok(())`, sans session, sans filet de verdict (rien n'était dû sur une
  tête morte).

Toute autre combinaison ⇒ chemin actuel inchangé. Un registre illisible
(`Err`) écrit un `warn!` `qa_build_callback_supersession_unreadable` et tombe
dans le chemin actuel.

On n'appelle **pas** `dispatch_next_deferred_callback` : son commentaire
justifie l'appel par « the blocking dispatch just completed, so the slot is
free », ce qui est faux pour un build ; le backstop périodique
(`promote_pending_deferred_if_idle`) reste le chemin de promotion.

### 3. Fonction pure de décision

Module `crate::qa_head_supersession` : la décision est une fonction pure
`decide_build_callback(label, metadata, later_syncs: Result<i64, _>) ->
BuildCallbackHead` (`NotABuildCallback` / `NoPrTarget(reason)` / `Current` /
`Superseded { target, later_syncs }` / `LedgerUnreadable`), testée sans base,
sans réseau. Le module porte aussi la constante du registre et la clé
`pr:{repo}#{pr}`, écrites une fois (leçon mika#2158 : une grammaire recopiée
entre deux lecteurs dérive).

## Ce que la phase A ne fait PAS

- Pas d'anti-rebond (AC1) ni de contrôle au début du tour de **revue** : phase B.
- Ne touche pas `verdict_handler.rs` ni aucun chemin CODEOWNERS
  (`perimeter/`, `pr_merge_with_gate.rs`, `docs/gate/`, `docs/egress/`).
- Ne change aucun prompt (le prompt qa-review est à son plafond de taille,
  `skills/bundled/qa-review/skill.toml`).
- Aucun appel GitHub ajouté.

## Fichiers

- `crates/mika-agent/src/qa_head_supersession.rs` (nouveau) — décision pure +
  constantes + tests unitaires.
- `crates/mika-agent/src/lib.rs` — `pub mod`.
- `crates/mika-agent/src/server/handlers.rs` — écriture du registre.
- `crates/mika-agent/src/task_engine/dispatcher.rs` — garde + tests
  d'intégration sur `test_dispatcher` (base SQLite réelle en mémoire, LLM
  bouchon qui **compte ses appels**).

## Definition of Done

- [ ] `cargo build`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt
      --check`, tests du crate `mika-agent` verts.
- [ ] Chaque AC de la phase porte un test positif **et** un contrôle négatif ;
      chaque contrôle négatif est vérifié rouge par mutation de la garde.
- [ ] Aucun appel GitHub réel depuis les tests.
- [ ] Sonde AC5 documentée dans le corps de PR.
- [ ] `/ce:compound` traversé.

## Acceptance criteria

Transcrits du ticket (mika#2671). Les cases de la phase B restent ouvertes et
sont nommées comme telles.

- [ ] **AC1. Anti-rebond.** Un `synchronize` ouvre une fenêtre d'attente (~5 min, réglable, trois paliers maison). Seule la **dernière** tête reçue dans la fenêtre déclenche un tour de revue. Tests : trois synchronize dans la fenêtre ⇒ un seul tour, sur la dernière tête ; contrôle négatif : deux synchronize séparés de plus que la fenêtre ⇒ deux tours. — *phase B*
- [ ] **AC2. Tête encore courante.** Avant chaque tour de revue **et** chaque tour de callback de build, le moteur vérifie que la tête visée est toujours la tête de la PR. Sinon le tour se termine sans appel LLM complet (au plus un appel de constat, ou zéro) et laisse une ligne d'observabilité nommée. Tests : tête périmée ⇒ aucun tour LLM complet ; contrôle négatif : tête courante ⇒ comportement inchangé. — *phase A : callback de build ; phase B : tour de revue*
- [ ] **AC3. Build périmé.** Le callback de build d'une tête remplacée ne relance pas de tour LLM complet, qu'il ait été annulé (mika#2335) ou non. Test avec un callback sur une tête périmée. — *phase A*
- [ ] **AC4. Aucune revue perdue.** La dernière tête est toujours revue, et un `opened` / `ready_for_review` / `review_requested` n'est pas retardé par la fenêtre. Contrôle négatif dédié. — *phase B (la phase A la garantit pour sa part par la décision « événement observé »)*
- [ ] **AC5. Mesure.** Sur un épisode équivalent (3 synchronize en ~12 min), la cible est **≤ 1,5 M** de tokens d'entrée contre ~4,5 M aujourd'hui. Sonde post-déploiement documentée (requête `llm_calls` par session). — *sonde livrée en A, mesure finale après B*

### Tests de la phase A

| AC | test positif | contrôle négatif |
|---|---|---|
| AC3 / AC2-callback | callback `long_running:build_mika` avec cible PR, une ligne `qa_pr_sync_observed` postérieure à `created_at` ⇒ 0 appel LLM, tâche `delivered`, ligne d'audit `qa_build_callback_superseded` | même callback, ligne d'observation **antérieure** (ou d'une autre PR) ⇒ le tour tourne (≥ 1 appel LLM) |
| AC3, « annulé ou non » | la décision ne lit pas le statut : `completed` et `failed` sont tous deux sautés | — |
| fail-safe | métadonnée sans cible ⇒ le tour tourne ; garde désarmée ⇒ le tour tourne | idem |
| portée | label `long_running:deploy_mika` + observation postérieure ⇒ le tour tourne | — |
| registre | `handle_message` d'un `PR synchronize` écrit une ligne `qa_pr_sync_observed` `pr:{repo}#{pr}` | un `PR opened` n'en écrit aucune |

## Sonde AC5 (post-déploiement)

```sql
-- coût par session mika-qa sur une PR, fenêtre d'un épisode
SELECT session_id, MIN(created_at), COUNT(*), SUM(input_tokens)
FROM llm_calls WHERE agent_id = 'mika-qa' AND created_at BETWEEN :t0 AND :t1
GROUP BY session_id ORDER BY 2;
-- les callbacks sautés par la phase A
SELECT created_at, target_key, after_value FROM audit_events
WHERE agent_id = 'mika-qa' AND tool_name = 'qa_build_callback_superseded'
ORDER BY created_at DESC LIMIT 20;
```

Rejouée sur l'épisode du 2026-10-06, la phase A seule aurait sauté le callback
de 14:28:49 (build 1 créé 14:22:20, sync 2 observée 14:22:30) : 4,5 M → ≈ 3,5 M.

**La cible ≤ 1,5 M est sous le plancher d'un cycle unique mesuré.** Le cycle le
moins cher de l'épisode (revue 0,58 M + callback 1,03 M) coûte déjà 1,6 M ; le
cycle de la dernière tête, 2,2 M. Même un anti-rebond parfait (phase B) qui ne
garderait qu'un cycle ne passe pas 1,5 M sans réduire aussi le coût par cycle —
ce qu'aucun AC de ce ticket ne touche. C'est dit ici pour que la mesure finale
ne soit pas lue comme un échec de l'anti-rebond.
