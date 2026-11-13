# mika#2948 — le callback de build inscrit la tête qu'il vient de juger

/ Ticket : senara-solutions/mika#2948
/ Branche : `fix/2948/qa-review-build-callback-le-callback`
/ Label de plafond : `loop-substrate`

## Le défaut, en une phrase

Le tour callback de build poste son verdict et n'inscrit pas la tête. Le filtre
(g′) de mika#2917 lit un registre vide, sort en `release=reviewed_head_absent`
et paie le tour — pour mika-qa comme pour mika-dev. Mesuré le 2026-10-28 :
**10 callbacks ont posté un `pr review` avec succès depuis le 2026-10-27 09:20Z,
1 seul a appelé `record_reviewed_head`.** La consigne est pourtant déployée
(`qa-review-build-callback/system_prompt.md:122-136`) ; le modèle ne l'exécute
pas. mika#2680 attend ce correctif avant toute fermeture.

---

## La décision de forme : **(b)**, et pourquoi

Le ticket délègue au groom l'arbitrage entre **(a)** — le moteur inscrit
`verdict_posted` lui-même — et **(b)** — le moteur reprend le tour une fois.
**Ce plan retient (b).** Trois raisons, dans l'ordre où elles mordent.

### 1. Le moteur ne possède pas la tête, et (a) n'est donc pas ce qu'elle a l'air d'être

Le ticket formule (a) comme un petit pas : « il le détecte déjà, puisque
`pr_review_posted_in_turn` est importé dans `qa_reviewed_head.rs:103` ». Ce que
le moteur détecte est l'**événement** « un `pr review` a réussi ». La clé du
registre est `head_key(repo, pr, head_sha)`, et le `head_sha` n'est atteignable
par aucun chemin moteur au tour callback. Quatre vérifications indépendantes :

1. `PrTarget` (`server/deadline_verdict.rs:209-238`) porte `{repo, pr_number}`
   et rien d'autre ; `to_metadata_value` rend `"{repo}#{pr}"`.
2. `resolve_qa_review_pr_target` (`skills/executor.rs:5540-5709`) dérive du
   `parse_pr_target(originating_message)`, et la ligne de fil posée par la
   gateway — `[GitHub] PR {action}: {repo}#{n} — {title} (branch: {branch})` —
   porte une **branche**, jamais un SHA.
3. `qa_pr_view`, seule source de `headRefOid`, a tourné au tour **précédent**
   (le tour de revue webhook). `all_tool_summaries` est par tour, et le prompt
   du callback interdit explicitement de le relancer
   (`system_prompt.md:132`).
4. Une requête `gh` fraîche est refusée par doctrine, et l'argument n'est pas le
   coût : `qa_review_submission.rs:232-239` — *« une requête à l'instant de
   l'émission rend le head maintenant, pas celui que le tour a revu »*.

La forme honnête de (a) est donc : **transporter la tête** depuis l'instant où
elle a été réellement observée. Cela veut dire élargir `PrTarget`, élargir la
valeur de metadata `qa_review_pr_target`, faire remonter le `headRefOid` du
`qa_pr_view` jusqu'au site de dispatch du build, poser un site d'écriture
moteur, **et** amender la garde SOLE WRITER. Ce n'est pas le petit pas que le
ticket suppose ; c'est le gros du travail.

### 2. Transporter la tête est **moins sûr** sur l'axe que ce module a choisi

Le § *La perte de revue* de `qa_reviewed_head.rs` pose son asymétrie en
clair : une écriture manquée coûte **un tour payé**, une fausse ligne coûte
**une revue**. Le module est fail-safe *vers* la revue, et c'est pour cela que
l'outil refuse plutôt que de deviner.

Or un stamp posé au dispatch du build nomme la tête de ce moment-là. L'incident
fondateur de mika#2727 est précisément un déplacement de tête en cours de route
(`4a216bf7` puis `3f2c3fa4`). Si la tête bouge pendant le build, (a) inscrit
`verdict_posted` sur une tête que le verdict ne concerne pas, le pré-filtre
écarte ensuite un `check_suite` vert sur du code jamais revu, et **une revue est
perdue en silence** — avec l'allure d'une mesure (motif mika#2293). Le modèle,
lui, a le `headRefOid` dans son contexte conversationnel : il écrit la tête
qu'il a *lue*, pas celle qu'on lui a transmise.

(a) n'est donc pas seulement une entorse au principe SOLE WRITER. Sur l'axe que
ce module a explicitement choisi, elle est le mauvais côté de l'asymétrie.

### 3. (b) ne dépend pas de « la seule mémoire du modèle » — AC1 est satisfaite littéralement

Le terme d'AC1 est *« sans dépendre de la seule mémoire du modèle »*. Une garde
EndTurn n'est pas de la mémoire : c'est du code qui refuse la sortie, calcule
`pr_review_posted_in_turn ∧ ¬record_reviewed_head` et re-prompte. Le précédent
est mesuré et dans ce même flux : mika#2355 a fermé exactement cette classe pour
le **verdict** lui-même, par le même mécanisme et sous le même budget
`intent_guard_retries`.

Le coût est un pas, sur un plafond de 30 pour mika-qa
(`planning::policy::max_tool_steps_for_agent`, mika#2926) — et seulement sur les
tours qui ont oublié.

### Ce que (b) ne garantit pas — nommé, pas tu : la population résiduelle

Un tour peut traverser la garde **et** son re-prompt sans inscrire. Le plan ne
laisse pas ce cas muet : il livre l'événement opérateur
`qa_reviewed_head_unrecorded`, exact pendant de `qa_turn_ended_without_review`
(mika#2732). Sans lui, « la garde ne mord jamais » et « la garde mord et échoue
toujours » rendraient les mêmes octets (classe mika#2205).

### Conséquence sur AC3

AC3 est conditionnée — *« si (a) est retenue »*. (a) n'est pas retenue : **AC3
est sans objet, et la garde d'écrivain unique reste verte sans amendement.** Ce
plan le vérifie plutôt que de le supposer (voir § Vérification, V3), parce que
la garde porte deux prédicats distincts et qu'un seul est évident.

---

## Livrables

### L1 — `crates/mika-agent/src/qa_reviewed_head.rs` : le prédicat, son label, son complémentaire

Le module de vérité du registre, qui importe déjà `pr_review_posted_in_turn` à
la ligne 103. Rien de neuf n'y est écrit : il gagne la moitié « détection » que
le vocabulaire appelait.

- `recorded_in_turn(summaries: &[ToolCallSummary]) -> bool` — un appel réussi à
  `RECORD_REVIEWED_HEAD_TOOL` dans le tour. La constante existe déjà
  (`tools/record_reviewed_head.rs:49` l'importe d'ici) ; pas de littéral neuf.
- `record_due_but_missing(qa_verdict_due: bool, summaries) -> bool` =
  `qa_verdict_due ∧ pr_review_posted_in_turn(summaries) ∧ ¬recorded_in_turn(summaries)`.
  **`qa_verdict_due` est réutilisé, pas recalculé** : le corps de boucle le tient
  déjà (`agent_loop/mod.rs:1726`, via `qa_build_callback::qa_verdict_required`).
  Un second discriminant de « ceci est un tour callback » serait un second
  lecteur de la même grammaire — la faute que `qa_review_submission.rs:68-82`
  refuse nommément (doctrine mika#2158).
- `RECORD_REVIEWED_HEAD_LABEL: &str = "qa_reviewed_head_record"` — le budget de
  re-prompt unique, sur le modèle de `QA_VERDICT_REQUIRED_LABEL`.
- `RECORD_REVIEWED_HEAD_CORRECTION: &str` — le texte du re-prompt. Il rappelle
  les trois termes que le prompt porte déjà : le `head_sha` est le `headRefOid`
  **déjà lu au Step 1**, aucun `gh` frais, et un `record_reviewed_head` qui
  échoue ne se réessaie pas.
- `record_unmet_after_retry(due, retries, summaries) -> bool` — complémentaire
  exact (même conjonction, terme de budget inversé). Vit ici et non chez son
  consommateur pour la raison déjà écrite en toutes lettres par
  `qa_review_submission.rs:152-159` : le site « budget déjà consommé » n'existe
  pas comme branche, donc le prédicat se pose sur **plusieurs** sorties EndTurn
  et deux copies d'une conjonction à trois termes divergent.
- `TURN_ENDED_WITHOUT_RECORD_EVENT: &str = "qa_reviewed_head_unrecorded"` +
  `emit_turn_ended_without_record(...)`.

  **L'événement ne porte ni `repo` ni `pr`, et ils sont absents, pas vides.**
  Le message qui ouvre un tour callback est
  `[callback: long_running:build_mika]`, que `parse_pr_action_event` ne lit pas ;
  le stamp `QA_REVIEW_PR_TARGET_KEY` vit dans `tasks.metadata`, hors du corps de
  boucle. Les champs seraient donc des devinettes, et un `repo = ""` se relirait
  comme « cherché, pas trouvé » (motif mika#2331). La jointure se fait par
  `trace_id` et `agent_id`, qui suffisent à retrouver la session.

### L2 — `crates/mika-agent/src/agent_loop/mod.rs` : les deux armements et l'émission

Copie du motif déjà posé à deux exemplaires (mika#2355 pour le verdict,
mika#2727/#2732 pour la soumission) :

- **Armement, chemin texte non vide** — à côté de l'entrée registre de
  `QA_VERDICT_REQUIRED_LABEL` (~l. 3630).
- **Armement, chemin texte vide** — miroir, à côté du garde mika#2355 des
  sorties EndTurn nues (~l. 4776). C'est la forme que prend un tour qui n'a plus
  rien à dire, et celle que le registre d'intentions ne voit jamais.
- **Émission** sur les sorties EndTurn, via l'appel unique à
  `emit_turn_ended_without_record` (~l. 4704-4724, 4961, 5201, 5258).

**Les deux gardes sont mutuellement exclusives par construction** : mika#2355
s'arme sur `¬pr_review_posted_in_turn`, celle-ci sur
`pr_review_posted_in_turn`. Un tour ne peut pas consommer les deux budgets.
À écrire dans le commentaire du site, parce que c'est la première question d'un
relecteur.

### L3 — `skills/bundled/qa-review-build-callback/system_prompt.md` : l'ordonnancement

La mesure dit où ça casse : le callback `58de1325` a posté APPROVED au pas 14,
puis appelé `update_task_status`, `store_fact` et `update_core_memory` — mais
pas `record_reviewed_head`. L'instruction n'est pas ignorée, elle est
**distancée**. Step 5b devient immédiatement adjacent au post, avec la condition
en tête (« la condition est le succès du post, littéralement ») plutôt qu'en
queue, et la ligne qui l'ordonne **avant** toute autre action de clôture.

C'est un renfort, pas le mécanisme : le mécanisme est L2. Le prompt seul a déjà
été essayé, et c'est le ticket.

### L4 — Tests

- **Unitaires, dans `qa_reviewed_head.rs`** : le prédicat sur ses quatre
  populations (post réussi sans record → arme ; post réussi + record → n'arme
  pas ; post échoué → n'arme pas ; pas de post → n'arme pas), plus le
  complémentaire après budget consommé. Les trois dernières **sont** AC2.
- **Eval, `tests/eval/test_qa_reviewed_head_record_2948.rs`** : un tour callback
  complet sous `MockLlmProvider`. Séquence 1 poste et n'inscrit pas → la garde
  re-prompte → séquence 2 inscrit → `audit_events` porte `verdict_posted` sur
  `head_key(repo, pr, head_sha)`. **Le contrôle négatif est dans ce même test**
  comme AC2 l'exige : une variante où le `run_gh pr review` rend `success:
  false` (la ligne `POST_FAILED`) n'arme pas la garde et n'écrit aucune ligne.
- **Prompt embarqué** : Step 5b suit immédiatement Step 5, lu par
  `include_str!` depuis les octets gelés au build — l'idiome déjà en place pour
  `PROMPTS_ECRIVANTS` (`qa_reviewed_head.rs:693-720`). Une édition de l'arbre de
  travail est invisible jusqu'à `make deploy` (motif mika#2340), et ce test lit
  ce que le binaire servira.

---

## Ordre de commit

1. L1 (prédicats + label + correction + émetteur) et ses tests unitaires — se
   compile et passe seul, n'arme rien.
2. L2 (les deux armements, l'émission).
3. L3 (le prompt) et son test d'adjacence.
4. L4 eval.

Plancher : après l'étape 1, l'arbre est vert et le registre se comporte comme
avant. Après l'étape 2, la garde mord.

---

## Vérification

- **V1** — `cargo test -p mika-agent qa_reviewed_head` et
  `cargo test -p mika-agent --test eval test_qa_reviewed_head_record_2948`.
- **V2** — `cargo test -p mika-agent qa_build_callback` et
  `test_qa_build_callback_verdict_2355` : la garde sœur ne doit pas changer de
  population.
- **V3 — la garde SOLE WRITER reste verte sans amendement.**
  `mika2917_a2_un_seul_ecrivain_et_au_plus_un_lecteur` porte **deux** prédicats
  et un seul est évident. (i) `audit_writers("QA_REVIEWED_HEAD_TOOL")` : aucun
  `log_audit_event` n'est ajouté, l'ensemble reste
  `["tools/record_reviewed_head.rs"]`. (ii) Le ban de littéral recopié porte sur
  `"qa_reviewed_head"` **entre guillemets** ; le label neuf vaut
  `"qa_reviewed_head_record"` et il est déclaré dans `qa_reviewed_head.rs`, le
  seul fichier que le scan exclut. `agent_loop/mod.rs` ne cite que le chemin de
  module, sans guillemets, et le needle du scan de lecteur est la constante en
  capitales. Rien ne mord — mais cela se vérifie, cela ne se suppose pas.
- **V4** — `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`
  (L3 touche un bundle), `scripts/verify-pipeline.sh origin/main`.

### AC4 — la sonde, après `make deploy` et pas avant

Rien de L3 n'atteint un agent avant la reconstruction : `skills/bundled/` est lu
au build (motif mika#2340). Au premier callback qui approuve une PR après le
déploiement :

1. `audit_events` porte une ligne `verdict_posted` sur
   `pr:{repo}#{n}@{sha}` — et non un `stop_already_reviewed_sha` écrit après coup.
2. Le `check_suite` vert suivant sur cette même tête sort en
   `release=green_check_suite_head_already_reviewed`, **sans** tour facturé —
   donc sans ligne `webhook_turn_charged` pour cette tête.
3. Contrôle de la population résiduelle : `qa_reviewed_head_unrecorded` dans
   `~/.mika/logs/mika-spirit.log*`. Son **absence** signifie que la garde n'a
   jamais eu à échouer ; sa présence nomme les tours qui ont traversé le
   re-prompt. Les deux se lisent, aucune ne se devine.

---

## Definition of Done

- La garde est armée sur les deux chemins de sortie EndTurn d'un tour callback
  de build, et son budget de re-prompt est unique.
- `qa_reviewed_head.rs` porte le prédicat, son complémentaire et l'émetteur ; le
  moteur n'écrit toujours pas le registre.
- Step 5b du prompt est adjacent au post, et un test lit les octets gelés au
  build.
- V1 à V4 passent.
- La PR dit en toutes lettres que (b) a été retenue, que (a) a été écartée pour
  les trois raisons du § ci-dessus, et que **le principe SOLE WRITER tient**.
- Aucune ligne `Tracked in:` n'est due : il n'y a pas de suite différée.

## Acceptance criteria

- **AC1.** Après un `pr review` réussi dans un tour callback de build, le registre porte `verdict_posted` sur `head_key(repo, pr, headRefOid)`, sans dépendre de la seule mémoire du modèle.
- **AC2.** Contrôle négatif dans le même test. Un `pr review` qui échoue (la ligne `POST_FAILED`) n'inscrit rien, conformément à la règle « la condition est le succès du post » (`system_prompt.md:130`). Un tour sans `pr review` n'inscrit rien non plus.
- **AC3.** Si (a) est retenue, la doc du module et la garde d'écrivain unique (`audit_writers("QA_REVIEWED_HEAD_TOOL")`, `qa_reviewed_head.rs:610`) sont mises à jour. Elles ne doivent pas être contournées, et la PR dit explicitement qu'elle revient sur SOLE WRITER.
- **AC4.** Sonde après déploiement. Au prochain callback qui approuve une PR, le registre porte `verdict_posted` sur sa tête, et le `check_suite` vert suivant sur cette tête sort en `green_check_suite_head_already_reviewed`, sans tour facturé.

> Lecture d'AC3 par ce plan : la forme (a) n'est **pas** retenue, donc l'antécédent
> est faux et AC3 est satisfaite à vide — mais pas en silence. La garde est
> exécutée et son invariance vérifiée (V3), et la PR dit explicitement qu'elle
> **ne** revient **pas** sur SOLE WRITER, ce qui est l'information que le lecteur
> d'AC3 cherche.

## Fire-Disposition

Ce plan livre des détecteurs : deux gardes EndTurn, un test d'adjacence sur le
prompt embarqué, et les tests de population du prédicat. Option retenue :
**(a) exception nommée en allowlist — livrée vide.**

- **L'allowlist existe, elle est vide, et sa vacuité est épinglée.** C'est le
  précédent de ce module même : `PROMPT_OUTCOME_ALLOWLIST` y est livrée vide et
  tenue vide par un test frère. Aucune violation préexistante n'est connue : la
  garde s'arme sur une population future, le test d'adjacence est vert à
  l'atterrissage puisque L3 le rend vrai, et les tests de population portent sur
  des fixtures.
- **L'assertion auto-nettoyante** est la vacuité elle-même : `assert!(ALLOWLIST
  .is_empty())` accompagné du commentaire qui dit quoi faire quand elle rougit —
  router le cas par le prédicat, nommer la donnée, référencer un ticket. Pas
  « ajouter une ligne » (doctrine mika#2201, déjà écrite dans ce fichier au
  § SOLE WRITER).
- **La seule réserve candidate est nommée et écartée.** mika#2727 a dû exempter
  les tours qui lancent un `build_mika` (réserve D3, `build_dispatched_in_turn`).
  Elle ne s'applique pas ici : un tour **callback de build** n'a plus de build à
  lancer, et sa population est déjà restreinte par `qa_verdict_due`. L'écrire
  évite qu'elle soit recopiée par réflexe.
- **Ce que la garde ne fait pas** : elle ne refuse pas la sortie. Budget épuisé,
  le tour sort, et l'émission de `qa_reviewed_head_unrecorded` prend le relais.
  Un détecteur qui bloquerait un verdict déjà posté coûterait la revue qu'il
  prétend protéger.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/qa_reviewed_head.rs` (prédicats, label, correction, émetteur) | 90 |
| `crates/mika-agent/src/qa_reviewed_head.rs` (tests unitaires de population) | 75 |
| `crates/mika-agent/src/agent_loop/mod.rs` (2 armements + sites d'émission) | 70 |
| `skills/bundled/qa-review-build-callback/system_prompt.md` | 20 |
| tests (`tests/eval/test_qa_reviewed_head_record_2948.rs`) | 130 |

Total estimé : 385 lignes
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 385 passe sous les deux bornes, la
plus serrée étant celle du vol (429). Trois propriétés mettent la variance du
côté bas plutôt que du côté haut :

1. **Aucun type neuf, aucune migration, aucune table.** Le registre existe, la
   clé existe, la constante d'outil existe, le budget de re-prompt existe.
2. **La plomberie est copiée, pas inventée.** Le motif garde-EndTurn +
   complémentaire + émetteur est déjà posé à deux exemplaires dans ce crate
   (mika#2355, mika#2727/#2732), au même endroit du même fichier. L2 est de la
   transcription.
3. **Le périmètre est fermé par la décision de forme.** C'est le choix de (b)
   qui tient le volume : (a) aurait demandé d'élargir `PrTarget`, la valeur de
   metadata, `resolve_qa_review_pr_target` et la chaîne du `headRefOid` jusqu'au
   dispatch — plus l'amendement de la garde SOLE WRITER et sa doc. Cette forme-là
   aurait demandé un découpage en vols.

## Hors périmètre

- La fermeture de mika#2680 et sa mesure de comparaison, qui suivent ce
  correctif.
- Le tour webhook `qa-review` (étape 5b), qui écrit déjà `verdict_posted`
  (#2941@57529b7f, 27/10 15:37Z).
- Le transport d'un `head_sha` dans `PrTarget` / `qa_review_pr_target`. Écarté
  par la décision de forme, pas différé : la § 2 ci-dessus en fait un **risque**
  pour l'asymétrie fail-safe du module, pas une amélioration à planifier. S'il
  devait revenir un jour, ce serait sur une décision de Vincent concernant SOLE
  WRITER, pas sur une itération de ce ticket.
