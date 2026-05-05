# Plan: fix(self-dev) run_claude_pilot dispatch record missing skill argument crashes handler

**Ticket:** mika issue#955
**Type:** fix
**Branch:** `fix/955/self-dev-run-claude-pilot-dispatch`

## Problem Summary

`run_claude_pilot` dispatch crashed at 2026-05-02 because the LLM emitted a tool call without the `skill` field. The tool's JSON schema declares `skill` as required, but the Rust executor does not validate tool inputs against the schema before spawning the subprocess. The handler script (`_shared/dispatch-lib.sh`) catches the missing field and exits with code 1, which triggers the exit trap and delivers a `"HANDLER CRASH (exit code 1)"` callback — but this generic crash string leaves the parent task in `blocked` status with no actionable error for mika-dev.

## Root Cause Analysis

Three layers failed:

1. **No schema enforcement at the Rust boundary.** `execute_long_running()` in `crates/mika-agent/src/skills/executor.rs` takes `input: serde_json::Value` and passes it directly to the subprocess. The `tools.json` schema is used only for LLM tool-array construction (telling the model what parameters exist), not for runtime validation of the model's actual output.

2. **Handler validation exits before structured callback delivery.** The `_validate_inputs()` function in `dispatch-lib.sh` (line 113) writes to stderr and calls `exit 1`. The exit trap captures the stderr and delivers it as a callback. However, the callback result string is free-form text (`"HANDLER CRASH (exit code 1). Script failed before building result.\n\nStderr (last 10KB):\nError: missing required argument 'skill'..."`), not structured JSON that the verdict handler or task-health system can parse into an actionable next-step.

3. **Parent task receives opaque crash.** mika-dev's callback turn sees a crash string but has no structured signal to distinguish "LLM forgot a required field" (retry-safe) from "handler has a real bug" (escalate). The result: task stuck in `blocked`.

## Fix Strategy

Three orthogonal fixes matching the ticket's three investigation surfaces:

### Fix 1: Rust-level required-field validation before spawn (primary fix)

**File:** `crates/mika-agent/src/skills/executor.rs`

Add a `validate_required_fields()` check at the top of `execute_long_running()`, before task creation. Parse `skill_tool.definition.input_schema` for the `required` array, verify each required field is present and non-null in the input JSON. On failure, return `ToolOutput::error()` with a structured JSON error (same pattern as `task_not_dispatchable`):

```json
{
  "error": "missing_required_field",
  "tool": "run_claude_pilot",
  "field": "skill",
  "valid_values": ["dev-pilot", "dev-groom"],
  "reason": "The 'skill' field is required by the tool schema but was not provided in the tool call."
}
```

This prevents the subprocess from ever spawning with invalid input, catches the bug at the cheapest possible point (no task creation, no subprocess, no callback needed), and gives the LLM a clear retry signal in the same turn.

**Why `execute_long_running` specifically:** Only long-running exec handlers have the cascade problem (task creation → subprocess spawn → crash → opaque callback). Short-lived exec handlers fail in the same turn and the error is visible immediately. The cost of schema validation is negligible relative to subprocess spawn + callback lifecycle.

**Scope limitation:** Validate only `required` fields (presence + non-null). Do NOT validate `enum` constraints or `type` assertions at this layer — that would duplicate logic better left to the handler. The goal is to prevent the "field entirely missing" class of failures that produce opaque crashes.

### Fix 2: Structured error in handler validation path (defense-in-depth)

**File:** `skills/bundled/_shared/dispatch-lib.sh`

When `_validate_inputs()` detects a missing required field, format the exit message as a structured JSON object before exiting. The exit trap already captures stderr — make the stderr parseable:

```bash
if [ -z "$SKILL" ]; then
    printf '{"error":"missing_required_field","field":"skill","valid_values":["dev-pilot","dev-groom"]}\n' >&2
    exit 1
fi
```

The exit trap delivers this as the callback result. Downstream consumers (mika-dev's callback turn) can then `jq` the result and decide to retry vs escalate.

**Additionally:** Add a prefix marker to the callback result when it originates from input validation failure (not a runtime crash). Modify the exit trap to detect the structured-error pattern and tag the result:

```
DISPATCH_VALIDATION_ERROR: {"error":"missing_required_field","field":"skill",...}
```

This lets the `ci_failure_handler` and verdict handler distinguish validation failures from runtime crashes without parsing free-form text.

### Fix 3: Fail-loud at dispatch-record insert time (belt-and-suspenders)

**File:** `crates/mika-agent/src/skills/executor.rs`

After Fix 1, the input_context stored in the callback task row will always contain required fields. But as belt-and-suspenders for any future code path that writes `input_context` directly:

Add a post-condition assertion after `serde_json::to_string(&input)` on line 845: if the tool's schema declares required fields, assert they exist in the serialized input. This is a `debug_assert!` (development-only) rather than a runtime check — Fix 1 is the runtime guard.

## Implementation Order

1. Fix 1 (Rust validation) — the primary fix. Prevents the bug class entirely.
2. Fix 2 (structured handler errors) — defense-in-depth. Makes any future bypass of Fix 1 produce actionable errors.
3. Fix 3 (debug assertion) — development-time safety net.

## Files to Modify

| File | Change |
|------|--------|
| `crates/mika-agent/src/skills/executor.rs` | Add `validate_required_fields()` before `execute_long_running` body; add `debug_assert!` after input serialization |
| `skills/bundled/_shared/dispatch-lib.sh` | Structured JSON stderr in `_validate_inputs()`; exit-trap detection of validation-origin errors |

## Testing

1. **Unit test:** Add test in `executor.rs` tests module — call `execute_long_running` (or `execute_skill_tool`) with input missing `skill`, verify `ToolOutput::error` contains `missing_required_field`.
2. **Integration test:** Add eval scenario — mock LLM emits `run_claude_pilot` without `skill` field, verify agent receives structured error and can retry.
3. **Manual verification:** Confirm the dispatch-readiness guard still rejects appropriately when the validation passes but other guards fail.

## Risk Assessment

- **Low risk.** Fix 1 adds a pre-check that returns early with an error. No existing working code paths are affected (they always provide `skill`). The validation only fires when the LLM omits a required field — which is already a broken state.
- **No breaking changes.** The handler script's structured error is backward-compatible — the exit trap still captures it and delivers as a callback string. Old consumers see a slightly different crash message format; new consumers can parse the JSON prefix.

## Out of Scope

- Generalizing schema validation to all tool types (future work, separate ticket).
- Retry logic in mika-dev for validation errors (separate concern — mika-dev already retries on tool errors).
- claude-pilot changes (this bug occurs before claude-pilot is invoked).
