# mika#2958 — le câblage de cycle et la fenêtre durable d'idempotence (vol 2d)

> **Périmètre touché hors de la zone déclarée par le corps du ticket — et pourquoi.**
> La « zone attendue » écrite dans le corps (`groom_trigger.rs`, `cadence.rs`,
> `spawn.rs`) datait du **vol 2b**. Ce vol en sort par **un** fichier :
> `crates/mika-agent/src/server/mod.rs`, pour y construire la source de lecture
> livrée par le vol 2c (`DbGroomInFlightSource`). C'est inévitable et c'est déjà
> ce que le plan du vol 2c a prescrit : la source prend une `AsyncDatabase`, et le
> seul endroit du dépôt qui tient les handles de base des agents est le site de
> production. La forme est **copiée trait pour trait** sur le voisin
> `DbFlightSource` déjà construit au même appel (`server/mod.rs:1765`), donc
> l'empreinte est de ~10 lignes dans une liste d'arguments existante.
>
> Les trois interdits de périmètre du corps sont **levés** : mika#2972 est mergée
> le 2026-10-30T08:21:55Z, donc `reader.rs`, `types.rs` et le bloc
> `pub use types::{…}` ne sont plus en vol. Ce plan ne les touche pas davantage
> pour autant, et il n'ajoute **aucun** littéral `MilestoneState { … }`. La zone de
> mika#2961 vol 2/2 (`evidence/guards.rs`, `skills/builtin_handlers.rs`) n'est pas
> approchée.
>
> `groom_trigger.rs` n'est **pas modifié** par ce vol : tout ce qu'il consomme y
> est déjà (voir D1).

## Contexte

Le vol 1 (PR #2740) nomme la relève en mode ombre. Les vols 2a → 2c ont construit
le rail et sa lecture :

- **vol 2a** (#2970, `9d6c1225`) — `GroomSender` / `A2aGroomSender`, le geste
  `message/send` déclaré à plat, les trois refus de fil, `GROOMS_PER_CYCLE = 1`.
- **vol 2b** (#2975) — `resolve_groom_rail` au site qui tient le jeton interne,
  l'injection `CycleContext::groom_sender`, `trigger_cycle_grooms` branché au
  corps de cycle.
- **vol 2c** (#2988, `43a7499fd`) — `GroomInFlightSource`,
  `GroomInFlightReading::{Read, Unreadable}`, `GroomIssue`, et
  `crates/mika-agent/src/groom_in_flight.rs` : la lecture container-scopée
  (mika#2792) des tâches moteur de classe `groom` par `reference_url IN (?, ?)`
  (mika#2638), avec `Unreadable` + WARN `groom_in_flight_read_failed` sur échec.

**Ce qui reste donc dû, et c'est ce vol.** Le chemin de production existe de bout
en bout, mais rien ne branche la lecture du vol 2c sur le candidat du vol 2a :
`trigger_cycle_grooms` pose encore `groom_en_cours: false` en dur
(`cadence.rs:972`), sous un commentaire `Tracked in: vol 2c`. Et la lecture en
base, à elle seule, ne ferme pas AC2 : la tâche moteur n'apparaît qu'**après** que
le moteur ait accepté le `message/send`, donc entre l'envoi et la pose du siège la
lecture rend `Read(∅)` et le manager renverrait. Le palliatif actuel est
`GROOMS_PER_CYCLE = 1`, qui borne le dégât « à un doublon au pire » — ce vol
remplace la borne par une fenêtre d'idempotence **durable**, portée par le
checkpoint, sur le patron `posted_flight_task_ids` de mika#2698 vol 2.

Le vol **2e** — la moitié ON d'AC4, le rendu de rapport — reste hors périmètre et
nommé par un `Tracked in:` (voir *Hors périmètre*).

`MIKA_MANAGER_GROOM_WRITE` reste OFF par défaut. Ce vol ne l'ouvre pas.

## Requirements

- **R1.** `CycleContext` porte la source de lecture des grooms en cours, injectée
  par le même idiome que ses six voisines, et construite au site de production.
- **R2.** `trigger_cycle_grooms` consomme cette source : `groom_en_cours` cesse
  d'être une constante `false` et devient la **disjonction** de la lecture en base
  et de la fenêtre durable.
- **R3.** Le checkpoint porte la fenêtre durable des grooms déclenchés par le
  manager, avec `#[serde(default)]` pour relire sans erreur un checkpoint antérieur.
- **R4.** La fenêtre a une **borne qui expire** : aucun ticket de relève ne peut
  être retenu indéfiniment par elle (pas de famine).
- **R5.** La condition de sauvegarde du checkpoint est élargie par « un groom a été
  envoyé », sans avaler l'événement `state_changed` en attente.
- **R6.** Avec l'interrupteur OFF : zéro lecture en base, zéro envoi, et le motif
  dit `disarmed` — pas `no_in_flight_source`, pas `groom_in_flight`.
- **R7.** Aucun littéral de dépôt n'entre dans `milestone_manager/`
  (`no_repo_list_test.rs`), et aucune exception n'est ajoutée ni amendée dans
  `no_dispatch_test.rs`.

## Décisions

### D1 — `groom_trigger.rs` n'est pas touché, parce qu'il est complet

Le vol 2c y a livré tout ce que ce vol consomme : `GroomIssue { number, url }`,
le trait `GroomInFlightSource`, `GroomInFlightReading` et son unique lecteur
`carries_groom`, `GroomCandidate { number, deja_groome,
groom_en_cours }`, `GroomTriggerOutcome { declenches, sautes }`,
`trigger_relief_grooms`, `cycle_refusal`, `envoyer`. Le seul champ que ce vol fait
changer de valeur est `groom_en_cours`, et c'est son **producteur**
(`trigger_cycle_grooms`, dans `cadence.rs`) qui change, pas son consommateur. Le
bloc `pub use groom_trigger::{…}` de `mod.rs:318` réexporte déjà les trois types
nécessaires : aucune réexportation neuve.

### D2 — l'URL se compose depuis `cfg.target.repo`, et un ticket de relève ne peut pas venir d'ailleurs

`GroomIssue` exige une `url`. `ReliefTicket` n'a **pas** de champ `repo` —
`grep -rn "repo" milestone_manager/relief.rs` ne rend rien. La question posée au
grooming était : `ReliefTicket.origin: CandidateOrigin` distingue `Milestone`
d'`Outside`, et un candidat « hors milestone » pourrait-il appartenir à un autre
dépôt ? **Non, et c'est écrit dans le type** (`pool.rs:484-489`) :

```rust
pub enum CandidateOrigin {
    /// Une `sub_issue` du milestone cible.
    Milestone,
    /// Une issue ouverte du dépôt qui n'appartient pas au milestone cible.
    Outside,
}
```

« du dépôt », singulier : les deux provenances sont dans `cfg.target.repo`, et le
geste d'écriture voisin le confirme — `admit_pool_tickets(armed, runner, repo,
milestone_title, admissions)` reçoit ce même `repo` pour les candidats des **deux**
provenances. La composition est donc légitime et unique :

```rust
let url = format!("https://github.com/{}/issues/{}", cfg.target.repo, t.number);
```

C'est le patron exact d'`evaluator::milestone_issue_urls` (`evaluator.rs:566`), qui
compose depuis un champ de donnée et **passe** `no_repo_list_test.rs` (R7). Aucune
constante d'hôte n'est introduite : le littéral est le préfixe de forge, déjà
présent à ce site voisin.

`DbGroomInFlightSource` normalise lui-même par `task_state::tasks::issue_url_variants`
(idempotent sur `?phase=groom`), donc l'URL canonique nue est la bonne entrée.

### D3 — `cfg` et le checkpoint entrent dans la signature, sur le patron de ses deux voisins

`trigger_cycle_grooms(ctx, assessment)` n'a accès ni au dépôt ni à la fenêtre. Les
deux autres gestes d'écriture du cycle ont déjà la forme qu'il faut :

| geste | signature actuelle |
|---|---|
| `write_pool_admissions` | `(cfg, ctx, state, assessment)` |
| `post_flight_verdicts` | `(cfg, ctx, verdicts, checkpoint.as_ref())` |

Nouvelle signature, copiée sur la seconde (elle est celle qui lit un checkpoint) :

```rust
async fn trigger_cycle_grooms(
    cfg: &ManagerConfig,
    ctx: &CycleContext<'_>,
    assessment: &Assessment,
    checkpoint: Option<&MilestoneCheckpoint>,
) -> GroomTriggerOutcome
```

`state` n'entre **pas** : la relève est sur l'`assessment`, et le dépôt sur `cfg`.
Une quatrième entrée inutile serait une dépendance que rien ne lit.

### D4 — l'ordre des refus : l'interrupteur d'abord, et la lecture n'est payée que s'il est ouvert

`groom_write_is_armed()` est lu **une fois** à ce site et passé en paramètre à
`trigger_relief_grooms` (qui le reçoit déjà : il ne relit rien). L'ordre est :

1. `assessment.relief == None` → refus propre au site, `reason = "no_relief_verdict"`
   (existant, inchangé).
2. `!armed` → **aucune lecture en base**, et l'on tombe directement dans
   `trigger_relief_grooms(false, …)` qui dit `disarmed` à son site unique
   `cycle_refusal` (mika#2201). C'est R6 : avec l'interrupteur OFF le cycle ne paie
   pas une requête SQL par tour, et le motif nomme la vraie cause dominante.
3. `armed && ctx.groom_in_flight.is_none()` → refus propre au site,
   `reason = "no_in_flight_source"`, sur le patron exact de `no_relief_verdict`.

Le point 3 est une **décision contre la facilité**. Traiter `None` comme
`Unreadable` « marcherait » (direction fail-safe ⇒ tous les candidats sautés), mais
le rapport et les logs diraient alors `groom_in_flight` pour deux causes
différentes — « la base dit qu'un groom tourne » et « il n'y a pas de source » :
deux populations sous un nom (mika#2205), sur l'axe même où AC2 se lit. Un refus
propre au site les sépare, pour trois lignes, et ne touche pas `groom_trigger.rs`.
La branche est atteignable : le site de production construit la source par
`agents.get(&default_agent).map(…)`, qui rend `None` si l'agent par défaut
n'existe pas — exactement comme `DbFlightSource`.

### D5 — la direction fail-safe de la lecture est conservée, et celle de la fenêtre est l'inverse — par asymétrie des conséquences

`GroomInFlightReading::carries_groom` rend `true` pour **tout** numéro quand la
lecture est `Unreadable` : le doute doit bloquer un *envoi* (vol 2c, l'inverse
assumé de `live_pilot`). Ce vol ne touche pas cette direction.

La fenêtre durable, elle, porte un instant par entrée, et un instant peut être
**illisible** (checkpoint édité à la main, écrit par un binaire futur). Direction
retenue : une entrée dont l'instant ne se parse pas est **écartée de la fenêtre**,
sous un WARN nommé `groom_window_entry_unparsable` portant le numéro. C'est la
direction opposée à `carries_groom`, délibérément, et l'argument est l'asymétrie
des conséquences :

- faux négatif (on renvoie un groom déjà envoyé) ⇒ **un doublon**, borné par
  `GROOMS_PER_CYCLE = 1`, et le moteur n'a de toute façon qu'un créneau
  `dispatch_class = 'groom'` (schéma v34) ;
- faux positif permanent (une entrée illisible retient un numéro pour toujours)
  ⇒ **famine** : un ticket de relève que rien ne groome plus jamais, et
  silencieusement.

Un plan qui garderait ici la direction du vol 2c choisirait la seconde. La
divergence est documentée au site, avec le renvoi à la direction du vol 2c, pour
qu'un futur éditeur ne la « réaligne » pas par symétrie.

### D6 — la fenêtre est bornée par une TTL, pas par un compte

Patron de surface : `posted_flight_task_ids` (mika#2698 vol 2), champ `Vec<…>` avec
`#[serde(default)]` et une fonction pure de fenêtre. Mais la **borne** diffère, et
c'est la décision :

`POSTED_FLIGHTS_RETAINED = 500` borne `posted_flight_task_ids` en **nombre**, parce
qu'un verdict posté l'est pour toujours et que le débordement est une
re-publication visible. Ici, une borne en nombre serait un piège : à 1 groom par
cycle, évacuer une entrée coûterait N cycles, et jusque-là le numéro est muselé.
C'est la famine de D5 sous un autre nom.

La fenêtre est donc bornée **en temps** :

```rust
/// Combien de temps la fenêtre durable tient un groom pour « en cours »
/// (mika#2958 vol 2d).
///
/// Ce n'est pas la durée d'un groom : c'est la majoration du **trou de course**
/// entre `message/send` accepté et l'apparition de la tâche moteur en base, le
/// seul intervalle que la lecture container-scopée du vol 2c ne peut pas voir.
/// Au-delà, c'est cette lecture qui détient la vérité — et si elle ne voit rien,
/// le groom est fini ou mort, deux cas où un re-déclenchement est le comportement
/// voulu. Une fenêtre plus longue n'achèterait pas de sûreté : elle achèterait de
/// la famine (voir D5).
const GROOM_WINDOW: Duration = Duration::minutes(30);
```

30 minutes majore très largement le budget de tour de mika-dev (~300 s) que le
corps du ticket cite pour le groom moteur.

**La taille de la liste n'a pas besoin d'une seconde borne** : à `GROOMS_PER_CYCLE
= 1`, la TTL la borne déjà par le nombre de cycles tenant dans 30 minutes. Une
double borne est une seconde autorité sur la même question (mika#2201).

### D7 — l'élagage est appliqué à la lecture, pas seulement à l'écriture

Un cycle qui ne déclenche ni ne poste rien ne sauvegarde pas le checkpoint (R5),
donc des entrées expirées peuvent y séjourner. Sans conséquence : la fonction de
**lecture** élague à `ctx.now` avant de répondre. L'écriture élague aussi, pour que
le fichier ne grossisse pas — mais la correction ne dépend que de la lecture.

Deux fonctions pures, toutes deux testables sans base ni horloge réelle :

```rust
/// Les numéros que la fenêtre tient encore pour « déclenchés », à `now`.
/// Une entrée dont l'instant ne se parse pas est écartée (D5).
fn triggered_window_at(window: &[TriggeredGroom], now: DateTime<Utc>) -> BTreeSet<u64>

/// La fenêtre que le prochain checkpoint portera : l'ancienne élaguée, plus les
/// numéros de ce cycle. Un numéro redéclenché garde l'instant le plus récent.
fn merge_triggered(
    previous: &[TriggeredGroom],
    newly: &[u64],
    now: DateTime<Utc>,
) -> Vec<TriggeredGroom>
```

`merge_triggered` est au site de sauvegarde, exactement comme `merge_posted` : le
geste rend `declenches: Vec<u64>` et le site compose la fenêtre. `trigger_cycle_grooms`
ne rend donc pas la fenêtre — une seule chose rend la fenêtre, et c'est la fonction
pure.

### D8 — le type d'entrée est nommé, pas un tuple sérialisé

```rust
/// Un groom que **ce manager** a envoyé, et quand (mika#2958 vol 2d).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggeredGroom {
    pub number: u64,
    /// ISO 8601 UTC, format `crate::timestamp`.
    pub at: String,
}
```

Un `Vec<(u64, String)>` sérialiserait en tableaux de deux éléments, illisible dans
un checkpoint qu'un opérateur ouvre à la main pour diagnostiquer. Le champ :

```rust
/// Les grooms que ce manager a envoyés, avec leur instant — la fenêtre
/// d'idempotence durable d'AC2 (mika#2958 vol 2d). `#[serde(default)]` : un
/// checkpoint antérieur se relit en fenêtre vide, jamais en erreur.
#[serde(default)]
pub triggered_grooms: Vec<TriggeredGroom>,
```

### D9 — le neuvième paramètre de `spawn_manager_cycle_task` suit l'exemption déjà nommée

`spawn_manager_cycle_task` porte déjà `#[allow(clippy::too_many_arguments)]` avec
sa justification écrite au vol 2b (« `groom_sender` est la huitième injection et
franchit le seuil de clippy… L'idiome du dépôt pour ce cas est l'exemption
nommée »). Le neuvième paramètre `groom_in_flight: Option<Arc<dyn
GroomInFlightSource>>` entre sous cette exemption existante, avec son paragraphe
de doc `**mika#2958 vol 2d — groom_in_flight.**` comme ses huit voisins. Aucun
regroupement en struct : ce serait une réécriture de cinq appelants pour une
injection, et le plan du vol 2b a déjà tranché contre.

Chaîne complète à amender : `server/mod.rs` → `spawn_manager_cycle_task` →
`run_manager_cycle_with_auth` (7ᵉ paramètre) → `CycleContext::with_groom_in_flight`
→ `run_manager_cycle_in` → `trigger_cycle_grooms`.

### D10 — pas de `## Fire-Disposition` : ce vol ne livre aucun détecteur

Question posée explicitement. Les livrables sont : un champ d'injection, une
composition d'URL, deux fonctions pures, un champ de checkpoint, une condition de
sauvegarde élargie, et des tests **comportementaux** (le câblage atteint le
candidat ; la fenêtre bloque un renvoi ; le contrôle négatif sans fenêtre renvoie ;
l'interrupteur OFF ne lit rien). Le chemin de succès de chacun est « le
comportement attendu s'est produit », jamais « aucune violation trouvée ».

Aucune garde structurelle n'est livrée ni amendée : `no_dispatch_test.rs::EXEMPTED_OCCURRENCES`
reste à ses **trois** entrées (ce vol n'ajoute aucun SQL dans
`milestone_manager/` — la source vit dans `groom_in_flight.rs`, hors du module
scanné), `LIVE_PILOT_TASK_READER_ALLOWED` reste **vide**, `no_repo_list_test.rs`
reste satisfait par D2, et
`canonical_tokens::mika2700_le_vol_1_nouvre_aucun_chemin_decriture` n'est pas
retouché (il l'a été au vol 2a). Le gate est donc **N/A**, et la section n'est pas
inventée (mika#2306).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/cadence.rs` — `CycleContext::groom_in_flight` + `new` + `with_groom_in_flight` | 22 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` — `TriggeredGroom`, `GROOM_WINDOW`, `triggered_window_at`, `merge_triggered` | 70 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` — champ `MilestoneCheckpoint::triggered_grooms` | 8 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` — `trigger_cycle_grooms` (signature, URLs, refus, lecture, fusion) | 60 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` — site d'appel + condition de sauvegarde élargie | 20 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` — `run_manager_cycle_with_auth` + `run_manager_cycle` | 12 |
| `crates/mika-agent/src/milestone_manager/spawn.rs` — 9ᵉ paramètre, doc, passage dans la boucle | 24 |
| `crates/mika-agent/src/server/mod.rs` — construction de `DbGroomInFlightSource` | 10 |
| ajustement des appelants de test existants (`spawn.rs`, 4 sites) | 15 |
| tests (`cadence.rs`, module `mod tests` inline) | 115 |

Total estimé : 356 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 356 < 429, et le périmètre est un câblage le long
d'une chaîne déjà construite : aucun type neuf hors `TriggeredGroom`, aucune
requête SQL, aucun fichier neuf. Le découpage n'est pas évité : la moitié ON
d'AC4 (le rendu de rapport, ≈ 120 lignes) est **sortie** de ce vol et nommée par un
`Tracked in:` — la garder porterait le total à ≈ 476, au-dessus de la borne. La
calibration de ce ticket justifie la marge : le vol 2b avait été estimé en deçà de
sa réalité (≈ 652 réelles), donc 356 est tenu volontairement à ~83 % de la borne
plutôt qu'à son ras.

## Ordre de commit

Chaque étape compile et la suite passe. Le plancher de chaque étape est ce sans
quoi la suivante n'a pas de sens.

1. **`TriggeredGroom`, `GROOM_WINDOW`, les deux fonctions pures, et le champ de
   checkpoint.** *Plancher* : `cargo test -p mika-agent milestone_manager::cadence`
   vert, avec les tests des deux fonctions pures (élagage, dédoublonnage, entrée
   illisible écartée) et la relecture d'un checkpoint JSON **sans** le champ neuf.
   Rien n'appelle encore ces fonctions : l'étape est isolée exprès, parce que c'est
   la seule qui porte une décision de direction (D5).
2. **`CycleContext::groom_in_flight` + `new` + `with_groom_in_flight`.**
   *Plancher* : compile, suite verte, aucun comportement changé (`None` partout).
3. **`trigger_cycle_grooms` : signature (D3), composition d'URL (D2), ordre des
   refus (D4), consommation de la lecture et de la fenêtre.** Le commentaire
   `Tracked in: vol 2c` sur `groom_en_cours` disparaît : il est honoré.
   *Plancher* : les tests de comportement du geste passent — câblage atteint le
   candidat, fenêtre bloque, négatif sans fenêtre, OFF ne lit rien.
4. **Le site d'appel et la condition de sauvegarde (R5).** `_grooms` cesse d'être
   `_`. *Plancher* : un cycle qui déclenche sans livrer sauvegarde, et les trois
   champs de cadence sont **préservés** dans la branche de non-livraison.
5. **`run_manager_cycle_with_auth` (7ᵉ paramètre) et `run_manager_cycle`.**
   *Plancher* : compile, suite verte.
6. **`spawn_manager_cycle_task` (9ᵉ paramètre, D9) et les 4 appelants de test.**
   *Plancher* : `cargo test -p mika-agent milestone_manager::spawn` vert.
7. **`server/mod.rs` — la construction.** *Plancher* : `cargo clippy --all-targets
   -- -D warnings`, `cargo fmt --check`, la suite complète, les lints du dépôt
   (`make verify-bundled-skills` non concerné), et
   `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- La chaîne d'injection est complète de `server/mod.rs` à `trigger_cycle_grooms`,
  et `groom_en_cours` n'est plus un littéral.
- `MilestoneCheckpoint` porte `triggered_grooms` avec `#[serde(default)]`, et un
  checkpoint écrit avant ce vol se relit sans erreur.
- La fenêtre expire : un test fixe une horloge au-delà de `GROOM_WINDOW` et montre
  que le numéro redevient déclenchable.
- Avec `MIKA_MANAGER_GROOM_WRITE` absent : zéro envoi, zéro lecture en base, motif
  `disarmed`.
- `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
  verts ; `scripts/verify-pipeline.sh origin/main` vert.
- `no_dispatch_test.rs`, `no_repo_list_test.rs` et
  `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` passent **sans amendement**.
- Le corps de PR porte le `Tracked in:` du vol 2e.

## Acceptance criteria

Transcrits verbatim du corps de senara-solutions/mika#2958.

- **AC1.** Avec l'interrupteur OFF, aucun groom n'est déclenché. C'est une garantie structurelle, avec un contrôle négatif dans le même test. Le geste entre comme **exception nommée** au scan de `no_dispatch_test.rs`, sur le patron des entrées de mika#2698 vol 2 et de mika#2699 vol 4. Le scan n'est pas assoupli. Le test `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` est amendé dans le même commit, et la PR dit ce qui le faisait rougir.
- **AC2.** Sélection et idempotence : un ticket déjà groomé, ou dont un groom est en cours, n'est pas relancé. Le plan dit par quelle lecture le manager voit un groom en cours : les tâches moteur `Groom …` et `ready-label`, ou autre chose. Le motif est la collision d'un groom manuel avec un groom moteur.
- **AC3.** La voie est celle du moteur : la même que `mika ask --agent mika-dev "groom mika#N"`, ou son équivalent interne. Elle n'ouvre ni spawn ni dispatch d'implémentation. Le plan dit comment le geste reste borné : combien de grooms par cycle, et ce qu'il fait quand mika-dev a déjà un pilote en vol, puisque le moteur n'en fait tourner qu'un à la fois.
- **AC4.** Ce qui reste de l'ombre se lit dans le rapport. Interrupteur OFF : la section « Relève à grommer » est inchangée. Interrupteur ON, en test seulement : le rapport dit quels grooms ont été déclenchés, et lesquels ont été sautés, avec leur motif.
- **Taille** : à estimer tests compris, et à découper en vols au-delà de la borne.

### Couverture par ce vol

| AC | état | ce que ce vol y ajoute |
|---|---|---|
| AC1 | **fermé au vol 2a**, préservé ici | R6/D4 : l'interrupteur est lu avant toute lecture en base, et le motif reste `disarmed`. L'exception nommée et l'amendement de `mika2700_…` ont été livrés au vol 2a ; ce vol ne les retouche pas, et un test l'atteste. |
| AC2 | **fermé par ce vol** | la moitié « groom en cours » : la lecture du vol 2c est consommée (R2) et complétée par la fenêtre durable (R3/R4/D6) qui couvre le trou de course que la lecture ne peut pas voir. La moitié « déjà groomé » est fermée depuis le vol 2b par `RefusalReason::remediable_by_grooming`. |
| AC3 | **fermé aux vols 2a/2b**, préservé ici | la voie moteur et `GROOMS_PER_CYCLE = 1` sont inchangés ; ce vol ne touche pas `envoyer` ni son `AgentBusy ⇒ break`. |
| AC4 | moitié OFF **fermée au vol 1** ; moitié ON **hors périmètre** | ce vol **lie** `GroomTriggerOutcome` au site d'appel (il cesse d'être `_grooms`) pour que le vol 2e soit l'ajout d'une ligne et non une reprise du site. |
| Taille | — | section `## Taille estimée` ci-dessus, 356 lignes, un vol, suite nommée par `Tracked in:`. |

## Hors périmètre

- **vol 2e — le rendu de rapport (AC4, moitié ON).** Le rapport dit quels grooms
  ont été déclenchés et lesquels ont été sautés, avec leur motif. Après ce vol,
  `GroomTriggerOutcome` est **lié** au site d'appel et non jeté, donc le vol 2e est
  une ligne de passage vers `Reporter::report_with_auth_notes` plus sa mise en
  forme et ses tests de rendu. Estimé ≈ 120 lignes.
  **Tracked in: senara-solutions/mika#2958**
- **L'ouverture de `MIKA_MANAGER_GROOM_WRITE`.** Geste opérateur, au seuil écrit
  dans mika#2700 : `MIKA_MANAGER_EVALUATE_WRITE` et `MIKA_MANAGER_POOL_WRITE`
  ouverts et stables sur un cycle complet. Les deux sont OFF aujourd'hui.
- **La canonicalisation de `reference_url` au point d'écriture** (mika#2638 § 8 S1).
  La lecture du vol 2c normalise par `issue_url_variants` ; rendre l'ensemble clos
  au point d'écriture reste le périmètre de mika#2638.
- **`reader.rs`, `types.rs`, `evidence/guards.rs`, `skills/builtin_handlers.rs`** —
  non touchés, et aucun littéral `MilestoneState { … }` ajouté.

## Sonde post-déploiement

Statique — aucune n'exige l'interrupteur ouvert.

- **S1.** `grep -n "groom_en_cours" crates/mika-agent/src/milestone_manager/cadence.rs`
  ne rend plus de ligne où la valeur est le littéral `false`, et ne rend plus le
  commentaire `Tracked in: vol 2c`.
- **S2.** `grep -c "triggered_grooms" crates/mika-agent/src/milestone_manager/cadence.rs`
  ≥ 4 (champ, lecture, fusion, test de relecture sans le champ).
- **S3.** `grep -n "DbGroomInFlightSource" crates/mika-agent/src/server/mod.rs`
  rend exactement une construction, dans la liste d'arguments de
  `spawn_manager_cycle_task`.
- **S4.** `cargo test -p mika-agent no_dispatch_test no_repo_list_test` vert, et
  le compte d'entrées d'`EXEMPTED_OCCURRENCES` dans
  `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs` inchangé : trois,
  pas quatre.
- **S5.** Après `make deploy`, un cycle manager avec l'interrupteur OFF émet
  `manager_groom_not_triggered` avec `reason = "disarmed"` — **jamais**
  `no_in_flight_source` ni `groom_in_flight` — et `groom_in_flight_read_failed`
  (vol 2c) n'apparaît pas, puisque aucune lecture n'est payée (R6/D4).
