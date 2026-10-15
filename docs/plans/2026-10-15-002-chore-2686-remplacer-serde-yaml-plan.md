---
date: 2026-10-15
type: chore
issue: mika#2686
---

# Plan : remplacer `serde_yaml` 0.9 par `yaml_serde`, lever RUSTSEC-2024-0449

## Problème

`cargo deny check` (job Security) refuse `unsafe-libyaml` 0.2.11 — RUSTSEC-2024-0449,
non maintenu, aucune faille connue, aucune mise à jour sûre. L'arbre inverse est court
et sans intermédiaire :

```
unsafe-libyaml v0.2.11
└── serde_yaml v0.9.34+deprecated
    ├── mika-agent
    └── mika-cli
```

mika#2685 a posé une exception `[[advisories.ignore]]` dans `deny.toml` en nommant
ce ticket comme le vrai remède. Ce plan est ce remède.

## Recensement des usages (ce que le groom devait établir)

**69 occurrences, 18 fichiers, deux crates.** Toutes sont pleinement qualifiées
(`serde_yaml::…`) — **aucun `use serde_yaml`** dans le dépôt, donc aucune ligne
d'import à réécrire.

| crate | fichiers | occurrences | nature |
|---|---|---|---|
| `mika-cli` | 17 (`src/commands/*.rs`) | 68 | sérialisation `--format yaml` |
| `mika-agent` | 1 (`src/calibration/role.rs:316`) | 1 | désérialisation d'un manifeste |

Détail de la surface d'API réellement employée — c'est elle qui décide du remplaçant :

- **`serde_yaml::to_string(&T)`** — 62 sites. Le point central est
  `mika-cli/src/commands/format_helper.rs::print_structured`, les autres sont des
  `print!` directs dans les sous-commandes (`agents` 11, `teams` 9, `skills` 7,
  `skills_variants` 6, `kg` 5, `provider` 5, `webhook` 3, `tasks` 3, `reminders` 2,
  `logs` 2, `model` 2, `milestone` 2, `ask` 2, `status` 1, `config` 1, `iterate` 1).
- **`serde_yaml::from_str::<T>`** — 3 sites : `role.rs:316` (production, charge
  `tests/eval/calibration_fixtures/<role>/manifest.yaml`) et deux tests de
  `format_helper.rs`.
- **`serde_yaml::Value`** (dont `Value::String` et l'indexation `v["name"]`) —
  2 sites, tous deux dans les tests de `format_helper.rs`.

Aucun usage de `Mapping`, `Number`, `Sequence`, `Deserializer`, `Serializer`,
`to_writer`, `from_reader`, `with::*`. Aucune occurrence hors `src/` : ni
`crates/*/tests/`, ni `build.rs`, ni `scripts/`, ni `.github/`. Aucun test ne
compare une sortie YAML à une chaîne littérale.

Les autres lecteurs de YAML du dépôt **ne passent pas par `serde_yaml`** et ne sont
donc pas concernés : le frontmatter de `docs/solutions/**` est découpé à la main par
`kg/chunker.rs` et `skills/builtin_handlers.rs`, et les manifestes de skills sont en
TOML.

## Choix de la bibliothèque : `yaml_serde` 0.10

**`yaml_serde` est déjà dans notre graphe de dépendances.** `utoipa` 6.0.0, déclaré
avec la feature `yaml` dans le `Cargo.toml` du workspace (ligne 96) et consommé par
`mika-agent`, en dépend déjà — `Cargo.lock` porte aujourd'hui `yaml_serde 0.10.7` et
`libyaml-rs 0.3.0`, tous deux compilés à chaque build.

C'est l'argument décisif, et il est mesurable plutôt que réputationnel : **le
remplacement n'ajoute aucune caisse transitive au graphe, aucune licence nouvelle,
aucun temps de compilation**. Il en **retire** deux (`serde_yaml`, `unsafe-libyaml`).

Les quatre propriétés vérifiées localement :

| propriété | valeur | vérifié par |
|---|---|---|
| API | fork de `serde_yaml` 0.9 — `from_str`, `to_string`, `Value`, `Index`, `Mapping`, `Number`, `Sequence`, `Error`, `Result`, `with`, `io` | `src/lib.rs:190-210` du crate |
| licence | `MIT OR Apache-2.0` (et `MIT` pour `libyaml-rs`) | déjà dans `deny.toml` `[licenses].allow` |
| parseur | `libyaml-rs` 0.3.0 — fork maintenu, **nom distinct** de `unsafe-libyaml` | `Cargo.toml` du crate |
| présence | déjà dans `Cargo.lock` via `utoipa` 6.0.0 | `Cargo.lock:5172-5180` |

La surface utilisée (`to_string`, `from_str`, `Value`, `Value::String`, `Index`) est
intégralement exportée : le remplacement est un **renommage de chemin**, sans
adaptation d'appel.

### Alternatives écartées, nommées pour qu'on ne les repropose pas à l'aveugle

- **`serde_norway` 0.9.42** (fork cafkafk, parseur `unsafe-libyaml-norway` 0.2.15) —
  même API, même qualité de maintenance, mais **ajoute deux caisses au graphe** là où
  `yaml_serde` n'en ajoute aucune. C'est le **repli** si `libyaml-rs` se révélait
  porter un avis RUSTSEC (voir « Risque » ci-dessous) : le renommage serait le même
  geste, `serde_norway::` au lieu de `yaml_serde::`.
- **`serde_yml`** — écarté : historique de maintenance contesté, et il vendorise
  `unsafe-libyaml`, donc ne lève pas l'avis.
- **`saphyr` / `yaml-rust2`** — Rust pur, mais sans adaptateur `serde` compatible de
  la forme `to_string(&T)` / `from_str::<T>()`. Les 62 sites de sérialisation
  devraient être réécrits : coût sans rapport avec le bénéfice.
- **Alias de paquet** — `serde_yaml = { package = "yaml_serde", version = "0.10" }`
  dans le `Cargo.toml` du workspace ramènerait la PR à trois lignes et satisferait
  littéralement les critères d'acceptation. **Refusé** : un lecteur qui grep
  `serde_yaml` dans `crates/mika-cli/src/` trouverait 68 appels à une caisse
  dépréciée qui n'est plus là, et le prochain audit de dépendances partirait sur une
  fausse piste. Le renommage honnête coûte 68 lignes mécaniques — très en deçà de la
  borne par vol — et c'est ce qu'on paie pour que le nom dise la vérité.

### Risque nommé : l'avis sur `libyaml-rs` n'est pas vérifiable hors CI

`~/.cargo/advisory-db` est absent de cette machine, donc **ce groom n'a pas pu
établir que `libyaml-rs` 0.3.0 ne porte aucun avis RUSTSEC** ; il a établi qu'il est
déjà compilé chez nous via `utoipa`, qu'il est maintenu et qu'il porte un nom distinct
de la caisse visée par RUSTSEC-2024-0449. Le `cargo deny check` du pas de vérification
tranche cette question avant la PR, et le job Security la retranche après. Si l'avis
existe, le repli est `serde_norway` et le geste est identique.

## Implémentation — un vol, deux commits

### Commit 1 — `chore(deps): remplacer serde_yaml 0.9 par yaml_serde (mika#2686)`

1. `Cargo.toml` (racine), ligne 27 : `serde_yaml = "0.9"` → `yaml_serde = "0.10"`.
   Garder l'entrée dans l'ordre alphabétique du bloc si c'est la convention locale.
2. `crates/mika-agent/Cargo.toml` ligne 71 et `crates/mika-cli/Cargo.toml` ligne 35 :
   `serde_yaml.workspace = true` → `yaml_serde.workspace = true`.
3. Renommer `serde_yaml::` → `yaml_serde::` dans les 18 fichiers recensés. Le geste
   est mécanique et sans exception : il n'y a ni import ni alias ni chaîne de
   caractères contenant `serde_yaml` dans `src/`. Outil `Edit` avec `replace_all`
   fichier par fichier (pas de `sed -i` : refus terminal).
4. `cargo build --workspace` régénère `Cargo.lock` — le diff attendu est le retrait
   des paquets `serde_yaml` et `unsafe-libyaml` et l'ajout de `yaml_serde` aux listes
   de dépendances de `mika-agent` et `mika-cli`. `yaml_serde` et `libyaml-rs` ont déjà
   leur entrée (via `utoipa`) : **aucun checksum nouveau** n'apparaît. Un diff plus
   large que cela est un signal — s'arrêter et le lire avant de commiter.

### Commit 2 — `chore(deny): lever l'exception RUSTSEC-2024-0449 et bannir les caisses retirées (mika#2686)`

5. `deny.toml` lignes 20-22 : retirer le bloc `[[advisories.ignore]]`
   `RUSTSEC-2024-0449`. Les trois exceptions voisines (`RUSTSEC-2026-0253`, `-0221`,
   `-0205`) sont hors périmètre et restent intactes.
6. `[bans].deny` : ajouter `{ name = "serde_yaml" }` et `{ name = "unsafe-libyaml" }`
   dans un **sous-bloc commenté séparé**, sous le bloc existant des fournisseurs
   STT/TTS. Le commentaire du bloc voisin documente l'invariant de non-transit de la
   voie testimony (mika#1796) et son processus d'échappement ; y mêler des entrées
   YAML sans séparation diluerait cette documentation. Le commentaire du nouveau
   sous-bloc nomme mika#2686, l'avis levé, et le geste d'échappement (retirer
   l'entrée en motivant, comme pour toute entrée de ce tableau).

Les deux commits sont dans la même PR : le commit 1 seul laisserait `deny.toml`
porter une exception sans objet, le commit 2 seul casserait le job Security.

## Fire-Disposition

Ce plan livre **un détecteur** : les deux entrées `[bans].deny` du commit 2. Leur
chemin de succès est « aucune violation trouvée » — elles rougissent le job Security
si `serde_yaml` ou `unsafe-libyaml` revient dans le graphe, par dépendance directe ou
transitive.

**Option (a) — exception nommée en allowlist, et l'allowlist est livrée VIDE.**

- **Zéro violation existante au moment où le détecteur atterrit.** C'est l'ordre des
  commits qui le garantit : le commit 1 retire les deux caisses du graphe, le commit 2
  les bannit. Aucune entrée n'est donc ajoutée à `[bans].allow`, qui reste tel quel.
- **Rien ne peut devenir stale, parce qu'il n'y a rien à nettoyer.** Une entrée
  d'allowlist périmée est exactement ce qu'une allowlist vide ne peut pas produire. La
  propriété auto-nettoyante est ici structurelle plutôt qu'assertive : si une entrée
  apparaît un jour dans `[bans].allow` pour une de ces deux caisses, c'est que la
  caisse est revenue — et l'entrée porte alors son ticket, par le processus
  d'échappement déjà écrit en commentaire au-dessus du tableau.
- **Le détecteur est déjà exécuté par la CI existante.** `cargo deny check` (job
  Security, `.github/workflows/ci.yml:797`) lance la sous-vérification `bans` sans
  configuration supplémentaire. Aucun job, aucun script, aucun lint nouveau.
- **Coût nommé.** Une dépendance future qui tirerait `serde_yaml` en transitif fera
  échouer le job Security de façon dure plutôt que d'être avalée en silence. C'est
  l'effet recherché, et la sortie est documentée au-dessus du tableau.

## Contrat de vérification

Dans l'ordre, après le commit 2 :

1. `cargo tree -i unsafe-libyaml` — doit **refuser** (« package ID specification
   did not match any packages »). C'est AC1, verbatim.
2. `cargo tree -i serde_yaml` — même refus attendu.
3. `cargo tree -i yaml_serde` — doit montrer trois consommateurs : `utoipa`,
   `mika-agent`, `mika-cli`. C'est le contrôle positif : sans lui, « l'arbre est vide »
   et « la commande ne sait pas chercher » rendent les mêmes octets.
4. `cargo deny check` — `cargo-deny` est installé sur cette machine
   (`~/.cargo/bin/cargo-deny`) mais `~/.cargo/advisory-db` est **absent** : la
   sous-vérification `advisories` doit d'abord cloner la base, ce que le bac à sable
   de dispatch peut refuser. Si le clone échoue, lancer
   `cargo deny check bans licenses sources` (hors ligne) et **le dire dans le corps de
   PR** : le job Security est alors le seul juge d'AC2, et le pilote ne doit pas
   prétendre l'avoir tranché localement.
5. `cargo test --workspace` — suite complète. Les deux tests de round-trip de
   `format_helper.rs` sont la couverture directe du changement.
6. `cargo test -p mika-common mika2398` — AC4, lancé explicitement en plus de la suite.
7. `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
8. Fumée sur la sortie réelle : `cargo run --bin mika -- status --format yaml` et
   `cargo run --bin mika -- agents list --format yaml`. `yaml_serde` est un fork de
   l'émetteur de `serde_yaml` (mêmes `itoa`/`ryu`, même lignée libyaml), donc une
   sortie identique est l'attendu — mais c'est un attendu, pas une mesure, et aucun
   test ne compare une sortie YAML à un littéral. Si l'indentation ou le marqueur
   `---` diffère, le signaler dans le corps de PR : c'est un changement visible par
   les scripts opérateur qui consomment `--format yaml`.
9. `scripts/verify-pipeline.sh origin/main` et les lints du dépôt.

## Definition of Done

- Le workspace compile, la suite complète est verte, clippy est propre, `cargo fmt` ne
  rend rien.
- `grep -rn "serde_yaml" --include="*.rs" .` ne rend plus rien. Les mentions
  résiduelles sous `docs/plans/` et `docs/solutions/` sont de l'histoire et restent
  telles quelles.
- `deny.toml` ne porte plus `RUSTSEC-2024-0449` et porte les deux bans commentés.
- Le corps de PR dit explicitement si `cargo deny check advisories` a tourné
  localement ou si le job Security est le seul juge (point 4 ci-dessus), et signale
  toute différence de forme sur la sortie `--format yaml` (point 8).
- Pas de migration manuelle à documenter : aucun changement de surface publique,
  aucun fichier de configuration opérateur touché.

## Acceptance criteria

Transcrits du corps de mika#2686 :

- **AC1** — `cargo tree -i unsafe-libyaml` ne rend plus rien.
- **AC2** — `deny.toml` ne porte plus d'exception `RUSTSEC-2024-0449`, et le job
  Security est vert.
- **AC3** — Les tests existants qui lisent du YAML (manifestes, frontmatter,
  configuration) passent **sans modification de leurs fixtures**. Concrètement :
  `crates/mika-agent/src/calibration/role.rs::load` continue de désérialiser
  `tests/eval/calibration_fixtures/<role>/manifest.yaml` tel quel, et les deux tests
  de round-trip de `format_helper.rs` passent avec `yaml_serde::Value`.
- **AC4** — `cargo test -p mika-common mika2398` est vert.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `Cargo.toml` (racine) + `crates/mika-agent/Cargo.toml` + `crates/mika-cli/Cargo.toml` | 3 |
| renommage dans `crates/mika-cli/src/commands/*.rs` (17 fichiers, 68 occurrences) | 67 |
| renommage dans `crates/mika-agent/src/calibration/role.rs` | 1 |
| `deny.toml` (retrait de l'exception −4, sous-bloc `[bans]` commenté +8) | 12 |

Total estimé : 83 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

`Cargo.lock` est régénéré et non compté : il est produit par `cargo build`, pas écrit
à la main, et son diff attendu est de l'ordre de quinze lignes de suppression.

**Un seul vol, et voici pourquoi il tient** — à 83 lignes il est à 34 % de la borne,
et la nature du travail est ce qui le rend sûr plutôt que le seul chiffre : 68 des 83
lignes sont un renommage mécanique d'un chemin de module pleinement qualifié, sans
import à ajuster, sans adaptation d'appel, sans fichier de test nouveau, sans fixture
touchée et sans changement de comportement. Il n'y a ni découpage en phases ni suite à
tracker : le ticket est un geste unique, et le scinder laisserait l'un des deux
commits incohérent avec la CI.
