# feat(groom) — porte 3 : la ligne `plan_size_estimate` porte la borne PAR VOL et le plafond RÉELLEMENT passé en argv

- **Ticket :** senara-solutions/mika#2701 (`p2-normal`, `ready`, `observability`, `dispatch:loop`, `loop-substrate`)
- **Branche :** `feat/2701/groom-porte-3-la-ligne-plan-size`
- **Date :** 2026-10-15
- **Lignée :** mika#2496 (plafond de tours, `_pilot_max_turns`) → mika#2542 (plafond par label, `PILOT_LABEL_TURN_CEILINGS`) → mika#2636 (borne par PR, `_plan_size_max_loc` + porte 3 en lignes) → mika#2693 (borne par vol, portes 1 et 2 — PR #2696, `dce41815`) → **ce ticket** (porte 3, la mesure par vol).
- **Doctrine :** `docs/solutions/dev-loop/une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md`, § *La seconde borne est en TOURS* et § *Comment recalibrer sur une distribution*.

---

## Contexte

mika#2693 a livré les deux portes qui **décident** de la borne par vol :

```
porte 1, le PROMPT du groomeur   `_PLAN_SIZE_RULE`          intention   ← livrée #2693
porte 2, l'ARCHITECTE            Plan-Size Gate L2/L3       REFUS       ← livrée #2693
porte 3, le DISPATCH implement   `plan_size_estimate`       MESURE      ← CE TICKET
```

Porte 3 existe déjà **en lignes** depuis mika#2636 : la ligne émise par
`_emit_pilot_budget_line` porte `total_loc=`, `threshold_loc=`, `verdict=` et
`plan=`. Il lui manque la seconde moitié — la borne par vol, et le plafond dont
elle dérive.

**Ce qui est en jeu n'est pas le confort de lecture.** Les quatre paramètres du
site de vérité (`_plan_size_per_flight_bound`) sont posés sur **n = 3**, et les
trois points sont des vols **morts** — donc tronqués dans le même sens (R2 du
plan de #2693 ; la méthode de recalibrage de la solution-doc dit explicitement
que « les vols qui ont **fini** sont les seuls à donner une fin de pipeline et un
débit non tronqués »). Sans cette ligne, le quadruplet (plafond, estimé, réel,
tours) doit être reconstitué à la main, par vol, sur des transcripts. Avec elle,
trois des quatre termes sont déjà dans le `.stderr` forensique, à côté de
`pilot_budget_armed`, sans jointure.

C'est aussi la seule sonde qui **verrait R1** : rien ne vérifie que le plafond
déclaré par un plan est celui de son ticket, parce que `_arch_ask` ne livre à
l'architecte que le contenu du fichier de plan sur stdin (D2 de #2693). Un plan
qui écrirait `Plafond de tours : 200` sur un ticket sans `loop-substrate`
obtiendrait la borne large en silence. La ligne de porte 3, elle, dérive le
plafond de l'**argv réellement passé** à claude-pilot : la comparaison entre ce
que le dispatch a armé et ce que le plan a déclaré devient possible.

---

## Décisions de grooming

Cinq points. Les quatre premiers sont des choix de forme ; le cinquième est un
**descopage, et il est argumenté contre le corps du ticket lui-même**.

### D1 — Le plafond vient de `$_mt`, c'est-à-dire de l'argv, et de rien d'autre

`_emit_pilot_budget_line` parcourt déjà `"$@"` à la recherche de `--max-turns`
et pose le résultat dans `_mt` (`none` quand l'option est absente). C'est
**exactement** la source que l'AC1 exige : « dérivés du plafond passé en argv et
non d'une relecture des labels ».

Trois raisons de ne pas lire `$_PILOT_MAX_TURNS` à la place, alors qu'il est
dans la portée :

1. **mika#2542 AC5 — `PILOT_LABEL_TURN_CEILINGS` garde un seul lecteur.** Relire
   les labels ici serait la seconde table de classification de mika#2539.
2. **`$_PILOT_MAX_TURNS` est un état posé par une autre fonction.** Le lire sans
   l'avoir résolu sur la même invocation logique est précisément ce que le scan
   de co-location de mika#2496 refuse — et le résoudre ici ferait de cette
   fonction un second décideur de plafond.
3. **L'argv est ce que le pilote a vraiment reçu.** C'est le seul terme qui ne
   peut pas mentir sur ce que le vol avait. Un plafond relu ailleurs mesurerait
   l'intention ; `$_mt` mesure le fait.

### D2 — `ceiling=` porte le plafond RÉSOLU, pas la valeur brute d'argv

Sur budget désarmé, `--max-turns` n'est pas passé du tout
(`${_PILOT_MAX_TURNS:+--max-turns …}`), donc `_mt` vaut `none`. La ligne pourrait
écrire `ceiling=none`. Elle écrit **`ceiling=150 ceiling_source=disarmed_fallback`**,
et c'est délibéré : sans le 150, le `per_flight_bound_loc=243` qui l'accompagne
serait un nombre sans dérivation possible — « un nombre plausible, présenté avec
autorité », la classe que l'ancre de `_PLAN_SIZE_TOTAL_RE` a déjà dû fermer.

Rien n'est perdu : la valeur brute d'argv est sur la ligne **immédiatement
au-dessus**, `pilot_budget_armed max_turns=none source=env`. Les deux lignes
consécutives portent les deux faits, et `ceiling_source=` est le lien explicite
entre elles.

### D3 — `under_bound` / `over_bound`, et non `under_threshold` / `over_threshold`

La ligne portera désormais **deux verdicts** dans **deux unités**. Réutiliser le
vocabulaire du verdict par PR ferait qu'un `grep over_threshold` sur le corpus de
logs apparierait indifféremment une borne en lignes-par-PR et une borne en
lignes-dérivées-de-tours. C'est le défaut nommé par le titre même de la
solution-doc de ce gate : *une estimation de taille se compare à un seuil dans sa
propre unité*. Deux unités, deux vocabulaires.

Les deux champs restent distinguables par leur **nom** (`verdict=` vs
`per_flight_verdict=`), mais un grep sur la valeur seule est la forme qu'un
opérateur tape réellement, et c'est elle qu'il faut rendre sûre.

### D4 — La note est conditionnelle ; un argv illisible a sa propre ligne

`_PLAN_SIZE_PER_FLIGHT_NOTE` est vide au cas nominal. Elle est donc émise comme
`label=` l'est depuis mika#2542 : **seulement quand elle dit quelque chose**, car
un champ vide se lirait comme une note nommée « vide ».

Et un `_mt` qui ne serait ni `none` ni une suite de chiffres n'est **pas** avalé
en silence : il obtient une ligne `plan_size_flight_ceiling_invalid`, du même
dessin que les deux lignes `*_invalid` que cette fonction porte déjà
(`pilot_budget_invalid`, `plan_size_threshold_invalid`), puis la borne retombe
sur le repli. Population mesurée aujourd'hui : **vide** — `_pilot_max_turns`
n'assigne jamais que `""` ou des chiffres. Le garde-fou existe pour la seconde
raison, qui est opératoire : `_plan_size_per_flight_bound` fait de
l'**arithmétique** sur son argument, et l'argv est la seule entrée de cette
fonction qui ne soit pas produite par un résolveur de la maison.

### D5 — Le recalibrage de `_PLAN_SIZE_FLIGHT_TAIL_TURNS` N'EST PAS dans ce vol

Le ticket en fait un § entier : « À recalibrer dans le même geste ». **Ce plan ne
le fait pas**, et voici les trois raisons, dont deux sont écrites dans le ticket
lui-même.

**(a) Deux de ses propres critères d'acceptation l'interdisent.**

- **AC4 :** « la borne par PR de mika#2636 et les sondes V1–V7 de mika#2693
  passent **sans modification** ». Or V1 épingle en littéral `431|200|resolved|`,
  `243|150|resolved|`, `131|120|resolved|`, `0|85|…floor`, `0|70|…floor` ; V2
  épingle `243|150` et `431|200` ; V5 épingle `« 431 lignes estimées »` et
  `« (200 − 35 − 50) × 7,5 / 2 »`. Changer `_PLAN_SIZE_FLIGHT_TAIL_TURNS` rend
  **rouges** une quinzaine d'assertions de V1/V2/V5, qu'il faudrait donc
  modifier. Le § « Attention au geste » du ticket recense quatre sites tenus par
  V4 — les constantes shell, les deux prompts, les deux bornes de référence — et
  **omet le harnais**, qui est le cinquième et qui est verrouillé par AC4.
- **AC3 :** « la note `ceiling_below_pipeline_floor` sur un plafond **≤ 85** ».
  85 = 35 + 50, c'est-à-dire le plancher **actuel**. Recalibrer la queue à 35
  déplacerait le plancher à 70 et rendrait l'AC3 fausse telle qu'elle est écrite.

Deux ACs indépendants décrivent une PR qui **ne change pas** la constante.

**(b) La donnée n'existe pas encore, et le ticket le dit.** « Le reste de la
queue — vérification complète, compound, PR — est ce qu'il faut remesurer », et
« le recalibrage demande une mesure sur des vols post-#2695 — **ce que cette
instrumentation produit précisément** ». C'est circulaire : on ne mesure pas avec
un instrument qu'on n'a pas encore posé. La seule mesure disponible
(vol de #2693 : queue ~80 tours, dont ~41 de simplification et ~41 de revue) rend
par soustraction un reste de **~0 tour** pour la vérification, le compound et la
PR — ce qui est faux, puisque mika#2687 vol 1 est mort *dans* la vérification
après 15 tours de queue. La soustraction naïve donnerait un plancher à 35 et une
borne de 487/300 posée sur rien.

**(c) La méthode de recalibrage de la maison l'exclut explicitement.** La
solution-doc, § *Comment recalibrer sur une distribution, et pas sur une mort* :
« n=3 n'est pas une distribution […] les vols qui ont **fini** sont les seuls à
donner une fin de pipeline et un débit non tronqués ». Post-#2695, le corpus de
vols finis compte **zéro** élément au moment d'écrire ce plan.

**Conduite retenue.** Le recalibrage part dans un ticket de suivi, ouvert
**avant** la PR par le pilote d'implémentation (étape 6), dont le numéro — et
jamais #2701 — est porté en `Tracked in:` du corps de PR. C'est la correction
directe du défaut que ce ticket-ci a dû réparer à la main : le `Tracked in:` de
#2693 pointait #2693, clos au merge, donc le report n'avait plus de tracker
ouvert. Un report dont le tracker est l'issue qu'on ferme n'est pas un report.

---

## Conception

### L1 — `_emit_pilot_budget_line` : quatre champs, une seule ligne, un seul site

Tout vit dans la branche existante `if [ -n "${_PLAN_SIZE_PLAN_PATH:-}" ]`, et
la ligne émise reste **une seule** — S10 de mika#2636 asserte exactement une
occurrence non commentée de `plan_size_estimate total_loc=` dans le fichier, et
cette assertion est sous AC4.

Forme finale (une ligne, coupée ici pour la lecture) :

```
dispatch-lib: plan_size_estimate total_loc=<t> threshold_loc=<T> verdict=<v>
  per_flight_bound_loc=<b> ceiling=<c> ceiling_source=<resolved|disarmed_fallback>
  per_flight_verdict=<under_bound|over_bound|unknown>[ per_flight_note=<n>]
  plan=<chemin>
```

Quatre exemples, et ils sont le contrat :

```
… verdict=under_threshold per_flight_bound_loc=431 ceiling=200 ceiling_source=resolved per_flight_verdict=under_bound plan=docs/plans/….md
… verdict=under_threshold per_flight_bound_loc=243 ceiling=150 ceiling_source=disarmed_fallback per_flight_verdict=over_bound plan=docs/plans/….md
… verdict=under_threshold per_flight_bound_loc=0 ceiling=70 ceiling_source=resolved per_flight_verdict=over_bound per_flight_note=ceiling_below_pipeline_floor plan=docs/plans/….md
… total_loc=absent threshold_loc=1000 verdict=unknown per_flight_bound_loc=243 ceiling=150 ceiling_source=resolved per_flight_verdict=unknown plan=docs/plans/….md
```

**`plan=` reste en DERNIER**, et les quatre champs s'insèrent avant lui. Deux
raisons, et la première suffirait : `plan=` porte un **chemin**, donc la seule
valeur de la ligne qui pourrait un jour contenir un espace — un champ en position
terminale reste parsable, un champ au milieu ne l'est plus. La seconde est le
précédent de mika#2542, qui a inséré `label=` *avant* `cost_bound=` pour que tout
grep existant continue d'apparier.

Le corps, trois gestes :

```sh
# 1. le plafond, depuis l'argv déjà parcouru en tête de fonction
local _pf_ceiling="" _pf_verdict="unknown" _pf_note=""
case "$_mt" in
    none)        : ;;                      # option absente = budget DÉSARMÉ
    ''|*[!0-9]*) echo "dispatch-lib: plan_size_flight_ceiling_invalid …" >&2 ;;
    *)           _pf_ceiling="$_mt" ;;
esac

# 2. les deux verdicts, dans la MÊME branche de parsabilité du total
case "$_ps_total" in
    ''|*[!0-9]*) : ;;
    *)
        if _plan_size_max_loc; [ "$_ps_total" -gt "$_PLAN_SIZE_MAX_LOC" ]; then …
        if _plan_size_per_flight_bound "$_pf_ceiling"; [ "$_ps_total" -gt "$_PLAN_SIZE_PER_FLIGHT_BOUND" ]; then
            _pf_verdict="over_bound"; else _pf_verdict="under_bound"; fi ;;
esac

# 3. la note conditionnelle, puis l'émission — résolveur co-localisé à chaque fois
_plan_size_per_flight_bound "$_pf_ceiling"; _pf_note="${_PLAN_SIZE_PER_FLIGHT_NOTE:+ per_flight_note=${_PLAN_SIZE_PER_FLIGHT_NOTE}}"
_plan_size_per_flight_bound "$_pf_ceiling"; echo "dispatch-lib: plan_size_estimate … plan=${_PLAN_SIZE_PLAN_PATH}" >&2
```

**Le résolveur est rappelé sur chacune des trois lignes qui lisent sa famille de
globales**, et ce n'est pas une redondance : c'est la contrainte V3 de mika#2693,
qui refuse toute lecture de `$_PLAN_SIZE_PER_FLIGHT_*` non co-localisée avec
`_plan_size_per_flight_bound`. L'idiome est exactement celui que les trois lignes
voisines emploient déjà pour `_plan_size_max_loc`. Le résolveur est pur pour un
argument donné, donc les trois appels rendent la même chose.

**Conséquence tenue, et c'est un bénéfice :** V3 reste **armée** sur ce code neuf
sans qu'une de ses lignes change. Une lecture mal placée ici la rend rouge.

### L2 — Le harnais : un bloc neuf, hors de la fenêtre de V6

Le bloc va dans `skills/bundled/_shared/test-dispatch-lib.sh` **après** la
section mika#2641 (vers la ligne 12826), et non à la suite du bloc mika#2693.
Raison mécanique : la contre-vacuité V6 de #2693 extrait son propre bloc par
`sed -n '/^# mika#2693 — la SECONDE borne…/,/^# mika#2641 —/p'`. Un bloc neuf
glissé dans cet intervalle entrerait dans la fenêtre de V6 et en changerait la
population mesurée — V6 passerait encore, mais elle mesurerait autre chose que ce
que son commentaire annonce. Placer ailleurs coûte zéro ligne et garde V6 exacte.

Sondes `W1`–`W7`. Détail au § Contrat de vérification.

Outillage : **une** fonction `_t2701_emit_probe`, qui prend l'argv à passer et
l'environnement à poser. `_t2636_emit_probe` n'est pas touchée — elle fige
`--max-turns 150` dans son argv, et elle est sous AC4.

### L3 — La solution-doc : la ligne, et ce qu'on en fait

Extension du § *Comment recalibrer sur une distribution* de
`une-estimation-de-taille-se-compare-a-un-seuil-dans-sa-propre-unite.md` : la
forme complète de la ligne, le `grep` qui la relève sur le corpus de `.stderr`,
et la correspondance champ → terme du quadruplet de recalibrage. Plus une note
datée disant que `_PLAN_SIZE_FLIGHT_TAIL_TURNS=50` surestime la queue depuis
mika#2695, avec le renvoi au ticket de suivi. Hors `docs/` : **zéro ligne**.

### Ce que ce plan NE touche pas

Aucun fichier Rust, aucun `system_prompt.md` d'architecte, aucune constante du
site de vérité, aucune variable d'environnement. Le seul fichier de code modifié
est `dispatch-lib.sh`, et le seul fichier de test est `test-dispatch-lib.sh`.
C'est ce qui rend l'AC4 tenable par **construction** plutôt que par vérification.

---

## Étapes d'implémentation

Ordre de commit, **et son plancher** — la ligne sous laquelle le vol n'a rien
livré. Si les tours manquent, on s'arrête à une frontière de commit.

1. **`feat(groom): porte 3 — la ligne plan_size_estimate porte la borne par vol`**
   → L1. Le parsing du plafond depuis `$_mt`, les deux verdicts, la note
   conditionnelle, la ligne `*_ceiling_invalid`, l'émission étendue, le
   commentaire qui dit D1/D2/D3/D4 au site.
2. **`test(groom): W1–W3 — les quatre champs, le désarmement, le plancher`**
   → **PLANCHER.** En dessous, rien n'est livré : des champs de journal non
   gardés ne sont pas une sonde, ce sont des caractères.
3. **`test(groom): W4–W5 — porte 3 ne refuse rien, et un total illisible rend unknown`**
   → AC2 y est.
4. **`test(groom): W6 — l'argv décide, jamais les labels`**
   → AC1 y est, par son discriminant comportemental. **Frontière de complétude
   des AC** avec le commit 5.
5. **`test(groom): W7 — anti-vacuité, hermétisme, ordre des champs`**
   → Fire-Disposition armée, allowlist déclarée vide.
6. **Ouvrir le ticket de suivi du recalibrage** (`gh issue create`, titre
   `feat(groom): recalibrer _PLAN_SIZE_FLIGHT_TAIL_TURNS sur des vols terminés
   post-mika#2695`), corps reprenant D5 (a)(b)(c) et la condition de
   déclenchement du § Hors périmètre. **Son numéro part en `Tracked in:` du corps
   de PR ; #2701 n'y figure jamais comme tracker.** Si `gh issue create` est
   refusé, le corps de PR porte un bloc `Suivi à ouvrir :` nommant le titre et le
   contenu, et le dit explicitement comme un manque — jamais un renvoi vers
   l'issue qu'on ferme.
7. **`docs(solutions): la ligne de porte 3 porte le quadruplet de recalibrage`**
   → L3. Hors `docs/` : zéro ligne.

---

## Contrat de vérification

Chaque sonde nomme ce qu'elle a vu ROUGE avant le correctif — une sonde qui n'a
jamais été rouge ne mesure rien.

### W1 — les quatre champs, aux deux paliers de plafond

| argv | `per_flight_bound_loc=` | `ceiling=` | `ceiling_source=` | `per_flight_verdict=` |
|---|---|---|---|---|
| `--max-turns 200`, total 400 | `431` | `200` | `resolved` | `under_bound` |
| `--max-turns 200`, total 500 | `431` | `200` | `resolved` | `over_bound` |
| `--max-turns 150`, total 400 | `243` | `150` | `resolved` | `over_bound` |
| `--max-turns 150`, total 200 | `243` | `150` | `resolved` | `under_bound` |

**Le même total (400) rend deux verdicts opposés selon le plafond.** C'est la
moitié qui prouve que la mesure est bien *par vol* et non une seconde lecture du
seuil par PR.

Les valeurs attendues `431` et `243` sont **calculées par le résolveur dans le
test**, jamais retapées en littéral : un test qui réécrirait 431 serait un
troisième site de vérité, et il survivrait au recalibrage du ticket de suivi en
mentant. (V4 de #2693 a posé cette règle ; on la reprend.)

Rouge avant : la ligne ne porte aucun des quatre champs.

### W2 — AC3, moitié désarmée

`--max-turns` **absent** de l'argv (ce que produit `PILOT_MAX_TURNS=""`) :

- `ceiling_source=disarmed_fallback` ;
- `ceiling=150`, et **pas** `none` : D2 ;
- `per_flight_bound_loc=243`, et **jamais** 431 — le repli n'emprunte pas le
  plafond amont de claude-pilot ;
- la ligne sœur `pilot_budget_armed max_turns=none` est présente sur la même
  sortie : les deux faits se lisent ensemble, sans jointure.

Rouge avant : aucun champ de provenance n'existait.

### W3 — AC3, moitié plancher, et la note est CONDITIONNELLE

- `--max-turns 70` ⇒ `per_flight_bound_loc=0` et
  `per_flight_note=ceiling_below_pipeline_floor` ;
- `--max-turns 85` (= le plancher exact) ⇒ même chose : **jamais un négatif** ;
- `--max-turns 200` ⇒ **aucune** occurrence de `per_flight_note=` sur la ligne.

Cette troisième assertion est la moitié qui empêche la note de devenir du bruit
permanent, et c'est aussi la contre-vacuité des deux premières : sans elle, une
note émise inconditionnellement les ferait passer toutes les deux.

Rouge **par mutation**, consigné : émettre la note inconditionnellement fait
tirer la 3e ; écrire `${_PLAN_SIZE_PER_FLIGHT_NOTE}` sans le `:+` produit
`per_flight_note=` vide et la fait tirer aussi.

### W4 — AC2, porte 3 NE REFUSE RIEN

Sur un plan à 900 lignes dispatché avec `--max-turns 150` (borne 243, donc
`over_bound` de très loin) :

- `_emit_pilot_budget_line` rend **0** ;
- la ligne `plan_size_estimate` **est émise**, complète ;
- aucun vocabulaire de refus sur la sortie : ni `REFUSED`, ni `abort`, ni
  `PIPELINE FAILURE` ;
- et le contrôle qui compte : la ligne `pilot_budget_armed` — émise **avant** —
  est toujours là, donc rien n'a été court-circuité en amont.

C'est l'invariant que la file groomée entière réclame : un plan déjà groomé ne
repasse par aucune porte de groom au dispatch suivant, donc refuser ici gèlerait
d'un coup tout `docs/plans/`.

Rouge **par mutation** : insérer un `return 1` après l'émission fait tirer la 1re
assertion.

### W5 — un total illisible rend `unknown`, dans les DEUX unités

`total_loc=absent` et `total_loc=unparsable` rendent `verdict=unknown` **et**
`per_flight_verdict=unknown`. Les deux verdicts partagent la même branche de
parsabilité, et ce test est ce qui interdit qu'ils divergent — un
`per_flight_verdict=under_bound` sur un total absent serait un `0` déguisé en
mesure (*un `null` n'est jamais un `0`*, mika#2331).

Rouge avant : le champ n'existait pas ; rouge **par mutation** : sortir le calcul
par vol du `case` lui fait comparer `absent` à un entier.

### W6 — AC1, L'ARGV DÉCIDE, et le discriminant est COMPORTEMENTAL

Sonde : `LABELS="p1-important,ready,loop-substrate"` **et**
`PILOT_MAX_TURNS=200` posés dans l'environnement, mais `--max-turns 150` dans
l'argv. La ligne doit porter `ceiling=150` / `per_flight_bound_loc=243`.

Si l'émetteur relisait les labels, elle porterait `200` / `431`. **C'est la seule
forme de preuve qui tient** : un scan de source dirait « le mot `LABELS`
n'apparaît pas », ce qui est vrai de n'importe quel code qui lirait le plafond
par une autre porte dérobée.

Complété par trois assertions de source, qui sont le détecteur :

- `_emit_pilot_budget_line` ne nomme pas `PILOT_LABEL_TURN_CEILINGS`
  (mika#2542 AC5, un seul lecteur) ;
- elle ne nomme pas `LABELS` et n'appelle pas `_pilot_max_turns` ;
- elle ne lit pas `$_PILOT_MAX_TURNS` **au sens strict** — l'ancre porte `\b`,
  donc les trois champs de diagnostic qu'elle lit légitimement
  (`_SOURCE`, `_INVALID`, `_LABEL`) ne sont pas appariés. C'est l'ancre que V5 de
  #2693 emploie déjà, reprise telle quelle.

Rouge **par mutation**, consigné : remplacer `"$_pf_ceiling"` par
`"$_PILOT_MAX_TURNS"` fait tirer le discriminant comportemental **et** la 3e
assertion de source — les deux moitiés, ce qui est la preuve qu'elles mesurent
bien la même chose par deux chemins.

### W7 — anti-vacuité, hermétisme, et l'ordre des champs

- **`plan=` est le DERNIER champ** de la ligne : assertion sur le suffixe, pas
  sur la présence. C'est la propriété de parsabilité de L1, et elle se perd en
  ajoutant un champ sans y penser.
- **Les quatre champs sont présents simultanément** sur une même ligne (une
  assertion sur la ligne entière, pas quatre `contains` indépendants qui
  passeraient sur quatre lignes différentes).
- **Un seul site d'émission** : `grep -c 'plan_size_estimate total_loc='` sur le
  fichier non commenté vaut **1**. Redondant avec S10 de mika#2636, et assumé
  comme tel : S10 est sous AC4 et ne doit pas bouger, celle-ci dit *pourquoi* le
  champ neuf n'a pas le droit de s'émettre ailleurs.
- **Contre-vacuité** : la sortie de chaque sonde est lue non vide avant qu'on
  conclue quoi que ce soit d'un `assert_not_contains`.
- **Hermétisme** : aucune invocation de `gh` dans la section ; tout est passé en
  littéral à l'émetteur.
- **Fire-Disposition** : `T2701_SOURCE_SCAN_ALLOWLIST=()` déclarée et **assertée
  vide à l'exécution**.

### Les gardes existantes, inchangées et vertes

- `make test-dispatch-lib` — **S1–S18 et S13-bis de mika#2636, et V1–V7 de
  mika#2693, passent sans une ligne modifiée** (AC4). Les sondes de #2636 qui
  lisent la ligne (`S2`, `S3`, `S9`, `S12`) emploient toutes `assert_contains` :
  allonger la ligne ne les touche pas. `S10` compte un seul site d'émission : L1
  étend l'`echo` existant au lieu d'en ajouter un.
- `make verify-bundled-skills` — structure des bundles.
- `make check-pilot-turn-ceiling-labels` — la table de labels n'est pas touchée.
- `bash -n` sur les deux fichiers shell ; `shellcheck` au niveau du dépôt.
- `scripts/verify-pipeline.sh origin/main`.
- `cargo test -p mika-common mika2398` — **vert par construction**, ce plan ne
  touche aucune ligne de Rust. Exécuté quand même : « inchangé » est une
  affirmation à attester, pas à supposer.

---

## Risques nommés

| # | risque | conduite |
|---|---|---|
| R1 | **La ligne s'allonge et casse un parseur en aval.** | Recensé : **aucun consommateur machine n'existe** (`grep -rn 'plan_size_estimate' --include='*.rs'` rend zéro hors du shell et de ses tests). Les lecteurs sont des `grep` humains et les sondes du harnais, toutes en `assert_contains`. Et `plan=` reste terminal (L1). |
| R2 | **Trois appels au résolveur sur trois lignes se lisent comme une redondance** et un relecteur « simplifie » en hissant la valeur. | C'est la contrainte V3 de mika#2693, et elle est armée : hisser la lecture rend V3 rouge. Le commentaire du site la nomme explicitement, avec le motif mika#2039 (`$(…)` dans une trace `set -x`). |
| R3 | **`ceiling=150 ceiling_source=disarmed_fallback` se lit comme un plafond réel de 150** par qui ne lit que ce champ. | D2, et la ligne sœur `pilot_budget_armed max_turns=none` est immédiatement au-dessus. La solution-doc (L3) écrit la paire comme un couple à lire ensemble. |
| R4 | **Un argv portant littéralement `--max-turns none`** serait lu comme un désarmement, pas comme une coquille. | Assumé et dit en commentaire. `_pilot_max_turns` n'émet jamais cette forme ; la distinguer coûterait un sentinel dans l'argv, pour une population vide. |
| R5 | **Le recalibrage de la queue est renvoyé et se perd** — exactement ce qui vient d'arriver à #2693. | Étape 6 : le ticket de suivi est ouvert **avant** la PR, et son numéro est le `Tracked in:`. Le § Hors périmètre nomme sa condition de déclenchement en nombre de vols, pas en date. |
| R6 | **L'instrument mesure, et personne ne lit.** | Le `grep` de relevé et la correspondance champ → terme du quadruplet vont dans la solution-doc (L3), au § qui porte déjà la méthode de recalibrage — donc à l'endroit que le prochain recalibreur ouvre de toute façon. |

---

## Hors périmètre

- **Le recalibrage de `_PLAN_SIZE_FLIGHT_TAIL_TURNS`** (et, par dérivation, des
  bornes 431 / 243 des deux prompts architecte). Motifs en D5 : deux ACs de ce
  ticket l'interdisent (AC3 ancre le plancher à 85 = 35 + 50 ; AC4 verrouille
  V1/V2/V5 qui portent les bornes en littéral), et le corpus de vols **terminés**
  post-mika#2695 est vide. **Condition de déclenchement du suivi :** au moins
  **cinq** vols post-#2695 **ayant ouvert leur PR**, dont le quadruplet est
  relevé par la méthode du § *Comment recalibrer* de la solution-doc. Le geste y
  est atomique sur cinq sites : les constantes shell, les deux `system_prompt.md`
  architecte et leurs deux bornes de référence, **et les littéraux de V1/V2/V5**
  — c'est le cinquième site, celui que le corps de #2701 omet.
  → **Tracked in:** ticket de suivi ouvert à l'étape 6 (son numéro, jamais
  #2701 : un report dont le tracker est l'issue qu'on ferme n'est pas un report).
- **Fermer R1 (le plafond déclaré par un plan n'est pas vérifié contre celui de
  son ticket).** Ce vol rend la comparaison **possible** — le plafond réel est
  journalisé à côté du chemin du plan — mais ne la **mécanise pas** : parser
  `Plafond de tours :` dans le plan ajouterait un second lecteur du format
  prescrit, avec son strip de fences et son ancre, et porte 3 ne refuse jamais,
  donc la détection n'aurait aucune conséquence ici. Le refus appartient à la
  porte 2, et il demande de passer le plafond à `_arch_ask` — un changement de
  contrat de canal. À décider séparément, comme le ticket le dit.
- **L'avis de tours restants envoyé au pilote en cours de vol** — ticket
  compagnon sur `claude-pilot`.
- **Les champs par vol sur `plan_size_threshold_resolved`** (la ligne émise même
  sans plan servi). La borne par vol est bien une propriété du dispatch, pas du
  plan — mais l'AC1 nomme `plan_size_estimate`, et surtout un second site
  d'émission de la même valeur est la divergence que S10 existe pour interdire.
  Le couple (borne, total) n'a de sens que là où un plan est servi.
- **Un palier d'environnement pour la borne par vol.** D3 de mika#2693,
  inchangé : les quatre paramètres se recalibrent à l'échelle du mois, par
  édition du site unique.
- **La borne par PR de mika#2636** : inchangée, défaut `1000`, palier
  `PLAN_SIZE_MAX_LOC` intact, tests intacts.

---

## Definition of Done

- [ ] La ligne `plan_size_estimate` porte `per_flight_bound_loc=`, `ceiling=`,
      `ceiling_source=` et `per_flight_verdict=`, dérivés de `$_mt`, avec `plan=`
      toujours en dernier.
- [ ] `per_flight_note=` n'est émise que non vide ; un argv illisible obtient sa
      ligne `plan_size_flight_ceiling_invalid`.
- [ ] `W1`–`W7` passent, et chacune a été vue rouge (par absence ou par mutation
      consignée dans le commentaire de la sonde).
- [ ] `make test-dispatch-lib` vert, **S1–S18 et S13-bis de mika#2636 et V1–V7 de
      mika#2693 sans une ligne modifiée**.
- [ ] `make verify-bundled-skills`, `make check-pilot-turn-ceiling-labels`,
      `bash -n`, `shellcheck`, `scripts/verify-pipeline.sh origin/main` propres.
- [ ] `cargo test -p mika-common mika2398` vert.
- [ ] La solution-doc porte la forme complète de la ligne, le `grep` de relevé,
      la correspondance champ → terme du quadruplet, et la note datée sur
      `_PLAN_SIZE_FLIGHT_TAIL_TURNS`.
- [ ] Le ticket de suivi du recalibrage est **ouvert**, et son numéro est le
      `Tracked in:` du corps de PR.
- [ ] Le corps de PR porte les cinq décisions de grooming (D1–D5), et D5 avec ses
      trois motifs.

---

## Acceptance criteria

Transcrits verbatim de senara-solutions/mika#2701 :

- La ligne `plan_size_estimate` du dispatch d'implémentation porte les quatre
  champs par vol, dérivés du plafond passé en argv et non d'une relecture des
  labels (mika#2542 AC5 : `PILOT_LABEL_TURN_CEILINGS` garde **un seul** lecteur).
- La porte 3 ne refuse rien : un plan groomé au-dessus de la borne est **mesuré**,
  jamais bloqué au dispatch. Un test le montre.
- Une sonde montre `ceiling_source=disarmed_fallback` sur budget désarmé
  (`PILOT_MAX_TURNS=""`) et la note `ceiling_below_pipeline_floor` sur un plafond
  ≤ 85.
- La borne par PR de mika#2636 et les sondes V1–V7 de mika#2693 passent **sans
  modification**.

**Correspondances :** AC1 → L1 (D1) + **W6**, dont le discriminant est
comportemental ; AC2 → **W4** ; AC3 → **W2** (désarmement) + **W3** (plancher et
note conditionnelle) ; AC4 → § *Gardes existantes*, tenu par construction
puisqu'aucun fichier de #2693 ni de #2636 n'est modifié et que L1 étend l'`echo`
unique au lieu d'en ajouter un.

**Le § « À recalibrer dans le même geste » du corps n'est pas un AC, et il ne
peut pas en devenir un dans cette PR** : AC3 et AC4 le contredisent directement
(D5(a)). Il part en suivi, avec sa condition de déclenchement et son tracker
ouvert.

---

## Fire-Disposition

Le plan livre un détecteur : **W6**, moitié de source — un scan de
`_emit_pilot_budget_line` qui refuse que l'émetteur nomme
`PILOT_LABEL_TURN_CEILINGS`, nomme `LABELS`, appelle `_pilot_max_turns`, ou lise
`$_PILOT_MAX_TURNS` au sens strict. Son chemin de succès est « aucune violation
trouvée ».

**Option retenue : (a) exception nommée en allowlist — allowlist déclarée mais
VIDE, détecteur livré ARMÉ.**

Détail d'implémentation :

- **Population mesurée avant correctif : zéro.** `_emit_pilot_budget_line` ne lit
  aujourd'hui aucune des quatre formes interdites — elle lit les trois champs de
  diagnostic `_PILOT_MAX_TURNS_{SOURCE,INVALID,LABEL}`, que l'ancre `\b` exclut
  par construction. Il n'y a donc aucune violation existante à qui donner une
  entrée grep-visible, et aucune exception n'est créée.
- **L'allowlist est livrée VIDE plutôt qu'absente**, pour que l'ajout futur d'une
  exception soit un geste visible dans une table existante et non l'invention
  d'un mécanisme. Même forme que `T2693_SOURCE_SCAN_ALLOWLIST=()` et que les
  trois scans de source de mika#2636 : `T2701_SOURCE_SCAN_ALLOWLIST=()`.
- **Assertion auto-nettoyante :** la vacuité est assertée **à l'exécution**
  (`assert_eq … "0" "${#T2701_SOURCE_SCAN_ALLOWLIST[@]}"`), donc le test rougit
  **le jour où** une entrée est ajoutée, pas seulement quand elle devient stale.
  Une table vide assertée vide est aussi ce qui distingue « aucune violation » de
  « le scan ne regarde rien ». Quand le scan tire, on **arme le site** ; on
  n'ajoute pas de ligne ici (doctrine mika#2201).
- **Le détecteur est vu rouge avant d'être vu vert**, et c'est sa condition
  d'armement : la mutation est consignée dans son commentaire — remplacer
  `"$_pf_ceiling"` par `"$_PILOT_MAX_TURNS"` au site d'appel du résolveur fait
  tirer la 3e assertion de source **et** le discriminant comportemental de W6.
  Les deux moitiés rougissent ensemble, ce qui est la preuve qu'elles mesurent la
  même propriété par deux chemins.
- **Garde-fou anti-vacuité (W7)** : la source de `_emit_pilot_budget_line` est
  lue **non vide** avant que le scan ne conclue — un `sed` dont la fenêtre se
  serait déplacée rendrait zéro violation sur zéro ligne, et se lirait comme un
  arbre propre (classe mika#2205). C'est la forme que le bras `else` de V4 et la
  contre-vacuité `S10` ont déjà dû prendre.

Les sondes W1–W5 et la moitié comportementale de W6 ne sont pas des détecteurs au
sens de mika#2306 : leur sujet est le comportement d'un chemin d'exécution, pas
la recherche d'une violation dans le dépôt. W7 est une garde de garde.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `dispatch-lib.sh` — extension de `_emit_pilot_budget_line` (parsing du plafond, deux verdicts, note conditionnelle, ligne `*_ceiling_invalid`, émission) | 30 |
| `dispatch-lib.sh` — le commentaire du site (D1 argv, D2 plafond résolu, D3 vocabulaire, D4 note et invalidité, rappel V3) | 25 |
| `test-dispatch-lib.sh` — en-tête de section + `_t2701_emit_probe` | 45 |
| `test-dispatch-lib.sh` — W1 (quatre champs, deux paliers, même total deux verdicts) | 35 |
| `test-dispatch-lib.sh` — W2 (désarmement, provenance, ligne sœur) | 25 |
| `test-dispatch-lib.sh` — W3 (plancher 70 et 85, note conditionnelle, absence au nominal) | 35 |
| `test-dispatch-lib.sh` — W4 (porte 3 ne refuse rien, rc, absence de vocabulaire de refus) | 30 |
| `test-dispatch-lib.sh` — W5 (`absent` / `unparsable` ⇒ `unknown` dans les deux unités) | 20 |
| `test-dispatch-lib.sh` — W6 (discriminant comportemental + trois assertions de source) | 45 |
| `test-dispatch-lib.sh` — W7 (ordre des champs, site unique, contre-vacuité, hermétisme, allowlist) | 35 |

Total estimé : 325 lignes

Plafond de tours : 200 (loop-substrate)
Borne par vol : 431 lignes estimées — (200 − 35 − 50) × 7,5 / 2

**Les deux bornes, et ce que ce plan en fait.**

- **Borne par PR (mika#2636) : 1 000.** 325 est très en dessous ; aucun découpage
  n'est requis à ce titre.
- **Borne par vol : 431.** Ce plan est **un seul vol à 325**, soit 75 % de la
  borne — 106 lignes de marge. La marge est réelle et non obtenue par rabotage :
  elle vient de D5, qui sort le recalibrage du périmètre pour deux raisons
  d'acceptation et une raison de donnée, pas pour atterrir sous un nombre.
  L'estimation des tests emploie la densité **mesurée** de ce harnais — le bloc
  mika#2693 fait ~490 lignes pour V1–V7, soit ~7 lignes par assertion commentaire
  compris — et non les ~13 l/assertion que la solution-doc nomme comme signal
  d'alarme.
- **La fin de pipeline compte en entier malgré mika#2695.** Les 50 tours de
  `_PLAN_SIZE_FLIGHT_TAIL_TURNS` surestiment la queue depuis le retrait de
  `ce-simplify-code` et `ce-code-review` — c'est le constat du ticket. Le sens de
  l'erreur est **prudent** (borne plus serrée ⇒ plan plus petit), donc ce plan
  l'accepte telle quelle plutôt que d'anticiper un recalibrage qu'il renvoie
  explicitement.

**Un seul vol, et voici pourquoi.** Le mécanisme livré est indivisible dans le
sens qui compte : les quatre champs sont **une seule ligne de journal**, et la
moitié d'une ligne de journal n'est pas une mesure. Un découpage couperait soit
les champs entre eux (une ligne qui porterait `ceiling=` sans
`per_flight_bound_loc=` ne recalibrerait rien), soit le code de ses sondes (des
caractères non gardés dans un `.stderr`). Le seul découpage qui coupe à une
frontière de mécanisme est celui que D5 a déjà pratiqué : l'instrument ici, le
recalibrage qu'il rend possible dans le suivi.

**Plancher du vol**, repris du § Étapes : commits 1–2 (l'émission et les sondes
W1–W3). En dessous, rien n'est livré. La frontière de complétude des AC est le
commit 5.

**Calibration citée**, telle que la consigne de groom la demande : mika#2693
lui-même, estimé 392, dernier commit de travail bien avant le plafond, PR #2696
ouverte et mergée — le seul vol du corpus récent dont le plan portait une
estimation et qui n'est pas mort de volume. Ce plan est 17 % plus petit que lui,
sur un périmètre qui touche **un** fichier de code au lieu de quatre.
