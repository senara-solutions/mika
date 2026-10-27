# mika#2922 — le registre des verdicts de vol (vol 3/3) : le compte de concordance

**Ticket :** senara-solutions/mika#2922 — `feat(manager): registre des verdicts de vol et jugement MPC en face — le compte de concordance avant EVALUATE_WRITE (mika#2698)`
**Labels :** `enhancement`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
**Vols antérieurs, tous deux mergés dans `main` :**
- vol 1 — `598055bfe` / PR #2928, plan `docs/plans/2026-10-26-003-feat-2922-manager-registre-des-verdicts-de-vol-et-plan.md` (l'écrivain du manager) ;
- vol 2 — `967e1f1fb` / PR #2937, plan `docs/plans/2026-10-27-002-feat-2922-manager-registre-des-verdicts-de-vol-et-plan.md` (l'écrivain de MPC).

Ce vol est le **dernier** : il livre le livrable 3 du ticket, AC3, AC4 et la
seconde moitié d'AC6 — exactement le périmètre que la ligne `Tracked in:` du
plan du vol 2 lui a nommé.

---

## Contexte — les deux colonnes existent, personne ne les lit

Le seuil qui ouvre `MIKA_MANAGER_EVALUATE_WRITE` (mika#2698) est « les mêmes
verdicts que MPC sur 10 vols consécutifs ou 7 jours ». Bearing de Prime du
2026-10-26, verbatim : « l accord ombre doit être mesuré et les verdicts
stockés *avant* tout réarmement ». Les deux vols précédents ont posé le
stockage ; **la mesure n'existe toujours pas**.

Ce qui est déjà en place, et sur quoi ce vol se branche sans rien déplacer :

| acquis | site |
|---|---|
| le verdict du manager, une ligne par vol et par premier cycle | `flight_verdict_ledger::write_and_swallow` (vol 1) |
| le jugement de MPC, sous **le même `tasks.id`** | `flight_verdict_ledger::record_mpc_judgment` (vol 2) |
| la grammaire commune des deux côtés | `VerdictRecord { issue, recommendation }`, JSON serde |
| la projection décisionnelle | `VerdictRecord::terms() -> VerdictTerms` (`flight_verdict_ledger.rs:252-277`) |
| la fenêtre, dérivée et non écrite | `inscription_window_start()` ← `AUDIT_RETENTION_DAYS` (90) |
| le chemin supporté d'écriture | `mika flight-verdicts judge` |

Et ce que la doc de module du vol 1 annonce en ces termes : « **Personne ne lit
encore.** Le compte de concordance — celui qui met les deux côtés en face et
rend « N vols consécutifs concordants » — est le vol 3. Il n'y a donc aucune
garde de lecteur unique dans ce fichier : une garde sans lecteur serait verte
par construction, ce qui est précisément la classe mika#2205. »

Aujourd'hui la seule lecture des deux colonnes est le SQL à la main des blocs
**Operator query** de `docs/agent-internals/audit-log.md` (l. 154 et l. 222) —
et c'est précisément ce que ce vol remplace, comme le plan du vol 2 l'écrit :
« La lecture en SQL des étapes 1 et 3 est **ce que le vol 3 remplace**. »

---

## Ce que livre ce vol

1. **Un accesseur de lecture borné dans le temps** — `audit_events` filtré par
   `tool_name` et `created_at >= since`, rendant `created_at` : les quatre
   accesseurs existants n'en rendent aucun, et sans lui ni la fenêtre en jours
   ni l'ordre chronologique du compte « consécutif » ne sont lisibles.
2. **Le lecteur de concordance** — un seul, dans
   `crates/mika-agent/src/flight_verdict_ledger.rs` : il apparie les deux
   colonnes sur `tasks.id`, compare par `VerdictTerms`, nomme le **premier**
   champ divergent, et compte les vols consécutifs concordants.
3. **Le rendu** — ligne par ligne (vol, ticket, verdict du manager, jugement de
   MPC, accord/désaccord, champ divergent) et son en-tête (« N vols consécutifs
   concordants » + la fenêtre en jours).
4. **`mika flight-verdicts concordance`** — la commande en lecture seule, avec
   `--format text|json|yaml`.
5. **La garde de lecteur unique** et son contrôle négatif : celle que le vol 1 a
   refusé d'écrire faute de lecteur, et qui naît donc ici.
6. **La sonde post-déploiement** complétant AC6, et la mise à jour des deux
   sections de `docs/agent-internals/audit-log.md` dont l'« Operator query »
   devient un pis-aller.

Hors périmètre, nommé pour ne pas être re-dérivé : **aucun verbe de
correction** (`judge --force`, `amend`). Le plan du vol 2 en a posé le critère
de bascule — « la première correction réellement nécessaire, constatée » — et
elle n'est pas constatée. **Aucune décision d'ouverture** non plus : la
commande rend le compte et la fenêtre, le seuil de mika#2698 reste jugé par
Vincent sur ce que MPC lui présente.

---

## Conception

### 1. L'accesseur est **nouveau**, et les quatre existants expliquent pourquoi

`db.rs` porte quatre lectures d'`audit_events` par `tool_name` :
`count_audit_events_by_tool_name` (l. 2112),
`get_audit_event_target_keys_by_tool_name` (l. 2126),
`get_audit_event_rows_by_tool_name` (l. 2150) et, par cible,
`latest_audit_event_for_target` (l. 2044). **Aucune ne rend `created_at`, et
aucune ne porte de borne basse.** Le lecteur a besoin des deux : l'en-tête doit
dire la fenêtre en jours, et « N vols **consécutifs** » n'a de sens que sur un
ordre.

Étendre `get_audit_event_rows_by_tool_name` est refusé : elle est
`#[doc(hidden)]`, documentée comme un helper de tests mika#1712, et comptée
~25 fois dans le dépôt — une signature changée ferait du vol 3 un patch de 25
sites de tests, dont aucun ne dit quoi que ce soit sur la concordance.

On ajoute donc **une** paire :

```rust
// db.rs
pub type AuditEventWindowRow = (String, Option<String>, Option<String>, String);
//                              target_key, after_value, reasoning, created_at

pub fn list_audit_events_for_tool_since(
    &self, agent_id: &str, tool_name: &str, since: &str,
) -> Result<Vec<AuditEventWindowRow>>   // ORDER BY id

// async_db.rs — l'enveloppe, qui injecte agent_id comme les trois autres
pub async fn list_audit_events_for_tool_since(&self, tool_name: &str, since: &str) -> …
```

`ORDER BY id` et non `ORDER BY created_at` : `created_at` est un TEXT ISO 8601
à la seconde, donc deux lignes du même cycle peuvent être à égalité, et `id`
est le seul ordre total que la table offre. L'ordre d'insertion **est** l'ordre
chronologique — il n'y a pas d'autre écrivain par construction (AC2).

L'accesseur n'est **pas** `#[doc(hidden)]` : c'est une lecture de production,
et le marquer en helper de tests est exactement ce qui a conduit
`get_audit_event_rows_by_tool_name` à être appelée en production sous une doc
qui dit le contraire.

### 2. Le lecteur vit dans `flight_verdict_ledger.rs`, et c'est structurel

Trois raisons, dont deux sont des gardes qui mordent déjà :

1. **La garde de l'adaptateur CLI l'y oblige.**
   `commands/flight_verdicts.rs § mika2922_ladaptateur_nest_pas_un_ecrivain`
   refuse dans tout `crates/mika-cli/src/` les quatre aiguilles
   `"mpc_flight_judgment"`, `"manager_flight_verdict"`,
   `MPC_FLIGHT_JUDGMENT_TOOL`, `MANAGER_FLIGHT_VERDICT_TOOL`. Un lecteur
   côté CLI devrait nommer au moins l'une d'elles. Son message d'échec nomme
   déjà le remède : « une **lecture** par l'accesseur du registre (le motif de
   mika#2267 : le lecteur ne recompose pas la clé du scribe, sinon il regarde
   où l'autre n'écrit pas) ». La fonction publique du lecteur ne prend donc
   **aucun** argument `tool_name` — les deux constantes restent du côté du
   module qui les déclare.
2. **`source_scan::production_files()` est enraciné sur `crates/mika-agent/src`**
   (INV-2, le crate CLI est un adaptateur mince). Un lecteur côté CLI serait
   invisible à la garde de lecteur unique, qui serait verte sans rien dire —
   classe mika#2205.
3. **AC5 tient sans qu'une exception l'excepte.** `flight_verdict_ledger.rs`
   est un **frère** de `src/milestone_manager/`, donc hors du périmètre que
   `no_dispatch_test.rs` balaie, exactement comme `flight_source.rs`. Son
   ensemble de jetons n'est pas touché.

### 3. L'appariement se fait sur la colonne **du manager**, et le ticket s'y lit

La population de référence est celle du manager : c'est elle qui est écrite par
le moteur à chaque cycle, et c'est elle qui porte le ticket. Fait de passation
du vol 2, verbatim : « `reasoning` ne porte pas la même donnée des deux côtés,
et c'est délibéré. Côté manager il porte `issue_number=<n>` ; côté MPC il porte
une note libre. Le vol 3 lira donc le ticket d'un vol sur la ligne **du
manager**, jamais sur celle de MPC. »

Le préfixe `issue_number=` est aujourd'hui un littéral inline
(`flight_verdict_ledger.rs:429`). On l'extrait en `const` privée **du module**,
consommée par l'écrivain et par le lecteur : deux littéraux pour une grammaire
sont la classe mika#2158, et ici l'écrivain et le lecteur sont dans le même
fichier, donc il n'y a aucune raison d'en avoir deux.

Un `reasoning` absent, ou présent mais sans ce préfixe, donne un **ticket
absent** — jamais `0`, jamais `#0` (motif mika#2331). Le rendu l'écrit `—`.

**Les orphelins sont dits, pas jetés.** Une ligne MPC sans ligne manager sous
la même clé est possible (MPC juge à la main, sur un `tasks.id` qu'il tape).
Le rapport porte donc un compte d'orphelins et l'en-tête le dit quand il est
non nul : les laisser tomber silencieusement rendrait « aucun orphelin » et
« le lecteur ne les regarde pas » identiques.

### 4. La comparaison passe par `VerdictTerms`, jamais par l'égalité des lignes

`VerdictRecord` porte `FlightIssue`, dont `PrOpened { number, url }` et
`RescuePr { number, url }` ont des champs que le manager remplit et que MPC
écrit `null` : une égalité de `VerdictRecord` rendrait **divergent** un accord
parfait. `VerdictRecord::terms()` est la projection décisionnelle — issue,
cause, recommandation — et c'est la seule chose sur laquelle les deux côtés se
sont engagés.

Le champ divergent est le **premier** des trois dans l'ordre fixe, et cette
règle est déjà écrite dans la doc de `VerdictTerms` : « L'ordre des trois est
fixe — `issue`, puis `cause`, puis `recommendation` — et le **premier** qui
diverge est le champ que la commande […] nommera (AC3). » Le lecteur ne la
re-décide pas ; il l'applique.

La désérialisation est l'inverse exact de serde : `serde_json::from_str`
(ou `from_value`) vers `VerdictRecord`, sur les types existants. Aucune table
de graphies côté lecture non plus — une sixième cause de mort ajoutée en amont
se lit sans qu'une ligne bouge. Un `after_value` illisible n'est **pas** avalé :
il est compté, nommé par un WARN sur l'idiome de `warn_held_unreadable`
(`flight_verdict_ledger.rs:725`), et la ligne apparaît dans le rendu comme
illisible plutôt que comme un désaccord.

### 5. « N vols consécutifs » se compte du plus récent vers le plus ancien

C'est le sens que mika#2698 donne au seuil : « les mêmes verdicts que MPC sur
10 vols **consécutifs** » décrit une série qui court jusqu'à maintenant. Donc :

- on parcourt les vols du manager du plus récent au plus ancien ;
- un vol **concordant** incrémente le compte ;
- un vol **divergent** arrête le parcours ;
- un vol **non jugé par MPC** est **sauté** — il n'incrémente pas et il
  n'arrête pas. C'est AC4 mot pour mot : « ne compte ni pour ni contre » ;
- un vol dont l'un des deux `after_value` est illisible arrête le parcours : un
  accord qu'on ne peut pas lire n'est pas un accord.

D'où le comportement qu'AC3 exige : trois vols dont deux concordants et un
divergent sur la cause rendent **2** si le divergent est le plus ancien, et
**0** s'il est le plus récent. Le compte dépend de l'ordre, et c'est voulu.

La fenêtre est `inscription_window_start()` — la même que celle de
l'idempotence des deux écrivains, dérivée d'`AUDIT_RETENTION_DAYS` et non
écrite ici (même geste que `degroom_marker_lookup_since`, mika#2242) : une
fenêtre de lecture plus large que la rétention lirait un « jamais inscrit » là
où il y a « inscrit puis purgé ». L'en-tête dit le nombre de jours, pris sur la
même constante. **Aucun drapeau `--days` :** un seuil de 7 jours lu sur une
fenêtre réglable par l'appelant n'est pas une mesure, et la fenêtre ne doit pas
pouvoir survivre aux lignes qu'elle lit.

### 6. Le rendu réemploie le vocabulaire français de `reporter.rs`

Le rapport du manager rend déjà `FlightIssue` et `FlightRecommendation` en
français : `flight_issue_label` (`reporter.rs:814`) et
`flight_recommendation_label` (`reporter.rs:850`), privées. Elles passent en
`pub(crate)` et le rendu les appelle. Écrire un second vocabulaire dans le
lecteur serait une seconde table de graphies à tenir en phase avec trois
`enum` — la classe mika#2158, celle que les deux vols précédents ont déjà
refusée sur l'axe de la sérialisation.

Un changement de visibilité ne touche aucun jeton de `no_dispatch_test.rs` :
`FORBIDDEN_TOKENS` ne porte que des jetons d'autorité d'écriture sur la forge.

Forme du rendu texte (le détail est à l'implémentation) :

```text
  Concordance manager / MPC — 3 vols, fenêtre de 90 jours
  2 vols consécutifs concordants (du plus récent au plus ancien ; seuil mika#2698 : 10 vols ou 7 jours)

  vol        ticket  manager                       MPC                           accord
  a1b2c3d4   #2931   PR ouverte (#2931)            PR ouverte (#2931)            concordant
  e5f6a7b8   #2928   mort au plafond (budget_ex…)  mort — escalade               divergent (cause)
  c9d0e1f2   —       re-groom                      —                             non jugé par MPC
```

`--format json|yaml` sérialise le même type de rapport : `text` est la surface
humaine, et c'est pourquoi le bras `Text` n'est **pas** l'« échappatoire JSON
pretty » de `commands/milestone.rs::emit` (l. 238-243). `concordance` porte
`--format` là où `judge` n'en porte pas, et le critère est celui que
`crates/mika-cli/CLAUDE.md` écrit déjà : « Pas de `--format` : `judge` est un
verbe d'écriture ». `concordance` est un verbe de lecture.

### 7. La garde de lecteur unique naît ici, et elle n'est pas vide de sens

Elle balaie `crates/mika-agent/src` (hors sources de test, via
`source_scan::is_test_source_path`) et refuse un **second** site nommant l'un
des deux `tool_name` en lecture, hors du module du registre. Elle a un lecteur
réel à protéger, donc elle n'est pas verte par construction — c'est la
condition que le vol 1 posait pour l'écrire.

Son anti-vacuité est obligatoire (classe mika#2205) : `files.len() >= 100`
avant toute assertion d'ensemble, plus le contrôle négatif sur une source
fabriquée portant le geste refusé. Sans eux, « aucun second lecteur » et « le
balayage n'a rien lu » rendent le même vert.

Quand elle rougit, le remède est de router le site par le lecteur — **jamais**
d'ajouter une entrée (doctrine mika#2201). La liste d'exception est *absente*,
pas vide.

### 8. AC5 — aucune autorité ajoutée

`MIKA_MANAGER_EVALUATE_WRITE` n'est ni lu ni nommé par ce vol. La commande est
en lecture seule : elle n'écrit aucune ligne d'audit, n'ouvre aucune
transaction en écriture, et ne touche ni la forge ni le dispatch.
`no_dispatch_test.rs` passe sans modification de son ensemble de jetons, et le
seul fichier touché sous `src/milestone_manager/` est `reporter.rs`, pour deux
mots-clés de visibilité.

---

## Ordre de commit

1. **L'accesseur** — `db.rs` (`list_audit_events_for_tool_since` +
   `AuditEventWindowRow`) et son enveloppe `async_db.rs`, avec le test qui
   prouve que la borne `since` exclut bien une ligne antérieure et que
   `created_at` remonte.
2. **Le lecteur** — types du rapport, appariement sur `tasks.id`, comparaison
   par `terms()`, premier champ divergent, compte consécutif, ticket lu sur la
   ligne du manager, orphelins comptés. Avec les tests AC3 et AC4 sur une base
   réelle.
3. **Le rendu** — texte, et la promotion des deux étiquettes de `reporter.rs`.
4. **La sous-commande** — `cli.rs` (bras `Concordance { format }`) et
   l'adaptateur dans `commands/flight_verdicts.rs`, qui ne fait qu'appeler le
   lecteur et rendre.
5. **La garde de lecteur unique** et son contrôle négatif.
6. **La doc** — les deux sections de `docs/agent-internals/audit-log.md`
   (l'« Operator query » devient le pis-aller, la commande est le chemin),
   `crates/mika-cli/CLAUDE.md § mika flight-verdicts`, et la sonde d'AC6.

**Plancher mergeable : les commits 1 à 4.** À ce point l'instrument existe,
AC3 et AC4 sont attestés, et la mesure que Prime exige est lisible en une
commande. Les commits 5 et 6 durcissent et documentent.

---

## Definition of Done

- `cargo test` vert sur l'ensemble du dépôt ; `cargo clippy` sans
  avertissement nouveau ; `cargo fmt` appliqué.
- `scripts/verify-pipeline.sh origin/main` vert.
- `mika flight-verdicts concordance` existe, rend les six colonnes et l'en-tête
  d'AC3, et accepte `--format text|json|yaml`.
- Le lecteur est unique et la garde qui l'atteste porte son anti-vacuité
  (`files.len() >= 100`) et son contrôle négatif.
- `crates/mika-agent/src/db.rs` : `CURRENT_SCHEMA_VERSION` reste à **56**.
  Aucune table, aucune migration — `audit_events` suffit, et c'est le patron
  que le ticket désigne (mika#2917).
- `milestone_manager/no_dispatch_test.rs` passe sans modification de
  `FORBIDDEN_TOKENS`, `EXEMPT_FILES` ni `EXEMPTED_OCCURRENCES`.
- Aucune des quatre aiguilles de
  `mika2922_ladaptateur_nest_pas_un_ecrivain` n'apparaît dans
  `crates/mika-cli/src/` : la garde du vol 2 reste verte sans être touchée.
- `docs/agent-internals/audit-log.md` dit, dans les deux sections, que le
  lecteur existe et le nomme ; les blocs **Operator query** sont présentés
  comme le pis-aller d'avant la commande.
- Le corps de PR porte `Code review: delegated to mika-qa (mika#2694)`.
- Le merge revient à Vincent (`loop-substrate`).

---

## Acceptance criteria

Transcrits verbatim du corps du ticket, avec l'attribution par vol. Ce vol
porte **AC3**, **AC4** et la seconde moitié d'**AC6** ; les autres sont
attestés par les vols 1 et 2, déjà mergés, et doivent rester verts.

- **AC1, l'inscription.** Après un cycle sur un corpus de vols, chaque vol jugé
  a exactement une ligne de verdict. Un second cycle sur les mêmes vols n'en
  ajoute aucune. Test sur une vraie base.
  *(Vol 1 — `write_and_swallow` + `InscriptionTally`. Reste vert.)*

- **AC2, un seul écrivain par `tool_name`.** Une garde fait rougir un second
  site d'écriture du verdict du manager, et un second site d'écriture du
  jugement de MPC.
  *(Vol 1 pour le manager, vol 2 pour MPC, plus la garde côté `mika-cli`.
  Restent verts, et ce vol n'y touche pas : son lecteur ne nomme aucun
  `tool_name` hors du module du registre.)*

- **AC3, la concordance.** Un corpus de 3 vols en donne 2 concordants et 1
  divergent sur la cause de mort. Le compte rend « 2 consécutifs » ou « 0 »,
  selon l'ordre, et nomme le champ divergent. Il y a un contrôle positif (tout
  concorde) et un contrôle négatif (une divergence) dans le même test.
  *(**Ce vol.** Un seul test, quatre assertions : les trois vols avec le
  divergent en queue ⇒ 2 et `cause` nommé ; les mêmes trois avec le divergent
  en tête ⇒ 0 ; le contrôle positif à trois concordants ⇒ 3 ; et le contrôle
  négatif dans le même test, qui est la divergence elle-même.)*

- **AC4, aucun verdict n'est inventé.** Un vol que MPC n'a pas jugé apparaît
  comme « non jugé par MPC » et ne compte ni pour ni contre.
  *(**Ce vol.** Le vol non jugé est sauté par le compte — il n'incrémente pas
  et il n'arrête pas — et le rendu l'écrit explicitement. Aucun `VerdictRecord`
  fabriqué pour combler : motif mika#2331, déjà appliqué par l'adaptateur du
  vol 2.)*

- **AC5, aucune autorité.** `MIKA_MANAGER_EVALUATE_WRITE` reste lu exactement
  comme avant. `no_dispatch_test.rs` passe sans modification de son ensemble de
  jetons.
  *(Tenu par les trois vols. Ici : le lecteur vit hors du module gardé, la
  commande est en lecture seule, et le seul fichier touché sous
  `src/milestone_manager/` l'est pour deux `pub(crate)`.)*

- **AC6, la sonde post-déploiement**, écrite dans le plan : après
  `make deploy`, au premier cycle qui juge un vol, la ligne existe en base, et
  la commande de concordance la rend avec « non jugé par MPC ».
  *(Première moitié : vol 1 (la ligne) et vol 2 (l'écriture de MPC). **Seconde
  moitié : ce vol** — la commande qui la rend. Voir § Sonde post-déploiement.)*

---

## Fire-Disposition

Ce vol livre des détecteurs : la garde de lecteur unique et son contrôle
négatif, plus les tests comportementaux d'AC3 et d'AC4. La section est donc
requise.

**Option (a) — exception nommée en allowlist, et l'allowlist est livrée
VIDE.** C'est la même disposition que les vols 1 et 2, pour la même raison :
il n'y a aucune violation préexistante à excepter, parce que le lecteur naît
avec sa garde.

- **(a.1) L'ensemble des lecteurs autorisés est en dur, pas en allowlist.** La
  garde admet le module du registre et rien d'autre. Une allowlist de fichiers
  serait une liste à tenir ; un ensemble en dur à un élément est une propriété.
  Quand elle rougit, le remède est de router le site par le lecteur, jamais
  d'ajouter une ligne (doctrine mika#2201) — et le message d'échec le dit, sur
  l'idiome de celui du vol 2.
- **(a.2) Anti-vacuité obligatoire.** `files.len() >= 100` avant toute
  assertion d'ensemble, et l'assertion explicite que le balayage a bien vu
  `flight_verdict_ledger.rs`. Un balayage qui ne voit pas le lecteur ne dit
  rien du lecteur (classe mika#2205) — c'est la forme exacte que porte déjà
  `mika2922_ladaptateur_nest_pas_un_ecrivain`.
- **(a.3) Contrôle négatif sur source fabriquée.** Le prédicat est reproduit à
  l'identique sur deux sources : l'une porte le geste refusé (une lecture
  nommant `MANAGER_FLIGHT_VERDICT_TOOL` hors du module), l'autre une lecture
  **légitime** d'`audit_events` sous un autre `tool_name` — c'est-à-dire la
  population que la garde ne doit pas refuser. Sans lui, une aiguille mal tapée
  laisserait la garde verte pour la mauvaise raison.
- **(a.4) Les aiguilles sont concaténées**, comme celles du vol 2, pour que la
  garde ne morde pas sur elle-même et ne dépende pas de l'exactitude du
  masquage de `cfg(test)`.
- **(a.5) Aucun littéral de `tool_name` recopié hors du module du registre**,
  ni en production ni dans les tests de ce vol : la garde du vol 2 est encore
  la preuve que la discipline tient côté CLI, et elle n'a pas à être affaiblie
  pour qu'un test lise.
- **(a.6) Les tests AC3/AC4 tournent sur une base réelle** (`open_in_memory`
  via `AsyncDatabase`), avec les lignes écrites par les **écrivains** du vol 1
  et du vol 2 — jamais par du SQL de fixture. Un corpus posé à la main
  attesterait le lecteur contre une forme de ligne inventée par le test, ce qui
  est la façon dont un instrument se met d'accord avec lui-même.

**Aucun détecteur n'est livré désarmé** : il n'y a ni `#[ignore]` ni
`#[cfg(skip)]` dans ce vol, et aucune halte à remonter.

---

## Sonde post-déploiement (la seconde moitié d'AC6)

La chaîne est **rebuild → seed → read** (motif mika#2340) : avant
`make deploy`, l'absence de sortie signifie un binaire antérieur à ce vol, et
**jamais** une absence de phénomène. Ne pas conclure sur une sous-commande
inconnue.

1. `make deploy`.
2. `mika flight-verdicts concordance --format text` — attendu, dès avant tout
   cycle : l'en-tête, la fenêtre de 90 jours, et `0 vols` si le registre est
   vide. Un registre vide se **dit** ; il ne rend pas une sortie muette.
3. Attendre le premier cycle du manager qui juge un vol (ou le provoquer).
   Vérifier la ligne du manager :
   `mika flight-verdicts concordance --format json | jq '.rows | length'` ≥ 1.
4. Attendu, et c'est le cœur d'AC6 : ce vol apparaît avec **« non jugé par
   MPC »**, et l'en-tête rend **0 vols consécutifs concordants** — pas une
   erreur, pas une ligne absente, et surtout pas un accord.
5. `mika flight-verdicts judge --task-id <le même tasks.id> --issue … [--cause …]
   --recommendation …` puis re-lire : le même vol doit passer à `concordant`
   ou à `divergent (<champ>)`, et le compte doit bouger de 0 à 1 dans le
   premier cas.
6. Contre-sonde de la fenêtre : la ligne « fenêtre de 90 jours » de l'en-tête
   doit porter la valeur d'`AUDIT_RETENTION_DAYS`. Si elle en diffère, la
   fenêtre a été écrite quelque part au lieu d'être dérivée, et le § 5 de la
   conception est violé.

Ce que la sonde **ne** prouve pas, et qu'il ne faut pas lui faire dire : elle
ne mesure pas l'accord d'ombre. Elle prouve que l'instrument existe et qu'il
lit les deux colonnes. Les « 10 vols consécutifs ou 7 jours » de mika#2698 se
constatent ensuite, en exécutant la commande dans le temps — c'est ce que MPC
présentera à Vincent, et c'est hors de ce vol.

---

## Alternatives rejetées

- **Un lecteur côté `mika-cli`.** Refusé pour deux raisons indépendantes, et
  chacune suffit : la garde du vol 2 refuse les quatre aiguilles dans ce crate,
  et `source_scan::production_files()` est enraciné sur `crates/mika-agent/src`,
  donc la garde de lecteur unique y serait aveugle et verte — classe mika#2205.
- **Étendre `get_audit_event_rows_by_tool_name` avec `created_at` et une
  borne.** Refusé : `#[doc(hidden)]`, documentée comme helper de tests
  mika#1712, ~25 appelants. Le vol deviendrait un patch de tests sans rapport
  avec la concordance.
- **Une table `flight_verdict_concordance` ou une vue SQL.** Refusé : le patron
  que le ticket désigne (mika#2917) dit « aucune table ni migration, si
  `audit_events` suffit », et il suffit. Le schéma reste à v56.
- **Comparer les `VerdictRecord` par égalité.** Refusé : `PrOpened`/`RescuePr`
  portent `number`/`url` que MPC écrit `null`, donc un accord parfait se
  lirait divergent. La comparaison est sur `terms()`.
- **Compter les concordances sur toute la fenêtre plutôt qu'en série depuis le
  plus récent.** Refusé : mika#2698 dit « sur 10 vols **consécutifs** », et un
  taux d'accord global masquerait précisément la régression que le seuil
  surveille — une série cassée hier.
- **Un drapeau `--days` pour la fenêtre.** Refusé : la fenêtre de lecture ne
  doit pas pouvoir dépasser la rétention, sinon « jamais inscrit » et « inscrit
  puis purgé » se confondent. Elle est dérivée d'`AUDIT_RETENTION_DAYS`, comme
  celle de l'idempotence des deux écrivains.
- **Un verbe de correction (`judge --force` / `amend`).** Refusé, et le critère
  de bascule est celui que le plan du vol 2 a écrit pour ne pas être re-dérivé :
  « la première correction réellement nécessaire, constatée ». Elle ne l'est
  pas.
- **Faire rendre un verdict d'ouverture par la commande.** Refusé : la commande
  mesure, Vincent décide. Un instrument qui conclut est un instrument qu'on
  croit.
- **Écrire un second vocabulaire français dans le lecteur** plutôt que de
  promouvoir les deux étiquettes de `reporter.rs`. Refusé : classe mika#2158,
  la divergence programmée d'une seconde table de graphies.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db.rs` — `list_audit_events_for_tool_since` + `AuditEventWindowRow` | 26 |
| `crates/mika-agent/src/async_db.rs` — son enveloppe | 14 |
| `crates/mika-agent/src/flight_verdict_ledger.rs` — le lecteur (types du rapport, appariement, premier champ divergent, compte consécutif, orphelins, `const` du préfixe) | 135 |
| `crates/mika-agent/src/flight_verdict_ledger.rs` — le rendu texte | 32 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` — promotion des deux étiquettes | 4 |
| `crates/mika-cli/src/cli.rs` — le bras `Concordance { format }` | 14 |
| `crates/mika-cli/src/commands/flight_verdicts.rs` — l'adaptateur et les trois formats | 22 |
| garde de lecteur unique + contrôle négatif (`flight_verdict_ledger.rs`) | 52 |
| tests AC3/AC4 sur base réelle (`flight_verdict_ledger.rs`) | 105 |

Total estimé : 404 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol, et pourquoi ce total tient.** 404 < 429, et le
découpage en trois vols a déjà été fait : ce plan est le troisième. Le découper
davantage produirait précisément ce que le plan du vol 2 a rejeté en ces termes
— « livrer le lecteur de concordance avant l'écrivain de MPC […] un instrument
qui ne peut mesurer que zéro » — dans l'autre sens : un lecteur sans commande,
ou une commande sans compte, ne mesure rien et n'est pas mergeable seul.

**La marge est mince, et elle est nommée.** Le plan du vol 2 a documenté que
l'estimation du vol 1 sous-comptait de 39 %. Appliquée ici, la même erreur
donnerait ~561 lignes, au-dessus de la borne. Deux choses bornent ce risque :

1. **Aucun livrable n'est neuf en nature.** Les neuf lignes du tableau sont
   toutes mesurées sur des sites existants : l'accesseur se compte sur ses
   quatre voisins de `db.rs` (14 à 23 lignes chacun), l'enveloppe sur ses trois
   voisines d'`async_db.rs` (9 à 11 lignes), la garde sur celle du vol 2
   (48 lignes de code), et l'adaptateur sur le bras `Judge` existant (48 lignes
   pour un bras qui écrit et rend deux issues ; `concordance` en rend une).
   Le vol 1 estimait sur des sites qui n'existaient pas encore.
2. **Une soupape nommée d'avance, parce que la nommer pendant le vol coûte un
   tour qu'on n'aura pas.** Le seul livrable qu'**aucun AC** n'exige est la
   garde de lecteur unique et son contrôle négatif (52 lignes) : AC2 ne parle
   que de sites d'**écriture**. Si le plancher (commits 1 à 4) a coûté plus que
   son budget, le commit 5 est reporté — la PR le déclare par une ligne
   `Tracked in:` nommant le ticket de suivi que le pilote ouvre, ce que
   `pr-body-validation.yml` exige déjà pour tout report. Le commit 6 (doc) ne
   compte pas dans le total et reste dû.

Le total est **sous** les deux bornes — 404 < 429 par vol et 404 < 1000 par PR
— donc aucun découpage en phases ni en vols n'est requis, et il n'y a pas de
suite à nommer par `Tracked in:` : ce vol ferme le ticket.
