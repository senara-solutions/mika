---
title: "feat: Validate UUID-typed arguments at tool boundary"
type: feat
status: active
date: 2026-04-13
issue: 531
---

# feat: Validate UUID-typed arguments at tool boundary

## Overview

Add UUID format validation at the tool boundary for all tools that accept UUID-typed arguments. Currently, tools only check for empty strings and length > 36, allowing malformed UUIDs (e.g., fabricated suffixes, truncated strings) to pass through to DB lookups. This wastes a round-trip and returns vague "not found" errors that don't teach the LLM what went wrong structurally.

## Problem Frame

On 2026-04-11, mika-dev (qwen3-coder) called `run_claude_pilot` with a fabricated task_id — it pattern-matched the UUID shape but fabricated the suffix `a123456789ab`. The tool accepted it, hit the DB, and returned a soft "Work item not found" error. The LLM retried with the correct ID, but the failure wasted a tool step and DB query.

This is a structural enforcement gap: prompt-level instructions ("use the exact task_id") are unreliable for this failure class (per `feedback_prompt_enforcement_fragile.md`). The fix must be at the tool boundary where the LLM cannot rationalize past it.

## Requirements Trace

- R1. Add a shared `validate_uuid()` helper to `tools/mod.rs` returning structured JSON errors
- R2. All tools accepting UUID-typed arguments call `validate_uuid()` before any side effect
- R3. Invalid UUIDs produce a structured error with field name, received value, and reason
- R4. Valid UUID inputs continue to pass through unchanged
- R5. Unit tests verify invalid UUIDs are rejected before DB/subprocess/network
- R6. Complements #525 (dispatch-readiness guard catches valid-but-stale UUIDs)

## Scope Boundaries

- `session_id` and `trace_id` are NOT pure UUIDs (session_id uses prefixes like `delegate-{uuid}`, trace_id is 32-char hex) — excluded from UUID validation
- `cancel_reminder` delegates to `CancelTaskTool.execute()` — transitively covered, no separate change needed
- `get_team_status` `run_id` is optional and a harmless filter — out of scope
- HTTP server endpoints (`/tasks/{id}/complete`, `/tasks/{id}/cancel`) accept UUID path params without validation — same rationale applies but different code area

### Deferred to Separate Tasks

- HTTP server endpoint UUID validation (`server/handlers.rs`): separate PR to keep this focused on tool boundary
- `get_team_status` `run_id` validation: low impact, track separately if desired

## Context & Research

### Relevant Code and Patterns

- `tools/mod.rs` — existing `validate_work_item()` helper (empty check + DB lookup), `ToolOutput::error()` pattern
- `tools/get_task.rs`, `cancel_task.rs`, `complete_task.rs`, `update_work_item_status.rs` — all share the same `empty + len > 36` pattern
- `tools/check_work_item.rs` — uses `MAX_INPUT_LEN` (10,000) instead of 36 for `id` — inconsistent
- `tools/create_work_item.rs` — `parent_task_id` is optional with no format validation
- `tools/delegate_task.rs` — uses `validate_work_item()` which skips format validation
- `uuid` crate already in workspace deps with `v4` and `serde` features; `Uuid::parse_str()` used in CLI for `--run-id` validation

### Institutional Learnings

- **Team workspace hardening** (`docs/solutions/security-issues/team-workspace-ref-dir-validation-hardening.md`): Precedent for `Uuid::parse_str()` at entry boundaries with helpful error messages pointing to where valid IDs come from
- **Dispatch-readiness guard** (`docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`): Structured JSON errors with specific error codes give the LLM programmatic feedback; soft "not found" errors don't deter fabrication
- **Fabricated action-claim guard** (`docs/solutions/architecture-patterns/fabricated-action-claim-guard.md`): Project philosophy — structural guards over prompt instructions for hallucination defense
- **Tool field aliases** (`docs/solutions/prompt-engineering/2026-04-09-tool-field-alias-for-llm-tokenization-quirks.md`): "Don't use prompt-level budgets/limits; use structural constraints"

## Key Technical Decisions

- **Structured JSON error format**: Use `{"error": "invalid_uuid", "field": "<name>", "received": "<truncated>", "reason": "..."}` matching the dispatch-readiness guard pattern. Other validation errors remain plain strings — this is the pattern for future structured errors, not a retrofit
- **Truncate `received` to 50 chars**: Prevents long garbage inputs from consuming LLM context in error messages
- **Remove redundant `len > 36` checks**: `Uuid::parse_str()` handles all format validation including length. The old length check is dead code once proper UUID parsing is in place
- **Helper returns `Result<Uuid, ToolOutput>`**: Callers get the parsed `Uuid` on success (useful if they need it) or a ready-to-return `ToolOutput` error. This matches the issue's specification
- **Add UUID validation inside `validate_work_item()`**: Since `delegate_task` routes through this shared helper, adding UUID validation there catches malformed IDs before the DB lookup. The standalone `validate_uuid()` helper is called from within `validate_work_item()`

## Open Questions

### Resolved During Planning

- **Should HTTP endpoints get validation too?** Deferred to separate PR — different code area (`server/handlers.rs`), keeps this PR focused on the tool boundary as specified in the issue
- **Should `cancel_reminder` be modified?** No — it delegates to `CancelTaskTool.execute()` which will have the validation
- **Should existing plain-string errors be converted to JSON?** No — introduce structured JSON for UUID errors only, document as the forward pattern

### Deferred to Implementation

- Exact test helper setup needed for tools that currently pass non-UUID strings in tests (e.g., `"nonexistent-id"`, `"some-id"`, `"abc"`) — these tests will need updated assertions

## Implementation Units

- [x] **Unit 1: Add `validate_uuid()` helper to `tools/mod.rs`**

  **Goal:** Create a shared UUID validation function that all tools can call

  **Requirements:** R1, R3

  **Dependencies:** None

  **Files:**
  - Modify: `crates/mika-agent/src/tools/mod.rs`
  - Test: `crates/mika-agent/src/tools/mod.rs` (inline `#[cfg(test)] mod tests`)

  **Approach:**
  - Add `pub fn validate_uuid(field_name: &str, value: &str) -> Result<Uuid, ToolOutput>` to `tools/mod.rs`
  - On success, return `Ok(uuid)` with the parsed UUID
  - On failure, return `Err(ToolOutput::error(json))` with structured JSON: `{"error": "invalid_uuid", "field": "<name>", "received": "<truncated to 50 chars>", "reason": "string is not a well-formed UUID (expected 8-4-4-4-12 hex segments)"}`
  - Use `uuid::Uuid::parse_str()` for validation
  - Truncate `received` value to 50 characters with `...` suffix if longer

  **Patterns to follow:**
  - `validate_work_item()` in same file — shared tool helper pattern
  - Dispatch-readiness guard structured JSON error pattern from `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`

  **Test scenarios:**
  - Happy path: valid hyphenated UUID (`"a1b2c3d4-e5f6-7890-abcd-ef1234567890"`) returns `Ok(Uuid)`
  - Happy path: valid non-hyphenated UUID (`"a1b2c3d4e5f67890abcdef1234567890"`) returns `Ok(Uuid)`
  - Error path: empty string returns `Err` with `invalid_uuid` error JSON containing field name
  - Error path: too-short string (`"abc"`) returns `Err` with structured JSON
  - Error path: almost-valid UUID with wrong segment (`"eda3190e-764c-4b0f-a123456789ab"`) returns `Err`
  - Error path: non-hex characters (`"xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"`) returns `Err`
  - Edge case: very long input (1000 chars) — `received` field is truncated to 50 chars
  - Edge case: field name is correctly propagated into the error JSON

  **Verification:**
  - `cargo test -p mika-agent` passes with new tests
  - Helper is `pub` and accessible from all tool modules

- [x] **Unit 2: Apply `validate_uuid()` to task tools (`get_task`, `cancel_task`, `complete_task`)**

  **Goal:** Replace the `empty + len > 36` pattern with proper UUID validation in the three task-by-id tools

  **Requirements:** R2, R4, R5

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/tools/get_task.rs`
  - Modify: `crates/mika-agent/src/tools/cancel_task.rs`
  - Modify: `crates/mika-agent/src/tools/complete_task.rs`
  - Test: `crates/mika-agent/src/tools/get_task.rs` (inline tests)
  - Test: `crates/mika-agent/src/tools/cancel_task.rs` (inline tests)
  - Test: `crates/mika-agent/src/tools/complete_task.rs` (inline tests)

  **Approach:**
  - In each tool's `execute()`, replace the existing `empty + len > 36` block with:
    1. Extract `id` string as before (empty check stays or moves into `validate_uuid`)
    2. Call `super::validate_uuid("id", id)?` — the `?` on `Result<Uuid, ToolOutput>` needs an early return pattern since `execute()` returns `Result<ToolOutput>`. Use a match or map_err: if `Err(tool_output)`, return `Ok(tool_output)`
  - Remove the now-redundant `len > 36` check
  - The parsed UUID's string form (`uuid.to_string()` or just use the original `id` string) is passed to DB calls as before

  **Patterns to follow:**
  - Existing empty-check + early-return pattern in each tool
  - `validate_work_item()` call pattern in `delegate_task.rs`

  **Test scenarios:**
  - Happy path: valid UUID passes through to DB lookup (existing tests should still pass)
  - Error path: `get_task` with `"not-a-uuid"` returns `invalid_uuid` error before any DB call
  - Error path: `cancel_task` with `"abc"` returns `invalid_uuid` error
  - Error path: `complete_task` with `"eda3190e-764c-4b0f-a123456789ab"` (the actual fabricated UUID from the incident) returns `invalid_uuid` error
  - Edge case: empty `id` still returns the existing required-field error

  **Verification:**
  - `cargo test -p mika-agent` passes
  - No test uses non-UUID strings that now need updating (audit and fix if found)

- [x] **Unit 3: Apply `validate_uuid()` to work item tools (`update_work_item_status`, `check_work_item`, `create_work_item`)**

  **Goal:** Add UUID validation to the three work-item tools

  **Requirements:** R2, R4, R5

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/tools/update_work_item_status.rs`
  - Modify: `crates/mika-agent/src/tools/check_work_item.rs`
  - Modify: `crates/mika-agent/src/tools/create_work_item.rs`
  - Test: `crates/mika-agent/src/tools/update_work_item_status.rs` (inline tests)
  - Test: `crates/mika-agent/src/tools/check_work_item.rs` (inline tests)
  - Test: `crates/mika-agent/src/tools/create_work_item.rs` (inline tests)

  **Approach:**
  - `update_work_item_status`: replace `empty + len > 36` on `task_id` with `validate_uuid("task_id", task_id)`
  - `check_work_item`: replace `MAX_INPUT_LEN` check on `id` with `validate_uuid("id", id)` — fixes the inconsistency where this tool allowed 10,000-char IDs
  - `create_work_item`: add `validate_uuid("parent_task_id", value)` only when `parent_task_id` is `Some` — validation fires after the `filter(|s| !s.is_empty())` step
  - Update existing tests that pass non-UUID strings (e.g., `"some-id"`, `"abc"`, `"nonexistent-id"`) to either use valid UUIDs (for "not found" tests) or assert the new `invalid_uuid` error

  **Patterns to follow:**
  - Same pattern as Unit 2
  - Optional field validation pattern: only validate when present

  **Test scenarios:**
  - Happy path: `update_work_item_status` with valid UUID passes through
  - Error path: `update_work_item_status` with `"some-id"` returns `invalid_uuid` error (update existing test)
  - Error path: `check_work_item` with `"not-a-uuid"` returns `invalid_uuid` error
  - Error path: `create_work_item` with invalid `parent_task_id` returns `invalid_uuid` error
  - Happy path: `create_work_item` with no `parent_task_id` skips validation entirely
  - Happy path: `create_work_item` with valid UUID `parent_task_id` passes through
  - Edge case: `check_work_item` with very long string now rejected by UUID validation instead of `MAX_INPUT_LEN`

  **Verification:**
  - `cargo test -p mika-agent` passes
  - All existing tests updated for new error format where they used non-UUID strings

- [x] **Unit 4: Add UUID validation to `validate_work_item()` for `delegate_task`**

  **Goal:** Add UUID format validation inside the shared `validate_work_item()` helper so `delegate_task` (and any future callers) get format checking before DB lookup

  **Requirements:** R2, R5

  **Dependencies:** Unit 1

  **Files:**
  - Modify: `crates/mika-agent/src/tools/mod.rs` (`validate_work_item()`)
  - Modify: `crates/mika-agent/src/tools/delegate_task.rs` (update tests)
  - Test: `crates/mika-agent/src/tools/delegate_task.rs` (inline tests)

  **Approach:**
  - In `validate_work_item()`, after the empty check and before the DB lookup, call `validate_uuid("work_item_id", id)`. If it returns `Err(tool_output)`, extract the error content and return it as `Some(error_string)` to match `validate_work_item()`'s existing `Option<String>` return type
  - Update `delegate_task` tests that pass non-UUID strings like `"nonexistent-id"` — they should now assert the `invalid_uuid` error instead of "not found"

  **Patterns to follow:**
  - Existing `validate_work_item()` early-return pattern
  - `validate_uuid()` integration from Units 2-3

  **Test scenarios:**
  - Happy path: `delegate_task` with valid UUID that doesn't exist returns "not found" from DB (existing behavior after UUID passes)
  - Error path: `delegate_task` with `"nonexistent-id"` returns `invalid_uuid` error before DB lookup
  - Error path: `delegate_task` with truncated UUID returns `invalid_uuid` error
  - Integration: `validate_work_item()` with invalid UUID short-circuits before DB call

  **Verification:**
  - `cargo test -p mika-agent` passes
  - `delegate_task` test at line ~419 updated

## System-Wide Impact

- **Interaction graph:** Only tool execute() entry points are affected. No callbacks, middleware, or observers touched. `cancel_reminder` is transitively covered via delegation to `CancelTaskTool`
- **Error propagation:** Tool errors return as `ToolOutput::error()` (is_error=true) in the tool result message. The LLM sees these and can self-correct. No change to error propagation paths
- **State lifecycle risks:** None — validation happens before any state mutation
- **API surface parity:** HTTP server endpoints (`/tasks/{id}/complete`, `/tasks/{id}/cancel`) have the same gap but are deferred to a separate task
- **Integration coverage:** The eval harness uses `MockLlmProvider` with pre-scripted tool calls containing valid UUIDs — these tests are unaffected. Only unit tests with non-UUID test strings need updating
- **Unchanged invariants:** Tool definitions (names, parameter schemas), DB schema, ToolOutput structure, tool dispatch in `agent.rs` — all unchanged

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Existing tests use non-UUID strings and will break | Audit all affected test files; update to use valid UUIDs for "not found" tests or assert new error for validation tests |
| Mixed error format (JSON for UUID, plain string for others) | Document as intentional forward pattern; existing plain-string errors are not retrofitted |
| `validate_work_item()` return type mismatch with `validate_uuid()` | Extract error content string from `ToolOutput` to fit `Option<String>` return — straightforward adaptation |

## Sources & References

- **Issue:** [#531](https://github.com/senara-solutions/mika/issues/531) — tools: validate UUID-typed arguments at tool boundary
- **Sibling issue:** [#525](https://github.com/senara-solutions/mika/issues/525) — work-item-status guard on dispatch (valid-but-stale UUIDs)
- Related learnings: `docs/solutions/architecture-patterns/dispatch-readiness-guard-long-running-status-validation.md`
- Related learnings: `docs/solutions/security-issues/team-workspace-ref-dir-validation-hardening.md`
- Related learnings: `docs/solutions/architecture-patterns/fabricated-action-claim-guard.md`
