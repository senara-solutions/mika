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

### D1 — KG fixture seeding: crate-shared helper module, four-layer seed, synchronous resolver

**Problem:** Scenarios need a known KG state: domain entities (from `kg_entities`), subject entities (from `kg_subject_entities`), chunks (from `kg_chunks`), and resolutions (from `kg_subject_resolutions`). Seeding requires writing to all four tables with correct FK relationships. The resolver normally runs async at startup (`entity_resolver.rs`); tests can't wait on a background task. Also: `#741`'s grounding scenarios will likely need KG seeding too (agent-ignores-correct-KG-result is a grounding failure whose setup looks like KG seeding) — the helpers must be importable across tickets, not private to #740.

**Decision:** One helper module at **`crates/mika-agent/tests/eval/kg_fixtures/mod.rs`** (crate-shared location, NOT nested under `kg_self_knowledge/`) with:

- `seed_domain_entity(db, spec)` — inserts into `kg_entities` with `entity_key = "<type>:<name>"` per convention
- `seed_subject_entity(db, agent_id, spec)` — inserts into `kg_subject_entities`
- `seed_chunk(db, agent_id, spec)` — inserts into `kg_chunks` with text + metadata
- `seed_chunk_subject(db, chunk_id, subject_id)` — inserts provenance row
- `seed_resolution(db, agent_id, subject_id, domain_id, confidence, outcome)` — inserts into both `kg_subject_resolutions` and `kg_resolutions_log`

Helpers are `pub` in the test module so `#741`'s scenarios can import them directly. Single implementation across tickets; schema evolution cascades to one place.

**Synchronous resolver path.** Scenarios that need "run the resolver on seeded subjects and assert outcomes" call `EntityResolver::resolve_pending().await` directly from the test, synchronously. The positive reason: **eval harness tests behavior under test-controlled input; the startup race is an orchestration concern with different infrastructure needs (integration-level), not a behavior-under-test concern.** `mika#739` owns the race case with its own coverage; this plan owns behavior-under-test.

**Verified precondition:** `resolve_pending` source at `crates/mika-agent/src/kg/entity_resolver.rs:199-236` reads state fresh from DB via `get_pending_entities()` on every call. No pre-fetch, no cache, no state carried across invocations. The only `Arc<...>` in the resolver is the `LlmProvider` handle — shared client, not shared state. Sync-bypass is behaviorally equivalent to async-spawn for the purposes of the test (modulo the pending query actually seeing the data, which the test controls). Verified 2026-04-22.

Each scenario file owns its fixture *composition*; helpers are shared, scenarios are not coupled (per #339 D3 precedent — Rust per-scenario setup, no coupling).

**Rationale:** Five typed helper functions map 1:1 to the five tables involved in KG layering. Seeding via the existing `Database` API (not raw SQL) keeps the fixture helper in step with schema migrations — when schema v26 lands, the helper breaks at compile time, not at runtime. Synchronous resolver call makes Stage 1 / Stage 2 outcome assertions reliable without touching the startup race. Crate-shared location prevents #741 from cargo-culting a second implementation.

**Rejected alternatives:**
- Helpers private to `#740`'s scenario directory. Rejected — forces `#741` to duplicate, which drifts.
- YAML-driven KG fixture files. Same reasoning as #339 D3 (locked as "Rejected alternative"): scenarios are test code, test code is Rust, YAML grows a DSL grows an interpreter grows regret.

### D2 — Scenario catalog: seven scenarios, distributed by entry path and resolver stage

**Problem:** How many scenarios, covering which behaviors? #339 committed 25 scenarios total across four classes; #740 claims a slice of the milestone-level budget.

**Decision:** **Seven scenarios**, one per file under `crates/mika-agent/tests/eval/kg_self_knowledge/`. Naming per #339 D2 (`{class}_{shape}_{descriptor}.rs`):

1. **`tool_selection_query_knowledge_graph.rs`** — User question "which skill handles PR merges?" → assert `query_knowledge_graph` tool called BEFORE any final text response. Covers the routing decision between `query_knowledge_graph` (structured) and `get_documentation` (reference).
2. **`path_a_direct_domain_match.rs`** — Seeded `skill:self-dev` domain entity; query matches name; assert one result, `entity_type=Skill`, `hop=0`.
3. **`path_b_subject_match_agent_scoped.rs`** — Seeded subject entity `pattern:fabrication-guard` for agent `mika-dev` only; assert mika-dev gets result; assert a different agent gets `starting_entity_missing` (agent-scoped isolation).
4. **`path_c_semantic_via_chunks.rs`** — Seeded chunk + subject + resolution; free-text paraphrase query; assert chunk → subject → domain bridge traversal. Second variant without resolution; assert subject-only result.
5. **`stage_1_exact_match.rs`** — Seeded case-variant subject (`skill:Self-Dev`) + domain (`skill:self-dev`); invoke `resolve_pending`; assert `outcome=matched_exact` in `kg_resolutions_log`; assert confidence = extraction_confidence.
6. **`stage_2_llm_disambiguation.rs`** — **Parameterized over three ambiguity classes** in a single test body: `Synonyms` (skill:qa-review vs skill:qa-review-build-callback, shared semantic neighborhood), `CaseVariants` (skill:self-dev vs skill:Self-Dev, exact-match-should-catch-but-doesn't test), `TrueDuplicates` (two entities with near-identical semantics from different seed paths). All three variants exercise the same Stage 2 code path against different input shapes; the per-class outcome is asserted distinctly so the test output shows `[synonyms] pass`, `[case-variants] pass`, `[true-duplicates] pass`. **Fixture shape is fixed: 5 candidates + ~500-token chunk context per variant.** Hard assertion: Stage 2 code path executed, resolution written to `kg_subject_resolutions`, `outcome=matched_llm` in log, confidence = min(extraction, llm). Soft assertion (tag-based): `self-knowledge:disambiguation-correct` when the LLM picked the structurally-expected candidate; `self-knowledge:disambiguation-plausible-alternative` when it picked a different-but-defensible candidate. Stage 2 is inherently noisy — calling the LLM's judgment a hard regression on a genuinely ambiguous input is false-signal. Hard-assert the execution; soft-tag the judgment. Fourth variant in the same file with `MIKA_KG_RESOLUTION_MODEL` unset; assert `outcome=skipped_no_llm`.
7. **`agent_context_annotation_disabled.rs`** — Disable `self-dev` via `skill_overrides`; query for `self-dev`; assert result includes the skill AND `agent_context.enabled=false` (annotated, not filtered).

**Coverage rationale:** seven scenarios cover the three entry paths (A/B/C), two resolver stages (1/2), the routing assertion, and the agent-context annotation. This is the KG milestone's behavioral surface mapped to tests; each missing scenario would leave a documented code path un-exercised.

**Milestone budget impact:** #339 has 25 scenarios for the general-quality slice; #740 adds 7 KG-specific; #741 will add its own count. Total milestone scenario count is the sum across siblings, not a shared pool — matches the namespace-per-ticket precedent from #339 D4.

**Rejected alternative:** Five scenarios (collapse Path A and Stage 1 exact match since both hit `kg_entities` directly). Rejected because they test different code paths — Path A is the query tool's entry resolution, Stage 1 is the resolver's matching step. Collapsing would mask a regression where one works and the other doesn't.

### D3 — Mock-LLM subset vs real-API subset

**Problem:** Which scenarios run on every CI push (mock-only) vs behind `MIKA_EVAL_REAL_PROVIDERS` + `--ignored`?

**Decision:** Per #339 D6 three-tier model, each scenario has unit / integration / calibration tiers. Stage 2 (scenario 6) needs real LLM for disambiguation — its **unit tier uses a deterministic mock response** (pre-scripted JSON picking one candidate), **integration tier uses a real resolution model**, **calibration tier adds artifact capture per #338 D7**.

All other scenarios (A, B, C without Stage 2 dependency, Stage 1 exact match, agent-context annotation, tool-selection routing) run **mock-only in unit tier** and are skipped from the real-API integration tier — they don't exercise LLM-dependent behavior, so running them against a real provider adds cost without coverage.

**Path C (scenario 4) uses a deterministic embedding stand-in for mock-tier coverage.** Mock tier uses a hash-to-fixed-vector embedding client that returns consistent results for the same input without pretending to be semantic — this exercises the *code structure* of Path C (hybrid search call, chunk→subject→domain traversal, ranking surface) even though the ranking is meaningless. Integration tier swaps in a real embedding client (`openai/text-embedding-3-small`) for actual semantic retrieval. Without the stand-in, mock tier either skips the embedding step (and doesn't exercise Path C) or passes `None` (and falls through to a different code branch).

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
- `self-knowledge:capability-claimed-without-query` — failure mode: agent claimed capability state without a KG query
- `self-knowledge:stage-1-skipped` — soft signal that Stage 1 exact match was bypassed even though it should have hit
- `self-knowledge:agent-context-missing` — result returned but the `agent_context` annotation was absent/wrong
- `self-knowledge:disambiguation-correct` — Stage 2 picked the structurally-expected candidate
- `self-knowledge:disambiguation-plausible-alternative` — Stage 2 picked a different-but-defensible candidate (noise absorption for genuinely ambiguous input)

**Scope boundary with `#741`.** `self-knowledge:*` tags cover the code path from tool-invocation through resolver. Agent *response quality* given a successful KG result — specifically "KG returned correct result but agent ignored it and answered from training data anyway" — is a **grounding failure**, not a self-knowledge failure. That case is routed to `#741`'s `grounding:*` namespace. The apparent vocabulary gap in #740 (no tag for "agent-ignores-correct-KG-result") is the namespace boundary working as intended, not a missing tag.

**Tag attribution rule — cause-location, not symptom.** Tags identify *where in the code path* a failure originated, not the user-visible symptom. A user seeing "agent stated a falsehood" is a symptom; the tag attributes to the cause. Three worked examples:
- KG returned wrong result → `self-knowledge:*` (resolver returned wrong data)
- KG returned right result, agent ignored it → `grounding:*` (response construction ignored evidence)
- KG state itself is stale/corrupt → hard-assertion fail or data-integrity ticket, NOT a soft tag (this is a data-quality problem, not a behavior failure)

When a new ambiguous case surfaces, apply this rule *before* expanding vocabulary: trace the failure to the code path it originated in, attribute the tag to that path. Symptoms cross namespace boundaries; causes do not. Mirror rule lives in `#741` D4.

**Vocabulary review checkpoint.** After all seven scenarios are implemented, revisit this vocabulary before merging. If any scenario's outcome doesn't fit one of the six tags AND the case isn't cleanly routable to `#741`'s namespace, expand this vocabulary (add a seventh tag with rationale) rather than stretching an existing tag. Review is an explicit AC, not a handshake.

Vocabulary is defined in `crates/mika-agent/tests/eval/kg_self_knowledge/README.md` (per #339 D8 — README-next-to-tests, not a separate doc tree). Calibration artifact (#338 D7) preserves the namespace structure; aggregation across tickets works at the tooling layer.

**Rationale:** Tags map to the behavioral failure modes that a KG-backed self-knowledge regression would produce, plus two for Stage 2 judgment noise absorption. Each scenario MAY emit one or more of these tags; not every scenario emits all. Scope sentence routes adjacent failure classes to sibling tickets instead of stretching this vocabulary.

**Rejected alternative:** Reuse `#339`'s `quality:*` namespace. Rejected per the ticket-namespacing principle — these tags describe KG-specific failure classes that don't fit general quality vocabulary.

### D5 — Fixture freshness: schema-version assertion, no auto-regeneration

**Problem:** The KG domain graph is deterministically projected from the live registries at server startup (`kg::domain_builder`). Seeded fixtures use fabricated entity keys (`skill:self-dev`, `problem_type:ci_failure`, etc.) that must match the domain graph's conventions. If the domain graph's schema or key convention drifts (e.g., capitalization, separator), seeded fixtures become invalid without any obvious test signal.

**Decision:** The fixture module asserts at load time that the current schema version is `25` (the KG schema). On a schema bump, the assertion fails loudly with an **actionable message — not a terse `left != right` diff**:

```rust
assert_eq!(
    crate::db::CURRENT_SCHEMA_VERSION, 25,
    "KG eval fixtures pinned to schema v25. Schema bumped to v{}; \
     update seed_* helpers in tests/eval/kg_fixtures/mod.rs and bump this pin. \
     See mika/docs/plans/740-kg-self-knowledge-eval.md D5.",
    crate::db::CURRENT_SCHEMA_VERSION
);
```

Any author advancing the schema MUST update the fixture alongside; the failure message is a checklist pointing at the exact files and pin. **No auto-regeneration** — the fixture is hand-crafted and frozen until a schema change forces a rewrite.

Additionally: the fixture module's doc comment explicitly references `docs/architecture/kg-id-convention.md` and `crates/mika-agent/src/kg/domain_builder.rs` as the canonical sources for the `<type>:<name>` key format, so a future author tracing "is this fixture still correct?" has a direct pointer.

**Rationale:** The domain graph is a living projection; fixtures are frozen snapshots. Keeping them in step requires either auto-regeneration (brittle, couples tests to the domain-builder internals) or loud failure on divergence (simple, explicit). Loud failure wins — the author advancing the schema is the right person to update the fixture. Same structural principle as #338 D9's frozen regression fixture: the test's failure mode tells you exactly what to do about it.

**Rejected alternatives:**
- Auto-regenerate fixtures from a minimal domain-builder run in test setup. Couples the eval to the domain-builder's internal state model; a domain-builder refactor cascades to test rewrites silently.
- Log-warn-and-continue. Rot signal. The theater option.

### D6 — Paired assertions within capability tests (each scenario covers a capability's full success/failure surface)

**Problem:** `query_knowledge_graph` returns structured status values: `ok`, `starting_entity_missing`, `traversal_empty`. The self-knowledge skill (#692) uses these for fallback logic. Should failure-mode surfacing get its own scenario files?

**Decision:** **Paired assertions within capability tests.** Each scenario file is a *capability under test*, and capabilities have both success-path and failure-path assertions that share fixture setup. Not "failure assertions smuggled into positive tests" — proper paired coverage.

- Scenario 3 (Path B agent-scoped) asserts both the success path (correct-agent query returns subject result) AND the failure path (`starting_entity_missing` for wrong-agent query) on the same fixture.
- Scenario 4 (Path C) asserts both the success path (chunk → subject → domain bridge on a resolved subject) AND the failure path (`traversal_empty` for unresolved subject) on the same fixture.
- Each scenario's AC enumerates its success-path hard assertion AND its failure-path hard assertion where applicable.

Extracting status checks into their own scenarios would duplicate fixture setup without adding behavioral coverage. The status is a property of the capability's response shape, not a separate capability.

**Capability × assertion-status matrix in README.** The `kg_self_knowledge/README.md` includes an explicit matrix — rows are the 7 scenarios, columns are the status outcomes (`ok`, `starting_entity_missing`, `traversal_empty`, `matched_exact`, `matched_llm`, `skipped_no_llm`, `agent_context=enabled|disabled`), cells are `✓` where the scenario asserts that outcome. Reviewers see coverage at-a-glance without reading individual test files.

**Rationale:** Paired-assertion framing correctly describes what the scenarios do: each covers the full behavioral surface of one capability, not just the happy path. Extracting failures into standalone scenarios inflates the count and loses the fixture-sharing that makes the pairing cheap.

**Rejected alternative:** Add scenarios 8–10 for `ok` / `starting_entity_missing` / `traversal_empty` as standalone. Rejected — the status is a property of a capability's response, not a separate capability. Fixture duplication without behavioral gain.

## Acceptance Criteria

- [ ] 7 scenario files under `crates/mika-agent/tests/eval/kg_self_knowledge/`, distributed per D2.
- [ ] **Fixture module at `crates/mika-agent/tests/eval/kg_fixtures/mod.rs` (crate-shared, NOT nested under the scenario directory).** Exports all five seed helpers `pub` for `#741` to import. Module asserts schema version `25` at load time with actionable failure message per D5.
- [ ] **Verified precondition recorded in plan body:** resolver source (`entity_resolver.rs:199-236`) reads DB state fresh on every invocation; no pre-fetch/cache. Verification done 2026-04-22 as part of this plan.
- [ ] Every scenario has ≥1 hard assertion; soft-tag output uses the `self-knowledge:*` namespace only (per D4).
- [ ] Each scenario runs in unit / integration / calibration tiers per #339 D6; tier table from D3 is authoritative.
- [ ] `register_scenario(name, meta)` uniqueness guard from #339 D7 protects against copy-paste-without-rename.
- [ ] **Path A / B / C coverage** — scenarios 2, 3, 4 each assert their entry path produces the expected result shape.
- [ ] **Stage 1 / Stage 2 coverage** — scenarios 5 and 6 each assert their outcome in `kg_resolutions_log`, confidence clamping behavior, and `skipped_no_llm` path.
- [ ] **Scenario 6 parameterized over three ambiguity classes** (`Synonyms`, `CaseVariants`, `TrueDuplicates`) within one test file per D2. Fixture shape fixed: 5 candidates + ~500-token chunk per variant. Hard assertion on code-path execution + DB writes; soft assertion (tags `self-knowledge:disambiguation-correct` / `...-plausible-alternative`) on judgment quality.
- [ ] **Paired assertions within each capability test** (per D6) — scenarios 3 and 4 each assert BOTH success path AND failure path (`starting_entity_missing`, `traversal_empty`) on shared fixture.
- [ ] **Agent-context annotation** — scenario 7 asserts disabled skill returns with `enabled=false` field set, NOT filtered from results.
- [ ] **Deterministic embedding stand-in for mock-tier scenario 4** (hash-to-fixed-vector). Integration tier swaps in real `text-embedding-3-small`.
- [ ] `crates/mika-agent/tests/eval/kg_self_knowledge/README.md` covers: fixture patterns, the `self-knowledge:*` vocabulary with each tag's trigger condition, how to add a scenario, **scope boundary with #741 (`grounding:*`)**, and a **capability × status matrix** (rows: 7 scenarios, columns: asserted status outcomes, cells: ✓).
- [ ] **Vocabulary review checkpoint before merge** (per D4): if any implemented scenario's outcome doesn't fit one of the six `self-knowledge:*` tags AND isn't cleanly routable to #741's namespace, expand the vocabulary with rationale.
- [ ] `crates/mika-agent/CLAUDE.md` KG section referenced from the README; no duplicate content.
- [ ] `cargo test -p mika-agent --test eval` green (unit tier).
- [ ] Integration tier green for scenarios 4 and 6 when invoked with `MIKA_EVAL_REAL_PROVIDERS=anthropic` + `--ignored` + `MIKA_KG_RESOLUTION_MODEL` + an embedding key.
- [ ] `cargo clippy` clean.

## Cost envelope (real-API integration tier)

**Scenario 6 (Stage 2 LLM disambiguation):** fixture shape fixed at 5 candidates + ~500-token chunk context × 3 ambiguity-class variants = 3 LLM invocations per integration run.

- Per invocation: ~1K input tokens (prompt template ~300 + 5 candidates ~200 + chunk ~500) + ~100 output tokens
- Against `claude-sonnet-4-6` at current Anthropic pricing ($3/MTok input, $15/MTok output): `1000 × $3/1M + 100 × $15/1M = $0.0045 per invocation`
- **Three variants × $0.0045 = ~$0.014 per integration run.**

**Scenario 4 (Path C real embedding):** one embedding call per integration run with `text-embedding-3-small`.

- ~500 input tokens × $0.02/MTok = ~$0.00001 per invocation. Effectively free.

**Total real-API cost per integration run of this ticket's scenarios: ~$0.015.** Calibration tier is the same cost (artifact capture overhead is local, not an API cost). These numbers rot as pricing changes — they live in this plan as design-time cost-envelope decisions, not as runtime guarantees. Per #338 D8 principle: workflow timeouts + scenario-count caps at the CI layer are the enforcement; plan-level numbers are design intent.

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

## Review log

**Vincent + friend review pass 1 (2026-04-22, relayed by Vincent):**

- **D1 fixture location moved to crate-shared** `tests/eval/kg_fixtures/mod.rs` (not nested under `kg_self_knowledge/`). Exports `pub` so #741 can import. Prevents cargo-cult duplication.
- **D1 `#739` bypass reframed** with positive reason: eval harness tests behavior under test-controlled input; startup-race is integration-test territory with different infrastructure. `#739` owns its case explicitly.
- **D1 resolver source verified:** `resolve_pending` at `entity_resolver.rs:199-236` reads DB fresh on every call, no pre-fetch, no cache, no cross-invocation state. Sync-bypass assumption validated against source code, not just asserted. Recorded as a verified precondition in the plan body.
- **D2 scenario 6 parameterized over three ambiguity classes** (`Synonyms`, `CaseVariants`, `TrueDuplicates`) within one test body. Fixture shape fixed at 5 candidates + ~500-token chunk per variant. Hard assertion on code-path execution (deterministic); soft tag on judgment quality (noise absorption for genuinely ambiguous input). Same hard/soft split as #339.
- **D3 deterministic embedding stand-in** for mock-tier scenario 4 (hash-to-fixed-vector). Ensures mock tier actually exercises Path C code structure, not a fallthrough.
- **D4 scope sentence added:** `self-knowledge:*` covers tool-invocation through resolver; agent-response-quality given a successful KG result routes to #741's `grounding:*` namespace. Two new tags for Stage 2 judgment (`disambiguation-correct` / `disambiguation-plausible-alternative`). Vocabulary review checkpoint at AC-completion to catch missed failure modes.
- **D5 assertion message customized** to be a checklist pointing at exact files and pin location — not a terse `left != right`. Same principle as #338 D9 frozen-fixture failures.
- **D6 reframed** as "paired assertions within capability tests" — each scenario covers success AND failure paths of one capability on shared fixture. Capability × status matrix added to README AC for at-a-glance coverage visibility.
- **Cost envelope priced at design time** (~$0.015 per integration run for this ticket's scenarios): 3 Stage-2 variants × ~$0.0045 each + negligible embedding cost. Fixture shape fixed as design decision; scenario-author doesn't "declare" cost at implementation time.

**Friend principle extended:** "pin the decision you're depending on" now applies to cost envelope (design-time fixture shape) and assertion messages (actionable on failure), not just upstream plan SHAs and vocabulary namespaces.

**Milestone-level friend review pass 2 (2026-04-23, relayed by Vincent):**

- **D4 extended with tag-attribution cause-location rule.** Previously the scope boundary with #741 worked case-by-case ("agent-ignores-KG-result → grounding"); now there's an explicit rule ("tags attribute to cause-location, not symptom") with three worked examples and a procedure for new ambiguous cases. Converts a handshake decision into a structural convention. Mirror rule landing in #741 D4 simultaneously.
