# mika#2362 — `retrying` est décidé par un prédicat qui ignore la deadline, et `max_attempts` compte une tentative que la garde refuse

- **Ticket :** senara-solutions/mika#2362
- **Priorité :** p1 (substrate — loop-slower)
- **Branche :** `feat/2362/llm-retry-retrying-mis-sans-2e-tentative`
- **Lignage :** #2331 (l'événement `llm_call_attempt`), #2342 (le filet, classe distincte), #2189 (`LlmTimeoutBudget`), #2293 (provenance du budget), #2361 (le redrive que ce défaut épuise)

---

## Contexte

Un tour mika-arch a été coupé au plafond HTTP exact et le journal a écrit
`outcome="retrying"` pour une tentative qui n'a jamais tourné :

```
14:11:36.940Z  llm_call_attempt  attempt=0  max_attempts=2  provider=openrouter
14:16:36.941Z  llm_call_attempt  attempt=0  max_attempts=2  elapsed_ms=300000  outcome="retrying"
14:16:36.959Z  turn_usage        stop_reason=error  latency_ms=300001  status=error
```

Dix-huit millisecondes séparent l'annonce du retry de l'échec du tour. Le ticket en
tire deux défauts — D1 (le journal ment) et D2 (sous 300/600, `max_attempts=2` est
décoratif). **Les deux sont réels et lisibles dans le code seul**, mais le second
n'a pas la forme que le ticket lui prête, et la différence décide du remède.

---

## Ce qui est établi, et comment le vérifier

### E1 — D1 est une divergence de prédicat, pas une ligne manquante

Deux prédicats répondent à la même question — *une tentative suivante va-t-elle
tourner ?* — et ils ne sont pas le même prédicat.

Le premier décide ce que la ligne dit (`crates/mika-common/src/llm/openai.rs:470-476`) :

```rust
let outcome = match &attempt_result {
    Ok(_) => SUCCESS,
    Err(e) if attempt + 1 < max_attempts && e.is_retryable() => RETRYING,
    Err(_) => EXHAUSTED,
};
```

Le second décide ce qui se passe, au tour d'itération suivant
(`openai.rs:382-431`) : il lit la deadline restante, la compare à un seuil
dépendant de la classe de la dernière erreur, et `break` si la marge est
insuffisante.

**Le premier ne consulte jamais la deadline.** Il est donc structurellement
possible — et c'est le cas mesuré — qu'il annonce `retrying` alors que le second
va immédiatement refuser. Vérification :
`grep -n "outcome = match" crates/mika-common/src/llm/openai.rs` puis lecture des
deux blocs ; aucune variable de deadline n'entre dans le premier.

### E2 — La divergence est écrite six fois : deux prédicats × trois rails

| Rail | Calcul de `outcome` | Garde de deadline |
|---|---|---|
| `llm/openai.rs` | 470-476 | 382-431 |
| `llm/ollama.rs` | 671-677 | 610-641 |
| `claude.rs` | 701-707 | 627-661 |

Les trois copies sont identiques à la nomenclature près (`claude.rs` lit
`MAX_RETRIES` et ses littéraux au lieu du budget — l'incohérence nommée hors
périmètre par mika#2189). **Corriger un seul rail reproduirait exactement la
classe que mika#2158 a dû fermer une fois** : un prédicat recopié est un prédicat
qui peut diverger, et la copie d'`auto_pull` portait sa propre confession en
commentaire pendant des mois sans que rien ne casse.

### E3 — La ligne `deadline_abort` existe déjà ; c'est la ligne `retrying` qui est fausse

Après `continue`, l'itération `attempt+1` émet **toujours** quelque chose : soit
`deadline_abort` (`elapsed_ms = 0`, la tentative qui n'a pas eu lieu), soit la
ligne de début de tentative de mika#2342. Le journal n'est donc pas muet — il est
**contradictoire** : une ligne annonce un retry, la suivante le dément.

Conséquence pour le remède : **D1 n'est pas « ajouter une ligne », c'est
« faire que la ligne N décrive la décision qui gouverne la ligne N+1 »**. Le
ticket propose `exhausted`/`deadline_abort` ; `deadline_abort` est exclu par sa
propre documentation (`llm/mod.rs:147-150` : *« `elapsed_ms` is 0 here, and that
is not a measurement of a fast call — it is the absence of one »*), et la
tentative N a bel et bien tourné 300 s. Reste `exhausted`, dont la documentation
dit déjà *« the error is not retryable, **or the attempt budget is spent** »* —
un budget qui ne peut plus loger un appel avant la deadline est un budget épuisé.

### E4 — D2 n'est pas « 300 = 2 tentatives sans marge », c'est un off-by-one sur `E ≡ 0 (mod P)`

Posons `P` = plafond par appel, `E` = enveloppe du tour.

- `max_attempts = floor(E / P)` (`budget.rs`, mika#2189 AC3-b).
- Seuil **non-transport** = `typical_call (0.75 P) + retry_buffer (0.25 P)` = **exactement `1.0 P`**.
- Après une tentative consommant le plafond plein, `remaining = E − P − ε`.
- La garde abandonne si `remaining < seuil`, donc si `E − P − ε < P`, donc dès que `E ≤ 2P`.

| Géométrie | `max_attempts` | tentatives réelles (non-transport, plafond plein) |
|---|---|---|
| 120/300 (défaut flotte) | 2 | **2** — `300−120 = 180 > 120` ✓ |
| **300/600 (incident)** | 2 | **1** — `600−300 = 300`, pas `> 300` ✗ |
| 240/900 (mika-arch #2189) | 3 | **3** ✓ |
| 200/600 | 3 | **2** — la 3ᵉ est refusée |
| 420/600 | 1 | 1 (cohérent) |

**La règle générale : quand `E` est un multiple exact de `P`, la dernière
tentative nominale est inatteignable.** `floor(E/P)` compte des tentatives dans
une arithmétique à surcoût nul ; la garde mesure une horloge réelle. À `E = k·P`
les deux ne diffèrent que de `ε`, et `ε > 0` toujours. 300/600 n'est pas un cas
particulier de 300 — c'est le cas `k = 2` d'une famille.

### E5 — La classe transport n'est PAS affectée, et c'est ce qui interdit le remède évident

Seuil transport = `0.5 P` (mika#1744). À 300/600 : `remaining = 300 > 150` → **la
2ᵉ tentative tourne**. Donc :

- Le pire cas réel à 300/600 est bien de **2 × 300 s = 600 s**, l'enveloppe.
- Corriger D2 en faisant dire la vérité à `max_attempts` (le ramener à 1) **casserait
  le filet de mika#2342** : `worst_case_failure_secs = max_attempts × P` passerait de
  660 s à 360 s, et le filet couperait une chaîne transport parfaitement légitime
  à mi-course. **C'est un faux positif garanti, sur le mécanisme dont la valeur
  tient à ce qu'il ne fire jamais.**

Ce résultat est ce qui déplace D2 d'un correctif de valeur vers un correctif
d'observabilité — voir D3 ci-dessous.

### E6 — Ce que la trace fondatrice ne prouve PAS, et pourquoi ça ne bloque pas

Sur le rail OpenAI, un timeout `reqwest` (`send()` comme lecture de corps) est
mappé en `LlmError::Transport` (`openai.rs:233` via `From<reqwest::Error>`,
`openai.rs:292` pour la coupure de corps), donc classe
transport, donc seuil `0.5 P`, donc la 2ᵉ tentative **aurait dû** tourner. Les
18 ms disent qu'elle n'a pas tourné. Deux lectures restent ouvertes :

- **L1** — l'erreur n'était pas transport (HTTP 429/500/503 après 300 s d'attente
  en file chez OpenRouter) : seuil `1.0 P`, abandon, cohérent avec E4.
- **L2** — la deadline avait déjà moins de `0.5 P` = 150 s de marge quand l'appel
  a démarré : le tour avait consommé > 150 s avant (étape ≥ 2, appel d'outil,
  assemblage de prompt). La deadline est posée au **début du tour**, pas au début
  de l'appel.

**Aucune des deux ne change le plan** : E1 et E4 sont vrais indépendamment, et la
correction rend la trace auto-descriptive dans les deux cas — la ligne portera la
classe d'erreur, la marge restante et le seuil appliqué, donc le prochain
incident tranchera L1 contre L2 sans reconstruction. C'est d'ailleurs la mesure
que le correctif doit produire, pas une précondition à son écriture.

---

## Décisions

### D1 — Un seul prédicat, nommé, lu par les deux sites de chaque rail

`crates/mika-common/src/llm/retry_gate.rs`, fonction pure :

```rust
pub enum RetryVerdict { Retry, BudgetSpent, DeadlineInsufficient { remaining_ms: u64, threshold_secs: u64 } }

pub fn next_attempt_verdict(
    attempt: u32,
    max_attempts: u32,
    error: Option<&LlmError>,          // None => succès, jamais appelé
    remaining: Option<Duration>,       // None => pas de deadline
    transport_threshold_secs: u64,
    default_threshold_secs: u64,
) -> RetryVerdict
```

**Les seuils sont des paramètres, pas un `&LlmTimeoutBudget`** : `claude.rs` ne
porte pas de budget (il consomme `MAX_RETRIES` et ses littéraux, incohérence
nommée hors périmètre par mika#2189). Prendre un budget forcerait soit à en
fabriquer un faux sur ce rail, soit à laisser le rail Anthropic hors du prédicat
partagé — c'est-à-dire à conserver exactement la divergence qu'on ferme.

**`error: Option<&LlmError>` et non deux booléens** : `is_retryable` et
`is_transport` sont deux lectures de la même valeur, et les passer séparément
autorise l'appelant à les désaccorder (`retryable=false, transport=true` n'a pas
de référent). Le rail Anthropic, dont les erreurs sont des `ClaudeApiError`,
passe par un adaptateur local exposant les deux prédicats — une petite trait
`RetryClass { fn is_retryable(&self) -> bool; fn is_transport(&self) -> bool; }`
implémentée pour `LlmError` et pour `ClaudeApiError`, ce qui est déjà la forme
que `error_class` a dû prendre (deux *mappings*, un site de *définition*).

### D2 — `exhausted`, jamais `deadline_abort`, sur la tentative qui a tourné

`RetryVerdict::DeadlineInsufficient` sur la tentative N donne
`outcome = "exhausted"` — la valeur dont la documentation couvre déjà « le budget
de tentatives est épuisé ». `deadline_abort` reste réservé à la tentative N+1 qui
n'a pas eu lieu, avec son `elapsed_ms = 0` invariant.

**Le format de fil bouge, et c'est la correction, pas un effet de bord.** Des
lignes aujourd'hui comptées `retrying` deviendront `exhausted`. Un opérateur qui
`GROUP BY outcome` verra la population se déplacer au déploiement ; c'est
précisément le déplacement qui dit la vérité. La rupture est annoncée dans
`CLAUDE.md` et datée.

### D3 — Le flux de contrôle ne bouge pas : D1 est purement observabilité

La garde garde son `break`, la boucle garde son `continue`. On ne fait que
consulter le même prédicat au moment de nommer l'issue de la tentative N. Deux
raisons :

1. **Le comportement de retry est exactement celui d'aujourd'hui**, donc le
   correctif ne peut ni allonger ni raccourcir une chaîne, ni déplacer un pire
   cas, ni désaccorder le filet de mika#2342.
2. Fusionner les deux sites en une seule décision collapserait les deux lignes en
   une — et la ligne `deadline_abort` porte `deadline_remaining_ms`, qui est
   l'information qui tranche L1 contre L2 (E6). On perdrait la mesure en
   corrigeant l'étiquette.

Deux lignes, chacune vraie, chacune avec son sens documenté : `exhausted` sur la
tentative qui a tourné, `deadline_abort` sur celle qui n'a pas eu lieu.

### D4 — D2 : on mesure, on ne change aucune valeur

Bearing Prime, cité par le ticket : *ne PAS rallonger l'enveloppe totale ; le
vrai levier est de réduire la taille/durée du tour arch*. E5 ajoute une seconde
raison, technique celle-là : toucher `max_attempts` casse le filet mika#2342.
Donc **aucune géométrie ne bouge dans cette PR** — ni défaut de flotte, ni
mika-arch, ni les fractions de seuil.

Ce qui est ajouté :

- `LlmTimeoutBudget::effective_max_attempts()` — le compte honnête sous la garde
  non-transport, dérivé de la même arithmétique que E4 et testé sur les cinq
  géométries du tableau. **Il ne remplace pas `max_attempts`** : `max_attempts`
  reste ce qui borne la boucle et dimensionne le filet (E5), `effective_*` est ce
  qu'on rapporte.
- Deux champs sur `llm_budget_resolved` (mika#2293) :
  `effective_max_attempts` et `retry_reachable` (booléen, `effective == nominal`).
- Un WARN nommé `llm_budget_retry_unreachable`, émis au même site, quand
  `effective < max_attempts` — portant les deux comptes, la géométrie, la
  provenance de chaque moitié, et le levier (`(E mod P) == 0` ⇒ décaler `P` ou `E`
  hors du multiple exact).

**Pourquoi pas la garde de démarrage (`server::budget_guard`).** `E = k·P` est
une configuration *valide et fonctionnelle* : les appels tournent, la classe
transport retry. Refuser le démarrage coucherait la flotte sur un réglage
sous-optimal — le mode de panne exact que mika#2293 a dû nommer pour son propre
garde (deux agents muets, un troisième qui répond). L'instrument produit la
mesure ; la décision reste à Prime, avec ses données.

### D5 — Garde structurelle, parce qu'un test comportemental ne voit pas cette classe

`retry_gate::tests::mika2362_no_rail_recomputes_the_retry_threshold_inline` —
scan de source sur `crates/mika-common/src/` refusant, hors de `retry_gate.rs`,
toute occurrence de la forme `typical_call_duration_secs() + retry_buffer_secs()`
ou `TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS`. **Allowlist vide** : les
trois rails migrent, aucun n'est exempté ; un quatrième site est halt-and-surface.

Un test comportemental ne peut pas attraper la régression : réintroduire un
prédicat local ne rendrait aucune décision fausse, il rendrait les deux réponses
*libres de diverger à nouveau*, et toutes les assertions existantes resteraient
vertes pendant la dérive. Même raisonnement que
`grooming_marker::tests::no_grooming_regex_outside_this_module`.

---

## Volets d'implémentation

### V1 — `crates/mika-common/src/llm/retry_gate.rs` (nouveau)

`RetryVerdict`, `RetryClass`, `next_attempt_verdict`, plus l'impl `RetryClass` pour
`LlmError`. Fonction pure, zéro I/O, zéro horloge (le `remaining` est passé).
Déclaré `pub mod retry_gate;` dans `llm/mod.rs`.

`attempt_outcome::outcome_for(verdict)` traduit le verdict en la chaîne de fil
(`RETRYING` / `EXHAUSTED`), au même endroit que les constantes, pour que la
correspondance verdict ↔ format de fil ait un seul site.

### V2 — `crates/mika-common/src/llm/openai.rs`

- La garde (382-431) appelle `next_attempt_verdict` au lieu de recalculer le seuil.
  Sur `DeadlineInsufficient`, elle émet `deadline_abort` **inchangée** (mêmes
  champs, même `elapsed_ms = 0`) et `break`.
- Le calcul d'`outcome` (470-476) appelle le même prédicat avec le `remaining`
  qu'il vient de mesurer (`deadline_remaining_ms`, déjà calculé ligne 468).

### V3 — `crates/mika-common/src/llm/ollama.rs`

Identique à V2. Les deux constantes hoistées (`transport_threshold_secs`,
`default_threshold_secs`, lignes 595-597) deviennent les arguments du prédicat.

### V4 — `crates/mika-common/src/claude.rs`

Identique, avec l'impl `RetryClass for ClaudeApiError` (déléguant à `is_retryable`
et `is_transport_class`, déjà présents dans ce fichier). Les littéraux
`TYPICAL_CALL_DURATION_SECS + RETRY_BUFFER_SECS` et
`TRANSPORT_RETRY_MIN_REMAINING_SECS` sont passés en arguments plutôt que lus
inline — c'est ce qui rend la garde D5 applicable à ce rail sans le forcer dans le
système de budget.

Note : ce fichier porte un **second** calcul du même seuil ligne 811 (hors de la
boucle). Il entre dans le périmètre de la garde D5 et migre aussi.

### V5 — `crates/mika-common/src/llm/budget.rs`

`effective_max_attempts(&self, hard_cap: u32) -> u32`, avec sa docstring portant
le tableau d'E4 et la raison pour laquelle elle ne remplace pas `max_attempts`
(E5, le filet mika#2342).

### V6 — `crates/mika-common/src/llm/budget_provenance.rs`

Deux champs sur `llm_budget_resolved` + le WARN `llm_budget_retry_unreachable`.
La signature de déduplication (`log_llm_budget_resolved`, ligne 384) intègre
`effective_max_attempts` : sans quoi un changement de géométrie qui ne bouge que
l'atteignabilité serait tu.

### V7 — Documentation

- `crates/mika-common/CLAUDE.md` § providers : la divergence fermée, le prédicat
  unique, le glissement `retrying` → `exhausted` daté.
- `CLAUDE.md` racine § *Lire un hang LLM* : la table de décision de l'étape 3 gagne
  la ligne « `outcome = exhausted` avec `deadline_remaining_ms` ≈ plafond → la
  garde de deadline a refusé la 2ᵉ tentative ; lire `llm_budget_retry_unreachable`
  avant de toucher quoi que ce soit ». Plus le § budget pour les deux nouveaux
  champs et leur lecture.

---

## Verification contract

### T1 — D1, le test que le ticket demande (`crates/mika-common/tests/llm_retry.rs`)

Serveur HTTP local (harnais existant), budget `LlmTimeoutBudget::new(P, 2P)`,
deadline posée à `2P` de l'instant présent, première réponse = erreur **retryable
non-transport** (HTTP 429), consommant la quasi-totalité du plafond.

Capture `tracing` (forme de `crates/mika-agent/tests/llm_call_attempt_2342.rs`).
Assertions :

- la ligne `attempt = 0` porte `outcome = "exhausted"`, **jamais** `"retrying"` ;
- une ligne `attempt = 1` porte `outcome = "deadline_abort"` et `elapsed_ms = 0` ;
- le serveur a reçu **un** hit, pas deux.

**Contrôle positif dans le même test** (doctrine
`feedback_a_probe_needs_both_controls_in_the_same_call`) : même scénario sous la
géométrie flotte `120/300`, où la marge est réelle — `attempt = 0` doit porter
`retrying`, le serveur doit recevoir **deux** hits. Sans lui, un prédicat qui
répondrait toujours `exhausted` passerait.

### T2 — La classe transport n'a pas régressé (E5)

Même géométrie `P/2P`, première erreur = coupure de corps en vol (classe
transport, fixture déjà présente dans le fichier). Attendu :
`attempt = 0` → `retrying`, deux hits. C'est le contrôle négatif de D2 : le
correctif ne doit pas resserrer la chaîne transport, qui est ce sur quoi le filet
mika#2342 est dimensionné.

### T3 — Parité des trois rails

T1 et T2 rejoués sur `ollama` et `anthropic` via les fixtures existantes
(`ollama_provider`, `anthropic_request`). **Trois rails et non un** : un correctif
sur le seul rail mesuré laisserait deux copies libres de diverger, et l'absence de
ligne sur un rail non instrumenté se lit comme « ce rail n'a pas retryé ».

### T4 — `effective_max_attempts`, les cinq géométries d'E4

Test unitaire dans `budget.rs` asserant la colonne de droite du tableau, plus
l'invariant `effective_max_attempts(b, c) <= b.max_attempts(c)` sur les mêmes
jeux que `worst_case_failure_never_exceeds_the_envelope`.

### T5 — `worst_case_failure_secs` est inchangé (E5, le garde-fou)

Assertion explicite : pour les cinq géométries, `worst_case_failure_secs` rend
exactement la même valeur qu'avant la PR. Elle existe pour qu'une future
« simplification » remplaçant `max_attempts` par `effective_max_attempts` dans le
dimensionnement du filet rougisse au lieu de raccourcir silencieusement le filet
de mika#2342.

### T6 — Garde structurelle D5

Scan de source, allowlist vide.

### T7 — Le vocabulaire de fil est épinglé

`attempt_outcome::{SUCCESS, RETRYING, EXHAUSTED, DEADLINE_ABORT}` : test
d'égalité littérale, comme `mika2131_filter_names_are_a_wire_format`. Ces chaînes
atterrissent dans des `GROUP BY` opérateur.

---

## Fire-Disposition

### FD1 — T1 échoue avec `attempt = 0` → `retrying` **et** deux hits serveur

Alors l'erreur HTTP 429 est classée transport quelque part, ou le seuil
non-transport n'est pas `1.0 P`. **Halt.** Ne pas ajuster le test : l'arithmétique
d'E4 serait fausse et D2 entier repose dessus. Re-dériver avant toute correction.

### FD2 — T5 échoue

Le filet de mika#2342 a été re-dimensionné par inadvertance. **Halt** — c'est la
régression la plus coûteuse que cette PR puisse produire (faux positif sur un
mécanisme dont la valeur tient à son silence). Ne pas relâcher l'assertion.

### FD3 — `llm_budget_retry_unreachable` fire sur un agent de production au déploiement

**C'est le résultat attendu, pas une panne.** 300/600 est posé dans
`~/.mika/.env` selon le ticket. La ligne est la mesure que D4 existe pour
produire ; elle alimente la décision de Prime. Ne pas « corriger » la géométrie
par réflexe dans cette PR — voir la sonde post-déploiement.

### FD4 — La ligne ne fire sur aucun agent

Alors `E ≡ 0 (mod P)` n'est vrai nulle part en production et l'affirmation du
ticket sur le budget en vigueur est fausse. Lire `llm_budget_resolved` et son
`http_source` (mika#2293) **avant** de conclure : `process_env` à 300 confirmerait
le ticket, `agent_config` ou `default` le contredirait, et l'incident du 17/09
demanderait alors une autre explication (piste L2 d'E6).

---

## Definition of Done

- `retry_gate.rs` existe, est la seule définition du prédicat « une tentative
  suivante va-t-elle tourner ? », et les six sites des trois rails le lisent.
- Aucun rail ne recalcule un seuil de deadline inline ; la garde D5 passe avec une
  allowlist vide.
- `outcome` sur une tentative qui a tourné ne vaut `retrying` que si la tentative
  suivante peut effectivement tourner.
- Le flux de contrôle du retry est inchangé — `worst_case_failure_secs`,
  `max_attempts` et les fractions de seuil rendent les mêmes valeurs qu'avant.
- `llm_budget_resolved` porte `effective_max_attempts` et `retry_reachable` ;
  `llm_budget_retry_unreachable` est émis quand les deux comptes divergent.
- Aucune valeur de géométrie n'a bougé (ni défaut de flotte, ni per-agent).
- T1–T7 verts ; `cargo clippy` et `cargo fmt` propres.
- `CLAUDE.md` racine et `crates/mika-common/CLAUDE.md` à jour, glissement de
  vocabulaire daté, `scripts/sync-agent-docs.sh` si `docs/` est touché.

## Acceptance criteria

*(Le corps du ticket ne porte pas de section `## Acceptance criteria` ; ceux-ci
sont dérivés de sa section « Correctif attendu » et des volets ci-dessus.)*

- **AC1 (D1, exigé littéralement par le ticket)** — sous une enveloppe égale à
  `2 ×` le plafond, une tentative-0 consommant le plafond plein sur une erreur
  retryable non-transport n'émet **pas** `outcome="retrying"`. Le test correspondant
  est T1, avec son contrôle positif sous `120/300` dans le même appel.
- **AC2** — la valeur émise dans ce cas est `exhausted`, et `deadline_abort`
  conserve son invariant `elapsed_ms = 0` sur la tentative qui n'a pas eu lieu.
- **AC3** — le prédicat est écrit une fois et lu par les six sites ; une
  septième occurrence d'un calcul de seuil inline fait échouer la CI.
- **AC4** — la correction est observable sur les trois rails (T3), pas seulement
  sur celui où l'incident a été mesuré.
- **AC5 (D2)** — `llm_budget_resolved` expose le nombre de tentatives réellement
  atteignables, et une ligne nommée le dit quand il est inférieur au nominal.
  Aucune valeur de configuration n'est modifiée par cette PR.
- **AC6** — le pire cas déclaré du rail (`worst_case_failure_secs`), sur lequel le
  filet de mika#2342 est dimensionné, est **bit pour bit** celui d'avant la PR (T5).

---

## Surfaces opérateur et sonde post-déploiement

**Grep** (`$MIKA_SPIRIT_LOG_FILE`) :

```
grep llm_budget_resolved $MIKA_SPIRIT_LOG_FILE | \
  jq '{agent_id, http_timeout_secs, agent_total_timeout_secs, max_attempts, effective_max_attempts, retry_reachable, http_source, total_source}'
```

```
grep llm_budget_retry_unreachable $MIKA_SPIRIT_LOG_FILE
```

**Volume réel des retry, désormais non ambigu** :

```
grep llm_call_attempt $MIKA_SPIRIT_LOG_FILE | \
  jq 'select(.event == "llm_call_attempt" and .outcome != "success") |
      {provider, model, attempt, max_attempts, elapsed_ms, outcome, error_class, deadline_remaining_ms}'
```

Le garde `.event == "llm_call_attempt"` reste **nécessaire** (mika#2331) : le nom
est homonyme de la ligne de début de tentative de mika#2342 et apparaît dans le
corps des lignes DEBUG de capture de body.

**Sonde, 48 h.**

1. `llm_budget_retry_unreachable` doit nommer au moins un agent si le ticket dit
   vrai sur `300/600`. Sinon → FD4.
2. Sur les hangs coupés au plafond, la ligne `attempt = 0` doit désormais porter
   `exhausted` et non `retrying`. Une seule ligne `retrying` suivie d'un
   `deadline_abort` immédiat signifie que le prédicat partagé n'est pas lu par les
   deux sites → **halte**, le correctif n'a pas pris.
3. Le décompte `GROUP BY outcome` se déplace de `retrying` vers `exhausted` au
   déploiement. **C'est attendu et daté** ; une population `retrying` inchangée
   signifie soit qu'aucun hang n'est tombé dans la classe visée sur la fenêtre,
   soit que le correctif est inerte — départager avec la sonde 2.

---

## Hors périmètre (suivi à ouvrir)

- **Le réglage lui-même (D2, la décision).** Accepter `300/600`, réduire le brief
  arch (levier désigné par Prime, plus strict que mika#2330 / mika#2295), ou
  décaler la géométrie hors du multiple exact. Cette PR produit la mesure ; la
  décision est à Prime et se posera dans le `config.toml` per-agent, pas dans une
  variable fleet-wide (mika#2293).
- **Le littéral `120s` de `claude.rs` et son ignorance de `max_attempts`** —
  incohérence réelle, nommée hors périmètre par mika#2189, rendue seulement plus
  lisible ici.
- **L'aplatissement de `ClaudeApiError` en `LlmError::ProviderError`**
  (`llm/anthropic.rs`) — change la classe d'erreur vue par tout le moteur sur ce
  rail, donc un format de fil ; ticket propre (mika#2331 le nomme déjà).
- **La garde de deadline ignore le backoff exponentiel** qui suit son verdict
  (500 ms × 2^(n−1)) : elle mesure la marge *avant* le sleep. Marginal aux
  géométries en vigueur, pré-existant, non touché.
- **La cause fournisseur des coupures à 300 s** (OpenRouter / kimi) — ce travail
  rend la chaîne lisible, il ne la fait pas aboutir.
- **Le redrive épuisé en aval (#2361)** — ce correctif réduit la fréquence des
  groom échoués, il ne borne pas le budget de redrive.

---

## Revision history

- 2026-09-17 — rédaction initiale (dev-groom, content-only, revue architecte en aval).
