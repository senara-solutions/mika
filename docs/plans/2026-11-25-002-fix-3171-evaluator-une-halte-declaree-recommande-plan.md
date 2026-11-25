# mika#3171 — une halte déclarée recommande `Regroom`, pas `ManualResumption`

Suivi nommé par mika#3170 (vol 2b de mika#3142), hors de sa borne par vol. Ce
plan exécute ce que la section « Ce que ce vol ne corrige PAS, et qui se lit en
aval » du plan `2026-11-25-001-fix-3142-…` avait mesuré et renvoyé :

> « La correction juste est une branche de `death_cause` sur le motif
> `pilot_halt_declared`, dans `evaluator.rs`, avec sa suite : hors périmètre de
> ce vol et de sa borne. »

Un seul vol. Le périmètre est une variante d'énumération et sa lecture en aval.

---

## Ce qui est déjà en place, et ne doit pas être refait

Trois moitiés sur quatre existent et sont attestées. **Ce vol n'en réécrit
aucune.**

1. **L'émetteur.** `skills/bundled/_shared/dispatch-lib.sh:15018-15031` — le
   troisième refus structurel du bloc Unit 2. Sur une halte déclarée il pose
   `NO_PR: pilot_halt_declared` et
   `Outcome: PIPELINE_INCOMPLETE — pilot_halt_declared: the pilot declared a
   per-flight-bound halt; the perimeter needs splitting before another flight.`
   Il est seul des trois refus à poser une ligne `Outcome:`, et son en-tête dit
   pourquoi (l'alternation d'`empty_completion` à `:7254`, et la réécriture de
   `:15283` qui n'aura pas lieu). La **poussée n'est pas retenue** — décision
   rendue le 2026-11-25 — donc la branche existe et le périmètre est
   récupérable.

2. **Le détecteur.** `_pilot_declared_halt` (`:8238-8289`), armé, attesté par
   `skills/bundled/_shared/tests/test_pilot_halt_declared.sh` (cas W5f, W5g).

3. **Le lecteur de motif.** `dispatcher.rs::parse_no_pr_reason` (`:6892`),
   `pub(crate)`, ancré en début de ligne (`(?m)^NO_PR:\s+([a-z_]+)`), avec son
   énumération nominative en doc-commentaire (`:6866-6896`) qui nomme déjà
   `pilot_halt_declared` et explique pourquoi il n'appartient PAS à la famille
   `pilot_terminated_*` : cette famille dit que le **substrat** a arrêté la
   session, tandis qu'une halte déclarée arrive avec `STATUS=success` et dit que
   le **pilote** a mesuré son périmètre contre la borne de mika#2693 et a
   décliné. Quatre tests le tiennent (`:8286-8332`).

**Ce qui manque est la quatrième moitié : la lecture en aval.**

---

## Le défaut qui reste ouvert

Le chemin complet, mesuré sur le code tel qu'il est :

1. Un vol halté n'a ni ligne `PR:` ni jeton `https://` dans son RESULT, donc
   `pr_reference` (`evaluator.rs:1090-1106`) rend `(None, None)`.
2. `classify_issue` (`:976-995`) ne prend donc pas le bras de sauvetage
   mika#1282 (`Some("PIPELINE_INCOMPLETE") if number.is_some() || url.is_some()`)
   et tombe sur `_ => Dead { cause: death_cause(result, outcome) }`.
   `not_a_flight_reason` ne lit aucun `NO_PR:` et ne l'intercepte pas.
3. `outcome` vaut `Some("PIPELINE_INCOMPLETE")` : `line_value(result, "Outcome:")`
   puis `.split_whitespace().next()` (`:983`) coupe avant le tiret cadratin.
4. `death_cause` (`:1070-1087`) n'a pas de branche sur le motif ; son
   `Some("PIPELINE_INCOMPLETE") => DeathCause::PipelineFailure` mord.
5. `recommend` (`:955-966`) rend `FlightRecommendation::ManualResumption`, dont
   le commentaire au site dit : *« le ticket n'est pas en cause, le substrat
   l'est. Un humain regarde avant de re-groomer. »*

Sur une halte déclarée, chacun de ces deux mots est faux. Le substrat n'est pas
en cause : il a fonctionné, c'est lui qui a produit le refus. Le ticket est en
cause — il est trop gros pour la borne. Et « un humain regarde avant de
re-groomer » envoie un opérateur lire un vol dont le RESULT dit déjà, en quatre
lignes de prose, ce qu'il faut faire : *« regroom the ticket into flights that
fit the bound »*.

La recommandation juste est `Regroom`, comme pour `CeilingReached`, `Refusal` et
`Escalated` — dont le commentaire du site dit l'exact contraire et dit juste :
*« le vol dit quelque chose sur le ticket lui-même — trop gros, mal posé, ou
hors périmètre »*.

---

## La décision de forme : une variante dédiée, pas un rattachement

L'AC1 laisse deux formes ouvertes : « variante de `DeathCause` dédiée, ou
rattachement nommé à une variante qui rend `Regroom` ». **Ce plan prend la
variante dédiée**, `DeathCause::HaltDeclared`, et voici le prix des deux.

| | variante dédiée `HaltDeclared` | rattachement à `CeilingReached` |
|---|---|---|
| lignes de production | ~20 | ~6 |
| sites d'énumération à amender | 3 (`reporter`, `cause_term`, `judgeable_cause_terms`) | 0 |
| ce que le registre de verdicts rend | `halt_declared`, saisissable par MPC | `ceiling_reached` |
| ce que l'opérateur lit en rapport | « mort — halte de borne déclarée » | « mort au plafond (budget_exhausted) » |

Le rattachement est moins cher et **il mentirait sur l'instrument**. Les deux
populations ne sont pas la même : au plafond, le pilote est mort en travaillant
et son travail est tronqué ; sur une halte déclarée, il a mesuré avant de
commencer et il a obéi — `STATUS=success`, et le travail qui existe est
intentionnellement en deçà. Les confondre les rendrait non soustractibles sous
un seul nom (mika#2205) précisément dans le registre que mika#2698 lit pour
mesurer la concordance manager/MPC, et le libellé français de `reporter.rs`
dirait « budget_exhausted » d'un vol qui n'a épuisé aucun budget.

Les deux formes rendent `Regroom` ; seule la première reste lisible. Le surcoût
est de ~14 lignes et de trois amendements d'énumération, tous mécaniques, tous
déjà cartographiés ci-dessous. **Ce n'est pas une décision réservée à
l'opérateur** : l'AC1 autorise explicitement les deux et le ticket ne réserve
rien — le plan tranche et dit son prix.

---

## Ce que livre ce vol

### 1. La variante, dans `DeathCause`

`crates/mika-agent/src/milestone_manager/evaluator.rs:104-119`. Une septième
variante, posée **avant** `Unknown(String)` (qui reste en queue, c'est l'ordre
que lit un futur éditeur) :

```rust
/// `NO_PR: pilot_halt_declared` — le pilote a mesuré son périmètre
/// contre la borne de mika#2693 et a décliné de livrer.
///
/// Distincte de `CeilingReached` **par décision** (mika#3171) : au
/// plafond le pilote meurt en travaillant et son travail est tronqué ;
/// ici il a mesuré avant de commencer, il a obéi, et `STATUS=success`.
/// Les confondre rendrait les deux populations non soustractibles sous
/// un seul nom (mika#2205) dans le registre que mika#2698 lit.
HaltDeclared,
```

Sérialisée `halt_declared` par le `#[serde(rename_all = "snake_case")]` déjà
porté par l'enum — aucune table de graphies à tenir (mika#2201).

### 2. La branche de `death_cause`, et son RANG

`:1070-1087`. La branche est posée **avant le `match outcome`**, entre le test
de `STRUCTURAL VIOLATION:` et lui :

```rust
// mika#3171 — AVANT le `match outcome`, et c'est l'AC2. Une halte
// déclarée porte `Outcome: PIPELINE_INCOMPLETE`, donc une branche
// posée dans le `match` ci-dessous serait inatteignable : le bras
// `Some("PIPELINE_INCOMPLETE")` mordrait d'abord.
if parse_no_pr_reason(result).as_deref() == Some(NO_PR_PILOT_HALT_DECLARED) {
    return DeathCause::HaltDeclared;
}
```

Deux propriétés, et elles sont le livrable plutôt que des détails :

- **L'ordre de spécificité est tenu STRUCTURELLEMENT, pas par un test.** La
  branche est un `return` en amont du `match`, donc l'AC2 n'est pas une
  convention qu'une relecture doit vérifier : un `PIPELINE_INCOMPLETE` qui ne
  porte pas le motif ne peut pas atteindre `HaltDeclared`, et un
  `pilot_halt_declared` ne peut pas atteindre `PipelineFailure`. C'est la forme
  que `classify_issue` emploie déjà en tête pour mika#2946 (`:977-982`), et pour
  la même raison : *« `death_cause` a un bras attrape-tout, donc tout test placé
  après lui est inatteignable »*.
- **La lecture passe par le lecteur unique, jamais par un `contains`.**
  `crate::task_engine::dispatcher::parse_no_pr_reason` est ancré en début de
  ligne ; un `result.contains("pilot_halt_declared")` serait un second lecteur du
  même fait et mordrait sur la prose du RESULT (qui contient déjà `mika#3142`,
  `halt` et `bound` dans ses quatre lignes). Doctrine mika#2201 : router par le
  lecteur qui existe, ne pas ajouter une ligne. Le lecteur est `pub(crate)` et
  l'évaluateur est dans le même crate — aucun changement de visibilité.

`NO_PR_PILOT_HALT_DECLARED` est une `pub(crate) const` posée **à côté de
`parse_no_pr_reason`**, dans `dispatcher.rs`, parce que c'est là que vit
l'énumération nominative des motifs. Le littéral reste écrit en clair dans le
doc-commentaire et dans le corps de `test_parse_no_pr_reason_known_reasons` : la
garde shell de mika#3038 (`test_rescue_tree_equals_main.sh:421-425`) exige deux
occurrences nominatives d'un motif dans `dispatcher.rs` — elle est ancrée sur
`rescue_adds_nothing_to_main` et non sur celui-ci, mais la propriété qu'elle
défend (*« un motif que le parseur accepte mais qu'aucune liste ne nomme est un
motif que personne ne peut chercher »*) vaut autant ici, donc la const **ajoute**
une troisième occurrence au lieu de remplacer l'une des deux.

### 3. Le bras de `recommend`

`:955-966`. `HaltDeclared` rejoint le premier bras, celui qui rend `Regroom`, et
le commentaire au site gagne la phrase qui la justifie :

```rust
// Plafond, refus, halte déclarée, escalade : le vol dit quelque chose
// sur le ticket lui-même — trop gros, mal posé, ou hors périmètre.
// Re-groom. mika#3171 : la halte déclarée est le cas où le vol le dit
// EXPLICITEMENT — son RESULT porte déjà « regroom the ticket into
// flights that fit the bound ».
DeathCause::CeilingReached
| DeathCause::Refusal
| DeathCause::HaltDeclared
| DeathCause::Escalated => FlightRecommendation::Regroom,
```

Le `match` est exhaustif sans `_`, donc la variante ne compile pas sans ce bras.

### 4. Les trois énumérations en aval, dans le MÊME commit

Le rayon de souffle est entièrement **dans `mika-agent`** : un balayage de
`crates/mika-cli/`, `dashboard/src/`, `docs/`, `skills/` et `scripts/` sur les
graphies des causes ne rend que de la documentation (notamment
`docs/plans/2026-10-27-002-feat-2922-…:147`, qui liste les valeurs de `--cause`).
Aucune union TypeScript, aucune liste de littéraux côté CLI : `mika
flight-verdicts judge --cause` est un `Option<String>` libre, sans `requires`
clap et **sans `ValueEnum`**, délibérément — le contrat vit dans
`parse_judgment_terms`.

| site | le geste | pourquoi il est obligatoire |
|---|---|---|
| `reporter.rs::flight_issue_label` (`:1138-1145`) | un bras `HaltDeclared => "mort — halte de borne déclarée"` | le `match` est exhaustif ; et c'est la seule surface française de la cause |
| `flight_verdict_ledger.rs::cause_term` (`:387-396`) | un bras rendant le littéral serde `"halt_declared"` | exhaustif aussi |
| `flight_verdict_ledger.rs::judgeable_cause_terms` (`:631-652`) | une sixième entrée, et le doc passe de « les cinq causes **classées** » à six | c'est la liste offerte à MPC sur `--cause` ; une cause absente d'ici n'est pas saisissable par l'opérateur |

`parse_cause_term` (`:686-756`) **ne bouge pas**, et son doc-commentaire dit
déjà pourquoi : il est l'inverse exact de serde, écrit pour qu'*« une sixième
cause de mort ajoutée à `DeathCause` soit acceptée sans qu'une ligne bouge
ici »*. C'est la doctrine mika#2201 / anti-mika#2158 qui paie ici, et ce vol est
sa première mesure réelle : il ne doit y toucher sous aucun prétexte.

---

## Tests

### T1 (AC3) — le rouge-avant, sur la forme RÉELLE qu'émet `dispatch-lib.sh`

Nouveau test dans `evaluator.rs`, module `tests`, à côté de
`mika2698_chaque_forme_du_producteur_a_sa_cause_de_mort` (`:1613-1654`). Son
fixture est **recopié de `dispatch-lib.sh:15024-15030`**, les quatre lignes de
prose comprises, parce que ce sont elles qui portent les pièges :

```text
claude-pilot completed.
Turns: 184
Draft PR withheld (mika#3142): the pilot declared a per-flight-bound halt as its last word, so it chose not to ship this perimeter — the absent shipping tail is a decision, not a truncated run.
Nothing was lost — branch fix/9999/x is pushed.
The perimeter needs splitting before another flight: regroom the ticket into flights that fit the bound.
To publish it anyway: gh pr create --repo senara-solutions/mika --head fix/9999/x --base main --draft
NO_PR: pilot_halt_declared
Outcome: PIPELINE_INCOMPLETE — pilot_halt_declared: the pilot declared a per-flight-bound halt; the perimeter needs splitting before another flight.
```

Quatre assertions sur ce seul fixture, et chacune ferme une panne distincte :

1. `classify_issue` rend `Dead { cause: HaltDeclared }` — et **pas**
   `RescuePr` : le RESULT contient `gh pr create --repo …`, donc ce test est
   aussi le témoin que `pr_reference` ne scrape pas un numéro dans la ligne de
   suggestion.
2. `recommend` rend `Regroom`.
3. `death_cause` seul rend `HaltDeclared` — l'unité, pour que la panne se
   localise sans déboguer `classify_issue`.
4. Le fixture est bien celui du producteur : un commentaire nomme
   `dispatch-lib.sh:15024-15030`. Ce test est **rouge avant** le vol (il rend
   `PipelineFailure` / `ManualResumption`) et c'est ce qui l'atteste comme
   voyant.

### T2 (AC3) — le contre-témoin, dans la table qui existe déjà

La table de contrôles négatifs de mika#2946 (`:1779-1812`) est une liste de
triplets `(result, cause, recommandation)`. Elle gagne **deux** lignes :

- un `Outcome: PIPELINE_INCOMPLETE` **sans** ligne `NO_PR:` →
  `PipelineFailure` / `ManualResumption` (la population ordinaire, qui ne doit
  pas basculer) ;
- un `Outcome: PIPELINE_INCOMPLETE` portant `NO_PR: rescue_scratch_only` →
  `PipelineFailure` / `ManualResumption` (un **autre** motif de non-vol : ce qui
  sépare « la branche lit le bon motif » de « la branche tire sur toute ligne
  `NO_PR:` »). Sans cette seconde ligne, un `parse_no_pr_reason(result).is_some()`
  fautif serait vert.

La table est le bon site plutôt qu'un test neuf : elle est déjà le lieu où les
trois causes de mort voisines sont tenues côte à côte, et l'ajout y est lu comme
une ligne de plus dans une matrice, non comme une exception.

### T3 — l'extension des tables d'exhaustivité

Quatre amendements, tous mécaniques, tous **dans le même commit** que la
variante — un `match` exhaustif les exige de toute façon pour trois d'entre eux :

| test | geste |
|---|---|
| `evaluator.rs::mika2698_chaque_forme_du_producteur_a_sa_cause_de_mort` (`:1613`) | une neuvième assertion sur la forme haltée |
| `evaluator.rs::mika2698_la_matrice_de_recommandation_est_exhaustive` (`:1888`) | les quatre lignes `dead(HaltDeclared)` × `PrStateKnowledge` |
| `reporter.rs::mika2698_chaque_cause_et_chaque_recommandation_a_son_libelle` (`:1427`) | une septième ligne, et le doc passe de « Les six causes de mort » à sept |
| `flight_verdict_ledger.rs::mika2922_lanalyse_des_termes_est_linverse_de_serde` (`:1861`) | `assert_eq!(analyses, 40)` → `44`, **et son message réécrit** |

Ce dernier mérite son détail, parce que le nombre nu ne suffit pas. Le message
actuel dit : *« 6 issues … dont la seule `dead` porte 5 causes, × 4
recommandations : (5 × 4) + (5 × 1 × 4) = 40 jugements saisissables ; un autre
compte veut dire que l'énumération a bougé »*. Avec une sixième cause classée le
compte devient `(6 × 4) + (5 × 1 × 4) = 44`, et **la dérivation doit être
réécrite avec le nouveau facteur** : laisser le message parler de « 5 causes »
devant un `44` ferait d'une assertion de recensement une assertion dont personne
ne peut recalculer le nombre.

`mika2922_un_jugement_ne_porte_pas_dinconnu` (`:1955-1977`) ne bouge pas :
`HaltDeclared` est une variante unitaire, donc classée, et `Unknown(String)`
reste structurellement hors de `judgeable_cause_terms` (un variant à charge
sérialise en objet, pas en littéral).

### T4 — le motif est nommé une troisième fois dans `dispatcher.rs`

`test_parse_no_pr_reason_known_reasons` (`:8286-8307`) garde son littéral
`"pilot_halt_declared"` à `:8301` **inchangé**, et une assertion s'ajoute :
`NO_PR_PILOT_HALT_DECLARED` vaut ce littéral. C'est ce qui empêche la const et
l'énumération nominative de diverger — la classe `grooming_marker` (mika#2158) —
sans retirer aucune des deux occurrences que la garde shell de mika#3038 compte.

---

## Ce que ce vol ne corrige PAS

- **`STRUCTURAL VIOLATION:` reste réservé à une violation d'autorité.** Le plan
  de mika#3170 l'avait déjà dit dans ces termes : *« L'écrire là serait un
  mensonge de token pour obtenir une recommandation »*. Une halte déclarée est
  l'exact contraire d'une violation d'autorité — c'est une **obéissance** à la
  borne. Le test de `death_cause` sur ce jeton (`:1072`) n'est ni déplacé ni
  élargi, et aucune ligne de ce vol n'écrit ce jeton nulle part. C'est l'item
  « Hors périmètre » du ticket, tenu par abstention et non par une garde : il n'y
  a rien à garder, puisque rien ne l'écrit.
- **Le seuil de concordance de mika#2698 n'est pas re-mesuré.** Ajouter une
  sixième cause saisissable change le dénominateur de
  `mika flight-verdicts concordance` pour les vols futurs, jamais pour les vols
  déjà jugés : `parse_cause_term` accepte la nouvelle graphie sans migration et
  les lignes anciennes gardent la leur. Aucun recomptage, aucune réécriture
  d'audit.
- **Les surfaces de documentation ne sont pas synchronisées.**
  `docs/plans/2026-10-27-002-feat-2922-…:147` liste les valeurs de `--cause` dans
  un plan **historique** : un plan daté décrit l'état au jour de sa rédaction, et
  le réécrire falsifierait un artefact d'archive.

---

## Ordre de commit

Un seul vol, mergeable seul. **Deux commits**, et le compte est une contrainte
du compilateur, pas une préférence de style.

1. **`fix(3171): une halte déclarée est sa propre cause de mort`** — la variante,
   la const, la branche de `death_cause`, le bras de `recommend`, les trois
   énumérations en aval, et T1 + T2 + T3 + T4. **Indivisible**, et c'est mesuré :
   trois `match` exhaustifs refusent de compiler sans leur bras, et une assertion
   de recensement rougit dès que la variante existe. Un commit de test seul
   serait donc *non compilable*, pas rouge — et un commit qui ne compile pas
   n'atteste rien.
2. **`docs(3171): les doc-commentaires d'énumération comptent juste`** — les
   doc-commentaires de `judgeable_cause_terms` (« cinq » → « six ») et de
   `flight_issue_label` (« six » → « sept »), si le commit 1 ne les a pas déjà
   emportés par proximité.

**La mesure rouge-avant est donc portée par le corps de PR, pas par un commit.**
C'est ce que l'AC3 demande et c'est reproductible : avant tout changement, les
deux fixtures de T1 et de T2 passés à `death_cause` / `recommend` rendent
`PipelineFailure` / `ManualResumption`. Le corps de PR rapporte cette sortie
observée, à l'appui, et nomme `evaluator.rs:1080` comme le bras qui la produit.

Le plancher mergeable est le commit 1. Il n'y a pas de suite : rien n'est renvoyé
à un `Tracked in:`.

---

## Risques

| risque | mesure |
|---|---|
| La const casse la garde shell de mika#3038 qui compte les occurrences nominatives dans `dispatcher.rs` | Elle est ancrée sur `rescue_adds_nothing_to_main`, pas sur ce motif — vérifié à `test_rescue_tree_equals_main.sh:421-425`. La const **ajoute** une occurrence sans retirer ni le doc ni le littéral du test. `bash skills/bundled/_shared/tests/test_rescue_tree_equals_main.sh` avant commit. |
| Une graphie de cause est tenue hors de `mika-agent` | Balayé : `crates/mika-cli/`, `dashboard/src/`, `docs/`, `skills/`, `scripts/` ne rendent que de la documentation. `--cause` est un `Option<String>` libre côté CLI, sans `ValueEnum` — délibérément (`crates/mika-cli/CLAUDE.md` § `judge`). |
| Un futur éditeur déplace la branche dans le `match outcome` et la rend inatteignable | **C'est T1 qui rougit** sur ce déplacement — le contre-témoin T2 resterait vert, puisqu'il mesure la population qui ne doit PAS basculer. Les deux ne sont donc pas redondants : T1 garde le rang, T2 garde la portée. Le commentaire au site dit l'inatteignabilité en une phrase, pour que la cause se lise sans déboguer. |
| `assert_eq!(analyses, 44)` passe avec un message qui dit encore « 5 causes » | Le message est réécrit avec sa dérivation — c'est un livrable nommé de T3, pas un détail de rédaction. |

---

## Definition of Done

- `DeathCause::HaltDeclared` existe, sérialise `halt_declared`, et son
  doc-commentaire nomme mika#3171 et la raison de sa distinction d'avec
  `CeilingReached`.
- `death_cause` rend `HaltDeclared` sur un RESULT portant
  `NO_PR: pilot_halt_declared`, par une branche posée **avant** le
  `match outcome`, et par un appel à `dispatcher::parse_no_pr_reason` et non par
  un `contains`.
- `recommend` rend `Regroom` sur `HaltDeclared`, dans le bras qui porte déjà
  `CeilingReached | Refusal | Escalated`.
- Les trois énumérations en aval sont amendées dans le même commit :
  `reporter.rs::flight_issue_label`, `flight_verdict_ledger.rs::cause_term`,
  `flight_verdict_ledger.rs::judgeable_cause_terms`. `parse_cause_term` n'a pas
  bougé.
- `assert_eq!(analyses, 44)` avec son message réécrit et sa dérivation
  recalculable.
- T1 existe, son fixture est recopié de `dispatch-lib.sh:15024-15030` avec un
  commentaire qui nomme ces lignes, et le corps de PR rapporte la mesure
  rouge-avant (`PipelineFailure` / `ManualResumption`).
- T2 porte **deux** contre-témoins : un `PIPELINE_INCOMPLETE` sans `NO_PR:`, et
  un avec un autre motif.
- Aucune ligne de ce vol n'écrit `STRUCTURAL VIOLATION:`.
- `cargo test -p mika-agent`, `cargo clippy --all-targets`, `cargo fmt --check`,
  `make verify-bundled-skills`, `bash scripts/verify-pipeline.sh origin/main`.
- `bash skills/bundled/_shared/tests/test_rescue_tree_equals_main.sh` et
  `bash skills/bundled/_shared/tests/test_pilot_halt_declared.sh` restent verts
  (aucune ligne de `dispatch-lib.sh` ne bouge, et c'est ce qu'ils mesurent).

## Acceptance criteria

Transcrits du corps de senara-solutions/mika#3171, § « Ce qui est demandé » :

- AC1 : un vol dont le RESULT porte `NO_PR: pilot_halt_declared` est classé à
  part (variante de `DeathCause` dédiée, ou rattachement nommé à une variante
  qui rend `Regroom`) et recommande `Regroom`.
- AC2 : l'ordre de spécificité de `death_cause` est tenu. La halte déclarée est
  testée avant la branche `PIPELINE_INCOMPLETE`, et un `PIPELINE_INCOMPLETE`
  sans ce motif rend toujours `PipelineFailure` / `ManualResumption`.
- AC3 : un test rouge-avant sur un RESULT réel de vol halté (forme émise par
  `dispatch-lib.sh`, refus Unit 2), plus un contre-témoin : un
  `PIPELINE_INCOMPLETE` ordinaire ne bascule pas.
- Hors périmètre : `STRUCTURAL VIOLATION:` reste réservé à une violation
  d'autorité. Une halte déclarée en est le contraire et ne doit pas emprunter ce
  jeton.

## Fire-Disposition

Ce vol livre des détecteurs, donc la porte s'applique. **Option retenue : (a)
exception nommée en allowlist — avec un ensemble d'exceptions VIDE, et ce vide
est le livrable mesuré.** Détail d'implémentation, détecteur par détecteur :

- **T1 et T2** sont des assertions de valeur sur un comportement **neuf**.
  Elles atterrissent **armées**, et leur population de violations
  préexistantes est vide par construction : avant le vol elles rougissent sur
  le comportement que le vol corrige (c'est ce qui les atteste comme voyantes),
  après le vol elles sont vertes. Il n'y a donc **rien à excepter, rien à
  désarmer et rien à remonter** — aucune entrée d'allowlist n'est créée, et
  aucun `#[ignore]` n'est posé.
- **Les deux détecteurs de recensement que ce vol amende** —
  `flight_verdict_ledger.rs::mika2922_lanalyse_des_termes_est_linverse_de_serde`
  (`analyses == 40`) et
  `reporter.rs::mika2698_chaque_cause_et_chaque_recommandation_a_son_libelle`
  — sont les seuls dont le chemin de succès est littéralement « l'énumération
  n'a pas bougé », donc les seuls qui portent un axe de violation préexistante.
  Ils rougissent **par construction** dès que la variante est ajoutée, et le
  geste prescrit est l'**amendement de leur compte à sa valeur exacte** (44 et
  sept lignes), **dans le commit 2, avec la dérivation réécrite**. Aucune
  entrée d'exemption, aucun `#[allow]`, aucun élargissement d'aiguille : le
  compte reste exact, donc le détecteur reste voyant. Un recensement exempté
  serait précisément la fausse exactitude que mika#2309 condamne.
- **L'assertion auto-nettoyante** qu'exige l'option (a) est le compte lui-même :
  il rougit au prochain mouvement de l'énumération, donc l'exception ne peut pas
  devenir stale sans se signaler — c'est le mécanisme, pas une promesse.

Ce vol n'ajoute **aucun** scan de source, aucune règle de lint, aucune garde CI
et aucune garde EndTurn. Le ticket de suivi qu'exige l'option (a) est donc sans
objet : il n'y a aucune violation reportée.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (variante + branche + bras + commentaires) | 24 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` (const + sa ligne de doc) | 6 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (bras de libellé + doc) | 4 |
| `crates/mika-agent/src/flight_verdict_ledger.rs` (`cause_term` + `judgeable_cause_terms` + docs) | 9 |
| tests — T1, le rouge-avant sur le RESULT réel (`evaluator.rs`) | 48 |
| tests — T2, les deux contre-témoins dans la table mika#2946 (`evaluator.rs`) | 8 |
| tests — T3, les quatre tables d'exhaustivité (`evaluator.rs`, `reporter.rs`, `flight_verdict_ledger.rs`) | 30 |
| tests — T4, la const nommée une troisième fois (`dispatcher.rs`) | 5 |

Total estimé : 134 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 134 lignes estimées contre une
borne par vol de 429 et un seuil par PR de 1000 : les deux bornes passent avec
une marge de plus du triple sur la plus serrée. Le périmètre est une variante
d'énumération dont le rayon de souffle est **entièrement cartographié et
entièrement dans un seul crate** — quatre fichiers, trois `match` exhaustifs que
le compilateur désigne un par un, et une seule assertion de recensement dont la
nouvelle valeur est déjà calculée ici (44). Il n'y a ni migration de schéma, ni
surface HTTP, ni consommateur hors crate, ni fixture à bâtir : le fixture de T1
est une recopie de huit lignes depuis `dispatch-lib.sh:15024-15030`, et les trois
moitiés amont du mécanisme (émetteur, détecteur, lecteur de motif) existent déjà
et sont attestées. La fin de pipeline est l'ordinaire — suite, clippy, fmt,
`verify-pipeline.sh`, deux suites shell qui doivent rester vertes par
abstention — et les 30 tours que la dérivation lui réserve la couvrent.
