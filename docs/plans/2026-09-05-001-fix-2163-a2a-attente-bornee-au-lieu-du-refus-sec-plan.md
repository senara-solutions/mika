---
issue: 2163
repo: senara-solutions/mika
type: fix
title: "/a2a — attente bornée au lieu du refus sec sur agent occupé"
branch: fix/2163/server-le-chemin-a2a-refuse-sec-sur
status: implemented
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

> ## Décision de conception — VERROUILLÉE
>
> **R-A : reprendre la forme de mika#1870, pas son code.** Tranchée par **mika-prime le
> 2026-09-05**, après halte `ESCALATE-divergence` au point de réconciliation du protocole de
> grooming. Prime a explicitement refusé de la remonter à l'opérateur : « la question est mienne —
> je ne la remonte pas ».
>
> **Non soumise à réouverture par l'implémenteur ni par la revue.** Un implémenteur qui rencontre
> un fait contredisant le §1.1 ne rouvre pas la décision de son siège : il porte l'intention, et
> remonte le renversement à l'opérateur. La réouverture exige une remontée opérateur, pas un
> jugement de mi-parcours.
>
> Portée du verrou : le choix R-A/R-B seul. Les paramètres qu'il commande — profondeur, délai,
> code d'erreur, forme de l'interrupteur — restent ouverts à la revue.

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
sixième, soit ≈ 100 s d'occupation. Ce plan en concluait **120 s d'attente**, absorbant l'incident
fondateur en un seul appel et laissant 180 s de budget client au tour.

> **MESURE — 2026-09-05, en revue. `A2aClient::DEFAULT_TIMEOUT` n'est pas le budget contraignant.**
>
> Le raisonnement ci-dessus part du plafond le plus *généreux*. Le chemin sur lequel ce ticket a été
> fiché — le rappel `canUseTool` du pilote — n'en dispose pas :
>
> ```
> .claude/claude-pilot.json  (identique dans les cinq dépôts du plan de travail)
> { "command": "mika", "args": ["--agent","mika-dev","ask"], "timeout": 120000 }
> ```
>
> | Appelant | Budget | Couvre |
> |---|---|---|
> | `A2aClient::DEFAULT_TIMEOUT` (`mika-a2a/src/client.rs:21`) | 300 s | attente + tour |
> | relais `canUseTool` de claude-pilot | **120 s** | attente + tour |
>
> Une attente de 120 s dépense donc **la totalité** du budget d'un pilote : le relais tue
> `mika ask` à l'instant exact où l'attente expire, et l'appelant ne lit jamais le refus
> `AGENT_BUSY` qu'on venait de lui construire. C'est le mode d'échec que §2 argumentait éviter,
> reproduit par le chiffre choisi pour l'éviter — parce que le chiffre a été dimensionné contre le
> mauvais plafond.
>
> **Défaut retenu : 30 s.** Un quart du budget contraignant, 90 s laissés au tour. Il absorbe le cas
> courant en un appel (les refus de l'incident étaient à 20 s d'intervalle : une frontière de tour
> tient dedans), et la contention plus longue rend un refus qui **dit combien attendre**
> (`retry_after_ms`) au lieu d'un timeout qui ne dit rien. Absorber les ≈ 100 s complets de
> l'incident en un seul appel n'est disponible à aucun réglage : cela n'entre pas dans 120 s qui
> doivent aussi payer un tour. La borne est épinglée par une assertion dans
> `config::tests::a2a_queue_defaults`, pour qu'une future générosité ait à discuter avec elle.

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
collision protocolaire avec un pair A2A tiers.

> **MESURE — 2026-09-05, à l'implémentation. Le nombre change ; la conception ne change pas.**
>
> Ce plan proposait `-32099` sur le modèle suivant : « JSON-RPC 2.0 réserve -32000..-32099 aux
> erreurs serveur définies par l'implémentation ; la spec A2A s'y taille une plage en numérotant
> depuis -32001 vers le bas ; `-32099` est l'extrémité opposée, la collision exigerait 92 codes de
> plus. » La confrontation à la spec publiée — que §3.3 exigeait **avant merge**, précisément parce
> que ce plan ne pouvait pas la faire — dit autre chose :
>
> > « A2A-specific errors use codes in the range `-32001` to `-32099`. »
> > — `a2aproject/A2A@main`, `docs/specification.md` §9.5
>
> La spec ne « numérote pas vers le bas depuis -32001 » : elle **revendique toute la bande**, dont
> -32001..-32009 sont assignés aujourd'hui (§5.4, y compris `-32008 ExtensionSupportRequired` et
> `-32009 VersionNotSupported`, postérieurs à v0.3). `-32099` n'est donc pas l'extrémité libre d'une
> plage voisine : c'est un numéro dans l'espace de noms d'autrui, en attente d'attribution. La
> clause de repli de ce paragraphe se déclenche telle qu'elle était écrite.
>
> **Retenu : `AGENT_BUSY = -32000`** — le seul code de la bande « erreur serveur définie par
> l'implémentation » que la spec A2A ne revendique pas, et le créneau sémantique exact de ce qu'est
> une contention d'agent. `from_code` reçoit son libellé, `"Agent is busy"`.
>
> Ce n'est pas une réouverture de la décision verrouillée (§1.3) : celle-ci portait sur R-A/R-B, et
> §3.3 avait pré-autorisé le changement de nombre. C'est la clause qui s'exécute, pas le verrou qui
> cède.

**Ce que T7 peut et ne peut pas fixer.** L'unicité locale ne voit pas cette classe de collision —
un code libre dans le module peut être pris par la spec. Le module porte donc **deux** tests :
`agent_busy_collides_with_no_other_code_in_this_module` (unicité locale) et
`agent_busy_is_outside_the_a2a_reserved_range` (la bande revendiquée, épinglée avec sa citation).
Le second est celui qui aurait rougi sur `-32099`.

L'objet `data` porte ce que le commentaire opérateur du 2026-09-04 00:38 CEST demande — un appelant
ne doit pas deviner combien attendre :

```json
{ "code": -32000,
  "message": "Agent is busy",
  "data": { "reason": "queue_full" | "wait_timeout",
            "retry_after_ms": 30000,
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
| `a2a_queue_wait_timeout_ms` | `MIKA_A2A_QUEUE_WAIT_TIMEOUT_MS` | 30000 | `0` → honoré (ne pas attendre) |
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

Le remède n'est pas un sondage mais une course. `tokio::sync::broadcast::Sender` expose
`closed()` — « completes when all receivers have dropped »
(`tokio-1.53.1/src/sync/broadcast.rs:919`, exemple aux lignes 905-917). La tâche attend donc les
deux événements ensemble :

```rust
tokio::select! {
    guard = agent_lock.clone().lock_owned() => { /* le tour peut commencer */ }
    _ = tx.closed()                          => { /* abandon : permis rendu, agent jamais lancé */ }
}
```

L'abandon a lieu **avant l'acquisition du verrou**, pas après : un appelant parti ne prend jamais
son tour, et ne le tient jamais devant les suivants. C'est plus fort que le sondage
`receiver_count()` que la version précédente de ce plan proposait — lequel laissait une fenêtre
entre la lecture et l'acquisition, et supposait une périodicité pour la refermer.

**Ce qui borne la détection.** `tx.closed()` complète quand le dernier `Receiver` est droppé ;
côté axum, ce `Receiver` vit dans le flux que `Sse` détient (`a2a.rs:522`, `:893`), et il est
libéré quand hyper abandonne le corps de la réponse. Une pile HTTP ne constate en général la
disparition du pair qu'à la première écriture — mais les deux flux sont montés avec
`.keep_alive(KeepAlive::default())` (`a2a.rs:523`, `:894`), qui écrit périodiquement. La détection
est donc bornée par l'intervalle de keep-alive d'axum, pas indéfinie. **Cette borne est à mesurer,
pas à supposer** : T8 la mesure.

Ce comportement est **nouveau** : aujourd'hui la question ne se pose pas, parce qu'aucune attente
n'existe. Il est donc couvert par AC7 et T8, et non supposé.

### 3.5 Ce qui ne change pas

`agent_lock` reste ce qu'il est : la sérialisation des tours d'un agent est délibérée, le ticket le
dit en hors-périmètre. Ce plan change ce qui arrive au demandeur pendant qu'il attend.

Les trois `try_lock_owned()` de `crates/mika-agent/src/server/rewind.rs` (`:134`, `:187`, `:273`)
restent inchangés : le ticket nomme deux points de prise, et le rewind est une opération
d'administration où un refus immédiat est le bon comportement. Nommé ici pour que l'omission soit
lue comme un choix.

## 4. Étapes

1. **`crates/mika-a2a/src/jsonrpc.rs`** — `pub const AGENT_BUSY: i32 = -32099;` + le bras dans
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
6. **`docs/openapi/mika-spirit.yaml`** — ~~vérifier si le fichier décrit `/a2a`~~. **Lu le
   2026-09-05 : il ne le décrit pas.** Le seul `'429'` du fichier (ligne 132) appartient à
   `POST /message`, et aucun chemin `/a2a` n'y figure (`grep -n 'a2a' docs/openapi/mika-spirit.yaml`
   → aucune correspondance). Rien à consigner : la spec ne décrit pas la surface que ce ticket
   change. Noté ici pour que l'absence de diff sur ce fichier se lise comme une lecture faite, pas
   comme une étape sautée.
7. **`crates/mika-agent/CLAUDE.md`** — la note `server::webhook_queue_v2` (ligne 225) dit que ce
   mécanisme « remplace le motif hérité `try_lock_owned()` → 429 » de `POST /message` ; ajouter la
   phrase qui dit que `/a2a` a sa propre forme d'attente et pourquoi elle diffère, pour qu'un
   lecteur futur ne relance pas l'unification sans relire le §1.1.

## 5. Tests

| # | AC | Test | Où |
|---|---|---|---|
| T1 | AC4 | Deux appels A2A concurrents sur le même agent : le premier tient le verrou, le second **attend et aboutit** au lieu de recevoir `"Agent is busy"` | `server::tests::a2a_send_waits_for_a_busy_agent_instead_of_refusing` |
| T2 | AC5 | `a2a_queue_enabled = Some(false)` + verrou tenu → `-32603 "Agent is busy"` immédiat, **code et message identiques à aujourd'hui** | `server::tests::a2a_send_with_the_queue_disabled_refuses_exactly_as_before` |
| T3 | AC2 | File pleine (profondeur 1, N+1 attendants) → `AGENT_BUSY`, `data.reason = "queue_full"`, `retry_after_ms` présent | `server::tests::a2a_send_refuses_with_queue_full_when_the_line_is_saturated` |
| T4 | AC2 | Délai dépassé (`wait_timeout_ms` court, verrou jamais rendu) → `AGENT_BUSY`, `data.reason = "wait_timeout"` | `server::tests::a2a_send_refuses_with_wait_timeout_when_the_lock_never_frees` |
| T5 | AC3 | Les trois accesseurs `effective_a2a_*` : absent → défaut ; `0` sur la profondeur → défaut + `warn!` ; `0` sur le délai → honoré | `mika-common/src/config.rs` tests, sur le modèle de `config.rs:2970-2978` |
| T6 | AC1 | Les deux portes passent par le même chemin : un test par porte, `message/send` **et** `message/stream`, sur T1 | `server::tests::a2a_stream_waits_for_a_busy_agent_instead_of_refusing` |
| T7 | — | `AGENT_BUSY` n'entre en collision avec aucun code A2A du module **ni avec la bande revendiquée par la spec** | `crates/mika-a2a/src/jsonrpc.rs` tests (deux tests, §3.3) |
| T8 | AC7 | `message/stream` : le client coupe pendant l'attente du verrou → `tx.closed()` gagne le `select!`, la tâche rend son permis et **n'acquiert jamais** `agent_lock` ; l'agent n'est jamais lancé (§3.6) | `server::tests::a2a_stream_abandoned_mid_wait_never_starts_a_turn` |
| T9 | AC1, AC2 | Concurrence réelle sur le mécanisme : tout attendant admis finit servi (équité FIFO), la borne compte les **attentes** et non les tours, 16 arrivées simultanées sur 8 places ne sur-admettent jamais, et l'interrupteur ne touche pas la file | `crates/mika-agent/tests/a2a_queue_contention.rs` (nouveau) |

> **Où vivent les tests, et pourquoi pas là où ce plan l'écrivait.** Le plan plaçait T1–T4, T6 et T8
> dans `tests/a2a_queue_contention.rs`. `server::a2a` est un module **privé** (`server/mod.rs:1`,
> `mod a2a;`) : un test d'intégration ne peut pas atteindre les deux portes. Les faire passer par
> là aurait exigé soit de rendre publics les handlers HTTP, soit de tester le mécanisme au lieu du
> câblage — c'est-à-dire de laisser vert exactement le refactor qui laisserait les deux portes sur
> `try_lock_owned()` avec un module d'attente parfait à côté. T1–T4, T6 et T8 vivent donc dans
> `server::tests`, où le harnais HTTP réel (`test_state_with_settings` + `build_router`, celui de
> mika#2070) existe déjà et exerce les vraies portes. Le fichier que le plan nomme existe et porte
> T9 : la concurrence sur le mécanisme public, que le harnais HTTP ne montre pas.

T1 est le test qui porte le ticket : il est une **vraie concurrence** — le verrou est tenu par une
tâche tierce pendant que la requête est en vol, et le test vérifie explicitement qu'elle n'a pas
répondu tant qu'il l'est. Deux appels séquentiels montreraient qu'un second appel après un premier
réussit, ce qui est vrai même aujourd'hui. Même piège et même forme que
`crates/mika-agent/tests/dispatcher_contention.rs`, qui le documente.

**Contrôle rouge-avant, terme par terme (2026-09-05).** Neutraliser le branchement
(`try_take_slot` rendant toujours `Disabled`, c'est-à-dire le code d'aujourd'hui) fait rougir T1,
T3, T4, T6 et T8 — et laisse T2 **vert**, ce qui est exactement ce qu'il doit faire : T2 atteste le
comportement d'aujourd'hui. Inversement, casser le contrat d'interrupteur (`legacy_busy_error`
rendant `AGENT_BUSY` au lieu de `-32603`) fait rougir T2 seul. Aucun test ne roule sur le succès
d'un autre.

T2 est la non-régression du ticket (AC5) : il doit échouer si quelqu'un « améliore » le chemin
désactivé.

## Acceptance criteria

AC1 à AC5 sont **repris verbatim du corps de mika#2163** — ni renommés, ni renumérotés, ni
reformulés. AC6 à AC8 sont ajoutés par ce plan et signalés comme tels.

- [ ] **AC1** — Le chemin `/a2a/{agent}` (les deux points de prise du verrou, `a2a.rs:226` et `:360`)
  attend dans une file bornée au lieu de refuser sec, en réutilisant le mécanisme de mika#1870
  plutôt qu'en en écrivant un second.
- [ ] **AC2** — La borne de file et le comportement à saturation sont explicites, et le refus à
  saturation porte un code JSON-RPC qui dit la contention, pas `INTERNAL_ERROR`.
- [ ] **AC3** — Un interrupteur de désactivation existe, de la même forme à trois paliers que le reste
  du module (absent → défaut ; illisible → défaut avec WARN ; sentinelle → désactivé), et son
  chemin désactivé rend le comportement d'aujourd'hui.
- [ ] **AC4** — Un test couvre deux appels A2A concurrents sur le même agent : le second attend et
  aboutit, au lieu de recevoir `"Agent is busy"`.
- [ ] **AC5** — Non-régression : avec la file désactivée, le refus immédiat d'aujourd'hui est inchangé.
- [ ] **AC6** *(ajouté — issu du commentaire opérateur du 2026-09-04 00:38 CEST, qui laisse ce point
  « à trancher au grooming »)* — Le refus de saturation porte le délai que l'appelant devrait
  attendre avant de réessayer, et la raison du refus, dans `error.data` :
  `{reason: "queue_full"|"wait_timeout", retry_after_ms, queue_depth}`.
- [ ] **AC7** *(ajouté — couvre le risque que ce plan crée lui-même, §3.6)* — Sur `message/stream`, une
  déconnexion du client pendant l'attente **annule la tâche avant l'acquisition d'`agent_lock`** :
  le permis est rendu, l'agent n'est jamais lancé, aucun tour orphelin n'est exécuté. Méthode de
  vérification : test d'intégration tenant le verrou par une tâche tierce, droppant l'abonné SSE
  pendant l'attente, puis assertion sur le compteur d'appels de l'agent (zéro) et sur la
  restitution du permis. Le délai de détection, borné par le keep-alive SSE, est mesuré et consigné
  par le test — il n'est pas supposé.
- [ ] **AC8** *(ajouté)* — L'attente et le refus sont observables : un événement d'audit à l'entrée en
  attente portant `wait_ms` à la sortie, un événement au refus portant la raison et la profondeur,
  tous deux à débit limité comme `emit_webhook_queue_audit`. Une contention n'est pas silencieuse.

### Tie-back

| AC | Où c'est tenu | Réserve |
|---|---|---|
| AC1 — attente bornée aux deux points de prise, en réutilisant mika#1870 | §3.1, §3.2, étape 4 ; T1, T6 | La réutilisation est celle de la **forme**, pas du code — §1.1, §1.2. Divergence relevée en réconciliation et **tranchée R-A par mika-prime le 2026-09-05** (§1.3). |
| AC2 — borne et saturation explicites, code qui dit la contention | §3.2, §3.3 ; T3, T4, T9 | `-32000` et non `-32099` : la spec A2A revendique **toute** la bande `-32001..-32099` (mesure du 2026-09-05, §3.3) |
| AC3 — interrupteur trois paliers, chemin désactivé = comportement d'aujourd'hui | §3.4 ; T5, T2 | — |
| AC4 — test de deux appels concurrents, le second aboutit | T1, T6 | — |
| AC5 — non-régression file désactivée | §3.4 ; T2 | Le code -32603 est conservé sur ce chemin, délibérément |
| AC6 — le refus dit combien attendre, et pourquoi | §3.3 ; T3, T4 | `retry_after_ms` = le délai configuré, pas une prédiction du tour |
| AC7 — pas de tour orphelin sur le chemin streaming | §3.6 ; T8 | T8 épingle le **mécanisme** (dernier récepteur droppé ⇒ abandon **avant** acquisition) et la comptabilité du permis, en process. Il **ne mesure pas** le délai sur le fil : le harnais `oneshot` n'a pas de transport, et le keep-alive SSE qui borne ce délai est une propriété du transport. Le test le dit plutôt que de laisser lire une assertion en process comme une mesure réseau |
| AC8 — attente et refus observables | étape 5 ; audit à débit limité | Pas de test dédié : la forme est celle de `emit_webhook_queue_audit`, déjà couverte |

## 7. Fire-Disposition

Ce plan introduit trois détecteurs. Ce qui arrive quand chacun tire sur l'existant est **spécifié
ici, avant l'implémentation** — pas décidé au moment où il tire.

| Détecteur | Ce qu'il verrait tirer sur l'existant | Disposition |
|---|---|---|
| **T2 / AC5** — non-régression du chemin désactivé (`-32603` verbatim) | Un rouge signifie que le chemin de repli ne rend plus le comportement d'aujourd'hui : l'interrupteur ne sert plus à revenir en arrière | **(c) Halte et remontée.** Aucune allowlist. Un interrupteur qui ne restaure pas l'état antérieur n'est pas un interrupteur ; le travail s'arrête et la question remonte à l'opérateur. |
| **T7** — non-collision de `AGENT_BUSY` dans le module | Un rouge signifie une collision de constante ; le module est petit et entièrement sous notre contrôle | **(b) Corriger sur place.** Choisir le code libre le plus haut de la plage et poursuivre. Pas de remontée : c'est un nombre, pas une conception. |
| **T8 / AC7** — pas de tour orphelin sur `message/stream` | Un rouge signifie que la déconnexion n'est pas détectée dans un délai utile — le risque que ce plan crée lui-même | **(c) Halte et remontée**, avec une porte de sortie nommée : si la détection s'avère non bornée, la disposition de repli est de **ne pas activer l'attente sur `message/stream`** (`try_lock_owned()` conservé sur cette seule porte) et de livrer `message/send` seul, en rouvrant AC1 sur son périmètre. Livrer les deux portes avec un tour orphelin possible n'est pas une option. |

**Ce qui a effectivement tiré (2026-09-05).** Aucun des trois détecteurs. Ce que la revue a trouvé
était ailleurs, et vaut d'être noté parce que la table ci-dessus ne pouvait pas le prévoir : le
contrat d'interrupteur (AC5) était tenu sur la porte `message/send` et **cassé sur
`message/stream`** — le chemin désactivé y différait le `try_lock_owned()` dans la tâche `spawn`,
donc répondait `200 OK` avec un flux ouvert puis une trame `failed`, là où le code d'avant rendait
un corps JSON-RPC `-32603` sans ligne de tâche, sans diffuseur, sans flux et sans `spawn`. T2 ne
couvrait que la porte `send`, donc rien ne le voyait. Leçon pour la table : **un détecteur écrit
pour un contrat doit exister une fois par porte qui prétend le tenir**, pas une fois par contrat.
Corrigé, et couvert par `a2a_stream_with_the_queue_disabled_refuses_exactly_as_before`.

**Aucune violation préexistante à mettre en allowlist.** Les trois détecteurs sont nouveaux et
portent sur du code nouveau ; il n'existe pas de population héritée qu'ils feraient rougir. Les
appelants qui recevaient `-32603` recevront `-32099` — c'est le changement voulu par AC2, pas une
violation tolérée.

## 8. Hors périmètre

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

## 9. Risques

- **Le pire cas est un timeout client, pas un refus.** Si l'attente serveur et le tour d'agent
  additionnés dépassent le budget de l'appelant, le client coupe et l'appelant voit une erreur de
  transport — moins lisible que le `-32603` d'aujourd'hui. **Ce risque s'est réalisé dans le plan
  lui-même** : le défaut de 120 s était dimensionné contre le budget de 300 s du client A2A alors
  que le relais du pilote n'en a que 120 (mesure de §2). Défaut ramené à 30 s ; T4 fixe la borne.
- **`returnImmediately` ne prend plus le verrou, et c'est un changement de comportement assumé.**
  Cette branche crée la ligne de tâche et rend un `submitted` sans jamais lancer la boucle : elle
  n'a besoin ni du tour ni d'une place. Prendre le verrou y était inoffensif tant que c'était un
  `try_lock` ; sous une attente, un appelant *fire-and-forget* stationnerait tout le budget et
  tiendrait une place devant des appelants qui, eux, ont besoin d'un tour. Épinglé par
  `a2a_send_return_immediately_neither_waits_nor_takes_a_place`.
- **Un cycle d'appels entre agents ne échoue plus vite, il stationne.** Si le tour de A lance
  `mika ask --agent B` pendant que le tour de B lance `mika ask --agent A`, chaque appel interne
  était refusé en microsecondes ; il bloque désormais jusqu'au budget d'attente pendant que son
  propre appelant tient le verrou de son propre agent. C'est inhérent au fait d'attendre plutôt que
  de refuser — aucune attente bornée ne l'évite. Ce qui borne le dégât est le budget lui-même (30 s
  et non 120) et l'interrupteur. Si les cycles inter-agents deviennent un motif de travail plutôt
  qu'un accident, la réponse est un détecteur de cycle, pas une attente plus longue.
- **La borne est par agent, pas globale.** Huit attendants sur chacun de N agents, c'est 8N tâches
  parkées. Chacune est une tâche tokio en attente sur un `Notify`, pas un fil ; le coût est
  négligeable et la borne existe. Nommé pour que ce ne soit pas une découverte.
- **Le permis libéré à l'acquisition du verrou** (§3.2) est le point subtil : le libérer à la fin du
  tour ferait compter attente et exécution dans la même borne. À vérifier explicitement en revue.
- **Le tour orphelin du chemin streaming** (§3.6) est le risque que ce plan crée lui-même : il
  n'existe pas aujourd'hui, parce qu'aujourd'hui personne n'attend. La course
  `select! { lock_owned(), tx.closed() }` l'annule avant l'acquisition ; AC7 et T8 le mesurent ; et
  §7 nomme la disposition si la détection s'avère non bornée — livrer `message/send` seul plutôt
  que les deux portes avec un tour orphelin possible. Le résidu n'est donc pas le comportement,
  c'est le **délai de détection**, borné par le keep-alive SSE et mesuré par T8.
- **La duplication de forme avec mika#1870** (§1.3) est assumée et triplement renvoyée. Le risque
  n'est pas technique, il est de lecture : qu'un futur passage l'unifie sans relire le §1.1.
