# mika#2946 — l'évaluateur ne compte plus un groom ni une enveloppe refusée comme un vol mort

| | |
|---|---|
| ticket | `senara-solutions/mika#2946` |
| labels | `bug`, `p1-important`, `dispatch:loop`, `loop-substrate` |
| branche | `fix/2946/mika-manager-l-valuateur-compte-les` |
| base | `origin/main` `e2e9774d` |
| vol | **1 / 2** — AC1, AC2, AC3. AC4 au vol 2 (`Tracked in:` plus bas) |

## Le défaut, mesuré

`mika flight-verdicts concordance`, 2026-10-28 ~12:10Z, 12 lignes sur 90 jours.
**4 seulement sont des vols d'implémentation** ; leurs quatre PR ont été mergées
(#2945, #2938, #2936, #2937) et MPC les a jugées concordantes. Les 8 autres sont
des grooms terminés et des enveloppes `ready-label` refusées au dispatch,
annulées ou remplacées. L'évaluateur les classe toutes en **mort**, et
`recommend` leur attribue « reprise à la main ».

Le point d'entrée est `milestone_manager/evaluator.rs::death_cause` (l. 382-398) :
tout `Outcome:` hors des cinq graphies connues tombe dans
`DeathCause::Unknown(other)` — d'où « mort — cause non classée (PLAN_GROOMED) » —
et un `result` sans ligne `Outcome:` tombe dans `Unknown("no_outcome_line")` —
d'où le classement des JSON `dispatch_no_grooming_marker` et des
`superseded_by_new_dispatch`. En face, `mika flight-verdicts judge --issue`
n'accepte que `pr_opened · rescue_pr · dead`, donc MPC n'a aucun terme pour
contredire ces lignes : `concordance` les saute en « non jugé » et annonce
« 4 vols consécutifs concordants ».

Le coût si le rapport passait en écriture est nommé par le ticket : huit reprises
à la main sur des vols qui n'ont jamais existé. `verdict_poster::post_verdicts`
ne filtre que sur `already_posted` et sur l'absence d'`issue_number` — et les 8
lignes en portent un — donc le bruit atteindrait les tickets eux-mêmes.

## Arbitrage AC1 — le plan tranche par écrit

AC1 offre deux voies. **Le vol prend la seconde : l'évaluateur classe les
non-vols sous une issue distincte qui ne recommande rien.
`finished_implement_flights` continue de les rendre.** Quatre raisons, dans
l'ordre où elles mordent.

1. **Un vol filtré est un silence indistinguable d'un vol absent.** Si la requête
   cesse de rendre ces lignes, « le manager n'a pas vu ce vol » et « le manager
   l'a vu et l'a écarté » rendent les mêmes octets — la classe mika#2205, et le
   motif mika#2331 sur l'absence qui n'est pas un zéro. Le jour où le prédicat se
   trompe sur un **vrai** vol, la ligne manquante n'est lisible nulle part. Une
   issue nommée, elle, se lit dans le rapport shadow.

2. **Le vocabulaire du refus ne doit pas descendre en SQL.** Les trois faits sont
   écrits par trois producteurs uniques : `Outcome: PLAN_GROOMED`
   (`task_state/tasks.rs::GROOM_SUCCESS_MARKER`), `dispatch_no_grooming_marker`
   (`skills/executor.rs:2568`, la porte des marqueurs), `superseded_by_new_dispatch`
   (`task_state/tasks.rs:220`, seul écrivain `db/tasks.rs:1262`). Le doc-comment de
   `list_finished_implement_flights_in_container` a déjà refusé de descendre la
   liste des statuts terminaux en SQL, au motif qu'elle « serait une troisième
   copie de la liste, dans un dialecte où sa règle *un statut inconnu n'est pas
   terminal* ne se lit pas ». La même phrase s'applique mot pour mot à ces trois
   graphies, et la requête porte déjà **trois** filtres Rust-side (population,
   vivacité, contenu) pour cette raison.

3. **AC3 exige que MPC ait un terme pour contredire la ligne** — ce qui suppose
   que la ligne existe. Filtrer ferme AC3 par sa première branche, mais ferme la
   porte à AC4 : plus rien ne peut corriger ce qui est déjà inscrit.

4. **Décisif : le cycle ne repasse jamais sur une tâche déjà vue.**
   `cadence.rs::collect_flight_verdicts` (l. 690) borne la lecture par
   `checkpoint.last_observed_at` — « depuis le cycle précédent ». Les 8 tâches ne
   seront plus jamais rendues à l'évaluateur, quelle que soit la voie choisie.
   Leurs 8 lignes `dead` resteraient donc dans la fenêtre de 90 jours, intactes,
   et l'inscription est idempotente par **lecture** (`write_and_swallow` l. 438 :
   `Ok(_) => InscriptionTally::one_already_recorded()`), jamais par réécriture. Le
   seul remède que le lecteur actuel supporte déjà est une **ré-inscription
   corrective en append** : `read_concordance` (l. 923-925) déclare « si deux
   s'y trouvaient malgré tout, **la plus récente fait foi** ». C'est le vol 2 ;
   la voie du filtrage le rendrait impossible.

**AC3 est donc fermé par sa seconde branche**, cohérente avec AC1 : les termes
`not_a_flight:*` entrent dans le vocabulaire jugeable de `judge --issue`, et une
ligne manager `not_a_flight:groom_completed` contredite par un MPC `dead` rend
`FlightAgreement::Divergent { field: "issue" }`, que `consecutive_concordant`
casse. Mélanger les branches — filtrer en AC1 puis donner un terme en AC3 —
produirait un vocabulaire pour des lignes qui ne s'inscrivent plus.

## Conception

### 1. Le vocabulaire du non-vol, dans `evaluator.rs`

```rust
/// Pourquoi cette tâche terminée n'est pas un vol d'implémentation.
/// Enum FERMÉ : chaque motif correspond à un fait qu'un producteur unique a
/// écrit, pas à une heuristique.
#[derive(…, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotFlightReason {
    /// Un groom qui a convergé — `Outcome: PLAN_GROOMED` en position de ligne.
    GroomCompleted,
    /// Une enveloppe `ready-label` refusée par la porte des marqueurs.
    DispatchRefused,
    /// Une ligne de tracking annulée parce qu'un dispatch frais l'a remplacée.
    Superseded,
}
```

Quatrième variante de `FlightIssue` : `NotAFlight { reason: NotFlightReason }`.
Quatrième variante de `FlightRecommendation` : `Nothing`.

**Pourquoi une variante de recommandation, et non `recommend -> Option<…>`** :
`FlightVerdict.recommendation` est un champ non-optionnel que `VerdictRecord`
sérialise, que `VerdictTerms` traduit, que `parse_judgment_terms` reconstruit et
que le `--recommendation` de clap exige. Un `Option` rippait sur ces cinq sites
et ferait de « rien à recommander » et « recommandation absente » deux lectures
d'un même `null`. Une variante nommée se dit, se juge et se compare.

Le classificateur est une fonction pure appelée **en tête** de `classify_issue`,
avant `death_cause` :

```rust
fn not_a_flight_reason(result: &str, pilot: &PilotFields) -> Option<NotFlightReason>
```

### 2. Les trois faits, et par quel lecteur — la contrainte qui décide de la forme

**`PLAN_GROOMED` est un jeton canonique de classe B** (`scripts/canonical-tokens.tsv`,
l. 279-284), déclaré sur quatre sites dont aucun n'est `evaluator.rs`, et
`canonical_tokens::tests::mika2201_every_match_site_is_declared` rougit sur tout
fichier de production qui en porte le littéral sans ligne au TSV. La doctrine du
module est explicite — « On déclare, on n'allowliste pas », et mika#2201 veut
qu'un site nouveau soit **routé par le lecteur unique**, jamais ajouté à
l'inventaire. Par ailleurs `mika2590_le_marqueur_de_convergence_na_quun_lecteur`
refuse un second `contains` / `instr` / `find` sur `GROOM_SUCCESS_MARKER`.

Conséquence, et elle n'est pas négociable : le classificateur appelle
**`crate::task_state::tasks::groom_result_convergence(result)`** et ne nomme
jamais `PLAN_GROOMED`. Zéro ligne de TSV, zéro exemption, zéro second lecteur.

**Seul `GroomConvergence::Converged` vaut `GroomCompleted`.** Un
`MarkerOutOfPosition` reste classé exactement comme aujourd'hui, et le plan dit
pourquoi : c'est le JSON d'auto-skip de `dispatch-lib.sh`, qui *cite* le marqueur
sans l'ancrer — le texte de 651 caractères que mika#2590 a dû écarter. Élargir le
prédicat à « le marqueur est mentionné » rouvrirait cette classe entière sous
couvert de correctif.

Pour `superseded_by_new_dispatch`, la constante existe :
`task_state::tasks::SUPERSEDED_BY_NEW_DISPATCH`. Le classificateur la lit.

Pour `dispatch_no_grooming_marker`, **il n'y a pas encore de constante** — le
littéral vit à `skills/executor.rs:2568`, dans le JSON que la porte écrit. Le vol
en pose une à côté de ses sœurs, dans la famille de `tasks.result` que
`task_state/tasks.rs` possède déjà (`SUPERSEDED_BY_NEW_DISPATCH`,
`ISSUE_CLOSED_UPSTREAM`, `UPSTREAM_PR_MERGED`, `UPSTREAM_PR_CLOSED_UNMERGED`), et
fait lire la constante au producteur. Une seconde graphie recopiée serait la
divergence programmée de mika#2158 ; `test_dispatch_no_grooming_marker_guard.rs`
reste vert et devient l'attestation que les deux bouts sont la même chaîne.

La lecture du JSON reste une lecture de **champ** — le `"error"` de l'enveloppe —
et non un `contains` sur le corps du `result` : un corps de PR qui citerait le
mot ne doit pas requalifier un vol.

### 3. Ce que le registre apprend sans qu'une ligne du CLI bouge

`flight_verdicts.rs` (mika-cli) ne tient aucune table de graphies — son
doc-module le dit : « une sixième cause de mort ajoutée en amont est acceptée ici
sans qu'une ligne bouge ». **Le vol n'écrit aucune ligne dans `mika-cli`**, et sa
garde `mika2922_ladaptateur_nest_pas_un_ecrivain` reste verte par construction.

Dans `flight_verdict_ledger.rs` :

- `issue_term` gagne un bras rendant trois `&'static str` composés —
  `not_a_flight:groom_completed`, `not_a_flight:dispatch_refused`,
  `not_a_flight:superseded`. La composition par `:` est le précédent de
  `cause_term` (`unknown:{token}`). Le `match` est exhaustif : la quatrième
  variante **ne compile pas** sans son terme.
- `judgeable_issue_terms()` instancie les variantes, donc les trois termes
  deviennent jugeables sans qu'on les y écrive à la main.
- `parse_issue_term` mappe chaque terme vers `{"not_a_flight":{"reason":"…"}}` et
  laisse serde refuser le reste.
- `recommendation_term(Nothing)` → `"nothing"`, et `judgeable_recommendation_terms()`
  l'expose par la même instanciation.
- **`--cause` garde son contrat** (« requis avec `dead`, refusé ailleurs »), donc
  `judgeable_cause_terms()` est intouchée : un `not_a_flight` n'a pas de cause de
  mort, et lui en donner une mettrait deux populations sous un nom (mika#2205).

Aucune table, aucune migration : le schéma reste v56, `after_value` reste le JSON
serde de `VerdictRecord`, et `FlightIssue` est externally-tagged.

`reporter.rs` gagne les libellés français (`flight_issue_label` :
« pas un vol — groom terminé » / « … — dispatch refusé » / « … — remplacé » ;
`flight_recommendation_label(Nothing)` → « aucune recommandation »).

### 4. Le poster ne commente pas un non-vol

`post_verdicts` filtre aujourd'hui sur `already_posted` et sur l'absence
d'`issue_number`. Les 8 lignes portent un `issue_number` : si l'écriture était
armée, le manager commenterait « mort — reprise à la main » sur mika#2943,
mika#2939, mika#2935, mika#2922, mika#2924. Le vol ajoute le troisième filtre —
un verdict qui ne recommande rien n'est pas postable — sous l'event existant
`manager_verdict_not_postable` avec `reason = "not_a_flight"`, parce que c'est le
champ `reason` qui porte la distinction et non un second nom d'event.

## Ordre de commit

1. `task_state/tasks.rs` : la constante `DISPATCH_NO_GROOMING_MARKER` et son
   doc-comment (sole-writer nommé). `skills/executor.rs:2568` la lit.
   → plancher : la suite compile et `test_dispatch_no_grooming_marker_guard.rs`
   est vert.
2. `evaluator.rs` : `NotFlightReason`, `FlightIssue::NotAFlight`,
   `FlightRecommendation::Nothing`, `not_a_flight_reason`, les bras de
   `classify_issue` et de `recommend`. **Le build casse ici** sur les `match`
   exhaustifs de `reporter.rs` et `flight_verdict_ledger.rs` — c'est la propriété
   recherchée, pas un accident.
3. `reporter.rs` + `flight_verdict_ledger.rs` : les termes et les libellés.
   → plancher : `cargo test -p mika-agent` vert, `concordance` rend les nouveaux
   termes.
4. `verdict_poster.rs` : le filtre et sa sonde.
5. Les tests d'AC1, AC2, AC3 et du poster.

Chaque étape laisse l'arbre compilable ; seule l'étape 2 est rouge au milieu, et
l'étape 3 la ferme dans le même commit.

## Contrat de vérification

| ce qui est attesté | où | AC |
|---|---|---|
| groom convergé → `NotAFlight{GroomCompleted}` + `Nothing` | `evaluator.rs § tests` | AC1 |
| JSON `dispatch_no_grooming_marker` → `DispatchRefused` | idem | AC1 |
| `superseded_by_new_dispatch` → `Superseded` | idem | AC1 |
| un `MarkerOutOfPosition` **n'est pas** requalifié | idem | AC1 (non-régression mika#2590) |
| mort au plafond → `Dead{CeilingReached}` + `Regroom` | idem, **même test** | AC2 |
| refus structurel → `Dead{Refusal}` | idem | AC2 |
| `PIPELINE_INCOMPLETE` sans PR → `Dead{PipelineFailure}` | idem | AC2 |
| `PR_OPENED` → `PrOpened` + `MergeCandidate` | idem | AC2 (contrôle positif) |
| les 3 termes sont dans `judgeable_issue_terms()` | `flight_verdict_ledger.rs § tests` | AC3 |
| `parse_judgment_terms` reconstruit chaque variante | idem | AC3 |
| manager `not_a_flight:*` vs MPC `dead` → `Divergent{"issue"}` | idem | AC3 |
| `consecutive_concordant` casse sur cette divergence | idem | AC3 |
| un non-vol n'est pas commenté ; un mort l'est | `verdict_poster.rs § tests` | AC1/AC3 |

Les contrôles négatifs d'AC2 vivent **dans le même test** que les cas d'AC1,
comme le ticket l'exige : un test qui ne verrait que les quatre requalifications
resterait vert si le classificateur avalait tout.

Hors tests : `cargo clippy`, `cargo fmt`, `make verify-bundled-skills` (sans
objet ici, aucun prompt bundlé ne bouge), `scripts/verify-pipeline.sh origin/main`.

## Ce que ce vol ne fait pas, et pourquoi

- **Il ne touche pas `finished_implement_flights`** ni la requête SQL — voir
  l'arbitrage AC1.
- **Il ne corrige pas les 8 lignes déjà inscrites** — vol 2, ci-dessous.
- **Il n'élargit pas le vocabulaire aux trois voisins** `ISSUE_CLOSED_UPSTREAM`,
  `UPSTREAM_PR_MERGED`, `UPSTREAM_PR_CLOSED_UNMERGED`, qui sont eux aussi des
  `result` de lignes de tracking et tombent eux aussi dans
  `Unknown("no_outcome_line")`. Le ticket ne les mesure pas, et classer sans
  mesure c'est deviner. `NotFlightReason` est un enum fermé : un quatrième motif
  coûte une variante, un bras, un terme, et **zéro ligne de CLI**. La sonde d'AC4
  est ce qui dira s'ils apparaissent.
- **Il ne touche pas `consecutive_concordant`** : le `continue` sur
  `NotJudgedByMpc` est juste — une ligne non jugée n'est ni un accord ni un
  désaccord. Ce que le vol change, c'est qu'elle **peut** l'être.

## Découpage en vols

**Vol 1 (ce plan) — AC1, AC2, AC3.** ≈ 394 lignes, sous la borne. Mergeable seul :
après lui, le manager classe correctement, le registre parle les trois termes, MPC
peut contredire et le poster se taît sur les non-vols. Le compte de concordance
cesse de produire de nouveaux faux morts.

**Vol 2 — AC4, la réconciliation des lignes déjà inscrites.** `Tracked in:
senara-solutions/mika#2946` (vol 2/2). Son périmètre, déjà borné par les faits
établis ici : les 8 lignes `dead` sont dans `audit_events`, le cycle ne les
reverra pas (`checkpoint.last_observed_at`) et `write_and_swallow` refuserait une
seconde inscription dans la fenêtre. Le vol 2 livre donc un chemin d'écriture
**corrective et append-only**, distinct de l'inscription de cycle, déclenché par
un geste opérateur borné à une fenêtre, qui ré-évalue les tâches de cette fenêtre
et pose une ligne plus récente là où l'issue a changé. Le lecteur n'a besoin
d'aucune modification : `read_concordance` déclare déjà que la plus récente fait
foi. **Ni suppression de ligne, ni migration de purge, ni table de
reclassification au moment de la lecture** (mika#2158) — les trois formes qui
rendraient le registre non auditable.

## Definition of Done

- Les quatre populations qu'AC1 nomme sont classées `NotAFlight{reason}` avec
  `FlightRecommendation::Nothing`, par lecture des producteurs uniques et via le
  lecteur unique de `GROOM_SUCCESS_MARKER`.
- `finished_implement_flights` et sa requête sont inchangées.
- Les trois termes `not_a_flight:*` et le terme `nothing` sont acceptés par
  `mika flight-verdicts judge`, sans qu'une ligne de `mika-cli` ait bougé.
- `post_verdicts` ne commente pas un verdict qui ne recommande rien.
- `mika2201_every_match_site_is_declared`, `mika2590_le_marqueur_de_convergence_na_quun_lecteur`,
  `mika2922_ladaptateur_nest_pas_un_ecrivain` et le scan de
  `milestone_manager/no_dispatch_test.rs` restent verts, **sans nouvelle entrée
  d'allowlist ni ligne de TSV**.
- Schéma inchangé (v56), aucune migration.
- `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt --check` et
  `scripts/verify-pipeline.sh origin/main` passent.
- AC4 est renvoyé au vol 2 par la ligne `Tracked in:` ci-dessus, avec son
  périmètre écrit.

## Acceptance criteria

- **AC1.** Un groom terminé (`Outcome: PLAN_GROOMED`), une enveloppe `ready-label`
  refusée au dispatch (résultat `dispatch_no_grooming_marker`), annulée ou
  remplacée (`superseded_by_new_dispatch`) ne sont pas des vols terminés. Soit
  `finished_implement_flights` ne les rend pas, soit l'évaluateur les classe sous
  une issue distincte, qui ne recommande rien. Le plan tranche par écrit.
- **AC2.** Contrôle négatif dans le même test. Un vol d'implémentation mort au
  plafond, sur refus structurel ou en échec de pipeline reste classé en mort, avec
  sa cause.
- **AC3.** `concordance` ne laisse plus une ligne du manager sans contrepartie
  hors du compte. Soit cette ligne disparaît (AC1), soit MPC dispose d'un terme
  pour la contredire, et elle compte alors comme désaccord.
- **AC4.** Sonde après déploiement : relire `mika flight-verdicts concordance` sur
  la même fenêtre. Les 8 lignes ci-dessus ne figurent plus en mort.

## Fire-Disposition

Le vol livre des détecteurs : les tests d'AC1/AC2/AC3 et le test du poster, dont
le chemin de succès est « aucune mauvaise classification trouvée ».

**Option retenue : (a) exception nommée en allowlist — et ce vol n'en consomme
aucune.** Le détail d'implémentation est la raison du zéro :

- Les détecteurs atterrissent **armés et verts**, parce que le correctif et eux
  sont dans le même commit (étapes 2-5 de l'ordre de commit). Donc ni (b) livrer
  désarmé — un `#[ignore]` ici serait un test qui ne mesure rien — ni (c)
  halte-et-remontée : rien n'est à cadrer, le ticket a déjà cadré.
- Les quatre détecteurs **préexistants** que le vol approche sont satisfaits par
  construction, pas par exemption : `mika2201_every_match_site_is_declared` et
  `mika2590_le_marqueur_de_convergence_na_quun_lecteur` parce que le
  classificateur passe par `groom_result_convergence` et ne nomme jamais
  `PLAN_GROOMED` ; `mika2922_ladaptateur_nest_pas_un_ecrivain` parce qu'aucune
  ligne de `mika-cli` ne bouge ; le scan de `no_dispatch_test.rs` parce que les
  trois graphies lues ne sont pas des jetons d'autorité d'écriture. **Aucune
  ligne de `scripts/canonical-tokens.tsv`, aucune entrée d'allowlist, aucune
  entrée dans `EXEMPTED_OCCURRENCES`.** C'est la discipline mika#2201 — on route
  par le lecteur unique, on n'ajoute pas une ligne — et mika#2323 : une allowlist
  née non vide est l'emplacement où déposer la prochaine infraction.
- Les 8 lignes déjà inscrites **ne sont pas une violation que ces détecteurs
  voient** : elles vivent dans `audit_events`, pas dans l'arbre. C'est
  exactement pourquoi elles sont le vol 2 et non une exception de ce vol — une
  exception les aurait rendues invisibles au lieu de les rendre dues.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (production) | 60 |
| `crates/mika-agent/src/flight_verdict_ledger.rs` (production) | 32 |
| `crates/mika-agent/src/milestone_manager/verdict_poster.rs` (production) | 16 |
| `crates/mika-agent/src/task_state/tasks.rs` (constante + doc) | 12 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (libellés) | 11 |
| `crates/mika-agent/src/skills/executor.rs` (lecture de la constante) | 3 |
| `crates/mika-cli/src/commands/flight_verdicts.rs` | 0 |
| tests AC1 (`evaluator.rs § tests`) | 95 |
| tests AC2, contrôles négatifs et positif (même module) | 55 |
| tests AC3 (`flight_verdict_ledger.rs § tests`) | 70 |
| tests du poster (`verdict_poster.rs § tests`) | 40 |

Total estimé : 394 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Le vol courant tient sous la borne, et pour une raison structurelle plutôt que
par chance : les trois sites en aval de l'enum — `reporter.rs`,
`flight_verdict_ledger.rs`, `mika-cli` — ne tiennent aucune table de graphies, de
sorte qu'une quatrième variante leur coûte respectivement 11, 32 et **0** lignes.
La part qui reste est celle des tests, et elle est irréductible : AC2 exige ses
contrôles négatifs dans le même test qu'AC1.

AC4 est au-dessus de ce qui reste sous la borne et est renvoyé au vol 2 par
`Tracked in: senara-solutions/mika#2946` (vol 2/2), dont le périmètre est écrit
au § *Découpage en vols*.
