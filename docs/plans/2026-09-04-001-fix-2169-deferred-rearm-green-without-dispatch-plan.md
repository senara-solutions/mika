# Plan : le re-tir différé se termine vert sans dispatcher (mika#2169)

**Ticket :** mika issue#2169 — `fix(task_engine): le re-tir différé se termine vert sans dispatcher — 80 min de boucle muette avec la fente libre depuis 4 secondes`
**Labels :** `bug`, `p1-important`
**Type :** issue (bug — moteur, classe « casse la boucle »)
**Palier de priorité :** Tier 1 — *casse la boucle*. La chaîne `ready-label` refusée sur fente occupée n'a aucun chemin de reprise : la tâche reste stérile jusqu'à ce qu'un balayage la fauche. Mesuré : 80 minutes de silence total sur le correctif de la porte de promotion.

---

## Ce que la base dit, relu avant de planifier

Le corps du ticket décrit la trace. La lecture de la base (`~/.mika/data/mika.db`, 2026-09-04) la complète — et sur un point la corrige — en trois faits qui décident de la forme du correctif.

### Fait 1 — le tour a bien eu lieu, trois fois, et chaque wrapper porte `delivered`

> **Correction du 2026-09-04 10:50 UTC.** Une première rédaction de ce plan (02:41 UTC) concluait que le wrapper `f0cd5967` avait été « promu puis abandonné dans la file de livraison », sur la foi de `status='completed'` et d'un `fired_at` vide. **C'était vrai à l'instant de la mesure et faux cinquante minutes plus tard.** La re-mesure faite avant grooming corrige le fait et, avec lui, la forme du correctif. Le paragraphe qui suit est la version mesurée ; l'ancienne lecture est conservée nulle part ailleurs pour éviter qu'on planifie contre elle.

La chaîne complète des enfants de `620ae345`, relue en base :

| wrapper | créé | statut final | `result` | effet |
|---|---|---|---|---|
| `f0cd5967` | `00:42:12Z` | **`delivered`** (`03:35:31Z`) | `deferred dispatch slot freed` | aucun dispatch |
| `9943f191` | `03:35:31Z` | **`delivered`** (`03:36:03Z`) | `deferred dispatch slot freed` | aucun dispatch |
| `4e025a63` | `03:59:50Z` | **`delivered`** (`04:00:18Z`) | `deferred dispatch slot freed` | aucun dispatch |

Trois enseignements, chacun portant.

**1. `completed` était un état de transit, pas un état terminal.** `f0cd5967` a été promu à `00:48:11Z` et livré à `03:35:31Z` — **deux heures quarante-sept** plus tard, la session `deferred-dispatch-c99ec7ea-d9b1-42c3-9ef7-91827548985a` (`03:35:03Z`) atteste le tour. L'attente vient du verrou d'agent : `dispatch_resume_agent` fait `try_lock` et rend `DispatchError::AgentBusy` sans bruit quand il est pris (`task_engine/dispatcher.rs:390-400`), et l'appelant supprime explicitement l'avertissement pour ce cas (`engine.rs:585-587`). La famine est réelle et longue ; elle n'est pas définitive. Les vingt-deux wrappers que la première mesure disait « jamais livrés » portent **tous** `delivered` aujourd'hui, horodatés entre `03:33Z` et `04:06Z`.

**2. L'échelle de re-armement existante a fonctionné.** Chaque livraison stérile a été détectée : `rearm_consumed_deferred_wrapper` (`dispatcher.rs:1256`) a tiré à `03:35:31Z` puis à `03:59:50Z`, créant les deux wrappers de remplacement à l'horodatage exact de la livraison précédente. Le parent porte `metadata.stuck_rearm_count = 2` — le budget `MAX_STUCK_REARMS` a été consommé jusqu'au bout. **Le mécanisme de détection n'est pas ce qui manque.**

**3. Ce qui manque est l'enregistrement, et la sortie du budget.** Les trois wrappers finissent `delivered` — le mot le plus affirmatif du vocabulaire — avec pour `result` `deferred dispatch slot freed`, qui décrit la promotion et non l'effet. Aucun des trois n'a dispatché : aucune tâche `long_running:run_claude_pilot` n'existe entre `00:42Z` et `06:40:07Z`. Et quand le budget s'est épuisé, **rien n'a écrit d'échec portant une raison** : `620ae345` est resté `blocked` jusqu'à ce que le balayage phantom le fauche à `03:59:50Z` avec `result='phantom_aged_out'` — un mot qui nomme le faucheur, pas la cause.

C'est exactement le défaut que le ticket décrit, mesuré une station plus loin que la première rédaction ne le croyait : non pas « le re-tir n'a jamais tiré », mais **« le re-tir a tiré trois fois, n'a rien dispatché trois fois, et a écrit `delivered` trois fois »**.

> **Conséquence sur AC1.** L'échec visible portant une raison qu'AC1 exige comme seconde branche **n'existe sur aucun chemin aujourd'hui**. L'épuisement du budget côté dispatcher (`RearmOutcome::Unrepairable`) ne fait pas passer le parent en `failed` avec motif ; il le laisse `blocked`. C'est le trou que L3a doit boucher, et il est plus large que ce que la première rédaction supposait.

### Fait 2 — le parent était `blocked`, et l'échelle de réparation ne regarde que `pending`

mika#2045 a posé une échelle de réparation (`reap_orphaned_pending_issue_tasks`, `engine.rs:612`) alimentée par `Database::find_orphaned_pending_issue_tasks` (`db.rs:7484`). Sa toute première clause :

```sql
AND parent.status = 'pending'
```

`620ae345` était `blocked`. Elle n'a donc **jamais** été candidate.

La preuve est dans les voisins. Cinq parents `ready-label` de la même nuit, même forme (`source='self_dev'`, `trigger_type='manual'`, `type='issue'`), refusés sur la même fente, portent `metadata.stuck_rearm_count = 2` et trois wrappers chacun — l'échelle les a réparés deux fois puis expirés. `620ae345` porte **zéro** re-armement et **un** wrapper :

| parent | statut au moment du balayage | `stuck_rearm_count` | wrappers |
|---|---|---|---|
| `14465667` (mika#2156) | `pending` | 2 | 3 |
| `1ce30da0` (mika#2158) | `pending` | 2 | 3 |
| `298d15e1` (mika#2157) | `pending` | 2 | 3 |
| `477ef611` (mika#2143) | `pending` | 2 | 3 |
| **`620ae345` (mika#2140)** | **`blocked`** | **absent** | **1** |

Le mot `blocked` n'est écrit par aucune ligne du moteur sur ce chemin : il vient du LLM via `update_task_status`, prescrit par les prompts d'escalade (`skills/bundled/self-dev-webhook-ready-label/system_prompt.md:41`, `self-dev-webhook-qa/system_prompt.md:52,72`, `self-dev-callback/system_prompt.md:102`). **La couverture de la réparation dépend donc du mot qu'un LLM a choisi.** C'est la classe `feedback_prompt_enforcement_fragile` exactement, un étage sous celui que mika#2045 a fermé.

### Fait 3 — le wrapper qui tire dans un parent terminal, signalé en commentaire

Le commentaire de `mika-platform-dev` sur le ticket (`2026-09-04T03:41:22Z`, **postérieur** à la première rédaction de ce plan) mesure une variante que la trace de `620ae345` ne montre pas :

- parent `14465667` (`ready-label: mika#2156`) auto-échoué à `00:54:11Z` par le balayage stuck-pending, `result='stuck_pending_no_deferred_wrapper'` ;
- le wrapper tire malgré tout et ré-invoque `run_claude_pilot` avec la configuration d'origine ;
- `run_claude_pilot` refuse : `Task is not an active task. It must be a manual task with status pending, in_progress, or blocked.` ;
- `failed` est terminal — `Cannot transition from 'failed' to 'in_progress'` : le re-armement lui-même est structurellement impossible.

Le tour se termine sans dispatch et sans bruit. C'est la même vacuité verte, mais sur une population que L1 seule ne répare pas : marquer le wrapper `expired` est juste, **et le re-armement qui suit ne peut pas aboutir**. Il faut que ce cas sorte par une branche visible plutôt que par un re-armement voué à l'échec — c'est le sens de L2 révisé ci-dessous.

### Ce que ces trois faits imposent

- **AC2 se corrige bien à la fin du tour** — contrairement à ce que la première rédaction déduisait. Le tour a lieu ; ce qu'il écrit (`delivered` + `deferred dispatch slot freed`) est ce qui ment. L1 est donc le livrable central, et son cas de mesure est la trace des trois wrappers ci-dessus.
- **Le chien de garde de la promotion « jamais tirée » perd son cas mesuré, et devient dangereux tel qu'il était spécifié.** Aucune promotion n'est restée non tirée : la plus lente a mis 2 h 47. Un chien de garde à 300 s aurait marqué `expired` les vingt-deux wrappers vers `00:53Z`, re-armé, épuisé le budget en une dizaine de minutes, et **échoué définitivement des parents qui allaient être servis à `03:35Z`**. La fenêtre seule ne peut pas séparer la famine du vrai orphelin : il faut un discriminant d'état, pas un délai. L2 est re-spécifié sur cette base.
- **AC1/AC3 ne se corrigent pas en élargissant `find_orphaned_pending_issue_tasks` à `status IN ('pending','blocked')`** : `blocked` est aussi le mot des portes délibérées (auto-merge refusé, `server/verdict_handler.rs:999/1289/1496/2160`, `server/ci_success_handler.rs:848`). Élargir rearmerait un dispatch contre une porte opérateur. Il faut un balayage **discriminant**, dont la population est exactement le refus de fente.
- **AC1 exige en plus une sortie de budget visible.** Mesuré : elle n'existe nulle part. L3a la porte.

---

## Portée

### Dans la portée

- **L1** — un enregistrement terminal qui porte l'**effet** pour chaque fin de vie d'un wrapper différé (AC2).
- **L2a** — refuser le re-armement vers un parent terminal, et le dire (AC2, cas du commentaire).
- **L2b** — rendre la famine de promotion **mesurable**, sans action destructrice (AC2, observabilité).
- **L3a** — consommer le `RearmOutcome` jeté, et écrire l'échec de budget avec son motif (AC1, AC3).
- **L3b** — un balayage du trio « bloqueur terminé + bail expiré + parent bloqué », en filet (AC1, AC3).
- **L4** — un test de rejeu de la trace du 2026-09-04, rouge sans le correctif, avec cas accentué (AC4).
- **L5** — deux tests de non-régression sur le bloqueur réellement actif (AC5).

### Hors portée (repris du corps du ticket)

- La sérialisation `implement` à 1 → mika#2160, sous garde opérateur.
- Le refus `bash-grep` qui a tué le pilote initial → claude-pilot#151 volet (A).
- Le balayage phantom qui a fauché `620ae345` à 02:04:01 (`result='phantom_aged_out'`) → mika#2156.

### Hors portée, découvert pendant la planification — à ficher séparément

`800d739f-a0ed-485d-bef1-9990beeac396` (`long_running:run_claude_pilot`, `completed_at = 2026-09-03T22:03:24Z`, `result = "PIPELINE FAILURE: claude-pilot exited 1 …"`) est encore `completed` — jamais `delivered` — et son `updated_at` avançait toujours à `2026-09-04T02:02:11Z`, **quatre heures** après. Les sessions `callback-*` de mika-dev montrent la cadence : 4 minutes de tour, toutes les 5 minutes, en boucle (`00:44:11→00:48:11`, `00:49:11→00:53:11`, `00:54:11→00:58:11`, `00:59:11→01:03:11`, …). Un callback définitivement non livrable monopolise le verrou d'agent et affame tout ce qui est derrière lui.

C'est le mécanisme **aggravant** du cas mesuré, pas sa cause : le correctif ici rend la stérilité détectable et réparable, il ne débouche pas le verrou. Ticket séparé à ouvrir (classe : « ralentit la boucle », Tier 2). **Conséquence assumée sur AC1 :** tant que le verrou est affamé, le re-armement peut ne pas produire de dispatch réel et finira par épuiser son budget — AC1 est alors satisfait par sa **seconde branche**, l'échec visible portant une raison, ce qui est exactement ce que l'AC autorise.

**Action automatique sur la famine de promotion.** L2b s'arrête à la mesure. Le ticket qui portera l'action — fauche ou re-armement d'un wrapper promu non pris — ne peut être écrit qu'une fois la distribution des latences connue, parce que la seule observation dont on dispose (2 h 47) est aussi celle qui montre qu'un seuil naïf détruit du travail vivant. **Condition de réveil, datée et concrète :** quand `deferred_dispatch_promotion_starved` a produit sept jours de mesures avec le drapeau `agent_busy`, soit à partir du **2026-09-11**, et qu'un cas `agent_busy = false` y figure.

**L'instance mika#2158 signalée à l'ouverture du grooming n'appartient pas à cette classe.** Mesurée avant révision : la tâche `662d9752-e0e8-4a2b-869a-c711c37a7244` (`ready-label: senara-solutions/mika#2158`, créée `2026-09-04T09:44:04Z`) porte `result.error = "unauthorized_webhook_dispatch"` — le refus de consentement positif de mika#841 — et **non** `global_dispatch_active`. Elle est de surcroît `in_progress`, pas `blocked`, et le reste à `10:45:03Z` sans transition terminale. Le prédicat de L3b (`$.error = 'global_dispatch_active'` **et** `parent.status = 'blocked'`) l'exclut deux fois, correctement.

Ce n'est donc pas une seconde instance de ce défaut, et le plan ne s'en sert pas comme telle — l'écrire aurait fabriqué une preuve. C'est en revanche une **observation distincte à mesurer avant de ficher** : une tâche qui acquiert un bail (`dispatch_slot_leases`, `mika-dev|implement|662d9752`, acquis `09:44:05Z`, expiré `09:46:05Z`), se voit refuser l'autorisation, et reste `in_progress` une heure sans écrire de fin. Le bail étant expiré, elle n'affame personne (`dispatch_slot_lease_holder` filtre `expires_at > now`, `db.rs:8116`), ce qui la maintient sous le seuil « casse la boucle ». À confirmer sur un second cas avant d'ouvrir un ticket — un `in_progress` d'une heure peut être un tour long et non un tour mort (`feedback_hard_evidence_before_filing`, `feedback_n_equals_2_is_the_signal`).

---

## Livrables

### L1 — `expired` : l'enregistrement terminal honnête d'un wrapper (AC2)

**Le problème.** Un wrapper consommé sans dispatcher reste `completed` (promotion) puis passe `delivered` (`dispatcher.rs:512`) — deux mots qui affirment un succès. Les deux branches qui détectent déjà l'absence d'effet (`silent_turn_error` `dispatcher.rs:504-508`, `noop_completion` `dispatcher.rs:598-601`) re-arment le parent mais **ne corrigent pas l'enregistrement du wrapper**.

**Le changement.**

1. `crates/mika-agent/src/db.rs` — nouvelle méthode :
   ```rust
   /// Terminal record for a deferred wrapper consumed without dispatching (mika#2169).
   ///
   /// `expired` is deliberate: it is already in the `status` CHECK constraint,
   /// it is terminal, and it is NOT in the `('completed','failed')` set that
   /// `get_undelivered_callback_tasks` scans — so the wrapper leaves the
   /// delivery queue instead of re-entering it. Marking it `failed` would
   /// livelock the delivery scan.
   pub fn mark_deferred_wrapper_noop(&self, id: &str, reason: &str) -> Result<bool>
   ```
   `UPDATE tasks SET status='expired', result=?2, completed_at=?3, updated_at=?3 WHERE id=?1 AND label='long_running:run_claude_pilot:deferred' AND status IN ('completed','delivered')`. Le garde sur `label` et sur le statut de départ rend l'écriture idempotente et impossible à pointer sur autre chose qu'un wrapper.
2. `crates/mika-agent/src/async_db.rs` — le passe-plat `pub async fn mark_deferred_wrapper_noop`.
3. `crates/mika-agent/src/task_engine/dispatcher.rs` — dans `rearm_consumed_deferred_wrapper` (`:1256`), après l'appel à `rearm_deferred_callback`, écrire l'enregistrement :
   `mark_deferred_wrapper_noop(&task.id, &format!("noop: aucun dispatch produit (cause={cause})"))`.
   Les deux appelants existants (`silent_turn_error`, `noop_completion`) en héritent sans changement.

**Ce que ça donne.** Trois fins de vie possibles, trois mots distincts : `delivered` = le tour a dispatché ; `expired` + raison = le tour a eu lieu et n'a rien dispatché ; `completed` sans suite = la promotion n'a jamais tiré — et ce troisième cas devient un état **détectable**, ce que L2b mesure.

### L2 — garde du parent terminal, et famine observable sans être fauchée (AC2, Fait 3)

> **Ce livrable a changé de nature après la re-mesure.** Il était « chien de garde de la promotion qui n'a jamais tiré », avec une fenêtre de 300 s et une action destructrice (`expired` + re-armement). Fait 1 montre qu'aucune promotion n'est restée non tirée et que la plus lente a mis **2 h 47** : ce chien de garde aurait fauché vingt-deux wrappers vers `00:53Z` et échoué définitivement des parents servis à `03:35Z`. **La fenêtre ne peut pas distinguer la famine du vrai orphelin.** L2 garde donc le seul cas où l'action est certaine — le parent terminal — et rend la famine visible sans y toucher.

#### L2a — refuser le re-armement vers un parent terminal (AC2, ferme le cas du commentaire)

**Le problème.** `rearm_consumed_deferred_wrapper` re-arme sans regarder le parent. Quand celui-ci est `failed` (balayage stuck-pending) ou `cancelled` (`superseded_by_new_dispatch`, mesuré sur `4be3bc3d`), le re-armement est structurellement voué à l'échec : `run_claude_pilot` refuse (`Task is not an active task`) et `failed` est terminal. Le budget se consomme dans le vide et rien ne le dit.

**Le changement.** Dans `rearm_consumed_deferred_wrapper` (`dispatcher.rs:1256`), **avant** l'appel à `rearm_deferred_callback` :

- `get_task(parent_task_id)` ; si le parent est absent ou porte `status IN ('failed','cancelled','expired','completed','delivered')` :
  - `mark_deferred_wrapper_noop(&task.id, &format!("aucun dispatch produit ; parent {parent_id} terminal ({parent_status}) — re-armement impossible"))` ;
  - `warn!(event = "deferred_wrapper_orphaned_by_terminal_parent", …)` + `log_audit_event` avec `tool_name = "deferred_wrapper_orphaned_by_terminal_parent"` ;
  - **retourner sans re-armer.** Le budget n'est pas consommé pour rien, et la trace nomme la cause.
- sinon, le chemin actuel, augmenté de l'enregistrement L1.

**Ce que ça ferme.** Exactement la variante mesurée par le commentaire du ticket (Fait 3) : la reprise ne se termine plus vert, elle se termine `expired` en nommant le parent terminal qui la rend vaine.

#### L2b — la famine du verrou devient une mesure, pas une victime

**Le changement.** `db.rs` — `pub fn count_promoted_undelivered_wrappers(&self, agent_id: &str, stale_seconds: i64) -> Result<i64>` :

```sql
SELECT COUNT(*) FROM tasks
WHERE agent_id = ?1
  AND trigger_type = 'callback'
  AND label = 'long_running:run_claude_pilot:deferred'
  AND status = 'completed'
  AND completed_at IS NOT NULL
  AND completed_at < strftime('%Y-%m-%dT%H:%M:%SZ','now', ?2)
```

Après L1 et L2a, `status='completed'` sur ce label est **exclusivement** « promu, pas encore pris » : la livraison écrit `delivered`, la consommation stérile écrit `expired`. Le commentaire de la méthode doit nommer cette exclusivité, parce que c'est elle qui rend le compte lisible.

`engine.rs` — dans le bloc `DB_SCAN_INTERVAL_TICKS` existant, si le compte est non nul :
`warn!(event = "deferred_dispatch_promotion_starved", count, oldest_age_secs, agent_busy = <bool>)` + un `log_audit_event` unique par balayage. Le drapeau `agent_busy` est lu du verrou en place — `self.agent_lock.try_lock().is_err()` — et c'est lui qui rend le signal interprétable : famine derrière un verrou tenu (bénigne, elle se résorbe) *versus* wrappers en attente avec un agent **libre**, qui est le seul état réellement anormal.

**Aucune mutation d'état. Aucun re-armement. Aucune fenêtre destructrice.** Le seuil `MIKA_DEFERRED_PROMOTION_STALE_SECS` (défaut **900 s**, sur le modèle de `stuck_pending_reaper_grace_secs()`, `engine.rs:2271`) ne pilote qu'un avertissement. On mesure d'abord ; c'est la mesure qui dira, avec des chiffres, si une action automatique se justifie un jour — et sous quel prédicat.

> **Pourquoi s'arrêter à la mesure.** La demi-vie d'un remède posé contre une famine qu'on n'a pas caractérisée est courte, et son mode d'échec ici est de **détruire du travail vivant**. Les 2 h 47 mesurées ne sont pas un budget de patience choisi : c'est la première observation de cette latence. Un ticket séparé portera l'action une fois la distribution connue (voir § hors portée découvert).

### L3a — le `RearmOutcome` est jeté ; le faucheur qu'il invoque ne vient jamais (AC1, AC3)

**C'est la cause, lue à la ligne.** `rearm_consumed_deferred_wrapper` (`dispatcher.rs:1256-1270`) appelle `rearm_deferred_callback` et **ignore sa valeur de retour** — l'appel se termine par `.await;`, pas par un `match`. Or `RearmOutcome` existe précisément pour être discriminé : sa documentation dit que « collapser `NotNow` et `Unrepairable` en un booléen est le bug que cet enum existe pour empêcher » (`skills/executor.rs:2455-2470`). Ce site d'appel ne les collapse pas en un booléen : il les jette tous les deux.

Quand le budget est épuisé, `rearm_deferred_callback` émet `warn!(event = "deferred_dispatch_rearm_budget_exhausted", … "repair budget exhausted — leaving the task for the reaper to expire")` puis rend `Unrepairable` (`executor.rs:2518-2528`). **Le faucheur qu'il nomme ne vient pas pour ce parent :** `find_orphaned_pending_issue_tasks` porte `AND parent.status = 'pending'` (`db.rs:7484`, Fait 2). Un parent `blocked` tombe entre les deux — le dispatcher délègue l'expiration au faucheur, le faucheur ne le voit pas, et personne n'écrit jamais rien.

C'est la chaîne complète des 80 minutes, et elle tient en une valeur de retour non consommée.

**Le changement.** Dans `rearm_consumed_deferred_wrapper`, `match` sur le résultat :

- `Rearmed` → `info!(event = "deferred_wrapper_rearmed", …)`. Comportement actuel, désormais nommé.
- `NotNow` → `debug!` seulement. Condition transitoire, c'est la doctrine de l'enum ; ne rien détruire.
- `Unrepairable` → **écrire l'échec ici**, ne pas le déléguer :
  `update_task_failed(parent_id, &format!("re-armement différé épuisé après {MAX_STUCK_REARMS} tentatives (cause={cause}) — aucun dispatch produit"))`,
  `warn!(event = "deferred_dispatch_unrepairable_parent_failed", …)` + `log_audit_event` avec `tool_name = "deferred_dispatch_unrepairable_parent_failed"`.

**Ce que ça donne sur la trace mesurée.** `620ae345` serait passé `failed` à `03:59:50Z` avec pour motif « re-armement différé épuisé après 2 tentatives » — au lieu de rester `blocked` et de recevoir `phantom_aged_out`, un mot qui nomme le faucheur et masque la cause. **C'est l'échec visible portant une raison qu'AC1 exige, et il arrive sur le chemin qui sait pourquoi.**

Corriger le mot de la fin du wrapper (L1) sans corriger cette valeur jetée laisserait le parent exactement aussi muet. L1 et L3a sont les deux moitiés d'AC1/AC2 ; ni l'une ni l'autre ne suffit seule.

### L3b — balayage du trio « bloqueur terminé + bail expiré + parent bloqué » (AC1, AC3)

L3a couvre les parents dont le wrapper **a été consommé**. Il reste la population dont le wrapper n'a jamais atteint la consommation — refusée à l'enregistrement, ou dont le wrapper a disparu. C'est celle-là que le balayage prend, en filet.

**Le changement.**

1. `db.rs` — `pub fn find_stale_blocked_dispatch_tasks(&self, agent_id: &str, grace_seconds: i64) -> Result<Vec<StaleBlockedTask>>`. Population **discriminée**, pas élargie :
   ```sql
   WHERE parent.agent_id = ?1
     AND parent.status = 'blocked'
     AND parent.source = 'self_dev'
     AND parent.trigger_type = 'manual'
     AND parent.type = 'issue'
     AND parent.reference_url IS NOT NULL
     AND json_valid(parent.result)
     AND json_extract(parent.result, '$.error') = 'global_dispatch_active'
     AND parent.created_at < strftime('%Y-%m-%dT%H:%M:%SZ','now', ?2)
     AND NOT EXISTS (… wrapper `pending` …)      -- clauses reprises telles quelles de
     AND NOT EXISTS (… callback non-différé actif …)  -- find_orphaned_pending_issue_tasks
   ```
   Rendu : `id`, `reference_url`, `dispatch_class`, `rearm_count` (via le même garde `json_valid` sur `metadata.stuck_rearm_count`), et `blocking_callback_id` extrait de `json_extract(parent.result,'$.blocking_callback_id')`.

   Le prédicat `$.error = 'global_dispatch_active'` est ce qui sépare cette population de celle des portes délibérées : un `blocked` d'auto-merge ou d'escalade QA ne porte pas ce `result`. `620ae345` le portait — le corps du ticket cite le JSON verbatim.
2. `engine.rs` — `async fn reap_stale_blocked_dispatch_tasks(&self)`, placé **juste après** `reap_orphaned_pending_issue_tasks()` (`engine.rs:395`), même bloc de cadence. Pour chaque candidat, dans cet ordre :
   1. **Le bloqueur nommé est-il terminé ?** `get_task(blocking_callback_id)`. Si absent → traiter comme terminé (la ligne a disparu). Si `status IN ('pending','in_progress')` → **passer**, le bloqueur vit (AC5).
   2. **Le bail est-il expiré ?** `dispatch_slot_lease_holder(agent_id, class)` — cette méthode filtre déjà `expires_at > now` (`db.rs:8116-8133`), donc `None` **est** la réponse « expiré ou absent ». `Some(_)` → **passer** (AC5).
   3. Les deux libres → `rearm_deferred_callback(…, "stale_blocked_dispatch")`.
      - `Rearmed` → `update_task_status(parent, "pending")`. Le parent rentre alors dans la population que l'échelle mika#2045 couvre déjà ; ce balayage ne se substitue pas à elle, il l'alimente. `info!(event = "stale_blocked_dispatch_rearmed")` + audit.
      - `Unrepairable` → `update_task_failed(parent, "stale_blocked_dispatch: bloqueur <id> terminé, bail expiré, budget de re-armement épuisé")`. `warn!(event = "loop_stuck_blocked_tasks")` + audit. C'est l'échec **visible portant une raison** qu'AC1 autorise comme seconde branche.
      - `NotNow` → laisser, retenter au tick suivant (`debug!` seulement — condition transitoire, cf. la doctrine de `RearmOutcome`).
3. Fenêtre de grâce : réutiliser `stuck_pending_reaper_grace_secs()`. Pas de second bouton pour la même notion.

**Pourquoi un balayage séparé plutôt qu'un `status IN ('pending','blocked')` sur l'existant** — la question sera posée en revue, la réponse est au § « Ce que ces deux faits imposent » et se résume à : la population de `find_orphaned_pending_issue_tasks` n'a pas de discriminant qui distingue un refus de fente d'une porte opérateur, et lui en ajouter un la transformerait en la requête écrite ici. Deux populations, deux requêtes, chacune lisible seule.

### L4 — rejeu anti-vacuité de la trace du 2026-09-04 (AC4)

`crates/mika-agent/src/task_engine/engine.rs`, module de tests, `#[tokio::test] async fn test_replay_2026_09_04_stale_blocked_parent_with_dead_blocker()`.

Graine, aux identifiants et horodatages **verbatim** du ticket :

| ligne | valeurs |
|---|---|
| `74b3ee7d-c429-4479-ba72-dc877cc8b415` | callback `long_running:run_claude_pilot`, `status='completed'`, `completed_at='2026-09-04T00:42:16Z'` |
| `620ae345-f97b-44a0-b099-ebdf720be88c` | `source='self_dev'`, `trigger_type='manual'`, `type='issue'`, `reference_url='https://github.com/senara-solutions/mika/issues/2140'`, `status='blocked'`, `created_at='2026-09-04T00:42:12Z'`, `result` = le JSON verbatim du ticket (`error`, `blocking_callback_id`, `blocking_task_id`, `dispatch_class`, `deferred_dispatch_registered`) |
| `f0cd5967-5f22-4c66-940f-86c90beb7ed1` | wrapper, parent `620ae345`, `status='delivered'`, `result='deferred dispatch slot freed'`, `completed_at='2026-09-04T00:48:11Z'` |
| `9943f191-57eb-466b-a8d8-178affd0999d` | wrapper de remplacement, parent `620ae345`, `status='delivered'`, `created_at='2026-09-04T03:35:31Z'` |
| `4e025a63-1e92-4138-9182-e53f11e8475a` | wrapper de remplacement, parent `620ae345`, `status='delivered'`, `created_at='2026-09-04T03:59:50Z'` |
| `620ae345.metadata` | `{"stuck_rearm_count": 2, "claude_pilot": {"branch": "fix/2140/auto-pull-la-porte-de-promotion-lit"}}` — budget épuisé, **branche accentuée** (voir ci-dessous) |
| `dispatch_slot_leases` | `(mika-dev, implement, c479c873-…, acquired 2026-09-03T23:29:44Z, expires 2026-09-03T23:31:44Z)` — expiré |

Les trois wrappers sont `delivered`, pas `completed` : c'est la trace réelle telle que Fait 1 la mesure, et c'est ce qui rend le rejeu honnête. Une graine qui les poserait `completed` testerait un état que la production n'a pas produit.

Assertions :

1. **AC2, sur le mot de la fin.** Après le tour stérile du troisième wrapper, `4e025a63.status == "expired"` et son `result` contient la cause. Aucun des trois ne reste `delivered` sans avoir dispatché.
2. **AC1/AC3, sur la sortie de budget.** `620ae345` porte `stuck_rearm_count = 2` ; le quatrième passage rend `Unrepairable`, et L3a écrit `620ae345.status == "failed"` avec un `result` qui contient `re-armement différé épuisé`. **Le trio n'est plus un état stable**, et le motif nomme la cause, pas le faucheur.
3. **AC3, sur le filet.** `reap_stale_blocked_dispatch_tasks()` sur une graine dont le wrapper n'a jamais été consommé → `620ae345.status != "blocked"`.
4. Un `audit_events` porte `tool_name = "deferred_dispatch_unrepairable_parent_failed"` (compté via `count_audit_events_by_tool_name`, déjà `#[doc(hidden)]` public pour cet usage).

**Cas accentué, obligatoire et non décoratif.** Les motifs que L1, L2a et L3a écrivent dans `tasks.result` sont en français et portent des accents (`re-armement différé épuisé`, `terminal — re-armement impossible`), et la métadonnée du parent porte une branche réelle `fix/2140/auto-pull-la-porte-de-promotion-lit`. Le rejeu **doit** asserter sur la sous-chaîne accentuée exacte — `assert!(result.contains("re-armement différé épuisé"))` — et non sur un préfixe ASCII. Deux raisons, toutes deux mesurées dans ce dépôt :

- c'est notre population nominale : nos motifs, nos branches et nos titres de tickets sont accentués (`feedback_notre_entree_nominale_est_accentuee`) ;
- `result` transite par des lecteurs qui tronquent, et la troncature sur frontière d'octet est une classe de panique déjà rencontrée ici (`docs/plans/2103-truncate-output-byte-boundary-panic-and-class-audit.md`). Un motif multi-octets tronqué à 120 octets casse au milieu d'un `é`.

Le cas doit **rougir quand on retire le correctif** : sans L3a, `result` vaut `phantom_aged_out` ou reste nul, et `contains("re-armement différé épuisé")` est faux. C'est un rougissement sur assertion, pas sur compilation.

**Vérification anti-vacuité, à exécuter et à consigner** (`feedback_verify_pipeline_passes_without_the_fix`).

La révision du plan a rendu cette vérification **directement exécutable**, et c'est un gain qu'il faut exploiter plutôt que contourner. Le cœur du correctif — L3a — ne crée aucune surface : il ajoute un `match` sur la valeur de retour d'un appel qui existe déjà, dans une fonction qui existe déjà (`rearm_consumed_deferred_wrapper`, `dispatcher.rs:1256`). L'assertion portante (AC1/AC3) s'écrit donc **entièrement contre des surfaces présentes sur `main`** :

```rust
// Graine : la trace de Fait 1, stuck_rearm_count = 2, parent `blocked`.
dispatcher.rearm_consumed_deferred_wrapper(&wrapper, "noop_completion").await;
let parent = db.get_task(PARENT_ID).await.unwrap().unwrap();
assert_eq!(parent.status, "failed");
assert!(parent.result.unwrap().contains("re-armement différé épuisé"));
```

Sur `main`, ce test **compile et rougit** : le budget est épuisé, `rearm_deferred_callback` rend `Unrepairable`, la valeur est jetée, et le parent reste `blocked`. `assert_eq!(parent.status, "failed")` échoue sur une assertion, pas sur la compilation. C'est la preuve de rougissement recevable, sans neutralisation de méthode ni protocole de démontage.

**Protocole :** `git stash` du correctif, `cargo test … test_replay_2026_09_04`, coller la sortie rouge dans le corps de la PR, restaurer. La sortie rouge est une pièce de la PR, pas une affirmation dans le texte.

Les assertions qui touchent des surfaces **nouvelles** (L2b, L3b) sont isolées dans des tests distincts, précisément pour que le test portant reste compilable sur `main`. Un test qui ne compile pas n'est pas une preuve de rougissement, et mélanger les deux dans un seul `#[tokio::test]` détruirait la preuve du premier.

### L5 — non-régression : un bloqueur réellement actif refuse toujours (AC5)

Deux tests, tous deux dans `engine.rs` :

1. `test_stale_blocked_sweep_skips_live_blocker()` — même graine que L4, sauf `74b3ee7d.status = 'in_progress'`. Après le balayage : `620ae345.status == "blocked"`, `stuck_rearm_count` absent, aucun wrapper de remplacement. Le balayage n'a rien touché.
2. `test_stale_blocked_sweep_skips_unexpired_lease()` — même graine que L4, bloqueur `completed`, mais le bail porte `expires_at = now + 600s`. Même assertion : rien touché. C'est le cas qui protège la sérialisation que mika#2160 tient sous garde opérateur.

Et une assertion de garde sur la porte elle-même : le test existant `executor.rs:5618-5662` (`assert_eq!(v["error"], "global_dispatch_active")`) reste vert sans modification — à vérifier, pas à réécrire. La sérialisation à un `implement` par classe n'est pas ce qu'on retire.

---

## Séquence

1. **L1** — `mark_deferred_wrapper_noop` (db + async_db + `rearm_consumed_deferred_wrapper`). C'est la fondation : L2a écrit par elle, et L2b dépend de l'exclusivité de `status='completed'` qu'elle établit.
2. **L3a** — le `match` sur `RearmOutcome` dans `rearm_consumed_deferred_wrapper`, et l'écriture de l'échec de budget. **Livrable minimal viable** : L1 + L3a satisfont déjà AC1, AC2 et AC3 sur la trace mesurée.
3. **L2a** — la garde du parent terminal, dans la même fonction que L3a. À faire dans la foulée : les deux touchent `rearm_consumed_deferred_wrapper`, et les séparer créerait un conflit inutile.
4. **L2b** — le compteur de famine + l'avertissement. Indépendant, sans mutation d'état.
5. **L3b** — `find_stale_blocked_dispatch_tasks` + `reap_stale_blocked_dispatch_tasks` + le branchement dans `tick()`. Le filet, écrit après que le chemin direct est correct.
6. **L4** — le rejeu, puis la mesure du rouge pré-correctif selon le protocole ci-dessous.
7. **L5** — les deux non-régressions.

L1 avant L3a et L2a est une dépendance dure. **L2a et L3a sont deux modifications de la même fonction et se font ensemble.** L3b est indépendante et peut être écrite en parallèle ; L4 les assemble et vient donc après.

> **Si la portée doit être réduite en revue**, l'ordre de coupe est : L3b, puis L2b. L1 + L2a + L3a est le noyau irréductible — c'est lui qui ferme la trace mesurée, et retirer l'un des trois rouvre AC1 ou AC2.

---

## Risques et ce qui les borne

| Risque | Ce qui le borne |
|---|---|
| **Fauchage de travail vivant** : un chien de garde marque `expired` un wrapper qui allait être livré. | **C'est le risque que la re-mesure a matérialisé**, et il est écarté par construction : L2b ne mute aucun état, et L2a n'agit que sur un parent déjà terminal — un état dont on ne revient pas. Aucun livrable ne détruit un wrapper sur un critère de délai. |
| **Double écriture d'échec** : L3a échoue le parent, puis L3b le retrouve et l'échoue encore. | L3b porte `parent.status = 'blocked'` dans sa population ; un parent que L3a vient de passer `failed` en sort. Les deux chemins sont disjoints par le statut, pas par l'ordonnancement — c'est ce qui les rend sûrs indépendamment du tick. Un test de L5 le vérifie. |
| **Double re-armement** : L2a/L3a et L3b réparent le même parent au même tick. | `rearm_deferred_callback` interroge `has_non_deferred_active_callback_child` en premier et rend `NotNow` si un dispatch existe ; le budget partagé `MAX_STUCK_REARMS = 2` borne le total quel que soit le chemin — c'est écrit dans sa doc et c'est le contrat qu'on hérite. L4 le vérifie (un seul wrapper de remplacement, pas deux). |
| **Re-armement contre une porte opérateur** : L3b relance un dispatch que Vincent avait délibérément bloqué. | Le prédicat `$.error = 'global_dispatch_active'` ; un `blocked` d'auto-merge ou d'escalade QA ne porte pas ce `result`. Vérifié : les quatre seuls sites de production qui écrivent `blocked` sont `server/verdict_handler.rs:999/1289/1496/2160`, et la porte forge-gate est `server/ci_success_handler.rs:848`. Aucun ne porte `global_dispatch_active` dans `result`. Contrôle négatif mesuré : `662d9752` porte `unauthorized_webhook_dispatch` et est exclue. |
| **Cycle `blocked → pending → blocked`** : le LLM re-bloque le parent que L3b vient de rendre `pending`. | Le budget `MAX_STUCK_REARMS` s'applique au parent, pas au cycle : au troisième passage, `Unrepairable` → `update_task_failed` avec raison. Le cycle est borné à deux tours et se termine **visiblement**. |
| **`expired` mal interprété ailleurs** : un lecteur compte `expired` comme un échec de pilote. | **Résolu à la lecture, plus « à confirmer ».** Le seul écrivain de production est `mark_tasks_expired` (`db.rs:6896`), gardé par `timeout_at IS NOT NULL` et `status NOT IN (…,'delivered')` — il ne peut pas toucher un wrapper marqué par L1. Le seul lecteur de production qui range `expired` avec les échecs est le garde anti-zombie de `create_recurring_task_if_absent` (`db.rs:5761`), filtré sur `trigger_type = 'recurring'` : un wrapper différé est `trigger_type = 'callback'` et lui est invisible. Aucun ajustement de lecteur n'est requis. |
| **Le verrou d'agent reste affamé** et aucun re-armement ne produit de dispatch. | Assumé et déclaré (§ hors portée). AC1 se satisfait alors par sa seconde branche — échec visible portant une raison — désormais réellement écrite par L3a. Le ticket séparé sur `800d739f` porte la cause, et L2b en fournira la mesure. |
| **La révision du plan se trompe à son tour**, comme la première rédaction s'est trompée sur Fait 1. | Les faits portants de cette révision sont des états terminaux (`delivered`, `stuck_rearm_count = 2`, `failed`/`phantom_aged_out`) et une lecture de code (`RearmOutcome` jeté), pas des états de transit. La première erreur venait d'avoir conclu sur un `status='completed'` qui était un transit — `feedback_never_conclude_inside_the_mechanism`. Le protocole anti-vacuité ci-dessus est le contrôle : si la lecture est fausse, le test rougit **aussi** avec le correctif. |

---

## Critères d'acceptation — correspondance

| AC | Livrable | Comment on le mesure |
|---|---|---|
| **AC1** — un `ready-label` refusé dont la fente se libère produit un dispatch réel **ou** un échec visible portant une raison ; jamais `blocked` en silence. | **L3a** + L3b | L4 assertion 2 : le budget épuisé écrit `620ae345.status == "failed"` avec un `result` contenant `re-armement différé épuisé`, sur le chemin qui connaît la cause. L3b couvre les parents dont le wrapper n'a jamais été consommé. Un `audit_events` est écrit dans les deux. **Mesuré aujourd'hui : cette seconde branche n'existait sur aucun chemin.** |
| **AC2** — le succès du re-tir se mesure sur son effet, pas sur le réveil ; une reprise qui n'a rien dispatché ne se termine pas `completed`. | L1 + L2a | L1 : toute consommation stérile écrit `expired` + raison, au lieu de `delivered` + `deferred dispatch slot freed`. L2a : le parent terminal sort par une branche nommée au lieu d'un re-armement voué à l'échec. L4 assertion 1 mesure ça sur les trois wrappers réels. |
| **AC3** — le trio « bloqueur terminé + bail expiré + parent bloqué » n'est pas un état stable. | **L3a** + L3b | L4 assertions 2 et 3. La graine **est** le trio (bloqueur `completed`, bail `expires 23:31:44Z`, parent `blocked`). Deux sorties disjointes par statut : L3a quand le wrapper a été consommé, L3b sinon. |
| **AC4** — anti-vacuité : le rejeu rougit sans le correctif. | L4 | Le test portant compile sur `main` et rougit sur assertion — pas de neutralisation de méthode. Protocole `git stash` ci-dessus, sortie rouge collée dans le corps de la PR. Cas accentué obligatoire. |
| **AC5** — un bloqueur réellement actif continue de refuser le second dispatch. | L5 | Deux tests (bloqueur vivant / bail non expiré) : le balayage ne touche rien. Plus la vérification que `executor.rs:5618-5662` reste vert sans modification. |

---

## Vérification

```bash
cargo test -p mika-agent test_replay_2026_09_04          # L4 — le rejeu portant (AC1/AC2/AC3/AC4)
cargo test -p mika-agent test_stale_blocked_sweep        # L5 — les deux non-régressions (AC5)
cargo test -p mika-agent rearm                           # L1/L2a/L3a — la famille rearm_* existante
cargo test -p mika-agent deferred                        # toute la famille existante mika#1011/#1124/#2045
cargo test -p mika-agent global_dispatch_active          # la porte que l'on ne retire pas (AC5)
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

Plus la mesure pré-correctif de L4 (`git stash` → test → sortie rouge → restauration), dont la sortie est une **pièce de la PR** et non une affirmation dans le corps du texte.

> **Garde de non-régression sur la famille existante.** `cargo test -p mika-agent rearm` et `… deferred` doivent être verts **avant** le correctif et **après**. `rearm_consumed_deferred_wrapper` est appelée par deux sites (`silent_turn_error`, `noop_completion`) : y ajouter une garde de parent terminal et un `match` change le comportement des deux. Si un test existant de cette famille rougit après le correctif, c'est un signal à lire — pas à ajuster.

---

## Fichiers touchés

| Fichier | Nature |
|---|---|
| `crates/mika-agent/src/db.rs` | +3 méthodes (`mark_deferred_wrapper_noop`, `count_promoted_undelivered_wrappers`, `find_stale_blocked_dispatch_tasks`) + 1 struct de rendu |
| `crates/mika-agent/src/async_db.rs` | +3 passe-plats |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | **Le cœur.** `rearm_consumed_deferred_wrapper` : garde du parent terminal (L2a), `match` sur `RearmOutcome` (L3a), enregistrement terminal du wrapper (L1) |
| `crates/mika-agent/src/task_engine/engine.rs` | +1 balayage (L3b), +1 compteur de famine (L2b), +1 lecteur d'env, 2 lignes dans `tick()`, +3 tests |

Aucune migration de schéma : `expired` est déjà dans la contrainte `CHECK` (`db.rs:1342-1343`), `dispatch_slot_leases` existe depuis v51.

**Où le poids s'est déplacé.** La première rédaction mettait l'essentiel dans `engine.rs` — deux balayages neufs pilotés par des fenêtres de temps. La re-mesure l'a déplacé dans `dispatcher.rs`, sur trois modifications d'une seule fonction existante, dont la principale est de **consommer une valeur de retour que le code jetait**. C'est un correctif plus petit, plus près de la cause, et dont l'anti-vacuité se mesure sans démonter quoi que ce soit.
