# Plan — vol 6 de mika#2680 : la cellule (g) est chiffrée, et sa prémisse est réfutée

> Vol 6. Vols antérieurs : #2705 (registre), #2747 (lecteur), #2764 (classe (e)),
> #2828 (terme de relâche), #2829 (croisement des taxonomies). Le protocole de
> mesure est `docs/eval/mika-2680/README.md`.

La mesure 3 existe. L'orchestrateur l'a prise hors bac à sable le 2026-10-19 sur
la fenêtre `06:02Z–16:30Z`, avec la commande exacte du § 3. **Ce vol ne mesure
rien : il commite l'agrégat et applique les §§ 4 à 6 à la cellule (g).**

La réponse est : **aucun filtre.** Ce document dit pourquoi, et la raison tient en
deux termes que l'agrégat et le code servi donnent séparément.

---

## État des lieux, vérifié sur l'arbre servi

| fait | où il se lit | état |
|---|---|---|
| `docs/eval/mika-2680/` ne contient que `README.md` | `ls docs/eval/mika-2680/` | aucun artefact n'y a jamais atterri |
| la cellule (g) est entièrement en `no_decision_attested` | corps du ticket, `by_agent_trigger_release` | 36 tours, 10 687 327 tokens d'entrée, 202 appels LLM |
| aucune ligne `mika-qa × check_suite_pr_branch × head_not_previously_decided` | idem | le seul `head_not_previously_decided` de la fenêtre est à mika-dev (1 tour) |
| `green_check_suite_head_decided` a écarté 1 tour, celui de mika-dev | `skipped_by_agent_class` | la classe (b) n'écarte rien côté relecteur |
| la porte d'entrée de `ci_success_handler` rend `Passthrough { enrichment: None }` | `ci_success_handler.rs:209-213` | **délibérément**, pour que le tour du relecteur voie l'événement brut |
| `qa-review-webhook-success` est allowlistée sur mika-qa | `well_known_agents.rs:323`, recensement `:2731` | le consommateur du tour existe et est servi |
| `DECISION_CORE_HOLD_TOOL` est écrit après la porte | `ci_success_handler.rs:139` (const), `:628` (écriture) | hors de portée d'un non-dispatcher |
| `DECISION_CORE_HOLD_TOOL` est `pub(crate)` | `ci_success_handler.rs:139` | inatteignable depuis `tests/eval/` |
| `docs/eval/` n'est pas dans la liste `DOCS` de la synchro | `scripts/sync-agent-docs.sh:13-23` | le job CI `docs-sync` n'est pas concerné par ce vol |

Arithmétique de l'agrégat, refaite : 10 687 327 / 65 507 602 = **16,31 %** des
tokens d'entrée de la fenêtre. `no_decision_attested` au grain de la fenêtre vaut
66 = 36 (mika-qa) + 30 (mika-dev) ; `head_not_previously_decided` vaut 1 au grain
de la fenêtre **et** 1 au grain de la cellule mika-dev. Les deux grains
concordent : il n'y a pas de ligne manquante, l'absence est une absence.

---

## Décision 1 — la soustraction que le § 7.1 prescrit est vide, et la raison est structurelle

Le § 7.1 demande de chiffrer « **après** retrait de ce que `skipped_by_class`
montre déjà écarté par (a)–(d) ». Le § 9 précise, depuis le vol 4, ce que ce
retrait vaut pour (g) : *« plafond moins les seaux que (b) revendique »*.

Sur cet agrégat, **(b) ne revendique rien** côté relecteur. Les deux opérandes
sont lus au grain que le vol 5 a livré :

- plafond : `mika-qa × check_suite_pr_branch` = 36 tours / 10 687 327 tokens ;
- retrait : la ligne `mika-qa × check_suite_pr_branch × head_not_previously_decided`
  **n'existe pas**, et `green_check_suite_head_decided` n'a écarté qu'un tour,
  attribué à mika-dev par `skipped_by_agent_class`.

Le chiffre de (g) après retrait est donc **égal à son plafond**. Ce n'est pas un
hasard de fenêtre, c'est une conséquence du code servi, et la chaîne se relit en
trois pas :

1. `decide_head_decided` (`webhook_prefilter.rs:815-842`) a besoin d'une ligne
   d'audit `DECISION_CORE_HOLD_TOOL` **pour cet événement** pour obtenir le
   préfixe de PR ; sans elle, elle rend `Decision::Llm(Release::NoDecisionAttested)`
   sans même lire les attestations de la PR.
2. Cette ligne est écrite à `ci_success_handler.rs:628`, soit **après** la porte
   d'entrée de mika#2260, qui refuse quand `!owns_merge_transition(agent_id)`.
   Le commentaire de la porte est explicite : *« la porte précède donc **tout** :
   le `evaluating` ci-dessous, l'exigence de token, le dedup, et toute ligne
   d'audit »*.
3. mika-qa n'est pas l'acteur de merge. Il n'atteint jamais le site
   d'attestation, donc **(b) ne peut pas même regarder** sa tête.

C'est pourquoi la cellule est à 36/36 en `no_decision_attested` et non pas
répartie : le terme de relâche ne décrit pas ici un cas limite, il décrit le
seul chemin possible. **Le plafond tient entier, à 16,31 % de la fenêtre.**

Le volume de (g) est donc **confirmé**. C'est le premier terme, et le seul que
l'agrégat donne.

---

## Décision 2 — la prémisse de (g) est fausse : le tour a un consommateur nommé, et il est délibéré

Le ticket pose (g) ainsi : *« Le relecteur reçoit le fan-out (mika#1711) **sans
qu'aucun verdict n'en dépende directement**. À mesurer : que fait mika-qa de ce
tour ? »* C'est la question du § 7.2 — *que fait le tour, et ce travail
survit-il autrement ?*

Elle a une réponse, et elle est positive : **ce tour est l'unique porte d'entrée
autonome de `qa-review` sur CI vert.**

- mika#2260 rend `Passthrough { enrichment: None }` **et non** `Handled`, et son
  commentaire dit pourquoi en nommant le consommateur :
  *« le tour du relecteur doit voir l'**événement brut**, que
  `qa-review-webhook-success` attend pour corréler la PR »*
  (`ci_success_handler.rs:209-213`). La porte a été écrite pour **préserver** ce
  tour, pas pour le subir.
- `qa-review-webhook-success` est une skill bundled, allowlistée sur mika-qa
  (`well_known_agents.rs:323`, recensée en `:2731`), et son étape 5 dispatche la
  vraie `qa-review`. Son prompt porte l'ordre inverse d'un tour vide :
  *« CRITICAL: DO NOT end your turn without acting. This is a CI-green
  notification that unblocks review. »*
- Son incident fondateur est mesuré, pas supposé : avant mika#1711, mika-qa ne
  recevait aucun signal de CI verte, ce qui a causé *« a 14-hour dead qa window
  on 2026-07-01 during which multiple autonomous-loop PRs landed READY+GREEN
  with zero qa attempts »*.

Écarter la cellule rouvrirait exactement cette fenêtre morte. Et le ticket
lui-même met ce travail hors périmètre, au § *Hors périmètre* : *« Le tour de
revue QA lui-même : légitime. »*

**La prémisse de (g) est donc réfutée sur le code servi** — de la même manière
que le vol 4 a réfuté celle de (f), et pour la même sorte de raison : le ticket
décrivait un tour que le code ne produit pas.

---

## Décision 3 — ce que l'agrégat ne peut pas dire, et pourquoi c'est là que ça s'arrête

Les décisions 1 et 2 ne s'additionnent pas en une implémentation, et il faut
dire précisément pourquoi.

Le registre porte `trigger`, `release`, `candidate`, `text_touched`,
`prefilter_enabled` et le coût. Il **ne porte pas ce que le tour a fait**. Or le
chemin de décision de `qa-review-webhook-success` a six étapes, dont quatre sont
des arrêts :

| étape | ce qu'elle fait | le tour est-il vide ? |
|---|---|---|
| 1 | corréler à une PR ouverte sur la tête ; aucune ⇒ arrêt | oui |
| 2 | hors périmètre (auteur non-boucle, brouillon) ⇒ arrêt | oui |
| 3 | déjà revue à **ce** SHA ⇒ arrêt | oui |
| 4 | verdict déjà enregistré sur la tête ⇒ arrêt | oui |
| 5 | **dispatcher la revue** | non — c'est le travail |
| 6 | discipline de tour (escalade si blocage) | non |

Les 36 tours de la cellule mélangent les deux populations, et **l'agrégat ne les
sépare pas**. Le seul indice qu'il porte est le coût moyen en appels LLM :

| cellule | tours | appels | appels/tour |
|---|---|---|---|
| `mika-qa × pr_other` | 31 | 427 | 13,8 |
| `mika-qa × pr_opened` | 11 | 138 | 12,5 |
| `mika-qa × check_suite_pr_branch` | 36 | 202 | **5,6** |

5,6 contre 12,5 et 13,8 : la cellule (g) coûte moins de la moitié d'un
déclencheur dont on sait qu'il dispatche la revue. Une décomposition tient
l'arithmétique — treize revues complètes à ~13 appels plus vingt-trois arrêts
précoces à ~1,5 donne 203,5, soit les 202 observés. **Mais elle repose sur un
coût par forme que le registre n'atteste pas**, et une autre décomposition le
tient aussi bien. Ce n'est pas un compte, c'est une vraisemblance.

Le § 7.3 n'admet que les classes dont la réponse au § 7.2 est *« rien, ou rien
qui nécessite un tour LLM »*. Ici la réponse est **« parfois tout, parfois
rien »**, et l'agrégat ne dit pas dans quelle proportion. Le § 7 tranche ce cas
en une phrase, et c'est la phrase qui s'applique :

> Une classe dont le chiffre est significatif mais dont la question reste
> ouverte ne s'implémente pas : elle se pose à l'opérateur.

**Aucun filtre n'est écrit.** C'est la seconde branche du point 2 du vol 6 :
*« Si elle ne la confirme pas, il l'écrit dans le corps de la PR, avec la raison
tirée de l'agrégat, et n'écrit aucun filtre. »*

---

## Décision 4 — aucun gel non plus, et c'est vérifié plutôt que supposé

Le vol 4 a gelé sa réfutation de (f) dans la suite
(`mika2680_f_le_pr_review_reecrit_est_deja_ecarte`, quatre assertions), *« de
sorte qu'un futur handler intercalé le fasse rougir au lieu de le rouvrir »*. Le
geste symétrique pour (g) serait d'épingler la conjonction qui porte la
décision 1 : *un non-dispatcher ne produit jamais de ligne
`DECISION_CORE_HOLD_TOOL`, donc (b) ne soustrait rien de son côté.*

**Ce gel n'est pas écrivable avec les instruments à portée**, et les deux sites
candidats échouent pour deux raisons distinctes :

- **`crates/mika-agent/tests/eval/test_ci_success_fanout_2260.rs`** — le seul
  harnais multi-agent. Deux empêchements : `DECISION_CORE_HOLD_TOOL` est
  `pub(crate)` (`ci_success_handler.rs:139`), donc inatteignable depuis un test
  d'intégration, qui est une crate externe ; et ce harnais tourne à
  `github_token = None`, si bien que **personne** n'atteint le site
  d'attestation — le chemin s'arrête sur l'exigence de token, la première ligne
  après la porte. Le compte serait zéro pour les deux agents. Ce fichier nomme
  lui-même ce piège et existe pour le retirer : *« le lire comme une preuve
  d'attribution serait une assertion vacue de plus, exactement ce que ce fichier
  existe pour retirer »* (l. 45-47). L'écrire serait un défaut, pas un gel.
- **le module de tests de `webhook_prefilter.rs`** — l'état y est bouchonné, donc
  « mika-qa ne produit jamais d'attestation » n'y est pas exprimable : le bouchon
  décide quelles lignes existent. La conjonction enjambe la porte (handler) et la
  relecture (pré-filtre), et les joindre demande un vrai jeton et une couture sur
  `gh`, que mika#1947 a déclarés hors périmètre.

Les deux moitiés sont d'ailleurs déjà épinglées séparément — l'ordre de la porte
par `test_ci_success_handler.rs`, la transparence du relecteur par
`assert_reviewer_transparent` dans le test du fan-out, et
« sans attestation ⇒ `NoDecisionAttested` » par
`mika2675_b_decide_sans_attestation_ne_lit_pas_la_pr`. Ce qui manque est leur
conjonction, et c'est précisément elle qui demande le jeton.

**Conséquence sur les gates de grooming :** ce vol ne livre **aucun détecteur** —
ni garde d'exécution, ni lint, ni assertion nouvelle. La section
`## Fire-Disposition` de mika#2306 est donc en **gate N/A**, et la règle dit de
ne pas l'inventer. Son absence ici est une décision écrite, pas un oubli.

---

## Décision 5 — le successeur (g′) est nommé et borné, et il se pose à l'opérateur

Poser une classe à l'opérateur sans dire ce qu'on lui demande n'est pas un
renvoi, c'est un haussement d'épaules. Voici la question, bornée.

Ce qui serait filtrable, ce ne sont pas les 36 tours : ce sont les **arrêts
précoces** des étapes 1 à 4. Et leur prédicat est lisible — le pré-filtre appelle
déjà la forge, `PrefilterState::pr_state` fait un `gh pr view <n> --json state`
(`webhook_prefilter.rs:538-552`) pour la classe (c). Techniquement, un
`gh pr list --head <branch> --state open` puis un
`gh pr view <n> --json reviews,headRefOid` suffiraient.

**Donc l'obstacle n'est pas la portée technique.** Ce sont trois arbitrages, et
aucun n'est à la main d'un pilote :

1. **Le coût que la porte de mika#2260 venait de retirer.** Son commentaire
   chiffre ce qu'elle a supprimé : *« cinq appels `gh` sous le PAT du
   relecteur »* (`ci_success_handler.rs:204`), et le commentaire de son unique
   ligne d'audit ajoute que connaître le numéro de PR *« coûterait le premier
   appel `gh`, c'est-à-dire l'effet de bord que cette porte retire »*
   (`:231-233`). Un filtre (g′) remettrait ces appels, au même endroit, sous le
   même jeton.
2. **Un garde de prompt qui descend dans le Rust.** Les étapes 2 à 4 vivent
   aujourd'hui dans `skills/bundled/qa-review-webhook-success/system_prompt.md`.
   Les porter en code servi crée deux sites de vérité qu'il faudra tenir
   alignés — et l'étape 4 du prompt dit explicitement que la source de vérité du
   dedup est le `commit_id` de la revue sur GitHub, *« do not add a separate
   task-metadata flag »*.
3. **La posture fail-open (AC3 de mika#2675).** Une lecture de forge en erreur
   doit laisser passer le tour. Le taux de `Release::StateUnreadable` sur ce
   chemin n'est pas mesuré ; un filtre dont la moitié des passages est un
   fail-open coûte ses appels `gh` sans rien écarter.

Et sa taille reste inconnue : l'agrégat donne le plafond (36 tours), jamais la
part qui serait effectivement écartée. **Mesurer (g′) demande un champ que le
registre ne porte pas** — ce que le tour a fait — donc un vol d'instrument, pas
un vol de filtre.

(g) rejoint ainsi (h) : chiffre significatif, question ouverte, posée à
l'opérateur. Mais sa question est désormais nommée, et c'est la différence.

---

## Décision 6 — ce que ce vol ne touche pas

| hors périmètre | pourquoi, et où ça vit |
|---|---|
| la classe (h) | § 7 : elle se pose, elle ne s'implémente pas. Aucune mesure ne la tranche (README § 9) |
| le résidu de (f) | une décision AC2 et trois chemins non-filtres (README § 9) ; ticket voisin de #2679 |
| le bind `bwrap` de `/var/log/mika` | décision opérateur ; élargir le bind donnerait à chaque pilote l'écriture sur une surface de diagnostic d'incident (README § 2 bis, mika#2041/#2165) |
| la permission-policy qui refuse le script en amont | décision opérateur, voisine du point précédent |
| le câblage CI de `make test-measure-webhook-turn-cost` | **déjà posé hors boucle** par `8ac93d62` (mika#2752), `ci.yml` l. 111-112. Rien à faire, et surtout **aucune entrée dans `scripts/suite-wiring.tsv`** : la comparaison y est bidirectionnelle depuis mika#2761, une entrée pour une suite que la CI atteint fait rougir `check-suite-wiring` |
| le fichier de tours (`turns-*.jsonl`, 244 lignes) | le ticket ne le fournit pas et il se régénère à l'identique par la commande du § 3 ; seul l'agrégat se commite |
| la renotification opérateur à chaque retraitement DECISION-CORE | nommée dans #2679, candidate à un ticket à part (ticket, § *Hors périmètre*) |

---

## Phases

### Phase 1 — l'artefact : l'agrégat, octet pour octet

Créer `docs/eval/mika-2680/summary-2026-10-19T0602Z-1630Z.json` avec le bloc
`json` du corps du ticket, **sans retouche** : ni reformatage, ni réindentation,
ni tri de clés, ni troncature du tableau `unattributed.trace_ids` (173 entrées).
C'est le premier artefact à atterrir dans ce répertoire.

**Le corps du ticket est tronqué à 16384 o dans le prompt de dispatch.** La
queue se récupère en bande, sans redirection ni chaînage — la règle
`bash-gh-issue-read` oppose un veto aux deux (`policy allow (bash-gh-issue-read)
vetoed — command chains a tier3-dangerous or command-substitution tail onto the
allowed prefix`) :

```
gh issue view 2680 --repo senara-solutions/mika --json body --jq '.body | .[16000:]'
```

Écrire le fichier avec l'outil `Write`, pas par redirection de shell.

Contrôle : `jq . docs/eval/mika-2680/summary-2026-10-19T0602Z-1630Z.json` sort en
0, et les sept invariants de l'AC1 se relisent dessus.

### Phase 2 — la lecture : §§ 4 à 6 appliqués à la cellule (g)

Aucun fichier produit par cette phase : c'est la lecture que les décisions 1 à 3
portent, et son résultat s'écrit en phase 3 (document durable) et dans le corps
de PR (ce que le vol 6 demande nommément).

Le § 6 d'abord, parce qu'il conditionne les autres : `exit 0` est attesté par
l'agrégat lui-même — `paid_turns: 244` et `unparseable_lines: 0`. Le § 5
ensuite : `window_precedes_field_epoch: false`, et aucun tour de la fenêtre n'est
à `prefilter_enabled: false` (aucun seau de l'agrégat n'en porte). **Les deux
haltes sont levées** : le chiffre est un résultat.

Le § 4 enfin, aux deux moitiés et au grain du vol 5 : la moitié payée par
`by_agent_trigger_release`, la moitié écartée par `skipped_by_agent_class`. Ne
pas additionner les deux moitiés, ne pas prêter de tokens à un tour écarté.

### Phase 3 — les documents, là où le prochain lecteur les trouvera

Toutes les retouches sont dans `docs/eval/mika-2680/README.md`.

1. **L'en-tête (l. 3-8).** Il annonce un répertoire qui *« accueillera son
   artefact »* et un vol 3 qui *« exécute la mesure »*. Les deux sont périmés :
   l'artefact est là, et c'est l'orchestrateur qui a pris la mesure, hors boucle.
   Réécrire au présent de ce qui est, en nommant le fichier commité.
2. **Le § 2 bis.** Il explique pourquoi un pilote dispatché ne peut pas prendre
   la mesure. Ce raisonnement reste vrai et se garde tel quel ; y ajouter un
   paragraphe de clôture : la mesure 3 **a été prise**, le 2026-10-19 par
   l'orchestrateur hors bac à sable, et l'artefact est au § 9. Le § 2 bis cesse
   d'être une impasse pour devenir un historique.
3. **Le titre du § 9** — « après les vols 3 à 5 » devient « après les vols 3
   à 6 », et sa colonne « état après les vols 3 à 5 » suit.
4. **La ligne (g) du tableau (l. 263).** De **« bloquée par la mesure »** à
   **« tranchée — chiffrée (plafond entier, soustraction de (b) vide) et sa
   prémisse réfutée ; successeur (g′) posé à l'opérateur »**. Colonne « ce qui
   décide de cet état » : *« la mesure 3 a donné le chiffre ; le code servi,
   relu, a réfuté la question »*.
5. **Le bloc (g) (l. 307-338).** Ses trois paragraphes décrivent une classe en
   attente de sa condition. La condition est remplie. Les réécrire en un bloc
   qui porte les trois décisions : la soustraction vide et sa cause structurelle
   (décision 1), le consommateur nommé (décision 2), ce que l'agrégat ne sépare
   pas et pourquoi c'est la halte (décision 3), puis le successeur (g′) borné
   (décision 5). **Garder** l'historique des vols 4 et 5 — ils ont livré
   l'opérande et le grain, et le dire reste exact.
6. **La phrase d'intro du § 9** (l. 256) dit que *« les quatre raisons ne sont
   pas la même raison »*. Elle reste vraie et gagne en force : (e) implémentée,
   (f) tranchée par le code seul, (g) **chiffrée puis réfutée — le seul cas où la
   mesure a parlé et n'a pas suffi**, (h) posée sans qu'aucune mesure ne la
   tranche. Ajuster pour nommer ce quatrième motif.

Le corps de PR porte la réfutation en propre : le chiffre, les deux termes, la
citation de la ligne d'agrégat, et le renvoi à (g′). Il s'écrit dans
`pr-body.md` à la racine du worktree, passé en `--body-file pr-body.md`, puis
supprimé par un simple `rm pr-body.md` (mika#2211).

---

## Definition of Done

1. `docs/eval/mika-2680/summary-2026-10-19T0602Z-1630Z.json` existe, est du JSON
   valide, et son contenu est celui du bloc du corps du ticket sans retouche.
2. Le tableau `unattributed.trace_ids` y porte ses **173** entrées, non tronqué.
3. `docs/eval/mika-2680/README.md` § 9 ne contient plus la chaîne « bloquée par
   la mesure » pour (g).
4. La ligne (g) du tableau du § 9 porte « tranchée », nomme la soustraction vide
   et renvoie au successeur (g′).
5. Le bloc (g) du § 9 cite les trois sites de code qui portent la réfutation :
   `webhook_prefilter.rs:815-842`, `ci_success_handler.rs:209-213` et
   `ci_success_handler.rs:628`.
6. Le § 2 bis dit que la mesure 3 a été prise, par qui, quand, et où est son
   artefact.
7. L'en-tête du README ne promet plus un artefact à venir ni un vol 3 qui
   mesurerait.
8. **Aucun filtre de classe (g) n'est écrit** : `git diff --stat origin/main` ne
   touche aucun fichier sous `crates/`, `scripts/`, `skills/` ou `.github/`.
9. Le corps de PR porte le chiffre (36 tours, 10 687 327 tokens, 16,31 %), les
   deux termes de la réfutation, et le renvoi à (g′).
10. Les quatre hors-périmètre de la décision 6 sont nommés dans le corps de PR.

---

## Acceptance criteria

**AC1 — l'agrégat est celui de l'opérateur, et sept invariants le pincent.**
`jq` sur le fichier commité rend exactement : `paid_turns` = 244,
`paid_turns_without_usage` = 0, `unparseable_lines` = 0,
`window_precedes_field_epoch` = `false`, `totals.input_tokens` = 65507602,
`field_epoch` = `"2026-10-16T13:02:24.914112Z"`, et
`(.unattributed.trace_ids | length)` = 173.

**AC2 — la cellule (g) s'y lit telle que la réfutation la cite.** La ligne de
`by_agent_trigger_release` où `agent_id == "mika-qa"` et
`trigger == "check_suite_pr_branch"` est unique, porte
`release == "no_decision_attested"`, `turns == 36`, `input_tokens == 10687327`,
`llm_calls == 202`.

**AC3 — la soustraction de (b) est vide, et l'absence est vérifiée comme
absence.** Aucune ligne de `by_agent_trigger_release` ne satisfait
`agent_id == "mika-qa" and trigger == "check_suite_pr_branch" and release == "head_not_previously_decided"` ;
et `skipped_by_agent_class` ne porte `green_check_suite_head_decided` que pour
`mika-dev`, avec `skipped == 1`.

**AC4 — aucun filtre, et c'est structurellement vérifiable.**
`git diff --name-only origin/main` ne rend que des chemins sous `docs/`.
`webhook_prefilter.rs` est inchangé — en particulier, l'énumération
`PrefilterClass` ne gagne aucune variante et `cas_de_relache` reste exhaustive
sans retouche.

**AC5 — le README ne promet plus, il constate.** `grep -c "bloquée par la
mesure" docs/eval/mika-2680/README.md` rend 0. `grep -n "summary-2026-10-19T0602Z-1630Z.json"`
rend au moins une occurrence. Les chaînes `accueillera` et « le vol 3 exécute la
mesure » ont disparu de l'en-tête.

**AC6 — la réfutation est citable, pas seulement affirmée.** Le bloc (g) du § 9
porte les trois références de la DoD n°5, et chacune se vérifie sur l'arbre :
`decide_head_decided` rend `NoDecisionAttested` sans attestation, la porte rend
`Passthrough { enrichment: None }` en nommant `qa-review-webhook-success`, et
l'attestation s'écrit après la porte.

**AC7 — les deux haltes du § 5 sont levées et le document le dit.** Le README
nomme `window_precedes_field_epoch: false` et l'absence de tour à
`prefilter_enabled: false` comme la raison pour laquelle ce chiffre est un
résultat et non une apparence.

**AC8 — la garde de pipeline passe, ou son refus est rapporté verbatim.** Le diff
ne touchant aucun Rust, `cargo test` n'est pas la porte de ce vol ; la porte est
`scripts/verify-pipeline.sh origin/main` (son contrôle U2 exige la section
`## Acceptance criteria` dans le plan cité). Si la permission-policy du bac à
sable refuse le script, **recopier le refus mot pour mot dans le corps de PR**
plutôt que d'annoncer un passage : ne jamais affirmer un état que rien n'a rendu.

**AC9 — le successeur est posé, pas suggéré.** Le corps de PR et le § 9 nomment
(g′) avec son prédicat, ses trois arbitrages opérateur (appels `gh` que
mika#2260 avait retirés, garde de prompt descendant en Rust, posture fail-open
non mesurée) et le fait que sa taille reste inconnue de l'agrégat.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| — aucun | 0 |

Total estimé : 0 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Volume de documentation, hors total par convention : ~235 lignes pour l'agrégat
(transcription à contenu fixé par le ticket, aucune décision à y prendre) et
~95 lignes touchées dans le README § 9 et son en-tête. Soit ~330 lignes de diff,
dont une seule — le § 9 — est rédigée.

**Pourquoi le total est zéro, et pourquoi c'est un résultat.** Le point 2 du
vol 6 porte deux branches, et la mesure a choisi la seconde : *« Si elle ne la
confirme pas, il l'écrit dans le corps de la PR, avec la raison tirée de
l'agrégat, et **n'écrit aucun filtre**. »* Un total de zéro ligne de code est
donc l'exécution de la consigne, pas son abandon — et la décision 4 dit en outre
pourquoi même le gel de test que le vol 4 avait écrit pour (f) n'est pas
reproductible ici : `DECISION_CORE_HOLD_TOOL` est `pub(crate)` et le seul
harnais multi-agent tourne sans jeton, si bien que l'assertion serait nulle pour
les deux agents.

**Pourquoi un seul vol.** Le volume est à 0 % de la borne par vol côté code et à
~77 % de celle-ci même en comptant la documentation au même tarif que du code.
Découper serait découper une transcription et une section de document, c'est-à-dire
payer deux pipelines pour un artefact dont le contenu est déjà figé.

---

## Fichiers touchés

| fichier | nature |
|---|---|
| `docs/eval/mika-2680/summary-2026-10-19T0602Z-1630Z.json` | **créé** — l'agrégat de la mesure 3, octet pour octet |
| `docs/eval/mika-2680/README.md` | modifié — en-tête, § 2 bis (clôture), § 9 (titre, ligne (g), bloc (g), phrase d'intro) |
| `docs/plans/2026-10-19-007-feat-2680-cellule-g-chiffree-et-refutee-plan.md` | ce plan |
| `pr-body.md` | transitoire à la racine du worktree, supprimé après `gh pr create` (mika#2211) |

Aucun fichier sous `crates/`, `scripts/`, `skills/` ou `.github/`.

---

## Renvois explicites

- **Classe (h)** — reste une question (protocole § 7). Aucune mesure ne la
  tranche ; elle est posée à l'opérateur.
- **Résidu de (f)** — une décision AC2 de mika#2675 plus trois chemins qui ne
  sont pas des filtres. Ticket voisin de #2679.
- **Bind `bwrap` de `/var/log/mika` et permission-policy** — décisions
  opérateur. Le § 2 bis du protocole garde le raisonnement intact : la mesure ne
  justifie pas d'élargir le bind.
- **Classe (g′)** — le successeur décrit en décision 5. Il demande un champ que
  le registre ne porte pas (ce que le tour a fait), donc un vol d'instrument
  avant tout vol de filtre, et trois arbitrages qui ne sont pas à la main d'un
  pilote.
