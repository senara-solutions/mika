# mika#2321 — Découper `db.rs` : sortir les deux régions qui croissent par construction, et réparer la prémisse que le découpage invalide

## Contexte

`crates/mika-agent/src/db.rs` pèse **1 108 301 octets** (1082 Ko, 27 594 lignes)
contre un plafond de 1 048 576 octets appliqué par `scripts/check-secrets.sh`
dans le hook pre-commit et dans le job CI `secret-scan`. Le fichier est **59 725
octets au-dessus**, et il ne rougit pas parce que mika#2310 l'a exempté par nom
dans `LARGE_FILE_ALLOWLIST` pour débloquer #2320, en ouvrant ce ticket comme
dette.

Le ticket annonce 1026 Ko et le commentaire de doc de `db/tests/harnais_porte.rs`
annonce 1 043 979 octets « 4,6 Ko sous le plafond ». **Les deux chiffres sont
périmés, et l'écart est le fait central** : depuis que l'entrée d'allowlist est
posée, le fichier a pris ~64 Ko. Mesure sur trois jours et cinq commits :

| commit | date | non-test | tests | total |
|---|---|---|---|---|
| `5a7a50fb` | 2026-09-15 | 610 239 | 442 775 | 1 053 014 |
| `ff0c263d` | 2026-09-18 | 634 538 | 473 763 | 1 108 301 |
| **delta** | 3 j | **+24 299** | **+30 988** | **+55 287** |

L'entrée d'allowlist n'est donc pas une exemption statique : c'est une exemption
**non bornée**, et elle est la seule raison pour laquelle la croissance ci-dessus
est muette. C'est cela que le ticket appelle « borne la croissance ».

Ce plan est un refactor pur : **aucun changement de comportement à l'exécution,
aucune requête SQL modifiée, aucun octet de logique déplacé sémantiquement.** Sa
difficulté n'est pas dans le déplacement — elle est dans une prémisse partagée
par deux gardes structurelles du dépôt, que tout déplacement de code de test
invalide, et qui est **déjà fausse aujourd'hui** en deux endroits.

## Ce qui est établi, et comment le vérifier

Toutes les mesures ci-dessous ont été relevées dans ce worktree sur `2bc61f2d`.

### E1 — La répartition du fichier, et les deux seules régions qui croissent par construction

| région | lignes | octets | nature |
|---|---|---|---|
| types / préambule / fonctions libres | 1–996 | ~36 Ko | churn |
| `open` / `open_in_memory` | 1001–1110 | ~4 Ko | stable |
| **échelle de migrations** (`migrate` + `migrate_v1` → `migrate_v53_to_v54`) | 1111–5765 | **~210 Ko** | **append-only** |
| sections `impl Database` (Agent/Task/Session/LLM/Team/Search/Dashboard…) | 5766–14063 | ~380 Ko | churn |
| `impl Database` résiduels + fonctions libres | 14064–14780 | ~14 Ko | churn |
| **`#[cfg(test)] mod tests`** | 14781–27594 | **473 763** (463 Ko) | **append-only** |

Vérification : `wc -c -l crates/mika-agent/src/db.rs` →
`27594 1108301` ; `head -14780 … | wc -c` → `634538` ;
`tail -n +14781 … | wc -c` → `473763` ;
`grep -c '#\[test\]' db.rs` → `431` ;
`grep -c '^    pub fn ' db.rs` → `313`.

**L'argument de périmètre ne repose pas sur une extrapolation de trois jours.**
Le taux mesuré (+18 Ko/j) est un régime de burst — le commentaire opérateur du
ticket dit lui-même « régime saturation de nuit » — et l'extrapoler linéairement
serait malhonnête. L'argument est structurel : **une migration n'est jamais
supprimée** (v1…v54, ajout pur, pour toujours) et **un test l'est rarement**.
Ce sont les deux seules régions de `db.rs` dont la taille est monotone. Les 380 Ko
de sections `impl Database` croissent aussi, mais elles churnent : une méthode y
est réécrite, remplacée, supprimée. Borner `db.rs` veut donc dire **sortir ses
deux régions monotones**, et cette phrase reste vraie quel que soit le taux.

### E2 — Le répertoire `db/` existe déjà, et l'idiome d'extraction est prouvé

`crates/mika-agent/src/db/` porte déjà `operational.rs` (37 611 o),
`kg_schema.rs` (20 980 o) et `tests/harnais_porte.rs` (6 102 o), déclarés
`pub mod kg_schema; pub mod operational;` aux lignes 1–2 de `db.rs`.

La coexistence `db.rs` + `db/` fonctionne (layout Rust 2018) : **aucun renommage
en `db/mod.rs` n'est nécessaire**. Et `db/operational.rs` montre l'idiome exact,
en trois lignes :

```rust
use super::Database;

impl Database {
    pub fn insert_operational_item(&self, …) -> Result<String> { … }
}
```

Conséquence décisive : **les appelants ne bougent pas.** Une méthode reste
`Database::foo` quel que soit le fichier qui porte son `impl`. `async_db.rs`
(156 201 o), qui enveloppe chaque méthode par dispatch de closure, et les ~30
modules qui font `use crate::db::…` sont **intouchés**. Ce n'est pas une
espérance : c'est ce que `operational.rs` et `kg_schema.rs` font déjà en
production.

### E3 — `impl Database` est déjà sectionné thématiquement

Plus de 30 marqueurs `// ===== X =====` bornent les sections :
`Agent CRUD` (5766), `Skill Overrides` (5831), `Team CRUD` (6104),
`Task CRUD` (6131–10253, **186 752 o**, la plus grosse), `Sessions` (10254),
`LLM Calls` (10380), `Tool Calls` (10456), `Messages` (11124),
`Core Memory` (11648), `Team Runs` (12384), `Search / Layer 3` (12840),
`Dashboard Queries` (13086), `KG corpus queries` (14039).

Le découpage thématique que le ticket nomme est donc **déjà tracé dans le
fichier**. C'est ce qui rend l'extraction mécanique quand elle est décidée —
et c'est aussi pourquoi elle n'est pas urgente (cf. D5).

### E4 — `crate::db::tests` est un contrat inter-modules, pas un module privé

`skills/executor.rs:8581` fait :

```rust
use crate::db::tests::{GROOM_CALLBACK_PLAN_GROOMED, completed_groom_pair, db};
```

et cite `crate::db::tests::GROOM_ISSUE_URL` à la ligne 8625. Le module est
déclaré `pub(crate) mod tests` et expose sept items `pub(crate)` :
`db()` (14785), `rust_sources_under()` (14907), `GROOM_ISSUE_URL` /
`GROOM_CALLBACK_PLAN_GROOMED` (26967–26968), `groom_parent` (26975),
`groom_callback` (27004), `completed_groom_pair` (27034).

**Le chemin `crate::db::tests::` doit survivre octet pour octet.** Un
déplacement qui le casserait ne compilerait pas — c'est le bon type d'échec,
mais il contraint la forme : les helpers partagés doivent rester à la racine du
module `tests`, pas descendre dans un sous-module thématique.

### E5 — Le `#[path]` de `harnais_porte` n'existe que parce que `mod tests` est inline

`db.rs:27218` porte `#[path = "harnais_porte.rs"] mod harnais_porte;`, et son
commentaire de doc dit pourquoi : `db.rs` était « 4,6 Ko sous le plafond », donc
mika#2310 a sorti le fichier tout en préservant le chemin de module
`db::tests::harnais_porte` que son critère de sortie interroge.

Si `mod tests` devient un module de fichier sous `db/tests/`, la résolution
naturelle trouve `db/tests/harnais_porte.rs` **sans attribut**. L'attribut
devient supprimable, et le critère `cargo test -p mika-agent harnais_porte`
(filtre par sous-chaîne sur le chemin complet) est inchangé.

### E6 — Les tests appellent ~31 migrations privées en direct

`grep -c '\.migrate_v[0-9]' db.rs` → **106** sites. Les 55 premiers sont le
dispatcher `fn migrate` (1122–1371). Les autres sont **dans le module de test** :
le harnais de convergence avant (23763–23803) appelle toute l'échelle
`migrate_v24_to_v25` → `migrate_v53_to_v54`, et d'autres tests appellent
directement v28→v29 (24379, 24419), v29→v30 (24495, 24537), v46→v47 (27298,
27363, 27474, 27581, 27583), v49→v50 (15786–15813), v50→v51 (14958, 15305),
v51→v52 (14994, 15079), v52→v53 (15021, 15054).

Toutes ces méthodes sont **privées** (`fn migrate_vN_to_vM`, sans `pub`). En
Rust, la visibilité est modulaire : un enfant voit les items privés de son
parent, **mais deux frères ne se voient pas**. Donc si les migrations partent
dans `db::migrations` et les tests dans `db::tests`, les deux deviennent frères
et les ~31 méthodes doivent être élargies. C'est ce que D3 évite.

`fn migrate` lui-même n'a que **deux appelants**, `db.rs:1035` et `db.rs:1045`
(dans `open` et `open_in_memory`), qui restent dans `db.rs`.

### E7 — Le danger central : deux gardes isolent « la production » en tronquant sur `#[cfg(test)]`

Deux gardes structurelles énumèrent tous les `.rs` sous `src/` et retirent la
queue de test de chaque fichier de la même façon :

- `db.rs:14855` — mika#2335 F2a :
  `let production = match src.find("#[cfg(test)]") { Some(i) => &src[..i], None => &src[..] };`
  puis cherche `update_manual_task_status` avec `"in_progress"` dans une fenêtre
  de 200 caractères.
- `agent_loop/mod.rs:14279` — `fn production_half(src: &str) -> &str`, même
  `match`, au service du compteur de lecteurs de `HistoryScope` (mika#2305).

Le commentaire de `production_half` énonce la prémisse mot pour mot : « Test
modules are full of legitimate mentions of both variants; a scan that refused
"any mention" would redden immediately on healthy code ».

**Un module de test extrait dans son propre fichier ne porte aucun littéral
`#[cfg(test)]`** — l'attribut reste sur la déclaration `#[cfg(test)] mod tests;`
dans le fichier parent. `src.find("#[cfg(test)]")` rend `None`, et la garde
scanne **le fichier de test entier comme du code de production**.

**La prémisse est déjà fausse aujourd'hui, en deux endroits, et bénigne par
chance :**

| fichier | occurrences de `cfg(test)` | statut vis-à-vis des gardes |
|---|---|---|
| `db/tests/harnais_porte.rs` | **0** | scanné intégralement comme production (ouvert par mika#2310) |
| `perimeter/tests.rs` (29 117 o) | 1, **dans un commentaire** (l. 179) | idem ; déclaré `#[cfg(test)] mod tests;` à `perimeter/mod.rs:49` |

Aucun des deux ne contient les aiguilles, donc rien ne rougit. Faire passer
**463 Ko et 431 tests** par ce trou, c'est remplacer la chance par un tirage.

Et la garde mika#2335 se scannerait **elle-même** : son propre littéral
`let needle = "update_manual_task_status";` (l. 14877) n'est pas une ligne de
commentaire, donc il survit à la neutralisation, et son test
`window.contains("\"in_progress\"")` est à environ 290 caractères en aval —
sous la fenêtre de 200, mais de ~90 caractères seulement. Aujourd'hui c'est
inatteignable parce que la garde tronque `db.rs` avant sa propre ligne. Extraite,
la marge devient une propriété du reflow `cargo fmt` d'une fenêtre de 200
caractères. **Ce n'est pas une propriété sur laquelle parier une porte CI**, et
c'est pourquoi V3 exige une vérification empirique et non un raisonnement.

### E8 — L'idiome de réparation existe déjà dans le dépôt, et il est testé

`perimeter/rules.rs:177` porte `"/tests/"` dans `MECHANICAL_CONTAINS`, avec son
commentaire : « Any test module directory anywhere under a crate […] Test modules
cannot change production behavior at runtime. »

Et `perimeter/tests.rs:170–182` épingle les deux cas, avec la raison :

| chemin | classification | raison écrite dans le test |
|---|---|---|
| `crates/mika-common/src/foo/tests/mod.rs` | `Mechanical` | répertoire `/tests/` |
| `crates/mika-agent/src/some_module/tests.rs` | `DecisionCore` | « `tests.rs` alone is NOT a `/tests/` substring […] might be accidentally cfg-flipped. **Fail-closed here.** » |

Une classification des chemins de test existe donc, elle est délibérée et elle
est couverte. C'est la pièce sur laquelle D1 et D2 s'appuient.

### E9 — Ce qui est verdict-neutre, et ce qui ne l'est pas

- `scripts/check-byte-slices.sh` scanne **tout** `crates/` (`SCAN_ROOT="${1:-$REPO_ROOT/crates}"`),
  pas le diff. Un déplacement de code y est donc **verdict-neutre** : ce qui
  passe aujourd'hui passe après. Le `&src[..i]` de la garde mika#2335 ne devient
  pas une violation en changeant de fichier.
- `scripts/check-secrets.sh` est **diff-scopé** (`--changed origin/main`, CI
  `ci.yml:278`). Les fichiers créés par le découpage sont des fichiers *ajoutés*
  et **seront** scannés — chacun doit être sous 1 Mo, ce qu'ils sont par
  construction. `db.rs`, modifié, sera scanné aussi : d'où la contrainte d'ordre
  de F1.
- Les citations `db.rs:<ligne>` dans `docs/solutions/**` (au moins dix :
  `db.rs:5752`, `db.rs:5839`, `db.rs:3123`, `db.rs:3230`, `db.rs:3809`,
  `db.rs:4495`, `db.rs::8117`, `db.rs:3307-3309`, `db.rs:4368`, `db.rs:2338`)
  sont **déjà fausses aujourd'hui** — les numéros ont dérivé à chaque commit sur
  un fichier de 27 594 lignes. Cf. D6.

## Décisions

### D1 — Réparer la classification des gardes **avant** de déplacer quoi que ce soit

L'ordre est la décision, pas un détail de séquençage. Déplacer d'abord, c'est
soit un faux positif CI sur une garde qui se scanne elle-même (E7), soit —
strictement pire — un **angle mort silencieux** : 463 Ko de code de test admis
comme production dans deux gardes dont tout l'intérêt est de refuser un site de
production. Une garde qui ne rougit pas pour la mauvaise raison ne se distingue
pas d'une garde qui va bien.

La réparation porte sur la **classification du fichier**, pas sur la troncature :
un fichier dont le chemin est un chemin de test **est** du code de test, qu'il
porte ou non un littéral `#[cfg(test)]`. La prémisse « le code de test vit
derrière un `#[cfg(test)]` inline dans le même fichier » était vraie quand les
deux gardes ont été écrites et a cessé de l'être avec mika#2310 ; c'est elle
qu'on corrige, une fois, aux deux sites.

Le prédicat est celui que le dépôt teste déjà (E8) : un segment `/tests/`
n'importe où dans le chemin, **ou** un nom de fichier `tests.rs`. Les deux
branches sont nécessaires — `db/tests/tasks.rs` relève de la première,
`perimeter/tests.rs` (qui existe et est déjà dans le trou) de la seconde.

**Refusé : élargir les aiguilles ou poser une allowlist par fichier.** C'est
exactement le mouvement que le commentaire de `production_half` anticipe (« the
natural repair would be to widen it until it caught nothing ») et que
`mika2335_no_production_dispatch…` refuse en toutes lettres pour un quatrième
site (« Pas d'entrée d'allowlist, pas de `#[ignore]` »).

**Contrôle négatif obligatoire.** Une réparation de classification qui
exempterait trop rendrait les deux gardes vacuous sans rien casser. V3 exige
donc, pour chaque garde, la preuve qu'un site de production fautif est **toujours**
attrapé après la réparation — pas seulement que la suite reste verte.

### D2 — `db/tests/mod.rs` (répertoire), jamais `db/tests.rs` (fichier unique)

Trois raisons, chacune mesurée, aucune esthétique.

1. **Classification du périmètre (E8).** `some_module/tests.rs` → `DecisionCore`
   par choix fail-closed explicite ; `foo/tests/mod.rs` → `Mechanical`. Or une PR
   DECISION-CORE n'est pas dé-draftée automatiquement (mika#2286) : le fichier
   unique ferait de **chaque PR qui touche un test de `db`** un draft en attente
   d'un geste humain de vérification. Le répertoire garde ces PR mécaniques.
2. **`harnais_porte` y vit déjà** (E5). Le répertoire en fait un frère naturel et
   retire l'attribut `#[path]`.
3. **Croissance.** Un `tests.rs` unique naîtrait à 463 Ko : la moitié du plafond,
   dans la région qui croît le plus vite (E1). Un répertoire répartit la
   croissance par thème et aucun fichier n'approche le plafond.

Les sept items `pub(crate)` de E4 restent à la racine (`db/tests/mod.rs`) pour
que `crate::db::tests::*` soit inchangé.

### D3 — Un module de test voyage avec le code qu'il teste

Règle, pas allocation : le `#[cfg(test)] mod tests` d'un sous-module reste
**enfant** du module qui détient les items privés qu'il exerce. Un enfant voit
les privés de son parent ; deux frères ne se voient pas.

Appliqué aux migrations, cela **annule les ~31 élargissements de visibilité** de
E6 : les tests de migration descendent dans `db/migrations.rs` sous leur propre
`#[cfg(test)] mod tests`, enfant de `db::migrations`, et continuent d'appeler
`migrate_v46_to_v47` privée sans une seule signature touchée.

Il reste **exactement un** élargissement dans tout le plan : `fn migrate` devient
`pub(super)`, parce que ses deux appelants (`open`, `open_in_memory`, E6) restent
dans `db.rs`, c'est-à-dire dans le parent. Un, nommé, justifié — au lieu de
trente-deux noyés dans un diff « mécanique ».

Corollaire assumé : le chemin de module des tests de migration devient
`db::migrations::tests::…` au lieu de `db::tests::…`. Rien n'en dépend (E4 ne
liste que les helpers de grooming) et les filtres `cargo test` sont des
sous-chaînes.

### D4 — Deux volets, et le premier suffit à fermer le ticket

**Volet A — tests + réparation des gardes + retrait de l'allowlist.** Amène
`db.rs` à 634 538 o, soit **414 Ko de marge**, et **zéro élargissement de
visibilité** : `db::tests` reste enfant de `db`, donc tous les privés de `db`
restent visibles exactement comme aujourd'hui. C'est A, et A seul, qui satisfait
l'objectif écrit du ticket (« chaque fichier < 1 Mo, et retirer db.rs de
LARGE_FILE_ALLOWLIST »).

**Volet B — échelle de migrations.** Sort ~210 Ko append-only vers
`db/migrations.rs`, amenant `db.rs` à ~425 Ko. Justifié par E1 (région monotone),
mécanique sous D3 (un `pub(super)`), et **ordonné après A** : A est ce qui retire
l'allowlist, et la réparation des gardes de A est une précondition de B.

**Pourquoi B est dans le périmètre et non un suivi.** Sans lui, l'objectif « borne
la croissance » n'est tenu que sur une moitié du fichier : A sort les tests, et
la moitié non-test — dont ~210 Ko sont une échelle qui ne rétrécit jamais —
continue de croître sans borne sous le plafond. A repousse le mur, B retire la
brique qui l'a construit.

### D5 — `tasks`, `grooming/verdict`, `retention/prune` : nommés, non découpés

Le ticket nomme quatre unités. Trois sont explicitement hors de ce plan, et la
raison est une classe de risque, pas un manque de temps.

`Task CRUD` fait 186 752 o et vit au milieu de 313 méthodes `pub` et 537 `fn`
privées dont les helpers traversent les sections. Les sortir demande, pour chaque
helper privé franchissant une frontière, la même question que D3 tranche pour les
migrations — mais sans la réponse simple, parce que les appelants sont dispersés
au lieu d'être un dispatcher unique. C'est un travail de jugement par méthode,
pas un déplacement de blocs.

Et cela n'achète rien contre le plafond : après A+B, `db.rs` est à ~425 Ko avec
600 Ko de marge. Découper ces trois unités maintenant, c'est payer la classe de
risque la plus chère du ticket pour une marge dont personne n'a besoin. Suivi à
ouvrir (§ Hors périmètre), avec E3 comme carte : les marqueurs de section sont
déjà là, le jour où une mesure le demandera.

### D6 — Les citations `db.rs:<ligne>` des solutions ne sont pas mises à jour

Elles sont déjà fausses (E9). Un fichier de 27 594 lignes rend la citation par
numéro de ligne un instrument cassé à la première PR, et il y a eu cinq commits
en trois jours. Les réparer dans cette PR, ce serait figer des numéros qui
dériveront au commit suivant.

Ce que le découpage achète à leur place est la forme citable : `db/migrations.rs::migrate_v45_to_v46`
au lieu de `db.rs:4495`. La conversion des citations existantes est un suivi de
documentation, pas une condition de ce refactor — et un suivi qui a maintenant
une cible stable vers laquelle converger.

## Volets d'implémentation

### Volet A — réparation des gardes, puis extraction du module de test

**A1. Réparer la classification, aux deux sites.**
Introduire un prédicat unique de chemin-de-test — segment `/tests/` **ou** nom de
fichier `tests.rs` — et le consulter dans les deux gardes, en complément de la
troncature existante (un fichier de production garde sa troncature ; un fichier
de test est écarté entièrement). Les deux sites : `db.rs:14855` (mika#2335 F2a)
et `agent_loop/mod.rs:14279` (`production_half`, mika#2305).

Un seul lecteur du prédicat, pas une copie par garde — c'est la classe que
`grooming_marker` a dû graver une fois (mika#2158 : une regex copiée dont le
commentaire disait « Mirrors … » et qui a ensuite raté deux élargissements).
Chaque garde documente dans sa doc que sa prémisse d'origine a cessé d'être vraie
avec mika#2310, et pourquoi la réparation porte sur la classification et non sur
les aiguilles (D1).

**A2. Créer `db/tests/mod.rs` et y déplacer le module.**
Remplacer, dans `db.rs`, le bloc `#[cfg(test)] pub(crate) mod tests { … }`
(14781–27594) par `#[cfg(test)] pub(crate) mod tests;`. Le corps part dans
`db/tests/mod.rs`. Le `use super::*;` de tête est inchangé : `super` désigne
toujours `db`.

Y restent obligatoirement (E4) : `db()`, `rust_sources_under()`,
`GROOM_ISSUE_URL`, `GROOM_CALLBACK_PLAN_GROOMED`, `groom_parent`,
`groom_callback`, `completed_groom_pair`.

**A3. Retirer le `#[path]` de `harnais_porte`.**
`db/tests/harnais_porte.rs` est désormais résolu naturellement. Remplacer
`#[path = "harnais_porte.rs"] mod harnais_porte;` par `mod harnais_porte;` et
mettre à jour le commentaire de doc du fichier, qui explique aujourd'hui un
`#[path]` qui n'existe plus et cite une taille (1 043 979 o) qui n'est plus la
vérité.

**A4. Répartir par thème sous `db/tests/`.**
En suivant les marqueurs de section déjà présents dans la région de test
(`Skill Override Tests` 20725, `tasks.type` 21069, `Internal message` 22874,
`mika#2295 conversation-window` 22948, `KG Schema Migration` 23236,
`Transaction RAII` 23956, `Secret scrubbing` 24171, `Agent Reset` 25417,
`task_messages` 25763, `Force-promote` 26316, `cancel_orphan_recurring` 26699,
`has_completed_groom_for_issue` 26959, `v46→v47` 27221, et les blocs
ticket-scopés 14796–17413).

Cible : aucun fichier au-dessus de ~150 Ko. Les tests d'une méthode privée de
`db` restent sous `db/tests/` (enfant de `db`, la visibilité est conservée) ;
ceux qui appellent des migrations privées partent en B2.

**A5. Retirer `db.rs` de `LARGE_FILE_ALLOWLIST`.**
`scripts/check-secrets.sh:44`. L'entrée est retirée, pas commentée — le tableau
retourne à son état de sortie (« Empty at rollout »), et son contrat interne
exige une issue de suivi par entrée, donc une entrée sans dette est une entrée
à supprimer.

### Volet B — échelle de migrations

**B1. Extraire vers `db/migrations.rs`.**
`fn migrate` (1111) et les `fn migrate_vN_to_vM` (1373–5765) partent dans
`db/migrations.rs` sous un `impl Database`, selon l'idiome de `db/operational.rs`
(E2). `fn migrate` devient `pub(super)` — le seul élargissement du plan (D3).
`open` / `open_in_memory` restent dans `db.rs` et leurs deux appels (1035, 1045)
sont inchangés.

`CURRENT_SCHEMA_VERSION` reste dans `db.rs` : `kg_fixtures::PINNED_SCHEMA_VERSION`
y est lié par un const-assert de compilation
(`docs/solutions/best-practices/schema-bump-fixture-pin-co-edit-2026-05-01.md`),
et la constante est lue bien au-delà des migrations.

**B2. Les tests de migration descendent avec elles.**
Sous `#[cfg(test)] mod tests` dans `db/migrations.rs` (D3) : le harnais de
convergence avant (23763–23803), le bloc `KG Schema Migration Tests` (23236–23920,
24 435 o), et les tests appelant directement v28→v29, v29→v30, v46→v47, v49→v50,
v50→v51, v51→v52, v52→v53 (E6). Aucune signature de migration n'est touchée.

Les helpers dont ils dépendent et qui sont partagés avec d'autres tests —
`db()` en tête — sont importés depuis `super::super::tests` ; s'ils ne sont
utilisés que par les tests de migration, ils descendent avec eux.

## Verification contract

- **V1 — compilation et suite complète.** `cargo build` et `cargo test -p mika-agent`
  verts. Le compte de tests avant/après est **égal**, pas « comparable » : un
  test perdu dans un déplacement de 431 tests est silencieux, et l'égalité du
  compte est ce qui le rend audible.
- **V2 — le contrat inter-modules tient.** `skills/executor.rs` compile sans
  modification : ses imports `crate::db::tests::{…}` sont la preuve que le chemin
  de module a survécu (E4). Toute retouche de ce fichier est le signe que D2 n'a
  pas été respecté.
- **V3 — les deux gardes réparées, avec leur contrôle négatif.** Pour
  `mika2335_no_production_dispatch_transitions_a_parent_without_stamping` et
  `mika2305_the_scope_has_a_single_decisional_reader` : (a) vertes après le
  déplacement ; (b) **vertes pour la bonne raison** — un site de production
  fautif injecté temporairement les fait rougir chacune. Sans (b), une
  classification trop large rend les deux gardes vacuous et la suite reste verte
  (D1). C'est la vérification que E7 exige d'être empirique et non raisonnée.
- **V4 — le trou de mika#2310 est fermé.** Les deux fichiers de E7
  (`db/tests/harnais_porte.rs`, `perimeter/tests.rs`) sont écartés par la
  classification réparée, et non plus scannés comme production. Assertion sur le
  prédicat lui-même.
- **V5 — la porte rend le verdict attendu.**
  `bash scripts/check-secrets.sh --changed origin/main` sort 0 avec l'entrée
  d'allowlist **retirée**. C'est la vérification que A5 est réelle et pas
  décorative.
- **V6 — aucun fichier ne repasse le plafond.** Tout `.rs` créé ou modifié est
  sous 1 048 576 o, et `db.rs` sous 700 Ko après A, sous 500 Ko après B.
- **V7 — `harnais_porte` reste interrogeable.**
  `cargo test -p mika-agent harnais_porte` sélectionne toujours les cas 4 et 8 —
  le critère de sortie que mika#2310 a inscrit et que mika#2288 lit (E5).
- **V8 — verdict-neutralité des lints pleine-arborescence.**
  `bash scripts/check-byte-slices.sh` rend le même verdict qu'avant (E9), et
  `cargo clippy` / `cargo fmt --check` sont propres.
- **V9 — aucune dérive de comportement.** Le diff ne contient aucune chaîne SQL
  modifiée, aucune signature publique modifiée, et exactement un changement de
  visibilité (`fn migrate` → `pub(super)`, volet B). Relecture dirigée : tout
  autre changement de visibilité dans le diff est une erreur, pas une nécessité.

## Fire-Disposition

- **F1 — ordre de commit contraint par le hook.** `check-secrets.sh` tourne dans
  le pre-commit sur les fichiers *staged* (E9). Retirer l'entrée d'allowlist
  avant que `db.rs` ne soit passé sous le plafond fait échouer le commit
  intermédiaire. **A5 est le dernier pas du volet A**, après A2/A4. Symétriquement,
  découper sans retirer l'entrée laisse la CI verte et le ticket ouvert : les deux
  doivent être dans la même PR.
- **F2 — un test perdu est la seule perte silencieuse possible.** Tout le reste
  échoue à la compilation. D'où l'égalité stricte du compte en V1.
- **F3 — si une garde rougit après A1, halte.** Ne pas élargir l'aiguille, ne pas
  poser d'exemption de fichier : c'est le mouvement que D1 refuse et que la doc
  de la garde refuse en toutes lettres. Établir d'abord si le site signalé est un
  vrai site de production (→ il migre) ou un site de test mal classé (→ le
  prédicat de A1 est à corriger).
- **F4 — si le compte de tests baisse, ne pas rééquilibrer.** Retrouver le test
  manquant par différence de noms, pas par ajustement du compte attendu.
- **F5 — B est abandonnable sans rien invalider.** A est autosuffisant : plafond
  franchi, allowlist vide, gardes réparées. Si B révèle une complication
  inattendue dans l'échelle de migrations, il se retire de la PR sans toucher A.
  L'inverse n'est pas vrai — B dépend de la réparation des gardes de A1.

## Definition of Done

- La classification des gardes est réparée aux deux sites, par un lecteur unique,
  avec contrôle négatif.
- Le module de test de `db.rs` vit sous `db/tests/`, réparti par thème, avec
  `crate::db::tests::*` inchangé et `#[path]` retiré.
- L'échelle de migrations vit dans `db/migrations.rs`, ses tests avec elle, avec
  un seul élargissement de visibilité.
- `db.rs` est sous 500 Ko ; aucun fichier `.rs` du crate n'atteint 1 Mo.
- `LARGE_FILE_ALLOWLIST` est vide et la CI `secret-scan` est verte sans elle.
- V1–V9 passent ; le compte de tests est strictement égal.
- `cargo build`, `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`
  propres.
- Le commentaire de doc de `db/tests/harnais_porte.rs` ne décrit plus un `#[path]`
  disparu ni une taille périmée.
- Les trois suivis du § Hors périmètre sont ouverts.

## Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` ; les
critères ci-dessous sont dérivés de son objet (« chaque fichier < 1 Mo, et
retirer db.rs de LARGE_FILE_ALLOWLIST », « borne la croissance ») et du
Verification contract.

- **AC1** — `crates/mika-agent/src/db.rs` pèse moins de 1 048 576 octets. Mesure :
  `wc -c crates/mika-agent/src/db.rs`.
- **AC2** — aucun fichier `.rs` sous `crates/` n'atteint 1 048 576 octets, fichiers
  créés par le découpage compris.
- **AC3** — `LARGE_FILE_ALLOWLIST` dans `scripts/check-secrets.sh` ne contient plus
  `crates/mika-agent/src/db.rs`, et
  `bash scripts/check-secrets.sh --changed origin/main` sort 0.
- **AC4** — l'échelle de migrations (`fn migrate` + `migrate_vN_to_vM`) et le module
  de test ne sont plus dans `db.rs` : les deux régions append-only identifiées en
  E1 vivent dans leurs propres fichiers.
- **AC5** — `cargo test -p mika-agent` est vert et le nombre de tests exécutés est
  **strictement égal** à celui mesuré sur la base avant le découpage.
- **AC6** — `skills/executor.rs` n'est pas modifié : ses imports
  `crate::db::tests::{GROOM_CALLBACK_PLAN_GROOMED, completed_groom_pair, db}` et
  sa citation de `GROOM_ISSUE_URL` résolvent toujours.
- **AC7** — `cargo test -p mika-agent harnais_porte` sélectionne toujours les cas
  du harnais de porte mika#2310.
- **AC8** — les deux gardes structurelles qui tronquent sur `#[cfg(test)]`
  (`db.rs` mika#2335 F2a, `agent_loop/mod.rs` mika#2305) écartent un fichier de
  test par son chemin, et chacune est accompagnée d'un contrôle négatif prouvant
  qu'un site de production fautif est toujours attrapé.
- **AC9** — `db/tests/harnais_porte.rs` et `perimeter/tests.rs` ne sont plus
  scannés comme code de production par ces deux gardes.
- **AC10** — le diff ne contient aucune modification de chaîne SQL, aucune
  modification de signature publique, et **exactement un** changement de
  visibilité (`fn migrate` → `pub(super)`).
- **AC11** — `cargo clippy` et `cargo fmt --check` sont propres.

## Surfaces opérateur et sonde post-déploiement

**Aucune surface opérateur, et c'est un fait à écrire plutôt qu'une section à
remplir.** Ce travail ne change aucun comportement à l'exécution : pas
d'événement de journal, pas de ligne `audit_events`, pas de variable
d'environnement, pas de requête SQL. Inventer une télémétrie pour un refactor
serait du remplissage, et une sonde qui ne mesure rien se lit comme une sonde qui
va bien.

La sonde est **la porte CI elle-même**, et elle n'a plus de béquille :

- **Sonde permanente.** Avec `LARGE_FILE_ALLOWLIST` vide, `secret-scan` refuse de
  lui-même tout fichier qui repasse 1 Mo. C'est le mécanisme que le ticket
  demande (« borne la croissance ») : il n'est pas observé, il est appliqué.
- **Sonde de régression de l'exemption.** Toute réapparition de
  `crates/mika-agent/src/db.rs` dans `LARGE_FILE_ALLOWLIST` est le retour du
  défaut, pas sa réparation. Le contrat du tableau (une issue de suivi par
  entrée, cf. son commentaire d'en-tête) rend cette réapparition auditable dans
  le diff de la PR qui la poserait.
- **Halte explicite.** Si une PR ultérieure se trouve bloquée par le plafond sur
  un fichier `db/*`, le geste est de sortir la région thématique concernée
  (E3 donne la carte), **jamais** de rouvrir l'allowlist. Une entrée d'allowlist
  a déjà laissé ce fichier prendre 64 Ko en silence (§ Contexte) ; c'est la
  mesure de ce que coûte le geste facile.

## Hors périmètre (suivi à ouvrir)

- **Découpage de `Task CRUD` / grooming-verdict / retention-prune** (D5). Les
  trois unités que le ticket nomme et que ce plan ne découpe pas : 313 méthodes
  `pub`, 537 `fn` privées, helpers traversant les sections — une classe de risque
  par jugement et par méthode, pour une marge dont on n'a pas besoin après A+B.
  E3 en est la carte le jour où une mesure le demandera.
- **Conversion des citations `db.rs:<ligne>` de `docs/solutions/**`** (D6). Au
  moins dix citations déjà fausses aujourd'hui, à convertir en
  `<fichier>::<nom_de_fonction>`. Le découpage leur donne une cible stable ;
  c'est un suivi de documentation, pas une condition de ce refactor.
- **Audit des autres scans de sources structurels.** Une dizaine de fichiers
  énumèrent `src/**/*.rs` (`image_disposition.rs`, `auto_pull_stop.rs`,
  `grooming_marker.rs`, `ready_label.rs`, `planning/policy.rs`,
  `server/deadline_verdict.rs`, `auto_pull.rs`, `server/mod.rs`,
  `milestone_manager/no_dispatch_test.rs`, `agent_loop/review_anchor.rs`). Deux
  seulement tronquent sur `#[cfg(test)]` (E7) et sont réparés ici ; les autres
  emploient d'autres mécanismes d'exclusion (`EXEMPT_FILES` par nom de fichier
  pour `no_dispatch_test.rs`, neutralisation de commentaires ailleurs), qui n'ont
  pas été audités face à la généralisation des modules de test en fichiers
  propres. Ce n'est pas requis par ce ticket — aucun ne rougit — mais c'est la
  même prémisse et elle mérite son propre inventaire.
- **Dérive documentaire `CURRENT_SCHEMA_VERSION`.** `CLAUDE.md` annonce
  « Schema v53 » alors que `db.rs:30` porte `54`. La table de
  `schema-bump-fixture-pin-co-edit-2026-05-01.md` marque déjà ce couple
  « Unprotected ». Sans rapport avec le découpage, relevé en chemin.
