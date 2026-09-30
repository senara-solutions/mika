---
title: "fix: `Accepted` n'est pas `Updated` — mesurer l'atterrissage de l'update-branch avant de le déclarer"
issue: 2252
type: fix
depth: Standard
origin: 2238
created: 2026-09-30
supersedes: "fb47130b:docs/plans/2026-09-09-001-fix-2252-update-branch-accepted-not-landed-plan.md"
---

# fix: `Accepted` n'est pas `Updated`

## Résumé

`attempt_update_branch` émet un `PUT repos/{repo}/pulls/{n}/update-branch` et traite un exit zéro
comme « la branche est à jour ». L'endpoint répond **`202 Accepted`** et effectue le merge de façon
**asynchrone** : le zéro prouve que GitHub a pris la requête, jamais qu'un commit existe. Le
doc-comment de la fonction le sait et le dit ; les deux doc-comments d'enum qu'elle alimente
affirment l'inverse, et le résultat rendu au moniteur (`BranchUpdated { new_main_sha }`) **publie une
donnée jamais mesurée**.

Quand le job asynchrone échoue, la boucle a consommé son unique tentative, déclaré la réparation
faite, et engagé un rendez-vous sur un webhook CI qui n'arrivera pas. La PR reste BEHIND, présentée
comme réparée, pendant **jusqu'à six heures** (`UPDATE_ATTEMPT_TTL`) ou jusqu'à ce que `main` bouge.

Ce plan remplace la déclaration par une **mesure** : après le `202`, re-lire le `baseRefOid` sous un
budget borné, et n'émettre `Updated` que si la base a réellement avancé. Sinon, émettre un état nommé
distinct et **relâcher le claim**, pour que la prochaine arrivée puisse re-tenter au lieu d'attendre
six heures.

---

## Re-mesure contre `main` (2026-09-30, HEAD `7455023b`)

L'ancien plan de ce ticket a été groomé le 2026-09-09 contre `main @ 037fe33d` et retiré de la
branche le 2026-09-30. Le commentaire opérateur prescrit de **re-mesurer les quatre prémisses et les
ancres avant l'architecte**. C'est fait, et c'est le premier livrable de ce plan.

### Les ancres ont dérivé d'environ +125 lignes ; leur contenu est intact

`crates/mika-agent/src/tools/pr_merge_with_gate.rs` (3594 lignes)

| Ancre | @037fe33d | @7455023b | Vérifié |
|-------|-----------|-----------|---------|
| `UPDATE_ATTEMPT_TTL` | 838 | **963** | `6 * 3600` s, inchangé |
| `UpdateBranchOutcome::Updated` | 847 | **973** | doc : « a new commit now sits on the PR head » — **mot pour mot** |
| `BehindMainRemediation::Updated` | 860 | **986** | doc : « The branch was brought up to date » — **mot pour mot** |
| `release_update_branch_attempt` | 931 | **1056** | doc : « Released only for `Failed` » |
| site de relâchement | 1093 | **1227** | `matches!(remediation, BehindMainRemediation::Failed(_))` |
| doc « `Updated` means accepted, not finished » | 986-991 | **1112-1117** | présent, et **renforcé** (voir ci-dessous) |
| `attempt_update_branch` | 993 | **1118-1130** | `Ok(_) => UpdateBranchOutcome::Updated` |
| `classify_update_branch_error` | 1011 | **1136** | `AlreadyUpToDate` testé en premier |
| `reconcile_already_up_to_date` | 1117 | **1242** | re-lit `baseRefOid`, rend `Contradiction` |
| `remediate_behind_main` | 1059 | **1184** | trois sites appelants (ci-dessous) |
| `log_behind_main_remediation` | 1153 | **1278** | `outcome = "updated"` |
| `disposition_for_remediation` | 1211 | **1336-1339** | `BranchUpdated { new_main_sha: info.current_main_sha }` |
| précédent `AlreadyAttempted` | 1216-1226 | **1340-1351** | `Blocked { reason: BehindMain, detail }` |
| `describe_behind_main_remediation` | 1271 | **1396** | bras `Updated` : « GitHub **has accepted** » |

Prompts embarqués : `self-dev/system_prompt.md:276 → **284**`,
`self-dev-webhook-ci/system_prompt.md:37 → **37**`, `self-dev-webhook-qa/system_prompt.md:278 → **287**`.
Les trois portent **mot pour mot** « returns **six** typed variants … Branch on these variants
**exhaustively** », et énumèrent **huit** `blocked.reason` dont `behind_main`.

`MergeGateResult` compte **six** variantes (`Merged`, `AutoMergeEnabled`, `Blocked`, `AlreadyMerged`,
`GateError`, `BranchUpdated`). `BlockReason` en compte **huit**, `behind_main` incluse.

### Les quatre prémisses de l'ESCALATE-divergence du 2026-09-09 tiennent toutes

1. **Mécanisme.** `gh api --method PUT repos/{repo}/pulls/{n}/update-branch` (l. 1123-1124).
   `gh pr update-branch` n'est invoqué **nulle part** dans le crate ; le doc-comment 1103-1110
   explique pourquoi y passer serait un no-op (#1577 KTD-1). ✅
2. **Branche `AlreadyUpToDate` fermée.** `classify_update_branch_error` la distingue (1142-1148) et
   `reconcile_already_up_to_date` re-lit le `baseRefOid` pour rendre `Contradiction` (1255-1266). ✅
3. **Chronologie.** Fait de ticket, non de code ; le corps ne prétend plus au no-op-success mesuré. ✅
4. **Forme du fix.** L'endpoint est asynchrone **par contrat écrit** (1112-1117), donc une comparaison
   SHA synchrone rendrait un faux négatif systématique. La re-lecture sous budget borné reste la
   forme juste. ✅

### D1 est plus net qu'en septembre — le fichier se contredit lui-même sur un point de plus

Le doc-comment de `attempt_update_branch` affirme désormais (l. 1114-1115) :

> **Every string this module renders for `Updated` says "accepted" for that reason.**

C'est vrai d'**une** surface et faux de **trois** :

| surface | ce qu'elle rend pour `Updated` | conforme ? |
|---|---|---|
| `describe_behind_main_remediation` (1406) | « GitHub **has accepted** an automatic branch update » | ✅ |
| `UpdateBranchOutcome::Updated` doc (972) | « a new commit **now sits** on the PR head » | ❌ |
| `BehindMainRemediation::Updated` doc (985) | « The branch **was brought up to date** » | ❌ |
| `log_behind_main_remediation` (1286) | `outcome = "updated"` | ❌ (jeton de fil) |
| `disposition_for_remediation` (1338) | `new_main_sha: info.current_main_sha` | ❌ (**donnée**, pas prose) |

La prose a donc été à moitié corrigée par mika#2250 ; **la donnée et le jeton de trace ne l'ont pas
été**. C'est la forme la plus dure de D1 : le fichier porte une affirmation sur lui-même que sa propre
donnée dément.

### Deux constats NEUFS qui changent la forme du fix

**F1 — `remediate_behind_main` n'a aucune couverture de test, et c'est la discipline du module.**
La fonction n'apparaît qu'à sa définition (1184) et à ses trois sites appelants. Tous les tests
existants portent sur les pièces **pures** autour d'elle : `classify_update_branch_error` (2121-2154),
`disposition_for_remediation` (2188-2506), `claim_update_attempt_in` (2323-2420),
`release_update_branch_attempt` (2449). Le module range systématiquement la décision dans du pur et
laisse l'orchestrateur réseau nu.

**Conséquence : les étapes 5-6 et les tests 11-12 de l'ancien plan ne sont pas implémentables tels
qu'écrits.** Ils plaçaient la boucle de vérification dans `remediate_behind_main` et asseyaient AC3/AC4
sur les sorties de cette fonction — ce qui exige soit un mock réseau que l'ancien plan ne nomme pas,
soit un refactor qu'il ne nomme pas non plus. Ce plan suit la discipline du module : la décision est
une **fonction pure**, l'observation est **injectée** (idiome `deadline_verdict::poster`,
`server/deadline_verdict.rs:602-612`, dont le doc-comment dit en propres termes que l'injection existe
pour que le contrat soit assertable sans toucher la forge), et `remediate_behind_main` reste du
câblage mince.

**F2 — le scan de source que le fichier revendique n'existe pas.** Le doc-comment 1098-1101 affirme :
« **This is the only call site of update-branch in the codebase (R1).** A source-scan test pins that ;
a behavioural test cannot ». Recherche exhaustive : `grep -rn 'update.branch' --include='*.rs' crates/`
rend le seul site (1123) plus trois commentaires dans les handlers ; **aucun test ne le pinne**. La
propriété tient ; le test qui prétend la tenir est absent. L'étape 14 de l'ancien plan
(« vérifier que le scan existant tient après le renommage ») décrit donc un test inexistant et doit
être remplacée. Fermer ce trou est **hors périmètre** (voir § *Hors périmètre*) : c'est R1 de
mika#2250, pas une exigence d'ici.

### Le risque « latence dans les handlers webhook » est levé — trois sites, une seule enveloppe

L'ancien grooming a laissé ce point en risque non vérifié. Mesuré :

| site | enveloppe qui contient la remédiation |
|---|---|
| `pr_merge_with_gate.rs:308` (l'outil) | `PrMergeWithGate::timeout_secs() = Some(60)` — budget par outil de la boucle d'agent |
| `ci_success_handler.rs:477` | **aucune** — les `tokio::time::timeout` de ce fichier (306, 620, 671) enveloppent les *checks* et deux autres futures, pas la remédiation |
| `verdict_handler.rs:606` | **aucune** — idem (542 = checks, 658 = merge) |

Le pire cas est donc le **site de l'outil** : 60 s partagés entre le préflight `gh pr view`,
`is_behind_main`, `run_gh_checks`, la remédiation et le merge. Un budget de 6 s y pèse 10 %, et il
n'est **payé que quand l'update n'atterrit pas** — c'est-à-dire dans le cas qui coûte aujourd'hui six
heures. Les deux sites handler l'absorbent sans borne encadrante.

---

## Les trois défauts

**D1 — Le résultat affirme un fait qu'il n'a pas mesuré.** `BranchUpdated { new_main_sha }` est un
champ **de données**, sérialisé vers le moniteur et vers le LLM, qui déclare que la base de la PR vaut
désormais `current_main_sha`. Rien ne l'a lu. Voir le tableau à cinq surfaces ci-dessus : la
contradiction est *dans le fichier*, pas entre le fichier et une hypothèse.

**D2 — La récupération est bornée par six heures, pas par l'échec.** Le claim anti-thrash est consommé
avant l'appel (délibérément, 1039-1047) et n'est relâché que pour `Failed` (1227). Un `202` suivi d'un
job qui échoue laisse le claim posé. Le seul déblocage est un mouvement de `main` (nouvelle clé) ou
l'expiration du TTL de six heures. Le code nomme lui-même la conséquence (1044-1047) — *« nothing
evicts the entry until the ledger hits its soft cap, and with a serialized dispatch `main` may not
advance for hours. A permanent stall is exactly the state mika#2238 exists to end »* — puis, à la
ligne 1117, énonce la borne comme si elle suffisait : *« resolves the next time `main` moves »*. Le
raisonnement était juste ; sa conclusion n'a été appliquée qu'à `Failed`.

**D3 — Le rendez-vous différé n'a pas de gardien.** R3 de mika#2238 a supprimé le re-merge dans le
même tour au profit d'un rendez-vous sur le webhook CI. Ce rendez-vous suppose qu'un commit
apparaisse. Si le job asynchrone échoue, il n'y a ni commit, ni CI, ni webhook — donc aucun tour
suivant ne vient re-mesurer. L'absence de signal est indistinguable du succès.

---

## Requirements

- **R1** — `UpdateBranchOutcome::Updated` est renommé `Accepted`. Son nom et son doc-comment disent ce
  que le `202` prouve — la requête est prise — et rien de plus. Aucune variante de cet enum n'affirme
  plus l'existence d'un commit.

- **R2** — L'observation de l'atterrissage est une fonction **dont l'effet est injecté** :
  `observe_update_branch_landing(before_base_sha, budget, read_base)`, où `read_base` est une closure
  rendant `Result<String, String>`. Elle rend `LandingObservation::Landed { observed_base_sha }` ou
  `NotObserved`. C'est cette injection qui rend AC3/AC4 assertables sans réseau (F1 ci-dessus), et
  c'est l'idiome déjà en vigueur dans `server/deadline_verdict.rs:602-612`.

- **R3** — L'attente précède la première lecture, jamais l'inverse. L'endpoint est asynchrone par
  contrat : lire immédiatement produirait le faux négatif systématique que la note du ticket met en
  garde contre. La boucle s'arrête au premier succès — aucune latence ajoutée quand l'atterrissage est
  rapide, et le budget n'est payé en entier que quand rien n'atterrit.

- **R4** — **Une base illisible n'est jamais un atterrissage.** Une `Err` de re-lecture, un SHA
  identique, **ou un SHA vide** comptent comme « pas encore observé » et la boucle continue ; budget
  épuisé ⇒ `NotObserved`. Le terme du SHA vide est porteur et non défensif :
  `PrPreflight::base_ref_oid` retombe sur la chaîne vide quand le champ n'est pas rendu — la
  population existe et un test la pinne déjà (`preflight_base_ref_oid_defaults_to_empty`, l. 3583).
  Sans ce terme, `"" != before_base_sha` serait vrai et un champ absent se lirait comme un
  atterrissage, c'est-à-dire exactement le défaut qu'on ferme, une couche plus bas.

- **R5** — Deux issues distinctes remplacent l'unique `BehindMainRemediation::Updated` :
  - `Updated { observed_base_sha }` — la re-lecture a constaté que la base a avancé. Le SHA porté est
    **celui qui a été lu**, jamais `info.current_main_sha`.
  - `AcceptedNotLanded` — le budget a expiré sans que la base bouge.

- **R6** — `AcceptedNotLanded` **relâche le claim** (`release_update_branch_attempt`). La prochaine
  arrivée re-mesure et peut re-tenter, au lieu d'attendre six heures ou un mouvement de `main`.

- **R7** — `MergeGateResult::BranchUpdated.new_main_sha` porte le SHA **observé** (R5), pas le SHA
  visé.

- **R8** — `AcceptedNotLanded` rend `MergeGateResult::Blocked { reason: BehindMain, detail }` avec un
  `detail` nommé — **pas une nouvelle variante de `MergeGateResult`.** Trois raisons, dans l'ordre de
  force :

  1. **C'est le fait.** `AcceptedNotLanded` signifie littéralement « la base n'a pas bougé », donc la
     PR *est* encore behind. `BehindMain` est la description exacte, pas un repli.
  2. **Le précédent est dans le fichier.** `BehindMainRemediation::AlreadyAttempted` rend déjà
     exactement cette forme (1340-1351) : `Blocked { reason: BehindMain, detail: "…an automatic branch
     update toward this main HEAD was already attempted and is not being retried." }`.
     `AcceptedNotLanded` en est le frère — même fait, autre cause.
  3. **Une septième variante rouvrirait T2 de mika#2238.** Les trois prompts embarqués énumèrent
     **six** variantes et instruisent « branch on these variants **exhaustively** » (re-vérifié
     ci-dessus, aux trois nouvelles lignes). Introduire une septième sans les mettre à jour reproduit
     précisément le trou que #2238 existe pour fermer : un agent qui reçoit une variante hors de sa
     liste exhaustive n'a pas de disposition définie, et le comportement observé est l'arrêt
     silencieux. `reason: BehindMain` est déjà l'une des **huit** `blocked.reason` énumérées, avec sa
     disposition déjà écrite — coût prompt nul.

- **R9** — Les jetons `outcome` de `log_behind_main_remediation` sont un **format de fil** : ils
  atterrissent dans le journal et le doc-comment de cette fonction (1271-1273) en fait une prédicat
  opérateur — *« a PR that shows up behind without a following `outcome="updated"` is the starvation
  signal »*. L'ajout est donc **additif** : `AcceptedNotLanded` gagne `"accepted_not_landed"`, et
  `Updated` **garde `"updated"`**. Renommer le second casserait une lecture opérateur publiée pour
  aucun gain — et après ce fix ce jeton devient *plus* vrai qu'avant, puisqu'il ne désigne plus qu'un
  atterrissage **observé**.

- **R10** — Le budget est **3 re-lectures espacées de 2 s (6 s au pire)**, porté par une constante
  nommée avec sa justification écrite, et paramétrable dans les tests (même discipline que
  `claim_update_attempt_in`, qui abstrait `now` et la map). La valeur est un choix assumé, pas une
  mesure : la latence réelle d'atterrissage d'un `202 update-branch` sur ce dépôt n'a jamais été
  observée. Ce qui la rend sûre est R11 ; ce qui la rend **acceptable en coût** est la mesure
  d'enveloppe ci-dessus (10 % du seul site borné, payés seulement quand rien n'atterrit) ; et la trace
  (R9) est l'instrument qui permettra de la corriger sur données plutôt que sur intuition.

- **R11** — Un faux négatif de vérification (l'atterrissage arrive après l'expiration du budget) est
  **bénin par construction** : il relâche le claim, et le tour suivant mesure `is_behind_main` qui
  rendra « plus behind ». Aucun double merge commit n'en résulte. Cette propriété est écrite dans le
  doc-comment, parce que c'est elle qui autorise un budget court — sans elle, un lecteur futur
  allongera le budget pour de mauvaises raisons.

- **R12** — Les deux doc-comments d'enum qui mentent (972, 985) sont corrigés, et l'auto-affirmation
  du doc-comment de `attempt_update_branch` (1114-1115) devient vraie sur les cinq surfaces du tableau
  D1 : après ce fix, le jeton de trace et le champ de donnée dérivent tous deux d'une observation.

---

## Hors périmètre

- La détection `is_behind_main` (#1577) — inchangée, elle reste l'autorité sur « behind ».
- Les trois sites d'appel de `remediate_behind_main` — ils reçoivent une variante de plus ; leur
  ordonnancement, leurs enveloppes et leurs textes ne bougent pas. Aucune ligne de
  `ci_success_handler.rs` ni de `verdict_handler.rs` n'est touchée, ce qui est ce qui garde le diff
  borné au module.
- Les prompts embarqués (R6 de #2238) — **et R8 garantit qu'ils n'ont pas besoin de l'être** : le
  choix de `Blocked { reason: BehindMain }` plutôt qu'une septième variante maintient les prompts
  exacts sans les toucher. Ce n'est pas un report, c'est une contrainte de conception respectée. Si
  une révision future introduit malgré tout une variante, la mise à jour des trois prompts entre dans
  son périmètre — pas dans un ticket séparé.
- **Le scan de source absent (F2).** Le doc-comment 1098-1101 revendique un test qui n'existe pas.
  Réel, adjacent, et ce n'est pas une exigence de ce ticket : la propriété qu'il garderait est R1 de
  mika#2250 (l'unicité du site d'appel, dont l'enjeu est la contournabilité de la garde anti-thrash),
  pas la véracité de `BranchUpdated`. Le renommage de R1 est de toute façon sûr par le compilateur,
  qui énumère les sites de la variante. **Ticket de suivi**, dont le périmètre est : livrer le scan que
  le doc-comment annonce, ou retirer la phrase. Ne pas l'absorber ici serait moins honnête que de
  le nommer — et l'absorber élargirait un p1 à la garde d'un voisin.
- Le gate de SHA périmé qui retient la PR après un update (documenté 540-545) — explicitement suivi
  ailleurs.
- Toute tentative d'expliquer rétrospectivement l'état de PR#2251 : la chronologie établie par
  mika-dev l'exclut du périmètre.

---

## Étapes

### Phase 1 — Nommer ce que le `202` prouve

1. Renommer `UpdateBranchOutcome::Updated` → `Accepted` (971-980). Réécrire son doc-comment : la
   requête est acceptée ; l'existence d'un commit n'est pas établie ici.
2. Mettre à jour `attempt_update_branch` (1127) et `classify_update_branch_error` (1136-1155) pour la
   nouvelle variante. Aucun changement de comportement réseau.

### Phase 2 — Mesurer l'atterrissage

3. Ajouter, à côté de `UPDATE_ATTEMPT_TTL` (963), le type `LandingBudget { attempts, delay }` et la
   constante `UPDATE_LANDING_BUDGET = { attempts: 3, delay: 2s }` portant la justification de R10 et
   la borne de R11.
4. Ajouter l'enum `LandingObservation { Landed { observed_base_sha }, NotObserved }`.
5. Ajouter `observe_update_branch_landing(before_base_sha, budget, read_base)` (R2) : pour chaque
   tentative, **attendre `delay` puis lire** (R3) ; un `Ok(sha)` non vide et différent de
   `before_base_sha` rend `Landed` ; tout le reste — `Err`, SHA identique, SHA vide — continue (R4).
   Budget épuisé ⇒ `NotObserved`. Le doc-comment porte R11.

### Phase 3 — Propager le fait mesuré

6. `BehindMainRemediation` : `Updated` devient `Updated { observed_base_sha: String }` ; ajouter
   `AcceptedNotLanded` avec un doc-comment qui nomme les trois défauts fermés (982-1014).
7. Dans `remediate_behind_main` (1214-1223), remplacer `Accepted => BehindMainRemediation::Updated`
   par l'appel à l'observateur, en lui injectant la closure
   `|| run_gh_pr_view(pr_number, repo, token).map(|p| p.base_ref_oid)`.
8. Étendre le `matches!` de relâchement (1227) à `AcceptedNotLanded` (R6), et mettre son commentaire
   en accord avec les deux cas. **C'est la seule étape sans filet du compilateur** — `matches!`
   continue de compiler quand une variante apparaît (voir § *Risques*) ; le test 16 est ce qui la
   couvre, et il doit asserter le claim relâché, pas seulement la disposition.
9. `disposition_for_remediation` (1336) : `Updated { observed_base_sha }` → `BranchUpdated` portant le
   SHA **observé** (R7) ; `AcceptedNotLanded` → `Blocked { reason: BehindMain, detail }` (R8), sur le
   modèle littéral du bras `AlreadyAttempted` voisin (1340-1351), avec un `detail` qui dit « acceptée,
   atterrissage non constaté sous le budget, une nouvelle tentative est ouverte ».
10. `log_behind_main_remediation` (1285-1293) : `AcceptedNotLanded` → `"accepted_not_landed"` ;
    `Updated` garde `"updated"` (R9). Le `match` est exhaustif sans bras `_`, donc la variante neuve
    est une erreur de compilation ici — c'est la garde, pas un test.
11. `describe_behind_main_remediation` (1403) : arm `AcceptedNotLanded` disant accepté / atterrissage
    non constaté / nouvelle tentative ouverte / ne pas merger / ne pas rebaser à la main, **sans**
    employer le vocabulaire de la garde completion-claim (`merged` / `deployed` / `complete(d)` /
    `shipped`), interdiction déjà écrite au doc-comment de cette fonction (1392-1395).
12. Corriger les deux doc-comments qui mentent (972, 985) et l'auto-affirmation 1114-1115 (R12).

### Phase 4 — Épingler

13. Test : un `Accepted` dont la re-lecture ne bouge jamais rend `NotObserved` (budget à `delay:
    ZERO`, donc sans coût d'horloge).
14. Test : un `Accepted` dont la re-lecture bouge à la deuxième tentative rend
    `Landed { observed_base_sha }` portant le SHA lu, et **s'arrête là** (le compteur d'appels de la
    closure vaut 2, pas 3 — c'est ce qui atteste R3).
15. Test : une re-lecture qui rend `Err` à chaque fois, puis une qui rend `""`, rendent toutes deux
    `NotObserved` — les deux contrôles négatifs de R4, **séparés**, parce qu'une conjonction de termes
    fail-safe ne se prouve pas en les neutralisant ensemble.
16. Test : `AcceptedNotLanded` → `Blocked { reason: BehindMain }` **et** claim relâché (le second
    appel sur le même SHA cible doit repasser).
17. Test : `Updated { observed_base_sha }` → `BranchUpdated` dont `new_main_sha` **est le SHA observé**,
    sur un cas où il diffère de `info.current_main_sha` (une PR qui a atterri pendant que `main`
    re-bougeait) — c'est le test qui distingue « mesuré » de « déclaré ».
18. Test : `Updated` **conserve** le claim (le plafond anti-thrash de #2238 R4 reste en vigueur pour
    le cas nominal).
19. Garde de cardinalité (AC8) : un `match` exhaustif **sans bras `_`** mappant chaque variante de
    `MergeGateResult` vers son jeton `action`, plus l'assertion que l'ensemble en compte **six**. La
    protection réelle est l'erreur de compilation sur une septième variante ; l'assertion de compte
    est le terme d'anti-vacuité.
20. Garde de format de fil (R9) : test épinglant les **huit** jetons `outcome` de
    `log_behind_main_remediation`, dont `"updated"` inchangé et `"accepted_not_landed"` neuf.

---

## Acceptance criteria

- **AC1** — Aucune variante d'enum de ce module n'affirme, dans son nom ou son doc-comment, qu'un
  commit existe sur la base d'un code retour d'API. Vérifiable : le doc de la variante issue du `202`
  dit « accepté » ; celui de la variante issue de la re-lecture dit « observé ».
- **AC2** — `MergeGateResult::BranchUpdated.new_main_sha` est le SHA lu sur la PR après
  l'update-branch, jamais `info.current_main_sha`. Épinglé par le test 17.
- **AC3** — Un `202` dont l'effet n'atterrit pas dans le budget produit
  `Blocked { reason: BehindMain }` — jamais `BranchUpdated` — et relâche le claim. Épinglé par le
  test 16.
- **AC4** — Un `202` dont l'effet atterrit produit `BranchUpdated` et **conserve** le claim. Épinglé
  par le test 18.
- **AC5** — Une re-lecture illisible, identique ou **vide** n'est jamais lue comme un atterrissage.
  Épinglé par les deux contrôles négatifs du test 15.
- **AC6** — La trace émet des `outcome` distincts pour « atterri » et « accepté non atterri », de
  sorte qu'une requête sur les traces puisse compter les seconds ; et le jeton `"updated"` est
  **inchangé**, donc le prédicat opérateur publié au doc-comment 1271-1273 continue de se lire.
  Épinglé par le test 20.
- **AC7** — Le doc-comment de l'observateur écrit explicitement pourquoi un faux négatif est bénin
  (R11) — c'est la justification du budget court, et sans elle un lecteur futur allongera le budget
  pour de mauvaises raisons.
- **AC8** — `MergeGateResult` compte toujours **six** variantes après ce changement, et les trois
  prompts embarqués restent inchangés et exacts (leur phrase « six typed variants » reste vraie).
  C'est la garde qui empêche ce fix de rouvrir T2 de mika#2238. Épinglé par le test 19.
- **AC9** — `cargo test -p mika-agent` et `cargo clippy --all-targets -- -D warnings` passent. Aucun
  test existant n'est modifié sauf pour suivre le renommage R1 et la nouvelle forme de
  `BehindMainRemediation::Updated` ; aucune assertion existante n'est affaiblie.

---

## Definition of Done

Livrables concrets, tous dans un seul fichier de production :

- `UpdateBranchOutcome::Accepted` remplace `::Updated` (nom + doc-comment).
- `LandingBudget`, `UPDATE_LANDING_BUDGET`, `LandingObservation` et
  `observe_update_branch_landing` existent, avec l'observation **injectée**.
- `BehindMainRemediation::Updated { observed_base_sha }` et `::AcceptedNotLanded` existent.
- `remediate_behind_main` appelle l'observateur et relâche le claim sur les **deux** cas `Failed` et
  `AcceptedNotLanded`.
- `disposition_for_remediation` porte le SHA **observé** et route `AcceptedNotLanded` vers
  `Blocked { reason: BehindMain }`.
- `log_behind_main_remediation` et `describe_behind_main_remediation` ont leur bras neuf ;
  `"updated"` est inchangé.
- Les trois doc-comments qui mentent (972, 985, 1114-1115) sont corrigés.
- Huit tests neufs (13-20), dont **trois contrôles négatifs séparés** (R4 : `Err`, SHA identique, SHA
  vide) et la garde de cardinalité.
- `cargo test -p mika-agent` et `cargo clippy --all-targets -- -D warnings` verts.
- Aucun fichier de prompt, aucun handler, aucune migration, aucune variable d'environnement.

### Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/tools/pr_merge_with_gate.rs` | **le seul** fichier de production modifié — enum, observateur, câblage, dispositions, trace, prose, tests |
| `docs/plans/2026-09-30-002-…-plan.md` | ce plan |

Explicitement **non** touchés, et c'est ce qui borne le diff : `server/ci_success_handler.rs`,
`server/verdict_handler.rs`, les trois `skills/bundled/self-dev*/system_prompt.md`.

---

## Fire-Disposition

Ce plan livre des détecteurs (tests comportementaux 13-18, garde de cardinalité 19, garde de format de
fil 20), donc la section est requise par mika#2306. **Disposition retenue : (a) exception nommée en
allowlist — avec une allowlist livrée VIDE, parce qu'il n'existe aucune violation à exempter.**

| détecteur | population en violation à la livraison | état à la livraison |
|---|---|---|
| tests 13-18 (comportement de l'observateur et des dispositions) | aucune — le comportement est *créé* par ce plan | **vert, armé** |
| test 19 (cardinalité `MergeGateResult` == 6) | aucune — mesuré ce jour : exactement six variantes | **vert, armé** |
| test 20 (jetons `outcome`) | aucune — les sept jetons existants sont inchangés, le huitième est neuf | **vert, armé** |

**Pourquoi (a) et non (b) « livrer désarmé ».** Aucun détecteur ne rougit sur l'arbre tel qu'il est,
donc il n'y a rien à désarmer : un `#[ignore]` ici ne protégerait de rien et créerait un test que
personne ne réarme. La forme (b) existe pour un détecteur qui *mordrait* sur une dette pré-existante,
ce qui n'est pas le cas.

**Pourquoi (a) et non (c) « halte-et-remontée ».** (c) est pour la découverte d'une violation qui exige
un cadrage opérateur. Ici la mesure est faite et rend zéro.

**L'allowlist est vide par contrat, et voici la conduite quand chacun tire.**

- **Test 19 rougit** ⇒ quelqu'un a ajouté une septième variante à `MergeGateResult`. **La résolution
  est de mettre à jour les trois prompts embarqués dans le même commit**, jamais d'assouplir
  l'assertion : c'est très exactement T2 de mika#2238, et l'assertion est la seule chose qui rende la
  divergence visible le jour où elle est écrite plutôt que le jour où un agent s'arrête en silence.
  L'erreur de compilation du `match` sans bras `_` arrive en premier et nomme le site.
- **Test 20 rougit** ⇒ un jeton `outcome` a été renommé. La résolution est de **restaurer le jeton** ou
  de dater la rupture dans `CLAUDE.md` et de corriger les prédicats opérateur publiés — jamais de
  mettre le test à jour en silence, ce qui casserait une lecture opérateur sans trace.
- **Tests 13-18 rougissent** ⇒ la mesure a été remplacée par une déclaration. C'est le défaut que ce
  ticket ferme, revenu. Ne pas allowlister : lire lequel des termes de R4 a été relâché.

**Aucune entrée d'allowlist n'est créée par ce plan** ; si un futur changement en demande une, la
doctrine mika#2201 s'applique — *on déclare, on n'allowliste pas* : on route le site fautif par la
garde, on n'ajoute pas de ligne.

---

## Risques et compromis

**Latence ajoutée, quantifiée.** La vérification s'arrête au premier succès ; le coût nominal est une
lecture `gh pr view`, celle-là même que le code fait déjà partout ailleurs. Le coût maximal est le
budget (6 s), payé uniquement quand l'update n'atterrit pas — c'est-à-dire dans le cas où la boucle
allait sinon perdre six heures. Le seul site borné est l'outil (60 s, mesuré ci-dessus) ; 6 s y pèsent
10 %. Les deux sites handler n'ont pas d'enveloppe encadrante et l'absorbent.

**Faux négatif sur atterrissage lent.** Traité en R11 : il relâche le claim, ce qui est le comportement
souhaitable de toute façon. La re-tentative éventuelle rencontre `AlreadyUpToDate`, que
`reconcile_already_up_to_date` (1242) sait déjà arbitrer par re-lecture.

**Le renommage `Updated` → `Accepted` touche un enum `pub(crate)`.** Périmètre clos au crate ; le
compilateur énumère les sites. Le `match` interne de `remediate_behind_main` (1214-1223) est exhaustif
sur `UpdateBranchOutcome` sans bras `_`, donc le renommage y est forcé.

**Le site de relâchement du claim est un `matches!`, et c'est le seul endroit sans filet du
compilateur.** `BehindMainRemediation::Updated { observed_base_sha }` est construite à un site de
production (1215) et lue à **trois** `match` exhaustifs sans bras `_` (1285 trace, 1334 disposition,
1403 prose) — ajouter `AcceptedNotLanded` y est une erreur de compilation, donc une garde. Le
quatrième site, le relâchement (1227), est `if matches!(remediation, …Failed(_))` : **`matches!`
continue de compiler quand une variante apparaît**, ce qui est très exactement la forme que mika#1940 a
dû nommer par écrit (*« `matches!` and `if let` are exactly the two forms that keep compiling when a
variant appears »*). L'étape 8 est donc la seule du plan qu'aucun compilateur ne rappelle, et **le
test 16 — qui asserte le claim relâché, pas seulement la disposition — est ce qui la couvre**. Un test
qui n'assertait que `Blocked { reason: BehindMain }` passerait au vert avec R6 oubliée et D2 grande
ouverte.

**Le budget de 6 s reste un choix, non une mesure**, et il le restera jusqu'à ce que la trace R9
produise une distribution. C'est R11 qui le rend sûr, pas sa valeur. Le jour où
`outcome="accepted_not_landed"` porte du trafic nominal, c'est la **valeur** qu'il faut réviser sur
données, jamais la mesure qu'il faut désarmer.

**Trois `gh pr view` de plus dans le pire cas.** Sur le site de l'outil ils partagent le jeton déjà
résolu et n'ajoutent aucun aller-retour d'authentification. Aucun quota n'est en jeu à ce volume (au
plus trois lectures par PR par mouvement de `main`).

---

## Sonde post-déploiement, et sa halte

Aucun compteur neuf, aucune ligne d'audit : le seul instrument est le jeton de trace de R9.

```bash
# La population que ce fix rend visible — régime attendu : NON VIDE si le défaut existe
grep behind_main_update_branch "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.outcome == "accepted_not_landed") | {site, pr_number, repo, target_main_sha}'

# CONTRÔLE POSITIF — la remédiation tourne-t-elle seulement ?
grep -c behind_main_update_branch "$MIKA_SPIRIT_LOG_FILE"
```

**Halte 1 — zéro `accepted_not_landed` sur 30 jours pendant que le contrôle positif est non nul.**
C'est un **résultat**, pas un échec : le job asynchrone de GitHub aboutit en pratique sous 6 s, et le
no-op-success que ce ticket ferme par lecture du code n'a pas de population mesurée. À écrire tel quel
plutôt qu'à corriger en allongeant le budget pour faire apparaître des lignes.

**Halte 2 — zéro des deux.** On ne peut rien conclure : aucune PR n'a été behind depuis le
déploiement. Vérifier le contrôle positif **avant** toute conclusion — *une garde que personne n'a
exercée se lit exactement comme une garde qui marche* (classe mika#2205).

**Halte 3 — `accepted_not_landed` porte du trafic nominal.** Ce n'est pas le budget qu'il faut
allonger par réflexe : lire d'abord si les PR concernées finissent par atterrir (auquel cas la valeur
de R10 est trop basse et se révise **sur cette distribution**) ou ne l'atterrissent jamais (auquel cas
le job GitHub échoue réellement et c'est la cause amont qu'il faut traiter).

**Préalable à toutes les sondes :** établir que le binaire servi porte le correctif. Une ligne absente
ne prouve rien tant qu'on n'a pas établi que le `mika-spirit` qui tourne sait l'écrire (classe
mika#2340).

---

## Ce que ce plan ne prétend pas

Il ne prouve pas qu'un no-op-success s'est produit en production. La chronologie établie par mika-dev
sur #2251 exclut cette observation. Il ferme un chemin qui, **par lecture du code**, publie un fait non
mesuré et consomme sa seule tentative de réparation en le faisant. La différence importe pour la prose
du commit et pour quiconque relira ce ticket : l'évidence est le fichier, pas l'incident.

Il ne fait pas atterrir un update-branch qui échoue : il rend l'échec **visible et rattrapable** là où
il était indistinguable d'un succès. Et il ne rétro-écrit rien — les `BranchUpdated` déjà rendus au
moniteur portaient un SHA non mesuré et le porteront toujours ; la sonde est la **prochaine**
occurrence.

Il ne ferme pas F2 (le scan de source que le fichier revendique et qui n'existe pas), et ne touche
aucune ligne des deux handlers.

---

## Note d'anticipation

`crates/mika-agent/src/tools/pr_merge_with_gate.rs` est un gate file `@samidarko`
(`.github/CODEOWNERS`). La PR issue de ce ticket **ne fermera pas en autonome** et exigera une revue de
Vincent — anticipé ici pour que ce ne soit pas découvert en fin de course.
