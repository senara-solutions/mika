# mika#1727 — vol V4-a : spirit lance une équipe et en diffuse les événements

- **Ticket** : senara-solutions/mika#1727 — *TUI as thin HTTP client of mika-spirit*
- **Vol** : V4-a (premier des trois vols de V4 ; le ticket se ferme au merge de V4-c, et seulement là)
- **Branche** : `feat/1727/v4a-spirit-sert-les-runs-d-equipe`
- **Base mesurée** : `main` @ `77a7cfb0`
- **Label** : `loop-substrate` (plafond de tours : 200)
- **Borne du groom** : 320 lignes ajoutées, posée par Vincent pour ce vol (corps du ticket,
  § *Borne de ce vol*). **Halte du pilote** : 450 lignes ajoutées.
- **Unité de toutes les bornes** : la ligne **ajoutée**, colonne 1 de
  `git diff --numstat origin/main...HEAD -- . ':!docs/plans' | awk '{a+=$1} END{print a}'` —
  code, commentaires, doc et tests ; les retraits ne comptent pas.
- **La PR porte `Refs mika#1727`**, jamais `Closes`, `Fixes` ni `Resolves` — ni dans le corps,
  ni dans un commit.

---

## 0. HALTE BORNE DU GROOM — 518 lignes estimées pour 320

**Ce plan s'arrête, et il ne recoupe pas.**

Le chiffrage poste par poste du § *Taille estimée* donne **518 lignes ajoutées**. La borne du
groom est **320**, posée par Vincent dans le corps du ticket :

> **Si ce total dépasse `320`, le groom s'arrête et le dit. Il ne recoupe pas lui-même.**
> Cette borne est posée par Vincent pour ce vol ; le plan cite cette décision.

Trois faits qui vont avec, parce qu'ils changent la nature de la décision à prendre :

1. **Ce n'est pas la borne du groom seule qui est dépassée.** La halte du pilote est à `450`,
   et le seul livrable C2 (le lanceur, la route, le flux et leurs tests) pèse **436** à lui
   seul : un vol conforme à ce périmètre ne peut pas atterrir sans déclencher la halte du
   pilote au deuxième commit. La borne par vol de l'unité de l'architecte — 429 lignes, hors
   `docs/` — est dépassée aussi (**500** dans cette unité). Les trois bornes refusent ce
   périmètre, pas seulement la plus serrée.
2. **L'estimation est conforme aux repères que le ticket donne lui-même**, et non gonflée.
   La tête annonce « environ 320 à 420 » et le détail du § *Taille estimée* se tient dans la
   même famille, poste par poste, avec deux écarts nommés au § 2.6 : le banc de test (206
   contre les 160 à 200 annoncés, pour six tests dont un de déconnexion incrémentale plus le
   lanceur scripté — à comparer aux ≈ 180 lignes de tests de `tasks_stream.rs`, qui n'a ni
   lanceur injecté ni test de coupure), et le lanceur (78 contre 30 à 45, parce que la forme
   choisie au § 2.4 est un trait avec sa valeur de production, et non une closure).
3. **Aucune coupe n'est proposée ici.** Le corps du ticket dit « le plan ne choisit aucune
   coupe » et « une halte ne se lève jamais par un amendement du plan ». Le chiffrage par
   poste ci-dessous est exactement l'information dont Vincent a besoin pour arbitrer ;
   l'arbitrage lui appartient.

**Le reste du plan est écrit en entier et reste exploitable tel quel** — formes de trames,
seam du lanceur, ordre de validation, six tests, trois mutations, ligne de doc, ordre de
commit. Si la borne est levée ou le périmètre recoupé par Vincent, le pilote n'a pas à
re-groomer : il lit le § 3 à partir du livrable que Vincent retient. Rien dans ce plan ne
doit être lu comme une autorisation de démarrer malgré la halte.

---

## 1. Ce qui est mesuré sur main (@ `77a7cfb0`)

Les mesures du corps du ticket ont été rejouées site par site. Elles concordent toutes ; ce
§ ajoute ce que la tête ne dit pas et dont les décisions du § 2 dépendent.

### `crates/mika-agent/src/teams/types.rs`

| site | ligne | mesure |
|---|---|---|
| `enum TeamEvent` | 279 | `#[derive(Clone, Debug)]` — **neuf** variantes |
| formes des variantes | 281-291 | trois **newtype** portant une `String` (`Progress`, `Deliverable`, `RunFailed`), six variantes **struct** |
| `TeamEventCallback` | ~295 | `pub type TeamEventCallback = Box<dyn Fn(TeamEvent) + Send + Sync>;` — rappel **synchrone**, ni `async`, ni `FnMut` |
| `TeamPhase` | 255-262 | `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`, **sans** `rename_all` — se sérialise `"Decompose"`, `"Execute"`, `"Review"`, `"Deliver"`, `"ReDecompose"` |
| `TeamRun`, `RunStatus`, `TaskAssignment`, `TaskStatus` | — | déjà `Serialize, Deserialize` |
| `mod tests` | présent | tests nommés en français, p. ex. `mika1940_disposition_covers_every_variant` — le précédent de forme du test d'exhaustivité |

### `crates/mika-agent/src/teams/mod.rs` (intact dans ce vol)

`run_team` (l. 28) prend **douze** arguments : `team_name`, `goal`, `global_home`, `settings`,
`callback: Option<TeamEventCallback>`, `team_db: AsyncDatabase`, `reference_run_id`,
`github_app`, `pr_reviews_posted`, `tier`, `deployment` — et rend `Result<TeamRun>`. Les deux
derniers portent dans leur commentaire la doctrine qui décide du § 2.4 : *« supplied by the
caller from its own cached authority (mika#1962) rather than re-read from the environment
here »*, et son frère mika#2290 pour le déploiement.

### `crates/mika-agent/src/tools/run_team.rs` (intact dans ce vol) — la référence des arguments

L'outil ouvre **sa propre** base (`home::container_db_path` → `Database::open` →
`AsyncDatabase::new`), appelle `validate_team_name`, puis (l. 128-139) :

```rust
let result = crate::teams::run_team(
    team_name, goal, &self.home_dir, &self.settings, callback,
    team_db.clone(), None, self.github_app.clone(),
    None,           // run_team tool: no AppState access for session-scoped dedup (#821)
    ctx.tier,       // mika#1962 — cached at agent init, never re-read here
    ctx.deployment, // mika#2290 — same, for the hosting ground truth
).await;
team_db.shutdown();
```

### `crates/mika-agent/src/server/state.rs`

| site | ligne | mesure |
|---|---|---|
| `AgentState.tier` | 98 | `mika_common::home::AgentTier` — **le tier est par agent, pas par serveur** |
| `AgentState.deployment` | 105 | `mika_common::home::Deployment`, idem |
| `struct AppState` | 166-235 | ~27 champs ; **ni `tier` ni `deployment`** ; porte `global_home_dir`, `settings`, `github_app`, `pr_reviews_posted` |
| dernier champ | ~234 | `pub task_events_channel: Arc<super::tasks_stream::TaskEventsChannel>,` |
| `AppState::resolve_agent` | 246 | `async fn resolve_agent(&self, name: &str) -> Option<Arc<AgentState>>` — une chaîne vide désigne l'agent par défaut ; construction paresseuse sur disque (#1399) |
| `impl Debug for AppState` | 353 | **manuel** — un champ neuf y coûte une ligne ou zéro, selon qu'on l'y nomme |

### `crates/mika-agent/src/server/mod.rs`

| site | ligne | mesure |
|---|---|---|
| littéral `AppState { … }` de production | 1948 | le seul ; `task_events_channel` ~1977 |
| littéral `AppState { … }` de test | 2518 | **le seul autre** ; `task_events_channel: Arc::new(tasks_stream::TaskEventsChannel::new()),` à 2544 |
| `fn test_settings` | 2322 | privé à `mod tests` |
| `fn test_state` | 2363 | privé ; délègue à `test_state_with_settings(test_settings())` |
| `fn test_app` | 2611 | privé ; rend le `Router` servi |
| `fn test_state_with_dashboard_token` | 4788 | privé ; jeton `"dashboard-token-secret"` |
| `pub(super) fn mika1727_state_compact` | 6548 | **le précédent** : une aide de `mod tests` exposée aux modules frères, consommée par `server/a2a.rs:2976` |
| groupes au jeton interne seul | 371, 397, 411 | `rewind_routes`, `session_routes` (V2d-1a), `operator_v1_routes` — chacun clos par `.route_layer(middleware::from_fn_with_state(state.clone(), auth::require_internal_token))` |
| fusion | 453-458 | `.nest("/api/v1", …)` puis `.merge(rewind_routes).merge(session_routes).merge(operator_v1_routes)` |
| routes d'équipe existantes | 219-230 | `/team-runs*`, **lecture seule**, au jeton du tableau de bord ou interne |

**Deux littéraux seulement** : un champ neuf sur `AppState` coûte deux lignes de
construction, pas quatorze.

### `crates/mika-agent/src/server/tasks_stream.rs` — le flux de référence (475 lignes, ≈ 180 de tests dès 294)

`#[serde(tag = "event", rename_all = "snake_case")] enum TaskEventFrame` ;
`const CHANNEL_CAP: usize = 256;` ; `PREVIEW_CAP_CHARS = 500` + `truncate_preview` ;
`TaskEventsChannel` sur `broadcast::Sender` ; handler sur `BroadcastStream` +
`Sse::new(stream).keep_alive(KeepAlive::default())`. **Divergence structurelle avec ce vol** :
c'est un canal **diffusé** par processus ; la route de V4-a sert **un** run, donc un canal
par requête — rien de `tasks_stream.rs` n'est réutilisable, seul son idiome l'est.

### `crates/mika-agent/src/server/sessions.rs` (251 lignes) et `compact.rs` (137 lignes)

Le gabarit d'un module de route POST au jeton interne : doc de module qui dit **pourquoi**,
types de requête et de réponse dérivés, un handler, des aides de réponse à un site par
opération. **Aucun des deux ne porte de tests** : ceux de V2d vivent dans le `mod tests` de
`server/mod.rs`. La tête de V4-a demande l'inverse (« un module neuf […] pour la route, son
flux et **ses tests** »), d'où la décision 2.7.

### `crates/mika-common/src/team.rs` et `crates/mika-agent/Cargo.toml`

`validate_team_name(name) -> Result<()>` (l. 56 : vide, > 32 caractères, puis le jeu de
caractères) et `team_exists(home_dir, name) -> bool` (l. 89 : `teams/<name>/team.toml`
existe). Côté dépendances, tout est déjà là : `async-trait`, `tokio-stream`, `tower` (l. 89),
`http-body-util` (l. 90).

### `crates/mika-agent/docs/architecture.md` — le catalogue des flux

§ **14.2 SSE Frame Catalog** (l. 885) : un tableau de **trois** surfaces —
`mika_a2a::streaming::StreamEvent` (`tag = "kind"`, par tâche),
`PermissionStreamFrame` et `TaskEventFrame` (`tag = "event"`, par processus) —, puis
« Five axes of deliberate divergence » (l'axe 1 **est** la graphie du discriminant : `kind`
pour le protocole, `event` pour le serveur) et « Shared discipline (all three surfaces) »,
qui demande un `#[serde(other)]` `Unknown`, la discipline de consommateur lent et la
troncature UTF-8 à 500 caractères.

---

## 2. Décisions

### 2.1 — `TeamEvent` se tague **adjacemment** : `tag = "kind"`, `content = "data"`

```rust
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum TeamEvent { … }
```

**Pourquoi pas le tagging interne des trois surfaces du catalogue.** Trois variantes sur neuf
sont des newtype portant une `String`. Le tagging interne (`tag = "…"` seul) exige que le
contenu d'une newtype se sérialise en **map** ; une `String` n'en est pas une, et serde
échoue **à l'exécution**, pas à la compilation. Les convertir en variantes struct
(`Progress { message: String }`) réécrirait chaque site d'émission de
`crates/mika-agent/src/teams/engine.rs` et chaque `match` de `crates/mika-cli/` — tous deux
hors périmètre, et le critère 9 l'interdit nommément. Le tagging adjacent accepte les neuf
formes sans toucher un seul site.

**Pourquoi `kind` et non `event`.** C'est l'axe 1 du catalogue, appliqué : `kind` est la
graphie du **domaine** (`mika_a2a::streaming::StreamEvent`), `event` celle des **surfaces du
serveur**. `TeamEvent` est un type du domaine des équipes, émis par le moteur bien avant
qu'une route existe ; l'enveloppe de la route (§ 2.3) prend `event`. Le bénéfice est
immédiat et pas seulement doctrinal : la trame finale d'erreur et `TeamEvent::RunFailed`
auraient porté le même mot à deux étages d'un même objet JSON.

**Les neuf formes, exactement :**

| variante | JSON |
|---|---|
| `Progress(String)` | `{"kind":"progress","data":"…"}` |
| `PhaseChanged { phase, iteration }` | `{"kind":"phase_changed","data":{"phase":"Execute","iteration":1}}` |
| `AgentStarted { agent, role }` | `{"kind":"agent_started","data":{"agent":"…","role":"…"}}` |
| `TasksAssigned { tasks, iteration }` | `{"kind":"tasks_assigned","data":{"tasks":[…],"iteration":1}}` |
| `AgentCompleted { agent, response }` | `{"kind":"agent_completed","data":{"agent":"…","response":"…"}}` |
| `AgentFailed { agent, error }` | `{"kind":"agent_failed","data":{"agent":"…","error":"…"}}` |
| `CriticReview { approved, feedback, iteration }` | `{"kind":"critic_review","data":{"approved":true,"feedback":"…","iteration":1}}` |
| `Deliverable(String)` | `{"kind":"deliverable","data":"…"}` |
| `RunFailed(String)` | `{"kind":"run_failed","data":"…"}` |

`"phase"` porte la graphie **PascalCase** de `TeamPhase`, qui n'a pas de `rename_all` et n'en
reçoit pas ici : lui en donner un changerait la forme du checkpoint que
`deserialize_checkpoint` relit, c'est-à-dire la logique des équipes. Hors périmètre, et c'est
la raison pour laquelle le `rename_all = "snake_case"` de `TeamEvent` ne descend pas dans
`data`. Le tableau de doc du § 3 C3 le dit, parce qu'un lecteur du catalogue le supposera.

### 2.2 — **Pas** de `#[serde(other)] Unknown` sur `TeamEvent`, et la doc dit l'écart

La discipline partagée du § 14.2 demande un catch-all. Il n'est pas ajouté, pour deux raisons
qui ne se recouvrent pas :

- Une dixième variante rend non exhaustifs les `match` sur `TeamEvent` de
  `teams/engine.rs` et de `crates/mika-cli/` — hors périmètre, critère 9. L'écart n'est donc
  pas un choix de confort : la discipline du catalogue n'est pas atteignable dans ce vol.
- `#[serde(other)]` ne sert que l'étage **désérialisation**, et le seul désérialiseur de ce
  flux dans ce vol est un test. Le client arrive en V4-b : c'est le vol qui aura un
  consommateur sous la main qui pourra décider ce que tolérer un `kind` inconnu veut dire
  pour lui.

Ni `OverflowMarker`, ni troncature à 500 caractères, pour la même raison de nature : voir
2.5 et 2.6.

### 2.3 — L'enveloppe du flux : `TeamRunFrame`, trois trames, `tag = "event"`

Dans le module neuf :

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TeamRunFrame {
    TeamEvent { payload: crate::teams::types::TeamEvent },
    RunCompleted { run: crate::teams::types::TeamRun },
    RunError { error: String },
}
```

| trame | JSON | quand |
|---|---|---|
| `team_event` | `{"event":"team_event","payload":{"kind":"progress","data":"…"}}` | une par `TeamEvent`, dans l'ordre d'émission |
| `run_completed` | `{"event":"run_completed","run":{…TeamRun…}}` | **trame finale**, le run a rendu `Ok` |
| `run_error` | `{"event":"run_error","error":"<message de l'erreur>"}` | **trame finale**, le run a rendu `Err` |

Exactement **une** trame finale par flux, puis le flux se ferme. `run_error` et non
`run_failed` : la collision de vocabulaire avec `TeamEvent::RunFailed` serait réelle — le
moteur peut émettre l'événement `run_failed` **et** le run rendre `Ok`, ou l'inverse. Deux
mots pour deux faits.

### 2.4 — Le lanceur : un trait sur `AppState`, et `tier`/`deployment` arrivent par argument

```rust
pub struct TeamLaunchRequest {
    pub team_name: String,
    pub goal: String,
    pub reference_run_id: Option<String>,
    pub tier: mika_common::home::AgentTier,
    pub deployment: mika_common::home::Deployment,
}

#[async_trait::async_trait]
pub trait TeamLauncher: Send + Sync {
    async fn launch(
        &self,
        req: TeamLaunchRequest,
        events: crate::teams::types::TeamEventCallback,
    ) -> anyhow::Result<crate::teams::types::TeamRun>;
}
```

Champ : `pub team_launcher: Arc<dyn TeamLauncher>,` sur `AppState`, nommé dans son `Debug`
manuel par un littéral (`"<dyn TeamLauncher>"`) — un trait objet n'est pas `Debug`.

**Un trait, pas une closure.** Le lanceur scripté des tests porte de l'état (le scénario
d'événements, le compte de lancements, l'attestation d'achèvement, le mode d'erreur) ; un
trait le porte dans sa struct, là où un `Arc<dyn Fn(…) -> BoxFuture>` l'enfermerait dans un
`Arc<Mutex<…>>` capturé. Et c'est l'idiome du dépôt pour l'injection de déterminisme
(`MessageSender`, `LlmProvider`).

**`tier` et `deployment` voyagent dans la requête, et c'est la décision.** Ils sont portés par
`AgentState` (l. 98, l. 105), pas par `AppState` : il n'existe pas de tier « du serveur ». Les
figer dans le lanceur de production au démarrage mentirait sur un agent résolu
paresseusement plus tard (#1399). Le lanceur étant un **champ** de l'`AppState`, il ne peut pas
appeler `resolve_agent` lui-même. C'est donc la route qui résout l'agent par défaut
(`resolve_agent("")`) et transmet son tier et son déploiement — exactement la doctrine
inscrite sur `run_team` : *l'appelant fournit son autorité cachée* (mika#1962, mika#2290).

**La valeur de production, `SpiritTeamLauncher`**, porte `global_home_dir: PathBuf`,
`settings: Settings`, `github_app: Option<Arc<GitHubApp>>`, et son `launch` reproduit l'appel
de l'outil, argument pour argument :

- ouvre **sa propre** base (`container_db_path` → `Database::open` → `AsyncDatabase::new`) et
  la `shutdown()` après l'appel, comme `tools/run_team.rs` — un run d'équipe est long, et le
  faire passer par la poignée de l'agent servi lui ferait partager son canal `mpsc(512)` ;
- passe `None` pour `reference_run_id` **non** : il passe `req.reference_run_id.as_deref()`,
  seul argument que l'outil fixe à `None` et que le corps de la requête HTTP porte (c'est au
  périmètre du vol) ;
- passe `None` pour `pr_reviews_posted`, **comme l'outil**, bien que l'`AppState` en porte un.
  Le critère 6 ancre la valeur de production sur l'argumentaire de l'outil ; brancher la
  dédup de revues de PR (#821) sur ce chemin serait un changement de la logique des équipes,
  non testé ici et hors périmètre. Le commentaire du champ le dit, pour que le prochain
  lecteur n'y voie pas un oubli.

### 2.5 — Le détachement : une tâche propriétaire du `tx`, et trois `let _ =` qui portent le contrat

Le handler, dans cet ordre :

1. `validate_team_name(&name)` → `Err` donne **400** `{"error":"<message de validate>"}` ;
2. `team::team_exists(&state.global_home_dir, &name)` faux → **404**
   `{"error":"team '<name>' not found"}` ;
3. `state.resolve_agent("").await` → `None` donne **500**
   `{"error":"default agent unavailable"}`. Inatteignable en service nominal ; c'est le prix
   de 2.4, et un `unwrap` ici ferait tomber le serveur sur une route ouverte ;
4. `let (tx, rx) = tokio::sync::mpsc::channel::<TeamRunFrame>(CHANNEL_CAP);` avec
   `const CHANNEL_CAP: usize = 256;` — la valeur de `tasks_stream.rs`, pour que les deux
   surfaces ne divergent pas sur un chiffre que rien ne distingue ;
5. le rappel : `Box::new(move |ev| { let _ = tx.try_send(TeamRunFrame::TeamEvent { payload: ev }); })`.
   **`try_send` et jamais `blocking_send`** : `TeamEventCallback` est synchrone et le moteur
   l'appelle depuis un contexte async — un envoi bloquant y bloquerait un worker tokio. La
   saturation perd la trame et le run continue ;
6. `tokio::spawn(async move { let r = launcher.launch(req, callback).await; let _ = tx_final.send(frame_finale(r)).await; })`.
   La tâche **possède** le `tx` ; la réponse ne possède que le `rx`. Rien ne relie le run au
   futur de la requête : c'est par là que le run survit au client, pas par une garde ;
7. `Sse::new(ReceiverStream::new(rx).map(…)).keep_alive(KeepAlive::default())`.

**Les trois `let _ =` sont le contrat, pas de la négligence.** Client parti ⇒ `rx` dropé ⇒
`try_send` et `send` rendent `Err`. Les ignorer **est** la survie du run ; les remonter par
`?` ou par un `return` est précisément la mutation 2 du § 3.5. Le commentaire à ce site le
dit, parce que le geste « propre » qu'un relecteur proposera est celui qui casse le critère 5.

**Pas d'`OverflowMarker`.** Un `mpsc` borné n'expose aucun compte de perte, contrairement au
`broadcast::error::RecvError::Lagged` dont `tasks_stream.rs` tire le sien. Fabriquer un
marqueur sans compte annoncerait une mesure qui n'existe pas, et ce vol n'a pas de lecteur à
qui la servir. La trame est perdue en silence et la doc le dit — un consommateur lent est
V4-b.

**Pas de troncature à 500 caractères.** Les champs de `TeamEvent` ne sont pas des aperçus :
`Deliverable` **est** le livrable, `AgentCompleted.response` **est** la réponse. Les tronquer
livrerait un flux inexploitable par le client de V4-b. La divergence est nommée dans la doc,
avec sa raison.

### 2.6 — Les deux écarts de chiffrage avec la tête du ticket

La tête annonce 30 à 45 lignes pour le lanceur et 160 à 200 pour le banc. Le chiffrage donne
**78** et **206**. Les deux écarts viennent de décisions, et aucune n'est gratuite :

- **le lanceur** : la forme 2.4 est un trait, sa requête, sa valeur de production et la
  reproduction de l'appel à douze arguments avec ses trois commentaires de décision. Une
  closure aurait tenu dans la fourchette et rendu le lanceur scripté plus coûteux du même
  montant, déplacé dans le banc ;
- **le banc** : six tests dont un lit le corps **incrémentalement** par
  `http_body_util::BodyExt::frame()` pour se déconnecter au milieu — `oneshot` + `to_bytes`
  ne sait pas le faire —, plus un lanceur scripté avec attestation d'achèvement. Le repère
  que le ticket donne lui-même (≈ 180 lignes pour les tests de `tasks_stream.rs`) porte un
  flux sans lanceur injecté et sans test de coupure.

### 2.7 — Les tests vivent dans le module neuf, et trois aides de `mod tests` s'ouvrent

La tête demande « un module neuf […] pour la route, son flux et **ses tests** », contre
l'usage de `sessions.rs` et `compact.rs`, dont les tests sont dans `server/mod.rs`. Le module
neuf les porte donc, et consomme les aides existantes par le précédent mesuré au § 1 :
`pub(super) fn mika1727_state_compact`, consommée par `server/a2a.rs:2976`. `test_state`,
`test_state_with_dashboard_token` et `test_app` passent de privées à `pub(super)` — trois
mots, aucun corps modifié, aucun appelant existant touché.

Le lanceur scripté s'installe en **mutant le champ** sur l'état rendu
(`state.team_launcher = Arc::new(scripte.clone());`), pas en construisant un `AppState` à la
main : le littéral de test de `server/mod.rs` garde la valeur de production, qui ne lance
rien faute d'équipe sur le disque.

---

## Fire-Disposition

**Gate N/A : ce vol ne livre aucun détecteur.** Ses livrables sont une dérive `serde`, un
seam d'injection, une route, six tests de comportement contre la route servie, et une ligne
de doc. Aucun scan de source, aucune garde lexicale, aucun allowlist — donc rien dont il
faudrait déclarer la population, le témoin et le contre-témoin. La section n'est pas inventée
pour la forme : la règle dit de ne pas la fabriquer en l'absence de détecteur.

Ce que les six tests tiennent, ils le tiennent par comportement, et les trois mutations du
§ 3.5 sont ce qui atteste qu'ils ne sont pas verts par construction.

---

## 3. Ordre de commit

Trois commits, dans cet ordre. Chacun compile, chacun passe ses tests, chacun arrive avec
les siens — jamais reportés.

> **Rappel, et il est opposable.** Le § 0 déclare la halte de la borne du groom. Ce § décrit
> l'exécution si et seulement si Vincent lève la borne ou recoupe le périmètre. Le pilote qui
> trouve ce plan tel quel ne démarre pas : il s'arrête et le dit.

### C1 — `serde` sur `TeamEvent` et son aller-retour (critère 1)

`crates/mika-agent/src/teams/types.rs` seul.

- la dérive étendue et l'attribut adjacent du § 2.1 ;
- le commentaire qui porte les deux décisions : pourquoi adjacent (les trois newtype à
  `String`, et les fichiers hors périmètre qu'un tagging interne exigerait), pourquoi pas
  d'`Unknown` ;
- **un** test dans le `mod tests` du fichier,
  `mika1727_v4a_chaque_variante_de_team_event_fait_laller_retour` : les neuf valeurs
  construites, `to_string` → `from_str` → re-`to_value`, égalité des deux valeurs JSON (et
  non `PartialEq` sur `TeamEvent`, qui serait une dérive de plus sur un type du domaine pour
  l'usage d'un seul test) ; puis un `match` **exhaustif** sur chaque valeur qui rend son
  `kind` attendu, de sorte qu'une dixième variante ne compile pas sans passer par ce test.
  C'est la forme de `mika1940_disposition_covers_every_variant`, déjà dans le fichier.

Vert : `cargo test -p mika-agent teams::types`.

### C2 — le lanceur, la route, le flux et leurs tests (critères 2 à 7, 9, 10)

`state.rs`, `server/mod.rs`, et le module neuf `server/team_runs.rs`.

- `state.rs` : `TeamLaunchRequest`, `trait TeamLauncher`, `SpiritTeamLauncher` et son
  `launch`, le champ `team_launcher` et sa ligne de `Debug` ;
- `server/mod.rs` : `mod team_runs;`, le champ aux **deux** littéraux (1948 et 2518), le
  `pub(super)` sur les trois aides de test, et le routeur ;
- `server/team_runs.rs` : la doc de module, `CreateTeamRunRequest`
  (`{"goal": "...", "reference_run_id": "..."}`, le second `#[serde(default)]`),
  `TeamRunFrame`, `CHANNEL_CAP`, le handler, les trois aides de réponse, le lanceur scripté,
  les six tests.

**Le routeur.** Un groupe neuf, sur le modèle de `session_routes` (l. 397) :

```rust
let team_run_routes = Router::new()
    .route("/teams/{name}/runs", post(team_runs::handle_create_team_run))
    .route_layer(middleware::from_fn_with_state(
        state.clone(),
        auth::require_internal_token,
    ));
```

puis `.merge(team_run_routes)` dans le `.nest("/api/v1", …)` de la l. 453. **Un groupe à lui
seul, et non une route ajoutée à `operator_v1_routes`** : la clôture par `route_layer` est ce
qui porte le critère 2, et la mutation 3 du § 3.5 consiste exactement à remplacer
`require_internal_token` par `require_dashboard_or_internal_token` ici.

**Les six tests** (noms français, idiome `tower::ServiceExt::oneshot` sur `test_app(state)`,
jeton interne `"test-token-secret"`, jeton du tableau de bord `"dashboard-token-secret"` via
`test_state_with_dashboard_token()`) :

| test | ce qu'il montre | critère |
|---|---|---|
| `mika1727_v4a_sans_jeton_la_route_rend_401_et_ne_lance_rien` | 401, et le compte de lancements du lanceur scripté vaut 0 | 2 |
| `mika1727_v4a_le_jeton_du_tableau_de_bord_rend_401_et_ne_lance_rien` | 401 sur l'état à deux jetons, compte à 0 | 2 |
| `mika1727_v4a_un_nom_invalide_rend_400_sans_lancer` | 400, compte à 0 | 3 |
| `mika1727_v4a_une_equipe_absente_rend_404_sans_lancer` | 404, compte à 0 | 3 |
| `mika1727_v4a_les_trames_arrivent_dans_lordre_puis_la_finale` | le corps entier, découpé en lignes `data:` : trois `team_event` dans l'ordre d'émission du scénario, puis `run_completed` portant le `TeamRun` rendu, et plus rien | 4 |
| `mika1727_v4a_un_run_en_erreur_donne_une_trame_derreur_et_ferme` | `run_error` porteur du message, le corps se termine (le `to_bytes` rend) | 4 |
| `mika1727_v4a_la_coupure_du_client_ne_coupe_pas_le_run` | une trame lue par `BodyExt::frame()`, `drop(resp)`, puis attente bornée de l'attestation d'achèvement du lanceur | 5 |

Soit **sept** fonctions pour les six tests du ticket : le critère 2 en porte deux, parce que
« sans jeton » et « avec le jeton du tableau de bord » n'empruntent pas le même chemin dans
`auth.rs` (absence d'en-tête contre comparaison en temps constant d'un jeton présent). Les
fondre perdrait la seconde moitié au premier `assert` rouge.

Le critère 6 — *la valeur de production appelle `teams::run_team` avec les arguments de
l'outil* — **n'est pas couvert par un test**, et ce plan le dit plutôt que de le prétendre :
l'appeler exigerait une équipe sur le disque et un fournisseur réel, que le ticket interdit.
Il se tient par revue sur l'appel du § 2.4, ligne à ligne contre `tools/run_team.rs:128-139`,
et le commentaire à ce site cite les deux lignes de doctrine (#821, mika#1962, mika#2290)
pour qu'une édition future ne diverge pas en silence.

Vert : `cargo build -p mika-agent --bin mika-spirit`, puis `cargo test -p mika-agent`, puis
`cargo clippy --all-targets -p mika-agent`. **Aucun `#[allow]` neuf, aucun `#[ignore]`**
(critère 10) : le seul candidat serait `clippy::too_many_arguments` sur l'appel à `run_team`,
et il n'est pas nécessaire — le lint porte sur les **définitions**, et le site d'appel est
dans `launch`, qui prend deux paramètres.

### C3 — la doc (critère 8)

`crates/mika-agent/docs/architecture.md`, § 14.2, et rien d'autre.

- une **quatrième ligne** au tableau :
  `TeamRunFrame` (mika#1727 V4-a) | `#[serde(tag = "event")]` ; `TeamEvent` imbriqué sous
  `payload` en `#[serde(tag = "kind", content = "data")]` | **par run** (un canal `mpsc` par
  requête, aucun état partagé) | `POST /api/v1/teams/{name}/runs` — **jeton interne seul** |
  `mika-agent` server | mika#1727 V4-a ;
- la phrase du critère 8 : la route, son jeton, et **que le run survit au client** — il tourne
  dans sa propre tâche, et une déconnexion ne l'arrête pas ;
- les **trois écarts** à la discipline partagée, avec leur raison en une ligne chacun : pas
  d'`Unknown` (2.2), pas d'`OverflowMarker` (`mpsc` n'expose pas de compte de perte, 2.5), pas
  de troncature à 500 caractères (les champs sont le livrable, pas un aperçu, 2.5). Le
  tableau du § 2.1 note aussi que `data.phase` reste en PascalCase, parce que `TeamPhase` sert
  le checkpoint.

**Ce fichier est la copie locale au crate.** `scripts/sync-agent-docs.sh` synchronise depuis
`docs/`, et le job `docs-sync` de CI échoue sur une PR qui modifie `docs/` sans l'avoir
lancé — ce vol ne touche pas `docs/`, le sens est donc l'inverse et le job n'a rien à dire.
Le pilote qui le trouve rouge s'arrête et le dit, il ne lance pas le script (il écrirait dans
`docs/`, hors des fichiers autorisés).

### Fin de pipeline

Dans cet ordre, avant d'ouvrir la PR : `cargo fmt`, `cargo build -p mika-agent --bin
mika-spirit`, `cargo test -p mika-agent`, `cargo clippy --all-targets -p mika-agent`, puis
`bash scripts/verify-pipeline.sh origin/main`. **Rouge ⇒ arrêt et déclaration** (critère 11).

`git diff --name-only origin/main...HEAD` ne doit rendre que : `docs/plans/<ce plan>`,
`crates/mika-agent/src/teams/types.rs`, `crates/mika-agent/src/server/team_runs.rs`,
`crates/mika-agent/src/server/mod.rs`, `crates/mika-agent/src/server/state.rs`,
`crates/mika-agent/docs/architecture.md`. Toute autre entrée est un écart au critère 9 :
arrêt, pas d'élargissement.

### 3.5 — Les trois mutations (critère 7)

Chacune **seule**, sur un commit vert, dans un worktree **détaché**, avec sa sortie dans le
corps de la PR.

| mutation | geste | test qui doit rougir |
|---|---|---|
| M1 — retirer l'émission d'un événement | dans le rappel du handler, sauter une variante avant l'envoi (p. ex. un `if matches!(ev, TeamEvent::Progress(_)) { return; }`) | `…les_trames_arrivent_dans_lordre_puis_la_finale` |
| M2 — faire suivre la coupure du client par l'arrêt du run | remonter l'échec d'envoi dans la tâche détachée : le `let _ = tx_final.send(…)` devient un `return` sur `Err`, et le rappel cesse d'ignorer le sien | `…la_coupure_du_client_ne_coupe_pas_le_run` |
| M3 — ouvrir la route au jeton du tableau de bord | dans le groupe `team_run_routes`, `require_internal_token` → `require_dashboard_or_internal_token` | `…le_jeton_du_tableau_de_bord_rend_401_et_ne_lance_rien` |

Pour chacune : la mutation, la sortie de `cargo test -p mika-agent <nom du test>` rouge, puis
le retour à l'état vert avant la suivante. Aucune mutation n'est commitée sur la branche.

### Halte du pilote sur la borne

Après **chaque** commit :

```
git diff --numstat origin/main...HEAD -- . ':!docs/plans' | awk '{a+=$1} END{print a}'
```

**Au-delà de `450`, arrêt** : pas de commit suivant, pas de push, pas de PR, et un dernier
message « HALTE borne : N lignes ajoutées pour `450` » avec le compte après chaque commit.
Au chiffrage de ce plan, **C1 + C2 vaut déjà ≈ 500** : la halte tombe au deuxième commit. Ce
n'est pas une prédiction à vérifier en route, c'est la raison du § 0.

### Si le code s'écarte de ce plan

Le pilote amende ce plan sur la branche, dans un commit `docs(plans)`, **avant** de pousser
le code qui s'en écarte, en nommant le point et la raison. Un amendement ne lève **jamais**
une halte, et n'introduit aucune coupe : les deux demandent le mot de Vincent.

Ce vol **n'ouvre aucun ticket**, ni suivi ni sous-ticket (gel jusqu'au 01/12). Un manque ou un
reste va en commentaire sur mika#1727 ou dans la PR.

---

## 4. Definition of Done

- `TeamEvent` dérive `Serialize` et `Deserialize` sous `#[serde(tag = "kind", content = "data", rename_all = "snake_case")]`, et les neuf variantes ont la forme du tableau du § 2.1.
- Un test du `mod tests` de `teams/types.rs` fait l'aller-retour des neuf variantes et porte un `match` exhaustif, de sorte qu'une variante neuve ne compile pas sans y passer.
- `POST /api/v1/teams/{name}/runs` est servie dans un groupe de routeur clos par `auth::require_internal_token`, et par ce groupe seul.
- Le corps de la requête est `{"goal": "...", "reference_run_id": "..."}`, le second optionnel.
- La route valide par `validate_team_name` (400), vérifie par `team_exists` (404), puis lance par `state.team_launcher` — dans cet ordre, et aucune des deux issues d'erreur ne lance de run.
- La réponse est un `Sse` sur un `mpsc` borné à 256 : une trame `team_event` par `TeamEvent` dans l'ordre d'émission, puis exactement une trame finale `run_completed` ou `run_error`, puis fermeture.
- Le run tourne dans un `tokio::spawn` propriétaire du `tx` ; les trois échecs d'envoi sont ignorés, et un commentaire au site dit que c'est le contrat.
- `AppState` porte `team_launcher: Arc<dyn TeamLauncher>`, nommé dans son `Debug` manuel, posé aux deux littéraux de construction.
- `SpiritTeamLauncher::launch` ouvre sa base, appelle `teams::run_team` avec les arguments de `tools/run_team.rs:128-139` — `None` pour `pr_reviews_posted`, `tier` et `deployment` reçus par argument —, puis `shutdown()`.
- Les sept fonctions de test du § 3 C2 sont vertes dans `server/team_runs.rs`, sans fournisseur réel et sans équipe sur le disque pour les chemins d'erreur.
- Les trois mutations du § 3.5 ont été faites chacune seule dans un worktree détaché, et la PR porte la sortie rouge de chacune.
- `architecture.md` § 14.2 porte la quatrième ligne du tableau, la route, son jeton, la survie du run au client, et les trois écarts motivés à la discipline partagée.
- `cargo build -p mika-agent --bin mika-spirit`, `cargo test -p mika-agent`, `cargo clippy --all-targets -p mika-agent` et `bash scripts/verify-pipeline.sh origin/main` sont verts ; aucun `#[allow]` ni `#[ignore]` neuf.
- `git diff --name-only origin/main...HEAD` ne rend que les six chemins du § *Fin de pipeline* ; `crates/mika-cli/`, `teams/mod.rs`, `tools/run_team.rs` et `crates/mika-agent/CLAUDE.md` sont intacts.
- La PR porte `Refs mika#1727` et aucun `Closes`, `Fixes` ni `Resolves`, y compris si le gabarit de reprise l'ouvre.

## 5. Acceptance criteria

Les douze critères de V4-a, transcrits du corps du ticket.

1. `TeamEvent` est sérialisable, et chaque variante fait l'aller-retour `serde` sans perte.
2. `POST /api/v1/teams/{name}/runs` est servie sous le jeton interne seul. Sans jeton, ou avec le jeton du tableau de bord, elle rend 401 et ne lance rien.
3. Un nom invalide rend 400, une équipe absente 404, et aucun des deux ne lance de run.
4. La réponse est un flux SSE : une trame par `TeamEvent`, dans l'ordre d'émission, puis une trame finale qui porte le `TeamRun` sérialisé ou l'erreur du run.
5. Le run se détache de la requête : un client qui se déconnecte ne l'arrête pas, et un test le montre.
6. La route appelle le lanceur porté par `AppState`. Sa valeur de production appelle `teams::run_team` avec les arguments de l'outil `run_team` de spirit, tier et déploiement compris.
7. Les trois mutations du périmètre rougissent chacune un test. La PR donne la sortie de chaque mutation, faite seule dans un worktree détaché.
8. `crates/mika-agent/docs/architecture.md` décrit la trame dans le catalogue des flux, avec la route, son jeton, et la survie du run au client.
9. `crates/mika-cli/` ne change pas, ni `teams/mod.rs`, ni `tools/run_team.rs`, ni `crates/mika-agent/CLAUDE.md`. `git diff origin/main...HEAD` ne touche que les fichiers listés plus haut.
10. Aucune attente ni aucun `allow` neuf.
11. `cargo build -p mika-agent --bin mika-spirit` d'abord, puis `cargo test -p mika-agent` est vert, et `cargo clippy --all-targets -p mika-agent` aussi. Le pilote lance `bash scripts/verify-pipeline.sh origin/main` avant d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
12. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si le gabarit de reprise l'ouvre.

## Taille estimée

| livrable | lignes estimées |
|---|---|
| `types.rs` — la dérive étendue et l'attribut adjacent (C1) | 2 |
| `types.rs` — le commentaire des deux décisions : adjacent, et pas d'`Unknown` (C1) | 16 |
| `types.rs` — le test d'aller-retour des neuf variantes et son `match` exhaustif (C1) | 46 |
| `state.rs` — `TeamLaunchRequest`, `trait TeamLauncher`, leurs docs (C2) | 30 |
| `state.rs` — `SpiritTeamLauncher` et son `launch` : base, appel à douze arguments, `shutdown`, les trois commentaires de décision (C2) | 40 |
| `state.rs` — le champ `team_launcher`, sa doc, sa ligne de `Debug` manuel (C2) | 8 |
| `server/mod.rs` — `mod team_runs;`, le champ aux deux littéraux, le groupe de routeur et sa fusion, `pub(super)` sur les trois aides de test (C2) | 16 |
| `server/team_runs.rs` — doc de module : pourquoi la route, pourquoi le détachement, pourquoi `kind` dedans et `event` dehors (C2) | 24 |
| `server/team_runs.rs` — `CreateTeamRunRequest`, `TeamRunFrame`, `CHANNEL_CAP`, leurs docs (C2) | 34 |
| `server/team_runs.rs` — le handler : validation, 400/404/500, canal, rappel, tâche détachée, `Sse` (C2) | 62 |
| `server/team_runs.rs` — les trois aides de réponse, un site par opération (C2) | 16 |
| `server/team_runs.rs` — le lanceur scripté et les aides de lecture du flux (C2) | 56 |
| `server/team_runs.rs` — les sept fonctions de test du § 3 C2 (C2) | 150 |
| `architecture.md` — la quatrième ligne du tableau, la phrase du critère 8, les trois écarts motivés (C3) | 18 |

Total estimé : 518 lignes

Plafond de tours : 200 (loop-substrate)

Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Les trois bornes sont dépassées, et le § 0 déclare la halte.** Dans l'unité des bornes de
ce vol (`git diff --numstat`, plan exclu), le total est **518 pour une borne de groom de 320**.
Dans l'unité de l'architecte, qui exclut `docs/`, il vaut **500 pour 429**. Et la halte du
pilote à **450** tombe au deuxième commit, puisque C1 + C2 pèse ≈ 500 à lui seul.

Ce plan ne recoupe pas, ne propose aucune coupe et n'ajoute aucune halte à celles de la tête
du ticket : la borne est posée par Vincent, et c'est à lui de la lever ou de recouper le
périmètre de V4-a. Le reste du plan est écrit en entier pour que l'arbitrage se fasse sur des
postes mesurés, et pour qu'un vol recoupé n'ait pas à être re-groomé depuis zéro.

Tracked in: senara-solutions/mika#1727
