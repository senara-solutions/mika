---
title: "Une dépendance déclarée par cible n'est évaluée que par sa cible — et le premier lecteur fut un tag"
date: 2026-10-23
category: build-errors
tags: [cargo, target-dependencies, cfg, libc, macos, darwin, release, ci-coverage, E0433]
modules:
  - crates/mika-agent/Cargo.toml
  - crates/mika-agent/src/skills/executor.rs
  - crates/mika-agent/tests/libc_target_scope_2900.rs
  - .github/workflows/release.yml
  - .github/workflows/ci.yml
severity: high
resolved: true
issue: "#2900"
---

# Une dépendance déclarée par cible n'est évaluée que par sa cible

## Le constat

Le candidat `v1.0.0-rc.1` — premier tag de release du dépôt — n'est pas publié.
Le run `release.yml` 39961929051 (2026-10-23) construit quatre cibles et ne
publie que si les quatre passent ; les deux builds macOS sortent avec le code
101 sur la même erreur :

```
error[E0433]: failed to resolve: use of unresolved module or unlinked crate `libc`
    --> crates/mika-agent/src/skills/executor.rs:1430:27
    --> crates/mika-agent/src/skills/executor.rs:1444:39
error: could not compile `mika-agent` (lib) due to 3 previous errors
```

Le défaut est entré le 2026-09-22 (70127bbf, mika#2438) et a été vu le
2026-10-23. Un mois, pendant lequel toute la CI était verte sur chaque PR.

## Cause racine

`crates/mika-agent/Cargo.toml` déclarait :

```toml
[target.'cfg(target_os = "linux")'.dependencies]
libc.workspace = true
```

et le `Drop` de `ProcessGroupKillGuard` (`src/skills/executor.rs`) appelle
`libc::killpg`, `libc::SIGKILL` et `libc::ESRCH` **hors de tout `#[cfg]`**.

Sous Linux la table de cible s'applique, le crate entre dans le graphe de
`mika-agent`, le nom `libc` est dans son extern prelude et le code résout. Pour
une cible darwin la table ne s'applique pas — et c'est **tout** ce qui change.

**La nuance qui rend le diagnostic non évident, et qu'un raisonnement rapide
inverse.** `libc` est dans le graphe darwin *de toute façon*, transitivement :
`core-foundation` et `security-framework`, tirés par
`rustls-platform-verifier`, en dépendent. Le crate est donc compilé et lié dans
le binaire. Ce qui manquait est le lien **direct** `mika-agent → libc`, le seul
qui place le nom dans l'extern prelude du crate. C'est exactement ce que dit le
message du compilateur — *« unresolved module or unlinked crate »* — mais il le
dit pour **ce crate-là**, pas pour le graphe. Mesure :

```
$ cargo tree -p mika-agent -i libc --target aarch64-apple-darwin --depth 1
libc v0.2.191
├── core-foundation v0.10.1
├── …                       # 20 dépendants, mika-agent n'y est pas
```

contre la même commande `--target x86_64-unknown-linux-gnu`, où `mika-agent`
figure. « `libc` est-il dans le graphe » est donc la **mauvaise question** ; la
bonne est « `mika-agent` est-il dépendant **direct** de `libc` pour cette
cible ».

## Pourquoi il a vécu un mois

Une table de dépendances par cible est un contrat que **seule une compilation
pour cette cible évalue**. Aucun job de `ci.yml` ne compile pour une cible
darwin : `Check` tourne sur `[self-hosted, Linux, X64, gentux]` et les autres
sur `ubuntu-22.04`. La première évaluation du manifeste pour une cible darwin a
eu lieu au premier tag.

Un `cargo check` Linux vert ne dit **rien** d'un `cfg(target_os)` qui n'est pas
le sien. Et comme le dépôt n'avait jamais été taggé, la matrice de release
n'avait jamais été exercée : le contrat existait depuis la création de la table
sans avoir eu un seul lecteur.

Les deux autres consommateurs de `libc` du crate ne cassaient pas, et pour une
raison qui mérite d'être nommée — ils étaient gardés, avec un repli explicite :

| site | garde | repli hors Linux |
|---|---|---|
| `task_engine/process_liveness.rs` (`libc::kill`) | `#[cfg(target_os = "linux")]` interne | `false` — watchdog désactivé |
| `worktree_reaper.rs` (`libc::flock`) | `#[cfg(target_os = "linux")]` sur trois sites | `LockProbe::Unevaluable` — conserve |
| `skills/executor.rs` (`libc::killpg`) | **aucune** | — |

Le seul non gardé est exactement celui que le compilateur nomme. Une déclaration
par cible plus étroite que la portée du code est un défaut **latent** : il ne se
manifeste qu'au premier site non gardé, et ce site peut être ajouté des mois
après la déclaration, par quelqu'un qui n'a aucune raison de lire le manifeste.

## Correctif

Deux lignes : la clé de la table passe à `cfg(unix)`.

```toml
[target.'cfg(unix)'.dependencies]
libc.workspace = true
```

Aucun `#[cfg]` de site n'est touché, `Cargo.lock` ne bouge pas (il ne liste pas
les dépendances par cible), et sous Linux `cfg(unix)` est vrai — le graphe y est
identique.

**La clé ne sur-approxime pas**, et c'est ce qui la justifie plutôt que
`cfg(any(target_os = "linux", target_os = "macos"))` : `executor.rs` appelle
`.process_group(0)` sans `#[cfg]`, méthode de
`std::os::unix::process::CommandExt`, donc le crate n'est compilable sur aucune
cible non-unix. Il n'existe pas de cible où `cfg(unix)` déclarerait `libc` dans
le vide. L'élargissement n'ouvre aucune plateforme ; il fait **coïncider la
déclaration avec la portée que le code avait déjà**.

L'alternative — borner les appels d'`executor.rs` à Linux — a été refusée :
`killpg`, `SIGKILL` et `ESRCH` existent sur macOS, et borner la garde
laisserait un groupe de processus orphelin sur la plateforme où le pilote n'a
justement pas de superviseur OpenRC. Faire compiler en désarmant la garde
échange une erreur visible contre une fuite silencieuse.

## Ce qu'il faut retenir

**Quand un site appelle un crate déclaré par cible, la clé de la table doit
couvrir toutes les cibles de `release.yml` — et c'est la matrice de release, pas
celle de CI, qui définit l'ensemble à couvrir.** Les deux divergeaient ici de
deux cibles sur quatre, et cette divergence était invisible par construction :
rien dans le dépôt ne la nommait.

Trois corollaires, chacun payé par ce ticket :

1. **Une CI qui ne compile pas une cible ne garde pas son manifeste.** Garder
   une table de cible demande ou bien un job qui compile pour cette cible, ou
   bien un substitut qui lit le manifeste. Le second est ce qui a été livré ici
   (`crates/mika-agent/tests/libc_target_scope_2900.rs`, un parseur TOML de
   quarante lignes, sans sous-processus) parce que `.github/workflows/` est hors
   du scope OAuth du pilote ; le job darwin part dans un ticket de suivi. Les
   deux sont utiles : le test rougit en local et sur chaque PR, le job prouve le
   link.
2. **Un détecteur de manifeste doit porter son auto-péremption.** La garde de
   clé n'est nécessaire *que* parce qu'un site appelle `libc` sans `#[cfg]`. Une
   seconde assertion exige que ce site existe encore, pour que la première ne
   survive pas en contrainte morte sur un manifeste qu'elle contraindrait sans
   raison.
3. **Une affirmation sur le manifeste vieillit avec le manifeste.** Deux textes
   du dépôt — un doc-comment de `worktree_reaper.rs` et son jumeau anglais dans
   `docs/agent-internals/tools.md` — présentaient la clé Cargo comme la *cause*
   du `#[cfg(target_os = "linux")]` des sites `flock`. C'est précisément cette
   lecture qui a produit le ticket : la clé prise pour une contrainte de
   plateforme alors qu'elle n'était qu'une déclaration trop étroite. Les deux
   gardent leur conclusion et changent leur raison — ce qui impose ce `#[cfg]`
   est que la sémantique de `flock` et la population de `.cargo-lock` ne sont
   vérifiées que sous Linux.

## Portée du correctif, dite honnêtement

`mika-agent` **compile** désormais pour darwin ; il n'y est pas plus
**fonctionnel** qu'avant. `read_process_start_time` lit `/proc`,
`cargo_build_lock_is_free` sonde avec `flock` : hors Linux ils rendent
respectivement `None`/`false` et `Unevaluable`. Régime documenté et délibéré,
que ce correctif n'élargit pas.

Et aucune commande exécutable hors d'un hôte Apple ne prouve qu'un binaire
darwin se **link** : `cargo tree` et le test de manifeste prouvent que la
résolution de noms passe, rien de plus. Le link est prouvé par un run manuel de
`release.yml`, et par rien d'autre.
