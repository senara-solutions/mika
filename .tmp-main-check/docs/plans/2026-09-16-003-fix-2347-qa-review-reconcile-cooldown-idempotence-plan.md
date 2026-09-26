# Plan : le réconciliateur de revues ne doit pas pouvoir redemander une revue indéfiniment (mika#2347)

**Ticket :** mika issue#2347 — `qa_review_reconcile rejoue les revues → enveloppe 600 s crevée`
**Labels :** non lus — `gh` n'est pas authentifié dans le worktree de grooming. Le corps du ticket a été fourni intégralement dans le brief de dispatch ; les commentaires ne sont pas lisibles d'ici.
**Type :** issue (bug de substrat de boucle — la revue QA cesse de produire des verdicts)
**Palier de priorité :** p1 (déclaré par le ticket) — le churn consomme l'enveloppe agent et fait poster des `hold[review]` vides à la place des verdicts.
**Bearing Prime (ratifié, repris tel quel) :** `MIKA_AGENT_TOTAL_TIMEOUT_SECS` ne bouge pas. Le correctif est côté `qa_review_reconcile`.
**Fichiers principaux :** `crates/mika-agent/src/qa_review_reconcile.rs`, `crates/mika-agent/tests/eval/test_qa_review_reconcile_2347.rs` (nouveau), `crates/mika-agent/tests/eval/test_qa_review_reconcile_2334.rs` (mise à jour de contrat), `crates/mika-agent/tests/eval.rs`, `CLAUDE.md`, `crates/mika-agent/CLAUDE.md`

---

## Problème

Depuis que `qa_review_reconcile` est effectif (mika#2341, restart du 2026-09-16 à 16:30:48), les revues QA de #2343/#2344/#2345 atteignent la limite de leur enveloppe et postent des `hold[review]` « aucune conclusion » (le filet mika#2276 M2) au lieu de verdicts. Le ticket impute le churn au réconciliateur : il rejoue des revues et en fait tourner deux en parallèle sur le même agent.

Le remède demandé tient en trois points : idempotence par (PR, head SHA), cooldown après verdict, sérialisation à une revue active par agent.

## Mesures — lues dans le code le 2026-09-16, depuis le worktree de grooming

Quatre faits. Trois confirment le remède ; le quatrième rectifie une partie du diagnostic et fait naître un critère d'attribution.

**M1 — rien ne borne le nombre de re-poses. C'est le défaut réel, et il est visible dans le code seul.** `reconcile_qa_review_requests` (`qa_review_reconcile.rs:449`) ne consulte aucun état local : les seuls termes d'idempotence sont *l'absence de demande* et *l'absence de revue* pour `REVIEWER_FORGE_LOGIN`, deux états **GitHub qui n'existent qu'après aboutissement de la revue**. Une PR dont le tour de revue meurt sans rien poster retombe donc dans la population au tick suivant, identique à elle-même. Avec le cron `0 */15 * * * *` et `MAX_PER_TICK_DEFAULT = 3` (`:106`), cela fait **jusqu'à 96 re-demandes par jour et par PR**, pendant les sept jours de `MAX_AGE_DEFAULT_SECS`. L'audit `qa_review_reconciled` est écrit (`:570`) mais **jamais relu** : le ledger existe et ne décide rien.

**M2 — le scan peut demander plus de revues que l'agent ne peut en produire.** Trois poses par tick, quatre ticks par heure : **12 demandes/heure**. À une enveloppe de 600 s par revue et une exécution sérialisée (M3), la capacité de mika-qa plafonne à **6 revues/heure** — et le trafic nominal (`opened`, `synchronize`, `review_requested`, fan-out `check_suite` de mika#1711) s'ajoute par-dessus. Le cap par tick, seul, autorise déjà une file qui croît sans borne. C'est une arithmétique, pas une intuition.

**M3 — « au plus une revue active par agent » est déjà un invariant du moteur, et il n'est pas tenu par le réconciliateur.** Le chemin `/message` passe par la file bornée mika#1870, drainée par **un seul worker par agent** qui prend `agent_lock` de façon bloquante (`server/state.rs:38`, `handlers.rs::run_agent_for_message` qui reçoit le garde en paramètre) ; `/a2a/{agent}` attend sur le même mutex depuis mika#2163 ; mika-qa est un agent bien connu, initialisé au boot, donc `resolve_agent` prend toujours le chemin rapide et il n'existe qu'un `AgentState` — donc qu'un `agent_lock`. **Le réconciliateur ne peut pas, par construction, faire tourner deux tours en parallèle** : il ne déclenche rien directement, il pose un relecteur et l'événement `review_requested` retombe dans la même file. Ce qu'il peut faire — et fait — c'est en mettre plusieurs en attente.

**M4 — les preuves horaires du ticket ne sont pas compatibles avec le réconciliateur comme source unique du churn.** Deux `hold[review]` sur #2344 à 11 minutes d'écart (15:49 et 16:00) ne peuvent pas venir de lui : son intervalle est de 15 minutes, et surtout le premier `hold` **est une revue postée**, ce qui sort la PR de la population dès le tick suivant. Le même raisonnement vaut pour #2343 (15:25 puis 16:12). Les autres déclencheurs d'une revue QA restent en lice — `pull_request.synchronize` (route vers mika-qa sans filtre, `mika-gateway/src/github.rs:337`), le fan-out `check_suite.completed(success)`, un rejeu de file — et aucun d'eux n'est dans le périmètre de ce ticket.

## Rectification apportée à la direction du ticket

Le remède demandé est **retenu intégralement** : les trois mécanismes sont justifiés par M1 et M2 indépendamment de M4. Un scan qui peut redemander la même revue 96 fois par jour est un défaut, qu'il ait ou non produit le churn du 16 septembre.

Deux points de cadrage, à porter dans le corps de PR plutôt qu'à découvrir après coup :

1. **L'AC3 (« sérialisation ») se décompose en deux affirmations distinctes.** « Au plus une revue QA active par agent » est un invariant du moteur, déjà vrai (M3) : ce plan l'**épingle** par un test de non-régression, il ne l'implémente pas. Ce que le réconciliateur doit garantir, et ce que ce plan livre, c'est de ne pas être une **source** de sur-demande : cap arithmétique et cooldown. Écrire un verrou de concurrence dans `qa_review_reconcile` serait un placebo — le scan ne détient aucun tour.
2. **Ce correctif ne peut pas, à lui seul, prouver que le churn cesse.** Si la mesure post-déploiement montre encore des revues QA qui crèvent l'enveloppe, la cause est ailleurs (M4) et relève d'un ticket de suivi. C'est la condition d'arrêt ci-dessous, pas une échappatoire.

## Conception

### Brique 1 — le ledger devient décisionnel, keyé (dépôt, PR, head SHA)

`PrSnapshot` gagne `head_ref_oid: String`, demandé dans le `--json` de `list_open_prs` aux côtés des six champs existants. **Pas de `#[serde(default)]`**, pour la raison déjà écrite au-dessus de la structure (`:117-122`) : un champ explicitement demandé est toujours rendu, et une absence doit avorter le parse plutôt que faire entrer la PR dans la population. Une valeur vide est traitée comme illisible et **sort** la PR — même discipline fail-safe que `createdAt`. `headRefOid` est déjà lu ailleurs dans le dépôt par la même voie (`ci_success_handler.rs:617`), donc le champ est acquis, pas pariés.

La clé d'audit passe de `pr:{repo}#{n}` à **`pr:{repo}#{n}@{sha}`** — la forme exacte que `ci_success_handler` emploie déjà pour sa dedup durable (`:216`). La requête opérateur `WHERE tool_name = 'qa_review_reconciled'` est inchangée ; seul le format de `target_key` s'allonge, et le préfixe reste `pr:{repo}#{n}`.

Deux bornes, lues avec la méthode existante `count_recent_audit_events_for_target` (`db.rs:7043`) — **aucune nouvelle méthode DB, aucune migration** :

| borne | lecture | effet |
|---|---|---|
| **cooldown** | `count(since = now − COOLDOWN)` > 0 | la PR est sautée ce tick |
| **budget** | `count(since = now − MAX_AGE)` ≥ `MAX_ATTEMPTS` | la PR est **abandonnée** pour ce SHA, définitivement |

Le budget est ce qui distingue ce correctif d'un simple ralentissement : un cooldown seul rejoue pour toujours, juste moins vite. Précédent direct du dépôt, à citer dans le code : `MIKA_AUTO_PULL_MAX_REDRIVES` (mika#2020), né du constat que `mika#1901` avait reçu le label `ready` seize fois en dix-neuf heures. **Un nouveau SHA rouvre naturellement le budget** : la clé change, le compte repart à zéro — c'est exactement l'idempotence « par (PR, SHA) » que le ticket demande, obtenue par la forme de la clé plutôt que par un champ de plus.

**Lecture du ledger impossible ⇒ on ne pose pas** (fail-closed), avec `qa_review_reconcile_ledger_unreadable` en WARN. C'est l'inverse du choix de `ci_success_handler` (fail-open) et le même que celui de `wip_rescue` (mika#2199), pour la raison qui y est écrite : un faux négatif fait attendre une PR qui, avant mika#2334, attendait indéfiniment ; un faux positif rejoue une revue, c'est-à-dire produit le défaut que ce ticket existe pour fermer.

**Transition des lignes héritées.** Les poses écrites avant ce correctif portent `pr:{repo}#{n}` sans SHA et seraient invisibles au nouveau cooldown, ce qui autoriserait une re-pose sur des PRs déjà rattrapées. Le calcul du cooldown consulte donc **aussi** l'ancienne clé, en deuxième requête exacte (jamais un `LIKE`, qui ferait matcher `#234` sur `#2343`). Ce second appel est daté dans le code : il peut disparaître dès que toute ligne héritée est plus vieille que `MAX_AGE` (sept jours).

### Brique 2 — la troncature déménage, et c'est un changement de contrat

Aujourd'hui `select_prs_needing_review` tronque à `max_per_tick` (`:254`). Si la troncature reste en amont du filtre cooldown, trois PRs en cooldown consomment tout le tick et une quatrième, légitimement rattrapable, n'est jamais vue. La fonction pure **cesse donc de tronquer** : elle garde les six termes et le tri (plus ancienne d'abord, puis par numéro), et l'appelant tronque après avoir appliqué le cooldown.

`test_qa_review_reconcile_2334.rs` assert la troncature contre cette fonction ; il est mis à jour pour l'asserter là où elle vit désormais. **À dire dans le corps de PR** : ce n'est pas une assertion perdue, c'est une assertion déplacée avec la décision qu'elle garde.

Le budget de tick n'est débité que sur **tentative de pose** — un saut de cooldown ne coûte rien, sans quoi le cap redeviendrait un plafond sur les sauts plutôt que sur les écritures. La règle inverse, déjà écrite (`:488-495`), reste vraie et reste justifiée : un échec de pose, lui, débite.

### Brique 3 — réglages

| clé | défaut | trois paliers |
|---|---|---|
| `MIKA_QA_REVIEW_RECONCILE_COOLDOWN_SECS` | `3600` | absent/vide → défaut ; illisible, `0`, négatif → défaut + WARN |
| `MIKA_QA_REVIEW_RECONCILE_MAX_ATTEMPTS` | `2` | idem |
| `MIKA_QA_REVIEW_RECONCILE_MAX_PER_TICK` | **`3` → `1`** | inchangé (palier déjà en place) |

Le cooldown à une heure est aligné sur `MIN_AGE` : il doit dépasser la durée d'un tour (600 s) avec une marge large, et une PR que le rattrapage n'a pas réveillée en une heure ne le sera pas davantage en quinze minutes. Le cap à 1 fait passer la demande de rattrapage de 12/h à **4/h**, sous la capacité de 6/h établie en M2 — c'est le seul réglage dont l'effet est arithmétiquement démontrable, et c'est pour ça qu'il est le seul défaut que ce plan déplace. `MAX_ATTEMPTS = 2` laisse une seconde chance à un webhook perdu et refuse la troisième.

`0` ne désarme aucune de ces clés : c'est le rôle de `MIKA_QA_REVIEW_RECONCILE`. Une lecture inverse ferait d'une faute de frappe un désarmement silencieux.

### Brique 4 — observabilité (doctrine mika#2131)

- **Agrégat**, sur la ligne `qa_review_reconcile_tick` existante, deux champs de plus : `skipped_cooldown`, `abandoned`. Un INFO par PR sautée serait du bruit à chaque tick ; l'agrégat dit la forme du tick. **Zéro action, zéro ligne** reste vrai : un tick où tout est en cooldown n'a rien fait et n'écrit rien de plus qu'aujourd'hui.
- **Par abandon**, un fait rare et terminal : WARN `qa_review_reconcile_abandoned` + ligne d'audit `tool_name = 'qa_review_reconcile_abandoned'` (**sole writer**), `target_key = pr:{repo}#{n}@{sha}`, `reasoning` portant le nombre de tentatives. C'est la réponse directe à « pourquoi cette PR n'est-elle plus rattrapée ? ».
- `qa_review_reconcile_ledger_unreadable` (WARN) — **doit rester vide**. Toute occurrence est un ledger illisible, donc un scan devenu inerte.

### Brique 5 — épingler l'invariant de sérialisation (M3)

Le test demandé par le ticket — « lancer 2 revues simultanées sur le même agent, asserter que la 2e est refusée/queue » — porte sur le moteur, pas sur le réconciliateur. Deux assertions, dans le nouveau fichier de test :

1. **Comportementale**, sur `webhook_queue_v2` : deux `review_requested` pour le même agent produisent deux entrées de file distinctes (`opened`/`review_requested` ne coalescent jamais — `coalescing_key` rend `None`), consommées l'une après l'autre.
2. **Structurelle**, scan de source : il n'existe qu'un site de spawn du drain worker par agent, et `run_agent_for_message` reçoit le garde `agent_lock` plutôt que de le prendre lui-même. Un test comportemental ne verrait pas la régression qui compte ici — un second consommateur ne rendrait aucune décision fausse, il lèverait l'invariant en silence.

## Fire-Disposition

Aucun geste destructeur. Le correctif est additif sur un scan déjà armé ; le seul défaut déplacé est `MAX_PER_TICK` (3 → 1), réversible par variable d'environnement sans redéploiement. Le kill-switch `MIKA_QA_REVIEW_RECONCILE=0` reste la sortie de secours complète.

## Definition of Done

- `qa_review_reconcile` lit son propre ledger avant toute pose, avec cooldown et budget keyés (dépôt, PR, head SHA).
- Une PR dont la revue ne produit rien est rattrapée au plus `MAX_ATTEMPTS` fois par SHA, puis abandonnée en le disant.
- Le cap par tick est ramené à 1.
- La troncature vit après le filtre cooldown ; `test_qa_review_reconcile_2334.rs` suit le déplacement.
- L'invariant « un tour par agent à la fois » est épinglé par un test.
- `cargo test -p mika-agent`, `cargo clippy`, `cargo fmt` verts.
- `CLAUDE.md` (§ *réconciliation des demandes de revue*) et `crates/mika-agent/CLAUDE.md` (§ *QA-Review Reconciler*) portent les trois clés, les trois nouveaux signaux opérateur et la rectification M4.

## Acceptance criteria

*Le corps du ticket ne porte pas de section `## Acceptance criteria`. Les critères ci-dessous sont dérivés de sa section « Fix attendu » et de sa ligne de test, sans extension.*

- **AC1 — idempotence par (PR, head SHA).** Une PR dont le ledger porte une pose pour le SHA courant, à l'intérieur du cooldown, n'est pas re-posée. Assertion sur la fonction de décision, sans réseau ni DB.
- **AC2 — cooldown.** Après une pose, aucune nouvelle pose pour le même (PR, SHA) avant `COOLDOWN_SECS`. Un **nouveau SHA** est posable immédiatement : la clé change.
- **AC3 — pas de replay en boucle.** Au-delà de `MAX_ATTEMPTS` poses pour un même (PR, SHA), la PR est abandonnée pour ce SHA, avec WARN et ligne d'audit nommant le motif.
- **AC4 — sérialisation.** Deux événements de revue simultanés pour mika-qa produisent deux exécutions **séquentielles**, jamais concurrentes : la seconde attend en file. Épinglé par les deux assertions de la brique 5.
- **AC5 — le réconciliateur ne sur-demande pas.** `MAX_PER_TICK` par défaut à 1, soit 4 demandes/heure au plus, sous la capacité de 6 revues/heure établie en M2.
- **AC6 — fail-closed sur ledger illisible.** Une erreur de lecture d'`audit_events` empêche la pose et émet `qa_review_reconcile_ledger_unreadable` ; elle ne fait jamais échouer le tick du moteur.
- **AC7 — trois paliers.** Les deux nouvelles clés suivent le palier maison (absent/vide → défaut ; illisible, `0`, négatif → défaut + WARN) et `0` ne désarme pas.
- **AC8 — l'enveloppe ne bouge pas.** Aucune modification de `MIKA_AGENT_TOTAL_TIMEOUT_SECS`, de `agent_total_timeout_secs`, ni d'un budget d'outil. Vérifiable par la seule lecture du diff.

## Hors portée, délibérément

- **L'enveloppe agent et les budgets LLM** — bearing explicite.
- **Les autres déclencheurs d'une revue QA** (`synchronize`, fan-out `check_suite`, rejeu de file). M4 montre qu'ils restent en lice comme sources du churn observé ; les traiter ici serait élargir un ticket p1 à un périmètre non mesuré. **Ticket de suivi à ouvrir** si la sonde post-déploiement ci-dessous ne retombe pas.
- **La cause du ralentissement par tour** (« 21 tours > 600 s »). Ce plan réduit le nombre de revues demandées ; il ne rend aucun tour plus rapide. Si la contention est provider-side, elle survivra à ce correctif — c'est précisément ce que la condition d'arrêt mesure.
- **Le filet mika#2276** et la forme de son `hold[review]`, inchangés.

## Vérification

1. `cargo test -p mika-agent --test eval test_qa_review_reconcile_2347` — AC1, AC2, AC3, AC6, AC7 sur la fonction de décision, sans réseau ni DB.
2. `cargo test -p mika-agent --test eval test_qa_review_reconcile_2334` — le contrat existant tient après le déplacement de la troncature.
3. `cargo test -p mika-agent` puis `cargo clippy --all-targets -- -D warnings` et `cargo fmt --check`.
4. **Attribution, post-déploiement, 24 h.** `SELECT target_key, count(*) FROM audit_events WHERE tool_name = 'qa_review_reconciled' GROUP BY 1 ORDER BY 2 DESC;` — aucune clé ne doit dépasser `MAX_ATTEMPTS`. Toute clé au-delà signifie que le ledger n'est pas relu, donc que le correctif n'a pas pris.
5. **Symptôme, post-déploiement, 48 h.** `grep qa_deadline_verdict $MIKA_SPIRIT_LOG_FILE | jq 'select(.outcome == "posted")'` — le régime attendu est **zéro** ligne. Relever au passage l'enveloppe réellement en vigueur pour mika-qa : `grep llm_budget_resolved $MIKA_SPIRIT_LOG_FILE | jq 'select(.agent_id == "mika-qa") | {agent_total_timeout_secs, total_source}'` (mika#2293). Le ticket annonce 600 s ; ni `MIKA_QA_CONFIG` ni les défauts du dépôt ne les portent, donc la valeur vient d'ailleurs et sa **provenance** est ce qu'il faut lire avant de conclure quoi que ce soit sur l'enveloppe.

## Conditions d'arrêt

- **Le `hold[review]` de deadline persiste après 48 h** alors que la sonde 4 est propre : le réconciliateur n'était pas la source du churn (M4 confirmé). **Halte** — ne pas baisser `MAX_PER_TICK` davantage, ne pas rallonger le cooldown : ouvrir le ticket de suivi sur les autres déclencheurs, avec la distribution de la sonde 5 en pièce jointe.
- **`qa_review_reconcile_ledger_unreadable` non vide** : le fail-closed rend le scan inerte. Halte, réparer la lecture d'audit avant toute autre chose.
- **Une PR abandonnée qui méritait sa revue** : ne pas relever `MAX_ATTEMPTS` par réflexe — un abandon prouve que deux rattrapages n'ont rien produit, c'est-à-dire que le chemin nominal est cassé en amont.

## Voisinage

mika#2334 (naissance du réconciliateur), mika#2341 (ce qui l'a rendu effectif), mika#2276 (le filet qui rend le dépassement visible et fournit la preuve du symptôme), mika#2020 (`MAX_REDRIVES` — le précédent direct du budget par clé), mika#2199 (le fail-closed de `wip_rescue` et le livelock qu'il a fermé), mika#1869 (la dedup durable keyée `head_sha` de `ci_success_handler`, dont ce plan reprend la forme), mika#2131 (la doctrine d'observabilité des exclusions), mika#1870 et mika#2163 (les deux files qui tiennent l'invariant de sérialisation).
