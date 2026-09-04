# Plan : la livraison d'un callback ne dit rien, et ne s'arrête jamais (mika#2179)

**Ticket :** mika issue#2179 — `fix(task_engine,llm): livraison des callbacks mika-dev affamée — 38 timeouts de transport LLM en 3 h, callback livré 5 h 06 après complétion, parent mort d'âge avant son retour`
**Labels :** `bug`, `p1-important`
**Type :** issue (bug — casseur de boucle : le retour du pilote existe en base et n'atteint pas l'agent)
**Palier de priorité :** Tier 1 — *casse la boucle*. Un parent légitime meurt d'âge (grâce 14 400 s) pendant que son retour attend son tour de livraison.
**Fichiers principaux :** `crates/mika-agent/src/task_engine/dispatcher.rs`, `crates/mika-agent/src/db.rs`, `crates/mika-agent/src/async_db.rs`, `crates/mika-common/src/config.rs`, `crates/mika-agent/tests/eval/test_callback_delivery_starvation.rs` (nouveau)

---

## Problème

`dispatch_resume_agent` a deux sorties, et une seule est écrite.

```rust
// crates/mika-agent/src/task_engine/dispatcher.rs:495-497
if let Err(e) = run_silent_agent(&params).await {
    warn!(task_id = %task.id, error = %e, "resume_agent run failed");
```

Sur cette branche : **aucun `mark_task_delivered`, aucun `next_fire_at`, aucun compteur,
aucun événement d'audit.** La tâche reste `status='completed'`, donc
`get_undelivered_callback_tasks` la resélectionne au balayage suivant
(`engine.rs:556`, cadence `DB_SCAN_INTERVAL_TICKS = 60` → **60 s**), et le cycle
recommence. Sans borne : la seule condition d'arrêt du code actuel est que l'appel LLM
finisse par réussir.

Chaque tentative n'est pas gratuite. Elle prend le verrou d'agent
(`dispatcher.rs:390-400`) et le garde jusqu'à
`AGENT_TOTAL_TIMEOUT_SECS = 300` (`planning/policy.rs:18`, appliqué en
`agent_loop/mod.rs:3985`). **C'est là qu'est la fente de 5 minutes du ticket** — pas dans
le balayage. Une reprise qui échoue sur transport consomme donc, en boucle, la ressource
la plus rare de mika-dev, et n'en laisse aucune trace interrogeable.

Le résultat mesuré, en base et sans le journal : `800d739f` complété à
`2026-09-03T22:03:24Z`, livré à `2026-09-04T03:09:16Z`. **5 h 06.** Le parent
`620ae345` (`ready-label: mika#2140`) est mort `phantom_aged_out` à `02:04:01Z` —
**une heure et cinq minutes avant** que le retour de son propre pilote soit lu.

## Mesures — exécutées le 2026-09-04, `~/.mika/data/mika.db` en lecture seule

### M1 — les trois lignes du ticket, vérifiées

```sql
SELECT id,label,status,completed_at,updated_at FROM tasks
WHERE id LIKE '800d739f%' OR id LIKE 'f0cd5967%' OR id LIKE '620ae345%';
```
```
800d739f…|long_running:run_claude_pilot         |delivered|2026-09-03T22:03:24Z|2026-09-04T03:09:16Z
f0cd5967…|long_running:run_claude_pilot:deferred|delivered|2026-09-04T00:48:11Z|2026-09-04T03:35:31Z
620ae345…|ready-label: senara-solutions/mika#2140|failed  |2026-09-04T02:04:01Z|2026-09-04T03:59:50Z
```

Les attentes de 5 h 06 et 2 h 47 sont exactes. Les citations de code du ticket le sont
aussi : `openai.rs:277` construit bien le `LlmError::Transport("failed to read response
body: …")`, `error.rs:17` porte bien la variante, `dispatcher.rs:496` porte bien le
`warn!`.

### M2 — la starvation n'est pas un incident, c'est le régime normal

Distribution de `updated_at − completed_at` sur les callbacks livrés depuis le
2026-08-01 (n = 1628) :

| tranche | n | part |
|---|---|---|
| ≤ 60 s | 293 | 18 % |
| 60 s – 5 min | 443 | 27 % |
| 5 min – 15 min | 343 | 21 % |
| 15 min – 1 h | 252 | 15 % |
| > 1 h | 297 | **18 %** |

`p50 = 377 s` (6 min 17), `p75 = 2138 s` (35 min), `p90 = 9585 s` (2 h 40),
`p99 = 112 949 s` (31 h), max = 115 917 s (32 h 12).

Sur les seules 24 h du 2026-09-03/04 (n = 161) : `p50 = 327 s`, `p90 = 10 216 s`
(2 h 50), max = 18 352 s — **et ce max, c'est `800d739f`**. Le cas du ticket n'est pas
une aberration : c'est la queue d'une distribution quotidienne.

**Ce que M2 impose au seuil de l'AC2.** La proposition de 900 s classerait
`549 / 1628 = 33,7 %` des livraisons en alerte. Un seuil qui crie sur un tiers de la
population se fait désarmer. Le plan retient donc : la **mesure** est inconditionnelle
(chaque livraison écrit son `wait_secs`), et le **seuil d'alerte** est nommé,
configurable, et calé au-dessus du p90 observé — `3600 s` par défaut. L'AC2 exige « une
valeur nommée », pas la valeur 900 ; sa proposition est explicitement étiquetée
« proposition » dans le corps du ticket. L'intention est portée intacte ; le chiffre est
choisi contre une mesure au lieu d'être choisi contre un souvenir.

**Limite honnête de M2 :** `updated_at` est un *proxy*. `mark_task_delivered` l'écrit au
moment de la livraison, mais toute écriture ultérieure sur la ligne le repousse. Les
chiffres ci-dessus sont donc une **borne supérieure**. C'est précisément pourquoi
l'événement explicite de l'AC2 n'est pas un confort — voir la contrainte de séquencement
en §Conception.

### M3 — le journal compte double

```
$ grep -c '"message":"resume_agent run failed"' /var/log/mika/server.log
1288
```

Les deux premières lignes de ce grep :
```
2026-04-21T14:43:00.010012Z | dispatcher | bdc2be17-406 | LLM response parse error: …
2026-04-21T14:43:00.010038Z | dispatcher | bdc2be17-406 | LLM response parse error: …
```

**26 µs d'écart, même cible, mêmes champs.** Chaque `resume_agent run failed` est écrit
deux fois (deux couches d'abonné). Le décompte par heure du 2026-09-03 23 h au
2026-09-04 03 h donne 38 **lignes**, soit **19 événements distincts** — et c'est
cohérent avec le tableau par callback du corps du ticket, qui somme à 16 et non à 38.

Rectification portée au corps du ticket (voir §Rectifications). Elle ne change aucune
AC : que la boucle stérile ait tourné 19 fois ou 38, le mécanisme et le correctif sont
identiques. Elle change ce qu'on écrit dans la PR, et elle vaut pour tout futur comptage
lu dans ce journal : **diviser par deux**.

### M4 — le chemin d'échec fabrique aussi des lignes

Pour un wrapper différé (`label == DEFERRED_DISPATCH_LABEL`), la branche d'erreur
appelle `rearm_consumed_deferred_wrapper` (`dispatcher.rs:507`), qui via
`skills/executor.rs:2490` **crée une nouvelle ligne `pending`**. L'ancienne, elle, reste
`completed` et non livrée : elle sera redispatchée au balayage suivant. Sur une tempête
de transport, un wrapper différé se **remplace et se réessaie** simultanément.

Ce n'est pas non borné — `MAX_STUCK_REARMS` et `MAX_PENDING_DEFERRED_CALLBACKS` plafonnent
le re-armement. Mais le plafond porte sur le re-armement, pas sur la reprise, et c'est la
reprise que l'AC3 vise. L'ordre des gestes en §Conception s'en déduit.

## Rectifications apportées au corps du ticket

Deux imprécisions du corps ont été corrigées **dans le ticket** avant grooming, avec la
méthode et la mesure. Elles sont signalées ici, pas appliquées en silence.

1. **« relancée par le balayage de 5 minutes »** → le balayage est à **60 s**
   (`DB_SCAN_INTERVAL_TICKS = 60`). Les 5 minutes sont la durée pendant laquelle chaque
   tentative **retient le verrou** (`AGENT_TOTAL_TIMEOUT_SECS = 300`). L'AC3 reste juste
   telle qu'écrite sous la lecture correcte : « la fente de 5 minutes » est la fente
   d'occupation du verrou, et c'est bien elle qu'il faut cesser de reprendre.
2. **« 38 échecs »** → 38 **lignes de journal**, 19 **événements** (M3).

Aucune AC, aucune portée, aucune séquence n'est modifiée par ces deux rectifications.

## Conception

Trois phases. **A avant B, et l'ordre est porteur** : la phase B écrit `next_fire_at` et
des métadonnées sur la ligne, donc elle **pousse `updated_at`** — et détruit le seul
proxy de latence dont dispose aujourd'hui l'enquête rétrospective (M2). Livrer B sans A,
c'est aveugler la mesure au moment même où on borne la panne. A est le prérequis de B, et
c'est la seule contrainte de séquencement dure de ce plan.

### Phase A — la panne parle (AC1, AC2)

**A1. Classer l'erreur.** Fonction pure, testée à l'unité, dans `dispatcher.rs` :

```rust
fn classify_delivery_error(err: &anyhow::Error) -> &'static str
```

Elle fait `err.downcast_ref::<LlmError>()` — `anyhow` parcourt toute la chaîne de causes,
donc un `.context()` intermédiaire ne la casse pas — et rend :
`transport_timeout` (variante `Transport` dont le texte contient `timed out`),
`transport`, `http_<status>`, `parse`, `provider`, `unsupported`, `other` (pas un
`LlmError`). **Pas de comparaison de chaînes sur le message d'erreur** : la variante est
la donnée, le texte est de l'affichage.

**A2. Écrire l'échec.** Sur la branche `Err` de `run_silent_agent`, pour
`is_callback` uniquement :
- incrémenter `metadata.$.delivery_attempts` (via `set_task_metadata_field`,
  `db.rs:7823`, qui fait un `json_set` — pas de migration de schéma) ;
- poser `metadata.$.delivery_first_failed_at` au premier échec, et
  `metadata.$.delivery_last_error_class` à chaque fois ;
- écrire l'événement d'audit **`callback_delivery_failed`** via `log_audit_event`
  (`async_db.rs:2113`), sur le modèle exact de `deferred_dispatch_noop_completion`
  (`dispatcher.rs:578-592`) :
  `target_key = "task:<id>"`, `after_value = <classe>`,
  `reasoning = "attempt:<n> label:<label> err:<texte tronqué à 300>"`,
  `trace_id = Some(&trace_id)`.

**A3. Écrire la réussite.** Sur la branche `else if is_callback`, après un
`mark_task_delivered` qui rend `true` :
- `wait_secs = now − task.completed_at` (les deux sont des `TEXT` ISO-8601 Z ;
  `crate::timestamp` porte déjà les aides) ;
- événement d'audit **`callback_delivered`**, `after_value = "<wait_secs>"`,
  `reasoning = "wait_secs=<n> attempts=<n> label=<label>"` ;
- `warn!` structuré si `wait_secs > settings.effective_callback_delivery_slow_threshold_secs()`.

`completed_at` peut être `NULL` (callback `failed` non complété) : dans ce cas, pas de
`wait_secs`, l'événement est écrit avec `after_value = "unknown"`. Aucun `unwrap`.

**A4. Le réglage.** Dans `mika-common/src/config.rs`, sur le modèle exact de
`phantom_sweep_age_seconds` (`config.rs:959` / `1466`) :

| champ `Settings` | défaut | env (automatique via `Environment::with_prefix("MIKA")`) |
|---|---|---|
| `callback_delivery_slow_threshold_secs: Option<u64>` | `3600` | `MIKA_CALLBACK_DELIVERY_SLOW_THRESHOLD_SECS` |
| `callback_delivery_max_attempts: Option<u32>` | `3` | `MIKA_CALLBACK_DELIVERY_MAX_ATTEMPTS` |
| `callback_delivery_backoff_base_secs: Option<u64>` | `60` | `MIKA_CALLBACK_DELIVERY_BACKOFF_BASE_SECS` |
| `callback_delivery_backoff_max_secs: Option<u64>` | `3600` | `MIKA_CALLBACK_DELIVERY_BACKOFF_MAX_SECS` |

Un `DEFAULT_*` nommé et un accesseur `effective_*()` par champ, avec le rationnel mesuré
en doc-comment (le seuil cite M2). **Réglage par `Settings`, pas par lecture directe
d'`std::env`** : les variables d'environnement sont globales au processus et les tests de
`mika-agent` partagent un binaire — c'est la raison que le plan mika#2156 a déjà écrite
pour `phantom_sweep_age_seconds`, et elle vaut ici à l'identique. `TaskDispatcher` porte
déjà `settings`.

### Phase B — la panne se borne (AC3)

**B1. Reculer.** Après A2, poser
`next_fire_at = now + min(base × 2^(attempts−1), max)` via `update_task_next_fire_at`
(`db.rs:6217`). Avec les défauts : 60 s, 120 s, 240 s, … plafonné à 3600 s.

**Aucun code de balayage n'est touché.** La garde existe déjà et n'attendait qu'une
valeur :

```rust
// engine.rs:571-577
if let Some(ref fire_at) = task.next_fire_at
    && fire_at.as_str() > now.as_str()
{ continue; }
```

C'est le levier posé par mika#1070 pour la reprise après `AgentBusy` ; il est ici réutilisé
tel quel. Pas de nouvelle colonne, pas de migration, pas de second chemin de décision.

**B2. Mettre à l'écart, visiblement.** Quand `attempts >= max_attempts` : continuer à
reculer au plafond **et** écrire l'événement **`callback_delivery_quarantined`**
(`after_value = <classe>`, `reasoning = "attempts=<n> backoff_secs=<n>"`), une fois par
franchissement de seuil — pas à chaque tentative, sinon l'événement devient du bruit.

**Ce que la mise à l'écart ne fait pas :** elle ne marque **jamais** la tâche `delivered`
ni `failed`. Le `result` du callback est le retour du pilote ; le perdre, c'est perdre le
travail. La sortie de l'AC3 est « recul exponentiel **ou** mise à l'écart visible » —
le plan livre les deux, et aucune des deux n'est destructrice.

**B3. Ordre avec le re-armement (M4).** L'enregistrement de l'échec (A2) et le recul (B1)
s'exécutent **avant** `rearm_consumed_deferred_wrapper`. Le re-armement est conservé tel
quel — il a son propre budget (`MAX_STUCK_REARMS`) et le supprimer risquerait d'abandonner
un parent réparable. Ce que la phase B garantit, c'est que la ligne qui a échoué cesse de
reprendre la fente à chaque minute ; le nombre de lignes que le re-armement fabrique est
un axe distinct, mesurable une fois les événements de l'AC1 en place. **Point ouvert
assumé, pas oublié :** si les événements montrent que re-armement et reprise se
composent en croissance, c'est un ticket suivant, pas un élargissement de celui-ci.

### Phase C — les tests (AC4, AC5)

**C1. Aide de test manquante.** `Database::backdate_task_completed_at(task_id, seconds_ago)`,
`#[doc(hidden)]`, frère exact de `backdate_task_updated_at` (`db.rs:7329`), plus son
enveloppe dans `async_db.rs`. `update_task_completed` écrit `completed_at = now` ; il faut
pouvoir semer 22:03:24Z.

**C2. Le rejeu anti-vacuité.** Nouveau fichier
`crates/mika-agent/tests/eval/test_callback_delivery_starvation.rs`, calqué sur
`test_phantom_task_row_sweep.rs` (DB en mémoire, `TaskDispatcher` construit à la main,
assertions sur un delta de comptage dans `audit_events`).

Semis : une tâche `trigger_type='callback'`, `action_type='resume_agent'`,
`label='long_running:run_claude_pilot'`, `status='completed'`, `result` non vide,
`completed_at` reculé de 5 h 06 — et **l'id forcé à `800d739f-a0ed-485d-bef1-9990beeac396`**
via l'aide de réécriture de clé primaire déjà présente (`db.rs`, voisine de
`backdate_task_updated_at`), pour que le test porte l'identifiant que le ticket nomme.

Transport : `MockLlmProvider::builder().error(LlmError::Transport("failed to read response
body: … operation timed out".into()))` (`mika-common/src/llm/mock.rs:213`), `cli_mode:
false`, `agent_lock: None`.

Trois appels successifs à `dispatch_resume_agent`, puis :

| assertion | sur `main` | avec le correctif |
|---|---|---|
| `count(audit_events WHERE tool_name='callback_delivery_failed')` | **0** | 3 |
| `after_value` du 1ᵉʳ événement | *(absent)* | `transport_timeout` |
| `next_fire_at` non nul et strictement croissant | **NULL** | 60 s → 120 s → 240 s |
| `count(… 'callback_delivery_quarantined')` | **0** | 1 |
| `status` de la tâche | `completed` | `completed` (jamais détruit) |

**Recette d'injection (obligatoire, en en-tête de module)** — sur le modèle littéral de
`test_phantom_task_row_sweep.rs` : commenter le `log_audit_event` de A2, relancer, les
assertions de comptage **doivent** rougir ; restaurer, relancer, elles passent. Le test
prouve alors que le chemin d'écriture est porteur.

**Sortie rouge collée dans la PR** (AC4), obtenue en exécutant le nouveau fichier contre
`main` avant d'appliquer A et B.

**C3. Non-régression (AC5) — tests existants nommés, non réécrits :**
- `crates/mika-agent/src/task_engine/engine.rs::test_cli_mode_skips_callback_dispatch`
- `…::test_complete_parent_tasks_on_callback_success_happy_path`
- `…::test_complete_parent_tasks_on_callback_success_idempotent_race_with_inline`
- `crates/mika-agent/src/task_engine/dispatcher.rs::test_dispatch_resume_agent_callback_no_result_returns_error`
- `…::test_dispatch_resume_agent_reminder_reads_action_config`
- `crates/mika-agent/tests/eval/test_callback_turn.rs` (toute la suite)
- `crates/mika-agent/tests/eval/test_callback_terminal_action.rs` (toute la suite)
- `crates/mika-agent/tests/eval/test_deferred_dispatch_idempotent_ack.rs`

Plus **un cas positif neuf** dans C2 : une livraison qui réussit du premier coup écrit
`callback_delivered`, ne pose **aucun** `next_fire_at`, et marque la tâche `delivered` —
c'est le contrôle négatif sans lequel les assertions rouges ne prouvent rien.

## Tie-back aux critères d'acceptation

| AC | Livré par | Observable |
|---|---|---|
| AC1 — événement nommé par échec, avec `task_id`, classe, rang | A1 + A2 | `audit_events.tool_name='callback_delivery_failed'`, `after_value` = classe, `reasoning` porte `attempt:<n>` |
| AC2 — latence mesurable hors journal, seuil nommé | A3 + A4 | `audit_events.tool_name='callback_delivered'`, `reasoning` porte `wait_secs=`; seuil `effective_callback_delivery_slow_threshold_secs()`, défaut 3600 s **justifié par M2** |
| AC3 — N échecs consécutifs ⇒ la fente n'est plus reprise, sortie non muette | B1 + B2 | `next_fire_at` croissant ; `callback_delivery_quarantined` au franchissement |
| AC4 — rejeu rouge sur `main`, vert avec le correctif, sortie collée | C2 | tableau d'assertions ci-dessus + recette d'injection |
| AC5 — la livraison qui réussit ne change pas | C3 | huit suites nommées + cas positif neuf |

## Hors portée (repris du ticket, sans extension)

- La **cause** des timeouts côté fournisseur (openrouter / `z-ai/glm-5.3`, taille des
  requêtes, proxy d'egress). Ce plan rend la panne visible et bornée ; il ne la fait pas
  disparaître.
- L'identification des sessions « autres » (`trigger: ''`).
- mika#2169 (re-armement vers un parent terminal).
- **Ajout du plan :** le nombre de lignes fabriquées par le re-armement sur tempête
  (M4) — mesurable une fois l'AC1 en place, ticket suivant si la mesure le demande.
- **Ajout du plan :** la double écriture du journal (M3) est *constatée*, pas corrigée
  ici. Elle n'affecte pas la boucle ; elle fausse les comptages lus au grep.

## Voisinage

`mika#2121` (ouvert) compte les **dispatches qui ne produisent pas de PR** — le callback
arrive et ne porte rien. Ce ticket-ci compte les **callbacks qui n'arrivent pas**. Les
deux touchent `dispatcher.rs` sans se recouvrir : #2121 vit sur le chemin de dispatch et
le motif `callback_delivered_without_pr_url`, #2179 sur la branche `Err` de
`run_silent_agent` et la boucle de reprise. Si les deux sont en vol simultanément,
attendre un conflit textuel de voisinage dans `dispatch_resume_agent`, pas un conflit de
conception.

## Conditions d'arrêt

- **S'arrêter si la phase B est écrite avant la phase A.** B pousse `updated_at` et
  détruit le proxy de latence ; A doit poser la mesure explicite d'abord (M2).
- **S'arrêter si la mise à l'écart marque la tâche `delivered` ou `failed`.** Le `result`
  est le retour du pilote ; le perdre coûte plus cher que la fente reprise.
- **S'arrêter si la classification des erreurs se fait par comparaison de chaînes** au
  lieu de `downcast_ref::<LlmError>()`. Le texte est de l'affichage, la variante est la
  donnée.
- **S'arrêter si un seuil est lu directement dans `std::env`** au lieu de `Settings` :
  les tests de ce crate partagent un binaire.
- **S'arrêter si le seuil d'alerte est laissé à 900 s sans reprendre M2.** Un seuil qui
  classe 33,7 % de la population en alerte se fait désarmer, et l'AC2 demande une valeur
  nommée — pas cette valeur-là.
- **S'arrêter si l'AC4 est déclarée verte sans la sortie rouge obtenue contre `main`.**
  Un test écrit après le correctif ne falsifie rien.
