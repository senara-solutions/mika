---
issue: 2335
type: fix
---

# fix(mika#2335) — la supersession et le stamp `fired_at` visaient la mauvaise ligne de task

## Symptôme (incident 2026-09-15)

La supersession a annulé le PARENT `094fc5f6` (`superseded_by_new_dispatch`) et laissé
tourner le pilote ENFANT `590a06c0` — deux pilotes vivants sur le même worktree, trois
fichiers laissés dirty sous les pieds du second. Un opérateur a lu la ligne db du pilote
vivant comme « inerte, jamais lancé » (`status=pending`, `fired_at=null`) et l'a annulée.

## Ce que la mesure établit

Artefacts de l'incident, lus dans ce worktree (`/var/log/claude-pilot/`,
`~/.mika/data/pilot-transcripts/`) :

- `590a06c0` : démarre `19:50:05Z`, `cwd=…/feat-2334-…/mika`, dernière écriture `20:29:09Z`.
- `f2bda0f2` : démarre `20:00:15Z`, **même worktree**, dernière écriture `20:42:39Z`.
- **Chevauchement : 28 min 54 s, deux pilotes sur `feat-2334`.** Confirme le ticket par une
  source indépendante.
- Le log de `590a06c0` ne contient **aucune** occurrence de `SIGTERM`, `CANCELLED_BY`,
  `superseded`, `Killed`, `interrupt` (0 hit), et s'arrête net sur `message_stop` en plein
  tour LLM. **La supersession ne lui a envoyé aucun signal** — elle ne l'a pas raté de peu,
  elle ne l'a jamais vu.

## Cause racine — une seule, deux visages

mika#2263 a livré les deux correctifs que ce ticket redemande (commits `57478654` défaut (a),
`0de6c63f` défaut (b), mergés `348f470f` le 2026-09-09). **Les deux sont posés sur la mauvaise
ligne de task**, et leurs deux tests sont verts parce que leurs fixtures écrivent des formes
que la production n'écrit jamais.

La topologie réelle d'un dispatch, c'est deux lignes :

| ligne | `trigger_type` | `reference_url` | `process_id` | écrite par |
|---|---|---|---|---|
| **parent** (tracking) | `manual` / `action_type='none'` | **oui** (URL de l'issue) | **jamais** | `server/ready_label_handler.rs:420+` |
| **enfant** (callback) | `callback` / `resume_agent` | **`None`** | **oui** (le pgid) | `skills/executor.rs:2897-2944` (l. **2936**), pgid posé l. **3279** |

L'URL est sur le parent. Le pgid est sur l'enfant. Rien ne porte les deux.

### Visage 1 — le kill ne peut structurellement trouver personne

`dispose_superseded_dispatch_processes` (`tracking_cleanup.rs:181`) sélectionne par
`find_live_dispatch_rows_by_reference_url_and_variants` (`db.rs:6964-6984`) :

```sql
WHERE agent_id = ?1
  AND process_id IS NOT NULL      -- seul l'ENFANT le satisfait
  AND status IN ('pending','in_progress')
  AND reference_url IN (?2, ?3)   -- seul le PARENT le satisfait
```

La conjonction est **vide sur la topologie de production**. `dispose_superseded_dispatch_processes`
ne peut jamais tuer un pilote de dispatch normal — pas dans ce cas, dans aucun. Le fix mika#2263
défaut (a) est inopérant depuis sa livraison. Corollaire : `cancel_task_superseded` (`db.rs:6995`)
est un UPDATE direct **sans cascade** (contrairement à `cancel_task`, `db.rs:6683`), donc rien en
aval ne rattrape.

**Le mécanisme de kill, lui, est correct** : `kill_process_gracefully` (`process_kill.rs:82`)
signale bien le **groupe** (`kill -TERM -<pid>`, l.53-58), possible parce que le spawn fait du
fils un chef de groupe (`.process_group(0)`, `executor.rs:3201`). Le ticket demande « tuer par
pgid » : c'est déjà fait. Ce qui manque, c'est la population.

**Le chaînon manquant existe déjà et est testé.** `find_dispatch_children_with_pid(parent_task_id)`
(`db.rs:7843-7910`, wrapper `async_db.rs:1023`) fait exactement la traversée parent → enfant-avec-pgid.
Sa propre doc l'énonce : *« the real process lives on a separate `long_running:*` recall row. That
recall row already points back via `parent_task_id` — the missing link is read here, not added. »*
Écrite pour mika#2156, consommée par `engine.rs:2291-2332` (`dispatch_liveness`). mika#2263 a écrit
un second résolveur par `reference_url` au lieu de réutiliser celui-ci — et le second est faux.

### Visage 2 — `fired_at` est stampé sur la ligne que personne ne lit

`set_task_process_id` (`db.rs:8648-8662`) stampe bien `fired_at` (clause `CASE … WHEN fired_at IS NULL`),
et c'est le seul écrivain de `process_id`. Mais son unique appelant de production est
`executor.rs:3279` — **sur l'enfant**. La transition de dispatch du parent passe par
`update_manual_task_status(task_id, "in_progress")` (`executor.rs:3067` → `db.rs:6708-6739`),
qui n'écrit **que** `status`, `updated_at`, `completed_at`.

Or les surfaces lisent le parent : CLI `mika tasks` (`mika-cli/src/commands/tasks.rs:464,519`),
dashboard (`server/dashboard.rs:573,610,696,726`), sondes de santé. **Il existe donc bien une
ligne de dispatch vivante qui se lit « jamais firée » — c'est le parent**, et c'est celle que
l'opérateur avait sous les yeux.

### Rectification d'une prémisse du ticket

`status=pending` sur une ligne **callback** n'est pas un mensonge : c'est la forme nominale
d'un dispatch en vol. mika#2272 l'a mesuré sur la base de production (897 lignes ayant porté un
`process_id` : 876 `delivered`, 19 `cancelled`, 1 `failed`, 1 `pending`, **zéro `in_progress`**),
et `db.rs:8708-8735` le documente. Faire passer les callbacks en `in_progress` toucherait le
watchdog #959, `get_undelivered_callback_tasks` et le reaper silent-stall #2272 : blast radius
hors de proportion avec le défaut. **On ne corrige donc pas le statut — on corrige le stamp
(qui, lui, est bien manquant sur le parent) et on rend la vivacité lisible.**

### Pourquoi les deux tests de mika#2263 sont verts

- `tests/eval/test_supersede_kills_live_pilot.rs:70-117` sème **une chimère** : une seule ligne
  portant à la fois `trigger_type: "callback"` (l.85), `reference_url: Some(url)` (l.97),
  `process_id` (l.108) et `parent_task_id: None` (l.82). Cette forme satisfait la conjonction
  SQL ; la production ne l'écrit jamais.
- `tests/eval/test_dispatch_fired_at_stamped.rs:38-96` sème la forme **parent**, puis appelle
  `set_task_process_id` **directement dessus** — un appel que la production ne fait jamais sur
  cette ligne. Le test valide le `CASE` SQL ; l'invariant qu'il énonce reste faux en prod.

Classe déjà nommée dans le dépôt : mika#2272 (« zéro était l'absence de mesure, pas la présence
de prudence »), corrigée là-bas par un contrôle positif **sur la forme de ligne que la production
écrit réellement**. Même remède ici.

## Correctifs

### F1 — la supersession dispose du pilote de l'enfant (facette 1)

1. `DispatchChild` (`task_state/tasks.rs:187-192`) gagne un champ `status`, et
   `find_dispatch_children_with_pid` (`db.rs:7865`) le sélectionne. Additif : `dispatch_liveness`
   (`engine.rs:2291`) ignore le champ, comportement mika#2156 inchangé. **Un seul prédicat de
   jointure, la décision de filtrage reste chez l'appelant** — plutôt qu'un second résolveur SQL,
   qui est précisément la duplication qui a produit ce défaut.
2. `dispose_superseded_dispatch_processes` cesse d'appeler
   `find_live_dispatch_rows_by_reference_url_and_variants` et prend en entrée la liste des
   **parents** candidats, puis, pour chacun, `find_dispatch_children_with_pid` filtré sur
   `status IN ('pending','in_progress')` (un enfant `delivered` n'a plus de pilote à tuer).
3. `supersede_prior_tracking_rows` calcule la liste de candidats **une fois**, avant le kill, et la
   réutilise pour l'annulation. Aujourd'hui les deux fonctions font deux lookups indépendants qui
   peuvent diverger ; après, il y en a un. **L'invariant d'ordre de mika#2263 est préservé** : le
   process meurt avant que la comptabilité des lignes commence (`tracking_cleanup.rs:60-66`).
4. **Fail-safe anti-réutilisation de PID** : un enfant dont `metadata.process_start_time` est absent
   ou illisible n'est **pas** signalé (`kill_process_gracefully` retomberait sur une simple
   existence de PID, qui ne distingue pas un PID recyclé). Le cas est compté et journalisé sous le
   vocabulaire déjà en place chez mika#2156 (`unusable_child_count`). Le coût est nommé : un enfant
   sans `start_time` survit à sa supersession — inertie, jamais un kill à l'aveugle.
5. Annulation de la ligne enfant et `clear_task_process_id` comme aujourd'hui
   (`tracking_cleanup.rs:222-238`), audit `superseded_dispatch_process_killed` inchangé.
6. `find_live_dispatch_rows_by_reference_url_and_variants` n'a plus d'appelant : supprimée, pas
   laissée en place. Une requête qui ne peut rien retourner et qui reste dans le fichier sera
   relue un jour comme une garantie.

### F2a — stamper `fired_at` sur le parent au dispatch (facette 2, lettre)

Écrivain dédié appelé en `executor.rs:3067` à la place de `update_manual_task_status(…, "in_progress")` :
transition de statut **et** `fired_at` sous la même clause « jamais réécrire un `fired_at` existant »
que `db.rs:8652-8656` (sinon l'âge d'un dispatch se remettrait à zéro sous les faucheurs qui le
mesurent). Un écrivain nommé plutôt qu'un `CASE` ajouté à `update_manual_task_status` : cette
méthode sert aussi `rewind.rs:484`, qui restaure un statut antérieur et ne doit rien stamper.

### F2b — la vivacité devient lisible là où l'opérateur regarde (facette 2, mode d'échec réel)

`print_task_summary` / `print_task_detail` (`mika-cli/src/commands/tasks.rs:421-493`) annotent déjà
`[executing, PID n]` **mais seulement pour `trigger_type == "callback"`** — pas pour le parent que
l'opérateur consulte — et **sans vérifier que le process est vivant**. Sur une ligne de tracking
parent, afficher l'état de son enfant de dispatch, verdict `is_same_process_alive`
(`task_engine/process_liveness.rs:51`) à l'appui : pilote vivant / PID mort / aucun enfant.
C'est la gravure dans l'outil de la leçon que le ticket grave dans la tête de l'opérateur
(« jamais de cancel sans pgrep PID + log mtime »).

### F2c — `mika tasks cancel` dit quand il s'apprête à tuer un pilote vivant

Le geste qui a causé l'incident. Avertissement nommant le PID et l'âge de la dernière écriture, et
confirmation requise quand un enfant de dispatch est vivant. Avertir et demander, pas refuser :
annuler un pilote vivant est parfois exactement ce qu'on veut.

## Risques et rayon d'action

- **F1** touche `tracking_cleanup.rs` et `db.rs` (zone décision-core) → PR human-gated.
  Fail-open de bout en bout conservé : toute erreur DB est journalisée et avalée, la supersession
  reste une courtoisie et jamais un préalable au dispatch.
- **F1 rend un kill effectif là où il n'y en avait aucun.** C'est l'objet du ticket, mais cela
  signifie qu'un défaut de sélection ne sera plus sans conséquence : d'où le fail-safe (4) et le
  filtrage de statut (2), qui bornent la population aux dispatches réellement en vol.
- **F2a change ce que les lecteurs de `fired_at` voient sur les lignes parents.** Vérifié : la
  sonde `long_running` de `get_task_health_summary` (`db.rs:7170-7186`) filtre
  `trigger_type != 'manual'` et ne regarde donc pas les parents. Les autres lecteurs
  (`db.rs:6397,6412` `row_to_task`, dashboard, CLI) sont des surfaces d'affichage. L'inventaire
  exhaustif des lecteurs est à refaire au moment de l'implémentation et à consigner dans le corps
  de PR.
- **Mesure indisponible dans cette session** : la base de production n'est pas montée dans le
  sandbox de grooming, donc les valeurs réelles de `process_id` / `fired_at` sur `590a06c0` et
  `094fc5f6` n'ont pas pu être relues. Ce n'est pas bloquant — les deux défauts sont établis
  **structurellement** (SQL vs forme d'écriture de production), indépendamment de cette lecture —
  mais elle appartient à la vérification post-déploiement (V3).

## Hors périmètre, délibérément

- Faire passer les lignes callback en `in_progress` (voir la rectification ci-dessus).
- La cause de l'arrêt net de `590a06c0` à `20:29:09Z` sans trace : hors signal de supersession,
  cela relève du reaper silent-stall (mika#2277) ou d'un SIGKILL manuel. Ce plan rend la
  supersession correcte ; il n'explique pas cette mort-là.
- `update_manual_task_status` au sens large et ses autres appelants.
- La sonde `long_running` de `get_task_health_summary`, qui filtre `trigger_type != 'manual'`
  alors que les callbacks sont `pending` et jamais `in_progress` — elle ne peut donc rien voir.
  Défaut réel trouvé en chemin, sans rapport avec celui-ci : **ticket de suivi à ouvrir.**

## Verification contract

- Test négatif (porte #2264) nommant l'invariant **« une supersession ne laisse jamais vivant le
  pilote de l'enfant »**, rouge-avant / vert-après, semé sur **la topologie de production** :
  parent `manual`/`action_type='none'` avec `reference_url` et **sans** `process_id`, enfant
  `callback`/`resume_agent` avec `parent_task_id`, **sans** `reference_url`, portant le pgid d'un
  vrai process et son `process_start_time` — via le chemin d'écriture de production autant que
  possible (précédent : `tests/eval/test_reaper_reaps_live_pending_pilot_2272.rs`).
- Contrôles négatifs : (a) un dispatch vivant sur **une autre** issue survit ; (b) un enfant
  `delivered` portant un vieux pgid n'est **pas** signalé ; (c) un enfant sans
  `process_start_time` n'est **pas** signalé (fail-safe F1.4).
- **Les deux fixtures de mika#2263 sont corrigées, pas conservées** : `test_supersede_kills_live_pilot.rs`
  (chimère l.70-117) et `test_dispatch_fired_at_stamped.rs` (appel que la prod ne fait pas, l.70-96).
  Les laisser vertes sur une fiction, c'est garder deux tests qui attestent l'inverse de ce qui tourne.
- Test F2a : un parent dispatché porte `fired_at` non-NULL ; un `fired_at` existant n'est jamais
  déplacé ; `rewind` ne stampe pas.
- `cargo build` + `cargo clippy --all-targets -- -D warnings` + suite verte. Sortie
  rouge-avant/vert-après collée au corps de PR (porte #2264).

## Definition of Done

- F1, F2a, F2b, F2c implémentés ; `find_live_dispatch_rows_by_reference_url_and_variants` supprimée.
- Tests ci-dessus verts, fixtures mika#2263 corrigées, clippy propre.
- Corps de PR : inventaire des lecteurs de `fired_at` sur lignes parents, et sortie
  rouge-avant/vert-après.
- PR human-gated (zone décision-core), non auto-mergée.

## Acceptance criteria

- **AC1** — Une supersession qui annule un parent portant un dispatch **vivant** signale le
  process du **fils** (par groupe) et le marque terminal. Ligne parent `cancelled`, ligne enfant
  terminale, `process_id` effacé, audit `superseded_dispatch_process_killed` écrit. Test négatif
  sur la topologie de production, rouge-avant / vert-après.
- **AC2** — La population du kill est obtenue par traversée `parent_task_id`
  (`find_dispatch_children_with_pid`), jamais par `reference_url` ; la requête inopérante est
  supprimée du dépôt.
- **AC3** — Fail-safe : un enfant sans `process_start_time` lisible, ou dont le statut est
  terminal, n'est **jamais** signalé ; le cas est compté et journalisé.
- **AC4** — Une ligne de tracking parent dont le dispatch a été lancé porte `fired_at` non-NULL ;
  un `fired_at` déjà posé n'est jamais réécrit ; `rewind` n'en pose aucun.
- **AC5** — `mika tasks get|list` sur un parent affiche l'état de vivacité **mesuré** de son
  enfant de dispatch (vivant / PID mort / aucun enfant), pas seulement la présence d'un PID.
- **AC6** — `mika tasks cancel` sur une task dont l'enfant de dispatch est vivant avertit en
  nommant le PID et exige confirmation.
- **AC7** — `cargo build` + `cargo clippy --all-targets -- -D warnings` + suite de tests verts ;
  les deux fixtures mika#2263 corrigées pour décrire la topologie de production.

## Note zone

Décision-core (`tracking_cleanup.rs`, `db.rs`, `task_engine/`, `skills/executor.rs`) → PR
human-gated (samidarko/Vincent).
