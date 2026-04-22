# Plan — mika#339 — Golden dataset: end-to-end quality testing with real LLM providers

**Issue:** senara-solutions/mika#339
**Branch:** `feat/339/golden-dataset`
**Milestone:** Evaluation (#16)
**Blocked by:** `#338` at plan commit **`fa54d950`** (multi-provider machinery required for real-API scenarios). Pinned SHA so downstream drift in #338's shape is a grep-findable version bump rather than implicit.
**Status:** Groomed draft — pending Vincent review

## Context

#338 ships the harness. This ticket ships what the harness measures — a curated dataset of scenarios that verify Mika produces good answers, not just that the plumbing works. The spec from the issue body: "Mock tests prove the plumbing works. This proves the water tastes good."

Mika today has zero end-to-end quality tests against real providers. Prompt changes, model migrations, skill edits ship blind until a user complains. The `task_id` incident (latent six weeks before provider switch surfaced it) is one class; silent quality degradation on memory recall or tool selection is another class that no harness-level test catches.

#339 is the **broadest** of the three downstream scenario tickets. #740 narrows to KG-backed self-knowledge (Path A/B/C of `query_knowledge_graph`). #741 narrows to grounding/fabrication regressions (four specific incidents from the KG retrospective). #339 covers the remaining scenario surface — memory, tool selection, multi-turn, conversation quality — without duplicating the narrower siblings.

## Scope boundary

**In scope:**
- 25 curated scenarios across 4 capability classes (memory, tool selection, conversation quality, skill-specific non-KG / non-grounding)
- **Multi-turn planning scenarios** (goal decomposition → step execution → adaptation on failure) — classified inside **tool selection**, exercising the conflict-case + multi-step-sequence sub-surface
- Scoring framework (hard assertions + soft assertions)
- Baseline score establishment for at least one configured model tuple
- Per-scenario fixture seeding (known facts, reminders, work items, skills)
- Documentation on how to run, add scenarios, interpret results

**Out of scope (spun out or deferred):**
- KG-backed self-knowledge scenarios → `#740`
- Grounding / fabrication regressions → `#741`
- JSON-schema divergence tests → `#338` (D4/D9)
- Per-skill provider override tests → `#338` (D5)
- Regression gating on baseline drift → `#742` (CI maintenance loop)
- Langfuse dashboard integration — explicit non-goal
- **Cross-ticket judge-tag vocabulary authority** — each ticket owns its own namespaced vocabulary (see D4). #339 owns `quality:*` only.

## Decisions

### D1 — Scenario count: 25, not 20-30 — commit to a target

**Problem:** The issue body says "20-30 representative scenarios." Ranges in plans invite scope creep.

**Decision:** Target **25 scenarios** for the first baseline. Initial distribution:
- **Memory: 8 scenarios** — store/recall across variations, multi-fact disambiguation, time-based retrieval, fact updates, privacy boundaries
- **Tool selection: 8 scenarios** — calendar invocations, memory search routing, `run_gh` targeting, `send_message` delivery, conflict cases (two plausible tools), **multi-turn planning (goal decomposition + step execution + failure-recovery adaptation)**
- **Conversation quality: 5 scenarios** — follow-up context, uncertainty admission (non-fabrication baseline), concise-vs-verbose response calibration, rewind/undo semantics, compaction survival
- **Skill-specific (non-KG, non-grounding): 4 scenarios** — self-dev plan-generation coherence, qa-review bug-catching, google-workspace calendar query structure, `run_gh` PR-body formatting

**Initial seed, not terminal answer.** The 8/8/5/4 shape reflects a prior about user-visible quality surfaces (memory drift, wrong-tool selection, compaction eating context) drawn from incident recall — it is NOT validated against actual regression-catch rates. **Distribution review after 3 months of `#742` calibration data** against the question "which scenario classes flagged provider drift or agent regressions?" If 8 memory scenarios all green-passed every run while 4 skill-specific ones caught all regressions, the initial seed was wrong and gets rebalanced. Plan commits to the rebalance review, not to the distribution being correct a priori.

**Scenario count review.** The 25 total is also a guess. After the first baseline PR lands, the count is reviewed against authoring friction: if the PR took 3+ weeks of grind and scenarios got thin to hit the count, 25 was too high; if #742's drift detection surface seems narrow, 25 was too low. Named feedback loop, not a silent calcification.

**Rationale:** 25 is at the lower end of "enough for meaningful baseline, cheap enough to run." #338 explicitly rejects stating dollar figures (D8 amendment) — this count is the structural substitute. Distribution is a hypothesis under test, not a posterior.

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
- **YAML-driven scenarios** (explicitly locked against re-litigation). Explained above. Scenarios are test code; test code is Rust. A YAML catalog is the kind of thing that grows into a mini-DSL, then a mini-DSL interpreter, then a mini-DSL debugger, then regret. If a future contributor wants YAML-ness, the correct move is authoring Rust scenarios with clear fixture patterns, not building a YAML runtime.
- One fixture module shared across scenarios. Creates coupling — one scenario's fixture change breaks another.

### D4 — Scoring framework: hard assertions primary, soft assertions as LLM-judged tags in ticket-namespaced vocabulary

**Problem:** The issue body describes two scoring classes: hard assertions (tool X called / output contains Y) and soft assertions (quality score 0-10 via judge LLM). Three sub-problems: avoid score drift, avoid judge drift confounding agent signals, decide vocabulary ownership across sibling tickets.

**Decision:** Three principles.

- **Hard assertions** are pass/fail and are the regression-gating signal. Every scenario MUST have at least one hard assertion. Examples: `assert!(trace.tool_calls.iter().any(|c| c.name == "calendar_create_event"))`, `assert_contains!(response, expected_field_name)`, `assert!(response.len() < 500)`.
- **Soft assertions** are LLM-judge emitted tags in a **ticket-namespaced vocabulary** (e.g., `quality:concise`, `quality:uncertain`, `quality:actionable`) reported alongside hard assertions. They go into the calibration artifact (#338 D7) but do NOT gate pass/fail. A scenario where all hard assertions pass but soft tags indicate `quality:verbose` emits a warning, not a failure.
- **Soft-assertion judge structure:** fixed prompt template + tag vocabulary constrained to the scenario's ticket namespace. Judge output is constrained to a tag set, not a free-form 0-10 score — score drift is eliminated because there's no score to drift.

**Vocabulary namespacing.** #339 defines the `quality:*` namespace (author-facing vocabulary in the README). Sibling tickets define their own: `#740` will own `self-knowledge:*`, `#741` will own `grounding:*` (e.g., `grounding:fabricated-ref-suppressed`, `grounding:source-cited-correctly`). Calibration artifact preserves the namespace structure so aggregation across tickets works at the tooling layer, not at the vocabulary layer. This plan does NOT attempt to freeze a shared cross-ticket vocabulary — each sibling's scenarios test genuinely different things, and orthogonality beats DRY for cross-ticket interfaces.

**Judge model pinning + deprecation.** Pinned to `claude-sonnet-4-6` for baseline stability (judge drift is a second variance source that confounds agent-drift detection). Override via env `MIKA_EVAL_JUDGE_MODEL` for offline developers without Anthropic access. **When the pinned judge is EOL'd by the provider, calibration baseline explicitly resets** — a new PR documents the judge transition and re-baselines all soft-assertion tags. This is NOT a drift event; it is a catalog reset, flagged as such in the artifact history. The pinned model string and its version are **recorded in the calibration artifact itself** (not only in config), so any archived artifact self-describes its judge. Otherwise a year-later diff mistakes a judge swap for agent drift.

**Rationale:** LLM-judged 0-10 scores are the worst of both worlds: they gate (if thresholded) but can't be trusted (LLMs are noisy evaluators), or they don't gate (and become decorative). Tag-based judging with ticket-namespaced vocabulary keeps the signal without the gate, without forcing cross-ticket vocabulary authority into #339. Judge pinning stabilizes tags; deprecation-as-reset names the discontinuity so it isn't silent.

**Rejected alternatives:**
- 0-10 soft scores as originally spec'd in the issue body. Replaced per above.
- Shared global vocabulary frozen in #339. Forces #740/#741 scenarios into tags that don't fit their assertion shapes; ticket-namespacing solves this cleanly.
- Pin a judge *family* ("current flagship Sonnet tier"). Smuggles the reset behind handwaving; explicit version string + explicit reset is more structurally honest.

### D5 — Baseline establishment: one model tuple, explicit, documented

**Problem:** "Baseline scores established for at least Anthropic Claude" from the issue body is vague. Which model? At what timestamp? Where does the baseline live?

**Decision:** Initial baseline = **Anthropic `claude-sonnet-4-6`, captured during the PR that lands this ticket**. Baseline file lives in `target/eval-calibration/{timestamp}.json` (per #338 D7 ephemeral) and is uploaded as a CI workflow artifact. NOT committed to the repo until `mika#742` ships the maintenance loop.

**PR description format is grep-able, not free prose.** The merge PR's body includes a fenced block with a stable header so future tooling can extract it programmatically without a parser rewrite:

````markdown
## Eval Baseline

```json
{
  "captured_at": "2026-04-22T...",
  "scenario_model": { "provider": "anthropic", "model": "claude-sonnet-4-6" },
  "judge_model": { "provider": "anthropic", "model": "claude-sonnet-4-6" },
  "scenarios": { ... per-scenario outcomes ... }
}
```
````

The `## Eval Baseline` header + fenced JSON block is addressable via `gh pr view <N> --json body | jq` or any future extractor. Free-form prose would lock out automation even if #742's maintenance loop later wants to bootstrap from PR-description baselines.

**Rationale:** Baseline-in-PR-description is the ephemeral-but-discoverable compromise. No committed-but-unmaintained artifact; no orphaned calibration output; operators can find "what was the baseline for scenario X?" via `gh pr view <N>`. The structured format keeps the option of building extraction tooling later open without making any commitment now.

**Rejected alternatives:**
- Commit `tests/fixtures/eval-baseline.json`. Rejected for same theater reasons as in #338 D7 — deferred to #742.
- Commit a human-readable `baselines/2026-04-22-sonnet-4-6.md`. Feels diligent; becomes a pseudo-committed baseline with no maintenance loop. Same rot problem, one layer of indirection worse because it looks more authoritative than a PR description.
- Free-form prose in PR description. Rejected — locks out future automation without any upside.

### D6 — Execution model: three tiers — unit / integration / calibration

**Problem:** The issue spec mixes `#[ignore]` gating, env vars, and "when to run." Needs decomposition.

**Decision:** Three tiers, each with a distinct invocation shape:

1. **Unit tier** — Scenario fixture setup + harness invocation + hard assertion, using `MockLlmProvider` seeded with canned responses. Runs on every CI push (no gate). Covers: setup correctness, assertion wiring, harness plumbing for the scenario. Does NOT cover: real quality.
2. **Integration tier** — Same scenarios against real providers via `MIKA_EVAL_REAL_PROVIDERS` + `--ignored`. Runs on-demand or on scheduled CI. Covers: model-specific response quality, hard-assertion survival against natural variation.
3. **Calibration tier** — Integration tier run with `MIKA_EVAL_CALIBRATE=1` (per #338 D7) capturing outcomes to the artifact file. Runs weekly via #742 maintenance loop. Covers: drift detection.

Each scenario file has a single `fn scenario_X()` body parameterized by the provider; unit tier invokes with mock, integration tier invokes with real, calibration tier adds artifact capture. One scenario body, three invocation paths.

**Rationale:** One scenario body + three invocations matches #338 D3's "scenarios as functions" pattern exactly. Duplication avoided; invocation discoverability preserved.

**Rejected alternative:** Separate scenario files per tier. Rejected — triples maintenance burden, breaks the "one file per scenario" rule from D2.

### D7 — Cost-per-scenario observability: register-function metadata with uniqueness guard

**Problem:** Some scenarios will be cheap (one turn, small prompt); some expensive (multi-turn with KG context, large fixture). How does a scenario author know if they're building an outlier?

**Decision:** Per-scenario metadata registered via a plain function call at module init — `register_scenario("memory_recall_cross_session", EvalScenarioMeta { class: Memory, expected_tokens: 2_000 })`. No proc-macro crate (compile-time tax, IDE edge cases, cargo-expand debugging overhead for zero ergonomic win). Matches the skill-registration idiom in `crates/mika-agent/src/skills/index.rs`.

**Compile-time uniqueness guard.** `register_scenario` internally `HashMap::insert`s the name; if `Some` is returned (duplicate), panics at init time with a clear error: `"Duplicate scenario name '<name>' — did you copy-paste without renaming?"`. Structural guardrail against the silent-overwrite bug where a copy-pasted scenario overwrites an earlier one because the author forgot to rename. The guard fires on test-binary load, before any scenario runs, so the failure is loud and immediate.

`eval-diff` (#338 D7) emits a per-scenario token count alongside the registered `expected_tokens`; mismatches >2× flag in CI log output. No hard cap, no enforcement — observability only. Matches #338 D8's explicit rejection of runtime cost enforcement.

**Rationale:** Metadata-declared expectation + actual-measurement diff gives the author a feedback loop without a structural cap. If the scenario author says "2K tokens" and it measures 12K, that's a review signal; if they say "20K tokens" and it measures 22K, that's fine. Const+match + uniqueness guard is the structural-over-prompt principle applied to scenario registration: a copy-paste mistake becomes a loud test-setup failure rather than a silent test replacement.

**Rejected alternatives:**
- `#[eval_scenario(expected_tokens = 2_000)]` proc-macro attribute. Proc macros are a tax (compile time, IDE smoothness, cargo-expand debugging, one more crate) for a one-line metadata registration. Zero ergonomic win.
- Automatic per-scenario runtime enforcement. See #338 D8 rationale.

### D8 — Docs: one eval README + one CLAUDE.md section, no separate doc tree

**Problem:** Where does "how to add a scenario," "how to interpret results," "how to run locally vs in CI" live?

**Decision:** One markdown file at `crates/mika-agent/tests/eval/golden/README.md` covering author-facing guidance (fixture patterns, assertion style, judge-tag vocabulary, how to add a scenario). One section in `crates/mika-agent/CLAUDE.md` "Agent Loop > Evaluation" covering architectural integration (how scenarios interact with the harness, the three-tier execution model, relationship to #338 + #740 + #741 + #742). No separate `docs/` tree for eval.

**Rationale:** Test authors find the README next to the code; architectural context belongs in CLAUDE.md where it's auto-loaded by Claude Code sessions. Splitting across a third location (e.g., `docs/eval/`) creates discovery friction.

**Rejected alternative:** Dedicated `docs/eval/` tree. Rejected — three places to keep in sync without a structural reason.

## Acceptance Criteria

- [ ] 25 scenario files under `crates/mika-agent/tests/eval/golden/`, initial distribution per D1 (8 memory, 8 tool-selection incl. multi-turn planning, 5 conversation-quality, 4 skill-specific). Distribution review committed at 3-month mark against `#742` calibration data.
- [ ] Every scenario has ≥1 hard assertion (regression-gating) and ≥0 soft-tag judge output.
- [ ] Each scenario runs in three tiers per D6: unit (mock, on-push), integration (real provider, `#[ignore]` + env gate), calibration (integration + artifact capture).
- [ ] Scoring framework implemented: `ScenarioOutcome { hard_assertions, soft_tags, tokens_measured, tokens_expected, duration_ms }`.
- [ ] Judge-tag vocabulary defined in README, **namespaced to `quality:*`** (`quality:concise`, `quality:uncertain`, `quality:actionable`, `quality:verbose`, `quality:off-topic`). Sibling tickets own their own namespaces — #339 does NOT define `self-knowledge:*` or `grounding:*`.
- [ ] Judge model pinned to `claude-sonnet-4-6` with `MIKA_EVAL_JUDGE_MODEL` env override for offline developers. Judge model + version recorded in every calibration artifact header. **Judge-deprecation reset protocol** documented in README: when pinned model is EOL'd, baseline resets via explicit PR and the reset is flagged (not drift) in artifact history.
- [ ] Baseline captured during merge PR: `claude-sonnet-4-6` scenario run + judge. **Stable grep-able format:** PR description contains `## Eval Baseline` header with fenced `json` block, extractable via `gh pr view <N> --json body | jq`. No free-form prose in the baseline section.
- [ ] `register_scenario(name, meta)` has **compile-time-equivalent uniqueness guard** (panic on init with duplicate scenario names). Protects against copy-paste-without-rename silent overwrites.
- [ ] `crates/mika-agent/tests/eval/golden/README.md` covers author-facing guidance including fixture patterns, assertion style, `quality:*` tag vocabulary, scenario registration, and judge-deprecation reset protocol.
- [ ] `crates/mika-agent/CLAUDE.md` eval section updated with three-tier model and relationships to sibling tickets (including the ticket-namespaced vocabulary structure).
- [ ] `cargo test -p mika-agent --test eval` green (unit tier only).
- [ ] Integration tier green when invoked with `MIKA_EVAL_REAL_PROVIDERS=anthropic` + `--ignored` + `MIKA_ANTHROPIC_API_KEY`.
- [ ] `cargo clippy` clean.

## Dependencies

- Blocked by #338 — needs the matrix runner (D3), real-provider gating (D1), calibration mode (D7), and `eval-diff` CLI. Specifically: the three-tier execution model in D6 relies on #338's `scenarios as async fn` pattern being final.

## Downstream

- **mika#742** — consumes this ticket's baseline as the reference for weekly drift detection
- Future model-calibration tickets (one per additional provider under test) will cite specific scenarios from this dataset

## Cost envelope (design-time, class-average)

Matching the design-time pricing discipline from `#740` and `#741`, but applied at class-level rather than per-scenario (25 scenarios is too speculative to price individually before implementation). Class-average rates reflect typical scenario shapes:

| Class | Count | Avg cost / scenario (single provider) | Class subtotal |
|---|---|---|---|
| Memory | 8 | ~$0.02 (single-turn, modest context) | ~$0.16 |
| Tool selection | 8 | ~$0.02 (single-turn, multi-step variant at top end) | ~$0.16 |
| Conversation quality | 5 | ~$0.03 (multi-turn, more context carried) | ~$0.15 |
| Skill-specific | 4 | ~$0.04 (self-dev/qa-review prompts are longer) | ~$0.16 |
| **Per-provider integration run** | **25** | — | **~$0.63** |

Against `#338`'s four-provider matrix ({Anthropic, OpenAI, Kimi, Groq}), full-matrix integration run ≈ **~$2.52**. Authors set per-scenario `expected_tokens` metadata (D7) at implementation; those refine the class-average into real numbers. The plan-level class bound is the acknowledged ceiling — if an author lands a scenario that measurably breaks their class average by >2×, `eval-diff` flags it in CI logs (per `#338` D7 token-count observability).

**Bound honesty:** these numbers rot. They exist as *design-time cost envelope decisions*, not as runtime guarantees. The structural enforcement for cost is `MIKA_EVAL_REAL_PROVIDERS` + `#[ignore]` + workflow timeout per `#338` D8. This class-average table is the asymmetric-pricing resolution for a large-ticket plan — small tickets price per-scenario, large tickets price per-class.

## Cross-cutting notes

- **Judge-tag vocabulary is ticket-namespaced, not globally frozen.** #339 owns `quality:*` only. #740 will define `self-knowledge:*` in its own plan; #741 will define `grounding:*`. Calibration artifact preserves namespace structure so aggregation works at the tooling layer. This ticket deliberately does NOT attempt to freeze cross-ticket vocabulary authority.
- Per-scenario `expected_tokens` metadata (D7) is the observability surface for cost awareness (referenced from #338 D8's rejection of the cost-table approach).
- Scenario naming (`{class}_{shape}_{descriptor}`) is opinionated — see D2. Any deviation in #740/#741 gets called out in review.
- `#339` is pinned to `#338` at plan commit `fa54d950`. If `#338` evolves before shipping, this plan needs explicit re-plumbing, not silent drift.

## Review log

**Vincent + friend review pass 1 (2026-04-22, relayed by Vincent):**

- **Scope clarified:** multi-turn planning scenarios (goal decomposition → step execution → adaptation) live inside tool selection, exercising the conflict-case + multi-step-sequence sub-surface. One of the 8 tool-selection slots is explicitly the multi-turn planning scenario.
- **YAML rejection locked** as a named "Rejected alternative" in D3. Scenarios are test code; test code is Rust. Friend explicitly flagged this class of pressure ("first author who wants to contribute without touching Rust") and the counter-pattern ("YAML grows mini-DSL, mini-DSL grows interpreter, interpreter grows debugger, regret").
- **D1 distribution committed as initial seed** with 3-month rebalance review against `#742` calibration data. "Which classes actually caught regressions?" is the rebalance criterion. Plan owns the feedback loop, not the posterior. Scenario count (25) also reviewed post-first-baseline-PR against authoring friction.
- **D4 judge pinning:** `claude-sonnet-4-6` pinned with `MIKA_EVAL_JUDGE_MODEL` env override. Critical addition: **judge-deprecation-as-explicit-reset protocol** — when the pinned model is EOL'd, baseline resets via new PR, flagged as a reset (not drift) in artifact history. Judge model + version recorded **in the calibration artifact itself**, so archived artifacts self-describe their judge. Otherwise a year-later diff mistakes a judge swap for agent drift.
- **D4 vocabulary namespacing:** #339 owns `quality:*` only. Sibling tickets own their own namespaces. Ticket-namespaced vocabulary beats a globally frozen one for a cross-ticket interface where scenarios test genuinely different things — orthogonality beats DRY. Reduces #339's scope by removing a decision that belongs to #740/#741.
- **D5 PR-description format locked as grep-able:** `## Eval Baseline` header + fenced `json` block. Keeps future automation hooks open without committing to build them now. Free-form prose explicitly rejected because it locks out automation with zero upside.
- **D7 registration idiom:** const+match via `register_scenario(...)` confirmed. Added **compile-time-equivalent uniqueness guard** (`HashMap::insert` panic on `Some`) so copy-paste-without-rename becomes a loud init-time failure, not a silent overwrite.
- **#338 dependency pinned to commit `fa54d950`:** plan cites the specific SHA of the #338 plan it depends on, so upstream drift is a grep-findable version bump rather than implicit breakage.

**Friend principle extended from #338:** "pin the decision you're depending on, make changes explicit as version bumps rather than implicit drift." Applied to judge model (D4), vocabulary namespace (D4), PR-description format (D5), scenario registration (D7), and upstream plan SHA.

**Milestone-level friend review pass 2 (2026-04-23, relayed by Vincent):**

- **Cost envelope added at class-average granularity.** Previous plan deferred all per-scenario pricing to implementation-time — asymmetric with `#740`/`#741` which priced at design time. Class-average pricing (25 scenarios aggregated to 4 classes) delivers a plan-level ceiling (~$0.63 per provider, ~$2.52 full-matrix) without the speculation cost of pricing 25 scenarios individually. Per-scenario `expected_tokens` refines during implementation; class bound is the acknowledged ceiling. Fed into `#741`'s milestone rollup.
