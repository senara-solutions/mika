# Vol 2 de mika#2963 — les trois handlers webhook lisent la CI avec le jeton App

**Ticket :** mika issue#2965
**Vol 1 :** mika issue#2963 (`docs/plans/2026-10-29-005-fix-2963-ci-reader-le-lecteur-ci-lit-plan.md`)
**Lié :** mika issue#2455, mika issue#2228, mika issue#2205, mika issue#2693
**Labels :** `bug`, `p2-normal`, `agent-core`, `ready`, `dispatch:loop`, `loop-substrate`

---

## 1. Contexte

Le vol 1 a livré trois choses, toutes présentes dans l'arbre :

- `Settings::resolve_ci_read_token(&self, github_app: Option<&GitHubApp>) -> Option<CiReadToken>`
  (`crates/mika-common/src/config.rs:2089`) — jeton d'installation d'App d'abord, PAT
  en repli, `None` quand rien n'est configuré ;
- le type `CiReadToken` et son `CiReadTokenSource::{App, Pat}`
  (`crates/mika-common/src/ci_read.rs`), avec `Debug` rédigé et
  `report_read(repo, pr)` qui émet `event = "ci_read_token_source"` ;
- le câblage de **deux** des cinq consommateurs de `run_gh_checks` :
  `pr_merge_with_gate::execute` et le garde de cohérence CI de mika#2455.

Les trois autres consommateurs lisent toujours la CI au PAT, par le repli unique
`ci_read_token_or_pat(None, pat)` (`pr_merge_with_gate.rs:1106`) :

| handler | signature publique | site de lecture |
|---|---|---|
| `server/verdict_handler.rs` | `try_handle_pr_review_verdict` (l. 142) | `handle_pass_verdict` (l. 403) → l. 617 |
| `server/ci_success_handler.rs` | `try_handle_ci_success` (l. 176) | étape 5, l. 434 |
| `server/ci_failure_handler.rs` | `try_handle_ci_failure` (l. 269) | `fetch_failure_context` (l. 939) → l. 950 |

**Ce n'est pas une régression.** C'est le comportement d'avant mika#2963, et le
log de provenance livré au vol 1 le dit honnêtement : ces trois chemins émettent
`ci_read_token_source` avec `source = "pat"`. Le découpage vient de la borne par
vol de mika#2693 — le vol 1 était estimé à 401 lignes pour une borne de 429.

**Ce que le PAT ne peut pas lire.** `gh pr checks <n> --json name,state,bucket,link`
résout ce `--json` par le champ GraphQL `statusCheckRollup`, auquel GitHub
n'accorde pas la permission Checks sur un PAT fine-grained. Mesure du vol 1
(`/var/log/mika/server.log`, 2026-10-07 → 2026-10-29) : 77 échecs
`GraphQL: Resource not accessible by personal access token (node.statusCheckRollup…)`,
dont 55 abstentions du garde mika#2455 sur `reason "gh_failed"`. Le jeton
d'installation de l'App, lui, porte `checks=read`. Tant que ces trois handlers
lisent au PAT, le webhook le plus chaud de la boucle — `verdict_handler`, qui
décide d'un merge sur un `VERDICT: pass` — lit la CI avec le jeton qui ne sait
pas la lire.

---

## 2. Le geste, et ce qu'il n'est pas

Un seul endroit résout, trois endroits reçoivent.

`server/handlers.rs:1790` résout déjà `verdict_github_token` pour les quatre
handlers de la chaîne `github`, et il a `a.settings` et `a.github_app` sous la
main. Le `CiReadToken` se résout **là**, une ligne plus bas, et descend en
paramètre.

```rust
// handlers.rs, à côté de verdict_github_token (l. ~1790)
let ci_read_token = a.settings.resolve_ci_read_token(a.github_app.as_deref()).await;
```

**Aucun type neuf, aucun résolveur neuf, aucune construction de `CiReadToken`.**
Tout existe depuis le vol 1 ; ce vol ne fait que porter une valeur déjà
résolvable jusqu'aux trois sites qui la demandaient en commentaire.

Deux conséquences que ce plan assume explicitement :

- **`merge_ready_handler` n'est pas concerné.** Il lit le périmètre
  (`perimeter::classify_pr_files`) et merge (`run_gh_merge`) ; il n'appelle pas
  `run_gh_checks`. Le ticket nomme trois handlers, et ce sont bien ces trois-là.
- **Les identités de post ne bougent pas.** `pr review`, `pr comment` et
  `pr merge` continuent de passer par `resolve_github_token` (PAT-first,
  ADR-008). Le vol 1 a tranché cette voie ; ce vol ne la relitige pas, et le
  paramètre ajouté est un **second** jeton à côté du premier, jamais un
  remplacement.

---

## 3. Diagnostic : comment AC2 peut être testé

AC2 demande un test montrant que `verdict_handler` lit la CI avec `app` quand
l'App est disponible, et `pat` quand elle ne l'est pas. Trois obstacles, chacun
réglé par une décision nommée.

### 3.1 Un test de bout en bout n'atteint pas la lecture CI, et il faut savoir pourquoi

`handle_pass_verdict` franchit la porte de périmètre (l. 441) **avant** la
lecture CI (l. 617). Cette porte appelle
`perimeter::fetch::fetch_pr_files(pr_number, repo, token)`, qui lance un vrai
`gh` en sous-processus, et le périmètre **tombe fermé** sur erreur de fetch :
`Classification::DecisionCore` est synthétisé et la fonction rend avant la
lecture. C'est la raison structurelle pour laquelle un test naïf avec un jeton
factice serait vert sans jamais avoir lu la CI — donc sans rien prouver.

### 3.2 Le seul point d'injection est un faux `gh` en tête de `PATH`

`run_gh_subprocess` fait `tokio::process::Command::new("gh")`
(`pr_merge_with_gate.rs:2283`) — résolution par `PATH`, donc interceptable. Il
appelle ensuite `scrub_mika_env_vars`, ce qui veut dire qu'**aucune variable
`MIKA_*` ne survit à l'exec** : le répertoire temporaire du faux `gh` doit être
écrit dans le texte du script lui-même, pas passé par l'environnement. Le
harnais existe déjà deux fois dans le dépôt et sera dupliqué tel quel :
`wip_rescue.rs:2540-2600` (`PathGuard`, `prepend_to_path`, `install_fake_gh`) et
`tests/worktree_reap_t7_wiring_2623.rs:151`, qui porte la même note.

Le faux `gh` répond à deux invocations :

- `pr view --json files` → `{"files":[{"path":"docs/plans/….md"}]}`. Le préfixe
  `docs/plans/` est dans `MECHANICAL_PREFIXES` (`perimeter/rules.rs:73`), donc
  le périmètre se classe MECHANICAL et le flot continue ;
- `pr checks` → une sortie dont **un check échoue**, pour que le flot s'arrête
  juste après la lecture, sans effet de bord de merge.

`report_read` est émis **avant** le sous-processus
(`run_gh_checks_raw`, `pr_merge_with_gate.rs:1176`), donc la ligne de provenance
part de toute façon : une lecture tentée et échouée est quand même comptée.
C'est la propriété mika#2205 du vol 1, et c'est elle qui rend ce test possible.

**Un faux `gh` n'est pas du réseau.** Le vol 1 a écrit noir sur blanc qu'une
assertion passant par un *vrai* `gh` « serait verte sur un poste à réseau et
rouge en CI, ce qui est pire que son absence » (en-tête de
`tests/eval/test_ci_read_token_gate_2963.rs`) — et refuser le réseau est la
doctrine mika#2495/mika#2569. Le shim `PATH` ne contredit rien de cela : il
n'ouvre aucune socket, il est déterministe, et deux suites du dépôt tournent
déjà sur ce motif en CI. La différence avec le vol 1 n'est pas une réouverture
de son arbitrage, c'est un point d'injection qu'il n'avait pas eu besoin de
poser.

### 3.3 Où le test doit vivre, et pourquoi pas dans `tests/eval/`

L'observation de la provenance passe par `install_capturing_subscriber`
(`src/test_utils.rs:64`), seul site d'installation de souscripteur `tracing`
autorisé (mika#2646) et **`#[cfg(test)]`-gaté** : il est donc inatteignable
depuis `tests/eval/`, qui compile contre la bibliothèque. Le test vit dans un
`#[cfg(test)] mod mika2965_tests` **inline** dans
`src/server/verdict_handler.rs`, à côté de ses quatre voisins
(`mika2237_sole_writer` l. 3296, `tests` l. 3386, `mika2667_tests` l. 4877,
`mika1381_tests` l. 5033).

`#[serial_test::serial]` est **obligatoire** et pour deux raisons indépendantes :
`std::env::set_var("PATH")` est un état de processus, et
`install_capturing_subscriber` touche le cache d'intérêt des callsites
(`rebuild_interest_cache()`), dont la doc dit que `#[serial]` est « la moitié
porteuse » et doit couvrir aussi les voisins empoisonneurs.

L'App de test vient de
`GitHubApp::new_with_test_token("ghs_app_installation")`
(`crates/mika-common/src/github_app.rs:164-176`, gaté
`#[cfg(any(test, feature = "test-utils"))]`), atteignable depuis les mods de test
de `src/` de `mika-agent` (dev-dep avec `features = ["test-utils"]`,
`Cargo.toml:91` ; précédent à `src/task_engine/dispatcher.rs:5792`).

### 3.4 Les deux sources dans UN SEUL test

Le vol 1 a posé la règle et elle s'applique telle quelle : « deux tests séparés
passeraient tous les deux sur un arbre où le champ est écrit en dur ». Le test
AC2 fait donc les deux lectures dans le même corps, sur **deux numéros de PR
distincts** et deux tâches semées distinctes, pour qu'aucune interférence par PR
ne vienne brouiller l'assertion — puis asserte que les deux lignes
`ci_read_token_source` capturées portent `source = "app"` et `source = "pat"`,
chacune corrélée à son numéro de PR.

---

## 4. Exigences

### R1 — `handlers.rs` résout une fois

Dans la branche `if req.channel == "github" {`, à côté de la résolution de
`verdict_github_token` (l. 1790), ajouter la résolution du jeton de lecture CI,
et passer `ci_read_token.as_ref()` aux trois appels :
`try_handle_pr_review_verdict` (l. 1794), `try_handle_ci_success` (l. 1826),
`try_handle_ci_failure` (l. 1884). `try_handle_merge_ready` (l. 1857) est
inchangé.

Le commentaire du site dit ce que le paramètre est et ce qu'il n'est pas : un
jeton de **lecture**, App-first, distinct du `verdict_github_token` PAT-first qui
porte les identités de post.

### R2 — `verdict_handler.rs` reçoit et transmet

`try_handle_pr_review_verdict` prend un paramètre
`ci_read_token: Option<&CiReadToken>` et le passe à `handle_pass_verdict`, qui
le passe à `ci_read_token_or_pat`. La fonction publique porte déjà
`#[allow(clippy::too_many_arguments)]` ; `handle_pass_verdict` passe de 6 à 7
paramètres et en aura besoin à son tour.

Le bloc de commentaire de la l. 610-617 est **réécrit**, pas supprimé : il
affirme aujourd'hui qu'un handler webhook « ne porte pas de `ToolContext` »,
donc que `None` dégrade vers le PAT, et que peupler ce jeton « demanderait de
faire descendre un `GitHubApp` jusqu'aux trois handlers, hors du périmètre de ce
ticket ». Les deux phrases cessent d'être vraies dans ce vol ; laissées en place
elles enseigneraient le contraire du code. La version nouvelle dit d'où vient le
jeton (résolu une fois dans `handlers.rs`) et pourquoi le repli au PAT subsiste
(déploiement sans App, ou échange d'App échoué — un refus honnête, gate fermé).

### R3 — `ci_success_handler.rs` reçoit

`try_handle_ci_success` prend le même paramètre et le passe à
`ci_read_token_or_pat` à l'étape 5 (l. 434). Commentaire mika#2963 réécrit de
même.

### R4 — `ci_failure_handler.rs` reçoit et le fait descendre d'un niveau

`try_handle_ci_failure` prend le paramètre ; `fetch_failure_context`, privée
(l. 939), prend un paramètre de plus et l'utilise l. 950. L'appel l. 462 est mis
à jour. Commentaire mika#2963 réécrit de même.

### R5 — recensement des sites d'appel existants

Un paramètre de plus sur trois fonctions publiques touche tous leurs appelants.
Compte à ce jour : 32 occurrences de `try_handle_pr_review_verdict(`, 11 de
`try_handle_ci_success(`, 3 de `try_handle_ci_failure(` — soit, définitions et
site de production retirés, **≈ 40 sites d'appel de test** à compléter d'un
`None`. Ce sont des mises à jour mécaniques d'une ligne ; `None` y est le bon
argument, puisque ces tests ne mesurent pas la provenance.

**Trois scans textuels ont été vérifiés insensibles à l'ajout** (leurs aiguilles
ne portent pas sur l'arité) :

| scan | aiguille |
|---|---|
| `tests/eval/test_ci_success_handler.rs:75` | `"pub async fn try_handle_ci_success("` |
| `tests/eval/test_merge_identity_2248.rs:109` | `"ci_success_handler::try_handle_ci_success("` |
| `tests/eval/test_merge_identity_2248.rs:115` | `"ci_failure_handler::try_handle_ci_failure("` |

Et le détecteur de sole-writer du vol 1 (`pr_merge_with_gate.rs:~3075`,
`SITES_AUTORISES = ["ci_read_token_or_pat"]`) scanne
`crates/mika-agent/src` à la recherche de `CiReadToken::new(` hors de ce corps :
**ce vol ne construit aucun `CiReadToken`**, donc il ne mord pas.

### R6 — T1, le test comportemental d'AC2

`#[cfg(test)] mod mika2965_tests` inline dans `src/server/verdict_handler.rs`.
Un seul test, `#[tokio::test]` + `#[serial_test::serial]` :

1. `install_capturing_subscriber` pour capturer les lignes `tracing` ;
2. `tempfile::TempDir` + `install_fake_gh` + `prepend_to_path` (`PathGuard` rendu
   en fin de test), le faux `gh` répondant `pr view --json files` par un chemin
   `docs/plans/…` et `pr checks` par un check en échec ;
3. base en mémoire, deux tâches `in_progress` semées sur deux `pr_url` distincts
   (motif de `tests/eval/test_verdict_handler.rs`), deux textes d'événement
   `VERDICT: pass` sous le login relecteur ;
4. passe 1 — `Settings::resolve_ci_read_token(Some(&GitHubApp::new_with_test_token(…)))`
   rend un jeton de source `App` ; appel sur la PR n° 1 ;
5. passe 2 — `resolve_ci_read_token(None)` sur des `Settings` portant un PAT rend
   un jeton de source `Pat` ; appel sur la PR n° 2 ;
6. assertions sur les lignes capturées : une ligne `ci_read_token_source` avec
   `source = "app"` et `pr = <n°1>`, une avec `source = "pat"` et `pr = <n°2>`.

L'assertion croisée sur le numéro de PR est ce qui empêche le test de passer sur
un arbre où la source serait écrite en dur.

### R7 — T2, le scan structurel d'AC1

AC1 a une moitié que le comportement ne voit pas : « résolu **une seule fois**
dans `server/handlers.rs` ». Une seconde résolution, ou un `None` passé par
inadvertance à l'un des trois appels, laisserait T1 vert (il appelle le handler
directement) et toute la suite verte.

Nouveau fichier `tests/eval/test_ci_read_token_handlers_2965.rs`, enregistré
dans `tests/eval.rs` à côté de `mod test_ci_read_token_gate_2963;` (l. 246), sur
le motif exact de `test_merge_identity_2248.rs` :
`const HANDLERS_SRC: &str = include_str!("../../src/server/handlers.rs");` —
source épinglée à la compilation, donc l'assertion ne peut pas passer contre une
copie périmée sur le disque. Trois énoncés :

1. `resolve_ci_read_token(` apparaît **exactement une fois** dans `HANDLERS_SRC` ;
2. les trois appels de handler portent l'argument : pour chacune des aiguilles
   `try_handle_pr_review_verdict(`, `ci_success_handler::try_handle_ci_success(`,
   `ci_failure_handler::try_handle_ci_failure(`, la région qui suit l'aiguille
   jusqu'à son `.await` contient `ci_read_token.as_ref()` ;
3. **contrôle de bonne foi** : les trois aiguilles sont présentes. Sans lui, un
   renommage de handler rendrait le scan vert sur un fichier où plus rien ne
   lit rien — la classe « le détecteur n'a rien lu » que les gardes du dépôt
   ferment par des ancres nommées.

---

## 5. Contrat de vérification

| énoncé | ce qui le tient |
|---|---|
| AC1, moitié « trois handlers reçoivent » | la compilation : le paramètre n'est pas optionnel |
| AC1, moitié « résolu une seule fois dans `handlers.rs` » | T2 (R7) |
| AC2 | T1 (R6) |
| AC3 | inchangé par construction — aucun site de `resolve_github_token` n'est touché, et `test_merge_identity_2248.rs` + `test_ci_read_token_gate_2963.rs` restent verts sans modification |
| AC4 | `scripts/verify-pipeline.sh origin/main`, `cargo test`, `cargo clippy`, `make lint` |

---

## 6. Ordre de commit

1. **`feat(2965): le jeton de lecture CI descend aux trois handlers webhook`**
   — R1 à R4 plus les ≈ 40 sites d'appel de test (R5). Un seul commit, parce que
   les quatre fichiers et leurs appelants ne compilent qu'ensemble : couper
   ailleurs livrerait un arbre rouge.
2. **`test(2965): verdict_handler lit la CI en app puis en pat`** — R6.
3. **`test(2965): un scan tient la résolution unique dans handlers.rs`** — R7.

Les deux détecteurs atterrissent **après** le câblage, donc armés et verts à la
livraison. Ils n'ont jamais été vus rouges sur cet arbre pour la raison qui rend
l'ordre inverse impossible : avant le commit 1 ils ne compilent pas. Le contrôle
de bonne foi de T2 (R7, énoncé 3) est ce qui remplace ici la démonstration « vu
rouge d'abord » — il asserte que le scan a bien lu quelque chose.

---

## 7. Risques nommés

- **Le faux `gh` doit couvrir les deux invocations.** S'il n'en couvre qu'une, le
  périmètre tombe fermé et le test est vert sans avoir lu la CI. Les assertions
  de T1 portent sur les lignes `ci_read_token_source` capturées, dont l'absence
  fait échouer le test — pas sur le `VerdictAction` rendu, qui serait vert dans
  les deux cas.
- **`#[serial]` sur les voisins.** Si un test voisin non sérialisé empoisonne le
  cache d'intérêt des callsites, T1 devient instable. La doc de
  `install_capturing_subscriber` prescrit `#[serial]` sur les empoisonneurs aussi ;
  à vérifier sur les quatre mods de test de `verdict_handler.rs` au moment de
  l'implémentation.
- **≈ 40 sites d'appel mécaniques** sont le gros du volume et la principale
  source de tours. `cargo check` après le premier passage, pas `cargo test`, pour
  les cueillir en bloc.
- **CODEOWNERS.** `verdict_handler.rs` est sous @samidarko : la PR ne fermera pas
  en autonome, et c'est attendu.

---

## Definition of Done

- [ ] `handlers.rs` résout `resolve_ci_read_token` une fois, à côté de
      `verdict_github_token`, et passe `ci_read_token.as_ref()` aux trois handlers.
- [ ] `try_handle_pr_review_verdict`, `try_handle_ci_success` et
      `try_handle_ci_failure` portent un paramètre `Option<&CiReadToken>` qui
      atteint `ci_read_token_or_pat` à chacun des trois sites de lecture.
- [ ] Les trois commentaires mika#2963 affirmant que le repli `None` est hors
      périmètre sont réécrits — aucun commentaire ne survit en contradiction avec
      le code.
- [ ] Les ≈ 40 sites d'appel de test compilent, et les trois scans textuels
      recensés en R5 restent verts sans modification.
- [ ] T1 (`mod mika2965_tests` dans `verdict_handler.rs`) asserte les deux
      sources dans un seul test, sur deux numéros de PR distincts.
- [ ] T2 (`tests/eval/test_ci_read_token_handlers_2965.rs`) asserte la résolution
      unique, les trois passages d'argument et la présence des trois ancres.
- [ ] Aucun `CiReadToken::new(` nouveau sous `crates/mika-agent/src` : le
      détecteur de sole-writer du vol 1 reste vert avec son allowlist inchangée.
- [ ] Aucun site de `resolve_github_token` touché.
- [ ] `scripts/verify-pipeline.sh origin/main`, `cargo test`, `cargo clippy` et
      `make lint` passent.

## Acceptance criteria

- **AC1.** Les trois handlers reçoivent un `Option<&CiReadToken>` résolu
  par `Settings::resolve_ci_read_token`, résolu une seule fois dans
  `server/handlers.rs`.
- **AC2.** Un test montre que `verdict_handler` lit la CI avec la source
  `app` quand l'App est disponible, et `pat` quand elle ne l'est pas.
- **AC3.** Les identités de post restent intactes : `pr review`,
  `pr comment` et `pr merge` continuent de passer par
  `Settings::resolve_github_token` (PAT-first). Le vol 1 l'a tranché
  (voie (i)) ; ce vol ne le relitige pas.
- **AC4.** `scripts/verify-pipeline.sh origin/main`, `cargo test`,
  `cargo clippy` et les lints du dépôt passent.

## Fire-Disposition

Ce plan livre deux détecteurs — T1 (R6), test comportemental, et T2 (R7), scan
structurel.

**Disposition retenue : (a), exception nommée en allowlist — et l'allowlist est
vide à la livraison.**

Détail d'implémentation, par détecteur :

- **T2** n'a aucune violation existante à exempter, parce que le câblage qu'il
  mesure atterrit dans le **même vol**, au commit qui le précède. Il n'y a donc
  pas de constante d'exemption à écrire : l'allowlist est vide, matériellement
  (aucune liste), et le détecteur atterrit armé et vert. C'est exactement la
  forme que le vol 1 a livrée pour son scan de sole-writer, et la raison est la
  même : une exception vide ne peut pas devenir l'allowlist que mika#2309
  condamne.
- **La propriété auto-nettoyante porte donc sur les ancres, pas sur des
  exemptions** : l'énoncé 3 de R7 asserte que les trois aiguilles de signature
  existent dans `handlers.rs`. Un renommage de handler, ou le retrait d'un
  appel, fait **rougir** le scan au lieu de le laisser passer sur un fichier où
  plus rien ne lit rien. C'est le garde-fou « ce détecteur a-t-il lu quelque
  chose ? » que le dépôt impose à ses scans de source, et il remplace ici
  l'assertion de staleness qu'une allowlist non vide aurait portée.
- **T1** est armé sans `#[ignore]` ni gate : il mesure un comportement que le
  commit 1 rend vrai, donc il n'a jamais de raison d'atterrir désarmé. Son coût
  nommé est `#[serial_test::serial]`, qui le sérialise avec les autres porteurs
  du fichier.

Les deux détecteurs vivent dans `cargo test` — T1 en `#[cfg(test)]` inline, T2
dans la suite `eval` déjà câblée. Ni `scripts/` ni `skills/bundled/_shared/` ne
sont touchés, donc `scripts/check-suite-wiring.sh` n'a rien à en connaître et
`scripts/suite-wiring.tsv` n'est pas modifié.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/handlers.rs` | 20 |
| `crates/mika-agent/src/server/verdict_handler.rs` (production) | 25 |
| `crates/mika-agent/src/server/ci_success_handler.rs` | 12 |
| `crates/mika-agent/src/server/ci_failure_handler.rs` | 15 |
| mise à jour des ≈ 40 sites d'appel de test existants | 45 |
| T1 — `mod mika2965_tests` inline (harnais faux `gh` compris) | 120 |
| T2 — `tests/eval/test_ci_read_token_handlers_2965.rs` + enregistrement | 45 |

Total estimé : 282 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol.** 282 estimées contre une borne par vol de 429 et un
seuil par PR de 1000 : les deux bornes passent, et la plus serrée — celle du vol
— garde 147 lignes de marge. Le volume n'est pas réparti en inconnues : 72 lignes
sont du passage de paramètre sur quatre fichiers déjà lus et cités à la ligne,
45 sont mécaniques et dénombrées (R5), et les 165 restantes sont les deux tests,
dont le harnais est une duplication d'un motif existant à deux endroits du dépôt
(`wip_rescue.rs:2540-2600`, `tests/worktree_reap_t7_wiring_2623.rs:151`) et non
une invention. Aucun type neuf, aucun résolveur neuf, aucune migration, aucun
fichier de production créé. Le vol 1 a déjà payé la conception ; celui-ci paie le
câblage.
