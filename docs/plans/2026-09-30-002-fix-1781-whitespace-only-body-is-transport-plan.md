# mika#1781 — un corps de réponse LLM fait uniquement de blancs est `Transport`, jamais `ParseError`

**Ticket :** senara-solutions/mika#1781
**Type :** fix (bug)
**Périmètre :** `crates/mika-common/src/llm/` — deux fichiers, ~15 lignes de production
**Branche :** `bug/1781/llm-whitespace-only-response-body-is-a`

---

## Le reliquat, re-mesuré contre `main` (2026-09-30)

La partie massive du ticket est fermée par deux PRs mergées le 2026-08-27, et **les deux sont
présentes sur `main`** — vérifié par lecture, pas par confiance dans le corps du ticket :

| PR | Ce qu'elle a livré | Site sur `main` |
|----|--------------------|-----------------|
| #2015 | lecture du body en texte avant désérialisation + `warn!` portant serde error, `body_len`, extrait plafonné | `openai.rs::send_once`, le `warn!` « LLM response body did not parse » |
| #2016 | échec de **lecture** du body → `Transport` (retryable) au lieu de `ParseError` | `openai.rs::send_once`, le `match response.text().await` et son bras `Err` |

**Le reliquat est AC1, et il est absent de `main`.** Mesure directe :

```
$ grep -rn "trim().is_empty()\|whitespace-only\|blank" crates/mika-common/src/llm/*.rs
```

Aucune occurrence dans `send_once`, sur aucun des deux rails. Les seules occurrences de
`trim().is_empty()` du répertoire concernent une clé d'API absente (`model_override.rs:101`) et des
lectures de réglage (`budget.rs`, `budget_provenance.rs`) — rien à la frontière de réponse.

Sur `main` aujourd'hui, un body de 1320 octets de blanc traverse le bras `Ok` de `response.text()`,
atteint `serde_json::from_str`, rend `EOF while parsing a value at line 241 column 0`, et devient un
`LlmError::ParseError` — que `is_retryable()` rend `false`. Le tour meurt sans retry.

**Volume : n=1 en 12 jours** (le body du 2026-08-28T20:53:23Z, provider `openrouter`). Ce qui garde
le ticket ouvert est l'invariant, pas le volume : *un cycle ne peut pas être tenu pour réussi si sa
sortie est vide ou illisible* (bearing Prime, RT#009). Le plan ne déguise pas n=1 en urgence.

---

## Trois rectifications que la lecture du code impose au matériau du 2026-09-09

Le plan `docs/plans/2026-09-09-001-fix-1781-whitespace-only-body-is-transport-plan.md` (GROOMED en
seconde passe, lisible au commit `bd036209`, retiré de la branche par `709682a7`) visait exactement
ce reliquat. Trois choses ont bougé depuis, et deux d'entre elles changent le code à écrire.

### R1 — La prémisse léguée non vérifiée est VÉRIFIÉE, et la raison est plus forte que celle avancée

Le commentaire de grooming du 2026-09-09 léguait ceci, explicitement :

> The plan commits to `body.trim().is_empty() → Transport` on the assumption that #1744's deadline
> arithmetic bounds the retry even if a provider returns blank on a *permanent* condition. That
> assumption was not checked by reading #1744. […] If reading #1744 shows the retry is not bounded
> there, that is a criterion change — loop back, do not ship around it.

**Vérifié par lecture de `openai.rs::send_message_inner` (l.436-461) :**

```rust
let max_attempts = if deadline.is_some() {
    self.budget.max_attempts(MAX_ATTEMPTS_HARD_CAP)
} else {
    MAX_ATTEMPTS_HARD_CAP
};
for attempt in 0..max_attempts { … }
```

La borne ne dépend **pas** de l'arithmétique de deadline dans tous les cas, et c'est ce que le plan
précédent n'avait pas établi :

- **Avec deadline** — `budget.max_attempts(…)` rend `floor(enveloppe / plafond)`, ce qui fait
  `attempts × plafond ≤ enveloppe` vrai **par construction** (mika#2189, ajusté par mika#2342 et
  mika#2362). À la géométrie de flotte 120/300 cela vaut **2**.
- **Sans deadline** — `MAX_ATTEMPTS_HARD_CAP` = `DEFAULT_ATTEMPTS_HARD_CAP` =
  `DEFAULT_MAX_RETRIES + 1` = **4** (`llm/mod.rs:89,97`). C'est une **constante finie**, pas un
  infini.

Donc la chaîne est bornée **des deux côtés de la condition** : `for attempt in 0..max_attempts` est
une boucle bornée quoi qu'il arrive, et le deadline ne fait que la **rétrécir**. Un provider qui
rendrait du blanc sur une condition *permanente* coûte au pire 4 tentatives, puis l'erreur remonte.

**Conséquence : ce n'est pas un changement de critère, et il n'y a pas de boucle en arrière à
faire.** La prémisse est validée par une lecture directe du code que le grooming du 09/09 ne
pouvait pas faire dans les mêmes termes — mika#2189 (borne `max_attempts`) a été livré après.

*Ce que cela ne dit pas :* le pire cas devient `4 × plafond` de latence sur un body blanc permanent
au lieu d'un échec immédiat. À 120 s de plafond sans deadline c'est 8 minutes de retry inutile. C'est
le prix, il est borné, et il est **identique** à celui que #2016 a déjà accepté pour les échecs de
lecture — qui tirent 25 à 56 fois par jour contre n=1 ici.

### R2 — La signature de `send_once` a changé : le fix doit décider de `cap_exhausted`

mika#2280 a été livré après le 09/09 et a changé la signature :

```rust
async fn send_once(&self, request: &OpenAiRequest) -> Result<OpenAiResponse, (LlmError, bool)>
```

Le `bool` est `cap_exhausted` — « le body a cessé d'arriver à ≈ le plafond par appel, i.e. *le
modèle générait encore* ». Tout retour d'erreur doit donc porter une valeur, et le plan du 09/09,
écrit contre l'ancienne signature, ne pouvait pas le nommer.

**Décision : `false`, via le helper `plain(…)` déjà en place.** Raison, et elle n'est pas
cosmétique : un body blanc **a fini d'arriver** — `response.text()` a rendu `Ok`. Ce n'est pas une
coupure, donc ce n'est pas une guillotine de plafond. Le CLAUDE.md déclare le régime attendu de la
population `llm_call_cap_exhausted` (« combien de coupures sont des guillotines ») ; y faire entrer
un body complet serait très exactement la fausse attribution que mika#2280 s'est écrite pour
éviter, et que son propre commentaire refuse déjà pour le `response.text()` des réponses non-2xx.

### R3 — `ollama.rs` porte la classe latente identique, et reste hors périmètre avec sa raison

Le rail `ollama.rs` a **le même code, aux mêmes deux étages** :

- `l.545` — `let body = match response.text().await { … }` → `Transport` (le fix #2016, porté là aussi)
- `l.595` — `let resp: OllamaChatResponse = serde_json::from_str(&body).map_err(…)` → `ParseError`

Donc un body blanc y serait classé `ParseError` exactement comme ici. Et le précédent est
troublant : mika#2280 déclare son périmètre comme « **les deux rails en forme OpenAI**
(`openai.rs`, `ollama.rs`) », donc la maison a déjà établi une fois que ces deux fichiers bougent
ensemble pour cette classe de correctif.

**Décision : livrer sur `openai.rs` seul, et nommer `ollama.rs:595` comme suivi.** Trois raisons,
dans l'ordre du poids :

1. **AC1 nomme `openai.rs`.** Élargir le périmètre d'un ticket que l'opérateur vient de re-cadrer
   à « le reliquat, c'est AC1 » (commentaires 7 et 8 du 2026-09-30) est une décision d'opérateur,
   pas d'implémenteur.
2. **La population ollama est structurellement absente.** Le défaut mesuré est un padding de
   keepalive émis par un proxy/LB pendant qu'un backend expire. Un ollama est servi en local
   (`localhost:11434` par défaut) — il n'y a pas de LB devant. Armer le classifieur là serait le
   poser sur une population dont rien ne montre l'existence.
3. **Le coût du report est nul**, parce que le design ci-dessous met la règle dans une **fonction
   pure partagée** (`error.rs`) : le suivi ollama est alors un `if let` de trois lignes au site
   `l.595`, sans duplication de la règle ni de la formulation du message.

**Suivi nommé**, précondition écrite : qu'une mesure montre un `LLM response body did not parse` sur
le rail ollama dont l'extrait est blanc. Tant que cette mesure n'existe pas, le suivi n'est pas
ouvert — un classifieur sur une population vide est un mécanisme dont le silence ne prouve rien.

---

## Design

### La règle vit dans une fonction pure, à un seul site

**Fichier : `crates/mika-common/src/llm/error.rs`**, à côté de `classify_transport_message` — le
module qui possède déjà la classification de frontière et se revendique « site de **définition** »
dans le doc-comment d'`error_class`.

```rust
/// Un corps de réponse composé uniquement de blancs est une réponse VIDE,
/// donc transitoire — jamais un JSON malformé (mika#1781).
///
/// Rend `Some(Transport(…))` quand `body.trim()` est vide, `None` sinon.
/// Le message ne contient jamais « timed out », donc `classify_transport_message`
/// le range en `transport` et non en `transport_timeout` — voir la décision D2.
#[must_use]
pub fn blank_response_body(body: &str) -> Option<LlmError> { … }
```

**Pourquoi une fonction et non un `if` inline au site :** trois propriétés, dont deux sont des
conditions de vérifiabilité.

1. **Testable sans réseau.** `send_once` fait un vrai POST HTTP ; les 30 tests de `openai.rs` sont
   tous des `#[test]` synchrones sur des fonctions pures. AC4 est atteignable sans monter de
   serveur et sans nouvelle dev-dependency.
2. **Un seul site de formulation du message.** Ce message décide de la classe de fil (voir D2) : le
   dupliquer sur deux rails, c'est programmer la divergence que les constantes `error_class`
   existent pour empêcher.
3. **Le suivi R3 devient trois lignes** au lieu d'une copie de la règle.

### Le site d'appel

**Fichier : `crates/mika-common/src/llm/openai.rs::send_once`**, entre le bras `Ok(b) => b` du
`match response.text().await` (l.~370) et `let resp: OpenAiResponse = serde_json::from_str(&body)`
(l.~371) — l'ordre exact qu'AC1 prescrit : *après* la lecture du body, *avant* la désérialisation.

```rust
if let Some(e) = blank_response_body(&body) {
    warn!(
        target: "mika::llm",
        provider = %self.provider_kind,
        body_len = body.len(),
        "LLM response body was whitespace-only (retryable transport, mika#1781)"
    );
    return Err(plain(e));   // cap_exhausted = false — voir R2
}
```

Le `warn!` reste **au site** et non dans la fonction pure : il porte `provider`, que `error.rs` ne
connaît pas. C'est la répartition déjà en place pour les deux `warn!` voisins.

### D2 — La classe de fil est `transport`, et c'est une décision

`LlmError::error_class` route `Transport(msg)` par `classify_transport_message(msg)`, qui rend
`transport_timeout` si le message contient « timed out » et `transport` sinon. Le message retenu —
`empty (whitespace-only) response body, N bytes` — ne le contient pas, donc la classe est
**`transport`**.

C'est le bon côté, et ce n'est pas un effet de bord : on ne **sait pas** que c'est un timeout. La
forme observée le suggère (padding pendant qu'un backend expire) mais rien dans le body ne
l'atteste, et `transport` est précisément la classe « any other transport failure ». Le rappeler
importe parce que ces sept chaînes sont **un format de fil** : l'opérateur en fait des `GROUP BY`
sur `audit_events.callback_delivery_failed` (mika#2179) et sur `llm_call_attempt` (mika#2331).

**Corollaire de lisibilité :** la population du body blanc reste distinguable de celle du body
coupé de #2016 — même variante, même classe, mais deux messages d'erreur à préfixes différents
(`empty (whitespace-only) response body` vs `failed to read response body`) et deux `warn!`
distincts. C'est ce que demande AC2, et c'est ce qui permet de compter les deux séparément.

---

## Definition of Done

1. `blank_response_body` existe dans `error.rs`, rend `Some(Transport(…))` sur un body blanc et
   `None` sinon, et son message nomme la taille en octets.
2. `send_once` de `openai.rs` l'appelle après la lecture du body et avant `serde_json::from_str`,
   émet le `warn!` dédié, et retourne l'erreur avec `cap_exhausted = false`.
3. Un body JSON malformé non blanc conserve `ParseError` et son diagnostic serde complet.
4. Les tests de V1, V2 et V3 ci-dessous passent, et celui de V1 a été **vu rouge** avant le
   correctif de production.
5. `cargo clippy --all-targets -- -D warnings` et `cargo test -p mika-common` passent.

## Acceptance criteria

Transcrits du corps du ticket, verbatim.

- **AC1** — Dans `send_once` (`crates/mika-common/src/llm/openai.rs`), après la lecture du body en
  texte et **avant** `serde_json::from_str`, un body dont `trim()` est vide retourne
  `LlmError::Transport` avec un message nommant la taille en octets, et **jamais** `ParseError`.
- **AC2** — Ce cas émet un `warn!` dédié sur le target `mika::llm` portant `provider` et
  `body_len`, distinct du `warn!` « LLM response body did not parse » de #2015 — les deux classes
  restent séparables dans les logs.
- **AC3** — Un body JSON réellement malformé (non-blanc) conserve `LlmError::ParseError` et son
  diagnostic serde complet (ligne, colonne, `body_len`, extrait) : aucune régression sur #2015.
- **AC4** — Tests unitaires : un body de 1320 espaces/retours à la ligne rend `Transport` et
  `is_retryable() == true` ; un body `{bad` rend `ParseError` et `is_retryable() == false`. Le cas
  blanc reproduit la forme exacte observée le 2026-08-28.
- **AC5** — Aucun changement au chemin de succès, aux autres variantes d'erreur, ni à la politique
  de retry elle-même : le fix ne fait que **classer** le cas blanc du bon côté de `is_retryable()`.
  `cargo clippy --all-targets -- -D warnings` et `cargo test -p mika-common` passent.

---

## Verification contract

### V1 — Le classifieur décide juste (AC4), et il est vu rouge avant

`error.rs`, `mod tests`. **Trois cas, pas deux** — le troisième est ce qui empêche le prédicat de
mordre trop large :

| cas | entrée | attendu |
|-----|--------|---------|
| V1a — la forme mesurée | 1320 octets d'espaces et de `\n` reproduisant le body du 2026-08-28 (240 lignes) | `Some(Transport)`, `is_retryable() == true`, message portant `1320` |
| V1b — contrôle négatif, JSON malformé | `{bad` | `None` — le classifieur **ne réclame pas** ce body, qui reste au serde |
| V1c — contrôle négatif, JSON valide | un `OpenAiResponse` minimal valide | `None` |

**V1a doit être vu rouge avant le correctif de production.** Sur `main` un body blanc n'atteint
aucun classifieur : la fonction n'existe pas. La procédure est donc : écrire `blank_response_body`
rendant `None` inconditionnellement, voir V1a rouge, puis écrire la règle. Sans ce passage, un test
qui passe du premier coup n'atteste pas qu'il mesure quelque chose.

V1b est le test qui garantit AC3 au niveau du prédicat : un body non blanc doit **traverser** le
classifieur sans être réclamé, sinon le fix mangerait la population de #2015.

### V2 — Le site d'appel traverse le classifieur (scan de source)

C'est la garde que V1 ne peut structurellement pas donner, et sa nécessité est la leçon écrite deux
fois dans ce dépôt : mika#1883 (« tester le helper atteste que `None + Some(n) = Some(n)` ; ça
n'atteste pas que les deux sites y passent ») et mika#2511 (« retirer l'acquisition ne rend **aucune
décision fausse** le jour où on l'écrit : la purge continue de purger, toute la suite reste verte,
et seule la fenêtre se rouvre, en silence »).

Notre cas est exactement celui-là : **retirer le `if let` du site ne fait rougir aucune assertion**.
V1 reste vert, le chemin de succès reste vert, `from_str` reprend le body blanc, et le défaut
revient en silence.

`openai.rs`, `mod tests` — `mika1781_le_site_appelle_le_classifieur_avant_la_deserialisation` :

1. Lit son propre source (`include_str!` / `std::fs` sur le chemin du module).
2. Isole le corps de `send_once`.
3. Assert que `blank_response_body` y apparaît, et que son **offset est inférieur** à celui de
   `let resp: OpenAiResponse = serde_json::from_str` — l'ordre, pas seulement la présence, parce
   qu'un appel placé après la désérialisation serait inerte.
4. **Anti-vacuité** : assert que les deux ancres sont trouvées. Un scan qui ne trouve ni l'une ni
   l'autre (renommage, refactor) passerait sinon en regardant le vide, et un scan silencieusement
   inerte se lit exactement comme un arbre propre (classe mika#2205).

### V3 — Aucune régression sur #2015 (AC3)

Le test existant qui couvre le `warn!` serde et la forme de `ParseError` doit rester vert sans
modification. S'il faut y toucher, c'est le signe que le classifieur mord au-delà de sa population
— **halte**, ne pas ajuster le test.

### V4 — AC5

`cargo clippy --all-targets -- -D warnings` et `cargo test -p mika-common`. Le diff de production ne
touche ni le bras `Ok` de `response.text()`, ni le `map_err` du `from_str`, ni la boucle de retry,
ni `is_retryable()`, ni aucune valeur de réglage.

### Ce que V1–V4 ne peuvent PAS attester, dit plutôt que découvert

Qu'un provider réel rende effectivement un body blanc et que le retry l'absorbe. Cela se passe
chez un tiers, à n=1 en 12 jours. Le contrat **côté mika** est *un body blanc est classé
retryable et le site le consulte*, et V1+V2 l'attestent déterministiquement. La moitié
comportementale est la sonde S1 ci-dessous.

---

## Fire-Disposition

Ce plan livre deux détecteurs. Disposition pour chacun, per mika#1574.

- **V1 et V3 (tests unitaires) et V2 (scan de source) — option (a), exception nommée en allowlist,
  avec une allowlist livrée VIDE.** Ils atterrissent **armés** et **verts** dans le même commit que
  le comportement qu'ils mesurent : il n'existe aucune violation préexistante à excepter, parce que
  le classifieur est du code neuf sur un chemin neuf et non un balayage de sites existants.
  L'ensemble sur lequel l'option « livrer désarmé » travaillerait est vide, donc elle n'a pas
  d'objet. **Quand V2 tire, la résolution est de RÉARMER le site, jamais d'ajouter une entrée
  d'allowlist** (doctrine mika#2201) — un site de désérialisation qu'on ne veut pas faire précéder
  du classifieur est un site dont il faut discuter, pas un site à excepter.

  **La seule violation que V2 pourrait légitimement rencontrer est `ollama.rs:595`** (R3), et elle
  est hors de sa population par construction : le scan lit le corps de `send_once` de `openai.rs` et
  de lui seul. Ce n'est pas une exemption déguisée mais un périmètre déclaré, et le suivi R3 porte
  l'élargissement avec sa précondition de mesure.

- **AC2 — le `warn!` comme sonde d'exécution (opérateur) — option (c), halte-et-remontée.** Voir
  S1/S2 ci-dessous. Cette ligne n'est pas une garde bloquante : elle ne refuse rien, elle rapporte.
  Sa disposition est donc une conduite d'opérateur, et elle est écrite avec ses haltes plutôt que
  laissée à l'inférence.

---

## Surfaces opérateur

```bash
# 1. La classe a-t-elle été attrapée et routée ? (régime attendu : très rare, ~n=1/12j)
grep "LLM response body was whitespace-only" "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{provider, body_len}'

# 2. CONTRÔLE POSITIF — la branche serde de #2015 tire-t-elle encore, et sur quoi ?
grep "LLM response body did not parse" "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c '{provider, body_len, body_excerpt}'

# 3. Le retry a-t-il absorbé ? — la seule question qui compte
grep llm_call_attempt "$MIKA_SPIRIT_LOG_FILE" \
  | jq -c 'select(.event == "llm_call_attempt" and .outcome == "retrying")
           | {provider, attempt, max_attempts, error_class}'
```

| surface | régime attendu | lecture |
|---|---|---|
| `LLM response body was whitespace-only` | **très rare** (n=1 en 12 j avant le fix) | chaque ligne est un tour que `ParseError` aurait tué |
| la même ligne, puis `resume_agent run failed` sur le même `task_id` | **vide** | le retry n'a pas absorbé — halte S2 |
| `LLM response body did not parse` avec un extrait **blanc** | **vide** | le classifieur est en amont et ne mord pas — halte S3 |
| `LLM response body did not parse` avec un extrait non blanc | non vide possible | population légitime de #2015, inchangée |

---

## Sondes post-déploiement, et leurs trois haltes

**S1 — la classe est routée (à la prochaine occurrence).** Une ligne de la commande 1, suivie d'un
`llm_call_attempt` portant `outcome: "retrying"` et `error_class: "transport"`, et **pas** d'un
`resume_agent run failed` sur le même tour.

**Halte S1 — la commande 1 reste vide sur 30 jours.** Cela ne prouve **rien** : à une base de n=1
en 12 jours, le silence est le régime le plus probable même si le fix est parfait. Lire le
**contrôle positif** (commande 2) avant toute conclusion : si la branche serde de #2015 ne tire pas
non plus, la classe n'a simplement pas récidivé. *La preuve du fix est V1a vu rouge puis vert, pas
ce silence* (classe mika#2205 : une garde que personne n'a exercée se lit exactement comme une
garde qui marche).

**Halte S2 — la ligne apparaît ET le tour meurt quand même.** Le fix a re-classé l'erreur sans
changer l'issue. **Ne pas élargir le classifieur** : établir d'abord si `max_attempts` valait 1 sur
ce tour (lire `llm_call started`, champ `max_attempts` — une géométrie où l'enveloppe est un
multiple exact du plafond rend la dernière tentative inatteignable, mika#2362), puis lire
`llm_budget_retry_unreachable`. Le levier est alors la géométrie, pas cette frontière.

**Halte S3 — un `LLM response body did not parse` porte un extrait blanc.** Le classifieur est en
amont et ne mord pas : c'est le défaut revenu. Lire V2 en premier — si le scan est vert alors que le
site ne classe pas, le prédicat du scan regarde la mauvaise ancre. **Établir cela avant de toucher
`blank_response_body`.**

**Halte transverse — aucune des deux commandes ne rend rien et un tour a visiblement échoué en
parse.** Vérifier que le binaire servi porte le correctif avant toute conclusion sur le code
(classe mika#2340) : `mika-common` est compilé dans `mika-spirit`, donc la sonde décrit le binaire
déployé, pas le checkout.

---

## Out of scope

- **Toute forme de réparation de JSON malformé** (schema-gated repair, JSONSuture, etc.). Un body
  non blanc mais invalide reste une erreur, pas quelque chose à recoudre. V1b est le test qui pose
  cette frontière.
- **Changer la politique de retry, ses bornes ou son backoff** — #1744 possède ce chemin, et R1
  établit qu'il n'y a rien à y changer : la chaîne est déjà bornée, deadline ou pas.
- **Découper `ParseError` en variantes par étage** (`ResponseEnvelopeParse` / `ToolArgumentsParse`),
  comme le proposait le commentaire externe du 2026-08-27. Mesure post-#2015 : la branche
  envelope-serde a tiré **une fois**, l'étage tool-arguments **jamais**. Refendre un type qui est
  par ailleurs un format de fil (`error_class`) pour n=1 n'est pas porté par l'évidence.
- **Capture de bodies bruts en fixture.** #2015 a fixé la posture : extrait plafonné, sur échec
  seulement. La fixture de V1a est **construite** (1320 octets de blanc générés), pas capturée —
  un body de blanc n'a aucun contenu client à protéger, ce qui est précisément ce qui la rend
  reproductible sans poser de question de vie privée.
- **Persister un cycle en échec typé et le remonter bruyamment quand les retries sont épuisés**
  (point 3 du commentaire externe). Le manque est réel et vit dans le contrat de complétion de
  cycle du task engine, pas à la frontière de réponse LLM. Ticket distinct contre
  `mika_agent::task_engine`.
- **`ollama.rs:595`** — classe latente identique, hors périmètre avec ses trois raisons (R3).
  Suivi nommé, précondition : une mesure montrant un body blanc sur ce rail.
- **Un test d'intégration `wiremock` au site de production.** `wiremock 0.6` est déclaré au
  workspace mais absent des dev-dependencies de `mika-common`, et les 30 tests de `openai.rs` sont
  tous synchrones — l'introduire demande une dev-dep et un runtime async dans un fichier qui n'en
  a aucun. V2 (scan de source) couvre la même classe de régression, et c'est la forme que ce dépôt
  a déjà retenue pour elle (mika#2511). Renforcement possible, non requis.

---

## Ce que ce travail n'achète PAS

- **Il ne fait pas cesser les bodies blancs.** Il les rend **retryables**, donc rattrapables. La
  cause est chez un proxy/LB en amont d'`openrouter` et n'est pas adressable ici.
- **Il ne garantit pas que le retry réussisse.** Si le provider rend du blanc quatre fois de suite,
  le tour échoue — bruyamment, avec une classe `transport` lisible, ce qui est l'invariant RT#009
  (retenté **ou** remonté bruyamment), jamais avalé en silence.
- **Il n'ajoute aucun compteur et aucune ligne `audit_events`.** `mika-common` n'a pas d'accès
  base ; les surfaces sont les greps ci-dessus et `llm_calls`. **Leur silence ne prouve rien tant
  que personne ne les exécute** — et à n=1 en 12 jours, leur silence ne prouve rien même exécutées.
- **Il ne ferme pas le rail Anthropic.** `claude.rs` a son propre chemin de désérialisation et son
  propre type d'erreur (`ClaudeApiError`, aplati en `ProviderError` par `llm/anthropic.rs`) ;
  aucune occurrence mesurée de cette classe n'y est attestée, et l'y porter demanderait de
  traverser cet aplatissement — autre périmètre.
