# chore(3131) — rusqlite 0.32 → 0.41 : la montée est une cascade à trois étages, et elle n'est pas dispatchable

**Ticket :** senara-solutions/mika#3131
**Parking d'origine :** senara-solutions/mika#2983 (commentaire 36)
**Exception visée :** RUSTSEC-2026-0459 (`deny.toml:21`, `.cargo/audit.toml`)

---

## Ce que le ticket demande, et ce que la mesure répond

Le ticket demande une chose qui se lit comme une seule : monter `Cargo.toml:30`
de `rusqlite = "0.32"` à `"0.41"`, puis retirer les deux exceptions
RUSTSEC-2026-0459. Le motif d'exception actuel dit la même chose :
*« the fix (>= 0.39.0) needs a major rusqlite bump to 0.41 »*.

**Ce n'est pas une montée, c'est une cascade à trois étages**, et le troisième
n'était nommé nulle part. Les trois ont été mesurés dans la session de grooming,
avec les commandes et les sorties reproduites au § suivant :

1. **rusqlite 0.32 → 0.41.1** — l'étage demandé. Adaptation d'API sur
   **484 sites répartis sur 50 fichiers**, neuf versions majeures d'écart.
2. **sqlx 0.8 → 0.9.1** — *forcé, non optionnel*. `rusqlite 0.41` et
   `sqlx 0.8.6` déclarent tous deux `links = "sqlite3"` dans des versions de
   `libsqlite3-sys` différentes, et Cargo **refuse de résoudre** le graphe. La
   montée rusqlite seule n'est pas un changement qui compile mal : c'est un
   changement qui n'a **pas de lockfile**. sqlx est une montée majeure qui touche
   `mika-gateway`.
3. **toolchain 1.93 → 1.94 + `rust-version` 1.91 → 1.94** — *forcé par l'étage 2*.
   `sqlx 0.9.1` porte `requires Rust 1.94.0`, là où `rust-toolchain.toml:2` fixe
   `1.93` et `Cargo.toml:14` déclare `rust-version = "1.91"`.

Et par-dessus ces trois étages, un quatrième fait, qui est celui du **routage** :

4. **Le bac à sable ne peut compiler aucun de ces trois étages.**
   `~/.cargo/registry/src/` est monté en **lecture seule** — `cargo` atteint le
   réseau, résout l'index, puis échoue au dépaquetage de la première caisse
   nouvelle. Aucune caisse absente du disque ne peut être dépaquetée, donc ni
   `cargo build`, ni `cargo test`, ni `cargo clippy` ne tournent sur le nouveau
   graphe. Un pilote dispatché écrirait une adaptation de 484 sites **sans jamais
   la compiler une fois**, contre une version de rusqlite dont le source n'est pas
   sur le disque et dont le changelog n'est pas atteignable (`WebFetch` refusé par
   la policy).

Ce dernier point est la décision du plan, et il a un précédent exact :
`docs/solutions/dependency-maintenance/2026-10-21-un-bump-de-lock-rustsec-nest-pas-dispatchable.md`,
écrit après qu'un vol complet de pilote a été dépensé sur mika#2861 pour produire
une halte que le grooming connaissait déjà. Sa règle :

> Un refus rencontré au grooming est un fait de **routage**, pas une note de bas
> de page. Il ne se range pas en branche de halte à l'intérieur du plan : il
> retire le ticket de la file de dispatch.

**Ce plan applique cette règle.** Le vol courant livre la part que le bac à sable
autorise — la moitié documentaire et les deux motifs d'exception, qui sont faux
par omission aujourd'hui — et renvoie la cascade à un geste opérateur, hors bac à
sable, dont la procédure exacte est écrite ci-dessous.

---

## Les mesures, et comment chacune se rejoue

Toutes datées du 2026-11-24, dans le worktree de dispatch de ce ticket.

### M1 — le préflight canonique est refusé

```
$ cargo update --help
no matching policy rule -- denied by default (production posture; widen rules to allow new tool footprints)
```

C'est la commande exacte que le document de 2026-10-21 § « Quoi faire » prescrit
en tête de tout plan de montée. Le refus porte sur le préfixe de sous-commande :
`cargo update -p rusqlite --dry-run` est refusé de la même façon.

### M2 — aucune résolution hors ligne

`~/.cargo/registry/index/index.crates.io-1949cf8c6b5b557f/` ne contient que
`config.json`. Pas de cache d'index épars.

### M3 — les versions corrigées ne sont pas sur le disque

`ls ~/.cargo/registry/cache/*/` donne, pour cette famille :
`rusqlite-0.31.0.crate`, `rusqlite-0.32.1.crate`, `libsqlite3-sys-0.28.0.crate`,
`libsqlite3-sys-0.30.1.crate`. **Ni rusqlite 0.41, ni aucune libsqlite3-sys ≥ 0.39.**

### M4 — rusqlite 0.41 seul ne résout pas : conflit `links = "sqlite3"`

Avec le seul changement `Cargo.toml:30` → `rusqlite = { version = "0.41", … }` :

```
$ cargo metadata --format-version 1
    Updating crates.io index
error: failed to select a version for `sqlx`.
    ... required by package `mika-gateway v1.0.0`
versions that meet the requirements `^0.8` are: 0.8.6, 0.8.5, 0.8.3, 0.8.2, 0.8.1, 0.8.0

package `sqlx` links to the native library `sqlite3`, but it conflicts with a previous package which links to `sqlite3` as well:
package `rusqlite v0.41.0`
    ... which satisfies dependency `rusqlite = "^0.41"` of package `mika-os v1.0.0`
Only one package in the dependency graph may specify the same links value.
…
failed to select a version for `sqlx` which could resolve this conflict
```

Deux lectures à ne pas rater :

- **Le conflit est invisible aujourd'hui** parce que `rusqlite 0.32.1` et
  `sqlx-sqlite 0.8.6` partagent `libsqlite3-sys 0.30.1` : une seule copie, un seul
  `links`. Monter rusqlite casse ce partage.
- **La feature `sqlite` de sqlx n'est pas activée** (`Cargo.toml:128` ne demande
  que `postgres`), et cela ne protège de rien : la vérification `links` se fait
  dans le **résolveur**, sur le graphe résolu, qui inclut les dépendances
  optionnelles non activées. `default-features = false` sur sqlx ne lève donc pas
  le conflit. Il n'y a pas de contournement par les features.

Le `sqlx does not have that feature tls-rustls-aws-lc-rs` qui suit dans la sortie
est un artefact des rétrogradations 0.8.0–0.8.5 que le résolveur a essayées en
cherchant une issue ; ce n'est pas un fait sur 0.8.6.

### M5 — rusqlite 0.41 + sqlx 0.9 résolvent, et donnent le remède exact

Avec `rusqlite = "0.41"` **et** `sqlx = "0.9"`, la résolution **passe**. Le lock
régénéré porte :

```
name = "rusqlite"        version = "0.41.1"
name = "libsqlite3-sys"  version = "0.39.1"
```

`libsqlite3-sys 0.39.1` est **exactement** le remède que RUSTSEC-2026-0459
réclame (fix `>= 0.39.0`). La cible du ticket est donc la bonne ; c'est son coût
qui était sous-estimé.

La sortie nomme aussi l'étage 3 :

```
    Updating sqlx-core v0.8.6 -> v0.9.1 (requires Rust 1.94.0)
    Updating sqlx-sqlite v0.8.6 -> v0.9.1 (requires Rust 1.94.0)
    Updating whoami v1.6.1 -> v2.1.3
```

### M6 — le dépaquetage est impossible : registre `src/` en lecture seule

La même commande, juste après la résolution :

```
error: failed to download `etcetera v0.11.0`
Caused by: failed to create directory `/home/samidarko/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/etcetera-0.11.0`
Caused by: Read-only file system (os error 30)
```

Et pendant la résolution :
`warning: failed to write cache, path: …/registry/index/…/.cache/an/yh/anyhow, error: Read-only file system (os error 30)`.

**C'est le fait de routage.** Il est plus fort que M1 : même si la policy
autorisait `cargo update`, le bac à sable ne peut pas matérialiser les sources des
caisses nouvelles, donc il ne peut **rien compiler** de ce graphe. Un pilote
n'aurait aucun moyen de vérifier son adaptation — ni un `cargo check`, ni un test.

`Cargo.toml` et `Cargo.lock` ont été restaurés (`git checkout --`) après M4–M6 ;
le worktree est propre.

### M7 — la toolchain, aux deux endroits qui la portent

| site | valeur |
|---|---|
| `rust-toolchain.toml:2` | `channel = "1.93"` |
| `Cargo.toml:14` | `rust-version = "1.91"` |
| exigé par `sqlx 0.9.1` | `1.94.0` |

Conséquence annexe à relire au moment du geste : `.github/workflows/ci.yml:1179`
épingle `cargo-deny@0.21.0` parce que `0.22.0` exige `rustc 1.95`, avec le
commentaire *« Monter ce pin va avec une montée de toolchain, jamais seul »*.
Monter à **1.94** ne débloque pas `0.22.0` — l'épingle reste telle quelle. Elle est
nommée ici pour qu'on ne la touche pas par réflexe.

### M8 — une seule arête à traiter, ce qui borne le travail

```
$ cargo tree -i libsqlite3-sys --workspace --all-features
libsqlite3-sys v0.30.1
└── rusqlite v0.32.1
    ├── mika-agent v1.0.0
    │   └── mika-cli v1.0.0
    └── mika-os v1.0.0
```

Même avec `--all-features`, **seul rusqlite** tire `libsqlite3-sys`. Il n'y a donc
pas de seconde arête cachée à monter : l'extinction de l'avis tient entièrement à
cet étage-là. (Le partage avec `sqlx-sqlite` noté en M4 est un fait du *résolveur*,
pas de l'arbre de features — c'est précisément ce qui le rend surprenant.)

### M9 — la surface d'adaptation, mesurée et non estimée

`grep -rn "rusqlite" crates/ --include=*.rs` : **484 lignes sur 50 fichiers**
(46 dans `mika-agent`, 3 dans `mika-os`, 1 ailleurs ; `crates/mika-agent/src/db.rs`
seul en porte 129). Surface qualifiée, par symbole :

| symbole | sites |
|---|---|
| `rusqlite::params` | 154 |
| `rusqlite::Result` | 117 |
| `rusqlite::types::ToSql` | 91 |
| `rusqlite::Connection` | 18 |
| `rusqlite::Row` | 15 |
| `rusqlite::params_from_iter` | 12 |
| `rusqlite::Error::QueryReturnedNoRows` | 12 |
| `rusqlite::Error::SqliteFailure` | 11 |
| `rusqlite::OptionalExtension` | 7 |
| `rusqlite::types::Value*` | 15 |
| `rusqlite::ffi::*` | 10 |
| `rusqlite::OpenFlags::*` | 4 |
| `rusqlite::Transaction` | 3 |
| `rusqlite::types::FromSql` | 3 |
| `rusqlite::ErrorCode::*` | 3 |

Le site le plus exposé est `crates/mika-agent/src/db.rs:25-31` :

```rust
pub fn init_sqlite_vec() {
    …
        rusqlite::ffi::sqlite3_auto_extension(Some(
            std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ()),
```

Un `transmute` à travers la frontière FFI entre `rusqlite::ffi` (donc
`libsqlite3-sys`, qui passe de 0.30.1 à 0.39.1) et `sqlite-vec`, qui compile son
propre objet C avec `cc`. C'est le seul endroit du dépôt où une montée de
`libsqlite3-sys` peut produire un défaut **silencieux à la compilation** et visible
seulement à l'exécution. Il demande une vérification positive (une requête
`vec0` qui tourne), pas un build vert.

**Ce que ce plan ne dit PAS, délibérément :** la liste des breaking changes
0.33 → 0.41. Elle n'est pas lisible ici — le changelog amont est hors d'atteinte
(`WebFetch` refusé) et le source de 0.41 n'est pas sur le disque (M3). L'écrire
serait de la fabrication, et le périmètre du geste opérateur se lit sur la surface
mesurée ci-dessus, pas sur une liste devinée.

### M10 — `sqlite-vec` ne contraint rien

`~/.cargo/registry/src/*/sqlite-vec-0.1.7-alpha.10/Cargo.toml` déclare
`[dev-dependencies.rusqlite] version = "0.31.0"`. Une **dev**-dependency : elle
n'entre pas dans notre graphe. `cargo tree -p sqlite-vec` le confirme (seul `cc`
en build-dep). Aucun blocage de ce côté.

---

## Requirements

### R1 — les deux motifs d'exception disent le coût réel

`deny.toml:21` et `.cargo/audit.toml` affirment aujourd'hui que le remède est
*« a major rusqlite bump to 0.41 »*. C'est **faux par omission** : M4 et M7
établissent que c'est une cascade de trois montées, dont une majeure de sqlx et
une de toolchain. Un lecteur qui planifie sur le motif actuel sous-estime le
travail d'un facteur large — c'est exactement ce qui a produit ce ticket avec
l'étiquette `p3-nice-to-have`.

Les deux motifs doivent nommer : le conflit `links = "sqlite3"` avec sqlx, la
montée sqlx 0.8 → 0.9 qu'il force, l'exigence `rustc 1.94` qu'elle force à son
tour, et le fait que la classe est **opérateur**. La contrainte « Keep in sync
with .cargo/audit.toml » / « Keep in sync with deny.toml » déjà présente dans les
deux motifs est maintenue.

### R2 — la classe du ticket est consignée là où le prochain groom la lira

Le document de 2026-10-21 couvre la classe « bump de nœud **transitif** dans
`Cargo.lock` ». Ce ticket est d'une classe **différente** — une montée de version
**déclarée dans `Cargo.toml`**, où l'édition du manifeste est permise et où c'est
la *vérification* qui est hors d'atteinte. Un groom qui lit le document de 2026-10-21
peut légitimement conclure « ceci ne s'applique pas, mon geste est une édition de
`Cargo.toml` » — et dispatcher. Le document doit couvrir les deux classes.

Il doit aussi consigner le fait neuf, qui n'est pas dans le document existant :
`~/.cargo/registry/src/` est en **lecture seule**, donc le réseau *fonctionne* et la
résolution *aboutit*, ce qui rend le blocage trompeur — `cargo` affiche
`Updating crates.io index` et un lock régénéré avant d'échouer. Un pilote peut
croire avoir réussi.

### R3 — le geste opérateur est exécutable en une passe

M4–M10 sont le travail analytique que le bac à sable autorise. Consignés, ils
épargnent à l'opérateur la découverte du conflit `links` en plein geste. La
procédure du § « Le geste opérateur » doit être suffisante pour mener la cascade
sans re-mesurer.

### R4 — le ticket ne se ferme pas

L'avis RUSTSEC-2026-0459 reste actif et les deux exceptions restent en place
après ce vol. La PR ne porte **pas** `Closes #3131`.

---

## Périmètre

### Dans le vol courant

| fichier | geste |
|---|---|
| `deny.toml` | réécriture du `reason` de `[[advisories.ignore]]` RUSTSEC-2026-0459 |
| `.cargo/audit.toml` | réécriture du commentaire de `"RUSTSEC-2026-0459"` |
| `docs/solutions/dependency-maintenance/2026-11-24-une-montee-declaree-dans-cargo-toml-nest-pas-plus-dispatchable.md` | nouveau document (hors compte de lignes) |

### Hors du vol courant — explicitement

- **La cascade elle-même** (les trois étages). Classe **opérateur**, hors bac à
  sable, procédure au § suivant.
- **Le retrait des deux exceptions.** Il est le dernier geste de la cascade, pas
  un geste séparable : retirer l'exception avant que `libsqlite3-sys 0.39.1` soit
  dans le lock fait rougir le job `Security` sur toute tête.
- **La montée de toolchain** (`rust-toolchain.toml`, `Cargo.toml:14`). Elle a sa
  propre surface de conséquences en CI (M7) et mérite d'être jugée pour
  elle-même, pas emportée dans un bump de dépendance.
- **Toute adaptation d'API rusqlite.** Non livrée : elle ne peut pas être
  compilée ici (M6), et la livrer non compilée serait une fabrication.

`Tracked in: senara-solutions/mika#3131` — le ticket **reste ouvert** après ce vol,
reclassé classe opérateur (retirer `dispatch:loop`, poser `pipeline-exempt`), avec
le § « Le geste opérateur » de ce plan pour procédure. Le parking
senara-solutions/mika#2983 reste la ligne de retour si la cascade est repoussée.

---

## Le geste opérateur — la cascade, hors bac à sable

Trois vols, chacun mergeable seul, dans cet ordre. Il n'y a pas d'ordre
alternatif : chaque étage est la précondition de résolution du suivant.

### Vol A — toolchain 1.93 → 1.94

`rust-toolchain.toml:2` → `channel = "1.94"`, `Cargo.toml:14` →
`rust-version = "1.94"`. Mergeable seul, et sans rapport apparent avec rusqlite —
d'où l'importance de nommer #3131 dans son corps de PR. Vérifier `cargo clippy
--workspace --all-targets` et la suite complète : une montée de toolchain réveille
des lints. **Ne pas** toucher au pin `cargo-deny@0.21.0` (M7).

### Vol B — sqlx 0.8 → 0.9

`Cargo.toml:128` → `version = "0.9"`. Montée majeure, `mika-gateway` seul
consommateur. Vérifier que `tls-rustls-aws-lc-rs`, `postgres`, `uuid`, `chrono`,
`json` existent toujours sous ce nom en 0.9 — le résolveur de M4 a signalé une
absence de cette feature sur les 0.8.x anciennes, ce qui atteste que le nom a déjà
bougé une fois dans cette famille. `whoami 1.6.1 → 2.1.3` suit (M5). Relire
`git diff Cargo.lock` et révoquer tout hunk que la montée ne réclame pas, selon la
discipline du document `quinn-proto`.

### Vol C — rusqlite 0.32 → 0.41 et extinction de l'avis

1. `Cargo.toml:30` → `version = "0.41"`, features `["bundled"]` inchangées.
2. Adapter les 484 sites (M9). Le changelog amont 0.33 → 0.41 est à lire **à ce
   moment-là** : il n'était pas atteignable au grooming, et c'est ce qui rend ce
   vol non dispatchable.
3. **Vérification positive de `db.rs:25-31`** — pas un build vert. Faire tourner
   un test qui exécute une requête `vec0` réelle contre une base ouverte après
   `init_sqlite_vec()`. Le `transmute` à travers `rusqlite::ffi` survit à la
   compilation même si la montée de `libsqlite3-sys` a changé ce qu'il traverse.
   Les sites existants `crates/mika-agent/src/db.rs:1099` et `:1136` et
   `crates/mika-agent/src/db/migrations/tests/kg_schema.rs:291` appellent déjà
   `init_sqlite_vec()` : ils sont le point de départ.
4. Attester `libsqlite3-sys 0.39.1` dans le lock (`grep -A3 'name = "libsqlite3-sys"' Cargo.lock`).
5. Retirer les **deux** exceptions RUSTSEC-2026-0459 — `deny.toml` et
   `.cargo/audit.toml`, dans le même commit.
6. `cargo audit` **puis** `cargo deny check`, en lisant la sortie des deux. Le
   document de 2026-10-21 § « Seconde occurrence » vaut ici : `cargo deny check`
   ne tourne pas quand `cargo audit` tombe, donc son premier passage vert est une
   **première** vérification et peut découvrir de la matière hors périmètre.
7. `Closes #3131` va sur la PR du vol C, et sur elle seule.

### Ou élargir la policy, si cette classe doit devenir dispatchable

Il faudrait `cargo update` autorisé, `~/.cargo/registry/src/` **inscriptible**, et
un clone joignable de `rustsec/advisory-db` (`deny.toml:5-6`). C'est accorder à un
pilote le droit de muter le graphe de dépendances du dépôt ; la décision est à
l'opérateur, et le `production posture` du refus se lit comme déjà prise dans
l'autre sens.

---

## Design

### D1 — les motifs d'exception portent la cascade, pas la montée

Les deux `reason` reçoivent la même matière, dans les deux syntaxes. Forme du
`deny.toml` :

```toml
[[advisories.ignore]]
id = "RUSTSEC-2026-0459"
reason = "Temporary exception set 2026-11-09, approved by Vincent; coût re-mesuré 2026-11-24 (mika#3131). libsqlite3-sys 0.30.1 (bundled SQLite < 3.54.0) via rusqlite 0.32 (Cargo.toml:30). Le remède (libsqlite3-sys >= 0.39.0, atteint par rusqlite 0.41.1) est une CASCADE de trois montées, pas une: rusqlite 0.41 et sqlx 0.8.6 declarent tous deux links = \"sqlite3\", donc cargo refuse de resoudre et sqlx doit monter en 0.9 (majeure, mika-gateway); sqlx 0.9.1 exige rustc 1.94.0 alors que rust-toolchain.toml fixe 1.93. Classe OPERATEUR, non dispatchable: le bac a sable monte ~/.cargo/registry/src en lecture seule, donc aucune caisse nouvelle ne se compile. Procedure: docs/plans/2026-11-24-001-chore-3131-rusqlite-0-41-cascade-plan.md. Keep in sync with .cargo/audit.toml."
```

Pourquoi le motif et non seulement le plan : `deny.toml` est ce qu'on `grep` quand
on se demande pourquoi un avis est ignoré. Un motif qui sous-estime le remède est
ce qui a produit un `p3-nice-to-have` dispatché à la boucle.

Les caractères accentués sont évités dans le `reason` — c'est une valeur TOML lue
par `cargo-deny` et rendue dans des sorties CI dont l'encodage n'est pas garanti.
Le `.cargo/audit.toml`, où la matière vit en **commentaire** et non en valeur, les
garde.

### D2 — le document couvre la classe, et nomme le piège du blocage trompeur

Le nouveau document sous `docs/solutions/dependency-maintenance/` porte le
frontmatter YAML du répertoire (`module`, `tags`, `problem_type`,
`resolution_type: routing`) et trois sections :

1. **Les deux classes sont une seule classe de routage.** Bump de lock transitif
   (2026-10-21) et montée déclarée dans `Cargo.toml` (ici) butent sur la même
   porte par deux chemins différents. Lien croisé depuis le document de 2026-10-21.
2. **Le blocage est trompeur, et voilà sa signature.** `Updating crates.io index`
   s'affiche, la résolution **aboutit**, un `Cargo.lock` est **écrit** — puis
   `Read-only file system (os error 30)` au dépaquetage. Un pilote peut croire
   avoir réussi et commiter un lock partiel. La signature à reconnaître est
   `failed to create directory …/registry/src/…`.
3. **Le préflight de cette classe-ci.** `cargo update --help` (M1) reste le
   premier geste, mais il ne suffit plus : pour une montée déclarée, il faut aussi
   savoir si une caisse **nouvelle** peut être dépaquetée. Le préflight est donc
   en deux temps, le second étant la tentative de résolution elle-même, dont
   l'échec au dépaquetage est la réponse.

### D3 — pourquoi ce vol ne livre aucune ligne d'adaptation

Trois faits se composent, et chacun suffirait :

- Le changelog 0.33 → 0.41 n'est pas lisible ici (M3, `WebFetch` refusé) : les
  changements seraient devinés.
- Rien ne compile sur le nouveau graphe (M6) : même juste, l'adaptation serait
  invérifiée.
- L'adaptation n'a pas de lock sous laquelle atterrir avant que sqlx et la
  toolchain aient bougé (M4, M7) : elle ne serait pas mergeable seule.

Livrer une adaptation ici serait du code que personne n'a compilé, contre une API
que personne n'a lue, sur un graphe qui ne résout pas.

---

## Fire-Disposition

**Gate N/A — ce vol ne livre aucun détecteur.**

Déclaré explicitement pour que l'absence de la section ne se lise pas comme un
oubli. Les deux fichiers touchés contiennent des entrées
`[[advisories.ignore]]` / `ignore = [...]`, qui **désarment** un détecteur
existant (`cargo audit`, `cargo deny check`) au lieu d'en livrer un ; le vol
n'ajoute ni test, ni assertion, ni règle de lint, ni garde CI, ni validateur de
schéma, et aucun livrable n'a « aucune violation trouvée » pour chemin de succès.
Les deux exceptions existent déjà en `main` et restent en place : ni leur nombre,
ni leur portée, ni leur durée ne changent — seul leur `reason` est réécrit.

Le détecteur que ce ticket finira par **réarmer** (le retrait des deux exceptions)
est au vol C du geste opérateur, hors de ce périmètre.

---

## Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | les deux motifs nomment la cascade | `grep -n 'links\|sqlx\|1\.94\|OPERATEUR\|operateur' deny.toml .cargo/audit.toml` → les quatre termes présents dans les deux entrées RUSTSEC-2026-0459 |
| V2 | les deux motifs restent déclarés synchrones | `grep -n 'Keep in sync' deny.toml .cargo/audit.toml` → une occurrence dans chaque, pointant vers l'autre fichier |
| V3 | `deny.toml` parse toujours | `cargo deny check advisories` → vert, et l'avis RUSTSEC-2026-0459 reste **ignoré** (pas de régression du job `Security`) |
| V4 | `.cargo/audit.toml` parse toujours | la liste `ignore` contient toujours exactement `RUSTSEC-2023-0071` et `RUSTSEC-2026-0459` |
| V5 | aucune mutation du graphe | `git diff --stat origin/main -- Cargo.toml Cargo.lock` → **vide** |
| V6 | le document porte son frontmatter | `head -20` du nouveau fichier : `module`, `tags`, `problem_type`, `resolution_type: routing` présents |
| V7 | le lien croisé existe dans les deux sens | le document de 2026-10-21 reçoit une ligne pointant le nouveau, et réciproquement |
| V8 | la chaîne complète | `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` |

V3 et V4 ne sont pas des formalités : une valeur TOML multi-lignes mal échappée
dans un `reason` fait rougir le job `Security` sur toute tête, et le § « Seconde
occurrence » du document de 2026-10-21 rappelle qu'une étape tombée masque les
suivantes.

---

## Definition of Done

- `deny.toml` et `.cargo/audit.toml` portent chacun un motif qui nomme le conflit
  `links = "sqlite3"`, la montée sqlx 0.8 → 0.9, l'exigence `rustc 1.94`, la classe
  opérateur, et le chemin de ce plan.
- Le document de solution est écrit, porte son frontmatter, et est lié depuis
  `docs/solutions/dependency-maintenance/2026-10-21-un-bump-de-lock-rustsec-nest-pas-dispatchable.md`.
- `Cargo.toml` et `Cargo.lock` sont **identiques** à `origin/main`.
- V1–V8 passent.
- La PR porte `Tracked in: senara-solutions/mika#3131` et **pas** `Closes #3131`.
- Le corps de PR nomme en une ligne la recommandation de routage : retirer
  `dispatch:loop`, poser `pipeline-exempt`, laisser le ticket ouvert.

## Acceptance criteria

Le ticket #3131 n'a pas de section `## Acceptance criteria` dans son corps (le
corps est de deux lignes : la source et les deux sites). Les critères ci-dessous
sont dérivés des Requirements et du Verification Contract, et ne portent que sur
le **vol courant**.

- **AC1** — `grep -n 'RUSTSEC-2026-0459' deny.toml .cargo/audit.toml` donne une
  entrée dans chaque fichier, et chaque entrée mentionne les quatre faits de M4,
  M5 et M7 : conflit `links = "sqlite3"`, sqlx 0.8 → 0.9, `rustc 1.94`, classe
  opérateur.
- **AC2** — `cargo deny check advisories` est vert et RUSTSEC-2026-0459 reste
  ignoré ; `cargo audit` ne régresse pas. Aucune exception n'est ajoutée ni retirée.
- **AC3** — `git diff origin/main -- Cargo.toml Cargo.lock` est vide : le vol ne
  mute pas le graphe de dépendances.
- **AC4** — un document existe sous
  `docs/solutions/dependency-maintenance/` qui (a) nomme la classe « montée
  déclarée dans `Cargo.toml` » comme non dispatchable, (b) donne la signature
  `failed to create directory …/registry/src/…` + `Read-only file system (os error 30)`
  comme marque du blocage trompeur, et (c) est atteignable depuis le document de
  2026-10-21.
- **AC5** — le plan contient la procédure opérateur en trois vols (A toolchain,
  B sqlx, C rusqlite + extinction), chacun avec son geste de vérification, et le
  vol C nomme la vérification positive de `db.rs:25-31` plutôt qu'un build vert.
- **AC6** — le ticket #3131 reste OPEN après le merge de ce vol, et la PR ne
  contient pas `Closes #3131`.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `deny.toml` (motif RUSTSEC-2026-0459 réécrit) | 9 |
| `.cargo/audit.toml` (commentaire réécrit) | 8 |

Total estimé : 17 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Le vol tient très largement sous les deux bornes, et il tient **par construction**
et non par chance : la seule matière volumineuse de ce ticket — l'adaptation des
484 sites rusqlite (M9) — est explicitement hors périmètre au § « Périmètre », pour
les trois raisons composées de D3. Ce qui reste est deux valeurs de configuration
et un document, et `docs/` ne compte pas.

Le découpage en vols n'est donc pas motivé par la taille : il est motivé par la
**capacité du bac à sable** (M6). Les vols A, B et C du geste opérateur ne sont pas
des phases de celui-ci — ils sont hors de la file de dispatch, et chacun demandera
sa propre estimation quand il sera cadré.

---

## Ordre de commit

1. `chore(deps): le motif RUSTSEC-2026-0459 nomme la cascade, pas une montée (mika#3131)`
   — `deny.toml` + `.cargo/audit.toml` dans **un seul** commit : les deux motifs se
   déclarent mutuellement synchrones, les séparer rend l'un des deux faux pendant
   un commit.
2. `docs(solutions): une montée déclarée dans Cargo.toml n'est pas plus dispatchable (mika#3131)`
   — le nouveau document + la ligne de lien croisé dans celui de 2026-10-21.
3. `docs(plans): groom mika#3131` — ce fichier.
