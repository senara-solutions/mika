# mika#2631 — un rescue qui a commité à la place du pilote ne peut plus se déclarer `not-applicable`

**Ticket :** senara-solutions/mika#2631
**Type :** fix — substrat de boucle (`dispatch-lib.sh`)
**Branche :** `fix/2631/dispatch-lib-compound-traversal-not`
**Tier :** 1 — l'armement `MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1` est le filet qui
empêche le merge autonome d'un travail non revu, et ce trou le contourne à chaque mort
de type cpp#267.

---

## Le défaut, reproduit par lecture

Le 2026-10-01, pilote `bb9163e1`, implement de mika#2627. Le pilote lance ses relecteurs
(`ce-simplify-code`), écrit « waiting on them » et rend la main. La session se clôt en
`[done] Success | 140 turns` (mécanisme cpp#267). Rien n'est fini : ni revue collectée,
ni compound, ni commit, ni PR. Le rescue commite `9d2e8a51`, ouvre PR #2630 — et le corps
porte `<!-- compound-traversal: not-applicable -->`, donc l'armement ne parque pas.

La chaîne, maillon par maillon, et **aucun maillon n'est fautif isolément** :

| # | site | ce qui se passe |
|---|---|---|
| 1 | `_run_claude_pilot` | la session rend `STATUS=success`, `PRE_RUN_HEAD == POST_RUN_HEAD` |
| 2 | `_rescue_dirty_worktree` | arbre sale ⇒ `git add -A` + `wip(…)` + **`RESCUED_DIRTY_WORKTREE=1`** (dev-pilot) |
| 3 | idem | `POST_RUN_HEAD` est **réécrit** sur le commit de rescue, l'arbre devient **propre** |
| 4 | `_compose_rescue_note` | `RESULT` reçoit `PIPELINE FAILURE: … HEAD unchanged — dirty worktree detected and auto-committed` |
| 5 | `dispatch_claude_pilot` | `RECOVERY_CLASS="dirty-worktree"` (le stamp est testé **en premier**) |
| 6 | `_rescue_compound_traversal` | lit `STATUS` **et rien d'autre** ⇒ `success` ⇒ **`not-applicable`** |
| 7 | `_measure_pipeline_verified` term 0 | `not-applicable` ≠ `absent` ⇒ ne refuse pas, même armé |
| 8 | `_compose_rescue_pr_body` | `compound-traversal: not-applicable` + `rescue-pipeline-verified: yes` |
| 9 | `wip_rescue` (mika#2286) | `yes` littéral ⇒ un brouillon DECISION-CORE est **sorti du brouillon** |

**Le maillon décisif est le 6**, et le ticket le nomme exactement : *« Ce sont deux
lectures du même fait qui divergent dans le même fichier. »* Le maillon 4 vient d'écrire
que le pipeline est tronqué ; le maillon 6, trois mille lignes plus bas, répond que
*« the session concluded. Nothing was truncated »*.

La phrase qui condamne ce court-circuit est déjà écrite dans ce fichier, par mika#2492,
sur l'en-tête de `_pilot_had_no_shipping_tail` :

> *Truncated work must not be presented as complete just because its perimeter had no
> shipping tail.*

mika#2563 l'a citée pour justifier d'ajouter `STATUS` au terme 0. Elle reste vraie quand
le pilote a conclu : **conclure n'est pas livrer.**

---

## Trois rectifications que la lecture impose au ticket

### R1 — « HEAD inchangé + worktree sale » est un fait PASSÉ, structurellement inmesurable au moment du classement

C'est la rectification centrale, et elle décide toute la conception. Quand
`_rescue_compound_traversal` tourne (dans `dispatch_claude_pilot`, après le commit de
rescue **et** après `_push_branch`), les deux moitiés du prédicat que l'AC1 nomme ont
**disparu de l'état mesurable** :

- `PRE_RUN_HEAD != POST_RUN_HEAD` est désormais **vrai** — le maillon 3 a réécrit
  `POST_RUN_HEAD` sur le commit de rescue ;
- l'arbre est **propre** — le maillon 2 a tout stagé puis commité.

Un prédicat qui irait relire `git status` ou comparer les deux SHA lirait donc exactement
l'inverse du fait qu'on veut détecter. **La seule trace survivante est le stamp
`RESCUED_DIRTY_WORKTREE=1`**, posé par son producteur aux deux sites d'auto-commit.

C'est le motif que ce dépôt a déjà écrit quatre fois : `PILOT_SHIPPING_TAIL` (mika#2492,
« the stamp is written by its PRODUCER … never reconstructed here »), `origin:loop`
(mika#2026), `closing_pr_closed_unmerged` (mika#2242), `qa_review_pr_target` (mika#2368).
Le ticket formule l'AC1 en termes de symptôme ; l'implémentation doit la formuler en
termes de **stamp**, sans quoi elle vise une donnée qui n'existe plus.

### R2 — le stamp, jamais `RECOVERY_CLASS`

`RECOVERY_CLASS` est le nom lisible de la classe, et il serait tentant de le lire. Il est
**refusé**, pour deux raisons :

1. Il est déclaré `local RECOVERY_CLASS=""` dans `dispatch_claude_pilot`. Le lire depuis
   `_rescue_compound_traversal` marcherait — bash donne la portée dynamique — et c'est
   précisément la fragilité que `dispatch-lib.sh` a déjà refusée une fois : mika#2496 fait
   passer `"${LABELS:-}"` **en argument** à ses trois sites de lancement plutôt que par
   portée dynamique, « jamais par portée dynamique ».
2. Il est **dérivé** du stamp (`if [ "${RESCUED_DIRTY_WORKTREE:-}" = "1" ]`). Lire la
   dérivation plutôt que le fait ajoute un maillon qui peut diverger, pour zéro gain.

Le stamp est un global simple, **déjà lu** par le calcul de classe quelques lignes plus
haut, et `= "1"` signifie très exactement « le rescue a commité du contenu à la place du
pilote » : le chemin scaffold-only pose `=0`, les deux chemins d'échec de hook le laissent
non posé, et dev-groom ne le pose jamais (`case "$SKILL" in dev-pilot)`).

### R3 — « toute classe qui auto-commite » ne comprend PAS `commit-pushed-no-pr`

L'AC1 écrit « ou toute classe qui auto-commite à la place du pilote ». Lue à la lettre, la
parenthèse balaierait `commit-pushed-no-pr`, qui crée bien un commit — le marqueur vide
`wip(mika#1383)`. **Elle doit en rester dehors**, et la raison est dans les mots « à la
place du pilote » : sur cette classe le pilote **a commité son propre travail**, le
marqueur est vide et n'existe que pour armer la garde 2 de `self-dev-webhook-qa`. La faire
entrer ferait mesurer la traversée compound sur un pilote qui a correctement fini et dont
seul `gh pr create` a échoué — c'est-à-dire mordre une partie du trafic nominal, sous un
autre nom. Le périmètre livré est donc : **`dirty-worktree` seule**, et ce choix est
épinglé par un test (T15o) pour qu'une relecture future de la parenthèse ne le défasse pas
en silence.

### R3-bis — amendement de revue (2026-10-02) : la Phase A de mika#1383 EST dans la population

La revue de code (relecteurs correctness et adversarial, indépendants) a réfuté une
prémisse de R3 : « sur cette classe le pilote a commité son propre travail, le commit
est un marqueur vide ». C'est vrai du marqueur `wip(mika#1383)`, faux de la **Phase A**
du même bloc : un pilote qui commite une partie de son travail, laisse le reste sale et
rend la main voit dispatch-lib commiter ce reste (`trailing content after pilot
end_turn`). C'est un auto-commit « à la place du pilote » au sens exact de l'AC1, et la
classe se lit alors `commit-pushed-no-pr`.

Correctif livré : un second stamp de producteur, `RESCUED_TRAILING_CONTENT`, remis à 0
par dispatch dans `_run_claude_pilot` à côté de son frère et posé sur le commit de Phase
A ; `_rescue_committed_in_the_pilots_place` lit les deux. Le **marqueur vide seul**
reste exempté (T15q inchangé) : la décision de R3 tient pour lui, pas pour la Phase A.
Épinglé par T15r, qui pilote le vrai `_post_flight_recovery`, et vu rouge des deux côtés
(stamp non posé ; prédicat sans le second terme).

---

## Conception

### D1 — un prédicat nommé, conjoint au court-circuit

`_rescue_committed_in_the_pilots_place()` : vrai quand `RESCUED_DIRTY_WORKTREE` vaut le
littéral `1`. Le court-circuit du terme devient une **conjonction** :

```
la session a conclu ET le rescue n'a pas commité à sa place  ⇒  not-applicable
```

Un helper d'une ligne pour une condition d'une ligne est un coût réel ; il est payé parce
qu'il achète trois choses qu'un test inline ne donne pas : la doctrine de R1 vit **sur la
fonction** plutôt que dans un commentaire au milieu d'un classeur ; le recensement
structurel de l'AC4 a un **symbole stable** à viser plutôt qu'une orthographe de variable ;
et si la population grandit un jour (un second stamp de producteur), il y a **un** site à
étendre. C'est la même séparation que mika#2563 s'est imposée entre
`_rescue_compound_traversal` (classer) et `_rescue_require_compound_traversal` (disposer).

### D2 — la valeur n'est pas forcée : elle est MESURÉE

Le correctif **retire une sortie anticipée**, il n'en ajoute pas. Une fois le
court-circuit refusé, le flot tombe dans les deux mesures que mika#2563 a déjà écrites, et
les trois issues sont celles de l'AC1 :

- `attested-solution` — un `docs/solutions/**/*.md` est dans `origin/main...HEAD`. **Cas
  réel et non théorique** : un pilote qui a écrit son learning sans le commiter le voit
  stagé par le rescue, donc la décision a bien été prise *et* exécutée.
- `attested-trailer` — un `Compound: none <raison>` ancré dans `origin/main..HEAD`
  (possible sur un re-dispatch ; le message de rescue auto-généré n'en porte pas).
- `absent` — ni l'un ni l'autre : l'incident fondateur.

La mesure est **signifiante et non vide** : à l'instant du classement, le commit de rescue
existe, donc `origin/main...HEAD` est non vide et les deux formes sont lisibles.

### D3 — aucun changement de disposition, et la conséquence est nommée

`_rescue_require_compound_traversal` n'est pas touché et reste **désarmé par défaut**.
Ce qui change est la **classification**, donc un déploiement désarmé continue de mesurer et
d'écrire un marqueur honnête sans rien retenir — la séparation que mika#2563 déclare et que
mika#2249/#2420 ont établie avant elle.

**Le coût sur la flotte armée, dit plutôt que découvert.** L'incident fondateur prouve que
`MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1` est posé en production. Après ce correctif, un
rescue `dirty-worktree` sans learning ni trailer lira `rescue-pipeline-verified: no`, donc
`wip_rescue` **parquera** un brouillon DECISION-CORE derrière un geste humain (mika#2286).
**C'est l'effet voulu** — c'est exactement ce que samidarko a fait à la main le
2026-10-01 — et c'est l'asymétrie que ce corps de PR a déjà arbitrée deux fois (mika#2157,
mika#2354) : *un `no` de trop coûte un geste opérateur visible et réversible ; un `yes` de
trop ouvre la revue sur un travail incomplet.*

### D4 — fail-closed préservé, dans le même sens

Le nouveau terme ne peut qu'**ajouter** des mesures, jamais en retirer : il ne touche
aucune des quatre sorties `absent` de D7 (dir vide, non-repo, `origin/main` absent, `git
log` illisible) et ne transforme aucun `absent` en valeur satisfaite. Le stamp est lu avec
l'exactitude de la maison (`= "1"`), sur le modèle de `[ "${PILOT_SHIPPING_TAIL:-}" =
"absent" ]` : une valeur illisible, vide ou `0` laisse le comportement d'aujourd'hui, ce
qui est le sens **sûr** ici — ne pas mesurer est l'état antérieur, et non une permission
neuve.

### D5 — la propriété « appelé deux fois, ne peut pas diverger » est préservée

L'en-tête actuel justifie le double appel : la fonction est pure en (`STATUS`, état du
worktree), et le site composeur ne peut pas lire un global écrit dans le `$(…)` du terme 0.
Elle devient pure en (`STATUS`, `RESCUED_DIRTY_WORKTREE`, état du worktree). **Rien entre
les deux appels ne mute le stamp** : il est posé dans `_run_claude_pilot` →
`_post_flight_recovery` → `_rescue_dirty_worktree`, bien avant, et le sous-shell du terme 0
ne peut rien écrire. Le raisonnement de l'en-tête s'étend donc verbatim, et il faut
l'étendre explicitement plutôt que de laisser l'en-tête affirmer une pureté sur deux
variables quand il y en a trois.

### D6 — observabilité : le marqueur EST la sonde, pas un grep

Le champ `compound-traversal=` de la ligne `rescue_pipeline_verified:` gagne le stamp, pour
que l'opérateur distingue « exempt parce que la session a conclu **et** commité » de
« mesuré parce que le rescue a commité à sa place ». **Mais cette ligne n'est pas une
sonde, et le plan le dit** : elle est émise au niveau de `dispatch_claude_pilot`, donc son
stderr est le `Stdio::piped()` de `spawn_long_running_exec`, que l'exécuteur ne lit que
dans `if !status.success()` — sur un dispatch qui réussit le tuyau est lâché sans être lu.
C'est le Signal M, classe mika#2050, corrigée trois fois sur le Signal S. Le même fichier a
déjà écrit ce refus mot pour mot (mika#2503 : *« the `echo` below is a convenience,
deliberately NOT presented as a probe »*), et ce correctif le reprend.

**La surface durable est le corps de PR**, et il est déjà complet sans un champ de plus :
il porte `<!-- compound-traversal: absent -->` **et** la phrase de classe
(« The pilot session wrote file changes but never committed. dispatch-lib auto-committed
with `wip()` prefix. »). Les deux moitiés de l'attribution y sont, lisibles par `gh pr
view`. Aucun événement, aucun compteur, aucune ligne `audit_events` n'est ajouté — un
dispatch shell n'a pas d'accès base, et inventer une surface que personne ne lit
reproduirait le défaut du Signal M.

---

## AC4 — le recensement des lecteurs de `STATUS = success`

`grep -n 'STATUS" = "success"\|STATUS:-}" = "success"' skills/bundled/_shared/dispatch-lib.sh`
rend **exactement trois** sites. La question posée à chacun est : *porte-t-il la prémisse
« conclu = complet », et cette prémisse a-t-elle une conséquence ?*

| # | site | lecture | verdict |
|---|---|---|---|
| 1 | `_rescue_compound_traversal` | `success` ⇒ rien n'a été tronqué | **LE DÉFAUT — corrigé** |
| 2 | `_pilot_had_no_shipping_tail` | `success` ⇒ la session a conclu | **nommé, délibérément NON corrigé** |
| 3 | garde Unit 1 (mika#940) | `success` ⇒ ne pas re-classer un échec | **nommé, aucun défaut** |

### Pourquoi #2 ne doit PAS être corrigé — un refus mesuré

`_pilot_had_no_shipping_tail` porte bien la prémisse : sur l'incident fondateur,
`SKILL=dev-pilot` + `PILOT_SHIPPING_TAIL=absent` + `STATUS=success` ⇒ **vrai**, alors que
le travail était tronqué. Le réflexe serait d'y conjoindre le même stamp. **Ce serait une
régression, et voici laquelle.**

Le changement serait **sans effet** sur le calcul de classe (le stamp y est testé en
premier, donc la classe est déjà `dirty-worktree`). Il serait en revanche **actif** sur la
garde Unit 1, où le terme est écrit `! _pilot_had_no_shipping_tail` : il basculerait à vrai
et la garde écrirait

> *PIPELINE FAILURE: … Pipeline truncated before git push + gh pr create.*

— une phrase **fausse** pour un rescue `dirty-worktree` (dispatch-lib est précisément en
train d'ouvrir la PR), et une **seconde** ligne `PIPELINE FAILURE:` sur un `RESULT` qui en
porte déjà une. On échangerait un marqueur faux contre une phrase fausse.

Ce qui protège #2 aujourd'hui est donc l'**ordre des instructions**, à deux endroits :

1. dans le calcul de `RECOVERY_CLASS`, la branche `RESCUED_DIRTY_WORKTREE` **précède** la
   branche `_pilot_had_no_shipping_tail` ;
2. dans la classification Unit 3, le bras `grep -qF "PIPELINE FAILURE:"` **précède** le
   bras `elif _pilot_had_no_shipping_tail`.

Un ordre est une propriété **émergente** : l'inverser ne rendrait aucune décision fausse
dans aucun test existant et rouvrirait ce ticket en silence. D'où un pin **structurel**
(T15n) plutôt qu'un pin comportemental.

### Pourquoi #3 n'est pas un défaut

Sa revendication est conditionnée à `PRE_RUN_HEAD != POST_RUN_HEAD` **et** à
`! _pilot_had_no_shipping_tail`, et sur la classe `dirty-worktree` le rescue a déjà écrit
sa propre ligne `PIPELINE FAILURE: … HEAD unchanged — dirty worktree detected and
auto-committed`. La troncature est donc **déjà nommée** ; cette garde se taisant, elle ne
produit ni faux vert ni fausse phrase.

### Et `rescue-pipeline-verified`, que l'AC4 cite nommément

Il **ne lit pas `STATUS`** : ses six termes passent par le terme 0, dont le seul lecteur de
`STATUS` est `_rescue_compound_traversal`. **Corriger le classeur corrige donc
`rescue-pipeline-verified` sans un seul changement supplémentaire** — c'est la réponse
exacte à l'exemple de l'AC4, et c'est ce qui garde ce correctif petit.

---

## Tâches

1. **`_rescue_committed_in_the_pilots_place()`** — nouveau prédicat dans
   `skills/bundled/_shared/dispatch-lib.sh`, posé immédiatement avant
   `_rescue_compound_traversal`. Corps : `[ "${RESCUED_DIRTY_WORKTREE:-}" = "1" ]`.
   En-tête portant R1 (le fait est passé, seul le stamp survit), R2 (pourquoi pas
   `RECOVERY_CLASS`) et R3 (pourquoi `commit-pushed-no-pr` est dehors).

2. **Le court-circuit devient une conjonction** dans `_rescue_compound_traversal` :
   `success` **et** `! _rescue_committed_in_the_pilots_place` ⇒ `not-applicable`.

3. **L'en-tête de `_rescue_compound_traversal` est mis à jour**, trois endroits :
   la description de `not-applicable` dans le tableau des quatre valeurs (elle dit
   aujourd'hui « `STATUS = success` » tout court) ; l'argument « CALLED TWICE PER DISPATCH »
   (pureté sur trois variables, D5) ; et la citation mika#2492, qui gagne sa seconde
   application — *conclure n'est pas livrer*.

4. **Le recensement AC4 est écrit dans le fichier**, en table, au-dessus du nouveau
   prédicat : les trois sites, leur verdict, et le refus mesuré de corriger #2 avec la
   phrase fausse qu'il produirait. C'est l'allowlist de la Fire-Disposition, et elle est
   grep-visible là où elle est exercée.

5. **D6** — le champ `compound-traversal=` de la ligne `rescue_pipeline_verified:` porte le
   stamp, avec le commentaire qui refuse de la présenter comme une sonde (patron mika#2503).

6. **`exec 9>/dev/null`** dans `test_rescue_pipeline_verified.sh` : le suite ne pilote pas
   encore `_rescue_dirty_worktree`, qui écrit sur le fd 9 (patron
   `test_dev_groom_dirty_rescue.sh:35`). Sans ça T15i meurt sur un fd fermé.

7. **T15i → T15o** dans `skills/bundled/_shared/tests/test_rescue_pipeline_verified.sh`
   (détail en Verification contract).

8. **T15e devient explicite** : `RESCUED_DIRTY_WORKTREE=0` posé plutôt que laissé non
   défini. Le contrôle passe aujourd'hui parce que la variable *se trouve* non posée dans
   ce process ; un contrôle implicite est un contrôle qu'une édition future casse en
   silence.

9. **Aucun autre fichier.** Pas de `canonical-tokens.tsv` (l'ensemble des quatre valeurs ne
   bouge pas, et `compound-traversal` n'a aucun lecteur strict à déclarer) ; pas de
   `executor.rs` (le relais d'environnement est intact) ; pas de `wip_rescue.rs` ; pas de
   prompt bundled (la moitié intention de `self-dev-webhook-qa` reste exacte — elle décrit
   l'étape que la session tronquée n'a pas atteinte, ce qui est précisément le cas qu'on
   ouvre).

---

## Verification contract

Suite : `skills/bundled/_shared/tests/test_rescue_pipeline_verified.sh`, lancée par
`make test-rescue-pipeline-verified` et par le job CI existant. Les fixtures sont de **vrais
dépôts git temporaires** avec un `origin/main` planté, selon le contrat que l'en-tête de la
suite écrit déjà : *une mesure reconstruite ne testerait que le plan.*

### T15i (AC3) — le défaut fondateur, bout-en-bout, **vu rouge**

Ne teste pas que le classeur lit une variable : pilote le **vrai**
`_rescue_dirty_worktree` (patron `test_dev_groom_dirty_rescue.sh`). Fixture :
`make_repo`, fichiers écrits **sans commit**, `SKILL=dev-pilot`, `STATUS=success`,
`PRE_RUN_HEAD == POST_RUN_HEAD`, puis appel du rescue. Assertions :

- le stamp vaut `1`, HEAD a avancé, l'arbre est **propre** — les trois faits de R1, donc la
  preuve que le prédicat du ticket serait inmesurable à cet instant ;
- `_rescue_compound_traversal` rend **`absent`** ← **rouge avant le correctif** (le
  court-circuit rend `not-applicable`).

C'est le test qui prouve la chaîne entière : le stamp est réellement posé par son
producteur et atteint réellement le classeur.

### T15i-bis (AC3) — armé, le terme refuse et court-circuite

`MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=1` sur la fixture de T15i :
`_measure_pipeline_verified` rend 1, nomme `compound-traversal`, l'extrait cite le statut
lu, et le log des shims est **vide** — aucun cargo, aucun `verify-pipeline.sh` n'a tourné
(le terme 0 est bien en tête). Le corps composé porte
`<!-- compound-traversal: absent -->` + `<!-- rescue-pipeline-verified: no -->`.

### T15j (AC2) — contrôle POSITIF, et il est la moitié qui empêche le remède d'être pire

`STATUS=success`, stamp **`0`**, HEAD avancé (`add_work`) : le classeur rend
`not-applicable`, et armé la mesure **n'est pas refusée**. Sans ce cas, « le terme mord »
serait indistinguable de « le terme mord tout le monde » — et la seconde lecture arrête le
drain, `verify-pipeline.sh` étant le terme 5 de cette même conjonction.

### T15j-bis (AC2, D3) — la route `no-shipping-tail` reste exempte

`SKILL=dev-pilot PILOT_SHIPPING_TAIL=absent STATUS=success RESCUED_DIRTY_WORKTREE=0` ⇒
`not-applicable`. C'est la route **nominale** de la boucle et les 71 % de merges sans
`docs/solutions` ; mordre là re-mordrait le régime sous un autre nom.

### T15k — le stamp ne masque pas une attestation réelle

Stamp `1` + `STATUS=success` + un `docs/solutions/<cat>/x.md` dans le diff ⇒
`attested-solution`. Idem avec un trailer ancré ⇒ `attested-trailer`. Un rescue qui a
commité le learning du pilote **a** exécuté la décision ; le correctif retire une exemption,
il ne fabrique pas un refus.

### T15l — le domaine du stamp, exactement

Sur `STATUS=success` : stamp `0`, stamp non posé, stamp `yes`, stamp `true`, stamp ` 1 `
rendent tous `not-applicable` ; **seul le littéral `1`** fait mesurer. Même exactitude que
`[ "${PILOT_SHIPPING_TAIL:-}" = "absent" ]`, et la direction du fail-safe de D4.

### T15m (AC4) — le recensement est exhaustif, comparé DANS LES DEUX SENS

Scan de source : chaque site de `dispatch-lib.sh` lisant `STATUS` contre le littéral
`success` figure dans la table du recensement, **et** chaque entrée de la table désigne un
site qui existe encore. La **cardinalité est assertée à 3** — c'est la seule forme d'échec
qu'aucune fixture ne voit : un prédicat devenu trop étroit passerait en ne regardant rien
(leçon U3 de mika#2496, classe mika#2205).

### T15n (AC4) — les deux ordres qui protègent `_pilot_had_no_shipping_tail`

Scan de source : dans le calcul de `RECOVERY_CLASS`, la ligne lisant
`RESCUED_DIRTY_WORKTREE` **précède** celle appelant `_pilot_had_no_shipping_tail` ; dans la
classification Unit 3, le bras `PIPELINE FAILURE:` **précède** le bras
`elif _pilot_had_no_shipping_tail`. Structurel et non comportemental, parce qu'inverser
l'un des deux ne rendrait **aucune** décision fausse dans la suite existante.

### T15o (R3) — `commit-pushed-no-pr` reste hors population

`STATUS=success`, stamp `0`, HEAD avancé par un commit vide `wip(mika#1383)` ⇒
`not-applicable`. Épingle la décision de R3 pour qu'une relecture littérale de la
parenthèse de l'AC1 rougisse au lieu d'élargir.

### Non-régression

`make test-rescue-pipeline-verified` (T1→T15h inchangés — **T15e devient explicite, pas
modifié dans son attendu**), `make test-dispatch-lib`, `make test-rescue-signal`,
`make test-rescue-closes-guard`, `make test-rescue-cause-token`,
`bash skills/bundled/_shared/tests/test_dev_groom_dirty_rescue.sh`, `make verify-bundled-skills`,
`bash -n skills/bundled/_shared/dispatch-lib.sh`, `shellcheck` au niveau du fichier.

---

## Fire-Disposition

Ce plan livre des détecteurs : deux scans structurels (T15m, T15n) et sept cas
comportementaux. Option retenue : **(a) exception nommée en allowlist**, et il faut
distinguer les deux moitiés parce qu'elles ne portent pas le même risque.

**Les cas comportementaux atterrissent ARMÉS, avec zéro exception.** Le correctif est dans
le même commit que le détecteur : T15i est rouge avant, verte après, et aucun autre cas ne
peut avoir de violation préexistante puisqu'il n'y a pas de population antérieure à ce
comportement. Les livrer désarmés (option b) viderait l'AC3 de son contenu — elle demande
explicitement un test *vu rouge*.

**T15m porte l'allowlist, et elle est l'instrument de l'AC4.** Sa population est de **trois**
entrées et **deux sont des exceptions déclarées**, chacune nommant la donnée précise et sa
raison :

| site | verdict | raison déclarée |
|---|---|---|
| `_rescue_compound_traversal` | **corrigé** | — |
| `_pilot_had_no_shipping_tail` | **exception** | corriger ce site ferait écrire à la garde Unit 1 la phrase *« Pipeline truncated before git push + gh pr create »*, fausse sur un rescue `dirty-worktree`, plus une **seconde** ligne `PIPELINE FAILURE:`. La propriété est tenue par l'ordre des instructions, épinglé par T15n. **Aucun ticket de suivi** : ce n'est pas une dette, c'est une décision — le site est correct dans son domaine. |
| garde Unit 1 (mika#940) | **exception** | la troncature est déjà nommée par la ligne `PIPELINE FAILURE: … HEAD unchanged` du rescue ; cette garde se taisant, elle ne produit ni faux vert ni fausse phrase. |

**L'assertion auto-nettoyante est la comparaison DANS LES DEUX SENS** : une entrée dont le
site a disparu (renommage, suppression) fait **rougir** le build, au lieu d'exempter en
silence un homonyme futur. C'est la forme que `FIRED_AT_LITERAL_WRITERS` (mika#2133) et le
registre `DISPATCH_ENV_KNOWN_INERT` (mika#2536) emploient déjà dans ce dépôt. La
cardinalité assertée à 3 est la seconde moitié : sans elle, un scan devenu aveugle se lit
exactement comme un arbre propre.

**Quand T15m tire sur un quatrième site, la résolution est de le TRANCHER**, pas
d'allonger la liste : chaque entrée porte un verdict, et « non classé » n'en est pas un
(doctrine mika#2201 — *on déclare, on n'allowliste pas*).

---

## Definition of Done

- `_rescue_compound_traversal` ne rend plus `not-applicable` sur un dispatch dont le rescue
  a commité à la place du pilote ; la valeur est alors mesurée.
- `not-applicable` est préservé pour une session `success` qui a commité elle-même, route
  `no-shipping-tail` comprise.
- Le recensement AC4 est écrit dans le fichier, en table, avec le verdict de chacun des
  trois sites et le refus mesuré de corriger le second.
- T15i est **vue rouge** sur `main` avant le correctif, puis verte.
- `make test-rescue-pipeline-verified`, `make test-dispatch-lib` et
  `make verify-bundled-skills` passent ; aucun autre test du dépôt ne bouge.
- Aucune valeur de réglage déplacée : `MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL` reste
  **désarmé par défaut**, `MIKA_RESCUE_VERIFY_ENABLED` est intact, et le chemin
  kill-switch reste byte-identique (`$8` vide).

## Acceptance criteria

- **AC1.** Quand le rescue est entré par la classe « HEAD inchangé + worktree sale » (ou
  toute classe qui auto-commite à la place du pilote), `compound-traversal` ne vaut
  **jamais** `not-applicable`. On mesure alors `attested-solution`, `attested-trailer` ou
  `absent`, comme pour une session coupée. — *Tâches 1–2, T15i, T15i-bis, T15k. Périmètre
  de la parenthèse : `dirty-worktree` **et** la Phase A de mika#1383 (R3-bis, T15r) ; le
  marqueur vide `commit-pushed-no-pr` reste dehors, épinglé par T15o. Les tests ont
  atterri renumérotés T15k…T15r.*
- **AC2.** `not-applicable` reste la valeur d'une session `success` qui a **elle-même**
  commité son travail (HEAD avancé, sans auto-commit du rescue) : c'est la route nominale
  `no-shipping-tail`. — *T15j, T15j-bis, T15e rendue explicite.*
- **AC3.** Test : `STATUS=success` + HEAD inchangé + worktree sale donne `absent` (**vu
  rouge** avant le correctif). `STATUS=success` + HEAD avancé donne `not-applicable`
  (contrôle positif). — *T15i (vue rouge), T15j.*
- **AC4.** Recenser les autres lecteurs de `STATUS = success` dans le rescue qui portent la
  même prémisse « conclu = complet » (par exemple `rescue-pipeline-verified`), et les
  corriger ou les nommer. — *Table ci-dessus : trois sites, un corrigé, deux nommés avec la
  raison de ne pas les corriger ; `rescue-pipeline-verified` ne lit pas `STATUS` et est
  corrigé par le classeur. Épinglé par T15m (deux sens, cardinalité) et T15n (les deux
  ordres).*

---

## Hors périmètre, délibérément

- **La porte de merge `--required` (mika#2617).** C'est la **seconde moitié** de l'incident
  fondateur : un check rouge hors de la porte n'a pas bloqué le merge. Ticket distinct, et
  le corps de mika#2631 le cite comme contexte. Les deux moitiés tombent séparément — ce
  correctif referme celle qui parque, pas celle qui merge.
- **cpp#267**, la cause de la main rendue avant le commit. Autre dépôt, hors d'atteinte.
- **Le chemin mika#2151** (commit de rescue dans une PR **déjà ouverte**) : `PR_URL` non
  vide ⇒ `_recovery_pr_due=0` ⇒ aucun corps composé ⇒ **aucun marqueur du tout**. Cette
  population n'en portait pas et n'en portera pas. Limite **nommée**, non couverte :
  l'ouvrir demande de décider qui écrit un marqueur sur une PR que le rescue n'a pas créée.
- **Armer `MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL` par défaut.** mika#2563 l'a livré
  désarmé sur une précondition écrite (la distribution des rescues `STATUS != success`,
  illisible depuis un bac à sable de dispatch). Ce correctif **élargit la population** du
  terme, donc il rend cette précondition *plus* exigeante, pas moins : l'armement reste un
  geste d'opérateur.
- **Corriger `_pilot_had_no_shipping_tail`** — refus mesuré, raison écrite dans la
  Fire-Disposition.
- **Faire entrer `commit-pushed-no-pr`** — refus de R3, épinglé par T15o.
- **Les cinq autres termes de `_measure_pipeline_verified`** et le budget : intacts.

## Ce que ce travail n'achète PAS

- **Il ne fait pas finir le travail du pilote.** Il fait en sorte qu'un travail inachevé ne
  se présente plus comme complet. Le pilote continuera de mourir de la même façon tant que
  cpp#267 est ouvert ; ce qui change est que la PR qui en sort est **parquée** au lieu
  d'être mergeable.
- **Il ne rattrape pas PR #2630.** Son corps porte `not-applicable` et **rien ici ne
  réécrit un marqueur passé** : fabriquer une ligne décrivant une mesure qu'on n'a pas
  prise est l'inverse de ce que ce travail défend. samidarko l'a remise en draft à la main,
  et la sonde est la **prochaine** occurrence.
- **Il n'ajoute aucun compteur, aucun événement, aucune ligne `audit_events`.** Le seul
  instrument est le marqueur dans le corps de PR, et **son silence ne prouve rien tant que
  personne ne le lit** — sur une population d'environ 1,1 rescue/jour, l'absence
  d'occurrence peut simplement vouloir dire qu'aucun pilote n'est mort cette semaine.
- **Il ne rend pas le champ surveillé, seulement lisible.** La ligne `rescue_pipeline_verified:`
  tombe dans un tuyau non lu sur un dispatch qui réussit (D6, Signal M) ; c'est dit, pas
  réparé — le réparer est un changement de substrat (rediriger vers le `.stderr` par
  dispatch), et c'est le ticket de suivi que le Signal M porte déjà.

---

## Sondes post-déploiement, et leurs quatre haltes

> **Préalable, non négociable.** `skills/bundled/_shared/` est une projection du
> **binaire**, pas du checkout (mika#2340). `cat ~/.mika/skills/.manifest-writer` doit
> porter le sha qu'on vient de bâtir — sans cette vérification, chacune des sondes décrit le
> `dispatch-lib.sh` d'hier.

**S1 — le défaut fondateur ne se rejoue pas (premier rescue `dirty-worktree` après
déploiement).** Sur la PR de rescue :

```bash
gh pr view <N> --repo senara-solutions/mika --json body \
  | jq -r '.body' | grep -E 'compound-traversal|rescue-pipeline-verified'
```

Attendu : `compound-traversal:` vaut `absent` (ou une attestation réelle), **jamais**
`not-applicable`, sur un corps portant la phrase de classe « dispatch-lib auto-committed
with `wip()` prefix ».
**Halte 1 — `not-applicable` réapparaît sur un corps portant cette phrase.** Ne pas
retoucher le prédicat par réflexe : établir d'abord que le binaire servi porte le correctif
(préalable ci-dessus), **puis** que le stamp a bien été posé — un rescue passé par le chemin
scaffold-only pose `=0` et `not-applicable` y est **correct**.

**S2 — contrôle POSITIF, et c'est lui qui empêche de lire un faux vert.**

```bash
gh pr list --repo senara-solutions/mika --state all --label wip-rescue --limit 30 \
  --json number,body --jq '.[] | {number, t: (.body | capture("compound-traversal: (?<v>[a-z-]+)").v)}'
```

La population doit montrer **les deux** valeurs : des `not-applicable` (route
`no-shipping-tail`, le régime à 71 %) **et** des `absent`/`attested-*`.
**Halte 2 — uniquement des `absent`.** Le terme mord tout le monde : la route nominale est
balayée, et armé cela arrêterait le drain. **Désarmer d'abord**
(`MIKA_RESCUE_REQUIRE_COMPOUND_TRAVERSAL=0`), diagnostiquer ensuite — c'est le mode de
panne que T15j/T15j-bis existent pour empêcher, donc une occurrence dit que le prédicat
déployé n'est pas celui que les tests décrivent.
**Halte 3 — uniquement des `not-applicable` sur 30 PR.** On ne peut **rien** conclure tant
qu'un rescue `dirty-worktree` n'a pas eu lieu depuis le déploiement : *une garde que
personne n'a exercée se lit exactement comme une garde qui marche* (mika#2205). Vérifier
qu'un corps porte la phrase de classe avant toute conclusion.

**S3 — le parquage se produit (flotte armée, 7 jours).** Un rescue `dirty-worktree` sans
learning doit porter `rescue-pipeline-verified: no` et rester **en brouillon** après le
passage de `wip_rescue`.
**Halte 4 — la PR est sortie du brouillon malgré `no`.** Le défaut est **en aval** :
`wip_rescue` ne lit pas le marqueur, ou le brouillon n'est pas classé DECISION-CORE (le
chemin MECHANICAL est délibérément ouvert au marqueur `no`, mika#2286). **Ne pas durcir le
marqueur** — établir lequel des deux avant de toucher quoi que ce soit, puis lire
`wip_rescue_parked_unverified` côté démon.

**Halte transverse — les deux sondes muettes.** Aucune PR `wip-rescue` depuis le
déploiement signifie que rien n'a été mesuré, ni dans un sens ni dans l'autre. Établir
qu'un rescue a eu lieu avant d'écrire la moindre conclusion.
