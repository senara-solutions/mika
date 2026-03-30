---
title: "feat: Agent eval & testing harness"
type: feat
status: completed
date: 2026-03-30
issue: "#329"
---

# Agent Eval & Testing Harness

## Overview

Build a Rust-native eval and testing harness for the agent loop. Currently all ~1,821 tests are unit-level — zero integration tests exercise the full `run_agent()` path. This introduces dual-mode testing: a `MockLlmProvider` for deterministic CI integration tests and a real-provider eval matrix for on-demand behavioral comparison across the 11 supported providers.

## Problem Statement

- No integration tests cover the agent loop today — biggest gap in test coverage
- 11 LLM providers with per-provider/per-model skill prompt variants need calibration
- Per-skill `[llm]` overrides can silently break when providers change behavior
- Required-tools enforcement, max-steps continuation, and step-awareness nudges are untested at the integration level
- Schema-prompt alignment drift (prompts referencing non-existent tools/fields) has caused production issues

## Proposed Solution

A two-phase approach:

**Phase 1 (this issue):** MockLlmProvider + EvalHarness + assertion helpers — deterministic CI tests exercising the full `run_agent()` path with no network calls.

**Phase 2 (follow-up):** Real-provider eval matrix, regression snapshots, similarity scoring, and skill variant coverage tracking — on-demand, not CI.

## Technical Approach

### Architecture

```
crates/mika-agent/tests/eval/          # Integration test directory
├── mod.rs                              # Shared imports, re-exports
├── harness.rs                          # EvalHarness builder
├── trace.rs                            # AgentTrace + AgentTraceBuilder
├── assertions.rs                       # Assertion helpers
├── test_basic_conversation.rs          # EndTurn without tools
├── test_tool_calling.rs                # Tool selection and execution
├── test_multi_step.rs                  # Multi-step tool chains
├── test_max_steps.rs                   # Max steps + continuation turn
├── test_error_handling.rs              # LLM errors, tool failures
└── test_required_tools.rs             # Required-tools enforcement gate

crates/mika-common/src/llm/
├── mock.rs                             # MockLlmProvider (feature-gated)
└── mod.rs                              # Re-export under #[cfg(any(test, feature = "test-utils"))]
```

### MockLlmProvider

**Location:** `crates/mika-common/src/llm/mock.rs`
**Visibility:** Behind `#[cfg(any(test, feature = "test-utils"))]` — accessible from both inline unit tests and integration tests in `tests/eval/`.

```rust
pub struct MockLlmProvider {
    responses: Vec<MockResponse>,
    cursor: AtomicUsize,
    captured_requests: Arc<Mutex<Vec<LlmRequest>>>,
    config: MockProviderConfig,
}

pub struct MockProviderConfig {
    pub provider_name: String,      // default: "mock"
    pub model_name: String,         // default: "mock-model"
    pub max_tokens: u32,            // default: 4096
    pub supports_tool_calling: bool, // default: true
    pub supports_vision: bool,       // default: false
    pub supports_extended_thinking: bool, // default: false
}

pub enum MockResponse {
    Success(LlmResponse),
    Error(LlmError),
}
```

**Key behaviors:**
- **Sequence exhaustion:** Panic with descriptive message (`"MockLlmProvider: exhausted all {n} responses at call {i}. Add more responses or verify agent behavior."`) — fast failure is better than silent degradation
- **Request capture:** All received `LlmRequest`s stored in `Arc<Mutex<Vec<LlmRequest>>>`, accessible via `captured_requests()` method
- **Thread-safe cursor:** `AtomicUsize` for parallel test compatibility (each test gets its own provider instance)
- **Error injection:** `MockResponse::Error(LlmError)` entries in the sequence simulate API failures at specific steps
- **Trait methods:** All configurable via `MockProviderConfig`, defaulting to values that enable full agent loop behavior (tool calling enabled, etc.)
- **`check_health()`:** Always returns `Ok(())`

**Builder API:**
```rust
MockLlmProvider::builder()
    .response(text_response("Hello!"))    // EndTurn text
    .response(tool_call_response("search_memory", json!({"query": "test"})))
    .response(text_response("Found it!")) // After tool result
    .error(LlmError::HttpError { status: 429, message: "rate limited".into(), retryable: true })
    .provider_name("anthropic")
    .model_name("claude-sonnet-4")
    .build()
```

**Helper constructors** (in `mock.rs`):
- `text_response(text: &str) -> MockResponse` — EndTurn with text content
- `tool_call_response(name: &str, args: Value) -> MockResponse` — ToolUse with one tool call
- `multi_tool_response(calls: Vec<(&str, Value)>) -> MockResponse` — ToolUse with multiple tool calls
- `thinking_response(text: &str, thinking: &str) -> MockResponse` — EndTurn with reasoning
- `max_tokens_response() -> MockResponse` — MaxTokens stop reason
- `content_filter_response() -> MockResponse` — ContentFilter stop reason

### EvalHarness

**Location:** `crates/mika-agent/tests/eval/harness.rs`

Builder extending the existing `TestHarness` pattern with `AgentParams` defaults.

```rust
pub struct EvalHarness {
    pub db: AsyncDatabase,
    pub mock_provider: Arc<MockLlmProvider>,
    pub tools: ToolRegistry,
    pub skills: SkillRegistry,
    pub home_dir: TempDir,
    pub session_id: String,
    pub settings: Settings,
}
```

**Builder API:**
```rust
EvalHarness::builder()
    .responses(vec![text_response("Hi!")])  // Required
    .message("Hello")                        // Required
    .tools(default_tools())                  // Optional, default: default_tools()
    .skills(SkillRegistry::empty())          // Optional, default: empty
    .session_id("test-session")              // Optional, default: UUID
    .home_dir(custom_path)                   // Optional, default: tempdir
    .is_onboarding(false)                    // Optional, default: false
    .is_callback_turn(false)                 // Optional, default: false
    .skip_compaction(true)                   // Optional, default: true (!)
    .build()
    .await                                   // Creates DB, session, temp dirs
```

**Critical defaults:**
- `skip_compaction: true` — compaction makes an additional LLM call that complicates mock sequences. Enable only when explicitly testing compaction.
- `SkillRegistry::empty()` — no skill prompt injection by default, keeping prompts deterministic
- `home_dir` — `TempDir` with minimal structure (empty directories for skills, data)
- `session_id` — unique UUID per test for isolation
- `is_onboarding: false` — standard (non-first-run) behavior
- `global_home_dir: None` — blocks cross-agent file access (not needed for most tests)
- `message_sender: None` — no outbound messages
- `embedding_client: None` — no vector search
- `trace_id: Some(uuid)` — always set for `llm_calls`/`tool_calls` correlation
- `settings.store_llm_calls: true` — required for post-run DB assertions
- `settings.store_tool_calls: true` — required for post-run DB assertions

**Run method:**
```rust
impl EvalHarness {
    pub async fn run(&self) -> Result<AgentTrace> {
        let params = self.build_agent_params();
        let output = run_agent(&params).await?;
        AgentTrace::from_run(&self.db, &self.session_id, &self.mock_provider, output).await
    }

    /// Run multiple turns on the same session (multi-turn conversation)
    pub async fn run_turn(&self, message: &str) -> Result<AgentTrace> { ... }
}
```

### AgentTrace

**Location:** `crates/mika-agent/tests/eval/trace.rs`

Post-run trace assembled from DB queries + in-memory mock data.

```rust
pub struct AgentTrace {
    pub output: AgentOutput,                    // Final text + thinking + usage
    pub llm_calls: Vec<LlmCallRecord>,          // From llm_calls table
    pub tool_calls: Vec<ToolCallRecord>,         // From tool_calls table
    pub captured_requests: Vec<LlmRequest>,      // From MockLlmProvider
    pub steps: usize,                            // Number of loop iterations
    pub provider: String,
    pub model: String,
}

pub struct LlmCallRecord {
    pub step: usize,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub stop_reason: String,
    pub latency_ms: u64,
}

pub struct ToolCallRecord {
    pub name: String,
    pub input: String,          // JSON, truncated at 50KB
    pub output: String,         // JSON, truncated at 50KB
    pub success: bool,
    pub duration_ms: u64,
    pub step: usize,
}
```

**Collection:** Queries `llm_calls` and `tool_calls` tables filtered by `trace_id`, combines with `MockLlmProvider::captured_requests()` and `AgentOutput`.

### Assertion Helpers

**Location:** `crates/mika-agent/tests/eval/assertions.rs`

All assertions operate on `&AgentTrace` and produce clear failure messages.

```rust
// Tool presence — exact set equality
pub fn assert_tools(trace: &AgentTrace, expected: &[&str]);
// Tool presence — subset (at least these tools were called)
pub fn assert_tools_include(trace: &AgentTrace, expected: &[&str]);
// Tool presence — exclusion (none of these tools were called)
pub fn assert_tools_exclude(trace: &AgentTrace, excluded: &[&str]);
// Tool order — subsequence (these tools in this relative order, others allowed between)
pub fn assert_tool_order(trace: &AgentTrace, expected_order: &[&str]);
// Tool arguments — assert args for the Nth call to a named tool
pub fn assert_tool_args(trace: &AgentTrace, tool_name: &str, call_index: usize, expected: Value);
// Tool arguments — partial match (expected is a subset of actual args)
pub fn assert_tool_args_contain(trace: &AgentTrace, tool_name: &str, call_index: usize, expected: Value);
// Step count
pub fn assert_max_steps(trace: &AgentTrace, max: usize);
pub fn assert_exact_steps(trace: &AgentTrace, expected: usize);
// Output text
pub fn assert_output_contains(trace: &AgentTrace, substring: &str);
pub fn assert_output_matches(trace: &AgentTrace, pattern: &str); // regex
// Stop reason
pub fn assert_stop_reason(trace: &AgentTrace, expected: &str);
// No errors
pub fn assert_no_tool_errors(trace: &AgentTrace);
// Tool output inspection
pub fn assert_tool_output_contains(trace: &AgentTrace, tool_name: &str, call_index: usize, substring: &str);
// No tools called
pub fn assert_no_tools(trace: &AgentTrace);
// Request inspection
pub fn assert_system_prompt_contains(trace: &AgentTrace, substring: &str);
pub fn assert_tools_registered(trace: &AgentTrace, tool_names: &[&str]);
```

### Implementation Phases

#### Phase 1: MockLlmProvider (Foundation)

**Files:** `crates/mika-common/src/llm/mock.rs`, `crates/mika-common/Cargo.toml`

- Implement `MockLlmProvider` struct with sequence-based responses
- Implement `LlmProvider` trait for `MockLlmProvider`
- Add builder pattern with helper constructors
- Request capture via `Arc<Mutex<Vec<LlmRequest>>>`
- Add `test-utils` feature flag to `mika-common/Cargo.toml`
- Gate `mock` module behind `#[cfg(any(test, feature = "test-utils"))]`
- Re-export from `mika-common::llm::mock`

**Estimated effort:** Small — ~200 lines

#### Phase 2: EvalHarness + AgentTrace

**Files:** `crates/mika-agent/tests/eval/mod.rs`, `harness.rs`, `trace.rs`, `crates/mika-agent/Cargo.toml`

- Create `tests/eval/` integration test directory structure
- Implement `EvalHarness` builder with all defaulted `AgentParams` fields
- Implement `AgentTrace` struct with DB query collection
- Add `test-utils` feature dependency from `mika-agent` dev-dependencies on `mika-common`
- Wire up `tempdir` for `home_dir` management
- Create session in DB during harness construction

**Estimated effort:** Medium — ~300 lines

#### Phase 3: Assertion Helpers

**Files:** `crates/mika-agent/tests/eval/assertions.rs`

- Implement all assertion functions listed above
- Clear failure messages with actual vs expected values
- Tool call indexing (Nth call to a named tool)

**Estimated effort:** Small — ~200 lines

#### Phase 4: Integration Test Scenarios

**Files:** `crates/mika-agent/tests/eval/test_*.rs`

Initial test suite covering critical agent loop paths:

1. **Basic conversation** (`test_basic_conversation.rs`)
   - Agent responds with text (no tool calls) — 1-step EndTurn
   - Agent responds with thinking + text

2. **Tool calling** (`test_tool_calling.rs`)
   - Single tool call → tool result → text response (2 steps)
   - Multiple parallel tool calls in one response
   - Tool returns error → agent handles gracefully

3. **Multi-step chains** (`test_multi_step.rs`)
   - Tool A → result → Tool B → result → text (3 steps)
   - Assert tool order and arguments at each step

4. **Max steps + continuation** (`test_max_steps.rs`)
   - Mock 10 tool-use responses → max steps exceeded
   - Continuation turn (tools disabled) → text summary
   - Continuation turn failure → structured fallback

5. **Error handling** (`test_error_handling.rs`)
   - LLM returns `MaxTokens` stop reason → unrecoverable
   - LLM returns `ContentFilter` → unrecoverable
   - Mock returns `LlmError::HttpError` at step 3

6. **Required tools enforcement** (`test_required_tools.rs`)
   - Skill declares `required_tools`, agent calls them → pass
   - Agent responds without calling required tools → retry → calls them
   - Agent responds without calling required tools → retry → still doesn't → accepted (single retry limit)

**Estimated effort:** Medium — ~400 lines across all test files

## System-Wide Impact

### Interaction Graph

- `EvalHarness.run()` → `run_agent()` → `run_loop()` → `MockLlmProvider.send_message()` + tool execution + DB writes (`llm_calls`, `tool_calls`, `messages`)
- `AgentTrace::from_run()` → queries `llm_calls` + `tool_calls` tables + `MockLlmProvider.captured_requests()`
- No production code paths are modified — all new code is test infrastructure

### Error Propagation

- `MockLlmProvider` sequence exhaustion → panic (test failure with descriptive message)
- `run_agent` returns `Err` → propagated to test via `?` operator → test failure
- DB query failures in `AgentTrace` → `anyhow::Error` → test failure

### State Lifecycle Risks

- Each test creates its own in-memory SQLite DB — no cross-test contamination
- `TempDir` auto-cleans on drop — no filesystem leaks
- `MockLlmProvider` is per-test — no shared mutable state between tests

### API Surface Parity

- `MockLlmProvider` implements the same `LlmProvider` trait as all 11 real providers — guaranteed interface compatibility
- `EvalHarness` uses the public `run_agent()` API — tests exercise the real code path

### Integration Test Scenarios

1. Full `run_agent` with mock → EndTurn → verify `AgentOutput.text` matches mock response
2. Full `run_agent` with mock → ToolUse → verify `tool_calls` table has the execution record
3. Full `run_agent` with 10 tool-use mocks → verify continuation turn fires and `max_steps_exceeded` behavior
4. Full `run_agent` with required-tools skill → verify retry mechanism fires when tools are not called
5. Full `run_agent` with `MockResponse::Error` at step 2 → verify error propagation

## Acceptance Criteria

- [ ] `MockLlmProvider` in `crates/mika-common/src/llm/mock.rs` implementing `LlmProvider` trait
- [ ] `MockLlmProvider` builder with helper constructors (`text_response`, `tool_call_response`, etc.)
- [ ] `MockLlmProvider` captures all received requests for post-run inspection
- [ ] `MockLlmProvider` panics on sequence exhaustion with descriptive message
- [ ] `EvalHarness` builder in `crates/mika-agent/tests/eval/harness.rs` with sensible `AgentParams` defaults
- [ ] `EvalHarness` defaults to `skip_compaction: true`
- [ ] `AgentTrace` struct populates from DB queries (`llm_calls`, `tool_calls`) + mock captured requests
- [ ] Assertion helpers: `assert_tools`, `assert_tools_include`, `assert_tool_order`, `assert_tool_args`, `assert_max_steps`, `assert_output_contains`, `assert_no_tool_errors`
- [ ] At least 6 integration test files covering: basic conversation, tool calling, multi-step, max steps, error handling, required tools
- [ ] All integration tests pass with `cargo test -p mika-agent --test eval`
- [ ] `test-utils` feature flag on `mika-common` gates `MockLlmProvider` visibility
- [ ] No existing tests broken
- [ ] `cargo clippy` passes
- [ ] `cargo fmt` passes

## Dependencies & Risks

- **`run_agent` requires full `AgentParams`:** The 22-field struct has many dependencies. Risk: EvalHarness builder may need updates when new fields are added to `AgentParams`. Mitigation: builder pattern with defaults makes this a one-line addition.
- **`tool_calls`/`llm_calls` gated by Settings:** If `store_llm_calls` or `store_tool_calls` are false, traces are empty. Mitigation: EvalHarness forces both to `true`.
- **`run_loop` is private:** We use `run_agent` (public), which exercises the full path including prompt assembly, skill matching, and session management. This is a feature — higher fidelity tests.
- **Feature flag for integration tests:** `mika-common` needs `test-utils` feature in `mika-agent` dev-dependencies for `MockLlmProvider` access in `tests/eval/`.

## Sources & References

- Issue: [#329](https://github.com/senara-solutions/mika/issues/329)
- `LlmProvider` trait: `crates/mika-common/src/llm/mod.rs:74-105`
- `LlmRequest`/`LlmResponse` types: `crates/mika-common/src/llm/types.rs`
- `run_agent`: `crates/mika-agent/src/agent.rs:862`
- `AgentParams`: `crates/mika-agent/src/agent.rs:821`
- `TestHarness`: `crates/mika-agent/src/test_utils.rs:58`
- `ToolRegistry`: `crates/mika-agent/src/tools/mod.rs:433`
- `SkillRegistry::empty()`: `crates/mika-agent/src/skills/mod.rs:52`
- Existing integration test: `crates/mika-agent/tests/smoke.rs`
- Solution: multi-provider LLM trait abstraction — `docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md`
- Solution: required-tools enforcement gate — `docs/solutions/prompt-engineering/required-tools-enforcement-gate.md`
- Solution: agent max-steps fallback — `docs/solutions/runtime-errors/agent-max-steps-no-followup.md`
- Solution: runtime observability recording — `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md`
