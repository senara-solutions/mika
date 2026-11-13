# mika#2839 — la porte d'auto-pull réécrit le callout d'une branche morte

> - **Ticket:** senara-solutions/mika#2839 (suivi de mika#2837)
> - **Type:** chore
> - **Vol:** unique

## Le problème, tel qu'il est mesuré

mika#2837 a appris à la porte d'auto-pull à **promouvoir** quand la branche du
callout est absente d'`origin` parce que le merge de sa PR l'a supprimée. La
mesure est portée par `StalenessMeasurement::BranchDeletedByMerge { pr }`
(`crates/mika-agent/src/auto_pull.rs:743`), produite par `classify_absent_branch`
(l. 1138) à partir de la sonde `gh pr list --head <branche> --state all`
(`gh_merged_pr_for_branch`, l. 3144), et le verdict qui en découle est
`PromotionGate::Promote { detail: "branch_deleted_by_merged_pr" }` (l. 1187).

Ce que mika#2837 n'a pas fait : **corriger le corps du ticket**. Il continue donc
de nommer une branche morte, et les deux conséquences sont celles du ticket :

1. la sonde est repayée à **chaque** pull, pour reconstruire une réponse qui ne
   changera plus — une branche supprimée par un merge ne revient pas ;
2. un lecteur humain lit une branche qui n'existe pas, et refait l'enquête que la
   porte vient de faire.

La population est déjà comptable depuis mika#2837 : les lignes
`audit_events` de `target_key = 'auto_pull_staleness_measured'` groupées par
`$.measurement` / `$.merged_pr_probe`, dont la ligne
`branch_deleted_by_merge / carried`. **C'est l'instrument de la correction aussi :
après ce vol, cette ligne doit cesser de remonter sur les mêmes tickets.**

L'opérateur a déjà appliqué la correction à la main — #2680 le 2026-10-19, #2108
le 2026-10-20 — sous une forme stable, une ligne de prose « Plan antérieur » qui
remplace le bloc de trois lignes :

```
> Plan antérieur : `docs/plans/2026-10-19-006-fix-2108-orienter-le-pilote-avant-le-corps-du-ticket-plan.md` (sur `main`). Le vol 1 est livré par #2834 (mergée en `35a8b0ac`), et sa branche est supprimée.
```

## Les deux objections de mika#2837, et comment ce plan y répond

mika#2837 a placé cette réécriture hors périmètre pour deux raisons. Elles ne sont
pas des détails d'implémentation : ce sont les deux contraintes de conception, et
un plan qui ne les adresse pas ne doit pas être livré.

### Objection 1 — c'est une écriture de corps d'issue, surface qu'`auto_pull` ne possède sur aucun chemin

Vrai aujourd'hui : `auto_pull` n'écrit que des **labels** (`gh_apply_label`
l. 2963, `gh_remove_label` l. ~3003 — les deux seuls sites `gh issue edit` du
module) et des **commentaires** (`gh_comment_issue` l. 3047).

Réponse : la surface s'ouvre **dans la classe de permission qui est déjà la
sienne**. `gh issue edit --body` et `gh issue edit --add-label` relèvent du même
scope `issues: write`, porté par le même `LabelWriteToken`
(`crates/mika-common/src/label_write.rs`) et déjà résolu sur le chemin de la
porte. Le nouvel écrivain prend donc `&LabelWriteToken` — **le type est le
gardien** : il rend impossible le passage du token de lecture — et appelle
`report_write_failure("auto_pull", issue, "body", err)` sur échec, de sorte qu'un
refus de permission atterrit sous `label_write_app_token_insufficient` comme ses
voisins, et pas sous un nom neuf que la checklist opérateur ne connaît pas.

Aucun nouveau scope, aucun nouveau token, aucune nouvelle provenance. Ce qui
s'élargit est le **verbe**, pas la permission.

### Objection 2 — elle entrerait en course avec `_write_canonical_callout`

« Deux écrivains sur le même callout sans ordre défini est le défaut à ne pas
introduire pour économiser un appel de sonde. » Le plan prend cette phrase au mot
et ordonne les deux écrivains par **le domaine sur lequel chacun agit**, et non
par un verrou :

- `_write_canonical_callout` (`skills/bundled/_shared/dispatch-lib.sh:9998`+)
  écrit le bloc de trois lignes quand un grooming vient de converger sur `GROOMED`,
  c'est-à-dire quand une branche **vivante** porte un plan tout juste commité.
- le nouvel écrivain n'agit **que** sur l'arm `BranchDeletedByMerge`, c'est-à-dire
  quand la forge vient de répondre que la branche nommée par le callout a été
  **supprimée par le merge d'une PR**. Aucun grooming ne peut être en train
  d'écrire ce callout-là : il n'écrit que des branches qu'il vient de pousser.

Les deux domaines sont disjoints par construction, pas par chance. Ce n'est
cependant pas une preuve d'exclusion mutuelle — rien n'empêche un **nouveau**
grooming de démarrer sur le même ticket pendant que la porte mesure l'ancien
callout, et de le réécrire pour sa propre branche. Le plan ferme ce reste avec un
**compare-and-write sur une lecture fraîche** :

1. relire le corps juste avant d'écrire (`gh issue view --json body`) ;
2. n'écrire que si ce corps frais nomme **encore** la même branche morte et le
   même chemin de plan que ceux sur lesquels la mesure a porté ;
3. sinon, ne rien écrire et émettre `cause = body_changed`.

**Ce n'est pas un CAS** et le plan le dit plutôt que de le laisser croire : il
reste une fenêtre entre la lecture fraîche et l'écriture, de l'ordre du temps d'un
appel `gh`. Ce qui rend ce reste acceptable est le **coût borné du perdant** : si
nous écrasons un callout frais, les trois signaux de `is_groomed` disparaissent,
le ticket redevient non-groomé, et mika#996 le re-groome au prochain dispatch.
Le coût d'une perte de course est **un dispatch de grooming**, jamais une
corruption ni un plan perdu — et il est compté, parce que l'événement de
réécriture est audité.

## Requirements

**R1 — le transform est pur et vit dans le seul module qui possède le motif.**
Une nouvelle fonction dans `crates/mika-agent/src/plan_callout.rs` :

```rust
pub fn rewrite_callout_as_prior_plan(body: &str, branch: &str, merged_pr: u64) -> Option<String>
```

Elle ne touche ni le réseau ni l'horloge. Elle vit là et pas dans `auto_pull.rs`
parce que mika#2194 fait de `plan_callout.rs` **le site unique** du motif de
callout, tenu par le scan `mika2194_aucun_motif_de_callout_hors_de_ce_module`
dont l'allowlist est `const ALLOWED: &[&str] = &[];` — vide, et qui doit le
rester. Un réécriveur qui porterait le motif ailleurs ferait rougir ce scan, et la
bonne réponse à un scan structurel est de **router le site**, pas de l'inscrire en
exception (mika#2201/#2323). La composition de la ligne de remplacement est dans
la même fonction, pour la même raison : le vocabulaire du callout — ce qu'on y
écrit comme ce qu'on y lit — reste dans un module.

**R2 — le transform est line-wise et conscient des fences.**
`strip_fenced_blocks` **supprime** des lignes : les offsets d'une recherche menée
sur le corps nettoyé ne se remappent pas sur le corps d'origine. Le transform
parcourt donc les lignes du corps **original** en suivant l'état de fence avec
`fence_marker`, et ne considère la ligne de callout que hors fence. Un corps qui
cite un callout dans un bloc de code n'est pas réécrit à cet endroit.

**R3 — le transform remplace le bloc entier, pas la seule ligne `Plan`.**
`_write_canonical_callout` écrit exactement trois lignes `> - ` contiguës
(`dispatch-lib.sh:10122-10124`) : `Branch`, `Plan`, `Grooming history`. Le
transform étend la ligne de callout sur le **run contigu de lignes `> - `** qui la
contient et remplace ce run par **une** ligne de prose. Remplacer la seule ligne
`Plan` laisserait la ligne `Branch` en place, donc `extract_branch_name` rendrait
toujours la branche morte, donc **la sonde serait encore repayée** : la
conséquence #1 du ticket ne serait pas corrigée.

**R4 — la prose n'affirme que ce qui a été mesuré.**
Forme exacte produite :

```
> Plan antérieur : `<chemin du plan>` — la branche `<branche>` qui le portait a été supprimée par le merge de #<pr> dans `main`. Callout réécrit par la porte d'auto-pull (mika#2839).
```

Trois faits, tous mesurés : le chemin lu dans le callout qu'on remplace, la
branche sur laquelle la mesure a porté, le numéro de PR rendu par
`MergedPrEvidence`. Ce qui est **délibérément absent** : toute affirmation que le
plan est présent sur `main`. La sonde a mesuré « la branche a été supprimée par le
merge de #N dans `main` », pas « le fichier existe à ce chemin sur `main` ». La
prose dit donc le premier. `merged_at`, porté par `MergedPrEvidence` mais pas par
`BranchDeletedByMerge { pr }`, n'est pas remonté : il coûterait un élargissement
du type pour une date que la PR citée porte déjà.

**R5 — l'idempotence est structurelle, sans nouveau marqueur.**
Le transform exige une ligne de callout `> - **Plan:** …` pour agir. Après une
réécriture, cette ligne n'existe plus : un second passage rend `None` par
construction. Aucun marqueur « déjà réécrit » n'est ajouté, et aucune garde
structurelle de second niveau — la prose `Plan antérieur` a un écrivain et zéro
lecteur, donc rien ne peut dériver de son orthographe.

**R6 — la réécriture ne décide pas du verdict.**
Elle est appelée depuis l'arm `BranchDeletedByMerge` de `promotion_gate_allows`
(`auto_pull.rs:3218-3299`), **après** l'écriture de la ligne d'audit de mika#2837,
et son échec est journalisé puis ignoré — même discipline de
*warn-and-continue* que `log_audit_event` sur ce chemin. La promotion que
mika#2837 a gagnée ne peut pas être perdue par une panne d'écriture de corps.

**R7 — la mutation de corps est comptable.**
Une ligne `audit_events` sous `target_key = 'auto_pull_callout_rewrite'`, dont
`after_value` porte `issue`, `branch`, `plan`, `merged_pr`, `outcome`
(`rewritten` | `abandoned`), `cause` et `token_source`. `cause` est `null` sur le
chemin nominal — jamais `""` ni `"none"` : absent n'est pas zéro (mika#2331), et
une valeur par défaut s'agrégerait comme une mesure. Deux sondes `tracing` :
`auto_pull_callout_rewritten` (INFO) et `auto_pull_callout_rewrite_abandoned`
(WARN, portant `cause` dans un vocabulaire clos — `body_changed`,
`body_unreadable`, `write_refused`). La sonde INFO du chemin nominal est ce qui
distingue « la réécriture n'a jamais eu lieu » de « le binaire en service est
antérieur à ce vol » (mika#2340).

**R8 — la chaîne de conséquences est voulue, et le plan la nomme.**
Après la réécriture, sur le ticket concerné :

| lecteur | avant | après |
|---|---|---|
| `extract_branch_name` | la branche morte | `None` |
| mesure de la porte | `BranchDeletedByMerge` (1 sonde `compare` + 1 sonde `pr list`) | `NoBranchCallout` (0 sonde) |
| `is_groomed` | `true` | `false` |
| `executor::check_grooming_markers` | passe | passe (substring `docs/plans/` toujours présent) |
| prochain dispatch | direct | mika#996 auto-groom, puis dispatch |

C'est **exactement** le cycle de vie de #2108 et #2680, reproduit par la machine :
le callout périmé est retiré, le ticket redevient non-groomé, le prochain dispatch
le re-groome et `_write_canonical_callout` écrit un callout frais pour une branche
vivante. La conséquence #1 du ticket est corrigée par la colonne « 0 sonde ».

`executor::check_grooming_markers` cherche le substring `docs/plans/` et **ne doit
pas être resserré** ; la prose `Plan antérieur` le porte encore, donc ce lecteur-là
ne bouge pas. C'est voulu : il répond « ce ticket a un plan quelque part », ce qui
reste vrai.

**R9 — on n'écrit que sur une lecture fraîche qui nomme encore la même cible.**
Le corps est relu juste avant l'écriture, et l'écriture n'a lieu que si ce corps
frais nomme **encore** la même branche morte (`extract_branch_name`) et le même
chemin de plan (`extract_plan_path`) que ceux sur lesquels la mesure a porté.
Sinon : aucune écriture, sortie `abandoned` / `cause = body_changed`. La
comparaison porte sur les deux champs discriminants, parce que c'est eux qu'un
grooming concurrent aurait changés. Ce n'est **pas** un CAS — la fenêtre résiduelle
et le coût borné de la perdre sont nommés plus haut, et comptés par R7.

## Conception

### `crates/mika-agent/src/plan_callout.rs`

`rewrite_callout_as_prior_plan(body, branch, merged_pr) -> Option<String>` :

1. parcourir `body.lines()` en maintenant l'état de fence via `fence_marker` ;
2. repérer l'index de la première ligne hors fence que `callout_re()` matche, et
   capturer le chemin du plan ; rendre `None` s'il n'y en a pas ;
3. étendre l'intervalle vers le haut et vers le bas tant que la ligne est hors
   fence et commence par `> - ` ;
4. rendre le corps reconstruit, le run remplacé par la ligne de prose de R4.

La fonction ne vérifie **pas** que la ligne `Branch` du run nomme `branch` : c'est
l'affaire de l'appelant, qui tient la comparaison fraîcheur de R9 ci-dessous.

### `crates/mika-agent/src/auto_pull.rs`

Trois pièces, dans cet ordre de dépendance :

- un lecteur de corps frais — `gh issue view <n> --json body -q .body` via
  `run_gh` / `GhSubject::Issue(n)` ;
- un écrivain — `gh issue edit <n> --body <corps>`, prenant `&LabelWriteToken`.
  `--body` **inline** et non `--body-file` : `run_gh` pose
  `stdin(Stdio::null())`, donc rien n'est *pipé*, et le précédent du module est
  `gh_comment_issue`, inline lui aussi. Aucun fichier temporaire, donc aucune
  écriture hors worktree ;
- un orchestrateur, appelé depuis l'arm `BranchDeletedByMerge` après l'audit de
  mika#2837 : lire frais → **R9** : exiger que ce corps frais nomme encore la même
  branche (`extract_branch_name`) et le même plan (`extract_plan_path`) que la
  mesure → transformer → écrire → auditer. Tout écart ou toute panne sort par
  `auto_pull_callout_rewrite_abandoned` avec sa `cause`, et rend `Ok(())`.

### Ce qui n'est pas livré, et pourquoi

- **Une sonde `gh api .../contents/<plan>?ref=main`** pour pouvoir écrire « sur
  `main` » comme l'opérateur l'a fait à la main. Rejetée : elle rachète un appel
  réseau par réécriture pour affirmer un fait que la suppression de branche
  implique déjà en pratique, et elle réintroduit sur un autre chemin exactement le
  coût que ce ticket existe pour supprimer. La prose de R4 dit ce qui a été mesuré.
- **Un verrou ou un ETag/CAS sur le corps.** La forge n'offre pas d'écriture
  conditionnée sur un corps d'issue ; prétendre à un CAS avec une relecture serait
  nommer une garantie qu'on n'a pas. Le reste de fenêtre est nommé, son coût est
  borné à un dispatch de grooming, et il est compté.
- **Un second scan structurel** sur la prose `Plan antérieur` : un écrivain, zéro
  lecteur (R5).
- **Tout élargissement de `BranchDeletedByMerge`** pour y porter `merged_at`.

## Ordre de commit

1. `plan_callout.rs` : le transform pur + ses tests unitaires. Mergeable seul,
   vert seul, et ne change aucun comportement en production (personne ne l'appelle
   encore).
2. `auto_pull.rs` : lecteur frais, écrivain, orchestrateur, sondes, ligne d'audit,
   + les tests inline qui les câblent.

## Verification Contract

Tous offline. Le double `gh` de `#[cfg(test)]` (`gh_process.rs`) fournit
`set_canned_stdout(GhSubject, …)` — clé par sujet, donc sûr en parallèle — et
`recorded_calls()`. Les fixtures adressent des issues `>= FIXTURE_ISSUE_FLOOR`
(`990_000`, mika#2713).

**V1 — le transform, dans `plan_callout.rs` :**
- un corps canonique à trois lignes `> - ` devient une ligne `Plan antérieur`
  portant le chemin du plan, la branche et `#<pr>` ; les trois lignes `> - ` ont
  disparu ;
- `rewrite_callout_as_prior_plan` sur un corps **déjà réécrit** rend `None`
  (idempotence structurelle, R5) ;
- un corps dont le seul callout est **dans un bloc de code fencé** rend `None` ;
- un corps sans aucune ligne de callout rend `None` ;
- le texte autour du callout (titres, paragraphes, autres citations) est préservé
  à l'octet près.

**V2 — la chaîne de conséquences, dans `plan_callout.rs` :**
sur la sortie du transform, `extract_plan_path` rend toujours `Some` (le substring
`docs/plans/` subsiste, R8) et le motif `> - **Plan:**` a disparu — c'est la
propriété qui fait tomber `extract_branch_name` et donc la sonde.

**V3 — l'orchestrateur, inline dans `auto_pull.rs` :**
- chemin nominal : corps frais identique à la mesure ⇒ `recorded_calls()` porte un
  `issue edit --body` sur la bonne issue, et son corps est la sortie du transform ;
- corps frais nommant une **autre** branche ⇒ **aucun** `issue edit` dans
  `recorded_calls()`, sortie `abandoned` / `body_changed` ;
- corps frais sans callout ⇒ aucun `issue edit`, `abandoned` / `body_changed` ;
- échec de l'écriture ⇒ la fonction rend `Ok(())` (R6) et `abandoned` /
  `write_refused`.

**V4 — régression de mika#2837 :** la suite `tests/auto_pull_promotion_gate.rs`
reste verte sans modification. Le verdict de l'arm `BranchDeletedByMerge` est
inchangé : `Promote { detail: "branch_deleted_by_merged_pr" }`.

**V5 — le scan mika#2194 reste vert avec son allowlist vide**, et son contrôle
négatif `mika2194_le_scan_voit_un_second_motif` reste rouge-quand-il-doit (il est
la preuve que le scan voit).

**V6 — pipeline :** `cargo test`, `cargo clippy`, `cargo fmt --check`, puis
`scripts/verify-pipeline.sh origin/main`.

## Definition of Done

- `rewrite_callout_as_prior_plan` existe dans `plan_callout.rs`, est pure,
  line-wise, fence-aware, et remplace le run `> - ` entier.
- `promotion_gate_allows` appelle l'orchestrateur sur le seul arm
  `BranchDeletedByMerge`, après l'audit de mika#2837, sans pouvoir changer le
  verdict.
- L'écriture passe par `&LabelWriteToken` et appelle `report_write_failure` sur
  échec.
- `auto_pull_callout_rewritten` (INFO), `auto_pull_callout_rewrite_abandoned`
  (WARN + `cause`) et la ligne `audit_events`
  `target_key = 'auto_pull_callout_rewrite'` sont émis.
- V1–V6 verts.
- Le scan mika#2194 reste vert, allowlist toujours vide.
- Aucune modification de `executor::check_grooming_markers`, de `is_groomed`, ni
  de `_write_canonical_callout`.

## Acceptance criteria

Dérivés des Requirements et du Verification Contract — le corps de mika#2839 ne
porte pas de section `## Acceptance criteria`.

- **AC1** — Un corps portant le callout canonique à trois lignes, passé au
  transform avec la branche et le numéro de PR mesurés, rend un corps où les trois
  lignes `> - ` sont remplacées par une unique ligne
  `> Plan antérieur : …` citant le chemin du plan, la branche morte et `#<pr>`.
  Le reste du corps est inchangé à l'octet près. (V1)
- **AC2** — Le même corps repassé au transform rend `None`. Aucun marqueur
  d'idempotence n'a été ajouté au corps. (V1, R5)
- **AC3** — Un corps dont le seul callout vit dans un bloc fencé rend `None` ;
  aucune ligne à l'intérieur de la fence n'est modifiée. (V1, R2)
- **AC4** — Sur la sortie du transform, `extract_branch_name` rend `None` et
  `extract_plan_path` rend `Some` : la sonde `gh pr list --head` n'est plus
  atteignable sur ce ticket, et `check_grooming_markers` le voit toujours comme
  portant un plan. (V2, R8)
- **AC5** — Quand le corps relu juste avant l'écriture ne nomme plus la même
  branche morte **ou** plus le même chemin de plan, aucun `gh issue edit`
  n'est émis, et la sortie est `abandoned` avec `cause = body_changed`. (V3, R9)
- **AC6** — Une panne d'écriture laisse le verdict de promotion intact : la
  fonction appelante rend la même décision
  `Promote { detail: "branch_deleted_by_merged_pr" }` qu'avant ce vol, et un refus
  de permission atterrit sous `label_write_app_token_insufficient` via
  `report_write_failure`. (V3, V4, R6)
- **AC7** — Chaque passage par l'arm `BranchDeletedByMerge` qui tente une
  réécriture laisse exactement une ligne `audit_events`
  `target_key = 'auto_pull_callout_rewrite'`, dont `$.outcome` vaut `rewritten` ou
  `abandoned` et dont `$.cause` est `null` sur le chemin nominal. (R7)
- **AC8** — Le scan `mika2194_aucun_motif_de_callout_hors_de_ce_module` est vert
  avec `ALLOWED` toujours vide : aucun motif de callout n'a été introduit hors de
  `plan_callout.rs`. (V5, R1)

## Fire-Disposition

Ce plan livre des détecteurs : les tests de V1–V3 sont des livrables dont la
fonction primaire est de signaler une violation, et V5 réutilise un scan
structurel existant.

**Option (a) — exception nommée en allowlist, et il n'y a rien à y inscrire.**

- Les tests de V1, V2 et V3 portent sur du code qui **n'existe pas encore** : leur
  population préexistante est vide par construction, donc ils atterrissent
  **armés**, sans `#[ignore]`, et aucune exception n'est créée.
- V5 n'ajoute aucun détecteur : il réutilise
  `mika2194_aucun_motif_de_callout_hors_de_ce_module`, dont l'allowlist est
  `const ALLOWED: &[&str] = &[];`. **Ce vol ne l'élargit pas, et c'est une
  contrainte de conception plutôt qu'un effet** : R1 place le transform dans
  `plan_callout.rs` précisément pour n'avoir rien à y inscrire. Une entrée
  d'allowlist ici serait l'allowlist que mika#2201/#2323 condamne — router le
  site, pas l'exempter.
- Aucun détecteur n'est livré désarmé, donc aucun ticket d'armement n'est ouvert.
  Le dire est l'engagement : si un test de V1–V3 devait atterrir `#[ignore]`,
  l'implémentation s'arrête et remonte (option (c)) plutôt que de livrer un
  détecteur muet — un désarmement non observable est un espoir (mika#2293).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/plan_callout.rs` — `rewrite_callout_as_prior_plan` + doc | 55 |
| `crates/mika-agent/src/plan_callout.rs` — tests du transform (V1, V2) | 60 |
| `crates/mika-agent/src/auto_pull.rs` — lecteur de corps frais + écrivain `gh issue edit --body` | 45 |
| `crates/mika-agent/src/auto_pull.rs` — orchestrateur, sondes, ligne d'audit | 50 |
| `crates/mika-agent/src/auto_pull.rs` — tests inline (V3) | 55 |

Total estimé : 265 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 265 < 279 (borne par vol, la plus
serrée des deux) et 265 < 1000 (borne par PR). Trois choix de conception tiennent
ce total, et chacun est une renonciation explicite plutôt qu'une estimation
optimiste :

- `BranchDeletedByMerge { pr: u64 }` **n'est pas élargi** pour porter `merged_at`
  (R4) — un champ de plus traverse le type, l'audit, les deux sondes et leurs
  tests ;
- **aucune garde structurelle** n'est ajoutée sur la prose `Plan antérieur` (R5) —
  un scan coûte sa quarantaine de lignes et son contrôle négatif, pour un motif à
  zéro lecteur ;
- **aucune sonde `contents?ref=main`** (§ *Ce qui n'est pas livré*) — elle coûte
  un appel `gh`, son parseur, son arm d'erreur et ses tests.

Si l'un de ces trois choix devait être rouvert en implémentation, le vol repasse
au-dessus de la borne et la suite se découpe en vol 2 avec sa ligne `Tracked in:`.
Aucun n'est nécessaire pour satisfaire AC1–AC8.

## Risques

- **Perte de course contre un grooming concurrent.** Nommée, bornée à un dispatch
  de grooming par R8, et comptée par R7 (`cause = body_changed` compte les
  abandons ; une ligne `rewritten` sur un ticket qui vient d'être groomé se lit
  dans l'audit).
- **Une forme de callout non canonique** — un bloc partiel laissé par mika#1123 ou
  mika#2012, un callout écrit à la main sur quatre lignes. Le transform agit sur le
  run `> - ` contigu qu'il trouve, quel que soit son nombre de lignes ; un callout
  ne portant pas de ligne `Plan` reconnaissable n'est pas touché (`None`).
- **Volume d'écritures de corps.** Une par ticket et par vie de callout, et non une
  par pull : la réécriture supprime sa propre condition de déclenchement (R5/R8).
  C'est la propriété qui empêche ce vol de remplacer une sonde répétée par une
  écriture répétée.
