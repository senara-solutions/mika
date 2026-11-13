# perf(db) — `search_content` cesse de stocker un embedding qu'elle ne relit jamais (mika#2658)

> Ticket : senara-solutions/mika#2658
> Type : `perf`
> Labels : `enhancement`, `p3-nice-to-have`, `ready`, `dispatch:loop`

## Contexte

L'avertissement de démarrage `database size exceeds 500MB`
(`crates/mika-agent/src/server/mod.rs:807-814`) est exact, et le ticket l'a
mesuré : `mika.db` fait 1 012 338 688 octets, `freelist_count = 0`, un `VACUUM`
ne récupère rien. La croissance est réelle. La répartition `dbstat` du ticket
donne `search_content` à 483 Mo pour 61 677 lignes, soit ≈ 8 Ko par ligne — un
embedding de 512 flottants sérialisé **en JSON texte**.

La lecture du code ajoute le fait décisif, que le ticket n'avait pas :

**`search_content.embedding_json` est une colonne en écriture seule.** Trois
sites la touchent, et aucun ne lit sa valeur :

| site | geste |
|---|---|
| `db.rs:4872-4886` `index_embedding` | **écrit** le JSON, puis passe *la même chaîne* à `vec_search` |
| `db.rs:4917-4925` `get_unembedded_content` | lit `embedding_json IS NULL` — un **marqueur**, jamais la valeur |
| `migrations.rs:751` (`migrate_v1`), `migrations.rs:1982` (v28→v29) | DDL |

Le balayage est exhaustif : `grep -rn embedding_json crates/` ne rend que ces
sites. Hors `crates/`, les seules occurrences sont de la documentation et des
plans passés.

La copie durable qu'emprunte réellement le chemin de recherche est la table
virtuelle `vec_search` (`vec0(embedding float[512])`, créée en
`migrations.rs:1215-1217`), interrogée par `vec_search_internal`
(`db.rs:4968-5002`) en joignant `search_content` **par rowid** — sans jamais
toucher `embedding_json`. vec0 stocke 512 × 4 octets = 2 Ko par vecteur ; le
JSON en coûte ≈ 7,7 Ko. Les 483 Mo mesurés sont donc ≈ 475 Mo de duplication
d'un index que SQLite tient déjà sous forme compacte, plus ≈ 8 Mo de `content`
et d'identifiants.

Deux faits du ticket se corrigent au passage.

**AC2 part d'une prémisse fausse** : `tool_calls` et `llm_calls` ne sont **pas**
gardés indéfiniment. `prune_old_llm_calls` / `prune_old_tool_calls`
(`db.rs:2441-2458`) existent, et `TaskEngine::startup_recovery` les appelle à
chaque démarrage avec une fenêtre de 30 jours (`task_engine/engine.rs:993-1006`,
étape 5). Les 153 + 87 Mo mesurés **sont** le volume de 30 jours, pas une
absence de rétention. Ce qui manque n'est pas un mécanisme : c'est l'écrit qui
le dit, et un test qui empêche la fenêtre de disparaître en silence.

**Le plancher d'après-correctif est au-dessus de 500 Mo.** 1 012 − 475 ≈ 537 Mo.
Le seuil de l'avertissement ne redevient donc pas un signal par le seul effet
d'AC1 : il doit être recalé. C'est exactement la première des deux branches
qu'AC3 propose.

## Décisions de grooming

**D1 — Un marqueur, pas une valeur, et un marqueur qui dit quelque chose.**
`embedding_json TEXT` est remplacée par `embedding_dims INTEGER` (nullable).
`index_embedding` y écrit `embedding.len()`. Un booléen aurait suffi au
prédicat `IS NULL` ; la dimension coûte les mêmes 8 octets et rend lisible une
classe de panne réelle — vec0 est déclarée `float[512]`, donc une ligne dont
l'embedding a une autre dimension a été **refusée** par vec0 (l'insert est
`let _ =`, l'erreur est avalée) tout en paraissant indexée dans
`search_content`. Avec `embedding_dims`, cette divergence est une requête ;
avec un booléen, elle reste invisible.

**D2 — `DROP COLUMN`, pas une reconstruction de table.** rusqlite 0.32
(`Cargo.toml:30`, `bundled`) embarque SQLite 3.46 : `ALTER TABLE … DROP COLUMN`
est disponible depuis 3.35. Les deux conditions de refus sont vérifiées absentes :
`embedding_json` n'est dans aucun index (`idx_search_agent` porte
`(agent_id, source_type)`) et `search_content` n'apparaît pas dans la vue
`unified_timeline` (`db.rs:139-200`). L'idiome de reconstruction complète
(`CREATE _new` / `INSERT SELECT` / `DROP` / `RENAME`, déjà employé en
`migrations.rs:1975-1995`) reste la **contingence nommée** si `DROP COLUMN` est
refusé à l'exécution : même résultat, ≈ 12 lignes de plus.

**D3 — Le `VACUUM` fait partie du livrable, et il est après le commit.**
`DROP COLUMN` réécrit les lignes mais rend les pages libérées à la *freelist*,
pas au système : sans `VACUUM`, le fichier reste à 1 Go et l'avertissement
continue de sonner — le correctif n'aurait aucun effet observable. `VACUUM` ne
peut pas s'exécuter dans une transaction, donc il vient **après**
`tx.commit()`. Cet ordre est load-bearing : la version v56 est estampillée
avant le `VACUUM`, si bien qu'un `VACUUM` en échec (disque plein : il lui faut
jusqu'à ≈ 1 Go de temporaire) laisse le schéma migré et ne fait **pas** rejouer
la migration au démarrage suivant. Un `VACUUM` en échec est un `warn!`, pas une
erreur de démarrage.

**D4 — La migration est l'instrument de mesure d'AC1.** L'implémentation tourne
dans un worktree, pas sur la base de 1 Go : elle ne peut pas mesurer le gain
réel. Ce que la migration peut faire, c'est le mesurer **au déploiement** et
l'écrire : deux lignes de log encadrant le `VACUUM`, portant
`db_size_bytes` avant et après. Le gain sur la base réelle est alors un fait
daté dans le journal de l'opérateur, pas une mesure manuelle à refaire.

**D5 — AC3 prend la branche « recalé », pas la branche « alerte de
croissance ».** Le ticket offre les deux (« recalé sur la taille attendue après
AC1/AC2, **ou** remplacé par une alerte de croissance »). On prend la première,
pour une raison de volume et une de méthode : une alerte de croissance demande
un état persistant (taille précédente + instant), un calcul de delta et un
seuil de *taux* — ≈ 80 lignes qui, ajoutées à AC1, feraient passer ce vol
au-dessus de sa borne ; et son seuil de taux ne peut être calibré qu'**après**
avoir observé le régime permanent qu'AC1 installe. La branche non prise est
donc une branche, pas un report (§ *Hors périmètre*).

**D6 — Aucun détecteur livré, la porte `## Fire-Disposition` est N/A.** Les
quatre tests de ce plan sont comportementaux : ils sèment un état, exécutent un
geste et affirment le résultat. Aucun n'a « aucune violation trouvée » pour
chemin de succès — ni balayage de source, ni règle de lint, ni garde CI. La
section n'est donc pas inventée (règle mika#2306, dernier alinéa).

## Conception

### C1 — Migration v55 → v56

`CURRENT_SCHEMA_VERSION` (`db.rs:35`) passe à 56 ;
`PINNED_SCHEMA_VERSION` (`tests/eval/kg_fixtures/mod.rs:56`) suit.

`migrate_v55_to_v56`, sur le patron verbatim de `migrate_v54_to_v55`
(`migrations.rs:353-382`) — garde d'idempotence, garde de version de départ,
`TransactionBehavior::Immediate` :

```sql
ALTER TABLE search_content ADD COLUMN embedding_dims INTEGER;

UPDATE search_content
   SET embedding_dims = CASE WHEN json_valid(embedding_json)
                             THEN json_array_length(embedding_json) END
 WHERE embedding_json IS NOT NULL;

ALTER TABLE search_content DROP COLUMN embedding_json;
```

Le `CASE WHEN json_valid(…)` n'est pas une précaution de style. `json_array_length`
lève sur un JSON malformé, ce qui ferait échouer la migration entière ; et sans
branche `ELSE`, une ligne malformée reçoit `NULL`, donc **repart dans la file de
backfill** et sera ré-embarquée. C'est le choix conservateur : une ligne dont le
JSON était illisible n'a jamais pu entrer dans vec0, donc la traiter comme
« indexée » (`ELSE 512`) aurait figé un trou dans l'index de recherche.

Puis, dans le même ordre : `INSERT INTO schema_version (version) VALUES (56)`,
`tx.commit()`, et **ensuite** hors transaction :

```rust
let before = self.db_size_bytes().unwrap_or(0);
info!(db_size_bytes = before, "v56: vacuum start (mika#2658)");
match self.conn.execute_batch("VACUUM;") {
    Ok(()) => info!(
        db_size_bytes_before = before,
        db_size_bytes_after = self.db_size_bytes().unwrap_or(0),
        "v56: vacuum complete (mika#2658)"
    ),
    Err(e) => warn!(error = %e, "v56: vacuum failed; schema is migrated, \
                                 space will be reclaimed at the next vacuum"),
}
```

La branche de dispatch dans `migrate()` suit le patron des précédentes
(`migrations.rs:300-303`) : `if (3..=55).contains(&version)`.

Le DDL *clean-slate* de `migrate_v1` (`migrations.rs:745-755`) est édité
— `embedding_json TEXT` devient `embedding_dims INTEGER` — parce que
`migrate_v1` crée le schéma **courant** pour une base neuve (`migrate()` saute
toutes les branches suivantes quand `version == 0`). Les corps historiques
(v28→v29 en `migrations.rs:1975-1995`) ne sont **pas** touchés : ils opèrent sur
le schéma de leur époque, et `db/migrations.rs` est append-only.

### C2 — Les deux sites applicatifs

`index_embedding` (`db.rs:4872`) garde la sérialisation JSON — vec0 l'accepte
comme paramètre d'entrée — mais ne la **stocke** plus :

```rust
pub fn index_embedding(&self, content_id: i64, embedding: &[f32]) -> Result<()> {
    let emb_json = serde_json::to_string(embedding)?;   // transitoire : paramètre vec0
    self.conn.execute(
        "UPDATE search_content SET embedding_dims = ?1,
                updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
          WHERE id = ?2",
        params![embedding.len() as i64, content_id],
    )?;
    // Upsert vec0 — inchangé
}
```

`get_unembedded_content` (`db.rs:4921`) : `WHERE agent_id = ?1 AND
embedding_dims IS NULL`. Le contrat d'idempotence du backfill (documenté en
`docs/architecture/kg-implementation-conventions.md:18-33` et
`docs/solutions/logic-errors/startup-backfill-skips-embedding-generation.md`)
est préservé à l'identique : seul le nom de la colonne-marqueur change.

### C3 — AC3 : le seuil redevient dérivé

Dans `server/mod.rs`, le littéral `500_000_000` (ligne 809) devient une
constante nommée portant sa dérivation :

```rust
/// Plancher mesuré de `mika.db` après la migration v56 (mika#2658), plus une
/// marge de 50 %.
///
/// Dérivation : 1 012 Mo mesurés le 2026-10-03 − ≈ 475 Mo de payload
/// `embedding_json` récupérés par la v56 ≈ 540 Mo de plancher, dont ≈ 240 Mo
/// sont les fenêtres de 30 jours de `tool_calls` + `llm_calls`
/// (`task_engine/engine.rs:993-1006`). × 1,5 ≈ 800 Mo.
///
/// Le 500 Mo d'origine était franchi en permanence : un seuil qu'on ne
/// repasse jamais sous n'alerte sur rien. Si la ligne `v56: vacuum complete`
/// du premier démarrage après déploiement donne un plancher qui s'écarte de
/// plus de 20 % de 540 Mo, corriger ce nombre ici — la dérivation est écrite
/// pour qu'il soit recalculable, pas pour qu'il soit cru.
const DB_SIZE_WARN_BYTES: u64 = 800_000_000;
```

L'avertissement gagne le seuil dans ses champs, pour qu'il soit lisible sans
avoir le code sous les yeux :

```rust
Ok(size) if size > DB_SIZE_WARN_BYTES => {
    warn!(size_bytes = size, threshold_bytes = DB_SIZE_WARN_BYTES,
          "database size exceeds threshold");
}
```

### C4 — AC2 : l'écrit, et la garde qui l'empêche de pourrir

La réponse à AC2 est un fait, pas une décision : rétention de 30 jours, câblée
à l'étape 5 de `startup_recovery`. Elle part dans
`docs/agent-internals/schema-version.md` (hors compte de lignes) et dans un
commentaire à l'appel de `prune_old_llm_calls` nommant mika#2658.

Un écrit seul pourrirait : rien ne redit la fenêtre si quelqu'un retire l'appel.
Un test comportemental la tient — deux lignes semées, une vieille et une
récente, dans chacune des deux tables ; après `prune_old_*(30 j)`, la vieille
est partie et la récente est là. Il n'affirme pas le **câblage** (faire tourner
`startup_recovery` coûterait un harnais de task engine complet) : cette limite
est dite dans le doc-comment du test, et le commentaire au site d'appel est ce
qui relie les deux.

## Étapes d'implémentation

1. `db.rs` — `CURRENT_SCHEMA_VERSION` 55 → 56.
2. `db/migrations.rs` — `migrate_v55_to_v56` (C1) + branche de dispatch dans
   `migrate()`.
3. `db/migrations.rs` — DDL *clean-slate* de `migrate_v1` :
   `embedding_json TEXT` → `embedding_dims INTEGER`.
4. `db.rs` — `index_embedding` et `get_unembedded_content` (C2).
5. `tests/eval/kg_fixtures/mod.rs` — `PINNED_SCHEMA_VERSION` 55 → 56.
6. `server/mod.rs` — `DB_SIZE_WARN_BYTES` + le `warn!` (C3).
7. `db/migrations/tests/mod.rs` — les trois tests de migration (V1–V3 du
   contrat de vérification).
8. `db/tests/tool_calls_and_messages.rs` — le test de rétention (V4).
9. `docs/agent-internals/schema-version.md` — entrée v55→v56 + bascule du
   « Current: v55 » ; `docs/runtime-structure.md:140` — la colonne ;
   `docs/architecture/kg-implementation-conventions.md:18-33` — le marqueur.
10. `cargo fmt`, `cargo clippy`, `cargo test -p mika-agent`,
    `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`.

## Contrat de vérification

**V1 — `mika2658_migration_v55_to_v56_replaces_payload_with_marker`**
(`db/migrations/tests/mod.rs`). Fabrique l'état v55 depuis `db()` : rajoute
`embedding_json TEXT`, retire la ligne de version 56, sème trois lignes dans
`search_content` — un JSON de 512 flottants valide, un JSON malformé, un `NULL`.
Appelle `migrate_v55_to_v56`. Affirme : la colonne `embedding_json` est absente
de `PRAGMA table_info(search_content)` ; la ligne valide porte
`embedding_dims = 512` ; la malformée et la `NULL` portent `embedding_dims IS
NULL` — donc repartent dans la file de backfill ; la version est 56.

**V2 — `mika2658_migration_v55_to_v56_is_idempotent`**. Patron verbatim de
`mika1833_migration_v54_to_v55_is_idempotent` : deux rejeux sur une base déjà
en v56, `Ok` les deux fois, version inchangée.

**V3 — `mika2658_fts_search_survives_the_column_drop`**. `search_content` est la
table de contenu externe de `fts_search`
(`fts5(content, content='search_content', content_rowid='id')`,
`migrations.rs:1213-1214`). C'est le seul test capable de prouver que
`DROP COLUMN` n'a pas cassé ce couplage — une assertion sur `table_info` ne
regarde pas du bon côté. Indexe un contenu, migre, puis `fts_search MATCH` doit
toujours rendre la ligne.

**V4 — `mika2658_llm_and_tool_calls_have_a_thirty_day_window`**
(`db/tests/tool_calls_and_messages.rs`). Deux lignes par table — une à 31 jours,
une à 1 jour. Après `prune_old_llm_calls(30 j)` et `prune_old_tool_calls(30 j)` :
la vieille est partie, la récente est là. C'est la réponse d'AC2 rendue
exécutable.

**V5 — sonde de déploiement (opérateur, post-merge).** Au premier démarrage
après `make deploy`, le journal doit porter `v56: vacuum complete` avec
`db_size_bytes_before ≈ 1 012 000 000` et `db_size_bytes_after` ≈ 540 000 000.
C'est la mesure du gain réel exigée par AC1. Au démarrage suivant,
`database size exceeds threshold` doit avoir **disparu** ; son retour signifie
que le plancher réel est au-dessus de 800 Mo et que `DB_SIZE_WARN_BYTES` doit
être corrigé (la dérivation est écrite pour ça). L'absence de la ligne
`v56: vacuum complete` sur une base qui était à 1 Go signifie un binaire servi
antérieur au correctif, jamais « la migration n'avait rien à faire ».

## Risques nommés

**R1 — `DROP COLUMN` refusé à l'exécution.** Les deux conditions de refus sont
vérifiées absentes (D2), mais la vérification est statique. Contingence :
l'idiome de reconstruction complète de `migrations.rs:1975-1995`, ≈ 12 lignes.
V1 est ce qui rougit.

**R2 — `VACUUM` long, et bloquant au démarrage.** Un `VACUUM` sur ≈ 1 Go
immobilise `open()` de l'ordre de la dizaine de secondes à la minute, une seule
fois (au franchissement v55→v56). Coût accepté : sans lui, le livrable ne rend
aucun octet et l'avertissement continue de sonner. Il lui faut aussi jusqu'à
≈ 1 Go de temporaire ; un échec est un `warn!` après un schéma déjà estampillé
(D3), donc non rejouable et non bloquant.

**R3 — la copie JSON était la seule source de reconstruction de vec0.** Après la
v56, reconstruire `vec_search` demande de ré-appeler l'API d'embedding. Aucun
chemin de production ne la reconstruit aujourd'hui (`grep -n vec_search
crates/mika-agent/src/db.rs crates/mika-agent/src/db/migrations.rs` : un `DROP`
dans `migrate_v1`, un `CREATE IF NOT EXISTS`, trois usages de requête). Le seul
lecteur *envisagé* est le plan `docs/plans/2026-06-15-001-feat-1538-ascension-slice-2-signed-bundle-plan.md`
(§ L3), qui proposait de reporter `embedding_json` dans un bundle pour éviter la
dépense de ré-embedding — **ce code n'existe pas** (`crates/mika-agent/src/bundle/`
ne contient ni `search_content` ni `embedding`, et `slices/` n'existe pas). Si
cette tranche est reprise, elle reportera `vec_search` plutôt que le JSON : vec0
expose ses vecteurs, et 2 Ko par ligne au lieu de 7,7 Ko est aussi son intérêt.

**R4 — lignes dont la dimension n'est pas 512.** vec0 est déclarée
`float[512]` ; une ligne d'une autre dimension a été refusée silencieusement par
vec0 (`let _ =`, `db.rs:4880`) tout en paraissant indexée. `embedding_dims` rend
cette divergence interrogeable (`SELECT count(*) FROM search_content WHERE
embedding_dims NOT IN (512)`), ce qui est le seul apport de D1 sur un booléen.
Réparer ces lignes n'est pas dans ce vol : rien ne dit aujourd'hui qu'il en
existe, et la requête est ce qui permettra de le savoir.

## Hors périmètre

**L'alerte de croissance.** AC3 offre deux formes — « recalé **ou** remplacé par
une alerte de croissance » — et ce vol prend la première (D5). La seconde n'est
pas du travail reporté : c'est la branche non prise de l'alternative que le
ticket pose. Elle n'a d'ailleurs pas de seuil calibrable avant qu'on ait observé
le régime permanent qu'AC1 installe — la sonde V5 produit précisément cette
observation. Si, après déploiement, le plancher réel invalide les 800 Mo, le
geste est d'abord la correction d'une constante, pas un nouveau mécanisme.

**Réduire les fenêtres de 30 jours de `tool_calls` / `llm_calls`.** 240 Mo pour
30 jours d'observabilité est un arbitrage déjà rendu
(`task_engine/engine.rs:993`) ; le resserrer est une décision d'opérateur, pas
une conséquence de ce ticket. AC2 demande de le **dire**, ce que fait C4.

**Les 38 Mo d'`audit_events` et les 28 Mo de `messages`.** Les premiers sont
déjà compactés (`compact_old_audit_events`, `server/mod.rs:793-805`) ; les
seconds sont la mémoire de conversation, bornée par la compaction. Ni l'un ni
l'autre n'est nommé par un AC.

## Definition of Done

- [ ] `migrate_v55_to_v56` ajoute `embedding_dims`, reporte la dimension depuis
      le JSON quand il est valide, retire `embedding_json`, estampille 56, puis
      `VACUUM` hors transaction avec ses deux lignes de log.
- [ ] `migrate_v1` crée `search_content` avec `embedding_dims`, sans
      `embedding_json`.
- [ ] `index_embedding` écrit `embedding.len()` et ne stocke plus de JSON ;
      l'upsert vec0 est inchangé.
- [ ] `get_unembedded_content` filtre sur `embedding_dims IS NULL`.
- [ ] `CURRENT_SCHEMA_VERSION` et `PINNED_SCHEMA_VERSION` valent 56.
- [ ] `DB_SIZE_WARN_BYTES` remplace le littéral et porte sa dérivation ; le
      `warn!` porte `threshold_bytes`.
- [ ] V1 à V4 passent ; `cargo test -p mika-agent` est vert.
- [ ] `cargo fmt --check`, `cargo clippy` sans avertissement nouveau,
      `make verify-bundled-skills`, `scripts/verify-pipeline.sh origin/main`.
- [ ] `docs/agent-internals/schema-version.md` porte l'entrée v55→v56 et
      « Current: v56 » ; `docs/runtime-structure.md` et
      `docs/architecture/kg-implementation-conventions.md` nomment
      `embedding_dims`.
- [ ] Le corps de PR porte la sonde V5 (ce que l'opérateur doit lire au premier
      démarrage) et la migration manuelle requise : aucune, le `VACUUM` est
      automatique — mais le premier démarrage est plus long d'une dizaine de
      secondes à une minute.
- [ ] `Code review: delegated to mika-qa (mika#2694)` dans le corps de PR.

## Acceptance criteria

- [ ] **AC1.** Une politique de taille pour `search_content` est décidée et
      documentée : **stockage compact**, en retirant la duplication plutôt qu'en
      la ré-encodant — l'embedding est déjà tenu sous forme compacte par vec0
      (2 Ko/ligne), et `embedding_json` (7,7 Ko/ligne) n'en était qu'une copie
      jamais relue, remplacée par un marqueur `embedding_dims INTEGER`.
      Migration v55→v56 comprise, `VACUUM` compris. Le gain sur la base réelle
      est mesuré par la ligne `v56: vacuum complete`
      (`db_size_bytes_before` / `_after`) au premier démarrage après
      déploiement — gain attendu ≈ 475 Mo, soit ≈ 540 Mo de base.
      Aucune rétention par âge n'est ajoutée : la file est un index de
      recherche, pas un journal, et supprimer une ligne par âge retirerait de
      la mémoire interrogeable sans que rien ne le demande.
- [ ] **AC2.** Dit explicitement : `tool_calls` et `llm_calls` **ne sont pas**
      gardés indéfiniment. La rétention est de 30 jours, appliquée à chaque
      démarrage (`task_engine/engine.rs:993-1006`, étape 5) ; les 153 + 87 Mo
      mesurés **sont** le volume de 30 jours. L'écrit part dans
      `docs/agent-internals/schema-version.md` et dans un commentaire au site
      d'appel ; V4 l'empêche de pourrir en silence.
- [ ] **AC3.** Le seuil redevient un signal par recalage (branche 1 des deux
      que le ticket propose) : `DB_SIZE_WARN_BYTES = 800_000_000`, dérivé du
      plancher d'après-AC1 (≈ 540 Mo) × 1,5, la dérivation écrite dans le
      doc-comment pour être recalculable. Le `warn!` porte désormais son seuil.
      La branche « alerte de croissance » est la branche non prise, pas un
      report (§ *Hors périmètre*).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/migrations.rs` (`migrate_v55_to_v56` + dispatch + DDL `migrate_v1`) | 70 |
| `crates/mika-agent/src/db.rs` (`index_embedding`, `get_unembedded_content`, `CURRENT_SCHEMA_VERSION`) | 20 |
| `crates/mika-agent/src/server/mod.rs` (`DB_SIZE_WARN_BYTES` + `warn!`) | 25 |
| tests de migration (`crates/mika-agent/src/db/migrations/tests/mod.rs`, V1–V3) | 55 |
| test de rétention (`crates/mika-agent/src/db/tests/tool_calls_and_messages.rs`, V4) | 35 |
| `crates/mika-agent/tests/eval/kg_fixtures/mod.rs` (pin v56) | 2 |

Total estimé : 207 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et voici pourquoi il tient.** Les trois AC ferment ensemble à 207
lignes, sous la borne de 243 et très loin du seuil par PR de 1 000. Ce n'est pas
un périmètre resserré par chance : il l'est par D5, qui prend d'AC3 la branche
« recalé » (une constante dérivée, 25 lignes) au lieu de la branche « alerte de
croissance » (état persistant + delta + seuil de taux, ≈ 80 lignes) — laquelle,
en plus de faire franchir la borne, n'est pas calibrable avant que la sonde V5
ait donné le régime permanent. Les trois AC se ferment aussi dans **un seul**
vol par nécessité et pas seulement par économie : AC3 n'est pas calculable sans
AC1 (le plancher dont le seuil est dérivé **est** le résultat d'AC1), et le
chiffre d'AC2 (240 Mo pour 30 jours) entre dans cette même dérivation. Les
découper aurait livré, au premier vol, un correctif de 475 Mo sous un seuil
toujours franchi en permanence — c'est-à-dire la moitié visible du ticket
laissée ouverte.
