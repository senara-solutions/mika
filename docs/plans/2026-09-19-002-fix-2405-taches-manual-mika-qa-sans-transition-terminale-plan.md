# Plan — mika#2405 : une row `manual` ouverte par un dispatch n'a pas de fermeur

- **Ticket :** senara-solutions/mika#2405
- **Type :** fix (substrat moteur)
- **Date :** 2026-09-19
- **Branche :** `feat/2405/t-ches-mika-qa-trigger-manual-du-chemin`

---

## Goal Capsule

Une row `tasks` `trigger_type='manual'` ouverte **mécaniquement** par la
Delegation Rule avant un dispatch long-running n'a, chez mika-qa, aucun
mécanisme qui la referme. Ce plan identifie le producteur à la ligne, tranche
H1/H2 sur preuve, et pose le fermeur manquant : un réconciliateur
parent↔enfant qui clôt un parent `manual` dont **tous** les enfants callback
sont terminaux, en réutilisant la garde de vivacité existante.

**Ce que le plan rectifie du ticket, et c'est le premier livrable.** Le ticket
cherche le producteur dans « le chemin auto-QA-verdict-inline (#2359) /
auto-QA (#2372) ». Il n'y est pas. Le producteur est la **Delegation Rule
générique du system prompt**, qui s'applique à tout agent dispatchant un outil
long-running — mika-qa n'est pas un cas particulier, c'est le seul agent dont
la population n'a pas de faucheur. Et H2 est réfutable par lecture : le
réconciliateur nommé ne lit jamais la table `tasks`.

---

## Product Contract

### AC1 — Le producteur, à la ligne

Chaîne à quatre maillons, tous vérifiés dans l'arbre :

| # | Site | Ce qu'il pose |
|---|---|---|
| 1 | `crates/mika-agent/src/prompt.rs:1709-1711` | **Delegation Rule** : « Before delegating any implementation work (via `delegate_task` **or long-running skills**), you MUST first create a task using `create_task`, then pass the task_id to the delegation tool. » |
| 2 | `crates/mika-agent/src/tools/create_task.rs:258-280` | La forme : `trigger_type='manual'`, `action_type='none'`, `timeout_at: None`, `process_id` absent (NULL), `source` = ce que le modèle passe (**rien**, hors chemins self-dev), `reference_url` optionnel |
| 3 | `crates/mika-agent/src/skills/executor.rs:3234-3247` | `mark_parent_dispatched(task_id)` — auto-transition `pending → in_progress` **et** stamp `fired_at`, un seul acte |
| 4 | `crates/mika-agent/src/skills/executor.rs:3251-3256` | `parent_task_id` de la row callback = le `task_id` passé par le modèle |

`build_mika` est déclaré `long_running` et passe donc par
`execute_long_running` (`executor.rs:3116`). La chaîne de processus relevée par
l'opérateur dans le commentaire 1/2 (`rustc ← cargo ← run.sh (build-mika
handler) ← mika-spirit`) est l'exécution du maillon 4.

**Il existe deux autres producteurs de rows `manual` en production, et tous
deux sont hors population.** `tools/create_task.rs:346` est le retry du même
outil après une course de dédup — même forme, même trou.
`server/ready_label_handler.rs:529-553` pré-crée sa row de suivi avec
`source: Some("self_dev")` et `type: Some("issue")`, ce qui la place
précisément dans le champ des reapers #871/#1162/#1687. C'est la différence
qui explique pourquoi le défaut se voit chez mika-qa et pas chez mika-dev :
**ce n'est pas l'agent qui compte, c'est le `source`.**

**Les deux sous-formes du ticket ont une seule cause, séparées par une seule
branche** — `if wi_status == "pending"` (`executor.rs:3234`) :

- `fired_at` non-NULL + `updated > created` (f41f7cb4, `fired_at` = created+3 s) :
  la row était `pending` au dispatch, `mark_parent_dispatched` l'a passée
  `in_progress` **et** stampée.
- `fired_at` NULL + `updated ≈ created` (88df0ff5 +2 s, f859ac68 +3 s) : le
  modèle l'avait déjà passée `in_progress` lui-même via `update_task_status`
  juste après création, la branche n'a pas tourné, donc pas de stamp.
  `update_manual_task_status` (`db.rs:6894-6924`) ne touche jamais `fired_at`.

Le ticket présentait ces deux formes comme deux énigmes. Elles n'en font
qu'une.

### AC2 — H1 vs H2, tranchées sur preuve

**H1 — vraie, mais relocalisée.** Il y a bien création sans clôture. Mais le
producteur n'est pas un chemin de code propre au verdict QA : c'est la règle
générique ci-dessus. Conséquence qui change le remède : **aucun chemin de code
ne « sait » qu'il doit clore cette row au retour du verdict**, parce qu'aucun
chemin de code ne l'a ouverte — c'est le modèle qui l'a ouverte, sur
injonction du prompt.

L'asymétrie est dans le prompt lui-même : l'**ouverture** est prescrite
inconditionnellement (`prompt.rs:1710`, « you MUST »), la **clôture** est
prescrite conditionnellement à une demande de l'utilisateur
(`prompt.rs:1697-1699`, « **Direct update:** When the user explicitly requests
a status change »). Un tour de callback de build QA n'a pas d'utilisateur qui
demande.

Trois textes pèsent sur ce tour, et aucun ne ramène à la clôture :

- `skills/bundled/qa-review-build-callback/system_prompt.md:11` — « It does
  **not** require `update_task_status` or `send_message` on this turn ».
- `crates/mika-agent/src/qa_build_callback.rs:178` — le re-prompt de la garde
  mika#2355 dit « Do not call `update_task_status` or `send_message`
  **instead** ». **Le mot « instead » est portant et il faut résister à la
  tentation de le lire comme une interdiction** : il refuse la *substitution*
  d'une clôture à une revue postée, pas la clôture elle-même. Ce texte n'est
  d'ailleurs servi que dans la branche où la garde tire.
- `crates/mika-agent/src/tools/update_task_status.rs:153` — l'outil est
  verrouillé sur `trigger_type == "manual"`, donc c'est bien le seul outil qui
  *pourrait* fermer cette row, et rien ne l'appelle.

Bilan : la clôture n'est **interdite** nulle part ; elle n'est **exigée** nulle
part non plus, et le seul texte qui la nomme sur ce chemin la nomme pour
l'écarter d'un autre usage. Un remède par prompt aurait donc à la fois à
ajouter une exigence et à désambiguïser un texte existant — deux raisons de
plus de ne pas le tenter (voir le refus n°2).

**H2 — fausse, et pas par un réglage de prédicat.** `qa_review_reconcile` ne
lit pas la table `tasks`. Sa source est un subprocess `gh pr list`
(`qa_review_reconcile.rs:643`), sa population est filtrée en mémoire sur ces
snapshots (`qa_review_reconcile.rs:275`), son action est
`gh pr edit --add-reviewer` (`qa_review_reconcile.rs:819`), et ses seuls accès
DB portent sur `audit_events` (`:539`, `:839`, `:882`). Une row `tasks` lui est
**structurellement invisible**. Le scan est bien enregistré sur mika-dev
(`server/mod.rs:1685-1703`), mais corriger ce scope ne changerait rien :
l'élargir à mika-qa lui ferait lister les PRs vues par mika-qa, pas fermer des
tâches. **H2 est un faux filet, et le croire disponible est le risque que cette
section ferme.**

**H3 — non listée au ticket, et c'est elle qui décide du remède.** Sur les
seize mécanismes qui transitionnent une row `tasks` vers un état terminal — les
treize reapers/sweepers périodiques plus **trois backstops inline** du
dispatcher que le ticket ne mentionne pas —, un seul voit cette forme : le
**phantom sweep** (mika#1712/#2156, `db.rs:8057-8072`), dont le prédicat est
`action_type='none' AND process_id IS NULL AND status IN ('in_progress','blocked')`
— sans filtre sur `trigger_type`, sur `source` ni sur l'agent. Il couvre donc
la forme produite par `create_task`. Les douze autres l'excluent, chacun par un
terme nommé :

| Mécanisme | Terme excluant |
|---|---|
| `mark_tasks_expired` (`db.rs:7624`) | `timeout_at IS NOT NULL` — `create_task` pose `timeout_at: None` |
| kill orphan processes (`db.rs:9009`) | `status='expired'` ∧ `process_id IS NOT NULL` |
| callback watchdog #959 (`db.rs:9036`) | `trigger_type='callback'` ∧ `process_id IS NOT NULL` |
| pilot silent-stall #2249 (`db.rs:9080`) | idem |
| orphaned-parent #871 (`db.rs:7972`) | `parent.source='self_dev'` |
| completer pr_url #1162 (`db.rs:8394`) | `parent.source='self_dev'` ∧ `pr_url IS NOT NULL` |
| childless-parent #1687 (`db.rs:8461`) | `source='self_dev'` ∧ `type='issue'` |
| stuck-pending #2045 (`db.rs:8578`) | `status='pending'` ∧ `source='self_dev'` ∧ `type='issue'` ∧ `reference_url IS NOT NULL` |
| stale-blocked #2169 (`db.rs:8665`) | `status='blocked'` ∧ `result.error='global_dispatch_active'` |
| `a2a_sweep_orphans` #2379 (`a2a_db.rs:315`) | `trigger_type='a2a'` |
| `TurnGuard` #2379 (`a2a_db.rs:291`) | la row doit figurer dans `a2a_task_map` |
| startup recovery (`engine.rs:449-458`) | garde explicite `trigger_type == MANUAL → continue` |
| **inline** `try_promote_parent_on_retry_success` (`dispatcher.rs:3152-3161`) | `source == Some("self_dev")` ∧ `status == "failed"` |
| **inline** `try_complete_parent_on_callback_success` (`dispatcher.rs:3254-3265`) | `source == Some("self_dev")` ∧ `dispatch_class == "implement"` ∧ `pr_url` |
| **inline** `try_resolve_parent_on_dispatch_refusal` (`dispatcher.rs:3431-3439`) | `source == Some("self_dev")` |

Les trois backstops inline sont le voisinage le plus proche du remède : ils
agissent **au retour du callback**, pas sur un tick. Ils sont exclus par le
même terme unique — `source == Some("self_dev")` — que les reapers #871/#1162.

### AC3 — La transition terminale, et pourquoi le filet existant ne suffit pas

Le phantom sweep **ramasse la forme, mais il la ramasse mal et tard** — et
c'est vrai indépendamment de la mesure M1 ci-dessous :

1. **Mal :** il écrit `failed` (`engine.rs:2473`+). Une revue QA menée à
   verdict a réussi ; marquer sa row de suivi `failed` est une affirmation
   fausse, qui pollue ensuite toute lecture d'un taux d'échec.
2. **Tard :** grâce par défaut `MIKA_PHANTOM_SWEEP_AGE_SECONDS = 14400` (4 h).
   Une row de corrélation de dispatch reste ouverte 4 h après la fin de son
   dispatch.
3. **Conditionnellement :** son prédicat porte sur `action_type='none'`. Toute
   row `manual` portant un `action_type` réel n'a **aucun** faucheur.

Le fermeur posé par ce plan clôt la row **au bon moment** (quand son enfant
callback devient terminal), **avec le bon mot** (`completed` — le parent a
rempli sa fonction de suivi), et **sans dépendre de `action_type`**.

### AC5 — Déjà satisfait par l'existant, et c'est un refus d'implémenter

Le ticket demande « pgrep + mtime avant tout balayage ». C'est exactement ce
que fait `dispatch_liveness` (mika#2156) : pid **et** `process_start_time`,
jamais une existence nue de `/proc/<pid>` où un PID recyclé serait
indistinguable. Le fermeur **réutilise** cette fonction. Réimplémenter un
`pgrep` serait un second lecteur de la même question — la classe que
`grooming_marker` (mika#2158) et `live_pilot` (mika#2279) ont déjà dû fermer
deux fois dans ce dépôt.

---

## Planning Contract

### Mesure M1 — bloquante, et elle précède le choix de réglage

Le ticket observe les trois rows entre **07:02 et 07:52**, dans un ticket écrit
à **08:14**. Elles avaient donc **moins d'une heure d'âge**, contre une fenêtre
de grâce de **4 h**. Au moment de l'observation, aucune n'était hors fenêtre :
**la preuve dure du ticket ne distingue pas une fuite d'une latence.** Et le
tableau du ticket ne porte pas la colonne `action_type`, qui est le seul terme
du prédicat phantom en jeu.

À relever avant de figer les réglages (la policy de cette session refuse
`sqlite3`, donc la mesure est à faire côté opérateur ou depuis un test
d'intégration) :

```sql
SELECT id, action_type, source, type, status, process_id,
       created_at, updated_at, fired_at,
       (SELECT COUNT(*) FROM tasks c WHERE c.parent_task_id = t.id)        AS children,
       (SELECT COUNT(*) FROM tasks c WHERE c.parent_task_id = t.id
          AND c.status NOT IN ('delivered','failed','cancelled','expired')) AS children_live
  FROM tasks t
 WHERE agent_id = 'mika-qa' AND trigger_type = 'manual'
   AND status IN ('in_progress','blocked')
 ORDER BY created_at DESC;
```

**Trois haltes, chacune change le travail :**

- **`action_type` ≠ `'none'`** → le phantom sweep ne les voit pas du tout, la
  population n'a strictement aucun faucheur, et U2–U5 sont le seul remède.
  Vérifier alors quel site pose cet `action_type`, car ce n'est pas
  `create_task.rs:270`.
- **`action_type = 'none'` et les rows > 4 h ont disparu** → le ticket décrit
  une **latence de 4 h avec un mauvais verdict**, pas une fuite. U2–U5 restent
  justifiées par les points 1 et 2 de l'AC3, mais **ne pas toucher
  `MIKA_PHANTOM_SWEEP_AGE_SECONDS`** : sa valeur est motivée par mika#2156 et
  son blast radius est mika-dev.
- **`action_type = 'none'` et des rows > 4 h subsistent** → un terme du
  prédicat phantom est raté ou la garde de vivacité les épargne à tort.
  **Halte : établir lequel avant d'écrire une ligne du fermeur** — un second
  faucheur posé par-dessus un premier qui échoue silencieusement masquerait la
  cause au lieu de la traiter.

### Décision de site : un scan, pas neuf interceptions

Le moment *juste* pour fermer le parent est l'instant où son enfant devient
terminal — c'est ce que font les trois backstops inline de `dispatcher.rs`.
Mais l'enfant a **neuf** chemins terminaux distincts : `executor.rs:3373`,
`:3409`, `:3509`, `:3551`, `server/handlers.rs:626`, le watchdog PID
(`engine.rs:1699`), le reaper de stall (`engine.rs:2187`), `mark_tasks_expired`
(`db.rs:7624`) et `tracking_cleanup.rs:299`. Les instrumenter tous, c'est neuf
sites d'écriture à tenir synchronisés pour une seule question — exactement la
duplication de prédicat que mika#2158 (`grooming_marker`) et mika#2279
(`live_pilot`) ont déjà dû refermer dans ce dépôt.

Un scan périodique pose **un** lecteur pour cette question, ne touche aucun
chemin existant, et couvre par construction les neuf sorties — y compris celles
qu'un futur chemin terminal ajouterait sans que personne y pense. Le prix est
nommé : jusqu'à une passe de retard (grâce de 600 s), ce qui est sans
conséquence sur une row dont la seule fonction est la corrélation.

### Refus argumentés

**1. Élargir `parent.source = 'self_dev'` dans la paire orphan/completable ou
dans les trois backstops inline.**
C'est le remède qui saute aux yeux — un seul terme à retirer, à six endroits —
et il est faux. Cette paire (`db.rs:7972` / `db.rs:8394`) est **taillée pour le
contrat self_dev** : son discriminant terminal est la présence de
`$.claude_pilot.pr_url` — `pr_url` absent ⇒ `failed`, présent ⇒ `completed`.
Une revue QA ne produit **jamais** de `pr_url`. Retirer le filtre `source`
enverrait donc **tout** le trafic QA sur la branche `failed`. Le même
raisonnement vaut pour `try_complete_parent_on_callback_success`, dont l'étape
4 court-circuite sur l'absence de ligne `PR:`. Le doc-comment de `db.rs:7967-7972` l'inscrit d'ailleurs
en garde : « Any filter change here (agent_id, status, source, trigger_type,
dispatch_class, sibling guard, grace window) MUST be applied symmetriquement
there » — l'invariant nomme la symétrie entre les deux requêtes, pas une
licence d'élargir leur population.

**2. Ajouter une phrase « clos ta tâche » au prompt.** Interdit par
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
(mika#2120 : neuf récurrences sous enforcement par prompt contre zéro quand
l'opérateur l'écrivait à la main). Et le défaut est ici *déjà* une règle de
prompt qui ne tient pas : en ajouter une seconde pour rattraper la première est
la définition du mode d'échec mesuré.

**3. Retirer la Delegation Rule ou faire poser un `source` par mika-qa.**
Hors périmètre et à blast radius large : la règle sert la corrélation des logs
pilote (`/var/log/claude-pilot/{uuid}.log`) sur toute la flotte, et `source`
est lu par cinq prédicats de reaper. Changer l'un ou l'autre pour fermer une
row de suivi déplacerait le risque sur mika-dev.

**4. Toucher `qa_review_reconcile`.** Voir AC2 : il ne lit pas `tasks`.

---

## Implementation Units

### U1 — La requête du fermeur (`db.rs`)

`find_settleable_dispatch_parents(agent_id, grace_seconds)` — nouveau, à poser
à côté de la paire `db.rs:7972` / `db.rs:8394`, avec un doc-comment qui nomme
explicitement **pourquoi ce n'est pas un troisième membre de cette paire**
(pas de prédicat `pr_url`, pas de prédicat `source`, verdict unique).

```sql
SELECT parent.id, parent.agent_id, parent.created_at,
       MAX(child.updated_at) AS last_child_at
  FROM tasks parent
  JOIN tasks child ON parent.id = child.parent_task_id
 WHERE parent.agent_id = ?1
   AND parent.status = 'in_progress'
   AND parent.trigger_type = 'manual'
   AND COALESCE(parent.source, '') != 'self_dev'
   AND child.trigger_type = 'callback'
   AND child.action_type  = 'resume_agent'
   AND NOT EXISTS (
         SELECT 1 FROM tasks sibling
          WHERE sibling.parent_task_id = parent.id
            AND sibling.status IN ('pending','in_progress','completed','blocked'))
 GROUP BY parent.id
HAVING MAX(child.updated_at) < strftime('%Y-%m-%dT%H:%M:%SZ','now', ?2)
 ORDER BY parent.id
```

La fenêtre de grâce est en `HAVING`, pas en `WHERE` : elle porte sur un agrégat
et SQLite refuse une fonction d'agrégation dans la clause `WHERE`. Les deux
requêtes voisines (`db.rs:7972`, `db.rs:8394`) posent leur grâce en `WHERE`
parce qu'elle y porte sur `child.updated_at` ligne à ligne ; ici le prédicat
est sur le **dernier** enfant, ce qui est un agrégat. C'est la seule différence
de forme avec elles, et elle est intentionnelle.

Le `NOT EXISTS` ne s'auto-exclut volontairement pas (pas de
`sibling.id != child.id`, contrairement aux deux requêtes voisines) : la liste
de statuts ne contient aucun état terminal, donc un `child` terminal ne peut
pas se compter lui-même, et un `child` non terminal *doit* exclure son parent.
Le garde s'écrit ainsi en un terme au lieu de deux.

Quatre termes portants, et chacun se justifie séparément :

- `COALESCE(parent.source,'') != 'self_dev'` — **complément**, jamais
  recouvrement. La population self_dev garde ses reapers #871/#1162, dont le
  verdict est plus riche. Le `COALESCE` est requis : `source` est NULL sur la
  population visée et `NULL != 'self_dev'` rend NULL en SQL, donc sans lui la
  requête ne rend **rien** — l'échec silencieux que ce plan existe pour éviter.
- `NOT EXISTS (… sibling.status IN ('pending','in_progress','completed','blocked'))`
  — **`completed` est dans la liste, délibérément.** Sur une row callback,
  `completed` signifie « le pilote est revenu, la livraison n'a pas encore eu
  lieu » ; `delivered` est l'état terminal. Fermer un parent dont un enfant est
  `completed` fermerait la row avant que son tour de verdict ait tourné. Même
  vocabulaire que la quarantaine mika#2179.
- Fenêtre de grâce sur `MAX(child.updated_at)` — le temps depuis que le
  **dernier** enfant a bougé, jamais depuis la création du parent : un parent
  réutilisé (motif mika#920) est ancien par construction.
- Aucun prédicat sur `action_type` du parent, ni sur `type`, ni sur
  `reference_url` : ce sont les trois termes qui excluent la population des
  reapers existants.

**Fail-safe :** un parent sans aucun enfant callback est hors population (le
`JOIN` l'exclut). C'est volontaire — un dispatch refusé avant création de
l'enfant (`dispatch_limit_exceeded`, `executor.rs:3216`) laisse une row sans
enfant, qui n'a **pas** reçu de dispatch et dont ce fermeur n'a rien à dire.
Population résiduelle nommée, couverte par le phantom sweep tant que
`action_type='none'`.

### U2 — Le fermeur (`task_engine/engine.rs`)

`settle_dispatch_parents()`, câblé dans le scan `DB_SCAN_INTERVAL_TICKS`
(`engine.rs:600-670`), **après** les reapers #871/#1162 pour que la population
self_dev soit déjà traitée quand il passe.

Par row :

1. `dispatch_liveness(&row.id)` — réutilisée telle quelle (AC5). `Live` →
   épargner, compter `spared`, **ne rien écrire** ; `Unreadable` → épargner
   aussi (un signal qu'on ne peut pas lire n'est jamais un terme satisfait —
   même règle que mika#2277 et mika#2279).
2. `update_task_status(&row.id, COMPLETED)`.
3. `log_audit_event` avec `tool_name = 'dispatch_parent_settled'` — **sole
   writer**, à épingler par un scan de source comme `phantom_aged_out` et
   `qa_callback_verdict`. L'absence de cette ligne sous un symptôme est alors
   elle-même une information.

**`completed` et non `failed` :** le verdict porte sur la row, qui est un
jeton de corrélation exigé par la Delegation Rule — sa fonction est remplie dès
que son dispatch est revenu. Le succès ou l'échec du *build* est porté par
l'enfant et par la revue postée, jamais par ce jeton. `failed` y affirmerait un
échec que rien n'établit, et c'est le défaut n°1 du filet phantom (AC3).

### U3 — Réglages

| Variable | Défaut | Contrat |
|---|---|---|
| `MIKA_DISPATCH_PARENT_SETTLE_ENABLED` | `true` | Kill-switch. `0`/`false`/`off`/`no` désarment ; valeur non reconnue → **armé + WARN nommant la valeur entre guillemets** (un désarmement par coquille sur un fermeur serait la panne silencieuse que ce plan ferme). |
| `MIKA_DISPATCH_PARENT_SETTLE_GRACE_SECS` | `600` | Trois paliers maison : absent/vide → défaut ; illisible, `0` ou négatif → défaut + WARN. Aligné sur la grâce des reapers #871/#1162 (`db.rs:7972`, 600 s), qui bornent la **même** transition parent↔enfant — deux grammaires de grâce pour une même question serait une dette de lecture. Clamp haut à 30 jours, même raison que `MIKA_PROMOTED_WRAPPER_LIVENESS_SECS` : SQLite rend NULL sur un modificateur `strftime` hors domaine, donc un override absurde désarmerait la fenêtre en silence. |

### U4 — Surfaces opérateur

Journal (`$MIKA_SPIRIT_LOG_FILE`) :

- `dispatch_parent_settled` (INFO, une ligne par row — champs `task_id`,
  `agent_id`, `child_count`, `idle_secs`). **Régime attendu : environ une par
  dispatch long-running non-self_dev.** Un volume très supérieur signifie qu'un
  producteur ouvre des rows que rien ne dispatche.
- `dispatch_parent_settle_spared` (INFO — `task_id`, `child_task_id`, `pid`,
  `reason` ∈ `{live, unreadable}`). **Sans cette ligne, un fermeur qui épargne
  se lit exactement comme un fermeur oisif** (mika#2205). `reason = unreadable`
  soutenu est le seul état qui ramène la population au comportement
  d'avant-correctif.
- `dispatch_parent_settle_complete` (INFO par passe, **émis seulement quand la
  passe agit** — zéro action, zéro ligne, doctrine mika#2131).

SQL :

```sql
SELECT COUNT(*) FROM audit_events WHERE tool_name = 'dispatch_parent_settled';
SELECT COUNT(*) FROM audit_events WHERE tool_name = 'phantom_aged_out';
```

Le second est le **contrôle négatif** : il doit **décroître** sur mika-qa après
déploiement. S'il ne décroît pas alors que le premier est non nul, deux
faucheurs se disputent la même population et il faut établir lequel arrive
premier avant de régler quoi que ce soit.

### U5 — Tests

1. **Comportemental, AC4** — un parent `manual` `in_progress` + un enfant
   callback `delivered` au-delà de la grâce, `source` NULL : une passe le rend
   `completed` et écrit la ligne d'audit. **Contrôle négatif** dans le même
   test : avec `source='self_dev'`, la row est intacte (c'est le refus n°1 qui
   est épinglé, pas seulement la fonctionnalité).
2. **Vivacité, AC5** — enfant avec pid vivant et `process_start_time` lisible :
   row intacte, `spared` écrit. Second cas : `process_start_time` illisible →
   row intacte aussi (fail-safe `Unreadable`).
3. **Sibling `completed`** — un enfant `delivered` et un sibling `completed` :
   row intacte. C'est le terme le plus facile à « simplifier » par erreur en
   relecture.
4. **Structurel** — `settle_dispatch_parents` est le seul écrivain de
   `dispatch_parent_settled` (scan de source). Un test comportemental ne peut
   pas voir un second écrivain : il ne rendrait aucune décision fausse, il
   rendrait l'attribution muette.
5. **Non-régression self_dev** — les fixtures existantes de #871/#1162 passent
   inchangées, et une row self_dev sans `pr_url` continue d'être `failed` par
   son reaper d'origine, pas `completed` par le nouveau.

---

## Verification Contract

- `cargo test -p mika-agent` vert.
- `cargo clippy` sans avertissement neuf.
- `make verify-bundled-skills` (aucun bundle touché, mais le gate est
  pré-merge).
- M1 relevée et **consignée dans le corps de la PR**, avec la branche de halte
  retenue. Un plan qui conditionne un réglage à une mesure doit livrer la
  mesure, sinon la condition n'a jamais été évaluée.
- Sonde post-déploiement, 48 h, avec ses haltes :
  - `dispatch_parent_settled` non vide sur mika-qa ;
  - `phantom_aged_out` en baisse sur mika-qa ;
  - **halte** — une row `manual` mika-qa `in_progress` de plus de 4 h subsiste
    alors que `dispatch_parent_settled` est non vide : un terme de U1 rate la
    population. Relire `dispatch_parent_settle_spared` **avant** d'élargir le
    prédicat ;
  - **halte** — `dispatch_parent_settled` tire sur une row dont la revue n'a
    pas encore été postée : désarmer par
    `MIKA_DISPATCH_PARENT_SETTLE_ENABLED=0` et réparer le terme `sibling`, ne
    pas rallonger la grâce (une grâce plus longue déplace le faux positif, elle
    ne le supprime pas).

---

## Definition of Done

1. `find_settleable_dispatch_parents` posée avec son doc-comment de non-appartenance à la paire self_dev.
2. `settle_dispatch_parents` câblée dans le scan moteur, après les reapers #871/#1162.
3. Garde de vivacité **réutilisée**, aucune réimplémentation de `pgrep`.
4. Deux variables d'environnement au contrat maison (trois paliers + clamp haut).
5. Trois événements de journal + un `tool_name` d'audit sole-writer.
6. Les cinq tests de U5, dont les deux contrôles négatifs et le scan structurel.
7. M1 relevée et consignée dans le corps de la PR.
8. Aucune modification de `prompt.rs`, de `qa_review_reconcile.rs`, ni des
   prédicats `source='self_dev'` existants.

---

## Acceptance criteria

Transcrits du corps de mika#2405, avec l'état que ce plan leur donne :

1. **Identifier le producteur exact des tâches `manual` mika-qa du chemin
   auto-QA (fichier:ligne).** → AC1 ci-dessus, chaîne à quatre maillons.
2. **Trancher H1 vs H2 avec preuve (grep du callsite de création + du chemin de
   sortie du verdict).** → AC2 : H1 vraie mais relocalisée (la Delegation Rule,
   pas #2359/#2372) ; H2 réfutée (`qa_review_reconcile` ne lit pas `tasks`) ;
   H3 ajoutée (le phantom sweep couvre la forme, mal et tard).
3. **Garantir une transition terminale pour chaque tâche `manual` mika-qa
   ouverte par ce chemin, fail-safe.** → U1+U2, verdict `completed`, garde de
   vivacité fail-safe dans les deux directions (`Live` et `Unreadable`
   épargnent).
4. **Test de non-régression : une revue auto-QA menée à verdict laisse zéro
   tâche `manual` mika-qa `in_progress` orpheline.** → U5 test 1, avec son
   contrôle négatif self_dev.
5. **Ne PAS annuler les tâches d'un agent QA en cours de travail (pgrep + mtime
   avant tout balayage).** → AC5 : déjà satisfait par `dispatch_liveness`
   (mika#2156), réutilisée plutôt que réimplémentée ; U5 test 2.

Ajouté au grooming, non présent au ticket :

6. **La population self_dev est inchangée** — aucun prédicat `source='self_dev'`
   existant n'est élargi, et U5 test 5 l'épingle.
7. **La mesure M1 est livrée avec la PR**, avec la branche de halte retenue.

---

## Sources

- `crates/mika-agent/src/prompt.rs:1709-1711` — Delegation Rule
- `crates/mika-agent/src/prompt.rs:1697-1699` — clôture conditionnée à une demande utilisateur
- `crates/mika-agent/src/tools/create_task.rs:258-280` et `:346` — la forme produite (chemin nominal + retry)
- `crates/mika-agent/src/server/ready_label_handler.rs:529-553` — le producteur `manual` qui, lui, pose `source='self_dev'`
- `crates/mika-agent/src/tools/update_task_status.rs:153` — l'outil verrouillé sur `trigger_type == "manual"`
- `crates/mika-agent/src/qa_build_callback.rs:178` — le « instead » du re-prompt mika#2355
- `crates/mika-agent/src/task_engine/dispatcher.rs:3152`, `:3254`, `:3431` — les trois backstops inline et leur terme `source='self_dev'`
- `crates/mika-agent/src/skills/executor.rs:3116` — `execute_long_running`
- `crates/mika-agent/src/skills/executor.rs:3234-3247` — `mark_parent_dispatched`
- `crates/mika-agent/src/skills/executor.rs:3251-3256` — `parent_task_id`
- `crates/mika-agent/src/skills/executor.rs:3001-3010` — `build_callback_task`
- `crates/mika-agent/src/db.rs:6894-6924` — `update_manual_task_status`
- `crates/mika-agent/src/db.rs:8057-8072` — prédicat du phantom sweep
- `crates/mika-agent/src/db.rs:7972` / `:8394` — la paire orphan/completable self_dev
- `crates/mika-agent/src/task_engine/engine.rs:449-458` — garde « manual = human work »
- `crates/mika-agent/src/qa_review_reconcile.rs:275`, `:643`, `:819` — H2 réfutée
- `crates/mika-agent/src/server/mod.rs:1685-1703` — enregistrement sur mika-dev
- `skills/bundled/qa-review-build-callback/system_prompt.md:11` — exclusion explicite de `update_task_status`
- mika#1712 / mika#2156 — phantom sweep et sa garde de vivacité
- mika#871 / mika#1162 — la paire orphan/completable
- mika#2120 — enforcement par prompt au substrat de boucle, mesuré
- mika#2205 — un scan silencieusement inactif se lit comme un scan oisif
- mika#2131 — doctrine journal vs `audit_events`
