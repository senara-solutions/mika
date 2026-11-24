---
title: "Une montée déclarée dans `Cargo.toml` n'est pas plus dispatchable — le registre `src/` est en lecture seule"
date: 2026-11-24
category: dependency-maintenance
module: Cargo.toml
component: development_workflow
problem_type: best-practice
severity: high
symptoms:
  - "`cargo metadata --format-version 1` affiche `Updating crates.io index`, résout le graphe, écrit un `Cargo.lock` neuf — puis `error: failed to download`"
  - "`Caused by: failed to create directory …/.cargo/registry/src/index.crates.io-*/<caisse>-<version>`"
  - "`Caused by: Read-only file system (os error 30)`"
  - "`warning: failed to write cache, path: …/registry/index/…/.cache/an/yh/anyhow, error: Read-only file system (os error 30)` pendant la résolution"
  - "`cargo update --help` refusé (`no matching policy rule`), comme pour la classe « bump de lock »"
  - "`cargo deny check advisories` refusé par la même règle — la vérification d'une exception n'est pas exécutable dans le bac à sable"
root_cause: ticket_class_outside_sandbox_capability
resolution_type: routing
tags: [mika-3131, mika-2861, mika-2983, dependency, security, cargo-update, cargo-metadata, rusqlite, sqlx, links, rustsec, dispatch, bac-a-sable, preflight, lecture-seule]
---

# Une montée déclarée dans `Cargo.toml` n'est pas plus dispatchable — le registre `src/` est en lecture seule

## Les deux classes sont une seule classe de routage

[Le document du 2026-10-21](2026-10-21-un-bump-de-lock-rustsec-nest-pas-dispatchable.md)
couvre une classe précise : le **bump d'un nœud transitif dans `Cargo.lock`**,
bloqué parce que `cargo update` est refusé et qu'éditer un `checksum` à la main
est un sabotage silencieux.

Un groom qui lit ce document pour un ticket d'un autre genre peut légitimement
conclure qu'il ne s'applique pas. C'est ce qui s'est passé sur
**senara-solutions/mika#3131** (monter `rusqlite = "0.32"` à `"0.41"` en
`Cargo.toml:30`, pour éteindre RUSTSEC-2026-0459) : ici le geste demandé est une
**montée déclarée dans `Cargo.toml`**, l'édition du manifeste est permise,
aucun `checksum` n'est à calculer à la main, et `cargo update` n'est même pas
nécessaire — `cargo metadata` suffit à régénérer le lock. Le raisonnement du
document de 2026-10-21 semble donc inapplicable.

Il l'est pourtant, par un autre chemin. **Les deux classes butent sur la même
porte :**

| classe | ce qui est permis | ce qui bloque |
|---|---|---|
| bump de nœud transitif dans `Cargo.lock` (2026-10-21) | rien du geste | `cargo update` refusé par la policy |
| montée déclarée dans `Cargo.toml` (ici) | **l'édition du manifeste, et la résolution** | le **dépaquetage** : `~/.cargo/registry/src/` est en lecture seule |

Dans le premier cas l'**écriture** est refusée. Dans le second l'écriture passe,
et c'est la **vérification** qui est hors d'atteinte : rien de ce qui est écrit
ne peut être compilé une seule fois, parce qu'aucune caisse absente du disque ne
peut être matérialisée. Un pilote dispatché sur mika#3131 aurait adapté 484
sites d'API répartis sur 50 fichiers sans jamais lancer un `cargo check`.

> Une montée de dépendance n'est dispatchable que si le bac à sable peut
> **compiler le graphe d'arrivée**. Que le geste d'édition soit permis ne dit
> rien de cela — et c'est précisément ce qui rend cette classe trompeuse pour un
> groom qui a lu le document de 2026-10-21 et conclu « ceci n'est pas mon cas ».

## Le blocage est trompeur, et voilà sa signature

C'est le fait neuf, celui que le document de 2026-10-21 ne pouvait pas porter :
dans ce bac à sable, **le réseau fonctionne**. `cargo` atteint crates.io, résout
l'index, et la résolution **aboutit**. Une montée qui échoue affiche donc, dans
l'ordre :

```
    Updating crates.io index
warning: failed to write cache, path: …/registry/index/…/.cache/an/yh/anyhow, error: Read-only file system (os error 30)
    Updating sqlx-core v0.8.6 -> v0.9.1 (requires Rust 1.94.0)
    …
error: failed to download `etcetera v0.11.0`
Caused by:
  failed to create directory `/home/<user>/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/etcetera-0.11.0`
Caused by:
  Read-only file system (os error 30)
```

Les trois premières lignes ressemblent à un succès, et un `Cargo.lock` neuf a
**réellement été écrit sur le disque** avant l'échec. Un pilote qui lit en
diagonale peut croire avoir réussi, commiter un lock qu'il n'a jamais compilé,
et ouvrir une PR dont le seul vert sera celui de la CI de l'opérateur — ou dont
le rouge sera inexplicable depuis le bac à sable.

**La signature à reconnaître, et à ne jamais lire comme un problème de réseau :**

```
failed to create directory …/registry/src/…
Read-only file system (os error 30)
```

`registry/index/` n'est pas inscriptible non plus (le `warning` ci-dessus), mais
celui-là est bénin : `cargo` se rabat sur une résolution sans cache. C'est
`registry/src/` qui est fatal, parce que c'est là que vivent les **sources**.
Aucune caisse dont le `.crate` n'est pas déjà sous `~/.cargo/registry/cache/`
n'existera jamais sous `src/`, donc elle ne compilera jamais.

Corollaire utile : la résolution, elle, est une **mesure valide**. Sur mika#3131
c'est elle qui a produit les trois faits que le grooming a pu consigner — le
conflit `links = "sqlite3"` entre `rusqlite 0.41` et `sqlx 0.8.6`, la montée
`sqlx 0.9` qu'il force, et le `requires Rust 1.94.0` que celle-ci force à son
tour. Le bac à sable permet donc d'**attester le graphe d'arrivée** tout en
interdisant de le compiler. C'est la moitié du ticket qu'un vol dispatché peut
honnêtement livrer.

## Le préflight de cette classe-ci

`cargo update --help` (§ « Quoi faire » du document de 2026-10-21) reste le
premier geste, mais **il ne suffit plus**. Pour une montée déclarée dans
`Cargo.toml`, il peut passer ou échouer sans rien dire du vol : le geste
nominal n'est pas `cargo update`.

Le préflight est en deux temps, et le second est la tentative de résolution
elle-même :

1. **`cargo update --help`** — refusé ⇒ la classe « bump de lock » est
   opérateur, et on s'arrête. Autorisé ⇒ cela ne conclut rien pour une montée
   déclarée ; passer au temps 2.

2. **Éditer la version dans `Cargo.toml`, puis `cargo metadata --format-version 1`**
   (sans `--locked`, qui refuserait de régénérer le lock). Trois issues, et
   seule la troisième autorise le dispatch :

   | sortie | lecture |
   |---|---|
   | le résolveur refuse (`failed to select a version`, conflit `links`, …) | la cible réelle n'est pas celle du ticket — re-cadrer avant tout dispatch |
   | résolution OK puis `failed to create directory …/registry/src/…` | **classe opérateur.** Le vol n'est pas vérifiable ici |
   | résolution OK et dépaquetage OK | les caisses sont sur le disque ; `cargo check` est possible, le vol est dispatchable |

   Puis **`git checkout -- Cargo.toml Cargo.lock`**, sans exception : le temps 2
   écrit un lock. Le laisser serait livrer la seule chose que ce vol ne doit
   jamais livrer.

Trois remarques qui valent pour cette classe :

- **La surface d'adaptation se mesure, elle ne s'estime pas.**
  `grep -rn "<caisse>" crates/ --include=*.rs` donne les sites, et
  `cargo tree -i <caisse-sys>` donne les arêtes à monter. Sur mika#3131 : 484
  sites, 50 fichiers, une seule arête. C'est cette mesure qui transforme un
  `p3-nice-to-have` en classe opérateur, et elle est entièrement faisable dans
  le bac à sable.
- **Un conflit `links` ne se contourne pas par les features.** La vérification
  se fait dans le **résolveur**, sur le graphe résolu, qui inclut les
  dépendances optionnelles non activées. `default-features = false` ne lève
  rien. Deux caisses déclarant le même `links` en versions différentes sont une
  montée conjointe, jamais une montée isolée.
- **La vérification de l'exception n'est pas exécutable non plus.**
  `cargo deny check advisories` tombe sous la même règle de policy que
  `cargo update`. Un plan qui porte « `cargo deny check` reste vert » comme
  critère d'acceptation décrit un geste de CI, pas un geste de pilote ; dans le
  bac à sable, le repli honnête est un parse TOML réel des deux fichiers
  (`deny.toml` et `.cargo/audit.toml`) qui atteste la forme et l'échappement,
  et rien de plus.

## Ce que cela change pour le routage

La règle du document de 2026-10-21 s'applique mot pour mot, et c'est bien son
extension qui est l'objet de celui-ci :

> Un refus rencontré au grooming est un fait de **routage**, pas une note de bas
> de page. Il ne se range pas en branche de halte à l'intérieur du plan : il
> retire le ticket de la file de dispatch.

Conduite pour un ticket de cette classe dont le temps 2 du préflight tombe sur
`Read-only file system` :

- Le vol dispatché livre la **moitié attestable** : les mesures du graphe
  d'arrivée, et la correction de tout motif d'exception que ces mesures rendent
  faux par omission. C'est ce que mika#3131 a livré.
- La cascade elle-même part en **geste opérateur**, avec sa procédure écrite dans
  le plan, découpée en vols chacun mergeable seul et dans l'ordre imposé par les
  préconditions de résolution.
- Le ticket **reste ouvert** : retirer `dispatch:loop`, poser `pipeline-exempt`.
  Le retrait de l'exception est le **dernier** geste de la cascade, pas un geste
  séparable — retirer l'exception avant que le remède soit dans le lock fait
  rougir le job `Security` sur toute tête.
- **Et l'élargissement de la policy est l'autre branche, qui s'énonce.** Rendre
  cette classe dispatchable demande `cargo update` autorisé,
  `~/.cargo/registry/src/` **inscriptible**, et un clone joignable de
  `rustsec/advisory-db`. C'est accorder à un pilote le droit de muter le graphe
  de dépendances du dépôt ; le `production posture` du refus se lit comme une
  décision déjà prise dans l'autre sens.
