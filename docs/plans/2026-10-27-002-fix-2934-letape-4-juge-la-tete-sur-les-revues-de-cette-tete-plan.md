# mika#2934 — l'étape 4 juge la tête sur les revues de cette tête

- Ticket : senara-solutions/mika#2934 (p1, `loop-substrate`)
- Branche : `fix/2934/qa-review-webhook-success-le-registre`
- Prédécesseur direct : mika#2927 (`docs/plans/2026-10-27-001-fix-2927-le-registre-nattest-que-le-verdict-poste-plan.md`)
- Bloque : mika#2920 (vol 3 de mika#2917), remis en draft en attendant ce ticket
- Le merge revient à Vincent.

## Contexte

Le registre `qa_reviewed_head` doit dire sur quelles têtes mika-qa a **statué**.
Le 2026-10-27, sous le moteur `468b2925` (mika#2927 déployé à 06:41:07Z), il a
déclaré trois têtes conclues 9 à 17 minutes **avant** le premier verdict de
mika-qa sur chacune d'elles, plus une quatrième ligne écrite avant son propre
post. Quatre lignes, deux défauts de nature différente.

| tête | ligne | heure | session | premier verdict mika-qa sur cette tête |
|---|---|---|---|---|
| `#2931@1f649bf1` | `stop_verdict_recorded` | 07:06:02Z | `7b8f16b1` | 07:15:57Z |
| `#2931@1f649bf1` | `verdict_posted` | 07:07:19Z | `7418ef0c` | 07:15:57Z |
| `#2920@f7542d43` | `stop_verdict_recorded` | 07:07:36Z | `cf6070e9` | 07:16:49Z |
| `#2928@13657863` | `stop_verdict_recorded` | 07:07:42Z | `10cb487d` | 07:24:54Z |

**Défaut 1 (n=3 sur 3) — l'étape 4 juge une tête sur un état de PR.** L'étape 4
de `qa-review-webhook-success` demande un verdict « tied to the current head
SHA ». Le modèle n'a en main qu'un champ de PR, `reviewDecision`, rendu par
l'appel `pr list` de l'étape 1. Or `reviewDecision` survit au force-push :
aucune règle ne révoque ici une approbation périmée. Une ancienne tête approuvée
suffit à faire déclarer la nouvelle « statuée », et la ligne part avec le SHA du
check_suite, qui est bien la nouvelle tête. Les trois sessions ont fait
exactement cette lecture, et une quatrième (`df7de3a0`, 07:33:25Z) l'a refaite.

**Défaut 2 (n=1) — `verdict_posted` est écrit avant le verdict.** Le tour
`qa-review` de `7418ef0c` a écrit la ligne alors qu'il venait de lancer un build
et attendait le callback. Son propre message de fin le dit. L'invariant 7 du
prompt `qa-review` exige pourtant l'appel « after invariant 6 returned success ».
**La consigne existait et le modèle l'a enfreinte.**

Le coût est nul aujourd'hui **parce que personne ne lit encore le registre**.
mika#2920 en fait le lecteur du pré-filtre : une tête inscrite y est écartée.
Avec le défaut 1, une PR rebasée après approbation ne serait plus jamais revue —
l'incident `#2920@0a55d28a` que mika#2927 fermait, sous une nouvelle forme.

### Pourquoi la sonde d'AC4 de mika#2927 ne l'a pas vu

Ses trois lectures sont **vertes** sur ces données. Sa Lecture 2 échoue pour deux
raisons indépendantes, et ce sont les deux que ce plan corrige :

1. Elle vérifie qu'un verdict existe **au moment où on la lance**, pas **avant**
   la ligne. Le verdict de 07:15:57Z couvre après coup la ligne de 07:07:19Z.
2. Elle ne lit que `verdict_posted`. Les deux valeurs `stop_*` déclarent pourtant
   elles aussi une tête conclue, et c'est par elles que passe le défaut 1.

## Ce qui est refusé (bearing, non négociable)

Hérité de mika#2917 et mika#2927, intact :

- **Pas de table, pas de migration, pas de DDL.** Le registre reste des lignes
  `audit_events` (`tool_name = "qa_reviewed_head"`).
- **Écrivain unique.** `crates/mika-agent/src/tools/record_reviewed_head.rs`
  reste le seul site qui écrit. Tenu par
  `mika2917_a2_un_seul_ecrivain_et_au_plus_un_lecteur`.
- **Vocabulaire fermé de trois valeurs.** `REVIEWED_HEAD_OUTCOMES` ne change pas :
  `verdict_posted`, `stop_already_reviewed_sha`, `stop_verdict_recorded`. Le
  littéral `review` reste hors vocabulaire (mika#2927).
- **Les six valeurs de `Webhook-Outcome:` (mika#2855) ne bougent pas.** L'asymétrie
  avec le vocabulaire du registre est délibérée. En particulier `stop_verdict_recorded`
  **reste** une issue possible de l'étape 4 : ce plan change la **preuve** qu'elle
  exige, pas son existence.
- **L'outil ne fait aucun appel réseau.** `record_reviewed_head` n'appelle ni `gh`
  ni aucun sous-processus. Il n'est pas le site où l'on va chercher une preuve ;
  il est le site où l'on refuse d'écrire sans preuve.
- **Aucune suppression de ligne en base.** Voir § *Le legs*.

## Les deux défauts appellent deux remèdes de classes différentes

C'est l'arbitrage central du plan, et il est demandé explicitement par AC3.

**Défaut 1 → le prompt, parce qu'il manque une information au modèle.** Le
prompt ne lui a jamais donné la lecture qui répond à la question posée. Il a lu
le seul champ qu'on lui tendait. Une consigne qui apporte une donnée absente est
le bon remède, et les trois occurrences sont trois lectures correctes d'un
prompt faux — pas trois désobéissances.

**Défaut 2 → le moteur, parce que la consigne existait, explicite, et a été
enfreinte.** Renforcer une consigne déjà littérale (« after invariant 6 returned
success ») serait la deuxième itération du même remède, sans aucune mesure qui la
soutienne : le ticket dit lui-même que n=1 ne permet pas de conclure qu'une
consigne seule tient cette propriété. Deux faits achèvent l'arbitrage :

- Le prompt `qa-review` est **au plafond dur**. `max_prompt_size = 81920` est
  `MAX_PROMPT_SIZE_CEILING` (`skills/index.rs:27`) ; son `skill.toml` déclare en
  toutes lettres que la remédiation « raise » est épuisée et que la prochaine
  addition doit trimmer ou sharder. Une consigne renforcée se paierait en coupant
  une autre règle d'une énumération fermée — exactement ce que ce fichier refuse.
- Le refus moteur est **plus fort que ce qu'AC2 demande** : là où un test atteste
  qu'une ligne n'a pas été écrite une fois, le refus rend l'écriture inatteignable.

**Et le même refus ferme AC2 en CI.** C'est la raison pour laquelle la garde
couvre les **trois** valeurs et pas la seule `verdict_posted` : un test de rejeu
du force-push piloté par `MockLlmProvider` n'atteste que le script du mock, jamais
l'obéissance d'un modèle. Ce qu'il peut attester, c'est qu'un tour sans la lecture
head-matched **ne peut pas** écrire `stop_verdict_recorded`. Une seule garde, un
seul site, trois valeurs, une preuve par valeur.

## Requirements

### R1 — `skills/bundled/qa-review-webhook-success/handlers/qa_head_reviews.sh` : l'outil qui donne la lecture (AC1)

`gh pr view --json reviews` **ne rend pas `commit_id`** — ce champ n'existe pas
dans le schéma `reviews` de `gh pr view`. L'étape 3 actuelle nomme donc déjà un
champ que sa propre lecture ne produit pas ; le défaut 1 n'est pas seul, il a un
frère silencieux à l'étape 3. Le seul endpoint qui porte `commit_id` est
`GET /repos/{owner}/{repo}/pulls/{n}/reviews`.

Ce chemin est **hors du scope `run_gh` de mika-qa** :
`validate_qa_review_gh_scope` (`skills/builtin_handlers.rs:2339`) n'autorise
`gh api` que pour `GET /advisories`, et comme `qa-review` est `always_on = true`
la porte s'applique aussi au tour webhook. Deux issues : élargir la porte, ou
donner la donnée par un handler. **Le handler**, pour la raison déjà écrite dans
`qa_pr_view.sh` : *remove capability, don't prohibit it*
(`docs/solutions/prompt-engineering/2026-04-08-remove-capability-not-prohibit-it.md`).
Élargir la porte rendrait **tout** `pulls/*/reviews` atteignable depuis n'importe
quel tour mika-qa pour fermer un besoin d'un seul tour.

Nouveau handler, calqué sur `qa_pr_view.sh` (même en-tête, même `set -euo
pipefail`, mêmes refus `gh`/`jq` absents, même validation anti-injection
d'option) :

- Entrée stdin JSON : `repo` (`owner/repo`), `pr` (entier), `head_sha` (40 hex).
  Chaque champ est validé par expression régulière **avant** d'atteindre la ligne
  de commande — `repo` sur `^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$`, `pr` sur `^[0-9]+$`,
  `head_sha` sur `^[0-9a-f]{40}$`. Un champ absent ou mal formé sort en erreur,
  jamais en liste vide : une liste vide est la réponse « aucune revue sur cette
  tête », et confondre les deux est exactement la classe mika#2205.
- Appel : `gh api "repos/$REPO/pulls/$PR/reviews" --paginate --jq`, filtré sur
  `.commit_id == $HEAD_SHA`.
- Sortie stdout : un tableau JSON d'objets
  `{author, state, submitted_at, carries_verdict_line}`, où `carries_verdict_line`
  est vrai quand le corps de la revue porte une ligne `VERDICT: pass` ou
  `VERDICT: block[…]`. Le **corps n'est pas rendu** : l'étape 4 a besoin du fait,
  pas du texte, et le prompt n'a pas à transporter des revues entières.

Déclaration dans `skills/bundled/qa-review-webhook-success/skill.toml` :
le handler, et `[constraints] required_tools = ["qa_head_reviews", "run_gh"]`.
`make verify-bundled-skills` (mika#1575) vérifie la cohérence des deux.

**Précondition à vérifier à l'implémentation** : `qa_pr_view.sh` appelle déjà
`gh` sans token explicite depuis un handler de skill, donc l'environnement
d'exécution des handlers est authentifié. Si le `gh api` sort en 401 au premier
essai réel, le défaut est là et nulle part ailleurs — ne pas le compenser dans le
prompt.

### R2 — `skills/bundled/qa-review-webhook-success/system_prompt.md` : les étapes 1, 3 et 4 (AC1)

Trois éditions, et une seule lecture pour deux étapes.

**Étape 1** — retirer `reviewDecision` de la liste `--json`. C'est le champ que
les quatre sessions ont cité ; le modèle ne peut pas lire ce qu'il n'a pas.
`mergeable`, `mergeStateStatus` et `statusCheckRollup` restent : ce ne sont pas
les champs du défaut mesuré, et les retirer serait un changement non motivé.

**Étape 3** — remplacer `run_gh("pr view <n> --json reviews")` par
`qa_head_reviews(repo: "<owner/repo>", pr: <n>, head_sha: "<le SHA du check_suite>")`.
Le prédicat ne change pas : une revue **à toi** (`mika-platform-qa`) dans le
tableau rendu ⇒ arrêt, `stop_already_reviewed_sha`.

**Étape 4** — le prédicat devient : dans **le même tableau déjà en main**, une
entrée dont `carries_verdict_line` est vrai ⇒ arrêt, `stop_verdict_recorded`.
Le texte nomme explicitement les deux refus :

- *« Ne juge jamais cette étape sur `reviewDecision`, `mergeStateStatus`, ni sur
  aucun autre état de PR : ils survivent au force-push. »*
- *« N'appelle pas `qa_head_reviews` une seconde fois : l'étape 3 l'a déjà
  appelé, son résultat est en contexte. »*

Les deux étapes restent **distinctes**, et c'est voulu : l'étape 3 n'étant pas
tirée quand on atteint l'étape 4, celle-ci porte le cas « un verdict faisant
autorité existe sur cette tête mais pas sous ton propre compte ». Les fondre
casserait les six valeurs de `Webhook-Outcome:` et les tables de conformité qui
les épinglent.

Le § *Record the head you concluded on* n'est pas touché : son « After step 3 or
after step 4 » reste exact, le tableau à deux lignes reste exact, et
« Dispatching a review at step 5 records NOTHING here » reste exact.

### R3 — `crates/mika-agent/src/qa_reviewed_head.rs` : le prédicat de preuve de tour (AC2, AC3)

Nouvelle moitié de production dans le module qui porte déjà le vocabulaire.

```rust
/// Ce que ce tour n'a pas fait, et qui interdit la ligne qu'il demande.
pub enum PreuveManquante {
    /// `verdict_posted` sans `run_gh pr review` réussi dans ce tour.
    VerdictSansPost,
    /// Un `stop_*` sans lecture head-matched des revues de CETTE tête.
    StopSansLectureDeTete,
}

pub fn preuve_du_tour_manquante(
    outcome: &str,
    head_sha: &str,
    deja_execute: &[ToolCallSummary],
) -> Option<PreuveManquante>
```

Une preuve par valeur, et les trois valeurs en ont une :

| `outcome` | preuve exigée dans ce tour |
|---|---|
| `verdict_posted` | `qa_build_callback::pr_review_posted_in_turn(deja_execute)` |
| `stop_already_reviewed_sha` | un `ToolCallSummary` `name == "qa_head_reviews"`, `success`, dont l'`input_summary` porte `head_sha` |
| `stop_verdict_recorded` | idem |

`pr_review_posted_in_turn` est **appelée**, pas recopiée. Son doc-comment porte
déjà la raison (mika#2355 : deux copies d'un prédicat divergent, leçon
`grooming_marker` / mika#2158), et elle est publique.

Trois propriétés que l'implémentation ne doit pas perdre :

- **Un `outcome` hors vocabulaire rend `None`.** Le refus de valeur appartient à
  `reviewed_head_outcome` et au schéma de l'outil ; ce prédicat ne duplique pas
  ce jugement, il ne répond qu'à « la preuve est-elle là ».
- **`deja_execute` comprend le pas courant.** Le site (R4) passe l'union des pas
  antérieurs et des appels déjà exécutés **dans le bloc courant**, dans l'ordre.
  Un `record_reviewed_head` émis avant son `pr review` dans le même bloc est donc
  refusé, ce qui est la propriété voulue : on ne peut pas attester le succès d'un
  post qu'on n'a pas encore vu revenir.
- **La correspondance du SHA passe par `input_summary`**, qui est tronqué à
  `planning::policy::INPUT_SUMMARY_MAX` et scrubbé. Un SHA de 40 caractères dans
  un objet à trois champs tient très largement ; si une évolution d'`INPUT_SUMMARY_MAX`
  le mettait en danger, c'est un test qui doit rougir, pas une troncature
  silencieuse — d'où le test de borne de R6.

### R4 — `crates/mika-agent/src/tool_execution/dispatch.rs` : le site de refus (AC2, AC3)

Un seul site, dans la boucle sur les blocs de `process_tool_calls`, **sur le
modèle exact du gate de frontière `send_message` (#771) déjà présent au même
endroit** (`dispatch.rs:178-211`) : pousser un `ToolCallSummary` en échec, pousser
un `tool_result` `is_error` pour garder l'historique apparié, `continue` sans
exécuter. L'outil n'est pas appelé, donc **aucune ligne n'est écrite** — ce n'est
pas un `field_error` de `record_reviewed_head`, c'est une suppression en amont.

Message servi au modèle, dans la grammaire du site voisin :

```
[mika-engine] Tool call suppressed: record_reviewed_head needs this turn's own
evidence (mika#2934). <ce qui manque, selon la variante>.
```

- `VerdictSansPost` : *« `verdict_posted` attests a verdict this turn posted. No
  successful `run_gh("pr review …")` appears in this turn's tool history. If you
  dispatched a build and are awaiting its callback, record nothing — the callback
  turn records the head after its own post (qa-review invariant 7). »*
- `StopSansLectureDeTete` : *« a `stop_*` outcome attests a head you ruled out on
  the evidence of THAT head's reviews. No successful `qa_head_reviews` call for
  `<head_sha>` appears in this turn's tool history. Call it first (step 3), then
  conclude. »*

WARN structuré au même site, nom d'événement `reviewed_head_write_without_evidence`,
champs `trace_id`, `step`, `outcome`, `variante`. C'est la sonde qui dit si la
garde tire en production, et son **zéro** est l'assertion que le prompt tient.

Passage du paramètre : `process_tool_calls` prend un
`prior_summaries: &[ToolCallSummary]`, renseigné aux **trois** sites d'appel de
`agent_loop/mod.rs` avec `&all_tool_summaries`. L'emprunt immuable se termine
avant l'`extend` qui suit (`agent_loop/mod.rs:2121`), donc aucun remaniement de
durée de vie. Les sites d'appel de test prennent `&[]`.

**Pourquoi pas une garde EndTurn** — la famille `qa_build_callback_verdict` /
`qa_review_submission` re-prompte à la fin du tour. Ici la ligne serait déjà en
base : une garde EndTurn nommerait le dégât au lieu de l'empêcher.

**Pourquoi pas dans l'outil** — `ToolContext` est construit **une fois par tour**
(trois sites de production) et ne peut pas porter un historique qui grandit
pendant le tour. Le commentaire de `is_webhook_fallthrough_turn` dit la règle de
la maison : ce qui traverse `ToolContext` est un verdict figé, calculé au site
qui possède la donnée. Ici le site qui possède la donnée est `dispatch.rs`.

### R5 — les détecteurs de conformité de prompt (AC1)

Dans `qa_reviewed_head.rs`, à côté de `CLAUSES_DU_POST_REUSSI` et dans la même
forme (`include_str!` sur l'arbre, motif mika#2340 : une édition de
`skills/bundled/` est invisible aux agents avant `make deploy`, donc le test lit
l'arbre et non le manifeste compilé) :

```rust
const CLAUSES_DE_LA_TETE_JUGEE: &[(&str, &[&str])] = &[(
    "qa-review-webhook-success",
    &["qa_head_reviews(", "carries_verdict_line", "commit_id"],
)];

const ETATS_DE_PR_REFUSES_A_LETAPE_4: &[&str] = &["reviewDecision"];
```

Deux assertions et leurs contrôles négatifs :

- `mika2934_letape_4_nomme_la_lecture_et_son_outil` — chaque littéral de
  `CLAUSES_DE_LA_TETE_JUGEE` est présent dans `PROMPT_WEBHOOK`.
- `mika2934_le_prompt_du_webhook_ne_cite_plus_aucun_etat_de_pr_revocable` —
  `reviewDecision` est **absent** du prompt entier. Un contrôle négatif, bâti sur
  une copie du prompt où le littéral est réinséré, atteste que l'assertion rougit.

### R6 — les tests d'acceptation

Dans `qa_reviewed_head.rs` (prédicat) et `dispatch.rs` (site), tous en
`#[cfg(test)]` :

1. `mika2934_ac3_un_tour_qui_attend_un_callback_de_build_necrit_pas_verdict_posted`
   — historique : `build_mika` réussi, aucun `run_gh pr review`. Le prédicat rend
   `VerdictSansPost`. C'est la reproduction littérale de `7418ef0c`.
2. **Contrôle négatif, dans le même test** — le même historique plus un
   `run_gh pr review` réussi : le prédicat rend `None`.
3. `mika2934_ac2_un_stop_sans_lecture_de_tete_est_refuse` — historique :
   `run_gh pr list` réussi, rien d'autre. `stop_verdict_recorded` rend
   `StopSansLectureDeTete`.
4. **Contrôle négatif, dans le même test** — un `qa_head_reviews` réussi portant
   le `head_sha` : rend `None`.
5. `mika2934_ac2_le_force_push_ne_satisfait_pas_la_preuve` — l'autre moitié
   d'AC2, celle qui porte la tête : un `qa_head_reviews` réussi portant
   **l'ancienne** tête ne satisfait pas une ligne demandée sur la **nouvelle**.
   C'est la propriété que `reviewDecision` n'avait pas.
6. `mika2934_le_site_supprime_lappel_sans_lexecuter` — au niveau de
   `process_tool_calls` : le `ToolCallSummary` rendu est en échec, le
   `tool_result` est `is_error`, et **l'outil n'a pas été appelé**.
7. `mika2934_la_borne_de_resume_dentree_laisse_passer_un_sha` — un appel
   `qa_head_reviews` nominal, résumé puis cherché : le SHA survit à
   `INPUT_SUMMARY_MAX`. Rougit si la borne descend sous le besoin, au lieu de
   transformer la garde en refus systématique.

## Non-Requirements (hors de ce plan, délibérément)

- **Le lecteur de mika#2920 et son interrupteur.** Hors périmètre par le ticket.
  Voir § *Ordre avec mika#2920*.
- **Le vocabulaire de `Webhook-Outcome:` (mika#2855).** Six valeurs, inchangées.
- **Le prompt `qa-review` et son invariant 7.** Il dit déjà la bonne chose ; R4
  le rend structurel. Le fichier est au plafond dur et ne reçoit rien.
- **La suppression ou la réécriture de lignes en base.** Voir § *Le legs*.
- **Un scénario de calibration mika-qa rejouant le force-push avec un vrai
  modèle.** Ce serait l'instrument de la question « le modèle obéit-il au nouveau
  prompt », et il existe déjà sous une autre forme : l'attestation
  `Webhook-Outcome:` que `scripts/measure-webhook-turn-cost` agrège par
  `charged_by_outcome`. La sonde d'AC5 la lit. Ajouter un sixième scénario de
  calibration coûterait un run payant hors CI pour une réponse que la production
  donne déjà.

## Le legs — on laisse, et on nomme la condition

Les quatre lignes de ce ticket **restent en base**. Trois raisons, dans cet ordre :

1. **La sonde d'AC4 exige de les rejouer.** Sa lecture de bonne foi doit les rendre
   en violation ; les neutraliser détruirait le seul corpus qui atteste que la
   sonde voit quelque chose.
2. **Elles sortent seules de la fenêtre le 2026-11-26**
   (`REVIEWED_HEAD_WINDOW_DAYS` = 30 jours à partir du 2026-10-27).
3. **Aucune écriture hors de l'écrivain unique.** Les retirer demanderait un `DELETE`
   sur `audit_events` depuis un chemin qui n'existe pas et qu'on ne créera pas pour
   quatre lignes.

**La condition, à tenir par mika#2920 et non par ce ticket** : #2920 ne sort de
draft qu'après le 2026-11-26, **ou** déploie avec
`MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD=off` et ne l'arme qu'après cette date.
C'est Vincent qui tranche, et c'est écrit ici pour qu'il n'ait pas à le
reconstruire.

## Verification Contract

- `cargo test -p mika-agent qa_reviewed_head` — vert, les sept tests de R6 inclus.
- `cargo test -p mika-agent tool_execution` — vert, le site de R4 inclus.
- `cargo test` — suite complète verte.
- `cargo clippy --all-targets -- -D warnings` — propre.
- `cargo fmt --check` — propre.
- `make verify-bundled-skills` — vert : le handler de R1 est déclaré, résolu, et
  cohérent avec `required_tools`.
- `shellcheck skills/bundled/qa-review-webhook-success/handlers/qa_head_reviews.sh`
  — propre.
- `scripts/verify-pipeline.sh origin/main` — vert.
- Le handler est exercé une fois à la main contre une vraie PR, pour attester que
  l'authentification `gh` est présente dans l'environnement des handlers et que
  `commit_id` revient bien rempli.

## Definition of Done

- Le prompt `qa-review-webhook-success` ne cite plus `reviewDecision` nulle part,
  et ses étapes 3 et 4 nomment `qa_head_reviews` comme unique source.
- `qa_head_reviews` existe, est déclaré, validé contre l'injection d'option, et
  distingue « aucune revue sur cette tête » d'une erreur d'entrée.
- `record_reviewed_head` est **supprimé avant exécution** quand la preuve du tour
  manque, pour les trois valeurs du vocabulaire, avec un message qui dit quoi faire.
- Les sept tests de R6 et les deux détecteurs de R5 passent, chacun avec son
  contrôle négatif.
- Le Verification Contract est vert en entier.
- La sonde d'AC4 est écrite dans ce plan, exécutable telle quelle, et sa lecture
  de bonne foi rend les quatre lignes du tableau de contexte en violation.
- La PR porte `Closes #2934` et cite ce plan.

## Acceptance criteria

- **AC1, l'étape 4 juge la tête sur les revues de cette tête.** La preuve
  « verdict déjà enregistré » provient des revues dont `commit_id == head`, jamais
  de `reviewDecision` ni d'aucun autre état de PR. Le prompt nomme la lecture à
  faire, et l'outil qui la donne.
- **AC2, le cas force-push est rejoué.** Un test présente au tour webhook une PR
  APPROVED sur l'ancienne tête et sans revue sur la nouvelle. Il atteste qu'aucune
  ligne `stop_verdict_recorded` n'est écrite pour la nouvelle tête et que le tour
  ne s'arrête pas à l'étape 4. Le contrôle négatif est dans le même test : avec une
  revue APPROVED dont `commit_id == head`, l'arrêt à l'étape 4 a lieu.
- **AC3, `verdict_posted` suit le verdict.** Le défaut 2 est traité à l'endroit que
  le plan choisit, consigne renforcée ou garde moteur, et ce choix est justifié par
  la mesure : n=1 contre n=3, et la consigne existait déjà. Un tour qui lance un
  build et attend le callback n'écrit pas `verdict_posted`.
- **AC4, une sonde temporelle sur toutes les valeurs de conclusion.** Elle remplace
  la lecture 2 du plan de #2927. Pour **chaque** ligne `qa_reviewed_head` de mika-qa
  (`verdict_posted`, `stop_already_reviewed_sha`, `stop_verdict_recorded`), elle
  compare `created_at` à la date du premier verdict de mika-qa sur cette tête
  (`submitted_at` d'une revue dont `commit_id == head`). Une ligne écrite avant tout
  verdict est une violation, même si un verdict arrive ensuite. Sa lecture de bonne
  foi rejoue les trois lignes de ce ticket et doit les rendre en violation.
- **AC5, la sonde post-déploiement**, écrite dans le plan : après `make deploy`, la
  sonde d'AC4 ne rend aucune violation sur les lignes postérieures au déploiement,
  sur au moins une PR poussée après une approbation.

### Comment chaque AC est tenu

| AC | livrable | preuve |
|---|---|---|
| AC1 | R1, R2 | R5 (deux détecteurs + contrôle négatif) |
| AC2 | R3, R4 | R6 tests 3, 4, 5, 6 |
| AC3 | R3, R4 | R6 tests 1, 2, 6 — et l'arbitrage justifié au § *Les deux défauts* |
| AC4 | § *Sonde post-déploiement* | sa lecture de bonne foi, quatre violations attendues |
| AC5 | § *Sonde post-déploiement* | lecture 2, après `make deploy` |

## Fire-Disposition

Ce plan livre des détecteurs : deux assertions de conformité de prompt (R5), sept
tests (R6), et une garde moteur dont le chemin de succès est « aucune violation
trouvée » (R4). La section est donc due.

**Option retenue : (a) exception nommée en allowlist, livrée VIDE.**

```rust
/// Prompts dispensés du détecteur de R5. LIVRÉE VIDE, et c'est le contrat.
const CONFORMITE_TETE_JUGEE_ALLOWLIST: &[(&str, &str)] = &[];
```

Même forme et même raison que `PROMPT_OUTCOME_ALLOWLIST` (mika#2927), un
`const` plus haut dans le même fichier. Elle est vide parce que **aucune
violation n'existe dans le code à l'issue de ce plan** : R2 corrige le seul
prompt concerné dans le même commit que le détecteur qui le surveille, et la
garde de R4 est neuve, donc sans antécédent à excuser.

L'assertion auto-nettoyante, telle que mika#2309 l'exige (*l'exception expire par
le geste qui la rend inutile*) :

```rust
#[test]
fn mika2934_lallowlist_de_conformite_est_livree_vide_et_le_reste() {
    assert!(
        CONFORMITE_TETE_JUGEE_ALLOWLIST.is_empty(),
        "Une entrée a été ajoutée ici. Une dispense de conformité sur ce \
         détecteur rouvre exactement le défaut 1 de mika#2934 : un prompt qui \
         juge une tête sur un état de PR révocable. Corrige le prompt, ou \
         ouvre un ticket de suivi et cite-le dans l'entrée."
    );
}
```

Une entrée future doit nommer le prompt précis, la clause dispensée et le ticket
de suivi ; le test ci-dessus rougit à la première, ce qui force la justification
en revue plutôt qu'en silence.

**Les quatre lignes du legs ne sont pas une exception d'allowlist.** Ce sont des
**données** en base, hors de portée de tout test unitaire. Leur disposition est
tranchée au § *Le legs* (on laisse, avec sa condition), et leur visibilité est la
lecture de bonne foi d'AC4 — qui doit les rendre en violation, pas les excuser.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/qa-review-webhook-success/handlers/qa_head_reviews.sh` | 45 |
| `skills/bundled/qa-review-webhook-success/skill.toml` | 6 |
| `skills/bundled/qa-review-webhook-success/system_prompt.md` | 25 |
| `crates/mika-agent/src/qa_reviewed_head.rs` — prédicat de preuve (R3) | 70 |
| `crates/mika-agent/src/qa_reviewed_head.rs` — détecteurs de conformité (R5) | 55 |
| `crates/mika-agent/src/tool_execution/dispatch.rs` — site de refus (R4) | 40 |
| `process_tool_calls` — paramètre + 3 sites d'appel | 12 |
| tests (R6, dans `qa_reviewed_head.rs` et `dispatch.rs`) | 120 |

Total estimé : 373 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 373 passe sous la borne par vol
(429) et très loin sous le seuil par PR (1000), sans découpage. Trois choses ont
été arbitrées pour qu'il en soit ainsi, et chacune est un refus argumenté plutôt
qu'un report :

- **Pas de test `tests/eval/` de bout en bout.** Les trois voisins du répertoire
  pèsent 196, 290 et 305 lignes, et un `MockLlmProvider` n'atteste que son propre
  script. La propriété qu'AC2 demande — « aucune ligne n'est écrite » — est tenue
  par un refus au niveau de `process_tool_calls`, attesté en 120 lignes de tests
  unitaires qui exercent le vrai site.
- **Pas de scénario de calibration mika-qa** (≈ 140 lignes avec sa fixture et son
  manifeste) : voir § *Non-Requirements*. L'instrument existe déjà en production.
- **Aucune ligne dans `qa-review/system_prompt.md`**, au plafond dur : la garde
  moteur remplace la consigne qu'il aurait fallu y écrire.

Un découpage en deux vols aurait séparé le prédicat de son site, ou le site de
ses tests — dans les deux cas un vol non mergeable seul, ce qui est précisément
ce que la règle interdit.

## Ordre de commit

Chaque étape laisse l'arbre compilable et la suite verte.

1. **`qa_head_reviews.sh` + `skill.toml`** — l'outil existe avant que le prompt
   le nomme. `make verify-bundled-skills` vert.
2. **`system_prompt.md`** (étapes 1, 3, 4) — le prompt lit l'outil.
3. **R5, les deux détecteurs de conformité** — écrits *après* 2, donc verts dès
   leur premier run, et leur contrôle négatif atteste qu'ils voient quelque chose.
4. **R3, le prédicat** seul, avec ses tests unitaires de R6 (1 à 5). Pur, sans
   appelant : la suite reste verte.
5. **R4, le site + le paramètre** à travers `process_tool_calls`, avec les tests
   6 et 7.
6. **Passe finale** — `cargo fmt`, `clippy`, suite complète,
   `scripts/verify-pipeline.sh origin/main`.

**Plancher (ce qui doit être dans la PR quoi qu'il arrive)** : étapes 1 à 4. Elles
ferment AC1 et donnent le prédicat testé. Si l'étape 5 devait déborder, elle part
en suivi nommé — mais AC2 et AC3 ne seraient alors pas tenus, donc ce n'est pas un
découpage prévu, c'est un plancher de sauvetage.

## Sonde post-déploiement (AC4, AC5)

Trois lectures. **La lecture 2 ci-dessous remplace la lecture 2 du plan de
mika#2927** ; les lectures 1 et 3 de ce plan-là restent valides et ne sont pas
reprises ici.

Préalables communs :

```bash
DB=~/.mika/data/mika.db
DEPLOY_AT=<instant ISO 8601 du make deploy, ex. 2026-10-28T09:12:00Z>
```

### Lecture 1 — la population (combien de lignes, et de quelle valeur)

```bash
sqlite3 "$DB" "
  SELECT after_value, count(*)
  FROM audit_events
  WHERE tool_name = 'qa_reviewed_head'
    AND created_at >= '$DEPLOY_AT'
  GROUP BY 1;
"
```

Attendu : des lignes, pas zéro. **Zéro n'est pas une réussite** — ce serait le
symptôme d'un registre qui n'écrit plus du tout, et la lecture 2 serait alors
vide par absence de population et non par absence de violation (classe mika#2205).
Si cette lecture rend zéro, la lecture 2 ne dit rien.

### Lecture 2 — la sonde temporelle (AC4), bornée au déploiement (AC5)

Pour **chaque** ligne, comparer `created_at` au `submitted_at` du **premier**
verdict de mika-qa sur cette tête.

```bash
sqlite3 -separator '|' "$DB" "
  SELECT target_key, after_value, created_at
  FROM audit_events
  WHERE tool_name = 'qa_reviewed_head'
    AND after_value IN ('verdict_posted','stop_already_reviewed_sha','stop_verdict_recorded')
    AND created_at >= '$DEPLOY_AT'
  ORDER BY created_at;
" | while IFS='|' read -r key outcome written; do
  # target_key : pr:{owner/repo}#{n}@{sha}
  repo=${key#pr:}; repo=${repo%%#*}
  rest=${key#*#}; pr=${rest%%@*}; sha=${rest#*@}
  first=$(gh api "repos/$repo/pulls/$pr/reviews" --paginate \
    --jq "[.[] | select(.user.login == \"mika-platform-qa\" and .commit_id == \"$sha\") | .submitted_at] | sort | first // empty")
  if [ -z "$first" ]; then
    echo "VIOLATION aucune-revue  $key  $outcome  écrite=$written"
  elif [ "$written" \< "$first" ]; then
    echo "VIOLATION avant-verdict $key  $outcome  écrite=$written  premier_verdict=$first"
  else
    echo "ok                      $key  $outcome  écrite=$written  premier_verdict=$first"
  fi
done
```

**Attendu (AC5) : aucune ligne `VIOLATION`**, sur une population non vide
comprenant au moins une PR poussée après une approbation.

Trois propriétés qui font la différence avec la lecture 2 de mika#2927, et qui
sont exactement les deux raisons de son angle mort plus une :

1. **La comparaison est temporelle**, pas existentielle : un verdict arrivé
   *après* la ligne est une violation, pas une excuse. C'est ce qui aurait attrapé
   `7418ef0c` (07:07:19Z contre 07:15:57Z).
2. **Les trois valeurs sont lues**, pas la seule `verdict_posted`. C'est ce qui
   aurait attrapé les trois `stop_verdict_recorded`.
3. **« Aucune revue » et « revue postérieure » sont deux verdicts distincts.**
   Un `0` d'un côté et un `0` de l'autre ne se somment pas : la première forme dit
   « la tête n'a jamais été statuée », la seconde « elle l'a été trop tard ».

Les deux comparaisons de date sont lexicographiques et c'est correct : les deux
champs sont en ISO 8601 UTC à largeur fixe (`%Y-%m-%dT%H:%M:%SZ` côté
`audit_events`, `Z`-suffixé côté API GitHub).

### Lecture 3 — la lecture de bonne foi (AC4)

La même lecture 2, **sans la borne `$DEPLOY_AT`**, sur les données du 2026-10-27.
Elle doit rendre **quatre** `VIOLATION avant-verdict`, et ce sont les quatre
lignes du tableau de contexte :

```
VIOLATION avant-verdict pr:senara-solutions/mika#2931@1f649bf1 stop_verdict_recorded écrite=2026-10-27T07:06:02Z premier_verdict=2026-10-27T07:15:57Z
VIOLATION avant-verdict pr:senara-solutions/mika#2931@1f649bf1 verdict_posted        écrite=2026-10-27T07:07:19Z premier_verdict=2026-10-27T07:15:57Z
VIOLATION avant-verdict pr:senara-solutions/mika#2920@f7542d43 stop_verdict_recorded écrite=2026-10-27T07:07:36Z premier_verdict=2026-10-27T07:16:49Z
VIOLATION avant-verdict pr:senara-solutions/mika#2928@13657863 stop_verdict_recorded écrite=2026-10-27T07:07:42Z premier_verdict=2026-10-27T07:24:54Z
```

Le ticket nomme trois lignes — celles de son tableau de mesure, qui sont les trois
`stop_verdict_recorded` du défaut 1. La quatrième est la ligne `verdict_posted` du
défaut 2, décrite dans le même ticket ; la sonde la rend parce qu'elle lit les
trois valeurs. **Si cette lecture rend moins de quatre violations, la sonde est
fausse et AC4 n'est pas tenu** — ne pas ajuster l'attendu, corriger la sonde.

La ligne du legs `#2920@0a55d28a` porte `after_value = 'review'`, hors du
vocabulaire canonique, donc hors de cette population. Elle reste surveillée par la
lecture 3 du plan de mika#2927, inchangée.

### Lecture 4 — la garde a-t-elle tiré (sonde de R4)

```bash
grep -c 'reviewed_head_write_without_evidence' ~/.mika/logs/mika-spirit.log
```

**Le zéro attendu est l'assertion que les prompts tiennent.** Un compte non nul
n'est pas une panne : c'est la garde qui fait son travail, et chaque ligne nomme
l'`outcome` et la variante de preuve manquante. Un compte non nul **persistant**
sur la variante `StopSansLectureDeTete` voudrait dire que le prompt de R2 n'est
pas suivi, et c'est le signal qui rouvrirait la question « consigne ou garde »
pour le défaut 1.

**L'absence totale de la ligne, elle, ne dit rien tant que `make deploy` n'a pas
eu lieu** — ni le prompt ni la garde n'existent pour un agent servi par un binaire
antérieur (motif mika#2340).

## Ordre avec mika#2920

1. Ce ticket merge et **déploie** (`make deploy`). Sans déploiement, ni le prompt
   corrigé ni la garde n'existent pour mika-qa.
2. La sonde d'AC5 tourne sur une fenêtre comprenant au moins une PR poussée après
   approbation, et rend zéro violation.
3. Alors seulement #2920 sort de draft, sous la condition du § *Le legs* :
   après le 2026-11-26, ou avec `MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD=off` jusqu'à
   cette date.

Tracked in: senara-solutions/mika#2920 (le lecteur et son interrupteur, hors
périmètre de ce plan).
