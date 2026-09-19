---
title: web_search — un 429 doit se nommer 429, et aucune branche d'échec ne doit parler d'opérateur à une famille — Plan
type: fix
date: 2026-09-19
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
issue: senara-solutions/mika#2407
---

# web_search — un 429 doit se nommer 429, et aucune branche d'échec ne doit parler d'opérateur à une famille — Plan

## Goal Capsule

- **Objectif :** l'échec de `web_search` cesse d'être un fourre-tout. Le rate-limit reçoit un nom qui traverse le substrat jusqu'à l'agent et jusqu'à `audit_events` ; le backoff cesse de retomber dans la fenêtre de débit qu'il est censé absorber ; et **aucune** branche d'échec ne sert de texte opérateur à un tenant famille.
- **Moyens :** une variante de taxonomie `rate_limited` au substrat (U1) ; un plancher de backoff supérieur à la fenêtre `w=1` de Brave (U1) ; `map_substrate_error` qui rend un **couple** (repli neutre, diagnostic opérateur) au lieu d'un texte unique servi au LLM (U2) ; le motif porté jusqu'à l'audit et au journal, tous tiers (U3) ; un garde structurel qui remplace celui que mika#1971 a fait disparaître (U4) ; les tests négatifs des AC, désormais réalisables de bout en bout (U5) ; la variable d'activation du substrat enfin documentée (U6).
- **Autorité :** ce plan > le corps de mika#2407 > le commentaire opérateur. **Le plan rectifie le diagnostic du ticket sur deux points mesurés sur le code**, et la rectification est le premier livrable — détail en § *Problem Frame*. Le commentaire opérateur (`ready`, motif p1 vécu) est confirmé : le défaut est réel, seule sa localisation change.
- **Conditions d'arrêt.**
  - **(a) DÉCLENCHÉE — rapportée, ne suspend pas le plan.** « Arrêter si le handler `web_search` ne parle pas à Brave. » Il ne lui parle plus depuis mika#1971 : il appelle `POST /internal/search` sur la gateway. Le retry 429 demandé par l'AC2 **existe déjà**, au substrat, depuis mika#1808. Le plan garde l'intégralité des AC et déplace leur site d'application ; deux défauts réels subsistent à ce site (plancher de backoff sous la fenêtre, taxonomie qui écrase le 429), plus un troisième que le ticket n'a pas vu et qui explique mieux son symptôme (M3).
  - **(b) NON LEVÉE — et c'est un résultat, pas un manquement.** « La veille d'Al échoue-t-elle par 429 ou par substrat non configuré ? » Départager exige de lire `audit_events` et le journal de la gateway du tenant, hors d'atteinte depuis ce worktree. Le plan livre l'instrument qui tranchera (U3) et **une sonde avec sa halte** (§ *Verification Contract*). Il ne conclut pas à la place de la mesure.
- **Profil d'exécution :** Rust, `crates/mika-gateway` (U1) + `crates/mika-agent` (U2–U5) + documentation (U6) ; tests inline `#[cfg(test)]` et `wiremock` (déjà dev-dep) ; **aucun changement de schéma**.
- **Finish/ship :** le pipeline `/mika` sur la branche `fix/2407/p1-tenant-web-search-choue` ouvre la PR qui clôt mika#2407.

---

## Product Contract

### Summary

La veille quotidienne d'Al échoue et son Mika lui répond que « l'outil de recherche web a un souci de configuration côté serveur (clé API manquante) ». Le ticket lit ce message comme une paraphrase inventée par le LLM à partir d'un échec neutre. La lecture du code dit autre chose et de plus grave : **il existe des branches où ce texte n'est pas inventé mais servi**. `map_substrate_error` produit littéralement « Ask the operator to set MIKA_BRAVE_API_KEY on mika-gateway », et ce texte arrive au LLM par `ToolOutput::error` — qui ne traverse pas le routage par tier de mika#1783. Sept des dix jetons de `FORBIDDEN_FAMILY_TIER_TOKENS` y figurent, et un test existant **asserte leur présence**. La doctrine mika#1783 et cette fonction se contredisent dans l'arbre depuis mika#1971, sans qu'aucun test ne les confronte.

Le 429 est réel par ailleurs, et mal servi : le substrat le retente déjà une fois, mais avec un plancher de 500 ms quand `Retry-After` est absent — **sous** la fenêtre d'une seconde que Brave annonce —, et un 429 persistant est aplati en `upstream_error`, indistinguable d'un 500. Ce plan nomme le rate-limit de bout en bout, relève le plancher au-dessus de la fenêtre, et rétablit la doctrine sur **toutes** les branches d'échec plutôt que sur la seule qui a été mesurée.

### Problem Frame

#### M1 — `web_search` ne parle plus à Brave, et le retry demandé existe déjà

Depuis mika#1971, `builtin_handlers.rs::web_search` appelle `POST {gateway_url}/internal/search` et son doc-comment dit en toutes lettres qu'il ne lit jamais `ctx.brave_api_key`. L'appel à Brave vit dans `crates/mika-gateway/src/egress_search/brave.rs`, qui porte depuis mika#1808 une politique de retry documentée dans son module-doc, avec 429 retryable et `Retry-After` honoré. Deux tests wiremock la couvrent déjà : `rate_limit_429_retries_once_then_succeeds` et `rate_limit_429_persists_after_retry_returns_upstream_status`.

**Conséquence sur l'AC2 :** « le handler applique un backoff + retry » est satisfait dans sa lettre. Ce qui ne l'est pas, ce sont ses deux détails opérationnels — et ils sont décisifs, voir M2.

**Conséquence sur le périmètre :** le correctif est à deux crates, et c'est structurel, pas un débordement. La taxonomie naît au substrat ; l'agent ne peut pas nommer un 429 que le substrat lui a livré sous le nom `upstream_error`.

#### M2 — Deux défauts réels au site réel

**(a) Le plancher de backoff est sous la fenêtre qu'il doit absorber.** `RETRY_BACKOFF_MS = 500` (`brave.rs:54`) est le repli quand la réponse 429 ne porte pas de `Retry-After`. La politique de débit annoncée par Brave est `1;w=1` : une requête par seconde. Un retry à 500 ms retombe **dans la même fenêtre** et reçoit un second 429 par construction. Le retry existe, il est calibré pour ne pas fonctionner sur la limite qui frappe Al.

**(b) Le 429 est aplati avant d'atteindre l'agent.** `SearchError::UpstreamStatus(429)` rend `tracing_status() = "upstream_error"` et `http_status() = 502`. Côté agent, `map_substrate_error(502, "upstream_error")` répond « Search upstream returned an error. Try again in a moment. » — le même texte qu'un 500. **Le code 429 est présent dans la variante et perdu à la frontière.** L'AC4 (distinguer `rate_limited` de `missing_key` dans `audit_events`) est donc inatteignable sans toucher la taxonomie du substrat : l'information n'arrive pas.

#### M3 — La fuite mika#1783 est ouverte sur toutes les branches non-config, et elle explique le symptôme au mot près

`web_search` n'utilise `ToolOutput::substrate_unavailable` que pour deux cas : `gateway_url` absent et `internal_token` absent. **Toutes** les autres sorties d'échec — y compris chaque branche de `map_substrate_error` — passent par `ToolOutput::error`, dont le champ `substrate_diagnostic` est `None`. `dispatch_substrate_diagnostic` est alors un no-op explicite (« Idempotent: if `output.substrate_diagnostic` is `None`, this is a no-op »), et le `content` part au LLM **quel que soit le tier**.

Ce que contiennent ces textes, confronté à `FORBIDDEN_FAMILY_TIER_TOKENS` (`builtin_handlers.rs:4022`) :

| branche | texte servi au LLM | jetons interdits présents |
|---|---|---|
| `(404, search_upstream_not_configured)` | « Search substrate is not configured on the gateway. Ask the operator to set MIKA_BRAVE_API_KEY on mika-gateway. » | `MIKA_BRAVE_API_KEY`, `operator`, `configuration` (via `configured`… voir note) |
| `(502, unauthorized)` | « …Ask the operator to rotate MIKA_BRAVE_API_KEY on mika-gateway. » | `MIKA_BRAVE_API_KEY`, `operator` |
| `(502, upstream_error)` | « Search upstream returned an error… » | — |

*Note :* le jeton `"configuration"` ne matche pas `"configured"` — la table est exacte sur ce point ; les deux premières lignes fuient par `MIKA_BRAVE_API_KEY` et `operator`, ce qui suffit.

**Cette fuite est l'explication la plus économique du symptôme exact d'Al.** « souci de configuration côté serveur (clé API manquante) » est une traduction fidèle de « Search substrate is not configured… set MIKA_BRAVE_API_KEY », bien plus qu'une invention à partir de « upstream returned an error ». Le ticket suppose une hallucination ; la lecture du code offre un chemin où le modèle dit la vérité qu'on lui a donnée.

**Le garde qui aurait dû l'empêcher a disparu.** Le commentaire de `web_search_family_tier_http_401_no_leak` (ligne 4180) renvoie à « the `web_search_no_raw_401_operator_error` source-scan test below », qui **n'existe plus** dans l'arbre (`grep -rn "fn web_search_no_raw_401_operator_error" crates/` ne rend rien). Il a vraisemblablement été retiré avec la branche 401 directe lors de mika#1971, en laissant sa promesse écrite. Le test survivant construit à la main le `ToolOutput` que le handler « émet » — un handler qui ne l'émet plus : il atteste une propriété d'un code mort.

**Et un test asserte aujourd'hui la fuite.** `test_map_substrate_error_taxonomy` vérifie que le message contient `"MIKA_BRAVE_API_KEY on mika-gateway"` et `"rotate MIKA_BRAVE_API_KEY"`. Ce contrat sera **retourné** par U2 : ces chaînes appartiennent désormais au diagnostic opérateur, jamais au texte LLM. C'est un changement de contrat assumé, pas un test cassé en passant.

#### M4 — Ce que la preuve du ticket établit, et ce qu'elle n'établit pas

Les en-têtes capturés sont `x-ratelimit-limit: 1, 2000`, `x-ratelimit-remaining: 0, 1879`, `x-ratelimit-policy: 1;w=1, 2000;w=2592000`. Ils établissent : clé valide, quota mensuel sain, débit d'une requête par seconde. Ils **n'établissent pas** l'épuisement en rafale : sur une fenêtre `w=1`, `remaining: 0` est l'état **attendu** immédiatement après la requête qui vient de réussir — le `wget` de la mesure a consommé lui-même l'unique jeton de sa seconde. La contention à 6 tenants sur une limite 1/s reste plausible et probablement réelle ; elle n'est simplement pas prouvée par cette capture.

#### M5 — Une hypothèse concurrente que le plan ne peut pas trancher, et la variable non documentée qui la rend crédible

Le substrat de recherche n'est construit que si **`MIKA_SEARCH_UPSTREAM`** vaut `brave` (`main.rs:190`) ; `MIKA_BRAVE_API_KEY` seule ne l'active pas. Cette variable n'est documentée **nulle part** dans les surfaces qu'un déployeur lit : absente de `.env.example`, absente de la section env du `CLAUDE.md` racine — laquelle affirme encore, à tort depuis mika#1971, que `MIKA_BRAVE_API_KEY` sert au « `web_search` builtin skill ». Sa seule mention est `docs/egress-search-searxng-contingency.md`. Un déploiement qui pose la clé (sur le pod, ou même sur la gateway) sans poser `MIKA_SEARCH_UPSTREAM=brave` laisse `search_egress_client = None` → 404 `search_upstream_not_configured` → le texte de la première ligne du tableau M3.

Départager exige les surfaces du tenant. **Le plan livre l'instrument et la sonde ; il ne tranche pas.** U6 documente la variable — le défaut de documentation est réel indépendamment de l'issue du départage.

#### M6 — Population sœur, nommée et hors périmètre

`fetch_url` (même fichier, même substrat) porte la même asymétrie : `substrate_unavailable` sur ses deux branches de configuration, `ToolOutput::error` sur ses réponses HTTP. Les AC nomment `web_search` ; l'élargir ici doublerait le rayon d'un p1. **Ticket de suivi**, à ouvrir avec ce plan en référence.

### Requirements

- **R1.** Trois classes d'échec au moins sont nommées distinctement de bout en bout — rate-limit, authentification, substrat non configuré — sans fourre-tout. (AC1)
- **R2.** Sur 429, le substrat retente après une attente **strictement supérieure à la fenêtre de débit annoncée**, en honorant `Retry-After` quand il est présent, avant de rendre un échec. (AC2)
- **R3.** Un 429 suivi d'un succès réussit sans échec visible ; un 429 persistant escalade proprement sous le motif `rate_limited`, jamais sous un motif de clé. (AC3)
- **R4.** `audit_events` permet à l'opérateur de distinguer `rate_limited` de `missing_key`. (AC4)
- **R5.** Aucune branche d'échec de `web_search` ne sert de jeton opérateur à un tenant famille ou champion — 429 comprise, et les autres aussi. (AC5, élargi par M3)
- **R6.** La variable qui active le substrat de recherche est documentée sur les surfaces qu'un déployeur lit, et la ligne devenue fausse depuis mika#1971 est corrigée. (M5)

### Scope Boundaries

**Dans le périmètre :** la taxonomie et le backoff de `egress_search` ; les sorties d'échec de `web_search` ; le routage par tier et l'audit de ces sorties ; les tests des AC ; la documentation de `MIKA_SEARCH_UPSTREAM`.

**Hors périmètre, délibérément :**
- Le **tier payant Brave** et les **clés par tenant** — le ticket les exclut lui-même ; ce sont les seuls remèdes à une contention 1 req/s partagée entre six tenants, et ce plan ne prétend pas l'absorber par du retry (KTD3).
- `fetch_url` (M6) — ticket de suivi.
- Le nombre de tentatives au-delà d'une (KTD3).
- Toute lecture ou modification de la configuration d'un tenant en production : le départage de M5 est une **mesure d'exploitation**, pas une modification de code.

---

## Planning Contract

### Key Technical Decisions

**KTD1 — Le rate-limit devient une variante de la taxonomie, et le statut HTTP ne bouge pas.** `SearchError::RateLimited` rejoint l'énumération ; `tracing_status()` rend `"rate_limited"` ; `http_status()` reste **502**. Le contrat entre l'agent et le substrat est le couple `(status, label)` et il est déjà en place : seule manquait une étiquette distincte. Rendre 429 ajouterait un second axe (et inviterait un futur proxy à retenter de lui-même) sans en fermer aucun. `UpstreamStatus(429)` devient inatteignable : le `match` de `send_once` route le 429 vers la nouvelle variante, et l'ancienne garde son sens — « un statut amont non classé ».

**KTD2 — Le plancher de backoff se déduit de la fenêtre annoncée, pas d'une rondeur.** `RATE_LIMIT_MIN_BACKOFF_MS = 1_100` : la fenêtre est `w=1`, donc tout retard sous 1 000 ms retombe dedans, et 100 ms de marge couvrent la dérive d'horloge et la latence de mise en file. L'attente devient `clamp(Retry-After.unwrap_or(plancher), plancher, MAX_HONORED_RETRY_AFTER_MS)` — le plafond de 2 s est **conservé** et reste supérieur au plancher, donc l'intervalle ne peut pas s'inverser. Budget : `EGRESS_HARD_TIMEOUT_SECS = 5` et un 429 revient vite (pas de travail amont) ; `0,3 + 1,1 + 3,0 = 4,4 s < 5 s`. Le cas dégradé (tentative 1 allant jusqu'au timeout de 3 s) était déjà borné par `wait.min(remaining)` et ne change pas.

**KTD3 — Une seule tentative, et le refus d'en ajouter est motivé par le quota.** Le module-doc de `brave.rs` pose la règle : « the Brave freemium tier is ~2000 requests / month; a naive retry doubles quota impact ». Passer à trois tentatives triple le pire cas sur une enveloppe partagée par six tenants, pour absorber une contention que le retry ne peut structurellement pas résoudre — six tenants sur 1 req/s se sérialisent ou échouent, quel que soit le nombre de reprises. **Ce plan rend l'échec honnête et attribuable ; il ne prétend pas le faire disparaître.** Le remède au débit est infra et le ticket le place hors périmètre.

**KTD4 — `x-ratelimit-reset` n'est pas lu, et le refus est nommé.** Le ticket le propose en alternative à `Retry-After`. Sa valeur est une **paire** (`1, 2592000`) dont le premier terme, sur une fenêtre `w=1`, vaut toujours ≈ 1 — exactement le plancher que KTD2 pose. Ajouter un second parseur d'en-tête propriétaire pour retrouver une constante déjà posée est du coût sans information. `Retry-After` reste lu (standard, déjà implémenté, déjà testé, refusant délibérément le format HTTP-date).

**KTD5 — `map_substrate_error` rend un couple, et le compilateur force chaque site à décider.** Sa signature devient `-> SubstrateFailure { fallback: String, diagnostic: String, reason: SubstrateReason }`. Chaque branche d'échec de `web_search` construit alors `ToolOutput::substrate_unavailable(fallback, diagnostic)` puis appelle le routage par tier. Le repli neutre est en français, aligné sur celui déjà servi par les deux branches de configuration (« La recherche web n'est pas disponible pour le moment. ») — un tenant famille ne doit pas voir le registre changer selon le motif. **Deux replis et non un seul :** le motif `rate_limited` mérite « Je n'arrive pas à faire la recherche pour l'instant, réessaie dans un moment. » (l'échec est transitoire et l'utilisateur peut agir), les autres gardent le repli d'indisponibilité. Aucun des deux ne porte de jeton interdit.

**KTD6 — Le motif voyage dans `reasoning`, jamais dans `target_key`.** `target_key = "web_search"` est le contrat « quel outil », pinné par les tests existants (`assert_eq!(evt.target_key, "web_search")`). `dispatch_substrate_diagnostic` gagne un paramètre `reason: Option<SubstrateReason>` et écrit `reason=<motif>; <tier> tier: substrate diagnostic gated from LLM`. **Signature étendue plutôt que fonction sœur** : une sœur laisserait les sites existants sur l'ancienne sans qu'aucune erreur ne l'annonce ; étendre force chaque appelant — y compris ceux de `fetch_url`, qui passeront `None` — à décider explicitement. Requête opérateur : `SELECT after_value, reasoning FROM audit_events WHERE tool_name = 'substrate_unavailable' AND target_key = 'web_search';`.

**KTD7 — Le motif est aussi journalisé, parce que l'audit ne couvre pas le tier opérateur.** `dispatch_substrate_diagnostic` **n'écrit aucun événement d'audit sur `Default`** — le code le dit et la raison est bonne (l'opérateur est le lecteur du `content`). L'AC4 n'est donc satisfaite par l'audit que pour family et champion. Un `warn!` structuré `web_search_substrate_unavailable` portant `reason` est émis **indépendamment du tier**, au site d'échec. Il ne porte ni la requête ni aucune valeur de secret : seulement le motif, qui est un mot fermé. **Régime attendu : non vide mais faible.** Une distribution dominée par `missing_key` tranche M5 ; dominée par `rate_limited`, elle confirme le ticket et renvoie au remède infra.

**KTD8 — Le garde disparu revient sous une forme qui tient.** Un scan de source (`skills::builtin_handlers::tests::mika2407_web_search_failure_paths_route_by_tier`) refuse tout `ToolOutput::error` dans le corps de `web_search` **en aval de la validation d'entrée** — les refus d'entrée (`query` manquante, trop longue) restent des erreurs nues, ils ne portent aucun jeton opérateur et ne concernent pas le substrat. Un test comportemental ne peut pas attraper cette classe : la régression ne rendrait aucune décision fausse, elle remettrait un texte opérateur dans le canal famille pendant que toutes les assertions existantes restent vertes. Le commentaire menteur de la ligne 4180 est corrigé dans le même geste, et le test qu'il décrit (`web_search_family_tier_http_401_no_leak`, qui construit son `ToolOutput` à la main) est remplacé par un test sur le chemin de production, désormais possible (KTD9).

**KTD9 — Les tests de bout en bout sont maintenant réalisables, et c'est un gain de mika#1971.** L'ancien handler codait en dur `https://api.search.brave.com/...`, ce qui interdisait le mock (le commentaire de la ligne 4173 le déplore). Le nouveau lit `ctx.gateway_url`, **injectable**. Un `wiremock` répondant `502 {"error":"rate_limited"}` ou `404 {"error":"search_upstream_not_configured"}` exerce le chemin de production complet, tier par tier. L'AC3 devient vérifiable des deux côtés : substrat (429→200 et 429 persistant) et agent (motif, repli, audit, absence de fuite).

### High-Level Technical Design

```
Brave  --429-->  egress_search::brave::send_once
                   |  429 -> Outcome::Retryable { RateLimited,
                   |            wait = clamp(Retry-After ?? 1100, 1100, 2000) }   [U1 / KTD2]
                   |  (une seule reprise — KTD3)
                   v
                 SearchError::RateLimited
                   |  http_status() = 502   tracing_status() = "rate_limited"     [U1 / KTD1]
                   v
  gateway  POST /internal/search  ->  502 {"error": "rate_limited"}
                   v
  agent  web_search
                   |  map_substrate_error(502, "rate_limited")
                   |     -> SubstrateFailure { fallback (FR, neutre),
                   |                           diagnostic (opérateur),
                   |                           reason: RateLimited }              [U2 / KTD5]
                   v
      ToolOutput::substrate_unavailable(fallback, diagnostic)
                   |
                   +--> warn! web_search_substrate_unavailable { reason }   (tous tiers)  [U3 / KTD7]
                   |
                   v
      dispatch_substrate_diagnostic(out, "web_search", Some(reason), ctx)     [U3 / KTD6]
          Family / Champion : LLM ne voit que `fallback`
                              audit_events(tool_name = substrate_unavailable,
                                           target_key = web_search,
                                           after_value = diagnostic,
                                           reasoning  = "reason=rate_limited; …")
          Default           : diagnostic replié dans `content`
```

### Assumptions

- **A1.** La politique de débit annoncée par Brave (`1;w=1`) est stable sur le tier gratuit. Si elle se resserre, le plancher de KTD2 est une constante nommée, à relever avec la mesure qui le justifie.
- **A2.** Un 429 revient en moins d'une seconde (pas de travail amont), ce qui laisse l'attente de 1,1 s tenir dans le budget de 5 s. Le cas contraire était déjà borné par `wait.min(remaining)` et dégrade vers « rendre le 429 maintenant » — comportement inchangé.
- **A3.** `FORBIDDEN_FAMILY_TIER_TOKENS` est la liste faisant foi pour « texte opérateur ». Elle est réutilisée telle quelle ; l'étendre serait une décision de doctrine hors de ce ticket.

---

## Implementation Units

### U1. Le substrat nomme le rate-limit et attend au-delà de sa fenêtre

`crates/mika-gateway/src/egress_search/` — `mod.rs` : variante `SearchError::RateLimited`, `http_status()` → 502, `tracing_status()` → `"rate_limited"` ; la table de la doc de module est mise à jour. `brave.rs` : constante `RATE_LIMIT_MIN_BACKOFF_MS = 1_100` documentée par la fenêtre `w=1` ; la branche 429 de `send_once` rend `Outcome::Retryable { error: SearchError::RateLimited, wait_before_retry: clamp(...) }`. Le module-doc du retry est corrigé sur les deux points (nouveau plancher, nouvelle variante). **Aucun `tracing` ajouté dans `brave.rs`** — la discipline Q4 du fichier l'interdit et le test de comptage d'événements l'attraperait.

### U2. `map_substrate_error` rend un couple, et chaque branche d'échec passe par le routage par tier

`crates/mika-agent/src/skills/builtin_handlers.rs` — `enum SubstrateReason { RateLimited, MissingKey, Unauthorized, Unconfigured, UpstreamError, Transport, Parse, Unknown }` et `struct SubstrateFailure`. `map_substrate_error` rend un `SubstrateFailure` ; la ligne `(502, "rate_limited")` est ajoutée, `(404, "search_upstream_not_configured")` porte `MissingKey`, `(502, "unauthorized")` porte `Unauthorized`. Les cinq sorties d'échec restantes de `web_search` (timeout, transport, corps trop grand, corps illisible, corps non parsable) reçoivent chacune leur `SubstrateFailure` et passent par `substrate_unavailable` + routage. **`match` exhaustif sur `SubstrateReason`, sans bras `_ =>`** : une future étiquette de taxonomie doit décider de son repli, pas hériter d'un défaut.

### U3. Le motif atteint l'audit et le journal

`crates/mika-agent/src/tools/mod.rs` — `dispatch_substrate_diagnostic` gagne `reason: Option<&str>` et compose `reasoning`. Les appelants de `fetch_url` passent `None` (leur motif n'est pas dans le périmètre ; la signature les force à le dire). `web_search` émet en plus un `warn!(event = "web_search_substrate_unavailable", reason = …)` avant le routage, tous tiers, sans requête ni secret.

### U4. Le garde structurel

`mika2407_web_search_failure_paths_route_by_tier` : scan du corps de `web_search`, refusant `ToolOutput::error` après la validation d'entrée. Le commentaire de la ligne 4180 est corrigé et `web_search_family_tier_http_401_no_leak` — qui atteste un code mort — est retiré au profit des tests U5.

### U5. Les tests négatifs des AC

**Substrat (wiremock, déjà en place) :** `rate_limit_429_persists_after_retry_returns_upstream_status` devient `…_returns_rate_limited` et asserte `SearchError::RateLimited` ; `rate_limit_429_retries_once_then_succeeds` est conservé ; un test nouveau asserte que l'attente sans `Retry-After` dépasse la fenêtre (assertion sur `clamp`, pure, sans horloge).

**Agent (wiremock sur `ctx.gateway_url`, nouveau) :** pour `502/rate_limited` et pour `404/search_upstream_not_configured`, sur tier `Family` et sur tier `Default` — (a) aucun jeton de `FORBIDDEN_FAMILY_TIER_TOKENS` dans le `content` servi en famille, (b) `substrate_diagnostic` consommé, (c) une ligne `audit_events` portant `reason=rate_limited` / `reason=missing_key`, (d) sur `Default`, le diagnostic présent dans le `content` et **aucune** ligne d'audit. **Contrôle négatif porteur :** les deux motifs doivent produire deux `reasoning` **différents** — sans quoi le test resterait vert sur un motif codé en dur.

`test_map_substrate_error_taxonomy` est réécrit : les jetons opérateur sont asserts sur le **diagnostic**, et leur **absence** est asserte sur le **fallback**. Le retournement est explicité en commentaire avec la référence mika#2407.

### U6. La documentation du substrat de recherche

`.env.example` : `MIKA_SEARCH_UPSTREAM=brave` avec la mention qu'il conditionne l'activation et que `MIKA_BRAVE_API_KEY` va **sur la gateway**. `CLAUDE.md` racine : la ligne `MIKA_BRAVE_API_KEY` est corrigée (elle décrit un chemin retiré par mika#1971) et la variable d'activation est ajoutée. `crates/mika-gateway/CLAUDE.md` : les deux variables rejoignent la liste des variables de la gateway, avec la conséquence d'un oubli (404 `search_upstream_not_configured`).

---

## Verification Contract

- **V1.** `cargo test -p mika-gateway egress_search` — vert, y compris les deux tests 429 et le test de plancher.
- **V2.** `cargo test -p mika-agent web_search` — vert, y compris les quatre nouveaux tests wiremock tier × motif et le garde structurel.
- **V3.** `cargo clippy --workspace --all-targets` sans avertissement nouveau ; `cargo fmt --check`.
- **V4.** Contrôle de non-régression sur la doctrine : `cargo test -p mika-agent family_tier` reste vert (les tests mika#1783 existants ne sont pas affaiblis).
- **V5 — sonde post-déploiement, avec ses haltes.** Sur le tenant d'Al, après un échec de veille :
  ```sql
  SELECT reasoning, count(*) FROM audit_events
   WHERE tool_name = 'substrate_unavailable' AND target_key = 'web_search'
   GROUP BY 1 ORDER BY 2 DESC;
  ```
  et `grep web_search_substrate_unavailable "$MIKA_SPIRIT_LOG_FILE" | jq .reason`.
  - `reason=missing_key` dominant → **M5 est confirmé** : le substrat n'est pas activé sur la gateway du tenant. Le remède est `MIKA_SEARCH_UPSTREAM=brave` (U6), pas un réglage de backoff. **Halte : ne pas toucher aux constantes de retry.**
  - `reason=rate_limited` dominant → le ticket est confirmé. Le backoff de KTD2 absorbe la limite 1/s d'un tenant isolé ; une persistance signifie une contention multi-tenant, et le remède est celui que le ticket place hors périmètre (tier payant, clés par tenant). **Halte : ne pas augmenter le nombre de tentatives** — voir KTD3, le quota mensuel est l'enveloppe qui casserait.
  - Les deux surfaces **vides** alors que la veille échoue toujours → l'échec ne passe pas par `web_search` : **halte**, établir quel outil échoue avant toute modification ici.
  - `reason=unauthorized` → la clé de la gateway est révoquée ou différente de celle mesurée depuis le pod. Rotation, pas correctif de code.

---

## Definition of Done

- Le substrat nomme `rate_limited` et attend au-delà de la fenêtre `w=1` avant sa reprise unique.
- `web_search` ne rend plus aucun `ToolOutput::error` sur une réponse du substrat ; chaque échec passe par `substrate_unavailable` et le routage par tier.
- Le motif est lisible dans `audit_events.reasoning` (family / champion) et dans le journal (tous tiers).
- Un garde structurel refuse la réintroduction d'un `ToolOutput::error` sur les chemins de réponse substrat, et le commentaire renvoyant à un test disparu est corrigé.
- Les tests des AC couvrent les deux côtés (substrat et agent) et les deux tiers, avec un contrôle négatif séparant « le motif est lu » de « le motif est constant ».
- `MIKA_SEARCH_UPSTREAM` est documenté sur `.env.example`, `CLAUDE.md` racine et `crates/mika-gateway/CLAUDE.md` ; la ligne `MIKA_BRAVE_API_KEY` devenue fausse depuis mika#1971 est corrigée.
- V1–V4 verts ; V5 posée dans le corps de la PR avec ses quatre haltes.
- Le ticket de suivi `fetch_url` (M6) est ouvert et référencé dans le corps de la PR.

---

## Acceptance criteria

Transcrites verbatim du corps de mika#2407. Le site d'application de l'AC2 est le substrat (`egress_search`) et non `builtin_handlers.rs` — voir M1 ; l'AC5 est élargie par M3 à toutes les branches d'échec, pas seulement la branche 429.

1. `web_search` distingue 429 (rate-limit) de 401 (auth) et de la clé absente — trois branches nommées, pas un fourre-tout.
2. Sur 429, le handler applique un **backoff + retry** (respecter `Retry-After`/`x-ratelimit-reset` si présent ; au moins 1 retry après ~1s pour absorber la limite 1/s), avant de tomber en `substrate_unavailable`.
3. **Test négatif** : un endpoint Brave mocké qui rend 429 puis 200 → le handler retente et réussit (pas de `substrate_unavailable`). Un mock qui rend 429 en boucle → le handler escalade proprement après N tentatives, avec un diagnostic substrate `rate_limited` (pas « missing key »).
4. Le diagnostic substrate distingue `rate_limited` de `missing_key` dans `audit_events` (l'opérateur doit pouvoir différencier quota/débit d'une clé absente).
5. Non-régression : la doctrine mika#1783 tient (aucun token opérateur ne fuit au family-tier sur la branche 429).

*Note sur l'AC2 :* `x-ratelimit-reset` n'est délibérément pas lu — KTD4 en donne la raison mesurée (sa valeur sur la fenêtre `w=1` est la constante que le plancher pose déjà). `Retry-After` reste honoré. Le « ~1s » de l'AC est réalisé par `RATE_LIMIT_MIN_BACKOFF_MS = 1_100`.

*Note sur l'AC4 :* l'audit ne couvre que les tiers family et champion — `dispatch_substrate_diagnostic` n'écrit rien sur `Default`, par une décision mika#1783 que ce plan ne renverse pas. Le journal (KTD7) porte le motif sur tous les tiers, ce qui est ce dont l'opérateur a besoin sur son propre poste.

---

## Sources

- `crates/mika-agent/src/skills/builtin_handlers.rs` — `web_search` (192–321), `map_substrate_error` (349–380), `FORBIDDEN_FAMILY_TIER_TOKENS` (4022), `web_search_family_tier_http_401_no_leak` (4170–4224), `test_map_substrate_error_taxonomy` (4418–4440).
- `crates/mika-agent/src/tools/mod.rs` — `substrate_unavailable` (394–405), `dispatch_substrate_diagnostic` (408–487).
- `crates/mika-gateway/src/egress_search/brave.rs` — politique de retry (22–41), `RETRY_BACKOFF_MS` (54), `MAX_HONORED_RETRY_AFTER_MS` (60), branche 429 (164–173), tests wiremock (490–547).
- `crates/mika-gateway/src/egress_search/mod.rs` — `SearchError` (152–212), `EGRESS_HARD_TIMEOUT_SECS` (67), `build_client` (223–233), `handle_internal_search` (318–351).
- `crates/mika-gateway/src/main.rs` (185–207), `settings.rs` (135–144, 247–262) — activation du substrat.
- `CLAUDE.md` racine § *Environment Variables*, `.env.example:39`, `docs/egress-search-searxng-contingency.md`.
- mika#1783 (doctrine substrat/tier), mika#1808 (client Brave), mika#1971 (bascule du `web_search` vers le substrat), mika#2023 (arm `Champion` explicite).
