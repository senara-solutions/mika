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
  (`db.rs:45`, `RECURRING_ZOMBIE_GRACE_HOURS = 24`). Trois marqueurs `metadata` modulent
  ce veto, et leurs chemins JSON sont à lire par leurs constantes plutôt que recopiés :
  `RECURRING_CONFIG_CANCEL_REVERTED_PATH` = `$.config_cancel_reverted` (`db.rs:66`,
  mika#2271), `RECURRING_UNKNOWN_TRIGGER_PATH` = `$.unknown_trigger_death` (`db.rs:86`) et
  `RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH` = `$.unknown_trigger_lift_consumed`
  (`db.rs:102`, mika#2337).

  **Le troisième nom est celui-là et pas un autre.** Une passe antérieure de ce plan l'avait
  écrit `$.unknown_trigger_lift_spent` — un nom qui n'existe nulle part dans l'arbre. Un
  chemin JSON erroné dans un `json_extract` ne casse pas la compilation et ne lève aucune
  erreur SQLite : il rend `NULL`, donc le marqueur est lu comme absent et le booléen répond
  **faux avec assurance**. C'est-à-dire précisément le faux `false` contre lequel le repli
  `Option<bool>` de §3.1 existe, obtenu par une faute de frappe plutôt que par une
  incertitude assumée.

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

`/admin/*` **existe déjà** sur la gateway (`routes.rs:292`, `:303`, `:311` : `POST
/admin/customers`, `GET /admin/customers/{id}`, `POST /admin/customers/{id}/unlink`) et il est
protégé par
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

`list_tasks_paginated` (`db.rs:13518`) trie `ORDER BY updated_at DESC`. Pour le diagnostic
D1 — *dédoublonner* une récurrence — c'est le tri qui cache le symptôme : deux lignes du même
label atterrissent à des endroits arbitraires de la liste selon leur dernière mise à jour.

Le registre se lit `ORDER BY label COLLATE NOCASE ASC, created_at ASC` : les doublons d'un
label sont contigus et leur ordre de naissance est visible. **La collation n'est pas un
raffinement** — sans elle, deux lignes ne différant que par la casse, qui sont le *même*
label pour le veto, sont rendues non contiguës par le tri même qui doit les rapprocher
(T6 b). Cela suffit à justifier une fonction DB dédiée plutôt qu'un appel à la fonction
générique — et cette fonction dédiée est de toute façon le véhicule de la projection fermée
de T1.

Le ticket demande de « réutiliser la machinerie `handle_tasks_list` ». C'est ce qui est fait :
la pagination (`resolve_pagination`, `dashboard.rs:38`), la forme `PaginatedResponse`, le
style de handler et le montage sous `/api/v1` sont repris tels quels. Ce qui n'est pas
réutilisé est la **projection** — et c'est le ticket lui-même qui le demande en interdisant
le contenu de message.

### T5 — un jeton admin global retire la validation qui protégeait `container_url_str`

C'est la lecture la plus importante de la passe, et elle inverse le signe du ticket si on
l'ignore : **le chemin le moins privilégié, écrit naïvement, exfiltre le jeton d'écriture.**

`container_url_str` (`routes.rs:679`) interpole son argument dans une URL sans le valider
d'aucune manière :

```rust
None => format!("http://mika-{customer_id}.{agents_namespace}.svc.cluster.local:8080"),
```

Sur les chemins qui l'appellent aujourd'hui, c'est sans danger — mais **pas grâce à cette
fonction**. `handle_a2a_proxy` (`a2a_routes.rs:31`) et `handle_a2a_agent_card` (`:166`)
valident d'abord une clé API qui *résout un customer* (`validate_a2a_api_key`, appelé
`:183`) et refusent en `401`/`403` tout `customer_id` qui n'est pas celui de la clé — avant
que l'interpolation ait lieu (`:210`). La sûreté vient du **préalable**, pas du formatage.

Or le jeton de R6 est **global** : il n'est lié à aucun customer. Le préalable disparaît, et
avec lui la seule chose qui contraignait l'argument. Un `customer_id` choisi par l'appelant
devient alors le début d'un nom d'hôte :

```
customer_id = "x.attacker.example/"
→ http://mika-x.attacker.example/.mika-agents.svc.cluster.local:8080
   ^ hôte = mika-x.attacker.example
```

et la requête que la gateway envoie à cet hôte porte `Bearer {internal_token}` (T3, hop
interne). **Le jeton d'écriture — superuser sur `/admin/*` — partirait vers un hôte
arbitraire.** Un ticket dont le propos est de *réduire* le privilège nécessaire à une lecture
aurait livré une exfiltration du privilège maximal. Le porteur du jeton read est certes déjà
un opérateur de confiance, mais c'est exactement la confiance que R6 cherche à pouvoir
distribuer plus largement : le gain organisationnel de T3 n'a de sens que si le jeton read ne
vaut pas le jeton write.

**La parade est double, bon marché, et déjà idiomatique à quinze lignes de la route à
créer.** `handle_get_customer` (`routes.rs:1432`), l'admin voisin, la porte entière :

```rust
async fn handle_get_customer(
    State(state): State<AppState>,
    Path(customer_id): Path<Uuid>,          // ← Axum refuse en 400 tout non-UUID
) -> impl IntoResponse {
    let sql = format!("SELECT {CUSTOMER_SAFE_COLUMNS} FROM customers WHERE id = $1");
    // … Ok(None) => 404
```

`customers.id` est un `UUID PRIMARY KEY` (`migrations/001_customers.sql`), donc :

1. **`Path<Uuid>`** — l'extracteur rejette en `400` avant l'entrée du handler. Aucun caractère
   capable de détourner une autorité d'URL ne survit à un parse d'UUID. C'est une garantie de
   **type**, pas une liste de caractères interdits à maintenir.
2. **Résolution en base** — `SELECT 1 FROM customers WHERE id = $1`, `404` si inconnu. Ce
   second terme n'est pas redondant : il empêche d'énumérer des pods pour des customers qui
   n'existent pas, et il aligne la route sur le comportement de son voisin `/admin/customers/{id}`.

Les deux, et dans cet ordre. Le premier suffit à fermer la SSRF ; le second est ce qui rend
la réponse honnête.

**Corollaire à écrire plutôt qu'à découvrir :** en mode single-tenant (`agent_base_url =
Some`), `container_url_str` **ignore** `customer_id` et rend la même base pour tout le monde.
La route répond donc avec le registre de l'unique agent local quel que soit l'id passé — et
la résolution en base du point 2 reste néanmoins exigée, sans quoi un id inconnu rendrait
`200` avec le registre de quelqu'un d'autre. C'est le mode de dev, pas le mode canari ; le
nommer évite de lire un test local vert comme une preuve de routage.

### T6 — le veto n'est pas un prédicat de ligne, et le tri sépare les doublons qu'il doit rapprocher

Deux lectures de `db.rs:6205-6258` et du schéma `tasks`. Elles ne changent pas le livrable ;
elles changent ce qu'un implémenteur doit écrire pour que T2 et T4 soient vrais plutôt que
plausibles.

**(a) `zombie_veto_active` se calcule sur DEUX requêtes, dont la première est un agrégat de
groupe.** Une passe antérieure de ce plan parlait d'« un fragment de prédicat » à extraire ou
à dupliquer. Il n'y en a pas un, il y en a deux, et le second consomme le résultat du premier
comme paramètre scalaire :

```rust
// 1) db.rs:6210 — EXISTS sur (agent_id, label), PAS sur une ligne
let lift_already_spent: bool = … "SELECT EXISTS(SELECT 1 FROM tasks
     WHERE agent_id = ?1 AND label = ?2 COLLATE NOCASE
       AND trigger_type = 'recurring'
       AND updated_at > strftime(…, ?3)
       AND json_valid(metadata)
       AND COALESCE(json_extract(metadata, ?4), 0) = 1)" …

// 2) db.rs:6231 — le booléen ci-dessus entre en ?5
… "AND NOT (?5 = 0 AND json_valid(metadata)
           AND COALESCE(json_extract(metadata, ?6), 0) = 1)" …
```

**Le lift est une propriété de `(agent_id, label)`, pas de la ligne.** Il peut avoir été
dépensé sur une ligne *autre* que celle qu'on est en train de projeter. Un implémenteur qui
lit le §3.1 dans sa forme antérieure — « calculé dans le `SELECT` en réutilisant les
constantes » — écrit naturellement `json_extract(metadata, …)` sur la ligne courante, et
obtient un booléen faux exactement dans le cas que mika#2337 a créé : deux lignes du même
label, le lift porté par l'une, le veto évalué sur l'autre. Le calcul par ligne exige donc
une **sous-requête corrélée** sur `(agent_id, label COLLATE NOCASE)`, pas une lecture du blob
de la ligne.

Conséquence sur l'arbitrage (a)/(b) du §3.1 : l'option « extraire le fragment » n'est pas
bloquée par la numérotation des paramètres — elle l'est par le fait qu'il faut extraire deux
fragments dont le second dépend de la valeur du premier, laquelle est calculée en Rust entre
les deux. **L'option (b), le test d'accord, devient donc le choix attendu**, et l'interdit
demeure le troisième terme : dupliquer sans épingler.

Note que `ORDER BY updated_at DESC LIMIT 1` dans la requête 2 ne restreint pas le prédicat —
il choisit seulement quelle ligne morte journaliser. Toute ligne satisfaisant le `WHERE` arme
le veto, donc « cette ligne arme-t-elle le veto » reste une question bien posée par ligne,
une fois le terme de lift correctement corrélé.

**(b) `label` n'a pas de collation de colonne, et le veto compare en `COLLATE NOCASE`.** Le
schéma déclare `label TEXT NOT NULL` (`db.rs:1460`) — sans `COLLATE NOCASE`, contrairement à
la convention énoncée pour les colonnes texte uniques. Les cinq comparaisons du veto posent
donc la collation explicitement, à chaque site.

Un `ORDER BY label ASC` nu trie donc en binaire : `Rappel` avant `rappel`, séparés par tout
ce qui commence par une minuscule intermédiaire. **Or deux lignes qui ne diffèrent que par la
casse sont le même label pour le veto** — c'est-à-dire précisément la classe de doublon la
plus probable quand un label est produit par un LLM à des semaines d'intervalle. Le tri censé
rendre les doublons contigus (T4) les éloignerait exactement sur le cas D1 qu'il existe pour
servir. Le tri est donc `ORDER BY label COLLATE NOCASE ASC, created_at ASC`, et la
sous-requête corrélée du point (a) groupe sur la même collation — les deux doivent s'accorder,
sinon le booléen et le voisinage visuel racontent deux histoires différentes sur la même page.

### T7 — quatre lectures sur les surfaces d'accueil, dont deux qui rectifient ce plan

Les passes antérieures ont vérifié la logique métier (T1–T6) et laissé les surfaces
d'accueil — montage du routeur, `Debug`, OpenAPI, documentation — au statut d'évidences.
Deux d'entre elles se révèlent **fausses à la lecture**, et une exigence fausse coûte plus
cher qu'une exigence absente : elle apprend au clavier une propriété du code qui sera
appliquée ailleurs.

**(a) Le `Debug` d'`AppState` se termine par `finish_non_exhaustive()` — l'omission est
déjà sûre.** Le §3.3 justifiait l'ajout du champ par « le `Debug` de `AppState` est rédacteur
par champ, un ajout non rédigé fuirait le secret ». C'est l'inverse : `routes.rs:166-183`
n'énumère que **six** champs sur la vingtaine que porte la struct (`pool`, `http_client`,
`github_app`, `search_egress_client`… sont déjà absents) et referme par
`.finish_non_exhaustive()`. Un champ qu'on n'ajoute pas n'est pas affiché ; il ne fuit pas.

L'exigence est **maintenue, pour une autre raison** : l'état d'armement de la route
(`Some`/`None` après la résolution R8) est exactement ce qu'un opérateur cherche dans un dump
de diagnostic, et c'est ce que le champ rend lisible sans exposer le secret. Ce qui change est
le classement du risque : ce n'est pas une fuite évitée, c'est une observabilité gagnée — et
aucune entrée de risque n'est ouverte pour une fuite qui ne peut pas se produire (le tableau
n'en portait pas — c'est la justification du §3.3, pas le tableau, qui était fausse).

**(b) `openapi.rs` de la gateway ne documente AUCUNE route `/admin/*`.** Le §3.3 et le DoD
exigeaient d'ajouter le chemin à `paths(...)`. Or les cinq annotations `#[utoipa::path]` de
`routes.rs` (`:69`, `:397`, `:1950`, `:2140`, `:2154`) portent sur `/webhook`, `/send`, les
sondes et la version ; les trois handlers admin — `handle_register_customer` (`:1034`),
`handle_admin_unlink` (`:1245`), `handle_get_customer` (`:1432`) — **n'en portent aucune**, et
`openapi.rs:19-29` ne les liste pas. La surface `/admin/*` est délibérément hors du spec
public : c'est un plan de contrôle opérateur, pas l'API de la gateway.

Satisfaire l'exigence telle qu'écrite ferait de cette route la **seule** route admin du spec,
et demanderait d'annoter un handler dont les trois voisins ne le sont pas. L'exigence est donc
**retirée** et remplacée par la surface documentaire qui existe réellement pour cette famille —
voir (d).

**(c) L'auth admin est posée par route, jamais par préfixe : oublier le `route_layer` laisse
la route OUVERTE.** Les onze occurrences de `require_bearer_token` (`routes.rs:206`→`:315`)
sont toutes des `.route_layer(...)` attachés à un `.route(...)` individuel. Il n'existe aucun
layer monté sur le préfixe `/admin`.

Deux conséquences, et la seconde est un mode de panne :

1. **Rien n'empêche la nouvelle route de porter un middleware différent de ses voisines** —
   le montage de R6 est mécaniquement possible, sans conflit ni superposition avec
   `require_bearer_token`. C'était l'hypothèse tacite du §3.3 ; elle est vérifiée.
2. **Le chemin `/admin/` ne protège rien par lui-même.** Un `.route(...)` sans `.route_layer`
   est servi sans aucune authentification — et non, comme on pourrait le supposer, protégé
   par défaut par l'internal token. Une route d'inspection multi-tenant ouverte est un
   incident d'un autre ordre que les `403` que ce ticket discute.

   La propriété est déjà couverte *par accident* par
   `mika2360_admin_read_rejects_missing_header_with_401` (sans middleware, la réponse ne
   serait pas `401`). Elle est ici **nommée** pour qu'un implémenteur sache ce que ce test
   tient, et que personne ne le juge redondant avec les quatre autres tests d'auth.

**(d) Le décalque `handle_get_customer` rend `500` sur erreur DB, là où ce plan exige `503`
— et la documentation de la gateway a deux surfaces que le DoD oubliait.** Sur l'erreur de
résolution, `routes.rs:1450` journalise et rend `500`. La section *harnais de test* exige
`503` pour la route neuve, et cette exigence est maintenue : `503` dit « réessaie », `500`
dit « c'est cassé », et l'indisponibilité de Postgres est le premier cas. Mais un
implémenteur qui copie le voisin — ce que le plan lui demande par ailleurs — écrira `500`
sans voir la contradiction. **La divergence est délibérée et porte sur ce seul code.**

Côté documentation, `crates/mika-gateway/CLAUDE.md` porte une table `## Endpoints` qui liste
les quatre routes `/admin/customers*` et une section `## Gateway Environment Variables`.
Ce sont les surfaces canoniques de cette famille — celles où un opérateur cherchera la route
et le jeton. Le DoD ne nommait que le `CLAUDE.md` racine.

---

## Requirements

**R1 — Tenant (mika-agent).** `GET /api/v1/recurring-tasks` rend le registre des lignes
`trigger_type = 'recurring'`, en métadonnées seules, trié `label COLLATE NOCASE, created_at`
(T6 b), tous statuts confondus. Paramètres : `agent_id` (optionnel), `page` / `per_page` (mêmes bornes que
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
ligne `audit_events` côté gateway. Cet endpoint lit les données d'un tenant tiers : « qui a
lu le registre d'Al, et quand » doit être une requête SQL, pas un grep. L'écriture est
fire-and-forget sur le modèle de `audit_events::log_webhook_drop`
(`crates/mika-gateway/src/audit_events.rs:83`) — un échec d'audit journalise un `WARN` et ne
change pas la réponse (R4 interdit de faire dépendre une lecture d'une écriture).

Trois précisions vérifiées dans l'arbre, qui font de R9 un ajout de quelques lignes et non un
chantier :

- **Aucune migration n'est nécessaire.** La table gateway (Postgres,
  `migrations/009_audit_events.sql`) déclare `tool_name TEXT NOT NULL` **sans contrainte
  `CHECK`** — un second `tool_name` s'y écrit sans DDL. L'index existant
  `audit_events_lookup_idx (tool_name, target_key, created_at DESC)` sert directement la
  requête opérateur ci-dessous, préfixe compris.
- **Constante dédiée, jamais la constante existante.** `audit_events::TOOL_NAME` vaut
  `"gateway_webhook"` et son doc-comment le déclare porteur pour la requête de mika#1774 ;
  y verser des lignes d'admin-read couperait cette population en deux sans le dire. Nouvelle
  constante `TOOL_NAME_ADMIN_READ: &str = "gateway_admin_read"` — préfixée `gateway_` pour
  rester dans la famille du fil, plutôt que l'`admin_read` nu d'une passe antérieure de ce
  plan.
- **`target_key = "tenant:{customer_id}"`**, avec le `customer_id` déjà validé par R12 (donc
  toujours un UUID canonique, jamais du texte libre de l'appelant dans la colonne indexée).

Requête opérateur : `SELECT target_key, created_at FROM audit_events WHERE tool_name =
'gateway_admin_read' ORDER BY created_at DESC;`

**R10 — Borne de volume.** `per_page` est déjà borné à 200 par `resolve_pagination`, et la
réponse porte son `total`. Un tenant pathologiquement dupliqué — c'est-à-dire précisément le
cas D1 — reste lisible page par page, et `total` dit l'ampleur sans rapatrier le registre.

**R11 — Périmètre.** Aucun endpoint d'écriture. Dédoublonner ou réinitialiser une récurrence
(D1/D3 de `mika issue#2358`) sont des endpoints séparés, derrière un scope write, dans leur
propre ticket.

**R12 — `customer_id` est validé avant toute interpolation d'URL.** Deux termes, dans cet
ordre, motivés en T5 :

1. **`Path<Uuid>`** sur le handler gateway — un non-UUID est refusé en `400` par
   l'extracteur, avant le corps du handler. Le type est la garantie ; pas de liste de
   caractères interdits.
2. **Résolution en base** — `customer_id` inconnu de la table `customers` → `404`, jamais un
   forward. Sur le modèle de `handle_get_customer` (`routes.rs:1432`).

`container_url_str` n'est appelée qu'**après** les deux. Cette exigence n'est pas
cosmétique : sans elle le hop interne, qui porte `Bearer {internal_token}`, est dirigeable
par l'appelant (T5).

**Cette exigence ne modifie pas `container_url_str`** et ne touche donc aucun appelant
existant. Durcir la fonction elle-même serait la parade la plus générale, mais elle change le
comportement de deux chemins A2A en production pour un risque qu'ils ne portent pas (leur
préalable de clé API les couvre) : hors périmètre, à faire dans son propre ticket si un
troisième appelant sans préalable apparaît un jour. Ce qui est livré ici est la garantie au
point d'entrée neuf, qui est le seul point d'entrée exposé.

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
- `ORDER BY label COLLATE NOCASE ASC, created_at ASC` (T4, T6 b).
- `zombie_veto_active` calculé dans le `SELECT`, en réutilisant les constantes
  déjà exportées plutôt qu'en réécrivant leurs littéraux : `RECURRING_ZOMBIE_GRACE_SQL`
  (`db.rs:49`), `RECURRING_CONFIG_CANCEL_REVERTED_PATH` (`:66`),
  `RECURRING_UNKNOWN_TRIGGER_PATH` (`:86`),
  `RECURRING_UNKNOWN_TRIGGER_LIFT_CONSUMED_PATH` (`:102`).

  **Le terme de lift est corrélé, jamais lu sur la ligne (T6 a).** Le booléen
  `lift_already_spent` de `create_recurring_task_if_absent` est un `EXISTS` sur
  `(agent_id, label COLLATE NOCASE)` dans la fenêtre de grâce — il peut donc être porté par
  une ligne différente de celle qu'on projette. Sa transposition dans un `SELECT` de liste est
  une **sous-requête corrélée** sur ce couple, pas un `json_extract` du `metadata` courant.
  Écrit sur la ligne, le booléen répondrait faux précisément sur la configuration que
  mika#2337 produit : deux lignes du même label, le lift sur l'une, le veto évalué sur l'autre.

  **Point de vigilance, à traiter comme un risque et non comme un détail :** le prédicat de
  veto vit aujourd'hui dans le SQL de `create_recurring_task_if_absent` (`db.rs:6205`). Une
  copie ici est une divergence en attente — la classe exacte que `grooming_marker.rs` a dû
  refermer une fois (mika#2158 : deux régexes du même verdict, dont l'une n'a pas suivi deux
  élargissements). Deux options, à trancher à l'implémentation :

  - **(a) Extraire le fragment de prédicat** en une constante `&'static str` consommée par
    les deux requêtes. Un seul littéral, aucune dérive possible.
  - **(b) Le dupliquer** en épinglant l'accord par un test qui construit les états pertinents
    (ligne `cancelled` récente, `cancelled` + marqueur reverted, `failed` hors fenêtre,
    `unknown_trigger` + lift dépensé **sur une ligne sœur**) et vérifie que
    `zombie_veto_active` et le comportement réel de `create_recurring_task_if_absent`
    **concordent sur chacun**.

  **(b) est le choix attendu, et T6 a dit pourquoi :** il n'y a pas *un* fragment à extraire
  mais deux, dont le second consomme en paramètre (`?5`) un booléen que le premier calcule et
  que Rust transporte entre les deux. (a) reste préférable *si* l'implémentation trouve une
  forme — par exemple une seule requête où la sous-requête corrélée remplace le paramètre —
  qui rende la dérive impossible plutôt que détectée ; c'est à évaluer au clavier, pas à
  décider ici. L'option interdite reste la troisième : dupliquer sans épingler.

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
(`server/mod.rs:313`). La gateway porte l'internal token : le hop passe sans nouveau secret
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
`Debug` manuel comme `.map(|_| "[REDACTED]")` (`routes.rs:166-183`), sur la forme exacte que
`webhook_secret` et `github_webhook_secret` y emploient déjà pour un `Option<SecretString>`.

**Ce n'est pas une fuite évitée, c'est une observabilité gagnée (T7 a).** Ce `Debug`
referme par `.finish_non_exhaustive()` et n'énumère que six champs sur la vingtaine de la
struct : un champ omis n'est pas affiché. L'ajout sert à rendre lisible **l'état d'armement
de la route** — `Some` ou `None` après la résolution R8 — dans un dump de diagnostic, ce qui
est précisément la question que R7 et R8 apprennent à l'opérateur à se poser. La forme
`Option::map` est ce qui distingue « armé » de « non configuré » sans exposer le secret ;
un `&"[REDACTED]"` nu les rendrait indiscernables et annulerait le seul gain de l'ajout.

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

**Handler proxy** — le décalque est **`handle_a2a_agent_card` (`a2a_routes.rs:166`)**, et non
`handle_a2a_proxy` (`:31`). Le nom compte : une passe antérieure de ce plan écrivait
`handle_agent_card_proxy`, qui n'existe nulle part dans l'arbre — un décalque qu'on ne peut
pas ouvrir n'est pas un décalque. À partir de sa ligne 209, la fonction fait exactement ce
qui est cherché ici : `container_url_str`, `format!("{container}/…")`,
`state.http_client.get(...)`, `Bearer {internal_token}` (`:222`), puis relais du statut et du
corps avec `BAD_GATEWAY` sur **les deux** échecs (envoi `:231`, lecture du corps). Ses
lignes 171-207 — extraction et validation de la clé API — sont ce qu'on ne reprend pas : ce
chemin est authentifié par R6 en amont, et c'est la disparition de ce préalable qui fait
exister R12 (T5). Le `POST` A2A ajoute en plus un corps dont ce chemin n'a que faire.

```rust
async fn handle_admin_tenant_recurring_tasks(
    State(state): State<AppState>,
    Path(customer_id): Path<Uuid>,          // R12 terme 1 — 400 sur non-UUID
    Query(q): Query<AdminRecurringQuery>,
) -> impl IntoResponse {
    // R12 terme 2 — 404 si le customer n'existe pas, AVANT toute interpolation
    // …SELECT 1 FROM customers WHERE id = $1 → Ok(None) ⇒ 404
    //                                        → Err(_)  ⇒ 503, JAMAIS de forward
    //   (le décalque handle_get_customer rend 500 ici ; divergence voulue, T7 d)

    let container = container_url_str(
        &customer_id.to_string(),
        state.agent_base_url.as_deref(),
        &state.agents_namespace,
    );
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
- **`openapi.rs` : ne rien ajouter (T7 b).** Les trois handlers `/admin/*` existants ne
  portent aucune annotation `#[utoipa::path]` et ne figurent pas dans `paths(...)` : la
  surface admin est délibérément hors du spec public. Y inscrire cette route en ferait la
  seule route admin documentée. La documentation de cette famille passe par
  `crates/mika-gateway/CLAUDE.md` — table `## Endpoints` et section
  `## Gateway Environment Variables` (T7 d).
- **Le `.route_layer` n'est pas optionnel (T7 c).** Aucun layer d'auth n'est monté sur le
  préfixe `/admin` : une route déclarée sans son `route_layer` est servie **sans
  authentification du tout**. Le montage ci-dessus le porte ; c'est la ligne à ne pas perdre
  dans un rebase.

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
| `mika2360_registry_orders_labels_case_insensitively` | **T6 b.** Trois lignes : `Rappel`, `aaa-autre-label`, `rappel`. Les deux variantes de casse sortent **contiguës** — un tri binaire les sépare par la ligne intermédiaire. Le test échoue si quelqu'un retire `COLLATE NOCASE` du `ORDER BY`, ce qu'aucune assertion sur un jeu mono-casse ne peut voir. |
| `mika2360_zombie_veto_flag_matches_registration_refusal` | Les quatre états du veto (récent `cancelled`, `cancelled` + reverted, `failed` hors fenêtre, `unknown_trigger` + lift dépensé) ; `zombie_veto_active` **concorde** avec le comportement observé de `create_recurring_task_if_absent`. Le test d'accord de T2/§3.1. |
| `mika2360_zombie_veto_flag_reads_lift_spent_on_a_sibling_row` | **T6 a, le test qui distingue les deux implémentations.** Deux lignes du même label : l'une porte `unknown_trigger_death`, l'*autre* porte `unknown_trigger_lift_consumed`. Une lecture par ligne rend un veto faux ; la sous-requête corrélée rend le même verdict que `create_recurring_task_if_absent`. Casse mélangée entre les deux lignes, pour épingler la collation du groupement en même temps que la corrélation. |
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
| `mika2360_admin_read_forwards_with_internal_token` | Serveur amont factice (`agent_base_url`) : le hop porte bien `Bearer {internal_token}` et frappe `/api/v1/recurring-tasks`. **Seul test de ce tableau à vivre dans `tests/admin_tenant_recurring_tasks.rs`** (harnais DB-backed (2)) — il doit franchir la résolution en base de R12 terme 2, que le pool paresseux du harnais inline ne peut pas servir. |
| `mika2360_admin_read_forwards_only_allowlisted_query_params` | `?agent_id=x&per_page=5&evil=1` → l'amont voit les deux premiers, jamais le troisième. |
| `mika2360_admin_read_upstream_failure_is_502_not_empty_200` | Amont injoignable → `502`. |
| `mika2360_no_mutating_method_on_admin_read_route` | `POST`/`PUT`/`DELETE`/`PATCH` sur le chemin → `405`. AC3, tenue par le routeur. |
| `mika2360_non_uuid_customer_id_is_rejected_before_any_forward` | **R12/T5, le test qui compte.** `customer_id = "x.attacker.example/"` (et un jeu de variantes : `../`, `a@b`, `id:8080`) → `400`, **et le serveur amont factice n'a reçu aucune requête**. L'assertion porte sur le compteur de l'amont, pas seulement sur le code : un `400` rendu *après* un forward aurait déjà fui le jeton. |
| `mika2360_unknown_customer_id_is_404_and_does_not_forward` | Un UUID bien formé mais absent de `customers` → `404`, zéro requête amont. R12 terme 2. |
| `mika2360_internal_token_never_reaches_an_unvalidated_host` | Garde de non-régression de la classe T5 : sur l'ensemble des entrées refusées ci-dessus, aucune requête sortante n'est émise — donc `internal_token` n'a pu partir nulle part. Le test échoue si quelqu'un déplace un jour la validation *après* la construction de l'URL. |

### Ce que le harnais de test de la gateway permet, et ce qu'il interdit

À lire avant d'écrire les tests. **La gateway a DEUX harnais, et une passe antérieure de ce
plan les a confondus** — la confusion tirait AC1 vers le bas sans raison, donc la rectification
est écrite ici plutôt que corrigée en silence.

**(1) Le harnais inline `#[cfg(test)]` n'a pas de Postgres.** Il construit un pool paresseux
sur un DSN factice — `PgPoolOptions::new().connect_lazy("postgres://fake:fake@localhost/fake")`
(`src/orchestrator_inbox.rs:531` et cinq sites dans `src/github.rs`) — et
`src/orchestrator_inbox.rs:419-421` écrit noir sur blanc que tout test y touchant réellement la
base « 1s-timeout on a real DELETE. Needs a docker-postgres or `sqlx::test!` harness — out of
scope ». C'est là que vivent les tests de routage et d'auth.

**(2) Le répertoire `crates/mika-gateway/tests/` est DB-backed, et c'est une convention établie,
pas une exception.** Cinq fichiers la suivent à l'identique (`admin_customers.rs`,
`admin_customers_read.rs`, `unlink.rs`, `pairing_rejection.rs`,
`audit_events_gateway_webhook.rs`), chacun avec les trois mêmes traits :

- `#[ignore = "requires a live Postgres at MIKA_DATABASE_URL / DATABASE_URL"]` — une raison
  **machine-lisible** dans l'attribut, pas un commentaire au-dessus ;
- un doc-comment de tête donnant la **commande exacte** de lancement
  (`MIKA_DATABASE_URL=… cargo test -p mika-gateway --test <fichier> -- --ignored --nocapture`) ;
- un **skip gracieux** — la variable absente fait `eprintln!("SKIP: …")` et rendre, plutôt
  qu'échouer.

Les deux fichiers que la passe antérieure citait comme preuves du harnais (1) appartiennent en
fait au (2) : leur ligne 25 / 23 est un `use sqlx::postgres::PgPoolOptions;`, et leur
`PgPoolOptions::new()` (ligne 60 / 54) porte une **vraie** URL lue dans l'environnement. Aucun
`connect_lazy` dans ce répertoire.

**Ce que la rectification change.** `admin_customers_read.rs` n'est pas un voisin quelconque :
c'est le test DB-backed de `GET /admin/customers/{id}`, c'est-à-dire du handler que R12 terme 2
décalque. Le chemin nominal complet de cette route est donc testable **sous une convention qui
existe déjà**, et son `#[ignore]` cesse d'être un aveu pour devenir la disposition normale de sa
famille. AC1 gagne un test exécutable — par une commande écrite — au lieu de reposer sur la seule
vérification manuelle.

Conséquences, et elles tombent du bon côté :

- **Le terme 1 de R12 est pleinement testable**, et c'est celui qui ferme la SSRF :
  `Path<Uuid>` rejette dans l'extracteur, donc **avant** le handler et avant toute requête
  DB. Les trois tests T5 du tableau ci-dessus tournent sans Postgres.
- **La propriété de sécurité elle-même est testable dans les deux cas.** Sur un
  `customer_id` non-UUID → `400` sans requête ; sur un UUID valide avec pool factice → la
  résolution échoue et le handler **ne forwarde pas** non plus. Dans les deux branches
  l'assertion « le serveur amont factice n'a reçu aucune requête » tient, donc
  « `internal_token` n'a pu partir nulle part » est vérifié sans base de données. C'est la
  raison de préférer une assertion sur le compteur de l'amont à une assertion sur le code de
  retour.
- **Les tests d'auth ne touchent pas la base** (le middleware ne lit que l'`AppState`) : les
  cinq tests `401` / `403` / `404` du tableau tournent tels quels.
- **`mika2360_admin_read_forwards_with_internal_token` (le chemin nominal complet) ne peut pas
  vivre dans le harnais (1)** : il exige de franchir la résolution en base, et un pool paresseux
  y répond par un timeout d'une seconde. Le dire évite qu'un implémenteur lise ce timeout comme
  un défaut de son code. Sa place est le harnais **(2)**, dans un fichier
  `crates/mika-gateway/tests/admin_tenant_recurring_tasks.rs` calqué sur
  `admin_customers_read.rs` : `#[ignore = "requires a live Postgres at MIKA_DATABASE_URL /
  DATABASE_URL"]`, commande de lancement en tête de fichier, skip gracieux. **AC1 est donc portée
  par trois choses** — le test tenant (`mika2360_recurring_registry_returns_paginated_shape`,
  côté SQLite, sans contrainte), ce test bout-en-bout exécutable à la demande, et la vérification
  manuelle post-déploiement. Le `#[ignore]` n'y est pas une AC affaiblie : il est la disposition
  de ses cinq voisins, pour la raison qu'ils nomment tous.

**Décision de conception qui en découle — l'échec de la résolution est fail-closed.** Si la
requête `SELECT 1 FROM customers WHERE id = $1` échoue (base indisponible, pool mort), le
handler rend `503` et **ne forwarde pas**. Jamais l'inverse : un fail-open « la base ne répond
pas, forwardons quand même » rouvrirait T5 en entier le jour d'une panne Postgres, c'est-à-dire
le jour où l'opérateur a le plus de raisons d'appeler cet endpoint. Le sens du repli est le
même que celui de R7/R8 : sur ce chemin, l'indisponibilité refuse, elle n'élargit pas.

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

# R12/T5 — un customer_id qui tente de détourner l'URL est refusé en 400,
# et l'hôte visé ne voit jamais passer de requête (à vérifier côté cible).
curl -s -o /dev/null -w '%{http_code}\n' \
  -H "Authorization: Bearer $MIKA_GATEWAY_ADMIN_READ_TOKEN" \
  "$GW/admin/tenants/x.attacker.example%2F/recurring-tasks"   # attendu : 400

# R12 — un UUID bien formé mais inconnu
curl -s -o /dev/null -w '%{http_code}\n' \
  -H "Authorization: Bearer $MIKA_GATEWAY_ADMIN_READ_TOKEN" \
  "$GW/admin/tenants/00000000-0000-0000-0000-000000000000/recurring-tasks"  # attendu : 404

# R9 — l'accès a laissé une trace
# SELECT target_key, created_at FROM audit_events
#   WHERE tool_name = 'gateway_admin_read' ORDER BY created_at DESC LIMIT 5;
```

### Commandes

`cargo test -p mika-agent`, `cargo test -p mika-gateway`, `cargo clippy --all-targets
-- -D warnings`, `cargo fmt --check`.

---

## Definition of Done

- [ ] `RecurringRegistryRow` + `list_recurring_registry` dans `db.rs`, projection fermée,
      `trigger_type` littéral, tri `label COLLATE NOCASE, created_at` (T6 b).
- [ ] `zombie_veto_active` calculé, **le terme de lift en sous-requête corrélée sur
      `(agent_id, label COLLATE NOCASE)` et non sur le `metadata` de la ligne** (T6 a), et son
      accord avec `create_recurring_task_if_absent` soit **rendu impossible à rompre**
      (fragment SQL partagé), soit **épinglé** par les deux tests d'accord.
- [ ] Wrapper async sans émission de frame.
- [ ] `handle_recurring_registry` + montage `/api/v1/recurring-tasks` sous l'auth dashboard.
- [ ] `gateway_admin_read_token` dans `GatewaySettings`, `admin_read_token` dans `AppState`,
      rédigé dans le `Debug` manuel en `Option::map` — l'état d'armement reste lisible, le
      secret non (T7 a).
- [ ] R8 résolu à la construction de l'`AppState` (jamais par requête), `WARN` nommé.
- [ ] `require_admin_read_token` avec ses cinq branches dans l'ordre spécifié.
- [ ] Route gateway + handler proxy sur le décalque `handle_a2a_agent_card`
      (`a2a_routes.rs:166`), query string en liste blanche, `502` sur échec amont.
- [ ] **La route porte son `.route_layer(require_admin_read_token)`** — aucun layer n'est
      monté sur le préfixe `/admin`, donc une route sans son layer est servie sans auth
      (T7 c).
- [ ] **R12 : `Path<Uuid>` + résolution en base, tous deux AVANT `container_url_str`.** La
      garde de non-régression T5 passe (aucune requête sortante sur entrée refusée).
- [ ] Ligne INFO de démarrage disant l'état d'armement (R7).
- [ ] Ligne `audit_events` `tool_name = 'gateway_admin_read'` par accès servi,
      fire-and-forget, via une constante dédiée distincte de `audit_events::TOOL_NAME` (R9).
      Aucune migration Postgres.
- [ ] **`openapi.rs` : aucune modification** — la surface `/admin/*` est hors du spec public
      (T7 b). Ne pas annoter le handler.
- [ ] `crates/mika-gateway/CLAUDE.md` : la route ajoutée à la table `## Endpoints` (colonne
      Auth = *Admin read token*, pas *Internal token*) et `MIKA_GATEWAY_ADMIN_READ_TOKEN` à
      `## Gateway Environment Variables` (T7 d).
- [ ] Les 24 tests ci-dessus passent (8 DB, 4 tenant, 12 gateway dont **1 dans
      `tests/admin_tenant_recurring_tasks.rs`**) ; clippy et fmt propres. Les 23 non-`#[ignore]`
      passent en CI ; le 24e passe contre un Postgres jetable par la commande écrite en tête de
      son fichier.
- [ ] Root `CLAUDE.md` : `MIKA_GATEWAY_ADMIN_READ_TOKEN` documenté (valeur, défaut, R7/R8,
      surfaces opérateur) ; `.env.example` mis à jour.
- [ ] Échec de la résolution en base → `503` sans forward (fail-closed, §*harnais de test*).
      **Le décalque `handle_get_customer` rend `500` sur ce cas ; la divergence est voulue et
      porte sur ce seul code** (T7 d) — copier le voisin ici manquerait le DoD.
- [ ] Le test du chemin nominal complet vit dans `crates/mika-gateway/tests/`, calqué sur
      `admin_customers_read.rs` : `#[ignore = "requires a live Postgres at MIKA_DATABASE_URL /
      DATABASE_URL"]`, commande de lancement en doc-comment de tête, skip gracieux si la
      variable est absente.
- [ ] Corps de PR : nomme la divergence 401/403 (T3), **la parade SSRF R12/T5**, et le ticket
      compagnon mika-cloud.
- [ ] Ticket compagnon mika-cloud ouvert (§3.4).

---

## Acceptance criteria

Transcrits verbatim du corps de `mika issue#2360`, suivis de ce qui les rend vérifiables.

- **AC1** — `GET /admin/tenants/{id}/recurring-tasks` avec token admin read → JSON du
  registre récurrent du tenant.
  → `mika2360_recurring_registry_returns_paginated_shape` (tenant, SQLite, sans contrainte de
  harnais) + `mika2360_admin_read_forwards_only_allowlisted_query_params` pour la forme du hop
  + `mika2360_admin_read_forwards_with_internal_token` pour le bout-en-bout, dans
  `tests/admin_tenant_recurring_tasks.rs` sous la convention DB-backed de ses cinq voisins
  (`#[ignore]` à raison machine-lisible, commande en tête de fichier, skip gracieux ; cf.
  § *harnais de test*) + la vérification manuelle post-déploiement. **Le `#[ignore]` est la
  disposition normale de cette famille de tests, pas une AC affaiblie** — et AC1 se juge de
  toute façon en traversant un pod réel.

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

**Deux exigences ne répondent à aucune AC du ticket, et c'est assumé.** R9 (traçabilité) et
**R12 (parade SSRF)** sont des ajouts. R12 n'est pas optionnel pour autant : sans elle le
livrable satisferait les quatre AC à la lettre tout en ouvrant un chemin d'exfiltration du
jeton d'écriture (T5) — une AC verte sur un endpoint qu'il faudrait retirer de production le
lendemain. Les quatre AC restent un sous-ensemble strict de ce qui est livré.

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
| **SSRF via `customer_id` → exfiltration de l'`internal_token`** (T5) — le risque le plus grave du ticket, et celui qui inverserait son propos | R12 : `Path<Uuid>` puis résolution en base, tous deux avant `container_url_str`. Épinglé par trois tests dont une garde qui assert **l'absence de requête sortante**, pas seulement le code de retour. |
| Le veto est lu par un chemin JSON inexistant et répond `false` avec assurance | Les trois chemins sont consommés par leurs constantes (`db.rs:66`, `:86`, `:102`), jamais recopiés — cf. la note de T2 sur `_lift_spent`, qui est exactement cette faute commise une fois. |
| **Le lift est lu sur la ligne projetée au lieu du groupe `(agent_id, label)`** — le veto répond faux sur la configuration même que mika#2337 produit (T6 a) | Sous-requête corrélée exigée par R1/§3.1, épinglée par `mika2360_zombie_veto_flag_reads_lift_spent_on_a_sibling_row`, qui est construit pour que la lecture par ligne échoue. |
| Deux lignes du même label en casses différentes sont rendues non contiguës, et le registre se lit comme deux récurrences distinctes sur le cas D1 (T6 b) | `ORDER BY label COLLATE NOCASE` + `mika2360_registry_orders_labels_case_insensitively`, dont le jeu porte une ligne intermédiaire — un jeu mono-casse ne peut pas voir la régression. |
| En single-tenant (`agent_base_url = Some`) un id inconnu rendrait le registre d'un autre | R12 terme 2 exigé même dans ce mode ; nommé en fin de T5 pour qu'un test local vert ne soit pas lu comme une preuve de routage. |
| **La route est montée sans son `.route_layer` et se retrouve servie sans aucune auth** — le préfixe `/admin` ne protège rien par lui-même (T7 c) | Les onze middlewares admin sont posés par route ; `mika2360_admin_read_rejects_missing_header_with_401` échoue si le layer manque. Le risque est nommé pour que ce test ne soit pas jugé redondant avec les quatre autres tests d'auth. |
| Un implémenteur copie le `500` de `handle_get_customer` sur l'échec de résolution et manque le fail-closed `503` (T7 d) | La divergence est écrite au site (§3.3, commentaire du handler) **et** dans le DoD, pas seulement dans la section harnais. |

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
