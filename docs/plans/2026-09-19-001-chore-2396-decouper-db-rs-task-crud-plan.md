# mika#2396 — Sortir `Task CRUD` de `db.rs` : la classe de risque annoncée n'existe pas, et la mesure dit pourquoi

## Contexte

Ce ticket est le premier des trois suivis que le § *Hors périmètre* du plan de
mika#2321 ouvre explicitement. Sa décision D5 les écarte tous les trois avec un
argument de **classe de risque** :

> `Task CRUD` fait 186 752 o et vit au milieu de 313 méthodes `pub` et 537 `fn`
> privées dont les helpers traversent les sections. Les sortir demande, pour
> chaque helper privé franchissant une frontière, la même question que D3
> tranche pour les migrations — mais sans la réponse simple, parce que les
> appelants sont dispersés au lieu d'être un dispatcher unique. C'est un travail
> de jugement par méthode, pas un déplacement de blocs.

**Cette phrase est fausse à la mesure, et la raison pour laquelle elle est fausse
est le seul contenu non trivial de ce plan.** D5 raisonne sur l'existence de
helpers franchissant la frontière — qui existent bel et bien — sans regarder la
**direction** de la relation de module que le découpage installe. En Rust un
module enfant voit les items privés de son parent ; deux frères ne se voient pas.
`db/tasks.rs`, déclaré `mod tasks;` dans `db.rs`, est un **enfant** de `db`. Tout
helper privé qui **reste** dans `db.rs` reste donc visible depuis la section
déplacée, sans une seule signature touchée. Le seul jugement à rendre porte sur
les helpers que le déplacement **emporterait** — et la mesure ci-dessous en
dénombre exactement deux.

Ce plan est un refactor pur : aucun changement de comportement à l'exécution,
aucune chaîne SQL modifiée, aucune sémantique déplacée.

**Ce que ce plan n'achète PAS : le plafond.** `scripts/check-secrets.sh` refuse
au-delà de 1 048 576 o et `db.rs` y est exempté par nom depuis mika#2310.
mika#2321 est le ticket qui retire cette exemption, et A+B l'amènent seuls à
~425 Ko. Sortir `Task CRUD` n'est pas requis pour franchir le plafond — c'est
une **conséquence** ici, pas une justification (cf. D6). Sa justification est la
lisibilité d'un fichier qui, même après mika#2321, garde ~380 Ko de sections
`impl Database` dont `Task CRUD` est de loin la plus grosse.

## Ce qui est établi, et comment le vérifier

Mesures relevées dans ce worktree sur `f833db77`.

### E1 — La section, ses bornes, son poids

`Task CRUD` court de la ligne **6131** (`// ===== Task CRUD =====`) à la ligne
**10253** incluse, la 10254 portant `// ===== Sessions =====`.

| mesure | valeur | commande |
|---|---|---|
| lignes | 4 123 | bornes des marqueurs, `grep -n '^    // ====='` |
| octets | **186 752** | `head -10253 … \| wc -c` (458 033) moins `head -6130 … \| wc -c` (271 281) |
| méthodes `pub fn` | 111 | dénombrement de E8 |
| `fn` privées | 3 | `spend_unknown_trigger_lift`, `row_to_task`, `parse_process_start_time` |
| `const` associées | 2 | `TASK_COLUMNS`, `TASK_COLUMN_COUNT` |

`db.rs` pèse aujourd'hui **1 108 301 o / 27 594 lignes** (`wc -c -l`). Après
retrait : **921 549 o**. C'est sous le plafond, et c'est un effet de bord — voir
D6 pour pourquoi ce plan ne touche pas l'allowlist malgré ça.

### E2 — L'idiome d'extraction est doublement établi pour ce domaine précis

`crates/mika-agent/src/db/` porte déjà `operational.rs` (37 611 o),
`kg_schema.rs` (20 980 o) et `tests/harnais_porte.rs`, déclarés
`pub mod kg_schema; pub mod operational;` aux lignes 1–2 de `db.rs`. La
coexistence `db.rs` + `db/` fonctionne (layout Rust 2018) ; aucun renommage en
`db/mod.rs` n'est nécessaire. `db/operational.rs` donne la forme exacte, en
trois lignes :

```rust
use super::Database;

impl Database {
    pub fn insert_operational_item(&self, …) -> Result<String> { … }
}
```

**Et le second précédent est plus proche encore : les types de ce domaine sont
déjà sortis.** `db.rs:16` porte `pub use crate::task_state::tasks::*;`, et
`crates/mika-agent/src/task_state/tasks.rs` (20 721 o, 454 lignes) détient
`Task`, `NewTask`, `TaskHealthSummary`, `BackgroundTaskCounts`, `DispatchChild`,
`OrphanedParentTask`, `PhantomTrackingTask`, `CompletableParentTask`,
`ChildlessStuckParent`, `OrphanedPendingTask`, `DeferredWrapperSummary`,
`StaleBlockedTask`, `ReaperChildSnapshot`, `ForcePromoteResult` et les
constantes du domaine. **Le découpage du domaine `tasks` est donc déjà à
moitié fait**, sur son axe le plus exposé (les types, que ~30 modules importent),
et ce plan ne fait que porter le second axe.

Conséquence décisive, déjà vraie en production : **les appelants ne bougent
pas.** Une méthode reste `Database::foo` quel que soit le fichier qui porte son
`impl`. `async_db.rs` (156 201 o), qui enveloppe chaque méthode par dispatch de
closure, est intouché.

### E3 — Un seul couple traverse réellement la frontière, et la mesure le nomme

Les trois `fn` privées et les deux `const` de la section, avec leurs appelants :

| item | déf. | appelants hors 6131–10253 | verdict |
|---|---|---|---|
| `spend_unknown_trigger_lift` | 6368 | **aucun** (seul 6334, interne) | part avec |
| `parse_process_start_time` | 8106 | **aucun** (8165, 8245, internes) | part avec |
| `TASK_COLUMN_COUNT` | 7014 | **aucun** (7053, interne) | part avec |
| **`row_to_task`** | 6557 | **12186** (Rewind), **13551**, **13599** (Dashboard: Paginated Task Listing), **13972**, **14033** (Dashboard: Dev Runs) | **reste** |
| **`TASK_COLUMNS`** | 6594 | **12174** (Rewind), **13549**, **13582**, **13969**, **14010** (Dashboard) | **reste** |

Vérification : `grep -n 'row_to_task\|TASK_COLUMNS\|TASK_COLUMN_COUNT\|spend_unknown_trigger_lift\|parse_process_start_time' crates/mika-agent/src/db.rs`.

Le couple `TASK_COLUMNS` + `row_to_task` **est** le désérialiseur de `Task`,
partagé par trois régions (`Task CRUD`, `Rewind`, `Dashboard`). C'est l'unique
« helper traversant la frontière » que D5 redoutait, et il y en a un, pas
trente-sept. D2 tranche son allocation.

### E4 — Dans l'autre sens, la relation parent/enfant paie tout

La section appelle un helper défini **hors** d'elle : `format_age` (`db.rs:14098`,
`fn` libre privée au module `db`), à cinq sites — 7382, 7402, 7430, 7457, 7489,
tous dans `get_task_health_summary`. Vérification :
`grep -n 'format_age' crates/mika-agent/src/db.rs`.

`db::tasks` étant enfant de `db`, `format_age` reste visible sans un caractère de
changement. C'est exactement la propriété que D3 de mika#2321 exploite pour les
migrations, et elle s'applique ici sans la contrepartie que D5 imaginait.

Recensement des autres candidats (`truncate_utf8_safe` 10389,
`insert_task_message_tx` 11164, `column_exists` 5756, `current_version` 1088,
`push_csv_in_clause` 13606, `build_task_filter_sql` 13624, `truncate_chars`
14086) : **aucun n'a d'appelant dans 6131–10253**. Même commande.

### E5 — Les tests n'appellent aucun privé de la section, et c'est ce qui rend ce ticket indépendant de mika#2321

Le module de test va de 14781 à 27594 (473 763 o, 431 `#[test]`). Les cinq items
privés de E3 n'ont **aucune occurrence au-delà de la ligne 14780** — c'est
lisible directement dans la sortie du `grep` de E3.

Conséquence : les tests de `Task CRUD` n'exercent la section que par ses méthodes
`pub` sur `Database`, résolues quel que soit le fichier qui les porte. **Le module
de test n'a donc pas à bouger**, et D3 pose ce non-déplacement comme une décision
mesurée plutôt que comme une économie.

### E6 — mika#2321 n'est pas mergé, et les régions sont disjointes

`git merge-base --is-ancestor origin/feat/2321/… origin/main` → faux. Dernier
commit de la branche : `4c9d6846`, 2026-09-18 23:45. `db.rs` sur `main` porte
toujours ses 27 594 lignes.

| ticket | volet | région de `db.rs` touchée |
|---|---|---|
| mika#2321 | A | 14781–27594 (module de test) + la garde en 14855 |
| mika#2321 | B | 1111–5765 (échelle de migrations) |
| **mika#2396** | — | **6131–10253 (`Task CRUD`)** |

Trois plages **deux à deux disjointes**. Un merge git de hunks disjoints d'un même
fichier ne conflicte pas. Les deux seuls points de contact sont nommés en F1.

### E7 — La garde mika#2335 est verdict-neutre ici, et la raison est positionnelle

`db.rs:14855` (F2a de mika#2335) énumère les `.rs` sous `src/`, tronque chaque
fichier au premier littéral `#[cfg(test)]`, neutralise les lignes de commentaire,
puis cherche `update_manual_task_status` avec `"in_progress"` dans une fenêtre de
200 caractères. E7 du plan mika#2321 établit que la prémisse « le code de test
vit derrière un `#[cfg(test)]` inline » est déjà fausse pour deux fichiers.

**Ce plan ne fait pas passer un octet de plus par ce trou, et la raison est
mesurable, pas conjecturale :** la section `Task CRUD` est **déjà** intégralement
dans la moitié scannée comme production, puisqu'elle se situe avant la troncature
à 14781. Extraire dans `db/tasks.rs` — fichier sans `#[cfg(test)]`, donc scanné
en entier — soumet les mêmes octets au même scan.

Restent les occurrences de l'aiguille que la section emporte : la définition en
6894 (dont la fenêtre de 200 caractères ne contient que la signature et un SELECT
SQL à guillemets simples, jamais le littéral Rust `"in_progress"`), et quatre
mentions en 6934 / 6940 / 6947 / 6974 — dont **6934 porte littéralement**
`update_manual_task_status(…, "in_progress")`, mais sur une ligne `///`,
neutralisée avant le scan. C'est précisément la clause que la doc de la garde
prévoit pour la doc de `mark_parent_dispatched`.

**Raisonnement suffisant pour dimensionner le risque, insuffisant pour clore :**
V3 exige la vérification empirique, pour la même raison que V3 de mika#2321 —
la marge dépend du reflow `cargo fmt` d'une fenêtre de 200 caractères, et ce
n'est pas une propriété sur laquelle parier une porte CI.

La seconde garde du même trou, `production_half` dans `agent_loop/mod.rs:14279`
(compteur de lecteurs de `HistoryScope`, mika#2305), est hors de question ici :
`db/tasks.rs` ne contiendrait aucun lecteur de `HistoryScope`
(`grep -rn 'HistoryScope' crates/mika-agent/src/db.rs` → zéro).

### E8 — Six méthodes de la plage ne concernent pas `tasks`

La plage 6131–10253 porte six méthodes dont la table n'est pas `tasks` :

| méthode | ligne | table |
|---|---|---|
| `count_recent_audit_events_for_target` | 7070 | `audit_events` |
| `get_schema_meta` | 7910 | `schema_meta` |
| `stamp_schema_meta_epoch_if_absent` | 7935 | `schema_meta` |
| `count_audit_events_by_tool_name` | 8260 | `audit_events` |
| `get_audit_event_target_keys_by_tool_name` | 8327 | `audit_events` |
| `get_audit_event_rows_by_tool_name` | 8351 | `audit_events` |

Poids : les deux `schema_meta` font 3 073 o (7910–7971) ; le bloc 8260–8393, qui
les mêle aux helpers de test, 6 187 o. Ordre de grandeur des six : **~8 Ko sur
187**. D4 tranche.

À ne pas confondre avec elles : `backdate_task_updated_at` (8276),
`backdate_task_completed_at` (8296) et `set_task_id_for_test` (8315) portent
`#[doc(hidden)]` et sont des helpers de test — mais leur SQL est `UPDATE tasks`.
Ce sont des méthodes du domaine et elles partent avec lui.

### E9 — Ce qui est verdict-neutre, et ce qui ne l'est pas

- `scripts/check-byte-slices.sh` scanne **tout** `crates/`
  (`SCAN_ROOT="${1:-$REPO_ROOT/crates}"`), jamais le diff : un déplacement y est
  verdict-neutre par construction.
- `scripts/check-secrets.sh` est diff-scopé (`--changed origin/main`). Le fichier
  créé est *ajouté* et **sera** scanné ; il fait 180 Ko, très loin du plafond.
  `db.rs`, modifié, sera scanné aussi — et il reste couvert par l'entrée
  d'allowlist que ce plan ne touche pas (D6), donc aucune contrainte d'ordre de
  commit ne pèse ici, contrairement à F1 de mika#2321.
- `perimeter/rules.rs` ne nomme pas `db.rs` et n'a pas de règle qui distingue
  `db.rs` de `db/tasks.rs` : une PR touchant l'un ou l'autre est classée par les
  mêmes règles. Le découpage ne change pas la classe de périmètre des PR futures.

## Décisions

### D1 — `db/tasks.rs`, fichier unique, et **pas** un répertoire

Décision opposée à D2 de mika#2321, et l'opposition est raisonnée plutôt
qu'incohérente : les trois raisons qui imposaient un répertoire là-bas tombent
toutes les trois ici.

1. **Classification du périmètre.** `perimeter/rules.rs:177` porte `"/tests/"`
   dans `MECHANICAL_CONTAINS`, et `perimeter/tests.rs:170–182` épingle
   `some_module/tests.rs → DecisionCore` par choix fail-closed explicite. Cette
   asymétrie ne concerne que les **modules de test** : elle est sans effet sur un
   fichier de code de production, qui est `DecisionCore` de toute façon — comme
   `db.rs` l'est déjà.
2. **`harnais_porte` y vit déjà.** Argument propre au répertoire `db/tests/`.
3. **Croissance.** Un `db/tests.rs` unique naîtrait à 463 Ko. `db/tasks.rs` naît
   à 180 Ko, soit 17 % du plafond, dans une région qui churne (une méthode y est
   réécrite, remplacée, supprimée) plutôt que dans une région monotone.

Un répertoire `db/tasks/` répartissant 180 Ko en sous-thèmes serait un second
découpage, par jugement, à l'intérieur d'un premier — et c'est exactement ce que
D5 refuse par ailleurs.

### D2 — `TASK_COLUMNS` et `row_to_task` restent dans `db.rs`

E3 mesure leurs appelants : cinq et cinq, hors de la section, dans `Rewind` et
`Dashboard`. Deux allocations étaient possibles et une seule est gratuite.

- **Les emporter** ferait de `db` et `db::tasks` des frères vis-à-vis d'elles :
  les dix sites de `Rewind`/`Dashboard` cesseraient de compiler, et le remède
  serait `pub(super)` sur la `fn` **et** sur la `const` — deux élargissements de
  visibilité, pour loger le désérialiseur de `Task` dans un fichier qui n'est pas
  son seul consommateur.
- **Les laisser** : `db::tasks` est enfant, il les voit ; les dix sites ne
  bougent pas ; **zéro élargissement de visibilité dans tout le plan**.

Le second choix est le bon et il coûte une phrase de doc dans `db/tasks.rs`
nommant les deux items restés au parent et pourquoi. Corollaire assumé : `db.rs`
garde ~1,5 Ko qui « ressemblent » à du Task CRUD. C'est le prix, il est dit, et
il est inférieur à deux élargissements de visibilité dans un refactor dont
l'argument de sûreté est justement qu'il n'en demande aucun.

### D3 — Le module de test ne bouge pas

Non pas par économie mais par mesure (E5) : aucun test n'atteint un privé de la
section. Déplacer des tests serait donc un déplacement **gratuit** — sans
bénéfice de compilation, sans bénéfice de visibilité — et il coûterait trois
choses réelles : (a) un chevauchement textuel intégral avec le volet A de
mika#2321, qui déplace précisément cette région ; (b) l'entrée dans le trou de
mika#2310 (un fichier de test extrait ne porte pas de `#[cfg(test)]` et se fait
scanner comme de la production par les deux gardes de E7) — c'est la réparation
que le volet A de mika#2321 porte, et l'emprunter avant lui, c'est la livrer
deux fois ; (c) le risque de perte silencieuse d'un test, que F2 nomme.

Corollaire : **ce ticket ne réduit pas la région append-only**. Il ne prétend pas
« borner la croissance » — c'est l'objet de mika#2321, dit ici pour que personne
ne lise ce plan comme le faisant.

### D4 — Les six méthodes de E8 restent dans `db.rs`

Un fichier nommé `db/tasks.rs` qui porterait quatre requêtes `audit_events` et
deux `schema_meta` poserait un nom faux dès sa première ligne, et le nom d'un
fichier est ce sur quoi le prochain lecteur s'appuie pour chercher.

Le coût est réel et se mesure : le déplacement cesse d'être une plage contiguë
unique et devient **quatre plages**, ce qui affaiblit la vérification la plus
forte disponible (« le diff est un déplacement, pas une réécriture »). Quatre
plages restent vérifiables une par une — V2 le fait — et le nom juste vaut cette
dépense. Les six méthodes restent donc exactement où elles sont ; leur regroupement
thématique éventuel est un autre ticket, qui n'est pas ouvert ici parce que rien
ne le demande.

**Refusé : re-trier l'intérieur de la section.** Les 111 méthodes déplacées le
sont dans leur ordre d'origine. Réordonner à l'occasion d'un déplacement rend le
diff illisible et la vérification de V2 impossible.

### D5 — Aucune autre section n'est sortie

Le ticket porte sur `Task CRUD`. `grooming/verdict` et `retention/prune`, les
deux autres suivis de D5 de mika#2321, ne sont pas dans ce périmètre — et la
mesure ci-dessus ne dit rien d'eux : elle établit que la relation parent/enfant
paie **pour cette section-là**, dont E3 dénombre les traversées. Une autre
section peut avoir une autre forme, et le même recensement devra être refait pour
elle. Généraliser « l'extraction est mécanique » à partir d'un cas mesuré serait
la conclusion que ce plan reproche à D5 d'avoir tirée dans l'autre sens.

### D6 — `LARGE_FILE_ALLOWLIST` n'est pas touchée

`db.rs` passe sous le plafond (E1 : 921 549 o) et l'entrée d'allowlist de
`scripts/check-secrets.sh:44` pourrait donc être retirée ici. Elle ne l'est pas.

Retirer cette entrée est le livrable écrit de mika#2321 (son AC3 et son A5), et
les deux tickets sont en vol simultanément. Deux PR qui suppriment la même ligne,
c'est un conflit textuel garanti sur le fichier le plus sensible du lot et, pire,
un livrable qui disparaît de l'un des deux tickets sans que son AC en rende
compte. Une exemption posée sur un fichier qui est passé sous le plafond ne
protège rien et ne cache rien : elle est inerte jusqu'à ce que mika#2321 la
retire.

**Halte explicite :** si mika#2321 est abandonné, retirer l'entrée redevient du
travail à faire — dans son ticket, ou dans un successeur nommé, jamais en passant
dans celui-ci.

### D7 — Les citations `db.rs:<ligne>` des `docs/solutions/**` ne sont pas mises à jour

Elles sont déjà fausses : les numéros dérivent à chaque commit sur un fichier de
27 594 lignes, et D6 de mika#2321 pose déjà cette décision avec son suivi. Les
réparer ici figerait des numéros qui dériveront au commit suivant, et le suivi
existe. Ce plan n'en ouvre pas un second.

## Volet d'implémentation

Un seul volet, quatre pas.

**P1. Créer `crates/mika-agent/src/db/tasks.rs`.**
En-tête : `//!` nommant le domaine, l'idiome (`use super::Database;`), **et** les
deux items restés au parent avec la raison de D2. Imports : `use super::Database;`
plus ce que la section consomme (`anyhow::Result`, `rusqlite::{OptionalExtension,
params, …}`, `crate::timestamp`, `crate::task_state::tasks::*` si `super::*`
n'est pas retenu, `tracing::{debug, info, warn}` selon les sites déplacés). Un
unique bloc `impl Database { … }`.

**P2. Déplacer les quatre plages.**
Bornes exactes, dérivées de E1 et E8 (frontières à ajuster aux lignes vides et aux
blocs `///` qui précèdent chaque méthode) :

| # | de | à | contenu |
|---|---|---|---|
| 1 | 6131 | 7069 | `create_task` → `list_manual_tasks` |
| 2 | 7091 | 7909 | `count_session_tasks` → `find_orphaned_parent_tasks` (moins `count_recent_audit_events_for_target`) |
| 3 | 7972 | 8259 | `find_orphaned_parent_tasks` (suite) → avant `count_audit_events_by_tool_name` |
| 4 | 8394 | 10253 | `find_completable_parent_tasks_on_pr_url` → `get_tasks_by_status_and_label` |

Restent dans `db.rs` : `TASK_COLUMNS` (6594), `row_to_task` (6557),
`count_recent_audit_events_for_target` (7070), `get_schema_meta` (7910),
`stamp_schema_meta_epoch_if_absent` (7935) et le bloc `audit_events` 8260–8393
**moins** les trois helpers `backdate_*` / `set_task_id_for_test` qui partent
(E8). Les plages ci-dessus sont indicatives : la frontière qui fait foi est
l'appartenance de la méthode au domaine, pas le numéro de ligne.

**P3. Déclarer le module.**
Ajouter `pub mod tasks;` aux lignes 1–2 de `db.rs`, à côté de `kg_schema` et
`operational`.

**P4. `cargo fmt` et rien d'autre.**
Aucune réécriture, aucun réordonnancement, aucun changement de visibilité.

## Verification contract

- **V1 — compilation et suite complète.** `cargo build` et
  `cargo test -p mika-agent` verts. Le nombre de tests exécutés est
  **strictement égal** au relevé pris sur la base avant le découpage — pas
  « comparable ». Un test perdu dans un refactor est la seule perte silencieuse
  possible (F2), et l'égalité du compte est ce qui la rend audible. Relever la
  baseline **avant** le premier déplacement.
- **V2 — le diff est un déplacement, pas une réécriture.** Pour chacune des
  quatre plages de P2, `diff` entre l'extraction de la plage sur `HEAD~` et la
  région correspondante de `db/tasks.rs` ne rend que le décalage d'indentation
  nul et rien d'autre. C'est la vérification qui rend D4 payable : quatre
  comparaisons plutôt qu'une, mais chacune exacte.
- **V3 — la garde mika#2335 est verte, et verte pour la bonne raison.**
  `cargo test -p mika-agent mika2335_no_production_dispatch_transitions_a_parent_without_stamping`
  passe (a) après le déplacement, et (b) **rougit** quand on injecte
  temporairement dans `db/tasks.rs` un site de production fautif — un
  `update_manual_task_status(id, agent, "in_progress")` hors commentaire. Sans
  (b), une garde devenue vacuous sur le nouveau fichier est indistinguable d'une
  garde qui va bien (E7).
- **V4 — le contrat inter-modules tient.** `async_db.rs` n'est pas modifié : ses
  ~111 closures de dispatch appellent `db.foo(…)` et sont la preuve que le chemin
  de résolution a survécu. Toute retouche de ce fichier est le signe que le
  déplacement a touché une signature.
- **V5 — zéro changement de visibilité.** Relecture dirigée du diff : aucun
  `pub(super)`, aucun `pub(crate)`, aucun `pub` ajouté ou retiré. C'est
  l'invariant central de D2 et il se vérifie à l'œil sur le diff.
- **V6 — aucune chaîne SQL modifiée.** Même relecture : chaque littéral SQL du
  diff apparaît une fois en `-` et une fois en `+`, identique.
- **V7 — tailles.** `db.rs` sous **950 Ko** ; `db/tasks.rs` sous **200 Ko** ;
  aucun `.rs` sous `crates/` n'atteint 1 048 576 o.
  `bash scripts/check-secrets.sh --changed origin/main` sort 0.
- **V8 — verdict-neutralité des lints pleine-arborescence.**
  `bash scripts/check-byte-slices.sh` rend le même verdict qu'avant (E9) ;
  `cargo clippy` et `cargo fmt --check` sont propres.
- **V9 — le module de test est intouché.** Le diff ne contient aucune ligne
  au-delà de l'ancienne 14780 de `db.rs`, hors décalage. C'est la vérification
  mécanique de D3, et donc de la disjonction avec le volet A de mika#2321.

## Fire-Disposition

- **F1 — les deux seuls points de contact avec mika#2321.** (a) Les lignes 1–2 de
  `db.rs` : ce ticket y ajoute `pub mod tasks;`, mika#2321 y ajoutera
  `pub mod migrations;` et `#[cfg(test)] pub(crate) mod tests;`. Conflit trivial,
  résolution évidente, mais à anticiper. (b) `LARGE_FILE_ALLOWLIST`, que D6 laisse
  intacte précisément pour qu'il n'y ait pas de second point de contact. **Aucune
  contrainte d'ordre de merge entre les deux tickets** : les trois plages de E6
  sont disjointes et chacun des deux plans reste applicable après l'autre, aux
  numéros de ligne près.
- **F2 — un test perdu est la seule perte silencieuse.** Tout le reste échoue à la
  compilation : une méthode oubliée, un import manquant, un privé devenu
  invisible. D'où l'égalité stricte de V1. Si le compte baisse, **ne pas
  rééquilibrer l'attendu** — retrouver le test manquant par différence de noms.
- **F3 — si la garde mika#2335 rougit après P2, halte.** Ne pas élargir
  l'aiguille, ne pas poser d'exemption par fichier : c'est le mouvement que la doc
  de la garde refuse en toutes lettres et que D1 de mika#2321 refuse pour le même
  trou. Établir d'abord si le site signalé est un vrai site de production (→ il se
  corrige) ou si le déplacement a fait franchir la fenêtre de 200 caractères à une
  paire jusque-là séparée (→ c'est E7 qui est à reprendre, et la réparation de
  classification du volet A de mika#2321 devient une précondition).
- **F4 — si un privé devient invisible, ne pas élargir par réflexe.** Le compilateur
  nommera l'item. La question à poser d'abord est celle de D2 : cet item a-t-il des
  appelants des deux côtés de la frontière ? Si oui, il **reste au parent**. Un
  `pub(super)` ajouté ici est le signe que le recensement de E3 était incomplet,
  pas que l'item devait voyager.
- **F5 — si le worktree part d'une base où mika#2321 a merǵé**, refaire E1 (les
  bornes auront bougé) mais pas E3/E4/E5 : aucune des deux plages de mika#2321 ne
  contient un item recensé là.

## Definition of Done

- `crates/mika-agent/src/db/tasks.rs` existe, porte les ~111 méthodes du domaine
  sous un unique `impl Database`, et son en-tête nomme les deux items restés au
  parent avec la raison.
- `db.rs` déclare `pub mod tasks;` et conserve `TASK_COLUMNS`, `row_to_task`, les
  quatre méthodes `audit_events` et les deux `schema_meta`.
- Le module de test n'est pas déplacé ; `LARGE_FILE_ALLOWLIST` n'est pas modifiée.
- `db.rs` est sous 950 Ko ; `db/tasks.rs` sous 200 Ko ; aucun `.rs` du crate
  n'atteint 1 Mo.
- V1–V9 passent ; le compte de tests est strictement égal ; le diff ne contient
  **aucun** changement de visibilité et **aucune** chaîne SQL modifiée.
- `cargo build`, `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`
  propres.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les critères
ci-dessous sont dérivés de son objet (« découper `db.rs`, volet `task CRUD` »,
suivi de mika#2321) et du Verification contract.

- **AC1** — La section `Task CRUD` (les ~111 méthodes `pub` du domaine `tasks`,
  plus `spend_unknown_trigger_lift`, `parse_process_start_time` et
  `TASK_COLUMN_COUNT`) vit dans `crates/mika-agent/src/db/tasks.rs`, déclaré
  `pub mod tasks;` dans `db.rs`.
- **AC2** — `TASK_COLUMNS` et `row_to_task` sont restés dans `db.rs`, et les dix
  sites de `Rewind` / `Dashboard` qui les consomment (12174, 12186, 13549, 13551,
  13582, 13599, 13969, 13972, 14010, 14033 sur la base actuelle) sont inchangés.
- **AC3** — Le diff contient **zéro** changement de visibilité : aucun `pub`,
  `pub(crate)` ou `pub(super)` ajouté, retiré ou modifié.
- **AC4** — Le diff contient **zéro** chaîne SQL modifiée et **zéro** signature
  publique modifiée. `async_db.rs` n'est pas touché.
- **AC5** — `cargo test -p mika-agent` est vert et le nombre de tests exécutés est
  **strictement égal** à celui mesuré sur la base avant le découpage.
- **AC6** — Le module de test de `db.rs` n'est pas déplacé : le diff ne porte
  aucune ligne au-delà de l'ancienne 14780, hors décalage.
- **AC7** — `LARGE_FILE_ALLOWLIST` dans `scripts/check-secrets.sh` est inchangée,
  et `bash scripts/check-secrets.sh --changed origin/main` sort 0.
- **AC8** — La garde `mika2335_no_production_dispatch_transitions_a_parent_without_stamping`
  est verte, **et** un contrôle négatif atteste qu'un site de production fautif
  injecté dans `db/tasks.rs` la fait rougir.
- **AC9** — `crates/mika-agent/src/db.rs` pèse moins de 950 Ko,
  `crates/mika-agent/src/db/tasks.rs` moins de 200 Ko, et aucun fichier `.rs` sous
  `crates/` n'atteint 1 048 576 o.
- **AC10** — `cargo clippy` et `cargo fmt --check` sont propres, et
  `bash scripts/check-byte-slices.sh` rend le même verdict qu'avant le découpage.

## Surfaces opérateur et sonde post-déploiement

**Aucune surface opérateur, et c'est un fait à écrire plutôt qu'une section à
remplir.** Ce travail ne change aucun comportement à l'exécution : pas d'événement
de journal, pas de ligne `audit_events`, pas de variable d'environnement, pas de
requête SQL, pas de migration. Inventer une télémétrie pour un refactor serait du
remplissage, et une sonde qui ne mesure rien se lit comme une sonde qui va bien.

La sonde est la suite de tests et le compilateur, et la halte est F2 : un compte
de tests qui baisse est le seul signal que quelque chose s'est perdu sans bruit.

## Hors périmètre (suivi à ouvrir)

- **Découpage de `grooming/verdict` et `retention/prune`** — les deux autres
  suivis que D5 de mika#2321 ouvre. Ce plan ne dit rien de leur forme : le
  recensement de E3 vaut pour `Task CRUD` et pour elle seule (D5 ci-dessus).
- **Regroupement thématique des six méthodes de E8** (quatre `audit_events`, deux
  `schema_meta`), restées dans `db.rs` faute d'un fichier où aller. Rien ne le
  demande aujourd'hui ; le jour où `db/audit.rs` existera, elles ont une cible.
- **Découpage interne de `db/tasks.rs`.** 180 Ko et 111 méthodes couvrant le CRUD,
  les reapers, les slots de dispatch, les claims de worktree et les callbacks
  différés. La frontière naturelle suivante est là, mais elle demande le même
  recensement de traversées que E3, sur une section qui n'a pas de marqueurs
  internes — donc un ticket à elle, conditionné à une mesure qui le demande.
- **Citations `db.rs:<ligne>` dans `docs/solutions/**`** (D7) — déjà couvertes par
  le suivi ouvert par D6 de mika#2321 ; rien à ouvrir de plus.
