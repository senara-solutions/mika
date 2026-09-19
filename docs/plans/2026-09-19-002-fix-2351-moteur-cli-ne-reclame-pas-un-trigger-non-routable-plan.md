---
title: "fix(2351): le moteur de tâches du TUI ne réclame pas une tâche run_skill qu'il ne sait pas router"
type: fix
issue: senara-solutions/mika#2351
date: 2026-09-19
status: groomed-draft
---

# fix(2351) — le moteur CLI ne réclame pas un trigger qu'il ne sait pas router

## Contexte et diagnostic

Le ticket a d'abord été ouvert contre la garde anti-zombie mika#1742 : « elle refuse de
ré-enregistrer `qa_review_reconcile` à cause d'une instance morte ». Il a été re-cadré le
2026-09-19 sur preuve (voir le corps du ticket, § Re-cadrage). **La garde fait ce pour quoi elle
existe.** Le défaut est en amont : un second moteur de tâches, lancé sur un binaire périmé, tire
une récurrence qu'il ne sait pas router, et la tue.

Chaîne causale mesurée (n=2) :

1. `mika-cli/src/commands/chat.rs:221-265` : le TUI (`mika --agent mika-dev`) construit un
   `TaskEngine` sur la **même** base que mika-spirit et lance `TaskEngine::spawn_tick_loop`.
   Son `TaskDispatcher` porte `cli_mode: true`.
2. `db.rs:6760` `claim_and_fire_task` est atomique sur
   `(id, agent_id, status IN ('pending','recurring_active'))`. Les deux moteurs se disputent
   chaque tick de chaque récurrence de l'agent, et le premier qui réclame tire.
3. Le TUI a tourné sans interruption du 15 au 18/09 (1440 heartbeats par jour dans
   `~/.mika/agents/mika-dev/logs/mika.log.2026-09-1{5..8}`). Son image date d'avant `1340936e`
   (bras `"qa_review_reconcile"`) et d'avant #2341 (marqueur `unknown_trigger_death`).
4. fb425f89 (09-16 07:00:00.891Z) et 9bc130d0 (09-17 10:00:00.892Z) sont morts **dans le log
   du TUI**, avec le message générique `task dispatch failed … unknown run_skill trigger:
   qa_review_reconcile`. Aucun marqueur : le code du TUI précède #2341.
5. La garde #1742 arme alors, correctement, le veto de 24 h (`RECURRING_ZOMBIE_GRACE_HOURS`).

La prémisse de l'exemption #2337 (`db.rs:72-79`, « le binaire qui ré-enregistre un trigger est,
par construction, celui qui porte le bras ») vaut à l'intérieur d'un binaire. Elle est fausse
dès que deux binaires partagent la base. `tests/eval/test_recurring_trigger_wiring_2337.rs`
(§ Portée honnête) note qu'aucune de ses gardes n'aurait attrapé l'incident du 16/09. Ce plan
ferme cette divergence inter-binaires, et elle seule.

## Décision de conception

**Le moteur `cli_mode` ne réclame pas une tâche `run_skill` dont il ne sait pas router le
`trigger` intégré. Le daemon garde le comportement actuel.**

- Même principe que #264 (`engine.rs`, `if !self.dispatcher.cli_mode` autour de
  `dispatch_undelivered_callbacks`) : le moteur CLI est secondaire et laisse au daemon, le
  moteur canonique, ce qu'il ne peut pas traiter correctement.
- Le daemon (`cli_mode: false`) n'est **pas** modifié. Une tâche au trigger inconnu y meurt
  toujours : nommée, auditée, marquée (#2337). Une tâche au trigger erroné (typo d'un outil qui
  crée un `run_skill`) échoue donc bruyamment au lieu d'attendre en silence. Si le saut
  s'appliquait aussi au daemon, le bras `UnknownTrigger` de `engine.rs:3867` deviendrait
  inatteignable en production, et cette classe d'erreur deviendrait silencieuse.
- La garde #1742 et l'exemption #2337 ne sont pas touchées (AC5).

## Acceptance criteria

Transcrits tels quels depuis le corps de senara-solutions/mika#2351 (re-cadré le 2026-09-19).

1. **AC1 — pas de claim sans route, côté CLI.** Dans `TaskEngine::fire_task` (`engine.rs:3727`), quand `dispatcher.cli_mode` est vrai, une tâche `action_type = run_skill` portant un `trigger` intégré que ce binaire ne sait pas router n'est pas réclamée. Après le tick, sa ligne est inchangée (`status`, `fired_at`, `next_fire_at`, `updated_at`). → C2, C4-a
2. **AC2 — source unique.** Les bras de `dispatch_run_skill` et le prédicat de routabilité dérivent d'une seule définition. Un test de garde échoue si un bras existe sans entrée dans le prédicat, ou inversement. → C1, C4-c
3. **AC3 — lisibilité.** Une tâche ignorée émet un WARN nommé (`event = "task_trigger_not_routable_here"`, avec `task_id`, `label`, `trigger`), au plus une fois par `task_id` et par processus. → C3, C4-d
4. **AC4 — tests.** (a) Moteur `cli_mode: true` : une récurrence due, avec un trigger inconnu du binaire de test, reste `recurring_active` et **jamais** `failed` après un tick. **Contrôle positif** dans le même test : une récurrence due avec un trigger routable est réclamée et tirée. (b) Moteur `cli_mode: false` : le test existant `mika2337_un_trigger_inconnu_meurt_nomme_audite_et_marque` reste vert sans modification. → C4-a, C4-b
5. **AC5 — #1742 et #2337 inchangées (non-objectif explicite).** Le prédicat de `create_recurring_task_if_absent`, ses tests `mika1742_*` / `mika2337_*` et le bras `UnknownTrigger` du moteur ne sont pas modifiés. Un `failed` ou un `expired` récent continue de bloquer. La demande initiale de ce ticket (« ignorer les instances terminales ») est **retirée** : elle aurait désarmé la garde dans le cas pour lequel elle existe. → Non-objectifs

## Engagements (tie-back aux AC)

### C1 — source unique des triggers routables (AC2)

Fichier : `crates/mika-agent/src/task_engine/dispatcher.rs`.

- Ajouter `pub const ROUTABLE_RUN_SKILL_TRIGGERS: &[&str] = &["heartbeat", "reflection",
  "auto_pull_groomed", "wip_rescue", "qa_review_reconcile", "curator_review"];` et
  `pub fn is_routable_run_skill_trigger(name: &str) -> bool` (appartenance à la const). Les
  deux sont placés près de `dispatch_run_skill`, avec un doc-comment qui renvoie à #2351.
- Le `match trigger_name` de `dispatch_run_skill` (`dispatcher.rs:~573-587`) **garde sa forme
  textuelle actuelle**, bras par bras. La garde V3.1 de #2337 (`match_arm_triggers()` dans
  `tests/eval/test_recurring_trigger_wiring_2337.rs:263`) parse ce bloc textuellement, et une
  refonte en enum la casserait. La source unique est donc imposée par un test de bijection
  (C4-c), pas par une refonte.

### C2 — vérification avant claim, moteur CLI seulement (AC1)

Fichier : `crates/mika-agent/src/task_engine/engine.rs`, `fire_task` (`:3727`).

Avant `self.db.claim_and_fire_task(&task_id)`, et seulement si
`self.dispatcher.cli_mode && queued.action_type == action_type::RUN_SKILL` :

1. Relire la tâche (`self.db.get_task(&task_id)`). `QueuedTask` ne porte pas `action_config` :
   une lecture ciblée, limitée au mode CLI et aux `run_skill`, coûte moins qu'élargir
   `QueuedTask` et tous ses sites de construction.
2. Parser `action_config`. Si l'objet a une clé `skill_name`, **ne rien changer** (hors
   périmètre, voir plus bas). Si `trigger` est une chaîne et que
   `!is_routable_run_skill_trigger(trigger)`, **retourner sans réclamer**. La ligne reste
   intacte.
3. En cas d'échec de lecture ou de parse (erreur DB, `action_config` illisible), **ne pas
   sauter** : on retombe dans le chemin existant (claim, puis dispatch, où
   `dispatch_run_skill` produit son erreur habituelle). Le saut ne s'applique que sur la
   preuve positive que le trigger est inconnu de ce binaire. Il n'est jamais un nouveau mode
   d'échec silencieux.

Après un saut, la tâche a quitté le tas (`pop_from_heap` retire l'id de `queued_ids`). Le
`scan_db_for_new_tasks` suivant, toutes les `DB_SCAN_INTERVAL_TICKS`, la remettra en file tant
qu'elle est planifiable. C'est acceptable : le daemon la tire entre-temps et replanifie
`next_fire_at`. Le coût est une relecture par période de scan et par tâche non routable. Il
n'y a pas de boucle serrée. Un ensemble d'exclusion valable pour toute la vie du processus
est écarté volontairement. Si le daemon meurt et que le TUI reste seul moteur, une tâche
exclue deviendrait invisible au seul moteur restant, y compris après une mise à jour qui le
rendrait capable de la router (argument de mika-arch en première passe).

### C3 — lisibilité (AC3)

- Nouveau champ `TaskEngine::not_routable_warned: HashSet<String>` (ids déjà signalés).
- À chaque saut : si l'id n'est pas encore dans l'ensemble, émettre
  `warn!(event = "task_trigger_not_routable_here", task_id, label, trigger, "…")` puis
  l'insérer. Le message dit en clair que la tâche est laissée au daemon et que ce binaire
  (le TUI) ne connaît pas ce trigger, probablement parce qu'il est plus ancien. Au plus une
  fois par `task_id` et par processus.
- Pas de ligne `audit_events` : rien n'est muté, et le WARN suffit à diagnostiquer. Il est
  écrit dans le log de l'agent, là où l'incident était invisible depuis `server.log`.

### C4 — tests (AC4, AC2)

Dans `crates/mika-agent/tests/eval/test_recurring_trigger_wiring_2337.rs`, qui porte déjà le
harnais (`test_db`, `test_dispatcher`, `fire_recurring`, `match_arm_triggers`). Un fichier
dédié n'apporterait rien et dupliquerait le harnais.

- **C4-a — `mika2351_le_moteur_cli_ne_reclame_pas_un_trigger_inconnu` (AC1 + AC4a).** Un
  dispatcher de test en `cli_mode: true` (variante paramétrée de `test_dispatcher`). Deux
  récurrences dues sur la même base : `{"trigger":"zorglub"}` (inconnu) et un trigger
  routable **qui se tire sans réseau** (celui qu'utilise déjà
  `mika2337_la_recurrence_qa_review_reconcile_ne_tombe_pas_dans_le_catch_all`). On fait
  tourner les ticks jusqu'à ce que le **contrôle positif** ait `fired_at` renseigné. Cela
  prouve que le moteur a atteint `fire_task` pendant cette fenêtre, et l'assertion négative
  ne peut donc pas passer à vide. On vérifie ensuite que la ligne `zorglub` a toujours
  `status = 'recurring_active'`, `fired_at IS NULL` et le même `updated_at` qu'avant les
  ticks, et qu'elle n'a **jamais** le statut `failed`. Enfin, aucun événement d'audit
  `recurring_unknown_trigger` n'a été écrit.
- **C4-b — le daemon ne change pas (AC4b).**
  `mika2337_un_trigger_inconnu_meurt_nomme_audite_et_marque` (`cli_mode: false`) reste vert,
  **sans modification**. C'est le contrôle croisé de C4-a : même trigger `zorglub`, autre
  mode, issue opposée. C4-a et C4-b ensemble épinglent terme par terme la condition
  `cli_mode && !routable` : chacun des deux termes a un test qui échoue si on le retire.
- **C4-c — `mika2351_les_triggers_routables_sont_exactement_les_bras_du_dispatcher` (AC2).**
  L'ensemble `ROUTABLE_RUN_SKILL_TRIGGERS` doit être égal à l'ensemble `match_arm_triggers()`
  (dans les deux sens, avec un message qui nomme le côté manquant). Contrôle négatif prouvant
  que le test mord, **dans les deux sens** : vérifier une fois à la main (1) qu'ajouter au
  `match` un bras fictif absent de la const fait échouer C4-c en nommant le côté « bras sans
  entrée », et (2) que retirer de la const une entrée dont le bras existe le fait échouer en
  nommant le côté « entrée manquante ». Consigner les deux sorties rouges dans la PR, sans les
  committer.
- **C4-d — lisibilité (AC3), dans le module `#[cfg(test)]` de `engine.rs`.** Deux passages de
  `fire_task` sur la même tâche non routable laissent `not_routable_warned` de taille 1. On
  asserte sur l'ensemble, pas sur la capture de tracing.

### C5 — documentation

- Doc-comment de `RECURRING_UNKNOWN_TRIGGER_PATH` (`db.rs:66-86`) : ajouter une phrase qui
  borne la prémisse (« vrai à l'intérieur d'un binaire ; avec plusieurs moteurs sur une même
  base, voir #2351 : le moteur CLI ne réclame pas ce qu'il ne sait pas router »). C'est de la
  documentation seulement, le prédicat n'est pas modifié (AC5).
- `/ce:compound` après implémentation : « deux moteurs de tâches sur une même base, le plus
  ancien tue ce qu'il ne connaît pas » (la classe, pas l'incident).

## Non-objectifs (AC5 et hors périmètre du ticket)

- **Garde #1742 inchangée.** `create_recurring_task_if_absent`, `RECURRING_ZOMBIE_GRACE_*`, les
  tests `mika1742_*` et `mika2337_*` de `db::tests` et le bras `UnknownTrigger` de
  `engine.rs:3867` ne sont pas modifiés. La demande initiale (« ignorer les instances
  terminales ») est retirée.
- Supprimer le `TaskEngine` du TUI, ou mettre en place un bail de moteur unique par base.
- Rendre lisible au démarrage la version du binaire en exécution.
- Les tâches `run_skill` par `skill_name` dont le skill manque au registre du TUI.
- Protéger contre un TUI **déjà** périmé : le fix ne vaut que pour les binaires qui le
  contiennent.

## Vérification

- `cargo test -p mika-agent --test eval mika2351_` et `mika2337_` (C4-a, C4-b, C4-c).
- `cargo test -p mika-agent task_engine::engine` (C4-d et non-régression
  `test_cli_mode_skips_callback_dispatch`).
- `cargo clippy --workspace --all-targets -- -D warnings`.
- Rouge avant : C4-a doit échouer sur `main`, où le moteur CLI réclame `zorglub` et la tâche
  finit `failed`. Le consigner dans la PR.

## Risques

- **Le saut masque une vraie erreur de configuration côté CLI.** Borné : le daemon tire la même
  tâche et échoue bruyamment si le trigger est vraiment faux (C4-b). Le CLI ne rend silencieux
  que son propre tir.
- **Pas de daemon en marche (TUI seul).** Une tâche non routable par le TUI reste alors
  planifiée sans être tirée. Avant ce fix, elle mourait et armait un veto de 24 h. Rester
  planifiée est strictement meilleur, et le WARN AC3 le signale.
- **Coût de relecture.** Une `get_task` par tir d'un `run_skill` en mode CLI seulement. Ces
  tirs se comptent en unités par heure, le coût est négligeable.
