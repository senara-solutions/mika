# Aucun `VACUUM` dans le chemin de démarrage — vol 1 (mika#2735)

- **Ticket** : senara-solutions/mika#2735 (`bug`, `p1-important`, `ready`,
  `dispatch:loop`, `loop-substrate`, jalon « Substrat de boucle »)
- **Périmètre de ce vol** : garde **(a)** du correctif retenu — plus aucun
  `VACUUM` dans le chemin de démarrage, et un détecteur structurel qui refuse
  sa réintroduction.
- **Suite** : vols 2 et 3 — `Tracked in: senara-solutions/mika#2735`
  (voir § *Découpage en vols*).

## Contexte

Le déploiement du 2026-10-16 a joué la migration v55→v56 (mika#2658) au
démarrage de mika-spirit. Entre `v56: vacuum complete` (08:50:38Z) et la
première salve de 19 WARN `file is not a database` (08:50:45Z), il s'est écoulé
sept secondes. À 09:00Z un `claim` de tâche échoue sur `database disk image is
malformed`. Le commentaire du 2026-10-16T11:05Z établit que **le fichier
lui-même est abîmé** : 4 arbres touchés à 09:15Z, 25 à 11:00Z, avec les erreurs
`btreeInitPage() returns error code 11`, `2nd reference to page`, **`Bad ptr map
entry`**, et une plage `Page 110606… never used`. `sqlite3 .recover` rend un
fichier propre dont le delta avec la sauvegarde v55 se limite à l'élagage du
démarrage.

Le corps du ticket fixe le périmètre : trois gardes **(a)**, **(b)**, **(c)**
qui protègent *quelle que soit la cause*, la cause exacte n'étant pas établie.
Ce plan livre **(a)** seule, parce que le volume des trois dépasse la borne par
vol (§ *Découpage en vols*), et parce que **(a)** est à elle seule la
remédiation p1 : c'est la seule des trois qui retire l'acte qui a réécrit le
fichier.

## Ce que le code offre déjà — relevé, parce qu'il change le périmètre

### Il y a DEUX `VACUUM` dans le chemin de démarrage, pas un

Le ticket, son hypothèse et son AC1 ne nomment que celui de
`migrate_v55_to_v56`. Le relevé en trouve un second, et c'est celui dont la
fenêtre d'exécution colle le mieux à la trace :

| site | quand il tourne | combien de connexions sont ouvertes alors |
|---|---|---|
| `db/migrations.rs:504` — `self.conn.execute_batch("VACUUM;")` | pendant le premier `Database::open` du processus, hors transaction, après l'estampille v56 | **aucune autre** dans ce processus (voir ci-dessous) |
| `server/mod.rs:814` — `db.vacuum().await` dans `startup_cleanup` | tâche `tokio::spawn` **fire-and-forget**, lancée *après* que `ready` soit posé (`server/mod.rs:1855`) | **toutes** : une par agent, plus le moteur de tâches qui tique |

`startup_cleanup` n'est pas gardé par une version de schéma : il tourne à
**chaque** démarrage, et son `VACUUM` est conditionné à
`compact_old_audit_events(…) > 0` — ce qu'un déploiement après plusieurs jours
d'uptime satisfait à coup sûr. C'est donc un `VACUUM` qui réécrit le fichier
pendant que les balayages de démarrage — `expire timed-out tasks`,
`phantom_sweep`, `callback_watchdog`, `pilot_stall_reaper` — lisent et écrivent
sur d'autres connexions. Exactement la forme que l'hypothèse du ticket décrit,
et celle qui explique « 25 s après le démarrage » mieux que la migration, dont
le `VACUUM` était terminé depuis sept secondes.

**Conséquence de périmètre.** Un détecteur scopé au seul `db/migrations.rs`,
comme l'écrit AC1, laisserait ce second site en place : le ticket serait clos et
le `VACUUM` concurrent tournerait au prochain démarrage. La prose de la garde
(a) est plus large que son AC illustratif — *« Plus aucun `VACUUM` dans le
chemin de démarrage »* — et c'est elle que ce vol applique. AC1 reste satisfait
au mot près : après ce vol, `grep -n VACUUM crates/mika-agent/src/db/migrations.rs`
ne rend que des commentaires.

### Les connexions du processus, et dans quel ordre elles naissent

`init_agent` (`server/mod.rs:599`) fait un `Database::open(&db_path)` sur **la
même** base par agent, dans une boucle `for … .await` **séquentielle**
(`server/mod.rs:1058`). Chaque `Database::open` appelle `migrate()`. Donc :

- le premier agent migre et, aujourd'hui, `VACUUM`e ; les agents 2..N ouvrent
  **après** et voient la v56, leur `migrate()` est un no-op ;
- `dashboard_db` (`server/mod.rs:1119`) est un **clone** de l'`AsyncDatabase` de
  l'agent par défaut : il partage la connexion, ce n'en est pas une seconde ;
- il n'y a donc, dans le processus, **aucune connexion antérieure** au `VACUUM`
  de la migration. Ce qui est antérieur à celui de `startup_cleanup`, en
  revanche, c'est tout le reste.

Deux autres sites de production ouvrent la même base : `teams/mod.rs:104` et
`teams/engine.rs:308`. Et **toute invocation de la CLI `mika`** ouvre la base et
joue `migrate()` : pendant la fenêtre d'un `make deploy`, une session pilote qui
fait `mika tasks list` est un second processus sur le même fichier. C'est la
question que pose l'AC2 d'origine, et le § *Runbook* y répond.

### La conversion de format, qui est la seule explication des `Bad ptr map entry`

`Database::open` pose `PRAGMA auto_vacuum = INCREMENTAL` (`db.rs:1095`) sur
chaque connexion. Sur une base née en `auto_vacuum=NONE`, ce pragma est
**inopérant et en attente** : SQLite documente que passer de `none` à
`full`/`incremental` ne prend effet **que par un `VACUUM`**. Le `VACUUM` du
démarrage est donc aussi une **conversion de format** : il ajoute au fichier les
pages de *pointer map* que la base n'avait pas.

Aucun `incremental_vacuum` n'existe nulle part dans le dépôt — le pragma n'a
jamais été exercé, et personne n'avait donc jamais observé sa conversion.

Deux des quatre signatures d'erreur relevées dans le commentaire — `Bad ptr map
entry` et `2nd reference to page` — sont des erreurs de **ptrmap**, c'est-à-dire
de la machinerie que cette conversion vient d'installer. Ce vol ne prétend pas
établir la causalité : il **mesure la conversion** par un test (§ *Conception*,
`conversion_auto_vacuum.rs`) pour que le fait soit épinglé plutôt que raconté,
et il retire l'acte qui la déclenche. La ligne `PRAGMA auto_vacuum =
INCREMENTAL` elle-même est laissée en place et nommée au § *Hors périmètre*.

### Ce que le dépôt fournit déjà pour écrire le détecteur

- `mika_common::source_guard::rust_sources_under(&Path) -> Vec<PathBuf>`
  (`source_guard.rs:932`) — l'énumération.
- `mika_common::source_guard::mask_test_regions(&str) -> String`
  (`source_guard.rs:121`) — le masquage des régions de test **inline**
  (mika#2398), et non une troncature au premier `#[cfg(test)]`.
- `crate::source_scan::is_test_source_path(&Path) -> bool` — la classification
  **par chemin** (mika#2321), qui écarte entièrement un fichier de test extrait.

L'ordre des deux écartements compte, et il est déjà écrit dans
`db/tests/mod.rs:79-93` : classification par chemin **d'abord**, masquage
ensuite. Le détecteur reprend cet idiome, neutralisation des lignes de
commentaire comprise — indispensable ici, puisque le paragraphe de doc réécrit
nomme `VACUUM` une demi-douzaine de fois.

Les tests de migration vivent dans `db/migrations/tests/` (sous-module de
`migrations.rs`, règle *« un module de test voyage avec le code qu'il teste »*),
aujourd'hui `mod.rs` + `kg_schema.rs`.

## Découpage en vols

Le total des trois gardes est estimé à ≈ 735 lignes hors `docs/`, pour une borne
par vol de 431. Découpage en trois vols, chacun mergeable seul :

| vol | contenu | lignes estimées | pourquoi là |
|---|---|---|---|
| **1 — celui-ci** | garde **(a)** : retrait des deux `VACUUM` du démarrage, détecteur structurel, runbook | **258** | c'est la remédiation p1 : il retire l'acte qui a abîmé le fichier. Mergeable et déployable sans rien attendre. |
| 2 | garde **(b)** : `mika db vacuum` sous exclusivité, refus journalisé | ≈ 375 | rend la capacité que le vol 1 retire. N'est pas un préalable : l'intervalle a un geste opérateur documenté (§ *Runbook*). |
| 3 | garde **(c)** : sauvegarde de pré-migration par `VACUUM INTO` au lieu de `fs::copy` | ≈ 150 | défaut indépendant (une sauvegarde sans son `-wal`), sur un chemin que le vol 1 ne touche pas. |

**Ce vol ne clôt pas mika#2735.** Les vols 2 et 3 restent dus et leurs critères
d'acceptation (2 et 3 de la seconde liste) ne sont pas servis ici.
`Tracked in: senara-solutions/mika#2735` — à ouvrir en sous-tickets de ce ticket
avant d'engager le vol 2, afin que chacun porte son propre corps et sa propre
estimation.

L'ordre 1 → 2 → 3 est celui du risque décroissant : le vol 1 arrête une
corruption observée, le vol 2 ferme une régression de capacité, le vol 3 ferme
un défaut qui n'a encore rien cassé.

## Conception — vol 1

### `crates/mika-agent/src/db/migrations.rs`

Retirer le bloc `VACUUM` (lignes 501-515) : la mesure `before`, les deux
`info!`, le `match … execute_batch("VACUUM;")` et son bras `warn!`.
`migrate_v55_to_v56` finit sur `info!("v55→v56: …")` puis `Ok(())`.

`db_size_bytes()` **reste utilisée** ailleurs (`server/mod.rs:823`,
`async_db.rs:2502`) : aucun code mort à nettoyer.

Réécrire le paragraphe de doc-comment *« Le `VACUUM` fait partie du livrable, et
il est après le commit »* (lignes 433-450). Il affirme aujourd'hui l'inverse de
ce que le code fera, et il porte le raisonnement de mika#2658 qu'il faut
**conserver en le datant** — pas effacer. Le paragraphe de remplacement dit, en
substance :

- ce que mika#2658 voulait (`DROP COLUMN` rend les pages à la *freelist*, pas au
  système) reste vrai, et **n'est plus obtenu par cette migration** ;
- pourquoi : mika#2735, avec la date, les deux signatures ptrmap, et le fait que
  le `VACUUM` sur une base `auto_vacuum=NONE` est **aussi** une conversion de
  format ;
- où l'espace est récupéré désormais : par la commande opérateur du vol 2, et
  d'ici là par le geste du runbook ;
- **le coût nommé** : le fichier reste à sa taille, donc le WARN
  `database size exceeds threshold` (`server/mod.rs:825`) continue de sonner à
  chaque démarrage tant que l'opérateur n'a pas compacté. C'est une régression
  assumée de l'effet observable de mika#2658, et la nommer ici est ce qui
  empêche un futur lecteur de « réparer » la migration en y remettant le
  `VACUUM`. Le détecteur, lui, l'empêche mécaniquement.

Les lignes de commentaire étant neutralisées par le détecteur, ce paragraphe
peut nommer `VACUUM` autant qu'il le faut.

### `crates/mika-agent/src/server/mod.rs`

Dans `startup_cleanup`, le bras

```rust
Ok(deleted) if deleted > 0 => {
    info!(deleted, "compacted old audit events");
    if let Err(e) = db.vacuum().await {
        warn!(error = %e, "failed to vacuum database");
    }
}
```

perd son `VACUUM` et garde son `info!`. Un commentaire prend la place du bloc
retiré et dit **la seule chose qu'un lecteur ne peut pas déduire** : que ce site
était un `VACUUM` concurrent de tout le tick du moteur — tâche fire-and-forget
lancée après `ready` — donc le plus exposé des deux, et que l'espace libéré par
la compaction des `audit_events` reste désormais dans la *freelist* jusqu'à la
commande opérateur.

Mettre à jour le doc-comment de la fonction : *« Run background startup cleanup
tasks (pruning, vacuum, backfill) »* → sans `vacuum`.

### `crates/mika-agent/src/async_db.rs` et `crates/mika-agent/src/db.rs`

Supprimer `AsyncDatabase::vacuum` (`async_db.rs:2510`, seul appelant :
`server/mod.rs:814`) et `Database::vacuum` (`db.rs:5075`, seul appelant :
`AsyncDatabase::vacuum`).

**Pourquoi les supprimer et ne pas seulement cesser de les appeler.** C'est ce
qui rend le registre du détecteur **vide** à la livraison, comme l'exige AC1.
Une méthode `pub fn vacuum()` sans appelant est une entrée d'allowlist
déguisée : elle survit aux relectures, et le prochain qui cherche « comment
compacter la base » la rappelle. Le vol 2 réintroduit un unique site de `VACUUM`
dans son propre module (`db/vacuum.rs`), avec sa garde d'exclusivité — et c'est
là, et seulement là, qu'il recevra son entrée déclarée au registre.

Coût nommé : entre ce vol et le vol 2, le dépôt n'offre **aucun** moyen
programmatique de compacter la base. Le geste est `sqlite3`, mika-spirit arrêté,
et le runbook l'écrit.

### `crates/mika-agent/src/db/migrations/tests/aucun_vacuum_au_demarrage.rs` (nouveau)

Le détecteur. Un module de test, pas un script shell : le prédicat a besoin de
`crate::source_scan::is_test_source_path`, qui est `pub(crate)`.

**Périmètre scanné** : `crates/mika-agent/src/**/*.rs`, via
`rust_sources_under`. Plus large que `migrations.rs` — c'est le relevé qui
l'impose (deux sites, un seul dans `migrations.rs`), et AC1 reste satisfait au
mot près.

**Le prédicat.** Pour chaque fichier :

1. si `is_test_source_path(path)` → zéro violation (le fichier n'a pas de moitié
   production) ;
2. sinon `mask_test_regions(src)` ;
3. puis neutralisation des lignes dont le `trim_start()` commence par `//` — en
   les remplaçant par une ligne vide, pour que les numéros de ligne restent ceux
   du fichier (idiome `db/tests/mod.rs:95-99`) ;
4. sur ce qui reste, chaque occurrence ASCII-insensible à la casse de `vacuum`
   est une violation **sauf si** :
   - le caractère qui précède est alphanumérique, `_` ou `:` — écarte
     `auto_vacuum`, `incremental_vacuum`, `crate::db::vacuum::…` ;
   - le caractère qui suit est alphanumérique, `_`, `:` ou `(` — écarte
     `vacuum_exclusive`, `db_vacuum_refused`, `vacuum::run_exclusive`, et un
     appel de fonction nommée `vacuum` ;
   - le mot suivant, après les blancs, est `INTO` — **écarte `VACUUM INTO`,
     par prédicat et non par fichier**. C'est load-bearing : le vol 3 pose un
     `VACUUM INTO` dans `db.rs`, qui est sur le chemin de démarrage. Les deux
     ne sont pas le même acte — `VACUUM` réécrit le fichier vivant,
     `VACUUM INTO` écrit un fichier neuf et ne touche pas à la base. Exempter
     par prédicat plutôt que par chemin est ce qui empêche le vol 3 d'avoir à
     désarmer le détecteur du vol 1.

**Le registre, livré vide** (`VACUUM_PRODUCTION_SITES: &[(&str, &str)]` —
chemin relatif à `src/`, raison lisible). Après ce vol, la moitié production de
`crates/mika-agent/src/` ne porte **aucune** occasion retenue : les deux sites
sont retirés, `db.rs:1095` est écarté par le `_` d'`auto_vacuum`, et
`server/mod.rs:133`/`787` sont de la prose. On déclare, on n'allowliste pas
(doctrine mika#2201) : quand il faudra une entrée — le vol 2 —, elle nommera son
site et sa raison.

**Les deux sens du registre**, qui lisent la même liste d'occurrences :

- `mika2735_aucun_vacuum_dans_la_production_de_mika_agent` — ce qui n'est pas
  déclaré est une violation. Le message d'échec nomme `chemin:ligne` et
  renvoie à ce plan.
- `mika2735_le_registre_des_vacuum_na_pas_dentree_morte` — **assertion
  auto-nettoyante** : chaque entrée du registre doit correspondre à une
  occurrence réellement présente, sinon elle est morte et le test rougit. Tant
  que le registre est vide, l'assertion est un no-op vrai ; elle est écrite
  maintenant parce que c'est le vol 2 qui lui donnera du travail, et qu'une
  garde ajoutée en même temps que sa première exception n'est jamais écrite.

**Trois contrôles négatifs**, sur des sources synthétiques en mémoire — le
prédicat est une fonction pure sur `(&Path, &str)`, donc testable sans écrire de
fichier :

| test | source synthétique | attendu |
|---|---|---|
| `mika2735_le_scan_rougit_sur_un_vacuum_reinsere` | `self.conn.execute_batch("VACUUM;")?;` | **1** violation — c'est AC1 : « la source scan est rouge si on réinsère un `VACUUM` » |
| `mika2735_vacuum_into_nest_pas_un_vacuum` | `self.conn.execute("VACUUM INTO ?1", [p])?;` | **0** violation — l'exemption du vol 3, par prédicat |
| `mika2735_la_prose_et_auto_vacuum_ne_sont_pas_des_violations` | une ligne `// … le VACUUM de mika#2658 …` et une ligne `PRAGMA auto_vacuum = INCREMENTAL;` | **0** violation — sinon le paragraphe de doc réécrit ferait rougir sa propre garde |

### `crates/mika-agent/src/db/migrations/tests/conversion_auto_vacuum.rs` (nouveau)

Ce que personne n'avait mesuré, et qui est la seule explication des erreurs de
ptrmap. Sur une base **sur fichier** (`tempfile`), avec `rusqlite` directement
et non `Database::open` :

1. créer une table, lire `PRAGMA auto_vacuum` → **0** (`NONE`) ;
2. poser `PRAGMA auto_vacuum = INCREMENTAL` comme le fait `Database::open`,
   relire → **toujours 0** : le pragma est en attente, inopérant ;
3. `VACUUM;`, relire → **2** (`INCREMENTAL`).

`mika2735_un_vacuum_convertit_le_format_du_fichier_sous_les_autres_connexions`.
Ce test ne garde rien : il **épingle un fait** sur lequel repose tout le
raisonnement de ce vol et du commentaire du ticket. Il ne dépend d'aucun code de
mika — il reste vert quoi qu'on change dans le dépôt, et rouge seulement si
SQLite change de comportement, ce qui est précisément quand on voudrait le
savoir. Son nom et son doc-comment portent les deux signatures d'erreur
relevées, pour qu'un futur lecteur retrouve le lien.

### `crates/mika-agent/src/db/migrations/tests/mod.rs`

Deux déclarations : `mod aucun_vacuum_au_demarrage;` et
`mod conversion_auto_vacuum;`.

## Runbook — AC2 de la première liste (hors budget de lignes)

### `docs/deployment.md`

Nouvelle section, après § 4 *Container Deployment*, intitulée **« Schema
migrations and file rewrites »**. Elle répond à la question d'AC2 — *« les
processus d'agents peuvent-ils tenir la base pendant une migration avec
réécriture, et sinon dans quel ordre les démarrer ? »* — par un **oui
conditionné**, et dit à quoi il est conditionné :

- **Oui, depuis ce vol** : plus aucune migration ne réécrit le fichier vivant.
  Une migration est du DDL et du DML sous transaction ; les autres connexions
  et les autres processus (une CLI `mika` d'une session pilote, par exemple)
  peuvent la tenir. Aucun ordre de démarrage n'est imposé.
- **Non pour une réécriture**, et il n'y en a plus qu'une seule, explicite :
  le compactage. Tant que le vol 2 n'a pas livré `mika db vacuum`, le geste est
  manuel, et il est écrit ici : arrêter `mika-spirit`, vérifier qu'aucune
  session pilote ne tourne (`pgrep -fa 'mika '`), `sqlite3 ~/.mika/data/mika.db
  'VACUUM;'`, puis `PRAGMA quick_check`, puis redémarrer. Jamais avec le service
  en marche.
- **Ce que le compactage change au fichier**, parce que c'est contre-intuitif et
  que c'est la piste du ticket : sur une base née en `auto_vacuum=NONE`, le
  premier `VACUUM` applique le `PRAGMA auto_vacuum = INCREMENTAL` que chaque
  connexion pose, et **convertit le format** en y ajoutant des pages de
  *pointer map*. Un `VACUUM` n'est donc pas une opération neutre sur ce
  fichier-là.
- **Pourquoi le WARN de taille continue de sonner** après ce déploiement, et que
  ce n'est pas un défaut : renvoi au § `## Taille de la base` et au geste
  ci-dessus.

`deployment.md` est dans le jeu de `scripts/sync-agent-docs.sh` (ligne 17) et de
`build.rs` : **lancer le script de sync dans le même commit**, sinon le job CI
`docs-sync` rougit.

### `docs/agent-internals/schema-version.md`

L'entrée `v55->v56` affirme aujourd'hui *« puis `VACUUM` »*, *« Le `VACUUM` est
hors transaction et après l'estampille »*, *« c'est lui qui produit le gain
réel »* et nomme les deux lignes de log comme instrument de mesure d'AC1 de
mika#2658. Les quatre deviennent faux. Réécrire l'entrée en gardant le
raisonnement de mika#2658 et en le datant de mika#2735 : la colonne est bien
supprimée, le gain d'espace n'est **plus** pris par la migration, les deux
lignes de log n'existent plus, et le compactage est un geste opérateur.
Ce fichier n'est pas dans le jeu de sync.

### `docs/solutions/database-issues/`

Un document de solution — convention du dépôt pour un incident de production :
`vacuum-au-demarrage-corruption-ptrmap-2026-10-16.md`, avec son frontmatter
YAML (`module: db`, `tags: [sqlite, vacuum, auto_vacuum, corruption,
migration]`, `problem_type: data_corruption`). Il porte la chronologie du
2026-10-16, les quatre signatures d'erreur, les deux sites de `VACUUM` relevés
avec leur fenêtre de concurrence respective, le fait mesuré par
`conversion_auto_vacuum.rs`, et le geste de récupération par `sqlite3 .recover`
que l'opérateur a déjà joué.

## Hors périmètre, nommé

**`PRAGMA auto_vacuum = INCREMENTAL` (`db.rs:1095`) reste en place.** C'est une
ligne, et la retirer rendrait la conversion de format impossible pour toujours
— ce qui est peut-être souhaitable. Trois raisons de ne pas le décider ici :

1. ce n'est aucune des trois gardes retenues, et le ticket dit *« C'est le
   périmètre du ticket »* ;
2. ce pragma n'a jamais été exercé (aucun `incremental_vacuum` dans le dépôt),
   donc le retirer ne retire rien d'actif — mais choisir entre `NONE` et
   `INCREMENTAL` pour le fichier de production est un arbitrage d'exploitation
   (surcoût de ptrmap contre récupération incrémentale) qui appartient à
   l'opérateur ;
3. après ce vol, plus aucun `VACUUM` ne tourne au démarrage : la conversion ne
   peut donc plus se produire sans un geste explicite, et le runbook dit que ce
   geste la provoque. Le risque est neutralisé sans la décision.

**Recommandation pour l'opérateur** : un ticket de suivi d'une ligne de code, à
instruire après le vol 2, qui tranchera cette valeur et fera que
`mika db vacuum` ne convertisse jamais le format en silence. Le vol 2 devra de
toute façon **lire** `auto_vacuum` sur le fichier plutôt que de l'écrire, et
asserter `auto_vacuum_after == auto_vacuum_before` : sa spécification le dira.

**Les deux `VACUUM` ne sont pas prouvés coupables.** Ce vol ne prétend pas
établir la causalité de la corruption, et aucun de ses tests ne la reproduit
(§ *Contrat de vérification*, dernière ligne). Il retire l'acte et épingle le
mécanisme.

## Contrat de vérification

| ce qui est vérifié | comment |
|---|---|
| plus aucun `VACUUM` dans la production de `mika-agent` | `mika2735_aucun_vacuum_dans_la_production_de_mika_agent` |
| le détecteur rougit sur une réinsertion | `mika2735_le_scan_rougit_sur_un_vacuum_reinsere` (contrôle négatif d'AC1) |
| `VACUUM INTO` n'est pas pris pour un `VACUUM` | `mika2735_vacuum_into_nest_pas_un_vacuum` |
| la prose et `auto_vacuum` ne sont pas des violations | `mika2735_la_prose_et_auto_vacuum_ne_sont_pas_des_violations` |
| le registre n'a pas d'entrée morte | `mika2735_le_registre_des_vacuum_na_pas_dentree_morte` |
| un `VACUUM` convertit le format du fichier | `mika2735_un_vacuum_convertit_le_format_du_fichier_sous_les_autres_connexions` |
| AC1 au mot près | `grep -n VACUUM crates/mika-agent/src/db/migrations.rs` ne rend que des commentaires |
| la migration v55→v56 fait toujours ce qu'elle doit | les trois tests mika#2658 existants restent verts, sans modification : `mika2658_migration_v55_to_v56_replaces_payload_with_marker`, `mika2658_migration_v55_to_v56_is_idempotent`, `mika2658_fts_search_survives_the_column_drop` |
| rien d'autre n'a bougé | `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` |

**Ce que ce vol ne vérifie pas, et le dit.** AC1 de la première liste demande un
contrôle négatif *comportemental* — « sans le correctif, au moins une connexion
échoue ». Il n'est **pas livré**, et promettre le contraire serait malhonnête :
la corruption observée a demandé un `VACUUM` concurrent d'écritures réelles sur
plusieurs connexions, sur un fichier de 1 Go, et une conversion de format. Un
test unitaire qui corrompt un fichier SQLite à la demande n'est pas quelque chose
que ce plan peut promettre de livrer vert-puis-rouge de façon déterministe. Ce
qui porte la garantie, à la place, est le **détecteur structurel** — dont le
contrôle négatif *est* déterministe — plus le fait mesuré par
`conversion_auto_vacuum.rs`. C'est une substitution, elle est consciente, et
elle est écrite ici pour que personne n'ait à la deviner.

## Fire-Disposition

Ce vol livre un détecteur : la source scan
`mika2735_aucun_vacuum_dans_la_production_de_mika_agent`.

**Option (a) — exception nommée en allowlist, registre livré VIDE.**

- Le registre est `VACUUM_PRODUCTION_SITES: &[(&str, &str)]`, grep-visible,
  chaque entrée portant `(chemin relatif à src/, raison lisible)`.
- **Il est livré vide, et c'est vrai, pas aspirationnel** : les deux violations
  existantes sont retirées **dans le même commit** que le détecteur
  (`migrations.rs`, `server/mod.rs`), et les deux méthodes qui auraient subsisté
  sans appelant sont supprimées (`Database::vacuum`, `AsyncDatabase::vacuum`)
  précisément pour qu'aucune entrée ne soit nécessaire.
- **Assertion auto-nettoyante** :
  `mika2735_le_registre_des_vacuum_na_pas_dentree_morte` lit la *même* liste
  d'occurrences que la garde et rougit sur toute entrée qui ne correspond plus à
  une occurrence réelle. Elle est écrite dès ce vol, alors qu'elle n'a rien à
  faire, parce que c'est le vol 2 qui posera la première entrée et qu'une garde
  ajoutée en même temps que son exception n'est jamais ajoutée.
- **Contrôle négatif** : `mika2735_le_scan_rougit_sur_un_vacuum_reinsere`, plus
  deux contrôles de faux positif
  (`mika2735_vacuum_into_nest_pas_un_vacuum`,
  `mika2735_la_prose_et_auto_vacuum_ne_sont_pas_des_violations`) — le second
  étant nécessaire à la survie du paragraphe de doc réécrit, qui nomme l'interdit.
- **Aucun désarmement** : pas de `#[ignore]`, pas de `#[cfg(skip)]`. Le
  détecteur atterrit armé et vert.

## Definition of Done

1. `migrate_v55_to_v56` ne contient plus de `VACUUM` ; son paragraphe de doc dit
   pourquoi, avec le ticket, la date, et le coût nommé (le WARN de taille
   continue de sonner).
2. `startup_cleanup` ne contient plus de `VACUUM` ; son commentaire dit que ce
   site était concurrent du tick du moteur.
3. `Database::vacuum` et `AsyncDatabase::vacuum` sont supprimées ; rien ne les
   appelle plus, et le registre du détecteur est vide.
4. Le détecteur et ses cinq tests (garde, auto-nettoyage, trois contrôles) sont
   verts et armés.
5. `conversion_auto_vacuum.rs` épingle la conversion `NONE → INCREMENTAL`.
6. Les trois tests mika#2658 existants sont verts sans modification.
7. `docs/deployment.md` porte la section de runbook, et
   `scripts/sync-agent-docs.sh` a été lancé dans le même commit.
8. `docs/agent-internals/schema-version.md` n'affirme plus que la v56 `VACUUM`e.
9. `docs/solutions/database-issues/vacuum-au-demarrage-corruption-ptrmap-2026-10-16.md`
   existe avec son frontmatter.
10. `cargo test`, `cargo clippy --all-targets -- -D warnings`,
    `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main` passent.
11. Le corps de PR dit que mika#2735 **n'est pas clos** par cette PR (pas de
    `Closes #2735`), nomme les vols 2 et 3, et porte
    `Tracked in: senara-solutions/mika#2735`.

## Acceptance criteria

Le ticket porte **deux** sections `## Acceptance criteria`. Les deux sont
transcrites ci-dessous. La seconde, postérieure, fixe le périmètre du correctif
retenu ; la première garde son AC2, que rien ne remplace.

### Première liste (corps d'origine)

- **AC1** : après une migration avec `VACUUM`, aucune connexion ouverte avant ne
  sert une requête. Test : ouvrir le pool, migrer avec `VACUUM`, puis requêter
  par chaque connexion du pool. Aucune erreur. Contrôle négatif dans le même
  test : sans le correctif, au moins une connexion échoue.
  → **Servi par substitution, et c'est dit** : le correctif retenu supprime le
  `VACUUM` du démarrage, si bien qu'il n'y a plus de « migration avec `VACUUM` »
  à tester. Ce que ce vol livre à la place est le détecteur structurel, avec son
  contrôle négatif déterministe. Le contrôle négatif *comportemental* n'est pas
  livré ; le § *Contrat de vérification* dit pourquoi.
- **AC2** : le runbook de déploiement dit si les processus d'agents peuvent tenir
  la base pendant une migration avec réécriture, et sinon dans quel ordre les
  démarrer. → **servi** par le § *Runbook* (`docs/deployment.md`).
- Taille : à estimer tests compris. → **servie** par le § *Taille estimée*.

### Seconde liste (correctif retenu, orchestrateur, 2026-10-16)

1. `grep -n VACUUM crates/mika-agent/src/db/migrations.rs` ne renvoie plus que
   des commentaires. La source scan est rouge si on réinsère un `VACUUM`.
   → **servi par ce vol**, et élargi : le scan couvre
   `crates/mika-agent/src/**`, parce que le second `VACUUM` du chemin de
   démarrage n'est pas dans `migrations.rs`.
2. La commande opérateur réussit sur une base sans autre connexion. Elle refuse,
   sans rien modifier, quand une seconde connexion tient la base. Le test ouvre
   une vraie seconde connexion. → **vol 2**, non servi ici.
3. La sauvegarde de pré-migration d'une base en WAL contenant des écritures non
   checkpointées les retrouve après restauration. Le test doit être rouge avec
   `fs::copy`. → **vol 3**, non servi ici.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/migrations.rs` (retrait du bloc `VACUUM` + réécriture du paragraphe de doc) | 35 |
| `crates/mika-agent/src/server/mod.rs` (retrait du `VACUUM` de `startup_cleanup` + commentaire + doc-comment) | 15 |
| `crates/mika-agent/src/async_db.rs` (suppression de `AsyncDatabase::vacuum`) | 5 |
| `crates/mika-agent/src/db.rs` (suppression de `Database::vacuum`) | 6 |
| `crates/mika-agent/src/db/migrations/tests/aucun_vacuum_au_demarrage.rs` (nouveau — prédicat, registre vide, garde, auto-nettoyage, 3 contrôles négatifs) | 150 |
| `crates/mika-agent/src/db/migrations/tests/conversion_auto_vacuum.rs` (nouveau) | 45 |
| `crates/mika-agent/src/db/migrations/tests/mod.rs` (2 déclarations de module) | 2 |

Total estimé : 258 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

Le vol tient sous les deux bornes, et il tient parce qu'il a été **découpé pour
cela** : les trois gardes du correctif retenu totalisent ≈ 735 lignes, soit 1,7
fois la borne par vol. Ce vol en prend une seule — celle qui arrête la
corruption — et son volume est dominé par un fichier de test neuf (150 lignes)
plutôt que par de la réécriture : les quatre modifications de production sont
des **suppressions** (61 lignes au total, dont 35 de prose réécrite), ce qui est
la forme de changement la moins coûteuse en tours de pipeline. Les documents
(`deployment.md`, `schema-version.md`, le document de solution) ne comptent pas
dans le total, mais ils comptent en tours : ils sont écrits après que la suite
de tests soit verte, en un seul passage.

## Ordre de commit

1. Retrait des deux `VACUUM` et des deux méthodes (`migrations.rs`,
   `server/mod.rs`, `async_db.rs`, `db.rs`), paragraphe de doc réécrit.
2. Le détecteur et ses cinq tests, plus `conversion_auto_vacuum.rs`, plus les
   déclarations de module. Dans cet ordre, parce qu'un détecteur posé avant le
   retrait atterrit rouge et qu'on n'a alors aucun moyen de distinguer « il
   fonctionne » de « il est cassé ».
3. `cargo test` complet, `clippy -D warnings`, `cargo fmt`.
4. Les documents, puis `scripts/sync-agent-docs.sh`, puis
   `scripts/verify-pipeline.sh origin/main`.

**Plancher** : si le détecteur demande plus d'ajustement que prévu, le plancher
mergeable est l'étape 1 plus `conversion_auto_vacuum.rs` plus le runbook — le
`VACUUM` est retiré, le fait est épinglé, l'opérateur sait quoi faire. Le
détecteur serait alors renvoyé au vol 2, qui en a de toute façon besoin pour
déclarer son unique site. Ce plancher est un **repli**, pas le plan : le
détecteur est ce qui empêche la réintroduction, et un retrait sans garde a déjà
été réintroduit une fois dans ce dépôt (le `VACUUM` de `startup_cleanup`
précédait mika#2658 et personne ne l'a vu en ajoutant le second).
