# mika#2922 — Le registre des verdicts de vol, et le compte de concordance

**Ticket :** `senara-solutions/mika#2922`
**Branche :** `feat/2922/manager-registre-des-verdicts-de-vol-et`
**Label :** `loop-substrate` (plafond 200 tours ; le merge revient à Vincent)
**Dépendance :** le ticket compagnon (le périmètre de l'évaluateur) rend les vols
visibles. Ce plan se construit en parallèle ; sa sonde post-déploiement n'a de vol
à montrer qu'après le premier.

---

## Le problème, mesuré

Le seuil qui ouvre `MIKA_MANAGER_EVALUATE_WRITE` — « l'évaluateur rend, en ombre,
les mêmes verdicts que MPC sur 10 vols consécutifs ou 7 jours » — n'est pas
seulement non mesuré : **il n'est pas mesurable**, et les trois absences sont
distinctes.

1. **Le verdict du manager n'est gardé nulle part.** `FlightVerdict`
   (`milestone_manager/evaluator.rs:101`) naît dans `collect_flight_verdicts`
   (`milestone_manager/cadence.rs:669`), est posé dans
   `assessment.flight_verdicts`, rendu en prose par
   `reporter::render_flight_verdict` et compté par le champ `verdicts` de
   l'événement `manager_flight_scan`. **Un compteur n'est pas un registre** : il
   dit *combien*, jamais *lequel*, et le texte du rapport du sink est de la prose
   qu'aucun lecteur ne sait re-typer.
2. **Le jugement de MPC n'est écrit nulle part.** Il n'y a donc rien à mettre en
   face, et aucun chemin supporté pour l'écrire.
3. **Le compte de concordance n'a pas d'instrument**, donc la demande de Vincent
   du 2026-10-23 n'a pas de réponse traçable.

Bearing de Prime, 2026-10-26 : « l accord ombre doit être mesuré et les verdicts
stockés *avant* tout réarmement ».

---

## Le patron de mika#2917 tient-il ici ? — oui, sur ses quatre termes

Le ticket demande que le plan tranche. Il tient, et chacun des quatre termes se
vérifie sur ce cas, pas par analogie :

| terme du patron | tient ici parce que |
|---|---|
| `audit_events` sous un `tool_name` dédié par écrivain | il y a exactement **deux** écrivains à distinguer — le manager et MPC — donc **deux** `tool_name`, et la question « qui a écrit cette ligne » devient une colonne au lieu d'un champ de contenu |
| un **seul** écrivain par `tool_name`, attesté par une garde | `source_scan::audit_writers("<CONST>")` existe déjà, promu au niveau du crate par mika#2917 ; AC2 est deux assertions d'ensemble exact |
| aucune table, aucune migration si `audit_events` suffit | il suffit : voir le § suivant, les cinq données du verdict entrent dans les colonnes existantes. Le schéma reste à **v56** |
| un accesseur existant pour la lecture | trois existent et répondent aux trois questions du lecteur, sans en ajouter un quatrième — `get_audit_event_target_keys_by_tool_name` (le corpus et son **ordre**, `ORDER BY id`), `latest_audit_event_for_target` (la valeur **datée** d'une clé), `count_recent_audit_events_for_target` (l'idempotence d'AC1) |

**Un cinquième terme, hérité et non négociable** : `AsyncDatabase::log_audit_event`
clone son propre `agent_id` et ne l'accepte **pas** en argument — son premier
paramètre est le `session_id`. Le scope par agent est donc **structurel**, pas une
discipline d'appel : le registre de mika-dev ne peut pas voir celui d'un autre
agent, et aucun test n'a à le vérifier.

**La borne que le patron impose au lecteur** : `AUDIT_RETENTION_DAYS = 90`
(`evidence/audit.rs`). La fenêtre que la commande de concordance annonce ne doit
jamais survivre au grand livre ; sa valeur par défaut est **7 jours** — la fenêtre
du seuil de mika#2698 lui-même — et son plafond est 90. Au-delà, elle rendrait
« 0 concordant » pour une raison qui n'est pas un désaccord.

---

## L'endroit, choisi et justifié

Deux `tool_name`, une seule clé, une seule grammaire de valeur.

```text
# le verdict du manager — écrit par le moteur, à chaque cycle qui juge un vol
tool_name   = MANAGER_FLIGHT_VERDICT_TOOL  ("manager_flight_verdict")
session_id  = "manager-flight-verdict"     (un verdict n'appartient à aucune conversation)
target_key  = FlightVerdict::task_id       (tasks.id — « la clé qui rend le verdict rejouable »)
after_value = JSON de VerdictRecord { issue, recommendation }
before_value= NULL                          (il n'y a aucun état antérieur à nommer)
reasoning   = "issue_number=<n>"            quand le vol porte un ticket lisible, sinon NULL
agent_id    = injecté par AsyncDatabase, jamais passé par l'appelant

# le jugement de MPC — écrit par une sous-commande mika, jamais par du SQL à la main
tool_name   = MPC_FLIGHT_JUDGMENT_TOOL     ("mpc_flight_judgment")
session_id  = "mpc-flight-judgment"
target_key  = le MÊME tasks.id             (c'est ce qui met les deux en face)
after_value = JSON de VerdictRecord { issue, recommendation }   — même grammaire
before_value= NULL
reasoning   = la note libre de l'opérateur, ou NULL
```

**`after_value` est le JSON serde des types existants, et c'est le cœur du
choix.** `FlightIssue`, `DeathCause` et `FlightRecommendation` dérivent déjà
`Serialize`/`Deserialize` avec `#[serde(rename_all = "snake_case")]`. `VerdictRecord`
est une struct de **deux champs** qui les porte tels quels :

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictRecord {
    pub issue: FlightIssue,                   // Dead { cause } porte la DeathCause
    pub recommendation: FlightRecommendation,
}
```

Les trois termes que le ticket nomme — issue, cause, recommandation — y sont au
complet, et **aucun n'est retapé**. Une grammaire `k=v;k=v` inventée ici serait un
second vocabulaire à maintenir en phase avec trois `enum` ; la divergence serait
programmée (classe mika#2158), et c'est exactement ce que le `head_key` réexporté
de mika#2917 refuse.

**La comparaison, elle, porte sur une projection décisionnelle de trois termes**,
pas sur l'égalité du JSON :

```rust
/// Les trois termes sur lesquels un accord se décide. `cause` vaut `None` hors
/// d'une mort : « pas de cause » n'est pas une cause.
pub struct VerdictTerms { issue: &'static str, cause: Option<String>, recommendation: &'static str }
```

**Pourquoi une projection et pas l'égalité des lignes** : `FlightIssue::PrOpened`
porte `number` et `url`. MPC, jugeant à la main, ne connaît pas l'URL — et un
désaccord sur une URL n'est pas un désaccord sur un verdict. Comparer les lignes
brutes rendrait « divergent » sur du bruit, et l'instrument annoncerait 0
concordant le jour où tout concorde. La ligne reste **fidèle** (elle garde
`number`/`url`, donc elle est rejouable) ; c'est la **comparaison** qui est
décisionnelle. L'ordre des trois termes est fixe — `issue`, puis `cause`, puis
`recommendation` — et le **premier** qui diverge est le champ que la commande
nomme (AC3).

### Pourquoi là

1. **Aucun `gh`, aucune autorité.** Écrire une ligne d'audit n'est pas un geste
   sur la forge. `MIGRATION`/`gh issue edit`/`gh issue comment` restent hors du
   chemin ; `MIKA_MANAGER_EVALUATE_WRITE` n'est pas lu par ce code (AC5, voir le
   § suivant).
2. **Aucune table, aucune migration.** Les cinq données entrent dans les colonnes
   existantes, et le précédent vit dans ce dépôt : `auth_boundary`
   (`Database::record_auth_boundary`) et `agent_recurring_gate` (mika#2456 vol 3)
   portent la même forme — `session_id` hors conversation, `before_value` toujours
   `NULL`, un écrivain unique déclaré en **SOLE WRITER** dans le doc-comment de la
   constante.
3. **Une seule clé pour les deux écrivains.** C'est `target_key` qui fait la
   jointure ; aucune colonne de corrélation n'est inventée, et un `tasks.id` est
   déjà la clé que l'évaluateur publie.
4. **L'idempotence d'AC1 est une lecture, pas un `UNIQUE`.** Avant d'écrire, le
   ledger demande `count_recent_audit_events_for_target(MANAGER_FLIGHT_VERDICT_TOOL,
   task_id, since)` avec `since = now − AUDIT_RETENTION_DAYS` : au-delà de la
   rétention la ligne n'existe plus, donc « jamais inscrit » et « inscrit hors
   rétention » sont la même population, et la fenêtre de l'idempotence est celle
   du grand livre par construction. Un index `UNIQUE` sur `audit_events`
   demanderait une migration pour une propriété qu'une lecture donne.
5. **`reasoning` porte une donnée, jamais une clé de regroupement.**
   `issue_number=<n>` est de l'information ; le regroupement se fait sur
   `tool_name` et `target_key`. C'est la règle que `agent_recurring_gate` écrit en
   propre (`reasoning = "task_id=<n>"`).
6. **Le vocabulaire de rendu n'est pas dupliqué.** Les étiquettes humaines
   (`« mort au plafond (budget_exhausted) »`, `« merge-candidat »`) restent la
   propriété de `reporter::flight_issue_label` / `flight_recommendation_label` ;
   le registre stocke les littéraux serde, et la commande de concordance
   **réutilise** les fonctions du reporter pour l'affichage. Deux graphies pour
   un verdict, c'est la divergence qui fait qu'un rapport et une commande ne
   disent pas la même chose du même vol.

---

## Où vit l'écrivain, et pourquoi la garde de `no_dispatch_test.rs` reste vraie

C'est le point que le ticket demande de trancher, et il est **contraint**, pas
arbitré.

`milestone_manager/no_dispatch_test.rs` grep chaque `.rs` sous
`src/milestone_manager/` à la recherche de 22 `FORBIDDEN_TOKENS` d'autorité, avec
une allowlist de **deux** entrées (`verdict_poster.rs` / `gh issue comment`,
`pool_writer.rs` / `gh issue edit`). Cette allowlist est tenue par
`mika2699_lallowlist_porte_les_deux_gestes_et_pas_un_de_plus`, qui assère
`EXEMPTED_OCCURRENCES.len() == 2` **et** re-lit chaque fichier pour vérifier que
le jeton y est encore (auto-nettoyante). AC5 exige que ce fichier passe **sans
modification de son ensemble de jetons**.

**Décision : les deux écrivains vivent dans un module frère de
`src/milestone_manager/`, à `crates/mika-agent/src/flight_verdict_ledger.rs`, et
le trait y est déclaré avec eux.**

L'idiome est celui d'`auth_boundary_ledger.rs`, choisi plutôt que celui de
`flight_source.rs`, et la nuance compte :

- `FlightSource` déclare son **trait dans** `evaluator.rs` parce que la signature
  de l'évaluateur a besoin du type : c'est son périmètre de lecture.
- `AuthBoundaryLedger` déclare trait **et** impl dehors, et `CycleContext` le porte
  déjà (`cadence.rs:510`). Un registre n'a besoin d'aucun type à l'intérieur du
  module : le module ne nomme que `Option<&Arc<dyn FlightVerdictLedger>>` et
  appelle `record_if_wired`. **Empreinte minimale dans le périmètre gardé**, et un
  précédent qui roule déjà dans le même `CycleContext`.

La raison matérielle, par-dessus la garde : **la tâche de cadence ne tient aucun
handle de base**. Elle est construite d'un `ManagerConfig`, d'un
`CancellationToken` et d'un `TokenResolver` (la doc de module
d'`auth_boundary_ledger.rs` l'écrit). L'injection n'est donc pas un goût de
découplage, c'est le seul moyen d'atteindre la base depuis là.

**Le câblage, sur le chemin qui existe déjà :**

| site | geste |
|---|---|
| `cadence.rs:514` | un champ `verdict_ledger: Option<&'a Arc<dyn FlightVerdictLedger>>` à côté de `flight_source` |
| `cadence.rs:550` | un `with_verdict_ledger` à côté de `with_flight_source` ; `new()` le laisse à `None` |
| `cadence.rs:844-855` | l'appel, après `assessment.flight_verdicts = flight_verdicts` et **avant** `post_flight_verdicts` |
| `spawn.rs:304-315, 479` | le `Option<Arc<dyn …>>` est enfilé dans la tâche de cycle, comme `flight_source` |
| `server/mod.rs:1775` | l'unique site de câblage de production, à côté de `DbFlightSource::new(...)` |

### `record` est `async` et rend `()` — les deux moitiés sont voulues

`AuthBoundaryLedger::record` rend `()` **et** part en `handle.spawn` détaché. Ici
on garde la première moitié et on abandonne la seconde :

- **Rend `()` :** une écriture d'audit en échec ne peut pas changer un verdict,
  parce que la signature n'offre aucun moyen de propager une erreur ni d'oublier
  un `?`. C'est la propriété R5 d'`auth_boundary_ledger`, structurelle et non
  conventionnelle. L'échec est nommé par un WARN
  `flight_verdict_audit_write_failed`.
- **Est `async` et **attendu** :** AC1 est une propriété **mesurée** — « exactement
  une ligne par vol, un second cycle n'en ajoute aucune ». Un `spawn` détaché la
  rend inobservable, et `auth_boundary_ledger` a dû extraire `write_and_swallow`
  précisément pour qu'un test puisse l'attendre (« A test that cannot fail is not
  a test »). Le prix du détachement n'est pas justifié ici : l'appelant n'est pas
  sur un chemin critique — la cadence tourne toutes les N minutes et attend déjà
  un appel `gh` dans `post_flight_verdicts`. Le garde `Handle::try_current()`
  d'`auth_boundary_ledger` disparaît avec le `spawn` qu'il protégeait.

### L'appel n'est pas derrière l'interrupteur, et c'est tout le ticket

`post_flight_verdicts` consulte `write_is_armed()` et court-circuite sur
`ctx.gh_runner == None`. L'inscription ne fait **ni l'un ni l'autre** :
l'interrupteur gouverne le geste **sur la forge**, et le registre existe pour
mesurer l'accord **en ombre**, c'est-à-dire exactement quand l'interrupteur est
fermé. Un registre conditionné à son propre interrupteur ne pourrait jamais
produire la mesure qui l'ouvre.

AC5 est donc satisfait par **omission attestée** : `ENV_EVALUATE_WRITE` garde son
lecteur unique `verdict_poster::write_is_armed` (doctrine mika#2201), et une garde
assère que `flight_verdict_ledger.rs` ne le nomme pas.

### La sonde, sur les trois branches

`record_flight_verdicts` émet `event = "manager_verdict_inscription"`,
`scope = "container"`, **inconditionnellement** — `candidates`, `inscribed`,
`already_recorded`, `failed`, et `branch` ∈ {`unwired`, `no_verdicts`, `written`}.
C'est la branche à zéro qui porte l'information : sans elle, « le ledger n'est pas
câblé » et « aucun vol n'a été jugé » rendent les mêmes octets (classe mika#2205),
et l'absence de la ligne signifie un binaire antérieur au vol, jamais une absence
d'inscription (motif mika#2340). `inscribed` et `already_recorded` sont deux
champs et jamais une somme : ils ne demandent pas le même geste.

---

## Ce qui a été écarté, et pourquoi

- **Une table `flight_verdicts` + une migration v57.** Écartée : `audit_events`
  porte les cinq données, le scope par agent, la rétention et trois accesseurs. Une
  table apporterait un `UNIQUE(agent_id, task_id, tool)` — l'idempotence gratuite —
  au prix d'une migration append-only *pour toujours* (`db/migrations.rs`), d'un
  second grand livre à purger et d'un `tool_name` perdu comme discriminant
  d'écrivain. Le patron que le ticket désigne refuse ce coût quand `audit_events`
  suffit, et il suffit.
- **`store_fact` / `memory_facts`.** Écartée : c'est la mémoire de l'agent, de la
  prose indexée pour une recherche hybride, pas un registre à clé. Un verdict y
  serait relu par un modèle, jamais par un comparateur.
- **Un fichier sous `~/.mika/state/`.** Écartée : il échappe au scope par agent,
  à la rétention et à toute requête SQL ; et il introduit un chemin à résoudre
  deux fois (écrivain, lecteur) — le défaut que `milestone_manager::sink_dir`
  (mika#2267) a dû refermer une fois.
- **Surcharger le `tool_name` de `manager_flight_scan`** (un seul nom pour les deux
  écrivains, discriminés par `after_value`). Écartée : AC2 demande **deux** gardes
  d'écrivain unique, et deux populations sous un nom ne sont pas soustractibles
  (mika#2205). La colonne qui discrimine doit être celle sur laquelle les gardes
  mordent.
- **Écrire le jugement de MPC par un outil du modèle** (la forme de mika#2917, où
  un skill écrit). Écartée : MPC juge **à la main**, en lisant ; le chemin
  supporté que le ticket demande est une sous-commande, déterministe et rejouable,
  pas un tour de modèle dont l'appel d'outil peut manquer.
- **Implémenter l'écrivain de MPC dans `mika-cli`.** Écartée, et c'est une
  contrainte structurelle, pas une préférence : `source_scan::production_files()`
  est enracinée sur `crates/mika-agent/src/` **seulement**, donc `audit_writers()`
  est aveugle à `crates/mika-cli/`. Un écrivain côté CLI rendrait la seconde garde
  d'AC2 **vide** — verte par construction. L'écrivain vit dans
  `flight_verdict_ledger.rs` ; la sous-commande est un adaptateur mince, selon la
  doctrine **Wrapper-only INV-2** de `crates/mika-cli/src/commands/milestone.rs`
  (« recomposer un chemin ou une lecture ici rendrait le lecteur capable de
  regarder là où l'écrivain n'écrit pas »).
- **Un quatrième accesseur `audit_events`.** Écartée : les trois existants
  répondent aux trois questions. `get_audit_event_rows_by_tool_name` et
  `get_audit_event_target_keys_by_tool_name` sont `#[doc(hidden)]` et annoncés
  comme aides de test mika#1712 ; le vol 2 **promeut** le second en lecteur déclaré
  (retrait de `#[doc(hidden)]`, doc-comment amendé). `worktree_reaper.rs` lit déjà
  le premier, donc la promotion constate un usage, elle ne l'ouvre pas.

### Coût assumé du choix

- **La rétention de 90 jours est aussi celle de l'idempotence.** Un vol jugé il y
  a plus de 90 jours, dont la ligne a été compactée, serait ré-inscrit au cycle
  suivant s'il reparaissait dans le corpus. Nommé plutôt que corrigé : c'est la
  même propriété que la déduplication en mémoire d'`agent_recurring_gate`, perdue
  au redémarrage, et un vol terminé il y a trois mois n'est plus dans la fenêtre
  de l'évaluateur.
- **Le JSON de `VerdictRecord` est un format de ligne.** Un renommage de variante
  serde dans `FlightIssue` rend illisibles les lignes déjà écrites. Le coût est
  borné par la rétention (90 jours) et par le fait que le lecteur dégrade une
  ligne illisible en `« illisible »` nommé, jamais en concordance ni en
  divergence.
- **La fenêtre par défaut de 7 jours n'est pas le seuil de 10 vols.** Le seuil de
  mika#2698 est une disjonction (« 10 vols consécutifs **ou** 7 jours ») ; la
  commande rend les deux nombres et ne décide pas lequel est atteint. C'est MPC
  qui présente, pas la commande qui arbitre.

---

## Découpage en vols

Le total estimé des trois livrables du ticket dépasse la borne par vol de 429
lignes (voir § *Taille estimée*). Coupe en **deux vols**, chacun mergeable seul.

### Vol 1 — ce que cette PR livre

Le **livrable 1** du ticket : chaque verdict de l'évaluateur est inscrit, une
ligne par vol et par cycle qui le juge pour la première fois. **Personne ne lit
encore** — c'est exactement la forme du vol 2 de mika#2917 (« le skill écrit,
personne ne lit »), que le ticket désigne comme le patron à reprendre. Couvre
**AC1**, la **première moitié d'AC2** (écrivain unique du verdict du manager),
**AC5**, et la **première moitié d'AC6** (la ligne existe en base).

### Vol 2 — la suite, nommée

Les **livrables 2 et 3** : le jugement de MPC par sous-commande `mika`, et la
commande de concordance en lecture seule. Couvre la **seconde moitié d'AC2**,
**AC3**, **AC4**, et la **seconde moitié d'AC6**.

**Tracked in: senara-solutions/mika#2922 (vol 2 — le jugement de MPC et le compte
de concordance).** Le ticket reste ouvert après le merge du vol 1.

Pourquoi cette ligne de coupe et pas une autre : le vol 1 est le seul des deux qui
n'a **aucune** dépendance sur l'autre, et le ticket lui-même dit que sa sonde n'a
de vol à montrer qu'après le premier cycle. Le vol 2 lit ce que le vol 1 écrit ;
l'inverse serait un lecteur devant un registre vide, c'est-à-dire un instrument
qui rend « 0 concordant » sans distinguer « aucun verdict » de « aucun accord » —
le défaut que ce ticket existe pour fermer.

### Ce que le vol 2 déclarera, et que le vol 1 ne déclare pas

`MPC_FLIGHT_JUDGMENT_TOOL` n'est **pas** déclaré au vol 1. Une constante que
personne n'écrit ferait une garde d'écrivain dont l'ensemble exact est vide :
verte, et dont le rouge attendu au vol 2 s'apprend à ignorer. Elle naît avec son
écrivain, dans le même fichier, à côté de celle du manager.

---

## Ordre de commit (vol 1)

Chaque étape a un **plancher** : la commande qui doit être verte avant de passer à
la suivante.

1. **`crates/mika-agent/src/flight_verdict_ledger.rs`** — la doc de module (le §
   *L'endroit, choisi et justifié* en condensé), `MANAGER_FLIGHT_VERDICT_TOOL`
   avec sa déclaration **SOLE WRITER**, `MANAGER_FLIGHT_VERDICT_SESSION_ID`,
   `VerdictRecord` + `VerdictTerms` + la projection, `trait FlightVerdictLedger`,
   `DbFlightVerdictLedger`, `write_and_swallow`, `record_if_wired`,
   `#[cfg(test)] pub(crate) mod test_support { RecordingLedger }`. Déclaration
   `pub mod flight_verdict_ledger;` dans `lib.rs`, alphabétiquement entre
   `evidence` et `flight_source`.
   **Plancher :** `cargo build -p mika-agent` + `cargo clippy -p mika-agent`.
2. **Les tests du module** — les quatre de l'idiome `auth_boundary_ledger`
   (non câblé = no-op ; câblé = reçoit ; échec d'écriture avalé, avec sa
   **précondition** `assert!(… .is_err())` sur un `agent_id` inexistant ;
   sérialisation aller-retour de `VerdictRecord`), **AC1** sur une vraie base
   (`Database::open_in_memory()` + `AsyncDatabase::new_with_agent`) : un corpus de
   3 vols, un premier passage inscrit 3 lignes, un second n'en ajoute **aucune**,
   et les `target_key` sont exactement les trois `tasks.id`.
   **Plancher :** `cargo test -p mika-agent flight_verdict_ledger`.
3. **Les gardes (AC2 première moitié, AC5)** — `audit_writers("MANAGER_FLIGHT_VERDICT_TOOL")`
   en ensemble **exact**, anti-vacuité (`files.len() >= 100`), aucun littéral
   recopié hors du module, le module ne nomme pas `ENV_EVALUATE_WRITE`, et le
   fichier ne vit pas sous `src/milestone_manager/`. Deux contrôles négatifs sur
   sources fabriquées (la fenêtre d'écrivain mord ; un second site est vu).
   **Plancher :** `cargo test -p mika-agent flight_verdict_ledger` +
   `cargo test -p mika-agent no_dispatch` **vert sans aucune modification de ce
   fichier**.
4. **`milestone_manager/cadence.rs`** — le champ, le builder,
   `record_flight_verdicts` avec sa sonde sur trois branches, et l'appel dans
   `run_manager_cycle_in` après `assessment.flight_verdicts = …`.
   **Plancher :** `cargo test -p mika-agent milestone_manager`.
5. **`milestone_manager/spawn.rs` + `server/mod.rs`** — l'enfilage et l'unique
   câblage de production.
   **Plancher :** `cargo test -p mika-agent` + `cargo clippy --all-targets`.
6. **`crates/mika-agent/CLAUDE.md` § *Audit Log*** — une entrée
   `tool_name = 'manager_flight_verdict'` sur la forme des deux existantes
   (forme de ligne, requête opérateur, propriétés load-bearing). Hors du compte de
   lignes.
   **Plancher :** `make lint` + `scripts/verify-pipeline.sh origin/main`.

---

## Contrat de vérification (vol 1)

| # | geste | attendu |
|---|---|---|
| V1 | `cargo test -p mika-agent flight_verdict_ledger` | vert, 9 tests |
| V2 | `cargo test -p mika-agent no_dispatch` | vert, **`no_dispatch_test.rs` non modifié** (`git diff --stat` le montre absent) |
| V3 | `cargo test -p mika-agent milestone_manager` | vert ; le test de sonde voit `branch = "unwired"` quand le ledger est `None` |
| V4 | `cargo test -p mika-agent` | vert, aucune régression |
| V5 | `cargo clippy --all-targets -- -D warnings` | vert |
| V6 | `make lint` + `make fmt` | vert |
| V7 | `scripts/verify-pipeline.sh origin/main` | vert (U2 : le plan porte `## Acceptance criteria`) |
| V8 | `grep -n "CURRENT_SCHEMA_VERSION" crates/mika-agent/src/db.rs` | **56**, inchangé — aucune migration |
| V9 | `grep -rn "MIKA_MANAGER_EVALUATE_WRITE" crates/mika-agent/src/ \| wc -l` | identique à avant la PR |

V2 et V9 sont les deux mesures d'AC5 : la garde passe sans être touchée, et
l'interrupteur n'a pas gagné de lecteur.

---

## Sonde post-déploiement (AC6)

AC6 demande que la sonde soit **écrite dans le plan**. Elle a deux moitiés, et la
seconde appartient au vol 2.

### Moitié du vol 1 — la ligne existe en base

**Préconditions :** `make deploy` (le binaire embarque le nouveau module ; rien
sous `skills/bundled/` ne change, donc aucune projection de skill n'est en jeu
ici), puis **attendre un cycle du manager qui juge au moins un vol** — ce qui
dépend du ticket compagnon. La sonde n'est pas exécutable avant le premier vol, et
c'est une attente, pas un échec.

1. **La sonde dit d'abord si le mécanisme a tourné**, avant de parler de lignes :

   ```
   grep '"event":"manager_verdict_inscription"' $MIKA_SPIRIT_LOG_FILE | tail -3
   ```

   Lecture : `branch = "unwired"` ⇒ le câblage de `server/mod.rs` n'a pas pris
   (défaut de cette PR). `branch = "no_verdicts"` ⇒ le mécanisme tourne et aucun
   vol n'a été jugé (nominal avant le ticket compagnon). `branch = "written"` avec
   `inscribed = n` ⇒ aller au point 2. **Aucune ligne du tout** ⇒ le binaire servi
   est antérieur au vol (motif mika#2340), pas une absence d'inscription : relire
   la sha de `make deploy`.

2. **La ligne, par la lecture seule qui existe :**

   ```sql
   SELECT target_key, after_value, reasoning, created_at
     FROM audit_events
    WHERE tool_name = 'manager_flight_verdict'
    ORDER BY id DESC LIMIT 5;
   ```

   Attendu : un `target_key` qui est un `tasks.id` du vol jugé, un `after_value`
   qui parse en `VerdictRecord`, `before_value` NULL.

3. **L'idempotence, en production :** relever le `COUNT(*)` par `target_key`, puis
   relever à nouveau après le cycle suivant. Attendu : **inchangé**, et la sonde du
   point 1 montre `already_recorded` qui a crû de la même quantité que `inscribed`
   n'a *pas* crû. Les deux champs se lisent ensemble ; aucun des deux seul ne dit
   si la non-réécriture a eu lieu ou si le cycle n'a rien vu.

### Moitié du vol 2 — la commande rend « non jugé par MPC »

Après le merge du vol 2, sur le même vol et sans avoir inscrit de jugement :
`mika flight-verdicts concordance --agent mika-dev` rend une ligne pour ce vol,
avec son ticket, le verdict du manager, et `« non jugé par MPC »` en colonne de
jugement ; en tête, `« 0 vols consécutifs concordants »` sur une fenêtre de 7
jours — et **pas** une divergence. C'est AC4 en production : une absence ne compte
ni pour ni contre.

---

## Definition of Done

- [ ] `flight_verdict_ledger.rs` existe comme module frère de
      `src/milestone_manager/`, déclaré dans `lib.rs`, portant `MANAGER_FLIGHT_VERDICT_TOOL`
      avec sa déclaration **SOLE WRITER** et sa doc de module expliquant pourquoi
      il vit là.
- [ ] Chaque vol jugé par un cycle a exactement une ligne `audit_events` sous ce
      `tool_name`, clé `tasks.id`, portant issue (et `DeathCause` pour une mort) et
      recommandation ; un second cycle sur les mêmes vols n'en ajoute aucune.
      Attesté sur une base réelle.
- [ ] `trait FlightVerdictLedger` rend `()` et est `async` ; l'échec d'écriture est
      avalé sous un WARN nommé et ne change aucun verdict.
- [ ] Le ledger est injecté par `CycleContext`, enfilé par `spawn.rs`, câblé à
      l'unique site de production `server/mod.rs`.
- [ ] `event = "manager_verdict_inscription"` est émis sur ses trois branches, y
      compris celle à zéro, avec `inscribed` et `already_recorded` distincts.
- [ ] Une garde fait rougir un second site d'écriture du verdict du manager, avec
      anti-vacuité et contrôle négatif sur source fabriquée.
- [ ] `MIKA_MANAGER_EVALUATE_WRITE` garde son lecteur unique ; `no_dispatch_test.rs`
      passe **sans modification de son ensemble de jetons** ni de son allowlist.
- [ ] Aucune table, aucune migration : `CURRENT_SCHEMA_VERSION` reste à 56.
- [ ] `crates/mika-agent/CLAUDE.md` § *Audit Log* porte l'entrée du nouveau
      `tool_name`, sur la forme des deux existantes.
- [ ] Le contrat de vérification V1–V9 est vert.
- [ ] La PR porte `Tracked in: senara-solutions/mika#2922 (vol 2)` et
      `Code review: delegated to mika-qa (mika#2694)`.

---

## Acceptance criteria

*(Transcrits verbatim du corps de `senara-solutions/mika#2922`. La répartition
vol 1 / vol 2 est annotée après chaque critère ; le texte du critère ne l'est
pas.)*

- **AC1, l'inscription.** Après un cycle sur un corpus de vols, chaque vol jugé a
  exactement une ligne de verdict. Un second cycle sur les mêmes vols n'en ajoute
  aucune. Test sur une vraie base.
  → **vol 1**, attesté par V1.
- **AC2, un seul écrivain par `tool_name`.** Une garde fait rougir un second site
  d'écriture du verdict du manager, et un second site d'écriture du jugement de
  MPC.
  → **première moitié au vol 1** (verdict du manager, V1) ; **seconde moitié au
  vol 2** (jugement de MPC, née avec son écrivain).
- **AC3, la concordance.** Un corpus de 3 vols en donne 2 concordants et 1
  divergent sur la cause de mort. Le compte rend « 2 consécutifs » ou « 0 », selon
  l'ordre, et nomme le champ divergent. Il y a un contrôle positif (tout concorde)
  et un contrôle négatif (une divergence) dans le même test.
  → **vol 2**.
- **AC4, aucun verdict n'est inventé.** Un vol que MPC n'a pas jugé apparaît comme
  « non jugé par MPC » et ne compte ni pour ni contre.
  → **vol 2** ; la propriété est préparée au vol 1 par le choix de
  `latest_audit_event_for_target`, dont le `None` est une absence et jamais un
  zéro (motif mika#2331).
- **AC5, aucune autorité.** `MIKA_MANAGER_EVALUATE_WRITE` reste lu exactement
  comme avant. `no_dispatch_test.rs` passe sans modification de son ensemble de
  jetons.
  → **vol 1**, attesté par V2 et V9.
- **AC6, la sonde post-déploiement**, écrite dans le plan : après `make deploy`, au
  premier cycle qui juge un vol, la ligne existe en base, et la commande de
  concordance la rend avec « non jugé par MPC ».
  → **écrite ci-dessus en entier** ; sa première moitié est exécutable après le
  vol 1, sa seconde après le vol 2.

---

## Fire-Disposition

Ce plan livre des détecteurs : la garde d'écrivain unique d'AC2, la garde d'AC5
(le module ne nomme pas `ENV_EVALUATE_WRITE` ; le fichier ne vit pas sous
`src/milestone_manager/`), et leurs contrôles négatifs. La section est donc due.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist est livrée
VIDE.** C'est la forme de `mika2456_the_gate_audit_name_has_exactly_one_writer` et
de `mika2917_a2_un_seul_ecrivain_et_au_plus_un_lecteur` : le registre naît avec ce
vol, il n'existe **aucune** violation préexistante à excepter, et une allowlist
non vide ici serait le faux vert que ces deux gardes refusent en propre.

Détail d'implémentation, et ce qui rougit :

- **(a.1) L'écrivain, ensemble exact et vide d'exception.**
  `audit_writers("MANAGER_FLIGHT_VERDICT_TOOL")` doit valoir exactement
  `["flight_verdict_ledger.rs"]`. Quand l'ensemble grandit, **le remède est de
  router l'écriture par le ledger, jamais d'ajouter une entrée** (doctrine
  mika#2201) : il n'y a pas d'allowlist à amender, et c'est le message d'assertion
  qui le dit.
- **(a.2) Pas de lecteur au vol 1, et la garde ne le prétend pas.** Aucune
  assertion de lecteur n'est livrée : personne ne lit. Une garde `≤ 1 lecteur`
  posée ici rougirait au vol 2 **par construction**, et un rouge attendu s'apprend
  à ignorer — c'est le raisonnement écrit de mika#2917 (a.2), repris tel quel,
  mais dans l'autre sens : lui livrait le `≤ 1` parce qu'un lecteur arrivait au
  vol 3 ; ici le lecteur arrive au vol 2 et l'assertion naîtra avec lui.
- **(a.3) Anti-vacuité, obligatoire (mika#2205).** `production_files().len() >= 100`
  avant toute assertion d'ensemble : un scan qui n'a rien lu rend `[]`, donc rend
  **vert** l'assertion « un seul écrivain » sur un arbre qu'il n'a pas vu.
- **(a.4) Contrôles négatifs sur sources fabriquées.** La fenêtre de cinq lignes
  d'`audit_writers` mord sur un `log_audit_event(` suivi de la constante
  (`mika2922_le_scan_decrivain_mord`), et l'auto-nettoyage de la garde d'AC5 :
  elle re-lit `flight_verdict_ledger.rs` et assère que la moitié production,
  commentaires retirés, **contient** bien `MANAGER_FLIGHT_VERDICT_TOOL` — sinon la
  garde serait verte sur un fichier vidé, c'est-à-dire exactement quand elle doit
  rougir.
- **(a.5) Aucun littéral recopié.** Le scan refuse `"manager_flight_verdict"`
  écrit en clair hors du module de vérité : un littéral recopié est un écrivain
  que `audit_writers` ne voit pas.

**Ce qui est explicitement hors Fire-Disposition :** `no_dispatch_test.rs` et son
`EXEMPTED_OCCURRENCES` ne sont **pas touchés** — ni une entrée ajoutée, ni un
jeton retiré. AC5 est satisfait parce que le plan place l'écrivain hors du
périmètre scanné, pas parce qu'il exempte quoi que ce soit. Son allowlist reste à
deux entrées, et `mika2699_lallowlist_porte_les_deux_gestes_et_pas_un_de_plus`
reste l'assertion qui le garantit.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/flight_verdict_ledger.rs` — doc de module, vocabulaire, `VerdictRecord`/`VerdictTerms`, trait, impl DB, `write_and_swallow`, `record_if_wired`, `test_support` | 150 |
| tests du même module — les 4 de l'idiome `auth_boundary_ledger` + AC1 sur vraie base + sérialisation | 95 |
| gardes AC2/AC5 + 2 contrôles négatifs (même `mod tests`) | 75 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` — champ, builder, `record_flight_verdicts`, sonde, appel | 55 |
| `crates/mika-agent/src/milestone_manager/spawn.rs` + `src/server/mod.rs` — enfilage et câblage unique | 25 |
| `crates/mika-agent/src/lib.rs` — déclaration du module | 1 |

Total estimé : 401 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient, et où est la marge.** 401 contre 429 : la marge est
de 28 lignes, soit 7 %. Elle est mince, et la coupe qui la défend est déjà faite —
les trois livrables du ticket totalisaient ≈ 715 lignes estimées (≈ 130 pour le
lecteur de concordance, ≈ 100 pour la sous-commande et ses trois bras de `match`
exhaustif dans `cli.rs`, ≈ 90 pour AC3/AC4), d'où le découpage en deux vols du §
*Découpage en vols*. Si l'implémentation déborde, la variable d'ajustement est
nommée d'avance : les tests de cadence de l'étape 4 se replient sur
`RecordingLedger` dans le `mod tests` du ledger (la sonde reste attestée, le
chemin de cycle est couvert par le test d'intégration existant du manager), ce qui
rend ≈ 25 lignes. Ce qui ne se replie pas : AC1 sur vraie base, et les gardes
d'AC2/AC5 — ce sont les critères.

Le reste du ticket est sous `Tracked in: senara-solutions/mika#2922 (vol 2 — le
jugement de MPC et le compte de concordance)`.
