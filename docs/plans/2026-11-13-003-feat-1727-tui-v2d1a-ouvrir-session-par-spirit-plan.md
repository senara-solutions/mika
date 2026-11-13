# Vol V2d-1a — mika-spirit ouvre une session pour la TUI (mika#1727)

**Ticket :** senara-solutions/mika#1727 — `Refs`, jamais `Closes`. Le ticket se ferme au merge de V4.
**Branche :** `feat/1727/tui-v2d1a-ouvrir-session-par-spirit`, neuve depuis `main`.
**Base mesurée :** `81bd4cb0` (merge de V2c-1b, #3081).

---

## 0. Ce qui est mesuré

Tout ce qui suit a été lu sur la branche, à `81bd4cb0`. Les numéros de ligne
sont ceux de cette révision ; ils servent à retrouver le site, pas à être
rejoués tels quels.

| fait | site | ce qu'il décide ici |
|---|---|---|
| `resolve_agent(&str) -> Option<Arc<AgentState>>`, `async`, chaîne vide ⇒ agent par défaut, résolution paresseuse sur disque sinon | `server/state.rs:246-251` | le 404 d'AC4 est **déjà** écrit : c'est le `None` de cette fonction, pas une vérification neuve |
| `AgentState.canonical_session_id: Option<String>`, résolu une fois à `init_agent` | `server/state.rs` (champ), `server/mod.rs:621` | **c'est le discriminant singleton / non singleton**. Aucune lecture d'`identity.toml` dans la route |
| `AsyncDatabase::create_session(id, agent_id, channel_type) -> Result<()>` | `async_db.rs:2018` | l'écriture du cas non singleton |
| `AsyncDatabase::get_or_create_canonical_session(session_id, channel_type) -> Result<String>`, `INSERT OR IGNORE`, `agent_id` pris sur la base elle-même | `async_db.rs:2111` | l'écriture du cas singleton, et **elle rend l'identifiant** |
| `/send` fait déjà exactement ce branchement — et **avale** l'erreur en `warn!` | `server/handlers.rs:1712-1729` | le modèle du branchement, et le point où ce vol s'en écarte (§ 1, D3) |
| `Session { id, agent_id, channel_type, … }`, `get_session(id) -> Result<Option<Session>>` | `db.rs:455-464`, `async_db.rs:3261` | la lecture en base du test d'AC2 |
| `count_sessions(agent_id, channel_type, session_id, task_id, from, to) -> Result<u64>` | `async_db.rs:3239` | le « aucune autre » d'AC3 et le « aucune ligne » d'AC4 |
| les routes rewind vivent dans `rewind_routes`, sous `require_internal_token`, mergé dans le `nest("/api/v1", …)` | `server/mod.rs:369-376`, `420-432` | le modèle d'enregistrement |
| `/api/v1/sessions` **existe déjà en GET** (`dashboard::handle_sessions_list`), sous `require_dashboard_or_internal_token` | `server/mod.rs:191` | le seul vrai risque de ce vol — § 1, D4 |
| `test_state()` pose `canonical_session_id: None` ; `test_state_with_dashboard_token()` ; `test_app(state).oneshot(...)` | `server/mod.rs:2334`, `2509`, `4303` | le harnais. Aucun des deux n'est modifié |
| un test mute un champ d'`AgentState` via `Arc::get_mut(entry.value_mut())` sur la `DashMap` | `server/mod.rs:5649-5655` (`mika2671_b2_state`) | **comment fabriquer un agent singleton en test sans toucher `test_state()`** |
| les tests de routes rewind vivent dans `server/mod.rs § mod tests`, pas dans `rewind.rs` | `server/mod.rs:5555+` | où vont les tests de ce vol, et pourquoi (§ 1, D5) |

Deux faits qui ne sont pas dans la table parce qu'ils sont des **absences**, et
qu'une absence se dit :

- `rewind.rs` ne porte **aucun** `mod tests`. Il n'y a donc pas de précédent de
  test de route colocalisé au module ; le précédent est l'inverse.
- `docs/openapi/mika-spirit.yaml` ne décrit **aucune** des trois routes rewind.
  Ce vol n'y touche pas non plus ; ce n'est pas un oubli, c'est l'état du
  contrat, et `server/openapi.rs` ne vérifie que des routes nommées.

---

## 1. La forme, et les cinq décisions qui la tiennent

Une route, un module, un branchement sur un champ déjà résolu. Ce qui mérite
d'être écrit, ce sont les endroits où la forme évidente est fausse.

### D1 — Le discriminant est `canonical_session_id`, et rien d'autre

La route ne lit pas `identity.toml`, n'appelle pas
`prompt::resolve_canonical_session_id`, ne consulte pas `Settings`. Elle lit
`agent_state.canonical_session_id`, qui est `Some` **si et seulement si**
l'agent déclare `[session] singleton = true`. Ce champ est figé à
`init_agent` ; le relire ailleurs serait répondre sur l'état du disque à cet
instant plutôt que sur l'état sous lequel l'agent tourne — le défaut nommé au
champ `budget_record` (mika#2457), un champ plus bas dans la même structure.

Conséquence directe pour le test d'AC3 : on ne provisionne pas un agent
singleton sur disque, on pose `Some(…)` sur l'`AgentState` de `test_state()`.

### D2 — La route ne prend pas `agent_lock`, et c'est une divergence délibérée avec rewind

Les trois portes rewind prennent le verrou d'agent en `try_lock_owned` et
rendent 409 quand l'agent travaille. C'est correct pour elles : rembobiner
pendant un tour corromprait le tour.

Ouvrir une session n'entre en conflit avec rien. Prendre le verrou ici ferait
rendre 409 à une TUI qui démarre pendant que l'agent répond à un webhook —
c'est-à-dire refuser d'ouvrir une fenêtre parce que l'agent est occupé, ce qui
est l'inverse du service rendu. La divergence est donc dans le sens de la
correction, pas un oubli de recopie. Elle est écrite en commentaire au site,
parce qu'un relecteur qui compare à `rewind.rs` la remarquera et doit trouver
la raison là où il regarde.

### D3 — L'échec d'écriture rend 500 ; `/send` avale le sien, et ce n'est pas le même contrat

`handlers.rs:1712-1729` logue `warn!("failed to create session")` puis continue
avec l'identifiant. `/send` peut se le permettre : il possède le tour qui suit,
et un message perdu est un message perdu.

Ici c'est l'inverse exact du défaut que la route existe pour fermer. Un client
qui reçoit un `session_id` dont la ligne n'existe pas est précisément le client
que `caller_session_is_adoptable` (`a2a_db.rs:119`) refusera d'adopter, et
`a2a_create_task` (l. 187) lui fabriquera une session `a2a-<id>` neuve **à
chaque tour** : il perdra son historique en silence, un tour sur un. Rendre un
identifiant non écrit, ici, c'est livrer le bug.

Donc : `Err` ⇒ `500 {"error": …}`, et aucun identifiant dans le corps.

**Ce 500 n'arrive pas avec un test, et le plan le dit plutôt que de le
maquiller.** Il n'existe aucun point d'injection de panne sur `AsyncDatabase`
dans ce harnais, et l'identifiant est fabriqué *par la route* — on ne peut donc
pas provoquer une collision de clé primaire depuis la requête. Fabriquer ce
point d'injection, c'est modifier `async_db.rs`, donc sortir du périmètre
« seuls des fichiers de `crates/mika-agent/src/server/` changent » (AC5). Le
500 est tenu par la **forme** du code — un `match` dont le bras `Err` ne
construit pas de réponse de succès — et non par une assertion. Aucun des sept
critères ne le demande. C'est une dette nommée, § 6.

### D4 — `POST /api/v1/sessions` partage son chemin avec un GET déjà servi, sous une autre porte d'auth

`server/mod.rs:191` enregistre déjà `GET /sessions` dans `dashboard_routes`,
qui porte `require_dashboard_or_internal_token`. Notre POST doit porter
`require_internal_token` **seul** (AC1 : le jeton du tableau de bord y est
refusé). Deux portes d'auth, un seul chemin, deux méthodes.

**Forme retenue (A)** — un groupe `session_routes` à côté de `rewind_routes`,
mergé dans le même `nest("/api/v1", …)` :

```rust
let session_routes = Router::new()
    .route("/sessions", post(sessions::handle_create_session))
    .route_layer(middleware::from_fn_with_state(
        state.clone(),
        auth::require_internal_token,
    ));
```

puis `.merge(session_routes)` dans la chaîne l. 426-430. Axum fusionne deux
`MethodRouter` sur un même chemin tant que les **méthodes** ne se recouvrent
pas ; `GET` et `POST` ne se recouvrent pas.

**Si cette fusion panique** — c'est-à-dire si Axum refuse de fusionner deux
`MethodRouter` déjà enveloppés par des `route_layer` distincts — le repli est
**la forme (B)**, et elle est sans risque par construction : une seule
déclaration de la route, dans `dashboard_routes`, avec le middleware interne
posé **sur la méthode POST seule** :

```rust
.route(
    "/sessions",
    get(dashboard::handle_sessions_list).post(
        post(sessions::handle_create_session).route_layer(
            middleware::from_fn_with_state(state.clone(), auth::require_internal_token),
        ),
    ),
)
```

`MethodRouter::route_layer` n'applique le middleware qu'aux méthodes déjà
déclarées sur ce `MethodRouter` — donc au POST seul, le GET gardant la porte du
groupe. Le pilote **ne délibère pas** : il écrit (A), lance les tests d'AC1, et
bascule sur (B) si et seulement si le routeur panique à la construction. Les
deux tests d'auth du commit 1 sont ce qui tranche, en un cycle `cargo test`,
parce que `test_app(state)` construit le routeur entier.

### D5 — Les tests vont dans `server/mod.rs § mod tests`, pas dans `sessions.rs`

Pas par goût : par visibilité. `test_state()`, `test_state_with_dashboard_token()`
et `test_app()` sont privés de `server::tests`. `server::sessions::tests` en est
le **frère**, pas le descendant : il ne les voit pas. Les colocaliser au module
coûterait d'élargir trois signatures du harnais — c'est-à-dire modifier le
harnais pour loger un test, exactement ce que la règle de visibilité de
`db/migrations.rs` (mika#2321) refuse dans l'autre sens.

C'est aussi le précédent : les tests des trois routes rewind sont dans
`mod.rs:5555+`, et `rewind.rs` ne porte pas de `mod tests`.

---

## 2. Les livrables, un par un

### 2.1 `crates/mika-agent/src/server/sessions.rs` — le module neuf

En-tête de module : ce que la route sert, et **pourquoi spirit doit écrire la
ligne** — le chemin `caller_session_is_adoptable` → `a2a_create_task` de D3,
cité par fichier et par ligne. C'est le pourquoi qu'on ne lit nulle part dans ce
fichier.

```rust
#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    /// Agent servi. Toujours envoyé par le client, comme pour rewind ;
    /// vide ⇒ agent par défaut, par `resolve_agent`.
    #[serde(default)]
    pub agent: String,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
    /// Vrai quand l'identifiant rendu est la session canonique de l'agent
    /// (mika#1401). Le client doit savoir qu'il ne la clôt jamais : une
    /// session canonique ne se remplace pas. V2d-1b s'appuiera dessus.
    pub canonical: bool,
}
```

Handler `handle_create_session(State(state), Json(req)) -> impl IntoResponse` :

1. `state.resolve_agent(&req.agent).await` ⇒ `None` : `404 {"error": format!("agent '{}' not found", req.agent)}`, mot pour mot la forme de `rewind.rs:124-130`.
2. Pas de verrou (D2, commenté au site).
3. `match agent_state.canonical_session_id.as_deref()` :
   - `Some(canonical)` ⇒ `get_or_create_canonical_session(canonical, CLI_CHANNEL).await`
     - `Ok(id)` ⇒ `200 { session_id: id, canonical: true }` — l'identifiant rendu est **celui que la base rend**, jamais `canonical.to_string()` : si les deux divergeaient un jour, la seule valeur juste est celle de la ligne.
     - `Err(e)` ⇒ 500.
   - `None` ⇒ `let id = Uuid::new_v4().to_string();` puis `create_session(&id, agent_state.db.agent_id(), CLI_CHANNEL).await`
     - `Ok(())` ⇒ `200 { session_id: id, canonical: false }`
     - `Err(e)` ⇒ 500, sans `session_id` (D3, commenté au site).

`const CLI_CHANNEL: &str = "cli";` — un seul site pour la graphie, avec le
renvoi à `commands/chat.rs:101-113` qui est le consommateur que V2d-4
branchera. Deux graphies pour un canal, c'est le défaut de classe D3 que le
corps du ticket condamne une couche plus haut.

### 2.2 `crates/mika-agent/src/server/mod.rs` — l'enregistrement

`pub mod sessions;` à côté de `pub mod rewind;` (l. 29), puis le groupe de D4,
mergé l. 426-430. Trois à cinq lignes de commentaire au groupe : pourquoi un
groupe séparé plutôt que `rewind_routes` (ce n'est pas un rembobinage), et
pourquoi `require_internal_token` seul alors que le GET voisin accepte le jeton
du tableau de bord (ouvrir une session est une mutation, pas une lecture
d'observabilité — le raisonnement de `operator_v1_routes`, l. 378-381).

### 2.3 Les tests, dans `server/mod.rs § mod tests`

Posés juste après le bloc rewind (l. 5629), sous un en-tête de section
`// ===== Routes de session (mika#1727 V2d-1a) =====`.

Un helper, pour ne pas recopier la mutation trois fois :

```rust
/// Un état dont l'agent `mika` est singleton (mika#1401). Même geste que
/// `mika2671_b2_state` : la `DashMap` détient seule l'`Arc`, donc
/// `Arc::get_mut` réussit tant qu'aucun clone n'est sorti.
fn mika1727_state_singleton(canonical: &str) -> AppState { … }
```

| test | AC | ce qu'il assert |
|---|---|---|
| `mika1727_sans_jeton_la_porte_de_session_rend_401` | 1 | 401 — et **pas** 404, ce qui prouve que la route existe ; c'est l'assertion que le test rewind homologue porte en commentaire |
| `mika1727_le_jeton_du_tableau_de_bord_est_refuse` | 1 | `test_state_with_dashboard_token()`, `Bearer dashboard-token-secret` ⇒ 401. Sépare « route interne » de « route d'observabilité » |
| `mika1727_un_agent_non_singleton_recoit_une_session_neuve` | 2 | 200 ; `canonical == false` ; puis `db.get_session(&session_id)` ⇒ `Some(s)` avec `s.agent_id == "mika"` et `s.channel_type == "cli"`. **La lecture en base est le test** : un 200 seul ne dirait pas que la ligne existe, et c'est tout le sujet du vol |
| `mika1727_un_agent_singleton_recoit_toujours_sa_session_canonique` | 3 | deux appels ; même `session_id` ; `canonical == true` aux deux ; `count_sessions(Some("mika"), …) == 1` après le second. Le compte est ce qui sépare « rend le même id » de « n'en crée aucune autre » |
| `mika1727_un_agent_inconnu_rend_404_sans_ecrire` | 4 | `{"agent":"agent-inexistant-1727"}` ⇒ 404, `json["error"]` non vide ; `count_sessions` **avant et après** ⇒ égal. `test_state()` pose `global_home_dir = /tmp/mika-test`, donc la résolution paresseuse ne trouve pas d'`identity.toml` et rend `None` sans toucher le disque réel |

Le compte avant/après du dernier est délibérément une **paire**, pas un
`== 0` : la route pourrait un jour être appelée sur un état déjà peuplé, et une
assertion à zéro mesurerait alors la vacuité du harnais plutôt que l'absence
d'écriture.

---

## 3. Ordre des commits

Deux commits. Chacun porte son code **et** ses tests ; aucun test n'est reporté.

**Commit 1 — la porte, le cas non singleton, l'agent inconnu.**
`sessions.rs` (en-tête, types, handler avec la seule branche `None` et le 404),
l'enregistrement dans `mod.rs`, et trois tests : 401, jeton du tableau de bord,
200 non singleton avec lecture en base, 404 sans écriture. Couvre AC1, AC2, AC4.
C'est aussi le commit qui tranche D4 : si le routeur panique, il panique ici.

**Commit 2 — le cas singleton.**
La branche `Some(canonical)` du `match`, le helper `mika1727_state_singleton`,
et le test des deux appels. Couvre AC3.

Après **chaque** commit : `git diff --numstat origin/main...HEAD`, plan exclu.
Si le total dépasse 429, halte — pas de commit suivant, pas de PR, et le dernier
message dit « HALTE borne : N lignes pour 429 » avec le compte par commit.

Avant d'ouvrir la PR, dans cet ordre : `cargo test -p mika-agent`,
`cargo clippy -p mika-agent --all-targets` (vert, **sans `allow`**),
`bash scripts/verify-egress-manifest.sh --report` (AC6 — rouge ⇒ halte et on le
dit), `bash scripts/verify-pipeline.sh origin/main`.

---

## 4. Definition of Done

- `POST /api/v1/sessions` servie, sous `require_internal_token` seul.
- Agent non singleton ⇒ `200 {"session_id": <uuid>, "canonical": false}` et la ligne existe en base (`agent_id` de l'agent, `channel_type = "cli"`).
- Agent singleton ⇒ `200 {"session_id": <canonique>, "canonical": true}`, stable d'un appel à l'autre, sans seconde ligne.
- Agent inconnu ⇒ `404 {"error": …}`, aucune écriture.
- Échec d'écriture ⇒ `500 {"error": …}`, jamais de `session_id`.
- `cargo test -p mika-agent` et `cargo clippy -p mika-agent --all-targets` verts, sans `allow`.
- `scripts/verify-egress-manifest.sh --report` et `scripts/verify-pipeline.sh origin/main` verts.
- Diff limité à `crates/mika-agent/src/server/` (+ `docs/plans/` pour ce fichier). `crates/mika-cli/`, `crates/mika-a2a/`, `docs/egress/`, `docs/openapi/`, `.github/workflows/`, `.claude/`, `skills/bundled/`, `well_known_agents.rs`, `verdict_handler.rs`, `.mika/` : intacts.
- PR : `Refs mika#1727`. Aucun `Closes` / `Fixes` / `Resolves`, corps **et** messages de commit. `gh pr view <N> --json closingIssuesReferences` rend une liste vide — vérifié même, et surtout, si la PR est ouverte par le gabarit de reprise (`wip-rescue`), qui écrit `Closes #N` par défaut et a dû être corrigé à la main sur #3077 et #3081.
- Aucun ticket ouvert (gel jusqu'au 01/12). Reste et manques ⇒ commentaire sur #1727 ou corps de PR, avec `Tracked in: senara-solutions/mika#1727`.

## 5. Acceptance criteria

Transcrits du corps de #1727, § « Critères de V2d-1a ».

1. Sans jeton, `POST /api/v1/sessions` rend 401, et le jeton du tableau de bord y est refusé. Un test pour chaque cas, sur le modèle des tests rewind.
2. Pour un agent non singleton, la route rend 200 avec un `session_id` neuf et `canonical: false`. La ligne existe en base avec cet `agent_id` et `channel_type = "cli"`. Un test lit la ligne en base.
3. Pour un agent singleton, la route rend sa session canonique avec `canonical: true`, la même à chaque appel, et n'en crée aucune autre. Un test appelle la route deux fois.
4. Pour un agent inconnu, la route rend 404 avec `{"error": …}`, et aucune ligne n'est créée. Un test le montre.
5. `cargo clippy -p mika-agent --all-targets` est vert, sans `allow`. Seuls des fichiers de `crates/mika-agent/src/server/` changent. `crates/mika-cli/`, `crates/mika-a2a/`, `docs/egress/` et `docs/openapi/` restent intacts.
6. Le pilote lance `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
7. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si elle est ouverte par le gabarit de reprise.

---

## 6. Ce que ce vol ne fait pas, et la dette nommée

**Hors périmètre, par le corps du ticket :**

- La reprise d'un `--session-id` existant et son contrôle d'appartenance — ce chemin peut lire `GET /api/v1/sessions/{id}`, qui existe déjà. Tranché en V2d-4.
- La clôture (`POST /api/v1/sessions/{id}/end`) — V2d-1b.
- `/compact` — V2d-2.
- Les portes côté client et l'entrée d'egress — V2d-3. **Aucun client sortant n'est ouvert ici**, donc `docs/egress/` reste intact.
- La bascule des appelants de la TUI et la garde structurelle — V2d-4. **Aucun appelant ne change dans ce vol** : `crates/mika-cli/` n'est pas touché, et la route n'a donc aucun consommateur à la fin de ce vol. C'est voulu — la série commence côté serveur parce que spirit n'a aucune route pour ces gestes.

`Tracked in: senara-solutions/mika#1727`

**Dette créée par ce vol, et non traitée :**

- **Le 500 de D3 n'a pas de test.** Pas de point d'injection de panne sur `AsyncDatabase`, et l'identifiant est fabriqué par la route, donc pas de collision provocable depuis la requête. En fabriquer un sort du périmètre d'AC5. La garantie est de forme, pas d'assertion. À rouvrir si un vol ultérieur a de toute façon besoin d'injecter une panne DB au niveau serveur.
- **`docs/openapi/mika-spirit.yaml` ne décrira pas cette route**, comme il ne décrit aucune des trois routes rewind. L'écart est antérieur à ce vol et son comblement toucherait un fichier exclu (toute modification serait copiée dans `crates/mika-agent/docs/openapi/`).

**Règle mika#2306 — gate N/A, et c'est une évaluation, pas un oubli.** Ce vol
ne livre aucun détecteur : pas de lint, pas de garde CI, pas de scan structurel,
pas de garde EndTurn, pas de validateur de schéma. Les cinq tests sont des tests
de comportement — leur chemin de succès est « la route a rendu 401 / 200 / 404
et la base porte la ligne attendue », pas « aucune violation trouvée » sur une
population existante. Aucune violation préexistante n'est donc à amnistier, à
désarmer ou à remonter, et la section `## Fire-Disposition` n'est pas écrite
plutôt qu'inventée vide.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/sessions.rs` (en-tête, 2 types, handler, `CLI_CHANNEL`) | 115 |
| `crates/mika-agent/src/server/mod.rs` (`pub mod`, groupe de routes, commentaires) | 18 |
| tests (`server/mod.rs § mod tests` — helper + 5 tests) | 155 |

Total estimé : 288 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 288 contre 429 : la marge est de 141 lignes,
soit ~33 %. Elle est réelle et non optimiste, pour trois raisons mesurées en
§ 0. *Aucun SQL n'est écrit* — les deux écritures existent déjà sur
`AsyncDatabase` (l. 2018 et 2111), ce vol les appelle. *Aucune résolution n'est
écrite* — le discriminant singleton est un champ déjà peuplé
(`canonical_session_id`), et le 404 est le `None` de `resolve_agent`. *Aucun
harnais n'est écrit* — `test_state`, `test_state_with_dashboard_token`,
`test_app` et le geste `Arc::get_mut` existent et ne sont pas modifiés. Le poste
le plus lourd est celui des tests (155), et c'est délibéré : AC2, AC3 et AC4
exigent chacun une lecture ou un comptage en base, pas seulement un code de
statut.

La borne par PR (1000) ne mord pas. La borne par vol (429) est la plus serrée
des deux, et c'est elle qui est tenue. Pas de découpage en vols
supplémentaires : la suite est déjà découpée par le corps du ticket en V2d-1b,
V2d-2, V2d-3 et V2d-4.
