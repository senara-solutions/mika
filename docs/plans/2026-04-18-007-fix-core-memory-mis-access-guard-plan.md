---
title: "fix: Structural guard against core_memory mis-access via read_agent_file"
type: fix
status: active
date: 2026-04-18
---

# fix: Structural guard against core_memory mis-access via read_agent_file

## Overview

Add three small engine-level guards that prevent agents from attempting to read core_memory sections via `read_agent_file`, which always fails (the content is DB-backed and auto-injected into the system prompt, not stored as files). Currently the agent gets a generic "file not found", then fabricates content. These guards make the structural fact explicit at the engine level.

## Problem Frame

core_memory sections (`self_model`, `user_summary`, `current_priorities`, `key_people`, `workflows`) are auto-injected into every agent's system prompt on every turn. They are stored in SQLite and have no filesystem representation. When an agent calls `read_agent_file({"path": "core_memory/self_model.md"})`, the tool returns a generic "File not found" error. The agent then fabricates content with made-up line numbers, citing content it never read.

The root cause is that the engine provides no structural signal that core_memory is already present in the prompt, the tool has no domain-aware rejection for core_memory paths, and the tool description doesn't disclaim core_memory access.

**Incident:** Session `6ee2bebf`, mika-dev tried `read_agent_file({"path": "core_memory/self_model.md"})`, got "File not found", then produced a fabricated structured analysis.

## Requirements Trace

- R1. `read_agent_file` rejects paths matching core_memory sections with a domain-specific error explaining where the content actually lives
- R2. The system prompt wraps injected core_memory content with a clear header indicating it is auto-loaded and should not be read via tools
- R3. `read_agent_file` tool schema description mentions that core_memory sections cannot be read via this tool
- R4. Regression test: unit test calling `read_agent_file` with core_memory paths asserts the helpful rejection message
- R5. System prompt tool-usage section for `read_agent_file` mentions core_memory exclusion

## Scope Boundaries

- No changes to `write_agent_file` or `list_agent_files` (follow-up if needed)
- No changes to `update_core_memory` tool
- No database schema changes
- The follow-up SQL update to simplify mika-dev/mika-qa's `self_model` content is out of scope (tracked in the issue as a post-deploy step)

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/read_agent_file.rs` — tool definition and execute method; path extracted at line 41, `validate_and_resolve_path` at line 50
- `crates/mika-agent/src/prompt.rs` — `write_core_memory_section()` at line 188 with `<core-memory>` XML delimiters; tool-usage instructions at lines 476–492
- `crates/mika-agent/src/db.rs` — `CORE_MEMORY_SECTIONS` constant at line 90 and `core_memory_section_names()` at line 102
- `crates/mika-agent/src/tools/mod.rs` — `validate_and_resolve_path()` shared helper

### Institutional Learnings

- **Deterministic skill context injection** (`docs/solutions/architecture-patterns/deterministic-skill-context-injection.md`): "Prompt enforcement is advisory — the model can comply or not." Engine-level guards are the correct approach for invariants.
- **Cross-agent file access** (`docs/solutions/architecture-patterns/cross-agent-file-access-builtin-tools.md`): Established pattern for early-return guards in file tools with descriptive error messages.
- **Tool path reporting** (`docs/solutions/logic-errors/tool-path-reporting-misbehavior.md`): Error messages should name the canonical tool so the LLM self-corrects.
- **Tilde home expansion** (`docs/solutions/logic-errors/tilde-home-expansion-file-tools.md`): Path normalization happens in `validate_and_resolve_path`. The core_memory guard should run BEFORE path resolution since it is a semantic/domain check, not a filesystem security check.

## Key Technical Decisions

- **Guard placement: before `validate_and_resolve_path`**: The core_memory check is a domain-level semantic rejection, not a filesystem security check. Placing it early (right after path extraction) avoids unnecessary filesystem operations and makes the intent clear. This follows the tilde-expansion learnings doc.
- **Path matching strategy: normalized prefix + bare name matching**: Check for `core_memory/` and `core-memory/` prefixes, plus bare section names (with or without `.md` extension). Use `core_memory_section_names()` from `db.rs` as the single source of truth — no hardcoded list in the tool.
- **Single helper function `is_core_memory_path()`**: Extract the check into a testable pure function that takes a path string and returns `Option<&str>` (the matched section name). This keeps `execute()` clean and allows direct unit testing of the matching logic.
- **Description update in both places**: Update both the tool schema description (affects API-level tool calling) and the system prompt tool-usage section (affects prompt-level guidance). Defense in depth.

## Implementation Units

- [ ] **Unit 1: Core memory path rejection in `read_agent_file`**

**Goal:** Add a domain-specific early rejection when `read_agent_file` is called with a path that matches a core_memory section.

**Requirements:** R1, R3, R4

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/read_agent_file.rs`
- Test: `crates/mika-agent/src/tools/read_agent_file.rs` (inline tests)

**Approach:**
- Add `fn is_core_memory_path(path: &str) -> Option<&'static str>` helper (outside the impl block, or as a free function). Normalize the input path (strip leading `./`, `~/`, lowercase), then check:
  - Prefix match: starts with `core_memory/` or `core-memory/`
  - Bare name match: the path's file stem (without `.md`) matches any entry from `core_memory_section_names()`
  - Exact directory match: path is exactly `core_memory` or `core-memory`
- In `execute()`, call `is_core_memory_path(path)` immediately after extracting the path string (line 41). If it matches, return `ToolOutput::error(...)` with a message explaining:
  - The path is not filesystem-accessible
  - core_memory sections are auto-injected into the system prompt (already available)
  - To modify core_memory, use `update_core_memory`
- Update the `description` field in `definition()` to append: `"Does NOT access core_memory — core_memory sections (self_model, user_summary, etc.) are auto-injected into your system prompt and cannot be read as files."`

**Patterns to follow:**
- Early return pattern from `resolve_agent_home()` at line 45–48
- Error message style from cross-agent access rejection
- Import `crate::db::core_memory_section_names` (already used in `prompt.rs` and `update_core_memory.rs`)

**Test scenarios:**
- Happy path: `read_agent_file({"path": "core_memory/self_model.md"})` returns error mentioning "auto-injected into your system prompt" and "update_core_memory"
- Happy path: `read_agent_file({"path": "core_memory/user_summary"})` (no extension) returns same domain-specific error
- Happy path: `read_agent_file({"path": "core-memory/self_model.md"})` (hyphenated prefix) returns same error
- Edge case: `read_agent_file({"path": "self_model"})` (bare name) returns domain-specific error
- Edge case: `read_agent_file({"path": "self_model.md"})` (bare name with extension) returns domain-specific error
- Edge case: `read_agent_file({"path": "~/core_memory/workflows.md"})` (tilde prefix) returns domain-specific error
- Edge case: `read_agent_file({"path": "core_memory"})` (directory itself) returns domain-specific error
- Edge case: `read_agent_file({"path": "notes/core_memory_backup.md"})` (non-matching path containing "core_memory" substring) is NOT rejected — normal file read behavior
- Edge case: `read_agent_file({"path": "my_self_model.md"})` (file containing section name as substring) is NOT rejected

**Verification:**
- All new tests pass
- Existing `read_agent_file` tests still pass (no regression)
- `cargo clippy -p mika-agent` clean

- [ ] **Unit 2: System prompt preamble for core_memory**

**Goal:** Wrap the injected core_memory block in the system prompt with a clear header that tells the agent the content is already available and should not be read via tools.

**Requirements:** R2, R5

**Dependencies:** None (independent of Unit 1)

**Files:**
- Modify: `crates/mika-agent/src/prompt.rs`
- Test: `crates/mika-agent/src/prompt.rs` (inline tests)

**Approach:**
- Modify `write_core_memory_section()` to enhance the description text. The existing description for `build_system_prompt` is `"These are your persistent memory blocks. Update them using the update_core_memory tool."`. Enhance to also state that the content is auto-loaded on every turn and should not be read via `read_agent_file`.
- For `build_silent_prompt` (passes `None` description), add a minimal but sufficient description since silent mode agents can also mis-access core_memory.
- Update the tool-usage section (lines 476–492) where `read_agent_file` is mentioned: append a note that core_memory sections cannot be read with this tool.

**Patterns to follow:**
- Existing `write_core_memory_section()` structure with `<core-memory>` XML delimiters
- The tool-usage instruction style in `write_tool_usage_section()`

**Test scenarios:**
- Happy path: `build_system_prompt()` output contains text indicating core_memory is auto-loaded and should not be read via `read_agent_file`
- Happy path: `build_system_prompt()` output's `read_agent_file` tool-usage line mentions core_memory exclusion
- Happy path: `build_silent_prompt()` output contains the auto-loaded indicator in the core_memory section
- Edge case: When core_memory is empty (no entries), the preamble still appears with the guidance text

**Verification:**
- Prompt tests pass
- `cargo test -p mika-agent` passes
- `cargo clippy -p mika-agent` clean

## System-Wide Impact

- **Interaction graph:** Only `read_agent_file` is modified. `write_agent_file` and `list_agent_files` are not affected. The prompt change applies globally to all agent modes (conversation, silent, team).
- **Error propagation:** The new `ToolOutput::error` follows the same pattern as existing rejections. The LLM sees it as a normal tool error and can self-correct.
- **API surface parity:** The tool description change is visible to all API consumers (Claude API tool_use). The system prompt change is internal.
- **Unchanged invariants:** `update_core_memory` tool is not modified. `validate_and_resolve_path()` is not modified. All existing file read/write behavior for non-core_memory paths is unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| False positive: legitimate file named `self_model.md` in agent home rejected | Unlikely — these exact names are reserved for core_memory. The substring check is strict (exact stem match, not contains). If needed, only match when prefixed with `core_memory/`. |
| Silent mode agents lack core_memory guidance | Both `build_system_prompt` and `build_silent_prompt` will be updated. |

## Sources & References

- Related issue: #645
- Incident session: `6ee2bebf-f44d-4b9b-bc0c-fd6a3f391835`
- Related code: `crates/mika-agent/src/tools/read_agent_file.rs`, `crates/mika-agent/src/prompt.rs`, `crates/mika-agent/src/db.rs`
- Learnings: `docs/solutions/architecture-patterns/deterministic-skill-context-injection.md`, `docs/solutions/architecture-patterns/cross-agent-file-access-builtin-tools.md`
