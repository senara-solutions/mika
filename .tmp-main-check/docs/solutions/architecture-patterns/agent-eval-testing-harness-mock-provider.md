---
title: "Agent eval testing harness with MockLlmProvider"
category: architecture-patterns
date: 2026-03-30
tags: [testing, mock, llm-provider, integration-tests, eval-harness, agent-loop]
issue: "#329"
---

# Agent Eval Testing Harness with MockLlmProvider

## Problem

All ~1,770 tests in the Mika codebase were unit-level — zero integration tests exercised the full `run_agent()` path. The agent loop orchestrates prompt assembly, skill matching, tool execution, LLM calls, DB persistence (llm_calls, tool_calls tables), and conversation history management. Without integration tests, regressions in the loop's orchestration logic (required-tools enforcement, max-steps continuation, error propagation) could only be caught by manual testing with real LLM providers.

The key challenge: `run_agent()` requires a real `LlmProvider` implementation that makes API calls. No mock provider existed, and the `dummy_provider()` (an `AnthropicProvider` with no API key) couldn't return controlled responses.

## Root Cause

The `LlmProvider` trait was designed for production use only. No test-friendly implementation existed because:
1. The trait requires `Send + Sync + #[async_trait]` — non-trivial to mock
2. `AgentParams` has 22+ fields, making test setup verbose
3. Integration tests in `tests/` can't access `#[cfg(test)]` modules from the library crate
4. `LlmError` didn't derive `Clone`, making mock error sequences awkward

## Solution

### MockLlmProvider (`mika-common::llm::mock`)

Sequence-based mock implementing `LlmProvider` with three capabilities:
- **Scripted responses**: `Vec<MockResponse>` consumed by index via `AtomicUsize` cursor
- **Request capture**: `Arc<Mutex<Vec<LlmRequest>>>` stores all received requests for post-run inspection
- **Error injection**: `MockResponse::Error(LlmError)` entries simulate API failures at specific steps

Gated behind `#[cfg(any(test, feature = "test-utils"))]` so it's available to both inline unit tests and integration tests (via `mika-common = { features = ["test-utils"] }` in dev-dependencies).

Helper constructors for common patterns:
```rust
text_response("Hello!")                    // EndTurn, no tools
tool_call_response("search_memory", json!({"query": "test"}))  // ToolUse
multi_tool_response(vec![...])             // Multiple parallel tool calls
thinking_response("Answer", "Reasoning")   // Extended thinking
max_tokens_response("partial")             // MaxTokens stop
content_filter_response()                  // ContentFilter stop
```

Panics on sequence exhaustion with a descriptive message — fast failure for debugging.

### EvalHarness (`mika-agent/tests/eval/harness.rs`)

Builder wrapping `run_agent()` with sensible defaults:
- In-memory SQLite (via `Database::open_in_memory()`)
- `TempDir` with minimal agent home structure (`soul.md`, `skills/`, `data/`)
- `skip_compaction: true` (avoids extra LLM calls in mock sequences)
- `SkillRegistry::empty()` (deterministic prompts)
- `default_tools()` (full tool registry)
- Unique session ID and trace ID per test

### AgentTrace (`mika-agent/tests/eval/trace.rs`)

Post-run trace assembled from:
- `db.query_llm_calls_by_trace(trace_id)` — LLM call records from SQLite
- `db.query_tool_calls_by_trace(trace_id)` — tool call records from SQLite
- `MockLlmProvider::captured_requests()` — full request payloads

### Key Design Decision: `test-utils` Feature Flag

Integration tests in `tests/` cannot access `#[cfg(test)]` modules from the library crate. The `test-utils` feature on `mika-common` exposes `MockLlmProvider` to integration tests while keeping it compiled out of production builds. This is the same pattern used for exposing test helpers across crate boundaries in Rust.

### Key Design Decision: Clone on LlmError

Added `#[derive(Clone)]` to `LlmError` (all fields are `u16`, `String`, `bool` — trivially cloneable). This simplified mock error handling from a 14-line manual match to `Err(err.clone())` and benefits any future code needing to clone errors.

## Prevention

- **When adding new `AgentParams` fields**: Update `EvalHarness` builder defaults. The compiler will catch missing fields.
- **When adding new `LlmError` variants**: `Clone` derive handles it automatically (no manual match arm needed).
- **When modifying `run_agent()` behavior**: Add an integration test in `tests/eval/` to verify the behavior under mock conditions.
- **When adding new `Settings` fields**: Update `dummy_settings()` in `harness.rs` (duplicated from `test_utils.rs` — a known tech debt item; compiler catches missing fields).

## Related

- `crates/mika-common/src/llm/mock.rs` — MockLlmProvider implementation
- `crates/mika-agent/tests/eval/` — integration test suite (13 tests)
- `crates/mika-agent/src/test_utils.rs` — existing unit-level TestHarness
- `docs/solutions/architecture-patterns/multi-provider-llm-trait-abstraction.md` — LlmProvider trait design
- `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md` — llm_calls/tool_calls tables used for trace assertions
