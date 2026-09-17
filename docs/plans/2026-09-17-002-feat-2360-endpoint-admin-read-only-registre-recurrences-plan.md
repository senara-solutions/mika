# Plan — mika#2360 : endpoint admin READ-ONLY pour lister le registre des récurrences d'un tenant

- **Ticket :** `mika issue#2360`
- **Type :** feat
- **Priorité :** p1 — débloque l'inspection du tenant canari Al (`mika issue#2358`)
- **Repos touchés :** `mika` (mika-agent + mika-gateway). Companion `mika-cloud` : ticket séparé (voir § Companion).

---

## Le besoin, et ce que la lecture du code y ajoute

Inspecter le registre des tâches récurrentes d'un tenant **sans `kubectl exec`**. Le chemin
validé par Vincent le 2026-09-17 est un endpoint admin read-only proxifié par la gateway.

Le ticket décrit correctement l'état du code. Quatre lectures le **déplacent**, et ce sont
elles qui décident la conception. Chacune est vérifiée dans l'arbre, avec sa référence.

### T1 — « Réutiliser `handle_tasks_list` » viole l'AC4 telle quelle

`TaskResponse` (`server/dashboard.rs:559-585`, la forme *liste*) porte deux champs que le
ticket interdit explicitement :

```rust
pub action_config_preview: Option<String>,   // db::truncate_chars(&t.action_config, 200)
pub result_preview: Option<String>,          // db::truncate_chars(r, 200)
```

Et `action_config` d'une récurrence **est** du contenu utilisateur dès que
`action_type = send_message` : `dispatcher.rs:377` lit `config["text"]` et l'envoie tel quel
au canal. Une récurrence créée par Al via le LLM (« rappelle-moi mes médicaments à 8h ») a
son texte intégral dans `action_config`, dont `TaskResponse` publierait les 200 premiers
caractères.

Ce n'est pas une hypothèse sur un usage futur : c'est la forme nominale d'un rappel
récurrent sur un tenant famille, c'est-à-dire exactement la population qu'on va inspecter.
**Brancher `GET /api/v1/tasks?trigger_type=recurring` derrière la gateway livrerait AC1 et
casserait AC4 dans le même commit.**

Corollaire de conception, qui vaut plus que le cas d'espèce : **AC4 doit être tenue par la
requête SQL, pas par une conversion en aval.** Retirer deux champs d'une struct partagée est
un *opt-out* — il se rouvre en silence le jour où quelqu'un ajoute un champ à `Task` ou à
`TaskResponse` pour une raison sans rapport, et aucun test de ce ticket ne rougira. Une
projection de colonnes nommées est un *opt-in* : ce qui n'est pas écrit dans le `SELECT`
n'existe pas dans la réponse, quoi que la table gagne ensuite.

### T2 — l'ensemble de champs demandé ne permet pas de trancher D1/D2/D3

Le ticket demande `{label, trigger_type, cron_expr, next_fire_at, status, created_at,
updated_at}` et se donne pour mission de débloquer le diagnostic de `mika issue#2358`. Or
l'état « une récurrence attendue n'existe pas » a **deux causes différentes** dans ce code,
et cet ensemble de champs ne les sépare pas :

- **Absence pure** — rien n'a jamais enregistré ce label.
- **Veto zombie mika#1742** — une ligne `failed | cancelled | expired` de moins de 24 h sur
  le même `(agent_id, label)` **refuse** la re-registration à chaque démarrage
  (`db.rs:38-49`, `RECURRING_ZOMBIE_GRACE_HOURS = 24`). Trois marqueurs `metadata` modulent
  ce veto : `$.config_cancel_reverted` (mika#2271), `$.unknown_trigger_death` et
  `$.unknown_trigger_lift_spent` (mika#2337).

Deux conséquences.

1. **Les lignes non-`active` doivent être visibles.** Un endpoint qui filtrerait
   `status = 'active'` par défaut rendrait un registre vide et lisible comme « pas de
   récurrence », alors que le registre contiendrait précisément la ligne morte qui bloque.
   Donc : **aucun filtre de statut par défaut**, et le tri place les lignes d'un même label
   côte à côte (voir T4).
2. **Deux champs dérivés sont ajoutés au contrat**, tous deux métadonnées pures :
   `action_type` (déjà dans la table, aucun contenu) et `zombie_veto_active: bool` — un
   booléen **calculé en SQL** qui dit si cette ligne, à l'instant de la lecture, arme le veto
   de re-registration. L'opérateur n'a pas à réimplémenter la fenêtre de 24 h ni la logique
   des trois marqueurs dans sa tête pour lire la réponse.

   **Le blob `metadata` n'est jamais exposé, ni tronqué, ni filtré.** Il porte du texte libre
   sur d'autres familles de tâches ; une projection fermée en booléen est la seule forme qui
   respecte AC4 par construction. Le coût est nommé : si un quatrième marqueur apparaît un
   jour, le booléen se recalcule dans la même requête — il ne se devine pas côté opérateur.

C'est un ajout, pas une réécriture : le contrat du ticket est un sous-ensemble strict de ce
qui est livré, et l'ajout ne franchit aucune ligne qu'AC4 trace.

### T3 — l'auth : deux frontières, deux jetons, et une garantie qu'il faut nommer honnêtement

`/admin/*` **existe déjà** sur la gateway (`routes.rs:288-317` : `POST /admin/customers`,
`GET /admin/customers/{id}`, `POST /admin/customers/{id}/unlink`) et il est protégé par
`require_bearer_token` (`routes.rs:1899`), qui compare à `state.internal_token` — le jeton
write. Le ticket demande explicitement « PAS l'INTERNAL_TOKEN write ».

Le chemin comporte **deux frontières distinctes**, et les confondre est l'erreur à éviter :

| Frontière | Sens | Jeton | Changement |
|---|---|---|---|
| opérateur → gateway | externe, hors cluster | **nouveau** `MIKA_GATEWAY_ADMIN_READ_TOKEN` | ce ticket |
| gateway → tenant | interne, ClusterIP | `MIKA_INTERNAL_TOKEN` (inchangé) | aucun |

Le hop interne reste sur l'internal token : c'est la façon dont `handle_a2a_proxy`
(`a2a_routes.rs:101-107`) parle déjà aux pods, et le tenant n'a aucun moyen d'authentifier un
opérateur. Ce que le ticket veut réellement est sur la frontière externe : **l'opérateur qui
inspecte n'a plus besoin de détenir le jeton d'écriture.**

**La garantie inverse n'existe pas et le plan refuse de la suggérer.** Le porteur de
l'INTERNAL_TOKEN peut déjà créer un client (`POST /admin/customers`) et délier un binding
Telegram (`.../unlink`) : il est superuser sur cette gateway. Lui refuser la route de lecture
ne lui retire aucun privilège — ça change qui a besoin de quoi, pas qui peut quoi. Le gain
est réel et il est **organisationnel** : un jeton de lecture peut être distribué, tourné et
révoqué sans toucher au jeton d'écriture.

Cela dit, **la lettre de l'AC2 est retenue** : l'INTERNAL_TOKEN présenté sur la route de
lecture est refusé. Le refus coûte trois lignes et rend l'AC2 testable ; l'accepter aurait
demandé de réécrire l'AC. La phrase ci-dessus existe pour qu'on ne lise pas ce refus comme
une barrière qu'il n'est pas.

**Divergence à arbitrer — 401 contre 403.** L'AC2 demande `403` pour trois cas que la
sémantique HTTP sépare, et que les cinq middlewares de la maison séparent déjà
(`server/auth.rs:33`, `:67`, `routes.rs:1914` rendent tous `401`) :

| Cas | Proposé | AC2 littérale |
|---|---|---|
| en-tête absent / illisible | `401` | `403` |
| jeton inconnu | `401` | `403` |
| jeton **reconnu** comme l'INTERNAL_TOKEN write | **`403`** | `403` |

Le troisième cas — le seul qui soit réellement « mauvais scope » — rend bien `403` :
authentifié, pas autorisé. Les deux premiers ne sont pas authentifiés du tout, et `401` est à
la fois la sémantique HTTP et l'invariant des cinq middlewares existants. Rendre `403` sur un
en-tête absent apprendrait à un client qu'un jeton absent et un jeton refusé sont le même
événement — ils ne le sont pas.

**Ceci est une divergence assumée avec la lettre de l'AC2, soumise à l'arbitrage de
l'architecte.** Si elle est refusée, la bascule est d'une ligne par branche et les tests de
l'AC2 se lisent dans l'autre sens ; le plan ne s'y oppose pas, il refuse seulement de la
faire en silence.

### T4 — le tri de `list_tasks_paginated` est le mauvais pour cet usage

`list_tasks_paginated` (`db.rs:13526`) trie `ORDER BY updated_at DESC`. Pour le diagnostic
D1 — *dédoublonner* une récurrence — c'est le tri qui cache le symptôme : deux lignes du même
label atterrissent à des endroits arbitraires de la liste selon leur dernière mise à jour.

Le registre se lit `ORDER BY label ASC, created_at ASC` : les doublons d'un label sont
contigus et leur ordre de naissance est visible. Cela suffit à justifier une fonction DB
dédiée plutôt qu'un appel à la fonction générique — et cette fonction dédiée est de toute
façon le véhicule de la projection fermée de T1.

Le ticket demande de « réutiliser la machinerie `handle_tasks_list` ». C'est ce qui est fait :
la pagination (`resolve_pagination`, `dashboard.rs:38`), la forme `PaginatedResponse`, le
style de handler et le montage sous `/api/v1` sont repris tels quels. Ce qui n'est pas
réutilisé est la **projection** — et c'est le ticket lui-même qui le demande en interdisant
le contenu de message.

---

## Requirements

**R1 — Tenant (mika-agent).** `GET /api/v1/recurring-tasks` rend le registre des lignes
`trigger_type = 'recurring'`, en métadonnées seules, trié `label, created_at`, tous statuts
confondus. Paramètres : `agent_id` (optionnel), `page` / `per_page` (mêmes bornes que
l'existant : défaut 50, max 200).

**R2 — `trigger_type = 'recurring'` est en dur dans le SQL.** Ce n'est pas un paramètre de
requête, pas une valeur par défaut surchargeable, pas un `TaskFilters`. Aucune chaîne de
requête ne peut élargir la population à `manual` / `callback` / `time`.

**R3 — Projection fermée au niveau SQL.** Le `SELECT` nomme ses colonnes. Ni `SELECT *`, ni
`Self::TASK_COLUMNS`, ni `action_config`, ni `result`, ni `input_context`, ni `metadata`
brut. Colonnes : `label, agent_id, trigger_type, action_type, cron_expr, next_fire_at,
status, created_at, updated_at` + le booléen dérivé `zombie_veto_active`.

**R4 — Lecture seule stricte.** Aucun `INSERT` / `UPDATE` / `DELETE`, aucun `exec`, aucun
appel au task engine, aucune émission de `TaskEventFrame`. Le handler ne fait qu'un `SELECT`.

**R5 — Gateway.** `GET /admin/tenants/{customer_id}/recurring-tasks` proxifie vers le tenant
en réutilisant `container_url_str` (`routes.rs:679`) et le hop interne existant.

**R6 — Scope admin read dédié.** Nouveau `MIKA_GATEWAY_ADMIN_READ_TOKEN` (`SecretString`,
optionnel). Seul ce jeton ouvre la route. L'INTERNAL_TOKEN y est refusé en `403` (cf. T3).

**R7 — Fail-closed sur la non-configuration.** `admin_read_token: None` → la route rend
`404`, exactement comme `github_webhook_secret`, `orchestrator_inbox_enabled` et
`search_egress_client` (`routes.rs:108-121`, `:209-215`). **Une ligne INFO au démarrage dit
lequel des deux états est en vigueur** — sans elle, un `404` sur une route qui existe se lit
comme « pas déployé », et l'opérateur débogue la mauvaise couche (la classe que mika#2293 a
dû fermer une fois).

**R8 — Une demi-configuration ne passe pas en silence.** Si `MIKA_GATEWAY_ADMIN_READ_TOKEN`
est **égal** à `MIKA_INTERNAL_TOKEN` (copier-coller du même secret), la ségrégation que R6
existe pour créer est annulée sans qu'aucun test d'AC2 puisse le voir. Dans ce cas la route
est **désarmée** (traitée comme non configurée → `404`) et un `WARN` nommé le dit.

Désarmer plutôt que refuser le démarrage : downer toute la gateway — routage Telegram,
webhooks GitHub, `/send` de tous les tenants — pour une route d'inspection serait une
rançon. Le fail-closed local ne prend personne en otage, et le `WARN` rend la cause lisible.

**R9 — Traçabilité des accès.** Chaque appel servi écrit une ligne INFO structurée et une
ligne `audit_events` (`tool_name = 'admin_read'`, `target_key = 'tenant:{customer_id}'`).
Cet endpoint lit les données d'un tenant tiers : « qui a lu le registre d'Al, et quand »
doit être une requête SQL, pas un grep. L'écriture est fire-and-forget sur le modèle de
`audit_events::log_webhook_drop` — un échec d'audit journalise un `WARN` et ne change pas la
réponse (R4 interdit de faire dépendre une lecture d'une écriture).

**R10 — Borne de volume.** `per_page` est déjà borné à 200 par `resolve_pagination`, et la
réponse porte son `total`. Un tenant pathologiquement dupliqué — c'est-à-dire précisément le
cas D1 — reste lisible page par page, et `total` dit l'ampleur sans rapatrier le registre.

**R11 — Périmètre.** Aucun endpoint d'écriture. Dédoublonner ou réinitialiser une récurrence
(D1/D3 de `mika issue#2358`) sont des endpoints séparés, derrière un scope write, dans leur
propre ticket.

---

## Conception

### 3.1 Couche DB — `crates/mika-agent/src/db.rs`

Une struct de projection dédiée, **distincte de `Task`**, pour que la croissance de `Task`
n'élargisse jamais cette surface (T1) :

```rust
/// mika#2360 — projection fermée du registre des récurrences.
///
/// Volontairement distincte de [`Task`] : ce qui n'est pas nommé ici ne peut
/// pas atteindre la réponse HTTP, quoi que la table `tasks` gagne ensuite.
/// `action_config`, `result`, `input_context` et `metadata` sont exclus par
/// construction — ils portent du contenu utilisateur (mika#2360 AC4).
pub struct RecurringRegistryRow {
    pub label: String,
    pub agent_id: String,
    pub trigger_type: String,        // toujours "recurring" (R2)
    pub action_type: String,
    pub cron_expr: Option<String>,
    pub next_fire_at: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    /// mika#1742 : cette ligne arme-t-elle le veto de re-registration ?
    pub zombie_veto_active: bool,
}
```

Et la fonction de lecture :

```rust
pub fn list_recurring_registry(
    &self,
    agent_id: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<(Vec<RecurringRegistryRow>, u64)>
```

- `WHERE trigger_type = 'recurring'` littéral dans la chaîne SQL (R2), `AND agent_id = ?`
  ajouté seulement si le paramètre est `Some`.
- `ORDER BY label ASC, created_at ASC` (T4).
- `zombie_veto_active` calculé dans le `SELECT`, en réutilisant les constantes
  déjà exportées plutôt qu'en réécrivant leurs littéraux : `RECURRING_ZOMBIE_GRACE_SQL`
  (`db.rs:49`), `RECURRING_CONFIG_CANCEL_REVERTED_PATH` (`:66`),
  `RECURRING_UNKNOWN_TRIGGER_PATH` (`:87`).

  **Point de vigilance, à traiter comme un risque et non comme un détail :** le prédicat de
  veto vit aujourd'hui dans le SQL de `create_recurring_task_if_absent` (`db.rs:6205`). Une
  copie ici est une divergence en attente — la classe exacte que `grooming_marker.rs` a dû
  refermer une fois (mika#2158 : deux régexes du même verdict, dont l'une n'a pas suivi deux
  élargissements). Deux options, à trancher à l'implémentation :

  - **(a) Extraire le fragment de prédicat** en une constante `&'static str` consommée par
    les deux requêtes. Un seul littéral, aucune dérive possible.
  - **(b) Le dupliquer** en épinglant l'accord par un test qui construit les états pertinents
    (ligne `cancelled` récente, `cancelled` + marqueur reverted, `failed` hors fenêtre,
    `unknown_trigger` + lift dépensé) et vérifie que `zombie_veto_active` et le comportement
    réel de `create_recurring_task_if_absent` **concordent sur chacun**.

  **(a) est préférée** si le fragment s'extrait sans contorsion de numérotation de paramètres
  — c'est la forme qui rend la dérive impossible plutôt que détectée. (b) est le repli
  acceptable ; l'option interdite est la troisième, dupliquer sans épingler.

  Si la complexité s'avère disproportionnée à l'implémentation, le repli explicite est
  d'exposer `zombie_veto_active` en `Option<bool>` avec `None` pour « non déterminé » plutôt
  que de rendre un `false` qui **affirmerait** l'absence de veto sans l'avoir vérifiée — un
  faux `false` sur ce champ renverrait l'opérateur exactement dans l'angle mort que T2
  existe pour éclairer.

Wrapper async dans `async_db.rs`, sur le modèle de `get_recurring_task_cron` (`:360`) :
`with_db(move |db| db.list_recurring_registry(...))`, **sans** émission de frame (R4 ; noter
que `create_recurring_task_if_absent` en émet une — la différence est le point).

### 3.2 Couche handler tenant — `crates/mika-agent/src/server/dashboard.rs`

```rust
#[derive(Deserialize)]
pub struct RecurringRegistryQuery {
    pub agent_id: Option<String>,
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

pub async fn handle_recurring_registry(
    State(state): State<AppState>,
    Query(q): Query<RecurringRegistryQuery>,
) -> impl IntoResponse
```

- `resolve_pagination` réutilisé tel quel (R1, R10).
- Réponse : `PaginatedResponse<RecurringTaskResponse>` — même enveloppe que
  `handle_tasks_list`, `{data, total, page, per_page}`.
- `RecurringTaskResponse` est un `From<RecurringRegistryRow>` **champ pour champ**, sans
  aucune troncature de contenu : il n'y a plus de contenu à tronquer, c'est le propos.
- Erreur DB → `internal_error` (`dashboard.rs:45`), inchangé.

Montage dans `build_router` (`server/mod.rs:129-315`), **dans `dashboard_routes`** :

```rust
.route("/recurring-tasks", get(dashboard::handle_recurring_registry))
```

Donc `/api/v1/recurring-tasks`, sous `require_dashboard_or_internal_token`
(`server/mod.rs:311`). La gateway porte l'internal token : le hop passe sans nouveau secret
côté pod (T3). Aucune route de mutation n'est touchée.

### 3.3 Couche gateway — `crates/mika-gateway/`

**`settings.rs`** — nouveau champ, sur le modèle de `github_webhook_secret` :

```rust
/// mika#2360 — jeton admin READ-ONLY. Optionnel : absent ⇒ la route
/// `/admin/tenants/{id}/recurring-tasks` rend 404 (R7).
/// Refusé s'il est égal à `internal_token` (R8).
#[serde(default)]
pub gateway_admin_read_token: Option<SecretString>,
```

**`routes.rs` — `AppState`** : `pub admin_read_token: Option<SecretString>`, ajouté au
`Debug` manuel comme `[REDACTED]` (`routes.rs:166-183`) — le `Debug` de `AppState` est
rédacteur par champ, un ajout non rédigé fuirait le secret dans un log de diagnostic.

La résolution R8 se fait **une fois à la construction de l'`AppState`**, pas à chaque
requête : si le jeton lu est égal à l'internal token, le champ est peuplé à `None` et le
`WARN` est émis là. Un état invalide ne doit pas exister dans l'`AppState` — il ne peut pas
alors être oublié sur un chemin.

**`routes.rs` — middleware** (voisin de `require_bearer_token`, `:1899`) :

```rust
async fn require_admin_read_token(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: Next,
) -> impl IntoResponse
```

Branches, dans cet ordre :

1. `state.admin_read_token == None` → `404` (R7/R8).
2. en-tête absent / non `Bearer ` → `401`.
3. `constant_time_eq(t, admin_read_token)` → passe.
4. `constant_time_eq(t, internal_token)` → **`403`** + `WARN` nommé (T3, AC2).
5. sinon → `401`.

La branche 1 précède toute lecture d'en-tête : sur une route non configurée, la réponse ne
doit dépendre d'aucune entrée du client. Les comparaisons restent en temps constant
(`constant_time_eq`, déjà utilisée `routes.rs:1911`) — l'égalité ne se compare pas avec `==`
sur un secret, ici pas plus qu'ailleurs.

**`routes.rs` — route :**

```rust
.route(
    "/admin/tenants/{customer_id}/recurring-tasks",
    get(handle_admin_tenant_recurring_tasks).route_layer(
        middleware::from_fn_with_state(state.clone(), require_admin_read_token),
    ),
)
```

**Handler proxy** — décalque de `handle_a2a_proxy` (`a2a_routes.rs:75-110`), en `GET` :

```rust
let container = container_url_str(&customer_id, state.agent_base_url.as_deref(),
                                  &state.agents_namespace);
let forward_url = format!("{container}/api/v1/recurring-tasks");
```

- `Bearer {internal_token}` sur le hop interne (T3).
- Query string transmise en liste blanche : `agent_id`, `page`, `per_page`. **Pas de
  passthrough opaque** — un passthrough laisserait un client poser un paramètre que le
  handler tenant ne connaît pas encore aujourd'hui mais pourrait connaître demain, et
  l'élargissement se ferait sans qu'aucune ligne de ce ticket ne change.
- Corps de réponse relayé tel quel ; statut relayé ; timeout aligné sur le client HTTP
  partagé de l'`AppState`.
- Échec de forward → `502` + `error!`. Ne jamais rendre `200` avec un corps vide : un
  registre vide et une gateway qui n'a pas joint le pod sont deux réponses différentes, et
  les confondre est exactement le mode de panne que T2 décrit une couche plus bas.
- `openapi.rs` : ajouter le chemin à `paths(...)` (`:19-29`) et le schéma de réponse à
  `components(schemas(...))`.

### 3.4 Companion mika-cloud

Hors de ce dépôt (`mika-cloud` n'est pas dans ce workspace ; vérifié). **Ticket compagnon**,
dont le périmètre est déjà déterminé par ce qui précède :

1. Ajouter `MIKA_GATEWAY_ADMIN_READ_TOKEN` au `Secret` de la gateway et le monter dans le
   `Deployment`.
2. Vérifier l'exposition ingress du préfixe `/admin/*` — les trois routes `/admin/customers*`
   existent déjà, donc **la voie est probablement déjà ouverte** ; à confirmer plutôt qu'à
   supposer.
3. Générer le secret **distinct** de l'internal token (R8 — sinon la route se désarme et
   l'opérateur cherchera longtemps la mauvaise cause).

**Ce ticket est livrable et testable sans le compagnon** : sans le secret, la route rend
`404` par R7 et la ligne de démarrage le dit. C'est ce qui permet de ne pas coupler les deux
merges.

---

## Verification contract

### Tests unitaires — DB (`db.rs`, module `tests` inline)

| Test | Ce qu'il tient |
|---|---|
| `mika2360_registry_excludes_non_recurring_rows` | Insère `manual` + `callback` + `time` + `recurring` ; seule la dernière sort. R2. |
| `mika2360_registry_projection_carries_no_message_content` | Une récurrence `send_message` dont `action_config` contient une sentinelle (`"SECRET-MEDICATION-REMINDER"`) ; la sérialisation JSON de la réponse **ne contient pas** la sentinelle. **AC4, testée sur le JSON rendu, pas sur les champs de la struct** — un test sur les champs passerait encore si quelqu'un ajoutait un `#[serde(flatten)]`. |
| `mika2360_registry_lists_cancelled_and_failed_rows` | Les lignes non-`active` sont présentes. T2. |
| `mika2360_registry_orders_by_label_then_created_at` | Deux lignes du même label, créées dans le désordre de `updated_at`, sortent contiguës et en ordre de naissance. T4. |
| `mika2360_zombie_veto_flag_matches_registration_refusal` | Les quatre états du veto (récent `cancelled`, `cancelled` + reverted, `failed` hors fenêtre, `unknown_trigger` + lift dépensé) ; `zombie_veto_active` **concorde** avec le comportement observé de `create_recurring_task_if_absent`. Le test d'accord de T2/§3.1. |
| `mika2360_registry_is_read_only` | Compte les lignes de `tasks` et lit `updated_at` de chacune avant/après ; identiques. **AC3.** |

### Tests d'intégration — handler tenant (`server/mod.rs`, module `tests`)

Le fichier porte déjà ce style (`.header("authorization", "Bearer test-token-secret")`,
`:2019` et suivants) ; les nouveaux tests s'y insèrent sans échafaudage.

| Test | Ce qu'il tient |
|---|---|
| `mika2360_recurring_registry_requires_auth` | Sans en-tête → `401`. |
| `mika2360_recurring_registry_accepts_dashboard_token` | Jeton dashboard → `200`. |
| `mika2360_recurring_registry_returns_paginated_shape` | `{data, total, page, per_page}`. AC1. |
| `mika2360_recurring_registry_agent_filter` | `?agent_id=` restreint bien. |

### Tests d'intégration — gateway (`routes.rs`, module `tests`)

| Test | Ce qu'il tient |
|---|---|
| `mika2360_admin_read_route_404_when_token_unconfigured` | `admin_read_token: None` → `404`, **même avec un en-tête valide**. R7. |
| `mika2360_admin_read_route_404_when_token_equals_internal` | R8 — la demi-configuration désarme. |
| `mika2360_admin_read_rejects_internal_token_with_403` | **AC2**, le cas « write-seul ». |
| `mika2360_admin_read_rejects_missing_header_with_401` | La divergence T3, épinglée pour qu'elle soit *décidée* et non *dérivée*. |
| `mika2360_admin_read_rejects_unknown_token_with_401` | Idem. |
| `mika2360_admin_read_forwards_with_internal_token` | Serveur amont factice (`agent_base_url`) : le hop porte bien `Bearer {internal_token}` et frappe `/api/v1/recurring-tasks`. |
| `mika2360_admin_read_forwards_only_allowlisted_query_params` | `?agent_id=x&per_page=5&evil=1` → l'amont voit les deux premiers, jamais le troisième. |
| `mika2360_admin_read_upstream_failure_is_502_not_empty_200` | Amont injoignable → `502`. |
| `mika2360_no_mutating_method_on_admin_read_route` | `POST`/`PUT`/`DELETE`/`PATCH` sur le chemin → `405`. AC3, tenue par le routeur. |

### Vérification manuelle (post-merge, après le compagnon cloud)

```bash
# AC1 — le registre d'Al
curl -s -H "Authorization: Bearer $MIKA_GATEWAY_ADMIN_READ_TOKEN" \
  "$GW/admin/tenants/<al>/recurring-tasks" | jq .

# AC2 — l'internal token ne suffit pas
curl -s -o /dev/null -w '%{http_code}\n' \
  -H "Authorization: Bearer $MIKA_INTERNAL_TOKEN" \
  "$GW/admin/tenants/<al>/recurring-tasks"        # attendu : 403

# AC4 — aucune sentinelle de contenu dans la réponse
# (aucun champ *_preview, action_config, result, input_context, metadata)
```

### Commandes

`cargo test -p mika-agent`, `cargo test -p mika-gateway`, `cargo clippy --all-targets
-- -D warnings`, `cargo fmt --check`.

---

## Definition of Done

- [ ] `RecurringRegistryRow` + `list_recurring_registry` dans `db.rs`, projection fermée,
      `trigger_type` littéral, tri `label, created_at`.
- [ ] `zombie_veto_active` calculé, et son accord avec `create_recurring_task_if_absent`
      soit **rendu impossible à rompre** (fragment SQL partagé), soit **épinglé** par le test
      d'accord.
- [ ] Wrapper async sans émission de frame.
- [ ] `handle_recurring_registry` + montage `/api/v1/recurring-tasks` sous l'auth dashboard.
- [ ] `gateway_admin_read_token` dans `GatewaySettings`, `admin_read_token` dans `AppState`,
      rédigé dans le `Debug` manuel.
- [ ] R8 résolu à la construction de l'`AppState` (jamais par requête), `WARN` nommé.
- [ ] `require_admin_read_token` avec ses cinq branches dans l'ordre spécifié.
- [ ] Route gateway + handler proxy, query string en liste blanche, `502` sur échec amont.
- [ ] Ligne INFO de démarrage disant l'état d'armement (R7).
- [ ] Ligne `audit_events` `tool_name = 'admin_read'` par accès servi, fire-and-forget (R9).
- [ ] `openapi.rs` de la gateway à jour.
- [ ] Les 19 tests ci-dessus passent ; clippy et fmt propres.
- [ ] Root `CLAUDE.md` : `MIKA_GATEWAY_ADMIN_READ_TOKEN` documenté (valeur, défaut, R7/R8,
      surfaces opérateur) ; `.env.example` mis à jour.
- [ ] Corps de PR : nomme la divergence 401/403 (T3) et le ticket compagnon mika-cloud.
- [ ] Ticket compagnon mika-cloud ouvert (§3.4).

---

## Acceptance criteria

Transcrits verbatim du corps de `mika issue#2360`, suivis de ce qui les rend vérifiables.

- **AC1** — `GET /admin/tenants/{id}/recurring-tasks` avec token admin read → JSON du
  registre récurrent du tenant.
  → `mika2360_admin_read_forwards_with_internal_token` +
  `mika2360_recurring_registry_returns_paginated_shape` + la vérification manuelle.

- **AC2** — token write-seul / absent / mauvais scope → 403 (gated).
  → `mika2360_admin_read_rejects_internal_token_with_403` pour le cas « write-seul », qui est
  le cas de scope. **Les cas « absent » et « inconnu » rendent `401`** et sont épinglés par
  leurs deux tests : divergence assumée avec la lettre de l'AC, motivée en T3, soumise à
  l'arbitrage de l'architecte, réversible en une ligne par branche.

- **AC3** — l'endpoint n'effectue **aucune mutation** (test : registre inchangé avant/après
  appel) et **aucun exec**.
  → `mika2360_registry_is_read_only` (compte + `updated_at` de chaque ligne, avant/après) et
  `mika2360_no_mutating_method_on_admin_read_route`. Aucun `exec` : le chemin complet est un
  `SELECT` derrière un `GET` proxifié ; aucun `Command`, aucun appel au task engine.

- **AC4** — aucun contenu de message/conversation dans la réponse (métadonnées de tâche
  seules).
  → `mika2360_registry_projection_carries_no_message_content`, assertion sur le **JSON
  rendu**. Tenue structurellement par R3 : `action_config`, `result`, `input_context` et
  `metadata` ne sont dans aucun `SELECT` de ce chemin.

  **Réserve nommée plutôt que tue :** `label` est demandé par le ticket et il est
  indispensable — c'est la clé par laquelle l'opérateur identifie une récurrence, et sans lui
  D1/D2/D3 sont inaccessibles. Or un `label` de récurrence créée par le LLM est libre, donc
  potentiellement descriptif (« rappel-medicaments-marie »). Ce n'est pas du contenu de
  message au sens d'AC4 — c'est un identifiant — mais c'est la seule surface de la réponse
  qui puisse porter des mots choisis par l'utilisateur. Le plan le **livre** (le ticket le
  demande) et l'**écrit** ici pour que ce soit une décision plutôt qu'un oubli.

---

## Risques et hors périmètre

**Risques.**

| Risque | Parade |
|---|---|
| Divergence du prédicat de veto entre `zombie_veto_active` et `create_recurring_task_if_absent` (classe mika#2158) | Fragment SQL partagé (préféré) ou test d'accord sur quatre états (repli). Jamais la duplication nue. |
| `TaskResponse`/`Task` gagne un champ de contenu et le rouvre | Impossible : projection dédiée, struct distincte, `SELECT` nommé (R3). |
| Le secret read n'est pas déployé et l'opérateur débogue la mauvaise couche | `404` + ligne INFO de démarrage (R7). |
| Les deux secrets sont identiques et AC2 est annulée en silence | Désarmement + `WARN` (R8), résolu à la construction de l'`AppState`. |
| Un `403` est lu comme « l'INTERNAL_TOKEN ne peut pas lire ce tenant » | T3 l'écrit noir sur blanc : le porteur du write est superuser ailleurs sur `/admin/*`. Le gain est organisationnel. |

**Hors périmètre, délibérément.**

- Tout endpoint d'écriture — dédoublonner, réinitialiser, annuler une récurrence (D1/D3 de
  `mika issue#2358`). Scope write, ticket séparé (R11).
- La **cause** du bug de récurrences d'Al. Ce ticket livre l'instrument qui permet de la
  lire ; il ne la corrige pas.
- Le companion mika-cloud (§3.4) — autre dépôt, ticket compagnon.
- Une surface dashboard pour ce registre. Le besoin est l'inspection opérateur en ligne de
  commande ; une carte de dashboard est une décision produit que personne n'a prise.
- L'élargissement de `/admin/*` à d'autres lectures par tenant (sessions, faits, audit). Le
  middleware `require_admin_read_token` les rendra bon marché le jour où un besoin mesuré les
  demandera ; anticiper serait deviner.
