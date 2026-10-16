---
title: "Une estimation de taille se compare à un seuil exprimé dans sa propre unité, pas dans celle de la mesure"
date: 2026-10-02
category: dev-loop
module: skills/bundled/_shared/dispatch-lib.sh
component: development_workflow
problem_type: workflow_issue
severity: high
applies_when:
  - "un plan groomé porte une section « Taille estimée » et la porte de groom la compare au seuil `_plan_size_max_loc`"
  - "on recalibre `PLAN_SIZE_MAX_LOC` ou le littéral des deux prompts architecte"
  - "un groomeur estime le volume d'un livrable dont la moitié est un harnais shell (`test-dispatch-lib.sh`)"
  - "on recalibre l'un des quatre paramètres de la borne par vol (`_plan_size_per_flight_bound`, mika#2693)"
  - "un vol meurt au plafond de tours alors que son plan passait la borne par PR"
symptoms:
  - "mika#2636 : plan estimé à 530 lignes de code, PR #2648 en porte 1 596 hors `docs/` et CLAUDE.md (×3,0)"
  - "mika#2641, même jour : estimé ≈ 550, livré 1 415 (×2,6), pilote mort au plafond de tours"
  - "la marge que le plan s'accordait (ratio 1,39 mesuré sur mika#1960 phase 2) a été dépassée d'un facteur 2"
  - "2026-10-14 : trois vols (mika#2683, #2654, #2687 vol 1) morts au plafond en PASSANT la borne par PR, aucun n'ayant ouvert sa PR"
  - "le dernier commit de travail tombe 15 à 30 tours avant la mort : c'est la fin de pipeline qui ne tient pas, pas le travail"
root_cause: unit_mismatch_estimate_vs_measured_threshold
tags: [plan-size, taille-estimee, max-turns, groom, estimation, calibration, threshold, plafond-de-tours, measure-before-remedy, borne-par-vol, loop-substrate]
related_components: [mika-arch-groom-ticket, mika-arch-second-review, dev-groom]
ticket: mika#2636
---

# Une estimation de taille se compare à un seuil exprimé dans sa propre unité, pas dans celle de la mesure

## Context

mika#2636 a rendu obligatoire une section « Taille estimée » dans tout plan groomé, et
a posé un seuil : au-delà de **1 000 lignes de code hors `docs/`**, le plan doit être
découpé en phases (`_plan_size_max_loc`, défaut `1000`,
`skills/bundled/_shared/dispatch-lib.sh:644` ; même littéral dans les deux
`system_prompt.md` architecte, tenu par le test S14).

Ce 1 000 vient d'une **mesure de diff** : les pilotes morts au plafond de tours
(mika#2161 ≈ 1 470 lignes, mika#2633 ≈ 1 170) avaient tous livré plus de ~1 000
lignes de code *mesurées*. Mais la porte le compare à un nombre **estimé** par le
groomeur avant toute ligne écrite. Les deux nombres ne sont pas dans la même unité.

Le premier point de données est la PR même qui pose la porte :

| livrable | estimé (plan) | 1er commit d'implémentation | après simplification + revue | ratio final |
|---|---|---|---|---|
| `dispatch-lib.sh` | 275 | 508 | 577 | ×2,1 |
| `test-dispatch-lib.sh` | 205 | 741 | 944 | **×4,6** |
| deux `system_prompt.md` | 50 | 61 | 61 | ×1,2 |
| `executor.rs` | 0 | 0 | 14 | — |
| **total** | **530** | **1 310** | **1 596** | **×3,0** |

Mesuré par `git show --numstat` commit par commit sur la branche
`feat/2636/groom-l-estimation-de-taille-devient-un` (PR #2648), `docs/` et
`CLAUDE.md` exclus comme le ticket le prescrit.

Trois constats sortent de ce tableau :

1. **L'écart est déjà là avant la revue.** Le premier commit d'implémentation pèse
   ×2,5 l'estimation. La simplification et la revue de code (deux P1, trois
   défauts de harnais, trois fixtures de régression) n'ajoutent que +22 %. Ce
   n'est pas la revue qui fait exploser le volume ; c'est l'estimation qui est
   fausse dès le départ.
2. **L'erreur se concentre sur les tests.** Le harnais shell porte des fixtures,
   des contrôles positifs et négatifs par assertion, et la densité de commentaire
   maison. L'estimation de 205 lignes pour « seize assertions + fixtures » valait
   ~13 lignes par assertion prévue ; la livraison en porte 944, soit ~59 par
   assertion prévue (la revue a ajouté des sondes S17, S18 et des bis, ce qui
   n'explique qu'une partie de l'écart : le premier commit portait déjà 741
   lignes de test). Les prompts, eux, sont estimés presque juste.
3. **La marge empruntée ne transférait pas.** Le plan corrigeait son estimation
   par le ratio 716/515 ≈ 1,39 mesuré sur mika#1960 phase 2, et en concluait
   « 737 lignes, toujours sous le seuil ». Un ratio mesuré sur un seul ticket, de
   composition différente, n'est pas un facteur de correction.

Le même jour, mika#2641 (estimé ≈ 550, livré 1 415, ×2,6, mesure MPC du
2026-10-02, PR #2647) est mort au plafond :
n=2 sur l'écart, dans le même sens et du même ordre.

## Guidance

**Un seuil qui gate une estimation doit être exprimé en unités d'estimation.** Si
la ligne de mort est ~1 000 lignes *livrées* et que les estimations sous-comptent
d'un facteur k, la porte qui lit l'estimation doit refuser vers 1 000 / k, pas à
1 000. Avec k ≈ 3 observé sur n=2, un plan estimé à 530 annonce ~1 600 lignes
livrées : au-dessus de la ligne de mort, et la porte le laisse passer.

Concrètement, jusqu'à ce que la distribution V4 (CLAUDE.md, *Sondes
post-déploiement*) soit lue :

- **Ne pas lire « sous 1 000 » comme « sûr ».** Un total estimé au-delà de ~350
  porte le risque de volume que le seuil est censé exclure.
- **Estimer les tests séparément, et par assertion.** Sur `test-dispatch-lib.sh`,
  compter ~60 lignes par assertion prévue (fixture, contrôle positif, contrôle
  négatif, commentaire), pas ~13. Un livrable de test estimé plus petit que le
  code qu'il teste est un signal d'alarme sur ce harnais, pas une économie.
- **Recalibrer par le ratio estimé/livré, pas par la mort.** La ligne
  `plan_size_estimate total_loc=…` (porte 3) est journalisée au dispatch ; la
  croiser avec `--numstat` du diff de la PR rend le facteur k sur la vraie
  distribution. C'est **le seuil** qui bouge selon k (halte 2 du CLAUDE.md), pas
  la mesure.
- **Se méfier d'une estimation qui atterrit juste sous le seuil qu'elle doit
  passer.** Le plan de mika#2636 écrivait que rester sous 1 000 « n'est pas une
  coïncidence de rédaction » : une estimation produite sous la contrainte du
  nombre qu'elle doit battre est ancrée par lui. La porte transforme l'estimation
  en cible, et une cible se rate dans le sens qui arrange.

## Why This Matters

La porte de mika#2636 existe pour qu'un pilote ne meure plus au plafond de tours
(35 à 90 USD par pilote, puis un spawn de reprise). Si l'estimation qu'elle lit
vaut un tiers du volume livré, elle valide précisément les plans qui tuent : deux
cas sur deux le jour de son écriture. Rien dans le code ne le dit — le seuil est
un littéral, l'estimation un entier, et la comparaison `-gt` est correcte. Le
défaut est dans l'unité, invisible à la lecture et aux tests.

La lecture inverse est aussi un piège : conclure que « la porte ne marche pas » et
la désarmer. La porte marche ; c'est son étalon qui est posé dans la mauvaise
unité. Le remède est un facteur mesuré, appliqué au seuil.

## When to Apply

- À la revue d'une section « Taille estimée » par l'architecte (première ou
  seconde passe) : juger le total contre ~1 000 / k, et exiger une ligne de tests
  dimensionnée par assertion.
- À toute révision de `_plan_size_max_loc` ou des littéraux S14 : partir du ratio
  estimé/livré mesuré sur les PR, jamais d'une nouvelle mort isolée.
- Au post-mortem d'un pilote mort au plafond : relever `total_loc` de la ligne
  `plan_size_estimate` et le diff livré, et ajouter le couple à la distribution.

## Examples

Lecture fausse, telle qu'elle figurait au plan de mika#2636 (§ 6) :

```
Total estimé : 530 lignes
Marge : 530 × 1,39 ≈ 737 — toujours sous le seuil, avec de la marge.
```

Lecture juste avec le facteur observé :

```
Total estimé : 530 lignes   (dont tests 205 pour 16 assertions ≈ 13 l/assertion)
Tests re-dimensionnés : 16 × ~60 ≈ 950
Livraison attendue : ~1 300–1 600 → au-dessus de la ligne de mort → découper en phases
```

Livré : 1 596.

## La seconde borne est en TOURS, et elle est par VOL (mika#2693, 2026-10-14)

Douze jours après la pose de la porte, trois vols sont morts au plafond **en
passant** la borne par PR. Mesures prises sur les transcripts de pilote, session
principale seulement, sous-agents exclus :

| ticket | plafond | estimé | réel hors `docs/` | 1er Edit | dernier commit de travail | mort |
|---|---|---|---|---|---|---|
| mika#2683 | 200 | 832 | 1 432 | t39 | t171 | 201, dans `ce-simplify-code` |
| mika#2654 | 200 | 880 | 1 160 | t36 | t177 | 201, dans `ce-simplify-code` |
| mika#2687 (vol 1) | 150 | 443 | 994 | t32 | t136 | 151, dans la vérification finale |

Aucun des trois n'a ouvert sa PR ; dispatch-lib les a sauvés en brouillon
(mika#2689, #2690, #2691) et l'orchestrateur a dû les finir à la main.

**Le travail tenait dans le vol.** Les derniers commits de travail tombent à t171,
t177 et t136 — soit 15 à 30 tours avant la mort. Ce qui n'a pas tenu, c'est la
**fin de pipeline** : simplification, revue, vérification complète, compound,
PR. Et cette fin de pipeline coûte des **tours**, pas des lignes. Un seuil en
lignes ne peut pas la voir, quel que soit son étalonnage : le § précédent corrige
l'*unité* de l'estimation, celui-ci dit qu'il manquait une *seconde dimension*.

La contrainte réelle dépend donc du **plafond du ticket**, qui n'est pas le même
pour tous : 150 par défaut, 200 avec le label `loop-substrate`
(`PILOT_LABEL_TURN_CEILINGS`). mika#2687 l'a montré en négatif — la règle avait
été donnée à la main par commentaire opérateur, et elle supposait 200 alors que
le ticket n'avait pas le label.

### La dérivation, et d'où vient chacun de ses quatre paramètres

```
tours_de_travail = plafond − orientation − fin_de_pipeline
borne_estimée    = tours_de_travail × débit / facteur
```

| paramètre | valeur | provenance |
|---|---|---|
| orientation | 27 tours | première mutation du corpus n=7 : t15, t23, t18, t17, t19, t15, t26 — tout ce qui précède est lecture de plan, de ticket et de code. 27 est pris **juste au-dessus** du max observé (26) |
| fin de pipeline | 30 tours | écart entre la dernière mutation et le dernier tour : 17, 21, 33, 21, **117**, 19, 13. Le 117 est écarté comme aberrant (mika#2724, 197 tours sur un plafond de 200). 30 couvre 5 des 6 restants |
| débit | 9 lignes réelles/tour | réel hors `docs/` divisé par les tours de travail : 12,1 / 33,2 / 24,8 / 14,5 / 15,4 / 12,8 (et 11,3 pour la reprise ✱). 9 est pris **sous** le min des six hors reprise |
| facteur estimé → réel | 3 (prudent) | réel/estimé : 2,86 / 2,21 / 2,29 / 2,64 / 2,33 / 2,37 (et 4,05 pour la reprise ✱). 3 est pris **au-dessus** du max des six hors reprise |

Soit **429** lignes estimées à un plafond de 200, et **279** à 150.

Les valeurs d'origine — 35 / 50 / 7,5 / 2, mesurées le 2026-10-14 sur n=3 vols
**morts** — vivent dans la note datée en fin de document, avec le relevé qui les
a remplacées.

Les quatre paramètres vivent nommés à un seul site,
`skills/bundled/_shared/dispatch-lib.sh`, juste au-dessus de
`_plan_size_per_flight_bound` ; le résolveur les applique, et un test de parité
du type S14 (V4) épingle les littéraux des deux prompts architecte contre eux.
Un `.sh` n'a jamais eu besoin de porter cette justification — ce qu'il doit
porter, c'est de quoi **recalculer**, et c'est pourquoi la dérivation y vit en
commentaire plutôt qu'un nombre nu.

### Arithmétique entière : les deux mises à l'échelle s'annulent

`bash` n'a pas de flottants. 9 lignes/tour et un facteur de 3 se portent donc
par deux entiers **dont le nom dit l'échelle** : `900` lignes pour `100` tours,
et le facteur en pourcentage (`300`). Les deux mises à l'échelle s'annulent
(×1/100 puis ÷1/100), ce qui rend le calcul exact en **une seule** division :

```
borne = (plafond − 27 − 30) × 900 / 300
plafond 200 → 143 × 900 / 300 = 429
plafond 150 →  93 × 900 / 300 = 279
```

Ces deux exemples travaillés vivent aussi en commentaire dans `dispatch-lib.sh`,
et là-bas une sonde les tient : **X2** extrait les deux lignes `plafond <c> → … =
<b>` du bloc et compare chaque `<b>` à ce que rend le résolveur. Un littéral de
prose survit au recalibrage **en mentant**, et il mentirait au seul lecteur qui
le consulte — celui qui s'apprête à redéplacer une constante.

**Qui recalibre le débit à 9 lignes/tour écrit `900`, jamais `9`.** L'annulation
est dite au site précisément pour que le prochain recalibrage ne divise pas la
borne par cent en croyant la corriger.

### Comment recalibrer sur une distribution, et pas sur une mort

n=3 n'est pas une distribution, et les trois points sont biaisés dans le même
sens : les trois pilotes sont morts, donc chacun des quatre paramètres est
mesuré sur une course tronquée. La méthode :

1. Par vol terminé — **mort ou non** —, relever le quadruplet : plafond, total
   estimé du plan, réel hors `docs/` par `git show --numstat`, et les trois tours
   (première mutation, **dernière mutation**, dernier tour).
2. **Le tour se compte sur `[cache] turn <N>`, jamais sur `[done] N turns`.**
   C'est `[cache] turn` qui est l'unité du budget — celle que le plafond borne.
   Le second compteur sur-compte de 7 à 24 % sur ce corpus, et un dénominateur
   sur-compté gonfle l'orientation et rabote le débit, c'est-à-dire déplace les
   quatre paramètres dans le sens où l'on croit être prudent.
3. **L'origine de la queue est la dernière mutation, pas le dernier commit.** En
   régime allégé (mika#2694) l'ordre est vérifier → commit → push → PR : la
   vérification tombe *avant* le dernier commit, qui avale donc une part de la
   queue qu'on prétend mesurer. On prend la dernière `Edit`/`Write` **hors
   `pr-body.md` et `.pilot-scratch/`** — ces deux-là appartiennent à la queue,
   pas au travail, et les compter comme travail la ferait disparaître.
4. Les vols qui ont **fini** sont les seuls à donner une fin de pipeline et un
   débit non tronqués. Un corpus qui ne contient que des morts majore
   l'orientation et minore tout le reste.
5. **Écarter les reprises du débit et du facteur, jamais de l'orientation.** Un
   vol qui reprend un plan déjà écrit un autre jour (plan daté d'avant le vol)
   porte des lignes qu'il n'a pas toutes produites : son débit apparent et son
   facteur estimé→réel sont tous deux faussés vers le haut. Son orientation et
   sa queue, elles, sont de vraies mesures de CE vol. Marquer ces vols ✱ dans le
   relevé, et dire quelle colonne les exclut.
6. Recalculer les quatre paramètres sur le corpus entier, puis la borne. Comme
   au § précédent, **c'est la borne qui bouge**, jamais la mesure.
7. Changer la constante au site unique. V4 tirera sur les deux prompts
   architecte tant qu'ils n'ont pas suivi — c'est exactement son office, un
   `system_prompt.md` étant statique et incapable de lire une constante shell.
   X2 tirera sur les deux exemples travaillés du commentaire.

Le plancher est explicite dans le résolveur : un plafond ≤ 57 (= 27 + 30) donne
un nombre de tours de travail nul ou négatif, et la borne vaut alors **0**, avec
la note `ceiling_below_pipeline_floor`. Jamais un négatif — « un nombre
plausible, présenté avec autorité, faux » est la classe de défaut que l'ancre de
`_PLAN_SIZE_TOTAL_RE` avait déjà dû fermer. Un plafond sous le plancher ne peut
héberger aucune implémentation, et la note le dit au groomeur au lieu de lui
servir un nombre.

Enfin, **l'omission va toujours vers la borne la plus serrée**. Un plan qui ne
déclare pas son plafond est jugé sur 150, pas sur 200 ; un site qui oublierait de
passer le plafond au résolveur tombe sur le même repli (`disarmed_fallback`). La
direction prudente donne un plan plus petit, donc un découpage parfois inutile —
jamais une mort par volume.

#### L'instrument : une ligne par dispatch, et où elle tombe (mika#2701)

L'étape 1 ci-dessus demande de relever un quadruplet par vol. Depuis mika#2701,
sa **moitié estimée** n'est plus à reconstituer : chaque dispatch
d'implémentation qui sert un plan la dépose, en clair, dans le `.stderr` du vol.
La ligne, complète :

```
dispatch-lib: plan_size_estimate total_loc=<t> threshold_loc=<T> verdict=<v> per_flight_bound_loc=<b> ceiling=<c> ceiling_source=<resolved|disarmed_fallback> per_flight_verdict=<under_bound|over_bound|unknown>[ per_flight_note=<n>] plan=<chemin>
```

Quatre propriétés de forme, et chacune a coûté quelque chose :

- **`plan=` est le dernier champ**, parce qu'il porte un chemin — la seule valeur
  de la ligne qui pourrait un jour contenir un espace. Un champ terminal reste
  parsable ; un champ au milieu ne l'est plus. Qui ajoute un champ l'insère
  **avant** `plan=`.
- **`per_flight_note=` n'apparaît que non vide**, comme `label=` sur la ligne
  `pilot_budget_armed` : un champ vide se lirait comme une note nommée « vide ».
  Un relevé qui compte les notes compte donc des occurrences, pas des champs.
- **Deux unités, deux vocabulaires.** `verdict=` dit `under_threshold` /
  `over_threshold` (lignes par PR) ; `per_flight_verdict=` dit `under_bound` /
  `over_bound` (lignes dérivées de tours). C'est le titre de ce document appliqué
  au corpus de logs : un `grep over_threshold` qui apparierait les deux
  mélangerait deux unités dans la même colonne.
- **`ceiling=` porte le plafond résolu, jamais `none`.** Sur budget désarmé il
  vaut 150 et `ceiling_source=disarmed_fallback` le dit. Sans cela, le
  `per_flight_bound_loc=` qui l'accompagne serait un nombre sans dérivation
  possible.

Le relevé, en ancrant sur `^dispatch-lib: ` et jamais sur le jeton nu — ce
`.stderr` porte aussi la prose du pilote, et mika#2050 a mesuré le faux positif
d'une session qui *discutait* du signal :

```sh
grep -h '^dispatch-lib: plan_size_estimate ' /var/log/claude-pilot/*.stderr
```

Le répertoire est bien `/var/log/claude-pilot/` — c'est le `_PILOT_LOG_DIR` que
`dispatch-lib.sh` résout tardivement (§ audit mika#2165). `~/.mika/logs/`, écrit
ici jusqu'au 2026-10-15, **n'existe pas** : un répertoire absent rend zéro
ligne, exactement comme un corpus vide, et les deux se confondent. C'est la
sonde elle-même qui doit être ancrée, pas seulement le motif.

Correspondance champ → terme du quadruplet de l'étape 1 :

| Champ de la ligne | Terme du quadruplet | Ce qu'il reste à joindre |
|---|---|---|
| `ceiling=` | plafond du vol | rien — c'est l'argv réellement passé |
| `total_loc=` | total estimé du plan | le réel hors `docs/`, par `git show --numstat` |
| `per_flight_bound_loc=` | la borne en vigueur | rien — recalculable des quatre constantes |
| `per_flight_verdict=` | le sens de l'écart | les trois tours, lus sur le vol lui-même |

La ligne donne donc l'**estimé** et la **borne** sans jointure ; le **réel** et
les **tours** restent à relever sur le vol. C'est la moitié qui manquait : avant
mika#2701, le plafond d'un vol passé ne se retrouvait que par les labels de son
ticket — c'est-à-dire par une relecture qui dit l'intention, pas le fait.

**Et porte 3 ne refuse rien.** Un plan groomé au-dessus de la borne est **mesuré**
au dispatch, jamais bloqué : la file groomée entière le lui doit, puisqu'un plan
déjà groomé ne repasse par aucune porte de groom au dispatch suivant. Refuser
ici gèlerait d'un coup tout `docs/plans/`. Détection permissive, décision
stricte — la répartition des trois portes de mika#2636, inchangée.

**Note datée, 2026-10-16 — `n = 7`, les quatre paramètres ont bougé.**
La chaîne mika#2340 a tourné, et le corpus s'est rempli d'un coup. Relevé avec
la sonde ancrée du § *L'instrument* : **7** lignes `plan_size_estimate` au
format à sept champs, soit sept vols post-mika#2695 ayant ouvert une PR — le
seuil de déclenchement était cinq.

| ticket | PR | estimé | réel hors `docs/` | plafond | 1ʳᵉ mut. | dern. mut. | dern. tour | queue |
|---|---|---|---|---|---|---|---|---|
| mika#2700 | #2740 | 390 | 1 117 | 200 | 15 | 107 | 124 | 17 |
| mika#2680 ✱ | #2705 | 345 | 1 398 | 200 | 23 | 147 | 168 | 21 |
| mika#2739 | #2743 | 375 | 830 | 200 | 18 | 43 | 76 | 33 |
| mika#2732 | #2741 | 184 | 422 | 150 | 17 | 34 | 55 | 21 |
| mika#2724 | #2738 | 335 | 884 | 200 | 19 | 80 | 197 | **117** |
| mika#2735 | #2744 | 258 | 600 | 200 | 15 | 54 | 73 | 19 |
| mika#2715 | #2742 | 232 | 550 | 150 | 26 | 69 | 82 | 13 |

✱ reprise : le plan de mika#2680 est daté du 2026-10-14 et le vol du 2026-10-16.
Les lignes qu'il livre ne sont pas toutes produites par ce vol, donc il est
écarté du **débit** et du **facteur** — jamais de l'orientation ni de la queue,
qui sont de vraies mesures de ce vol-là.

**La jointure est ce qui fait le recalibrage.** La ligne `plan_size_estimate` ne
donne que la moitié estimée du quadruplet — plafond, total estimé, borne,
verdict. Les trois tours se lisent sur le `.log` du vol, et c'est en les
appariant au `.stderr` par le chemin du plan qu'on obtient un point de mesure.

**Trois paramètres serrent, un desserre.** Orientation 35 → 27 et fin de
pipeline 50 → 30 **élargissent** la borne (moins de tours volés au travail) ;
débit 7,5 → 9 l'élargit aussi ; facteur 2 → 3 la **resserre**, et c'est le seul
des quatre qui va dans ce sens. Net : 431 → 429 à 200 tours, 243 → 279 à 150.
La borne du palier 200 ne bouge pour ainsi dire pas ; celle du palier 150, qui
mordait sur des plans de 250 lignes, se desserre de 15 %.

**Le contrôle composite, et il tient 6/7 avant comme après.** La question qui
compte n'est pas « chaque paramètre est-il juste » mais « la borne prédit-elle
la survie ». Tours prédits = orientation + estimé × facteur / débit + queue,
comparés au dernier tour réel : six des sept vols sont prédits au-dessus de ce
qu'ils ont consommé, avec les anciens paramètres comme avec les nouveaux. Le
septième est mika#2724 — 197 tours pour 169 prédits — et c'est exactement le vol
dont la queue de 117 est écartée comme aberrante. Un recalibrage qui ne change
pas le nombre de vols correctement prédits n'est pas un échec : il réduit la
marge gaspillée sans acheter de mort.

**La queue de 30 couvre 5 des 6, pas 6 sur 6.** Le max hors aberrant est 33
(mika#2739). On retient 30 et on le dit, plutôt que de laisser croire à un
majorant : ce que la borne garantit, c'est le composite ci-dessus, pas chaque
colonne prise seule. Le jour où deux vols dépassent 30, c'est la queue qu'il
faut rouvrir en premier.

**Ce que la mesure a corrigé dans la méthode elle-même.** Trois points, tous
reportés dans la § *Comment recalibrer* ci-dessus : le tour se compte sur
`[cache] turn`, l'origine de la queue est la dernière mutation et non le dernier
commit, et les reprises s'écartent du débit et du facteur seulement. Les trois
vont dans le même sens — la méthode d'avant **sous-estimait** la queue, donc
surestimait sa propre prudence.

Le geste a tenu en un commit sur cinq sites, comme annoncé : les quatre
constantes, les deux `system_prompt.md` d'architecte, les assertions de valeur
du harnais (V1, V2, V5 — et **V7-quater**, un quatrième bloc que l'inventaire du
ticket omettait), la sonde X2 nouvellement armée, et ce document. Les sondes de
plancher, elles, n'ont pas été touchées : X1 les avait rendues dérivées la
veille, et le plancher est passé de 85 à 57 sans qu'une seule assertion bouge —
c'est exactement ce pour quoi X1 existe.

## Related

- CLAUDE.md, section d'observabilité de l'estimation de taille (sondes 1 à 4,
  halte 2 « c'est le seuil qui bouge ») — le thermomètre ; ce document est la
  première lecture qu'il produit.
- `docs/solutions/best-practices/sealing-a-pre-registered-estimand-2026-08-29.md` —
  sceller ce qu'on mesure avant de le mesurer.
- `docs/solutions/dev-loop/un-marqueur-de-completude-doit-lire-si-la-session-a-conclu-2026-09-29.md` —
  un pilote tué à 151/150 tours, la classe de mort que cette porte vise.
- mika#2693 — la borne par vol. Le § ci-dessus est sa justification ; sa
  dérivation vit en commentaire au-dessus de `_plan_size_per_flight_bound`
  (`skills/bundled/_shared/dispatch-lib.sh`), et les sondes V1 à V6 de
  `test-dispatch-lib.sh` la tiennent.
- mika#2701 — porte 3, la mesure. Les quatre champs par vol de la ligne
  `plan_size_estimate`, tenus par les sondes W1 à W7 de `test-dispatch-lib.sh` ;
  le § *L'instrument* ci-dessus est son mode d'emploi.
- mika#2707 — le recalibrage que cette mesure a rendu possible. **Fait le
  2026-10-16 sur n=7** : 35/50/750/200 → 27/30/900/300, bornes 431/243 → 429/279,
  plancher 85 → 57. La note datée ci-dessus porte le corpus, la méthode corrigée
  et le contrôle composite ; la sonde X2 de `test-dispatch-lib.sh` tient
  désormais les deux exemples travaillés du commentaire contre le résolveur.
