---
title: "« database is locked » malgré busy_timeout : réessayer l'unité idempotente, hors du thread base, classée par la variante"
module: mika-agent/db
date: 2026-09-30
problem_type: database_issue
component: database
severity: medium
symptoms:
  - "WARN `failed to register recurring task label=heartbeat error=\"database is locked\"` à chaque redémarrage du démon (n=5 du 2026-09-29 au 2026-09-30)"
  - "La ligne ne nomme pas l'agent : impossible de savoir quel agent a perdu son enregistrement"
  - "Une récurrente dont l'enregistrement échoue reste absente jusqu'au redémarrage suivant, sans bruit"
root_cause: concurrency
resolution_type: code_fix
related_components:
  - task_engine
  - async_db
tags:
  - sqlite
  - sqlite-busy
  - busy-timeout
  - wal
  - retry
  - recurring-tasks
  - startup
  - error-classification
---

# « database is locked » malgré busy_timeout : réessayer l'unité idempotente, hors du thread base, classée par la variante

## Problem

L'enregistrement des tâches récurrentes au démarrage (`ensure_recurring_task`)
échouait sur `database is locked` à chaque redémarrage, en WARN, sans réessai
et sans `agent_id` (mika#2601). Le défaut est latent tant que les lignes
existantes survivent au redémarrage. Il devient une perte muette le jour où
l'enregistrement doit **recréer** une ligne : c'est le chemin que le
ré-armement au démarrage de mika#2575 / PR #2599 emprunte.

## Symptoms

- `WARN failed to register recurring task label=heartbeat error="database is locked"`
  dans `/var/log/mika/server.log`, une fois par redémarrage, 5 fois sur 5.
- Aucun `agent_id` sur la ligne.
- Aucune récurrente en `failed` après coup : rien ne signale la perte si elle
  a lieu.

## What Didn't Work

Les deux remèdes que le ticket proposait ne tiennent pas à la lecture du code
(plan `docs/plans/2026-09-30-002-fix-2601-…-plan.md`, § R1 à R3) :

- **« Poser un `busy_timeout` suffisant sur cette connexion. »** Il est déjà
  posé : `Database::open` exécute `PRAGMA busy_timeout = 5000` sur chaque
  connexion (`crates/mika-agent/src/db.rs:1084`). Le texte de l'erreur le
  confirme : `sqlite3_errmsg(SQLITE_BUSY)` rend `"database is locked"`, alors
  que `SQLITE_LOCKED` rend `"database table is locked"`. La ligne mesurée est
  donc un `SQLITE_BUSY` **après** expiration des 5 s. Un autre écrivain a tenu
  le verrou plus longtemps. Par ailleurs, `busy_timeout` est une propriété de
  la **connexion**, pas d'un appel. La connexion d'un agent sert aussi sa
  boucle de tick et ses tours. Le relever pour l'enregistrement revient à le
  relever pour tout le monde (refus D2 du plan).
- **« Supprimer la concurrence en partageant une connexion entre agents. »**
  Dans le démon, chaque agent ouvre sa propre connexion sur le même fichier
  (`Database::open` dans `init_agent`, `crates/mika-agent/src/server/mod.rs:582`).
  Mais le CLI `mika` ouvre aussi sa propre connexion sur ce fichier
  (`open_db` → `Database::open`, `crates/mika-cli/src/init.rs:174`, et
  `run_team` dans `crates/mika-cli/src/commands/chat.rs:1061`). La contention est donc
  **inter-processus** : aucune consolidation intra-processus ne la retire
  (refus D3).
- **« Deux agents s'enregistrent en même temps. »** Faux au sens littéral :
  la boucle d'enregistrement de `run_server` est séquentielle. Ce qui tourne
  en parallèle, ce sont les tâches **détachées** des agents déjà initialisés :
  leur boucle de tick, et `startup_cleanup`, qui compacte les événements
  d'audit puis lance `vacuum()` si des lignes ont été supprimées
  (`crates/mika-agent/src/server/mod.rs:791-797`). `VACUUM` est le candidat
  principal au verrou tenu plus de 5 s. C'est une hypothèse nommée par le
  plan, **non mesurée**.

## Solution

Correctif ouvert dans la PR #2605, non mergée à la date de rédaction.

1. **Classer par la variante, au code primaire.** `db::is_sqlite_busy`
   (`crates/mika-agent/src/db.rs`) fait un `downcast_ref::<rusqlite::Error>()`
   sur la chaîne de causes de l'`anyhow::Error`, comme son voisin
   `is_unique_violation` :

   ```rust
   pub fn is_sqlite_busy(err: &anyhow::Error) -> bool {
       matches!(
           err.downcast_ref::<rusqlite::Error>(),
           Some(rusqlite::Error::SqliteFailure(e, _))
               if e.code == rusqlite::ErrorCode::DatabaseBusy
                   || e.code == rusqlite::ErrorCode::DatabaseLocked
       )
   }
   ```

   `e.code` est le code **primaire**. Il attrape `SQLITE_BUSY_SNAPSHOT` (517),
   que tester `extended_code` raterait. Le message n'est jamais lu : un
   `anyhow!("database is locked")` nu est refusé, et un test l'épingle.

2. **Réessayer hors du thread base.** `retry_on_busy`
   (`crates/mika-agent/src/task_engine/mod.rs`) est une boucle `async` qui
   dort avec `tokio::time::sleep`. Réessayer dans
   `Database::create_recurring_task_if_absent` aurait dormi **sur le thread
   base** de l'agent, et bloqué toutes les fermetures en file derrière,
   boucle de tick comprise (refus D4).

3. **Réessayer l'unité entière, parce qu'elle est idempotente.**
   `try_ensure_recurring_task` enchaîne quatre appels base. Une erreur busy sur
   n'importe lequel avorte la tentative, et l'enregistrement **entier** est
   rejoué. C'est sûr sans raisonner sur une écriture partielle : le `WHERE` du
   revert exclut les lignes déjà traitées, `create_recurring_task_if_absent`
   fait deux `SELECT` puis un `INSERT OR IGNORE` avec l'écriture en dernier,
   et `update_recurring_task_cron` écrit une valeur fixe. Toute autre erreur
   garde sa disposition antérieure.

4. **Budget borné, abandon bruyant.** Trois tentatives, avec 250 ms puis
   1000 ms d'attente. Chaque tentative porte déjà ses 5 s de `busy_timeout`,
   donc le pire cas par label est d'environ 16 s. Ce budget absorbe un pic ; il
   ne survit pas à un `VACUUM` long, et c'est voulu. À l'épuisement :
   `error!` avec `event = recurring_registration_failed`, `agent_id`, `label`,
   `attempts`, `waited_ms` et `busy`. Une réussite après réessai émet un `info!`
   `recurring_registration_retried`. Ce second événement est la sonde : il
   distingue « pas de contention » de « classifieur inerte ».

5. **ERROR dans le journal, pas de ligne `audit_events`.** L'`INSERT` d'audit
   passerait par la connexion dont on rapporte la contention. Sur un échec
   définitif, le verrou est encore tenu par construction : l'audit brûlerait
   5 s de plus pour échouer à son tour.

## Why This Works

WAL n'admet qu'un écrivain à la fois sur le fichier. Plusieurs connexions, dans
un processus ou dans plusieurs, se disputent ce verrou. `busy_timeout` n'est
qu'une attente passive, bornée, et par connexion. Tant que le fichier est
partagé entre processus, un `SQLITE_BUSY` après expiration reste possible :
aucune valeur de `busy_timeout` ne le rend impossible. Le réessai n'est donc pas
un palliatif. C'est le traitement correct d'une erreur transitoire par
conception, à condition de trois choses :

- l'unité rejouée est idempotente ;
- le sommeil a lieu hors du thread qui sert la connexion ;
- la classification lit la variante, pour ne jamais rejouer une vraie erreur.

## Prevention

- **Tout nouveau site d'écriture au démarrage** (ou sur un chemin qui ne se
  rattrape pas avant le redémarrage suivant) doit se demander : que se
  passe-t-il sur un `SQLITE_BUSY` après 5 s ? La même classe reste ouverte sur
  `cancel_recurring_task_by_label` (refus D6 du plan, conséquence inverse :
  un scan désactivé qui continue de tirer).
- **Un seul site d'enregistrement.** Le test de scan
  `mika2601_la_registration_recurrente_a_un_seul_appelant_et_deux_ecrivains`
  (`crates/mika-agent/src/canonical_tokens.rs`) échoue si un second site de
  production appelle `create_recurring_task_if_absent` sans passer par
  `ensure_recurring_task`. Un tel site perdrait le réessai sans qu'aucun test
  comportemental ne rougisse.
- **Pour tester une contention réelle** (`crates/mika-agent/src/task_engine/mod.rs`, tests
  `mika2601_*`) :
  - base **sur fichier** (`tempfile`), jamais `:memory:` ;
  - un thread avec une `rusqlite::Connection` nue prend `BEGIN IMMEDIATE`,
    puis **signale par canal** qu'il tient le verrou, avant que le test
    lance l'enregistrement. Sans ce rendez-vous, le registrant peut écrire
    avant le bloqueur, et le test passe sans avoir rencontré de contention ;
  - abaisser `busy_timeout` à 200 ms sur les connexions registrantes, sinon
    les 5 s de production absorbent le verrou tenu et le réessai n'est
    jamais exercé ;
  - `PRAGMA busy_timeout = N` **rend une ligne** : `execute` échoue avec
    `ExecuteReturnedResults`. Passer par `query_scalar::<i64>` ;
  - contrôle négatif : le **même** montage, via
    `ensure_recurring_task_with_policy` avec un budget d'une tentative. Aucune
    ligne `recurring_active`, et l'appel rend `RegistrationFailure`. Même
    chemin de code, budget différent : pas de simulation.
- **Après déploiement**, un `grep recurring_registration_failed` vide ne prouve
  rien seul. Il faut aussi un contrôle positif : des `registered recurring task`
  présents au même démarrage.

## Related

- mika#2601 (ticket), PR #2605 (correctif)
- mika#2575 / PR #2599 : ré-armement au démarrage, le chemin qui rend ce
  défaut conséquent
- [`consolidate-per-agent-team-dbs-into-single-container-db.md`](consolidate-per-agent-team-dbs-into-single-container-db.md) :
  origine de la topologie un fichier / WAL / `busy_timeout=5000`
- [`../runtime-errors/tache-a2a-orpheline-appelant-raccroche-avant-etat-terminal.md`](../runtime-errors/tache-a2a-orpheline-appelant-raccroche-avant-etat-terminal.md) :
  autre écriture terminale qui doit survivre à un `SQLITE_BUSY`
