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

Cette session observe un contre-exemple :

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

#### Deux dates, et il faut les séparer (F1)

| | instant | ce qui s'est passé |
|---|---|---|
| le **fait** | 2026-11-23 08:35 | l'extraction de `sqlx-core 0.9.1` aboutit sous `registry/src/` |
| commit du vol 1 | 2026-11-24 05:09 UTC — `cc32f5e0` | M6 conclut « monté en lecture seule, donc aucune caisse nouvelle ne se compile » |
| merge du vol 1 | 2026-11-24 07:05 UTC — `3585e7be` | le document de solution part avec ce fondement |
| la **mesure** | 2026-11-24 ~20:08 UTC, cette session | le `ls -ld` ci-dessus observe le fait |

Le fait **préexistait** au vol 1 d'environ 21 h. Il n'est donc pas apparu entre le
commit et le merge, et il n'est pas apparu depuis : **ce plan ne revendique pas
une découverte, il revendique une mesure.** Le contre-exemple était sur le disque
quand M6 a été écrit, et M6 ne l'a pas regardé.

Pourquoi le vol 1 ne l'a pas inclus, nommé sans détour : **il n'a pas cherché de
contre-exemple.** M6 a généralisé une seule sortie d'erreur — un `failed to
create directory` sur une caisse — en propriété invariante du montage. Un `ls` sur
`registry/src/` l'aurait démenti en une commande. C'est une inférence trop large,
pas un fait ignoré, et c'est exactement le défaut que R1 corrige : un document qui
enseigne cette signature comme marque permanente reproduit l'inférence pour la
prochaine session.

Ce que cela change pour la crédibilité de E5 : **rien**, parce que la crédibilité
d'une mesure ne tient pas à sa nouveauté mais à sa vérifiabilité. La commande est
re-jouable à l'identique par l'architecte de seconde passe et par l'opérateur. Le
fait que le vol 1 **aurait pu** la contenir est une part du constat de E5, pas une
objection contre lui.

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

### E6 — fait neuf : les deux consommateurs d'`advisory-db` sont refusés par la même porte

Mesuré dans cette session, pour chiffrer honnêtement l'option 3 (F2) :

```
$ cargo deny --version
no matching policy rule -- denied by default (production posture; widen rules to allow new tool footprints)
$ cargo audit --version
no matching policy rule -- denied by default (production posture; widen rules to allow new tool footprints)
$ ls -ld ~/.cargo/advisory-db
ls: cannot access '/home/samidarko/.cargo/advisory-db': No such file or directory
```

Trois faits en un : la base d'avis n'est pas clonée dans ce bac à sable, et ses
**deux** consommateurs — `cargo deny` et `cargo audit` — sont refusés par la même
règle que `cargo update`. `cargo audit` était déjà attesté refusé par le document
du 2026-10-21 ; `cargo deny` ne l'était pas, et c'est ce qui manquait pour
répondre à F2 sur mesure plutôt que sur principe.

Conséquence directe, qui est l'objet de la décomposition de l'option 3 ci-dessous :
**cloner `advisory-db` ne rend rien exécutable.** Un clone sans consommateur
autorisé est une base que personne ne peut lire.

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

Ces trois ingrédients n'ont pas le même coût de posture, et l'opérateur peut les
trancher séparément. La décomposition, avec ce que chacun donne — et surtout ce
qu'il ne donne pas :

| ingrédient | coût | ce qu'il débloque seul |
|---|---|---|
| **3a** — cloner `rustsec/advisory-db` dans `~/.cargo/advisory-db` (`deny.toml:5`) | un clone git à provisionner, et sa **fraîcheur** : un `git pull` périodique dont rien dans ce dépôt n'est aujourd'hui le propriétaire déclaré. Taille non mesurée ici — le clone est absent (E6) | **rien.** Ses deux consommateurs, `cargo deny` et `cargo audit`, sont refusés par la même règle que `cargo update` (E6). Un clone sans consommateur autorisé est une base que personne ne peut lire |
| **3b** — autoriser `cargo audit` / `cargo deny` | élargissement **en lecture** : ces deux outils n'écrivent pas le graphe. C'est le plus petit des trois arbitrages | rendrait lisible la plage `versions.patched` d'un avis, donc un **préflight** de cette classe. Ne mute rien : la cascade de E2 reste hors d'atteinte |
| **3c** — autoriser `cargo update` + garantir `registry/src/` inscriptible | élargissement **en écriture** : c'est accorder à un pilote le droit de muter le graphe de dépendances du dépôt. Le `production posture` du refus E4 se lit comme une décision déjà prise dans l'autre sens | la cascade devient techniquement dispatchable — mais le vol C dépasse seul les deux bornes de mika#2636 et mika#2693 d'un facteur large, donc l'ouverture de la policy ne suffirait pas à le rendre dispatchable *en pratique* |

**3a et 3b ne sont pas une voie vers la cascade, et ne doivent pas se lire comme
telle.** Accordés ensemble sans 3c, ils donnent un préflight — la capacité de
*nommer* ce qu'un avis demande — pas la capacité de le satisfaire. Seul 3c lève
E4, et c'est le plus coûteux des trois.

Correction d'une lecture possible, consignée parce que la seconde passe la fera
peut-être aussi : le document de référence est daté du **2026**-10-21 (et non
2025), et il ne présente pas le clone d'`advisory-db` comme une alternative à
l'élargissement — il le liste comme **ingrédient** de sa voie 3
(`docs/solutions/dependency-maintenance/2026-10-21-un-bump-de-lock-rustsec-nest-pas-dispatchable.md:167-172`,
« il faut `cargo update`, `cargo fetch` et `cargo audit`, plus un clone joignable
de `rustsec/advisory-db` »). E6 mesure pourquoi cette lecture est la bonne : sans
3b, le clone n'a pas de lecteur. Énumérer 3a comme troisième voie autonome
inscrirait au plan une option qui ne mène nulle part — et sous mika#3122 règle 1,
un repli qui *paraît* obtenir le même effet par un autre outil est précisément ce
qu'il ne faut pas écrire.

**Coût global :** un arbitrage de posture en trois crans, dont un seul (3c) ouvre
la cascade. **Ce plan n'inscrit aucun geste d'aucun des trois** : mika#3122
règle 1 fait du refus un arbitrage opérateur, et un repli par un autre outil est
explicitement interdit. Ni 3a, ni 3b, ni 3c ne sont pratiqués, ni préparés, ni
recommandés ici — ils sont chiffrés.

### Ce que ce plan ne fait pas

Il ne choisit pas entre les trois. Il ne pose ni ne retire de label. Il
n'ouvre pas de ticket. Il n'édite ni `Cargo.toml`, ni `Cargo.lock`, ni
`rust-toolchain.toml`, ni les deux fichiers d'exception. Il ne contourne pas E4.
Il ne clone pas `rustsec/advisory-db` et ne prépare pas ce clone (3a) : E6
mesure qu'il serait sans lecteur, et le provisionner depuis un pilote de
grooming serait pratiquer un cran d'une option réservée.

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

Ce dispatch n'est pas le second pilote dépensé sur #3131 pour le même constat,
mais au moins le troisième : le vol 1 l'a groomé et livré, une première passe
d'architecte a produit la rev 2 de ce plan, et la boucle a re-dispatché un
grooming par-dessus — c'est la session qui écrit cette ligne. Le décompte exact
n'est pas établi ici et n'a pas besoin de l'être : il croît d'un à chaque tour de
boucle tant que le label reste posé. La cause est un label, pas un défaut de
plan, et chaque re-dispatch re-découvre une halte que deux passes d'architecte
ont déjà acceptée. Une garde structurelle est
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

V3 et V4 ne sont pas des formalités : ce sont les deux assertions qui attestent
que le plan **n'a pas pratiqué** l'une des options qu'il chiffre.

### Sanity check — pas un critère de succès du vol (F3)

`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --check`, `bash scripts/verify-pipeline.sh origin/main`.

**Attendu vert parce qu'aucun code n'a changé**, et la chaîne tourne quand même
— c'est une PR, la CI la fera tourner. Mais elle ne vérifie rien que V3–V5
n'attestent déjà : le seul fichier touché est sous `docs/`, sans doc-test ni
proc-macro, donc aucune de ces quatre commandes ne peut rougir *à cause de ce
vol*. Si l'une rougit, le défaut est préexistant sur `origin/main` et appartient à
un autre ticket — pas à celui-ci.

Consigné comme sanity check et non comme ligne du contract, pour que le contract
ne porte que des assertions dont ce vol est la cause.

---

## Definition of Done

- Le document de solution du 2026-11-24 porte la correction de E5 : le `src/` en
  lecture seule est un état de session, l'invariant de la classe est le refus de
  `cargo update` par la politique de permission.
- `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `deny.toml` et
  `.cargo/audit.toml` sont **identiques** à `origin/main`.
- V1–V5 passent, et le sanity check est vert — vert par construction, aucun code
  n'ayant changé (F3).
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
- **AC4** — le fichier
  `docs/plans/2026-11-24-002-chore-3131-la-cascade-reste-reservee-a-loperateur-plan.md`
  porte une section `## La décision réservée à l'opérateur` qui (a) chiffre les
  trois options avec leur coût — deux labels / trois tickets / élargissement de
  posture, ce dernier décomposé en ses trois crans 3a–3c et ce que chacun
  débloque seul ; (b) donne le coût mesuré de chacun des trois vols A, B et C de
  la cascade ; et (c) n'en pratique ni n'en prépare aucune — rapporter laquelle
  le vol 1 recommandait est permis, l'appliquer non.
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

---

## Revision history

- **rev 1** (2026-11-24) — première passe. Plan de halte : la part attestable de
  #3131 est livrée par le vol 1 (PR #3161) ; la cascade reste réservée à
  l'opérateur, trois options chiffrées, aucune pratiquée (mika#3122).
- **rev 2** (2026-11-24) — adresse les quatre findings de la première passe
  d'architecte (`Disposition: ITERATE`) :
  - **F1** (bloquant, ambiguïté temporelle de E5) — adressé par le sous-§
    « Deux dates, et il faut les séparer » : une table sépare l'instant du
    **fait** (extraction aboutie le 2026-11-23 08:35) de celui de la **mesure**
    (`ls -ld` re-joué le 2026-11-24 ~20:08 UTC, cette session), et situe les deux
    par rapport au commit `cc32f5e0` (05:09 UTC) et au merge `3585e7be`
    (07:05 UTC). La réponse à la question posée par le finding — pourquoi le vol 1
    ne l'a pas inclus — est écrite sans détour : le fait préexistait d'environ
    21 h, le vol 1 **n'a pas cherché de contre-exemple**, M6 a généralisé une
    sortie d'erreur unique en propriété de montage. Le plan ne revendique donc
    plus une « découverte de cette session » mais une mesure, re-jouable à
    l'identique — et c'est précisément l'inférence trop large que R1 corrige.
  - **F2** (bloquant, énumération incomplète de l'option 3) — adressé par la
    décomposition de l'option 3 en trois crans séparables (3a clone
    d'`advisory-db` / 3b lecture `cargo audit`+`cargo deny` / 3c écriture
    `cargo update`), chacun avec son coût et ce qu'il débloque **seul**, plus la
    mesure neuve **E6**. La prémisse du finding est corrigée sur mesure plutôt
    que sur principe : le document de référence est daté du **2026**-10-21 (non
    2025) et liste le clone comme *ingrédient* de sa voie 3, non comme
    alternative — citation préservée
    (`docs/solutions/dependency-maintenance/2026-10-21-un-bump-de-lock-rustsec-nest-pas-dispatchable.md:167-172`).
    E6 mesure que `cargo deny` **et** `cargo audit` sont refusés par la même règle
    que `cargo update` et que `~/.cargo/advisory-db` est absent : un clone local
    n'aurait aucun lecteur. 3a est donc inscrit comme cran chiffré, et
    explicitement **pas** comme troisième voie autonome — l'écrire ainsi aurait
    inscrit au plan un repli apparent, ce que mika#3122 règle 1 interdit.
  - **F3** (sharpening, redondance de V6) — adressé : V6 sort de la table du
    Verification Contract et devient un sous-§ « Sanity check — pas un critère de
    succès du vol », attendu vert par construction. La DoD suit (« V1–V5 passent,
    et le sanity check est vert »). Le contract ne porte plus que des assertions
    dont ce vol est la cause.
  - **F4** (sharpening, AC4 méta-récursif) — adressé : AC4 porte désormais sur
    l'artefact nommé (le fichier de plan et sa section
    `## La décision réservée à l'opérateur`) avec trois clauses vérifiables
    (a)(b)(c). Non affaibli mais élargi — la clause (c) est neuve et interdit de
    préparer une option, tout en préservant la permission de *rapporter* celle
    que le vol 1 recommandait, comme la DoD l'exige.

  Aucun finding n'est resté sans réponse. La révision ne change pas la nature du
  plan : il reste un plan de halte à 0 ligne de code, son unique livrable reste
  documentaire (R1), et son verdict attendu reste `ESCALATE` — sortie nominale
  sous mika#3122 règle 3.
- **rev 3** (2026-11-24) — re-dispatch de grooming. La rev 2 était **présente
  mais non committée** dans le worktree ; cette révision la commite et corrige
  deux défauts qu'elle portait :
  - **Titre avalé.** L'insertion du sous-§ F1 et de la mesure E6 en rev 2 a
    supprimé le titre `## La décision réservée à l'opérateur` (rev 1, ligne 140).
    Les trois Options étaient devenues filles de `### E6`, et deux sites du plan
    — **AC4** et le § `## Taille estimée` — citaient une section qui n'existait
    plus : un critère d'acceptation invérifiable par construction. Le titre est
    restauré à son niveau `##`, son paragraphe d'ouverture retrouve sa place, et
    les trois Options redeviennent ses filles. Correction **structurelle
    seulement** : aucun mot des Options 1–3, de E5, de E6 ni des findings F1–F4
    n'est modifié.
  - **Décompte de pilotes dépassé.** R3 disait « second pilote » ; ce
    re-dispatch le rend faux. R3 nomme désormais ce qui est observable — au moins
    trois, et un de plus à chaque tour de boucle tant que le label reste posé —
    sans revendiquer un décompte exact que cette session ne peut pas établir.

  Les faits portants ont été re-mesurés dans ce worktree et tiennent à
  l'identique : `3585e7be` est bien ancêtre de `HEAD` (E1) ; les cinq sites de la
  cascade sont inchangés — `Cargo.toml:14` `rust-version = "1.91"`,
  `Cargo.toml:30` rusqlite `"0.32"`, `Cargo.toml:128` sqlx `"0.8"`,
  `rust-toolchain.toml:2` `channel = "1.93"` (E2) ; les deux exceptions sont en
  place (`deny.toml:21`, `.cargo/audit.toml:27`) ; et le document cible de R1
  existe. **E4 n'a pas été re-joué** : le plan nomme ce refus au lieu de le
  sonder, et re-lancer une commande que la politique refuse n'ajouterait rien
  qu'un tour.

  La nature du plan est inchangée : 0 ligne de code, unique livrable
  documentaire, verdict attendu `ESCALATE`. Aucune des trois options n'est
  pratiquée, ni préparée, ni recommandée par cette révision.
