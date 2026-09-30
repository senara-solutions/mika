# Une récurrente en vol au démarrage est ré-armée, jamais échouée (mika#2575)

- **Ticket** : `mika issue#2575`
- **Branche** : `fix/2575/loop-substrate-un-restart-pendant-le-tir`
- **Labels** : `bug`, `p2-normal`, `dispatch:loop`, `loop-substrate`
- **Périmètre** : `crates/mika-agent/src/task_engine/` (moteur), `crates/mika-agent/src/task_engine/cron.rs` (helper extrait). Aucune migration, aucune variable d'environnement, aucune valeur de réglage déplacée.

---

## Le défaut, mesuré (2026-09-28)

Le restart n°11 de `17:01:20Z` a tué le scan récurrent `wip_rescue`. La ligne
`ce90ad84` (`label=wip_rescue`, `trigger_type=recurring`) avait tiré à
`17:00:00Z` et était `in_progress` à l'arrêt ; elle est passée à `failed` à
`17:01:21Z`, **une seconde après** `mika-spirit starting`. Aucune ligne neuve.
Le tick de `17:05` est absent.

Le restart n°13 (`19:01:03Z`) **n'a pas réparé** — c'est la correction que
l'opérateur a apportée à sa propre lecture, et elle déplace le remède :

```
19:01:05.371Z WARN mika#1742: refusing to re-register recurring task …
  label=wip_rescue previous_task_id=ce90ad84… previous_status=failed
  previous_updated_at=2026-09-28T17:01:21Z grace_hours=24
```

Le scan est mort au moins jusqu'au premier restart postérieur au
2026-09-29 17:01Z.

**Contrôle positif du ticket, et il est décisif.** `worktree_reap` et
`qa_review_reconcile`, déjà `failed` *avant* le restart, ont reçu une ligne
`recurring_active` neuve. L'enregistrement sait donc remplacer un `failed`
**ancien** ; ce qu'il ne sait pas remplacer est un `failed` que le démarrage
vient lui-même d'écrire.

---

## Re-mesure de terrain (2026-09-30) — le défaut s'est résorbé par expiration, pas par correctif

Le groom moteur `8fc22e77` (2026-09-29) avait rendu READY en première passe puis
`ESCALATE — second-pass-after-ready` sur un défaut de **réponse** (seconde
réponse de l'architecte sans ligne d'ancre, `anchors_found=0`), pas de plan. Un
plan groomé a une demi-vie : avant de le re-soumettre, ses prémisses ont été
re-mesurées (lectures seules — base en `sqlite3 -readonly`, journal par `grep`
filtré sur `"level":"WARN"`, les corps LLM du groom précédent citant eux-mêmes le
WARN mika#1742 et polluant un grep nu).

**Le défaut n'a pas été corrigé ; il s'est résorbé par expiration de la grâce, et
il a coûté ~28 h de scan.**

| instant (UTC) | événement |
|---|---|
| 2026-09-28 17:00:00Z | tir `wip_rescue` (`ce90ad84`) |
| 17:01:20Z | restart n°11 (`91372f95`) — la ligne passe `failed` à 17:01:21Z |
| 17:31, 19:01, 02:00, 07:00, 07:30, 10:00 | six restarts, **six refus** mika#1742 sur `ce90ad84` |
| 2026-09-29 17:00:44Z | restart n°18 — **refusé à 37 s de la fin de grâce** (17:01:21Z) |
| 21:00:54Z | restart n°19 (`2975e3e1`) — ligne neuve `358c6e37` créée à **21:01:01Z** |
| 2026-09-30 04:05Z | `wip_rescue` tire normalement (`recurring_active`, `next_fire_at` futur) |

- **`wip_rescue` est redevenu `recurring_active` après le n°19, mais par une ligne
  NEUVE** (`358c6e37`), pas par une réparation de `ce90ad84` — qui reste `failed`,
  `updated_at 2026-09-28T17:01:21Z`. C'est la garde mika#1742 qui a cessé de
  refuser (fenêtre de 24 h écoulée), rien d'autre.
- **Zéro** ligne `wip_rescue: running auto-resume scan` entre 2026-09-28T17:01Z et
  2026-09-29T21:01Z : le scan a été mort ~28 h, et la durée n'est pas « 24 h » mais
  « 24 h, arrondies au restart suivant » — le n°18 l'a ratée de 37 s.
- **La sonde du ticket passe aujourd'hui** : les quatre labels (`wip_rescue`,
  `auto_pull_groomed`, `qa_review_reconcile`, `worktree_reap`) sont
  `recurring_active` pour `mika-dev`. Ce n'est **pas** une preuve de correctif :
  c'est l'état d'une base où aucun restart n'est tombé pendant un tir depuis le n°19.
- **Seconde occurrence, n=2, autre label et autre agent.** `heartbeat` de
  **mika-arch** (`2b71969e`) : tiré 2026-09-25T16:00:00Z, restart à 16:00:47Z,
  `failed` à 16:01:22Z (mika-arch est servi ~35 s après le démarrage par la boucle
  par agent de `run_server`, d'où le décalage), refusé par mika#1742 le 26/09 à
  09:02, 10:35, 11:01 et 14:31, ré-armé par une ligne neuve (`4a88fd04`) à
  16:31:03Z — **~24,5 h** sans heartbeat. `heartbeat` est l'un des sept
  `ensure_recurring_task` (`server/mod.rs:1767`), donc R2 le couvre sans ligne de
  plus : la classe « toute récurrente » du ticket est désormais **mesurée**, pas
  seulement inférée.

---

## Re-vérification du code (2026-09-30, re-groom moteur) contre `origin/main` @ `a923b352`

Le groom par spawn a rendu READY en première passe, mais depuis mika#2591 un
groom hors moteur ne frappe aucune preuve `PLAN_GROOMED` en base : le ticket
n'était pas dispatchable (cul-de-sac mika#2484 D4). Le plan est donc re-dérivé
par la boucle. **Un plan groomé a une demi-vie ; ses prémisses sont re-lues, pas
recopiées.** Chaque affirmation du § *La chaîne* et du § *Fire-Disposition* a été
re-vérifiée dans le code à `a923b352`.

**Trois ancres ont bougé, et la cause est nommée.** Le commit `a923b352`
(mika#1990, liveness de mika-manager) ajoute **5 lignes** à
`crates/mika-agent/src/server/mod.rs` — le seul fichier du périmètre qu'il
touche. Les ancres corrigées :

| ancre du plan (mesure 09-30 matin) | valeur à `a923b352` | objet |
|---|---|---|
| `server/mod.rs:1762-1905` | **`:1767-1910`** | les sept `ensure_recurring_task` |
| `server/mod.rs:1917` | **`:1922`** | l'appel à `startup_recovery` |
| « onze autres occurrences » du littéral dans `db/tasks.rs` | **dix** (12 au total : 1 `UPDATE` + 1 `INSERT` + 10 lectures) | compte du § *Fire-Disposition* |

**Ancres re-vérifiées exactes** (aucune correction) : `startup_recovery`
(`engine.rs:797`), `enqueue_queued_task` (`:4682`, calcul `:4696-4705`),
`fire_task` (`:4755`, calcul `:4786-4810`), le `warn!` *cannot reschedule
recurring task* (`:4810`), `make_task`/`trigger_type: "time"` (`:5505`/`:5512`),
`create_recurring_task_if_absent` (`db/tasks.rs:204`), l'`INSERT` posant le
littéral (`:292`), `update_task_rescheduled` (`:860`, `UPDATE` à `:862`), son
doc-comment (`:858`), `get_schedulable_tasks` (`:722`, prédicat `:725`),
`update_task_status` (`:737-739`), `mark_tasks_expired` (`:1643`), le chemin CLI
(`mika-cli/src/commands/chat.rs:261`), et le prédicat de
`idx_tasks_unique_recurring` (`db/migrations.rs:1403-1406`).

**Quatre prémisses re-mesurées vertes**, chacune portante pour un livrable :

1. **Les six maillons de la chaîne sont lus, pas déduits.** La garde cherche
   `status IN ('failed','cancelled','expired')` (`db/tasks.rs:232`) — `in_progress`
   lui est **invisible** ; le prédicat de l'index unique exclut
   `('cancelled','failed','expired','delivered')` — `in_progress` est donc
   **couvert**, d'où `n = 0` ; l'étape 2 n'épargne que `MANUAL`
   (`engine.rs:876-885`) ; l'étape 2a **est** gatée `!cli_mode` avec son
   raisonnement écrit sur place, la boucle générique ne l'est pas.
2. **`update_task_rescheduled` est le seul `UPDATE` qui pose le statut** dans
   tout `crates/` — vérifié par grep sur `UPDATE tasks SET` croisé au littéral :
   une seule ligne, `db/tasks.rs:862`. Le terme T1 du scan est vert.
3. **`mika tasks rearm` existe** — `task_engine::rearm_recurring_task`, appelé par
   `mika-cli/src/commands/tasks.rs:195`, audité sous
   `tool_name = 'recurring_operator_rearm'`. Le geste que le plan nomme pour les
   lignes déjà empoisonnées n'est pas une intention, il est livré.
4. **La limite « récurrentes portant un `timeout_at` » est exacte** :
   `ensure_recurring_task` pose `timeout_at: None` (`task_engine/mod.rs:88`), donc
   cette population est bien vide pour les sept récurrentes du démarrage.

**Une lacune réelle du prédicat de Fire-Disposition a été trouvée**, et elle est
de la classe même que le scan existe pour fermer : voir § *Fire-Disposition*,
terme T2. Les trois fonctions DB à statut **dynamique** (`db/tasks.rs:739`,
`:917`, `:1297`) permettent de poser `recurring_active` sans jamais écrire le
littéral dans un `UPDATE`.

---

## La chaîne, maillon par maillon (lue dans le code, pas déduite)

| # | site | ce qui se passe |
|---|---|---|
| 1 | `server/mod.rs:1767-1910` | les **sept** `ensure_recurring_task` tournent **avant** `startup_recovery` (l. 1922) |
| 2 | `db/tasks.rs::create_recurring_task_if_absent` | la garde anti-zombie cherche `status IN ('failed','cancelled','expired')` — la ligne est `in_progress`, **elle ne la voit pas**, aucun refus |
| 3 | idem | `INSERT OR IGNORE` entre en collision avec `idx_tasks_unique_recurring`, dont le prédicat est `status NOT IN ('cancelled','failed','expired','delivered')` — `in_progress` est **couvert** ⇒ `n = 0` ⇒ `Ok(None)` « already existed ». **Aucune ligne neuve** |
| 4 | `engine.rs::startup_recovery` étape 2 | seul `trigger_type = 'manual'` est épargné ; la récurrente est passée à `failed` |
| 5 | `engine.rs::startup_recovery` étape 3 | `get_schedulable_tasks` sélectionne `status IN ('pending','recurring_active')` — la ligne `failed` **n'entre pas dans le tas** |
| 6 | restart suivant, maillon 2 | la garde voit désormais `failed` avec `updated_at` dans la fenêtre ⇒ **refus pendant 24 h** |

Aucune décision fautive isolée : c'est un **ordre**, et le maillon 6 transforme
une panne d'un cycle en panne d'une journée.

---

## Ce que la lecture du code déplace, et c'est le premier livrable

**R-A — la voie « réordonner » du corps est insuffisante, et seule elle
aggrave.** Le corps propose « ou bien l'enregistrement a lieu **après** le
balayage ». Déplacer `startup_recovery` avant les `ensure_recurring_task` ferait
lire à la garde un `failed` vieux de quelques millisecondes : le maillon 6 se
déclencherait **dans le même démarrage** au lieu du suivant. Le réordonnancement
seul convertit une panne d'un cycle en panne immédiate de 24 h.

**R-B — les deux voies du commentaire supposent toutes deux un état terminal, et
c'est la supposition à retirer.** Le commentaire propose soit un statut distinct
(`interrupted_by_restart`), soit une garde excluant les échecs contemporains du
démarrage. Les deux acceptent que la ligne **doive** recevoir un état terminal.
Elle ne le doit pas : pour une récurrente, `in_progress` est un **état de tir
transitoire**, pas un état de registre.

`claim_and_fire_task` pose `in_progress` le temps du tir ; sur succès
`fire_task` appelle `update_task_rescheduled`, qui repose `recurring_active`.
Donc `in_progress` au démarrage ne dit pas « ce scan a échoué », il dit **« un
tir a été interrompu »**. Écrire `failed` confond l'échec d'un *tir* avec la mort
d'un *enregistrement* — et c'est cette confusion, et rien d'autre, qui arme
mika#1742.

**R-C — le coût de chaque voie refusée est concret.** `interrupted_by_restart`
est une valeur neuve de la contrainte `CHECK` sur `tasks.status`, que SQLite ne
sait pas altérer sur place : reconstruction de table sur `tasks`, qui est
référencée par `tasks.parent_task_id` et par `sessions.task_id`, plus la revue de
chaque requête énumérant les statuts (indices `idx_tasks_next_fire`,
`idx_tasks_schedulable`, `idx_tasks_unique_recurring`, `idx_tasks_manual_active`,
le tableau de bord, le CLI). Rayon de souffle sans commune mesure avec le défaut.
Une garde sur la coïncidence horodatée (`updated_at ≈ démarrage du process`)
échangerait, elle, un prédicat d'état contre un prédicat d'horloge — exactement
ce que la maison refuse ailleurs (mika#2277 : *« une fenêtre bornée sur un proxy
est une dette datée »*).

**R-D — l'horodatage stocké ne peut pas provoquer un tir immédiat, et il faut le
savoir avant de discuter du `next_fire_at`.** `enqueue_queued_task` **recalcule**
l'instant de tir depuis le cron pour toute ligne `recurring`, en ignorant le
`next_fire_at` de la base (`engine.rs:4699-4719`). Le champ stocké est donc, pour
une récurrente, une surface d'affichage et un critère de tri — pas un
déclencheur. On le recalcule quand même (le corps l'exige explicitement, et une
colonne qui ment est une mesure fausse), mais **aucun tir immédiat n'en dépend**.

---

## Le remède

`startup_recovery` étape 2 cesse d'écrire un état terminal sur une ligne
**récurrente** : elle la **ré-arme** — `next_fire_at` recalculé depuis le cron,
`status = 'recurring_active'` — par le même primitif que le tir nominal,
`Database::update_task_rescheduled`.

Trois propriétés en découlent, et aucune ne demande de mécanisme :

1. **Le ré-armement a lieu dans le même démarrage.** L'étape 2 précède
   l'étape 3, et `get_schedulable_tasks` sélectionne `recurring_active` : la
   ligne entre dans le tas au même démarrage. Aucun réordonnancement de
   `run_server`, aucune attente du scan périodique de 60 ticks.
2. **La garde anti-zombie n'est jamais armée pour cette classe.** Aucun état
   terminal n'est écrit, donc `create_recurring_task_if_absent` n'a rien à voir
   au restart suivant. mika#1742 n'est ni modifiée, ni exemptée, ni contournée :
   elle cesse simplement d'avoir une population fabriquée par le démarrage.
3. **Un seul écrivain de `recurring_active`.** `update_task_rescheduled` est
   aujourd'hui le **seul** `UPDATE` qui pose ce statut (l'autre site est
   l'`INSERT` de `create_recurring_task_if_absent`). Le ré-armement passe par
   lui : le ré-armement et le repos nominal deviennent **un seul acte textuel**,
   pas deux formulations qui peuvent diverger.

### Le repli, et il est déjà la règle maison

Si le cron est absent ou illisible, aucun instant futur n'est calculable : la
ligne retombe sur `failed`, comportement d'aujourd'hui, avec un motif nommé.
C'est très exactement ce que `fire_task` fait déjà dans le même cas
(`engine.rs:4810`, *« cannot reschedule recurring task, marking failed »*), donc
le repli n'introduit aucune sémantique neuve — et dans ce cas la garde de
mika#1742 s'arme **légitimement** : une récurrente dont le cron ne se calcule pas
ne doit pas se ré-inscrire toutes les minutes.

### Le mode CLI est la seconde porte du même défaut

`mika chat` exécute `startup_recovery` (`crates/mika-cli/src/commands/chat.rs:261`) avec
`cli_mode: true`, contre la base **partagée avec le démon**. L'étape 2a (balayage
A2A) est déjà gatée sur `!cli_mode`, avec son raisonnement écrit sur place :
*« there the "dead process" argument is false, and sweeping would fail live
rows »*. La boucle générique de l'étape 2, elle, **ne l'est pas** : lancer
`mika chat` pendant qu'une récurrente tire tue ce scan, exactement comme le
restart mesuré.

Le ré-armement **n'est donc pas appliqué en mode CLI** : la ligne est laissée
`in_progress`, intacte. Deux raisons, dans cet ordre : le CLI ne peut pas savoir
si le démon tire en ce moment, et il ne va de toute façon pas exécuter le scan
qu'il replanifierait. Si le démon est mort, son propre démarrage ré-armera.
C'est une amélioration stricte du comportement actuel (qui écrit `failed`), et
elle ferme la seconde porte sans toucher au reste de la boucle générique — les
lignes non récurrentes gardent en mode CLI le traitement d'aujourd'hui, mot pour
mot.

### Un lecteur unique pour le calcul de l'instant de tir

Le calcul `metadata → timezone → next_fire_from_cron{,_tz}` existe **deux fois**
(`fire_task` l. 4784-4818, `enqueue_queued_task` l. 4696-4719). Le ré-armement en
serait la troisième. `task_engine::cron::next_fire_for_recurring(cron_expr,
metadata, now) -> Result<String>` est extrait et les **trois** sites l'appellent ;
chacun garde sa disposition d'erreur (`fire_task` et le ré-armement marquent
`failed`, `enqueue_queued_task` renonce à empiler). Extraction pure, aucun
changement de comportement, couverte par les tests existants de `cron.rs` et du
moteur.

---

## Refus raisonnés

**Aucun compteur d'interruptions, et le refus est mesuré.** La question légitime
est : qu'est-ce qui borne une récurrente qui tuerait le process à chaque tir, si
l'on cesse d'écrire `failed` ? Trois mesures répondent, et une quatrième explique
pourquoi le compteur serait pire que le mal.

1. **Il n'existe pas de mécanisme par lequel un scan tue le process.**
   `fire_task` dispatche dans un `tokio::spawn` : une panique y est récoltée en
   `JoinError`, elle ne tue pas le processus. Les scans shellent via
   `tokio::process` : un sous-processus qui meurt ne tue pas le parent.
2. **La cause réelle d'un `in_progress` au démarrage est externe** — SIGTERM de
   déploiement, opérateur, OOM killer — et elle est mesurée : ce ticket documente
   les restarts n°11 et n°13 dans la même soirée.
3. **Un compteur à seuil bas se déclencherait sur le régime sain.** `wip_rescue`
   a un cron de 5 min et un tir pouvant durer ~900 s : sur un hôte qui redémarre
   plusieurs fois dans l'heure — le régime **documenté par ce ticket** — trois
   interruptions consécutives sans tir complet intercalé sont banales. Un budget
   de 3 tuerait le scan pour 24 h sur une journée de déploiement, c'est-à-dire
   **rouvrirait ce défaut sous un autre nom**.
4. Un compteur exigerait en outre un site de remise à zéro sur le tir abouti
   (motif `reset_stuck_rearm_count`, mika#2413, dont la leçon écrite est
   *« un compteur que le succès ne remet jamais à zéro finit par borner autre
   chose »*), donc un second écrivain sur le chemin de succès récurrent.

Ce qui remplace le compteur est une **mesure** : un événement de journal et une
ligne d'audit par ré-armement, à écrivain unique. Si un label se met à apparaître
plusieurs fois par jour, le suivi s'ouvre **avec un compte**, jamais avec une
intuition. La précondition du ticket de suivi est écrite dans § *Hors périmètre*.

*Ce que l'on retire en le disant :* le `failed` actuel se comporte comme un
disjoncteur — il fait taire le scan 24 h après une interruption. Mais il se
déclenche sur le cas **commun** (un restart) et pas sur le cas rare (un scan
pathologique, dont aucun mécanisme n'est identifié) : ce n'est pas un frein,
c'est un classificateur faux. Le retirer ne retire aucune protection réelle.

**Aucune réparation rétroactive des lignes déjà empoisonnées.** `ce90ad84` et ses
semblables restent `failed`. Réécrire après coup un état terminal rendrait faux
ce que la ligne a dit à l'instant où elle a été écrite (motif mika#2361), et le
geste existe déjà : `mika tasks rearm <label>` (mika#2446), qui pose le marqueur
`operator_rearm` que la garde exclut. Le plan **nomme** ce geste ; il ne le
remplace pas.

**Aucune modification de la garde mika#1742**, de `RECURRING_ZOMBIE_GRACE_HOURS`,
de l'exemption config-cancel (mika#2271), de l'exemption unknown-trigger
(mika#2337) ou du relèvement opérateur (mika#2446). Ce plan retire une population
que le démarrage fabriquait ; il ne touche pas au prédicat qui la lisait.

---

## Livrables

### R1 — `next_fire_for_recurring`, lecteur unique du calcul de tir

`crates/mika-agent/src/task_engine/cron.rs` :

```rust
pub fn next_fire_for_recurring(
    cron_expr: Option<&str>,
    metadata: Option<&str>,
    now: &str,
) -> Result<String>
```

`None` de `cron_expr` ⇒ `Err` nommant l'absence de cron. Timezone lue par
`extract_timezone_from_metadata` + `parse_timezone`, repli UTC — sémantique
identique aux deux sites existants. **Trois appelants** : `fire_task` (repos
nominal), `enqueue_queued_task` (mise en tas), le ré-armement de R2. Chacun garde
sa disposition d'erreur.

### R2 — le ré-armement dans `startup_recovery`

`crates/mika-agent/src/task_engine/engine.rs`, étape 2. La branche `manual`
(épargne) est inchangée. Une branche est ajoutée **avant** l'écriture `failed` :

| condition | action |
|---|---|
| `trigger_type != 'recurring'` | inchangé — `failed` |
| `trigger_type == 'recurring'` **et** `cli_mode` | **aucune écriture** ; ligne laissée `in_progress`, événement `recurring_restore_skipped_cli` (DEBUG) |
| `trigger_type == 'recurring'`, cron calculable | `update_task_rescheduled(id, next)` ⇒ `recurring_active` + `next_fire_at` futur ; événement + ligne d'audit |
| `trigger_type == 'recurring'`, cron absent/illisible | `failed` (comportement d'aujourd'hui) ; événement `recurring_restore_failed_no_cron` (WARN) |

Fail-safe : un échec de l'écriture de ré-armement est journalisé et **n'interrompt
pas** `startup_recovery` — la ligne reste `in_progress` et le démarrage suivant
réessaiera, jamais un démarrage avorté. L'écriture d'audit est *fire-and-forget*
(motif `qa_build_verdict_alert_audit_failed`) : perdre une ligne d'audit ne doit
pas pouvoir changer l'état d'une tâche.

### R3 — surfaces opérateur

| événement | niveau | audit `tool_name` | régime attendu |
|---|---|---|---|
| `recurring_restored_after_restart` | INFO | `recurring_restart_restore` | **non vide, faible** — une ligne par récurrente en vol au restart |
| `recurring_restore_failed_no_cron` | WARN | `recurring_restart_restore` (`after_value = failed_no_cron`) | **vide** |
| `recurring_restore_skipped_cli` | DEBUG | — | non instrumenté (population CLI, sans conduite associée) |

Champs de la ligne INFO : `task_id`, `label`, `agent_id`, `cron_expr`,
`previous_next_fire_at`, `next_fire_at`, `fired_at`.
Ligne d'audit : `session_id = system-<agent_id>`,
`target_key = recurring:<label>`, `before_value = in_progress`,
`after_value ∈ {recurring_active, failed_no_cron}`,
`reasoning = task_id:<id> cron:<expr> next_fire_at:<ts>`.

`after_value` est un **format de fil** : l'opérateur en fait des `GROUP BY`. Les
deux valeurs sont des constantes d'un seul site, épinglées par test.

---

## Surfaces opérateur

```bash
# 1. Quelles récurrentes ont été ré-armées, et lesquelles ont échoué ?
grep recurring_restored_after_restart "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{agent_id, label, cron_expr, previous_next_fire_at, next_fire_at}'

# 2. CONTRÔLE NÉGATIF — un cron illisible a-t-il fait retomber une ligne ?
grep recurring_restore_failed_no_cron "$MIKA_SPIRIT_LOG_FILE"

# 3. CONTRÔLE POSITIF — la garde de mika#1742 s'arme-t-elle encore sur ce chemin ?
grep 'mika#1742: refusing to re-register' "$MIKA_SPIRIT_LOG_FILE" | tail
```

```sql
-- La sonde du ticket, mot pour mot
SELECT label FROM tasks
 WHERE trigger_type = 'recurring' AND status = 'recurring_active'
   AND agent_id = 'mika-dev';
-- doit contenir wip_rescue, auto_pull_groomed, qa_review_reconcile, worktree_reap

-- Les deux populations du ré-armement, soustractibles en une requête
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'recurring_restart_restore' GROUP BY 1;
```

Les deux populations sont soustractibles parce que `recurring_restart_restore` a
un **écrivain unique** (scan de source, allowlist livrée vide) — motif
`ready_label_outcome` (mika#2323) : un seul nom, l'issue dans `after_value`,
plutôt que deux noms, parce que les deux issues appartiennent au même site et à
la même population.

---

## Sondes post-déploiement, et leurs cinq haltes

> **Préalable.** Ces mesures décrivent le **binaire servi**. Après `make deploy`,
> vérifier que `mika-spirit` qui tourne est bien celui qu'on vient de bâtir avant
> toute conclusion (classe mika#2340).

**S1 — le défaut fondateur ne se rejoue pas (premier restart en vol).**
Redémarrer `mika-spirit` pendant qu'une récurrente est `in_progress` (le plus
simple : un restart dans la fenêtre de 900 s d'un tir `wip_rescue`). Attendu :
une ligne `recurring_restored_after_restart` pour ce label, la requête SQL du
ticket porte les quatre labels, et le tick suivant du scan a lieu.
**Halte 1 — la ligne est absente et le scan est mort.** Ne pas toucher au
prédicat : vérifier d'abord que le binaire servi porte le correctif, puis que la
ligne était bien `in_progress` et non déjà `failed` d'un restart antérieur (une
ligne déjà empoisonnée relève de `mika tasks rearm`, pas de ce correctif).

**S2 — la garde de mika#1742 ne s'arme plus sur ce chemin (7 jours).** Aucun
`refusing to re-register` sur un label dont le dernier état était `in_progress` à
l'arrêt.
**Halte 2 — elle s'arme encore.** Lire `after_value` de la ligne d'audit : un
`failed_no_cron` explique le refus et il est **légitime** (le cron est cassé, et
c'est *lui* qu'il faut lire) ; en son absence, un autre site écrit un état
terminal sur ces lignes — l'établir **avant** d'élargir quoi que ce soit.

**S3 — contrôle négatif de bruit (7 jours).** Aucun
`recurring_restore_failed_no_cron`, et aucune récurrente ré-armée avec un
`next_fire_at` **passé**.
**Halte 3 — un `next_fire_at` passé apparaît.** Le calcul ne passe pas par le
lecteur unique de R1 ou la timezone n'est pas lue : réparer le calcul, ne pas
compenser à l'affichage.

**S4 — contrôle négatif de la population non récurrente (7 jours).** Une tâche
`time`, `event` ou `callback` interrompue au démarrage reste marquée `failed`.
**Halte 4 — une tâche à un coup ressort `recurring_active`.** La branche mord
trop large ; désarmer par revert **avant** diagnostic — un tir à un coup rejoué
est un effet de bord qu'aucun seuil ne corrige.

**S5 — le régime de ré-armement reste faible (30 jours).**
`SELECT count(*) … WHERE tool_name = 'recurring_restart_restore'` groupé par
label, rapporté au nombre de restarts.
**Halte 5 — un label domine largement les autres.** Ce n'est pas une panne de ce
correctif : c'est la mesure qui conditionne le suivi « borner les interruptions
répétées » nommé en § *Hors périmètre*. L'ouvrir **avec ce compte**, et surtout
ne pas rétablir le `failed` par réflexe — il ne fermerait rien et rouvrirait
mika#2575.

**Halte transverse — les sondes muettes.** Zéro ligne de ré-armement **et** zéro
refus de mika#1742 ne prouve rien tant qu'aucun restart n'a eu lieu pendant un
tir. Vérifier qu'un tel restart s'est produit avant toute conclusion : *une garde
que personne n'a exercée se lit exactement comme une garde qui marche*
(mika#2205).

---

## Fire-Disposition

Ce plan livre des détecteurs (tests comportementaux, un scan de source). Option
retenue : **(a) exception nommée en allowlist — allowlist livrée VIDE.**

- **Le scan `mika2575_le_statut_recurring_active_a_un_ecrivain_unique`** refuse
  un second écrivain de `recurring_active` en code de production hors de
  `Database::update_task_rescheduled`. **Son prédicat est une disjonction à deux
  termes, et le second est une correction que la re-mesure du re-groom a
  imposée** — le formuler sur T1 seul aurait produit un scan aveugle à la voie la
  plus probable.

  **T1 — l'écriture littérale.** Un `UPDATE … SET status = 'recurring_active'`
  hors de `update_task_rescheduled`. **Vérifié vert** : le croisement de
  `UPDATE tasks SET` et du littéral ne rend qu'**une** ligne dans tout `crates/`,
  `db/tasks.rs:862`. L'`INSERT` de `create_recurring_task_if_absent` pose le même
  littéral (`db/tasks.rs:292`, sous la forme `VALUES (…,'recurring_active',…)`) et
  tombe hors du terme, à dessein : créer l'enregistrement est l'autre acte
  légitime. Les dix autres occurrences du fichier sont des lectures
  `status IN (…)`.

  **T2 — l'écriture par statut dynamique, la voie que T1 ne voit pas.** Trois
  fonctions DB prennent le statut en **paramètre** (`db/tasks.rs:739`, `:917`,
  `:1297`, toutes de la forme `SET status = ?1`). Un futur écrivain peut donc
  poser le statut par `update_task_status(&id, task_status::RECURRING_ACTIVE)` —
  ou par son littéral — **sans qu'aucun `UPDATE … SET status = 'recurring_active'`
  n'apparaisse dans l'arbre**. C'est mot pour mot la classe que ce scan existe
  pour fermer : *un second écrivain ne rendrait aucune décision fausse le jour où
  il est écrit, il divergerait plus tard, en silence, tous les tests au vert.* T2
  refuse donc qu'un argument de statut valant `recurring_active` (littéral ou
  `task_status::RECURRING_ACTIVE`) soit passé à l'une de ces trois fonctions.

  **L'anti-vacuité de T2 ne peut PAS porter sur la présence du nom, et c'est le
  piège.** `task_status::RECURRING_ACTIVE` (`task_engine/types.rs:12`) n'est
  consommée **nulle part** aujourd'hui — zéro occurrence hors sa définition —
  donc T2 est vert par **population vide**, ce qui se lit exactement comme un
  arbre propre (mika#2205). Son contrôle porte donc sur la **forme du prédicat**,
  par une fixture négative **vue rouge** : un appel
  `update_task_status(&id, task_status::RECURRING_ACTIVE)` construit pour le test
  doit faire tirer le scan. T1, lui, garde l'anti-vacuité par le nom (il échoue
  si le littéral n'est écrit nulle part) — les deux termes n'ont pas le même
  contrôle parce qu'ils n'ont pas la même population.

  **Le prédicat dépouille les commentaires avant de scanner.** Le doc-comment de
  `update_task_rescheduled` (`db/tasks.rs:858`) écrit *« set next_fire_at and
  status = 'recurring_active' »* : un prédicat ancré sur `SET status =` ne le
  matche pas, mais un prédicat sur `status = 'recurring_active'` — la forme
  laxiste vers laquelle un futur éditeur glisserait pour « être sûr de ne rien
  rater » — compterait cette prose comme un second écrivain. C'est le faux
  positif que mika#2050 a mesuré sur le Signal S, et le dépouillement le ferme
  quelle que soit la précision du prédicat.

  L'allowlist `RECURRING_ACTIVE_WRITERS_ALLOWED` est livrée **vide** pour les deux
  termes, et un test frère (`…_allowlist_is_empty`) refuse qu'elle cesse de
  l'être — *une allowlist née vide est un tiroir où déposer la prochaine
  infraction* (mika#2323). **Quand il tire, on retire le second site — on ne
  l'allowliste pas** (doctrine mika#2201).
- **Le scan d'écrivain unique du nom d'audit** (`recurring_restart_restore`) :
  nom neuf, donc allowlist vide par construction, même contrôle anti-vacuité.
- **Les tests comportementaux** sont armés d'emblée : ils décrivent la
  transition neuve et n'ont aucune population préexistante à exempter.
- **Aucun test existant n'est désarmé.**
  `test_startup_recovery_marks_orphaned_in_progress_failed` reste vert sans
  modification : son `make_task` pose `trigger_type: "time"`
  (`engine.rs:5512`), il est donc **hors** de la population du ré-armement — il
  devient, sans être touché, le contrôle négatif de S4.

Aucun détecteur n'est livré désarmé, et aucune halte-et-remontée n'est requise.

---

## Verification Contract

### Tests comportementaux — `crates/mika-agent/src/task_engine/engine.rs` (`mod tests`)

| test | ce qu'il établit | vu rouge sans le correctif |
|---|---|---|
| `mika2575_une_recurrente_en_vol_ressort_recurring_active` | une ligne `recurring` + `in_progress` ressort `recurring_active` avec un `next_fire_at` **strictement futur** — le test littéral du corps du ticket | oui : ressort `failed` |
| `mika2575_la_ligne_rearmee_entre_dans_le_tas_au_meme_demarrage` | après `startup_recovery`, `queue_len` compte la ligne — le ré-armement a lieu **dans le même démarrage** | oui : la ligne `failed` n'est pas schedulable |
| `mika2575_la_garde_zombie_ne_sarme_pas_apres_un_rearmement` | après le ré-armement, `create_recurring_task_if_absent` sur le même label ne refuse pas et ne journalise pas de refus | oui : refus dans la fenêtre de grâce |
| `mika2575_une_tache_a_un_coup_reste_failed` | **contrôle négatif** — `trigger_type = 'time'` interrompue ressort `failed` | n/a (non-régression) |
| `mika2575_un_cron_illisible_retombe_sur_failed` | cron absent ⇒ `failed` + `after_value = failed_no_cron` | n/a (repli) |
| `mika2575_le_mode_cli_ne_rearme_pas` | `cli_mode: true` laisse la ligne `in_progress`, sans écriture | oui : passe à `failed` |
| `mika2575_le_rearmement_ecrit_sa_ligne_daudit` | une ligne `recurring_restart_restore` avec `before_value = in_progress` et `after_value = recurring_active` | oui |

### Tests structurels

| test | classe couverte |
|---|---|
| `mika2575_le_statut_recurring_active_a_un_ecrivain_unique` | **T1 + T2** — un second écrivain ne rendrait **aucune décision fausse** le jour où il est écrit ; il divergerait plus tard, en silence, tous les tests au vert |
| `…_le_prédicat_voit_lécriture_par_statut_dynamique` | **contrôle négatif de T2, à voir ROUGE** — une fixture passant `task_status::RECURRING_ACTIVE` à `update_task_status` doit faire tirer le scan. Sans lui, T2 est vert par population vide et se lit comme un arbre propre (mika#2205) |
| `…_le_prédicat_ne_compte_pas_un_doc_comment` | **contrôle de bonne foi, à voir VERT** — le doc-comment de `db/tasks.rs:858` ne doit pas compter comme écrivain ; sans lui, un prédicat laxiste rend le scan rouge en permanence, donc désarmé (classe mika#2050) |
| `…_allowlist_is_empty` | contrôle de bonne foi de l'allowlist |
| `mika2575_le_nom_daudit_a_un_seul_ecrivain` | rend le `GROUP BY after_value` exact plutôt qu'un nombre sur lequel deux sites peuvent diverger |
| `mika2575_les_valeurs_daudit_sont_un_format_de_fil` | fige `recurring_active` / `failed_no_cron` |

### Tests du helper extrait — `crates/mika-agent/src/task_engine/cron.rs`

`next_fire_for_recurring` : cron valide UTC, cron valide avec `metadata`
timezone, `cron_expr = None` ⇒ `Err`, cron illisible ⇒ `Err`, timezone illisible
⇒ repli UTC sans erreur.

### Non-régression

`cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`. Les suites `task_engine` et `db::tests::recurring_tasks`
doivent passer **sans modification** — les trois sites migrés vers R1 sont une
extraction pure.

### Ce qui n'est PAS testable ici, écrit plutôt que découvert

« Un restart réel pendant un tir réel de `wip_rescue` » n'est pas reproductible
en test : il demande un processus tué au milieu d'un `tokio::spawn` contre une
base partagée. Le contrat **côté moteur** est que `startup_recovery` appliquée à
une ligne `recurring` + `in_progress` rend `recurring_active` et l'empile — et
les tests ci-dessus l'établissent de façon déterministe. La moitié
comportementale en service est la sonde S1.

---

## Definition of Done

- [ ] `next_fire_for_recurring` extrait dans `cron.rs`, **trois** appelants migrés.
- [ ] `startup_recovery` ré-arme une ligne `recurring` + `in_progress` par
      `update_task_rescheduled`, dans le même démarrage.
- [ ] Repli `failed` nommé quand le cron n'est pas calculable.
- [ ] Mode CLI : aucune écriture sur une ligne récurrente.
- [ ] Événement INFO + ligne d'audit à écrivain unique, valeurs figées.
- [ ] Le scan d'écrivain unique porte ses **deux** termes (T1 littéral, T2 statut
      dynamique), avec le contrôle négatif de T2 **vu rouge** et le contrôle de
      bonne foi du doc-comment **vu vert**.
- [ ] Les sept tests comportementaux et les six tests structurels passent ; les
      quatre « vus rouges » l'ont été avant le correctif.
- [ ] `cargo test -p mika-agent`, `clippy -D warnings`, `fmt --check` verts.
- [ ] `crates/mika-agent/CLAUDE.md` § *Unified Task Engine* documente la
      transition et son raisonnement ; le `CLAUDE.md` racine porte les surfaces
      opérateur, les sondes et leurs haltes.
- [ ] Corps de PR : les rectifications R-A à R-D, la mesure qui refuse le
      compteur, le geste `mika tasks rearm` pour les lignes déjà empoisonnées, et
      la voie T2 — un scan formulé sur T1 seul aurait été aveugle à l'écriture
      par statut dynamique.

## Acceptance criteria

*Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés du § « Correctif attendu », du § « Sonde » et de
la correction apportée en commentaire.*

- **AC1** — Une récurrente `in_progress` au démarrage ressort `recurring_active`
  avec un `next_fire_at` **futur** (test littéral du corps).
- **AC2** — Le ré-armement a lieu **dans le même démarrage** : la ligne est
  schedulable et empilée à l'issue de `startup_recovery`, sans attendre le scan
  périodique ni un redémarrage.
- **AC3** — Le démarrage n'écrit plus, pour cette classe, d'état que la garde
  mika#1742 compte comme échec terminal ; la garde n'est ni modifiée ni
  contournée.
- **AC4** — Après un restart, `SELECT label FROM tasks WHERE
  trigger_type='recurring' AND status='recurring_active' AND
  agent_id='mika-dev'` contient `wip_rescue`, `auto_pull_groomed`,
  `qa_review_reconcile` et `worktree_reap`.
- **AC5** — Les tâches non récurrentes interrompues gardent le comportement
  actuel (`failed`), contrôle négatif inclus dans la suite.
- **AC6** — Un cron absent ou illisible retombe sur `failed`, avec un motif
  nommé et lisible.
- **AC7** — Chaque ré-armement est observable : un événement de journal et une
  ligne d'audit à écrivain unique, dont le régime attendu est écrit.

---

## Hors périmètre, délibérément

- **La garde mika#1742 et ses quatre exemptions** (config-cancel mika#2271,
  unknown-trigger mika#2337, relèvement opérateur mika#2446,
  `RECURRING_ZOMBIE_GRACE_HOURS`) : inchangées. Ce plan retire une population
  fabriquée par le démarrage, il ne touche pas au prédicat qui la lisait.
- **La réparation des lignes déjà `failed`** (dont `ce90ad84`) :
  `mika tasks rearm <label>` est le geste, et il existe.
- **Le réordonnancement de `run_server`** : refusé par R-A, et devenu inutile.
- **Borner les interruptions répétées d'un même label.** Refusé ici sur mesure
  (§ *Refus raisonnés*). **Précondition du suivi :** que la sonde S5 montre un
  label ré-armé plusieurs fois par jour sur plusieurs jours, avec son compte.
- **Le gating CLI de la boucle générique de l'étape 2 pour les tâches NON
  récurrentes.** `mika chat` peut encore marquer `failed` une tâche `time` /
  `event` / `callback` vivante du démon. Population distincte, rayon de souffle
  distinct, et le remède demande de décider ce qu'un CLI a le droit de balayer
  dans la base d'un démon vivant. **Ticket de suivi**, précondition : une mesure
  montrant qu'une tâche vivante a été fauchée par une invocation CLI.
- **Les récurrentes portant un `timeout_at`.** L'étape 1 (`mark_tasks_expired`)
  précède le balayage et écrit `expired`, que la garde compte aussi. Les sept
  récurrentes posées par `ensure_recurring_task` ont `timeout_at: None`, donc
  cette population est vide pour elles ; une récurrente créée par l'outil de
  planification avec un `timeout_at` échapperait à ce correctif. Limite
  **nommée**, non couverte ; sa sonde est l'absence attendue de
  `status = 'expired'` sur des lignes `recurring` dans la requête SQL ci-dessus.
- **La cause des redémarrages eux-mêmes** : ce travail rend la récurrente
  survivante, il ne rend pas le processus stable.

## Ce que ce travail n'achète PAS

Aucun scan n'est rendu plus fiable : ce qui change est qu'un redémarrage cesse de
le tuer. Aucun compteur, aucune ligne d'audit existante n'est réécrite, et les
deux occurrences mesurées du 2026-09-28 **ne sont pas rattrapées** — fabriquer
une ligne décrivant un ré-armement qui n'a pas eu lieu serait l'inverse de ce que
ce travail défend. La sonde est la **prochaine** occurrence, et son silence ne
prouve rien tant qu'aucun restart n'est tombé pendant un tir.
