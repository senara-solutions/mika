---
title: Un module de test se reconnaît à sa déclaration `#[cfg(test)] mod …;`, pas à son chemin
date: 2026-10-26
category: best-practices
module: mika-agent/source_scan
problem_type: best_practice
component: testing_framework
severity: medium
tags:
  - "structural-guard"
  - "source-scan"
  - "test-classification"
  - "mika-2321"
  - "mika-2921"
applies_when:
  - "Vous écrivez une garde de source qui appelle `source_scan::production_files()`"
  - "Vous extrayez un module de test dans un fichier dont le nom n'est pas `tests.rs`"
  - "Une garde neuve rougit sur son propre fichier, ou sur `source_scan.rs` ou `test_utils.rs`"
---

# Un module de test se reconnaît à sa déclaration, pas à son chemin

## Le fait

`crate::source_scan::production_files()` est le lecteur partagé des gardes de
source : il rend la moitié production de chaque `.rs` de `crates/mika-agent/src`.
Il écarte le code de test de deux façons, par le **chemin**
(`is_test_source_path` : un segment `tests/` ou un fichier `tests.rs`, la
réparation de mika#2321), puis par la **troncature** au `#[cfg(test)] mod tests`
inline (`production_half`).

Il existe une troisième forme qu'aucune des deux ne voit : un module **extrait**
dont le nom n'est ni `tests` ni sous `tests/`, et que son parent déclare en test :

```rust
#[cfg(test)]
mod no_dispatch_test;
```

Un tel fichier ne porte aucun littéral `#[cfg(test)] mod tests`, et son chemin
n'a rien d'un chemin de test. `production_files()` le rend donc **entier comme
de la production**. Relevé du 2026-10-26 dans `crates/mika-agent/src`, en
cherchant les déclarations `#[cfg(test)]` suivies d'un `mod <nom>;`. Dans tout
ce document, un chemin de fichier sans racine est relatif à
`crates/mika-agent/src/` :

| fichier | déclaré dans | lu comme production par `production_files()` |
|---|---|---|
| `source_scan.rs` | `lib.rs` | oui |
| `test_utils.rs` | `lib.rs` | oui |
| `milestone_manager/no_dispatch_test.rs` | `milestone_manager/mod.rs` | oui |
| `milestone_manager/no_repo_list_test.rs` | `milestone_manager/mod.rs` | oui |

C'est la prémisse de mika#2321, revenue par une forme que sa réparation ne
couvrait pas. Comme alors, le défaut est bénin **par chance** : aucune des
gardes qui appellent `production_files()` aujourd'hui (`qa_reviewed_head.rs`,
`qa_head_supersession.rs`, `source_scan::audit_writers`) ne cherche une aiguille
que ces quatre fichiers portent.

## Comment c'est apparu

La garde d'AC3 de mika#2921 (`no_repo_list_test.rs`) refuse un littéral
`senara-solutions/` et tout identifiant `_REPOS` dans la production du module
`milestone_manager/`. Son contrôle de bonne foi pose ces motifs dans des
sources fabriquées, donc **dans son propre fichier**. Bâtie sur
`production_files()` seule, elle se serait scannée elle-même et aurait rougi sur
son propre contrôle. Le même raisonnement couvre `no_dispatch_test.rs`, qui écrit
tous les jetons interdits de sa garde en littéraux. Ce fichier s'en sort par une
exemption **par nom de fichier** (`EXEMPT_FILES`), la forme que mika#2321 refuse
pour toute nouvelle garde.

## La règle

**Un module de test se reconnaît à la déclaration qui le rend test.** Une garde
qui scanne un module où vivent des tests extraits lit la déclaration dans le
fichier parent, au lieu de recopier une liste de noms :

```rust
/// Les modules de test extraits déclarés par `mod_rs` : un `#[cfg(test)]`
/// immédiatement suivi de `mod <nom>;`.
fn modules_de_test_extraits(mod_rs: &str) -> Vec<String> {
    let lignes: Vec<&str> = mod_rs.lines().map(str::trim).collect();
    lignes
        .windows(2)
        .filter(|w| w[0] == "#[cfg(test)]")
        .filter_map(|w| {
            let nom = w[1].strip_prefix("mod ")?.strip_suffix(';')?;
            Some(format!("{nom}.rs"))
        })
        .collect()
}
```

Ce qui en fait une classification et non une exemption déguisée :

- le nom du fichier n'est écrit **nulle part** dans la garde. C'est l'attribut
  porté par le parent qui décide, l'attribut même que le compilateur lit ;
- un anti-vacuité épingle que la lecture trouve bien les modules attendus. Dans
  `no_repo_list_test.rs`, `mika2921_le_scan_a_regarde_larbre_reel` exige les deux
  gardes dans la liste et hors des fichiers scannés. Une lecture qui ne
  trouverait rien rendrait le scan identique à celui d'avant ;
- la forme est volontairement étroite : `#[cfg(test)]` seul sur sa ligne, puis
  `mod <nom>;` sans visibilité. `#[cfg(test)] pub mod test_utils;` (lib.rs) n'est
  pas reconnu, ce qui est sans effet pour une garde bornée à
  `milestone_manager/`. Une garde sur tout `src/` devrait élargir la forme
  reconnue à `pub mod`/`pub(crate) mod` et dire pourquoi.

## Ce qui n'a pas été fait, et pourquoi

`production_files()` n'a pas été corrigée par mika#2921. Le vol n'en avait pas
le périmètre, et la corriger change la population de trois gardes existantes.
C'est une décision à prendre avec ses contrôles négatifs, pas un effet de bord.
Le geste naturel serait que `production_files()` collecte ces déclarations dans
**tous** les fichiers parents (`lib.rs`, chaque `mod.rs` et chaque fichier
module), puis écarte les fichiers désignés, avec un test qui rougit si l'un des
quatre fichiers du tableau ci-dessus réapparaît dans la population.

## Note d'applicabilité

Avant d'écrire une garde sur `production_files()`, demandez-vous si un module de
test extrait sous un autre nom que `tests` vit dans le périmètre scanné, et si
la garde y écrit ses propres aiguilles. Si les deux réponses sont oui, la garde
se scanne elle-même. La réparation est la classification par déclaration,
jamais une exemption par nom de fichier.

## L'apprentissage que ce document ne porte pas

Le brief proposait un autre candidat : « lu, et vide » n'est pas « pas lu »
(`PrStateKnowledge`), et le périmètre réutilise la liste la plus serrée existante
(`DISPATCHABLE_REPOS`). Ce raisonnement est déjà durable là où le code vit : le
plan `docs/plans/2026-10-26-003-feat-2921-manager-l-valuateur-juge-tout-vol-plan.md`,
le doc-comment de `PrStateKnowledge` dans `milestone_manager/evaluator.rs` et
celui de `flight_source.rs`, tenus par les tests de mika#2921. La classification
ci-dessus n'était écrite nulle part en dehors de la garde qui l'a découverte.
