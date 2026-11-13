# mika#2822 — Une PR ouverte compte comme PR ouverte, quelle que soit la forme de son lien au ticket

> **Suivi de mika#1517** (le filtre `has_open_pr` lui-même), conservé entier. Ce plan
> n'enlève rien à son périmètre : il élargit la **source** de l'ensemble qu'il consomme.

## Le défaut, mesuré

Le 2026-10-19 entre 00:00Z et 06:00Z, 8 vols d'implémentation ont fini en
`PIPELINE FAILURE: … HEAD unchanged … Zero new commits produced`, 11 à 27 tours et
1,0 $ à 2,6 $ chacun. Deux tickets en portent 7 sur 8 :

| ticket | PR ouverte pendant le vol | lien PR → ticket | vols à vide |
|---|---|---|---|
| #2680 | #2812 (vol 4) | `Refs #2680` seul | 6 |
| #1381 | #2813 (vol 1/2) | aucun — seul le titre cite `mika#1381` | 1 |

Chaque re-drive repartait de la tête de la PR (`pre == post == b047d0d5`, puis
`deaf2721`), ne trouvait rien à faire et s'arrêtait sans commit. Le rail était occupé.
Le retrait de `ready` a été fait à la main, à chaque garde.

La cause est à une ligne : `gh_list_open_pr_closing_issues`
(`crates/mika-agent/src/auto_pull.rs:2589`) demande
`--json number,closingIssuesReferences` et ne lit que ce champ-là. Or
`closingIssuesReferences` n'est peuplé que par un mot-clé de fermeture (`Closes #N`).
**Une PR de vol partiel ne peut pas en porter**, par construction : le ticket doit
rester ouvert pour le vol suivant, donc son corps dit `Refs #N`. Pour le filtre, ces
tickets sont « sans PR » — et tant qu'ils gardent `ready`, ils sont re-dispatchés.

Le vol à vide de #2699 (`0249167e`) précède l'ouverture de sa PR : le ticket le met
hors périmètre, ce plan ne le traite pas.

## La décision du plan : deux sources, et la troisième est refusée

L'ensemble devient l'**union** de deux lectures de la **même** réponse `gh pr list` —
aucun appel réseau ajouté, le champ `headRefName` voyage dans la requête existante :

1. `closingIssuesReferences` — l'existant, inchangé ;
2. `headRefName` — les branches de pilote suivent `<type>/<N>/<slug>`
   (`scripts/derive-branch-name`), donc le deuxième segment donne `N`.

**La troisième source (`Refs #N` du corps) est refusée, pas différée.** Le ticket la dit
« moins sûre » ; la raison est dirimante dans ce dépôt précisément. Les corps de PR y
citent des tickets en **doctrine** et non en périmètre — « la classe mika#2158 », « motif
mika#2331 », « suivi de mika#1517 » — donc un scan de `#N` sur le corps rendrait
positifs des tickets qu'aucune PR ne traite. Et l'asymétrie des coûts est inverse de
celle du reste du filtre : un faux négatif coûte **un vol à vide** (11-27 tours, ≤ 2,6 $,
réparable au tick suivant), un faux positif **retire un ticket du dispatch tant que la PR
citante reste ouverte**, en silence, sans que rien le dise. Le corps n'est d'ailleurs pas
dans le `--json` aujourd'hui : le refuser garde aussi la requête à sa taille.

### Pourquoi la grammaire de branche n'est pas réécrite ici

`crate::worktree_reaper::issue_number_from_branch` est le lecteur **unique** de cette
grammaire — « le deuxième segment, et rien d'autre », premier segment non vide exigé,
`None` sur non conforme et **jamais** de numéro inventé (B3.2, mika#2482/#2619). Il est
déjà appelé depuis `webhook_dispatch.rs`, dont le doc-comment dit en propres termes
pourquoi : *« Écrire une troisième extraction … serait la classe mika#2158 à son
troisième tour »*. Ce plan l'appelle. Il n'en recopie pas un octet, et il n'y touche pas.

Conséquence directe sur **AC3** : `dependabot/cargo/serde-1.0.2` a `cargo` en deuxième
segment, non numérique ⇒ `None` ⇒ l'ensemble n'est pas alimenté. L'AC est satisfait par
la réutilisation, pas par une clause spéciale `dependabot`.

## Ce que le changement coûte, nommé plutôt que découvert

**Une PR ouverte sur `feat/N/slug` exclut N du dispatch, même si elle ne livre pas N.**
C'est l'intention de mika#1517, pas un effet de bord : la population concrète est le
brouillon de sauvetage post-vol (`wip()` + draft PR, mika#1282), et sur cette même
population `draft_pr_opened_handler` retire **déjà** `ready`. Les deux gestes disent la
même chose — du travail est en vol sur cette branche.

**L'exclusion se lève d'elle-même.** La requête est `--state open` : PR mergée ou fermée
⇒ le ticket redevient éligible au tick suivant. Rien à retirer, aucun label à connaître.

**`FILTER_OPEN_PR` garde sa valeur `"open_pr_closing"`.** C'est du **format de fil** :
la chaîne est persistée dans le ledger d'exclusion et épinglée par un `assert_eq!`
(`auto_pull.rs:8733`). La renommer rendrait non sommables les agrégats opérateur d'avant
et d'après — une même population sous deux noms successifs ne s'additionne pas, miroir
exact de mika#2205. Le plan ajoute donc une phrase au doc-comment de la constante disant
que sa valeur est historique et que le filtre est désormais plus large que son nom ; il
ne change pas l'octet.

## La forme du code : un parseur pur, sur le motif déjà en place

Le parsing sort de la fonction async — exactement le découpage de
`parse_compare_payload` (mika#2123 U1), *« split from the subprocess call so the whole
decision path is testable against frozen payloads with no network »*. C'est ce qui rend
**AC5** atteignable : aucun test ne peut aujourd'hui atteindre ce parsing, puisqu'il vit
dans le corps d'une fonction qui spawne `gh`.

```rust
/// Ce que la réponse `gh pr list` apprend sur les tickets en vol (mika#2822).
pub(crate) struct OpenPrLinkage {
    /// L'ensemble que les trois phases consomment : union des deux sources.
    pub numbers: HashSet<u64>,
    pub prs: usize,
    /// PR ayant au moins un `closingIssuesReferences`.
    pub via_closing: usize,
    /// PR que **seule** la branche relie à un ticket — la population des 8 vols.
    pub via_branch_only: usize,
    /// PR dont la branche ne suit pas `<type>/<N>/<slug>`.
    pub nonconforming_branches: usize,
}

pub(crate) fn parse_open_pr_issue_numbers(stdout: &str) -> Result<OpenPrLinkage>
```

**Deux niveaux d'échec, et leur asymétrie est délibérée** — c'est celle que
`parse_compare_payload` tient déjà pour son champ `files` optionnel :

- **JSON global illisible** ⇒ `Err`. L'appelant garde son `match` fail-open à
  l'identique, donc l'ensemble vide, donc le comportement d'avant le correctif. **AC4.**
- **Une entrée malformée** (pas de `headRefName`, type inattendu) ⇒ cette PR ne
  contribue simplement pas par la branche ; `closingIssuesReferences` continue, les 99
  autres PR gardent leurs exclusions. Faire échouer le lot sur une entrée ferait perdre
  tout le filtre pour une PR — le fail-open global est trop grossier pour servir de
  traitement d'erreur par entrée.

## Observabilité : la question « le nouvel axe sert-il à quelque chose ? » doit avoir une réponse

Le census de pool émet déjà `open_pr = census.open_pr` (`auto_pull.rs:3755`), mais ce
nombre est muet sur la **provenance** — avant comme après ce vol, il dirait la même
chose. Une ligne INFO est ajoutée au site du fetch, une par tick, sur **toutes** les
branches y compris celle où `via_branch_only == 0` :

`auto_pull_open_pr_set_resolved` — `prs`, `via_closing`, `via_branch_only`,
`nonconforming_branches`, `total` (`numbers.len()`).

Le champ qui porte le ticket est `via_branch_only` : il compte les PR que **seule** la
branche relie, c'est-à-dire exactement #2812 et #2813 du tableau ci-dessus. Son absence
de la ligne signifie un binaire antérieur à ce vol (classe mika#2340), jamais « aucune
PR dans ce cas ». Comme toute sonde de ce dépôt, elle n'est vive qu'après `make deploy`.

**Sonde post-déploiement :** `grep auto_pull_open_pr_set_resolved` dans le journal de
mika-spirit après un tick ⇒ `via_branch_only` ≥ 1 tant qu'un vol partiel est en cours.
Contre-épreuve : le ticket ainsi exclu apparaît sous `reason=open_pr_closing` dans
l'agrégat du ledger d'exclusion du même tick.

## Plan d'implémentation

### U1 — `crates/mika-agent/src/auto_pull.rs` : le parseur et son type

- Ajouter `OpenPrLinkage` + `parse_open_pr_issue_numbers` (forme ci-dessus), placés
  juste avant la fonction async, section « Selection logic » inchangée.
- La branche est lue par `crate::worktree_reaper::issue_number_from_branch` — appel, pas
  copie.
- Compter `via_closing` / `via_branch_only` **par PR**, pas par numéro : une PR dont les
  deux sources donnent le même `N` compte en `via_closing` (la source historique
  suffisait), ce qui garde `via_branch_only` exactement égal à la population que ce vol
  ajoute.

### U2 — la fonction async : un champ de plus, un nom plus vrai

- `--json number,closingIssuesReferences,headRefName` (`--limit 100` inchangé,
  `GhSubject::Repo` inchangé, `run_gh` inchangé — la couture mika#2713 reste la seule
  porte des spawns `gh`).
- Renommer `gh_list_open_pr_closing_issues` → `gh_list_open_pr_issue_numbers` : le nom
  actuel décrit la source, pas la question. Un seul appelant de production
  (`auto_pull.rs:3521`), aucun appelant de test. Mettre à jour la citation du nom dans
  le doc-comment de module de `ready_label.rs:76`.
- Le corps délègue à `parse_open_pr_issue_numbers`, émet la ligne INFO et rend
  `linkage.numbers`.

### U3 — le site d'appel : AC4 à l'octet

Le `match` de `auto_pull.rs:3521` ne change pas de forme : bras `Ok` → l'ensemble, bras
`Err` → `warn!` + `HashSet::new()`, **commentaire conservé**. Une seule phrase lui est
ajoutée, nommant les deux sources désormais unies, pour qu'un futur lecteur sache ce que
le fail-open abandonne au juste.

### U4 — doc-comment de `FILTER_OPEN_PR`

Une phrase : valeur historique, format de fil, filtre plus large que son nom depuis
mika#2822. Aucun changement de valeur.

### U5 — les tests (inline `mod tests` d'`auto_pull.rs`, module du ticket)

Un unique JSON figé, reproduisant les deux PR réelles du 2026-10-19 plus les contrôles :

| entrée du JSON figé | ce qu'elle épingle |
|---|---|
| `#2812`, `feat/2680/terme-de-relache`, `closingIssuesReferences: []` | **AC1** — 2680 entre dans l'ensemble |
| `#2813`, `feat/1381/…`, champ absent | **AC2** — 1381 entre dans l'ensemble |
| `#1`, `dependabot/cargo/serde-1.0.2` | **AC3** — n'ajoute rien, et compte en `nonconforming_branches` |
| `#2`, `fix/606/x`, `closingIssuesReferences: [{number: 606}]` | non-régression mika#1517, et `via_closing` (jamais `via_branch_only`) |
| `#3`, `main`, `closingIssuesReferences: [{number: 9}]` | une branche non conforme ne **retire** rien : 9 entre par la source historique |

Six tests, nommés `mika2822_*` :

1. `mika2822_une_pr_en_refs_seul_compte_par_sa_branche` — AC1.
2. `mika2822_une_pr_sans_aucune_reference_compte_par_sa_branche` — AC2.
3. `mika2822_une_branche_hors_convention_najoute_rien` — AC3.
4. `mika2822_la_source_historique_est_intacte` — non-régression des deux dernières
   lignes du tableau.
5. `mika2822_un_json_illisible_rend_une_erreur` — AC4, la moitié testable sans réseau :
   le fail-open du site d'appel reste **atteignable**, et une erreur ne dégénère pas en
   ensemble partiel (qui, lui, serait pris pour une vérité).
6. `mika2822_la_provenance_est_comptee_par_pr` — les quatre compteurs sur le JSON
   complet, dont `via_branch_only == 2` : c'est l'assertion qui lie le code aux 8 vols
   du constat, et la seule qui rougirait si un futur refactor recomptait par numéro.

## Fire-Disposition

Ce plan livre un détecteur : les six tests de U5, dont le chemin de succès est « aucune
violation trouvée ».

**Option (a) — exception nommée en allowlist — avec une allowlist VIDE, et c'est
mesuré, pas supposé.** Les six tests portent sur une fonction *créée par ce vol*, et le
correctif atterrit dans le même commit qu'eux : il n'existe aucune violation préexistante
à excepter. La vérification est faite : `grep -rn gh_list_open_pr_closing_issues
crates/ tests/` ne rend qu'**un** appelant de production (`auto_pull.rs:3521`) et
**zéro** appelant de test — tous les tests existants du filtre construisent
`open_pr_issue_numbers` comme un `HashSet` littéral (`auto_pull.rs:6846`, `6859`, et les
~15 sites de `has_open_pr`) et ne traversent donc pas le parseur. Aucun d'eux ne peut
rougir, et aucune entrée d'allowlist ne serait écrite, donc aucune ne pourrait devenir
stale.

Les tests atterrissent **armés** (ni `#[ignore]` ni `cfg(skip)`) : désarmer un détecteur
dont la population de violations est vide à l'atterrissage n'achèterait rien et
coûterait un suivi.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/auto_pull.rs` — `OpenPrLinkage` + `parse_open_pr_issue_numbers` (U1) | 75 |
| `crates/mika-agent/src/auto_pull.rs` — fonction async, champ JSON, renommage, sonde INFO (U2) | 30 |
| `crates/mika-agent/src/auto_pull.rs` — site d'appel + `FILTER_OPEN_PR` (U3, U4) | 10 |
| `crates/mika-agent/src/ready_label.rs` — citation du nom dans le doc-comment de module | 2 |
| tests (`mod tests` inline d'`auto_pull.rs`, JSON figé + 6 tests, U5) | 130 |

Total estimé : 247 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 247 estimées contre 429 : 58 % de la borne
par vol, et un quart du seuil par PR de 1000. Le périmètre est un fichier et demi, sans
migration, sans schéma, sans endpoint, sans nouvel appel réseau. Le ratio
production/tests (117/130) est celui d'un correctif ciblé, pas d'un sous-système : les
deux tiers du volume de production sont des doc-comments, que ce dépôt exige denses mais
qui ne coûtent pas de tours de mise au point. Aucun livrable n'a de dépendance sur un
autre dépôt ni sur un déploiement.

## Definition of Done

- `cargo test -p mika-agent auto_pull` vert, dont les six tests `mika2822_*`.
- `cargo clippy` et `cargo fmt --check` propres.
- `scripts/verify-pipeline.sh origin/main` vert (dont U2 : la section
  `## Acceptance criteria` de ce plan).
- `make verify-bundled-skills` vert (aucun bundle touché, non-régression).
- La suite complète `cargo test` verte — en particulier les gardes de scan d'`auto_pull`
  (mika#2361) et la carte des spawns `gh` (mika#2713), que ce vol ne doit pas déplacer.
- Le corps de la PR nomme le coût accepté de la § *Ce que le changement coûte* et la
  sonde post-déploiement.

## Acceptance criteria

- **AC1** : une PR ouverte dont la branche est `feat/2680/…` et qui ne porte que
  `Refs #2680` met 2680 dans l'ensemble « a une PR ouverte ».
- **AC2** : une PR sans aucune référence, sur la branche `feat/1381/…`, met 1381 dans
  cet ensemble.
- **AC3** : une branche hors convention (`dependabot/…`) n'ajoute rien.
- **AC4** : le fail-open existant en cas d'erreur `gh` est conservé, avec son
  commentaire.
- **AC5** : un test dans le module couvre AC1 à AC3 sur un JSON `gh pr list` figé.

## Hors périmètre

- **Le vol à vide de #2699** (`0249167e`), antérieur à l'ouverture de sa PR : le ticket
  le déclare hors périmètre et ne l'instruit pas. Aucune des deux sources de ce plan ne
  l'aurait vu — une branche poussée sans PR n'apparaît pas dans `gh pr list`.
- **La troisième source `Refs #N`** : refusée, avec sa raison, § *La décision du plan*.
  Ce n'est pas un report.
- **Le renommage de la valeur de fil `"open_pr_closing"`** : refusé, avec sa raison,
  § *Ce que le changement coûte*.
- **Le retrait automatique de `ready`** sur un ticket à PR ouverte : le filtre empêche le
  dispatch, il ne touche pas aux labels. C'est le périmètre de
  `draft_pr_opened_handler` et de `abandon_stuck_ready`, inchangés.
