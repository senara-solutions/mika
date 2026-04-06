---
title: "Adding skill-review builtin handler for model-tuned prompt variants"
category: architecture-patterns
date: 2026-04-06
tags: [skills, builtin-handler, variant-resolution, toolcontext, path-traversal]
---

# Adding skill-review builtin handler for model-tuned prompt variants

## Problem

The per-provider/model skill variant system (#241, #245, #246) enables model-specific prompts but creating variants was entirely manual. Needed an automated way for the agent to generate model-tuned `system_prompt.md` variants.

## Root Cause / Design Challenge

The skill needed access to: (1) the agent's current provider/model config, (2) aggregator detection logic (OpenRouter canonical tuple extraction), (3) the skills directory filesystem. Exec handlers can't access MIKA_* env vars (scrubbed). Prompt-only skills don't have structured filesystem access or config awareness.

## Solution

**Builtin handler** that performs structural work (no LLM calls), returns structured JSON for the agent loop to do creative adaptation:

1. **ToolContext extension** — Added `provider_name: &'a str` and `model_name: &'a str` to `ToolContext`. Updated all 7 construction sites (agent.rs x3, investigate.rs, send_message.rs, test_utils.rs x4). These fields are populated from `effective_llm.provider_name()` / `effective_llm.model_name()` which are already available near each construction site.

2. **Aggregator resolution** — `resolve_canonical_provider_model()` extracts canonical `(provider, model)` from OpenRouter-style model names (`anthropic/claude-sonnet-4` -> `("anthropic", "claude-sonnet-4")`). Uses `ProviderKind::model_names_contain_slash()` to detect aggregators.

3. **Linked skill safety** — `fs::symlink_metadata()` detects symlinked skill directories and refuses to write (executor READ-ONLY invariant).

4. **Path traversal protection** — Validates `skill_name` rejects `/`, `\`, `..`, and null bytes. Critical because `skills_dir.join(skill_name)` is used without `validate_and_resolve_path()` (which is designed for file paths, not directory names).

5. **Batch mode** — `skill_name = "*"` returns a summary list of eligible/skipped skills. Agent processes individually, limited by 20-step tool limit (~8 skills per invocation).

## Key Learnings

- **ToolContext extension is mechanical but high-impact**: 7 construction sites must all be updated. Forgetting one causes a compile error (Rust catches it), but in non-Rust codebases this would be a runtime bug. Pattern: grep for `ToolContext {` to find all sites.

- **Path traversal in directory joins vs file paths**: The existing `validate_and_resolve_path()` helper handles file path security (tilde expansion, symlink checks, canonicalize containment). But for directory name validation (just a component, not a path), simple character rejection is more appropriate. Always validate user-provided path components even when they're "just names."

- **Aggregator canonical tuple is write-path only**: `resolve_prompt()` at runtime uses the configured provider/model verbatim. Variants generated via OpenRouter are only resolved when the agent later switches to the native provider. This is by design — the aggregator is transport, not identity.

- **Builtin handlers returning intermediate data is a valid pattern**: Unlike other builtins (`web_search`, `git_ops`) that return final results, `review_skill` returns structured data for multi-step agent processing. The system prompt teaches the agent the workflow: call tool -> receive data -> generate adaptation -> write via `write_agent_file`.

## Prevention

- When adding fields to shared structs like `ToolContext`, use `cargo check` immediately to catch all construction sites
- Always validate path components for traversal characters, even when using `Path::join()`
- Test symlink detection with `#[cfg(unix)]` guards for cross-platform compatibility
