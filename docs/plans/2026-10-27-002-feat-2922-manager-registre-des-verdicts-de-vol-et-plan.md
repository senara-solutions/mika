# mika#2922 — le registre des verdicts de vol (vol 2/3) : le jugement de MPC

**Ticket** : `senara-solutions/mika#2922`
**Vol** : 2 sur 3 — *le jugement de MPC, et son chemin supporté*
**Branche** : `feat/2922/manager-registre-des-verdicts-de-vol-et`
**Plan du vol 1** (mergé, `598055bfe` / PR #2928) : `docs/plans/2026-10-26-003-feat-2922-manager-registre-des-verdicts-de-vol-et-plan.md`

---

## Contexte — ce que le vol 1 a posé, et ce qui manque encore

Le vol 1 a livré le **livrable 1** du ticket et la première moitié d'AC2 :
`crates/mika-agent/src/flight_verdict_ledger.rs` inscrit chaque verdict de
l'évaluateur dans `audit_events` sous `tool_name = "manager_flight_verdict"`,
une ligne par vol et par premier cycle qui le juge, clé `tasks.id`, sans
réécriture. Une garde d'ensemble exact attestée anti-vacuité refuse un second
écrivain. Le module vit **hors** de `milestone_manager/`, sur l'idiome de
`flight_source.rs`, et `no_dispatch_test.rs` passe sans que son ensemble de
jetons bouge (AC5, première moitié).

Ce qui manque pour que le seuil de mika#2698 — *« l'évaluateur rend, en ombre,
les mêmes verdicts que MPC sur 10 vols consécutifs ou 7 jours »* — devienne
mesurable :

1. **Le jugement de MPC n'est écrit nulle part.** La colonne d'en face est vide.
   C'est le **livrable 2** du ticket, et c'est ce vol.
2. **Le compte de concordance n'a pas de lecteur.** C'est le **livrable 3**, et
   c'est le vol 3.

Le ticket reste ouvert après le merge de ce vol.

## Le re-découpage, et pourquoi il diffère de ce que le vol 1 annonçait

Le plan du vol 1 annonçait un vol 2 portant **les livrables 2 et 3 ensemble**,
pour ≈ 320 lignes. Le re-mesurage fait ici, sur les sites réels, donne
**≈ 594 lignes** pour cet ensemble — 39 % au-dessus de la borne par vol de 429.
L'estimation du vol 1 sous-comptait trois postes qui n'existent qu'à l'écriture :

- la **surface CLI** elle-même (déclaration `clap` + adaptateur + câblage) —
  comptée pour « ≈ 100 incluant les bras de `match` », mesurée à ≈ 98 pour la
  **seule** sous-commande d'écriture, et autant pour celle de lecture ;
- la **promotion des accesseurs** (`db.rs` et son miroir `async_db.rs`) et des
  deux étiquettes de rendu de `reporter.rs`, qui sont du travail réel sur quatre
  fichiers ;
- le **rendu** du compte, qui n'était pas distingué du lecteur.

Ce dépôt a historiquement **sous**-estimé (mika#2683 : 832 estimées pour 1 432
réelles). Au-dessus de la borne, la règle mika#2693 impose de découper en vols
plutôt qu'en phases d'un même vol, et c'est ce qui est fait ici.

**La coupe suit la ligne écrivain / lecteur — la même que celle du vol 1.** Le
vol 1 a livré l'écrivain du manager sans son lecteur, pour la raison écrite dans
son § *Découpage* : « le lecteur arrive au vol 2 et l'assertion naîtra avec
lui ». Le vol 2 livre l'écrivain de MPC sans son lecteur, et le vol 3 livre le
lecteur **unique** des deux colonnes.

**Et cette coupe ne retarde pas la date à laquelle le seuil peut être atteint.**
Le seuil de mika#2698 est une **accumulation dans le temps** : 10 vols
consécutifs ou 7 jours. Il ne peut être atteint avant que les deux écrivains
aient tourné ensemble sur une fenêtre entière. Un lecteur qui arrive un ou deux
jours après le second écrivain ne coûte rien à cette horloge ; un écrivain qui
arrive un ou deux jours plus tard la décale d'autant. Livrer l'écriture
d'abord est donc l'ordre qui fait avancer la mesure au plus tôt.

## Ce que livre ce vol

Le **livrable 2** du ticket, au complet :

> Le jugement de MPC s'inscrit sous la même clé, par un chemin supporté (par
> exemple une sous-commande `mika`), jamais par du SQL à la main. Le jugement
> porte la même grammaire : issue, cause, recommandation.

Concrètement :

- `MPC_FLIGHT_JUDGMENT_TOOL` / `MPC_FLIGHT_JUDGMENT_SESSION_ID` et leur écrivain
  unique, dans `flight_verdict_ledger.rs`, à côté de celui du manager ;
- l'analyse des trois termes saisis en ligne de commande vers les types
  existants, **inverse exact** des littéraux serde ;
- la sous-commande `mika flight-verdicts judge`, adaptateur mince ;
- la **seconde moitié d'AC2** (garde d'écrivain unique du jugement de MPC + son
  contrôle négatif) et la part d'**AC5** qui concerne le nouveau site.

AC3, AC4 et la seconde moitié d'AC6 partent au vol 3.
**Tracked in: senara-solutions/mika#2922 (vol 3 — le compte de concordance).**

---

## Conception

### 1. L'écrivain vit dans `flight_verdict_ledger.rs`, à côté de celui du manager

Le plan du vol 1 l'a déjà tranché et la raison tient toujours :

> `MPC_FLIGHT_JUDGMENT_TOOL` n'est pas déclaré au vol 1. Une constante que
> personne n'écrit ferait une garde d'écrivain dont l'ensemble exact est vide
> […] Elle naît avec son écrivain, dans le même fichier, à côté de celle du
> manager.

Elle naît donc ici. Et elle naît **dans `crates/mika-agent/src/`**, pas dans
`crates/mika-cli/`, pour une raison structurelle qui n'est pas une préférence :
`source_scan::production_files()` est enracinée sur
`Path::new(env!("CARGO_MANIFEST_DIR")).join("src")` du crate `mika-agent`, donc
`audit_writers()` est **aveugle** à `crates/mika-cli/`. Un écrivain côté CLI
rendrait la garde d'AC2 **vacuement verte** : elle attesterait « un seul
écrivain » en n'ayant jamais regardé là où il est. C'est la classe mika#2205, et
c'est précisément ce que l'anti-vacuité `files.len() >= 100` du vol 1 existe
pour refuser.

La sous-commande est donc un **adaptateur mince**, sur la doctrine *Wrapper-only
INV-2* que `crates/mika-cli/src/commands/milestone.rs` énonce pour lui-même :
« No GitHub logic lives here — the CLI dispatches straight into the agent-crate
module ». Ici : aucune logique d'inscription côté CLI, aucun `log_audit_event`,
aucune recomposition de la clé.

La forme de ligne est celle que le vol 1 a annoncée :

```text
tool_name   = MPC_FLIGHT_JUDGMENT_TOOL     ("mpc_flight_judgment")
session_id  = MPC_FLIGHT_JUDGMENT_SESSION_ID ("mpc-flight-judgment")
target_key  = le MÊME tasks.id             (c'est ce qui met les deux en face)
after_value = JSON serde de VerdictRecord { issue, recommendation }
before_value= NULL                          (aucun état antérieur à nommer)
reasoning   = la note libre de l'opérateur (--note), ou NULL
```

**`reasoning` ne porte pas la même donnée des deux côtés, et c'est délibéré.**
Côté manager il porte `issue_number=<n>` ; côté MPC il porte une note libre. Le
vol 3 lira donc le ticket d'un vol sur la ligne **du manager**, jamais sur celle
de MPC — qui n'en a pas et n'a pas à en inventer une. C'est écrit ici pour que
le vol 3 n'ait pas à le redécouvrir.

### 2. L'analyse des termes est l'inverse exact de serde, par construction

La sous-commande prend trois termes en clair :

```
mika flight-verdicts judge --task-id <tasks.id> \
     --issue dead --cause refusal --recommendation regroom [--note "…"]
```

Les valeurs acceptées sont **exactement** les littéraux serde des trois `enum`,
ceux que `VerdictRecord::terms()` rend déjà au vol 1 :

| terme | valeurs |
|---|---|
| `--issue` | `pr_opened` · `rescue_pr` · `dead` |
| `--cause` | `ceiling_reached` · `refusal` · `pipeline_failure` · `launcher_dead` · `escalated` |
| `--recommendation` | `merge_candidate` · `manual_resumption` · `regroom` |

**L'analyse ne recopie aucune de ces tables.** Elle construit une
`serde_json::Value` de la forme que `#[serde(rename_all = "snake_case")]`
produit (`{"dead":{"cause":"refusal"}}`, `{"pr_opened":{"number":null,"url":null}}`)
et la passe à `serde_json::from_value::<FlightIssue>` /
`::<FlightRecommendation>`. Une seconde table de correspondance écrite à la main
serait la classe mika#2158 — la regex de `grooming_marker` dont le commentaire
disait « Mirrors … » et qui a ensuite raté deux élargissements. Ici, une
sixième cause de mort ajoutée à `DeathCause` est acceptée par l'analyseur **sans
qu'une ligne bouge**, et un terme mal orthographié est refusé par le même
mécanisme qui a sérialisé la ligne du manager.

`number` et `url` sont écrits `null` : MPC ne connaît pas le numéro de la PR, et
`VerdictTerms` ne les compare pas (c'est pour cela que la projection
décisionnelle existe depuis le vol 1). La ligne de MPC est donc honnête — elle
ne prétend pas un numéro — sans cesser d'être comparable.

**Une cause non classée n'est pas saisissable, et c'est un refus nommé.**
`DeathCause::Unknown(String)` est produite par le manager quand il rencontre une
famille de halte qu'il ne connaît pas ; elle n'est pas offerte à MPC. Une cause
hors des cinq classées ne peut jamais concorder avec un verdict du manager, donc
l'accepter en silence fabriquerait une divergence permanente à partir d'une faute
de frappe. Le refus énumère les cinq valeurs. La forme `unknown:<jeton>` que
`cause_term` rend côté manager reste **lisible** au vol 3 — c'est l'asymétrie
correcte : on lit ce que le producteur a écrit, on ne laisse pas saisir ce
qu'aucun producteur ne produit.

### 3. L'erreur remonte, elle n'est pas avalée — à l'inverse de l'écrivain du manager

Le vol 1 a `write_and_swallow` : un échec d'écriture est journalisé en `WARN` et
le cycle d'évaluation continue, parce qu'un registre défaillant ne doit pas tuer
le rapport. Ici le contexte est l'opposé : il y a un opérateur en face, qui a
tapé une commande et attend de savoir si son jugement est en base.

```rust
pub async fn record_mpc_judgment(
    db: &AsyncDatabase,
    task_id: &str,
    record: &VerdictRecord,
    note: Option<&str>,
) -> Result<MpcJudgmentOutcome>;

pub enum MpcJudgmentOutcome { Inscribed, AlreadyRecorded }
```

Un échec rend `Err` et la sous-commande sort en code non nul avec la cause
nommée. Un « jugement avalé » qui sortirait en 0 est exactement la silence dont
le ticket se plaint au § *Pourquoi*.

### 4. L'idempotence réemploie la fenêtre et l'accesseur du vol 1

`count_recent_audit_events_for_target(MPC_FLIGHT_JUDGMENT_TOOL, task_id, since)`
avec `since = inscription_window_start()` — **la fonction du vol 1**, bornée par
`AUDIT_RETENTION_DAYS = 90`, pas une seconde. Non nul ⇒ `AlreadyRecorded`, aucune
seconde ligne, et la sous-commande **rend le jugement déjà inscrit** plutôt qu'un
simple « déjà là » : l'opérateur qui re-saisit veut voir ce qui est en face.

**Pas de réécriture, et pas de verbe de correction dans ce vol.** « Pas de
réécriture » est la règle que le ticket pose au livrable 1 ; l'appliquer au
livrable 2 garde les deux colonnes sous la même loi. Un verbe de correction est
nommé en alternative rejetée ci-dessous, avec son critère de bascule.

### 5. La sous-commande, et comment MPC trouve un `tasks.id`

`mika flight-verdicts judge` suit le câblage de `mika tasks` plutôt que celui de
`mika milestone` : il lui faut une base (`init::init_db_only_for_agent`), pas un
jeton GitHub. Un groupe `flight-verdicts` plutôt qu'un verbe nu, parce que le
vol 3 y ajoute `concordance` — la population est de **deux** dès la suite nommée,
ce qui est le critère que `crates/mika-cli/CLAUDE.md` § `mika plan-callout`
énonce pour refuser une famille à population de un.

**Ce que ce vol ne donne pas : l'index des vols à juger.** Obtenir un `tasks.id`
passe aujourd'hui par `mika tasks list` / `mika tasks get`. C'est maigre, et
c'est dit plutôt que masqué. L'ergonomie se referme **par construction** au
vol 3 : le compte de concordance rend une ligne par vol inscrit, avec son ticket,
le verdict du manager et « non jugé par MPC » pour ceux qui attendent — c'est
exactement l'index dont MPC a besoin, et il n'y a donc aucune surface de liste à
livrer ici qui ne serait pas la concordance à 80 %.

### 6. AC5 — aucune autorité ajoutée

Le nouveau code vit dans le **même fichier** que celui du vol 1, hors de
`milestone_manager/`. `no_dispatch_test.rs` grep les `.rs` **du module** ; ce
fichier n'en est pas, et son ensemble de 22 jetons n'a pas à bouger. La garde
`mika2922_ac5_le_module_ne_nomme_pas_linterrupteur` du vol 1 est **étendue** au
nouveau nom d'outil : la moitié production contient `MPC_FLIGHT_JUDGMENT_TOOL`,
ne contient ni `ENV_EVALUATE_WRITE` ni `write_is_armed`, et le fichier n'est pas
sous `milestone_manager/`. `MIKA_MANAGER_EVALUATE_WRITE` reste lu exactement
comme avant — ce vol ne touche aucun de ses sites.

---

## Ordre de commit

1. `flight_verdict_ledger.rs` — les deux constantes, l'analyse des termes,
   `record_mpc_judgment`, `MpcJudgmentOutcome`.
2. `flight_verdict_ledger.rs` `mod tests` — les gardes d'AC2 (écrivain unique +
   contrôle négatif), l'extension d'AC5, l'aller-retour des termes,
   l'idempotence sur vraie base.
3. `crates/mika-cli/src/cli.rs` + `commands/flight_verdicts.rs` +
   `commands/mod.rs` + `main.rs` — la sous-commande et son câblage.
4. `crates/mika-agent/CLAUDE.md` § *Audit Log* — l'entrée `mpc_flight_judgment`,
   sur la forme exacte de l'entrée `manager_flight_verdict` écrite au vol 1
   (tableau de forme de ligne, requête opérateur, propriétés porteuses).
   `crates/mika-cli/CLAUDE.md` — la sous-commande dans la liste.

Chaque commit compile et passe `cargo test -p mika-agent` ; le plancher du vol
est le commit 2 (l'écrivain sans sa surface CLI est déjà un écrivain gardé).

---

## Definition of Done

- [ ] `MPC_FLIGHT_JUDGMENT_TOOL` et `MPC_FLIGHT_JUDGMENT_SESSION_ID` déclarés
      dans `flight_verdict_ledger.rs`, avec leur doc-comment nommant l'écrivain
      unique et le lecteur à venir.
- [ ] `record_mpc_judgment` écrit une ligne `audit_events` sous la clé
      `tasks.id`, `after_value` = JSON serde de `VerdictRecord`, `before_value`
      NULL, `reasoning` = la note ou NULL.
- [ ] Une seconde saisie sur le même `tasks.id` dans la fenêtre rend
      `AlreadyRecorded` et n'ajoute **aucune** ligne.
- [ ] L'analyse des termes passe par `serde_json::from_value` sur les types
      existants ; aucune seconde table de correspondance n'est écrite.
- [ ] Un terme hors vocabulaire est refusé en nommant les valeurs acceptées ;
      `--cause` est exigé pour `dead` et refusé sinon.
- [ ] `mika flight-verdicts judge` inscrit via `record_mpc_judgment` et rien
      d'autre ; sortie non nulle sur échec, cause nommée.
- [ ] `mika2922_ac2_un_seul_ecrivain_du_jugement_de_mpc` : ensemble exact
      `["flight_verdict_ledger.rs"]`, anti-vacuité `files.len() >= 100`, aucun
      littéral `"mpc_flight_judgment"` recopié ailleurs.
- [ ] Le contrôle négatif de la fenêtre de scan existe pour le nouveau nom.
- [ ] `mika2922_ac5_le_module_ne_nomme_pas_linterrupteur` étendu au nouveau nom
      d'outil et toujours vert.
- [ ] `cargo test`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`
      et `scripts/verify-pipeline.sh origin/main` passent.
- [ ] `crates/mika-agent/CLAUDE.md` § *Audit Log* porte l'entrée
      `mpc_flight_judgment` ; `crates/mika-cli/CLAUDE.md` porte la sous-commande.
- [ ] Le corps de PR porte `Tracked in: senara-solutions/mika#2922 (vol 3 — le
      compte de concordance)` et ne ferme pas le ticket.

## Acceptance criteria

Transcrits **verbatim** du corps de `senara-solutions/mika#2922`. L'attribution
par vol suit chaque critère ; elle est une annotation de ce plan, pas une
modification du critère.

- **AC1, l'inscription.** Après un cycle sur un corpus de vols, chaque vol jugé a exactement une ligne de verdict. Un second cycle sur les mêmes vols n'en ajoute aucune. Test sur une vraie base.

  *(Livré au vol 1 — `mika2922_ac1_chaque_vol_juge_a_exactement_une_ligne`.)*

- **AC2, un seul écrivain par `tool_name`.** Une garde fait rougir un second site d'écriture du verdict du manager, et un second site d'écriture du jugement de MPC.

  *(Première moitié livrée au vol 1 ; **la seconde moitié — le jugement de MPC — est livrée par ce vol**.)*

- **AC3, la concordance.** Un corpus de 3 vols en donne 2 concordants et 1 divergent sur la cause de mort. Le compte rend « 2 consécutifs » ou « 0 », selon l'ordre, et nomme le champ divergent. Il y a un contrôle positif (tout concorde) et un contrôle négatif (une divergence) dans le même test.

  *(Vol 3.)*

- **AC4, aucun verdict n'est inventé.** Un vol que MPC n'a pas jugé apparaît comme « non jugé par MPC » et ne compte ni pour ni contre.

  *(Vol 3 — c'est un critère sur le **rendu** du compte, qui n'existe pas avant lui.)*

- **AC5, aucune autorité.** `MIKA_MANAGER_EVALUATE_WRITE` reste lu exactement comme avant. `no_dispatch_test.rs` passe sans modification de son ensemble de jetons.

  *(Tenu au vol 1 et **ré-attesté par ce vol** sur le nouveau site : la garde est étendue au nouveau nom d'outil.)*

- **AC6, la sonde post-déploiement**, écrite dans le plan : après `make deploy`, au premier cycle qui juge un vol, la ligne existe en base, et la commande de concordance la rend avec « non jugé par MPC ».

  *(Première moitié — la ligne du manager en base — couverte au vol 1. **Ce vol ajoute la moitié d'écriture de MPC** (§ Sonde ci-dessous). La seconde moitié — le rendu par la commande de concordance — est au vol 3.)*

---

## Fire-Disposition

Ce vol livre des détecteurs : deux gardes structurelles neuves (l'écrivain
unique du jugement de MPC et le contrôle négatif de sa fenêtre de scan) et
l'extension d'une garde existante. **Option (a) — exception nommée en allowlist,
avec l'allowlist livrée VIDE.** C'est la forme exacte retenue au vol 1, et elle
tient pour la même raison.

- **(a.1) L'ensemble d'écrivains est écrit en dur, pas en allowlist.**
  `mika2922_ac2_un_seul_ecrivain_du_jugement_de_mpc` assert
  `audit_writers("MPC_FLIGHT_JUDGMENT_TOOL") == vec!["flight_verdict_ledger.rs"]`.
  Il n'y a **aucune violation existante** à excepter : la constante naît dans ce
  vol, donc l'ensemble est exact dès le premier jour et l'allowlist est vide par
  construction plutôt que par indulgence. Le message d'échec dit le remède —
  *router l'écriture par `record_mpc_judgment`, ne pas ajouter d'entrée* — sur le
  modèle du message du vol 1 et la doctrine mika#2201.

- **(a.2) L'assertion de lecteur n'est pas écrite ici.** Il n'y a aucun lecteur
  de `MPC_FLIGHT_JUDGMENT_TOOL` dans ce vol ; une garde de lecteur unique
  porterait sur un ensemble vide, ce qui est exactement la vacuité que (a.3)
  refuse. Elle naît avec le lecteur, au vol 3. C'est la décision que le vol 1 a
  prise pour le lecteur du manager, reconduite.

- **(a.3) Anti-vacuité obligatoire dans la garde.** `files.len() >= 100` avant
  toute assertion d'ensemble, parce que « aucun second écrivain » et « le scan
  n'a rien lu » rendent le même vert (classe mika#2205). Ce terme est ce qui
  rend l'énoncé « un seul écrivain » falsifiable.

- **(a.4) Contrôle négatif sur source fabriquée.**
  `mika2922_le_scan_du_jugement_mord` inline la fenêtre de cinq lignes d'un
  appel `log_audit_event` rustfmt-é et vérifie qu'elle **mord** sur le nouveau
  nom et **ne mord pas** sur un nom d'outil étranger. Sans lui, « le scan n'a
  trouvé qu'un écrivain » est indistinguable de « la fenêtre ne mord sur rien ».
  Même idiome que `mika2922_le_scan_decrivain_mord` du vol 1 et que
  `qa_reviewed_head::tests::mika2917_a2_le_scan_decrivain_mord`.

- **(a.5) Aucun littéral recopié hors du module de vérité.** La garde balaie la
  moitié production de tout le crate et refuse `"mpc_flight_judgment"` écrit en
  clair ailleurs que dans `flight_verdict_ledger.rs` : un site qui recopie le
  littéral au lieu de passer la constante est invisible à `audit_writers`, dont
  le `needle` est le **nom de la constante** par décision écrite de
  `source_scan.rs`.

Aucune des trois options (b) ni (c) n'est retenue : il n'y a rien à désarmer
(l'ensemble est exact au premier jour) et rien à faire remonter (le périmètre est
tranché par le vol 1 et par le ticket).

---

## Sonde post-déploiement (la part de ce vol dans AC6)

Après `make deploy`, sur une base où le vol 1 a déjà inscrit au moins un verdict
du manager :

1. Relever un `tasks.id` de vol jugé — `mika tasks list`, ou la ligne du
   manager : `SELECT target_key, reasoning FROM audit_events WHERE tool_name = 'manager_flight_verdict' ORDER BY id DESC LIMIT 5;`
2. Inscrire le jugement :
   `mika flight-verdicts judge --task-id <id> --issue dead --cause refusal --recommendation regroom --note "sonde mika#2922 vol 2"`
   ⇒ sortie « jugement inscrit », code 0.
3. Vérifier la ligne, **en lecture seule** :
   `SELECT target_key, after_value, reasoning FROM audit_events WHERE tool_name = 'mpc_flight_judgment' AND target_key = '<id>';`
   ⇒ exactement une ligne, `after_value` re-typable en `VerdictRecord`,
   `reasoning` portant la note.
4. Re-jouer la commande à l'identique ⇒ « déjà inscrit », le jugement en place
   est rendu, et l'étape 3 donne **toujours une seule** ligne.

**Une sortie absente n'est pas une absence de phénomène** : elle signifie un
binaire antérieur à ce vol (motif mika#2340, § *Deploying a bundled-skill
change* du `CLAUDE.md` racine — le même raisonnement vaut pour un binaire
`mika`). La vérification est l'étape 1 du chemin rebuild → seed → read, pas une
seconde exécution de l'étape 2.

La lecture en SQL des étapes 1 et 3 est **ce que le vol 3 remplace** : la
commande de concordance est précisément le chemin supporté de cette lecture.
Elle est acceptable ici parce que le ticket interdit le SQL à la main pour
l'**écriture** (« jamais par du SQL à la main », livrable 2), et que l'écriture
passe bien par la sous-commande.

---

## Alternatives rejetées

- **Écrire le jugement depuis `crates/mika-cli/`.** Refusé pour une raison
  structurelle et non esthétique : `source_scan::production_files()` est
  enracinée sur `crates/mika-agent/src/`, donc la garde d'AC2 serait vacuement
  verte. Le crate CLI reste adaptateur, doctrine *Wrapper-only INV-2*.

- **Un verbe de correction (`judge --force` / `amend`).** Non livré. « Pas de
  réécriture » est la loi que le ticket pose au registre, et une ligne de
  jugement réécrite rendrait le compte de concordance non reproductible — deux
  lectures du même `tasks.id` à deux instants donneraient deux réponses, ce qui
  est exactement ce qu'un instrument de seuil ne peut pas faire.
  **Critère de bascule, pour que le vol 3 n'ait pas à le re-dériver : la
  première correction réellement nécessaire, constatée.** Le remède d'ici là est
  qu'une saisie erronée se voit — la seconde saisie rend le jugement en place.

- **Une table dédiée plutôt que `audit_events`.** Refusé au vol 1 et toujours :
  `audit_events` porte la clé, la date, l'agent et deux colonnes de valeur, et
  le patron mika#2917 dit explicitement « aucune table ni migration, si
  `audit_events` suffit ».

- **Une seconde table de correspondance terme → variante, écrite à la main.**
  Refusé : c'est la classe mika#2158. `serde_json::from_value` sur les types
  existants donne l'inverse exact de la sérialisation, par construction, et
  suit toute variante ajoutée en amont sans qu'une ligne bouge ici.

- **Accepter une cause de mort libre et la porter en `DeathCause::Unknown`.**
  Refusé : elle ne pourrait jamais concorder, donc une faute de frappe
  deviendrait une divergence permanente que le compte du vol 3 rendrait comme un
  désaccord réel. Le refus nomme les cinq causes classées.

- **Livrer les livrables 2 et 3 en un seul vol, comme le vol 1 l'annonçait.**
  Refusé par le re-mesurage : ≈ 594 lignes contre une borne par vol de 429
  (§ *Le re-découpage* ci-dessus).

- **Livrer le lecteur de concordance avant l'écrivain de MPC.** Refusé : la
  colonne d'en face serait vide, et le lecteur rendrait « non jugé par MPC » sur
  tous les vols sans qu'aucune saisie soit possible — un instrument qui ne peut
  mesurer que zéro.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/flight_verdict_ledger.rs` — constantes, analyse des termes, `record_mpc_judgment`, `MpcJudgmentOutcome` | 110 |
| `crates/mika-cli/src/cli.rs` — `FlightVerdictsArgs` / `FlightVerdictsCommand::Judge` | 40 |
| `crates/mika-cli/src/commands/flight_verdicts.rs` — adaptateur mince | 50 |
| `crates/mika-cli/src/commands/mod.rs` + `src/main.rs` — câblage | 8 |
| tests (`flight_verdict_ledger.rs` `mod tests`) — AC2 seconde moitié + contrôle négatif, extension d'AC5, aller-retour des termes, idempotence sur vraie base | 145 |

Total estimé : 353
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient.** 353 < 429, avec 76 lignes de marge. La marge n'est pas
confortable par hasard : elle est ce qui reste après avoir sorti le livrable 3
(≈ 241 lignes mesurées : lecteur 50, rendu 35, promotion des accesseurs
`db.rs`/`async_db.rs` 12, promotion des deux étiquettes de `reporter.rs` 4,
sous-commande `concordance` 55, tests AC3/AC4 85), dont la somme avec ce vol
donnait 594. Le périmètre jugé ici est celui du **vol courant** seul.

**Tracked in: senara-solutions/mika#2922 (vol 3 — le compte de concordance :
livrable 3, AC3, AC4 et la seconde moitié d'AC6).**
