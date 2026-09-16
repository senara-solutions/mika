# mika#2342 — l'appel LLM principal n'a qu'un seul mécanisme de coupure, et rien ne dit quand il a manqué

> - **Ticket :** senara-solutions/mika#2342
> - **Priorité :** p1 (substrate — loop-slower : grooms qui hangent, budget OpenRouter brûlé, slots cap tenus)
> - **Classe :** appel `llm_call` non borné (> 27 min) sans `completed`, sans `turn_usage`, sans erreur

## Contexte

Un tour mika-arch part en `llm_call started` et ne rend plus rien pendant 27+ minutes :
ni `llm_call completed`, ni `turn_usage`, ni erreur. La tâche reste `in_progress`, le
client A2A s'en va, le groom livre `first-pass _arch_ask failed`. n=2 sur le même brief
(#2331), kimi/OpenRouter. Le ticket note que le plafond HTTP (420 s) et l'enveloppe
(600 s) n'ont pas firé, et propose deux hypothèses : (1) une chaîne de retry silencieuse
de N×420 s, (2) un `.timeout` reqwest inopérant sur le chemin de continuation.

La lecture du code déplace le diagnostic. Les deux hypothèses sont utiles et l'une des
deux est partiellement vraie — mais **aucune des deux n'est le défaut structurel**, et
celui-là, lui, est lisible sans reproduire l'incident.

## Ce qui est établi, et comment le vérifier

**E1 — Le site d'appel principal n'a AUCUNE borne indépendante de reqwest.**
`crates/mika-agent/src/agent_loop/mod.rs:1103-1107` appelle
`llm.send_message_with_deadline(request, Some(deadline.into())).await` **nu**, sans
`tokio::time::timeout`. L'enveloppe de 600 s n'est pas un enveloppeur : c'est un test en
haut d'itération de boucle (`mod.rs:~1045`, et la garde CI `scripts/check-loop-select.sh`
existe précisément pour protéger cette sémantique « iteration-top »). Tant que l'appel ne
rend pas la main, ce test n'est jamais atteint. `crates/mika-agent/CLAUDE.md` l'écrit noir
sur blanc et le présente comme délibéré : *« The provider's per-request `reqwest` timeout
is the sole cancellation mechanism for in-flight HTTP calls — the outer agent deadline
never drops a future mid-flight. »*

**Conséquence, qui est le cœur de ce ticket : si le `.timeout()` de reqwest manque, pour
quelque raison que ce soit, plus rien dans le process ne peut borner l'appel.** Le
symptôme observé n'est pas « un timeout mal réglé », c'est « un mécanisme unique dont la
défaillance n'a pas de second filet et ne laisse aucune trace ».

Vérification : `grep -n "send_message_with_deadline" crates/mika-agent/src/agent_loop/mod.rs`
→ deux sites, 543 et 1105.

**E2 — L'asymétrie est à l'intérieur du même fichier, ce qui rend E1 difficile à lire
comme un choix.** `attempt_continuation_turn` (`mod.rs:541-545`) enveloppe *son* appel
dans `tokio::time::timeout(continuation_timeout, …)` et persiste une ligne `llm_calls` sur
**les trois** arms (succès, erreur provider, timeout — `mod.rs:548-630`).
`crates/mika-agent/src/server/investigate.rs:799` fait de même
(`tokio::time::timeout(INVESTIGATION_TIMEOUT, …)`). Le site principal de `run_loop` est
donc le **seul** appel LLM du chemin agent sans filet. Le motif sûr existe déjà dans la
maison ; il n'a jamais été appliqué là où la boucle passe le plus souvent.

**E3 — L'hypothèse 1 du ticket est confirmée comme défaut de journalisation, et réfutée
comme explication.** Confirmée : `llm_call started` est émis **une seule fois, avant** la
boucle de retry (`crates/mika-common/src/llm/openai.rs:341-346`, boucle
`for attempt in 0..max_attempts` ligne 373). N tentatives ⇒ un seul `started`, aucune
latence par tentative, aucune taille de requête. Nuance à ne pas escamoter : la ligne 414
émet bien un `warn!("retrying OpenAI-compatible API call")` par retry, donc la chaîne
n'est pas *totalement* muette — mais elle ne porte ni identité d'appel ni durée.

Réfutée comme explication de 27 min : avec `deadline: Some`,
`max_attempts = floor(enveloppe / plafond)` clampé à `MAX_RETRIES + 1 = 4`
(`openai.rs:367-371`, `budget.rs:270-275`). À 420/600 cela vaut **1**. Une seule tentative
de 420 s au plus. Même à 240/900 (le réglage mika#2189 de mika-arch), cela vaut 3, soit
720 s = 12 min. **Aucune configuration en vigueur ne produit 27 min par la chaîne de
retry**, tant que le deadline est présent — et `run_loop:1105` le passe toujours, aucun
appelant du chemin agent ne passe `None` (recherche exhaustive de `.send_message(` : les
seuls appelants sans deadline sont la calibration, la compaction, le KG et l'investigate,
aucun n'émet sous `run_loop`).

L'arithmétique qui *collerait* est `MAX_ATTEMPTS_HARD_CAP × 420 = 1680 s = 28 min`,
c'est-à-dire la branche `deadline.is_none()` de `openai.rs:370`. Elle est retenue comme
**hypothèse à mesurer**, pas comme conclusion : rien dans le code lu ne montre comment le
chemin arch y arriverait.

**E4 — L'hypothèse 2 n'est pas diagnosticable depuis le code seul, et c'est une raison de
plus de ne pas parier dessus.** `reqwest 0.12.28` / `hyper 1.11` ; `ClientBuilder::timeout`
(`openai.rs:188-191`) est documenté comme un timeout *total* couvrant la lecture du corps,
et `send_once` lit bien le corps via `response.text()` (`openai.rs:281`) à l'intérieur de
cette portée. Il n'y a **pas** de chemin de continuation distinct qui contournerait le
client : la continuation après un `ToolUse` repasse par la même itération de `run_loop` et
le même `send_once`. Autrement dit, la formulation du ticket (« le chemin de continuation
applique-t-il le `.timeout` ? ») porte sur une distinction qui n'existe pas dans le code ;
la question honnête est « pourquoi ce timeout-là n'a pas firé », et elle ne se tranche pas
par lecture.

**E5 — La corrélation mesurée par le ticket pointe vers la taille de la requête, et cette
taille est précisément ce qu'un hang détruit.** Repro B : tour `ToolUse` de 319 s et
**8391 tokens de sortie**, puis `gh_read`, puis l'appel qui ne revient pas. La requête
suivante porte donc le prompt système d'arch (~59,8 KB, cf. mika#2189) plus l'historique
plus une grosse sortie d'outil plus un résultat `gh_read`. `request_bytes` existe depuis
le schéma v53 et est *mesuré avant l'appel* (`mod.rs:1101`) — mais il n'est **écrit
qu'après** le retour de l'appel (`mod.rs:1110-1178`). Sur un hang, la ligne `llm_calls`
n'est jamais écrite et la taille est perdue. L'axe que le ticket désigne comme suspect est
exactement celui qu'on ne peut pas mesurer aujourd'hui sur les cas qui nous intéressent.

**E6 — Le pire cas légitime a déjà un nom canonique.**
`LlmTimeoutBudget::worst_case_failure_secs(hard_cap)` (`budget.rs:281-283`) rend
`max_attempts(hard_cap) × plafond`, et `budget.rs:419` asserte déjà
`worst_case ≤ enveloppe`. Le filet n'a donc pas besoin d'un nombre neuf. Ce que la méthode
n'inclut pas : les `sleep` de backoff (`openai.rs:413`, `500 ms × 2^(n-1)`, soit 3,5 s au
plus sur 4 tentatives) — d'où une marge explicite plutôt qu'implicite.

**E7 — Un rail n'honore pas ce budget, et le filet doit le savoir.**
`crates/mika-common/src/claude.rs:381` pose `Duration::from_secs(120)` en dur au lieu de
lire le plafond ; mika#2189 nomme l'incohérence et la laisse hors périmètre.
`MIN_HTTP_TIMEOUT_SECS = 10` (`llm/mod.rs:76`), donc un opérateur peut légalement poser un
plafond de 10 s — et un filet dérivé mécaniquement de ce budget vaudrait alors 40 s face à
un rail Anthropic qui prend physiquement jusqu'à ~480 s. **Faux positif garanti.** Le
filet ne doit donc pas *dériver* le pire cas ; il doit le **demander au rail**.

## Décisions

**D1 — Un filet de sécurité au site principal, calqué sur celui de la continuation.**
Envelopper `mod.rs:1103-1107` dans `tokio::time::timeout`, avec persistance de la ligne
`llm_calls` sur l'arm timeout, exactement comme `attempt_continuation_turn` le fait déjà
pour ses trois arms. On répare la classe (« un seul mécanisme, sans redondance et sans
trace ») plutôt que la cause reqwest, qu'on ne sait pas nommer.

**D2 — Le filet vaut `pire cas du rail + marge`, jamais le deadline restant.** Borner par
`remaining` réintroduirait mika#848 : un appel coupé en vol perd son résultat *et* sa
ligne `llm_calls`, y compris quand il allait aboutir. Le contrat maison est explicite —
*« Worst-case turn duration is `envelope + plafond` »* — et il est préservé tel quel. Calé
sur le pire cas, le filet **ne peut firer que lorsque reqwest a déjà manqué**, c'est-à-dire
sur un appel déjà perdu : le coût de le couper est nul, et son déclenchement est une
information de premier ordre.

**D3 — Le pire cas est déclaré par le rail, pas dérivé du budget.** Nouvelle méthode par
défaut sur `LlmProvider` :

```rust
fn worst_case_failure_secs(&self) -> u64 {
    self.timeout_budget().worst_case_failure_secs(DEFAULT_ATTEMPTS_HARD_CAP)
}
```

surchargée sur le rail Anthropic pour rendre son pire cas **réel** (`120 s × (MAX_RETRIES+1)`,
cf. E7). Ceci ne corrige pas `claude.rs:381` — hors périmètre, et le corriger ici
mélangerait deux tickets — mais transforme l'incohérence d'un piège silencieux en une
valeur déclarée. Marge : `LLM_WATCHDOG_MARGIN_SECS = 60`, largement au-dessus des 3,5 s de
backoff (E6). Soixante secondes de retard sur la détection d'un blocage de 27 minutes ne
coûtent rien ; un faux positif coûterait un tour.

À la géométrie en vigueur (420/600) : `max_attempts = 1`, filet à **480 s**. À 240/900 :
`max_attempts = 3`, filet à **780 s**. Dans les deux cas, très en deçà des 27 minutes
observées, et au-dessus de toute chaîne de retry légitime.

**D4 — L'instrumentation par tentative est le discriminateur, pas une décoration.** Le
filet seul dirait « quelque chose a dépassé le pire cas » sans dire quoi. Un événement
par tentative — `llm_call_attempt` avant `send_once`, avec `attempt`, `max_attempts`,
`request_bytes`, `provider`, `model` — sépare les trois lectures possibles du même
silence :

| ce qu'on verra | lecture |
|---|---|
| 4 tentatives d'environ 420 s | E3-bis : la branche `deadline.is_none()` est atteinte, la cause est en amont du provider |
| 1 tentative qui dépasse 420 s, puis le filet | le `.timeout()` reqwest n'a pas borné : hypothèse 2, cause à chercher au niveau transport/runtime |
| aucune tentative après le `started` | le blocage est **avant** `send_once` (sérialisation du corps, acquisition de connexion) — piste cohérente avec E5 |

`request_bytes` est porté sur l'événement de **début** précisément parce que c'est la
seule façon de connaître la taille d'une requête qui ne revient jamais (E5), et parce que
la corrélation du ticket désigne cet axe.

**D5 — Une garde structurelle, parce que la régression ne rendrait aucune assertion
fausse.** Un futur refactor qui retire le filet ne casse aucun test : il rend seulement le
site muet à nouveau, et toutes les assertions existantes restent vertes. C'est la classe
que la maison traite par scan de source (`policy.rs::no_bare_agent_timeout_constant_remains`,
`scripts/check-loop-select.sh`, `grooming_marker::tests::no_grooming_regex_outside_this_module`).
Garde : tout `send_message_with_deadline` dans `agent_loop/mod.rs` doit être argument
d'un `tokio::time::timeout`.

**D6 — Le filet a son propre nom d'événement, et il doit rester vide.** `llm_call_watchdog_fired`
(WARN + ligne `audit_events`), **sole writer**. Son absence est une information : elle dit
que reqwest borne effectivement les appels. Le confondre avec une erreur transport
ordinaire perdrait exactement le signal pour lequel on écrit ce code.

**D7 — Aucune valeur n'est changée.** Ni le plafond, ni l'enveloppe, ni `MAX_RETRIES`, ni
le défaut de flotte. Ce travail rend l'appel borné et le blocage lisible ; le recalibrage
éventuel se décidera sur la mesure que D4 produit.

## Volets d'implémentation

### V1 — `worst_case_failure_secs` sur le trait (`crates/mika-common/src/llm/mod.rs`)

Méthode par défaut sur `LlmProvider` (D3), à côté de `timeout_budget()`, avec la constante
`DEFAULT_ATTEMPTS_HARD_CAP` exposée depuis `openai.rs` ou remontée dans `llm/mod.rs`
(`MAX_ATTEMPTS_HARD_CAP` y est déjà `pub(crate)` depuis mika#2293). Surcharge sur le rail
Anthropic (`crates/mika-common/src/llm/anthropic.rs`) rendant `120 × (MAX_RETRIES + 1)`,
avec un commentaire nommant `claude.rs:381` et le ticket qui le laisse en place.

Tests : le pire cas déclaré par chaque rail est ≥ son plafond effectif ; le rail Anthropic
ne descend pas sous son littéral même quand l'environnement pose un plafond plus petit
(contrôle négatif de E7, à `MIN_HTTP_TIMEOUT_SECS`).

### V2 — Instrumentation par tentative (`crates/mika-common/src/llm/openai.rs`)

Dans `send_message_inner`, à l'intérieur de la boucle et **avant** `send_once` :
`info!(target: "mika::otel", attempt, max_attempts, request_bytes, provider, model, "llm_call_attempt")`.
`LlmRequest::payload_bytes()` existe déjà (utilisé par `mod.rs:1101`) — le calculer une
fois avant la boucle, pas par tentative. L'`info!("llm_call started")` existant reste, il
porte désormais aussi `max_attempts` (aujourd'hui invisible, et c'est la moitié de
l'ambiguïté de E3).

Même traitement sur `crates/mika-common/src/llm/ollama.rs` (structure identique,
`started` hors boucle ligne 527) et sur le rail Anthropic `crates/mika-common/src/claude.rs`
(`started` ligne 512) : n'instrumenter qu'un rail laisserait la même cécité sur les autres,
et c'est le défaut de couverture partielle que la maison a déjà eu à refuser une fois.

### V3 — Le filet (`crates/mika-agent/src/agent_loop/mod.rs`)

Au site 1103-1107 :

```rust
let watchdog = Duration::from_secs(
    llm.worst_case_failure_secs() + LLM_WATCHDOG_MARGIN_SECS,
);
let llm_result = match tokio::time::timeout(
    watchdog,
    llm.send_message_with_deadline(request, Some(deadline.into())),
).await {
    Ok(r) => r,
    Err(_) => { /* WARN nommé + audit + Err synthétique */ }
};
```

L'arm timeout : `warn!(target: "mika::otel", …, "llm_call_watchdog_fired")` portant
`watchdog_secs`, `http_timeout_secs`, `max_attempts`, `request_bytes`, `provider`,
`model`, `step`, `trace_id` ; une ligne `audit_events` (`tool_name = "llm_call_watchdog"`) ;
puis un `LlmError` synthétique routé dans la branche `Err` **existante** (`mod.rs:1152`),
pour que la ligne `llm_calls` soit persistée avec `status = "error"`, la latence réelle et
`request_bytes` — sans dupliquer le chemin de persistance.

Choix de la variante d'erreur : `LlmError::Transport`, pour que la classification
mika#2179 (`error_class = "transport_timeout"`) et le seuil de retry transport de
mika#1744 la traitent comme la coupure qu'elle est. Le message doit nommer le filet, pas
mimer une erreur reqwest — un opérateur qui lit `failed to read response body` doit
pouvoir continuer à croire que c'est reqwest qui a parlé.

`LLM_WATCHDOG_MARGIN_SECS` vit à côté du site, avec le raisonnement de D3 en commentaire.

### V4 — Garde structurelle (`crates/mika-agent/src/agent_loop/mod.rs`, `#[cfg(test)]`)

Scan du source de `agent_loop/mod.rs` : chaque occurrence de `send_message_with_deadline`
hors bloc de test doit être précédée, dans la même expression, d'un
`tokio::time::timeout`. Écrire le jeton en deux moitiés via `concat!` pour que la garde ne
soit pas son propre premier contrevenant (motif de `policy.rs:155-158`). Le message
d'échec doit nommer mika#2342 et dire *pourquoi* le filet est là — une garde qui dit
seulement « interdit » se fait désarmer au premier refactor pressé.

### V5 — Documentation

- `crates/mika-agent/CLAUDE.md` § *Deadline enforcement* : la phrase « the provider's
  per-request reqwest timeout is the **sole** cancellation mechanism » devient fausse et
  doit être réécrite, en gardant la raison d'origine (ne pas dropper un future en vol) et
  en expliquant pourquoi un filet calé sur le pire cas ne la contredit pas.
- `CLAUDE.md` racine § *Optional (LLM timeout budgets — mika#2189)* : ajouter les signaux
  opérateur (D6) et la sonde post-déploiement.
- `crates/mika-common/CLAUDE.md` : `worst_case_failure_secs` sur le trait, et pourquoi le
  rail Anthropic le surcharge.

## Verification contract

**Tests unitaires (mika-common)**

1. `worst_case_failure_secs` par rail : OpenAI-compatible suit le budget ; Anthropic ne
   descend jamais sous son littéral de 120 s, y compris à `MIN_HTTP_TIMEOUT_SECS`.
2. `llm_call_attempt` est émis une fois par tentative : contrôle positif à 1 tentative,
   contrôle positif à N > 1 (via un serveur de test rendant des 500 retryables). Un test
   à une seule tentative ne prouverait rien — c'est exactement l'état actuel.

**Tests d'intégration (mika-agent, eval harness)**

3. **Contrôle positif du filet** : un `MockLlmProvider` dont la réponse est
   `Delayed { sleep_ms }` très au-delà du filet, sous `tokio::time::pause()` +
   `start_paused = true` (motif de `tests/eval/test_deadline_in_flight_llm_call.rs`). Le
   tour se termine ; une ligne `llm_calls` en `status = "error"` existe, portant
   `request_bytes` non nul ; `llm_call_watchdog_fired` est émis.
4. **Contrôle négatif** : un appel dont la durée est sous le filet mais **au-dessus** du
   plafond par tentative n'est pas coupé par le filet. C'est le test qui prouve que le
   filet est un anti-hang et non un second deadline.
5. **Contrôle négatif de chaîne** : une chaîne de retry légitime complète (`max_attempts`
   tentatives, chacune à son plafond, plus les backoffs) reste sous le filet. Sans lui, la
   marge de D3 est une affirmation, pas une propriété.
6. **Non-régression mika#848** : sur un appel qui aboutit juste après le deadline mais
   avant le filet, la ligne `llm_calls` est bien persistée et le résultat utilisé.

**Garde structurelle**

7. V4 rougit sur un `send_message_with_deadline` non enveloppé (contrôle positif écrit
   dans le test lui-même, sur une chaîne fabriquée, pas en modifiant le source).

**CI**

8. `cargo test`, `cargo clippy`, `cargo fmt --check` verts ;
   `cargo test -p mika-agent --test eval` vert.

## Definition of Done

- Le site d'appel LLM principal de `run_loop` est enveloppé, la ligne `llm_calls` est
  persistée sur l'arm timeout, et le déclenchement porte un nom d'événement dédié.
- Chaque tentative de la chaîne de retry est observable sur les trois rails, avec la
  taille de requête disponible **avant** le retour de l'appel.
- Le pire cas est déclaré par le rail et non dérivé d'un budget qu'un rail n'honore pas.
- Une garde structurelle refuse le retour d'un appel non enveloppé.
- Aucune valeur de plafond, d'enveloppe ou de `MAX_RETRIES` n'a bougé.
- Les trois `CLAUDE.md` concernés disent ce que le code fait, en particulier là où ils
  affirmaient le contraire.

## Acceptance criteria

*Le ticket ne porte pas de section `## Acceptance criteria` ; celles-ci sont dérivées de
son énoncé — « borner réellement l'appel (le timeout doit firer) » et « instrumenter un log
par tentative » — et des faits E1-E7.*

- **AC1** — Un appel LLM du chemin agent ne peut plus dépasser
  `pire-cas-du-rail + LLM_WATCHDOG_MARGIN_SECS` en horloge murale, quelle que soit la
  raison pour laquelle le timeout reqwest ne fire pas. Vérifié par le contrôle positif (3)
  avec temps virtuel, donc sans dépendre d'une reproduction de l'incident.
- **AC2** — Le dépassement produit une erreur et une trace, jamais un silence : un
  événement WARN nommé, une ligne `audit_events`, et une ligne `llm_calls` en `error`
  portant `request_bytes` et la latence réelle.
- **AC3** — Chaque tentative de la chaîne de retry émet un événement portant `attempt`,
  `max_attempts` et `request_bytes`, sur les rails OpenAI-compatible, Ollama et Anthropic.
  Un opérateur peut distinguer « un appel non borné » de « N appels bornés muets » en
  lisant le journal, sans instrumenter davantage.
- **AC4** — Le filet ne coupe aucun appel légitime : une chaîne de retry complète à la
  géométrie configurée reste sous le seuil (contrôles négatifs 4 et 5).
- **AC5** — La régression est refusée structurellement : retirer le filet fait rougir un
  test, et le message d'échec nomme mika#2342 et sa raison.
- **AC6** — Aucun réglage n'est modifié par cette PR (contrôle : le diff ne touche ni
  `DEFAULT_HTTP_TIMEOUT_SECS`, ni `DEFAULT_AGENT_TOTAL_TIMEOUT_SECS`, ni `MAX_RETRIES`, ni
  un `config.toml` d'agent).

## Surfaces opérateur et sonde post-déploiement

**Grep** dans `$MIKA_SPIRIT_LOG_FILE` :

- `llm_call_watchdog_fired` — **doit rester vide**. Toute occurrence est un appel que
  reqwest n'a pas borné ; c'est la cause racine de mika#2342 rendue visible, et elle
  alimente son ticket de suivi. Les champs `request_bytes` et `max_attempts` sont ce qui
  permet de trancher entre les trois lectures de D4.
- `llm_call_attempt` — `jq 'select(.attempt > 0)'` donne le volume réel de retry, jusqu'ici
  impossible à compter.
- `llm_budget_resolved` (mika#2293, déjà en place) —
  `jq '{agent_id, http_timeout_secs, agent_total_timeout_secs, http_source}'`.

**SQL** : `SELECT COUNT(*) FROM audit_events WHERE tool_name = 'llm_call_watchdog';`

**Sonde, avec sa halte.** Le ticket affirme « env du process mika-spirit (vérifié) :
420/600 ». Si c'est exact, alors le `240/900` posé à mika-arch par mika#2189 le 06/09 est
**écrasé** : la cascade est `.env per-agent > env du process > config.toml per-agent`
(mika#2218), donc une variable de service annule le réglage per-agent. C'est précisément
la classe que mika#2293 a outillée. Lire `llm_budget_resolved` pour mika-arch et regarder
`http_source` :

- `process_env` à 420 → confirmé, `max_attempts = 1`, et la correction relève de
  l'environnement du service, pas d'une valeur à remonter ;
- `agent_config` à 240 → l'affirmation du ticket est fausse et l'arithmétique de E3 doit
  être refaite avant toute conclusion.

Sur 48 h après déploiement, `llm_call_watchdog_fired` non vide est un **résultat**, pas une
panne : il attribue enfin le blocage. `llm_call_watchdog_fired` répété sur un même agent
sans `llm_call_attempt` intermédiaire pointe vers un blocage **avant** `send_once` (E5) —
**halte** : ne pas remonter le filet, c'est un autre défaut.

## Hors périmètre (suivi à ouvrir)

- **La cause reqwest elle-même.** Ce travail borne et rend lisible ; il ne fait pas
  disparaître la raison pour laquelle un `.timeout()` total n'a pas firé. Le ticket de
  suivi s'ouvre sur la première occurrence de `llm_call_watchdog_fired` avec ses champs.
- **`claude.rs:381`** — le littéral `120s` au lieu de `http_timeout_secs()`, déjà nommé
  hors périmètre par mika#2189. D3 le rend déclaré, pas cohérent. Ticket de suivi.
- **`MIKA_AGENT_TOTAL_TIMEOUT_SECS=600` posé en variable de service** alors que mika#2189
  prescrit le `config.toml` per-agent (voir la sonde ci-dessus). Question de déploiement,
  pas de code.
- **mika#2331** (« `_arch_ask` doit retenter 1× sur timeout/hang ») reste en aval : son
  retry dépend du fait que l'appel rende la main ou erre. AC1/AC2 lui rendent cette
  précondition ; ce plan ne l'implémente pas.
- **Le volume du prompt système d'arch** (59,8 KB, mika#2189), piste plausible de E5 :
  ticket distinct, la mesure de D4 le renseignera.

## Revision history

- 2026-09-16 — rédaction initiale (content-only, revue architecte à suivre).
