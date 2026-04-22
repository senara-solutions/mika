# Plan — mika#340 — Eval harness Phase 1 follow-ups

**Issue:** senara-solutions/mika#340
**Branch:** `feat/340/eval-harness-followups`
**Milestone:** Evaluation (#16)
**Status:** Groomed draft — pending Vincent review

## Context

Phase 1 of the agent eval harness shipped as PR #330 (merged 2026-03-30). The review surfaced five follow-up items, bundled into #340 because they all touch the same files (`harness.rs`, `mock.rs`, `assertions.rs`, `trace.rs`). This ticket is the root of milestone #16's dependency DAG — #338 (Phase 2 multi-provider) is blocked on the DI builder methods landing here, and #740/#741 (KG + grounding scenarios) both transit through #338.

Scope is deliberately narrow: these are Phase 1 cleanup items, not new capabilities. The design surface is small; the value comes from unblocking downstream tickets cleanly.

## Decisions

### D1 — DI builder methods for four optional dependencies

**Problem:** `EvalHarness` hardcodes four fields to `None` in the `run()` call (`harness.rs:63-70`):
- `embedding_client: None` — blocks Layer 3 hybrid search tests (needed by #740 Path C)
- `brave_api_key: None` — `web_search` builtin silently degrades
- `github_token: None` — GitHub tools silently degrade (needed by #741 scenario 1)
- `mcp_manager: None` — MCP tools unavailable

**Decision:** Add four parallel builder methods on `EvalHarnessBuilder`, each optional and storing `Option<T>`:

```rust
pub fn embedding_client(mut self, client: Arc<dyn EmbeddingClient>) -> Self { ... }
pub fn brave_api_key(mut self, key: String) -> Self { ... }
pub fn github_token(mut self, token: String) -> Self { ... }
pub fn mcp_manager(mut self, mgr: Arc<McpManager>) -> Self { ... }
```

Store on `EvalHarness` as `Option<T>` fields. `run()` threads each through to `AgentParams`. Default = `None` (no behavior change for existing tests).

**Rationale:** Four parallel opt-ins map 1:1 to the four `AgentParams` fields. Alternative — a single `dependencies()` struct — adds ceremony without covering the common case (tests that want one of four). Parallel builders match the existing builder idiom (`.tools()`, `.skills()`, `.session_id()`).

**Tradeoff:** Four near-identical methods feels repetitive. Accepted — matching existing surface beats inventing a new abstraction for four call sites.

### D2 — `Settings::test_defaults()` lives on `Settings` in `mika-common`

**Problem:** `dummy_settings()` in `harness.rs:238` explicitly sets every `Settings` field. Breaks whenever a new field is added.

**Decision:** Extract into `impl Settings { pub fn test_defaults() -> Self { ... } }` in `mika-common/src/config.rs`, gated behind `#[cfg(any(test, feature = "test-utils"))]` (matches the existing `MockLlmProvider` pattern). Delete `dummy_settings()` from `harness.rs`, replace with `Settings::test_defaults()`.

**Rationale:** The test-utils feature is already the established pattern for cross-crate test helpers in this workspace. Placing `test_defaults()` on `Settings` itself means any new field author who breaks tests sees the compile error exactly where it originates.

**Rejected alternative:** A `test_utils::dummy_settings()` helper function in mika-common. Adds a second function to maintain and doesn't get free compile-time protection against missing new fields.

### D3 — Callback turn test coverage — one scenario, two assertions

**Problem:** `.callback_turn(true)` builder method exists (`harness.rs:149`) but zero tests use it (grep confirmed). The callback flow (agent creates task → external completion → silent continuation) is a correctness-critical path for claude-pilot integration.

**Decision:** Add `test_callback_turn.rs` with one scenario:

1. Turn 1: agent receives a user message, calls `run_claude_pilot` (mocked), response ends with a callback task created
2. Turn 2 (via a second `harness.run_with_callback(...)`): callback delivers a result, agent extracts metadata + notifies user + closes out

Two assertions:
- `SilentTrigger::Callback` framing actually fires (verify via trace)
- Engine-level metadata extraction (`try_extract_callback_metadata`) captures the expected fields

**Rationale:** One scenario covers the golden path. Callback-turn-specific edge cases (retry, heartbeat interference, dispatch guards) already have integration coverage elsewhere — this test's job is to prove the harness wiring is correct, not to re-test the task engine.

**Dependency:** Requires a new builder method `EvalHarness::run_callback(...)` that accepts a `CallbackResult` payload. Lands in the same PR.

### D4 — Trace collection timing guarantees documented as doc comments on `AgentTrace::from_run`

**Problem:** It's not obvious to a test author whether `db.query_llm_calls_by_trace()` and `db.query_tool_calls_by_trace()` return complete data immediately after `run_agent()` returns.

**Decision:** Add a doc comment on `AgentTrace::from_run` in `trace.rs` stating:

> All DB writes by `run_agent()` are synchronous — LLM calls and tool calls are persisted before `run_agent()` returns. `AgentTrace::from_run` can query them immediately without waiting or polling.

Not a runtime assertion, not in CLAUDE.md — doc comment next to the code that depends on the guarantee. One sentence.

**Rejected alternative:** A test that asserts the invariant (would be trivially true and expensive to maintain).

### D5 — `MockLlmProvider::health_error()` — configurable single error

**Problem:** `check_health()` always returns `Ok(())`. No way to simulate a degraded provider scenario (needed by #741 scenario 4).

**Decision:** Add a builder method `MockLlmProvider::builder().health_error(LlmError)` that, when set, makes `check_health()` return `Err(err.clone())` on every call. No sequence semantics — one configured error, applied consistently.

**Rationale:** Test needs are simple — "is this agent resilient when health check fails?". Sequence-based mock health (first fails, then succeeds) is YAGNI for current scenarios; can be added later if a scenario demands it.

**Rejected alternative:** A `Vec<Option<LlmError>>` sequence matching the responses. Complexity premium not justified.

### D6 — One PR, no item split

**Problem:** Five items could theoretically split into three small PRs (DI builders, Settings helper, callback+health+trace). Review explicitly said "can ship in a single PR" because all changes touch the same files.

**Decision:** Keep as one PR. Implementation order within the PR:

1. D2 first — `Settings::test_defaults()` — trivial, reduces noise in subsequent diffs
2. D1 — four DI builders — the structural change
3. D3 — callback-turn test — depends on D1 being in place (won't use it directly but shares the builder pattern)
4. D5 — `health_error()` — standalone in `mock.rs`
5. D4 — doc comment on `trace.rs` — last

**Rationale:** The review's rationale (same files) still holds. A multi-PR split would paper-cut reviewers without reducing risk. CI will cover the individual tests.

## Acceptance Criteria

Maps 1:1 to the issue's five items:

- [ ] D1: four builder methods on `EvalHarnessBuilder` present, threaded into `AgentParams`, default `None`. At least one existing test uses each new builder (smoke test, can be in one scenario exercising all four set to non-None mocks).
- [ ] D2: `Settings::test_defaults()` exists in `mika-common`, gated by `test-utils` feature; `harness.rs::dummy_settings()` removed; all 13 existing eval tests still pass.
- [ ] D3: `test_callback_turn.rs` with the two assertions described. Trace validates `SilentTrigger::Callback` path.
- [ ] D4: doc comment present on `AgentTrace::from_run`.
- [ ] D5: `MockLlmProvider::builder().health_error(LlmError)` method present; returns the configured error on `check_health()`; default unchanged (returns Ok).
- [ ] `cargo test -p mika-agent --test eval` green
- [ ] `cargo clippy` clean on changed files

## Dependencies

- None (root of milestone #16 DAG)

## Downstream (unblocked by this ticket)

- #338 — uses D1's DI builders to inject real providers
- #740 — uses D1's `embedding_client` for Path C
- #741 — uses D1's `github_token` (scenario 1) and D5's `health_error` (scenario 4)

## Cross-cutting notes

- Plan follows KG-milestone Socratic pattern: D-numbered decisions with rationale and rejected alternatives so downstream plans can cite upstream decisions by number.
- Branch name `feat/340/eval-harness-followups` is the canonical dispatch branch. Issue body carries a `> **Branch:** \`feat/340/eval-harness-followups\`` callout so `/mika` handlers resolve to this branch on dispatch.

## Open questions (for Vincent before dispatch)

Flag anything you want amended before I dispatch to mika-dev. Specifically:

1. D1 — should the four DI builders also be exposed via a single aggregate method (e.g., `.dependencies(deps)`) for tests that want all four at once? Or is four parallel methods final?
2. D3 — one callback scenario is minimal. Should there be a second for the error-path case (callback delivers failure)?
3. D5 — should `health_error()` also support "fail N times, then succeed" for retry-loop tests?
