# mika#2927 — le registre n'atteste que le verdict posté

> Correctif du vol 2 de mika#2917 (#2919, commit `25e3b0f1`, déployé). Le lecteur
> — la classe (g′) de mika#2920 — **n'est pas mergé**. Ce plan est donc écrit pour
> atterrir sur `main` **avant** #2920, sans rien en attendre, et pour que #2920
> hérite de la propriété mécaniquement quand il atterrira.

## Contexte

Le registre `qa_reviewed_head` a inscrit `review` sur une tête qui n'a jamais été
revue. Mesuré le 2026-10-26 dans `audit_events` :

```
2026-10-26T15:18:30Z | mika-qa | qa_reviewed_head | pr:senara-solutions/mika#2920@0a55d28a9d6a43848aaadbb301bd3c236a477088 | review
```

Le tour qui l'a écrit — session `4f94fbc8` — a épuisé son budget de pas à
15:19:16Z (`max_steps_tool_census`). Il n'a lancé aucun build et n'a posté aucun
verdict. Aucune revue n'existe sur `0a55d28a` ; la seule revue de la PR est un
`COMMENTED` sur `83d47a50`.

La cause est à une ligne, et elle est dans le prompt :
`skills/bundled/qa-review-webhook-success/system_prompt.md:70` fait appeler
`record_reviewed_head(..., outcome: "review")` au **dispatch** de `qa-review`, et
non à sa conclusion. Le § *La perte de revue* de
`crates/mika-agent/src/qa_reviewed_head.rs:51-53` énonce déjà l'invariant violé —
« **L'écriture est postérieure à l'action.** […] Une ligne écrite d'avance serait
une tête déclarée traitée par une revue qui n'a pas eu lieu » — mais « après le
dispatch » n'est pas « après l'action » : le dispatch n'est pas la revue, c'est sa
promesse. Le vol 2 avait nommé ce résidu, et le plan du vol 3 le cite.

**Pourquoi maintenant.** Tant que personne ne lit le registre, la ligne fausse ne
coûte rien. Le jour où #2920 se déploie avec `MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD`
armé, elle devient un **silence permanent** sur cette tête, là où elle coûte
aujourd'hui un tour payé. Et la cause de mort est fréquente : 13 épuisements de
pas de mika-qa en 3,5 jours.

## Ce qui est refusé (bearing, non négociable)

- **Aucun appel `gh` depuis `webhook_prefilter.rs`** (mika#2680 / mika#2260). Ce
  plan n'en ajoute aucun et ne touche pas ce fichier. `GH_CALL_SITES_ALLOWED =
  &["pr_state"]` reste tel quel.
- **Un seul écrivain par `tool_name`.** Aucun écrivain Rust n'est ajouté :
  l'écriture reste un effet direct d'un appel d'outil du modèle, par
  `tools/record_reviewed_head.rs` et lui seul (AC3).
- **Aucun SQL à la main sur la base de production.** La ligne déjà inscrite est
  traitée par la règle choisie, pas par une rustine d'opérateur.
- **Aucune table, aucune migration, aucune DDL.** Le registre reste
  `audit_events` sans schéma, comme `qa_head_supersession`.
- **Le vocabulaire de l'attestation mika#2855 ne change pas.** Ses six valeurs
  `Webhook-Outcome:` sont un enum **distinct**
  (`server/webhook_turn_outcome.rs`) ; aucune n'est ajoutée, retirée ni renommée.

## La règle choisie, et celle qui est rejetée

Le ticket délègue le choix au plan, entre deux formulations :

> `review` n'est inscrit qu'une fois le verdict posté sur cette tête, **ou bien**
> n'écarte rien tant qu'un verdict de mika-qa n'existe pas sur ce `commit_id`.

### Rejetée — « n'écarte rien tant qu'un verdict n'existe pas sur ce `commit_id` »

Cette formulation déplace la vérification du côté **lecteur** : avant d'écarter,
le pré-filtre irait confirmer qu'un verdict de `mika-platform-qa` porte ce
`commit_id`. Deux raisons de la refuser, et la première suffit :

1. **Elle exige un appel à la forge depuis le pré-filtre**, que le bearing de
   mika#2680 interdit. L'unique site `gh` de `webhook_prefilter.rs` (`pr_state`)
   est un legs tracké pour retrait, pas une porte ouverte.
2. **Il n'existe aucune trace locale du verdict par tête.** Le verdict vit sur
   GitHub ; mika-qa ne l'enregistre nulle part par `commit_id` — c'est précisément
   le manque que le vol 2 comblait. La formulation suppose donc l'existence du
   registre qu'elle prétend vérifier.

Et elle laisse le défaut intact : la ligne `review` resterait écrite au dispatch,
donc **fausse**, simplement non crue par un lecteur qui paie un aller-retour forge
pour ne pas la croire. Une ligne d'audit qui atteste ce qui n'a pas eu lieu est un
défaut en soi, indépendamment de qui la lit.

### Retenue — l'écriture a lieu au verdict, et le littéral du dispatch est retiré du vocabulaire

Deux gestes, et le second est ce qui rend le premier complet :

1. **Le site d'écriture se déplace du dispatch à la conclusion.** Le tour
   `qa-review-webhook-success` qui dépêche la revue **n'écrit plus rien**. C'est le
   tour qui a posté la revue sur la forge — `qa-review` Step 5, ou
   `qa-review-build-callback` Step 5 quand la revue est passée par un build — qui
   appelle l'outil, après que `run_gh("pr review …")` a rendu un succès.
2. **Le littéral `review` quitte `REVIEWED_HEAD_OUTCOMES`**, remplacé par
   `verdict_posted`. Le vocabulaire ne contient plus de mot pour « revue
   dépêchée », donc l'état « dépêchée » cesse d'être représentable comme une tête
   traitée — la propriété est structurelle, pas une discipline de prompt (c'est le
   point 2 du § *La perte de revue* appliqué à lui-même).

**Et c'est ce second geste qui traite la ligne déjà inscrite, sans SQL.**
`reviewed_head_outcome("review")` rend alors `None`. Le lecteur du vol 3 écarte un
tour sur la condition littérale `reviewed_head_outcome(&raw).is_some()` : la ligne
de `#2920@0a55d28a` n'est plus reconnue, elle est relâchée sous la *release*
`reviewed_head_out_of_contract` que #2920 porte déjà, et le `check_suite` suivant
sur cette tête passe. La règle traite la donnée ; l'opérateur ne touche pas la
base. Toute autre ligne `review` antérieure au déploiement est traitée
identiquement, sans qu'il faille les énumérer.

### Ce que le déplacement gagne au-delà du correctif

La clé écrite devient la tête sur laquelle le verdict **est posté** — le
`headRefOid` que `qa_pr_view` a rendu au Step 1 et que Step 3e.2 utilise déjà pour
le build, qui est aussi le `commit_id` que la forge enregistre sur la revue. Si un
push a eu lieu entre le `check_suite` et le verdict, la ligne nomme la tête revue
et non celle de l'événement : la clé et le fait coïncident exactement, ce qui est
aussi ce qui rend la sonde d'AC4 exacte.

La propriété s'étend par ailleurs aux revues qui n'entrent pas par le webhook
(`requested_reviewers`, demande opérateur) : toute revue de mika-qa inscrit
désormais sa tête. Le registre cesse d'être « les têtes qu'un tour de
`qa-review-webhook-success` a conclues » pour devenir « les têtes sur lesquelles
mika-qa a statué », ce qui est la propriété que le lecteur veut réellement.

### Pourquoi l'outil reste atteignable depuis ces deux prompts

`record_reviewed_head` est enregistré globalement (`tools/mod.rs:1458`) : c'est un
builtin, pas un outil de skill, et l'allowlist d'identité de mika-qa gouverne des
skills. L'atteignabilité est de plus **attestée par l'incident lui-même** — la
session `4f94fbc8` est un tour de mika-qa qui a appelé cet outil avec succès.
Aucune déclaration `required_tools` n'est ajoutée : un `required_tools` non résolu
est un échec **terminal**, donc une revue perdue là où une écriture manquée ne
coûte qu'un tour (`qa_reviewed_head.rs:63-66`).

Si un autre agent exécutait `qa-review`, sa ligne atterrirait sous son propre
`agent_id` — injecté par `AsyncDatabase::log_audit_event`, jamais passé par
l'appelant — donc invisible au lecteur de mika-qa. Le cloisonnement reste une
propriété de construction.

## Requirements

### R1 — `crates/mika-agent/src/qa_reviewed_head.rs` : le vocabulaire

- `REVIEWED_HEAD_OUTCOMES` devient `&["verdict_posted", "stop_already_reviewed_sha",
  "stop_verdict_recorded"]`. Trois valeurs, toujours : `verdict_posted` **remplace**
  `review`, il ne s'y ajoute pas.
- La doc de la constante dit ce que chaque littéral atteste et **par quel tour** il
  est écrit : les deux `stop_*` par le tour webhook (ils sont aussi deux des six
  valeurs `Webhook-Outcome:`), `verdict_posted` par le tour qui a posté la revue,
  et il n'a **pas** de contrepartie dans l'attestation mika#2855 — parce que le tour
  qui l'écrit n'est pas un tour de webhook et n'émet aucune ligne
  `Webhook-Outcome:`. Cette asymétrie est délibérée et doit être écrite, sans quoi
  elle se lira comme un oubli.
- Doc de module : le titre et le § *La perte de revue* sont réécrits. Le registre
  est « les têtes sur lesquelles mika-qa a statué ». Le point 1 (« L'écriture est
  postérieure à l'action ») cesse de dire « après la revue dispatchée » et dit
  « après le verdict posté » ; l'incident du 2026-10-26 y est nommé par sa date,
  sa session et sa clé, comme la mesure qui a montré que « dispatché » n'est pas
  « fait ».
- La ligne 18 de la doc de module décrit `reasoning = "request_id=<uuid>"`, ce que
  l'outil n'écrit pas. La corriger pour qu'elle décrive la chaîne réellement
  écrite (R2).

### R2 — `crates/mika-agent/src/tools/record_reviewed_head.rs` : l'écrivain unique

Aucun changement de structure : même outil, même nom, même clé, mêmes quatre
refus de champ. Trois corrections de **vérité du texte** :

- L'`enum` du schéma suit `REVIEWED_HEAD_OUTCOMES` (le test
  `mika2917_le_schema_nomme_exactement_les_trois_issues` l'épingle déjà).
- La `description` cesse de dire « avec la MÊME valeur que la ligne
  `Webhook-Outcome:` de ce tour » : c'est vrai des deux `stop_*`, faux de
  `verdict_posted`. Elle dit désormais quand appeler — à la fin d'un tour qui a
  constaté un stop, ou après un `pr review` réussi — et que `head_sha` est la tête
  sur laquelle le tour a statué.
- Le `reasoning` écrit cesse de nommer `qa-review-webhook-success`, puisque deux
  autres skills l'appellent désormais : il nomme l'issue et la tête, pas le skill.
- La doc de module cesse de dire « Le skill `qa-review-webhook-success`
  l'appelle » et nomme les trois sites.

### R3 — `skills/bundled/qa-review-webhook-success/system_prompt.md` : le dispatch n'écrit plus

- Le § *Record the head you concluded on* perd sa troisième ligne de tableau
  (`step 5 → outcome: "review"`, l. 70). Il ne reste que les deux stops.
- Son **When** cesse de dire « ou après step 5 ».
- Une phrase explicite dit que le dispatch d'une revue n'enregistre **rien**, et
  pourquoi : une revue dépêchée peut mourir avant son verdict — mesuré le
  2026-10-26, session `4f94fbc8` — et c'est le tour qui poste le verdict qui
  inscrit la tête. Sans cette phrase, l'absence du step 5 dans le tableau se lit
  comme un oubli et le modèle l'invente.
- La liste « les autres issues n'enregistrent rien » accueille `review` (step 5)
  à côté de `stop_no_open_pr`, `stop_out_of_scope` et `review_failed`, avec sa
  raison propre : la revue n'a pas encore abouti.
- Le § *Outcome attestation (mika#2855)* et ses six lignes sont **intouchés**.

### R4 — `skills/bundled/qa-review/system_prompt.md` : Step 5 inscrit la tête

- Après le bloc de post du verdict, un **Step 5b** : une fois
  `run_gh("pr review …")` revenu en succès, appeler
  `record_reviewed_head(repo, pr, head_sha: <headRefOid du Step 1>, outcome:
  "verdict_posted")`.
- La condition est **le succès du post**, dite littéralement : un `POST_FAILED`
  (l. 678) n'inscrit **rien**, parce qu'aucun verdict n'est visible sur cette tête
  et qu'un tour futur doit encore la revoir.
- `head_sha` est le `headRefOid` déjà en main (Step 1 `qa_pr_view`, réutilisé en
  3e.2) : **aucun appel supplémentaire à la forge**.
- Mêmes consignes d'échec que le site webhook : un échec de l'outil ne se
  retente pas, ne s'escalade pas, et ne change pas le verdict. Une ligne
  manquante coûte un tour payé ; une escalade transformerait un manque de
  comptabilité en interruption d'opérateur.

### R5 — `skills/bundled/qa-review-build-callback/system_prompt.md` : le même Step 5b

Le chemin build poste son verdict **dans le tour de callback** : `qa-review`
termine son tour sur `build_mika` (l. 632) et le callback reprend avec son propre
Step 5. Le Step 5b y est donc **dupliqué**, mot pour mot, avec la même condition de
succès. Deux sites, parce qu'il y a deux tours qui postent — et la divergence des
deux textes est tenue par le détecteur de R6, pas par la vigilance.

### R6 — les détecteurs

Dans `qa_reviewed_head.rs`, module `tests` :

1. **`mika2927_le_vocabulaire_du_registre_nadmet_plus_la_graphie_du_dispatch`** —
   `reviewed_head_outcome("review")` rend `None`, et `REVIEWED_HEAD_OUTCOMES` ne
   contient pas `"review"`. Auto-nettoyant par construction : le jour où quelqu'un
   remet le littéral du dispatch dans le vocabulaire, il rougit, et il n'y a
   aucune liste d'exceptions qui puisse devenir stale.
2. **La conformité des prompts, étendue à trois fichiers.** Trois
   `include_str!` (motif mika#2340 : `skills/bundled/` est lu au build, donc une
   édition de l'arbre est invisible aux agents jusqu'au `make deploy`), et une
   égalité d'ensemble **par fichier**, parce que la propriété n'est pas la même
   sur chacun :
   - `qa-review-webhook-success` : `issues_appelees` rend exactement
     `["stop_already_reviewed_sha", "stop_verdict_recorded"]` — et **pas**
     `verdict_posted`, ce qui est la propriété que ce ticket achète.
   - `qa-review` : exactement `["verdict_posted"]`.
   - `qa-review-build-callback` : exactement `["verdict_posted"]`.
   - L'union des trois égale `REVIEWED_HEAD_OUTCOMES` : aucun littéral du
     vocabulaire n'est orphelin de prompt, aucun prompt ne passe un littéral hors
     vocabulaire. La bidirectionnalité est ce qui empêche l'assertion d'être
     l'allowlist que mika#2309 condamne.
   - Chaque littéral appelé est revérifié par le prédicat de vérité lui-même
     (`reviewed_head_outcome(issue) == Some(issue)`), comme aujourd'hui.
   - Les six lignes `Webhook-Outcome:` restent assertées présentes dans le prompt
     webhook.
   - `issues_appelees` est réutilisé tel quel : il lit la valeur citée de
     `outcome: "…"` sur les seules lignes portant `record_reviewed_head(`, parce
     que `review` est un sous-mot de `review_failed` et de
     `stop_already_reviewed_sha`. Son contrôle négatif
     (`mika2917_le_scan_du_prompt_mord_sur_une_issue_hors_contrat`) reste en place.
3. **Le succès du post est une condition écrite, et elle est épinglée.** Chacun
   des deux prompts de verdict doit porter, sur ou près de la ligne d'appel, la
   condition de succès du `pr review`. Un test l'asserte par recherche littérale
   dans les deux `include_str!` — faible, et dit comme tel : il tient la présence
   de la condition, pas son respect par le modèle. Ce que le moteur tient
   réellement est ailleurs et existe déjà : la garde `qa_build_callback_verdict`
   refuse un EndTurn de callback sans `run_gh pr review` réussi dans l'historique
   du tour.

### R7 — les tests d'acceptation

Dans `qa_reviewed_head.rs` pour AC1, dans `tools/record_reviewed_head.rs` pour
AC2 (l'écriture passe par l'outil réel, `TestHarness::new()` + `ctx.db
.get_audit_events`).

- **AC1, un seul test, les deux contrôles dedans.** Le corpus de `4f94fbc8` est
  reconstitué par sa forme mesurée : une ligne écrite dans la base de test sous
  `QA_REVIEWED_HEAD_TOOL`, à la clé `head_key("senara-solutions/mika", 2920,
  "0a55d28a9d6a43848aaadbb301bd3c236a477088")`, avec `after_value = "review"`.
  Relue, `reviewed_head_outcome` la refuse — donc la condition d'écartement du
  lecteur, `reviewed_head_outcome(&raw).is_some()`, est **fausse**, et le
  `check_suite` suivant sur cette tête passe. Contrôle positif dans le même
  test : une seconde tête écrite par l'outil réel avec `verdict_posted`, relue,
  reconnue — donc écartée.
- **AC2, un test par valeur**, chacun par l'outil réel : la ligne atterrit à la
  clé de `head_key`, `after_value` porte le littéral **canonique** (graphie libre
  de l'appelant normalisée), et la valeur est reconnue par
  `reviewed_head_outcome`. Les quatre refus de champ et le refus d'une issue hors
  vocabulaire restent testés tels quels, `"review"` rejoignant les valeurs
  refusées.
- **AC3 ne reçoit aucun test nouveau, et c'est l'assertion.**
  `mika2917_a2_un_seul_ecrivain_et_au_plus_un_lecteur` passe **sans qu'une ligne
  en change** : l'ensemble exact des écrivains reste
  `["tools/record_reviewed_head.rs"]`, le `readers.len() <= 1` reste vrai (zéro
  lecteur sur `main`), et aucun fichier ne recopie le littéral du `tool_name`. Le
  diff de ce plan ne touche pas ce test — c'est la vérification.

**Sur la fidélité d'AC1 à sa lettre.** AC1 dit « n'écarte pas le `check_suite`
suivant ». Le décideur qui écarte vit dans #2920, qui n'est pas mergé : sur `main`
aujourd'hui, **personne ne lit** ce registre (`qa_reviewed_head.rs:87`,
« Lecteur : aucun dans ce vol »). AC1 est donc tenu sur le prédicat qui **est** la
condition d'écartement, nommé littéralement dans le plan et dans le commentaire du
test : #2920 écarte sur `Ok(Some(raw)) if
crate::qa_reviewed_head::reviewed_head_outcome(&raw).is_some()`. La classe (g′)
hérite de la propriété mécaniquement le jour où elle atterrit, sans ligne de test
à y ajouter. Attendre #2920 pour tenir AC1 contredirait l'urgence que le ticket
énonce — c'est précisément pour passer **avant** lui que ce ticket existe.

## Non-Requirements (hors de ce plan, délibérément)

- **Aucune modification de `webhook_prefilter.rs`**, donc aucune reprise du vol 3.
  #2920 garde son diff.
- **Aucun changement aux six valeurs de mika#2855**, ni à
  `server/webhook_turn_outcome.rs`. Son
  `PROMPT_PARITY_ALLOWLIST` reste vide et ses tests restent verts : les lignes
  `Webhook-Outcome:` du prompt ne bougent pas.
- **Aucune trace locale du verdict par `commit_id` au-delà de ce registre.** Le
  verdict reste la revue sur la forge comme source de vérité.
- **Aucun retrait du site `gh` legacy `pr_state`** — tracké ailleurs.
- **Aucune correction des 13 épuisements de pas de mika-qa.** La cause de mort est
  le ticket compagnon ; ce plan rend seulement la mort inoffensive pour le
  registre.
- **Aucun SQL sur la base de production**, ni dans le plan ni dans la sonde : la
  sonde **lit**.

## Verification Contract

- `cargo test -p mika-agent qa_reviewed_head` — vocabulaire, conformité des trois
  prompts, AC1, contrôles négatifs.
- `cargo test -p mika-agent record_reviewed_head` — schéma, quatre refus, AC2.
- `cargo test -p mika-agent webhook_turn_outcome` — les six valeurs mika#2855 sont
  intactes.
- `cargo test -p mika-agent` — la garde d'unicité de mika#2917 passe sans
  modification.
- `make verify-bundled-skills` — les trois bundles restent structurellement
  complets.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- `scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- `REVIEWED_HEAD_OUTCOMES` ne contient plus `"review"` et contient
  `"verdict_posted"` ; `reviewed_head_outcome("review")` rend `None`.
- `qa-review-webhook-success/system_prompt.md` n'appelle plus
  `record_reviewed_head` au dispatch, et dit pourquoi.
- `qa-review/system_prompt.md` et `qa-review-build-callback/system_prompt.md`
  appellent `record_reviewed_head(..., outcome: "verdict_posted")` après un
  `pr review` réussi, et pas autrement.
- `tools/record_reviewed_head.rs` reste le seul écrivain ; sa `description` et son
  `reasoning` ne nomment plus un seul skill ni la ligne `Webhook-Outcome:` comme
  contrainte universelle.
- Les détecteurs de R6 passent, et leurs contrôles négatifs mordent.
- Les tests de R7 passent ; `mika2917_a2_un_seul_ecrivain_et_au_plus_un_lecteur`
  passe sans que le diff ne le touche.
- Les six valeurs `Webhook-Outcome:` sont inchangées et toujours assertées.
- La suite complète, clippy, fmt, `make verify-bundled-skills` et
  `scripts/verify-pipeline.sh` sont verts.
- La sonde post-déploiement ci-dessous est dans le corps de la PR, exécutable
  telle quelle par l'opérateur.

## Acceptance criteria

- **AC1.** Le corpus de la session 4f94fbc8 (`review` dépêché, tour épuisé, aucun
  verdict posté) n'écarte pas le `check_suite` suivant sur la même tête. Le
  contrôle positif est dans le même test : une tête dont le verdict est posté est
  écartée.
- **AC2.** `stop_already_reviewed_sha` et `stop_verdict_recorded` gardent leur
  comportement, testés chacun.
- **AC3.** Le seul écrivain reste le seul, et la garde d'unicité de mika#2917
  passe sans modification de son ensemble.
- **AC4, la sonde post-déploiement**, écrite dans le plan : après `make deploy`,
  aucune ligne `review` n'existe sur une tête sans verdict de mika-qa. La commande
  qui croise `audit_events` et les revues de la forge est donnée.

## Fire-Disposition

Ce plan livre des détecteurs : le refus de vocabulaire
(`mika2927_le_vocabulaire_du_registre_nadmet_plus_la_graphie_du_dispatch`), la
conformité des trois prompts embarqués, et l'épinglage de la condition de succès.
Leur chemin de succès est « aucune violation trouvée ».

**Option (a) — exception nommée en allowlist, et l'allowlist est LIVRÉE VIDE.**

Le précédent est dans ce même périmètre et porte la même forme :
`PROMPT_PARITY_ALLOWLIST` dans `server/webhook_turn_outcome.rs:273`, livrée vide
avec sa raison écrite et son assertion auto-nettoyante. La forme est reprise
telle quelle, sur le détecteur de conformité des prompts :

```rust
/// Lignes d'un prompt embarqué qui appellent `record_reviewed_head` avec un
/// littéral hors du vocabulaire du registre. **LIVRÉE VIDE** : les trois
/// prompts sont corrigés dans ce même commit, donc aucune occurrence hors
/// contrat n'existe à l'atterrissage — le détecteur atterrit ARMÉ.
///
/// Toute entrée future doit nommer la ligne exacte, référencer son ticket de
/// suivi, et disparaître quand la ligne disparaît : l'assertion
/// auto-nettoyante ci-dessous rougit sur une entrée stale.
const PROMPT_OUTCOME_ALLOWLIST: &[(&str, &str)] = &[];
```

et, dans le test, l'assertion qui la nettoie :

```rust
for (fichier, ligne) in PROMPT_OUTCOME_ALLOWLIST {
    assert!(
        prompt_de(fichier).contains(ligne),
        "exception stale : {ligne:?} n'est plus dans {fichier} — retirer l'entrée"
    );
}
```

Pourquoi elle est vide, précisément :

- **La seule violation au niveau du code est la ligne 70 du prompt webhook, et le
  même commit la retire.** Aucune autre ligne de prompt n'appelle l'outil avec un
  littéral hors vocabulaire : l'ensemble rendu par `issues_appelees` sur les trois
  prompts est exactement le vocabulaire, par construction du diff.
- **Le détecteur de vocabulaire n'a pas d'allowlist, et ne doit pas en avoir.**
  Il asserte l'absence d'un littéral dans une constante de trois éléments ; une
  exception y serait le défaut lui-même. Sa propriété auto-nettoyante est l'égalité
  elle-même, pas une liste.
- **La seule violation vivante est une DONNÉE, hors de portée de tout test.** La
  ligne `review` sur `#2920@0a55d28a` est dans la base de l'hôte ; un test
  unitaire ne lit pas `~/.mika/data/mika.db`, et il ne doit pas. Elle n'est donc
  pas une entrée d'allowlist : elle est **traitée par la règle** — le vocabulaire
  ne la reconnaît plus, le lecteur du vol 3 la relâche — et **attestée par la
  sonde d'AC4**, qui est le détecteur de cette classe, à l'échelle où elle existe.
  Cette asymétrie est le cœur de la disposition : code armé et vide, donnée
  couverte par une sonde d'opérateur.

Aucun détecteur n'atterrit désarmé ; aucun `#[ignore]`.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/qa_reviewed_head.rs` (vocabulaire + doc de module + R6 + AC1) | 170 |
| `crates/mika-agent/src/tools/record_reviewed_head.rs` (schéma, description, `reasoning`, doc, AC2) | 55 |
| `skills/bundled/qa-review-webhook-success/system_prompt.md` | 30 |
| `skills/bundled/qa-review/system_prompt.md` | 18 |
| `skills/bundled/qa-review-build-callback/system_prompt.md` | 18 |

Total estimé : 291 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 291 est sous la borne par vol (429) et
très loin du seuil par PR (1000). Le volume est concentré dans un seul fichier
Rust déjà lu et déjà structuré : aucun module nouveau, aucun type nouveau, aucune
signature de trait touchée, aucune migration, aucun fichier créé. Les deux tiers
du total sont du test et de la doc dans un module dont les patrons — `include_str!`
embarqué, `issues_appelees`, égalité d'ensemble, contrôle négatif apparié — sont
déjà écrits et se dupliquent par fichier plutôt que de s'inventer. Les trois
éditions de prompt sont chacune d'une section. Rien à découper : un découpage en
vols livrerait soit le vocabulaire sans les prompts (le modèle appellerait un
littéral refusé), soit les prompts sans le vocabulaire (la ligne du dispatch
resterait reconnue) — les deux moitiés ne sont pas mergeables seules, ce qui est
exactement le critère qui interdit de les séparer.

## Ordre de commit

Un seul commit, parce que les deux moitiés ne sont pas séparément correctes.
Ordre d'écriture à l'intérieur :

1. `qa_reviewed_head.rs` — le vocabulaire et sa doc. À ce point, le test de
   conformité du prompt webhook **rougit** : c'est le détecteur qui nomme le
   travail restant.
2. Les trois prompts (R3, R4, R5).
3. Les détecteurs étendus et les tests d'acceptation (R6, R7).
4. `tools/record_reviewed_head.rs` — schéma, `description`, `reasoning`, doc.
5. Suite complète, clippy, fmt, `make verify-bundled-skills`,
   `scripts/verify-pipeline.sh origin/main`.

## Sonde post-déploiement (AC4)

**Opérateur, après `make deploy`** — hors de portée du pilote : elle lit la base
de l'hôte et appelle la forge. Elle **lit** ; aucune écriture, aucun SQL de
modification.

Noter l'instant du déploiement, qui sert de borne aux deux lectures :

```
DEPLOY_AT=2026-10-27T00:00:00Z   # remplacer par l'instant réel du make deploy
```

**Lecture 1 — aucune ligne `review` n'est écrite après le déploiement.** Le
vocabulaire ne contient plus ce littéral, donc l'outil le refuse ; une ligne
postérieure au déploiement signifierait un second écrivain.

```sh
sqlite3 -noheader ~/.mika/data/mika.db \
  "SELECT created_at, target_key, after_value
     FROM audit_events
    WHERE agent_id = 'mika-qa'
      AND tool_name = 'qa_reviewed_head'
      AND after_value = 'review'
      AND created_at >= '$DEPLOY_AT';"
```

Attendu : **aucune ligne**. Les lignes `review` antérieures au déploiement — dont
celle de `#2920@0a55d28a` — restent en base comme legs, hors contrat, relâchées
par le lecteur : leur présence est la règle qui opère, pas un reste à nettoyer. On
les voit en retirant la clause `created_at`.

**Lecture 2 — toute ligne de conclusion porte un verdict de mika-qa sur cette
tête.** Le croisement `audit_events` × revues de la forge, sur la fenêtre de 30
jours que le lecteur du vol 3 utilise (`REVIEWED_HEAD_WINDOW_DAYS`) — au-delà, une
ligne est de toute façon invisible au lecteur :

```sh
sqlite3 -noheader -separator '|' ~/.mika/data/mika.db \
  "SELECT target_key, after_value
     FROM audit_events
    WHERE agent_id = 'mika-qa'
      AND tool_name = 'qa_reviewed_head'
      AND after_value = 'verdict_posted'
      AND created_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-30 days')
    ORDER BY created_at;" \
| while IFS='|' read -r key outcome; do
    rest="${key#pr:}"
    repo="${rest%%#*}"
    tail="${rest#*#}"
    pr="${tail%%@*}"
    sha="${tail#*@}"
    n=$(gh api "repos/$repo/pulls/$pr/reviews" --paginate \
          --jq "[.[] | select(.user.login == \"mika-platform-qa\" and .commit_id == \"$sha\")] | length" \
        || echo "ERR")
    printf '%s\t%s\tverdicts_mika_qa_sur_cette_tete=%s\n' "$key" "$outcome" "$n"
  done
```

Attendu : **chaque ligne porte `verdicts_mika_qa_sur_cette_tete` ≥ 1**. Un `0` est
la violation exacte que ce ticket ferme — une tête déclarée statuée sans verdict
visible. Un `ERR` est un échec de lecture de la forge (réseau, droits), pas un
constat : relancer, ne pas le compter comme un zéro.

**Lecture 3 — contrôle de bonne foi de la sonde.** Une sonde qui ne voit rien est
indiscernable d'une sonde qui ne trouve rien. La même boucle, sur `after_value =
'review'` et sans borne de date, doit faire apparaître la ligne de
`#2920@0a55d28a` avec `verdicts_mika_qa_sur_cette_tete=0` : c'est l'incident
d'origine, relu par l'instrument, et la preuve que les Lectures 1 et 2 regardent
au bon endroit.

## Ordre avec mika#2920

À trancher par Vincent, avant ou au merge de #2920 :

- soit ce ticket passe avant — et alors la ligne de `#2920@0a55d28a` est déjà hors
  contrat quand le lecteur arrive, sans geste d'opérateur ;
- soit #2920 se déploie avec `MIKA_WEBHOOK_PREFILTER_REVIEWED_HEAD=off` jusqu'à ce
  que ce ticket soit déployé.

L'interrupteur a été conçu pour ce cas. Ce plan est écrit pour la première
branche : il n'attend rien de #2920, ne touche pas son diff, et #2920 hérite de la
propriété par sa condition d'écartement existante.

Le merge revient à Vincent (`loop-substrate`).
