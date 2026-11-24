# chore(3131) — le vol d'attestation est livré ; la cascade reste une décision d'opérateur

**Ticket :** senara-solutions/mika#3131
**Vol 1 (livré) :** PR senara-solutions/mika#3161, mergée le 2026-11-24 à 07:05 UTC — `3585e7be`
**Plan du vol 1 :** `docs/plans/2026-11-24-001-chore-3131-rusqlite-0-41-cascade-plan.md`
**Exception visée :** RUSTSEC-2026-0459 (`deny.toml:21`, `.cargo/audit.toml:27`)

---

## Ce plan ne livre pas de code, et c'est son contenu

Ce ticket a déjà été groomé (first-pass READY → second-pass GROOMED) et son vol
courant a été livré et mergé **ce matin même**. Le plan du vol 1 a mesuré que la
montée demandée est une cascade à trois étages, a livré la part que le bac à
sable autorisait (les deux motifs d'exception, faux par omission, et le document
de routage), et a **réservé la cascade elle-même à l'opérateur**, avec sa
procédure en trois vols et une recommandation de routage explicite :

> **Routage :** retirer `dispatch:loop`, poser `pipeline-exempt`, laisser #3131
> ouvert.

Cette recommandation n'a pas été appliquée. Le ticket porte encore
`dispatch:loop` et `p3-nice-to-have`, et aucun `pipeline-exempt`. C'est
l'unique cause de ce re-dispatch : la boucle a trouvé un ticket ouvert, étiqueté
pour elle, sans plan non consommé — et a dépensé un pilote de grooming pour
re-découvrir une halte déjà établie et déjà acceptée par deux passes
d'architecte.

Sous mika#3122 règle 2, **une halte réservée ne se lève jamais par un amendement
du plan**. Ce plan ne rouvre donc pas la cascade au dispatch. Il fait les trois
gestes que la règle prescrit : il nomme la décision, il chiffre les options avec
leur coût, et il n'en pratique aucune. Son verdict attendu est `ESCALATE`, qui
est une sortie **nominale** (règle 3), et non un défaut de préparation.

---

## État mesuré, 2026-11-24, dans le worktree de ce dispatch

### E1 — le vol 1 est bien dans la branche, et rien de plus

```
$ git merge-base --is-ancestor 3585e7be HEAD && echo dans-la-branche
dans-la-branche
```

Les livrables du vol 1 sont en place :

| livrable du vol 1 | état |
|---|---|
| `deny.toml:21` — entrée RUSTSEC-2026-0459 | présente, motif réécrit |
| `.cargo/audit.toml:27` — `"RUSTSEC-2026-0459"` | présente, commentaire réécrit |
| `docs/solutions/dependency-maintenance/2026-11-24-une-montee-declaree-dans-cargo-toml-nest-pas-plus-dispatchable.md` | présent |

### E2 — la cascade entière reste à faire, aux trois étages

| site | valeur en `HEAD` | valeur visée |
|---|---|---|
| `Cargo.toml:30` | `rusqlite = { version = "0.32", features = ["bundled"] }` | `"0.41"` |
| `Cargo.lock` — `libsqlite3-sys` | `0.30.1` | `>= 0.39.0` (remède RUSTSEC-2026-0459) |
| `Cargo.toml:128` | `sqlx = { version = "0.8", … }` | `"0.9"` |
| `rust-toolchain.toml:2` | `channel = "1.93"` | `1.94` |
| `Cargo.toml:14` | `rust-version = "1.91"` | `1.94` |

Aucun des cinq n'a bougé. Le vol 1 ne les a pas touchés — c'était son contrat
(`git diff --stat origin/main -- Cargo.toml Cargo.lock` → vide, V5 du vol 1).

### E3 — la surface d'adaptation, re-mesurée

```
$ grep -rn "rusqlite" crates/ --include="*.rs" | wc -l
484
$ grep -rln "rusqlite" crates/ --include="*.rs" | wc -l
50
$ grep -rn "sqlx" crates/ --include="*.rs" | wc -l
149
```

484 sites rusqlite sur 50 fichiers, et 149 sites sqlx sur 13 fichiers — tous les
13 dans `crates/mika-gateway/`. Les deux chiffres du vol 1 (M9) tiennent, et le
second est neuf : il borne le vol B.

### E4 — le refus de la politique de permission tient

```
$ cargo update --help
no matching policy rule -- denied by default (production posture; widen rules to allow new tool footprints)
```

Identique à la mesure M1 du vol 1. C'est un **arbitrage de l'opérateur
matérialisé en code** (mika#3122 règle 1) : il n'est pas contourné ici, et aucun
repli obtenant le même effet par un autre outil n'est inscrit à ce plan.

### E5 — fait neuf : le fondement M6 était trop fort, et la conclusion tient quand même

Le plan du vol 1 écrit, en M6, que `~/.cargo/registry/src/` est « monté en
lecture seule, donc aucune caisse nouvelle ne se compile », sur la foi de cette
sortie du 2026-11-24 :

```
error: failed to download `etcetera v0.11.0`
Caused by: failed to create directory `…/registry/src/…/etcetera-0.11.0`
Caused by: Read-only file system (os error 30)
```

Cette session mesure un contre-exemple daté :

```
$ ls -ld …/registry/src/index.crates.io-…/sqlx-core-0.9.1
drwxr-x--- 1 samidarko samidarko 184 Nov 23 08:35 …/sqlx-core-0.9.1
$ ls -a …/sqlx-core-0.9.1/
.  ..  .cargo-ok  .cargo_vcs_info.json  Cargo.lock  Cargo.toml  Cargo.toml.orig  LICENSE-APACHE  LICENSE-MIT  src
```

`sqlx-core 0.9.1` — une caisse **nouvelle**, absente du graphe actuel — est
dépaquetée sous `src/`, et la sentinelle `.cargo-ok` que cargo écrit en fin
d'extraction est présente : **l'extraction a abouti**, le 2026-11-23 à 08:35.
Le `src/` du bac à sable a donc été inscriptible la veille de la mesure M6.

Lecture correcte, et c'est une correction de **fondement**, pas de conclusion :

- M6 décrit l'état du bac à sable **d'une session**, pas une propriété
  invariante de l'environnement. Un futur dispatch peut très bien trouver
  `src/` inscriptible et croire la halte levée.
- Ce qui reste invariant est **E4**, le refus de `cargo update`. Il ne dépend
  d'aucun montage : il est écrit dans la politique de permission. Sans cette
  commande, un pilote ne peut pas muter le lock par le geste canonique, et
  mika#3122 règle 1 interdit d'en chercher un équivalent par un autre outil.
- Donc la classe **opérateur** du ticket tient, mais **par E4 seul**. Toute
  relecture future qui s'appuierait sur M6 pour la justifier s'appuierait sur un
  fait daté, pas sur une règle.

Cette correction est consignée parce qu'elle sera lue : le document de solution
du vol 1 porte la signature `failed to create directory …/registry/src/…` comme
marque du blocage. Cette marque est réelle, mais elle est **intermittente**, et
un document qui la donne pour permanente enseigne un préflight qui passera au
vert un jour sur deux.

---

## La décision réservée à l'opérateur

Une seule décision, et elle n'est pas technique : **quelle suite donner à #3131,
maintenant que sa part attestable est livrée et que sa part restante est hors du
bac à sable.** Les trois options, avec leur coût.

### Option 1 — appliquer le routage que le vol 1 recommandait (coût : deux labels)

Retirer `dispatch:loop`, poser `pipeline-exempt`, laisser le ticket ouvert. La
cascade attend un créneau opérateur ; la procédure est écrite au § « Le geste
opérateur » du plan du vol 1 et n'a pas besoin d'être re-mesurée.

**Coût :** deux `gh issue edit`. **Effet :** la boucle cesse de re-dispatcher.
**Ce que ça laisse ouvert :** l'avis RUSTSEC-2026-0459 reste ignoré, par une
exception dont le motif dit désormais le coût réel (vol 1).

C'est l'option que le vol 1 a recommandée, que deux passes d'architecte ont
acceptée, et qu'aucune mesure de cette session ne contredit. Ce plan ne la
pratique pas pour autant : appliquer un label de routage sur le ticket est le
geste de l'opérateur, et le pratiquer depuis un pilote de grooming serait lever
une halte par un amendement du plan.

### Option 2 — ouvrir les trois vols comme tickets opérateur et fermer #3131 (coût : trois tickets)

`#3131` devient un parent ; les vols A (toolchain), B (sqlx), C (rusqlite +
extinction) deviennent trois tickets `pipeline-exempt`, chacun mergeable seul,
dans l'ordre forcé A → B → C.

**Coût mesuré, par vol :**

| vol | surface | ce qui le borne |
|---|---|---|
| A — toolchain 1.93 → 1.94 | 2 lignes (`rust-toolchain.toml:2`, `Cargo.toml:14`) | les lints qu'une montée de toolchain réveille, sur tout le workspace — volume non bornable à l'avance. **Ne pas** toucher le pin `cargo-deny@0.21.0` (`ci.yml:1179`) : 1.94 ne débloque pas `0.22.0`, qui exige 1.95 |
| B — sqlx 0.8 → 0.9 | 1 ligne de manifeste, 149 sites sur 13 fichiers, tous dans `mika-gateway` | montée majeure ; les six features demandées (`runtime-tokio`, `tls-rustls-aws-lc-rs`, `postgres`, `uuid`, `chrono`, `json`) doivent être re-vérifiées sous 0.9 — le résolveur du vol 1 a signalé que `tls-rustls-aws-lc-rs` n'existe pas sous les 0.8.x anciennes, donc ce nom a déjà bougé une fois |
| C — rusqlite 0.32 → 0.41 + extinction | 1 ligne de manifeste, **484 sites sur 50 fichiers**, dont 129 dans `crates/mika-agent/src/db.rs` seul | neuf versions majeures d'écart ; le changelog 0.33 → 0.41 est à lire au moment du geste. Demande une **vérification positive** de `crates/mika-agent/src/db.rs:25-31` (le `transmute` à travers `rusqlite::ffi`), pas un build vert |

Le vol C dépasse à lui seul les deux bornes de mika#2636 et mika#2693 d'un
facteur large, et c'est une raison de plus pour laquelle il n'est pas
dispatchable même si la policy s'ouvrait.

**Coût :** trois tickets à cadrer, chacun avec sa propre estimation de taille.
**Effet :** la cascade devient suivable pièce par pièce et le parking #2983
cesse d'être la seule ligne de retour.

### Option 3 — élargir la politique de permission pour cette classe (coût : un arbitrage de posture)

Il faudrait trois choses ensemble : `cargo update` autorisé, `~/.cargo/registry/src/`
inscriptible **de façon garantie** (E5 montre qu'il l'est par intermittence,
ce qui est pire qu'un refus franc), et un clone joignable de
`rustsec/advisory-db` (`deny.toml:5-6`).

**Coût :** c'est accorder à un pilote le droit de muter le graphe de dépendances
du dépôt. Le `production posture` du refus E4 se lit comme une décision déjà
prise dans l'autre sens. **Ce plan n'inscrit aucun geste de cette option** :
mika#3122 règle 1 fait du refus un arbitrage opérateur, et un repli par un autre
outil est explicitement interdit.

### Ce que ce plan ne fait pas

Il ne choisit pas entre les trois. Il ne pose ni ne retire de label. Il
n'ouvre pas de ticket. Il n'édite ni `Cargo.toml`, ni `Cargo.lock`, ni
`rust-toolchain.toml`, ni les deux fichiers d'exception. Il ne contourne pas E4.

---

## Requirements

### R1 — la correction du fondement E5 est consignée là où le préflight se lit

Le document de solution du vol 1 enseigne la signature
`failed to create directory …/registry/src/…` + `Read-only file system (os error 30)`
comme marque du blocage. E5 établit que cette marque est **intermittente** : la
veille, la même opération réussissait. Un préflight qui la tient pour permanente
passera au vert un jour sur deux et re-dispatchera la classe.

Le document doit donc porter deux faits de plus : que le `src/` en lecture seule
est un état de session et non une propriété de l'environnement, et que le fait
invariant de cette classe est le refus de `cargo update` par la politique de
permission, qui ne dépend d'aucun montage.

**C'est le seul livrable candidat de ce vol, et il est documentaire** (donc hors
compte de lignes). Il ne dépend d'aucune des trois options et ne les préempte
pas : quelle que soit la suite donnée au ticket, la correction reste vraie.

### R2 — le ticket ne se ferme pas, et la cascade ne se dispatche pas

L'avis RUSTSEC-2026-0459 reste actif et les deux exceptions restent en place.
Aucune PR issue de ce plan ne porte `Closes #3131`, et aucune n'édite les cinq
sites de E2.

### R3 — la récurrence du re-dispatch est nommée, pas corrigée en silence

Ce dispatch est le second pilote dépensé sur #3131 pour le même constat. La
cause est un label, pas un défaut de plan. Une garde structurelle est
imaginable — un lint qui refuse `dispatch:loop` sur un ticket dont le dernier
plan mergé déclare une classe opérateur — mais c'est un **détecteur**, donc un
périmètre à cadrer pour lui-même, avec sa propre `## Fire-Disposition` et sa
propre estimation. Il n'est pas inscrit à ce vol : le nommer est le livrable,
le construire serait trancher l'option 1 par un détour technique.

---

## Périmètre

### Dans le vol courant

| fichier | geste |
|---|---|
| `docs/solutions/dependency-maintenance/2026-11-24-une-montee-declaree-dans-cargo-toml-nest-pas-plus-dispatchable.md` | ajout des deux faits de E5 : le `src/` ro est un état de session, l'invariant est le refus `cargo update` |

Un seul fichier, et il est sous `docs/`. **Le vol courant ne livre aucune ligne
de code.**

### Hors du vol courant — explicitement

- **Les trois étages de la cascade** (E2). Classe opérateur, options 1–3
  ci-dessus, procédure détaillée au § « Le geste opérateur » du plan du vol 1.
- **Le retrait des deux exceptions RUSTSEC-2026-0459.** Dernier geste du vol C :
  le retirer avant que `libsqlite3-sys >= 0.39.0` soit dans le lock fait rougir
  le job `Security` sur toute tête.
- **Le choix de routage du ticket** (labels, ouverture de tickets enfants).
  Geste opérateur — c'est la décision que ce plan nomme sans la trancher.
- **L'élargissement de la politique de permission** (option 3). Arbitrage de
  posture, mika#3122 règle 1.
- **La garde contre le re-dispatch** (R3). Détecteur, à cadrer pour lui-même.

`Tracked in: senara-solutions/mika#3131` — le ticket **reste ouvert**. Le parking
senara-solutions/mika#2983 reste la ligne de retour si la cascade est repoussée.

---

## Fire-Disposition

**Gate N/A — ce vol ne livre aucun détecteur.**

Déclaré explicitement pour que l'absence ne se lise pas comme un oubli. Le vol
courant touche **un seul fichier**, sous `docs/` : aucun test, aucune assertion,
aucune règle de lint, aucune garde CI, aucun validateur de schéma, aucune garde
EndTurn, et aucun livrable dont le chemin de succès serait « aucune violation
trouvée ».

La garde imaginable du R3 **serait** un détecteur, et c'est précisément pourquoi
elle est hors périmètre : elle demanderait sa propre `## Fire-Disposition`, et
son option (a) ne serait pas évidente — le ticket #3131 lui-même serait la
première violation existante à exempter, ce qui est un cadrage, pas une ligne de
code.

---

## Verification Contract

| # | ce qui est vérifié | comment |
|---|---|---|
| V1 | le document porte les deux faits de E5 | `grep -n 'cargo update\|session\|intermittent' docs/solutions/dependency-maintenance/2026-11-24-une-montee-declaree-dans-cargo-toml-nest-pas-plus-dispatchable.md` → l'invariant (`cargo update` refusé) et le caractère non permanent du `src/` ro sont tous deux nommés |
| V2 | le frontmatter du document est intact | `head -20` → `module`, `tags`, `problem_type`, `resolution_type` toujours présents et inchangés |
| V3 | aucune mutation du graphe de dépendances | `git diff --stat origin/main -- Cargo.toml Cargo.lock rust-toolchain.toml` → **vide** |
| V4 | aucune mutation des exceptions | `git diff --stat origin/main -- deny.toml .cargo/audit.toml` → **vide** ; les deux entrées RUSTSEC-2026-0459 du vol 1 restent telles quelles |
| V5 | le vol ne livre aucune ligne de code | `git diff --stat origin/main` → un seul fichier, sous `docs/` |
| V6 | la chaîne complète | `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `bash scripts/verify-pipeline.sh origin/main` |

V3 et V4 ne sont pas des formalités : ce sont les deux assertions qui attestent
que le plan **n'a pas pratiqué** l'une des options qu'il chiffre.

---

## Definition of Done

- Le document de solution du 2026-11-24 porte la correction de E5 : le `src/` en
  lecture seule est un état de session, l'invariant de la classe est le refus de
  `cargo update` par la politique de permission.
- `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `deny.toml` et
  `.cargo/audit.toml` sont **identiques** à `origin/main`.
- V1–V6 passent.
- La PR porte `Tracked in: senara-solutions/mika#3131` et **pas** `Closes #3131`.
- Le corps de PR nomme les trois options de routage et dit laquelle le vol 1
  recommandait, **sans en pratiquer aucune**.

## Acceptance criteria

Le ticket #3131 n'a pas de section `## Acceptance criteria` dans son corps — son
corps est le callout de grooming du vol 1 suivi de deux lignes de source. Les
critères ci-dessous sont dérivés des Requirements et du Verification Contract, et
ne portent que sur le **vol courant**.

- **AC1** — le document
  `docs/solutions/dependency-maintenance/2026-11-24-une-montee-declaree-dans-cargo-toml-nest-pas-plus-dispatchable.md`
  nomme (a) que la signature `Read-only file system (os error 30)` sous
  `registry/src/` est un état de session, avec le contre-exemple daté de E5, et
  (b) que l'invariant de routage de cette classe est le refus de `cargo update`
  par la politique de permission.
- **AC2** — `git diff origin/main -- Cargo.toml Cargo.lock rust-toolchain.toml deny.toml .cargo/audit.toml`
  est **vide** : aucune option de routage n'a été pratiquée.
- **AC3** — `git diff --stat origin/main` ne liste qu'un fichier, et il est sous
  `docs/` : le vol ne livre aucune ligne de code.
- **AC4** — le plan contient une section nommant la décision réservée à
  l'opérateur, avec les trois options chiffrées (deux labels / trois tickets /
  élargissement de posture) et le coût mesuré de chacun des trois vols de la
  cascade.
- **AC5** — le ticket #3131 reste OPEN, et aucune PR issue de ce plan ne contient
  `Closes #3131`.
- **AC6** — aucun geste de contournement du refus E4 n'apparaît dans le plan ni
  dans la PR : pas de programme jetable, pas de réécriture du geste par un autre
  outil, pas de variante du même accès.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| (aucun) | 0 |

Total estimé : 0 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

Le vol tient sous les deux bornes parce qu'il ne livre **rien** hors `docs/`, et
ce n'est pas une sous-estimation : c'est la conséquence directe du § « La
décision réservée à l'opérateur ». La seule matière volumineuse du ticket — les
484 sites rusqlite et les 149 sites sqlx (E3), plus le volume non bornable des
lints qu'une montée de toolchain réveille — est hors du bac à sable par E4, et
n'a donc pas de vol courant sous lequel atterrir.

Le découpage n'est pas motivé par la taille mais par la **capacité** : les vols
A, B et C de l'option 2 ne sont pas des phases de celui-ci. Ils sont hors de la
file de dispatch, et chacun demandera sa propre estimation quand il sera cadré —
le vol C dépasse à lui seul les deux bornes d'un facteur large.

Un total de 0 ligne est inhabituel et doit se lire comme tel : **ce plan est un
plan de halte**, et mika#3122 règle 3 en fait une sortie nominale. Son verdict
attendu est `ESCALATE`.

---

## Ordre de commit

1. `docs(solutions): le registre en lecture seule est un etat de session, pas l'invariant de la classe (mika#3131)`
   — le document du 2026-11-24 reçoit les deux faits de E5.
2. `docs(plans): groom mika#3131` — ce fichier.

Le plancher du vol est le commit 1 seul : il est vrai indépendamment de l'option
de routage que l'opérateur retiendra, et il corrige un préflight que la prochaine
session de grooming lira.
