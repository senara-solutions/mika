# mika#1727 — vol V2 : l'état local quitte la TUI

- **Ticket :** senara-solutions/mika#1727 (vol V2 sur 4) — la PR porte
  `Refs mika#1727`, **jamais** `Closes` : le ticket se ferme au merge de V4.
- **Base :** `main` à `3f382cb5d` (merge de #3060, vol V1).
- **Branche :** `feat/1727/tui-v2-etat-local-par-mika-spirit`, créée depuis `main`
  au `first-push`. Les branches `feat/1727/tui-tui-as-thin-http-client-of-mika` et
  `feat/1727/spirit-emit-plumbing` restent du matériau : ni reprises, ni rebasées,
  ni supprimées.
- **Dépôt touché :** `senara-solutions/mika` seul. Aucun fichier du plan de
  contrôle (`.github/workflows/`, `.claude/`, `skills/bundled/`,
  `crates/mika-agent/src/well_known_agents.rs`, `.mika/`) et aucun chemin de
  `.github/CODEOWNERS` (vérifié : les sept entrées sont `perimeter/`,
  `server/verdict_handler.rs`, `tools/pr_merge_with_gate.rs`, `docs/gate/`,
  `docs/egress/`, `well_known_agents.rs` et `CODEOWNERS` lui-même — aucun des
  fichiers de ce vol n'y figure).
- **Gel des tickets (jusqu'au 01/12) :** ce vol n'ouvre aucun ticket, ni suivi ni
  sous-ticket. Tout reste est nommé par une ligne
  `Tracked in: senara-solutions/mika#1727` et va en commentaire sur ce ticket ou
  dans la PR.

---

## 0 — L'écart avec le texte du ticket, à juger avant le reste

Cinq écarts. Les trois premiers changent la forme du vol ; les deux derniers sont
des faits mesurés qui contredisent du texte en production.

### 0.1 — Les quatre critères de V2 ne tiennent pas sous la borne : V2 se livre en trois vols

La borne par vol de ce ticket vaut **279 lignes estimées** hors `docs/` et
`CLAUDE.md`. Les quatre critères de V2 pèsent, mesurés site par site au § 1,
environ **780 lignes**. Découpé :

| sous-vol | critère livré | estimation |
|---|---|---|
| **V2a — ce vol** | critère 1 : la TUI ne construit plus de moteur de tâches | **234** |
| V2b | critère 2 : les callbacks arrivent par le SSE, plus par un poll local | ~300 |
| V2c | critère 3 : création de session, `/compact`, `/rewind`, `/undo` par spirit | ~250 |

Le découpage est **en vols, pas en phases d'un même vol** : V2a est mergeable
seul, porte son propre ordre de commit et ses propres planchers, et ne laisse
aucune régression derrière lui (§ D6 : le poll des callbacks continue de
fonctionner inchangé après V2a). Le critère 4 — comportement visible conservé —
est tenu par V2a par **non-intervention** : le diff ne touche ni la création de
session, ni `/compact`, ni `/rewind`, ni `/undo`, ni `/clear` ; il est porté
activement par V2b et V2c, là où ces chemins bougent.

*Tracked in: senara-solutions/mika#1727* — pour V2b et V2c, qui partiront chacun
sur un `ready` humain après le merge du précédent, règle « un vol à la fois ».

**Pourquoi V2a d'abord, et pas V2b.** Le critère 1 est celui qui porte l'effet
que le ticket nomme comme en cours *aujourd'hui* : la fin de la course entre
l'engine de la TUI et les daemons sur les récurrentes de l'agent (mika#2351,
§ D7). V2b et V2c n'y changent rien. Livrer le critère 1 en premier éteint cette
course au premier merge.

### 0.2 — Le SSE task-events n'est pas inerte : mika#1758 a déjà livré les émetteurs

Le ticket décrit la cible de V2 comme « le SSE des événements de tâche (#1732) ».
Le texte de production dit encore que ce fil est dormant, et c'est **faux depuis
mika#1758** :

- `crates/mika-agent/src/async_db.rs` porte `emit_task_event` et **dix sites
  d'émission** couvrant les six transitions : `TaskCreated` (l. 317, 349),
  `TaskClaimed` (l. 501), `TaskCancelled` (l. 544, 709, 742), `completed`
  (l. 580, 639), `failed` (l. 607), `TaskDelivered` (l. 1184). Trois sites de
  non-émission délibérée portent leur doc-comment (l. 653, 755, 766).
- Le canal est attaché par `OnceLock` **dans le processus serveur seulement**
  (`server/mod.rs:1914`, `server/state.rs:312`). Dans la TUI il est vide, donc
  une écriture côté TUI n'émet rien : le daemon est l'unique diffuseur — qui est
  exactement la topologie d'après-V2.
- Un consommateur SSE complet existe déjà dans ce crate :
  `crates/mika-cli/src/commands/tasks.rs::stream_task_events`, avec son parseur
  de trames (`find_event_separator`, qui traite `\r\n\r\n`, `\n\n` et `\r\r`).
  Son doc-comment dit : *« Diagnostic — not integrated into the TUI. mika#1727
  handles TUI consumption. »*

**Deux commentaires de production mentent encore là-dessus**, et V2b les corrige
en quatre lignes : `server/mod.rs:354-356` (« Wire-only in v1 — emission from
task_engine / transition sites lands in a follow-up ticket ») et le module doc de
`server/tasks_stream.rs` (« the wire is dormant until that lands »). Ce fait est
établi ici plutôt que dans le groom de V2b parce qu'il **réduit** le coût de V2b
et qu'un plan qui le découvrirait en vol rejouerait la mesure.

**Le vrai manque de V2b, lui, est ailleurs** : `TaskCompleted` et `TaskFailed`
ne portent ni `label`, ni `session_id`, ni `trace_id`, ni `parent_task_id`, ni
`original_status` — précisément les cinq champs que
`AgentRequest::CallbackResult` consomme (`tui/app.rs:1696-1702`). Seul
`TaskCreated` porte `label` et `parent_task_id`. V2b tranchera entre champs
additifs sur les trames et un endpoint de détail ; ce plan ne le pré-décide pas.

### 0.3 — Retirer le bloc des récurrentes fait rougir deux détecteurs existants

`crates/mika-agent/src/canonical_tokens.rs` recense
`"crates/mika-cli/src/commands/chat.rs"` dans **deux** périmètres :
`HEARTBEAT_GATE_REGISTRATION_SITES` (l. 1777, mika#2847) et
`REFLECTION_GATE_REGISTRATION_SITES` (l. 2081, mika#2850). Les deux portent une
anti-vacuité qui **exige** que la porte de section soit lue dans chaque entrée
recensée, et une boucle `seen == len`. Retirer la lecture de `chat.rs` sans
toucher au recensement rend quatre tests rouges, avec le message « ce scan vise
un nom mort ».

Le geste correct est de **retirer l'entrée dans le même commit que la lecture** —
jamais de l'ajouter à `HEARTBEAT_GATE_READER_ALLOWED` /
`REFLECTION_GATE_READER_ALLOWED`, livrées vides et assertées vides. § D3 détaille
pourquoi ce retrait **durcit** le détecteur au lieu de le relâcher.

### 0.4 — Le commentaire mika#2329 de `chat.rs` contredit l'observation du ticket sur `auto_pull`

Le ticket dit : *« Depuis 2026-11-08 12:10Z, c'est [l'engine de la TUI] qui
exécute `auto_pull` pour mika-dev. »* Le commentaire de `chat.rs:187-189` dit
l'inverse : *« `auto_pull` ne tourne pas en mode CLI »*.

Les deux se concilient, et le mécanisme est lisible sans mesurer un processus :
`cli_mode` ne garde **pas** le dispatch de la récurrente `auto_pull_groomed`.
Il garde les balayages de parentes terminales (`engine.rs:1011, 1044, 1213, 1445,
1475, 1511`) et le dispatch des callbacks ; le bras
`"auto_pull_groomed" => self.dispatch_auto_pull_groomed(task)`
(`task_engine/dispatcher.rs:1086`) est hors garde. La ligne récurrente vit dans
la base **partagée** par les deux processus, donc tout engine qui tourne un tick
peut la réclamer, TUI comprise. Le commentaire décrit l'**enregistrement** du
feeder, pas le dispatch d'une ligne déjà présente.

Le plan ne corrige pas ce commentaire : il part avec le `TaskDispatcher` entier.
Il le nomme parce qu'un lecteur du diff y verrait sinon une contradiction non
traitée.

### 0.5 — Ce que V2a ne touche pas, et qui aurait pu paraître dû

- **`cli_mode` reste.** Le champ, ses six gardes dans `engine.rs` et le détecteur
  `task_engine/tick_cli_guard_scan.rs` qui les exige restent en place (§ D4).
- **`commands/tasks.rs` reste.** Il importe largement
  `mika_agent::task_engine::*` — mais des utilitaires (`process_kill`,
  `process_liveness`, `rearm_recurring_task`, `worktree_activity`), jamais un
  moteur. Le choix des aiguilles du détecteur en découle (§ D5).
- **Le bras `CallbackResult` et l'allowlist à une entrée de V1 restent.** Ils
  partent avec V2b, qui retire le poll qui les alimente.

---

## 1 — L'état mesuré sur `3f382cb5d`

Mesuré sur la base de ce vol, pas repris du « Découpage » (qui datait de
`94b21916`). Les trois chiffres de taille du ticket sont confirmés ; le
quatrième ne s'y trouvait pas.

| fichier | lignes | ce que le vol y fait |
|---|---|---|
| `crates/mika-cli/src/commands/chat.rs` | 1325 | tout le retrait de V2a |
| `crates/mika-cli/src/tui/app.rs` | 2261 | rien en V2a (V2b) |
| `crates/mika-cli/src/tui/commands/handlers.rs` | 2324 | rien en V2a (V2c) |
| `crates/mika-cli/src/commands/chat_turn.rs` | 600 | le détecteur, dans son module `tests` (l. 205) |
| `crates/mika-agent/src/canonical_tokens.rs` | — | deux entrées de recensement |

**Sites exacts du retrait, dans `commands/chat.rs` :**

| site | lignes | contenu |
|---|---|---|
| import de tête | 32 | `use mika_agent::task_engine::{self, TaskDispatcher, TaskEngine};` |
| champ de `AgentWorker` | 46 | `poller_handle: JoinHandle<()>,` |
| `TaskDispatcher` | 178-216 | 24 champs, dont `cli_mode: true` et le commentaire `#264` |
| `TaskEngine::new` | 218-221 | dans un `Arc<tokio::sync::Mutex<…>>` |
| paire `heartbeat` | 223-239 | `heartbeat_enabled_for_agent` → `ensure_recurring_task` / `cancel_recurring_task_by_label("heartbeat")` |
| paire `reflection` | 240-256 | `reflection_cron_for_agent` → idem avec `"reflection"` |
| `startup_recovery` | 256-262 | sous `task_engine.lock().await` |
| `prune_old_tasks` | 263 | purge des tâches de plus de 30 jours |
| boucle de tick | 264 | `let poller_handle = TaskEngine::spawn_tick_loop(task_engine);` |
| construction | 585 | `poller_handle,` dans le `AgentWorker` |
| arrêts | 828, 935, 1035 | `worker.poller_handle.abort();` |

`commands/chat.rs` **n'a aucun module `#[cfg(test)]`** — c'est mesuré, pas
supposé. Le détecteur va donc dans `commands/chat_turn.rs`, où vivent déjà
`mika1727_v1_*`.

**Côté spirit, pour mémoire de V2b et V2c** (mesuré, à ne pas re-mesurer) :
`POST /api/v1/rewind/{preview,execute,resolve}` existent (jeton interne, sans
CORS) ; `/dashboard/tasks/stream` existe **et émet** ; `/sessions`,
`/sessions/{id}`, `/sessions/{id}/messages` sont en **GET seul** — aucun POST de
création de session ; **aucun endpoint de compaction n'existe**. Le précédent de
montage pour une mutation est `operator_v1_routes` (mika#2506 : une mutation
n'est pas une lecture d'observabilité, donc elle ne va pas dans
`dashboard_routes`).

---

## 2 — Les points de conception

### D1 — Ce que le vol retire, et ce qu'il ne remplace par rien

Le critère 1 est une **soustraction**, et c'est sa qualité : rien n'est réécrit,
rien n'est déplacé, aucun nouveau chemin n'apparaît. Le `TaskDispatcher` de la
TUI est un deuxième exemplaire, divergent par construction, du dispatcher que le
daemon possède : `agent_lock: None`, `pr_reviews_posted: None`, `cli_mode: true`,
et un `global_home_dir` renseigné pour un scan qui ne tourne pas. Le supprimer
ne demande pas de substitut parce que le daemon en a déjà un, complet.

### D2 — Qui enregistre les récurrentes après le retrait, et ce qui se passe sans daemon

`server/mod.rs` enregistre la même paire, **par agent chargé**, au démarrage du
daemon (appel l. 1906, label l. 1915 pour `heartbeat` ; l. 1918 et 1928 pour
`reflection`). Après V2a, c'est le seul enregistreur — ce qui est l'état voulu :
aujourd'hui les deux processus écrivent la même ligne et se disputent son
exécution.

**Sans daemon, la TUI n'enregistre plus rien, et c'est déjà le régime depuis V1.**
`chat::run` lit `dashboard::query_dashboard_status()` et pousse un message
`System` nommant l'endpoint et les deux causes possibles quand le spirit est
muet ; un tour échoue en nommant l'endpoint plutôt qu'en rendant une réponse vide
(mika#2270). Une TUI sans daemon n'a plus de mode autonome depuis `3f382cb5d` :
V2a ne crée pas cette dépendance, il en hérite. Le plan ne livre donc **pas** de
garde neuve au démarrage pour ce motif — elle dirait une seconde fois ce que la
sonde de V1 dit déjà, à un endroit différent.

### D3 — Le retrait des deux entrées de recensement durcit le détecteur, il ne le relâche pas

C'est le point qu'un relecteur pressé lira à l'envers. `mika2847_*` a deux
volets : l'anti-vacuité (« chaque entrée recensée lit-elle la porte ? ») et le
volet **`strangers`** (« un lecteur hors recensement ? » → rouge, avec
« Ne PAS l'ajouter à HEARTBEAT_GATE_READER_ALLOWED (mika#2201) »).

Tant que `commands/chat.rs` est **au** recensement, une lecture de la porte y est
légitime. Dès qu'il en sort, toute réapparition d'une lecture dans
`crates/mika-cli/` devient un **étranger** et rougit. Le retrait de l'entrée est
donc, littéralement, ce qui interdit à la régression de revenir — la doctrine
citée par le détecteur lui-même (« un site qu'on ne veut pas armer est un site à
supprimer »).

Les deux constantes gardent un commentaire nommant le vol et la raison, pour que
le prochain lecteur ne lise pas la suppression comme un relâchement.

### D4 — `cli_mode` reste, et c'est délibéré

Après V2a, plus aucun site de production n'arme `cli_mode: true`
(`dispatcher.rs:6627` écrit `false` ; les autres occurrences sont des tests).
Le champ, ses six gardes dans `engine.rs` et le module
`task_engine/tick_cli_guard_scan.rs` — dont la doctrine dit « la TUI construit
son propre `TaskEngine` (`cli_mode: true`) » — **ne sont pas touchés**.

Trois raisons. (1) Ce nettoyage est hors périmètre : le ticket borne V2 à
`crates/mika-cli/` plus les endpoints manquants sous `server/`. (2) Le détecteur
`tick_cli_guard_scan` exige la présence des gardes ; les retirer dans le même
vol que le retrait du seul armeur serait deux changements de sens opposés dans un
diff. (3) `cli_mode: false` reste la valeur du daemon, donc les gardes restent
correctes — elles perdent leur deuxième population, pas leur sens. La doctrine du
module devient en revanche **périmée** après ce merge ; la PR le dit, sans le
corriger.

*Tracked in: senara-solutions/mika#1727*

### D5 — Les aiguilles du détecteur visent la construction, jamais le module

Un détecteur qui refuserait `mika_agent::task_engine::` dans
`crates/mika-cli/` rougirait immédiatement sur `commands/tasks.rs`, qui en
importe neuf utilitaires légitimes (`process_kill::cancel_task_and_kill`,
`rearm_recurring_task`, `stuck_pending_reaper_grace_secs`,
`promoted_wrapper_liveness_secs`, `worktree_activity::seconds_since_file_write`,
`process_liveness::*`, `engine::HANDLER_FAILURE_METADATA_KEY`). Il faudrait alors
une allowlist pour `tasks.rs` — c'est-à-dire la case où déposer la prochaine
infraction (mika#2323), posée pour rien.

Les aiguilles portent donc sur le **geste de construction et de lancement**, qui
est ce que le critère 1 nomme :

```rust
const NEEDLES: &[&str] = &[
    "TaskEngine::new(",
    "TaskDispatcher {",
    "spawn_tick_loop(",
    "startup_recovery(",
];
```

Quatre aiguilles plutôt qu'une, parce que les quatre gestes sont séparables : on
peut rendre un moteur sans tick (`TaskEngine::new` seul), ou relancer une
récupération sur un moteur emprunté (`startup_recovery` seul). Une aiguille
unique sur `TaskEngine` serait inerte sur trois des quatre régressions.

`prune_old_tasks(` n'est **pas** une aiguille : c'est une purge idempotente qui
ne construit ni ne lance rien, et un futur `mika tasks prune` serait un appelant
légitime depuis ce crate. Le retirer ici est du nettoyage de cohérence (la purge
appartient au processus qui possède le moteur), pas une propriété à garder.

### D6 — Le poll des callbacks survit à V2a, inchangé, et c'est mesurable

`tui/app.rs::poll_callback_tasks` (l. 1596-1704) ne passe par aucun moteur : il
lit `db.get_undelivered_callback_tasks_for_session(&since, &self.session_id)`,
réclame par `db.mark_task_delivered(&task.id)` — atomique, `Ok(true)` / `Ok(false)` —
puis envoie `AgentRequest::CallbackResult` au worker. Retirer le moteur de la TUI
ne touche aucun de ces trois gestes.

**La course que le commentaire `#264` décrit existe déjà, et V2a n'en crée pas
une nouvelle.** Ce commentaire motive `cli_mode: true` par « empêcher le moteur
de voler les callbacks au poll » — il parle du moteur **de la TUI**. Le moteur du
daemon, lui, tourne en `cli_mode: false` et dispatche déjà ; l'arbitre entre les
deux est `mark_task_delivered`, premier arrivé. Après V2a il reste un seul
concurrent côté moteur au lieu de deux : la livraison des callbacks devient plus
déterministe, pas moins.

Le commentaire `#264` devient faux **au merge de V2b**, pas ici : il décrit un
réglage qui existe encore après V2a.

### D7 — L'effet mika#2351, et où lire `auto_pull` après le merge

Le ticket le pose en « fait de lecture, sans objectif propre », et c'est le bon
cadrage : V2a ne vise pas `auto_pull`, il le libère par construction. Après le
merge, l'engine de la TUI n'existe plus, donc la récurrente `auto_pull_groomed`
(cron `0 */10 * * * *`, enregistrée par `task_engine/mod.rs:962-964` et
dispatchée par `dispatcher.rs:1086`) n'a plus qu'un réclamant : le daemon de
l'agent.

**Ce que la PR doit dire, et où lire `auto_pull` ensuite** — trois surfaces, à
citer telles quelles dans le corps :

1. `$MIKA_SPIRIT_LOG_FILE` (défaut `/var/log/mika/server.log`), filtré sur les
   événements du module : `auto_pull_phase`, `auto_pull_promotion_gate`,
   `auto_pull_promotion_allowed` / `_declined` / `_refused`, `auto_pull_skip`,
   `auto_pull_exclusion`, `auto_pull_failure`. `mika logs paths --agent <a>`
   imprime le chemin résolu et le filtre `jq` prêt à l'emploi.
2. `mika tasks list` — la ligne récurrente `auto_pull_groomed` et son
   `fired_at`.
3. Avant V2a, les mêmes phases pouvaient sortir dans le journal **CLI** de
   l'agent (`~/.mika/agents/<nom>/logs/mika.log.YYYY-MM-DD`), parce que le tick
   tournait dans le processus de la TUI. **Après le merge, l'absence de ces
   lignes dans le journal CLI est le régime nominal, et non une absence
   d'`auto_pull`** — c'est la classe mika#2340 (une ligne absente dit un binaire
   plus vieux que le changement, pas une absence du phénomène). La PR le dit
   explicitement : c'est la seule phrase qui empêche la lecture inverse le
   premier jour.

### D8 — Ce que V2b et V2c porteront

Nommé ici pour que leur groom ne refasse pas la mesure du § 1, et **pas**
pré-décidé : l'architecte jugera chaque vol sur son propre plan.

- **V2b (critère 2).** Les trames `TaskCompleted` / `TaskFailed` manquent cinq
  champs (§ 0.2) ; arbitrage entre champs additifs sur les trames et un endpoint
  de détail. Le flux est **par processus**, pas par session : le consommateur
  filtre sur `agent_id` et sur la session. Le parseur SSE de
  `commands/tasks.rs::stream_task_events` est à promouvoir plutôt qu'à réécrire.
  Au même vol : le bras `CallbackResult` part, l'allowlist à une entrée de V1 se
  vide, et les deux commentaires périmés de `server/mod.rs:354-356` et
  `tasks_stream.rs` tombent.
- **V2c (critère 3).** Deux endpoints à créer sous
  `crates/mika-agent/src/server/` — une création de session et une compaction —
  plus le câblage de `/rewind` et `/undo` sur les trois POST existants. Le point
  de montage est à justifier contre le précédent mika#2506.

*Tracked in: senara-solutions/mika#1727*

---

## 3 — Les tests

Trois tests neufs, tous dans `crates/mika-cli/src/commands/chat_turn.rs`, module
`tests` (l. 205) — le seul module de tests du chemin TUI, et celui où vivent
`mika1727_v1_*`.

**T1 — le détecteur du critère 1.** `mika-cli` ne construit plus de moteur de
tâches, et l'allowlist est **vide**.

```rust
/// **AC1** — aucun site de production de ce crate ne construit ni ne lance un
/// moteur de tâches.
#[test]
fn mika1727_v2a_la_tui_ne_construit_plus_de_moteur_de_taches() {
    // Livrée vide, et T3 l'assert. Quand ce scan tire, on supprime le site :
    // le daemon possède déjà le moteur, un second est la duplication que le
    // vol a retirée (doctrine mika#2201).
    const ALLOWED: &[&str] = &[];
    // Quatre gestes séparables, § D5. Jamais `mika_agent::task_engine::` :
    // `commands/tasks.rs` en importe neuf utilitaires légitimes.
    const NEEDLES: &[&str] = &[
        "TaskEngine::new(",
        "TaskDispatcher {",
        "spawn_tick_loop(",
        "startup_recovery(",
    ];
    // ProductionScanner::for_crate(env!("CARGO_MANIFEST_DIR")),
    // strip_prefix(scanner.src_root()), saute ALLOWED et les lignes "//",
    // collecte les hits, message nommant le remède.
}
```

**T2 — anti-vacuité du balayage.** Le mode de panne qui se lit comme un arbre
propre est un balayage devenu aveugle. Sur le patron de
`mika1727_v1_le_scan_nest_pas_vide` : `scanner.files().len() > 10`, et
`commands/chat.rs` **ainsi que** `commands/tasks.rs` sont vus — le second parce
que c'est le fichier qui prouve que les aiguilles de D5 discriminent (il contient
`task_engine::` en abondance et ne doit pas rougir).

**T3 — l'allowlist est livrée vide et doit le rester.** Forme
mika#2425 / #2457 / #2759 : `assert!(ALLOWED.is_empty(), …)` avec le message qui
nomme le remède (supprimer le site, pas ajouter la ligne). Une allowlist née vide
est un emplacement où déposer la prochaine infraction (mika#2323) ; ce test est
ce qui le ferme.

**T4 — plancher, pas test neuf : les quatre tests de porte de section restent
verts.** `mika2847_la_porte_de_section_heartbeat_na_que_des_lecteurs_denregistrement`,
`mika2847_chaque_lecteur_recense_annule_a_lenregistrement` et leurs deux
homologues `mika2850_*` doivent passer **après** le retrait des deux entrées de
recensement. C'est un plancher de l'ordre de commit (§ *Ordre de commit*, commit
1), vérifié par `cargo test -p mika-agent canonical_tokens`.

**T5 — pas de test de comportement sur les quatre commandes, et c'est raisonné.**
Le critère 4 porte sur la création de session, `/compact`, `/rewind`, `/undo` et
`/clear`. V2a ne touche aucun de ces cinq chemins : un test de non-régression
écrit ici mesurerait une population que le diff ne contient pas, c'est-à-dire un
vert par construction (classe mika#2205). Le critère 4 se tient pour ce vol par
le **périmètre du diff**, attesté par le plancher du commit 2 (`git diff --stat`
ne montre que les trois fichiers annoncés), et il est porté activement par V2b et
V2c.

`commands/chat.rs` n'a pas de module de tests et n'en reçoit pas : le créer pour
y mettre un test d'absence demanderait un harnais `AppContext` complet pour
`spawn_agent_worker`, pour une propriété que T1 établit en balayant la source.

---

## 4 — Fire-Disposition

Ce plan livre un détecteur structurel (T1). Disposition : **option (a) —
exception nommée en allowlist**, dans sa forme dégénérée : **zéro entrée**.

Le détecteur ne tire sur aucune violation préexistante, parce que le **même
commit** supprime le seul site de production qui le ferait tirer
(`commands/chat.rs`, les quatre gestes du § 1). La population a été recensée
avant d'écrire le scan : `grep -rn "task_engine" crates/mika-cli/src` rend dix
lignes, neuf dans `chat.rs` (toutes retirées par ce vol) et le reste dans
`commands/tasks.rs`, qui ne construit rien (§ D5).

L'allowlist est donc livrée vide, et **T3 l'y maintient**. Quand le scan tire, le
remède est de supprimer le nouveau site, jamais d'ajouter une ligne : le daemon
possède déjà le moteur, et un second exemplaire est exactement la duplication que
le vol retire.

**Pourquoi ni (b) ni (c).** (b) *land disabled* suppose une violation existante
dangereuse à laisser non signalée — il n'y en a aucune après le commit 2, et un
`#[ignore]` ferait du détecteur une décoration. (c) *halt-and-surface* suppose que
la forme de la résolution est la question à poser à l'opérateur — elle est
tranchée par le ticket lui-même (« un test rougit si `mika-cli` en construit
un »).

---

## 5 — Definition of Done

- [ ] `crates/mika-cli/src/commands/chat.rs` ne construit plus de
      `TaskDispatcher` ni de `TaskEngine`, ne lance plus `spawn_tick_loop` ni
      `startup_recovery`, n'appelle plus `prune_old_tasks`, et n'enregistre plus
      les récurrentes `heartbeat` et `reflection`.
- [ ] `AgentWorker` n'a plus de champ `poller_handle` ; les quatre sites (585,
      828, 935, 1035) sont retirés et les liaisons devenues orphelines le sont
      aussi, sans `#[allow]` neuf.
- [ ] L'import de tête `use mika_agent::task_engine::{self, TaskDispatcher, TaskEngine};`
      est retiré.
- [ ] `"crates/mika-cli/src/commands/chat.rs"` est retiré de
      `HEARTBEAT_GATE_REGISTRATION_SITES` et de
      `REFLECTION_GATE_REGISTRATION_SITES`, avec un commentaire nommant ce vol ;
      aucune entrée n'est ajoutée aux deux allowlists de lecteurs, qui restent
      vides.
- [ ] T1, T2 et T3 sont verts sur ce diff, et T1 est **rouge** si l'on rétablit
      le `TaskEngine::new` de `chat.rs` (vérifié à la main avant de commiter).
- [ ] Les quatre tests `mika2847_*` / `mika2850_*` sont verts.
- [ ] `crates/mika-cli/CLAUDE.md` § *Un tour de la TUI exige mika-spirit* gagne
      le paragraphe du vol V2a : la TUI ne fait plus tourner de moteur, qui
      enregistre les récurrentes désormais, et la lecture d'`auto_pull` après le
      merge (§ D7).
- [ ] `cargo test` complet, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check`, `make verify-bundled-skills`,
      `bash scripts/verify-pipeline.sh origin/main` : tous verts.
- [ ] La PR porte `Refs mika#1727` (jamais `Closes`), les trois surfaces de
      lecture d'`auto_pull` du § D7 avec la phrase de la classe mika#2340, la
      ligne `Tracked in: senara-solutions/mika#1727` pour V2b, V2c et la doctrine
      périmée de `tick_cli_guard_scan.rs`, et
      `Code review: delegated to mika-qa (mika#2694)`.
- [ ] Le diff ne touche aucun fichier du plan de contrôle ni aucun chemin de
      `.github/CODEOWNERS` (vérifié par `git diff --stat`).

---

## Acceptance criteria

Les quatre critères sont ceux du corps du ticket, transcrits. AC1 est livré par
ce vol ; AC2 et AC3 le sont par V2b et V2c ; AC4 est tenu ici par
non-intervention et porté activement par V2b et V2c (§ 0.1).

- **AC1 — La TUI ne construit plus de `TaskEngine` et ne lance plus de boucle de
  tick ni de `startup_recovery`. Un test rougit si `mika-cli` en construit un.**
  Livré par ce vol : les neuf sites du § 1 partent, T1 le garde avec une
  allowlist vide, T2 atteste que le balayage voit ses cibles, T3 maintient
  l'allowlist vide.
- **AC2 — Les callbacks terminés arrivent dans la conversation de la TUI par le
  SSE, et non plus par un poll local de la base.** *Non livré par ce vol.* Le
  poll reste inchangé et fonctionnel (§ D6) ; l'état mesuré du fil SSE est
  consigné au § 0.2 pour le groom de V2b.
  *Tracked in: senara-solutions/mika#1727*
- **AC3 — La création de session, `/compact`, `/rewind` et `/undo` passent par
  mika-spirit. Chacun a un test qui rougit sur l'écriture locale.** *Non livré
  par ce vol.* Inventaire spirit mesuré au § 1, point de montage à justifier au
  § D8. *Tracked in: senara-solutions/mika#1727*
- **AC4 — Le comportement visible de ces quatre commandes est conservé, y
  compris pour un agent `singleton` (`/clear` n'efface que l'affichage).** Tenu
  par ce vol par non-intervention : les sites 69-115 (`get_or_create_canonical_session`
  l. 104, `create_session` l. 111), `handle_compact`, `handle_rewind_impl` et
  `/clear` sont hors diff, attesté par le plancher du commit 2. § T5 dit pourquoi
  un test ici serait vert par construction.

### Correspondance AC → livrable

| AC | livrable | test |
|---|---|---|
| AC1 | `commands/chat.rs` — neuf sites retirés ; `canonical_tokens.rs` — deux entrées retirées | T1, T2, T3, T4 |
| AC2 | — (V2b) | — |
| AC3 | — (V2c) | — |
| AC4 | périmètre du diff | plancher du commit 2 |

---

## Taille estimée

| livrable | lignes de code (hors `docs/` et `CLAUDE.md`) |
|---|---|
| `crates/mika-cli/src/commands/chat.rs` — `TaskDispatcher`, `TaskEngine`, les deux paires de récurrentes, `startup_recovery`, `prune_old_tasks`, la boucle de tick, l'import de tête | 100 |
| `crates/mika-cli/src/commands/chat.rs` — champ `poller_handle` et ses quatre sites ; liaisons devenues orphelines (`tool_registry`, `message_sender`, `embedding_client`, `brave_api_key`, `github_token`, `skills_dirty`, `cli_tier`, `cli_deployment`) retirées ou conservées avec leur raison | 35 |
| `crates/mika-agent/src/canonical_tokens.rs` — l'entrée `commands/chat.rs` sort des deux recensements, avec sa raison | 14 |
| détecteur structurel + anti-vacuité + allowlist vide (`commands/chat_turn.rs`, module `tests` — T1 à T3) | 60 |
| marge de recompilation : `use` restants, signatures de `spawn_agent_worker` et de ses trois appelants si le tuple de retour change | 25 |

Total estimé : 234 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi ce découpage et pas un autre.** 234 < 279, et très loin du seuil par
PR de 1 000. Les quatre critères de V2 pèsent ~780 lignes : le découpage en trois
vols est imposé, pas choisi (§ 0.1). Le vol courant prend le critère 1 **entier**
— pas une moitié — parce qu'il est la seule des trois tranches qui soit une pure
soustraction, donc la seule qui n'introduise aucun chemin neuf à éprouver, et
parce qu'il porte l'effet mika#2351 que le ticket signale comme en cours
aujourd'hui.

Découper plus bas n'a pas de sens : retirer le moteur sans le détecteur livrerait
AC1 sans sa garde, et le détecteur sans le retrait rougirait sur `main`. Les deux
entrées de `canonical_tokens.rs` ne sont pas séparables non plus — les laisser un
commit de plus rend quatre tests rouges (§ 0.3).

---

## Ordre de commit

1. `refactor(1727)` — `commands/chat.rs` : le `TaskDispatcher`, le `TaskEngine`,
   les deux paires de récurrentes, `startup_recovery`, `prune_old_tasks`, la
   boucle de tick, le champ `poller_handle` et ses quatre sites, l'import de
   tête ; **dans le même commit**, les deux entrées de recensement de
   `canonical_tokens.rs`. **Plancher :** compile ;
   `cargo clippy --all-targets -- -D warnings` vert sans `#[allow]` neuf ;
   `cargo test -p mika-agent canonical_tokens` vert (les quatre tests de porte de
   section, T4) ; `cargo test -p mika-cli` vert. À ce point la TUI n'a plus de
   moteur et la course mika#2351 est éteinte, mais rien ne l'empêche de revenir.
2. `test(1727)` — T1, son anti-vacuité T2 et la garde d'allowlist vide T3, dans
   le module `tests` de `commands/chat_turn.rs`. **Plancher :** les trois verts
   sur ce diff, et T1 **rouge** si l'on rétablit le `TaskEngine::new` de
   `chat.rs` (vérifié à la main avant de commiter) ; `git diff --stat` ne montre
   que `commands/chat.rs`, `commands/chat_turn.rs` et `canonical_tokens.rs` —
   c'est l'attestation d'AC4 par le périmètre.
3. `docs(1727)` — `crates/mika-cli/CLAUDE.md` : le paragraphe du vol V2a (plus de
   moteur dans la TUI, le daemon seul enregistreur des récurrentes, la lecture
   d'`auto_pull` après le merge avec la phrase de la classe mika#2340).
   **Plancher :** `cargo test -p mika-common claude_md_size` vert (le cap de
   taille des `CLAUDE.md`).
4. Fin de pipeline : `cargo test` complet, `cargo fmt --check`,
   `make verify-bundled-skills`, `bash scripts/verify-pipeline.sh origin/main`,
   puis la PR avec `Refs mika#1727`, les trois surfaces de lecture d'`auto_pull`,
   les lignes `Tracked in: senara-solutions/mika#1727` et
   `Code review: delegated to mika-qa (mika#2694)`. **Plancher :** les cinq
   verts ; aucune invocation de `ce-simplify-code` ni de `ce-code-review`
   (mika#2694).
