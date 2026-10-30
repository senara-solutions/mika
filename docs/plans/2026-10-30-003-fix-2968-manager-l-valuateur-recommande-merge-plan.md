# mika#2968 — vol 2a : la CI de la tête est lue par numéro de PR, et l'état de PR avec elle

> **Vol 2a de 3.** Le vol 1 (`docs/plans/2026-10-29-005-fix-2968-manager-l-valuateur-recommande-merge-plan.md`, PR #2971, mergée le 2026-10-30 à 07:00:01Z) a fait **entrer** la CI dans `recommend`. Il ne l'a pas fait **lire** pour les vols de la boucle. Ce vol la fait lire, par le numéro de PR du vol, et fait lire `pr_state` par la même porte.
>
> Le vol 1 n'est pas regroomé. La revue de la tête (AC6, AC7, AC8) part au vol 2b — voir `Tracked in:` au § 11.

## 1. Ce que le vol 1 a laissé, et ce que ce vol prend

Le vol 1 a livré, et c'est en place sur cette branche :

- `CiKnowledge::{Read { state, at }, Unread(CiUnreadCause)}` et `CiUnreadCause::{NotFromPrList, NoSubIssue, NoIssueNumber, NotMeasured}` (`evaluator.rs`, l. 147-200) — « lue » et « non lue » sont une distinction de **type**, jamais un `Option`.
- Le champ `#[serde(default = "CiKnowledge::default_unread")] pub ci: CiKnowledge` sur `FlightVerdict`, de sorte qu'un blob antérieur se relit `Unread(NotMeasured)` et jamais comme un état (motif mika#2340).
- Le bras AC1 de `recommend` (`evaluator.rs`, l. 497-520) : une CI **lue** et rouge sur une PR encore ouverte rend `ManualResumption`. `Pending` et `Unread(_)` restent `MergeCandidate`, pour les raisons que le doc-comment porte.
- Le segment CI du rapport (`reporter.rs`, `flight_ci_label` l. 874, `ci_unread_label` l. 891), qui ne s'omet jamais.
- `VerdictTerms` inchangé (AC4) : la CI n'est pas dans la struct comparée.

Ce que le vol 1 a **déféré**, et que ce vol prend intégralement :

1. **Le lecteur par numéro de PR.** Aujourd'hui `evaluate_flights` (`evaluator.rs`, l. 379-416) ne pose `CiKnowledge::Read` que lorsque la sous-issue appariée porte `pr_link_source == Some(PrLinkSource::MilestonePrList)`. Or les PR de la boucle ne portent pas le milestone (mesure du ticket, 2026-10-29 : cinq PR sur cinq sans milestone). Pour tout vol de la boucle, la CI est donc `Unread(NotFromPrList)` — honnête, et inutile.
2. **`pr_state` lu par la même requête**, au lieu de l'appariement par sous-issue du milestone.
3. **Le découplage de `flight_milestone_label`**, précondition du point 2 (§ 6).
4. **La sonde AC5** (§ 10).

## 2. Mesures du 2026-10-30 (ce qui décide la forme de la requête)

Toutes contre `senara-solutions/mika`, jeton de cette session.

| requête | durée | rendu |
|---|---|---|
| `gh pr list --state all --json number,state,headRefOid,statusCheckRollup --limit 100` | **11,3 s puis `HTTP 504: Gateway Timeout` sur `api.github.com/graphql`** | rien |
| idem, `--limit 20` | 4,2 s | 20 PR, 44 checks chacune |
| idem, `--state open --limit 100` | **1,7 s** | **6 PR** |
| `gh pr list --state all --json number,state --limit 100` (sans rollup) | **0,6 s** | 100 PR |
| `gh pr view <n> --json number,state,headRefOid,statusCheckRollup` | 0,9 s | 1 PR |

Trois lectures de ce tableau, et elles décident tout le § 4 :

- **C'est `statusCheckRollup` qui coûte**, pas la pagination : la même page de 100 sans lui tient en 0,6 s, avec lui elle meurt en 504. La forme que le vol 1 avait esquissée — une requête par dépôt, `--state all --limit 100`, rollup compris — **n'est pas exécutable**. Ce n'est pas une préférence : elle a été lancée et elle a rendu un 504.
- **Rabattre la limite n'est pas une réparation.** `--limit 20` passe, mais 20 est une fenêtre **glissante** : une PR ouverte avant-hier en sort, et le lecteur rendrait « PR introuvable » pour une PR qui existe. Une borne qui dépend de l'activité du dépôt plutôt que de la question posée est une borne qui ment un jour sur deux.
- **La population qui a besoin du rollup est exactement celle des PR ouvertes.** `recommend` ne rend `MergeCandidate` que sur une PR ouverte ; sur toute autre, le bras `pr_state` tranche **avant** le bras CI (`evaluator.rs`, l. 500-507) et la CI n'entre pas dans la décision. Lire le rollup des PR fermées, c'est payer un 504 pour une donnée que personne ne consulte.

D'où la coupe du § 4 : **deux requêtes par dépôt**, chacune bornée par ce qu'elle doit rendre.

Quatrième mesure, qui tranche le point 2 du ticket (« requis ou tous ? ») :

```
$ gh pr view 2975 --repo senara-solutions/mika --json statusCheckRollup --jq '.statusCheckRollup[0] | keys'
["__typename","completedAt","conclusion","detailsUrl","name","startedAt","status","workflowName"]
```

**La projection `gh` du rollup ne porte aucune marque « requis ».** Le choix « requis ou tous » n'en est donc pas un à ce niveau : la donnée disponible est l'ensemble complet. `classify_ci_state` (`reader.rs`, l. 896) réduit cet ensemble, et ce vol le réutilise **tel quel** — lecteur unique du rollup (mika#2201), aucun second site de classification. Le coût est nommé plutôt que tu : un check **non requis** rouge rend `CiState::Failed`, donc `ManualResumption` sur une PR que le gate de merge aurait acceptée. C'est un sur-refus, pas un sous-refus : il envoie un humain regarder une PR mergeable, il n'envoie jamais merger une PR rouge. Pour la mesure du jour, la tête de #2975 porte 44 checks dont 28 requis — le sur-ensemble est réel, et sa direction est la bonne.

## 3. La contrainte de périmètre, tranchée **en tête** comme elle l'exige

La contrainte du 2026-10-30 demande, avant toute implémentation : *« Si le lecteur par numéro de PR doit être injecté dans le cycle, le plan dit comment il y parvient sans toucher `CycleContext`, ou il dit en tête pourquoi il ne le peut pas. »*

**Il le peut, et sans rien ajouter : `gh_runner` est déjà un champ de `CycleContext`.**

Mesuré sur cette branche :

- `cadence.rs` l. 529 : `pub gh_runner: Option<&'a dyn GhRunner>,` — champ existant, posé par mika#2698 vol 2.
- `cadence.rs` l. 568 : `with_gh_runner` — builder existant.
- `cadence.rs` l. 1228 et 1234 : câblage de production existant (`let gh_runner = ProcessGhRunner::new(cfg.github_token.clone());` puis `.with_gh_runner(Some(&gh_runner));`).
- `cadence.rs` l. 792 et l. 860 : deux consommateurs existants, tous deux sur la forme `let Some(runner) = ctx.gh_runner else { … };`.

Donc le lecteur de ce vol s'appelle **depuis le corps de `collect_flight_verdicts`** (l. 682-708) sur `ctx.gh_runner`, et il en découle quatre non-touchés, chacun nommé par la contrainte :

| zone nommée par la contrainte | touchée ? |
|---|---|
| `CycleContext` (l. 500) | **non** — aucun champ ajouté |
| les builders `with_*` (l. 558-570) | **non** — aucun builder ajouté |
| `write_pool_admissions` (l. 836) et son appel dans `run_manager_cycle_in` (l. 891-947) | **non** — la signature de `collect_flight_verdicts` ne change pas, donc son site d'appel l. 927 reste identique octet pour octet |
| `spawn.rs`, `groom_trigger.rs`, `server/mod.rs` l. 1727-1810, le `pub use groom_trigger::{…}` de `mod.rs`, `evidence/guards.rs`, `skills/builtin_handlers.rs` | **non** — aucun n'est lu par ce vol |

Ce que ce vol change dans `cadence.rs` : **le seul corps de `collect_flight_verdicts`**, entre la ligne du `source.finished_implement_flights(since)` et la ligne de l'appel `evaluate_flights` (l. 698), plus cette ligne d'appel elle-même. Rien d'autre dans ce fichier. Les deux travaux en vol (mika#2974 et le vol 2b de mika#2958) n'y touchent pas.

`reader.rs` est libre depuis le merge de mika#2972 (2026-10-30, 08:21:55Z, `1685810a`).

## 4. Le lecteur : `read_flight_prs`

Nouveau dans `reader.rs`, à côté de `read_timeline_probe` (l. 369) dont il copie la discipline : borné, fail-soft, sans jamais faire mourir le cycle.

```rust
/// Ce qu'une lecture de PR a rendu pour un dépôt donné.
pub(crate) struct FlightPrRead {
    /// `OPEN` / `MERGED` / `CLOSED`, tel que `gh` l'écrit.
    pub state: String,
    /// `Some` seulement pour une PR **ouverte** : le rollup n'est demandé que
    /// pour elles (§ 2). `None` sur une PR fermée n'est pas « pas de check ».
    pub ci: Option<CiState>,
}

/// L'issue de la lecture d'**un dépôt**. Trois populations, trois variantes :
/// elles n'appellent pas la même action, et un `HashMap` vide les confondrait.
pub(crate) enum RepoPrRead {
    Read(HashMap<u64, FlightPrRead>),
    Failed,
}

pub(crate) async fn read_flight_prs(
    runner: &dyn GhRunner,
    repos: &BTreeSet<String>,
) -> HashMap<String, RepoPrRead>
```

**Deux appels `gh` par dépôt**, dans cet ordre :

1. `pr list --repo <R> --state all --json number,state --limit 100` — l'état de PR. Mesuré 0,6 s.
2. `pr list --repo <R> --state open --json number,statusCheckRollup --limit 100` — le rollup des seules PR ouvertes. Mesuré 1,7 s pour 6 PR.

La deuxième est **jointe** à la première par le numéro : une PR dont l'appel 1 dit `OPEN` reçoit son `ci` de l'appel 2 ; une PR dont l'appel 1 dit `MERGED` ou `CLOSED` garde `ci: None`.

**Ce qui borne la lecture, et ce qui ne la borne pas.** Le fan-out est `2 × |repos|`, où `repos` est l'ensemble des `FinishedFlight::repo` **distincts** du cycle — donc au plus `controlled_repos().len()` = 4, soit 8 appels et ~9 s dans le pire cas. Il est **indépendant du nombre de vols** : c'est précisément l'objection que le vol 1 avait posée contre une lecture par vol, et au premier cycle (pas de checkpoint, `finished_implement_flights(None)` rend tous les vols terminés) elle mord. Les dépôts ne sont pas écrits ici : ils viennent de `flight.repo` (garde `no_repo_list_test.rs`), jamais d'un littéral.

**Fail-soft, par dépôt.** Un `GhCommandError` sur l'appel 1 rend `RepoPrRead::Failed` pour ce dépôt, WARN nommé, et les autres dépôts sont lus quand même. Un échec de l'appel 2 seul rend `Read(map)` avec tous les `ci: None` — l'état de PR reste connu, la CI ne l'est pas, et les deux se disent séparément. Le 504 mesuré au § 2 et le jeton sans droit de mika#2963 sur `mika-cloud` / `mika-platform` sont le **même chemin**, et ce chemin est mesuré, pas théorique.

**Un seul lecteur par prédicat.** `classify_ci_state` reste le seul site qui lit un `statusCheckRollup`. `read_flight_prs` l'appelle ; il n'en écrit pas une seconde version.

## 5. Les causes de non-lecture, nommées

`CiUnreadCause` gagne six variantes. Le `match` de `ci_unread_label` (`reporter.rs`, l. 891) est exhaustif sans bras `_` (mika#2923), donc **une variante sans libellé ne compile pas**.

| variante | ce qui l'a produite | libellé du rapport | ce que l'opérateur en fait |
|---|---|---|---|
| `NoPrNumber` | le vol n'est pas un `PrOpened`/`RescuePr`, ou son numéro est inconnu | « ce vol n'a pas de PR » | rien — cas nominal d'un vol mort |
| `NoRepo` | le vol est un `PrOpened` mais `FlightVerdict::repo` est `None` | « vol sans dépôt » | défaut d'instrumentation (mika#2921) |
| `NoRunner` | `ctx.gh_runner` absent | « aucun lecteur gh câblé » | binaire ou test non câblé |
| `ReadFailed` | la requête `gh` a échoué pour ce dépôt | « lecture gh en échec » | jeton (mika#2963) ou 504 |
| `NotFound` | le dépôt a été lu, le numéro n'est pas dans la page | « PR hors de la page lue » | plus de 100 PR depuis l'ouverture |
| `PrNotOpen` | la PR existe et n'est pas ouverte | « PR non ouverte » | rien — `pr_state` tranche déjà |

Les quatre variantes du vol 1 sont **conservées et cessent d'être produites** : `NotFromPrList`, `NoSubIssue`, `NoIssueNumber` sortent du chemin de production parce que la CI ne passe plus par les sous-issues, et `NotMeasured` reste le défaut de désérialisation. Elles ne sont pas supprimées, parce qu'elles sont **sérialisées dans `audit_events`** : retirer une variante ferait échouer la relecture de tout verdict écrit entre le 2026-10-30 07:00Z et ce vol. Leur doc-comment porte la date à partir de laquelle plus aucune ne s'écrit — c'est ce qui rend leur réapparition lisible comme une régression plutôt que comme un cas normal.

La composition vit dans `evaluate_flights`, comme pour le vol 1 : `evaluate_flight` et `recommend` **reçoivent** la `CiKnowledge`, ils ne la fabriquent pas. `recommend` n'est pas touché par ce vol — AC1 est déjà fermé, et son bras est insensible à la provenance.

## 6. `pr_state` lu directement, et le découplage de `flight_milestone_label`

**Pourquoi l'un exige l'autre, et dans cet ordre.** Aujourd'hui `flight_milestone_label` (`reporter.rs`, l. 842) lit `PrStateKnowledge` pour répondre à une **autre** question :

```rust
fn flight_milestone_label(pr_state: &PrStateKnowledge) -> &'static str {
    match pr_state {
        PrStateKnowledge::Read(_) => "milestone cible",
        PrStateKnowledge::NotRead => "hors milestone cible",
    }
}
```

Son doc-comment dit pourquoi c'était juste : *« la connaissance de l'état de PR est celle de l'appartenance »*, parce que l'unique source de `pr_state` était l'appariement aux sous-issues du milestone cible. Faire lire `pr_state` par `read_flight_prs` **casse cette identité** : un vol hors milestone cible aurait `Read(Some("OPEN"))`, et le rapport écrirait « milestone cible » pour un ticket qui n'y est pas. C'est pourquoi le découplage se fait **avant**, dans le même vol et dans l'ordre de commit du § 8.

**Le champ.** `FlightVerdict` gagne un axe explicite :

```rust
/// L'appartenance du ticket au milestone cible — lue par appariement avec
/// `state.sub_issues`, et seulement par là (mika#2968, vol 2a).
///
/// Cet axe était porté par `PrStateKnowledge` tant que l'appariement était
/// l'unique source de `pr_state`. Depuis que la PR est lue par son numéro, les
/// deux questions ont deux sources : les porter sur un champ ferait dire
/// « milestone cible » à un vol de la boucle qui n'y est pas.
#[serde(default)]
pub in_target_milestone: MilestoneMembership,

pub enum MilestoneMembership {
    InTarget,
    OutOfTarget,
    /// Défaut de désérialisation : un blob antérieur à ce vol ne porte pas le
    /// champ, donc l'appartenance n'y est pas écrite. « Non lue » n'est pas
    /// « hors cible » (motif mika#2331).
    #[default]
    NotRead,
}
```

`flight_milestone_label` prend ce champ et rend trois libellés : « milestone cible », « hors milestone cible », « appartenance au milestone non lue ». Le troisième n'est produit que par relecture d'un blob du vol 1 ou antérieur — `evaluate_flights` lit toujours `state.sub_issues`, donc il écrit toujours l'une des deux premières.

**La source de `pr_state`.** Pour un vol `PrOpened { number } | RescuePr { number }` dont le dépôt a été lu :

| lecture | `pr_state` |
|---|---|
| numéro trouvé | `Read(Some(state))` — `OPEN` / `MERGED` / `CLOSED` tel que `gh` l'écrit |
| dépôt lu, numéro absent de la page | `NotRead` |
| dépôt en échec, pas de runner, vol sans dépôt ou sans numéro | `NotRead` |

`PrStateKnowledge` n'est **pas** modifié : ses deux variantes et leur sémantique « lu / non lu » conviennent telles quelles, et `flight_pr_state_label` (l. 850) n'est pas touché. L'appariement aux sous-issues cesse d'alimenter `pr_state` et n'alimente plus que `in_target_milestone` — un lecteur, une question (mika#2201).

**Ce que ce changement ferme.** Le bras *« fusionnée ou fermée pendant que le vol volait »* de `recommend` (l. 500-507) ne tirait jamais pour un vol de la boucle, puisque `pr_state` y valait toujours `NotRead`. Il tire maintenant. Une PR de la boucle déjà fermée cesse de sortir en `MergeCandidate`.

## 7. Ce que ce vol ne change pas

- **`recommend`** — AC1 est fermé par le vol 1, et son bras est insensible à la provenance de la `CiKnowledge`. Zéro ligne.
- **`VerdictTerms` / `first_divergence` / `consecutive_concordant`** (`flight_verdict_ledger.rs`) — AC4. La divergence du vol `4f1fe163` reste divergente ; elle gagne seulement une explication au rapport.
- **`SubIssue::ci_state` et `is_hard_blocked`** — un autre consommateur les lit, sur une autre question (le blocage dur d'une sous-issue du milestone). Ce vol ne leur retire rien ; il cesse seulement de s'en servir pour la CI d'un vol.
- **Les sept appels `gh` de `Reader::read`** (l. 188-368), dont l'appel 3 (la liste du milestone, l. 242-257). Le nouveau lecteur est un huitième chemin, séparé, appelé depuis le cycle et non depuis `read()`.
- **`MIKA_MANAGER_EVALUATE_WRITE`** — reste fermé. Son ouverture revient à Vincent.
- **Le lecteur CI du moteur et son jeton** (mika#2963) — hors périmètre.

## 8. Ordre de commit

Chaque commit compile et laisse la suite verte.

1. **`reporter.rs` + `evaluator.rs` — le découplage seul.** `MilestoneMembership`, le champ `in_target_milestone`, son écriture dans `evaluate_flights` depuis l'appariement existant, `flight_milestone_label` sur le nouveau champ, l'essaimage littéral dans les tests. Comportement **inchangé** : `InTarget` ⟺ l'ancien `Read(_)`. C'est le plancher — il rend le § 6 possible sans rien casser, et il est mergeable seul.
2. **`reader.rs` — `read_flight_prs`.** Le type, les deux requêtes, la jointure, le fail-soft par dépôt, ses tests sur `RecordingGhRunner` (l. 2270). Aucun appelant encore : le lecteur est écrit et testé avant d'être branché.
3. **`evaluator.rs` — les six causes et la nouvelle composition.** Signature de `evaluate_flights` étendue d'un paramètre `pr_reads`, composition de `ci` et de `pr_state` depuis lui, doc des quatre variantes héritées. Les tests de la matrice.
4. **`cadence.rs` — le branchement.** Collecte des dépôts depuis `flights`, appel de `read_flight_prs` sur `ctx.gh_runner`, ligne 698. Quatre lignes de corps et une ligne d'appel.
5. **`reporter.rs` — les six libellés** et le test AC3 qui les épingle.

**Plancher mergeable : les commits 1 à 3.** Ils livrent le découplage et le lecteur testé ; le manager écrit alors exactement ce qu'il écrit aujourd'hui. Si le vol meurt au plafond après le commit 3, la PR est utile et sans régression.

## 9. Vérification

- `cargo test -p mika-agent milestone_manager` — la matrice du vol 1 (`evaluator.rs`, l. 1562-1655) doit rester verte telle quelle, l'essaimage littéral mis à part.
- `cargo clippy --all-targets -- -D warnings` et `cargo fmt --check`.
- `scripts/verify-pipeline.sh origin/main`.
- La suite complète : `cargo test`.

Tests neufs, par critère :

- **AC2 (étendu)** — la matrice existante gagne les six causes de non-lecture, chacune croisée avec PR ouverte / fusionnée / fermée. Le cas rouge-et-ouverte continue de partir des données mesurées de #2966 (vol 1), pas d'un littéral inventé.
- **AC3** — `render_flight_verdict` porte le bon libellé pour chacune des six causes et pour chacune des trois appartenances.
- **Le lecteur** — quatre cas sur `RecordingGhRunner` : nominal (une PR ouverte avec rollup rouge, une mergée sans rollup), échec de l'appel 1 (`Failed` pour ce dépôt, les autres lus), échec du seul appel 2 (états connus, `ci: None` partout), numéro absent de la page (`NotFound`). Les arguments `gh` passés sont assertés — c'est ce qui épingle `--state open` sur l'appel 2, donc la mesure du § 2.
- **Contrôle négatif** — un vol dont le dépôt n'a pas été lu ne doit **jamais** sortir avec `CiKnowledge::Read`. C'est la propriété que tout le ticket existe pour tenir.

## 10. La sonde AC5 (post-déploiement)

À exécuter après `make deploy`, sur les **10 vols suivants** qui ouvrent une PR :

```sql
SELECT json_extract(payload,'$.issue_number')   AS ticket,
       json_extract(payload,'$.recommendation') AS recommandation,
       json_extract(payload,'$.ci')             AS ci,
       json_extract(payload,'$.pr_state')       AS pr_state,
       created_at
FROM audit_events
WHERE event_type = 'manager_flight_verdict'
  AND json_extract(payload,'$.issue.type') = 'pr_opened'
ORDER BY created_at DESC
LIMIT 10;
```

Cible : **zéro** `merge_candidate` dont la tête portait un check rouge au moment du verdict.

**Trois règles de lecture, sans lesquelles la sonde ment.**

- **Lire la cible avec son dénominateur.** « Zéro merge_candidat sur CI rouge » est atteint trivialement si rien n'est lu. Le chiffre à écrire sous AC5 est donc une **paire** : le compte visé, et le nombre de `ci` en `read` parmi les 10. Un numérateur à 0 sur un dénominateur à 0 ne ferme rien.
- **Un champ `ci` absent n'est pas une CI absente** : c'est un binaire antérieur au vol 1 (motif mika#2340). Il se compte **à part** de `unread`, jamais avec.
- **Depuis ce vol, `unread` porte une cause, et les causes ne se somment pas.** `ReadFailed` sur `mika-cloud` dit « le jeton » (mika#2963) ; `NotFound` dit « la page de 100 est trop courte » ; `PrNotOpen` est nominal et ne compte pas comme un manque. Les regrouper reconstituerait les deux populations sous un nom que `CiUnreadCause` existe pour séparer.

Le résultat se dépose en commentaire sur mika#2968 **après** déploiement. Ce vol n'écrit pas la mesure, il écrit la requête et sa règle de lecture.

## 11. Hors périmètre, et la suite

Hors périmètre, nommé :

- Le lecteur CI du moteur et son jeton (mika#2963).
- L'ouverture de `MIKA_MANAGER_EVALUATE_WRITE` — décision de Vincent.
- La révision d'un verdict déjà écrit. Point 1 du ticket, tranché par le vol 1 et inchangé : un rouge posé **après** le verdict ne le fait pas réviser ; l'`at` de `CiKnowledge::Read` est ce qui rend la mesure honnête.
- Les quatre zones de `cadence.rs`, `spawn.rs`, `groom_trigger.rs`, `server/mod.rs`, `mod.rs`, `evidence/guards.rs` et `skills/builtin_handlers.rs` nommées par la contrainte du 2026-10-30 (§ 3).

**Tracked in: senara-solutions/mika#2968 — vol 2b, la revue de la tête (AC6, AC7, AC8).** Ce qu'il livre, et pourquoi il est séparé :

- `ReviewKnowledge` / `ReviewUnreadCause`, de la même forme que `CiKnowledge` et pour la même raison.
- Deux champs `--json` de plus sur l'appel 2 de `read_flight_prs` — `latestReviews` et `reviewDecision`, vérifiés présents dans la surface `gh pr list --json` — donc **aucun appel supplémentaire** : le vol 2a est ce qui rend le vol 2b bon marché, et c'est ce qui justifie l'ordre.
- Un bras de `recommend` : CI verte et PR ouverte ne rendent `MergeCandidate` que si la tête porte une approbation de mika-platform-qa **sur ce commit** — d'où le besoin de `headRefOid`, qui entre au vol 2b.
- La distinction d'une revue `COMMENTED` portant `VERDICT: hold[...]` d'un simple commentaire, à partir des données mesurées de mika#2972 (tête `be265527`, revue `COMMENTED` à 22:42:22Z, cause `cut_off_max_steps`, `reviewDecision: REVIEW_REQUIRED`).
- La matrice AC7 (six cellules) et le second compteur de la sonde (AC8).

Il est séparé parce que la somme des deux moitiés est estimée à ~630 lignes, au-dessus de la borne par vol. La coupe est faite sur l'axe causal, pas au milieu d'un livrable : le vol 2a ferme « la tête est-elle verte ? », le vol 2b ferme « la tête est-elle approuvée ? ». Chacun est mergeable seul, chacun améliore la recommandation de son côté.

## Definition of Done

- Pour un vol `PrOpened` de la boucle — PR sans milestone — `CiKnowledge::Read` est posée depuis une lecture du `statusCheckRollup` de **cette** PR, par son numéro, et `pr_state` depuis le `state` de la même lecture.
- Les six causes de non-lecture sont produites chacune par son chemin, portent chacune un libellé français distinct au rapport, et le `match` qui les rend est exhaustif sans bras `_`.
- Aucun chemin ne produit `CiKnowledge::Read` sans qu'une requête `gh` ait rendu un rollup pour ce numéro de PR.
- `flight_milestone_label` lit `in_target_milestone` et ne lit plus `pr_state` ; un vol de la boucle hors milestone cible est rendu « hors milestone cible » tout en portant un `pr_state` lu.
- Un blob antérieur se relit sans erreur : `ci` → `Unread(NotMeasured)`, `in_target_milestone` → `NotRead`.
- `VerdictTerms` est inchangé, et le test qui le dit est vert.
- Le fan-out `gh` du cycle est borné par `2 × |dépôts distincts des vols|` ≤ 8, indépendamment du nombre de vols.
- Un échec de lecture sur un dépôt n'empêche pas la lecture des autres et ne fait pas mourir le cycle.
- `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check` et `scripts/verify-pipeline.sh origin/main` passent.
- La requête de la sonde AC5 et sa règle de lecture sont écrites (§ 10) ; son résultat est déposé après déploiement.

## Acceptance criteria

Transcrits du corps de mika#2968.

- **AC1.** *Pour un vol `PrOpened` dont la PR est ouverte, une CI rouge sur la tête donne une recommandation autre que `MergeCandidate`. Le plan nomme laquelle : `ManualResumption` ou une variante neuve, avec ce que le rapport et `VerdictTerms` en écrivent.*
  → **Fermé par le vol 1** (`ManualResumption`, bras à `evaluator.rs` l. 500-520). Ce vol le rend **atteignable** pour les vols de la boucle, qui jusqu'ici n'avaient jamais de CI lue. Non rouvert.
- **AC2.** *Test dans le même appel, sur une matrice : CI verte, rouge, en cours et non lue, chacune croisée avec PR ouverte, puis PR fusionnée ou fermée. Le cas rouge et ouverte part des données mesurées de #2966, pas d'un littéral inventé. Le cas vert et ouverte reste `MergeCandidate`.*
  → **Fermé par le vol 1, étendu ici** : la matrice existante (l. 1562-1655) gagne les six causes de non-lecture du § 5, croisées avec les trois états de PR. Le cas rouge-et-ouverte garde sa provenance mesurée.
- **AC3.** *Le rapport du sink dit, à côté de la recommandation, l'état CI sur lequel elle repose, avec le moment de la lecture. Une CI non lue se dit « non lue ».*
  → **Fermé par le vol 1, étendu ici** : six libellés de plus (§ 5), plus les trois libellés d'appartenance au milestone (§ 6). Le segment CI continue de ne jamais s'omettre.
- **AC4.** *Le comparateur de `mika flight-verdicts concordance` n'en est pas changé : la divergence du vol 4f1fe163 reste divergente, et elle est expliquée par le nouveau champ.*
  → **Fermé par le vol 1, non rouvert** (§ 7). Ce vol n'ajoute rien à `VerdictTerms`.
- **AC5.** *Sonde après déploiement : sur les 10 vols suivants qui ouvrent une PR, le compte de verdicts `MergeCandidate` dont la tête portait un check requis rouge au moment du verdict est écrit ici, avec la requête. La cible est 0.*
  → **Fermé par ce vol** pour la requête et sa règle de lecture (§ 10) ; le chiffre se dépose après déploiement.
- **AC6.** *Pour un vol `PrOpened` dont la PR est ouverte et la CI verte, `recommend` ne rend `MergeCandidate` que si la tête porte une approbation de mika-platform-qa sur ce commit. Le plan nomme la recommandation rendue sinon, et ce que le rapport du sink en écrit à côté (état de la revue, commit revu, moment de la lecture). Une revue non lue se dit « non lue ».*
  → **Reporté au vol 2b** (§ 11).
- **AC7.** *Contrôle négatif dans le même appel. Une matrice sur l'état de revue : approuvée sur la tête, approuvée sur un commit antérieur, `hold[review]` coupée, `hold` d'un autre motif, aucune revue, non lue. Avec CI verte et PR ouverte, seule la première cellule reste `MergeCandidate`. Le cas « coupée » part des données mesurées de mika#2972, pas d'un littéral inventé.*
  → **Reporté au vol 2b** (§ 11).
- **AC8.** *La sonde AC5 compte aussi, sur ses 10 vols, les `MergeCandidate` dont la tête n'avait pas d'approbation QA au moment du verdict. La cible est 0.*
  → **Reporté au vol 2b** (§ 11), qui ajoute le second compteur à la requête du § 10.

## Fire-Disposition

**Gate N/A — ce plan ne livre aucun détecteur.**

Les quatre livrables de test du § 9 sont des **assertions de comportement** : leur chemin de succès est « la cause vaut `ReadFailed` », « le rapport porte ce libellé », « l'appel 2 porte `--state open` », « ce vol ne sort pas avec `CiKnowledge::Read` ». Aucun ne scanne une population à la recherche d'une violation, aucun n'a « aucune violation trouvée » pour chemin de succès, aucun n'a d'allowlist à tenir.

Deux détecteurs plausibles ont été envisagés et refusés, pour que leur absence soit une décision et non un oubli :

- **Un garde « un seul lecteur du `statusCheckRollup` ».** C'est la question que le vol 1 avait explicitement renvoyée à ce plan : *« Si le vol 2 ajoute un second site de classification, c'est là que la question se pose, et son plan devra la trancher. »* Elle est tranchée par **la conception, pas par un scan** : `read_flight_prs` appelle `classify_ci_state` et n'écrit aucune seconde réduction du rollup (§ 4). Un scan de source défendrait ici une propriété que le vol ne fabrique pas, au prix d'une allowlist à tenir. Si un troisième chemin de lecture apparaît un jour, c'est **ce** vol-là qui porte le garde, avec la violation qui le motive sous les yeux.
- **Un garde « aucun littéral de dépôt dans le nouveau lecteur ».** `no_repo_list_test.rs` existe déjà et couvre le chemin de l'évaluateur ; `read_flight_prs` prend ses dépôts en **paramètre**, donc il n'a structurellement rien à nommer. Un second garde sur le même axe dédoublerait le lecteur unique de cette règle.

Aucune des trois options de mika#1574 n'est donc applicable : ni allowlist à livrer (a), ni détecteur à désarmer (b), ni cadrage à remonter (c). La section est présente pour que son « N/A » soit une décision lisible, avec ses deux candidats écartés nommés.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/milestone_manager/reader.rs` (`FlightPrRead`, `RepoPrRead`, `read_flight_prs`, doc) | 90 |
| `crates/mika-agent/src/milestone_manager/evaluator.rs` (six causes, `MilestoneMembership`, champ, composition de `ci` et `pr_state`) | 80 |
| `crates/mika-agent/src/milestone_manager/cadence.rs` (collecte des dépôts, appel du lecteur, ligne 698, doc de la décision) | 25 |
| `crates/mika-agent/src/milestone_manager/reporter.rs` (six libellés, `flight_milestone_label` sur le nouveau champ, doc) | 25 |
| tests `reader.rs` (quatre cas sur `RecordingGhRunner`, arguments `gh` assertés) | 75 |
| tests `evaluator.rs` (matrice des six causes × trois états de PR, appartenance, contrôle négatif) | 70 |
| tests `reporter.rs` (AC3 — les six libellés et les trois appartenances) | 20 |
| essaimage littéral (constructions `FlightVerdict {` des tests existants, champ `in_target_milestone`) | 20 |

Total estimé : 405 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 405 < 429, avec 24 lignes de marge, et 405 < 1000 pour le seuil par PR. La coupe qui fait tenir est celle du § 11 : le périmètre complet du vol 2 — la CI, l'état de PR, le découplage, **plus** l'état de revue de la tête (AC6-AC8) — est estimé à ~630 lignes, soit 200 au-dessus de la borne. Deux découpes ont été envisagées :

- **CI + revue au 2a, état de PR + découplage + sonde au 2b** : ~583 lignes au 2a. Toujours au-dessus de la borne, et le 2b hériterait du découplage de `flight_milestone_label` sans le `pr_state` qui le motive.
- **CI + état de PR + découplage + sonde au 2a, revue au 2b** (retenue) : 405 au 2a. Chaque vol ferme une question entière, le 2b réutilise les deux requêtes du 2a en y ajoutant deux champs `--json` plutôt qu'un appel.

La marge est mince et elle est tenue par le plancher du § 8 : les commits 1 à 3 livrent le découplage et le lecteur testé, et sont mergeables sans régression si le vol se termine court.
