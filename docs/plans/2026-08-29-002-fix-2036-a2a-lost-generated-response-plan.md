---
issue: senara-solutions/mika#2036
type: fix
status: draft
branch: bug/2036/a2a-cli-une-r-ponse-g-n-r-e-est-perdue
date: 2026-08-29
---

# mika#2036 — une réponse générée est perdue au transport

## Le fait, remesuré

`mika ask` rend `Error: … connection error: error sending request` alors que **le serveur a
entièrement généré la réponse**. Le travail est fait, facturé au fournisseur, puis jeté ; l'appelant
ne reçoit rien et ne sait pas que la réponse existe.

n=3 le 2026-08-29, sur les trois briefs de revue de plan de la nuit. Les deux réponses perdues ont été
**récupérées à la main** dans `/var/log/mika/server.log` (entrées `llm response body`) — elles étaient
complètes, substantielles, et l'une portait le `Disposition: ITERATE` qui a fait avancer le grooming
de mika#2013.

La taille n'est pas le discriminant : 10 492 o passe, 8 345 o échoue. Le seul appel livré au-delà de
la trivialité a duré 114 s ; les deux perdus étaient des générations plus longues. **La durée de
génération est le facteur, pas le volume.**

## Ce que dit le code

### Aucun timeout, aucun retry

`crates/mika-a2a/src/client.rs:20` :

```rust
http: reqwest::Client::new(),
```

Pas de `ClientBuilder`, pas de `.timeout(...)`, pas de politique de reprise. L'envoi est un
`req.send().await?` nu (`:73`), et le `?` convertit toute erreur de transport en
`A2aError::ClientError`.

### L'erreur ment sur elle-même

`crates/mika-cli/src/remote_ask.rs:136` :

```rust
Err(A2aError::ClientError(e)) => anyhow::bail!("connection error: {e}"),
```

Toute `ClientError` devient « connection error », sans distinguer « je n'ai pas pu joindre le
serveur » de « j'ai attendu N secondes et abandonné ». Coût constaté : ce message m'a fait conclure à
un serveur à terre alors que `/health` répondait en **0,5 ms**, et m'a fait perdre du temps sur une
fausse piste avant de trouver la réponse dans le log.

### Le chemin de récupération existe déjà dans le protocole

Trois pièces sont en place et ne sont pas reliées :

1. **`tasks/get` est une méthode A2A définie** — `crates/mika-a2a/src/jsonrpc.rs:135`
   (`"tasks/get" => Some(Self::TasksGet)`).
2. **Le serveur l'implémente** — `crates/mika-agent/src/server/a2a.rs:93` route vers
   `handle_tasks_get`, qui résout par `agent_state.db.a2a_build_task(&params.id, …)` (`:522-525`).
   **Les tâches sont donc persistées et récupérables par identifiant.**
3. **L'appelant fournit déjà un identifiant** — `MessageSendParams.message` est un `Message`
   (`params.rs:10`) et `Message` porte `pub message_id: String` (`types.rs:97`).

Ce qui manque : `A2aClient` n'expose **que** `send_message`. Il n'a aucune méthode `tasks/get`, et
surtout il ne conserve rien qui lui permette de renommer la tâche après un échec de transport.

## Correctif — trois volets

**Le cœur est le volet C**, conformément au corps du ticket (« Le point 3 est le cœur : c'est la
classe *un rapport qui ment sur sa propre situation* »). A et B sont présentés d'abord parce qu'ils
sont indépendants et sans décision en suspens ; C vient en dernier parce qu'il porte la seule
décision de conception à trancher, pas parce qu'il compte moins.

### Volet A — l'erreur dit la vérité (indépendant)

Distinguer, dans `remote_ask.rs`, les classes que `A2aError::ClientError` confond aujourd'hui :

- **injoignable** — connexion refusée, DNS, socket fermée avant réponse ;
- **délai dépassé** — la requête a été envoyée, N secondes se sont écoulées, on a abandonné ;
- **réponse illisible** — reçue mais non décodable.

Le message doit nommer la durée attendue et l'URL, et **dire que la réponse peut exister côté
serveur**. C'est le volet le moins cher et celui qui a le plus coûté cette nuit.

### Volet B — un timeout explicite (indépendant)

Remplacer `reqwest::Client::new()` par un `ClientBuilder` avec un timeout **choisi**, généreux et
adapté à une génération LLM longue. Aujourd'hui le comportement est celui du défaut de la
bibliothèque, ce qui n'est pas une décision.

**Compatibilité ascendante** (passe architecte 1, F2). `A2aClient::new` **garde sa signature
actuelle** et gagne un timeout par défaut généreux ; un constructeur `with_timeout(dur)` est ajouté
pour les appelants qui veulent un budget propre. Ne pas changer `new()` : une revue de plan de 10 Ko
et une sonde de santé n'ont pas le même besoin, mais aucun appelant ne doit être forcé de migrer.

Les call sites sont connus et il y en a **exactement deux** :

- `crates/mika-cli/src/remote_ask.rs:131` — `A2aClient::new(url, auth_token)` ;
- `crates/mika-agent/src/tools/a2a_call.rs:123` — `A2aClient::new(url, api_key)`.

Aucun autre dans le dépôt (`grep -rn "A2aClient::new" crates/`). Les deux continuent de compiler sans
modification.

### Volet C — ne pas perdre une génération terminée (le cœur)

`A2aClient` gagne une méthode `get_task(id)` qui appelle `tasks/get`, et `remote_ask` s'en sert : sur
échec de transport **après envoi réussi**, retenter une fois la récupération par identifiant plutôt
que d'abandonner.

**Décision tranchée** (passe architecte 1, F1 bloquant — résolue par lecture du code, pas reportée
à l'implémenteur).

Le client doit pouvoir nommer la tâche sans avoir reçu la réponse. Trois formes étaient possibles ;
le code en élimine deux.

- **C1 — corréler sur `message.message_id` : REJETÉE.** `crates/mika-agent/src/server/a2a.rs:221`
  frappe `let task_id = Uuid::new_v4().to_string();` — **l'identifiant de tâche est généré par le
  serveur**, et le `message_id` du client n'entre nulle part dans son calcul. Le client ne peut pas le
  connaître ni le dériver. C1 n'est pas viable.
- **C2 — l'appelant fournit l'identifiant de tâche : ÉCARTÉE comme trop lourde.** `Message` porte déjà
  `task_id: Option<String>` (`crates/mika-a2a/src/types.rs:103`), mais le serveur l'ignore et frappe
  le sien. L'honorer changerait la sémantique du protocole côté serveur pour tous les appelants.
- **C3 — corréler sur le `context_id` fourni par le client : RETENUE.** `Message` porte
  `context_id: Option<String>` (`types.rs:101`), le serveur le lit à `a2a.rs:222`
  (`params.message.context_id.clone()`) et le **persiste** avec la tâche via
  `a2a_create_task(a2a_task_id, agent_id, context_id)` (`async_db.rs:2656-2668`, `a2a_db.rs:70`).

**Forme retenue.** `remote_ask` génère un `context_id` avant l'envoi et le place dans le message. Sur
échec de transport survenu après envoi, il interroge le serveur par ce `context_id` pour récupérer la
tâche et sa réponse. Le protocole `message/send` ne change pas ; il faut seulement un chemin de
lecture « donne-moi la tâche de ce contexte » côté serveur, adossé à une colonne déjà écrite.

C'est la forme de moindre surface : rien de nouveau dans le contrat d'envoi, aucune sémantique
modifiée pour les appelants existants, et un identifiant que le client choisit déjà légitimement.

## Critères d'acceptation

- **AC1** — Un délai dépassé et une connexion refusée produisent des messages **distincts**, chacun
  nommant l'URL ; le message de délai nomme la durée attendue. Test anti-vacuité : les deux cas
  vérifiés, et l'assertion échoue si les deux messages sont identiques.
- **AC2** — Le timeout du client A2A est explicite dans le code, pas hérité du défaut de `reqwest`.
  `A2aClient::new` **garde sa signature** ; `with_timeout` permet de le surcharger. Test : deux
  clients construits avec des budgets différents les portent effectivement, et les deux call sites
  existants (`remote_ask.rs:131`, `a2a_call.rs:123`) compilent sans changement.
- **AC3** — Après un échec de transport survenu **après** l'envoi, l'appelant récupère la réponse
  générée en interrogeant par le `context_id` qu'il a lui-même fourni. Test (forme validée en passe
  architecte 1) : un `tokio::net::TcpListener` accepte la connexion, lit la requête JSON-RPC, puis
  ferme brutalement le socket (`drop(stream)`) sans écrire de réponse ; l'appelant doit **rendre la
  réponse** via la récupération, pas une erreur. Anti-vacuité : un échec **avant** l'envoi (port
  refusé dès le départ) doit toujours rendre une erreur, jamais une récupération fantôme.
- **AC4** — Le message d'erreur, quand la récupération échoue elle aussi, **dit où chercher** :
  identifiant de session ou de tâche, afin qu'un humain ou un pilote puisse retrouver la réponse.
- **AC5** — `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` verts.

## Hors périmètre

- Le mécanisme de journalisation `llm response body` dans `server.log`, qui a servi de filet cette
  nuit. Il n'est pas conçu pour ça et ne doit pas devenir un contrat.
- Le comportement de `mika-arch` qui rend une disposition sans revue — mika#2037, autre défaut du
  même chemin, sans rapport de cause.
- Un mécanisme de reprise généralisé (file, réémission automatique). Ce ticket rend une réponse déjà
  produite récupérable ; il ne construit pas une garantie de livraison.

## Pourquoi p1

Chaque passe architecte d'un grooming envoie un brief de plusieurs kilo-octets et attend une
génération longue — exactement le profil qui perd sa réponse. Les **deux** passes de mika#2013 ont été
perdues ; je les ai récupérées à la main dans le log serveur, et la seconde a dû tourner sans
`--session-id` parce que celui-ci voyageait dans l'enveloppe perdue. Un pilote autonome n'a pas ce
réflexe : il lit « connection error » et conclut à un échec.

## Historique de grooming

- Passe architecte 1 (`mika-arch`, 2026-08-29) — **ITERATE**, deux constats.
  - **F1 (bloquant)** : décision C1/C2 non résolue (mika#1244 Unresolved-Decision Gate). **Résolue
    par lecture du code** plutôt que par un critère laissé à l'implémenteur : C1 rejetée
    (`a2a.rs:221`, UUID serveur), C2 écartée, **C3 retenue** (corrélation par `context_id`, déjà
    fourni par le client et déjà persisté).
  - **F2 (affinage)** : compatibilité ascendante du timeout. Appliquée — `new()` garde sa signature,
    `with_timeout` ajouté, et les **deux** call sites sont énumérés.
  - Point 2 (testabilité d'AC3) validé, avec la forme de test reprise dans l'AC.

## Lié

- mika#2013 — le grooming pendant lequel c'est apparu, deux réponses perdues.
- mika#2037 — l'autre défaut du même chemin (disposition sans revue). Cause distincte.
- mika#2040 — le transcript par appel LLM qui n'a pas de lecteur : troisième surface d'observation
  absente de la même nuit.
