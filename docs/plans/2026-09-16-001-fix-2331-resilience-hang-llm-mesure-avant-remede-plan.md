# mika#2331 — Résilience au hang LLM « 420 s sans octet » : rendre le tour mesurable là où il échoue, puis combler les trous de retry réels

- **Ticket** : senara-solutions/mika#2331
- **Type** : fix
- **Date** : 2026-09-16
- **Relie** : mika#2326 (transport OpenRouter), mika#2295 / #2327 (borne du brief arch), mika#2293 (`llm_budget_resolved`), mika#2189 (budgets LLM), mika#2015 (body-read → transport), mika#2179 (classes d'erreur de livraison), mika#1889 (`turn_usage`)

---

## 1. Ce que le ticket demande, et ce que le code en dit

Le ticket a trois strates, et **la plus récente prime** :

| Strate | Demande |
|---|---|
| Corps | `_arch_ask` (shell) doit retenter 1× sur échec transport |
| Commentaire 1 (17:04Z) | Non : le retry doit vivre **au client LLM moteur**, pour couvrir dev/qa/arch et tous transports |
| Commentaire 2 (17:31Z) | Raffinement : la classe est **callback**, pas transport. **« Mesure à faire (avant d'implémenter #2331) »**. Si la cause est la taille du prompt, *« le retry 1× ne suffira PAS »* — « un retry d'un prompt trop gros re-hangue » |

La lecture du code **déplace le diagnostic une troisième fois**. Sept faits, tous vérifiables :

### F1 — Le retry demandé existe déjà, sur exactement le rail qui porte tous les hangs mesurés

`crates/mika-common/src/llm/error.rs:32-38` :

```rust
pub fn is_retryable(&self) -> bool {
    match self {
        LlmError::HttpError { retryable, .. } => *retryable,
        LlmError::Transport(_) => true,
        _ => false,
    }
}
```

`LlmError::Transport` est **inconditionnellement retryable**, et depuis mika#2015 un échec de lecture du corps en cours de route y est classé — `crates/mika-common/src/llm/openai.rs:281-300` produit `LlmError::Transport("failed to read response body: …")`. Le rail OpenAI-compat couvre **10 providers sur 13**, dont OpenRouter, Z.AI et Kimi : c'est-à-dire **100 % des hangs listés par le ticket** (glm-5.3/OpenRouter, glm-5.3/Z.AI, kimi/OpenRouter).

> **Conséquence** : « retenter 1× sur un échec transport au niveau du client LLM » est, sur le chemin de l'incident, **déjà le comportement livré**. Le correctif principal demandé par le corps et par le commentaire 1 ne peut donc pas être la cause de la perte du groom #2293.

### F2 — Le nombre de tentatives est déjà borné, et vaut exactement 1 retry pour dev/qa

`crates/mika-common/src/llm/budget.rs:270-275` :

```rust
pub fn max_attempts(&self, hard_cap: u32) -> u32 {
    let derived = self.agent_total_timeout_secs / self.http_timeout_secs.max(1);
    u32::try_from(derived).unwrap_or(u32::MAX).clamp(1, hard_cap.max(1))
}
```

Consommé par `openai.rs:367-371` dès qu'une deadline est présente — ce qui est **toujours** le cas depuis `agent_loop` (`crates/mika-agent/src/agent_loop/mod.rs:1105` et `:543`, tous deux `Some(deadline.into())`, y compris pour le chemin silent, qui passe par la même `run_loop`).

- mika-dev / mika-qa : `300 / 120 = 2` → tentative initiale **+ exactement 1 retry**.
- mika-arch : `900 / 240 = 3`.

Le `MAX_RETRIES = 3` d'`openai.rs:147` est donc **inatteignable en production sur le chemin agent**.

### F3 — Le tour ne dit ni sa taille, ni son nombre de tentatives

`emit_turn_usage` (`agent_loop/mod.rs:6617-6646`) porte : `step`, `stop_reason`, les quatre compteurs de tokens, `latency_ms`, `tool_use_in_turn`, `status`. **Aucun champ d'octets, aucun compteur de tentatives.** Sur le bras `Err` (`mod.rs:1223`), `usage = None` ⇒ tous les tokens à 0 : c'est exactement la ligne `input_tokens=0 / latency≈420 000 / status=error` que l'opérateur a mesurée.

Un hang de 420 s est donc aujourd'hui **indiscernable** entre : une tentative anormalement longue, deux tentatives de 120 s, quatre tentatives, ou un échec en amont de l'appel.

### F4 — 420 s n'est un multiple d'aucun budget documenté

| Géométrie | `max_attempts` | Durée maximale de la chaîne (avec backoff 500·2ⁿ ms) |
|---|---|---|
| dev/qa : 120 / 300 | 2 | ≈ 240,5 s |
| arch : 240 / 900 | 3 | ≈ 721,5 s |
| arch si seule l'enveloppe a pris : 120 / 900 | 4 (plafonné) | ≈ 483,5 s |
| défaut flotte : 120 / 300 | 2 | ≈ 240,5 s |

**420 s ne tombe sur aucune de ces valeurs.** Le nombre n'est expliqué par aucune guillotine connue du code. Ce fait à lui seul invalide symétriquement les deux remèdes proposés : on ne peut pas décider qu'« un retry suffit » ni qu'« il faut borner le prompt » tant qu'on ne sait pas si la chaîne a tourné une ou quatre fois.

> C'est très exactement la situation que mika#2293 a déjà rencontrée sur un cas jumeau — un couple 240/900 livré le 06/09, des coupures toujours à 120 s pile le 11/09, et **aucune ligne de journal capable de dire pourquoi**. Le remède accepté alors fut l'instrument (`llm_budget_resolved`), pas un plafond remonté à l'aveugle.

### F5 — L'hypothèse « le prompt callback embarque le log pilote complet » est réfutée par trois barrages indépendants

1. `skills/bundled/_shared/dispatch-lib.sh:2667-2747` ne met jamais le log dans `RESULT` — seulement son **chemin**, plus une queue de stderr de `tail -c 10000` ; puis `head -c 92000` (`:2749`, `:1475`).
2. Ingestion : rejet dur au-delà de `100_000` (`crates/mika-cli/src/commands/ask.rs:192`, `crates/mika-agent/src/server/handlers.rs:526`).
3. **Avant injection dans le prompt** : `format_callback_framing` (`agent_loop/mod.rs:265-277`) tronque à `CALLBACK_RESULT_MAX_BYTES = 10_240` (`crates/mika-agent/src/planning/policy.rs:48`).

Le log pilote n'entre que comme **tool result au pas ≥ 2** (`skills/bundled/self-dev-callback/system_prompt.md:132-135` prescrit `tail -200`), capé à 10 Ko par `crates/mika-agent/src/skills/executor.rs:25`.

### F6 — En revanche, le différentiel callback / non-callback a bien une explication structurelle, et elle est ailleurs

Le tour callback est **le seul** tour silencieux qui sélectionne ses skills par `callback_safe_skills()` (`crates/mika-agent/src/skills/mod.rs:963`) — always_on **plus les dépendances transitives en BFS**, handlers exec/http compris. Tous les autres triggers silencieux passent par `safe_always_on_skills()` (`skills/mod.rs:924`), strictement plus restrictif.

Ce bloc est injecté par `inject_skills_and_resolve_tools` **sans aucun plafond**. Masse disponible : `self-dev/system_prompt.md` = 62 790 o, `qa-review` = 67 979 o, `self-dev-webhook-qa` = 33 430 o, `self-dev-callback` = 14 482 o ; 301 519 o de prompts bundled au total. Le `result` capé à 10 Ko est, devant ça, un contributeur mineur.

C'est le seul mécanisme du code qui sépare la population « callback » de la population « non-callback » exactement comme la mesure de l'opérateur les sépare (8 % contre 0 %). **C'est une corrélation structurelle, pas encore une cause démontrée.**

### F7 — La donnée qui trancherait existe déjà — dans la mauvaise surface

`agent_loop/mod.rs:1101` :

```rust
// mika#2189 D5: measured BEFORE the call, because the failure this
// closes is on the error path — a call that times out returns no usage
// and, until v53, left `input_tokens = 0` as its only record of how big
// the brief was. Taken here, the number survives the timeout.
let request_bytes = Some(request.payload_bytes() as i64);
```

`request_bytes` (v53) et `system_prompt_bytes` (v38) sont calculés **avant** l'appel et écrits **y compris sur le bras erreur** (`mod.rs:1169-1173`) — mais uniquement si `store_llm_calls`, et **jamais dans le flux de journal**. D'où la mesure opérateur qui ne voit qu'`input_tokens=0`.

Aux trois sites d'émission de `turn_usage`, les deux valeurs sont **déjà en portée lexicale** :

- `mod.rs:1204` (bras `Ok`) et `mod.rs:1223` (bras `Err`) : `request_bytes` (l. 1101), `system_prompt_len` ;
- `mod.rs:704` (`save_continuation_llm_call`) : `system_prompt_bytes` et `request_bytes` sont déjà des **paramètres de la fonction** (l. 685).

> **Ancrage des citations** : tous les numéros de ligne de ce plan sont relevés au SHA `45cec264` de la branche, et **re-vérifiés contre l'arbre à ce SHA** — `is_retryable` (`llm/error.rs`), `max_attempts` (`llm/budget.rs`), les trois paires `build_turn_usage_fields`/`emit_turn_usage`, `request_bytes` (l. 1101), les deux `response.json()` d'`ollama.rs`/`claude.rs`, le `retry_threshold_secs` sans branche transport d'`ollama.rs`, et le `.timeout(Duration::from_secs(120))` en dur de `claude.rs`. Les noms de symboles restent l'ancre porteuse — si un numéro a dérivé sous un rebase, c'est le symbole qui fait foi.

> L'instrument central de ce plan ne demande donc **aucune nouvelle mesure** : il déplace une donnée déjà calculée d'une surface gatée vers la surface ungated qui existe précisément pour ça.

---

## 2. Décisions de conception

### D1 — La mesure est le premier livrable, parce que le ticket l'ordonne et parce que F4 la rend indispensable

Le commentaire 2 écrit *« Mesure à faire (avant d'implémenter #2331) »*. Ce plan exécute cette instruction plutôt que de l'enjamber. F4 la rend structurelle : **trois hypothèses concurrentes (guillotine client / prompt hors borne / échec en amont) produisent aujourd'hui exactement la même ligne de journal.** Poser un remède avant de les séparer, c'est reproduire mika#2293 — un réglage livré le 06/09 qui échoue en silence jusqu'au 11/09.

Le plan livre donc, dans le même travail : **l'instrument** (§3.1, §3.2), **le remède dont la justification ne dépend d'aucune mesure** (§3.3), et **la procédure qui décide du remède restant** (§3.5).

### D2 — Ne pas réécrire le retry qui existe ; le combler là où il manque réellement

Le commentaire 1 demande la couverture « tous transports ». Elle est acquise sur le rail OpenAI (F1) et **manque sur les deux autres**, où un échec de lecture du corps est classé `ParseError`, donc **non retryable** :

- `crates/mika-common/src/claude.rs:785` : `response.json().await.map_err(ClaudeApiError::ParseError)?`
- `crates/mika-common/src/llm/ollama.rs:502-505` : `.json().await.map_err(|e| LlmError::ParseError(…))?`

C'est **la régression exacte que mika#2015 a fermée sur openai.rs et qui n'a jamais été portée**. Aucun hang mesuré n'est sur ces rails — mais la demande du ticket est explicitement transverse, le trou est de la même classe, et le correctif est mécanique.

### D3 — La classification d'erreur remonte dans `mika-common`, elle ne se duplique pas

`classify_delivery_error` (`crates/mika-agent/src/task_engine/dispatcher.rs:218-236`) produit déjà le vocabulaire `transport_timeout` / `transport` / `http_<status>` / `parse` / `provider` / `unsupported` / `other`, et ce vocabulaire est **un format de fil** : l'opérateur en fait des `GROUP BY` sur `audit_events.callback_delivery_failed` (mika#2179). En écrire une seconde orthographe dans le client LLM couperait la population en deux sans le dire — le défaut que les constantes `FILTER_*` de mika#2131 ont dû fermer une fois.

La classification devient donc `LlmError::error_class()` dans `crates/mika-common/src/llm/error.rs`, et `classify_delivery_error` se réduit à un adaptateur (`downcast_ref` puis délégation), **ses tests existants inchangés et toujours verts** (`dispatcher.rs:3781-3810`).

### D4 — Pas de `correlation_id` traversant le client LLM

Corréler un `llm_call_attempt` à son `turn_usage` demanderait soit d'élargir la signature du trait `LlmProvider` (3 implémentations + `MockLlmProvider`), soit d'ajouter un champ d'observabilité à `LlmRequest`. Pour la classe mesurée — des hangs **isolés** de 420 s, pas une rafale — la fenêtre temporelle plus `provider`/`model` suffit à recoller les lignes.

**Angle mort assumé, à écrire dans le CLAUDE.md** : sous forte concurrence sur un même couple provider/model, l'appariement tentative ↔ tour est ambigu. Le lever coûterait un champ dans le trait ; il sera payé le jour où la mesure le demandera, pas avant.

### D5 — L'instrument est ungated, comme `turn_usage`

Ni `MIKA_STORE_LLM_CALLS`, ni `MIKA_LOG_LLM_BODIES` ne doivent le taire. Doctrine déjà écrite dans le dépôt (mika#1889 R2/D2, reprise par mika#2295) : *« un instrument qui se tait quand on coupe la persistance n'est pas un instrument, c'est une option »*. C'est d'autant plus vrai ici que la panne à diagnostiquer est précisément celle où l'on coupe la télémétrie pour réduire le bruit.

### D6 — Les nouveaux champs de `turn_usage` sont `Option`, et l'absence se distingue du zéro

`request_bytes = 0` serait un mensonge lisible (aucune requête n'est vide) ; `null` dit « non mesuré ». Le schéma du flux ne change pas sous l'analyseur : les champs sont **ajoutés**, jamais renommés, et aucun champ existant ne bouge — contrainte reprise de mika#2295 brique 0, qui a shippé ses champs `truncated_*` à 0 pour la même raison.

### D7 — Ce qui n'est **pas** fait, et pourquoi

- **Retry dans `_arch_ask`** : écarté par le commentaire 1, et F1 montre que le retry client couvre déjà ce rail. Un retry shell poserait en outre une question non triviale de continuité de session (`_arch_ask` passe `--session-id`, `dispatch-lib.sh:4517`) : un second essai doit-il reprendre la session de l'essai mort, ou en ouvrir une neuve ? Sans la mesure, la réponse est indécidable ; et si la cause est un prompt hors limite, le second essai re-hangue et l'on aura payé deux fois 420 s pour rien.
- **Borner le prompt du tour callback** : c'est le remède que F6 rend plausible, et le seul que le commentaire 2 juge décisif. Il est **conditionné à la mesure** (§3.5) parce qu'élider des skills n'est *pas* symétrique d'élider de l'historique ancien : mika#2295 pouvait couper le plus vieux message sans rien casser, alors qu'un prompt de skill retiré retire une instruction dont le traitement du callback dépend. Choisir quoi couper, et jusqu'où, exige de savoir d'abord si la taille est bien la cause.
- **Factoriser les trois boucles de retry** : la duplication `openai.rs` / `ollama.rs` / `claude.rs` est réelle et assumée par les commentaires du code (`ollama.rs:581-582`, `claude.rs:538-542`). La réduire est un refactor dont le rayon d'action dépasse ce ticket. Nommée en dette, laissée dehors.

---

## 3. Implémentation

### 3.1 — `turn_usage` porte la taille du brief (AC1)

**Fichier** : `crates/mika-agent/src/agent_loop/mod.rs`

1. Ajouter à `TurnUsageFields` (l. 6544-6554) : `request_bytes: Option<i64>`, `system_prompt_bytes: Option<i64>`.
2. Étendre `build_turn_usage_fields` (l. 6576) de deux paramètres, passés tels quels (fonction pure, testable sans subscriber).
3. Émettre dans `emit_turn_usage` (l. 6617) : `request_bytes = ?fields.request_bytes`, `system_prompt_bytes = ?fields.system_prompt_bytes` (le sigil `?` préserve la distinction `null` / valeur, cf. D6).
4. Câbler les **trois** sites. Chacun est une paire `build_turn_usage_fields` (où les deux arguments s'ajoutent) suivie de `emit_turn_usage` (qui lit `fields`) — **ce sont les appels `build_*` qu'il faut modifier**, les `emit_*` ne changent pas d'appel :
   - bras `Ok` : build l. 1196, emit l. 1204 ;
   - bras `Err` : build l. 1215, emit l. 1223 ;
   — pour ces deux-là, `request_bytes` (l. 1101) et `Some(system_prompt_len as i64)` sont en portée lexicale ;
   - continuation (`save_continuation_llm_call`) : build l. 696, emit l. 704 — les deux valeurs sont déjà des paramètres de la fonction (l. 685).

   Les six sites sont énumérables d'un seul grep, qui est aussi la vérification que le câblage est complet :
   `grep -n "emit_turn_usage\|build_turn_usage_fields" crates/mika-agent/src/agent_loop/mod.rs`
   → trois paires hors bloc `#[cfg(test)]`. Aucune quatrième paire ne doit apparaître sans que ce plan soit relu.

5. **Coût mécanique nommé d'avance** : le même grep rend **dix** appels supplémentaires *dans* le bloc `#[cfg(test)]` (l. ≈12287–12370), qui construisent la structure avec six arguments. Élargir la signature les casse tous, et cette modification-là est **attendue et sans signification** — elle n'est qu'un ajout d'arguments. C'est l'exacte inverse de la contrainte portée par V4 sur `classify_delivery_error`, où le moindre test à modifier est un signal d'alarme ; les deux contraintes sont énoncées ensemble en §*Verification contract* pour qu'aucune ne soit lue comme l'autre.

Le commentaire de `TurnUsageFields` mentionne la condition dure Prime #1 (aucun champ `phase`/`is_planning`/`role`). **Les deux champs ajoutés sont des dimensions RAW**, pas une classification : ils la respectent. Le noter au site.

### 3.2 — Un événement structuré par tentative (AC2)

**Nouveau** : `crates/mika-common/src/llm/error.rs`

```rust
impl LlmError {
    /// Wire-format error class, shared with the callback-delivery ledger
    /// (mika#2179). Single writer of this vocabulary — see D3.
    pub fn error_class(&self) -> std::borrow::Cow<'static, str> { … }
}
```

Corps déplacé verbatim depuis `dispatcher.rs:226-235` (y compris le test de sous-chaîne `"timed out"`, seul moyen de séparer un timeout d'un refus de connexion : `From<reqwest::Error>` aplatit le `reqwest::Error` en `String` — `error.rs:53-57` — et perd `is_timeout()`).

**Adaptation** : `dispatcher.rs:218` devient
```rust
fn classify_delivery_error(err: &anyhow::Error) -> std::borrow::Cow<'static, str> {
    err.downcast_ref::<LlmError>()
        .map_or(std::borrow::Cow::Borrowed("other"), LlmError::error_class)
}
```
Ses quatre tests existants (`dispatcher.rs:3781-3810`) restent inchangés et deviennent le garde-fou de la factorisation.

**Nouveau** : `mika_common::llm::emit_llm_call_attempt(...)`, dans `llm/mod.rs`, émis **par tentative** sur les trois rails, en `info!(target: "mika::otel", event = "llm_call_attempt", …)` :

| Champ | Source |
|---|---|
| `provider`, `model` | `self.provider_kind`, `request.model` |
| `attempt`, `max_attempts` | boucle |
| `elapsed_ms` | `Instant` pris autour du seul `send_once` |
| `outcome` | `"success"` \| `"retrying"` \| `"exhausted"` \| `"deadline_abort"` |
| `error_class` | `LlmError::error_class()`, `null` si succès |
| `http_timeout_secs` | `self.budget.http_timeout_secs()` |
| `deadline_remaining_ms` | `null` si aucune deadline |

Sites : `openai.rs:423-449`, `ollama.rs:~570-590`, `claude.rs:~560-590` (le rail Anthropic n'ayant ni budget ni `max_attempts`, il reporte `MAX_RETRIES + 1` et `http_timeout_secs = 120` — **le littéral en dur de `claude.rs:381-384`, que ce plan ne corrige pas mais rend au moins visible**).

Les `warn!` actuels (`"transient API error"`, `"retrying OpenAI-compatible API call"`, `"aborting retry chain — …"`) sont **conservés** : ils sont greppables par message et un opérateur peut déjà s'y appuyer. Le nouvel événement ne les remplace pas, il ajoute le champ `event` structuré, la durée par tentative et la classe d'erreur — les trois choses qui manquent pour trancher F4.

> **Couverture, pas partialité.** Les trois rails sont instrumentés dans le même travail, y compris ceux qui ne portent aucun hang mesuré. Un événement posé sur un seul rail ferait lire l'absence de ligne comme « ce tour n'a pas retenté » alors qu'elle ne dirait que « ce rail n'est pas instrumenté » — le piège « l'absence n'est pas une preuve » que ce dépôt a déjà dû nommer plusieurs fois.

### 3.3 — Combler les deux trous de retry (AC3)

**`crates/mika-common/src/llm/ollama.rs:502-505`** et **`crates/mika-common/src/claude.rs:786-787`** : remplacer `response.json()` par le motif mika#2015 d'`openai.rs:281-300` — lire `.text()`, et **seulement ensuite** désérialiser :

- échec de **lecture** ⇒ `LlmError::Transport` (resp. `ClaudeApiError::Transport`) + `warn!` avec la chaîne de causes ⇒ retryable ;
- échec de **désérialisation** ⇒ `ParseError` ⇒ non retryable, inchangé.

C'est ce découpage, et non un élargissement de `is_retryable`, qui distingue les deux causes : les octets ne sont jamais arrivés, contre les octets sont arrivés et sont illisibles.

**`ollama.rs:540-541`** : porter le seuil transport-aware de mika#1744, aujourd'hui absent de ce rail (il utilise le seuil long `0,75 + 0,25` du plafond même après une erreur transport, là où `openai.rs:396` bascule sur `transport_retry_min_remaining_secs()`).

**Coût assumé, à écrire au site** : retenter une complétion facture un second appel. C'est le prix déjà accepté sur le rail OpenAI depuis mika#2015, et une complétion n'a pas d'effet de bord côté serveur au-delà de la facturation.

**Hors périmètre, nommé au site** : `claude.rs:381-384` pose `.timeout(Duration::from_secs(120))` en dur au lieu d'appeler `budget.http_timeout_secs()`, et la boucle Anthropic ignore `max_attempts`. Incohérence réelle, déjà nommée dans `llm/mod.rs:281-284` et dans le CLAUDE.md de la crate, et sans rapport avec la classe de ce ticket (aucun hang mesuré n'est sur le rail Anthropic).

### 3.4 — Le test négatif que le ticket exige (AC4)

**Il n'existe aujourd'hui aucun test de la boucle de retry** — sur aucun des trois rails. `MockLlmProvider` ne peut pas en produire : il n'override pas `send_message_with_deadline` et rend son `MockResponse::Error` sans boucle (`llm/mock.rs:37`, `:122`, `:213`).

Ajouter `wiremock` aux dev-dependencies de `crates/mika-common/Cargo.toml` (déjà au workspace, `Cargo.toml:151`, déjà dev-dep de `mika-agent` et `mika-gateway`) et écrire `crates/mika-common/tests/llm_retry.rs` :

1. **Le test du ticket** — 1ʳᵉ réponse coupée mid-body (corps annoncé plus long qu'envoyé) ⇒ `Transport` ; 2ᵉ réponse valide ⇒ **succès, exactement 2 requêtes reçues**.
2. **Pas de boucle infinie** — deux échecs transport d'affilée ⇒ `Err` propagée, **exactement `max_attempts` requêtes reçues**, jamais davantage.
3. **`max_attempts` est bien la borne** — à la géométrie 120/300, un échec permanent produit **2** requêtes, pas 4 ; épingle F2 contre une dérive vers `MAX_RETRIES`.
4. **La classe est la bonne** — l'erreur propagée rend `error_class() == "transport_timeout"` sur la chaîne d'incident verbatim (déjà figée à `dispatcher.rs:3778-3779`).
5. **Non-retryable reste non-retryable** — un HTTP 400 ⇒ **1** requête.
6. **Les rails Ollama et Anthropic** — un corps tronqué produit une erreur retryable et **2** requêtes (aujourd'hui : `ParseError` et **1** requête ; c'est le test qui échoue avant §3.3 et passe après).

Tests unitaires complémentaires, sans réseau :
- `error_class` couvre les sept classes et **coïncide avec `classify_delivery_error`** sur tout le domaine (garde de la factorisation D3) ;
- `build_turn_usage_fields` propage `None` et `Some(n)` sans les confondre (D6).

### 3.5 — Documentation et procédure de diagnostic (AC5)

Dans `CLAUDE.md`, sous § *Optional (runtime observability)*, étendre la Signal O (`turn_usage`) et ajouter les greps du nouvel événement. Le cœur est la **procédure à quatre branches** du prochain hang, dans cet ordre :

**Étape 0 — avant toute ligne de code, lire un instrument qui existe déjà.**
```
grep llm_budget_resolved $MIKA_SPIRIT_LOG_FILE | jq '{agent_id, http_timeout_secs, agent_total_timeout_secs, max_attempts, http_source, total_source}'
```
Livré par mika#2293. Si le plafond effectif de l'agent qui a hangué n'est pas celui qu'on croit, **F4 s'explique sans rien implémenter** et le remède est un geste de configuration, pas de code. `http_source` dit lequel des trois mondes répond (`agent_config` / `process_env` / `default`).

**Étape 1 — combien de tentatives, et de quelle durée.**
```
grep llm_call_attempt $MIKA_SPIRIT_LOG_FILE | jq 'select(.outcome != "success") | {provider, model, attempt, max_attempts, elapsed_ms, outcome, error_class}'
```

**Étape 2 — quelle taille de brief, sur le tour qui a échoué.**
```
grep turn_usage $MIKA_SPIRIT_LOG_FILE | jq 'select(.status == "error") | {session_id, mode, provider, model, latency_ms, request_bytes, system_prompt_bytes}'
```

**Étape 3 — la décision, et ses trois branches.** Sur un échantillon d'au moins **cinq hangs** postérieurs au déploiement :

| Observation | Lecture | Remède |
|---|---|---|
| `attempt` atteint `max_attempts`, chaque `elapsed_ms ≈ http_timeout_secs × 1000` | La guillotine client se déclenche, le retry tourne **et ne sauve rien** | **Le retry n'est pas le remède.** Aller à la branche suivante ou remonter au fournisseur |
| `request_bytes` des tours en échec nettement supérieur à celui des tours sains du même agent | La taille du brief corrèle : F6 est confirmé | **Ouvrir le ticket « borner le prompt du tour callback »** (§4), pattern mika#2295 appliqué à `callback_safe_skills()` |
| `attempt = 1` avec `elapsed_ms` très supérieur à `http_timeout_secs × 1000` | Le timeout `reqwest` **ne coupe pas** : la panne est sous le client HTTP | **Halte.** Ni le retry ni la borne ne s'appliquent ; instruire au niveau connexion (relie mika#2313, egress-proxy) |
| `request_bytes` indiscernable entre tours sains et tours en échec | F6 est infirmé malgré la corrélation structurelle | Ne pas borner ; rouvrir le diagnostic |

**Critère de halte explicite** : si les cinq hangs suivants ne portent **aucune** ligne `llm_call_attempt`, alors la panne est **en amont de l'appel HTTP** et toute la lignée « retry / borne » est hors sujet. C'est un résultat, pas un échec de l'instrument.

---

## 4. Suite conditionnée (hors de ce PR)

Ouvrir, **si et seulement si** la branche 2 de §3.5 se vérifie : *« Borner en octets le bloc de skills injecté dans le tour callback »*. Contenu pressenti, pour que le ticket naisse groomé :

- `callback_safe_skills()` (`skills/mod.rs:964`) est le seul sélecteur silencieux qui résout les dépendances transitives et garde exec/http ;
- la borne s'**ajoute** à la sélection, elle ne la remplace pas (brique 1+2 de mika#2295, `agent_loop/mod.rs:3510-3534`) ;
- `per_skill_bytes` de `emit_system_prompt_assembled` est **déjà** la brique 0 équivalente et n'a pas à être réécrit ;
- prise d'effet sur agent déjà provisionné : toute clé nouvelle d'`identity.toml` doit entrer dans `CODE_OWNED_IDENTITY_SECTIONS` (`well_known_agents.rs:477`), sans quoi `write_default_if_missing` ne réécrit rien et *« les sondes post-déploiement liraient comme un correctif qui n'a pas marché plutôt que comme un interrupteur qu'on n'a jamais actionné »* ;
- **le risque propre à ce cas** : couper un prompt de skill retire une instruction dont le traitement du callback dépend — asymétrie avec l'élision d'historique ancien, qui est ce qui interdit de le faire à l'aveugle.

Second ticket de dette, non conditionné : `claude.rs` n'honore ni `http_timeout_secs()` ni `max_attempts` (D2, §3.3).

---

## Verification contract

| # | Vérification | Commande |
|---|---|---|
| V1 | Compilation et lints | `cargo clippy --workspace --all-targets -- -D warnings` |
| V2 | Suite complète non régressée | `cargo test --workspace` |
| V3 | Boucle de retry, les six cas | `cargo test -p mika-common --test llm_retry` |
| V4 | Factorisation `error_class` sans changement de fil | `cargo test -p mika-agent classify_delivery_error` (quatre tests préexistants, **non modifiés**) |
| V5 | Champs `turn_usage` | `cargo test -p mika-agent build_turn_usage_fields` |
| V6 | Structure des bundles | `make verify-bundled-skills` |
| V7 | Formatage | `cargo fmt --check` |

**Contrainte sur V4, et son inverse sur V5 — les deux se lisent ensemble.**

- **V4 est une contrainte de non-modification** : si un test de `classify_delivery_error` doit être modifié, la factorisation a changé le format de fil et le travail est à reprendre. C'est la seule façon de constater la divergence que D3 existe pour empêcher.
- **V5 est exactement le contraire** : les dix appels de test à `build_turn_usage_fields` (§3.1 point 5) *doivent* être modifiés, puisque la signature gagne deux paramètres. Cette modification-là ne dit rien et n'atteste rien ; ce que V5 atteste est l'assertion ajoutée — `None` et `Some(n)` ne se confondent pas (D6).

Énoncer les deux côte à côte est nécessaire : sans ça, « aucun test ne doit bouger » se lit comme une règle du PR entier et interdirait le seul câblage que ce plan demande.

---

## Definition of Done

- [ ] `turn_usage` porte `request_bytes` et `system_prompt_bytes` aux trois sites d'émission, en `Option`, ungated, sans qu'aucun champ existant ne change de nom ni de sémantique.
- [ ] `llm_call_attempt` est émis par tentative sur les trois rails, ungated, avec les sept champs de §3.2.
- [ ] `LlmError::error_class()` est l'unique source du vocabulaire de classes ; `classify_delivery_error` y délègue et ses tests préexistants passent **sans modification**.
- [ ] Un échec de **lecture de corps** est retryable sur les trois rails ; un échec de **désérialisation** ne l'est sur aucun.
- [ ] Le seuil transport-aware (mika#1744) s'applique aussi au rail Ollama.
- [ ] `crates/mika-common/tests/llm_retry.rs` couvre les six cas de §3.4, `wiremock` ajouté en dev-dep de `mika-common`.
- [ ] V1 à V7 passent.
- [ ] `CLAUDE.md` porte les greps et la procédure à quatre branches, **avec son critère de halte** et l'angle mort de corrélation de D4.
- [ ] Le corps de PR énonce les rectifications F1/F4/F5 au ticket, et dit explicitement que le retry demandé par le corps du ticket existait déjà sur le rail de l'incident.

---

## Acceptance criteria

**AC1 — Le tour dit sa taille, y compris quand il échoue.**
Un tour LLM dont l'appel échoue émet un `turn_usage` portant `request_bytes` et `system_prompt_bytes` non nuls, **que `MIKA_STORE_LLM_CALLS` soit actif ou non**. Les champs valent `null` — jamais `0` — lorsque la valeur n'a pas pu être mesurée. Aucun champ préexistant de `turn_usage` n'est renommé, supprimé ni resémantisé.

**AC2 — Le tour dit combien de fois il a essayé.**
Chaque tentative d'appel LLM émet un `llm_call_attempt` portant `provider`, `model`, `attempt`, `max_attempts`, `elapsed_ms`, `outcome` ∈ {`success`, `retrying`, `exhausted`, `deadline_abort`}, `error_class` et `http_timeout_secs`. Les **trois** rails (OpenAI-compat, Ollama, Anthropic) l'émettent, de sorte que l'absence d'événement soit un fait sur l'appel et non sur le rail. L'événement est ungated.

**AC3 — Un timeout en cours de lecture du corps est retryable sur tous les rails.**
Sur les rails Ollama et Anthropic, un échec de lecture du corps produit une erreur de classe transport, retryable ; un échec de désérialisation d'un corps intégralement reçu reste `parse`, non retryable. Le seuil de reprise transport-aware de mika#1744 s'applique au rail Ollama.

**AC4 — Le test négatif du ticket passe, sur la boucle réelle.**
Un appel dont la première réponse est coupée en cours de corps est **retenté une fois** ; si le second essai aboutit, l'appel réussit ; si le second échoue, l'erreur est propagée et le nombre total de requêtes émises est **exactement `max_attempts`**, jamais davantage. Vérifié contre un serveur HTTP de test, pas contre un mock de provider.

**AC5 — Une classe d'erreur, une seule orthographe.**
Le vocabulaire `transport_timeout` / `transport` / `http_<status>` / `parse` / `provider` / `unsupported` / `other` a un unique site de définition dans `mika-common`. `classify_delivery_error` (mika#2179) y délègue et ses tests préexistants passent sans modification, de sorte que les `GROUP BY` opérateur existants sur `audit_events` restent valides.

**AC6 — La décision suivante est écrite avant d'être prise.**
`CLAUDE.md` porte la procédure de diagnostic à quatre branches de §3.5, son ordre (l'instrument mika#2293 d'abord), son critère de halte (« cinq hangs sans `llm_call_attempt` ⇒ la panne est en amont de l'appel HTTP »), et l'angle mort de corrélation de D4. Chaque branche nomme son remède ou son ticket de suite.

**AC7 — Ce qui n'est pas fait est dit.**
Le corps de PR énonce : que le retry « 1× sur transport » demandé par le corps du ticket **existait déjà** sur le rail portant 100 % des hangs mesurés (F1/F2) ; que 420 s n'est expliqué par aucun budget du code (F4) ; que l'hypothèse « le log pilote complet est dans le prompt » est réfutée par trois barrages (F5) ; et que la borne du prompt callback est **conditionnée à la mesure**, avec le ticket de suite pré-décrit en §4.
