---
title: Garde de dérive code↔runtime du modèle des agents bien connus - Plan
type: feat
date: 2026-09-22
issue: 2473
artifact_contract: ce-unified-plan/v1
product_contract_source: issue-body
execution: code
---

# Garde de dérive code↔runtime du modèle des agents bien connus - Plan

> **Ce que ce plan livre, en une phrase.** Quand le modèle sous lequel un agent
> bien connu **tourne** n'est pas celui que `well_known_agents.rs` **déclare**,
> ou quand son `config.toml` a **bougé depuis le boot** sans que le process l'ait
> vu, mika-spirit le dit — au boot, avant le premier tour, sur le journal et sur
> la route budget que mika#2457 a ouverte — et ne refuse rien.

## Goal Capsule

- **Objective.** Un opérateur (ou l'orchestrateur, avant de dispatcher un
  groom) sait, sans lire 19 Go de journal ni comparer deux fichiers à la main,
  si mika-arch / mika-qa / mika-dev servent le modèle que le dépôt déclare, et
  si une édition du `config.toml` attend encore un redémarrage — **avant** que
  la divergence coûte un verdict ou un tour, plus jamais après (classe
  mika#2328 : glm-5.3 en service, `glm-5.2` au dépôt, verdict perdu sur #2327).
- **Means.** Deux détecteurs qui signalent et ne refusent jamais (KTD1) : D1 au
  boot, code-déclaré vs cascade-résolue (KTD2) ; D2 au tour, mtime du
  `config.toml` vs instant du boot (KTD4) — tous deux rendus sur le record
  mika#2457 (KTD3).
- **Authority hierarchy.** Corps du ticket mika#2473 > ce plan > le plan de
  mika#2457 (`docs/plans/2026-09-21-002-fix-2457-verdict-arch-sous-le-plafond-plan.md`,
  dont ce ticket est le suivi nommé) > conventions des crates touchés.
- **Stop conditions.** (1) PR #2461 n'est pas mergée au moment du dispatch —
  le travail ne compile pas contre `main` (voir *Dependencies*) ; (2) le
  scan structurel de mika#2457 (« un seul constructeur du record ») rougit —
  ce plan ne doit introduire aucun second constructeur ; (3) un test existant de
  `budget_provenance.rs` ou `well_known_agents.rs` doit être modifié pour
  passer — aucune valeur de réglage ne bouge ici.
- **Execution profile.** Pilote autonome (claude-pilot) ; PR unique sur `mika`,
  base `main`, après merge de #2461. Aucun changement de valeur, aucune
  migration, aucun refus au boot.

## Product Contract

### Summary

mika-spirit compare, à `init_agent`, le modèle que la constante `config_toml`
de l'agent bien connu déclare avec le modèle que la cascade a résolu pour lui,
et porte le résultat (`in_sync` / `drift` / `not_applicable`) sur
`AgentState`, sur `GET /api/v1/agents/{id}/budget` et sur `mika agents budget`.
À chaque tour, il vérifie que le `config.toml` de l'agent n'a pas changé depuis
le boot ; s'il a changé, il dit ce que le disque porte, ce que le process sert,
et si un redémarrage est requis. Les deux détecteurs émettent un WARN structuré
une fois par condition et **ne refusent rien**.

### Problem Frame

Le dépôt déclare un modèle par agent bien connu (`crates/mika-agent/src/well_known_agents.rs:184-185`,
`:254-255`, `:1527-1528`). Sur ce poste, `MIKA_DISABLE_AGENT_PROVISIONING=1`
(`~/.mika/.env:45`) gèle `reconcile_well_known_config` (`well_known_agents.rs:759`),
et les trois `config.toml` divergent du dépôt depuis des éditions datées à la
main : mika-arch sert `moonshotai/kimi-k3` (dépôt : `kimi-k2.5`), mika-qa sert
`openrouter / z-ai/glm-5.2` (dépôt : `zai / glm-5.2`), mika-dev est en phase
sur le modèle. mika#2328 a mesuré ce que cette dérive coûte quand personne ne la
voit : un modèle jamais calibré (mika#1190) en service, un verdict perdu. Le
doc-comment de `MIKA_QA_CONFIG` (`well_known_agents.rs:242`) le dit en
toutes lettres : *« Nothing here prevents it — this is an instrument, not a
guard. »* mika#2457 a livré la mesure (le record, daté) et a nommé la garde
comme hors périmètre ; ce plan est cette garde.

Le second mécanisme est nommé par le corps du ticket dans sa définition même
du défaut — *« le modèle effectif au runtime peut diverger du modèle attendu par
le code (config figée au boot, **flip non rechargé à chaud** — cf.
`feedback_agent_config_flip_needs_restart`) »* — et c'est cette parenthèse qui
fait autorité pour D2 (R8–R9) : une édition du `config.toml` **après** le boot
n'entre en service qu'au redémarrage
(`feedback_agent_config_flip_needs_restart_read_model_in_turn_usage`). Le siège
TUI affiche le disque ; `turn_usage.model` affiche le process ; rien ne signale
l'écart entre les deux tant qu'un humain ne les compare pas. Une garde qui ne
verrait que D1 laisserait cette moitié de la classe silencieuse.

### Requirements

**Détection D1 — code ↔ runtime (au boot)**

- R1. Pour tout agent bien connu dont la spec porte `config_toml: Some(_)`,
  spirit établit à `init_agent` si le couple `(provider, modèle)` résolu par la
  cascade est celui que la constante déclare, et fixe le résultat sur
  `AgentState` pour la durée du process — même contrat *not hot-swappable* que
  `budget_record` (mika#2457 U2).
- R2. Le côté « déclaré » est lu **par la même dérivation de clé** que le côté
  résolu (`model_config_key(provider)`, `budget_provenance.rs:368`) et applique
  la même règle de défaut (`provider.default_model()` quand la constante ne pose
  pas de modèle). Un second parseur serait libre de diverger du premier.
- R3. Le côté « résolu » est le record mika#2457 (`ResolvedBudgetRecord.provider`,
  `.model`, `.model_source`) — donc **la cascade entière**, pas le seul fichier :
  un `MIKA_OPENROUTER_MODEL` en environnement de service ou dans le `.env`
  per-agent est une dérive au même titre qu'une édition du `config.toml`, et
  elle est rapportée avec sa porte.
- R4. Sur dérive, spirit émet **un** WARN `well_known_model_drift` par agent et
  par init, portant `agent_id`, `declared_provider`, `declared_model`,
  `runtime_provider`, `runtime_model`, `runtime_model_source`,
  `runtime_provider_source`, `model_config_key`, et
  `declared_by = "well_known_agents.rs"`. En phase, un INFO
  `well_known_model_in_sync` — pour qu'une absence de WARN soit distinguable
  d'une garde qui n'a pas tourné. Un agent sans constante n'émet rien.
  **« Aux mêmes champs » se borne à ceux que l'arm porte** (tranché à
  l'implémentation) : `InSync` ne déclare que le couple déclaré, donc la ligne
  porte `agent_id`, `declared_provider`, `declared_model`, `declared_by` — et
  c'est tout ce qui existe. Les `*_source` et `model_config_key` n'ont **aucune
  source honnête** sur ce bras, `emit_model_drift` ne recevant que le check ;
  les inventer serait la fausse provenance que `budget_provenance.rs` refuse en
  toutes lettres. L'enum d'U1 gouverne, parce que c'est lui que la route et le
  CLI sérialisent (KTD3), et l'intention de R4 — rendre la garde audible — est
  servie par la ligne elle-même, pas par son nombre de champs.
- R5. La garde **ne refuse jamais** : ni au boot, ni au tour. La dérive est
  rapportée avec sa provenance ; la décision (restaurer, recalibrer, ou
  réconcilier la constante) reste à l'opérateur, record en main.

**Surface — route et CLI**

- R6. `GET /api/v1/agents/{id}/budget` rend, à côté de `budget`, une clé
  `model_drift` portant le résultat de R1 sérialisé
  (`status ∈ {in_sync, drift, not_applicable}` + les champs de R4 quand ils
  existent). La route ne relit pas le disque et ne recalcule rien (mika#2457 U3
  inchangé). 404 inchangé.
- R7. `mika agents budget` rend `model_drift` en texte (une ligne `code …`
  après la ligne `model`), en JSON et en YAML. Un spirit qui ne sert pas la
  clé (binaire antérieur à ce correctif) est rendu *« dérive non évaluée par ce
  serveur »*, jamais comme `in_sync`.

**Détection D2 — disque ↔ process (au tour)**

- R8. À chaque tour, avant le premier appel LLM, spirit compare le mtime du
  `{agent_home}/config.toml` à celui noté à `init_agent`. S'il a changé et n'a
  pas encore été rapporté, spirit re-résout le record **depuis le disque** (sans
  toucher `AgentState.budget_record`), le compare au record du boot hors
  `resolved_at`, et émet une fois par mtime : WARN
  `agent_config_changed_since_boot` avec `restart_required = true` **ssi un
  champ du record budget/modèle de mika#2457 diffère**, INFO avec
  `restart_required = false` sinon. Le bras `false` n'est **pas** « rien n'a
  changé » : `ResolvedBudgetRecord` ne porte ni `openrouter_base_url`, ni
  `zai_base_url`, ni `log_level`, donc une édition de ces clés — un geste réel
  sur ces agents, et qui change l'endpoint servi — y tombe. Le message dit donc
  *« aucun champ du record budget/modèle n'a bougé — un autre champ du fichier
  peut néanmoins exiger un redémarrage »*, jamais *« aucun champ effectif n'a
  bougé »* : la seconde formulation ferait rester un opérateur sur l'ancien
  endpoint en lui disant que tout va bien. Champs : `agent_id`, `config_mtime` (RFC 3339 UTC),
  `provider_on_disk`, `model_on_disk`, `provider_in_service`,
  `model_in_service`, `budget_changed`.
- R9. D2 coûte un `stat` par tour et **une** re-résolution par mtime distinct ;
  il ne lit ni ne relit rien tant que le mtime n'a pas bougé. D2 évalue **tout
  tour qui passe par `load_agent_context`, runs d'équipe compris** — c'est la
  conséquence directe de KTD5, dont l'entonnoir unique inclut `:6590`, qui est
  `run_team_agent_inner_impl` — et ne déclenche rien pour un agent dont ce
  process ne détient **aucune note de boot**, `BOOT_NOTES` étant clé par
  `agent_id` seul. La population exempte est donc l'agent d'équipe que ce
  process n'a jamais passé à `init_agent`, et non « les runs d'équipe ».

### Scope Boundaries

- **Refuser le boot ou le tour sur dérive** — hors périmètre par principe
  (R5). mika#1190 interdit un swap de modèle sans calibration, mais le swap a
  déjà eu lieu ; refuser coucherait la flotte sur une décision opérateur datée.
- **Le budget (`llm_max_tokens`, plafond, enveloppe) dans D1** — hors
  périmètre. Ces clés ont des portes d'environnement légitimes
  (`MIKA_LLM_HTTP_TIMEOUT_SECS=300` est en service ici) ; une comparaison
  constante-vs-runtime tirerait à chaque boot sur un réglage voulu. Le record
  mika#2457 les rapporte déjà avec leur porte. mika-dev diverge sur
  `llm_max_tokens` (8192 au dépôt, 32768 sur disque) : mesuré par le record,
  pas par cette garde.
- **Le modèle que le fournisseur a réellement servi** (champ `model` de la
  réponse HTTP) — hors périmètre. `LlmResponse` (`crates/mika-common/src/llm/types.rs:185`)
  ne le porte pas ; `turn_usage.model` est `llm.model_name()`
  (`agent_loop/mod.rs:917`, `:1580`), c'est-à-dire le modèle **demandé** par le
  process. C'est ce que le ticket appelle « réellement servi », et ce plan s'y
  tient (KTD2).
- **Les overrides per-skill** (`make_provider_for`, `config.rs:1857`) — hors
  périmètre, même angle mort que mika#2293 et mika#2457 : la question est le
  modèle *nominal* de l'agent.
- **L'override de modèle par l'appelant** (`mika.model_override` /
  `mika ask --model`, mika#2304 ; `server/a2a.rs::caller_model_provider` `:621`,
  qui construit son provider par le même `make_provider_for`) — hors périmètre
  pour la même raison, et **nommé** parce que le pré-vol est une pratique vivante
  sur ce poste : le `config.toml` de mika-arch porte une campagne `arch-probe`
  datée du 2026-09-18. L'équivalence de KTD2 est énoncée pour les tours **sans**
  override d'appelant ; sur un tour qui en porte un, `mika agents budget` dira
  `in_sync` pendant que le `turn_usage` du tour portera un troisième modèle, et
  les deux auront raison.
- **Le `.env` per-agent dans D2** — hors périmètre : D2 surveille le fichier
  que les trois éditions datées de ce poste ont touché. Le `.env` reste couvert
  par D1 au boot (R3).
- **Réconcilier les constantes avec le runtime** (k3 pour mika-arch, etc.) —
  c'est mika#2472 (baseline `calibrate-mika-arch`), pas ici. Ce plan rend la
  dérive lisible ; il ne choisit pas de quel côté la résoudre.
- **Une relecture à chaud des `Settings`** — hors périmètre. Le gel au boot est
  le contrat (mika#1962, mika#2290, mika#2457 U2) ; D2 le rend visible, il ne
  l'abolit pas.

### Dependencies

- **PR #2461 (mika#2457) mergée d'abord.** Ce plan s'appuie sur
  `ResolvedBudgetRecord` / `resolve_llm_budget_record` / `emit_llm_budget_resolved`
  (`crates/mika-common/src/llm/budget_provenance.rs`), `AgentState.budget_record`
  (`crates/mika-agent/src/server/state.rs`), `dashboard::handle_agent_budget` et
  la route `/agents/{id}/budget` (`server/mod.rs`, `server/dashboard.rs`), et la
  sous-commande `budget()` + son miroir local `BudgetRecord`
  (`crates/mika-cli/src/commands/agents.rs`). Vérifié contre la tête `c72e40ea`
  de `feat/2457/p1-substrat-mika-arch-verdict-sous-le`. Le ticket porte
  `blockedBy mika#2457` ; la garde de dispatch (`validate_dispatch_readiness`,
  check 6) tient le dispatch tant que #2457 est ouvert, et dispatch-lib rebase
  la branche sur `origin/main` au dispatch.
- **Levée, et mesurée.** PR #2461 est **mergée** (2026-09-22T10:21:29Z), mika#2457
  fermé ; la branche de ce ticket est rebasée sur `origin/main` @ `9aa39998`.
  Le contrôle promis ci-dessous a été fait avant `/ce:work`, contre le `main`
  post-merge et non contre `c72e40ea` : **aucun nom n'a bougé**. Les quatorze
  symboles dont ce plan dépend existent avec la forme attendue —
  `ResolvedBudgetRecord` (`:649`, dérive `Debug, Clone, PartialEq, Eq, Serialize,
  Deserialize` — load-bearing pour la comparaison hors `resolved_at` d'U2 et pour
  KTD3), `resolve_llm_budget_record(agent_id, global_home, agent_home)` (`:711`),
  `emit_llm_budget_resolved` (`:849`), `AgentState.budget_record` (`state.rs:129`),
  `handle_agent_budget` (`dashboard.rs:315`, rendant aujourd'hui
  `json!({ "budget": … })` — le point d'extension d'U3/R6), `budget()` (`:95`),
  `BudgetRecord` (`:144`), `render_budget_text` (`:172`), `render_unattested`
  (`:290`), `sample()` (`:1293`), `ProviderKind::ALL` / `config_prefix` /
  `default_model` (`llm/mod.rs:581`, `:598`, `:687`), `find_well_known_agent`
  (`:823`), `load_agent_context` (`:478`).
- **Ce qui a bougé, ce sont les numéros de ligne — dans les deux fichiers que
  #2461 a touchés, et là seulement.** À rectifier en lisant, non en recopiant les
  ancres de la section *Sources* : `init_agent` `:446 → :450`, `run_server`
  `:757 → :775`, l'appel `budget_guard` `:822 → :840`, `AppState.agents`
  `:120 → :142` ; dans `budget_provenance.rs`, `homes()` `:811 → :965`
  (`clean_budget_env` `:953`), `log_llm_budget_resolved` `:673 → :814`, dédup
  `LAST_EMITTED` `:601`, et les tests cités `mika2293_dedup_silences_repetition_and_re_emits_a_change`
  `:1059 → :1213`, `mika2328_the_model_key_is_the_one_settings_reads_for_every_provider`
  `:1404 → :1556`, `mod tests` `:836 → :939`. Le second site de
  `log_llm_budget_resolved` est `teams/engine.rs:222` (plan : `:217`). **Et
  c'est le mauvais site pour juger R9** — corrigé dans R9 ci-dessus après
  vérification : D2 ne vit pas là mais dans `load_agent_context`, que le chemin
  d'équipe traverse bel et bien (`:6590` est dans `run_team_agent_inner_impl`,
  `agent_loop/mod.rs:6571`). L'absence d'`AgentState` à ce site n'exempte donc
  rien ; ce qui exempte, c'est l'absence de note de boot. Les sites `llm.model_name()` cités `:917, :1580, :1605` sont
  aujourd'hui `:773, :810, :840` et `:1586, :1611` ; `emit_turn_usage` `:8489` est
  **exact**.
- **Sont exactes et n'ont pas bougé** : tout `well_known_agents.rs` (`config_toml`
  `:62`, `MIKA_DEV_CONFIG` `:181`, `MIKA_QA_CONFIG` `:251`, `MIKA_ARCH_CONFIG`
  `:1524`, `reconcile_well_known_config` `:759`, `find_well_known_agent` `:823`,
  `provision_well_known_agents` `:884`, `WELL_KNOWN_AGENTS` `:477`) et tout
  `agent_loop/mod.rs` (`load_agent_context` `:478` et ses trois appelants `:4647`,
  `:5713`, `:6590` — KTD5 tient tel quel), plus `AgentState` `:27`,
  `model_config_key` `:368`, `config_key_as_string` `:588`, `ModelProvenance`
  `:283`, `effective_model` `:333`, `CascadeLayers::read` `:503`,
  `MODEL_SOURCE_UNKNOWN_PROVIDER` `:164`, `make_llm_provider` `:1833`,
  `make_provider_for` `:1857`, `LlmResponse` `:185` — qui **ne porte toujours pas**
  de champ `model` de réponse, ce qui maintient la frontière de périmètre que
  KTD2 pose.
- **Une correction de fond, en faveur de KTD1.** La phrase citée par KTD1 — *« a
  warning that contradicts a decision gets muted »* — n'est pas en `:697-704`
  mais en **`:739-742`**, et son contexte réel la **renforce** : *« A guard firing
  on the declaration would contradict a documented decision **at every startup**,
  and a warning that contradicts a decision gets muted. What earns an operator's
  attention is a *crossing*. »* C'est mot pour mot la situation de D1 sur ce poste
  — trois agents, une émission par boot — et c'est ce qui justifie que D1 émette
  **une ligne par agent et par init** (R4) et non par tour, et que la
  *Fire-Disposition* refuse l'allowlist plutôt que de museler. Voisin utile pour
  U2, à lire avant d'écrire la dédup par mtime : `:895-897`, qui explique pourquoi
  une émission est placée **après** le `return` de dédup.
- Le scan « un seul constructeur » de mika#2457 est
  `mika2457_the_record_has_a_single_construction_site` (`:1910`) ; il exclut déjà
  `pub struct` et `-> ResolvedBudgetRecord {` (`:1941-1946`). U2 **appelle** le
  constructeur sans en créer un second : le scan reste vert, allowlist vide,
  conformément à la *Fire-Disposition*. Les deux tests de route à garder verts
  sont `mika2457_budget_route_serves_the_record_or_404s` (`server/mod.rs:4501`) et
  `mika2457_the_route_serves_the_record_of_the_init_not_the_disk` (`:4577`), et
  `test_state_full` est en `server/mod.rs:2102` (appelé `:2092`, `:4586`).

## Planning Contract

### Key Technical Decisions

- KTD1. **Signaler, jamais refuser.** Les deux détecteurs émettent et rendent ;
  aucun `bail!`, aucun `Err` sur dérive. Raison : la dérive présente est une
  suite de décisions opérateur datées dans les trois `config.toml` ; un refus
  ferait de `budget_guard` (`server/budget_guard.rs`, qui refuse une paire
  *invalide*) le modèle d'une garde qui refuserait une configuration *valide*.
  Le motif « a warning that contradicts a decision gets muted »
  (`budget_provenance.rs:739-742`) tranche : une ligne par condition, jamais
  par tour. Gouverne R5.
- KTD2. **Le côté runtime de D1 est le record mika#2457, pas une relecture.**
  `turn_usage.model` = `llm.model_name()` = `Settings::active_llm_config().model`
  (`config.rs:1834-1837`) = `ModelProvenance::effective_model()`, égalité
  épinglée pour chaque fournisseur par
  `mika2328_the_model_key_is_the_one_settings_reads_for_every_provider`
  (`budget_provenance.rs:1404`). Donc « lu dans `turn_usage` » et « lu sur le
  record de l'init » sont **la même valeur** pour le fournisseur nominal, et
  comparer au boot est comparer ce que chaque tour rapportera. Gouverne R1, R3.
- KTD3. **Le résultat de D1 est un sibling du record, pas un champ du record.**
  `ModelDriftCheck` vit dans `mika-common` (pour que la route et le CLI le
  sérialisent sans dépendre de `mika-agent`), mais il est **construit dans
  `mika-agent`**, seul crate qui connaît `WellKnownAgent`. Le record mika#2457
  garde son constructeur unique et sa signature de dédup ; le scan structurel
  de #2457 reste vert sans allowlist. La route rend `{ budget, model_drift }`.
  Gouverne R6, R7.
- KTD4. **D2 surveille le mtime, pas le contenu.** Un `stat` par tour, une
  re-résolution par mtime distinct, dédup dans un `OnceLock<Mutex<HashMap>>`
  frère de `LAST_EMITTED` (`budget_provenance.rs`). La note de boot (mtime,
  `global_home`, record cloné) est prise à `init_agent`, là où le record est
  résolu — pas au premier tour, qui pourrait déjà être postérieur à une
  édition. Gouverne R8, R9.
- KTD5. **Le site de D2 est `load_agent_context`** (`agent_loop/mod.rs:478`),
  l'entonnoir unique des trois boucles (`:4647`, `:5713`, `:6590`), qui tient
  déjà `db.agent_id()` et `home_dir`. Une garde posée dans une seule boucle
  serait aveugle aux deux autres. Gouverne R8.
- KTD6. **Le côté déclaré est lu par `declared_model(&str)` dans
  `budget_provenance.rs`**, à partir de `config_key_as_string` (`:588`) et
  `model_config_key` (`:368`) existants, sans passer par `CascadeLayers` : le
  walk de la cascade lit l'environnement du process (`resolve_raw`, `:528`), et
  la constante du dépôt n'a **aucune** porte d'environnement. Équivalence
  épinglée (U1) : écrire la constante dans un `agent_home` vierge, sans
  variable, et résoudre par la cascade rend le même couple. Gouverne R2.

### High-Level Technical Design

```
boot (server/mod.rs::init_agent, après resolve_llm_budget_record)
  declared = find_well_known_agent(name).and_then(|s| s.config_toml).and_then(declared_model)
  check    = ModelDriftCheck::compare(declared.as_ref(), &budget_record)     [mika-common, pur]
  emit     : WARN well_known_model_drift | INFO well_known_model_in_sync | rien
  note_config_at_boot(name, global_home, agent_home, &budget_record)          [D2, mika-common]
  AgentState { budget_record, model_drift: Arc<ModelDriftCheck>, .. }

tour (agent_loop/mod.rs::load_agent_context)
  detect_config_change(db.agent_id()) -> Option<ConfigChangedSinceBoot>
    stat mtime == noté ? None : (déjà rapporté ? None : re-résoudre, comparer, noter, Some)
  report_config_change(&finding)  : WARN si restart_required, INFO sinon

route GET /api/v1/agents/{id}/budget  ->  { "budget": record, "model_drift": check }
CLI  mika agents budget               ->  ligne `code … (well_known_agents.rs)` + JSON/YAML
```

### Sequencing

U1 → U2 → U3 → U4 → U5 → U6. U1 et U2 sont `mika-common` et ne dépendent que de
#2461 ; U3/U4 sont `mika-agent` ; U5 est `mika-cli` ; U6 est la documentation.
La logique de comparaison d'U4 (D2) est indépendante de celle d'U3 (D1), mais
U4 **dépend de U3** pour la note de boot : `detect_config_change` rend `None`
quand aucune note n'existe, donc D2 est inerte tant qu'U3 n'appelle pas
`note_config_at_boot` à `init_agent`.

## Implementation Units

### U1. `declared_model` et `ModelDriftCheck` — le côté déclaré et la comparaison (mika-common)

- **Goal.** Lire ce qu'une constante `config_toml` déclare, avec la dérivation
  de clé existante, et comparer purement au record.
- **Requirements.** R2, R3, R4, R5 (pas d'effet de refus), KTD1, KTD3, KTD6.
- **Files.** `crates/mika-common/src/llm/budget_provenance.rs`,
  `crates/mika-common/src/llm/mod.rs` (ré-export).
- **Approach.**
  - `pub struct DeclaredModel { pub provider: ProviderKind, pub model: String, pub model_is_provider_default: bool }`
    et `pub fn declared_model(config_toml: &str) -> Option<DeclaredModel>` :
    parse en `toml::Table` ; `llm_provider` **trimmé avant `from_str`**, comme
    `ModelProvenance::from_layers` (`:304`) le fait de son côté — sans quoi une
    future constante écrite avec une espace parasite ferait rendre `None` ici et
    un couple valide là-bas, c'est-à-dire un `Drift` **faux** rapporté avec
    `unknown_provider` sur une configuration correcte, que le test d'U1 ne
    verrait pas puisqu'il écrit son propre texte sans padding ; absent ⇒
    `DEFAULT_PROVIDER` ;
    présent mais illisible ⇒ `None` (même règle que `ModelProvenance::from_layers`,
    `:308-313`) ; modèle = `config_key_as_string(&table, &model_config_key(p))`
    ou `p.default_model()`.
  - `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] #[serde(tag = "status", rename_all = "snake_case")]
    pub enum ModelDriftCheck { NotApplicable, InSync { declared_provider, declared_model }, Drift { declared_provider, declared_model, runtime_provider, runtime_model, runtime_model_source, runtime_provider_source, model_config_key } }`
    (champs `String`), constante `pub const DECLARED_BY: &str = "well_known_agents.rs"`.
  - `impl ModelDriftCheck { pub fn compare(declared: Option<&DeclaredModel>, record: &ResolvedBudgetRecord) -> Self }` :
    `None` ⇒ `NotApplicable` ; sinon `Drift` ssi `record.provider != declared.provider.config_prefix()`
    **ou** `record.model != declared.model`. Un `record.model == ""`
    (`unknown_provider`, `:164`) est une dérive rapportée avec
    `runtime_model_source = unknown_provider`, jamais absorbée.
  - `pub fn emit_model_drift(agent_id: &str, check: &ModelDriftCheck)` — **le
    site unique d'émission de D1**, posé ici et non dans `mika-agent`, à côté
    d'`emit_llm_budget_resolved` (`:849`) dont il est le frère : WARN
    `well_known_model_drift` sur `Drift`, INFO `well_known_model_in_sync` sur
    `InSync`, **rien** sur `NotApplicable`, portant les champs de R4 plus
    `declared_by = DECLARED_BY`. La comparaison (`compare`) reste pure ;
    l'émission est la seule fonction de cette unité qui a un effet, et KTD1
    tient parce qu'elle n'a que celui-là — aucun `bail!`, aucun `Err`.
  - Doc-comment du module : une section *« The guard mika#2328 said was
    missing »* qui renvoie ici depuis le paragraphe existant sur `turn_usage`.
- **Test Scenarios.**
  - `mika2473_declared_model_equals_the_cascade_on_a_clean_home` : pour chaque
    `ProviderKind::ALL`, écrire `llm_provider = "<p>"\n<p>_model = "probe"` dans
    un `agent_home` vierge (helper `homes()`, `:811`, env nettoyé par
    `clean_budget_env`, `#[serial]`), asserter
    `declared_model(text) == (ModelProvenance::resolve(..).provider, .effective_model())` ;
    puis le texte sans ligne modèle ⇒ `model == p.default_model()` et
    `model_is_provider_default` ; puis `llm_provider = "nope"` ⇒ `None`.
  - `mika2473_compare_has_three_arms_and_unknown_provider_is_a_drift` : `None`
    ⇒ `NotApplicable` ; record construit sur un home où la constante a été
    écrite ⇒ `InSync` ; même home avec `openrouter_model` réécrit ⇒ `Drift`
    portant les deux valeurs et `runtime_model_source = agent_config` ; home
    avec `llm_provider = "nope"` ⇒ `Drift` avec `runtime_model_source = unknown_provider`.
  - `mika2473_a_model_set_by_env_is_a_drift_with_its_door` : constante en phase
    sur disque, `MIKA_<P>_MODEL` posé ⇒ `Drift` avec `runtime_model_source = process_env`
    (contrôle : sans la variable, `InSync` — les deux dans le même test).
  - `mika2473_the_drift_line_carries_its_level_and_its_fields` — **le seul test
    qui observe AC3**, sans lequel « la dérive est dite une fois, avec sa porte »
    n'est attesté par rien d'autre qu'un `grep` post-déploiement que le plan
    lui-même dit confondable avec un binaire périmé. Installer une couche
    `tracing_subscriber` capturante (motif déjà en place dans ce dépôt,
    `builtin_handlers.rs:9716`), appeler `emit_model_drift` sur les trois bras
    et asserter : sur `Drift`, le nom d'événement `well_known_model_drift`, le
    niveau **WARN** et les huit champs de R4 plus
    `declared_by = "well_known_agents.rs"` ; sur `InSync`, le nom
    `well_known_model_in_sync` et le niveau **INFO** ; sur `NotApplicable`,
    **aucune ligne** — ce troisième bras est le contrôle négatif, et sans lui le
    test ne distingue pas « n'émet rien » de « émet toujours ».
- **Verification.** `cargo test -p mika-common -- llm::budget_provenance` vert,
  **compte de tests non nul** ; tous les tests `mika2293_*`, `mika2328_*`,
  `mika2457_*` existants inchangés.

### U2. `config_freshness` — la note de boot et la détection au tour (mika-common)

- **Goal.** Savoir, sans relire tant que rien n'a bougé, si le `config.toml`
  d'un agent a changé depuis le boot et si le process en sert encore l'ancien
  contenu.
- **Requirements.** R8, R9, KTD4.
- **Files.** `crates/mika-common/src/llm/config_freshness.rs` (nouveau),
  `crates/mika-common/src/llm/mod.rs` (module + ré-exports).
- **Approach.**
  - `struct BootNote { mtime: Option<SystemTime>, global_home: PathBuf, agent_home: PathBuf, record: ResolvedBudgetRecord, reported: Option<SystemTime> }`
    dans `static BOOT_NOTES: OnceLock<Mutex<HashMap<String, BootNote>>>`.
  - `pub fn note_config_at_boot(agent_id, global_home, agent_home, record: &ResolvedBudgetRecord)` :
    `std::fs::metadata(agent_home/config.toml).and_then(|m| m.modified()).ok()`.
  - **Un `stat` illisible sort le tour de la population — il n'est jamais un
    terme satisfait.** Écrit ici parce que la lecture littérale du reste fait
    l'inverse en silence : `mtime` et `reported` sont tous deux
    `Option<SystemTime>` et `reported` démarre à `None`, donc un `stat` qui
    échoue rend `None`, égale `reported`, et se lit « déjà rapporté » — un
    `config.toml` présent au boot puis supprimé ou rendu illisible ne serait
    jamais signalé. La maison tranche l'inverse deux fois : mika#2277 sort du
    périmètre un signal de vivacité illisible sous son propre nom, mika#2328
    donne à un fournisseur illisible le mot distinct `unknown_provider` plutôt
    que de le fondre dans `default`. Donc : lecture impossible ⇒ `None`,
    `reported` **intact**, et une ligne nommée `agent_config_mtime_unreadable`
    (WARN, **régime attendu : zéro**) ; et `reported` prend une représentation
    qui ne peut pas égaler une lecture manquée.
  - `pub struct ConfigChangedSinceBoot { agent_id, config_mtime: String /* RFC 3339 UTC */, provider_on_disk, model_on_disk, provider_in_service, model_in_service, budget_changed: bool, restart_required: bool }`.
  - `pub fn detect_config_change(agent_id: &str) -> Option<ConfigChangedSinceBoot>` :
    pas de note ⇒ `None` ; mtime courant == noté ⇒ `None` ; == `reported` ⇒
    `None` ; sinon `resolve_llm_budget_record(agent_id, &global_home, &agent_home)`
    (le constructeur unique de #2457, appelé — pas dupliqué), comparaison au
    record du boot avec `resolved_at` vidé des deux côtés ⇒ `restart_required` ;
    `budget_changed` = l'un de `http_timeout_secs`, `agent_total_timeout_secs`,
    `llm_max_tokens` diffère ; noter `reported = mtime` ; `Some`.
  - `pub fn report_config_change(finding: &ConfigChangedSinceBoot)` : WARN
    `agent_config_changed_since_boot` si `restart_required`, INFO sinon, message
    nommant le geste (*« restart mika-spirit to apply »* / *« no effective
    field moved »*).
  - `#[cfg(test)] fn reset_notes_for_test()`.
  - Le `stat` est synchrone (`std::fs`), comme les lectures de `CascadeLayers::read` ;
    une métadonnée, pas un contenu.
- **Test Scenarios.** (`#[serial]`, env nettoyé, `homes()`-like tempdir)
  - `mika2473_an_unchanged_config_reports_nothing` : note puis detect ⇒ `None` ;
    agent inconnu ⇒ `None`.
  - `mika2473_a_touch_is_reported_once_without_restart` : réécrire le même
    contenu et forcer un mtime distinct (`File::set_modified`) ⇒ `Some` avec
    `restart_required = false`, `budget_changed = false` ; second detect ⇒ `None`.
  - `mika2473_a_model_edit_requires_a_restart_and_names_both_sides` : réécrire
    avec un autre `openrouter_model` (mtime forcé distinct) ⇒ `Some` avec
    `restart_required = true`, `model_on_disk` = nouvelle valeur,
    `model_in_service` = valeur du boot ; puis réécrire avec un autre
    `llm_max_tokens` ⇒ `budget_changed = true`.
  - Contrôle négatif : `AgentState`-side non concerné ici, mais le test asserte
    que le record **noté** est inchangé après detect (D2 ne remplace rien).
- **Verification.** `cargo test -p mika-common llm::config_freshness` vert.

### U3. D1 à `init_agent` — comparer, émettre, garder sur `AgentState` (mika-agent)

- **Goal.** Établir la dérive code↔runtime là où le record est résolu, avant le
  premier tour, et la garder pour la route.
- **Requirements.** R1, R4, R5, R6, R8 (la note de boot, sans laquelle D2 est inerte), KTD1, KTD2, KTD3.
- **Files.** `crates/mika-agent/src/server/mod.rs` (`init_agent`, `:446` ; après
  la ligne `emit_llm_budget_resolved(&budget_record)` de #2461),
  `crates/mika-agent/src/server/state.rs` (`AgentState`, `:27` ;
  `test_state_full`), `crates/mika-agent/src/server/dashboard.rs`
  (`handle_agent_budget`), `crates/mika-agent/src/well_known_agents.rs`
  (doc-comments de `MIKA_DEV_CONFIG` `:165-180` et `MIKA_QA_CONFIG` `:229-244`).
- **Approach.**
  - Dans `init_agent` : `let declared = crate::well_known_agents::find_well_known_agent(agent_name).and_then(|s| s.config_toml).and_then(mika_common::llm::declared_model);`
    `let model_drift = ModelDriftCheck::compare(declared.as_ref(), &budget_record);`
    puis `mika_common::llm::emit_model_drift(agent_name, &model_drift)` — **livré
    et testé par U1**, appelé ici, jamais redéfini (R4) ; puis
    `note_config_at_boot(agent_name, global_home, agent_home, &budget_record)` (U2) ;
    puis `model_drift: Arc::new(model_drift)` sur `AgentState`.
  - `handle_agent_budget` : `json!({ "budget": &*agent_state.budget_record, "model_drift": &*agent_state.model_drift })`.
    404 inchangé, doc-comment complété d'un paragraphe *« and the drift, frozen
    with it »*.
  - `well_known_agents.rs` : remplacer dans les deux doc-comments la phrase
    *« Nothing here prevents it — this is an instrument, not a guard »* par le
    renvoi à `well_known_model_drift` (mika#2473) et à la commande de lecture ;
    aucune constante ne change.
- **Test Scenarios.**
  - `mika2473_a_freshly_provisioned_well_known_agent_has_no_drift_and_an_edit_has` (`well_known_agents.rs`, `#[serial]`, env `MIKA_*_MODEL` nettoyé) :
    pour chaque spec de `WELL_KNOWN_AGENTS` avec `config_toml: Some`,
    `declared_model(config_toml).is_some()` (les trois constantes déclarent un
    couple lisible) ; `provision_well_known_agents(home, settings, false)` —
    la précondition « env `MIKA_*_MODEL` nettoyé » a besoin d'un helper que
    `budget_provenance.rs` garde privé à son module de test (`clean_budget_env`
    `:953`) : l'exporter sous la feature `test-utils` dont `mika-agent` dépend
    déjà, plutôt que de ré-écrire `format!("MIKA_{}_MODEL", …)` ici — ce serait
    la seconde copie d'une dérivation dont le doc-comment dit qu'une table
    serait un second endroit où se tromper —
    **avec `test_settings_with_kg_roots()`** (`well_known_agents.rs:1585`) et
    `home/agents` créé au préalable, faute de quoi ce test ne mesure pas ce
    qu'il prétend : `build_mika_arch_identity` (`:412-422`) rend `Err` quand
    `kg_docs_roots` est absent, le provisionnement `continue` sans écrire de
    `config.toml`, et mika-arch — l'agent même sur lequel la *Fire-Disposition*
    est bâtie — résout alors la cascade vide (`DEFAULT_PROVIDER`, anthropic) et
    rend `Drift` là où le contrôle positif attend `InSync`. Le saut est déjà
    épinglé par `test_provision_skips_mika_arch_when_kg_docs_roots_unset`
    (`:1679`). **Une spec sautée au provisionnement est un défaut de montage du
    test, jamais une dérive à asserter** ;
    `compare(declared, resolve_llm_budget_record(name, home, agent_home))` ⇒
    `InSync` (contrôle positif) ; réécrire `<p>_model` dans le `config.toml`
    provisionné ⇒ `Drift` (contrôle négatif, même test). `MIKA_TEST`
    (`config_toml: None`) ⇒ `NotApplicable`.
  - `mika2473_the_budget_route_serves_the_drift_beside_the_record` (`server/mod.rs` tests) :
    `test_state_full` étendu d'un paramètre `model_drift` ; état avec
    `Drift{..}` ⇒ `json["model_drift"]["status"] == "drift"` et les deux modèles
    présents ; état avec `NotApplicable` ⇒ `"not_applicable"` ; 404 ⇒ ni
    `budget` ni `model_drift`. **Plus un bras de gel, sans lequel la seconde
    moitié d'AC4 n'est attestée par rien** : réécrire le modèle dans le
    `config.toml` de l'agent **après** la construction de l'état, prouver que le
    disque a bougé, puis asserter que `json["model_drift"]` est identique à sa
    valeur d'avant la mutation. Miroir exact de
    `mika2457_the_route_serves_the_record_of_the_init_not_the_disk`
    (`server/mod.rs:4577`), qui fait déjà ce geste pour le record — et qui ne
    peut pas l'attester pour le sibling, puisqu'il n'indexe que
    `json["budget"]["http_timeout_secs"]` et que ses assertions ne bougent pas.
  - Non-régression : `mika2457_budget_route_serves_the_record_or_404s` et
    `mika2457_the_route_serves_the_record_of_the_init_not_the_disk` verts sans
    modification de leurs assertions.
- **Verification.** `cargo test -p mika-agent -- well_known_agents server::` vert, compte non nul.

### U4. D2 dans `load_agent_context` — un `stat` par tour (mika-agent)

- **Goal.** Rendre structurel ce que la mémoire opérateur fait à la main : au
  premier tour après une édition, dire ce que le disque porte et ce que le
  process sert.
- **Requirements.** R8, R9, KTD4, KTD5.
- **Files.** `crates/mika-agent/src/agent_loop/mod.rs` (`load_agent_context`, `:478`).
- **Approach.** En tête de `load_agent_context`, avant les lectures existantes :
  `if let Some(finding) = mika_common::llm::detect_config_change(db.agent_id()) { mika_common::llm::report_config_change(&finding); }`
  avec un commentaire nommant le contrat (gel au boot mika#1962/#2457 U2) et le
  coût (un `stat`). Aucun autre site : les trois boucles passent ici.
- **Test Scenarios.** La logique est épinglée en U2 sur la primitive (motif de
  `mika2293_dedup_silences_repetition_and_re_emits_a_change`, `:1059`). Ici,
  un test structurel dans `agent_loop/mod.rs` :
  `mika2473_the_freshness_check_sits_in_the_one_funnel` — le source de
  `agent_loop/mod.rs` contient exactement **un** appel à `detect_config_change(`
  et il est dans `load_agent_context` (scan de source, même famille que
  `mika1883_run_usage_accumulates_only_via_the_one_helper`). Population
  pré-existante : zéro appel à HEAD.
- **Verification.** `cargo test -p mika-agent agent_loop::` vert ; clippy vert.

### U5. `mika agents budget` rend la dérive (mika-cli)

- **Goal.** L'opérateur lit la dérive sur la surface qu'il consulte déjà, et un
  vieux spirit ne passe jamais pour « en phase ».
- **Requirements.** R7, KTD3.
- **Files.** `crates/mika-cli/src/commands/agents.rs` (`budget()`,
  `BudgetRecord`, `render_budget_text`, `render_unattested`).
- **Approach.**
  - Miroir local `#[derive(Serialize, Deserialize)] struct ModelDrift { status: String, declared_provider: Option<String>, declared_model: Option<String>, runtime_provider: Option<String>, runtime_model: Option<String>, runtime_model_source: Option<String> }`
    (miroir, pas import : même raison que `BudgetRecord`, versions croisées).
  - `budget()` lit `v["model_drift"]` en `Option<ModelDrift>` (absent ou
    illisible ⇒ `None`) et le porte dans un champ
    `#[serde(default)] model_drift: Option<ModelDrift>` de `BudgetRecord`, donc
    présent en JSON/YAML.
  - `render_budget_text` : après la ligne `model`, une ligne
    `  code       <declared_model> (well_known_agents.rs)` suivie de
    ` — en phase` / ` — DÉRIVE : le runtime sert <runtime_model> (<runtime_model_source>)` /
    `  code       (aucun modèle déclaré par le code pour cet agent)` /
    `  code       (dérive non évaluée par ce serveur — binaire antérieur à mika#2473)`.
  - `render_unattested` inchangé (rien d'attesté ⇒ rien de dérivé).
- **Test Scenarios.** (`agents.rs` tests, sur `render`/`sample()` existants)
  - `mika2473_the_render_carries_the_four_drift_states` : `drift` (les deux
    modèles et la porte apparaissent), `in_sync`, `not_applicable`, `None`
    (la phrase *non évaluée*), et le mot `en phase` **n'apparaît pas** hors du
    cas `in_sync` (contrôle négatif).
  - `mika2473_json_output_carries_model_drift` : JSON du record rendu contient
    la clé `model_drift` avec `status`.
  - Non-régression : `mika2457_the_cli_resolves_no_budget_locally` et le scan
    `mika2457_the_local_resolution_scan_is_not_vacuous` verts — le CLI ne
    résout toujours rien localement.
- **Verification.** `cargo test -p mika-cli commands::agents` vert.

### U6. Documentation

- **Goal.** Le lecteur suivant trouve la garde, ses deux événements et sa
  lecture sans rouvrir ce plan.
- **Requirements.** R4, R8 (lisibilité).
- **Files.** `crates/mika-agent/CLAUDE.md` (sous *Observability + boot guard
  (mika#2293)*, après le paragraphe mika#2457), `crates/mika-common/CLAUDE.md`
  (sous *Budget provenance*), `CLAUDE.md` racine (sous *Observabilité du
  budget effectif*, la ligne `code` de la commande et les deux `grep`).
- **Approach.** Un paragraphe par surface, nommant : les deux événements et
  leurs niveaux, la règle « signale, ne refuse pas » (KTD1), le fait que le
  côté déclaré est celui **du binaire servi** (un binaire en retard sur `main`
  déclare la constante d'hier — `feedback_binary_staleness_vs_main`), et la
  sonde post-déploiement ci-dessous.
- **Verification.** `docs/` et `CLAUDE.md` relus par `/ce:doc-review` du pipeline.

## Verification Contract

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p mika-common -- llm::budget_provenance llm::config_freshness
cargo test -p mika-agent -- well_known_agents server:: agent_loop::
cargo test -p mika-cli -- commands::agents
cargo test -p mika-common -- mika2457   # le scan « un seul constructeur » reste vert, allowlist vide
```

**Deux formes de ces commandes sont fausses, et l'une des deux est verte.**
*(a) Les filtres passent derrière `--`.* **Avant** `--`, `cargo test` n'accepte
qu'un seul `TESTNAME` positionnel et refuse le second par
`error: unexpected argument '…' found` — rien ne tourne. **Derrière** `--` ce ne
sont plus des arguments de cargo mais de libtest, qui en accepte plusieurs et les
combine en OU. Les deux moitiés sont mesurées sur ce poste (cargo 1.93.1) : la
forme d'origine échoue à l'analyse des arguments, la forme corrigée fait tourner
623 tests (77 + 546). C'est bruyant dans les deux cas, donc sans danger — mais le
contrat était inexécutable tel qu'il était écrit. *(b) Le filtre du scan D2 est `agent_loop::`, jamais
`agent_loop::mika2473`.* Les tests de ce module vivent dans
`#[cfg(test)] mod tests` (`agent_loop/mod.rs:9546`), donc leur chemin réel est
`agent_loop::tests::mika2473_…`, dont `agent_loop::mika2473` n'est **pas** une
sous-chaîne — le filtre libtest en est une, et il ne matche rien. Mesuré, les
deux contrôles dans le même appel : `agent_loop::mika2342` rend
`0 passed; 5185 filtered out` **et sort 0** ; `agent_loop::tests::mika2342` rend
`2 passed`. AC7 aurait donc été signée par une commande verte qui n'exécute
aucun test — la sonde inerte que la *Fire-Disposition* de ce plan refuse par
ailleurs. `agent_loop::` corrige la classe **et** fait tourner la suite
existante que l'insertion d'U4 pourrait casser, ce qu'U4 déclare déjà
autoritatif pour son propre vert.

**Lire le compte, jamais le seul code de sortie.** Chaque ligne doit rapporter
un nombre de tests **non nul** : un `0 passed` qui sort 0 est la forme que ce
correctif existe pour fermer.

Chaque contrôle négatif (U1 env-door, U2 mtime, U3 édition post-provision, U5
*en phase* absent) est **vu rouge** en neutralisant son terme avant d'être vu
vert — terme par terme (`feedback_red_before_control_is_term_by_term`).

### Sonde post-déploiement (sur le vrai serveur, après `make deploy` + restart)

```bash
# D1 — trois agents bien connus, trois lignes, avant tout tour
grep -E 'well_known_model_(drift|in_sync)' "$MIKA_SPIRIT_LOG_FILE" \
  | jq '{agent_id, declared_model, runtime_model, runtime_model_source}'
# attendu sur ce poste (état du 2026-09-22) : mika-arch DRIFT kimi-k2.5→kimi-k3 (agent_config),
# mika-qa DRIFT zai/glm-5.2→openrouter/z-ai/glm-5.2 (agent_config), mika-dev in_sync

mika agents budget --agent mika-arch        # la ligne `code … — DÉRIVE …` est rendue

# D2 — les deux contrôles, dans l'ordre
touch ~/.mika/agents/mika-arch/config.toml && mika ask --agent mika-arch "ping"
grep agent_config_changed_since_boot "$MIKA_SPIRIT_LOG_FILE" | tail -1 | jq .restart_required   # false
# puis une édition réelle du modèle (réversible : .bak daté), un tour, restart_required == true,
# puis restauration du fichier — sans redémarrer spirit, le modèle en service n'a pas bougé.
```

Zéro ligne D1 après restart ⇒ binaire antérieur au correctif (classe mika#2340) :
établir le déploiement avant toute conclusion (`feedback_mika_skills_update_noop_verify_prompt_by_diff`).

## Fire-Disposition

**Deux familles de détecteurs, et une seule a une population pré-existante.**

| Détecteur | Population « données existantes » | Disposition |
|---|---|---|
| Tests unitaires U1–U5 | aucune — hermétiques (tempdir, env nettoyé, états construits) | N/A |
| Scan structurel U4 (un seul appel à `detect_config_change`) | **zéro** appel à HEAD `367be118` | **(a)** allowlist **vide** ; conduite au déclenchement écrite dans le doc-comment : retirer le second site, ne pas allowlister |
| Scan mika#2457 (constructeur unique du record) | inchangé, allowlist vide | (a), rien à ajouter — U2 **appelle** le constructeur, il n'en crée pas |
| **D1 / D2 en production** | **les trois agents de ce poste** : mika-arch et mika-qa en dérive, mika-dev en phase ; `MIKA_DISABLE_AGENT_PROVISIONING=1` | **(a) Named allowlist exception — zéro entrée.** Ces détecteurs ne font pas échouer un test ni un boot (KTD1) : ils rapportent, et rapporter cette population **est le livrable** — la même raison pour laquelle mika#2457 a refusé d'allowlister la divergence que sa route rend. Une entrée d'allowlist ici serait la garde muette que mika#2328 a mesurée ; conduite au déclenchement : lire, ne pas filtrer (paragraphe ci-dessous) |

**Conduite quand D1 tire à chaque boot sur les trois agents** : ce n'est pas
du bruit à museler, c'est l'état de ce poste, lisible. La résolution appartient
à mika#2472 (réconcilier + calibrer), pas à un `#[ignore]` ni à un filtre.

## Acceptance criteria

1. **AC1 — Le côté déclaré est lu par la dérivation existante.** `declared_model`
   rend, pour chaque `ProviderKind`, le couple que `ModelProvenance::resolve`
   rend sur un home vierge portant le même texte ; il applique
   `default_model()` en l'absence de ligne modèle et rend `None` sur un
   `llm_provider` illisible. Aucune table de clés n'est ajoutée.
2. **AC2 — D1 est fixé à l'init et ne refuse rien.** `AgentState.model_drift`
   est posé dans `init_agent` à partir du record mika#2457 et de la constante de
   l'agent bien connu ; `in_sync` / `drift` / `not_applicable` ; aucun chemin
   d'erreur nouveau dans `init_agent` ni dans `run_server`. `budget_guard` est
   inchangé.
3. **AC3 — La dérive est dite une fois, avec sa porte.** `well_known_model_drift`
   (WARN) ou `well_known_model_in_sync` (INFO) est émis une fois par agent et
   par init, avec les champs de R4 ; un modèle posé par variable
   d'environnement ou `.env` per-agent est une dérive rapportée avec
   `runtime_model_source` = sa porte.
4. **AC4 — La route rend le sibling, sans relire.** `GET /api/v1/agents/{id}/budget`
   rend `{ budget, model_drift }` ; muter le `config.toml` après l'init ne
   change ni l'un ni l'autre ; 404 inchangé et sans `model_drift`.
5. **AC5 — Le CLI rend quatre états et n'invente pas le cinquième.**
   `mika agents budget` rend `drift`, `in_sync`, `not_applicable` et
   *non évaluée* (clé absente) ; *en phase* n'apparaît que sur `in_sync` ;
   JSON/YAML portent `model_drift` ; *non attesté* inchangé ; le scan « le CLI
   ne résout rien localement » reste vert.
6. **AC6 — D2 rapporte une édition post-boot une fois, et dit si un
   redémarrage est requis.** Après `note_config_at_boot`, un mtime inchangé ne
   rapporte rien ; un mtime changé rapporte une fois `agent_config_changed_since_boot`
   avec `restart_required` vrai ssi un champ effectif du record diffère hors
   `resolved_at`, et `model_on_disk` / `model_in_service` nommés ;
   `AgentState.budget_record` n'est jamais remplacé.
7. **AC7 — D2 a un seul site.** `detect_config_change` est appelé exactement
   une fois dans `agent_loop/mod.rs`, dans `load_agent_context` ; scan de
   source livré, allowlist vide.
8. **AC8 — Rien ne bouge.** Aucune constante de `well_known_agents.rs`, aucune
   valeur de `budget_provenance.rs`, **aucune assertion** des tests
   `mika2293_*` / `mika2328_*` / `mika2457_*` modifiée pour passer — seuls les
   appels à `test_state_full` gagnent mécaniquement le paramètre `model_drift`.
   La précision est nécessaire pour que le critère soit satisfiable : U3 étend
   la signature de `test_state_full` (`server/mod.rs:2102`), qui est appelée
   depuis le corps de `mika2457_the_route_serves_the_record_of_the_init_not_the_disk`
   (`:4586`), lequel ne compilerait pas sans gagner l'argument. C'est la
   formulation qu'U3 portait déjà (« verts sans modification de leurs
   assertions ») ; l'intention d'AC8 est intacte.
9. **AC9 — Fire-Disposition renseignée** : la section ci-dessus nomme
   l'option par détecteur, mesure la population pré-existante, et écrit la
   conduite au déclenchement.

## Definition of Done

- [ ] U1 : `declared_model`, `DeclaredModel`, `ModelDriftCheck::compare`, `emit_model_drift` livrés et ré-exportés ; **quatre** tests verts (dont celui qui observe l'émission — AC3), contrôles négatifs vus rouges
- [ ] U2 : `config_freshness` livré (`note_config_at_boot`, `detect_config_change`, `report_config_change`) ; trois tests verts
- [ ] U3 : `init_agent` compare, émet, note, garde ; route étendue ; `test_state_full` étendu ; deux tests verts ; doc-comments `MIKA_DEV_CONFIG` / `MIKA_QA_CONFIG` renvoient à la garde
- [ ] U4 : un appel dans `load_agent_context` ; scan structurel vert, allowlist vide
- [ ] U5 : miroir `ModelDrift`, quatre rendus, JSON/YAML ; tests verts ; scans mika#2457 verts
- [ ] U6 : les trois `CLAUDE.md` mis à jour, sonde post-déploiement écrite
- [ ] `cargo fmt`, `cargo clippy -D warnings`, `cargo test` des trois crates verts ; scan « constructeur unique » mika#2457 vert sans allowlist
- [ ] Corps de PR : les deux événements, la règle KTD1, la dépendance #2461, et la sonde
- [ ] Aucun code d'essai abandonné dans le diff

## Sources

- Ticket mika#2473 ; DoD de mika#2457 (AC7, suivi (b)) ; mika#2328 (classe, doc-comment `well_known_agents.rs:228-244`) ; mika#2327 (verdict perdu).
- `crates/mika-agent/src/well_known_agents.rs` : `WellKnownAgent.config_toml` `:62`, `MIKA_DEV_CONFIG` `:181`, `MIKA_QA_CONFIG` `:251`, `MIKA_ARCH_CONFIG` `:1524`, `reconcile_well_known_config` `:759`, `find_well_known_agent` `:823`, `provision_well_known_agents` `:884` (chemin gelé `:885-899`), `WELL_KNOWN_AGENTS` `:477`.
- `crates/mika-common/src/llm/budget_provenance.rs` : en-tête (trois mondes, cascade inversée, « a false provenance is strictly worse than none »), `ModelProvenance` `:283`, `effective_model` `:333`, `model_config_key` `:368`, `CascadeLayers::read` `:503`, porte process-env `:528`, `config_key_as_string` `:588`, dédup `LAST_EMITTED` / `log_llm_budget_resolved` `:673`, « a warning that contradicts a decision gets muted » `:739-742`, tests `:836`, `:1059`, `:1404`, `:1499`.
- PR #2461 (`feat/2457/p1-substrat-mika-arch-verdict-sous-le` @ `c72e40ea`) : `ResolvedBudgetRecord` (derive `Clone, PartialEq, Eq, Serialize, Deserialize`), `resolve_llm_budget_record`, `emit_llm_budget_resolved`, `AgentState.budget_record`, `handle_agent_budget`, CLI `budget()` / `BudgetRecord` / `render_budget_text` / `render_unattested`, tests `mika2457_*`.
- `crates/mika-agent/src/server/mod.rs` : `init_agent` `:446`, `run_server` `:757`, provisioning `:808`, `budget_guard` `:822` ; `server/budget_guard.rs` (refus d'une paire invalide — le contre-modèle de KTD1) ; `server/state.rs` `AgentState` `:27`, `AppState.agents` `:120`.
- `crates/mika-agent/src/agent_loop/mod.rs` : `load_agent_context` `:478` et ses trois appelants `:4647`, `:5713`, `:6590` ; `emit_turn_usage` `:8489` et ses sites `:917`, `:1580`, `:1605` (`llm.model_name()`).
- `crates/mika-common/src/config.rs` : `make_llm_provider` `:1833` (`active_llm_config().model`), `make_provider_for` `:1857` ; `crates/mika-common/src/llm/types.rs` `LlmResponse` `:185` (pas de champ `model` de réponse).
- `crates/mika-agent/src/teams/engine.rs:217` — second appelant, hors D1/D2 (pas d'`AgentState`).
- État de ce poste (2026-09-22) : `~/.mika/.env:45` `MIKA_DISABLE_AGENT_PROVISIONING=1` ; env process de `mika-spirit` : `MIKA_LLM_HTTP_TIMEOUT_SECS=300` ; `config.toml` de mika-arch (`kimi-k3`, mtime 2026-09-18 21:01), mika-qa (`openrouter`, 2026-09-19 14:46), mika-dev (2026-09-19 14:46).
- Mémoires : `feedback_agent_config_flip_needs_restart_read_model_in_turn_usage`, `feedback_binary_staleness_vs_main`, `feedback_a_probe_needs_both_controls_in_the_same_call`, `feedback_red_before_control_is_term_by_term`, `feedback_deployed_ne_equal_pas_efficace`.
