# Vol V2d-1b — mika-spirit clôt une session pour la TUI (mika#1727)

**Ticket :** senara-solutions/mika#1727 — `Refs`, jamais `Closes`. Le ticket se ferme au merge de V4.
**Branche :** `feat/1727/tui-v2d1b-clore-session-par-spirit`, neuve depuis `main`.
**Base mesurée :** `d33e84f3` (merge de V2d-1a, #3082).

---

## 0. Ce qui est mesuré

Tout ce qui suit a été lu sur la branche, à `d33e84f3`. Les numéros de ligne
sont ceux de cette révision ; ils servent à retrouver le site, pas à être
rejoués tels quels.

| fait | site | ce qu'il décide ici |
|---|---|---|
| `handle_create_session`, 123 lignes, avec `resolve_agent` → 404, `CLI_CHANNEL`, et les aides `ok` / `write_failed` | `server/sessions.rs:48-123` | le module d'accueil et le moule du handler. Les deux aides sont **privées au module** : elles sont réutilisables telles quelles, sans rien élargir |
| le commentaire « pas de verrou d'agent » et son raisonnement complet | `server/sessions.rs:63-69` | ce vol **y renvoie** et ne le recopie pas (§ 1, D4) |
| `session_routes`, un `Router` sous `require_internal_token` seul, mergé dans le `nest("/api/v1", …)` | `server/mod.rs:385-396`, `449` | le site d'enregistrement : **une ligne `.route(…)` à ajouter**, pas un groupe neuf |
| `GET /sessions/{id}` vit dans `dashboard_routes`, sous `require_dashboard_or_internal_token` | `server/mod.rs:193` | **pas de collision** : `/sessions/{id}/end` est un autre chemin. Le risque D4 de V2d-1a ne se rejoue pas ici |
| `AsyncDatabase::get_session(id) -> Result<Option<Session>>` | `async_db.rs:3261` | la lecture d'appartenance et le `None` du 404 |
| `Session { id, agent_id, channel_type, started_at, ended_at: Option<String>, … }` | `db.rs:455-464` | `ended_at` est le champ que les tests d'AC2, AC3 et AC5 lisent |
| `AsyncDatabase::end_session_unless_canonical(id, Option<&str>) -> Result<()>` | `async_db.rs:2093` ; corps sync `db.rs:2385-2390` | l'écriture. Le corps est `if canonical_id == Some(id) { return Ok(()) }` puis `end_session(id)` |
| `AgentState.canonical_session_id: Option<String>`, figé à `init_agent` | `server/state.rs:111` | le discriminant singleton, déjà lu par `handle_create_session` |
| `AsyncDatabase::agent_id() -> &str` | `async_db.rs:130` | l'identifiant auquel la session lue est comparée |
| `AsyncDatabase::create_session(id, agent_id, channel_type)` prend l'`agent_id` **en paramètre** | `async_db.rs:2018` | **comment fabriquer en test une session appartenant à un autre agent** sans provisionner un second agent |
| `count_sessions(agent_id, channel_type, session_id, task_id, from, to)` | `async_db.rs:3239` | le « aucune écriture » des cas 404, en couple avant/après |
| les cinq tests de V2d-1a + le helper `mika1727_state_singleton` | `server/mod.rs:5656-5843` | le modèle **et** le banc singleton, réutilisé tel quel |

Trois faits qui sont des **absences**, et une absence se dit :

- `sessions.rs` ne porte **aucun** `mod tests`, comme `rewind.rs`. Les tests de
  routes vivent dans `server/mod.rs § mod tests` ; ce vol ne change pas ce
  précédent (§ 1, D5 de V2d-1a, toujours en vigueur et non relitigé ici).
- `docs/openapi/mika-spirit.yaml` ne décrit **ni** les trois routes rewind **ni**
  le `POST /api/v1/sessions` de V2d-1a. Ce vol n'y touche pas non plus : le corps
  du ticket l'exclut nommément.
- **Aucun appelant n'existe** pour cette route à la fin du vol. `crates/mika-cli/`
  n'est pas touché ; la bascule est en V2d-4.

---

## 1. La forme, et les cinq décisions qui la tiennent

Un handler, quatre issues, aucune ligne de SQL. Ce qui mérite d'être écrit, ce
sont les quatre endroits où la forme évidente est fausse, plus celui où elle est
juste mais pour une raison qui ne se lit pas.

### D1 — La garde canonique passe AVANT la lecture en base, et l'ordre est la décision

La forme évidente est : lire la session, vérifier qu'elle appartient à l'agent,
puis regarder si c'est la canonique. **Elle est fausse sur AC5.** La session
canonique d'un agent singleton n'est écrite qu'au premier
`get_or_create_canonical_session` ; un client qui appelle `/end` dessus avant que
la ligne existe recevrait un 404 là où le critère exige
`200 {"ended": false, "canonical": true}`. Pire, cette réponse dépendrait de
l'ordre des appels du client — une route dont l'issue dépend d'un état que
l'appelant ne peut pas observer.

L'ordre retenu est donc :

1. `resolve_agent(&req.agent)` ⇒ `None` : 404.
2. `agent_state.canonical_session_id.as_deref() == Some(id)` ⇒
   `200 {"ended": false, "canonical": true}`, **sans aucune lecture et sans
   aucune écriture**.
3. `get_session(id)` ⇒ `None`, ou `Some(s)` avec `s.agent_id != db.agent_id()` :
   le **même** 404.
4. sinon `end_session_unless_canonical(id, canonical)` ⇒
   `200 {"ended": true, "canonical": false}`.

Court-circuiter à l'étape 2 **saute le contrôle d'appartenance**, et c'est sans
risque par construction, mais la raison n'est pas évidente et va en commentaire :
`canonical_session_id` est un champ de l'`AgentState` **de l'agent servi**, résolu
à son `init_agent` depuis sa propre `identity.toml`. Il ne peut donc pas désigner
la session d'un autre agent. Ce n'est pas une donnée de la requête.

### D2 — Session inconnue et session d'autrui rendent le MÊME 404, et c'est la décision de sécurité du vol

Le corps du ticket la tranche, ce plan l'applique et dit son coût. Un 403 sur la
session d'autrui, ou un message d'erreur qui diffère entre les deux cas, répond à
une question que l'appelant n'a pas le droit de poser : *cette session existe-t-elle
chez quelqu'un d'autre ?* Avec deux issues distinguables, un porteur du jeton
interne énumère les identifiants de session de tout le conteneur.

La conséquence en code est contraignante et doit être écrite : **les deux cas
construisent le même corps d'erreur, par la même aide**, et pas deux `format!`
voisins qu'une édition future ferait diverger sans rien casser. Un seul site :

```rust
/// Un seul site pour les deux cas, et c'est le fond de la décision : un
/// message qui différerait entre « inconnue » et « à quelqu'un d'autre »
/// rendrait les sessions du conteneur énumérables par un appelant qui n'a
/// que le jeton. Le texte ne nomme donc jamais l'agent propriétaire.
fn session_not_found(session_id: &str) -> axum::response::Response
```

**Le test d'AC3 est ce qui tient cette propriété** : il asserte le 404 *et* que
`ended_at` de la session d'autrui est resté `NULL` — le second sans le premier
laisserait passer un 403 bavard, le premier sans le second laisserait passer une
clôture silencieuse.

### D3 — `end_session_unless_canonical` est appelée avec son `canonical`, bien que D1 ait déjà écarté ce cas

C'est redondant, délibérément, et ce n'est pas une ceinture-et-bretelles de
confort : les deux gardes répondent à deux questions différentes. D1 décide la
**réponse** (`canonical: true`, `ended: false` — un fait, pas une erreur). Le
paramètre `canonical_id` tient l'**écriture** (mika#1401 : l'`ended_at` de la
canonique reste `NULL` pour toujours).

Elles ne sont pas dans la même unité de temps non plus. Si un vol ultérieur
réordonne le handler — et V2d-4 y touchera — le court-circuit de D1 est ce qui
peut bouger ; l'invariant mika#1401, lui, doit survivre à ce réordonnancement.
Appeler `end_session(id)` « puisque le cas est déjà traité » déplacerait
l'invariant dans l'ordre des lignes du handler. Le commentaire au site dit cela
en une phrase.

### D4 — Pas de verrou d'agent, et le commentaire RENVOIE au raisonnement de la création

`sessions.rs:63-69` porte déjà la divergence délibérée avec les trois portes de
`rewind.rs` : rembobiner pendant un tour corromprait le tour ; ouvrir une session
n'entre en conflit avec rien. Clore une session tombe du même côté — clore la
fenêtre d'une TUI pendant que l'agent répond à un webhook n'invalide aucun tour
en cours, puisque le tour en cours n'est pas celui de cette session.

Le commentaire du handler de clôture fait donc **deux lignes et un renvoi**, pas
une recopie. Une seconde copie du raisonnement est une seconde chose à mettre à
jour, et le corps du ticket dit explicitement de renvoyer.

### D5 — La lecture d'appartenance est nécessaire PARCE QUE la base est unique par conteneur

Ce qu'un relecteur peut croire superflu : `agent_state.db` est une
`AsyncDatabase` scopée à l'agent, donc pourquoi vérifier `session.agent_id` ?

Parce que le scope est **l'identifiant porté par la poignée**, pas une partition
des données. Il y a un fichier SQLite par conteneur, partagé par tous les agents
(`AsyncDatabase::with_agent` est un clone qui partage l'`Arc` interne), et
`get_session(id)` fait un `SELECT … WHERE id = ?` sans clause sur `agent_id`. La
table `sessions` du conteneur est donc entièrement visible depuis n'importe
quelle poignée. Sans la comparaison explicite, la route clôt la session de
n'importe quel agent du conteneur sur simple connaissance de son identifiant.

C'est aussi ce qui rend le test d'AC3 écrivable sans provisionner un second
agent : `create_session(id, "autre-agent-1727", "cli")` sur la poignée de `mika`
écrit une ligne dont l'`agent_id` est celui qu'on lui passe.

---

## 2. Les livrables, un par un

### 2.1 `crates/mika-agent/src/server/sessions.rs` — la route de clôture

L'en-tête de module existant parle de la création. Il reçoit un paragraphe sur la
clôture : ce que la route sert, et **pourquoi `ended: false` n'est pas une
erreur** — une session canonique ne se remplace pas (mika#1401), donc le client
lit ce booléen comme un fait sur la nature de la session, et non comme un refus.

```rust
#[derive(Debug, Deserialize)]
pub struct EndSessionRequest {
    /// Agent servi. Toujours envoyé, comme pour la création ; une chaîne vide
    /// désigne l'agent par défaut, par `resolve_agent`.
    #[serde(default)]
    pub agent: String,
}

#[derive(Debug, Serialize)]
pub struct EndSessionResponse {
    /// Faux **sans erreur** quand la session est la canonique de l'agent : elle
    /// ne se clôt jamais (mika#1401). Le client lit un fait, pas un refus.
    pub ended: bool,
    pub canonical: bool,
}
```

Handler `handle_end_session(State(state), Path(session_id), Json(req)) -> impl IntoResponse`,
dans l'ordre de D1. `Path` avant `Json` : l'extracteur qui consomme le corps est
nécessairement le dernier argument.

Deux aides privées, à côté des deux existantes :

- `session_not_found(session_id)` — D2, un seul site pour les deux cas.
- `ended(ended: bool, canonical: bool)` — le pendant d'`ok`, qui sérialise
  `EndSessionResponse`.

L'échec de `get_session` et celui de `end_session_unless_canonical` rendent tous
deux 500 ; `write_failed` existe mais son message dit « session creation
failed ». Deux choix, et le plan tranche : **un second site,
`end_failed(e)`**, plutôt qu'un message générique qui couvrirait les deux routes.
Un 500 dont le texte ne dit pas quelle opération a échoué est un 500 qu'on
diagnostique en relisant le code ; les deux routes ne coûtent que deux fonctions
de cinq lignes. (Le coût est compté au § *Taille estimée*.)

### 2.2 `crates/mika-agent/src/server/mod.rs` — l'enregistrement

Une ligne dans `session_routes` (l. 391-396), à côté de la création :

```rust
.route("/sessions/{id}/end", post(sessions::handle_end_session))
```

Le commentaire du groupe (l. 385-390) couvre déjà le « jeton interne seul » ; il
reçoit **une** phrase de plus : clore écrit une ligne, donc c'est une mutation,
comme ouvrir. Pas de nouveau groupe, pas de nouveau `route_layer`.

### 2.3 Les tests, dans `server/mod.rs § mod tests`

Posés après le bloc V2d-1a (l. 5843), sous un en-tête
`// ===== Clôture de session (mika#1727 V2d-1b) =====`. Le helper
`mika1727_state_singleton` est réutilisé tel quel — aucun harnais n'est écrit ni
modifié.

| test | AC | ce qu'il assert |
|---|---|---|
| `mika1727_sans_jeton_la_cloture_de_session_rend_401` | 1 | 401 — et **pas** 404, ce qui prouve que la route existe |
| `mika1727_le_jeton_du_tableau_de_bord_est_refuse_a_la_cloture` | 1 | `test_state_with_dashboard_token()`, `Bearer dashboard-token-secret` ⇒ 401. Le `GET /sessions/{id}` voisin accepte ce jeton ; la clôture non |
| `mika1727_une_session_de_lagent_se_clot` | 2 | `create_session("sess-1727-a", "mika", "cli")`, puis `/end` ⇒ 200, `ended == true`, `canonical == false` ; puis `get_session` ⇒ `ended_at.is_some()`. **La lecture de la ligne est le test** |
| `mika1727_une_session_dun_autre_agent_rend_404_et_reste_ouverte` | 3 | `create_session("sess-1727-b", "autre-agent-1727", "cli")`, `/end` avec `{"agent":"mika"}` ⇒ 404 ; `get_session("sess-1727-b").ended_at.is_none()`. Le couple des deux assertions est la propriété (D2) |
| `mika1727_une_session_inconnue_rend_404` | 4 | identifiant jamais écrit ⇒ 404, `json["error"]` non vide ; `count_sessions` avant/après ⇒ égal |
| `mika1727_la_session_canonique_ne_se_clot_jamais` | 5 | banc `mika1727_state_singleton("canonical-mika-1727")` ; `POST /sessions` pour faire naître la ligne, puis `/end` sur cet identifiant ⇒ 200, `ended == false`, `canonical == true` ; `get_session(…).ended_at.is_none()` |

Le dernier test ouvre la session **par la route de V2d-1a** plutôt que par
`get_or_create_canonical_session` directement : c'est le chemin que le client
suivra en V2d-4, et le faire ainsi atteste au passage que les deux routes du vol
V2d parlent du même identifiant.

Le `count_sessions` avant/après du cas « inconnue » est délibérément une
**paire**, pas un `== 0` : la base du harnais porte déjà des sessions, donc une
assertion à zéro mesurerait la vacuité du banc plutôt que l'absence d'écriture.

---

## 3. Ordre des commits

Trois commits. Chacun porte son code **et** ses tests ; aucun test n'est reporté
à un commit ultérieur ni à un vol ultérieur.

**Commit 1 — la porte et le cas nominal.**
`EndSessionRequest` / `EndSessionResponse`, le handler avec `resolve_agent`, la
lecture, le contrôle d'appartenance et l'appel à `end_session_unless_canonical`
— **déjà avec son argument `canonical`** (D3), pour qu'aucun commit intermédiaire
ne puisse clore une canonique — l'aide `ended`, l'enregistrement dans `mod.rs`.
Tests : 401, jeton du tableau de bord, clôture nominale avec lecture de la ligne.
Couvre AC1 et AC2.

**Commit 2 — le 404 des deux cas, et son site unique.**
L'aide `session_not_found` et les deux bras qui l'appellent, avec le commentaire
de D2. Tests : session d'un autre agent (404 + `ended_at` toujours `NULL`),
session inconnue (404 + couple `count_sessions`). Couvre AC3 et AC4.

**Commit 3 — la garde canonique.**
Le court-circuit de D1 avec son commentaire, et son test sur le banc singleton.
Couvre AC5.

Après **chaque** commit : `git diff --numstat origin/main...HEAD`, plan exclu.
Si le total dépasse 429, **halte** — pas de commit suivant, pas de PR, et le
dernier message dit « HALTE borne : N lignes pour 429 » avec le compte par
commit.

Avant d'ouvrir la PR, dans cet ordre : `cargo test -p mika-agent`,
`cargo clippy -p mika-agent --all-targets` (vert, **sans `allow`**),
`bash scripts/verify-egress-manifest.sh --report` (AC7 — rouge ⇒ halte et on le
dit), `bash scripts/verify-pipeline.sh origin/main`.

---

## 4. Definition of Done

- `POST /api/v1/sessions/{id}/end` servie dans `session_routes`, sous `require_internal_token` seul.
- Session de l'agent servi ⇒ `200 {"ended": true, "canonical": false}` et `ended_at` posé en base.
- Session inconnue **ou** appartenant à un autre agent ⇒ le **même** `404 {"error": …}`, par un site unique, sans aucune écriture.
- Session canonique de l'agent ⇒ `200 {"ended": false, "canonical": true}`, sans lecture ni écriture, `ended_at` toujours `NULL`.
- Agent inconnu ⇒ `404 {"error": …}`.
- Échec de lecture ou d'écriture ⇒ `500 {"error": …}`.
- Aucun verrou d'agent pris ; le commentaire renvoie au raisonnement de `handle_create_session` sans le recopier.
- `cargo test -p mika-agent` et `cargo clippy -p mika-agent --all-targets` verts, sans `allow`.
- `scripts/verify-egress-manifest.sh --report` et `scripts/verify-pipeline.sh origin/main` verts.
- Diff limité à `crates/mika-agent/src/server/sessions.rs` et `crates/mika-agent/src/server/mod.rs` (+ `docs/plans/` pour ce fichier). `crates/mika-cli/`, `crates/mika-a2a/`, `docs/egress/`, `docs/openapi/`, `.github/workflows/`, `.claude/`, `skills/bundled/`, `well_known_agents.rs`, `verdict_handler.rs`, `.mika/` : intacts.
- PR : `Refs mika#1727`. Aucun `Closes` / `Fixes` / `Resolves`, corps **et** messages de commit. `gh pr view <N> --json closingIssuesReferences` rend une liste vide — vérifié même, et surtout, si la PR est ouverte par le gabarit de reprise (`wip-rescue`), qui écrit `Closes #N` par défaut et a dû être corrigé à la main sur #3077 et #3081.
- Aucun ticket ouvert (gel jusqu'au 01/12). Reste et manques ⇒ commentaire sur #1727 ou corps de PR, avec `Tracked in: senara-solutions/mika#1727`.

## 5. Acceptance criteria

Transcrits du corps de #1727, § « Critères de V2d-1b ».

1. Sans jeton, `POST /api/v1/sessions/{id}/end` rend 401. Le jeton du tableau de bord y est refusé. Un test pour chaque cas.
2. Pour une session de l'agent, la route rend `200 {ended: true, canonical: false}`, et `ended_at` est posé en base. Un test lit la ligne.
3. Pour une session d'un autre agent, la route rend 404, et l'`ended_at` de cette session reste NULL. Un test lit la ligne.
4. Pour une session inconnue, la route rend 404. Un test le montre.
5. Pour la session canonique d'un agent singleton, la route rend `200 {ended: false, canonical: true}`, et son `ended_at` reste NULL. Un test lit la ligne.
6. `cargo clippy -p mika-agent --all-targets` est vert, sans `allow`. Seuls `server/sessions.rs` et `server/mod.rs` changent. `crates/mika-cli/`, `crates/mika-a2a/`, `docs/egress/` et `docs/openapi/` sont intacts.
7. Le pilote lance `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
8. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si elle est ouverte par le gabarit de reprise.

---

## 6. Ce que ce vol ne fait pas, et la dette nommée

**Hors périmètre, par le corps du ticket :**

- **Le cas d'une session déjà close.** Un second `/end` réécrit `ended_at`, exactement comme `end_session` le fait aujourd'hui dans la TUI (`crates/mika-cli`). Le comportement n'est donc ni meilleur ni pire qu'avant, et la bascule V2d-4 n'introduira rien de neuf sur cet axe. Ni code ni test ici.
- `POST /api/v1/compact` — V2d-2.
- Les portes côté client et l'entrée d'egress — V2d-3. **Aucun client sortant n'est ouvert ici**, donc `docs/egress/` reste intact.
- La bascule des appelants de la TUI et la garde structurelle — V2d-4. **Aucun appelant ne change dans ce vol** : `crates/mika-cli/` n'est pas touché, et la route n'a aucun consommateur à la fin du vol.

`Tracked in: senara-solutions/mika#1727`

**Dette créée par ce vol, et non traitée :**

- **Les deux 500 n'ont pas de test**, pour la raison déjà nommée en V2d-1a (§ 6 de `2026-11-13-003`) : il n'existe aucun point d'injection de panne sur `AsyncDatabase` dans ce harnais, et en fabriquer un demanderait de modifier `async_db.rs` — donc de sortir du périmètre « seuls `server/sessions.rs` et `server/mod.rs` changent » (AC6). La garantie est de **forme** — deux `match` dont le bras `Err` ne construit aucune réponse de succès — et non d'assertion. Aucun des huit critères ne la demande. À rouvrir si un vol ultérieur a de toute façon besoin d'injecter une panne DB au niveau serveur.
- **`docs/openapi/mika-spirit.yaml` ne décrira pas cette route**, comme il ne décrit ni les trois routes rewind ni le `POST /api/v1/sessions` de V2d-1a. L'écart est antérieur à ce vol et son comblement toucherait un fichier exclu (toute modification serait copiée dans `crates/mika-agent/docs/openapi/`).

**Règle mika#2306 — gate N/A, et c'est une évaluation, pas un oubli.** Ce vol ne
livre aucun détecteur : pas de lint, pas de garde CI, pas de scan structurel, pas
de garde EndTurn, pas de validateur de schéma. Les six tests sont des tests de
comportement — leur chemin de succès est « la route a rendu 401 / 200 / 404 et la
ligne porte l'`ended_at` attendu », et non « aucune violation trouvée » sur une
population existante. Aucune violation préexistante n'est donc à amnistier, à
désarmer ou à remonter, et la section `## Fire-Disposition` n'est pas écrite
plutôt qu'inventée vide.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/sessions.rs` (paragraphe d'en-tête, 2 types, handler, 3 aides) | 100 |
| `crates/mika-agent/src/server/mod.rs` (une route, une phrase de commentaire) | 5 |
| tests (`server/mod.rs § mod tests` — 6 tests, helper réutilisé) | 230 |

Total estimé : 335 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient.** 335 contre 429 : la marge est de 94 lignes, soit
~22 %. Elle est calibrée sur une mesure, pas sur une intuition : V2d-1a a estimé
288 lignes pour un module **neuf** de 123 lignes, un groupe de routes neuf et
cinq tests occupant `mod.rs:5656-5843`, soit 188 lignes réelles pour les tests.
Ce vol est strictement plus léger côté code — le module existe, le groupe de
routes existe, les deux aides `ok` et `write_failed` sont le moule, aucun SQL
n'est écrit (`get_session` et `end_session_unless_canonical` existent déjà) et
aucun harnais n'est touché (`mika1727_state_singleton` est réutilisé tel quel).
Le poste lourd est celui des tests (230 pour 6, contre 188 pour 5), et c'est
délibéré : AC2, AC3 et AC5 exigent chacun une **lecture de ligne**, AC4 un couple
`count_sessions`, et AC3 fait naître une session d'un autre agent avant d'appeler.

La borne par PR (1000) ne mord pas. La borne par vol (429) est la plus serrée des
deux, et c'est elle qui est tenue. Pas de découpage en vols supplémentaires : la
suite est déjà découpée par le corps du ticket en V2d-2, V2d-3 et V2d-4.
