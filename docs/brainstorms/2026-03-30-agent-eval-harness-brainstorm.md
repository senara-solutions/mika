# Brainstorm: Agent Eval & Testing Harness

**Date:** 2026-03-30
**Status:** Draft
**Origin:** Synthesis of two external proposals (ChatGPT + Claude) adapted to Mika's Rust architecture

## What We're Building

A Rust-native eval and testing harness for Mika's agent loop that fills the biggest gap in the test suite: **zero integration tests exercise the full `run_loop()` today**. All 1,821 existing tests are unit-level (tool execution, DB operations, prompt building).

The harness operates in **dual mode**:

1. **Mock CI mode** — `MockLlmProvider` with scripted responses for deterministic, fast `cargo test` integration tests. Validates tool selection, arguments, step counts, and call ordering.
2. **Real provider eval matrix** — runs the same scenarios against each configured provider (Anthropic, OpenAI, OpenRouter, Groq, etc.) to compare behavior across providers and models. On-demand, not in CI.

### Core capabilities

1. **Mock `LlmProvider`** — scripted sequence of `LlmResponse` values, no network calls
2. **Drive the full agent loop** — `run_agent()` end-to-end with mock or real LLM + in-memory DB
3. **Capture structured traces** — tool calls, arguments, ordering, final output, errors, provider/model metadata
4. **Assert on behavior** — tool selection, argument correctness, step counts, call ordering
5. **Support regression snapshots** — record traces, compare against baselines to detect drift
6. **Multi-provider behavioral comparison** — same input across providers, surface divergences
7. **Skill variant coverage tracking** — flag untested provider/model combinations

## Why This Approach

### Rust-native over Python external

Both external proposals assumed Python. We chose Rust-native because:
- Mika already has `TestHarness`, in-memory SQLite, and the `Tool` trait — reuse, don't rewrite
- `LlmProvider` is already `dyn`-dispatched in `run_loop()` — mock injection requires zero refactoring
- Type safety catches test case errors at compile time
- Runs in `cargo test` — no separate test runner, no running server required
- `llm_calls` + `tool_calls` DB tables already record every execution — traces are built-in

### Sequence-based mock over pattern-matching or replay

- Simple: `Vec<LlmResponse>` popped in order, panics if exhausted
- Predictable: each test step maps 1:1 to a mock response
- Readable: test author sees the exact conversation flow inline
- No brittleness from prompt changes (unlike recorded replay)

### Rust code tests over YAML/JSON datasets

- Type-safe with IDE support and refactor-friendly
- Builder pattern for ergonomic test case construction
- No custom parser needed
- Compile-time checks on tool names, argument shapes

### Dual mode for multi-provider coverage

Mika supports 11 LLM providers, skills have per-provider/per-model prompt variants, and individual skills can override the LLM provider entirely. A single mock can't catch:
- Provider-specific tool calling quirks (e.g., argument formatting differences)
- Skill prompt variants that work for Anthropic but break on OpenAI
- Regressions introduced by provider API changes

Mock mode handles CI (fast, deterministic). Real provider matrix handles calibration (thorough, on-demand).

## Key Decisions

1. **Language:** Rust, in `mika-agent` crate (`tests/eval/` integration test directory)
2. **Mock style:** Sequence-based `MockLlmProvider` implementing `LlmProvider` trait
3. **Test format:** `#[tokio::test]` functions with builder pattern
4. **Trace capture:** Leverage existing `llm_calls`/`tool_calls` DB tables + in-memory struct
5. **Scope:** Full stack — mock provider, loop tests, assertions, snapshots, provider matrix
6. **AgentParams:** `EvalHarness` builder with sensible defaults (extends `TestHarness` pattern)
7. **Dry-run mode:** Skip for now — mock provider already gives deterministic tool selection testing
8. **Multi-provider:** Dual mode — mock for CI, real provider matrix on-demand

## Design Sketch

### MockLlmProvider

```rust
struct MockLlmProvider {
    responses: Mutex<VecDeque<LlmResponse>>,
    recorded_requests: Mutex<Vec<LlmRequest>>,
    model: String,
    provider_name: String,
}

impl MockLlmProvider {
    fn new(responses: Vec<LlmResponse>) -> Self { ... }
    fn with_provider(mut self, name: &str, model: &str) -> Self { ... }
    fn requests(&self) -> Vec<LlmRequest> { ... } // inspect what was sent
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    async fn send_message(&self, request: &LlmRequest) -> Result<LlmResponse, LlmError> {
        self.recorded_requests.lock().push(request.clone());
        self.responses.lock().pop_front()
            .ok_or_else(|| LlmError::Other("mock exhausted".into()))
    }
    fn provider_name(&self) -> &str { &self.provider_name }
    fn model_name(&self) -> &str { &self.model }
    // ...
}
```

### AgentTrace

```rust
struct AgentTrace {
    input: String,
    steps: Vec<TraceStep>,
    final_output: Option<String>,
    errors: Vec<String>,
    // Provider context
    provider: String,
    model: String,
    // Skill context
    matched_skills: Vec<String>,
    skill_variant_used: Option<String>, // e.g., "anthropic/claude-sonnet-4"
}

enum TraceStep {
    LlmCall {
        request_summary: String,
        response_summary: String,
        usage: Option<LlmUsage>,
        latency_ms: Option<u64>,
    },
    ToolCall {
        name: String,
        args: Value,
        result: ToolOutput,
        source: ToolSource, // Builtin, Skill, Mcp
    },
}
```

Built from the `tool_calls` and `llm_calls` DB tables after a run, or captured inline via instrumentation.

### EvalHarness (builder pattern)

```rust
// Mock mode (CI)
let trace = EvalHarness::new()
    .with_responses(vec![
        mock_tool_call("search_products", json!({"query": "ikea chair"})),
        mock_end_turn("Here are some IKEA chairs..."),
    ])
    .with_tools(vec![search_products_tool()])
    .run("find me an ikea chair")
    .await;

assert_tools(&trace, &["search_products"]);
assert_max_steps(&trace, 2);
assert_tool_args(&trace, "search_products", 0, json!({"query": "ikea chair"}));

// Real provider mode (on-demand eval)
let traces = EvalHarness::new()
    .with_real_providers(&["anthropic", "openai", "groq"])
    .with_tools(vec![search_products_tool()])
    .with_skills(vec!["product-search"])
    .run_matrix("find me an ikea chair")
    .await;

// Compare behavior across providers
let report = ProviderComparisonReport::from_traces(&traces);
assert!(report.tool_selection_consistent()); // same tools selected by all
report.print_divergences(); // surface differences
```

### Assertion Helpers

```rust
fn assert_tools(trace: &AgentTrace, expected: &[&str]);
fn assert_tool_not_called(trace: &AgentTrace, tool_name: &str);
fn assert_tool_order(trace: &AgentTrace, expected_order: &[&str]);
fn assert_max_steps(trace: &AgentTrace, max: usize);
fn assert_tool_args(trace: &AgentTrace, tool_name: &str, call_index: usize, expected: Value);
fn assert_no_unknown_tools(trace: &AgentTrace, allowed: &[&str]);
fn assert_no_errors(trace: &AgentTrace);

// Multi-provider assertions
fn assert_consistent_tools(traces: &[AgentTrace]); // all providers selected same tools
fn assert_consistent_tool_order(traces: &[AgentTrace]);
```

### Provider Comparison Report

```rust
struct ProviderComparisonReport {
    input: String,
    traces: Vec<AgentTrace>,
}

impl ProviderComparisonReport {
    fn tool_selection_consistent(&self) -> bool { ... }
    fn divergences(&self) -> Vec<Divergence> { ... }
    fn print_summary(&self) { ... }
}

enum Divergence {
    ToolMismatch { provider: String, expected: Vec<String>, actual: Vec<String> },
    ArgDifference { provider: String, tool: String, field: String, values: Vec<Value> },
    StepCountDifference { provider: String, steps: usize, baseline: usize },
    SkillVariantMissing { provider: String, model: String, skill: String },
}
```

### Skill Variant Coverage

```rust
struct VariantCoverage {
    skill: String,
    root_prompt: bool,
    provider_variants: HashMap<String, Vec<String>>, // provider -> [models]
    tested_combinations: HashSet<(String, String)>,   // (provider, model) pairs
    untested: Vec<(String, String)>,
}

fn check_variant_coverage(
    skills_dir: &Path,
    active_providers: &[(&str, &str)], // (provider, model) pairs in use
) -> Vec<VariantCoverage>;
```

Scans `skills/` directories for variant folders, cross-references against configured providers, flags gaps.

### Regression Snapshots

```rust
// Record
let trace = harness.run("find me an ikea chair").await;
trace.save_snapshot("tests/snapshots/search_products.json");

// Compare
let old = AgentTrace::load_snapshot("tests/snapshots/search_products.json");
let similarity = trace.compare(&old); // 0.0 - 1.0
assert!(similarity > 0.9);

// Provider-specific snapshots
let traces = harness.run_matrix("find me an ikea chair").await;
for trace in &traces {
    trace.save_snapshot(&format!(
        "tests/snapshots/search_products_{}.json",
        trace.provider
    ));
}
```

## What We Take From Each Proposal

### From ChatGPT (report.md)
- AgentTrace structure (adapted to Rust with provider/skill metadata)
- Assertion helper pattern (assert_tools, assert_max_steps, etc.)
- FakeLLM concept (-> MockLlmProvider)
- Guardrails (max steps, required args, no empty calls)
- Regression snapshot comparison with similarity scoring

### From Claude (AGENT_EVAL_HARNESS_SPEC.md)
- Rich assertion vocabulary (tool_called, tool_not_called, tool_args with partial match, tool_call_order)
- Trace model detail (capture requests sent to LLM, not just responses)
- AgentUnderTest protocol concept (-> our EvalHarness wrapping AgentParams)
- Mock replay mode (-> MockLlmProvider sequence)
- Tags/filtering concept (-> Rust test attributes + module organization)
- `--repeat N` concept (-> adapted for real provider evals where non-determinism exists)

### Mika-specific additions (not in either proposal)
- Multi-provider eval matrix with behavioral comparison
- Skill variant coverage tracking across provider/model combinations
- Provider-specific regression snapshots
- Awareness of per-skill LLM overrides (`[llm]` section in `skill.toml`)
- Trace includes `matched_skills` and `skill_variant_used` for debugging prompt variant issues

### What We Skip (for now)
- Python harness, YAML test cases, CLI runner (Rust-native instead)
- LLM-as-judge (can add later for real provider evals)
- Web dashboard for results (cargo test output + JSON snapshots suffice)
- Dry-run mode (mock provider covers this use case)

## Resolved Questions

1. **Where does the code live?** -> `mika-agent/tests/eval/` (integration test directory). Simpler, no new crate. `cargo test` discovers it automatically. The harness only tests `mika-agent`, so a separate crate isn't justified.

2. **Dry-run mode?** -> Skip for now. `MockLlmProvider` already gives deterministic tool selection testing. Dry-run would require modifying `run_loop()` for marginal benefit. Can revisit later.

3. **AgentParams construction?** -> `EvalHarness` builder with sensible defaults. Extends `TestHarness` pattern. Builder fills in dummy `session_id`, in-memory DB, empty skills, test `home_dir`. Only LLM responses and tools need explicit setup per test.

4. **Multi-provider testing?** -> Dual mode. Mock for CI (fast, deterministic). Real provider matrix on-demand for calibration. Same scenarios, different providers, behavioral comparison report.

5. **What to observe across providers?** -> Three dimensions: tool selection consistency (same tools for same input), full behavioral comparison (args, steps, output), and variant coverage tracking (flag untested provider/model combos).
