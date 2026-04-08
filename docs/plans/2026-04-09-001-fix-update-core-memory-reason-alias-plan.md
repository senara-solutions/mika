---
title: "fix: update_core_memory accept 'reason' as alias for 'reasoning'"
type: fix
status: active
date: 2026-04-09
issue: 488
---

# fix: update_core_memory accept 'reason' as alias for 'reasoning'

The `update_core_memory` tool requires a `reasoning` field. `minimax/minimax-m2.7` consistently truncates it to `reason` in tool input JSON, producing:

```
Missing required parameter(s): reasoning. You must provide: 'section' (...), 'action' (...), 'reasoning' (...), and 'content' (...).
```

Observed across multiple turns on 2026-04-08 (traces `091d4ec0a6f34e15bab4f507d9ee556c`, `5d613822-3348-11f1-81f8-c22d175085f1`). The pattern is reliable enough to treat as a persistent model-specific tokenization quirk — prompt-level instructions do not prevent it. Per the `feedback_prompt_enforcement_fragile` memory, prompt discipline is not a reliable fix for tokenization artifacts; an engine-level structural accommodation is the right solution.

## Fix

Accept `reason` as an alias for `reasoning` inside the `update_core_memory` tool handler only. The alias is narrow, risk-free (canonical `reasoning` always wins when both are present), and does not appear in the tool JSON schema — we do not want to advertise the misspelling.

**File:** `crates/mika-agent/src/tools/update_core_memory.rs`

Current code (line 70):

```rust
let reasoning = input["reasoning"].as_str().unwrap_or("");
```

Proposed change:

```rust
// Accept `reason` as an alias for `reasoning` to accommodate tokenization
// quirks in some LLMs (e.g., minimax/minimax-m2.7 consistently truncates
// the key to `reason`). Canonical `reasoning` wins when both are present.
// See issue #488.
let reasoning = input["reasoning"]
    .as_str()
    .or_else(|| input["reason"].as_str())
    .unwrap_or("");

if reasoning.is_empty() == false
    && input.get("reasoning").and_then(|v| v.as_str()).is_none()
    && input.get("reason").is_some()
{
    tracing::debug!(
        target: "mika::tools",
        model = ?ctx.model_name,
        provider = ?ctx.provider_name,
        "update_core_memory: accepted 'reason' as alias for 'reasoning'"
    );
}
```

The `required` array in the JSON schema (line 60) stays `["section", "action", "reasoning"]` — the schema is the canonical contract; the alias is an engine-layer compatibility shim.

## Acceptance Criteria

- [x] `update_core_memory` accepts `reason` as an alias for `reasoning` (tool succeeds when only `reason` is provided, all other required fields present).
- [x] When both `reasoning` and `reason` are present, `reasoning` wins (canonical field takes precedence).
- [x] On alias hit, a DEBUG log line is emitted at target `mika::tools` with `model` and `provider` fields for telemetry.
- [x] Tool JSON schema (`parameters.required`) is unchanged — still declares `reasoning`.
- [x] Unit test: input with only `reason` (no `reasoning`) succeeds and writes the expected core memory update.
- [x] Unit test: input with both `reason` and `reasoning` uses the `reasoning` value (not `reason`).
- [x] Existing tests continue to pass (including `test_missing_section_and_reasoning_lists_both`, `test_missing_only_section_specific_error`, `test_missing_content_for_non_reset_action`, `test_all_fields_missing_lists_all`).
- [x] `cargo fmt`, `cargo clippy`, `cargo test -p mika-agent` all pass.

## Context

- **Target file:** `crates/mika-agent/src/tools/update_core_memory.rs` — lines 60 (schema `required`), 66–87 (`execute` signature and field extraction / missing-field validation), existing test module at lines ~617+.
- **ToolContext fields used:** `ctx.model_name`, `ctx.provider_name` (both `Option<String>`, threaded through `ToolContext` per `CLAUDE.md` Tools section).
- **Tracing target convention:** existing tool debug logs use `target: "mika::tools"` pattern.
- **Test pattern:** follow existing `#[tokio::test]` harness in the same file (`test_missing_*` tests at lines 617+ show the setup for invoking `execute` with a `Value` input).

## Out of Scope

- Generalizing alias handling to other tools (`update_work_item_status`, `check_work_item`, etc.) — file follow-ups only if observed.
- Adding a fuzzy-match parser for all tool field names — overkill and risky.
- Fixing minimax tokenization at the LLM-response-parser layer — larger, riskier change.
- Documenting the alias in the tool schema or prompt — intentionally hidden to avoid encouraging misspelling.

## Sources

- **Issue:** [#488](https://github.com/senara-solutions/mika/issues/488)
- **Target file:** `crates/mika-agent/src/tools/update_core_memory.rs:60-87`
- **Traces:** `091d4ec0a6f34e15bab4f507d9ee556c`, `5d613822-3348-11f1-81f8-c22d175085f1` (mika-dev SQLite)
- **Memory:** `feedback_prompt_enforcement_fragile` — "Don't use prompt-level budgets/limits; LLMs rationalize crossing them. Use structural constraints."
- **Related umbrella:** `senara-solutions/mika-platform#17` (model calibration follow-up)
