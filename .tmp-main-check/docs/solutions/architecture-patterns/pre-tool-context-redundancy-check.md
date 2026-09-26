---
title: Pre-tool context-redundancy check for read tools
date: 2026-04-20
category: architecture-patterns
module: mika-agent/tools
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding new read tools that fetch data which may already be in the system prompt
  - Extending context injection to include new data sources
  - Debugging redundant tool calls in agent sessions
tags:
  - context-redundancy
  - tool-guard
  - read-agent-file
  - search-memory
  - skill-prompt
  - core-memory
  - defense-in-depth
---

# Pre-tool context-redundancy check for read tools

## Context

The agent exhibits a "tool-first reflex" — reaching for read tools even when the requested data is already injected into the system prompt. This wastes tokens, DB queries, context window space, and latency. Prompt-level instructions ("don't re-read data already in your prompt") are advisory and the model can ignore them. The #645 core_memory path guard proved that engine-level guards are the durable fix for this class of problem.

Issue #647 extended the #645 pattern to cover three additional redundancy cases: skill prompt files via `read_agent_file`, core_memory category via `search_memory`, and core_memory section name hints via `search_memory`.

## Guidance

### Guard placement: inside each tool's `execute()` method

Each guard runs as an early-return check inside the tool's `execute()` method, before any I/O (filesystem, database). This follows the #645 pattern where `is_core_memory_path()` runs before `validate_and_resolve_path()`.

The alternative — a pre-dispatch hook in `execute_tool()` — was rejected because it would mix dispatch routing with domain-specific guard logic and require threading context-awareness into the dispatch layer.

### Return semantics: error for definitive redirects, hints for soft nudges

- `ToolOutput::error()` when the data is definitively in context (skill prompt file, `category="core_memory"`)
- Hint text prepended to `ToolOutput::success()` when the query *might* match in-context data but other results are also relevant (section name match in `category="all"`)

### The three guards

1. **`read_agent_file` skill prompt guard** — `is_active_skill_prompt()` checks the requested path against `ToolContext.active_skill_paths` (a `&[SkillPathInfo]` slice populated from matched skills in `run_agent_inner()`). Returns error if the path matches an active skill's `system_prompt.md`. Uses `normalize_path_prefix()` shared with `is_core_memory_path()`.

2. **`search_memory` core_memory category redirect** — Hard redirect when `category="core_memory"`. Core memory is always auto-injected into the system prompt; querying the DB for it is always redundant.

3. **`search_memory` section name hint** — When `category="all"` and the query case-insensitively matches a `core_memory_section_names()` entry, a hint line is prepended to results. The search still runs because the query may match structured facts too.

### Threading context data to tools

`ToolContext` gained `active_skill_paths: &'a [SkillPathInfo]` — a slice computed once in `run_agent_inner()` from matched skills. Each entry records the skill name and its prompt file's path relative to the agent home. Empty (`&[]`) in silent mode, team mode, and tests by default.

The computation uses `e.dir.strip_prefix(params.home_dir)` with a `warn!` log on failure (e.g., skill dir outside home_dir) so mismatches are visible rather than silently dropped.

## Why This Matters

Each redundant tool call costs: (1) a full round-trip through the tool dispatch chain, (2) a DB query or filesystem read for data already in the prompt, (3) the tool result consuming context window space that could hold useful conversation history. For an agent running 20-step loops with multiple read tools per step, redundant reads compound quickly.

Engine-level guards enforce the invariant structurally. Unlike prompt instructions, they cannot be ignored by the model, and they provide clear redirect messages that teach the model where to find the data.

## When to Apply

- When adding a new read tool that returns data which may already be in the agent's system prompt (core memory, skill prompts, task health, active work items)
- When extending the system prompt to inject new data sources — consider whether existing read tools now have a redundancy vector
- When debugging observed redundant tool calls in agent session logs

### Guard family

This is part of the growing guard family documented in the codebase:

| Guard | Scope | Type |
|-------|-------|------|
| Core memory path guard (#645) | `read_agent_file` | Pre-tool, definitive |
| Skill prompt path guard (#647) | `read_agent_file` | Pre-tool, definitive |
| Core memory category redirect (#647) | `search_memory` | Pre-tool, definitive |
| Core memory heading hint (#647) | `search_memory` | Pre-tool, soft hint |
| Per-turn tool_use dedup (#582) | All tools | Dispatch-time |
| 5 EndTurn post-condition guards | Agent text response | Post-response |

## Examples

### Skill prompt guard

```rust
// In read_agent_file.rs — after core_memory guard, before validate_and_resolve_path
if let Some(skill_name) = is_active_skill_prompt(path, ctx.active_skill_paths) {
    return Ok(ToolOutput::error(format!(
        "The file '{path}' is already loaded as the '{skill_name}' skill prompt ..."
    )));
}
```

### search_memory core_memory redirect

```rust
// Early return before any DB queries
if category == "core_memory" {
    return Ok(ToolOutput::error(
        "core_memory is auto-injected into your system prompt on every turn ..."
    ));
}
```

### search_memory section name hint

```rust
// After results are collected, before response assembly
let core_memory_hint = if category == "all" {
    let query_lower = query.to_lowercase();
    core_memory_section_names()
        .iter()
        .find(|section| section.to_lowercase() == query_lower)
        .map(|section| format!("Hint: '{section}' is a core_memory section ..."))
} else {
    None
};
```

## Related

- `docs/solutions/architecture-patterns/core-memory-path-guard-read-agent-file.md` — predecessor pattern (#645)
- `docs/solutions/architecture-patterns/per-turn-tool-use-dedup-guard.md` — dispatch-time guard (#582)
- `docs/solutions/best-practices/list-tool-status-summary-reduces-redundant-calls.md` — output-side redundancy reduction
- GitHub issue #647
