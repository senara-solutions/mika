---
module: mika-agent/server
tags: [ci-success-handler, merge-ready, fan-out, agent-identity, dedup, loop-substrate, forge-gate]
problem_type: missing-entry-gate
category: bug
type: fix
issue: senara-solutions/mika#2260
branch: bug/2260/loop-substrate-double-chemin-de-merge-le
status: groomed
---

# fix mika#2260 — la transition évaluer→merger n'appartient qu'au dispatcher : porte à l'ENTRÉE de `ci_success_handler`

**Issue :** senara-solutions/mika#2260 (p2, bug, loop-substrate)
**Branche :** `bug/2260/loop-substrate-double-chemin-de-merge-le`
**Lignée :** mika#1711 (fan-out `check_suite` vers mika-qa) → mika#1869 (dedup burst) → mika#2248/PR#2258 (signal, pas acteur) → **ce ticket** (le relecteur n'entre plus)

## Le défaut, mesuré

PR#2258 a corrigé **qui merge** : `mergedBy = mika-platform-dev` sur #2259, `human_gate_events = 0`, re-preuve C2 réussie. Il n'a pas corrigé **qui évalue**. Le relecteur (`mika-qa`) parcourt toujours l'évaluateur complet — jusqu'à émettre le signal — et n'est retenu qu'à l'acteur.

### Comptes depuis le déploiement de #2258 (`~/.mika/data/mika.db`, `audit_events`, 2026-09-09 → 2026-09-10)

| agent | `ci_success_handler_processed` | `ci_success_handler_human_gate_required` | `ci_success_merge_ready` | `merge_ready_hold_reviewer_is_not_merge_actor` | `ci_success_merge` |
|---|---|---|---|---|---|
| mika-dev | 65 | 7 | 1 | 0 | 1 |
| **mika-qa** | **81** | **6** | **1** | **1** | 0 |

81 parcours complets sous le PAT du relecteur (jusqu'à cinq appels `gh` chacun : `pr list`, `api reviews`, `pr checks`, `pr view --json files`, `pr view` + `api` behind-main). Six tenues DECISION-CORE — donc six notifications « Operator must merge manually » **en double** de celles de mika-dev.

### Chronologie #2259, head `047e9eea` (`/var/log/mika/server.log` + `audit_events`)

| horodatage (Z) | agent | événement |
|---|---|---|
| 09:07:26 | mika-dev | `deferring webhook: task has in-flight callback` (60 s) |
| 09:08:11 | mika-qa | `CI success handler: evaluating` → `processed` écrit → clé dedup mémoire **enregistrée** → checks encore `pending` |
| 09:08:26 | mika-dev | replay du webhook différé → **`ci_success_dedup.skip`** — l'évaluation du dispatcher est **avalée** par la clé posée 15 s plus tôt par le relecteur |
| 09:12:36.705 | mika-qa | `evaluating` → toutes portes vertes → `ci_success_merge_ready` (09:12:40) → **`merge_ready_hold_reviewer_is_not_merge_actor`** |
| 09:12:36.709 | mika-dev | `deferring webhook` (60 s) |
| 09:13:36.710 | mika-dev | replay — **60,005 s** après l'enregistrement de mika-qa, fenêtre dedup = 60 s : passe à 5 ms près ; puis fait la queue derrière un tour LLM occupé |
| 09:20:17 | mika-dev | `evaluating` → `ci_success_merge_ready` → `merge_ready_merge_initiated` (09:20:29) |

Le « double chemin en ~15 s » observé par Vincent est la paire 09:08:11 / 09:08:26. Elle montre la conséquence que le ticket redoutait sans la nommer : **le dedup mémoire est global au processus `mika-spirit` et clé sur `{repo}:{branch}:{head_sha}` sans identité d'agent** (`crates/mika-agent/src/server/check_suite_dedup.rs:59-61`). Quand le relecteur entre en premier, il consomme le créneau du dispatcher. À 09:08 c'était sans conséquence (checks `pending`). À 09:12, avec toutes les portes vertes, le dispatcher n'a évalué que parce que sa copie est arrivée **5 ms après** la fin de la fenêtre. Un deferral à 59,9 s au lieu de 60,005 s : mika-dev rend `ci_success_dedup.skip`, aucun signal n'est émis sous une identité qui peut agir, et plus aucun `check_suite` n'arrive sur ce head (tous les workflows sont déjà terminés). **La PR reste verte et ouverte sans que rien ne la ferme.** Ce n'est pas un résidu cosmétique : c'est une classe d'arrêt de boucle à un timing près.

### Les effets de bord que le relecteur exerce aujourd'hui AVANT la garde

Tous mesurés sur le code de `try_handle_ci_success` (`crates/mika-agent/src/server/ci_success_handler.rs:123-587`), dans l'ordre d'exécution :

1. **Clé dedup mémoire** (2b, `check_suite_dedup::try_dedup_check_suite`) — globale au processus, avale l'évaluation du dispatcher (mesuré ci-dessus).
2. **Cinq appels `gh` sous le PAT du relecteur** (2, 3, 5, 5b, 5c) — quota et latence pour rien : le signal émis par mika-qa n'a **aucun consommateur** par construction (il voyage en bande dans `req.text`, relu par le handler suivant *du même agent*, où l'acteur le retient).
3. **`PUT /repos/{repo}/pulls/{n}/update-branch` sous le PAT du relecteur** (5c, `remediate_behind_main` → `attempt_update_branch`, `crates/mika-agent/src/tools/pr_merge_with_gate.rs:998-999`) — une **écriture** sur la forge, posée par l'agent qui ne doit jamais agir sur cette PR. Pas encore observée en audit (aucune ligne behind-main sous mika-qa), mais atteignable dès que le relecteur entre avec une PR en retard sur main.
4. **Notification opérateur DECISION-CORE en double** (5b) — 6 mesurées.
5. **Le pré-digest de l'évaluateur REMPLACE `req.text` dans le tour du relecteur** (`handlers.rs:1299-1301`, `Handled { pre_digest }`) — mika-qa reçoit « Merge-ready signal emitted… the dispatcher owns the merge » ou « Investigate the error… » à la place de l'événement brut que `qa-review-webhook-success` attend (repo + branche, pour corréler la PR et décider de la revue). Le prompt du relecteur est écrit pour le texte brut ; aujourd'hui il ne le voit que quand l'évaluateur rend `Passthrough`.

## La cause, en une phrase

Le fan-out gateway vers mika-qa est **voulu** (mika#1711 : `secondary_targets`, `crates/mika-gateway/src/github.rs:358-368` — c'est ce qui déclenche la revue autonome sur CI verte), mais la chaîne de handlers structurels de `handlers.rs` (`:1289-1337`) tourne à l'identique dans chaque agent, et `try_handle_ci_success` ne se sélectionne que sur le **type d'événement** (`parse_check_suite_success`), jamais sur **l'agent**. #2258 a mis l'identité à la sortie (l'acteur) ; il l'a laissée absente de l'entrée (l'évaluateur), par une décision explicite — « Aucune logique d'identité ici, par conception » (`ci_success_handler.rs:531`) — qui confond deux questions : *qui merge* (l'acteur, réglé) et *pour qui cette évaluation existe* (personne, quand c'est le relecteur qui la fait).

## La conception

**Un seul énoncé, une seule porte :** la transition évaluer→signaler→merger est un tout qui appartient au dispatcher. Un agent qui ne peut pas consommer le signal ne l'évalue pas. La porte se pose au premier point où l'agent est connu et avant tout travail — après la sélection par type d'événement, avant même l'exigence de token.

### D1 — `forge_identity::owns_merge_transition(agent_id) -> bool`

`crates/mika-common/src/forge_identity.rs`. Un nom pour le concept, défini comme `merge_disposition(agent_id) == MergeDisposition::Act`. La liste blanche reste unique (`DISPATCHER_AGENT`) ; ajouter un acteur reste un geste explicite à un seul endroit, et la porte d'entrée en hérite sans seconde table. La doc du module gagne une quatrième ligne dans le tableau des couches : « la porte d'entrée de l'évaluateur — seul le dispatcher évalue ; les autres agents restent transparents à l'événement ».

Pourquoi un symbole plutôt que réutiliser `merge_disposition` en ligne : l'évaluateur ne décide pas *qui merge* — c'est l'acteur qui le fait, et ça ne bouge pas. Il décide si *son évaluation a un consommateur*. Deux questions, deux noms ; une seule table sous-jacente.

### D2 — la porte d'entrée dans `try_handle_ci_success`

`crates/mika-agent/src/server/ci_success_handler.rs`, immédiatement après l'étape 1 (`parse_check_suite_success`), **avant** le `match github_token` et avant tout appel `gh`, toute écriture dedup, toute ligne d'audit existante :

- si `!owns_merge_transition(db.agent_id())` : `info!(event = "ci_success_handler_skipped_not_merge_actor", repo, branch, agent_id, dispatcher = DISPATCHER_AGENT, …)`, une ligne `audit_events` du même nom (clé cible `event:{repo}@{branch}` — le numéro de PR n'est pas connu, et ne doit pas l'être : le connaître coûte le premier appel `gh`), puis `return VerdictAction::Passthrough { enrichment: None }`.
- sinon : la suite inchangée.

`Passthrough { enrichment: None }` et non `Handled` : le tour du relecteur voit **l'événement brut**, exactement ce que `qa-review-webhook-success` attend (effet 5 ci-dessus). Pas d'enrichissement non plus — le prompt du relecteur sait déjà qu'il ne merge pas ; lui redire n'ajoute qu'un second texte à suivre.

L'ordre est le point porteur : la porte doit précéder `try_dedup_check_suite` (effet 1) — c'est ce que le test structurel épingle (T3).

### D3 — la seconde ceinture reste

`merge_ready_handler::authorize_merge` (`WouldMergeAsReviewer` puis `NotDispatcher`) et le refus outil `pr_merge_with_gate_reviewer_refused` **ne bougent pas**. Après D2 le hold du relecteur devient inatteignable en production ; il reste la défense si un signal arrivait par un chemin que l'évaluateur n'a pas posé (le module le dit déjà). Les tests de #2248 qui l'exercent en appelant l'acteur directement restent verts tels quels.

### D4 — les commentaires et la doc qui disent l'inverse

- `ci_success_handler.rs` § invariants d'en-tête : ajouter l'invariant « porte d'entrée (mika#2260) » ; étape 2c, la note de portée « check_suite success events for a given PR route to the same agent (mika-qa) » est fausse depuis #1711 et devient « seul le dispatcher atteint ce point depuis mika#2260 » ; étape 6, « Aucune logique d'identité ici, par conception » devient « l'identité décide à l'entrée si l'évaluation a un consommateur ; elle ne décide jamais ici qui merge ».
- `crates/mika-agent/CLAUDE.md` § *Structural CI Success Handler* et § *Merge Actor (mika#2248)* : un paragraphe « Entry gate (mika#2260) » avec le signal opérateur `ci_success_handler_skipped_not_merge_actor` et le fait mesuré du dedup global au processus.
- `docs/solutions/architecture-patterns/2026-09-09-un-handler-diffuse-a-deux-agents-ne-peut-pas-etre-acteur.md` : une section « Le résidu (mika#2260) » — une garde tardive résout la course sans la supprimer ; ce que l'évaluateur du relecteur touchait quand même (les cinq effets) ; la règle générale devient : *un handler diffusé à N agents n'évalue que dans l'agent qui peut consommer son évaluation*.

### Ce que ce plan ne fait pas, et pourquoi

- **Poser la porte dans `handlers.rs` autour de la paire évaluateur+acteur.** Considéré, écarté : cela garderait l'évaluateur littéralement « sans identité », mais le corps de `handle_message` n'est pas testable en unité — la porte ne serait épinglable que par un test de source. Dans le handler, l'auto-sélection par agent vit au même endroit que l'auto-sélection par type, et T1 l'exerce sans réseau. La phrase « Aucune logique d'identité ici, par conception » de #2248 décrivait l'état après #2248 (l'évaluateur ne décide pas *qui merge* — toujours vrai) ; ce n'est pas un invariant qui interdit une porte d'entrée (validé mika-arch, première passe).
- **Retirer mika-qa du fan-out gateway.** Non : le relecteur a besoin de l'événement brut pour déclencher la revue (mika#1711, 14 h de fenêtre QA morte avant). La porte est côté agent, pas côté routage.
- **Ajouter `agent_id` à la clé du dedup mémoire.** Considéré, écarté : une fois la porte posée, un seul agent atteint le dedup ; la clé multi-agent ne servirait qu'à ré-autoriser N évaluations si un second acteur était un jour listé — ce que la liste blanche rend délibérément coûteux. Le test structurel T3 (porte avant dedup) est la protection, pas la clé.
- **Le retard de 8 min de mika-dev sur #2259** (09:12 → 09:20) : file d'attente derrière un tour LLM `MaxTokens` de mika-dev, pas ce ticket. Observation à part.

## Acceptance Criteria

- **AC1 — le relecteur est transparent à l'événement.** Pour un `check_suite.completed(success)` bien formé, dans un agent où `owns_merge_transition == false` (`mika-qa`, et tout agent hors liste blanche : `mika`, `mika-arch`, un nom inconnu), `try_handle_ci_success` rend `Passthrough { enrichment: None }` **sans** appel `gh`, **sans** ligne `ci_success_handler_processed`, **sans** clé dedup, et laisse une ligne `audit_events` `ci_success_handler_skipped_not_merge_actor`. Contrôle positif dans le **même test** : l'agent `mika-dev`, même texte, `github_token = None`, rend `Passthrough { enrichment: Some(…no GitHub token…) }` — preuve que la porte l'a laissé passer **et** qu'elle a joué avant l'exigence de token.
- **AC2 — la porte précède tout travail (structurel).** Dans le corps de `try_handle_ci_success`, l'indice de `owns_merge_transition(` est strictement inférieur à ceux de `github_token`, `find_open_pr(`, `try_dedup_check_suite(` et `count_recent_audit_events_for_target(`. Épinglé par `include_str!` (même forme que `test_merge_identity_2248.rs`).
- **AC3 — le nom d'audit est réel.** `ci_success_handler_skipped_not_merge_actor` passe `assert_audit_event_name_is_real` (`tests/eval/test_ci_success_handler.rs`) et apparaît dans les signaux opérateur de `crates/mika-agent/CLAUDE.md`.
- **AC4 — la seconde ceinture tient encore.** `mika2248_le_relecteur_natteint_jamais_le_merge`, `ac3_*`, `ac1_*`, `ac4_*` de `tests/eval/test_merge_identity_2248.rs` et les tests in-file de `merge_ready_handler.rs` restent verts **sans modification** — la porte d'entrée s'ajoute, elle ne remplace pas.
- **AC5 — mesure post-déploiement (une seule requête, deux contrôles).** Après déploiement, pour tout `check_suite.completed(success)` postérieur à l'horodatage du déploiement : `audit_events` sous `mika-qa` compte `ci_success_handler_skipped_not_merge_actor ≥ 1` et `ci_success_handler_processed = 0` ; sous `mika-dev`, `ci_success_handler_processed ≥ 1`. Et `/var/log/mika/server.log` ne contient plus aucun `ci_success_dedup.skip` dont la première sighting appartient à un autre agent que mika-dev (en pratique : plus de paire `evaluating` mika-qa → `dedup.skip` mika-dev sur le même head).
- **AC6 — rouge avant.** T1 est rouge sur `main` (l'agent `mika-qa` avec `token = None` rend aujourd'hui `Passthrough { enrichment: Some(…) }` — l'enrichissement « no GitHub token » — et aucune ligne `skipped`). Le test échoue pour la bonne raison, terme par terme.

## Périmètre

| fichier | geste |
|---|---|
| `crates/mika-common/src/forge_identity.rs` | D1 : `owns_merge_transition` + doc module + test unitaire (dispatcher vrai, relecteur faux, inconnu faux) |
| `crates/mika-agent/src/server/ci_success_handler.rs` | D2 : porte d'entrée ; D4 : invariants et commentaires |
| `crates/mika-agent/tests/eval/test_ci_success_handler.rs` | T1, T2, T3, AC3 |
| `crates/mika-agent/tests/eval/test_merge_identity_2248.rs` | inchangé — AC4 le vérifie tel quel |
| `crates/mika-agent/CLAUDE.md` | D4 |
| `docs/solutions/architecture-patterns/2026-09-09-un-handler-diffuse-a-deux-agents-ne-peut-pas-etre-acteur.md` | D4 |

**Zone DECISION-CORE.** Source Rust sous `crates/` : le classifieur de périmètre est fail-closed (`crates/mika-agent/src/perimeter/rules.rs`), la PR sera tenue par la forge-gate et mergée par l'opérateur. C'est la forme attendue — le ticket le dit, et c'est le même chemin que PR#2258.

**Non touchés :** `handlers.rs` (la porte vit dans le handler, pas dans le câblage — le test d'ordre de #2248 sur `handlers.rs` reste vrai), `merge_ready_handler.rs`, `pr_merge_with_gate.rs`, `check_suite_dedup.rs`, `mika-gateway`.

## Tests

Tous sans réseau : base en mémoire portée par `agent_id` (`db_for_agent`, forme déjà présente dans `merge_ready_handler.rs`), `github_token = None` pour que le seul chemin qui passe la porte s'arrête sur l'enrichissement « no GitHub token » — la première ligne après la porte.

- **T1 (AC1, AC6) `mika2260_le_relecteur_est_transparent_a_levenement`** — même texte `[GitHub] Check suite success on senara-solutions/mika (branch: fix/x)`, trois agents dans le même test : `mika-qa` → `Passthrough { enrichment: None }`, `skipped == 1`, `processed == 0` ; `mika` (hors liste) → idem ; `mika-dev` → `Passthrough { enrichment: Some(s) }` avec `s` contenant `no GitHub token`, `skipped == 0`. Le contrôle positif et les deux négatifs dans un seul appel de test, sinon un handler qui rendrait toujours `Passthrough { None }` passerait.
- **T2 (AC1) `mika2260_un_evenement_non_check_suite_reste_passthrough_sans_audit`** — texte non correspondant, agent `mika-qa` → `Passthrough { None }` et `skipped == 0` : la porte ne se déclenche qu'après la sélection par type d'événement (elle ne doit pas écrire une ligne d'audit pour chaque webhook `pull_request` qui passe par la chaîne).
- **T3 (AC2) `mika2260_la_porte_precede_tout_travail`** — `include_str!` + `try_handle_ci_success_body()` : indices ordonnés comme dit en AC2.
- **T4 (AC3)** — `assert_audit_event_name_is_real("ci_success_handler_skipped_not_merge_actor")`.
- **T5 (D1)** — dans `forge_identity.rs` : `owns_merge_transition("mika-dev")`, `!owns_merge_transition("mika-qa")`, `!owns_merge_transition("")`, `!owns_merge_transition("mika-dev-2")`, et l'égalité avec `merge_disposition(...) == Act` sur ces mêmes entrées (les deux fonctions ne doivent pas diverger).
- **AC4** — `cargo test -p mika-agent --test eval` et `cargo test -p mika-common` verts, aucune modification des tests de #2248.

## Risques et questions ouvertes

- **Un agent client (`customer-42`) qui recevrait un `check_suite` ne sera plus jamais évalué.** C'est voulu par la liste blanche de #2248 (« un agent inconnu signale, il ne merge pas ») — ici il ne signale même plus. Si un jour un tenant doit auto-merger ses propres PRs, c'est `DISPATCHER_AGENT` qu'il faut faire évoluer (une table d'acteurs par tenant), pas la porte.
- **Volume d'audit.** La ligne `skipped` remplace, sous mika-qa, une ligne `processed` **plus** jusqu'à cinq appels `gh` par événement ; sur une rafale de 8 workflows elle remplace 1 `processed` + 7 `dedup.skip` avec chacun son `gh pr list`. Le solde est net négatif en écritures et en appels. Pas de dedup nécessaire sur la ligne `skipped` : elle est le compteur qu'AC5 mesure.
- **Pas de question ouverte de conception.** Le point d'insertion est dicté par l'effet 1 (avant le dedup) et l'effet 5 (`Passthrough`, pas `Handled`) ; les deux sont mesurés, pas choisis.

## Fire-Disposition

| test | rouge sur `main` ? | preuve de la bonne raison |
|---|---|---|
| T1 | oui | mika-qa rend `Some(…no GitHub token…)` et `skipped == 0` — deux assertions distinctes échouent, pas une seule |
| T2 | non (vert des deux côtés) | contrôle négatif de T1 : garantit que la porte ne s'est pas mise *avant* la sélection par type |
| T3 | oui | `owns_merge_transition(` absent du corps → l'assertion d'ordre échoue sur `find` = `None` |
| T4 | oui | nom absent de la source |
| T5 | ne compile pas | symbole absent |

## Vérification

1. `cargo test -p mika-common forge_identity` ; `cargo test -p mika-agent ci_success_handler` ; `cargo test -p mika-agent --test eval` ; `cargo clippy --all-targets -- -D warnings`.
2. Après déploiement (`make deploy` depuis `main`, poste sur `main`), attendre **au moins une PR** de la boucle qui passe au vert (période du composant le plus lent : un cycle CI complet, ~10 min), puis la requête AC5 :

   ```sql
   select agent_id, tool_name, count(*)
   from audit_events
   where created_at >= '<horodatage du déploiement>'
     and tool_name in ('ci_success_handler_processed',
                       'ci_success_handler_skipped_not_merge_actor',
                       'ci_success_merge_ready')
   group by 1, 2 order by 1, 2;
   ```

   Attendu : mika-qa n'a **que** des `skipped` ; mika-dev n'a **aucun** `skipped` et au moins un `processed`.
3. `grep -c ci_success_dedup.skip /var/log/mika/server.log` après le déploiement : chaque occurrence restante doit avoir sa première sighting sous mika-dev (rafale de workflows d'un même push — le cas pour lequel le dedup existe), jamais sous mika-qa.
