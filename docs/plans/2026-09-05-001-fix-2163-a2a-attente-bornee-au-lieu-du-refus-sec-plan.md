---
issue: 2163
repo: senara-solutions/mika
type: fix
title: "/a2a — attente bornée au lieu du refus sec sur agent occupé"
branch: fix/2163/server-le-chemin-a2a-refuse-sec-sur
status: draft
---

# mika#2163 — `/a2a` attend au lieu de refuser sec

## 1. Ce que le ticket demande, et ce que le code impose

Le ticket est exact sur le fait : `crates/mika-agent/src/server/a2a.rs:226` (`message/send`) et
`:360` (`message/stream`) prennent `agent_lock` en `try_lock_owned()` et, sur échec, rendent
`JsonRpcError::with_message(INTERNAL_ERROR, "Agent is busy")` — un `-32603` qui annonce une
défaillance serveur pour de la contention normale. `POST /message` a reçu en mika#1870 une file
bornée avec contre-pression ; `/a2a` ne l'a pas reçue. Et `/a2a` est le chemin de tout `mika ask`
(`crates/mika-cli/src/commands/ask.rs:332` → `{spirit_url}/a2a/{agent_name}`), donc de tout rappel
`canUseTool` de pilote.

**AC1 demande de réutiliser le mécanisme de mika#1870. La lecture du code montre que ce mécanisme,
pris littéralement, ne s'applique pas à ce chemin** — et ce plan documente pourquoi, parce que
c'est le seul point du ticket où le corps et le code ne se rejoignent pas.

### 1.1 Pourquoi `WebhookQueue` n'est pas transposable tel quel

`webhook_queue_v2::WebhookQueue` a trois propriétés, et chacune bute sur `/a2a` :

| Propriété de mika#1870 | Sur `POST /message` | Sur `/a2a` |
|---|---|---|
| Producteur sans canal de retour — `enqueue` rend `202 accepted`, le drain worker exécute plus tard (`handlers.rs:281-373`) | Correct : le gateway ne veut qu'un accusé | **Faux** : `message/send` est synchrone, l'appelant attend le `Task` complété (`a2a.rs:288-330`) |
| Coalescing par `coalescing_key` | Utile : un burst `check_suite` s'effondre | **Inerte** : deux `mika ask` sont deux demandes distinctes — la classe est celle de `PrReview` / `Other`, « ne jamais fusionner » |
| Saturation = `drop_oldest_and_push` | Acceptable : surface dead-letter auditée | **Faux** : on ne peut pas jeter en silence une requête dont un client tient la connexion ouverte |

Le type transporté diffère aussi (`MessageRequest` contre `MessageSendParams` + `JsonRpcId`), et le
drain worker (`handlers::spawn_webhook_drain_worker`, `handlers.rs:972-1017`) appelle
`run_agent_for_message`, pas `run_a2a_agent` + `a2a_create_task` + `a2a_build_task`.

Réutiliser le module au sens strict voudrait donc dire : le rendre générique sur son item, y ajouter
un `oneshot::Sender` de retour par entrée, y ajouter une politique de saturation « refuser le
nouvel arrivant » à côté de « jeter le plus ancien », et écrire un second drain worker. C'est-à-dire
réécrire l'essentiel d'un module éprouvé, **et faire porter le risque de régression au chemin
`POST /message`, qui est le chemin de dispatch de la boucle autonome** (tier 1 de l'ordre de
priorité) — pour un facteur commun qui se réduit, une fois les trois lignes du tableau retirées, au
mot « file ».

### 1.2 Ce que ce plan retient de mika#1870

L'intention d'AC1 — *une seule forme d'attente dans le dépôt, pas deux formes divergentes* — est
tenue sans le refactor, parce que la forme retenue **n'introduit aucune seconde structure de file**.
Elle utilise l'attente que `tokio::sync::Mutex` fournit déjà et qui est équitable (acquisition dans
l'ordre de demande), bornée par un `Semaphore` qui matérialise la profondeur. Ce qui est repris de
mika#1870, littéralement :

- la **forme de réglage** : trois clés dans le registre `mika-common/src/config.rs`, avec les
  accesseurs `effective_*` à trois paliers (absent → défaut ; invalide → défaut + `warn!` ;
  sentinelle → désactivé), exactement comme `effective_webhook_queue_max_depth` /
  `_block_timeout_ms` / `_enabled` (`config.rs:1643-1676`) ;
- le **contrat d'interrupteur** (mika#1870 AC9) : chemin désactivé = code d'aujourd'hui verbatim ;
- la **forme d'audit** : un événement par issue (attente, saturation), sur le modèle de
  `emit_webhook_queue_audit`.

### 1.3 La divergence AC1, portée et tranchée

La lecture littérale d'AC1 et ce plan divergent ; la divergence a été relevée au point de contrôle
de réconciliation (halte `ESCALATE-divergence`, liste en `/tmp/groom-divergence-mika-2163.md`) et
**tranchée par mika-prime le 2026-09-05 en faveur de R-A** — reprendre la forme, pas le code —
comme question de doctrine et non de jalon :

> « AC1 voulait empêcher **deux formes divergentes** ; R-A n'introduit **aucune seconde
> structure**. […] R-B, pour réutiliser le code, devrait généraliser la structure au point d'y
> ajouter oneshot-de-retour + politique-de-saturation-paramétrée + second drain — c'est-à-dire
> faire diverger la forme unique pour la faire tenir sur deux chemins. »

Et sur le poids qui fait pencher :

> « R-B fait entrer `POST /message` — le chemin de dispatch de la boucle autonome, tier 1 — dans le
> rayon de régression […]. Loop-health > propreté du facteur commun. »

**Le coût assumé, nommé parce qu'il est réel.** La forme de mika#1870 est ici *délibérément
ré-exprimée* et non partagée : les trois paliers de réglage, le contrat d'interrupteur et la forme
d'audit existent en deux endroits du dépôt. Un lecteur futur verra deux descriptions du même patron
de réglage et pourra vouloir les unifier. Il ne doit pas : **le contrat de contrôle diffère**
(`POST /message` est fire-and-forget, `/a2a` est synchrone), et c'est cette différence — pas
l'inattention — qui produit la duplication. Le renvoi croisé vit à trois endroits : ici, dans le
`CLAUDE.md` de `mika-agent` (étape 7) et dans les commentaires de module de l'étape 4.

## 2. Le paramètre qui dimensionne tout : le budget du client

`mika ask` appelle avec `A2aClient::DEFAULT_TIMEOUT = 300 s`
(`crates/mika-a2a/src/client.rs:21`). Ce budget couvre **l'attente ET le tour d'agent**, pas
l'attente seule.

Conséquence de conception, et c'est la contrainte dure du ticket : si l'attente serveur est trop
généreuse, on remplace un refus lisible à 0 s par un timeout client illisible à 300 s — strictement
pire que le statu quo, parce qu'un timeout ne dit ni pourquoi ni combien attendre. **Le refus à
saturation doit donc arriver franchement avant le plafond client**, et l'attente par défaut doit
rester une fraction du budget.

La preuve du ticket dimensionne le défaut : cinq refus à 20 s d'intervalle avant la réussite à la
sixième, soit ≈ 100 s d'occupation. **Défaut retenu : 120 s d'attente.** Cette valeur absorbe
l'incident fondateur en un seul appel, et laisse 180 s de budget client au tour lui-même.

Ce plan ne prétend pas supprimer le refus. Un tour d'agent long (revue de PR, passe architecte) peut
dépasser toute attente raisonnable. Ce qu'il change : la contention courte — de la seconde à la
minute, le cas courant — est absorbée en silence, et la contention longue produit un refus qui
**dit sa nature et son délai** au lieu de mentir en `-32603`.

## 3. Conception

### 3.1 Portes

Deux portes, une par point de prise, dans `crates/mika-agent/src/server/a2a.rs` :

- `handle_message_send` (`:226`) — synchrone. L'attente a lieu **dans le handler**, avant la
  création de la tâche DB.
- `handle_message_stream` (`:360`) — SSE. Le verrou est aujourd'hui pris dans le handler puis
  **déplacé dans la tâche `tokio::spawn`** (`a2a.rs:396`, `let _lock_guard = lock_guard;`).

Les deux portes diffèrent, et la différence est délibérée :

| | `message/send` | `message/stream` |
|---|---|---|
| Prise de la place en file (`Semaphore`) | dans le handler, avant tout travail | dans le handler, avant le `spawn` |
| Attente du verrou | dans le handler | **dans la tâche spawned** |
| Réponse à saturation | erreur JSON-RPC | erreur JSON-RPC (le flux n'est pas encore ouvert) |

Pour le streaming, attendre dans le handler retiendrait l'ouverture du flux SSE : le client ne
verrait rien jusqu'à l'acquisition. Attendre dans la tâche spawned laisse le flux s'ouvrir tout de
suite ; l'attente devient visible plutôt que muette. La place en file, elle, doit être prise
**avant** le `spawn`, sinon on spawne sans borne et la contre-pression est fictive.

### 3.2 La borne

Un `Semaphore` par agent, dans `AgentState` (`crates/mika-agent/src/server/state.rs:38`, à côté de
`agent_lock`), de `a2a_queue_max_depth` permis. `try_acquire_owned()` :

- succès → le demandeur a une place, il attend le verrou jusqu'à `a2a_queue_wait_timeout_ms` ;
- échec → file pleine, refus immédiat avec le code de contention (§3.3).

**L'attente est équitable, et c'est vérifié, pas supposé.** `tokio::sync::Mutex` documente
« a simple FIFO (first in, first out) style where all calls to `lock` complete in the order they
were performed » (`tokio-1.53.1/src/sync/mutex.rs:112-115`), et `lock_owned` précise « uses a queue
to fairly distribute locks in the order they were requested » (`:598`). Le dépôt épingle
`tokio = "1"` (`Cargo.toml:18`), résolu en 1.53.1. Un attendant ne peut donc pas être doublé
indéfiniment : la borne de §3.2 borne bien une file, pas une mêlée.

La même source porte une seconde phrase qui compte : « Cancelling a call to `lock_owned` makes you
lose your place in the queue » (`:599-600`). Elle a une conséquence **asymétrique** entre les deux
portes, traitée en §3.6.

La profondeur borne le nombre d'appelants **en attente**, pas le nombre de tours. Le permis est
détenu pendant l'attente et libéré à l'acquisition du verrou, pas à la fin du tour : autrement une
attente et un tour compteraient dans la même borne et la profondeur ne voudrait plus rien dire.

Le dépassement du délai d'attente rend le même refus que la saturation (§3.3), avec une `reason`
distincte dans `data` — l'appelant doit pouvoir distinguer « la file était pleine » de « j'ai
attendu mon tour et il n'est pas venu ».

**Défaut retenu : profondeur 8.** Au-delà, l'attente cumulée dépasse le budget de tous les
attendants ; refuser vite est plus honnête que faire patienter un neuvième appelant derrière huit
tours d'agent.

### 3.3 Le code d'erreur, et le délai à réessayer

`INTERNAL_ERROR` (-32603) part. À sa place, un code A2A dédié dans
`crates/mika-a2a/src/jsonrpc.rs`, à côté de `TASK_NOT_FOUND` (-32001) … `INVALID_AGENT_RESPONSE`
(-32006).

**Le code doit éviter la plage assignée par la spec A2A.** `-32007` est pris par la spec pour
`AuthenticatedExtendedCardNotConfiguredError` ; le prendre pour « agent occupé » créerait une
collision protocolaire avec un pair A2A tiers. **Retenu : `AGENT_BUSY = -32010`** — dans la plage
serveur réservée par JSON-RPC 2.0 (-32000 à -32099), hors de la plage assignée A2A aujourd'hui.
`from_code` reçoit son libellé, `"Agent is busy"`.

L'objet `data` porte ce que le commentaire opérateur du 2026-09-04 00:38 CEST demande — un appelant
ne doit pas deviner combien attendre :

```json
{ "code": -32010,
  "message": "Agent is busy",
  "data": { "reason": "queue_full" | "wait_timeout",
            "retry_after_ms": 120000,
            "queue_depth": 8 } }
```

`retry_after_ms` est le délai d'attente configuré — une borne honnête et connue du serveur, pas une
prédiction de la durée du tour en cours, que le serveur ne connaît pas. Ce plan ne prétend pas
mieux : annoncer un délai faux serait pire que ne rien annoncer.

### 3.4 L'interrupteur

Trois clés dans `ConfigKeyInfo` (`crates/mika-common/src/config.rs`, à la suite du bloc
mika#1870), trois champs `Option<…>` dans `Settings`, trois accesseurs `effective_*` :

| Clé | Env | Défaut | Invalide |
|---|---|---|---|
| `a2a_queue_max_depth` | `MIKA_A2A_QUEUE_MAX_DEPTH` | 8 | `0` → défaut + `warn!` |
| `a2a_queue_wait_timeout_ms` | `MIKA_A2A_QUEUE_WAIT_TIMEOUT_MS` | 120000 | `0` → honoré (ne pas attendre) |
| `a2a_queue_enabled` | `MIKA_A2A_QUEUE_ENABLED` | `true` | — |

`0` sur le délai est honoré comme « ne pas attendre » — le miroir exact de
`effective_webhook_queue_block_timeout_ms` (`config.rs:1663`), et une seconde façon d'obtenir le
comportement d'aujourd'hui sans toucher à l'interrupteur.

`a2a_queue_enabled = false` rend le `try_lock_owned()` → `-32603 "Agent is busy"` **verbatim**, y
compris le code -32603, parce qu'un retour en arrière qui changerait aussi le code d'erreur ne
serait pas un retour en arrière. Les trois clés sont ajoutées au `match` de résolution
(`config.rs:638-644`) pour rester visibles à `mika config`.

### 3.6 L'abandon de l'appelant, et l'asymétrie entre les deux portes

`message/send` attend dans le handler. Si le client se déconnecte, axum abandonne le future du
handler : l'attente sur `lock_owned` est annulée, la place en file est perdue (comportement voulu),
et le permis `Semaphore` est rendu par `Drop`. Rien à faire.

`message/stream` attend dans une tâche `tokio::spawn` (§3.1), et **une tâche spawned n'est pas
annulée par la déconnexion du client**. Un appelant qui coupe pendant l'attente laisse donc la
tâche acquérir le verrou et exécuter un tour d'agent complet dont plus personne ne lit le flux —
un tour payé pour rien, et une place tenue devant les attendants suivants.

Le remède est déjà à portée : le `BroadcasterGuard` (`a2a.rs:31-39`) est retiré de la `DashMap` au
`Drop`, mais rien n'observe la disparition de l'abonné. **Avant d'attendre le verrou, la tâche
vérifie `tx.receiver_count()`** ; à zéro, elle abandonne, rend son permis et n'exécute pas le tour.
Le contrôle est refait une fois après l'acquisition, avant de lancer l'agent — la fenêtre entre les
deux est bornée par la durée d'un `lock_owned`.

Ce comportement est **nouveau** : aujourd'hui la question ne se pose pas, parce qu'aucune attente
n'existe. Il est donc à couvrir par un test (T8) et non à supposer.

### 3.5 Ce qui ne change pas

`agent_lock` reste ce qu'il est : la sérialisation des tours d'un agent est délibérée, le ticket le
dit en hors-périmètre. Ce plan change ce qui arrive au demandeur pendant qu'il attend.

Les trois `try_lock_owned()` de `crates/mika-agent/src/server/rewind.rs` (`:134`, `:187`, `:273`)
restent inchangés : le ticket nomme deux points de prise, et le rewind est une opération
d'administration où un refus immédiat est le bon comportement. Nommé ici pour que l'omission soit
lue comme un choix.

## 4. Étapes

1. **`crates/mika-a2a/src/jsonrpc.rs`** — `pub const AGENT_BUSY: i32 = -32010;` + le bras dans
   `from_code`. Test : le code n'entre en collision avec aucune constante existante du module.
2. **`crates/mika-common/src/config.rs`** — trois `ConfigKeyInfo`, trois champs `Settings`, trois
   `effective_*`, trois bras dans le `match` de résolution, trois constantes `DEFAULT_*`.
3. **`crates/mika-agent/src/server/state.rs`** — `pub a2a_wait_slots: Arc<tokio::sync::Semaphore>`
   sur `AgentState`, construit dans `server/mod.rs:508` près de `agent_lock`, dimensionné par
   `effective_a2a_queue_max_depth()`.
4. **`crates/mika-agent/src/server/a2a.rs`** — une fonction `acquire_agent_lock_or_busy(...)`
   partagée par les deux portes, rendant `Result<OwnedMutexGuard<()>, JsonRpcError>` ; les deux
   sites l'appellent, avec pour `message/stream` l'acquisition du permis avant le `spawn` et
   l'attente du verrou dedans (§3.1). Le chemin désactivé garde le `try_lock_owned()` d'aujourd'hui.
5. **Audit** — un événement `a2a_queue_wait` (à l'entrée en attente, avec `wait_ms` à la sortie) et
   `a2a_queue_reject` (saturation ou dépassement), sur la forme de `emit_webhook_queue_audit`, avec
   la même limitation de débit qu'elle pour qu'un flux de refus n'inonde pas la table d'audit.
6. **`crates/mika-agent/docs/openapi/mika-spirit.yaml`** — la description `'429'` (ligne 132-133) est
   celle de `POST /message` ; vérifier si le fichier décrit `/a2a` et, le cas échéant, y consigner le
   nouveau code. Aucune promesse ici avant lecture.
7. **`crates/mika-agent/CLAUDE.md`** — la note `server::webhook_queue_v2` (ligne 225) dit que ce
   mécanisme « remplace le motif hérité `try_lock_owned()` → 429 » de `POST /message` ; ajouter la
   phrase qui dit que `/a2a` a sa propre forme d'attente et pourquoi elle diffère, pour qu'un
   lecteur futur ne relance pas l'unification sans relire le §1.1.

## 5. Tests

| # | AC | Test | Où |
|---|---|---|---|
| T1 | AC4 | Deux appels A2A concurrents sur le même agent : le premier tient le verrou, le second **attend et aboutit** au lieu de recevoir `"Agent is busy"` | `crates/mika-agent/tests/a2a_queue_contention.rs` (nouveau) |
| T2 | AC5 | `a2a_queue_enabled = Some(false)` + verrou tenu → `-32603 "Agent is busy"` immédiat, **code et message identiques à aujourd'hui** | idem |
| T3 | AC2 | File pleine (profondeur 1, N+1 attendants) → `-32010`, `data.reason = "queue_full"`, `retry_after_ms` présent | idem |
| T4 | AC2 | Délai dépassé (`wait_timeout_ms` court, verrou jamais rendu) → `-32010`, `data.reason = "wait_timeout"` | idem |
| T5 | AC3 | Les trois accesseurs `effective_a2a_*` : absent → défaut ; `0` sur la profondeur → défaut + `warn!` ; `0` sur le délai → honoré | `config.rs` tests, sur le modèle de `config.rs:2970-2978` |
| T6 | AC1 | Les deux portes passent par le même chemin : un test par porte, `message/send` **et** `message/stream`, sur T1 | `a2a_queue_contention.rs` |
| T7 | — | `AGENT_BUSY` n'entre en collision avec aucun code A2A du module | `crates/mika-a2a/src/jsonrpc.rs` tests |
| T8 | — | `message/stream` : abonné SSE disparu pendant l'attente → la tâche abandonne, rend son permis, **n'exécute pas** le tour (§3.6) | `a2a_queue_contention.rs` |

T1 est le test qui porte le ticket : il doit être une **vraie concurrence** (deux tâches tokio
interleavées), pas deux appels séquentiels — un test séquentiel montrerait qu'un second appel après
un premier réussit, ce qui est vrai même aujourd'hui. La forme à suivre est celle de
`crates/mika-agent/tests/dispatcher_contention.rs`, qui documente précisément ce piège.

T2 est la non-régression du ticket (AC5) : il doit échouer si quelqu'un « améliore » le chemin
désactivé.

## 6. Tie-back aux AC

| AC | Où c'est tenu | Réserve |
|---|---|---|
| AC1 — attente bornée aux deux points de prise, en réutilisant mika#1870 | §3.1, §3.2, étape 4 ; T1, T6 | La réutilisation est celle de la **forme**, pas du code — §1.1, §1.2. Divergence relevée en réconciliation et **tranchée R-A par mika-prime le 2026-09-05** (§1.3). |
| AC2 — borne et saturation explicites, code qui dit la contention | §3.2, §3.3 ; T3, T4 | `-32010` et non `-32007` (collision spec A2A) |
| AC3 — interrupteur trois paliers, chemin désactivé = comportement d'aujourd'hui | §3.4 ; T5, T2 | — |
| AC4 — test de deux appels concurrents, le second aboutit | T1, T6 | — |
| AC5 — non-régression file désactivée | §3.4 ; T2 | Le code -32603 est conservé sur ce chemin, délibérément |
| AC6 (issu du commentaire du 2026-09-04) — le refus dit combien attendre | §3.3 ; T3 | `retry_after_ms` = le délai configuré, pas une prédiction du tour |

## 7. Hors périmètre

- Le plafond de concurrence de la classe `implement` (mika#2160). Ce ticket répare le chemin de
  rappel, pas le nombre de pilotes. mika#2160 peut livrer son réglage à défaut 1 sans lui ; c'est un
  prérequis d'exploitation de N>1, pas un prérequis de livraison.
- `agent_lock` lui-même (§3.5).
- Les `try_lock_owned()` de `rewind.rs` (§3.5).
- Une file **persistante** qui survivrait au redémarrage du serveur. La borne est en mémoire ; un
  redémarrage perd les attendants, qui reçoivent une erreur de transport. C'est le comportement
  d'aujourd'hui et ce ticket ne le change pas.
- Un retry côté client dans `mika ask`. Le ticket vise le serveur ; `retry_after_ms` (§3.3) donne à
  un futur retry client de quoi se régler, sans l'écrire ici.

## 8. Risques

- **Le pire cas est un timeout client, pas un refus.** Si l'attente serveur et le tour d'agent
  additionnés dépassent 300 s, le client coupe et l'appelant voit une erreur de transport — moins
  lisible que le `-32603` d'aujourd'hui. §2 dimensionne le défaut contre ce risque (120 s + 180 s
  de marge) ; T4 en fixe la borne. Si l'architecte juge la marge trop mince, le défaut baisse, la
  conception ne change pas.
- **La borne est par agent, pas globale.** Huit attendants sur chacun de N agents, c'est 8N tâches
  parkées. Chacune est une tâche tokio en attente sur un `Notify`, pas un fil ; le coût est
  négligeable et la borne existe. Nommé pour que ce ne soit pas une découverte.
- **Le permis libéré à l'acquisition du verrou** (§3.2) est le point subtil : le libérer à la fin du
  tour ferait compter attente et exécution dans la même borne. À vérifier explicitement en revue.
- **Le tour orphelin du chemin streaming** (§3.6) est le risque que ce plan crée lui-même : il
  n'existe pas aujourd'hui, parce qu'aujourd'hui personne n'attend. Le contrôle
  `receiver_count()` et T8 sont la contrepartie ; si la revue les juge insuffisants, c'est ce point
  qu'il faut renforcer, pas la conception d'ensemble.
- **La duplication de forme avec mika#1870** (§1.3) est assumée et triplement renvoyée. Le risque
  n'est pas technique, il est de lecture : qu'un futur passage l'unifie sans relire le §1.1.
