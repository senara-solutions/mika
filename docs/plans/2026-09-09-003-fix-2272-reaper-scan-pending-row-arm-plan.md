# fix mika#2272 — le reaper D1 scanne la row qui PORTE le pid, et il est armé

**Issue:** senara-solutions/mika#2272 (p1, bug, loop-substrate)
**Branche:** `fix/2272/reaper-scan-pending-row-arm` (branchée sur `test/2265` — elle a le harness multi-agents)
**Companion:** mika#2274

## Le défaut, mesuré

Le reaper D1 (mika#2249 / PR#2261) est **inerte**. Deux causes indépendantes, et
chacune suffit à elle seule à l'expliquer.

### 1. Il scanne une surface vide

`get_active_callback_tasks_with_pid` (`db.rs:8622`) filtre
`trigger_type='callback' AND status='in_progress' AND process_id IS NOT NULL`.
Mesure sur la base de production (`~/.mika/data/mika.db`, 2026-09-09) :

```
sqlite> SELECT status, trigger_type, count(*) FROM tasks
        WHERE process_id IS NOT NULL GROUP BY 1,2 ORDER BY 3 DESC;
delivered|callback|876
cancelled|callback|19
failed|callback|1
pending|callback|1
```

**Zéro row `in_progress`.** Pas « peu » : aucune, sur les 897 dispatches de
l'historique. La population du reaper est vide par construction.

Le cycle de vie l'explique. `build_callback_task` (`skills/executor.rs:2810`)
ne pose pas de `status` → `create_task` écrit `pending`. C'est le **parent**
(la row manuelle) qui passe `in_progress` (`executor.rs:2980`), pendant que
l'enfant callback — celui sur lequel `spawn_long_running_exec` estampille
`process_id` et `process_start_time` (`executor.rs:3192`) — **reste `pending`**
jusqu'à ce que le callback atterrisse et le fasse passer `delivered`.

Les deux zombies du ticket le confirment nommément : pgid `1926933` et
`1934255` vivaient sur des rows `pending` pendant leurs ~50 min de silence, et
portent aujourd'hui `delivered`.

Écho direct du fantôme #2263b : un comptable posé sur la mauvaise row.

### 2. La disposition est désarmée

`DEFAULT_PILOT_STALL_REAP_ENABLED = false` (Décision 4 de #2249). Même la row
trouvée, le reaper observerait sans faucher.

**Les deux causes composent en une seule apparence** : « rien dans les logs ».
Corriger une seule laisse le reaper inerte, ce qui rend la vérification par
paire obligatoire (un test qui n'épingle que la population passerait encore
avec un flag désarmé, et réciproquement).

## Le fix

| # | Où | Quoi |
|---|-----|------|
| 1 | `db.rs` | `get_live_dispatch_callback_tasks_with_pid` — `status IN ('pending','in_progress')`. **Nouvelle** méthode, pas un élargissement de `get_active_callback_tasks_with_pid` : le watchdog #959 partage cette dernière et son rayon d'action (marquer `failed` un process mort) ne doit pas bouger dans ce ticket. |
| 2 | `async_db.rs` | Le wrapper correspondant. |
| 3 | `engine.rs` | Le reaper consomme la nouvelle méthode ; le terme 6 (re-lecture) accepte les deux status vivants au lieu du seul `in_progress` ; l'`audit_event` porte le status **réellement observé** en `before_value` au lieu d'un `in_progress` codé en dur. |
| 4 | `config.rs` | `DEFAULT_PILOT_STALL_REAP_ENABLED = true`. L'observation devient un mode explicite (`MIKA_PILOT_STALL_REAP_ENABLED=0` ou `pilot_stall_reap_enabled = false`). |

### Sur l'armement — la condition de bascule de #2249 et pourquoi elle ne s'applique plus

#2249 posait : armer une fois ≥3 rows `pilot_silent_stall` revues sans faux
positif. Cette condition est **insatisfiable** telle quelle : elle compte les
rows d'un détecteur dont la population est vide. Zéro row depuis le déploiement
n'est pas un signal de prudence, c'est l'absence de mesure.

Ce que la garde achète est réel — un faux positif détruit des heures de travail
dans un worktree décision-core — et il reste payé par trois choses qui, elles,
ne changent pas : les termes 3-4 (worktree **déclaré**, chemin **existant**,
mtime lisible — toute absence sort de la population plutôt que d'y entrer), le
seuil de 2700 s validé contre son contrôle négatif mesuré (le run sain
`c3f9a2f9`, 24 min d'écart inter-écriture), et `MIKA_PILOT_STALL_REAP_ENABLED=0`
qui désarme sans rebuild.

Ce que ce ticket ajoute, et que #2249 n'avait pas : un **contrôle positif réel**
— un pilote authentiquement vivant, sur la vraie forme de row de production, est
trouvé et tué par le mécanisme complet. C'est le n=1 mesuré qui manquait.

La décision d'armer est celle de Vincent sur #2272, prise en connaissance de
l'asymétrie. Elle est consignée ici, pas déduite.

## Tests — le contrôle positif est un pilote RÉEL

La leçon du reaper inerte est que la fixture a fabriqué la population qui
faisait passer le test : `seed_dispatch` posait `update_task_status(id,
"in_progress")`, une transition que la production n'écrit jamais sur cette row.
Un synthétique de plus reproduirait exactement la faute.

1. **Forme de production épinglée** (`test_pilot_silent_stall_reaper.rs`) : la
   fixture passe par `build_callback_task` + `create_task`, le **chemin
   d'écriture de production**, et assert que la row qui porte le pid est
   `pending`. Ce test est le rouge-avant de la cause 1 : il échoue si quelqu'un
   remet un `in_progress` synthétique.
2. **Intégration réelle, harness multi-agents #2265**
   (`test_reaper_reaps_live_pending_pilot_2272.rs`) : `spawn_live_child` donne
   un processus authentiquement vivant, son pid est posé sur la row
   pending-callback de `mika-dev` via le chemin de production, le worktree est
   déclaré et antidaté. Le scan du moteur de `mika-dev` **trouve** la row, le
   processus est **mort** après le tick, la row est `failed`, l'audit
   `pilot_silent_stall` est émis.
   Contrôle d'attribution que seul le harness multi-agents rend exprimable : le
   moteur de `mika-qa`, sur la **même base partagée**, ne fauche pas la row de
   `mika-dev`.
3. **Défaut armé** : `Settings::load` sans surcharge → la disposition tire.
   Contrôle négatif apparié dans le même fichier : `Some(false)` → l'audit est
   écrit, la row n'est pas touchée, le process est toujours vivant.
4. Les six contrôles négatifs d'AC4 de #2249 restent verts, portés sur la
   nouvelle forme de row.

## Fire-Disposition

**(c) halte-et-remontée, gate CI bloquant** pour tous les tests ci-dessus. Un
rouge signifie que le reaper est redevenu inerte, ou qu'il fauche du vivant —
les deux demandent un humain, aucun ne se remédie automatiquement.

## Hors scope (à ficher séparément, évidence en main)

Le watchdog #959 `check_callback_process_liveness` filtre `in_progress` sur la
**même** requête et est donc aveugle pour la même raison structurelle. Le
corriger change qui marque `failed` une row dont le process est mort, ce qui a
son propre rayon d'action et sa propre course avec le moniteur de spawn. Ticket
distinct, pas un bundle.
