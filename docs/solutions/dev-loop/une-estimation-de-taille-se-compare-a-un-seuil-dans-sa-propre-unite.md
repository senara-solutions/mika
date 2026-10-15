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
| orientation | ~35 tours | premier `Edit` des trois vols : t39, t36, t32 — tout ce qui précède est lecture de plan, de ticket et de code |
| fin de pipeline | ~50 tours | écart entre le dernier commit de travail et la mort (t171→201, t177→201, t136→151), majoré : les trois sont morts *dedans*, donc l'écart observé est un plancher, pas la durée réelle |
| débit | ~7,5 lignes réelles/tour | 1 432 / (171−39), 1 160 / (177−36), 994 / (136−32) → 10,8 / 8,2 / 9,6, rabattu à 7,5 car les trois sont morts avant la fin |
| facteur estimé → réel | 2 (prudent) | 1 432/832 = 1,7 ; 1 160/880 = 1,3 ; 994/443 = 2,2. L'intervalle 1,3–2,2 est pris à 2, pas à sa moyenne |

Soit **431** lignes estimées à un plafond de 200, et **243** à 150.

Les quatre paramètres vivent nommés à un seul site,
`skills/bundled/_shared/dispatch-lib.sh`, juste au-dessus de
`_plan_size_per_flight_bound` ; le résolveur les applique, et un test de parité
du type S14 (V4) épingle les littéraux des deux prompts architecte contre eux.
Un `.sh` n'a jamais eu besoin de porter cette justification — ce qu'il doit
porter, c'est de quoi **recalculer**, et c'est pourquoi la dérivation y vit en
commentaire plutôt qu'un nombre nu.

### Arithmétique entière : les deux mises à l'échelle s'annulent

`bash` n'a pas de flottants. 7,5 lignes/tour et un facteur de 2 se portent donc
par deux entiers **dont le nom dit l'échelle** : `750` lignes pour `100` tours,
et le facteur en pourcentage (`200`). Les deux mises à l'échelle s'annulent
(×1/100 puis ÷1/100), ce qui rend le calcul exact en **une seule** division :

```
borne = (plafond − 35 − 50) × 750 / 200
plafond 200 → 115 × 750 / 200 = 431
plafond 150 →  65 × 750 / 200 = 243
```

**Qui recalibre le débit à 9 lignes/tour écrit `900`, jamais `9`.** L'annulation
est dite au site précisément pour que le prochain recalibrage ne divise pas la
borne par cent en croyant la corriger.

### Comment recalibrer sur une distribution, et pas sur une mort

n=3 n'est pas une distribution, et les trois points sont biaisés dans le même
sens : les trois pilotes sont morts, donc chacun des quatre paramètres est
mesuré sur une course tronquée. La méthode :

1. Par vol terminé — **mort ou non** —, relever le quadruplet : plafond, total
   estimé du plan, réel hors `docs/` par `git show --numstat`, et les trois tours
   (premier `Edit`, dernier commit de travail, dernier tour).
2. Les vols qui ont **fini** sont les seuls à donner une fin de pipeline et un
   débit non tronqués. Un corpus qui ne contient que des morts majore
   l'orientation et minore tout le reste.
3. Recalculer les quatre paramètres sur le corpus entier, puis la borne. Comme
   au § précédent, **c'est la borne qui bouge**, jamais la mesure.
4. Changer la constante au site unique. V4 tirera sur les deux prompts
   architecte tant qu'ils n'ont pas suivi — c'est exactement son office, un
   `system_prompt.md` étant statique et incapable de lire une constante shell.

Le plancher est explicite dans le résolveur : un plafond ≤ 85 (= 35 + 50) donne
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

**Note datée, 2026-10-15 — relevé fait, `n = 0`, la constante n'a pas bougé.**
Le corpus a été relevé le 2026-10-15 avec la sonde ci-dessus : **16** lignes
`plan_size_estimate` ancrées, dont **zéro** portant `ceiling=` ou
`per_flight_bound_loc=`. Toutes sont au format pré-mika#2701, à quatre champs.
Le seuil de recalibrage de `_PLAN_SIZE_FLIGHT_TAIL_TURNS` est cinq vols
post-#2695 ayant ouvert une PR ; `n = 0 < 5`, donc **aucune des quatre
constantes n'a été touchée** et le suivi reste mika#2707.

La cause est plus nette que « l'instrument est trop jeune » : **aucun binaire
déployé ne l'embarque**. Porte 3 est née le 2026-10-15 et la chaîne mika#2340
— *rebuild → seed → read* — n'a pas tourné depuis, donc les vols du jour
exécutent un `dispatch-lib.sh` qui ignore les trois champs. **Le corpus ne
commence à se remplir qu'au premier vol suivant un `make deploy`** : c'est la
condition que la condition de déclenchement présuppose sans la dire.

La prémisse du ticket, elle, tient. Deux queues reconstruites à la main hors
instrument — 18 tours (vol de mika#2701) et 10 tours (vol de mika#2697) — sont
bien sous 50. Mais n=2 *est* le n=3 que la § ci-dessus refuse, et ces deux
mesures sous-estiment même la queue : en régime allégé l'ordre est
vérifier → commit → push → PR, donc la vérification tombe *avant* le dernier
commit de travail que l'étape 1 prend pour origine. Contrôle de plausibilité,
jamais une distribution.

Ce qui a bougé à la place : les **cinq** sondes à plafond de plancher du harnais
dérivent désormais leur plafond de `orientation + fin de pipeline` au lieu de
retaper `85` et `70`, et un détecteur armé (X1) refuse qu'un littéral y
revienne. Cinq, et non les quatre recensées : la mutation de X1 en a découvert
une qu'aucun inventaire ne portait. Le littéral n'était pas qu'un doublon : choisi juste *sous* l'ancien
plancher, il passe *au-dessus* du nouveau dès que la queue baisse, et la sonde
mesure alors le bras nominal du résolveur en se croyant sur le plancher — verte,
pour la mauvaise raison. Le recalibrage du jour J ne touchera donc que les
constantes, les deux prompts d'architecte et les assertions qui épinglent des
**valeurs** (V1, V2, V4, V5) — jamais le plancher.

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
- mika#2707 — le recalibrage de `_PLAN_SIZE_FLIGHT_TAIL_TURNS` que cette mesure
  rend possible. Ouvert, non actionnable tant que le corpus post-#2695 n'atteint
  pas cinq vols ayant ouvert une PR.
