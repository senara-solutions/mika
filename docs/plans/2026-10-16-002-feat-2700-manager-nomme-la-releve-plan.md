# Le manager nomme la relève — vol 1 (mika#2700, Phase 2 (c))

Ticket : senara-solutions/mika#2700 — *mika-manager Phase 2 (c) : nommer la relève
et grommer d'avance, en ombre puis derrière un interrupteur.*
Label porteur du plafond : `loop-substrate`.

## Contexte

Troisième ticket de la Phase 2 du manager, dans l'ordre **b → a → c** :

| ticket | ce qu'il a livré | état |
|---|---|---|
| (b) mika#2698 | évaluation des vols terminés, `MIKA_MANAGER_EVALUATE_WRITE` | vols 1 **et** 2 atterris (`4cabcddd`, `aba4dec8`) |
| (a) mika#2699 | le bassin `ready` comme verdict de lecture | vols 1 **et** 2 atterris (`5798e507`) — vol 3 (`MIKA_MANAGER_POOL_WRITE`) **non livré** |
| (c) mika#2700 | **ce plan** | — |

Le manager reste **LECTURE SEULE** par charte, et le scan structurel de
`no_dispatch_test.rs` (22 tokens interdits, dont `gh issue edit`, `"POST"`,
`run_claude_pilot`) le tient par construction. Toute écriture de Phase 2 arrive
comme **exception nommée** à ce scan, jamais comme un relâchement — et le scan et
le docstring de `mod.rs` sont amendés dans le même commit que le geste.

Le ticket demande trois choses :

1. **Relève nommée** — à chaque cycle, les 4 prochains tickets à grommer d'avance
   derrière le bassin `ready`, dans l'ordre de tri du bassin, avec le motif de chacun.
2. **Groom d'avance** — derrière `MIKA_MANAGER_GROOM_WRITE`, OFF par défaut, par la
   voie moteur existante (le groom passe par mika-dev, jamais par un spawn). La consigne
   de groom compte les tests dans l'estimation de taille.
3. **Mode ombre** — interrupteur OFF, la relève n'apparaît que dans le rapport.

Les points **1 et 3 sont le vol 1** et font ce plan. Le point 2 est le vol 2, nommé
par une ligne `Tracked in:` au § *Découpage en vols*.

**Le seuil d'ouverture du ticket est un geste opérateur, pas un livrable de code.**
Il demande que les interrupteurs de l'évaluation des vols et du bassin `ready`
soient ouverts et stables sur un cycle complet. Or `MIKA_MANAGER_POOL_WRITE` —
l'interrupteur du bassin — est le **vol 3 de mika#2699, qui n'est pas livré** : il
n'existe pas, donc il ne peut pas être « ouvert et stable ». Ce seuil borne donc
l'ouverture du **vol 2** de ce ticket, et le vol 1 ne le franchit pas : il n'écrit
rien, et son interrupteur n'existe pas encore. Dit ici plutôt que découvert au vol 2.

## Ce que le code offre déjà — relevé, parce qu'il change le périmètre

`pool.rs` (827 lignes, mika#2699 vols 1+2) tient déjà **toute la sélection** dont la
relève a besoin. `assess_pool(pool, milestone_candidates, outside_candidates)` filtre
dans cet ordre :

1. dédoublonnage par numéro (l'ensemble milestone d'abord, donc un ticket du milestone
   n'est jamais étiqueté `Outside`) ;
2. rejet des non-`Open` ;
3. rejet des déjà-`ready` (union du bassin dépôt-large et des labels du candidat) ;
4. rejet des **en vol** — prédicat sur la famille de labels `dispatch:` ;
5. `!plan_present` ⇒ `AdmissionRefusal { reason: NoGroomedPlan }` ;
6. sinon éligible.

Puis `eligible` est trié par `(priority_rank.unwrap_or(u8::MAX), origin.rank(), number)`
— **la priorité domine la provenance** — et `refusals` par `(origin.rank(), number)`.

Trois conséquences qui décident le vol 1 :

- **La relève est exactement l'ensemble des refus `NoGroomedPlan`.** Un ticket « à
  grommer d'avance » est un candidat du bassin (ouvert, pas `ready`, pas en vol) à qui
  il manque un plan groomé. C'est la définition du refus, mot pour mot. Rien à
  re-filtrer : une seconde copie des cinq filtres serait un second lecteur de la même
  question (doctrine mika#2201) et divergerait au premier changement.
- **AC2 est déjà structurellement acquis par ces filtres.** « Déjà groomé » ⇔
  `plan_present == true` ⇒ le ticket est candidat à l'admission, jamais relève.
  « En cours de groom » ⇔ un label de la famille `dispatch:` ⇒ écarté au filtre 4,
  **sans émettre de refus** ⇒ jamais relève. Les deux sont lisibles sur la forge, sans
  DB et sans appel `gh` supplémentaire. Le vol 1 n'a donc à livrer aucun mécanisme pour
  AC2 : il a à livrer les **tests** qui le prouvent, et c'est ce que demande le ticket.
- **`AdmissionRefusal` ne porte pas `priority_rank`, et les refus ne sont pas triés par
  priorité.** C'est délibéré côté bassin (un refus s'arbitre par provenance). Mais la
  relève, elle, est demandée « dans l'ordre de tri du bassin » — qui est l'ordre à
  priorité d'abord. C'est la seule vraie difficulté de conception du vol, et la
  décision 2 ci-dessous la nomme.

Le reste est du câblage déjà tracé par les deux tickets frères :

- `types.rs::Assessment` porte déjà `pool: Option<PoolVerdict>` et
  `flight_verdicts: Vec<FlightVerdict>`, tous deux `#[serde(default)]` pour que le puits
  hors-ligne relise des blobs antérieurs au champ.
- `assessor.rs` appelle `pool::assess_pool` à la composition de l'`Assessment` et pose
  le résultat sans rien décider — la relève s'y greffe au même endroit, de la même façon.
- `reporter.rs` rend `### Bassin ready (mode ombre — aucune écriture)` avec
  `candidate_suffix(state, number, origin)`, qui rend déjà le **nom du label** de
  priorité et la mention `hors milestone`, cherchés dans les deux ensembles de candidats.
  La relève réutilise ce helper tel quel.
- `MilestoneState` porte `ready_pool`, `outside_candidates` et `outside_read_truncated:
  Option<bool>` — `Some(true)` signifie « page pleine, donc `outside_candidates` est vide
  **par décision** ».
- **Aucune lecture `gh` nouvelle n'est nécessaire.** Les cinq lectures du cycle suffisent.
  C'est ce qui rend ce vol petit.
- `state_digest` (`cadence.rs`) ne hache que `sub_issues` : la relève, dérivée, est hors
  détection de changement **par construction**. Un fait à épingler, pas à corriger — un
  cycle ne doit pas être réputé « changé » parce qu'un plan est apparu ailleurs dans le dépôt.

## Découpage en vols

| vol | contenu | AC couverts |
|---|---|---|
| **1 (ce plan)** | la relève nommée, calculée des refus du bassin, dans l'ordre de tri du bassin, avec son motif ; rendue dans le rapport ; **mode ombre entier** — aucun chemin d'écriture n'existe | AC1 sous sa forme dégénérée (aucun déclenchement **possible**), AC2 (sélection + idempotence), Taille |
| 2 | `MIKA_MANAGER_GROOM_WRITE` + déclenchement du groom par la voie moteur (mika-dev), consigne comptant les tests, idempotence par checkpoint, **exception nommée** au scan de `no_dispatch_test.rs` avec contrôle négatif, amendement atomique du docstring de `mod.rs` | AC1 sous sa forme armée (contrôle négatif interrupteur ON/OFF) |

Tracked in: senara-solutions/mika#2700 (vol 2 — `MIKA_MANAGER_GROOM_WRITE` et le
déclenchement par la voie moteur, à ouvrir après le seuil opérateur du § *Contexte*).

**Pourquoi le vol 1 s'arrête là.** Le vol 2 ne demande pas une ligne de plus dans une
fonction existante : il demande un **port injecté** — un `Arc<dyn GroomDispatcher>` passé
par `run_server` à `spawn_manager_cycle_task`, sur le précédent exact de
`FlightSource` / `LivenessSink` / `AuthBoundaryLedger` — parce que « la voie moteur
existante » est une requête à mika-dev (l'équivalent HTTP de
`mika ask --agent mika-dev "groom mika issue#N"`, soit `POST /message` derrière le
jeton interne), et que le manager n'a pas d'agent à lui. Il demande aussi un champ de
checkpoint pour l'idempotence, parce que le label `dispatch:loop` n'est posé par
`_stamp_issue_seat` qu'à la mise en place du worktree : entre la requête et le label
visible il existe une fenêtre de latence face au sondage par défaut de 300 s, et deux
cycles dans cette fenêtre relanceraient le même groom. Et il demande l'exception nommée
au scan avec son contrôle négatif. Agréger ça au vol 1 mettrait l'estimation au-delà de
la borne par vol, et surtout : **le vol 1 est mergeable et utile seul** — la relève dans
le rapport est précisément ce que le point 3 du ticket appelle le mode ombre, et c'est
ce qui permet à l'opérateur de juger la sélection avant d'armer quoi que ce soit.

Le vol 1 porte son propre ordre de commit : `relief.rs` (pur, avec ses tests) →
`pool.rs` (extraction de la clé de tri) → `types.rs` → `assessor.rs` → `reporter.rs` →
`mod.rs`. Son plancher : la suite `milestone_manager` verte, `no_dispatch_test.rs`
**intouché** et vert.

## Conception — vol 1

### `crates/mika-agent/src/milestone_manager/relief.rs` (nouveau)

Module **pur** : aucun `async`, aucun I/O, aucun accès à l'environnement. Il répond à
une seule question — *quels sont les 4 prochains tickets à grommer d'avance, et
pourquoi ?*

```rust
/// Combien de tickets la relève nomme par cycle.
///
/// 4 par le ticket. Pas dérivé de `POOL_MAX` : ce sont deux grandeurs qui se
/// ressemblent aujourd'hui et ne répondent pas à la même question — l'une est
/// la cible du bassin, l'autre la profondeur de la file de grooming.
pub const RELIEF_SIZE: usize = 4;

/// Un ticket de la relève, avec son motif.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReliefTicket {
    pub number: u64,
    pub origin: CandidateOrigin,
    /// Le rang de priorité qui a décidé sa place. `None` = aucun label de
    /// priorité (trié en dernier, jamais confondu avec le rang 0).
    pub priority_rank: Option<u8>,
    /// Pourquoi ce ticket est relève — le motif du ticket, point 1.
    pub motive: RefusalReason,
}

/// La relève d'un cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReliefVerdict {
    /// Au plus [`RELIEF_SIZE`], dans l'ordre de tri du bassin.
    pub next: Vec<ReliefTicket>,
    /// Combien de candidats à la relève ont été écartés par la troncature à
    /// [`RELIEF_SIZE`]. Dit, pas tu : « 4 » et « 4 sur 31 » n'appellent pas le
    /// même geste.
    pub beyond: usize,
}

/// Nomme la relève à partir du verdict du bassin.
///
/// Les deux tranches de candidats sont celles qui ont été passées à
/// [`pool::assess_pool`] : elles servent à retrouver le `priority_rank` d'un
/// refus, que [`pool::AdmissionRefusal`] ne porte pas.
pub fn name_relief(
    verdict: &PoolVerdict,
    milestone_candidates: &[PoolCandidate],
    outside_candidates: &[PoolCandidate],
) -> ReliefVerdict
```

**Décision 1 — la relève est dérivée des refus, elle ne re-filtre rien.** `name_relief`
lit `verdict.refusals` et rien d'autre côté sélection. Les cinq filtres du bassin restent
dans `assess_pool`, lecteur unique de « qui est candidat ». Conséquence voulue : le jour
où un filtre change (un sixième état, un autre label de siège), la relève suit sans
qu'on y touche, et elle ne peut pas diverger du bassin.

**Décision 2 — la clé de tri est extraite dans `pool.rs`, pas réécrite ici.** `pool.rs`
expose `pub fn pool_sort_key(priority_rank: Option<u8>, origin: CandidateOrigin, number:
u64) -> (u8, u8, u64)`, et `assess_pool` l'utilise pour son propre tri d'éligibles.
`name_relief` l'appelle pour le sien. « Dans l'ordre de tri du bassin » devient alors
vrai *par construction* au lieu d'être vrai par coïncidence entre deux expressions de
tri recopiées. Alternative écartée : élargir `AdmissionRefusal` d'un `priority_rank` —
cela aurait changé un type déjà sérialisé dans les blobs du puits hors-ligne, et aurait
suggéré que les refus se trient par priorité, ce qui est faux côté bassin et délibéré.

**Décision 3 — un refus dont le numéro est introuvable prend `priority_rank: None`, il
n'est pas écarté.** Par construction les deux tranches sont exactement l'entrée
d'`assess_pool`, donc chaque numéro y est. Mais une fonction pure ne doit pas dépendre
de la prudence de son appelant — c'est ce que dit déjà le docstring d'`assess_pool` à
propos du dédoublonnage. Un appelant distrait obtient une relève **complète et mal
ordonnée** (le ticket passe en queue), pas une relève **amputée** : perdre un ticket est
la panne qu'on ne verrait pas. Épinglé par un test.

**Décision 4 — le motif réutilise `RefusalReason`, pas un enum jumeau.** Le motif d'une
relève *est* la raison du refus du bassin. Réutiliser le type garantit que la relève ne
peut jamais porter un motif que le bassin ne reconnaît pas, et évite une table de
correspondance entre deux enums qui dériveraient. Le libellé côté relève est affaire de
**rendu** : `RefusalReason::NoGroomedPlan::label()` dit « plan groomé absent », ce qui est
la formulation du refus ; le rapport de la relève écrit « à grommer — plan absent ».
Deux phrases pour un fait, chacune juste à sa place, et une seule donnée.

**Décision 5 — `beyond` est un compte, pas un booléen.** Même motif que
`outside_read_truncated` : une mesure plutôt qu'un drapeau, parce que « 27 autres en
attente » et « 1 autre » n'appellent pas le même arbitrage.

### `crates/mika-agent/src/milestone_manager/pool.rs`

Une seule modification : extraire `pool_sort_key` (décision 2), la documenter comme
**site unique de l'ordre du bassin**, et l'utiliser au tri des éligibles. Le
comportement d'`assess_pool` est inchangé — ses tests existants sont le contrôle.

### `crates/mika-agent/src/milestone_manager/types.rs`

```rust
    /// La relève nommée de ce cycle (mika#2700 Phase 2 (c) vol 1).
    ///
    /// `Option` et pas un `ReliefVerdict` nu : `None` dit « la relève n'a pas
    /// été nommée » — blob écrit par un binaire antérieur au champ. Un verdict
    /// par défaut dirait « aucun ticket à grommer d'avance », ce qui est une
    /// *mesure*, pas une absence de mesure (motif mika#2331 : un null n'est
    /// jamais un 0).
    ///
    /// **Vol 1 : le rapport est la seule destination de ce champ.** Aucun
    /// chemin d'écriture ne part de lui.
    #[serde(default)]
    pub relief: Option<relief::ReliefVerdict>,
```

### `crates/mika-agent/src/milestone_manager/assessor.rs`

Le verdict du bassin est déjà calculé dans le littéral `Assessment`. La relève en
dépend, donc il est sorti en `let` juste avant, et les deux projections de candidats
aussi — pour que le bassin et la relève voient **exactement** les mêmes entrées. Un
calcul des candidats fait deux fois serait la porte ouverte à deux réponses.

```rust
let milestone_candidates = milestone_pool_candidates(state);
let pool_verdict = pool::assess_pool(
    &state.ready_pool,
    &milestone_candidates,
    &state.outside_candidates,
);
let relief = relief::name_relief(
    &pool_verdict,
    &milestone_candidates,
    &state.outside_candidates,
);
```

L'assesseur ne décide rien : il passe les lectures et pose les résultats, comme pour le
bassin.

### `crates/mika-agent/src/milestone_manager/reporter.rs`

Une section conditionnelle, après `### Bassin ready`, parce que la relève se lit
*derrière* le bassin — c'est l'ordre du ticket et l'ordre de la décision.

```
### Relève à grommer (mode ombre — aucun groom déclenché)
- #2731 (p1) — à grommer, plan absent
- #2726 (agent-core) — à grommer, plan absent
- #2740 (hors milestone) — à grommer, plan absent
- #2744 — à grommer, plan absent
- 7 autres candidats au-delà des 4 nommés
- Lecture hors milestone tronquée à 100 issues — relève limitée au milestone cible
```

Quatre propriétés du rendu, chacune pour une raison :

- **« mode ombre — aucun groom déclenché » est dans le TITRE**, pas en note. Même motif
  que `### Bassin ready` : une liste de tickets « à grommer » se lit comme un geste fait
  si rien ne dit le contraire à l'instant où l'œil tombe dessus.
- **La section ne s'affiche pas quand `relief` est `None`** (blob antérieur au champ) ni
  quand `next` est vide : une section vide affirmerait une mesure qui n'a pas eu lieu, ou
  inviterait à chercher une cause là où il n'y a rien à grommer.
- **Le suffixe réutilise `candidate_suffix`** : priorité par son **nom de label** (pas
  son rang — `agent-core` vaut le rang 2, l'écrire « p2 » serait faux) et mention
  `hors milestone`. Zéro duplication.
- **La troncature se dit dans la section de la relève aussi.** Sur
  `outside_read_truncated == Some(true)`, `outside_candidates` est vide par décision :
  la relève est alors celle du **milestone cible**, pas celle du dépôt. Un opérateur qui
  lit quatre numéros sans cette ligne lirait « les 4 prochains du dépôt ». La condition
  est sur `Some(true)` seul — `None` veut dire « pas mesuré » et ne justifie aucune ligne.

### `crates/mika-agent/src/milestone_manager/mod.rs`

`pub mod relief;` et un paragraphe de docstring : ce que le vol 1 livre, et la phrase qui
engage la suite — *le geste d'écriture est le vol 2, derrière `MIKA_MANAGER_GROOM_WRITE`
(OFF par défaut), et il arrivera comme **exception nommée** au scan de
`no_dispatch_test.rs`, amendée dans le même commit que ce docstring.* La charte LECTURE
SEULE n'est pas touchée : le vol 1 ne l'entame pas.

## Ce que le vol 1 ne livre pas, et pourquoi c'est dit ici

- **Aucun interrupteur.** `MIKA_MANAGER_GROOM_WRITE` n'est pas déclaré au vol 1. Un
  `pub const ENV_*` sans lecteur serait une promesse d'armement non tenue, et il
  déclencherait `mika2267_every_manager_env_const_is_declared_in_env_example` (le test T7
  scanne tout le répertoire) sans qu'aucun comportement ne le justifie. Il arrive au vol
  2, avec son `write_is_armed()`, sa table de vérité pure
  (`armed_from_value(Option<&str>)` : absent / vide / faute de frappe = OFF ; seuls `1`
  et `true`, insensibles à la casse et aux espaces, = ON), sa déclaration dans
  `.env.example` et son `armed` passé en **paramètre** — jamais relu de
  l'environnement — pour que la branche OFF se prouve par « zéro appel » et non par
  l'absence d'un chemin.
- **Aucun port de déclenchement, aucun geste.** Pas de `GroomDispatcher`, pas de
  constante de geste plat, rien dans `run_server`.
- **Aucune modification de `no_dispatch_test.rs`.** Il reste intouché et vert, et c'est
  précisément la preuve d'AC1 pour ce vol.
- **Aucune lecture `gh` nouvelle, aucun champ de checkpoint, aucun changement de
  `state_digest`.** La relève est dérivée des lectures du cycle.
- **La consigne de groom** (« la taille s'estime tests compris ») n'est pas rédigée au
  vol 1 : elle n'a pas de destinataire tant qu'il n'y a pas de requête. Elle est au vol 2,
  dans le corps du message envoyé à mika-dev.

## Contrat de vérification

Tous les tests sont **en ligne**, en `#[cfg(test)] mod tests`, dans le fichier qu'ils
testent. Porte 1 l'exige : tout ce qui vit sous `crates/mika-agent/src/milestone_manager/**`
doit classer `DecisionCore`, et `MECHANICAL_CONTAINS` accorde MECHANICAL à tout chemin
contenant `/tests/` — un répertoire `tests/` imbriqué y est interdit, et
`milestone_manager_has_no_nested_tests_directory` le tient.

**`relief.rs`** — la sélection et son ordre :

1. `mika2700_la_releve_est_exactement_les_refus_du_bassin` — un ensemble de candidats
   mêlant admissibles, refusés, fermés, déjà-`ready` et en vol : la relève contient les
   refusés, et personne d'autre.
2. `mika2700_la_releve_suit_lordre_de_tri_du_bassin` — priorité d'abord, puis provenance,
   puis numéro. Le cas discriminant est un ticket **hors milestone** à priorité **haute**
   face à un ticket **du milestone** sans priorité : le premier passe devant. C'est ce qui
   distingue « l'ordre du bassin » d'un tri par provenance, et c'est l'ordre dans lequel
   `AdmissionRefusal` n'arrive **pas** trié.
3. `mika2700_un_ticket_deja_groome_nest_pas_releve` (AC2) — `plan_present: true` : le
   ticket est admissible, jamais relève.
4. `mika2700_un_ticket_en_cours_de_groom_nest_pas_releve` (AC2) — un label
   `dispatch:loop` : écarté par le bassin sans refus, donc absent de la relève. Le
   contrôle négatif du même test retire le label et voit le ticket apparaître — c'est la
   paire qui sépare « l'idempotence tient » de « le ticket n'était pas candidat de toute façon ».
5. `mika2700_la_releve_est_bornee_a_quatre_et_dit_le_reste` — sept refus : `next` en porte
   4, `beyond == 3`.
6. `mika2700_un_refus_sans_candidat_retrouve_passe_en_queue_sans_disparaitre`
   (décision 3) — tranches de candidats vides : la relève est complète, les rangs sont
   `None`, l'ordre dégrade sur le numéro.
7. `mika2700_un_bassin_sans_refus_ne_nomme_personne` — `next` vide, `beyond == 0`.
8. `mika2700_le_motif_est_celui_du_bassin` (décision 4) — le `motive` de chaque entrée est
   le `reason` du refus correspondant, par égalité de valeur.

**`pool.rs`** — l'extraction :

9. `mika2700_la_cle_de_tri_du_bassin_a_un_site_unique` — `pool_sort_key` ordonne priorité
   avant provenance avant numéro, et `None` trie après tout rang nommé (jamais comme 0).
   Les tests existants d'`assess_pool` sont le contrôle de non-régression du tri.

**`types.rs`** :

10. `mika2700_un_blob_sans_releve_se_relit_en_none` — un JSON d'`Assessment` sans la clé
    `relief` se désérialise, et `relief` vaut `None` — pas un verdict vide. Le puits
    hors-ligne tient de tels blobs.

**`assessor.rs`** :

11. `mika2700_lassesseur_pose_la_releve_du_meme_bassin_quil_a_pose` — un état avec un
    candidat sans plan : `assessment.relief` est `Some`, et sa liste correspond aux refus
    de `assessment.pool`. C'est le test qui attrape un assesseur ayant calculé les
    candidats deux fois.

**`reporter.rs`** :

12. `mika2700_la_section_releve_nomme_chaque_ticket_avec_son_motif` — un ticket par ligne,
    avec son numéro, son suffixe de priorité, sa mention `hors milestone` le cas échéant,
    et son motif.
13. `mika2700_sans_releve_la_section_est_absente` — `relief: None` **et** `next` vide :
    aucune des deux ne rend de titre. Un seul test, deux assertions, parce que c'est une
    seule propriété.
14. `mika2700_la_releve_se_lit_derriere_le_bassin_et_porte_le_mode_ombre_dans_son_titre` —
    position relative des deux titres par `find`, et présence de « aucun groom déclenché »
    dans le titre de la relève.
15. `mika2700_une_lecture_hors_milestone_tronquee_borne_la_releve_au_milestone` — sur
    `Some(true)` la ligne de caveat est là ; sur `None` elle est absente.

**`mod.rs`** :

16. `mika2700_le_vol_1_nouvre_aucun_chemin_decriture` (AC1, forme dégénérée) — la preuve
    est le scan existant de `no_dispatch_test.rs`, qui couvre tout le répertoire et donc
    `relief.rs` par construction. Ce test n'ajoute pas un second scanner — ce serait un
    second lecteur de la même question (mika#2201) : il **assert que l'allowlist
    d'exceptions du scan est vide**, ce qui est l'énoncé « aucune exception nommée n'existe
    encore » et ce qui rougira au vol 2 au bon endroit, dans le commit qui amende le scan.

**Inchangés et devant rester verts** — c'est la moitié du contrat :

- `no_dispatch_test.rs` **en entier**, intouché (AC1).
- `milestone_manager_has_no_nested_tests_directory` (Porte 1).
- Toute la suite d'`assess_pool` dans `pool.rs` (contrôle du tri extrait).
- `mika2267_every_manager_env_const_is_declared_in_env_example` (T7) — vert sans
  modification, puisque le vol 1 ne déclare aucune variable.
- Les tests de `state_digest` dans `cadence.rs` — la relève est hors détection de
  changement, et doit le rester.
- Les tests du puits hors-ligne et de `reporter.rs` antérieurs.

**Lignes de commande :**

```
cargo test -p mika-agent milestone_manager
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/verify-pipeline.sh origin/main
```

## Fire-Disposition

**N/A pour ce vol.** Le vol 1 ne livre aucun détecteur : `relief.rs` est une fonction de
sélection, le rendu du rapport est de l'affichage, et les seize tests ci-dessus sont des
tests de comportement — leur chemin de succès est « la relève est celle attendue », pas
« aucune violation trouvée ». Le seul détecteur en jeu, `no_dispatch_test.rs`, **existe
déjà, est armé, et n'est pas touché** : le vol s'inscrit dans son périmètre au lieu de le
modifier. La section n'est donc pas inventée (mika#2306 : si le plan ne livre aucun
détecteur, la section n'est pas requise — et un gate N/A rempli au hasard est un faux
engagement).

**Le vol 2 en livrera un**, et son plan portera la section en **option (a)** : l'exception
nommée en allowlist du scan de `no_dispatch_test.rs` pour le geste de requête vers
mika-dev, avec une entrée grep-visible nommant le token précis, la référence à ce ticket,
et l'assertion auto-nettoyante qui rougit quand l'exception devient stale.

## Definition of Done

- [ ] `relief.rs` livré, pur, avec ses huit tests en ligne.
- [ ] `pool_sort_key` extrait dans `pool.rs`, site unique de l'ordre du bassin, et
      `assess_pool` l'utilise sans changer de comportement.
- [ ] `Assessment::relief: Option<ReliefVerdict>` avec `#[serde(default)]` et son test de
      relecture de blob ancien.
- [ ] `assessor.rs` calcule les candidats **une fois** et pose bassin et relève sur la
      même entrée.
- [ ] `reporter.rs` rend `### Relève à grommer (mode ombre — aucun groom déclenché)`
      derrière la section du bassin, un ticket par ligne avec son motif, le reste compté,
      la troncature dite.
- [ ] `mod.rs` déclare le module et porte le paragraphe qui engage le vol 2.
- [ ] `no_dispatch_test.rs` intouché et vert ; allowlist d'exceptions toujours vide,
      asserté.
- [ ] Aucune variable d'environnement nouvelle, aucun port, aucun geste, aucune lecture
      `gh` nouvelle.
- [ ] `cargo test -p mika-agent milestone_manager`, `cargo clippy --all-targets -- -D
      warnings`, `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` verts.

## Acceptance criteria

- [ ] **AC1 : avec l'interrupteur OFF, aucun déclenchement de groom (garantie structurelle
      avec contrôle négatif).** *Vol 1 — forme dégénérée et la plus forte : il n'y a pas
      d'interrupteur, donc pas de branche ON, donc aucun déclenchement n'est possible. La
      garantie structurelle est le scan de `no_dispatch_test.rs`, qui couvre tout le
      répertoire et reste intouché et vert, plus l'assertion que son allowlist d'exceptions
      est vide (test 16). Vol 2 — forme armée : table de vérité de l'interrupteur, `armed`
      en paramètre, « interrupteur OFF ⇒ zéro appel » et son contrôle négatif
      « interrupteur ON ⇒ un seul appel, et c'est le bon geste ».*
- [ ] **AC2 : tests de la sélection et de l'idempotence : un ticket déjà groomé ou en cours
      de groom n'est pas relancé.** *Vol 1 — sélection : tests 1, 2, 5, 6, 7, 8.
      Idempotence : test 3 (déjà groomé ⇔ `plan_present`, donc candidat à l'admission et
      jamais relève) et test 4 avec son contrôle négatif (en cours de groom ⇔ label de la
      famille `dispatch:`, écarté par le bassin sans refus). Les deux faits sont lisibles
      sur la forge, sans DB ni appel supplémentaire. Vol 2 — l'idempotence du geste
      lui-même, par champ de checkpoint, contre la fenêtre de latence entre la requête et
      la pose de `dispatch:loop`.*
- [ ] **Taille : à estimer tests compris, et à découper en vols au-delà de la borne.**
      *Estimation tests compris au § suivant ; découpage en deux vols au § Découpage en
      vols, suite nommée par `Tracked in:`, vol courant sous la borne.*

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/relief.rs` (production) | 110 |
| tests en ligne de `relief.rs` (tests 1–8) | 105 |
| `crates/mika-agent/src/milestone_manager/pool.rs` (extraction + test 9) | 25 |
| `crates/mika-agent/src/milestone_manager/types.rs` (champ + test 10) | 25 |
| `crates/mika-agent/src/milestone_manager/assessor.rs` (câblage + test 11) | 30 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (section + tests 12–15) | 75 |
| `crates/mika-agent/src/milestone_manager/mod.rs` (déclaration, docstring, test 16) | 20 |

Total estimé : 390 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

**Pourquoi un seul vol tient ici.** 390 < 431, avec 41 lignes de marge, et trois raisons
structurelles rendent ce vol peu susceptible de déborder : il n'ouvre **aucune** lecture
`gh` (le poste le plus coûteux de mika#2699 vol 1 était `reader.rs` à +433 lignes, et ce
vol ne touche pas `reader.rs`) ; il ne touche ni `cadence.rs` ni `run_server`, donc aucun
port à injecter et aucun câblage à six sites ; et son seul fichier neuf est une fonction
pure dont l'entrée est déjà calculée par le code en place.

**Marge et calibration, dites plutôt que tues.** Le plan frère mika#2699 a estimé
422 lignes et ses vols 1+2 ont atterri en 2 304 insertions — soit environ 1 935 lignes de
code hors plan, un facteur ~2,3 sur deux vols. Le facteur vient pour l'essentiel de la
densité de docstrings que la Porte 1 `DecisionCore` impose à ce répertoire : chaque
décision nommée se paie en prose dans le `.rs`. Ce plan estime dans la **même convention**
que son frère — celle que l'architecte a jugée — pour que les deux nombres restent
comparables, et choisit un périmètre franchement plus petit que celui de mika#2699 vol 1
(un fichier neuf pur et cinq retouches chirurgicales, contre un module de 827 lignes plus
une lecture `gh` nouvelle) pour que l'écart de convention ne mange pas la marge. Si le
compte réel dépasse la borne en cours de vol, la coupe est nommée d'avance : **le rendu du
rapport (`reporter.rs`, tests 12–15) part au vol 2**, et le vol 1 se réduit à la relève
calculée et portée dans l'`Assessment` — elle reste alors visible par le puits hors-ligne,
ce qui tient encore le point 3 du ticket sous une forme plus pauvre.
