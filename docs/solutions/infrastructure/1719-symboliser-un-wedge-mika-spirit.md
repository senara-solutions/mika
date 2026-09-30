---
module: mika-spirit, build, observability
tags: [wedge, deadlock, symbolication, diagnosability, gdb, dwarf, addr2line, ci-guard]
problem_type: observability-gap
issues: [1719, 1722, 1723, 1724, 2195]
date: 2026-09-30
---

# Symboliser un wedge de mika-spirit

**À lire à 3 h du matin, quand `mika-spirit` ne répond plus.** Ce document donne
la chaîne complète : capture → base PIE → offset → nom et `fichier:ligne`. Il ne
prévient aucun wedge ; il raccourcit l'enquête. La prévention est ailleurs et elle
est déjà en place (mika#1723, mika#1724).

## Pourquoi ce document existe

Le 2026-07-02, `mika-spirit` s'est figé 8 h 30 : 29 threads sur 30 parkés au même
frame, dashboard noir, dispatch autonome mort. Cinq récurrences en trois jours.

La cause était **un bug d'une ligne** — un `Ref` DashMap détenu par le scrutinee
d'un `match` à travers un `.insert()` de même shard, dans `handle_message`. Il a
fallu **quatre jours** pour la nommer, parce que le binaire était strippé :

- cinq captures gdb rendant `?? ()` sur chaque frame ;
- une base PIE recalculée à la main, par soustraction d'adresses ;
- trois hypothèses successivement falsifiées (`AgentState.skills`, le double-init
  du souscripteur tracing, `flush_failed_sends`) ;
- un tableau d'invariants lui-même révisé à n=4.

Un `addr2line` sur un binaire porteur de tables de lignes aurait rendu le site au
premier jour. **C'est ce que ce document rend possible.**

## Les deux moitiés de la chaîne, et qui a fermé chacune

| Moitié | Question | Fermée par |
|---|---|---|
| Amont | *Quelle est la base PIE ?* | **mika#1722** — `scripts/mika-spirit-wedge-capture.sh` snapshote `/proc/<pid>/maps` au même instant que la pile, donc la base est un **artefact de la capture** et non une valeur dérivée dans l'analyse |
| Aval | *Que vaut cet offset ?* | **mika#1719** (ce document) — le profil release conserve les tables de lignes |

Avant mika#1719, la corroboration d'un offset passait par un `addr2line` sur
*une reconstruction* — « necessarily indirect: both endpoints of the subtraction
are the thing being verified », comme le dit l'en-tête du script de capture.

## La procédure

### 1. Capturer — geste opérateur, sur l'hôte

```bash
scripts/mika-spirit-wedge-capture.sh <pid>
```

Émet dans `/tmp/spirit-wedge-<ts>/` : `<ts>-stacks.txt` (gdb `thread apply all
bt 30`), `<ts>-maps.txt` (**l'artefact de base PIE**), `<ts>-status.txt`.

Le script imprime la base PIE lui-même. Ne la recalculez pas :

```
PIE base (verified from maps.first_line): 55945c799000
```

**Deux pièges, tous deux mesurés en juillet.**

- **Yama est absent de cet hôte** (`/proc/sys/kernel/yama/ptrace_scope` n'existe
  pas), donc gdb attache un process de même uid **sans root**. En juillet seul
  root-Claude capturait, à tort.
- **`/run/mika-spirit.pid` ne désigne pas nécessairement le process à capturer.**
  À n=5, *trois PID orphelins wedgés* coexistaient avec un child sain que
  `supervise-daemon` venait de respawner, et c'est l'orphelin qui portait
  l'évidence. Vérifiez qui tient le port :
  `ss -ltnp | grep 8081`, et capturez l'orphelin, pas le vivant.

### 2. Calculer l'offset

```
offset = runtime_addr - pie_base
```

La pile donne `runtime_addr`, le maps donne `pie_base`. Rien d'autre n'entre dans
ce calcul — c'est l'invariant 1 de mika#1722.

### 3. Symboliser

```bash
addr2line -f -C -e ~/.local/bin/mika-spirit 0x<offset>
```

`-f` donne la fonction, `-C` démangle, `-e` nomme le binaire.

**Trois haltes, et la première est un contresens facile.**

- **Un nom mangelé (`_RNvNtCs…` ou `_ZN10mika_agent…`) n'est PAS un échec.** La
  chaîne offset → symbole → ligne **a fonctionné** ; c'est le démanglage v0 de
  Rust que le `-C` de binutils ne gère pas toujours. Complétez avec `rustfilt`.
  Lire un nom mangelé comme un échec conclut à l'inverse de la mesure.
- **`??:0` sur le binaire déployé** alors que le worktree symbolise : le binaire
  servi n'est pas celui qui a été bâti. **Ne retouchez pas le profil** —
  établissez le déploiement (`make install` a-t-il tourné ?) avant toute
  conclusion. Classe mika#2340.
- **binutils absent :** le repli est `llvm-addr2line` (composant rustup
  `llvm-tools`). L'outil change, la procédure ne change pas.

### 4. Contrôle négatif, dans la même session

```bash
addr2line -f -C -e ~/.local/bin/mika-spirit 0xdeadbeef00
```

Doit rendre `??` / `??:0`. **Une sonde a besoin de ses deux contrôles au même
moment :** sans celui-ci, « symbolisé » et « symbolisé n'importe comment » rendent
des octets identiques.

## Exemple travaillé, mesuré le 2026-09-30

Sur `target/release/mika-spirit` bâti par `make build` depuis le profil livré.
Le binaire déployé est **la copie** de celui-là — `make install` fait `cp` puis
`mv` — donc cette preuve porte sur le binaire qui sera servi.

**Contrôle positif :**

```
address:  0x888b20
function: _ZN10mika_agent11task_engine21ensure_recurring_task…{{closure}}…
location: crates/mika-agent/src/task_engine/mod.rs:59
```

**Contrôle négatif, même session :**

```
address:  0xdeadbeef00
function: ??
location: ??:0
```

**Et le site du wedge de juillet, dans les deux sens.** `handle_message` est à
`0x159d8a0` → `crates/mika-agent/src/server/handlers.rs:145`. L'inverse aussi :

```
0x15a1cc6 -> crates/mika-agent/src/server/handlers.rs:245
0x15a1e5a -> crates/mika-agent/src/server/handlers.rs:255
```

Ces deux lignes sont **exactement** le `handlers.rs:245-255` que quatre jours
d'enquête ont mis à nommer. Le mapping est donc *exact*, pas seulement *présent*.

> **Note de méthode.** Ces sorties ont été produites par un lecteur ELF/DWARF
> autonome en Python, parce que `nm`, `addr2line` et `readelf` sont refusés par la
> policy du bac à sable de dispatch (`no matching policy rule — denied by
> default`), alors que binutils **est** installé sur l'hôte. Sur l'hôte, employez
> `addr2line` : c'est le même résultat par l'outil standard. La sonde S1 de
> mika#1719 le rejoue sur une capture réelle.

## Ce que le binaire porte, et ce qu'il ne porte pas

Mesuré sur le profil livré :

| Section | Taille | Rôle |
|---|---|---|
| `.symtab` | 1,25 Mo | **la moitié « nom »** — ce que `nm` lit |
| `.debug_line` | 16,6 Mo | **la moitié « fichier:ligne »** — ce que `addr2line` lit |
| `.debug_info` | 4,8 Mo | résidu (le gros est dans le `.dwp`) |
| `.debug_str` | 0,4 Mo | idem |

**Le `.dwp` n'est pas requis pour cette procédure, et c'est une mesure et non une
prévision.** `split-debuginfo = "packed"` déporte les **types et variables**
(`.debug_info` 44,7 → 4,8 Mo ; `.debug_str` 20,3 → 0,4) et **conserve** `.symtab`
et `.debug_line` dans le binaire. Le binaire déployé est donc autosuffisant pour
lire un wedge, et `make install` ne copie pas le `.dwp` — délibérément.

`target/release/mika-spirit.dwp` (51,8 Mo) reste sur la machine de build. Vous en
aurez besoin **seulement** pour inspecter des types ou des locales sous gdb, pas
pour nommer un frame.

## Le réglage, et la garde qui le tient

```toml
[profile.release]
lto = true
codegen-units = 1
strip = "none"                # conserve ce que debug émet
debug = "line-tables-only"    # l'émet en premier lieu
split-debuginfo = "packed"    # repli de taille, pris sur mesure
```

**Les deux premières clauses sont UN réglage, et chacune a l'air de suffire
seule.** `strip = "none"` seul ne conserve rien — un profil release vaut
`debug = 0` par défaut, donc `addr2line` rend `??:0`. Et
`debug = "line-tables-only"` seul émet des tables que `strip` retire ensuite. Un
futur éditeur qui en retire une aura **raison** de croire que l'autre porte la
garantie.

C'est pourquoi `scripts/check-release-debuginfo.sh` (job CI
`release-debuginfo-lint`, cible `make check-release-debuginfo`) existe :
`strip = true` était **une ligne**, et son retour serait silencieux — aucun test
ne rougit, le binaire redevient muet, et on l'apprend au prochain wedge, c'est-à-dire
au moment le plus coûteux possible. Classe mika#2205 : *un réglage silencieusement
annulé se lit exactement comme un réglage en vigueur.*

**Quand cette garde tire, on répare le profil** (mika#2201). L'allowlist est
livrée vide, comparée dans les deux sens, et un profil qu'on ne veut pas rendre
diagnosticable est un profil dont il faut discuter.

**Si la taille d'image force un jour un changement**, le levier est
`split-debuginfo`, **jamais** le retour de `strip` — qui rouvrirait mika#1719 en
entier.

## Le coût, chiffré

| | Taille | Ratio |
|---|---|---|
| Référence (`strip = true`) | 36 262 568 o | 1,00× |
| `line-tables-only` seul | 152 373 632 o | **4,20×** — au-delà du seuil de 3× |
| `+ split-debuginfo = "packed"` | 96 758 664 o | **2,67×** |

Le repli a donc été **pris sur mesure**, pas par principe. Durée de build
inchangée à la minute (5 min 01 s dans les deux cas) : `line-tables-only` ajoute
l'émission des tables, et le temps dominant reste le LTO et le codegen.

## Ce que ceci ne ferme pas

- **Symboliser ne prévient aucun wedge.** La classe précise de juillet est morte
  (mika#1723 a corrigé le site, mika#1724 l'interdit structurellement via
  `significant_drop_in_scrutinee`), mais **la classe d'enquête** était restée
  ouverte : tout futur park process-wide repartait d'un binaire muet. Fermer
  mika#1719 ne doit pas se lire comme « la classe est morte ».
- **La détection reste hors-process.** Un watchdog in-process ne peut
  structurellement pas voir cette classe : `spawn_engine_wedge_watchdog` est un
  `tokio::spawn`, donc il attend un worker que le wedge a précisément consommés,
  et ses deux canaux de signal étaient eux-mêmes parkés à n=1 (le thread
  `tracing-appender` au frame partagé, les dix threads `mika-db` de même). *Un
  observateur qui meurt avec ce qu'il observe n'est pas un observateur.* La
  détection vit dans cm#126 (`host-probes.sh`, heartbeat inversé sur
  `localhost:8081/healthz`, cron */10).
- **Aucun redémarrage automatique.** RT#003 le refuse
  (`crates/mika-agent/src/task_engine/liveness.rs:155-157`) : « auto-recovery
  breaks the bounded-authority principle for process-restart operations. Operator
  decides remediation. »
- **Le déclenchement automatique de la capture** sur franchissement « healthz
  muet » vit dans `control-monitor` — ticket compagnon, alert-only, une fois par
  franchissement.

## Voisinage

- mika#1722 — la moitié amont (base PIE indépendante)
- mika#1723 / PR#1726 — le site fautif, corrigé
- mika#1724 — `significant_drop_in_scrutinee`, l'interdiction structurelle
- mika#2195 / PR#2210 — la double-écriture des logs
- cm#126 — la détection hors-process
