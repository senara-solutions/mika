# Vol 3 de mika#2680 — le filtre de la classe (e), et la mesure 3 rendue à l'opérateur

Le vol 1 (#2705, `185362cc`) a livré le registre des tours webhook ; le vol 2
(#2747, `034698b2`) a livré son lecteur et le protocole de mesure
(`docs/eval/mika-2680/README.md`). Le vol 3 devait exécuter ce protocole, puis
implémenter les classes (e), (f) et (g) que la mesure confirme.

**Ce vol-ci livre une classe et rend une halte.** Il implémente le filtre de la
classe **(e)** — `PR closed` / `merged` sans tâche de boucle active — parce que
cette classe est la seule dont la décision ne dépende pas de la mesure 3, et il
l'argumente plutôt que de le supposer. Il n'exécute **pas** la mesure 3 : elle
n'est pas exécutable depuis un pilote dispatché, pour une raison structurelle
nommée au § *Décision 2*, et un pilote qui la tenterait brûlerait ses tours à
redécouvrir le même mur. Les classes (f), (g) et (h) sont écrites **comme
résultats**, pas comme code.

---

## Décision 1 — pourquoi (e) s'implémente sans la mesure 3

Le corps du ticket dit, en toutes lettres : *« Avant toute implémentation,
refaire cette mesure. »* Cette phrase a une raison précise, écrite juste à côté :
*« pour retirer ce que (b) couvre déjà »*. La mesure 3 existe pour **soustraire**
ce que les classes (a)–(d) écartent déjà, afin qu'un chiffrage de (e)+(f)+(g) ne
compte pas comme à filtrer un tour qu'un filtre existant évite déjà.

Pour (e), **il n'y a rien à soustraire, et c'est démontrable par construction** :

- Les quatre classes livrées portent sur trois déclencheurs seulement —
  `check_suite` pour (a) `green_check_suite_default_branch`, (b)
  `green_check_suite_head_decided`, `red_check_suite_default_branch` ;
  `pr_review` et les sorties sans-action de CI pour (c) ; `issues labeled` pour
  (d) `inert_label`.
- (e) porte sur `trigger_class() == TriggerClass::PrClosed`
  (`"pr_closed"` sur le fil). **Aucune classe des phases 1 à 3 ne classe ce
  déclencheur** : `classify()` n'a aujourd'hui aucun bras sur le préfixe
  `PR closed`, et c'est précisément pourquoi `Candidate` n'a pas de variante
  pour lui.
- Donc `skipped_by_class` de la mesure 3 ne peut contenir, pour `pr_closed`,
  qu'un seau vide. La soustraction que la mesure sert à opérer vaut zéro pour
  cette classe-là, quelle que soit la fenêtre.

Le chiffre de (e), lui, est déjà **à n=2**, par deux mesures humaines
indépendantes : **193 k** tokens d'entrée (mesure 1, merge de #2678) et
**279 k** (mesure 2, merge de #2679). Le commentaire opérateur du 2026-10-08 le
dit comme un acquis : *« La classe (e) est confirmée à n=2. »*

Ce que la mesure 3 apporterait à (e) est donc une troisième observation du même
ordre de grandeur, pas une décision. Ce qu'elle apporte à (g) est en revanche
**décisif** — (g) est `check_suite` sur branche de PR, exactement le déclencheur
que (b) couvre — et c'est pourquoi (g) n'est pas implémentée ici.

**La vérification que le ticket demande pour (e) est d'une autre nature, et elle
a été faite.** Le corps écrit : *« sans tâche, que peut faire le tour ? La
clôture de ticket et la mémoire passent par d'autres chemins, à vérifier. »*
C'est une lecture de source, exécutable en bac à sable, et son résultat est au
§ *Décision 3*.

---

## Décision 2 — la mesure 3 n'est pas exécutable depuis un pilote dispatché

**Le fait.** Le corpus de la mesure est le **journal** de `mika-spirit`
(`docs/eval/mika-2680/README.md` § 3 : *« Le corpus est le journal, jamais la
base »*), désigné par `$MIKA_SPIRIT_LOG_FILE`. Aucun pilote dispatché ne peut le
lire :

- `docs/runtime-structure.md:248` déclare ce journal **optionnel** et sans
  emplacement par défaut : il vit là où le lanceur l'a mis. Sur l'hôte de
  production, c'est le `supervise-daemon --stdout /var/log/mika/server.log`
  documenté dans la même section.
- `skills/bundled/_shared/dispatch-lib.sh` (l. 56–110) énumère l'allowlist du
  bac à sable `bwrap` et dit dans son propre commentaire pourquoi elle est
  étroite : *« no `--ro-bind / /` — that would expose /var, /srv, cross-worktree
  /data/workspace paths »*. Les montages sont le worktree, `~/.mika/data`,
  `$_PILOT_LOG_DIR` et quelques chemins git. Ne sont **pas** montés : `/var`,
  `~/.mika` hors `/data`, et `/data/workspace` hors du worktree de la branche.
- La variable elle-même est hors de portée : dans la session de grooming qui
  écrit ce plan, un simple `printenv MIKA_SPIRIT_LOG_FILE` est refusé par la
  permission-policy (`no matching policy rule -- denied by default`).

Ce n'est pas un jugement sur la difficulté de la mesure : c'est une propriété de
l'ensemble de montages. Aucune variante de commande ne la contourne, et
l'instrument se comporte correctement face à elle — `scripts/measure-webhook-turn-cost`
rend `exit 2` (« pas de fichier ») plutôt qu'un zéro qui ressemblerait à un
résultat (§ 6 du protocole, leçon mika#2205).

**Conséquence assumée, et c'est une déviation du corps du ticket.** Le corps dit
*« Il commite la mesure (`turns-*.jsonl`, `summary-*.json`) »*. Ce vol **ne les
commite pas**, parce qu'il ne peut pas les produire. Il fait trois choses à la
place :

1. Il amende `docs/eval/mika-2680/README.md` d'un § *2 bis — qui exécute cette
   commande*, qui nomme le mur et son cause structurelle, pour que le prochain
   lecteur ne le redécouvre pas à ses frais.
2. Il donne à l'opérateur la commande du § 3 telle quelle, avec la fenêtre déjà
   résolue (borne basse `2026-10-16T13:01:00Z`, borne haute au choix, ≥ 4 h 30,
   avec activité PR).
3. Il écrit (f), (g) et (h) comme **résultats en attente**, en distinguant
   explicitement *« non mesurée »* de *« mesurée et non confirmée »* — ce sont
   deux phrases différentes et le corps du ticket n'autorise à écrire que la
   seconde comme un refus.

---

## Décision 3 — la forme du filtre (e), et la lecture de source qui la justifie

### Ce que la lecture de source a établi

Deux handlers traitent un `PR closed`, et **les deux finissent leur travail en
code, sans jamais enrichir le texte** :

- `crates/mika-agent/src/server/milestone_context_handler.rs` (l. 28–100) —
  l'étape 2 exige la vérité du merge, l'étape 3 extrait l'URL de PR, et
  **l'étape 4 porte la réponse** : `db.find_active_task_by_pr_url(&pr_url)` →
  `Ok(None)` rend `Passthrough { enrichment: None }`. Sans tâche active, le
  marqueur `[milestone-parent: <id>]` n'est **jamais** injecté : le LLM ne reçoit
  aucun jalon à faire avancer, donc il n'a rien à avancer.
- `crates/mika-agent/src/server/upstream_close_handler.rs` (l. 85–290) —
  `try_handle_upstream_close` rend **toujours** `Passthrough { enrichment: None }`
  (le bras `Some(_)` est un `unreachable!`). `handle_pr_closed` résout la vérité
  du merge, écrit `("completed", UPSTREAM_PR_MERGED)` ou
  `("cancelled", UPSTREAM_PR_CLOSED_UNMERGED)`, lit `parse_closing_issue_refs`,
  et `record_closing_pr_closed_unmerged` écrit sa ligne d'audit
  `closing_pr_closed_unmerged` **plus** sa ligne `info!` — entièrement en code.

### La propriété d'architecture qui rend la question tranchable

Le pré-filtre est placé **après** les handlers structurels et **avant**
`run_agent` (`crates/mika-agent/src/server/handlers.rs`, l. 2023–2058). Un filtre
qui écarte le tour ne peut donc **priver aucun handler de quoi que ce soit** :
quand `skip_turn()` est consulté, tout ce que les deux handlers ci-dessus
écrivent est déjà écrit. La seule question qui reste est : *que fait le tour LLM
en plus ?* — et pour un `PR closed` sans tâche de boucle, la réponse établie par
la lecture ci-dessus est : rien de durable.

### La forme

Le filtre est **le jumeau de la classe (a)** : le texte doit être intact dans la
fenêtre la plus stricte, et un seul état est lu.

| terme | valeur | pourquoi celui-là |
|---|---|---|
| déclencheur | préfixe `PR closed` | lu par la **constante** `mika_common::github_event_format::PR_CLOSED_EVENT_PREFIX`, jamais par un littéral (voir *Contrainte de recensement* ci-dessous) |
| texte | `Touched::by_any_handler == false` | la fenêtre la plus stricte, celle de (a) et de (d). Aucun handler n'a touché le texte ⇒ aucun handler n'a quelque chose à dire au LLM |
| état | `pr_has_active_task(pr_url) == false` | **le même prédicat que le handler**, lu par la même porte `PrefilterState` — pas une ré-dérivation |
| erreur de lecture | `state_unreadable(...)` ⇒ le tour a lieu | AC3 de #2675 : fail-safe vers le LLM, inchangé |

**Aucune ré-dérivation, et c'est le point le plus facile à rater.**
`milestone_context_handler` décide sur `find_active_task_by_pr_url` ;
`PrefilterState::pr_has_active_task` lit la même chose par la même porte.
Le filtre ne relit **pas** la vérité du merge, ne reparse **pas**
`Merged: true`, ne réinterprète **pas** `parse_closing_issue_refs` : la classe
couvre `closed` **et** `merged` indifféremment, donc elle n'a aucun besoin de
cette distinction. C'est la doctrine que l'en-tête du module écrit déjà, et dont
il cite les deux précédents payés deux fois (mika#2158 `grooming_marker`,
mika#2362 le seuil de retry recopié).

**Réutiliser l'extracteur d'URL, ne pas en écrire un cinquième.** Il existe
aujourd'hui quatre fonctions nommées `extract_pr_url`, dont **deux** ont
exactement la signature utile — `fn extract_pr_url(text: &str) -> Option<String>`
dans `upstream_close_handler.rs:421` et dans `milestone_context_handler.rs:455`
(les deux autres, `skills/context.rs:247` et `skills/executor.rs:4646`, ont des
signatures et des sémantiques différentes et ne sont pas du périmètre). Le geste
de ce vol est **minimal et sans copie** : passer celle de
`upstream_close_handler.rs` en `pub(crate)` et la `use` depuis le pré-filtre.
La duplication préexistante entre les deux handlers n'est **pas** créée par ce
vol et n'est pas réparée par lui — elle est renvoyée à un ticket de suivi nommé
au § *Renvois explicites*. Écrire une troisième copie, en revanche, serait la
divergence programmée que ce module condamne.

**Contrainte de recensement, à ne pas deviner.**
`canonical_tokens::tests::mika2517_the_fallthrough_domain_has_a_single_definition`
exige qu'un seul corps écrive la conjonction des littéraux `"[GitHub] PR "` et
`"[GitHub] Check suite "`, et ce corps est
`crates/mika-common/src/github_event_format.rs`. Son prédicat est l'**égalité**
exacte sur chaque littéral, et il exclut les lignes de commentaire (`//`, `/*`,
`*`) : un `"[GitHub] PR closed:"` ne l'égale pas, et une mention en
doc-commentaire ne le déclenche pas. Le scan ne force donc pas le geste — mais
la doctrine le force : on lit `PR_CLOSED_EVENT_PREFIX`, qui existe déjà
(`github_event_format.rs:110-124`), et on n'écrit aucun littéral neuf.
Corollairement, la section d'en-tête que ce vol ajoute au module nomme la
**constante**, jamais la chaîne.

**Pas d'entrée dans `scripts/canonical-tokens.tsv` pour le nom de classe.** Les
onze noms de fil existants (`green_check_suite_default_branch`, `inert_label`,
`verdict_pr_closed`, …) n'y figurent pas — vérifié par grep. Le nouveau nom suit
la même règle : il est recensé par les assertions inline du module, pas par le
census. Un seul ajout au census est prévu, et seulement s'il manque : la
variante de `Candidate` et son nom de fil, au même titre que ses cinq sœurs.

---

## Décision 4 — (f), (g) et (h) : des résultats, pas du code

Le corps dit : *« Une classe que la mesure ne confirme pas est écrite comme
résultat, pas implémentée. »* Aucune des trois n'est confirmée, et chacune pour
une raison **différente** — les confondre sous un « non mesurées » ferait perdre
l'information qui les sépare.

**(g) `check_suite success` sur branche de PR, côté mika-qa — bloquée par la
mesure, et seule véritablement bloquée par elle.** C'est exactement le
déclencheur que la classe (b) traite. Sans `skipped_by_class` sur une fenêtre
armée avec activité PR, tout chiffre de (g) compte comme à filtrer ce que (b)
écarte peut-être déjà. C'est la soustraction que la mesure 3 existe pour faire,
et elle n'a pas été faite. **Rien d'autre ne manque** : la forme du filtre est
connue (déclencheur `check_suite_pr_branch`, agent `mika-qa`). Première classe à
implémenter quand l'artefact existe.

**(f) `pr_review` sur une PR fermée/mergée au texte réécrit — en partie déjà
livrée, et le résidu est une question ouverte.** La classe (c) porte déjà
`verdict_pr_closed` : le côté *texte intact* de (f) est écarté aujourd'hui. Le
résidu est le côté **texte touché** par le `verdict_handler`
(`verdict_classification_failed`, `CIRCUIT BREAKER TRIPPED`) — et celui-là bute
sur une question que le corps du ticket renvoie lui-même hors périmètre : *« La
renotification opérateur à chaque retraitement DECISION-CORE : nommée dans
#2679, candidate à un ticket à part. »* Écarter ce tour, c'est décider que la
renotification n'a pas lieu. Ce n'est pas une décision d'implémentation.

**(h) `issue_comment` / `issues closed` sans tâche de boucle — le plus gros
chiffre, et la question est explicitement posée à l'opérateur.** 1,30 M tokens,
**79 %** de la fenêtre de la mesure 2. L'opérateur a posé sa question dans le
même commentaire et ne l'a pas tranchée : *« que doit faire le tour quand un
commentaire d'opérateur arrive sur un ticket sans tâche ? Aujourd'hui, il met à
jour la mémoire et le fil de priorités. Ce travail est utile mais pas forcément
d'un coût LLM complet. »* Le protocole dit quoi en faire : *« Une classe dont le
chiffre est significatif mais dont la question reste ouverte ne s'implémente pas :
elle se pose à l'opérateur »* (§ 7). Elle est posée, pas implémentée.

---

## Décision 5 — le coût accepté de (e), nommé plutôt que passé sous silence

Le tour `PR closed` ne fait pas *rien* : comme tout tour, il peut écrire de la
mémoire et toucher le fil de priorités. C'est **le même résidu que (h)**, et
l'honnêteté exige de le dire ici aussi. Trois raisons de l'accepter pour (e)
quand on le refuse pour (h) :

1. **L'opérateur a déjà rangé (e) du côté à implémenter**, à n=2, et n'a laissé
   ouvert qu'un *« à vérifier »* sur les chemins de clôture — vérifié au
   § *Décision 3*. Pour (h) il a au contraire laissé ouverte la question de la
   valeur du tour.
2. **Le volume par événement est d'un autre ordre** : un merge de PR est un
   événement rare et déjà entièrement tracé en base par
   `upstream_close_handler`, là où un `issue_comment` d'opérateur porte du texte
   humain neuf que seul un tour peut lire.
3. **Le geste est réversible et la population reste comptable.**
   `MIKA_WEBHOOK_PREFILTER` désarme le filtre entier, et chaque tour écarté
   écrit une ligne `webhook_prefilter_skipped` portant sa classe — donc
   `skipped_by_class` chiffrera (e) dès la première fenêtre mesurée, et l'écart
   avec les 193 k / 279 k attendus sera lisible plutôt que perdu.

---

## Phases

### Phase 1 — le filtre de la classe (e)

Dans `crates/mika-agent/src/server/webhook_prefilter.rs` :

1. `PrefilterClass::PrClosedNoLoopTask` + son nom de fil
   `"pr_closed_no_loop_task"` dans `as_str()`.
2. `Candidate::PrClosedNoTask { pr_url: String }` + son nom de fil
   `"pr_closed_no_task"` dans `Candidate::as_str()` (la section registre).
3. Un bras dans `classify(text)` sur `PR_CLOSED_EVENT_PREFIX` (constante
   importée de `mika_common::github_event_format`), qui appelle l'extracteur
   d'URL promu `pub(crate)` ; `None` si l'URL n'est pas extractible — un
   événement dont on ne sait pas de quelle PR il parle n'est pas un candidat,
   il est un tour.
4. Un bras dans `Touched::applies_to` : `Candidate::PrClosedNoTask { .. }` →
   `self.by_any_handler`, rangé avec `GreenDefaultBranch` et `InertLabel`.
5. Un bras dans `decide()` → `decide_pr_closed(pr_url, state)`, qui lit
   `state.pr_has_active_task(&pr_url)` : `Ok(false)` ⇒ `Skip(PrClosedNoLoopTask)` ;
   `Ok(true)` ⇒ le tour a lieu ; `Err(e)` ⇒ `state_unreadable("pr_has_active_task", pr_url, &e)`.
6. Une section dans l'en-tête du module, à la suite des phases 1–3 : la classe,
   la lecture de source qui la justifie, le coût accepté du § *Décision 5*, et
   la raison pour laquelle la mesure 3 ne la conditionne pas.

Dans `crates/mika-agent/src/server/upstream_close_handler.rs` : `extract_pr_url`
passe en `pub(crate)`, avec deux lignes de doc disant qui la lit et pourquoi elle
n'est pas recopiée.

### Phase 2 — les assertions

Sept, inline dans `mod tests` du module, sur le modèle des classes (a)–(d) :

1. `classify` positif : un texte `PR closed` extractible rend
   `Some(PrClosedNoTask { pr_url })` avec la bonne URL.
2. `classify` négatif : un `PR closed` sans URL extractible rend `None` (le tour
   a lieu).
3. `classify` négatif de voisinage : un `PR opened` ne rend **pas** cette
   variante — `PR_OPENED_EVENT_PREFIX` et `PR_CLOSED_EVENT_PREFIX` partagent leur
   tête.
4. `applies_to` : texte touché dans **n'importe quelle** fenêtre ⇒ le candidat
   ne s'applique pas. Le test vérifie que la fenêtre retenue est bien la plus
   stricte, au lieu de le supposer.
5. `decide` écarte : pas de tâche active ⇒ `Skip(PrClosedNoLoopTask)`.
6. `decide` laisse passer : tâche active ⇒ le tour a lieu. **Contrôle négatif
   obligatoire** — sans lui, un filtre qui écarte tout est vert (leçon
   mika#2277 ; même graine, seul l'état diffère).
7. `decide` fail-safe : `pr_has_active_task` en `Err` ⇒ le tour a lieu, et la
   raison est nommée.

### Phase 3 — la halte, écrite où le prochain lecteur la trouvera

Aucune ligne de code. Trois documents :

1. `docs/eval/mika-2680/README.md` — nouveau § *2 bis — qui exécute cette
   commande* : le bac à sable ne monte pas le journal, la cause (bind set de
   `dispatch-lib.sh`), et la conséquence (la mesure est un geste opérateur, ou
   une session hors bac à sable). Plus un § *9 — l'état des quatre classes* avec
   le tableau (e) implémentée / (f) résidu renvoyé / (g) bloquée par la mesure /
   (h) posée à l'opérateur, chacune avec sa raison propre.
2. `docs/solutions/best-practices/un-pre-filtre-de-webhook-se-place-apres-les-handlers-2026-10-07.md`
   — la classe (e) dans la table des classes, et la propriété qui l'autorise
   (le filtre est après les handlers, donc il ne prive aucun handler).
3. Le corps de la PR porte la commande du § 3 prête à coller, et la question de
   (h) à l'opérateur.

---

## Definition of Done

1. `classify()` reconnaît un `PR closed` et rend `Candidate::PrClosedNoTask`,
   par la constante `PR_CLOSED_EVENT_PREFIX` et sans littéral neuf.
2. `decide()` écarte le tour quand, et seulement quand, le texte est intact dans
   la fenêtre `by_any_handler` **et** `pr_has_active_task` rend `Ok(false)`.
3. Une erreur de lecture d'état rend le tour au LLM (AC3 de #2675 inchangé), et
   `state_unreadable` nomme le prédicat et la cible.
4. Le tour écarté écrit une ligne `webhook_prefilter_skipped` portant la classe
   `pr_closed_no_loop_task`, donc `skipped_by_class` le chiffrera.
5. Aucune ré-dérivation : le filtre lit `pr_has_active_task` par
   `PrefilterState`, et ne reparse ni la vérité du merge ni les refs de clôture.
6. Aucune troisième copie d'`extract_pr_url` : le pré-filtre `use` celle de
   `upstream_close_handler`, promue `pub(crate)`.
7. Les sept assertions de la phase 2 passent, dont le contrôle négatif n° 6.
8. `cargo test`, `cargo clippy`, `cargo fmt --check`, `make verify-bundled-skills`
   et `scripts/verify-pipeline.sh origin/main` passent.
9. L'en-tête du module porte la section de la phase (e), et les deux documents
   de la phase 3 sont amendés.
10. **Hors périmètre, nommé et non livré :** l'artefact de la mesure 3
    (`turns-*.jsonl`, `summary-*.json`), pour la raison structurelle du
    § *Décision 2* ; les filtres (f), (g) et (h), écrits comme résultats ; et le
    câblage CI de `make test-measure-webhook-turn-cost` (touche
    `.github/workflows/`, le jeton des pilotes n'a pas le scope `workflow` —
    décision opérateur en attente, voisine de mika#2752).

## Acceptance criteria

- **AC1 — la classe existe et porte son nom sur le fil.** `PrefilterClass::PrClosedNoLoopTask`
  rend `"pr_closed_no_loop_task"` ; `Candidate::PrClosedNoTask` rend
  `"pr_closed_no_task"`. Assertions 1 et 5.
- **AC2 — le filtre écarte le cas mesuré à n=2.** Un `PR closed` au texte intact
  sur une PR sans tâche de boucle active ⇒ aucun tour LLM. Assertion 5.
- **AC3 — le filtre ne mord pas quand la boucle travaille.** Même événement,
  tâche active ⇒ le tour a lieu. Assertion 6, contrôle négatif de même graine.
- **AC4 — fail-safe vers le LLM.** État illisible ⇒ le tour a lieu, et la raison
  est journalisée. Assertion 7.
- **AC5 — le texte touché rend le tour.** Toute fenêtre de `Touched` à `true` ⇒
  le candidat ne s'applique pas ; la fenêtre retenue est `by_any_handler`, la
  plus stricte. Assertion 4.
- **AC6 — zéro divergence programmée.** Aucun nouveau littéral `[GitHub] PR …`,
  aucune troisième copie d'`extract_pr_url`, aucune relecture de la vérité du
  merge. `cargo test -p mika-agent canonical_tokens` vert, en particulier
  `mika2517_the_fallthrough_domain_has_a_single_definition`.
- **AC7 — la halte est lisible sans ce plan.** `docs/eval/mika-2680/README.md`
  porte le § 2 bis (pourquoi aucun pilote dispatché ne peut exécuter le § 3) et
  le § 9 (l'état des quatre classes, une raison par classe). Le prochain lecteur
  ne redécouvre pas le bac à sable à ses frais.
- **AC8 — la population reste comptable.** Le premier `summary-*.json` produit
  par l'opérateur porte `skipped_by_class["pr_closed_no_loop_task"]`, et ce
  chiffre est comparable aux 193 k / 279 k des mesures 1 et 2.

## Fire-Disposition

**Option (a) — exception nommée en allowlist, et l'allowlist est vide à la
livraison.**

Le détecteur livré est la batterie de sept assertions de la phase 2, dont le
chemin de succès est « aucune divergence trouvée » entre la classe déclarée et
la décision rendue. Elle porte **exclusivement** sur du code créé par ce vol —
la variante de `Candidate`, le bras de `classify`, le bras de `decide` — et sur
des états synthétiques construits dans le test. Il n'existe donc, par
construction, **aucune violation préexistante à excepter**, et l'ensemble
d'exceptions est vide.

Vide, pas absent : c'est l'emplacement où la prochaine infraction se déposera
nommée plutôt que tolérée (mika#2323). Deux conséquences d'implémentation :

- **Elle atterrit ARMÉE.** Aucun `#[ignore]`, aucune cible locale : les
  assertions sont inline dans `mod tests` du module, donc dans `cargo test`, donc
  dans le job `Check`. Un détecteur que CI n'appelle pas est l'option (b) sans
  l'avoir déclarée.
- **L'asymétrie est obligatoire.** Une batterie purement positive est satisfaite
  par un filtre qui écarte tout ; une batterie purement négative, par un filtre
  qui n'écarte jamais rien. D'où l'assertion 5 (il écarte le cas visé) **et**
  l'assertion 6 (il ne mord pas quand la tâche est active), de même graine et ne
  différant que par l'état — plus l'assertion 7 pour le terme fail-safe, qui ne
  se prouve pas en invalidant les autres termes avec lui (leçon mika#2277).

Le scan de recensement `mika2517_the_fallthrough_domain_has_a_single_definition`
est un détecteur **préexistant** que ce vol doit laisser vert ; il n'est pas
livré par ce vol et ne reçoit aucune exception (AC6).

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/webhook_prefilter.rs` — production (variante de classe + variante de `Candidate` + noms de fil, bras `classify`, bras `applies_to`, bras `decide` + `decide_pr_closed`) | 75 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — section d'en-tête de la phase (e) | 55 |
| `crates/mika-agent/src/server/webhook_prefilter.rs` — les sept assertions de la phase 2 | 115 |
| `crates/mika-agent/src/server/upstream_close_handler.rs` — `extract_pr_url` en `pub(crate)` + doc du contrat de lecture unique | 12 |
| `scripts/canonical-tokens.tsv` + `crates/mika-agent/src/canonical_tokens.rs` — recensement du nom de fil, si et seulement si le census le réclame | 25 |

Total estimé : 282
Plafond de tours : 200 (loop-substrate)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

Sous les deux bornes. La marge est délibérée : le relevé opérateur du
2026-10-14 mesure une sous-estimation de ×2,25 à ×4,1 sur trois vols consécutifs
de la même soirée, dont ×3,8 sur le vol 1 de **ce** ticket (345 lignes déclarées,
1 314 livrées, arrêt à 201/200 tours). Ce vol livre **une** classe, pas trois,
et renvoie sa moitié mesure à l'opérateur plutôt que de la tenter : c'est la
réponse à ce relevé, pas une coïncidence.

## Fichiers touchés

**Code :**

- `crates/mika-agent/src/server/webhook_prefilter.rs` — la classe, le candidat,
  les trois bras, la section d'en-tête, les sept assertions.
- `crates/mika-agent/src/server/upstream_close_handler.rs` — `extract_pr_url` en
  `pub(crate)`.
- `crates/mika-agent/src/canonical_tokens.rs` + `scripts/canonical-tokens.tsv` —
  seulement si le census réclame le nouveau nom de fil.

**Documentation :**

- `docs/eval/mika-2680/README.md` — § 2 bis (qui peut exécuter la commande) et
  § 9 (l'état des quatre classes).
- `docs/solutions/best-practices/un-pre-filtre-de-webhook-se-place-apres-les-handlers-2026-10-07.md`
  — la classe (e) dans la table, et la propriété « après les handlers » qui
  l'autorise.

**Non touchés, et c'est une décision :**

- `.github/workflows/ci.yml` — hors périmètre déclaré du corps du ticket, le
  jeton des pilotes n'a pas le scope `workflow`.
- `crates/mika-agent/src/server/milestone_context_handler.rs` — sa copie
  d'`extract_pr_url` est préexistante ; la dédupliquer est un ticket de suivi,
  pas ce vol.
- `crates/mika-agent/src/server/handlers.rs` — le site d'appel du pré-filtre est
  déjà générique sur `Candidate` ; une nouvelle variante n'y change rien.

## Renvois explicites

| à renvoyer | pourquoi ici et pas ailleurs |
|---|---|
| **La mesure 3 elle-même** — exécuter le § 3 du protocole sur une fenêtre ≥ 4 h 30, borne basse ≥ `2026-10-16T13:01:00Z`, avec activité PR, puis committer `turns-*.jsonl` + `summary-*.json` | geste **opérateur** : le journal n'est pas monté dans le bac à sable (§ *Décision 2*) |
| **La classe (g)** `check_suite success` sur branche de PR côté mika-qa | la forme est connue ; seul manque le `skipped_by_class` qui soustrait (b). Première classe à implémenter dès que l'artefact existe |
| **La classe (h)** `issue_comment` / `issues closed` sans tâche de boucle — 79 % de la fenêtre de la mesure 2 | la question de l'opérateur du 2026-10-08 est ouverte : que doit faire ce tour ? Protocole § 7 : elle se pose, elle ne s'implémente pas |
| **Le résidu de (f)** — `pr_review` sur PR fermée au texte **touché** par le `verdict_handler` | écarter ce tour décide que la renotification opérateur du disjoncteur n'a pas lieu ; le corps du ticket renvoie cette question à un ticket à part (voisin de #2679) |
| **La déduplication d'`extract_pr_url`** entre `upstream_close_handler` et `milestone_context_handler` | duplication préexistante, non créée par ce vol ; la réparer ici ajouterait ~145 lignes et un scan de source à un vol qui tient sa borne de peu |
| **Le câblage CI de `make test-measure-webhook-turn-cost`** | hors périmètre déclaré ; `.github/workflows/`, scope `workflow` absent du jeton (voisin de mika#2752) |

Refs #2675, #2705, #2747
