---
title: "Record resolved prompt variant on llm_calls for audit observability"
category: architecture-patterns
date: 2026-04-09
tags: [observability, skills, prompt-variants, llm-calls, schema-migration, dashboard]
issue: "#481"
modules: [mika-agent/skills/index, mika-agent/agent, mika-agent/db, dashboard]
---

# Prompt Variant Observability on LLM Calls

## Problem

`SkillEntry::resolve_prompt()` silently returns the winning prompt from a 4-step fallback chain (hand-authored model variant -> generated model variant -> generated canonical variant -> base `system_prompt.md`). No metadata about which step won was recorded — not in logs, not in the database. This made it impossible to verify that per-model prompt variants were actually being used in production turns, blocking the calibration workflow.

## Root Cause

The `resolve_prompt()` method returned a bare `&str` with no metadata. The caller (`inject_skills_and_resolve_tools()`) consumed the prompt text and discarded any notion of which fallback step produced it. The `llm_calls` observability table had no column to store this information.

## Solution

### 1. New types in `skills/index.rs`

Added `PromptVariantSource` enum (4 variants: `HandAuthoredModel`, `GeneratedModel`, `GeneratedCanonical`, `Base`) and `ResolvedPrompt<'a>` struct bundling the prompt text with source metadata and lookup key:

```rust
pub struct ResolvedPrompt<'a> {
    pub text: &'a str,
    pub source: PromptVariantSource,
    pub key: Option<String>,  // None for Base
}
```

`variant_descriptor()` method produces compact storage format: `"base"` or `"source:key"` (e.g., `"generated_model:anthropic/claude-sonnet-4-6"`).

### 2. Variant map in `inject_skills_and_resolve_tools()`

Changed return type from `Vec<ToolDefinition>` to `(Vec<ToolDefinition>, Option<String>)`. The second element is a JSON-serialized `HashMap<skill_name, variant_descriptor>` — only skills with non-empty resolved prompts are included. `None` when no skills contributed prompts.

### 3. Threading through `run_loop()` to `save_llm_call()`

Added `prompt_variant: Option<&str>` parameter to `run_loop()`. All three call paths (conversation mode, silent mode, team mode) thread the variant data. Both success and error `save_llm_call()` paths receive it.

### 4. Schema migration v20->v21

`ALTER TABLE llm_calls ADD COLUMN prompt_variant TEXT` — nullable, no default. Idempotent via `column_exists()` guard. No VIEW recreation needed (unified_timeline doesn't reference the column).

### 5. Dashboard rendering

`LlmCallDetail.tsx` parses the JSON and renders a "Skill Variants" card with `MetadataRow` per skill. Defensive try/catch falls back to raw text for malformed data.

## Key Design Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Return type | `ResolvedPrompt<'a>` struct | Callers that only need text use `.text`; variant metadata is opt-in |
| DB format | Nullable JSON TEXT | Preserves skill-to-variant mapping; NULL when no skills active |
| Per-call vs per-turn | Same value repeated per LLM call | Simpler; each row is self-contained for querying |
| Debug logging | Single `debug!` at call site | Avoids 4 duplicated log blocks inside `resolve_prompt()` |

## Prevention / Best Practices

- When adding a new fallback step to `resolve_prompt()`, add a corresponding `PromptVariantSource` variant — the enum ensures exhaustive tracking.
- Observability writes use `if let Err(e) = ... { warn!() }` — never propagate errors from instrumentation.
- New nullable columns on `llm_calls` can use `ALTER TABLE ADD COLUMN` (instant in SQLite, no table rebuild) — follow the `column_exists()` idempotency pattern.

## Related

- `docs/solutions/architecture-patterns/per-provider-skill-variant-directories.md` — the variant resolution system this observes
- `docs/solutions/architecture-patterns/runtime-observability-llm-tool-call-recording.md` — the `llm_calls` table this extends
- `docs/solutions/database-issues/trace-id-as-observability-join-key.md` — precedent for `ALTER TABLE llm_calls ADD COLUMN` (v15->v16 added `step`)
