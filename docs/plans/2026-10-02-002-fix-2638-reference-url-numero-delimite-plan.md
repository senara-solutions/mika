# mika#2638 — `reference_url` est comparée à un numéro délimité, jamais à un préfixe

**Ticket :** senara-solutions/mika#2638
**Parent :** mika#2161 (fermé par PR #2635)
**Origine :** revue de PR #2635, constat P3 — défaut **préexistant**, rendu visible
par le message (b) que #2161 a livré.

---

## 1. Ce que la lecture du code déplace dans le ticket

Le ticket est **juste sur le défaut** et le recensement le confirme à la ligne
près. Trois choses que la lecture ajoute, et elles changent la forme du remède.

**(R1) Le défaut tient à DEUX sites SQL, pas un.** Le ticket parle de « la sonde
des tâches en vol » au singulier. Il y en a deux, dans le même fichier, qui
posent deux questions différentes sur la même table :

- `find_active_self_dev_task_for_issue` (`db/tasks.rs:1735`) — *« une tâche
  self_dev active référence-t-elle ce ticket ? »*, lue par `auto_pull` Phase 0
  (comptage *pullable* + exclusion de promotion), Phase 2 filtre 4, et la sonde
  post-dispatch. `has_active_self_dev_task_for_issue` en est **dérivée** (un seul
  site SQL, mika#2161 U3) — donc réparer le premier répare le second, sans geste.
- `find_dispatch_children_for_issue_url` (`db/tasks.rs:2256`) — *« un pilote
  est-il vif pour ce ticket ? »*, le lecteur unique de `live_pilot` (mika#2279),
  lu par la porte 2c du `ready_label_handler` et par le filtre 4b de Phase 2.

Un correctif sur le seul site de la sonde de vol laisserait la porte 2c de
mika#2279 refuser un `labeled ready` **au motif du pilote d'un autre ticket**.
Le rayon de souffle du second site est plus large que celui du premier.

**(R2) La troisième option de l'AC2 — « une colonne `issue_number` si elle
existe » — n'existe pas.** `tasks` ne porte que `reference_url TEXT`
(`db/migrations.rs:521`) ; `issue_number` n'existe que sur `worktree_claims`,
`qa_review_*` et leurs sœurs. L'AC2 se réduit donc à ses deux premières options,
et le dépôt a déjà tranché laquelle : `find_active_tracking_rows_by_reference_url_and_variants`
(`db/tasks.rs:1215`, mika#1934) fait `reference_url IN (?2, ?3)` — **énumération
exacte de l'ensemble des variantes**. C'est le modèle, il est à trois lignes du
défaut, et il porte déjà son doc-comment.

**(R3) Le préfixe `LIKE` porte DEUX élargissements de plus que celui que le
ticket nomme**, tous deux fermés gratuitement par l'égalité :

- **`_` est un joker `LIKE` d'un caractère** (le piège que `prompt.rs:361`
  documente). GitHub autorise `_` dans un nom de dépôt ; une sonde pour
  `…/my_repo/issues/42` matcherait `…/myXrepo/issues/42`. Population **vide
  aujourd'hui** (`DEFAULT_REPO` = `senara-solutions/mika`, sans `_`), réelle
  demain.
- **`LIKE` est insensible à la casse en ASCII** alors que `reference_url` est
  déclarée `TEXT` sans `COLLATE NOCASE`. Conséquence de performance, pas de
  justesse : un `LIKE 'préfixe%'` insensible à la casse **ne peut pas** utiliser
  `idx_tasks_manual_active_ref_url`, là où un `IN (…)` peut. La sonde tourne
  jusqu'à ~50 fois par tick de 10 min (`FEEDER_WORKING_SET_CAP`) : le correctif
  est aussi, accessoirement, moins cher.

---

## 2. Recensement (AC1) — le livrable, pas une annexe

Grep consigné, à recopier dans le corps de la PR :

```bash
grep -rn "reference_url" crates/ --include="*.rs" | grep LIKE
grep -rn "LIKE" crates/ --include="*.rs" | grep -v "/tests/" \
  | grep -viE "input LIKE|output LIKE|id LIKE|session_id LIKE|canonical_name LIKE|relationship LIKE|notes LIKE|description LIKE|category LIKE|value LIKE|context LIKE|content LIKE|NOT LIKE|escape|metachar"
grep -rn "format!(\"{}%\"" crates/ --include="*.rs"
```

| # | site | prédicat | dans la population ? |
|---|---|---|---|
| 1 | `db/tasks.rs:1735` `find_active_self_dev_task_for_issue` | `reference_url LIKE '<url>%'` | **OUI — le défaut** |
| 2 | `db/tasks.rs:2256` `find_dispatch_children_for_issue_url` | `parent.reference_url LIKE '<url>%'` | **OUI — le défaut** |
| 2b | `db/tasks.rs:1757` `has_active_self_dev_task_for_issue` | **aucun SQL** — dérivée de #1 | couverte par #1, sans geste |
| 3 | `db/tasks.rs:1215` `find_active_tracking_rows_by_reference_url_and_variants` | `reference_url IN (?2, ?3)` | non — **déjà correct**, et c'est le modèle de l'AC2 |
| 4 | `db/tasks.rs:1132` `find_active_task_by_ref_url` | `reference_url = ?2` | non — égalité stricte |
| 5 | `db/tasks.rs:1523` (anomalies de santé) | `reference_url LIKE '%github.com%'` | non — prédicat sur le **domaine**, aucun numéro en jeu |
| 6 | `db/tasks.rs:615` (résolution d'id abrégé) | `id LIKE ?1 \|\| '%'` | non — préfixe d'UUID : la fuzziness **est** la fonction |
| 7 | `db/tasks.rs:4673` `list_tasks --label` | `label LIKE '%' \|\| ?n \|\| '%'` | non — filtre de **recherche opérateur** ; voir § 4 |
| 8 | `upstream_close_handler.rs:70` `degroom_marker_key` | `target_key =` exact | non — et son doc-comment **est** l'art antérieur (`#234` vs `#2343`, piège mika#2347) |
| 9 | `wip_rescue.rs:207` `hold_audit_key` | `pr:…#N@motif` — préfixe **borné par `@`** | non — le séparateur délimite, et le doc le dit |
| 10 | `worktree_reaper.rs:460` | `target_key LIKE '…@%'` | non — même séparateur `@` |
| 11 | `live_pilot.rs:318` (test) | assertion qui **épingle** le préfixe | **OUI** — l'assertion et son message doivent changer |
| 12 | `skills/bundled/**/system_prompt.md` | appariement `list_tasks` **par le LLM** | non — aucun SQL ; voir § 4 |

**Deux faits à lire dans ce tableau.** Les sites 8, 9 et 10 montrent que la
maison a déjà tranché cette classe **trois fois**, chaque fois en délimitant —
et le site 8 écrit le piège mot pour mot. Les sites 1 et 2 sont les deux seuls
qui ne l'ont pas appliqué. Et le site 3 montre que le remède exact existe dans le
même fichier : il n'y a rien à inventer, seulement à aligner.

---

## 3. Ce qui est livré

### U1 — Un site de définition unique de l'ensemble des variantes

`crates/mika-agent/src/task_state/tasks.rs`, à côté de `GROOM_PHASE_SUFFIX` et
`strip_groom_phase_suffix` :

```rust
/// Les deux — et seulement les deux — écritures de `reference_url` qui
/// désignent une issue donnée (mika#2638).
///
/// `base_url` DOIT être canonique (sans `?phase=groom`) ; les appelants le
/// nettoient via [`strip_groom_phase_suffix`]. Même contrat, et même raison, que
/// `Database::find_active_tracking_rows_by_reference_url_and_variants`.
pub fn issue_url_variants(base_url: &str) -> [String; 2];
```

Les **trois** sites SQL (1, 2 et 3) passent par lui. Trois appelants pour une
définition : c'est ce qui empêche deux requêtes d'épeler différemment l'ensemble
des variantes — la leçon que `grooming_marker` a dû graver une fois (mika#2158,
où promotion et routage répondaient différemment à la même question pendant des
mois sans que rien ne casse) et que `live_pilot` a payée une seconde fois
(mika#2335).

Son doc-comment porte le **recensement du § 2** : une liste qui vit à côté du
code qu'elle décrit pourrit moins vite qu'une liste qui vit dans un corps de PR.

### U2 — Les deux sites défectueux comparent un numéro délimité

```sql
-- site 1, find_active_self_dev_task_for_issue
AND reference_url IN (?2, ?3)
-- site 2, find_dispatch_children_for_issue_url
AND parent.reference_url IN (?2, ?3)
```

Le site 3 est **réécrit à travers le helper sans changement de comportement**
(sa sortie est octet pour octet la même), épinglé par test : le but est que les
trois sites ne puissent plus diverger, pas de modifier celui qui était juste.

### U3 — Les doc-comments qui affirment le contraire sont corrigés

Quatre narratifs disent aujourd'hui « prefix `LIKE` so the `?phase=groom`
variant is covered » et deviennent faux :

- `db/tasks.rs:1703` (site 1), `db/tasks.rs:2205` et `:2212` (site 2) ;
- `live_pilot.rs:24` et `:86` (le module doc du lecteur unique) ;
- `live_pilot.rs:318` — le **message d'assertion** `"le LIKE préfixe doit
  couvrir la variante ?phase=groom"`. La variante reste couverte, par
  l'énumération et non par le préfixe. Ce message est porteur : remis en l'état,
  il réinstalle le modèle mental qui a produit le défaut.

### U4 — Les détecteurs

- **D1 (AC3).** Deux tests au niveau DB, un par site : une tâche de `#2161`
  seedée, la sonde de `#216` rend `None` / `[]`. **Vu rouge** avant U2 (le
  préfixe matche). Plus le contrôle positif : une tâche de `#216` est bien
  trouvée.
- **D2.** Délimitation complète, en table : `216` vs `{216, 2160, 21600, 2161}`,
  `216?phase=groom` (couvert), `216/` et `216#issuecomment-1` (**non** couverts —
  voir § 5), et le cas `_` de R3 (dépôt `my_repo`, **vu rouge** sur l'ancien
  `LIKE`).
- **D3.** Extension du test d'équivalence existant
  `mika2161_le_booleen_en_vol_delegue_au_lecteur_unique` (`auto_pull.rs:9029`)
  avec un voisin numérique : les deux réponses doivent **l'exclure ensemble**.
- **D4.** Scan de source, `db/tasks.rs` et ses sœurs sous `crates/mika-agent/src/db/` :
  aucune ligne ne conjugue `reference_url` et `LIKE`, et aucun
  `format!("{…}%", issue_url)` ne reconstruit un préfixe. Anti-vacuité : le scan
  doit **voir** une population non vide (les trois sites `IN`), sans quoi un scan
  devenu aveugle se lit exactement comme un arbre propre (classe mika#2205).
  Allowlist : **une entrée déclarée** (§ 6).
- **D5.** `issue_url_variants` a **≥ 3** sites d'appel — la garde qui empêche
  qu'un site soit réparé à la main et sorte du lecteur unique.

---

## 4. L'arbitrage de conception, et l'option déclinée

Deux formes satisfont l'AC2. Elles ne sont pas équivalentes et le choix se
justifie.

**Option A — énumération exacte, `reference_url IN (base, base‖'?phase=groom')`. RETENUE.**

**Option B — garde non-chiffre, `reference_url = ?2 OR reference_url GLOB ?3`
avec `?3 = base‖'[^0-9]*'`.** Déclinée, pour quatre raisons de poids décroissant :

1. **L'index de dédup est DÉJÀ une égalité stricte.**
   `idx_tasks_manual_active_ref_url` est `UNIQUE ON tasks(agent_id,
   reference_url)` : le système tient déjà l'invariant « l'ensemble des variantes
   est clos et exact », et c'est cet index qui garantit qu'une issue n'a qu'une
   tâche active. Une sonde **plus permissive que son propre index de dédup** est
   l'incohérence : une variante que le GLOB tolérerait est précisément une
   variante qui a déjà créé une seconde ligne active pour la même issue, donc un
   problème plus grave et en amont. L'option A fait coïncider les sondes avec
   l'invariant ; l'option B les laisse plus larges que lui.
2. **Précédent dans le même fichier, sur la même question.** Le site 3 fait
   déjà `IN (?2, ?3)` depuis mika#1934, avec son raisonnement écrit. Trois sites
   sous un motif battent deux sites sous deux motifs.
3. **`GLOB` n'a aucun précédent dans le dépôt** et porte un risque de syntaxe
   réel : SQLite nie une classe de caractères par `^`, pas par `!`, donc
   `[!0-9]` serait lu comme l'ensemble littéral `{!, 0..9}` — un prédicat
   silencieusement inversé, qui matcherait `2160` et dont aucune erreur de
   compilation n'avertirait.
4. **`GLOB` ne peut pas utiliser l'index**, là où `IN` peut (R3).

Le seul avantage de B est de tolérer un **troisième suffixe inconnu**. Il est
payé par le § 5 et sa sonde, pas par un prédicat plus large.

**Deux sites hors population, nommés plutôt que découverts.**

- **Site 7, `label LIKE '%…%'`** (`mika tasks list --label`) : même classe —
  chercher `mika#216` rend les lignes de `mika#2161`. Hors périmètre, et le motif
  est que c'est un **filtre de recherche opérateur** dont la fuzziness est la
  fonction, et qu'**aucune décision moteur ne le lit**. Le resserrer casserait
  la recherche par sous-chaîne partielle. **Suivi**, précondition : une mesure
  montrant qu'un opérateur a été trompé par ce filtre.
- **Site 12, les prompts.** L'appariement `reference_url` dans
  `skills/bundled/self-dev*/system_prompt.md` est fait par le LLM sur la sortie
  de `list_tasks`, sans SQL. Hors du périmètre du ticket, et la moitié qui tient
  ne serait de toute façon pas là
  (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`).

---

## 5. Ce que ce travail n'achète PAS

- **Il ne tolère pas un troisième suffixe.** Passer du préfixe à l'énumération
  **rétrécit** : une ligne portant `…/issues/2638/` ou
  `…/issues/2638#issuecomment-1` était vue en vol hier et ne l'est plus. C'est le
  coût réel de l'option A et il est **nommé, borné et sondé** (§ 8 S1) plutôt
  que découvert. Borné parce qu'une telle ligne est **déjà** hors de l'index de
  dédup, donc déjà un défaut en amont et plus grave que celui-ci. Et l'unique
  écrivain non-`format!` est l'outil `create_task`, dont le prompt ne prescrit
  que les deux variantes déclarées.
- **Il rétrécit aussi sur la casse.** `LIKE` était insensible à la casse, `IN`
  ne l'est pas. Population vide par construction : tous les écrivains bâtissent
  l'URL par `format!` à partir de littéraux minuscules et de `DEFAULT_REPO` /
  `location.owner_repo()`, qui portent la casse canonique de GitHub. Nommé parce
  qu'invisible sinon.
- **Il ne répare aucune ligne existante.** Aucune migration, aucune réécriture
  de `reference_url` : les lignes portant une variante non déclarée restent là et
  sortent simplement de la population des sondes.
- **Il ne rétro-corrige aucun message (b) déjà émis.** Les lignes
  `auto_feeder_pool_in_flight` déjà écrites nomment ce qu'elles nommaient, et
  **rien ne les réécrit** — réécrire rendrait faux ce qu'elles ont dit au moment
  où elles ont été écrites (motif mika#2361). La sonde est la **prochaine**
  occurrence.
- **Il n'ajoute AUCUNE surface d'observabilité : ni événement, ni compteur, ni
  ligne d'audit.** Le défaut est un faux appariement, pas un silence : il n'y a
  rien à émettre. Les seuls instruments sont les tests de D1/D2 et la requête
  opérateur de S1, et **le silence de cette requête ne prouve rien tant que
  personne ne l'exécute**.
- **Il ne ferme ni le site 7 ni le site 12** (§ 4).

---

## 6. Fire-Disposition

Ce plan livre des détecteurs (D1 à D5). Disposition : **(a) — exception nommée
en allowlist**, avec **une seule entrée déclarée**.

Le scan D4 refuse toute conjonction de `reference_url` et `LIKE` sous
`crates/mika-agent/src/db/`. Une occurrence légitime subsiste après le correctif
et reçoit son entrée :

```rust
/// Les lignes `reference_url` + `LIKE` que le scan D4 tolère, avec leur donnée
/// précise et sa raison. **Quand D4 tire sur un site neuf, on DÉLIMITE ce
/// site ; on n'ajoute pas de ligne ici** (doctrine mika#2201).
const REFERENCE_URL_LIKE_ALLOWED: &[(&str, &str)] = &[(
    "reference_url LIKE '%github.com%'",
    "anomalies de santé (`db/tasks.rs`, « cette tâche manuelle est-elle liée à \
     GitHub ? ») — prédicat sur le DOMAINE, aucun numéro d'issue en jeu, donc \
     hors de la population de mika#2638. Pas de ticket de suivi : ce site est \
     correct, pas toléré.",
)];
```

**Assertion auto-nettoyante :** le scan échoue si une entrée de l'allowlist
n'apparaît plus dans l'arbre — une exception périmée exempterait silencieusement
un futur homonyme. Elle rougit le jour de la réparation, pas des mois après.

**Les détecteurs sont livrés ARMÉS, sans `#[ignore]`.** Après U2 il reste **zéro
violation** : les deux sites sont réparés dans le même commit, donc l'option (b)
n'aurait rien à désarmer et l'option (c) rien à faire remonter.

---

## 7. Contrat de vérification

| # | vérification | commande / forme | critère |
|---|---|---|---|
| V1 | AC3, les deux sites, **vu rouge** | `cargo test -p mika-agent mika2638` sur l'arbre **avant** U2 | les deux tests échouent : la sonde de `#216` trouve la tâche de `#2161` |
| V2 | AC3, contrôle positif | idem après U2 | la sonde de `#216` trouve bien la tâche de `#216` |
| V3 | non-régression mika#1934 / mika#2279 | `the_groom_phase_url_variant_is_covered`, `find_active_tracking_rows_…` | la variante `?phase=groom` reste couverte sur les **trois** sites |
| V4 | délimitation complète | table D2 | `2160`, `21600`, `2161` exclus ; `216` inclus ; cas `_` **vu rouge** avant U2 |
| V5 | lecteur unique préservé | `mika2161_le_booleen_en_vol_delegue_au_lecteur_unique` étendu | les deux réponses excluent le voisin **ensemble** |
| V6 | D4 regarde quelque chose | contrôle négatif : réintroduire un `LIKE` préfixe dans une fixture | le scan rougit ; puis **vu vert** sur l'arbre réparé, population ≥ 3 |
| V7 | D5 | compte des appelants de `issue_url_variants` | **≥ 3** |
| V8 | suite complète | `cargo test -p mika-agent`, `cargo clippy --all-targets`, `cargo fmt --check` | vert |

**V1 et V4 sont les deux seules vérifications dont le « rouge d'abord » est
non négociable** : sans lui, un test qui passe sur l'arbre réparé ne distingue
pas « le prédicat délimite » de « la fixture ne pose pas la question ».

---

## 8. Sondes post-déploiement, et leurs quatre haltes

> **Préalable.** Ces mesures décrivent le **binaire servi**. Après `make deploy`,
> établir que le `mika-spirit` qui tourne porte le correctif avant toute
> conclusion (classe mika#2340). Ce sont des **gestes d'opérateur sur l'hôte** :
> `~/.mika/data/mika.db` n'est pas montée dans le bac à sable de dispatch, donc
> aucune de ces requêtes n'est exécutable par un pilote.

**S1 — l'ensemble des variantes est bien clos (une fois, après déploiement).**
C'est la sonde du rétrécissement nommé au § 5 :

```sql
-- Toute écriture de `reference_url` d'issue, groupée par son SUFFIXE réel.
SELECT ltrim(substr(reference_url, instr(reference_url, '/issues/') + 8),
             '0123456789') AS suffixe,
       count(*)
  FROM tasks
 WHERE reference_url IS NOT NULL
   AND instr(reference_url, '/issues/') > 0
 GROUP BY 1 ORDER BY 2 DESC;
```

Attendu : **exactement deux lignes**, `''` et `?phase=groom`.
*Halte 1 — une troisième ligne apparaît.* **Ne pas élargir le prédicat par
réflexe.** Cette ligne est aussi, et d'abord, une ligne que
`idx_tasks_manual_active_ref_url` traite comme distincte de sa sœur canonique :
la dédup est donc déjà cassée pour cette issue, et c'est **ce défaut-là** qu'il
faut établir en premier. Lire ensuite quel écrivain l'a produite (`create_task`
est le seul candidat non-`format!`) ; le remède est au site d'écriture, pas au
prédicat de lecture.

**S2 — le défaut fondateur ne se rejoue pas (30 jours).**

```bash
grep auto_feeder_pool_in_flight "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{raw_ready, pullable, in_flight, stuck}'
```

Chaque entrée de `stuck` doit nommer une tâche **du ticket de sa propre ligne**.
*Halte 2 — une tâche étrangère y figure encore.* Le prédicat réparé n'est pas
celui que cette ligne lit : établir **par quel chemin** le recensement a été
construit (`probe_in_flight` → `find_active_self_dev_task_for_issue`) avant de
toucher à quoi que ce soit.

**S3 — CONTRÔLE POSITIF, et il décide de la lecture de S2 (30 jours).**

```bash
grep -c auto_feeder_pool_in_flight "$MIKA_SPIRIT_LOG_FILE"
grep -c ready_label_pilot_in_flight "$MIKA_SPIRIT_LOG_FILE"
```

Les deux comptes doivent **rester non nuls**. Zéro ligne en vol **et** zéro faux
appariement ne prouve rien : ça se lit exactement comme un rétrécissement qui a
vidé la population (classe mika#2205).
*Halte 3 — l'un des deux tombe à zéro alors que des dispatches tournent.* Le
rétrécissement est allé trop loin : **désarmer d'abord** (revert de U2),
diagnostiquer ensuite — une sonde de vol aveugle produit des dispatches en
double, c'est-à-dire la boucle mika#2279.

**S4 — contrôle négatif de la porte 2c (30 jours).** Aucun
`ready_label_pilot_in_flight` sur un ticket dont aucun pilote ne tourne.
*Halte 4 — une occurrence.* C'est le faux positif du site 2, et son coût est un
ticket gelé : lire le `parent_task_id` de la ligne, il nomme le ticket réellement
en vol.

**Halte transverse — les sondes muettes.** Il faut qu'un tick de feeder et qu'un
`labeled ready` aient eu lieu depuis le déploiement. Vérifier le contrôle positif
S3 avant toute conclusion.

---

## 9. Hors périmètre, délibérément

- **Le site 7** (`label LIKE '%…%'`, recherche opérateur) et le **site 12** (les
  prompts) — refusés avec leurs motifs au § 4, suivi conditionné à une mesure.
- **Une colonne `issue_number` sur `tasks`** — ce serait la forme la plus robuste
  (AC2, troisième option), et elle demande une migration de schéma, un backfill
  de toutes les lignes existantes et un second écrivain à tenir cohérent avec
  `reference_url`. Hors de proportion pour un P3 dont le remède tient en trois
  lignes de SQL. **Suivi**, précondition : que S1 montre un ensemble de variantes
  qui n'est **pas** clos, c'est-à-dire que l'énumération ne suffise plus.
- **La réécriture des lignes portant une variante non déclarée** — c'est un geste
  d'opérateur sur la base, et son préalable est S1.
- **Le message (b) lui-même**, sa cadence, sa déduplication et son vocabulaire
  (mika#2161) — inchangés. Ce travail répare ce que la sonde *lit*, pas ce que le
  message *dit*.
- **Les sites 8, 9 et 10** — déjà délimités, aucune ligne touchée.
- **L'index `idx_tasks_manual_active_ref_url`** — aucune migration, aucune
  modification d'index.

---

## Definition of Done

- [ ] `task_state::tasks::issue_url_variants` existe, porte le recensement du
      § 2 dans son doc-comment, et a **≥ 3** sites d'appel.
- [ ] `find_active_self_dev_task_for_issue` compare `reference_url IN (?2, ?3)`.
- [ ] `find_dispatch_children_for_issue_url` compare
      `parent.reference_url IN (?2, ?3)`.
- [ ] `find_active_tracking_rows_by_reference_url_and_variants` passe par le
      helper, sortie inchangée, épinglé par test.
- [ ] Les six doc-comments / messages d'assertion de U3 ne disent plus
      « prefix `LIKE` ».
- [ ] D1 (AC3, deux sites) : **vu rouge** avant U2, vert après, contrôle positif
      inclus.
- [ ] D2 (table de délimitation, cas `_` compris) : **vu rouge** avant U2.
- [ ] D3 : le test d'équivalence `has_` ≡ `find_` étendu au voisin numérique.
- [ ] D4 : scan de source armé, anti-vacuité ≥ 3, allowlist à **une** entrée
      déclarée avec son assertion auto-nettoyante, contrôle négatif **vu rouge**.
- [ ] D5 : garde sur le nombre de sites d'appel du helper.
- [ ] `cargo test -p mika-agent` vert, `cargo clippy --all-targets` sans
      avertissement neuf, `cargo fmt --check` propre.
- [ ] Le corps de la PR porte le **grep du § 2** et sa table (AC1), plus le
      rétrécissement nommé du § 5 et la requête S1.
- [ ] Aucune migration de schéma, aucun événement de journal neuf, aucune ligne
      `audit_events` neuve.

---

## Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#2638 :

- [ ] **AC1.** Recenser **tous** les sites `LIKE '…issues/<N>%'` ou équivalents
      sur `reference_url` (grep consigné dans la PR).
- [ ] **AC2.** Chaque site compare l'URL à un **numéro délimité** : égalité
      stricte, ou `LIKE '<url>'` plus `LIKE '<url>#%'` / `'<url>/%'`, ou une
      colonne `issue_number` si elle existe.
- [ ] **AC3.** Test : la sonde de #216 ne matche pas une tâche de #2161 (**vu
      rouge** avant le correctif) ; elle matche bien une tâche de #216 (contrôle
      positif).

**Lecture de l'AC2 par ce plan :** la troisième option est indisponible
(`tasks` ne porte pas de colonne `issue_number` — § 1 R2) ; la première est
retenue, sous la forme de l'énumération exacte de l'ensemble clos des variantes
que le site 3 établit déjà dans le même fichier (§ 4). L'arbitrage contre la
seconde est écrit, avec ses quatre raisons et le coût du rétrécissement nommé
au § 5.
