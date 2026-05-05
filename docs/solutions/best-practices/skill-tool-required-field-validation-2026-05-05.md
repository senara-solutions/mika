---
title: "Skill tool required-field validation — three-layer defense against LLM-omitted fields in dispatch"
date: 2026-05-05
category: best-practices
module: crates/mika-agent/src/skills/executor.rs
problem_type: best_practice
component: executor
severity: high
applies_when:
  - Adding a new skill tool with required fields in its JSON Schema
  - Debugging a handler crash where the subprocess received malformed input
  - A long-running dispatch record is persisted without expected fields
tags: [executor, validation, dispatch, run_claude_pilot, required-fields, skill-tools, defense-in-depth]
---

# Skill tool required-field validation — three-layer defense against LLM-omitted fields in dispatch

## Context

On 2026-05-02, `run_claude_pilot` dispatch crashed during mika#928 redispatch because the LLM emitted a tool call without the `skill` field. The JSON Schema declared `skill` as required, but the Rust executor passed the input directly to the subprocess without schema validation. The handler script caught the missing field and exited with code 1, but the resulting error was an opaque crash string ("HANDLER CRASH (exit code 1)") that left the parent task `blocked` with no actionable retry signal.

Three layers failed: (1) no schema enforcement at the Rust boundary, (2) handler validation produced free-form text instead of structured errors, (3) the parent task received an unparseable crash message.

## Solution

Three orthogonal fixes at decreasing cost levels:

### Layer 1: Rust-level `validate_required_fields()` (primary)

Added at the top of `execute_skill_tool()` — runs before both the long-running and short-lived handler paths. Checks all top-level `required` fields from the tool's `input_schema` for presence and non-null values. On failure, returns `ToolOutput::error()` with structured JSON:

```json
{
  "error": "missing_required_field",
  "tool": "run_claude_pilot",
  "field": "skill",
  "valid_values": ["dev-pilot", "dev-groom"],
  "reason": "The 'skill' field is required by the tool schema but was not provided."
}
```

The LLM sees this in the same turn and can retry with the correct fields. No subprocess is spawned, no task is created, no callback is needed.

**Scope limitation:** Validates only top-level `required` fields (presence + non-null). Does NOT validate `enum` constraints, `type` assertions, or nested `properties`. This is intentional — nested validation is a handler-side concern.

**Defensive schema parsing:** If `required` is missing, non-array, or contains non-string elements, the validation degrades gracefully to an empty required list with a `warn!` event (`skill_tool_malformed_schema_skipped_validation`).

### Layer 2: Structured handler errors (defense-in-depth)

`dispatch-lib.sh` `_validate_inputs()` now emits structured JSON with a `DISPATCH_VALIDATION_ERROR:` prefix:

```
DISPATCH_VALIDATION_ERROR: {"error":"missing_required_field","field":"skill","valid_values":["dev-pilot","dev-groom"]}
```

The exit trap captures this to stderr and delivers it as the callback result. Downstream consumers can `jq` the result after the prefix to distinguish validation failures from runtime crashes.

### Layer 3: `debug_assert!` in dispatch record (development-time)

After `serde_json::to_string(&input)` in `execute_long_running`, a `debug_assert!` verifies that required fields survived serialization. Fires only in debug builds — Layer 1 is the runtime guard.

## Key Decisions

1. **Unified entry-point validation:** The validation runs for both long-running and short-lived handlers. The mika-arch first-pass review correctly identified that two different error paths for the same violation is architecturally incomplete.

2. **First missing field reported:** The validation returns on the first missing field rather than collecting all missing fields. This matches the LLM retry pattern — the model will retry with the one field, and if it misses another, the next call catches it. Avoids complex multi-error formatting.

3. **`valid_values` from enum:** When the missing field has an `enum` constraint in the schema, the error includes the valid values. This gives the LLM maximum information for a successful retry.

## References

- mika#955 (this fix)
- mika#932 (sibling: `run_claude_pilot` tool collision — different code path)
- `crates/mika-agent/src/skills/executor.rs` — `validate_required_fields()`, `execute_skill_tool()`
- `skills/bundled/_shared/dispatch-lib.sh` — `_validate_inputs()`
