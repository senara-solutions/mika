# mika#3022 — le merge d'une PR de la boucle laisse `ready` sur le ticket qu'elle sert

- **Ticket :** `senara-solutions/mika#3022`
- **Branche :** `fix/3022/ready-le-merge-d-une-pr-de-la-boucle`
- **Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
- **Source :** parking mika#2983, commentaire 1 (2026-11-01) et son additif (2026-11-03)

## Contexte mesuré

Cinq merges depuis le 2026-10-30 ont laissé `ready` posé sur le ticket que la PR
servait, et dans les cinq cas **une main humaine** (compte `samidarko`) l'a retiré
après coup — une fois *avant* le merge (#2194 / #3017, 11-02 18:00:32Z pour un merge
à 18:04:07Z), ce qui dit bien que le geste est manuel et non asservi. Le moteur n'a
retiré `ready` que deux fois sur la période, et seulement sous `mika-platform-qa`
après une PR draft de sauvetage.

Le site existant, `crates/mika-agent/src/server/draft_pr_opened_handler.rs`
(mika#1849), retire `ready` sur `PR opened` **draft uniquement**, et choisit les
tickets par `closingIssuesReferences`. Sur la population visée — un ticket gardé
ouvert en travers de plusieurs vols — ce champ est **vide** : #3008, #3014, #3017,
#3018 et #3020 rendent toutes `closing=[]`. La branche est donc le seul porteur de
`<N>` sur ces événements, et le texte de `PR closed` la porte déjà
(`(branch: <type>/<N>/…)`), ainsi que `Merged: true`.

Entre le merge et le retrait manuel, le ticket reste exposé au feeder : c'est la
chaîne du parking (tâche `ready-label` 74130014 à 14:00:04Z, puis un tour de mika-dev
qui ferme #3002 à 14:46:22Z).

## Requirements

1. Sur un `[GitHub] PR closed:` portant `Merged: true` et une branche de la forme
   `<type>/<N>/…`, le label `ready` est retiré du ticket `#N`, **par le code**, avant
   tout tour LLM.
2. Le même événement avec `Merged: false` ne produit **aucun** geste.
3. Aucune route d'ajout de `ready` n'est créée, et aucune garde de voie B (mika#2995)
   ne reçoit d'entrée nouvelle dans ses allowlists.
4. Chaque cause de non-geste — branche hors forme, dépôt non reconnu, erreur `gh` —
   laisse passer l'événement et émet une ligne de log **nommée** qui dit laquelle.
5. Aucun échec `gh` ne fait perdre le tour ni ne panique (fail-open).

## Design decisions

### D1 — Un handler structurel dédié, à côté du patron, pas dedans

Livrable : `crates/mika-agent/src/server/merged_pr_ready_cleanup_handler.rs`, de la
forme « injecteur à effet de bord seul » — il rend `Passthrough { enrichment: None }`
sur **tous** ses chemins, et jamais `Handled` / `Dispatched`.

Pourquoi un module neuf plutôt qu'une branche de plus dans
`draft_pr_opened_handler.rs` : ce dernier est épinglé sur l'absence d'effet hors draft
(`plan_cleanup_non_draft_is_noop`) et sélectionne par `closingIssuesReferences`. Le
nouveau chemin sélectionne par **branche** et sur un **autre préfixe d'événement** —
deux prédicats de population distincts dans un même handler, c'est la forme mika#2158.
Le geste `gh`, lui, reste partagé (D3).

### D2 — Aucun parseur nouveau : quatre lecteurs canoniques existants sont **appelés**

Le dépôt refuse structurellement un second lecteur d'une grammaire d'événement
(`canonical_tokens.rs::mika2649_aucun_second_parseur_de_grammaire_devenement`,
recensement `EVENT_GRAMMAR_PARSER_SITES`), et sa section RÉSOLUTION nomme les lecteurs
à appeler. Ce vol les appelle tous :

| ce qu'il faut lire | lecteur appelé | site |
|---|---|---|
| le préfixe `[GitHub] PR closed:` | `PR_CLOSED_PREFIX` | `server/upstream_close_handler.rs:34` |
| « la PR est mergée » | `MERGED_TRUE_LINE` | `server/upstream_close_handler.rs:37` |
| `<repo>` et `<pr>` du header | `webhook_queue_v2::parse_pr_action_event` | `server/webhook_queue_v2.rs:135` |
| la branche du header | `upstream_close_handler::extract_head_branch` | `server/upstream_close_handler.rs` |
| `<N>` porté par la branche | `worktree_reaper::issue_number_from_branch` | `worktree_reaper.rs:2617` |

Trois promotions `pub(crate)` sont nécessaires (`PR_CLOSED_PREFIX`,
`MERGED_TRUE_LINE`, `extract_head_branch`) : précédents exacts `extract_pr_url`
(mika#2680) et `extract_issue_url` (mika#2854), promues dans ce même fichier
précisément pour qu'une quatrième copie du scanner ne soit pas écrite.
`issue_number_from_branch` est déjà `pub` et déjà gardée par une assertion
auto-nettoyante (`canonical_tokens.rs:7267`) : elle accepte `feat/2482/slug` et
`incident/1696`, refuse `main`, `feat/slug` et `/2482/x`, et **n'invente jamais** de
numéro (B3.2). C'est la définition de la forme dans ce dépôt ; le plan ne la
redéfinit pas.

`extract_head_branch` lit la **première** ligne et son **dernier** `(branch: …)`, donc
un titre de PR qui porte la forme ne déplace pas la branche réelle
(`mika2242_a_title_carrying_the_shape_does_not_displace_the_branch`).

> Dette nommée, hors vol : `server/milestone_context_handler.rs:28,31` porte un
> **doublon** de `PR_CLOSED_PREFIX` et `MERGED_TRUE_LINE`. Ce vol n'en écrit pas un
> troisième (il importe les promues) ; il ne déduplique pas l'existant non plus —
> c'est un geste sur un handler tiers, étranger au défaut mesuré. Nommé ici pour
> qu'un futur éditeur apprenne que le doublon existe avant d'en écrire un troisième.

### D3 — Un seul geste de retrait, réutilisé

`draft_pr_opened_handler::remove_ready_label(issue, repo, token)` est promu
`pub(crate)` et appelé tel quel : argv
`["issue","edit",&n,"--repo",repo,"--remove-label",READY_LABEL]`, sous
`tokio::time::timeout(60s)`. Le littéral `ready` et la forme argv restent donc à
**un** site pour le serveur, déjà épinglé par les tests de ce module.
`fetch_issue_labels` + `issue_has_ready_label` sont réutilisées de la même façon : on
ne retire que si le label est **lu** présent, pour que la ligne INFO et l'entrée
d'audit disent une chose vraie (un « retiré » émis sur un ticket qui ne portait rien
est un instrument qui ment — mika#2331).

### D4 — Pourquoi « avant le tour LLM » est une propriété du site de câblage

Le handler se câble dans le bloc des handlers structurels de
`server/handlers.rs`, juste avant la fin du bloc (l. 2057), sur le patron exact de
`upstream_close_handler` (quatre arms, `unreachable!()` sur `Handled` /
`Dispatched`). Ce bloc tourne **avant** le pré-filtre
(`super::webhook_prefilter::skip_turn(`, l. 2084) et donc avant le tour. Le handler
s'auto-sélectionne sur son préfixe : son rang dans le bloc est indifférent.

Cette propriété est d'**ordonnancement**, pas de comportement : aucun test de
comportement ne rougit le jour où un refactor déplace le bloc sous le pré-filtre.
D'où le détecteur de D5.

### D5 — Le détecteur : l'ordre du câblage, mesuré sur la source

`crates/mika-agent/tests/eval/test_ready_cleanup_on_merge_3022.rs`, sur le patron de
`tests/eval/test_merge_identity_2248.rs` (`include_str!` + comparaison d'offsets) :

- l'appel `merged_pr_ready_cleanup_handler::try_handle_merged_pr_ready_cleanup(`
  apparaît dans `handlers.rs` à un offset **strictement inférieur** à celui de
  `super::webhook_prefilter::skip_turn(` ;
- les deux ancres existent (sinon le scan rougit au lieu de cesser de regarder) ;
- dans le handler, `plan_ready_cleanup_on_merge(` apparaît avant
  `remove_ready_label(` : la décision précède le geste.

### D6 — Les quatre causes de non-geste, et leur ligne nommée (AC3)

Le seam de décision est **pur** et rend une raison, jamais un booléen :

```rust
pub(crate) enum SkipReason { NotOurEvent, NotMerged, RepoUnrecognized, BranchOffForm }
pub(crate) enum MergedPrCleanup { Skip(SkipReason), Clean { repo: String, issue: u64 } }
pub(crate) fn plan_ready_cleanup_on_merge(text: &str) -> MergedPrCleanup
```

Ordre des portes, et ce que chacune émet (champ structuré `event = "…"`, forme de
`draft_pr_opened_handler.rs:202`) :

| porte | raison | ligne |
|---|---|---|
| `starts_with(PR_CLOSED_PREFIX)` faux | `NotOurEvent` | aucune (ce n'est pas notre événement) |
| `contains(MERGED_TRUE_LINE)` faux | `NotMerged` | `debug!` — contrôle négatif d'AC1 |
| `parse_pr_action_event` rend `None`, ou `action != "closed"` | `RepoUnrecognized` | `warn! event = "merged_pr_ready_cleanup_repo_unrecognized"` |
| `extract_head_branch` ou `issue_number_from_branch` rend `None` | `BranchOffForm` | `warn! event = "merged_pr_ready_cleanup_branch_off_form"` |
| token absent | — | `warn! event = "merged_pr_ready_cleanup_no_token"` |
| `gh` échoue (lecture des labels, ou retrait) | — | `warn! event = "merged_pr_ready_cleanup_gh_failed"`, champ `step = "issue_view" \| "remove_label"` |
| retrait effectif | — | `info! event = "ready_label_cleaned_up_on_merge"` + audit `merged_pr_ready_label_cleaned` |

`NotOurEvent` est volontairement muet : le handler voit **tout** le trafic webhook, et
une ligne par message étrangère noierait les six autres.

### D7 — Pourquoi AC2 tient sans toucher une seule allowlist

Vérifié par lecture des gardes, pas supposé :

- **Test 13** (`ready_label.rs:1244`, `no_ready_label_write_outside_this_module`) :
  prédicat = une ligne contenant `gh_apply_label` ou `add_label_to_issue` **et** le
  jeton `ready`. Ce vol n'appelle ni l'un ni l'autre.
- **D1 de #2995** (`ready_label.rs:1434`) : population
  `est_surface_doutil(rel) = rel.starts_with("tools/") || rel == "skills/builtin_handlers.rs" || rel == "skills/executor.rs"`.
  Un fichier sous `server/` est **hors population**.
- **D2 de #2995** (`ready_label.rs:1736`) : `ARGV_ADD_LABEL_FLAG = "--add-label"`,
  `ARGV_AMBIGUOUS_LABEL_FLAGS = ["\"--label\"", "--label=", "\"-l\""]`.
  `--remove-label` n'est dans **aucune** des deux listes.
- **AC1 de #2995** (`canonical_tokens.rs:7485+`) : porte sur les *prescriptions* dans
  les prompts bundle ; ce vol ne touche aucun prompt.
- Le chemin de retrait ne passe **pas** par `apply_ready` : cet applicateur est la
  porte *gardée par le park* de l'**ajout**. « Retirer, c'est rendre un consentement,
  jamais s'en donner un. »

Aucun littéral nouveau couplé au gateway n'est introduit (tous importés), donc aucune
entrée dans `scripts/canonical-tokens.tsv`.

## Ordre de commit

1. **Promotions** — `upstream_close_handler.rs` (`PR_CLOSED_PREFIX`,
   `MERGED_TRUE_LINE`, `extract_head_branch` en `pub(crate)`, doc nommant le
   précédent mika#2680/#2854 et le doublon de `milestone_context_handler`) ;
   `draft_pr_opened_handler.rs` (`remove_ready_label`, `fetch_issue_labels` en
   `pub(crate)`). Aucun changement de comportement : `cargo test -p mika-agent` vert
   à ce commit.
2. **Le seam pur** — `merged_pr_ready_cleanup_handler.rs` avec
   `plan_ready_cleanup_on_merge` + ses tests inline (dont le test AC1 qui porte le
   contrôle négatif dans le même appel). Vert avant tout câblage.
3. **Le geste et le fail-open** — `try_handle_merged_pr_ready_cleanup`, les six lignes
   nommées, l'entrée d'audit.
4. **Le câblage** — `server/mod.rs` + `server/handlers.rs` (avant l. 2057).
5. **Le détecteur** — `tests/eval/test_ready_cleanup_on_merge_3022.rs` +
   `mod test_ready_cleanup_on_merge_3022;` dans `tests/eval.rs`.

Plancher mergeable : commits 1 à 4. Le commit 5 ne peut pas précéder le 4 (il mesure
son offset).

## Verification Contract

| énoncé | ce qui l'épingle |
|---|---|
| merge + branche en forme ⇒ `Clean { repo, issue }` ; même texte en `Merged: false` ⇒ `Skip(NotMerged)` | **un seul** `#[test]` qui porte les deux appels (AC1) |
| branche hors forme (`main`, `feat/slug`, `(branch: )` absent) ⇒ `Skip(BranchOffForm)` | tests de table sur le seam |
| header non parsable ⇒ `Skip(RepoUnrecognized)` | test de table sur le seam |
| un titre portant `(branch: …)` ne déplace pas la branche | test sur le seam, fixture du header réel (`webhook_dispatch.rs:2215+`) |
| le handler rend toujours `Passthrough { enrichment: None }` | les arms `unreachable!()` du câblage + test sur le chemin `NotOurEvent` |
| le geste précède le pré-filtre et le tour | détecteur d'ordre (D5) |
| les gardes de voie B restent vertes, allowlists inchangées | `cargo test -p mika-agent ready_label`, `cargo test -p mika-agent canonical_tokens`, plus `git diff` vide sur les blocs `*_EXCEPTIONS` / `*_DECLARED` |
| aucune régression de pipeline | `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `scripts/check-byte-slices.sh`, `scripts/verify-pipeline.sh origin/main`, `make verify-bundled-skills` |

Le geste `gh` lui-même n'est pas exécuté en test : `remove_ready_label` est appelée
via `run_gh_subprocess`, sans seam injectable, et son argv est déjà épinglée au site
qui la porte (`draft_pr_opened_handler`, plus `evidence/guards.rs:7503`). Ce vol
n'ajoute pas de double de cette assertion ; il épingle la **décision** et l'**ordre**,
qui sont ce que le défaut a cassé.

## Definition of Done

- `merged_pr_ready_cleanup_handler.rs` livré, câblé dans le bloc structurel de
  `handlers.rs` avant `skip_turn`, déclaré dans `server/mod.rs`.
- Les trois promotions `pub(crate)` faites, aucun parseur recopié, aucun littéral
  nouveau.
- Les six lignes nommées émises, une par cause, plus l'entrée d'audit sur le succès.
- Le détecteur d'ordre livré **armé** et vert, câblé dans `tests/eval.rs`.
- Suite complète verte : `cargo test`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check`, `scripts/check-byte-slices.sh`,
  `scripts/verify-pipeline.sh origin/main`, `make verify-bundled-skills`.
- Aucune entrée nouvelle dans une allowlist de voie B (diff vérifié, pas supposé).
- Corps de PR portant `Code review: delegated to mika-qa (mika#2694)` et la sonde AC4
  déclarée comme suivi opérateur.

## Acceptance criteria

- **AC1.** Un événement `PR closed` avec `Merged: true`, dont la branche suit la forme
  `<type>/<N>/…`, laisse #N sans `ready`. Le geste se fait avant le tour LLM. Le test
  porte, dans le même appel, le contrôle négatif : même événement avec
  `Merged: false`, et le label reste.
- **AC2.** Aucune route d'ajout de `ready` n'est créée. Les gardes de voie B livrées
  pour #2995 restent vertes sans entrée nouvelle dans leurs allowlists.
- **AC3.** Une branche hors de la forme, un dépôt non reconnu ou une erreur `gh`
  laissent passer l'événement sans geste. Chacun de ces cas émet une ligne de log
  nommée, qui dit lequel.
- **AC4.** Sonde : sur les 5 merges suivants d'une PR de la boucle pour un ticket
  gardé ouvert, on compte les retraits de `ready` par un compte humain, et on écrit le
  compte ici. La cible est 0.

### Comment AC4 se tient (zéro ligne de code)

AC4 est une sonde **opérateur**, pas un livrable. Après merge de ce vol, sur chacun
des 5 merges suivants d'une PR de la boucle servant un ticket gardé ouvert :

```
gh api repos/senara-solutions/mika/issues/<N>/timeline --paginate \
  --jq '.[] | select(.event=="unlabeled" and .label.name=="ready") | {actor:.actor.login, at:.created_at}'
```

Le compte à écrire est le nombre de retraits dont `actor.login` n'est ni
`mika-platform-qa` ni le compte applicatif du moteur. Il s'écrit **en commentaire sur
mika#3022**, et le ticket reste ouvert jusque-là. Cible : 0. Un retrait machine
(`ready_label_cleaned_up_on_merge` dans les logs, et l'entrée d'audit
`merged_pr_ready_label_cleaned`) est le chemin sain et n'entre pas dans le compte.

## Fire-Disposition

Ce vol livre **un** détecteur : l'assertion d'ordonnancement de D5
(`tests/eval/test_ready_cleanup_on_merge_3022.rs`), dont le chemin de succès est
« aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist — livrée VIDE, avec son assertion
auto-nettoyante.**

Détail d'implémentation :

- Le détecteur naît avec le code qu'il garde : au moment où il est écrit, la seule
  population possible est le câblage que le commit 4 vient de poser. Il ne peut donc
  pas rougir sur de l'existant, et il est livré **armé** — jamais `#[ignore]`.
- La forme porte malgré tout une allowlist déclarée vide,
  `const WIRING_ORDER_EXCEPTIONS: &[(&str, &str)] = &[]` (chemin, raison + ticket de
  suivi), avec son test frère de vacuité sur le patron de
  `ready_label.rs::mika2995_lallowlist_de_surface_est_vide` : le jour où quelqu'un
  veut y inscrire un site, la vacuité rougit et force la décision à passer par une
  revue plutôt que par un ajout silencieux.
- **Assertion auto-nettoyante** : les deux ancres (`try_handle_merged_pr_ready_cleanup(`
  et `super::webhook_prefilter::skip_turn(`) sont cherchées avec `.expect(…)`. Si
  l'une est renommée ou disparaît, le scan **rougit** au lieu de cesser de regarder —
  c'est la direction qui compte (mika#2205 : un recensement qui survit à son site se
  lit comme une couverture).
- **Si le détecteur rougit malgré tout à l'implémentation**, c'est-à-dire si le geste
  se retrouve câblé après `skip_turn` : bascule en **(c) halte-et-remontée**. Une
  exception nommée serait là un retrait qui s'exécute *après* le tour LLM — donc AC1
  faux derrière une ligne verte, et le geste redevenu « geste de mika-dev » que la
  contrainte de doctrine refuse. L'implémenteur s'arrête, nomme le site, et surface
  dans le corps de PR sous un titre `Fire-Disposition: le geste est tombé après le
  pré-filtre`.

Les tests de table du seam (D6) ne sont **pas** des détecteurs : leur chemin de succès
est « le comportement est celui attendu », pas « aucune violation trouvée ».

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/merged_pr_ready_cleanup_handler.rs` (production) | 155 |
| ses tests inline (`#[cfg(test)] mod tests`) | 140 |
| `crates/mika-agent/src/server/upstream_close_handler.rs` (3 promotions + doc) | 10 |
| `crates/mika-agent/src/server/draft_pr_opened_handler.rs` (2 promotions + doc) | 8 |
| `crates/mika-agent/src/server/handlers.rs` (câblage + commentaire) | 22 |
| `crates/mika-agent/src/server/mod.rs` (déclaration du module) | 1 |
| `crates/mika-agent/tests/eval/test_ready_cleanup_on_merge_3022.rs` + `tests/eval.rs` | 55 |

Total estimé : 391 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** Le patron mesuré par le ticket
(`draft_pr_opened_handler.rs`, 462 lignes dont 314 avant `#[cfg(test)]`) est un
majorant, pas une cible : ce vol n'en réécrit pas la moitié basse. Il n'écrit **aucun
parseur** — cinq lecteurs canoniques sont appelés (D2) là où le patron a dû écrire
`parse_pr_opened`, `parse_pr_view_json` et `parse_issue_labels_json` (≈ 90 lignes de
production et 10 tests à lui seul) — et **aucun appel `gh`** : les trois fonctions
`fetch_issue_labels`, `issue_has_ready_label` et `remove_ready_label` sont réutilisées
par promotion (≈ 60 lignes de production économisées). Ce qui reste est le seam de
décision, les sept portes de log et le câblage. 391 laisse 38 lignes de marge sous la
borne ; si l'implémentation dépasse, la variable d'ajustement est le volume de tests
de table du seam (D6), pas le détecteur ni une porte de log.

## Hors vol

- Nettoyer les 144 tickets fermés qui portent encore `ready` (constat du relevé).
- Le retrait à l'**ouverture** de la PR (le patron draft couvre déjà son cas).
- Dédupliquer `PR_CLOSED_PREFIX` / `MERGED_TRUE_LINE` de
  `server/milestone_context_handler.rs:28,31` vers les constantes promues : dette
  préexistante, nommée en D2, sur un handler étranger au défaut mesuré.
- Unifier les quatre sites de retrait de `ready` du moteur (`auto_pull.rs:4075`,
  `:4252`, `:6012` passent le littéral `"ready"` nu à `gh_remove_label` ;
  `draft_pr_opened_handler.rs:263` passe par `remove_ready_label`) derrière un lecteur
  unique. C'est une doctrine de site unique à quatre violations préexistantes : elle
  ne se livre pas en travers d'un correctif de `p1`.
