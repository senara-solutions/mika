# Le bassin ne recommande pas un dormeur, et il le nomme (mika#2952, vol 2/2)

**Vol 2/2.** Le vol 1/2 est livré et mergé — commit `449006f6b`, PR #2954. Il a fermé
**AC1** (un vol déjà livré porte le refus nommé `PriorPlanOnly`, hors relève à grommer) et
**AC3** (les deux contrôles négatifs : plan vivant ⇒ admissible ; vol 1/2 d'un découpage
en N vols ⇒ admissible, puisque le prédicat ne tombe pas sur « au moins une PR mergée »
mais sur la réécriture du callout par la porte d'auto-pull de mika#2839).

Ce vol ferme **AC2** (la dormance) et **AC4** (la sonde après déploiement, qui porte sur les
quatre tickets). Il n'y a pas de vol 3 : les deux AC restantes tiennent sous la borne, et la
raison se lit au § *Taille estimée*.

Le plan antérieur — `docs/plans/2026-10-28-003-fix-2952-manager-le-bassin-recommande-d-admettre-plan.md`,
§ 3 — avait **pré-tranché** la conception de la dormance pour que ce vol ne la re-décide pas.
Ce plan reprend ces cinq décisions, en nomme les deux que la mesure du 2026-10-29 a corrigées,
et en ajoute les trois qui restaient ouvertes.

---

## 1. Ce que la mesure du 2026-10-29 a appris, et qui n'était pas su hier

Trois faits mesurés sur `origin/main` au 2026-10-29. Chacun change un livrable.

### 1.a — #2854 est **déjà** refusé par le vol 1, avec le **mauvais** motif

Le plan antérieur écrivait que « #2854 y **reste** en « à admettre » ». C'est faux depuis le
merge du vol 1. Mesure du corps de #2854 : **zéro** callout `Plan:` strict, **deux** lignes
`> Plan antérieur :`. Donc `plan_callout::plan_reading` rend déjà `PlanReading::PriorOnly`,
et la boucle d'admission le refuse déjà — en `RefusalReason::PriorPlanOnly`.

Conséquence sur le périmètre : **la contribution de ce vol n'est pas de retirer #2854 de la
liste d'admission.** Il en est déjà sorti. La contribution est de lui donner le motif **vrai** :
un opérateur qui lit « un vol déjà livré » sur #2854 va chercher une PR de vol à mesurer, alors
que ce qui bloque le ticket est une variable d'environnement qu'il a lui-même laissée à `off`.
Un motif faux est un instrument qui ment, pas une imprécision.

Conséquence sur la sûreté du vol : **entre le vol 1 et ce vol, aucune admission mal formée ne
subsiste pour #2854.** Ce vol corrige une étiquette, pas une fuite. C'est ce qui le rend
ordonnançable sans urgence.

### 1.b — Le marqueur s'écrit en **trois orthographes**, et aucune n'est négociable

Population mesurée : parmi les tickets **ouverts** du dépôt, exactement **cinq** portent une
ligne `Condition de réveil` dans leur corps — #2117, #2472, #2854, #2912, #2913 — et ils
l'écrivent de trois façons :

| forme mesurée | tickets | ce qui précède le mot |
|---|---|---|
| `> - **Condition de réveil :** …` | #2854 | `> `, puis `- `, puis `**` |
| `> **Condition de réveil :** …` | #2472, #2117 | `> `, puis `**` |
| `**Condition de réveil : …**` | #2912, #2913 | `**` seul, **sans** blockquote, et le **deux-points est à l'intérieur** de l'emphase |

Le prédicat ne peut donc pas être un `starts_with` sur une seule chaîne, ni un test sur
`"**Condition de réveil :**"` — la troisième forme ne le contient pas. Il doit **normaliser**
un préfixe de décoration (`> `, `- `, `*`, `_`, espaces) puis tester le début du texte restant
sur `"Condition de réveil"` **sans** le deux-points ni l'emphase fermante. C'est la seule
forme qui couvre les trois sans allowlist de littéraux.

C'est aussi ce qui rend le livrable **plus gros** que le plan antérieur ne le laissait croire :
il avait écrit « la ligne de corps `Condition de réveil`, avec ou sans emphase `**` », ce qui
sous-estimait la variation d'un cran.

### 1.c — Le delta de comportement est **nul sur les admissions**

Mesure, ticket par ticket, de ce que le bras de dormance changerait :

| ticket | motif aujourd'hui | motif après ce vol | pourquoi |
|---|---|---|---|
| #2117 | `OperatorHeld { blocked }` | **inchangé** | label `blocked` ⇒ `OperatorHeld` mord avant |
| #2472 | `OperatorHeld { blocked }` | **inchangé** | idem |
| #2854 | `PriorPlanOnly` | `Dormant { … }` | motif vrai au lieu de motif faux |
| #2912 | `NoGroomedPlan` | `Dormant { … }` | **sort de la relève à grommer** — correct, aucun groom ne réveille un dormeur |
| #2913 | `NoGroomedPlan` | `Dormant { … }` | idem |

**Aucun candidat aujourd'hui admissible ne devient refusé.** Le vol renomme trois refus et ne
retire **aucune** admission. C'est la mesure qui autorise à poser le bras sans crainte de
tarir le bassin — et c'est aussi la mesure qui dit que `POOL_MIN = 2` n'est pas menacé.

---

## 2. Les décisions, et laquelle de chaque alternative tombe

### 2.a — Le marqueur est la ligne de corps, **pas** `docs/dormeurs.md`

Reprise du plan antérieur, et la mesure la confirme. Le contrat d'entrée du registre
(`docs/dormeurs.md` § *Contrat d'une entrée*) dit : « Quand la condition est remplie, **rouvrir
le ticket GitHub cité** … et retirer la ligne d'ici. » Un ticket listé au registre est donc
**fermé**, et les treize entrées actuelles le sont — toutes déjà écartées par le filtre
silencieux `state != IssueState::Open`, en amont de tout refus nommé. Lire le fichier
n'ajouterait aucun refus et **introduirait une lecture de disque dans un cycle qui n'en a
aucune** : `well_formed_candidates` est pure, et c'est ce qui la rend testable sans fixture
de système de fichiers.

La dormance d'un ticket **ouvert** vit donc uniquement dans son corps. Les cinq tickets
mesurés au § 1.b en sont la preuve : aucun des cinq n'est au registre, et les cinq dorment.

### 2.b — Le lecteur vit dans `pool.rs`, pas dans un module neuf, pas dans `plan_callout.rs`

Trois candidats, et le choix se fait sur « où est l'écrivain ».

- **`plan_callout.rs`** — non. Ce module existe parce qu'une grammaire y a un **écrivain** dans
  le dépôt (`rewrite_callout_as_prior_plan`) et un lecteur, et que le test d'aller-retour
  `mika2952_la_ligne_ecrite_par_la_porte_est_relue_par_le_lecteur` les tient ensemble. La ligne
  `Condition de réveil` **n'a pas d'écrivain** : des humains l'écrivent à la main, en trois
  orthographes, et c'est précisément pourquoi le § 1.b existe. Y poser la grammaire ferait croire
  à une symétrie qui n'est pas là.
- **un `dormancy.rs` neuf** — non, et c'est un renoncement assumé. Un module porte son en-tête
  `//!`, son enregistrement dans `mod.rs`, et ses `use` croisés : une vingtaine de lignes pour
  une grammaire d'un seul consommateur. Le coût est réel (cf. § *Taille estimée*) et le bénéfice
  — l'isolement — est déjà obtenu par la visibilité.
- **`pool.rs`** — oui. Le bassin est l'**unique** consommateur de la lecture, la grammaire est
  `pub(crate)` et non `pub`, et `reader.rs` l'appelle comme il appelle déjà `plan_size`. Si un
  second consommateur apparaît, le déplacement est mécanique et le plan de ce jour-là le dira.

### 2.c — Le type est un enum à deux variants dans un `Option`, et **jamais** un `Option<String>`

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DormancyReading {
    /// Le corps porte une ligne `Condition de réveil` : le travail est dû, sa
    /// condition d'exécution ne l'est pas. `condition` est la condition **citée**,
    /// jamais interprétée (§ 2.e).
    Dormant { condition: String },
    /// Le corps a été lu et ne porte pas le marqueur. **Preuve positive seule** :
    /// `Awake` ne dit pas « le travail est à faire », il dit « rien ici ne dit
    /// qu'il dort » — et le candidat continue vers les portes suivantes.
    Awake,
}
```

porté par `dormancy: Option<DormancyReading>` avec `#[serde(default)]`, exactement comme
`plan_reading` au vol 1. Les trois états et leur sens, qui est le motif mika#2340/#2331 :

- `None` — **ce binaire n'a pas lu le corps.** Un blob du sink hors-ligne écrit avant que le
  champ existe se relit ici, et `None` ne veut dire ni « éveillé » ni « zéro ».
- `Some(Awake)` — lu, négatif.
- `Some(Dormant { .. })` — lu, positif, avec la condition.

Ce qui tombe : `Option<Option<String>>`, dont les trois états existent mais dont aucun ne porte
son nom au site de lecture ; et un cinquième variant d'un enum fusionné avec `PlanReading`, que
le plan antérieur a déjà écarté — « **champ orthogonal, pas un variant de plus** » — parce que
la dormance et la lecture de plan sont deux axes qu'un ticket peut porter **ensemble** (#2854
les porte tous les deux), et que la classe mika#2205 dit que deux populations sous un nom ne
sont pas soustrayables.

Le champ se justifie sur `PoolCandidate` par l'argument exact que le vol 1 a écrit pour
`plan_reading` (doc de `PoolCandidate`, ll. 123-140) : c'est **une lecture du corps**, et les
deux lectures d'issues demandent déjà `--json body`. Contrairement à `pr_state` / `ci_state` /
`branch_present`, elle ne coûte **aucun appel `gh` supplémentaire** et elle est disponible sur
**les deux** chemins de composition. Le budget d'appels du cycle reste à 7.

### 2.d — Ordre des bras : `OperatorHeld` **avant** la dormance, dormance **avant** `PriorPlanOnly`

`AdmissionRefusal` ne porte qu'**un** motif. L'ordre des bras **est** donc la priorité du motif,
et chaque frontière se justifie séparément.

1. **`OperatorHeld` reste premier.** Le plan antérieur avait écrit « la **dormance d'abord** » —
   la mesure du § 1.c dit pourquoi cette phrase, prise à la lettre, aurait été un dégât : #2117
   et #2472 portent **tous les deux** `blocked` **et** une condition de réveil. Mettre la
   dormance devant changerait silencieusement leur motif, sans que personne l'ait demandé, et
   dans le mauvais sens : un label de forge est une déclaration **explicite** de l'opérateur,
   une ligne de prose en est une **dérivée**. La déclaration gagne. Après ce vol, le bras de
   dormance est donc **inerte** sur ces deux tickets, et c'est le résultat voulu.
2. **La dormance passe avant `PriorPlanOnly`.** Là, la phrase du plan antérieur tient, avec sa
   raison : « Le nommer `PriorPlanOnly` enverrait l'opérateur chercher une mesure que
   l'opérateur a lui-même déclarée bloquée. » C'est le cas #2854 du § 1.a.
3. **La dormance passe avant `NoGroomedPlan`.** Cas #2912/#2913. Un dormeur sans plan ne manque
   pas d'un plan : il manque de sa condition. Le placer en `NoGroomedPlan` le fait entrer dans
   `relief::name_relief`, qui propose à l'opérateur de grommer quelque chose qu'aucun groom ne
   débloque — l'instrument donne un conseil faux, pas un conseil imprécis.

Ordre final dans `well_formed_candidates`, après les filtres silencieux :
`operator_held_label` → **dormance** → `plan_reading == Some(PriorOnly)` → `!plan_present`.

### 2.e — La condition est **citée**, jamais interprétée, et sa citation porte un budget

`Dormant { condition }` porte le **texte de la ligne**, normalisé des décorations de tête
(§ 1.b) et tronqué à un budget. Il ne porte **pas** un verdict « remplie / non remplie ».

Pourquoi : le contrat du registre dit qu'une condition est valable « quand un lecteur peut dire,
**sans contexte**, si elle est remplie » — et ce lecteur est un **humain ou une commande**, pas
`well_formed_candidates`. Les conditions mesurées contiennent `git rev-list --count …`,
`gh issue list --state open …`, « le 2026-11-16 au plus tard », une variable d'environnement.
Les évaluer demanderait un sous-processus, un appel réseau et une horloge dans une fonction pure.
Le bassin **rapporte** la condition ; il ne l'arbitre pas.

Le budget de troncature suit la forme que le vol 1 a posée pour `PRIOR_PLAN_LINE_BUDGET` —
**un budget, pas une borne** : la condition de #2609 fait plusieurs centaines de caractères, et
un rapport de sink n'a pas à en porter le corps entier. `DORMANCY_CONDITION_BUDGET: usize = 200`,
troncature **sur frontière de caractère** (jamais un `&s[..n]` d'octets — garde `byte-slice-lint`
de mika#764), avec un suffixe `…` qui dit à l'opérateur que la ligne est coupée et qu'il doit
ouvrir le ticket.

### 2.f — Marqueur absent ⇒ le candidat **continue**

Reprise littérale du plan antérieur : « **Marqueur absent ⇒ non dormant**, et le candidat
**continue** vers les portes suivantes. Le prédicat est à preuve positive seule. » C'est la
seconde moitié d'AC2 — « il ne doit pas l'exclure en silence » — et elle est tenue par un test
nommé, pas par une lecture du code.

---

## 3. Les livrables

### 3.a — `crates/mika-agent/src/milestone_manager/pool.rs` — la grammaire

```rust
/// Le mot qui ouvre une ligne de condition de réveil, décorations retirées.
///
/// Mesuré le 2026-10-29 sur les tickets ouverts du dépôt : cinq le portent, en
/// **trois** orthographes — `> - **Condition de réveil :**` (#2854),
/// `> **Condition de réveil :**` (#2472, #2117) et `**Condition de réveil : …**`
/// (#2912, #2913), cette dernière sans blockquote et le deux-points **dans**
/// l'emphase. D'où un test sur le mot seul, après normalisation des décorations :
/// ni le deux-points ni l'emphase fermante n'en font partie.
pub(crate) const DORMANCY_MARKER: &str = "Condition de réveil";
```

et le lecteur, qui normalise puis teste :

```rust
pub(crate) fn dormancy_reading(body: &str) -> DormancyReading { … }
```

La normalisation retire, en tête de ligne et de façon répétée, les espaces, `>`, `-`, `*` et `_`,
puis compare sans tenir compte de la casse du premier caractère. Elle est **inline** dans le
lecteur : huit lignes de corps, un seul appelant, aucun second site à tenir en cohérence.

Puis : le champ `dormancy` sur `PoolCandidate` avec sa doc ; sa propagation dans
`impl From<&SubIssue> for PoolCandidate` ; le variant `RefusalReason::Dormant { condition: String }` —
un variant **porteur**, contrairement à `PriorPlanOnly` que le vol 1 a laissé unitaire, et la doc
dira pourquoi la dissymétrie est voulue : `PriorPlanOnly` renvoie à un plan que le rapport cite
déjà ailleurs, la condition de réveil n'est citée nulle part ailleurs ; le bras de `label()` ;
le bras de `remediable_by_grooming()` rendant **`false`** ; le bras d'admission avec le
commentaire de précédence du § 2.d ; et un § au `//!` du module remplaçant l'annonce « livré par
le vol 2/2 de mika#2952 » (l. 74-92) par ce que le module fait désormais.

Les deux `match` de `RefusalReason` sont **exhaustifs et sans bras `_`** — patron mika#2923. Le
cinquième variant ne compile pas sans que son auteur ait répondu « est-ce qu'un groom débloque
ça ? », et c'est la propriété qu'il ne faut pas perdre.

### 3.b — `crates/mika-agent/src/milestone_manager/types.rs`

`SubIssue.dormancy: Option<DormancyReading>` avec `#[serde(default)]`, à côté de `plan_reading`
(l. 153). Sans lui, `PoolCandidate::from(&SubIssue)` ne peut rien propager sur le chemin
milestone — qui est **celui de #2854** (milestone 33) — et le vol serait inerte là où le ticket
le mesure.

`is_in_governed_progress` (l. 204) **n'est pas touché**. La dormance n'est pas un progrès ; c'est
l'absence de condition d'exécution. Les toucher ensemble mélangerait deux populations.

### 3.c — `crates/mika-agent/src/milestone_manager/reader.rs` — les trois sites d'alimentation

Les mêmes trois endroits que le vol 1, dans le même geste :

- `compose_outside_candidates` — à côté de `plan_reading: Some(…)` (l. 647) ;
- `compose_from_gh_outputs` — les deux constructions (ll. 962 et 1028).

`reader::plan_callout_present` **reste intact**, comme au vol 1 : mika#2608 l'a ratifié et pinné
avec deux tests, et la dormance est un axe de plus, pas un resserrement de celui-là.

### 3.d — `crates/mika-agent/src/milestone_manager/relief.rs` — la garde

Aucun changement de code : le filtre `.filter(|r| r.reason.remediable_by_grooming())` (l. 101)
fait déjà le travail dès que le bras rend `false`. Le livrable est **un test**, voisin de celui
que le vol 1 a posé vers la l. 444 : un dormeur refusé n'apparaît pas dans la relève à grommer.
Une garde sans test est une garde que le prochain bras d'enum peut inverser en silence.

### 3.e — `reporter.rs` — une **propriété à vérifier**, pas un livrable

Le rapport écrit déjà `"- Refusé : #{} — {}"` avec `r.reason.label()` (ll. 388-391). Un nouveau
motif s'y affiche **sans édition**. Le vol ne touche pas le fichier, et la vérification est
`git diff --stat` : si `reporter.rs` y apparaît, c'est que `label()` n'a pas suffi, et il faut
comprendre pourquoi avant de commiter.

### 3.f — Le churn de constructeurs

`assessor.rs`, `cadence.rs`, `evaluator.rs`, `spawn.rs` et
`tests/manager_delivery_observability_2267.rs` construisent des `SubIssue` ou des
`PoolCandidate` littéraux. Chacun prend `dormancy: None` — une ligne. Mesure du vol 1 : huit
lignes au total sur ces cinq fichiers.

### 3.g — Les tests

Six, et chacun ferme un énoncé nommé :

1. **Le corpus mesuré, en table.** Les trois orthographes du § 1.b rendent
   `Dormant`, chacune avec sa condition citée ; un corps sans marqueur rend `Awake`. Un seul
   test paramétré, parce que les quatre cas interrogent la même frontière — la normalisation —
   et qu'un test par orthographe la ré-affirmerait trois fois.
2. **Un dormant n'est pas admis et son refus cite la condition.** AC2, moitié positive.
3. **Marqueur absent ⇒ le candidat continue.** AC2, moitié négative : un candidat avec plan
   vivant et sans marqueur reste dans `eligible`, et aucun `AdmissionRefusal` ne le nomme.
4. **`OperatorHeld` mord avant la dormance.** Le cas #2117/#2472 du § 1.c : un corps dormant
   **plus** un label `blocked` rend `OperatorHeld`, pas `Dormant`.
5. **La dormance mord avant `PriorPlanOnly` et avant `NoGroomedPlan`.** Les cas #2854 et
   #2912/#2913, dans le même test parce que c'est la même frontière lue deux fois.
6. **Un blob antérieur se relit, et `None` sérialise en `null`.** Aller-retour `serde` sur un
   `SubIssue` sans le champ, sur le modèle du test du vol 1 (ll. 994-1029) : `dormancy: None`,
   et les champs du vol 1 survivent.

Plus le test de relève du § 3.d, soit sept assertions nommées.

---

## 4. Ordre de commit

Trois commits, et l'ordre est choisi pour que le cœur atterrisse avant la fin de pipeline —
leçon des trois vols morts au plafond de mika#2683/#2654/#2687, dont le dernier commit de travail
tombait aux tours 171, 177 et 136.

1. **`DormancyReading`, le marqueur, le lecteur et son test de corpus** (§ 3.a grammaire, § 3.g
   test 1). Rien ne consulte encore la lecture. Contrôle : la suite entière verte, aucun verdict
   changé.
2. **Le champ voyage et s'alimente** (§ 3.b, § 3.c, § 3.f, § 3.g test 6). Toujours aucun jugement.
   Contrôle : `cargo test -p mika-agent milestone_manager` verte **sans** qu'un test de verdict
   existant ait eu besoin d'être édité — si l'un l'a été, le champ n'était pas orthogonal.
3. **Le motif et son bras** (§ 3.a jugement, § 3.d, § 3.g tests 2-5). C'est le commit qui change
   le comportement, et le § 1.c dit exactement lequel : trois refus renommés, zéro admission
   retirée.

## 5. Vérification

```
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test -p mika-agent milestone_manager
cargo test -p mika-agent plan_callout
cargo test
scripts/check-byte-slices.sh
scripts/verify-pipeline.sh origin/main
git diff --stat origin/main -- crates/mika-agent/src/milestone_manager/reporter.rs
```

La dernière ligne doit rendre **vide** (§ 3.e).

## 6. AC4 — la sonde après déploiement

AC4 porte sur les **quatre** tickets du rapport et se mesure **après** `make deploy`, sur le
premier rapport du sink du milestone 33. Elle ne coûte aucune ligne de code et ne peut pas être
fermée dans la PR : elle est la seule AC de ce ticket dont le juge est un artefact de production.

Attendu, motif par motif — mesuré sur `origin/main` au 2026-10-29, donc falsifiable :

| ticket | mesure du corps | motif attendu |
|---|---|---|
| #2943 | `plan_strict: 0`, `prior: 1`, marqueur: 0 | `PriorPlanOnly` (vol 1, déjà livré) |
| #2946 | `plan_strict: 0`, `prior: 1`, marqueur: 0 | `PriorPlanOnly` (vol 1, déjà livré) |
| #2680 | `plan_strict: 0`, `prior: 1`, marqueur: 0 | `PriorPlanOnly` (vol 1, déjà livré) |
| #2854 | `plan_strict: 0`, `prior: 2`, marqueur: **1** | `Dormant` citant `MIKA_WEBHOOK_COMMENT_TRIAGE` |

Aucun des quatre n'apparaît en « à admettre », et les quatre sont **nommés avec leur motif** sur
une ligne `- Refusé : #N — …`. Un ticket qui disparaîtrait du rapport sans ligne de refus ne
ferme pas AC4 : l'exclusion silencieuse est précisément ce que le ticket reproche au bassin.

Si la sonde contredit la table, la lecture se fait dans cet ordre : (1) le binaire déployé
embarque-t-il le commit (`mika agents budget`, § *Deploying a bundled-skill change* du
`CLAUDE.md` pour le motif général d'une projection stale) ; (2) le corps du ticket a-t-il changé
depuis la mesure — un groom ou un merge peut y avoir réécrit le callout ; (3) alors seulement le
code.

## Definition of Done

- `DormancyReading` et `dormancy_reading` existent dans `pool.rs`, et les **trois** orthographes
  mesurées au § 1.b rendent `Dormant` avec leur condition citée.
- `dormancy: Option<DormancyReading>` avec `#[serde(default)]` est porté par `SubIssue` et par
  `PoolCandidate`, propagé par `From<&SubIssue>`, alimenté aux **trois** sites de composition de
  `reader.rs`.
- `RefusalReason::Dormant { condition }` existe, rend un `label()` qui cite la condition, et
  rend `remediable_by_grooming() == false`. Les deux `match` restent exhaustifs, sans bras `_`.
- Le bras de dormance est placé **après** `operator_held_label` et **avant** le bras
  `PriorPlanOnly`, avec le commentaire qui dit pourquoi (§ 2.d).
- Un candidat sans marqueur **continue** vers les portes suivantes, et un test le nomme.
- Un dormeur refusé n'entre pas dans `relief::name_relief`, et un test le nomme.
- `reader::plan_callout_present` et `is_in_governed_progress` sont inchangés.
- `reporter.rs` est inchangé, et le `git diff --stat` du § 5 l'atteste.
- Un blob du sink écrit avant ce vol se relit avec `dormancy: None`, et `None` sérialise en
  `"dormancy":null`.
- `cargo test`, `cargo clippy -D warnings`, `scripts/check-byte-slices.sh` et
  `scripts/verify-pipeline.sh origin/main` passent.
- Le corps de PR porte `Closes senara-solutions/mika#2952` et **nomme AC4 comme mesurée après
  déploiement**, avec la table du § 6 — pas une promesse, une attente falsifiable.

## Acceptance criteria

- **AC1.** Un candidat ouvert dont le travail de vol est livré (PR mergées) et dont ne reste qu'une sonde, ou une mesure, n'est pas admis. Il porte un refus **nommé**, distinct de `NoGroomedPlan` : il ne doit pas entrer dans la relève à grommer (`relief::name_relief`), puisqu'aucun groom ne le débloque. Le plan tranche comment « vol livré » se lit.
- **AC2.** Un candidat dormant, qui porte une condition de réveil (règle « Dormeurs visibles », ratifiée par Vincent le 2026-09-01), n'est pas admis. Il porte un refus nommé qui cite la condition. Le plan dit par quel marqueur la dormance se lit, et ce que le bassin fait d'un ticket dont le marqueur est absent : il ne doit pas l'exclure en silence.
- **AC3.** Contrôle négatif dans le même test. Un ticket ouvert avec un plan groomé, sans PR mergée et sans dormance, reste admis. Un ticket dont une **seule** PR de vol est mergée sur un découpage en N vols, avec un vol suivant dû, reste admissible. Le prédicat ne doit pas tomber sur « au moins une PR mergée ». Le plan dit comment il distingue « vol 1/2 livré » de « tout livré ».
- **AC4.** Sonde après déploiement. Le premier rapport du sink sous le nouveau binaire ne propose plus #2943, #2946, #2680 ni #2854 en « à admettre », et les nomme avec leur motif.

**Répartition sur les deux vols.** AC1 et AC3 sont fermées par le vol 1/2 (`449006f6b`, PR #2954).
AC2 et AC4 sont fermées par ce vol. AC3 reste **vérifiée** ici au titre de non-régression : les
tests du vol 1 qui la tiennent doivent rester verts sans édition, et le § 4 commit 2 en fait un
contrôle explicite.

## Fire-Disposition

Ce plan livre des détecteurs : sept assertions dont le chemin de succès est « aucun candidat mal
formé admis ». Option **(a) — exception nommée en allowlist, allowlist déclarée et vide.**

Précédents de la forme : mika#2425, mika#1833, et l'allowlist `MIKA2952_CORPUS_EXCEPTIONS` que le
vol 1 de ce même ticket a posée vide.

**Pourquoi vide.** Le § 1.c est un recensement exhaustif, pas un échantillon : la population
dormante des tickets **ouverts** est de cinq, et pour chacun le motif attendu après ce vol est
nommé. Aucune violation préexistante ne reste donc à excepter. Une allowlist non vide ici
voudrait dire qu'un des cinq ne se comporte pas comme la table le dit — et ce serait une erreur
de mesure à corriger, pas une exception à inscrire.

**Détail d'implémentation.**

```rust
/// Corps de la population dormante qui **échappent** au prédicat du § 1.b.
///
/// Vide au 2026-10-29 : le recensement des tickets ouverts porteurs du marqueur
/// rend exactement cinq corps (#2117, #2472, #2854, #2912, #2913), et les trois
/// orthographes qu'ils emploient sont toutes couvertes par le test de corpus.
/// Une entrée ici nomme le corps précis, la raison, et son ticket de suivi.
const MIKA2952_DORMANCY_EXCEPTIONS: &[(&str, &str)] = &[];
```

**Assertion auto-nettoyante.** Un test voisin affirme que chaque entrée de l'allowlist est
**encore** en échec sur le prédicat d'aujourd'hui :

```rust
#[test]
fn mika2952_une_exception_de_dormance_qui_passe_est_une_exception_morte() { … }
```

Une entrée dont le corps rend désormais `Dormant` fait **rougir** le test. Le geste qui rend
l'exception inutile est donc le geste qui la fait expirer — c'est ce qui sépare cette allowlist de
celle que mika#2309 condamne. Sur une allowlist vide, le test est vert et boucle zéro fois : il
existe pour le jour où quelqu'un y écrit une ligne, et le coût de cette existence est de six
lignes.

**Suivi.** Aucun ticket de suivi n'est dû tant que l'allowlist est vide. La première entrée en
ouvre un, et la doc du const le dit.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `pool.rs` — marqueur, `DormancyReading`, lecteur + normalisation inline (§ 3.a) | 77 |
| `pool.rs` — champ, `From`, variant `Dormant`, `label()`, `remediable_by_grooming()` (§ 3.a) | 43 |
| `pool.rs` — bras d'admission + commentaire de précédence, § du `//!` (§ 3.a, § 2.d) | 34 |
| `pool.rs` — allowlist vide + assertion auto-nettoyante (Fire-Disposition) | 24 |
| `pool.rs` — tests 1 à 5 (§ 3.g) | 112 |
| `types.rs` — champ + doc (§ 3.b) | 12 |
| `types.rs` — test d'aller-retour `serde`, test 6 (§ 3.g) | 26 |
| `reader.rs` — trois sites + doc du geste (§ 3.c) | 14 |
| `relief.rs` — test de garde (§ 3.d) | 28 |
| churn de constructeurs : `assessor.rs`, `cadence.rs`, `evaluator.rs`, `spawn.rs`, `manager_delivery_observability_2267.rs` (§ 3.f) | 8 |
| `reporter.rs` (§ 3.e) | 0 |

Total estimé : 378 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient**, et la raison est mesurée sur ce ticket même, pas postulée :
le vol 1/2 a annoncé 415 lignes estimées et coûté **746 insertions réelles sur 12 fichiers**
(`git diff --stat 8e9ab6c24 449006f6b -- crates/`), sous ce même plafond de 200 tours, et il est
passé. Ce vol est structurellement **plus petit** que son prédécesseur sur les trois postes qui
avaient dominé son coût : pas de grammaire partagée entre un écrivain et un lecteur (§ 2.b),
donc pas de test d'aller-retour écrivain/lecteur ; pas de rectification du doc-comment de la
porte d'auto-pull ; et `reporter.rs` à zéro au lieu de +9. Son plus gros fichier,
`pool.rs`, pèse 290 lignes au vol 1 contre ≈ 290 ici — à parité, et non en croissance.

**La marge est de 51 lignes, soit serrée.** C'est pourquoi l'ordre de commit du § 4 place le
cœur du comportement au commit 3 et les tests de corpus au commit 1 : si la fin de pipeline
mord, ce qui est déjà commité est fonctionnel et nommé, jamais un bras à moitié posé.

**Ce qui déclencherait un découpage en vols.** Un seul fait : si la normalisation du § 1.b
demandait plus qu'un `trim_start_matches` répété — par exemple si une sixième orthographe
apparaissait, portant le marqueur dans un tableau ou derrière un `<!-- -->`. Dans ce cas, la
grammaire et son corpus partent seuls en vol 2/3, le jugement en vol 3/3, et la frontière est
celle du commit 2 du § 4 : un vol où le champ voyage sans qu'aucun verdict ne change. Elle est
nommée ici pour que la décision ne se prenne pas sous la pression des tours.

## Hors périmètre

- **Lire `pr_state`, `ci_state` ou une sonde de timeline par candidat.** AC1 est fermée, et le
  vol 1 l'a fermée **sans** aucune de ces lectures — par la réécriture du callout. Le budget
  d'appels `gh` du cycle reste à 7, `TIMELINE_PROBE_MAX` à 1.
- **Lire `docs/dormeurs.md`.** § 2.a. Les treize entrées nomment des tickets fermés, déjà
  écartés par `state != Open`, et le fichier introduirait une lecture de disque dans une
  fonction pure.
- **Évaluer la condition de réveil.** § 2.e. Le bassin cite ; il n'arbitre pas.
- **Toucher `reader::plan_callout_present`.** mika#2608 l'a ratifié et pinné. La dormance est un
  axe de plus.
- **Ouvrir `MIKA_MANAGER_POOL_WRITE`.** mika#2699 en est propriétaire. Ce vol retire une
  objection à son ouverture ; il ne l'ouvre pas.
- **Retirer un ticket du registre des dormeurs, ou en ajouter un.** Le réveil a sa propre
  procédure, opérateur, décrite par `docs/dormeurs.md`.
