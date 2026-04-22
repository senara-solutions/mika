# Plan — mika#339 — Golden dataset: end-to-end quality testing with real LLM providers

**Issue:** senara-solutions/mika#339
**Branch:** `feat/339/golden-dataset`
**Milestone:** Evaluation (#16)
**Blocked by:** #338 (multi-provider machinery required for real-API scenarios)
**Status:** Groomed draft — pending Vincent review

## Context

#338 ships the harness. This ticket ships what the harness measures — a curated dataset of scenarios that verify Mika produces good answers, not just that the plumbing works. The spec from the issue body: "Mock tests prove the plumbing works. This proves the water tastes good."

Mika today has zero end-to-end quality tests against real providers. Prompt changes, model migrations, skill edits ship blind until a user complains. The `task_id` incident (latent six weeks before provider switch surfaced it) is one class; silent quality degradation on memory recall or tool selection is another class that no harness-level test catches.

#339 is the **broadest** of the three downstream scenario tickets. #740 narrows to KG-backed self-knowledge (Path A/B/C of `query_knowledge_graph`). #741 narrows to grounding/fabrication regressions (four specific incidents from the KG retrospective). #339 covers the remaining scenario surface — memory, tool selection, multi-turn, conversation quality — without duplicating the narrower siblings.

## Scope boundary

**In scope:**
- 20–30 curated scenarios across 4 capability classes (memory, tool selection, conversation quality, skill-specific non-KG / non-grounding)
- Scoring framework (hard assertions + soft assertions)
- Baseline score establishment for at least one configured model tuple
- Per-scenario fixture seeding (known facts, reminders, work items, skills)
- Documentation on how to run, add scenarios, interpret results

**Out of scope (spun out or deferred):**
- KG-backed self-knowledge scenarios → #740
- Grounding / fabrication regressions → #741
- JSON-schema divergence tests → #338 (D4/D9)
- Per-skill provider override tests → #338 (D5)
- Regression gating on baseline drift → #742 (CI maintenance loop)
- Langfuse dashboard integration — explicit non-goal

## Decisions

### D1 — Scenario count: 25, not 20-30 — commit to a target

**Problem:** The issue body says "20-30 representative scenarios." Ranges in plans invite scope creep.

**Decision:** Target **25 scenarios** for the first baseline. Distributed as:
- **Memory: 8 scenarios** — store/recall across variations, multi-fact disambiguation, time-based retrieval, fact updates, privacy boundaries
- **Tool selection: 8 scenarios** — calendar invocations, memory search routing, `run_gh` targeting, `send_message` delivery, conflict cases (two plausible tools)
- **Conversation quality: 5 scenarios** — follow-up context, uncertainty admission (non-fabrication baseline), concise-vs-verbose response calibration, rewind/undo semantics, compaction survival
- **Skill-specific (non-KG, non-grounding): 4 scenarios** — self-dev plan-generation coherence, qa-review bug-catching, google-workspace calendar query structure, `run_gh` PR-body formatting

**Rationale:** 25 is the lower end of "enough for meaningful baseline, cheap enough to run" per the issue body's $0.50-2.00 cost estimate (#338 explicitly rejects stating dollar figures — this count is the structural substitute). Allows ~8 per major class; smaller classes (skill-specific here) get fewer.

**Rejected alternative:** Leave open-ended at 20-30. Rejected — open ranges are implementation-drift vectors; a committed count is a review target.

### D2 — Scenario file organization: one file per scenario, named for assertion shape

**Problem:** Scenarios can live in one big file, one file per class, or one file per scenario. Ergonomics, test discoverability, git-blame clarity, per-scenario skip/ignore gating.

**Decision:** **One file per scenario** under `crates/mika-agent/tests/eval/golden/`. Naming convention: `{class}_{shape}_{descriptor}.rs` — e.g., `memory_recall_cross_session.rs`, `tool_selection_calendar_vs_memory.rs`, `conversation_quality_admit_uncertainty.rs`.

**Rationale:** Single files per scenario:
- Make `cargo test memory_recall_cross_session` exact-match addressable
- Isolate test-author blast radius (one scenario's fixture seeding doesn't leak into another's)
- Make `git blame` attribute each scenario's history to a single author thread
- Support per-scenario `#[ignore]` gating without aggregate-file surgery

Tradeoff accepted: 25 files is a larger tree than 4 files-per-class. Ergonomically fine — `cargo test --list` remains legible, and IDE navigation is file-based.

**Rejected alternative:** One file per class (4 files, 6-8 tests each). Rejected because per-scenario fixture seeding has meaningful per-scenario setup; co-locating 8 scenarios' setup in one file becomes a `fn setup_for_memory_scenario_N()` maze.

### D3 — Fixture seeding: per-scenario `.rs` setup blocks, NO YAML/JSON scenario definitions

**Problem:** Scenarios need pre-seeded state: known facts, reminders, work items, skill overrides, core memory content. Do fixtures live in Rust code, in YAML/JSON definition files parsed by a harness runner, or somewhere in between?

**Decision:** **Setup in Rust per-scenario-file**. Each scenario's fixture is an `async fn setup_{name}_fixture(db: &AsyncDatabase) -> Result<()>` that stores facts, creates reminders, seeds skill overrides, etc. No external YAML/JSON parser.

**Rationale:** YAML scenario definitions look ergonomic (declarative, editable by non-Rust people) but have two fatal problems for this project:
1. They require a separate runner/interpreter that has to stay in sync with the `EvalHarness` + mock provider surface. Double-maintenance.
2. They invite "data-driven testing" patterns that hide failure attribution in a parameterized runner, conflicting with #338 D3's explicit preference for `#[test]` discoverability.

Rust setup functions are harder to author (have to know Rust) but co-locate the fixture with the assertion, keep compile-time type-checking on fixture shape, and stay compatible with the harness's existing builder surface.

**Rejected alternatives:**
- YAML-driven scenarios. Explained above.
- One fixture module shared across scenarios. Creates coupling — one scenario's fixture change breaks another.

### D4 — Scoring framework: hard assertions primary, soft assertions as LLM-judged tags

**Problem:** The issue body describes two scoring classes: hard assertions (tool X called / output contains Y) and soft assertions (quality score 0-10 via judge LLM). How to avoid soft-assertion score drift (which defeats regression detection) while keeping signal on response quality?

**Decision:** Two scoring tiers with strict separation.

- **Hard assertions** are pass/fail and are the regression-gating signal. Every scenario MUST have at least one hard assertion. Examples: `assert!(trace.tool_calls.iter().any(|c| c.name == "calendar_create_event"))`, `assert_contains!(response, expected_field_name)`, `assert!(response.len() < 500)`.
- **Soft assertions** are LLM-judge emitted tags (e.g., "concise", "uncertain", "actionable") reported alongside hard assertions. They go into the calibration artifact (#338 D7) but do NOT gate pass/fail. A scenario where all hard assertions pass but soft tags indicate "verbose" emits a warning, not a failure.

**Soft-assertion judge structure:** fixed prompt template + fixed tag vocabulary. Judge model is configured separately from the scenario-under-test model (avoid self-judging). Judge output is constrained to a tag set, not a free-form 0-10 score — score drift is eliminated because there's no score to drift.

**Rationale:** LLM-judged 0-10 scores are the worst of both worlds: they gate (if thresholded) but can't be trusted (LLMs are noisy evaluators), or they don't gate (and become decorative). Tag-based judging keeps the signal without the gate.

**Rejected alternative:** 0-10 soft scores as originally spec'd in the issue body. Replaced for the reason above.

### D5 — Baseline establishment: one model tuple, explicit, documented

**Problem:** "Baseline scores established for at least Anthropic Claude" from the issue body is vague. Which model? At what timestamp? Where does the baseline live?

**Decision:** Initial baseline = **Anthropic `claude-sonnet-4-6`, captured during the PR that lands this ticket**. Baseline file lives in `target/eval-calibration/{timestamp}.json` (per #338 D7 ephemeral) and is uploaded as a CI workflow artifact. NOT committed to the repo until `mika#742` ships the maintenance loop.

The PR description for this ticket's merge includes the captured baseline inline — human-readable snapshot, dated, provider-versioned. Future scenario authors compare to the PR-description baseline until #742 automates it.

**Rationale:** Baseline-in-PR-description is the ephemeral-but-discoverable compromise. No committed-but-unmaintained artifact; no orphaned calibration output; operators can find "what was the baseline for scenario X?" via `gh pr view <N>`.

**Rejected alternative:** Commit `tests/fixtures/eval-baseline.json`. Explicitly rejected for same theater reasons as in #338 D7 — deferred to #742.

### D6 — Execution model: three tiers — unit / integration / calibration

**Problem:** The issue spec mixes `#[ignore]` gating, env vars, and "when to run." Needs decomposition.

**Decision:** Three tiers, each with a distinct invocation shape:

1. **Unit tier** — Scenario fixture setup + harness invocation + hard assertion, using `MockLlmProvider` seeded with canned responses. Runs on every CI push (no gate). Covers: setup correctness, assertion wiring, harness plumbing for the scenario. Does NOT cover: real quality.
2. **Integration tier** — Same scenarios against real providers via `MIKA_EVAL_REAL_PROVIDERS` + `--ignored`. Runs on-demand or on scheduled CI. Covers: model-specific response quality, hard-assertion survival against natural variation.
3. **Calibration tier** — Integration tier run with `MIKA_EVAL_CALIBRATE=1` (per #338 D7) capturing outcomes to the artifact file. Runs weekly via #742 maintenance loop. Covers: drift detection.

Each scenario file has a single `fn scenario_X()` body parameterized by the provider; unit tier invokes with mock, integration tier invokes with real, calibration tier adds artifact capture. One scenario body, three invocation paths.

**Rationale:** One scenario body + three invocations matches #338 D3's "scenarios as functions" pattern exactly. Duplication avoided; invocation discoverability preserved.

**Rejected alternative:** Separate scenario files per tier. Rejected — triples maintenance burden, breaks the "one file per scenario" rule from D2.

### D7 — Cost-per-scenario observability: annotate, don't cap

**Problem:** Some scenarios will be cheap (one turn, small prompt); some expensive (multi-turn with KG context, large fixture). How does a scenario author know if they're building an outlier?

**Decision:** Per-scenario metadata attribute — `#[eval_scenario(class = "memory", expected_tokens = 2_000)]` — that records the author's estimate. `eval-diff` (#338 D7) emits a per-scenario token count alongside the estimate; mismatches >2× flag in CI log output. No hard cap, no enforcement — observability only. Matches #338 D8's explicit rejection of runtime cost enforcement.

**Rationale:** Metadata-declared expectation + actual-measurement diff gives the author a feedback loop without a structural cap. If the scenario author says "2K tokens" and it measures 12K, that's a review signal; if they say "20K tokens" and it measures 22K, that's fine.

**Rejected alternative:** Automatic per-scenario enforcement. See #338 D8 rationale.

### D8 — Docs: one eval README + one CLAUDE.md section, no separate doc tree

**Problem:** Where does "how to add a scenario," "how to interpret results," "how to run locally vs in CI" live?

**Decision:** One markdown file at `crates/mika-agent/tests/eval/golden/README.md` covering author-facing guidance (fixture patterns, assertion style, judge-tag vocabulary, how to add a scenario). One section in `crates/mika-agent/CLAUDE.md` "Agent Loop > Evaluation" covering architectural integration (how scenarios interact with the harness, the three-tier execution model, relationship to #338 + #740 + #741 + #742). No separate `docs/` tree for eval.

**Rationale:** Test authors find the README next to the code; architectural context belongs in CLAUDE.md where it's auto-loaded by Claude Code sessions. Splitting across a third location (e.g., `docs/eval/`) creates discovery friction.

**Rejected alternative:** Dedicated `docs/eval/` tree. Rejected — three places to keep in sync without a structural reason.

## Acceptance Criteria

- [ ] 25 scenario files under `crates/mika-agent/tests/eval/golden/`, distributed per D1 (8 memory, 8 tool-selection, 5 conversation-quality, 4 skill-specific).
- [ ] Every scenario has ≥1 hard assertion (regression-gating) and ≥0 soft-tag judge output.
- [ ] Each scenario runs in three tiers per D6: unit (mock, on-push), integration (real provider, `#[ignore]` + env gate), calibration (integration + artifact capture).
- [ ] Scoring framework implemented: `ScenarioOutcome { hard_assertions, soft_tags, tokens_measured, tokens_expected, duration_ms }`.
- [ ] Judge-tag vocabulary defined in README with canonical tags (concise, uncertain, actionable, verbose, hallucinated-ref, off-topic).
- [ ] Baseline captured during merge PR: `claude-sonnet-4-6` run, uploaded as workflow artifact, snapshot copy-pasted into PR description.
- [ ] `crates/mika-agent/tests/eval/golden/README.md` covers author-facing guidance.
- [ ] `crates/mika-agent/CLAUDE.md` eval section updated with three-tier model and relationships to sibling tickets.
- [ ] `cargo test -p mika-agent --test eval` green (unit tier only).
- [ ] Integration tier green when invoked with `MIKA_EVAL_REAL_PROVIDERS=anthropic` + `--ignored` + `MIKA_ANTHROPIC_API_KEY`.
- [ ] `cargo clippy` clean.

## Dependencies

- Blocked by #338 — needs the matrix runner (D3), real-provider gating (D1), calibration mode (D7), and `eval-diff` CLI. Specifically: the three-tier execution model in D6 relies on #338's `scenarios as async fn` pattern being final.

## Downstream

- **mika#742** — consumes this ticket's baseline as the reference for weekly drift detection
- Future model-calibration tickets (one per additional provider under test) will cite specific scenarios from this dataset

## Cross-cutting notes

- Judge-tag vocabulary is the interface to #740/#741: their scenarios MAY emit tags into the same vocabulary, so a unified calibration artifact can aggregate across the full eval surface. Vocabulary is frozen in this ticket's README; changes require a sibling ticket update.
- Per-scenario `expected_tokens` annotation in D7 is the observability surface for cost awareness (referenced from #338 D8's rejection of the cost-table approach).
- Scenario naming (`{class}_{shape}_{descriptor}`) is opinionated — see D2. Any deviation in #740/#741 gets called out in review.

## Open questions (for Vincent before dispatch)

1. **D1 count distribution** — 8/8/5/4 weighted toward memory and tool selection. Alternative weighting: 6/6/6/7 (balanced) or 10/10/3/2 (memory + tools heavier, conversation quality lighter). My weighting reflects "memory and tool selection are the highest-leverage surfaces for user-visible quality" — worth a sanity check.
2. **D4 soft-assertion judge model** — should the judge model be pinned (e.g., always `claude-sonnet-4-6`) or configured per env (`MIKA_EVAL_JUDGE_MODEL`)? Pinning stabilizes tag output across runs; config gives flexibility but introduces judge-drift. I lean pinned with an optional override env var for offline work.
3. **D5 baseline-in-PR-description** — acceptable one-off, or does this deserve its own structured format (e.g., `baselines/2026-04-22-sonnet-4-6.md` in the repo, human-readable but deliberately NOT the machine-compared baseline)? My default is PR-description-only for now; structured file adds maintenance.
4. **D7 `#[eval_scenario]` attribute macro** — adding a proc macro crate for one attribute is overkill. Fine to use a const + match at registration time (`register_scenario("memory_recall_cross_session", class: Memory, expected_tokens: 2_000)`) instead? Matches the existing skill-registration idiom more closely.
