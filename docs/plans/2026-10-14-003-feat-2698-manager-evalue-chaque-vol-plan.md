# Plan : le manager évalue chaque vol terminé — Phase 2 (b), vol 1 (mika#2698)

## Contexte

`milestone_manager` est en Phase 1 : **lecture seule**. Son absence d'autorité
d'écriture n'est pas une convention, c'est une garantie structurelle — le grep
de jetons de `milestone_manager/no_dispatch_test.rs` refuse `gh issue comment`,
`gh issue edit`, `run_claude_pilot`, `"POST"`, `dispatcher_source = 'mika_manager'`
et seize autres, sur **tout** fichier du module sauf lui-même.

La Phase 2 rend au manager la conduite du rail, en trois tickets dans l'ordre
**b → a → c**. Celui-ci est le premier, et c'est le seul qui ne touche rien :
il produit la **mesure** dont dépend l'ouverture des deux autres. Le seuil est
un geste opérateur, pas une ligne de code — l'interrupteur s'ouvre quand le
verdict ombre concorde avec le jugement de l'orchestrateur sur 10 vols
consécutifs, ou sur 7 jours. Il faut donc d'abord qu'un verdict ombre existe et
soit lisible.

## Ce que le code offre déjà — relevé, parce qu'il change le périmètre

Quatre constats mesurés dans l'arbre, chacun retire du travail au ticket :

1. **Les tours sont déjà persistés.** `server/dashboard_dev_runs.rs:21-105`
   (`DevRunResponse`) extrait depuis `tasks.metadata.claude_pilot` :
   `turns`, `pr_number`, `pr_url`, `session_id`, `duration_ms`, `cost_usd`,
   `branch`, `repo`. C'est une surface de production, pas une hypothèse. Il
   n'y a **aucun transcript à parser** pour les tours consommés.

2. **L'état de la PR est déjà hydraté.** `SubIssue` (`types.rs:80-102`) porte
   `pr_number` **et** `pr_state` (`open` / `closed` / `merged`, chaîne brute de
   `gh pr view`) ainsi que `labels`. Le `reader` les remplit à chaque cycle. La
   contrainte « aucune nouvelle dépendance réseau » du ticket est donc tenue
   sans effort : l'évaluateur corrèle, il n'appelle pas.

3. **L'issue du vol est écrite en clair dans `tasks.result`.** `dispatch-lib.sh`
   y pose des lignes structurées : `Outcome: PR_OPENED — <url>` /
   `PIPELINE_INCOMPLETE` / `ESCALATE` / `LAUNCHER_DEAD`, `Halt class: <famille> — <sens>`
   (`_halt_family()`, ~l. 4999), `Turns: N`, `PR:` / `NO_PR:`,
   `STRUCTURAL VIOLATION:`, `PIPELINE FAILURE:`. Une PR de sauvetage (mika#1282)
   se reconnaît à un `Outcome: PIPELINE_INCOMPLETE` **portant** une PR.

4. **Le précédent d'injection existe et est à une ligne près celui qu'il faut.**
   `server/mod.rs:1679-1714` construit `Arc<dyn AuthBoundaryLedger>` depuis
   `agents.get(&default_agent)…db.clone()`, `None` quand l'agent par défaut est
   absent, et le passe à `spawn_manager_cycle_task`. Le trait vit dans
   `crate::auth_boundary_ledger`, l'impl DB aussi — le module `milestone_manager`
   n'a jamais de `Database` en main.

**Ce qui manque réellement, et c'est un seul datum : le plafond de tours.**
`tasks.result` porte `Turns: N` mais jamais la borne. Côté shell, le plafond
vient de `_pilot_max_turns` / `PILOT_LABEL_TURN_CEILINGS` (`loop-substrate` → 200),
et `dispatch-lib.sh` déclare explicitement que cette table **a un seul lecteur**
(note l. 726). La redupliquer en Rust pour « lire le plafond » serait exactement
le second lecteur qu'elle refuse. Le vol 1 rapporte donc ce qu'il peut lire sans
mentir : **les tours consommés** et **si le plafond a été atteint** (famille de
halte `budget_exhausted`, issue de `error_max_turns`). La valeur du plafond
s'obtient par un changement côté **producteur** — une ligne `Turn ceiling: N`
dans le corps de callback — qui appartient au vol 2.

## Découpage en vols

Le ticket l'autorise en toutes lettres (« Découper en vols si l'estimation
dépasse la borne par vol »), et l'axe de coupe est celui du ticket lui-même :
son point 3 (mode ombre) avant son point 4 (écriture derrière interrupteur).

| vol | contenu | AC couverts |
|---|---|---|
| **vol 1 — ce plan** | détection, verdict structuré, rendu en mode ombre dans le rapport, contrôle négatif du détecteur | AC3 entier ; AC1 dans sa forme forte (aucun chemin d'écriture n'existe) |
| **vol 2 — Phase 2 (b) vol 2** | `MIKA_MANAGER_EVALUATE_WRITE`, le geste `gh issue comment`, l'idempotence par checkpoint, l'exception nommée en allowlist, la ligne `Turn ceiling: N` côté producteur, le tour du dernier commit | AC1 dans sa forme nominale (exception nommée + contrôle négatif), AC2 |

Le vol 1 ne déclare **pas** la constante d'environnement. Un interrupteur dont
la branche ON ne fait rien est un mensonge de surface : il se lit comme armé et
ne l'est pas. Il naît avec son geste, au vol 2.

Le vol 2 est ouvert à la fusion du vol 1, sur ce même ticket mika#2698 (le
ticket reste ouvert : son AC2 n'est pas tenu par le vol 1).

## Conception — vol 1

### 1. `milestone_manager/evaluator.rs` — les types du verdict

Nouveau module, déclaré dans `milestone_manager/mod.rs` (`pub mod evaluator;`)
et ses types ré-exportés comme ceux de `assessor` / `reporter`. **Pas de
sous-répertoire `tests/`** : la broche de Porte 1
`milestone_manager_has_no_nested_tests_directory` (`perimeter/tests.rs`) l'interdit,
parce que `MECHANICAL_CONTAINS` accorde MECHANICAL à tout chemin contenant
`/tests/`. Les tests vivent inline, comme partout dans le module.

```rust
pub enum FlightIssue {
    PrOpened { number: Option<u64>, url: Option<String> },
    RescuePr { number: Option<u64>, url: Option<String> },
    Dead { cause: DeathCause },
}

pub enum DeathCause {
    CeilingReached,        // Halt class: budget_exhausted (error_max_turns)
    Refusal,               // STRUCTURAL VIOLATION: / refus explicite
    PipelineFailure,       // PIPELINE FAILURE: sans PR
    LauncherDead,          // Outcome: LAUNCHER_DEAD
    Escalated,             // Outcome: ESCALATE
    Unknown(String),       // famille de halte non classée — nommée, pas avalée
}

pub enum FlightRecommendation { MergeCandidate, ManualResumption, Regroom }

pub struct FlightVerdict {
    pub task_id: String,
    pub issue_number: Option<u64>,
    pub issue: FlightIssue,
    pub turns_used: Option<u32>,
    pub hit_ceiling: bool,          // le plafond a été atteint
    pub turn_ceiling: Option<u32>,  // toujours None au vol 1 — voir § Contexte
    pub last_commit_turn: Option<u32>, // toujours None au vol 1 — vol 2
    pub recommendation: FlightRecommendation,
    pub completed_at: Option<String>,
}
```

`turn_ceiling` et `last_commit_turn` sont des `Option` **présentes et `None`**,
pas des champs absents : `None` dit « non instrumenté », `0` ou l'absence
diraient « mesuré à rien » (motif mika#2331). Le rendu écrit `non instrumenté`,
jamais un chiffre.

**Matrice de recommandation**, dérivée et testée exhaustivement :

| issue | état PR (depuis `SubIssue.pr_state`) | recommandation |
|---|---|---|
| `PrOpened` | `open` | merge-candidat |
| `PrOpened` | `merged` / `closed` | reprise à la main (le vol est derrière l'état réel) |
| `RescuePr` | quelconque | reprise à la main |
| `Dead { CeilingReached }` | — | re-groom (le plan ne tient pas dans le plafond) |
| `Dead { Refusal \| Escalated }` | — | re-groom |
| `Dead { PipelineFailure \| LauncherDead \| Unknown }` | — | reprise à la main |

### 2. La source des vols : un trait lu, injecté

Le manager n'a pas de `Database` et n'en aura pas. Le trait vit dans
`evaluator.rs`, l'implémentation DB vit **hors** du module — exactement le
partage `AuthBoundaryLedger` / `DbAuthBoundaryLedger`, ce qui a aussi pour effet
que le SQL ne passe jamais sous le grep de `no_dispatch_test.rs` et n'a donc
pas à être excepté.

```rust
// milestone_manager/evaluator.rs
pub struct FinishedFlight {
    pub task_id: String,
    pub status: String,
    pub result: Option<String>,
    pub metadata: Option<String>,   // JSON brut ; claude_pilot lu ici
    pub reference_url: Option<String>,
    pub completed_at: Option<String>,
}

#[async_trait]
pub trait FlightSource: Send + Sync {
    /// Vols d'implémentation terminés depuis `since` (exclu), pour les issues
    /// données. Lecture seule. Jamais d'erreur visible : un échec rend `vec![]`
    /// et un WARN — un rapport sans section vaut mieux qu'un cycle mort.
    async fn finished_implement_flights(
        &self,
        since: Option<&str>,
        issue_urls: &[String],
    ) -> Vec<FinishedFlight>;
}
```

```rust
// crates/mika-agent/src/flight_source.rs  (hors milestone_manager)
pub struct DbFlightSource { db: AsyncDatabase }
```

appuyé sur une méthode nouvelle `Database::list_finished_implement_flights`
dans `db.rs` (région qui brasse — pas `db/migrations.rs`, qui est append-only) :

```sql
SELECT <TASK_COLUMNS> FROM tasks
WHERE COALESCE(dispatch_class, 'implement') = 'implement'
  AND status IN (<statuts terminaux>)
  AND completed_at > ?1
  AND reference_url IN (<variantes>)
ORDER BY completed_at ASC
```

Les variantes d'URL viennent de `task_state::tasks::issue_url_variants`
(canonique + suffixe `?phase=groom`), les statuts terminaux de
`is_terminal_task_status`. Aucune nouvelle migration : zéro DDL.

### 3. Rendu en mode ombre

Un champ sur `Assessment` (`types.rs`), sur le patron exact de
`contention_events` :

```rust
/// Verdicts des vols terminés observés pendant ce cycle (mika#2698).
/// Vol 1 : mode ombre — ce champ est la seule destination du verdict.
///
/// `#[serde(default)]` porte sa charge : un blob de rapport écrit avant ce
/// champ se désérialise encore (le sink disque en détient), et une liste vide
/// se sérialise en `[]` et non `null`.
#[serde(default)]
pub flight_verdicts: Vec<FlightVerdict>,
```

avec les deux tests du patron (`…_serde_default_empty_flight_verdicts`,
`…_serde_roundtrip_empty_flight_verdicts`) qui épinglent `[]` et non `null`.

Côté `reporter.rs`, une section qui **n'apparaît que lorsqu'il y a quelque
chose** — précédent `render_auth_boundary_note` / `report_with_auth_notes`. Elle
se place après « Next action », avant les alertes de dérive :

```
### Vols terminés depuis le cycle précédent (mode ombre — aucune écriture)
- #2693 — PR ouverte (#2696) — 83 tours (plafond non instrumenté) — merge-candidat
- #2661 — mort au plafond (budget_exhausted) — 200 tours — re-groom
- #2655 — PR de sauvetage (#2690) — 141 tours — reprise à la main
```

La mention « mode ombre — aucune écriture » est dans le titre de section, pas
dans une note de bas de page : c'est elle qui permet à l'opérateur de comparer
sans se demander si le manager a agi.

### 4. Câblage

- `CycleContext::with_flight_source(Option<&'a Arc<dyn FlightSource>>)` — un
  builder de plus, pas un paramètre positionnel de plus (idiome déjà posé par
  `with_auth_ledger` / `with_auth_notes`, `cadence.rs:431-470`).
- `run_manager_cycle_in` appelle la source entre l'assessment et le rapport,
  borné par `checkpoint.last_observed_at` pour le « depuis le cycle précédent ».
  Source absente ⇒ `flight_verdicts` vide ⇒ section absente ⇒ rapport identique
  à aujourd'hui.
- `spawn.rs` : un paramètre de plus à `spawn_manager_cycle_task`, transporté
  jusqu'au contexte.
- `server/mod.rs:1679-1714` : la construction, calquée ligne pour ligne sur
  `DbAuthBoundaryLedger`, `None` quand l'agent par défaut est absent.

## Ce que le vol 1 ne livre pas, et pourquoi c'est dit ici

- **`MIKA_MANAGER_EVALUATE_WRITE` et le commentaire sur le ticket** (AC2) — vol 2.
  Motif : taille (cf. § Taille estimée) et refus d'un interrupteur sans geste.
- **La valeur du plafond de tours** — elle exige une ligne `Turn ceiling: N`
  côté `dispatch-lib.sh`, parce que la lire en Rust voudrait dire dupliquer
  `PILOT_LABEL_TURN_CEILINGS`, qui se déclare à lecteur unique. Vol 2.
- **Le tour du dernier commit** — seul datum sans source structurée existante ;
  il faudrait le log de session pilote (`$PILOT_LOG_DIR/<task-id>.log`) ou un
  marquage producteur. Vol 2, par le même geste que le point précédent.
- **Dépendance mika#2697 (rapports lisibles).** Elle n'est pas une dépendance de
  *code* : le vol 1 écrit dans le corps du rapport, pas dans le sink. Elle est
  une dépendance de *mesure* — la fenêtre de 10 vols ne s'ouvre que lorsque le
  rapport est effectivement lisible sur disque. Le vol 1 peut donc fusionner
  avant mika#2697 ; le compteur de l'opérateur, non.

## Contrat de vérification

- `cargo test -p mika-agent milestone_manager` — vert, y compris
  `no_dispatch_scaffolding_in_milestone_manager` **et** son nouveau contrôle négatif.
- `cargo test -p mika-agent perimeter` — les broches de Porte 1 tiennent
  (le nouveau module est classé `DecisionCore`, pas de répertoire `tests/` imbriqué).
- `cargo test -p mika-agent` — suite complète.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- `scripts/verify-pipeline.sh origin/main`.
- `make verify-bundled-skills` (aucun bundle touché, mais le vol 1 lit le format
  produit par `dispatch-lib.sh` : la garde reste pertinente).
- Preuve manuelle du mode ombre : un cycle sur un milestone réel, puis
  `grep -c 'gh issue comment' <sink>/*.md` ⇒ 0, et la section « Vols terminés »
  présente.

## Definition of Done

- `milestone_manager/evaluator.rs` livré, déclaré, ré-exporté ; tests inline.
- `flight_source.rs` + `Database::list_finished_implement_flights` livrés ;
  aucune migration, aucun DDL.
- `Assessment::flight_verdicts` livré avec ses deux tests de sérialisation.
- Section « Vols terminés » rendue conditionnellement par `reporter.rs`.
- Injection câblée de `server/mod.rs` jusqu'à `CycleContext`, `None`-safe.
- `no_dispatch_test.rs` : contrôle négatif livré, allowlist d'exceptions
  **vide** et épinglée vide.
- Le docstring de module de `milestone_manager/mod.rs` dit ce que la Phase 2 (b)
  vol 1 a ajouté et ce qu'elle n'a **pas** ajouté (aucune autorité d'écriture).
- Vérification complète passée ; PR ouverte avec `Code review: delegated to
  mika-qa (mika#2694)` ; pas de `Closes` (AC2 reste dû — le ticket reste ouvert
  pour le vol 2).

## Acceptance criteria

Transcrits depuis le corps de mika#2698. La colonne « vol » dit lequel des deux
vols les tient ; le vol 1 est celui que ce plan livre.

- [ ] **AC1** — avec l'interrupteur OFF, aucun appel d'écriture `gh`. Garantie
      structurelle par l'extension de `no_dispatch_test.rs`, avec un contrôle
      négatif. *(vol 1 : forme forte — aucun chemin d'écriture n'existe, et le
      contrôle négatif est livré ; vol 2 : forme nominale, avec l'exception
      nommée que l'arrivée de `gh issue comment` rend nécessaire.)*
- [ ] **AC2** — avec l'interrupteur ON, un seul commentaire par vol (idempotent
      sur le rejeu du cycle). *(vol 2.)*
- [ ] **AC3** — tests sur fixtures des trois issues (PR, sauvetage, mort au
      plafond) avec les tours lus correctement. *(vol 1, entier.)*
- [ ] **Taille** — à estimer tests compris. Ils pesaient environ ×4 du code sur
      mika#2693. Découper en vols si l'estimation dépasse la borne par vol.
      *(vol 1 : découpage effectué, cf. § Découpage en vols et § Taille estimée.)*

## Fire-Disposition

Ce plan livre un détecteur : l'extension de
`milestone_manager/no_dispatch_test.rs`, dont le chemin de succès est « aucune
violation trouvée », plus son contrôle négatif. La section est donc due.

**Option retenue : (a), exception nommée en allowlist — avec une allowlist
livrée VIDE et épinglée vide.**

C'est la forme honnête ici, parce que **le vol 1 ne produit aucune violation**.
Le détecteur existant parcourt tout `src/milestone_manager/` ; le nouveau
`evaluator.rs` y atterrit et sera scanné automatiquement. Il ne porte aucun des
vingt-deux jetons interdits : pas de `gh`, pas de `"POST"`, pas de
`dispatcher_source`. Le SQL, lui, vit dans `src/flight_source.rs` et `src/db.rs`,
**hors** du répertoire parcouru — placement délibéré, pas accident : c'est le
partage trait/impl du précédent `AuthBoundaryLedger`, et il évite d'avoir à
excepter quoi que ce soit.

Détail d'implémentation, trois gestes :

1. **Une allowlist explicite, vide, grep-visible.** Une constante
   `EXEMPTED_OCCURRENCES: &[(&str, &str, &str)]` — `(fichier, jeton, ticket de
   suivi)` — livrée `&[]`, avec un commentaire disant qu'une entrée n'est
   légitime qu'accompagnée d'un ticket. C'est l'idiome « allowlist shipped
   **empty** » déjà en vigueur dans ce dépôt (mika#2425, mika#1833).
2. **L'assertion auto-nettoyante.** Un test
   `mika2698_lallowlist_dexceptions_est_vide_et_le_reste` rougit dès qu'une
   entrée est ajoutée sans que ce test soit amendé dans le même commit. C'est
   la version « vide » de l'exception auto-nettoyante : l'exception ne peut pas
   devenir stale si elle ne peut pas naître en silence.
3. **Le contrôle négatif, exigé nommément par AC1.** Aujourd'hui le test ne
   prouve pas qu'il attraperait quelque chose : il lit le disque et le disque
   est propre, donc vert et muet — un vert qui ne distingue pas « rien à
   trouver » de « ne trouve rien » (classe mika#2205). Correctif : extraire
   `scan_source(path: &str, source: &str) -> Vec<(String, String)>` du corps de
   `walk_and_scan`, puis `mika2698_le_scan_rougit_sur_une_source_fautive`
   lui donne une source littérale portant `gh issue comment` et exige un
   signalement. Le littéral est sans danger : `no_dispatch_test.rs` est
   lui-même dans `EXEMPT_FILES`.

Les deux autres options sont écartées, et pour des raisons, pas par défaut :
**(b) livrer désarmé** n'a pas de sens — le détecteur est déjà armé et déjà
vert en production ; le désarmer pour livrer serait une régression franche.
**(c) halte-et-remontée** n'a rien à remonter : il n'y a ni violation ni
arbitrage ouvert sur ce vol.

**Trajectoire annoncée, pour que le vol 2 ne la redécouvre pas.** Quand
`gh issue comment` atterrira derrière `MIKA_MANAGER_EVALUATE_WRITE`, il
deviendra la première entrée de `EXEMPTED_OCCURRENCES`, et le docstring de
`no_dispatch_test.rs` impose déjà le protocole : *« update this test AND the
module docstring atomically — never silently loosen the test »*. L'exception
nommera le fichier, le jeton `gh issue comment`, le ticket mika#2698, et rien
d'autre : ni `gh issue edit`, ni `gh pr comment`, ni une exception par
répertoire. Un trou nommé au jeton près reste un trou lisible.

Note : `strip_line_comments` retire `//` mais **pas** les blocs `/* */` — écart
documenté et bruyant par construction (un jeton interdit dans un bloc fait
rougir). Le vol 1 ne l'élargit pas et n'écrit aucun commentaire de bloc
contenant un jeton interdit.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (types, trait `FlightSource`, dérivation, lecture des lignes `result` et de `metadata.claude_pilot`) | 240 |
| tests inline de `evaluator.rs` (AC3 : fixtures PR / sauvetage / mort au plafond, matrice de recommandation, tours) — les trois fixtures partagent un constructeur `fn flight(outcome, halt, turns)`, d'où le rapport tests/code de ×1,1 et non ×4 | 230 |
| `crates/mika-agent/src/flight_source.rs` (`DbFlightSource`) | 55 |
| `crates/mika-agent/src/db.rs` (`list_finished_implement_flights`) | 30 |
| tests inline de `flight_source.rs` + `db.rs` | 60 |
| `crates/mika-agent/src/milestone_manager/types.rs` (champ `Assessment::flight_verdicts`) | 20 |
| tests inline de `types.rs` (serde `default` / roundtrip `[]`) | 40 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (section « Vols terminés ») | 50 |
| tests inline de `reporter.rs` (section absente / présente / trois formes de verdict) | 45 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` (`with_flight_source`, appel borné par le checkpoint) | 40 |
| tests inline de `cadence.rs` (source absente, source vide, bornage par `last_observed_at`) | 45 |
| `crates/mika-agent/src/milestone_manager/spawn.rs` + `src/server/mod.rs` (injection) | 30 |
| `crates/mika-agent/src/milestone_manager/mod.rs` (déclaration, ré-exports, docstring) | 25 |
| `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` (extraction de `scan_source`, allowlist vide, contrôle négatif) | 60 |

Total estimé : 970 lignes

Sous le seuil de 1000, et **par découpage, pas par optimisme** : le ticket
entier, mesuré d'un bloc, pesait ~1260 lignes. Ce qui en sort est nommé et
chiffré — le vol 2 (interrupteur `MIKA_MANAGER_EVALUATE_WRITE`, geste
`gh issue comment`, idempotence par checkpoint, exception nommée en allowlist,
`Turn ceiling: N` côté producteur, tour du dernier commit) est estimé à ~290
lignes et renvoyé à la **phase nommée « Phase 2 (b) vol 2 »** du § Découpage en
vols, qui dit aussi quels AC il porte.

Marge de 30 lignes seulement : si la mesure réelle dérive pendant
l'implémentation, le poste à sortir en premier est
`list_finished_implement_flights` + `flight_source.rs` + leurs tests (~145
lignes), au profit d'un premier déploiement en ombre où la source est injectée
en mémoire et la lecture DB arrive au vol 2. La décision se prend sur une
mesure, pas sur une impression : `git diff --stat origin/main` hors `docs/`.
