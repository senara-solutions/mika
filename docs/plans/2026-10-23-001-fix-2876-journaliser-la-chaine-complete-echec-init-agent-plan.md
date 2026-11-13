---
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
issue: mika#2876
branch: fix/2876/server-l-chec-d-initialisation-d-un
---

# fix(server): l'échec d'initialisation d'un agent journalise la chaîne complète, pas son seul contexte externe

## Problem Frame

`crates/mika-agent/src/server/mod.rs:1104` écrit :

```rust
warn!(agent = name, error = %e, "failed to initialize agent, skipping");
```

Le sigil `%` de `tracing` est un `Display`. Sur une erreur `anyhow`, `Display`
rend **le seul contexte le plus externe** ; la cause posée dessous par
`.with_context(...)` n'apparaît pas. C'est `{e:#}` — l'`alternate Display` —
qui aplatit la chaîne `source()` entière, jointe par `: `.

Le site qui pose ce contexte est `server/mod.rs:610-611` :

```rust
let kg_config = crate::kg::config::resolve_per_agent_docs_root(&identity, &agent_settings)
    .with_context(|| format!("failed to resolve [kg] config for agent {agent_name}"))?;
```

et la cause qu'il enveloppe est `KgConfigError::PathNotFound`
(`crates/mika-agent/src/kg/config.rs:58-65`), dont le `Display` est
`docs_root path does not exist: {path}`, avec une `std::io::Error` en `#[source]`.

Mesuré le 2026-10-22 (moteur `7eb63e43`) : `/var/log/mika/server.log` porte
**37 occurrences** de `failed to initialize agent` pour mika-litha, toutes avec
pour seul texte `failed to resolve [kg] config for agent mika-litha`, et
**aucune cause**. Le `docs_root` absent n'a été trouvé qu'en lisant
`identity.toml` à la main. Après le correctif, la même ligne porterait :

```
failed to resolve [kg] config for agent mika-litha: docs_root path does not exist: <chemin>: path does not exist
```

C'est du substrat de niveau 2 : la boucle ne casse pas, mais **chaque incident
d'initialisation re-paye l'enquête**, et le log de production ment par omission
sur un agent qui ne démarre plus.

Un second axe, du même ticket et de la même classe (mika#2205) : les **deux**
chemins d'initialisation rendent aujourd'hui des octets indistinguables sur ce
point. Le démarrage (`server/mod.rs:1104`) et la résolution paresseuse
(`server/state.rs:336-340`, mika#1399) appellent tous deux `init_agent` et
masquent tous deux la cause de la même manière. Corriger un seul laisserait la
moitié de la population muette.

## Requirements

- **R1** — Les sites qui journalisent un échec de `init_agent` doivent rendre la
  chaîne `source()` complète de l'erreur `anyhow`, et non son seul contexte le
  plus externe.
- **R2** — Le comportement est inchangé : l'agent est toujours **sauté** (pas de
  propagation, pas d'arrêt du serveur), et le **niveau** du log reste `WARN`.
- **R3** — Le **message** et le **nom des champs** de chaque site restent
  identiques octet pour octet (`"failed to initialize agent, skipping"`,
  `"lazy agent construction failed"`, champs `agent` et `error`) : les greps
  opérateur et les mesures existantes sur ces littéraux doivent continuer à
  répondre.
- **R4** — Une couverture de régression doit rougir contre `%e` et verdir contre
  la chaîne aplatie, en capturant un **vrai** appel à `init_agent` dont la
  config kg pointe vers un `docs_root` absent.
- **R5** — La propriété « ces sites aplatissent la chaîne » doit être tenue
  structurellement, pour que l'ajout d'un troisième appelant de `init_agent`
  copiant-collant l'ancienne ligne ne réintroduise pas le masque en silence.

## Key Technical Decisions

### KTD1 — `let chain = format!("{e:#}")` puis `error = %chain`, aux deux sites, sans helper

L'idiome existe déjà **trois fois** dans le dépôt, sous exactement cette forme :
`crates/mika-common/src/claude.rs:995`, `crates/mika-common/src/llm/openai.rs:325`,
`crates/mika-common/src/llm/ollama.rs:558` — une chaîne construite localement,
puis passée en `error = %chain`. Le correctif suit cet idiome.

Forme exacte, au site de démarrage (`server/mod.rs:1104`) :

```rust
Err(e) => {
    // mika#2876 : `%e` ne rend que le contexte le plus externe ; `{e:#}`
    // aplatit la chaîne `source()` entière. Message, champs et niveau
    // inchangés — les greps opérateur portent sur ce littéral.
    let chain = format!("{e:#}");
    warn!(agent = name, error = %chain, "failed to initialize agent, skipping");
}
```

et au site de résolution paresseuse (`server/state.rs:336-340`), à l'identique,
avec son propre littéral de message et `agent = normalized.as_str()`.

**Pourquoi pas de helper d'émission partagé** (un `warn_agent_init_failed(agent,
site, &e)` dans un nouveau `server/init_failure.rs`, avec un `enum InitSite`).
C'était l'autre forme envisagée, et elle est refusée :

- Ce qui est dupliqué n'est pas une **décision**, c'est un **idiome de
  formatage** d'une ligne. La doctrine du dépôt refuse le second site
  *décisionnel* (mika#2201) ; elle ne demande pas d'abstraire `format!`.
- Un helper qui doit préserver deux littéraux de message distincts porte un
  `match` sur un `enum` à deux variantes dont le seul rôle est de rendre une
  constante de chaîne — une indirection qui coûte un module, un `pub(crate)`,
  un import dans `state.rs`, et qui rend la ligne **moins** lisible au site.
- La synchronisation des deux sites est justement ce que livre U3 : un scan de
  source, qui tient la propriété sans déplacer le code.

**Pourquoi pas non plus un `site` structuré en plus** (un champ `site =
"startup" | "lazy_resolve"`). La distinction est réelle et de classe mika#2205,
mais elle est **déjà portée par le message** : les deux littéraux sont distincts
et grep-visibles (`failed to initialize agent, skipping` contre `lazy agent
construction failed`). Ajouter un champ redondant élargirait le périmètre d'un
ticket dont la taille déclarée est S, sans rien rendre lisible qui ne le soit.

**Coût accepté, nommé :** la ligne de log s'allonge. Sur l'incident fondateur la
chaîne complète fait ~115 caractères contre ~50 — c'est exactement le contenu
qui manquait, donc le coût est le bénéfice. Aucun consommateur en aval ne parse
le champ `error` (grep manuel et lecture opérateur uniquement), donc
l'aplatissement ne casse aucun lecteur.

### KTD2 — AC1 passe par un **vrai** `init_agent`, pas par une erreur reconstruite

Un test qui fabriquerait une `anyhow::Error` de la bonne forme puis appliquerait
`format!("{e:#}")` dessus serait tautologique : il attesterait `anyhow`, pas le
site. AC1 demande « un test capture le log d'une initialisation » — donc l'appel
réel.

Il est atteignable, et les quatre obstacles sont résolus :

1. **`make_llm_provider()` (`server/mod.rs:605`) échouerait d'abord** —
   `ClaudeClient::new` refuse une `api_key` absente ou vide
   (`crates/mika-common/src/claude.rs:401-416`), et `llm_provider` vaut
   `ProviderKind::Anthropic` par défaut (`config.rs:740, 2710`). Le test écrit
   donc `MIKA_ANTHROPIC_API_KEY=sk-ant-test-2876` dans `<agent_home>/.env` : par
   la cascade de `Settings::load_for_agent` (`config.rs:2613-2653`), le `.env`
   de l'agent est la source de **plus haute** priorité — inversion délibérée,
   épinglée par `mika2218_per_agent_dotenv_wins_over_process_env_in_server_mode`
   (`config.rs:3133-3159`). Donc pas de `unsafe set_var`, pas de dépendance à
   l'environnement ambiant. La valeur n'a pas la forme d'un jeton OAuth, donc
   aucune excursion vers `resolve_home_dir()`.
2. **Rien après la ligne 611 n'est atteint** quand la résolution kg échoue :
   pas de passerelle, pas de réseau, pas d'amorçage de skills.
3. **Les onze arguments sont tous constructibles en test** :
   `Arc::new(default_tools())` (`tools/mod.rs:1400`), `reqwest::Client::new()`,
   `SecretString::from("test")`, `Arc::new(dashmap::DashMap::new())`, `None`
   pour l'embedding et la clé Brave, `true` pour `disable_bundled_skills`, et
   deux `tempfile::TempDir` pour `global_home` / `agent_home`.
   `std::fs::create_dir_all` sur `home::container_db_path(global_home).parent()`
   (`home.rs:338-340`) est exécuté par `init_agent` lui-même, après 611 — donc
   hors du chemin du test.
4. **Le harnais de capture existe déjà** — voir KTD3.

L'`identity.toml` écrit par le test porte la forme de `KgIdentityConfig`
(`prompt.rs:470-487`) :

```toml
name = "mika-test-2876"

[kg]
enabled = true
docs_root = "<tmp>/absent-2876"
```

Le chemin n'est jamais créé, donc `resolve_per_agent_docs_root` prend la branche
`Ok(false) => Err(KgConfigError::PathNotFound { .. })` (`kg/config.rs:347-359`),
politique d'erreur dure pour un chemin explicite.

### KTD3 — La capture est celle du harnais mika#2680, filtrée sur `message`

`crates/mika-agent/src/server/mod.rs:6424-6527` porte déjà `Mika2680Layer`,
`Mika2680Event`, `mika2680_capture()` et `mika2680_events_named()`. **Aucun
nouveau harnais** : le test réutilise `mika2680_capture()`, et
`crate::test_utils::test_helpers::install_capturing_subscriber`
(`test_utils.rs:64-71`) reste le seul site d'installation d'abonné permis
(mika#2646).

Une différence à ne pas découvrir à l'exécution : `mika2680_events_named()`
filtre sur un champ `event`, que notre `warn!` **ne porte pas**. Le test filtre
donc sur le champ `message` — `tracing` enregistre le message du macro comme un
champ nommé `message`, déjà présent dans la `HashMap` de `Mika2680Event`.

Doctrine de callsite (mika#2680 / mika#2646), obligatoire ici : l'`Interest`
d'un callsite est figé par le premier thread qui l'atteint. Le test porte donc
`#[serial_test::serial]`, et fait un **tour de chauffe** (un premier appel à
`init_agent` dont le résultat est ignoré) **avant** d'installer l'abonné, puis
`rebuild_interest_cache()` — ce dernier est déjà fait par
`install_capturing_subscriber`.

### KTD4 — Le scan porte sur la **fenêtre du macro**, pas sur le fichier

U3 ne compte pas des occurrences dans un fichier : il localise chaque littéral
de message canonique, remonte jusqu'au `warn!(` qui l'ouvre (au plus 8 lignes),
et juge **ce bloc** :

- il doit contenir `error = %` (le champ survit) ;
- il ne doit **pas** contenir `error = %e` exactement (le masque est revenu).

Le prédicat est extrait en fonction pure prenant `(rel, contenu)` et rendant les
violations, pour qu'un **contrôle de bonne foi** puisse lui soumettre un bloc
fabriqué portant `error = %e` et vérifier qu'il rougit — sans quoi un scan vert
parce qu'il ne regarde rien serait indistinguable d'un scan vert parce que le
code est sain (classe `grooming_marker`, mika#2158).

**Angle mort nommé, plutôt que prétendu absent :** un troisième site qui
inventerait un message **nouveau** et le journaliserait avec `%e` n'est pas vu
par ce scan. Le prédicat est sur les deux littéraux canoniques, pas sur « tout
log d'erreur d'init », parce qu'un prédicat plus large attraperait les `error =
%e` légitimes voisins — notamment `state.rs:280-284`, la garde de dérive de
tier (mika#1962), dont l'erreur est un `thiserror` sans contexte empilé et pour
qui `%e` est **correct**. Un scan dont l'allowlist se remplirait de voisins
légitimes est précisément l'allowlist que mika#2309 condamne. L'angle mort va
dans le doc-comment du scan.

## Implementation Units

### U1. Aplatir la chaîne aux deux sites de journalisation

- **Objectif :** les deux sites qui journalisent un `Err` de `init_agent`
  rendent `{e:#}` au lieu de `%e`, message / champs / niveau inchangés.
- **Requirements :** R1, R2, R3, AC1
- **Dépendances :** aucune
- **Fichiers :**
  - `crates/mika-agent/src/server/mod.rs` (modifier le bras `Err(e)` à ~1104)
  - `crates/mika-agent/src/server/state.rs` (modifier le bras `Err(e)` à ~336-340)
- **Approche :** à chaque site, insérer `let chain = format!("{e:#}");` avant le
  `warn!`, et remplacer `error = %e` par `error = %chain`. Ajouter un commentaire
  d'une ligne citant mika#2876 et la raison (`%` est un `Display`, donc le
  contexte externe seul). Ne rien changer d'autre : `warn!` reste `warn!`, le
  littéral de message reste identique, le `None` rendu par `state.rs` reste
  `None`, la boucle de `mod.rs` continue de passer à l'agent suivant.
- **Sites explicitement NON touchés**, et pourquoi (entrée d'AC3) :
  - `server/mod.rs:1060` — chemin `DEFAULT_AGENT` mono-agent : il propage par
    `.await?` et ne journalise rien. Ce n'est pas un site de journalisation.
  - `server/state.rs:280-284` — garde de cohérence de tier de famille
    (`check_agent_tier_consistency`, mika#1962) : même forme, **autre source
    d'erreur**, sans contexte `anyhow` empilé. `%e` y est correct.
  - `crates/mika-cli/src/commands/setup.rs:36` — site de **propagation**
    (`.with_context(...)?`), pas d'émission. Son imprimeur terminal est
    `mika-cli/src/main.rs:396`, qui rend **déjà** `{e:#}`. (La divergence avec
    `main.rs:425`, qui rend `{e}`, est du territoire mika#1985 — hors périmètre.)
- **Patterns à suivre :** `crates/mika-common/src/claude.rs:995`,
  `llm/openai.rs:325`, `llm/ollama.rs:558` — même idiome `chain` + `error = %chain`.
- **Vérification :** `cargo build -p mika-agent` ; `cargo clippy -p mika-agent
  --all-targets -- -D warnings` propre.
- **Scénarios de test :** couverts par U2 (comportement) et U3 (structure).

### U2. Test comportemental AC1/AC2 — un vrai `init_agent` sur un `docs_root` absent

- **Objectif :** capturer le `WARN` d'un appel réel à `init_agent` dont la
  config kg pointe vers un `docs_root` inexistant, et asserter que le champ
  `error` porte la **cause interne**.
- **Requirements :** R4, AC1, AC2
- **Dépendances :** U1
- **Fichiers :**
  - `crates/mika-agent/src/server/mod.rs` (ajouter au `#[cfg(test)] mod tests`
    existant, près du harnais mika#2680)
- **Approche :**
  1. Deux `tempfile::TempDir` : `global_home`, `agent_home`.
  2. Écrire `<agent_home>/identity.toml` avec le `[kg]` de KTD2, `docs_root`
     pointant sur un sous-chemin de `global_home` **jamais créé**.
  3. Écrire `<agent_home>/.env` avec `MIKA_ANTHROPIC_API_KEY=sk-ant-test-2876`.
  4. Construire les onze arguments (KTD2 point 3).
  5. **Tour de chauffe** : appeler `init_agent` une première fois, ignorer le
     résultat — fige l'`Interest` du callsite avant l'installation de l'abonné.
  6. `let (_guard, events) = mika2680_capture();`
  7. Appeler `init_agent`, asserter que le résultat est `Err`.
  8. Filtrer `events` sur `fields["message"] == "failed to initialize agent, skipping"`
     — asserter exactement **une** ligne (son absence voudrait dire que le test
     n'a rien mesuré, pas que le code est sain).
  9. Sur son champ `error` :
     - `assert!(contains("docs_root path does not exist"))` — **la propriété
       d'AC1**, et le rouge-avant d'AC2 : avec `%e` ce texte est absent ;
     - `assert!(contains("failed to resolve [kg] config for agent"))` — le
       contexte externe **survit**, l'aplatissement n'a rien remplacé ;
     - `assert!(fields.contains_key("agent"))` — le champ structuré survit (R3).
- **Patterns à suivre :** le harnais `mika2680_capture()` /
  `Mika2680Event` à `server/mod.rs:6424-6527` ; `#[serial_test::serial]` +
  `#[tokio::test]` comme les tests voisins de la famille mika#2680.
  `install_capturing_subscriber` (`test_utils.rs:64-71`) reste le seul site
  d'installation.
- **Scénarios de test :**
  - **Chemin nominal (AC1) :** la cause interne est dans le log capturé.
  - **Contexte préservé (R3) :** le texte du contexte externe y est aussi.
  - **Champs préservés (R3) :** `agent` est toujours un champ structuré.
- **Vérification :** `cargo test -p mika-agent server::tests::mika2876` passe.
  Rouge-avant : voir V5.

### U3. Scan de source — les deux sites canoniques aplatissent la chaîne

- **Objectif :** tenir R5 structurellement — un troisième appelant de
  `init_agent` qui copierait-collerait un des deux littéraux avec `error = %e`
  rougit, au lieu de réintroduire le masque en silence.
- **Requirements :** R5
- **Dépendances :** U1
- **Fichiers :**
  - `crates/mika-agent/src/canonical_tokens.rs` (ajouter au `mod tests`
    existant : deux constantes, un prédicat pur, le scan, son contrôle négatif,
    et l'assertion d'allowlist vide)
- **Approche :**
  1. Deux constantes grep-visibles pour les littéraux :
     `INIT_FAILURE_STARTUP_MESSAGE = "failed to initialize agent, skipping"` et
     `INIT_FAILURE_LAZY_MESSAGE = "lazy agent construction failed"`.
  2. `const INIT_FAILURE_CHAIN_EXCEPTIONS: &[&str] = &[];` — **livrée vide**.
  3. Un prédicat pur `init_failure_masked_sites(rel, content) -> Vec<String>` :
     pour chaque ligne portant un des deux littéraux, remonter au plus 8 lignes
     jusqu'au `warn!(` ouvrant, et signaler le bloc s'il porte `error = %e` ou
     s'il ne porte aucun `error = %`.
  4. Le scan itère `production_sources()` (`canonical_tokens.rs:132`, qui écarte
     déjà le code de test par `is_test_source_path` **et** par troncature au
     premier `#[cfg(test)]`), saute les entrées d'allowlist, et asserte zéro
     violation.
  5. **Terme de vivacité** (`owner_seen`, idiome
     `mika2474_the_threshold_has_a_single_reader`,
     `canonical_tokens.rs:5255-5300`) : les **deux** littéraux doivent avoir été
     trouvés au moins une fois dans la production. Sans ce terme, un renommage
     de message rendrait le scan silencieux et donc décoratif.
  6. Message d'assertion portant une clause `RÉSOLUTION :` — *router le site par
     `let chain = format!("{e:#}")`, ne pas l'allowlister* — plus le nom des
     deux sites attendus.
  7. Doc-comment portant l'angle mort nommé en KTD4 (un message **nouveau**
     journalisé avec `%e` n'est pas vu) et la raison du refus d'un prédicat plus
     large (`state.rs:280-284` est un `%e` légitime).
- **Patterns à suivre :** `mika2474_the_threshold_has_a_single_reader` et son
  contrôle de bonne foi (`canonical_tokens.rs:5240-5302`) ;
  `mika2474_the_overrun_sole_writer_allowlist_is_empty` pour l'assertion
  auto-nettoyante d'allowlist vide.
- **Scénarios de test :**
  - **Scan vert sur la production corrigée.**
  - **Contrôle de bonne foi :** le prédicat, nourri d'un bloc fabriqué portant
    `warn!(agent = name, error = %e, "failed to initialize agent, skipping")`,
    rend exactement une violation.
  - **Allowlist vide :** assertion auto-nettoyante.
- **Vérification :** `cargo test -p mika-agent canonical_tokens::tests::mika2876`
  passe, y compris le contrôle négatif.

## Verification Contract

- **V1** — `cargo build -p mika-agent` réussit.
- **V2** — `cargo test -p mika-agent` passe, y compris les tests de U2 et U3.
- **V3** — `cargo clippy --all-targets -- -D warnings` propre.
- **V4** — `cargo fmt --check` propre. Attention : une reformulation du `warn!`
  par rustfmt ne doit pas éclater le macro au point de sortir le littéral de la
  fenêtre de 8 lignes du scan U3 ; si c'est le cas, c'est le scan qu'on élargit,
  jamais le site qu'on allowliste.
- **V5** — **Contrôle fix-in-diff** (mémoire
  `feedback_verify_pipeline_passes_without_the_fix`) : en remettant `error = %e`
  aux deux sites de U1 tout en gardant U2 et U3, `cargo test -p mika-agent`
  **doit échouer** — U2 sur l'absence de `docs_root path does not exist`
  (c'est littéralement AC2), U3 sur la détection du masque. Preuve que les deux
  gardes portent sur le correctif et ne sont pas tautologiques.
- **V6** — `scripts/verify-pipeline.sh origin/main` passe.
- **V7** — Le corps de PR porte la **liste des sites d'initialisation modifiés**
  (AC3), et nomme aussi les sites **non** modifiés avec leur raison — la liste
  d'U1 § *Sites explicitement NON touchés*. Une liste qui ne dit que ce qui a
  bougé laisse « non concerné » et « oublié » rendre les mêmes octets.
- **V8** — Vérification opérateur **après `make deploy`** : sur un agent dont le
  `docs_root` est absent (mika-litha est le cas réel), la ligne
  `failed to initialize agent, skipping` de `/var/log/mika/server.log` porte le
  texte `docs_root path does not exist`. L'instrument n'est pas vivant avant le
  rebuild — son absence signifie un binaire antérieur au correctif
  (motif mika#2340), jamais une absence de cause.

## Definition of Done

- [ ] Le code compile (`cargo build -p mika-agent`).
- [ ] Clippy propre (`cargo clippy --all-targets -- -D warnings`).
- [ ] Formaté (`cargo fmt --check`).
- [ ] Tous les critères d'acceptation cochés (AC1, AC2, AC3).
- [ ] Les tests de régression passent (`cargo test -p mika-agent`).
- [ ] Le contrôle fix-in-diff passe (V5).
- [ ] `scripts/verify-pipeline.sh origin/main` passe.
- [ ] Le corps de PR porte la sortie verbatim de `git diff --stat main..HEAD`.
- [ ] Le corps de PR porte `Closes #2876`.
- [ ] Le corps de PR porte l'empreinte SHA courte de HEAD.
- [ ] Le corps de PR porte la liste des sites d'initialisation modifiés **et**
      des sites non modifiés avec leur raison (AC3, V7).

## Acceptance criteria

- [ ] **AC1** — un test capture le log d'une initialisation dont la config kg
  pointe vers un `docs_root` absent. Le message capturé contient la cause interne
  (le texte de `PathNotFound`, par exemple « docs_root path does not exist »), et
  pas seulement le contexte externe.
- [ ] **AC2** — rouge-avant, le test rougit contre `%e`.
- [ ] **AC3** — la liste des sites d'initialisation modifiés est écrite dans la PR.

## Fire-Disposition

Le plan livre un détecteur : **U3**, le scan de source de
`canonical_tokens.rs`, dont le chemin de succès est « aucune violation trouvée ».
(U2 n'en est pas un : son chemin de succès est une assertion positive — « le log
porte la cause » — et non un corpus sans infraction.)

**Option retenue : (a) exception nommée en allowlist, allowlist livrée VIDE.**

- `INIT_FAILURE_CHAIN_EXCEPTIONS: &[&str] = &[]` est livrée vide, et le reste :
  après U1 il n'existe **aucune** violation dans l'arbre. Les deux seuls sites
  qui portent les littéraux canoniques sont précisément ceux qu'U1 corrige, et
  l'inventaire a été fait (voir U1 § *Sites explicitement NON touchés*) — les
  trois sites voisins ne portent ni l'un ni l'autre des deux littéraux, donc le
  prédicat ne les voit pas et aucun n'a besoin d'exception.
- **Assertion auto-nettoyante :** `mika2876_the_chain_allowlist_is_empty` asserte
  `INIT_FAILURE_CHAIN_EXCEPTIONS.is_empty()`. Une allowlist née vide est un
  emplacement où déposer la prochaine infraction (mika#2323) ; l'assertion
  rougit à la première entrée et force à argumenter.
- **Si le scan tire** sur un site futur : la `RÉSOLUTION :` du message
  d'assertion prescrit de **router le site** par `let chain = format!("{e:#}")`,
  pas de l'allowlister. Aucun ticket de suivi n'est à ouvrir, parce qu'il n'y a
  aucune violation existante à tracker — c'est la différence entre une allowlist
  née vide et une allowlist née pleine.

L'angle mort du détecteur est nommé en KTD4 et porté par son doc-comment : un
troisième site inventant un message **nouveau** et le journalisant avec `%e`
n'est pas vu. Élargir le prédicat à « tout log d'erreur d'init » attraperait les
`%e` légitimes voisins (`state.rs:280-284`) et remplirait l'allowlist de voisins
sains — l'allowlist que mika#2309 condamne. Sous-approximer rend des faux
rouges, jamais des faux verts (mika#2761) ; ici on sur-approxime volontairement
dans l'autre sens, et on l'écrit.

## Taille estimée

| livrable | lignes de code (hors `docs/`) |
|---|---|
| `crates/mika-agent/src/server/mod.rs` (site de démarrage) | 5 |
| `crates/mika-agent/src/server/state.rs` (site de résolution paresseuse) | 5 |
| test AC1/AC2 (`server/mod.rs`, `mod tests`) | 105 |
| scan + prédicat pur + contrôle négatif + allowlist (`canonical_tokens.rs`) | 85 |

Total estimé : 200 lignes
Plafond de tours : 150 (défaut)
Borne par vol : 279 lignes estimées — (150 − 27 − 30) × 9 / 3

**Un seul vol, et voici pourquoi il tient.** 200 estimées contre une borne de 279
— 79 lignes de marge, soit ~28 %. Trois raisons rendent cette estimation peu
susceptible de déraper comme celles de mika#2683 (832 estimées / 1 432 réelles) :

1. **Le correctif de production fait 10 lignes.** Il n'y a pas de nouveau module,
   pas de nouveau type, pas de nouvelle signature (KTD1 refuse explicitement le
   helper et son `enum`) — ce qui était la source du dérapage des vols cités, où
   l'estimation portait sur du code neuf dont la forme était encore ouverte.
2. **Aucun harnais n'est construit.** La capture de log (mika#2680) et
   l'énumérateur de sources (`production_sources()`) existent et sont réutilisés
   tels quels. Le poste le plus gros — les 105 lignes du test — est de la mise en
   place de fixtures (deux `TempDir`, deux fichiers, onze arguments), dont la
   taille est connue parce que les onze arguments ont été inventoriés un par un.
3. **Le périmètre est clos par un inventaire, pas par un jugement.** Les cinq
   sites candidats ont été énumérés et trois sont écartés avec leur raison ; il
   n'y a pas de « et les autres sites » ouvert qui pourrait grossir en vol.

Le total est aussi très loin du seuil par PR de 1000 lignes : aucun découpage en
phases n'est requis non plus.

## Scope Boundaries

**Dans le périmètre :**
- Les deux sites qui journalisent un `Err` de `init_agent` :
  `server/mod.rs:1104` (démarrage) et `server/state.rs:336-340` (résolution
  paresseuse, mika#1399).
- Un test comportemental sur un vrai `init_agent`.
- Un scan de source tenant la propriété aux deux sites.

**Hors périmètre, explicitement :**
- **Réparer le `docs_root` de mika-litha** — geste opérateur, dit tel quel par le
  ticket. Ce plan rend la cause lisible ; il ne la corrige pas.
- **`server/mod.rs:1060`** (chemin `DEFAULT_AGENT`) — propage par `?`, ne
  journalise rien.
- **`server/state.rs:280-284`** (garde de dérive de tier, mika#1962) — même
  forme, autre source d'erreur, sans contexte `anyhow` empilé ; `%e` y est
  correct.
- **`mika-cli/src/commands/setup.rs:36` et la divergence `main.rs:396` (`{e:#}`)
  contre `main.rs:425` (`{e}`)** — territoire mika#1985, site de propagation et
  non d'émission. Un ticket séparé si l'opérateur veut uniformiser l'imprimeur
  terminal de la CLI.
- **Un champ `site` structuré** sur les deux lignes — refusé en KTD1 : les deux
  littéraux de message portent déjà la distinction.
- **Un helper d'émission partagé / un nouveau module `server/init_failure.rs`** —
  refusé en KTD1.
- **Tout changement de comportement ou de niveau de log** — R2 l'interdit.

## Sources & Research

- Ticket : `senara-solutions/mika#2876`
- Site primaire : `crates/mika-agent/src/server/mod.rs:1079-1108` (boucle de
  démarrage, bras `Err(e)` à 1104)
- Site secondaire : `crates/mika-agent/src/server/state.rs:246-344`
  (`AppState::resolve_agent`, bras `Err(e)` à 336-340)
- Site posant le contexte : `crates/mika-agent/src/server/mod.rs:560-611`
  (`init_agent`, `.with_context` à 610-611)
- Cause interne : `crates/mika-agent/src/kg/config.rs:58-65` (`KgConfigError::PathNotFound`,
  `#[source] std::io::Error`) et `347-359` (la branche qui la construit)
- Idiome de formatage retenu : `crates/mika-common/src/claude.rs:995`,
  `crates/mika-common/src/llm/openai.rs:325`,
  `crates/mika-common/src/llm/ollama.rs:558`
- Harnais de capture réutilisé : `crates/mika-agent/src/server/mod.rs:6424-6527`
  (mika#2680) ; seul site d'installation d'abonné permis :
  `crates/mika-agent/src/test_utils.rs:64-71` (mika#2646)
- Template de scan : `crates/mika-agent/src/canonical_tokens.rs:5240-5302`
  (`mika2474_the_threshold_has_a_single_reader`, son terme `owner_seen`, sa
  clause `RÉSOLUTION :`, son contrôle de bonne foi) et `132` (`production_sources`)
- Cascade de configuration : `crates/mika-common/src/config.rs:2613-2653`
  (`Settings::load_for_agent`) et `3133-3159`
  (`mika2218_per_agent_dotenv_wins_over_process_env_in_server_mode`)
- Clé d'API requise pour atteindre la ligne 611 :
  `crates/mika-common/src/claude.rs:401-416` (`ClaudeClient::new`),
  `config.rs:740, 2710` (`ProviderKind::Anthropic` par défaut)
- Forme de l'`identity.toml` du test : `crates/mika-agent/src/prompt.rs:470-487`
  (`KgIdentityConfig`)
- Plan frère de la même famille (masquage de chaîne d'erreur) :
  `docs/plans/1985-cli-ask-remote-ask-error-chain-masking.md`
- Sémantique de l'`alternate Display` d'`anyhow` : `{e}` rend le contexte le plus
  externe seul, `{e:#}` aplatit la chaîne `source()` entière jointe par `: ` ;
  le sigil `%` de `tracing` est un `Display`, donc équivalent à `{e}`
