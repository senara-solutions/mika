# mika#2958 — le manager groome la relève d'avance (vol 2c : la lecture « un groom est en cours »)

**Ticket :** senara-solutions/mika#2958
**Type :** feat
**Branche :** `feat/2958/manager-phase-2-c-vol-2-le-manager`
**Vol :** 2c — la lecture container-scoped des dispatches de groom. Vols 1 (#2960),
2a (#2970) et 2b (#2975) sont mergés.
**Suite :** vol 2d (câblage de cycle + fenêtre durable), vol 2e (rendu de rapport).
Les deux sont nommés par une ligne `Tracked in:` dans la section *Hors périmètre*.

---

## Périmètre : ce que ce vol touche hors de la zone déclarée

La contrainte de périmètre écrite dans le corps du ticket le 2026-10-30 visait le
**vol 2b**. Elle est relevée, et les trois faits qui la relèvent sont vérifiables :

- **mika#2972 est mergée** (2026-10-30T08:21:55Z). `milestone_manager/reader.rs`,
  `milestone_manager/types.rs` et le bloc `pub use types::{…}` ne sont plus en vol :
  le champ `milestone_read_audit` est dans `main`. Ce plan ne touche aucun des trois
  de toute façon, mais l'interdit de littéral `MilestoneState { … }` n'a plus de
  motif de rebase.
- **Aucune PR concurrente n'est ouverte sur la zone du manager** — `gh pr list` ne
  rend que des PR dependabot. mika#2961 reste OPEN sans PR ouverte, donc
  `evidence/guards.rs` et `skills/builtin_handlers.rs` sont libres ; ce plan ne les
  touche pas davantage.
- **Le bloc de réexportation visé est celui de `groom_trigger`**, pas celui de
  `types` ni celui de `reader`. La règle « une réexportation neuve va dans le bloc
  de son propre module » est respectée telle quelle.

`MIKA_MANAGER_GROOM_WRITE` reste **OFF par défaut** et ce vol ne l'ouvre pas : seul
Vincent l'ouvre. Ce vol ne déclenche d'ailleurs aucun groom — il ne touche pas le
chemin d'envoi.

**Un fichier neuf hors du module, et c'est délibéré.** `crates/mika-agent/src/groom_in_flight.rs`
naît **à côté** de `milestone_manager/`, pas dedans. Le motif est écrit dans le
module-doc de `crates/mika-agent/src/flight_source.rs` (l. 1-40) et ce plan le cite
plutôt que de le re-dériver : `no_dispatch_test.rs` grep tous les `.rs` de l'arbre
du module à la recherche de 23 jetons d'autorité d'écriture, et un fichier qui porte
du SQL **et** un handle de base ne passe pas ce grep sans une exception nommée. Le
trait se déclare dans le module, l'implémentation vit dehors : c'est le même
découpage que `FlightSource` / `DbFlightSource`, mergé sur ce motif.

---

## Contexte

### Ce que les vols 1, 2a et 2b savent déjà faire

- **Vol 1 (#2960)** — le manager **nomme** la relève dans son rapport, section
  « Relève à grommer (mode ombre — aucun groom déclenché) ». `relief.rs` :
  `RELIEF_SIZE = 4`, `ReliefVerdict { next, beyond }`, et depuis mika#2923 le filtre
  sur `RefusalReason::remediable_by_grooming` avant le calcul de `beyond`.
- **Vol 2a (#2970)** — le **rail** d'envoi. `milestone_manager/groom_trigger.rs`
  porte `GroomSender`, `A2aGroomSender`, `GroomSendError`, `resolve_groom_rail`,
  `trigger_relief_grooms(armed, sender, candidats)`, `GROOMS_PER_CYCLE = 1`,
  `GROOM_SEND_BUDGET`. Le rail est entré comme **exception nommée** au scan, pour le
  seul jeton `message/send` — troisième et dernière entrée de `EXEMPTED_OCCURRENCES`.
- **Vol 2b (#2975)** — le **câblage** : `resolve_groom_rail` au site de production
  (`server/mod.rs:1742`), le paramètre de `spawn_manager_cycle_task`, le champ
  `CycleContext::groom_sender`, et `trigger_cycle_grooms(ctx, assessment)` qui
  construit les `GroomCandidate` depuis `assessment.relief`.

### La fenêtre que ce vol ferme

Le module-doc de `groom_trigger.rs` nomme lui-même ce qui manque :

> Ce qui manque est la lecture « un groom est **en cours** » : [`GroomCandidate`] la
> reçoit à `false` par construction […] Cette fenêtre est le vol 2c, et le rendu de
> rapport (AC4, moitié ON) le vol 2d.

Et le site l'écrit en clair dans `cadence.rs::trigger_cycle_grooms` :

```rust
// Tracked in: vol 2c — fenêtre d'idempotence durable + lecture
// container-scoped des tâches moteur `Groom …` / `ready-label`.
groom_en_cours: false,
```

Un `false` constant veut dire : aujourd'hui, interrupteur ouvert, le manager
re-déclencherait un groom sur un ticket qu'un groom moteur **ou manuel** est déjà en
train de grommer. C'est exactement la collision que le motif d'AC2 nomme.

### Pourquoi la lecture de base est la moitié *essentielle* d'AC2, et pas le checkpoint

Il y a deux idempotences à construire, et elles ne couvrent pas la même population :

| lecture | voit un groom **moteur** | voit un groom **manuel** | survit au redémarrage |
|---|---|---|---|
| checkpoint du manager (`MilestoneCheckpoint`) | seulement ceux **qu'il a envoyés** | **non** | oui |
| tâches moteur, lues en base | oui | **oui** | oui |

Le motif écrit dans AC2 est *« la collision d'un groom manuel avec un groom
moteur »*. Un groom manuel — `mika ask --agent mika-dev "groom mika#N"`, ou un
webhook `ready` — n'écrit **rien** dans le checkpoint du manager. Aucune fenêtre que
le manager tient lui-même ne peut le voir. Seule la lecture des tâches le voit.
C'est pourquoi ce vol livre la lecture, et pourquoi la fenêtre durable (utile, mais
non suffisante) part au vol 2d avec son consommateur.

### Pourquoi `live_pilot_for_issue` ne peut pas servir

`crates/mika-agent/src/live_pilot.rs` répond déjà à une question de forme voisine —
*un pilote tourne-t-il sous cette issue ?* — et c'est le lecteur **unique** de la
vivacité de pilote (mika#2653, mika#2201). Il est néanmoins inutilisable ici, pour
une raison structurelle et non de goût : sa traversée est **agent-scopée**.
`find_dispatch_children_for_issue_url` porte `WHERE parent.agent_id = ?1 AND
parent.reference_url IN (?2, ?3)`. Le manager est une tâche de processus **sans
agent** : tout filtre `agent_id` lui rend une lecture constamment vide. C'est le
défaut mika#2792, mot pour mot, et il a déjà coûté un correctif sur
`list_finished_implement_flights_in_container`.

La conséquence est une requête **neuve**, dont le nom porte l'invariant —
`…_in_container` — exactement comme son précédent. Ce n'est pas un second lecteur de
la vivacité de pilote : la question posée ici est *« une tâche de groom non terminale
existe-t-elle sur ce ticket ? »*, qui ne consulte **aucune** preuve d'instance de
processus (voir D6 pour la vérification du scan mika#2653).

---

## Requirements

### R1 — La requête, container-scopée, dans `db/tasks.rs`

Nouvelle méthode sur `impl Database`, posée à côté de son précédent
(`list_finished_implement_flights_in_container`, l. 5208), dont elle reprend le
gabarit ligne pour ligne :

```rust
/// Les dispatches de **groom** vivants ou morts portant une de ces URLs
/// d'issue, sur tout le conteneur (mika#2958, vol 2c).
pub fn list_groom_dispatches_in_container(
    &self,
    issue_urls: &[String],
) -> Result<Vec<GroomDispatchRow>>
```

```rust
/// Une ligne de dispatch de groom : l'URL qui l'a appariée, et le statut de la
/// tâche qui la porte. Le **statut** sort brut — le filtre terminal est en Rust.
pub struct GroomDispatchRow {
    pub issue_url: String,
    pub status: String,
}
```

Six propriétés, chacune reprise d'un précédent nommé :

1. **Aucun `agent_id`** dans la requête (mika#2792). L'invariant est porté par le
   **nom** de la méthode, pas par un commentaire : `…_in_container`. Le handle ne
   sert que de connexion — le conteneur n'a qu'une base.
2. **Appariement par ensemble fermé de variantes** (mika#2638) :
   `task_state::tasks::issue_url_variants(base)` rend `[base, base + "?phase=groom"]`,
   et la clause est un `reference_url IN (?, ?, …)`. **Jamais** un `LIKE 'url%'` : le
   préfixe faisait apparier `/issues/216` avec `/issues/2161`, et c'est le bug que
   mika#2638 a corrigé.
3. **La classe est lue par `COALESCE(dispatch_class, 'implement') = 'groom'`**
   (schéma v34) : les lignes antérieures à v34 portent `NULL`, qui vaut `'implement'`,
   donc jamais `'groom'`.
4. **Deux formes de ligne, en `UNION ALL`** — et la seconde n'est pas du zèle :
   - (a) la ligne de classe `groom` qui porte elle-même `reference_url` : la parente
     de suivi du dispatch de groom (`trigger_type='manual'`, `action_type='none'`,
     `dispatch_class='groom'`) ;
   - (b) une ligne de classe `groom` **enfant** jointe à une parente qui porte
     l'URL : la forme mika#2279 — parente annulée, enfant encore vivant. Sans la
     branche (b), un enfant en vol sous une parente morte lit comme « aucun groom »,
     et c'est précisément la collision que le ticket veut éviter.

   La parente porte l'URL et jamais le `process_id` ; l'enfant porte le pgid et
   jamais l'URL. Dans la branche (b), `EXISTS` est **corrélé** et la colonne `id` de
   la sous-requête est **qualifiée** (`parent.id`) : un `id` nu se résoudrait sur
   `enfant.id` et la sous-requête deviendrait une tautologie — l'avertissement est
   déjà écrit en commentaire au précédent, et il est recopié ici.
5. **Le filtre de statut terminal est en Rust**, jamais en SQL :
   `crate::task_state::tasks::is_terminal_task_status`. Le SQL rend le statut brut ;
   l'appelant (R2) décide. Même doctrine que le précédent.
6. **`issue_urls` vide ⇒ `Ok(vec![])` sans requête.** Un `IN ()` est un `SELECT` qui
   ne rend rien mais qui a coûté un aller-retour, et un `IN` vide est une syntaxe
   SQLite refusée selon la génération du `format!`.

**Pas de second plafond nommé.** Le bornage de la liste vient de son unique
appelant : la relève vaut au plus `RELIEF_SIZE = 4` tickets. Une constante
`GROOM_READ_LIMIT` ici serait un deuxième lecteur du même bornage, donc un endroit
de plus où le chiffre peut divergir (doctrine mika#2201).

**Tests (dans `db/tests/`)**, sur les fixtures qui existent déjà
(`groom_parent(agent_id, reference_url)` et `groom_callback(agent_id, parent_id,
dispatch_class)`, `db/tests/mod.rs` l. 1086-1140) :

- une parente de groom non terminale sur l'URL ⇒ une ligne, statut brut rendu ;
- la même avec `?phase=groom` ⇒ appariée (variante fermée) ;
- **contrôle négatif de préfixe** : une parente sur `/issues/2161` n'apparie **pas**
  une demande sur `/issues/216` ;
- une parente **annulée** portant un enfant de groom vivant ⇒ une ligne par la
  branche (b) ;
- une parente de classe `implement` sur la même URL ⇒ **aucune** ligne ;
- une parente de groom portée par **un autre `agent_id`** ⇒ **une** ligne (c'est
  l'assertion qui tient le container-scope : elle rougit le jour où quelqu'un
  réintroduit le filtre `agent_id`) ;
- `issue_urls` vide ⇒ `Ok(vec![])`.

### R2 — La source, hors du module

Nouveau fichier `crates/mika-agent/src/groom_in_flight.rs`, sur le patron et pour le
motif de `flight_source.rs` (cité, pas re-dérivé) :

```rust
pub struct DbGroomInFlightSource { db: AsyncDatabase }

impl DbGroomInFlightSource {
    pub fn new(db: AsyncDatabase) -> Self { … }
}

impl GroomInFlightSource for DbGroomInFlightSource {
    async fn grooms_in_flight(&self, issues: &[GroomIssue]) -> GroomInFlightReading { … }
}
```

Trois propriétés :

1. **Le numéro ne refait jamais l'aller-retour par une URL.** L'entrée est une liste
   de `GroomIssue { number: u64, url: String }` ; la source garde sa **propre** table
   `url → number` (y compris les deux variantes) et rend des numéros. Un `parse` de
   numéro depuis une URL rendue par la base serait un second analyseur de la même
   chaîne, et il échouerait silencieusement sur la variante `?phase=groom`.
2. **Un échec de base rend `Unreadable` plus un WARN nommé, jamais une erreur.** Le
   nom d'événement est propre à cette lecture — `groom_in_flight_read_failed`, avec
   le nombre d'URLs demandées — parce que deux populations sous un même nom ne sont
   pas soustractibles (mika#2205) et que le grep qui répond *« la lecture atteint-elle
   quelqu'un ? »* doit nommer la lecture. Un cycle ne meurt pas d'une base muette.
3. **Le filtre terminal est appliqué ici**, sur le statut brut rendu par R1, via
   `is_terminal_task_status`. Un seul site, et c'est le site qui possède la question.

**Tests** : en base en mémoire, les mêmes formes qu'en R1 mais à travers la source —
la lecture rend bien un `BTreeSet` de numéros ; un statut terminal est **exclu** ; la
variante `?phase=groom` est appariée sur le bon numéro ; liste vide ⇒ `Read(∅)` et
non `Unreadable` (la distinction du point 2 est inutile si elle n'est pas testée).

### R3 — Le trait et la lecture typée, dans le module

Dans `milestone_manager/groom_trigger.rs` :

```rust
/// Quels tickets de la relève portent **déjà** un groom en vol.
///
/// Déclaré ici, implémenté dehors (`crate::groom_in_flight`) : voir le
/// module-doc de `crate::flight_source` pour le motif — `no_dispatch_test.rs`
/// grep l'arbre de ce module, et du SQL avec un handle de base n'y passe pas.
pub trait GroomInFlightSource: Send + Sync {
    async fn grooms_in_flight(&self, issues: &[GroomIssue]) -> GroomInFlightReading;
}

/// La lecture, **typée** : « rien en vol » et « je n'ai pas pu lire » ne sont pas
/// la même phrase (mika#2205, mika#2331).
pub enum GroomInFlightReading {
    /// Les numéros portant un groom non terminal. Un numéro absent n'en porte pas.
    Read(BTreeSet<u64>),
    /// La lecture a échoué, ou aucune source n'est câblée. **Rien n'est su.**
    Unreadable,
}
```

`GroomInFlightReading` est le miroir exact de
`evaluator::FlightPrReads::{Read, Unwired}` — même motif, même raison : un
`Option<BTreeSet>` ou un `BTreeSet` vide confondrait « aucun groom en vol » avec
« je n'ai pas regardé », et ces deux lectures appellent des gestes **opposés**.

**La direction du fail-safe est l'inverse de celle de `live_pilot`, et c'est
argumenté.** `live_pilot_for_issue` penche vers *« pas de pilote »* en cas de doute,
parce que son consommateur refuse un dispatch et qu'un refus à tort bloque le vol.
Ici le consommateur **envoie** : `Unreadable` doit donc valoir *« un groom est
peut-être en vol »* et faire **sauter** le candidat. Le coût d'un saut est un cycle
de retard — borné, et le cycle suivant réessaie. Le coût d'un doublon est la
collision qu'AC2 existe pour empêcher : deux pilotes sur le même ticket. Les deux
asymétries sont justes ; c'est la question qui change de sens, pas la doctrine.

Ce vol **déclare** cette direction dans le doc du type et la **teste** sur
`trigger_relief_grooms` (le motif `groom_in_flight` existe déjà dans ses sauts) ;
c'est le vol 2d qui branche la source réelle.

### R4 — La réexportation

`milestone_manager/mod.rs` : ajout de `GroomInFlightReading`, `GroomInFlightSource`
et `GroomIssue` au bloc `pub use groom_trigger::{…}` qui existe déjà — **son propre
bloc**, ni celui de `types`, ni celui de `reader`.

---

## Décisions

**D1 — Une requête neuve, pas une généralisation de `live_pilot`.** Rendre
`find_dispatch_children_for_issue_url` container-scopable ferait de ce lecteur
unique un lecteur à deux régimes, dont le mauvais est silencieusement vide pour le
manager. La question posée est d'ailleurs différente : *une tâche de groom non
terminale* contre *un processus vivant*. Deux questions, deux lecteurs — et le nom
de chacun dit laquelle il répond.

**D2 — `UNION ALL`, pas `UNION`.** Les doublons sont attendus et sans conséquence :
la source dédoublonne en `BTreeSet`. Un `UNION` paierait un tri pour une propriété
que le type garantit déjà.

**D3 — Le statut sort brut de SQL.** Le vocabulaire terminal vit dans
`task_state::tasks::is_terminal_task_status` et nulle part ailleurs. Un
`status NOT IN ('completed', …)` en SQL serait une seconde définition de « terminal »,
dans un langage où personne ne pense à la mettre à jour.

**D4 — Pas de consommateur dans ce vol.** C'est exactement la forme du vol 2a — un
rail que personne n'instancie — mergé sur ce motif. Elle est choisie ici par
arithmétique, pas par préférence : lecture + câblage + fenêtre durable estime ≈ 667
lignes, lecture + câblage ≈ 490 ; les deux passent la borne de 429. Le découpage est
dans *Hors périmètre*. Un lecteur qui compile, se teste et ne change le comportement
de rien est mergeable seul et sans risque : `groom_en_cours: false` reste `false`, et
l'interrupteur reste OFF.

**D5 — Le bornage vient de l'appelant, pas d'une constante nouvelle.** `RELIEF_SIZE = 4`
borne déjà la liste au seul site qui appellera. Voir R1.

**D6 — Aucune garde structurelle n'est franchie, et c'est vérifié plutôt que supposé.**

- **Le scan mika#2653** (`canonical_tokens::mika2653_la_vivacite_dun_pilote_a_un_lecteur_unique`)
  est une conjonction à **trois** termes, lue à `canonical_tokens.rs:2733-2744` :
  une traversée (`find_dispatch_children_with_pid` **ou**
  `find_dispatch_children_for_issue_url`) **et** `is_terminal_task_status` **et**
  `is_same_process_alive`. `groom_in_flight.rs` porte le second terme et **aucun**
  des deux autres : il n'appelle ni l'une des deux traversées nommées, ni la preuve
  d'instance. Au plus un terme sur trois. `LIVE_PILOT_TASK_READER_ALLOWED` reste
  vide, et ce plan n'y ajoute rien.
- **`no_dispatch_test.rs`** ne voit aucun jeton neuf : le fichier qui porte le SQL
  est **hors** de l'arbre du module, et ce qui entre dans le module (un trait, un
  enum, une réexportation) ne contient ni `message/send`, ni `POST`, ni `PATCH`, ni
  `dispatcher_source = 'mika_manager'`. `EXEMPTED_OCCURRENCES` reste à ses **trois**
  entrées, non amendé. C'est le but du découpage de R2/R3, pas un effet de bord.
- **`no_repo_list_test.rs`** : aucun littéral de dépôt n'entre dans le module. Les
  URLs se composent au vol 2d depuis `cfg.target.repo`, sur le patron de
  `evaluator::milestone_issue_urls` (`evaluator.rs:566`), qui fait déjà exactement
  cela et passe la garde.

**D7 — Pourquoi ce plan ne porte pas de section `## Fire-Disposition`.** La porte a
été considérée et elle est **N/A** (mika#2306 : *« si le plan ne livre aucun
détecteur, la section n'est pas requise : ne l'invente pas »*). Les livrables sont
une requête de lecture, une source, un trait, un type somme et leurs tests de
comportement — aucun livrable dont la fonction primaire est de signaler une
violation : pas de lint, pas de scan structurel, pas de garde CI, pas de validateur,
pas de garde EndTurn, aucune allowlist neuve. Le seul scan touché de près est
mika#2653, qui n'est **pas** modifié (D6). L'assertion de container-scope de R1 est
un test de comportement sur une requête, pas un détecteur de violations dans le
corpus.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/db/tasks.rs` (requête + `GroomDispatchRow`) | 80 |
| tests DB (`crates/mika-agent/src/db/tests/`) | 85 |
| `crates/mika-agent/src/async_db.rs` (enveloppe async) | 16 |
| `crates/mika-agent/src/groom_in_flight.rs` (source + table `url → number`) | 85 |
| tests de `groom_in_flight.rs` | 75 |
| `crates/mika-agent/src/milestone_manager/groom_trigger.rs` (trait + `GroomInFlightReading`) | 40 |
| `crates/mika-agent/src/milestone_manager/mod.rs` (réexportation) | 5 |

Total estimé : 386 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 386 < 429, avec 43 lignes de marge. L'étalonnage
vient des deux vols mergés du même ticket, dans le même style dense en
documentation : vol 2a ≈ 420 lignes réelles, vol 2b ≈ 652. La marge est donc
mince, et c'est ce qui a décidé D4 : le câblage de cycle part au vol 2d plutôt que
de passer la borne.

---

## Ordre de commit

1. **R1** — la requête et `GroomDispatchRow` dans `db/tasks.rs`, avec ses tests DB.
   Plancher : la requête compile, les sept assertions passent, aucun autre fichier
   n'est touché. Mergeable seule.
2. **R3** — le trait, `GroomIssue` et `GroomInFlightReading` dans `groom_trigger.rs`,
   plus la réexportation (R4). Plancher : `cargo test -p mika-agent milestone_manager`
   vert, `no_dispatch_test.rs` vert sans amendement de son allowlist.
3. **R2** — l'enveloppe `async_db.rs` et `groom_in_flight.rs` avec ses tests.
   Plancher : la source implémente le trait de l'étape 2 sur la requête de l'étape 1.

Chaque étape laisse l'arbre vert. Aucune ne change un comportement observable :
`groom_en_cours` reste `false` au site d'appel, et l'interrupteur reste OFF.

---

## Definition of Done

- `list_groom_dispatches_in_container` existe dans `db/tasks.rs`, sans `agent_id`
  dans sa requête, avec `IN (…)` sur les variantes fermées et le filtre terminal
  **hors** SQL ; ses sept tests passent, dont le contrôle négatif de préfixe
  (`/issues/2161` n'apparie pas `/issues/216`) et l'assertion de container-scope
  (une parente portée par un autre agent est **lue**).
- `GroomDispatchRow` rend le statut **brut**.
- `crates/mika-agent/src/groom_in_flight.rs` existe **hors** de `milestone_manager/`,
  cite le module-doc de `flight_source.rs` pour le motif, rend `Unreadable` plus le
  WARN `groom_in_flight_read_failed` sur échec de base, et ne fait jamais remonter
  d'erreur.
- `GroomInFlightSource` et `GroomInFlightReading { Read(BTreeSet<u64>), Unreadable }`
  sont déclarés dans `groom_trigger.rs` et réexportés depuis le bloc
  `pub use groom_trigger::{…}` de `mod.rs`.
- Le doc de `GroomInFlightReading` écrit la direction du fail-safe (`Unreadable` ⇒
  sauter) **et** pourquoi elle est l'inverse de celle de `live_pilot`.
- `EXEMPTED_OCCURRENCES` de `no_dispatch_test.rs` est **inchangé**, à trois entrées ;
  `LIVE_PILOT_TASK_READER_ALLOWED` est **inchangé**, vide.
- `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` passent.
- La PR nomme les deux vols de suite par leurs lignes `Tracked in:`.
- Aucun comportement observable ne change : `MIKA_MANAGER_GROOM_WRITE` reste OFF par
  défaut et aucun groom n'est déclenché par ce vol.

---

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2958 :

- **AC1.** Avec l'interrupteur OFF, aucun groom n'est déclenché. C'est une garantie
  structurelle, avec un contrôle négatif dans le même test. Le geste entre comme
  **exception nommée** au scan de `no_dispatch_test.rs`, sur le patron des entrées de
  mika#2698 vol 2 et de mika#2699 vol 4. Le scan n'est pas assoupli. Le test
  `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` est amendé dans le même commit, et
  la PR dit ce qui le faisait rougir.
- **AC2.** Sélection et idempotence : un ticket déjà groomé, ou dont un groom est en
  cours, n'est pas relancé. Le plan dit par quelle lecture le manager voit un groom en
  cours : les tâches moteur `Groom …` et `ready-label`, ou autre chose. Le motif est la
  collision d'un groom manuel avec un groom moteur.
- **AC3.** La voie est celle du moteur : la même que
  `mika ask --agent mika-dev "groom mika#N"`, ou son équivalent interne. Elle n'ouvre ni
  spawn ni dispatch d'implémentation. Le plan dit comment le geste reste borné : combien
  de grooms par cycle, et ce qu'il fait quand mika-dev a déjà un pilote en vol, puisque
  le moteur n'en fait tourner qu'un à la fois.
- **AC4.** Ce qui reste de l'ombre se lit dans le rapport. Interrupteur OFF : la section
  « Relève à grommer » est inchangée. Interrupteur ON, en test seulement : le rapport dit
  quels grooms ont été déclenchés, et lesquels ont été sautés, avec leur motif.
- **Taille** : à estimer tests compris, et à découper en vols au-delà de la borne.

### Couverture par ce vol

| AC | état | par quoi |
|---|---|---|
| AC1 | **déjà fermé** (vol 2a, PR #2970) | exception nommée pour `message/send`, contrôle négatif dans `cycle_refusal`. Ce vol n'y touche pas et n'amende pas l'allowlist (D6). |
| AC2 | **la moitié essentielle, ici** | R1 + R2 + R3 livrent la lecture qui voit un groom moteur **et** un groom manuel. La fenêtre durable complémentaire et le branchement partent au vol 2d. |
| AC3 | **déjà fermé** (vols 2a + 2b) | `GROOMS_PER_CYCLE = 1`, `GROOM_SEND_BUDGET`, et `GroomSendError::AgentBusy` ⇒ saut `agent_busy` quand mika-dev a un pilote en vol. |
| AC4 | moitié OFF **déjà fermée** (vol 1) | la section « Relève à grommer » est inchangée par ce vol. Moitié ON ⇒ vol 2e. |
| Taille | **fermé par ce plan** | section `## Taille estimée`, 386 < 429, découpage en vols nommés. |

---

## Hors périmètre

- **Le câblage de cycle et la fenêtre durable d'idempotence.** Champ
  `CycleContext::groom_in_flight` + son constructeur, paramètre de
  `spawn_manager_cycle_task`, construction au site de production dans
  `server/mod.rs`, et `trigger_cycle_grooms` consommant la source pour que
  `groom_en_cours` cesse d'être `false` ; plus le champ neuf de
  `MilestoneCheckpoint` sur le patron `posted_flight_task_ids` (mika#2698 vol 2), sa
  fonction de fenêtre pure, et la condition de sauvegarde élargie par « un groom a
  été envoyé ». Estimé ≈ 280 lignes.
  **Tracked in: senara-solutions/mika#2958**
- **Le rendu de rapport, moitié ON d'AC4** : quels grooms ont été déclenchés, lesquels
  ont été sautés et avec quel motif. `trigger_cycle_grooms` rend déjà un
  `GroomTriggerOutcome` **lié** et non jeté au site d'appel, donc le rendu est une
  ligne de plus plus sa mise en forme. Estimé ≈ 120 lignes.
  **Tracked in: senara-solutions/mika#2958**
- **L'ouverture de `MIKA_MANAGER_GROOM_WRITE`.** Geste de l'opérateur, au seuil écrit
  dans mika#2700 ; `MIKA_MANAGER_EVALUATE_WRITE` et `MIKA_MANAGER_POOL_WRITE` sont
  OFF aujourd'hui.
- **Toute évolution de `live_pilot.rs`.** Lecteur unique de la vivacité de pilote
  (mika#2653) : il n'est ni lu, ni généralisé, ni dupliqué par ce vol (D1, D6).

---

## Sonde post-déploiement

Ce vol ne change aucun comportement observable, donc il n'a pas de sonde propre : il
n'y a rien à mesurer avant que le vol 2d ne branche la source. Ce qui se vérifie
après merge est **statique** et se lit sans déployer :

- `grep -c 'EXEMPTED_OCCURRENCES' crates/mika-agent/src/milestone_manager/no_dispatch_test.rs`
  et son contenu : **trois** entrées, inchangées ;
- `LIVE_PILOT_TASK_READER_ALLOWED` : **vide** ;
- `grep -n 'agent_id' crates/mika-agent/src/db/tasks.rs` au voisinage de la nouvelle
  requête : **aucune** occurrence dans son corps.

La sonde de comportement — `groom_in_flight_read_failed` (WARN) et le motif de saut
`groom_in_flight` dans le rapport — naît avec son consommateur au vol 2d ; l'absence
de ces lignes aujourd'hui signifie qu'aucun consommateur n'est câblé, et non qu'une
lecture a échoué (motif mika#2340).
