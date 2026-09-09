---
title: "fix: `Accepted` n'est pas `Updated` — vérifier l'atterrissage de l'update-branch avant de le déclarer"
issue: 2252
type: fix
depth: Standard
origin: 2238
created: 2026-09-09
---

# fix: `Accepted` n'est pas `Updated`

## Résumé

`attempt_update_branch` (mika#2250) émet un `PUT .../update-branch` et traite un exit zéro comme
« la branche est à jour ». L'endpoint répond **`202 Accepted`** et effectue le merge de façon
**asynchrone** : le zéro prouve que GitHub a pris la requête, jamais qu'un commit existe. Le
doc-comment de la fonction le sait et le dit ; les deux doc-comments d'enum qu'elle alimente
affirment l'inverse, et le résultat rendu au moniteur (`BranchUpdated { new_main_sha }`) **publie
une donnée jamais mesurée**.

Quand le job asynchrone échoue, la boucle a consommé son unique tentative, déclaré la réparation
faite, et engagé un rendez-vous sur un webhook CI qui n'arrivera pas. La PR reste BEHIND, présentée
comme réparée, pendant **jusqu'à six heures** (`UPDATE_ATTEMPT_TTL`) ou jusqu'à ce que `main` bouge.

Ce plan remplace la déclaration par une **mesure** : après le `202`, re-lire le `baseRefOid` sous un
budget borné, et n'émettre `Updated` que si la base a réellement avancé. Sinon, émettre un état
nommé distinct et **relâcher le claim**, pour que la prochaine arrivée puisse re-tenter au lieu
d'attendre six heures.

---

## Cadre du problème

### Ce que le code fait aujourd'hui (mesuré, 2026-09-09, sur `main` @ 037fe33d)

`crates/mika-agent/src/tools/pr_merge_with_gate.rs`

| Ancre | Ligne | Fait |
|-------|-------|------|
| `attempt_update_branch` | 993 | `run_gh_subprocess(["api","--method","PUT", ".../update-branch"])` → `Ok(_) => UpdateBranchOutcome::Updated` |
| doc de `attempt_update_branch` | 986-991 | « **`Updated` means accepted, not finished.** The endpoint answers `202 Accepted` and performs the merge asynchronously, so a zero exit proves GitHub took the request, never that a commit exists. » |
| `UpdateBranchOutcome::Updated` | 847-848 | « GitHub accepted the update — **a new commit now sits on the PR head.** » |
| `BehindMainRemediation::Updated` | 860-861 | « The branch **was brought up to date.** A fresh CI run is expected. » |
| `disposition_for_remediation` | 1211-1214 | `Updated => MergeGateResult::BranchUpdated { pr_base_sha, new_main_sha: info.current_main_sha }` |
| `release_update_branch_attempt` | 931 | appelé **uniquement** pour `Failed` (1093-1095) |
| `UPDATE_ATTEMPT_TTL` | 838 | `6 * 3600` secondes |

### Les trois défauts

**D1 — Le résultat affirme un fait qu'il n'a pas mesuré.** `BranchUpdated { new_main_sha }` est un
champ **de données**, sérialisé vers le moniteur et vers le LLM, qui déclare que la base de la PR
vaut désormais `current_main_sha`. Rien ne l'a lu. Les deux doc-comments d'enum contredisent
frontalement le doc-comment de la fonction qui les produit — la contradiction est *dans le fichier*,
pas entre le fichier et une hypothèse.

**D2 — La récupération est bornée par six heures, pas par l'échec.** Le claim anti-thrash est
consommé avant l'appel (délibérément, 916-918) et n'est relâché que pour `Failed`. Un `202` suivi
d'un job qui échoue laisse le claim posé. Le seul déblocage est un mouvement de `main` (nouvelle
clé de claim) ou l'expiration du TTL de six heures. Le code nomme lui-même la conséquence à la
ligne 920-922 : *« nothing evicts the entry until the ledger hits its soft cap, and with a
serialized dispatch `main` may not advance for hours. A permanent stall is exactly the state
mika#2238 exists to end »* — le raisonnement était juste, sa conclusion n'a été appliquée qu'à
`Failed`.

**D3 — Le rendez-vous différé n'a pas de gardien.** R3 de mika#2238 a délibérément supprimé le
re-merge dans le même tour au profit d'un rendez-vous sur le webhook CI. Ce rendez-vous suppose
qu'un commit apparaisse. Si le job asynchrone échoue, il n'y a ni commit, ni CI, ni webhook — donc
aucun tour suivant ne vient re-mesurer. L'absence de signal est indistinguable du succès.

### Ce que l'évidence établit, et ce qu'elle n'établit pas

Le commentaire de `mika-platform-dev` du 2026-09-09T06:31Z **corrige la chronologie du corps du
ticket** : sur PR#2251, `c2fa9536` est le merge commit produit par claude-pilot
(`resolve_pr_conflicts`, task 2123c652, atterri 06:21Z), **avant** le déploiement 08:05 du fix
#2250. La PR est `CLEAN`, plus BEHIND. Aucune exécution autonome post-déploiement n'a donc été
observée : **le no-op-success n'est pas mesuré en production.**

Ce qui est mesuré, en revanche, est structurel et suffit : le chemin `202 → Updated → BranchUpdated`
ne comporte aucune lecture, et la contradiction D1 est lisible dans le fichier. On ferme un trou
établi par lecture du code, pas une observation contestée. Ce plan ne prétend pas expliquer #2251.

---

## Exigences

- **R1** — `UpdateBranchOutcome::Updated` est renommé `Accepted`. Son nom et son doc-comment disent
  ce que le `202` prouve — la requête est prise — et rien de plus. Aucune variante de cet enum
  n'affirme plus l'existence d'un commit.

- **R2** — `remediate_behind_main` **vérifie** l'atterrissage après un `Accepted`, par re-lecture du
  `baseRefOid` de la PR, sous un budget borné et nommé. La vérification s'arrête au premier succès :
  aucune latence ajoutée quand l'atterrissage est immédiat.

- **R3** — Deux issues distinctes remplacent l'unique `BehindMainRemediation::Updated` :
  - `Updated { observed_base_sha }` — la re-lecture a constaté que la base a avancé. Le SHA porté
    est **celui qui a été lu**, jamais `info.current_main_sha`.
  - `AcceptedNotLanded` — le budget a expiré sans que la base bouge.

- **R4** — `AcceptedNotLanded` **relâche le claim** (`release_update_branch_attempt`). La prochaine
  arrivée re-mesure et peut re-tenter, au lieu d'attendre six heures ou un mouvement de `main`.

- **R5** — `MergeGateResult::BranchUpdated.new_main_sha` porte le SHA **observé** (R3), pas le SHA
  visé. `AcceptedNotLanded` rend une disposition distincte de `BranchUpdated` — la boucle ne doit
  pas lire « réparé » sur un atterrissage non constaté.

- **R6** — La trace (`log_behind_main_remediation`) distingue les deux issues par des `outcome`
  différents, de sorte que le moniteur puisse compter les acceptations non atterries.

- **R7** — Le budget de vérification est une constante nommée avec sa justification écrite, et il est
  paramétrable dans les tests (même discipline que `claim_update_attempt_in`, qui abstrait `now`).

- **R8** — Un faux négatif de vérification (l'atterrissage arrive après l'expiration du budget) est
  **bénin par construction** : il relâche le claim, et le tour suivant mesure `is_behind_main` qui
  rendra « plus behind ». Aucun double merge commit n'en résulte. Cette propriété est écrite dans le
  doc-comment, parce que c'est elle qui autorise un budget court.

---

## Hors périmètre

- La détection `is_behind_main` (mika#1577) — inchangée, elle reste l'autorité sur « behind ».
- Les trois sites d'appel de `remediate_behind_main` — ils reçoivent une variante de plus, leur
  ordonnancement ne bouge pas.
- Les prompts embarqués (R6 de #2238) — l'énumération des `blocked.reason` n'est pas touchée ; si
  `AcceptedNotLanded` doit y apparaître, c'est un ticket séparé.
- Le gate de SHA périmé qui retient la PR après un update (documenté 455-460) — explicitement suivi
  ailleurs.
- Toute tentative d'expliquer rétrospectivement l'état de PR#2251 : la chronologie établie par
  mika-dev l'exclut du périmètre.

---

## Étapes

### Phase 1 — Nommer ce que le `202` prouve

1. Renommer `UpdateBranchOutcome::Updated` → `Accepted` (846-855). Réécrire son doc-comment : la
   requête est acceptée ; l'existence d'un commit n'est pas établie ici.
2. Mettre à jour `attempt_update_branch` (993) et `classify_update_branch_error` (1011) pour la
   nouvelle variante. Aucun changement de comportement réseau.
3. Réécrire le doc-comment de `BehindMainRemediation::Updated` (860-861) : il décrira, après la
   phase 2, un fait **observé**.

### Phase 2 — Mesurer l'atterrissage

4. Ajouter la constante de budget (R7) à côté de `UPDATE_ATTEMPT_TTL` (838), avec sa justification.
5. Ajouter un helper `await_update_branch_landing(pr_number, repo, token, before_base_sha, budget)`
   qui re-lit `run_gh_pr_view(...).base_ref_oid` jusqu'à ce qu'il diffère de `before_base_sha` ou que
   le budget expire. Rend `Some(observed_sha)` ou `None`. Paramétré sur le budget pour les tests.
6. Dans `remediate_behind_main` (1059), remplacer `Accepted => BehindMainRemediation::Updated` par
   l'appel au helper, produisant `Updated { observed_base_sha }` ou `AcceptedNotLanded`.
7. Étendre `release_update_branch_attempt` au cas `AcceptedNotLanded` (le `matches!` de 1093).

### Phase 3 — Propager le fait mesuré

8. Ajouter `BehindMainRemediation::AcceptedNotLanded` (859-889) avec un doc-comment qui nomme les
   trois défauts fermés.
9. `disposition_for_remediation` (1198) : `Updated { observed_base_sha }` → `BranchUpdated` portant
   le SHA **observé** ; `AcceptedNotLanded` → une disposition distincte qui dit « acceptée,
   atterrissage non constaté, une nouvelle tentative est ouverte ».
10. `describe_behind_main_remediation` (1271) et `log_behind_main_remediation` (1153) : prose et
    `outcome` distincts pour la nouvelle variante (R6).

### Phase 4 — Épingler

11. Test : un `Accepted` dont la re-lecture ne bouge jamais produit `AcceptedNotLanded` **et** un
    claim relâché (le second appel sur le même SHA cible doit repasser).
12. Test : un `Accepted` dont la re-lecture bouge produit `Updated { observed_base_sha }` portant le
    SHA lu, et **conserve** le claim.
13. Test : `BranchUpdated.new_main_sha` provient de l'observation — un cas où le SHA observé diffère
    de `info.current_main_sha` (une PR qui a atterri pendant que `main` re-bougeait) doit sérialiser
    le SHA observé.
14. Test de source-scan existant (2383, 2439) : vérifier qu'il tient toujours après le renommage.

---

## Critères d'acceptation

- **AC1** — Aucune variante d'enum de ce module n'affirme, dans son nom ou son doc-comment, qu'un
  commit existe sur la base d'un code retour d'API. Vérifiable : le doc de la variante issue du `202`
  dit « accepté » ; celui de la variante issue de la re-lecture dit « observé ».
- **AC2** — `MergeGateResult::BranchUpdated.new_main_sha` est le SHA lu sur la PR après
  l'update-branch, jamais `info.current_main_sha`. Épinglé par le test 13.
- **AC3** — Un `202` dont l'effet n'atterrit pas dans le budget produit une disposition **distincte**
  de `BranchUpdated`, et relâche le claim. Épinglé par le test 11.
- **AC4** — Un `202` dont l'effet atterrit produit `BranchUpdated` et **conserve** le claim (le
  plafond anti-thrash de #2238 R4 reste en vigueur pour le cas nominal). Épinglé par le test 12.
- **AC5** — La trace émet des `outcome` distincts pour « atterri » et « accepté non atterri », de
  sorte qu'une requête sur les traces puisse compter les seconds.
- **AC6** — `cargo test -p mika-agent` et `cargo clippy --all-targets -- -D warnings` passent.
- **AC7** — Le doc-comment du helper de vérification écrit explicitement pourquoi un faux négatif est
  bénin (R8) — c'est la justification du budget court, et sans elle un lecteur futur allongera le
  budget pour de mauvaises raisons.

---

## Risques et compromis

**Latence ajoutée dans un handler webhook.** La vérification s'arrête au premier succès ; le coût
nominal est une lecture `gh pr view`, celle-là même que le code fait déjà partout ailleurs. Le coût
maximal est le budget, payé uniquement quand l'update n'atterrit pas — c'est-à-dire dans le cas où la
boucle allait sinon perdre six heures.

**Faux négatif sur atterrissage lent.** Traité en R8 : il relâche le claim, ce qui est le
comportement souhaitable de toute façon. La re-tentative éventuelle rencontre `AlreadyUpToDate`, que
`reconcile_already_up_to_date` (1117) sait déjà arbitrer par re-lecture.

**Le renommage `Updated` → `Accepted` touche un enum `pub(crate)`.** Périmètre clos au crate ; le
compilateur énumère les sites. Le test de source-scan (2439) porte une chaîne littérale mentionnant
`attempt_update_branch` — non affectée par le renommage de la variante, à re-vérifier en étape 14.

---

## Ce que ce plan ne prétend pas

Il ne prouve pas qu'un no-op-success s'est produit en production. La chronologie établie par mika-dev
sur #2251 exclut cette observation. Il ferme un chemin qui, **par lecture du code**, publie un fait
non mesuré et consomme sa seule tentative de réparation en le faisant. La différence importe pour la
prose du commit et pour quiconque relira ce ticket : l'évidence est le fichier, pas l'incident.
