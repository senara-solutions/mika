# mika#2968 — vol 2b : la revue de la tête entre dans la recommandation

Ce plan livre **AC6 et AC7** : `recommend` ne rend plus `MergeCandidate` pour un vol
`PrOpened` à PR ouverte et CI verte sans une approbation de `mika-platform-qa` **sur la
tête exacte**, et le rapport du sink dit l'état de la revue, le commit revu et le moment
de la lecture.

Les vols antérieurs sont livrés et ne sont pas regroomés :

| vol | PR | ce qu'il a fermé |
|---|---|---|
| 1 | mika#2971 (`a219d0a3c` lignée) | AC1 à AC4 : `recommend` lit `CiKnowledge` ; `CiState::Pending` reste `MergeCandidate` ; un verdict déjà écrit n'est pas réécrit ; `VerdictTerms` inchangée |
| 2a | mika#2977 (`a9a673473`) | le lecteur par numéro de PR (`read_flight_prs`), `pr_state` lu par la même requête, les 6 causes de non-lecture, le découplage de `flight_milestone_label` |

Il reste **ce vol** (AC6, AC7) et la sonde (AC5, AC8), qui n'est pas du code — voir § 9.

---

## 1. Contrainte de périmètre — tranchée en tête

Le corps du ticket exige : *« Si le lecteur par numéro de PR doit être injecté dans le
cycle, le plan dit comment il y parvient sans toucher `CycleContext`, ou il dit en tête
pourquoi il ne le peut pas, avant toute implémentation. »*

**Il n'a rien à injecter : le vol 2a l'a déjà fait.** `CycleContext::gh_runner:
Option<&'a dyn GhRunner>` (`cadence.rs` l. 529) et son builder `with_gh_runner` (l. 568)
sont des champs **existants**, et `read_flight_prs` est déjà appelé depuis le corps de
`collect_flight_verdicts` (`cadence.rs` l. 723-725) :

```rust
Some(runner) => FlightPrReads::Read(read_flight_prs(runner, &repos).await),
None => FlightPrReads::Unwired,
```

Ce vol **n'ajoute que deux champs `--json` à une requête `gh` déjà émise**. Conséquence
directe : aucune signature de cycle ne change.

- `cadence.rs` : **aucune ligne touchée.** Pas même celle de l'appel `evaluate_flights`
  (l. 698 sur le texte du ticket, l. 723-725 aujourd'hui) — la signature de
  `evaluate_flights` est inchangée, puisque la revue voyage dans `FlightPrReads`, qui est
  déjà son paramètre.
- `CycleContext` (l. 500), les builders `with_*` (l. 558-570), `write_pool_admissions`
  (l. 836), `run_manager_cycle_in` (l. 891-947) : **hors d'atteinte par construction**.
- `spawn.rs`, `groom_trigger.rs`, `server/mod.rs`, le bloc `pub use groom_trigger::{…}`
  de `mod.rs`, `evidence/guards.rs`, `skills/builtin_handlers.rs` : **non touchés**.

Périmètre réel : `reader.rs`, `evaluator.rs`, `reporter.rs`, plus les ré-exports de
`mod.rs` (l. 312-335) et l'essaimage du champ neuf dans les constructions de test de
`flight_verdict_ledger.rs`, `verdict_poster.rs`, `types.rs`.

---

## 2. La source de la revue — ce que la mesure tranche

Le ticket demande de trancher *« la source de la revue (la même requête par numéro de PR
que la CI, ou une autre) »*. Le § 11 du plan du vol 2a esquissait `latestReviews`. **La
mesure l'invalide.**

Mesures du 2026-10-30 contre `senara-solutions/mika` :

| requête | rendu |
|---|---|
| `gh pr view 2972 --json latestReviews` | 1 revue, `state: APPROVED`, **`commit: ""`** |
| `gh pr list --state open --json latestReviews --limit 10` | 5 PR, **`commit: ""` partout** |
| `gh pr view 2972 --json reviews` | 2 revues, **`commit.oid` plein** : `COMMENTED` à `2026-10-29T22:42:22Z` et `APPROVED` à `2026-10-30T07:14:30Z`, les deux sur `be2655270f30db9a59b7110cad08034faeca48a8` |
| surface de `gh pr list --json` | contient `headRefOid`, `latestReviews`, `reviewDecision`, **`reviews`** |
| appel 2 actuel + `headRefOid,reviewDecision,latestReviews`, `--limit 100` | **1,73 s** (≡ les 1,7 s mesurés par le vol 2a) |
| idem **avec `reviews`** | **2,18 s** ; volumes par PR : 1204, 1541, 3234, 4442, **20030** octets (#2663, 10 revues) |

`latestReviews[].commit.oid` revient **vide**. Il est donc inutilisable pour AC6
(« approbation **sur ce commit** ») et pour la cellule « approuvée sur un commit
antérieur » d'AC7. `reviews` porte `commit.oid`, `state`, `submittedAt`, `author.login`
et le body — tout ce que les deux ACs demandent.

**Décision : enrichir l'appel 2 existant de `read_flight_prs` des champs
`headRefOid,reviewDecision,reviews`.** Zéro appel `gh` supplémentaire, donc le fan-out
reste `2 × |dépôts distincts|` ≤ 8, indépendant du nombre de vols. Coût mesuré : +0,45 s
par dépôt.

`reviewDecision` est pris pour le **rapport seul** (il vaut `REVIEW_REQUIRED` sur #2972,
ce qui est l'information qu'un opérateur lit) ; il ne fonde aucune décision, parce qu'il
ne dit pas sur quel commit.

**Risque nommé, et il a un précédent.** Le vol 2a a mesuré un **504** sur
`--state all --limit 100` avec `statusCheckRollup`. `reviews` ajoute du volume à l'appel
2 (jusqu'à 20 Ko pour une PR à 10 revues). Le volume est borné par la population des **PR
ouvertes** d'un dépôt — 5 mesurées, contre les ~100 que l'appel 1 pagine — et c'est
précisément pourquoi l'appel 2 est déjà restreint à `--state open`. Le fail-soft du vol 2a
couvre la régression sans travail neuf : un appel 2 en échec rend `RepoPrRead::Failed`
**par dépôt**, jamais par cycle, et la cause remonte en `ReadFailed`.

---

## 3. La frontière décision / rapport

Le ticket demande de trancher *« comment une revue `COMMENTED` portant
`VERDICT: hold[...]` se distingue d'un simple commentaire »*.

- **La décision (AC6) ne lit que des champs structurés** : `state == "APPROVED"`,
  `author.login` reconnu comme le relecteur, `commit.oid == headRefOid`. Un `COMMENTED`
  ne vaut **jamais** approbation, quel que soit son texte — donc la décision ne dépend
  d'aucun parsing de texte libre produit par un LLM.
- **Le rapport seul lit le body**, pour en extraire le motif de la ligne
  `VERDICT: hold[<motif>]`. C'est ce qui sépare les deux cellules `hold` d'AC7 (« coupée »
  = motif `review`, « un autre motif » = tout le reste) d'un commentaire nu.

Rationale : faire dépendre une décision de merge du parsing d'un texte libre est fragile ;
un libellé de rapport, non. La distinction qu'AC7 demande est **descriptive**, et les deux
cellules `hold` rendent de toute façon la même recommandation.

**Le login du relecteur ne s'écrit pas ici.** `mika_common::forge_identity::`
`is_reviewer_forge_login` (mika#2667) est le lecteur unique du côté lecture de la
séparation des rôles, et il tolère déjà ce que les surfaces transportent — bords rognés,
`@` de tête, casse, suffixe `[bot]`. Ce vol l'appelle ; il n'écrit pas un huitième
comparateur par login (`builtin_handlers.rs` l. 5313 recense les sept).

---

## 4. Les types

### 4.1 `reader.rs` — le DTO

`FlightPrRead` (l. 947-954) gagne deux champs ; un struct porte la revue retenue :

```rust
pub struct FlightPrRead {
    pub state: String,
    pub ci: Option<CiState>,
    pub head_oid: Option<String>,
    pub review: Option<HeadReview>,
}

/// La **dernière** revue du relecteur attendu, par `submittedAt`.
pub struct HeadReview {
    pub state: String,            // `APPROVED`, `COMMENTED`, `CHANGES_REQUESTED`, …
    pub commit: Option<String>,   // le commit que cette revue visait
    pub submitted_at: String,
    pub hold_reason: Option<String>, // le motif de `VERDICT: hold[<motif>]`, pour le rapport
}
```

`Option` dans le DTO, type de connaissance dans l'évaluateur : **c'est le moule du vol
2a**, pas une exception. `ci: Option<CiState>` y a déjà cette forme, et sa conversion se
fait dans `compose_pr_knowledge`.

`review: None` n'est pas ambigu, pour la même raison que `ci: None` ne l'est pas : la
non-lecture est portée par `RepoPrRead::{Read, Failed}` et par l'absence de clé, pas par
le `None` d'un `FlightPrRead` qui existe. Et la cause est tranchée par l'**ordre de
lecture** que le vol 2a a établi et documenté (`state` avant `ci`) — voir § 5.

### 4.2 Le réducteur

Un seul, au côté de `classify_ci_state` (l. 896-931), qui reste le seul réducteur du
`statusCheckRollup` :

```rust
/// Retient la dernière revue du relecteur attendu, par `submittedAt`.
fn classify_head_review(reviews: &Value) -> Option<HeadReview>
```

Trois règles de lecture, à écrire comme telles :

1. **Filtrer sur `is_reviewer_forge_login(author.login)`.** Une revue d'un autre auteur
   n'entre pas dans la population — ni pour approuver, ni pour tenir « aucune revue ».
2. **Trier par `submittedAt`, ne pas s'appuyer sur l'ordre rendu.** La mesure le rend
   chronologique ascendant, mais rien ne le contractualise.
3. **`hold_reason` vient du body de cette revue-là**, pas d'un balayage des revues. Le
   motif d'un `hold` périmé n'a pas à décrire la tête.

### 4.3 `evaluator.rs` — la connaissance

```rust
/// Les causes de non-lecture de la **tête** sont exactement les mêmes pour la CI
/// et pour la revue : les deux sortent de l'appel 2, joint au même appel 1. Un
/// second énuméré de dix variantes dirait la même chose deux fois.
pub type HeadUnreadCause = CiUnreadCause;

pub enum ReviewKnowledge {
    Read { state: HeadReviewState, at: String },
    Unread(HeadUnreadCause),
}

pub enum HeadReviewState {
    /// Approuvée par le relecteur attendu, **sur la tête exacte**. Le seul état
    /// qui laisse `MergeCandidate`.
    ApprovedOnHead,
    /// Approuvée, mais sur un commit antérieur à la tête.
    ApprovedOnOlderCommit { reviewed: String },
    /// La dernière revue porte un `VERDICT: hold[<motif>]`.
    Held { reason: String, on_head: bool },
    /// La dernière revue n'approuve pas et ne porte pas de verdict moteur.
    NoVerdict { on_head: bool },
    /// Le relecteur attendu n'a posé aucune revue.
    NoReview,
}
```

Même forme que `CiKnowledge` (l. 196-204) : connaissance comme **type**, jamais un
`Option` où deux populations rendraient les mêmes octets (classe mika#2205). Mêmes
attributs serde que ses voisins — `#[serde(rename_all = "snake_case")]`.

**L'alias est sûr côté sérialisation.** `#[serde(rename_all = "snake_case")]` porte sur
les **variantes**, jamais sur le nom du type : le JSON déjà écrit dans `audit_events` par
les vols 1 et 2a se relit à l'identique. Vérifié sur `evaluator.rs` l. 196-296.

`FlightVerdict` gagne un champ `review: ReviewKnowledge`, avec le même défaut serde
partagé que `ci` — `ReviewKnowledge::default_unread()`, `pub(crate)` parce que
`VerdictRecord` (`flight_verdict_ledger.rs`) prend le même.

---

## 5. La composition

`compose_pr_knowledge` (l. 574-614) rend un troisième terme. **Sa chaîne de `let … else`
ne change pas de forme** : les six causes de production (`NoPrNumber`, `NoRepo`,
`NoRunner`, `ReadFailed`, `NotFound`, `PrNotOpen`) sont rendues aux mêmes points, pour les
trois termes à la fois.

```rust
fn compose_pr_knowledge(
    repo: Option<&str>,
    pr_number: Option<u64>,
    pr_reads: &FlightPrReads,
    read_at: &str,
) -> (PrStateKnowledge, CiKnowledge, ReviewKnowledge)
```

La règle **« l'ordre de lecture est porteur »** du vol 2a vaut inchangée, et c'est elle
qui lève l'ambiguïté du § 4.1 : `state` est lu avant `ci` et avant `review`, donc une PR
présente à l'appel 1 mais absente de l'appel 2 sort en `PrNotOpen` sur les trois termes
**avant** qu'un `None` de `review` puisse être lu comme « aucune revue ».

Quand la lecture a abouti, `HeadReview` + `head_oid` donnent l'état :

| `review` | `state` | `commit` vs `head_oid` | `HeadReviewState` |
|---|---|---|---|
| `None` | — | — | `NoReview` |
| `Some` | `APPROVED` | égal | `ApprovedOnHead` |
| `Some` | `APPROVED` | différent ou absent | `ApprovedOnOlderCommit` |
| `Some` | autre | — | `Held` si `hold_reason`, sinon `NoVerdict` |

`head_oid` absent ne peut pas valoir approbation : la comparaison échoue, donc
`ApprovedOnOlderCommit`. La direction sûre.

---

## 6. Le bras de `recommend` et la matrice AC7

`recommend` (l. 701-748) prend un troisième argument. Le bras `PrOpened` s'étend d'une
garde, **après** celle de la CI :

```rust
FlightIssue::PrOpened { .. } => match pr_state {
    PrStateKnowledge::Read(Some(s)) if !s.eq_ignore_ascii_case("open") => ManualResumption,
    _ if matches!(ci, CiKnowledge::Read { state: CiState::Failed, .. }) => ManualResumption,
    _ if !matches!(
        review,
        ReviewKnowledge::Read { state: HeadReviewState::ApprovedOnHead, .. }
    ) => ManualResumption,
    _ => MergeCandidate,
},
```

Les trois décisions que le doc-comment du vol 1 acte restent fermées et ne se rouvrent
pas : `ManualResumption` plutôt qu'une variante neuve (pour ne pas ajouter un terme à
`VerdictTerms` ni au vocabulaire de MPC — mika#2158), `CiState::Pending` reste
`MergeCandidate`, `CiKnowledge::Unread(_)` reste `MergeCandidate`.

**Une revue non lue, elle, ne reste pas `MergeCandidate`** — et c'est délibérément
l'inverse de la politique de la CI. AC6 est écrit en exigence **positive** (« ne rend
`MergeCandidate` **que si** la tête porte une approbation »), là où AC1 est écrit en
exigence négative (« une CI rouge donne autre chose »). Une non-lecture ne porte pas
d'approbation.

**Le coût est réel et doit se dire ici.** Sur un dépôt où le jeton ne lit pas (mika-cloud,
mika-platform — mika#2963) et sur un cycle sans runner câblé (`FlightPrReads::Unwired`),
**tous** les vols sortiront en `ManualResumption`. C'est un sur-refus, assumé : une
approbation QA est le gate de merge, et « ne pas envoyer un humain merger sans
approbation » est la direction sûre, alors qu'une CI non lue n'est qu'un signal manquant.
Le rapport le dit — « revue non lue », avec la cause — donc l'opérateur lit *pourquoi* et
non un refus muet. `MIKA_MANAGER_EVALUATE_WRITE` étant fermé, la concordance est
justement la surface qui doit faire voir ce coût avant l'ouverture.

**La matrice d'AC7**, dans un seul `#[test]`, à CI verte et PR ouverte :

| cellule | `ReviewKnowledge` | attendu |
|---|---|---|
| approuvée sur la tête | `Read { ApprovedOnHead }` | `MergeCandidate` |
| approuvée sur un commit antérieur | `Read { ApprovedOnOlderCommit }` | `ManualResumption` |
| `hold[review]` coupée | `Read { Held { reason: "review" } }` | `ManualResumption` |
| `hold` d'un autre motif | `Read { Held { reason: "ci" } }` | `ManualResumption` |
| aucune revue | `Read { NoReview }` | `ManualResumption` |
| non lue | `Unread(ReadFailed)` | `ManualResumption` |

**La cellule « coupée » part de la mesure, et la mesure doit être figée.** Les données de
mika#2972 : `state: COMMENTED`, `submittedAt: 2026-10-29T22:42:22Z`, `commit.oid:
be2655270f30db9a59b7110cad08034faeca48a8`, body portant `VERDICT: hold[review]`, cause
`cut_off_max_steps`, `reviewDecision: REVIEW_REQUIRED`, `headRefOid: be265527…`.

**Ne pas re-lire #2972 pour construire ce fixture** : une seconde revue `APPROVED` a été
posée le 2026-10-30 à 07:14:30Z sur le même commit, donc la PR ne montre plus l'état
mesuré. Le fixture part des octets ci-dessus, et le test porte en commentaire la date de
la mesure et la raison pour laquelle elle n'est plus re-lisible.

---

## 7. Le rendu

`reporter.rs` gagne un segment de revue, sur le patron exact de `flight_ci_label`
(l. 882-893) : **il ne s'omet jamais**.

```rust
fn flight_review_label(review: &ReviewKnowledge) -> String
```

- `ApprovedOnHead` → « revue : approuvée sur la tête, lue à `<at>` »
- `ApprovedOnOlderCommit { reviewed }` → « revue : approuvée sur `<reviewed[..7]>`, pas sur la tête, lue à `<at>` »
- `Held { reason, on_head }` → « revue : `hold[<reason>]` sur la tête » / « … sur un commit antérieur »
- `NoVerdict { on_head }` → « revue : commentée sans verdict »
- `NoReview` → « revue : aucune revue du relecteur »
- `Unread(cause)` → « revue non lue (`<ci_unread_label(cause)>`) »

Le libellé de non-lecture **réemploie `ci_unread_label`** (l. 905-917) — l'alias de type
du § 4.3 est précisément ce qui le rend possible, et c'est pourquoi ce vol n'écrit aucun
libellé de cause neuf. Ce `match` reste **exhaustif sans bras `_`** (mika#2923) : une
cause ajoutée plus tard devra passer par le compilateur.

AC3 est déjà fermé par le vol 1 pour la CI ; ce segment en est le pendant pour la revue,
et AC6 exige les trois mêmes éléments — état, commit revu, moment de la lecture.

---

## 8. Ordre de commit, et le plancher qui protège d'une mort au plafond

Le vol 2a est mort au plafond (201 tours) sur ce module, et c'est la chaîne de secours wip
(mika#1383) qui a livré son travail. L'ordre ci-dessous fait de ce scénario une **sortie
dégradée nommée**, pas une perte.

1. `reader.rs` — les deux champs `--json`, `head_oid`, `HeadReview`,
   `classify_head_review`, et ses tests sur `RecordingGhRunner` (les arguments `gh` sont
   assertés, comme les 4 cas existants l. 3559-3850).
2. `evaluator.rs` — l'alias, `ReviewKnowledge`, `HeadReviewState`, le champ de
   `FlightVerdict`, `compose_pr_knowledge`, l'essaimage. Le build repasse au vert.
3. `recommend` + la matrice AC7, **sans** la distinction des deux cellules `hold` : elles
   sortent toutes deux en `NoVerdict`. AC6 est entier, AC7 a six cellules dont deux
   indistinctes.
4. `hold_reason` — l'extraction de `VERDICT: hold[<motif>]`, qui sépare les deux cellules.
5. `reporter.rs` — le segment et ses tests.

**Plancher mergeable : les commits 1 à 3.** Ils livrent AC6 dans la décision avec sa
matrice ; un pilote qui meurt là ouvre une PR cohérente, et ce qui manque est le **motif**
d'un `hold` au rapport, c'est-à-dire du descriptif. Si cette sortie se produit, le corps
de PR la nomme et la suite s'écrit `Tracked in: senara-solutions/mika#2968`. En marche
nominale les cinq commits tiennent dans un vol — voir § 11.

---

## 9. La sonde (AC5, AC8) — pas du code

AC5 et AC8 demandent un relevé **après déploiement** : sur les 10 vols suivants qui ouvrent
une PR, le compte de `MergeCandidate` dont la tête portait un check requis rouge (AC5) ou
n'avait pas d'approbation QA (AC8), cible 0 pour les deux. Le vol 2a a posé les trois
règles de lecture de la sonde ; ce vol n'y ajoute qu'un terme, lisible dans le même
`audit_events` grâce au champ `review` :

```sql
SELECT json_extract(payload, '$.recommendation'),
       json_extract(payload, '$.review')
FROM audit_events
WHERE event_type = 'manager_flight_verdict'
  AND json_extract(payload, '$.issue') LIKE '%pr_opened%'
ORDER BY id DESC LIMIT 10;
```

Un `MergeCandidate` dont `review` n'est pas `read/approved_on_head` est une violation
d'AC8. Le relevé s'écrit dans le ticket après déploiement — il n'est pas un livrable de
cette PR, et le plan ne le compte donc pas en lignes.

---

## 10. Definition of Done

- [ ] `read_flight_prs` émet son appel 2 avec `number,statusCheckRollup,headRefOid,reviewDecision,reviews` — **un seul appel de plus qu'avant : aucun**. Le fan-out reste `2 × |dépôts|`, indépendant du nombre de vols, et le fail-soft reste par dépôt.
- [ ] `classify_head_review` est le **seul** lecteur du tableau `reviews`, et il filtre par `is_reviewer_forge_login` (aucun huitième comparateur par login).
- [ ] `recommend` ne rend `MergeCandidate` pour un `PrOpened` ouvert que si CI non-rouge **et** `ReviewKnowledge::Read { ApprovedOnHead }`.
- [ ] La matrice AC7 est dans **un seul** `#[test]`, six cellules, et la cellule « coupée » part des octets mesurés de mika#2972 (§ 6), avec la date de mesure en commentaire.
- [ ] Le segment de revue du rapport ne s'omet jamais, et une revue non lue se dit « non lue » avec sa cause.
- [ ] `VerdictTerms` est **inchangée** (AC4) : aucun terme neuf, `ManualResumption` réemployé.
- [ ] Le JSON d'`audit_events` déjà écrit se relit : l'alias de type ne change aucun nom de variante, et `review` prend un défaut serde.
- [ ] `cadence.rs` n'a **aucune ligne modifiée** (§ 1).
- [ ] `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check`, `scripts/verify-pipeline.sh origin/main`.
- [ ] **Fire-Disposition : N/A.** Ce vol ne livre aucun détecteur au sens de mika#2306 — pas de lint, pas de garde CI, pas de scan structurel, aucun livrable dont le chemin de succès est « aucune violation trouvée ». Les tests de la matrice épinglent un comportement sur des entrées construites ; ils ne balaient aucun corpus. La sonde AC5/AC8 est un relevé d'opérateur, pas du code livré (§ 9). La section n'est donc pas inventée.

---

## 11. Discipline documentaire — le poste qui a fait dérailler le vol 2a

Le corps de la PR mika#2977 le dit de lui-même : *« Environ 529 lignes de production hors
tests, contre environ 220 estimées par le plan. L'écart vient surtout des doc-comments de
`reader.rs` et `evaluator.rs`. »* Le vol 2a a **posé et argumenté le moule** — les six
causes, l'enveloppe `FlightPrReads`, la règle de l'ordre de lecture, le placement de
l'enveloppe dans `evaluator.rs` plutôt que dans `reader.rs`.

Ce vol **hérite** du moule et ne le réargumente pas. Consigne opérationnelle, mesurable à
la relecture : un doc-comment du vol 2b qui aurait à justifier « connaissance comme type »,
« absent ≠ zéro », la forme de l'enveloppe ou le fail-soft par dépôt **renvoie** au
doc-comment correspondant de `CiKnowledge` / `FlightPrReads` au lieu de le redire. Seules
trois choses s'argumentent ici, parce qu'elles sont neuves : le `commit.oid` vide de
`latestReviews` (§ 2), la frontière décision/rapport (§ 3), et l'asymétrie
revue-non-lue / CI-non-lue (§ 6).

---

## 12. Acceptance criteria

Transcrits verbatim du corps du ticket, avec la disposition de chacun.

- **AC1.** Pour un vol `PrOpened` dont la PR est ouverte, une CI rouge sur la tête donne une recommandation autre que `MergeCandidate`. Le plan nomme laquelle : `ManualResumption` ou une variante neuve, avec ce que le rapport et `VerdictTerms` en écrivent.
  → *Fermé par le vol 1 (mika#2971). Non regroomé.*
- **AC2.** Test dans le même appel, sur une matrice : CI verte, rouge, en cours et non lue, chacune croisée avec PR ouverte, puis PR fusionnée ou fermée. Le cas rouge et ouverte part des données mesurées de #2966, pas d'un littéral inventé. Le cas vert et ouverte reste `MergeCandidate`.
  → *Fermé par le vol 1. Ce vol le laisse passant : la matrice CI tourne avec `ReviewKnowledge::Read { ApprovedOnHead }` pour que « vert et ouverte » reste `MergeCandidate`.*
- **AC3.** Le rapport du sink dit, à côté de la recommandation, l'état CI sur lequel elle repose, avec le moment de la lecture. Une CI non lue se dit « non lue ».
  → *Fermé par le vol 1. § 7 en est le pendant pour la revue.*
- **AC4.** Le comparateur de `mika flight-verdicts concordance` n'en est pas changé : la divergence du vol 4f1fe163 reste divergente, et elle est expliquée par le nouveau champ.
  → *Fermé par le vol 1, et tenu ici : `VerdictTerms` est inchangée, aucun terme neuf (§ 6, § 10).*
- **AC5.** Sonde après déploiement : sur les 10 vols suivants qui ouvrent une PR, le compte de verdicts `MergeCandidate` dont la tête portait un check requis rouge au moment du verdict est écrit ici, avec la requête. La cible est 0.
  → *Relevé d'opérateur après déploiement, pas du code. § 9.*
- **AC6.** Pour un vol `PrOpened` dont la PR est ouverte et la CI verte, `recommend` ne rend `MergeCandidate` que si la tête porte une approbation de mika-platform-qa sur ce commit. Le plan nomme la recommandation rendue sinon, et ce que le rapport du sink en écrit à côté (état de la revue, commit revu, moment de la lecture). Une revue non lue se dit « non lue ».
  → **Fermé par ce vol.** Recommandation rendue sinon : `ManualResumption` (§ 6). Rapport : § 7.
- **AC7, contrôle négatif dans le même appel.** Une matrice sur l'état de revue : approuvée sur la tête, approuvée sur un commit antérieur, `hold[review]` coupée, `hold` d'un autre motif, aucune revue, non lue. Avec CI verte et PR ouverte, seule la première cellule reste `MergeCandidate`. Le cas « coupée » part des données mesurées de mika#2972, pas d'un littéral inventé.
  → **Fermé par ce vol.** § 6.
- **AC8.** La sonde AC5 compte aussi, sur ses 10 vols, les `MergeCandidate` dont la tête n'avait pas d'approbation QA au moment du verdict. La cible est 0.
  → *Relevé d'opérateur après déploiement. La requête est écrite au § 9 ; le champ `review` qu'elle lit est livré par ce vol.*

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/reader.rs` (champs `--json`, `head_oid`, `HeadReview`, `classify_head_review`) | 80 |
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (alias, `ReviewKnowledge`, `HeadReviewState`, champ de `FlightVerdict`, `compose_pr_knowledge`, `recommend`) | 95 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (segment de revue) | 32 |
| tests `reader.rs` (extraction : approuvée tête, approuvée antérieure, `hold` mesuré de #2972, commentée sans verdict, aucune revue QA, appel 2 en échec) | 70 |
| tests `evaluator.rs` (matrice AC7, six cellules, plus la non-régression d'AC2) | 70 |
| tests `reporter.rs` (libellés du segment, par état et par cause) | 25 |
| essaimage du champ `review` (`mod.rs`, `types.rs`, `verdict_poster.rs`, `flight_verdict_ledger.rs`) | 25 |

Total estimé : 397 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol, et pourquoi il tient.** 397 < 429 et < 1000 : les deux bornes
passent, la plus serrée mord à 429. La marge est de 32 lignes, et le précédent impose de
dire d'où elle vient, parce que le vol 2a a estimé 405 et réalisé ~529 **en production hors
tests** sur ce même module, puis est mort au plafond à 201 tours.

Trois postes d'économie structurels, chiffrés contre ce précédent :

1. **L'alias `HeadUnreadCause` au lieu d'un énuméré jumeau** (§ 4.3). Un
   `ReviewUnreadCause` à dix variantes aurait coûté ~60 lignes de production, ~30 de tests
   et dix libellés français neufs dans `ci_unread_label`. Il dirait exactement la même
   chose : les deux termes sortent du même appel 2 joint au même appel 1, donc leurs
   causes de non-lecture sont identiques, les six de production comprises. Coût retenu :
   5 lignes. Économie ~90, et zéro risque de sérialisation (§ 4.3).
2. **`is_reviewer_forge_login` réemployé** (§ 3). Le comparateur par login existe,
   canonique depuis mika#2667, et il gère déjà casse, `@` et `[bot]`. Économie ~15 lignes
   de production et ~20 de tests de normalisation.
3. **Le moule n'est pas réargumenté** (§ 11). C'est le poste que la PR mika#2977 désigne
   elle-même comme la cause de son dépassement, et le vol 2b est exactement celui qui peut
   s'en passer, puisque le 2a a posé le moule.

Et si l'estimation dérape malgré tout, le § 8 l'a prévu : le plancher aux commits 1-3 livre
AC6 entier avec sa matrice, et ce qui tomberait est le **motif** d'un `hold` au rapport —
du descriptif, nommé par un `Tracked in:` dans le corps de PR. Le risque de volume est donc
borné en périmètre dégradé, pas en mort sèche.

Aucune suite n'est reportée en marche nominale : ce vol ferme AC6 et AC7, et il est le
dernier de mika#2968 côté code. Ne restent que les relevés d'opérateur AC5 et AC8 (§ 9).
