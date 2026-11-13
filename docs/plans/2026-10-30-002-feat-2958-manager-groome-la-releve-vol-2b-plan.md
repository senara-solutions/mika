# mika#2958 — le manager groome la relève d'avance (vol 2b : câblage de cycle)

> **Vol 2b de mika#2958.** Le vol 1 (PR #2740) nomme la relève en mode ombre. Le vol 2a
> (PR #2970, mergée le 2026-10-30 à 07:02:41Z, commit `9d6c1225`) livre le **rail** :
> `groom_trigger.rs`, le `GroomSender`, l'`A2aGroomSender`, la classification des sorties
> et `trigger_relief_grooms`. Ce vol-ci **branche** le rail au cycle : il résout l'URL, le
> jeton et le budget à leur site d'appel, injecte l'expéditeur dans le contexte de cycle,
> sélectionne les candidats depuis la relève, et pose la garantie structurelle d'AC1 de
> bout en bout — la production entière présente et prouvée silencieuse interrupteur fermé.

## Périmètre : ce que ce vol touche hors de la zone déclarée

Le corps du ticket déclare la zone attendue (`groom_trigger.rs`, `cadence.rs` hors tests,
`spawn.rs` hors tests) et interdit six fichiers ou blocs. Ce vol **ne touche aucun des six
interdits** : ni `milestone_manager/reader.rs`, ni `milestone_manager/types.rs`, ni aucun
littéral `MilestoneState { … }` (tests compris — les tests de cadence passent par les
constructeurs existants du module de tests), ni le bloc `pub use types::{…}` ou
`pub use reader::{…}` de `milestone_manager/mod.rs`, ni `crates/mika-agent/src/evidence/guards.rs`,
ni `skills/builtin_handlers.rs`.

Il sort de la zone attendue sur quatre points, et les voici en tête, avec leur raison :

1. **`crates/mika-agent/src/server/mod.rs`** (≈ 22 lignes, au site de spawn du manager,
   l. 1727-1810). C'est le **seul** endroit du dépôt qui tient déjà le jeton requis :
   `let internal_token = settings.internal_token.clone().ok_or_else(…)` à la l. 1030, et
   l'exposition sanctionnée `.expose_secret().to_string()` à la l. 700. Construire
   l'`A2aGroomSender` ailleurs demanderait un second lecteur de la clé de configuration —
   exactement ce que mika#2201 refuse. La zone déclarée ne pouvait pas le prévoir : le
   vol 2a a laissé la résolution « au câblage de cycle », et son site d'appel est ici.
2. **`.env.example`** (≈ 14 lignes). `MIKA_MANAGER_GROOM_URL` est une constante `ENV_*` qui
   vit dans `groom_trigger.rs`, et le test T7
   `mika2267_every_manager_env_const_is_declared_in_env_example` scanne **tout** le
   répertoire `milestone_manager/` : une constante non déclarée dans `.env.example` rougit.
   La déclaration n'est donc pas une courtoisie de documentation, c'est une contrainte
   structurelle. Le même commit corrige la phrase devenue fausse du bloc existant
   (l. 455-483), qui dit qu'« aucun rail n'est câblé, donc un interrupteur armé ne dit que
   `reason: no_sender` ».
3. **Le module de tests de `spawn.rs`** (4 lignes, une par site d'appel). Le 8ᵉ paramètre de
   `spawn_manager_cycle_task` force la mise à jour de ses 5 sites d'appel :
   `server/mod.rs:1740` (production) et `spawn.rs:2046, 2513, 2526, 2691` (tests). Ces
   quatre-là sont dans le module de tests, que la zone exclut. Le coût est d'une ligne
   chacun — ajouter `None` — et il n'y a pas d'alternative : un paramètre optionnel sans
   site d'appel ne compile pas.
4. **`docs/egress/egress-manifest.toml`** (sous `docs/`, donc **non compté** dans la taille).
   Le sink `manager-groom-a2a` existe déjà (vol 2a, l. 306-315). Ce vol ne construit **aucun
   nouveau client reqwest**, donc l'inventaire mécanique de `scripts/verify-egress-manifest.sh`
   reste satisfait sans nouvelle entrée. La seule modification est la dernière phrase du champ
   `notes` — « le site d'appel qui les résout arrive au vol 2b » — que ce vol rend fausse.
   mika#2408 veut l'entrée dans le commit du client qui en a besoin ; ici le client est déjà
   là, c'est sa note qui bouge.

## Contexte

### Ce que le rail du vol 2a sait déjà faire

`crates/mika-agent/src/milestone_manager/groom_trigger.rs` (999 l.) porte :

- `ENV_GROOM_WRITE = "MIKA_MANAGER_GROOM_WRITE"` et `groom_write_is_armed()`, **seul lecteur**
  de la variable dans le dépôt, avec son prédicat pur `armed_from_value(Option<&str>)` —
  mot pour mot celui de `pool_writer`.
- `GESTE: &str = "message/send"` — plat exprès, parce que la forme éclatée échapperait au
  grep du scan de `no_dispatch_test.rs`. C'est la **troisième et dernière** entrée de son
  allowlist, posée au vol 1 (`8d127552e`), épinglée par `len() == 3`.
- `GROOMS_PER_CYCLE: usize = 1` — non par prudence : un seul créneau
  `dispatch_class = 'groom'` par agent depuis le schéma v34.
- Le trait `GroomSender`, l'erreur `GroomSendError { AgentBusy, CredentialRefused, Unreachable, Other }`
  et sa `classe()`, l'`A2aGroomSender::new(url, token, budget)`, `build_send_body`
  (sans `configuration`, exprès), `classify_send_outcome` (lit `body.error.code` **avant** le
  statut).
- `GroomCandidate { number, deja_groome, groom_en_cours }` — les deux booléens sont des
  **lectures faites par l'appelant**, et le vol 2a dit explicitement que ces lectures arrivent
  avec le site d'appel.
- `trigger_relief_grooms(armed, sender, candidats) -> GroomTriggerOutcome` et son unique site
  de motifs de cycle, `cycle_refusal(armed, sender_present, releve_vide)`.

Ce que le vol 2a **ne fait pas**, et que le docstring de `A2aGroomSender::new` annonce
textuellement : « Leur résolution depuis l'environnement arrive avec son site d'appel, au
câblage de cycle. » Aujourd'hui `trigger_relief_grooms` n'a **aucun appelant en production**.
Interrupteur armé, le manager ne dirait que `reason: no_sender`.

### D'où viennent les candidats

`relief.rs` (544 l., pur, lu sans être modifié) produit
`ReliefVerdict { next: Vec<ReliefTicket>, beyond }` avec `RELIEF_SIZE = 4`.
`name_relief` filtre `verdict.refusals` sur `r.reason.remediable_by_grooming()`
(`pool.rs:773`, exhaustif, sans `_`) **avant** de calculer `beyond`. Sa décision de module 1
affirme déjà la moitié structurelle d'AC2 : « un ticket déjà groomé n'est pas un refus, donc
n'est pas une relève ; un ticket en cours de groom porte un siège `dispatch:`, donc n'est pas
un refus non plus. »

La relève est portée par `MilestoneAssessment.relief: Option<ReliefVerdict>` (`types.rs:459`,
**lu, jamais écrit par ce vol**), peuplée à `assessor.rs:89` et `:117`. Le docstring du champ
dit pourquoi c'est une `Option` : « `None` dit "la relève n'a pas été nommée" … Une
`ReliefVerdict` par défaut dirait "rien à grommer d'avance", ce qui est une *mesure* et non
son absence (motif mika#2331 : un null n'est jamais un 0). »

### Pourquoi la voie est A2A et pourquoi l'appelant ne lit pas la base

AC3 veut la voie moteur, « la même que `mika ask --agent mika-dev "groom mika#N"` ». C'est
`POST /a2a/mika-dev` avec `message/send` (`server/mod.rs:388-396`), derrière
`auth::require_internal_token`, qui n'accepte **que** `MIKA_INTERNAL_TOKEN`.

AC3 demande aussi ce que le geste fait « quand mika-dev a déjà un pilote en vol ». La réponse
est que **demander au moteur *est* la lecture** : le moteur répond `AGENT_BUSY` (−32000),
`classify_send_outcome` le reconnaît par son code, le cycle s'arrête sur le motif `agent_busy`.
Il n'y a pas de pré-lecture en base, et mika#2792 dit pourquoi une pré-lecture serait fausse :
chaque accesseur `*_for_class` d'`AsyncDatabase` clone `self.agent_id`, le manager n'a pas
d'agent, donc une lecture agent-scoped rend une constante `false`. 43 vols en quatre jours sans
verdict l'ont montré. Le précédent container-scoped est
`list_finished_implement_flights_in_container` (pas d'`agent_id` dans le SQL) — c'est la forme
que prendra la lecture durable du vol 2c, pas une lecture agent-scoped d'ici.

### Où se branche l'injection

`CycleContext<'a>` (`cadence.rs:500`) a **zéro site de construction par littéral** dans tout le
dépôt : il se bâtit par `with_flight_source` (558), `with_verdict_ledger` (563),
`with_gh_runner` (568). Un champ de plus plus son builder est donc du churn zéro — c'est
l'idiome d'injection déjà porté par `token_resolver` (mika#2013), `auth_ledger` (mika#1949),
`liveness_sink` (mika#1990), `flight_source` (mika#2698), `verdict_ledger` (mika#2922).

`ManagerConfig` (`cadence.rs:44`) est écarté exprès : il a quatre sites de littéral, dont
`liveness.rs:497`, **hors zone**. Un champ là-bas sortirait du périmètre sans rien gagner.

## Requirements

### R1 — La résolution du rail, dans le fichier qui le nomme

Dans `groom_trigger.rs` :

```rust
pub const ENV_GROOM_URL: &str = "MIKA_MANAGER_GROOM_URL";

/// Le budget du geste de groom, dérivé du budget de tour de la flotte.
///
/// Le facteur 2 n'est pas une marge de confort : il **majore** le budget de tour de
/// mika-dev. Le geste est synchrone, et si le manager abandonne la requête avant que le
/// tour ait rendu la main, le tour continue côté mika-dev, le groom part, et le manager le
/// compte `send_failed` — puis le relance au cycle suivant : double groom sous un rapport
/// qui dit « échec ». Dérivé du site de vérité du défaut de flotte plutôt que retapé, pour
/// que la valeur suive si ce défaut bouge.
pub const GROOM_SEND_BUDGET: Duration =
    Duration::from_secs(2 * mika_common::llm::budget::DEFAULT_AGENT_TOTAL_TIMEOUT_SECS);

pub fn resolve_groom_rail(internal_token: Option<&str>) -> Option<A2aGroomSender>;
```

`DEFAULT_AGENT_TOTAL_TIMEOUT_SECS` vaut 300 (`crates/mika-common/src/llm/budget.rs:218`), donc
600 s — le même nombre que le plancher `client >= total` de la CLI, pour la même raison
(incident du 2026-09-11, mika#2759).

`resolve_groom_rail` a trois issues, sur le patron de la décision D4 de `liveness` :

1. `ENV_GROOM_URL` absente ou vide après `trim` ⇒ `None`. **C'est l'état nominal**, pas une
   erreur : le cycle dira `reason = "no_sender"`.
2. URL présente, `internal_token` à `None` ⇒ `None`, avec un refus nommé dans la sonde. Un
   rail sans jeton ne peut produire que des `401`.
3. Les deux présents ⇒ `Some(A2aGroomSender::new(url, token.to_string(), GROOM_SEND_BUDGET))`.

**Aucune dérivation depuis une URL voisine** (§ R2 de `liveness`) : l'URL du groom ne se déduit
pas de celle de la liveness ni du routage. Une URL dérivée est une URL que personne n'a
déclarée, et quand elle est fausse elle l'est en silence.

Une sonde de résolution, sur le patron de `manager_delivery_resolved` (mika#2293 — un réglage
qu'on ne peut pas observer n'est pas un réglage) :

```
manager_groom_rail_resolved { url_set: bool, token_set: bool, budget_secs: u64 }
```

Elle est émise aux trois issues, et elle ne porte **ni** l'URL **ni** le jeton, seulement leur
présence.

**Pas de garde de forme sur l'URL**, et la raison est écrite plutôt que tue : vérifier que le
chemin est bien un `/a2a/{nom}` à segment unique coûte ≈ 45 lignes contre la borne, et une URL
mal dirigée est **déjà** visible — un non-2xx devient `send_failed` sous
`manager_groom_failed`, et la sonde de résolution dit que l'URL était posée. Le
`budget_floor::local_a2a_agent` de `mika-cli` n'est pas importable depuis `mika-agent`, et le
redupliquer ferait un second lecteur d'une question différente.

### R2 — Le câblage de cycle

Dans `cadence.rs`, hors du module de tests :

- `CycleContext<'a>` reçoit `groom_sender: Option<&'a dyn GroomSender>` et son builder
  `with_groom_sender`, strictement sur le patron de `with_verdict_ledger` (l. 563).
- Un helper `trigger_cycle_grooms`, calqué sur `write_pool_admissions` (l. 836) :

```rust
async fn trigger_cycle_grooms(
    ctx: &CycleContext<'_>,
    assessment: &MilestoneAssessment,
) -> GroomTriggerOutcome;
```

  Il porte **un seul** motif qui lui appartienne, `no_relief_verdict`, pour
  `assessment.relief == None` — une lecture absente, jamais une relève vide (mika#2331 et
  mika#2205 : jamais deux populations sous un même nom). Les **trois** motifs de fil
  (`disarmed`, `no_sender`, `no_relief`) ne sont pas redits ici : ils restent à leur site
  unique `cycle_refusal`, et `trigger_relief_grooms` les rend (mika#2201).

  Il lit l'interrupteur par `groom_write_is_armed()` au moment de l'appel, exactement comme
  `write_pool_admissions` lit `pool_write_is_armed()`, puis délègue.

- Son appel dans `run_manager_cycle_in` (l. 891), une ligne, immédiatement après le geste du
  bassin de la l. 947 :

```rust
let _admitted = write_pool_admissions(cfg, ctx, &state, &assessment).await;
// mika#2958 vol 2b — le geste de la relève. Lié ici, après le bassin, parce que la relève
// dérive du verdict de bassin : la nommer avant de l'avoir mesurée n'a pas de sens.
let _grooms = trigger_cycle_grooms(ctx, &assessment).await;
```

  Le résultat est **lié et non jeté**, pour que le rendu du rapport au vol 2d soit l'ajout
  d'une ligne et non une reprise du site — exactement la raison écrite au-dessus de
  `_admitted`.

### R3 — Le paramètre de spawn

`spawn_manager_cycle_task` prend un 8ᵉ paramètre, `groom_sender: Option<Arc<dyn GroomSender>>`,
à la suite de `verdict_ledger`, et le pose dans le `CycleContext` par `with_groom_sender` à
chaque tour de boucle. `None` est l'état nominal, comme pour `flight_source` et
`verdict_ledger`.

`Arc` plutôt que la propriété nue, parce que le contexte est rebâti à chaque cycle et que
l'expéditeur, lui, vit aussi longtemps que la tâche. `manager_config_from_env` (l. 257) n'est
**pas** touché : la résolution du rail n'est pas une affaire de `ManagerConfig` (cf. Contexte),
elle appartient au site qui tient le jeton.

Les quatre sites d'appel du module de tests (`spawn.rs:2046, 2513, 2526, 2691`) reçoivent
`None`.

### R4 — La construction en production, et la déclaration d'environnement

Dans `server/mod.rs`, au site de spawn du manager (l. 1727-1810) : résoudre le rail depuis le
jeton déjà en portée à la l. 1030, et le passer au spawn.

```rust
// mika#2958 vol 2b — le rail du geste de groom se résout ici, parce que c'est ici que le
// jeton interne est en portée. `/a2a/{agent}` n'accepte que MIKA_INTERNAL_TOKEN.
let groom_sender = milestone_manager::resolve_groom_rail(Some(internal_token.as_str()))
    .map(|s| Arc::new(s) as Arc<dyn milestone_manager::GroomSender>);
```

La réexportation de `resolve_groom_rail` et de `GroomSender` va dans le bloc
`pub use groom_trigger::{…}` de `mod.rs` — **son propre module**, ni le bloc `types`, ni le
bloc `reader`, conformément à la contrainte du ticket.

Dans `.env.example`, bloc du manager (l. 325-490) : déclarer `MIKA_MANAGER_GROOM_URL` avec son
défaut (absent ⇒ rail désarmé ⇒ `reason: no_sender`), sa forme
(`http://127.0.0.1:8080/a2a/mika-dev`), le fait que le jeton n'est **pas** une variable à part
(c'est `MIKA_INTERNAL_TOKEN`, déjà requis par mika-spirit), et le budget de 600 s avec sa
raison. Et corriger la phrase du bloc `MIKA_MANAGER_GROOM_WRITE` (l. 455-483) devenue fausse :
le rail **est** câblé à partir de ce vol ; ce qui manque avant l'ouverture, c'est la fenêtre
d'idempotence durable du vol 2c, et le bloc le dit comme précondition.

### R5 — La sélection des candidats

La relève donne des `ReliefTicket`, le rail veut des `GroomCandidate`. La traduction est le
cœur d'AC2, et chaque booléen est une lecture dont la dérivation est écrite :

```rust
relief.next.iter().map(|t| GroomCandidate {
    number: t.number,
    // `deja_groome` ne se lit PAS sur `plan_present` / `plan_reading` : c'est la lecture
    // fausse. `UngroomedForDispatch` cite un plan et c'est pourtant un groom qui le
    // débloque ; `EstimateOverFlightBound` a un plan dont le remède est un re-groom. La
    // dérivation honnête est le motif de refus lui-même, lu à son site unique.
    deja_groome: !t.motive.remediable_by_grooming(),
    // La fenêtre entre `message/send` et la pose du siège par le moteur n'est pas lisible
    // d'ici sans état durable. Agent-scoped, la lecture rendrait une constante `false`
    // (mika#2792). `false` est donc la valeur honnête à ce vol, et `GROOMS_PER_CYCLE = 1`
    // borne le dégât à un doublon au pire.
    // Tracked in: vol 2c — fenêtre d'idempotence durable + lecture container-scoped des
    // tâches moteur `Groom …` / `ready-label`.
    groom_en_cours: false,
})
```

`deja_groome` sera **toujours** `false` pour un ticket de relève, par construction : `name_relief`
a déjà filtré sur ce même prédicat. Ce n'est pas du code mort, c'est un **contrôle de cohérence** :
si la relève cessait un jour de filtrer, le candidat continuerait de refuser au lieu de grommer
un ticket que rien ne débloque. Le prédicat est appelé à son site unique `pool.rs:773`, jamais
réécrit (mika#2201).

Une lecture par label est écartée avec sa raison : `assess_pool` écarte déjà silencieusement les
porteurs de siège (`labels_hold_dispatch_seat`, `pool.rs:1012-1100`), donc un `groom_en_cours`
tiré des labels serait du code inerte — il ne pourrait jamais être vrai.

## Décisions

**D1 — Le budget est un argument, et il vaut 600 s dérivées.** Le vol 2a a fait du budget un
argument exprès : « celui qui le choisit est celui qui connaît le budget de tour ». Ce vol est
ce choisisseur. Aucune surcharge d'enveloppe propre à mika-dev n'existe dans le dépôt : la clé
par agent `agent_total_timeout_secs` vit dans le `config.toml` de l'agent, propriété de
l'opérateur, et retombe sur le défaut de flotte
(`docs/operator/environment-variables.md:2799` — « Absent/empty → the fleet default »). D'où
`2 × DEFAULT_AGENT_TOTAL_TIMEOUT_SECS`.
**Danger résiduel nommé plutôt que caché :** un agent dont le `config.toml` relève l'enveloppe
au-dessus de 600 s réouvre la fenêtre du double groom. Sa sonde est un
`manager_groom_failed { class = "send_failed" }` récurrent à ≈ 600 s, croisé avec
`mika agents budget --agent mika-dev`. Le remède durable — résoudre le budget par le chemin
d'attestation de mika#2759 — demanderait un lecteur d'attestation en loopback dans le manager.
*Tracked in: suivi à ouvrir si la sonde parle ; pas de ce vol.*

**D2 — `resolve_groom_rail` prend le jeton en paramètre, pas en lecture d'environnement.** Le
jeton est déjà résolu et validé (64 hex, `config.rs:2832-2833`) par `Settings`, exposé au seul
endroit sanctionné. Le relire depuis l'environnement dans `groom_trigger.rs` ferait un second
lecteur de la même clé, avec une validation en moins.

**D3 — L'URL n'est pas dérivée, elle est déclarée.** Cf. R1. C'est la décision R2 de
`liveness`, reprise mot pour mot dans sa forme.

**D4 — L'absence de rail n'est pas une erreur de démarrage.** `resolve_groom_rail` rend
`Option`, jamais `Result`, et mika-spirit démarre sans rail de groom comme il démarre sans
`liveness_sink`. L'interrupteur est OFF par défaut : refuser de démarrer pour un rail absent
transformerait une capacité en option en une dépendance dure.

**D5 — Le résultat du geste est lié, pas jeté.** Cf. R2. Le vol 2d rend le rapport ON ; il doit
être l'ajout d'une ligne.

**D6 — Aucune touche à `types.rs`, et aucun nouveau littéral `MilestoneState`.** Les tests de
cadence construisent leur état par les constructeurs existants du module de tests. Vérifié :
`grep -rn "MilestoneCheckpoint {" crates/` ne rend que deux sites, tous deux en production dans
`cadence.rs` (1069, 1081) — et ce vol **n'en touche aucun**, parce que la fenêtre d'idempotence
durable est au vol 2c.

**D7 — Le test ON/OFF suit l'idiome en place.** Les prédicats purs se testent sans
`unsafe { set_var }`, comme le dit `pool_writer.rs:21` : « sans `unsafe { set_var }`, et il
reste exactement un lecteur de la variable ». Là où l'environnement doit vraiment bouger, c'est
`#[serial_test::serial]` + `unsafe { set_var }` / `remove_var`, exactement comme le fait déjà
le couple OFF/ON du bassin (`cadence.rs:2745-2753`, assertion AC1 à la l. 2806). Treize tests
du module portent déjà l'attribut.

## Fire-Disposition

**Option (a) — exception nommée en allowlist**, et la précision qui compte : ce vol
**n'ajoute aucune exception**, parce que la seule qu'il lui faudrait est **déjà là**.

- **Le détecteur concerné** est le scan structurel de
  `crates/mika-agent/src/milestone_manager/no_dispatch_test.rs`, dont le chemin de succès est
  « aucun chemin d'écriture non déclaré dans `milestone_manager/` ».
- **La violation existante est déjà nommée.** Le jeton `message/send` est la **troisième**
  entrée de l'allowlist, posée par le vol 1 (`8d127552e`) sur le patron des entrées de
  mika#2698 vol 2 et mika#2699 vol 4. Elle nomme la donnée précise (le jeton, plat, dans
  `groom_trigger.rs`) et référence mika#2958.
- **L'assertion auto-nettoyante est l'épinglage `len() == 3`.** Elle rougit si le nombre de
  jetons bouge dans un sens ou dans l'autre : une exception devenue inutile fait échouer le
  test aussi sûrement qu'une exception manquante. C'est ce qui l'empêche d'être l'allowlist
  que mika#2309 condamne.
- **Ce vol n'ajoute aucun jeton d'écriture.** Le site d'appel de cadence, le paramètre de
  spawn et la construction serveur ne portent aucun verbe d'écriture : le seul geste est
  derrière l'`A2aGroomSender` déjà couvert. Le scan **n'est pas assoupli** ;
  `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` reste vert sans amendement, et le test de
  ce vol est ce qui le prouve de bout en bout, production câblée comprise.

Ce vol livre par ailleurs ses propres détecteurs — l'assertion AC1 et son contrôle négatif —
et ils sont **armés**, pas `#[ignore]`.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/groom_trigger.rs` (hors tests : `ENV_GROOM_URL`, `GROOM_SEND_BUDGET`, `resolve_groom_rail`, sonde) | 70 |
| tests (`groom_trigger.rs` : trois issues de résolution, budget dérivé, absence de dérivation d'URL) | 65 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` (hors tests : champ + builder, `trigger_cycle_grooms`, appel de cycle) | 80 |
| tests (`cadence.rs` : double enregistreur, AC1 OFF + contrôle négatif ON, `no_relief_verdict`, sélection) | 90 |
| `crates/mika-agent/src/milestone_manager/spawn.rs` (hors tests : 8ᵉ paramètre, pose au contexte) | 25 |
| tests (`spawn.rs` : quatre sites d'appel à `None`) | 8 |
| `crates/mika-agent/src/server/mod.rs` (résolution + passage au spawn) | 22 |
| `.env.example` (`MIKA_MANAGER_GROOM_URL`, correction du bloc `GROOM_WRITE`) | 14 |

Total estimé : 374 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient, et pourquoi il a été re-découpé.** 374 passe sous la borne de 429.
La variante qui portait en plus la fenêtre d'idempotence durable pesait 431 estimées — déjà
au-dessus. Et la calibration du vol 2a mord : estimé ≈ 370, atterri à 432, soit **+17 %**.
À ce facteur, 431 devient ≈ 504, un risque réel de mort au plafond ; 374 devient ≈ 438, ce qui
tient avec la fin de pipeline. Le découpage retenu :

- **Vol 2b (ce plan, 374)** — résolution du rail, câblage de cycle, paramètre de spawn,
  construction serveur, sélection des candidats. Ferme **AC1 dans sa forme la plus forte**
  (production entière présente et prouvée silencieuse, contrôle négatif compris), la moitié
  **bornage** d'**AC3** (`GROOMS_PER_CYCLE = 1` + l'arrêt sur `agent_busy`), et la moitié
  **sélection** d'**AC2**.
- **Vol 2c** — la fenêtre d'idempotence durable (un champ de `MilestoneCheckpoint`,
  `merge_posted` généralisée en `merge_retained<T: Clone>` pour qu'il y ait un lecteur et non
  deux) **et** la lecture container-scoped des tâches moteur `Groom …` / `ready-label`. Ferme
  la moitié **« non relancé »** d'**AC2**. C'est la précondition que `.env.example` déclare
  avant toute ouverture de l'interrupteur.
  *Tracked in: suivi vol 2c de mika#2958.*
- **Vol 2d** — le rendu ON du rapport (quels grooms déclenchés, lesquels sautés et pourquoi).
  Ferme la moitié **ON** d'**AC4** ; la moitié **OFF** est close par ce vol-ci, la section
  « Relève à grommer » restant inchangée par construction.
  *Tracked in: suivi vol 2d de mika#2958.*

Chaque vol est mergeable seul : 2b laisse un rail câblé et silencieux, ce qui est l'état
nominal voulu par le ticket.

## Ordre de commit

1. **`groom_trigger.rs` + ses tests** — `ENV_GROOM_URL`, `GROOM_SEND_BUDGET`,
   `resolve_groom_rail`, la sonde de résolution. Compile et teste seul : aucun appelant encore.
2. **`.env.example`** — déclaration de `MIKA_MANAGER_GROOM_URL` et correction du bloc
   `GROOM_WRITE`. Dans ce commit-ci, parce que T7 scanne le répertoire et rougirait entre 1 et 2.
3. **`cadence.rs` + ses tests** — champ `groom_sender`, `with_groom_sender`,
   `trigger_cycle_grooms`, son appel, le double enregistreur, AC1 OFF et son contrôle négatif ON.
   Le contexte accepte l'expéditeur avant que quiconque en fournisse un : `None` partout, vert.
4. **`spawn.rs` + ses quatre sites de test** — le 8ᵉ paramètre et sa pose au contexte.
5. **`server/mod.rs`** — la résolution en production et le passage au spawn. Dernier, parce
   que c'est le commit qui rend le chemin réel ; tout ce qui le précède est vert sans lui.
6. **`docs/egress/egress-manifest.toml`** — la phrase de `notes` rendue fausse par le commit 5.

**Plancher :** les commits 1 et 2 ensemble. En deçà, T7 rougit.

## Definition of Done

- `cargo test -p mika-agent` vert, `cargo clippy -p mika-agent --all-targets --all-features -- -D warnings`
  sans avertissement (le `--all-targets` vérifie `lib` sans `cfg(test)` : `resolve_groom_rail`
  doit avoir son appelant de production, et il l'a au commit 5).
- `cargo fmt --check` vert.
- `scripts/verify-egress-manifest.sh` vert : aucun nouveau site de construction de client
  reqwest, donc aucun nouveau sink requis.
- `scripts/verify-pipeline.sh origin/main` vert.
- `mika2267_every_manager_env_const_is_declared_in_env_example` vert avec
  `MIKA_MANAGER_GROOM_URL` déclarée.
- `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` vert **sans amendement** : ce vol n'ajoute
  aucun jeton d'écriture, et la PR le dit.
- L'allowlist de `no_dispatch_test.rs` reste épinglée à `len() == 3`.
- Le test AC1 de cadence : un `GroomSender` enregistreur injecté dans `CycleContext`,
  `run_manager_cycle_in` lancé avec `MIKA_MANAGER_GROOM_WRITE` absente ⇒ **zéro** appel ; même
  graine avec la variable à `"1"` ⇒ **exactement un** appel. Les deux sous
  `#[serial_test::serial]`.
- Aucun littéral `MilestoneState { … }` ni `MilestoneCheckpoint { … }` ajouté, tests compris.
- `git diff --stat` ne montre ni `reader.rs`, ni `types.rs`, ni les blocs `pub use types`/
  `pub use reader` de `mod.rs`, ni `evidence/guards.rs`, ni `skills/builtin_handlers.rs`.
- `MIKA_MANAGER_GROOM_WRITE` reste OFF par défaut. Ce vol ne l'ouvre pas.

## Acceptance criteria

Transcrits depuis le corps de senara-solutions/mika#2958.

- **AC1.** Avec l'interrupteur OFF, aucun groom n'est déclenché. C'est une garantie
  structurelle, avec un contrôle négatif dans le même test. Le geste entre comme **exception
  nommée** au scan de `no_dispatch_test.rs`, sur le patron des entrées de mika#2698 vol 2 et de
  mika#2699 vol 4. Le scan n'est pas assoupli. Le test
  `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` est amendé dans le même commit, et la PR
  dit ce qui le faisait rougir.
- **AC2.** Sélection et idempotence : un ticket déjà groomé, ou dont un groom est en cours,
  n'est pas relancé. Le plan dit par quelle lecture le manager voit un groom en cours : les
  tâches moteur `Groom …` et `ready-label`, ou autre chose. Le motif est la collision d'un
  groom manuel avec un groom moteur.
- **AC3.** La voie est celle du moteur : la même que `mika ask --agent mika-dev "groom mika#N"`,
  ou son équivalent interne. Elle n'ouvre ni spawn ni dispatch d'implémentation. Le plan dit
  comment le geste reste borné : combien de grooms par cycle, et ce qu'il fait quand mika-dev a
  déjà un pilote en vol, puisque le moteur n'en fait tourner qu'un à la fois.
- **AC4.** Ce qui reste de l'ombre se lit dans le rapport. Interrupteur OFF : la section
  « Relève à grommer » est inchangée. Interrupteur ON, en test seulement : le rapport dit quels
  grooms ont été déclenchés, et lesquels ont été sautés, avec leur motif.
- **Taille** : à estimer tests compris, et à découper en vols au-delà de la borne.

### Couverture par ce vol

| critère | ce vol | reste dû |
|---|---|---|
| AC1 | **clos**, dans sa forme forte : production câblée, silence prouvé, contrôle négatif, allowlist inchangée à 3 | — |
| AC2 | moitié **sélection** : `deja_groome` dérivé du motif à son site unique, `groom_en_cours` avec sa dérivation écrite | moitié **« non relancé »** durable — vol 2c |
| AC3 | **clos** pour la voie (A2A `message/send` vers mika-dev, ni spawn ni dispatch d'implémentation) et pour le bornage (`GROOMS_PER_CYCLE = 1`, arrêt sur `agent_busy`) | — |
| AC4 | moitié **OFF** : la section « Relève à grommer » inchangée par construction | moitié **ON** du rapport — vol 2d |
| Taille | estimée tests compris, découpée en vols 2b/2c/2d | — |

## Hors périmètre

- **La fenêtre d'idempotence durable** — champ de `MilestoneCheckpoint`, `merge_posted`
  généralisée en `merge_retained<T: Clone>`, et la lecture container-scoped des tâches moteur
  `Groom …` / `ready-label` sur le patron de `list_finished_implement_flights_in_container`.
  C'est la moitié « non relancé » d'AC2, et la précondition déclarée à l'ouverture de
  l'interrupteur. *Tracked in: suivi vol 2c de mika#2958.*
- **Le rendu ON du rapport** — la moitié ON d'AC4. *Tracked in: suivi vol 2d de mika#2958.*
- **La résolution du budget par attestation** (mika#2759) plutôt que par le défaut de flotte,
  qui demanderait un lecteur d'attestation en loopback dans le manager. Cf. D1.
  *Tracked in: suivi à ouvrir si la sonde de D1 parle.*
- **Une garde de forme sur l'URL du rail** (`/a2a/{nom}` à segment unique). Écartée avec sa
  raison en R1 : ≈ 45 lignes contre la borne pour un défaut déjà visible par deux sondes.
- **L'ouverture de `MIKA_MANAGER_GROOM_WRITE`.** Geste de l'opérateur, au seuil écrit dans
  mika#2700 : évaluation des vols et bassin ready ouverts **et** stables sur un cycle complet.
  Les deux sont OFF aujourd'hui. Seul Vincent l'ouvre.
- **Tout changement à `relief.rs`, `pool.rs`, `assessor.rs`, `types.rs`, `reader.rs`.** Ce vol
  les **lit**.

## Sonde post-déploiement

Après `make deploy`, interrupteur **toujours OFF** — ce qui se vérifie est le silence et
l'observabilité, pas le geste :

1. **Le rail se résout et le dit.**
   `grep manager_groom_rail_resolved ~/.mika/logs/mika-spirit.log | tail -1`
   ⇒ une ligne par démarrage, avec `url_set` / `token_set` / `budget_secs: 600`. Son absence
   signifie un binaire plus ancien, **jamais** un rail à zéro (mika#2340).
2. **Le cycle refuse, et nomme son motif.**
   `grep manager_groom_not_triggered ~/.mika/logs/mika-spirit.log | tail -3`
   ⇒ `reason: "disarmed"` à chaque cycle, interrupteur fermé. Un `no_sender` dit que
   `MIKA_MANAGER_GROOM_URL` n'est pas posée ; un `no_relief` dit que la relève est vide.
3. **Rien n'est parti.**
   `grep -c manager_groom_triggered ~/.mika/logs/mika-spirit.log` ⇒ `0`.
4. **Le rapport n'a pas bougé.** La section « Relève à grommer (mode ombre — aucun groom
   déclenché) » du rapport de cycle est identique à celle d'avant le déploiement.
5. **STOP.** Retirer `MIKA_MANAGER_GROOM_URL` de l'environnement et redémarrer désarme le rail
   sans toucher au code : le cycle retombe sur `reason: "no_sender"`. Le geste reste par
   ailleurs doublement fermé par `MIKA_MANAGER_GROOM_WRITE`, qui a son propre arrêt immédiat
   avant même que l'expéditeur soit consulté.
