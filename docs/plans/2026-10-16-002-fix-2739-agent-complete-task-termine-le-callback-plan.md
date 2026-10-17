# mika#2739 — `complete_task` ne termine pas le callback d'un pilote vivant

> Ticket : senara-solutions/mika#2739 · labels `bug`, `p1-important`, `ready`,
> `dispatch:loop`, `loop-substrate`
> Branche : `fix/2739/agent-complete-task-termine-le-callback`

## Constat

Le 2026-10-16 à 13:19Z, un tour mika-dev de type « deferred-dispatch » a appelé
`complete_task` sur la tâche `long_running:run_claude_pilot` du vol
d'implémentation de mika#2700, avec ce résultat rédigé par le modèle :
« Parent task … (mika#2700 grooming) already completed. Stale callback cleared to
free dispatch slot. »

Le pilote était **vivant** : `dev-pilot/handlers/run.sh` tournait depuis 52 min,
sa sandbox `bwrap → claude-pilot → claude` était active.

La prémisse du tour est fausse par construction. La tâche ready-label parente est
`completed` **normalement**, puisque le moteur relance le pilote après le groom
en réutilisant la tâche (mika#1614). « Parent terminé » ne veut pas dire
« callback orphelin ».

Deux conséquences mesurées :

- la place `implement` a été libérée alors que le vol continuait, et un autre vol
  est parti 3 min plus tard (#2732) — trois pilotes sous un cap de 2 ;
- quand le pilote de #2700 rendra la main, son callback tombera sur une ligne
  déjà `completed`, et le traitement de son issue (PR_OPENED, etc.) n'aura pas
  lieu.

## Cause, vérifiée dans l'arbre

`crates/mika-agent/src/tools/complete_task.rs:67-87` ne vérifie que deux choses
avant d'écrire : `trigger_type == "callback"` (l. 67) et un statut
`pending|in_progress` (l. 74). Rien ne regarde `tasks.process_id`.

Le substrat de la garde manquante **existe déjà et est à portée** :

| brique | site | portée |
|---|---|---|
| `live_pilot_for_task(db, task_id) -> LivePilotVerdict` | `crates/mika-agent/src/live_pilot.rs:356` | `pub(crate)` — **même crate que l'outil** |
| la traversée parent→enfant | `live_pilot::resolve_task_pilot` | lecteur unique depuis mika#2653 phase B / mika#2670 |
| la preuve d'instance | `task_engine::process_liveness::is_same_process_alive` | appelée par `live_pilot` seul |
| la garde sœur, complète | `tools/cancel_task.rs:26-380` (site d'appel `:439`) | mika#2653 — forme à répliquer |

Autrement dit : **aucune logique de vivacité n'est à écrire**. Le travail est de
poser la garde au bon endroit, de trancher son arbitrage de fail-safe, et de la
rendre comptable.

### Les deux portes voisines sont déjà fermées — vérifié, pas supposé

Le ticket demande « `complete_task` **et tout outil LLM qui termine une tâche
`long_running:*`** ». Le recensement :

- `update_task_status` refuse déjà toute ligne non-`manual`
  (`update_task_status.rs:153`, *« exists but is not a manual task »*), test à
  `update_task_status.rs:465`. Une ligne `long_running:*` est un `callback` :
  **hors de portée de cet outil**.
- `cancel_task` porte déjà la garde mika#2653 — mais **gatée sur
  `ctx.is_webhook_pr_event_turn`** (`cancel_task.rs:439`). Le tour de l'incident
  est un tour « deferred-dispatch », donc **hors de cette population**. Voir
  § *Hors périmètre*, où c'est une décision et non un report.
- `promote_deferred_callback` mute le plan de dispatch, jamais un statut
  terminal.

Donc **`complete_task` est aujourd'hui la seule porte LLM** vers un statut
terminal sur une ligne `callback`. C'est ce qui rend un correctif mono-site
suffisant, et c'est pourquoi ce plan ne livre pas de scan « recense les portes ».

### Le chemin de livraison nominal n'est PAS sur ce chemin, et c'est décisif

`POST /tasks/{id}/complete` écrit par `agent_state.db.update_task_completed`
directement (`server/handlers.rs:674` et `:765`) — **il ne traverse pas l'outil**.
C'est par là que le trap EXIT de `dispatch-lib` délivre le callback, et
`live_pilot.rs` l'écrit noir sur blanc :

> *« `dispatch-lib`'s EXIT trap delivers the callback from inside the
> still-running wrapper, so `delivered` precedes process exit by a real
> window. »*

Pendant cette fenêtre le processus **est vivant** et la complétion **est
légitime**. Une garde posée dans `update_task_completed` ou dans le handler HTTP
casserait donc toute livraison nominale. **La garde va dans l'outil, et
uniquement dans l'outil** — même raisonnement que mika#2653, qui l'a mis dans
`CancelTaskTool` et non dans `cancel_task_and_kill` pour préserver les trois
chemins opérateur par construction plutôt que par prédicat.

## Décisions de conception

### D1 — Routage par le lecteur unique, jamais une seconde composition

La garde appelle `live_pilot::live_pilot_for_task(ctx.db, id)`. Elle ne touche ni
`find_dispatch_children_with_pid`, ni `is_terminal_task_status`, ni
`is_same_process_alive`.

Trois raisons, dont deux sont structurelles :

1. `canonical_tokens::mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique`
   refuse tout fichier de production composant les trois termes, avec une
   allowlist livrée **vide** et pinnée vide. Un `complete_task.rs` qui
   recomposerait la traversée la ferait rougir.
2. Les deux poignées de `find_dispatch_children_with_pid` sont `pub(crate)`
   depuis mika#2670 — *« un scan dit *tu as écrit ça* ; `pub(crate)` dit *tu ne
   peux pas l'écrire* »*.
3. `live_pilot_for_task` applique le **filtre terminal sur la ligne nommée
   aussi**, ce que la CLI avait oublié (mika#2670) : une ligne terminale portant
   encore un pgid vivant ne doit pas répondre `Alive`.

C'est un **quatrième appelant** du lecteur unique, pas une dérogation : *un
recensement n'est pas une allowlist — on y ajoute, on n'y exempte pas*
(mika#2633).

### D2 — Placement : après les deux refus existants, avant toute écriture

Ordre final dans `execute` :

1. `id` / `result` non vides, `result` sous `MAX_RESULT_LEN` *(inchangé)*
2. `AgentScopedTaskId::from_tool_context` + `validate_task_exists` *(inchangé)*
3. `trigger_type != "callback"` → erreur *(inchangé)*
4. statut hors `pending|in_progress` → erreur *(inchangé)*
5. **garde mika#2739** ← nouveau
6. `update_task_completed` + audit + succès *(inchangé)*

Après l'étape 4, pour deux raisons convergentes : une ligne terminale garde son
erreur honnête existante (*« has status 'completed' and cannot be completed »*),
et aucune requête de vivacité n'est payée sur une ligne déjà refusée. Avant
l'étape 6, donc **aucune écriture de statut ne peut précéder le verdict** — c'est
AC1.

### D3 — Arbitrage de fail-safe : fail-CLOSED, `Unreadable` refuse

Même direction que mika#2653, pour la même raison, et l'asymétrie est encore plus
franche ici :

| | faux `Alive` (refus à tort) | faux `None` (passage à tort) |
|---|---|---|
| coût | un appel d'outil refusé ; la place reste tenue jusqu'à ce que le watchdog #959, le faucheur #2249/#2277 ou `timeout_at` rende la ligne terminale | **un vol entier perdu** : place libérée sous le cap, un vol de trop parti (#2732), et le callback du pilote tombe sur une ligne `completed` — le PR_OPENED n'est jamais traité |
| visibilité | `tool_calls.output` + un WARN + une ligne d'audit | **aucune ligne** — c'est précisément pourquoi l'incident a dû être reconstitué à la main |
| boucle ? | non — aucun `required_tools` ne réclame `complete_task` : `grep -rn complete_task skills/` ne rend **aucune ligne**, donc aucune garde EndTurn ne peut réclamer un appel que la garde refuse | oui — se rejoue à chaque tour de dispatch différé |
| rattrapage | le callback arrive de lui-même ; l'opérateur garde `POST /tasks/{id}/complete` et `mika tasks cancel --yes` | aucun : le vol est détruit |

Donc `LivePilotVerdict::Unreadable` **refuse**, dans ses deux causes
(`UNREADABLE_DB_ERROR`, `UNREADABLE_NO_START_TIME`).

**Coût nommé plutôt que découvert.** Une ligne `callback` portant un
`process_id` **sans** `process_start_time` lisible — la population que mika#2335
compte sous `unusable_child_count` — ne pourra plus être complétée par l'outil
LLM. Borné (`timeout_at`, panic-fallback 6 h), compté par sa propre valeur
d'audit (`blocked_start_time_unreadable`), et les deux chemins opérateur restent
entiers. **L'arbitrage est local et ne se transporte pas** — il vaut pour cet
outil, pas pour `update_task_completed`.

### D4 — Pas de terme de classe de tour. Garde inconditionnelle.

`cancel_task` gate sur `is_webhook_pr_event_turn` parce que `cancel_task` est
« le geste de reprise le plus court de l'opérateur ». Ici le raisonnement ne
s'applique pas, et c'est vérifiable :

- `complete_task` est référencé hors de son fichier à **trois** sites de
  production — l'enregistrement `tools/mod.rs:1319`, l'allowlist
  `well_known_agents.rs:406`, la classification write `classification.rs:21` —
  plus un site de test (`classification.rs:238`) et la table de
  `docs/architecture.md:225`. **Aucun n'est un chemin opérateur** : les trois
  sites de production disent *qui peut appeler l'outil* et *comment il est
  classé*, jamais *qui termine une tâche sans passer par lui*.
- il n'existe **pas** de `mika tasks complete` : le chemin opérateur est
  `POST /tasks/{id}/complete`, qui ne traverse pas l'outil (voir plus haut).

Donc la garde est inconditionnelle, et AC1 est littéral : *« `complete_task` sur
une tâche callback dont le `process_id` est vivant renvoie une erreur »* — sans
qualificatif de tour. Le classificateur reste malgré tout une **fonction pure**
prenant le verdict en paramètre, pour que ses quatre issues soient testables sans
base.

### D5 — Vocabulaire d'audit partagé, `tool_name` distinct

Les quatre valeurs de verdict sont **réutilisées** depuis `cancel_task` par
import (`AUDIT_BLOCKED_LIVE_PILOT`, `AUDIT_BLOCKED_DB_UNREADABLE`,
`AUDIT_BLOCKED_START_TIME_UNREADABLE`, `AUDIT_ALLOWED_NO_PILOT`, déjà
`pub(crate)`), et `unreadable_audit_value` est élargie de privée à `pub(crate)`.

Ce qui sépare les deux populations est le **`tool_name`**, pas la valeur : un
opérateur groupe par `(tool_name, after_value)`. Réécrire les littéraux ici
couperait un vocabulaire en deux orthographes sans le dire — exactement ce que le
commentaire de `ERROR_CANCEL_LIVE_PILOT` interdit pour les motifs d'erreur.

Nouveaux noms, et eux seuls :

| constante | valeur |
|---|---|
| `COMPLETE_TASK_PILOT_GUARD_AUDIT_TOOL` | `complete_task_pilot_guard` |
| `ERROR_COMPLETE_LIVE_PILOT` | `complete_refused_live_pilot` |
| `ERROR_COMPLETE_PILOT_UNREADABLE` | `complete_refused_pilot_unreadable` |

Les trois issues écrivent une ligne d'audit, **y compris le passage**
(`allowed_no_pilot`). Sans ce contrôle positif, « zéro refus » et « la garde ne
tourne pas » rendent les mêmes octets (classe mika#2205). Le churn est borné : la
garde ne tourne que sur une ligne `callback` en statut complétable, population
rare par nature.

Écriture d'audit **fire-and-forget** : un échec d'audit ne peut pas changer un
verdict (motif `AuthBoundaryLedger`). Le résidu
`complete_task_pilot_guard_audit_failed` dit qu'à partir de là le `GROUP BY`
sous-compte.

### D6 — Le refus nomme le fait, jamais le gabarit du contournement

Doctrine mika#2520 / mika#2292, et elle mord ici : `MIKA_DEV_IDENTITY` porte
`shell-exec`. Le corps du refus porte le PID, le libellé de la tâche, la ligne
qui porte le pgid, les **deux** sorties correctes (attendre le callback ; rendre
la main en signalant), et à qui appartient la levée (un opérateur, sur l'hôte).
Il **ne nomme aucune commande**.

Il nomme en particulier la prémisse réfutée, puisque c'est elle qui a produit
l'incident : *un parent `completed` est l'état normal après un groom
(mika#1614) et n'est pas la preuve d'un callback orphelin.*

## Ordre d'implémentation

Trois commits, chacun compilant et testable seul.

### Commit 1 — élargir le vocabulaire réutilisé (`cancel_task.rs`)

- `fn unreadable_audit_value` → `pub(crate) fn`.
- Un paragraphe de doc sur `ALL_CANCEL_PILOT_GUARD_VERDICTS` disant que les
  quatre valeurs sont un vocabulaire **partagé** avec la garde mika#2739, et que
  c'est le `tool_name` qui sépare les populations.

Plancher : `cargo test -p mika-agent cancel_task` vert, aucune assertion
existante touchée.

### Commit 2 — la garde (`complete_task.rs`)

Dans l'ordre du fichier, au-dessus du `impl Tool` :

1. les trois constantes de D5 + `ALL_COMPLETE_TASK_PILOT_GUARD_ERRORS` ;
2. `enum CompletePilotDecision` à **trois** issues —
   `AllowedNoPilot`, `RefusedLivePilot { owner_task_id, pid }`,
   `RefusedUnreadable { reason }` — avec `audit_value()` en `match` **sans bras
   `_ =>`** (une quatrième issue doit être tranchée par le compilateur) et
   `refuses()` ;
3. `fn classify_complete_on_live_pilot(&LivePilotVerdict) -> CompletePilotDecision`
   — pure, trois bras, `Unreadable` → `RefusedUnreadable` (D3) ;
4. `fn refusal_body(&CompletePilotDecision, task_id, task_label) -> String` — JSON
   portant `error`, `doctrine: "mika#2739"`, `task`, `task_label`, `pilot_row`,
   `pilot_pid`, `reason`, `remedy` (D6) ;
5. `async fn report_complete_pilot_decision(...)` — un WARN par refus
   (`complete_task_live_pilot_blocked`, `complete_task_pilot_unreadable`) et un
   INFO sur le passage (`complete_task_pilot_absent`), chacun au niveau de son
   régime attendu ; puis la ligne d'audit fire-and-forget sous
   `COMPLETE_TASK_PILOT_GUARD_AUDIT_TOOL`. Champs du WARN : `event`, `agent_id`,
   `session_id`, `trace_id`, `task_id`, `task_label`, `owner_task_id`, `pid`
   — **AC3** ;
6. le site d'appel, à la position D2, avec le commentaire de placement.

Plancher : les six tests existants de `complete_task::tests` restent verts.

> **Ne pas supprimer `test_complete_task_wrong_trigger_type`.** Ce test est le
> **seul site de construction de `CreateScheduledTaskTool` dans tout l'arbre**, et
> `evidence/guards.rs` (variante `TestimonyProposalChannel::CreateScheduledTask`)
> écrit cette propriété comme un fait sur lequel il raisonne. Le retirer rendrait
> ce commentaire faux en silence.

### Commit 3 — détecteurs

**3a — `complete_task.rs`, `mod tests`.** Quatre cas de chemin de production
(`tool.execute(input, &ctx)`), plus les quatre issues du classificateur pur.

Les tests inline **suffisent et sont le chemin de production** ici, contrairement
à mika#2653 : cette garde n'a pas de terme de classe de tour (D4), donc elle
tourne dans `CompleteTaskTool::execute` quel que soit le `ToolContext`. Le mode
de panne que l'eval de mika#2653 devait fermer — *« le booléen n'est jamais
calculé »* — n'existe pas ici. Pas d'eval : ce serait une dépense sans axe.

| # | cas | montage | attendu |
|---|---|---|---|
| V1 | **pilote vivant** (AC1, rouge avant le correctif) | ligne `callback` `pending`, `process_id = std::process::id()`, `metadata.process_start_time` lu par `read_process_start_time` | `is_error`, contient `complete_refused_live_pilot` et le PID ; **statut toujours `pending`** après l'appel |
| V2 | **PID mort** (AC2) | `process_id = 999_999_999`, `process_start_time = 1` | succès, statut `completed` |
| V3 | **PID absent** (AC2) | `process_id = NULL` (le `add_callback_task` existant) | succès — c'est `test_complete_task_success`, inchangé, qui devient le contrôle négatif |
| V4 | `process_start_time` illisible (D3) | `process_id` vivant, `metadata = NULL` | `is_error`, contient `complete_refused_pilot_unreadable` ; statut `pending` |

`std::process::id()` plutôt qu'un enfant `sleep` + RAII : cette garde **ne
traverse aucun chemin de kill** — la raison même qui obligeait mika#2653 à
spawner un enfant (`cancel_task_and_kill` envoie un SIGTERM au groupe de
processus) tombe ici. C'est l'idiome que `live_pilot.rs` emploie déjà pour ses
propres unités (`self_pid_and_start`), et pour la même raison écrite au même
endroit.

**V1 et V4 sont `#[cfg(target_os = "linux")]`** : `is_same_process_alive` lit
`/proc/<pid>/stat` et rend `false` partout ailleurs. V2 et V3 sont portables.

**3b — `canonical_tokens.rs`.** Le scan d'écrivain unique du nouveau nom
d'audit, strictement sur le modèle `mika2653_le_nom_daudit_a_un_seul_ecrivain` :
aiguille composée à l'exécution (pour que le fichier ne se dénonce pas
lui-même), réutilisation de l'helper `audit_name_literal_present`, assertion
**d'anti-vacuité** d'abord (*un scan qui ne trouve personne se lit exactement
comme un scan propre*), puis l'assertion d'unicité. Plus
`COMPLETE_PILOT_GUARD_SOLE_WRITER_EXCEPTIONS: &[&str] = &[]` et son assertion
auto-nettoyante, **nommée** plutôt que désignée par position (leçon
mika#2496 : la phrase « l'allowlist ci-dessus » a déjà désigné la mauvaise).

### Documentation (hors décompte de lignes)

- `docs/architecture.md:225` — la cellule de `complete_task` dit la garde.
- `docs/agent-internals/tools.md` — un paragraphe sous la chaîne de validation :
  la garde, son fail-closed, son coût nommé, ses quatre valeurs d'audit.
- `scripts/sync-agent-docs.sh` **obligatoire** après : le job CI `docs-sync`
  échoue sinon.

## Contrat de vérification

```
cargo test -p mika-agent complete_task
cargo test -p mika-agent cancel_task
cargo test -p mika-agent canonical_tokens
cargo test                      # suite entière
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/sync-agent-docs.sh      # puis git diff --exit-code
make verify-bundled-skills
scripts/verify-pipeline.sh origin/main
```

**Preuve que V1 est rouge avant le correctif.** À écrire dans le corps de PR :
V1 posé sur le `complete_task.rs` d'avant la garde passe l'appel et rend
`completed` — c'est l'incident reproduit. Le constater avant d'ajouter la garde,
et le dire. Un test qui n'a jamais été vu rouge n'atteste rien (motif
`grooming_marker`, mika#2158).

### Sonde post-déploiement

```sql
SELECT after_value, count(*)
FROM audit_events
WHERE tool_name = 'complete_task_pilot_guard'
GROUP BY 1;
```

Lecture : `allowed_no_pilot` **doit** être non nul après quelques jours — c'est
le contrôle positif qui dit que la garde tourne. `blocked_live_pilot` non nul =
l'incident a été rattrapé. `blocked_start_time_unreadable` non nul = la
population `unusable_child_count` touche ce chemin, et le coût de D3 est réel
plutôt que théorique. **L'absence totale de lignes signifie un binaire antérieur
au correctif** (classe mika#2340), jamais « aucun appel » — l'instrument n'est
vivant qu'après `make deploy`.

## Fire-Disposition

Ce plan livre des détecteurs : les huit assertions de 3a et le scan d'écrivain
unique de 3b. Disposition retenue : **(a) exception nommée en allowlist, avec une
allowlist vide**.

- `COMPLETE_PILOT_GUARD_SOLE_WRITER_EXCEPTIONS` est livrée **vide**, et son
  caractère vide est **asserté** par un test nommé qui rougit si quelqu'un y
  dépose une entrée. Quand le scan tire, on route le site par
  `COMPLETE_TASK_PILOT_GUARD_AUDIT_TOOL` ; on n'exempte pas (doctrine mika#2201).
  Une allowlist née non vide serait un emplacement où déposer la prochaine
  infraction (mika#2323).
- **Aucune violation existante ne demande d'entrée**, et c'est vérifié plutôt que
  supposé : `grep -rn 'complete_task_pilot_guard' crates/` ne rend aujourd'hui
  **aucun** site, donc le nom naît avec exactement un écrivain. L'assertion
  d'anti-vacuité de 3b est ce qui empêche ce zéro de se lire comme un succès.
- Les huit assertions de 3a n'ont pas de population préexistante à exempter :
  V1 est le feu voulu (rouge avant, vert après), V2–V4 sont ses contrôles.
  Aucune n'atterrit `#[ignore]`.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/tools/complete_task.rs` (garde : constantes, enum, classificateur pur, corps de refus, report, site d'appel) | 165 |
| tests (`complete_task.rs`, `mod tests` : V1–V4 + 4 issues du classificateur pur) | 155 |
| `crates/mika-agent/src/tools/cancel_task.rs` (2 élargissements de visibilité + note de doc) | 10 |
| `crates/mika-agent/src/canonical_tokens.rs` (allowlist vide + scan d'écrivain unique + assertion auto-nettoyante) | 45 |

Total estimé : 375 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

**Un seul vol, et voici pourquoi il tient.** 375 < 431, avec 56 lignes de marge
sur une borne de 431, et sous le seuil par PR de 1000. Le volume est tenu par un
levier précis : **aucune logique de vivacité n'est écrite**. La traversée, le
filtre terminal, la preuve d'instance et le verdict à trois bras existent et sont
`pub(crate)` dans la même crate ; le vocabulaire d'audit et `unreadable_audit_value`
sont réutilisés par import (D5). Ce qui reste est un classificateur à trois bras,
un corps de refus, un report, un site d'appel.

**Si la région de production dépasse 200 lignes pendant l'implémentation**, le
levier est la densité de doc, pas un découpage : l'arbitrage de fail-safe, le
tableau de coût et la raison du placement sont **cités** depuis mika#2653
(`cancel_task.rs:26-380`) plutôt que redérivés — ils sont identiques, et une
seconde dérivation est une seconde chose qui peut diverger. Le § D3 de ce plan
porte la seule différence (la colonne « coût d'un faux `None` »), et c'est elle
qui va au site.

## Definition of Done

- [ ] La garde tourne dans `CompleteTaskTool::execute`, à la position D2, et
      aucune écriture de statut ne la précède.
- [ ] Elle passe par `live_pilot::live_pilot_for_task` ; aucun des trois termes
      du scan mika#2653 n'apparaît dans `complete_task.rs`.
- [ ] `Unreadable` refuse, dans ses deux causes, et le coût est écrit au site.
- [ ] Le refus porte le PID, le libellé de la tâche et la ligne qui porte le
      pgid ; il ne nomme aucune commande.
- [ ] Un WARN structuré par refus avec `task_id`, `task_label`, `pid`,
      `agent_id`, `session_id`, `trace_id`.
- [ ] Les trois issues écrivent une ligne d'audit sous
      `complete_task_pilot_guard`, en fire-and-forget.
- [ ] V1 constaté **rouge** avant la garde, et le corps de PR le dit.
- [ ] V1–V4 verts après ; V1/V4 gatés Linux.
- [ ] Le scan 3b est vert, son allowlist vide, son anti-vacuité assertée.
- [ ] Les six tests existants de `complete_task::tests` intacts, y compris
      `test_complete_task_wrong_trigger_type`.
- [ ] `docs/architecture.md` + `docs/agent-internals/tools.md` à jour, et
      `scripts/sync-agent-docs.sh` passé (`git diff --exit-code` propre).
- [ ] Contrat de vérification entièrement vert.
- [ ] Corps de PR : `Code review: delegated to mika-qa (mika#2694)`.

## Acceptance criteria

1. `complete_task` sur une tâche callback dont le `process_id` est vivant renvoie
   une erreur et ne change pas le statut.
2. Le même appel sur un `process_id` mort ou absent garde le comportement actuel.
3. Le refus produit une ligne de log structurée, avec la tâche, le PID et
   l'agent.

## Hors périmètre — décisions, pas des reports

**La population de la garde `cancel_task`.** `cancel_task` ne garde que les tours
`is_webhook_pr_event_turn` (`cancel_task.rs:439`) ; le tour de l'incident était
un tour « deferred-dispatch », donc un `cancel_task` depuis ce type de tour
tuerait encore un pilote vivant. **Ce plan ne l'élargit pas**, et c'est une
décision : mika#2653 écrit son arbitrage comme **local** — *« L'arbitrage est
local et ne se transporte pas »* — et son gating existe pour préserver le geste
de reprise de l'opérateur, lequel **traverse** `cancel_task` (`mika tasks
cancel`) alors qu'il ne traverse pas `complete_task` (D4). Élargir la population
de `cancel_task` est donc un arbitrage distinct, avec son propre tableau de coût,
et il appartient à un ticket qui le pose. Aucun AC de mika#2739 ne le demande.
Recommandation à l'opérateur : ouvrir ce ticket.

**Fermer `LivePilotVerdict::Unreadable` en enum de causes.** Rendrait exhaustif
le `match` de `unreadable_audit_value` et retirerait son `debug_assert_eq!` plus
son repli. Déjà nommé comme suivi par mika#2653, et touche les deux appelants de
mika#2279 ; ce plan en ajoute un troisième sans changer la forme.

**Une garde dans `update_task_completed` ou dans le handler HTTP.** Refusé par
construction : casserait la fenêtre du trap EXIT de `dispatch-lib`, qui délivre
le callback depuis l'intérieur du wrapper encore vivant.
