---
module: skills
date: 2026-05-06
problem_type: runtime_error
component: tooling
severity: critical
tags:
  - validate-required-fields
  - bundled-skills
  - silent-failure
  - dispatch-guard
  - schema-validation
symptoms:
  - "run_claude_pilot dispatch accepted without required 'skill' field"
  - "Subprocess exit 1 after engine validation silently passed"
  - "Parent task stuck in 'blocked' — dispatch queue frozen"
  - "validate_dispatch_readiness global guard prevents all subsequent dispatches"
root_cause: missing_validation
resolution_type: code_fix
related_components:
  - development_workflow
---

# validate_required_fields silently no-ops on production schema

## Problem

`validate_required_fields()` in `executor.rs` silently returned `None` (pass) when the `required` field in a tool's `input_schema` was malformed (non-array) or when the on-disk schema was stale. This allowed dispatches missing required fields to spawn subprocesses that immediately crashed, leaving callback tasks in a stuck state that blocked the entire dispatch queue via the `validate_dispatch_readiness` global guard.

## Symptoms

- `run_claude_pilot` dispatched with `{"prompt": "mika#666", "task_id": "<uuid>"}` (missing `skill`) was accepted by the engine
- Subprocess crashed immediately (exit 1) — no claude-pilot log written
- Parent task transitioned to `blocked`, freezing milestone#13 queue
- Repeated re-dispatch attempts produced the same failure pattern
- `tool_calls.output` showed "Task submitted (long-running)" — proving `validate_required_fields` returned `None`

## What Didn't Work

- **Hypothesis H1 (stale on-disk tools.json)**: Falsified — the per-agent `~/.mika/agents/mika-dev/skills/dev-pilot/tools.json` had the correct `required: ["skill", "prompt", "task_id"]` and `MIKA_DISABLE_BUNDLED_SKILLS` was unset
- **Hypothesis F3 (bypass via alternate code path)**: Structurally invalid — `execute_long_running()` is private and only reachable through `execute_skill_tool()` which always calls `validate_required_fields` first

## Solution

Three-layer fix addressing the validator, observability, and schema drift:

### 1. Malformed schema rejection (Step 3.5)

Before this fix, when `required` existed but was not a JSON array (null, string, object), the validator silently returned `None`. Now it returns a structured `malformed_required_schema` error:

```rust
// Before: silent pass on malformed schema
if skill_tool.definition.input_schema.get("required").is_some_and(|v| !v.is_array()) {
    warn!(...);
}
return None;  // ← silent pass

// After: reject with structured error
if let Some(raw) = &required_raw && !raw.is_array() {
    warn!(...);
    return Some(ToolOutput::error(json!({
        "error": "malformed_required_schema",
        "tool": tool_name,
        "reason": "...",
    }).to_string()));
}
```

### 2. F5 diagnostic instrumentation

Added permanent tracing at two levels:
- **DEBUG** (silent in production): emits `tool_name`, `input_keys`, `required_fields` when all fields are present
- **WARN** (immediate visibility): emits full diagnostics including `required_raw` when any field is missing

This ensures the next occurrence of this class of bug is diagnosable from a single log line.

### 3. Build-time content-hash drift detection

Added a `content_hash` field to `BundledSkill` computed at build time over all file contents (sorted order, using `DefaultHasher`). When `MIKA_DISABLE_BUNDLED_SKILLS=true` prevents re-seeding, `check_bundled_skill_drift()` compares on-disk files against embedded hashes and emits ERROR logs per drifted skill.

## Why This Works

The root cause was a **silent-pass gap** in the validator. Two distinct code paths led to `return None` without logging actionable diagnostics:

1. Missing `required` key (legitimate — schemas without required fields)
2. Malformed `required` (present but wrong type — a configuration bug)

By splitting these two cases — legitimate absence stays a pass, malformed presence becomes an error — dispatches with schema corruption are now rejected synchronously before any subprocess spawns.

The drift detection addresses the operational vector: even if the validator is correct, stale on-disk schemas (from disabled bundled-skill seeding) would still bypass validation since the `required` array might be absent or outdated in the stale file.

## Prevention

1. **Cross-layer integration tests** (`tests/skills_load_path.rs`): Load the real production manifest through `seed_bundled_skills()` → disk → `scan_skills_dir()` and assert `required` contains `["skill", "prompt", "task_id"]`. This is the test PR #969 missed.

2. **Provider-format serialization test** (`mika-common/src/llm/types.rs`): Assert the `required` field survives the `ToolDefinition → LlmToolDefinition` conversion and serialization roundtrip.

3. **Drift detection on startup**: When `MIKA_DISABLE_BUNDLED_SKILLS=true`, ERROR-level logs surface any schema staleness immediately.

4. **Pattern rule**: Any validator that reads schema state should log the raw schema value at WARN level when validation fails or degrades — never silently return "pass" for an unrecognized shape. The two-level (DEBUG/WARN) pattern in this fix is reusable.

## Key Files

- `crates/mika-agent/src/skills/executor.rs` — `validate_required_fields()` (the fix site)
- `crates/mika-agent/src/bundled_skills.rs` — `check_bundled_skill_drift()` (drift detection)
- `crates/mika-agent/src/startup.rs` — `seed_bundled_skills_if_needed()` (integration point)
- `crates/mika-agent/build.rs` — content hash generation at build time
- `crates/mika-agent/tests/skills_load_path.rs` — cross-layer integration tests
- `crates/mika-common/src/llm/types.rs` — provider serialization test

## References

- Issue: [mika#984](https://github.com/senara-solutions/mika/issues/984)
- PR #969 (original validation, closed #955): introduced `validate_required_fields` but with hand-constructed test fixtures only
- Commit `06dc9e40` (2026-04-28): added `"skill"` to `required` in `tools.json`
