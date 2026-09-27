# mika#2506 — Un déclencheur d'itération déterministe, et le flux documenté qui ne peut pas marcher

- **Ticket:** senara-solutions/mika#2506 (enfant 8 de l'umbrella #2491)
- **Branche:** `fix/2506/defect-8-manual-trigger-iterate`
- **Date:** 2026-09-26
- **Type:** fix (substrat de boucle)

---

## 0. Préalable : la condition de réveil n'est PAS remplie, et c'est le premier fait à poser

Le corps du ticket porte **DORMANT** et trois conditions de réveil. La seule
preuve versée depuis — le commentaire opérateur du 2026-09-23 — établit
l'**inverse** de la première :

> « Preuve (A) n=2 (2026-09-23). Le geste 1-étape … a de nouveau routé
> **correctement** … Résumé : « (A) iterate marche (n=2), dispatch mika#N ne
> marche pas ». **Condition de réveil inchangée**. »

La condition 1 exige un **mis-route ou un crash** du geste 1-étape après le
2026-09-23. Le compte mesuré est **zéro échec contre deux succès**, et le
commentaire dit lui-même que la condition est inchangée. La condition 2 (blocage
réel dépassant un créneau) n'est pas attestée : les deux cas ont été réparés en
un geste, PR #2504 mergée à 15:30:37Z. Reste la condition 3 — *Vincent nomme le
besoin* — qui est la lecture la plus probable du fait qu'un grooming ait été
dispatché sur ce ticket.

**Ce que ça change pour ce plan, et ce que ça ne change pas.** Le grooming est
légitime et ce plan est livré : groomer un dormeur est strictement meilleur que
de le laisser non groomé, puisque le plan est prêt le jour où la condition tire.
Mais **la pose de `ready` reste conditionnée** par le corps du ticket, et ce plan
ne la demande pas. Le corps dit « Ne PAS poser `ready` tant que la condition
n'est pas remplie » ; grooming ≠ `ready`, et rien ici ne lève cette consigne.

**Conséquence sur la justification.** Un plan qui s'appuierait sur la fragilité
de routage du geste 1-étape s'appuierait sur une population vide. Ce plan ne le
fait pas : il se justifie sur deux faits **lisibles dans le code**, indépendants
de toute statistique, établis au § 1.

---

## 1. Mesure fondatrice, et ce que la lecture du code y déplace

### 1a. Le flux 2-étapes documenté ne peut PAS marcher — chaîne complète, lisible

Le ticket dit que `dispatch mika#N` mis-route, avec n=1 (`c4494e32`, annulée). La
lecture du code établit mieux : **ce n'est pas intermittent, c'est structurel et
toujours vrai.** Quatre maillons, chacun cité :

| # | site | ce qui se passe |
|---|---|---|
| 1 | `db/migrations.rs:527-531` | `idx_tasks_manual_active_ref_url` porte `WHERE … status NOT IN ('completed','cancelled','failed','delivered')`. Une tâche **complétée** est hors de l'index unique. |
| 2 | `self-dev/system_prompt.md` Step 2 | `dispatch mika#N` fait `create_task(reference_url=<issue>)`. Le dedup ne matche rien (§1 ci-dessus) ⇒ **tâche fraîche, métadonnées vides**. |
| 3 | `skills/executor.rs:2991` | `check_task_has_open_pr` bypass 3 : `let pr_url = extract_pr_url(&task.metadata)?;` — la garde lit le `claude_pilot.pr_url` **de la tâche**. Une tâche fraîche n'en a pas ⇒ `None` ⇒ **dispatch autorisé**. |
| 4 | `dispatch-lib.sh:2708+` | Le pilote dérive la branche de l'issue, trouve le worktree, et lance `/mika` : un **implement neuf** sur une PR revue. |

**Le défaut en une phrase :** `dispatch_task_has_open_pr` est une garde sur
l'**identité de la tâche**, et le modèle mental de l'opérateur est l'**identité
de l'issue**. La règle documentée à `self-dev/system_prompt.md:88` — « If
`run_claude_pilot` returns `dispatch_task_has_open_pr`, … surface the rejection's
`pr_url` … » — décrit donc un retour que ce chemin **ne peut pas produire** dès
lors que la tâche précédente est terminale. C'est la classe mika#1971 :
*un déployeur qui suit l'instruction met la clé au seul endroit où elle ne peut
pas marcher.*

### 1b. Le geste qui marche n'a aucune garantie structurelle

`iterate on mika#N with iteration_context: …` fonctionne (n=2) et **n'existe que
dans un prompt** : c'est le modèle de mika-dev qui choisit `run_claude_pilot` et
compose `iteration_context`. Par
`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate` (mika#2120,
neuf récurrences sous enforcement de prompt contre zéro quand le fait est posé
par le code), une route qui vit dans un prompt n'a pas de garantie — et le coût
de la variante voisine est déjà écrit dans ce même prompt (`:267`) : un
`run_claude_pilot` en free-text **crash** la session, sans worktree.

**Donc la justification de ce travail n'est pas « le geste va casser » (non
mesuré) mais « la route documentée est cassée (1a) et la route qui marche n'est
pas une route, c'est une chance répétée deux fois (1b) ».**

### 1c. Ce qui n'est PAS rejouable

Les deux dispatches mesurés sont terminés, leurs worktrees fauchés, et `gh` n'est
pas authentifié dans le bac à sable de dispatch (établi à l'ouverture de ce
grooming). Aucune sonde de ce plan ne prétend rejouer le 2026-09-23 ; toutes
portent sur la **prochaine** occurrence.

---

## 2. Le remède : la voie (b), et la voie (a) est refusée avec ses raisons

Le ticket offre deux voies. **(b) est retenue, (a) est refusée**, et le refus est
argumenté plutôt que tranché par préférence.

### 2a. Pourquoi (a) — le handler moteur automatique — est refusée

1. **Le déclencheur serait un événement unique non rejouable.**
   `ci_failure_handler` tire sur `check_suite.completed(failure)`, perdable aux
   quatre endroits que la maison a déjà nommés (drop-oldest de la file bornée
   mika#1870 → 429 → disjoncteur du gateway → DLQ `dead`). Dans le cas fondateur
   la CI était **déjà rouge** quand l'opérateur a regardé : l'événement avait
   déjà été consommé sous une tâche `in_progress`. Un handler keyé là-dessus ne
   rattrape pas cette population — c'est très exactement la classe que mika#2334
   a dû fermer par un **scan périodique**, pas par un handler.
2. **Lever `task.status != "in_progress"` est un changement de POLITIQUE, pas
   d'observabilité.** Le moteur rouvrirait un travail qu'il a clos, sur toute PR
   de la flotte qui passe au rouge après approbation — y compris celles qu'un
   opérateur a garées délibérément. Blast radius sur toute la flotte, pour une
   population mesurée à **deux**, réparée deux fois en un geste.
3. **Le DoD demande un « geste déterministe ».** Un handler automatique n'est pas
   un geste. La parenthèse « (pas de routage LLM, pas de dispatch neuf) » décrit
   ce qui est cassé dans les chemins manuels d'aujourd'hui ; elle ne demande pas
   une automatisation.
4. **Suivi nommé, avec sa précondition.** Un scan périodique (forme mika#2334)
   qui rattraperait l'état PR-approuvée + CI-rouge + tâche-complétée est
   concevable. Précondition avant de l'ouvrir : une mesure montrant qu'une PR de
   cette forme **reste bloquée plus d'un créneau** — ce qui est la condition de
   réveil 2 du ticket, aujourd'hui non attestée. Ouvrir ce scan maintenant serait
   armer un filet sur une population dont on ne sait pas si elle existe.

### 2b. La voie (b), et sa forme : CLI mince → mika-spirit

`mika iterate <repo>#<N> --context <texte>` est une **commande de l'opérateur**
qui atteint mika-spirit par HTTP et n'exécute **aucune** dérivation locale.

Deux formes étaient possibles et la première est refusée :

| forme | verdict | pourquoi |
|---|---|---|
| CLI ouvre la base et spawne le pilote elle-même | **refusée** | ferait de la CLI un **second dispatcher** hors du démon ; obligerait à répliquer `SkillRegistry`, résolution du répertoire de skill, résolution du jeton ; et contredit mika#1727, qui a fait de la CLI un client mince précisément pour que l'exécution ait un seul propriétaire |
| CLI → route de mutation mika-spirit → `try_engine_dispatch` | **retenue** | spirit possède le dispatch, la CLI rend ; les routes de mutation existent déjà (`post(handlers::handle_skill_promote)`) ; et le dispatch déterministe **existe déjà** |

`mika tasks promote-deferred` ouvre bien la base directement — c'est le précédent
qui rend la première forme tentante. Il est écarté parce que son action est une
**écriture de ligne**, pas un **spawn de pilote** : il n'a ni skill à résoudre,
ni sous-processus à détacher, ni callback à faire revenir au démon.

### 2c. Le dispatch déterministe existe déjà — c'est un appelant qu'on ajoute

`verdict_handler::try_engine_dispatch` (mika#1630) fait déjà, sans le moindre
routage LLM : résolution de l'outil dans le `SkillRegistry` → résolution du
handler exec long-running → `validate_dispatch_readiness` → création de la row
callback → `mark_parent_dispatched` (estampille `fired_at`, mika#2335) →
`spawn_long_running_exec`. Deux appelants en production (`block[ac]`,
`block[ci]`), éprouvés.

**Donc ce plan n'écrit pas de machinerie de dispatch. Il ajoute un troisième
appelant et généralise la signature d'un cran** — et cette généralisation est un
gain de lecteur unique, pas un coût.

---

## 3. Le nombre qui voyage est le numéro d'ISSUE

Contrainte de conception découverte à la lecture, et elle décide la signature de
la commande.

`dispatch-lib.sh:2708` fait `gh issue view "$ISSUE_NUM" --repo …` puis
`derive-branch-name --issue "$ISSUE_NUM" --body-callout "$ISSUE_BODY"` : le
nombre passé dans `prompt: "repo#N"` est consommé comme un **numéro d'issue**, et
la branche est dérivée du callout `> - **Branch:**` du corps de l'issue.

**Donc la commande prend le numéro d'ISSUE. La PR est résolue uniquement pour
*vérifier* les préconditions et n'est JAMAIS passée en aval.** C'est cohérent
avec le geste qui marche : `iterate on mika#2503` → worktree `chore-2503`, PR
#2504 ; et avec le second cas → worktree `fix-2496`, PR #2501.

**Observation à porter, non corrigée ici.** `verdict_handler.rs:837` compose
`format!("{repo_name}#{}", event.pr_number)` — un numéro de **PR** dans le
créneau que `dispatch-lib` lit comme une **issue**. Je ne peux pas établir ce que
`gh issue view <numéro-de-PR>` rend sans un `gh` authentifié, donc c'est nommé
comme **observation avec son propre suivi**, jamais comme un défaut affirmé ni
comme quelque chose que ce ticket répare. Précondition du suivi : une mesure de
ce que rend cet appel, puis la lecture d'un log de dispatch `block[ci]` réel.

---

## 4. Requirements

- **R1** — Une commande CLI `mika iterate <repo>#<N>` déclenche une itération sur
  la branche de la PR ouverte de l'issue `<N>`, **sans routage LLM** et **sans
  dispatch neuf**, par un chemin déterministe de bout en bout.
- **R2** — Le nombre pris en entrée est un numéro d'**issue** ; la PR est résolue
  pour vérification et n'est pas transmise en aval (§ 3).
- **R3** — Les préconditions sont **vérifiées et nommées** ; tout signal illisible
  **refuse** (fail-closed), avec un motif appartenant à un vocabulaire fermé.
- **R4** — La commande **refuse et rapporte** quand le créneau d'exec est occupé ;
  elle ne diffère pas silencieusement.
- **R5** — La tâche créée porte `dispatcher_source = 'operator'` et **pré-estampe**
  l'URL de la PR qu'elle itère.
- **R6** — Le dispatch déterministe a **un seul site de définition**, avec trois
  appelants, tenu par un scan de source.
- **R7** — Le prompt qui documente la route cassée de 1a cesse de la prescrire et
  nomme le geste qui marche. La moitié prompt est l'**intention** ; la moitié
  structurelle est R1.
- **R8** — Aucun changement de comportement pour les deux appelants existants de
  `try_engine_dispatch` (diff zéro à l'observable).
- **R9** — Une surface opérateur dit, pour chaque invocation, ce qui a été
  dispatché ou refusé et pourquoi.

---

## 5. Design

### U1 — Généraliser `try_engine_dispatch` sans toucher ses appelants

`try_engine_dispatch` ne lit de `&PrReviewEvent` que `event.repo` et
`event.pr_number` (lignes 834, 837) plus deux champs de journal (965-966).

```
try_engine_dispatch_for(db, skills, task_id, github_token,
                        repo: &str, number: u64,   // ← remplace &PrReviewEvent
                        target_skill, target_tool, iteration_context,
                        session_id, trace_id) -> EngineDispatchResult
```

`try_engine_dispatch(event, …)` devient un **adaptateur mince** qui appelle la
forme ci-dessus avec `(&event.repo, event.pr_number)`. Les deux appelants
existants voient un **diff zéro** (R8) : c'est ce qui borne le blast radius d'une
retouche à une fonction partagée par deux handlers vivants.

### U2 — Route de mutation `POST /api/v1/agents/{id}/iterate`

Voisine de `/agents/{id}/budget` pour la forme du chemin, et de
`post(handlers::handle_skill_promote)` pour l'authentification : jeton
`MIKA_INTERNAL_TOKEN` (les endpoints de mutation n'acceptent que celui-là).

Corps : `{ "repo": "<name>", "issue": <u64>, "iteration_context": "<texte>" }`.
Réponse : `{ "outcome": "<…>", "task_id": …, "callback_task_id": …, "pr_url": …,
"refusal_reason": … }`.

Elle lit l'agent dans la carte des agents **résolus**, comme
`handle_agent_budget` : un **404** signifie « ce serveur n'a pas résolu cet
agent », jamais un défaut — passer par `resolve_agent` construirait un agent
(base, skills, task engine, KG) en effet de bord d'une mutation, ce que la route
budget refuse déjà nommément.

### U3 — Les cinq refus, dans cet ordre, et l'ordre est un livrable

`server::iterate_dispatch` compose la décision. Chaque terme **nomme** son refus ;
un signal illisible refuse (R3).

| # | motif | refuse quand | pourquoi à ce rang |
|---|---|---|---|
| 1 | `missing_context` | `iteration_context` absent ou vide | le seul refus **gratuit** : aucun appel réseau, aucune lecture de base. Et c'est la faute la plus probable de l'opérateur — la voie free-text **crash** (`dispatch-lib.sh:3141`, `self-dev:267`) |
| 2 | `issue_unresolvable` | l'issue n'existe pas, est fermée, ou `gh` échoue | avant de chercher une PR, établir qu'il y a une issue. Une issue fermée est le cas mika#988, déjà traité en aval par un auto-skip : le refuser ici évite un dispatch qui se saborde |
| 3 | `no_open_pr` / `ambiguous_pr` | zéro PR ouverte sur la branche dérivée / plus d'une | **zéro** = il n'y a rien à itérer, un dispatch serait un implement neuf, c'est-à-dire le défaut 1a réintroduit. **Plus d'une** = la cible n'est pas décidable, et choisir serait deviner |
| 4 | `pr_not_iterable` | PR en brouillon, ou `CONFLICTING` | un brouillon a sa propre voie (`wip_rescue` → `gh pr ready`) ; une PR en conflit relève de `resolve-pr-conflicts`, dont le geste est différent |
| 5 | `slot_busy` | `validate_dispatch_readiness` rend `global_dispatch_active` | dernier, parce que c'est le seul refus **transitoire** : le rapporter avant les quatre autres ferait réessayer l'opérateur sur une commande de toute façon mal formée |

**Sans cet ordre, une faute de contexte se rapporterait « pas de PR ouverte » :
vrai, et strictement moins utile** — c'est un refus qui envoie l'opérateur
chercher une PR au lieu de corriger son invocation. Même motif, même formulation
que la garde `cwd-guard` de mika#2536.

**Le sens du fail-closed, et il ne se transporte pas.** L'action est une écriture
sur une branche portant une PR revue : un refus à tort coûte une invocation —
visible, rattrapable, bornée ; un passage à tort peut ré-écrire une
implémentation revue. C'est l'arbitrage de mika#2520, et **l'inverse** de celui
du faucheur mika#2420 (où un signal illisible *conserve*), parce que là-bas
l'action détruisait du travail et ici l'action *est* l'écriture. L'arbitrage est
local.

### U4 — Ce qui n'est PAS une précondition, et c'est une décision

**« QA-approuvée » et « CI rouge » ne sont pas vérifiées.** Ce sont
l'**occasion** de la commande, pas ses conditions. Trois raisons :

1. Les exiger rendrait la commande **inutilisable** pour les cas adjacents
   légitimes — CI rouge sans approbation, PR approuvée qu'un opérateur veut
   simplement faire itérer, PR dont la CI n'a jamais tourné (le cas que le
   fan-out mika#1711 laisse déjà passer).
2. Elles coûtent deux allers-retours GitHub pour trancher une question que
   l'opérateur a déjà tranchée **en tapant la commande**.
3. Elles feraient de la commande un juge de l'état de la revue, c'est-à-dire un
   second lecteur de verdict à côté de `server::verdict` — la duplication que
   mika#2237 a dû refuser explicitement.

**Ce que la commande vérifie est ce qu'elle a besoin de savoir pour ne pas casser
quelque chose** ; ce qu'elle ne vérifie pas est ce que l'opérateur a décidé.

### U5 — L'identité de la tâche, et pourquoi la tâche complétée ne peut pas servir

La tâche d'origine est **complétée**, et `completed` est terminal : la machine à
états refuse toute transition, et `validate_dispatch_readiness` check (1) exige
`pending`/`in_progress`. **On ne peut donc pas dispatcher sous elle.**

La commande crée une tâche `manual` / `source = 'self_dev'` / `type = 'issue'`
portant :

- `reference_url` = l'URL de l'**issue** — ce qui la fait entrer dans l'index de
  dedup actif, donc un second `mika iterate` pendant que le premier tourne
  collisionne au lieu d'ouvrir un second pilote ;
- `dispatcher_source = 'operator'` — ce qui lui donne la **priorité opérateur** de
  `promote_pending_deferred_if_idle`, qui se retire quand l'opérateur a du
  `pending` dans la classe. Un opérateur qui demande une itération ne doit pas
  être affamé derrière les wrappers de la boucle ;
- `metadata.claude_pilot.pr_url` **pré-estampé** — la PR sur laquelle elle itère.

**Le pré-estampage est le point non évident.** Il rend la tâche
auto-descriptive **dès sa création** : `mika tasks get` dit sur quelle PR elle
itère, sans attendre que la ligne `PR:` du callback revienne. C'est exactement la
corrélation que l'opérateur a dû faire à la main le 2026-09-23. Il arme aussi
déterministe le chemin du *parent-completer* (mika#1162, prédicat
`pr_url IS NOT NULL`) plutôt que de faire dépendre la résolution de la tâche de la
découverte de la PR par `gh pr list --head` en aval.

### U6 — `slot_busy` refuse, il ne diffère pas — divergence délibérée

`try_engine_dispatch` rend `Deferred` sur créneau occupé et enregistre un wrapper.
**Pour un geste d'opérateur, ce bras est traité différemment : on refuse en
nommant le porteur.**

Raison : un geste différé part quelques minutes plus tard, sans personne qui
regarde, alors que l'opérateur a tapé une commande en attendant une réponse. Et
l'échappatoire existe déjà et se nomme : `mika tasks promote-deferred <class>
[--override]`. Le refus la cite.

**Aucune ligne de `try_engine_dispatch` ne change pour ça** : la fonction rend un
enum, le nouvel appelant traite `Deferred` à sa façon. C'est ce qui garde R8 vrai.

### U7 — Le prompt cesse de prescrire l'inaccessible

`self-dev/system_prompt.md:88` décrit un retour que ce chemin ne produit pas
(§ 1a). La règle est réécrite pour : (i) nommer le geste déterministe, (ii) dire
que `dispatch <repo>#<N>` sur une issue portant une PR ouverte dispatche un
implement **neuf** quand la tâche précédente est terminale, (iii) garder la
description de la garde pour le cas où elle tire réellement (tâche **active** qui
porte déjà un `pr_url`).

Par `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`, **cette
moitié ne tient pas seule** : le prompt exprime l'intention, la commande tient le
fait. Elle est livrée parce qu'un prescripteur qui prescrit une route morte
fabrique la prochaine occurrence, pas parce qu'elle protège quoi que ce soit.

### U8 — Un seul site de définition, tenu par un scan

`try_engine_dispatch_for` est le **seul** site qui compose un dispatch pilote
déterministe côté moteur, et il a **trois** appelants. Un quatrième site écrit à
la main ne rendrait **aucune décision fausse le jour où il est écrit** : il
dispatcherait, tous les tests resteraient verts, et il divergerait plus tard en
silence — sur `mark_parent_dispatched` (le défaut que mika#2335 a dû fermer après
trois copies), sur le bras `Deferred`, ou sur l'estampille `fired_at`. C'est la
classe que seul un scan de source voit.

---

## Fire-Disposition

Ce plan livre des détecteurs (un scan de source + un scan de nom-unique). La
disposition est **(a) exception nommée en allowlist — livrée VIDE**, doctrine
mika#2201 : *on déclare, on n'allowliste pas.*

**Détecteur 1 — `mika2506_le_dispatch_deterministe_a_un_seul_site`**
(scan de source, `crates/mika-agent/src/`).

- *Population* : les compositions de dispatch pilote moteur, reconnues par la
  conjonction `create_task` d'une row callback **et** `spawn_long_running_exec`
  dans la même fonction.
- *Allowlist* : `ENGINE_DISPATCH_SITES_ALLOWED`, **livrée vide et épinglée vide**
  par un test frère (`…_lallowlist_est_vide`). Une allowlist née vide est un
  tiroir où déposer la prochaine infraction (mika#2323).
- *Résolution quand il tire* : **on route le site vers `try_engine_dispatch_for`**,
  on n'ajoute pas de ligne.
- *Anti-vacuité* : **cardinalité assertée à 2** (le site moteur + celui de
  `skills::executor::execute_long_running`, qui est l'autre chemin légitime et
  n'est pas un dispatch *moteur*). Sans ce terme, un prédicat devenu trop étroit
  passerait en ne regardant rien — classe mika#2205, et c'est la seule forme
  d'échec qu'aucune fixture ne voit.
- *Contrôle négatif de bonne foi* : `…_rougit_sur_un_troisieme_site` plante un
  troisième site et vérifie que le scan tire — sans quoi « le scan compte les
  sites » est indistinguable de « le scan ne regarde rien ».
- *Contrôle de bonne foi positif* : le scan ne doit **pas** accuser un commentaire
  ou de la prose citant ces deux symboles (les lignes de commentaire sont
  retirées avant analyse — motif mika#2536, mesuré : la prose *à propos* d'un
  signal n'est pas une occurrence du signal, leçon du Signal S / mika#2050).

**Détecteur 2 — `mika2506_le_nom_daudit_a_un_seul_ecrivain`** : SOLE WRITER de
`operator_iterate_dispatch`, même disposition, allowlist vide, **plus une
assertion auto-nettoyante** (le scan rougit si le nom n'est écrit **nulle part**,
parce qu'un scan visant un nom mort se lit exactement comme un arbre propre).

**Aucun détecteur n'est livré désarmé** : les deux sont neufs, leur population
est connue à la ligne, et il n'y a donc **aucune violation existante** à excepter
— ce qui est précisément la condition pour que l'option (a) soit servie avec une
allowlist vide plutôt que l'option (b).

---

## 6. Verification Contract

### V1 — Le vocabulaire de refus est un format de fil

`ALL_ITERATE_REFUSAL_REASONS`, un site de définition, épinglé par test. Les six
valeurs (`missing_context`, `issue_unresolvable`, `no_open_pr`, `ambiguous_pr`,
`pr_not_iterable`, `slot_busy`) atterrissent dans `audit_events.after_value` et
l'opérateur en fait des `GROUP BY` : deux orthographes d'un même motif
couperaient une population en deux sans le dire.

### V2 — L'ordre des refus est asserté terme par terme

Cinq tests, un par rang, **chacun vu rouge par mutation** avant d'être considéré
acquis. La forme qui compte : une invocation sans contexte **sur une issue sans
PR** doit rendre `missing_context` et non `no_open_pr` — l'assertion qui distingue
« l'ordre est celui-là » de « les deux termes sont vrais ».

### V3 — Diff zéro pour les appelants existants (R8)

La suite existante de `verdict_handler` (`block[ac]`, `block[ci]`, dispatch
moteur, bras `Deferred`, `Fallback`) passe **sans modification**. Un test
modifiée y est un signal : la généralisation a changé un comportement.

### V4 — Le chemin de production, pas seulement le prédicat

Un test au travers de la route HTTP (auth, corps, agent non résolu → 404, refus
→ motif dans la réponse, succès → `callback_task_id`). Un test de prédicat seul
ne verrait pas une route qui lit le bon champ et n'appelle jamais le décideur.

### V5 — Le nombre qui voyage est le numéro d'issue (R2)

Assertion sur l'`input` de dispatch effectivement composé : `prompt` porte
`<repo>#<numéro-d-issue>`, **jamais** le numéro de PR. Contrôle négatif : une
issue `#2503` dont la PR est `#2504` ne doit produire aucun `#2504` dans
`prompt`. C'est le terme que § 3 existe pour tenir.

### V6 — La tâche est auto-descriptive à la création (R5)

`dispatcher_source = 'operator'` et `metadata.claude_pilot.pr_url` présents
**avant** tout callback. Contrôle négatif : sans le pré-estampage, l'assertion
rougit.

### V7 — Ce qui n'est PAS testable ici, écrit plutôt que découvert

« L'itération pousse sur la branche existante et n'ouvre pas de seconde PR » est
exécuté par `dispatch-lib` et par claude-pilot, dans un autre processus, contre
GitHub. Le contrat **côté mika** est *l'input de dispatch porte le bon numéro
d'issue et le bon `iteration_context`*, et V5 l'atteste déterministe. La moitié
comportementale est la sonde S1.

---

## 7. Sondes post-déploiement, et leurs quatre haltes

> **Préalable, non négociable.** `skills/bundled/` est une projection du
> **binaire**, pas du checkout (mika#2340) : `cat ~/.mika/skills/.manifest-writer`
> doit porter le sha qu'on vient de bâtir. Et les deux moitiés se déploient
> ensemble — la CLI *et* mika-spirit. **Sans cette vérification, chaque sonde
> ci-dessous décrit le binaire d'hier.**

**S1 — le geste marche (première occasion réelle).** Sur une issue portant une PR
ouverte : `mika iterate mika#<N> --context "<…>"` → la PR reçoit un push, aucune
nouvelle PR n'est ouverte, l'implémentation n'est pas ré-écrite. **C'est la
vérification du DoD, et c'est un geste d'opérateur sur l'hôte réel.**

**Halte 1 — la commande refuse `no_open_pr` alors que la PR existe visiblement.**
**Ne pas élargir la recherche de PR par réflexe.** La branche dérivée ne
correspond pas à la `headRefName` de la PR : établir d'abord la dérivation
(`derive-branch-name` contre le callout `> - **Branch:**` du corps de l'issue),
qui est la cause la plus probable et dont le remède est une réparation de callout,
pas un élargissement de prédicat.

**S2 — contrôle négatif de bruit (7 jours).** Aucun refus sur une invocation
nominale. **Halte 2 —** un refus sur un cas sain coûte une invocation :
diagnostiquer le **terme** fautif, pas régler un seuil — il n'y a aucun seuil
dans ce prédicat.

**S3 — contrôle POSITIF, et il n'est pas décoratif.**
`SELECT after_value, count(*) FROM audit_events WHERE tool_name =
'operator_iterate_dispatch' GROUP BY 1;` — un `dispatched` non nul.
**Halte 3 — zéro ligne quoi qu'il arrive.** On ne peut **rien** conclure des
sondes S1/S2 : établir que le binaire servi porte le correctif (classe mika#2340)
et que la route est atteinte, **avant** toute conclusion sur le prédicat. *Une
commande qu'on n'a pas déployée se lit exactement comme une commande qui
marche* (mika#2205).

**S4 — le flux 2-étapes ne mis-route plus en silence (30 jours).** Le prompt
corrigé (U7) doit réduire le recours à `dispatch <repo>#<N>` sur une issue portant
une PR ouverte. **Halte 4 —** si un implement neuf part malgré tout sur une PR
ouverte, **c'est la limite nommée de ce travail** (§ 10), pas une régression : la
garde `dispatch_task_has_open_pr` n'a pas été élargie, et c'est le suivi du § 9
qui s'ouvre, **avec cette occurrence en précondition**.

---

## 8. Definition of Done

- Un état PR-QA-approuvée + CI-rouge + tâche-complétée déclenche une itération sur
  la branche existante **par un geste déterministe** — sans routage LLM, sans
  dispatch neuf — vérifié sur un cas : la PR reçoit un push corrigeant la CI,
  aucune nouvelle PR n'est ouverte, l'implémentation n'est pas ré-écrite (sonde
  S1).
- Les cinq refus sont nommés, ordonnés, et chacun est vu rouge par mutation (V2).
- Les deux appelants existants de `try_engine_dispatch` ont un diff zéro à
  l'observable (V3).
- Le prompt ne prescrit plus la route de 1a (U7).
- `make test`, `cargo clippy`, `cargo fmt --check` passent.
- Le scan de dispatch unique et le scan SOLE WRITER passent, allowlists vides.

---

## 9. Acceptance criteria

Le corps du ticket ne porte pas de section `## Acceptance criteria` (il porte un
`### DoD`), donc les critères ci-dessous sont **dérivés** des Requirements (§ 4)
et du Verification Contract (§ 6).

- **AC1** — `mika iterate <repo>#<N> --context <texte>` existe, atteint
  mika-spirit par HTTP, et ne compose **aucune** dérivation de branche ni
  résolution de skill dans le processus CLI.
- **AC2** — Le dispatch résultant porte `prompt = "<repo>#<numéro-d-ISSUE>"` et le
  `iteration_context` fourni ; le numéro de PR n'apparaît **nulle part** dans
  l'input de dispatch (V5).
- **AC3** — Les six motifs de refus sont un format de fil à site unique, épinglé
  par test (V1), et l'ordre des rangs 1→5 est asserté terme par terme (V2).
- **AC4** — Tout signal illisible **refuse** ; aucun ne fait entrer une invocation
  dans la population dispatchable (R3).
- **AC5** — Un créneau d'exec occupé **refuse en nommant le porteur** et cite
  `mika tasks promote-deferred` ; aucun wrapper différé n'est enregistré par ce
  chemin (R4, U6).
- **AC6** — La tâche créée porte `dispatcher_source = 'operator'` et
  `metadata.claude_pilot.pr_url` **avant** tout callback (R5, V6).
- **AC7** — `try_engine_dispatch_for` est le seul site de composition d'un
  dispatch pilote moteur, avec trois appelants, tenu par un scan à allowlist vide
  portant son anti-vacuité et son contrôle négatif (R6, § Fire-Disposition).
- **AC8** — `operator_iterate_dispatch` a un écrivain unique dans le journal et
  dans `audit_events` (R9).
- **AC9** — Les tests existants de `verdict_handler` passent **sans
  modification** (R8, V3).
- **AC10** — `self-dev/system_prompt.md` ne prescrit plus la route de 1a et nomme
  le geste déterministe (R7).

---

## 10. Ce que ce travail n'achète PAS

- **Il ne rend pas le routage LLM fiable**, et il ne le mesure pas. Le geste
  `mika ask "iterate on …"` reste ce qu'il est : deux succès et aucune garantie.
  Ce qui change est qu'il existe désormais une route **qui n'en dépend pas**.
- **Il ne répare pas le mis-route silencieux du flux 2-étapes.** La garde
  `dispatch_task_has_open_pr` reste une garde sur l'identité de la **tâche**
  (§ 1a) ; `dispatch <repo>#<N>` sur une issue portant une PR ouverte et une
  tâche terminale dispatchera encore un implement neuf. Ce qui est retiré est
  **l'instruction qui y mène**, pas la capacité. Suivi au § 11.
- **Il n'ajoute aucun déclencheur automatique** (§ 2a), donc il ne rattrape pas
  une PR que personne ne relance. *Une commande que personne ne tape est un
  silence.*
- **Il ne rejoue pas les cas du 2026-09-23** (§ 1c).
- **Aucune itération n'est possible sur une issue dont le callout `> - **Branch:**`
  est absent ou faux** : la dérivation échoue et la commande refuse. C'est le
  comportement voulu — deviner une branche est la faute que `derive-branch-name`
  existe pour ne pas commettre (mika-platform#58) — mais c'est une limite réelle,
  et c'est la Halte 1.
- **Il ne surveille rien.** Le seul instrument neuf est la ligne d'audit du § 7,
  et **son silence ne prouve rien tant que personne n'exécute S3**. Sur une
  commande tapée quelques fois par semaine, l'absence de refus peut simplement
  vouloir dire que personne ne l'a tapée.

---

## 11. Hors périmètre, délibérément

- **Le handler moteur automatique de la voie (a)** — refusé avec ses quatre
  raisons (§ 2a). **Suivi**, précondition : la condition de réveil 2 du ticket
  attestée (une PR de cette forme bloquée plus d'un créneau).
- **Élargir `dispatch_task_has_open_pr` pour tester la PR ouverte de l'ISSUE** —
  ce serait le correctif *racine* de 1a, et il est écarté ici : il ajoute un
  aller-retour GitHub sur le chemin chaud de **chaque** dispatch dev-pilot de la
  boucle, y compris les milliers de premiers dispatches où la réponse est
  toujours « pas de PR », dans une fonction qui en fait déjà plusieurs (corps
  d'issue pour les marqueurs de grooming, `blockedBy` en GraphQL). Blast radius
  sur la boucle entière pour réparer une route manuelle que ce plan remplace.
  **Suivi**, précondition : une mesure du nombre d'implements neufs réellement
  partis sur une PR ouverte (Halte 4).
- **`verdict_handler.rs:837` qui passe un numéro de PR là où `dispatch-lib` lit
  une issue** (§ 3) — observation portée, non corrigée, non affirmée comme
  défaut. **Suivi**, précondition : établir ce que rend `gh issue view
  <numéro-de-PR>`, puis lire un log de dispatch `block[ci]` réel.
- **`claude-pilot`** — hors de ce dépôt.
- **Toute vérification de l'état de revue dans la commande** (§ U4) : arbitrage
  assumé, à rouvrir seulement si une mesure montre un `mika iterate` lancé sur une
  PR qu'il aurait fallu refuser.
- **`mika iterate` sur une PR non issue de la boucle** (auteur humain) : la
  commande ne filtre pas sur l'auteur, mais aucune mesure ne demande ce cas et
  aucun terme n'y est consacré.
