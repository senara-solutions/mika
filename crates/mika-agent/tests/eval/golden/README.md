# Golden Dataset — End-to-End Quality Testing

Reference: [mika#339](https://github.com/senara-solutions/mika/issues/339), `docs/plans/339-golden-dataset.md`

## Overview

25 curated scenarios that verify Mika gives good answers, not just that the plumbing works. Each scenario tests a specific capability against mock or real LLM providers.

**Distribution (D1):**

| Class | Count | Scenarios |
|-------|-------|-----------|
| Memory | 8 | recall, variations, disambiguation, time-based, updates, cross-session, privacy, category routing |
| Tool Selection | 8 | calendar, memory search, GitHub, messaging, conflicts, no hallucination, multi-step, multi-turn planning |
| Conversation Quality | 5 | follow-up context, uncertainty admission, conciseness, rewind semantics, compaction survival |
| Skill-Specific | 4 | self-dev plan coherence, qa-review bug catch, google-workspace, run_gh formatting |

## Three-Tier Execution Model (D6)

Each scenario runs in three tiers:

### Tier 1: Unit (Mock)

Runs on every CI push. Uses `MockLlmProvider` with canned responses. Tests agent behavior wiring — does the agent call the right tools in the right order?

```bash
cargo test -p mika-agent --test eval golden
```

### Tier 2: Integration (Real Provider)

Runs on-demand. Uses real LLM providers via `MIKA_EVAL_REAL_PROVIDERS`. Tests actual model response quality.

```bash
MIKA_EVAL_REAL_PROVIDERS=anthropic \
MIKA_ANTHROPIC_API_KEY=sk-... \
cargo test -p mika-agent --test eval golden -- --ignored
```

### Tier 3: Calibration (Artifact Capture)

Integration tier with `MIKA_EVAL_CALIBRATE=1`. Writes calibration artifacts to `target/eval-calibration/`. Used for weekly drift detection (#742).

```bash
MIKA_EVAL_CALIBRATE=1 \
MIKA_EVAL_REAL_PROVIDERS=anthropic \
MIKA_ANTHROPIC_API_KEY=sk-... \
cargo test -p mika-agent --test eval golden -- --ignored
```

## Scoring Framework (D4)

### Hard Assertions

Pass/fail checks that gate regressions. Every scenario MUST have at least one.

```rust
assert_tools_include(&trace, &["search_memory"]);
assert_output_contains(&trace, "March");
assert_tool_order(&trace, &["search_memory", "store_fact"]);
```

### Soft Tags

LLM-judge quality signals in the `quality:*` namespace. NOT regression-gating — observability only.

**Vocabulary:**

| Tag | Meaning |
|-----|---------|
| `quality:concise` | Response is appropriately brief |
| `quality:verbose` | Response is unnecessarily long |
| `quality:uncertain` | Response admits uncertainty |
| `quality:actionable` | Response provides clear next steps |
| `quality:off-topic` | Response doesn't address the question |

**Namespace ownership:** #339 owns `quality:*` only. Sibling tickets own their namespaces:
- #740: `self-knowledge:*`
- #741: `grounding:*`

### Judge Model

Pinned to `claude-sonnet-4-6` for baseline stability. Override via `MIKA_EVAL_JUDGE_MODEL` env var.

**Judge-deprecation reset protocol:** When the pinned model is EOL'd by the provider:
1. A new PR explicitly documents the judge transition
2. All soft-assertion baselines reset
3. The reset is flagged as a catalog reset, not a drift event
4. The new judge model + version are recorded in every calibration artifact header

## How to Add a Scenario

1. Create `golden/{class}_{shape}_{descriptor}.rs`:

```rust
//! Golden scenario: [description]
//!
//! Class: [Memory|ToolSelection|ConversationQuality|SkillSpecific] | Expected tokens: [N]

use super::*;

pub fn register(registry: &GoldenRegistry) {
    registry.register(
        "scenario_name",
        GoldenScenarioMeta {
            class: ScenarioClass::Memory, // or ToolSelection, etc.
            expected_tokens: 2000,
            description: "What this scenario tests",
        },
    );
}

#[tokio::test]
async fn test_scenario_name() {
    let harness = EvalHarness::builder()
        .responses(vec![
            tool_call_response("search_memory", json!({"query": "test"})),
            text_response("Here's what I found."),
        ])
        .build()
        .await
        .unwrap();

    let trace = harness.run("User question").await.unwrap();

    // Hard assertions (at least one required)
    assert_tools_include(&trace, &["search_memory"]);
    assert_has_output(&trace);
}
```

2. Add `pub mod scenario_name;` to `golden/mod.rs`
3. Add `scenario_name::register(&registry);` in `default_golden_registry()`
4. Run `cargo test -p mika-agent --test eval golden`

### Naming Convention (D2)

`{class}_{shape}_{descriptor}.rs`

- `class`: `memory`, `tool_selection`, `conversation_quality`, `skill`
- `shape`: what kind of assertion (recall, create, admit, catch, etc.)
- `descriptor`: specific scenario (single_fact, two_plausible, uncertainty)

Examples: `memory_recall_cross_session.rs`, `tool_selection_calendar_vs_memory.rs`

### Scenario Registration (D7)

Every scenario MUST call `register()` with metadata. The `GoldenRegistry` has a compile-time-equivalent uniqueness guard — duplicate names panic at test binary load time:

```
"Duplicate scenario name 'foo' — did you copy-paste without renaming?"
```

Set `expected_tokens` based on your scenario's complexity. `eval-diff` flags mismatches >2x in CI log output.

## Fixture Patterns (D3)

Fixtures are Rust setup code, NOT YAML/JSON definitions.

**No DB pre-seeding.** The `MockLlmProvider` controls the agent's "script" — what tools it calls and what text it returns. The mock sequence defines behavior, not DB state.

**Multi-turn:** Use `harness.run_turn()` with fresh responses for subsequent turns:

```rust
let trace1 = harness.run("First message").await.unwrap();
let trace2 = harness.run_turn(
    "Second message",
    vec![text_response("Response")],
).await.unwrap();
```

**Tool-dependent scenarios:** Use `.github_token("test")` or `.brave_api_key("test")` on the builder for tools that require credentials.

## Interpreting Results

### Unit Tier

All 25 scenarios should pass. A failure means the agent loop wiring changed — a mock response sequence that used to work no longer does. Fix the scenario or the agent code.

### Integration Tier

Some scenarios may fail with specific providers due to model quirks. This is expected. The calibration artifact tracks which scenarios pass/fail per provider.

### Cost Envelope

| Class | Count | Avg cost/scenario | Class subtotal |
|-------|-------|-------------------|----------------|
| Memory | 8 | ~$0.02 | ~$0.16 |
| Tool Selection | 8 | ~$0.02 | ~$0.16 |
| Conversation Quality | 5 | ~$0.03 | ~$0.15 |
| Skill-Specific | 4 | ~$0.04 | ~$0.16 |
| **Total (single provider)** | **25** | — | **~$0.63** |
| **Full matrix (4 providers)** | — | — | **~$2.52** |

## Related Tickets

- **#338** — Eval harness Phase 2 (multi-provider mechanics). This ticket depends on #338.
- **#740** — KG-backed self-knowledge scenarios (`self-knowledge:*` namespace)
- **#741** — Grounding/fabrication regression scenarios (`grounding:*` namespace)
- **#742** — Regression monitoring and weekly drift detection (consumes this ticket's baseline)
