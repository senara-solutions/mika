---
title: "feat(skills): built-in skill-review skill to generate model-tuned prompt variants"
type: feat
status: completed
date: 2026-04-06
issue: "#243"
---

# feat(skills): built-in skill-review skill to generate model-tuned prompt variants

## Overview

Add a built-in `skill-review` skill with a `review_skill` builtin handler that generates model-tuned `system_prompt.md` variants for any installed skill. The builtin handler performs structural work (locating skills, reading prompts, resolving provider/model, computing paths), and the agent's own LLM performs the creative prompt adaptation. Variants are written via `write_agent_file` to `{skill_dir}/{provider}/{model}/system_prompt.md`, following the existing two-level variant resolution system.

## Problem Statement / Motivation

Mika supports 11 LLM providers with different instruction-following styles, tool call formats, and prompt engineering quirks. The per-provider/model variant system (#241, #245, #246) enables model-specific prompts, but creating variants is currently a manual process. A built-in skill that automates this adaptation reduces the friction of multi-model support and enables Mika Dev to batch-generate variants when switching providers.

## Proposed Solution

### Handler Type Decision: Builtin Handler

The issue proposes two options. **Builtin handler** (Option 1) is chosen because:

1. **Config access**: The handler needs the agent's current `llm_provider` and `llm_model` — not available to prompt-only skills or exec handlers (MIKA_* env scrubbing)
2. **Aggregator detection**: OpenRouter canonical tuple extraction requires `ProviderKind::model_names_contain_slash()` — Rust-level API
3. **Path correctness**: Variant directory paths must match `sanitize_model_dir_name()` exactly
4. **Skill scanning**: The handler needs to iterate the skills directory and check for existing variants
5. **Linked skill safety**: Must detect symlinked skill directories to enforce the executor READ-ONLY invariant

The handler does NOT make LLM calls. It returns structured data; the agent loop does the creative adaptation using its own LLM, then writes via `write_agent_file`.

### Aggregator Provider Resolution

Per the issue spec, aggregator providers (currently only OpenRouter) extract the **canonical (provider, model) tuple** from the model name for the write path:

- OpenRouter `anthropic/claude-sonnet-4` -> writes to `anthropic/claude-sonnet-4/system_prompt.md`
- OpenRouter `openai/gpt-4o` -> writes to `openai/gpt-4o/system_prompt.md`
- Non-aggregator `claude-sonnet-4-6` -> writes to `anthropic/claude-sonnet-4-6/system_prompt.md`

**Runtime resolution is unchanged** — `resolve_prompt()` uses the configured provider/model verbatim. This means a variant generated via OpenRouter is used when the agent later runs on the native provider directly. When on OpenRouter, the variant won't resolve (by design — the aggregator is transport, not identity).

For model names with unrecognized provider prefixes (e.g., `meta-llama/llama-3.3-70b-instruct`), the extracted provider `meta-llama` is used as-is for the directory name. It won't match a `ProviderKind` but is valid as a directory name.

### Linked Skill Safety

The executor READ-ONLY invariant prohibits runtime writes to skill directories. For `--link` installed skills, `skill_dir` is a symlink to the author's source directory. The `review_skill` handler **refuses to generate variants for linked skills** and returns an error suggesting the user unlink first or copy the skill.

### Batch Mode and Step Limits

The agent loop has a 20-step tool limit. Each skill in batch mode requires ~2 steps (review_skill returns data, agent reasons, write_agent_file writes). With `skill_name = "*"`:

1. The handler returns a **summary list** of all eligible skills (name, has existing variant, is linked) in one tool call
2. The system prompt instructs the agent to process skills one at a time, prioritizing those without existing variants
3. With 20 steps, the agent can process ~8 skills per invocation
4. The agent reports which skills were processed and which remain
5. Users re-invoke to continue processing remaining skills

A `force` parameter (boolean, default false) controls overwrite behavior — skip existing variants by default, overwrite when `force = true`.

## Technical Approach

### Phase 1: ToolContext Extension

Add `provider_name` and `model_name` fields to `ToolContext` so the builtin handler can access the agent's current LLM configuration.

#### `crates/mika-agent/src/tools/mod.rs`

Add two fields to `ToolContext`:

```rust
pub struct ToolContext<'a> {
    // ... existing fields ...
    /// Current LLM provider name (e.g., "anthropic", "openrouter").
    pub provider_name: &'a str,
    /// Current LLM model name (e.g., "claude-sonnet-4-6", "anthropic/claude-sonnet-4").
    pub model_name: &'a str,
}
```

#### Construction sites (4 files)

- `crates/mika-agent/src/agent.rs` — main conversation loop ToolContext construction
- `crates/mika-agent/src/server/investigate.rs` — investigation panel (can use empty strings, no skills there)
- `crates/mika-agent/src/tools/send_message.rs` — send_message tool's inner ToolContext
- `crates/mika-agent/src/test_utils.rs` — test harness `TestHarness::ctx()` (use "anthropic" / "claude-sonnet-4-6" defaults)

### Phase 2: Template Files

Create `crates/mika-agent/templates/skills/skill-review/` with three files:

#### `skill.toml`

```toml
[skill]
name = "skill-review"
description = "Generate model-tuned system_prompt.md variants for skills"
version = "0.1.0"
always_on = false
timeout_secs = 60

[triggers]
keywords = ["review skill", "adapt skill", "generate variant", "tune prompt", "skill variant"]

[constraints]
required_tools = ["review_skill"]
```

- `always_on = false` — keyword-triggered only (not injected into every conversation)
- `timeout_secs = 60` — the builtin handler does filesystem I/O; 60s is generous
- `required_tools = ["review_skill"]` — ensures the agent actually calls the tool rather than fabricating a response
- Keywords scoped to avoid false positives (e.g., "review" alone would match too broadly)

#### `tools.json`

```json
[
  {
    "name": "review_skill",
    "description": "Gather skill prompt data and resolve variant paths for generating a model-tuned system_prompt.md. Returns structured data including the root prompt, tool signatures, resolved provider/model, and target write path. Use write_agent_file to write the adapted prompt afterward.",
    "input_schema": {
      "type": "object",
      "properties": {
        "skill_name": {
          "type": "string",
          "description": "Skill name to review, or '*' for batch mode (returns list of all eligible skills)"
        },
        "dry_run": {
          "type": "boolean",
          "default": false,
          "description": "If true, return data for review without expecting a write afterward"
        },
        "force": {
          "type": "boolean",
          "default": false,
          "description": "If true, include skills that already have variants for the current model (overwrite mode)"
        }
      },
      "required": ["skill_name"]
    },
    "handler": {
      "type": "builtin",
      "function": "review_skill"
    }
  }
]
```

#### `system_prompt.md`

The meta-prompt that teaches the agent how to adapt skill prompts. Key sections:

1. **Role**: "You are a prompt engineering expert. When the user asks you to review or adapt a skill, use the `review_skill` tool to gather the skill's source prompt and metadata."
2. **Workflow**: Call `review_skill` -> receive structured data -> generate adapted prompt -> write via `write_agent_file` (or display if dry_run)
3. **Adaptation guidelines**: Preserve semantic intent, adapt instruction style for the target model, adjust tool call format guidance, maintain all tool references
4. **Model capability profiles**: Static section with known characteristics for key models:
   - `claude-sonnet-4-6`: Strong instruction following, XML tag conventions, extended thinking, cache-aware
   - `gpt-4o`: System/user message distinction, function calling format, concise instruction preference
   - `gemini-2.0-flash`: Grounding with Google Search, structured output preferences, safety filter awareness
   - Fallback guidance for unknown models
5. **Batch workflow**: When `*` is used, process the returned list one at a time, prioritize skills without existing variants
6. **Quality checklist**: Verify adapted prompt preserves all tool names, doesn't invent capabilities, stays within size limits

Prompt size target: ~8KB (well under 16KB default limit).

### Phase 3: Builtin Handler Implementation

#### `crates/mika-agent/src/skills/builtin_handlers.rs`

Add `review_skill` to `KNOWN_BUILTINS` array (alphabetical order) and `execute()` dispatch.

##### Handler function: `review_skill`

```rust
async fn review_skill(input: &serde_json::Value, ctx: &ToolContext<'_>) -> ToolOutput
```

**Input validation:**
- `skill_name`: required, non-empty, max 200 chars
- `dry_run`: optional boolean (default false)
- `force`: optional boolean (default false)

**Single skill flow** (`skill_name != "*"`):

1. Resolve skills directory: `{ctx.home_dir}/skills/`
2. Check skill exists: `skills_dir.join(skill_name)` — verify directory exists
3. Check linked status: `fs::symlink_metadata()` → if symlink, return error "Skill '{name}' is installed with --link. Variants cannot be written to linked skills. Unlink first with `mika skills uninstall {name}` then reinstall without --link."
4. Read root prompt: `{skill_dir}/system_prompt.md` — if missing, return error "Skill '{name}' has no system_prompt.md to adapt."
5. Read tools: `{skill_dir}/tools.json` — if missing, use `"[]"` (prompt-only skills have no tools)
6. Resolve canonical provider/model:
   - Use `ProviderKind::from_str(ctx.provider_name)` to get the provider kind
   - If `provider_kind.model_names_contain_slash()` (OpenRouter): split `ctx.model_name` on first `/` → `(canonical_provider, canonical_model)`
   - Else: `(ctx.provider_name, ctx.model_name)` as-is
   - Model dir name: use `sanitize_model_dir_name(canonical_model)` (replaces `/` with `--`)
7. Compute variant path: `skills/{skill_name}/{canonical_provider}/{sanitized_model}/system_prompt.md`
8. Check existing variant: read file at variant path if it exists
9. If variant exists and `!force`: return info with `"skipped": true, "reason": "variant already exists"`
10. Return JSON:

```json
{
  "skill_name": "web-search",
  "root_prompt": "...",
  "tools_json": "[...]",
  "provider": "anthropic",
  "model": "claude-sonnet-4-6",
  "variant_path": "skills/web-search/anthropic/claude-sonnet-4-6/system_prompt.md",
  "existing_variant": null,
  "dry_run": false,
  "skipped": false
}
```

**Batch flow** (`skill_name == "*"`):

1. Scan skills directory for all subdirectories
2. For each skill directory:
   - Skip if symlink (linked)
   - Skip if no `system_prompt.md`
   - Compute variant path (same as single flow)
   - Check existing variant; skip if exists and `!force`
   - Collect into list
3. Return JSON array with summary:

```json
{
  "mode": "batch",
  "provider": "anthropic",
  "model": "claude-sonnet-4-6",
  "eligible_skills": [
    {"name": "web-search", "has_variant": false},
    {"name": "shell-exec", "has_variant": false}
  ],
  "skipped_skills": [
    {"name": "tmux", "reason": "linked"},
    {"name": "github", "reason": "variant exists"}
  ],
  "total_eligible": 8,
  "total_skipped": 3
}
```

The agent then calls `review_skill` for individual skills from the eligible list.

**Output cap:** Apply `MAX_OUTPUT_LEN` (10,000 chars) to root_prompt content. If the root prompt exceeds this, truncate with a note.

### Phase 4: Registration

#### `crates/mika-agent/src/bundled_skills.rs`

Add skill template registration:

```rust
static SKILL_REVIEW_SKILL: BundledSkill = skill!("skill-review", [
    ("skill.toml" => "../templates/skills/skill-review/skill.toml"),
    ("system_prompt.md" => "../templates/skills/skill-review/system_prompt.md"),
    ("tools.json" => "../templates/skills/skill-review/tools.json"),
]);
```

Add `&SKILL_REVIEW_SKILL` to the `BUNDLED_SKILLS` array (alphabetical position after `SHELL_EXEC_SKILL`).

### Phase 5: Documentation

#### `docs/skills.md`

Add a "Model Tuning" section explaining:

1. What the `skill-review` skill does
2. How to invoke it (example phrases)
3. Where variants are stored (`{skill}/{provider}/{model}/system_prompt.md`)
4. The two-level prompt resolution: model-specific -> root (no provider-level prompts)
5. Dry-run workflow for reviewing before writing
6. Batch mode usage and step limit behavior
7. Aggregator provider behavior (OpenRouter -> canonical tuple)
8. How to manually edit or remove variants

### Phase 6: Tests

#### Unit tests in `builtin_handlers.rs`

1. `test_review_skill_in_known_builtins` — verify `"review_skill"` is in `KNOWN_BUILTINS`
2. `test_review_skill_missing_skill_name` — empty/missing input validation
3. `test_review_skill_nonexistent_skill` — skill directory doesn't exist
4. `test_review_skill_no_prompt` — skill exists but has no `system_prompt.md`
5. `test_review_skill_linked_skill_refused` — symlinked skill returns error
6. `test_review_skill_single_happy_path` — returns structured data with correct fields
7. `test_review_skill_existing_variant_skipped` — variant exists, `force=false` -> skipped
8. `test_review_skill_existing_variant_force` — variant exists, `force=true` -> included
9. `test_review_skill_batch_mode` — `skill_name="*"` returns eligible/skipped lists
10. `test_review_skill_aggregator_resolution` — OpenRouter model name extracted to canonical tuple
11. `test_review_skill_dry_run` — dry_run=true flag passed through correctly

#### Test helpers

Tests need a temporary directory with mock skill structures. Use `tempdir` + create mock `skill.toml` and `system_prompt.md` files. The `TestHarness` provides `ctx()` with `home_dir` pointing to the temp directory.

## System-Wide Impact

### Interaction Graph

`review_skill` builtin handler -> reads filesystem (skills directory) -> returns JSON to agent loop -> agent generates adapted prompt (LLM call, within normal agent turn) -> agent calls `write_agent_file` -> `write_agent_file` writes to `{home_dir}/skills/{name}/{provider}/{model}/system_prompt.md` -> `skills_dirty` flag set -> next turn rescans `SkillRegistry` via `scan_skills_dir()` -> new variant picked up by `resolve_prompt()`.

### Error Propagation

- Missing skill directory: `ToolOutput` with `is_error: true`, agent receives error message
- Missing prompt: `ToolOutput` with `is_error: true`
- Linked skill: `ToolOutput` with `is_error: true`
- Write failure in `write_agent_file`: handled by existing overwrite confirmation flow
- Filesystem I/O errors: caught by `std::fs::read_to_string()` -> `ToolOutput` error

### State Lifecycle Risks

- Variant written but `skills_dirty` not set: variant exists on disk but not in registry until restart. Mitigated: `write_agent_file` already sets `skills_dirty` when writing under `skills/`.
- Partial batch: some variants written, agent hits step limit. Safe — each write is atomic, partial progress is useful.
- Stale variant: model behavior changes but variant is not regenerated. Acceptable — user re-runs skill-review to update.

### API Surface Parity

- CLI: `mika skills list` already shows `[variants: N]` badge. No changes needed.
- CLI: `mika skills info` already shows model variants. No changes needed.
- TUI: `/skills` command already shows variants. No changes needed.
- Dashboard: skill display is read-only, no changes needed.

## Acceptance Criteria

- [x] `skill-review` skill is bundled and seeded to `~/.mika/agents/<name>/skills/skill-review/` on startup
- [x] `review_skill` builtin tool accepts `skill_name`, `dry_run`, and `force` parameters
- [x] Provider/model are inferred from agent's current configuration via `ToolContext`
- [x] System prompt includes dynamic specialization: "You are a prompt engineering expert"
- [x] Model capability profiles present for `claude-sonnet-4-6`, `gpt-4o`, `gemini-2.0-flash`
- [x] Single skill: returns root prompt, tools, resolved provider/model, variant path
- [x] Batch mode (`skill_name = "*"`): returns eligible/skipped skill lists
- [x] Linked skills: refused with actionable error message
- [x] Existing variants: skipped by default, included with `force = true`
- [x] `dry_run = true`: data returned without expectation of write
- [x] Aggregator providers (OpenRouter): canonical tuple extracted from model name
- [x] `ToolContext` extended with `provider_name` and `model_name` fields
- [x] `docs/skills.md` updated with Model Tuning section
- [x] `cargo clippy` passes clean
- [x] Unit tests cover: validation, happy path, linked refusal, batch, aggregator, existing variant skip/force

## Dependencies & Risks

- **ToolContext change cascades** to 4 construction sites — mechanical but must not be missed
- **Tool name collision risk**: `review_skill` must not collide with any existing builtin or skill tool. Verified: no existing tool named `review_skill`.
- **Prompt quality**: The generated variants are only as good as the agent's understanding of target model quirks. This is inherently best-effort.
- **Step limit in batch mode**: Users may need multiple invocations for large skill sets. Documented behavior, not a bug.

## Sources & References

- Issue: #243
- Per-provider variant system: #241, #245, #246
- Provider-level prompt removal: commit a21c12c
- Existing pattern: `docs/solutions/architecture-patterns/adding-builtin-handler-skill-git-ops.md`
- Prompt-only skill pattern: `docs/solutions/integration-issues/adding-prompt-only-bundled-skill.md`
- Tool name shadowing gotcha: `docs/solutions/logic-errors/builtin-skill-tool-name-shadowing.md`
- Variant directory conventions: `docs/solutions/architecture-patterns/per-provider-skill-variant-directories.md`
