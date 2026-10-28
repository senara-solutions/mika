# mika#2952 — le bassin ne recommande plus un ticket dont le vol est livré

> **Vol 1/2.** Ce vol ferme **AC1** et **AC3** : le candidat dont le vol est livré
> porte un refus nommé, et les deux contrôles négatifs tiennent. **AC2** (la
> dormance) et **AC4** (la sonde après déploiement, qui porte sur les quatre
> tickets) sont renvoyées au vol 2/2 du même ticket.
> `Tracked in: senara-solutions/mika#2952 (vol 2/2)`

## Le défaut, en une phrase

Le bassin lit `plan_present` — un booléen produit par un lecteur **lâche** — et en
conclut « ce ticket a un plan groomé, donc un vol est dû ». Sur #2943, #2946 et
#2680 ce booléen est vrai pour une raison qui dit exactement le contraire : la
seule référence `docs/plans/` que leur corps porte encore est une ligne
**`Plan antérieur :`**, écrite précisément parce que le vol a été livré et que la
branche qui le portait a disparu au merge.

## Ce qui a été mesuré, et pourquoi la racine n'est pas celle que le ticket désigne

Le ticket pointe « rien ne lit l'état des PR du ticket ». La mesure dit que le
mécanisme est en amont, et moins cher à corriger.

`reader.rs::plan_callout_present` (l. 746) a deux branches :

```rust
if t.starts_with("> ") && t.contains("**Plan:**")    { return true; }
if t.starts_with("> ") && t.contains("docs/plans/")  { return true; }  // la lâche
```

`plan_callout::rewrite_callout_as_prior_plan` (la porte d'auto-pull, mika#2839)
remplace **tout le run** de lignes `> - ` — `Branch`, `Plan`, `Grooming history` —
par une ligne unique :

```
> Plan antérieur : `{plan}` — la branche `{branch}` qui le portait a été supprimée
  par le merge de #{merged_pr} dans `main`. Callout réécrit par la porte d'auto-pull (mika#2839).
```

Cette ligne commence par `> ` et contient `docs/plans/`. La seconde branche la lit
donc comme un callout vivant. Corps relus le 2026-10-28 :

| ticket | ligne `> - **Plan:**` stricte | ligne `Plan antérieur :` | `plan_present` |
|---|---|---|---|
| #2943 | **absente** | machine (merge de #2945) | `true` ← faux |
| #2946 | **absente** | machine (merge de #2949) | `true` ← faux |
| #2680 | **absente** | **écrite à la main** (cite #2833, *fermée sans merge*) | `true` ← faux |
| #2854 | **absente** | deux, machine | `true` ← faux |

`reader.rs:738-745` nommait déjà cette faiblesse **et sa précondition de
levée** : « *aucune mesure ne l'établit. Suivi nommé, avec sa précondition : un
rapport dont `plan_present` est mesurément faux* ». Le rapport du sink du
2026-10-28T17:06:59 **est** cette mesure.

### La levée n'est PAS un resserrement de `plan_callout_present`

mika#2608 a **décidé** de conserver la tolérance lâche, l'a inscrite dans la table
des quatre lecteurs (`plan_callout.rs:50`) et l'a épinglée par deux tests
(`mika2608_la_tolerance_lache_est_une_decision_epinglee` et
`mika2608_la_branche_lache_est_un_surensemble_du_lecteur_strict`). Resserrer ce
booléen casserait une décision ratifiée et changerait aussi
`SubIssue::is_in_governed_progress`, le palier 2 de l'`Assessor` et la relève — des
consommateurs que ce ticket ne juge pas.

Ce vol **sépare** au lieu de resserrer. Le booléen reste exactement ce que
mika#2608 a figé ; le bassin reçoit à côté une lecture **à trois états**. La
propriété de sur-ensemble que mika#2608 a épinglée est précisément ce qui rend
l'état intermédiaire bien défini : `lâche \ stricte` est non vide, et c'est la
population mesurée. Un booléen portait deux populations sous un nom — classe
mika#2205 ; on leur donne deux noms.

## Les trois questions que le ticket exige de trancher

### 1. Comment « vol livré » se lit — et ce que coûte chaque source

| source | coût `gh` | couvre quels candidats | verdict |
|---|---|---|---|
| **le corps, sur le marqueur `Plan antérieur :`** | **zéro appel supplémentaire** — la lecture 2 (`gh issue list --milestone`) et la lecture 5 (`gh issue list --state open`) demandent déjà `body` | **les deux** origines, milestone et hors-milestone, à l'identique | **retenu** |
| `s.pr_state == Some("merged")` (la donnée de `detect_silent_progress`) | zéro appel, mais **hydratée pour les seuls sous-tickets du milestone** (`gh pr list --search milestone:N`), absente pour tout candidat hors milestone, et ne nomme **qu'une** PR de clôture | moitié de la population | écarté : prédicat aveugle par origine (mika#2205), et incapable de répondre à AC3 — une seule PR nommée ne distingue pas « vol 1/2 » de « tout livré » |
| sonde de timeline par candidat (la donnée de mika#2930) | **un `gh api …/timeline` par candidat**, soit jusqu'à `PLAN_SIZE_READ_MAX` = 12 appels par cycle là où le cycle en paie 7 au total aujourd'hui — et `read_timeline_probe` borne volontairement à `TIMELINE_PROBE_MAX` = **1** au site d'appel, précisément pour qu'élargir la cible ne puisse pas élargir le coût | les deux origines | écarté : ~2,7× le budget `gh` du cycle, pour une donnée (`cross-referenced`) dont le doc-comment de `compose_merged_prs` dit déjà qu'elle est « plus large que « la PR qui livre ce ticket » » |

**Décision.** Le marqueur `Plan antérieur :` du corps. C'est le seul signal
*produit par la livraison elle-même*, uniforme sur les deux origines, et gratuit.

**Son prix, et il est réel.** Le doc-comment de `rewrite_callout_as_prior_plan`
dit aujourd'hui : « *la prose produite a un écrivain et zéro lecteur — rien ne
peut dériver de son orthographe* ». Ce vol crée ce lecteur. L'orthographe devient
donc un contrat, et la mitigation est structurelle, pas prudentielle :

- le préfixe littéral devient une **constante unique**, `PRIOR_PLAN_PREFIX`, que
  l'écrivain `rewrite_callout_as_prior_plan` et le lecteur `plan_reading`
  consomment tous deux — aucun des deux ne l'épelle ;
- un **test d'aller-retour** fait écrire une ligne par l'écrivain et exige que le
  lecteur la reconnaisse. Changer la prose sans changer la constante devient
  impossible à merger ;
- le doc-comment est rectifié sur place : « un écrivain, un lecteur, et un test
  qui les joint ».

**Ce que le lecteur n'ancre PAS**, et c'est délibéré : la clause
« a été supprimée par le merge de #N ». #2680 porte une ligne `Plan antérieur :`
**écrite à la main**, sans cette clause, et dont le numéro de PR qu'elle cite
(#2833) est celui d'une PR **fermée sans merge**. Ancrer sur la clause raterait
#2680 ; en extraire un numéro de PR mentirait. Le refus porte donc **le chemin du
plan**, qui est la donnée réellement mesurée, et jamais un numéro de PR.

### 2. Comment « vol 1/2 livré » se distingue de « tout livré »

Le prédicat ne tombe **pas** sur « au moins une PR mergée ». Il tombe sur une
lecture à trois états du corps, et c'est la **convention de la boucle** qui le
rend correct :

```
PlanReading::Live       — le corps porte un callout `> - **Plan:** `docs/plans/…`` strict
                          (plan_callout(body, FenceHandling::Strip).is_some())
PlanReading::PriorOnly  — pas de callout strict, et au moins une ligne PRIOR_PLAN_PREFIX
PlanReading::Absent     — ni l'un ni l'autre
```

Quand un découpage en N vols a son vol 1 mergé et son vol 2 dû, la boucle
**re-groome** le ticket : `_write_canonical_callout` écrit un callout `Plan:`
canonique frais pour le vol suivant, sur une branche neuve. Le corps porte alors
les **deux** — la ligne `Plan antérieur :` du vol livré *et* un callout strict pour
le vol dû. L'état est `Live`, et le candidat reste admissible. C'est exactement le
cas AC3-b.

Quand tout est livré, la porte d'auto-pull a consommé le run et rien ne l'a
réécrit depuis : il ne reste que `PriorOnly`. #2943, #2946, #2680.

**Le signal corroborant qui existe, et pourquoi il n'est pas le prédicat.**
`SubIssue.branch_present` tombe à `false` au même instant, puisque la réécriture
emporte la ligne `Branch`. Mais il n'est calculé que sur le chemin milestone
(`PoolCandidate` ne le porte pas), et la ligne manuscrite de #2680 n'a jamais eu de
ligne `Branch` à perdre : il serait aveugle par origine, et faux sur un cas sur
trois. Il reste utile comme vérification croisée dans le test du corpus mesuré.

### 3. La dormance — renvoyée au vol 2/2, avec sa décision déjà prise

Pour qu'un futur éditeur ne re-tranche pas ce qui est tranché :

- **Le registre `docs/dormeurs.md` n'est pas le marqueur.** Son contrat dit
  « *rouvrir le ticket GitHub cité* » au réveil, donc ses 13 entrées désignent des
  tickets **fermés** — déjà écartés par le filtre silencieux `state != Open`.
  Lire le fichier n'ajouterait rien et introduirait une lecture de disque dans un
  cycle qui n'en a aucune.
- **Le marqueur est la ligne de corps `Condition de réveil`**, avec ou sans
  emphase `**`. Mesurée sur #2854 (« *quand `MIKA_WEBHOOK_COMMENT_TRIAGE` passe à
  `on`* ») et déjà présente dans le corpus de fixtures
  (`tests/fixtures/mika2924/issues-milestone-33.json:51`).
- **Marqueur absent ⇒ non dormant**, et le candidat **continue** vers les portes
  suivantes. Le prédicat est à preuve positive seule : c'est ce que « *il ne doit
  pas l'exclure en silence* » exige, et c'est mika#2331 lu dans le sens sûr.
- **Champ orthogonal, pas un variant de plus.** Le vol 2 ajoutera
  `dormancy: Option<DormancyReading>` à côté de `plan_reading`, et non une
  cinquième variante d'un enum fusionné : les deux lectures sont indépendantes, et
  un ticket peut porter les deux (#2854 le fait).
- **Précédence quand les deux marqueurs sont là** (#2854) : la **dormance
  d'abord**. `AdmissionRefusal` ne porte qu'**un** motif, donc l'ordre des bras
  *est* la priorité du motif. Un dormeur a une condition de réveil vérifiable :
  c'est le geste correctif. Le nommer `PriorPlanOnly` enverrait l'opérateur
  chercher une mesure que l'opérateur a lui-même déclarée bloquée.

## Livrables (vol 1/2)

### L1 — `crates/mika-agent/src/plan_callout.rs` : la grammaire du marqueur, chez son écrivain

- `pub const PRIOR_PLAN_PREFIX: &str = "> Plan antérieur :";` — site unique.
  `rewrite_callout_as_prior_plan` cesse d'épeler le préfixe et le consomme.
- `pub enum PlanReading { Live, PriorOnly, Absent }` — `Debug, Clone, Copy,
  PartialEq, Eq, Serialize, Deserialize`, `#[serde(rename_all = "snake_case")]`.
  Le doc-comment de chaque variante dit **quelle question elle répond** et quel
  geste correctif elle appelle.
- `pub fn plan_reading(body: &str) -> PlanReading` :
  `plan_callout(body, FenceHandling::Strip).is_some()` ⇒ `Live` ; sinon une ligne
  dont le `trim_start` commence par `PRIOR_PLAN_PREFIX` ⇒ `PriorOnly` ; sinon
  `Absent`. La politique de fences est `Strip` — celle d'`auto_pull` — parce qu'un
  callout *cité* dans un bloc clôturé n'est pas un plan vivant (mika#2120).
- Le § *L'idempotence est structurelle* est rectifié : la prose a désormais un
  lecteur, nommé, et le test d'aller-retour est cité par son nom.

Tests inline :

- `mika2952_la_ligne_ecrite_par_la_porte_est_relue_par_le_lecteur` — aller-retour
  écrivain → lecteur. **C'est le test qui tient le contrat d'orthographe.**
- `mika2952_un_callout_vivant_a_cote_dune_ligne_anterieure_reste_live` — le cas
  AC3-b, au niveau de la grammaire.
- `mika2952_le_corpus_mesure_rend_prior_only` — extraits de corps de #2943, #2946
  et **#2680** (la ligne manuscrite), en `const &str`. C'est le corpus de
  régression, et il contient le cas que la clause de merge aurait raté.

### L2 — `crates/mika-agent/src/milestone_manager/types.rs`

- `SubIssue` gagne `#[serde(default)] pub plan_reading: Option<PlanReading>`.
  `None` veut dire **« le corps n'a pas été lu par ce binaire »**, jamais
  « aucun plan » — le motif de `merged_prs` et de `pr_link_source` (mika#2340), et
  la raison pour laquelle le puits hors ligne relit ses blobs anciens sans
  changer de verdict.
- Test : un blob antérieur au champ se relit avec `plan_reading: None`, et un
  `None` sérialise en `null`.

### L3 — `crates/mika-agent/src/milestone_manager/reader.rs`

- `compose_from_gh_outputs` : `plan_reading: Some(plan_callout::plan_reading(&body))`,
  au même endroit où `plan_present` est déjà calculé — **une seule lecture du
  corps, deux questions**.
- `compose_outside_candidates` (l. 641) : idem sur le `PoolCandidate`.
- `plan_callout_present` est **inchangé**, et son doc-comment gagne la phrase qui
  manquait : la précondition qu'il nommait est remplie, et la levée a pris la
  forme d'une séparation, pas d'un resserrement — mika#2608 reste en vigueur.
- Test : les deux chemins de composition posent la même lecture sur le même corps
  (un écart entre origines serait la classe mika#2205 réintroduite).

### L4 — `crates/mika-agent/src/milestone_manager/pool.rs`

- `PoolCandidate` passe de 5 à 6 champs : `#[serde(default)] pub plan_reading:
  Option<PlanReading>`. Le doc-comment du type explique pourquoi ce champ n'est
  pas une violation de sa borne déclarée : c'est une lecture du **corps**, que les
  deux ensembles de candidats paient déjà, et non une mesure de PR/CI/branche que
  `gh issue list` ne rend pas (la raison pour laquelle `PoolCandidate` n'est pas
  un `SubIssue`, mika#2331).
- `impl From<&SubIssue> for PoolCandidate` propage le champ.
- `RefusalReason` gagne :

  ```rust
  /// Le corps ne porte plus de callout `Plan:` vivant — seulement une ligne
  /// `Plan antérieur :`, écrite par la livraison du vol (mika#2952).
  PriorPlanOnly { plan: String },
  ```

  - `label()` : composé **ici**, jamais chez le rapporteur. Il nomme le chemin lu
    et le geste correctif — mesurer et fermer, ou re-groomer le vol suivant — et
    **ne nomme aucun numéro de PR** (§ question 1).
  - `remediable_by_grooming()` : `PriorPlanOnly { .. } => false`. Le `match`
    exhaustif sans bras `_` rendait cette réponse **obligatoire à la compilation**
    — précédent `OperatorHeld { .. } => false` (mika#2923). C'est la moitié
    structurelle d'AC1.
- `well_formed_candidates` (l. 582-660) gagne **un** bras, placé après
  `OperatorHeld` et **avant** `!s.plan_present` :

  ```rust
  if s.plan_reading == Some(PlanReading::PriorOnly) {
      refusals.push(AdmissionRefusal { number: s.number, origin,
          reason: RefusalReason::PriorPlanOnly { plan: … } });
      continue;
  }
  ```

  Avant `NoGroomedPlan` parce que le lecteur lâche rend `plan_present == true` sur
  cette population : l'ordre inverse serait inerte, pas seulement moins précis.
  `None` (binaire antérieur) et `Absent` et `Live` ne déclenchent rien ici —
  `Absent` tombe sur `NoGroomedPlan` comme aujourd'hui.

Tests inline :

- `mika2952_un_vol_livre_porte_un_refus_nomme_et_nest_pas_admis` — **AC1**.
- `mika2952_un_plan_groome_sans_marqueur_anterieur_reste_admis` — **AC3-a**,
  contrôle négatif, même test que le précédent au sens du ticket : même graine,
  seul `plan_reading` change.
- `mika2952_un_vol_1_sur_2_livre_avec_un_callout_vivant_reste_admissible` —
  **AC3-b**. Le corps porte la ligne antérieure **et** le callout frais.
- `mika2952_une_lecture_absente_nest_pas_un_refus` — `plan_reading: None` ⇒
  admissible. Un blob antérieur garde son comportement (mika#2340).

### L5 — `crates/mika-agent/src/milestone_manager/relief.rs`

- `mika2952_un_vol_livre_nentre_pas_dans_la_releve_a_grommer` — la seconde moitié
  d'AC1, au niveau du comportement : `name_relief` filtre sur
  `remediable_by_grooming()`, donc le test vérifie l'effet, pas la seule réponse
  de l'enum.

### L6 — churn de constructeurs

`assessor.rs`, `cadence.rs`, `spawn.rs`, `db/tests/vols_termines_du_manager.rs`
construisent `PoolCandidate` / `SubIssue` en littéral dans leurs tests. Le nouveau
champ y arrive en `None` ou via l'`fn cand(...)` partagée. Aucun comportement
n'y change.

## Ordre de commit

1. **L1** seul — `PRIOR_PLAN_PREFIX`, `PlanReading`, `plan_reading`, l'aller-retour
   et le corpus. `cargo test -p mika-agent plan_callout` vert. La grammaire est
   close avant qu'un consommateur en dépende.
2. **L2 + L3 + L6** — le champ voyage et s'alimente, sans qu'aucun jugement ne
   change. La suite entière doit rester verte : c'est le contrôle que l'ajout est
   inerte tant que le bassin ne lit pas.
3. **L4 + L5** — le bras d'admission, le refus nommé, la garde de relève. C'est le
   seul commit qui change un verdict.

## Vérification

```bash
cargo test -p mika-agent plan_callout
cargo test -p mika-agent milestone_manager
cargo test -p mika-agent              # la suite entière : le bassin a 4 consommateurs
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/verify-pipeline.sh origin/main
```

**Sonde après déploiement (AC4, partielle à ce vol).** AC4 porte sur les quatre
tickets et n'est donc close qu'au vol 2. Ce que le vol 1 rend vérifiable, sur le
premier rapport du sink sous le nouveau binaire :

- #2943, #2946 et #2680 **disparaissent** de la ligne `- À admettre :` ;
- chacun apparaît sur une ligne `- Refusé : #N (<priorité>) — <motif>`, rendue par
  `reporter.rs:385-392` via `reason.label()` — le rapporteur n'est **pas** modifié
  par ce vol, et c'est la preuve que le patron de refus nommé de mika#2923 était
  le bon point d'extension ;
- #2854 y **reste** en « à admettre », et ce n'est pas une régression : c'est le
  périmètre du vol 2/2, nommé ici.

L'**absence** de ces lignes ne veut pas dire « plus de faux candidats » mais
« binaire antérieur au vol » (mika#2340) : la section n'existe que si
`assessment.pool` est `Some`, et le motif n'existe que dans le nouveau binaire.
La lecture se fait donc sur la **présence** des lignes `Refusé`, jamais sur
l'absence des lignes `À admettre`.

## Definition of Done

- `PRIOR_PLAN_PREFIX` est l'unique site du préfixe, consommé par l'écrivain et par
  le lecteur ; le test d'aller-retour les joint.
- `plan_callout_present` est **inchangé** et les deux tests de mika#2608 sont
  verts sans édition.
- `PoolCandidate` et `SubIssue` portent `plan_reading: Option<PlanReading>` avec
  `#[serde(default)]`, et un blob antérieur se relit en `None`.
- `RefusalReason::PriorPlanOnly` existe, compose son libellé dans `label()`, et
  rend `false` depuis le `match` exhaustif de `remediable_by_grooming()`.
- Le bras d'admission est à **un seul** endroit, `well_formed_candidates`, avant
  `NoGroomedPlan`.
- Les quatre tests du bassin et la garde de relève sont verts ; la suite complète,
  clippy, fmt et `verify-pipeline.sh` sont verts.
- Le périmètre renvoyé au vol 2/2 est écrit dans le corps de la PR par une ligne
  `Tracked in:`.

## Acceptance criteria

- **AC1.** Un candidat ouvert dont le travail de vol est livré (PR mergées) et dont ne reste qu'une sonde, ou une mesure, n'est pas admis. Il porte un refus **nommé**, distinct de `NoGroomedPlan` : il ne doit pas entrer dans la relève à grommer (`relief::name_relief`), puisqu'aucun groom ne le débloque. Le plan tranche comment « vol livré » se lit.
- **AC2.** Un candidat dormant, qui porte une condition de réveil (règle « Dormeurs visibles », ratifiée par Vincent le 2026-09-01), n'est pas admis. Il porte un refus nommé qui cite la condition. Le plan dit par quel marqueur la dormance se lit, et ce que le bassin fait d'un ticket dont le marqueur est absent : il ne doit pas l'exclure en silence.
- **AC3.** Contrôle négatif dans le même test. Un ticket ouvert avec un plan groomé, sans PR mergée et sans dormance, reste admis. Un ticket dont une **seule** PR de vol est mergée sur un découpage en N vols, avec un vol suivant dû, reste admissible. Le prédicat ne doit pas tomber sur « au moins une PR mergée ». Le plan dit comment il distingue « vol 1/2 livré » de « tout livré ».
- **AC4.** Sonde après déploiement. Le premier rapport du sink sous le nouveau binaire ne propose plus #2943, #2946, #2680 ni #2854 en « à admettre », et les nomme avec leur motif.

**Répartition sur les deux vols.** Vol 1/2 (ce plan) ferme **AC1** et **AC3**, et
rend AC4 vérifiable sur trois des quatre tickets. Vol 2/2
(`Tracked in: senara-solutions/mika#2952 (vol 2/2)`) ferme **AC2** — champ
`dormancy: Option<DormancyReading>`, `RefusalReason::Dormant { condition }`,
précédence devant `PriorPlanOnly`, contrôle positif du marqueur absent — et
**AC4** en entier.

## Fire-Disposition

Ce vol livre des détecteurs : les sept tests nommés ci-dessus, dont le chemin de
succès est « aucune violation trouvée ».

**Option (a) — exception nommée en allowlist, livrée avec une allowlist VIDE.**

- Aucun des sept ne scanne le dépôt ni une population existante : ils assertent un
  comportement neuf sur des corps construits à la main, plus trois extraits de
  corps mesurés le 2026-10-28 dont la lecture attendue est précisément celle que ce
  vol produit. **Zéro violation préexistante**, donc zéro entrée à inscrire.
- L'allowlist est néanmoins **déclarée et vide**, dans
  `plan_callout.rs` à côté du corpus, sous la forme
  `const MIKA2952_CORPUS_EXCEPTIONS: &[(&str, &str)] = &[];` — un tuple
  `(corps, ticket de suivi)`. Une future ligne `Plan antérieur :` dont la lecture
  devrait être exceptée doit donc être **nommée avec son ticket**, jamais absorbée
  par un élargissement du prédicat.
- L'assertion auto-nettoyante est son voisin,
  `mika2952_lallowlist_du_corpus_est_vide` : elle rougit si l'allowlist cesse
  d'être vide sans que son ticket de suivi soit cité, et c'est elle qui empêche
  l'exception de devenir l'allowlist que mika#2309 condamne. Précédent de forme :
  mika#2425 et mika#1833, dont les scans de source sont livrés allowlist vide et
  épinglés vides par un test frère.
- Rien n'est livré désarmé : aucun `#[ignore]`, aucun `#[cfg(skip)]`.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/plan_callout.rs` (L1 — const, `PlanReading`, `plan_reading`, doc, 3 tests + allowlist) | 135 |
| `crates/mika-agent/src/milestone_manager/types.rs` (L2 — champ, doc, 1 test) | 30 |
| `crates/mika-agent/src/milestone_manager/reader.rs` (L3 — 2 sites d'alimentation, doc, 1 test) | 45 |
| `crates/mika-agent/src/milestone_manager/pool.rs` (L4 — champ, variant, `label()`, bras d'admission, 4 tests) | 160 |
| `crates/mika-agent/src/milestone_manager/relief.rs` (L5 — 1 garde) | 25 |
| churn de constructeurs (L6 — `assessor.rs`, `cadence.rs`, `spawn.rs`, `db/tests/vols_termines_du_manager.rs`) | 20 |

Total estimé : 415 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

415 < 429 : **un seul vol**, et il tient pour une raison nommable — la dormance
est un champ orthogonal, pas une variante de plus, donc elle se détache sans
toucher une ligne de L1 à L5. Le découpage suit la frontière du type, pas une
coupure arbitraire dans un même livrable.

Tracked in: senara-solutions/mika#2952 (vol 2/2)

## Hors périmètre

- **Resserrer `plan_callout_present`.** Décision ratifiée par mika#2608, épinglée
  par deux tests. Ce vol sépare, il ne resserre pas.
- **Faire lire `pr_state` ou une sonde de timeline au bassin.** Écartés avec leur
  coût, § question 1. Si une mesure future montre une population que le marqueur
  de corps ne couvre pas, c'est ce § qu'il faut rouvrir — avec la mesure.
- **Lire `docs/dormeurs.md`.** Ses entrées désignent des tickets fermés, déjà
  écartés par `state != Open`.
- **Ouvrir POOL_WRITE** (`MIKA_MANAGER_POOL_WRITE`, mika#2699). Le bassin reste en
  mode ombre ; ce ticket retire une condition à son ouverture, il ne l'ouvre pas.
- **Toucher `reporter.rs`.** Il appelle déjà `reason.label()` ; qu'aucune édition
  n'y soit nécessaire est une propriété à vérifier, pas un livrable.
