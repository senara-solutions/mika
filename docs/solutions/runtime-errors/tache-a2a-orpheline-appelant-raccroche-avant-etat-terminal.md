---
module: mika-agent/server, mika-agent/task_engine
date: 2026-09-18
problem_type: runtime_error
category: runtime-errors
component: api_layer
severity: high
symptoms:
  - "Des lignes tasks trigger_type='a2a' de mika-arch restent status='in_progress' pour toujours, avec updated_at == created_at"
  - "Dans server.log, la trace s'arrête net au milieu d'un llm_call ~660 s après la création, sans ligne de fin ni d'échec"
  - "Côté appelant, un timeout de mika ask ; côté serveur, aucune erreur enregistrée"
root_cause: concurrency
resolution_type: code_fix
tags: [a2a, dropped-future, orphaned-task, raii-guard, startup-recovery, terminal-state]
related_components:
  - crates/mika-agent/src/server/a2a.rs
  - crates/mika-agent/src/task_engine/engine.rs
  - crates/mika-a2a/src/client.rs
---

# Un tour A2A exécuté dans le futur du handler laisse sa ligne `in_progress` pour toujours quand l'appelant raccroche (mika#2379)

Fix sur la branche `bug/2379/agent-les-t-ches-mika-arch-orphelines`, PR pas encore ouverte au moment où ces lignes sont écrites.

## Problème

Un `message/send` A2A exécute tout son tour **à l'intérieur du futur du handler axum** (`crates/mika-agent/src/server/a2a.rs:695`, `handle_message_send`). La séquence est la suivante : création de la ligne (`pending`), puis `a2a_update_task_state(.., "working")` dans la foulée (d'où le `updated_at == created_at` observé sur les lignes orphelines), puis `run_a2a_agent(..).await` (`a2a.rs:821`), et c'est seulement **après** cet `.await` que `completed` ou `failed` est écrit (`a2a.rs:836`, `a2a.rs:848`).

Quand l'appelant HTTP raccroche, hyper abandonne ce futur au milieu du tour. Aucune des deux écritures terminales ne s'exécute, et la ligne `tasks` reste `in_progress` indéfiniment. Aucune ligne de log ne signale le départ de l'appelant.

L'appelant raccroche parce que, en prod, les deux budgets sont identiques mais ne démarrent pas au même moment. Côté client, `mika ask` résout son budget d'envoi comme `max(MIKA_A2A_TIMEOUT_SECS, défaut 600 s, MIKA_AGENT_TOTAL_TIMEOUT_SECS)` (`crates/mika-a2a/src/client.rs:30-53`), soit 660 s avec le `MIKA_AGENT_TOTAL_TIMEOUT_SECS=660` de `~/.mika/.env`. Côté serveur, le tour a la même enveloppe de 660 s, mais son horloge démarre plus tard (transit, puis attente du verrou de l'agent). Sur un tour lent, le client raccroche donc toujours en premier.

## Symptômes

- 11 lignes orphelines vivantes mesurées le 2026-09-18, toutes `mika-arch`, `trigger_type='a2a'`, `status='in_progress'`, `updated_at == created_at`.
- Dans `/var/log/mika/server.log`, cinq de ces onze tâches, relevées dans le log (identifiants de tâche A2A 17ba0089, 035e962b, 3d8ab24d, 2cbc1f35 et ff071c78), s'arrêtent net au milieu d'un `llm_call`, environ 660 s après la création, sans « A2A task completed via agent loop » ni « A2A agent loop failed ».
- Côté appelant, un timeout de `mika ask`. Côté serveur, rien.

## Ce qui n'a pas marché

- **Le reaper à l'âge.** Le ticket rangeait le bug dans la classe de #2263 (un supersede qui ne tue pas le pilote) et demandait un reaper à N minutes, sur le modèle du reaper pilote #2277. Écarté : un tour A2A a toujours un propriétaire en mémoire dans le processus. Un garde RAII ferme donc toutes les sorties, et un reaper à l'âge risquerait de faucher un tour vivant qui attend le verrou ou qui est simplement lent.
- **« Le redémarrage les purge », lu comme une propriété à construire.** C'était déjà vrai. La boucle générique préexistante de `TaskEngine::startup_recovery` (`crates/mika-agent/src/task_engine/engine.rs:391`, étape 2) fait passer en `failed` toute ligne `in_progress` non manuelle au démarrage du démon. Les 4 orphelines d'avant le redémarrage citées dans le ticket (lignes `tasks` 46e37d7a, e004b761, 6b320746 et 9821489e) étaient déjà `failed`, avec `updated_at = 2026-09-18T04:00:55Z`, l'instant du redémarrage. Il ne fallait donc pas de second balayage, seulement des spécificités A2A à l'intérieur de cette récupération.
- **Un balayage dans `init_agent`.** Premier jet écarté, parce que `init_agent` tourne aussi à la demande depuis `AppState::resolve_agent` pendant que des requêtes sont en vol (il aurait fauché une ligne vivante), et parce que `startup_cleanup` tourne après que le routeur sert déjà.

## Solution

**1. `TurnGuard`** (`a2a.rs:45-147`). Le garde est armé dès que la ligne existe. Il est désarmé par `settle(result)` (`a2a.rs:93`), uniquement si l'écriture terminale du tour lui-même a renvoyé `Ok`. S'il est détruit encore armé, son `Drop` (`a2a.rs:107`) lance sur le runtime courant (`tokio::runtime::Handle::try_current`) une écriture conditionnelle, puis émet un WARN `a2a_turn_abandoned` (agent, task_id, port `send|stream`, elapsed_ms).

Avant :

```rust
let _ = agent_state.db.a2a_update_task_state(&task_id, "working").await;
match run_a2a_agent(..).await {          // futur abandonné ici -> rien après
    Ok(text) => { let _ = db.a2a_update_task_state(&task_id, "completed").await; .. }
    Err(e)   => { let _ = db.a2a_update_task_state(&task_id, "failed").await; .. }
}
```

Après :

```rust
let mut turn_guard = TurnGuard::arm(&agent_state.db, &task_id, "send");
let _ = agent_state.db.a2a_update_task_state(&task_id, "working").await;
match run_a2a_agent(..).await {
    Ok(text) => { turn_guard.settle(db.a2a_update_task_state(&task_id, "completed").await); .. }
    Err(e)   => { turn_guard.settle(db.a2a_update_task_state(&task_id, "failed").await); .. }
}
// abandon du futur -> Drop -> a2a_abandon_task_if_live(..)
```

L'écriture conditionnelle, `a2a_abandon_task_if_live` (`crates/mika-agent/src/a2a_db.rs:291`), passe la ligne en `cancelled` **seulement si** elle est encore en `status IN ('pending', 'in_progress')`. Un état terminal déjà écrit n'est jamais écrasé. Le même garde est armé dans la tâche lancée par `message/stream` (`a2a.rs:1004`), où il couvre un panic. La branche `returnImmediately` n'arme aucun garde, puisqu'elle n'exécute pas de tour.

**2. `startup_recovery`, étape 2a** (`engine.rs:404-434`). `a2a_sweep_orphans` (`a2a_db.rs:315`) passe en `failed`, avec `completed_at` et une raison, toute ligne `trigger_type = 'a2a'` encore en `pending`/`in_progress`, puis émet un WARN `a2a_orphans_swept`. Les lignes `pending` sont incluses parce qu'un tour stream mort en attendant le verrou n'a jamais atteint `in_progress`. Cette étape tourne **seulement dans le démon** : elle est sautée quand `dispatcher.cli_mode` est vrai, parce que `mika chat` exécute le même `startup_recovery` (`crates/mika-cli/src/commands/chat.rs:258`) sur la base partagée pendant que le démon sert peut-être des tours vivants. Si le processus meurt sans runtime, le `Drop` du garde laisse la ligne à cette étape.

**3. La raison est écrite en JSON.** Elle va dans `tasks.result` sous la forme `{"a2a_close_reason": "..."}` (`a2a_close_reason_metadata`, `a2a_db.rs:43`), parce que `a2a_build_task` (`a2a_db.rs:654`) passe `t.result` à `serde_json::from_str(..)?` pour en faire `Task.metadata` (`a2a_db.rs:688`).

## Pourquoi ça marche

Le défaut, c'est qu'une écriture placée *après* un `.await` n'appartient à personne quand le futur est abandonné. Un garde RAII rattache l'état terminal à la durée de vie du tour lui-même : toute sortie (retour normal, futur abandonné, panic) passe par `Drop`. Grâce au `WHERE status IN ('pending','in_progress')`, la fermeture est idempotente et ne peut pas écraser le verdict réel. `settle` ne désarme que sur une écriture réussie : un `SQLITE_BUSY` sur l'écriture terminale laisse donc au garde une seconde tentative. Le seul trou restant, un processus qui meurt sans runtime, est couvert par la récupération au démarrage. Celle-ci est sûre dans le démon, parce qu'aucun tour ne survit au processus qui l'exécutait.

## Prévention

- Tout tour exécuté dans le futur d'un handler de requête a besoin d'un propriétaire de l'état terminal qui survit à l'abandon de ce futur : un garde RAII avec écriture conditionnelle, pas une écriture après le `.await`. Faute de propriétaire en mémoire, un reaper à l'âge est le dernier recours.
- Avant d'ajouter un balayage au démarrage, lire ce que `startup_recovery` fait déjà, et vérifier **chacun** de ses appelants (le démon *et* `mika chat`). Une hypothèse « processus mort » vraie dans le démon est fausse dans la CLI qui partage la base.
- Un test d'une nouvelle écriture doit la relire par le lecteur de production (`a2a_build_task` / `tasks/get`), pas en SQL brut. Une première version écrivait la raison en texte brut dans `tasks.result` : `tasks/get` renvoyait alors `INTERNAL_ERROR` précisément sur les lignes qu'on fermait, y compris pour la lecture de récupération de mika#2036. Deux relecteurs indépendants l'ont vu. Les nouveaux tests, qui lisaient les colonnes en SQL, ne l'ont pas vu.
- Non fait, et laissé en suite : aligner les budgets client/serveur 660/660 pour que le serveur termine avant que le client abandonne (voir `docs/solutions/best-practices/dimensionner-une-attente-contre-le-budget-le-plus-serre-2026-09-05.md`), et extraire le garde dans un module dédié (`server/turn_guard.rs`, proposé à la revue, pas encore créé).
- Ne pas déclarer le bug fermé sur la foi des tests verts. Une fois déployé, vérifier qu'un vrai raccroché produit bien un `a2a_turn_abandoned` et une ligne `cancelled`.

## Voir aussi

- `docs/solutions/runtime-errors/a2a-transport-failure-discards-generated-response-2026-09-04.md`
- `docs/solutions/best-practices/dimensionner-une-attente-contre-le-budget-le-plus-serre-2026-09-05.md`
- `docs/solutions/logic-errors/a2a-dual-write-duplicate-rows.md`
- `docs/solutions/best-practices/1652-team-runs-orphan-reaper-and-tool-error-as-ok-not-err.md`
- Plan : `docs/plans/2026-09-18-005-fix-2379-a2a-abandoned-turn-terminal-state-plan.md` ; ticket senara-solutions/mika#2379.
