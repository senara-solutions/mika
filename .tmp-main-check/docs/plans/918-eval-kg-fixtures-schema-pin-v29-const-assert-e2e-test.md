# Plan: Bump KG eval fixture pin to v29 + const-assert + E2E test (#918)

## Context

PR #915 ships secret-scrubbing for `tool_calls` (closes #908) with schema migration v28→v29. 10 `eval::kg_self_knowledge::*` tests panic because `PINNED_SCHEMA_VERSION` is still 28. Additionally, mika-qa blocked the PR because the EvalHarness integration test for R3 (live ToolOutput to LLM unscrubbed) and R5 (end-to-end DB redaction) was not delivered.

## Changes

### Change 1: Bump fixture pin to v29

**File:** `crates/mika-agent/tests/eval/kg_fixtures/mod.rs:25`

- Change `const PINNED_SCHEMA_VERSION: i32 = 28` → `const PINNED_SCHEMA_VERSION: i64 = 29`
- Change `query_scalar::<i32>` → `query_scalar::<i64>` at line 63-65 in `assert_schema_version`
- Rationale: Align type to `i64` to match `db.rs::CURRENT_SCHEMA_VERSION: i64` source of truth

### Change 2: Add compile-time co-edit guard

**File:** `crates/mika-agent/tests/eval/kg_fixtures/mod.rs` (after pin declaration)

Add a `const _: () = assert!(...)` that fails compilation if `CURRENT_SCHEMA_VERSION != PINNED_SCHEMA_VERSION`. Message must be a string literal (no `format!` in const context).

### Change 3: Add R3+R5 end-to-end test

**File:** `crates/mika-agent/tests/eval/tool_call_secret_redaction.rs` (new)

Register in `crates/mika-agent/tests/eval/eval.rs`.

Test structure:
1. EvalHarness with MockLlmProvider, stage a `read_agent_file` tool call against a fixture `.env` containing a GitHub PAT
2. Run the agent turn
3. **R3 assertion:** Check captured LLM request's `tool_result` content — the secret IS present (agent needs real values)
4. **R5 assertion:** Query `tool_calls.output` from DB — assert `<REDACTED>` present and original PAT absent

## Acceptance Criteria

- `cargo clippy --all-targets --all-features -- -D warnings` passes
- `cargo test -p mika-agent --test eval` passes — all 10 `eval::kg_self_knowledge::*` tests green
- `tests/eval/tool_call_secret_redaction.rs` exists, registered in `eval.rs`
- Load-bearing comment block at top of test file

## Out of Scope

- Other prose-shaped co-edit invariants (#917)
- Generalized co-edit framework
- Changing the runtime `assert_schema_version` helper beyond i32→i64
