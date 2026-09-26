---
title: "feat: Pre-tool context-redundancy check for read tools"
type: feat
status: active
date: 2026-04-20
issue: 647
---

# Pre-Tool Context-Redundancy Check

## Overview

Add engine-level guards that detect when read tools (`read_agent_file`, `search_memory`, `get_documentation`) request data already present in the agent's context (system prompt, injected skill prompts, core memory). On detection, return a redirect message pointing to the existing context location instead of executing the tool. Extends the #645 core_memory path guard pattern to a broader set of redundancy cases.

## Problem Frame

The agent exhibits a "tool-first reflex" — reaching for read tools even when the requested data is already injected into the system prompt. Observed post-#645: mika-dev called `read_agent_file("self_model.md")` despite knowing the data was in her system prompt. The #645 guard caught this specific case, but the same class of redundancy applies to `search_memory` (querying for content visible in core memory or skill prompts) and `get_documentation` (fetching docs whose content is already injected via skill prompts).

Each redundant tool call wastes tokens, DB queries, context window space, and latency. Engine-level guards are the durable fix — prompt-level instructions are advisory and the model can ignore them.

## Requirements Trace

- R1. `read_agent_file` with a path matching an active skill's prompt file returns a warn-and-redirect
- R2. `search_memory` with `category="core_memory"` returns a redirect noting core memory is already in the system prompt
- R3. `search_memory` with a query whose tokens match system prompt section headings returns a hint appended to results
- R4. `get_documentation` with a topic whose content overlaps with an already-loaded skill prompt returns a redirect hint
- R5. Unit tests for each redirect path with no regression on legitimate reads
- R6. Telemetry: log redirects with `info!` including tool name, matched context source, and trace_id

## Scope Boundaries

- Path-matching (option 1) and identifier-matching (option 2) only — no semantic/embedding-based redundancy detection
- Guards run inside individual tool `execute()` methods, following the #645 pattern — not at the dispatch layer
- No changes to `ToolContext` struct — guards use data already available or passed via a new lightweight mechanism

### Deferred to Separate Tasks

- Semantic redundancy detection (option 3 from issue): deferred until hit-rate data justifies the complexity
- Cross-turn redundancy tracking (detecting re-reads of data fetched in previous turns): separate concern
- `get_documentation` topic-to-skill-prompt overlap mapping: deferred — the overlap is rare and the `get_documentation` guard is limited to a simple topic-name check

## Context & Research

### Relevant Code and Patterns

- `crates/mika-agent/src/tools/read_agent_file.rs` — #645 `is_core_memory_path()` guard, reference implementation
- `crates/mika-agent/src/tools/search_memory.rs` — `SearchMemoryTool`, `search_core_memory()` always queries DB
- `crates/mika-agent/src/skills/builtin_handlers.rs` — `get_documentation()` handler, compile-time embedded docs
- `crates/mika-agent/src/agent.rs:2880` — `inject_skills_and_resolve_tools()` where skill prompts are written to system prompt
- `crates/mika-agent/src/skills/index.rs:87` — `SkillEntry` with `dir: PathBuf`, `prompt_snippet: String`, `manifest.skill.name`
- `crates/mika-agent/src/tools/mod.rs` — `ToolContext`, `ToolOutput::error()`, `ToolOutput::success()`

### Institutional Learnings

- `docs/solutions/architecture-patterns/core-memory-path-guard-read-agent-file.md` — Three-layer defense: runtime guard + system prompt preamble + tool description. Guard runs before I/O validation.
- `docs/solutions/architecture-patterns/per-turn-tool-use-dedup-guard.md` — Dispatch-layer guard pattern; dedup must happen at execution time, not persistence time.
- `docs/solutions/architecture-patterns/persistence-evaluation-guard.md` — Nudge vs rejection: softer "consider" language when the agent may have legitimate reasons.
- `docs/solutions/best-practices/list-tool-status-summary-reduces-redundant-calls.md` — Tool outputs should make the right next action obvious.

## Key Technical Decisions

- **Guard placement: inside each tool's `execute()` method** — follows the #645 pattern. Each tool knows its own domain best. Avoids mixing dispatch routing with domain logic. The alternative (pre-dispatch hook in `execute_tool()`) would require threading context-awareness into the dispatch layer, adding coupling.

- **Return semantics: `ToolOutput::error()` for definitive redirects, hints appended to `ToolOutput::success()` for soft nudges** — When the data is definitively already in context (skill prompt file, core memory category), use `error()` like #645 — the tool call is fundamentally unnecessary. For `search_memory` heading-match hints, append a note to the normal results since the query may also match non-prompt data.

- **Skill prompt path matching via new `ToolContext` field** — `ToolContext` gains an `active_skill_paths: &[SkillPathInfo]` field (a slice of `(skill_name, prompt_file_path)` tuples). Computed once in `run_agent_inner()` from the matched skills, passed to all tool executions. Lightweight — no new allocations per tool call.

- **`search_memory` core_memory category redirect** — When `category="core_memory"`, the tool currently queries the DB. Since core memory is always in the system prompt, redirect with an informative message instead. This is a hard redirect (error), not a hint.

- **`search_memory` heading-match hint** — For `category="all"` queries, check if the query string case-insensitively matches a core memory section name. If so, append a hint to the results noting the data is likely in the system prompt. This is a soft hint — the search still runs because the query may match structured facts too.

## Open Questions

### Resolved During Planning

- **Should `get_documentation` check overlap with skill prompts?** — No, deferred. The topic-to-skill-prompt mapping would require maintaining a manual mapping table. For now, `get_documentation` has no redundancy guard — its compile-time docs rarely overlap with skill prompts.

- **Should `read_agent_file` check against ALL injected context or just skill prompts?** — Just skill prompt files. Core memory is already covered by #645. Skill prompts are the remaining known redundancy vector from observed behavior.

- **How to identify skill prompt file paths from `ToolContext`?** — Each matched `SkillEntry` has `dir: PathBuf` — the prompt file is at `{dir}/system_prompt.md`. We build a `Vec<SkillPathInfo>` from matched skills in `run_agent_inner()` and pass it via `ToolContext`.

### Deferred to Implementation

- Exact wording of redirect messages — will iterate based on what reads naturally
- Whether `search_memory` heading-match should also check against loaded skill section headings (start with core memory headings only, expand if hit rate is low)

## Implementation Units

- [x] **Unit 1: Add `active_skill_paths` to ToolContext**

**Goal:** Thread active skill prompt file path info through to tools so they can detect redundant reads.

**Requirements:** Foundation for R1

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/mod.rs`
- Modify: `crates/mika-agent/src/agent.rs`
- Modify: `crates/mika-agent/src/test_utils.rs`

**Approach:**
- Define `SkillPathInfo { skill_name: String, prompt_relative_path: String }` struct in `tools/mod.rs`
- `prompt_relative_path` stores the path relative to the agent home (e.g., `skills/<skill-name>/system_prompt.md`)
- Add `active_skill_paths: &'a [SkillPathInfo]` to `ToolContext`
- In `run_agent_inner()`, after `inject_skills_and_resolve_tools()`, build the `Vec<SkillPathInfo>` from matched `SkillEntry` items — extract `dir` relative to `home_dir`, join with `system_prompt.md`
- Update `TestHarness` to provide an empty slice by default
- For silent mode (`run_silent_agent`), use an empty slice — silent mode doesn't need this guard

**Patterns to follow:**
- Existing `ToolContext` fields (e.g., `brave_api_key: Option<&'a str>`)
- `TestHarness::ctx()` and `ctx_with_home()` patterns

**Test scenarios:**
- Happy path: `ToolContext` construction with non-empty `active_skill_paths` compiles and is accessible
- Edge case: empty `active_skill_paths` slice (default for tests and silent mode)

**Verification:**
- `cargo build` succeeds with the new field
- All existing tests pass (TestHarness provides empty slice)

---

- [x] **Unit 2: Skill prompt path guard in `read_agent_file`**

**Goal:** Detect when `read_agent_file` targets a file that is already loaded as an active skill's system prompt and return a redirect.

**Requirements:** R1, R5, R6

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/mika-agent/src/tools/read_agent_file.rs`
- Test: `crates/mika-agent/src/tools/read_agent_file.rs` (inline tests)

**Approach:**
- Add `fn is_active_skill_prompt(path: &str, active_skill_paths: &[SkillPathInfo]) -> Option<&str>` helper
- Normalize the input path (strip `./`, `~/`) and compare against each `SkillPathInfo.prompt_relative_path`
- Also match `skills/<name>/system_prompt.md` pattern variants
- Place the check after the existing `is_core_memory_path()` guard, before `validate_and_resolve_path()`
- Return `ToolOutput::error()` with message: "The file '{path}' is already loaded as the '{skill_name}' skill prompt in your system prompt context. Read it from the '<context type=\"skill\">' block above instead of re-fetching."
- Log `info!` with tool name, matched skill, and trace_id

**Patterns to follow:**
- `is_core_memory_path()` pattern — private helper function, guard check before I/O validation
- Domain-specific error message that names the content and redirects

**Test scenarios:**
- Happy path: `read_agent_file("skills/self-dev/system_prompt.md")` with `self-dev` in active_skill_paths → error with redirect message
- Happy path: `read_agent_file("notes.md")` with skills active → succeeds normally (no match)
- Edge case: path with `~/` prefix matching a skill prompt → redirect
- Edge case: path with `./` prefix matching a skill prompt → redirect
- Edge case: empty `active_skill_paths` → all reads pass through
- Error path: path partially matching (e.g., `skills/self-dev/handlers/run.sh`) → no redirect (only `system_prompt.md` matches)
- Integration: core_memory guard still fires first for core memory paths (ordering preserved)

**Verification:**
- `cargo test -p mika-agent -- read_agent_file` passes with new tests
- Existing read_agent_file tests unchanged

---

- [x] **Unit 3: Core memory category redirect in `search_memory`**

**Goal:** Redirect `search_memory` calls with `category="core_memory"` since core memory is always in the system prompt.

**Requirements:** R2, R5, R6

**Dependencies:** None

**Files:**
- Modify: `crates/mika-agent/src/tools/search_memory.rs`
- Test: `crates/mika-agent/src/tools/search_memory.rs` (inline tests)

**Approach:**
- At the top of `execute()`, after input validation, check if `category == "core_memory"`
- Return `ToolOutput::error()` with message: "core_memory is auto-injected into your system prompt on every turn — the content is already in the 'Core Memory' block above. Search there directly instead of querying the database. If you need to modify core memory, use the update_core_memory tool."
- Log `info!` with `context_redundancy_redirect`, query, and trace_id
- This replaces the DB query entirely for `category="core_memory"` — the data is always fresher in the prompt

**Patterns to follow:**
- #645 core_memory path guard error message style
- Input validation pattern at top of `execute()`

**Test scenarios:**
- Happy path: `search_memory(query="self_model", category="core_memory")` → error redirect
- Happy path: `search_memory(query="Alice", category="person")` → normal search (no redirect)
- Happy path: `search_memory(query="Alice", category="all")` → normal search (category is "all", not "core_memory")
- Edge case: `search_memory(query="", category="core_memory")` → validation error fires before redirect (empty query)

**Verification:**
- `cargo test -p mika-agent -- search_memory` passes
- No regression on existing search tests

---

- [x] **Unit 4: Core memory heading hint in `search_memory` for `category="all"`**

**Goal:** When `search_memory` with `category="all"` has a query matching a core memory section name, append a hint to the results.

**Requirements:** R3, R5

**Dependencies:** Unit 3

**Files:**
- Modify: `crates/mika-agent/src/tools/search_memory.rs`
- Test: `crates/mika-agent/src/tools/search_memory.rs` (inline tests)

**Approach:**
- After all search results are collected (before the final response assembly), check if the query case-insensitively matches any `core_memory_section_names()` entry
- If matched, prepend a hint line to the output: "Hint: '{section}' is a core_memory section already in your system prompt. Check the 'Core Memory' block above first."
- This is a soft hint — search results are still returned because the query may match structured facts too
- Use `core_memory_section_names()` from `db.rs` as single source of truth (same as #645)

**Patterns to follow:**
- `core_memory_section_names()` usage from `read_agent_file.rs`
- Hint appended to results, not replacing them

**Test scenarios:**
- Happy path: `search_memory(query="self_model", category="all")` with core memory seeded → results include hint line
- Happy path: `search_memory(query="Alice", category="all")` → no hint (not a section name)
- Edge case: `search_memory(query="Self_Model", category="all")` → hint fires (case-insensitive)
- Edge case: `search_memory(query="user_summary", category="all")` with no DB results → hint still appears in "no results" output

**Verification:**
- All search_memory tests pass
- Hint appears only for core memory section name queries

---

- [x] **Unit 5: Telemetry and documentation**

**Goal:** Ensure all redirects are logged consistently and update tool descriptions for defense-in-depth.

**Requirements:** R6

**Dependencies:** Units 2, 3, 4

**Files:**
- Modify: `crates/mika-agent/src/tools/read_agent_file.rs` (tool description update)
- Modify: `crates/mika-agent/src/tools/search_memory.rs` (tool description update)

**Approach:**
- Verify all redirect paths have `info!` logs with consistent fields: `tool`, `redirect_reason`, `matched_source`, `trace_id`
- Update `read_agent_file` tool description to mention that skill prompt files are auto-injected
- Update `search_memory` tool description to mention that core_memory is in the system prompt
- These description updates are defense-in-depth (prompt-level + runtime guard)

**Patterns to follow:**
- `read_agent_file` tool description already mentions core_memory non-accessibility
- `search_memory` description currently lists core_memory as a searchable category

**Test expectation: none — description text changes and log format verified by manual inspection**

**Verification:**
- `cargo clippy` passes
- Tool descriptions updated

## System-Wide Impact

- **Interaction graph:** Guards run inside tool `execute()` — no impact on dispatch, post-conditions, or other tools. `ToolContext` gains one new field but all existing callsites provide a default (empty slice).
- **Error propagation:** Redirect errors follow the existing `ToolOutput::error()` pattern — the LLM sees them as tool errors and can self-correct.
- **State lifecycle risks:** None — guards are stateless checks, no DB writes or side effects.
- **API surface parity:** Dashboard tool_calls table will record redirected calls as errors. This is correct — the tool was called but redirected.
- **Unchanged invariants:** `execute_tool()` dispatch, `process_tool_calls()` dedup, post-condition guards, silent mode behavior — all unchanged.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| False positive: `read_agent_file` for a skill prompt file that legitimately needs re-reading | Unlikely — skill prompts are static after injection. If needed, the error message tells the agent where to find the data. |
| `search_memory` core_memory redirect blocks legitimate DB queries | The redirect only fires for `category="core_memory"` explicitly. `category="all"` still queries the DB and just adds a hint. |
| `ToolContext` lifetime changes break compilation | New field is `&'a [SkillPathInfo]` — same lifetime as existing fields. Empty slice `&[]` is a valid default. |

## Sources & References

- Related issues: #645 (core_memory path guard), #648 (persistence evaluation guard)
- Related code: `is_core_memory_path()` in `read_agent_file.rs`, `core_memory_section_names()` in `db.rs`
- Solution docs: `docs/solutions/architecture-patterns/core-memory-path-guard-read-agent-file.md`
