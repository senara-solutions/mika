# Vol V2c-1a-1 — le transport rewind de mika-spirit (mika#1727)

> **Branche :** `feat/1727/tui-v2c1a1-transport-rewind-de-spirit`
> **Base :** `e96c1d83` (`main` au moment du groom)
> **PR :** `Refs mika#1727`, jamais `Closes` — le ticket se ferme au merge de V4.

---

## Provenance : ce plan est une reprise, pas une re-mesure

Le groom de V2c-1a a mesuré son périmètre à **456 lignes** contre une borne de
429, et s'est arrêté sans recouper, en nommant deux coupes chiffrées. **Vincent a
choisi la coupe A** : le transport d'abord, les portes ensuite.

Le matériau est le plan de V2c-1a au commit `6a5d20a3` de
`feat/1727/tui-v2c1a-client-rewind-de-spirit`, lu par
`git show 6a5d20a3:docs/plans/2026-11-12-001-feat-1727-tui-v2c1a-client-rewind-de-spirit-plan.md`.
Ses faits F1 à F12, ses § 2.1, 2.2, 2.3, 2.6, 2.7, 2.8, son § 4.1 et ses tests T1
et T6 à T9 sont **repris sans re-mesure**, conformément au ticket. La branche de
V2c-1a n'est ni rebasée, ni réutilisée, ni supprimée.

**Trois choses sont neuves dans ce plan**, et elles sont les seules :

1. Le **critère 3 du ticket de ce vol** — « les six miroirs se désérialisent
   depuis la forme exacte que spirit sérialise » — n'a aucun test dans le
   matériau. Le T11 du matériau mesurait `ResultMirror` *à travers la porte
   `execute`*, et le ticket reporte T11 à V2c-1a-2. Le critère 3 demande autre
   chose : la désérialisation des miroirs **sans porte**. D'où T13, mesuré ici
   (§ 0, faits N1 à N3) et non supposé.
2. Le **« plus l'autre statut » du critère 2** — un test du bras par défaut
   d'`Unexpected`, absent du matériau (ses T6-T9 couvraient 422/404/409/401). D'où
   T12.
3. **T1 et T6-T9 sont adaptés à `post`**, puisqu'aucune porte n'est écrite ici :
   ils appellent `post` directement, là où le matériau passait par
   `resolve`/`preview`/`execute`.

---

## 0. Ce qui est mesuré, avant toute conception

### Les faits repris du matériau (F1 à F12, non re-mesurés)

**F1 — les trois routes existent et sont derrière le jeton interne.**
`crates/mika-agent/src/server/mod.rs:369-377` monte `/rewind/preview`,
`/rewind/execute` et `/rewind/resolve` en `post`, sous
`auth::require_internal_token`. Le préfixe est `/api/v1`. Le jeton de dashboard
n'y passe pas.

**F2 — les réponses sont `Serialize` seulement.** `RewindPreviewResponse`,
`RewindResolveResponse`, `RewindResultResponse` et leurs trois sous-structs ne
dérivent que `Serialize`. Aucune n'est désérialisable côté client, et
`crates/mika-agent/` est hors périmètre : **les miroirs côté CLI sont forcés**,
pas un choix de style.

**F3 — `ReversalPreviewResponse.action` est une forme `Debug`.**
`server/rewind.rs:217` écrit `action: format!("{:?}", r.action)`. Les trois
valeurs possibles sont `"Restore"`, `"Delete"`, `"Skip"`.

**F4 — le 422 de `preview` porte le corps d'un 200.** `server/rewind.rs:234-241`
construit la réponse complète, puis choisit le statut sur `resp.blocked`.

**F5 — le 422 d'`execute` porte un autre corps.** `server/rewind.rs:302-308` rend
`{"error": "<message contenant 'Rewind blocked'>"}`. Même refus, forme différente
de F4.

**F6 — le 404 a deux causes, sur les trois routes.** `resolve_agent` rend 404 pour
un agent inconnu, et `resolve` rend en plus 404 quand `find_recent_exchanges` ne
trouve rien. Le message du client ne peut donc pas affirmer l'une des deux causes.

**F7 — 409 sur les trois routes.** `agent_lock.clone().try_lock_owned()` sans
attente.

**F8 — un 400 est atteignable.** `validate_rewind_request` (session_id vide ou
> 1000 caractères, `after_message_id` négatif) et les deux garde-fous de
`resolve`. Le client doit donc avoir un cas par défaut, pas une énumération
fermée des statuts.

**F10 — le précédent de client.** `fetch_task_detail`
(`crates/mika-cli/src/commands/tasks.rs:653-676`) : `reqwest::Client::new()`,
`Bearer`, `timeout(5s)`, **statut lu avant le corps**, `Err` explicite sans repli.
Les serveurs factices du même fichier ne servent que des `get` — aucun ne sert de
POST, d'où le factice neuf.

**F11 — le lint d'egress scanne la construction de client.**
`scripts/lib/egress_manifest_lint.py:148-152` cherche `Client::new()` /
`Client::builder()` / `ClientBuilder::new(` dans les tranches de **production**
(les `#[cfg(test)] mod` sont coupés). Donc D1 exige une entrée pour le fichier
neuf, et D3 (« `client_site` fantôme ») exige que la construction du client vive
dans ce fichier, en production. Les URL étant `format!("{base}/api/v1/…")`, aucun
host littéral n'y figure et D2 reste muet — d'où `destination_source = "config"`.

**F12 — `mika-cli` a tout ce qu'il faut.** `reqwest` (`Cargo.toml:44`) et
`serde_json` (`:34`) en dépendances, `axum` en `dev-dependencies` (`:68`),
`mika-agent` en dépendance normale (`:24`). `commands` n'est pas réexporté par
`src/lib.rs` : le module vivra dans l'arbre du binaire, où `dead_code` tire
effectivement sur un `pub(crate)` non appelé. C'est ce qui donne du mordant à
l'`expect` du § 2.6.

**F9 n'est pas repris** : il porte sur `format_preview` et `trace_id`, qui
n'intéressent que la conversion du § 2.4 — hors de ce vol.

### Les faits neufs, mesurés sur `e96c1d83` pour le critère 3

**N1 — la forme sérialisée de l'aperçu et de ses trois imbriquées.**
`crates/mika-agent/src/server/rewind.rs:21-52` :

| struct serveur | champs et types |
|---|---|
| `RewindPreviewResponse` | `messages: Vec<MessagePreviewResponse>`, `reversals: Vec<ReversalPreviewResponse>`, `tasks: Vec<TaskPreviewResponse>`, `warnings: Vec<String>`, `blocked: bool`, `block_reason: Option<String>` |
| `MessagePreviewResponse` | `id: i64`, `role: String`, `content_snippet: String` |
| `ReversalPreviewResponse` | `audit_event_id: i64`, `tool_name: String`, `target_key: String`, `action: String`, `description: String` |
| `TaskPreviewResponse` | `task_id: String`, `label: String`, `actioned: bool` |

Aucun `#[serde(rename)]`, aucun `skip_serializing_if` : les noms de champs
sérialisés sont les noms Rust. C'est ce qui rend les miroirs écrivables de façon
mécanique, et c'est pourquoi T13 est un test de **forme**, pas de domaine.

**N2 — `RewindResolveResponse` n'est pas où le matériau le situait.** Elle est à
`server/rewind.rs:97-101`, hors de la tranche 21-62 que F2 citait, et porte
`session_id: String`, `after_message_id: i64`, `trace_ids: Vec<String>`. Les trois
champs du matériau sont les bons ; seule la ligne change. Noté pour que la
relecture ne cherche pas à la mauvaise adresse.

**N3 — `RewindResultResponse` porte bien huit champs** (`:54-64`) :
`messages_deleted`, `reversals_applied`, `reversals_skipped`, `tasks_deleted`,
`tasks_cancelled` (tous `usize`), `warnings: Vec<String>`,
`reversal_descriptions: Vec<String>`, `rewind_trace_id: String`. Les cinq
compteurs sont des `usize` : le miroir prend `usize`, pas `i64`, pour que le type
du miroir reste le type du fil.

**N4 — les types de domaine dont le § 2.2 dépend sont publics.**
`crates/mika-agent/src/lib.rs:60` déclare `pub mod rewind`, et
`crates/mika-agent/src/rewind.rs:62-69` déclare `pub struct RewindPreview` avec
les six champs de `RewindPreviewResponse`. Donc `mika_agent::rewind::RewindPreview`
est nommable depuis `mika-cli` **sans toucher `crates/mika-agent/`** — ce qui est
la condition pour que le variant `Blocked` du § 2.2 atterrisse dans ce vol sous sa
forme finale.

**N5 — la place du module dans `commands/mod.rs`.** `pub mod reminders;` est en
l. 22 et `pub mod setup;` en l. 23 : `pub mod rewind_remote;` s'insère entre les
deux.

**N6 — le modèle d'entrée d'egress.** `cli-spirit-tasks`
(`docs/egress/egress-manifest.toml:352-360`) donne les neuf clés et leur
alignement, `destination = "MIKA_SPIRIT_URL (défaut http://localhost:8080)"`,
`owner = "@samidarko"`.

---

## 1. La forme : un module, trois couches, aucune porte

Le module `crates/mika-cli/src/commands/rewind_remote.rs` porte **trois** des
quatre couches du matériau :

1. **les miroirs du fil** (§ 2.1) — ce que le serveur envoie, rien de plus ;
2. **`RewindRemoteIssue`** (§ 2.2) — ce que le serveur a répondu quand il n'a pas
   répondu un succès ;
3. **le transport** (§ 2.3) — un `post` unique, qui traduit les statuts que les
   trois routes partagent.

La quatrième — les trois portes et la conversion de l'aperçu — est V2c-1a-2.

**Ce qui ne vit pas ici.** L'URL de base et le jeton arrivent **par argument**,
pour la raison déjà écrite sur `TaskEventStream::open` et `fetch_task_detail` : un
client qui résout ses propres accès ne se pilote pas depuis un test. Le module ne
lit ni `MIKA_SPIRIT_URL` ni `MIKA_INTERNAL_TOKEN` — c'est V2c-1b qui les résoudra,
au site d'appel.

**Ce que la coupe A coûte, nommé d'avance.** Ce vol livre un module dont *aucun
item n'a d'appelant en production* : `post` n'est appelé que par ses tests, et les
miroirs par aucun code de ce vol (`post` rend le corps en texte brut, il ne
désérialise rien). C'est conforme au ticket — « aucune porte n'est écrite dans ce
vol, et aucun appelant ne change » — mais c'est ce qui fait de l'attribut du § 2.6
la pièce la plus délicate du vol, et non un détail de fin de parcours. D'où son
traitement en § 2.6 et sa place dans l'ordre des commits (§ 3).

---

## 2. Les livrables, un par un

### 2.1 Les miroirs du fil

Six structs `#[derive(Debug, Deserialize)]`, champs `pub(crate)`, en regard exact
de F2, N1, N2 et N3 :

| miroir | champs | source |
|---|---|---|
| `ResolveMirror` | `session_id: String`, `after_message_id: i64`, `trace_ids: Vec<String>` | `RewindResolveResponse` (N2) |
| `PreviewMirror` | `messages: Vec<MessageMirror>`, `reversals: Vec<ReversalMirror>`, `tasks: Vec<TaskMirror>`, `warnings: Vec<String>`, `blocked: bool`, `block_reason: Option<String>` | `RewindPreviewResponse` |
| `MessageMirror` | `id: i64`, `role: String`, `content_snippet: String` | `MessagePreviewResponse` |
| `ReversalMirror` | `audit_event_id: i64`, `tool_name: String`, `target_key: String`, `action: String`, `description: String` | `ReversalPreviewResponse` |
| `TaskMirror` | `task_id: String`, `label: String`, `actioned: bool` | `TaskPreviewResponse` |
| `ResultMirror` | les **huit** champs de N3, les cinq compteurs en `usize` | `RewindResultResponse` |

Un commentaire de bloc, trois lignes, dit le pourquoi non lisible dans le code :
les réponses du serveur sont `Serialize` seulement (F2), et `crates/mika-agent/`
est hors périmètre — ces miroirs ne sont pas un doublon évitable, ils sont le
contrat de fil, et ils ne portent **aucune** règle de domaine. `action` reste une
`String` : sa conversion vers `ReversalAction` est le § 2.4, hors de ce vol.

Les six portent l'attribut du § 2.6 : dans ce vol, aucun n'est lu en production.

### 2.2 `RewindRemoteIssue`

```rust
pub(crate) enum RewindRemoteIssue {
    Blocked { reason: String, preview: Option<Box<RewindPreview>> },
    NotFound { url: String },
    Busy,
    Denied,
    Unexpected { status: Option<u16>, url: String, body: String },
}
```

**Les cinq variants atterrissent ici, `Blocked` compris, et ce n'est pas une
extension de périmètre.** Le ticket nomme « § 2.2 » du matériau, qui est l'enum
entier ; `post` ne *produit* jamais `Blocked` (il rend le 422 à l'appelant), mais
le variant est la seule pièce du vocabulaire d'erreur que V2c-1a-2 aurait à
**redéfinir** plutôt qu'à construire s'il manquait ici — changer la forme d'un
variant est une modification, l'ajouter est additif. Et il est mesuré, pas
seulement déclaré : T14 construit `Blocked { preview: None }` et lit son `Display`,
donc aucun bras de l'enum n'atterrit sans test. N4 établit que
`mika_agent::rewind::RewindPreview` est nommable sans toucher `crates/mika-agent/`.

Cinq décisions, chacune mesurée :

- **`Blocked.preview` est un `Option<Box<…>>`.** `Option` parce que les deux 422
  diffèrent : celui de `preview` porte l'aperçu entier (F4), celui d'`execute` n'a
  qu'une raison (F5). `Box` parce que `RewindPreview` pèse ~128 octets (quatre
  `Vec`, un `bool`, un `Option<String>` — N4) et que le variant nu porterait l'enum
  au seuil où clippy `result_large_err` tire. **Ce lint mord dès ce vol** : `post`
  rend `Result<(u16, String), RewindRemoteIssue>`, donc la taille du variant `Err`
  est évaluée ici et non à l'arrivée des portes. Le `Box` est la condition d'un
  `cargo clippy` vert, pas une précaution de style.
- **`NotFound` porte son URL.** F6 : le 404 a deux causes et le client ne peut pas
  trancher. Le message de `Display` les nomme toutes deux au lieu d'en affirmer une
  — « no exchanges to rewind, or unknown agent ». Un message qui choisirait
  enverrait le lecteur chercher une session là où le vrai défaut est un nom
  d'agent.
- **`Denied` ne porte rien et son message nomme `MIKA_INTERNAL_TOKEN`.** Le nom de
  la variable est la moitié utile du message : sans lui, un 401 se lit comme une
  panne du serveur, avec lui comme un jeton à poser. Critère 2.
- **`Unexpected.status` est un `Option<u16>`.** `None` = aucune réponse reçue
  (panne de transport). Un `0` conventionnel se lirait comme un statut HTTP dans un
  journal ; l'`Option` ne ment pas.
- **`Display` est implémenté à la main**, cinq bras, ~16 lignes. Pas de
  `thiserror` : l'enum n'a pas de source à chaîner et n'est pas rendue à travers
  une frontière de bibliothèque.

### 2.3 Le transport — `post`

Une fonction privée au module, signature :

```rust
async fn post(base_url: &str, token: &str, route: &str, body: &serde_json::Value)
    -> Result<(u16, String), RewindRemoteIssue>
```

Elle construit l'URL
(`format!("{}/api/v1/rewind/{route}", base_url.trim_end_matches('/'))`), construit
le client (`reqwest::Client::new()` — **ici**, c'est ce que D3 exige, F11), pose
`Bearer`, sérialise le corps en JSON, délai de 5 s du précédent F10, puis :

| statut | ce que `post` rend |
|---|---|
| panne de transport | `Err(Unexpected { status: None, url, body: <cause> })` |
| 200, 422 | `Ok((statut, corps))` — l'appelant tranche |
| 401 | `Err(Denied)` |
| 404 | `Err(NotFound { url })` |
| 409 | `Err(Busy)` |
| tout autre (400, 500, …) | `Err(Unexpected { status: Some(s), url, body })` |

Deux points que le code ne dit pas et le commentaire doit dire :

- **Pourquoi 422 remonte à l'appelant** et n'est pas traduit ici : F4 et F5
  divergent, et seule la porte sait quelle forme attendre. Dans ce vol il n'y a pas
  de porte — la ligne reste donc la frontière que V2c-1a-2 viendra consommer, et
  non un report d'arbitrage.
- **Pourquoi le statut est lu avant le corps** : précédent F10 — un corps d'erreur
  désérialisé comme un succès est la panne exacte que ce module ne doit pas
  produire. Le bras par défaut existe pour F8.

`route` est un `&str` et non un enum : les trois valeurs sont fixées par les trois
appels de V2c-1a-2, et un enum à trois variants dont aucun n'a de producteur dans
ce vol serait trois items morts de plus sous l'attribut du § 2.6 pour zéro
propriété gagnée. T1 mesure les trois chemins sur la socket, ce qu'un enum
n'aurait pas mesuré mieux.

### 2.6 L'attribut de non-appel

Chaque item que ce vol n'appelle pas en production porte :

```rust
#[cfg_attr(not(test), expect(dead_code, reason = "câblé par V2c-1a-2, mika#1727"))]
```

Le ticket fixe le texte de `reason` à **V2c-1a-2** (et non V2c-1b comme le
matériau, dont les portes partaient plus loin) : c'est le vol qui câble, donc le
vol qui retirera l'attribut.

**Candidats** : les six miroirs (§ 2.1), `RewindRemoteIssue` et ses variants,
`post`. C'est-à-dire, dans ce vol, **tout le module** — voir § 1.

**Le piège à mesurer, pas à supposer.** `rustc` ne signale `dead_code` que sur la
**racine** d'un sous-graphe inatteignable. Ici `post` est la seule racine évidente,
et l'enum comme les miroirs ne sont peut-être atteints par rien d'autre que lui :
poser l'attribut sur un item que le lint ne vise pas le transforme en
`unfulfilled_lint_expectation`, un warning. C'est-à-dire que l'attribut rougit dans
les deux sens, ce qui est précisément sa valeur.

**Conduite, en deux temps.** Le pilote pose l'attribut à la naissance de chaque
item, dans le commit qui le livre — un commit qui laisse un `dead_code` nu
derrière lui n'est pas vert au sens du § 3. Puis, au commit 4, il lance
`cargo clippy -p mika-cli --all-targets` et **retire** chaque attribut
insatisfait. `--all-targets` est ce qui compile les deux faces du `cfg_attr` : la
cible de test, où l'attribut n'existe pas et l'item est vivant, et la cible
normale, où il est mort. Il ne substitue **jamais** `allow` à `expect` pour faire
taire le warning : `allow` est muet au câblage, et c'est l'obligation faite à
V2c-1a-2 de retirer l'attribut qui justifie son existence.

### 2.7 L'entrée d'egress

Une entrée neuve dans `docs/egress/egress-manifest.toml`, sur le modèle de
`cli-spirit-tasks` (N6), et **rien d'autre** dans ce fichier :

```toml
[[sink]]
id                 = "cli-spirit-rewind"
destination        = "MIKA_SPIRIT_URL (défaut http://localhost:8080)"
destination_source = "config"
class              = "internal"
data               = "un POST générique vers les routes rewind de mika-spirit (/api/v1/rewind/{resolve,preview,execute}) : le corps JSON que l'appelant fournit — identifiant de session, identifiant de message d'ancrage, nom d'agent, nombre d'échanges — et le jeton interne en en-tête Authorization"
logged             = false
client_site        = "crates/mika-cli/src/commands/rewind_remote.rs"
call_site          = "crates/mika-cli/src/commands/rewind_remote.rs"
owner              = "@samidarko"
```

`client_site` et `call_site` nomment le même fichier : dans ce vol le client est
construit et appelé au même endroit (F11 D3). V2c-1b fera porter `call_site` sur
`tui/commands/handlers.rs` quand l'appel y naîtra.

Le champ `data` décrit **ce que ce vol envoie** — un POST générique dont le corps
vient de l'appelant — et non les trois corps nommés que le matériau décrivait, qui
sont ceux des portes. Décrire des champs qu'aucun code de ce vol ne pose rendrait
l'entrée fausse le jour où on la relit pour savoir ce qui sort.

**Ce chemin est sous CODEOWNERS `@samidarko`** : la PR attend sa revue en plus de
la QA.

### 2.8 La déclaration du module

Une ligne dans `crates/mika-cli/src/commands/mod.rs` : `pub mod rewind_remote;`,
à sa place alphabétique entre `reminders` et `setup` (N5). Plus un en-tête de
module `//!` qui dit ce que le module est (le transport des routes rewind de
spirit) et ce qu'il n'est pas encore (aucune porte, aucun appelant — V2c-1a-2).

---

## 3. Ordre des commits

Le pilote commite après chaque garde verte avec son test, pas en fin de vol. Les
attributs du § 2.6 sont posés **dans le commit qui livre l'item**, puis ajustés par
la mesure au commit 4.

| # | contenu | tests qui atterrissent avec |
|---|---|---|
| 1 | § 2.8 déclaration + en-tête, § 2.1 les six miroirs | T13 (désérialisation des six depuis la forme de N1-N3) |
| 2 | § 2.2 enum + `Display` | T14 (le bras `Blocked`, seul sans producteur) |
| 3 | § 2.3 `post`, § 4.1 factice | T1 (socket), T6-T9 (401/404/409/422), T12 (autre statut) |
| 4 | § 2.6 mesure et ajustement des attributs, § 2.7 entrée d'egress | `cargo clippy -p mika-cli --all-targets` vert ; `bash scripts/verify-egress-manifest.sh --report` vert |

**Pourquoi les miroirs d'abord et non `post` d'abord.** Les miroirs sont la seule
couche dont le test ne dépend de rien (T13 est un `serde_json::from_str` sur un
littéral) : elle est verte en un commit sans factice. `post` ne tient pas sans le
factice, d'où leur commit commun au 3 — un factice qui atterrirait seul serait du
code sans mesure.

**Pourquoi l'egress en dernier.** Le lint d'egress scanne la construction de
client en production (F11) : l'entrée ne peut être vérifiée verte qu'une fois le
`Client::new()` du § 2.3 présent. Posée avant, elle désignerait un `client_site`
fantôme — exactement ce que D3 refuse.

---

## 4. Le jeu de tests

Tous dans `#[cfg(test)] mod tests` **à la fin** du fichier neuf — à la fin, parce
que `production_slices` coupe au premier `#[cfg(test)]` et qu'un item de production
qui reprendrait après la coupure sortirait de l'inventaire d'egress
(`audit_cut_holes`, F11).

### 4.1 `fake_spirit` — le factice POST

Un `axum::Router` à **`fallback`** unique, et non trois routes : le gestionnaire
lit `method`, `uri().path()`, l'en-tête `authorization` et le corps, les pousse
dans un témoin `Arc<Mutex<Vec<RequeteRecue>>>`, puis répond avec le
`(statut, corps)` que le test a associé à ce chemin. Le `fallback` est ce qui rend
un seul factice capable de servir les trois routes et de prouver que le client a
visé la bonne — une route déclarée explicitement rendrait 404 sur une faute de
chemin, ce qui se lirait comme un cas métier (F6) au lieu d'une erreur de client.

Le témoin fait de « le jeton part en `Bearer` » et « le corps arrive intact » des
mesures prises **sur la socket**, non une relecture du code. C'est l'exigence
explicite du critère 1.

### 4.2 Les huit tests

Les noms suivent la convention du dépôt (préfixe de ticket, énoncé en français de
la propriété tenue).

| test | critère | ce qu'il mesure |
|---|---|---|
| T1 `mika1727_post_vise_les_trois_routes_avec_son_jeton` | 1 | `post` appelé trois fois sur un même factice, `route` valant `resolve`, `preview`, `execute` : méthode `POST`, les trois chemins `/api/v1/rewind/{route}`, `authorization: Bearer <jeton>`, et le corps JSON donné **relu intact** sur la socket |
| T6 `mika1727_un_422_remonte_intact_a_lappelant` | 2 | 422 avec un corps quelconque → `Ok((422, corps))`, corps **égal** octet pour octet à celui du serveur — c'est la moitié « non traduit ici » du § 2.3 |
| T7 `mika1727_un_404_nomme_ses_deux_causes` | 2 | 404 → `NotFound`, et `to_string()` nomme les deux causes de F6 (session sans échange **et** agent inconnu) |
| T8 `mika1727_un_409_est_un_verrou` | 2 | 409 → `Busy` |
| T9 `mika1727_un_401_nomme_la_variable_a_poser` | 2 | 401 → `Denied`, et `to_string()` contient `MIKA_INTERNAL_TOKEN` |
| T12 `mika1727_un_statut_hors_contrat_porte_son_statut_et_son_url` | 2 | 500 → `Unexpected { status: Some(500), … }`, et le message porte **le statut et l'URL** ; c'est le bras que F8 rend atteignable |
| T13 `mika1727_les_miroirs_se_lisent_depuis_la_forme_du_serveur` | 3 | un littéral JSON à la forme exacte de N1 → `PreviewMirror` avec ses **trois** imbriquées non vides (un message, une réversion, une tâche), tous champs assertés ; plus `ResolveMirror` (N2) et les huit champs de `ResultMirror` (N3) |
| T14 `mika1727_le_bras_bloque_se_lit_sans_apercu` | — | `Blocked { reason, preview: None }` construit à la main → `to_string()` porte la raison. Le seul bras de l'enum qu'aucun statut ne produit dans ce vol : sans ce test il atterrirait sans mesure (§ 2.2) |

**T6 n'est pas le T6 du matériau**, et la différence est le cœur de la coupe A : le
matériau mesurait un 422 *traduit en `Blocked` par la porte `preview`* ; ici il
mesure un 422 *non traduit*, rendu intact. La traduction est V2c-1a-2 et son test
y naîtra.

**T12 et T13 sont neufs** (voir § *Provenance*). T13 couvre cinq des six miroirs
en profondeur et le sixième — `ResultMirror` — par ses huit champs ; la mesure de
`ResultMirror` **à travers la porte `execute`** reste le T11 du matériau, reporté à
V2c-1a-2 par le ticket. Le critère 3 dit « au moins pour `PreviewMirror` » : ce
plan va au-delà du minimum demandé sans rapatrier le test que le ticket exclut.

**Ce que ce vol ne teste pas, et qui n'est pas un oubli :** T2 à T5 (les trois
actions et l'action inconnue), T10 (`originating_session_id` présent/absent) et
T11 (le résultat à travers `execute`). Ils mesurent la conversion et les portes,
que le ticket renvoie à V2c-1a-2. `Tracked in: senara-solutions/mika#1727`.

---

## Fire-Disposition

Ce plan livre des détecteurs — huit tests (mika#2306), et l'attribut du § 2.6,
dont le chemin de succès est « cet item est bien encore mort ».

**Option retenue : (a) atterrissage armé, et pour l'attribut du § 2.6, une
exception nommée, grep-visible et auto-nettoyante.**

Les huit tests mesurent une surface **entièrement neuve de ce vol** : un module qui
n'existe pas sur `e96c1d83`. Aucun n'est un scan de source ni un parcours de
corpus, aucun ne peut donc révéler une violation préexistante. Il n'y a **rien à
amnistier**, et aucune allowlist à ouvrir, pas même vide. Les huit atterrissent
armés et verts dans les commits qui les justifient (§ 3).

Pour l'attribut du § 2.6, l'exception est l'attribut lui-même, et elle porte les
trois propriétés que mika#1574 (a) demande :

- **elle nomme la donnée précise** — l'item exact qu'elle couvre, un attribut par
  item, jamais un `#![allow(dead_code)]` de module ;
- **elle référence un suivi** — `reason = "câblé par V2c-1a-2, mika#1727"`,
  grep-visible par `rg 'V2c-1a-2' crates/mika-cli/src/commands/rewind_remote.rs` ;
- **elle rougit quand elle devient stale** — c'est le choix d'`expect` plutôt
  qu'`allow` : au câblage par V2c-1a-2, l'attribut devient
  `unfulfilled_lint_expectation` et le build le dit. Le § 2.6 interdit
  explicitement de retomber sur `allow`, qui serait muet au câblage.

**Le risque propre à ce vol, nommé.** Parce que la coupe A laisse *tout* le module
sans appelant (§ 1), l'attribut y est posé sur bien plus d'items que dans le
matériau — et chaque attribut de trop est un warning. La conduite du § 2.6 (poser,
mesurer au commit 4, retirer l'insatisfait) est donc une étape de travail à part
entière, pas une vérification de fin de course. Si la mesure montre qu'un seul
attribut suffit — sur `post`, racine unique — le pilote retire les autres : **le
plan prescrit la mesure, pas le nombre.**

Si un test rougit à l'atterrissage, la conduite est celle de mika#2201 : corriger
le code, **pas** élargir la mesure. Trois élargissements tentants sont nommés pour
être refusés — remplacer l'égalité octet pour octet de T6 par un `contains`, ce qui
effacerait la propriété « non traduit » du § 2.3 ; faire de T1 un test sur un seul
chemin parce que les trois se ressemblent, alors que c'est la *distinction* des
trois que le critère 1 demande ; et lire le `MIKA_INTERNAL_TOKEN` de T9 dans le
code au lieu du `Display`, ce qui ferait du test une tautologie.

Les options (b) et (c) sont écartées. **(b) expédier désarmé** (`#[ignore]`) est
refusé par le ticket lui-même — « chaque garde arrive avec ses tests, dans le même
commit ; les tests ne se reportent jamais à un vol suivant, même si le code
reste ». **(c) halte-et-remontée** n'a pas lieu d'être : le périmètre de ce vol
passe sous la borne (§ *Taille estimée*), ce qui est précisément ce que la coupe A
de Vincent a produit.

---

## Definition of Done

- [ ] `crates/mika-cli/src/commands/rewind_remote.rs` existe et porte les six
      miroirs, `RewindRemoteIssue` + `Display` à cinq bras, et `post`.
- [ ] **Aucune porte, aucune conversion** : ni `resolve`, ni `preview`, ni
      `execute`, ni `preview_to_domain`, ni `reversal_action_from_label`.
- [ ] `pub mod rewind_remote;` déclaré dans `commands/mod.rs` entre `reminders` et
      `setup`.
- [ ] Les huit tests du § 4.2 passent, contre le factice `fallback` du § 4.1.
- [ ] `cargo clippy -p mika-cli --all-targets` vert, sans `allow(dead_code)` et
      sans `unfulfilled_lint_expectation`.
- [ ] `cargo fmt --check` vert, `cargo test -p mika-cli` vert.
- [ ] `docs/egress/egress-manifest.toml` porte `cli-spirit-rewind` et rien d'autre
      de neuf ; `bash scripts/verify-egress-manifest.sh --report` vert. **Lancé
      avant d'ouvrir la PR ; s'il est rouge, le pilote s'arrête et le dit.**
- [ ] `bash scripts/verify-pipeline.sh origin/main` vert.
- [ ] `git diff --name-only origin/main...HEAD` ne contient **aucun** de :
      `crates/mika-cli/src/tui/commands/handlers.rs`,
      `crates/mika-cli/src/tui/app.rs`, `crates/mika-cli/src/commands/chat.rs`,
      quoi que ce soit sous `crates/mika-agent/` ou `crates/mika-a2a/`, ni aucun
      fichier du plan de contrôle (`.github/workflows/`, `.claude/`,
      `skills/bundled/`, `well_known_agents.rs`, `.mika/`).
- [ ] `git diff --numstat origin/main...HEAD` hors `docs/plans/` ≤ **429** lignes
      ajoutées.
- [ ] La PR porte `Refs mika#1727`, et ni `Closes`, ni `Fixes`, ni `Resolves`, ni
      dans le corps ni dans un message de commit.
- [ ] Aucun ticket ouvert (gel jusqu'au 01/12) ; tout reste va en commentaire, avec
      `Tracked in: senara-solutions/mika#1727`.

---

## Acceptance criteria

Transcrits du corps de mika#1727, § *Critères de V2c-1a-1* :

1. `post` envoie un POST sur `/api/v1/rewind/{route}` pour chacune des trois
   routes, avec `Bearer <jeton>` et le corps JSON qu'on lui donne. Un test (T1,
   adapté à `post` puisque les portes ne sont pas dans ce vol) le lit **sur la
   socket** du factice, pas dans le code.
2. Un 422 remonte à l'appelant comme `Ok((422, corps))`, corps intact. Un 404 rend
   `NotFound`, dont le message nomme les deux causes. Un 409 rend `Busy`. Un 401
   rend `Denied`, dont `to_string()` contient `MIKA_INTERNAL_TOKEN`. Un autre
   statut rend `Unexpected` avec le statut et l'URL. Chaque cas a son test (T6 à T9
   adaptés à `post`, plus l'autre statut).
3. Les six miroirs se désérialisent depuis la forme exacte que spirit sérialise. Un
   test le montre au moins pour `PreviewMirror`, avec ses trois structures
   imbriquées.
4. `cargo clippy -p mika-cli --all-targets` est vert, sans `allow(dead_code)` et
   sans attente insatisfaite.
5. `handlers.rs`, `app.rs`, `chat.rs`, `crates/mika-agent/` et `crates/mika-a2a/`
   ne sont pas modifiés. `docs/egress/` ne change que par l'entrée
   `cli-spirit-rewind`. Le pilote lance
   `bash scripts/verify-egress-manifest.sh --report` avant d'ouvrir la PR. S'il le
   trouve rouge, il s'arrête et le dit. La PR porte `Refs mika#1727` et aucun
   `Closes`, `Fixes` ni `Resolves`.

---

## 5. Ce que ce vol ne fait pas

- **Il n'écrit aucune porte.** Ni `resolve`, ni `preview`, ni `execute` — § 2.5 du
  matériau est V2c-1a-2.
- **Il ne convertit rien vers le domaine.** § 2.4 du matériau (et son fait F9) est
  V2c-1a-2. `ReversalMirror.action` reste la `String` de F3.
- **Il ne câble rien.** `handle_rewind_impl` continue d'appeler
  `mika_agent::rewind` sur la base locale. V2c-1b le retourne, et refusera sans
  `MIKA_INTERNAL_TOKEN`.
- **Il ne crée pas de session par spirit** et ne touche pas `/compact` — deux vols
  distincts, chacun mesuré à son tour.
- **Il ne résout ni URL ni jeton.** Pas de lecture d'environnement dans ce module
  (§ 1).
- **Il ne touche pas au serveur.** Les trois routes existent (F1) ; aucun
  `Deserialize` n'est ajouté côté `mika-agent`, ce qui aurait économisé les miroirs
  mais violé le périmètre.
- **Il n'ouvre aucun ticket** (gel jusqu'au 01/12).

---

## 6. La dette, nommée et non traitée

- **Les miroirs sont une seconde déclaration du contrat de fil.** Rien ne les tient
  en lockstep avec `server/rewind.rs` : un champ renommé côté serveur se verrait à
  l'exécution, pas à la compilation. T13 mesure la forme d'aujourd'hui, il ne la
  verrouille pas pour demain. Le remède propre — un type partagé
  `Serialize + Deserialize` dans `mika-common`, ou `Deserialize` sur les types du
  serveur — demande de toucher `crates/mika-agent/`, que ce vol exclut. À porter en
  commentaire sur mika#1727 à l'ouverture de la PR.
- **Le 422 d'`execute` n'a pas la forme de celui de `preview`** (F4 vs F5). Le
  client s'y adaptera en V2c-1a-2 ; c'est le serveur qui est irrégulier. Même
  canal.
- **`RewindRemoteIssue::NotFound` reste ambigu** par construction (F6). Le serveur
  pourrait distinguer les deux causes par un code ; il ne le fait pas. Même canal.
- **Un vol entier sous l'attribut du § 2.6.** C'est le coût nommé de la coupe A : le
  module est inerte jusqu'à V2c-1a-2, et sa seule garantie d'être câblé un jour est
  l'`expect` qui rougira. Même canal.

`Tracked in: senara-solutions/mika#1727`

---

## Taille estimée

Lignes de code ajoutées, commentaires et tests compris, dans l'unité de
`git diff --numstat origin/main...HEAD`. **L'entrée d'egress est comptée dans le
total**, comme le ticket l'exige (« son `Total estimé :` dans cette même unité,
entrée d'egress comprise »), bien qu'elle vive sous `docs/` : quand les deux règles
se croisent, celle du ticket mord.

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `commands/mod.rs` déclaration + en-tête `//!` du module (§ 2.8) | 13 |
| six miroirs + le commentaire de bloc (§ 2.1) | 53 |
| `RewindRemoteIssue` + `Display` + les cinq commentaires de décision (§ 2.2) | 35 |
| `post` + ses deux commentaires (§ 2.3) | 38 |
| `fake_spirit` à `fallback`, témoin et table de réponses (§ 4.1) | 34 |
| T1 — les trois routes sur la socket | 28 |
| T6-T9 — 422 intact, 404, 409, 401 | 36 |
| T12 — statut hors contrat | 10 |
| T13 — désérialisation des six miroirs | 30 |
| T14 — le bras `Blocked` | 8 |
| attributs `cfg_attr`/`expect` (§ 2.6) | 14 |
| `docs/egress/egress-manifest.toml` entrée `cli-spirit-rewind` (§ 2.7) | 10 |

Total estimé : 309 lignes
Plafond de tours : 200 (`loop-substrate`)
Borne par vol : 429 lignes estimées — (200 − 27 − 30) × 9 / 3

**Pourquoi ce vol tient en un seul vol.** 309 contre 429 : 120 lignes de marge,
28 %. Et la marge résiste au biais d'estimation que le matériau avait mesuré sur ce
ticket — V2b-2b-1, le vol le plus proche en forme (un client HTTP neuf + un serveur
factice + ses tests), avait estimé 241 pour **279** livrées, soit +16 %. Appliqué
ici, 309 estimées donnent ~358 réelles, encore 71 lignes sous la borne. C'est
exactement le calcul que le groom de V2c-1a avait fait pour conclure l'inverse sur
456 ; il conclut ici que la coupe A de Vincent a tenu sa promesse.

La suite est nommée et non estimée ici — l'architecte juge le périmètre du vol
courant, jamais la somme des vols :

- **V2c-1a-2** : la conversion de l'aperçu (§ 2.4 du matériau), les portes
  `resolve`, `preview` et `execute` (§ 2.5), et les tests T2 à T5, T10 et T11.
  Estimé 217 par la coupe A du matériau.
- **V2c-1b** : `/rewind`, `/rewind to` et `/undo` passent par ce client, et
  refusent sans `MIKA_INTERNAL_TOKEN`.
- puis la création de session et `/compact` par spirit, chacun mesuré à son tour ;
- puis V3 et V4. Le ticket se ferme au merge de V4.

`Tracked in: senara-solutions/mika#1727`
