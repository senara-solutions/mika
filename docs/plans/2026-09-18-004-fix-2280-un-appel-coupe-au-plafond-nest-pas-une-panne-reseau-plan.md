# mika#2280 — un appel coupé au plafond n'est pas une panne réseau, et rien ne les distingue

- **Ticket :** senara-solutions/mika#2280
- **Priorité :** p1 (substrate — 32 coupures/jour mesurées, dont 7 tours morts sans réponse)
- **Branche :** `bug/2280/llm-openrouter-32-body-read-failed-mid`
- **Lignage :** mika#2015 (le corps coupé devient `Transport`, retryable), mika#1744 (seuil transport réduit), mika#1660 (`MIKA_LLM_HTTP_TIMEOUT_SECS`), mika#2189 (`LlmTimeoutBudget`, le couple plafond/enveloppe), mika#2293 (`llm_budget_resolved` + provenance), mika#2331 / mika#2342 (`llm_call_attempt`, le filet), mika#2362 (`retry_gate`, l'atteignabilité), mika#2296 (le plafond de sortie de mika-arch et **l'arithmétique qui décide ce ticket**)
- **Refs du ticket :** #2279 (bruit deferred), #2278 (dépendance tuée), #2270 (`mika ask` vide)

---

## Contexte

Le ticket mesure, le 2026-09-10 : **32 × `LLM response body read failed
mid-stream (retryable transport)`** sur `openrouter`, message
`error decoding response body: … operation timed out`, sur `z-ai/glm-5.3`
(mika-dev) et `moonshotai/kimi-k2.5` (mika-arch) ; **7 × `aborting retry chain —
remaining deadline insufficient`**, c'est-à-dire sept tours morts sans réponse,
dont des tours webhook `labeled` qui alimentent le bruit de #2279.

Le ticket pose trois pistes : (1) un timeout de lecture aligné sur `max_tokens`
ou du streaming, (2) un retry qui préserve le budget, (3) un repli fournisseur
mesuré. Il note aussi, correctement, que **ce n'est ni une auth ni un parse** :
les en-têtes arrivent, le corps ne finit pas.

La lecture du code déplace le diagnostic sur un point, et écarte deux des trois
pistes sur des éléments déjà écrits dans ce dépôt.

---

## Ce qui est établi, et comment le vérifier

### E1 — Le plafond borne la requête ENTIÈRE, corps compris, et c'est déjà documenté

`crates/mika-common/src/llm/openai.rs:184-187` construit **un seul** client :

```rust
let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(budget.http_timeout_secs()))
    .build()
```

`crates/mika-common/src/llm/budget.rs` (doc de module, mika#2189) le dit mot
pour mot, avec sa mesure :

> Sur les sept jours finissant le 2026-09-05, `llm_calls` porte **209 échecs**
> sous un seul message — `failed to read response body: … operation timed out` —
> dont la distribution de latence n'a pas de queue, seulement deux valeurs :
> **240 s** (171 occurrences) et **120 s** (37). Ce n'est pas de la variance
> fournisseur. C'est une guillotine client, franchie une ou deux fois.

Le symptôme de mika#2280 **est** celui de mika#2189, six jours plus tard, avec
le même message et le même mécanisme. mika#2189 a rendu la géométrie
*réglable* ; il n'a pas fait aboutir les générations longues.

### E2 — L'arithmétique qui décide ce ticket est déjà écrite, dans `well_known_agents.rs`

`crates/mika-agent/src/well_known_agents.rs:1446-1452` (mika#2296), à propos de
mika-arch :

> 32768 n'est pas une cible, c'est un plafond rendu non contraignant. Le débit
> mesuré est ~66 tok/s (8192 tokens en 123 s), donc le plafond de 240 s
> ci-dessous cape un appel à ~16 000 tokens : **le budget TEMPS est le vrai
> frein**, et il borne déjà l'emballement que cette valeur pourrait ouvrir.
> 16384 a été refusé pour exactement cette raison — il coïncide avec le maximum
> atteignable et redeviendrait contraignant au premier accroissement de vitesse
> ou élargissement de plafond.

Appliquée aux trois agents que le ticket mesure, avec les constantes du dépôt :

| agent | plafond | `llm_max_tokens` (source) | atteignable à 66 tok/s |
|---|---|---|---|
| mika-dev | 120 s (défaut flotte) | **8 192** (`MIKA_DEV_CONFIG:186`) | ≈ 7 900 |
| mika-qa | 120 s (défaut flotte) | **16 384** (`MIKA_QA_CONFIG:203`) | ≈ 7 900 |
| mika-arch | 240 s (mika#2189) | 32 768 (`well_known_agents.rs:1461`) | ≈ 15 800 |

mika-dev déclare un budget de sortie **qui coïncide avec son maximum
atteignable à 4 % près** — l'état exact que mika#2296 a refusé par écrit pour
mika-arch. mika-qa en déclare **le double**. Seul mika-arch a de la marge, et
c'est le seul dont le couple a été posé par un ticket qui a fait l'arithmétique.

« Les échecs sont sur des réponses longues » (ticket) n'est donc pas une
observation sur le fournisseur : c'est le frein temps qui mord, sur une requête
autorisée à demander plus que son plafond ne peut porter.

### E3 — Le plafond est une propriété du CLIENT, jamais de la requête

Conséquence directe de E1 : un appel d'outil de 200 tokens et une rédaction de
plan de 8 000 tokens courent sous la **même** guillotine. L'enveloppe (300 s)
porte une marge qu'un appel unique ne peut pas utiliser, parce que la borne est
figée à la construction du provider.

### E4 — La piste 2 du ticket est déjà réfutée par la procédure du dépôt

La table de décision de `mika/CLAUDE.md` § *Lire un hang LLM « 420 s sans
octet » (mika#2331)* :

> | `attempt` atteint `max_attempts`, chaque `elapsed_ms ≈ http_timeout_secs ×
> 1000` | La guillotine client se déclenche, le retry tourne **et ne sauve
> rien** | **Le retry n'est pas le remède.** |

Et les 7 × « deadline insufficient » ne sont pas un défaut de la porte de
deadline : à 120/300, `max_attempts = 2`, et la deadline est celle du **tour**,
partagée par tous ses pas. Un appel qui a consommé 120 s après que les pas
précédents en ont consommé 150 laisse 30 s, sous le seuil non-transport
(`1.0 × plafond = 120 s`). La porte fait exactement son travail. Réessayer avec
un créneau de 120 s un appel qui en demande 130 ne peut pas aboutir.

### E5 — La piste 3 est explicitement bornée par le ticket lui-même

« à ne tester que borné (un agent, une journée) et **sur décision opérateur** ».
Ce n'est pas du code à écrire ici.

### E6 — Le bras qui échoue est déjà structurellement étroit

`openai.rs:277-297` : l'erreur du ticket naît dans le bras `Err` de
`response.text().await`, **après** que le statut a été lu. Un échec de connexion
ou un timeout de connexion sortent par le `?` sur `.send()` et ne l'atteignent
jamais. Ce bras signifie donc déjà, exactement : *les en-têtes sont arrivés, le
corps n'a pas fini*. C'est le point d'ancrage naturel du discriminant.

**Mais « le bras » n'est pas « le seul appel » sur le rail ollama, et la nuance
décide de la justesse de l'attribution.** `ollama.rs` lit le corps à **deux**
endroits : `487`, dans la branche `!status.is_success()`, avec un
`unwrap_or_default()` qui avale l'erreur pour composer un message de diagnostic
HTTP ; et `521`, le `match` qui est le jumeau du bras d'`openai.rs`. Seul le
second est la population du ticket. Instrumenter le premier ferait entrer un
429 lent — dont le corps d'erreur arrive tard — dans une population qui affirme
« le modèle générait encore », c'est-à-dire produirait exactement l'attribution
fausse que D2 existe pour éviter.

### E7 — Rien ne distingue « le modèle générait encore » de « le réseau est mort »

`error_class()` (`llm/error.rs:122`) rend `transport_timeout` dans les deux cas.
`llm_calls` ne stocke **pas** `max_tokens` (`db.rs:3238`), `llm_call_attempt`
non plus (`llm/mod.rs:216`), et `llm_budget_resolved` ne porte que le couple
temps (`budget_provenance.rs:422`). Sur les 32 occurrences du ticket, **rien
dans le dépôt ne permet aujourd'hui de dire combien demandaient plus que leur
plafond ne pouvait porter.** C'est ce trou qui a laissé le ticket poser trois
pistes sans pouvoir en départager aucune.

### E8 — Le rail Anthropic ne lit pas le plafond, et c'est déjà nommé

`llm/mod.rs:426-433` : `claude.rs` code en dur un timeout reqwest de `120s` au
lieu de lire le plafond — inconsistance que mika#2189 nomme et laisse à son
propre ticket, que mika#2342 a rendue *déclarée* sans la corriger. Un
discriminant « écoulé ≈ plafond » y comparerait à un plafond que le rail
n'applique pas.

---

## Décisions

### D1 — Le livrable est l'attribution, pas le réglage

Trois faits l'imposent, et aucun n'est une préférence.

1. **Le remède structurel demande une mesure que ce dépôt n'a pas.** Le seul
   débit mesuré en dépôt (66 tok/s) l'est sur `kimi-k2.5`. Sur `z-ai/glm-5.3`,
   le modèle de la majorité des 32 occurrences, il n'y en a aucun. Dimensionner
   un plafond par requête sur le débit d'un autre modèle serait une constante
   inventée.
2. **La mesure du ticket est antérieure à quatre instruments.** Elle date du
   2026-09-10 ; `llm_call_attempt` (mika#2331/#2342), la provenance
   (mika#2293) et `retry_gate` (mika#2362) ont atterri les 17 et 18. Re-décider
   un remède sur des chiffres de huit jours quand les instruments pour
   re-mesurer existent est exactement le réflexe que la § *Lire un hang LLM* du
   `CLAUDE.md` borne par une halte.
3. **Le geste de configuration évident est un piège écrit dans le code.**
   `well_known_agents.rs:168-179` interdit de toucher `llm_max_tokens` dans
   `MIKA_DEV_CONFIG` : `reconcile_well_known_config` réécrit le fichier
   **entier**, et cette constante a dérivé de son runtime (elle déclare
   `z-ai/glm-5.2` quand mika#2179 et mika#2189 mesurent mika-dev sur
   `z-ai/glm-5.3`). Baisser le budget de sortie de mika-dev ici **rétrograderait
   le modèle en silence, sans calibration**. L'interdiction vaut dans les deux
   sens ; elle n'est pas écrite « ne pas augmenter » par hasard de vocabulaire,
   elle protège la réécriture, pas la direction.

Ce travail produit donc le nombre et l'attribution. Le remède est le ticket de
suivi de la § *Hors périmètre*, et il est **conditionné** à ce que ce nombre
dira.

### D2 — Le discriminant est l'écoulé contre le plafond, pas une nouvelle classe d'erreur

Un corps coupé à ≈ plafond est une guillotine ; un corps coupé à un instant
quelconque est une panne. Le premier terme est l'**écoulé**, mesuré dans
`send_once`, comparé au plafond effectif du provider. Le second, corroborant,
est la chaîne de cause disant `timed out` plutôt que `unexpected EOF` — la
distinction que `claude.rs:1569` relève déjà.

**Aucune variante de `LlmError` n'est ajoutée, et aucune classe d'erreur ne
bouge.** Deux raisons, toutes deux portantes :

- `error_class` est un **format de fil** : mika#2179 groupe
  `callback_delivery_failed` dessus. Scinder `transport_timeout` en deux
  couperait cette population en travers d'un déploiement.
- La rétryabilité ne doit pas bouger d'un iota. mika#2015 a fait passer ce bras
  de `ParseError` (non retryable) à `Transport` après avoir mesuré que 48 des 48
  erreurs de parse d'une heure étaient des corps coupés ; toucher la variante
  rouvre ce risque pour un besoin d'observabilité.

Le discriminant vit donc en **champ d'événement**, jamais en type. Précédent
exact : mika#2179 s'autorise une seule consultation de chaîne *à l'intérieur* de
`Transport` pour séparer un timeout d'une connexion refusée — ici la chaîne est
le terme secondaire, l'écoulé porte le poids.

### D3 — La tolérance est de 2 %, et elle est dérivée de la distribution mesurée

`elapsed_ms >= plafond_ms × 98 / 100`. La distribution de mika#2189 **n'a pas de
queue** : 171 occurrences à 240 s pile, 37 à 120 s pile. Les événements
s'agglutinent sur la valeur exacte, donc une tolérance serrée suffit — et elle
est le bon sens du risque : un faux positif enverrait l'opérateur relever un
plafond qui n'était pas en cause.

Le bras est déjà restreint aux corps coupés (E6), donc un timeout de connexion
ne peut pas entrer dans la population, quelle que soit la tolérance.

### D4 — La déclaration n'alarme pas ; seule une coupure réelle alarme

C'est la décision qui empêche ce travail de produire du bruit sur une
configuration décidée.

`llm_max_tokens > atteignable` **n'est pas un défaut en soi** : mika#2296 a
choisi 32768 pour mika-arch précisément comme « un plafond rendu non
contraignant », en sachant que le temps est le frein. Une garde statique
« déclaré > atteignable » firerait sur arch à chaque démarrage, contre une
décision prise et argumentée — et un avertissement qui contredit une décision
documentée est un avertissement qu'on coupe.

Donc : `reachable_output_tokens` est posé en **contexte INFO** sur
`llm_budget_resolved`, sans WARN ; le WARN (`llm_call_cap_exhausted`) naît d'un
**franchissement réel**, une fois par appel coupé. Ce qu'un opérateur veut
savoir n'est pas « ce réglage pourrait mordre » mais « ce réglage a mordu, N
fois, sur ce modèle ».

### D5 — `llm_max_tokens` passe par la cascade existante, comme troisième clé

`BudgetProvenance::resolve` (`budget_provenance.rs:210`) appelle
`layers.resolve_key(clé_config, var_env)`, qui est **entièrement générique** :
la troisième clé est une ligne. `CascadeLayers::read` lit déjà chaque fichier
une fois, donc le coût est nul.

Cela ferme au passage un trou réel de même nature que celui de mika#2293 : rien
aujourd'hui ne dit sous quel budget de sortie un agent tourne, **ni par quelle
porte il est arrivé**. C'est la seule chose qui pourra trancher si le runtime de
mika-dev porte encore 8192 ou une valeur relevée à la main comme celle de
mika-arch l'avait été (`well_known_agents.rs:1458-1462`) — question à laquelle
le checkout seul ne répond pas.

### D6 — La constante de débit est un plancher, et son biais est nommé

`DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR`, surchargeable par
`MIKA_LLM_OUTPUT_TOKENS_PER_SEC_FLOOR` (trois paliers maison : absent/vide →
défaut ; illisible, `0` ou négatif → défaut + WARN).

Valeur retenue : **50 tok/s**, soit le 66 tok/s mesuré en dépôt (mika#2296,
kimi-k2.5) avec ~25 % de marge vers le bas. La marge n'est pas de la prudence
décorative : **la population dont on dispose est censurée.** Les débits ne sont
observables que sur les appels *réussis*, et les lents ont précisément été tués
à 120 s — donc le plancher observé est lui-même une **surestimation** du vrai
plancher. Une constante posée sur la valeur observée décrirait un monde dont on
a retiré les cas qui nous intéressent.

La requête qui produit la distribution par modèle, à exécuter avant tout
réglage :

```sql
SELECT model,
       COUNT(*)                                    AS n,
       MIN(output_tokens * 1000.0 / latency_ms)    AS floor_tok_s,
       AVG(output_tokens * 1000.0 / latency_ms)    AS mean_tok_s
  FROM llm_calls
 WHERE status = 'success' AND latency_ms > 0 AND output_tokens > 500
 GROUP BY model
 ORDER BY n DESC;
```

Le nombre sert un **contexte de lecture**, jamais un frein : rien dans le chemin
de retry ni dans la construction du client ne le lit. Une constante grossière
est acceptable pour cela et ne l'est pas pour un plafond — ce qui est la seconde
raison pour laquelle le plafond par requête part en suivi (D1).

### D7 — Périmètre : les deux rails en forme OpenAI, pas Anthropic

`openai.rs` et `ollama.rs` honorent le budget avec lequel ils sont construits ;
`claude.rs` code `120s` en dur (E8), donc « écoulé ≈ plafond » y comparerait à
un plafond inappliqué et produirait une attribution fausse. La population du
ticket est à **100 %** OpenAI-compatible (openrouter). Le rail Anthropic reste
hors périmètre, avec la même raison que mika#2189 et mika#2342 lui ont déjà
donnée.

### D8 — La déduplication de `llm_budget_resolved` intègre les nouveaux champs

`dedup_signature` (`budget_provenance.rs:390`) doit inclure `llm_max_tokens` et
`reachable_output_tokens`. C'est la leçon littérale de mika#2362, qui a dû
ajouter `effective_max_attempts` à cette même signature : sans cela, un
changement de géométrie qui ne bouge *que* le champ neuf — c'est-à-dire
exactement ce que ce champ existe pour dire — serait tu comme « pas de
changement ».

---

## Changements

### 1. `crates/mika-common/src/llm/budget.rs`

- `PREFILL_RESERVE_NUM/DEN = 1/4` — part du plafond réservée à la connexion, à
  l'upload de la requête et au prefill, dans la forme fractionnaire que le
  module emploie déjà pour ses trois autres seuils (mika#2189 D3), donc suivant
  automatiquement tout plafond ultérieur.
- `DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR: u64 = 50` +
  `OUTPUT_TOKENS_PER_SEC_FLOOR_ENV_VAR`, avec la dérivation et le biais de
  censure en doc de module (D6).
- `LlmTimeoutBudget::reachable_output_tokens(&self, floor_tok_per_sec: u64) -> u64`
  = `cap_secs × (1 − 1/4) × floor`. Rendu **non validé et non appliqué** : la
  doc dit explicitement que rien dans le chemin de retry ne le lit.
- `output_tokens_per_sec_floor()` — lecteur d'environnement à trois paliers, non
  paniquant (même contrat que le lecteur de mika#2293, qui doit pouvoir être
  appelé depuis la garde de démarrage).

### 2. `crates/mika-common/src/llm/budget_provenance.rs`

- `MAX_TOKENS_CONFIG_KEY = "llm_max_tokens"`, `MAX_TOKENS_ENV_VAR =
  "MIKA_LLM_MAX_TOKENS"` ; `BudgetProvenance` gagne un champ `max_tokens:
  ResolvedBudgetValue`, résolu par le `resolve_key` existant (D5).
- `effective_max_tokens()` — miroir de `effective_budget()` : reprend le défaut
  compilé (`config.rs::default_max_tokens`) quand aucune porte ne porte la clé.
- `llm_budget_resolved` gagne quatre champs : `llm_max_tokens`,
  `max_tokens_source`, `max_tokens_raw`, `reachable_output_tokens`.
  **Aucun WARN n'est ajouté ici** (D4).
- `dedup_signature` intègre les deux nouvelles valeurs (D8).

### 3. `crates/mika-common/src/llm/mod.rs`

- `emit_llm_call_attempt` gagne `max_tokens: u32` et
  `cap_exhausted: Option<bool>`. Le second est un `Option` parce que
  `deadline_abort` décrit une tentative **qui n'a pas eu lieu** : y écrire
  `false` affirmerait qu'un appel n'a pas été coupé au plafond, alors qu'aucun
  appel n'a été fait. `null` n'est jamais `false`, comme `request_bytes` de
  mika#2342 n'est jamais `0`.
- La macro interne s'étend aux nouvelles combinaisons d'`Option`.
- **Les sites d'appel sont six, sur les trois rails, et `claude.rs` en porte
  deux** — constatés plutôt que supposés :
  `openai.rs:428` / `:493`, `ollama.rs:628` / `:682`, **`claude.rs:656` /
  `:725`**. Élargir la signature est donc une mise à jour de six sites, pas de
  deux. Sur les six, `request.max_tokens` est un `u32` non-optionnel déjà en
  portée et déjà journalisé (`claude.rs:596`, `ollama.rs:602` / `:763`), donc
  aucune question d'absence à trancher nulle part.
- **`cap_exhausted` vaut `None` aux deux sites d'`claude.rs`, toujours**, et
  c'est la conséquence directe de D7 : ce rail n'applique pas le plafond qu'il
  déclare (E8), donc y poser `false` affirmerait qu'un appel n'a pas été
  guillotiné par une borne qui ne le gouvernait pas — l'attribution fausse que
  D7 existe pour refuser. Le rail passe le `max_tokens` (une déclaration, vraie
  partout) sans passer le drapeau (un jugement, qui demande un plafond
  appliqué).
- **Ce que cela coûte à la lecture, nommé plutôt que découvert :** `cap_exhausted`
  absent a désormais **deux** causes — la tentative n'a pas eu lieu
  (`deadline_abort`, tous rails) ou le rail n'est pas instrumenté (Anthropic,
  toutes tentatives). `select(.cap_exhausted == true)` reste exact ; c'est
  **compter les `false`** qui ne rend que la population OpenAI-compatible. Le
  champ `provider`, déjà porté par l'événement, tranche les deux cas — la
  § *Surfaces opérateur* le dit.

### 4. `crates/mika-common/src/llm/openai.rs` et `.../ollama.rs`

- `send_once` mesure son propre `Instant` d'entrée et applique le discriminant
  de D2/D3 dans le bras d'erreur du `match response.text()` — `openai.rs:277`,
  `ollama.rs:521`. **Pas `ollama.rs:487`** : ce site-là lit le corps d'une
  réponse non-2xx et son erreur est déjà avalée par `unwrap_or_default()` (E6).
  Sur franchissement : `warn!(event = "llm_call_cap_exhausted", …)` portant
  `provider`, `model`, `max_tokens`, `http_timeout_secs`, `elapsed_ms`,
  `reachable_output_tokens`.
- `send_once` rend l'information au boucleur **par sa signature**, pas par un
  `Cell` ni un champ partagé : elle est privée et n'a qu'**un seul appelant**
  (`openai.rs:474`, `let attempt_result = self.send_once(…)`), donc
  `Result<OpenAiResponse, (LlmError, bool)>` se propage en un site. Un état
  latéral coûterait la même écriture en rendant le drapeau atteignable depuis
  ailleurs. La forme retenue ne doit **pas** modifier `LlmError` (D2).
- Les quatre appels à `emit_llm_call_attempt` de ces deux rails (`openai.rs:428`
  et `:493`, `ollama.rs:628` et `:682`) passent `request.max_tokens` — déjà un
  `u32` non-optionnel sur `OpenAiRequest:24`, donc aucune question d'absence à
  trancher — et le drapeau (`None` sur les deux sites `deadline_abort`). Les
  deux sites restants sont ceux d'`claude.rs`, traités au § 3 ci-dessus.
- Le passage de 9 à 11 paramètres ne rougit pas clippy : `mod.rs:215` porte déjà
  `#[allow(clippy::too_many_arguments)]`. Noté parce que la DoD exige
  `-D warnings` et que la question se pose sinon à l'implémentation.

### 5. `mika/CLAUDE.md`

Un bloc § *Lire une coupure au plafond (mika#2280)* sous la section
`llm_budget_resolved` de mika#2293 : les greps, la requête SQL, le régime
attendu, et la **halte** de la § *Sonde post-déploiement* ci-dessous. Plus les
quatre nouveaux champs de `llm_budget_resolved` et la variable
`MIKA_LLM_OUTPUT_TOKENS_PER_SEC_FLOOR` dans la liste des variables optionnelles.

### 6. `crates/mika-agent/src/well_known_agents.rs` — **le test d'AC8, et rien d'autre**

Ce fichier apparaît dans les Changements uniquement pour lever une ambiguïté que
son absence créerait : il reçoit le test de l'AC8 (module `tests`, voir la § Tests
pour la contrainte de dépendance qui l'y oblige) et **aucune de ses constantes
n'est touchée**. `MIKA_DEV_CONFIG`, `MIKA_QA_CONFIG` et la config de mika-arch
sont lues, jamais écrites — l'interdiction de D1.3 et d'AC9 porte sur les
valeurs, pas sur le fichier.

---

## Tests

- `budget::tests` — `reachable_output_tokens` reproduit l'arithmétique de
  mika#2296 sur ses propres chiffres (240 s, 66 tok/s → ~11 900 après réserve de
  prefill ; la valeur brute sans réserve, ~15 800, est celle que le commentaire
  cite — le test asserte la formule et **documente l'écart**, il ne prétend pas
  reproduire un nombre arrondi à la main).
- `budget::tests` — les trois paliers du lecteur d'environnement.
- **`well_known_agents::tests::mika2280_les_trois_geometries_livrees_et_leur_verdict`**
  — la constatation de E2, figée : mika-dev (120 s / 8 192) et mika-qa
  (120 s / 16 384) sont au-dessus de leur atteignable, mika-arch (240 s / 32 768)
  aussi **et c'est décidé** (mika#2296). Le test n'exige aucune correction ; il
  rougit le jour où quelqu'un déplace un de ces six nombres sans refaire
  l'arithmétique. C'est la garde que E2 appelle : elle ne peut pas être
  comportementale, parce qu'aucune décision ne deviendrait fausse — seule la
  cohérence entre deux constantes le deviendrait.

  **L'emplacement est contraint, pas préféré.** `mika-agent` dépend de
  `mika-common` et jamais l'inverse (`crates/mika-common/Cargo.toml` ne porte
  aucune dépendance vers `mika-agent`), donc un test vivant dans
  `budget_provenance::tests` **ne peut pas** lire `MIKA_DEV_CONFIG`,
  `MIKA_QA_CONFIG` ni `MIKA_ARCH_CONFIG`, qui sont des constantes de
  `crates/mika-agent/src/well_known_agents.rs`. Le sens légal est l'autre : le
  test vit dans `mika-agent` et appelle `reachable_output_tokens` depuis
  `mika-common`. Il se place à côté de
  `mika2296_no_well_known_config_declares_an_output_budget_below_8192`, dont il
  reprend la forme exacte (scan de `WELL_KNOWN_AGENTS`, lecture du `config_toml`
  en TOML, `continue` sur une absence de déclaration — une absence est un
  non-choix, jamais une violation) et dont il est le **complément
  d'orientation** : cette garde-là pose un *plancher* (`declared >= 8192`),
  celui-ci lit le *rapport* entre le budget déclaré et ce que le plafond temps
  peut porter. Les deux cohabitent sans se recouvrir.
- `mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`
  étendu à la troisième clé, sur les quatre positions.
- `budget_provenance::tests` — une géométrie qui ne change *que* `llm_max_tokens`
  ré-émet `llm_budget_resolved` (D8) ; une répétition à l'identique reste tue.
- `openai.rs` tests — sur un serveur local qui rend des en-têtes puis se tait :
  (a) une coupure à ≈ plafond porte `cap_exhausted = true` ; (b) une coupure
  précoce (corps tronqué immédiatement) porte `false` ; (c) dans les deux cas la
  variante d'erreur reste `Transport` et `is_retryable()` reste `true` — la
  non-régression de mika#2015, assertée plutôt qu'espérée.
- `ollama.rs` tests — **contrôle négatif de site** : une réponse non-2xx dont le
  corps d'erreur n'arrive qu'après le plafond n'émet aucun
  `llm_call_cap_exhausted` (AC5). Sans lui, instrumenter le mauvais des deux
  sites `response.text()` laisserait tous les autres tests verts tout en
  gonflant la population que la sonde (c) doit lire — une attribution fausse ne
  rend aucune décision incorrecte, elle rend la mesure menteuse.
- `llm/mod.rs` tests — `deadline_abort` porte `cap_exhausted` absent, jamais
  `false`.

---

## Surfaces opérateur

**Journal** (`$MIKA_SPIRIT_LOG_FILE`) :

```bash
# Combien de coupures sont des guillotines, et sur quel modèle
grep llm_call_cap_exhausted $MIKA_SPIRIT_LOG_FILE \
  | jq '{provider, model, max_tokens, http_timeout_secs, elapsed_ms, reachable_output_tokens}'

# Le budget de sortie réellement en vigueur, et par quelle porte
grep llm_budget_resolved $MIKA_SPIRIT_LOG_FILE \
  | jq '{agent_id, llm_max_tokens, max_tokens_source, reachable_output_tokens, http_timeout_secs, http_source}'

# Les tentatives, avec ce qu'elles demandaient (mika#2331 étendu)
grep llm_call_attempt $MIKA_SPIRIT_LOG_FILE \
  | jq 'select(.event == "llm_call_attempt" and .cap_exhausted == true)'
```

**Lire un `cap_exhausted` absent : deux causes, et `provider` les sépare.** Le
champ manque quand la tentative n'a pas eu lieu (`deadline_abort`, tous rails) et
quand le rail n'est pas instrumenté (Anthropic, **toutes** ses tentatives — D7 /
E8). La requête ci-dessus est donc exacte, mais **compter les `false` ne rend que
la population OpenAI-compatible** ; pour la borner explicitement, ajouter
`.provider != "anthropic"` au filtre plutôt que conclure d'une absence.

**Régime attendu de `llm_call_cap_exhausted` : NON VIDE.** C'est l'objet de ce
travail — la ligne n'existe que pour compter une population dont le ticket
affirme qu'elle vaut ~32/jour. Un volume conforme confirme le diagnostic et
arme le ticket de suivi ; **zéro ligne pendant que les timeouts continuent est
une halte** (voir sonde b).

**Régime attendu de `llm_budget_resolved` :** une ligne par agent et par couple
résolu, comme aujourd'hui. `max_tokens_source` discrimine les trois mondes de
mika#2293 (H1 `agent_config` / H2 `process_env` / H3 `default`) appliqués à la
troisième clé — et c'est la seule chose qui dira si le runtime de mika-dev porte
encore la valeur de la constante ou une valeur relevée à la main (D5).

**SQL.** Pas d'`audit_events` : `emit_llm_call_attempt` vit dans `mika-common`,
qui n'a pas d'accès base ; y plomber un writer serait une dépendance à sens
unique pour un compteur. La surface SQL existante reste `llm_calls`, requêtée
avec le plafond en paramètre — limitation nommée plutôt que découverte :

```sql
SELECT model, COUNT(*) FROM llm_calls
 WHERE status = 'error' AND latency_ms >= 120000 * 98 / 100   -- plafond de l'agent
 GROUP BY 1 ORDER BY 2 DESC;
```

---

## Sonde post-déploiement, et ses haltes

**(a) Lire la provenance AVANT toute conclusion.** `max_tokens_source` et
`http_source` pour mika-dev et mika-qa. Un `process_env` sur l'une des deux clés
signifie qu'une variable de service écrase le `config.toml`, et **le remède est
de la retirer de l'environnement**, pas de toucher une constante (mika#2293 H2).

**(b) Contrôle négatif, 48 h — la halte principale.** Si
`llm_call_cap_exhausted` est **vide** alors que
`LLM response body read failed mid-stream` continue d'apparaître : **halte.** La
coupure n'est pas au plafond, donc le diagnostic de E2 est faux, et le ticket de
suivi sur le plafond par requête **ne doit pas être ouvert**. La piste devient
celle de mika#2313/#2317 (le relais, la fin de corps) ou le fournisseur. Ne pas
élargir la tolérance de D3 pour faire apparaître des lignes : c'est la seule
manière de transformer cet instrument en menteur.

**(c) Attribution, 7 jours.** Distribution de `llm_call_cap_exhausted` par
modèle. Dominée par un modèle → le suivi est par modèle. Étalée → le suivi est
géométrique (couple plafond/enveloppe). Cette distribution **est** la décision
du ticket de suivi ; elle n'est pas disponible aujourd'hui, et c'est pourquoi ce
travail la produit d'abord.

**(d) Débit réel.** Exécuter la requête de D6 après 7 jours et comparer le
plancher observé par modèle à la constante de 50 tok/s. Un plancher observé
**sous** 50 signifie que la réserve de 25 % ne suffit pas et que
`reachable_output_tokens` est optimiste — corriger la constante, pas la formule.
Se rappeler que ce plancher observé reste censuré vers le haut (D6).

**(e) Non-régression.** `grep llm_call_attempt … | jq 'select(.outcome ==
"retrying")'` sur 48 h : le volume et la forme de la chaîne de retry ne doivent
pas bouger. Ce travail n'y touche pas ; s'ils bougent, c'est le drapeau qui a
fuité dans la rétryabilité et il faut désarmer (mika#2015).

---

## Hors périmètre, délibérément

- **Le plafond par requête — le remède, et c'est un ticket de suivi conçu, pas
  différé dans le vague.** Forme : `RequestBuilder::timeout()` (reqwest 0.12
  l'accepte par requête et surcharge le défaut du client), plafond effectif
  `clamp(plafond_configuré, max_tokens / plancher + réserve, enveloppe −
  marge)`, **jamais sous le plafond configuré** — donc strictement non
  régressif. Deux conséquences à traiter dans ce ticket-là, nommées ici pour
  qu'il ne les redécouvre pas : (i) `RetryThresholds::from_budget` doit lire le
  plafond **effectif**, sans quoi la porte de deadline juge un appel de 250 s
  avec les seuils d'un appel de 120 s ; (ii) `max_attempts × plafond ≤
  enveloppe` cesse d'être vrai *par construction* (mika#2189) et redevient tenu
  *par l'horloge* (la porte de deadline) — c'est un affaiblissement d'un
  invariant portant, à argumenter, pas à glisser. (iii) `worst_case_failure_secs`
  dimensionne le filet de mika#2342 et ne prend pas la requête : il faut un
  `worst_case_failure_secs_for(max_tokens)` à défaut de trait, sous peine d'un
  filet plus court que la chaîne qu'il borne. **Ouverture conditionnée aux
  sondes (b) et (c).**
- **Le streaming** — c'est le remède *structurel* : un corps qui coule rend la
  durée de génération non-bloquante et permet un `read_timeout` inter-octets de
  quelques secondes au lieu d'un plafond total de plusieurs minutes. La
  fonctionnalité reqwest `stream` est déjà activée (`Cargo.toml:22`). Mais il
  faut accumuler les deltas SSE pour le contenu, les `tool_calls` fragmentés par
  index, `reasoning_content`, `finish_reason` et l'`usage`
  (`stream_options.include_usage`), sur un rail partagé par treize
  fournisseurs dont les divergences ne sont pas mesurées. C'est un ticket entier,
  et l'ouvrir avant la sonde (c) serait le dimensionner à l'aveugle.
- **Le repli fournisseur (piste 3)** — borné par le ticket lui-même à une
  décision opérateur (E5).
- **Toute modification de valeur** : `llm_max_tokens`, plafonds, enveloppes.
  Interdit par D1.3 pour les constantes de `well_known_agents.rs` (la réécriture
  totale rétrograderait le modèle), et par la doctrine de mika#2293 pour le
  reste (« ce travail ne change aucune valeur »).
- **Le rail Anthropic** et son `120s` en dur (E8/D7), déjà porté par son propre
  ticket depuis mika#2189.
- **Une garde de démarrage** sur `llm_max_tokens > atteignable` : refusée par D4
  (elle alarmerait sur la configuration décidée de mika-arch) et par la règle que
  mika#2293 a dû énoncer pour sa propre garde — coucher la flotte sur un réglage
  sous-optimal mais fonctionnel est le mode de panne qu'on répare, pas celui
  qu'on crée.
- **#2279 / #2278 / #2270**, référencés par le ticket comme conséquences. Ce
  travail rend leur cause commune attribuable ; il ne les ferme pas.

---

## Fire-Disposition

Ce plan porte trois livrables de classe détecteur dont le firing peut porter sur
des **données existantes** (mika#1574). Chacun reçoit sa disposition ; les autres
détecteurs du plan sont inventoriés et écartés en clôture.

### FD1 — AC8, la garde de géométrie : **option (a), allowlist nommée**

Le détecteur fige les trois géométries livrées et leur verdict d'atteignabilité.
Les trois sont **déjà** au-dessus de leur atteignable au moment où le test est
écrit : ce sont trois violations préexistantes de l'invariant naïf « déclaré ≤
atteignable », et le test doit les porter nommément plutôt que les interdire —
sinon il rougit au premier `cargo test` contre une configuration de production
qui n'a rien fait de mal.

| donnée nommée (1) | pourquoi elle est exceptée | suivi (2) |
|---|---|---|
| mika-arch, 240 s / 32 768 (`well_known_agents.rs:1461`) | **décidé et argumenté** : mika#2296 pose 32768 comme « un plafond rendu non contraignant », en sachant que le temps est le frein. Rien à corriger. | aucun — l'exception est terminale |
| mika-dev, 120 s / 8 192 (`MIKA_DEV_CONFIG:186`) | à 4 % de son atteignable ; c'est la mesure que ce ticket produit, pas un défaut qu'il tranche (D1) | conditionné aux sondes (b)/(c) |
| mika-qa, 120 s / 16 384 (`MIKA_QA_CONFIG:203`) | le double de son atteignable ; idem | conditionné aux sondes (b)/(c) |

**L'assertion auto-nettoyante (3) est la forme même du test** : il asserte les
six nombres et leur verdict, donc il rougit le jour où l'un d'eux bouge. Quand le
suivi baissera `llm_max_tokens` de mika-dev sous son atteignable, la ligne
d'allowlist correspondante devient périmée **et le test le dit**, en exigeant que
l'arithmétique soit refaite plutôt qu'en passant en silence. C'est la propriété
que la clause (3) demande, obtenue par l'orientation du test (il fige un rapport)
et non par un mécanisme d'expiration ajouté à côté.

**Ce que la clause (2) ne peut pas avoir ici, dit plutôt que simulé :** le ticket
de suivi **n'existe pas encore**, parce que son ouverture est conditionnée aux
sondes (b) et (c) par D1 — ouvrir un tracker maintenant serait décider par avance
ce que la mesure doit décider. La référence portée par les deux lignes est donc
**mika#2280 lui-même**, et la § *Hors périmètre* y décrit la forme du suivi. Le
remplacement de cette référence par le numéro réel est le premier geste du suivi
le jour où il s'ouvre. Une référence morte vers un ticket inventé coûterait plus
que cette honnêteté.

### FD2 — `llm_call_cap_exhausted`, le détecteur runtime : **option (a), population nommée**

Il fire sur données existantes **dès le déploiement**, et c'est l'objet du
travail : la population est nommée — les **~32 coupures/jour** mesurées le
2026-09-10 sur `z-ai/glm-5.3` et `moonshotai/kimi-k2.5` via openrouter (1). Le
détecteur **ne gate rien** : pas de refus de démarrage, pas de refus d'appel, pas
de changement de rétryabilité (AC6). Il rend lisible une condition qui existait
déjà et que `error_class` ne pouvait pas distinguer (E7) ; son taux de firing est
l'**entrée** du suivi (2, même référence conditionnée qu'en FD1), jamais une
alarme qui halte.

**La clause (3) n'est pas assertable sur ce détecteur, et c'est une limite de
nature, pas un oubli.** Un WARN runtime n'a pas de site où poser une assertion
qui échoue quand l'exception devient périmée : le jour où le suivi corrige les
plafonds, la population tombera à zéro et **rien ne le dira**, parce que zéro
ligne est aussi ce que produit un détecteur cassé. Ce qui tient ce rôle est
daté et déjà écrit : la sonde **(b)** (48 h, contrôle négatif — zéro ligne
pendant que `LLM response body read failed mid-stream` continue ⇒ **halte**, le
diagnostic d'E2 est faux, le suivi ne s'ouvre pas) et la sonde **(c)** (7 jours,
distribution par modèle). La règle explicite de (b) — *ne pas élargir la
tolérance de D3 pour faire apparaître des lignes* — est ce qui empêche
l'auto-nettoyage manquant de devenir un instrument menteur.

### FD3 — AC3, l'extension de la garde de reconstruction : **option (c), halt-and-surface**

`mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`
existe déjà et passe sur deux clés ; l'étendre à `llm_max_tokens` peut le faire
rougir sur l'**ordre de cascade en vigueur**, qui est une donnée existante que ce
plan ne crée pas. Si cela arrive, l'implémentation **s'arrête et remonte à
l'opérateur** : ni allowlist, ni `#[ignore]`. La raison est celle que mika#2293 a
dû énoncer pour sa propre garde — une provenance **fausse** est strictement pire
qu'aucune provenance —, et un rouge ici signifierait que `max_tokens_source`
mentirait sur la porte d'où la valeur vient, c'est-à-dire que la sonde (a), qui
existe pour choisir entre « retirer une variable de service » et « ne toucher à
rien », enverrait l'opérateur au mauvais remède. La résolution de cette
divergence **est** la décision de portée, ce qui est exactement le critère
d'emploi de l'option (c).

### Clôture de l'inventaire

Les autres détecteurs du plan — tests d'`openai.rs` / `ollama.rs` (AC5/AC6), le
contrôle négatif de site sur `ollama.rs:487`, le test de déduplication (AC4/D8),
le test de `deadline_abort` (AC7) — portent **uniquement sur du code introduit
par cette PR**. Ils n'ont pas de données préexistantes sur lesquelles firer, donc
la gate est N/A pour eux (décision 3 de l'arbre mika#1574). Ils sont couverts par
la § *Tests*, pas ici.

---

## Definition of Done

- `reachable_output_tokens` est calculable depuis un `LlmTimeoutBudget` et un
  plancher de débit, avec sa réserve de prefill en fraction du plafond.
- `llm_max_tokens` est résolu par la cascade de mika#2293 et porté, avec sa
  provenance, sur `llm_budget_resolved` ; la déduplication en tient compte.
- Une coupure de corps à ≈ plafond est distinguée dans le journal d'une panne
  réseau, sur les deux rails en forme OpenAI, sans qu'aucune variante de
  `LlmError`, aucune classe d'erreur ni aucune décision de rétryabilité ne
  change.
- `llm_call_attempt` porte ce que l'appel demandait (`max_tokens`) et s'il a été
  guillotiné (`cap_exhausted`, absent quand aucun appel n'a eu lieu).
- Aucune valeur de configuration n'est modifiée.
- `cargo test -p mika-common` **et `cargo test -p mika-agent`**,
  `cargo clippy --all-targets -- -D warnings` et `cargo fmt --all --check`
  passent. Les deux crates, pas un : le test d'AC8 vit dans `mika-agent`
  (contrainte de dépendance, cf. § Tests), donc un `-p mika-common` seul
  sauterait la garde de géométrie — exactement le détecteur que ce plan ajoute.
  `cargo test --workspace` convient aussi ; c'est la couverture qui est exigée,
  pas la forme de l'invocation.
- `mika/CLAUDE.md` porte les greps, la requête SQL, le régime attendu et les
  haltes.

## Acceptance criteria

- **AC1** — `LlmTimeoutBudget::reachable_output_tokens(floor)` existe, dérive de
  `http_timeout_secs` par une fraction (jamais un littéral en secondes), et est
  couvert par un test reproduisant l'arithmétique de mika#2296 sur ses propres
  chiffres.
- **AC2** — `DEFAULT_OUTPUT_TOKENS_PER_SEC_FLOOR` vaut 50, est surchargeable par
  `MIKA_LLM_OUTPUT_TOKENS_PER_SEC_FLOOR` selon les trois paliers maison, et sa
  doc de module énonce la mesure d'origine (66 tok/s, mika#2296) **et** le biais
  de censure qui justifie la marge.
- **AC3** — `BudgetProvenance` résout `llm_max_tokens` /
  `MIKA_LLM_MAX_TOKENS` par `resolve_key`, et
  `mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`
  couvre la troisième clé sur les quatre positions de la cascade.
- **AC4** — `llm_budget_resolved` porte `llm_max_tokens`, `max_tokens_source`,
  `max_tokens_raw` et `reachable_output_tokens` ; **aucun WARN n'est ajouté à ce
  site** ; un test montre qu'une géométrie ne changeant que `llm_max_tokens`
  ré-émet la ligne, et qu'une répétition à l'identique reste tue.
- **AC5** — dans `openai.rs` et `ollama.rs`, une coupure de corps dont l'écoulé
  atteint 98 % du plafond émet `llm_call_cap_exhausted` (WARN) portant
  `provider`, `model`, `max_tokens`, `http_timeout_secs`, `elapsed_ms` et
  `reachable_output_tokens` ; une coupure précoce n'en émet pas. L'émission vit
  **uniquement** dans le bras d'erreur du `match response.text()` ; une réponse
  non-2xx dont le corps d'erreur arrive tard (`ollama.rs:487`) n'émet rien —
  assertée par un test, sans quoi un 429 lent serait compté comme guillotine.
- **AC6** — dans les deux cas d'AC5, l'erreur rendue reste
  `LlmError::Transport`, `is_retryable()` reste `true` et `error_class()` reste
  `transport_timeout` — assertés, pas supposés (non-régression mika#2015 /
  mika#2179).
- **AC7** — `emit_llm_call_attempt` porte `max_tokens` et un `cap_exhausted`
  optionnel ; sur le site `deadline_abort` le champ est **absent**, jamais
  `false`.
- **AC8** — un test fige les trois géométries livrées (mika-dev 120/8192,
  mika-qa 120/16384, mika-arch 240/32768) avec leur verdict d'atteignabilité, et
  rougit si l'un de ces six nombres bouge sans que l'arithmétique soit refaite.
  Il vit dans `crates/mika-agent/src/well_known_agents.rs` (module `tests`), le
  seul crate d'où les trois constantes **et** `reachable_output_tokens` sont
  simultanément visibles — `mika-common` ne voit pas `mika-agent`. Les trois
  lignes sont portées comme l'allowlist nommée de **FD1** (donnée, motif,
  référence de suivi), et le test **n'exige aucune correction** sur aucune des
  trois.
- **AC9** — aucune constante de configuration n'est modifiée : ni
  `MIKA_DEV_CONFIG`, ni `MIKA_QA_CONFIG`, ni la config de mika-arch, ni
  `DEFAULT_HTTP_TIMEOUT_SECS`, ni `DEFAULT_AGENT_TOTAL_TIMEOUT_SECS`.
- **AC10** — `mika/CLAUDE.md` documente les trois greps, la requête SQL, le
  régime attendu non vide de `llm_call_cap_exhausted`, la variable
  d'environnement, et la halte du contrôle négatif (b) — « zéro ligne pendant
  que les timeouts continuent : le diagnostic est faux, ne pas ouvrir le
  suivi ».

---

## Revision history

- **rev 2 (2026-09-18)** — première passe architecte (`Disposition: ITERATE`),
  trois findings, tous adressés.
  - **F1 (BLOCKING)** adressé par une section `## Fire-Disposition` à trois
    entrées, chacune nommant son option canonique plutôt qu'une disposition
    unique plaquée sur trois détecteurs de natures différentes. **FD1** (AC8,
    garde de géométrie) prend l'**option (a)** suggérée par le finding : les
    trois géométries livrées sont portées comme allowlist nommée (donnée, motif,
    référence), et l'auto-nettoyage de la clause (3) est la forme même du test —
    il fige un *rapport*, donc il rougit quand le suivi corrige un des six
    nombres. **FD2** (`llm_call_cap_exhausted`) prend aussi l'**option (a)** avec
    la population préexistante nommée (~32/jour, 2026-09-10), en disant que la
    clause (3) **n'est pas assertable** sur un WARN runtime et que ce rôle est
    tenu par les sondes datées (b) et (c) — limite écrite plutôt que simulée par
    un mécanisme décoratif. **FD3** est un détecteur que le finding n'avait pas
    inventorié (AC3, l'extension de
    `mika2293_reconstruction_equals_load_for_agent_on_every_cascade_position`,
    qui peut rougir sur l'ordre de cascade **existant**) : il prend l'**option
    (c) halt-and-surface**, parce qu'une provenance fausse est « strictement pire
    qu'aucune provenance » (mika#2293) et que sa résolution serait elle-même la
    décision de portée. Clôture d'inventaire : les quatre autres détecteurs du
    plan ne portent que sur du code neuf, gate N/A (arbre mika#1574, décision 3).
    AC8 renvoie désormais à FD1 pour la forme de l'allowlist.
  - **F2** adressé : la DoD exige `cargo test -p mika-agent` en plus de
    `cargo test -p mika-common` (ou `--workspace`), avec la raison — c'est la
    couverture qui est exigée, pas la forme de l'invocation.
  - **F3** adressé, et la vérification **contredit l'hypothèse implicite du
    plan** : `claude.rs` **appelle bien** `emit_llm_call_attempt`, à deux sites
    (`:656`, `:725`). Les sites d'appel sont donc **six** sur trois rails, pas
    deux — le plan n'en citait que deux et omettait aussi les deux d'`ollama.rs`
    (`:628`, `:682`). Le nouveau `max_tokens` y est renseignable trivialement
    (`request.max_tokens`, déjà en portée et déjà journalisé) ; `cap_exhausted`
    y vaut **toujours `None`**, par D7 — le rail n'applique pas le plafond qu'il
    déclare (E8), donc `false` y affirmerait une non-guillotine sous une borne
    qui ne gouverne pas l'appel. Conséquence de lecture nommée dans les Changements
    **et** dans les Surfaces opérateur : un `cap_exhausted` absent a désormais
    deux causes, `select(.cap_exhausted == true)` reste exact, mais compter les
    `false` demande de filtrer sur `provider`.
