# Vol V2b — les callbacks arrivent par le SSE

> - **Ticket :** senara-solutions/mika#1727
> - **Repo :** `senara-solutions/mika`
> - **Branch :** `feat/1727/tui-v2b-callbacks-par-le-sse` (neuve depuis `main`, first-push)
> - **Base mesurée :** `225d87ad1` (`refactor(1727): la TUI ne construit plus de moteur de tâches (vol V2a/4)`, #3061)
> - **PR :** porte `Refs mika#1727`, **jamais** `Closes`. Le ticket se ferme au merge de V4, et seulement là.
> - **Plan de contrôle :** ce vol ne touche ni `.github/workflows/`, ni `.claude/`, ni `skills/bundled/`, ni `crates/mika-agent/src/well_known_agents.rs`, ni `.mika/`, ni aucun chemin de CODEOWNERS.
> - **Gel des tickets (jusqu'au 01/12) :** ce vol n'ouvre **aucun** ticket, pas même de suivi ni de sous-ticket. Tout reste et tout manque va en commentaire sur mika#1727 ou dans le corps de la PR, noté `Tracked in: senara-solutions/mika#1727`.
> - **Revue de code :** déléguée à mika-qa (mika#2694). Ni `ce-simplify-code`, ni `ce-code-review`.

---

## 0. Ce que la mesure dit sur `225d87ad1`

Le plan V2 (`docs/plans/2026-11-08-002-feat-1727-tui-v2-etat-local-par-mika-spirit-plan.md`,
§ 0.2 et § D8) est du **matériau mesuré**, pas le plan de ce vol. Les chiffres
ci-dessous sont remesurés sur `225d87ad1`.

### 0.1 Le fil émet déjà, et il n'est pas dormant

`GET /api/v1/dashboard/tasks/stream` (`server/mod.rs:338-346`, auth
dashboard-ou-interne) sert `tasks_stream::handle_tasks_events_stream`. Depuis
mika#1758, `async_db.rs` émet depuis dix sites couvrant six transitions. Trois
d'entre eux portent ce vol :

| site | ligne | trame |
|---|---|---|
| `AsyncDatabase::update_task_completed` | `async_db.rs:540-566` | `TaskCompleted` |
| `AsyncDatabase::update_task_failed` | `async_db.rs:591-616` | `TaskFailed` |
| `AsyncDatabase::promote_task_completed` | `async_db.rs:619-648` | `TaskCompleted` (second légitime pour le même `task_id`, justifié sur place) |

Les trois n'émettent que si l'`UPDATE` gardé a réellement transité
(`if transitioned`) : `Ok(false)` ne produit aucune trame. Le canal est attaché
**dans le processus serveur seul** (`server/mod.rs:1914`, `server/state.rs:312`) ;
dans la TUI l'`OnceLock` est vide, donc une écriture côté TUI n'émet rien. C'est
la topologie que ce vol exploite, pas une à construire.

**Deux commentaires de production mentent encore** et disent ce fil dormant :

- `server/mod.rs:335-337` : *« Wire-only in v1 — emission from task_engine transition sites lands in a follow-up ticket. »*
- module doc de `server/tasks_stream.rs:8-12` : *« Emission from `task_engine` transition sites is deferred to a follow-up ticket; the wire is dormant until that lands. »*

Ce suivi **a atterri** (mika#1758). Les deux phrases tombent dans ce vol — c'est
la dette la plus directement toxique du lot : elle dit à un lecteur de ne pas
compter sur un fil qui marche.

### 0.2 Les trames ne portent pas les cinq champs du callback

`TaskEventFrame` (`tasks_stream.rs:73-134`, `#[serde(tag = "event", rename_all = "snake_case")]`,
avec un `#[serde(other)] Unknown` attrape-tout) :

```rust
TaskCompleted { task_id: String, agent_id: String, completed_at: String,
                result_preview: Option<String> },
TaskFailed    { task_id: String, agent_id: String, failed_at: String,
                error_preview: Option<String> },
```

`AgentRequest::CallbackResult` (`tui/app.rs:289-315`) consomme six champs :
`task_id`, `label`, `result`, `trace_id`, `original_status`, `parent_task_id`.
Cinq manquent sur la trame.

Deux faits mesurés tranchent l'arbitrage du § D8 :

1. **`result_preview` est plafonné par contrat.** `PREVIEW_CAP_CHARS: usize = 500`
   (`tasks_stream.rs:49`), et son doc-comment dit lui-même où va le reste :
   *« Frames are glanceable receipts; the full call remains queryable via
   `GET /api/v1/tasks/{task_id}` for the details view. »* Les constructeurs
   `TaskEventFrame::completed` / `::failed` (`tasks_stream.rs:140-168`) imposent
   l'invariant via `truncate_preview`. Le poll actuel injecte le résultat
   **entier** dans la conversation : porter le contenu sur la trame casserait le
   critère 3, ou forcerait l'élargissement d'un plafond que son propre
   doc-comment refuse.
2. **Les sites d'émission n'ont pas la ligne en main.** Les trois closures ne
   tiennent que `id`, `agent_id` et `result`/`error` ; aucune ne lit la ligne
   `tasks`. Chaque champ additif coûte donc une lecture **sur le chemin critique
   de toute complétion de tâche**, pour un fil que zéro ou un abonné lit.

### 0.3 L'endpoint de détail existe et porte tout

`GET /api/v1/tasks/{task_id}` (`server/mod.rs:204` →
`dashboard::handle_task_detail`, `server/dashboard.rs:786-858`) rend un
`TaskDetailResponse` qui porte déjà `label`, `parent_task_id`, `status`,
`created_by_session`, `created_trace_id` **et le `result` non tronqué**. Les cinq
champs manquants y sont tous. **Aucun endpoint n'est à créer.**

### 0.4 Un consommateur SSE complet existe côté client

`commands/tasks.rs::stream_task_events` (`tasks.rs:409-471`) ouvre le flux,
pose `authorization: Bearer <token>` (`dashboard::auth_token()`) et
`accept: text/event-stream` sur l'URL dérivée de `dashboard::spirit_url()`,
puis boucle sur `resp.chunk()` dans un tampon drainé par
`find_event_separator` (`tasks.rs:481-494`) — qui essaie `\r\n\r\n`, puis
`\n\n`, puis `\r\r`, parce qu'axum émet `\n\n` mais qu'un proxy peut
normaliser, et que ne matcher que `\n\n` ferait **paraître le flux bloqué**.
Son doc-comment (`tasks.rs:405-408`) se nomme lui-même « stub consumer […]
Diagnostic — not integrated into the TUI. mika#1727 handles TUI consumption. »

**~65 lignes à promouvoir, pas à réécrire** (§ D8).

### 0.5 Ce que la TUI fait aujourd'hui, et avec quoi

`poll_callback_tasks` (`tui/app.rs:1595-1704`, ~110 lignes), appelée au tick
(`app.rs:1126`) : fenêtre de 7 jours, `get_undelivered_callback_tasks_for_session(&since, &self.session_id)`,
garde de fraîcheur sur les échecs (`STALE_FAILED_CALLBACK_MINUTES`, nettoyage
silencieux + message agrégé), réclamation atomique (`mark_task_delivered` →
`Ok(true)` injecte, `Ok(false) => continue`), substitution
`FAILED_TASK_FALLBACK`, message système `[{label}] completed|failed`, envoi de
`AgentRequest::CallbackResult`, `status = AgentStatus::Thinking`, et un `break`
(un callback par tick).

Tout ce dont le remplacement a besoin est déjà sur `App` : `self.db: AsyncDatabase`
(`app.rs:576`, qui porte `agent_id()`), `self.session_id` (`app.rs:546`),
`self.is_singleton_session` (`app.rs:550`), `self.agent_tx` (`app.rs:531`). Le
pattern « cloner `self.db` et le passer à une tâche » existe déjà (`app.rs:911`),
et un appel HTTP à spirit depuis la TUI aussi (`app.rs:994-1002`). **`App` n'a
pas de champ `agent_id`** — il se lit sur `self.db.agent_id()`, aucun champ neuf.

### 0.6 Le daemon concourt déjà, et il gagne dans une autre session

`TaskEngine::tick` saute la livraison des callbacks en mode CLI
(`task_engine/engine.rs:1441-1444`, « the TUI's `poll_callback_tasks()` handles
callback delivery »), mais un daemon vivant, lui, balaie
`dispatch_undelivered_callbacks` (`engine.rs:1689-1748`) avec
`get_undelivered_callback_tasks(&since)` — **sans filtre de session** — puis
`dispatch_resume_agent` (`dispatcher.rs:1891`), qui frappe une session **neuve**
`callback-<uuid>` (`dispatcher.rs:2004`) et tourne en `run_silent_agent`
(`dispatcher.rs:2172`). Donc : la course TUI/daemon **préexiste sur main**, elle
est arbitrée par `mark_task_delivered`, et un callback que le daemon gagne
n'atterrit pas dans la conversation de la TUI. Ce vol ne l'aggrave pas — il fait
pencher la course du bon côté (voir D4).

### 0.7 Trou mesuré : rien n'atteste l'émission

Un seul test touche le canal : `receiver_count() == 0`
(`tasks_stream.rs:446`). **Aucun test n'atteste qu'une complétion émet
`TaskCompleted`, ni qu'un échec émet `TaskFailed`.** Le critère 2 repose
entièrement sur cette émission : le vol la met sous test (§ 4.1). Sans ce
socle, un bout-en-bout côté client ne prouverait que le client.

### 0.8 Le bras `CallbackResult` ne peut pas partir dans ce vol

Le § D8 du plan V2 écrit « le bras `CallbackResult` part, l'allowlist à une
entrée de V1 se vide ». La mesure dit que ce n'est pas tenable dans le périmètre
déclaré : `commands/chat.rs:360-401` est le dernier `agent::run_agent` local, et
son remplacement par le fil A2A butte sur `server/a2a.rs:343`, qui écrit
`is_callback_turn: false` **en dur** — le fil ne sait pas exprimer un tour de
callback. Le réparer demande une clé dans `crates/mika-a2a/` et sa lecture dans
`crates/mika-agent/`, soit deux crates hors du périmètre de ce vol et un
changement de contrat de fil. **Le bras reste, et il est désormais déclenché par
le SSE au lieu du poll** ; l'allowlist à une entrée de V1 reste pleine. Résidu
nommé en § 7.

---

## 1. Décisions de conception

### D1 — Aucun champ additif sur les trames, aucun endpoint neuf : le filtre est en deux temps

La trame porte déjà `agent_id`. Le consommateur filtre donc **en deux temps** :

1. **Sur le fil, gratuitement** : `frame.agent_id == self.db.agent_id()`. Une
   trame d'un autre agent est jetée sans aucun aller-retour.
2. **Par le détail** : `GET /api/v1/tasks/{task_id}` →
   `created_by_session == self.session_id`. Une trame d'une autre session de
   **ce** agent est jetée après une lecture, sans rien injecter et sans rien
   écrire.

Pourquoi pas de champ additif (`session_id` sur les deux trames) : le § 0.2.2
le chiffre — chaque champ coûte une lecture de ligne au site d'écriture, sur le
chemin critique de toute complétion, pour un fil à zéro ou un abonné ; et le
§ 0.2.1 interdit de toute façon de porter le **contenu** sur la trame. Un champ
`session_id` seul économiserait un GET par trame du bon agent, et seulement
celles-là : un gain borné et petit, contre un changement de contrat de fil et
une lecture ajoutée à chaque transition. Le fil reste un **reçu**, pas un
transport de charge — c'est ce que son propre doc-comment déclare.

Pourquoi pas d'endpoint neuf : le § 0.3 montre que le détail existant porte les
cinq champs, `result` non tronqué compris. Le ticket autorise un endpoint « si
le plan les nomme et les justifie » ; ici il n'y a rien à justifier, il y a un
endpoint à utiliser.

**`original_status` vient de la variante de trame**, jamais d'un champ :
`TaskCompleted` → `"completed"`, `TaskFailed` → `"failed"`. Un champ serait une
seconde vérité à côté du discriminant, exactement la classe que D3 refuse. Le
`status` du détail n'est pas utilisable pour cela : au moment où la TUI lit, la
réclamation l'a déjà passé à `"delivered"`.

### D2 — Le consommateur SSE est promu, pas réécrit

`find_event_separator` et la boucle de lecture **se déplacent** de
`commands/tasks.rs` vers un module neuf `commands/task_events.rs`, paramétré par
un callback de trame au lieu d'un `println!`, et désérialisant chaque charge
`data:` en `TaskEventFrame` (`mika-cli` dépend déjà de `mika-agent`). Les trois
séparateurs et leur ordre d'essai sont conservés à l'identique — c'est une
propriété durement acquise (§ 0.4), pas un détail.

`commands/tasks.rs::stream_task_events` **reste** et délègue au module : la
sous-commande `mika tasks stream` est son appelant de production, donc le module
promu n'est à aucun moment une garde que rien ne fait tourner (mika#2205,
mika#2578). C'est important : la faute que ce vol corrige *est* celle du
« wire-first, consommation en suivi » (§ 0.1) ; la reproduire en livrant un
consommateur sans consommateur serait la refaire d'un cran.

### D3 — L'abonnement est une tâche, le traitement reste dans le tick

Une `tokio::task` lancée au démarrage de la TUI ouvre le flux et écrit chaque
trame **déjà filtrée sur l'agent** sur un `mpsc::UnboundedSender<TaskEventFrame>`.
`App` tient le `Receiver` et le draine au tick, **à l'endroit exact où
`poll_callback_tasks` était appelée** (`app.rs:1126`) : aucun appel réseau dans
le chemin de rendu, un seul callback traité par tick (le `break` du poll est
conservé), et la suite — détail, filtre session, réclamation, message système,
`AgentRequest::CallbackResult`, `AgentStatus::Thinking` — est littéralement
celle du poll.

Cette forme rend le vol suivant mécanique : le critère 1 se réduit alors à
retirer la fonction et son appel, le remplaçant étant déjà en place et attesté.

Si le flux tombe, la tâche retente avec un petit back-off. Le poll conservé
(D5) couvre le trou pendant la reconnexion.

### D4 — La réclamation reste atomique et locale ; elle est le point d'unicité

`mark_task_delivered` → `Ok(true)` injecte, `Ok(false)` jette. C'est l'`UPDATE`
gardé déjà en place, déjà l'arbitre de la course TUI/daemon (§ 0.6). « Livré une
fois et marqué livré une fois » est une propriété de **cet** `UPDATE`, pas du
transport : elle tient donc identiquement quel que soit le nombre de chemins qui
proposent un candidat.

Pas d'endpoint `POST /tasks/{id}/deliver` : il faudrait justifier une mutation
neuve là où l'`UPDATE` gardé suffit, et le critère 1 ne parle que de **lecture**
(« la TUI ne lit plus la base pour trouver les callbacks terminés et non
livrés »). Le verrou structurel qui interdirait toute écriture locale est le
périmètre déclaré de V4.

Effet de bord favorable : le SSE arrive **à l'instant de la transition**, là où
le balayage du daemon tourne au tick. La course du § 0.6 penche désormais du
côté de la conversation de l'opérateur, qui est le bon.

### D5 — Le poll reste ce vol-ci, en filet subordonné

Ce vol livre les critères 2 et 3 ; le critère 1 part au vol suivant (§ 2). Donc
`poll_callback_tasks` **reste en place**, inchangée, derrière le drain SSE.

Ce n'est pas une seconde vérité : la réclamation atomique (D4) fait du poll un
proposant strictement subordonné — il ne peut que trouver `Ok(false)` sur ce que
le SSE a déjà pris. Et il couvre deux trous réels : la reconnexion (D3) et le
callback terminé pendant que la TUI était fermée, pour lequel il n'y a **aucun
replay SSE** (§ 7.1). L'ordre est le prudent : on ne retire le filet qu'après
avoir vu le trapèze tenir en production.

### D6 — La garde de fraîcheur ne migre pas : elle est déjà en double ailleurs

Sur le chemin SSE une trame arrive à l'instant de la transition : elle n'est
jamais périmée, et `STALE_FAILED_CALLBACK_MINUTES` n'y a aucun sens. La garde
reste dans le poll ce vol-ci, et le daemon la porte déjà à l'identique, même
seuil (`engine.rs:1714-1732`). Quand le poll partira, rien ne sera perdu : la
fonction vit dans le seul processus qui doit la porter, et elle y est déjà.

### D7 — Le `singleton` n'est pas un cas à part, c'est le même filtre

Pour un agent `singleton` (mika#1401), `self.session_id` est la session
canonique de l'agent (`prompt::resolve_canonical_session_id`,
`chat.rs:78`) et c'est aussi le `created_by_session` que spirit a adopté depuis
V1 (`chat_turn.rs:50-52` passe la session de la TUI en `caller_session_id`,
mika#2070). Le filtre de D1 est donc littéralement le même ; le critère 3
l'atteste par une entrée de table de cas, pas par un chemin séparé.

---

## 2. Pourquoi ce vol s'arrête aux critères 2 et 3

Les trois critères ensemble pèsent ~346 lignes estimées (251 ici + 95 au vol
suivant), soit au-delà de la borne par vol de 279 (mika#2693). La règle
prescrit alors un découpage en vols, chacun mergeable seul.

L'axe de découpe n'est pas libre : séparer « le fil » de « son consommateur »
reproduirait la faute du § 0.1. La découpe tenable est **le chemin chaud
d'abord, le retrait du filet ensuite** :

- **Ce vol — critères 2 et 3.** Le callback arrive par le SSE, dans la bonne
  conversation, une seule fois, et le comportement visible est conservé, échec
  et `singleton` compris. Rien de dormant : l'abonnement est branché, le module
  promu a son appelant de production, l'émission passe sous test.
- **Vol suivant — critère 1.** `poll_callback_tasks` et son appel disparaissent,
  un détecteur structurel rougit si `mika-cli` relit la requête en local
  (allowlist épinglée vide + anti-vacuité, sur le patron de V1 et V2a,
  `commands/chat_turn.rs:607-714`), le commentaire `#264` de `chat.rs:412-414`
  devient vrai, et `crates/mika-cli/CLAUDE.md` gagne son paragraphe V2b.
  `Tracked in: senara-solutions/mika#1727`.

Ce découpage se note en commentaire sur mika#1727 et dans le corps de la PR
(gel des tickets). Il n'ouvre rien.

---

## 3. Changements, fichier par fichier

### 3.1 `crates/mika-agent/src/server/mod.rs`

Le commentaire `l. 335-337` dit le contraire du code depuis mika#1758. Il
devient : le flux émet, depuis les sites de transition de `async_db.rs`, dans
le processus serveur seul. Pas de changement de route, pas de changement d'auth.

### 3.2 `crates/mika-agent/src/server/tasks_stream.rs`

Même correction sur le module doc `l. 8-12` : la phrase « the wire is dormant
until that lands » tombe, remplacée par le fait mesuré — émission effective,
canal par processus, consommateurs filtrant sur `agent_id`. La table
« Sibling divergence » et les invariants de `PREVIEW_CAP_CHARS` ne bougent pas :
D1 s'appuie dessus.

### 3.3 `crates/mika-cli/src/commands/task_events.rs` (neuf)

Le consommateur promu. Surface :

- ouverture du flux : URL dérivée de `dashboard::spirit_url()`, en-têtes
  `authorization: Bearer` (`dashboard::auth_token()`) et
  `accept: text/event-stream`, échec explicite sur statut non-succès ;
- `find_event_separator`, **déplacée telle quelle** depuis `tasks.rs:481-494`,
  avec son doc-comment et ses trois séparateurs ;
- la boucle de tampon, paramétrée par un consommateur de trame ;
- désérialisation de chaque charge `data:` en `TaskEventFrame` ; une trame
  inconnue tombe sur `Unknown` et est ignorée sans bruit (l'attrape-tout est
  déjà dans l'enum) ;
- filtre sur `agent_id`, appliqué avant de rendre la trame à l'appelant.

### 3.4 `crates/mika-cli/src/commands/tasks.rs`

`stream_task_events` perd son corps de boucle et délègue au module, en gardant
son `println!` par trame et son doc-comment ajusté (il n'est plus « non
intégré »). `find_event_separator` et ses tests suivent le module.

### 3.5 `crates/mika-cli/src/tui/app.rs`

- un `Receiver<TaskEventFrame>` sur `App`, alimenté par la tâche d'abonnement
  lancée au démarrage (D3) ;
- `drain_task_event_frames` au tick, à l'emplacement de l'appel du poll : pour
  la première trame `TaskCompleted`/`TaskFailed` du tick, `GET /api/v1/tasks/{task_id}`,
  filtre `created_by_session == self.session_id`, puis le chemin existant —
  `mark_task_delivered`, substitution `FAILED_TASK_FALLBACK` sur un `result`
  vide, message système `[{label}] completed|failed`,
  `AgentRequest::CallbackResult` avec `original_status` tiré de la variante
  (D1), `AgentStatus::Thinking` ;
- `poll_callback_tasks` et son appel **restent** (D5).

### 3.6 Ce qui ne bouge pas, et pourquoi

- `commands/chat.rs` : le bras `CallbackResult` est conservé à l'identique
  (§ 0.8) ; son commentaire `#264` reste vrai tant que le poll vit, et tombe au
  vol suivant.
- `async_db.rs` et les trois sites d'émission : aucun champ ajouté (D1). Seuls
  des tests s'y ajoutent.
- `crates/mika-cli/CLAUDE.md` : documenté au vol suivant, quand la forme est
  définitive. L'écrire deux fois serait écrire une fois de trop.

---

## 4. Tests

### 4.1 Socle — l'émission est attestée (`crates/mika-agent`)

Le § 0.7 mesure le trou. Trois tests sur le canal, un par site :

- `update_task_completed` sur une tâche en vol émet exactement une
  `TaskCompleted` portant le `task_id` et l'`agent_id` ;
- `update_task_failed` émet exactement une `TaskFailed` ;
- un second appel sur une tâche déjà terminale (`Ok(false)`) n'émet **rien** —
  c'est l'invariant `if transitioned` du § 0.1, et c'est lui qui empêche une
  double injection côté TUI.

Sans ces trois, le critère 2 reposerait sur une émission supposée.

### 4.2 Critère 2 — de bout en bout, avec contrôle négatif (`crates/mika-cli`)

Un `axum::Router` de test sert les deux surfaces réelles : le flux SSE (émettant
des trames construites par `TaskEventFrame::completed`, donc soumises au même
invariant de troncature que la production) et `GET /api/v1/tasks/{task_id}`
rendant un détail avec `created_by_session`, `label`, `parent_task_id`,
`created_trace_id` et un `result` plus long que `PREVIEW_CAP_CHARS`. En face,
une `AsyncDatabase` temporaire et le chemin de traitement de l'App.

La seule pièce substituée est l'émetteur, et son fidèle est attesté par § 4.1 ;
rien dans le chemin filtre → détail → réclamation → injection n'est simulé.

Assertions, en table de cas sur un seul test :

| trame | agent | session | attendu |
|---|---|---|---|
| `TaskCompleted` | le nôtre | la nôtre | un `CallbackResult` sur `agent_tx`, `result` **entier** (pas le preview), `original_status == "completed"`, ligne marquée `delivered` |
| `TaskCompleted` | **un autre** | la nôtre | rien sur `agent_tx`, rien de marqué, **aucun** appel au détail |
| `TaskCompleted` | le nôtre | **une autre** | rien sur `agent_tx`, rien de marqué |
| la même trame, deux fois | le nôtre | la nôtre | **un seul** `CallbackResult`, **un seul** passage à `delivered` |

La dernière ligne est l'unicité de D4, et elle vaut aussi contre le poll
conservé : le second proposant ne peut que trouver `Ok(false)`.

### 4.3 Critère 3 — le comportement visible est conservé

- `TaskFailed` : `original_status == "failed"`, substitution
  `FAILED_TASK_FALLBACK` sur un `result` vide, message système
  `[{label}] failed`, et le cadrage d'échec du bras `CallbackResult`
  (`is_failed = true`) inchangé.
- Agent `singleton` : la même table de cas avec la session canonique en
  `session_id` — le filtre accepte, et aucun chemin séparé n'existe (D7).
- Les tests existants de `poll_callback_tasks` et du bras `CallbackResult`
  restent verts : ils sont le plancher de « comportement conservé », et D5 les
  laisse applicables.

### 4.4 Non-régression du parseur

Les tests de `find_event_separator` suivent le module (D2) et restent verts
sur les trois séparateurs : `\r\n\r\n`, `\n\n`, `\r\r`, dans cet ordre d'essai.

---

## Fire-Disposition

**Option retenue : (a) exception d'allowlist nommée, sous sa forme dégénérée —
aucune allowlist n'est créée par ce vol.**

Ce vol ne livre **aucun détecteur structurel** : il livre des tests de
comportement (§ 4.1 à § 4.4), qui n'ont ni scan de source, ni allowlist, ni
garde de vacuité à armer. Le scan du critère 1 — celui qui rougira si
`mika-cli` relit `get_undelivered_callback_tasks*` en local — part au vol
suivant (§ 2) et portera sa propre allowlist épinglée **vide**, sur le patron
déjà en place pour V1 et V2a (`commands/chat_turn.rs:607-714` : constante
module-level, anti-vacuité sur le même symbole que le scan lit, et test
d'allowlist vide).

Il n'y a donc **rien à désarmer et rien à excepter** : tous les tests livrés ici
sont armés, verts à la livraison, et chacun rougit sur la régression qu'il
nomme.

Pourquoi pas (b) *land disarmed* : il n'y a pas de détecteur à poser en veille.
Livrer ici un scan désarmé — ou un scan que rien ne fait tourner — serait
exactement la faute que le § 0.1 corrige, d'un cran plus haut : un fil déclaré
dormant par son propre commentaire. Refusé par le sujet même du vol.

Pourquoi pas (c) *halt-and-surface* : rien n'est à arrêter. Aucun test livré ne
rougit sur `main` au moment de la livraison, et aucune dette préexistante n'est
découverte par ce vol qui exigerait de suspendre la livraison. Les trois résidus
connus sont bornés, nommés en § 7, et aucun ne tient à un geste que ce vol
pourrait faire.

---

## 5. Ordre de commit, et le plancher de chacun

Plancher commun à tous : `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`.

1. **`fix(1727): le flux task-events n'est plus déclaré dormant`**
   — `server/mod.rs`, `server/tasks_stream.rs` (§ 3.1, § 3.2).
   Plancher : `cargo test -p mika-agent`.
2. **`test(1727): les sites d'émission des trames de tâche sont attestés`**
   — § 4.1. Plancher : `cargo test -p mika-agent`. Ce commit vient **avant** le
   client : le socle d'abord, ce qui s'appuie dessus ensuite.
3. **`refactor(1727): le consommateur SSE des tâches devient réutilisable`**
   — `commands/task_events.rs`, `commands/tasks.rs` (§ 3.3, § 3.4), tests du
   parseur déplacés (§ 4.4). Plancher : `cargo test -p mika-cli`, et
   `mika tasks stream` reste fonctionnellement identique.
4. **`feat(1727): les callbacks terminés arrivent par le SSE (vol V2b/4)`**
   — `tui/app.rs` (§ 3.5). Plancher : `cargo test -p mika-cli`.
5. **`test(1727): le callback SSE arrive une fois, dans la bonne conversation`**
   — § 4.2, § 4.3. Plancher : `cargo test`, puis
   `make verify-bundled-skills` et `bash scripts/verify-pipeline.sh origin/main`.

---

## 6. Risques

- **Le flux ne porte pas la session : chaque trame du bon agent coûte un GET.**
  Borné par le volume de callbacks d'un agent, qui est petit, et payé seulement
  pour les trames qui ont passé le filtre gratuit de D1. Si la mesure dit un
  jour le contraire, le champ `session_id` sur les deux trames est l'issue — et
  le § 0.2 chiffre déjà ce qu'elle coûterait.
- **La tâche d'abonnement meurt en silence.** Le poll conservé (D5) couvre le
  trou ce vol-ci ; au vol suivant, ce filet s'en va, donc le back-off et son
  observabilité devront y être attestés. Noté dans la PR.
- **Un `Unknown` masque une trame future.** L'attrape-tout est déjà dans l'enum
  et c'est le bon défaut sur un fil versionné ; le coût est qu'une trame neuve
  non gérée est ignorée sans bruit. Hors périmètre, inchangé par ce vol.

---

## 7. Résidus et comportements assumés

Tous vont en commentaire sur mika#1727 et dans le corps de la PR. **Aucun
ticket n'est ouvert** (gel jusqu'au 01/12). Chacun porte
`Tracked in: senara-solutions/mika#1727`.

### 7.1 Il n'y a pas de replay SSE

Un callback terminé pendant que la TUI est fermée ne produit aucune trame pour
la session qui rouvre : la fenêtre de 7 jours du poll était ce rattrapage. Rien
n'est perdu — le balayage du daemon le livre (§ 0.6) — mais il atterrit dans une
session `callback-<uuid>` en tour silencieux, pas dans la conversation de la
TUI. **Changement de comportement assumé**, et c'est précisément pourquoi le
poll reste ce vol-ci (D5) : le vol qui le retire devra dire ce qui le remplace.

### 7.2 La course daemon/TUI préexiste et survit

`dispatch_undelivered_callbacks` est aveugle à la session (§ 0.6). Ce vol ne la
résout pas ; il fait pencher la course du bon côté (D4) et laisse
`mark_task_delivered` arbitre. Hors périmètre déclaré.

### 7.3 Le bras `CallbackResult` et l'allowlist de V1 restent

Bloqué par `server/a2a.rs:343` (`is_callback_turn: false` en dur), hors des deux
crates du périmètre (§ 0.8). Le § D8 du plan V2 annonçait leur départ à ce
vol : il ne l'est pas, et le dire est préférable à le tenter à moitié.

### 7.4 Le critère 1 part au vol suivant

Découpage justifié par la borne par vol (§ 2, mika#2693).

---

## Acceptance criteria

Transcrits du corps de mika#1727 (critères de V2b). Les critères 2 et 3 sont
livrés par ce vol ; le critère 1 est porté par le vol suivant (§ 2).

1. **La TUI ne lit plus la base pour trouver les callbacks terminés et non
   livrés : `poll_callback_tasks` et son appel disparaissent. Un test rougit si
   `mika-cli` relit cette requête en local.**
   → **Non livré par ce vol.** Découpé par la borne par vol (§ 2).
   `Tracked in: senara-solutions/mika#1727`.
2. **Un callback terminé par le daemon arrive dans la conversation de la TUI
   ouverte sur son agent et sa session, porté par le SSE. Il est livré une fois
   et marqué livré une fois. Le test est de bout en bout, avec un contrôle
   négatif : une autre session et un autre agent ne reçoivent rien.**
   → Livré. § 3.3 à § 3.5 pour le chemin, § 4.1 pour l'émission, § 4.2 pour le
   bout-en-bout et ses trois contrôles négatifs (autre agent, autre session,
   seconde trame identique).
3. **Le comportement visible des callbacks est conservé, y compris pour un
   échec, et un agent `singleton` compris.**
   → Livré. § 4.3, et les tests existants comme plancher (D5, D7).

---

## Definition of Done

- [ ] Les deux commentaires périmés de `server/mod.rs:335-337` et du module doc
      de `server/tasks_stream.rs:8-12` ne disent plus le fil dormant.
- [ ] Les trois sites d'émission de `async_db.rs` sont sous test, y compris
      l'invariant « `Ok(false)` n'émet rien » (§ 4.1).
- [ ] `commands/task_events.rs` porte le consommateur promu ; `mika tasks stream`
      délègue et reste fonctionnellement identique ; les trois séparateurs du
      parseur sont conservés et testés.
- [ ] La TUI reçoit les trames de son agent par SSE, filtre sur sa session via
      le détail, réclame atomiquement, et injecte le `result` **entier** dans la
      conversation.
- [ ] Le test de bout en bout du critère 2 passe, avec ses trois contrôles
      négatifs et l'assertion d'unicité (§ 4.2).
- [ ] Le critère 3 est attesté pour l'échec et pour un agent `singleton`, et les
      tests existants du poll et du bras `CallbackResult` restent verts.
- [ ] Aucun champ ajouté aux trames, aucun endpoint créé (D1).
- [ ] Aucun fichier du plan de contrôle touché.
- [ ] `cargo test` vert sur le workspace.
- [ ] `cargo clippy --all-targets -- -D warnings` vert.
- [ ] `cargo fmt --check` vert.
- [ ] `make verify-bundled-skills` vert.
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] Le corps de la PR porte `Refs mika#1727` (jamais `Closes`),
      `Code review: delegated to mika-qa (mika#2694)`, les quatre résidus du
      § 7 et une ligne `Tracked in: senara-solutions/mika#1727` pour chacun.
- [ ] Aucun ticket ouvert (gel jusqu'au 01/12).

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `server/mod.rs` + `server/tasks_stream.rs` — les deux commentaires périmés tombent | 8 |
| `commands/task_events.rs` — consommateur SSE promu, trames désérialisées, filtre sur l'agent | 35 |
| `commands/tasks.rs` — `stream_task_events` délègue au module promu | 8 |
| `tui/app.rs` — réception des trames, détail HTTP, filtre session, réclamation, injection | 60 |
| tests `mika-agent` — les trois sites d'émission sont attestés sur le canal | 40 |
| tests `mika-cli` — critère 2 de bout en bout, table de cas et contrôles négatifs | 70 |
| tests `mika-cli` — critère 3 : échec et agent `singleton` | 30 |

Total estimé : 251 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Pourquoi la borne tient.** 251 < 279, et sous le seuil de 1000 lignes par PR,
donc un seul vol et une seule phase. Elle ne tenait pas pour les trois critères
réunis (~346 lignes), d'où le découpage du § 2 : le chemin chaud ici, le retrait
du poll et son détecteur structurel au vol suivant,
`Tracked in: senara-solutions/mika#1727`.

Deux éléments rendent cette estimation prudente plutôt qu'optimiste : le
consommateur SSE est **déplacé** et non écrit (§ 0.4, 65 lignes existantes dont
seules ~35 sont neuves), et le chemin de traitement de `tui/app.rs` reprend
littéralement celui de `poll_callback_tasks` après le filtre — réclamation,
substitution d'échec, message système, `AgentRequest::CallbackResult`,
`AgentStatus::Thinking` — sans en réinventer une étape. Les tests pèsent 140 des
251 lignes, soit 56 % : c'est le profil normal d'un vol dont deux des trois
critères sont des assertions de comportement.
