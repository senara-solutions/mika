---
title: "Core memory path guard: structural rejection in read_agent_file"
date: 2026-04-18
category: architecture-patterns
module: mika-agent/tools
problem_type: best_practice
component: tooling
severity: medium
applies_when:
  - Adding engine-level guards to reject tool inputs targeting protected resources
  - Preventing agents from reading DB-backed content via filesystem tools
  - Choosing between prompt-level and engine-level enforcement of invariants
tags:
  - core-memory
  - read-agent-file
  - tool-guard
  - path-rejection
  - structural-enforcement
  - prompt-vs-engine
---

# Core memory path guard: structural rejection in read_agent_file

## Context

core_memory sections (`self_model`, `user_summary`, `current_priorities`, `key_people`, `workflows`) are DB-backed and auto-injected into the agent's system prompt on every turn. They have no filesystem representation. Yet agents repeatedly called `read_agent_file({"path": "core_memory/self_model.md"})`, received a generic "File not found" error, and then fabricated content with made-up line numbers.

The root cause was the absence of any engine-level signal that core_memory is already available. The tool returned a generic filesystem error, and the agent rationalized the gap instead of recovering.

This follows the established principle from `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md`: "Prompt enforcement is advisory — the model can comply or not." When correctness depends on an invariant, enforce it structurally in the engine.

## Guidance

### Three-layer defense in depth

1. **Runtime guard (primary):** `is_core_memory_path(path)` in `read_agent_file.rs` rejects matching paths with a domain-specific error *before* `validate_and_resolve_path`. Returns the matched section name so the error message can be specific.

2. **System prompt preamble (secondary):** `write_core_memory_section()` in `prompt.rs` wraps injected core_memory with a description stating the content is auto-loaded, should not be read via `read_agent_file`, and should be modified via `update_core_memory`.

3. **Tool description (tertiary):** The `read_agent_file` tool schema description explicitly states that core_memory sections cannot be read as files.

### Path matching design

The `is_core_memory_path()` helper uses `core_memory_section_names()` from `db.rs` as the single source of truth — no hardcoded section lists in the matching logic. It normalizes the input (strips leading `./` and `~/`) then checks:

- Prefix match: `core_memory/` or `core-memory/` followed by a section name
- Bare name match: section name with or without `.md` extension
- Directory match: exact `core_memory` or `core-memory`

### Guard placement: before filesystem validation

The guard runs *before* `validate_and_resolve_path` because it is a semantic/domain check, not a filesystem security check. core_memory paths don't correspond to real files, so filesystem validation (tilde expansion, traversal inspection, symlink checks) is irrelevant and would produce misleading errors.

### Error message design

The error message names the specific section, explains where core_memory lives (system prompt), and redirects to `update_core_memory`. This follows the pattern from `docs/solutions/architecture-patterns/cross-agent-file-access-builtin-tools.md`: error messages should name the canonical tool so the LLM self-corrects.

## Why This Matters

Without engine-level enforcement, agents that attempt to read core_memory via file tools get a generic "File not found" error. This provides no structural signal about *why* the read failed or what to do instead, leading to fabrication — the agent invents content it never read. The three-layer approach ensures that even if the agent ignores the prompt guidance (layers 2-3), the runtime guard (layer 1) catches the attempt and returns actionable guidance.

## When to Apply

- Adding guards for DB-backed content that agents might try to access via filesystem tools
- Rejecting tool inputs that target engine-managed resources (core_memory, bundled skills, etc.)
- Choosing between pre-validation guards (semantic rejection) vs post-validation guards (filesystem rejection)
- Designing error messages that enable LLM self-correction

## Examples

**Guard function pattern:**

```rust
fn is_core_memory_path(path: &str) -> Option<&'static str> {
    let stripped = path.strip_prefix("./")
        .or_else(|| path.strip_prefix("~/"))
        .unwrap_or(path);

    // Check prefix, bare name, and directory patterns
    // Use core_memory_section_names() as single source of truth
    // Return Some(section_name) on match, None otherwise
}
```

**Error message pattern:**

```rust
if let Some(section) = is_core_memory_path(path) {
    return Ok(ToolOutput::error(format!(
        "Path '{}' is not filesystem-accessible. core_memory sections ({}) \
         are auto-injected into your system prompt on every turn — \
         the content is already available in the 'Core Memory' block above. \
         To modify core_memory, use the update_core_memory tool.",
        path, section_description
    )));
}
```

**System prompt preamble pattern:**

```
These are your persistent memory blocks, auto-loaded into this prompt
on every turn. The content below is already available — do NOT attempt
to read it via read_agent_file (core_memory is DB-backed, not
filesystem-stored). To modify core_memory, use update_core_memory.
```

## Related

- #645 — Engine: structural guard against core_memory mis-access
- `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md` — "Prompt enforcement is advisory"
- `docs/solutions/architecture-patterns/cross-agent-file-access-builtin-tools.md` — Error message pattern for file tools
- `docs/solutions/architecture-patterns/harden-write-skill-variant-no-path-input.md` — Removing LLM control over paths
- `docs/solutions/logic-errors/tilde-home-expansion-file-tools.md` — `validate_and_resolve_path` pipeline
