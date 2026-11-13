# mika#3001 — la porte juge un callout périmé sur le merge de son plan, pas sur l'absence de la branche

**Ticket :** senara-solutions/mika#3001
**Labels :** `bug`, `p1-important`, `ready`, `dispatch:loop`, `loop-substrate`
**Type :** fix
**Vols :** un seul (voir § 8)

---

## 1. Où en est le ticket, et pourquoi ce vol-là

### 1.1 La cause, vérifiée dans le code

Le ticket la donnait pour « probable, non vérifiée ». Elle est exacte, et elle
tient en une ligne de `promotion_gate_allows`
(`crates/mika-agent/src/auto_pull.rs:3655-3662`) :

```rust
if let (StalenessMeasurement::BranchDeletedByMerge { pr }, Some(b)) =
    (&measurement, branch.as_deref())
{
    let rewrite = rewrite_dead_callout(…).await;
}
```

La réécriture est gardée par **une seule** variante de mesure, et cette variante
n'est atteignable que par un chemin unique :

1. `gh_compare_branch` (`:3148`) appelle `gh api repos/…/compare/main...<branch>`.
   Un `HTTP 404` — la branche n'est pas sur `origin` — rend
   `StalenessMeasurement::BranchAbsent` ; **tout** autre succès rend
   `Measured(BranchStaleness { … })`.
2. `promotion_gate_allows` (`:3634-3640`) ne pose la sonde
   `gh_merged_pr_for_branch` que sur l'arm `BranchAbsent`, et seul
   `classify_absent_branch` (`:1199`) produit `BranchDeletedByMerge`.

Donc : **branche présente sur `origin` ⇒ la réécriture n'est jamais tentée.**
Un repush fait répondre `compare` avec un `200`, la mesure devient `Measured`, et
le `if let` ne mord pas. C'est la mécanique exacte du 2026-10-31 — le groom du
vol 2e a repoussé `feat/2958/manager-phase-2-c-vol-2-le-manager` à 16:36Z
(« pushing as first-push », mika#2626), douze minutes après le merge de #2992 à
16:24:32Z, et le callout du vol 2d a survécu jusqu'à sa correction à la main le
2026-11-01.

### 1.2 Ce que le défaut coûte, et pourquoi p1

La chaîne est celle que le ticket décrit, et son dernier maillon est une
provenance de dispatch :

- le groom d18d6ef1 (16:52Z) a lu un callout nommant le plan du vol 2d, l'a
  trouvé « déjà groomé » et a rendu `auto_skipped` / `already_groomed` ;
- depuis mika#2981, la **preuve de groom est liée au plan nommé par le callout**.
  Une preuve convergée existait donc pour un plan déjà réalisé, et un dispatch
  posé sur ce callout aurait été accepté sur un plan livré.

Le défaut ne produit pas une erreur : il produit un **oui** sur une question à
laquelle la réponse était non.

### 1.3 La mesure qui dessine le correctif

Les six PR de la branche de #2958, relues sur la forge le 2026-11-01
(`gh pr list --head feat/2958/manager-phase-2-c-vol-2-le-manager --state all
--json number,mergedAt,files`) :

| PR | mergedAt | plan porté |
|---|---|---|
| #3005 | 2026-11-01T10:03:58Z | `…-2026-10-31-006-…-vol-2e-plan.md` |
| #2992 | 2026-10-31T16:24:32Z | `…-2026-10-31-005-…-vol-2d-plan.md` |
| #2988 | 2026-10-31T12:47:21Z | `…-2026-10-31-003-…-vol-2c-plan.md` |
| #2975 | 2026-10-30T11:04:06Z | `…-2026-10-30-002-…-vol-2b-plan.md` |
| #2970 | 2026-10-30T07:02:41Z | `…-2026-10-29-005-…-vol-2-plan.md` |
| #2960 | 2026-10-29T14:02:54Z | `…-2026-10-29-003-…-plan.md` |

**Une PR par plan, six vols, une seule branche.** Cette table dit deux choses
que le dessin du § 2 consomme directement :

- la liaison plan → PR est **exacte** sur la population réelle : chaque plan est
  dans les fichiers d'une PR et d'une seule ;
- « la PR mergée la plus récente » ne suffirait pas. Au moment du groom du vol 2e
  la liste portait cinq PR mergées ; le `mergedAt` maximal donnait #2992, qui
  portait bien le plan 2d, **par coïncidence de séquence**. Le jour où deux vols
  mergent dans le désordre, ce raccourci nomme la mauvaise PR. AC1 demande la PR
  « qui a porté son plan » : la lire dans les fichiers de la PR la prouve, la
  lire dans l'horodatage la suppose.

### 1.4 Ce qui est déjà payé, et ce qui coûterait un appel

Le correctif ne doit pas ajouter un appel réseau à chaque pull — c'est
précisément le résidu que mika#2839 existait pour fermer (`plan_callout.rs:290`,
« la sonde `gh pr list --head` repayée à *chaque* pull »). Deux faits de l'arbre
rendent cela possible :

- `BranchStaleness` porte déjà **`changed_files: Option<Vec<String>>`**
  (`:739-745`), lu par `parse_compare_payload` depuis la clé `files` du
  `compare`. La question « le plan du callout est-il encore un ajout de cette
  branche à `main` ? » est donc **gratuite** : elle se lit sur une charge déjà
  reçue ;
- `plan_callout::PlanCallout` porte déjà **`normalized`** (`:185`), le chemin sans
  préfixe de dépôt — exactement la forme des `filename` du `compare` et des
  `files[].path` d'une PR. Aucune normalisation à écrire.

La sonde `gh pr list` n'est donc posée que sur la population où `main` a déjà
absorbé le plan du callout : rare par construction, et **auto-tarissante** —
après réécriture le run de callout n'existe plus, donc `extract_branch_name`
rend `None`, donc la porte ne mesure plus rien sur ce ticket.

---

## 2. Le dessin

Le discriminant change de nature. Aujourd'hui : *« la branche du callout est-elle
absente d'`origin` ? »*. Après ce vol : *« la PR qui a porté le plan du callout
est-elle mergée ? »* — ce qui rend l'état de la branche **non pertinent**, et
c'est la lettre d'AC1.

Deux chemins de lecture y mènent, et **les deux nomment la PR**.

### 2.1 Chemin A — inchangé : la branche a été supprimée par le merge

`StalenessMeasurement::BranchDeletedByMerge { pr }`. La sonde de mika#2837 a déjà
établi que le merge de cette PR a supprimé la branche ; la branche n'a eu aucune
autre vie depuis, donc la PR nommée est celle qui a porté le plan. Rien à
changer : c'est le cas 3 d'AC2, et les quatre tests mika#2839 existants sont son
contrôle de non-régression.

### 2.2 Chemin B — nouveau : la branche vit, mais `main` a absorbé le plan

Trois lectures en séquence, dont deux gratuites.

**Lecture 1 (gratuite) — le plan du callout est-il un ajout de la branche ?**
Fonction pure, sur la `BranchStaleness` déjà reçue :

```rust
/// Le plan nommé par le callout est-il encore un ajout de cette branche à
/// `main` (mika#3001) ?
enum PlanPresence {
    /// La branche l'ajoute : le callout est frais, il n'y a rien à faire.
    OnBranch,
    /// Elle ne l'ajoute pas : `main` l'a absorbé, ou il n'existe nulle part.
    NotOnBranch,
    /// La liste n'autorise aucune conclusion d'absence.
    Unreadable,
}

fn plan_presence(staleness: &BranchStaleness, plan: &str) -> PlanPresence
```

Les quatre règles, dans cet ordre :

1. `ahead_by == 0` ⇒ `NotOnBranch`. Le fait est établi par le **compte**, pas par
   la liste : une branche qui n'ajoute rien à `main` n'ajoute pas ce plan-là. Cette
   règle est nécessaire et pas cosmétique — un first-push qui recrée la branche à
   `main` est exactement le cas du 2026-10-31 à 16:36Z, et GitHub omet la clé
   `files` sur un `compare` sans diff, ce qui ferait tomber la règle 2 en
   `Unreadable` sur le cas mesuré ;
2. `changed_files == Some(f)` et `f` contient `plan` ⇒ `OnBranch` ;
3. `changed_files == Some(f)`, `plan` absent, `f.len() < GITHUB_COMPARE_FILES_CAP`
   ⇒ `NotOnBranch` ;
4. sinon — `None`, ou liste au cap ⇒ `Unreadable`.

La règle 4 est la doctrine D5 du module appliquée telle quelle : « une absence
ne peut pas se lire sur une liste que GitHub a pu couper » (`:888`). Elle
réemploie la constante existante `GITHUB_COMPARE_FILES_CAP` (`:366`), elle n'en
crée pas une.

`OnBranch` et `Unreadable` sortent en silence : ce ne sont pas des abandons de
réécriture, ce sont des non-candidatures, et une ligne d'audit par pull sur le cas
nominal transformerait la sonde en bruit. `Unreadable` est néanmoins journalisé en
`debug!` avec son `issue` — la population existe, elle doit être trouvable.

**Lecture 2 (un appel, sur la seule population candidate) — quelle PR mergée
porte ce plan ?** `gh pr list --head <branch> --state all` est déjà la forme de la
sonde existante (`:3205`) ; ce vol lui ajoute `files` à son `--json` et un
paramètre au parseur :

```rust
pub fn parse_merged_pr_for_branch(
    stdout: &str,
    branch: &str,
    carrying: Option<&str>,   // ← mika#3001
) -> Result<Option<MergedPrEvidence>>
```

Un **paramètre** et non une seconde fonction, parce que les trois conditions que
le parseur applique déjà — `mergedAt` non nul, `headRefName == branch`,
`baseRefName == "main"` — sont les mêmes ici, et un second parseur les
dupliquerait dans un fichier qui tient explicitement à n'avoir qu'un site de
décision par question. `carrying: Some(plan)` ajoute une quatrième condition : la
PR doit porter `plan` dans ses `files[].path`. Les appelants du chemin A passent
`None` et ne changent pas de comportement.

**C'est `files[].path`, pas `filename`.** Les deux API ne nomment pas le champ
pareil — le `compare` rend `filename`, `gh pr list --json files` rend `path`
(vérifié sur la charge réelle de #2992). Le parseur lit `path` ; le test de
fixture du § 2.3 est ce qui empêche de le confondre avec son voisin.

**Le `mergedAt` maximal reste le tri, mais il ne décide plus seul.** Sur la table
du § 1.3 le filtre par fichiers laisse exactement une PR ; le tri ne sert donc que
de départage si deux PR mergées portaient le même chemin de plan — ce que la
population n'offre pas, et qui ne mérite pas une erreur.

**Troncature, et une fusion assumée.** `gh pr list --json files` est servi par
GraphQL et plafonne à 100 fichiers par PR ; j'introduis `GH_PR_LIST_FILES_CAP`
pour le nommer. La dissymétrie joue dans le bon sens : une **présence** trouvée
sur une liste coupée reste une présence, et seule une **absence** serait suspecte.
`Ok(None)` recouvre donc deux populations — « aucune PR mergée visible ne porte ce
plan » et « une liste était coupée » — et je ne les sépare pas, contrairement à ce
que mika#2205 demande d'habitude. La raison est que les deux appellent le **même**
geste et que ce geste est l'état d'avant ce vol : le callout n'est pas réécrit,
exactement comme aujourd'hui. Aucune des deux ne peut faire réécrire à tort, qui
est le seul sens dangereux ici.

**Lecture 3 — la relecture R9 et l'écriture**, inchangées (`:3482-3547`) : relire
le corps frais, vérifier qu'il nomme toujours la même branche *et* le même plan,
transformer, écrire. Le comparer-puis-écrire de mika#2839 R9 couvre la course avec
`_write_canonical_callout` sans modification.

### 2.3 La cause de péremption devient un type, et la prose la suit

`rewrite_callout_as_prior_plan` écrit aujourd'hui une phrase qui serait **fausse**
sur le chemin B : « la branche `X` qui le portait a été supprimée par le merge de
`#N` » — sur le chemin B la branche existe. La prose devient fonction de la cause :

```rust
/// Pourquoi le callout d'un vol est périmé — et par quel merge (mika#3001).
///
/// Les deux variantes nomment une PR **mergée**, ce qui est le discriminant
/// qu'AC1 demande. Ce qu'elles séparent est l'état de la branche, qui n'est plus
/// ce qui décide mais reste ce que la prose doit dire sans mentir.
pub enum PriorPlanCause {
    /// Le merge de cette PR a supprimé la branche (mika#2839, chemin A).
    BranchDeletedByMerge { pr: u64 },
    /// Le merge de cette PR a mis le plan sur `main` ; la branche a été
    /// repoussée depuis et porte autre chose (mika#3001, chemin B).
    PlanMergedBranchReused { pr: u64 },
}

pub fn rewrite_callout_as_prior_plan(
    body: &str,
    branch: &str,
    cause: PriorPlanCause,
) -> Option<String>
```

Prose du chemin A — **inchangée à l'octet**, parce que trois corps de tickets
réels la portent déjà et sont le corpus figé de `plan_reading` (`:786`, `:795`) :

> `<plan>` — la branche `<branch>` qui le portait a été supprimée par le merge de
> #`<pr>` dans `main`. Callout réécrit par la porte d'auto-pull (mika#2839).

Prose du chemin B :

> `<plan>` — ce plan a été mergé par #`<pr>` dans `main` ; la branche `<branch>`
> a été repoussée depuis et porte un autre vol. Callout réécrit par la porte
> d'auto-pull (mika#3001).

Les deux partent de `PRIOR_PLAN_PREFIX` (`:146`), **sans l'épeler**. C'est ce qui
garde le contrat de mika#2952 : `plan_reading` relit la ligne et rend
`PlanReading::PriorOnly`, et le test d'aller-retour
`mika2952_la_ligne_ecrite_par_la_porte_est_relue_par_le_lecteur` vaut alors pour
les deux variantes. Le vol l'étend à la variante B plutôt que d'en écrire un
second.

**Le run entier reste remplacé**, pas la seule ligne `Plan` : le raisonnement de
`plan_callout.rs:285-292` tient mot pour mot sur le chemin B — laisser la ligne
`Branch` ferait repayer la mesure à chaque pull, et l'idempotence structurelle
(agir exige une ligne `> - **Plan:**`, qui n'existe plus après) est ce qui rend un
second passage inoffensif sans marqueur.

### 2.4 L'orchestration, et où elle se branche

Une fonction async qui ne contient **que** la séquence, chacune de ses décisions
étant déléguée à une fonction pure testée :

```rust
/// La PR dont le merge rend ce callout périmé, ou rien (mika#3001).
async fn stale_callout_cause(
    github_token: &str,
    issue_number: u64,
    measurement: &StalenessMeasurement,
    branch: &str,
    plan: &str,            // la forme `normalized`
) -> Option<PriorPlanCause>
```

- `BranchDeletedByMerge { pr }` ⇒ `Some(BranchDeletedByMerge { pr })` ;
- `Measured(s)` ⇒ `plan_presence(s, plan)` ; sur `NotOnBranch` seulement, poser la
  sonde et rendre `PlanMergedBranchReused { pr }` si une PR mergée porte le plan ;
- toute autre mesure (`NoBranchCallout`, `Unavailable`, `BranchAbsent`) ⇒ `None`.

Et `promotion_gate_allows` remplace son `if let` à une variante par un appel
**inconditionnel** :

```rust
if let (Some(b), Some(callout)) = (branch.as_deref(), &callout_plan) {
    if let Some(cause) =
        stale_callout_cause(github_token, issue.number, &measurement, b, &callout.normalized).await
    {
        if let Err(e) = rewrite_stale_callout(…, cause, …).await { warn!(…) }
    }
}
```

Le `callout_plan` est obtenu par un `plan_callout(&issue.body, FenceHandling::Strip)`
au même endroit où `extract_branch_name` est déjà appelé — la même lecture du
corps, une fois, et `extract_plan_path` (qui jette `normalized`) n'est pas
réemployée ici.

Deux invariants du site de mika#2839 sont conservés mot pour mot :

- **la réécriture ne peut pas coûter le verdict.** L'appel reste placé après
  l'audit `auto_pull_staleness_measured` et avant le `match decision`, et
  `rewrite_stale_callout` continue de rendre `Ok(())` sur tous ses écarts ;
- **`cause` garde un vocabulaire clos** (`body_changed`, `body_unreadable`,
  `write_refused`) : le chemin B n'en ajoute aucune, parce que ses non-conclusions
  ne sont pas des abandons de réécriture mais des non-candidatures, et elles
  sortent par `plan_presence`.

Ce que la ligne d'audit nominale gagne, en revanche, est **un champ** :
`"staleness":"branch_deleted_by_merge"` ou `"plan_merged_branch_reused"`. C'est
par lui qu'un opérateur compte la population du chemin B, et il est gratuit — le
JSON est déjà écrit à cet endroit.

### 2.5 Ce que ce vol ne fait pas

- **Il ne touche pas `classify_absent_branch` ni le verdict de promotion.** La
  mesure de mika#2837 garde son propre discriminant, qui est juste pour sa propre
  question (« le plan est sur `main`, donc pas de gate »).
- **Il ne relie pas un plan à une PR dont le `headRefName` est une autre
  branche.** La liaison passe par `--head <branch>`, donc par la branche que le
  callout nomme lui-même. Un callout nommant une branche et le plan d'une *autre*
  branche est une incohérence interne du callout, d'une autre classe que celle-ci,
  et aucune occurrence n'en est mesurée. Le geste reste l'état d'avant ce vol :
  pas de réécriture.
- **Il n'ajoute aucun appel réseau au chemin nominal** (§ 1.4).

---

## 3. Fichiers touchés

| fichier | nature du changement |
|---|---|
| `crates/mika-agent/src/plan_callout.rs` | `PriorPlanCause` + prose par variante dans `rewrite_callout_as_prior_plan` ; 7 sites de test adaptés + 2 tests (§ 2.3) |
| `crates/mika-agent/src/auto_pull.rs` | `PlanPresence` + `plan_presence`, `GH_PR_LIST_FILES_CAP`, `parse_merged_pr_for_branch(carrying)`, `gh_merged_pr_for_branch(carrying)`, `stale_callout_cause`, `rewrite_dead_callout` → `rewrite_stale_callout`, branchement dans `promotion_gate_allows`, champ d'audit `staleness` ; tests |

Pas de migration, pas de nouveau module, pas de nouvelle surface HTTP, pas de
changement de `CLAUDE.md` : le discriminant se documente dans les doc-comments
des deux fonctions pures, là où le prochain lecteur le rencontrera.

---

## 4. Ordre de commit

Trois commits. L'ordre est celui qui rend chaque rouge lisible.

1. **`fix(3001): la prose du callout périmé nomme sa cause`** —
   `PriorPlanCause`, les deux prose, les 7 sites de test adaptés, les 2 nouveaux
   tests de `plan_callout`. `cargo test -p mika-agent plan_callout` vert à ce
   commit ; `auto_pull` ne compile pas encore (un appelant), donc ce commit n'est
   pas mergeable seul — il est la moitié basse d'un plancher.
2. **`fix(3001): la porte lit si `main` a absorbé le plan du callout`** —
   `PlanPresence` + `plan_presence`, `GH_PR_LIST_FILES_CAP`, le paramètre
   `carrying`, et leurs tests, dont la fixture #2958. Les fonctions pures sont
   vertes et **encore inemployées** par la porte : c'est ce qui permet de lire le
   rouge du commit 3 comme un rouge de câblage et non de discriminant.
3. **`fix(3001): le callout d'un vol mergé est réécrit même si la branche est repoussée`**
   — `stale_callout_cause`, `rewrite_stale_callout`, le branchement, le champ
   d'audit, et le test de câblage du chemin B.

**Plancher de ce vol :** les trois commits. Le 1 seul ne compile pas ; le 1+2
livrent un discriminant que rien n'appelle, c'est-à-dire la classe exacte que
mika#2578 condamne — une garde que le pipeline n'atteint jamais.

---

## 5. Definition of Done

- [ ] La réécriture n'est plus gardée par `StalenessMeasurement::BranchDeletedByMerge` :
      `promotion_gate_allows` appelle `stale_callout_cause` inconditionnellement
      dès que le corps porte une branche **et** un plan.
- [ ] `plan_presence` est pure, sans E/S, et ses quatre règles sont dans l'ordre
      du § 2.2 — `ahead_by == 0` **avant** la lecture de `changed_files`.
- [ ] `parse_merged_pr_for_branch` reste le site unique de la décision « quelle PR
      mergée » ; `carrying: None` préserve le comportement du chemin A.
- [ ] La prose du chemin A est inchangée **à l'octet** :
      `mika2839_le_run_entier_devient_une_ligne_de_prose` passe sans
      modification de sa constante attendue.
- [ ] `plan_reading` rend `PlanReading::PriorOnly` sur la prose du chemin B, par
      extension du test d'aller-retour mika#2952 et non par un second test.
- [ ] La ligne d'audit nominale porte `"staleness"` avec l'une des deux valeurs ;
      le vocabulaire de `cause` est inchangé.
- [ ] Aucun appel `gh` supplémentaire quand le plan du callout est dans la diff de
      la branche (le cas nominal d'une PR ouverte).
- [ ] Les quatre tests mika#2839 et les tests de corpus mika#2952 passent.
- [ ] `cargo test -p mika-agent` vert.
- [ ] `cargo clippy --all-targets -- -D warnings` vert.
- [ ] `cargo fmt --check` vert.
- [ ] `scripts/verify-pipeline.sh origin/main` vert.
- [ ] Corps de PR : `Code review: delegated to mika-qa (mika#2694)`.

---

## 6. Acceptance criteria

Transcrits verbatim du corps du ticket, avec leur disposition.

- **AC1.** La porte juge un callout périmé sur le **merge de la PR** qui a porté
  son plan, et non sur l'absence de la branche. Le plan dit par quelle lecture
  elle relie le plan du callout à cette PR.
  → **Livré**, § 2.1–2.2. La lecture est nommée et en trois temps : (i) le plan
  du callout, sous sa forme `normalized` déjà portée par `PlanCallout` ; (ii)
  `plan_presence`, qui dit sur la charge `compare` **déjà reçue** si la branche
  ajoute encore ce plan à `main` ; (iii) `parse_merged_pr_for_branch(…,
  carrying: Some(plan))`, qui retient la PR **mergée** dont `files[].path`
  contient ce plan. La liaison est donc par **les fichiers de la PR**, prouvée
  sur la population réelle au § 1.3, et non par l'horodatage du merge — dont la
  même table montre qu'il nomme la bonne PR par coïncidence de séquence.

- **AC2.** Test dans le même appel, avec contrôle négatif :
  - un callout dont la PR est mergée, avec une branche repoussée qui porte un
    plan neuf, est réécrit ;
  - un callout dont la PR est ouverte n'est pas réécrit ;
  - le cas déjà couvert, branche supprimée par le merge, reste réécrit.
  → **Livré**, § 7. Les deux premiers cas sont les deux moitiés d'**une même
  fonction de test** sur **une même charge** — celle de #2958, qui porte à la
  fois la PR mergée du vol 2d et une PR ouverte portant le plan du vol 2e — de
  sorte que le contrôle négatif n'est pas supprimable séparément du cas positif.
  Le troisième est tenu par les quatre tests mika#2839 existants, conservés et
  seulement adaptés à la nouvelle signature : leur prose attendue ne change pas
  d'un octet, ce qui en fait un contrôle de non-régression et non un test
  réécrit.

- **AC3.** Le cas mesuré de mika#2958 sert de fixture.
  → **Livré**, § 1.3 et § 7. La fixture est la charge `gh pr list --head
  feat/2958/manager-phase-2-c-vol-2-le-manager --state all --json
  number,state,mergedAt,baseRefName,headRefName,files` relue sur la forge le
  2026-11-01, avec ses six PR et le plan que chacune porte. Les numéros de
  tickets qu'elle contient sont des numéros réels mais la charge est **figée** :
  aucun appel réseau, et le plancher de fixture de mika#2713 n'est pas franchi
  puisque le sujet `gh` de cette charge est `GhSubject::Repo` et que le test ne
  l'adresse pas par la couture.

---

## 7. Fire-Disposition

**Option (a) — exception nommée en allowlist, et il n'y a aucune allowlist à
livrer, parce que la population de violations est vide par construction.**

Les détecteurs livrés par ce vol sont des **tests**, et aucun n'est un scan d'une
population existante : chacun porte son entrée en paramètre.

| détecteur | population scannée | violations existantes |
|---|---|---|
| `mika3001_plan_presence_lit_le_compte_avant_la_liste` | les 4 règles du § 2.2, sur charges figées | aucune — table de cas |
| `mika3001_la_pr_mergee_portant_le_plan_est_celle_qui_est_nommee` | la charge figée de #2958 (AC2 cas 1 + cas 2, AC3) | aucune — fixture |
| `mika3001_le_chemin_b_ecrit_la_prose_de_sa_cause` | un corps de fixture (bande `990_3xx`) | aucune |
| `mika2952_…_relue_par_le_lecteur` (étendu) | les deux prose | aucune |
| les 4 tests `mika2839_…` (conservés) | les corps canoniques existants | aucune |

Aucun de ces tests ne balaie l'arbre, aucun ne lit le réseau, aucun ne peut
rougir sur une donnée préexistante qu'on devrait exempter : il n'y a donc **rien
à mettre en allowlist**, et en inventer une vide serait du décor. Ce qui remplace
l'allowlist, et qui est la vraie garde de ce vol, est la conservation à l'octet
de la prose du chemin A : les trois corps de tickets réels du corpus mika#2952
(#2943, #2946, #2680) sont déjà dans l'arbre et une dérive de cette prose les
ferait rougir immédiatement.

Ni (b) ni (c) : les détecteurs atterrissent **armés** — leur population de
violations est vide à la fin du vol, mesurée et non espérée — et aucune question
de cadrage ne reste ouverte pour l'opérateur.

---

## 8. Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `plan_callout.rs` — `PriorPlanCause` + doc | 30 |
| `plan_callout.rs` — prose par variante + signature | 20 |
| `plan_callout.rs` — 7 sites de test adaptés | 8 |
| `plan_callout.rs` — 2 tests (prose B figée, mika#2952 étendu) | 35 |
| `auto_pull.rs` — `PlanPresence` + `plan_presence` + doc | 50 |
| `auto_pull.rs` — `GH_PR_LIST_FILES_CAP` + `carrying` dans le parseur + doc | 25 |
| `auto_pull.rs` — `gh_merged_pr_for_branch` (`files`, `carrying`) | 10 |
| `auto_pull.rs` — `stale_callout_cause` + doc | 45 |
| `auto_pull.rs` — `rewrite_stale_callout` (cause, champ d'audit) | 25 |
| `auto_pull.rs` — branchement dans `promotion_gate_allows` | 18 |
| tests `auto_pull.rs` — `plan_presence`, table des 4 règles | 30 |
| tests `auto_pull.rs` — fixture #2958, cas positif + négatif (AC2/AC3) | 60 |
| tests `auto_pull.rs` — câblage du chemin B (prose écrite) | 45 |
| tests `auto_pull.rs` — adaptation des 4 tests mika#2839 | 10 |

Total estimé : 411 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol, et pourquoi il tient.** 411 sous la borne de 429 et très
largement sous le seuil par PR de 1000 : la plus serrée des deux est la borne par
vol, et le périmètre repasse dessous. Le découpage en vols serait d'ailleurs
contre-productif ici — le § 4 le dit : les commits 1 et 2 livrés seuls laissent
un discriminant que rien n'appelle, qui est la classe mika#2578. Le plancher est
indivisible.

Les deux tiers du total sont des doc-comments et des tests, dans un fichier dont
c'est la densité habituelle ; le code de décision proprement dit fait moins de
100 lignes, et deux de ses trois lectures réemploient des valeurs déjà reçues
(§ 1.4). La seule inconnue de volume est l'adaptation des 7 sites de test de
`plan_callout` — mécanique, un argument par site.
