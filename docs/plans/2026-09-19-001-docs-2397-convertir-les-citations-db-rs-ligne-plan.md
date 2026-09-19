# mika#2397 — Convertir les citations `db.rs:<ligne>` du corpus servi en citations par symbole

> **Type** : docs — conversion de citations, aucune ligne de code de production touchée.
> **Suivi de** : mika#2321 § Hors périmètre, décision D6.

## Préambule — comment ce ticket a été lu

`gh` n'est pas authentifié dans cette session (`gh auth status` → *not logged into
any GitHub hosts*), donc **le corps de l'issue #2397 n'a pas pu être lu**. Le
ticket est reconstruit à partir de trois sources présentes dans le dépôt :

1. le nom de la branche, `feat/2397/convertir-citations-db-rs-ligne-de-docs` ;
2. le commentaire opérateur transmis au dispatch (« Suite du découpage db.rs
   (#2321) ») ;
3. **la décision D6 du plan de mika#2321**, qui ouvre ce suivi nommément et en
   écrit le contenu :

   > **Conversion des citations `db.rs:<ligne>` de `docs/solutions/**`** (D6). Au
   > moins dix citations déjà fausses aujourd'hui, à convertir en
   > `<fichier>::<nom_de_fonction>`. Le découpage leur donne une cible stable ;
   > c'est un suivi de documentation, pas une condition de ce refactor.

   (`docs/plans/2026-09-18-005-chore-2321-decouper-db-rs-monolithe-plan.md`,
   accessible via `git show 184e9b0c:<chemin>` — la branche de #2321 n'est pas
   mergée.)

**Conséquence sur les critères d'acceptation** : la section
`## Acceptance criteria` ci-dessous est **dérivée**, pas transcrite. Si le corps
de #2397 porte ses propres AC, ce sont les siens qui font foi et ce plan doit
être révisé pour les reprendre verbatim.

## Contexte

Le plan de mika#2321 annonçait « au moins dix » citations fausses. La mesure sur
le dépôt à HEAD (`6c813454`) en trouve davantage, et le compte dépend
entièrement du périmètre retenu :

| périmètre | occurrences `db.rs:<ligne>` |
|---|---|
| `docs/plans/**` | **497** |
| `docs/solutions/**` | 17 |
| `docs/architecture/**` | 2 |
| `docs/brainstorms/**` | 5 |
| `docs/audits/**` | 4 |
| `crates/**` (commentaires et fixtures) | 5 |
| `CLAUDE.md`, `skills/bundled/**` | 0 |

Commande de recensement (elle doit gérer quatre écritures, cf. E4) :

```bash
grep -rnE '(^|[^a-zA-Z0-9_])db\.rs:{1,2}[0-9]+([-–][0-9]+)?' docs/ crates/
```

## Évidences

### E1 — Aucune des citations du corpus servi ne résout aujourd'hui

Résolution de chaque citation contre `crates/mika-agent/src/db.rs` à HEAD :

| citation | symbole visé | ligne réelle | état |
|---|---|---|---|
| `db.rs:5752` | `has_active_callback_tasks_excluding` | 9252 | dérive +3 500 |
| `db.rs:5839` | `has_any_active_callback` | 10038 | dérive +4 199 |
| `db.rs::8117` | `load_recent_messages` | 11284 | dérive +3 167 |
| `db.rs:3307-3309` | `get_user_visible_tasks` | 7711 | dérive ≈ +4 400 |
| `db.rs:4495` | `migrate_v44_to_v45` | 5310 | dérive +815 |
| `db.rs:3809` / `:3845` | `migrate_v29_to_v30` | 4151 | dérive +342 |
| `db.rs:9541` / `:9541-9570` | `has_completed_groom_for_issue` | 9885 | dérive +344 |
| `db.rs:1422` | DDL `tasks`, `parent_task_id … ON DELETE SET NULL` | 1485 | dérive +63 |
| `db.rs:27-46` | `UNIFIED_TIMELINE_VIEW_SQL` | 106 | dérive +79 |
| `db.rs:264–285` | `TaskHealthSummary` / `TaskHealthAnomaly` | **`task_state/tasks.rs:435,449`** | **autre fichier** |
| `db.rs:3123` | `find_active_work_item_by_ref_url` | **inexistant** | **symbole disparu** |
| `db.rs:3230` | `find_active_work_item_by_label` | **inexistant** | **symbole disparu** |

**Zéro sur dix-neuf résout.** Le taux n'est pas « la plupart sont fausses » : il
est total. Deux cas sont pires que la dérive — un symbole qui a changé de
fichier sans que personne le remarque, et deux symboles qui n'existent plus du
tout dans `crates/` (`grep -rn 'fn .*work_item' crates/mika-agent/src/db.rs`
rend zéro ligne).

### E2 — La doctrine que ce ticket applique est déjà écrite, une fois, dans le dépôt

`docs/solutions/best-practices/observability-side-channel-must-not-be-able-to-change-the-verdict-2026-08-31.md:68-72` :

> **Un numéro de ligne cité dans un plan est une affirmation à revérifier.** Le
> plan désignait `db.rs:2338` comme « l'écrivain `audit_events` » ; cette ligne
> est en réalité une migration v8→v9. Le vrai écrivain est
> `evidence/audit.rs::log_audit_event`. **Grep le symbole, jamais la ligne.**

La règle existe, datée du 31/08, tirée d'un piège réellement rencontré, et elle
prescrit exactement la forme cible `<fichier>::<symbole>`. Ce ticket ne
l'invente pas : il la **met en vigueur** sur le corpus qui la porte.

### E3 — Une règle écrite interdit explicitement ce que ce ticket fait

`docs/solutions/documentation-gaps/stale-issue-citation-drift-in-code-comments-2026-05-12.md`,
dernière ligne :

> Historical documents (`docs/plans/`, `docs/solutions/`) should NOT be
> retroactively updated -- they record what was believed at the time and serve
> as historical records.

Elle nomme `docs/solutions/`. C'est la contrainte centrale du ticket et elle est
traitée en D2, pas contournée.

### E4 — La grammaire des citations dans la nature compte quatre écritures

Mesurées : `db.rs:5752` (deux-points, ligne simple), `db.rs::8117` (**double**
deux-points), `db.rs:3307-3309` (plage, trait d'union ASCII), `db.rs:264–285`
(plage, **tiret demi-cadratin** U+2013, dans
`docs/architecture/core-memory-promotion-protocol.md`). Un recensement écrit
avec `[0-9]+(-[0-9]+)?` **manque la quatrième** : il capte `264` et laisse
croire à une citation simple. Toute commande de vérification de ce plan gère les
quatre.

Ce n'est pas une précaution théorique : **le premier recensement de ce plan a
sous-compté `docs/plans/**` de 34 occurrences** (463 au lieu de 497) pour
exactement cette raison, avant d'être refait avec `[-–]`. La table du § Contexte
porte le compte corrigé.

### E5 — `docs/solutions/**` est un corpus servi à des agents, pas une archive

`crates/mika-agent/src/kg/lexical_ingestor.rs:3` : *« Per-agent ingestion of
`docs/solutions/**/*.md` into the lexical layer »*. `MIKA_KG_DOCS_ROOT` vaut par
défaut `<CWD>/docs/solutions`. Ces fichiers sont découpés en chunks, indexés
FTS5 + vecteurs, et **retournés comme contexte** à mika-arch. `docs/plans/**`
n'est ingéré par rien.

Un localisateur faux dans `docs/solutions/**` n'est donc pas une coquille dans
une archive : c'est une affirmation servie à un lecteur — humain ou modèle — qui
va la suivre.

### E6 — Dans quinze cas sur dix-neuf, le symbole est déjà écrit à côté du numéro

Exemple, `asymmetric-perimeter-predicate-drift.md:56` :

> **Tool-boundary gate** (`Database::has_active_callback_tasks_excluding`,
> `crates/mika-agent/src/db.rs:5752`)

Le symbole est là. Le numéro n'ajoute rien qu'une fausseté. Cette mesure change
la nature du geste : dans la majorité des cas, convertir n'est pas *réécrire une
affirmation*, c'est **retirer une information redondante et fausse**. C'est
l'argument matériel de D2.

### E7 — La classe est dix fois plus large que `db.rs`

Dans `docs/solutions/**` + `docs/architecture/**`, les citations
`<fichier>.rs:<ligne>` toutes cibles confondues sont **189**, dont 19 pour
`db.rs` (10 %). Les autres têtes de liste : `mod.rs` 19, `executor.rs` 16,
`agent.rs` 14, `prompt.rs` 13, `a2a.rs` 10, `engine.rs` 9.

Ce nombre décide D5 : une garde CI armée aujourd'hui serait rouge sur 170
citations que ce ticket ne convertit pas.

### E8 — Cinq citations vivent dans `crates/`, et deux d'entre elles sont des fixtures gelées

| site | citation | nature |
|---|---|---|
| `task_engine/engine.rs:218` | `db.rs:1484-1490` | commentaire — **exacte aujourd'hui** |
| `db.rs:5589` | `db.rs:2696` | commentaire — dans `migrate_v11_to_v12` (2634), **exacte** |
| `db/tests/harnais_porte.rs:38` | `db.rs:6552` | commentaire — `update_task_status` est à 6738, **fausse** |
| `agent_loop/review_anchor.rs:640` | `db.rs:7843-7910` | **fixture gelée** — extrait de brief mesuré de mika#2338 |
| `agent_loop/review_anchor.rs:654` | `db.rs:7843-7910` | **fixture gelée** — réponse architecte mesurée de mika#2338 |

Les deux dernières sont des constantes `BRIEF_2338_MARKDOWN` et
`RESPONSE_2338_REAL` : du texte **mesuré et recopié verbatim** pour épingler un
comportement d'ancrage. Les modifier falsifierait la mesure — c'est le régime
que le dépôt applique déjà à ses jeux de fixtures (mika#2158 et mika#2120 :
« Fixtures are frozen, not refreshed »).

Elles fournissent la preuve indépendante que la distinction posée en D4 n'est
pas une commodité de périmètre : le dépôt la pratique déjà, ailleurs, pour la
même raison.

## Décisions

### D1 — Le périmètre est le corpus **servi**, jamais l'archive datée

**Retenu** : `docs/solutions/**` (17) et `docs/architecture/**` (2). **Écarté** :
`docs/plans/**` (497), `docs/brainstorms/**` (5), `docs/audits/**` (4),
`crates/**` (5, cf. D7).

Le discriminant n'est pas le volume, c'est **ce que le document prétend être**.
Un plan, un brainstorm, un audit portent une date dans leur nom et décrivent
l'état du monde à cette date ; les réécrire rendrait faux ce qu'ils ont dit au
moment où ils l'ont dit — la règle que mika#2361 a dû écrire pour ses propres
lignes d'audit. Une solution et une référence d'architecture prétendent décrire
le code **tel qu'il est** ; un localisateur faux y est un défaut, pas un
témoignage.

E5 donne à ce discriminant sa forme la plus dure pour `docs/solutions/**` : ces
fichiers sont ingérés et **servis** à mika-arch. `docs/architecture/**` n'est pas
ingéré, et est néanmoins retenu : l'ingestion KG est l'instance la plus forte de
la classe « document vivant », pas sa définition. `core-memory-promotion-protocol.md`
ne porte pas de date, est maintenu, et ses deux citations sont du pire genre
(mauvais fichier, E1).

**Coût nommé** : 497 citations fausses restent dans `docs/plans/**` après cette
PR. Ce n'est pas une dette laissée par paresse, c'est la conséquence assumée de
la règle — et c'est aussi pourquoi aucune garde ne peut être armée ici (D5).

### D2 — La règle de non-réécriture rétroactive protège les **affirmations**, pas les **localisateurs**

E3 interdit littéralement ce qui est fait ici. Trois raisons, dans l'ordre de
leur force, font que la règle ne couvre pas ce geste — et la troisième est une
mesure, pas un argument.

1. **Sa classe d'origine.** Elle est écrite dans un document sur la dérive des
   citations d'*issues* (`#265`), où la citation **est** une affirmation
   historique : « c'est #265 qui a introduit cette convention » était une
   croyance à la date de rédaction. Un numéro de ligne n'est la croyance de
   personne ; c'est un pointeur, et il a cessé d'être vrai au commit suivant sa
   rédaction, pas à une date où quelqu'un aurait changé d'avis.

2. **Ce qu'un document assert versus ce par quoi il pointe.** Remplacer
   `db.rs:5752` par le symbole ne modifie **aucune** proposition du document :
   ni le diagnostic, ni la chronologie, ni le verdict. Le seul énoncé retiré est
   « ce code est à la ligne 5752 », qui est faux et que personne n'a jamais
   voulu affirmer.

3. **La mesure E6.** Dans quinze cas sur dix-neuf, le symbole est déjà écrit
   dans la même parenthèse. Le geste y est **une suppression**, pas une
   réécriture — on retire un chiffre faux d'à côté d'un nom juste.

**La preuve que la distinction est prise au sérieux et non invoquée pour la
forme** : D4 gèle quatre citations, précisément celles où le numéro de ligne
*est* l'affirmation. Une règle qui n'exclut rien ne fait que couvrir ce qu'on
voulait déjà faire.

**Refusé : modifier la ligne de E3 pour la rendre compatible.** Elle est juste
dans sa classe. On lui ajoute, en fin de PR, **une** phrase disant que la
conversion d'un localisateur en citation par symbole n'est pas une réécriture
rétroactive — de sorte que le prochain lecteur trouve l'arbitrage à l'endroit où
il trouvera la règle, et non enterré dans un plan.

### D3 — La forme cible est `<fichier>::<symbole>`, et ce qu'elle achète est **l'échec bruyant**, pas la permanence

D6 de mika#2321 prescrit `<fichier>::<nom_de_fonction>`. Cette forme a un défaut
connu et **daté** : mika#2396 (en vol) sort 111 méthodes du domaine `tasks` vers
`db/tasks.rs`, et mika#2321 sort l'échelle de migrations. Toute citation
`db.rs::has_completed_groom_for_issue` ou `db.rs::migrate_v44_to_v45` sera
fausse **par sa moitié fichier** au merge de ces PR.

La forme est retenue quand même, et la raison est une asymétrie, pas un
compromis :

- Un **numéro de ligne faux échoue en silence.** `sed -n '5752p' db.rs` rend du
  code plausible, bien formé, sans rapport. Le lecteur ne peut pas savoir qu'il
  a été trompé. C'est très exactement le piège que E2 a documenté (« cette ligne
  est en réalité une migration v8→v9 »).
- Un **fichier faux avec un symbole juste échoue bruyamment**, et se répare par
  un geste : `grep -rn '<symbole>' crates/` rend la bonne réponse. E1 en donne
  la démonstration en deux exemplaires — c'est exactement par ce grep que
  `TaskHealthSummary` a été retrouvé dans `task_state/tasks.rs`, et c'est le même
  grep, revenu **vide**, qui a établi que `find_active_work_item_by_ref_url`
  n'existe plus.

Ce ticket ne rend donc pas les citations éternelles, et ne doit pas le prétendre.
Il fait passer leur mode de défaillance de *silencieux et plausible* à *bruyant
et réparable en une commande*.

**Corollaire d'écriture, qui est la règle opératoire :** la moitié porteuse est
le **symbole** ; le chemin de fichier est un repère d'orientation. Quand le
symbole est déjà nommé dans la phrase (E6, quinze cas), **on retire le numéro et
on n'ajoute rien** — dupliquer le symbole dans la parenthèse alourdirait sans
rien acheter.

**Refusé : `Database::<méthode>` sans chemin de fichier.** Cette forme survivrait
au découpage sans une retouche, et elle a été considérée sérieusement pour cette
raison. Elle est écartée parce qu'elle perd le repère de localisation que le
dépôt emploie partout ailleurs, et parce que l'invariant qui compte — le grep
répare — tient déjà avec le chemin.

### D4 — Quatre citations sont **gelées**, et la règle qui les distingue est écrite

Une citation est un **localisateur** quand le document invite à aller lire le
code pour vérifier ou comprendre une affirmation sur le code *tel qu'il est*.
Elle est un **enregistrement** quand elle fait partie de ce que le document
rapporte d'un état passé — temps du passé, inventaire daté, exemple exhibé, ou
citation qui est elle-même l'objet d'étude.

Les quatre enregistrements, chacun avec son motif :

| site | citation | motif du gel |
|---|---|---|
| `observability-side-channel-…-2026-08-31.md:71` | `db.rs:2338` | la citation **est** l'objet d'étude : le document l'exhibe comme fausse. La convertir détruirait sa démonstration |
| `stale-issue-citation-drift-…-2026-05-12.md:36` | `db.rs:4368` | exemple illustratif dans un document sur la dérive des citations |
| `tasks-type-column-orthogonal-work-item-role.md:122` | `db.rs:3123` | inventaire au passé (« in mika-agent these **were** ») d'un symbole aujourd'hui disparu |
| `tasks-type-column-orthogonal-work-item-role.md:123` | `db.rs:3230` | idem |

Les deux dernières sont aussi les deux seules **non convertibles** : le symbole
n'existe plus (E1), donc aucune citation par symbole ne résoudrait. La règle et
la contrainte matérielle tombent ici au même endroit, ce qui est un bon signe
pour la règle.

**Refusé : supprimer les deux phrases devenues inexactes.** Le document enseigne
un motif — « tout site `SELECT` écrit à la main est un risque de migration » —
et l'inventaire en est la preuve. L'effacer coûterait l'enseignement pour
gagner une exactitude que le temps du passé assure déjà.

### D5 — Aucun convertisseur, aucune garde CI dans cette PR

**Pas de script de conversion.** Dix-neuf citations, onze fichiers, et chacune
demande un jugement qu'aucun `sed` ne rend : de quel symbole s'agit-il ? a-t-il
changé de fichier (`TaskHealthSummary`) ? a-t-il disparu (`find_active_work_item_*`) ?
est-ce un enregistrement (D4) ? Un convertisseur automatique aurait converti les
quatre gelées — c'est-à-dire cassé la démonstration de E2 — et aurait produit
deux citations vers des symboles inexistants.

**Pas de garde CI.** Une garde qui refuse `<fichier>.rs:<ligne>` dans
`docs/solutions/**` serait **rouge au premier `push`** : E7 mesure 170 citations
sœurs dans le même corpus que ce ticket ne convertit pas. Restreindre la garde à
`db.rs` la rendrait arbitraire — le défaut n'a rien de propre à ce fichier ;
l'étendre à tout le corpus ferait de ce ticket de documentation un chantier de
189 conversions.

Le suivi est nommé au § Hors périmètre, avec le nombre qui le conditionne. C'est
la même discipline que mika#2321 s'est appliquée à lui-même : le geste facile
(une entrée d'allowlist, ici une garde à périmètre taillé sur mesure) est refusé
et sa dette est écrite.

### D6 — Une phrase touchée est convertie **en entier**, y compris ses citations non-`db.rs`

`core-memory-promotion-protocol.md:50` porte trois citations dans la même
parenthèse : `db.rs:264–285`, `agent.rs:2692–2704`, `prompt.rs:815–873`.

Laisser les deux dernières produirait une phrase qui **enseigne les deux
formes côte à côte**, la nouvelle à gauche et celle qu'elle remplace à droite —
pire que l'un ou l'autre état pur. Les phrases effectivement touchées sont donc
converties en entier.

**La règle est bornée par construction** : elle ne s'applique qu'aux lignes déjà
modifiées pour une citation `db.rs`. Le débordement mesuré est de **2**
citations (`agent.rs`, `prompt.rs`, toutes deux sur la ligne 50). Il n'ouvre pas
E7 : les 170 autres vivent sur des lignes que cette PR ne touche pas.

### D7 — Les cinq citations de `crates/**` sont hors périmètre, et la raison n'est pas la même pour toutes

- Les **deux fixtures** de `review_anchor.rs` (E8) sont gelées par la même règle
  que D4, en plus forte : ce sont des mesures recopiées, et le dépôt gèle déjà
  ses fixtures par décision écrite.
- Les **trois commentaires** (`engine.rs:218`, `db.rs:5589`,
  `db/tests/harnais_porte.rs:38`) relèvent du périmètre « citations dans le
  code », que ce ticket n'ouvre pas : leur conversion touche des fichiers `.rs`,
  donc passe la CI Rust, la garde `check-byte-slices`, la garde de périmètre —
  pour un bénéfice documentaire. Deux des trois sont d'ailleurs **exactes
  aujourd'hui**, ce qui rendrait la PR d'autant moins lisible (« pourquoi
  change-t-il une ligne juste ? »).

Suivi nommé au § Hors périmètre.

## Volet d'implémentation

Un seul volet, un seul commit de conversion (plus le commit de plan). Aucun
fichier `.rs` n'est modifié.

### I1 — Recenser et geler la liste de travail

```bash
grep -rnE '(^|[^a-zA-Z0-9_])db\.rs:{1,2}[0-9]+([-–][0-9]+)?' docs/solutions/ docs/architecture/
```

Attendu : 19 occurrences sur 17 lignes dans 11 fichiers. Si le compte diffère,
**arrêter** : le corpus a bougé depuis ce plan et la table E1 doit être refaite
avant toute conversion. Retrancher les 4 gelées de D4 → **15 à convertir**.

### I2 — Résoudre chaque citation vers son symbole actuel

Pour chacune des 15, dans l'ordre :

1. Lire la phrase qui porte la citation et identifier ce qu'elle désigne.
2. `grep -rn '<symbole>' crates/mika-agent/src/` — **le grep est la résolution**,
   jamais le numéro cité.
3. Trois issues possibles :
   - une seule définition → c'est la cible ;
   - définition dans un **autre fichier** → la cible porte cet autre fichier
     (cas `TaskHealthSummary` → `task_state/tasks.rs`) ;
   - **aucune** définition → la citation n'est pas convertible ; elle relève de
     D4 et doit être réexaminée comme enregistrement avant toute écriture.

Cibles déjà résolues par ce plan (E1) — à revérifier, pas à recopier de
confiance :

| fichier | cible |
|---|---|
| `asymmetric-perimeter-predicate-drift.md` ×2 | symboles déjà nommés → **retirer le numéro** (D3 corollaire) |
| `deferred-dispatch-promotion-deadlock-2026-05-10.md` ×2 | idem |
| `per-user-content-serve-ledger-fidelity-2026-08-21.md:26` | idem (`Database::load_recent_messages`) |
| `per-user-…:138` | `db.rs::migrate_v44_to_v45` |
| `schema-bump-rollback-semantics-2026-05-28.md:48` ×2 | `db.rs::migrate_v29_to_v30` **une fois** ; l'ancre de la seconde est le message d'audit déjà cité entre guillemets |
| `tui-background-task-running-indicator.md:19` | « le commentaire sur `get_user_visible_tasks` » |
| `grooming-provenance-gate-….md:44,116` | `db.rs::has_completed_groom_for_issue` |
| `grooming-provenance-gate-….md:188` | `db.rs`, schéma `tasks` — la colonne est déjà citée verbatim |
| `trace-id-observability-gaps-….md:29` | `db.rs::UNIFIED_TIMELINE_VIEW_SQL` |
| `core-memory-promotion-protocol.md:50` | `db.rs::get_task_health_summary` + les deux sœurs `agent.rs` / `prompt.rs` (D6) |
| `core-memory-promotion-protocol.md:298` | **`task_state/tasks.rs::TaskHealthSummary` / `::TaskHealthAnomaly`** — changement de fichier |

### I3 — Écrire, sans toucher aux propositions

Contrainte d'écriture : **aucune phrase ne change de sens**. Le diff attendu est
fait de suppressions de `:NNNN` et de substitutions `:NNNN` → `::<symbole>`. Un
diff qui reformule un diagnostic est hors contrat et doit être défait.

### I4 — Ajouter l'arbitrage à l'endroit où vit la règle qu'il nuance

Dans `stale-issue-citation-drift-in-code-comments-2026-05-12.md`, sous la ligne
citée en E3 : **une** phrase disant que convertir un localisateur
`<fichier>:<ligne>` en citation par symbole n'est pas une mise à jour
rétroactive — le document continue d'affirmer exactement ce qu'il affirmait —
avec le renvoi à mika#2397.

C'est le seul ajout de ce ticket qui n'est pas une conversion, et il est
délibéré : sans lui, le prochain lecteur de cette règle conclura que la PR l'a
violée.

### I5 — Vérifier

```bash
# a) plus aucune citation par ligne dans le corpus servi, hors les 4 gelées
grep -rnE '(^|[^a-zA-Z0-9_])db\.rs:{1,2}[0-9]+([-–][0-9]+)?' docs/solutions/ docs/architecture/
#   attendu : exactement 4 lignes, celles de la table D4

# b) chaque symbole cité résout — pour chaque `<fichier>::<symbole>` introduit
grep -rn '<symbole>' crates/mika-agent/src/<fichier>
#   attendu : au moins une ligne, et c'est la définition

# c) aucun fichier de code n'est touché
git diff --name-only main... | grep -v '^docs/'
#   attendu : aucune sortie
```

Le résultat de (a) et (c) est reporté **verbatim** dans le corps de la PR.

## Definition of Done

- Les 15 citations convertibles de `docs/solutions/**` et `docs/architecture/**`
  ne portent plus de numéro de ligne.
- Chaque symbole nouvellement cité résout par `grep` dans le fichier cité.
- Les 4 citations gelées de D4 sont intactes, et le plan dit pourquoi.
- `stale-issue-citation-drift-in-code-comments-2026-05-12.md` porte la phrase
  d'arbitrage de I4.
- Aucun fichier hors `docs/` n'est modifié.
- Les trois suivis du § Hors périmètre sont ouverts.

## Acceptance criteria

> **Dérivés**, pas transcrits — `gh` n'était pas authentifié et le corps de
> #2397 n'a pas pu être lu (cf. Préambule). Si le ticket porte ses propres AC,
> ils prévalent.

1. **AC1 — Le corpus servi ne cite plus par ligne.**
   `grep -rnE '(^|[^a-zA-Z0-9_])db\.rs:{1,2}[0-9]+([-–][0-9]+)?' docs/solutions/ docs/architecture/`
   rend **exactement 4** occurrences, et ce sont les quatre de la table D4.

2. **AC2 — Toute citation introduite résout.** Pour chaque `<fichier>::<symbole>`
   écrit par cette PR, `grep -n '<symbole>' crates/mika-agent/src/<fichier>` rend
   au moins une ligne qui est la définition du symbole. Aucun symbole cité n'est
   absent de `crates/`.

3. **AC3 — Les deux citations à symbole disparu ne sont pas converties.**
   `find_active_work_item_by_ref_url` et `find_active_work_item_by_label` restent
   citées telles quelles dans
   `tasks-type-column-orthogonal-work-item-role.md`, et le plan en donne le
   motif.

4. **AC4 — La démonstration de E2 est intacte.** Le fragment `db.rs:2338` de
   `observability-side-channel-must-not-be-able-to-change-the-verdict-2026-08-31.md`
   est inchangé, ainsi que la phrase qui l'exhibe comme fausse.

5. **AC5 — Aucune proposition n'a changé.** Le diff ne contient que des
   suppressions de `:NNNN` et des substitutions `:NNNN` → `::<symbole>` (plus la
   phrase de I4). Aucune phrase n'est reformulée, aucun diagnostic réécrit.

6. **AC6 — Aucun code n'est touché.** `git diff --name-only main...` ne rend que
   des chemins sous `docs/`.

7. **AC7 — L'arbitrage est écrit où vit la règle.**
   `stale-issue-citation-drift-in-code-comments-2026-05-12.md` porte la phrase de
   I4, avec le renvoi à mika#2397.

8. **AC8 — Les suivis sont ouverts** (une issue par entrée du § Hors périmètre).

## Ce que ce travail n'achète pas

- **Pas de permanence.** D3 : les citations `db.rs::<symbole>` seront fausses par
  leur moitié fichier au merge de mika#2396 et de mika#2321. Ce qui est acheté
  est un mode de défaillance bruyant et réparable par un `grep`, pas une
  exactitude durable.
- **Pas de prévention.** Rien n'empêche la prochaine PR d'écrire
  `db.rs:12345` dans une solution. La garde est refusée ici avec sa raison (D5) ;
  tant qu'elle n'existe pas, la conversion est un geste ponctuel.
- **Pas d'événement, pas de compteur, pas de surface opérateur.** Il n'y a rien
  à observer : aucune ligne de code ne change de comportement. La seule sonde est
  la commande de I5, rejouable à tout instant.
- **497 citations fausses restent dans `docs/plans/**`**, par décision (D1), et
  5 dans `crates/**` (D7).

## Hors périmètre (suivi à ouvrir)

- **Généraliser la conversion aux 170 citations sœurs de `docs/solutions/**` et
  `docs/architecture/**`** (E7 : `mod.rs` 19, `executor.rs` 16, `agent.rs` 14,
  `prompt.rs` 13, `a2a.rs` 10, `engine.rs` 9, …). Même défaut, même remède, dix
  fois le volume. C'est la **précondition** de la garde ci-dessous : tant qu'elles
  sont là, aucune garde de corpus ne peut être verte.
- **Garde CI refusant `<fichier>.rs:<ligne>` dans le corpus servi** (D5). À armer
  seulement après le suivi précédent, sur le modèle de
  `scripts/check-byte-slices.sh` + un job `ci.yml` — c'est-à-dire un scan de
  corpus avec son propre test (`scripts/test-check-*.sh`, l'idiome du dépôt).
  Armée avant, elle est rouge sur 170 lignes.
- **Les trois citations `db.rs:<ligne>` des commentaires de `crates/**`**
  (`engine.rs:218`, `db.rs:5589`, `db/tests/harnais_porte.rs:38`, cf. D7). Deux
  sont exactes aujourd'hui, une est fausse ; leur conversion est un ticket de
  code, pas de documentation. Les deux fixtures de `review_anchor.rs` sont
  gelées et n'en font **pas** partie.
- **Relevé en chemin, sans rapport :**
  `docs/architecture/core-memory-promotion-protocol.md` décrit
  `TaskHealthSummary` / `TaskHealthAnomaly` comme vivant dans `db.rs` ; ils sont
  dans `task_state/tasks.rs` depuis un déplacement que personne n'a répercuté.
  La citation est corrigée ici ; savoir si le reste du document a dérivé avec le
  type n'a pas été audité.
