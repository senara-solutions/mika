---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#2896
branch: fix/2896/deps-monter-ctutils-0-4-4-rustsec-2026
---

# fix(deps): monter `ctutils` ≥ 0.4.4 (RUSTSEC-2026-0385), le check requis Security est rouge sur toute PR

## Problem Frame

`Cargo.lock:805-811` résout :

```toml
[[package]]
name = "ctutils"
version = "0.4.2"
checksum = "7d5515a3834141de9eafb9717ad39eea8247b5674e6066c404e8c4b365d2a29e"
dependencies = [
 "cmov",
]
```

RUSTSEC-2026-0385, publié le 2026-10-23, porte sur cette version et demande
`>= 0.4.4`. Le job **Security** du CI (`ci.yml:1072-1084`) lance `cargo audit`
sur le lock et rougit donc sur **toute** PR ouverte depuis la publication —
relevé à la source sur le run 39938791465 de mika#2894. Aucune PR, y compris
celles de la boucle, ne peut avoir tous ses checks requis verts. C'est du
substrat : le défaut n'est pas dans le produit, il est dans la porte que tout
vol doit franchir.

Le dernier CI vert de `main`, sur `94b21916`, a tourné le 2026-10-23 à 08:01Z,
avant la publication de l'avis — le rouge est donc né d'un changement du monde
extérieur, pas d'un commit du dépôt.

### Par où `ctutils` entre dans le graphe

Un seul chemin, et il est court :

- `Cargo.toml:169` déclare `hmac = "0.13"`, `Cargo.toml:171` déclare `sha2 = "0.11"`
  (deps **directes** du workspace, consommées par `mika-agent`, `mika-gateway`
  et `mika-common`).
- `hmac` 0.13.0 (`Cargo.lock:1564-1571`) et `sha2` 0.11.1 (`Cargo.lock:3993-4002`)
  tirent tous deux `digest` 0.11.3.
- `digest` 0.11.3 (`Cargo.lock:949-959`) liste `ctutils` dans ses dépendances,
  via sa feature `mac` (`digest-0.11.3/Cargo.toml:51` — `mac = ["dep:ctutils"]`),
  que `hmac` active.

La seconde chaîne cryptographique du lock — `sha2` 0.10.9, `hmac` 0.12.1,
`digest` 0.10.7 — **ne tire pas** `ctutils` : `digest` 0.10.7 dépend de
`block-buffer` 0.10 et `crypto-common` 0.1, pas de `ctutils`. Il n'y a donc
qu'**une seule** entrée `ctutils` dans les 575 paquets du lock, et un seul
dépendant à satisfaire.

### Le fait qui décide de la forme du correctif

`digest` 0.11.3 déclare sa contrainte sur `ctutils` en **caret**, pas en version
exacte — lu dans le crate téléchargé,
`~/.cargo/registry/src/index.crates.io-*/digest-0.11.3/Cargo.toml:83-85` :

```toml
[dependencies.ctutils]
version = "0.4"
optional = true
```

`"0.4"` admet tout `0.4.z`. Les versions 0.4.4 et 0.4.5 étant publiées sur
crates.io, `cargo update -p ctutils` les atteint **sans toucher `Cargo.toml`**,
et sans monter `digest`, `sha2` ni `hmac`. Le « si la contrainte de `digest`
l'admet » du ticket est donc tranché par la lecture : elle l'admet. Ce vol ne
modifie aucun manifeste.

## Requirements

- **R1** — `Cargo.lock` résout `ctutils` à une version ≥ 0.4.4.
- **R2** — Aucun `Cargo.toml` n'est modifié. La montée est une re-résolution du
  lock sous des contraintes inchangées.
- **R3** — `ctutils` est le seul paquet dont l'entrée du lock change. Si la
  re-résolution en entraîne d'autres, chacun est **listé dans le corps de PR
  avec sa raison** — le seul candidat structurel est `cmov` (`Cargo.lock:525-528`,
  aujourd'hui 0.5.4), unique dépendance de `ctutils`.
- **R4** — Aucune entrée `[[advisories.ignore]]` n'est ajoutée à `deny.toml`
  pour RUSTSEC-2026-0385. Voir *Non-objectifs*.
- **R5** — Le job **Security** est vert sur la tête de la PR, dans ses **deux**
  commandes : `cargo audit` et `cargo deny check` (`ci.yml:1081-1084`).
- **R6** — `cargo build`, `cargo test --workspace` et
  `cargo clippy --all-targets --all-features -- -D warnings` sont verts.

## Non-objectifs

**Pas d'`[[advisories.ignore]]`.** `deny.toml:8-18` porte déjà trois entrées de
ce genre (RUSTSEC-2026-0253, -0221, -0205), et chacune nomme la même raison :
une *unsoundness* en transitif **sans correctif publié**, dont on argumente que
le dépôt ne déclenche pas le chemin fautif. Ici le correctif **est** publié —
0.4.4 et 0.4.5 sont sur crates.io. Un `ignore` à la place d'une montée
échangerait deux lignes de lock contre une dette qui ne rougit plus jamais, et
qui survivrait à l'avis suivant sur le même crate. Si l'implémentation est
tentée par ce chemin, c'est le signal qu'elle a rencontré un obstacle non prévu
par ce plan : elle s'arrête et remonte (voir *Fenêtres d'échec*).

**Pas de `cargo update` nu, ni de `cargo update --workspace`.** Les deux
re-résolvent les 575 paquets du lock et violent R3 par construction.
`release-pr.yml:96` lance bien `cargo update --workspace`, mais sur la PR de
release, dont c'est le métier ; ce n'est pas le geste de ce vol.

**Pas de nouveau détecteur de rétrogradation.** Un test Rust qui parserait
`Cargo.lock` pour asserter `ctutils >= 0.4.4` dupliquerait un instrument qui
existe, qui est déjà armé, qui est déjà requis, et qui vient de **prouver qu'il
mord** : `cargo audit` sur le job Security, run 39938791465. Il deviendrait de
surcroît stale au prochain avis sur un autre crate, en épinglant un seuil que
l'advisory-db porte mieux. C'est l'allowlist sans objet que mika#2309 condamne,
transposée en test.

**Gate Fire-Disposition (mika#2306) : N/A.** Ce vol livre deux lignes de
`Cargo.lock` et aucun livrable dont le chemin de succès soit « aucune violation
trouvée ». Le détecteur qui gouverne ce ticket — `cargo audit` — est antérieur
au vol et n'est pas modifié par lui. La section n'est donc pas requise, et
l'inventer serait affirmer une disposition sur un détecteur que personne ne
livre.

## Approche

Un seul geste, puis sa vérification.

### D1 — Re-résoudre `ctutils` dans le lock

```
cargo update -p ctutils
```

Sans `--precise`. Deux raisons :

1. Le ticket nomme exactement cette commande.
2. `--precise 0.4.4` prendrait le **plancher** de l'avis, ce qui laisse le vol à
   une patch du prochain. La résolution nue prend la plus haute compatible avec
   `^0.4` — aujourd'hui 0.4.5 — et le caret interdit structurellement de
   déborder sur une 0.5.x, donc il n'y a pas de risque de saut majeur à borner.

La version effectivement obtenue est **lue dans le diff**, pas supposée, et
consignée dans le corps de PR.

### D2 — Lire le diff du lock et le qualifier

```
git diff --stat Cargo.lock
git diff Cargo.lock
```

Trois lectures, dans cet ordre :

| ce que montre le diff | lecture | suite |
|---|---|---|
| `ctutils` seul, `version` + `checksum` | R3 tenu nominalement | corps de PR : la version obtenue |
| `ctutils` + `cmov` | R3 par sa clause de dérogation | corps de PR : les deux entrées, et que `cmov` a bougé parce que `ctutils` 0.4.x resserre sa contrainte sur sa seule dépendance |
| autre chose que `ctutils`/`cmov` | **hors prémisse du plan** | halte, et remontée à l'opérateur avec le diff |

La troisième ligne n'est pas une précaution de style : le plan a prouvé qu'il
n'existe qu'un dépendant et qu'une dépendance. Un diff plus large veut dire que
le graphe n'est pas celui qui a été lu, et c'est à l'opérateur de trancher, pas
à l'implémentation.

### D3 — Vérifier

```
cargo build
cargo test --workspace
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
scripts/verify-pipeline.sh origin/main
```

`cargo audit` et `cargo deny check` en local sont **best-effort** : tous deux
lisent l'advisory-db sous `~/.cargo/advisory-db` (`deny.toml:5`), dont la
fraîcheur dépend d'un `git fetch` que le bac à sable de dispatch peut refuser.
Un `cargo audit` local vert sur une db antérieure au 2026-10-23 ne prouve
**rien** — il n'a jamais vu l'avis. La preuve de R5 est le job Security sur la
tête de la PR, et elle seule.

### Ordre de commit

Un commit.

```
fix(deps): monter ctutils ≥ 0.4.4 (RUSTSEC-2026-0385) (mika#2896)
```

Le lock est le seul fichier touché, avec ce plan.

## Fenêtres d'échec

| fenêtre | signe | conduite |
|---|---|---|
| le réseau du bac à sable refuse crates.io | `cargo update` échoue sur le fetch de l'index | halte et remontée — rien dans ce vol ne se fait hors ligne |
| le diff touche autre chose que `ctutils`/`cmov` | D2 ligne 3 | halte et remontée avec le diff |
| `cargo deny check` rougit après la montée | licence de 0.4.5 hors de `deny.toml:22-35`, ou nouvelle dep bannie | halte et remontée — une licence à ajouter à l'allowlist est une décision, pas un ajustement |
| le job Security reste rouge sur un **autre** avis | `cargo audit` nomme un ID ≠ RUSTSEC-2026-0385 | ce vol n'y répond pas : le consigner dans le corps de PR et ouvrir un ticket de suivi |

Dans les quatre cas, la conduite interdite est la même : ajouter une entrée
`[[advisories.ignore]]` pour faire verdir la porte.

## Verification Contract

| critère | preuve |
|---|---|
| R1 | `grep -A 2 'name = "ctutils"' Cargo.lock` montre une `version` ≥ 0.4.4 |
| R2 | `git diff --name-only origin/main` ne liste aucun `Cargo.toml` |
| R3 | `git diff --stat Cargo.lock` ; le corps de PR nomme chaque paquet changé et sa raison |
| R4 | `git diff --name-only origin/main` ne liste pas `deny.toml` |
| R5 | job **Security** vert sur la tête de la PR (`cargo audit` + `cargo deny check`) |
| R6 | les trois commandes de D3 vertes, plus `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` |

**Rouge-avant.** Il est déjà au dossier et n'a pas à être reproduit : le run
39938791465 de mika#2894 porte le `cargo audit` rouge sur `ctutils` 0.4.2, avant
toute modification. La preuve d'après est le même job, vert, sur la tête de
cette PR.

**Déterminisme de la résolution.** `rust-toolchain.toml` épingle le canal 1.93
et `Cargo.lock` est en `version = 4` : le lock régénéré sur CI est celui qui est
commité. Aucun job de `ci.yml` ne lance `cargo build --locked` — l'unique
`--locked` du pipeline (`ci.yml:1080`) porte sur l'installation de
`cargo-audit`/`cargo-deny`, pas sur le workspace — donc aucune porte ne rougira
sur une dérive de lock, mais aucune ne rattrapera non plus un lock oublié : le
commit du lock est le livrable, pas un effet de bord.

## Definition of Done

- [ ] `Cargo.lock` résout `ctutils` à une version ≥ 0.4.4.
- [ ] Aucun `Cargo.toml` modifié, aucun `deny.toml` modifié.
- [ ] Le diff du lock est limité à `ctutils` — ou à `ctutils` + `cmov`, auquel
      cas le corps de PR nomme `cmov` et sa raison.
- [ ] La version obtenue est consignée dans le corps de PR, lue dans le diff.
- [ ] `cargo build`, `cargo test --workspace`,
      `cargo clippy --all-targets --all-features -- -D warnings`,
      `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Job **Security** vert sur la tête de la PR, et ne portant plus
      RUSTSEC-2026-0385.

## Acceptance criteria

- **AC1.** `Cargo.lock` résout `ctutils` à une version ≥ 0.4.4, et c'est le seul
  paquet qui change dans le lock. Si la résolution en entraîne d'autres, la PR
  les liste avec leur raison.
- **AC2.** Le job **Security** est vert sur la tête de la PR.
- **AC3.** `cargo build`, `cargo test --workspace` et
  `cargo clippy --all-targets --all-features -- -D warnings` sont verts.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `Cargo.lock` — entrée `ctutils` (`version` + `checksum`) | 2 |

Total estimé : 2 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 2 ≤ 279 et 2 ≤ 1000 : les deux
bornes passent avec une marge qui ne se discute pas. Le volume n'est pas le
risque de ce vol — le risque est au-dehors, dans la disponibilité de crates.io
et dans la fraîcheur de l'advisory-db sur le runner, et les deux sont traités
par *Fenêtres d'échec*. Aucun découpage n'a de sens : `ctutils` ne monte pas en
deux fois, et un lock à demi re-résolu ne serait pas mergeable.

Les deux lignes sont **générées par `cargo`**, jamais écrites à la main : le
`checksum` du lock est celui que le registre sert, et une édition manuelle le
ferait diverger. C'est pourquoi le livrable est la commande de D1, pas un patch.
