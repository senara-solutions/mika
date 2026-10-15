# feat(groom) — recalibrer `_PLAN_SIZE_FLIGHT_TAIL_TURNS` sur des vols terminés post-mika#2695

- **Ticket :** senara-solutions/mika#2707 (`p2-normal`, `ready`, `observability`, `dispatch:loop`)
- **Branche :** `feat/2707/groom-recalibrer-plan-size-flight-tail`
- **Date :** 2026-10-15
- **Lignée :** mika#2496 (plafond de tours) → mika#2542 (plafond par label) → mika#2636 (borne par PR) → mika#2693 (borne par vol, portes 1 et 2 — PR #2696, `dce41815`) → mika#2701 (porte 3, la mesure — PR #2708, `385504a9`) → **ce ticket** (le recalibrage que cette mesure rend possible).
- **Doctrine :** `docs/solutions/dev-loop/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md`, § *Comment recalibrer sur une distribution, et pas sur une mort* et § *L'instrument*.

---

## Contexte

`_PLAN_SIZE_FLIGHT_TAIL_TURNS=50` chiffre la **fin de pipeline** d'un vol : les
tours qui ne produisent pas de lignes. La constante a été posée sur trois vols
qui passaient par `ce-simplify-code` et `ce-code-review` ; mika#2694 / PR #2695
(`6f1e5a4a`) a retiré ces deux étapes des vols de pilote. La constante
**surestime donc la queue**, et la borne qui en dérive — `(plafond − 35 − 50) ×
750 / 200` — est plus serrée que nécessaire.

Le sens de l'erreur est **prudent** : borne plus serrée ⇒ plan plus petit ⇒
jamais une mort par volume. Rien n'est cassé. Le coût est des vols en trop.

**Ce qui fait la difficulté de ce ticket n'est pas le geste, c'est sa donnée.**
Le geste est trivial — quatre constantes nommées, et les valeurs seules bougent.
La donnée, elle, est un corpus qui n'existait pas quand le ticket a été écrit, et
dont l'état au moment de ce grooming est mesuré en D1. C'est pourquoi ce plan
est construit **autour d'une porte**, et non autour d'une valeur.

### L'état du corpus au 2026-10-15, mesuré

| fait | valeur | source |
|---|---|---|
| merge de mika#2695 (invalide la constante) | 2026-10-14 22:11 +0200 | `git log -1 6f1e5a4a` |
| merge de mika#2701 (pose l'instrument) | 2026-10-15 06:12 UTC | `git log -1 385504a9` |
| lignes `plan_size_estimate` relevées | **0** | `grep -rh '^dispatch-lib: plan_size_estimate ' ~/.mika/logs/` |

L'instrument a moins d'une journée, et il ne produit sa ligne que depuis un
binaire **déployé** après `385504a9` (`make deploy` → seed → lecture ; chaîne
mika#2340). La condition de déclenchement du ticket — **cinq vols post-#2695
ayant ouvert une PR** — ne peut donc pas être satisfaite au moment où ce plan
est écrit, et elle peut l'être au moment où le vol d'implémentation part. C'est
le vol qui mesure, pas le grooming : la mesure est datée, et un plan n'est pas
le bon endroit pour figer une donnée périssable.

---

## Décisions de grooming

Six points. D1 est la décision structurante ; D2 corrige un recensement
incomplet du corps du ticket ; D3 à D6 sont des choix de forme et de périmètre.

### D1 — La condition de déclenchement est une PORTE DU VOL, à trois branches

Le ticket écrit : « Avant ce seuil, il reste ouvert et n'est pas groomé ». Pris
au pied de la lettre, cela décrirait un grooming qui ne produit pas de plan — ce
qui n'est pas un état atteignable : le ticket porte `ready` et `dispatch:loop`,
il a été dispatché, et une session dispatchée qui s'arrête sans livrable est une
session morte. La lecture retenue est l'autre, et elle est plus forte : **la
condition est une porte, et c'est le vol qui la franchit ou non**, parce que le
vol est le seul des deux à tourner sur la machine qui porte les logs.

La porte a **trois** sorties, pas deux, et la troisième est celle que le corps du
ticket ne prévoit pas :

| branche | condition | ce que le vol fait |
|---|---|---|
| **A** | `n ≥ 5` **et** `max(queue) < 50` | recalibrage complet, geste atomique sur les quatre sites, `Closes #2707` |
| **B** | `n < 5` | **aucune constante touchée.** Livre X1 seul, consigne `n` mesuré, `Tracked in: #2707` — le ticket reste ouvert |
| **C** | `n ≥ 5` **et** `max(queue) ≥ 50` | **aucune constante touchée.** Livre X1 seul, consigne la mesure, **`Closes #2707`** : la prémisse est démentie, le ticket n'a plus d'objet |

**La branche C existe parce que le ticket présume son propre résultat.** « La
constante surestime donc désormais la queue » est une inférence, pas une mesure —
elle soustrait deux étapes d'un pipeline dont le ticket admet lui-même, en § (b),
que la soustraction naïve rend un reste faux. Un plan qui n'offrirait que A et B
rendrait le recalibrage inévitable dès que `n ≥ 5`, c'est-à-dire qu'il
transformerait l'instrument en formalité. Mesurer, c'est accepter de ne rien
changer.

Les trois branches livrent X1 (§ Conception L4) : c'est ce qui rend le vol utile
**quelle que soit** la branche, et ce qui interdit qu'un vol de branche B ou C
soit un vol pour rien.

### D2 — Le harnais porte HUIT ancres, pas trois

Le § *Ce qu'il faudra toucher* du ticket recense cinq sites et signale, à juste
titre, que le plan de mika#2701 en avait omis un (le harnais). Il en détaille le
contenu par « V1, V2 et V5 ». **Le relevé exhaustif en donne huit**, et les deux
manquants sont exactement du type qui survit à un recalibrage en mentant :

| # | ancre | lignes | ce qui est épinglé | sort après recalibrage de la queue |
|---|---|---|---|---|
| T1 | V1 | 12323-12336 | `431\|200`, `243\|150`, `131\|120`, `243\|150\|disarmed_fallback` | **rouge** |
| T2 | V1 (plancher) | 12331-12334 | `0\|85\|…floor`, `0\|70\|…floor` | **rouge sur 85**, vert sur 70 par coïncidence |
| T3 | V1 (contrôle négatif) | 12337-12338 | `assert_not_contains "431"` | vert, mais **pour la mauvaise raison** |
| T4 | V2 | 12344-12356 | cinq asserts `243\|150` / `431\|200` | **rouge** |
| T5 | V5 | 12435-12446 | « 431 lignes estimées » / « 243 lignes estimées » | **rouge** |
| T6 | V5 (dérivation) | 12452-12455 | `(200 − 35 − 50) × 7,5 / 2`, idem à 150 | **rouge** |
| T7 | V5 (format) | 12498 | `for t2693_val in 200 431 243` | vert, mais **n'interdit plus les bonnes valeurs** |
| T8 | **W3** (mika#2701) | 13511-13522 | plafonds `85` et `70` = le plancher et le sous-plancher | **rouge sur 85** |
| T9 | **V7-quater** | 12818-12824 | `assert_not_contains '− 35 − 50'`, `assert_contains '(200 − 35 − 50) × 7,5 / 2'` | un **rouge**, un vert **pour la mauvaise raison** |

Trois des neuf lignes restent **vertes** après un recalibrage incomplet : T3, T7
et la moitié `assert_not_contains` de T9. Ce sont des contrôles négatifs posés
sur des valeurs datées ; une fois la valeur changée, ils interdisent une chaîne
qui n'existe plus. Ils ne cassent rien, et c'est précisément le problème : ils
cessent de mesurer sans le dire. C'est la classe de défaut que X1 ferme.

**T2 et T8 sont la paire qui compte.** Le plancher vaut `orientation + queue` ;
si la queue descend à 35, le plancher descend à 70, et un `_t2693_bound_probe 85`
rend alors `56|85|resolved|` au lieu de `0|85|…floor`. Le ticket ne les nommait
pas. Les deux sites portent le **même** littéral pour la **même** raison, dans
deux blocs de test séparés par 1 200 lignes.

### D3 — Les deux bornes de référence restent LITTÉRALES dans V1 ; le plancher, non

La doctrine de ce fichier est posée par V4 de mika#2693 et reprise par W1 de
mika#2701 : « un test qui réécrirait 431 serait un troisième site de vérité ».
Elle s'applique aux tests qui **comparent deux sites** (prompt contre shell) — là,
recalculer est juste, parce que le test mesure un accord, pas une valeur.

Elle **ne s'applique pas** à V1, et l'inverser y serait une régression. V1 est la
seule assertion du système qui vérifie l'**arithmétique elle-même** : que
`(150 − 35 − 50) × 750 / 200` vaut bien `243`. Un V1 qui dériverait ses attentes
des constantes recalculerait la formule qu'il teste et passerait sur n'importe
quelle valeur, y compris une division par cent — exactement la régression que le
commentaire « ÉCRIT `900`, JAMAIS `9` » existe pour empêcher.

**Conduite retenue :** les paliers de borne (T1, T4, T5, T6) sont **retapés à la
main** avec les valeurs recalculées, et ce calcul à la main est une étape
d'implémentation explicite, pas un effet de bord. Le **plancher** (T2, T8), lui,
n'est pas une arithmétique mais une somme de deux constantes : il est dérivé, et
X1 interdit qu'il redevienne littéral.

### D4 — En branche B et C, la PR ne porte pas `Closes #2707` au même titre

Le canal par lequel le vol « remonte » est le **corps de la PR**, parce que c'est
le seul dont la disponibilité ne dépend d'aucune permission. Un commentaire
`gh issue comment` est un supplément, jamais le canal.

- **Branche B** : corps de PR portant le relevé (`n`, la liste des vols, la date),
  **pas** de `Closes`, et une ligne `Tracked in: senara-solutions/mika#2707`. Le
  ticket reste ouvert avec sa condition de déclenchement intacte.
- **Branche C** : corps de PR portant le relevé **et** la queue mesurée, avec
  `Closes #2707` : la mesure a répondu, la réponse est « la constante tient ».
  La note datée de la solution-doc est remplacée par cette mesure (§ D5).

Dans les deux cas le vol ouvre une PR et la PR est mergeable seule — c'est X1 qui
la rend telle.

### D5 — Ce qui NE bouge pas, et pourquoi chacun est un piège

Trois sites portent les mêmes nombres et **doivent rester en l'état** :

1. **`dispatch-lib.sh:3624-3628`** — le commentaire qui cite `` (`− 35 − 50) ×
   7,5 / 2`) `` comme la rédaction **rejetée** de `_PLAN_SIZE_FORMAT_SPEC`. C'est
   un récit historique : il dit ce qui a été écrit un jour et pourquoi ça a été
   retiré. Le mettre à jour le rendrait faux.
2. **`_PLAN_SIZE_FORMAT_SPEC`** (`dispatch-lib.sh:3632`) — déjà **à chevrons**
   depuis la revue de code de mika#2693 (V7-quater). Il ne porte aucune valeur ;
   il n'y a rien à y recalibrer. Y toucher ferait tirer V7-quater.
3. **Les plans archivés sous `docs/plans/`** — `2026-10-14-002` et
   `2026-10-15-001` portent `431` et la dérivation datée. Ce sont des archives
   horodatées, pas des sites de vérité.

Et le **mécanisme** ne bouge pas : `_plan_size_per_flight_bound`, sa signature,
son plancher, son repli `disarmed_fallback`, l'ordre des champs de la ligne de
porte 3. Hors périmètre explicite du ticket, et c'est la raison d'être des
constantes nommées.

### D6 — Si le corpus demande aussi de bouger le débit, V7-ter mord

Le ticket ne parle que de la queue, mais le même corpus rend aussi le **débit**
et le **facteur** non tronqués pour la première fois — la solution-doc, § 3 de la
méthode, dit « recalculer **les quatre** paramètres sur le corpus entier ».

Ce plan **borne le vol à la queue**, pour deux raisons. La première est le
périmètre du ticket, qui nomme `_PLAN_SIZE_FLIGHT_TAIL_TURNS` et lui seul. La
seconde est une contrainte de forme mesurée : **V7-ter** épingle que le débit et
le facteur sont des **multiples de 10**, parce que le rendu décimal de la
dérivation (`× 7,5 / 2`) est produit par `% 100 / 10` et devient faux en silence
sinon — « un débit recalibré à 775 ferait écrire × 7,7 / 2 (442) à côté d'une
borne de 445 ». Recalibrer le débit demande donc soit un arrondi au multiple de
10 assumé, soit un formateur décimal ; ni l'un ni l'autre n'est un changement de
valeur, et le second est un changement de mécanisme.

**Conduite retenue :** si le corpus montre que le débit ou le facteur dévient de
plus de 10 % de leur valeur actuelle, le vol le **consigne** dans le corps de PR
et ouvre un ticket de suivi nommé en `Tracked in:` — il ne les change pas.

---

## Conception

### L1 — La méthode de relevé, et ce qu'elle joint

L'étape 1 de la solution-doc demande un quadruplet par vol. La ligne de porte 3
en donne la moitié sans jointure ; l'autre moitié se lit sur le vol.

**Relevé (ancré sur `^dispatch-lib: `, jamais sur le jeton nu — mika#2050 a
mesuré le faux positif d'une session qui *discutait* du signal) :**

```sh
grep -rh '^dispatch-lib: plan_size_estimate ' ~/.mika/logs/
```

Pour chaque ligne relevée, le vol constitue :

| terme | d'où il vient |
|---|---|
| plafond | `ceiling=` de la ligne — l'argv réellement passé, pas les labels |
| estimé | `total_loc=` de la ligne |
| borne en vigueur | `per_flight_bound_loc=` de la ligne |
| réel hors `docs/` | `git show --numstat` sur les commits de la branche du vol |
| premier `Edit`, dernier commit de travail, dernier tour | lus sur le vol : `~/.mika/data/pilot-transcripts/<session>.jsonl` et `git log` de la branche |

**Critère d'admission au corpus**, et il est strict : le vol doit être
**postérieur à `6f1e5a4a`** (2026-10-14 22:11) *et* avoir **ouvert une PR**. Un
vol mort avant la PR ne donne pas de fin de pipeline — c'est l'étape 2 de la
méthode, et c'est ce qui disqualifiait les n=3 d'origine. Le compte `n` est le
nombre de vols qui passent **les deux** filtres, et il est écrit dans le corps de
PR avec la liste nominative : un `n` sans liste n'est pas vérifiable.

### L2 — La statistique retenue pour la queue, et pourquoi

Pour chaque vol admis : `queue = dernier_tour − tour_du_dernier_commit_de_travail`.

```
_PLAN_SIZE_FLIGHT_TAIL_TURNS = ⌈ max(queue) / 5 ⌉ × 5
```

**Le maximum, et non la moyenne ni la médiane.** À `n = 5`, aucune statistique de
centile n'est définie ; le maximum est le 80ᵉ centile empirique, et c'est
l'estimateur dont la direction d'erreur est celle que la doctrine impose —
« l'omission va toujours vers la borne la plus serrée ». Une moyenne mettrait la
moitié des vols au-dessus de la queue budgétée, c'est-à-dire qu'elle rendrait la
mort par volume de nouveau atteignable pour corriger un coût qui n'est qu'un vol
de trop. **À `n ≥ 10`, le 90ᵉ centile remplace le maximum** — le plan le dit pour
que le prochain recalibrage n'ait pas à re-trancher.

**Arrondi au multiple de 5 supérieur**, parce que la précision du relevé (un
numéro de tour lu sur un transcript) ne justifie pas l'unité, et que les quatre
paramètres du site sont des entiers ronds. L'arrondi va vers le haut : même
direction d'erreur.

**Garde de branche C :** si `max(queue) ≥ 50`, la constante actuelle ne
surestimait pas, et le vol ne la touche pas (D1).

### L3 — Le geste, dans l'ordre où V4 le rend sûr

V4 de mika#2693 compare les quatre paramètres littéraux des deux prompts
architecte aux **constantes shell**, et leurs deux bornes de référence à ce que
le **résolveur calcule**. C'est ce qui rend le geste atomique : changer la
constante seule laisse V4 rouge sur six assertions, et rien ne peut merger.

L'ordre d'implémentation suit donc le sens de la garde — on casse d'abord, on
répare ensuite, et le vert final est la preuve que les quatre sites ont bougé
ensemble :

```
1. dispatch-lib.sh      _PLAN_SIZE_FLIGHT_TAIL_TURNS=<T>     → V4 rouge, V1 rouge, W3 rouge
2. dispatch-lib.sh      le commentaire de dérivation (772-782)
3. les deux prompts     paramètres + bornes de référence     → V4 vert
4. le harnais           les neuf ancres de D2                → V1/V2/V5/W3/V7-quater verts
5. X1                   le détecteur de plancher dérivé      → vert, et armé
6. la solution-doc      la note datée remplacée par la mesure
```

Les **nouvelles valeurs de borne** des étapes 3 et 4 se retapent à la main (D3),
mais elles se **vérifient** contre le résolveur avant d'être écrites : le vol lit
`_t2693_bound_probe 200` et `_t2693_bound_probe 150` **après** l'étape 1, et
compare. Écrire d'abord et vérifier ensuite, c'est le seul ordre qui distingue
une arithmétique tenue d'une arithmétique recopiée — et c'est ce que D3 exige de
garder littéral sans pour autant le laisser faux.

### L4 — X1 : le plancher est dérivé, et un littéral de plancher est refusé

**Le défaut que X1 ferme est mesuré sur ce ticket même** : le corps du ticket
recense trois ancres de test et en oublie deux, dont T8 (W3), qui porte le
plancher `85` pour exactement la même raison que T2 (V1) et dans un bloc situé
1 200 lignes plus loin. Le prochain recalibrage de la queue retombera dans le même
trou, parce que rien dans le fichier ne relie ces deux littéraux à la somme dont
ils dérivent.

X1 est composé de deux moitiés, et la seconde est la contre-vacuité de la
première :

1. **Le plancher est dérivé.** Les sondes de T2 et T8 prennent
   `$T2707_FLOOR = _PLAN_SIZE_FLIGHT_ORIENTATION_TURNS + _PLAN_SIZE_FLIGHT_TAIL_TURNS`
   et `$T2707_UNDER = T2707_FLOOR - 15` au lieu de `85` et `70`. Ce n'est pas
   tautologique : la valeur attendue reste `0|…|ceiling_below_pipeline_floor`,
   c'est-à-dire le **comportement**, jamais la formule.
2. **Un littéral de plancher est refusé.** Un scan de source compte, dans les
   deux blocs de test (`mika#2693` et `mika#2701`), les appels de sonde dont
   l'argument de plafond est le littéral égal au plancher courant. Attendu : `0`.
   C'est la moitié qui tire quand quelqu'un réintroduit la forme, et c'est la
   seule des deux qui soit un détecteur.

Le scan est ancré sur les noms de sonde (`_t2693_bound_probe`,
`_t2701_emit_probe`) et sur un argument numérique en fin de ligne, jamais sur le
nombre nu : `85` apparaît ailleurs dans un fichier de 13 000 lignes, et un
`grep -c 85` serait vert pour la mauvaise raison le jour où il compte un numéro
de ligne dans un commentaire.

**Contre-vacuité du scan lui-même** : les deux blocs sont extraits et assertés
**non vides** avant qu'on conclue quoi que ce soit de leur contenu — une section
renommée ou déplacée rendrait `0` en ayant mesuré le vide. C'est la garde que V6
point 2 pose déjà pour le bloc mika#2693, reprise telle quelle.

---

## Étapes d'implémentation

1. **Relever le corpus** (L1). Écrire le relevé — `n`, la liste nominative des
   vols, leurs quadruplets — dans `.pilot-scratch/2707-corpus.md`. Ce fichier est
   un brouillon : il reste en place, il n'est jamais supprimé (mika#2548).
2. **Franchir la porte** (D1). Déterminer la branche A, B ou C et l'**écrire** en
   tête du brouillon, avec la valeur de `n` et de `max(queue)` qui la décide.
3. **Branches B et C :** passer directement à l'étape 7 (X1 seul). Ne toucher
   **aucune** constante, **aucun** prompt, **aucune** ancre de D2.
4. **Branche A — étape 1 de L3 :** poser `_PLAN_SIZE_FLIGHT_TAIL_TURNS=<T>`, puis
   réécrire le commentaire de dérivation (`dispatch-lib.sh:760-782`) : la ligne
   `borne = (plafond − 35 − <T>) × 750 / 200` et les deux paliers chiffrés.
   Vérifier par `_t2693_bound_probe 200` / `150` que les nouvelles bornes sont
   bien celles qu'on s'apprête à retaper (D3).
5. **Branche A — étape 3 de L3 :** les deux `system_prompt.md`. Trois lignes
   chacun : la ligne d'exemple `Borne par vol :`, la ligne
   `**Per-flight parameters:**`, la ligne `**Reference bounds:**`. Faire tourner
   V4 : vert ⇒ les quatre sites sont d'accord.
6. **Branche A — étape 4 de L3 :** les neuf ancres de D2, dans l'ordre du tableau.
   Les trois qui restaient **vertes pour la mauvaise raison** (T3, T7, la moitié
   `assert_not_contains` de T9) sont réécrites avec les nouvelles valeurs, pas
   laissées en l'état.
7. **Toutes branches :** livrer X1 (L4), dans le bloc de test mika#2693, juste
   après V7-quater, sous un titre `--- X1 : le plancher est dérivé ---`. Vérifier
   la **rougeur par mutation** : réintroduire `_t2693_bound_probe 85` en littéral
   doit faire tirer la moitié 2.
8. **Toutes branches :** la solution-doc. En branche A, la § *La dérivation* (le
   tableau des quatre paramètres, la ligne « Soit **431** … »), la §
   *Arithmétique entière* (les deux paliers), la mention du plancher `≤ 85`, et la
   **note datée est remplacée par la mesure** — « à remplacer, pas à compléter ».
   En branches B et C, seule la note datée bouge, et elle porte `n`, la date du
   relevé et la décision.
9. `make verify-bundled-skills`, `bash skills/bundled/_shared/test-dispatch-lib.sh`,
   `cargo test`, `cargo clippy`, `scripts/verify-pipeline.sh origin/main`.
10. Corps de PR selon D4 : relevé, branche franchie, et `Closes` ou `Tracked in:`
    selon le cas. Écrit dans `pr-body.md` à la racine du worktree (mika#2211).

---

## Contrat de vérification

### X1 — le plancher est dérivé, et un littéral de plancher tire

**Rouge avant :** la moitié 2 n'existe pas ; les deux sondes de plancher portent
`85` en littéral à deux endroits du fichier.

**Rouge par mutation, à consigner :** remplacer `$T2707_FLOOR` par `85` dans la
sonde de T2 fait tirer la moitié 2 ; vider l'un des deux blocs extraits fait
tirer la contre-vacuité.

| sonde | attendu |
|---|---|
| `_t2693_bound_probe $T2707_FLOOR` | `0\|<plancher>\|resolved\|ceiling_below_pipeline_floor` |
| `_t2693_bound_probe $T2707_UNDER` | `0\|<plancher−15>\|resolved\|ceiling_below_pipeline_floor` |
| `_t2701_emit_probe 400 $T2707_FLOOR` | `per_flight_bound_loc=0 … per_flight_note=ceiling_below_pipeline_floor` |
| scan des deux blocs, sondes à plafond littéral `= plancher` | `0` |
| les deux blocs extraits | non vides, assertés avant le scan |
| `$T2707_FLOOR` | égal à `orientation + queue`, et **non nul** |

La dernière ligne est la contre-vacuité arithmétique : un `$T2707_FLOOR` vide
rendrait `_t2693_bound_probe ""` — c'est-à-dire la sonde de **repli désarmé**,
qui rend `243|150|disarmed_fallback|` et non un plancher. Le test passerait en
mesurant une autre branche du résolveur.

### Les gardes existantes, et leur état attendu

| garde | branche A | branches B et C |
|---|---|---|
| V1 (T1, T2, T3) | réécrite, verte | inchangée, verte sauf T2 adaptée à X1 |
| V2 (T4) | réécrite, verte | inchangée, verte |
| V4 (parité prompts ↔ shell) | **verte, et c'est la preuve de l'atomicité** | inchangée, verte |
| V5 (T5, T6, T7) | réécrite, verte | inchangée, verte |
| V6 (anti-vacuité, hermétisme, table d'exceptions) | verte | verte |
| V7-ter (débit et facteur multiples de 10) | verte — ni l'un ni l'autre ne bouge (D6) | verte |
| V7-quater (T9) | réécrite, verte | inchangée, verte |
| W1–W7 (mika#2701), dont T8 | W3 réécrite ; W1/W2/W4/W5/W6/W7 dérivent déjà et restent vertes | W3 adaptée à X1 |

**V4 est la garde qui rend le geste sûr, et c'est aussi celle qui le rend
atomique.** Un vol de branche A qui la laisserait rouge n'a pas fini.

### Vérification de non-régression du canal

Le plan ne change aucun mécanisme, mais il change la valeur qu'un canal
transporte. Deux contrôles de bout en bout :

- `_t2693_inject "dev-groom" "150"` rend une règle dont la ligne `Borne par vol :`
  porte la **nouvelle** borne et la **nouvelle** dérivation, et aucun placeholder
  `__…__` ne survit.
- `_t2701_emit_probe 400 150` rend une ligne `plan_size_estimate` dont
  `per_flight_bound_loc=` porte la nouvelle borne, les sept champs présents et
  `plan=` toujours **dernier**.

---

## Risques nommés

| # | risque | traitement |
|---|---|---|
| R1 | **Le corpus n'atteint pas 5 au moment du vol** — probable au 2026-10-15 : l'instrument a moins d'un jour. | Branche B de D1. Le vol livre X1, consigne `n`, laisse le ticket ouvert. Ce n'est pas un échec de vol. |
| R2 | **La mesure dément la prémisse du ticket** (`max(queue) ≥ 50`). | Branche C de D1. Le ticket se ferme sur la mesure. Prévu, pas subi. |
| R3 | **Un recalibrage partiel laisse trois assertions vertes pour la mauvaise raison** (T3, T7, T9-négatif). | Recensées nommément en D2, réécrites à l'étape 6. X1 ferme la classe pour le plancher. |
| R4 | **T2 et T8 oubliés** — le défaut exact que le corps du ticket a commis. | X1, moitié 2 : le scan tire sur tout littéral de plancher réintroduit. |
| R5 | **Le corpus mélange des vols de grooming et d'implémentation.** Seul le dispatch d'implémentation servant un plan émet la ligne ; un vol de groom n'en a pas. | Le filtre est la ligne elle-même : pas de ligne, pas d'entrée au corpus. Le critère « a ouvert une PR » de L1 est le second filtre. |
| R6 | **Le relevé est fait sur une machine dont les logs sont partiels** (rotation, conteneur recréé). | Le corps de PR porte la **liste nominative** des vols, pas seulement `n`. Un relevé non vérifiable n'est pas un relevé. |
| R7 | **Le débit et le facteur dévient aussi.** | D6 : consignés, jamais changés dans ce vol ; ticket de suivi nommé en `Tracked in:`. |

---

## Hors périmètre

- **Changer le mécanisme de dérivation** — `_plan_size_per_flight_bound`, sa
  signature, son plancher, son repli `disarmed_fallback`, l'ordre des champs de
  la ligne de porte 3. Seules les valeurs bougent ; c'est la raison d'être des
  constantes nommées (ticket, § *Hors périmètre*).
- **Recalibrer le débit ou le facteur** — D6. Consignés, pas changés.
- **Fermer R1 de mika#2701** (rien ne vérifie que le plafond déclaré par un plan
  est celui de son ticket). Déjà hors périmètre de mika#2701 ; c'est un
  changement de contrat de canal, pas un recalibrage.
- **Les plans archivés sous `docs/plans/`** — D5, point 3.
- **Le commentaire historique de `dispatch-lib.sh:3624-3628`** — D5, point 1.
- **Généraliser le rendu décimal de la dérivation** (le formateur que V7-ter rend
  nécessaire si le débit cesse d'être un multiple de 10) — D6.

---

## Definition of Done

- [ ] Le corpus est relevé, et `n` ainsi que la liste nominative des vols sont
      écrits dans `.pilot-scratch/2707-corpus.md` puis repris dans le corps de PR.
- [ ] La branche franchie (A, B ou C) est nommée explicitement dans le corps de
      PR, avec la valeur qui la décide.
- [ ] **Branche A uniquement :** les quatre sites de valeur bougent dans le même
      commit — constante shell, commentaire de dérivation, deux prompts
      architecte, neuf ancres du harnais.
- [ ] X1 est livré, armé, et sa rougeur par mutation est consignée dans le corps
      de PR.
- [ ] La note datée de la solution-doc est **remplacée** par la mesure, jamais
      complétée.
- [ ] `bash skills/bundled/_shared/test-dispatch-lib.sh` vert, V4 compris.
- [ ] `make verify-bundled-skills`, `cargo test`, `cargo clippy`,
      `scripts/verify-pipeline.sh origin/main` verts.
- [ ] Le corps de PR porte `Closes #2707` (branches A et C) ou
      `Tracked in: senara-solutions/mika#2707` (branche B), jamais les deux.

---

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria`. Les critères
ci-dessous sont dérivés de son § *Condition de déclenchement*, de son § *Ce qu'il
faudra toucher* et du contrat de vérification de ce plan.

- **AC1 — La porte est franchie sur une mesure, jamais sur une présomption.** Le
  vol relève le corpus avant de toucher quoi que ce soit, et le corps de PR porte
  `n`, la liste nominative des vols admis, et la date du relevé. Un `n` sans
  liste ne satisfait pas AC1.
- **AC2 — Sous le seuil, aucune constante ne bouge.** Si `n < 5`, le diff ne
  contient **aucune** modification de `_PLAN_SIZE_FLIGHT_*`, des deux
  `system_prompt.md` d'architecte, ni des neuf ancres de valeur de D2. Le ticket
  reste ouvert.
- **AC3 — La mesure peut dire non.** Si `n ≥ 5` et `max(queue) ≥ 50`, aucune
  constante ne bouge non plus, et le ticket se ferme sur la mesure consignée. Un
  plan qui ne rendrait que « recalibré » ou « pas assez de données » ne satisfait
  pas AC3.
- **AC4 — Le geste est atomique.** En branche A, les quatre sites de valeur
  bougent dans **un seul commit**, et `V4` est verte à la fin. Un commit
  intermédiaire laissant V4 rouge est admis pendant le travail, jamais dans le
  diff final d'un commit isolé.
- **AC5 — Les neuf ancres, pas trois.** En branche A, les neuf lignes du tableau
  de D2 sont traitées, y compris les trois qui seraient restées **vertes** (T3,
  T7, T9-négatif) et les deux que le corps du ticket omettait (T2, T8).
- **AC6 — Le plancher ne redevient pas littéral.** X1 est livré dans toutes les
  branches, armé, avec sa contre-vacuité, et sa moitié 2 rend `0`. Réintroduire
  `_t2693_bound_probe 85` en littéral le fait tirer.
- **AC7 — V1 garde ses bornes littérales.** Les paliers `200`, `150`, `120` de V1
  portent des valeurs attendues **retapées à la main**, jamais dérivées des
  constantes : V1 est la seule assertion qui vérifie l'arithmétique (D3).
- **AC8 — La note datée est remplacée.** La solution-doc ne porte pas, à la fin
  du vol, à la fois l'ancienne note « 50 surestime la queue » et la nouvelle
  mesure.
- **AC9 — Aucune régression de canal.** `_t2693_inject "dev-groom" "150"` rend la
  règle avec la borne et la dérivation en vigueur et zéro placeholder survivant ;
  `_t2701_emit_probe 400 150` rend les sept champs avec `plan=` en dernier.

---

## Fire-Disposition

Ce plan livre **un détecteur** : X1, moitié 2 — le scan de source qui refuse un
littéral de plafond égal au plancher dans les blocs de test mika#2693 et
mika#2701. Son chemin de succès est « aucune violation trouvée ».

**Option retenue : (a) exception nommée en allowlist, table DÉCLARÉE et VIDE.**

C'est le patron déjà posé par V6 point 4 dans le même fichier, et il est repris
tel quel :

```sh
# FIRE-DISPOSITION, option (a) : la table d'exceptions du scan de X1 est
# DÉCLARÉE et VIDE, et son vide est asserté à l'exécution. Déclarée pour que le
# prochain qui veut y mettre une entrée voie d'abord qu'elle existe exprès ;
# assertée vide pour qu'une entrée ajoutée en silence tire. Le détecteur part
# ARMÉ — doctrine mika#2201 : on route le site vers la règle, on n'ajoute pas
# d'exception.
T2707_FLOOR_LITERAL_ALLOWLIST=()
```

**Le détecteur part armé et vert, dans les trois branches.** C'est la propriété
qui rend l'option (a) juste ici plutôt que l'option (b) : il n'y a **aucune
violation existante à excuser**. Après l'étape 7, les deux seules sondes de
plancher du dépôt (T2 et T8) prennent `$T2707_FLOOR` ; le scan rend `0` dès le
premier run, et il n'y a rien à mettre dans la table.

En branches B et C — où la queue ne bouge pas — le plancher reste `85`, le scan
rend `0` tout autant, et le détecteur mord au **prochain** recalibrage. C'est
exactement son office : il est livré par le vol qui a failli oublier les deux
sites, pour que celui d'après ne les oublie pas.

L'assertion auto-nettoyante demandée par (a) est la ligne
`assert_eq "X1: table d'exceptions — zero entries" "0" "${#T2707_FLOOR_LITERAL_ALLOWLIST[@]}"` :
elle rougit dès qu'une entrée est ajoutée sans que quelqu'un ait à y revenir.

---

## Taille estimée

Le périmètre chiffré est celui de la **branche A**, la plus large des trois.
Les branches B et C livrent le seul X1 et la note datée, soit ≈ 37 lignes.

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — la constante + le commentaire de dérivation | 14 |
| `skills/bundled/mika-arch-groom-ticket/system_prompt.md` — 3 lignes de valeur | 3 |
| `skills/bundled/mika-arch-second-review/system_prompt.md` — 3 lignes de valeur | 3 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — les neuf ancres de D2 recalibrées | 30 |
| `skills/bundled/_shared/test-dispatch-lib.sh` — X1 et sa Fire-Disposition | 35 |

Total estimé : 85 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 243 lignes estimées — (150 − 35 − 50) × 7,5 / 2

**Un seul vol, et voici pourquoi il tient.** 85 lignes, soit 35 % de la borne. Le
geste est un changement de **valeurs** sur des sites déjà nommés et déjà
recensés — D2 en donne les neuf ancres avec leurs numéros de ligne, et V4 dit
mécaniquement quand il est complet. Aucune lecture de code neuve n'est nécessaire
à l'implémentation : le plan a déjà fait le relevé.

Le poste qui consomme des tours ici n'est pas l'écriture, c'est le **relevé du
corpus** (L1) — une jointure manuelle sur `n` transcripts. À `n = 5` elle tient
largement dans les 65 tours de travail que le plafond de 150 laisse ; au-delà de
`n = 15`, le vol consignerait le relevé et s'arrêterait là, ce qui est la branche
B avec un `n` grand — cas assez improbable pour ne pas découper le vol par
avance, et assez nommé pour ne pas surprendre.
