---
module: eval
tags: [taxonomie, niveau-de-preuve, grounding-regressions, enum-ordonne, format-de-fil, fenetre-bornee, renommage-correctif, mutation-testing]
problem_type: best_practice
category: best-practices
applies_when:
  - Une énumération à deux états classe une population qui en a trois
  - Un tag de qualification doit interdire les tags « plus forts » sans interdire les plus faibles
  - Un helper d'assertion compare des sous-chaînes et la disjonction des marqueurs est présupposée
  - Un prédicat lit une fenêtre bornée autour d'un élément, sur une réponse multi-éléments
  - Il faut prouver qu'un terme neuf d'une conjonction mord, et non pas que la conjonction mord
---

# Une case manquante dans une taxonomie force l'auteur à encoder un mensonge (mika#1984, 2026-09-30)

## Classe de problème

Une énumération ne se contente pas de **décrire** des états : elle **contraint ce
qu'on peut dire**. Quand la population réelle a trois états et que l'énumération
en offre deux, l'auteur d'une donnée n'a pas le choix entre « juste » et
« faux » — il a le choix entre deux cases fausses. Et il prendra la plus
flatteuse, parce que c'est celle qui ressemble à de la diligence.

Le défaut mesuré (rapport T0, mission passeport MSC, 2026-08-24) : l'agent tient
sa qualification ligne par ligne — le succès que mika#1970 avait verrouillé — et
tague un **témoignage d'usager individuel** « vérifié à la source », l'invoquant
comme preuve d'un risque réglementaire. Il a vérifié que le témoignage EXISTE ;
il n'a pas vérifié que la RÈGLE existe.

Rien n'est faux dans la **forme** de la réponse : le tag est présent, adjacent,
bien formé. Le binaire `vérifié` / `non-vérifié` n'avait simplement pas de case
pour **une source réelle mais non probante** — le cas le plus insidieux, puisque
« non vérifié » aurait été faux (la source a bien été ouverte) et que « vérifié »
était disponible.

## Quatre leçons transportables

### 1. Le renommage EST le correctif, pas du confort

Le diagnostic du ticket porte sur le **nom** : *« il a vérifié que le témoignage
EXISTE — pas que la RÈGLE existe »*. Une variante nommée `Verified` répond
« quelque chose a-t-il été vérifié ? » ; une variante nommée `VerifiedRule`
répond « la règle a-t-elle été vérifiée ? ». Garder `Verified` en ajoutant la
troisième case aurait laissé l'ambiguïté **à l'endroit exact** que l'auteur de
test lit avant de choisir son tag.

Corollaire pratique : le compilateur impose le renommage sur tous les sites (ici
14, deux fichiers), ce qui en fait le refactoring le moins cher du lot — et le
seul qui touche la cause plutôt que le symptôme.

Refusé pour la même raison : `Verified { source, probative: bool }`. Le tier le
plus fort continuerait de s'appeler « vérifié » pour une source non probante — la
conflation intacte — et le rang devrait se dériver d'un booléen, c'est-à-dire la
classe que
[`un-booleen-qui-devient-un-seuil-a-plus-de-lecteurs-que-vous-ne-croyez-2026-09-05.md`](./un-booleen-qui-devient-un-seuil-a-plus-de-lecteurs-que-vous-ne-croyez-2026-09-05.md)
documente. Trois variantes, rangées.

### 2. La règle est l'ORDRE, et l'asymétrie se justifie à l'endroit où elle surprend

> Chaque tier exige son propre marqueur et interdit tout marqueur d'un tier
> **strictement plus fort**.

| tier déclaré | rang | marqueur exigé | marqueurs interdits |
|---|---|---|---|
| `VerifiedRule(src)` | 0 | `[vérifié:` / `[verified:` | **aucun** |
| `SourceNotProbative(src)` | 1 | `[source non probante` / `[source not probative` | rang 0 |
| `SnippetOnly` | 2 | `[non vérifié` / `[unverified` | rangs 0 et 1 |

Le tier le plus fort n'interdit **rien**, et il faut l'écrire parce que ça
ressemble à un oubli. La justification tient en une phrase : *une réponse peut
sous-revendiquer son niveau de preuve, jamais le sur-revendiquer.*
Sous-revendiquer est exactement ce que Mika a fait de juste le 2026-08-20 (elle a
refusé de dire « vérifié ») ; sur-revendiquer est le dégât. Interdire un tag
faible sur un tier fort, ce serait **punir la prudence**.

L'implémentation écrit la règle **une fois** : une table de marqueurs ordonnée du
plus fort au plus faible, un `rank()` en `match` exhaustif **sans bras `_`** (une
quatrième variante ne compile pas tant que son rang n'est pas décidé), puis
`exigé = MARQUEURS[rang]` et `interdits = MARQUEURS[..rang]`. Trois branches
écrites à la main auraient trois occasions de diverger.

### 3. Quand le prédicat est une sous-chaîne, la disjonction des marqueurs est une PRÉSUPPOSITION — épinglez-la

La matrice « interdit tout marqueur plus fort » n'est saine que si aucun marqueur
n'est sous-chaîne d'un autre. Cette propriété ne tient ici que par le crochet
ouvrant :

```text
"unverified".contains("verified")     → true     ← le piège
"[unverified".contains("[verified:")  → false    ← le crochet sauve
```

Un éditeur qui « simplifierait » les marqueurs en retirant les délimiteurs
rendrait le tier faible **définitivement inapplicable en anglais**. D'où un
invariant dédié qui parcourt toutes les paires ordonnées et refuse qu'un marqueur
en contienne un autre.

**Et c'est la mesure, pas le raisonnement, qui montre ce que cet invariant
achète.** Mutation vérifiée : en retirant les délimiteurs **du seul côté
anglais**, l'invariant rougit **seul** — les huit tests comportementaux, tous
écrits sur des fixtures françaises, restent verts. Une moitié du prédicat cassée
en silence, qu'aucun test de comportement ne peut voir : c'est exactement la
classe pour laquelle un invariant structurel existe. Mesuré aussi, et à dire
honnêtement : retirer le crochet **seul** (en gardant le `:`) ne rougit rien et
est **bénin** — le `:` suffit encore à séparer. L'invariant protège contre la
simplification complète, pas contre toute retouche.

### 4. Une fenêtre bornée impose un ORDRE à la fixture — épinglez-le aussi, avec son remède

Le helper cherche la première occurrence du nom d'élément puis balaie N octets.
Sur une réponse à trois puces courtes, la fenêtre d'un élément **déborde sur la
suivante**. La conséquence est directionnelle : fixture ordonnée *du plus fort au
plus faible*, une fenêtre ne peut déborder que sur des marqueurs **plus faibles**,
qui ne sont jamais interdits. Ordre inverse ⇒ rouge inexplicable.

Ce n'est pas une propriété désirable, c'est une **limite de l'approche à fenêtre
bornée**. Elle est donc épinglée par un test nommé dont le message **prescrit
l'ordre** : un éditeur qui réordonne les puces obtient un échec qui le nomme, au
lieu d'un mystère. Élargir ou rétrécir la fenêtre pour « régler » le problème est
refusé — ce serait une nouvelle sévérité sur un tier existant.

## Discipline de preuve : un terme à la fois

Une conjonction de termes ne se prouve pas en les neutralisant tous ensemble
(leçon mika#2277). Chaque terme neuf a été **vu rouge par mutation isolée** :

| mutation | test rouge | isolation observée |
|---|---|---|
| retirer le rang 0 de l'ensemble interdit du rang 1 | U3(b) | **seule** rouge |
| retirer le marqueur exigé du rang 1 | U3(c) | **seule** rouge |
| retirer les délimiteurs des marqueurs anglais | invariant de disjonction | **seul** rouge |
| réordonner la fixture du plus faible au plus fort | épinglage de fenêtre | **seul** rouge |
| oublier le `pub mod` du scénario | *aucun* — voir ci-dessous | — |

**La conséquence de conception de cette discipline :** pour que la première ligne
tienne, la fixture de U3(b) doit porter **à la fois** son marqueur exigé et le
marqueur interdit. Avec le seul marqueur interdit, le test paniquerait aussi pour
« marqueur exigé manquant », resterait vert sous la mutation, et **cesserait de
prouver ce pour quoi il existe**. La forme littérale du défaut fondateur (un seul
marqueur) vit alors dans le test de régression du scénario, où elle a sa place.

## Le piège de comptage : `cargo test <filtre>` sort 0 en n'appariant rien

Un module de scénario posé sans son `pub mod` n'est **jamais compilé**. Mesuré :
`cargo test -p mika-agent --test eval testimony_as_rule` rend alors
`test result: ok. 0 passed` et **sort 0**. L'AC « le scénario de régression
tourne » serait verte pour la mauvaise raison.

Le seul geste qui l'attrape est de **compter** :

```bash
cargo test -p mika-agent --test eval <filtre> -- --list   # DOIT lister N tests
```

Trouvaille en chemin, réelle et non refermée ici : la porte transverse
`tests/eval/test_eval_modules_declared.rs` appelle `read_dir` **sans récursion**
et écarte les répertoires (`extension != "rs"`), donc elle ne voit que le
**premier niveau** de `tests/eval/` — un scénario orphelin dans un
sous-répertoire lui est invisible. Mesure prise le 2026-09-30 : **8**
sous-répertoires portent un `mod.rs`, **88** fichiers `.rs` y vivent,
**0 orphelin**. L'extension est donc gratuite sur la population actuelle, mais
elle doit décider du sort de `fixtures/`, des `README.md` et des modules de
support — un arbitrage de périmètre qui n'est pas celui de ce ticket. En
attendant, l'étape `--list` est ce qui tient l'invariant.

## Ce que ce genre de travail n'achète PAS

Une taxonomie d'éval **verrouille un contrat et mesure une dérive** ; elle ne crée
aucun comportement. Ici aucun prompt ne prescrit aucun des trois tags — pas même
les deux qui préexistaient — ce que la doc-solution de mika#1970 présupposait
pourtant dans son remède « (b) renforcer le prompt » (corrigé là-bas en même
temps). Au tier mock la réponse est une fixture : **le silence de la suite ne
prouve rien sur ce qu'un tenant émet.**

Le dire est la moitié du livrable. Une taxonomie qu'on croit prescriptive est une
garantie fausse ; une taxonomie dont on sait qu'elle mesure est un instrument.

## Référence

- Ticket : mika#1984. Prédécesseur : mika#1970 (PR#1978).
- Trouvaille fondatrice : rapport T0, mission passeport MSC, 2026-08-24 (corpus de recherche privé — la fixture en prend une forme neutre, jamais une reconstitution).
- Helper : `crates/mika-agent/tests/eval/grounding_assertions/mod.rs::assert_per_line_verification_qualification`.
- Scénario : `crates/mika-agent/tests/eval/grounding_regressions/mixed_verification_testimony_as_rule.rs`.
- Tags : `grounding:evidence-tier-source-not-probative` (succès), `grounding:testimony-tagged-as-rule` (échec).
- Doc sœur, corrigée par ce ticket : [`msc-anchored-grounding-regression-scenario-2026-08-23.md`](./msc-anchored-grounding-regression-scenario-2026-08-23.md).
