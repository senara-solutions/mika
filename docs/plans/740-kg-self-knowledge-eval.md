# Plan — mika#740 — KG-backed self-knowledge eval scenarios

**Issue:** senara-solutions/mika#740
**Branch:** `feat/740/kg-self-knowledge-eval`
**Milestone:** Evaluation (#16)
**Blocked by:**
- `#338` at plan commit **`fa54d950`** (matrix machinery, Stage 2 LLM tests need real providers)
- `#340` item #1 (`embedding_client` DI builder for Path C semantic retrieval)
- `#340` item #3 (callback-turn harness surface for chained tool + query patterns)
- `#339` at plan commit **`5fb14662`** (three-tier execution model, `register_scenario` idiom, ticket-namespaced vocabulary pattern)

**Status:** Groomed draft — pending Vincent review

## Context

Milestone #14 shipped KG-backed self-knowledge (`#692` / PR #733): the `query_knowledge_graph` tool with three entry paths (domain match, subject match, semantic via chunks) and a two-stage resolver (exact-match then LLM disambiguation). Nothing in milestone #16 exercises it end-to-end against a seeded KG — which means a prompt or model change can silently degrade KG utilization and no test surfaces the regression. This ticket is the eval slice that locks in the KG milestone's behavioral wins.

#740 narrowly owns **KG-backed self-knowledge scenario coverage**. Provider matrix, golden-dataset breadth, and grounding regressions are sibling tickets. The boundary is tight: if a scenario is about the agent's self-knowledge (does it know what skill handles X, what tools it has, what problem types it can solve?), it belongs here. Anything else goes to #339 or #741.

## Scope boundary

**In scope:**
- Fixture helper that seeds a known KG state per agent: domain entities, subject entities, chunks, resolutions
- Tool-selection assertions — does the agent invoke `query_knowledge_graph` before claiming capability state?
- Entry path coverage — Path A (domain match), Path B (subject match), Path C (semantic via chunks)
- Resolver coverage — Stage 1 exact match, Stage 2 LLM disambiguation, `skipped_no_llm` outcome
- Agent-context annotation — disabled skills surface `agent_context.enabled=false` (annotate, don't filter)
- Failure-mode surfacing — `starting_entity_missing`, `traversal_empty` statuses visible to the agent

**Out of scope:**
- Provider strictness / JSON-schema divergence → `#338` (D4/D9)
- Lexical ingestion correctness (covered by unit tests under `crates/mika-agent/src/kg/lexical_ingestor.rs`; eval here tests the query path, not the ingest path)
- Extraction pipeline correctness (same — eval consumes ingested state, doesn't re-test extraction)
- Resolver startup race (distinct bug tracked as `mika#739`; here we assume the race is fixed before these tests run, OR we work around it in the test fixture by calling `resolve_pending` synchronously)
- Non-KG scenarios → `#339`
- Fabrication/grounding regressions → `#741`

## Decisions

### D1 — KG fixture seeding: per-scenario helper, four-layer seed, synchronous resolver

**Problem:** Scenarios need a known KG state: domain entities (from `kg_entities`), subject entities (from `kg_subject_entities`), chunks (from `kg_chunks`), and resolutions (from `kg_subject_resolutions`). Seeding requires writing to all four tables with correct FK relationships. The resolver normally runs async at startup (`entity_resolver.rs`); tests can't wait on a background task.

**Decision:** One helper module `tests/eval/kg_fixtures.rs` with:

- `seed_domain_entity(db, spec)` — inserts into `kg_entities` with `entity_key = "<type>:<name>"` per convention
- `seed_subject_entity(db, agent_id, spec)` — inserts into `kg_subject_entities`
- `seed_chunk(db, agent_id, spec)` — inserts into `kg_chunks` with text + metadata
- `seed_chunk_subject(db, chunk_id, subject_id)` — inserts provenance row
- `seed_resolution(db, agent_id, subject_id, domain_id, confidence, outcome)` — inserts into both `kg_subject_resolutions` and `kg_resolutions_log`

**Synchronous resolver path.** Scenarios that need "run the resolver on seeded subjects and assert outcomes" call `EntityResolver::resolve_pending().await` directly from the test, synchronously. This bypasses the startup `tokio::spawn` race (`mika#739`) — the test is deterministic because the test owns the spawn boundary. Not a workaround for the bug; the test simply doesn't use the startup-race code path.

Each scenario file owns its fixture; no shared fixture across scenarios (per #339 D3 precedent — Rust per-scenario setup, no coupling).

**Rationale:** Four typed helper functions map 1:1 to the four tables. Seeding via the existing `Database` API (not raw SQL) keeps the fixture helper in step with schema migrations — when schema v26 lands, the helper breaks at compile time, not at runtime. Synchronous resolver call makes Stage 1 / Stage 2 outcome assertions reliable without touching the startup race.

**Rejected alternative:** YAML-driven KG fixture files. Same reasoning as #339 D3 (locked as "Rejected alternative"): scenarios are test code, test code is Rust, YAML grows a DSL grows an interpreter grows regret.

### D2 — Scenario catalog: seven scenarios, distributed by entry path and resolver stage

**Problem:** How many scenarios, covering which behaviors? #339 committed 25 scenarios total across four classes; #740 claims a slice of the milestone-level budget.

**Decision:** **Seven scenarios**, one per file under `crates/mika-agent/tests/eval/kg_self_knowledge/`. Naming per #339 D2 (`{class}_{shape}_{descriptor}.rs`):

1. **`tool_selection_query_knowledge_graph.rs`** — User question "which skill handles PR merges?" → assert `query_knowledge_graph` tool called BEFORE any final text response. Covers the routing decision between `query_knowledge_graph` (structured) and `get_documentation` (reference).
2. **`path_a_direct_domain_match.rs`** — Seeded `skill:self-dev` domain entity; query matches name; assert one result, `entity_type=Skill`, `hop=0`.
3. **`path_b_subject_match_agent_scoped.rs`** — Seeded subject entity `pattern:fabrication-guard` for agent `mika-dev` only; assert mika-dev gets result; assert a different agent gets `starting_entity_missing` (agent-scoped isolation).
4. **`path_c_semantic_via_chunks.rs`** — Seeded chunk + subject + resolution; free-text paraphrase query; assert chunk → subject → domain bridge traversal. Second variant without resolution; assert subject-only result.
5. **`stage_1_exact_match.rs`** — Seeded case-variant subject (`skill:Self-Dev`) + domain (`skill:self-dev`); invoke `resolve_pending`; assert `outcome=matched_exact` in `kg_resolutions_log`; assert confidence = extraction_confidence.
6. **`stage_2_llm_disambiguation.rs`** — Seeded two similarly-named candidates (`skill:qa-review`, `skill:qa-review-build-callback`) + subject with unqualified "qa review" chunk context; assert `outcome=matched_llm`, confidence = min(extraction, llm). Second variant with `MIKA_KG_RESOLUTION_MODEL` unset; assert `outcome=skipped_no_llm`.
7. **`agent_context_annotation_disabled.rs`** — Disable `self-dev` via `skill_overrides`; query for `self-dev`; assert result includes the skill AND `agent_context.enabled=false` (annotated, not filtered).

**Coverage rationale:** seven scenarios cover the three entry paths (A/B/C), two resolver stages (1/2), the routing assertion, and the agent-context annotation. This is the KG milestone's behavioral surface mapped to tests; each missing scenario would leave a documented code path un-exercised.

**Milestone budget impact:** #339 has 25 scenarios for the general-quality slice; #740 adds 7 KG-specific; #741 will add its own count. Total milestone scenario count is the sum across siblings, not a shared pool — matches the namespace-per-ticket precedent from #339 D4.

**Rejected alternative:** Five scenarios (collapse Path A and Stage 1 exact match since both hit `kg_entities` directly). Rejected because they test different code paths — Path A is the query tool's entry resolution, Stage 1 is the resolver's matching step. Collapsing would mask a regression where one works and the other doesn't.

### D3 — Mock-LLM subset vs real-API subset

**Problem:** Which scenarios run on every CI push (mock-only) vs behind `MIKA_EVAL_REAL_PROVIDERS` + `--ignored`?

**Decision:** Per #339 D6 three-tier model, each scenario has unit / integration / calibration tiers. Stage 2 (scenario 6) needs real LLM for disambiguation — its **unit tier uses a deterministic mock response** (pre-scripted JSON picking one candidate), **integration tier uses a real resolution model**, **calibration tier adds artifact capture per #338 D7**.

All other scenarios (A, B, C without Stage 2 dependency, Stage 1 exact match, agent-context annotation, tool-selection routing) run **mock-only in unit tier** and are skipped from the real-API integration tier — they don't exercise LLM-dependent behavior, so running them against a real provider adds cost without coverage.

**Table:**

| Scenario | Unit (mock, on-push) | Integration (real, opt-in) | Calibration (artifact) |
|---|---|---|---|
| 1 tool_selection_query_knowledge_graph | ✅ | — (mock suffices) | — |
| 2 path_a_direct_domain_match | ✅ | — | — |
| 3 path_b_subject_match_agent_scoped | ✅ | — | — |
| 4 path_c_semantic_via_chunks | ✅ (mocked embedding) | ✅ (real embedding client from `#340` D1) | ✅ |
| 5 stage_1_exact_match | ✅ | — | — |
| 6 stage_2_llm_disambiguation | ✅ (mocked judge) | ✅ (real `MIKA_KG_RESOLUTION_MODEL`) | ✅ |
| 7 agent_context_annotation_disabled | ✅ | — | — |

**Rationale:** Only scenarios that exercise LLM or embedding behavior warrant real-API tier. Others test deterministic code paths where mock is faithful.

**Rejected alternative:** Run all seven against real providers on every opt-in run. Triples cost for scenarios that don't learn anything from real-API invocation.

### D4 — Tag vocabulary: `self-knowledge:*` namespace (owned here, not shared)

**Problem:** Soft-assertion LLM-judged tags need a namespace. #339 D4 established ticket-namespacing (#339 owns `quality:*`; #740 owns `self-knowledge:*`).

**Decision:** #740 owns `self-knowledge:*` namespace. Tag vocabulary:

- `self-knowledge:query-invoked` — agent called `query_knowledge_graph` before the final response (hard-assertable; included as a tag too for cross-scenario aggregation)
- `self-knowledge:capability-claimed-without-query` — fabrication-adjacent failure mode: agent claimed capability state without a KG query
- `self-knowledge:stage-1-skipped` — soft signal that Stage 1 exact match was bypassed even though it should have hit
- `self-knowledge:agent-context-missing` — result returned but the `agent_context` annotation was absent/wrong

Vocabulary is frozen in the `crates/mika-agent/tests/eval/kg_self_knowledge/README.md` (per #339 D8 — README-next-to-tests, not a separate doc tree). Calibration artifact (#338 D7) preserves the namespace structure; aggregation across tickets works at the tooling layer.

**Rationale:** Tags map to the four behavioral failure modes that a KG-backed self-knowledge regression would produce. Each scenario MAY emit one or more of these tags; not every scenario emits all.

**Rejected alternative:** Reuse `#339`'s `quality:*` namespace. Rejected per the ticket-namespacing principle — these tags describe KG-specific failure classes that don't fit general quality vocabulary.

### D5 — Fixture freshness: schema-version assertion, no auto-regeneration

**Problem:** The KG domain graph is deterministically projected from the live registries at server startup (`kg::domain_builder`). Seeded fixtures use fabricated entity keys (`skill:self-dev`, `problem_type:ci_failure`, etc.) that must match the domain graph's conventions. If the domain graph's schema or key convention drifts (e.g., capitalization, separator), seeded fixtures become invalid without any obvious test signal.

**Decision:** The fixture module asserts at load time that the current schema version is `25` (the KG schema). On a schema bump, the assertion fails loudly. Any author advancing the schema MUST update the fixture alongside. **No auto-regeneration** — the fixture is hand-crafted and frozen until a schema change forces a rewrite.

Additionally: the fixture module's doc comment explicitly references `docs/architecture/kg-id-convention.md` and `crates/mika-agent/src/kg/domain_builder.rs` as the canonical sources for the `<type>:<name>` key format, so a future author tracing "is this fixture still correct?" has a direct pointer.

**Rationale:** The domain graph is a living projection; fixtures are frozen snapshots. Keeping them in step requires either auto-regeneration (brittle, couples tests to the domain-builder internals) or loud failure on divergence (simple, explicit). Loud failure wins — the author advancing the schema is the right person to update the fixture.

**Rejected alternative:** Auto-regenerate fixtures from a minimal domain-builder run in test setup. Couples the eval to the domain-builder's internal state model; a domain-builder refactor cascades to test rewrites silently.

### D6 — Failure-mode scenarios included in positive tests, not separate files

**Problem:** `query_knowledge_graph` returns structured status values: `ok`, `starting_entity_missing`, `traversal_empty`. The self-knowledge skill (#692) uses these for fallback logic. Should failure-mode surfacing get its own scenario files?

**Decision:** **Embedded assertions within the positive tests, not separate scenarios.**

- Scenario 3 (Path B agent-scoped) already asserts `starting_entity_missing` when queried as a wrong-agent — that's the failure mode.
- Scenario 4 (Path C) asserts `traversal_empty` when a chunk's subject has no resolution — that's the second failure mode.
- Adding dedicated `status_*` scenario files would duplicate the fixture setup without adding behavioral coverage.

Failure-mode surfacing isn't a separate test class; it's a coverage requirement on each scenario where the condition applies. AC explicitly lists which scenarios assert which status outcomes.

**Rationale:** Embedded failure-mode assertions keep scenario count aligned with behavioral coverage. Extracting status checks into their own scenarios would inflate the count without adding signal.

**Rejected alternative:** Add scenarios 8–10 for `ok` / `starting_entity_missing` / `traversal_empty` as standalone. Rejected — the status is a property of the scenario's response, not a separate capability.

## Acceptance Criteria

- [ ] 7 scenario files under `crates/mika-agent/tests/eval/kg_self_knowledge/`, distributed per D2.
- [ ] Fixture module `tests/eval/kg_fixtures.rs` with the five seed helpers from D1. Module asserts schema version `25` at load time per D5.
- [ ] Every scenario has ≥1 hard assertion; soft-tag output uses the `self-knowledge:*` namespace only (per D4).
- [ ] Each scenario runs in unit / integration / calibration tiers per #339 D6; tier table from D3 is authoritative.
- [ ] `register_scenario(name, meta)` uniqueness guard from #339 D7 protects against copy-paste-without-rename.
- [ ] **Path A / B / C coverage** — scenarios 2, 3, 4 each assert their entry path produces the expected result shape.
- [ ] **Stage 1 / Stage 2 coverage** — scenarios 5 and 6 each assert their outcome in `kg_resolutions_log`, confidence clamping behavior, and `skipped_no_llm` path.
- [ ] **Failure-mode coverage** — scenario 3 asserts `starting_entity_missing` for wrong-agent query; scenario 4 asserts `traversal_empty` for unresolved subject.
- [ ] **Agent-context annotation** — scenario 7 asserts disabled skill returns with `enabled=false` field set, NOT filtered from results.
- [ ] `crates/mika-agent/tests/eval/kg_self_knowledge/README.md` covers fixture patterns, the `self-knowledge:*` vocabulary with each tag's trigger condition, and how to add a scenario.
- [ ] `crates/mika-agent/CLAUDE.md` KG section referenced from the README; no duplicate content.
- [ ] `cargo test -p mika-agent --test eval` green (unit tier).
- [ ] Integration tier green for scenarios 4 and 6 when invoked with `MIKA_EVAL_REAL_PROVIDERS=anthropic` + `--ignored` + `MIKA_KG_RESOLUTION_MODEL` + an embedding key.
- [ ] `cargo clippy` clean.

## Dependencies

- Blocked by `#338` (`fa54d950`) — scenario-as-function pattern, matrix runner, calibration artifact, `eval-diff`
- Blocked by `#340` item #1 — `embedding_client` DI for scenario 4 integration tier
- Blocked by `#340` item #3 — callback-turn harness surface IF any scenario involves a callback flow; current scope does not, so this dep is precautionary. If scenario 6's Stage 2 real-API invocation needs callback wrapping (it shouldn't — it's a direct `resolve_pending` call), this becomes hard-required.
- Blocked by `#339` (`5fb14662`) — three-tier execution model, `register_scenario` idiom, ticket-namespaced vocabulary precedent, README + CLAUDE.md doc structure

## Downstream

- None within milestone #16. Future KG enhancement tickets may cite these scenarios for regression coverage.

## Cross-cutting notes

- **Scenario 3 is the agent-scoping canary:** if subject-entity isolation regresses (one agent sees another's subjects), this test fires loudest.
- **Scenario 6 is the only real-API cost surface in this ticket.** Integration + calibration tiers for Stage 2 LLM hit the configured resolution model. Matches the per-scenario cost awareness pattern from #339 D7 — `expected_tokens` metadata on scenario 6 declares the expected cost.
- **Vocabulary ownership boundary is strict:** #740 does NOT define `grounding:*` or `quality:*` tags. If a KG scenario accidentally exhibits a non-KG failure mode (e.g., fabrication), it surfaces a hard assertion failure OR gets re-classified to the right sibling ticket.
- #740 pinned to #338 at `fa54d950` and #339 at `5fb14662`. Upstream drift on either is a grep-findable version bump.

## Open questions (for Vincent before dispatch)

1. **D2 scenario count at 7** — is this tight enough to author in the baseline PR, or should Stage 2 real-API coverage expand to two scenarios (one per candidate-ambiguity class: synonyms vs case-variants vs true duplicates)? My default is 7 (treating Stage 2 as one scenario with internal variants), but splitting is a principled alternative.
2. **D3 integration-tier scope** — I limited real-API tests to scenarios 4 and 6. If you want agent-facing scenarios (e.g., #1 tool-selection routing) tested against real models too, that's an add. My default is "real API only where deterministic code isn't enough."
3. **D5 schema-version assertion** — `assert_eq!(current_schema_version, 25)` at fixture load. Acceptable, or should it be softer (log warn + continue) to avoid blocking all eval tests during a schema-bump PR? I lean hard assert; schema changes are rare enough that loud failure is the right signal.
4. **D6 failure-mode embedded vs separate** — I embedded `starting_entity_missing` + `traversal_empty` into scenarios 3 and 4. Alternative: two additional standalone scenarios for these statuses with minimal fixtures, bringing total to 9. Embedded wins on scenario-count parsimony; standalone wins on failure-mode discoverability. I picked embedded.
