# mika#3142 — vol 2b : le secours REFUSE la draft sur une halte déclarée

**Vol 2b de mika#3142**, le dernier. Le vol 2a est livré par #3168 (squash `ee6cf125`,
déployé le 2026-11-25) : le détecteur `_pilot_declared_halt` est en place, armé et
attesté, **sans aucun consommateur en production**. Ce vol livre le consommateur.

Ticket : senara-solutions/mika#3142
Branche : `fix/3142/dispatch-lib-le-secours-lit-la-halte-d`
Instruction de périmètre, citée du corps du ticket : *« Le vol 2b porte le reste : le
refus structurel (`_recovery_pr_due=0`, `NO_PR: pilot_halt_declared`, le
`PIPELINE_INCOMPLETE` nommé) et la garde de `_push_branch`. Ne pas re-groomer le
périmètre complet. »*

Plans amont : `docs/plans/2026-11-24-003-fix-3142-…-plan.md` (vol 2a, GROOMED) et
`docs/plans/2026-11-22-003-fix-3119-dispatch-lib-le-secours-wip-rescue-plan.md`
§ *Défaut B*, qui porte la mesure des deux logs.

---

## Ce qui est déjà en place, et ne doit pas être refait

Vérifié sur la branche à `ee6cf125` :

| livrable du vol 2a | site mesuré |
|---|---|
| `_PILOT_HALT_TAIL_WINDOW=120`, `_PILOT_HALT_DECL_RE` | `dispatch-lib.sh:8269`, `:8273` |
| `_pilot_declared_halt` + son en-tête doctrinal (8225-8263) | `dispatch-lib.sh:8280-8289` |
| suite offline, 13 cas V1-V11 + S0 + S1 + S2 | `skills/bundled/_shared/tests/test_pilot_halt_declared.sh` (309 l.) |
| step nommé du job `dispatch-lib-offline-suites` | `.github/workflows/ci.yml:762` |
| target local + `.PHONY` | `Makefile:237`, `Makefile:4` |

Le prédicat n'a **rien** à corriger. Son en-tête porte en revanche un paragraphe qui
cessera d'être vrai au premier commit de ce vol :

```
# NO CONSUMER IN THIS FLIGHT. The detector is armed — its suite runs in the
# `dispatch-lib-offline-suites` job and goes red on a regression — but neither
# `_push_branch` nor the recovery block consults it yet: the refusal and the
# push abstention depend on an arbitration reserved to the operator and ship
# under `Tracked in: senara-solutions/mika#3142`.          (dispatch-lib.sh:8259-8263)
```

Ce paragraphe est un livrable de ce vol : il doit être **remplacé par le nom de son
consommateur**, et non supprimé. Un détecteur dont l'en-tête annonce « aucun
consommateur » alors qu'un refus le consulte est la forme exacte du commentaire qui
mentira au prochain lecteur.

## Le défaut qui reste ouvert

Le prédicat sait reconnaître une halte ; **personne ne le lui demande**. Donc, à
l'identique de ce qui a été mesuré sur #3081, #3092, #3100 et #3108 : la signature
observable d'un vol halté — `STATUS=success`, pas de PR, un HEAD avancé, pas de queue
de livraison — est celle que le secours de fin de vol traite en chemin **nominal**
(`no-shipping-tail`, mika#2492, chemin normal sous les queues allégées de mika#2694).
Le secours pousse puis ouvre une draft, et sur #3081 le démon
`auto_resume_wip_rescue_drafts` (mika#1852) l'a dé-draftée : le travail halté est parti
en revue sans décision.

**Les DEUX classes de secours sont atteignables sur cette population, et c'est ce qui
décide du périmètre du refus.** Si le pilote a commité avant de mesurer et de halter,
le worktree est propre, `PRE_RUN_HEAD != POST_RUN_HEAD` et la classe est
`no-shipping-tail` (`dispatch-lib.sh:14942-14944`) — c'est très probablement #3081, dont
la draft portait `rescue-pipeline-verified: yes`, donc un pipeline vérifié sur un travail
abouti. S'il a halté sans commiter, le secours WIP commite, pose
`RESCUED_DIRTY_WORKTREE=1` (`:8073`) et la classe est `dirty-worktree` (`:14940-14941`).
**Un refus clavé sur une classe en laisserait donc une ouverte.** Le refus porte sur
`_recovery_pr_due` (`:14955-14958`), qui couvre les trois classes par construction —
c'est l'argument exact de mika#2503, dont le commentaire (`:15007-15014`) dit qu'un
correctif posé au site du drapeau de classe *« passerait tous les tests locaux et ne
changerait RIEN en production »*.

## Décision réservée à l'opérateur — la garde de `_push_branch`

Le plan du vol 2a a nommé cette réserve et a écrit, textuellement, que *« c'est le
grooming du vol 2b qui doit le porter à l'opérateur »*. Je la porte ici, chiffrée, et
**je n'en pratique aucune branche** (mika#3122 §2).

**La question, en une phrase :** sur une halte déclarée, le secours s'abstient-il de la
**draft seule**, ou du **push ET de la draft** ?

**Pourquoi elle ne se tranche pas au grooming.** Le coût de l'option 2 est une perte
irréversible, mesurable au site : `_set_up_worktree` force-supprime le worktree au
dispatch suivant — le commentaire de `test_rescue_scratch_not_published.sh:15-18` le dit
en propres termes, et c'est le fait fondateur de mika#1282. Une branche non poussée est
donc du travail **détruit**, pas du travail en attente.

| option | geste | coût, nommé | taille |
|---|---|---|---|
| **1 — draft seule** | Le patron des deux refus existants : `_push_branch` tourne (`:14911`), la branche part sur `origin`, puis `_recovery_pr_due=0`, `NO_PR: pilot_halt_declared`, commande de publication manuelle dans `RESULT`. `_push_branch` n'est pas touché. | **Ferme l'effet mesuré en entier** : sans draft, `auto_resume_wip_rescue_drafts` n'a rien à dé-drafter, et l'incident #3081 est clos. Le travail halté survit sur `origin`. Reste un artefact que personne n'a réclamé : une branche poussée sur une halte. **Le corps du ticket demande « ni push ni PR »** — l'option 1 n'honore que la seconde moitié. | **229 l.** (le socle ci-dessous) |
| **2 — push et draft** | L'option 1, **plus** une garde en tête de `_push_branch` (`:9281`), sur le précédent `Push: SKIPPED — … ; return 1` du garde de commits dupliqués (`:9293-9299`). | Le travail halté reste **local au worktree**, donc **détruit au dispatch suivant**. Cela contredit la doctrine *rien n'est perdu* que les deux refus existants écrivent dans `RESULT` (`:14997`, `:15046`) et qui est la raison d'être de mika#1282. Honore la lettre du corps du ticket. | **314 l.** (socle + 30 l. de garde + 55 l. de suite) |

**Ce que l'en-tête du corps dit, et ses deux lectures.** L'en-tête *« Prochain vol : 2b »*,
écrit le 2026-11-25 après la livraison du vol 2a, nomme *« la garde de `_push_branch` »*
dans le périmètre. C'est l'indice le plus fort en faveur de l'option 2, et il se lit de
deux façons : (a) l'opérateur a tranché pour l'option 2, en connaissance du plan du vol 2a
qui nommait la réserve ; (b) l'en-tête re-cite la liste du § *Périmètre attendu* du corps,
écrite **avant** que la réserve n'existe, et n'arbitre donc rien sur le coût. **Je ne lève
pas la réserve sur cette ambiguïté** : la question n'est pas « quel livrable », elle est
« accepte-t-on de détruire le travail halté », et cette réponse n'est pas dans l'en-tête.

**Ce que la réponse change, et ce n'est pas seulement 85 lignes.** Elle change la phrase
que l'opérateur lit dans le callback. Sous l'option 1, `RESULT` peut écrire *« rien n'a
été perdu — la branche `<B>` est poussée »*, sur le modèle de `:15046`. Sous l'option 2,
cette phrase serait **fausse**, et le refus doit dire l'inverse : le travail est local et
sera détruit au prochain dispatch, avec le geste de sauvetage nommé
(`git -C <WORKTREE_DIR> push origin HEAD:refs/heads/<B>`). Un refus qui promet une
conservation qu'il vient d'annuler est pire que pas de refus.

**Réponse attendue de l'opérateur :** `option 1` ou `option 2`. Rien d'autre n'est
bloqué par cette réponse — le socle ci-dessous ne la consulte pas.

## Ce que livre ce vol — le socle, qui ne dépend pas de la réponse

### 1. Le troisième refus structurel du bloc Unit 2

**Placement : PREMIER des trois, juste après le calcul de `_recovery_pr_due`
(`:14955-14958`) et avant le refus mika#3038 (`:14989`).** La règle d'ordre est
déjà écrite par mika#3038 (`:14968-14976`) : *« les deux sont mutuellement exclusifs par
le drapeau que le premier à tirer éteint, donc celui qui tourne en premier est celui qui
nomme la raison — la diagnose la plus spécifique passe en premier. »* Une halte déclarée
est la plus spécifique des trois : c'est une **déclaration explicite du pilote**, là où
les deux autres sont des **inférences sur le contenu**. Sur un vol halté dont l'arbre se
trouverait égal à main, ou dont le contenu serait du scratch, `rescue_adds_nothing_to_main`
ou `rescue_scratch_only` enverraient l'opérateur examiner une propriété du contenu alors
que le fait est une décision du pilote.

Conséquence du placement, assumée : `_branch_tree_equals_main` est appelée **hors** du
`if` (`:14988`), donc elle tourne même quand le refus de halte a déjà éteint le drapeau (`:14989`),
et son `elif [ "$_tree_rc" -eq 2 ]` peut encore appeler `_note_tree_compare_undetermined`.
**Laisser tel quel** : la note reste vraie (la comparaison est bien indéterminée), elle
devient seulement sans conséquence, et la garder inchangée évite de toucher le refus
voisin sans nécessité.

Le bloc, sur le patron des deux refus existants :

```sh
if [ "$_recovery_pr_due" = "1" ] && _pilot_declared_halt; then
    _recovery_pr_due=0
    # Convenance, jamais une sonde : sur un dispatch réussi le tuyau de
    # `spawn_long_running_exec` est abandonné sans lecture — Signal M,
    # classe mika#2050. La surface opérateur est RESULT.
    echo "rescue_halt_refused: the pilot declared a halt — draft PR withheld (branch=$BRANCH, mika#3142)" >&2
    RESULT="${RESULT}
<la prose ; sa dernière phrase dépend de l'option retenue — voir § Décision réservée>"
    _set_pr_status_line "NO_PR: pilot_halt_declared"
    _set_outcome_line "Outcome: PIPELINE_INCOMPLETE — pilot_halt_declared: the pilot declared a per-flight-bound halt; the perimeter needs splitting before another flight."
fi
```

Trois propriétés de forme, chacune mesurée dans ce fichier :

- **`_set_pr_status_line`, jamais une concaténation** (`:3305`) : la fonction efface toute
  `PR:`/`NO_PR:` antérieure, donc le callback délivré porte **exactement une** ligne de
  statut — `_post_flight_recovery` en a déjà écrit une (`:8969`, `:8972`) au moment où on
  arrive ici.
- **Le motif est un token `snake_case` nu.** Le consommateur est strict :
  `dispatcher.rs::RE_NO_PR` vaut `(?m)^NO_PR:\s+([a-z_]+)`, déclaré classe B dans
  `scripts/canonical-tokens.tsv:376`. De la prose sur cette ligne ferait enregistrer au
  parseur le premier mot minuscule de la phrase.
- **`_set_outcome_line`, jamais une concaténation** (`:3322`) : même raison, pour la ligne
  `Outcome:` — `_post_flight_recovery` en pose une en amont (`:9021-9034`) et elle est
  transitoire par construction.

**Rien à ajouter à `scripts/canonical-tokens.tsv`** : il déclare le token `NO_PR` et son
site, jamais ses motifs individuels (vérifié : une seule ligne, `:376`).

### 2. La ligne `Outcome:` — et pourquoi elle n'est pas un ornement

Les deux refus existants n'appellent **pas** `_set_outcome_line` : ils laissent la ligne
`Outcome:` que `_post_flight_recovery` a posée. Le ticket en demande une, et le site le
justifie : `^Outcome: PIPELINE_INCOMPLETE` est **dans l'alternation** du garde
`empty_completion` (`:7254`). Un vol halté qui n'ouvre plus de draft produit un cycle
maigre, exactement la population que ce garde classerait `PIPELINE FAILURE:
empty_completion — this cycle produced nothing` (`:7259`). La ligne posée ici éteint cette
seconde bannière, qui serait fausse : le cycle a produit du travail, et c'est le pilote
qui a refusé de le livrer.

Deuxième raison, structurelle : sur le chemin nominal c'est **l'ouverture de la draft**
qui réécrit la ligne, par `_set_outcome_line "Outcome: PR_OPENED — ${PR_URL}"` (`:15210`).
Un refus qui retient la draft retient donc aussi cette réécriture, et laisse en place
l'`Outcome:` transitoire posé en amont par `_post_flight_recovery` (`:9021-9034`, dont le
commentaire de `:9025` dit explicitement que le site d'ouverture la réécrit). La ligne
posée par le refus est ce qui remplace la réécriture qui n'aura pas lieu.

### 3. L'énumération de `parse_no_pr_reason`, dans le MÊME commit que le site émetteur

`crates/mika-agent/src/task_engine/dispatcher.rs:6866-6882` — le commentaire de doc de
`parse_no_pr_reason` (`:6883`), qui se déclare lui-même *« the only enumeration an
operator reads to know what the loop can answer »* et qui raconte déjà qu'un motif émis
non énuméré est arrivé une fois (`rescue_scratch_only`, trouvé manquant par mika#3038) —
et la table de `test_parse_no_pr_reason_known_reasons` (`:8277-8295`, **7 entrées
mesurées**, qui passe à 8). Un motif émis que l'énumération ne nomme pas est un motif
qu'aucun opérateur ne peut chercher ; les deux moitiés vont donc dans le commit du site
émetteur, pas dans un suivant.

**`pilot_halt_declared` n'est PAS un membre de la famille `pilot_terminated_*`, et le
plan le dit pour que personne ne les fusionne.** `_no_pr_reason_from_halt_subtype`
(`:7397`) produit déjà une famille **ouverte** de motifs `pilot_terminated_<sous-type>`,
posée au site `:6822`, à partir du sous-type que claude-pilot rend (`max_turns`, session
silencieuse, refus de politique) ou que le repli mika#2149 gratte dans stderr. Les deux
notions sont disjointes, sur les deux axes qui comptent :

- **le statut** : la famille `pilot_terminated_*` vit sous le chemin `terminated` ; une
  halte déclarée arrive avec `STATUS=success` — c'est la signature même que le corps du
  ticket décrit, et la raison pour laquelle le secours la traite en chemin nominal. Le
  site `:6822` n'intercepte donc **jamais** cette population, et les deux motifs ne
  peuvent pas se disputer la ligne ;
- **le sujet** : `pilot_terminated_*` dit que le **substrat** a arrêté la session ;
  `pilot_halt_declared` dit que le **pilote** a mesuré le périmètre et refusé de le
  livrer, en obéissant à sa borne.

Deux conséquences pratiques. Le nouveau motif s'écrit **en clair dans l'énumération**, et
non par un détour dans `_no_pr_reason_from_halt_subtype` — y ajouter un sous-type
synthétique ferait mentir sa normalisation et son nom. Et sa longueur (`pilot_halt_declared`,
19 caractères) reste sous le motif le plus long déjà émis, les 46 caractères de
`callback_no_pr_pilot_terminated_error_max_turns` que le commentaire de `:7380` pèse : le
format de colonne n'est pas un sujet.

### 4. L'en-tête du prédicat, qui cesse de dire « aucun consommateur »

Remplacement du paragraphe `:8259-8263` par le nom de son consommateur : le refus
d'Unit 2, son numéro de ligne approximatif, et ce qui reste différé si l'opérateur
retient l'option 1 (l'abstention de push).

## Tests

**Extension de `test_pilot_halt_declared.sh`**, pas une nouvelle suite : ses helpers
(`assert_eq`, `assert_contains`, `section`, `case_of`), son `DISPATCH_LIB`, son `CI_YML`
et son `REPO_ROOT` sont déjà posés, et le step CI qui la lance existe déjà. Une seconde
suite re-paierait tout cela et ajouterait un step à câbler.

**Le câblage est attesté STATIQUEMENT, et c'est le précédent, pas un raccourci.** Le bloc
Unit 2 vit inline dans `dispatch_claude_pilot` : il n'est pas appelable depuis une suite.
mika#2503 (V14-V18) et mika#3038 ont tous deux attesté leur refus par `grep` et `sed` sur
`dispatch-lib.sh`. On fait pareil, et on nomme la limite : ces assertions prouvent que le
refus **est au bon endroit et dit la bonne chose**, jamais qu'il a tiré sur un vol réel.

| cas | ce qu'il assure | ce qu'il ferme |
|---|---|---|
| **W1** | les quatre conditions de `_recovery_pr_due` sont énoncées **exactement une fois**, et le bloc d'ouverture est gardé sur ce seul drapeau (`grep -c '^    if \[ "\$_recovery_pr_due" = "1" \]; then$'` = 1) | un second bras qui re-énoncerait les conditions contournerait le refus |
| **W2** | le refus consulte `_pilot_declared_halt` et éteint le drapeau (`_recovery_pr_due=0` dans les deux lignes qui suivent le test) | le détecteur reste inerte si personne ne l'appelle — le défaut que ce vol ferme |
| **W3** | le refus n'est clavé sur **aucune** classe : ni `RESCUED_DIRTY_WORKTREE`, ni `RECOVERY_CLASS`, ni `no-shipping-tail` dans sa région | le piège mika#2503, et les deux classes atteignables du § *Le défaut qui reste ouvert* |
| **W4** | **l'ordre** : l'offset du refus de halte est strictement inférieur à celui du refus mika#3038, lui-même inférieur à celui de mika#2503 | une « harmonisation » qui réordonnerait les trois ferait nommer la mauvaise raison |
| **W5** | la région du refus porte : `RESULT=`, `mika#3142`, la branche, le geste manuel, exactement un `_set_pr_status_line "NO_PR: [a-z_]+"$`, et exactement un `_set_outcome_line` dont le corps contient `PIPELINE_INCOMPLETE` **et** `pilot_halt_declared` | AC2, AC3, AC4 — et le format de fil que `RE_NO_PR` impose |
| **W6** | l'en-tête de `_pilot_declared_halt` ne contient plus `NO CONSUMER IN THIS FLIGHT`, et nomme `_recovery_pr_due` | un commentaire qui mentira au prochain lecteur |
| **W7** | **contre-témoin de l'extension** : la région du refus ne contient **pas** `_branch_tree_equals_main` ni `_rescue_touches_tracked_tree` — les trois refus restent trois prédicats distincts | une fusion des trois en un seul test, qui rendrait un seul motif pour trois faits |
| **W8** *(option 2 uniquement)* | `_push_branch` consulte `_pilot_declared_halt` **avant** son `fetch` (`:9302`), écrit une ligne `Push: SKIPPED — …` nommant la halte, et `return 1` sur le patron de `:9293-9299` ; et la prose d'Unit 2 ne promet **pas** une branche poussée | la cohérence des deux moitiés : sous l'option 2, `RESULT` doit dire la destruction, pas la conservation |

**Côté Rust**, aucun nouveau test : `test_parse_no_pr_reason_known_reasons` est une table
et gagne une entrée. Son voisin `test_parse_no_pr_reason_anchored_not_free_text:8299`
couvre déjà l'ancrage pour tout motif.

**`scripts/check-pilot-push-sites.sh` ne rougit sur aucune des deux options — vérifié.**
Sa population est `skills/bundled/*/handlers/*.sh` et son en-tête déclare `_shared/`
**hors** population (`:27-28`), parce que ce répertoire porte *« the one legitimate,
guarded push site »* — c'est-à-dire exactement `_push_branch`. Le plan du vol 2a nommait ce lint comme une « contrainte supplémentaire » de
l'option 2 ; la mesure dit que c'est un non-sujet.

**Rien à ajouter à `scripts/suite-wiring.tsv`** : ce registre ne porte que le résidu, et
la suite y est déjà absente — c'est l'assertion S2d de la suite elle-même.

## Ce que ce vol ne corrige PAS, et qui se lit en aval

`Outcome: PIPELINE_INCOMPLETE` fait classer le vol `DeathCause::PipelineFailure` par
l'évaluateur de milestone (`crates/mika-agent/src/milestone_manager/evaluator.rs:1080`),
donc `FlightRecommendation::ManualResumption` — dont le commentaire au site dit *« le
ticket n'est pas en cause, le substrat l'est »* (`:963`). **Sur une halte déclarée c'est
l'inverse du fait** : le ticket EST en cause, il est trop gros, et la recommandation
juste est `Regroom` — celle que `CeilingReached | Refusal | Escalated` obtiennent
(`:956-959`), pour la raison exacte que le site écrit : *« le vol dit quelque chose sur
le ticket lui-même — trop gros, mal posé »*.

Ce vol **n'y touche pas**, et ne détourne pas un token existant pour l'obtenir :
`STRUCTURAL VIOLATION:` (`→ Refusal`, `:1072`) est réservé à une violation d'autorité —
un push du pilote, `dispatch-lib.sh:14733` — et une halte déclarée est le contraire,
c'est le pilote qui **obéit** à la borne. L'écrire là serait un mensonge de token pour
obtenir une recommandation.

La correction juste est une branche de `death_cause` sur le motif `pilot_halt_declared`,
dans `evaluator.rs`, avec sa suite : hors périmètre de ce vol et de sa borne.
**À ouvrir en ticket de suivi par l'opérateur** — je ne l'ouvre pas d'ici, et le vol ne
dépend pas de son existence. Formulation pour le corps de PR, quand le numéro existera :
`Tracked in: senara-solutions/mika#<n>`.

## Ordre de commit

1. Le refus d'Unit 2 + son en-tête doctrinal + la réécriture de l'en-tête de
   `_pilot_declared_halt` (`:8259-8263`). Un seul commit : le second rend vrai ce que le
   premier change.
2. `dispatcher.rs` — le commentaire de doc et la table, **dans le même commit que (1)**
   si le diff reste lisible, sinon immédiatement après et jamais plus tard.
3. L'extension de suite : W1 → W7.
4. *(option 2 seulement)* La garde de `_push_branch`, la correction de la prose d'Unit 2,
   et W8 — dans un **seul** commit : une garde dont le `RESULT` promet encore une branche
   poussée est un refus qui ment.

**Plancher mergeable** : (1) + (2) + W1, W2, W3, W5, W6. Si la suite déborde son
estimation, **W4 et W7** sont les deux cas à partir — ils attestent l'ordre et la
distinction des trois refus, pas l'existence du refus ni sa forme de fil. Rien d'autre ne
part : retirer W1 ou W2 livrerait un refus non attesté, et retirer (2) livrerait un motif
illisible.

## Risques

| risque | parade |
|---|---|
| Le refus tire à tort et **retient** un artefact — la perte que mika#1282 existe pour empêcher. | La direction du fail-safe est déjà celle du prédicat, livrée et attestée au vol 2a : toute indéterminabilité rend « pas de halte » (V9-V11). Le refus hérite de cette direction sans la redécider. |
| Le refus est posé au bon endroit mais le bloc d'ouverture est atteint par un autre chemin. | W1 : les quatre conditions sont énoncées une seule fois et le bloc est gardé sur ce seul drapeau. C'est l'argument structurel de mika#2503, et son assertion V14 existe déjà pour le refus voisin. |
| Le placement en premier des trois rend `_note_tree_compare_undetermined` appelable sans conséquence. | Assumé et nommé au § *Placement* : la note reste vraie, et la modifier toucherait le refus voisin sans nécessité. |
| Le nouveau motif est émis et l'énumération Rust ne le nomme pas. | Même commit que le site émetteur (§ *Ordre de commit*), et c'est l'incident que le commentaire de `parse_no_pr_reason` raconte déjà sur `rescue_scratch_only`. |
| `_signal_rescue_into_open_pr` (`:14921`, définie `:9736`) écrirait sur une PR alors que le refus l'a retenue. | Sans objet, vérifié, et pour une raison structurelle plutôt que par chance : les deux gestes d'écriture sont derrière `[ -z "$_open_pr" ] → return 0` (`:9747-9753`), et le cas où une PR **est** ouverte est exactement celui où `PR_URL` n'est pas vide, donc où `_recovery_pr_due` vaut 0 et où notre refus ne tire pas. Les deux branches sont disjointes par la même condition. |
| Effet résiduel, **partagé avec les deux refus existants** : sur une halte en worktree sale, `_signal_rescue_into_open_pr` marque les commits de secours « signalés » (`_mark_rescue_commits_signalled`, `:9751`) avant qu'Unit 2 ne retienne la draft. Si l'opérateur publie la draft plus tard à la main, le commentaire mika#2151 ne sera jamais posté dessus. | Nommé, non corrigé : l'effet est identique sous mika#2503 et mika#3038 et ne naît pas de ce vol. Le corriger demanderait de déplacer le marquage après le calcul de `_recovery_pr_due`, donc de toucher les deux refus voisins — hors périmètre et hors borne. |
| Le vol dépasse la borne parce que l'opérateur retient l'option 2. | Chiffré : 314 lignes, soit 115 sous la borne de 429. Les deux options tiennent en un vol. |
| La réponse de l'opérateur n'arrive pas. | Le socle (229 l.) ne la consulte pas et ferme l'effet mesuré sur #3081 en entier. C'est l'option 1 ; l'option 2 est un incrément de 85 lignes sur la même branche. |

## Definition of Done

- Un troisième refus structurel existe dans le bloc Unit 2 de
  `skills/bundled/_shared/dispatch-lib.sh`, **avant** les refus mika#3038 et mika#2503,
  qui consulte `_pilot_declared_halt`, pose `_recovery_pr_due=0`, écrit sa prose dans
  `RESULT`, et pose `NO_PR: pilot_halt_declared` + une ligne
  `Outcome: PIPELINE_INCOMPLETE — pilot_halt_declared: …` par les helpers canoniques.
- L'en-tête de `_pilot_declared_halt` nomme son consommateur et ne dit plus
  `NO CONSUMER IN THIS FLIGHT`.
- `parse_no_pr_reason` nomme `pilot_halt_declared` dans son commentaire de doc, et
  `test_parse_no_pr_reason_known_reasons` porte 8 entrées.
- `test_pilot_halt_declared.sh` porte W1-W7 (et W8 sous l'option 2) en plus de ses
  13 cas existants, et passe hors réseau.
- Sous l'option 2 seulement : `_push_branch` porte sa garde, et la prose d'Unit 2 nomme
  la destruction au prochain dispatch plus le geste de sauvetage, au lieu de promettre
  une branche poussée.
- `scripts/verify-pipeline.sh origin/main`, `make test-pilot-halt-declared`,
  `make verify-no-sigpipe-grep`, `make check-suite-wiring`, `make test-pilot-push-guard`,
  `make verify-bundled-skills`, `cargo test -p mika-agent`, `cargo clippy` et
  `cargo fmt --check` passent.
- La PR porte `Closes mika#3142` (ce vol est le dernier du ticket),
  `Code review: delegated to mika-qa (mika#2694)`, et une ligne `Tracked in:` nommant le
  ticket de suivi de `evaluator.rs` si l'opérateur l'a ouvert.

## Acceptance criteria

Dérivés : le corps de senara-solutions/mika#3142 ne porte pas de section
`## Acceptance criteria`.

- **AC1 — sur une halte déclarée, aucune draft n'est ouverte.** Un terme conjonctif
  consultant `_pilot_declared_halt` éteint `_recovery_pr_due`, il est placé avant les deux
  refus existants, et le bloc d'ouverture reste gardé sur ce seul drapeau, énoncé une
  seule fois. (W1, W2, W4)
- **AC2 — le motif part sur le fil dans le format que le parseur accepte.** Exactement une
  ligne `NO_PR: pilot_halt_declared`, posée par `_set_pr_status_line`, motif `snake_case`
  nu — `(?m)^NO_PR:\s+([a-z_]+)`. (W5)
- **AC3 — le refus est lisible sans grep.** `RESULT` nomme le ticket, la branche et le
  geste manuel ; aucune sonde n'est annoncée sur le `echo` de stderr. (W5)
- **AC4 — une ligne `Outcome:` et une seule, qui nomme le motif.**
  `Outcome: PIPELINE_INCOMPLETE — pilot_halt_declared: …`, posée par `_set_outcome_line`.
  (W5)
- **AC5 — l'énumération que l'opérateur lit nomme le motif.** Le commentaire de doc de
  `parse_no_pr_reason` et la table de `test_parse_no_pr_reason_known_reasons` portent
  `pilot_halt_declared`, livrés dans le commit du site émetteur. (`cargo test -p mika-agent`)
- **AC6 — le refus n'est clavé sur aucune classe de secours.** Sa région ne mentionne ni
  `RESCUED_DIRTY_WORKTREE`, ni `RECOVERY_CLASS`, ni un nom de classe, et reste distincte
  des deux prédicats voisins. (W3, W7)
- **AC7 — l'en-tête du détecteur dit la vérité sur son consommateur.** Il ne contient plus
  `NO CONSUMER IN THIS FLIGHT` et nomme `_recovery_pr_due`. (W6)
- **AC8 — l'abstention de push suit la décision de l'opérateur, dans les deux moitiés.**
  Sous l'option 1 : `_push_branch` ne mentionne pas `_pilot_declared_halt`, et la prose
  d'Unit 2 dit que la branche est poussée. Sous l'option 2 : `_push_branch` consulte le
  prédicat avant son `fetch`, écrit `Push: SKIPPED — …` nommant la halte, `return 1`, et
  la prose d'Unit 2 nomme la destruction au prochain dispatch plus le geste de sauvetage.
  (W8 sous l'option 2 ; la moitié « option 1 » est une assertion d'absence)

## Fire-Disposition

Ce vol livre un **détecteur** : le refus structurel, dont le chemin de succès est « aucune
halte déclarée », et les sept à huit cas de l'extension de suite.

**Option retenue : (a) exception nommée en allowlist — et l'allowlist atterrit VIDE par
absence d'objet.**

Le raisonnement du vol 2a reste exact et se vérifie au site : le prédicat porte sur un
**log de session éphémère** (`$_PILOT_LOG_DIR/<LOG_ID>.log`, posé en `:6678`), jamais sur
un artefact versionné. Aucun fichier du dépôt, aucune ligne de code, aucune population
existante ne peut le violer, donc **rien n'est exceptable** — pas « rien d'excepté par
choix », mais pas d'objet. Les quatre drafts déjà ouvertes par le défaut (#3081, #3092,
#3100, #3108) ne sont pas une population que le détecteur scanne : il décide au moment du
vol, sur un log qui n'existe plus après.

Conformément à mika#1574, l'allowlist vide est remplacée par la **paire témoin /
contre-témoin** qui prouve que le détecteur discrimine au lieu de toujours répondre
pareil — et pour ce vol elle porte sur le **câblage**, le prédicat lui-même étant déjà
attesté par V1-V11 :

- **témoins** (le refus est bien là, et il parle) : W2 — il consulte le prédicat et éteint
  le drapeau ; W5 — il écrit les trois lignes canoniques ; W6 — l'en-tête nomme son
  consommateur ;
- **contre-témoins** (il ne retient rien de plus que ce qu'il doit) : W1 — le bloc
  d'ouverture reste gardé sur un seul drapeau énoncé une seule fois, donc le refus ne peut
  être ni contourné ni élargi ; W3 — il n'est clavé sur aucune classe ; W4 — il ne déplace
  pas les deux refus voisins ; W7 — il ne fusionne pas avec eux.

Sans les témoins, un diff qui n'ajouterait rien passerait tous les contre-témoins. Sans
les contre-témoins, un refus posé en travers du bloc entier — qui retiendrait toute draft
— passerait tous les témoins. La paire est l'attestation.

**Ce n'est pas l'option (b).** Rien n'atterrit désarmé : le step CI du job
`dispatch-lib-offline-suites` existe déjà (`ci.yml:762`) et lance la suite étendue sans
modification. **Ce n'est pas non plus l'option (c)** pour le détecteur : la halte-et-
remontée de ce plan porte sur la **garde de `_push_branch`** (§ *Décision réservée*), pas
sur l'armement du refus de draft, qui part dans tous les cas.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `skills/bundled/_shared/dispatch-lib.sh` — refus structurel d'Unit 2 (en-tête doctrinal + code) | 55 |
| `skills/bundled/_shared/dispatch-lib.sh` — en-tête de `_pilot_declared_halt` : son consommateur remplace `NO CONSUMER IN THIS FLIGHT` | 10 |
| `crates/mika-agent/src/task_engine/dispatcher.rs` — commentaire de doc de `parse_no_pr_reason` + table de `test_parse_no_pr_reason_known_reasons` | 14 |
| tests — extension de `test_pilot_halt_declared.sh`, W1-W7 | 150 |
| `skills/bundled/_shared/dispatch-lib.sh` — garde de `_push_branch` + prose d'Unit 2 corrigée — **option 2 seulement** | 30 |
| tests — W8, la cohérence des deux moitiés — **option 2 seulement** | 55 |

Total estimé : 314 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient en UN vol, dans les deux issues de la décision réservée.** Le
total déclaré est le **pire cas chiffré** — option 2 retenue : 314 lignes, 115 sous la
borne. Sous l'option 1 le vol pèse 229 lignes, 200 sous la borne. Le périmètre complet du
ticket était re-mesuré à ~660 lignes, au-dessus de la borne, d'où le découpage que le
corps prescrit ; le vol 2a en a emporté 410, et ce vol emporte le reste. Le seuil par PR
de mika#2636 (1000 lignes) n'est atteint par aucun des deux vols : c'est la borne par vol
qui mord, et elle ne mord plus ici.

Trois raisons pour lesquelles le reste tient, là où le périmètre complet ne tenait pas :
la suite est **étendue** et non créée (ses helpers, son `source`, son step CI et sa target
`Makefile` sont déjà payés par le vol 2a), le détecteur est déjà écrit et attesté, et le
câblage s'atteste **statiquement** sur le précédent de mika#2503 — là où les 741 lignes de
`test_rescue_tree_equals_main.sh` ont aussi payé un dépôt git fixture et un rejeu de
secours que ce vol n'a pas besoin de monter.
