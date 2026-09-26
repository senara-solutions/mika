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

### F8 — Le rail Anthropic porte deux barrages supplémentaires que les deux autres n'ont pas

Mesure faite en écrivant la §*Fire-Disposition* (rev 2), et elle **contredit la rev 1 sur un point** : la §3.3 telle qu'elle était écrite n'aurait pas suffi à rendre le rail Anthropic retryable, et le cas 6 de §3.4 aurait échoué **après** le correctif autant qu'avant.

1. **`is_retryable` y est conditionnel.** `crates/mika-common/src/claude.rs:796-801` :
   ```rust
   ClaudeApiError::Transport(e) => e.is_timeout(),
   ```
   Là où `LlmError::is_retryable` rend `Transport(_) => true` **inconditionnellement** (F1). Un corps coupé en cours de route n'est pas un timeout `reqwest` (`unexpected EOF`, `decode`, `reset` — la chaîne de causes que le motif mika#2015 exhume, `openai.rs:279-300`) : classer l'échec de lecture en `ClaudeApiError::Transport` le laisserait donc **non retryable**. Le découpage `.text()` puis désérialisation est nécessaire sur ce rail, il n'est pas suffisant.

2. **La classe d'erreur y est aplatie à la frontière.** `crates/mika-common/src/llm/anthropic.rs:64` :
   ```rust
   .map_err(|e| LlmError::ProviderError(e.to_string()))?
   ```
   Toute erreur du rail Anthropic sort du `LlmProvider` en `ProviderError`, donc en classe `provider`, quelle qu'ait été sa cause. Conséquence pour §3.4 : sur ce rail, **la seule assertion honnête est le nombre de requêtes reçues**, jamais la classe propagée. Conséquence pour §3.2 : l'événement `llm_call_attempt` doit y être émis **depuis la boucle interne de `claude.rs`**, où la cause est encore typée, et sa classe vient d'un mapping local — d'où la contrainte d'orthographe unique reformulée en AC5.

Ces deux faits ne changent rien aux hangs mesurés (aucun n'est sur ce rail, F1) ; ils changent ce que §3.3 et §3.4 doivent livrer pour que l'AC3 « tous les rails » soit vrai plutôt qu'annoncé.

> **Ancrage des citations** : tous les numéros de ligne de ce plan sont relevés au SHA `45cec264` de la branche, et **re-vérifiés contre l'arbre à ce SHA** — `is_retryable` (`llm/error.rs`), `max_attempts` (`llm/budget.rs`), les trois paires `build_turn_usage_fields`/`emit_turn_usage`, `request_bytes` (l. 1101), les deux `response.json()` d'`ollama.rs`/`claude.rs`, le `retry_threshold_secs` sans branche transport d'`ollama.rs`, et le `.timeout(Duration::from_secs(120))` en dur de `claude.rs`. Les citations de F8 et de la §*Fire-Disposition* (rev 2) sont relevées au SHA `0874eab1` : `is_retryable` d'`claude.rs`, le `map_err` d'`anthropic.rs:64`, `classify_delivery_error` (`dispatcher.rs:218-236`), les deux `matches!(e, ClaudeApiError::Transport(_))` du seuil mika#1744 (`claude.rs:545`, `:654`), et les `dev-dependencies` de `crates/mika-common/Cargo.toml:61-64`. Les noms de symboles restent l'ancre porteuse — si un numéro a dérivé sous un rebase, c'est le symbole qui fait foi.

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

**Le rail Anthropic émet depuis sa boucle interne, et c'est F8 qui l'impose.** `anthropic.rs:64` aplatit toute erreur en `LlmError::ProviderError` : un événement émis au-dessus de cette frontière rapporterait `error_class = "provider"` sur un timeout transport, c'est-à-dire une ligne fausse plutôt qu'une ligne absente. L'émission est donc posée dans la boucle de `claude.rs` (l. 533-590), là où la cause est encore typée, et la classe vient d'un `ClaudeApiError::error_class()` local. **Les deux mappings, un seul vocabulaire** : les sept littéraux deviennent des constantes d'un unique site dans `llm/error.rs`, auxquelles `LlmError::error_class()` et `ClaudeApiError::error_class()` se réfèrent toutes deux. C'est la forme que D3 exige — un site de définition — et non un site d'implémentation, qu'un enum étranger ne peut de toute façon pas partager.

Les `warn!` actuels (`"transient API error"`, `"retrying OpenAI-compatible API call"`, `"aborting retry chain — …"`) sont **conservés** : ils sont greppables par message et un opérateur peut déjà s'y appuyer. Le nouvel événement ne les remplace pas, il ajoute le champ `event` structuré, la durée par tentative et la classe d'erreur — les trois choses qui manquent pour trancher F4.

> **Couverture, pas partialité.** Les trois rails sont instrumentés dans le même travail, y compris ceux qui ne portent aucun hang mesuré. Un événement posé sur un seul rail ferait lire l'absence de ligne comme « ce tour n'a pas retenté » alors qu'elle ne dirait que « ce rail n'est pas instrumenté » — le piège « l'absence n'est pas une preuve » que ce dépôt a déjà dû nommer plusieurs fois.

### 3.3 — Combler les deux trous de retry (AC3)

**`crates/mika-common/src/llm/ollama.rs:502-505`** et **`crates/mika-common/src/claude.rs:786-787`** : remplacer `response.json()` par le motif mika#2015 d'`openai.rs:281-300` — lire `.text()`, et **seulement ensuite** désérialiser :

- échec de **lecture** ⇒ `LlmError::Transport` (resp. `ClaudeApiError::Transport`) + `warn!` avec la chaîne de causes ⇒ retryable ;
- échec de **désérialisation** ⇒ `ParseError` ⇒ non retryable, inchangé.

C'est ce découpage, et non un élargissement de `is_retryable`, qui distingue les deux causes : les octets ne sont jamais arrivés, contre les octets sont arrivés et sont illisibles.

**Sur le rail Anthropic, le découpage ne suffit pas (F8-1), et voici le complément.** `claude.rs:796-801` rend `Transport(e) => e.is_timeout()` : un `ClaudeApiError::Transport` né d'un corps coupé resterait non retryable. Plutôt que d'élargir `Transport` en `=> true` — ce qui rendrait aussi retryable un refus de connexion et changerait un comportement que ce ticket n'a pas mesuré —, ajouter un variant dédié :

```rust
/// Body read failed mid-stream: the bytes never arrived (mika#2015 pattern,
/// ported to this rail by mika#2331). Retryable unconditionally, unlike
/// `Transport`, whose is_timeout() condition this variant deliberately
/// does not inherit.
#[error("Claude API response body read failed")]
BodyRead(String),
```

- `is_retryable` : `ClaudeApiError::BodyRead(_) => true`.
- **Seuil transport-aware** : les deux `matches!(e, ClaudeApiError::Transport(_))` qui calculent `last_was_transport` (`claude.rs:545` et `:654`) doivent inclure `BodyRead`, sans quoi le rail retomberait sur le seuil long `0,75 + 0,25` après précisément l'erreur que mika#1744 existe pour traiter vite. **C'est le trou le plus facile à laisser ouvert de tout ce plan** : l'oubli ne casse aucun test et ne produit aucun symptôme lisible — seulement une chaîne de retry abandonnée plus tôt qu'il ne faut.
- `error_class()` : `BodyRead` rend `transport_timeout` si la chaîne de causes contient `timed out`, `transport` sinon — même prédicat que `LlmError`, mêmes constantes (§3.2).
- **Coût mécanique, borné et mesuré** : `ClaudeApiError` n'est nommé **dans aucun fichier hors `crates/mika-common/src/claude.rs`**. Les sites à compléter sont donc les six de ce fichier : le `match` exhaustif de contextualisation (l. 617-642), `is_retryable` (l. 796), les deux `matches!` du seuil, et le site de construction (l. 785). Aucun consommateur externe ne voit le nouveau variant.

Le variant n'est **pas** ajouté à `LlmError`, qui n'en a pas besoin : `Transport` y est déjà retryable sans condition.

**`ollama.rs:540-541`** : porter le seuil transport-aware de mika#1744, aujourd'hui absent de ce rail (il utilise le seuil long `0,75 + 0,25` du plafond même après une erreur transport, là où `openai.rs:396` bascule sur `transport_retry_min_remaining_secs()`).

**Coût assumé, à écrire au site** : retenter une complétion facture un second appel. C'est le prix déjà accepté sur le rail OpenAI depuis mika#2015, et une complétion n'a pas d'effet de bord côté serveur au-delà de la facturation.

**Hors périmètre, nommé au site** : `claude.rs:381-384` pose `.timeout(Duration::from_secs(120))` en dur au lieu d'appeler `budget.http_timeout_secs()`, et la boucle Anthropic ignore `max_attempts`. Incohérence réelle, déjà nommée dans `llm/mod.rs:281-284` et dans le CLAUDE.md de la crate, et sans rapport avec la classe de ce ticket (aucun hang mesuré n'est sur le rail Anthropic).

### 3.4 — Le test négatif que le ticket exige (AC4)

**Il n'existe aujourd'hui aucun test de la boucle de retry** — sur aucun des trois rails. `MockLlmProvider` ne peut pas en produire : il n'override pas `send_message_with_deadline` et rend son `MockResponse::Error` sans boucle (`llm/mock.rs:37`, `:122`, `:213`).

Ajouter `wiremock` aux dev-dependencies de `crates/mika-common/Cargo.toml` (déjà au workspace, `Cargo.toml:151`, déjà dev-dep de `mika-agent` et `mika-gateway`) et écrire `crates/mika-common/tests/llm_retry.rs` :

1. **Le test du ticket** — 1ʳᵉ réponse coupée mid-body (corps annoncé plus long qu'envoyé) ⇒ `Transport` ; 2ᵉ réponse valide ⇒ **succès, exactement 2 requêtes reçues**.
2. **Pas de boucle infinie** — deux échecs transport d'affilée ⇒ `Err` propagée, **exactement `max_attempts` requêtes reçues**, jamais davantage.
3. **`max_attempts` est bien la borne** — à la géométrie 120/300, un échec permanent produit **2** requêtes, pas 4 ; épingle F2 contre une dérive vers `MAX_RETRIES`.
4. **La classe est la bonne** — **sur le rail OpenAI-compat**, l'erreur propagée rend `error_class() == "transport_timeout"` sur la chaîne d'incident verbatim (déjà figée à `dispatcher.rs:3778-3779`). La restriction au rail est portante : F8-2 montre que sur le rail Anthropic la classe propagée est toujours `provider`, et asserter `transport_timeout` à cette frontière asserterait quelque chose de faux.
5. **Non-retryable reste non-retryable** — un HTTP 400 ⇒ **1** requête.
6. **a) Rail Ollama** — 1ʳᵉ réponse coupée mid-body, 2ᵉ valide ⇒ succès, **2** requêtes (aujourd'hui : `ParseError`, non retryable, **1** requête ; c'est le test qui échoue avant §3.3 et passe après).
   **b) Rail Anthropic** — même scénario, même assertion : **2** requêtes. **L'assertion porte sur le compte, jamais sur la classe** (F8-2). Elle est de surcroît indépendante de `MAX_RETRIES`, que ce rail consomme encore à la place de `max_attempts` (§3.3, hors périmètre) : un succès au 2ᵉ essai s'arrête à 2 quelle que soit la borne.

**Accessibilité des trois rails à un serveur de test — mesurée, et elle n'est pas uniforme.** `OpenAiCompatClient` et `OllamaClient` prennent une `base_url` en paramètre de construction (`openai.rs:180`, `ollama.rs:199`) : pointables sur wiremock sans toucher au code de production. `ClaudeClient` **poste sur une constante en dur** — `API_URL` (`claude.rs:11`), unique usage à `claude.rs:746` — et n'est donc atteignable par aucun test. Le cas 6b exige par conséquent un champ `base_url` sur `ClaudeClient`, initialisé à `API_URL` par `new()`, plus un constructeur de test derrière la feature **`test-utils` déjà déclarée** par ce crate (`Cargo.toml:43`, la convention qui gate déjà `MockLlmProvider`) — un test d'intégration de `tests/` ne voyant pas le `#[cfg(test)]` du crate. La dépendance de test s'ajoute alors en `mika-common = { path = ".", features = ["test-utils"] }` côté dev-dependencies. **Ce n'est pas un contournement du test : c'est la condition pour que le rail soit testable du tout**, et l'absence de cette condition est précisément pourquoi aucune boucle de retry n'a jamais été testée ici.

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

Troisième ticket de dette, découvert en rev 2 : `anthropic.rs:64` aplatit **toute** erreur du rail Anthropic en `LlmError::ProviderError(e.to_string())`, donc en classe `provider`, quelle qu'ait été la cause (F8-2). Ce plan contourne le défaut là où il le faut — l'événement `llm_call_attempt` est émis sous la frontière, et le cas 6b n'asserte que le compte de requêtes — mais ne le corrige pas : traduire fidèlement `ClaudeApiError` en `LlmError` change la classe d'erreur vue par tout le moteur sur ce rail, ce qui est un changement de format de fil et mérite son propre ticket et sa propre mesure.

---

## Fire-Disposition

*(mika#1574 / review-guide.md § Fire-Disposition Gate — section ajoutée en rev 2 sur F1.)*

Quatre livrables de ce plan sont de classe détecteur. Tous sont des `#[test]` / `#[tokio::test]` ordinaires, donc **bloquants en CI par le `cargo test --workspace` existant** (V2) : aucun nouveau job à créer, et aucune possibilité de les lander verts mais inertes.

La disposition ci-dessous est écrite sur une population **comptée dans l'arbre à `0874eab1`**, pas supposée. Le comptage a contredit la rev 1 sur deux points, tous deux reportés dans le corps (F8, §3.3, §3.4) plutôt que dissimulés ici.

| Détecteur | Où | Fire sur l'existant ? |
|---|---|---|
| T1 — `llm_retry.rs` cas 1 à 5 | rail OpenAI-compat | **Non** (mesuré) |
| T2 — `llm_retry.rs` cas 6a / 6b | rails Ollama / Anthropic | **Oui** — c'est le seul |
| T3 — parité `error_class` ↔ `classify_delivery_error` | `mika-common` + `mika-agent` | **Non** (mesuré) |
| T4 — propagation `None` / `Some(n)` de `build_turn_usage_fields` | `mika-agent` | **Non** (code neuf) |

### T1 — la boucle OpenAI-compat fait déjà ce que les cas 1 à 5 assertent

**Population : zéro violation.** C'est le contenu même de F1 et F2 — `LlmError::Transport` est inconditionnellement retryable (`llm/error.rs:32-38`) et `max_attempts` borne déjà la chaîne à 2 pour la géométrie 120/300 (`llm/budget.rs:270-275`, consommé à `openai.rs:367-371`). Les cinq cas sont des **tests de caractérisation** : ils figent un comportement livré, ils ne le demandent pas.

**Disposition : (a) allowlist nommée, avec une liste vide.** Aucune exemption n'est écrite, parce qu'exempter d'un détecteur ce qui le passe déjà crée une dispense morte que rien ne nettoie ensuite.

**Halt-and-surface conditionnel, et il est porteur.** Si l'un de ces cinq cas est rouge à la pose — en particulier le cas 3, « 2 requêtes et non 4 » —, alors **F2 est faux**, et F2 est ce sur quoi repose toute la §3.5 (la lecture de `attempt` contre `max_attempts`). L'implémenteur ne doit alors ni exempter, ni ajuster l'assertion à ce qu'il observe : il **halte et remonte**, parce qu'un plan dont la prémisse de mesure vient d'être démentie ne se répare pas au niveau du test.

### T2 — le seul détecteur qui fire sur l'état existant, et il land avec son correctif

**Population : exactement deux sites, tous deux nommés.**
- `crates/mika-common/src/llm/ollama.rs:502-505` — `response.json().await.map_err(… LlmError::ParseError …)`, non retryable ;
- `crates/mika-common/src/claude.rs:785` — `response.json().await.map_err(ClaudeApiError::ParseError)`, non retryable, **et** `is_retryable` y conditionne même `Transport` à `is_timeout()` (F8-1).

Avant §3.3, le cas 6a observe 1 requête là où il en attend 2, et le cas 6b fait de même. **Ce sont des échecs attendus, et ils mesurent exactement le trou que §3.3 comble.**

**Disposition : co-location atomique — ni exemption, ni détecteur désarmé.** Les cas 6a/6b et le correctif §3.3 (découpage `.text()` + variant `BodyRead` + seuil transport-aware) atterrissent dans **le même PR**. À aucun instant de l'historique de `main` le détecteur n'existe sans son correctif, donc il n'y a **aucune violation préexistante à exempter** : la population passe de deux à zéro dans le commit qui introduit le détecteur. C'est la seconde branche que F1(b) prévoit explicitement, et c'est celle-ci qui s'applique.

L'ordre interne au PR est libre (test d'abord et rouge, ou correctif d'abord) ; ce qui est contraint est ce que porte le merge.

**Dérogation unique, et sa condition est nommée.** Si et seulement si §3.3 devait être scindé hors de ce PR — par exemple parce que le variant `BodyRead` révèle à l'implémentation un consommateur d'`ClaudeApiError` que le comptage n'a pas vu (le comptage dit : **aucun fichier hors `claude.rs` ne nomme cet enum**) —, alors et alors seulement le sous-cas concerné bascule en **(b) land disabled** :

```rust
// FIRE-DISPOSITION (mika#1574, plan mika#2331 §Fire-Disposition T2):
// disabled until <tracker> lands the body-read split on this rail.
#[ignore = "mika#2331: enable with the <tracker> body-read split"]
```

avec, dans le même test, l'**assertion auto-nettoyante** qui fait de l'`#[ignore]` une dette datée plutôt que permanente :

```rust
// Fires when the rail is repaired and the #[ignore] has gone stale.
assert!(
    matches!(err, /* encore la variante non retryable */),
    "rail repaired — remove the #[ignore] above and this assertion with it"
);
```

Le numéro de tracker est à créer **au moment de la scission**, pas d'avance : un ticket ouvert pour une scission qui n'aura pas lieu est un tracker mort, et ce plan prévoit que la scission n'ait pas lieu.

### T3 — le vocabulaire des classes n'a aujourd'hui qu'une seule orthographe

**Population : zéro seconde orthographe, mesurée.** `grep -rn 'transport_timeout' crates/` rend six lignes : le site de définition (`dispatcher.rs:228`), deux assertions de ses tests (`:3786`, `:3842`), et trois lignes de `tests/eval/test_callback_delivery_starvation.rs` (un nom de fonction, une valeur attendue, un commentaire de triage). **Aucune seconde écriture du vocabulaire n'existe** — ce que D3 existe pour empêcher n'a pas encore eu lieu.

**Disposition : (a) allowlist nommée, avec une liste vide.** Le détecteur de parité est bloquant dès le land, sans exemption ni période de grâce.

**Nuance qui appartient à cette section, parce qu'elle est née de la mesure (F8-2)** : la parité à vérifier n'est pas « une fonction » mais « un vocabulaire ». `ClaudeApiError` est un enum étranger à `LlmError` et ne peut pas partager une implémentation ; §3.2 fait donc des sept littéraux des **constantes d'un site unique**, et T3 asserte que les deux mappings ne produisent que des valeurs tirées de ce jeu. Un `error_class()` qui renverrait une huitième chaîne est exactement la divergence que D3 nomme, et T3 la voit.

### T4 — `build_turn_usage_fields` ne fire sur rien d'existant

**Population : zéro.** Les deux champs `request_bytes` / `system_prompt_bytes` n'existent pas encore ; l'assertion porte sur un comportement introduit par ce PR.

**Disposition : sans objet** — il n'y a pas d'état préexistant sur lequel ce détecteur puisse firer. Les **dix** appels de test que l'élargissement de signature casse (§3.1 point 5) ne sont **pas** un fire : ce sont des appels à mettre à jour, sans signification, et la §*Verification contract* dit déjà pourquoi il ne faut pas les lire comme un signal.

### Ce qui n'est pas un détecteur, et qu'il ne faut pas confondre

**V4 est une contrainte de non-modification, pas un détecteur.** « Les quatre tests de `classify_delivery_error` passent sans être modifiés » est une règle sur le **diff**, que rien dans le code ne peut faire firer ; aucune disposition ne lui est due. Elle est énoncée en §*Verification contract*, avec son inverse sur V5, et les deux se lisent ensemble.

---

## Verification contract

| # | Vérification | Commande |
|---|---|---|
| V1 | Compilation et lints | `cargo clippy --workspace --all-targets -- -D warnings` |
| V2 | Suite complète non régressée | `cargo test --workspace` |
| V3 | Boucle de retry, les six cas (6a et 6b inclus) | `cargo test -p mika-common --features test-utils --test llm_retry` |
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
- [ ] Les sept classes ont un unique site de définition (constantes) ; `classify_delivery_error` délègue à `LlmError::error_class()` et ses tests préexistants passent **sans modification** ; `ClaudeApiError::error_class()` tire ses valeurs des mêmes constantes.
- [ ] Un échec de **lecture de corps** est retryable sur les trois rails ; un échec de **désérialisation** ne l'est sur aucun. Sur le rail Anthropic, la retryabilité passe par un variant dédié (`BodyRead`), inconditionnel, et non par un élargissement de `Transport`.
- [ ] Le seuil transport-aware (mika#1744) s'applique aussi au rail Ollama, et compte `ClaudeApiError::BodyRead` comme transport aux deux sites `last_was_transport` de `claude.rs`.
- [ ] `crates/mika-common/tests/llm_retry.rs` couvre les six cas de §3.4 (6a et 6b inclus), `wiremock` ajouté en dev-dep de `mika-common` ; `ClaudeClient` porte une `base_url` (défaut `API_URL`) et un constructeur de test derrière la feature `test-utils` existante, sans quoi le rail Anthropic n'est atteignable par aucun test.
- [ ] La section `## Fire-Disposition` couvre les quatre détecteurs T1 à T4 (mika#1574) ; T2 land atomiquement avec son correctif §3.3 et n'écrit aucune exemption.
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
Sur les rails Ollama et Anthropic, un échec de lecture du corps produit une erreur de classe transport, retryable ; un échec de désérialisation d'un corps intégralement reçu reste `parse`, non retryable. Sur le rail Anthropic la retryabilité est **inconditionnelle** — elle n'hérite pas de la condition `is_timeout()` que `ClaudeApiError::Transport` porte (F8-1) —, et le seuil de reprise transport-aware de mika#1744 compte cette erreur comme transport (`last_was_transport`, `claude.rs:545` et `:654`). Le même seuil s'applique au rail Ollama, où il est aujourd'hui absent.

**AC4 — Le test négatif du ticket passe, sur la boucle réelle.**
Un appel dont la première réponse est coupée en cours de corps est **retenté une fois** ; si le second essai aboutit, l'appel réussit ; si le second échoue, l'erreur est propagée et le nombre total de requêtes émises est **exactement `max_attempts`**, jamais davantage. Vérifié contre un serveur HTTP de test, pas contre un mock de provider.

**AC5 — Une classe d'erreur, une seule orthographe.**
Le vocabulaire `transport_timeout` / `transport` / `http_<status>` / `parse` / `provider` / `unsupported` / `other` a un unique site de **définition** dans `mika-common` — un jeu de constantes. `classify_delivery_error` (mika#2179) délègue à `LlmError::error_class()` et ses tests préexistants passent sans modification, de sorte que les `GROUP BY` opérateur existants sur `audit_events` restent valides. Le mapping du rail Anthropic (`ClaudeApiError::error_class()`, requis par F8-2) tire ses valeurs de ces mêmes constantes : deux mappings, jamais deux orthographes.

**AC6 — La décision suivante est écrite avant d'être prise.**
`CLAUDE.md` porte la procédure de diagnostic à quatre branches de §3.5, son ordre (l'instrument mika#2293 d'abord), son critère de halte (« cinq hangs sans `llm_call_attempt` ⇒ la panne est en amont de l'appel HTTP »), et l'angle mort de corrélation de D4. Chaque branche nomme son remède ou son ticket de suite.

**AC7 — Ce qui n'est pas fait est dit.**
Le corps de PR énonce : que le retry « 1× sur transport » demandé par le corps du ticket **existait déjà** sur le rail portant 100 % des hangs mesurés (F1/F2) ; que 420 s n'est expliqué par aucun budget du code (F4) ; que l'hypothèse « le log pilote complet est dans le prompt » est réfutée par trois barrages (F5) ; que le rail Anthropic porte deux barrages supplémentaires qui rendaient la §3.3 de la rev 1 insuffisante (F8), dont l'un — l'aplatissement en `ProviderError` — est contourné et non corrigé, avec son ticket de dette en §4 ; et que la borne du prompt callback est **conditionnée à la mesure**, avec le ticket de suite pré-décrit en §4.

---

## Revision history

- **rev 2 (2026-09-16)** — première passe architecte, `Disposition: ITERATE`, un finding bloquant.
  - **F1 adressé** : ajout de la section `## Fire-Disposition` (mika#1574 / review-guide.md § Fire-Disposition Gate), couvrant les quatre livrables de classe détecteur T1 à T4. Elle a été écrite **après avoir compté la population de chaque détecteur dans l'arbre à `0874eab1`**, et le comptage a contredit la rev 1 sur deux points, tous deux reportés dans le corps du plan plutôt que gardés dans la section :
    - **T2, le seul détecteur qui fire sur l'existant, a une population de deux sites nommés** (`ollama.rs:502-505`, `claude.rs:785`). Disposition retenue : **co-location atomique** — la branche que F1(b) prévoit en second (« document that the detector lands alongside the §3.3 fix »). Aucune exemption n'est écrite : la population passe de deux à zéro dans le commit qui introduit le détecteur, et exempter ce qui n'existera plus créerait une dispense morte. La dérogation en option (b) *land disabled* est écrite avec sa condition de déclenchement, son `#[ignore]` porteur de tracker et son assertion auto-nettoyante — à n'activer que si §3.3 est scindé hors du PR, ce que le plan ne prévoit pas.
    - **Le correctif de la rev 1 ne suffisait pas sur le rail Anthropic.** F1 supposait que « les tests vérifient un rail réparé » ; la mesure dit que `claude.rs:796-801` conditionne `Transport` à `is_timeout()`, qu'un corps coupé n'est pas un timeout, et que le seul découpage `.text()` aurait donc laissé le cas 6b rouge **après** §3.3. D'où le nouveau **F8**, le variant `BodyRead` en §3.3, et l'inclusion de `BodyRead` dans le `last_was_transport` du seuil mika#1744 — l'oubli le plus silencieux du plan, nommé au site.
    - **Deuxième contradiction, même rail** : `anthropic.rs:64` aplatit toute erreur en `ProviderError`. Le cas 6b n'asserte donc **que le compte de requêtes**, jamais la classe ; l'événement `llm_call_attempt` est émis **sous** cette frontière ; et AC5 devient « un site de définition, deux mappings ». Le défaut lui-même est nommé en dette (§4, troisième ticket) parce que le corriger change un format de fil.
    - **Troisième mesure, sur la faisabilité même du cas 6b** : `ClaudeClient` poste sur la constante `API_URL` en dur (`claude.rs:11`, `:746`) et n'est atteignable par aucun serveur de test. §3.4 porte désormais la `base_url` + le constructeur derrière la feature `test-utils` déjà déclarée par le crate. Sans cela, le détecteur T2-6b n'aurait pas pu exister — ce qui est aussi l'explication de son absence jusqu'ici.
  - T1, T3 et T4 sont disposés en **(a) avec liste vide**, chacun sur un comptage explicite (zéro violation). T1 porte en plus un **halt-and-surface conditionnel** : un cas 3 rouge signifierait que F2 est faux, donc que la §3.5 entière repose sur une prémisse démentie — l'implémenteur remonte au lieu d'ajuster l'assertion.
  - AC3, AC5, AC7, la *Definition of Done* et la ligne V3 du *Verification contract* sont mis à jour en conséquence. Aucun AC n'est affaibli : AC3 gagne la clause d'inconditionnalité et celle du seuil, AC5 gagne la contrainte sur le second mapping.
  - Les annotations A1 à A3 de la première passe ne portaient aucune demande de changement ; la strate la plus récente du ticket, la citation du commentaire 2 et l'étape 0 de la procédure de diagnostic sont conservées telles quelles.
