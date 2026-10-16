# mika#2707 — Recalibrer les quatre paramètres de la borne par vol, sur une distribution mesurée

**Ticket :** senara-solutions/mika#2707
**Lignée :** mika#2496 → mika#2542 → mika#2636 → mika#2693 (portes 1 et 2) → mika#2701
(porte 3, la mesure) → **ce plan** (le recalibrage que cette mesure rend possible).
**Vol précédent :** PR #2712 (mergée, `02764f6a`) — a franchi la porte B (`n = 0 < 5`),
n'a déplacé aucune constante, et a dérivé les cinq sondes de plancher du harnais.

---

## 1. Contexte : la condition de déclenchement est remplie, et le relevé a été fait

Le ticket est devenu actionnable le 2026-10-16 : `make deploy` a tourné, la chaîne
mika#2340 (*rebuild → seed → read*) a porté porte 3 dans le binaire qui dispatche, et
l'instrument de mika#2701 s'est mis à écrire ses sept champs.

**Ce plan ne demande pas à l'implémentation de mesurer.** Le relevé est fait, il est
dans ce document, et l'implémentation en écrit le résultat. La raison est la raison
d'être du grooming : un plan qui dirait « mesure puis décide » remettrait un jugement
non revu au pilote, et l'architecte ne pourrait juger aucune valeur. L'implémentation
re-passe la sonde de contre-vacuité (§ 6, AC1) pour vérifier que la prémisse du relevé
tient encore ; elle ne refait pas l'arbitrage.

### 1.1 Le constat du ticket, et ce que la mesure en fait

Le ticket pose que `_PLAN_SIZE_FLIGHT_TAIL_TURNS=50` **surestime la queue** depuis que
mika#2694 / PR #2695 a retiré `ce-simplify-code` et `ce-code-review` des vols. C'est
vrai, et mesuré : la queue médiane est de **17 tours**, son maximum hors aberrant de
**30**.

Mais la méthode de la maison (solution-doc, § *Comment recalibrer sur une distribution*,
étape 3) ne dit pas « change la constante qui a motivé le ticket ». Elle dit :
« **Recalculer les quatre paramètres sur le corpus entier**, puis la borne. » Appliquée,
elle rend un résultat que le ticket ne pouvait pas prévoir :

| paramètre | déclaré | mesuré | sens de l'erreur |
|---|---|---|---|
| orientation | 35 tours | max 27, médiane 18,5 | déclaré **trop grand** (serre) |
| fin de pipeline | 50 tours | max 30 hors aberrant, médiane 17 | déclaré **trop grand** (serre) |
| débit | 7,5 l/tour | min 10,8, médiane 14,3 | déclaré **trop petit** (serre) |
| facteur d'estimation | 2,0 | max 2,82, médiane 2,24 | déclaré **trop petit** (desserre) |

**Trois des quatre serrent, un desserre, et ils se compensent en grande partie.** C'est
pourquoi le recalibrage complet laisse la borne du palier 200 pratiquement où elle est
(431 → 429) tout en la desserrant là où le coût du ticket mord vraiment : le palier 150,
qui est le défaut de flotte et donc le plafond de la quasi-totalité des tickets
(243 → 279), et le palier 120 (131 → 189). Le plancher de pipeline passe de 85 à 57.

Recalibrer la seule queue (50 → 30) aurait porté la borne du palier 150 à 356 — **+47 %**
— en laissant intact un facteur d'estimation que la mesure dit optimiste de 15 à 40 %.
Ce plan ne le fait pas, et le § 4.2 montre que le composite mesuré l'interdit.

---

## 2. Le relevé : méthode, instrument, et la jointure

### 2.1 L'unité, et le compteur qu'on refuse

La solution-doc s'appelle *une estimation de taille se compare à un seuil dans sa propre
unité*. Le premier geste du relevé est donc de nommer le compteur, parce que le `.stderr`
d'un vol en porte **trois**, et deux d'entre eux ne sont pas le budget :

| compteur | forme dans le log | pour mika#2724 (plafond 200) |
|---|---|---|
| le budget SDK | `[guardrail] error_max_turns: SDK limit reached after <N> turns` | — (n'a pas tiré) |
| le total du harnais | `[done] Success \| <N> turns` | **208** |
| le tour d'appel API | `[cache] turn <N>: cache_read_input_tokens=…` | **197** |

`[done] 208` sur un vol armé à `maxTurns=200` **qui n'a pas été tué** : le relevé a lu la
bannière (`[guardrails] maxTurns=200`), compté une seule session, un seul `[done]`, aucune
intervention de garde-fou. Et ailleurs dans le corpus le tueur dit `after 201 turns` à
plafond 200, `after 151` à plafond 150 — donc le budget mord bien à plafond + 1. Les deux
faits ne tiennent ensemble que d'une façon : **`[done] N turns` n'est pas l'unité du
budget.** Un recalibrage qui l'aurait prise aurait sur-compté toutes les durées de 7 à
24 % — et aurait commis, sur le corpus de logs, exactement l'erreur dont ce document
porte le titre.

**L'unité retenue est `[cache] turn <N>`**, le seul des trois qui reste sous le plafond sur
le vol où les deux autres se contredisent.

### 2.2 L'origine de la queue : la dernière mutation, pas le dernier commit

L'étape 1 de la méthode demande « les trois tours (premier `Edit`, **dernier commit de
travail**, dernier tour) ». Ce plan remplace le terme du milieu, et il le fait avec
l'argument que la solution-doc écrit elle-même (§ note datée 2026-10-15, l. 332-335) :

> « en régime allégé l'ordre est vérifier → commit → push → PR, donc la vérification tombe
> *avant* le dernier commit de travail que l'étape 1 prend pour origine. »

Un dernier-commit-de-travail pris pour origine **exclut la vérification de la queue**, et
la vérification est précisément ce que la queue coûte. L'origine retenue est donc :

> **le dernier appel `Edit` / `Write` sur un chemin qui n'est ni `pr-body.md` ni sous
> `.pilot-scratch/`.**

Trois propriétés, et chacune compte :

- **Purement en-log.** Aucune date git, donc aucune exposition à l'écrasement de
  `committedDate` par le rebase — mesuré sur ce corpus : les trois premiers commits de
  la PR #2705 portent `09:03:19`, `09:03:19`, `09:03:20`, c'est-à-dire la date du rebase
  et non celle du travail. Une jointure sur `committedDate` aurait rendu des queues
  fausses sur au moins deux vols sur sept.
- **`pr-body.md` et `.pilot-scratch/` sont exclus par construction**, pas par
  commodité : le corps de PR et les messages de commit sont *produits par la queue*. Les
  compter comme production ferait disparaître la queue qu'on mesure.
- **Elle absorbe la vérification intercalée.** Le vol de mika#2700 a vérifié entre
  13:04 et 13:21, puis muté une dernière fois à 13:21:14 (l'annotation du garde
  byte-slice). Ces 17 minutes de vérification tombent donc dans la fenêtre de
  *production* et déflatent le débit mesuré. Le relevé l'assume et le compense en
  prenant le débit **minimum** du corpus (§ 3.2), jamais sa médiane.

### 2.3 Les commandes du relevé, reproductibles

Ancrées sur `^dispatch-lib: `, jamais sur le jeton nu — mika#2050 a mesuré le faux
positif d'une session qui *discutait* du signal.

```sh
# a. le corpus : les vols dont le dispatch embarque porte 3
grep -rl '^dispatch-lib: plan_size_estimate .*per_flight_bound_loc=' /var/log/claude-pilot

# b. un plan dispatché deux fois est un vol de REPRISE — le détecter avant de conclure
grep -rh '^dispatch-lib: plan_size_estimate ' /var/log/claude-pilot \
  | grep -oE 'plan=docs/plans/.*' | sort | uniq -c | sort -rn

# c. par vol : le dernier tour
grep -oE 'turn [0-9]+: cache_read' /var/log/claude-pilot/<sid>.stderr | tail -1

# d. par vol : la dernière mutation (hors pr-body.md et .pilot-scratch/)
grep -E '\[tool\].{0,10}(Edit|Write)\b' /var/log/claude-pilot/<sid>.stderr | cut -c2-21,145-200

# e. par vol : le tour à cet instant — fenêtre de deux minutes autour de l'horodatage de (d)
grep -oE '^\[<date>T<hh>:<mm>:[0-9]{2}.{0,40}turn [0-9]+' /var/log/claude-pilot/<sid>.stderr \
  | grep -oE 'T<hh>:<mm>:[0-9]{2}|turn [0-9]+'

# f. par vol : le réel hors docs/ et hors CLAUDE.md
gh pr view <n> --repo senara-solutions/mika --json files \
  --jq '[.files[] | select(.path | startswith("docs/") or endswith("CLAUDE.md") | not) | .additions] | add'
```

Le tour d'une mutation est celui dont le marqueur est **à son horodatage ou juste avant** :
le marqueur `[cache] turn N` est écrit à la clôture du message de réponse, l'outil
s'exécute ensuite.

### 2.4 Le corpus, et la jointure — n = 7

Sept vols portent la ligne à sept champs, et **les sept ont ouvert une PR** (quatre
mergées, trois ouvertes). `n = 7 ≥ 5` : la condition de déclenchement du ticket est
remplie, et par le format, pas par le vide (2 769 `.stderr` sous
`/var/log/claude-pilot/`, 34 lignes `plan_size_estimate` ancrées, dont 7 au format
porte 3).

| # | ticket | PR | session | plafond | est. | réel hors `docs/` | 1er `Edit` | dern. mut. | dern. tour | orient. | **queue** | tours prod. | débit | facteur |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | mika#2732 | #2741 | `8dedc173` | 150 | 184 | 417 | 17 | 34 | 55 | 17 | **21** | 17 | 24,5 | 2,27 |
| 2 | mika#2739 | #2743 | `7aa1272d` | 200 | 375 | 828 | 18 | 46 | 76 | 18 | **30** | 28 | 29,6 | 2,21 |
| 3 | mika#2715 | #2742 | `c561dc3c` | 150 | 232 | 463 | 27 | 70 | 82 | **27** | **12** | 43 | **10,8** | 2,00 |
| 4 | mika#2735 | #2744 | `b11118d4` | 200 | 258 | 548 | 21 | 59 | 73 | 21 | **14** | 38 | 14,4 | 2,12 |
| 5 | mika#2680 | #2705 | `344d821f` | 200 | 345 | 1 358 | 24 | 148 | 168 | 24 | **20** | 124 | 11,0 ✱ | 3,94 ✱ |
| 6 | mika#2700 | #2740 | `0f453098` | 200 | 390 | 1 100 | 18 | 107 | 124 | 18 | **17** | 89 | 12,4 | **2,82** |
| 7 | mika#2724 | #2738 | `91b576e1` | 200 | 335 | 878 | 19 | 81 | 197 | 19 | **116** † | 62 | 14,2 | 2,62 |

**✱ Vol n° 5 : une reprise.** La sonde (b) le dit sans ambiguïté — le plan
`2026-10-14-003-feat-2680-…` a été dispatché **deux fois**, et `344d821f` est le second ;
les six autres plans comptent un dispatch chacun. Son réel (1 358 lignes) inclut donc la
production de son prédécesseur, ce qui **déflate son débit** et **gonfle son facteur**.
Ses deux colonnes de ratio sont exclues des estimateurs ; ses trois tours, qui sont bien
les siens, restent dans le corpus. C'est l'étape 2 de la méthode appliquée à une forme
qu'elle ne nommait pas : un vol de reprise ne donne pas un débit non tronqué, même
lorsqu'il a fini.

**† Vol n° 7 : une queue aberrante, et pourquoi on ne la couvre pas.** 116 tours de queue
contre 12 à 30 pour les six autres. La cause se lit sur les mutations : entre la dernière
mutation de source (12:29:26) et la fin (13:14:11), ce vol a écrit un corps d'issue de
suivi, un avis d'édition d'issue, un corps de PR de suivi et un correctif de câblage CI —
**du périmètre pris après la PR, pas une fin de pipeline**. L'argument décisif n'est
toutefois pas qualitatif : couvrir 116 porterait le plancher à 27 + 116 = **143**, donc la
borne du palier 150 à `(150 − 143) × 3 = 21` lignes et celle du palier 120 à **0** avec la
note `ceiling_below_pipeline_floor`. **Le maximum n'est pas un estimateur utilisable ici
parce qu'il désarmerait la porte au plafond par défaut.** La queue est donc calibrée sur
le maximum des six, et l'aberrant est nommé plutôt que dilué dans une moyenne.

---

## 3. La décision : les quatre valeurs, et l'estimateur de chacune

### 3.1 Le principe : prudent par paramètre, vérifié en composite

La solution-doc pose la direction (§ *l'omission va toujours vers la borne la plus
serrée*) : « La direction prudente donne un plan plus petit, donc un découpage parfois
inutile — jamais une mort par volume. » Chaque paramètre est donc pris à son **extrême
prudent**, pas à sa médiane. Mais quatre extrêmes composés donnent une borne
sur-conservatrice — et le § 4.2 vérifie le composite au lieu de le supposer.

### 3.2 Les quatre valeurs

| constante | avant | **après** | estimateur | justification |
|---|---|---|---|---|
| `_PLAN_SIZE_FLIGHT_ORIENTATION_TURNS` | 35 | **27** | maximum des 7 | 27 est l'orientation du vol n° 3 ; aucun vol du corpus n'a orienté plus longtemps |
| `_PLAN_SIZE_FLIGHT_TAIL_TURNS` | 50 | **30** | maximum des 6 hors aberrant | {12, 14, 17, 20, 21, 30} ; l'aberrant est nommé au § 2.4 † |
| `_PLAN_SIZE_FLIGHT_LOC_PER_100_TURNS` | 750 | **900** | **sous** le minimum des 6 hors reprise | min mesuré 10,8 l/tour ⇒ 9,0 est prudent, et entier |
| `_PLAN_SIZE_FLIGHT_ESTIMATE_FACTOR_PCT` | 200 | **300** | **au-dessus** du maximum des 6 hors reprise | max mesuré 2,82 ⇒ 3,0 ; la médiane (2,24) serait imprudente |

**`900`, jamais `9`.** Le bloc de commentaire du site avertit déjà : « QUI RECALIBRE LE
DÉBIT À 9 LIGNES/TOUR ÉCRIT `900`, JAMAIS `9` — l'annulation est dite ici précisément pour
que le prochain recalibrage ne divise pas la borne par cent en croyant la corriger. » Ce
recalibrage est celui-là, et il écrit `900`. **La phrase d'avertissement ne change pas** :
elle cesse d'être une hypothèse pour devenir la description du site, et la sonde V3 qui
l'épingle en littéral reste verte sans retouche. Un geste en moins.

### 3.3 La borne qui en dérive

Le mécanisme est inchangé — c'est le hors-périmètre explicite du ticket. Seules les
valeurs bougent, et c'est exactement pourquoi elles sont des constantes nommées :

```
borne = (plafond − 27 − 30) × 900 / 300   ≡   (plafond − 57) × 3

plafond 200 -> 143 × 900 / 300 = 429      (était 431)
plafond 150 ->  93 × 900 / 300 = 279      (était 243)
plafond 120 ->  63 × 900 / 300 = 189      (était 131)
plancher     -> 57 = 27 + 30              (était 85)
```

Le rendu humain de la dérivation suit sans intervention : `_plan_size_rule` ramène `900`
à `9` et `300` à `3`, et un dixième nul ne s'écrit pas. La règle injectée au groomeur
dira donc `(150 − 27 − 30) × 9 / 3`, sous la forme exacte que le format prescrit.

---

## 4. Ce que ces valeurs changent, et ce qu'elles ne cassent pas

### 4.1 Le sens du déplacement, palier par palier

| palier | borne avant | borne après | écart | qui est à ce plafond |
|---|---|---|---|---|
| 200 | 431 | 429 | **−0,5 %** | les tickets portant `loop-substrate` |
| 150 | 243 | 279 | **+15 %** | **le défaut de flotte** — la quasi-totalité des tickets |
| 120 | 131 | 189 | **+44 %** | une surcharge `PILOT_MAX_TURNS` délibérée |
| plancher | 85 | 57 | −33 % | la borne vaut 0 et la note le dit |

Le coût que le ticket nomme (« plus de vols que nécessaire ») se payait au palier 150,
puisque c'est celui de presque tous les tickets. C'est là que le desserrage arrive. Au
palier 200, la borne ne bouge pratiquement pas — et le § suivant montre que c'est le
corpus qui l'exige, non un arrondi.

### 4.2 Le composite : le nouveau modèle est aussi sûr que l'actuel, pas moins

Le modèle inverse de la borne prédit les tours qu'un plan de `e` lignes estimées
consommera : `tours = orientation + e × facteur / débit + queue`.

| modèle | formule | prédiction |
|---|---|---|
| actuel | `35 + e × 2 / 7,5 + 50` | `85 + 0,267·e` |
| **proposé** | `27 + e × 3 / 9 + 30` | `57 + 0,333·e` |

| ticket | est. | observé | prédit (actuel) | prédit (**proposé**) | actuel couvre | proposé couvre |
|---|---|---|---|---|---|---|
| mika#2732 | 184 | 55 | 134 | 118 | ✓ | ✓ |
| mika#2739 | 375 | 76 | 185 | 182 | ✓ | ✓ |
| mika#2715 | 232 | 82 | 147 | 134 | ✓ | ✓ |
| mika#2735 | 258 | 73 | 154 | 143 | ✓ | ✓ |
| mika#2680 | 345 | 168 | 177 | **172** | ✓ | ✓ (marge 4) |
| mika#2700 | 390 | 124 | 189 | 187 | ✓ | ✓ |
| mika#2724 | 335 | **197** | 174 | 169 | ✗ | ✗ |

**6/7 pour les deux, et le seul échec est le même** — le vol n° 7, dont les 197 tours
viennent de la queue aberrante du § 2.4 †, que le modèle actuel ne couvre pas davantage.
Le recalibrage **ne dégrade donc pas la sûreté du modèle sur le corpus** : il déplace la
borne sans déplacer le nombre de vols que le modèle aurait laissés mourir.

C'est aussi ce qui interdit le geste « queue seule ». Avec queue = 30 et les trois autres
paramètres inchangés, le modèle prédirait `65 + 0,267·e`, soit **157** pour mika#2680
contre 168 observés : le desserrage de +47 % au palier 150 se paierait par **deux** échecs
au lieu d'un, dont un vol qui a réellement fini. Le facteur d'estimation doit suivre la
queue ; c'est la mesure qui le dit, pas une préférence.

### 4.3 Un invariant du harnais que le choix doit respecter

W1 (mika#2701) porte une sonde dite « le cœur » : **à total 400 égal, le verdict par vol
doit être opposé aux deux paliers** — `over_bound` à 150, `under_bound` à 200. Elle exige
donc `B150 < 400 ≤ B200`. Avant : `243 < 400 ≤ 431`. Après : `279 < 400 ≤ 429` ✓. Les
sondes W tirent leurs deux bornes du résolveur (`T2701_B200` / `T2701_B150`) et **ne
demandent aucune retouche** ; seul cet invariant de fixture devait être vérifié, et il
l'est. Les autres fixtures W survivent aussi (total 900 reste `over_bound` à 279 ; total
200 reste `under_bound`).

La marge est de 29 lignes au palier 200. **Un recalibrage futur qui porterait `B200` sous
400 rendrait W1 rouge pour une raison qui n'est pas un défaut** : c'est noté ici, pas
corrigé — le réécrire en fixture dérivée est un geste orthogonal à ce ticket.

---

## 5. Les livrables, site par site

Les cinq sites sont tenus ensemble. **V4 est la garde qui rend le geste sûr, et c'est
aussi elle qui le rend atomique** : elle dérive les quatre paramètres et les deux bornes
de référence des deux `system_prompt.md` depuis les constantes shell, donc elle tire tant
que les prompts n'ont pas suivi. Un commit unique, pas une série.

Le recensement a été refait indépendamment de celui du ticket, par
`grep -rln '431 lignes\|431 estimated\|35 − 50\|431|200\|243|150'` hors `docs/plans/` : il
rend **exactement** les quatre fichiers ci-dessous plus la solution-doc. Aucun sixième
site.

### D1 — `skills/bundled/_shared/dispatch-lib.sh` : les quatre constantes et leur bloc

1. Les quatre affectations (l. 780-783) prennent `27`, `30`, `900`, `300`.
2. Dans le bloc de commentaire `# mika#2693 — LA SECONDE BORNE` qui les précède, les
   **exemples travaillés en prose** suivent : la formule `(plafond − 35 − 50) × 750 / 200`
   et les deux lignes `plafond 200 -> …` / `plafond 150 -> …`.
3. Le § PLANCHER du commentaire de `_plan_size_per_flight_bound` dit `85 (= 35 + 50)` en
   littéral : il devient `57 (= 27 + 30)`. Il y nomme aussi `PILOT_MAX_TURNS=70` comme le
   seul chemin délibéré vers le plancher — `70` est désormais **au-dessus** de 57, donc
   l'exemple doit devenir une valeur sous 57 (`PILOT_MAX_TURNS=50`), sans quoi le
   commentaire illustrerait le cas nominal en croyant illustrer le plancher. C'est la
   même classe de défaut que celle qu'X1 a fermée côté harnais en PR #2712 : un littéral
   choisi juste sous l'ancien plancher passe au-dessus du nouveau dès que la queue baisse.
4. La phrase `QUI RECALIBRE LE DÉBIT À 9 LIGNES/TOUR ÉCRIT 900, JAMAIS 9` **ne change
   pas** (§ 3.2), donc V3 reste verte sans retouche.

Rien d'autre dans ce fichier : le gabarit de règle et le texte d'`ALERTE PLAFOND`
interpolent les constantes.

### D2 / D3 — les deux `system_prompt.md` d'architecte

- `skills/bundled/mika-arch-groom-ticket/system_prompt.md` l. 143 et 148
- `skills/bundled/mika-arch-second-review/system_prompt.md` l. 121 et 126

Chacun porte une ligne de format (`Borne par vol : 431 lignes estimées — (200 − 35 − 50)
× 7,5 / 2`) et une ligne de bornes de référence (`431 estimated lines at a 200-turn
ceiling · 243 at a 150-turn ceiling`). Elles deviennent `429 … (200 − 27 − 30) × 9 / 3`
et `429 … · 279 …`. Un `system_prompt.md` est statique et incapable de lire une
constante shell — c'est précisément l'office de V4.

### D4 — `skills/bundled/_shared/test-dispatch-lib.sh` : les assertions qui épinglent des **valeurs**

Le cinquième site que le § « Attention au geste » de mika#2701 omettait. Trois blocs, et
**uniquement les assertions de valeur** — ni les sondes de plancher (dérivées depuis
PR #2712 via `T2707_FLOOR` / `T2707_UNDER`), ni V3, ni V4, ni les sondes W :

- **V1** (l. ~12630-12647) : `431|200|resolved|` → `429|200|resolved|` ;
  `243|150|resolved|` → `279|150|resolved|` ; `131|120|resolved|` → `189|120|resolved|` ;
  `243|150|disarmed_fallback|` → `279|150|disarmed_fallback|` ; le repli du résolveur
  appelé sans argument, `431` → `429`. Les deux sondes de plancher
  (`$T2707_FLOOR`, `$T2707_UNDER`) **ne bougent pas** — c'est ce que PR #2712 a payé
  d'avance, et le détecteur X1 refuse qu'un littéral y revienne.
- **V2** (l. ~12653-12664) : les six occurrences de `243|150|resolved` → `279|150|resolved`
  et de `431|200|resolved` → `429|200|resolved`, en gardant intacte la démonstration de
  l'appariement **exact** du label (`not-loop-substrate` et `loop-substrate-v2` rendent
  le défaut de flotte).
- **V5** (l. ~12744-12770) : `431 lignes estimées` → `429 lignes estimées` et
  `243 lignes estimées` → `279 lignes estimées` (y compris les deux `assert_not_contains`
  croisés) ; les deux chaînes de dérivation `(200 − 35 − 50) × 7,5 / 2` et
  `(150 − 35 − 50) × 7,5 / 2` → `(200 − 27 − 30) × 9 / 3` et `(150 − 27 − 30) × 9 / 3` ;
  et l'`assert_not_contains` du dixième nul passe de `/ 2,0` à `/ 3,0` — sans quoi il
  garderait une forme que le gabarit ne peut plus produire, et serait vert par vacuité.

### D5 — `docs/solutions/dev-loop/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md`

Hors du compte de lignes (`docs/`), mais dans le périmètre : c'est le site de
justification, et le ticket le nomme comme cinquième site.

1. § *Arithmétique entière* (l. 208-212) : les deux exemples travaillés suivent les
   constantes.
2. § *Comment recalibrer sur une distribution, et pas sur une mort* : l'étape 1 voit son
   terme du milieu corrigé — « dernier commit de travail » → « dernière mutation
   `Edit`/`Write` hors `pr-body.md` et `.pilot-scratch/` », **avec l'argument du § 2.2 de
   ce plan**, qui est l'argument que la note datée du document écrivait déjà sans en
   tirer la conséquence. Deux ajouts à la méthode, tous deux payés par ce relevé :
   l'unité (§ 2.1 — le compteur `[done]` est refusé, avec son contre-exemple) et la
   détection des vols de reprise (§ 2.4 ✱ — un plan dispatché deux fois).
3. Le plancher y est dit `85 (= 35 + 50)` : → `57 (= 27 + 30)`.
4. **La note datée du 2026-10-15 est remplacée, pas complétée** — le ticket l'exige
   explicitement. La note disait `n = 0`, aucune constante touchée, cause = chaîne
   mika#2340 non parcourue. Elle devient la note datée du **2026-10-16** : `n = 7`, le
   corpus, la jointure, les quatre valeurs, et le fait que trois paramètres serraient et
   un desserrait. Le § *Related* voit son entrée mika#2707 passer de « ouvert, non
   actionnable » à son état réel.

### D6 — la sonde X2 (nouvelle) : les exemples travaillés en prose cessent d'être un troisième site de vérité

Le bloc de commentaire de D1 et le § *Arithmétique entière* de D5 portent `431` et `243`
**en prose**, et rien ne les relie aux constantes. C'est la classe exacte de défaut qu'X1
a fermée pour le plancher en PR #2712 — et dont la note datée dit qu'elle a « découvert
une [occurrence] qu'aucun inventaire ne portait ».

X2 extrait les deux lignes `plafond <c> -> … = <b>` du bloc de commentaire de
`dispatch-lib.sh` et asserte que chaque `<b>` égale `$(_t2693_bound_probe <c>)`. Elle vit
à côté de V3, dont elle est le pendant par valeur.

Elle **ne lit pas** la solution-doc : `test-dispatch-lib.sh` teste `dispatch-lib.sh`, et
un test shell qui tire sur un document de `docs/` serait une dépendance que ni le
harnais ni la CI ne portent aujourd'hui. Le suivi du document reste tenu par la revue,
comme pour les autres solution-docs. C'est une limite assumée de la sonde, nommée ici
pour qu'elle ne se lise pas comme une couverture.

---

## Fire-Disposition

Le plan livre **un** détecteur : la sonde **X2** de D6 (son chemin de succès est « aucun
exemple travaillé en divergence »). D1-D5 ne font que re-épingler des valeurs dans des
détecteurs existants, ce qui ne crée pas de détecteur.

**Disposition retenue : (a) exception nommée en allowlist — avec une allowlist VIDE, et
l'assertion qui l'exige.**

- **Pourquoi vide, et pourquoi c'est honnête.** X2 atterrit dans le **même commit** que la
  correction des exemples travaillés (D1 §2). Au moment où elle s'arme, il n'existe aucune
  violation à exempter : les deux lignes du bloc portent `429` et `279`, qui sont ce que le
  résolveur rend. Une allowlist à entrées serait donc une fiction.
- **Pourquoi (b) est refusée.** Livrer désarmée (`#[ignore]` / `return 0` en tête) poserait
  un suivi pour armer un détecteur dont la seule violation possible est déjà corrigée dans
  le même diff : le suivi ne pourrait rien faire d'autre que retirer le désarmement.
- **Pourquoi (c) est refusée.** Rien ne demande de cadrage : la sonde a zéro violation et
  un périmètre d'un seul fichier.
- **Ce qui empêche le vert par vacuité**, et c'est la contrepartie obligatoire d'une
  allowlist vide : X2 porte **d'abord** une assertion de contre-vacuité — l'extraction
  rend **exactement deux** lignes d'exemple, et les deux bornes extraites sont non vides
  et **distinctes**. Sans elle, une extraction muette (un bloc renommé, une regex qui
  n'apparie plus) rendrait X2 verte en n'ayant rien mesuré. C'est le patron de `W1
  contre-vacuité` et de `X1 contre-vacuité`, repris tel quel.
- **Ce qui la rend auto-nettoyante.** X2 n'épingle aucune valeur : elle compare la prose
  au résolveur. Le prochain recalibrage la rend rouge s'il oublie la prose, et verte dès
  qu'il la met à jour — sans qu'une ligne de X2 ait à bouger. Il n'y a donc pas
  d'exception à périmer.

---

## 6. Contrat de vérification

| # | ce qui est vérifié | comment |
|---|---|---|
| T1 | **la prémisse du relevé tient encore** | `grep -rl '^dispatch-lib: plan_size_estimate .*per_flight_bound_loc=' /var/log/claude-pilot \| wc -l` ≥ 7, et la sonde (b) du § 2.3 ne révèle aucune reprise nouvelle parmi les sept vols du § 2.4. Si < 7, **halte et remontée** : le corpus ne peut que croître, un effondrement signalerait une rotation de logs et le relevé devrait être refait. |
| T2 | le résolveur rend les trois paliers et le plancher | V1 verte avec `429|200`, `279|150`, `189|120`, `0|57|…floor`, `0|42|…floor` — les deux derniers **dérivés**, pas retapés |
| T3 | la distinction de palier arrive au groomeur | V2 et V5 vertes ; la règle injectée porte `429` à 200 et `279` à 150, et **pas** l'autre |
| T4 | la dérivation est écrite sous la forme prescrite | V5 : `(200 − 27 − 30) × 9 / 3` et `(150 − 27 − 30) × 9 / 3` ; `/ 3,0` absent |
| T5 | **les deux prompts d'architecte ont suivi** | V4 verte — les quatre paramètres et les deux bornes de référence des deux `system_prompt.md` égalent les constantes shell. C'est la garde d'atomicité ; elle est rouge avant D2/D3 et c'est son office. |
| T6 | les sondes W de porte 3 survivent | W1-W7 vertes sans retouche ; en particulier « le cœur » (`279 < 400 ≤ 429`) |
| T7 | X1 n'a pas régressé | les cinq sondes de plancher restent dérivées ; aucun littéral `57` ni `42` n'apparaît dans le harnais |
| T8 | X2 mesure, puis conclut | contre-vacuité : deux lignes d'exemple extraites, bornes non vides et distinctes ; puis égalité avec le résolveur |
| T9 | le harnais entier | `bash skills/bundled/_shared/test-dispatch-lib.sh` — zéro échec |
| T10 | le dépôt | `cargo test`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`, les lints structurels de CI, `scripts/verify-pipeline.sh origin/main` |

**Rouge avant / vert après.** V1, V2, V4 et V5 sont rouges à l'instant où D1 atterrit sans
D2-D4, et X2 est rouge si D1 §2 est omis. Les deux sens sont à constater pendant
l'implémentation, pas à supposer.

**Hors PR, à la main de l'opérateur :** un changement sous `skills/bundled/` n'atteint un
agent **que par `make deploy`** (chaîne mika#2340, *rebuild → seed → read*). Jusque-là,
les architectes continuent de lire `431`/`243` et les groomeurs de recevoir l'ancienne
règle. À dire dans le corps de PR ; hors périmètre du diff.

---

## 7. Ordre de commit

Un seul commit de travail. V4 est une garde de **parité** : tout découpage qui poserait
les constantes avant les prompts laisserait le harnais rouge entre deux commits, et la
PR porterait un état intermédiaire que rien ne justifie.

```
feat(groom): les quatre paramètres de la borne par vol, recalibrés sur n=7 (mika#2707)
  — dispatch-lib.sh      : 27 / 30 / 900 / 300, exemples travaillés et plancher
  — les deux system_prompt.md d'architecte : 429 / 279 et la nouvelle dérivation
  — test-dispatch-lib.sh : V1, V2, V5 re-épinglées ; sonde X2 neuve
  — solution-doc         : la note datée remplacée par la mesure, méthode corrigée
```

---

## 8. Hors périmètre

- **Le mécanisme de dérivation.** Seules les valeurs bougent — hors périmètre explicite
  du ticket, et la raison d'être de constantes nommées.
- **R1** (rien ne vérifie que le plafond déclaré par un plan est celui de son ticket) :
  déjà hors périmètre de mika#2701, et c'est un changement de contrat de canal.
- **Dériver les fixtures de W1** pour que la sonde « le cœur » survive à un `B200 < 400`
  (§ 4.3). Noté, non corrigé.
- **Faire lire la solution-doc par X2** (§ D6). Limite assumée et nommée.
- **`make deploy`.** Geste d'opérateur, pas de diff.

---

## 9. Risques

| risque | portée | parade |
|---|---|---|
| **n = 7 vient d'une seule journée** (2026-10-16) et de 2 plafonds sur 3 ; le palier 120 n'est pas observé | le palier 120 est extrapolé par la structure affine de la formule, pas mesuré | la formule est affine par construction (hors périmètre de ce ticket) ; le palier 120 n'existe que par surcharge `PILOT_MAX_TURNS` délibérée, donc sous l'œil d'un opérateur |
| la queue aberrante du vol n° 7 pourrait être le régime, pas l'exception | une queue réelle à 116 ferait mourir des vols que la borne autorise | le § 4.2 montre que le modèle **actuel** ne couvre pas davantage ce vol : le risque est préexistant, et ce plan ne l'aggrave pas. Le re-relevé après ~10 vols supplémentaires le tranchera ; c'est le même instrument, désormais posé. |
| le desserrage de +15 % au palier 150 autorise des plans plus gros, donc plus de volume par vol | une mort par volume | le composite du § 4.2 reste conservateur sur 6/7, et la marge de mika#2680 (4 tours) est la plus serrée du corpus — c'est elle qui a fixé le débit à 900 plutôt qu'à 1000 |
| le relevé a été fait au grooming ; le corpus aura grossi à l'implémentation | les valeurs du plan ne seraient plus les valeurs mesurées | T1 : contre-vacuité `n ≥ 7` et absence de reprise nouvelle. Le corpus **croît** ; un recalibrage sur n=7 reste valide sur n=12, et le re-relevé complet est un ticket de suivi, pas une condition de celui-ci. |

---

## Definition of Done

- Les quatre constantes de `dispatch-lib.sh` valent `27` / `30` / `900` / `300`, et le
  bloc de commentaire qui les précède porte les exemples travaillés et le plancher
  correspondants.
- Les deux `system_prompt.md` d'architecte portent `429` / `279` et la dérivation
  `(… − 27 − 30) × 9 / 3`.
- V1, V2 et V5 n'épinglent plus que des valeurs à jour ; les sondes de plancher restent
  dérivées ; V3, V4, W1-W7 et X1 sont vertes **sans retouche**.
- X2 existe, est armée, porte sa contre-vacuité, et tire si un exemple travaillé en prose
  diverge du résolveur.
- La solution-doc porte la note datée du 2026-10-16 **en remplacement** de celle du
  2026-10-15, et sa méthode de recalibrage porte les trois corrections du § 2.
- `bash skills/bundled/_shared/test-dispatch-lib.sh` : zéro échec. `cargo test`,
  `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`, les lints
  structurels et `scripts/verify-pipeline.sh origin/main` : verts.
- Le corps de PR porte `Closes senara-solutions/mika#2707`, le tableau du § 2.4, les
  quatre valeurs avec leur estimateur, et le rappel que `make deploy` est nécessaire pour
  que les architectes lisent les nouvelles bornes.

## Acceptance criteria

Le corps du ticket mika#2707 ne porte pas de section `## Acceptance criteria`. Les
critères ci-dessous sont dérivés de son § *Ce qu'il faudra toucher*, de sa § *Condition de
déclenchement*, de son § *Hors périmètre*, et du contrat de vérification du § 6.

- **AC1 — la condition de déclenchement est vérifiée, pas supposée.** L'implémentation
  re-passe la sonde de corpus avant de toucher une constante et constate `n ≥ 7` vols
  post-mika#2695 au format porte 3 ayant ouvert une PR, aucune reprise nouvelle parmi les
  sept. Un `n` inférieur déclenche une halte et une remontée à l'opérateur, pas un
  recalibrage sur un corpus qu'on n'a pas.
- **AC2 — les cinq sites bougent dans le même commit.** Les quatre constantes shell, les
  deux `system_prompt.md` d'architecte, les assertions de valeur du harnais et la
  solution-doc atterrissent ensemble. V4 est verte à la fin du commit et aurait été rouge
  à l'intérieur : l'atomicité est constatée, pas affirmée.
- **AC3 — les quatre valeurs sont celles du § 3.2**, chacune avec son estimateur nommé au
  site ou dans la solution-doc : `27` (max des 7), `30` (max des 6 hors aberrant), `900`
  (sous le min des 6 hors reprise), `300` (au-dessus du max des 6 hors reprise).
- **AC4 — la borne et le plancher sont ceux du § 3.3** : `429` à 200, `279` à 150, `189` à
  120, plancher `57`, et `0` avec `ceiling_below_pipeline_floor` à `57` et en dessous.
- **AC5 — aucune sonde dérivée n'est retapée en littéral.** Les cinq sondes de plancher de
  PR #2712 et les deux bornes de référence des sondes W restent calculées depuis le
  résolveur. X1 est verte sans retouche, et aucun littéral `57` ni `42` n'apparaît dans le
  harnais.
- **AC6 — le mécanisme n'a pas bougé.** `_plan_size_per_flight_bound` garde sa signature,
  ses quatre propriétés documentées, son plancher à zéro et ses deux notes. Le diff de ce
  fichier hors bloc de constantes et hors commentaire est vide.
- **AC7 — le détecteur X2 mesure avant de conclure.** Sa contre-vacuité extrait deux
  lignes d'exemple, bornes non vides et distinctes ; retirée, X2 serait verte sur un bloc
  muet, et cela doit être constaté à l'écriture.
- **AC8 — la note datée est remplacée.** La solution-doc ne porte plus qu'une note datée,
  celle du 2026-10-16, et ses trois corrections de méthode (unité, origine de la queue,
  détection des reprises) sont écrites avec l'argument qui les motive.
- **AC9 — le harnais et le dépôt sont verts.** `bash skills/bundled/_shared/test-dispatch-lib.sh`
  sans échec ; `cargo test`, `cargo clippy`, `cargo fmt --check`,
  `make verify-bundled-skills`, les lints structurels de CI et
  `scripts/verify-pipeline.sh origin/main` verts.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` (4 constantes + exemples travaillés + § plancher) | 20 |
| `skills/bundled/mika-arch-groom-ticket/system_prompt.md` | 2 |
| `skills/bundled/mika-arch-second-review/system_prompt.md` | 2 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — re-épinglage V1, V2, V5 | 20 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — sonde X2 (neuve, contre-vacuité comprise) | 40 |

Total estimé : 84 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Pourquoi un seul vol tient.** 84 lignes contre une borne de 243 et un seuil par PR de
1000 : la plus serrée des deux mord, et le plan passe dessous d'un facteur 2,9. Le volume
est intrinsèquement petit — quatre affectations, quatre lignes de prose, une trentaine
d'assertions re-épinglées et une sonde — parce que c'est exactement ce que des constantes
nommées existent pour permettre. Le coût réel de ce ticket était le **relevé**, et il est
déjà payé : il vit au § 2 de ce plan, pas dans le budget de tours du vol. Le
re-relevé de T1 est une commande et une comparaison, pas une analyse.

Les valeurs de la section ci-dessus sont celles des constantes **actuelles**, non
recalibrées : ce plan est jugé sous le régime qu'il modifie, et c'est le bon sens de
lecture.
