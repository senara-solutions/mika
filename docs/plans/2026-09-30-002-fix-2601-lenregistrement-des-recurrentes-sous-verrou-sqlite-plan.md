# L'enregistrement d'une récurrente réessaie sous verrou, et son échec définitif se nomme (mika#2601)

**Ticket :** senara-solutions/mika#2601
**Labels :** `bug`, `p2-normal`, `dispatch:loop`, `loop-substrate`
**Lignée :** mika#2575 / PR #2599 (ré-armement au démarrage), mika#1742 (garde de
ré-enregistrement), mika#2337 / mika#2446 (ses deux exemptions), mika#2205 (un
scan silencieusement inactif se lit comme un scan oisif).

---

## Constat

À chaque redémarrage récent, l'enregistrement d'une récurrente `heartbeat` meurt
sur un verrou SQLite (n=3, `/var/log/mika/server.log`) :

```
2026-09-29T10:00:48.336860Z WARN failed to register recurring task label=heartbeat error="database is locked" target=mika_agent::task_engine
2026-09-29T17:01:12.325550Z WARN failed to register recurring task label=heartbeat error="database is locked" target=mika_agent::task_engine
2026-09-30T09:31:08.697814Z WARN failed to register recurring task label=heartbeat error="database is locked" target=mika_agent::task_engine
```

Trois propriétés de cette ligne, et chacune est un défaut distinct :

1. **Elle ne réessaie pas.** `ensure_recurring_task`
   (`crates/mika-agent/src/task_engine/mod.rs:131`) avale l'erreur dans un
   `warn!` et rend la main. L'enregistrement est perdu jusqu'au redémarrage
   suivant.
2. **Elle ne nomme pas l'agent.** Aucun `agent_id` — et c'est vrai des **cinq**
   `warn!` de cette fonction, pas seulement de celui-là. Depuis le journal, on
   ne peut pas savoir lequel des agents a raté son enregistrement.
3. **Elle est en WARN alors que sa conséquence n'est pas rattrapable.** Voir
   § *La conséquence, et pourquoi elle justifie un ERROR* ci-dessous.

**Portée aujourd'hui.** Mesuré après le restart n°21 : toutes les lignes
`heartbeat` en base sont `recurring_active`. Aucun tir n'a été manqué **cette
fois**, parce que les lignes existantes ont survécu et que le `create` a rendu
« existe déjà ». Mais l'enregistrement au démarrage est précisément le chemin
qui **recrée** une récurrente dont la ligne n'existe plus — et c'est la classe
que mika#2575 vient de refermer par un autre chemin, rouverte ici.

---

## Trois rectifications que la lecture du code impose au ticket

### R1 — `busy_timeout` est déjà posé, et l'erreur mesurée dit qu'il a été épuisé

`Database::open` (`db.rs:1048`) pose `PRAGMA busy_timeout = 5000` sur **chaque**
connexion. Le ticket propose comme alternative « ou `busy_timeout` suffisant sur
cette connexion » : il y est déjà, et il n'a pas suffi.

Mieux : le message **identifie le code**. `sqlite3_errmsg(SQLITE_BUSY)` rend
`"database is locked"` ; `SQLITE_LOCKED` rend `"database table is locked"`, une
chaîne différente. La ligne mesurée est donc un **`SQLITE_BUSY` après expiration
du `busy_timeout`** — pas un conflit intra-connexion, pas un `SQLITE_LOCKED`.
Le verrou d'écriture a été tenu **plus de 5 secondes** par un autre écrivain.

Corollaire, et il invalide la lettre de l'alternative : **`busy_timeout` est une
propriété de la connexion, jamais d'un appel.** On ne peut pas le « rendre
suffisant sur cette connexion » pour l'enregistrement seul : la connexion de
l'agent sert aussi sa boucle de tick, son drain webhook et chacun de ses tours.
Le relever, c'est le relever pour tout le monde — voir le refus D2.

### R2 — le défaut n'est pas « le démarrage concurrent de plusieurs agents » au sens où le ticket l'écrit

L'enregistrement est une boucle **séquentielle** sur les agents
(`server/mod.rs:1758-2035`), un seul `await` à la fois : deux agents ne
s'enregistrent jamais en même temps. Ce qui est concurrent est autre chose, et
c'est lisible dans la forme de la boucle. Pour **chaque** agent, dans l'ordre :

| # | étape | effet |
|---|---|---|
| 1 | 7 × `ensure_recurring_task` | écritures (le site du défaut) |
| 2 | `startup_recovery` | grosse transaction d'écriture |
| 3 | `prune_old_tasks` | DELETE sur 30 jours |
| 4 | `spawn_tick_loop` | **boucle d'une seconde, qui écrit, détachée** |
| 5 | `tokio::spawn(startup_cleanup(...))` | **détachée** : prune, `compact_old_audit_events(90)`, et **`vacuum()`** si des lignes ont été supprimées |

Donc l'agent n°1 a sa boucle de tick **et** son nettoyage de démarrage en vol
pendant que les agents 2..N s'enregistrent encore. Et chaque agent possède sa
**propre connexion** : `init_agent` fait un `Database::open` par agent
(`server/mod.rs:584`), sur le **même fichier** `~/.mika/data/mika.db`. N
connexions, N threads, un seul verrou d'écriture WAL.

`VACUUM` prend un verrou exclusif et peut tenir des dizaines de secondes sur une
base de plusieurs centaines de mégaoctets. C'est le **candidat principal** au
tenant de verrou > 5 s — hypothèse nommée, pas mesurée : voir *Hors périmètre*.

### R3 — la contention est une propriété de conception, donc irréductible

On pourrait croire que la vraie réparation est de supprimer la concurrence
intra-processus : `AsyncDatabase::with_agent` existe déjà et **partage** le
thread et la connexion, donc faire descendre tous les agents d'une seule
connexion sérialiserait les écritures et supprimerait la classe.

Ça ne marche pas, et pour une raison qui décide du remède : **la base est
partagée entre processus**. `mika chat` ouvre sa propre connexion sur le même
fichier (`chat.rs:1061`), et mika#2575 documente noir sur blanc que le CLI
exécute le même balayage de démarrage contre la base du démon. Aucune
consolidation intra-processus ne peut retirer la contention.

**Donc le réessai n'est pas un palliatif : c'est le remède correct**, et la
consolidation de connexions serait un arbitrage débit-vs-contention sur le
chemin chaud de chaque tour d'agent, hors sujet ici (refus D3).

---

## Le remède

### La classification vient de la VARIANTE, jamais du texte rendu

Doctrine maison, écrite trois fois (mika#2179, mika#2289, mika#2522) : une
classe d'erreur se lit sur la variante. Et le précédent exact existe **dans le
même fichier** : `db::is_unique_violation` (`db.rs:170`) descend par
`downcast_ref::<rusqlite::Error>()` sur la chaîne de causes d'un
`anyhow::Error`.

```rust
/// Un `SQLITE_BUSY` / `SQLITE_LOCKED` : le verrou d'écriture était tenu
/// ailleurs quand cette instruction a voulu écrire (mika#2601).
pub fn is_sqlite_busy(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::DatabaseBusy
                || e.code == rusqlite::ErrorCode::DatabaseLocked
    )
}
```

Trois choses que ce site doit tenir, et qui sont établies par lecture :

- **Le `downcast` traverse la frontière `AsyncDatabase`.** `with_db`
  (`async_db.rs:149-172`) transmet le `Result` **verbatim** (`tx.send(f(db))`,
  puis `rx.await.map_err(...)?`), et `create_recurring_task_if_absent` produit
  ses erreurs par `?` sur `rusqlite`. Preuve empirique : `is_unique_violation`
  est déjà consommé en production sur une erreur qui a traversé exactement cette
  frontière (`tools/create_task.rs:320`). **Aucune plomberie nouvelle.**
- **Le code PRIMAIRE, pas l'étendu.** `SQLITE_BUSY_SNAPSHOT` (517) a pour code
  primaire `SQLITE_BUSY`, donc `ErrorCode::DatabaseBusy` l'attrape. Tester
  `extended_code` (ce que fait son voisin, pour une bonne raison qui est la
  sienne) raterait cette famille.
- **`DatabaseLocked` est inclus** parce que sa population est bornée par le
  budget de réessai et que son coût en faux réessai est de trois tentatives.
  Ce n'est pas le code mesuré (cf. R1), c'est le voisin de famille.

### L'unité réessayée est l'enregistrement ENTIER, parce qu'il est idempotent

`ensure_recurring_task` fait quatre appels base :
`revert_config_cancel_recurring_task` (écriture),
`create_recurring_task_if_absent` (lecture ×2 puis écriture),
`get_recurring_task_cron` (lecture), `update_recurring_task_cron` (écriture).

**Chacun est idempotent, et c'est ce qui autorise à rejouer le tout plutôt qu'à
raisonner sur une écriture partielle :**

- `revert_config_cancel_recurring_task` : son `WHERE` exclut les lignes portant
  déjà le marqueur, donc un second passage rend `n = 0` (`db/tasks.rs:407-425`).
- `create_recurring_task_if_absent` : c'est sa raison d'être — deux `SELECT`
  puis un `INSERT OR IGNORE`, en autocommit, **l'écriture en dernier**. Un échec
  laisse la base intacte.
- `update_recurring_task_cron` : un `UPDATE` vers une valeur fixe.

**Une règle, un classifieur, quatre sites :** *une erreur busy avorte la
tentative (donc l'enregistrement entier est rejoué) ; toute autre erreur garde
la disposition d'aujourd'hui.* C'est un sur-ensemble strict du comportement
actuel — aucune régression possible sur les erreurs non-busy, qui continuent
d'être avalées exactement comme avant.

**Un trou trouvé en chemin, et fermé au passage :** l'erreur de
`get_recurring_task_cron` est aujourd'hui **entièrement muette**
(`if let Ok(Some(existing_cron)) = …`). Elle gagne un `warn!` — même classe,
coût nul, et sans lui un cron qui ne se relit pas se lit comme un cron en phase.

### La forme

```
pub async fn ensure_recurring_task(db, label, cron_expr, action_config)      // signature INCHANGÉE
    → ensure_recurring_task_with_policy(db, …, RecurringRetryPolicy::production())

pub(crate) async fn ensure_recurring_task_with_policy(…, policy)
    → boucle de réessai ; émet les deux événements ; rend Result<(), RegistrationFailure>

async fn try_ensure_recurring_task(db, label, cron_expr, action_config)
    → Result<()>   // le corps d'aujourd'hui, avec la règle ci-dessus aux quatre sites
```

Deux propriétés du découpage :

- **La signature publique ne change pas**, donc **zéro diff** aux huit sites
  d'appel (sept dans `run_server`, un dans `rearm_recurring_task`). C'est la
  discipline de périmètre : le ticket porte sur le réessai, pas sur la forme des
  appelants.
- **La politique est injectable**, ce qui donne au contrôle négatif d'AC3 le
  **même chemin de code** que la production, au lieu d'une simulation. Idiome
  maison des points d'entrée `*_with_deadline` réservés aux tests.

### Les nombres, et leur arithmétique

```rust
/// Trois tentatives, entrecoupées de 250 ms puis 1000 ms (mika#2601).
///
/// **Chaque tentative porte déjà les 5 s de `busy_timeout` de la connexion**, donc
/// le compte est un multiplicateur sur une unité de 5 s, pas sur rien :
///   - régime nominal (verrou libre)  : 0 ms ajouté, une tentative
///   - le cas mesuré (un pic)         : ≤ 5,25 s, deuxième tentative servie
///   - pire cas par label             : 3 × 5 s + 1,25 s ≈ 16,25 s
///
/// Pire cas sur un démarrage à 4 agents × 7 labels : ≈ 7 min 35 s, contre
/// ≈ 2 min 20 s aujourd'hui — et aujourd'hui les scans meurent pour 24 h. Un
/// démarrage de sept minutes est bruyant par lui-même, et chaque label produit
/// une ligne ERROR.
const PRODUCTION_ATTEMPTS: u32 = 3;
const PRODUCTION_BACKOFFS: [Duration; 2] = [ms(250), ms(1000)];
```

**Ce budget absorbe un pic ; il ne peut pas survivre à un `VACUUM`**, et il faut
le dire plutôt que de le laisser découvrir. Un verrou tenu 60 s ferait échouer
les trois tentatives de chaque label — on aurait payé sept minutes pour rien.
Le choix est donc délibéré : *absorber le pic à bas coût, abandonner fort*. La
population mesurée est **un** label par redémarrage, c'est-à-dire un pic. La
sonde S3 est ce qui distingue les deux régimes, et la cause-racine du régime
soutenu a son propre suivi.

### La conséquence, et pourquoi elle justifie un ERROR

Un échec définitif n'est **pas rattrapable autrement que par un redémarrage** :

- aucune ligne n'a été créée, donc rien ne tire ;
- le balayage des 60 ticks ne réenregistre aucune récurrente ;
- `mika tasks rearm <label>` exige une ligne **morte** et refuse
  `RearmError::NoDeadRow` — *« a rearm never creates a recurrence ex nihilo »*.

C'est exactement l'argument d'AC2 rendu concret : un WARN convient à ce qui se
répare tout seul, pas à un scan mort jusqu'au prochain redémarrage.

### Deux événements, et le second est la sonde

```
recurring_registration_retried   INFO   régime attendu : NON VIDE, faible
recurring_registration_failed    ERROR  régime attendu : VIDE
```

Champs des deux : `agent_id`, `label`, `cron`, `attempts`, `waited_ms` ; plus
`error` et `busy` (booléen) sur l'échec. Convention d'émission :
`event = CONST` en champ, prose en message — la forme exacte de
`RECURRING_RESTORED_AFTER_RESTART_EVENT` (mika#2575, `engine.rs:263`).

**La ligne INFO n'est pas décorative : c'est la mesure directe que le correctif
mord.** Sans elle, « aucune contention n'a eu lieu » (le bon état) et « le
classifieur est inerte » rendent des octets identiques — classe mika#2205, que
ce dépôt a payée quatre fois.

Les cinq `warn!` existants de la fonction gagnent `agent_id` (AC2 s'applique à
la classe, pas au seul site cité).

### Aucune ligne `audit_events`, et le refus est chiffré

AC2 offre le choix (« audit **ou** ERROR ). L'audit est refusé, sur un coût
concret : **la ligne d'audit s'écrirait par la connexion dont on rapporte la
contention.** Sur un échec définitif, le verrou est **encore** tenu par
construction (s'il s'était libéré, la tentative 3 aurait abouti). L'`INSERT`
d'audit brûlerait donc son propre `busy_timeout` — **+5 s sur le chemin
d'échec** — pour finir par échouer. On allongerait le démarrage afin de produire
une ligne qui, systématiquement, n'atterrit pas : un `SELECT count(*)` lirait
bas et se lirait sain.

Un instrument, honnête : l'ERROR, qui part dans le fichier de journal par un
écrivain que le verrou ne touche pas.

---

## Ce qui est refusé, avec son motif

**D1 — un compteur/budget configurable par variable d'environnement.** Précédent
maison cité mot pour mot pour `GH_AUTH_PROBE_TIMEOUT` : *« deliberately not
env-configurable — the same YAGNI rule »*. Et un `0` sur un budget de réessai
serait un désarmement silencieux d'un filet de sûreté. Constantes nommées,
arithmétique dans le doc-comment.

**D2 — relever `busy_timeout` globalement.** Trois coûts : le rayon de souffle
est **chaque instruction de chaque connexion** (y compris les handlers HTTP, qui
ont leurs propres budgets) ; un écrivain réellement coincé tiendrait tout le
monde plus longtemps ; et `docs/runtime-structure.md` documente la valeur, donc
un `docs-sync`. Surtout, ça ne borne rien : le réessai est ce qui fait la
différence entre « l'enregistrement a eu lieu » et « il a mis plus longtemps à
échouer ».

**D3 — une seule connexion pour tous les agents** (via `with_agent`). Réfuté par
R3 : la contention inter-processus subsisterait. Et ce serait un arbitrage
débit-vs-contention sur le chemin chaud de chaque tour, sans mesure.

**D4 — réessayer dans `Database::create_recurring_task_if_absent`.** Cette
fonction tourne **sur le thread base** de l'agent : y dormir bloque toutes les
fermetures en file pour cet agent, boucle de tick comprise. Le réessai doit être
asynchrone, hors de ce thread.

**D5 — différer l'enregistrement raté au balayage des 60 ticks.** C'est le
remède le plus robuste, et il est hors périmètre pour une raison structurelle :
la liste des labels n'existe nulle part comme valeur. Elle vit dans la boucle de
`run_server`, avec ses conditions par agent (`heartbeat_enabled_for_agent`, le
cron curateur de l'`identity.toml`, `name == "mika-dev"`, quatre knobs
d'environnement). La reconstruire dans le moteur, c'est la classe
`grooming_marker` (mika#2158) — deux lecteurs d'une même question, libres de
diverger. **Suivi nommé, avec sa précondition.**

**D6 — le réessai sur `cancel_recurring_task_by_label`** (les branches `else`
des knobs). Même classe, conséquence **inverse** : un cancel raté laisse une
ligne qui *continue* de tirer un scan désactivé — bruyant, pas muet — et borné
par le prochain redémarrage. Population distincte, hors périmètre.

---

## Requirements

- **R-1** `db::is_sqlite_busy(&anyhow::Error) -> bool`, à côté de
  `db::is_unique_violation`, classifiant sur `ErrorCode::DatabaseBusy` /
  `DatabaseLocked` (code **primaire**), jamais sur le message rendu.
- **R-2** `ensure_recurring_task` conserve sa signature publique et délègue à
  `ensure_recurring_task_with_policy(…, RecurringRetryPolicy::production())`.
  Zéro diff aux huit sites d'appel.
- **R-3** `try_ensure_recurring_task` porte le corps d'aujourd'hui avec, aux
  quatre appels base : *busy ⇒ `Err` (tentative avortée) ; autre erreur ⇒
  disposition d'aujourd'hui*. L'erreur de `get_recurring_task_cron`, aujourd'hui
  muette, gagne un `warn!`.
- **R-4** La boucle de réessai : 3 tentatives, backoff 250 ms puis 1000 ms,
  constantes nommées portant l'arithmétique en doc-comment. Aucune variable
  d'environnement.
- **R-5** `recurring_registration_retried` (INFO) émis **une fois** quand une
  tentative > 1 aboutit ; `recurring_registration_failed` (ERROR) émis **une
  fois** sur abandon. Champs : `agent_id`, `label`, `cron`, `attempts`,
  `waited_ms` (+ `error`, `busy` sur l'échec). Chaque nom a **un seul** site
  d'émission en production.
- **R-6** Les cinq `warn!` existants de `ensure_recurring_task` portent
  `agent_id`.
- **R-7** Aucune ligne `audit_events` (refus chiffré ci-dessus). Aucune
  migration, aucune valeur de réglage déplacée, `busy_timeout` inchangé.
- **R-8** Scan de source `mika2601_la_registration_recurrente_a_un_seul_appelant_et_deux_ecrivains`,
  allowlist livrée **vide**, avec assertion d'anti-vacuité.

---

## Fichiers touchés

| fichier | nature |
|---|---|
| `crates/mika-agent/src/db.rs` | +`is_sqlite_busy` (~15 lignes, à côté de `is_unique_violation`) |
| `crates/mika-agent/src/task_engine/mod.rs` | découpage en trois, boucle de réessai, deux constantes d'événement, `agent_id` sur cinq `warn!`, `warn!` sur la lecture de cron |
| `crates/mika-agent/src/task_engine/mod.rs` (`mod tests`) | tests de la boucle + contention réelle + contrôle négatif |
| `crates/mika-agent/src/db/tests/` | tests du classifieur, à côté de `test_is_unique_violation_only_catches_unique` |
| `crates/mika-agent/src/canonical_tokens.rs` | le scan R-8 |
| `crates/mika-agent/CLAUDE.md` | § *L'enregistrement d'une récurrente réessaie sous verrou* |
| `CLAUDE.md` (racine) | surfaces opérateur, sondes, haltes — après le § mika#2575 |
| `docs/solutions/best-practices/…-2026-09-30.md` | entrée compound (hors périmètre `docs-sync`, vérifié) |

**Zéro diff** dans `server/mod.rs`, `async_db.rs`, `db/tasks.rs`.

---

## Verification Contract

### V1 — le classifieur (unitaire, `db/tests/`)

- `mika2601_un_busy_est_reconnu_a_travers_la_chaine_anyhow` — un
  `rusqlite::Error::SqliteFailure(ErrorCode::DatabaseBusy)` enveloppé par
  `anyhow::Error::from` **puis** par deux `.context(...)` est reconnu. **Le terme
  porteur est le `context`** : c'est ce qui atteste que le `downcast` traverse
  la chaîne de causes, donc la frontière `with_db`.
- `mika2601_snapshot_est_un_busy` — code primaire `DatabaseBusy` avec
  `extended_code = SQLITE_BUSY_SNAPSHOT` : reconnu.
- **Contrôles négatifs** (sans eux, « le classifieur décide » est
  indistinguable de « le classifieur rend toujours vrai ») : un
  `SQLITE_CONSTRAINT_UNIQUE` → non ; un `anyhow!("database is locked")` **nu** →
  **non**, ce qui épingle que la classification ne lit pas le texte.

### V2 — la boucle (unitaire, `task_engine/mod.rs`)

- Réessaie tant que l'erreur est busy et que le budget reste, s'arrête au
  premier `Ok`, **ne réessaie pas** une erreur non-busy, abandonne après N.
- `waited_ms` reflète la somme des backoffs réellement dormis.

### V3 — AC3 : contention réelle, deux agents (`task_engine/mod.rs`)

Une base **sur fichier** (`tempfile`), trois connexions :

1. un `rusqlite::Connection` nu (code de test) prend `BEGIN IMMEDIATE` et tient
   le verrou d'écriture ;
2. deux `AsyncDatabase` distincts (`Database::open` sur le même chemin,
   `agent_id` différents) — **la topologie de production exacte** ;
3. un fil relâche le verrou après ~600 ms.

`tokio::join!` sur deux `ensure_recurring_task` ; les deux doivent finir avec
une ligne `recurring_active`, une par agent.

**Le test abaisse `busy_timeout` à 200 ms sur les deux connexions
registrantes**, et il faut le dire : sans ça le `busy_timeout` de production
absorberait les 600 ms et **le réessai ne serait pas exercé du tout** — il
faudrait tenir le verrou > 5 s, soit un test de six secondes sur chaque `cargo
test`. Ce que le test mesure est le *mécanisme* (une erreur busy est classée,
réessayée, et l'enregistrement atterrit) ; l'arithmétique de production vit dans
le doc-comment des constantes, pas dans ce test.

*Piège d'implémentation :* `PRAGMA busy_timeout = N` **rend une ligne**, donc
`Database::execute_sql` (qui passe par `Connection::execute`) échoue avec
`ExecuteReturnedResults`. Passer par `query_scalar::<i64>`.

*Propriété fine que ce montage exerce :* en WAL, les deux `SELECT` de
`create_recurring_task_if_absent` **aboutissent** (les lecteurs ne sont pas
bloqués) et l'échec tombe sur l'`INSERT`. C'est très exactement la forme
mesurée en production.

- **Contrôle négatif d'AC3** (« sans le retry, le test rougit ») :
  `mika2601_sans_reessai_lenregistrement_est_perdu` — le **même** montage via
  `ensure_recurring_task_with_policy` à une seule tentative : aucune ligne
  `recurring_active`, et l'appel rend son `RegistrationFailure`. Pas une
  simulation : le même chemin de code, un budget différent.

### V4 — le scan de source (R-8)

Sur `canonical_tokens::production_sources()` :

- les occurrences de `create_recurring_task_if_absent(` **hors** des deux
  fichiers de plomberie (`db/tasks.rs` la définition, `async_db.rs` le wrapper)
  sont **exactement une**, et elle est dans `task_engine/mod.rs` ;
- chacun des deux noms d'événement est écrit à **exactement un** site de
  production ;
- **anti-vacuité** : chaque aiguille doit apparaître au moins une fois, sinon un
  renommage rend le scan vert en ne regardant plus rien (classe mika#2103 /
  mika#2205) ;
- l'allowlist est **vide**, épinglée vide par une assertion sœur.

*Aucun test comportemental ne peut voir cette classe :* un second site
d'enregistrement ne rend **aucune** décision fausse le jour où il est écrit —
l'enregistrement fonctionne, toutes les assertions restent vertes, et seul le
réessai disparaît, en silence.

*Vérifié avant d'écrire le scan :* la population de production est déjà de **1**
(`task_engine/mod.rs:101`). Les trois autres occurrences hors plomberie
(`server/mod.rs:4324`, `engine.rs:6238`, `dispatcher.rs:5049`) sont **toutes**
au-delà de la frontière `#[cfg(test)]` de leur fichier (respectivement 2030,
5645, 4647), donc hors de `production_sources()`.

*Note de mécanisme :* `production_sources()` tronque au **premier**
`#[cfg(test)]` du fichier. Dans `task_engine/mod.rs` il est en 409, après
`ensure_recurring_task` (54-133) : l'aiguille reste visible. Si un
`#[cfg(test)]` indenté apparaissait plus haut, le scan deviendrait aveugle —
c'est ce que l'assertion d'anti-vacuité attrape.

### V5 — non-régression

`cargo test -p mika-agent` ; en particulier les tests mika#1742 / mika#2271 /
mika#2337 / mika#2446 de `db/tests/recurring_tasks.rs` et les quatre tests
`ensure_recurring_task` de `task_engine/mod.rs` doivent passer **sans
modification** : la signature publique ne bouge pas et les erreurs non-busy
gardent leur disposition.

`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.

---

## Fire-Disposition

Ce plan livre des détecteurs (V1–V4, dont un scan de source). Option **(a) —
exception nommée en allowlist — avec une allowlist livrée VIDE**, parce que la
population de violations existantes a été recensée et elle est vide :

| détecteur | population existante | disposition |
|---|---|---|
| V1, V2, V3 | tests de code neuf | aucun, armés |
| V4, terme « un seul appelant » | **1** site, qui est le site attendu (`task_engine/mod.rs:101`) ; les 3 autres sont hors production (frontières `#[cfg(test)]` vérifiées) | **allowlist vide**, armé |
| V4, terme « un écrivain par événement » | noms **neufs** ⇒ exactement 1 chacun | **allowlist vide**, armé |

Trois garanties sur cette allowlist :

1. Elle est **épinglée vide** par une assertion sœur
   (`…_lallowlist_du_scan_est_livree_vide`) — une allowlist née vide est un
   tiroir où déposer la prochaine infraction (mika#2323).
2. **Quand le scan tire, on route le nouveau site par `ensure_recurring_task` ;
   on n'ajoute pas de ligne** (doctrine mika#2201). Un site qu'on ne veut pas
   armer est un site à supprimer.
3. L'assertion d'anti-vacuité est ce qui empêche le scan de se lire comme un
   arbre propre après un renommage.

Aucun détecteur n'est livré désarmé, aucune halte-et-remontée n'est requise.

---

## Acceptance criteria

- **AC1** — L'enregistrement des récurrentes au démarrage **réessaie** sur
  `SQLITE_BUSY` / `SQLITE_LOCKED` avec un backoff **borné** (3 tentatives,
  250 ms puis 1000 ms), au lieu d'abandonner en WARN. La classification vient de
  la variante d'erreur, jamais du message rendu. Attesté par V1, V2, V3.
- **AC2** — Un échec **définitif** nomme l'**agent** et le **label**, et laisse
  une trace exploitable : `recurring_registration_failed` en **ERROR**, avec
  `agent_id`, `label`, `cron`, `attempts`, `waited_ms`, `error`, `busy`. Les
  cinq `warn!` existants de la fonction portent aussi `agent_id`. Le choix
  ERROR-plutôt-qu'audit est chiffré au § correspondant.
- **AC3** — Test : un enregistrement concurrent sous verrou finit
  `recurring_active` **pour chaque agent** (V3, deux agents, deux connexions,
  verrou réellement tenu). **Négatif :** le même montage à une seule tentative
  rougit (`mika2601_sans_reessai_lenregistrement_est_perdu`).
- **AC4** — Aucune valeur de réglage n'est déplacée : `busy_timeout` reste à
  5000, aucune variable d'environnement n'est créée, aucune migration, aucune
  garde mika#1742 / mika#2271 / mika#2337 / mika#2446 n'est modifiée, exemptée
  ou contournée.
- **AC5** — Un réessai **réussi** est visible :
  `recurring_registration_retried` (INFO), sans quoi « aucune contention » et
  « le réessai est inerte » seraient indistinguables (classe mika#2205).

---

## Definition of Done

- [ ] R-1 … R-8 livrés.
- [ ] V1 … V5 verts ; chaque contrôle négatif **vu rouge** par mutation avant
      d'être déclaré (un terme fail-safe ne se prouve pas en les neutralisant
      tous ensemble — leçon mika#2277).
- [ ] `cargo test -p mika-agent`, `cargo clippy --all-targets -- -D warnings`,
      `cargo fmt --check` verts.
- [ ] `crates/mika-agent/CLAUDE.md` et `CLAUDE.md` racine à jour (surfaces,
      régimes attendus, sondes, haltes, ce que ça n'achète pas).
- [ ] Entrée compound dans `docs/solutions/best-practices/` (frontmatter YAML :
      `module`, `tags`, `problem_type`).
- [ ] Corps de PR nommant les trois rectifications (R1–R3) et les six refus
      (D1–D6).

---

## Surfaces opérateur

```bash
# 1. Un enregistrement a-t-il été sauvé par le réessai ? (régime attendu : NON VIDE, faible)
grep recurring_registration_retried "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{agent_id, label, attempts, waited_ms}'

# 2. Un enregistrement a-t-il été PERDU ? (régime attendu : VIDE)
grep recurring_registration_failed "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{agent_id, label, attempts, waited_ms, busy, error}'

# 3. CONTRÔLE POSITIF — le démarrage a-t-il seulement enregistré quelque chose ?
grep -c 'mika-spirit listening' "$MIKA_SPIRIT_LOG_FILE"
```

```sql
-- Les sept récurrentes attendues, après redémarrage
SELECT agent_id, label, status, next_fire_at FROM tasks
 WHERE trigger_type = 'recurring' AND status = 'recurring_active'
 ORDER BY agent_id, label;
-- mika-dev doit porter : heartbeat, reflection, curator_review,
-- auto_pull_groomed, wip_rescue, qa_review_reconcile, worktree_reap
```

| surface | niveau | régime attendu | lecture |
|---|---|---|---|
| `recurring_registration_retried` | INFO | **non vide, faible** | chaque ligne est un enregistrement que l'ancien code perdait — la mesure directe que le correctif mord |
| `recurring_registration_failed` | ERROR | **vide** | chaque ligne est un scan mort jusqu'au prochain redémarrage, et `agent_id` + `label` disent lequel |
| `busy = false` sur un `_failed` | ERROR | **vide** | l'échec n'est pas de la contention : lire `error`, c'est un autre défaut |
| `failed to register recurring task` **sans** `agent_id` | WARN | **vide** | le binaire servi est antérieur au correctif (classe mika#2340) |

---

## Sondes post-déploiement, et leurs cinq haltes

> **Préalable.** Ces mesures décrivent le **binaire servi**. Après `make deploy`,
> établir que le `mika-spirit` qui tourne porte le correctif — la présence du
> champ `agent_id` sur les WARN de cette fonction est le discriminant — **avant**
> toute conclusion (classe mika#2340).

**S1 — le défaut fondateur ne se rejoue plus (premier redémarrage).** La requête
SQL rend les sept labels pour mika-dev, et `recurring_registration_failed` est
vide.
**Halte 1 — un `_failed` sur `heartbeat` :** ne pas rallonger le budget par
réflexe. Lire `waited_ms` et `attempts` : si les trois tentatives ont brûlé
~5 s chacune, le verrou est tenu **longtemps** et c'est un `VACUUM` ou un gros
DELETE, pas un pic — le remède est le suivi cause-racine, pas ce budget.

**S2 — CONTRÔLE POSITIF du mécanisme (geste opérateur, une fois).** On ne peut
pas conclure d'un silence : il faut **provoquer** la collision. Depuis un autre
shell, tenir le verrou d'écriture pendant un redémarrage :

```bash
sqlite3 ~/.mika/data/mika.db   # session interactive
sqlite> BEGIN IMMEDIATE; SELECT 1;     -- puis, dans un autre shell : restart mika-spirit
sqlite> COMMIT;                        -- relâcher après ~3 s
```

Attendu : au moins une ligne `recurring_registration_retried`, et les sept
labels présents.
**Halte 2 — aucune ligne des deux côtés alors que le verrou était tenu :** le
classifieur est inerte. **Ne pas élargir le prédicat** — vérifier d'abord que
l'erreur traverse bien en `rusqlite::Error` (c'est ce que V1 épingle), puis que
le binaire servi porte le correctif. *Une garde que personne n'a exercée se lit
exactement comme une garde qui marche* (mika#2205).

**S3 — attribution du régime (30 jours).** Le compte de `_retried` rapporté au
nombre de redémarrages. Quelques lignes par redémarrage = le pic est absorbé,
c'est le régime visé.
**Halte 3 — `_retried` porte du trafic soutenu** (plusieurs lignes par label et
par redémarrage, sur tous les agents) : ce n'est pas un pic mais un état. C'est
**la mesure** qui ouvre le suivi cause-racine (`startup_cleanup` / `vacuum()`),
et elle s'ouvre **avec ce compte**, jamais avec une intuition.

**S4 — contrôle négatif de bruit (7 jours).** Aucun `_retried` et aucun
`_failed` hors d'une fenêtre de redémarrage.
**Halte 4 — une occurrence en régime établi :** un chemin autre que le démarrage
enregistre des récurrentes. Lire `agent_id` — un agent résolu paresseusement
(mika#1399) est la population légitime ; autre chose est à établir avant de
toucher quoi que ce soit.

**S5 — non-régression des gardes voisines (7 jours).**
`grep 'mika#1742: refusing to re-register' "$MIKA_SPIRIT_LOG_FILE"` garde son
régime d'avant.
**Halte 5 — il s'arme sur un label qu'un `_retried` vient de sauver :** le
réessai a fait aboutir un enregistrement que la garde aurait dû refuser, ou
l'inverse. **Désarmer par revert avant diagnostic** — les deux mécanismes se
composent par construction (le réessai rejoue la garde, il ne la contourne pas),
donc une interaction est un défaut de câblage, pas un seuil.

**Halte transverse — les deux sondes muettes.** Zéro réessai **et** zéro échec ne
prouve rien tant qu'une collision n'a pas eu lieu. Exécuter S2 avant toute
conclusion.

---

## Ce que ce travail n'achète PAS

- **Il ne supprime pas la contention.** Il la rend survivable sur un pic. La
  cause du verrou tenu > 5 s n'est pas identifiée par mesure ; elle a une
  hypothèse principale et un suivi.
- **Il ne survit pas à un `VACUUM`.** Dit au § arithmétique : un verrou tenu
  60 s fait échouer les trois tentatives. Le budget absorbe un pic, par choix.
- **Il ne rattrape pas les trois occurrences mesurées.** Rien ne rétro-écrit une
  ligne décrivant un réessai qui n'a pas eu lieu — la sonde est la **prochaine**
  occurrence.
- **Il ne réenregistre pas plus tard.** Un échec définitif reste réparable par un
  seul geste : redémarrer. C'est le suivi D5.
- **Il ne rend pas le champ surveillé, seulement lisible.** Aucun compteur,
  aucune ligne d'audit : les seuls instruments sont les greps et la requête
  ci-dessus, et **leur silence ne prouve rien tant que personne ne les
  exécute**, ni tant que S2 n'a pas établi le contrôle positif.

---

## Hors périmètre, délibérément

- **La cause-racine du verrou long.** Hypothèse principale, nommée et non
  mesurée : `startup_cleanup` est lancé en `tokio::spawn` **par agent** et fait
  `compact_old_audit_events(90)` puis `vacuum()` si des lignes ont été
  supprimées, pendant que les agents suivants s'enregistrent. **Suivi**, et sa
  précondition est la sonde S3 plus le voisinage des lignes
  `compacted old audit events` / `failed to vacuum` autour d'un `_retried`. Le
  nouvel ERROR et son `waited_ms` **sont** l'instrument qui rend ce suivi
  ouvrable avec une mesure.
- **Le réenregistrement différé (D5)**, dont la précondition est que S1/S3
  montrent une population non négligeable de `_failed`.
- **La consolidation des connexions (D3)** et le relèvement de `busy_timeout`
  (D2).
- **Le réessai des `cancel_recurring_task_by_label` (D6).**
- **Les gardes mika#1742 / mika#2271 / mika#2337 / mika#2446** : inchangées.
  Ce travail retire une population que le démarrage fabriquait, il ne touche pas
  les prédicats qui la lisaient.
- **Le balayage de démarrage de mika#2575** : inchangé. Les deux correctifs sont
  orthogonaux — celui-là traite une ligne **en vol**, celui-ci une ligne
  **absente** — et se composent dans n'importe quel ordre de merge.
