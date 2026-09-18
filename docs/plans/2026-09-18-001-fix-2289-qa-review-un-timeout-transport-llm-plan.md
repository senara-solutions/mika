# mika#2289 — un tour mort sur erreur LLM ne pose aucun verdict : le filet de mika#2276 M2 ne voit que la branche `Ok`

- **Ticket :** senara-solutions/mika#2289
- **Priorité :** p2 (substrate, loop)
- **Branche :** `bug/2289/qa-review-un-timeout-transport-llm`
- **Lignage :** #2276 M2 (le filet `deadline_verdict`, dont ce travail est le frère manquant), #2015 (`Transport` retryable sans condition), #2189 (`LlmTimeoutBudget`, qui borne la chaîne), #2362 (`retrying` / `exhausted` rendus honnêtes), #2334 + #2347 (le réconciliateur QA, qui ne couvre pas cette population), #2036 (la recovery A2A côté client), #1823 (le retry `_arch_ask`, conditionné à `UNPARSED`), #2296 (le `.content` vide, à ne pas confondre avec un défaut de transport)

---

## Contexte

Le ticket rapporte deux morts, sur deux maillons, avec la même cause apparente et
le même effet :

- **Maillon A — la revue QA** (corps du ticket, mesuré 2026-09-11 10:25 sur la
  PR #2288) : le tour mika-qa meurt sur un timeout transport OpenRouter, aucun
  verdict n'est posté, le merge reste bloqué jusqu'à une relance CI manuelle
  (run 34577920272).
- **Maillon B — le `_arch_ask` du groom** (commentaire opérateur du 11/09
  11:29Z) : l'appel a2a vers mika-arch meurt sur le même défaut (kimi-k2.5, 2×
  « body read failed mid-stream, operation timed out », 11:21–11:25Z) →
  `PIPELINE_INCOMPLETE`, pas de `PLAN_GROOMED`.

La piste proposée est double : *« envelopper l'appel LLM de la revue QA d'un
retry sur erreur transport ; ou re-déclencher le tour QA à la détection d'un tour
mort sans verdict posté »*.

**La première moitié de cette piste est déjà en place et n'aurait rien changé ;
la seconde est le remède, et elle a déjà une forme dans ce dépôt.** Ce plan
établit les deux faits, puis construit le remède sur la forme existante.

---

## Ce qui est établi, et comment le vérifier

### E1 — Le retry sur erreur transport existe déjà, sur le rail qui porte les deux incidents

`LlmError::Transport` est retryable **sans condition** depuis mika#2015, et la
chaîne tourne dans la boucle d'attempts du rail OpenAI-compatible :

```
crates/mika-common/src/llm/openai.rs:387   for attempt in 0..max_attempts {
crates/mika-common/src/llm/openai.rs:529   if attempt + 1 < max_attempts && e.is_retryable() {
```

`max_attempts` vient de `LlmTimeoutBudget::max_attempts` (mika#2189) et vaut
`floor(enveloppe / plafond)`, borné par le `MAX_RETRIES + 1` du rail. Les deux
incidents du ticket passent par ce rail (OpenRouter des deux côtés).

**Conséquence :** ajouter « un retry sur erreur transport autour de l'appel LLM »
consisterait à ajouter une seconde fois ce qui tourne déjà. Le remède demandé ne
peut pas être le remède. Vérification :
`grep -n "is_retryable\|for attempt in" crates/mika-common/src/llm/openai.rs`.

### E2 — Maillon A : le défaut est que l'ÉPUISEMENT de cette chaîne est muet côté PR

Quand la chaîne s'épuise, l'erreur remonte par un `?` unique :

```
crates/mika-agent/src/agent_loop/mod.rs:1400   let response = llm_result?;
```

Elle sort donc de `run_loop`, puis de `run_agent`, et atterrit ici :

```rust
// crates/mika-agent/src/server/handlers.rs:1569-1572
Err(e) => {
    error!(error = %e, "agent loop failed");
    let _ = sender_arc.send(AGENT_ERROR_REPLY).await;
}
```

Une notification part sur le canal de réponse ; **rien n'est posté sur la PR.**
C'est mot pour mot le symptôme que mika#2276 M2 a fermé pour la deadline —
*« Telegram notifié, PR muette »* — resté ouvert pour l'erreur.

Et le filet existant ne peut structurellement pas voir cette branche : il est
appelé **uniquement** depuis la branche `Ok` (`handlers.rs:1538`) et sort
immédiatement si le tour n'a pas dépassé sa deadline :

```rust
// crates/mika-agent/src/server/handlers.rs:1131
if output.deadline_exceeded.is_none() || parse_pr_target(&req.text).is_none() {
    return;
}
```

La raison est de forme, pas d'oubli : le signal de la deadline voyage dans
`AgentOutput.deadline_exceeded`, et sur la branche `Err` **il n'y a pas
d'`AgentOutput` du tout**. Les deux causes de mort n'ont aujourd'hui aucun point
de rencontre.

> *« Le tour a abouti » et « le tour a conclu » étaient deux faits différents que
> rien dans le code ne séparait.* — `deadline_verdict.rs:18`. Ce plan ajoute la
> troisième : **« le tour est mort »**, qui n'était séparée d'aucune des deux.

### E3 — Le réconciliateur QA (mika#2334/#2347) ne rattrape pas cette population, et son code le dit

`select_prs_needing_review` exclut toute PR portant déjà une demande de revue
pour `mika-platform-qa` :

```rust
// crates/mika-agent/src/qa_review_reconcile.rs:291-298
if pr.review_requests.iter().filter_map(|r| r.login.as_deref())
    .any(|l| is_login(l, REVIEWER_FORGE_LOGIN)) { return None; }
```

commenté en toutes lettres : *« idempotence : une demande déjà posée sort la PR
de la population »* (`qa_review_reconcile.rs:254`). Or GitHub ne retire une
demande de revue que lorsque la revue est **soumise**. Donc :

| état de la PR | rattrapée par mika#2334 ? |
|---|---|
| aucune demande, aucune revue (le tour est né d'un `opened`) | **oui**, après `MIN_AGE` (1 h), ≤ `MAX_ATTEMPTS` (2) |
| demande posée, tour mort sans revue | **non — exclue à jamais** |

Le second cas est exactement celui du ticket dès qu'un `review_requested` est
dans le circuit — y compris celui que le réconciliateur pose lui-même. Le
premier cas n'est couvert qu'avec 1 h de latence et deux tentatives.

**Ce filet est donc partiel par construction et le restera :** son terme
d'idempotence est ce qui l'empêche de doubler les revues, et le relâcher
rouvrirait la classe #886. Le remède doit être en amont, dans le moteur.

### E4 — Maillon B : quatre `_arch_ask`, aucun retry sur échec de transport

Le seul retry existant (mika#1823, `dispatch-lib.sh:5286-5345`) est conditionné à
une disposition **`UNPARSED`** — c'est-à-dire à une réponse *reçue mais mal
formée*. Un échec de `_arch_ask` lui-même (code de retour non nul) fait
`return 1` immédiat, aux quatre sites :

| ligne | pass | traitement de l'échec |
|---|---|---|
| `dispatch-lib.sh:5288` | first-pass | `_groom_warn "first-pass _arch_ask failed"; return 1` |
| `dispatch-lib.sh:5308` | first-pass (retry UNPARSED) | `_groom_warn "retry _arch_ask failed"; return 1` |
| `dispatch-lib.sh:5351` | second-pass | `_groom_warn "second-pass _arch_ask failed"; return 1` |
| `dispatch-lib.sh:5400` | second-pass après revise | `_groom_warn "second-pass _arch_ask failed (after revise)"; return 1` |

Vérification : `grep -n "_arch_ask" skills/bundled/_shared/dispatch-lib.sh`.

### E5 — Le client A2A sait déjà nommer les cas, et dit lui-même que rejouer est le bon geste

mika#2036 a doté `mika ask` d'une lecture de récupération après échange échoué,
avec une taxonomie explicite (`crates/mika-cli/src/remote_ask.rs:133-148`) :

- `Recovered` — la réponse existait, elle est rendue. **Déjà traité.**
- `StillRunning` — *« The answer does not exist yet; **retrying is the right
  move**. »* ← personne ne retente.
- `Ended` / `NoTask` / `Unavailable` — les trois autres issues.

Le maillon B n'a donc pas besoin d'inventer une discrimination : elle existe, elle
est nommée, et le geste qu'elle recommande n'est simplement câblé nulle part.

### E6 — Ce que ce plan n'a pas pu vérifier

`gh` n'est pas authentifié dans ce worktree : le corps et les deux commentaires du
ticket ont été lus depuis le contexte de dispatch, pas depuis l'API. Les traces de
production (PR #2288, run 34577920272, les deux `body read failed mid-stream`)
sont **reprises du ticket, non re-mesurées**. Tout ce qui précède, hors les dates
et les identifiants d'incident, est établi par lecture du code seul.

---

## Ce que ce plan déplace par rapport au ticket

| le ticket propose | ce que la lecture du code impose |
|---|---|
| « envelopper l'appel LLM de la revue QA d'un retry sur erreur transport » | **Refusé : le retry existe (E1).** L'ajouter une seconde fois masquerait le vrai défaut. |
| « distinguer transport-error d'un vrai verdict/refus » | **Repris, mais déplacé** : la distinction utile n'est pas *transport vs verdict*, c'est **le tour a conclu / le tour est mort** (A), et **le transport a échoué / la réponse est vide** (B, où mika#2296 a déjà payé la confusion inverse). |
| « re-déclencher le tour QA à la détection d'un tour mort sans verdict posté » | **Repris comme remède du maillon A**, sous la forme que ce dépôt a déjà choisie pour la cause sœur : le moteur **pose le verdict** plutôt que de relancer un tour (voir D2). |
| « le retry doit couvrir DEUX maillons » (commentaire 1) | **Repris intégralement.** Les deux maillons sont traités, par deux remèdes différents — parce que ce sont deux défauts différents sous un symptôme commun (voir D1). |

---

## Décisions

### D1 — Deux maillons, deux remèdes, et c'est le fond du ticket plutôt qu'une commodité

Le commentaire opérateur lit A et B comme un seul défaut (« le retry-sur-transport
doit couvrir DEUX maillons »). Le code dit autre chose :

- **A est un défaut de report** : le tour meurt, la mort est correcte et
  inévitable, mais elle ne laisse aucune trace là où la décision se prend (la PR).
  Un retry de plus n'y changerait rien — la chaîne est déjà épuisée.
- **B est un défaut d'absence de retry** : l'appelant abandonne à la première
  erreur de transport alors qu'un second essai est bon marché, sûr, et
  explicitement recommandé par le client lui-même (E5).

Les traiter par un seul mécanisme demanderait soit d'ajouter un retry inutile en
A, soit de poser un verdict de secours en B — où il n'y a pas de PR. **Un seul
remède pour deux défauts serait faux des deux côtés.**

### D2 — Maillon A : généraliser `deadline_verdict`, ne pas écrire un second filet

Le module `server::deadline_verdict` porte déjà tout ce dont la branche `Err` a
besoin :

- la grammaire de la PR, avec **un seul lecteur** (`parse_pr_target`, qui délègue
  aux deux regex existantes plutôt que de les recopier — la leçon mika#2158) ;
- l'anti-double-post à deux couches indépendantes (registre `pr_reviews_posted`
  par session + 422 lu comme succès idempotent) ;
- la taxonomie des issues (`DeadlineVerdictOutcome`, cinq variantes) ;
- l'événement opérateur `qa_deadline_verdict` avec son champ `outcome`.

Écrire un second filet pour l'erreur rejouerait exactement la divergence que
mika#2158 et mika#2120 ont chacun dû fermer une fois : deux lecteurs d'une même
grammaire qui cessent de répondre pareil. **Le module est donc élargi, pas
dupliqué** — son entrée passe de « le tour a-t-il dépassé sa deadline ? » à
« le tour a-t-il conclu ? ».

### D3 — Le signal d'entrée devient un enum fermé, et le compilateur force la décision

Aujourd'hui `DeadlineVerdictInput.overrun: Option<DeadlineOverrun>` mélange deux
questions dans un `Option` : *y a-t-il eu dépassement ?* et *le filet
s'applique-t-il ?* Avec une troisième façon de mourir, l'`Option` ne suffit plus.

Le champ devient un enum à variantes explicites — au minimum `Concluded`,
`DeadlineExceeded { steps_completed }`, `Failed { error_class, detail }` — **sans
bras `_ =>` dans le `match` qui le consomme**, sur le modèle de
`webhook_queue_v2::coalescing_key` et de `tools/mod.rs::dispatch_substrate_diagnostic`.
Une quatrième façon de mourir devra alors *décider* au lieu de tomber dans un
silence par défaut : c'est la seule protection qui survit à l'oubli, et ce ticket
existe précisément parce qu'une deuxième façon de mourir est passée inaperçue.

### D4 — Le filet fire sur TOUTE erreur du loop, pas seulement sur la classe transport

Le ticket dit « erreur transport ». La classe est **reportée** (corps du verdict,
champ du journal) mais n'est **pas** une condition. Trois raisons :

1. `hold[review]` signifie *« ce tour n'a pas conclu, un humain regarde »* — un
   sens qui ne dépend pas de la cause. `deadline_verdict` ne discrimine pas non
   plus la cause du dépassement.
2. Restreindre à `transport` crée une **seconde population muette** (erreurs de
   parse, de provider, de configuration) indiscernable de la première depuis
   l'extérieur : PR muette, merge bloqué. C'est le défaut du ticket, reconstruit
   sous un autre nom.
3. La classe d'erreur est déjà nommable sans heuristique : `LlmError` est un enum
   et `downcast_ref` traverse la chaîne `anyhow` — la même technique que
   mika#2179 emploie pour `callback_delivery_failed`, jamais un `contains()` sur
   le message rendu.

**Coût nommé :** une panne systémique (budget invalide, provider absent) poserait
un `hold[review]` par webhook au lieu d'un silence par webhook. C'est borné par
les deux couches d'anti-double-post, et un `hold[review]` ne fait que notifier
l'opérateur en laissant la tâche `in_progress` — le contrat de `verdict_handler`.
Un signal bruyant sur une panne vaut mieux qu'un silence sur une panne.

### D5 — Poser `hold[review]` ferme le rattrapage automatique de mika#2334, et c'est assumé

Une fois une revue postée par `mika-platform-qa`, la PR sort définitivement de la
population du réconciliateur (terme « aucune revue de `REVIEWER_FORGE_LOGIN` »,
`qa_review_reconcile.rs:299-306`). Le filet **remplace** donc un rattrapage
partiel et différé par une notification immédiate.

C'est le bon échange, et il est déjà celui de mika#2276 M2 : le réconciliateur ne
couvrait de toute façon pas la population « demande déjà posée » (E3), et son
budget est de deux tentatives. Mais l'échange doit être écrit, sans quoi le
prochain lecteur conclura que le filet a cassé le réconciliateur.

### D6 — Maillon B : un seul enrobage, jamais quatre politiques

Les quatre sites `_arch_ask` (E4) passent par **un unique** helper de retry. Quatre
sites retriant chacun pour son compte est la forme littérale de la classe que
`grooming_marker` (mika#2158) et le prédicat de callout (mika#2120) ont dû fermer :
des lecteurs d'une même question qui divergent en silence.

### D7 — Maillon B : rejouer seulement ce qui est rejouable, et le retry est idempotent par nature

Le tour architecte est en **lecture seule** : mika-arch porte le denylist
`MIKA_ARCH_DISABLED_TOOLS` (#811), donc un rejeu ne peut produire aucun effet de
bord observable. Le rejeu est sûr — reste à ne pas le gaspiller.

**On rejoue** : échec de transport, `AGENT_BUSY` (`-32000`, mika#2163 — le code
porte `retry_after_ms`, fait pour être respecté), `Recovery::StillRunning`.
**On ne rejoue pas** : `.content` vide (mika#2296 — un défaut de budget de sortie,
qu'un rejeu reproduira à l'identique, et dont `_groom_warn_empty_content` a déjà
payé la confusion), `.metadata.session_id` absent (enveloppe incomplète),
disposition `UNPARSED` (déjà couvert par mika#1823, avec son propre prompt
correctif — les deux boucles se composent, elles ne se remplacent pas).

### D8 — Maillon B : le budget de retry est borné globalement, pas par site

`_arch_ask` n'est sous aucun `timeout` shell ; c'est le client A2A qui borne, à
`DEFAULT_TIMEOUT = 600 s` avec plancher `>= MIKA_AGENT_TOTAL_TIMEOUT_SECS`
(`crates/mika-a2a/src/client.rs:24`, mika#2297). Un retry par site sur quatre
sites ferait passer le pire cas d'un groom de 4 × 600 s à 8 × 600 s — **80
minutes**, pour une boucle qui n'a aucune borne globale en temps
(`_iterate_groom_loop`, `dispatch-lib.sh:5226`).

Le budget est donc **un compteur partagé par invocation de `_iterate_groom_loop`**,
pas une tentative par site : au plus un rejeu pour toute la convergence, valeur
réglable par variable d'environnement avec la lecture à trois paliers de la maison
(absent/vide → défaut ; illisible, `0` ou négatif → défaut + WARN). Le `0` ne
désarme pas — c'est le rôle d'un interrupteur nommé.

---

## Travail

### Volet A — le moteur pose un verdict quand le tour meurt

**A1. Élargir le signal.** Remplacer `DeadlineVerdictInput.overrun:
Option<DeadlineOverrun>` par l'enum fermé de D3. Le `match` qui le consomme
(`deadline_verdict.rs:231-233`) devient exhaustif, sans bras générique.

**A2. Classer l'erreur sans heuristique de chaîne.** Un classifieur qui lit la
*variante* de `LlmError` via `downcast_ref` sur la chaîne `anyhow` (modèle
mika#2179 : `transport_timeout`, `transport`, `http_<status>`, `parse`,
`provider`, `unsupported`, `other`). La seule chaîne consultée reste celle qui
sépare un timeout d'une connexion refusée à l'intérieur de `Transport`.

**A3. Généraliser le corps du verdict.** `build_verdict_body` prend le nouvel enum
et rend un corps qui nomme la cause réelle — « coupé par son enveloppe après N
pas » ou « mort sur `<classe>` » — en conservant la ligne canonique
`VERDICT: hold[review]` inchangée (aucune nouvelle branche dans
`verdict_handler`, donc pas de gate CODEOWNERS : l'argument Q1 de mika#2276 vaut
ici mot pour mot).

**A4. Appeler le filet depuis les deux branches.** Restructurer le `match
agent::run_agent(...)` de `run_agent_for_message` (`handlers.rs:1526-1573`) pour
que le filet soit invoqué sur `Ok` **et** sur `Err`, avant l'envoi sur le canal
de réponse dans les deux cas — l'ordre déjà retenu en `Ok` et pour la même
raison : le symptôme est que la notification part et que la PR reste muette. Les
trois appelants de `run_agent_for_message` (`handlers.rs:479`, `:898`, `:1048`, ce
dernier étant le drain worker de la file bornée mika#1870) en héritent sans
modification.

**A5. Garde structurelle.** Un test de scan de source refusant qu'une branche de
sortie de `run_agent` dans `run_agent_for_message` n'appelle pas le filet.
Motivation écrite dans le test : **une régression ici ne rend aucune décision
fausse, elle rend une mort muette** — toutes les assertions existantes restent
vertes pendant que la PR redevient silencieuse. C'est la classe que
`mika2342_every_llm_call_is_wrapped_in_a_timeout` et
`mika2131_exclusion_skips_never_return_to_an_uncollected_debug` couvrent déjà,
chacune pour la même raison.

**A6. Observabilité.** Le champ `outcome` de `qa_deadline_verdict` gagne les
valeurs de la nouvelle cause ; un champ `cause` (`deadline` | `error`) et un champ
`error_class` séparent les deux populations. **Elles doivent rester comptables
séparément** : confondre « la QA n'a pas eu le temps » et « la QA est morte »
ferait disparaître le signal que mika#2276 a construit. Le nom d'événement est
conservé (un seul site d'écriture, greps et historique préservés) ; si la
sémantique élargie le rend trompeur, le renommage est une décision à prendre
explicitement, pas un effet de bord.

### Volet B — `_arch_ask` retente une fois sur un échec rejouable

**B1. Un helper unique** enveloppant `_arch_ask`, appliquant D7 (quoi rejouer) et
D8 (budget partagé), avec un backoff court et fixe. Les quatre sites de E4 passent
par lui ; aucun n'appelle plus `_arch_ask` directement.

**B2. Discrimination lisible.** Le helper distingue et **nomme** ce qu'il a vu :
échec de transport, refus `AGENT_BUSY`, réponse vide (mika#2296), enveloppe
incomplète. `_groom_warn_empty_content` reste le message du cas non rejouable et
ne doit pas être atteint par un rejeu.

**B3. Interrupteur + budget.** Une variable pour le nombre de rejeux par
convergence (défaut : 1) et une pour désarmer, toutes deux en lecture à trois
paliers. Désarmé, le comportement est **verbatim** celui d'avant ce plan.

**B4. Épuisement visible.** Quand le budget est consommé, `GROOM_LOOP_FAILURE_REASON`
nomme le rejeu dépensé, de sorte que le `PIPELINE FAILURE` final
(`dispatch-lib.sh:6686`) distingue « transport mort une fois » de « transport mort
deux fois » — sans quoi le correctif serait invérifiable depuis l'extérieur.

**B5. Test shell.** Étendre `skills/bundled/_shared/test-dispatch-lib.sh` : un
échec de transport suivi d'un succès converge ; une réponse vide **n'est pas**
rejouée ; le budget est bien partagé entre les quatre sites et non multiplié par
quatre.

---

## Contrat de vérification

| # | Ce qui est vérifié | Comment |
|---|---|---|
| V1 | Un tour QA mort sur erreur LLM poste `VERDICT: hold[review]` | Test de la production : `run_agent_for_message` avec un provider mock échouant en `Transport`, sur un événement PR ; le `poster` injecté reçoit la requête |
| V2 | Un tour **conclu** ne poste rien | Contrôle négatif, même harnais, provider nominal → `NotApplicable("turn_completed")` |
| V3 | Un tour mort sur un événement **non-PR** ne poste rien | Contrôle négatif → `NotApplicable("not_a_pr_event")` |
| V4 | Aucun double-post | Deux passages dans une session ; second → `AlreadyReviewed`. 422 → `AlreadyPostedUpstream` |
| V5 | La branche deadline est **inchangée** | Les tests existants de `deadline_verdict` passent sans modification de leurs assertions |
| V6 | Une quatrième façon de mourir ne compile pas en silence | Ajout d'une variante à l'enum de D3 → erreur de compilation au `match` |
| V7 | `_arch_ask` rejoue un échec de transport | Test shell, `mika` bouchonné : échec puis succès → convergence |
| V8 | `_arch_ask` ne rejoue **pas** une réponse vide | Contrôle négatif : `.content` vide → un seul appel, `_groom_warn_empty_content` |
| V9 | Le budget de rejeu est global à la convergence | Test shell : deux sites échouant → un seul rejeu au total |
| V10 | Désarmé, le comportement est verbatim l'ancien | Test shell avec l'interrupteur à `0` |

**Sonde post-déploiement, 72 h, avec sa halte.**
`grep qa_deadline_verdict $MIKA_SPIRIT_LOG_FILE | jq 'select(.cause == "error")'` —
chaque ligne est une PR qui serait restée muette. **Régime attendu : rare.** Un
volume soutenu signifie que les tours meurent souvent et que **ce filet masque une
panne amont** (provider, budget, égress) : c'est cette panne qu'il faut traiter,
pas ce filet qu'il faut élargir. Le filet n'est pas un chemin.

**Halte explicite :** si une PR reste muette alors que `qa_deadline_verdict` ne
porte aucune ligne pour elle, **ne pas élargir le filet** — le tour est passé par
un assembleur que le filet ne traverse pas (la voie silent, hors périmètre ci-
dessous), et établir lequel vient d'abord.

Côté B : `grep "_arch_ask" $MIKA_SPIRIT_LOG_FILE` et les traces `dispatch-lib` sur
les lignes de rejeu — un rejeu qui échoue systématiquement en second essai dit
que le défaut n'est pas transitoire, et ce n'est pas le budget qu'il faut monter.

---

## Definition of Done

- Les volets A et B sont implémentés selon D1–D8.
- V1–V10 passent.
- `cargo test`, `cargo clippy`, `cargo fmt --check` verts.
- `make verify-bundled-skills` vert (le volet B touche `_shared/`).
- Les surfaces opérateur (A6, B4) sont documentées dans le `CLAUDE.md` racine et
  dans `crates/mika-agent/CLAUDE.md`, aux sections des filets qu'elles étendent.
- Le corps de PR nomme explicitement D4 (fire sur toute erreur) et D5 (fermeture
  du rattrapage mika#2334) comme des coûts assumés, pas comme des effets de bord.

## Acceptance criteria

Le ticket ne porte pas de section `## Acceptance criteria` ; les critères suivants
sont dérivés de son « Attendu » et du contrat de vérification ci-dessus.

- **AC1** — Une erreur LLM (timeout transport, 5xx, connexion coupée) pendant un
  tour traitant une PR ne laisse plus la PR sans verdict : le moteur poste
  `VERDICT: hold[review]` en nommant la classe d'erreur. *Un timeout transport
  n'est pas un verdict — mais il ne doit pas non plus être un silence.*
- **AC2** — Un tour qui **conclut** normalement ne poste aucun verdict de secours,
  et la branche deadline de mika#2276 M2 est inchangée dans son comportement
  observable.
- **AC3** — Aucun double-post : deux couches indépendantes (registre par session,
  422 lu comme succès idempotent), vérifiées séparément.
- **AC4** — Le filet est atteint depuis **toutes** les branches de sortie de
  `run_agent` dans `run_agent_for_message`, et une garde structurelle refuse
  qu'une branche future y échappe.
- **AC5** — `_arch_ask` retente **une fois** un échec rejouable (transport,
  `AGENT_BUSY`, `StillRunning`) et **ne retente pas** un échec non rejouable
  (`.content` vide, enveloppe incomplète, `UNPARSED`).
- **AC6** — Le budget de rejeu de B est partagé par convergence, réglable, et
  désarmable en restaurant verbatim le comportement antérieur.
- **AC7** — Les deux causes de mort (`deadline`, `error`) restent comptables
  séparément dans le journal et ne sont pas fusionnées sous un compteur unique.
- **AC8** — Aucune valeur de budget LLM (plafond, enveloppe, `max_attempts`) n'est
  modifiée par ce travail. Il rend une mort visible ; il ne la rend pas plus rare.

---

## Hors périmètre, délibérément

- **La voie silent / callback.** Un tour callback mort sur erreur LLM souffre du
  même défaut, mais sa généralisation du filet est **déjà portée par mika#2368**
  (nommée telle quelle dans `crates/mika-agent/CLAUDE.md`, § post-condition 6a :
  *« the engine-side `hold[review]` net that would cover that case … is
  mika#2368 »*). Empiéter dessus doublonnerait un ticket ouvert. Ce plan traite la
  voie webhook/conversationnelle, qui est celle du symptôme mesuré.
- **La cause des timeouts OpenRouter** (classe #2280, « 5e/6e occurrence du jour »
  selon le commentaire). Ce travail rend la mort visible et borne le groom ; il ne
  fait pas disparaître la panne fournisseur.
- **Le littéral `120 s` du rail Anthropic** (`claude.rs`), nommé et laissé ouvert
  par mika#2189 puis mika#2342. Aucun des deux incidents du ticket n'est sur ce
  rail.
- **Relancer le tour QA** plutôt que poser un verdict. La piste du ticket
  l'évoque ; ce plan choisit la forme que mika#2276 M2 a déjà retenue pour la
  cause sœur — un tour relancé peut mourir de la même façon, un verdict posé ne
  le peut pas. Si la mesure post-déploiement montre que `hold[review]` laisse trop
  de PRs en attente humaine, la relance est un ticket de suivi, avec sa propre
  borne de re-déclenchement (modèle `MIKA_AUTO_PULL_MAX_REDRIVES`).
- **Relâcher le terme d'idempotence de mika#2334** pour couvrir la population
  « demande posée, pas de revue ». Ce terme est ce qui empêche la revue double
  (classe #886) ; le remède est en amont, pas dans ce filtre.

---

## Risques et incertitudes

1. **La restructuration du `match` de `run_agent_for_message` touche un chemin
   chaud** — trois appelants, dont le drain worker de la file bornée. Le risque
   est de déplacer l'envoi sur le canal de réponse ou de modifier la libération du
   verrou d'agent. Mitigation : le filet est ajouté *avant* l'envoi dans les deux
   branches sans toucher à l'envoi lui-même ni au `drop(_lock)`.
2. **D4 (fire sur toute erreur) peut se révéler trop large** si une classe
   d'erreur fréquente et bénigne existe que je n'ai pas mesurée. Mitigation : la
   classe est journalisée dès le premier jour, donc la restriction est une décision
   fondée sur une mesure plutôt qu'une prudence a priori — et elle se prend dans un
   sens facile (ajouter une condition), pas dans l'autre.
3. **Le budget de rejeu de B (D8) est posé sans mesure de la distribution des
   échecs transport de `_arch_ask`.** Un seul rejeu est le choix conservateur ; le
   ticket rapporte deux occurrences consécutives (11:21–11:25Z), ce qui suggère que
   **un rejeu n'aurait pas suffi ce jour-là**. C'est assumé : un rejeu supprime la
   classe transitoire, et une rafale de deux minutes est une panne fournisseur qui
   relève de la classe #2280, pas d'un budget plus large.
4. **`gh` non authentifié dans ce worktree** (E6) : les identifiants d'incident du
   ticket n'ont pas été re-vérifiés contre l'API. Aucune décision de ce plan n'en
   dépend — toutes s'appuient sur le code.
