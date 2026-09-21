# mika#2455 — Un `pass` QA ne peut plus affirmer ce qu'un check requis rouge contredit

- **Ticket :** senara-solutions/mika#2455
- **Type :** fix (substrat)
- **Priorité :** p2 santé-substrat
- **Date :** 2026-09-21

## Problème

Sur PR #2439 (tête `73ec3e3e`), mika-qa a rendu `pass` / APPROVED alors que deux
checks CI requis étaient rouges (`SIGPIPE grep-q Lint`, `Check`). Deuxième cas
mesuré le même jour sur PR #2461 (tête `1302b0d0`, `Check` rouge sur un test
unitaire mika-cli). n=2, deux surfaces d'échec différentes, même angle mort.

Le ticket propose un remède littéral : *« le verdict QA doit intégrer l'état
`statusCheckRollup` de la tête (`gh pr view --json statusCheckRollup`) avant
d'émettre pass »*.

La lecture du code **déplace ce remède**. Les trois mesures ci-dessous sont le
premier livrable de ce plan : elles changent où le correctif doit vivre, et
elles écartent trois implémentations qui paraissent économiques.

### M1 — Le risque nommé (« faire merger du code rouge ») est déjà fermé, structurellement

Le ticket écrit « risque de faire merger du code rouge », au conditionnel, et
ajoute « le seul filet restant = l'humain ». Ce n'est pas exact, et la
rectification est load-bearing pour le dimensionnement du correctif.

Le merge sur verdict `pass` passe par `pr_merge_with_gate`
(`skills/bundled/self-dev-webhook-qa/system_prompt.md:28` — *« On a `pass`
verdict, your FIRST output MUST be a `pr_merge_with_gate` tool call »*), qui
lit `gh pr checks <n> --repo <repo> --required --json name,state,bucket,link`
et refuse sur tout bucket `fail`/`cancel`
(`crates/mika-agent/src/tools/pr_merge_with_gate.rs:680` `classify_checks`).
C'est un `Tool` trait implementor enregistré dans `default_tools()`, donc
**non désactivable par agent** — la propriété que
`docs/solutions/architecture-patterns/ci-gate-tool-structural-backstop-for-pr-merges.md`
a été écrit pour poser, après l'incident jumeau mika#485 où une PR *avait*
mergé sur CI rouge.

Donc un APPROVED sur CI rouge **ne merge pas** : le gate renvoie `blocked` /
`ci_failure` et webhook-qa dispatche un fix
(`self-dev-webhook-qa/system_prompt.md:70`).

**Conséquence sur le périmètre.** Le défaut n'est pas une porte de merge
ouverte : c'est un **signal faux**. `pass` / APPROVED affirme une chose que la
CI contredit, ce qui trompe l'humain qui lit la PR et la discipline
orchestrateur — ce que le ticket dit lui-même dans sa dernière phrase (« la QA
ne devrait pas induire en erreur »), et ce que la mémoire
`feedback_verify_approved_commit_equals_head_before_mergeable` qu'il cite dit
déjà d'une autre moitié du même problème. Ce plan ferme le signal faux. Il
n'ajoute pas un second gate de merge, et il ne faut pas en attendre un.

### M2 — Le reviewer ne voit pas la CI **par décision datée**, pas par oubli

`qa_pr_view` strippe les champs CI à la source. Ce n'est pas un défaut de
projection : c'est le sujet de l'outil, écrit dans son en-tête
(`skills/bundled/qa-review/handlers/qa_pr_view.sh:6` — *« CI fields
(statusCheckRollup, statusChecks, mergeStateStatus, mergeable, commits) are
excluded so the reviewing LLM never sees CI data »*), doublé d'une validation
anti-injection d'option qui nomme `--json statusCheckRollup` comme l'attaque à
empêcher (`:25`), et répété comme règle d'intégrité des données à deux endroits
(`qa-review/system_prompt.md:45`, `qa-review-build-callback/system_prompt.md:24`).
La capacité est **retirée**, pas seulement interdite — et `QA_REVIEW_GH_ALLOWED`
(`builtin_handlers.rs:2240` `validate_qa_review_gh_scope`) borne le périmètre
`gh` de qa-review à `pr review`, `pr diff`, `pr list`, `issue view` et
`GET /advisories`.

Une troisième trace de la décision existe côté périmètre d'outil :
`docs/solutions/prompt-engineering/qa-review-pipeline-exempt-trailer-parity-2026-05-20.md:40`
refuse d'ajouter un champ `commits` à `qa_pr_view` précisément parce qu'il
« fuirait le statut CI via `statusCheckRollup` ».

**Pourquoi la décision tient encore.** Le rôle de qa-review est la revue de
diff et des artefacts de pipeline. Rendre la CI visible au reviewer LLM lui
donne un raccourci — approuver parce que c'est vert — qui est exactement ce que
« retirer la capacité » a fermé. Le correctif ne doit donc pas rendre la CI
lisible au modèle.

*(Note documentaire : le lien `docs/solutions/prompt-engineering/2026-04-08-remove-capability-not-prohibit-it.md`
cité en tête de `qa_pr_view.sh:12` pointe un fichier absent de l'arbre. Le
raisonnement survit dans les trois sites ci-dessus ; la référence cassée est
notée en U5.)*

### M3 — `block[ci]` existe déjà, et ce n'est pas un label inerte

`VERDICT: block[ci]` est **déjà** une ligne de trailer autorisée pour qa-review
(`skills/bundled/qa-review/skill.toml:78`, dans
`output.required_tool_arg_suffixes`). Le token demandé par le ticket est donc
émissible aujourd'hui ; il est simplement inatteignable puisque le reviewer n'a
aucun signal qui le justifierait.

Mais `block[ci]` **agit** : `handle_block_ci`
(`crates/mika-agent/src/server/verdict_handler.rs:1286`) dispatche un
claude-pilot avec un prompt CI-fix, borné par `BLOCK_CI_MAX_RETRIES = 3`
(`:58`). C'est le même raisonnement que Step 1.5 a déjà dû faire pour
`block[ac]` (mika#2157, cité dans `qa-review/system_prompt.md:122` : *« `block[ac]`
is not an inert label »*). Émettre `block[ci]` consomme donc un slot de
dispatch, et le chemin `self-dev-webhook-ci` fait déjà ce travail sur
`check_suite.completed(failure)`.

Deux protections anti-doublon existent déjà, dans les deux sens :
`self-dev-webhook-ci/system_prompt.md:13` saute si mika-qa a déjà posé un
`block[ci]`, et `self-dev-webhook-qa/system_prompt.md:140` saute si
`ci_fix_dispatched_from` est posé. Le moteur supprime aussi le dispatch
dupliqué (`verdict_handler.rs:1330`).

**Conséquence.** Le correctif ne doit pas **prescrire** `block[ci]` de force :
il doit refuser `pass`. Voir D3.

### M4 — Le remède naïf (« un gate CI sur `--approve` ») est structurellement piégé

`validate_pr_review_flag_coherence` (mika#2237,
`builtin_handlers.rs:2929`) refuse déjà un `gh pr review` dont le flag
contredit le `VERDICT:` de son propre corps — y compris la dégradation
`pass` → `--comment` quand aucun `--approve` n'a été tenté dans le tour.

Sa documentation nomme explicitement le mode de panne à éviter
(`crates/mika-agent/CLAUDE.md:542`) : *« A guard that always refused
`--comment` on `pass` would turn a real constraint into an inability to post the
review at all — the turn loops and dies. »*

Donc un gate qui refuserait **`--approve`** sur CI rouge produirait, sur un
corps `pass` :

- `--approve` → refusé par le nouveau gate ;
- `--comment` → refusé par mika#2237 (pas de tentative recevable) ;
- **aucune revue postable.** La session boucle et meurt.

La trappe de mika#2237 n'y suffit pas : elle laisserait passer un `pass` en
`--comment`, c'est-à-dire **conserverait le verdict trompeur** que ce ticket
existe pour fermer.

**Conséquence, et c'est la décision centrale de ce plan :** le gate porte sur le
**verdict**, jamais sur le flag. La sortie qu'il nomme (`block[ci]` ou
`hold[review]`, posté en `--comment`) est atteignable **quelle que soit
l'histoire du tour**, donc il n'y a pas d'interblocage. Voir D1.

### M5 — Le `pending` est le cas nominal au moment de la revue, et il borne ce que le correctif peut fermer

`route_event` (`crates/mika-gateway/src/github.rs:335`) route
`pull_request.opened | synchronize | review_requested | ready_for_review` vers
mika-qa **sans aucun terme CI**. La revue peut donc partir avant que la CI ait
conclu. Le fan-out vers mika-qa sur CI verte existe (mika#1711,
`secondary_targets`, `github.rs:366`) mais il ne couvre que `success` — sur
`failure`/`timed_out` le routage va à mika-dev seul (`:341`).

Par ailleurs `qa_review_reconcile` (mika#2334/#2347) pose un relecteur sur toute
PR ouverte non revue **sans lire la CI** : sa conjonction de six termes n'en
comporte aucun.

**Conséquence.** Un gate ne peut refuser que ce qu'il peut observer **à
l'instant de la publication**. Deux populations :

| état de la CI au moment du `pr review` | ce que le gate peut faire |
|---|---|
| terminée, un check requis `fail`/`cancel` | **refuser** — la population que ce plan ferme |
| `pending` (CI en cours) | **rien**, et refuser serait faux |

Si la CI d'un des deux cas mesurés était `pending` à l'instant du verdict, le
gate ne l'aurait pas attrapé. **C'est une mesure à faire avant de conclure sur
AC2** (sonde 1), et elle n'est pas exécutable depuis ce worktree : `gh` n'y est
pas authentifié (classe mika#2201 V5c). Le plan est conçu pour être correct dans
les deux cas ; ce que la mesure décide est la **taille** de ce qu'il ferme, pas
sa justesse.

## Requirements

- **R1** — Un appel `gh pr review` dont le corps porte `VERDICT: pass` est
  refusé, avant tout sous-processus, quand au moins un check **requis** est en
  bucket `fail`/`cancel` sur la PR visée.
- **R2** — Le refus nomme la ou les sorties correctes et l'état constaté (les
  noms des checks rouges), de sorte qu'une réécriture du verdict soit
  immédiatement postable.
- **R3** — Aucun autre verdict n'est touché. `hold[review]`, `block[ac]`,
  `block[ci]`, `block[dependency]`, `block[security]`, `block[pipeline]` et un
  corps sans `VERDICT:` passent inchangés.
- **R4** — Le gate ne rend **jamais** la CI lisible au modèle autrement que par
  le corps de son refus : la **capacité** de `qa_pr_view` n'est pas touchée (sa
  liste `SAFE_FIELDS` et sa validation anti-injection d'option sont inchangées
  au caractère près), le périmètre `gh` de qa-review n'est pas élargi, aucune
  règle de prompt n'est retirée. *R4 porte sur la capacité, pas sur l'octet :
  U5 répare une référence de documentation cassée dans un commentaire de
  `qa_pr_view.sh`, ce qui n'ouvre aucune capacité — la DoD nomme la commande qui
  sépare les deux.*
- **R5** — Un signal CI illisible (pas de token, pas de repo, `gh` en échec,
  timeout, sortie non parsable) **n'est jamais un terme satisfait** : le gate
  s'abstient et le dit.
- **R6** — Un check `pending` ne refuse rien.
- **R7** — Kill-switch : le gate se désarme sans redéploiement, et son
  désarmement est dit au démarrage.
- **R8** — Le gate ne rend jamais impossible la publication d'une revue
  légitime : la sortie nommée est atteignable indépendamment de l'historique du
  tour.
- **R9** — Contrôle positif : la décision nominale (un `pass` vérifié contre une
  CI verte) est observable, pour que zéro refus soit distinguable de zéro
  regard.
- **R10** — Un seul lecteur de la classification CI. Le gate réutilise
  `run_gh_checks` / `classify_checks`, il ne réimplémente pas la notion de
  « check requis ».

## Décisions

### D1 — Le gate porte sur le VERDICT, jamais sur le flag

Établi par M4. Un gate sur `--approve` crée l'interblocage que mika#2237 nomme
comme son propre mode de panne. Un gate sur le verdict a une sortie toujours
atteignable : réécrire le corps en `block[ci]` ou `hold[review]` et poster en
`--comment`, ce qui est précisément le mapping que `required_review_flag`
attend (`evidence/guards.rs`, mika#2237) et une ligne de trailer déjà autorisée
(`qa-review/skill.toml:78`, M3).

Corollaire d'ordonnancement : le nouveau gate s'exécute **après**
`validate_pr_review_flag_coherence`. Deux raisons. (a) La chaîne de `run_gh`
est documentée « du plus local au plus engageant »
(`builtin_handlers.rs:3235`) et ce gate est le seul à faire un appel réseau —
le placer en dernier évite de le dépenser sur une revue que mika#2237 va
refuser de toute façon. (b) L'ordre rend la composition lisible : un corps
`pass` posté en `--comment` sans tentative est refusé par mika#2237 et n'atteint
jamais ce gate ; un corps `pass` posté en `--approve` le franchit et c'est ici
que la CI décide.

### D2 — Un gate moteur, zéro octet de prompt

Le correctif est entièrement dans le moteur, pre-subprocess, frère de mika#2237
et de `validate_destructive_action_grounding` (mika#1646). Trois raisons, dans
l'ordre de poids.

1. **La classe de correctif est établie par mesure dans ce dépôt.**
   `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
   (mika#2120 : neuf récurrences sous enforcement par prompt contre zéro quand
   la consigne était écrite à la main) interdit de fermer un défaut de substrat
   par une phrase de prompt. Et une règle de prompt ne peut de toute façon pas
   être appliquée par un modèle qui n'a pas le signal (M2).
2. **R4 est préservée par construction.** Le modèle ne lit pas la CI : c'est le
   moteur qui lit, et le modèle n'en reçoit que le refus. La règle « ne fetch
   pas la CI » du prompt reste vraie et n'est pas retirée.
3. **Budget de prompt.** `qa-review/skill.toml` documente que son
   `max_prompt_size` a déjà été relevé deux fois et que le gate de 95 % **panique**
   plutôt qu'il n'avertit, avec un re-déclenchement à 70 041 octets. Zéro octet
   ajouté est la seule option qui ne rouvre pas cette question.

**Conséquence assumée :** le reviewer n'est pas prévenu à l'avance. C'est le
régime de mika#2237, dont le corps de refus porte tout le message. Le refus doit
donc être auto-suffisant (R2).

### D3 — Le gate refuse `pass`, il ne prescrit pas `block[ci]`

Le ticket demande `block[ci]` « (ou hold) ». Son test négatif, lui, est plus
étroit et plus juste : *« verdict QA ≠ pass »*. C'est cette formulation qui est
implémentée.

Raison, établie par M3 : `block[ci]` dispatche un pilote CI-fix borné à trois
tentatives. Le gate n'a **aucun moyen** de savoir si la CI rouge est réparable
par un pilote — un lint rouge l'est, une infra cassée ou un flake ne l'est pas —
et forcer `block[ci]` dépenserait un slot de dispatch sur la seconde catégorie.
Même arbitrage, et même raison écrite, que le choix `hold[review]` plutôt que
`block[ac]` en Step 1.5 (mika#2157).

Le corps du refus nomme donc **les deux** sorties, avec leur conséquence, et
laisse le modèle choisir — exactement le motif de mika#2237, dont la
documentation explique pourquoi un refus ne doit pas ne nommer qu'une seule
issue (`CLAUDE.md:546`).

### D4 — Fail-OPEN sur tout signal illisible, et l'asymétrie est mesurée

L'inverse de `validate_destructive_action_grounding` (fail-closed), et pour une
raison qui se calcule :

- **Faux négatif** (laisser passer un `pass` sur CI rouge) : le signal trompeur
  subsiste — le défaut d'origine — mais le merge reste fermé par
  `pr_merge_with_gate` (M1). Coût borné, et déjà le régime actuel.
- **Faux positif** (refuser un `pass` légitime) : la revue doit être réécrite ;
  si le modèle s'obstine, la session boucle et meurt sans revue — le mode de
  panne de M4, celui que ce plan existe pour ne pas créer.

L'asymétrie penche donc vers l'ouverture. Populations qui abstiennent : `repo`
absent (le gate ne devine pas le dépôt), `ctx.github_token` absent, `gh` en
échec ou timeout, sortie non parsable, et R6 (`pending`). Chacune est **dite**
(D5), jamais silencieuse — sans quoi le gate serait inerte de la même manière
que `MIKA_LOG_PILOT_TRANSCRIPTS` désarmé rend le faucheur mika#2249 aveugle.

### D5 — Trois noms d'événements, un nom d'audit, et un contrôle positif

Noms distincts, chacun SOLE WRITER du sien (motif `phantom_aged_out` /
`phantom_sweep_spared`, mika#2156) pour que les populations restent
soustractibles :

- `qa_ci_coherence_refused` (WARN) — un `pass` refusé. **Régime attendu :
  faible mais non nul** ; chaque ligne est un verdict faux intercepté.
- `qa_ci_coherence_abstained` (WARN) — le gate n'a pas pu lire. Porte un champ
  `reason` ∈ `{no_repo, no_token, gh_failed, gh_timeout, unparseable}`.
  **Régime attendu : proche de zéro.**
- `qa_ci_coherence_gate_disabled` (INFO, une fois au démarrage, seulement
  désarmé) — parce que le silence d'un gate désarmé se lit exactement comme le
  silence d'un gate sain (mika#2205).

Audit : `tool_name = "qa_ci_coherence_guard"`,
`target_key = "pr_review:{repo}#{n}"`, `after_value` ∈
`{refused, abstained, allowed_pending, allowed_green}`. Écritures
warn-and-continue : perdre la ligne ne change jamais la décision (motif
mika#2237).

**`allowed_green` est le contrôle positif de R9, et il n'est pas décoratif.**
Sans lui, « zéro refus » ne distingue pas un parc sain d'un gate inerte —
exactement la panne que mika#2205 a dû nommer. Le volume est borné à une ligne
par `pass` posté (quelques unités par jour), très loin du churn que la doctrine
mika#2131 borne.

### D6 — Un seul lecteur de la classification, réutilisé

`run_gh_checks` (`pr_merge_with_gate.rs:733`, `pub(crate)`, déjà `--required`)
et `classify_checks` (`:680`, fonction pure, sept tests) sont réutilisés tels
quels. Réimplémenter « check requis » donnerait deux définitions divergentes de
ce qui bloque un merge, sur les deux extrémités du même contrat — la classe que
`grooming_marker` (mika#2158) a dû fermer après des mois de divergence
silencieuse.

Corollaire : le gate n'appelle **pas** `gh pr view --json statusCheckRollup`
comme le ticket le suggère. `gh pr checks --required` est le lecteur qui porte
déjà la notion de *requis*, et `statusCheckRollup` ne la porte pas — il rend
tous les checks, requis ou non, ce qui ferait refuser un `pass` sur un check
optionnel rouge.

### D7 — La tête lue est la tête courante, et la limite est nommée

`gh pr checks` porte sur la tête courante de la PR. Si la tête a bougé pendant
la revue, le gate peut refuser un `pass` au vu de checks d'un commit que le
reviewer n'a pas lu. Ce `pass` est de toute façon périmé — le gate de merge le
refuse pour tête obsolète, et
`feedback_verify_approved_commit_equals_head_before_mergeable` (cité par le
ticket) est la moitié « commit périmé » du même problème. Refuser est donc
cohérent, et c'est la direction sûre.

Comparer explicitement à `headRefOid` est écarté, **et la raison portée n'est
pas le coût de l'appel.** Le coût est nul ou presque : `headRefOid` figure déjà
dans les `SAFE_FIELDS` de `qa_pr_view.sh:34`, donc la tête que le reviewer a lue
est déjà connue du chemin — invoquer un « second appel `gh` » comme
justification serait faux, et surtout ce serait la mauvaise raison.

La vraie raison est que **la direction du refus est sûre de toute façon** : si
la tête a bougé, le `pass` porte sur un diff que la tête courante ne contient
plus, il est périmé, et le refuser est juste. Comparer les deux têtes ne
produirait donc pas une meilleure décision — elle produirait une **abstention**
là où le refus est correct, c'est-à-dire un affaiblissement du gate présenté
comme une précision.

*Conséquence pour un relecteur futur, écrite parce que c'est elle qui se perd :*
si vous voyez ici une optimisation évidente (« la tête est déjà dans la payload,
autant comparer »), vous êtes en train de rouvrir un cas que ce plan laisse
**délibérément** fermé du côté du refus. Le cas « tête bougée » n'est pas un
angle mort à combler, c'est une population que D7 range du côté sûr. Citation :
review-guide § citation-or-silence (la justification portée doit être la vraie),
et `feedback_verify_approved_commit_equals_head_before_mergeable`, qui est la
moitié « commit périmé » du même problème.

## Scope Boundaries

**Dans le périmètre**

- Un gate pre-subprocess sur `run_gh` pour `gh pr review`.
- Sa variable de désarmement, ses trois événements, ses lignes d'audit.
- Les tests (unités de prédicat + chemin de production + contrôles négatifs).
- La documentation (`crates/mika-agent/CLAUDE.md`, racine `CLAUDE.md`).

**Hors périmètre, délibérément**

- **`qa_pr_view` et les règles de prompt « ne fetch pas la CI »** — inchangés
  par R4. C'est la décision de M2, et la rendre caduque est le contraire du
  correctif.
- **Le périmètre `gh` de qa-review** (`QA_REVIEW_GH_ALLOWED`) — non élargi.
- **`pr_merge_with_gate`** — il fait son travail (M1) ; ce plan ne le double pas.
- **Le routage de `pull_request.opened`/`synchronize` vers mika-qa** (M5). Le
  retirer ferait partir la revue seulement sur CI verte et fermerait la
  population `pending` à la racine — mais il casserait `review_requested` (le
  réconciliateur mika#2334) et `ready_for_review` (mika#1822). Blast radius trop
  large pour ce ticket : **ticket de suivi**, conditionné à la sonde 1.
- **Router `check_suite.completed(failure)` vers mika-qa** — écarté, pas
  reporté : ce serait déclencher une revue *sur* une CI rouge, c'est-à-dire
  multiplier la population que ce plan refuse.
- **Révoquer une approbation déjà postée** — GitHub ne permet de la contredire
  qu'en postant une autre revue ; un `pass` publié avant que la CI ne rougisse
  reste visible. Ce plan empêche le verdict faux d'être **émis**, il ne réécrit
  pas l'historique.
- **La cause des CI rouges** de #2439 et #2461 (un test `mika-cli`, un lint
  SIGPIPE). Ce plan rend le verdict honnête, il ne répare pas la CI.

## Implementation Units

### U1 — Le lecteur CI et son plafond de temps (R1, R5, R6, R10, D6)

Dans `crates/mika-agent/src/evidence/guards.rs`, section mika#2455 :

- Une constante de nom d'audit `QA_CI_COHERENCE_AUDIT_TOOL = "qa_ci_coherence_guard"`.
- Un `enum CiCoherenceOutcome { Refused { failing: Vec<String> }, AllowedGreen, AllowedPending, Abstained { reason: &'static str } }`.
- Une **fonction pure** `classify_ci_coherence(checks: &[GhCheck]) -> CiCoherenceOutcome`
  qui délègue à `classify_checks` et n'ajoute que l'extraction des noms rouges
  pour R2. Aucune notion de « requis » réimplémentée (D6).

L'appel réseau est enveloppé d'un `tokio::time::timeout` court (10 s, constante
nommée, très inférieure au `timeout_secs = 30` déclaré par `qa-review`) ; le
dépassement rend `Abstained { reason: "gh_timeout" }`. Vérifier à
l'implémentation si `run_gh_checks` porte déjà son propre plafond — si oui, ne
pas en empiler un second, et documenter lequel borne.

### U2 — Le gate et son branchement (R1, R2, R3, R7, R8, D1)

Dans `builtin_handlers.rs`, `async fn validate_qa_ci_coherence(args, repo, ctx)`,
sur le modèle exact de `validate_pr_review_flag_coherence` :

1. Reconnaissance fail-open, dans cet ordre : kill-switch désarmé → `Ok(())` ;
   `extract_pr_review_body(args)` absent → `Ok(())` ;
   `parse_verdict(&body)` ≠ `Verdict::Pass` → `Ok(())` (R3) ;
   `pr_review_target(args)` absent → abstention ; `repo` absent → abstention ;
   `ctx.github_token` absent → abstention.
2. Lecture + classification (U1).
3. `AllowedGreen` / `AllowedPending` → ligne d'audit, `Ok(())`.
4. `Refused` → WARN + audit + `ToolOutput::error` avec un corps JSON structuré
   (`error`, `doctrine`, `target`, `failing_checks`, `remedy`), sur le modèle de
   mika#1646. Le `remedy` nomme **les deux** sorties (D3) et rappelle que le
   flag correct est `--comment`, de sorte que la réécriture ne tombe pas dans
   mika#2237.

Branchement dans `run_gh` **immédiatement après**
`validate_pr_review_flag_coherence` (`builtin_handlers.rs:3240`), avec le
commentaire qui dit pourquoi cet ordre (D1).

**Forme du corps du refus, et pourquoi un seul token est masqué.** La contrainte
exacte est : **aucune ligne du corps ne doit commencer par `VERDICT:`** — après
espaces de tête et after emphase markdown éventuelle, et quelle que soit la
casse. Les tokens `block[ci]` et `hold[review]` sont au contraire nommés **en
clair, entre backticks, au milieu d'une phrase**, et c'est sûr.

L'asymétrie n'est pas un oubli : elle est dérivée de la forme des deux lecteurs,
et chacun a été relu pour ce plan.

1. `parse_verdict` (`server/verdict.rs:235`) capture par
   `VERDICT_RE = (?mi)^\s*[*_]*\s*VERDICT:\s*(.+)$` (`verdict.rs:54`). Le motif
   est **ancré en début de ligne sur le littéral `VERDICT:`** : un token nu
   `block[ci]` au fil d'une phrase n'a aucun moyen de le satisfaire. Ce qui le
   satisferait, en revanche, c'est n'importe quelle ligne commençant par
   `VERDICT:` — y compris indentée, y compris en gras, y compris à l'intérieur
   d'un bloc de code, la regex ne connaissant pas les clôtures. D'où la
   formulation ci-dessus, **plus forte** que « aucune ligne `VERDICT:`
   complète » : c'est le *préfixe de ligne* qui est interdit, pas la ligne
   canonique entière.
2. `validate_tool_arg_suffixes` (`builtin_handlers.rs:2096`) cherche la **ligne
   complète** (`"VERDICT: block[ci]"`, via `str::contains`) dans les trois
   dernières lignes non vides — et il lit l'argument `pr_review_body` d'un appel
   `run_gh`, **jamais** le corps d'un `ToolOutput::error`. Il est donc hors
   d'atteinte du corps du refus dans les deux sens : ni déclenchable par lui, ni
   satisfait par un token nu.

**Conséquence, et c'est la direction sûre :** masquer aussi `block[ci]` et
`hold[review]` ne protégerait de rien (aucun lecteur ne les capte hors préfixe)
et coûterait la lisibilité du remède — un refus qui n'ose pas nommer sa sortie
est le mode de panne que D3 existe pour éviter. **Ne pas généraliser ce
raisonnement par analogie :** il tient parce que les deux lecteurs ont été lus,
pas parce que « seul `VERDICT:` compte » serait une règle. Toute nouvelle
grammaire de verdict ancrée autrement rouvrirait la question — c'est la borne
que `verdict.rs:200-217` écrit déjà pour ses propres exemptions. Citation : U2
+ mika#2201 (`check-canonical-tokens.sh`, qui vérifie la casse des tokens et
l'absence d'espace avant les deux-points, et dont L1/L4 portent précisément sur
les lignes de callout — pas sur les mentions entre backticks, qu'il traite comme
des mentions et non des instructions).

### U3 — Le kill-switch (R7, D5)

`MIKA_QA_CI_COHERENCE_GATE`, motif `MIKA_TELEGRAM_HTML_RENDER` (mika#2291) :
défaut **armé** ; `0`/`false`/`off`/`no` (insensible à la casse, espaces
tolérés) désarment ; absent, vide **ou non reconnu** reste **armé** avec un WARN
nommant la valeur entre guillemets — un désarmement par coquille sur un gate de
sûreté serait la panne silencieuse que tout ceci ferme. Lu une fois par process
(`OnceLock`), `qa_ci_coherence_gate_disabled` émis au démarrage seulement si
désarmé.

### U4 — Tests (R1, R3, R5, R6, R9)

**Unités de prédicat** (`evidence::guards::tests::mika2455`) : un check requis
`fail` → `Refused` avec son nom ; `cancel` → `Refused` ; `pending` seul →
`AllowedPending` (R6) ; liste vide → `AllowedGreen` (aligné sur le
« empty → treat as all-pass » de `classify_checks`) ; tous `pass`/`skipping` →
`AllowedGreen`.

**Contrôles négatifs, sans lesquels « le gate décide » ne se distingue pas de
« le gate bloque »** : un corps `hold[review]` sur CI rouge passe ; un
`block[ac]` sur CI rouge passe ; un corps sans `VERDICT:` passe ; un `pass` sur
CI verte passe **et écrit `allowed_green`** (R9).

**Chemin de production** (`crates/mika-agent/tests/eval/test_qa_ci_coherence_2455.rs`,
`MockLlmProvider`, sans réseau, motif `test_pr_review_flag_coherence_2237.rs`) :
le test négatif littéral du ticket dans ses deux directions — CI requise rouge →
la revue n'est pas postée et le refus nomme les checks ; tous verts → la revue
est postée. Plus une assertion qui sépare « accepté parce que vert » de
« accepté parce qu'abstenu », par la ligne d'audit.

**Non-collision avec mika#2237** (le point de M4, et le test qui empêche la
régression) : sur un corps `pass` + CI rouge, la sortie prescrite
(`block[ci]` + `--comment`) est effectivement postable — donc les deux gardes
composées laissent une issue. Sans ce test, l'interblocage reviendrait sans
qu'aucune assertion ne rougisse.

### U5 — Documentation (R2, D5)

- `crates/mika-agent/CLAUDE.md` : une section mika#2455 à la suite du bloc
  « Verdict↔flag coherence gate », qui pose les cinq points portants —
  le gate est sur le verdict et non sur le flag (D1), le fail-open et son
  asymétrie (D4), le `pending` hors population (R6/M5), les surfaces et le
  contrôle positif (D5), et la limite de tête (D7).
- Racine `CLAUDE.md` : la variable dans la liste, avec sa sémantique de
  désarmement, ses greps et sa sonde.
- Réparer la référence cassée de `qa_pr_view.sh:12` (M2) — la remplacer par les
  sites qui portent encore le raisonnement, sans réécrire la décision.

  **Borne explicite, parce que ce point touche R4.** C'est le **seul** hunk
  autorisé sur ce fichier : la ligne `SAFE_FIELDS` (`:34`) et la validation
  anti-injection d'option (`:25-31`) ne bougent pas d'un caractère. La
  distinction porte : réparer un lien de documentation dans un commentaire
  n'ouvre aucune capacité, alors que toucher `SAFE_FIELDS` rouvrirait exactement
  la décision de M2. La DoD nomme la commande qui sépare les deux, et c'est le
  `grep` sur `SAFE_FIELDS` — pas le `git diff` — qui porte l'invariant.

## Verification Contract

### Séquence rouge prescrite

Chaque unité doit avoir été observée **rouge avant d'être verte**, sous peine de
ne rien attester :

1. Écrire d'abord le test de production « CI requise rouge → pas de revue
   postée » **avant** le gate. Il doit échouer en affirmant que la revue *a*
   été postée — c'est la reproduction du défaut mesuré sur #2439.
2. Écrire le contrôle négatif « `pass` sur CI verte → postée + `allowed_green` »
   **avant** l'écriture de la ligne d'audit nominale. Il doit échouer sur la
   ligne manquante, jamais sur la revue.
3. Écrire le test de non-collision mika#2237 (U4) **avant** de brancher le gate,
   et vérifier qu'il rougit si on le branche sur le flag plutôt que sur le
   verdict. C'est la seule assertion qui protège de M4.

### Commandes

```
cargo test -p mika-agent mika2455
cargo test -p mika-agent --test eval test_qa_ci_coherence_2455
cargo test -p mika-agent mika2237   # non-régression du frère
cargo test -p mika-agent classify_checks
cargo clippy --all-targets -- -D warnings
cargo fmt --check
make verify-bundled-skills
scripts/verify-pipeline.sh
```

Si U5 touche un `system_prompt.md` ou un `.claude/commands/*.md` (ce que ce plan
n'attend pas), `scripts/check-canonical-tokens.sh` devient bloquant : la casse
des tokens de verdict et l'absence d'espace avant les deux-points de `VERDICT:`
sont vérifiées (mika#2201).

## Definition of Done

- [ ] `validate_qa_ci_coherence` refuse un corps `pass` quand un check requis
      est rouge, avant tout sous-processus, et le corps du refus nomme les
      checks rouges et les deux sorties correctes.
- [ ] Aucun autre verdict, et aucun corps sans `VERDICT:`, n'est affecté.
- [ ] **R4, surface 1 — la capacité de `qa_pr_view` est inchangée.** Vérifié
      par une commande, pas par lecture :
      ```
      grep -n 'SAFE_FIELDS=' skills/bundled/qa-review/handlers/qa_pr_view.sh
      # attendu, au caractère près :
      # SAFE_FIELDS="title,body,additions,deletions,files,labels,state,headRefName,headRefOid,baseRefName,author"
      git fetch origin && git diff origin/main -- skills/bundled/qa-review/handlers/qa_pr_view.sh
      # attendu : vide, OU un hunk unique sur la ligne de commentaire `See: docs/solutions/…` (U5)
      ```
      La seconde commande est ce qui sépare « capacité intacte » de « fichier
      intact » : U5 modifie ce fichier par conception, et R4 ne l'interdit pas —
      la première commande est celle qui porte l'invariant.
- [ ] **R4, surface 2 — `QA_REVIEW_GH_ALLOWED` est inchangé.** Un `git diff` de
      fichier ne convient pas : U2 modifie `builtin_handlers.rs` par
      construction. La commande porte sur le **bloc** de la constante :
      ```
      diff <(git show origin/main:crates/mika-agent/src/skills/builtin_handlers.rs \
               | sed -n '/^const QA_REVIEW_GH_ALLOWED/,/^];/p') \
           <(sed -n '/^const QA_REVIEW_GH_ALLOWED/,/^];/p' \
               crates/mika-agent/src/skills/builtin_handlers.rs)
      # attendu : vide
      ```
- [ ] **R4, surface 3 — les règles de prompt « ne fetch pas la CI » sont
      inchangées.** Deux commandes, et la seconde n'est pas redondante :
      ```
      git fetch origin && git diff origin/main --stat -- \
        skills/bundled/qa-review/ skills/bundled/qa-review-build-callback/
      # attendu : vide
      grep -c 'Do NOT fetch or reason about GitHub CI status' \
        skills/bundled/qa-review/system_prompt.md \
        skills/bundled/qa-review-build-callback/system_prompt.md
      # attendu : 1 pour chacun des deux fichiers
      ```
      Le `grep` survit à un rebase, à un `git diff` mal basé et à un
      déplacement de la règle dans le fichier ; le `diff` ne survit à aucun des
      trois. C'est le `grep` qui porte l'invariant, le `diff` qui donne la
      portée.
- [ ] **La base de comparaison est `origin/main`, jamais `main`, et c'est
      mesuré.** Au moment de la rédaction, `main` local était `acd7ac2f` et
      `origin/main` `acd3980c` : `git diff main -- skills/bundled/qa-review/`
      rendait **six lignes** de mika#2419 avant qu'une seule ligne de ce
      correctif soit écrite. Une case DoD basée sur `main` aurait donc rougi
      faussement, et l'aurait fait d'une manière qu'on impute spontanément au
      correctif. Le `git fetch origin` en tête de chaque commande n'est pas
      décoratif.
- [ ] Toute impossibilité de lire la CI abstient et émet
      `qa_ci_coherence_abstained` avec son `reason`.
- [ ] `pending` ne refuse jamais.
- [ ] `MIKA_QA_CI_COHERENCE_GATE=0` désarme sans redéploiement ; une valeur non
      reconnue laisse armé avec un WARN.
- [ ] Un `pass` sur CI verte écrit `allowed_green`.
- [ ] Le test de non-collision mika#2237 est vert, et rougit si le gate est
      branché sur le flag.
- [ ] `cargo test`, `clippy -D warnings`, `fmt --check`,
      `make verify-bundled-skills` et `scripts/verify-pipeline.sh` passent.
- [ ] `crates/mika-agent/CLAUDE.md` et la racine `CLAUDE.md` sont à jour.

## Acceptance criteria

Le ticket n'a pas de section `## Acceptance criteria` formelle. Les critères
ci-dessous transcrivent son « Attendu » et son « Test négatif », et explicitent
ce que M1–M5 ont déplacé.

- **AC1** — Une PR portant au moins un check **requis** en échec sur la tête ne
  peut pas recevoir un verdict `pass` de mika-qa : l'appel `gh pr review` est
  refusé avant publication. *(Transcription du « Attendu » et de la première
  moitié du « Test négatif ».)*
- **AC2** — Une PR dont tous les checks requis sont verts peut recevoir `pass`,
  et la revue est postée. *(Seconde moitié du « Test négatif » — le contrôle
  négatif sans lequel AC1 ne prouve pas que le gate décide.)*
- **AC3** — Un check requis `pending` ne refuse rien : la revue est postée.
  *(Non demandé par le ticket, rendu nécessaire par M5 — sans lui le gate
  refuserait le cas nominal.)*
- **AC4** — Le refus nomme les checks rouges **et** au moins deux sorties
  correctes, et la sortie nommée est effectivement postable en présence du gate
  mika#2237. *(Rendu nécessaire par M4 : sans lui le remède crée un
  interblocage.)*
- **AC5** — Le verdict QA n'a **pas** accès à l'état CI : `qa_pr_view`, le
  périmètre `gh` de qa-review et les règles de prompt correspondantes sont
  inchangés. *(Rectification de la lettre du ticket, qui demandait
  `gh pr view --json statusCheckRollup` côté reviewer — voir M2 et D6.)*
- **AC6** — Toute impossibilité de lire l'état CI laisse passer la revue et
  émet un événement d'abstention nommant la cause.
- **AC7** — Le gate est désarmable par variable d'environnement sans
  redéploiement, et son désarmement est visible au démarrage.
- **AC8** — Les décisions du gate sont comptables en SQL par
  `tool_name = 'qa_ci_coherence_guard'`, refus et acceptations nominales
  incluses.

## Fire-Disposition

**Livré armé.** Le précédent qui décide est mika#2272 : mika#2249 avait été
livré désarmé « par prudence », derrière une condition d'armement qui s'est
révélée **insatisfiable** parce que la population scrutée était vide par
construction — *« zéro était l'absence de mesure, pas la présence de
prudence »*. Ici, le défaut est mesuré à n=2 en un jour, et ce qui paie la
prudence est structurel plutôt que conditionnel : le fail-open de D4 (toute
incertitude laisse passer), l'exclusion du `pending` (R6), la sortie toujours
atteignable (R8/D1) et le kill-switch de U3.

## Sondes post-déploiement, et leurs haltes

### Sonde 1 — Caractérisation, **à exécuter avant de conclure sur AC1**

Non exécutable depuis ce worktree (`gh` non authentifié — M5). Sur #2439 et
#2461, établir si les checks requis étaient **terminés** rouges ou encore
`pending` à l'instant du verdict QA :

```
gh pr view 2439 --repo senara-solutions/mika --json reviews,statusCheckRollup
gh pr view 2461 --repo senara-solutions/mika --json reviews,statusCheckRollup
```

Comparer l'horodatage de la revue `mika-platform-qa` à celui de conclusion des
checks.

**La sonde a trois branches, et chacune est un résultat à écrire.** Une halte
qui n'en nomme que deux laisse la troisième — la plus probable — se lire comme
un demi-échec, ce qu'elle n'est pas.

- **(a) Les deux checks étaient terminés rouges au moment du verdict.** Le gate
  ferme les deux cas mesurés. AC1 est confirmée sur la population réelle, et le
  ticket de suivi « ne réviser que sur CI conclue » (Scope Boundaries) perd sa
  précondition — il reste ouvrable, il n'est plus appelé par cette mesure.
- **(b) Cas mixte — un terminé, un `pending`.** Le gate ferme **un cas sur
  deux**, et c'est un résultat complet, pas une moitié de correctif. Ce qu'il
  établit est précisément ce que ni l'un ni l'autre des cas extrêmes
  n'établirait : que **les deux populations existent réellement** dans le parc,
  donc que le gate est nécessaire *et* insuffisant. C'est la branche qui appelle
  le ticket de suivi avec la meilleure évidence — un cas documenté de chaque
  côté de la frontière. Consigne de conduite identique aux deux autres branches :
  **ne pas élargir le gate au `pending`** au motif qu'« il en restait un ».
  L'écrire comme la mesure qu'elle est : « le gate ferme la population terminée,
  la population `pending` est attestée à n=1, suivi ouvert ».
- **(c) Les deux étaient `pending`.** Le gate ne ferme aucun des deux cas
  mesurés. Ne pas élargir au `pending` par réflexe : ce serait refuser le cas
  nominal (M5). Le résultat oriente vers le même ticket de suivi, avec l'urgence
  la plus forte — et il faut l'écrire comme un résultat, pas comme un échec :
  le gate reste juste (il refuse ce qu'il peut observer), sa population est
  simplement ailleurs.

Dans les trois cas, ce que la mesure décide est la **taille** de ce que le plan
ferme, jamais sa justesse (M5). Citation : review-guide § KISS — la mesure
dimensionne, elle ne redessine pas.

### Sonde 2 — Le gate mord (48 h)

```
grep qa_ci_coherence_refused "$MIKA_SPIRIT_LOG_FILE" | jq -c '{pr, failing_checks}'
```

**Régime attendu : non vide et faible.** Chaque ligne est un verdict faux
intercepté. **Halte 2 — si le compte porte le trafic nominal** (plusieurs par
heure, sur des PR différentes), le gate n'intercepte plus une anomalie, il
refuse les revues : établir d'abord *quel* terme est trop large avant tout
réglage. Ce gate est un filet, pas un chemin.

### Sonde 3 — Contrôle positif, la halte la plus importante

```
SELECT after_value, count(*) FROM audit_events
 WHERE tool_name = 'qa_ci_coherence_guard' GROUP BY 1;
```

**Halte 3 — zéro ligne de toute nature** signifie que le gate n'a rien regardé,
pas que tout va bien : soit aucun `pass` n'a été posté (vérifier), soit le
binaire déployé est antérieur au correctif (classe mika#2340), soit le gate est
désarmé. **Établir le déploiement avant de toucher au prédicat.** C'est
exactement la lecture que mika#2205 a dû écrire pour ses deux scans.

### Sonde 4 — L'inertie est muette

```
grep qa_ci_coherence_abstained "$MIKA_SPIRIT_LOG_FILE" | jq -c '{reason, pr}'
```

**Régime attendu : proche de zéro.** Une abstention soutenue sur `no_token`
signifie que la résolution de jeton est cassée sur ce chemin (classe mika#2205),
sur `gh_timeout` que le plafond de U1 est trop serré. Dans les deux cas c'est
la cause nommée qu'il faut traiter, pas le prédicat.

### Sonde 5 — Non-régression du frère

```
grep pr_review_flag_refused "$MIKA_SPIRIT_LOG_FILE" | wc -l
```

Le volume ne doit pas changer d'ordre de grandeur. Une **hausse** brutale dit
que le nouveau gate pousse le modèle à dégrader son flag au lieu de son verdict
— le contournement que D3 laisse ouvert et que le corps du refus doit fermer par
sa formulation (R2/AC4).

## Ce que ce travail n'achète PAS

- **Il ne rend pas une CI verte garantie par un APPROVED.** Un `pass` posté
  alors que la CI était `pending`, et qui rougit ensuite, reste visible et
  trompeur (M5). Ce qui couvre alors est en aval : `pr_merge_with_gate` refuse
  le merge, `self-dev-webhook-ci` dispatche un fix.
- **Il ne ferme pas la population `pending`**, qui est peut-être celle de l'un
  des deux cas mesurés, ou des deux — sonde 1 le dira, et ses trois branches
  sont écrites comme trois résultats.
- **Il ne double pas le gate de merge** (M1) et n'ajoute aucune garantie de
  merge.
- **Il ne donne aucune connaissance nouvelle au reviewer** : le modèle ne voit
  toujours pas la CI, par R4. Ce qu'il gagne est un refus, pas un signal.
- **Aucune migration, aucune colonne, aucun nouvel appel réseau sur le chemin
  nominal d'une revue** — l'appel n'a lieu que sur un corps `pass`.

## Références

- Ticket : senara-solutions/mika#2455 (+ ses deux commentaires, n=2)
- Preuves : PR #2439 @`73ec3e3e`, PR #2461 @`1302b0d0`
- Frère de classe et contrainte centrale : mika#2237
  (`crates/mika-agent/CLAUDE.md:530-554`,
  `builtin_handlers.rs:2929`, `evidence/guards.rs`)
- Motif pre-subprocess : mika#1646 (`validate_destructive_action_grounding`)
- Gate de merge CI : mika#485/#490,
  `crates/mika-agent/src/tools/pr_merge_with_gate.rs:680,733`,
  `docs/solutions/architecture-patterns/ci-gate-tool-structural-backstop-for-pr-merges.md`
- Capacité retirée plutôt qu'interdite : `qa-review/handlers/qa_pr_view.sh:6,25`,
  `qa-review/system_prompt.md:45`,
  `docs/solutions/prompt-engineering/qa-review-pipeline-exempt-trailer-parity-2026-05-20.md:40`
- `block[ci]` n'est pas inerte : `verdict_handler.rs:58,1286`,
  `qa-review/skill.toml:78`, et le précédent `block[ac]` (mika#2157,
  `qa-review/system_prompt.md:122`)
- Routage et `pending` : `crates/mika-gateway/src/github.rs:335,341,366`
  (mika#1711, mika#1822), `qa_review_reconcile` (mika#2334/#2347)
- Classe de correctif : `feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`
  (mika#2120)
- Livré armé : mika#2272 (contre mika#2249)
- Un instrument silencieux se lit comme un instrument sain : mika#2205, mika#2131
- Mémoire citée par le ticket :
  `feedback_verify_approved_commit_equals_head_before_mergeable`

## Revision history

- **v1** (2026-09-21) — Plan initial. Trois déplacements par rapport à la lettre
  du ticket, chacun établi par lecture du code : le risque de merge rouge est
  déjà fermé donc le défaut est un signal faux (M1) ; le reviewer ne doit pas
  lire la CI et le gate vit dans le moteur (M2/D2) ; un gate sur le flag
  produirait un interblocage avec mika#2237 donc il porte sur le verdict
  (M4/D1). Le `pending` est nommé comme la borne de ce que le plan ferme, et sa
  mesure est une précondition de conclusion (M5, sonde 1).
- **v2** (2026-09-21) — 1re passe architecte, `Disposition: ITERATE`, quatre
  findings de resserrage, **toutes adressées** :
  - **F1** (Halte 1 ne nommait que deux branches sur trois) — la sonde 1 est
    réécrite en trois branches explicites (a) deux terminés / (b) **cas mixte** /
    (c) deux `pending`, chacune écrite comme un résultat avec sa conduite. La
    branche (b) est nommée comme celle qui porte la meilleure évidence pour le
    ticket de suivi, puisqu'elle atteste les deux populations à la fois. La
    consigne « ne pas élargir au `pending` par réflexe » est répétée sur les
    trois branches plutôt que sur une seule. Citation préservée : review-guide
    § KISS + M5 (la mesure dimensionne, elle ne redessine pas).
  - **F2** (case DoD « diff vide sur ces trois surfaces » non opérationnalisable)
    — la case unique est éclatée en trois cases, **une commande nommée par
    surface** : `grep` sur `SAFE_FIELDS` pour `qa_pr_view`, `diff` du **bloc** de
    la constante pour `QA_REVIEW_GH_ALLOWED` (un `git diff` de fichier ne pouvant
    pas convenir, `builtin_handlers.rs` étant modifié par U2), `git diff` +
    `grep` littéral pour les règles de prompt, avec la raison pour laquelle le
    second n'est pas redondant. Une quatrième case a été ajoutée sur la **base de
    comparaison** : la vérification a montré que `git diff main` rend six lignes
    de mika#2419 aujourd'hui, avant tout correctif (`main` local `acd7ac2f` en
    retard sur `origin/main` `acd3980c`) — la case naïve aurait rougi faussement
    et l'aurait fait d'une façon qu'on impute spontanément au correctif. En
    creusant F2, une **contradiction interne** est apparue entre U5 (réparer la
    référence cassée de `qa_pr_view.sh:12`) et la case « diff vide » : elle est
    levée en précisant R4 (elle porte sur la **capacité**, pas sur l'octet) et en
    bornant U5 au seul hunk du commentaire. Citation préservée : § Verification
    Contract (rouge avant vert, sous peine de ne rien attester).
  - **F3** (D7 justifiait l'écart de `headRefOid` par le coût d'un appel) — D7 est
    reformulée pour porter **la direction sûre du refus** comme raison. La lecture
    du code a montré que l'argument du coût était non seulement secondaire mais
    **faux** : `headRefOid` figure déjà dans les `SAFE_FIELDS` de
    `qa_pr_view.sh:34`. Un paragraphe nomme explicitement l'« optimisation »
    qu'un relecteur futur tenterait et dit pourquoi elle rouvrirait un cas
    délibérément rangé du côté sûr. Citation préservée : review-guide
    § citation-or-silence.
  - **F4** (asymétrie non expliquée entre `VERDICT:` masqué et `block[ci]` /
    `hold[review]` nommés en clair) — **confirmé que seul le préfixe de ligne
    `VERDICT:` est relu**, par relecture des deux lecteurs : `VERDICT_RE`
    (`verdict.rs:54`) est ancré `^\s*[*_]*\s*VERDICT:` et ne peut pas capter un
    token nu au fil d'une phrase ; `validate_tool_arg_suffixes`
    (`builtin_handlers.rs:2096`) cherche la ligne **complète** dans les trois
    dernières lignes non vides de `pr_review_body`, et ne lit jamais un
    `ToolOutput::error`. La contrainte U2 est **renforcée** au passage : c'est le
    *préfixe de ligne* qui est interdit (y compris indenté, en gras ou dans un
    bloc de code, la regex ne connaissant pas les clôtures), ce qui est plus fort
    que « aucune ligne `VERDICT:` complète ». La non-généralisation est écrite :
    le raisonnement tient parce que les deux lecteurs ont été lus, pas parce
    qu'une règle « seul `VERDICT:` compte » existerait. Citation préservée : U2 +
    mika#2201.

  Aucun AC n'a été affaibli ; AC5 gagne sa vérification opérationnelle via les
  trois cases de F2, et la sonde 1 reste la précondition de conclusion sur AC1.
