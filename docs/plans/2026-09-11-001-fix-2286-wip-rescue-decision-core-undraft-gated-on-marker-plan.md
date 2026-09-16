---
issue: 2286
repo: senara-solutions/mika
type: fix
module: mika-agent/wip_rescue
tags: [wip-rescue, decision-core, perimeter, rescue-pipeline-verified, fail-open, audit-events, loop-substrate, qa-review]
problem_type: fail-open-gate
status: groomed
---

# mika#2286 — l'auto-resume un-drafte une PR DecisionCore avec le marqueur à `no`

**Issue :** senara-solutions/mika#2286
**Branche :** `bug/2286/wip-rescue-auto-resume-un-drafte-une-pr`
**Palier :** Tier 1 — brèche sur le cœur de décision (ratifié p1 par Vincent le 2026-09-11 après bearing Prime : « ce n'est pas une amélioration, c'est la fermeture d'une brèche »).

**Revérifié contre `main` le 2026-09-16** (le worktree ayant été recréé depuis `main` après le
grooming du 11/09). Aucun fait du plan n'a bougé : `grep -c rescue-pipeline-verified
crates/mika-agent/src/wip_rescue.rs` rend toujours **0** — le démon ignore toujours le marqueur ;
`classify_route` est toujours appelé à `wip_rescue.rs:669`, le `gh pr ready` de l'étape 7 à `:672`,
le commentaire hand-merge DecisionCore à `:698` ; `select_eligible` occupe toujours `:531-562` avec
son unique prédicat injecté `is_bailed` ; le garde-fou LLM `validate_pr_ready_undraft_scope`
(mika#1682) est toujours à `builtin_handlers.rs:2571` ; la prémisse QA « operator un-drafted it »
est toujours à `qa-review/system_prompt.md:127`. Deux pointeurs de ligne seulement ont dérivé et
sont corrigés ci-dessous (`_compose_rescue_pr_body` 5645 → 5735, `WIP_RESCUE_CRON` 103 → 104).
La brèche est donc toujours ouverte à l'identique.

---

## 1. Le fait, relu dans le code

Le démon `wip_rescue` (`crates/mika-agent/src/wip_rescue.rs`) classe chaque brouillon **puis**
l'un-drafte, quelle que soit la classe :

```rust
// Step 6: substrate-diff perimeter classification (AC4/AC5). Fail-closed —
// a fetch/parse failure classifies DECISION-CORE (never auto-merge).
let route = classify_route(pr.number, token).await;                // :669

// Step 7: un-draft (F1). Any failure bails; the draft stays a draft.
if let Err(e) = gh(&["pr", "ready", &pr.number.to_string(), …], token).await { … }   // :672-694

// DECISION-CORE → leave a hand-merge comment (do NOT auto-merge).
if route == UndraftRoute::DecisionCore { /* gh pr comment … */ }   // :698-720
```

La classification ne conditionne **que le commentaire**, jamais l'un-draft. Le marqueur
`<!-- rescue-pipeline-verified: no -->` que dispatch-lib pose dans le corps
(`skills/bundled/_shared/dispatch-lib.sh:5735`, `_compose_rescue_pr_body`) n'est lu **nulle
part** dans ce fichier — `grep rescue-pipeline-verified crates/mika-agent/src/wip_rescue.rs`
rend zéro ligne. Le démon ne sait pas que ce marqueur existe.

Ce comportement n'est pas un accident : c'est la lettre de mika#1852 (RT#004, étape 6 de la spec :
« If ANY DECISION-CORE file → un-draft PR + comment "ready-for-Vincent-review" »). Mais la spec
plaçait l'un-draft **après** l'étape 5 « Re-run pilot » — c'est-à-dire après une re-vérification
du pipeline. La v1 a déscopé les étapes 4-fix et 5 (module doc § *Scope boundary (v1)*) et gardé
l'un-draft. Ce qui restait « vérifié » dans la spec ne l'est plus dans le code, et le marqueur qui
devait le dire est ignoré. C'est le trou du ticket.

### Pourquoi ça court-circuite aussi la QA

`skills/bundled/qa-review/system_prompt.md:126-130` (Step 1.5, item 4) considère la PR comme
vérifiée si **l'une** de ces conditions tient :

> - The marker reads `yes`
> - The PR `isDraft` field is `false` (**operator un-drafted it** — this is a stronger signal than any body marker)

La deuxième prémisse est vraie avant #1852 (seul un humain pouvait un-drafter une PR de secours —
l'outil LLM est bloqué par mika#1682) et fausse depuis : le démon un-drafte aussi, et la QA ne
distingue pas les deux gestes. Le `gh pr ready` du démon est donc, aux yeux de la QA, un geste
humain de vérification. Le marqueur ne gate plus rien pour la classe DecisionCore.

## 2. La trace, mesurée (PR #2285, secours de mika#2023)

Timeline `gh api …/issues/2285/timeline` + revues :

| Heure (Z) | Acteur | Événement |
|---|---|---|
| 17:42:57 | mika-platform-dev | label `wip-rescue` posé (dispatch-lib, PR ouverte en draft, marqueur `no`) |
| 17:43:18 | mika-platform-qa | `VERDICT: hold[review]` — « still in draft with pipeline-verification marker set to no » |
| 18:00:01 | démon | `wip_rescue_resume_attempt pr=2285 age_secs=1028` |
| 18:01:02 | mika-platform-dev | `ready_for_review` (le `gh pr ready` de l'étape 7) |
| 18:01:03 | mika-platform-dev | commentaire « classified this PR as DECISION-CORE … needs a Vincent hand-merge » |
| 18:01:17 | démon | `wip_rescue_success classification=DecisionCore` |
| 18:08:20 | mika-platform-qa | `VERDICT: pass` — code-level, 7 AC ✅ |
| 18:09:36 | samidarko | merge |

Le marqueur du corps était encore `no` à 18:01:02 (il lit `yes` aujourd'hui : édité après coup).
La QA a tenu la porte à 17:43 et l'a ouverte à 18:08 **sans qu'aucun humain ait touché la PR
entre les deux** — la seule différence d'état est l'un-draft du démon. Le fix de #2285 était bon
et la QA a relu pour de vrai ; c'est de la chance, pas une garantie (Prime).

### Point de mesure demandé par Prime : un site ou plusieurs ?

**Un seul.** Recensement de tous les chemins qui peuvent faire passer une PR de draft à ready :

| Surface | Site | État |
|---|---|---|
| démon `wip_rescue` | `wip_rescue.rs:672` (`gh pr ready`) — **seul** `"ready"` argv de `gh` dans `crates/` | **le trou** |
| outil LLM (`gh pr ready` via `builtin_handlers`) | `validate_pr_ready_undraft_scope` (`builtin_handlers.rs:2571`, mika#1682) | déjà **bloqué** pour toute PR `wip-rescue` ou tête `wip(` |
| dispatch-lib.sh | — | n'un-drafte jamais (`grep 'pr ready' skills/` : zéro) |
| humain (`gh` / UI) | — | c'est le geste attendu, il reste ouvert |

La garde est un `if` à un seul site — comme le ticket le pressentait.

## 3. La décision de conception

Le ticket propose (a) ne jamais un-drafter DecisionCore en auto, ou (b) un-drafter DecisionCore
**seulement** si le marqueur est `yes` ou sur geste humain. **Ce plan livre (b)**, dont (a) est le
sous-ensemble : (b) garde une voie automatique quand l'humain a fait exactement ce que le corps de
la PR lui demande (« set the marker above to `yes` ») sans avoir à un-drafter lui-même — le corps
de secours promet déjà cette équivalence, autant la tenir.

Trois contraintes structurent l'implémentation :

1. **Fail-closed sur le marqueur.** Seul le littéral `yes` déverrouille. `no`, absent, autre
   valeur, corps illisible → non vérifié. Même forme que le fix de #2285 lui-même (« une valeur de
   tier inconnue échoue fermée »). Le repli « pas de marqueur = compatibilité pré-#1618 » que la QA
   tolère n'a pas sa place ici : toutes les PR `wip-rescue` sont produites par dispatch-lib, qui
   pose le marqueur depuis le 2026-06-29.

2. **Une PR laissée en draft doit sortir de l'élection, sinon c'est #2199 qui revient.** Le
   filtre d'éligibilité (`select_eligible`, `:531-562`) ne lit que `--draft` + labels + âge +
   marqueur de bail. Un brouillon DecisionCore laissé en draft sans autre état serait **ré-élu au
   tick suivant** (cron `0 */5 * * * *`, `server/mod.rs:104` `WIP_RESCUE_CRON`) : fetch → dry-run → rebase → clippy
   (jusqu'à 900 s) → push → classify → « toujours `no` » → et ainsi de suite toutes les 5 min, en
   occupant l'unique slot (AC6 cap = 1, plus vieux d'abord) et en affamant les brouillons derrière.
   C'est exactement la forme mesurée le 2026-09-05 (14 bails sur #2197). L'exclusion doit donc
   être un **marqueur durable** dans `audit_events`, comme le bail — mais **ré-armable** : le
   bail est terminal (« nothing clears it »), le parcage-en-attente-de-vérification ne l'est pas,
   puisque le marqueur `yes` doit le lever.

3. **Ce n'est pas un bail.** Pas de label `human-review-required`, pas de marqueur
   `wip_rescue_bailed`, pas de « A human owns this PR from here ». Le démon n'a rien trouvé
   d'anormal ; il attend une vérification que la classe exige. Le vocabulaire (événement, audit,
   commentaire) doit dire *parqué en attente de vérification*, pas *abandonné à un humain* — sinon
   l'opérateur qui lit la PR croit à un conflit ou à un clippy rouge.

## 4. Ce que ce plan livre

### 4.1 — Le marqueur est lu, et lu fail-closed (AC1)

Dans `wip_rescue.rs`, un lecteur **pur** :

```rust
/// `<!-- rescue-pipeline-verified: yes -->` → `true`. Everything else — `no`, absent,
/// any other value, an empty body — reads as NOT verified (mika#2286, fail-closed).
fn pipeline_verified(body: &str) -> bool
```

Tolérance : espaces autour de la valeur, casse ASCII de `yes` (un humain qui tape `Yes` a fait
le geste). Rien d'autre. Un marqueur dupliqué avec des valeurs contradictoires lit `false`.

`DraftPr` gagne un champ `body: String` (`#[serde(default)]`) et `list_wip_rescue_drafts`
ajoute `body` à `--json` — un seul appel de plus pour rien, le corps est déjà sur la liste. La
chaîne, elle, **relit le corps frais** (`gh pr view N --json body`) au moment de décider :
rebase + clippy peuvent durer plusieurs minutes et l'humain a pu poser `yes` entre-temps. Échec
de cette lecture → non vérifié (fail-closed) ; le tick suivant relira la liste et se ré-armera
seul si le corps dit `yes`.

### 4.2 — La décision d'un-draft est une fonction pure, et le `if` est là (AC2)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UndraftDecision {
    /// Proceed to `gh pr ready` (Mechanical, or DecisionCore already verified).
    Undraft,
    /// Leave the draft as is; park it until a human verifies (DecisionCore + not `yes`).
    ParkUnverified,
}

fn undraft_decision(route: UndraftRoute, verified: bool) -> UndraftDecision
```

Table de vérité — c'est le contrat du ticket :

| route | marqueur `yes` | décision |
|---|---|---|
| Mechanical | oui | Undraft |
| Mechanical | non | Undraft (inchangé — voie auto de #1852, gardée par périmètre + QA + CI + forge-gate) |
| DecisionCore | oui | Undraft + commentaire hand-merge (inchangé) |
| DecisionCore | non | **ParkUnverified** |

`resume_chain` appelle `undraft_decision` entre l'étape 6 (classify) et l'étape 7 (`gh pr ready`).
La classification `Err(_) => DecisionCore` reste fail-closed et compose avec le nouveau `if` : une
classification illisible sur un marqueur `no` **parque** au lieu d'un-drafter — c'est le sens de
« fail-closed » propagé une étape plus loin.

### 4.3 — Le parcage est durable, nommé, et ré-armable (AC3, AC4)

Nouveau `ChainOutcome::ParkedUnverified` et nouvelle fonction `park_unverified`, miroir de
`bail_to_human` **sans** label et **sans** marqueur de bail :

1. **Marqueur durable d'abord, inconditionnellement** (même ordre porteur que #2199 §4.2) :
   `audit_events.tool_name = "wip_rescue_parked_unverified"`, `target_key = pr:{repo}#{n}`,
   via `log_audit_event`. Nouvelle constante `PARKED_MARKER_TOOL`.
2. **Un commentaire** sur la PR (best-effort, `warn!` sur échec), qui nomme le geste qui
   ré-arme — et pas un autre :

   > Auto-resume (wip-rescue, mika#1852) rebased this branch onto `main` and it passes clippy,
   > but substrate-diff classified it **DECISION-CORE** and its pipeline-verification marker is
   > not `yes` — so it stays a draft (mika#2286). To make it reviewable: verify the pipeline, then
   > either mark it Ready for Review yourself, or set `<!-- rescue-pipeline-verified: yes -->`
   > in the body and the next scan will un-draft it for you. Merge stays a Vincent hand-merge
   > either way.

   Le marqueur durable garantit un seul passage, donc un seul commentaire : pas de spam à chaque
   tick.
3. `info!(…, "wip_rescue_parked_unverified")` + audit `wip_rescue_parked_unverified` (l'action,
   distincte du marqueur d'état — même séparation que `wip_rescue_bail_to_human` vs
   `wip_rescue_bailed`).
4. **Pas** d'incrément de `wip_rescue_depth` : aucune reprise n'a eu lieu ; la reprise qui
   suivra le `yes` l'incrémentera.

`auto_resume_wip_rescue_drafts` traite `ParkedUnverified` comme `Bailed` pour la valeur de retour
(`Some(0)`) et loggue à `info!`.

**Ré-armement** — dans `select_eligible`, un brouillon est exclu s'il est bailé (inchangé) **ou**
si `has_parked_marker(n) && !pipeline_verified(&pr.body)`. Dès que le corps dit `yes`, le
prédicat tombe et le brouillon redevient candidat au tick suivant ; la chaîne rejoue (rebase
no-op, clippy, push no-op, classify DecisionCore, corps frais `yes`) et un-drafte avec le
commentaire hand-merge existant. Si l'humain un-drafte lui-même, la PR quitte la liste `--draft` :
aucun état à nettoyer. Le marqueur de parcage reste en base, inerte — même régime que le bail
(compaction à 90 jours).

`has_parked_marker` est **fail-closed** comme `has_bailed_marker` : audit illisible → parqué. Une
PR DecisionCore exclue à tort ne perd rien (draft, commentée) ; ré-attemptée en boucle, elle
coûte la file.

Signature de `select_eligible` : le second prédicat s'injecte comme le premier (`is_parked: G(u64)`
asynchrone), le marqueur du corps se lit sur `pr` — les tests existants passent avec
`|_| async { false }` pour le nouveau prédicat.

### 4.4 — Le contrat écrit change avec le code (AC5)

- Module doc `wip_rescue.rs` : la ligne du schéma `└─ un-draft (gh pr ready) ── F1` devient un
  embranchement (`DecisionCore ∧ marker≠yes → park`), l'invariant *Perimeter gate is
  authoritative* dit désormais qu'un brouillon DECISION-CORE **n'est un-drafté que vérifié**, et
  une note nomme #2286 comme révision de #1852 étape 6 (la spec un-draftait après « re-run
  pilot » ; la v1 sans re-run n'a plus le droit d'un-drafter à l'aveugle).
- `skills/bundled/qa-review/system_prompt.md` Step 1.5 item 4 : la parenthèse « operator
  un-drafted it » redevient vraie pour DecisionCore grâce à ce fix ; ajouter une phrase qui le dit
  et cite mika#2286, pour qu'un lecteur futur ne conclue pas que la QA fait confiance à n'importe
  quel un-draft. Pas de changement de logique QA (hors périmètre, §5).
- `_compose_rescue_pr_body` (dispatch-lib.sh) : le corps promet déjà « either un-draft this PR
  or set the marker above to `yes` ». Inchangé — le fix tient la promesse au lieu de la réécrire.

### 4.5 — Les tests (AC6)

Tous dans `wip_rescue.rs::tests`, sans `gh` ni réseau (le style §4.5(a) de #2199 : le prédicat
injecté rend la sélection falsifiable sans base) :

1. `pipeline_verified` : `yes` → true ; `Yes ` → true ; `no` → false ; absent → false ; `maybe` →
   false ; corps vide → false ; deux marqueurs contradictoires → false.
2. `undraft_decision` : les quatre lignes de la table §4.2, chacune une assertion nommée.
3. **Le rejeu de #2285** : `select_eligible` avec un brouillon DecisionCore parqué (prédicat
   `is_parked` vrai, corps `no`) et un brouillon plus jeune derrière → le second est élu **au même
   tick**. Contrôle négatif dans le même test, terme par terme : même entrée avec `is_parked`
   toujours faux → le plus vieux est ré-élu trois ticks de suite (c'est `main`).
4. **Le ré-armement** : même brouillon parqué, corps `yes` → il est élu. Avec corps `no` → il ne
   l'est pas. Les deux dans un seul test, pour que l'un ne puisse pas passer sans l'autre.
5. `has_parked_marker` sur une base sans table `audit_events` → `true` (fail-closed, copie du
   test `an_unreadable_audit_trail_excludes_the_draft`).
6. Les tests existants (`a_bailed_draft_is_not_re_elected…`, `the_pre_existing_predicates…`,
   etc.) passent inchangés hors le prédicat supplémentaire trivial.

Pas de rejeu fake-`gh` de la chaîne complète : la décision est pure, la sélection est pure, et
l'appel `gh pr ready` est derrière le `match` — la seule façon d'atteindre `Undraft` sur
DecisionCore est `verified == true`, ce que le test 2 fixe.

## 5. Périmètre

**Dedans** : `crates/mika-agent/src/wip_rescue.rs` (lecteur, décision, parcage, sélection,
tests, module doc) ; une phrase de contexte dans `qa-review/system_prompt.md` Step 1.5 ; un
`docs/solutions/dev-loop/` en compound.

**Dehors, nommé pour ne pas y glisser :**

- **La voie Mechanical reste un-draftée avec marqueur `no`.** C'est la conception de #1852
  (RT#004, Vincent + Prime) : la classe mécanique passe par périmètre + QA + CI + forge-gate. Le
  ticket ne vise que DecisionCore, et le ratifie comme tel. Si Vincent veut fermer aussi la voie
  Mechanical, c'est un autre ticket avec sa propre mesure.
- **La QA ne distingue pas un un-draft du démon d'un un-draft humain** (Step 1.5 item 4). Après ce
  fix, le démon n'un-drafte plus de DecisionCore non vérifiée, donc la prémisse redevient vraie
  pour la classe sensible. Rendre la QA robuste à la source de l'un-draft (lire l'acteur de
  `ready_for_review` dans la timeline) serait un durcissement supplémentaire, non demandé.
- **Retrait de `MIKA_DISPATCH_BYPASS_GROOMING_CHECK=1`** et **#2029** : points (2) et (3) de
  l'ordre du jour ratifié, tickets distincts.
- **Le worktree orphelin `mika/.wip-rescue-wt/pr-2243/`** : résidu d'une passe interrompue du
  démon, sans rapport ; on n'y touche pas dans cette PR.

## Acceptance criteria

- **AC1 — Le marqueur est lu, fail-closed.** `pipeline_verified(body)` rend `true` pour le
  littéral `<!-- rescue-pipeline-verified: yes -->` (casse ASCII et espaces tolérés) et `false`
  pour `no`, absent, toute autre valeur, corps vide, marqueurs contradictoires. Test unitaire
  couvrant chaque cas.
- **AC2 — Une PR DecisionCore n'est un-draftée par le démon que si le marqueur est `yes`.**
  `undraft_decision(DecisionCore, false) == ParkUnverified` ; `undraft_decision(DecisionCore, true)
  == Undraft` ; `undraft_decision(Mechanical, _) == Undraft`. `resume_chain` n'atteint `gh pr
  ready` que sur `Undraft`. Test unitaire des quatre lignes.
- **AC3 — Le parcage est durable et n'est pas un bail.** `park_unverified` écrit le marqueur
  `wip_rescue_parked_unverified` **avant** tout appel GitHub, poste un commentaire nommant les deux
  gestes qui débloquent (un-draft humain, ou marqueur `yes`), n'applique **pas**
  `human-review-required`, n'écrit **pas** `wip_rescue_bailed`, n'incrémente **pas**
  `wip_rescue_depth`. Événement `wip_rescue_parked_unverified` à `info!`.
- **AC4 — Une PR parquée n'est pas ré-élue, et le `yes` la ré-arme.** `select_eligible` exclut un
  brouillon dont le marqueur de parcage existe **et** dont le corps ne lit pas `yes` ; le brouillon
  suivant est élu au même tick. Le même brouillon avec corps `yes` est élu. Contrôle négatif
  (prédicat toujours faux → ré-élection) dans le même test. `has_parked_marker` fail-closed sur
  audit illisible.
- **AC5 — Le contrat écrit suit.** Module doc de `wip_rescue.rs` (schéma, invariant *Perimeter
  gate*, note #2286 ⊃ révision #1852 étape 6) et une phrase de contexte dans `qa-review`
  Step 1.5 item 4 citant mika#2286. Le corps de secours de dispatch-lib est inchangé.
- **AC6 — Vert.** `cargo test -p mika-agent wip_rescue` et `cargo clippy --tests -- -D warnings`
  passent ; les tests existants du module passent sans modification de leurs assertions.

## Fire-Disposition

- **Sur `Undraft` (Mechanical, ou DecisionCore vérifiée)** : comportement d'aujourd'hui, à
  l'identique, commentaire hand-merge compris.
- **Sur `ParkUnverified`** : marqueur durable → commentaire → audit → `ChainOutcome::ParkedUnverified`
  → `Some(0)`. Le brouillon reste draft avec `wip-rescue` seul. Aucun retry automatique tant que le
  corps ne lit pas `yes`.
- **Sur échec de lecture du corps frais** : non vérifié → `ParkUnverified`. Auto-réparation au
  tick suivant si la liste lit `yes`.
- **Sur échec d'écriture du marqueur de parcage** : `warn!("wip_rescue_marker_write_failed")`
  (même nom que pour le bail — un hit = une PR dont l'exclusion n'est pas tenue), commentaire tout
  de même ; la PR sera ré-élue au tick suivant et re-parquée — dégradé mais borné par le coût d'un
  tick, jamais un un-draft.

## 7. Vérification

1. `cargo test -p mika-agent wip_rescue` — les six familles de tests §4.5 vertes.
2. `cargo clippy --tests -- -D warnings`.
3. Rejeu manuel possible après déploiement, sans attendre un incident : sur une PR `wip-rescue`
   draft DecisionCore réelle, observer `wip_rescue_parked_unverified` dans `server.log` au tick
   suivant l'âge seuil, un seul commentaire, pas de `ready_for_review` dans la timeline ; poser
   `yes` dans le corps ; observer `wip_rescue_success classification=DecisionCore` au tick
   suivant et `ready_for_review` par mika-platform-dev. (Rappel `never_conclude_inside_the_mechanism_period` :
   attendre au moins deux ticks de 5 min avant de conclure « parqué et pas ré-élu ».)

## 8. Risques

- **Un brouillon DecisionCore parqué que personne ne regarde.** Il reste en draft avec
  `wip-rescue` et un commentaire ; c'est l'état que le corps de secours décrit déjà comme normal.
  La QA le tient en `hold[review]` (marqueur `no` + draft) — visible, pas perdu. Le coût
  d'attention est exactement celui que Vincent + Prime ont choisi pour cette classe.
- **Le corps est lu deux fois (liste + frais).** Si la liste dit `yes` et le frais dit `no`
  (édition en sens inverse pendant la chaîne), on parque : fail-closed, un tick de retard au pire.
- **`has_parked_marker` ajoute une lecture SQLite par candidat non bailé.** Même bornage que le
  bail : court-circuit sur le premier éligible, liste plafonnée à 100.
