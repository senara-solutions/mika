# Plan : le re-tir différé se termine vert sans dispatcher (mika#2169)

**Ticket :** mika issue#2169 — `fix(task_engine): le re-tir différé se termine vert sans dispatcher — 80 min de boucle muette avec la fente libre depuis 4 secondes`
**Labels :** `bug`, `p1-important`
**Type :** issue (bug — moteur, classe « casse la boucle »)
**Palier de priorité :** Tier 1 — *casse la boucle*. La chaîne `ready-label` refusée sur fente occupée n'a aucun chemin de reprise **qui laisse une trace** : le re-tir écrit `delivered` sans avoir dispatché, brûle son budget contre un parent parfois déjà mort, et le ticket finit fauché sous un mot qui nomme le faucheur. **Ce que ce plan ne fait pas** : raccourcir les 80 minutes de silence du 2026-09-04 — leur cause est la famine de la file de livraison, hors portée et portée par un ticket séparé (Fait 1bis).

---

## Ce que la base dit, relu avant de planifier

Le corps du ticket décrit la trace. La lecture de la base (`~/.mika/data/mika.db`, 2026-09-04) la complète — et sur un point la corrige — en quatre faits qui décident de la forme du correctif. Le quatrième — Fait 1bis — a été mesuré au checkpoint de réconciliation et **corrige la troisième rédaction de ce plan**, pas seulement le corps du ticket.

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

**2. L'échelle de re-armement existante a fonctionné.** Chaque livraison stérile a été détectée : `rearm_consumed_deferred_wrapper` (`dispatcher.rs:1256`) a tiré à `03:35:31Z` puis à `03:59:50Z`, créant les deux wrappers de remplacement à l'horodatage exact de la livraison précédente. Le parent porte `metadata.stuck_rearm_count = 2` — le budget `MAX_STUCK_REARMS` a été consommé jusqu'au bout. **Le mécanisme de détection n'est pas ce qui manque** — mais Fait 1bis montre contre quoi il a tiré : un parent déjà terminal depuis `02:04:01Z`.

**3. Ce qui manque est l'enregistrement, et la sortie du budget.** Les trois wrappers finissent `delivered` — le mot le plus affirmatif du vocabulaire — avec pour `result` `deferred dispatch slot freed`, qui décrit la promotion et non l'effet. Aucun des trois n'a dispatché : sur la fenêtre élargie `00:00Z`–`07:00Z`, **une seule** tâche `long_running:run_claude_pilot` existe (`154dc21d`, créée `06:40:07Z`), soit près de six heures après le refus.

C'est exactement le défaut que le ticket décrit, mesuré une station plus loin que la première rédaction ne le croyait : non pas « le re-tir n'a jamais tiré », mais **« le re-tir a tiré trois fois, n'a rien dispatché trois fois, et a écrit `delivered` trois fois »**.

### Fait 1bis — le parent était déjà mort quand les trois tours ont eu lieu

> **Seconde correction, 2026-09-04, au checkpoint de réconciliation.** La rédaction précédente écrivait que `620ae345` « est resté `blocked` jusqu'à ce que le balayage phantom le fauche à `03:59:50Z` ». **Les deux moitiés sont fausses**, et l'ordre qu'elles cachaient déplace l'attribution de la cause. Corriger ici est moins coûteux que faire signer un plan dont la phrase centrale ne tient pas.

`audit_events` donne la séquence, sans interprétation :

| horodatage (UTC) | `tool_name` | transition |
|---|---|---|
| `00:42:12Z` | `ready_label_handled`, `deferred_dispatch_registered` | dispatch préparé, différé enregistré |
| `00:48:31Z` | `update_task_status` | `pending` → **`blocked`** (écrit par le LLM, cf. Fait 2) |
| `02:04:01Z` | `phantom_aged_out` | `blocked` → **`failed`** |
| `03:35:31Z` | `deferred_dispatch_rearmed` | re-armement **n°1** |
| `03:59:50Z` | `deferred_dispatch_rearmed` | re-armement **n°2** |

Le parent est terminal depuis `02:04:01Z`. La livraison du premier wrapper — donc son premier tour stérile — est à `03:35:31Z`, **une heure et demie plus tard**. Les trois consommations stériles (`03:35:31Z`, `03:59:50Z`, `04:01:55Z`) ont donc **toutes** eu lieu contre un parent déjà `failed`, et les deux re-armements ont brûlé le budget contre un cadavre.

**Conséquence 1 — la trace mesurée appartient à la classe L2a, pas à la classe L3a.** `620ae345` est une **seconde instance** du cas que le commentaire du ticket signale sur `14465667` (Fait 3) : re-armement vers un parent terminal. n=2, sur deux parents distincts et deux causes de terminaison distinctes — `stuck_pending_no_deferred_wrapper` pour l'un, `phantom_aged_out` pour l'autre. **L2a cesse d'être un livrable de bord : c'est lui qui ferme la trace du ticket.**

**Conséquence 2 — les quatre-vingts minutes ne tiennent pas dans le `RearmOutcome` jeté.** La fenêtre muette va de `00:42:12Z` à `02:04:01Z`. Pendant toute cette fenêtre le wrapper était promu (`00:48:11Z`) et **non encore livré** : il attendait derrière la file de livraison, dont la tête `800d739f` — un callback `PIPELINE FAILURE` `completed` depuis `2026-09-03T22:03:24Z` — n'a été marquée `delivered` qu'à `03:09:16Z`, **cinq heures et demie** après sa complétion. Aucun re-armement n'avait encore tiré ; aucune valeur de retour n'avait encore été jetée. La cause du silence est la **famine de la file de livraison**, et elle est hors portée de ce ticket (§ hors portée découvert, dont le palier est relevé en conséquence).

Ce plan **ne prétend donc plus expliquer les 80 minutes par son correctif**. Il ferme la classe que le titre du ticket nomme — « se termine vert sans dispatcher » — et il nomme, sans le recouvrir, le mécanisme qui a produit le silence. Le confondre reviendrait à livrer un correctif qui laisse la boucle muette et à croire l'avoir réparée.

> **Conséquence sur AC1.** L'échec visible portant une raison qu'AC1 exige comme seconde branche **n'existe sur aucun chemin aujourd'hui** — ni sur le chemin du parent terminal (L2a le pose), ni sur celui du budget épuisé contre un parent vivant (`RearmOutcome::Unrepairable` est jeté ; L3a le pose). Ces deux populations n'ont pas le même statut de preuve : la première est mesurée deux fois, la seconde est **lue à la ligne et sans instance mesurée**. Le plan le dit plutôt que d'emprunter à la trace de `620ae345` une preuve qu'elle ne porte pas.

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

**Et cette population n'a pas une seule instance.** Fait 1bis mesure la seconde : `620ae345`, le parent du ticket lui-même, dont les trois tours stériles et les deux re-armements ont tous eu lieu après `02:04:01Z`. Deux parents, deux causes de terminaison différentes, un seul défaut. `feedback_n_equals_2_is_the_signal` : ce n'est plus une variante signalée en commentaire, c'est le cas central.

### Ce que ces quatre faits imposent

- **AC2 se corrige à la fin du tour** — le tour a lieu ; ce qu'il écrit (`delivered` + `deferred dispatch slot freed`) est ce qui ment. **L1 et L2a sont les deux livrables portants**, et leur cas de mesure est la trace des trois wrappers consommés contre un parent terminal (Fait 1 + Fait 1bis).
- **Le chien de garde de la promotion « jamais tirée » perd son cas mesuré, et devient dangereux tel qu'il était spécifié.** Aucune promotion n'est restée non tirée : la plus lente a mis 2 h 47. Un chien de garde à 300 s aurait marqué `expired` les vingt-deux wrappers vers `00:53Z`, re-armé, épuisé le budget en une dizaine de minutes, et **échoué définitivement des parents qui allaient être servis à `03:35Z`**. La fenêtre seule ne peut pas séparer la famine du vrai orphelin : il faut un discriminant d'état, pas un délai. L2 est re-spécifié sur cette base.
- **AC1/AC3 ne se corrigent pas en élargissant `find_orphaned_pending_issue_tasks` à `status IN ('pending','blocked')`** : `blocked` est aussi le mot des portes délibérées (auto-merge refusé, `server/verdict_handler.rs:999/1289/1496/2160`, `server/ci_success_handler.rs:848`). Élargir rearmerait un dispatch contre une porte opérateur. Il faut un balayage **discriminant**, dont la population est exactement le refus de fente.
- **AC1 exige une sortie visible sur deux populations, dont une seule est mesurée.** Population A — parent terminal : mesurée deux fois (Fait 1bis, Fait 3), fermée par L2a. Population B — budget épuisé contre un parent vivant : **aucune instance mesurée**, mais le défaut se lit à la ligne (`RearmOutcome` jeté, § L3a) et l'AC l'exige explicitement. L3a la porte, et le plan assume la différence de statut de preuve entre les deux plutôt que de la maquiller.
- **La cause du silence mesuré n'est pas dans la portée de ce ticket.** Les 80 minutes viennent de la famine de la file de livraison (Fait 1bis, conséquence 2). Ce plan rend la stérilité **enregistrée, bornée et visible** ; il ne débouche pas la file. Dire l'inverse ferait passer pour réparée une boucle qui resterait muette.

---

## Portée

### Dans la portée

- **L1** — un enregistrement terminal qui porte l'**effet** pour chaque fin de vie d'un wrapper différé (AC2).
- **L2a** — refuser le re-armement vers un parent terminal, et le dire (AC2, cas du commentaire).
- **L2b** — rendre la famine de promotion **mesurable**, sans action destructrice (AC2, observabilité).
- **L3a** — consommer le `RearmOutcome` jeté, et écrire l'échec de budget avec son motif (AC1, AC3).
- **L3b** — un balayage du trio « bloqueur terminé + bail expiré + parent bloqué », en filet (AC1, AC3).
- **L4a** — rejeu **verbatim** de la trace du 2026-09-04 (parent terminal), rouge sans le correctif, cas accentué (AC2, AC4).
- **L4b** — rejeu **construit** du contrefactuel (parent vivant, budget épuisé), rouge sans le correctif (AC1, AC3, AC4).
- **L4c** — test du filet L3b ; surface neuve, donc hors du protocole anti-vacuité, et dit comme tel (AC3).
- **L5** — deux tests de non-régression sur le bloqueur réellement actif (AC5).

### Hors portée (repris du corps du ticket)

- La sérialisation `implement` à 1 → mika#2160, sous garde opérateur.
- Le refus `bash-grep` qui a tué le pilote initial → claude-pilot#151 volet (A).
- Le balayage phantom qui a fauché `620ae345` à 02:04:01 (`result='phantom_aged_out'`) → mika#2156.

### Hors portée, découvert pendant la planification — à ficher séparément

`800d739f-a0ed-485d-bef1-9990beeac396` (`long_running:run_claude_pilot`, `completed_at = 2026-09-03T22:03:24Z`, `result = "PIPELINE FAILURE: claude-pilot exited 1 …"`) est resté en tête de la file de livraison **cinq heures et demie** : `completed` à `22:03:24Z` le 09-03, `delivered` seulement à `2026-09-04T03:09:16Z`. Les sessions `callback-*` de mika-dev montrent la cadence pendant ce temps : 4 minutes de tour, toutes les 5 minutes, en boucle (`00:44:11→00:48:11`, `00:49:11→00:53:11`, `00:54:11→00:58:11`, `00:59:11→01:03:11`, …). Un callback qui ne se laisse pas livrer monopolise le verrou d'agent et affame tout ce qui est derrière lui.

> **Ce n'est pas un aggravant : c'est la cause du silence mesuré** (Fait 1bis, conséquence 2). La rédaction précédente le rangeait en « ralentit la boucle », Tier 2, sur la foi d'une lecture qui attribuait les 80 minutes au `RearmOutcome` jeté. Cette attribution est fausse. La chronologie est sans ambiguïté : le wrapper de `620ae345` est promu à `00:48:11Z` et livré à `03:35:31Z`, **immédiatement après** que la tête de file se soit débloquée à `03:09:16Z`. **Palier relevé : Tier 1 — casse la boucle.** Le ticket séparé est à ouvrir avec cette trace, et il porte le silence ; celui-ci porte l'enregistrement.

Ce que le présent correctif fait et ne fait pas, dit sans ambiguïté : il rend la stérilité **enregistrée, bornée et visible** ; il **ne débouche pas** la file de livraison. **Conséquence assumée sur AC1 :** tant que la file est affamée, le re-armement peut ne pas produire de dispatch réel et finira par épuiser son budget — AC1 est alors satisfait par sa **seconde branche**, l'échec visible portant une raison, ce qui est exactement ce que l'AC autorise. Une boucle silencieuse mais **lisible** est le contrat de ce ticket ; une boucle qui redémarre est le contrat de l'autre.

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

**Le défaut, lu à la ligne — et sans instance mesurée.** `rearm_consumed_deferred_wrapper` (`dispatcher.rs:1256-1270`) appelle `rearm_deferred_callback` et **ignore sa valeur de retour** — l'appel se termine par `.await;`, pas par un `match`. Or `RearmOutcome` existe précisément pour être discriminé : sa documentation dit que « collapser `NotNow` et `Unrepairable` en un booléen est le bug que cet enum existe pour empêcher » (`skills/executor.rs:2455-2470`). Ce site d'appel ne les collapse pas en un booléen : il les jette tous les deux.

Quand le budget est épuisé, `rearm_deferred_callback` émet `warn!(event = "deferred_dispatch_rearm_budget_exhausted", … "repair budget exhausted — leaving the task for the reaper to expire")` puis rend `Unrepairable` (`executor.rs:2518-2528`). **Le faucheur qu'il nomme ne vient pas pour ce parent :** `find_orphaned_pending_issue_tasks` porte `AND parent.status = 'pending'` (`db.rs:7484`, Fait 2). Un parent `blocked` tombe entre les deux — le dispatcher délègue l'expiration au faucheur, le faucheur ne le voit pas, et personne n'écrit jamais rien.

Le trou est réel et il est structurel : **un parent `blocked` dont le budget s'épuise n'a, aujourd'hui, aucun chemin qui écrive quoi que ce soit.** Le dispatcher délègue l'expiration à un faucheur qui ne regarde que `pending` ; le faucheur ne vient jamais ; personne n'écrit.

> **Ce trou n'a pas d'instance mesurée, et le plan ne lui en invente pas.** Sur la trace du 2026-09-04, le balayage phantom a terminalisé `620ae345` à `02:04:01Z` **avant** que le premier tour stérile n'ait lieu (Fait 1bis) : le budget s'est épuisé contre un parent déjà mort, ce qui relève de L2a. L3a ferme la branche où le parent est **encore vivant** quand le budget tombe — une population que le code autorise, que AC1 et AC3 exigent de fermer, et dont la preuve est une lecture de `dispatcher.rs:1256-1270` plus la doctrine écrite de `RearmOutcome` (`executor.rs:2455-2470`), pas un incident. C'est un statut de preuve plus faible que celui de L2a, et il est dit ici plutôt que masqué.

**Le changement.** Dans `rearm_consumed_deferred_wrapper`, `match` sur le résultat :

- `Rearmed` → `info!(event = "deferred_wrapper_rearmed", …)`. Comportement actuel, désormais nommé.
- `NotNow` → `debug!` seulement. Condition transitoire, c'est la doctrine de l'enum ; ne rien détruire.
- `Unrepairable` → **écrire l'échec ici**, ne pas le déléguer :
  `update_task_failed(parent_id, &format!("re-armement différé épuisé après {MAX_STUCK_REARMS} tentatives (cause={cause}) — aucun dispatch produit"))`,
  `warn!(event = "deferred_dispatch_unrepairable_parent_failed", …)` + `log_audit_event` avec `tool_name = "deferred_dispatch_unrepairable_parent_failed"`.

**Ce que ça donne, et sur quelle trace.** Sur le **contrefactuel** de la trace mesurée — la même chaîne sans le balayage phantom de `02:04:01Z`, donc un parent resté `blocked` — le troisième tour stérile rendrait `Unrepairable` et L3a écrirait `620ae345.status = 'failed'` avec pour motif « re-armement différé épuisé après 2 tentatives », au lieu de laisser la tâche `blocked` indéfiniment. Sur la trace **réelle**, c'est L2a qui parle en premier, et le mot final du parent reste `phantom_aged_out` — un mot qui nomme le faucheur et masque la cause, mais qui appartient à mika#2156, hors portée ici.

**C'est l'échec visible portant une raison qu'AC1 exige**, posé sur le chemin qui connaît la cause ; il se mesure en rejeu construit (L4b), pas en rejeu verbatim.

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

### L4 — rejeu anti-vacuité : un rejeu verbatim, un rejeu construit, et la frontière dite (AC4)

Fait 1bis impose de **séparer ce que la trace prouve de ce qu'elle ne prouve pas**. La trace réelle est celle d'un parent terminal (population A) ; l'épuisement de budget contre un parent vivant (population B) n'y figure pas. Les mélanger dans un seul test ferait passer un scénario construit pour une mesure — exactement la faute que la première rédaction de ce plan a commise sur `f0cd5967`, et que la seconde a commise sur l'horodatage du fauchage. Trois tests, trois statuts de preuve distincts.

Tous dans `crates/mika-agent/src/task_engine/engine.rs`, module de tests, préfixe commun `test_replay_2026_09_04` pour rester sélectionnables en une commande.

#### L4a — `test_replay_2026_09_04_terminal_parent_rearm_into_corpse` — **rejeu verbatim** (AC2, AC4)

Graine, aux identifiants et horodatages **verbatim** de la trace mesurée :

| ligne | valeurs |
|---|---|
| `74b3ee7d-c429-4479-ba72-dc877cc8b415` | callback `long_running:run_claude_pilot`, `status='completed'`, `completed_at='2026-09-04T00:42:16Z'` |
| `620ae345-f97b-44a0-b099-ebdf720be88c` | `source='self_dev'`, `trigger_type='manual'`, `type='issue'`, `reference_url='https://github.com/senara-solutions/mika/issues/2140'`, **`status='failed'`**, `result='phantom_aged_out'`, `completed_at='2026-09-04T02:04:01Z'`, `created_at='2026-09-04T00:42:12Z'` |
| `f0cd5967-5f22-4c66-940f-86c90beb7ed1` | wrapper, parent `620ae345`, promu `00:48:11Z`, **non encore consommé** au moment du rejeu |
| `620ae345.metadata` | `{"claude_pilot": {"branch": "fix/2140/auto-pull-la-porte-de-promotion-lit"}}` — **sans** `stuck_rearm_count` : le budget est intact au moment où le tour a lieu |
| `dispatch_slot_leases` | `(mika-dev, implement, c479c873-…, acquired 2026-09-03T23:29:44Z, expires 2026-09-03T23:31:44Z)` — expiré |

Le parent est semé **`failed`**, et non `blocked` : c'est son état réel à `03:35:31Z`, l'instant du premier tour stérile. Une graine qui le poserait `blocked` testerait un état que la production n'avait plus.

Acte : `dispatcher.rearm_consumed_deferred_wrapper(&f0cd5967, "noop_completion").await;`

Assertions :

1. `f0cd5967.status == "expired"` et son `result` contient la sous-chaîne accentuée `terminal — re-armement impossible` ainsi que l'identifiant du parent (L1 + L2a).
2. **Aucun wrapper de remplacement n'a été créé** : `children(620ae345).len() == 1`. C'est l'assertion qui mesure le budget non brûlé.
3. `620ae345.metadata.stuck_rearm_count` reste **absent** — le budget n'est pas consommé contre un parent terminal.
4. Un `audit_events` porte `tool_name = "deferred_wrapper_orphaned_by_terminal_parent"` (compté via `count_audit_events_by_tool_name`, déjà `#[doc(hidden)]` public pour cet usage).

**Rougissement sur `main`, mesuré et non affirmé.** Le test n'appelle que des surfaces existantes (`rearm_consumed_deferred_wrapper`, `get_task`, `count_audit_events_by_tool_name`) : il **compile** sur `main`. Il y rougit sur trois assertions à la fois — le wrapper y finit `delivered` et non `expired`, un wrapper de remplacement y est créé, et `stuck_rearm_count` y passe à 1. C'est un rougissement sur assertion, sans neutralisation de méthode ni protocole de démontage.

#### L4b — `test_replay_2026_09_04_live_blocked_parent_budget_exhausted` — **rejeu construit** (AC1, AC3, AC4)

> **Ce test ne rejoue pas la trace du ticket. Il rejoue son contrefactuel**, et le nom du test comme son commentaire d'en-tête doivent le dire : la même chaîne **sans** le balayage phantom de `02:04:01Z`, c'est-à-dire le parent resté `blocked` jusqu'à l'épuisement du budget. C'est la population B, celle dont Fait 1bis établit qu'elle n'a **aucune instance mesurée**. Le test la couvre parce que AC1 et AC3 l'exigent, pas parce qu'elle s'est produite.

Graine : identique à L4a, sauf `620ae345.status = 'blocked'`, `result` = le JSON verbatim du ticket (`error='global_dispatch_active'`, `blocking_callback_id`, `blocking_task_id`, `dispatch_class`, `deferred_dispatch_registered`), et `metadata.stuck_rearm_count = 2` — budget déjà épuisé, l'état qui force `Unrepairable`.

Acte, puis assertions :

```rust
dispatcher.rearm_consumed_deferred_wrapper(&wrapper, "noop_completion").await;
let parent = db.get_task(PARENT_ID).await.unwrap().unwrap();
assert_eq!(parent.status, "failed");
assert!(parent.result.unwrap().contains("re-armement différé épuisé"));
```

Plus : un `audit_events` porte `tool_name = "deferred_dispatch_unrepairable_parent_failed"`.

**Rougissement sur `main`.** Même propriété que L4a, et pour la même raison : L3a n'ajoute aucune surface, il ajoute un `match` sur la valeur de retour d'un appel qui existe déjà. Sur `main`, `rearm_deferred_callback` rend `Unrepairable`, la valeur est jetée, le parent reste `blocked` — `assert_eq!(parent.status, "failed")` échoue **sur une assertion**, pas sur la compilation.

#### L4c — `test_replay_2026_09_04_stale_blocked_sweep_recovers` — le filet (AC3)

Graine de L4b, mais **sans wrapper consommé** : le wrapper n'a jamais atteint la consommation. Acte : `reap_stale_blocked_dispatch_tasks()`. Assertion : `620ae345.status != "blocked"`, et le motif écrit nomme `stale_blocked_dispatch`.

**Ce test ne compile pas sur `main`** — `reap_stale_blocked_dispatch_tasks` et `find_stale_blocked_dispatch_tasks` sont des surfaces neuves. Il est donc isolé dans son propre `#[tokio::test]`, et il **n'est pas** une pièce de la preuve anti-vacuité : un test qui ne compile pas ne prouve pas de rougissement. Le mélanger à L4a ou L4b détruirait la preuve de ceux-là (`feedback_verify_pipeline_passes_without_the_fix`).

#### Cas accentué, obligatoire et non décoratif

Les motifs que L1, L2a et L3a écrivent dans `tasks.result` sont en français et portent des accents (`re-armement différé épuisé`, `terminal — re-armement impossible`), et la métadonnée du parent porte une branche réelle `fix/2140/auto-pull-la-porte-de-promotion-lit`. L4a et L4b **doivent** asserter sur la sous-chaîne accentuée exacte — `assert!(result.contains("re-armement différé épuisé"))` — et non sur un préfixe ASCII. Deux raisons, toutes deux mesurées dans ce dépôt :

- c'est notre population nominale : nos motifs, nos branches et nos titres de tickets sont accentués (`feedback_notre_entree_nominale_est_accentuee`) ;
- `result` transite par des lecteurs qui tronquent, et la troncature sur frontière d'octet est une classe de panique déjà rencontrée ici (`docs/plans/2103-truncate-output-byte-boundary-panic-and-class-audit.md`). Un motif multi-octets tronqué à 120 octets casse au milieu d'un `é`.

#### Protocole anti-vacuité, à exécuter et à consigner

`git stash` du correctif, `cargo test -p mika-agent test_replay_2026_09_04_terminal_parent` puis `… _live_blocked_parent`, coller **les deux** sorties rouges dans le corps de la PR, restaurer. Les sorties rouges sont des pièces de la PR, pas des affirmations dans le texte (`feedback_verify_pipeline_passes_without_the_fix`). L4c n'entre pas dans ce protocole et le corps de la PR doit dire pourquoi, en une ligne.

### L5 — non-régression : un bloqueur réellement actif refuse toujours (AC5)

Deux tests, tous deux dans `engine.rs` :

1. `test_stale_blocked_sweep_skips_live_blocker()` — même graine que L4b, sauf `74b3ee7d.status = 'in_progress'`. Après le balayage : `620ae345.status == "blocked"`, `stuck_rearm_count` absent, aucun wrapper de remplacement. Le balayage n'a rien touché.
2. `test_stale_blocked_sweep_skips_unexpired_lease()` — même graine que L4b, bloqueur `completed`, mais le bail porte `expires_at = now + 600s`. Même assertion : rien touché. C'est le cas qui protège la sérialisation que mika#2160 tient sous garde opérateur.

Et une assertion de garde sur la porte elle-même : le test existant `executor.rs:5618-5662` (`assert_eq!(v["error"], "global_dispatch_active")`) reste vert sans modification — à vérifier, pas à réécrire. La sérialisation à un `implement` par classe n'est pas ce qu'on retire.

---

## Fire-Disposition

*(F1, bloquant, de la seconde passe architecte — session `9f9d5edc`. Les livrables de classe détecteur qui entrent en service : l'indicateur de famine L2b, les trois rejeux L4a/L4b/L4c, les deux non-régressions L5. Chacun dit ici, à l'avance, ce que signifie son premier tir. Dispositions canoniques de `docs/solutions/best-practices/fire-disposition-doctrine.md`.)*

### L2b — `deferred_dispatch_promotion_starved` : disposition **(c) halt-and-surface, en mode indicateur**

**Ce que le détecteur regarde.** Des wrappers `long_running:run_claude_pilot:deferred` en `status='completed'` depuis plus de `MIKA_DEFERRED_PROMOTION_STALE_SECS` (900 s). Après L1 et L2a, cet état signifie exclusivement « promu, pas encore pris ». Le drapeau `agent_busy` accompagne chaque tir.

**Ce que le premier tir signifie, décidé d'avance.**

| Tir | Sens | Disposition |
|---|---|---|
| `count > 0`, `agent_busy = true` | famine derrière un verrou tenu — le cas mesuré à 2 h 47 le 2026-09-04 | **attendu, bénin.** Aucune action, aucune mutation. C'est la distribution qu'on cherche à connaître. |
| `count > 0`, `agent_busy = false` | wrappers promus avec un agent **libre** — le seul état anormal | **halt-and-surface :** le tir est un avertissement + un événement d'audit, rien de plus. L'action automatique est **explicitement différée** au ticket post-2026-09-11 nommé en § hors portée découvert, qui la décidera sur les chiffres que ce détecteur aura produits. Ce plan ne livre aucun remède sur ce tir. |
| `count = 0` | rien | silence. |

**Le seul tir sur données existantes, nommé.** Avant L1, une consommation stérile laissait un wrapper en `completed` sans jamais le passer `expired`. Des rangées de cette forme peuvent exister dans `mika.db` au moment du déploiement ; comptées par L2b, elles produiraient un tir **permanent**, toutes les 60 s, sans rapport avec une famine. Disposition :

1. Le pilote **mesure** ce résidu avant d'atterrir, sur une copie de la base vivante, avec la requête de L2b sans borne temporelle, et **consigne le compte dans le corps de la PR** (zéro compris).
2. La requête de L2b porte une borne basse, mais **ce n'est pas une date écrite à la main** : c'est l'instant du déploiement, que le code s'auto-installe.

   > **F1 (bloquant) de la seconde passe architecte, session `af938ffc`.** La rédaction précédente figeait `DEFERRED_PROMOTION_EPOCH` au `2026-09-04T00:00:00Z`, le jour du plan. Le jour où `completed` devient exclusif est celui où **L1 tourne en production**, pas celui où le plan est écrit : tout wrapper né entre les deux serait compté comme résidu et produirait un tir parasite permanent. L'architecte a raison, et les deux remèdes qu'il propose sont refusés tous les deux — voir ci-dessous.

   **Le mécanisme.** Au premier démarrage du binaire qui porte L1, l'engine estampille une clé dans la table `schema_meta(key, value)` — celle qui porte déjà les marqueurs one-shot `v27_coalesce_complete` et `well_known_d2_migration_v1` :

   ```sql
   INSERT OR IGNORE INTO schema_meta(key, value)
   VALUES ('deferred_promotion_epoch', strftime('%Y-%m-%dT%H:%M:%SZ','now'));
   ```

   `INSERT OR IGNORE` rend l'écriture idempotente : le premier démarrage après déploiement pose l'instant, tous les suivants ne font rien. La requête de L2b lit la valeur et l'applique en `AND completed_at >= :epoch`. **Aucune DDL, aucun bump de `CURRENT_SCHEMA_VERSION`** — la table existe déjà, on y ajoute une ligne.

   **Pourquoi pas les deux options proposées.**
   - *« Écrire la date de merge au moment du merge »* : c'est un pas manuel dont l'événement qui le défait — un merge un autre jour que le plan — a une probabilité de 1. Un remède manuel dont la demi-vie est plus courte que le délai avant son premier usage n'est pas un remède (`feedback_un_remede_manuel_a_une_demi_vie`).
   - *`env!("DEFERRED_PROMOTION_MERGE_DATE")`* : deux compilations du même commit produiraient deux binaires au comportement différent. Le binaire cesse d'être une fonction de la source, et une sonde qui compare les deux ne peut plus rien conclure. C'est plus coûteux que le défaut qu'on répare.

   **Ce que la borne auto-installée ne couvre pas, et qui est assumé.** Un déploiement, un retour arrière, puis un re-déploiement : l'époque reste celle du premier passage, et le résidu produit pendant la fenêtre de retour arrière sera compté. La fenêtre est étroite, le symptôme est un sur-comptage de l'indicateur — jamais une mutation d'état — et le ticket post-2026-09-11 qui porte le retrait de la borne porte aussi ce cas.

   La ligne `schema_meta` et la clause `completed_at >= :epoch` portent chacune un commentaire nommant la raison, et le ticket post-2026-09-11 qui les retirera quand le résidu antérieur aura été purgé.
3. **Aucune mutation des rangées résiduelles.** Elles ne sont ni expirées ni supprimées par ce plan.

### L4a / L4b / L4c — les trois rejeux : disposition **(c) halt-and-surface**

Aucun des trois n'est alimenté par un balayage de données préexistantes : leurs graines sont posées verbatim ou construites. **Aucun n'a d'arriéré**, donc aucun n'a de tir « sur données existantes » à nommer.

| Test | Rouge **sans** le correctif | Rouge **avec** le correctif |
|---|---|---|
| **L4a** (verbatim, parent terminal) | **attendu** — c'est la pièce anti-vacuité n°1, sa sortie va dans le corps de la PR | **halt-and-surface** |
| **L4b** (construit, parent vivant) | **attendu** — pièce anti-vacuité n°2 | **halt-and-surface** |
| **L4c** (filet L3b) | ne compile pas sur `main` — **ne prouve rien**, et n'entre pas dans le protocole | **halt-and-surface** |

**Halt-and-surface, dit précisément.** Le pilote n'ouvre pas la PR en l'état (ou la passe en brouillon), colle la sortie rouge dans son corps, et s'arrête. Interdits, sans exception : `#[ignore]`, affaiblissement d'une assertion — notamment les sous-chaînes accentuées `re-armement différé épuisé` et `terminal — re-armement impossible` —, et modification d'une graine pour faire passer un test. Un rouge ici dit que L1, L2a ou L3a ne produisent pas la trace promise : **c'est le plan qui doit être corrigé, pas le test.** Ce plan s'est déjà trompé deux fois sur cette trace ; la troisième correction doit passer par la même porte que les deux premières.

### L5 — `test_stale_blocked_sweep_skips_live_blocker`, `test_stale_blocked_sweep_skips_unexpired_lease`, et le test existant `executor.rs:5618-5662` : disposition **(c) halt-and-surface**

Ces trois tests gardent la sérialisation « un `implement` par classe » que mika#2160 tient sous garde opérateur. Un rouge sur l'un d'eux, avec le correctif, signifie que L3b touche un bloqueur vivant ou un bail non expiré — c'est-à-dire que le plan a franchi la ligne qu'il s'interdit. Disposition : **halt-and-surface**, PR non ouverte ou en brouillon avec la sortie rouge, aucun assouplissement de prédicat dans L3b pour « faire passer ». Le test existant `executor.rs:5618-5662` est **vérifié, pas réécrit** ; toute modification de ce test est hors portée de ce ticket.

**Aucune exemption nommée n'est nécessaire** pour les rejeux ni pour les non-régressions ; la seule borne posée est celle de L2b, datée, commentée et rattachée à un ticket qui porte son retrait.

## Séquence

1. **L1** — `mark_deferred_wrapper_noop` (db + async_db + `rearm_consumed_deferred_wrapper`). C'est la fondation : L2a écrit par elle, et L2b dépend de l'exclusivité de `status='completed'` qu'elle établit.
2. **L3a** — le `match` sur `RearmOutcome` dans `rearm_consumed_deferred_wrapper`, et l'écriture de l'échec de budget.
3. **L2a** — la garde du parent terminal, dans la même fonction que L3a. À faire dans la foulée : les deux touchent `rearm_consumed_deferred_wrapper`, et les séparer créerait un conflit inutile.
4. **L2b** — le compteur de famine + l'avertissement. Indépendant, sans mutation d'état.
5. **L3b** — `find_stale_blocked_dispatch_tasks` + `reap_stale_blocked_dispatch_tasks` + le branchement dans `tick()`. Le filet, écrit après que le chemin direct est correct.
6. **L4a puis L4b** — les deux rejeux portants, et la mesure des deux rouges pré-correctif selon le protocole. **L4a d'abord** : c'est celui qui rejoue la trace réelle. **L4c** vient avec L3b.
7. **L5** — les deux non-régressions.

L1 avant L3a et L2a est une dépendance dure. **L2a et L3a sont deux modifications de la même fonction et se font ensemble.** L3b est indépendante et peut être écrite en parallèle ; les rejeux les assemblent et viennent donc après.

> **Deux noyaux, deux statuts de preuve — à ne pas confondre** (F1 de la revue architecte, session `af938ffc`).
>
> - **Noyau de trace : L1 + L2a.** C'est lui, et lui seul, qui ferme l'incident mesuré du 2026-09-04. Fait 1bis établit que les trois tours stériles ont eu lieu contre un parent terminal ; la branche L3a n'y a jamais été atteinte. Retirer L1 ou L2a rouvre AC2 et laisse l'incident sans correctif.
> - **Complément AC : L3a.** Requis par AC1 et AC3, qui exigent explicitement qu'un parent `blocked` dont le budget s'épuise ne reste pas muet. Cette population **n'a aucune instance mesurée** ; sa preuve est une lecture de code, et son test est un rejeu construit (L4b). Il n'est pas optionnel pour autant : c'est un AC, pas une intuition. Mais il ne doit pas être présenté comme ce qui ferme la trace.
>
> **Si la portée doit être réduite en revue**, l'ordre de coupe est : L3b, puis L2b — un filet et un indicateur, ni l'un ni l'autre ne fermant la trace. Le noyau de trace et le complément AC ne se coupent pas.

---

## Risques et ce qui les borne

| Risque | Ce qui le borne |
|---|---|
| **Fauchage de travail vivant** : un chien de garde marque `expired` un wrapper qui allait être livré. | **C'est le risque que la re-mesure a matérialisé**, et il est écarté par construction : L2b ne mute aucun état, et L2a n'agit que sur un parent déjà terminal — un état dont on ne revient pas. Aucun livrable ne détruit un wrapper sur un critère de délai. |
| **Double écriture d'échec** : L3a échoue le parent, puis L3b le retrouve et l'échoue encore. | L3b porte `parent.status = 'blocked'` dans sa population ; un parent que L3a vient de passer `failed` en sort. Les deux chemins sont disjoints par le statut, pas par l'ordonnancement — c'est ce qui les rend sûrs indépendamment du tick. L4a le vérifie sur le parent terminal (aucune seconde écriture), et un test de L5 sur le bloqueur vivant. |
| **Double re-armement** : L2a/L3a et L3b réparent le même parent au même tick. | `rearm_deferred_callback` interroge `has_non_deferred_active_callback_child` en premier et rend `NotNow` si un dispatch existe ; le budget partagé `MAX_STUCK_REARMS = 2` borne le total quel que soit le chemin — c'est écrit dans sa doc et c'est le contrat qu'on hérite. L4a le vérifie par son assertion 2 (un seul wrapper enfant, pas deux). |
| **Re-armement contre une porte opérateur** : L3b relance un dispatch que Vincent avait délibérément bloqué. | Le prédicat `$.error = 'global_dispatch_active'` ; un `blocked` d'auto-merge ou d'escalade QA ne porte pas ce `result`. Vérifié : les quatre seuls sites de production qui écrivent `blocked` sont `server/verdict_handler.rs:999/1289/1496/2160`, et la porte forge-gate est `server/ci_success_handler.rs:848`. Aucun ne porte `global_dispatch_active` dans `result`. Contrôle négatif mesuré : `662d9752` porte `unauthorized_webhook_dispatch` et est exclue. |
| **Cycle `blocked → pending → blocked`** : le LLM re-bloque le parent que L3b vient de rendre `pending`. | Le budget `MAX_STUCK_REARMS` s'applique au parent, pas au cycle : au troisième passage, `Unrepairable` → `update_task_failed` avec raison. Le cycle est borné à deux tours et se termine **visiblement**. |
| **`expired` mal interprété ailleurs** : un lecteur compte `expired` comme un échec de pilote. | **Résolu à la lecture, plus « à confirmer ».** Le seul écrivain de production est `mark_tasks_expired` (`db.rs:6896`), gardé par `timeout_at IS NOT NULL` et `status NOT IN (…,'delivered')` — il ne peut pas toucher un wrapper marqué par L1. Le seul lecteur de production qui range `expired` avec les échecs est le garde anti-zombie de `create_recurring_task_if_absent` (`db.rs:5761`), filtré sur `trigger_type = 'recurring'` : un wrapper différé est `trigger_type = 'callback'` et lui est invisible. Aucun ajustement de lecteur n'est requis. |
| **Le verrou d'agent reste affamé** et aucun re-armement ne produit de dispatch. | Assumé et déclaré (§ hors portée). AC1 se satisfait alors par sa seconde branche — échec visible portant une raison — désormais réellement écrite par L3a. Le ticket séparé sur `800d739f` porte la cause, et L2b en fournira la mesure. |
| **La révision du plan se trompe à son tour** — ce qui est arrivé **deux fois** : la première rédaction sur `f0cd5967` (un `completed` de transit lu comme terminal), la seconde sur l'horodatage du fauchage (`03:59:50Z` au lieu de `02:04:01Z`), qui masquait que les trois tours ont eu lieu contre un parent déjà mort. | Les faits portants de cette troisième version sont des lignes d'`audit_events` — des transitions horodatées, pas des états lus à un instant — plus une lecture de code (`RearmOutcome` jeté). La règle tirée des deux erreurs : **ne pas conclure sur un `status` sans lire la transition qui l'a écrit** (`feedback_never_conclude_inside_the_mechanism`). Le protocole anti-vacuité est le contrôle : si la lecture est encore fausse, L4a ou L4b rougit **aussi** avec le correctif, et la disposition est halt-and-surface. |

---

## Critères d'acceptation — correspondance

| AC | Livrable | Comment on le mesure |
|---|---|---|
| **AC1** — un `ready-label` refusé dont la fente se libère produit un dispatch réel **ou** un échec visible portant une raison ; jamais `blocked` en silence. | L2a (population A, mesurée) + **L3a** (population B, lue à la ligne) + L3b (filet) | L4a : le tour contre un parent terminal écrit `expired` + motif au lieu de `delivered`, et ne brûle pas le budget. L4b : le budget épuisé contre un parent **vivant** écrit `failed` + `re-armement différé épuisé`. Un `audit_events` dans les deux. **Mesuré aujourd'hui : aucune des deux branches n'existait.** |
| **AC2** — le succès du re-tir se mesure sur son effet, pas sur le réveil ; une reprise qui n'a rien dispatché ne se termine pas `completed`. | **L1 + L2a** | L1 : toute consommation stérile écrit `expired` + raison, au lieu de `delivered` + `deferred dispatch slot freed`. L2a : le parent terminal sort par une branche nommée au lieu d'un re-armement voué à l'échec. L4a mesure ça sur la trace verbatim — la seule des deux populations qui s'est réellement produite. |
| **AC3** — le trio « bloqueur terminé + bail expiré + parent bloqué » n'est pas un état stable. | L3a + L3b | L4b (le trio complet en graine : bloqueur `completed`, bail `expires 23:31:44Z`, parent `blocked`) et L4c (le filet, quand le wrapper n'a jamais été consommé). Deux sorties disjointes par statut du parent, donc sûres quel que soit l'ordonnancement des ticks. |
| **AC4** — anti-vacuité : le rejeu rougit sans le correctif. | L4a + L4b | Les deux compilent sur `main` et y rougissent **sur assertion** — pas de neutralisation de méthode. Protocole `git stash`, les deux sorties rouges collées dans le corps de la PR. Cas accentué obligatoire. L4c est explicitement exclu du protocole, et le corps de la PR dit pourquoi. |
| **AC5** — un bloqueur réellement actif continue de refuser le second dispatch. | L5 | Deux tests (bloqueur vivant / bail non expiré) : le balayage ne touche rien. Plus la vérification que `executor.rs:5618-5662` reste vert **sans modification**. |

> **Ce que la correspondance ne prétend pas.** Aucun livrable de ce plan ne raccourcit les 80 minutes de silence du 2026-09-04 : leur cause est la famine de la file de livraison (Fait 1bis), portée par un ticket séparé en Tier 1. Ce que les AC ci-dessus mesurent, c'est qu'un silence de cette forme laisse désormais une trace lisible et bornée derrière lui, au lieu de trois `delivered` et d'un budget brûlé sans témoin.

---

## Acceptance criteria

- [ ] **AC1** — un `ready-label` refusé dont la fente se libère produit un dispatch réel **ou** un échec visible portant une raison ; jamais `blocked` en silence.
- [ ] **AC2** — le succès du re-tir se mesure sur son effet, pas sur le réveil ; une reprise qui n'a rien dispatché ne se termine pas `completed`.
- [ ] **AC3** — le trio « bloqueur terminé + bail expiré + parent bloqué » n'est pas un état stable.
- [ ] **AC4** — anti-vacuité : le rejeu rougit sans le correctif.
- [ ] **AC5** — un bloqueur réellement actif continue de refuser le second dispatch.

---

## Vérification

```bash
cargo test -p mika-agent test_replay_2026_09_04          # L4a/L4b/L4c — les trois rejeux (AC1/AC2/AC3/AC4)
cargo test -p mika-agent test_stale_blocked_sweep        # L5 — les deux non-régressions (AC5)
cargo test -p mika-agent rearm                           # L1/L2a/L3a — la famille rearm_* existante
cargo test -p mika-agent deferred                        # toute la famille existante mika#1011/#1124/#2045
cargo test -p mika-agent global_dispatch_active          # la porte que l'on ne retire pas (AC5)
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

Plus la mesure pré-correctif de **L4a et L4b** (`git stash` → tests → deux sorties rouges → restauration), dont les sorties sont des **pièces de la PR** et non des affirmations dans le corps du texte. L4c en est exclu : il ne compile pas sur `main`, et le corps de la PR doit le dire en une ligne.

> **Garde de non-régression sur la famille existante.** `cargo test -p mika-agent rearm` et `… deferred` doivent être verts **avant** le correctif et **après**. `rearm_consumed_deferred_wrapper` est appelée par deux sites (`silent_turn_error`, `noop_completion`) : y ajouter une garde de parent terminal et un `match` change le comportement des deux. Si un test existant de cette famille rougit après le correctif, c'est un signal à lire — pas à ajuster.

---

## Fichiers touchés

| Fichier | Nature |
|---|---|
| `crates/mika-agent/src/db.rs` | +3 méthodes (`mark_deferred_wrapper_noop`, `count_promoted_undelivered_wrappers`, `find_stale_blocked_dispatch_tasks`) + 1 struct de rendu |
| `crates/mika-agent/src/async_db.rs` | +3 passe-plats |
| `crates/mika-agent/src/task_engine/dispatcher.rs` | **Le cœur.** `rearm_consumed_deferred_wrapper` : garde du parent terminal (L2a), `match` sur `RearmOutcome` (L3a), enregistrement terminal du wrapper (L1) |
| `crates/mika-agent/src/task_engine/engine.rs` | +1 balayage (L3b), +1 compteur de famine (L2b), +1 lecteur d'env (`MIKA_DEFERRED_PROMOTION_STALE_SECS`), +1 estampille `schema_meta` idempotente au démarrage, 2 lignes dans `tick()`, **+5 tests** (L4a, L4b, L4c, 2× L5) |

Aucune migration de schéma : `expired` est déjà dans la contrainte `CHECK` (`db.rs:1342-1343`), `dispatch_slot_leases` existe depuis v51, et l'époque de L2b est une **ligne** dans `schema_meta` — pas une DDL, donc pas de bump de `CURRENT_SCHEMA_VERSION` (51).

**Où le poids s'est déplacé.** La première rédaction mettait l'essentiel dans `engine.rs` — deux balayages neufs pilotés par des fenêtres de temps. La re-mesure l'a déplacé dans `dispatcher.rs`, sur trois modifications d'une seule fonction existante, dont la principale est de **consommer une valeur de retour que le code jetait**. C'est un correctif plus petit, plus près de la cause, et dont l'anti-vacuité se mesure sans démonter quoi que ce soit.
