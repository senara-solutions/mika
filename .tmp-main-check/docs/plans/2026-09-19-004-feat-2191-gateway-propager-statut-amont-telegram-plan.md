---
title: Le statut d'amont Telegram cesse d'être de la prose — Plan
type: feat
date: 2026-09-19
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
issue: senara-solutions/mika#2191
---

# Le statut d'amont Telegram cesse d'être de la prose — Plan

## Goal Capsule

- **Objectif :** quand Telegram refuse un token à la validation, `POST /admin/customers` porte le statut d'amont dans un champ fermé (`upstream_status`) au lieu de le laisser dans une phrase. Le Console (mika-cloud) cesse d'avoir à reconnaître une chaîne de caractères pour distinguer « ce token est mort » de « le gateway a eu un hoquet ».
- **Moyens :** un lecteur unique de la question « quel statut l'amont a-t-il renvoyé ? » sur `TelegramApiError` (U1) ; le corps d'erreur de validation de token devient une fonction pure qui est le seul producteur du contrat de fil (U2) ; le message `error` existant est épinglé **par son littéral** parce qu'il est, lui aussi, un format de fil (U3) ; la documentation de la route (U4).
- **Autorité :** le corps de mika#2191 > le commentaire opérateur du 19/09 (qui porte le motif du dé-parquage, pas de correction de trajectoire technique). Le corps **tranche déjà** le nom du champ : `upstream_status`, entier, fixé par le consommateur. Ce plan ne rouvre pas ce choix.
- **Conditions d'arrêt.**
  - **(a) NON DÉCLENCHÉE.** « Arrêter si le correctif vit dans mika-cloud. » Il n'y vit pas : le ticket dit explicitement « Aucune modification du Console n'est requise pour que ce ticket prenne effet ». La totalité du travail est dans `crates/mika-gateway`.
  - **(b) LEVÉE PAR CONSTAT, et le constat est une limite à écrire.** `ls /data/workspace/mika-platform/` rend `claude-pilot` et `mika` — **pas `mika-cloud`**. `classify_gateway_error` et son test ne sont pas lisibles depuis ce worktree. Le ticket affirme qu'ils existent et passent au vert sur une valeur que personne n'émet encore ; ce plan **prend cette affirmation pour ce qu'elle est** et n'en fait pas une vérification. Conséquence opérationnelle : le nom et le type du champ ne sont pas négociables ici (M6).
- **Profil d'exécution :** Rust, `crates/mika-gateway` seul. **Aucun changement de schéma, aucune migration, aucun changement de statut HTTP, aucune modification côté agent, aucun appel réseau nouveau.**
- **Finish/ship :** le pipeline `/mika` sur la branche `feat/2191/gateway-propager-le-statut-telegram-d` ouvre la PR qui clôt mika#2191.

---

## Product Contract

### Summary

`POST /admin/customers` valide le `bot_token` fourni en appelant `getMe` chez Telegram. Quand Telegram répond `401`, le gateway renvoie un `400` dont le corps est `{"error": "invalid bot_token: Telegram returned 401 Unauthorized"}`. Le `401` d'amont **est là** — dans la phrase. Aucun champ ne le porte.

Le Console de mika-cloud a besoin de cette distinction : sur token mort, son assistant d'inscription ne propose plus « réessayer sans retaper le token », il renvoie l'utilisateur chercher le token courant dans BotFather. C'est le dernier maillon de la chaîne qui a coûté le premier champion externe (mika-cloud#205). Faute de champ, `classify_gateway_error` reconnaît aujourd'hui la sous-chaîne `invalid bot_token` — et son propre commentaire la qualifie de **dette de transition**, pas de critère.

Le défaut n'est donc pas que le Console fasse la mauvaise chose : il fait la bonne, sur une base que rien n'oblige à rester stable. Une reformulation du message côté gateway ferait retomber le Console dans la branche « réessayer », c'est-à-dire droit dans le défaut qu'il vient de corriger, **sans qu'aucun test des deux dépôts ne rougisse**.

### Problem Frame

#### M1 — Le site est unique, et c'est ce qui rend le correctif petit

`handle_register_customer` (`routes.rs:1081`) est le **seul** chemin du crate qui valide un `bot_token`. `get_me()` n'a qu'un appelant (`routes.rs:1122`). Le `match` qui suit a trois bras :

| bras | corps produit | statut gateway |
|---|---|---|
| `Ok(actual)` avec `actual != payload.bot_username` | `{"error": "bot_username mismatch: provided '…' but Telegram returned '…'"}` | 400 |
| `Err(TelegramApiError::Unauthorized)` | `{"error": "invalid bot_token: Telegram returned 401 Unauthorized"}` | 400 |
| `Err(e)` (générique) | `{"error": "bot token validation failed: {e}"}` | 400 |

Le `set_webhook` qui suit plus bas dans le même handler échoue **fail-open** (`routes.rs:1205`, `warn!` + `webhook_registered: false`) : il ne produit jamais de corps d'erreur. Il n'y a pas d'autre endpoint admin qui valide un token — `/admin/customers/{id}` est en `GET` seul, `/unlink` ne touche pas au token.

**Conséquence :** trois bras, un fichier, aucun autre producteur à réconcilier. Le ticket est bien aussi petit que son AC le laisse croire.

#### M2 — `TelegramApiError` porte déjà un statut, mais de façon partielle et à provenance inégale

`telegram.rs:9-22` :

| variante | statut d'amont ? | provenance |
|---|---|---|
| `Unauthorized` | **401, certain** | construit uniquement sur `401 =>` (3 sites : 812, 848, 970) |
| `BotBlocked` | **403, certain** | construit uniquement sur `403 =>` (1 site : 813) |
| `RateLimited { .. }` | **429, certain** | construit uniquement sur `429 =>` (2 sites : 814, 849) |
| `Other { status, .. }` | **`status`, porté explicitement** | mais `status` vaut parfois `200` — voir M3 |
| `BadRequest { message }` | **ambigu** | parfois un vrai `400 =>` (817), parfois fabriqué **sans réponse HTTP** — voir M4 |
| `Network(reqwest::Error)` | **aucun** | aucune réponse n'est arrivée |

Le statut existe donc déjà dans le type ; ce qui manque est (a) une façon de le lire uniformément et (b) le transport de cette lecture jusqu'au corps JSON. Rien à inventer, une question à poser.

#### M3 — `Other { status: 200 }` est une convention de `get_me`, et la respecter naïvement produirait un mensonge de catégorie

Trois sites de `get_me` (`telegram.rs:978`, `984`, `995`) construisent `Other { status: 200, … }` pour dire « Telegram a répondu 200 et le corps est inexploitable » : parse JSON impossible, `ok: false`, `username` absent.

Poser `upstream_status: 200` sur un corps d'erreur serait **factuellement vrai et catégoriquement faux** : l'amont n'a pas refusé, il a répondu. Un consommateur qui lirait « champ présent ⇒ échec d'amont » se tromperait exactement là.

La règle qui ferme la classe sans coder en dur la valeur `200` est : **le champ répond « l'amont a-t-il refusé, et avec quel statut », pas « quel octet HTTP est passé sur le fil »** — donc `status >= 400` seulement. Elle reste générale pour 403, 429, 5xx et tout statut d'erreur futur, ce qui est exactement l'argument du ticket en faveur de `upstream_status` contre `code`.

#### M4 — `BadRequest` a une double provenance, donc il n'est pas mappable

`BadRequest` est construit sur un vrai `400 =>` HTTP (`telegram.rs:817`), **et aussi** localement sans qu'aucune requête n'ait été émise : `validate_file_path` (381, 386, 391, 397), la taille de média (902, 925), la détection de type (934).

Le mapper vers `400` affirmerait un statut d'amont là où il n'y a parfois eu **aucune réponse d'amont**. La règle maison s'applique dans son sens habituel — *un signal qu'on ne peut pas lire n'est jamais un terme satisfait* — donc `BadRequest ⇒ None`.

**Ce que cela coûte, nommé :** rien dans le périmètre du ticket. `get_me` mappe `401 => Unauthorized, _ => Other`, il ne produit **jamais** `BadRequest`. La variante est traitée pour l'exhaustivité du match et pour les appelants futurs, pas pour ce chemin.

#### M5 — Le test ne peut passer ni par le réseau ni par la base, et le dépôt a déjà tranché cette question

Deux obstacles, mesurés :

- `api_url` (`telegram.rs:370`) code en dur `https://api.telegram.org`. Il n'existe **aucune** surcharge de base URL. Le précédent est explicite et vaut décision : le doc-comment de `should_fall_back_to_plain` (`telegram.rs:672-682`) écrit que rendre `api_url` injectable pour un serveur de mock « élargirait la surface de production pour de l'observabilité de test » et que « la décision est ce qui doit être épinglé, et elle l'est ici ». `wiremock` est bien en dev-dependency, mais il ne peut pas intercepter un hôte codé en dur.
- `AppState.pool` (`routes.rs:91`) est un `PgPool` non optionnel. Un test `oneshot` sur le routeur exige un Postgres ; les tests d'intégration DB-backed du crate (`tests/admin_customers.rs`) sont `#[ignore]`'d pour cette raison.

**Le chemin déjà tracé par la maison est donc : extraire la décision en fonction pure et l'épingler là.** `should_fall_back_to_plain` est le précédent direct — même enum, même crate, même raison.

#### M6 — Le consommateur est hors du workspace, et c'est ce qui rend le nom du champ non négociable

`ls /data/workspace/mika-platform/` rend `claude-pilot` et `mika`. **`mika-cloud` n'est pas là.** `classify_gateway_error`, son échelle statut → code → texte, et le test que le ticket annonce comme « déjà écrit et vert sur une valeur que personne n'émet » ne sont pas lisibles d'ici.

Deux conséquences, toutes deux à écrire plutôt qu'à découvrir :

1. **Le nom (`upstream_status`) et le type (entier) sont fixés par le consommateur.** Le ticket le dit : « Le consommateur a déjà fixé le nom du contrat — il est en place et testé côté Console, en attente de ce qui l'alimente. » Choisir autre chose ici — `upstream_http_status`, une chaîne `"401"` — laisserait les deux dépôts attendre l'autre. Ce plan n'exerce aucun arbitrage de nommage.
2. **Aucune vérification de bout en bout n'est possible depuis ce worktree.** La sonde qui recoupe réellement les deux moitiés est post-déploiement (V5).

#### M7 — En réparant le couplage, il ne faut pas casser le couplage existant

La marche textuelle de `classify_gateway_error` reste en service **tant que mika-cloud ne l'a pas retirée**. Le ticket dit que son retrait sera « une suppression de code morte », ce qui suppose que l'échelle statut → code → texte a été écrite dans cet ordre — mais la suppression n'est pas datée, et elle n'est pas dans ce dépôt.

Donc, pendant une fenêtre de durée inconnue, **les deux mécanismes coexistent** et AC2 n'est pas une politesse : le littéral `"invalid bot_token: Telegram returned 401 Unauthorized"` est un format de fil vivant. Il n'est aujourd'hui épinglé par aucun test du crate — `grep` le trouve à un seul endroit, le site de production (`routes.rs:1142`). Une reformulation innocente passerait toute la CI.

C'est précisément le défaut que le ticket décrit — « le couplage tient sur une chaîne de caractères que rien n'oblige à rester stable » — et il serait singulier de livrer le champ structuré en laissant la chaîne aussi peu tenue qu'avant, pendant la fenêtre où elle porte encore la décision.

### Requirements

- **R1.** Un refus d'authentification de l'amont Telegram produit un corps qui porte le statut d'amont dans un champ fermé, lisible sans parser de texte. (AC1)
- **R2.** Le champ `error` conserve sa forme actuelle, byte pour byte, sur la branche 401. (AC2)
- **R3.** Un échec de validation qui n'est pas un refus d'authentification ne porte pas `upstream_status: 401` ; les autres branches restent distinguables entre elles. (AC3)
- **R4.** La règle décidant de la présence et de la valeur du champ est portée par un lecteur unique, à match exhaustif, et testée par variante. (AC4)
- **R5.** Le littéral que le Console reconnaît encore est épinglé, de sorte qu'une reformulation rougisse en CI au lieu de casser silencieusement l'appelant. (M7)

### Scope Boundaries

**Dans le périmètre :** `crates/mika-gateway/src/telegram.rs` (le lecteur de statut), `crates/mika-gateway/src/routes.rs` (les corps d'erreur de `handle_register_customer` et leurs tests), `crates/mika-gateway/CLAUDE.md` (la ligne de contrat).

**Hors périmètre, délibérément :**

- **`mika-cloud` dans son entier**, y compris le retrait de la marche textuelle de `classify_gateway_error`. Hors du workspace (M6), et le ticket le place explicitement hors de son propre périmètre.
- **Le statut HTTP du gateway lui-même.** Le ticket l'écrit : « Le statut HTTP du gateway lui-même n'a pas besoin de changer. » Le 400 reste un 400 — le changer casserait tout appelant qui distingue 4xx de 5xx, pour un gain nul.
- **La convention `Other { status: 200 }` de `get_me`** (M3). La corriger — par exemple en introduisant une variante `MalformedResponse` — changerait la sémantique d'un enum partagé par tout le rail Telegram (envoi, `getFile`, téléchargement de média, webhook). Rayon large, bénéfice nul pour ce contrat : la règle `>= 400` la contourne sans la toucher. **Dette nommée, non traitée.**
- **La double provenance de `BadRequest`** (M4), pour la même raison et avec la même règle de contournement.
- **Rendre `api_url` injectable** pour un test wiremock de bout en bout (M5). Refus déjà motivé par écrit dans le crate, sur le même enum ; ce plan l'emprunte, il ne le rouvre pas.
- **L'OpenAPI.** `docs/openapi/gateway.yaml` ne décrit **aucune** route `/admin/*` (ses `paths` sont `/livez`, `/orchestrator/inbox/*`, `/readyz`, `/send`, `/version`, `/webhook/telegram`). Y ajouter `POST /admin/customers` entière serait un travail de documentation distinct et non demandé ; ajouter le seul champ d'erreur d'une route absente est impossible.

---

## Planning Contract

### Key Technical Decisions

**KTD1 — `upstream_status` plutôt que `code`, et le choix est enregistré, pas refait.** Le ticket offre les deux et tranche : « Le premier est préférable : il est général et couvre les prochains statuts d'amont sans nouveau vocabulaire. » Le consommateur l'a déjà implémenté (M6). Un `code: "invalid_bot_token"` exigerait un nouveau symbole à chaque nature de refus (`rate_limited`, `bot_blocked`, …) et une table de correspondance à tenir dans deux dépôts ; l'entier n'en demande aucune. Décision reprise telle quelle.

**KTD2 — Le champ répond « l'amont a-t-il refusé, et avec quel statut », pas « quel octet HTTP est passé sur le fil ».** C'est la phrase qui décide des six lignes de la table ci-dessous et qui évite les deux pièges de M3 et M4 :

| variante | `upstream_status` | pourquoi |
|---|---|---|
| `Unauthorized` | `Some(401)` | seule provenance : un `401` HTTP |
| `BotBlocked` | `Some(403)` | seule provenance : un `403` HTTP |
| `RateLimited { .. }` | `Some(429)` | seule provenance : un `429` HTTP |
| `Other { status, .. }` si `status >= 400` | `Some(status)` | l'amont a bien refusé, avec ce statut |
| `Other { status, .. }` si `status < 400` | `None` | convention `get_me` : réponse **reçue** mais inexploitable — ce n'est pas un refus (M3) |
| `BadRequest { .. }` | `None` | provenance ambiguë : parfois aucun appel n'a eu lieu (M4) |
| `Network(_)` | `None` | aucune réponse, donc aucun statut |

Le seuil est `>= 400` et non `!= 200` : il exprime la règle (« famille d'erreur HTTP ») plutôt qu'une valeur observée, donc il couvre un futur `3xx` sans nouvelle décision.

**KTD3 — Deux fonctions, parce que ce sont deux questions, à deux endroits.**

- `telegram::upstream_status(&TelegramApiError) -> Option<u16>` — « quel statut d'amont ce refus porte-t-il ? » C'est une **propriété de l'erreur**, et elle doit vivre à côté de l'enum et des six sites qui le construisent : la connaissance « `Other{200}` signifie parse-failure » y est locale. L'exporter dans `routes.rs` la mettrait loin de ce qui la produit, et c'est exactement comment ces conventions dérivent.
- `routes::token_validation_error_body(&TelegramApiError) -> serde_json::Value` — « quel corps `POST /admin/customers` renvoie-t-il ? » C'est le **contrat de fil de la route**, et il consomme le précédent.

L'alternative — une seule fonction dans `routes.rs` — a été écartée pour cette seule raison de localité. `should_fall_back_to_plain` (`telegram.rs:683`) est le précédent qui place ce genre de prédicat du côté de l'enum.

**KTD4 — Les deux bras `Err` fusionnent en un seul, et c'est ce qui rend AC2 testable.** Aujourd'hui chaque bras fabrique son `json!` sur place ; après, le handler écrit `Err(e) => (StatusCode::BAD_REQUEST, Json(token_validation_error_body(&e))).into_response()`. La fonction décide **aussi** du message : `Unauthorized` rend le littéral historique, les autres rendent `format!("bot token validation failed: {e}")`.

Le bénéfice n'est pas l'économie de lignes : c'est que le corps que lit le Console devient productible dans un test sans réseau ni base, donc assertable byte pour byte.

**KTD5 — `match` exhaustif, aucun bras `_ =>`.** Modèle maison (`dispatch_substrate_diagnostic`, `hosting_ground_truth_line`) : une septième variante de `TelegramApiError` doit **ne pas compiler** tant que quelqu'un n'a pas décidé si elle porte un statut d'amont. Un `_ => None` ferait silencieusement de tout ajout futur un « pas de statut », ce qui est précisément la panne muette que ce ticket répare.

**KTD6 — Le test porte le littéral en dur, jamais la constante.** Asserter `body["error"] == INVALID_BOT_TOKEN_MESSAGE` ne prouverait rien : quiconque change la constante fait passer le test et casse le Console. Le test écrit la chaîne en toutes lettres, avec un commentaire disant qu'elle est consommée par `classify_gateway_error` dans mika-cloud et qu'un changement est une rupture **à dater**, pas une mise à jour de test. C'est la discipline « format de fil » déjà appliquée à `FILTER_*` (mika#2131) et aux noms de portes (mika#2323).

**KTD7 — Une garde structurelle légère sur le littéral, parce que le sujet du ticket EST la fragilité d'une chaîne.** Un scan de source refuse que `invalid bot_token` apparaisse ailleurs qu'au site unique de production. Dix lignes. On pourrait juger la garde disproportionnée pour un seul site — sauf que le ticket est intégralement écrit autour de « le couplage tient sur une chaîne que rien n'oblige à rester stable », et livrer le champ structuré en laissant la chaîne aussi peu tenue qu'avant serait réparer la moitié visible du problème pendant la fenêtre où l'autre moitié décide encore (M7).

### High-Level Technical Design

```
 POST /admin/customers
   │
   ├─ validations locales (gateway_url, plan, bot_username)   [inchangé]
   │
   └─ customer_tg.get_me()
        │
        ├─ Ok(actual) ── actual ≠ demandé ──▶ bot_username_mismatch_body()      [U2]
        │                                        { "error": "bot_username mismatch: …" }
        │                                        PAS de upstream_status  ── AC3
        │
        └─ Err(e) ─────────────────────────▶ token_validation_error_body(&e)    [U2]
                                                 │
                                                 ├─ message :
                                                 │    Unauthorized → littéral historique  ── AC2
                                                 │    sinon        → "bot token validation failed: {e}"
                                                 │
                                                 └─ upstream_status :
                                                      telegram::upstream_status(&e)       [U1]
                                                        Unauthorized      → 401  ── AC1
                                                        BotBlocked        → 403
                                                        RateLimited       → 429
                                                        Other{s} s ≥ 400  → s
                                                        Other{s} s < 400  → absent  (M3)
                                                        BadRequest        → absent  (M4)
                                                        Network           → absent

 Statut HTTP du gateway : 400 dans tous les cas — inchangé.
 Clé `error` : présente dans tous les cas — inchangée.
```

### Assumptions

- **A1.** Le champ attendu par `classify_gateway_error` s'appelle `upstream_status` et vaut un **nombre** JSON, pas une chaîne. Le ticket le dit deux fois (« valant l'entier du statut Telegram (`401` …) »). Non vérifiable depuis ce worktree (M6). Si l'hypothèse est fausse, le correctif est d'une ligne dans `token_validation_error_body`, et V5 le détecte au premier token mort réel.
- **A2.** Le Console lit le corps du `400` et non le seul statut HTTP. Le ticket le décrit ainsi (`classify_gateway_error` reconnaît une sous-chaîne du corps). Si c'était faux, le champ serait inerte plutôt que nuisible.
- **A3.** La marche textuelle reste en service côté Console pendant une fenêtre inconnue (M7). C'est ce qui rend AC2 contraignante ; le plan ne dépend d'aucune date pour son retrait.

---

## Implementation Units

### U1. Le lecteur unique du statut d'amont

`crates/mika-gateway/src/telegram.rs`, à côté de `should_fall_back_to_plain` :

```rust
pub(crate) fn upstream_status(err: &TelegramApiError) -> Option<u16>
```

`match` exhaustif sur les six variantes, **sans bras `_`** (KTD5), suivant la table de KTD2. Doc-comment portant les deux pièges qui décident de deux lignes sur six : la convention `Other { status: 200 }` de `get_me` (M3) et la double provenance de `BadRequest` (M4). Sans ces deux phrases écrites au bon endroit, la prochaine lecture rétablira le mapping « évident » et réintroduira le mensonge de catégorie.

### U2. Le contrat de fil de `POST /admin/customers`

`crates/mika-gateway/src/routes.rs` :

- `const INVALID_BOT_TOKEN_MESSAGE: &str = "invalid bot_token: Telegram returned 401 Unauthorized";` — le littéral historique, nommé, avec un commentaire disant qu'il est consommé par `classify_gateway_error` (mika-cloud#205) et que le changer est une rupture inter-dépôts.
- `fn token_validation_error_body(err: &TelegramApiError) -> serde_json::Value` — `{"error": …}` toujours, plus `"upstream_status": n` quand `telegram::upstream_status(err)` rend `Some(n)`.
- `fn bot_username_mismatch_body(provided: &str, actual: &str) -> serde_json::Value` — extraction du corps déjà construit inline dans le bras `Ok`. Quatre lignes, dont l'intérêt est de rendre AC3 **assertable** au lieu d'être déduite par lecture du handler.
- `handle_register_customer` : les deux bras `Err` fusionnent (KTD4) ; le bras `Ok` appelle la seconde fonction. Aucun autre changement au handler — ni statut, ni ordre des validations, ni journalisation.

### U3. Les tests (AC4, et la garde de M7)

Dans le `mod tests` de `telegram.rs` :

- `mika2191_upstream_status_par_variante` — table sur les six variantes, incluant **les deux contrôles négatifs porteurs** : `Other { status: 200 }` → `None` (M3) et `BadRequest` → `None` (M4). Sans eux, un mapping naïf passerait la suite entière.
- `mika2191_other_au_dessus_de_400_porte_son_statut` — `Other { status: 500 }` → `Some(500)`, `Other { status: 403 }` → `Some(403)`. Prouve que la règle est un seuil et non une liste.

Dans le `mod tests` de `routes.rs` :

- `mika2191_ac1_le_401_porte_le_statut_damont` — le corps rend `upstream_status == 401`, en tant que **nombre** JSON (`as_u64()`, pas `as_str()`).
- `mika2191_ac2_le_message_du_401_est_un_format_de_fil` — assertion **sur le littéral écrit en toutes lettres** (KTD6), avec le commentaire nommant le consommateur.
- `mika2191_ac3_les_branches_non_401_ne_portent_pas_401` — `Other { status: 500 }` porte `500` et pas `401` ; `Network` ne porte aucune clé `upstream_status` ; le corps de mismatch de `bot_username` ne porte aucune clé `upstream_status`. Les trois restent distinguables par leur `error`.
- `mika2191_la_cle_error_est_toujours_presente` — sur les mêmes cas : AC2 vaut pour toutes les branches, pas seulement pour le 401.
- `mika2191_le_litteral_na_quun_seul_site_de_production` — scan de source (KTD7) : `invalid bot_token` n'apparaît dans `crates/mika-gateway/src/` qu'à la définition de la constante, hors `#[cfg(test)]`.

### U4. La ligne de contrat dans la documentation

`crates/mika-gateway/CLAUDE.md` — la ligne `POST /admin/customers` de la table *Endpoints* gagne la mention du corps d'erreur : `error` (inchangé) plus `upstream_status` quand l'amont a refusé, avec la règle en une phrase (`>= 400` seulement, absent quand aucun refus d'amont n'est établi) et le renvoi à mika#2191 / mika-cloud#205.

Une ligne de table, pas une section : le fait est petit et il appartient à l'endroit où un lecteur cherche déjà le contrat de cette route.

---

## Verification Contract

- **V1.** `cargo test -p mika-gateway` — vert, y compris les six tests de U3.
- **V2.** `cargo clippy -p mika-gateway --all-targets` sans avertissement nouveau ; `cargo fmt --check`.
- **V3 — non-régression de forme.** Les tests DB-backed `#[ignore]`'d de `tests/admin_customers.rs` restent compilables et inchangés : aucune signature publique ni aucun `UPSERT_CUSTOMER_SQL` n'est touché.
- **V4 — contrôle négatif du scan.** Le test de U3 doit rougir si le littéral est dupliqué. À vérifier une fois à la main en ajoutant temporairement une seconde occurrence, puis en la retirant : un scan qui ne rougit jamais n'atteste rien.
- **V5 — sonde post-déploiement, et ses trois haltes.** Sur la prochaine inscription réelle avec un token révoqué (ou en rejouant l'appel avec un token volontairement invalide contre le gateway déployé) :
  ```bash
  curl -s -X POST https://<gateway>/admin/customers \
    -H "Authorization: Bearer $MIKA_INTERNAL_TOKEN" \
    -H 'Content-Type: application/json' \
    -d '{"customer_id":"<uuid>","name":"probe","bot_username":"probe_bot","bot_token":"<token-mort>"}' \
    | jq '{error, upstream_status}'
  ```
  - `upstream_status: 401` **et** `error` inchangé → le contrat est en vigueur des deux côtés du champ. Régime attendu.
  - **Halte 1 — le champ est absent.** Ne pas retoucher la règle par réflexe : établir d'abord que le binaire déployé porte le correctif (classe mika#2340). Un gateway antérieur rend exactement le corps d'avant, ce qui est indistinguable d'une règle qui n'aurait pas mordu.
  - **Halte 2 — le champ est là et le Console retombe quand même dans « réessayer ».** Le défaut est côté consommateur (nom, type, ou marche non atteinte), pas ici : A1 est en cause. **Ne pas ajouter un second champ** `code` « au cas où » — ce serait installer les deux vocabulaires que KTD1 a écartés.
  - **Halte 3 — `error` a changé de forme.** AC2 est violée et la fenêtre de M7 est encore ouverte : le Console est cassé **en silence**. Le scan de U3 aurait dû l'empêcher ; s'il ne l'a pas fait, c'est le scan qu'il faut réparer, pas le message qu'il faut réécrire après coup.

---

## Definition of Done

- `telegram::upstream_status` existe, est le **seul** lecteur de cette question dans le crate, et son `match` n'a pas de bras `_`.
- `POST /admin/customers` porte `upstream_status: 401` (nombre) quand Telegram refuse le token à `getMe`.
- Le champ `error` du 401 est byte-identique à l'actuel, et la clé `error` est présente sur **toutes** les branches d'erreur de validation.
- Aucune branche non-401 ne porte `upstream_status: 401` ; le mismatch de `bot_username` et l'échec réseau n'en portent aucun ; un `Other { status: 500 }` porte `500`.
- `Other { status: 200 }` et `BadRequest` ne portent pas de statut, et les deux raisons sont écrites au site de la décision.
- Le littéral historique a un site de production unique, épinglé par un scan de source dont le contrôle négatif a été exercé (V4).
- La ligne `POST /admin/customers` de `crates/mika-gateway/CLAUDE.md` décrit le corps d'erreur.
- V1–V4 verts ; V5 posée dans le corps de la PR avec ses trois haltes.
- Aucun changement de statut HTTP, de schéma, de signature publique, ni de comportement hors des trois bras du `match get_me()`.

---

## Acceptance criteria

Transcrites du corps de mika#2191.

- **AC1** — Quand Telegram répond `401` à la validation du token, le corps d'erreur de `POST /admin/customers` porte le statut d'amont dans un champ dédié, en plus du message existant.
- **AC2** — Le champ `error` existant conserve sa forme actuelle : aucun appelant lisant `error` ne casse.
- **AC3** — Un échec de validation dont la cause n'est **pas** un refus d'authentification ne porte pas `upstream_status: 401`. Les autres branches (`bot_username mismatch`, échec générique de validation) restent distinguables.
- **AC4** — Test : la branche `TelegramApiError::Unauthorized` produit un corps portant le statut d'amont, et une branche non-401 n'en porte pas.

*Note sur l'AC3 :* lue dans sa lettre — « ne porte pas `upstream_status: 401` », et non « ne porte aucun `upstream_status` ». Un `Other { status: 500 }` porte donc `500`, ce qui satisfait l'AC et réalise l'argument de généralité que le corps du ticket donne en faveur de `upstream_status` contre `code`. Les deux branches nommées par l'AC (`bot_username mismatch`, échec générique) restent distinguables par leur `error`, et le mismatch — qui vit dans le bras `Ok`, l'amont ayant répondu 200 — ne porte aucun statut.

*Note sur l'AC4 :* réalisée sans réseau ni base, par extraction des corps en fonctions pures (M5). Le test end-to-end sur le handler exigerait soit un Postgres (les tests DB-backed du crate sont `#[ignore]`'d), soit une surcharge de la base URL Telegram — refus déjà motivé par écrit dans le crate, sur le même enum. Deux contrôles négatifs portent la moitié « et une branche non-401 n'en porte pas » : `Other { status: 200 }` et `Network`.

*Ce que les AC ne couvrent pas et que ce plan ajoute :* l'épinglage du littéral (R5 / KTD7). Aucun AC ne le demande ; le corps du ticket en fait pourtant son diagnostic central (« le couplage tient sur une chaîne de caractères que rien n'oblige à rester stable »), et la chaîne reste décisionnelle tant que mika-cloud n'a pas retiré sa marche (M7).

---

## Sources

- `crates/mika-gateway/src/routes.rs` — `handle_register_customer` (1081), appel `get_me` et les trois bras (1122–1150), `set_webhook` fail-open (1205–1215), `AppState.pool` (91), routage `/admin/customers` (305–313).
- `crates/mika-gateway/src/telegram.rs` — `enum TelegramApiError` (9–22), `api_url` codé en dur (370), `BadRequest` fabriqué localement (381–397, 902, 925, 934), mapping de statuts de `send_message` (812–823) et de `getFile` (848–852), `should_fall_back_to_plain` + son doc-comment refusant l'injection d'URL (672–684), `get_me` (959–999) dont les trois `Other { status: 200 }` (978, 984, 995).
- `crates/mika-gateway/src/telegram.rs` § tests — `mika2291_ac2_fallback_fires_only_on_400` (2437–2462), précédent direct d'un test par variante sur le même enum.
- `crates/mika-gateway/Cargo.toml` — `wiremock` en dev-dependency (79), inutilisable ici faute de base URL surchargeable.
- `crates/mika-gateway/CLAUDE.md` § *Endpoints* — la ligne `POST /admin/customers` (mika#1609).
- `crates/mika-gateway/tests/admin_customers.rs` — tests DB-backed `#[ignore]`'d, et leur motif.
- `docs/openapi/gateway.yaml` — `paths` sans aucune route `/admin/*`.
- mika-cloud#205 (la distinction construite côté Console et la dette de transition), RT#009.
- mika#2131 (les noms d'un format de fil sont épinglés parce que deux orthographes coupent une population en deux), mika#2323 (même discipline sur les noms de portes), mika#2290 (`match` exhaustif sans bras `_` comme mécanisme de décision forcée), mika#2340 (établir le déploiement avant de conclure sur le code).
