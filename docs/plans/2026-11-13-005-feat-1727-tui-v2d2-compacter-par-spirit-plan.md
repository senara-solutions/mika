# Vol V2d-2 — mika-spirit compacte la conversation d'un agent servi (mika#1727)

**Ticket :** senara-solutions/mika#1727 — `Refs`, jamais `Closes`. Le ticket se ferme au merge de V4.
**Branche :** `feat/1727/tui-v2d2-compacter-par-spirit`, neuve depuis `main`.
**Base mesurée :** `62456241` (merge de V2d-1b, #3084).

---

## 0. Ce qui est mesuré

Tout ce qui suit a été lu sur la branche, à `62456241`. Les numéros de ligne sont
ceux de cette révision ; ils servent à retrouver le site, pas à être rejoués tels
quels.

| fait | site | ce qu'il décide ici |
|---|---|---|
| `compaction::maybe_compact(db: &AsyncDatabase, llm: &dyn LlmProvider) -> Result<()>` | `compaction.rs:47` | **tout** le travail du vol se délègue à cet appel. Aucune logique de compaction, aucun SQL n'est écrit ici |
| `pub(crate) const COMPACTION_THRESHOLD: usize = 50` | `compaction.rs:11` | le seuil est **lu**, jamais recopié : `server` est dans le même crate, donc `pub(crate)` suffit (§ 1, D4) |
| `maybe_compact` relit `count_messages()` elle-même et rend `Ok(())` sans rien faire sous le seuil | `compaction.rs:48-55` | pourquoi `compacted: true` est **déclaratif** et non attesté (§ 1, D3) |
| `CONTEXT_WINDOW = 20`, `load_messages_before_window(20)`, puis `summarize_messages` (un appel LLM) puis `replace_with_summary` | `compaction.rs:12`, `57-90` | **un** appel LLM par compaction ⇒ `calls_made() == 1` d'AC4 ; et l'erreur du LLM survient **avant** l'écriture ⇒ compte inchangé d'AC6 |
| le couple `resolve_agent` → 404 puis `agent_lock.clone().try_lock_owned()` → 409 « Agent is busy. Try again when idle. », trois fois à l'identique | `server/rewind.rs:122-143`, `171-196`, `257-282` | le moule exact des deux premières issues |
| `AgentState { db: AsyncDatabase, agent_lock: Arc<tokio::sync::Mutex<()>>, llm: Arc<dyn LlmProvider>, … }` | `server/state.rs` | les trois champs que le handler touche, et aucun autre |
| `AsyncDatabase::count_messages() -> Result<usize>`, scopée à l'agent par la poignée, exclut les lignes `summary` | `async_db.rs:2316` | le `messages: N` de la réponse, et le « compte inchangé » d'AC3 et d'AC6 |
| `AsyncDatabase::load_conversation_summary() -> Result<Option<SessionMessage>>` | `async_db.rs:2310` | le « une ligne `summary` existe en base » d'AC4, lu par le test |
| `AsyncDatabase::save_message(session_id, role, content, trace_id) -> Result<i64>` — l'`agent_id` vient de la poignée, pas d'un paramètre | `async_db.rs:2133` ; usage en test `server/mod.rs:4409` | comment semer les 51 messages d'AC4 et d'AC6, **sans créer de session** : le test voisin le fait déjà ainsi |
| `session_routes`, un `Router` sous `require_internal_token` seul, mergé dans le `nest("/api/v1", …)` | `server/mod.rs:392-398` | le site d'enregistrement : **une ligne `.route(…)` à ajouter**, pas un groupe neuf, pas de `route_layer` de plus |
| le commentaire du groupe, qui porte déjà « une mutation n'est pas une lecture d'observabilité » | `server/mod.rs:379-391` | il reçoit **une** phrase, il n'est pas réécrit |
| `mika2671_b2_state()` — `Arc::get_mut` sur l'entrée de la `DashMap` pour remplacer `AgentState.llm` par un `MockLlmProvider` | `server/mod.rs:6078-6098` | le mécanisme d'injection, repris tel quel dans un helper local au bloc |
| `MockLlmProvider::builder().responses(Vec<MockResponse>)` / `.error(LlmError)` / `calls_made() -> usize` | `mika-common/src/llm/mock.rs:95`, `105`, `207`, `213` | le banc. `.error(e)` pousse un `MockResponse::Error(e)` **dans la même séquence** que `.responses` |
| `test_state()` `:2357`, `test_app(state)` `:2576`, `test_state_with_dashboard_token()` `:4326` | `server/mod.rs` | les quatre helpers du harnais, réutilisés sans modification |
| les tests `mika1727_*` de V2d-1a et V2d-1b | `server/mod.rs:5661-6021` | le modèle du 401 et du refus du jeton du tableau de bord |
| la compaction d'après tour s'exécute **hors du verrou** (`drop(_lock)` puis `tokio::spawn`) | `server/handlers.rs:2390-2401` | la course nommée au § 6, hors de ce vol |

Trois faits qui sont des **absences**, et une absence se dit :

- `compact.rs` n'existe pas. Le module est neuf, et comme `sessions.rs` et
  `rewind.rs` il ne portera **aucun** `mod tests` : les tests de routes vivent
  dans `server/mod.rs § mod tests`. Ce précédent n'est pas relitigé ici.
- `docs/openapi/mika-spirit.yaml` ne décrit **ni** les trois routes rewind, **ni**
  les deux routes de session de V2d-1a/V2d-1b. Ce vol n'y touche pas non plus :
  le corps du ticket l'exclut nommément.
- **Aucun appelant n'existe** pour cette route à la fin du vol.
  `crates/mika-cli/src/tui/commands/handlers.rs:194-208` garde son `/compact`
  local, qui résume avec `app.llm` ; la bascule est en V2d-4.

---

## 1. La forme, et les cinq décisions qui la tiennent

Un handler, cinq issues, aucune ligne de SQL et aucune ligne de compaction. Ce
qui mérite d'être écrit, ce sont les quatre endroits où la forme évidente est
fausse ou incomplète, plus celui où elle est juste pour une raison qui ne se lit
pas dans le code.

### D1 — Le verrou est tenu pendant TOUT le handler, appel LLM compris, et la variable doit être nommée

La forme évidente pour un relecteur venant de `sessions.rs` est « pas de verrou,
comme les deux routes voisines ». **Elle est fausse ici**, et c'est la divergence
la plus importante du vol. `handle_create_session` et `handle_end_session`
écrivent une ligne qui n'entre en conflit avec rien ; compacter **réécrit
l'historique** que le tour en cours est en train de lire. Entre
`load_messages_before_window` et `replace_with_summary` il y a un appel réseau de
plusieurs secondes ; un tour qui écrirait dans cette fenêtre verrait ses messages
remplacés par un résumé qui ne les contient pas.

Donc : `agent_lock.clone().try_lock_owned()`, exactement comme les trois portes de
`rewind.rs`, et **le garde vit jusqu'au retour du handler**. Un message qui arrive
pendant ce temps attend, au lieu de courir en parallèle. Le commentaire au site
dit ce pourquoi-là, pas une paraphrase du code.

Une conséquence technique qui ne se lit pas et qui va dans ce commentaire :
`let _lock = …` garde l'`OwnedMutexGuard` vivant jusqu'à la fin de la portée,
tandis que `let _ = …` le **détruirait sur place** et relâcherait le verrou avant
l'appel LLM — une ligne visuellement identique qui annule la garantie, sans
qu'aucun test de ce vol ne rougisse (le test d'AC5 prend le verrou *avant*
l'appel, il ne mesure pas sa durée de détention).

### D2 — `messages: N` est le compte d'AVANT, dans les DEUX issues

Le corps du ticket le tranche ; la raison va en commentaire parce qu'elle décide
la lisibilité du champ. Le compte d'avant est le seul que le handler ait déjà en
main : il l'a lu pour décider du seuil. Rendre le compte d'après dans l'issue
`compacted: true` et celui d'avant dans l'issue `compacted: false` ferait d'un
même champ deux grandeurs différentes selon l'issue — un client qui trace
`messages` au fil des appels mesurerait tantôt une taille, tantôt une taille
moins le résumé, sans rien qui le lui dise.

Un second `count_messages()` après la compaction n'est donc pas une amélioration :
c'est un second SELECT pour un nombre que l'appelant ne peut pas comparer au
premier.

### D3 — `compacted: true` est DÉCLARATIF, et le dire est la décision

La forme évidente est de lire `compacted` comme « une ligne `summary` a été
écrite ». **C'est faux, et le code ne peut pas le rendre vrai dans ce vol.**
`maybe_compact` rend `anyhow::Result<()>` : elle ne dit pas ce qu'elle a fait.
Elle relit `count_messages()` pour son propre compte et rend `Ok(())` sans rien
faire si le seuil n'est plus franchi.

Ce cas est atteignable. La compaction d'après tour tourne **hors du verrou**
(`handlers.rs:2390`), donc elle peut compacter entre notre `count_messages()` et
l'entrée dans `maybe_compact` ; la route rendrait alors `compacted: true` avec
zéro appel LLM et aucune écriture de son fait. Ce n'est pas un défaut neuf — la
TUI croise déjà la même course — et le corps du ticket le met hors vol.

La décision est donc : `compacted: true` signifie **« la compaction a été
demandée et n'a pas échoué »**, et le commentaire du champ le dit en toutes
lettres, plutôt que de laisser un futur appelant (V2d-3, V2d-4) bâtir une garde
cliente sur une attestation que la route ne donne pas. Le test d'AC4 observe
`calls_made() == 1` et la ligne `summary` **parce qu'aucune course n'existe dans
le harnais**, pas parce que la route le promet.

### D4 — Le seuil est LU depuis `compaction`, jamais récrit

`COMPACTION_THRESHOLD` est `pub(crate)` et `server` est dans le même crate :
`compaction::COMPACTION_THRESHOLD` se lit sans rien élargir. Un `50` littéral dans
le handler serait un second site pour une valeur dont le premier décide du
comportement réel — et la divergence serait **silencieuse dans le mauvais sens** :
si `compaction.rs` montait son seuil, la route rendrait `compacted: true` pendant
que `maybe_compact` ne ferait rien, c'est-à-dire exactement le mensonge que D3
s'efforce de borner.

La comparaison est `<= COMPACTION_THRESHOLD`, à l'identique de `compaction.rs:49`,
pour que les deux branches du programme s'accordent sur le cas limite des 50
messages pile.

### D5 — Deux sites de 500, et pas un message générique

La lecture (`count_messages`) et la compaction (`maybe_compact`) échouent toutes
deux en `anyhow::Error` et rendent toutes deux 500 : l'appelant n'a rien à faire
de différent selon laquelle. Mais le **texte** doit dire laquelle, et c'est le
précédent posé par `sessions.rs` avec ses deux aides `write_failed` et
`end_failed` : un 500 dont le message ne nomme pas l'opération est un 500 qu'on
diagnostique en relisant le code.

Une nuance que le commentaire doit porter : le 500 de `compaction_failed` couvre
**aussi l'échec du LLM**, parce que `maybe_compact` aplatit l'erreur du provider
dans son `anyhow::Error`. Ce n'est pas un oubli de classification — la route n'a
aucun moyen de distinguer un LLM en panne d'une écriture refusée sans inspecter
une chaîne de causes, et aucun des neuf critères ne demande deux codes distincts.

---

## 2. Les livrables, un par un

### 2.1 `crates/mika-agent/src/server/compact.rs` — le module neuf

En-tête de module sur le modèle de `sessions.rs` : **pourquoi c'est spirit qui
compacte**. Aujourd'hui `/compact` dans la TUI résume avec `app.llm`, le provider
construit par la TUI (`crates/mika-cli/src/init.rs`), qui n'est pas forcément
celui de l'agent servi — un agent dont la conversation est résumée par un modèle
qui n'est pas le sien reçoit un résumé dont la forme ne correspond pas à son
propre prompt. Cette route résume avec `agent_state.llm`, le provider figé à
l'`init_agent` de l'agent servi. L'en-tête dit aussi que le vol n'a **aucun
appelant** : la bascule est en V2d-4.

```rust
#[derive(Debug, Deserialize)]
pub struct CompactRequest {
    /// Agent servi. Toujours envoyé par le client, comme pour les routes de
    /// session ; une chaîne vide désigne l'agent par défaut, par `resolve_agent`.
    #[serde(default)]
    pub agent: String,
}

#[derive(Debug, Serialize)]
pub struct CompactResponse {
    /// Vrai quand la compaction a été **demandée et n'a pas échoué** — jamais
    /// une attestation qu'une ligne `summary` a été écrite (§ 1, D3).
    pub compacted: bool,
    /// Compte **d'avant**, dans les deux issues (§ 1, D2).
    pub messages: usize,
}
```

Handler `handle_compact(State(state), Json(req)) -> impl IntoResponse`, dans cet
ordre :

1. `resolve_agent(&req.agent)` ⇒ `None` : `404 {"error": …}`, **sans appel LLM**.
2. `agent_lock.clone().try_lock_owned()` ⇒ `Err` : `409 {"error": "Agent is busy. Try again when idle."}`, **sans appel LLM**. Le garde est nommé et vit jusqu'au retour (D1).
3. `count_messages()` ⇒ `Err` : 500 par `count_failed`.
4. `n <= compaction::COMPACTION_THRESHOLD` ⇒ `200 {"compacted": false, "messages": n}`, **sans appel LLM**.
5. `compaction::maybe_compact(&agent_state.db, agent_state.llm.as_ref())` ⇒ `Ok(())` : `200 {"compacted": true, "messages": n}` ; `Err` : 500 par `compaction_failed`.

Trois aides privées, sur le moule d'`ok` / `write_failed` / `end_failed` de
`sessions.rs` : `ok(compacted, messages)`, `count_failed(e)`,
`compaction_failed(e)`. Le 404 et le 409 sont construits en ligne, comme dans
`rewind.rs` — deux `format!` que rien ne partage avec une autre route.

### 2.2 `crates/mika-agent/src/server/mod.rs` — la déclaration et l'enregistrement

`mod compact;` à côté de `mod sessions;`, puis une ligne dans `session_routes`
(l. 392-398) :

```rust
.route("/compact", post(compact::handle_compact))
```

Le commentaire du groupe (l. 379-391) couvre déjà le « jeton interne seul » et le
raisonnement « une mutation n'est pas une lecture d'observabilité » ; il reçoit
**une** phrase : compacter remplace des lignes par un résumé, donc c'est une
mutation, la plus destructrice des trois. Pas de nouveau groupe, pas de nouveau
`route_layer`.

Le chemin est `/compact` et non `/sessions/…/compact`, parce que la compaction est
**scopée à l'agent et non à une session** : `count_messages`,
`load_messages_before_window` et `replace_with_summary` keyent toutes sur
`agent_id`. Un chemin sous `/sessions/{id}` promettrait une granularité qui
n'existe pas en base.

### 2.3 Les tests, dans `server/mod.rs § mod tests`

Posés après le bloc V2d-1b, sous un en-tête
`// ===== Compaction (mika#1727 V2d-2) =====`. Deux helpers locaux au bloc, et
aucun helper existant n'est modifié :

- `mika1727_state_compact(mock: Arc<MockLlmProvider>) -> AppState` — `test_state()`
  puis l'injection de `mika2671_b2_state` (`Arc::get_mut` sur l'entrée de la
  `DashMap`). Le mock est **construit par l'appelant**, parce que le test d'AC6 a
  besoin d'un `.error(…)` là où les autres veulent une réponse texte ; une fabrique
  qui construirait le mock elle-même ne servirait pas ce test.
- `mika1727_seed_messages(db: &AsyncDatabase, n: usize)` — `n` appels à
  `save_message("sess-compact-1727", "user", …, None)`. Aucune session n'est créée :
  le test `test_timeline_returns_data_after_message_insert` (l. 4402) établit que
  le harnais l'accepte.

| test | AC | ce qu'il assert |
|---|---|---|
| `mika1727_sans_jeton_la_compaction_rend_401` | 1 | 401 — et **pas** 404, ce qui prouve que la route existe |
| `mika1727_le_jeton_du_tableau_de_bord_est_refuse_a_la_compaction` | 1 | `test_state_with_dashboard_token()`, `Bearer dashboard-token-secret` ⇒ 401 |
| `mika1727_un_agent_inconnu_rend_404_sans_appeler_le_llm` | 2 | `{"agent":"agent-inexistant-1727"}` ⇒ 404 ; `mock.calls_made() == 0` |
| `mika1727_sous_le_seuil_la_route_ne_compacte_pas` | 3 | 50 messages semés ⇒ 200, `compacted == false`, `messages == 50` ; `calls_made() == 0` ; `count_messages()` inchangé. **50 pile**, pas 10 : le cas limite du `<=` de D4 |
| `mika1727_au_dela_du_seuil_la_route_compacte_avec_le_llm_de_lagent` | 4 | 51 messages ⇒ 200, `compacted == true`, `messages == 51` ; `calls_made() == 1` ; `load_conversation_summary()` ⇒ `Some(_)` ; `count_messages()` strictement inférieur à 51. **Les trois lectures de base sont le test** |
| `mika1727_un_agent_occupe_rend_409_sans_appeler_le_llm` | 5 | le test tient `agent_lock.clone().lock_owned().await` avant `test_app(state)` et le garde vivant ⇒ 409 ; `calls_made() == 0` |
| `mika1727_une_erreur_du_llm_rend_500_et_ne_touche_pas_la_base` | 6 | mock construit par `.error(…)` ; 51 messages ⇒ 500, `json["error"]` non vide ; `calls_made() == 1` ; `count_messages() == 51` et `load_conversation_summary()` ⇒ `None` |

Deux précisions que le pilote lira au site plutôt que de les deviner :

- la variante de `LlmError` passée à `.error(…)` n'a **aucune importance pour ce
  test** — seul le `Err` compte, et la route n'inspecte pas la cause (D5). Le
  pilote prend la première variante qui compile en lisant
  `mika-common/src/llm/error.rs` ; il n'ajoute pas de variante et ne change pas le
  mock.
- le test d'AC5 doit cloner l'`Arc` du `agent_lock` **avant** `test_app(state)`,
  qui consomme l'`AppState`. Le garde est lié à une variable nommée, pour la raison
  de D1.

Le couple `count_messages()` avant/après d'AC3 et d'AC6 est délibérément une
**paire mesurée**, pas un `== 50` en dur : si un jour le harnais sème des messages
de son côté, une assertion absolue mesurerait la vacuité du banc plutôt que
l'absence d'écriture.

---

## 3. Ordre des commits

**Deux commits, et c'est assumé.** Le handler n'a que deux branches de fond, et un
commit intermédiaire qui rendrait `compacted: false` au-dessus du seuil livrerait
une route qui ment — pire qu'un commit un peu plus gros. Chacun porte son code
**et** ses tests ; aucun test n'est reporté à un commit ultérieur ni à un vol
ultérieur.

**Commit 1 — la porte, le verrou et le seuil.**
`compact.rs` avec son en-tête, les deux types, le handler jusqu'à l'issue
`compacted: false` (étapes 1 à 4), l'aide `ok` et l'aide `count_failed` ;
`mod compact;` et la ligne de route dans `mod.rs` avec sa phrase de commentaire.
Tests : 401, jeton du tableau de bord, agent inconnu sans LLM, agent occupé sans
LLM, sous le seuil. Couvre AC1, AC2, AC3 et AC5.

**Commit 2 — la compaction et son 500.**
L'étape 5 : l'appel à `maybe_compact`, l'issue `compacted: true` avec le
commentaire de D3, et l'aide `compaction_failed` avec le commentaire de D5.
Tests : au-delà du seuil avec les trois lectures de base, erreur du LLM avec le
compte inchangé. Couvre AC4 et AC6.

Après **chaque** commit : `git diff --numstat origin/main...HEAD`, plan exclu. Si
le total dépasse 429, **halte** — pas de commit suivant, pas de PR, et le dernier
message dit « HALTE borne : N lignes pour 540 » avec le compte par commit, selon
la formule du corps du ticket.

Avant d'ouvrir la PR, dans cet ordre : `cargo test -p mika-agent`,
`cargo clippy -p mika-agent --all-targets` (vert, **sans `allow`**),
`bash scripts/verify-egress-manifest.sh --report` (AC8 — rouge ⇒ halte et on le
dit), `bash scripts/verify-pipeline.sh origin/main`.

---

## 4. Definition of Done

- `POST /api/v1/compact` servie dans `session_routes`, sous `require_internal_token` seul.
- Agent inconnu ⇒ `404 {"error": …}`, sans aucun appel LLM.
- Agent occupé (`try_lock_owned` échoue) ⇒ `409 {"error": …}`, sans aucun appel LLM.
- Verrou tenu pendant toute la compaction, appel LLM compris, par un garde nommé ; un commentaire dit ce pourquoi.
- `count_messages() <= compaction::COMPACTION_THRESHOLD` ⇒ `200 {"compacted": false, "messages": N}`, sans aucun appel LLM.
- Sinon `compaction::maybe_compact(&agent_state.db, agent_state.llm.as_ref())` puis `200 {"compacted": true, "messages": N}`, N étant le compte d'avant.
- Échec de lecture, d'écriture ou du LLM ⇒ `500 {"error": …}`, par deux aides dont le texte nomme l'opération.
- Le seuil est lu depuis `compaction`, jamais recopié ; aucune logique de compaction et aucun SQL n'est écrit.
- `cargo test -p mika-agent` et `cargo clippy -p mika-agent --all-targets` verts, sans `allow`.
- `scripts/verify-egress-manifest.sh --report` et `scripts/verify-pipeline.sh origin/main` verts.
- Diff limité à `crates/mika-agent/src/server/compact.rs` et `crates/mika-agent/src/server/mod.rs` (+ `docs/plans/` pour ce fichier). `crates/mika-cli/`, `crates/mika-a2a/`, `crates/mika-agent/src/compaction.rs`, `docs/egress/`, `docs/openapi/`, `.github/workflows/`, `.claude/`, `skills/bundled/`, `well_known_agents.rs`, `verdict_handler.rs`, `.mika/` : intacts.
- PR : `Refs mika#1727`. Aucun `Closes` / `Fixes` / `Resolves`, corps **et** messages de commit. `gh pr view <N> --json closingIssuesReferences` rend une liste vide — vérifié même, et surtout, si la PR est ouverte par le gabarit de reprise (`wip-rescue`), qui écrit `Closes #N` par défaut et a dû être corrigé à la main sur #3077 et #3081.
- Aucun ticket ouvert (gel jusqu'au 01/12). Reste et manques ⇒ commentaire sur #1727 ou corps de PR, avec `Tracked in: senara-solutions/mika#1727`.

## 5. Acceptance criteria

Transcrits du corps de #1727, § « Critères de V2d-2 ».

1. Sans jeton, `POST /api/v1/compact` rend 401. Le jeton du tableau de bord y est refusé. Un test pour chaque cas.
2. Pour un agent inconnu, la route rend 404, et aucun LLM n'est appelé. Un test le montre.
3. À 50 messages ou moins, la route rend `200 {compacted: false}`. Le `MockLlmProvider` de l'agent a `calls_made() == 0`, et le compte de messages est inchangé. Un test le montre.
4. Au-delà de 50 messages, la route rend `200 {compacted: true}`. **Le `MockLlmProvider` injecté dans l'`AgentState` de l'agent servi** a `calls_made() == 1`. Une ligne `summary` existe en base, et `count_messages()` a baissé. Un test lit la base.
5. Pendant que le test tient `agent_lock`, la route rend 409, et le LLM n'est pas appelé. Un test le montre.
6. Si le LLM de l'agent rend une erreur, la route rend 500, et le compte de messages est inchangé. Un test le montre.
7. `cargo clippy -p mika-agent --all-targets` est vert, sans `allow`. Seuls `server/compact.rs` et `server/mod.rs` changent. `crates/mika-cli/`, `crates/mika-a2a/`, `compaction.rs`, `docs/egress/` et `docs/openapi/` sont intacts.
8. Le pilote lance `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le trouve rouge, il s'arrête et le dit.
9. La PR porte `Refs mika#1727`, et aucun `Closes`, `Fixes` ni `Resolves`, y compris si elle est ouverte par le gabarit de reprise.

---

## 6. Ce que ce vol ne fait pas, et la dette nommée

**Hors périmètre, par le corps du ticket :**

- **La course avec la compaction d'après tour.** Elle tourne hors du verrou
  (`server/handlers.rs:2390`), donc elle peut croiser une compaction demandée par
  la route. Ce n'est pas un défaut neuf : la TUI croise déjà la même. Sa
  conséquence observable est écrite au § 1, D3 — `compacted: true` peut être rendu
  sans qu'aucun appel LLM n'ait eu lieu de notre fait.
- Les portes côté client et l'entrée d'egress — V2d-3. **Aucun client sortant
  n'est ouvert ici** (la route est servie, elle n'appelle rien hors du conteneur),
  donc `docs/egress/` reste intact.
- La bascule des appelants de la TUI — V2d-4. `crates/mika-cli/` n'est pas touché
  et la route n'a aucun consommateur à la fin du vol.

`Tracked in: senara-solutions/mika#1727`

**Dette créée par ce vol, et non traitée :**

- **Le 500 de `count_messages` n'a pas de test**, contrairement à celui du LLM.
  L'asymétrie a une cause nette et c'est ce qui la rend acceptable : le mock LLM
  **est** un point d'injection de panne (AC6 l'exploite), alors qu'il n'en existe
  aucun sur `AsyncDatabase` dans ce harnais — en fabriquer un demanderait de
  modifier `async_db.rs`, donc de sortir du périmètre « seuls `server/compact.rs`
  et `server/mod.rs` changent » (AC7). La garantie est de **forme** — un `match`
  dont le bras `Err` ne construit aucune réponse de succès — et non d'assertion.
  Aucun des neuf critères ne la demande.
- **`docs/openapi/mika-spirit.yaml` ne décrira pas cette route**, comme il ne
  décrit ni les trois routes rewind ni les deux routes de session de V2d-1.
  L'écart est antérieur à ce vol et son comblement toucherait un fichier exclu
  (toute modification serait copiée dans `crates/mika-agent/docs/openapi/`).
- **Rien ne mesure la durée de détention du verrou.** Le test d'AC5 prend le
  verrou avant l'appel et observe le 409 ; il ne verrait pas une régression où le
  garde serait relâché trop tôt (D1). La garantie est de forme, et le commentaire
  au site est ce qui la porte.

**Règle mika#2306 — gate N/A, et c'est une évaluation, pas un oubli.** Ce vol ne
livre aucun détecteur : pas de lint, pas de garde CI, pas de scan structurel, pas
de garde EndTurn, pas de validateur de schéma. Les sept tests sont des tests de
comportement — leur chemin de succès est « la route a rendu 401 / 404 / 409 / 200
/ 500, le mock a été appelé zéro ou une fois, et la base porte le compte attendu »,
et non « aucune violation trouvée » sur une population existante. Aucune violation
préexistante n'est donc à amnistier, à désarmer ou à remonter, et la section
`## Fire-Disposition` n'est pas écrite plutôt qu'inventée vide.

---

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/compact.rs` (en-tête de module, 2 types, handler, 3 aides) | 130 |
| `crates/mika-agent/src/server/mod.rs` (`mod compact;`, une route, une phrase de commentaire) | 5 |
| tests (`server/mod.rs § mod tests` — 7 tests, 2 helpers locaux) | 250 |

Total estimé : 385 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi un seul vol tient, et où est la marge.** 385 contre 429 : 44 lignes,
soit ~10 %. C'est plus mince que les 22 % de V2d-1b, et la mesure de ce vol-là est
ce qui calibre celui-ci. V2d-1b avait estimé 335 et livré **362** lignes ajoutées
(`git diff --numstat d33e84f3 62456241` : `sessions.rs` +129 pour 100 estimées,
`mod.rs` +233 pour 235 estimées) — l'estimation des tests est tombée juste, celle
du module a sous-estimé de ~30 %. Le poste de `compact.rs` est donc majoré en
conséquence : 130 pour un module neuf, contre les 123 lignes réelles du module
neuf de V2d-1a, qui portait une route et deux aides là où celui-ci en porte une et
trois.

Appliquer le même écart de 8 % au total donnerait ~416 lignes, encore sous la
borne. La marge est réelle mais étroite, et le corps du ticket dit ce qui se passe
si elle est mangée : le pilote **s'arrête après le commit qui franchit la borne**
et le dit ; il ne rabote pas les commentaires qui portent un pourquoi, et il ne
reporte aucun test. Le découpage en deux commits est ce qui rend cette halte fine
— le commit 1 porte les cinq tests les plus légers, de sorte qu'une halte après
lui laisse une route servie, cohérente et testée sur quatre critères.

La borne du ticket (540) et la borne par vol (429) ne disent pas la même chose :
**c'est la plus serrée qui mord**, donc 429 est la borne de travail, et 540 reste
la formule du message de halte que le corps du ticket impose au pilote. La borne
par PR (1000) ne mord pas. Pas de découpage en vols supplémentaires : la suite est
déjà découpée par le corps du ticket en V2d-3 et V2d-4.
