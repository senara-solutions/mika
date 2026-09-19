# Plan — mika#2387 : endpoint admin READ-ONLY de l'historique des envois sortants d'un tenant

- **Ticket :** `mika issue#2387`
- **Type :** feat
- **Priorité :** p1 — débloque l'inspection `mika issue#2358` (récurrences en doublon vécues par Al le 2026-09-17)
- **Butoir dur :** **2026-09-24**. `cleanup_old_outbound_messages` (`routes.rs:2512-2519`) purge `created_at < now() - interval '7 days'` par lots de 1000, toutes les ~100 requêtes webhook. L'incident du 17/09 sort de la table le 24/09.
- **Repos touchés :** `mika` (`mika-gateway` seul). Companion `mika-cloud` : **armement du secret en production**, voir § *Chemin critique du butoir*.

---

## Le besoin, et ce que la lecture du code y ajoute

Lire l'historique des envois sortants d'un tenant sans `kubectl exec` (bloqué par le
classifier) ni requête RDS directe (VPC privé). Le ticket demande de calquer
`GET /admin/tenants/{id}/recurring-tasks` (mika#2360).

Le ticket décrit correctement l'intention. **Six lectures de l'arbre la déplacent**, et ce
sont elles qui décident la conception. Chacune est vérifiée, avec sa référence.

### C1 — Le pattern de référence est mergé et réutilisable

mika#2360 est dans `main` (commit `215bc151`, « feat(2360): endpoint admin read-only du
registre des récurrences, proxifié gateway (#2366) »). Quatre pièces se réutilisent **sans
modification** :

| Pièce | Emplacement | Réutilisation |
|---|---|---|
| `require_admin_read_token` | `routes.rs:1965-2000` | **Telle quelle.** 404 non armé → 403 absent → passe sur token read → 403 + WARN sur token d'écriture → 403 autre. |
| `resolve_admin_read_token` | `settings.rs:376-405` | Telle quelle. Déjà appelée une fois au démarrage (`main.rs:225`). |
| `AppState.admin_read_token` | `routes.rs:171` | Tel quel. |
| Harnais de test `mod mika2360` | `routes.rs:3221-3280` | `state()` / `call()` réutilisables ; pool **lazy qui ne connecte jamais**. |

Conséquence : **aucune nouvelle variable d'environnement, aucun nouveau scope d'auth,
aucune modification de la résolution du jeton.** Le ticket le demandait (« même token,
même audit, même surface de sûreté ») et le code le permet littéralement.

### C2 — Ce n'est PAS un proxy, et c'est la différence structurelle majeure

`handle_admin_tenant_recurring_tasks` (`routes.rs:2049`) **proxifie** : il construit
`container_url_str(...)` puis `{container}/api/v1/recurring-tasks` et fait un hop portant
`Bearer {internal_token}` (`forward_recurring_registry`, `routes.rs:2088-2150`). La donnée
vit chez le tenant.

`outbound_messages` est une table **de la gateway elle-même** (`migrations/002_outbound_messages.sql`,
écrite par `routes.rs:2303`, relue par `routes.rs:2490`, purgée par `routes.rs:2514`). Ce
nouvel endpoint **lit sa propre base**.

Ce que ça retire du pattern — et ce n'est pas une simplification cosmétique :

- pas de `container_url_str` (donc pas d'interpolation d'un argument dans un nom d'hôte) ;
- **le jeton interne ne quitte jamais le processus** sur ce chemin ;
- pas de 502, pas de dépendance à la joignabilité du pod, pas de `wiremock` amont ;
- pas de `to_query_pairs()` : les paramètres ne sont pas transmis, ils sont **exécutés**.

Ce que ça ajoute en responsabilité, symétriquement : **le cap de pagination devient celui
de la gateway** (voir D3). Sur #2360 la gateway ne faisait que transmettre `per_page` ; ici
un `per_page` non borné est un déni de service sur sa propre base.

Ce qui se réutilise reste l'essentiel : auth, validation `Path<Uuid>`, résolution en base
avant toute action, audit, discipline read-only.

### C3 — Le schéma ne porte AUCUNE colonne de contenu, ce qui change la NATURE du test négatif

```sql
CREATE TABLE outbound_messages (
    telegram_message_id BIGINT NOT NULL,
    chat_id BIGINT NOT NULL,
    agent_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (telegram_message_id, chat_id)
);
```

C'est la table entière. La contrainte STRICTE du ticket (« métadonnées SEULES, JAMAIS le
contenu ») est donc **structurellement déjà satisfaite aujourd'hui**.

C'est la différence avec mika#2360, où la contrainte était une *amputation* réelle :
`TaskResponse` portait `action_config_preview` et `result_preview`, et le plan #2360 a dû
écrire sa propre requête pour ne pas les publier. Ici il n'y a rien à retirer.

**Le test négatif de l'AC3 n'est donc pas une amputation : c'est un garde de régression.**
Et cela décide sa forme, parce que la forme naïve ne tiendrait pas :

> Une assertion par liste noire — `assert!(!body.contains("text") && !body.contains("payload"))`
> — est **vraie aujourd'hui et le resterait** le jour où quelqu'un ajoute une colonne
> `message_snippet` et l'expose. Elle ne peut pas voir ce qu'elle n'a pas anticipé, c'est-à-dire
> exactement le cas qu'elle existe pour attraper.

Seule une **allowlist exacte** de l'ensemble des clés de la réponse rougit sur *tout* champ
ajouté, quel que soit son nom. C'est R6/T1.

### C4 — La CI ne provisionne pas Postgres pour ce crate : un test DB-backed serait vert par ABSENCE D'EXÉCUTION

Les cinq tests d'intégration de `crates/mika-gateway/tests/` sont **tous** `#[ignore]`, et
leurs doc-comments le disent en toutes lettres — `admin_customers_read.rs:4` : *« `#[ignore]`
by design — CI does not provision Postgres for the gateway crate »*. Même disposition dans
`admin_customers.rs`, `unlink.rs`, `pairing_rejection.rs`, `audit_events_gateway_webhook.rs`.

Conséquence directe sur la contrainte STRICTE du ticket : **un test négatif écrit uniquement
comme test DB-backed ne tournerait jamais en CI.** Il serait vert sans rien mesurer — la
classe de panne que ce dépôt nomme partout (`mika#2205` : un scan silencieusement inactif se
lit exactement comme un scan qui n'a rien trouvé à faire).

Un ticket qui exige un test négatif n'est pas satisfait par un test négatif non exécuté.

**C'est le constat qui décide tout le contrat de vérification** : l'AC3, et avec elle chaque
invariant de sûreté qui *peut* l'être, doit être portée par au moins un test **exécuté en CI,
sans base de données**. Ce qui ne le peut structurellement pas (la sémantique SQL réelle) est
nommé comme tel en § *Ce que la CI ne garantit pas*, plutôt que découvert plus tard.

### C5 — `outbound_messages` n'a pas de `customer_id` : le lien tenant passe par `chat_id`, et c'est là que vit le risque de fuite

La table ne porte que `chat_id`. Le lien canonique est `customers.telegram_chat_id`
(`migrations/001_customers.sql` : `BIGINT UNIQUE`), déjà utilisé par le routage entrant
(`routes.rs:724`, `routes.rs:1855`). L'unicité garantit qu'un `chat_id` désigne au plus un
tenant.

Mais la colonne est **nullable** (`routes.rs:1369` : `telegram_chat_id: Option<i64>`), et
deux chemins produisent ce NULL en exploitation : un tenant provisionné non encore appairé,
et un tenant délié par `POST /admin/customers/{id}/unlink` (`routes.rs:1294` :
`SET telegram_chat_id = NULL`).

D'où le mode de panne précis, et il est sévère :

```sql
-- Réflexe « filtre optionnel ». Sur un tenant non appairé ($1 = NULL),
-- rend TOUTES les lignes de TOUS les tenants.
WHERE ($1::bigint IS NULL OR chat_id = $1)
```

Une fuite inter-tenants sur un endpoint d'inspection, déclenchée par l'état le plus banal
qui soit. La forme naïve concurrente (`WHERE chat_id = $1` avec `$1 = NULL`) rend 0 ligne —
correct par accident, par la sémantique de NULL en SQL, et non par conception.

**Aucune des deux ne doit décider.** D6 fait court-circuiter le handler **avant** toute
requête sur `outbound_messages` : la valeur NULL n'atteint jamais une clause `WHERE`. Le
fail-closed devient structurel, et — point qui compte au vu de C4 — **testable sans base**,
puisque la décision devient une fonction pure.

### C6 — Pas de surface OpenAPI à étendre

`docs/openapi/gateway.yaml` (385 lignes) ne contient aucune route `/admin`. mika#2360 ne
l'a pas étendue. Ce plan non plus : c'est une **décision de cohérence**, pas un oubli. Les
routes admin sont documentées dans `crates/mika-gateway/CLAUDE.md` § *Routes* (ligne 26 pour
#2360), et c'est là que #2387 s'inscrit.

---

## Décisions de conception

### D1 — `tool_name` inchangé (`gateway_admin_read`), `metadata.route` **paramétrée**

`log_admin_read` (`audit_events.rs:45-70`) code sa route en dur :

```rust
let metadata = json!({
    "route": "GET /admin/tenants/{customer_id}/recurring-tasks",   // ← audit_events.rs:48
    "customer_id": customer_id,
});
```

L'appeler tel quel depuis le nouveau handler **ferait mentir l'audit** : chaque lecture de
l'historique des envois serait enregistrée comme une lecture du registre des récurrences.
Un instrument qui répond faux est pire qu'un instrument absent. `route` devient donc un
paramètre — c'est la seule modification apportée à du code existant, et elle est forcée.

**L'arbitrage, lui, porte sur `tool_name`, et il est réversible.** Deux options :

| | **Retenue** — un nom, route discriminante | Écartée — second `tool_name` |
|---|---|---|
| « Qui a accédé aux données d'Al ? » | `WHERE tool_name='gateway_admin_read'` — une requête | Deux requêtes ou un `IN (...)` à maintenir |
| « Cet endpoint est-il utilisé ? » | `WHERE metadata->>'route' LIKE '%outbound-messages'` | `WHERE tool_name='gateway_admin_read_outbound'` |
| Populations comptables séparément | Oui, par `metadata->>'route'` | Oui, par `tool_name` |

Motif de la décision : **un seul scope d'auth = une seule population.** Les deux routes sont
armées par le même `MIKA_GATEWAY_ADMIN_READ_TOKEN`, portent le même `target_key`
(`tenant:{uuid}`) et répondent à la même question de sûreté — *qui a lu les données de ce
tenant*. La question principale veut un nom ; la question secondaire est servie par
`metadata`, qui existe pour ça.

**Le contre-argument, nommé plutôt qu'écarté en silence :** la doctrine mika#2368 (« deux
noms, et c'est ce qui sauve les sondes ») et mika#2131 (le vocabulaire des filtres est un
format de fil). Elle s'applique quand les populations doivent être séparables **par `grep`**
sur un flux de journal. Ici la surface d'audit est SQL par construction — le doc-comment de
`TOOL_NAME_ADMIN_READ` (`audit_events.rs:31-33`) donne une requête SQL, pas un grep — et
`metadata->>'route'` y est un discriminant de première classe.

Bascule si l'arbitrage se révèle faux : une constante et un appelant. Aucune migration,
aucune donnée réécrite.

### D2 — `since` optionnel avec défaut ; **un `since` illisible est un 400, jamais un défaut silencieux**

- Absent → défaut **7 jours**, c'est-à-dire toute la rétention : la table ne peut rien
  contenir de plus ancien (`routes.rs:2514`). Un opérateur qui omet la borne veut tout, et
  le défaut ne peut pas rendre davantage que ce qui existe.
- Présent et valide (ISO 8601 / RFC 3339) → utilisé.
- **Présent et illisible → 400**, avec la valeur fautive citée. Le remplacer silencieusement
  par le défaut ferait croire à l'opérateur qu'il lit depuis le 17 quand il lit depuis le 11 —
  un instrument qui ment sur sa propre fenêtre, sur le ticket dont l'objet *est* de compter
  des envois dans une fenêtre. Doctrine du dépôt : une valeur non reconnue est **dite**.

**`until` est ajouté**, même grammaire et même traitement de l'erreur. Ce n'est pas dans
l'AC ; c'est dans l'usage déclaré : #2358 veut « compter les veilles réellement envoyées
**ce jour-là** », ce qui est un intervalle, pas une borne basse. Coût : une clause SQL et un
champ. Sans lui, lire le 17/09 impose de paginer depuis maintenant à rebours.

Fenêtre vide (`until <= since`) → **400**, pas une liste vide : une liste vide se lit comme
« ce tenant n'a rien envoyé », qui est une réponse fausse à une question mal posée.

### D3 — Pagination bornée **par la gateway**, `has_more` plutôt que `total`

`page` (1-based, défaut 1) et `per_page` (défaut 100, **max 1000, clampé**). Le cap est ici
et non chez un tenant, par C2 : la gateway exécute la requête.

`has_more` est obtenu par `LIMIT per_page + 1` puis troncature — gratuit. Un `total` imposerait
un second `COUNT(*)` pour une valeur dont personne n'a besoin sur une fenêtre de 7 jours.

Enveloppe de réponse : `{ items: [...], page, per_page, has_more }`.

### D4 — Ordre **total** : `ORDER BY created_at DESC, telegram_message_id DESC`

Le second terme n'est pas cosmétique. La PK est `(telegram_message_id, chat_id)` ; deux
envois d'une même rafale partagent leur `created_at` à la milliseconde. Sans départage, la
pagination par `OFFSET` peut **dupliquer ou sauter** une ligne entre deux pages.

Or une rafale d'envois quasi simultanés est très exactement le phénomène que #2358 cherche
à compter. Un tri instable ferait fausser le décompte par l'instrument lui-même.

### D5 — **Pas de migration d'index**

`idx_outbound_messages_created ON (created_at)` existe déjà. La requête filtre
`chat_id = $1 AND created_at >= $2 [AND created_at < $3]`. Un composite `(chat_id, created_at)`
serait théoriquement meilleur. Il n'est pas retenu :

1. le volume est borné à 7 jours de trafic Telegram d'une flotte réduite ;
2. l'index existant sert la borne temporelle, la plus sélective sur une fenêtre courte ;
3. **une migration sur le chemin critique d'un butoir à six jours ajoute un risque de
   déploiement disproportionné au gain.**

Nommé plutôt qu'ignoré : à rouvrir si une mesure le demande — pas par réflexe.

### D6 — Fail-closed structurel sur le tenant sans `chat_id`

Résolution en deux temps, dans cet ordre :

1. `SELECT telegram_chat_id FROM customers WHERE id = $1`
   - `Err(_)` → **503** (base indisponible ; jamais une liste vide, qui se lirait comme « rien envoyé ») ;
   - `Ok(None)` → **404** (tenant inconnu — invariant mika#2360) ;
   - `Ok(Some(None))` → **200, liste vide, sans jamais interroger `outbound_messages`** ;
   - `Ok(Some(Some(chat_id)))` → requête filtrée sur cette valeur.
2. La requête sur `outbound_messages` ne reçoit **jamais** un `chat_id` nullable.

`Ok(Some(None))` n'est pas une erreur : le tenant existe et n'a rien envoyé sur Telegram
(non appairé, ou délié). C'est une réponse vraie.

Le court-circuit est ce qui rend la fuite de C5 **structurellement impossible** au lieu de
dépendre de la sémantique de NULL — et, par C4, il est **testable en CI** puisque la décision
est une fonction pure sur `Option<i64>`.

### D7 — La validation des paramètres précède **tout** accès à la base

Conséquence de D2 et de C4 réunis. Si `since` était parsé dans le corps du handler après la
résolution du tenant, alors : (a) une entrée malformée solliciterait la base avant d'être
refusée ; (b) le test du 400 serait impossible sans Postgres, puisque le pool paresseux du
harnais rendrait 503 avant d'atteindre le parse.

L'ordre — valider, puis résoudre, puis lire — est à la fois la bonne conception et la
condition de testabilité de l'AC en CI. Les deux poussent dans le même sens ; c'est ce qui
rend la décision facile.

---

## Requirements

**R1 — Route.** `GET /admin/tenants/{customer_id}/outbound-messages`, montée dans
`build_router` avec `.route_layer(middleware::from_fn_with_state(state.clone(), require_admin_read_token))`.
L'auth de ce routeur est **par route** : sans ce `route_layer` la route est servie sans
aucune authentification (le commentaire de `routes.rs:336-338` le dit déjà pour #2360).

**R2 — Auth : le middleware existant, inchangé.** 404 si non armé (avant toute lecture
d'en-tête) ; 403 si absent, inconnu, ou porteur du jeton d'écriture.

**R3 — Aucune méthode mutante.** Seul `get(...)` est monté : POST/PUT/DELETE/PATCH → 405
par le routeur.

**R4 — `customer_id` validé avant toute action.** `Path<Uuid>` refuse un non-UUID en 400
dans l'extracteur. Un UUID inconnu → 404 sans lecture de `outbound_messages`.

**R5 — Paramètres : allowlist explicite, validée avant tout accès base** (D7). `since`,
`until`, `page`, `per_page`. Tout autre paramètre est ignoré. `since`/`until` illisibles ou
fenêtre vide → 400 citant la valeur fautive. `per_page` clampé à `[1, 1000]`.

**R6 — Métadonnées seules, tenu par une struct explicite et une projection nommée.** La
réponse ne porte **que** `telegram_message_id`, `chat_id`, `agent_name`, `created_at`. La
projection SQL est une **constante nommée** énumérant ses colonnes ; `SELECT *` est
proscrit. Deux gardes indépendants (T1 sérialisation, T2 scan de la constante) : la struct
seule ne verrait pas un passage à `serde_json::Value`, la constante seule ne verrait pas un
champ ajouté à la struct.

**R7 — Filtrage tenant fail-closed** (D6). Aucun `chat_id` nullable n'atteint la clause
`WHERE` ; un tenant sans `chat_id` rend une liste vide sans requête.

**R8 — Ordre total et pagination bornée** (D3, D4).

**R9 — Audit par lecture servie.** Une ligne `audit_events` par réponse 200 :
`tool_name = 'gateway_admin_read'`, `target_key = 'tenant:{uuid}'`,
`metadata.route = 'GET /admin/tenants/{customer_id}/outbound-messages'` (D1).
**Fire-and-forget** : un échec d'écriture logge un WARN et ne change pas la réponse — une
lecture ne doit pas dépendre d'une écriture (contrat de `log_admin_read`,
`audit_events.rs:43-44`).

**R10 — Aucune nouvelle variable d'environnement, aucun nouveau scope** (C1).

**R11 — Documentation.** Une ligne dans le tableau des routes de `crates/mika-gateway/CLAUDE.md`
(après la ligne 26 de #2360), nommant : le jeton, la discipline métadonnées-seules, le
fail-closed tenant, la forme de la ligne d'audit, et **la rétention 7 jours** — l'endpoint
ne lit pas plus loin que la purge, et l'appelant doit le savoir avant de conclure d'une
absence.

---

## Contrat de vérification

La ligne de partage est C4 : ce qui tourne en CI, et ce qui ne tourne que sur une base
montée à la main.

### Exécutés en CI, sans Postgres — c'est là que vit la garantie

Harnais : `mod mika2387` dans `routes.rs`, calqué sur `mod mika2360` (`routes.rs:3221`),
pool paresseux qui ne connecte jamais.

| Test | Ce qu'il tient |
|---|---|
| `mika2387_response_keys_are_exactly_the_four_metadata_fields` | **AC3.** Sérialise une instance **peuplée**, collecte l'ensemble des clés JSON, `assert_eq!` contre l'ensemble exact des quatre. Le contrôle positif est dans l'instance peuplée : un objet vide échoue au lieu de passer. Rougit sur **tout** champ ajouté — ce qu'une liste noire ne peut pas faire (C3). |
| `mika2387_select_list_is_an_explicit_allowlist` | **R6, second filet.** La constante de projection ne contient pas `*` ; l'ensemble des colonnes projetées est exactement l'allowlist. Attrape le passage de la struct à `serde_json::Value`, que T1 ne verrait pas. |
| `mika2387_admin_read_route_404_when_token_unconfigured` | R2. Non armé → 404, **y compris avec le jeton d'écriture**. |
| `mika2387_admin_read_rejects_internal_token_with_403` | R2. Authentifié mais non autorisé. |
| `mika2387_admin_read_rejects_missing_and_unknown_token_with_403` | R2, et **c'est ce 403 qui prouve que le `route_layer` est monté** : ce routeur n'a pas d'auth par préfixe. |
| `mika2387_no_mutating_method_on_outbound_messages_route` | R3. Les quatre verbes → 405. |
| `mika2387_non_uuid_customer_id_is_rejected_by_the_extractor` | R4. 400 avant le handler, donc avant toute requête. |
| `mika2387_unparseable_since_is_a_400_never_a_silent_default` | **D2/D7.** `since=pas-une-date` → 400. Ne passe en CI que parce que la validation précède la résolution en base (D7) : c'est le test qui **épingle cet ordre**. |
| `mika2387_empty_window_is_a_400` | D2. `until <= since` → 400. |
| `mika2387_per_page_is_clamped` | D3. Fonction pure : `0 → 1`, `10_000 → 1000`, absent → 100. |
| `mika2387_tenant_without_chat_id_yields_an_empty_list_without_querying` | **R7/D6, le garde anti-fuite testable sans base.** Fonction pure de décision sur `Option<i64>` : `None` ⇒ liste vide, aucune requête émise. |
| `mika2387_audit_route_names_this_endpoint` | **D1.** `metadata.route` du nouvel appel nomme `outbound-messages` et **pas** `recurring-tasks` (`assert_ne!` avec la route de #2360). Sans cette seconde moitié, un writer resté codé en dur passerait. |

### `#[ignore]`, DB-backed, manuels — le bout-en-bout

`crates/mika-gateway/tests/admin_tenant_outbound_messages.rs`, même disposition que ses
quatre voisins :

```bash
MIKA_DATABASE_URL=postgres://mika:mika@localhost/mika \
  cargo test -p mika-gateway --test admin_tenant_outbound_messages -- --ignored --nocapture
```

| Test | Ce qu'il tient |
|---|---|
| `outbound_messages_of_another_tenant_are_never_returned` | **L'anti-fuite réel.** Deux tenants, chacun ses lignes : la réponse de A n'en contient aucune de B. |
| `tenant_with_null_chat_id_returns_empty_not_everything` | **C5 en vrai.** Le tenant non appairé rend une liste vide, et surtout pas la table entière. |
| `since_and_until_bound_the_window` | D2 sur de vraies lignes de part et d'autre des bornes. |
| `pagination_neither_duplicates_nor_skips_across_pages` | **D4.** Plusieurs lignes au **même `created_at`** — le cas de rafale de #2358 : l'union des pages est exactement l'ensemble, sans doublon. |
| `a_served_read_writes_one_audit_row_naming_this_route` | R9 sur la vraie table. |

### Ce que la CI ne garantit pas — dit ici plutôt que découvert plus tard

La CI tient : la forme de la réponse, l'auth, le refus des entrées malformées, le
court-circuit fail-closed, le nom de la route auditée. Elle **ne tient pas** la sémantique
SQL réelle — c'est-à-dire que `outbound_messages_of_another_tenant_are_never_returned` et
`tenant_with_null_chat_id_returns_empty_not_everything`, les deux tests qui prouvent
l'absence de fuite inter-tenants, **ne tournent pas en CI**.

Cette limite est **héritée** de la disposition du crate (C4), pas introduite par ce ticket,
et la refermer — provisionner Postgres pour `mika-gateway` en CI — est un travail qui touche
les cinq fichiers de tests existants et le workflow : **hors périmètre, et un ticket de
suivi à ouvrir**. Ce que ce plan fait, c'est ne pas laisser croire que l'AC3 est tenue par
un test qui ne s'exécute pas : D6 déplace la part testable de l'invariant dans une fonction
pure, précisément pour que la CI en tienne ce qu'elle peut.

Les deux tests DB-backed sont donc à exécuter **à la main avant merge**, et le PR doit en
porter la sortie.

---

## Fire-Disposition

*(Exigée par le Fire-Disposition Gate — mika#1574, `docs/solutions/best-practices/fire-disposition-doctrine.md`.
Ce plan livre des détecteurs : T1 allowlist exacte des clés de réponse, T2 scan de la
constante de projection SQL, T3 `assert_ne!` sur le nom de la route auditée, plus les
détecteurs d'auth / 405 / 400 / clamp / fail-closed. Cette section dit comment chacun se
comporte face aux données et au code **préexistants**.)*

**Disposition retenue : (c) halt-and-surface.** Aucun allowlist d'exception, aucun
détecteur livré désarmé. La justification est **mesurée avant l'écriture**, pas
prudentielle : l'état courant du dépôt est à zéro violation pour chacun des détecteurs,
donc il n'y a rien à tolérer.

### Pourquoi (a) n'est pas retenue, alors qu'elle est le défaut de la doctrine

L'option (a) demande, par entrée, un nom de donnée spécifique, un ticket de suivi et une
assertion auto-nettoyante. Ici l'ensemble des entrées serait **vide** : un allowlist à
zéro entrée n'est pas un allowlist, c'est une égalité exacte — et c'est très exactement ce
que T1 et T2 écrivent déjà. L'introduire comme structure ajouterait une porte ouverte sans
rien tolérer aujourd'hui, sur les deux détecteurs qui gardent la contrainte STRICTE du
ticket. C3 dit pourquoi cette porte serait coûteuse : le mode de panne de l'AC3 est
précisément un garde qui ne voit pas ce qu'il n'a pas anticipé.

### État du préexistant, détecteur par détecteur

| Détecteur | Ce sur quoi il tire | État vérifié à l'écriture |
|---|---|---|
| T1 (clés de réponse) | Struct **neuve** de ce ticket | Le schéma n'a que les quatre colonnes autorisées (`migrations/002_outbound_messages.sql:2-5`). Rien à exclure. |
| T2 (projection SQL) | Constante **neuve** de ce ticket | Idem. Aucune colonne de contenu n'existe. |
| T3 (`assert_ne!` sur `metadata.route`) | **Code existant** — `log_admin_read` (`audit_events.rs:45`) | **Seul détecteur qui touche du préexistant.** `log_admin_read` a exactement **un** appelant de production (`routes.rs:2079`, la route #2360), et U1 lui fait passer littéralement la chaîne qu'il codait en dur : la route auditée de #2360 est inchangée, octet pour octet. Zéro violation. |
| Auth / 405 / 400 / clamp / fail-closed | Route **neuve** | Sans préexistant par construction. |

**U1 et son test de non-régression sont ce qui rend T3 vert sans toucher au comportement
de #2360.** C'est la condition qui rend (c) disponible ici, et elle est vérifiée, pas
espérée : si U1 devait changer la valeur observée par l'audit de #2360, la disposition
serait à rouvrir avant d'écrire la ligne.

### Ce qui se passe si un détecteur tire plus tard

**Toute violation future est une régression bloquante, et son remède n'est jamais
l'élargissement du détecteur.** Concrètement :

- **T1 ou T2 rougit après l'ajout d'une colonne à `outbound_messages`** (p. ex. un
  `message_snippet`, une `payload`) : **halte**. Ajouter la clé à l'allowlist pour faire
  repasser la CI publierait la donnée — c'est-à-dire exécuterait la violation que le
  détecteur venait d'attraper, en croyant réparer un test. Publier une nouvelle colonne
  sur cet endpoint est une **décision d'exposition de donnée**, qui remonte à l'opérateur
  et se tranche dans son propre ticket ; l'allowlist ne s'élargit que sur cette décision
  explicite, et le rouge est le comportement correct jusque-là.
- **T3 rougit** : un appelant de `log_admin_read` a cessé de nommer sa propre route, ou
  les deux routes ont convergé sur la même valeur. Le remède est de réparer l'appelant,
  jamais d'assouplir l'assertion — un audit qui répond faux est pire qu'un audit absent
  (D1).
- **Un détecteur d'auth rougit** : la route est servie sans `route_layer` (R1). Halte
  immédiate, avant merge — c'est le seul chemin par lequel cet endpoint devient public.

Aucun de ces cas n'est traité par `#[ignore]` : l'option (b) supposerait une violation
préexistante dangereuse à laisser non signalée, et il n'y en a aucune. Les cinq tests
`#[ignore]` du § *Contrat de vérification* relèvent de la disposition du crate (C4), pas
d'une disposition de tir — la distinction est faite ici pour qu'on ne lise pas l'une comme
l'autre.

---

## Découpage

| # | Unité | Fichiers |
|---|---|---|
| U1 | `route` paramétrée dans `log_admin_read` + son test de non-régression | `audit_events.rs` |
| U2 | Struct de réponse, constante de projection, fonctions pures (clamp, fenêtre, décision fail-closed) | `routes.rs` |
| U3 | Handler + montage de la route | `routes.rs` |
| U4 | `mod mika2387` (douze tests CI) | `routes.rs` |
| U5 | Test d'intégration `#[ignore]` (cinq tests) | `tests/admin_tenant_outbound_messages.rs` |
| U6 | Ligne du tableau des routes | `crates/mika-gateway/CLAUDE.md` |

U1 précède U3 (sans elle l'audit mentirait, D1). U2 précède U3 et U4. U5 est indépendante.

---

## Chemin critique du butoir (2026-09-24)

**L'endpoint mergé ne suffit pas.** Il ne sert que si `MIKA_GATEWAY_ADMIN_READ_TOKEN` est
**armé sur la gateway de production**. mika#2360 a été mergé le 17/09 ; si le secret n'est
pas encore posé côté `mika-cloud`, la route rend 404 (`resolve_admin_read_token` →
`require_admin_read_token` branche 1) et l'inspection de #2358 est impossible **même ce
ticket livré**.

Cette dépendance est **externe à ce dépôt** et sur le chemin critique. À vérifier
explicitement, pas à supposer :

1. au démarrage de la gateway, la ligne INFO `admin read route enabled
   (MIKA_GATEWAY_ADMIN_READ_TOKEN set)` (`settings.rs:401`). Son **absence** dit que le jeton
   n'est pas posé ; une ligne WARN `equals MIKA_INTERNAL_TOKEN` dit qu'il l'est mal ;
2. ```bash
   curl -s -o /dev/null -w '%{http_code}\n' \
     -H "Authorization: Bearer $MIKA_GATEWAY_ADMIN_READ_TOKEN" \
     "$GATEWAY/admin/tenants/$CUSTOMER_ID/outbound-messages?since=2026-09-17T00:00:00Z"
   ```
   `200` = servi ; `404` = **jeton non armé** (ou tenant inconnu — les distinguer par la
   route #2360, qui partage l'armement) ; `403` = mauvais scope.

**Ce ticket ne préserve aucune donnée, il la rend lisible avant sa purge.** Passé le 24/09
la fenêtre du 17/09 est sortie de la table et aucun rattrapage n'existe côté endpoint.
Allonger la rétention est une autre décision, hors périmètre — et la mentionner ici évite
qu'on croie ce plan capable de sauver l'historique après coup.

---

## Definition of Done

1. `GET /admin/tenants/{customer_id}/outbound-messages` monté avec `require_admin_read_token`.
2. Réponse limitée aux quatre métadonnées, tenue par une struct explicite **et** une
   projection SQL nommée sans `SELECT *`.
3. Filtrage tenant fail-closed : aucun `chat_id` nullable n'atteint la clause `WHERE`.
4. `since` / `until` / `page` / `per_page` validés **avant** tout accès base ; illisible → 400.
5. Ordre total, pagination bornée (`per_page` ≤ 1000), `has_more`.
6. Une ligne d'audit par lecture servie, nommant **cette** route ; fire-and-forget.
7. Les douze tests CI passent ; les cinq tests `#[ignore]` sont exécutés à la main et leur
   sortie figure dans le PR.
8. `cargo clippy` et `cargo fmt` propres ; `cargo test -p mika-gateway` vert.
9. `crates/mika-gateway/CLAUDE.md` documente la route, la discipline métadonnées-seules et
   la rétention 7 jours.
10. L'armement du jeton en production est **vérifié** (§ *Chemin critique*), pas supposé.

---

## Acceptance criteria

1. `GET /admin/tenants/{id}/outbound-messages?since=<ISO>` rend **200** avec les
   métadonnées seules ; **403** sans jeton ou avec un jeton erroné ; **404** lorsque le
   jeton admin est absent ou en collision avec `MIKA_INTERNAL_TOKEN` (invariant mika#2360).
2. La réponse est une liste de `{telegram_message_id, chat_id, agent_name, created_at}`,
   **paginée** et **bornée par `since`**.
3. **Test négatif** : un test exécuté **en CI** asserte que la réponse ne contient **aucune**
   clé de contenu ou de texte — par égalité exacte de l'ensemble des clés avec l'allowlist
   des quatre métadonnées, et non par liste noire (C3), et non par un test DB-backed que la
   CI n'exécute pas (C4).
4. Chaque lecture servie écrit une ligne d'audit `gateway_admin_read` dont la
   `metadata.route` nomme **cette** route et non celle de mika#2360.
5. Filtrage par tenant : la réponse ne contient que les envois du `chat_id` du tenant,
   résolu depuis `customers.telegram_chat_id` ; un tenant sans `chat_id` rend une **liste
   vide** et jamais les lignes d'un autre tenant.

---

## Risques et hors périmètre

**Risques**

| Risque | Traitement |
|---|---|
| Fuite inter-tenants via un `chat_id` NULL (C5) | D6 : court-circuit **avant** toute requête ; part testable en CI (fonction pure), part réelle en test `#[ignore]` exécuté à la main. |
| Une colonne de contenu ajoutée plus tard et exposée | R6 + T1 (allowlist exacte des clés) + T2 (projection nommée). La liste noire, elle, ne l'aurait pas vue. Le rouge est alors le comportement correct : § *Fire-Disposition* nomme la halte plutôt que l'élargissement de l'allowlist. |
| Un test négatif vert par absence d'exécution (C4) | Contrat de vérification scindé CI / manuel, et la limite écrite en § *Ce que la CI ne garantit pas*. |
| Audit mensonger sur la route lue (D1) | `route` paramétrée (U1) + `assert_ne!` contre la route de #2360. |
| Jeton non armé en production → 404 au moment de l'inspection | § *Chemin critique* : sonde au démarrage + sonde `curl`, à exécuter avant le 24/09. |
| Décompte faussé par une pagination instable sur une rafale | D4 : ordre total ; test `pagination_neither_duplicates_nor_skips_across_pages` sur des `created_at` identiques. |

**Hors périmètre, délibérément**

- **Provisionner Postgres en CI pour `mika-gateway`** — la vraie fermeture de C4. Touche les
  cinq fichiers de tests existants et le workflow. **Ticket de suivi à ouvrir**, et c'est la
  limite la plus importante de ce plan.
- **Filtre `?agent_name=`** — inutile à #2358 (le `chat_id` identifie le tenant, `agent_name`
  est une colonne de sortie). Extension d'une ligne si une mesure le demande.
- **Index composite `(chat_id, created_at)`** — D5.
- **Surface OpenAPI** — C6, cohérence avec mika#2360.
- **Allonger la rétention 7 jours** — autre décision, autre ticket. Ce plan rend l'historique
  lisible avant sa purge ; il ne le préserve pas.
- **L'analyse de #2358 elle-même** — cet endpoint fournit la donnée, il ne conclut pas sur
  les doublons vécus par Al.

---

## Revision history

- **rev 2 (2026-09-18)** : adressé **F1** (BLOCKING, Fire-Disposition Gate — mika#1574) en
  ajoutant la section `## Fire-Disposition` entre le contrat de vérification et le
  découpage. L'option canonique retenue est **(c) halt-and-surface**, et non (a) comme le
  suggérait le finding : l'ensemble des exceptions serait vide (le schéma
  `outbound_messages` n'a que les quatre colonnes autorisées, vérifié à
  `migrations/002_outbound_messages.sql:2-5`), or un allowlist à zéro entrée n'est pas un
  allowlist mais l'égalité exacte que T1 et T2 écrivent déjà — l'introduire ouvrirait une
  porte sans rien tolérer, sur les deux détecteurs qui gardent la contrainte STRICTE du
  ticket. La section couvre le reste du *Change required* tel quel : état du préexistant
  détecteur par détecteur (T3 est le seul à toucher du code existant — `log_admin_read`
  a un unique appelant de production, `routes.rs:2079`, dont U1 laisse la route inchangée
  octet pour octet), et clause explicite de traitement des violations futures — ajout
  d'une colonne dans la projection ou la struct ⇒ **régression bloquante**, halte, jamais
  l'élargissement de l'allowlist pour faire repasser la CI, la publication d'une colonne
  étant une décision d'exposition de donnée qui remonte à l'opérateur. Renvoi ajouté
  depuis la ligne correspondante du § *Risques*. Aucune AC modifiée ni affaiblie ; aucune
  autre section touchée.
