# fix(webhook) — les sorties « rien à faire » du CI failure handler ne coûtent plus un tour LLM ; eval-calibration.yml redevient exécutable

- **Ticket :** senara-solutions/mika#2683 (`bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`)
- **Branche :** `fix/2683/webhook-les-sorties-rien-faire-du-ci`
- **Date :** 2026-10-14
- **Lignée :** mika#2675 (pré-filtre, phase 1) → mika#2678 (phase 2) → mika#2679 (phase 3) → mika#2680 (phase 4) → **ce ticket** ; mika#742 (création du workflow de calibration) ; mika#1745 (`surface_for_adoption`)

---

## Contexte

Trois constats de la même famille : **le moteur a déjà tranché en code, et le
tour LLM a lieu quand même.**

**(a)** `ci_failure_handler.rs` étape 2 (« loop prevention ») rend
`VerdictAction::Passthrough { enrichment: None }` sur une `check_suite` rouge de
la branche par défaut. `handlers.rs` traite ce `Passthrough` sans enrichissement
en laissant `req.text` intact — le tour part. Mesuré le 2026-10-12 à 17:25Z
(workflow « Eval Calibration », run 38282970921) : 2 appels mika-dev (glm-5.2),
116 k tokens d'entrée, conclusion du modèle « No active tasks. CI failure on
`main` is not from my tracked work — no action. » Le pré-filtre
(`server/webhook_prefilter.rs`) ne voit pas cet événement : `classify` ne lit que
`parse_check_suite_success`, donc une `check_suite` **rouge** sort de sa
population par construction.

**(b)** `.github/workflows/eval-calibration.yml` lance
`cargo test -p mika-agent --test eval -- --ignored --include-ignored`. Les deux
flags sont mutuellement exclusifs (`error: the options --include-ignored and
--ignored are mutually exclusive`), le `|| true` avale le code de sortie,
l'étape suivante ne trouve aucun artefact et sort en `exit 1`. **17 exécutions
sur 17 en `failure`, du 2026-06-22 au 2026-10-12.** La calibration n'a donc
jamais produit d'artefact, et chaque lundi elle déclenche (a).

**(c)** Même forme hors `main`. Le 2026-10-12 à 22:12:58Z, la CI en échec de la
PR dependabot mika#2682 atteint l'étape 4 du handler : aucune tâche active →
`surface_for_adoption` (mika#1745), qui notifie l'opérateur et écrit sa ligne
d'audit **en code**, puis rend `Passthrough { enrichment: Some(..) }` — le tour
part. Mesuré : 1 appel mika-dev, 55 754 tokens d'entrée, 33 de sortie. Les
autres sorties terminales de l'étape 3 et suivantes (`no open PR for branch`,
`task not in_progress`) ont la même forme.

### Ce que le pré-filtre est, et pourquoi il reste le bon placement

Le pré-filtre est une porte placée **après** tous les handlers structurels — qui
gardent chacun l'intégralité de leurs effets de bord — et **avant** `run_agent`.
Il ne décide qu'une chose : faut-il interroger le modèle ensuite. Il est
*fail-safe vers le LLM* : interrupteur désarmé, texte touché par un handler,
état illisible → le tour a lieu. Le ticket recommande explicitement cette porte
plutôt qu'un nouveau variant de `VerdictAction`, « pour garder un seul endroit
qui décide “pas de tour” et son audit » ; ce plan suit cette recommandation sans
la relitiger.

---

## Décisions de grooming (les deux points que le ticket laisse ouverts)

mika#2683 **ne porte aucun commentaire** à ce jour. Ni le point à trancher de
(a), ni la décision de budget de (b) ne sont consignés nulle part. Les deux sont
donc tranchés ici, dans le plan, avec leur raison.

### D1 — (a) : pas de notification déterministe d'une CI rouge sur `main` dans cette PR

Le ticket pose : *« une CI rouge sur `main` est un signal utile pour
l'opérateur. Si on le veut, la notification doit être déterministe (le tour
actuel ne notifie rien : il conclut “no action”), pas confiée au modèle. »*

**Décision : on ne notifie pas dans cette PR.** Quatre raisons, dans l'ordre de
poids :

1. **Le tour actuel ne notifie rien.** Il conclut « no action » et s'arrête.
   Supprimer le tour ne fait donc perdre **aucune** notification existante : ce
   n'est pas une régression de couverture, c'est un retrait de coût à
   information constante.
2. **Une notification sans déduplication serait une régression de bruit par
   construction.** Un seul push produit jusqu'à 8 événements
   `check_suite.completed` — mesuré par mika#1869 et documenté au commentaire de
   `SURFACE_FOR_ADOPTION_DEDUP_WINDOW`. Livrer la notification *et* sa fenêtre de
   dédup dans cette PR gonfle un périmètre déjà à trois volets, pour un signal
   que (b) rend beaucoup plus rare dès ce même merge.
3. **Le fait reste lisible.** La classe écrit sa ligne d'audit
   (`webhook_prefilter_skipped`, `after_value = red_check_suite_default_branch`)
   et sa ligne `info!`. Un `GROUP BY after_value` compte la population ; rien
   n'est perdu, seul le canal « notification » n'est pas ouvert.
4. **(b) retire le déclencheur hebdomadaire.** La population de (a) après merge
   est essentiellement « une vraie CI rouge sur `main` », qui est déjà visible
   par la page Actions et par la QA.

**Suivi :** si l'opérateur veut le signal, il demande un ticket de suivi dédié
qui porte **sa propre déduplication** (fenêtre sur `(repo, head_sha)`, modèle
`SURFACE_FOR_ADOPTION_DEDUP_WINDOW`). Ce plan ne crée pas ce ticket de son
propre chef : il nomme la condition pour l'ouvrir.

### D2 — (b) : on retire le `schedule`, on garde `workflow_dispatch`, et on corrige le flag

Le critère d'acceptation (b2) offre explicitement deux branches : *« décision de
budget de l'opérateur consignée dans le ticket avant merge, **ou** `schedule`
retiré »*.

**Décision : la seconde branche.** Raisons :

1. **Une session dispatchée qui pose une question est une session morte.** Le
   pilote ne peut pas obtenir la décision de budget pendant son exécution, et il
   n'a pas autorité pour la prendre à la place de l'opérateur.
2. **L'engagement de dépense serait non autorisé.** L'étape exporte
   `MIKA_EVAL_REAL_PROVIDERS: all` et quatre clés (Anthropic, OpenAI,
   OpenRouter, Groq). Corriger le flag *en gardant le cron* ferait passer le
   workflow de « échoue avant tout appel » à « appelle de vrais fournisseurs
   chaque lundi » — ≈ 2,57 USD par exécution, ≈ 150 USD/an selon
   `crates/mika-agent/CLAUDE.md`. Armer une dépense récurrente sans décision
   consignée est exactement ce que le garde-fou du ticket interdit.
3. **La correction du flag reste due, et elle est livrée.** La branche « retirer
   le `schedule` » n'autorise pas à laisser une commande invalide : AC (b1) est
   indépendant de (b2). Donc `--ignored` seul + retrait du `|| true` + le lint
   qui empêche la régression.
4. **Réversible en une ligne.** Réarmer le cron, une fois la décision consignée,
   est un diff de trois lignes de YAML.

**Choix du flag : `--ignored` seul.** C'est la commande que `CLAUDE.md` documente
déjà pour cette suite (`cargo test -p mika-agent --test eval -- --ignored` après
avoir posé `MIKA_EVAL_REAL_PROVIDERS`), et l'intention du workflow est la
matrice de calibration, dont les cas portent `#[ignore]`.
`--include-ignored` lancerait en plus toute la suite eval non ignorée — des
centaines de cas `MockLlmProvider` qui ne produisent aucun artefact de
calibration et n'ont rien à faire dans une fenêtre de 30 minutes partagée avec
de vrais appels réseau.

---

## Conception

### Volet (a) — une classe pour la `check_suite` rouge sur la branche par défaut

**Nouveau `Candidate` et nouvelle classe :**

| ajout | valeur de fil |
|---|---|
| `Candidate::RedDefaultBranch { repo, branch }` | — (interne) |
| `PrefilterClass::RedCheckSuiteDefaultBranch` | `"red_check_suite_default_branch"` |

La valeur de fil a été vérifiée absente du dépôt (`grep`), donc elle ne
fusionne pas une population existante — la discipline mika#2156 (« des classes
distinctes pour que les populations restent dénombrables séparément »).

**`classify` lit une grammaire de plus.** Elle appelle désormais
`parse_check_suite_failure` (déjà écrite, dans `ci_failure_handler.rs`) en plus
de `parse_check_suite_success`. Son contrat documenté — *lit des grammaires
existantes, n'en écrit aucune* — est préservé : aucun parseur n'est dupliqué
ici. Prérequis mécanique : les champs de `CheckSuiteFailureEvent` (`repo`,
`branch`, `conclusion`) passent de privés à `pub(crate)`, le type étant déjà
`pub(crate)`.

**Les deux termes de la classe**, qui donnent directement le test de mutation de
l'AC :

- `conclusion ∈ {failure, timed_out}` — porté par `parse_check_suite_failure`,
  qui n'accepte que `[GitHub] Check suite {failure|timed_out} on {repo} (branch:
  {branch})` ;
- `branch == DEFAULT_BRANCH` — le même terme que la classe verte voisine.

Une `check_suite` rouge d'une branche de PR ne produit **pas**
`Candidate::RedDefaultBranch` : elle est jugée par (c), comme l'AC l'exige.

**Terme d'état.** La classe consulte `branch_has_active_task(branch)` par
symétrie avec `green_check_suite_default_branch` : une tâche active sur `main`
est un état que le modèle doit prendre en compte, et une lecture locale de SQLite
coûte un ordre de grandeur de moins qu'un tour. L'étape 2 du handler, elle, skip
inconditionnellement — la porte est donc **strictement plus conservatrice** que
le handler, ce qui est le bon sens de l'écart pour un dispositif fail-safe vers
le LLM. Une lecture qui échoue passe par `state_unreadable` → `Decision::Llm`.

### Volet (c) — lire la trace d'audit, ne jamais re-dériver la décision

Le handler décide (c) au terme d'une chaîne qui comprend un appel GitHub
(`find_open_pr`), des recherches de tâches et la vérification d'un enfant de
callback. **Re-dériver cette décision dans le pré-filtre serait une divergence
programmée** — la classe d'erreur que ce dépôt a déjà payée deux fois
(mika#2158 `grooming_marker`, mika#2362 le seuil de retry recopié). Ce plan suit
l'idiome établi par la phase 3 (mika#2679) : **le handler laisse une trace
d'audit durable pour cet événement, et le pré-filtre la lit** via
`event_audit_rows()`, déjà indexé sur `trace_id`.

**Côté handler — deux lignes d'audit nouvelles, un nom d'outil nouveau :**

```rust
pub(crate) const CI_FAILURE_NO_ACTION_TOOL: &str = "ci_failure_no_action";
pub(crate) const CI_FAILURE_NO_ACTION_NO_OPEN_PR: &str = "no_open_pr";
pub(crate) const CI_FAILURE_NO_ACTION_TASK_NOT_IN_PROGRESS: &str = "task_not_in_progress";
```

- étape 3, bras `Ok(None)` (« no open PR for branch ») → audit
  `after_value = "no_open_pr"` ;
- étape 5 (`task.status != "in_progress"`) → audit
  `after_value = "task_not_in_progress"`.

Le bras `Err(e)` de l'étape 3 (état GitHub illisible) et le chemin « pas de
jeton GitHub » **n'écrivent rien** : ce sont précisément les sorties qui doivent
garder leur tour. C'est l'absence de ligne qui les protège, et c'est ce terme
que le test épingle.

`surface_for_adoption` n'est pas touché : sa notification et sa ligne d'audit
sont **lues**, jamais réécrites, et restent épinglées par les tests `mika1745`
existants.

**Côté pré-filtre — trois classes, une fonction pure :**

| classe | valeur de fil | source de la ligne lue |
|---|---|---|
| `CiFailureNoOpenPr` | `"ci_failure_no_open_pr"` | `ci_failure_no_action` / `no_open_pr` |
| `CiFailureSurfacedForAdoption` | `"ci_failure_surfaced_for_adoption"` | `surface_for_adoption` (existante) |
| `CiFailureTaskNotInProgress` | `"ci_failure_task_not_in_progress"` | `ci_failure_no_action` / `task_not_in_progress` |

Trois classes et non une, pour la raison mika#2156 : l'opérateur doit pouvoir
compter séparément « dependabot non adopté » et « tâche terminée depuis ».

```rust
/// Rend la classe lorsque les lignes d'audit de CET événement montrent une
/// sortie terminale du CI failure handler — et `None` dès qu'une ligne dit que
/// le handler a agi (`ci_failure_handled`, `ci_failure_escalated`), auquel cas
/// le tour a lieu.
fn ci_failure_no_action_class(this_event: &[AuditFact]) -> Option<PrefilterClass>
```

Fonction **pure** sur les lignes de l'événement, donc assertable sans base ni
réseau — la même forme que `head_decided_and_notified` de la phase 3. Elle
refuse explicitement en présence d'une ligne `ci_failure_handled` /
`ci_failure_escalated` : c'est le garde-fou qui empêche la classe de mordre sur
un événement où le handler a fait quelque chose.

**Contrainte SOLE WRITER à respecter littéralement.**
`canonical_tokens::tests::mika1745_the_surface_name_has_a_single_writer` refuse
tout littéral nu `"surface_for_adoption"` hors de `ci_failure_handler.rs`, et son
message de résolution dit *« retirer le second site. Ne PAS l'ajouter à
`SURFACE_FOR_ADOPTION_SOLE_WRITER_EXCEPTIONS` »*. Conséquence opératoire : le
pré-filtre `use` la **constante** `SURFACE_FOR_ADOPTION_TOOL` (un identifiant,
que le scan ne voit pas, parce que son prédicat est l'égalité sur un littéral
entier) et n'écrit jamais la chaîne. La constante passe donc de privée à
`pub(crate)`. La valeur de fil de la nouvelle classe est
`"ci_failure_surfaced_for_adoption"` — distincte de `"surface_for_adoption"`, et
de toute façon hors de portée d'un prédicat d'égalité.

Le nouveau nom d'outil `ci_failure_no_action` reçoit son propre garde SOLE
WRITER dans `canonical_tokens.rs`, sur le modèle exact du voisin mika#1745 :
propriétaire nommé (`ci_failure_handler.rs`) assertion anti-vacuité comprise,
allowlist vide.

### Le quatrième terme de `Touched`

`surface_for_adoption` **enrichit** `req.text`. Les trois termes actuels de
`Touched` excluraient donc la classe (c) par construction :
`by_any_handler` et `outside_ci_success_handler` sont tous deux vrais dès que le
handler a enrichi. Il faut un terme dont la fenêtre est l'appel du
`ci_failure_handler` lui-même :

```rust
pub(crate) struct Touched {
    pub(crate) by_any_handler: bool,
    pub(crate) after_verdict_handler: bool,
    pub(crate) outside_ci_success_handler: bool,
    pub(crate) outside_ci_failure_handler: bool,   // nouveau
}
```

`applies_to` câble `Candidate::RedDefaultBranch` et les trois candidats (c) sur
`outside_ci_failure_handler` ; les quatre candidats existants sont inchangés.

Dans `handlers.rs`, une capture symétrique de celle de `ci_success_handler`
autour de l'appel au `ci_failure_handler` :

```rust
let text_before_ci_failure = req.text.clone();
// … try_handle_ci_failure(…) …
text_around_ci_failure = Some((text_before_ci_failure, req.text.clone()));
```

et le terme, de forme identique à son voisin :

```rust
outside_ci_failure_handler: text_around_ci_failure.as_ref()
    .is_none_or(|(before, after)| before != original || req.text != *after),
```

**Pourquoi un quatrième terme plutôt qu'un assouplissement du premier :** le
sens de `Touched` est « quel handler a le droit d'avoir touché ce texte pour
cette classe ». Élargir `by_any_handler` ferait porter le relâchement à toutes
les classes, dont celles où un texte touché est précisément le signal de danger.

### Volet (b) — le YAML et le lint qui empêche la régression

**`.github/workflows/eval-calibration.yml` :**

1. `schedule:` / `cron: '0 8 * * 1'` retirés (D2) ; `workflow_dispatch` conservé.
2. Un commentaire d'en-tête qui nomme la raison du retrait, le coût mesuré, et
   la condition de réarmement (décision de budget consignée sur mika#2683).
3. `cargo test -p mika-agent --test eval -- --ignored 2>&1` — `--include-ignored`
   et `|| true` supprimés. Un échec cargo fait désormais échouer l'étape
   directement (AC b1, seconde moitié).

**Nouveau lint CI — `scripts/check-eval-ignored-flags.sh`**, sur le modèle de
`check-a2a-timeout-literals.sh` (prose du POURQUOI, règles nommées, allowlist
avec raison, avertissement explicite contre le resserrement vers le prédicat
naïf, pointeur vers cette section) :

- **R1 :** refuse toute invocation `cargo test` portant `--ignored` **et**
  `--include-ignored` dans la même ligne, sous `.github/workflows/*.yml`,
  `Makefile` et `scripts/*.sh`.
- **R2 :** plancher anti-vacuité — le scan doit avoir lu au moins N fichiers
  et vu au moins une invocation `cargo test … -- …`, sinon il échoue en disant
  qu'il n'a rien lu. « Un scan qui ne trouve personne ne doit pas se lire comme
  un scan propre » (mika#2103 / mika#2205).
- **Ne pas resserrer vers le prédicat naïf :** le sujet est la **co-occurrence
  sur un même site d'invocation**, jamais la présence de `--include-ignored`,
  qui est un flag parfaitement légitime seul.

Câblé dans `ci.yml` comme job nommé `eval-ignored-flags-lint`, avec sa seconde
étape « Pin the guard's negative behaviour » →
`scripts/test-check-eval-ignored-flags.sh`, exactement la discipline des sept
lints voisins.

**Vérification à faire pendant l'implémentation :** confirmer que l'eval KG,
gatée séparément derrière `MIKA_EVAL_KG_PROVIDERS` (que ce workflow n'exporte
pas), continue de s'auto-skipper sous `--ignored` seul. Si ce n'est pas le cas,
la commande reçoit en plus un filtre de nom et le plan le dit — pas de dépense
silencieuse.

---

## Étapes d'implémentation

1. **`ci_failure_handler.rs`** — champs de `CheckSuiteFailureEvent` en
   `pub(crate)` ; `SURFACE_FOR_ADOPTION_TOOL` en `pub(crate)` ; les trois
   constantes `CI_FAILURE_NO_ACTION_*` ; les deux écritures d'audit aux étapes 3
   (`Ok(None)`) et 5. Aucun changement de notification, aucun changement de
   `VerdictAction` rendu.
2. **`webhook_prefilter.rs`** — `Candidate::RedDefaultBranch` ; les quatre
   variants de `PrefilterClass` + leurs `as_str()` ; `classify` lisant
   `parse_check_suite_failure` ; `ci_failure_no_action_class` ; le quatrième
   terme de `Touched` et son câblage dans `applies_to` ; le bras de `decide`
   pour chaque nouvelle classe.
3. **`handlers.rs`** — capture `text_around_ci_failure` autour de l'appel du
   handler ; quatrième terme passé à `skip_turn`.
4. **`canonical_tokens.rs`** — garde SOLE WRITER pour `ci_failure_no_action`
   (propriétaire nommé, anti-vacuité, allowlist vide).
5. **Tests unitaires du pré-filtre** — amendement de
   `mika2675_classify_a_chaque_terme_sort_de_la_population` (les textes
   `failure` / `timed_out` sur `main` ne rendent plus `None` ; ceux d'une branche
   de PR continuent de sortir de la population de (a)) ; extension de
   `mika2675_valeurs_daudit_format_de_fil` aux quatre valeurs nouvelles ; cas de
   `ci_failure_no_action_class`, y compris le refus en présence de
   `ci_failure_handled` / `ci_failure_escalated`.
6. **Tests bout-en-bout** dans `crates/mika-agent/src/server/mod.rs` (où vivent
   déjà les tests `mika2675_*` : c'est le seul module consommateur de
   `webhook_prefilter::tests`, il n'existe aucun fichier de pré-filtre sous
   `crates/mika-agent/tests/eval/`).
7. **`.github/workflows/eval-calibration.yml`** — retrait du `schedule`,
   commentaire de raison, correction de la ligne cargo.
8. **`scripts/check-eval-ignored-flags.sh`** + son test négatif +
   le job `eval-ignored-flags-lint` dans `ci.yml`.

---

## Contrat de vérification

| ce qui est asserté | où |
|---|---|
| texte réel d'un webhook `check_suite` `failure` sur `main`, sans tâche → 0 appel modèle, 1 ligne d'audit `red_check_suite_default_branch` | `server/mod.rs` (e2e, `MockLlmProvider`) |
| même texte, tâche active sur `main` → le tour a lieu, 0 ligne de skip | `server/mod.rs` |
| mutation « retirer le terme branche par défaut » → un test rouge | le cas « branche de PR jugée par (c), pas par (a) » |
| mutation « retirer le terme conclusion `failure|timed_out` » → un test rouge | le cas `classify` sur une `check_suite` d'un autre type |
| texte réel d'un `check_suite` `failure` d'une branche de PR **sans tâche** → notification et ligne d'audit `surface_for_adoption` inchangées, 0 appel modèle | `server/mod.rs` (e2e) |
| branche de PR **avec** tâche `in_progress` → chemin de dispatch inchangé | `server/mod.rs` (e2e) |
| sortie « état GitHub illisible » (étape 3 `Err`) → le tour a lieu | test unitaire : aucune ligne `ci_failure_no_action` écrite |
| `ci_failure_handled` / `ci_failure_escalated` présent → `ci_failure_no_action_class` rend `None` | test unitaire pur |
| `cargo test -p mika-common mika2398` vert, **sans exemption nouvelle** | recensement `source_guard.rs` |
| `cargo test -p mika-agent mika1745` vert | notification + audit de `surface_for_adoption` |
| la commande cargo du workflow est valide, un échec cargo fait échouer l'étape | `check-eval-ignored-flags.sh` + lecture du YAML |
| le lint mord sur une fixture fautive et pas sur une légitime | `test-check-eval-ignored-flags.sh` |

**Commandes :** `cargo test -p mika-agent --lib`,
`cargo test -p mika-common mika2398`, `cargo clippy --all-targets`,
`cargo fmt --check`, `bash scripts/check-eval-ignored-flags.sh`,
`bash scripts/test-check-eval-ignored-flags.sh`,
`bash scripts/check-canonical-tokens.sh`.

---

## Risques nommés

1. **Recensement mika#2398.** Les nouveaux helpers de test `pub(crate)` du
   pré-filtre peuvent entrer dans la population masquée de
   `crates/mika-common/src/source_guard.rs`, dont le recensement est **gelé et
   n'est pas une allowlist** (« this census is not an allowlist and carries no
   exemption »). AC (a2) exige ce test vert. Résolution attendue : garder les
   nouveaux helpers `pub(crate)` et non `pub` — le recensement ne compte que les
   `pub fn` — et surtout **ne pas** ajouter d'entrée. Si le test rouge malgré
   tout, la bonne correction est au site, jamais au recensement.
2. **Rendre `SURFACE_FOR_ADOPTION_TOOL` `pub(crate)`** élargit sa visibilité
   sans élargir son écriture. Le garde SOLE WRITER porte sur le **littéral**,
   pas sur l'identifiant : la propriété reste tenue, et le message de résolution
   du garde interdit explicitement le recours à l'allowlist.
3. **Les deux nouvelles lignes d'audit sont un format de fil.**
   `after_value ∈ {no_open_pr, task_not_in_progress}` sera lu par des
   `GROUP BY` ; les valeurs sont épinglées par test, comme les valeurs de
   `PrefilterClass::as_str()`.
4. **Coût de (b) si le cron était réarmé par erreur.** Le commentaire d'en-tête
   nomme le montant et la condition ; le réarmement reste un acte explicite de
   l'opérateur sur un diff de trois lignes.
5. **L'eval KG.** Vérification nommée ci-dessus ; si `--ignored` seul la
   réveillait, un filtre de nom est ajouté dans la même PR.

---

## Hors périmètre

- La notification déterministe d'une CI rouge sur `main` (D1) — ticket de suivi
  à la demande de l'opérateur, avec sa propre déduplication.
- Le réarmement du `schedule` de la calibration (D2) — conditionné à une
  décision de budget consignée sur mika#2683.
- Les sorties du handler qui **informent** le modèle d'un état (étape 6
  « correctif déjà en vol », étape 8b auto-réparation, étape 7 escalade) : elles
  gardent leur tour, par conception et par test.

---

## Definition of Done

- [ ] Une `check_suite` rouge sur la branche par défaut ne coûte aucun tour LLM,
      et laisse une ligne d'audit de sa classe.
- [ ] Les trois sorties terminales de (c) ne coûtent aucun tour LLM, avec une
      classe distincte chacune.
- [ ] `surface_for_adoption` notifie et audite exactement comme avant ; les
      tests `mika1745` sont verts sans modification de leurs assertions.
- [ ] Les sorties qui informent le modèle (état illisible, correctif en vol,
      escalade) gardent leur tour, et un test l'épingle.
- [ ] Fail-safe vers le LLM inchangé : interrupteur désarmé, texte touché, état
      illisible → le tour a lieu.
- [ ] `eval-calibration.yml` porte une commande cargo valide, sans `|| true`, et
      sans `schedule`.
- [ ] Le lint `eval-ignored-flags-lint` est câblé dans `ci.yml` avec son test
      négatif, et a été vu rouge sur une fixture fautive avant d'être vu vert.
- [ ] `cargo test -p mika-common mika2398` vert sans exemption nouvelle.
- [ ] `cargo clippy --all-targets` et `cargo fmt --check` propres.
- [ ] Les deux décisions de grooming (D1, D2) sont lisibles dans ce plan et
      reprises dans le corps de la PR.

---

## Acceptance criteria

Transcrits verbatim de senara-solutions/mika#2683 :

- (a) Un test sur le texte réel d'un webhook `check_suite` `failure` sur `main`
  montre zéro appel au modèle et une ligne d'audit de la classe ; même texte sur
  une branche de PR : jugé par (c), pas par (a). Mutation : retirer le terme
  « branche par défaut » ou le terme « conclusion failure|timed_out » fait
  échouer un test.
- (a) `cargo test -p mika-common mika2398` vert (recensement des helpers
  `cfg(test)`).
- (b) La commande cargo du workflow est valide (`cargo test … -- <flags>` ne sort
  pas en erreur d'arguments), et un échec de cargo fait échouer l'étape.
- (b) Décision de budget de l'opérateur consignée dans le ticket avant merge, ou
  `schedule` retiré.
- (c) Test sur le texte réel d'un webhook `check_suite` `failure` d'une branche
  de PR sans tâche : notification et ligne d'audit `surface_for_adoption`
  inchangées, zéro appel au modèle. Branche de PR AVEC tâche `in_progress` :
  chemin de dispatch inchangé.

**Note sur (b2) :** ce plan satisfait la **seconde branche** du critère — le
`schedule` est retiré — par la décision D2 ci-dessus. La décision de budget
reste à consigner par l'opérateur s'il veut réarmer le cron ; ce n'est alors plus
un bloquant de merge mais le préalable d'un diff de trois lignes.

---

## Fire-Disposition

Le plan livre un détecteur : `scripts/check-eval-ignored-flags.sh`, règle de
lint CI dont le chemin de succès est « aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist — avec une allowlist
déclarée mais VIDE, le détecteur livré ARMÉ.**

Détail d'implémentation :

- **Population mesurée avant correctif : une violation**, la ligne 59 de
  `.github/workflows/eval-calibration.yml`. Elle est **corrigée dans la même
  PR** (volet b), pas exceptée. Après correctif, la population est vide — il n'y
  a donc aucune violation à qui donner une entrée grep-visible, et l'allowlist
  est livrée vide plutôt qu'absente, pour que l'ajout futur d'une exception soit
  un geste visible dans un tableau existant et non l'invention d'un mécanisme.
- **Constante d'allowlist nommée et commentée dans le script**
  (`ALLOWLIST=()`), avec la sémantique entrée-plus-raison des sept lints
  voisins : toute entrée future doit nommer la donnée précise, référencer un
  ticket de suivi, et le script refuse une entrée sans raison.
- **Assertion auto-nettoyante :** le script refuse une entrée d'allowlist qui ne
  correspond plus à aucune violation réelle (« exception stale »), donc le jour
  où le site excepté est corrigé, le lint rougit sur l'exception devenue morte.
  Sur une allowlist vide, cette assertion est trivialement satisfaite et devient
  active dès la première entrée.
- **Le détecteur a été vu rouge avant d'être vu vert** — c'est la condition
  d'armement, pas un détail : `scripts/test-check-eval-ignored-flags.sh` lui
  donne une fixture fautive (une ligne portant les deux flags) et exige l'échec,
  puis une fixture légitime (`--include-ignored` seul, `--ignored` seul) et exige
  le succès. Le job CI porte les deux étapes, comme
  `a2a-timeout-literal-lint` et `pilot-push-lint`.
- **Garde-fou anti-vacuité (mika#2103 / mika#2205) :** plancher de fichiers lus
  et au moins une invocation `cargo test … -- …` vue, sinon le script échoue en
  disant qu'il n'a rien lu — un scan qui ne trouve personne ne doit pas se lire
  comme un scan propre.

Les tests comportementaux du pré-filtre (volets a et c) ne sont pas des
détecteurs au sens de mika#2306 : leur sujet est le comportement d'un chemin
d'exécution, pas la recherche d'une violation dans le dépôt. Le garde SOLE
WRITER ajouté dans `canonical_tokens.rs` pour `ci_failure_no_action` en est un,
et relève de la même disposition (a) : propriétaire nommé, anti-vacuité,
allowlist vide, livré armé — strictement le modèle de son voisin mika#1745.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/webhook_prefilter.rs` (prod) | 185 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` (tests unitaires) | 135 |
| `crates/mika-agent/src/server/ci_failure_handler.rs` (prod) | 55 |
| `crates/mika-agent/src/server/ci_failure_handler.rs` (tests) | 40 |
| `crates/mika-agent/src/server/handlers.rs` | 15 |
| `crates/mika-agent/src/canonical_tokens.rs` (garde SOLE WRITER) | 70 |
| `crates/mika-agent/src/server/mod.rs` (tests bout-en-bout) | 150 |
| `scripts/check-eval-ignored-flags.sh` | 95 |
| `scripts/test-check-eval-ignored-flags.sh` | 60 |
| `.github/workflows/eval-calibration.yml` | 15 |
| `.github/workflows/ci.yml` (job du lint) | 12 |

Total estimé : 832 lignes

Sous le seuil de 1000 lignes de mika#2636 : aucun découpage en phases n'est
requis, et le périmètre des trois volets tient dans cette PR. Les deux éléments
qui l'auraient fait franchir le seuil sont explicitement renvoyés hors périmètre
(ci-dessus) : la notification déterministe de D1 avec sa fenêtre de dédup, et le
réarmement du `schedule` de D2.
