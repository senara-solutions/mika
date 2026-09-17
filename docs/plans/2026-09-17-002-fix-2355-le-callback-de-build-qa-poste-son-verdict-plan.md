---
issue: 2355
type: fix
module: skills/qa-review, agent_loop/INTENT_GUARDS, server/deadline_verdict, task_engine/dispatcher
tags: [loop-breaker, qa-gate, callback, substrate, mika-2276, mika-1251, mika-2334]
problem_type: loop-breaker
---

# fix(2355) : le callback de build QA poste son verdict

## Problème (p1 loop-breaker)

Les revues mika-qa ne posent **aucun** verdict. Les trois tâches de revue du
2026-09-17 (`7309c48c`/#2352, `52780caa`/#2353, `1ea9f92c`/#2350) ont pour
résultat *uniquement* « Build succeeded » : zéro `VERDICT:`, zéro commentaire sur
la PR. Aucune PR du drain ne franchit le gate QA, et l'opérateur pose les
verdicts à la main, une PR à la fois (`mika ask --agent mika-qa`, décision Prime
2026-09-17).

## Racine — quatre faits de source, quatre conséquences

Le ticket écrit « l'agent finit son tour pour attendre le callback, et ne reprend
jamais ». **La première moitié est exacte et voulue ; la seconde est à corriger :
l'agent reprend — c'est le moteur qui lui donne le mauvais contrat de reprise.**
Les quatre faits ci-dessous se lisent dans le source, sans accès à la production.

**F1 — le prompt de reprise QA n'est jamais chargé.**
`qa-review/skill.toml` : `always_on = true`, `dependencies = ["github", "build-mika"]`.
`qa-review-build-callback/skill.toml` : `always_on = false`,
`dependencies = ["qa-review"]` — une arête **entrante**.
`SkillRegistry::callback_safe_skills()` (`skills/mod.rs:964`) amorce sur les
skills `always_on` et suit en BFS les dépendances **sortantes** ; il n'applique
**aucun** matching par mot-clé. `qa-review-build-callback` n'est la dépendance de
personne : le BFS ne l'atteint jamais.
⇒ **Sur un tour de callback, tout le fichier `qa-review-build-callback/system_prompt.md`
est absent** — y compris le « Build Callback Entry Point », la relecture
obligatoire du plan, et la phrase qui porte tout le contrat : *« A qa-review turn
is ONLY complete when a successful `run_gh("pr review …")` call appears in this
turn's tool history. »*

C'est la classe exacte de mika#1251, déjà rencontrée et déjà résolue **pour
self-dev seulement**. Le commentaire de `self-dev/skill.toml:20-24` le dit :
*« self-dev (always_on) is the only edge that makes a non-always-on … skill
reachable on keyword-less … turns »*. Le geste n'a jamais été généralisé à
`qa-review`.

**F2 — le moteur prescrit activement le contrat d'un autre flux.**
`build_callback_trigger_context` (`agent_loop/mod.rs:243-252`) injecte en fin de
contexte : *« This turn MUST end with both of the following before EndTurn :
1. `update_task_status` … 2. `send_message` … EndTurn without both (1) and (2)
will be rejected by the engine and you will be re-prompted. »*
⇒ Le tour de callback QA reçoit, en position de priorité maximale, l'instruction
terminale du flux **self_dev**, et n'a aucun prompt concurrent (F1).

**F3 — la garde de post-condition verrouille ce mauvais contrat.**
L'entrée `callback_terminal_action` de `INTENT_GUARDS` (#870) se déclenche sur
`msg.starts_with("[callback:")` (`callback_trigger_active`, ligne 7552) — donc
sur `[callback: long_running:build_mika]` — et n'est satisfaite que par
`update_task_status && send_message` (ligne 7576). Son commentaire affirme :
*« F1 callback-site audit confirmed only one callback flow exists today
(long_running:run_claude_pilot) »*. **C'était vrai ; `build_mika`
(`long_running: true` dans `build-mika/tools.json`) en est le second.**
⇒ Le moteur re-prompte jusqu'à obtenir `update_task_status` + `send_message`, et
**aucune garde n'exige `run_gh pr review`**. Un tour qui répond « Build
succeeded » par `send_message` satisfait le moteur : c'est exactement le symptôme
mesuré.

**F4 — aucun filet, et aucun rattrapage.**
Le filet mika#2276 (`post_deadline_verdict_if_cut_off`) est câblé au seul
`server/handlers.rs:1536` et ne s'arme que sur `output.deadline_exceeded.is_some()`.
Le tour 1 se termine **proprement** après avoir lancé `build_mika` →
`NotApplicable("turn_completed")`. Le tour de callback passe par
`task_engine/dispatcher.rs::dispatch_resume_agent`, qui ne connaît pas ce filet.
⇒ Rien ne poste. Et le rattrapage de mika#2334 ne s'applique pas : sa population
exclut les PR qui portent **déjà** une demande de revue pour `mika-platform-qa` —
or GitHub ne retire cette demande qu'à la soumission d'une revue, laquelle
n'arrive jamais. **Le silence est définitif**, ce qui est précisément pourquoi il
a fallu un geste opérateur.

### Le fil rouge

Trois affirmations du source ont cessé d'être vraies le jour où `build_mika` est
devenu un second flux de callback `long_running`, et aucune ne fait d'erreur de
compilation : *« only one callback flow exists today »* (F3), le framing
générique qui est en réalité le prompt de `self-dev-callback` recopié en dur
(F2), et `pr_reviews_posted: None, // Silent mode: no session-scoped dedup needed`
(`agent_loop/mod.rs:4723`) — faux dès qu'un tour silencieux poste une revue.

**La troisième se contredit à 155 lignes de distance, dans le même fichier.** En
4566-4569, le commentaire qui arme la validation de suffixe d'argument en mode
silencieux écrit noir sur blanc : *« Tool-arg suffix validation fires in silent
mode too — qa-review runs in callback turns and must still validate verdict
trailers before GitHub submission »* (mika#899). Le moteur **sait** donc que ce
tour-là poste des revues GitHub — il en valide le corps — puis affirme 155 lignes
plus bas qu'aucun dédoublonnage de revue n'y est nécessaire. Ce n'est pas une
hypothèse à confirmer en production : la preuve que B3 est possible et que son
commentaire est faux tient dans un seul écran de code.

### Corollaire : `self-dev-callback` non plus n'est pas chargé

`self-dev/skill.toml` ne déclare pas `self-dev-callback` dans ses dépendances :
par F1, lui non plus n'est jamais chargé sur un tour de callback. Le flux
self_dev ne le voit pas, parce que F2 et F3 portent son contrat **en dur dans le
moteur**. Cela explique pourquoi la famille self_dev fonctionne sans son skill et
pourquoi qa-review, dont le contrat terminal est différent, est détourné vers le
contrat self_dev. **Hors périmètre ici** (voir § Hors périmètre) : rendre
`self-dev-callback` soudainement actif changerait le comportement du chemin le
plus chaud du dépôt sans que personne l'ait décidé.

## La voie « build synchrone » est fermée, et il faut le dire

Le ticket propose en alternative de rendre le build synchrone dans le tour
qa-review. **Cette voie rouvre exactement le défaut que mika#2276 vient de
fermer.** `build_skill_tool_timeouts` (`agent_loop/mod.rs:5718`) a été introduit
parce que, trace `921f11f0`, deux `cargo test --release` de 237,9 s et 231,1 s
ont consommé 469 s d'une enveloppe de ~506 s dans un tour QA, qui est mort sur sa
deadline sans écrire de verdict. Le budget d'outil par skill existe pour retirer
au tour QA le moyen de se suicider par recompilation. Un build synchrone le lui
rend. La voie asynchrone est la bonne ; ce qui manque est la reprise.

## Correctif — trois briques, par ordre de nécessité

### B1 — rendre le prompt de reprise atteignable (la cause)

Ajouter `"qa-review-build-callback"` aux `dependencies` de
`skills/bundled/qa-review/skill.toml`, avec le commentaire qui dit pourquoi
(l'arête entrante ne suffit pas ; `callback_safe_skills` ne suit que les
sortantes ; précédent mika#1251).

Une ligne, effet décisif : le tour de callback reçoit enfin son propre contrat.

**Précondition vérifiée — l'aval ne bloque pas.** `qa-review-build-callback` est
**déjà** déclaré dans l'allowlist de mika-qa (`well_known_agents.rs`,
`MIKA_QA_IDENTITY`, entre `qa-review` et `qa-review-webhook-success`). Les skills
bundled étant refusés par défaut hors allowlist, une dépendance résolue mais non
allowlistée aurait été filtrée juste après le BFS, et B1 aurait été une ligne sans
effet. Ce n'est pas le cas : le skill est autorisé et n'a jamais été atteignable —
ce qui est exactement la signature d'un chaînon manquant, pas d'un choix.

**Coût nommé, et ce qu'il n'est pas.** `qa-review` étant `always_on`, ses
dépendances sont résolues sur **tous** ses tours, pas seulement les callbacks :
+19 330 octets de prompt sur chaque tour mika-qa (taille mesurée du fichier).
C'est un coût de **contexte à l'exécution**, pas un coût de gate — le gate
`max_prompt_size` est **par skill** et non sur la somme, et
`qa-review-build-callback` mesure 19 330 octets contre son propre plafond de
32 768 (59 %). B1 ne rapproche donc aucun gate de son seuil. Accepté parce que la
dépendance déclarée est le geste maison (mika#1251), qu'elle est bornée au skill
qu'on nomme, et qu'un axe `callback_handler` générique dans le manifeste
embarquerait `self-dev-callback` par la même mécanique, sans décision. Cet axe est
la bonne généralisation ; il a son propre ticket.

**Le coût qui compte n'est pas les octets : c'est que le tour 1 lise un prompt de
reprise. B1 ne peut donc pas être livrée nue.** `match_skills`
(`skills/matcher.rs:135-152`) fait **le même BFS sortant** que
`callback_safe_skills` ; la dépendance déclarée est donc résolue sur tous les
tours de `qa-review`, et le snippet est injecté verbatim —
`agent_loop/mod.rs:6361` écrit `## {nom} Skill\n{prompt}` sans aucun cadrage
conditionnel. Or la **première ligne** du fichier est : *« You are mika-qa
resuming a QA review after a build_mika callback. **Steps 1–3d were completed in
the previous turn — do NOT re-run them.** »*
⇒ Sur le tour 1, B1 nue place cette phrase dans le prompt d'un tour qui n'a rien
fait, et elle autorise textuellement à sauter Step 3 — la revue du diff. Le risque
n'est pas cosmétique : il **change la classe du défaut**, d'un loop-breaker
visible (aucun verdict) vers une régression silencieuse (un verdict `pass` posé
sans revue de diff). Un correctif de gate QA qui dégrade la QA sans le dire est
pire que le silence qu'il remplace.

**Garde-fou, livré avec B1 :** un en-tête de portée en tête de
`qa-review-build-callback/system_prompt.md`, énonçant que tout le fichier ne
s'applique **que** si le message du tour porte le label de callback de build
(§ B2 pour le discriminant, nommé une seule fois), et qu'autrement il doit être
ignoré intégralement. C'est l'énoncé de la **condition d'applicabilité d'un
document**, non l'application d'une règle de processus par prompt : la doctrine
maison (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`)
vise la seconde. La marge existe là où elle manque ailleurs :
`qa-review-build-callback` est à 19 330 octets pour un plafond de 32 768, soit
**11 799 octets** sous le gate à 95 % — contre 1 661 côté `qa-review`.

La forme définitive reste l'axe manifeste `callback_handler` (une dépendance
résolue **uniquement** sur les tours de callback), qui rendrait ce garde-fou sans
objet. Il a son ticket ; le garde-fou est ce qui rend B1 livrable sans lui.

**En revanche, la marge de `qa-review` lui-même est mince, et elle contraint B2.**
`qa-review/system_prompt.md` mesure 68 380 octets contre un plafond déclaré de
73 728 : le gate à 95 % **panique** (il n'avertit pas —
`tests/bundled_skills_load.rs:141,166`) à 70 041 octets, soit **1 661 octets de
marge**. Conséquence opératoire directe : **B2 ne peut pas être résolue en
ajoutant du texte au prompt de `qa-review`.** Le correctif du contrat terminal
doit vivre dans le moteur (garde + framing), ce qu'il fait déjà ci-dessous — mais
la tentation du paragraphe de prompt est la première qui vient, et elle casse le
build.

### B2 — cesser de prescrire le mauvais contrat terminal, exiger le bon

Le contrat terminal d'un tour de callback dépend du **flux**, pas du fait d'être
un callback. Discriminant : le label de la tâche, déjà présent dans le message
(`long_running:{tool_name}`, `executor.rs:2911`).

**Ce que ce label discrimine, et ce qu'il ne discrimine pas.** Il porte le nom de
l'outil, jamais l'agent ni le skill. Les outils `long_running: true` du dépôt sont
`build_mika`, `deploy_mika`, `address_pr_comments`, `resolve_pr_conflicts`
(`tools.json`), plus `run_claude_pilot` / `run_claude_pilot_groom`. Un prédicat
posé sur `long_running:build_mika` retire donc le contrat self_dev à **tout**
callback de build, y compris ceux de mika-dev — qui porte `build-mika` dans son
allowlist (`well_known_agents.rs:145`). C'est **accepté et motivé**, pas subi :
*« marquer la tâche parent self_dev terminale »* + `send_message` est le contrat
terminal d'un **dispatch de pilote**, pas d'un build ; l'imposer à un `build_mika`
lancé hors flux self_dev était déjà la même sur-portée que F3 décrit, au même
endroit. Ce que B2 ne doit pas faire, c'est l'élargir : les trois autres outils
`long_running` gardent leur comportement d'aujourd'hui à l'identique (AC9).

1. `callback_trigger_active` cesse de couvrir le callback de build : la garde
   `callback_terminal_action` (#870) ne s'y arme plus. **Deux sites lisent ce
   prédicat** — l'entrée du registre `INTENT_GUARDS` (chemin texte non-vide) et le
   contrôle *inline* de `agent_loop/mod.rs:2898`, dont le commentaire dit qu'il
   « mirrors the INTENT_GUARDS entry » pour le chemin **texte vide**. Les deux
   suivent d'office puisqu'ils appellent le même prédicat, mais le test doit
   l'asserter sur les deux : un EndTurn sec n'est pas un texte, et c'est
   exactement la forme que prend un tour qui n'a rien à dire. Corriger au passage
   le commentaire dont l'affirmation « un seul flux » est ce qui a rendu la
   sur-portée invisible.
2. `build_callback_trigger_context` cesse d'injecter l'instruction
   `update_task_status` + `send_message` sur ce flux.
3. **Positivement** : une garde qui exige « un `run_gh` `pr review` appelé avec
   succès dans ce tour », dont le message de correction nomme `run_gh pr review`
   et la ligne `VERDICT:` attendue. C'est la force réelle du moteur sur ce chemin,
   et la même mécanique qui re-prompte aujourd'hui vers le mauvais contrat
   re-promptera vers le bon.

   **Elle ne peut pas être une entrée d'`INTENT_GUARDS`, et c'est structurel.**
   `IntentPrecondition.trigger` a pour signature `fn(&str) -> bool` : le message
   seul. Or le label ne distingue pas mika-qa de mika-dev (ci-dessus) — une entrée
   du registre armée sur `long_running:build_mika` **re-prompterait mika-dev pour
   poster une revue de PR** qu'il n'a aucune raison de poster, c'est-à-dire
   qu'elle échangerait le loop-breaker QA contre un loop-breaker dev. La garde est
   donc **inline**, sur le précédent exact et voisin de
   `callback_milestone_advance` (`mod.rs:2093-2098`), dont le commentaire énonce
   la même raison : *« Inline rather than in INTENT_GUARDS because the satisfied
   predicate needs [more than the registry signature] »*. L'information nécessaire
   est déjà là : `run_silent_agent` calcule `matched` en `mod.rs:4531`. Le trigger
   est **conjonctif** — label de callback de build **ET** `qa-review` parmi les
   skills du tour — donc il ne s'arme que là où un verdict est dû.

Le discriminant vit à **un seul endroit** (une constante + un prédicat nommés),
jamais recopié entre le framing, la garde négative, la garde positive et l'en-tête
de portée de B1 : quatre lecteurs d'une même grammaire, c'est la classe que
mika#2158 a dû fermer une fois — et l'en-tête de B1 étant du **prompt**, il ne
peut pas partager la constante Rust : un test doit alors épingler que la chaîne
qu'il cite est bien celle que le moteur émet.

### B3 — le filet : une PR n'est plus jamais muette

B1 et B2 réparent le prompt et la garde. B3 borne ce qui resterait, selon la
doctrine que ce dépôt applique déjà (`feedback_prompt_enforcement_empirically_confirmed_at_loop_substrate`,
mesurée par mika#2120) et que le module `deadline_verdict` énonce en propres
termes : *« M1 sans ce filet laisse la prochaine cause de dépassement muette. »*

Si le tour de callback de build QA se termine sans qu'un `run_gh pr review` ait
réussi, le moteur poste lui-même `VERDICT: hold[review]` sur la PR, en réutilisant
`server::deadline_verdict` — même forme, même registre anti-double-post, même
traitement idempotent du 422, corps adapté au motif (*ce tour n'a pas conclu*,
pas *ce tour a été coupé*).

Deux pièces manquent pour que ce soit possible, et toutes deux suivent une
trajectoire déjà employée :

- **La cible PR doit traverser la frontière du tour.** Le tour de callback n'a
  pas `req.text` et ne peut donc pas `parse_pr_target`. Elle lui est **dite, jamais
  dérivée** : le call-site webhook la calcule déjà (`handlers.rs:1131`), la porte
  sur le `ToolContext`, et `build_callback_task` (`executor.rs:2897`, dont
  `metadata` vaut aujourd'hui `None`) la stampe sur la tâche callback. Même
  trajectoire que `metadata.dispatch_worktree_file` (mika#2249) et
  `metadata.pilot_transcript_expected` (mika#2040). **Fail-safe dans le sens de
  la maison** : pas de stamp, stamp illisible, cible absente → le filet
  s'abstient et le dit ; il ne devine jamais une PR.
- **Le tour silencieux doit pouvoir dire qu'il a posté.** `dispatch_resume_agent`
  détient déjà `self.pr_reviews_posted` (`dispatcher.rs:312`) et le passe au
  chemin conversationnel (ligne 1011) ; il ne le passe pas au chemin silencieux,
  où il vaut `None` (`agent_loop/mod.rs:4723`). Le propager suffit : `run_gh`
  écrit dans ce registre sur succès de `pr review`, la session du callback est
  neuve, donc le registre porte exactement ce que **ce tour-là** a posté — la
  granularité voulue. Corriger le commentaire qui affirme le contraire.

Le filet ne renvoie jamais d'erreur et ne fait jamais échouer le tick : un filet
qui tombe remplacerait un silence par une panne.

## Hors périmètre, délibérément

- **`self-dev-callback` inatteignable** (même classe, F1) : son contrat est porté
  en dur par le moteur, donc le rendre actif changerait le comportement du chemin
  self_dev sans qu'on l'ait décidé. **Ticket de suivi**, avec l'axe manifeste
  `callback_handler` qui est sa bonne forme.
- **`qa-review-webhook-success`** : lui est déclenché par des mots-clés présents
  dans le texte de l'événement `check_suite`, donc atteignable sur un tour
  conversationnel. Pas le même défaut.
- **Le trou de rattrapage de mika#2334** (une PR portant une demande de revue en
  place n'est jamais re-demandée, quoi qu'il arrive à son tour) : réel, et B3 le
  rend sans objet pour cette classe. L'élargir demande son propre arbitrage —
  une demande en place pendant qu'une revue tourne est le cas nominal.
  **Ticket de suivi.**
- **Le build synchrone** : réfuté ci-dessus, pas descopé.

## Tests

Tous rouge-avant : les trois faits de racine sont des propriétés du code, donc
assertables sans production.

1. **F1 figé** — sur le registre réellement embarqué (`all_bundled_skills()`, pas
   une fixture synthétique), `callback_safe_skills()` contient
   `qa-review-build-callback`. Rouge avant B1. Sœur du test mika#1251
   (`test_self_dev_declares_both_dispatch_siblings_as_dependencies`), qui ne peut
   structurellement pas voir cette classe : il raisonne sur des **noms d'outils**
   référencés par le moteur (`ENGINE_REFERENCED_SKILL_TOOLS`), et
   `qa-review-build-callback` n'apporte aucun outil — seulement du prompt.
1b. **B1 ne dégrade pas le tour 1** — sur un tour **conversationnel** de
   `qa-review` (le tour d'ouverture d'une revue), le prompt assemblé contient bien
   le snippet du callback (c'est le BFS de `match_skills`, attendu) **et**
   l'en-tête de portée qui le neutralise. Assertion comportementale jumelle : un
   tour 1 exécute toujours la revue de diff (Steps 1–3d). Rouge si B1 est livrée
   sans son garde-fou — c'est-à-dire que ce test est le seul qui garde la classe de
   défaut la plus coûteuse de tout ce plan, une QA qui passe sans regarder.
2. **F3 figé, sur les deux sites** — `callback_trigger_active("[callback: long_running:build_mika]")`
   est `false` après B2, et reste `true` pour
   `[callback: long_running:run_claude_pilot]`. Asserté **et** par le chemin
   texte non-vide (registre) **et** par le chemin texte vide
   (`mod.rs:2898`) : la garde #870 ne perd rien de sa portée d'origine.
2b. **Portée non élargie** — `deploy_mika`, `address_pr_comments` et
   `resolve_pr_conflicts` conservent à l'identique framing, garde et contrat
   terminal. Les trois sont nommés : une assertion sur le seul
   `run_claude_pilot` laisserait trois outils `long_running` hors contrôle.
3. **B2 positive, et bornée** — un tour de callback de build **avec `qa-review`
   chargé** qui termine sans `run_gh pr review` est re-prompté ; le même tour avec
   un `pr review` réussi passe. **Contrôle négatif, qui est la moitié qui compte :**
   un callback `long_running:build_mika` **sans** `qa-review` chargé (le cas
   mika-dev) n'est **pas** re-prompté. Rouge si la garde est posée dans le
   registre plutôt qu'inline.
4. **B3 filet** — tour de callback de build QA sans verdict et cible PR stampée
   → **un** POST `pr review` portant `VERDICT: hold[review]`. Poster injecté,
   comme `maybe_post_deadline_verdict` le fait déjà : le contrat à asserter est
   *« un verdict EST posté »*, ce qu'un test ne voit qu'en tenant l'exécution.
5. **B3 fail-safe** — pas de stamp / stamp illisible / cible absente → zéro POST
   et une ligne nommant l'abstention.
6. **B3 anti-double-post** — un tour qui **a** posté son verdict n'en reçoit pas
   un second. Pinne la propagation du registre : sans elle ce test est rouge,
   c'est-à-dire que le filet doublerait chaque revue réussie.
7. **Budget de prompt** — les gates `max_prompt_size` restent verts après B1.
   Attendu par construction (le gate est par skill, et `qa-review-build-callback`
   est à 59 % du sien), donc ce test ne garde pas B1 : il garde **B2**, dont la
   solution naïve — un paragraphe ajouté au prompt de `qa-review` — dispose de
   1 661 octets avant de faire **paniquer** le build (le gate ne prévient pas).

## Definition of Done

- B1, B2, B3 livrées ; `cargo test -p mika-agent` vert ; `cargo clippy` propre.
- B1 livrée **avec** son en-tête de portée dans
  `qa-review-build-callback/system_prompt.md` — jamais la dépendance seule.
- Les trois commentaires devenus faux sont corrigés dans le même commit que le
  code qui les dément (#870 « only one callback flow », le framing générique,
  `// Silent mode: no session-scoped dedup needed`).
- `docs/` et `CLAUDE.md` : nouvelles variables d'observation et signaux grep
  documentés ; `scripts/sync-agent-docs.sh` si `docs/` bouge.
- PR ouverte avec le corps écrit sous le worktree (mika#2211).

## Acceptance criteria

- **AC1** — Sur le registre de skills réellement embarqué,
  `callback_safe_skills()` contient `qa-review-build-callback`. Test rouge avant
  le correctif, vert après.
- **AC1b** — B1 ne dégrade pas le tour d'ouverture : sur un tour conversationnel
  de `qa-review`, la présence du snippet de reprise ne fait sauter aucun des
  Steps 1–3d, parce que le fichier porte en tête une condition de portée qui le
  neutralise hors callback de build.
- **AC2** — La garde `callback_terminal_action` (#870) ne s'arme plus sur
  `[callback: long_running:build_mika]`, et s'arme toujours sur
  `[callback: long_running:run_claude_pilot]` — sur **les deux** sites de lecture
  du prédicat (registre `INTENT_GUARDS`, et contrôle inline du chemin texte vide).
- **AC3** — Le framing injecté sur un callback de build QA ne contient plus
  l'instruction terminale `update_task_status` + `send_message`.
- **AC4** — Un tour de callback de build QA qui atteint EndTurn sans
  `run_gh pr review` réussi est re-prompté par une garde dont le message nomme
  `run_gh pr review` et la ligne `VERDICT:`.
- **AC4b** — Cette garde ne s'arme que là où un verdict est dû : un callback
  `long_running:build_mika` sans `qa-review` parmi les skills du tour (le cas
  mika-dev, qui porte `build-mika` dans son allowlist) n'est **pas** re-prompté.
- **AC5** — Filet : un tour de callback de build QA terminé sans verdict, avec
  une cible PR stampée, produit exactement **un** POST `pr review` portant
  `VERDICT: hold[review]`.
- **AC6** — Filet fail-safe : stamp absent, illisible, ou cible non résoluble →
  **zéro** POST, et une ligne de journal nommant l'abstention et son motif.
- **AC7** — Anti-double-post : un tour de callback qui a posté son propre verdict
  ne reçoit pas de verdict de secours (le registre `pr_reviews_posted` est
  propagé au chemin silencieux).
- **AC8** — Bout en bout : une revue qa-review sur une PR à ACs comportementaux
  produit un build **et** un verdict posté sur la PR — pas seulement
  « Build succeeded ». C'est le test nommé par le ticket.
- **AC9** — Aucune régression de portée : les callbacks `run_claude_pilot`,
  `run_claude_pilot_groom`, `deploy_mika`, `address_pr_comments` et
  `resolve_pr_conflicts` conservent à l'identique leur framing, leur garde et leur
  contrat terminal. Les cinq sont nommés parce que quatre outils portent
  `long_running: true` hors du pilote, et qu'une AC écrite sur le seul pilote
  laisserait les trois autres hors contrôle.

## Sondes post-déploiement, et leurs haltes

- **Sonde 1 (symptôme, 48 h).** Toute PR ayant reçu une demande de revue
  `mika-platform-qa` porte une revue soumise. Contrôle direct du loop-breaker.
- **Sonde 2 (attribution).**
  `SELECT count(*) FROM audit_events WHERE tool_name = '<événement du filet B3>';`
  — **doit rester proche de zéro.** Un filet qui porte le trafic nominal a
  remplacé un silence par un `hold[review]` systématique : B1/B2 n'ont alors pas
  pris, et c'est **là** qu'il faut chercher, pas dans le réglage du filet.
- **Sonde 2b (la QA regarde toujours le diff).** Sur les premières revues après
  déploiement, chaque verdict posté porte une section `DIFF ANALYSIS` à puces
  code-level non vides. C'est le contrôle du risque de B1 : un `pass` sans revue
  de diff serait *moins* visible que le silence qu'on vient de réparer, donc il se
  vérifie **avant** de se féliciter que les verdicts reviennent. **Halte** — un
  verdict sans `DIFF ANALYSIS` réelle ⇒ revenir sur l'en-tête de portée, ne pas
  se contenter du test unitaire qui l'aura pourtant validé.
- **Sonde 3 (contrôle négatif).** `grep qa_deadline_verdict $MIKA_SPIRIT_LOG_FILE | jq 'select(.outcome == "posted")'`
  — le filet mika#2276 ne doit pas se mettre à firer : ce correctif ne rallonge
  aucun tour.
- **Halte.** Si les verdicts restent absents alors que les sondes 1 et 2 sont
  propres, **ne pas rendre le build synchrone** (voie réfutée ci-dessus) et ne
  pas élargir le filet : c'est que le callback de build ne revient pas du tout —
  un autre défaut, avec son ticket. Le discriminer d'abord :
  `grep 'resuming agent for callback' $MIKA_SPIRIT_LOG_FILE` filtré sur le label
  `long_running:build_mika`. Aucune ligne ⇒ le tour de callback n'a jamais lieu,
  et la piste est la livraison des callbacks (quarantaine mika#2179, file), pas
  le contrat de reprise. C'est la seule branche du diagnostic que la lecture de
  source ne tranche pas, et elle se tranche en une commande.

## Rollback

Revert du commit. B1 redevient une dépendance non déclarée, B2 rend au callback
de build le contrat self_dev, B3 cesse de poster — c'est-à-dire l'état d'avant,
sans autre changement de comportement. Le filet B3 est en outre désarmable seul
si son kill-switch est jugé nécessaire à la revue.
