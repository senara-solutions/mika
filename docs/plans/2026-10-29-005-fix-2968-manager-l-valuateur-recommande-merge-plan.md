# fix(manager) — la recommandation de vol lit la CI de la tête, et le rapport dit laquelle

**Ticket :** senara-solutions/mika#2968 (`bug`, `p1-important`, `dispatch:loop`, `loop-substrate`)
**Branche :** `fix/2968/manager-l-valuateur-recommande-merge`
**Base mesurée :** `origin/main` à `7df2c1e1c`
**Vol :** 1 sur 2 — ce plan livre la **décision** et sa **lecture dans le rapport** ; le lecteur `gh` par numéro de PR est le vol 2 (§ 9, `Tracked in:`)
**Lié :** mika#2698 (l'évaluateur, Phase 2 b), mika#2921 (`PrStateKnowledge`, R7), mika#2924 (`PrLinkSource`), mika#2963 et mika#2966 (le vol mesuré), mika#2675 (le pré-filtre qui écarte l'échec CI), mika#2205 (deux populations rendant les mêmes octets), mika#2331 (`None` n'est pas zéro), mika#2340 (l'absence d'un signal dit un binaire antérieur), mika#2158 (divergence programmée de deux vocabulaires), mika#2201 (un seul lecteur par prédicat)

---

## 1. Ce qui est mesuré

### 1.1 La recommandation ne consulte pas la CI

`evaluator.rs:363`, la fonction entière :

```rust
fn recommend(issue: &FlightIssue, pr_state: &PrStateKnowledge) -> FlightRecommendation {
    match issue {
        FlightIssue::PrOpened { .. } => match pr_state {
            PrStateKnowledge::Read(Some(s)) if !s.eq_ignore_ascii_case("open") => {
                FlightRecommendation::ManualResumption
            }
            _ => FlightRecommendation::MergeCandidate,
        },
        ...
```

Deux entrées : l'issue et l'état de la PR. Pas d'état CI. Pour `PrOpened`, toute PR qui n'est pas explicitement `closed`/`merged` rend `MergeCandidate` — y compris une PR dont un check requis est rouge, et y compris une PR dont l'état n'a jamais été lu (`PrStateKnowledge::NotRead` tombe dans le bras `_`).

### 1.2 L'appariement jette `ci_state`

`evaluator.rs:301` :

```rust
pub fn evaluate_flights(flights: &[FinishedFlight], sub_issues: &[SubIssue]) -> Vec<FlightVerdict> {
    flights.iter().map(|f| {
        let issue_number = issue_number_from_url(f.reference_url.as_deref());
        let pr_state = match issue_number.and_then(|n| sub_issues.iter().find(|s| s.number == n)) {
            Some(s) => PrStateKnowledge::Read(s.pr_state.clone()),
            None => PrStateKnowledge::NotRead,
        };
        evaluate_flight(f, issue_number, &pr_state)
    }).collect()
}
```

La sous-issue appariée est en main. `s.pr_state` en sort, `s.ci_state` reste dedans. Le champ existe (`types.rs:221`), il est alimenté par `reader::classify_ci_state` (`reader.rs:858`), et il sert déjà à `SubIssue::is_hard_blocked` (`types.rs:237`) — mais pas à la recommandation.

### 1.3 Le vol 4f1fe163, minute par minute

| instant | fait |
|---|---|
| 17:43:53Z | le pilote ouvre mika#2966, tête `b972dd2e` |
| 17:44:16Z | `Egress Manifest Lint` finit en `failure` (job 123624936407, run 40924115345) |
| 17:46:12Z | `audit_events` ligne 805100 : `manager_flight_verdict`, `pr_opened #2966`, `recommendation: merge_candidate` — le check rouge est posé depuis **116 s** |
| 17:57:56Z | le run se termine |
| 17:58:58Z | le webhook `check_suite(failure)` est filtré sans tour (`webhook_prefilter_skipped`, classe `ci_failure_surfaced_for_adoption`) |
| 19:15:18Z | MPC juge (ligne 805201) : `manual_resumption` → `divergent (recommendation)`, fin d'une série de 6 vols concordants |

La valeur de `ci_state` lue à 17:46:12Z n'est pas mesurée. Elle n'a pas à l'être : le § 1.1 dit que le verdict ne la consulte pas, quelle qu'elle soit.

### 1.4 Le second trou : la requête milestone ne voit pas les PR de la boucle

`ci_state` ne vient que d'un endroit — le `statusCheckRollup` de l'appel 3 du reader (`reader.rs:211-225`) :

```
gh pr list --repo <R> --state all --search milestone:<N> \
   --json number,state,closingIssuesReferences,statusCheckRollup --limit 100
```

`--search milestone:N` ne rend que les PR portant **elles-mêmes** le milestone. Mesure du 2026-10-29 vers 19:24Z :

| PR | milestone | `closingIssuesReferences` |
|---|---|---|
| #2960 | aucun | aucune |
| #2962 | aucun | #2959 |
| #2964 | aucun | #1680 |
| #2966 | aucun | #2963 |
| #2967 | aucun | aucune |

Aucune. Donc pour ces vols la PR n'est jamais dans l'index inverse, et `ci_state` n'est pas le résultat d'une lecture.

**Le code porte déjà le discriminant de cette distinction**, et c'est ce qui rend le vol 1 possible sans lecteur neuf. `SubIssue::pr_link_source` (`types.rs:210`, mika#2924) dit de quel chemin vient `pr_number` :

```rust
pub enum PrLinkSource {
    /// L'index inverse des PR du milestone — porte `pr_state` et `ci_state`.
    MilestonePrList,
    /// La référence de clôture côté issue — ne porte que numéro et URL.
    IssueClosingRef,
}
```

`IssueClosingRef` ne porte **pas** `ci_state` : sur ce chemin le champ garde sa valeur par défaut, `CiState::None`. Et `CiState::None` est **aussi** ce que `classify_ci_state` rend pour un rollup vide, c'est-à-dire pour une PR réellement lue qui ne porte aucun check. Deux populations, les mêmes octets : la classe mika#2205, déjà à l'intérieur du champ. Le § 2 ne corrige pas `ci_state` — il refuse de le lire sans sa provenance.

### 1.5 Le gate de merge raisonne sur le rollup entier, pas sur les checks requis

Le ticket pose, au point 2, que « le gate de merge raisonne sur les checks requis ». La mesure dit le contraire. `tools/pr_merge_with_gate.rs:900` :

```rust
pub(crate) fn classify_checks(checks: &[GhCheck]) -> CheckClassification {
    let has_failures = checks.iter().any(|c| matches!(c.bucket.as_str(), "fail" | "cancel"));
    let has_pending = checks.iter().any(|c| c.bucket == "pending");
    if has_failures { CheckClassification::HasFailures }
    else if has_pending { CheckClassification::HasPending }
    else { CheckClassification::AllPassed }
}
```

Aucune notion de « requis » : le gate agrège tout ce que `gh pr checks` rend. C'est cette divergence avec la prémisse du ticket qui tranche le point 2, et le § 2.4 en tire la conséquence.

---

## 2. La décision

### 2.1 Un type, pas un `CiState` nu

`CiState` répond à « quel est l'état ? ». La recommandation a besoin de « **sait-on** quel est l'état, et depuis quand ? ». Deux questions, deux types. Le patron existe deux fois dans le module et se recopie sans rien inventer : `PrStateKnowledge` (`evaluator.rs:132`, mika#2921) et `PlanSizeReading`/`UnreadCause` (`pool.rs:550-580`).

Dans `evaluator.rs` :

```rust
/// Ce que le verdict sait de la CI de la tête de la PR du vol.
///
/// Pas un `CiState` nu, et pas un `Option<CiState>`, pour la raison de
/// mika#2921 R7 appliquée à un autre champ : « lu, et aucun check » et
/// « pas lu » ont des remèdes opposés, et `CiState::None` porte déjà les
/// deux (§ 1.4). Une CI non lue ne doit jamais passer pour une CI verte —
/// ni dans la recommandation, ni dans le rapport.
///
/// L'instant est dans la variante `Read` et non à côté : un état CI sans
/// son instant est une affirmation sur le présent, et le verdict est écrit
/// ~2 min après l'ouverture de la PR, pendant que la CI tourne (§ 2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiKnowledge {
    /// Le rollup de la PR a été lu, au cycle daté de cet instant ISO 8601
    /// UTC. C'est l'instant du **cycle**, pas celui de la requête `gh` : la
    /// granularité disponible est le cycle, parce qu'un cycle fait une seule
    /// lecture (§ 2.5).
    Read { state: CiState, at: String },
    /// Il ne l'a pas été — et la cause est dite.
    Unread(CiUnreadCause),
}

/// Pourquoi la CI d'un vol n'a pas été lue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiUnreadCause {
    /// La sous-issue appariée existe, mais son `pr_number` vient de
    /// [`PrLinkSource::IssueClosingRef`] : ce chemin ne porte pas de rollup,
    /// donc son `ci_state` n'est pas une lecture (§ 1.4).
    NotFromPrList,
    /// Aucune sous-issue du milestone cible ne porte le numéro du vol — le cas
    /// mesuré des cinq PR du § 1.4, et ce que le vol 2 ferme.
    NoSubIssue,
    /// Le vol ne nomme pas de ticket (`reference_url` absente ou non parsable),
    /// donc il n'y a rien à apparier.
    NoIssueNumber,
    /// La lecture n'a pas été tentée : verdict d'un binaire antérieur à ce
    /// champ, relu depuis l'arriéré du puits hors-ligne (motif mika#2340).
    NotMeasured,
}
```

Quatre causes et non une, pour la raison de `MergedPrReading` (`types.rs:118`) : « la PR n'est pas dans la liste du milestone » se corrige par le vol 2, « le vol ne nomme pas de ticket » ne se corrige pas du tout, et « blob antérieur » ne se corrige pas en production. Les replier donnerait un seul mot pour trois remèdes.

### 2.2 Le champ, et son défaut de relecture

Sur `FlightVerdict` (`evaluator.rs:171`), à côté de `pr_state` :

```rust
/// Ce que ce verdict savait de la CI (mika#2968). `#[serde(default)]` pour la
/// même raison que `pr_state` : le puits hors-ligne tient des blobs écrits
/// avant ce champ, et leur relecture doit dire « non mesurée » — jamais
/// « verte », jamais `CiState::None` (motif mika#2331).
#[serde(default = "CiKnowledge::default_unread")]
pub ci: CiKnowledge,
```

```rust
impl CiKnowledge {
    /// Le défaut de désérialisation : *pas mesuré*, jamais un état.
    fn default_unread() -> Self {
        CiKnowledge::Unread(CiUnreadCause::NotMeasured)
    }
}
```

C'est ce défaut qui rend la seconde moitié d'AC4 littérale : relire la ligne 805100 du vol 4f1fe163 sous le nouveau binaire rend `ci: unread(not_measured)`, donc « le manager d'alors ne lisait pas la CI » — l'explication de la divergence, écrite par le champ lui-même (§ 2.6).

### 2.3 Ce que chaque état donne — le point 1 du ticket, tranché

```rust
fn recommend(
    issue: &FlightIssue,
    pr_state: &PrStateKnowledge,
    ci: &CiKnowledge,
) -> FlightRecommendation {
    match issue {
        FlightIssue::PrOpened { .. } => match pr_state {
            PrStateKnowledge::Read(Some(s)) if !s.eq_ignore_ascii_case("open") => {
                FlightRecommendation::ManualResumption
            }
            // AC1 : une CI **lue** et rouge sur une PR encore ouverte.
            _ if matches!(ci, CiKnowledge::Read { state: CiState::Failed, .. }) => {
                FlightRecommendation::ManualResumption
            }
            _ => FlightRecommendation::MergeCandidate,
        },
        ...
```

| CI | PR ouverte | PR fusionnée ou fermée |
|---|---|---|
| `Read{Success}` | `MergeCandidate` | `ManualResumption` |
| `Read{Failed}` | **`ManualResumption`** (change) | `ManualResumption` |
| `Read{Pending}` | `MergeCandidate` | `ManualResumption` |
| `Read{None}` (lue, aucun check) | `MergeCandidate` | `ManualResumption` |
| `Unread(_)` | `MergeCandidate` | `ManualResumption` |

Trois décisions sont écrites là, et chacune est le point 1 ou le point 3 du ticket :

**`ManualResumption`, pas une variante neuve.** C'est ce que MPC a jugé pour ce vol (ligne 805201), et c'est le geste dû : la PR attend un correctif, un humain la reprend. Une quatrième variante (`WaitForCi`, `CiRed`…) ajouterait un terme au vocabulaire que `VerdictTerms` compare, donc à celui que MPC doit apprendre (mika#2158) — pour un geste identique. Le détail *pourquoi* la reprise est due va dans le segment CI du rapport (§ 2.5), pas dans la recommandation.

**`Pending` reste `MergeCandidate`, et le rapport le dit.** Un verdict est écrit une fois, ~2 min après l'ouverture ; pour #2966 le run n'a fini qu'à 17:57:56Z, soit 11 min plus tard. Refuser `MergeCandidate` sur `Pending` rendrait `ManualResumption` sur la quasi-totalité des vols sains — un instrument qui crie à chaque vol ne dit plus rien. Une CI en cours n'est pas une preuve de refus : c'est une absence de preuve, et le gate de merge la refusera de lui-même (`CheckClassification::HasPending`). Le rapport écrit « CI en cours (lue à …) », donc l'opérateur sait qu'il attendra. Noter que le cas mesuré **n'est pas** celui-là : à 17:46:12Z le check était rouge depuis 116 s, et `classify_ci_state` fait gagner `any_failed` sur `any_pending` (`reader.rs:881`) — donc le rollup de #2966 à cet instant rend `Failed`, pas `Pending`. Le défaut mesuré est couvert par le bras rouge.

**Un rouge postérieur ne fait pas réviser le verdict.** La règle « un vol déjà jugé n'est pas rejugé » est étendue au manager, pas contournée : le verdict est une **mesure horodatée**, et c'est le `at` de la variante `Read` qui le rend honnête — il dit « CI rouge, lue à 17:46:12Z », jamais « CI rouge ». Conséquence assumée et nommée : un vol jugé sur `Pending` dont la CI rougit ensuite garde un verdict `MergeCandidate`, et rien ne le reprend. Le faire réapparaître au rapport suivant demanderait de rejouer les verdicts déjà écrits, donc de les relire depuis le registre et de les réconcilier — un livrable qui n'est ni dans AC1–AC5 ni sous la borne de ce vol. Il est renvoyé au § 9.

### 2.4 La population : le rollup entier — le point 2, tranché

La recommandation se fonde sur le `statusCheckRollup` **entier**, sans notion de check requis, pour trois raisons mesurées :

1. **C'est la population du gate.** `classify_checks` (§ 1.5) agrège tous les buckets. Fonder la recommandation sur les checks requis et le refus de merge sur tout le rollup ferait deux prédicats pour une question — exactement ce que mika#2201 refuse — et ferait dire « merge-candidat » à une PR que le gate refuse. La prémisse du ticket au point 2 est donc corrigée ici, et le tableau du § 1.5 est la correction.
2. **`classify_ci_state` est déjà ce lecteur** (`reader.rs:858`), et il est le lecteur unique du rollup côté manager. Ce plan n'en écrit pas un second.
3. **Les checks requis ne sont pas lisibles ici.** La liste des contextes requis vit dans les branch-protection rules, que `statusCheckRollup` ne rend pas et qu'un jeton restreint ne lit pas (c'est précisément la classe mika#2963). Un prédicat qui dépend d'une lecture indisponible sur deux dépôts sur quatre est un prédicat qui échoue en silence là où il compte.

Coût assumé : un check non requis rouge (un lint avancé, un job facultatif) fait dire `ManualResumption`. Le rapport nomme l'état sur lequel la décision repose, donc l'opérateur peut arbitrer. Le faux positif est du côté sûr : il envoie regarder, il n'envoie pas merger.

### 2.5 D'où vient `ci` au vol 1 — la provenance, pas le champ nu

`evaluate_flights` apparie déjà le vol à sa sous-issue. Il lit désormais **deux** champs de la sous-issue appariée, et le second seulement si sa provenance le garantit :

```rust
pub fn evaluate_flights(
    flights: &[FinishedFlight],
    sub_issues: &[SubIssue],
    read_at: &str,
) -> Vec<FlightVerdict> {
    flights.iter().map(|f| {
        let issue_number = issue_number_from_url(f.reference_url.as_deref());
        let matched = issue_number.and_then(|n| sub_issues.iter().find(|s| s.number == n));
        let pr_state = match matched {
            Some(s) => PrStateKnowledge::Read(s.pr_state.clone()),
            None => PrStateKnowledge::NotRead,
        };
        let ci = match (issue_number, matched) {
            (None, _) => CiKnowledge::Unread(CiUnreadCause::NoIssueNumber),
            (Some(_), None) => CiKnowledge::Unread(CiUnreadCause::NoSubIssue),
            (Some(_), Some(s)) => match s.pr_link_source {
                Some(PrLinkSource::MilestonePrList) => CiKnowledge::Read {
                    state: s.ci_state,
                    at: read_at.to_string(),
                },
                _ => CiKnowledge::Unread(CiUnreadCause::NotFromPrList),
            },
        };
        evaluate_flight(f, issue_number, &pr_state, &ci)
    }).collect()
}
```

Trois propriétés de cette forme, chacune délibérée :

- **`pr_link_source` est le garde, pas une précaution.** `ci_state` vaut `CiState::None` aussi bien pour « lue, aucun check » que pour « ce chemin ne lit pas de rollup » (§ 1.4). Sans le discriminant, le vol 1 ferait passer la seconde pour la première — c'est-à-dire ferait exactement ce que le point 3 du ticket interdit.
- **`Some(PrLinkSource::IssueClosingRef)` et `None` tombent dans le même bras**, mais pour des raisons différentes qui ne changent pas la conclusion : le premier ne porte pas de rollup, le second est un blob antérieur à mika#2924. Les deux donnent `NotFromPrList`, dont le libellé dit « pas lue par la liste des PR » et non « pas de checks ».
- **`read_at` vient de `ctx.now`, et c'est une décision mesurée.** `collect_flight_verdicts` reçoit déjà `ctx` (`cadence.rs:682`) : il passe `ctx.now.to_rfc3339()`. Ce n'est pas un instant inventé sur place — dans le chemin de production, `run_manager_cycle_with_auth` (`cadence.rs:1205`) appelle `reader.read()` à la l. 1212 **puis** construit `CycleContext::new(&deliverer, Utc::now())` à la l. 1223. `ctx.now` est donc capturé *après* la lecture : il la **majore**, de la durée du probe de santé de la l. 1214. C'est pourquoi le doc-comment du § 2.1 et le libellé du § 2.7 disent « au cycle de », jamais « à la seconde où la requête est partie ».

  **Deux alternatives écartées, avec leur coût.** (a) `timestamp::now()` dans `evaluate_flights` : daterait la lecture de l'instant du verdict, après les sept appels `gh` — strictement moins juste que `ctx.now`, pour la même plomberie. (b) Un instant posé à la source : `reader.rs` ne porte **aucune** horloge (`Utc::now` absent du fichier), donc il faudrait injecter un `now` dans `read_with_runner` et reprendre ses **11 sites d'appel**, dont 10 en test. Cela achèterait quelques secondes de précision sur un instant que le rapport présente déjà à la granularité du cycle. Si un besoin futur réclame l'instant de la requête, c'est là qu'il faudra payer — et `MilestoneState` est le porteur naturel, sur le patron déjà établi de `ready_pool` / `outside_candidates`, posés après composition (`reader.rs:280-283`).

**Effet mesuré au vol 1, dit sans l'enjoliver.** Sur les cinq PR du § 1.4, aucune n'est dans la liste du milestone : le verdict portera `Unread(NoSubIssue)` ou `Unread(NotFromPrList)`, la recommandation restera `MergeCandidate`, et le rapport dira **« CI non lue (PR hors liste du milestone) »** au lieu de se taire. C'est le changement réel du vol 1 : la recommandation cesse de prétendre, et la cause de la non-lecture est nommée là où un opérateur la lit. Le vol 2 remplace cette non-lecture par une lecture et ferme AC5.

### 2.6 Ce que le comparateur ne voit pas — AC4

`VerdictTerms` (`flight_verdict_ledger.rs`) a trois champs — `issue`, `cause`, `recommendation` — et `first_divergence` les compare dans cet ordre fixe. **`ci` n'y entre pas**, et `terms()` ne le lit pas. Donc :

- la ligne 805100 du vol 4f1fe163 reste `Divergent { field: "recommendation" }` — la divergence ne disparaît pas, elle reste exactement ce qu'elle était ;
- `consecutive_concordant` et la série de 6 vols sont inchangés ;
- aucun drapeau CLI n'est ajouté à `mika flight-verdicts concordance` : MPC ne mesure pas la CI, donc un champ comparé n'aurait pas de contrepartie, et un drapeau pour un champ non comparé serait du YAGNI.

`VerdictRecord` porte en revanche le champ, avec `#[serde(default)]`, **hors** de `terms()` :

```rust
/// Ce que le manager savait de la CI (mika#2968). **Hors de [`Self::terms`]** :
/// MPC ne mesure pas la CI, donc la comparer ferait diverger tous les vols sur
/// un champ que le juge n'a pas. Porté quand même, parce qu'AC4 demande que la
/// divergence soit **expliquée** : relire un verdict d'avant ce champ rend
/// `unread(not_measured)`, ce qui nomme la cause de la divergence du vol
/// 4f1fe163 — le manager d'alors ne lisait pas la CI.
#[serde(default)]
pub ci: CiKnowledge,
```

### 2.7 Le rapport — AC3

`reporter.rs:754`, un segment de plus, inséré entre l'état de PR et la recommandation, sur le patron exact de `flight_pr_state_label` (`reporter.rs:806`) :

```rust
/// L'état CI sur lequel la recommandation repose, avec le moment de sa lecture.
///
/// Toujours `Some` — contrairement à [`flight_pr_state_label`], qui omet son
/// segment sur `Read(None)`. Un verdict dont le segment CI manque serait
/// indistinguable d'un verdict dont la CI est verte, et c'est la lecture que le
/// point 3 de mika#2968 interdit : *une CI non lue ne doit jamais passer pour
/// une CI verte*. Une ligne par vol, y compris verte.
fn flight_ci_label(ci: &CiKnowledge) -> String {
    match ci {
        CiKnowledge::Read { state, at } => {
            let etat = match state {
                CiState::Success => "CI verte",
                CiState::Failed => "CI rouge",
                CiState::Pending => "CI en cours",
                CiState::None => "CI sans check",
            };
            // « au cycle de » et non « à » : l'instant est celui du cycle qui a
            // lu, capturé après le retour du reader (§ 2.5). Un « lue à »
            // promettrait la seconde de la requête `gh`, que le manager n'a pas.
            format!("{etat} (lue au cycle de {at})")
        }
        CiKnowledge::Unread(c) => format!("CI non lue ({})", ci_unread_label(*c)),
    }
}

fn ci_unread_label(c: CiUnreadCause) -> &'static str {
    match c {
        CiUnreadCause::NotFromPrList => "PR hors liste du milestone",
        CiUnreadCause::NoSubIssue => "aucune sous-issue appariée",
        CiUnreadCause::NoIssueNumber => "vol sans ticket",
        CiUnreadCause::NotMeasured => "non mesurée",
    }
}
```

Les deux `match` sont exhaustifs sans bras `_` (mika#2923) : une cinquième cause — celles du vol 2 — ne compile pas sans son libellé.

Le vol mesuré, tel qu'il se lira — la recommandation change (AC1) **et** la ligne dit sur quoi elle repose (AC3) :

```
- mika#2963 (milestone cible) — PR #2966 ouverte — 7 tours / 200 — PR open — CI rouge (lue au cycle de 2026-10-29T17:46:12Z) — reprise à la main
```

Et le même vol tel qu'il se lira au vol 1, où la PR de la boucle n'est pas dans la liste du milestone (§ 2.5) — la recommandation ne change pas, mais elle cesse de prétendre :

```
- mika#2963 — PR #2966 — 7 tours / 200 — état de PR non lu — CI non lue (aucune sous-issue appariée) — merge-candidat
```

---

## 3. Ce que le plan ne touche pas

- **`classify_ci_state`** (`reader.rs:858`) — lecteur unique du rollup, réutilisé tel quel. Aucune seconde classification.
- **`CiState`** (`types.rs:71`) — quatre variantes, inchangées. Le tri-état « su / pas su » est au-dessus, dans `CiKnowledge`, pas dedans : `CiState` a d'autres lecteurs (`is_hard_blocked`, l'Assessor) qui n'ont pas à apprendre une variante.
- **`pr_state`, et c'est une décision, pas un oubli.** Le ticket demande que le plan dise comment `pr_state` est lu, puisque la même requête l'alimente. Réponse : **il reste lu par l'appariement au milestone, inchangé, aux deux vols.** La raison est dans `reporter.rs:798` — `flight_milestone_label` *dérive* l'appartenance au milestone cible de `PrStateKnowledge` (`Read(_)` ⇒ « milestone cible », `NotRead` ⇒ hors cible). Poser `Read` depuis une lecture directe de PR ferait écrire « milestone cible » pour un ticket qui n'y est pas : un mensonge dans le rapport, au titre d'une amélioration. Découpler le libellé demande un champ d'appartenance explicite sur `FlightVerdict`, son `#[serde(default)]`, et la reprise des tests de `reporter.rs` qui en dépendent — c'est le vol 2, § 9.
- **`VerdictTerms`, `first_divergence`, `consecutive_concordant`, `print_terms`** — non édités (§ 2.6). AC4.
- **Le pré-filtre webhook** (mika#2675, la classe `ci_failure_surfaced_for_adoption`) — il écarte l'échec CI sans tour, et c'est ce qui rend le verdict du manager seul porteur du signal. Son comportement n'est pas touché ici.
- **`MIKA_MANAGER_EVALUATE_WRITE`** — reste OFF, lecteur unique `verdict_poster::write_is_armed`. Ce plan rend la concordance présentable ; l'ouverture revient à Vincent.
- **Les gardes structurels du module** — `no_dispatch_test.rs` (aucun jeton d'écriture ajouté : le vol 1 n'appelle pas `gh`) et `no_repo_list_test.rs` (aucun nom de dépôt écrit). Les deux restent verts sans exception nouvelle.

---

## 4. Livrables

| # | fichier | geste |
|---|---|---|
| L1 | `evaluator.rs` | `CiKnowledge` + `CiUnreadCause` + `default_unread`, avec les doc-comments du § 2.1 |
| L2 | `evaluator.rs` | champ `ci` sur `FlightVerdict`, `#[serde(default = …)]` |
| L3 | `evaluator.rs` | `recommend` prend `&CiKnowledge`, bras rouge du § 2.3 |
| L4 | `evaluator.rs` | `evaluate_flight` et `evaluate_flights` : signatures, `read_at`, appariement par `pr_link_source` (§ 2.5) |
| L5 | `cadence.rs` | `collect_flight_verdicts` (l. 682) passe `ctx.now.to_rfc3339()` à `evaluate_flights` ; aucune horloge neuve, aucune signature d'appelant touchée (§ 2.5) |
| L6 | `reporter.rs` | `flight_ci_label` + `ci_unread_label` + segment dans `render_flight_verdict` |
| L7 | `flight_verdict_ledger.rs` | champ `ci` sur `VerdictRecord`, `From<&FlightVerdict>`, **hors** `terms()` |
| L8 | tests `evaluator.rs` | la matrice d'AC2, sur la fixture mesurée de #2966 |
| L9 | tests `reporter.rs` | les cinq libellés CI, dont « non lue » |
| L10 | tests `flight_verdict_ledger.rs` | le comparateur ne voit pas la CI (AC4) |

### 4.1 La matrice d'AC2 (L8), et sa fixture

Un seul test, table-driven, dans `evaluator.rs`. Quatre états CI × deux états de PR, plus les causes de non-lecture :

```rust
/// Le rollup **mesuré** de mika#2966 à 17:46:12Z, réduit au check qui décide.
/// Pas un littéral inventé : `Egress Manifest Lint` est le job 123624936407 du
/// run 40924115345, terminé en `failure` à 17:44:16Z — 116 s avant le verdict
/// de la ligne 805100 d'`audit_events`.
fn rollup_2966_mesure() -> serde_json::Value {
    json!([
        { "name": "Egress Manifest Lint", "status": "COMPLETED", "conclusion": "FAILURE" },
        { "name": "Check", "status": "COMPLETED", "conclusion": "SUCCESS" }
    ])
}
```

Le test fait passer ce rollup par `classify_ci_state` — le lecteur réel, pas une valeur posée à la main — et assied `CiState::Failed`. C'est ce qui rend le cas « rouge et ouverte » issu de la mesure et non d'un littéral : si `classify_ci_state` changeait d'avis sur ce rollup, le test le dirait.

Les douze cellules, dont les trois d'AC1 et le contrôle négatif d'AC2 (`vert + ouverte ⇒ MergeCandidate`) sont dans la **même** fonction de test, parce que c'est l'appariement des deux qui prouve que le bras rouge n'est pas un refus inconditionnel.

### 4.2 Le test d'AC4 (L10)

```rust
// `VerdictRecord` porte la CI ; `terms()` ne la lit pas. Les deux moitiés
// d'AC4 dans une seule fonction : la divergence du vol 4f1fe163 reste
// divergente, et un verdict d'avant le champ l'explique.
let rouge = record_avec_ci(CiKnowledge::Read { state: CiState::Failed, at: "…17:46:12Z".into() });
let vert  = record_avec_ci(CiKnowledge::Read { state: CiState::Success, at: "…17:46:12Z".into() });
assert_eq!(rouge.terms(), vert.terms(), "la CI n'entre pas dans les termes comparés");

// Le blob historique, relu : non mesurée, jamais verte.
let historique: VerdictRecord = serde_json::from_str(r#"{"issue":"pr_opened","recommendation":"merge_candidate"}"#).unwrap();
assert_eq!(historique.ci, CiKnowledge::Unread(CiUnreadCause::NotMeasured));
assert_eq!(first_divergence(&historique, &mpc_manual_resumption()), Some("recommendation"));
```

---

## 5. Ordre de commit

1. **L1 + L2** — le type et le champ, avec le défaut serde. Vert et additif : rien ne lit encore `ci`, tous les tests existants passent (le champ prend son défaut).
2. **L3 + L4 + L5** — la décision et son alimentation. C'est le seul commit qui change un comportement.
3. **L6** — le rapport.
4. **L7** — le registre.
5. **L8 + L9 + L10** — les tests.

Un vol interrompu après le commit 1 laisse l'arbre sain et le comportement inchangé.

---

## 6. Vérification

- `cargo test -p mika-agent milestone_manager` et la suite complète.
- `cargo clippy --all-targets -- -D warnings`.
- `cargo fmt --check`.
- `scripts/check-byte-slices.sh`, `scripts/verify-pipeline.sh origin/main`.
- Les deux gardes du module : `no_dispatch_test.rs`, `no_repo_list_test.rs` — verts sans exception nouvelle (§ 3).
- Les tests existants de `reporter.rs` (l. 1214-1290) : ils construisent des `FlightVerdict` littéraux et prennent le défaut `Unread(NotMeasured)`, donc leurs lignes gagnent un segment « CI non lue (non mesurée) ». Les assertions qui comparent une ligne entière sont à reprendre — c'est l'essaimage compté au § 8.

---

## 7. Definition of Done

- `CiKnowledge` et `CiUnreadCause` livrés dans `evaluator.rs`, avec les doc-comments du § 2.1 (la raison mika#2921 R7, l'instant dans la variante, les quatre causes distinctes).
- `FlightVerdict::ci` livré avec `#[serde(default = "CiKnowledge::default_unread")]` ; un blob antérieur relu rend `Unread(NotMeasured)` et jamais un état.
- `recommend` prend `&CiKnowledge` ; une CI **lue** et rouge sur une PR ouverte rend `ManualResumption` ; `Pending`, `None` et `Unread(_)` rendent `MergeCandidate`, et le § 2.3 porte par écrit pourquoi.
- `evaluate_flights` alimente `ci` depuis la sous-issue appariée **sous condition de `pr_link_source == Some(MilestonePrList)`** ; les trois autres cas donnent leur cause nommée.
- `read_at` alimenté par `ctx.now.to_rfc3339()` depuis `collect_flight_verdicts` ; aucun `timestamp::now()` ni `Utc::now()` dans l'évaluateur, aucune horloge ajoutée à `reader.rs`, aucun des 11 sites de `read_with_runner` touché.
- `render_flight_verdict` porte un segment CI **sur chaque vol**, y compris vert, y compris non lu ; les deux `match` de libellé exhaustifs sans bras `_`.
- `VerdictRecord::ci` livré avec `#[serde(default)]` et **absent** de `terms()` ; `VerdictTerms`, `first_divergence`, `consecutive_concordant` et `print_terms` non édités.
- La matrice d'AC2 verte dans une seule fonction, son cas rouge passant par `classify_ci_state` sur le rollup mesuré de #2966, son contrôle négatif vert dans la même fonction.
- Le test d'AC4 vert : `terms()` insensible à la CI, blob historique relu en `NotMeasured`, divergence du vol 4f1fe163 toujours `recommendation`.
- Les tests de `reporter.rs` repris pour le segment neuf ; aucun test existant supprimé.
- Suite complète, clippy, fmt, `check-byte-slices.sh` et `verify-pipeline.sh origin/main` verts.
- `no_dispatch_test.rs` et `no_repo_list_test.rs` verts sans exception nouvelle.
- Le corps de la PR porte le périmètre du vol 1, la ligne `Tracked in:` du vol 2 (§ 9), et la correction de la prémisse du point 2 (§ 1.5).

## 8. Acceptance criteria

- **AC1.** Pour un vol `PrOpened` dont la PR est ouverte, une CI rouge sur la tête donne une recommandation autre que `MergeCandidate`. Le plan nomme laquelle : `ManualResumption` ou une variante neuve, avec ce que le rapport et `VerdictTerms` en écrivent.
- **AC2.** Test dans le même appel, sur une matrice : CI verte, rouge, en cours et non lue, chacune croisée avec PR ouverte, puis PR fusionnée ou fermée. Le cas rouge et ouverte part des données mesurées de #2966, pas d'un littéral inventé. Le cas vert et ouverte reste `MergeCandidate`.
- **AC3.** Le rapport du sink dit, à côté de la recommandation, l'état CI sur lequel elle repose, avec le moment de la lecture. Une CI non lue se dit « non lue ».
- **AC4.** Le comparateur de `mika flight-verdicts concordance` n'en est pas changé : la divergence du vol 4f1fe163 reste divergente, et elle est expliquée par le nouveau champ.
- **AC5.** Sonde après déploiement : sur les 10 vols suivants qui ouvrent une PR, le compte de verdicts `MergeCandidate` dont la tête portait un check requis rouge au moment du verdict est écrit ici, avec la requête. La cible est 0.

**Où chacun est fermé.** AC1 → § 2.3 (`ManualResumption`, nommé, avec la raison de ne pas créer de variante) + § 2.6 (ce que `VerdictTerms` en écrit : rien, et pourquoi) + § 2.7 (ce que le rapport en écrit) + L3. AC2 → § 4.1, une seule fonction, fixture passée par `classify_ci_state`, contrôle négatif inclus. AC3 → § 2.7 + L6 + L9. AC4 → § 2.6 + § 4.2 + L7 + L10. **AC5 → vol 2** (§ 9) : la sonde compte des verdicts fondés sur une CI *lue*, et au vol 1 les PR de la boucle restent hors de la liste du milestone (§ 2.5). Sa requête est écrite au § 9 pour que le vol 2 n'ait pas à la redécouvrir.

## 9. Le vol 2, et pourquoi il est séparé

**Périmètre du vol 2 :** le lecteur CI par numéro de PR, `pr_state` lu par la même requête, le découplage de `flight_milestone_label`, et la sonde d'AC5.

- **Le lecteur.** Un appel `gh pr list --repo <R> --state all --json number,state,statusCheckRollup --limit 100` **par dépôt distinct** présent dans les verdicts `PrOpened`/`RescuePr`, exécuté via `ctx.gh_runner` (déjà porté par `CycleContext`, `cadence.rs:529`) depuis `collect_flight_verdicts`, puis un lookup par `FlightIssue::PrOpened { number }`. Un appel par dépôt et non par vol, parce qu'au premier cycle il n'y a pas de checkpoint et la source rend **tous** les vols terminés (`cadence.rs:691`, `finished_implement_flights(None)`) : un appel par vol n'a pas de borne, un appel par dépôt est borné par `flight_source::controlled_repos().len()`. Le dépôt vient de `FlightVerdict::repo`, jamais d'un littéral (garde `no_repo_list_test.rs`). Refus nommés sur le patron de `write_pool_admissions` : `NoRunner`, `ReadFailed`, `NotFound` — trois causes de plus sur `CiUnreadCause`, dont les libellés manquants ne compilent pas (§ 2.7).
- **`pr_state` et le label milestone.** La même requête rend `state`, donc le vol 2 peut poser `PrStateKnowledge::Read` depuis la lecture directe — mais seulement après avoir donné à `flight_milestone_label` un champ d'appartenance explicite (§ 3), sans quoi le rapport écrirait « milestone cible » pour un ticket hors cible.
- **La sonde d'AC5**, pour que le vol 2 n'ait pas à la retrouver :

```sql
-- Les 10 verdicts de vol les plus récents qui ont ouvert une PR, avec l'état CI
-- sur lequel leur recommandation reposait. Cible : zéro ligne où la
-- recommandation est merge_candidate et la CI lue rouge.
SELECT json_extract(payload, '$.issue_number')     AS ticket,
       json_extract(payload, '$.recommendation')   AS recommandation,
       json_extract(payload, '$.ci')               AS ci,
       created_at
FROM audit_events
WHERE event_type = 'manager_flight_verdict'
  AND json_extract(payload, '$.issue.type') = 'pr_opened'
ORDER BY created_at DESC
LIMIT 10;
```

Deux mises en garde de lecture. L'**absence** du champ `ci` sur une ligne signifie un binaire antérieur au vol 1, jamais une CI absente (classe mika#2340) : ces lignes se comptent à part, elles ne se somment pas avec les `unread`. Et un `unread` n'est **pas** un succès de la sonde : la cible « zéro `merge_candidate` sur CI rouge » est atteinte trivialement si rien n'est lu — donc la sonde se lit avec son dénominateur, le compte de `read` sur les 10.

**Pourquoi la séparation, et pas un seul vol.** La section `## Taille estimée` chiffre le vol 1 à 326 lignes, soit 103 sous la borne de 429. Le lecteur et ses tests sont estimés à 176 lignes (`reader.rs` 52, `cadence.rs` 46, tests 78) : ils ne tiennent pas dans cette marge, et le découpage est donc celui que mika#2693 prescrit, non une commodité. La coupe est posée là plutôt qu'ailleurs parce que c'est la seule qui donne deux vols mergeables seuls : livrer la décision sans le rendu écrirait un verdict sans dire sur quoi il repose — exactement le défaut que le point 3 du ticket condamne —, et livrer le lecteur sans la décision alimenterait un champ que personne ne lit.

Tracked in: senara-solutions/mika#2968 — vol 2 : le lecteur CI par numéro de PR, `pr_state` lu directement, le découplage de `flight_milestone_label`, et la sonde d'AC5. À ouvrir en sous-ticket, ou à rouvrir sur ce ticket à la fermeture du vol 1.

## Fire-Disposition

**Gate N/A — ce plan ne livre aucun détecteur.**

Les quatre livrables de test (L8, L9, L10, et les reprises du § 6) sont des **assertions de comportement** : leur chemin de succès est « la recommandation vaut `ManualResumption` », « la ligne porte le segment CI », « `terms()` est insensible à la CI ». Aucun ne scanne une population à la recherche d'une violation, aucun n'a « aucune violation trouvée » pour chemin de succès, aucun n'a d'allowlist à tenir.

Deux détecteurs plausibles ont été envisagés et refusés, pour que leur absence soit une décision et non un oubli :

- **Un scan « la CI n'entre pas dans `VerdictTerms` ».** AC4 tient par construction — le champ n'est pas dans la struct comparée — et se mesure par comportement (§ 4.2) : deux `VerdictRecord` ne différant que par la CI ont des `terms()` égaux. Un scan de source ajouterait une allowlist à maintenir pour une propriété que le type porte déjà.
- **Un garde « un seul lecteur du rollup ».** `classify_ci_state` est déjà ce lecteur unique et ce plan n'en écrit pas un second (§ 3). Le garde serait écrit pour défendre une propriété que le vol n'attaque pas. Si le vol 2 ajoute un second site de classification, c'est **là** que la question se pose, et son plan devra la trancher.

Donc aucune des trois options de mika#1574 n'est applicable ici : il n'y a ni allowlist à livrer (option a), ni détecteur à désarmer (option b), ni cadrage à remonter (option c). La section est présente pour que son « N/A » soit une décision lisible, avec les deux candidats écartés nommés, plutôt qu'un silence qu'un relecteur devrait interpréter.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (L1 — `CiKnowledge`, `CiUnreadCause`, doc-comments) | 63 |
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (L2 — champ `ci`, défaut serde, doc) | 16 |
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (L3 — `recommend`, signature et bras) | 26 |
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (L4 — `evaluate_flight`, `evaluate_flights`, appariement) | 28 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` (L5 — passage de `ctx.now.to_rfc3339()`, doc de la décision) | 8 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (L6 — deux libellés, segment, doc) | 31 |
| `crates/mika-agent/src/flight_verdict_ledger.rs` (L7 — champ hors `terms()`, doc) | 18 |
| essaimage littéral (constructions `FlightVerdict {` des tests existants) | 14 |
| tests (L8 — matrice d'AC2, fixture mesurée de #2966) | 72 |
| tests (L9 — les cinq libellés CI) | 26 |
| tests (L10 — le comparateur et le blob historique) | 24 |

Total estimé : 326 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 326 < 429 et très en dessous du seuil de 1000 par PR. Trois raisons de ne pas découper davantage :

1. **Le découpage a déjà eu lieu, et il est nommé.** Le périmètre total du ticket est estimé à ~500 lignes ; les 176 du lecteur CI et de la sonde sont au vol 2, avec leur `Tracked in:` au § 9. La marge restante du vol 1 — 103 lignes — ne les contient pas, ce qui est l'argument du découpage et non une coïncidence.
2. **Ce qui reste est une seule chaîne causale.** Le type, la décision qui le lit, le rendu qui la justifie et le registre qui l'explique ferment AC1 à AC4 ensemble. Couper plus bas donnerait un vol livrant un champ que personne ne lit (aucun AC fermé) ou un verdict changé sans la ligne de rapport qui dit sur quoi il repose — le défaut même du point 3 du ticket.
3. **Deux postes sur onze ne coûtent pas de tours.** Les 14 lignes d'essaimage littéral et les 16 du champ sont mécaniques, produites en une passe. La borne convertit des tours en lignes **rédigées** ; ces 30-là en consomment une fraction, ce qui laisse la marge réelle au-dessus des 103 nominales.

L'ordre de commit du § 5 est construit pour que ce soit vrai même si un tour manque : le commit 1 est vert et additif, et seul le commit 2 change un comportement — un vol interrompu après le 1 laisse l'arbre sain.

## Hors périmètre

- **Le lecteur CI du moteur et son jeton** — mika#2963. Sur mika-cloud et mika-platform le jeton ne rend pas `statusCheckRollup` ; ce plan en fait une **cause de non-lecture nommée** (vol 2, `ReadFailed`), il ne répare pas le jeton.
- **L'ouverture de `MIKA_MANAGER_EVALUATE_WRITE`** — revient à Vincent. Ce plan rend la concordance présentable, il ne décide pas de l'ouvrir.
- **La révision d'un verdict déjà écrit** — § 2.3. Un rouge postérieur au verdict ne le fait pas réviser et ne réapparaît pas au rapport suivant ; le faire demanderait de relire le registre et de réconcilier, ce qu'aucun des cinq AC ne demande.
- **Les checks requis** — § 2.4. La population est le rollup entier, délibérément, et la prémisse du point 2 du ticket est corrigée par la mesure du § 1.5.
- **Le pré-filtre webhook** (mika#2675) qui écarte `check_suite(failure)` sans tour — c'est ce qui rend le verdict du manager seul porteur du signal, et c'est un autre ticket.
- **Un drapeau CLI pour comparer la CI** dans `mika flight-verdicts concordance` — § 2.6. MPC ne mesure pas la CI ; un drapeau pour un champ non comparé serait du YAGNI.
- **L'instant exact de la requête `gh`** — § 2.5. Le rapport date la lecture à la granularité du **cycle** (`ctx.now`, capturé après le retour du reader). Descendre à la seconde de la requête demanderait une horloge dans `reader.rs`, qui n'en a aucune, et la reprise de ses 11 sites d'appel. L'alternative est nommée au § 2.5 avec son porteur naturel (`MilestoneState`, patron `ready_pool`), pour le jour où un besoin la réclame.
