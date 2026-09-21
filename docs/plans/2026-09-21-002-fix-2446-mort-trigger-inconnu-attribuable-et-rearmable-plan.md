# Plan — mika#2446 : une mort par trigger inconnu devient attribuable, et une récurrente corrigée redevient ré-armable

- **Ticket** : senara-solutions/mika#2446
- **Type** : fix (substrat, p1)
- **Branche** : `fix/2446/p1-worktree-reap-clean-auto-2420-meurt`
- **Date** : 2026-09-21
- **Lignée** : mika#2420 (le reaper), mika#2337 (la levée de veto), mika#1742 (le veto),
  mika#2066 (l'empreinte de build), mika#2360 (l'inspecteur de registre), mika#2340
  (la classe « déployé ≠ exécuté »)

---

## Contexte

### Le symptôme, mesuré

La récurrente `worktree_reap` (mika#2420) est morte deux fois sur deux
redémarrages, avec le même message :

```
action_type=run_skill  action_config={"trigger":"worktree_reap"}
status=failed  result="unknown run_skill trigger: worktree_reap"
metadata={"unknown_trigger_death":1}   next_fire_at=(vide)
```

Conséquence : le reaper ne tourne pas, les worktrees de PR terminale ne sont pas
fauchés, et la seule protection du disque de la forge est la purge manuelle de
l'orchestrateur — dont mika#2420 a mesuré la demi-vie à **trois heures**.

### Trois rectifications, et elles sont le premier livrable

Le ticket et ses deux commentaires posent trois hypothèses. La lecture du code en
réfute deux et requalifie la troisième. Chacune change le remède, donc chacune est
écrite avant le reste.

**R1 — l'hypothèse « build incrémental stale » est structurellement invalide en
Rust.** Le commentaire 1/2 suppose « un objet compilé de `dispatcher.rs` périmé :
le binaire a été lié depuis un `.o` d'AVANT l'arm ». Cela ne peut pas se produire
ici. La registration (`crates/mika-agent/src/server/mod.rs:1741`) et le bras de
match (`crates/mika-agent/src/task_engine/dispatcher.rs:596`) sont **dans la même
crate** `mika-agent`. En Rust l'unité de compilation est la *crate*, pas le
fichier : il n'existe aucun `.o` par fichier qu'on pourrait lier périmé, et une
modification de `dispatcher.rs` invalide l'empreinte de toute la crate, qui est
recompilée intégralement. Plus fort encore : `crates/mika-common/build.rs` déclare
`cargo::rerun-if-changed=<git-dir>/HEAD`, donc un HEAD qui bouge recompile
`mika-common` — et par dépendance `mika-agent`. **Un binaire qui affiche
`fc96a341` ne peut donc pas porter un `dispatcher.rs` antérieur.** Le rebuild
propre du commentaire 2/2 ne pouvait rien corriger ; il n'a d'ailleurs rien
prouvé, la garde mika#1742 ayant empêché tout tir.

**R2 — le refus observé au redémarrage de 04:00Z est le comportement NOMINAL de
mika#2337, pas un second défaut.** Le commentaire 2/2 le lit comme un blocage
inattendu. C'est le design, écrit mot pour mot dans
`crates/mika-agent/src/db/tasks.rs:139-147` et dans la doctrine : *« The exemption
is **spent** when honoured … a second unknown-trigger death on the same label
inside the window meets a fully armed veto. The lift buys one restart, not
immunity. »* La trajectoire mesurée est exactement celle-là : mort à 00:10 →
marqueur posé → redémarrage 02:01 consomme la levée unique → ré-enregistrement →
seconde mort à 02:10 → veto pleinement armé pour 24 h. **Il n'y a donc rien à
corriger dans le veto.** Ce qui manque est sa **porte de sortie opérateur**, et
c'est précisément le point (2) de la demande du commentaire 2/2.

**R3 — le source est cohérent, et c'est prouvé par un test qui passe sur ce
checkout.** Le guard de classe de mika#2337 asserte que *tout trigger enregistré
a un bras dans le dispatcher*, en s'ancrant sur les appels à
`task_engine::ensure_recurring_task`. `worktree_reap` est enregistré par ce
chemin, donc il est dans la population. Les cinq tests sont verts :

```
mika2337_tout_trigger_enregistre_a_un_bras_dans_le_dispatcher ... ok
mika2337_la_garde_de_classe_a_une_population_non_vide ... ok
mika2337_les_faux_membres_restent_hors_de_la_population ... ok
mika2337_la_recurrence_qa_review_reconcile_ne_tombe_pas_dans_le_catch_all ... ok
mika2337_un_trigger_inconnu_meurt_nomme_audite_et_marque ... ok
```

La divergence n'est donc **pas intra-binaire**. Elle appartient à la classe que
mika#2337 a nommée comme suivi non traité, en toutes lettres : *« Honest scope:
these close the intra-binary divergence; none of them would have caught the
2026-09-16 incident, which is a skew between merged and running code. »*
**mika#2446 est la deuxième occurrence de cette classe**, après
`qa_review_reconcile` le 2026-09-16.

### Le défaut réel : la ligne affirme sa cause sans la porter

La ligne de mort (`crates/mika-agent/src/task_engine/engine.rs:4260`) écrit :

> `"mika#2337: run_skill trigger registered but not routable by this binary — this
> is a version skew between the merged code and the running code, not a defect of
> the dispatch itself"`

C'est une **affirmation sans preuve dans la ligne elle-même**. Elle porte
`task_id`, `label`, `trigger`, `trigger_type` — et ni la version du binaire, ni
son empreinte git, ni son pid, ni l'inventaire des triggers qu'il sait router. La
surface opérateur de mika#2337 prescrit *« do not touch the dispatcher — establish
the version of the running binary instead »* **sans fournir l'instrument pour le
faire**.

Le coût est mesuré dans ce ticket même : l'opérateur a dû recourir à
`/proc/<pid>/exe`, `strings | grep -c`, et `mika --version` — puis a formulé une
hypothèse (R1) qu'un seul champ de la ligne aurait réfutée immédiatement. Quatre
heures, deux redémarrages, un `cargo clean`, et le verdict reste inobservable.

Deux trous concourants aggravent l'aveuglement :

1. **Le démarrage n'atteste pas sa propre version dans le journal.**
   `server/mod.rs:762` appelle `print_banner("mika-spirit", env!("CARGO_PKG_VERSION"))` —
   la version sémantique **seule**, sans `GIT_HASH`, et sur **stdout**, pas dans
   `$MIKA_SPIRIT_LOG_FILE`. Or mika#2066 a livré `mika_common::build_info::version_string()`
   exactement pour cela (*« a deploy is then verified by interrogating the binary,
   not only by reasoning about provenance »*) et `bundled_skills.rs` s'en sert déjà
   pour le sidecar `.manifest-writer`. Le moteur de tâches, lui, ne s'en sert nulle
   part.

2. **Rien ne dit quel *processus* a tué la ligne.** `mika chat` construit un
   `TaskEngine` complet sur la même base (`crates/mika-cli/src/commands/chat.rs:223`),
   et `cli_mode` ne court-circuite que la livraison des callbacks et une partie de
   la recovery (`engine.rs:524`, `engine.rs:731`) — **jamais le dispatch
   `run_skill`**. Un `mika chat --agent mika-dev` servi par un binaire antérieur
   tirerait donc la récurrente et la tuerait en `UnknownTrigger`. Les preuves du
   ticket n'ont vérifié que le pid de `mika-spirit`. **Cette population n'est ni
   établie ni écartée**, et ce plan ne prétend pas la trancher : il rend la
   question mesurable en un champ.

### Ce que ce plan fait, et ce qu'il ne fait pas

Il **ne corrige pas** le routage — R3 établit qu'il n'y a rien à y corriger. Il
**ne désarme pas** mika#1742. Il **ne prétend pas** identifier le binaire fautif
du 21/09, dont l'instant est passé et dont le processus n'existe plus.

Il rend la classe **attribuable localement** (la ligne porte sa propre preuve),
**observable au démarrage** (la version en exécution est dans le journal),
**couverte par une sonde qui tire réellement chaque trigger**, et **réparable sans
attendre 24 h ni éditer la base**.

---

## Requirements

- **R-1 — La mort porte sa preuve.** L'événement `recurring_unknown_trigger` et sa
  ligne `audit_events` portent la version et l'empreinte git du binaire qui
  refuse, le pid et le nom du processus, et l'inventaire des triggers que ce
  binaire sait router. Lire la ligne doit suffire à trancher entre « binaire
  ancien » et « défaut de routage », sans `strings` ni `/proc`.
- **R-2 — Le démarrage atteste la version en exécution**, dans
  `$MIKA_SPIRIT_LOG_FILE`, avec l'inventaire des triggers routables. Son
  **absence** est elle-même de l'information : elle dit que le binaire servi est
  antérieur à ce correctif (classe mika#2340).
- **R-3 — Une sonde tire réellement chaque trigger enregistré** par le chemin
  récurrent (`ensure_recurring_task` → `TaskEngine::tick` → `fire_task` →
  `dispatch`), sans réseau et sans effet de bord destructif, et échoue si l'un
  d'eux tombe dans le catch-all. Ajouter un trigger sans décider comment le rendre
  hermétique doit **faire rougir un test**, jamais passer en silence.
- **R-4 — Un geste sanctionné ré-arme une récurrente après correction**, sans
  attendre la fenêtre de grâce et sans édition manuelle de la base. Il est tracé,
  per-label, et ne lève le veto que pour la ligne visée.
- **R-5 — Le ré-armement ne peut pas rejouer le défaut.** Il refuse de ré-armer un
  label dont le trigger n'est pas routable par le binaire qui exécute la commande.
- **R-6 — L'inventaire des triggers routables a un site unique**, et une garde
  refuse qu'il diverge du `match` qui décide réellement. Un inventaire qui mentirait
  serait strictement pire que pas d'inventaire.
- **R-7 — Aucun comportement de dispatch, de veto ou de reaper n'est modifié.** Le
  périmètre est l'attribution, l'observabilité, la couverture de test et la
  réparabilité.

---

## Approche / Conception

### C-1 — `ROUTABLE_TRIGGERS` : un inventaire, projection du `match`, jamais son remplaçant

Une constante `pub const ROUTABLE_TRIGGERS: &[&str]` dans
`crates/mika-agent/src/task_engine/dispatcher.rs`, à côté du `match` de
`dispatch_run_skill`.

**Le `match` reste la vérité d'exécution.** On ne le remplace pas par une table de
dispatch dynamique : ce serait déplacer la décision dans une donnée, et une donnée
périmée ne fait pas rougir un compilateur. La constante est une **projection**, et
son honnêteté est tenue par une garde de source qui parse le bloc `match` et
asserte l'égalité ensembliste avec la constante. Le guard de classe mika#2337
parse déjà ce bloc (`call_arguments`, `DISPATCHER_SRC`) : l'extraction est réutilisée,
pas réécrite.

**Refusé : dériver l'inventaire au runtime.** Il n'y a pas de réflexion sur un
`match` en Rust, et toute dérivation serait une seconde liste écrite à la main —
c'est-à-dire le problème, avec une étape de plus.

### C-2 — R-1 : la ligne de mort devient une preuve

Au site `engine.rs:4223`, l'événement `recurring_unknown_trigger` gagne :

| champ | valeur | ce qu'il tranche |
|---|---|---|
| `binary_version` | `build_info::VERSION` | la version sémantique servie |
| `binary_git_hash` | `build_info::GIT_HASH` | **le champ décisif** — si c'est le commit qui porte l'arm, le skew est réfuté et la cause est ailleurs |
| `process_id` | `std::process::id()` | quel processus a refusé |
| `process_name` | `std::env::current_exe()` (nom de fichier) | `mika-spirit` ou `mika` — sépare le démon d'un `mika chat` sur la même base |
| `routable_triggers` | `ROUTABLE_TRIGGERS.join(",")` | l'inventaire **de ce binaire**, à comparer au trigger refusé |

Les mêmes valeurs sont portées par le `reasoning` de la ligne `audit_events`, pour
que la question soit répondable en SQL sans grep sur dix-neuf gigaoctets.

`GIT_HASH` peut valoir `"unknown"` (build hors checkout, couche Docker) —
`build_info` le documente. C'est **rapporté tel quel, jamais corrigé** : `"unknown"`
signifie « ce binaire ne peut pas énoncer sa provenance », ce qui est une réponse,
pas une absence de réponse.

**Ce que ce champ ne peut pas faire, nommé.** Il atteste la version du binaire qui
**refuse**, jamais celle qui a *enregistré* la récurrente — les deux peuvent
différer, et c'est précisément l'hypothèse du ticket. Le plan ne stampe pas la
version à l'enregistrement : ce serait une seconde écriture sur un chemin nominal
pour une population pathologique, et l'inventaire du refusant suffit à trancher.

### C-3 — R-2 : l'attestation de démarrage

Un `info!` dans `run_server`, **sur le chemin nominal** (à côté du banner, avant
l'initialisation des agents) :

```
event = "task_engine_trigger_registry"
version, git_hash, routable_triggers, process_id
```

Émis **une fois par processus**, sur toutes les branches, y compris saine —
doctrine mika#2293 (*« un réglage qu'on ne peut pas observer n'est pas un réglage,
c'est un espoir »*). Le banner sur stdout est **conservé tel quel** : il sert un
autre public et le modifier n'apporte rien.

**Son absence est de l'information.** Un journal qui tourne sans cette ligne dit
que le binaire servi est antérieur au correctif — établir le déploiement avant de
toucher au code, classe mika#2340.

### C-4 — R-3 : la sonde de tir exhaustive, et son herméticité par trigger

`fire_recurring` (dans `tests/eval/test_recurring_trigger_wiring_2337.rs`) est
**déjà paramétrée** par label et `action_config`, et panique si le moteur n'a
jamais fait feu — elle ne peut donc pas être verte sans avoir tiré. Il reste à
l'appliquer aux sept triggers au lieu d'un.

La difficulté réelle n'est pas la boucle, c'est l'**herméticité**, et elle diffère
par trigger. Le point de sûreté central :

> **`worktree_reap` SUPPRIME des répertoires.** Une sonde qui résoudrait un jeton
> sur la machine d'un opérateur partirait sur le réseau et pourrait faucher de
> vrais worktrees. L'absence de jeton ne peut pas être la seule ceinture : elle
> dépend d'un `Settings::load` qui lit l'environnement.

Donc, pour ce trigger, la sonde **arme le STOP** (`auto_pull_stop::WORKTREE_REAP_SCAN`,
fichier sentinelle sous un `global_home_dir` temporaire). Ce court-circuit est en
**tête** de `dispatch_worktree_reap` — avant la résolution du jeton et avant tout
`git` (`dispatcher.rs:1859`) — donc même un jeton qui fuiterait ne toucherait rien.
Herméticité **par construction**, pas par absence.

La sonde est structurée par une table `(trigger, armement d'herméticité)` et le
test **échoue si un membre de `ROUTABLE_TRIGGERS` n'a pas d'entrée**. C'est le
patron « `match` exhaustif sans bras `_ =>` » de la maison transposé à une donnée
de test : ajouter un trigger force à décider comment le rendre hermétique, au lieu
de le laisser silencieusement hors couverture.

**Assertion, et pourquoi ce n'est pas « la tâche est replanifiée ».** Les sept
triggers ont des pré-filtres différents (budget proactif, jeton, répertoire de
dépôt, STOP, requête DB pure) et certains peuvent légitimement rendre `Err` dans
un harnais hermétique. L'assertion porte donc sur **la mort par catch-all**, seule
chose que la sonde prétend mesurer :

- le `result` ne contient pas `unknown run_skill trigger` ;
- `count_audit_events_by_tool_name("recurring_unknown_trigger") == 0`.

Le prédicat d'arrêt reste `next_fire_at.is_some() || status == "failed"`, de sorte
que la boucle ne peut pas sortir sans que le moteur ait statué.

Le contrôle négatif existant (`zorglub`) est conservé : sans lui, une sonde qui ne
tirerait rien serait verte pour la mauvaise raison.

### C-5 — R-4/R-5 : `mika tasks rearm <label>`

`mika tasks` opère déjà directement sur `ctx.async_db` (`commands/tasks.rs:44`), et
`cancel` y est le précédent d'un geste opérateur destructif. `rearm` s'y insère
sans nouvelle surface.

**Le mécanisme réutilise le patron mika#2271 plutôt que d'en inventer un.** La
garde `dead_sibling` (`db/tasks.rs:177-205`) exclut déjà les lignes portant
`RECURRING_CONFIG_CANCEL_REVERTED_PATH`. On ajoute un marqueur jumeau,
`RECURRING_OPERATOR_REARM_PATH = "$.operator_rearm"`, posé sur la **ligne morte
visée** et exclu par la même requête.

Conséquences de ce choix, toutes voulues :

- Le statut terminal n'est pas réécrit — la ligne morte reste un fait daté.
- La levée est **per-label et per-ligne** : mika#1742 reste armé partout ailleurs.
- Elle est **durable** : elle survit au processus, contrairement à un état en
  mémoire.
- Elle **vieillit avec la ligne** : passé la fenêtre de grâce, la ligne sort de la
  requête de toute façon, donc le marqueur ne laisse pas de dette.

Séquence de la commande :

1. Résoudre la ligne récurrente la plus récente du `(agent, label)` en état
   terminal. **Absente → erreur explicite, jamais de création ex nihilo** : créer
   une récurrente à partir d'un label inconnu serait le seul geste capable
   d'introduire un trigger non routable.
2. Lire son `action_config`, en extraire le `trigger`. **R-5 : si ce trigger n'est
   pas dans `ROUTABLE_TRIGGERS`, refuser**, en nommant le trigger, la version et
   l'empreinte du binaire courant, et l'inventaire. Ré-armer une récurrente qu'on
   sait non routable ne produirait qu'une troisième mort.
3. Poser le marqueur, écrire une ligne `audit_events`
   (`tool_name = 'recurring_operator_rearm'`).
4. Ré-enregistrer immédiatement via `ensure_recurring_task`, avec le `cron_expr`
   lu **sur la ligne morte** — l'opérateur ne doit pas retaper un cron, et le
   prochain démarrage réconcilie de toute façon.

**Refusé : une exemption automatique au second décès.** Ce serait désarmer
mika#1742 pour la classe entière, c'est-à-dire supprimer la garde au motif qu'elle
a gêné une fois. Le ré-armement est un **acte** — tracé, explicite, imputable.

**Refusé : un endpoint HTTP.** mika#2360 a livré la lecture
(`GET /api/v1/recurring-tasks`, avec `zombie_veto_active`) ; le pendant en écriture
est une surface publique pour un geste d'opérateur local, et `mika tasks cancel` a
déjà tranché ce choix dans l'autre sens. Un endpoint reste possible plus tard
**par-dessus** cette commande ; l'inverse ne serait pas vrai.

**Coût nommé.** Un `rearm` sur un label désactivé par knob (`MIKA_WORKTREE_REAP=0`)
ressuscite la ligne jusqu'au prochain démarrage, qui l'annulera de nouveau. C'est
cohérent — le knob est un levier de démarrage — et sans conséquence : le tick
intermédiaire court-circuite ou s'abstient.

---

## Phases d'implémentation

### Phase 1 — L'inventaire et sa garde (C-1, R-6)

1. `ROUTABLE_TRIGGERS` dans `dispatcher.rs`, adjacent au `match`.
2. Garde de source dans `test_recurring_trigger_wiring_2337.rs` : égalité
   ensembliste entre les bras littéraux du `match` et la constante, en réutilisant
   l'extracteur existant. Contrôle de bonne foi : la population extraite est non
   vide.

### Phase 2 — L'attribution (C-2, C-3, R-1, R-2)

3. Champs d'attribution sur le `warn!` et sur le `reasoning` de la ligne
   `audit_events`, au site `engine.rs:4223`.
4. `task_engine_trigger_registry` au démarrage de `run_server`.
5. Tests : le contrôle négatif `zorglub` existant atteste désormais la présence et
   la non-vacuité des champs d'attribution ; un test asserte que
   `binary_git_hash` n'est jamais vide (jumeau de `build_info::tests::git_hash_is_never_empty`).

### Phase 3 — La sonde exhaustive (C-4, R-3)

6. Table `(trigger, armement)` et boucle de tir sur `ROUTABLE_TRIGGERS`.
7. Armement STOP pour `worktree_reap` (ceinture anti-destruction), jeton absent
   pour les scans de forge, provider factice pour les tours silencieux.
8. Test de complétude : tout membre de `ROUTABLE_TRIGGERS` sans entrée dans la
   table fait rougir.

### Phase 4 — Le ré-armement (C-5, R-4, R-5)

9. `RECURRING_OPERATOR_REARM_PATH` dans `db.rs`, exclusion dans la requête
   `dead_sibling` de `db/tasks.rs`, fonction `rearm_recurring_task`.
10. Sous-commande `TaskCommand::Rearm { label }` dans `cli.rs` + `commands/tasks.rs`.
11. Tests `db::tests` : le marqueur lève le veto pour son label seul ; **tout autre
    décès continue d'armer le veto** (jumeau de
    `mika2337_any_other_death_still_arms_the_veto`) ; un trigger non routable est
    refusé.

### Phase 5 — Documentation

12. `crates/mika-agent/CLAUDE.md` § *Unknown-Trigger Veto Lift* : l'attribution,
    le geste de ré-armement, et la mise à jour de la surface opérateur — la phrase
    *« establish the version of the running binary instead »* nomme désormais
    l'instrument qui le permet.
13. `CLAUDE.md` racine : `mika tasks rearm` dans la liste des commandes.

---

## Contrat de vérification

- `cargo test -p mika-agent --test eval recurring_trigger` — les cinq gardes
  existantes restent vertes, les nouvelles passent.
- `cargo test -p mika-agent db::tests::mika2337` et `::mika2446` — veto et
  ré-armement.
- `cargo test -p mika-cli` — surface CLI.
- `make lint && make fmt && make test`.
- **Contrôle négatif obligatoire** : retirer temporairement le bras
  `"worktree_reap" =>` doit faire rougir *à la fois* la garde d'inventaire (C-1) et
  la sonde de tir (C-4). Une sonde qui resterait verte ne mesurerait rien — c'est
  l'erreur que ce ticket existe pour ne pas rejouer.

### Sondes post-déploiement, et leurs haltes

**Sonde A — le reaper tire enfin.** Après `mika tasks rearm worktree_reap`, le
prochain tick (≤ 10 min) doit produire un tir sans mort. Lecture :
`grep worktree_reap_tick $MIKA_SPIRIT_LOG_FILE` et
`SELECT * FROM audit_events WHERE tool_name = 'worktree_reaped' ORDER BY created_at DESC;`

**Sonde B — l'attribution est lisible.**
```bash
grep task_engine_trigger_registry "$MIKA_SPIRIT_LOG_FILE" \
  | jq '{version, git_hash, routable_triggers, process_id}'
```
Doit porter `worktree_reap` dans l'inventaire. **Halte** : aucune ligne alors que
le démon tourne ⇒ le binaire servi est antérieur au correctif — établir le
déploiement **avant** toute conclusion sur le routage (classe mika#2340).

**Sonde C — la classe reste vide.**
```sql
SELECT COUNT(*) FROM audit_events WHERE tool_name = 'recurring_unknown_trigger';
```
**Régime attendu : zéro nouvelle ligne.** Toute occurrence est désormais
**attribuable** : lire `binary_git_hash`, `process_name` et `routable_triggers` de
la ligne.
- Le hash est celui qui porte l'arm **et** le trigger est dans l'inventaire ⇒ la
  ligne se contredit : **halte**, c'est un vrai défaut de routage et *là* seulement
  le dispatcher est en cause.
- Le trigger est **absent** de l'inventaire ⇒ le skew est confirmé et daté ; le
  remède est un déploiement, jamais une modification du dispatcher.
- `process_name` vaut `mika` et non `mika-spirit` ⇒ **c'est la population que ce
  ticket n'a pas su écarter** : un processus CLI tire les récurrentes de la même
  base. Ne pas élargir le dispatch — ouvrir le ticket de suivi sur le périmètre du
  `TaskEngine` en `cli_mode`.

**Sonde D — le ré-armement reste un acte rare.**
```sql
SELECT target_key, count(*) FROM audit_events
 WHERE tool_name = 'recurring_operator_rearm' GROUP BY 1 ORDER BY 2 DESC;
```
Un même label ré-armé plusieurs fois signifie que la correction n'a pas pris :
**ne pas ré-armer une troisième fois par réflexe**, lire d'abord la sonde C.

---

## Definition of Done

- [ ] `ROUTABLE_TRIGGERS` existe, site unique, garde de cohérence avec le `match` verte.
- [ ] `recurring_unknown_trigger` (log + `audit_events`) porte version, empreinte
      git, pid, nom de processus et inventaire.
- [ ] `task_engine_trigger_registry` est émis une fois par démarrage de mika-spirit.
- [ ] Les sept triggers enregistrés sont tirés par le chemin récurrent dans la
      sonde ; un trigger sans armement d'herméticité fait rougir.
- [ ] La sonde `worktree_reap` est hermétique **par le STOP**, pas par l'absence de
      jeton.
- [ ] `mika tasks rearm <label>` lève le veto per-label, trace l'acte, ré-enregistre,
      et refuse un trigger non routable.
- [ ] mika#1742 reste armé pour toute autre cause de décès (test).
- [ ] `make lint && make fmt && make test` verts ; contrôle négatif vérifié rouge.
- [ ] `crates/mika-agent/CLAUDE.md` et `CLAUDE.md` à jour.

---

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria`. Les critères ci-dessous
sont dérivés du travail explicitement demandé dans le commentaire 2/2 — *« (1) la
cause du UnknownTrigger alors que le binaire porte l'arm, avec un test qui tire
réellement le déclencheur par le chemin récurrent ; (2) un moyen sanctionné de
ré-armer une récurrente après correction, sans attendre 24 h ni éditer la base »* —
et des Requirements ci-dessus.

- **AC1** — Le plan établit la cause par lecture du code et **rectifie** les trois
  hypothèses du ticket : build-stale structurellement invalide (R1), refus au
  redémarrage nominal et non défectueux (R2), source cohérent attesté par un test
  vert (R3).
- **AC2** — Une mort `UnknownTrigger` est attribuable **depuis la ligne seule** :
  version, empreinte git, pid, nom du processus et inventaire des triggers
  routables, dans le journal et dans `audit_events`. Aucun recours à `strings` ni
  à `/proc` n'est nécessaire pour trancher entre skew de version et défaut de
  routage.
- **AC3** — La version réellement en exécution est lisible dans
  `$MIKA_SPIRIT_LOG_FILE` au démarrage, avec l'inventaire des triggers routables.
- **AC4** — Une sonde tire **chaque** trigger enregistré par le chemin récurrent
  réel (`ensure_recurring_task` → `tick` → `fire_task` → `dispatch`), sans réseau,
  et échoue si l'un tombe dans le catch-all. La sonde de `worktree_reap` ne peut
  supprimer aucun worktree, et cette impossibilité est structurelle (STOP en tête
  de dispatch), pas circonstancielle.
- **AC5** — Ajouter un trigger sans décider de son herméticité de test fait
  **rougir un test**, jamais passer en silence.
- **AC6** — `mika tasks rearm <label>` ré-arme une récurrente morte sans attendre
  la fenêtre de 24 h et sans édition manuelle de la base ; l'acte est tracé dans
  `audit_events`.
- **AC7** — Le ré-armement refuse un label dont le trigger n'est pas routable par
  le binaire courant, en nommant le trigger et l'inventaire.
- **AC8** — mika#1742 reste pleinement armé pour toute cause de décès autre qu'une
  levée opérateur explicite ou l'exemption mika#2337 déjà en place ; un test le
  pinne.
- **AC9** — Aucun comportement de dispatch, de veto, de reaper ou de scan
  périodique n'est modifié. Le périmètre est attribution, observabilité, couverture
  et réparabilité.
- **AC10** — La contradiction apparente du ticket (« le binaire porte l'arm et
  pourtant le catch-all mord ») est **rendue falsifiable** par les sondes C, avec
  une halte explicite pour chacune des trois lectures possibles — dont la
  population `mika chat` que les preuves du ticket n'ont pas écartée.

---

## Revision history

- **v1 (2026-09-21)** — Rédaction initiale. Diagnostic par lecture du code :
  réfutation structurelle de l'hypothèse build-stale, requalification du refus de
  ré-enregistrement en comportement nominal mika#2337, confirmation de la cohérence
  du source par les cinq gardes vertes. Le défaut est requalifié en **absence
  d'attribution**, et le travail est recentré sur : inventaire à site unique,
  preuve portée par la ligne de mort, attestation au démarrage, sonde de tir
  exhaustive et hermétique, ré-armement opérateur sanctionné.
