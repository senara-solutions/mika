---
title: "fix(2351): le moteur de tâches du TUI ne réclame pas une tâche run_skill qu'il ne sait pas router"
type: fix
issue: senara-solutions/mika#2351
date: 2026-10-15
status: groomed-draft
---

# fix(2351) — le moteur CLI ne réclame pas un trigger qu'il ne sait pas router

## Contexte

Le ticket a d'abord été ouvert contre la garde anti-zombie mika#1742 : « elle refuse de
ré-enregistrer `qa_review_reconcile` à cause d'une instance morte ». Il a été re-cadré le
2026-09-19 sur preuve (corps du ticket, § Re-cadrage). **La garde fait ce pour quoi elle
existe.** Le défaut est en amont : un second moteur de tâches, lancé sur un binaire périmé,
tire une récurrence qu'il ne sait pas router, et la tue.

Chaîne causale mesurée (n=2) :

1. `crates/mika-cli/src/commands/chat.rs:215-226` : le TUI (`mika --agent mika-dev`) construit
   un `TaskDispatcher` portant `cli_mode: true` sur la **même** base que mika-spirit, puis
   lance `TaskEngine::spawn_tick_loop`. Son moteur est complet.
2. `claim_and_fire_task` est atomique sur
   `(id, agent_id, status IN ('pending','recurring_active'))`. Les deux moteurs se disputent
   chaque tick de chaque récurrence de l'agent, et le premier qui réclame tire.
3. Le TUI a tourné sans interruption du 15 au 18/09 (1440 heartbeats par jour dans
   `~/.mika/agents/mika-dev/logs/mika.log.2026-09-1{5..8}`). Son image date d'avant `1340936e`
   (bras `"qa_review_reconcile"`) et d'avant #2341 (marqueur `unknown_trigger_death`).
4. fb425f89 (09-16 07:00:00.891Z) et 9bc130d0 (09-17 10:00:00.892Z) sont morts **dans le log
   du TUI**, avec le message générique `task dispatch failed … unknown run_skill trigger:
   qa_review_reconcile`. Aucun marqueur : le code du TUI précède #2341.
5. La garde #1742 arme alors, correctement, le veto de 24 h.

La prémisse de l'exemption #2337 (`RECURRING_UNKNOWN_TRIGGER_PATH`, « le binaire qui
ré-enregistre un trigger est, par construction, celui qui porte le bras ») vaut à l'intérieur
d'un binaire. Elle est fausse dès que deux binaires partagent la base. Le module-doc de
`tests/eval/test_recurring_trigger_wiring_2337.rs` (§ Portée honnête) note qu'aucune de ses
gardes n'aurait attrapé l'incident du 16/09. Ce plan ferme cette divergence inter-binaires, et
elle seule.

---

## Relevé de l'arbre au 2026-10-15 (ce qui a changé depuis le grooming du 19/09)

Le plan de septembre (branche `feat/2351/mika-1742-le-guard-de-r-enregistrement`, retiré par
`c73ba8ee` faute de preuve de groom en base) a reçu READY en première passe. Près d'un mois
s'est écoulé et **mika#2446 a été mergé entre-temps** (`2b5456cc`). Trois de ses effets
changent le périmètre, et le quatrième change une prémisse d'un AC. Relevé daté, parce que
c'est ce qui sépare un plan re-mesuré d'un plan recopié :

- **La source unique existe déjà.** `crates/mika-agent/src/task_engine/dispatcher.rs:75-99`
  porte `ROUTABLE_TRIGGERS` (sept entrées : `heartbeat`, `reflection`, `auto_pull_groomed`,
  `wip_rescue`, `qa_review_reconcile`, `worktree_reap`, `curator_review`),
  `routable_triggers_csv()` et `is_routable_trigger()` — « lecteur unique ». Le plan de
  septembre prévoyait de les créer (son C1) : **ce livrable est désormais sans objet.** Le
  prédicat d'AC1 consomme la constante existante.
- **Le septième bras.** `worktree_reap` a été ajouté au `match trigger_name` depuis. Toute
  liste retapée serait déjà périmée : raison de plus pour ne pas en écrire une.
- **La garde de bijection, elle, n'a pas été livrée.** Le doc-comment de `ROUTABLE_TRIGGERS`
  affirme que « son honnêteté est tenue par une garde de source
  (`mika2446_l_inventaire_est_la_projection_exacte_du_match`, dans
  `tests/eval/test_recurring_trigger_wiring_2337.rs`) » et le module-doc de ce fichier la
  décrit en D-1. **Ce test n'existe nulle part dans l'arbre**
  (`grep -rn 'l_inventaire_est_la_projection_exacte_du_match' crates/` ne rend que les deux
  citations, aucune définition). L'inventaire est donc un inventaire dont l'honnêteté est
  *affirmée* sans être *tenue* — précisément ce que son propre commentaire dit être pire que
  pas d'inventaire. **C'est AC2 de ce ticket, mot pour mot**, et c'est ce plan qui le livre.
- **La prémisse d'AC4(b) est fausse.** AC4(b) demande que
  `mika2337_un_trigger_inconnu_meurt_nomme_audite_et_marque` « reste vert **sans
  modification** », en le qualifiant de test `cli_mode: false`. Il tourne en réalité en
  `cli_mode: true` : son harnais `test_dispatcher_in`
  (`tests/eval/test_recurring_trigger_wiring_2337.rs:536`) câble `cli_mode: true` en dur.
  Implémenter AC1 le rendrait **rouge** — la tâche `zorglub` ne serait plus réclamée, donc ne
  mourrait plus. Voir § Décisions de grooming, D3.

Deux autres gardes citées par le module-doc manquent aussi
(`mika2446_chaque_trigger_routable_a_un_armement`, D-2, et
`mika2446_le_stop_precede_le_jeton_et_git_dans_le_reaper`, D-3), ainsi que la sonde
exhaustive `mika2446_chaque_trigger_routable_est_tire`. Elles ne sont **pas** dans le
périmètre de ce ticket (sûreté de la sonde destructive de #2446, pas routabilité au claim) :
le plan rectifie le module-doc pour qu'il n'affirme plus leur existence, et les nomme comme
constat à remonter plutôt que de les livrer en passager clandestin. Voir § Hors périmètre.

---

## Décisions de grooming

**D1 — le saut vaut pour `cli_mode`, pas pour le daemon.** Le moteur CLI est secondaire et
laisse au daemon, moteur canonique, ce qu'il ne peut pas traiter correctement. Deux précédents
exacts dans le même fichier : `engine.rs:1286` (#264, le CLI ne vole pas les callbacks) et
`mika2575_le_mode_cli_ne_rearme_pas` (`engine.rs:6336`, « le CLI laisse la ligne intacte — ni
ré-armée, ni échouée »). Ce plan ajoute un troisième membre à cette famille, il n'en invente
pas la forme.

**D2 — le daemon n'est pas touché, et c'est ce qui garde la classe bruyante.** Une tâche au
trigger erroné (typo d'un outil qui crée un `run_skill`) meurt toujours chez le daemon :
nommée, auditée, marquée, avec son attribution de binaire (#2337 + #2446). Si le saut
s'appliquait aussi au daemon, le bras `UnknownTrigger` (`engine.rs:5134`) deviendrait
inatteignable en production et cette classe d'erreur deviendrait silencieuse. C'est le
non-objectif qui donne sa valeur au reste.

**D3 — la bascule de `mika2337_un_trigger_inconnu_meurt_nomme_audite_et_marque` est
une modification, et c'est la bonne.** AC4(b) veut que le comportement #2337 survive côté
daemon ; il suppose à tort que ce test est déjà un test daemon. L'intention est tenue en
faisant du test ce qu'il prétend être : il bascule en `cli_mode: false`. Coût : cinq lignes,
par le pattern déjà en service dans `mika2575_le_mode_cli_ne_rearme_pas`
(`Arc<TaskDispatcher> { cli_mode: false, ..Arc::try_unwrap(d) }`), passé à
`fire_recurring_with` — aucun paramètre ajouté au harnais, aucun appelant existant touché.
**Aucune assertion du test n'est modifiée** : ses onze assertions (statut `failed`, message,
ligne d'audit, les cinq champs d'attribution non vides, l'inventaire émis, le marqueur
entier) restent mot pour mot. Ce qui change est le mode du moteur qui les produit, et c'est
exactement la distinction que ce ticket introduit. L'alternative — exempter `zorglub` du saut
— reviendrait à percer la règle pour que le test ne la voie pas.

**D4 — le prédicat lit la constante, il ne la réécrit pas.** `is_routable_trigger` est le
lecteur unique de `ROUTABLE_TRIGGERS` et il est déjà public. AC2 est donc tenu par
construction côté production ; ce qui restait à livrer est la garde qui empêche l'inventaire
et le `match` de diverger (D-1 de #2446, jamais écrite). Le `match trigger_name` **garde sa
forme textuelle actuelle**, bras par bras : la garde de classe `mika2337_tout_trigger_
enregistre_a_un_bras_dans_le_dispatcher` le parse textuellement (`match_arm_triggers()`,
`tests/eval/…:365`), et une refonte en table de dispatch la casserait — en plus de déplacer la
décision dans une donnée, qui ne fait rougir aucun compilateur.

**D5 — pas d'ensemble d'exclusion persistant.** Une tâche sautée quitte le heap et sera
re-enfilée par `scan_db_for_new_tasks` (`engine.rs:1458`, toutes les `DB_SCAN_INTERVAL_TICKS`
= 60 ticks), qui re-enfile toute tâche schedulable absente de `queued_ids` et recalcule
l'échéance depuis `cron_expr`. Mémoriser l'exclusion pour la vie du processus serait moins
cher mais faux : si le daemon meurt et que le TUI reste seul moteur, une tâche exclue
deviendrait invisible au seul moteur restant, y compris après une mise à jour qui le rendrait
capable de la router (objection de mika-arch en première passe, conservée).

---

## Conception

### C1 — vérification avant claim, moteur CLI seulement (AC1)

Fichier : `crates/mika-agent/src/task_engine/engine.rs`, `fire_task` (`:4985`).

Avant `self.db.claim_and_fire_task(&task_id)`, et seulement si
`self.dispatcher.cli_mode && queued.action_type == action_type::RUN_SKILL` :

1. Relire la tâche (`self.db.get_task(&task_id)`). `QueuedTask` (`queue.rs:9-17`) ne porte pas
   `action_config` : une lecture ciblée, limitée au mode CLI et aux `run_skill`, coûte moins
   qu'élargir `QueuedTask` et tous ses sites de construction.
2. Parser `action_config`. Si l'objet porte une clé `skill_name`, **ne rien changer** (hors
   périmètre — c'est la branche `dispatch_skill_by_name`, une autre forme, non observée ici).
   Si `trigger` est une chaîne et que `!dispatcher::is_routable_trigger(trigger)`, **retourner
   sans réclamer**. La ligne reste intacte.
3. En cas d'échec de lecture ou de parse (erreur DB, `action_config` illisible), **ne pas
   sauter** : on retombe dans le chemin existant (claim, puis dispatch, où `dispatch_run_skill`
   produit son erreur habituelle). Le saut ne s'applique que sur la **preuve positive** que le
   trigger est hors de l'inventaire de ce binaire. Il n'est jamais un nouveau mode d'échec
   silencieux.

### C2 — lisibilité (AC3)

- Nouveau champ `TaskEngine::not_routable_warned: HashSet<String>` (ids déjà signalés).
  `fire_task` prend `&mut self` : pas de verrou nécessaire. Précédent de forme dans la même
  struct : `activity_not_recorded_warned` (`engine.rs:795`), pour la même raison — le reaper
  repasse toutes les 60 s et une condition stable ne doit pas produire un WARN par minute
  pour toujours.
- À chaque saut : si l'id n'est pas encore dans l'ensemble, émettre
  `warn!(event = "task_trigger_not_routable_here", task_id, label, trigger, …)` puis
  l'insérer. Le message dit en clair que la tâche est laissée au daemon et que ce binaire ne
  connaît pas ce trigger, probablement parce qu'il est plus ancien — avec l'inventaire de ce
  binaire (`routable_triggers_csv()`), pour que la ligne soit lisible seule, comme celle de
  #2446.
- Pas de ligne `audit_events` : rien n'est muté, et le WARN suffit à diagnostiquer. Il est
  écrit dans le log de l'agent, là où l'incident était invisible depuis `server.log`.

### C3 — rectifier deux affirmations devenues fausses (AC1, constat #2446)

- `dispatcher.rs:124` — le doc-comment de `BinaryAttribution::process_name` affirme que
  « `cli_mode` ne court-circuite jamais le dispatch `run_skill` ». Cette phrase devient fausse
  avec C1 : elle est remplacée par ce qui sera vrai (le CLI tire les mêmes lignes de la même
  base, **sauf** celles dont il ne sait pas router le trigger — renvoi #2351). Aucune garde de
  source ne l'épingle : la correction est textuelle.
- Module-doc de `tests/eval/test_recurring_trigger_wiring_2337.rs:54-78` — D-1 est livrée par
  ce plan (C5) et son renvoi devient vrai. D-2, D-3 et la sonde exhaustive
  `mika2446_chaque_trigger_routable_est_tire` ne sont pas dans l'arbre : le module-doc cesse
  de les décrire à l'indicatif et les nomme comme non livrées, avec le constat daté. Un
  document qui décrit des gardes absentes est le même défaut qu'un inventaire qui mentirait.

### C4 — comportement : tests (AC1, AC4)

Dans `crates/mika-agent/tests/eval/test_recurring_trigger_wiring_2337.rs`, qui porte déjà le
harnais (`test_db`, `test_dispatcher`, `fire_recurring`, `fire_recurring_with`,
`match_arm_triggers`). Un fichier dédié dupliquerait ce harnais sans rien ajouter.

- **C4-a — `mika2351_le_moteur_cli_ne_reclame_pas_un_trigger_inconnu` (AC1, AC4a).**
  `test_dispatcher` est **déjà** en `cli_mode: true` : rien à paramétrer. Deux récurrences dues
  sur la même base, le même moteur les voit toutes deux (`startup_recovery` enfile l'existant,
  puis le scan) :
  - `zorglub_scan` → `{"trigger":"zorglub"}`, enregistrée par `ensure_recurring_task` **avant**
    l'appel, et sa ligne capturée aussitôt (`status`, `fired_at`, `next_fire_at`, `updated_at`) ;
  - **contrôle positif** `qa_review_reconcile` → trigger routable qui se tire **sans réseau**
    (`github_token` et `github_app` tous deux `None` ⇒ `dispatch_qa_review_reconcile` rend
    `Ok(())` avant tout appel à `gh` ; c'est l'hermétisme sur lequel
    `mika2337_la_recurrence_qa_review_reconcile_ne_tombe_pas_dans_le_catch_all` repose déjà),
    passé à `fire_recurring` dont le prédicat `settled` est « `fired_at` renseigné ».

  Le prédicat d'arrêt porte sur le **contrôle positif**, jamais sur la ligne sautée : c'est ce
  qui empêche l'assertion négative de passer à vide. Quand il est satisfait, le moteur a
  prouvé qu'il a atteint `fire_task` pendant la fenêtre. On asserte alors que la ligne
  `zorglub` est **identique** à sa capture sur les quatre colonnes d'AC1, qu'elle n'est
  **jamais** `failed`, et qu'aucun événement d'audit `recurring_unknown_trigger` n'a été
  écrit. Rouge sur `main` : le moteur CLI y réclame `zorglub` et la ligne finit `failed`.
- **C4-b — le daemon ne change pas (AC4b).**
  `mika2337_un_trigger_inconnu_meurt_nomme_audite_et_marque` bascule en `cli_mode: false`
  (D3) et reste vert, **assertions inchangées**. C'est le contrôle croisé de C4-a : même
  trigger `zorglub`, autre mode, issue opposée. Les deux ensemble épinglent terme par terme la
  condition `cli_mode && !routable` — retirer l'un ou l'autre terme fait rougir l'un des deux.
- **C4-c — dédup du WARN (AC3), dans le module `#[cfg(test)]` de `engine.rs`.** Deux passages
  de `fire_task` sur la même tâche non routable laissent `not_routable_warned` de taille 1. On
  asserte sur l'ensemble (champ privé, accessible depuis le module de test), pas sur une
  capture de tracing. Le harnais `test_dispatcher_with` + override `cli_mode: true` y est déjà
  en service (`mika2575_le_mode_cli_ne_rearme_pas`).

### C5 — source : la garde de bijection (AC2)

`mika2446_l_inventaire_est_la_projection_exacte_du_match`, dans le même fichier — **le nom
cité par `dispatcher.rs:55` et par le module-doc D-1**, repris tel quel pour que ces deux
renvois deviennent vrais au lieu d'être réécrits, avec un renvoi #2351 ajouté dans son
doc-comment pour dire qui l'a finalement livrée.

L'ensemble `ROUTABLE_TRIGGERS` doit être l'égal ensembliste de `match_arm_triggers()`, **dans
les deux sens**, et le message d'échec nomme le côté manquant (« bras sans entrée
d'inventaire » / « entrée d'inventaire sans bras »). Liste blanche **vide** : une exception ici
recréerait exactement le défaut que la garde interdit, un inventaire qui ment. La population
est non vide par construction — `match_arm_triggers()` panique si le bloc `match` est
introuvable, et la garde asserte `≥ 7` membres avant de comparer, pour qu'un parseur devenu
aveugle rougisse au lieu de passer sur deux ensembles vides.

**Contrôle négatif, dans les deux sens.** Vérifier une fois à la main (1) qu'ajouter au `match`
un bras fictif absent de la constante fait échouer la garde en nommant « bras sans entrée », et
(2) que retirer de la constante une entrée dont le bras existe la fait échouer en nommant
« entrée sans bras ». Consigner les deux sorties rouges dans la PR, sans les committer.

---

## Étapes d'implémentation

1. `engine.rs` — champ `not_routable_warned` + initialisation dans `new()` (C2).
2. `engine.rs` — le saut en tête de `fire_task`, avec son WARN dédupliqué (C1 + C2).
3. `dispatcher.rs` — rectifier le doc-comment de `process_name` (C3).
4. `tests/eval/test_recurring_trigger_wiring_2337.rs` — C5 d'abord (garde de bijection : elle
   doit être verte sur l'arbre actuel, sinon l'inventaire est déjà faux et c'est un constat à
   remonter avant d'aller plus loin), puis C4-a, puis la bascule C4-b, puis le module-doc.
5. `engine.rs` — le test de dédup C4-c.
6. Rouge-avant : lancer C4-a sur l'arbre **avant** l'étape 2 et consigner son échec.

---

## Contrat de vérification

- `cargo test -p mika-agent --test eval mika2351_` — C4-a.
- `cargo test -p mika-agent --test eval mika2337_ mika2446_` — C4-b et C5, plus les gardes de
  classe existantes.
- `cargo test -p mika-agent task_engine::engine` — C4-c et non-régression
  `test_cli_mode_skips_callback_dispatch`, `mika2575_le_mode_cli_ne_rearme_pas`,
  `mika2379_cli_mode_startup_recovery_leaves_a2a_rows_alone`.
- `cargo test -p mika-agent db::tests::recurring_tasks` — les jumeaux `mika1742_*` /
  `mika2337_*` côté ré-inscription, inchangés (AC5).
- `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
- `scripts/verify-pipeline.sh origin/main`.
- **Rouge avant, consigné dans la PR :** C4-a échoue sur `main` (ligne `zorglub` → `failed`) ;
  les deux contrôles négatifs de C5 (§ C5).

---

## Risques nommés

- **Le saut masque une vraie erreur de configuration côté CLI.** Borné : le daemon tire la même
  tâche et échoue bruyamment si le trigger est vraiment faux (C4-b). Le CLI ne rend silencieux
  que son propre tir, et le WARN d'AC3 le dit sur place.
- **Pas de daemon en marche (TUI seul).** Une tâche non routable par le TUI reste alors
  planifiée sans être tirée. Avant ce fix, elle mourait et armait un veto de 24 h. Rester
  planifiée est strictement meilleur — et D5 garantit qu'elle redevient tirable dès que le
  binaire sait la router.
- **Coût de relecture.** Une `get_task` par tir d'un `run_skill`, en mode CLI seulement. Ces
  tirs se comptent en unités par heure.
- **La bijection de C5 rouge dès l'atterrissage.** Si `ROUTABLE_TRIGGERS` diverge déjà du
  `match` sur l'arbre, l'étape 4 le révèle. Disposition : halte-et-remontée (§ Fire-Disposition)
  — un inventaire déjà faux est un fait à porter à l'opérateur, pas une entrée d'allowlist à
  écrire.

---

## Fire-Disposition

Le plan livre trois détecteurs : **C4-a** (comportement CLI), **C4-c** (dédup du WARN) et
**C5** (garde de source, bijection inventaire ↔ `match`).

**Disposition retenue : (c) halte-et-remontée, pour les trois.**

- **C4-a et C4-c** sont des tests de comportement sur du code que cette PR écrit. Leur
  population est ce que la PR produit : il n'y a aucune violation préexistante à inscrire en
  allowlist, et les livrer désarmés les rendrait décoratifs le jour de leur atterrissage.
- **C5** a une liste blanche **vide**, et ce vide est la propriété : une exception
  « ce bras est dispensé d'entrée d'inventaire » recréerait exactement le défaut que la garde
  interdit — un inventaire qui ment. L'implémentation l'exécute à l'étape 4, **avant** le
  reste : si elle est rouge sur l'arbre actuel, l'implémentation **s'arrête et remonte à
  l'opérateur** avec le diff des deux ensembles. Un inventaire déjà divergent est un fait
  d'exploitation (le `match` et la constante ont bougé séparément depuis `2b5456cc`), pas une
  donnée à exempter.

Relevé daté à l'appui de la halte : au 2026-10-15, `ROUTABLE_TRIGGERS` (7 entrées) et les bras
du `match` (`dispatcher.rs:722-737` : les mêmes 7) **coïncident**. La garde est donc attendue
verte à l'atterrissage, et la halte est une disposition de sûreté, non un pronostic.

Les deux gardes manquantes de #2446 (D-2, D-3) ne sont pas livrées ici : voir § Hors
périmètre. Leur absence est rendue lisible (C3) plutôt que comblée en passager clandestin.

---

## Hors périmètre

- **Garde #1742 inchangée (AC5).** `create_recurring_task_if_absent`, `RECURRING_ZOMBIE_GRACE_*`,
  les tests `mika1742_*` / `mika2337_*` de `db::tests::recurring_tasks` et le bras
  `UnknownTrigger` de `engine.rs:5134` ne sont pas modifiés. Un `failed` ou un `expired` récent
  continue de bloquer. La demande initiale du ticket (« ignorer les instances terminales ») est
  **retirée** : elle aurait désarmé la garde dans le cas pour lequel elle existe.
- Supprimer le `TaskEngine` du TUI, ou mettre en place un bail de moteur unique par base.
- Rendre lisible au démarrage la version du binaire en exécution — **déjà livré** par #2446
  (`binary_attribution()`, attestation `task_engine_trigger_registry`).
- Les tâches `run_skill` par `skill_name` dont le skill manque au registre du TUI : autre
  forme, non observée ici, explicitement non traitée par C1 étape 2.
- Protéger contre un TUI **déjà** périmé : le fix ne vaut que pour les binaires qui le
  contiennent. Un binaire ne peut pas corriger celui qui le précède.
- **Les deux gardes de #2446 absentes de l'arbre** (`mika2446_chaque_trigger_routable_a_un_
  armement`, D-2 ; `mika2446_le_stop_precede_le_jeton_et_git_dans_le_reaper`, D-3) et sa sonde
  exhaustive `mika2446_chaque_trigger_routable_est_tire`. Elles relèvent de la sûreté d'une
  sonde destructive (`dispatch_worktree_reap`), pas de la routabilité au claim. Ce plan rend
  leur absence lisible (C3) et la remonte dans le corps de la PR ; leur livraison est une
  décision de périmètre qui appartient à l'opérateur, pas un geste de poseur.

---

## Mesure post-déploiement

Après `make deploy` et un redémarrage du TUI, la question « ce fix sert-il ? » se lit sans
grep sur des giga-octets :

```
grep -c 'task_trigger_not_routable_here' ~/.mika/agents/mika-dev/logs/mika.log.$(date +%F)
```

- **0 et aucun `recurring_unknown_trigger`** — nominal : les deux binaires sont alignés.
- **≥ 1** — un TUI est en retard sur le spirit, et la récurrence a **survécu**. La ligne porte
  `trigger` et l'inventaire du binaire : la comparaison donne la lecture directement.
- **Un `recurring_unknown_trigger` dans le log de l'agent** (et non dans `server.log`) après
  déploiement — le saut n'a pas mordu. C'est la contre-mesure du fix.

---

## Definition of Done

- Le saut `cli_mode` est en place dans `fire_task`, borné aux `run_skill` à `trigger` intégré,
  et ne s'applique que sur preuve positive de non-routabilité.
- Le WARN `task_trigger_not_routable_here` est émis au plus une fois par `task_id` et par
  processus.
- La garde de bijection `mika2446_l_inventaire_est_la_projection_exacte_du_match` existe et est
  verte ; les deux renvois qui l'annonçaient (`dispatcher.rs:55`, module-doc D-1) sont devenus
  vrais.
- Le doc-comment de `process_name` et le module-doc du fichier de test ne portent plus
  d'affirmation fausse.
- C4-a, C4-b, C4-c verts ; `mika1742_*`, `mika2337_*`, `mika2446_*` et
  `test_cli_mode_skips_callback_dispatch` verts.
- Rouge-avant de C4-a et les deux contrôles négatifs de C5 consignés dans le corps de la PR.
- `cargo clippy -D warnings`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`
  verts.
- Le corps de la PR porte le constat sur D-2/D-3/sonde exhaustive de #2446.

---

## Acceptance criteria

Transcrits tels quels depuis le corps de senara-solutions/mika#2351 (re-cadré le 2026-09-19).

1. **AC1 — pas de claim sans route, côté CLI.** Dans `TaskEngine::fire_task` (`engine.rs:3727`), quand `dispatcher.cli_mode` est vrai, une tâche `action_type = run_skill` portant un `trigger` intégré que ce binaire ne sait pas router n'est pas réclamée. Après le tick, sa ligne est inchangée (`status`, `fired_at`, `next_fire_at`, `updated_at`). → C1, C4-a. *(La citation de ligne du ticket est de septembre ; `fire_task` est à `engine.rs:4985` au 2026-10-15.)*
2. **AC2 — source unique.** Les bras de `dispatch_run_skill` et le prédicat de routabilité dérivent d'une seule définition. Un test de garde échoue si un bras existe sans entrée dans le prédicat, ou inversement. → C5. *(La définition unique — `ROUTABLE_TRIGGERS` / `is_routable_trigger` — a été livrée par #2446 ; la garde de bijection, non. C'est elle que ce plan livre.)*
3. **AC3 — lisibilité.** Une tâche ignorée émet un WARN nommé (`event = "task_trigger_not_routable_here"`, avec `task_id`, `label`, `trigger`), au plus une fois par `task_id` et par processus. → C2, C4-c
4. **AC4 — tests.** (a) Moteur `cli_mode: true` : une récurrence due, avec un trigger inconnu du binaire de test, reste `recurring_active` et **jamais** `failed` après un tick. **Contrôle positif** dans le même test : une récurrence due avec un trigger routable est réclamée et tirée. (b) Moteur `cli_mode: false` : le test existant `mika2337_un_trigger_inconnu_meurt_nomme_audite_et_marque` reste vert sans modification. → C4-a, C4-b. *(AC4b : le test tourne aujourd'hui en `cli_mode: true` — sa bascule en mode daemon est une modification de harnais de cinq lignes, assertions inchangées. Voir D3.)*
5. **AC5 — #1742 et #2337 inchangées (non-objectif explicite).** Le prédicat de `create_recurring_task_if_absent`, ses tests `mika1742_*` / `mika2337_*` et le bras `UnknownTrigger` du moteur ne sont pas modifiés. Un `failed` ou un `expired` récent continue de bloquer. La demande initiale de ce ticket (« ignorer les instances terminales ») est **retirée** : elle aurait désarmé la garde dans le cas pour lequel elle existe. → Hors périmètre

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/task_engine/engine.rs` — saut + champ + WARN (C1, C2) | 50 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — doc-comment rectifié (C3) | 6 |
| `crates/mika-agent/src/task_engine/engine.rs` — test de dédup (C4-c) | 30 |
| tests (`tests/eval/test_recurring_trigger_wiring_2337.rs`) — C4-a | 70 |
| tests (`tests/eval/test_recurring_trigger_wiring_2337.rs`) — C4-b, bascule daemon | 6 |
| tests (`tests/eval/test_recurring_trigger_wiring_2337.rs`) — C5, bijection | 30 |
| tests (`tests/eval/test_recurring_trigger_wiring_2337.rs`) — module-doc rectifié (C3) | 12 |

Total estimé : 204 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et voici pourquoi il tient.** 204 < 243, avec 39 lignes de marge. Le plan de
septembre estimait plus large parce qu'il prévoyait de créer la source unique
(`ROUTABLE_TRIGGERS`, `routable_triggers_csv`, `is_routable_trigger` et leurs doc-comments) :
#2446 l'a livrée entre-temps, et ce livrable disparaît du périmètre (§ Relevé). Trois autres
facteurs serrent le volume, et chacun est vérifié sur l'arbre plutôt que supposé :

- `test_dispatcher` est **déjà** en `cli_mode: true` — C4-a n'ajoute aucun paramètre au
  harnais.
- La bascule de C4-b réutilise le pattern `Arc<TaskDispatcher> { cli_mode, ..try_unwrap }`
  déjà en service à `engine.rs:6340`, et `fire_recurring_with` existe déjà : six lignes, aucun
  appelant existant touché.
- C5 réutilise `match_arm_triggers()` (`tests/eval/…:365`), qui parse déjà le bloc `match` pour
  la garde de classe de #2337. La garde de bijection est une comparaison ensembliste
  par-dessus, pas un nouveau parseur.

Les 50 lignes de production sont denses en doc-comment (convention du fichier : chaque garde
`cli_mode` y porte le raisonnement de son `!cli_mode`), et c'est délibéré — c'est ce qui a
permis de retrouver #264 et #2575 comme précédents un an plus tard. Aucune suite n'est
renvoyée à un vol ultérieur : le périmètre entier des cinq AC tient dans cette PR.
